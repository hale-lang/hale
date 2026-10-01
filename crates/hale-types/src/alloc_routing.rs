//! The allocation-routing rows of the lowering view (F.40 phase 3, E3a):
//! the facts codegen reads when it decides which arena an allocation
//! lands in, derived here once over the program lowering walks
//! ([`crate::resolved::LoweringView::merged`]) and read, never
//! re-derived, by lowering.
//!
//! [`derive_alloc_routing`] produces them; [`AllocRouting`] holds:
//!
//! - `scratch_local`: the free fns whose allocations cannot outlive
//!   the call except through the return value (GH #1148).
//! - `nonalloc`: the free fns that provably allocate nothing (FORM-3),
//!   with the numeric-returning subset.
//! - `elidable_methods`: per locus, the `fn` methods and modes whose
//!   `self.m()` call allocates nothing (stage 2 of method-scratch
//!   elision).
//! - `loci`: per locus, whether its arena is elided, and per lifecycle
//!   hook, `fn` method and mode, whether it lowers without its per-call
//!   scratch ([`LocusElision`]).
//!
//! A generic locus's monomorph is synthesized by lowering, so the view
//! has no row for it: lowering asks [`AllocRouting::specialize`], the
//! same producer over the synthesized declaration.
//!
//! The rows are the classifications codegen computed while it lowered,
//! moved as they were: the checker's reclaim model
//! (`crate::alloc_summary::ReclaimScope`) does not read them yet, and
//! the disagreement the registry records for it stands until it does.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;

/// One locus's elision rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocusElision {
    /// The locus's arena is elided at instantiation: `__arena` is the
    /// caller's current arena, and dissolve destroys none.
    pub arena: bool,
    /// By position in the locus's `members`: whether that lifecycle
    /// hook, `fn` method or mode lowers without its per-call scratch
    /// subregion (stage-1 method-scratch elision).
    pub scratch: BTreeMap<usize, bool>,
}

/// The allocation-routing rows over one program.
#[derive(Debug, Clone, Default)]
pub struct AllocRouting {
    /// The scratch-local free fns, by the name the call site spells
    /// ([`scratch_local_free_fns`]).
    pub scratch_local: BTreeSet<String>,
    /// The FORM-3 non-allocating free fns ([`nonalloc_free_fns`]): each
    /// skips its per-call scratch arena, and a call to one allocates
    /// nothing.
    pub nonalloc: BTreeSet<String>,
    /// Of `nonalloc`, the fns returning a numeric scalar.
    pub nonalloc_numeric_ret: BTreeSet<String>,
    /// Stage 2 of method-scratch elision ([`elidable_methods`]): per
    /// locus name, the elidable `fn` methods and modes, and the
    /// numeric-returning subset. A monomorph has no entry.
    pub elidable_methods: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>,
    /// Per non-generic locus of the program, by name.
    pub loci: BTreeMap<String, LocusElision>,
    /// What [`AllocRouting::specialize`] reads beside the rows.
    types: TypeFacts,
}

/// The program's type facts the elision rows read: what a field's
/// declared type denotes, as lowering's own type mapping resolves it.
#[derive(Debug, Clone, Default)]
struct TypeFacts {
    /// `type Name = T;` targets by name, for every non-generic alias,
    /// later declarations overriding earlier ones.
    aliases: BTreeMap<String, TypeExpr>,
    /// The cross-seed rename table, keyed by the path a use spells.
    imports: BTreeMap<Vec<String>, String>,
    /// Every flat `type` record's numeric-scalar fields
    /// ([`struct_numeric_field_map`]).
    structs: BTreeMap<String, BTreeSet<String>>,
}

impl AllocRouting {
    /// Whether the free fn `name` is scratch-local.
    pub fn is_scratch_local(&self, name: &str) -> bool {
        self.scratch_local.contains(name)
    }

    /// Whether the free fn `name` allocates nothing.
    pub fn is_nonalloc(&self, name: &str) -> bool {
        self.nonalloc.contains(name)
    }

    /// The rows of a locus the view does not hold: a generic locus's
    /// monomorph, which lowering synthesizes. The same producer as the
    /// view's rows, over the synthesized declaration; a monomorph has no
    /// stage-2 set, so a `self.m()` call in its bodies stays
    /// conservative, as it always has.
    pub fn specialize(&self, l: &LocusDecl) -> LocusElision {
        locus_elision(l, self, &self.types)
    }
}

/// The elision rows of one locus: its arena verdict
/// ([`locus_arena_elidable`]) and every lifecycle hook's, `fn`
/// method's and mode's scratch verdict ([`member_scratch_elidable`]).
fn locus_elision(l: &LocusDecl, rows: &AllocRouting, types: &TypeFacts) -> LocusElision {
    let (numeric_self_fields, scalar_self_fields) = self_field_classes(l, types);
    let empty_methods = BTreeSet::new();
    let (elidable_self_methods, numeric_ret_self_methods) = rows
        .elidable_methods
        .get(&l.name.name)
        .map(|(e, n)| (e, n))
        .unwrap_or((&empty_methods, &empty_methods));
    let mut scratch = BTreeMap::new();
    for (i, m) in l.members.iter().enumerate() {
        let (body, params, ret) = match m {
            LocusMember::Lifecycle(lc) => (&lc.body, &lc.params, lc.ret.as_ref()),
            LocusMember::Fn(fd) => (&fd.body, &fd.params, fd.ret.as_ref()),
            LocusMember::Mode(md) => (&md.body, &md.params, md.ret.as_ref()),
            _ => continue,
        };
        let ctx = MemberCtx {
            nonalloc: &rows.nonalloc,
            numeric_ret: &rows.nonalloc_numeric_ret,
            numeric_self_fields: &numeric_self_fields,
            scalar_self_fields: &scalar_self_fields,
            elidable_self_methods,
            numeric_ret_self_methods,
            structs: &types.structs,
        };
        scratch.insert(i, member_scratch_elidable(body, params, ret, &ctx));
    }
    LocusElision { arena: locus_arena_elidable(l), scratch }
}

/// What a member's scratch verdict reads beside its own declaration.
struct MemberCtx<'a> {
    nonalloc: &'a BTreeSet<String>,
    numeric_ret: &'a BTreeSet<String>,
    numeric_self_fields: &'a BTreeSet<String>,
    scalar_self_fields: &'a BTreeSet<String>,
    elidable_self_methods: &'a BTreeSet<String>,
    numeric_ret_self_methods: &'a BTreeSet<String>,
    structs: &'a BTreeMap<String, BTreeSet<String>>,
}

