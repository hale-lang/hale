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
//!     in, sans `.ap` — so two files in the same library can share a
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

use aperio_syntax::ast::*;

/// Rewrite `prog` in place so its top-level decls and any
/// intra-seed references carry the `__lib_<alias>_<file_stem>_*`
/// prefix. `file_stem` is the basename of the source file the
/// program was parsed from, without the `.ap` extension.
pub fn mangle_program(prog: &mut Program, alias: &str, file_stem: &str) {
    let mut renames: HashMap<String, String> = HashMap::new();
    for item in &prog.items {
        if let Some(n) = top_decl_name(item) {
            renames.insert(n.to_string(), mangled(alias, file_stem, n));
        }
    }
    if renames.is_empty() {
        return;
    }
    let mut walker = Mangler {
        renames: &renames,
        scopes: Vec::new(),
    };
    for item in &mut prog.items {
        walker.walk_top_decl(item);
    }
}

fn mangled(alias: &str, file_stem: &str, name: &str) -> String {
    format!("__lib_{}_{}_{}", alias, file_stem, name)
}

fn top_decl_name(d: &TopDecl) -> Option<&str> {
    match d {
        TopDecl::Locus(l) => Some(&l.name.name),
        TopDecl::Perspective(p) => Some(&p.name.name),
        TopDecl::Type(t) => Some(&t.name.name),
        TopDecl::Const(c) => Some(&c.name.name),
        TopDecl::Fn(f) => Some(&f.name.name),
        TopDecl::Interface(i) => Some(&i.name.name),
        TopDecl::Module(_) => None,
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
            TopDecl::Module(_) => {}
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
                        BusMember::Subscribe { handler, ty, .. } => {
                            // Handler resolves against top-level fns in
                            // the seed; rewrite if it's one of ours.
                            self.rewrite_ident(&mut handler.name);
                            if let Some(t) = ty {
                                self.walk_type_expr(t);
                            }
                        }
                        BusMember::Publish { ty, .. } => self.walk_type_expr(ty),
                    }
                }
            }
            LocusMember::Lifecycle(lc) => self.walk_fn_like(&mut lc.params, lc.ret.as_mut(), &mut lc.body),
            LocusMember::Mode(md) => self.walk_fn_like(&mut md.params, md.ret.as_mut(), &mut md.body),
            LocusMember::Failure(f) => self.walk_fn_like(&mut f.params, None, &mut f.body),
            LocusMember::Closure(c) => {
                self.walk_expr(&mut c.assertion.left);
                self.walk_expr(&mut c.assertion.right);
                self.walk_expr(&mut c.assertion.tolerance);
                for cl in &mut c.clauses {
                    if let ClosureClause::Epoch(EpochSpec::Duration(e)) = cl {
                        self.walk_expr(e);
                    }
                }
            }
            LocusMember::Fn(f) => self.walk_fn_decl(f),
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

    fn walk_fn_decl(&mut self, f: &mut FnDecl) {
        self.rewrite_ident(&mut f.name.name);
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
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) => {}
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
            Stmt::Send { subject, value, .. } => {
                self.walk_expr(subject);
                self.walk_expr(value);
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
                if let OrDisposition::Substitute(rhs) = disposition {
                    self.walk_expr(rhs);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aperio_syntax::parse_source;

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
    fn empty_program_no_op() {
        let mut prog = parse("");
        mangle_program(&mut prog, "toy", "main");
        assert!(prog.items.is_empty());
    }
}
