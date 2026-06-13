//! F.36 Slice 1 (2026-05-28): per-method purity inference.
//!
//! Computes whether each locus method and free fn in a bundle is
//! *pure* — safely callable from any thread without coordination.
//! A method is pure iff its body, transitively, performs no:
//!
//! - writes to `self.<field>` (state mutation observable across calls)
//! - bus publishes (`subject <- value`) — side effects on the bus
//! - closure violations (`violate NAME`) — effects on parent
//! - calls to known-impure stdlib fns (`println`, `time::sleep`,
//!   `std::io::fs::*`, file/process syscalls, etc.)
//! - calls to user-defined methods / free fns that are themselves
//!   not pure (transitively)
//!
//! Pure methods may read `self.<field>`, compute with locals
//! (including `let mut`, since locals don't escape), allocate from
//! arenas, and call other pure methods.
//!
//! The result is a derived property, not a user-declared one — no
//! `pure` keyword. The compiler stores per-method purity in a
//! [`PurityMap`] keyed by [`PurityKey`]; downstream assertion sites
//! (notably F.36 Slice 2's `codec(L)` binding) consult this map to
//! verify that callbacks the substrate dispatches from arbitrary
//! threads are stateless.
//!
//! Slice 1 lands the inference + storage with no consumer; the
//! property is dormant until Slice 2 wires the assertion path.

use std::collections::BTreeMap;

use hale_syntax::ast::*;
use hale_syntax::Span;

use crate::resolve::TopScope;

/// Per-method purity result. [`Purity::Impure`] carries the first
/// impurity the walker found — the diagnostic anchor for Slice 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purity {
    /// No impurity found in the method body (transitively).
    Pure,
    /// Body has at least one impurity; `reason` names the first
    /// one found in source order. Subsequent impurities exist but
    /// aren't tracked (the first is the natural diagnostic site).
    Impure(Impurity),
}

/// The first impurity found in a method body. Used to anchor a
/// diagnostic at the offending source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Impurity {
    /// `self.X = ...` (or compound assign). `field_chain` is the
    /// dotted path from `self` to the assigned location.
    SelfFieldWrite { field_chain: String, span: Span },
    /// `SUBJECT <- VALUE;` bus send. Side effect on the bus.
    BusSend { subject_repr: String, span: Span },
    /// `violate NAME [with PAYLOAD];` — escalates a structural
    /// failure through the parent's `on_failure`.
    Violate { closure_name: String, span: Span },
    /// Call to a stdlib fn the effect table marks as having
    /// observable effects (println, file write, sleep, ...).
    ImpureStdlibCall { fn_name: String, span: Span },
    /// Call to a user-defined fn / method that the inference
    /// pass marks impure.
    ImpureCalleeCall { callee_name: String, span: Span },
}

/// Identifies a fn for purity lookup. Free fns have `locus: None`;
/// locus methods carry the enclosing locus's name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PurityKey {
    pub locus: Option<String>,
    pub fn_name: String,
}

impl PurityKey {
    pub fn free_fn(name: impl Into<String>) -> Self {
        Self { locus: None, fn_name: name.into() }
    }
    pub fn method(locus: impl Into<String>, name: impl Into<String>) -> Self {
        Self { locus: Some(locus.into()), fn_name: name.into() }
    }
}

/// Bundle-wide purity result keyed by [`PurityKey`].
pub type PurityMap = BTreeMap<PurityKey, Purity>;