/// Method-scratch elision (stage 1, 2026-06-28): true iff this
/// method's per-call scratch subregion can be skipped entirely —
/// its body provably allocates nothing AND its return value is a
/// by-value scalar (or Unit), so no return deep-copy is needed.
/// When true the caller skips `open_method_scratch`, leaving
/// `current_method_scratch` None: allocations (there are none) would
/// fall through to `self.__arena`, destroy/close become no-ops, and
/// the return path takes the no-deep-copy "store the pointer
/// directly" branch (correct precisely because the body builds no
/// fresh heap value to return).
///
/// Conservative: false-negatives just keep a redundant scratch;
/// a false-positive that skips a needed scratch/deep-copy is a
/// memory bug, so both gates must hold.
///   1. `ret` is `None` or a scalar primitive (Int/Uint/Float/
///      Bool/Duration). Any heap return (String/Bytes/struct/array)
///      keeps the scratch — eliding would dangle the return alias.
///   2. The body is non-allocating under the method-aware
///      classifier (self numeric fields → arithmetic, scalar
///      self-field writes → by-value).
fn member_scratch_elidable(
    body: &Block,
    params: &[Param],
    ret: Option<&TypeExpr>,
    cx: &MemberCtx,
) -> bool {
    // Gate 1: by-value scalar (or Unit) return only.
    match ret {
        None => {}
        Some(TypeExpr::Primitive(
            PrimType::Int
            | PrimType::Uint
            | PrimType::Float
            | PrimType::Bool
            | PrimType::Duration,
            _,
        )) => {}
        Some(_) => return false,
    }
    // Gate 2: method-aware non-allocating body, seeded with the
    // locus's self-field classes.
    let param_seed: BTreeSet<String> = params
        .iter()
        .filter(|p| type_expr_is_numeric_scalar(&p.ty))
        .map(|p| p.name.name.clone())
        .collect();
    // Scalar-param-field facts (2026-06-30): a read of a numeric-scalar
    // field of a struct-typed param (`s.value` for `s: Sample`) is a
    // non-allocating numeric scalar, so `self.sum + s.value` is
    // arithmetic — exactly what lets a quiet handler like
    // `Aggregator.on_sample(s: Sample)` drop its per-call scratch. Built
    // from the same whole-program `type` map the fixpoints use.
    let param_field_numeric = param_field_numeric_map(params, cx.structs);
    // Stage 2: the same-locus elidable-method sets, so a `self.m(args)`
    // call in this body resolves as non-allocating exactly when `m`
    // reached the elidable fixpoint.
    let fnptr_numeric_ret = fnptr_numeric_param_set(params);
    let ctx = AllocCtx {
        nonalloc: cx.nonalloc,
        numeric_ret: cx.numeric_ret,
        numeric_self_fields: cx.numeric_self_fields,
        scalar_self_fields: cx.scalar_self_fields,
        elidable_self_methods: cx.elidable_self_methods,
        numeric_ret_self_methods: cx.numeric_ret_self_methods,
        param_field_numeric: &param_field_numeric,
        fnptr_numeric_ret: &fnptr_numeric_ret,
    };
    fn_body_definitely_non_allocating(&body.stmts, &ctx, &param_seed)
}

/// A field's scalar class: whether lowering stores it as a numeric
/// scalar (`Int`, `Float`, `Duration`: `self.x + 1` is arithmetic) or
/// as a `Bool` (a by-value store), or as neither.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScalarClass {
    Numeric,
    Bool,
}

/// The self-field classes of `l` (numeric, then scalar: numeric plus
/// `Bool`), from its `params { }` blocks, each field typed as lowering
/// lays it out: a literal default by its literal, anything else by its
/// ascription, through `type` aliases and the rename table.
fn self_field_classes(l: &LocusDecl, types: &TypeFacts) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut numeric = BTreeSet::new();
    let mut scalar = BTreeSet::new();
    for m in &l.members {
        let LocusMember::Params(pb) = m else { continue };
        for p in &pb.params {
            let literal = match &p.init {
                ParamInit::Value(e) => literal_class(e),
                ParamInit::Inferred => None,
            };
            let class = match literal {
                Some(c) => c,
                None => p.ty.as_ref().and_then(|t| type_class(t, types, &mut Vec::new())),
            };
            match class {
                Some(ScalarClass::Numeric) => {
                    numeric.insert(p.name.name.clone());
                    scalar.insert(p.name.name.clone());
                }
                Some(ScalarClass::Bool) => {
                    scalar.insert(p.name.name.clone());
                }
                None => {}
            }
        }
    }
    (numeric, scalar)
}

/// A literal default's class, `Some(None)` for a literal of no scalar
/// class (a String, Decimal or Time), `None` when the default is not a
/// literal and the field takes its ascription's type.
fn literal_class(e: &Expr) -> Option<Option<ScalarClass>> {
    match e {
        Expr::Literal(Literal::Int(_) | Literal::Float(_) | Literal::Duration(_), _) => {
            Some(Some(ScalarClass::Numeric))
        }
        Expr::Literal(Literal::Bool(_), _) => Some(Some(ScalarClass::Bool)),
        Expr::Literal(Literal::String(_) | Literal::Decimal(_) | Literal::Time(_), _) => {
            Some(None)
        }
        _ => None,
    }
}

/// A declared type's class, resolved through non-generic `type`
/// aliases (a single-segment name, or a qualified one through the
/// stdlib's path table and then the rename table); an alias chain
/// that comes back to itself denotes no scalar.
fn type_class(t: &TypeExpr, types: &TypeFacts, seen: &mut Vec<String>) -> Option<ScalarClass> {
    let alias = |name: &str, seen: &mut Vec<String>| -> Option<ScalarClass> {
        if seen.iter().any(|s| s == name) {
            return None;
        }
        let target = types.aliases.get(name)?;
        seen.push(name.to_string());
        type_class(target, types, seen)
    };
    match t {
        TypeExpr::Primitive(PrimType::Int | PrimType::Float | PrimType::Duration, _) => {
            Some(ScalarClass::Numeric)
        }
        TypeExpr::Primitive(PrimType::Bool, _) => Some(ScalarClass::Bool),
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && path.segments.len() == 1 =>
        {
            alias(&path.segments[0].name, seen)
        }
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && path.segments.len() > 1 =>
        {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            let mangled = match crate::ownership::stdlib_mangled_for_path(&segs) {
                Some(name) => name.to_string(),
                None => {
                    let key: Vec<String> = segs.iter().map(|s| s.to_string()).collect();
                    types.imports.get(&key)?.clone()
                }
            };
            alias(&mangled, seen)
        }
        _ => None,
    }
}

/// The rows of `program` (the view's `merged`), cross-seed calls
/// resolved through `import_renames`, the rename table the view was
/// resolved with.
pub fn derive_alloc_routing(
    program: &Program,
    import_renames: &[(Vec<String>, String)],
) -> AllocRouting {
    let imports: BTreeMap<Vec<String>, String> = import_renames
        .iter()
        .map(|(segs, mangled)| (segs.clone(), mangled.clone()))
        .collect();
    let (nonalloc, nonalloc_numeric_ret) = nonalloc_free_fns(&program.items);
    let elidable = elidable_methods(&program.items, &nonalloc, &nonalloc_numeric_ret);
    let mut aliases = BTreeMap::new();
    for item in hale_syntax::ast::flat_decls(&program.items) {
        if let TopDecl::Type(t) = item {
            if let TypeDeclBody::Alias(te) = &t.body {
                if t.generics.is_empty() {
                    aliases.insert(t.name.name.clone(), te.clone());
                }
            }
        }
    }
    let types = TypeFacts {
        aliases,
        imports: imports.clone(),
        structs: struct_numeric_field_map(&program.items),
    };
    let mut rows = AllocRouting {
        scratch_local: scratch_local_free_fns(&program.items, &imports),
        nonalloc,
        nonalloc_numeric_ret,
        elidable_methods: elidable,
        loci: BTreeMap::new(),
        types,
    };
    let loci: BTreeMap<String, LocusElision> = hale_syntax::ast::flat_decls(&program.items)
        .filter_map(|item| match item {
            TopDecl::Locus(l) if l.generics.is_empty() => {
                Some((l.name.name.clone(), locus_elision(l, &rows, &rows.types)))
            }
            _ => None,
        })
        .collect();
    rows.loci = loci;
    rows
}

