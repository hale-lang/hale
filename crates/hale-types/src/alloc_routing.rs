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
//!
//! The rows are the classifications codegen computed at the start of
//! `lower_program`, moved as they were: the checker's reclaim model
//! (`crate::alloc_summary::ReclaimScope`) does not read them yet, and
//! the disagreement the registry records for it stands until it does.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;

/// The allocation-routing rows over one program.
#[derive(Debug, Clone, Default)]
pub struct AllocRouting {
    /// The scratch-local free fns, by the name the call site spells
    /// ([`scratch_local_free_fns`]).
    pub scratch_local: BTreeSet<String>,
}

impl AllocRouting {
    /// Whether the free fn `name` is scratch-local.
    pub fn is_scratch_local(&self, name: &str) -> bool {
        self.scratch_local.contains(name)
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
    AllocRouting { scratch_local: scratch_local_free_fns(&program.items, &imports) }
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