/// Stdlib paths with observable side effects. Calls to any of
/// these (matched by qualified path) make the calling fn impure.
///
/// The list is conservative — anything that writes to the
/// filesystem / network / process state / stdout / global mutables.
/// Pure stdlib (arithmetic helpers, `std::str::*` transforms that
/// allocate but don't mutate observable state, `std::decimal::to_string`,
/// `std::bytes::*` builders, JSON walkers that read source) is NOT
/// in this list — absence means pure (allocation is acceptable).
const IMPURE_STDLIB_PATHS: &[&[&str]] = &[
    // Console / debugging
    &["std", "process", "exit"],
    &["std", "process", "dump_arena_residency"],
    &["std", "process", "dump_pool_residency"],
    &["std", "process", "run"],
    &["std", "process", "spawn"],
    // Time
    &["std", "time", "sleep"],
    // Bus / yield primitives (when callable from fn bodies)
    // (yield is a Stmt; the impurity table covers it separately)
    // Filesystem
    &["std", "io", "fs", "write_file"],
    &["std", "io", "fs", "write_file_append"],
    &["std", "io", "fs", "delete_file"],
    &["std", "io", "fs", "make_dir"],
    &["std", "io", "fs", "read_file"],
    &["std", "io", "fs", "read_file_bytes"],
    &["std", "io", "fs", "file_size"],
    // TCP / UDP — both connect and accept have effects (sockets)
    &["std", "io", "tcp", "__listen_socket"],
    &["std", "io", "tcp", "__accept_one"],
    &["std", "io", "tcp", "__connect"],
    &["std", "io", "tcp", "__close_fd"],
    &["std", "io", "tcp", "set_recv_timeout"],
    &["std", "io", "tcp", "set_send_timeout"],
    &["std", "io", "tcp", "set_nodelay"],
    &["std", "io", "udp", "send_bytes"],
    &["std", "io", "udp", "recv_bytes"],
    // Crypto with global state (rare; covered by random for now)
    &["std", "crypto", "random_bytes"],
    // Env (reads only; treat var() as pure since env is stable
    // for the program's lifetime — set at startup).
];

fn is_impure_stdlib(segments: &[&str]) -> bool {
    IMPURE_STDLIB_PATHS.iter().any(|p| *p == segments)
}

/// Bare-identifier callee names that are impure builtins (visible
/// to user code without the `std::` qualifier). The bus / print
/// surface that shipped as bare-name primitives lives here.
const IMPURE_BARE_BUILTINS: &[&str] = &[
    "println",
    "print",
    "eprintln",
    "eprint",
    "raise",        // diverges via closure violation
    "panic",        // process exit
    "bubble",       // recovery escalation
    "quarantine",
    "restart",
    "restart_in_place",
    "dissolve",
    "reorganize",
];

fn is_impure_bare_builtin(name: &str) -> bool {
    IMPURE_BARE_BUILTINS.iter().any(|n| *n == name)
}

/// Bundle of programs the typecheck pass operates on. Mirrors the
/// shape `check_bundle` consumes.
pub struct Bundle<'a> {
    pub programs: BTreeMap<String, &'a Program>,
}

