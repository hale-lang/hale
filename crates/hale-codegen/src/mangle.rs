//! v1.x-IMPORT PR2: auto-mangler for imported library seeds.
//!
//! Rewrites a parsed library Program's top-level decl names and
//! every use-site that resolves to one of those decls so the
//! library's symbols don't collide with the importer's. Shape
//! mirrors the hand-spelled `__StdLangMorpheme` / `__MoaBraidId`
//! prefixes the std and moa seeds carry today — same flat-namespace
//! discipline, but generated per imported library so users don't
//! have to author the prefix themselves.
//!
//! Mangled form: `__lib_<alias>_<file_stem>_<name>`.
//!   - `alias` is the importer-supplied namespace (`import "x" as foo;`).
//!   - `file_stem` is the basename of the source file the decl lives
//!     in, sans `.hl` — so two files in the same library can share a
//!     name without colliding.
//!   - `name` is the original decl name as written in source.
//!
//! Scope discipline: locals (let-bindings, fn params, lifecycle
//! params, for-loop vars, pattern bindings, generics) shadow
//! top-level names per ordinary lexical rules. The walker maintains
//! a stack of in-scope local names; a Path/Ident reference is
//! rewritten only when it (a) names a seed top-level decl and (b)
//! is not currently shadowed.
//!
//! Out of scope: multi-segment paths (`std::*`, `moa::*`,
//! `<alias>::*`) are never rewritten here. The downstream path-
//! rename table handles them; the mangler only rewrites references
//! to names declared in the same imported seed.

use std::collections::{HashMap, HashSet};

use hale_syntax::ast::*;

/// Rewrite `prog` in place so its top-level decls and any
/// intra-seed references carry the `__lib_<alias>_<file_stem>_*`
/// prefix. `file_stem` is the basename of the source file the
/// program was parsed from, without the `.hl` extension.
///
/// This entry is correct only for single-file libraries (or
/// multi-file libraries whose files don't reference each other).
/// For multi-file libraries with intra-seed cross-file references,
/// use `build_seed_renames` + `mangle_with_renames` so every file
/// shares a unified name table — otherwise a reference from
/// `a.hl` to a decl in `b.hl` won't resolve, since the mangler
/// only sees `a.hl`'s own top-level decls when building its
/// rename map.
pub fn mangle_program(prog: &mut Program, alias: &str, file_stem: &str) {
    let mut renames: HashMap<String, String> = HashMap::new();
    for item in &prog.items {
        if let Some(n) = top_decl_name(item) {
            renames.insert(n.to_string(), mangled(alias, file_stem, n));
        }
    }
    mangle_with_renames(prog, &renames);
}

/// Build the unified `name -> mangled_name` map for a multi-file
/// library: every program in the bundle contributes its top-level
/// names. Pair with `mangle_with_renames` to mangle each file
/// against the shared map so cross-file references resolve.
///
/// `programs` is a slice of `(file_stem, &Program)` pairs — the
/// stem becomes part of the mangled prefix for that file's own
/// decls, while use-sites of any name in the seed resolve through
/// the shared map regardless of which file the use-site lives in.
/// Build the rename map for one imported lib seed. `lib_id` is
/// a stable, sanitized identifier for the lib — derived from the
/// canonical path of the lib's directory (workspace-root-relative
/// when known), NOT the importer's chosen alias. Same lib →
/// same `lib_id` → same mangled names across consumers, so a DTO
/// seed imported by two apps produces symbol-identical types
/// (which is the natural shape for shared contracts on a bus).
///
/// 2026-05-22: switched from alias-based mangling (which was
/// per-importer and broke cross-app type identity for shared
/// seeds) to path-based mangling. Collision avoidance is
/// preserved (two different libs live at different paths and get
/// different `lib_id`s). The importer's alias is still used at
/// the path-rename table level so call-site references
/// `alias::Name` resolve correctly.
pub fn build_seed_renames(
    programs: &[(String, &Program)],
    lib_id: &str,
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for (stem, prog) in programs {
        for item in &prog.items {
            // Stage-2 FFI (2026-05-22): `@ffi("c") fn name(...)`
            // declarations must keep the literal `name` as their
            // LLVM symbol — the linker resolves against C glue
            // that exports that exact name. Map identity so the
            // mangler walk doesn't rewrite the fn name, AND the
            // per-build path-rename table gets `(alias::name,
            // name)` so call sites still resolve correctly.
            // Library authors who want symbol uniqueness across
            // multiple FFI libs should prefix at the C side
            // (raylib_init_window vs glfw_init_window) — the
            // substrate doesn't auto-namespace FFI symbols.
            if let TopDecl::Fn(f) = item {
                if f.ffi.is_some() {
                    out.insert(f.name.name.clone(), f.name.name.clone());
                    continue;
                }
            }
            if let Some(n) = top_decl_name(item) {
                out.insert(n.to_string(), mangled(lib_id, stem, n));
            }
        }
    }
    out
}

/// Mangle `prog` using a pre-built `renames` map. Used by the CLI
/// when mangling multi-file libraries: the caller builds one
/// unified map via `build_seed_renames` then applies it to each
/// file, so a use-site in `a.hl` referencing a decl in `b.hl`
/// rewrites to the correct mangled name.
pub fn mangle_with_renames(prog: &mut Program, renames: &HashMap<String, String>) {
    if renames.is_empty() {
        return;
    }
    let mut walker = Mangler {
        renames,
        scopes: Vec::new(),
    };
    for item in &mut prog.items {
        walker.walk_top_decl(item);
    }
}