/// GH #1148: the free fns whose allocations provably cannot outlive the
/// call except through the return value — "scratch-local" fns. Their body
/// allocates into the fn's own m49 subregion (destroyed at return, after
/// the epilogue deep-copies the return value into the caller's arena),
/// not into the caller's arena, which every other free fn uses because
/// codegen has no general escape analysis (see `current_arena_ptr`).
///
/// Why it matters: a helper that walks a String by re-slicing a
/// loop-carried local — `rest = rest[(nl + 1)..len(rest)]`, the shape of
/// every line scanner — makes one fresh suffix copy per iteration. In
/// the caller's arena none of them is reclaimed until the CALLER's scope
/// ends, so a method that calls the helper once per line holds a cubic
/// amount of garbage at once: a composed API head scanning a ~600-row
/// record grew past its 512 MiB address-space bound in one command and
/// died on the NULL the exhausted allocator returned (downstream
/// handoff). In the fn's own subregion the same garbage lives for one
/// call.
///
/// The class is a greatest fixpoint over a deliberately narrow shape, so
/// that nothing the body allocates can be reachable from anywhere but the
/// return value:
///   * signature: not fallible, not FFI, not generic, not exported; every
///     param and the return (if any) a by-value scalar or a String — no
///     struct, locus, Bytes, array or fn pointer can carry a pointer in
///     or out;
///   * body: no struct or locus literal, no method call, no publish, no
///     `self`; assignments only to bare locals; calls only to other fns
///     of the class (by name, by import path, or by a `std::` path that
///     names a Hale-source stdlib fn), to the value-returning builtins in
///     `SCRATCH_LOCAL_BUILTINS`, or to runtime primitives in the pure
///     `std::` namespaces of `SCRATCH_LOCAL_STD_NAMESPACES`.
/// Anything else demotes the fn to the caller-arena default.
///
/// A Hale-source stdlib fn is held to the class like any other fn, not
/// waved through by its namespace: `std::str::bytes_view` returns a
/// struct carrying its argument, and that struct is deep-copied into the
/// calling fn's arena. In the caller's arena the copy of a String that
/// already lives there is skipped (`lotus_str_clone`'s same-arena
/// passthrough); in a fresh subregion it is a real copy of the whole
/// text per call.
const SCRATCH_LOCAL_STD_NAMESPACES: &[&str] =
    &["str", "math", "json", "crypto", "env"];

/// Builtin callees (bare identifiers) that return a fresh value or
/// nothing and retain no argument.
const SCRATCH_LOCAL_BUILTINS: &[&str] = &[
    "len", "to_string", "min", "max", "abs", "Int", "Float", "println",
    "eprintln", "print", "eprint",
];

fn type_expr_is_scratch_value(ty: &TypeExpr) -> bool {
    matches!(
        ty,
        TypeExpr::Primitive(
            PrimType::Int
                | PrimType::Uint
                | PrimType::Float
                | PrimType::Bool
                | PrimType::Duration
                | PrimType::String,
            _,
        )
    )
}

/// What a qualified call path names, for the scratch-local classifier:
/// the fn a stdlib or import path is renamed to, or `None` for a runtime
/// primitive (a `std::` path with no Hale-source body).
struct ScratchPaths<'a> {
    imports: &'a BTreeMap<Vec<String>, String>,
}

impl ScratchPaths<'_> {
    fn call_ok(&self, q: &hale_syntax::ast::QualifiedName, set: &BTreeSet<String>) -> bool {
        let segs: Vec<String> = q.segments.iter().map(|s| s.name.clone()).collect();
        if let Some(target) = self.imports.get(&segs) {
            return set.contains(target);
        }
        if let Some((_, target)) = hale_stdlib::PATH_RENAMES
            .iter()
            .find(|(p, _)| p.len() == segs.len() && p.iter().zip(&segs).all(|(a, b)| a == b))
        {
            return set.contains(*target);
        }
        segs.len() >= 3
            && segs[0] == "std"
            && SCRATCH_LOCAL_STD_NAMESPACES.contains(&segs[1].as_str())
    }
}

fn scratch_local_free_fns(
    items: &[TopDecl],
    imports: &BTreeMap<Vec<String>, String>,
) -> BTreeSet<String> {
    let paths = ScratchPaths { imports };
    let fns: Vec<&FnDecl> = hale_syntax::ast::flat_decls(items)
        .filter_map(|it| match it {
            TopDecl::Fn(f)
                if f.fallible.is_none()
                    && f.ffi.is_none()
                    && !f.export
                    && f.generics.is_empty()
                    && f.params.iter().all(|p| type_expr_is_scratch_value(&p.ty))
                    && f.ret.as_ref().map_or(true, type_expr_is_scratch_value) =>
            {
                Some(f)
            }
            _ => None,
        })
        .collect();
    let mut set: BTreeSet<String> =
        fns.iter().map(|f| f.name.name.clone()).collect();
    loop {
        let demote: Vec<String> = fns
            .iter()
            .filter(|f| set.contains(&f.name.name))
            .filter(|f| !scratch_local_block(&f.body, &set, &paths))
            .map(|f| f.name.name.clone())
            .collect();
        if demote.is_empty() {
            return set;
        }
        for n in demote {
            set.remove(&n);
        }
    }
}

fn scratch_local_block(b: &Block, set: &BTreeSet<String>, paths: &ScratchPaths) -> bool {
    b.stmts.iter().all(|s| scratch_local_stmt(s, set, paths))
        && b.tail.as_deref().map_or(true, |e| scratch_local_expr(e, set, paths))
}

fn scratch_local_if(i: &IfStmt, set: &BTreeSet<String>, paths: &ScratchPaths) -> bool {
    scratch_local_expr(&i.cond, set, paths)
        && scratch_local_block(&i.then_block, set, paths)
        && match i.else_block.as_deref() {
            None => true,
            Some(ElseBranch::Else(b)) => scratch_local_block(b, set, paths),
            Some(ElseBranch::ElseIf(inner)) => scratch_local_if(inner, set, paths),
        }
}

fn scratch_local_stmt(s: &Stmt, set: &BTreeSet<String>, paths: &ScratchPaths) -> bool {
    match s {
        Stmt::Let { value, .. } => scratch_local_expr(value, set, paths),
        Stmt::Assign { target, value, .. } => {
            target.head.name != "self"
                && target.tail.is_empty()
                && scratch_local_expr(value, set, paths)
        }
        Stmt::Return(e, _) => {
            e.as_ref().map_or(true, |e| scratch_local_expr(e, set, paths))
        }
        Stmt::Expr(e) => scratch_local_expr(e, set, paths),
        Stmt::If(i) => scratch_local_if(i, set, paths),
        Stmt::While { cond, body, .. } => {
            scratch_local_expr(cond, set, paths) && scratch_local_block(body, set, paths)
        }
        Stmt::For { iter, body, .. } => {
            matches!(iter, Expr::Range { .. })
                && scratch_local_expr(iter, set, paths)
                && scratch_local_block(body, set, paths)
        }
        Stmt::Block(b) => scratch_local_block(b, set, paths),
        Stmt::Break(_) | Stmt::Continue(_) => true,
        _ => false,
    }
}