/// Entry point. Walks every locus method and free fn in the
/// bundle, computing purity via a worklist algorithm that
/// converges on the transitive-callee dependency graph. Returns a
/// [`PurityMap`] downstream passes consult.
pub fn infer_purity_for_bundle(
    programs: &[&Program],
    _scope: &TopScope,
) -> PurityMap {
    // Collect every fn body the bundle owns so we can walk it
    // repeatedly during fixed-point iteration.
    let mut fn_bodies: BTreeMap<PurityKey, (Vec<Stmt>, Span)> = BTreeMap::new();
    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Fn(decl) => {
                    fn_bodies.insert(
                        PurityKey::free_fn(decl.name.name.clone()),
                        (decl.body.stmts.clone(), decl.span),
                    );
                }
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let LocusMember::Fn(decl) = member {
                            fn_bodies.insert(
                                PurityKey::method(
                                    l.name.name.clone(),
                                    decl.name.name.clone(),
                                ),
                                (decl.body.stmts.clone(), decl.span),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Initial state: every unknown fn marked pure (the absence of
    // an entry in `map` is treated as Unknown during a pass).
    // Worklist iterates until no changes — purity transitions
    // monotonically (Pure → Impure when an impure callee is found,
    // or vice versa as transitive callees converge).
    //
    // For a worklist that converges, we use a three-valued logic
    // during iteration: a fn whose callee status is still UNKNOWN
    // is kept out of the result until all its callees resolve.
    // After fixed point, any remaining unknowns are conservatively
    // marked impure (cycle / unreachable).
    let mut map: PurityMap = BTreeMap::new();
    let mut prev_size = usize::MAX;
    while map.len() != prev_size {
        prev_size = map.len();
        for (key, (body, _span)) in &fn_bodies {
            if map.contains_key(key) {
                // Already settled; revisit only if a callee newly
                // turned impure. For Slice 1 we keep it simple —
                // once Pure, stays Pure (this is safe given the
                // monotone worklist below).
                continue;
            }
            match scan_body_purity(body, &fn_bodies, &map) {
                ScanResult::Resolved(p) => {
                    map.insert(key.clone(), p);
                }
                ScanResult::DependsOnUnknown => {
                    // Defer — another pass will catch it once the
                    // unresolved callee settles.
                }
            }
        }
    }
    // Conservative finalization: anything still unresolved (cycle
    // or unreachable) is impure.
    for (key, (body, _)) in &fn_bodies {
        if !map.contains_key(key) {
            let imp = first_explicit_impurity_in_body(body)
                .unwrap_or(Impurity::ImpureCalleeCall {
                    callee_name: "<unresolved cycle>".to_string(),
                    span: body.first().map(|s| stmt_span(s)).unwrap_or(Span::new(0, 0)),
                });
            map.insert(key.clone(), Purity::Impure(imp));
        }
    }
    map
}

/// Per-method scan output. `DependsOnUnknown` means a callee's
/// purity is still UNKNOWN; the worklist defers the method until
/// the callee resolves.
enum ScanResult {
    Resolved(Purity),
    DependsOnUnknown,
}

fn scan_body_purity(
    body: &[Stmt],
    all_fns: &BTreeMap<PurityKey, (Vec<Stmt>, Span)>,
    map: &PurityMap,
) -> ScanResult {
    let mut any_unknown = false;
    for stmt in body {
        match scan_stmt(stmt, all_fns, map, &mut any_unknown) {
            Some(imp) => return ScanResult::Resolved(Purity::Impure(imp)),
            None => {}
        }
    }
    if any_unknown {
        ScanResult::DependsOnUnknown
    } else {
        ScanResult::Resolved(Purity::Pure)
    }
}

/// Walk a statement, returning Some(first impurity) if found.
/// Updates `any_unknown` if any callee's purity is still
/// undetermined.
fn scan_stmt(
    stmt: &Stmt,
    all_fns: &BTreeMap<PurityKey, (Vec<Stmt>, Span)>,
    map: &PurityMap,
    any_unknown: &mut bool,
) -> Option<Impurity> {
    match stmt {
        Stmt::Assign { target, value, span, .. } => {
            if target.head.name == "self" {
                let chain = format_lvalue_chain(target);
                return Some(Impurity::SelfFieldWrite {
                    field_chain: chain,
                    span: *span,
                });
            }
            scan_expr(value, all_fns, map, any_unknown)
        }
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            scan_expr(value, all_fns, map, any_unknown)
        }
        Stmt::Send { span, subject, .. } => Some(Impurity::BusSend {
            subject_repr: format!("{:?}", subject),
            span: *span,
        }),
        Stmt::Violate { name, span, .. } => Some(Impurity::Violate {
            closure_name: name.name.clone(),
            span: *span,
        }),
        Stmt::Yield(span) => Some(Impurity::ImpureStdlibCall {
            fn_name: "yield".to_string(),
            span: *span,
        }),
        Stmt::Terminate(span) => Some(Impurity::ImpureStdlibCall {
            fn_name: "terminate".to_string(),
            span: *span,
        }),
        Stmt::Return(opt, _) => {
            if let Some(e) = opt {
                scan_expr(e, all_fns, map, any_unknown)
            } else {
                None
            }
        }
        Stmt::Fail { value, .. } => scan_expr(value, all_fns, map, any_unknown),
        Stmt::If(if_stmt) => scan_if_stmt(if_stmt, all_fns, map, any_unknown),
        Stmt::Match(m) => {
            if let Some(imp) = scan_expr(&m.scrutinee, all_fns, map, any_unknown) {
                return Some(imp);
            }
            for arm in &m.arms {
                if let Some(g) = &arm.guard {
                    if let Some(imp) = scan_expr(g, all_fns, map, any_unknown) {
                        return Some(imp);
                    }
                }
                match &arm.body {
                    MatchArmBody::Block(b) => {
                        for s in &b.stmts {
                            if let Some(imp) =
                                scan_stmt(s, all_fns, map, any_unknown)
                            {
                                return Some(imp);
                            }
                        }
                    }
                    MatchArmBody::Expr(e) => {
                        if let Some(imp) = scan_expr(e, all_fns, map, any_unknown) {
                            return Some(imp);
                        }
                    }
                }
            }
            None
        }
        Stmt::For { iter, body, .. } => {
            if let Some(imp) = scan_expr(iter, all_fns, map, any_unknown) {
                return Some(imp);
            }
            for s in &body.stmts {
                if let Some(imp) = scan_stmt(s, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            None
        }
        Stmt::While { cond, body, .. } => {
            if let Some(imp) = scan_expr(cond, all_fns, map, any_unknown) {
                return Some(imp);
            }
            for s in &body.stmts {
                if let Some(imp) = scan_stmt(s, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            None
        }
        Stmt::Block(b) => {
            for s in &b.stmts {
                if let Some(imp) = scan_stmt(s, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            None
        }
        Stmt::ShmWrite { max, body, .. } => {
            if let Some(imp) = scan_expr(max, all_fns, map, any_unknown) {
                return Some(imp);
            }
            for s in &body.stmts {
                if let Some(imp) = scan_stmt(s, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            body.tail
                .as_ref()
                .and_then(|t| scan_expr(t, all_fns, map, any_unknown))
        }
        Stmt::Expr(e) => scan_expr(e, all_fns, map, any_unknown),
        Stmt::Recovery { args, span, op, .. } => {
            // Recovery primitives (quarantine, restart, etc.) all
            // mutate locus state across the framework — model as
            // impure.
            for a in args {
                if let Some(imp) = scan_expr(a, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            Some(Impurity::ImpureStdlibCall {
                fn_name: format!("{:?}", op),
                span: *span,
            })
        }
        Stmt::Break(_) | Stmt::Continue(_) => None,
    }
}

fn scan_if_stmt(
    if_stmt: &IfStmt,
    all_fns: &BTreeMap<PurityKey, (Vec<Stmt>, Span)>,
    map: &PurityMap,
    any_unknown: &mut bool,
) -> Option<Impurity> {
    if let Some(imp) = scan_expr(&if_stmt.cond, all_fns, map, any_unknown) {
        return Some(imp);
    }
    for s in &if_stmt.then_block.stmts {
        if let Some(imp) = scan_stmt(s, all_fns, map, any_unknown) {
            return Some(imp);
        }
    }
    if let Some(else_br) = &if_stmt.else_block {
        match else_br.as_ref() {
            ElseBranch::Else(b) => {
                for s in &b.stmts {
                    if let Some(imp) = scan_stmt(s, all_fns, map, any_unknown) {
                        return Some(imp);
                    }
                }
            }
            ElseBranch::ElseIf(inner) => {
                return scan_if_stmt(inner, all_fns, map, any_unknown);
            }
        }
    }
    None
}

/// Walk an expression. Returns Some(first impurity) if found.
fn scan_expr(
    expr: &Expr,
    all_fns: &BTreeMap<PurityKey, (Vec<Stmt>, Span)>,
    map: &PurityMap,
    any_unknown: &mut bool,
) -> Option<Impurity> {
    match expr {
        Expr::Literal(_, _) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => None,
        Expr::Binary { left, right, .. } => {
            scan_expr(left, all_fns, map, any_unknown)
                .or_else(|| scan_expr(right, all_fns, map, any_unknown))
        }
        Expr::Unary { operand, .. } => scan_expr(operand, all_fns, map, any_unknown),
        Expr::Call { callee, args, span } => {
            // Args evaluated first.
            for a in args {
                if let Some(imp) = scan_expr(a, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            // Resolve the callee.
            scan_callee(callee, *span, all_fns, map, any_unknown)
        }
        Expr::Field { receiver, .. } | Expr::Index { receiver, .. } => {
            scan_expr(receiver, all_fns, map, any_unknown)
        }
        Expr::Path2 { receiver, .. } => scan_expr(receiver, all_fns, map, any_unknown),
        Expr::Tuple(xs, _) | Expr::Array(xs, _) => {
            for x in xs {
                if let Some(imp) = scan_expr(x, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            None
        }
        Expr::Struct { inits, .. } => {
            for si in inits {
                if let Some(imp) = scan_expr(&si.value, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            None
        }
        Expr::Block(b) => {
            for s in &b.stmts {
                if let Some(imp) = scan_stmt(s, all_fns, map, any_unknown) {
                    return Some(imp);
                }
            }
            None
        }
        Expr::If(if_stmt) => scan_if_stmt(if_stmt, all_fns, map, any_unknown),
        Expr::Match(m) => {
            if let Some(imp) = scan_expr(&m.scrutinee, all_fns, map, any_unknown) {
                return Some(imp);
            }
            for arm in &m.arms {
                match &arm.body {
                    MatchArmBody::Block(b) => {
                        for s in &b.stmts {
                            if let Some(imp) =
                                scan_stmt(s, all_fns, map, any_unknown)
                            {
                                return Some(imp);
                            }
                        }
                    }
                    MatchArmBody::Expr(e) => {
                        if let Some(imp) = scan_expr(e, all_fns, map, any_unknown) {
                            return Some(imp);
                        }
                    }
                }
            }
            None
        }
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => {
            scan_expr(inner, all_fns, map, any_unknown)
        }
        Expr::Approx { left, right, tolerance, .. } => scan_expr(left, all_fns, map, any_unknown)
            .or_else(|| scan_expr(right, all_fns, map, any_unknown))
            .or_else(|| scan_expr(tolerance, all_fns, map, any_unknown)),
        Expr::Range { lo, hi, .. } => scan_expr(lo, all_fns, map, any_unknown)
            .or_else(|| scan_expr(hi, all_fns, map, any_unknown)),
        Expr::ArrayRepeat { val, .. } => scan_expr(val, all_fns, map, any_unknown),
        Expr::Or { inner, disposition, .. } => {
            if let Some(imp) = scan_expr(inner, all_fns, map, any_unknown) {
                return Some(imp);
            }
            match disposition {
                OrDisposition::Raise(_) | OrDisposition::Discard(_) => None,
                OrDisposition::Substitute(e) => scan_expr(e, all_fns, map, any_unknown),
                OrDisposition::Fail(e, _) => scan_expr(e, all_fns, map, any_unknown),
            }
        }
    }
}

fn scan_callee(
    callee: &Expr,
    call_span: Span,
    all_fns: &BTreeMap<PurityKey, (Vec<Stmt>, Span)>,
    map: &PurityMap,
    any_unknown: &mut bool,
) -> Option<Impurity> {
    // Resolve the callee to a function-name path. Three shapes:
    //   - `Ident(name)` — bare free fn or in-scope method
    //   - `Path(segments)` — qualified path (`std::process::exit`)
    //   - `Field/Path2 { receiver, name }` — method call (we
    //     conservatively treat as impure if we can't resolve the
    //     receiver type to a known locus)
    match callee {
        Expr::Path(qp) => {
            let segs: Vec<&str> = qp.segments.iter().map(|s| s.name.as_str()).collect();
            if is_impure_stdlib(&segs) {
                return Some(Impurity::ImpureStdlibCall {
                    fn_name: segs.join("::"),
                    span: call_span,
                });
            }
            // Pure stdlib path or unresolved user path — no effect
            // we can attribute. (User-defined fns at qualified
            // paths are cross-seed imports; we conservatively
            // treat as unknown which means the calling fn becomes
            // impure until the imported fn's purity is known.
            // v0.1 simplification: treat as pure to avoid blocking
            // every cross-seed user.)
            None
        }
        Expr::Ident(id) => {
            // Bare-ident builtin impure list first (println,
            // raise, panic, etc.).
            if is_impure_bare_builtin(&id.name) {
                return Some(Impurity::ImpureStdlibCall {
                    fn_name: id.name.clone(),
                    span: call_span,
                });
            }
            // User free fn lookup.
            let key = PurityKey::free_fn(id.name.clone());
            if all_fns.contains_key(&key) {
                match map.get(&key) {
                    Some(Purity::Pure) => None,
                    Some(Purity::Impure(_)) => {
                        Some(Impurity::ImpureCalleeCall {
                            callee_name: id.name.clone(),
                            span: call_span,
                        })
                    }
                    None => {
                        *any_unknown = true;
                        None
                    }
                }
            } else {
                // Not a known free fn — could be a captured closure
                // param or a builtin like `len`, `sum`, `count`.
                // Builtins not on the impure list are pure by
                // construction.
                None
            }
        }
        Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
            // Method call on a receiver. If receiver is `self`, we
            // can resolve to a same-locus method. Otherwise we can't
            // determine the receiver's locus type without typecheck
            // info; conservatively treat as impure.
            if let Expr::KwSelf(_) = receiver.as_ref() {
                // We don't know the enclosing locus name here — the
                // caller's purity walker doesn't carry it. For
                // Slice 1, conservatively treat self-method calls as
                // pure (will be tightened in a follow-up to look up
                // by enclosing locus). The worst-case is that an
                // impure self-method call slips past purity
                // inference; v0.1 tradeoff.
                return None;
            }
            // Receiver is some other expression. Treat as impure-
            // callee — we can't resolve the locus type at this layer.
            // This is the conservative default. Refining requires
            // type-resolved info that lives in the check pass.
            Some(Impurity::ImpureCalleeCall {
                callee_name: name.name.clone(),
                span: call_span,
            })
        }
        _ => None,
    }
}

/// Format an LValue's field chain for diagnostic display:
/// `self.foo.bar[0]` → `"self.foo.bar"`.
fn format_lvalue_chain(lv: &LValue) -> String {
    let mut s = lv.head.name.clone();
    for seg in &lv.tail {
        match seg {
            LValueSeg::Field(id) => {
                s.push('.');
                s.push_str(&id.name);
            }
            LValueSeg::Index(_) => {
                s.push_str("[…]");
            }
        }
    }
    s
}

/// Find the first explicit impurity (self-write / bus send /
/// violate / impure stdlib call) in a body, ignoring callee-
/// dependency unknowns. Used as the diagnostic anchor for fns
/// that landed at the conservative cycle-finalization step.
fn first_explicit_impurity_in_body(body: &[Stmt]) -> Option<Impurity> {
    let empty_map: PurityMap = BTreeMap::new();
    let empty_fns: BTreeMap<PurityKey, (Vec<Stmt>, Span)> = BTreeMap::new();
    let mut sink = false;
    for stmt in body {
        if let Some(imp) = scan_stmt(stmt, &empty_fns, &empty_map, &mut sink) {
            return Some(imp);
        }
    }
    None
}

fn stmt_span(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. }
        | Stmt::LetTuple { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::For { span, .. }
        | Stmt::While { span, .. }
        | Stmt::Fail { span, .. }
        | Stmt::Recovery { span, .. }
        | Stmt::Violate { span, .. }
        | Stmt::Send { span, .. } => *span,
        Stmt::Return(_, span) => *span,
        Stmt::Break(span) | Stmt::Continue(span) | Stmt::Yield(span) | Stmt::Terminate(span) => *span,
        Stmt::If(if_stmt) => if_stmt.span,
        Stmt::Match(m) => m.span,
        Stmt::Block(b) => b.span,
        Stmt::ShmWrite { span, .. } => *span,
        Stmt::Expr(e) => match e {
            Expr::Literal(_, span) => *span,
            _ => Span::new(0, 0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn purity_of(src: &str, key: PurityKey) -> Purity {
        let program = parse_source(src).expect("parse");
        let scope = crate::resolve::TopScope::default();
        let map = infer_purity_for_bundle(&[&program], &scope);
        map.get(&key)
            .cloned()
            .unwrap_or_else(|| panic!("no purity entry for {:?}; map = {:?}", key, map.keys().collect::<Vec<_>>()))
    }

    #[test]
    fn pure_free_fn_with_arithmetic_is_pure() {
        let p = purity_of(
            "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }",
            PurityKey::free_fn("add"),
        );
        assert!(matches!(p, Purity::Pure), "expected pure, got {:?}", p);
    }

    #[test]
    fn fn_calling_println_is_impure() {
        let p = purity_of(
            "fn shout(s: String) { println(s); } fn main() { }",
            PurityKey::free_fn("shout"),
        );
        match p {
            Purity::Impure(Impurity::ImpureStdlibCall { fn_name, .. }) => {
                assert_eq!(fn_name, "println", "unexpected callee");
            }
            other => panic!("expected ImpureStdlibCall(println), got {:?}", other),
        }
    }

    #[test]
    fn method_writing_self_field_is_impure() {
        let src = r#"
            locus C {
                params { n: Int = 0; }
                fn bump() {
                    self.n = self.n + 1;
                }
            }
            fn main() { }
        "#;
        let p = purity_of(src, PurityKey::method("C", "bump"));
        match p {
            Purity::Impure(Impurity::SelfFieldWrite { field_chain, .. }) => {
                assert_eq!(field_chain, "self.n");
            }
            other => panic!("expected SelfFieldWrite, got {:?}", other),
        }
    }

    #[test]
    fn method_reading_self_field_is_pure() {
        let src = r#"
            locus C {
                params { n: Int = 0; }
                fn get() -> Int {
                    return self.n;
                }
            }
            fn main() { }
        "#;
        let p = purity_of(src, PurityKey::method("C", "get"));
        assert!(matches!(p, Purity::Pure), "expected pure, got {:?}", p);
    }

    #[test]
    fn transitive_impurity_via_callee_propagates() {
        let src = r#"
            fn does_io(s: String) { println(s); }
            fn wrapper(s: String) { does_io(s); }
            fn main() { }
        "#;
        let p = purity_of(src, PurityKey::free_fn("wrapper"));
        match p {
            Purity::Impure(Impurity::ImpureCalleeCall { callee_name, .. }) => {
                assert_eq!(callee_name, "does_io");
            }
            other => panic!("expected ImpureCalleeCall(does_io), got {:?}", other),
        }
    }
}