/// brained F.1 / 2026-05-23 — apply the cross-seed
/// path-rename table to every `TypeExpr::Named { path: [alias,
/// Name] }` in the program. Where the multi-segment path
/// matches a rename entry, the path is collapsed to a single-
/// segment path carrying the mangled name. Lets the type
/// checker see `Project` (single name, lookable in the bundle
/// scope) instead of `model::Project` (qualified, opaque to
/// type-resolve).
///
/// Pre-fix: a qualified-path cell type in `@form(hashmap)`
/// (e.g. `pool entries of model::Project indexed_by id`)
/// was rejected at typecheck because the shape-check only
/// admits single-segment paths. The codegen-side path
/// renames table already resolves the call shape, but it
/// ran AFTER typecheck, so the @form check fired against
/// the un-rewritten qualified path. This pre-typecheck pass
/// closes that window.
///
/// Scope: only TypeExpr position rewrites here.
/// Expression-position qualified-path references
/// (`model::frob(x)` etc.) continue to lower via the codegen
/// `mangled_for_path` machinery — those don't need to round-
/// trip through the type checker.
///
/// The `renames` list comes from `hale_cli::ImportRenames`
/// (Vec<(Vec<String>, String)>): each entry is
/// `(["alias", "Name"], "__lib_<alias>_<stem>_Name")`.
pub fn apply_qualified_path_renames(
    prog: &mut Program,
    renames: &[(Vec<String>, String)],
) {
    if renames.is_empty() {
        return;
    }
    let mut walker = QualifiedRenameApplier { renames };
    for item in &mut prog.items {
        walker.walk_top_decl(item);
    }
}

struct QualifiedRenameApplier<'a> {
    renames: &'a [(Vec<String>, String)],
}

impl<'a> QualifiedRenameApplier<'a> {
    fn rewrite_type_expr(&self, t: &mut TypeExpr) {
        match t {
            TypeExpr::Primitive(_, _) => {}
            // Phase 2a: `perspective(P)` is a single-name ref; the
            // qualified-path applier only rewrites 2+ segment paths.
            TypeExpr::Perspective { .. } => {}
            TypeExpr::Named { path, generic_args, .. } => {
                if path.segments.len() >= 2 {
                    let path_segs: Vec<String> = path
                        .segments
                        .iter()
                        .map(|s| s.name.clone())
                        .collect();
                    for (key, mangled) in self.renames {
                        if key.len() == path_segs.len()
                            && key
                                .iter()
                                .zip(path_segs.iter())
                                .all(|(k, p)| k == p)
                        {
                            let span = path.segments[0].span;
                            path.segments.truncate(1);
                            path.segments[0].name = mangled.clone();
                            path.segments[0].span = span;
                            break;
                        }
                    }
                }
                for g in generic_args {
                    self.rewrite_type_expr(g);
                }
            }
            TypeExpr::Projection { inner, .. } => {
                self.rewrite_type_expr(inner);
            }
            TypeExpr::Array { elem, .. } => self.rewrite_type_expr(elem),
            TypeExpr::Bounded { elem, .. } => {
                self.rewrite_type_expr(elem)
            }
            TypeExpr::Tuple(ts, _) => {
                for t in ts {
                    self.rewrite_type_expr(t);
                }
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.rewrite_type_expr(p);
                }
                if let Some(r) = ret {
                    self.rewrite_type_expr(r);
                }
            }
        }
    }

    fn walk_top_decl(&mut self, d: &mut TopDecl) {
        match d {
            TopDecl::Locus(l) => {
                for m in &mut l.members {
                    self.walk_locus_member(m);
                }
            }
            TopDecl::Type(t) => match &mut t.body {
                TypeDeclBody::Alias(te) => self.rewrite_type_expr(te),
                TypeDeclBody::Struct(fields) => {
                    for f in fields {
                        self.rewrite_type_expr(&mut f.ty);
                    }
                }
                TypeDeclBody::Enum(variants) => {
                    for v in variants {
                        for field_ty in &mut v.fields {
                            self.rewrite_type_expr(field_ty);
                        }
                    }
                }
            },
            TopDecl::Const(c) => self.rewrite_type_expr(&mut c.ty),
            TopDecl::Fn(f) => {
                for p in &mut f.params {
                    self.rewrite_type_expr(&mut p.ty);
                }
                if let Some(r) = &mut f.ret {
                    self.rewrite_type_expr(r);
                }
                if let Some(fal) = &mut f.fallible {
                    self.rewrite_type_expr(fal);
                }
            }
            TopDecl::Interface(i) => {
                for m in &mut i.methods {
                    for p in &mut m.params {
                        self.rewrite_type_expr(&mut p.ty);
                    }
                    if let Some(r) = &mut m.ret {
                        self.rewrite_type_expr(r);
                    }
                }
            }
            TopDecl::Topic(t) => self.rewrite_type_expr(&mut t.payload),
            // ring_layout members are layout tokens (idents/ints) — no
            // TypeExpr to path-rewrite for cross-seed imports.
            TopDecl::RingLayout(_) => {}
            TopDecl::Perspective(p) => {
                for m in &mut p.members {
                    match m {
                        PerspectiveMember::Params(pb) => {
                            for p in &mut pb.params {
                                if let Some(ty) = &mut p.ty {
                                    self.rewrite_type_expr(ty);
                                }
                            }
                        }
                        PerspectiveMember::SerializeAs(te) => {
                            self.rewrite_type_expr(te);
                        }
                        PerspectiveMember::Fn(fd) => {
                            for p in &mut fd.params {
                                self.rewrite_type_expr(&mut p.ty);
                            }
                            if let Some(r) = &mut fd.ret {
                                self.rewrite_type_expr(r);
                            }
                            if let Some(fal) = &mut fd.fallible {
                                self.rewrite_type_expr(fal);
                            }
                        }
                        PerspectiveMember::StableWhen(_) => {}
                    }
                }
            }
            TopDecl::Module(_) => {}
            TopDecl::Target(_) => {
                // FUv0.8.2 #7: target capability blocks carry
                // no TypeExprs the import-rename pass needs
                // to rewrite.
            }
        }
    }

    fn walk_locus_member(&mut self, m: &mut LocusMember) {
        match m {
            LocusMember::Params(pb) => {
                for p in &mut pb.params {
                    if let Some(ty) = &mut p.ty {
                        self.rewrite_type_expr(ty);
                    }
                }
            }
            // (Above uses ParamDecl, ty: Option<TypeExpr>.)
            LocusMember::Capacity(cb) => {
                for slot in &mut cb.slots {
                    self.rewrite_type_expr(&mut slot.elem_ty);
                }
            }
            LocusMember::Bus(bb) => {
                for m in &mut bb.members {
                    match m {
                        BusMember::Subscribe { ty, .. } => {
                            if let Some(t) = ty {
                                self.rewrite_type_expr(t);
                            }
                        }
                        BusMember::Publish { ty, .. } => {
                            if let Some(t) = ty {
                                self.rewrite_type_expr(t);
                            }
                        }
                    }
                }
            }
            LocusMember::Lifecycle(lc) => {
                for p in &mut lc.params {
                    self.rewrite_type_expr(&mut p.ty);
                }
            }
            LocusMember::Mode(md) => {
                if let Some(r) = &mut md.ret {
                    self.rewrite_type_expr(r);
                }
            }
            LocusMember::Failure(fd) => {
                for p in &mut fd.params {
                    self.rewrite_type_expr(&mut p.ty);
                }
            }
            LocusMember::Fn(fd) => {
                for p in &mut fd.params {
                    self.rewrite_type_expr(&mut p.ty);
                }
                if let Some(r) = &mut fd.ret {
                    self.rewrite_type_expr(r);
                }
                if let Some(fal) = &mut fd.fallible {
                    self.rewrite_type_expr(fal);
                }
            }
            LocusMember::Const(c) => self.rewrite_type_expr(&mut c.ty),
            LocusMember::Type(t) => match &mut t.body {
                TypeDeclBody::Alias(te) => self.rewrite_type_expr(te),
                TypeDeclBody::Struct(fields) => {
                    for f in fields {
                        self.rewrite_type_expr(&mut f.ty);
                    }
                }
                TypeDeclBody::Enum(variants) => {
                    for v in variants {
                        for field_ty in &mut v.fields {
                            self.rewrite_type_expr(field_ty);
                        }
                    }
                }
            },
            LocusMember::Contract(_)
            | LocusMember::Closure(_)
            | LocusMember::Bindings(_)
            | LocusMember::Placement(_)
            | LocusMember::Topology(_)
            | LocusMember::BirthCheck(_) => {}
        }
    }
}