fn scratch_local_expr(e: &Expr, set: &BTreeSet<String>, paths: &ScratchPaths) -> bool {
    match e {
        Expr::Literal(_, _) | Expr::Ident(_) => true,
        Expr::Field { receiver, .. } => {
            !matches!(receiver.as_ref(), Expr::KwSelf(_))
                && scratch_local_expr(receiver, set, paths)
        }
        Expr::Index { receiver, index, .. } => {
            scratch_local_expr(receiver, set, paths) && scratch_local_expr(index, set, paths)
        }
        Expr::Range { lo, hi, .. } => {
            scratch_local_expr(lo, set, paths) && scratch_local_expr(hi, set, paths)
        }
        Expr::Unary { operand, .. } => scratch_local_expr(operand, set, paths),
        Expr::Binary { left, right, .. } => {
            scratch_local_expr(left, set, paths) && scratch_local_expr(right, set, paths)
        }
        Expr::Tuple(parts, _) if parts.len() <= 1 => {
            parts.iter().all(|p| scratch_local_expr(p, set, paths))
        }
        Expr::If(i) => scratch_local_if(i, set, paths),
        Expr::Block(b) => scratch_local_block(b, set, paths),
        Expr::Or { inner, disposition, .. } => {
            scratch_local_expr(inner, set, paths)
                && match disposition {
                    OrDisposition::Substitute(sub) => scratch_local_expr(sub, set, paths),
                    OrDisposition::Discard(_) => true,
                    _ => false,
                }
        }
        Expr::Call { callee, args, .. } => {
            let callee_ok = match callee.as_ref() {
                Expr::Ident(id) => {
                    set.contains(&id.name)
                        || SCRATCH_LOCAL_BUILTINS.contains(&id.name.as_str())
                }
                Expr::Path(q) => paths.call_ok(q, set),
                _ => false,
            };
            callee_ok && args.iter().all(|a| scratch_local_expr(a, set, paths))
        }
        _ => false,
    }
}

/// FORM-3 (2026-05-13): syntactic classifier for fn bodies that
/// provably don't allocate. Conservative — false negatives are
/// fine (just leaves the existing subregion wrapping in place);
/// false positives would skip the subregion when an allocation
/// actually happens and the resulting allocation would land in
/// the *caller's* arena rather than a per-call subregion. That's
/// a correctness break, not just a perf bug — so the predicate
/// errs strongly on the side of `false`.
///
/// Safe (returns true): literal values (incl. String — those are
/// global statics, no per-use alloc), identifier reads, KwSelf,
/// field reads, indexed reads on non-range indices, Unary on a
/// safe operand, Binary on two safe operands when the op is
/// numeric (Sub/Mul/Div/Mod/comparisons/bool/bitwise). If with
/// non-allocating arms.
///
/// Unsafe (returns false): Add (could be String concat — no type
/// info at AST walk), function/path/method calls (callee may
/// allocate), struct/tuple/array/array-repeat/f-string literals
/// (arena_alloc'd), match (codegen detail), `or` (the fallible
/// machinery allocs), and anything not explicitly enumerated.
/// Global interprocedural facts for the allocation classifier, computed
/// once by `nonalloc_free_fns` before any fn is lowered.
struct AllocCtx<'a> {
    /// Free fns proven to allocate nothing — their body, INCLUDING every fn
    /// they call, is non-allocating. A call to one of these allocates
    /// nothing, so it no longer forces the caller's per-call scratch arena
    /// (the interprocedural step — modular helper-calls-helper code stops
    /// paying a malloc/free per call).
    pub nonalloc: &'a BTreeSet<String>,
    /// Of `nonalloc`, the fns whose declared return is a non-allocating
    /// numeric scalar — so `helper(x) + 1` (a call result inside an `Add`)
    /// classifies as arithmetic.
    pub numeric_ret: &'a BTreeSet<String>,
    /// Method-elidability only: `self` fields of a non-allocating numeric
    /// scalar type (Int / Uint / Float / Duration), so `self.x + 1` is
    /// arithmetic. EMPTY on the free-fn / locus-arena paths (no `self`).
    pub numeric_self_fields: &'a BTreeSet<String>,
    /// Method-elidability only: scalar (by-value, non-heap) `self` fields —
    /// the numeric set PLUS `Bool`. A write to one of these stores by value
    /// (no deep-copy), so it's non-allocating. EMPTY off the method path.
    pub scalar_self_fields: &'a BTreeSet<String>,
    /// Method-elidability only (stage 2, 2026-06-28): fn-methods of the
    /// CURRENT locus proven elidable — their scratch is skippable, which is
    /// EXACTLY the property that a call `self.m(args)` allocates nothing
    /// (non-allocating body + scalar/Unit return ⇒ no return deep-copy). So a
    /// `self.m(args)` call with non-allocating args allocates nothing. EMPTY
    /// off the method path (free-fn / locus-arena fixpoints).
    pub elidable_self_methods: &'a BTreeSet<String>,
    /// Of `elidable_self_methods`, the subset whose declared return is a
    /// numeric scalar — so `self.helper(x) + 1` classifies as arithmetic.
    /// EMPTY off the method path.
    pub numeric_ret_self_methods: &'a BTreeSet<String>,
    /// Scalar-param-field reads (2026-06-30): a map from an in-scope
    /// PARAM/LOCAL name → the set of that variable's numeric-scalar
    /// (Int/Uint/Float/Duration) field names, resolved from the variable's
    /// declared struct/`type` shape. Lets `s.value` (a scalar field of a
    /// flat struct param `s`) classify as a non-allocating numeric scalar,
    /// so an enclosing `Add` like `self.sum + s.value` is arithmetic rather
    /// than possible String concat. Only numeric-scalar fields are listed —
    /// String/Bytes/Vec/nested-struct/interface fields are deliberately
    /// absent (a read of one is still non-allocating, but it is NOT a
    /// numeric scalar, so it can't make an `Add` arithmetic). DEFAULT-EMPTY
    /// when no param/local type is resolvable — a missing entry just leaves
    /// the read possibly-allocating (the safe default; a false
    /// non-allocating would leak).
    pub param_field_numeric: &'a BTreeMap<String, BTreeSet<String>>,
    /// Fn-pointer PARAMS with a numeric-scalar declared return
    /// (2026-07-02, fn-call protocol shave): a call through one of
    /// these allocates nothing FROM THE CALLER'S PERSPECTIVE — the
    /// callee opens/destroys its own scratch off the threaded
    /// caller arena, and a scalar return means no deep-copy lands
    /// in the caller's arena. (If the callee publishes, the cells
    /// wait for the enclosing cell's next drain point — fn exit is
    /// not a spec-required yield point.) Lets callback-style code
    /// (`fn outer(x: Int, g: fn(Int) -> Int)`) stay elidable
    /// instead of paying subregion + drain per call.
    pub fnptr_numeric_ret: &'a BTreeSet<String>,
}

/// `Int` / `Uint` / `Float` / `Duration` — the scalar types whose `+` is
/// arithmetic (never String concat) and which never allocate.
fn type_expr_is_numeric_scalar(ty: &TypeExpr) -> bool {
    matches!(
        ty,
        TypeExpr::Primitive(
            PrimType::Int | PrimType::Uint | PrimType::Float | PrimType::Duration,
            _,
        )
    )
}

/// Build, from the whole program AST, a map `user-type name → its
/// numeric-scalar field names` (Int/Uint/Float/Duration). Used to recognize
/// `s.value` (a scalar field of a flat-struct param/local `s`) as a
/// non-allocating numeric scalar in the allocation classifier. Walks plain
/// `type T { ... }` data records (the only shape whose field is read with a
/// statically known scalar type by a `.field` GEP+load) and recurses into
/// modules. Non-struct types (aliases, enums) contribute nothing; their
/// fields are never numeric-scalar field reads in this sense.
fn struct_numeric_field_map(
    items: &[TopDecl],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    fn walk(items: &[TopDecl], out: &mut BTreeMap<String, BTreeSet<String>>) {
        for it in items {
            match it {
                TopDecl::Type(td) => {
                    if let TypeDeclBody::Struct(fields) = &td.body {
                        let nf: BTreeSet<String> = fields
                            .iter()
                            .filter(|f| type_expr_is_numeric_scalar(&f.ty))
                            .map(|f| f.name.name.clone())
                            .collect();
                        if !nf.is_empty() {
                            out.insert(td.name.name.clone(), nf);
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    walk(items, &mut out);
    out
}

/// Map a fn/method's params to `param name → numeric-scalar field names of
/// its struct type`, consulting the precomputed `structs` map. Only params
/// whose declared type is a single-segment `Named` referring to a known
/// struct contribute an entry; everything else (primitive, generic, array,
/// tuple, unresolved path) is omitted — a missing entry leaves a `param.field`
/// read possibly-allocating (the conservative default). Locals are not tracked
/// here (params are the resolvable surface); an unascribed `let s = mk()`
/// field read therefore stays conservative.
fn param_field_numeric_map(
    params: &[Param],
    structs: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in params {
        if let TypeExpr::Named { path, generic_args, .. } = &p.ty {
            // Only a bare (non-generic) SINGLE-SEGMENT name maps to a flat
            // user struct — matching how codegen keys `user_types` (by short
            // name). A generic instantiation could be a collection; a
            // path-qualified name could alias a same-named local struct, so
            // both are conservatively skipped (the read stays
            // possibly-allocating).
            if generic_args.is_empty() && path.segments.len() == 1 {
                if let Some(nf) = structs.get(&path.segments[0].name) {
                    out.insert(p.name.name.clone(), nf.clone());
                }
            }
        }
    }
    out
}

/// Compute, by a greatest-fixpoint over the call graph, the set of free fns
/// that allocate nothing (and the subset returning a numeric scalar).
///
/// Candidates are non-fallible (the `fail` path allocs a payload) and
/// non-FFI (opaque C). Start optimistic — assume every candidate is
/// non-allocating — then demote any whose body provably allocates GIVEN the
/// current assumptions, until stable. Optimism lets mutually-recursive
/// numeric fns converge to non-allocating; it's sound because at the
/// fixpoint every remaining fn's body is non-allocating with all of ITS
/// callees non-allocating, so the set contains no allocator.
/// Fn-pointer params whose declared return is a numeric scalar —
/// `g: fn(Int) -> Int` — for the classifier's fnptr_numeric_ret set
/// (2026-07-02 fn-call protocol shave).
fn fnptr_numeric_param_set(params: &[Param]) -> BTreeSet<String> {
    params
        .iter()
        .filter(|p| {
            matches!(
                &p.ty,
                TypeExpr::Function { ret: Some(r), .. }
                    if type_expr_is_numeric_scalar(r)
            )
        })
        .map(|p| p.name.name.clone())
        .collect()
}

fn nonalloc_free_fns(
    items: &[TopDecl],
) -> (BTreeSet<String>, BTreeSet<String>) {
    // GH #884: module nesting flattened — a module-nested free fn
    // is declared and lowered like any other, and the fixpoint is
    // keyed by the bare name the call site spells.
    let fns: Vec<&FnDecl> = hale_syntax::ast::flat_decls(items)
        .filter_map(|it| match it {
            TopDecl::Fn(f) if f.fallible.is_none() && f.ffi.is_none() => Some(f),
            _ => None,
        })
        .collect();
    let numeric_ret_of = |nonalloc: &BTreeSet<String>| -> BTreeSet<String> {
        fns.iter()
            .filter(|f| {
                nonalloc.contains(&f.name.name)
                    && f.ret.as_ref().is_some_and(|t| type_expr_is_numeric_scalar(t))
            })
            .map(|f| f.name.name.clone())
            .collect()
    };
    let structs = struct_numeric_field_map(items);
    let mut nonalloc: BTreeSet<String> =
        fns.iter().map(|f| f.name.name.clone()).collect();
    loop {
        let numeric_ret = numeric_ret_of(&nonalloc);
        let empty_self = BTreeSet::new();
        let demote: Vec<String> = fns
            .iter()
            .filter(|f| nonalloc.contains(&f.name.name))
            .filter(|f| {
                // Per-fn scalar-param-field facts: which of this fn's
                // struct-typed params' fields are numeric scalars.
                let param_field_numeric =
                    param_field_numeric_map(&f.params, &structs);
                let fnptr_numeric_ret = fnptr_numeric_param_set(&f.params);
                let ctx = AllocCtx {
                    nonalloc: &nonalloc,
                    numeric_ret: &numeric_ret,
                    numeric_self_fields: &empty_self,
                    scalar_self_fields: &empty_self,
                    elidable_self_methods: &empty_self,
                    numeric_ret_self_methods: &empty_self,
                    param_field_numeric: &param_field_numeric,
                    fnptr_numeric_ret: &fnptr_numeric_ret,
                };
                // Seed with the fn's numeric scalar params.
                let seed: BTreeSet<String> = f
                    .params
                    .iter()
                    .filter(|p| type_expr_is_numeric_scalar(&p.ty))
                    .map(|p| p.name.name.clone())
                    .collect();
                !fn_body_definitely_non_allocating(&f.body.stmts, &ctx, &seed)
            })
            .map(|f| f.name.name.clone())
            .collect();
        if demote.is_empty() {
            let numeric_ret = numeric_ret_of(&nonalloc);
            return (nonalloc, numeric_ret);
        }
        for n in demote {
            nonalloc.remove(&n);
        }
    }
}

fn fn_body_definitely_non_allocating(
    stmts: &[Stmt],
    ctx: &AllocCtx,
    num: &BTreeSet<String>,
) -> bool {
    // `num` is the set of in-scope locals known to hold a non-allocating
    // numeric scalar (Int / Uint / Float / Duration) — seeded by the
    // caller (a free fn's numeric params) and extended by numeric `let`s
    // as we walk. It's read by the type-aware `Add` classification: `a + b`
    // allocates only as String concat, so when both operands are provably
    // such scalars the `+` is arithmetic and allocates nothing. `ctx`
    // carries the interprocedural facts (which free fns / calls are
    // non-allocating). (2026-06-28 — closes the "factoring out a function
    // costs a malloc/free" penalty: first the type-aware `+`, then calls to
    // proven-non-allocating fns.)
    let mut scope = num.clone();
    for s in stmts {
        if !stmt_definitely_non_allocating(s, ctx, &scope) {
            return false;
        }
        if let Stmt::Let { name, ty, value, .. } = s {
            if let_binds_nonalloc_numeric(ty.as_ref(), value, ctx, &scope) {
                scope.insert(name.name.clone());
            }
        }
    }
    true
}

/// True iff `e` provably has a non-allocating numeric scalar type (Int /
/// Uint / Float / Duration), so an enclosing `Add` is arithmetic, not
/// String concatenation. Conservative — anything not resolvable to a
/// numeric literal, a known-numeric local (in `num`), or numeric
/// arithmetic over those is `false`, leaving the `Add` classified as
/// allocating (the safe default).
fn expr_is_nonalloc_numeric(
    e: &Expr,
    ctx: &AllocCtx,
    num: &BTreeSet<String>,
) -> bool {
    match e {
        Expr::Literal(Literal::Int(_), _)
        | Expr::Literal(Literal::Float(_), _)
        | Expr::Literal(Literal::Duration(_), _) => true,
        Expr::Ident(id) => num.contains(&id.name),
        // Method-elidability: `self.x` where `x` is a numeric scalar self
        // field is itself a numeric scalar — so `self.x + 1` is arithmetic.
        Expr::Field { receiver, name, .. }
            if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
        {
            ctx.numeric_self_fields.contains(&name.name)
        }
        // Scalar-param-field read (2026-06-30): `s.value` where `s` is a
        // PARAM/LOCAL of a flat struct type and `value` is one of that
        // struct's numeric-scalar (Int/Uint/Float/Duration) fields is itself
        // a numeric scalar — so `self.sum + s.value` is arithmetic, not
        // String concat. A scalar field load (GEP+load) never allocates, so
        // this only ever REMOVES a false "possibly-allocating" signal. If the
        // variable's type or the field's type is unresolvable, there's no
        // map entry and the read stays possibly-allocating (the safe default).
        Expr::Field { receiver, name, .. } => match receiver.as_ref() {
            Expr::Ident(id) => ctx
                .param_field_numeric
                .get(&id.name)
                .is_some_and(|nf| nf.contains(&name.name)),
            _ => false,
        },
        Expr::Unary { operand, .. } => {
            expr_is_nonalloc_numeric(operand, ctx, num)
        }
        Expr::Binary { op, left, right, .. } => {
            matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
            ) && expr_is_nonalloc_numeric(left, ctx, num)
                && expr_is_nonalloc_numeric(right, ctx, num)
        }
        // A call to a proven-non-allocating fn returning a numeric scalar
        // (with non-allocating args) yields a numeric scalar — so
        // `helper(x) + 1` is arithmetic. Both a free fn (`numeric_ret`) and,
        // stage 2, a same-locus `self.m()` whose method is elidable and
        // returns a numeric scalar (`numeric_ret_self_methods`).
        Expr::Call { callee, args, .. } => {
            let callee_numeric = match callee.as_ref() {
                Expr::Ident(id) => {
                    ctx.numeric_ret.contains(&id.name)
                        // 2026-07-02: numeric-scalar-returning
                        // fn-pointer param — see fnptr_numeric_ret.
                        || ctx.fnptr_numeric_ret.contains(&id.name)
                }
                Expr::Field { receiver, name, .. }
                    if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
                {
                    ctx.numeric_ret_self_methods.contains(&name.name)
                }
                _ => false,
            };
            callee_numeric
                && args
                    .iter()
                    .all(|a| expr_definitely_non_allocating(a, ctx, num))
        }
        // Parenthesized single value `( e )`.
        Expr::Tuple(parts, _) if parts.len() == 1 => {
            expr_is_nonalloc_numeric(&parts[0], ctx, num)
        }
        _ => false,
    }
}

/// A numeric-scalar `let` binding — an explicit Int/Uint/Float/Duration
/// ascription, or a RHS that's provably numeric. Its name then joins the
/// numeric scope so later `Add`s over it classify correctly. (Typecheck
/// guarantees the RHS matches an ascription, so trusting the ascription is
/// sound even when the RHS is opaque, e.g. `let n: Int = f();`.)
fn let_binds_nonalloc_numeric(
    ty: Option<&TypeExpr>,
    value: &Expr,
    ctx: &AllocCtx,
    num: &BTreeSet<String>,
) -> bool {
    if let Some(t) = ty {
        if type_expr_is_numeric_scalar(t) {
            return true;
        }
    }
    expr_is_nonalloc_numeric(value, ctx, num)
}

fn stmt_definitely_non_allocating(
    s: &Stmt,
    ctx: &AllocCtx,
    num: &BTreeSet<String>,
) -> bool {
    match s {
        Stmt::Let { value, .. } => expr_definitely_non_allocating(value, ctx, num),
        Stmt::Return(Some(e), _) => expr_definitely_non_allocating(e, ctx, num),
        Stmt::Return(None, _) => true,
        Stmt::Assign { target, value, .. } => {
            // The target gates allocation independently of the value:
            //  - a bare local (`x = …`, no tail) stores by value;
            //  - a scalar `self` field (`self.f = …`, f a by-value field)
            //    stores by value — no deep-copy;
            //  - anything else (a heap `self` field deep-copies; an index
            //    or nested write may touch heap) is conservatively
            //    allocating.
            let target_ok = if target.head.name == "self" {
                matches!(
                    target.tail.as_slice(),
                    [LValueSeg::Field(f)] if ctx.scalar_self_fields.contains(&f.name)
                )
            } else {
                target.tail.is_empty()
            };
            target_ok && expr_definitely_non_allocating(value, ctx, num)
        }
        Stmt::Expr(e) => expr_definitely_non_allocating(e, ctx, num),
        Stmt::If(IfStmt {
            cond,
            then_block,
            else_block,
            ..
        }) => {
            expr_definitely_non_allocating(cond, ctx, num)
                && fn_body_definitely_non_allocating(&then_block.stmts, ctx, num)
                && match else_block.as_deref() {
                    None => true,
                    Some(ElseBranch::Else(b)) => {
                        fn_body_definitely_non_allocating(&b.stmts, ctx, num)
                    }
                    Some(ElseBranch::ElseIf(if_stmt)) => {
                        stmt_definitely_non_allocating(
                            &Stmt::If(if_stmt.clone()),
                            ctx,
                            num,
                        )
                    }
                }
        }
        Stmt::While { cond, body, .. } => {
            expr_definitely_non_allocating(cond, ctx, num)
                && fn_body_definitely_non_allocating(&body.stmts, ctx, num)
        }
        Stmt::Block(b) => fn_body_definitely_non_allocating(&b.stmts, ctx, num),
        Stmt::Break(_) | Stmt::Continue(_) => true,
        // Conservative: for-loops, match, recovery, send, fail,
        // yield, let-tuple all touch machinery that may alloc.
        _ => false,
    }
}

/// GH #720 — stdlib path calls the FORM-3 classifier may treat as
/// non-allocating. Deliberately a hand-verified allowlist rather
/// than "anything PURE": `std::str::substring` is pure and
/// allocates, so purity is the wrong predicate. Each entry below
/// returns a numeric scalar and touches no arena, which is what
/// the subregion exists to manage:
///
///   - `byte_at_unchecked` lowers to a GEP + load;
///   - `byte_at` is `__str_byte_at` — a compare against the view's
///     `n` plus that same load.
///
/// A wrong entry does not dangle — an elided body's allocations
/// route to the caller's arena rather than being freed at return —
/// but they then live as long as the CALLER's arena instead of the
/// call, so a hot loop would grow instead of reusing. Add only
/// scalar-returning, arena-free calls.
const NONALLOC_STDLIB_PATHS: &[&[&str]] = &[
    &["std", "str", "byte_at_unchecked"],
    &["std", "str", "byte_at"],
];

fn path_call_is_nonalloc(q: &hale_syntax::ast::QualifiedName) -> bool {
    let segs: Vec<&str> =
        q.segments.iter().map(|s| s.name.as_str()).collect();
    NONALLOC_STDLIB_PATHS.iter().any(|p| *p == segs.as_slice())
}

fn expr_definitely_non_allocating(
    e: &Expr,
    ctx: &AllocCtx,
    num: &BTreeSet<String>,
) -> bool {
    match e {
        Expr::Literal(_, _) => true,
        Expr::Ident(_) => true,
        Expr::KwSelf(_) => true,
        Expr::Field { receiver, .. } => {
            expr_definitely_non_allocating(receiver, ctx, num)
        }
        Expr::Index { receiver, index, .. } => {
            // Index on a String with a Range is a slice (allocates).
            // The cheap conservative cut: reject any Range-typed
            // index, accept simple integer indices.
            if matches!(index.as_ref(), Expr::Range { .. }) {
                false
            } else {
                expr_definitely_non_allocating(receiver, ctx, num)
                    && expr_definitely_non_allocating(index, ctx, num)
            }
        }
        Expr::Unary { operand, .. } => {
            expr_definitely_non_allocating(operand, ctx, num)
        }
        Expr::Binary { op, left, right, .. } => match op {
            // `+` allocates only as String concat. When both operands are
            // provably non-allocating numeric scalars it's arithmetic and
            // allocates nothing — the type-aware Add classification
            // (2026-06-28). Otherwise (String, or an operand whose type we
            // can't resolve) stay conservative and treat it as allocating.
            BinOp::Add => {
                expr_is_nonalloc_numeric(left, ctx, num)
                    && expr_is_nonalloc_numeric(right, ctx, num)
            }
            // All other BinOps are numeric / bool / bitwise; non-allocating
            // when their operands are.
            BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Mod
            | BinOp::Eq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq
            | BinOp::And
            | BinOp::Or
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr => {
                expr_definitely_non_allocating(left, ctx, num)
                    && expr_definitely_non_allocating(right, ctx, num)
            }
        },
        // A direct call to a proven-non-allocating free fn, with every
        // argument non-allocating, allocates nothing (the callee's body
        // allocates nothing and returns a scalar). Stage 2 (2026-06-28) adds
        // same-locus `self.m(args)` where `m` is a proven-elidable fn-method
        // of the current locus — its body allocates nothing and its
        // scalar/Unit return needs no deep-copy, so the call allocates
        // nothing. A call on any OTHER receiver (`x.m()`, cross-locus) stays
        // conservative (needs type resolution — that's stage 3); std builtins
        // / unresolved callees likewise.
        Expr::Call { callee, args, .. } => {
            let callee_nonalloc = match callee.as_ref() {
                Expr::Ident(id) => {
                    ctx.nonalloc.contains(&id.name)
                        // 2026-07-02: numeric-scalar-returning
                        // fn-pointer param — the callee scratches off
                        // the threaded caller arena and a scalar
                        // return leaves nothing behind. See
                        // fnptr_numeric_ret.
                        || ctx.fnptr_numeric_ret.contains(&id.name)
                }
                // GH #720: a stdlib path call from the
                // non-allocating allowlist. Without this, ANY helper
                // fn that inspects a byte — the shape every parser
                // has — was classified allocating, so each call paid
                // a subregion create/destroy that the inlined body
                // then never used (measured: ~15ns per call, 24ms vs
                // 2ms over a 1 MiB scan).
                Expr::Path(q) => path_call_is_nonalloc(q),
                Expr::Field { receiver, name, .. }
                    if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
                {
                    ctx.elidable_self_methods.contains(&name.name)
                }
                _ => false,
            };
            callee_nonalloc
                && args
                    .iter()
                    .all(|a| expr_definitely_non_allocating(a, ctx, num))
        }
        Expr::If(if_stmt) => {
            expr_definitely_non_allocating(&if_stmt.cond, ctx, num)
                && fn_body_definitely_non_allocating(
                    &if_stmt.then_block.stmts,
                    ctx,
                    num,
                )
                && match if_stmt.else_block.as_deref() {
                    None => true,
                    Some(ElseBranch::Else(b)) => {
                        fn_body_definitely_non_allocating(&b.stmts, ctx, num)
                    }
                    Some(ElseBranch::ElseIf(inner)) => {
                        expr_definitely_non_allocating(
                            &Expr::If(Box::new(inner.clone())),
                            ctx,
                            num,
                        )
                    }
                }
        }
        Expr::Block(b) => fn_body_definitely_non_allocating(&b.stmts, ctx, num),
        // Empty tuple = Unit value, no alloc. Single-elem tuple
        // is parenthesized expression; non-empty multi-elem
        // tuples allocate a tuple struct.
        Expr::Tuple(parts, _) if parts.is_empty() => true,
        Expr::Tuple(parts, _) if parts.len() == 1 => {
            expr_definitely_non_allocating(&parts[0], ctx, num)
        }
        // Conservative for everything else: Path, Path2, Struct,
        // multi-Tuple, Array, ArrayRepeat, Match, Or, Sum, Prod, Approx,
        // Range, FString (Literal::FString).
        _ => false,
    }
}

/// Gate 1 for method-scratch elision (stage 2's, [`elidable_methods`];
/// stage 1, [`member_scratch_elidable`], spells the same gate inline): a
/// method is elision-eligible only if its declared return is `None` (Unit)
/// or a by-value scalar primitive (Int / Uint / Float / Bool / Duration).
/// Any heap return (String / Bytes / struct / array) keeps the scratch —
/// eliding would dangle the return alias, and as a CALL TARGET its result
/// would be a fresh heap value the caller can't treat as non-allocating.
fn ret_is_scalar_or_unit(ret: Option<&TypeExpr>) -> bool {
    matches!(
        ret,
        None | Some(TypeExpr::Primitive(
            PrimType::Int
                | PrimType::Uint
                | PrimType::Float
                | PrimType::Bool
                | PrimType::Duration,
            _,
        ))
    )
}

/// Stage 2 (2026-06-28): compute, per locus, the set of `fn` methods whose
/// scratch is elidable — i.e. whose body allocates nothing AND whose return
/// is a by-value scalar/Unit (no return deep-copy). That property is exactly
/// "a call `self.m(args)` allocates nothing", so this set feeds the
/// classifier's `self.m()` Call arm, letting a method that factors work into
/// sibling self-methods drop its own per-call scratch too.
///
/// A greatest fixpoint, mirroring the non-allocating free-fn fixpoint
/// (`hale_types::alloc_routing`) but scoped to one locus's `self.m()`
/// graph: start with every candidate (gate-1-eligible,
/// non-fallible, non-FFI) optimistically elidable, then demote any whose body
/// provably allocates GIVEN the current elidable set, until stable. Optimism
/// lets mutually-recursive numeric self-methods converge; it's sound because
/// at the fixpoint every remaining method's body is non-allocating with all
/// of ITS `self.*` callees non-allocating, so the set contains no allocator.
///
/// `free_nonalloc` / `free_numeric_ret` are the already-computed free-fn
/// facts (a self-method may also call proven-cheap free fns). Returns a map
/// `locus name → (elidable fn-method names, the numeric-scalar-returning
/// subset)`.
fn elidable_methods(
    items: &[TopDecl],
    free_nonalloc: &BTreeSet<String>,
    free_numeric_ret: &BTreeSet<String>,
) -> BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> {
    let mut out: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> =
        BTreeMap::new();
    // Program-wide `type` struct numeric-scalar field map — lets a method's
    // `s.value` (scalar field of a struct param) classify as numeric.
    let structs = &struct_numeric_field_map(items);
    // GH #884: module nesting flattened, as in
    // the non-allocating free-fn fixpoint.
    for it in hale_syntax::ast::flat_decls(items) {
        let TopDecl::Locus(l) = it else { continue };
        // Candidates: gate-1-eligible (scalar/Unit return), non-fallible,
        // non-FFI `fn` methods. Heap-returning methods are never elidable and
        // never a non-allocating call target.
        // Aliasing stage 2 (2026-07-02): MODES join the candidate
        // set under their synthetic names (bulk/harmonic/
        // resolution) — they lower as ordinary locus methods and
        // brain-tower pulls hit them hot, so both scratch elision
        // and the noalias-self attribute want the same proof.
        let fns: Vec<(&str, &[Param], Option<&TypeExpr>, &Block)> = l
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Fn(fd)
                    if fd.fallible.is_none()
                        && fd.ffi.is_none()
                        && ret_is_scalar_or_unit(fd.ret.as_ref()) =>
                {
                    Some((
                        fd.name.name.as_str(),
                        fd.params.as_slice(),
                        fd.ret.as_ref(),
                        &fd.body,
                    ))
                }
                LocusMember::Mode(md)
                    if ret_is_scalar_or_unit(md.ret.as_ref()) =>
                {
                    let name = match md.kind {
                        ModeKind::Bulk => "bulk",
                        ModeKind::Harmonic => "harmonic",
                        ModeKind::Resolution => "resolution",
                    };
                    Some((
                        name,
                        md.params.as_slice(),
                        md.ret.as_ref(),
                        &md.body,
                    ))
                }
                _ => None,
            })
            .collect();
        if fns.is_empty() {
            continue;
        }
        // Self-field sets from the locus's `params { }` block, by ascription
        // only: numeric scalar fields make `self.x + 1` arithmetic; scalar
        // (numeric + Bool) fields make `self.x = ...` a by-value store.
        // Stage 1 (`self_field_classes`) also types a field by its literal
        // default and through aliases, so the two can disagree (`n = 0` is
        // numeric to stage 1, not here): moved as it was.
        let mut numeric_self_fields: BTreeSet<String> = BTreeSet::new();
        let mut scalar_self_fields: BTreeSet<String> = BTreeSet::new();
        for m in &l.members {
            if let LocusMember::Params(pb) = m {
                for p in &pb.params {
                    match p.ty.as_ref() {
                        Some(TypeExpr::Primitive(
                            PrimType::Int
                            | PrimType::Uint
                            | PrimType::Float
                            | PrimType::Duration,
                            _,
                        )) => {
                            numeric_self_fields.insert(p.name.name.clone());
                            scalar_self_fields.insert(p.name.name.clone());
                        }
                        Some(TypeExpr::Primitive(PrimType::Bool, _)) => {
                            scalar_self_fields.insert(p.name.name.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
        let numeric_ret_of = |elidable: &BTreeSet<String>| -> BTreeSet<String> {
            fns.iter()
                .filter(|(name, _, ret, _)| {
                    elidable.contains(*name)
                        && ret.is_some_and(|t| type_expr_is_numeric_scalar(t))
                })
                .map(|(name, _, _, _)| name.to_string())
                .collect()
        };
        let mut elidable: BTreeSet<String> =
            fns.iter().map(|(name, _, _, _)| name.to_string()).collect();
        loop {
            let numeric_ret_self = numeric_ret_of(&elidable);
            let demote: Vec<String> = fns
                .iter()
                .filter(|(name, _, _, _)| elidable.contains(*name))
                .filter(|(_, params, _, body)| {
                    // Per-method scalar-param-field facts (e.g. `s.value` for
                    // a `Sample` param) — `self.sum + s.value` is arithmetic.
                    let param_field_numeric =
                        param_field_numeric_map(params, structs);
                    let fnptr_numeric_ret =
                        fnptr_numeric_param_set(params);
                    let ctx = AllocCtx {
                        nonalloc: free_nonalloc,
                        numeric_ret: free_numeric_ret,
                        numeric_self_fields: &numeric_self_fields,
                        scalar_self_fields: &scalar_self_fields,
                        elidable_self_methods: &elidable,
                        numeric_ret_self_methods: &numeric_ret_self,
                        param_field_numeric: &param_field_numeric,
                        fnptr_numeric_ret: &fnptr_numeric_ret,
                    };
                    // Seed with the method's numeric scalar params.
                    let seed: BTreeSet<String> = params
                        .iter()
                        .filter(|p| type_expr_is_numeric_scalar(&p.ty))
                        .map(|p| p.name.name.clone())
                        .collect();
                    !fn_body_definitely_non_allocating(&body.stmts, &ctx, &seed)
                })
                .map(|(name, _, _, _)| name.to_string())
                .collect();
            if demote.is_empty() {
                let numeric_ret_self = numeric_ret_of(&elidable);
                out.insert(l.name.name.clone(), (elidable, numeric_ret_self));
                break;
            }
            for n in demote {
                elidable.remove(&n);
            }
        }
    }
    out
}

/// True iff `l`'s arena can be elided at instantiation —
/// `__arena` points at the caller's current arena instead of a
/// fresh `lotus_arena_create()`, and dissolve skips
/// `lotus_arena_destroy`. Mirror of the FORM-3 fn-elision: when
/// nothing in the locus's lifecycle or methods allocates, the
/// per-locus arena is dead substrate, and the
/// `malloc(arena_struct) + malloc(chunk) + ... + free` pair is
/// pure overhead.
///
/// Conservative — rejects on any structural arena consumer
/// (capacity slots, bus subscriptions, closures, failure
/// handlers) or any method body that doesn't pass the FORM-3
/// non-allocating predicate. The `Empty { }` shape passes;
/// loci with bodies that do real work generally won't.
///
/// Applies to both `AcquireStrategy::Fresh` and
/// `AcquireStrategy::Subregion` instantiations — the structural
/// predicate (body non-allocating, no slots, no bus, no closures,
/// no failure handler) holds independently of how `__arena` is
/// acquired. RecpoolFixed/RecpoolSlab children stay on the
/// original path (recpool slots have their own pre-allocated
/// lifecycle).
fn locus_arena_elidable(l: &LocusDecl) -> bool {
    // Structural disqualifiers — these consume arena at
    // instantiation regardless of method-body content.
    for m in &l.members {
        match m {
            LocusMember::Capacity(c) if !c.slots.is_empty() => return false,
            LocusMember::Bus(b) if !b.members.is_empty() => return false,
            LocusMember::Closure(_) => return false,
            LocusMember::Failure(_) => return false,
            _ => {}
        }
    }
    // Empty interprocedural context — this locus-arena classifier doesn't
    // (yet) consult the free-fn non-allocating set, so a call in a method
    // body stays conservative. The type-aware `Add` + numeric `let`s still
    // apply via the local scope.
    let empty = BTreeSet::new();
    let empty_pf = BTreeMap::new();
    let ec = AllocCtx {
        nonalloc: &empty,
        numeric_ret: &empty,
        numeric_self_fields: &empty,
        scalar_self_fields: &empty,
        elidable_self_methods: &empty,
        numeric_ret_self_methods: &empty,
        param_field_numeric: &empty_pf,
        fnptr_numeric_ret: &empty,
    };
    // All method-like bodies non-allocating.
    for m in &l.members {
        let body = match m {
            LocusMember::Lifecycle(lc) => &lc.body,
            LocusMember::Mode(md) => &md.body,
            LocusMember::Fn(fd) => &fd.body,
            _ => continue,
        };
        if !fn_body_definitely_non_allocating(&body.stmts, &ec, &BTreeSet::new()) {
            return false;
        }
    }
    // All param-default expressions non-allocating.
    for m in &l.members {
        if let LocusMember::Params(p) = m {
            for param in &p.params {
                if let ParamInit::Value(expr) = &param.init {
                    if !expr_definitely_non_allocating(expr, &ec, &BTreeSet::new()) {
                        return false;
                    }
                }
            }
        }
    }
    true
}