fn mangled(lib_id: &str, file_stem: &str, name: &str) -> String {
    format!("__lib_{}_{}_{}", lib_id, file_stem, name)
}

fn top_decl_name(d: &TopDecl) -> Option<&str> {
    match d {
        TopDecl::Locus(l) => Some(&l.name.name),
        TopDecl::Perspective(p) => Some(&p.name.name),
        TopDecl::Type(t) => Some(&t.name.name),
        TopDecl::Const(c) => Some(&c.name.name),
        TopDecl::Fn(f) => Some(&f.name.name),
        TopDecl::Interface(i) => Some(&i.name.name),
        TopDecl::Topic(t) => Some(&t.name.name),
        TopDecl::RingLayout(r) => Some(&r.name.name),
        TopDecl::Module(_) => None,
        TopDecl::Target(t) => Some(&t.name.name),
    }
}

struct Mangler<'a> {
    renames: &'a HashMap<String, String>,
    scopes: Vec<HashSet<String>>,
}

impl<'a> Mangler<'a> {
    fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn bind(&mut self, name: &str) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string());
        }
    }
    fn is_shadowed(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }
    fn rewrite_ident(&self, n: &mut String) {
        if self.is_shadowed(n) {
            return;
        }
        if let Some(new_name) = self.renames.get(n) {
            *n = new_name.clone();
        }
    }
    fn rewrite_single_segment_path(&self, q: &mut QualifiedName) {
        if q.segments.len() == 1 {
            self.rewrite_ident(&mut q.segments[0].name);
        }
    }

    fn walk_top_decl(&mut self, d: &mut TopDecl) {
        self.push_scope();
        match d {
            TopDecl::Locus(l) => self.walk_locus(l),
            TopDecl::Perspective(p) => self.walk_perspective(p),
            TopDecl::Type(t) => self.walk_type_decl(t),
            TopDecl::Const(c) => {
                self.walk_type_expr(&mut c.ty);
                self.walk_expr(&mut c.value);
                self.rewrite_ident(&mut c.name.name);
            }
            TopDecl::Fn(f) => self.walk_fn_decl(f),
            TopDecl::Interface(i) => self.walk_interface(i),
            TopDecl::Topic(t) => {
                self.walk_type_expr(&mut t.payload);
                self.rewrite_ident(&mut t.name.name);
            }
            TopDecl::RingLayout(r) => {
                // Layout members are tokens, not types; only the
                // declaration name participates in the rename table.
                self.rewrite_ident(&mut r.name.name);
            }
            TopDecl::Module(_) => {}
            TopDecl::Target(t) => {
                // FUv0.8.2 #7: rewrite the target name only —
                // capability paths are structural identifiers,
                // not user-namespace names, so they don't
                // participate in the import rename table.
                self.rewrite_ident(&mut t.name.name);
            }
        }
        self.pop_scope();
    }

    fn walk_locus(&mut self, l: &mut LocusDecl) {
        self.rewrite_ident(&mut l.name.name);
        for g in &l.generics {
            self.bind(&g.name.name);
        }
        for m in &mut l.members {
            self.walk_locus_member(m);
        }
    }

    fn walk_locus_member(&mut self, m: &mut LocusMember) {
        match m {
            LocusMember::Params(p) => {
                for pd in &mut p.params {
                    if let Some(ty) = &mut pd.ty {
                        self.walk_type_expr(ty);
                    }
                    if let ParamInit::Value(e) = &mut pd.init {
                        self.walk_expr(e);
                    }
                }
            }
            LocusMember::Contract(c) => {
                if let ContractKind::Members(ms) = &mut c.kind {
                    for cm in ms {
                        if let Some(ty) = &mut cm.ty {
                            self.walk_type_expr(ty);
                        }
                    }
                }
            }
            LocusMember::Bus(b) => {
                for bm in &mut b.members {
                    match bm {
                        BusMember::Subscribe { subject, handler, ty, .. } => {
                            // A1 (G1/G32): rewrite the topic-ref ident so a
                            // cross-seed `subscribe alias::Foo as h;` resolves
                            // to the mangled topic decl in the imported seed.
                            // Literal-string subjects are unaffected.
                            // A7 (G16): QualifiedTopic carries a multi-segment
                            // path resolved later through the per-build
                            // path-rename table; the mangler leaves it alone
                            // (same shape as multi-segment Expr::Path).
                            if let BusSubject::Topic(ident) = subject {
                                self.rewrite_ident(&mut ident.name);
                            }
                            // Handler resolves against top-level fns in
                            // the seed; rewrite if it's one of ours.
                            self.rewrite_ident(&mut handler.name);
                            if let Some(t) = ty {
                                self.walk_type_expr(t);
                            }
                        }
                        BusMember::Publish { subject, ty, .. } => {
                            if let BusSubject::Topic(ident) = subject {
                                self.rewrite_ident(&mut ident.name);
                            }
                            if let Some(t) = ty {
                                self.walk_type_expr(t);
                            }
                        }
                    }
                }
            }
            LocusMember::Lifecycle(lc) => self.walk_fn_like(&mut lc.params, lc.ret.as_mut(), &mut lc.body),
            LocusMember::Mode(md) => self.walk_fn_like(&mut md.params, md.ret.as_mut(), &mut md.body),
            LocusMember::Failure(f) => self.walk_fn_like(&mut f.params, None, &mut f.body),
            LocusMember::Closure(c) => {
                if let Some(a) = &mut c.assertion {
                    self.walk_expr(&mut a.left);
                    self.walk_expr(&mut a.right);
                    self.walk_expr(&mut a.tolerance);
                }
                for cl in &mut c.clauses {
                    if let ClosureClause::Epoch(EpochSpec::Duration(e)) = cl {
                        self.walk_expr(e);
                    }
                }
            }
            LocusMember::Fn(f) => self.walk_method_decl(f),
            LocusMember::Const(c) => {
                self.walk_type_expr(&mut c.ty);
                self.walk_expr(&mut c.value);
            }
            LocusMember::Type(t) => self.walk_type_decl(t),
            LocusMember::Capacity(cb) => {
                for slot in &mut cb.slots {
                    self.walk_type_expr(&mut slot.elem_ty);
                    if let Some(ap) = &mut slot.as_parent_for {
                        // Top-level locus name reference.
                        self.rewrite_ident(&mut ap.name);
                    }
                }
            }
            LocusMember::Bindings(bb) => {
                // Topic idents resolve against top-level topic decls;
                // current substitution scope (locus generic params)
                // never collides. v1.x's only TransportSpec variant
                // (`Unix`) carries a literal path string with no
                // expressions inside it, so nothing to walk past
                // the topic ident.
                for entry in &mut bb.entries {
                    self.rewrite_ident(&mut entry.topic.name);
                }
            }
            LocusMember::Placement(pb) => {
                // F.31: placement entries key on main-locus params
                // field names. Field names are local to the main
                // locus — no cross-seed mangling applies here. Pool
                // names (inside cooperative(pool = X)) are also
                // local. Walk does nothing in v1.
                let _ = pb;
            }
            LocusMember::Topology(tb) => {
                // Topology Phase 1b: reserved cores, node ids, and
                // L3-domain names/cores are all literals / local
                // idents (no cross-seed type references). Nothing
                // to rewrite.
                let _ = tb;
            }
            LocusMember::BirthCheck(bc) => {
                // F.27 v2: walk the cond + payload exprs so any
                // generic substitution touches references inside
                // them; the closure name is a regular ident.
                self.walk_expr(&mut bc.cond);
                if let Some(payload) = &mut bc.payload {
                    self.walk_expr(payload);
                }
                self.rewrite_ident(&mut bc.closure_name.name);
            }
        }
    }

    fn walk_perspective(&mut self, p: &mut PerspectiveDecl) {
        self.rewrite_ident(&mut p.name.name);
        for g in &p.generics {
            self.bind(&g.name.name);
        }
        for m in &mut p.members {
            match m {
                PerspectiveMember::Params(pb) => {
                    for pd in &mut pb.params {
                        if let Some(ty) = &mut pd.ty {
                            self.walk_type_expr(ty);
                        }
                        if let ParamInit::Value(e) = &mut pd.init {
                            self.walk_expr(e);
                        }
                    }
                }
                PerspectiveMember::StableWhen(b) => self.walk_block(b),
                PerspectiveMember::SerializeAs(t) => self.walk_type_expr(t),
                PerspectiveMember::Fn(f) => self.walk_fn_decl(f),
            }
        }
    }

    fn walk_type_decl(&mut self, t: &mut TypeDecl) {
        self.rewrite_ident(&mut t.name.name);
        for g in &t.generics {
            self.bind(&g.name.name);
        }
        match &mut t.body {
            TypeDeclBody::Alias(te) => self.walk_type_expr(te),
            TypeDeclBody::Struct(fields) => {
                for f in fields {
                    self.walk_type_expr(&mut f.ty);
                    if let Some(d) = &mut f.default {
                        self.walk_expr(d);
                    }
                }
            }
            TypeDeclBody::Enum(variants) => {
                for v in variants {
                    for fty in &mut v.fields {
                        self.walk_type_expr(fty);
                    }
                }
            }
        }
    }

    fn walk_interface(&mut self, i: &mut InterfaceDecl) {
        self.rewrite_ident(&mut i.name.name);
        for m in &mut i.methods {
            for p in &mut m.params {
                self.walk_type_expr(&mut p.ty);
                if let Some(d) = &mut p.default {
                    self.walk_expr(d);
                }
            }
            if let Some(r) = &mut m.ret {
                self.walk_type_expr(r);
            }
        }
    }

    /// Top-level free fn: its name is a seed top-level decl, so rewrite it
    /// through the rename map, then walk the signature + body.
    fn walk_fn_decl(&mut self, f: &mut FnDecl) {
        self.rewrite_ident(&mut f.name.name);
        self.walk_fn_signature_and_body(f);
    }

    /// Locus method: its name lives in *member* position — looked up on the
    /// locus by its original name (the locus name carries the seed
    /// prefix), never through the seed top-level rename map. Rewriting it
    /// when a top-level fn happens to share the name renamed the decl away
    /// from its (correctly-unrewritten) call sites → "locus … has no
    /// method `name`" (pond P1, FRICTION method-name-shadowed-by-fn). So
    /// walk the signature + body but leave the name alone.
    fn walk_method_decl(&mut self, f: &mut FnDecl) {
        self.walk_fn_signature_and_body(f);
    }

    fn walk_fn_signature_and_body(&mut self, f: &mut FnDecl) {
        self.push_scope();
        for g in &f.generics {
            self.bind(&g.name.name);
        }
        for p in &mut f.params {
            self.walk_type_expr(&mut p.ty);
            if let Some(d) = &mut p.default {
                self.walk_expr(d);
            }
            self.bind(&p.name.name);
        }
        if let Some(r) = &mut f.ret {
            self.walk_type_expr(r);
        }
        if let Some(fb) = &mut f.fallible {
            self.walk_type_expr(fb);
        }
        self.walk_block(&mut f.body);
        self.pop_scope();
    }

    fn walk_fn_like(&mut self, params: &mut [Param], ret: Option<&mut TypeExpr>, body: &mut Block) {
        self.push_scope();
        for p in params.iter_mut() {
            self.walk_type_expr(&mut p.ty);
            if let Some(d) = &mut p.default {
                self.walk_expr(d);
            }
            self.bind(&p.name.name);
        }
        if let Some(r) = ret {
            self.walk_type_expr(r);
        }
        self.walk_block(body);
        self.pop_scope();
    }

    fn walk_block(&mut self, b: &mut Block) {
        self.push_scope();
        for s in &mut b.stmts {
            self.walk_stmt(s);
        }
        if let Some(t) = &mut b.tail {
            self.walk_expr(t);
        }
        self.pop_scope();
    }

    fn walk_stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { name, ty, value, .. } => {
                if let Some(t) = ty {
                    self.walk_type_expr(t);
                }
                self.walk_expr(value);
                self.bind(&name.name);
            }
            Stmt::LetTuple { names, ty, value, .. } => {
                if let Some(t) = ty {
                    self.walk_type_expr(t);
                }
                self.walk_expr(value);
                for n in names.iter() {
                    self.bind(&n.name);
                }
            }
            Stmt::Assign { target, value, .. } => {
                self.walk_lvalue(target);
                self.walk_expr(value);
            }
            Stmt::If(i) => self.walk_if(i),
            Stmt::Match(m) => self.walk_match(m),
            Stmt::For { name, iter, body, .. } => {
                self.walk_expr(iter);
                self.push_scope();
                self.bind(&name.name);
                self.walk_block(body);
                self.pop_scope();
            }
            Stmt::While { cond, body, .. } => {
                self.walk_expr(cond);
                self.walk_block(body);
            }
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    self.walk_expr(e);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => {}
            Stmt::Fail { value, .. } => self.walk_expr(value),
            Stmt::Block(b) => self.walk_block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.walk_expr(a);
                }
                if let Some(m) = modifier {
                    match m {
                        RecoveryModifier::For(e) | RecoveryModifier::Until(e) => self.walk_expr(e),
                    }
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.walk_expr(p);
                }
            }
            Stmt::Send { subject, value, .. } => {
                self.walk_expr(subject);
                self.walk_expr(value);
            }
            Stmt::ShmWrite { topic, max, body, .. } => {
                // Mangle the topic ref like a bus publish/subscribe
                // subject, so `Topic.write(...)` resolves to the same
                // (possibly import-renamed) subject the binding registers.
                self.rewrite_ident(&mut topic.name);
                self.walk_expr(max);
                self.walk_block(body);
            }
            Stmt::Expr(e) => self.walk_expr(e),
        }
    }

    fn walk_lvalue(&mut self, lv: &mut LValue) {
        // The head is either a local var name or a self.* root —
        // never a top-level decl reference (you can't assign into a
        // top-level decl directly). But for safety apply the same
        // shadow-aware rewrite anyway; a shadowed head won't be
        // rewritten and an unshadowed head matching a top-level
        // decl would also have errored at typecheck.
        self.rewrite_ident(&mut lv.head.name);
        for seg in &mut lv.tail {
            if let LValueSeg::Index(e) = seg {
                self.walk_expr(e);
            }
        }
    }

    fn walk_if(&mut self, i: &mut IfStmt) {
        self.walk_expr(&mut i.cond);
        self.walk_block(&mut i.then_block);
        if let Some(e) = &mut i.else_block {
            match e.as_mut() {
                ElseBranch::Else(b) => self.walk_block(b),
                ElseBranch::ElseIf(nested) => self.walk_if(nested),
            }
        }
    }

    fn walk_match(&mut self, m: &mut MatchStmt) {
        self.walk_expr(&mut m.scrutinee);
        for arm in &mut m.arms {
            self.push_scope();
            self.walk_pattern(&mut arm.pattern);
            if let Some(g) = &mut arm.guard {
                self.walk_expr(g);
            }
            match &mut arm.body {
                MatchArmBody::Expr(e) => self.walk_expr(e),
                MatchArmBody::Block(b) => self.walk_block(b),
            }
            self.pop_scope();
        }
    }

    fn walk_pattern(&mut self, p: &mut Pattern) {
        match p {
            Pattern::Literal(_, _) | Pattern::Wildcard(_) => {}
            Pattern::Binding(i) => self.bind(&i.name),
            Pattern::Constructor { path, args, .. } => {
                self.rewrite_single_segment_path(path);
                for a in args {
                    self.walk_pattern(a);
                }
            }
            Pattern::Tuple(ps, _) => {
                for p in ps {
                    self.walk_pattern(p);
                }
            }
        }
    }

    fn walk_type_expr(&mut self, t: &mut TypeExpr) {
        match t {
            TypeExpr::Primitive(_, _) => {}
            // Phase 2a: rewrite the perspective contract name like
            // any other intra-seed single-name type reference.
            TypeExpr::Perspective { name, .. } => {
                self.rewrite_ident(&mut name.name)
            }
            TypeExpr::Named { path, generic_args, .. } => {
                self.rewrite_single_segment_path(path);
                for g in generic_args {
                    self.walk_type_expr(g);
                }
            }
            TypeExpr::Projection { inner, .. } => self.walk_type_expr(inner),
            TypeExpr::Array { elem, size, .. } => {
                self.walk_type_expr(elem);
                if let Some(s) = size {
                    self.walk_expr(s);
                }
            }
            TypeExpr::Bounded { elem, .. } => self.walk_type_expr(elem),
            TypeExpr::Tuple(ts, _) => {
                for t in ts {
                    self.walk_type_expr(t);
                }
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.walk_type_expr(p);
                }
                if let Some(r) = ret {
                    self.walk_type_expr(r);
                }
            }
        }
    }

    fn walk_expr(&mut self, e: &mut Expr) {
        match e {
            Expr::Literal(_, _) | Expr::KwSelf(_) => {}
            Expr::Ident(i) => self.rewrite_ident(&mut i.name),
            Expr::Path(q) => self.rewrite_single_segment_path(q),
            Expr::Binary { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand),
            Expr::Call { callee, args, .. } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Field { receiver, .. } => self.walk_expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.walk_expr(receiver);
                self.walk_expr(index);
            }
            Expr::Path2 { receiver, .. } => self.walk_expr(receiver),
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                for e in es {
                    self.walk_expr(e);
                }
            }
            Expr::Struct { path, inits, .. } => {
                self.rewrite_single_segment_path(path);
                for i in inits {
                    self.walk_expr(&mut i.value);
                }
            }
            Expr::Block(b) => self.walk_block(b),
            Expr::If(i) => self.walk_if(i),
            Expr::Match(m) => self.walk_match(m),
            Expr::Sum(e, _) | Expr::Prod(e, _) => self.walk_expr(e),
            Expr::Approx { left, right, tolerance, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
                self.walk_expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.walk_expr(lo);
                self.walk_expr(hi);
            }
            Expr::ArrayRepeat { val, .. } => self.walk_expr(val),
            Expr::Or { inner, disposition, .. } => {
                self.walk_expr(inner);
                match disposition {
                    OrDisposition::Substitute(rhs) => self.walk_expr(rhs),
                    // B3 / G6 — `or fail <payload>`: the payload is an
                    // ordinary expression whose type names and bare
                    // free-fn calls must be cross-seed mangled the same
                    // way every other expression position is. Without
                    // this recursion, `... or fail LibError { ... }`
                    // and `... or fail make_err(...)` left their
                    // identifiers unrewritten, so the consumer build
                    // rejected the seed with `unknown type` /
                    // `unknown identifier` after import.
                    OrDisposition::Fail(payload, _) => self.walk_expr(payload),
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn parse(src: &str) -> Program {
        parse_source(src).expect("parse failed")
    }

    fn find_locus<'a>(p: &'a Program, name: &str) -> Option<&'a LocusDecl> {
        p.items.iter().find_map(|d| match d {
            TopDecl::Locus(l) if l.name.name == name => Some(l),
            _ => None,
        })
    }
    fn find_fn<'a>(p: &'a Program, name: &str) -> Option<&'a FnDecl> {
        p.items.iter().find_map(|d| match d {
            TopDecl::Fn(f) if f.name.name == name => Some(f),
            _ => None,
        })
    }

    #[test]
    fn renames_top_level_decls() {
        let src = r#"
            type Point { x: Int; y: Int; }
            locus Greeter { params { } }
            const DEFAULT_GREETING: String = "hi";
            fn greet() { }
            interface Sink { fn write(s: String); }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "toy", "main");
        let names: Vec<String> = prog
            .items
            .iter()
            .filter_map(top_decl_name)
            .map(|s| s.to_string())
            .collect();
        assert!(names.contains(&"__lib_toy_main_Point".to_string()), "names={:?}", names);
        assert!(names.contains(&"__lib_toy_main_Greeter".to_string()));
        assert!(names.contains(&"__lib_toy_main_DEFAULT_GREETING".to_string()));
        assert!(names.contains(&"__lib_toy_main_greet".to_string()));
        assert!(names.contains(&"__lib_toy_main_Sink".to_string()));
    }

    #[test]
    fn rewrites_use_sites_for_struct_literal_and_call() {
        let src = r#"
            type Greeting { msg: String; }
            fn make() -> Greeting {
                return Greeting { msg: "hi" };
            }
            fn caller() {
                let g = make();
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "toy", "greet");

        // The `Greeting { msg: ... }` Expr::Struct path should be rewritten.
        let make_fn = find_fn(&prog, "__lib_toy_greet_make").expect("make renamed");
        match &make_fn.ret {
            Some(TypeExpr::Named { path, .. }) => {
                assert_eq!(path.segments[0].name, "__lib_toy_greet_Greeting");
            }
            other => panic!("unexpected ret type: {:?}", other),
        }
        // Find the Return in body; it's `Stmt::Return(Some(Expr::Struct{...}))`.
        let ret_stmt = make_fn.body.stmts.iter().find(|s| matches!(s, Stmt::Return(_, _)));
        match ret_stmt {
            Some(Stmt::Return(Some(Expr::Struct { path, .. }), _)) => {
                assert_eq!(path.segments[0].name, "__lib_toy_greet_Greeting");
            }
            other => panic!("expected Struct return, got {:?}", other),
        }

        // The `make()` call site should be rewritten.
        let caller = find_fn(&prog, "__lib_toy_greet_caller").expect("caller renamed");
        let let_stmt = &caller.body.stmts[0];
        match let_stmt {
            Stmt::Let { value: Expr::Call { callee, .. }, .. } => match callee.as_ref() {
                Expr::Ident(i) => assert_eq!(i.name, "__lib_toy_greet_make"),
                other => panic!("call callee not Ident: {:?}", other),
            },
            other => panic!("expected let stmt: {:?}", other),
        }
    }

    #[test]
    fn leaves_outside_seed_references_alone() {
        // `println` is a builtin (not in our seed); `Int`/`String`
        // are primitives. Neither should be rewritten.
        let src = r#"
            fn driver() {
                let x: Int = 1;
                let s: String = "hi";
                println(s);
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "toy", "main");
        // `println` should NOT have been rewritten — it's a builtin
        // call, not a seed top-level fn.
        let driver = find_fn(&prog, "__lib_toy_main_driver").expect("driver renamed");
        let last_stmt = driver.body.stmts.last().expect("stmts non-empty");
        match last_stmt {
            Stmt::Expr(Expr::Call { callee, .. }) => match callee.as_ref() {
                Expr::Ident(i) => assert_eq!(i.name, "println"),
                other => panic!("println callee not Ident: {:?}", other),
            },
            other => panic!("expected println call: {:?}", other),
        }
    }

    #[test]
    fn local_shadows_top_level_name() {
        // A local named `greet` should NOT be rewritten when it
        // shadows the top-level fn `greet`.
        let src = r#"
            fn greet() { }
            fn caller() {
                let greet = 1;
                let n = greet;
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "toy", "main");
        // The top-level `greet` fn becomes `__lib_toy_main_greet`.
        assert!(find_fn(&prog, "__lib_toy_main_greet").is_some());
        // Inside `caller`, the let-binding `greet` and the
        // subsequent Ident reference should NOT be rewritten.
        let caller = find_fn(&prog, "__lib_toy_main_caller").expect("caller renamed");
        // Second statement: `let n = greet;`
        match &caller.body.stmts[1] {
            Stmt::Let { value: Expr::Ident(i), .. } => {
                assert_eq!(i.name, "greet", "local should shadow top-level");
            }
            other => panic!("unexpected stmt 2: {:?}", other),
        }
    }

    #[test]
    fn capacity_slot_as_parent_for_rewrites() {
        let src = r#"
            type Item { v: Int; }
            locus Child {
                capacity { pool entries of Item; }
            }
            locus Parent {
                capacity { pool entries of Item as_parent_for Child; }
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "toy", "main");
        let parent = find_locus(&prog, "__lib_toy_main_Parent").expect("Parent renamed");
        let cap = parent.members.iter().find_map(|m| match m {
            LocusMember::Capacity(c) => Some(c),
            _ => None,
        }).expect("capacity block");
        let slot = &cap.slots[0];
        // elem_ty TypeExpr::Named { path: ["Item"] } → __lib_toy_main_Item
        match &slot.elem_ty {
            TypeExpr::Named { path, .. } => {
                assert_eq!(path.segments[0].name, "__lib_toy_main_Item");
            }
            other => panic!("unexpected elem_ty: {:?}", other),
        }
        // as_parent_for → __lib_toy_main_Child
        assert_eq!(slot.as_parent_for.as_ref().unwrap().name, "__lib_toy_main_Child");
    }

    #[test]
    fn rewrites_topic_ref_in_publish_and_subscribe() {
        // A1 (G1/G32) regression. A seed that declares a topic and
        // publishes / subscribes to it via the topic-ref form must
        // have BOTH the decl and the bus-member references rewritten.
        let src = r#"
            type Tick { n: Int; }
            topic Ticks { payload: Tick; }
            locus Pub {
                bus { publish Ticks; }
            }
            locus Sub {
                bus { subscribe Ticks as on_tick; }
                fn on_tick(t: Tick) { }
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "toy", "main");

        // The topic decl renames.
        let topic_renamed = prog.items.iter().any(|d| matches!(d,
            TopDecl::Topic(t) if t.name.name == "__lib_toy_main_Ticks"));
        assert!(topic_renamed, "topic decl should be mangled");

        // The publish-site subject ident renames.
        let pub_locus = find_locus(&prog, "__lib_toy_main_Pub").expect("Pub renamed");
        let pub_bus = pub_locus.members.iter().find_map(|m| match m {
            LocusMember::Bus(b) => Some(b),
            _ => None,
        }).expect("Pub.bus");
        match &pub_bus.members[0] {
            BusMember::Publish { subject: BusSubject::Topic(i), .. } => {
                assert_eq!(i.name, "__lib_toy_main_Ticks",
                    "publish topic ident should be mangled");
            }
            other => panic!("expected Publish topic-ref, got {:?}", other),
        }

        // The subscribe-site subject ident renames.
        let sub_locus = find_locus(&prog, "__lib_toy_main_Sub").expect("Sub renamed");
        let sub_bus = sub_locus.members.iter().find_map(|m| match m {
            LocusMember::Bus(b) => Some(b),
            _ => None,
        }).expect("Sub.bus");
        match &sub_bus.members[0] {
            BusMember::Subscribe { subject: BusSubject::Topic(i), .. } => {
                assert_eq!(i.name, "__lib_toy_main_Ticks",
                    "subscribe topic ident should be mangled");
            }
            other => panic!("expected Subscribe topic-ref, got {:?}", other),
        }
    }

    #[test]
    fn literal_bus_subject_not_rewritten() {
        // Defensive: legacy literal-string subjects must not be
        // touched — they're already wire-format strings and would
        // not collide with anything the mangler cares about.
        let src = r#"
            type Event { msg: String; }
            locus Logger {
                bus {
                    subscribe "log.error" as on_err of type Event;
                    publish "log.info" of type Event;
                }
                fn on_err(e: Event) { }
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "toy", "main");
        let locus = find_locus(&prog, "__lib_toy_main_Logger").expect("Logger renamed");
        let bus = locus.members.iter().find_map(|m| match m {
            LocusMember::Bus(b) => Some(b),
            _ => None,
        }).expect("bus block");
        // Both members should still carry literal subjects unchanged.
        match &bus.members[0] {
            BusMember::Subscribe { subject: BusSubject::Literal { subject, .. }, .. } => {
                assert_eq!(subject, "log.error");
            }
            other => panic!("expected literal subscribe, got {:?}", other),
        }
        match &bus.members[1] {
            BusMember::Publish { subject: BusSubject::Literal { subject, .. }, .. } => {
                assert_eq!(subject, "log.info");
            }
            other => panic!("expected literal publish, got {:?}", other),
        }
    }

    #[test]
    fn rewrites_struct_type_in_or_fail_payload() {
        // Cross-seed regression: `... or fail StructName { ... }` —
        // the payload expression's struct-literal type name must be
        // rewritten by the mangler. Before the fix, `OrDisposition::
        // Fail` was a no-op in `walk_expr`, so the consumer build
        // rejected the seed with `unknown type StructName`.
        let src = r#"
            type LibError { kind: String = ""; detail: String = ""; }
            fn raise_it() -> Int fallible(LibError) {
                fail LibError { kind: "boom", detail: "" };
            }
            fn wrap_it() -> Int fallible(LibError) {
                let _x = raise_it() or fail LibError { kind: "wrap", detail: "" };
                return 0;
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "errlib", "err");

        let wrap = find_fn(&prog, "__lib_errlib_err_wrap_it").expect("wrap_it renamed");
        // First stmt: `let _x = (raise_it() or fail LibError { ... });`
        // Pull the Or expr out and inspect the disposition payload.
        let let_value = match &wrap.body.stmts[0] {
            Stmt::Let { value, .. } => value,
            other => panic!("expected Let, got {:?}", other),
        };
        let dispo = match let_value {
            Expr::Or { disposition, .. } => disposition,
            other => panic!("expected Or expr, got {:?}", other),
        };
        match dispo {
            OrDisposition::Fail(payload, _) => match payload.as_ref() {
                Expr::Struct { path, .. } => {
                    assert_eq!(
                        path.segments[0].name,
                        "__lib_errlib_err_LibError",
                        "or-fail struct payload type should be mangled"
                    );
                }
                other => panic!("expected Struct payload, got {:?}", other),
            },
            other => panic!("expected OrDisposition::Fail, got {:?}", other),
        }
    }

    #[test]
    fn rewrites_free_fn_call_in_or_fail_payload() {
        // Companion to `rewrites_struct_type_in_or_fail_payload`:
        // a bare free-fn call inside an `or fail` payload (e.g.
        // `... or fail make_err("x")`) must also have its callee
        // ident rewritten when the fn lives in the same seed.
        let src = r#"
            type LibError { kind: String = ""; detail: String = ""; }
            fn raise_it() -> Int fallible(LibError) {
                fail LibError { kind: "boom", detail: "" };
            }
            fn make_err(k: String) -> LibError {
                return LibError { kind: k, detail: "" };
            }
            fn wrap_it() -> Int fallible(LibError) {
                let _x = raise_it() or fail make_err("via_call");
                return 0;
            }
        "#;
        let mut prog = parse(src);
        mangle_program(&mut prog, "errlib", "err");

        let wrap = find_fn(&prog, "__lib_errlib_err_wrap_it").expect("wrap_it renamed");
        let let_value = match &wrap.body.stmts[0] {
            Stmt::Let { value, .. } => value,
            other => panic!("expected Let, got {:?}", other),
        };
        let dispo = match let_value {
            Expr::Or { disposition, .. } => disposition,
            other => panic!("expected Or expr, got {:?}", other),
        };
        match dispo {
            OrDisposition::Fail(payload, _) => match payload.as_ref() {
                Expr::Call { callee, .. } => match callee.as_ref() {
                    Expr::Ident(i) => assert_eq!(
                        i.name,
                        "__lib_errlib_err_make_err",
                        "or-fail call callee should be mangled"
                    ),
                    other => panic!("expected Ident callee, got {:?}", other),
                },
                other => panic!("expected Call payload, got {:?}", other),
            },
            other => panic!("expected OrDisposition::Fail, got {:?}", other),
        }
    }

    #[test]
    fn empty_program_no_op() {
        let mut prog = parse("");
        mangle_program(&mut prog, "toy", "main");
        assert!(prog.items.is_empty());
    }
}
