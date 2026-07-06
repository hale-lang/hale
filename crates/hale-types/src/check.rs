//! Type checking — milestone 2 cut.
//!
//! Walks every program in the bundle and verifies a tractable
//! subset of the type rules:
//!
//! - Literal expressions get their natural primitive type.
//! - Binary / unary operator operand-type compatibility.
//! - `let x: T = e;` — e's inferred type assignable to T.
//! - Struct-literal field names + types match the type
//!   declaration.
//! - Bus send (`"subject" <- v`): subject is declared in the
//!   enclosing locus's bus block, payload type matches.
//! - `~~` closure assertion: left and right have compatible
//!   types; tolerance is numeric-ish (we don't enforce strictly
//!   in milestone 2 — just that something is there).
//! - `self.field`: resolves against enclosing locus's params.
//!
//! Names referenced via paths the bundle can't see (stdlib,
//! `time::sleep`, `println`) resolve to `Ty::Unknown`, which
//! is bidirectionally compatible — milestone 2 does not error
//! on these. Milestone 3 will tighten.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::resolve::{resolve_type_expr, TopScope};
use crate::symbol::*;
use crate::ty::{is_flat_shapeable, is_key_eligible, Ty};

fn method_to_fn_ty(m: &MethodInfo) -> Ty {
    Ty::Function {
        params: m.params.clone(),
        ret: Box::new(m.ret.clone()),
    }
}

/// True if the match arms cover every possible scrutinee
/// value. v0 rules:
///   - Any arm without a guard whose pattern is wildcard `_`
///     or a bare binding makes the match exhaustive.
///   - For Bool scrutinee: literal `true` AND literal `false`
///     arms (both unguarded) is also exhaustive.
///   - For an enum-typed scrutinee (m47): every declared variant
///     must be covered by an unguarded `EnumName::Variant`
///     constructor pattern.
///   - For everything else: a wildcard / binding is required.
fn match_is_exhaustive(scrut_ty: &Ty, arms: &[MatchArm], top: &TopScope) -> bool {
    let unguarded = |a: &&MatchArm| a.guard.is_none();
    let has_catchall = arms.iter().filter(unguarded).any(|a| {
        matches!(a.pattern, Pattern::Wildcard(_) | Pattern::Binding(_))
    });
    if has_catchall {
        return true;
    }
    if matches!(scrut_ty, Ty::Prim(PrimType::Bool)) {
        let mut has_true = false;
        let mut has_false = false;
        for arm in arms.iter().filter(unguarded) {
            if let Pattern::Literal(Literal::Bool(b), _) = &arm.pattern {
                if *b {
                    has_true = true;
                } else {
                    has_false = true;
                }
            }
        }
        return has_true && has_false;
    }
    if let Ty::Named(name) = scrut_ty {
        if let Some(TopSymbol::Type(TypeInfo {
            kind: TypeKind::Enum(variants),
            ..
        })) = top.symbols.get(name)
        {
            let mut covered: std::collections::BTreeSet<&str> =
                std::collections::BTreeSet::new();
            // m68: also accept arms whose enum_seg is a
            // synthesized monomorph of `name` — e.g. arms
            // written as `Result_Int_String::Ok` count as
            // covering `Ok` for a scrutinee typed as the
            // generic `Result` template. Codegen monomorphizes
            // generic enums into mangled-name decls
            // (`Result_Int_String`) but the typechecker only
            // sees the original template, so the user's match
            // arms (which use the mangled names that codegen
            // recognizes) would otherwise false-positive as
            // non-exhaustive. The mangle convention is
            // `<template>_<arg>_<arg>...` so the prefix check
            // is unambiguous.
            let mangle_prefix = format!("{}_", name);
            for arm in arms.iter().filter(unguarded) {
                if let Pattern::Constructor { path, .. } = &arm.pattern {
                    if let [enum_seg, variant_seg] = path.segments.as_slice() {
                        let matches_template_or_monomorph =
                            enum_seg.name == *name
                                || enum_seg
                                    .name
                                    .starts_with(&mangle_prefix);
                        if matches_template_or_monomorph {
                            // m47-payloads: a Constructor arm
                            // covers its variant whether the
                            // sub-patterns are wildcards / bindings
                            // (catch-all over the payload) or
                            // empty (no-payload variant). Literal
                            // sub-patterns are narrower and
                            // wouldn't cover all values of the
                            // variant; we still treat them as
                            // covering for v0.1 — same permissive
                            // policy the Bool literal arms get.
                            covered.insert(variant_seg.name.as_str());
                        }
                    }
                }
            }
            return variants.iter().all(|v| covered.contains(v.name.as_str()));
        }
        // m68: a named type the typechecker doesn't know about
        // at all (commonly: a fully-mangled monomorph that
        // somehow flows in — codegen synthesizes those, the
        // typechecker doesn't see them) should be permissive
        // for exhaustiveness, same as Ty::Unknown. Narrowed
        // to "name not in top.symbols" so known structs / loci
        // / perspectives still require a wildcard / binding arm.
        if !top.symbols.contains_key(name) {
            return true;
        }
    }
    // Be permissive on Unknown — we genuinely can't say.
    matches!(scrut_ty, Ty::Unknown)
}

/// True if `e` is composed entirely of literals — no
/// identifiers, no `self`, no calls, no field access. Used by
/// closure-cycle-existence: a closure assertion with pure-
/// literal sides has nothing to audit.
fn is_pure_literal(e: &Expr) -> bool {
    match e {
        Expr::Literal(_, _) => true,
        Expr::Unary { operand, .. } => is_pure_literal(operand),
        Expr::Binary { left, right, .. } => {
            is_pure_literal(left) && is_pure_literal(right)
        }
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
            parts.iter().all(is_pure_literal)
        }
        _ => false,
    }
}

/// M3 stage 3: collect generic fn templates (recursing modules).
fn collect_generic_fns<'a>(
    items: &'a [TopDecl],
    out: &mut BTreeMap<String, &'a FnDecl>,
) {
    for item in items {
        match item {
            TopDecl::Fn(f) if !f.generics.is_empty() => {
                out.insert(f.name.name.clone(), f);
            }
            TopDecl::Module(m) => collect_generic_fns(&m.items, out),
            _ => {}
        }
    }
}

/// M3 stage 3 tranche 2: collect generic type templates.
fn collect_generic_types<'a>(
    items: &'a [TopDecl],
    out: &mut BTreeMap<String, &'a TypeDecl>,
) {
    for item in items {
        match item {
            TopDecl::Type(t) if !t.generics.is_empty() => {
                out.insert(t.name.name.clone(), t);
            }
            TopDecl::Module(m) => collect_generic_types(&m.items, out),
            _ => {}
        }
    }
}

/// M3 stage 3 tranche 2: resolve one mangle token (`Int`, `Float`,
/// a user type name, or a nested mangled monomorph) to a Ty. The
/// codegen mangle joins tokens with `_`, so this stays permissive
/// (Unknown) for anything it can't confidently name.
fn mangle_token_to_ty(
    tok: &str,
    known: &BTreeMap<String, Span>,
) -> Ty {
    match tok {
        "Int" => Ty::Prim(PrimType::Int),
        "Float" => Ty::Prim(PrimType::Float),
        "Bool" => Ty::Prim(PrimType::Bool),
        "String" => Ty::Prim(PrimType::String),
        "Duration" => Ty::Prim(PrimType::Duration),
        "Decimal" => Ty::Prim(PrimType::Decimal),
        "Time" => Ty::Prim(PrimType::Time),
        other => {
            if known.contains_key(other) {
                Ty::Named(other.to_string())
            } else {
                Ty::Unknown
            }
        }
    }
}

/// M3 stage 3: Ty-level mirror of codegen's m62
/// `unify_generic_param_bindings`. Binds generic names appearing in
/// `param_te` against the actual arg type. Top-level generic names
/// bind directly; Array/Bounded recurse on the element. Generic
/// names nested under generic-ARG'd Named types (Box<T> in param
/// position) stay unbound here — permissive, codegen's own unifier
/// still runs. Returns Err((name, existing, new)) on a conflict.
fn unify_generic_ty(
    param_te: &TypeExpr,
    arg: &Ty,
    generics: &std::collections::BTreeSet<String>,
    bindings: &mut BTreeMap<String, Ty>,
) -> Result<(), (String, Ty, Ty)> {
    match param_te {
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && path.segments.len() == 1 =>
        {
            let name = &path.segments[0].name;
            if !generics.contains(name) {
                return Ok(());
            }
            if matches!(arg, Ty::Unknown) {
                return Ok(());
            }
            match bindings.get(name) {
                Some(existing) if existing != arg => Err((
                    name.clone(),
                    existing.clone(),
                    arg.clone(),
                )),
                Some(_) => Ok(()),
                None => {
                    bindings.insert(name.clone(), arg.clone());
                    Ok(())
                }
            }
        }
        TypeExpr::Array { elem, .. } => match arg {
            Ty::Array(a_elem, _) => {
                unify_generic_ty(elem, a_elem, generics, bindings)
            }
            _ => Ok(()),
        },
        TypeExpr::Bounded { elem, .. } => match arg {
            Ty::Bounded(a_elem, _) => {
                unify_generic_ty(elem, a_elem, generics, bindings)
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// M3 stage 3: resolve a template TypeExpr with generic bindings
/// applied — generic names map through `bindings`; everything else
/// through the ordinary resolver. Unbound generics (or shapes the
/// resolver can't see) fall to Unknown, keeping the checks
/// permissive exactly where inference was.
fn substitute_generic_ty(
    te: &TypeExpr,
    bindings: &BTreeMap<String, Ty>,
    known: &BTreeMap<String, Span>,
) -> Ty {
    match te {
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && path.segments.len() == 1 =>
        {
            if let Some(t) = bindings.get(&path.segments[0].name) {
                return t.clone();
            }
            resolve_type_expr(te, known)
        }
        TypeExpr::Array { elem, size, .. } => {
            let n = match size {
                Some(Expr::Literal(Literal::Int(n), _)) if *n >= 0 => {
                    Some(*n as u64)
                }
                _ => None,
            };
            Ty::Array(
                Box::new(substitute_generic_ty(elem, bindings, known)),
                n,
            )
        }
        TypeExpr::Bounded { elem, cap, .. } => Ty::Bounded(
            Box::new(substitute_generic_ty(elem, bindings, known)),
            *cap,
        ),
        _ => resolve_type_expr(te, known),
    }
}

pub fn check_bundle(
    bundle: &Bundle<'_>,
    top: &TopScope,
    allow_unowned_subscriber: bool,
) -> Vec<Diag> {
    let mut diags = Vec::new();
    let known = collect_known_names(top);
    // WASM plan: the bundle targets wasm if any program declares
    // `target wasm` / `target browser_js`. Drives stdlib gating below.
    let wasm_target = bundle.programs.values().any(|p| {
        p.items.iter().any(|it| {
            matches!(it, TopDecl::Target(t)
                if matches!(t.name.name.as_str(), "wasm" | "browser_js"))
        })
    });
    for program in bundle.programs.values() {
        let mut generic_fns: BTreeMap<String, &FnDecl> = BTreeMap::new();
        collect_generic_fns(&program.items, &mut generic_fns);
        let mut generic_types: BTreeMap<String, &TypeDecl> =
            BTreeMap::new();
        collect_generic_types(&program.items, &mut generic_types);
        let mut cx = Checker {
            top,
            known: &known,
            diags: &mut diags,
            locals: ScopeStack::new(),
            current_locus: None,
            in_lifecycle: false,
            in_closure: false,
            in_on_failure: false,
            fallible_ctx: None,
            wasm_target,
            or_value_discarded: false,
            generic_fns,
            generic_types,
        };
        for item in &program.items {
            cx.check_top_decl(item);
        }
    }
    // Bundle-level rules around topic bindings:
    //   - at most one `main` locus per bundle
    //   - bindings entries reference declared topics
    //   - duplicate bindings for the same topic are forbidden
    check_main_and_bindings(bundle, top, &mut diags);
    // Phase 3 routing-keys (2026-05-25): bundle-level checks
    //   - `on_unmatched: fallback` topics must have at least one
    //     `where key == _` subscriber program-wide.
    //   - `where key == _` is only legal on fallback topics.
    check_phase3_fallback_subscribers(bundle, &mut diags);
    // F.31 Phase 5: single-threaded-method invariant. Walks
    // method bodies looking for cross-pool `self.X.foo()` calls
    // where X's locus type is placed on a different pool than
    // self's. Cross-pool coordination must go through the bus,
    // not a direct method call. See spec/types.md
    // § "Single-threaded-method invariant (F.31)".
    check_placement_single_thread(bundle, top, &mut diags);
    // F.31-followup (2026-05-28): the nested-long-running-child
    // antipattern. A non-main locus whose `run()` body has work
    // to do, holding a params field of a locus type whose own
    // `run()` doesn't return (or is on the known-long-running
    // stdlib list), gets a hard error pointing at the canonical
    // sibling-in-main + placement fix. See `spec/runtime.md §
    // Long-running cooperative children`.
    check_nested_long_running_child(bundle, &mut diags);
    check_cooperative_pool_blocking(bundle, &mut diags);
    // 2026-05-29: a bus-subscribing locus instantiated non-owned
    // inside another locus's method/handler body dissolves at that
    // method's scope exit, so its subscription can never fire.
    // Hard error unless `--allow-unowned-subscriber` is set.
    check_unowned_subscriber_locus(bundle, allow_unowned_subscriber, &mut diags);
    // GH #18 #4: bus-graph property checks over the typed topic
    // topology. v1 (PR A): orphan topics — declared/used subjects
    // wired to only one end. Gated on a closed-world program (a
    // `main` locus present), so library seeds whose consumers are
    // external aren't falsely flagged.
    check_bus_graph(bundle, top, &mut diags);
    // GH #18 #4 (PR B): bus-graph cycles. A cross-locus publish→
    // subscribe→publish loop spins the cooperative queue (warning);
    // an intra-locus loop is devirtualized synchronous self-dispatch
    // that recurses without bound (error).
    check_bus_cycles(bundle, &mut diags);
    // GH #18 #4: backpressure. An unbounded publish loop with no
    // yield/throttle floods the bus — the producer has no
    // backpressure. Structural heuristic (warning).
    check_bus_backpressure(bundle, &mut diags);
    // GH #18 #4: subject type-mismatch. Two literal-subject sites
    // addressing the same wire subject must agree on the payload
    // type — otherwise a subscriber decodes the publisher's bytes as
    // the wrong type. Declared `topic`s are already unified by their
    // declaration; this closes the literal-subject gap.
    check_bus_subject_types(bundle, &mut diags);
    diags
}

/// True if the locus declares at least one `bus { subscribe ... }`.
fn locus_has_bus_subscribe(l: &LocusDecl) -> bool {
    l.members.iter().any(|m| match m {
        LocusMember::Bus(b) => b
            .members
            .iter()
            .any(|bm| matches!(bm, BusMember::Subscribe { .. })),
        _ => false,
    })
}

/// True if `parent` declares `accept(c: <child_name>)` — i.e. it
/// owns instantiations of that locus type as children (accept
/// fires by type, regardless of let-vs-statement binding).
fn locus_accepts(parent: &LocusDecl, child_name: &str) -> bool {
    parent.members.iter().any(|m| match m {
        LocusMember::Lifecycle(lc)
            if lc.kind == LifecycleKind::Accept =>
        {
            lc.params.first().is_some_and(|p| {
                matches!(&p.ty, TypeExpr::Named { path, .. }
                    if path.segments.last()
                        .is_some_and(|s| s.name == child_name))
            })
        }
        _ => false,
    })
}

/// Collect single-segment locus-instantiation sites
/// (`L { ... }`) reachable in a method body, as
/// (locus_name, span). Filtered to actual loci by the caller.
fn collect_locus_instantiations(
    block: &Block,
    out: &mut Vec<(String, Span)>,
) {
    for stmt in &block.stmts {
        collect_in_stmt(stmt, out);
    }
}

fn collect_in_stmt(stmt: &Stmt, out: &mut Vec<(String, Span)>) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            collect_in_expr(value, out)
        }
        Stmt::Assign { value, .. } => collect_in_expr(value, out),
        Stmt::Send { subject, value, .. } => {
            collect_in_expr(subject, out);
            collect_in_expr(value, out);
        }
        Stmt::If(i) => collect_in_if(i, out),
        Stmt::Match(m) => collect_in_match(m, out),
        Stmt::For { iter, body, .. } => {
            collect_in_expr(iter, out);
            collect_locus_instantiations(body, out);
        }
        Stmt::While { cond, body, .. } => {
            collect_in_expr(cond, out);
            collect_locus_instantiations(body, out);
        }
        Stmt::Return(Some(e), _) => collect_in_expr(e, out),
        Stmt::Fail { value, .. } => collect_in_expr(value, out),
        Stmt::Violate { payload: Some(e), .. } => collect_in_expr(e, out),
        Stmt::Recovery { args, .. } => {
            for a in args {
                collect_in_expr(a, out);
            }
        }
        Stmt::Block(b) => collect_locus_instantiations(b, out),
        Stmt::Expr(e) => collect_in_expr(e, out),
        _ => {}
    }
}

fn collect_in_if(stmt: &IfStmt, out: &mut Vec<(String, Span)>) {
    collect_in_expr(&stmt.cond, out);
    collect_locus_instantiations(&stmt.then_block, out);
    if let Some(eb) = &stmt.else_block {
        match eb.as_ref() {
            ElseBranch::Else(b) => collect_locus_instantiations(b, out),
            ElseBranch::ElseIf(nested) => collect_in_if(nested, out),
        }
    }
}

fn collect_in_match(stmt: &MatchStmt, out: &mut Vec<(String, Span)>) {
    collect_in_expr(&stmt.scrutinee, out);
    for arm in &stmt.arms {
        if let Some(g) = &arm.guard {
            collect_in_expr(g, out);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => collect_in_expr(e, out),
            MatchArmBody::Block(b) => collect_locus_instantiations(b, out),
        }
    }
}

fn collect_in_expr(expr: &Expr, out: &mut Vec<(String, Span)>) {
    match expr {
        Expr::Struct { path, inits, span } => {
            if path.segments.len() == 1 {
                out.push((path.segments[0].name.clone(), *span));
            }
            for init in inits {
                collect_in_expr(&init.value, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_in_expr(left, out);
            collect_in_expr(right, out);
        }
        Expr::Unary { operand, .. } => collect_in_expr(operand, out),
        Expr::Call { callee, args, .. } => {
            collect_in_expr(callee, out);
            for a in args {
                collect_in_expr(a, out);
            }
        }
        Expr::Field { receiver, .. }
        | Expr::Path2 { receiver, .. } => collect_in_expr(receiver, out),
        Expr::Index { receiver, index, .. } => {
            collect_in_expr(receiver, out);
            collect_in_expr(index, out);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for e in es {
                collect_in_expr(e, out);
            }
        }
        Expr::Block(b) => collect_locus_instantiations(b, out),
        Expr::If(i) => collect_in_if(i, out),
        Expr::Match(m) => collect_in_match(m, out),
        Expr::Sum(e, _) | Expr::Prod(e, _) => collect_in_expr(e, out),
        Expr::Approx { left, right, tolerance, .. } => {
            collect_in_expr(left, out);
            collect_in_expr(right, out);
            collect_in_expr(tolerance, out);
        }
        Expr::Range { lo, hi, .. } => {
            collect_in_expr(lo, out);
            collect_in_expr(hi, out);
        }
        Expr::ArrayRepeat { val, .. } => collect_in_expr(val, out),
        Expr::Or { inner, disposition, .. } => {
            collect_in_expr(inner, out);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    collect_in_expr(e, out)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn check_unowned_subscriber_locus(
    bundle: &Bundle<'_>,
    allow: bool,
    diags: &mut Vec<Diag>,
) {
    if allow {
        return;
    }
    let mut local_loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Locus(l) = item {
                local_loci.insert(l.name.name.as_str(), l);
            }
        }
    }

    for program in bundle.programs.values() {
        for item in &program.items {
            let TopDecl::Locus(p) = item else {
                continue;
            };
            // Collect this locus's bus-handler fn names. The
            // antipattern is narrow on purpose: a subscriber
            // spawned in a BUS HANDLER body is unambiguously
            // broken — a handler returns after each message, so
            // the spawned subscriber dissolves before it could
            // receive the next one. A subscriber spawned in
            // `run()` / `birth()` / a plain method is NOT flagged:
            // it lives for that scope and can legitimately receive
            // messages published during it (the canonical
            // `run()` spawns N watchers then publishes` pattern).
            let mut handler_names: std::collections::BTreeSet<&str> =
                std::collections::BTreeSet::new();
            for member in &p.members {
                if let LocusMember::Bus(b) = member {
                    for bm in &b.members {
                        if let BusMember::Subscribe { handler, .. } = bm {
                            handler_names.insert(handler.name.as_str());
                        }
                    }
                }
            }
            if handler_names.is_empty() {
                continue;
            }
            for member in &p.members {
                let LocusMember::Fn(fd) = member else {
                    continue;
                };
                if !handler_names.contains(fd.name.name.as_str()) {
                    continue;
                }
                let mut hits: Vec<(String, Span)> = Vec::new();
                collect_locus_instantiations(&fd.body, &mut hits);
                for (name, span) in hits {
                    let Some(child) = local_loci.get(name.as_str()) else {
                        continue;
                    };
                    if !locus_has_bus_subscribe(child) {
                        continue;
                    }
                    if locus_accepts(p, &name) {
                        continue; // owned as a child — fine
                    }
                    diags.push(Diag::ty(
                        span,
                        format!(
                            "locus `{}` declares `bus subscribe` but is \
                             instantiated unowned inside `{}`'s bus handler \
                             `{}`. A bus handler returns after each message, \
                             so the locals it binds dissolve immediately — \
                             `{}`'s subscription would never fire for a later \
                             message.\n\n\
                             Own it so it shares the parent's lifetime \
                             instead of the handler's:\n\
                             - `accept(c: {})` on `{}` (child membership; the \
                             canonical N-dynamic-children shape), or\n\
                             - a capacity pool / params field of `{}`.\n\n\
                             If you manage its lifetime another way, pass \
                             `--allow-unowned-subscriber` to downgrade this \
                             to allowed.",
                            name,
                            p.name.name,
                            fd.name.name,
                            name,
                            name,
                            p.name.name,
                            p.name.name,
                        ),
                    ));
                }
            }
        }
    }
}

/// Known stdlib loci whose `run()` body is structurally non-
/// terminating (accept loops, daemon loops). Used by the
/// nested-long-running-child check; the typechecker can't see
/// stdlib bodies, so the list is maintained explicitly.
const KNOWN_LONG_RUNNING_STDLIB_LOCI: &[&[&str]] = &[
    &["std", "http", "Server"],
];

fn is_known_long_running_stdlib(path_segments: &[&str]) -> bool {
    KNOWN_LONG_RUNNING_STDLIB_LOCI
        .iter()
        .any(|known| *known == path_segments)
}

fn locus_has_nontrivial_run(l: &LocusDecl) -> bool {
    l.members.iter().any(|m| match m {
        LocusMember::Lifecycle(LifecycleDecl {
            kind: LifecycleKind::Run,
            body,
            ..
        }) => !body.stmts.is_empty(),
        _ => false,
    })
}

/// Stdlib path calls that block the calling OS thread until the I/O
/// completes. A cooperative (non-`async_io`) locus that runs one in
/// its `run()` loop holds the pool's thread for the call's whole
/// duration — stalling every other locus scheduled on that pool and
/// the pool's bus drain. (`async_io` parks instead of blocking;
/// `pinned` owns its own thread.) The warning path follows the call
/// graph interprocedurally — a `run()` that blocks through a helper
/// fn or a `self.method` is flagged (see `find_blocking_deep_in_block`
/// and the `blocking_*_fns` fixpoints). Still best-effort: blocking
/// via a method on a stdlib *handle* (`stream.recv(...)`) or across a
/// cross-locus `self.field.method()` hop isn't traced — this is a
/// warning, so the residual incompleteness is acceptable.
const BLOCKING_STDLIB_PATHS: &[&[&str]] = &[
    &["std", "io", "tcp", "recv_into"],
    &["std", "io", "tcp", "recv_stamped_into"],
    &["std", "io", "tcp", "__recv"],
    &["std", "io", "tcp", "__recv_bytes"],
    &["std", "io", "tcp", "__accept_one"],
    &["std", "io", "tls", "recv_into"],
    &["std", "io", "tls", "recv_stamped_into"],
    &["std", "io", "tls", "recv_bytes"],
    &["std", "process", "run"],
    &["std", "process", "wait"],
    &["std", "process", "__wait_pid"],
];

fn blocking_path_match(segs: &[&str]) -> Option<String> {
    BLOCKING_STDLIB_PATHS
        .iter()
        .find(|p| **p == segs)
        .map(|p| p.join("::"))
}

fn find_blocking_in_block(block: &Block) -> Option<(String, Span)> {
    block.stmts.iter().find_map(find_blocking_in_stmt)
}

fn find_blocking_in_stmt(stmt: &Stmt) -> Option<(String, Span)> {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            find_blocking_in_expr(value)
        }
        Stmt::Assign { value, .. } => find_blocking_in_expr(value),
        Stmt::Send { subject, value, .. } => {
            find_blocking_in_expr(subject).or_else(|| find_blocking_in_expr(value))
        }
        Stmt::Return(Some(e), _) => find_blocking_in_expr(e),
        Stmt::Fail { value, .. } => find_blocking_in_expr(value),
        Stmt::Violate { payload: Some(e), .. } => find_blocking_in_expr(e),
        Stmt::Recovery { args, .. } => args.iter().find_map(find_blocking_in_expr),
        Stmt::Expr(e) => find_blocking_in_expr(e),
        Stmt::If(i) => find_blocking_in_if(i),
        Stmt::Match(m) => find_blocking_in_match(m),
        Stmt::For { iter, body, .. } => {
            find_blocking_in_expr(iter).or_else(|| find_blocking_in_block(body))
        }
        Stmt::While { cond, body, .. } => {
            find_blocking_in_expr(cond).or_else(|| find_blocking_in_block(body))
        }
        Stmt::Block(b) => find_blocking_in_block(b),
        _ => None,
    }
}

fn find_blocking_in_if(i: &IfStmt) -> Option<(String, Span)> {
    find_blocking_in_expr(&i.cond)
        .or_else(|| find_blocking_in_block(&i.then_block))
        .or_else(|| match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => find_blocking_in_block(b),
            Some(ElseBranch::ElseIf(n)) => find_blocking_in_if(n),
            None => None,
        })
}

fn find_blocking_in_match(m: &MatchStmt) -> Option<(String, Span)> {
    find_blocking_in_expr(&m.scrutinee).or_else(|| {
        m.arms.iter().find_map(|arm| {
            arm.guard
                .as_ref()
                .and_then(find_blocking_in_expr)
                .or_else(|| match &arm.body {
                    MatchArmBody::Expr(e) => find_blocking_in_expr(e),
                    MatchArmBody::Block(b) => find_blocking_in_block(b),
                })
        })
    })
}

fn find_blocking_in_expr(expr: &Expr) -> Option<(String, Span)> {
    match expr {
        Expr::Call { callee, args, span } => {
            if let Expr::Path(qn) = callee.as_ref() {
                let segs: Vec<&str> =
                    qn.segments.iter().map(|s| s.name.as_str()).collect();
                if let Some(name) = blocking_path_match(&segs) {
                    return Some((name, *span));
                }
            }
            find_blocking_in_expr(callee)
                .or_else(|| args.iter().find_map(find_blocking_in_expr))
        }
        Expr::Binary { left, right, .. } => {
            find_blocking_in_expr(left).or_else(|| find_blocking_in_expr(right))
        }
        Expr::Unary { operand, .. } => find_blocking_in_expr(operand),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            find_blocking_in_expr(receiver)
        }
        Expr::Index { receiver, index, .. } => {
            find_blocking_in_expr(receiver).or_else(|| find_blocking_in_expr(index))
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().find_map(find_blocking_in_expr)
        }
        Expr::Struct { inits, .. } => {
            inits.iter().find_map(|i| find_blocking_in_expr(&i.value))
        }
        Expr::Block(b) => find_blocking_in_block(b),
        Expr::If(i) => find_blocking_in_if(i),
        Expr::Match(m) => find_blocking_in_match(m),
        Expr::Sum(e, _) | Expr::Prod(e, _) => find_blocking_in_expr(e),
        Expr::Or { inner, disposition, .. } => find_blocking_in_expr(inner)
            .or_else(|| match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    find_blocking_in_expr(e)
                }
                _ => None,
            }),
        _ => None,
    }
}

// === Interprocedural blocking detection (warning path only) =========
//
// The direct-call walk above only sees blocking ops written literally
// in `run()`. A `run()` that calls a helper fn — `self.drain()` or a
// free `pump(conn)` — that itself blocks holds the pool's thread just
// as surely, but escaped the syntactic walk. These helpers build a
// small call graph and propagate "blocks" from leaf stdlib ops up
// through callees, so the pool-stall **warning** also fires on
// blocking reached one or more fn-hops deep. (The dead-receiver ERROR
// deliberately stays direct-call-only — it over-fired once before, so
// we don't widen its call-graph surface; see
// `check_cooperative_pool_blocking`.)

/// Names a call expression's callee resolves to, split into free-fn
/// names (bare ident / single-segment path) and `self.method` names.
/// Used to build the call graph; over-collection is harmless (the
/// fixpoint only follows edges into fns it actually knows).
#[derive(Default)]
struct CalleeSet {
    free: BTreeSet<String>,
    self_methods: BTreeSet<String>,
}

fn collect_callees_in_block(b: &Block, out: &mut CalleeSet) {
    for s in &b.stmts {
        collect_callees_in_stmt(s, out);
    }
}

fn collect_callees_in_stmt(stmt: &Stmt, out: &mut CalleeSet) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            collect_callees_in_expr(value, out)
        }
        Stmt::Assign { value, .. } => collect_callees_in_expr(value, out),
        Stmt::Send { subject, value, .. } => {
            collect_callees_in_expr(subject, out);
            collect_callees_in_expr(value, out);
        }
        Stmt::Return(Some(e), _) => collect_callees_in_expr(e, out),
        Stmt::Fail { value, .. } => collect_callees_in_expr(value, out),
        Stmt::Violate { payload: Some(e), .. } => collect_callees_in_expr(e, out),
        Stmt::Recovery { args, .. } => {
            args.iter().for_each(|e| collect_callees_in_expr(e, out))
        }
        Stmt::Expr(e) => collect_callees_in_expr(e, out),
        Stmt::If(i) => collect_callees_in_if(i, out),
        Stmt::Match(m) => collect_callees_in_match(m, out),
        Stmt::For { iter, body, .. } => {
            collect_callees_in_expr(iter, out);
            collect_callees_in_block(body, out);
        }
        Stmt::While { cond, body, .. } => {
            collect_callees_in_expr(cond, out);
            collect_callees_in_block(body, out);
        }
        Stmt::Block(b) => collect_callees_in_block(b, out),
        _ => {}
    }
}

fn collect_callees_in_if(i: &IfStmt, out: &mut CalleeSet) {
    collect_callees_in_expr(&i.cond, out);
    collect_callees_in_block(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => collect_callees_in_block(b, out),
        Some(ElseBranch::ElseIf(n)) => collect_callees_in_if(n, out),
        None => {}
    }
}

fn collect_callees_in_match(m: &MatchStmt, out: &mut CalleeSet) {
    collect_callees_in_expr(&m.scrutinee, out);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            collect_callees_in_expr(g, out);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => collect_callees_in_expr(e, out),
            MatchArmBody::Block(b) => collect_callees_in_block(b, out),
        }
    }
}

/// Record the callee a `Call` resolves to (if a free fn or
/// `self.method`), then recurse into sub-expressions.
fn collect_callees_in_expr(expr: &Expr, out: &mut CalleeSet) {
    match expr {
        Expr::Call { callee, args, .. } => {
            match callee.as_ref() {
                Expr::Ident(id) => {
                    out.free.insert(id.name.clone());
                }
                Expr::Path(qn) if qn.segments.len() == 1 => {
                    out.free.insert(qn.segments[0].name.clone());
                }
                Expr::Field { receiver, name, .. }
                    if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
                {
                    out.self_methods.insert(name.name.clone());
                }
                _ => {}
            }
            collect_callees_in_expr(callee, out);
            args.iter().for_each(|a| collect_callees_in_expr(a, out));
        }
        Expr::Binary { left, right, .. } => {
            collect_callees_in_expr(left, out);
            collect_callees_in_expr(right, out);
        }
        Expr::Unary { operand, .. } => collect_callees_in_expr(operand, out),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            collect_callees_in_expr(receiver, out)
        }
        Expr::Index { receiver, index, .. } => {
            collect_callees_in_expr(receiver, out);
            collect_callees_in_expr(index, out);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().for_each(|e| collect_callees_in_expr(e, out))
        }
        Expr::Struct { inits, .. } => {
            inits.iter().for_each(|i| collect_callees_in_expr(&i.value, out))
        }
        Expr::Block(b) => collect_callees_in_block(b, out),
        Expr::If(i) => collect_callees_in_if(i, out),
        Expr::Match(m) => collect_callees_in_match(m, out),
        Expr::Sum(e, _) | Expr::Prod(e, _) => collect_callees_in_expr(e, out),
        Expr::Or { inner, disposition, .. } => {
            collect_callees_in_expr(inner, out);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    collect_callees_in_expr(e, out)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// The set of free-fn names that block — directly (a leaf stdlib op in
/// the body) or transitively (they call a blocking free fn).
/// Fixpoint over the free-fn call graph; free fns can't reference
/// `self`, so they only depend on other free fns.
fn blocking_free_fns(free_fns: &BTreeMap<String, &Block>) -> BTreeSet<String> {
    let mut blocking: BTreeSet<String> = free_fns
        .iter()
        .filter(|(_, body)| find_blocking_in_block(body).is_some())
        .map(|(n, _)| n.clone())
        .collect();
    let mut callees: BTreeMap<&str, CalleeSet> = BTreeMap::new();
    for (n, body) in free_fns {
        let mut cs = CalleeSet::default();
        collect_callees_in_block(body, &mut cs);
        callees.insert(n.as_str(), cs);
    }
    loop {
        let mut changed = false;
        for (n, cs) in &callees {
            if blocking.contains(*n) {
                continue;
            }
            if cs.free.iter().any(|c| blocking.contains(c)) {
                blocking.insert((*n).to_string());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    blocking
}

/// The set of a locus's own method names that block — directly, via a
/// blocking free fn, or via another blocking method on the same locus.
/// Seeded by `blocking_free`; fixpoint over the intra-locus method
/// call graph.
fn blocking_self_methods(
    methods: &BTreeMap<String, &Block>,
    blocking_free: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut callees: BTreeMap<&str, CalleeSet> = BTreeMap::new();
    let mut blocking: BTreeSet<String> = BTreeSet::new();
    for (n, body) in methods {
        let mut cs = CalleeSet::default();
        collect_callees_in_block(body, &mut cs);
        if find_blocking_in_block(body).is_some()
            || cs.free.iter().any(|c| blocking_free.contains(c))
        {
            blocking.insert(n.clone());
        }
        callees.insert(n.as_str(), cs);
    }
    loop {
        let mut changed = false;
        for (n, cs) in &callees {
            if blocking.contains(*n) {
                continue;
            }
            if cs.self_methods.iter().any(|c| blocking.contains(c)) {
                blocking.insert((*n).to_string());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    blocking
}

/// Interprocedural form of `find_blocking_in_block`: reports the first
/// blocking reach in `run()` — a direct leaf stdlib op, OR a call to a
/// blocking free fn / `self.method`. The returned span is the
/// run()-level call site; the name describes what blocks. Used for the
/// pool-stall warning only.
fn find_blocking_deep_in_block(
    b: &Block,
    blocking_free: &BTreeSet<String>,
    blocking_self: &BTreeSet<String>,
) -> Option<(String, Span)> {
    b.stmts
        .iter()
        .find_map(|s| find_blocking_deep_in_stmt(s, blocking_free, blocking_self))
}

fn find_blocking_deep_in_stmt(
    stmt: &Stmt,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            find_blocking_deep_in_expr(value, bf, bs)
        }
        Stmt::Assign { value, .. } => find_blocking_deep_in_expr(value, bf, bs),
        Stmt::Send { subject, value, .. } => {
            find_blocking_deep_in_expr(subject, bf, bs)
                .or_else(|| find_blocking_deep_in_expr(value, bf, bs))
        }
        Stmt::Return(Some(e), _) => find_blocking_deep_in_expr(e, bf, bs),
        Stmt::Fail { value, .. } => find_blocking_deep_in_expr(value, bf, bs),
        Stmt::Violate { payload: Some(e), .. } => {
            find_blocking_deep_in_expr(e, bf, bs)
        }
        Stmt::Recovery { args, .. } => {
            args.iter().find_map(|e| find_blocking_deep_in_expr(e, bf, bs))
        }
        Stmt::Expr(e) => find_blocking_deep_in_expr(e, bf, bs),
        Stmt::If(i) => find_blocking_deep_in_if(i, bf, bs),
        Stmt::Match(m) => find_blocking_deep_in_match(m, bf, bs),
        Stmt::For { iter, body, .. } => find_blocking_deep_in_expr(iter, bf, bs)
            .or_else(|| find_blocking_deep_in_block(body, bf, bs)),
        Stmt::While { cond, body, .. } => find_blocking_deep_in_expr(cond, bf, bs)
            .or_else(|| find_blocking_deep_in_block(body, bf, bs)),
        Stmt::Block(b) => find_blocking_deep_in_block(b, bf, bs),
        _ => None,
    }
}

fn find_blocking_deep_in_if(
    i: &IfStmt,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    find_blocking_deep_in_expr(&i.cond, bf, bs)
        .or_else(|| find_blocking_deep_in_block(&i.then_block, bf, bs))
        .or_else(|| match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => find_blocking_deep_in_block(b, bf, bs),
            Some(ElseBranch::ElseIf(n)) => find_blocking_deep_in_if(n, bf, bs),
            None => None,
        })
}

fn find_blocking_deep_in_match(
    m: &MatchStmt,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    find_blocking_deep_in_expr(&m.scrutinee, bf, bs).or_else(|| {
        m.arms.iter().find_map(|arm| {
            arm.guard
                .as_ref()
                .and_then(|g| find_blocking_deep_in_expr(g, bf, bs))
                .or_else(|| match &arm.body {
                    MatchArmBody::Expr(e) => find_blocking_deep_in_expr(e, bf, bs),
                    MatchArmBody::Block(b) => find_blocking_deep_in_block(b, bf, bs),
                })
        })
    })
}

fn find_blocking_deep_in_expr(
    expr: &Expr,
    bf: &BTreeSet<String>,
    bs: &BTreeSet<String>,
) -> Option<(String, Span)> {
    match expr {
        Expr::Call { callee, args, span } => {
            match callee.as_ref() {
                Expr::Path(qn) => {
                    let segs: Vec<&str> =
                        qn.segments.iter().map(|s| s.name.as_str()).collect();
                    if let Some(name) = blocking_path_match(&segs) {
                        return Some((name, *span));
                    }
                    if segs.len() == 1 && bf.contains(segs[0]) {
                        return Some((
                            format!("{}() (which makes a blocking call)", segs[0]),
                            *span,
                        ));
                    }
                }
                Expr::Ident(id) if bf.contains(&id.name) => {
                    return Some((
                        format!("{}() (which makes a blocking call)", id.name),
                        *span,
                    ));
                }
                Expr::Field { receiver, name, .. }
                    if matches!(receiver.as_ref(), Expr::KwSelf(_))
                        && bs.contains(&name.name) =>
                {
                    return Some((
                        format!("self.{}() (which makes a blocking call)", name.name),
                        *span,
                    ));
                }
                _ => {}
            }
            find_blocking_deep_in_expr(callee, bf, bs)
                .or_else(|| args.iter().find_map(|a| find_blocking_deep_in_expr(a, bf, bs)))
        }
        Expr::Binary { left, right, .. } => find_blocking_deep_in_expr(left, bf, bs)
            .or_else(|| find_blocking_deep_in_expr(right, bf, bs)),
        Expr::Unary { operand, .. } => find_blocking_deep_in_expr(operand, bf, bs),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            find_blocking_deep_in_expr(receiver, bf, bs)
        }
        Expr::Index { receiver, index, .. } => {
            find_blocking_deep_in_expr(receiver, bf, bs)
                .or_else(|| find_blocking_deep_in_expr(index, bf, bs))
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().find_map(|e| find_blocking_deep_in_expr(e, bf, bs))
        }
        Expr::Struct { inits, .. } => {
            inits.iter().find_map(|i| find_blocking_deep_in_expr(&i.value, bf, bs))
        }
        Expr::Block(b) => find_blocking_deep_in_block(b, bf, bs),
        Expr::If(i) => find_blocking_deep_in_if(i, bf, bs),
        Expr::Match(m) => find_blocking_deep_in_match(m, bf, bs),
        Expr::Sum(e, _) | Expr::Prod(e, _) => find_blocking_deep_in_expr(e, bf, bs),
        Expr::Or { inner, disposition, .. } => {
            find_blocking_deep_in_expr(inner, bf, bs).or_else(|| match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    find_blocking_deep_in_expr(e, bf, bs)
                }
                _ => None,
            })
        }
        _ => None,
    }
}

/// Warn when a locus placed `cooperative(pool = X)` without
/// `where async_io` calls a known-blocking stdlib op in its `run()`.
/// Such a call holds the pool's OS thread, starving co-scheduled loci
/// (this silently bricked a downstream team's metrics server when a
/// blocking gateway was moved onto a shared pool). A warning, not an
/// error — a single-purpose blocking server with nothing co-scheduled
/// is legitimate; the smell is real but situational.
/// A comparable key for a bus subject (topic name / literal /
/// qualified path) — used to tell whether a subscription is to a
/// topic the locus also publishes.
fn bus_subject_key(s: &BusSubject) -> String {
    match s {
        BusSubject::Literal { subject, .. } => subject.clone(),
        BusSubject::Topic(id) => id.name.clone(),
        BusSubject::QualifiedTopic(qn) => qn
            .segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::"),
    }
}

/// Handler names for the locus's `subscribe` entries on topics it
/// does NOT itself publish — i.e. genuine cross-context receives. A
/// self-publish→self-subscribe is devirtualized to a direct
/// `self.handler(...)` call (same instance, same thread), not a bus
/// receive, so it's excluded.
fn external_subscription_handlers(decl: &LocusDecl) -> Vec<String> {
    let mut published: BTreeSet<String> = BTreeSet::new();
    let mut handlers: Vec<(String, String)> = Vec::new(); // (subject_key, handler)
    for m in &decl.members {
        let LocusMember::Bus(b) = m else { continue };
        for bm in &b.members {
            match bm {
                BusMember::Publish { subject, .. } => {
                    published.insert(bus_subject_key(subject));
                }
                BusMember::Subscribe { subject, handler, .. } => {
                    handlers.push((bus_subject_key(subject), handler.name.clone()));
                }
            }
        }
    }
    handlers
        .into_iter()
        .filter(|(subj, _)| !published.contains(subj))
        .map(|(_, h)| h)
        .collect()
}

/// Flag blocking calls on cooperative pools. Two outcomes:
///   * a non-main cooperative SUBSCRIBER whose `run()` blocks is a
///     **dead receiver** (error) — the blocking call starves the
///     dispatch that would deliver to its handlers;
///   * any other cooperative locus whose `run()` blocks gets a
///     **warning** — it stalls co-scheduled loci on the pool.
/// An event-driven subscriber (no blocking call — handlers + a sleep
/// loop, or `where async_io`) is flagged by neither: it receives fine.
fn check_cooperative_pool_blocking(
    bundle: &Bundle<'_>,
    diags: &mut Vec<Diag>,
) {
    let mut local_loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    let mut free_fns: BTreeMap<String, &Block> = BTreeMap::new();
    for program in bundle.programs.values() {
        for item in &program.items {
            match item {
                TopDecl::Locus(l) => {
                    local_loci.insert(l.name.name.as_str(), l);
                }
                TopDecl::Fn(f) => {
                    free_fns.insert(f.name.name.clone(), &f.body);
                }
                _ => {}
            }
        }
    }
    // Interprocedural call graph for the warning path: free fns that
    // block (directly or via another blocking free fn).
    let blocking_free = blocking_free_fns(&free_fns);

    for program in bundle.programs.values() {
        for item in &program.items {
            let TopDecl::Locus(main) = item else { continue };
            if !main.is_main {
                continue;
            }
            let Some(pb) = main.members.iter().find_map(|m| match m {
                LocusMember::Placement(pb) => Some(pb),
                _ => None,
            }) else {
                continue;
            };
            // field name -> single-segment locus type name.
            let mut field_locus: BTreeMap<&str, &str> = BTreeMap::new();
            for m in &main.members {
                let LocusMember::Params(params) = m else { continue };
                for pd in &params.params {
                    if let Some(TypeExpr::Named { path, .. }) = &pd.ty {
                        if path.segments.len() == 1 {
                            field_locus.insert(
                                pd.name.name.as_str(),
                                path.segments[0].name.as_str(),
                            );
                        }
                    }
                }
            }

            for entry in &pb.entries {
                let PlacementSpec::Cooperative { pool } = &entry.spec else {
                    continue;
                };
                // `where async_io` parks blocking I/O — not a stall.
                if entry
                    .constraints
                    .iter()
                    .any(|c| matches!(c.kind, PlacementConstraint::AsyncIo))
                {
                    continue;
                }
                let Some(locus_name) = field_locus.get(entry.field.name.as_str())
                else {
                    continue;
                };
                let Some(decl) = local_loci.get(locus_name) else {
                    continue;
                };
                let Some(run_body) = decl.members.iter().find_map(|m| match m {
                    LocusMember::Lifecycle(LifecycleDecl {
                        kind: LifecycleKind::Run,
                        body,
                        ..
                    }) => Some(body),
                    _ => None,
                }) else {
                    continue;
                };
                // The locus's own methods (named fns + lifecycle
                // bodies) form the intra-locus call graph for the
                // interprocedural warning.
                let mut methods: BTreeMap<String, &Block> = BTreeMap::new();
                for m in &decl.members {
                    match m {
                        LocusMember::Fn(f) => {
                            methods.insert(f.name.name.clone(), &f.body);
                        }
                        LocusMember::Lifecycle(LifecycleDecl {
                            kind, body, ..
                        }) => {
                            methods.insert(format!("{:?}", kind), body);
                        }
                        _ => {}
                    }
                }
                let blocking_self =
                    blocking_self_methods(&methods, &blocking_free);

                // WARNING trigger: blocking reachable from run() either
                // directly or through a helper fn / self-method.
                let Some((deep_call, deep_span)) = find_blocking_deep_in_block(
                    run_body,
                    &blocking_free,
                    &blocking_self,
                ) else {
                    // Event-driven (nothing blocking reachable): the
                    // pool thread stays free, the bus dispatch runs,
                    // cells arrive. Nothing to flag — even a non-main
                    // cooperative subscriber receives fine this way.
                    continue;
                };
                // DEAD-RECEIVER trigger stays direct-call-only — its
                // call-graph surface is deliberately NOT widened (it
                // over-fired once; see below).
                let direct = find_blocking_in_block(run_body);
                let pool_name =
                    pool.as_ref().map(|i| i.name.as_str()).unwrap_or("main");
                // Handlers for topics this locus does NOT itself publish
                // (a self-publish→subscribe is a devirtualized direct
                // call, not a bus receive).
                let dead = external_subscription_handlers(decl);
                if pool_name != "main" && !dead.is_empty() && direct.is_some() {
                    let (call, span) = direct.expect("is_some checked");
                    // DEAD RECEIVER (error). A non-main cooperative
                    // subscriber whose run() blocks: cross-process
                    // dispatch reaches a cooperative locus only when its
                    // pool thread is free to run the dispatch, and a
                    // blocking call monopolizes it, so these handlers
                    // never fire. (Corrected 2026-06-03 from the
                    // placement-only rule, which over-fired on
                    // event-driven subscribers — `PriceView`/`WsDispatcher`
                    // received fine for 16h+ in production.)
                    diags.push(Diag::ty(
                        span,
                        format!(
                            "locus `{}` (field `{}`) subscribes to bus topics \
                             ({}) but its `run()` makes the blocking call `{}` \
                             while placed `cooperative(pool = {})`. The \
                             blocking call monopolizes the pool's thread, so \
                             the dispatch that would deliver those cells never \
                             runs — the handlers can't fire. (An event-driven \
                             subscriber that yields — handlers plus a \
                             `time::sleep` loop, or `where async_io` — receives \
                             fine; the problem is the blocking call, not the \
                             placement.) Use `pinned` (its own thread + a \
                             mailbox drained at sleep/yield), or keep `run()` \
                             non-blocking.",
                            locus_name,
                            entry.field.name,
                            dead.join(", "),
                            call,
                            pool_name,
                        ),
                    ));
                } else {
                    // Blocking on a cooperative pool stalls co-scheduled
                    // loci (and the pool's bus drain), even when this
                    // locus isn't itself a subscriber. Interprocedural:
                    // `deep_call` may name a helper fn / self-method that
                    // blocks transitively, not just a literal stdlib op.
                    diags.push(Diag::warn(
                        deep_span,
                        format!(
                            "locus `{}` (field `{}`) is placed `cooperative(pool \
                             = {})` and reaches the blocking call `{}` in its \
                             `run()`. A blocking call holds the pool's OS thread \
                             for its whole duration, stalling every other locus \
                             scheduled on `{}` (and the pool's bus drain). Use \
                             `pinned` (its own thread — the prescribed shape for \
                             blocking I/O), or `cooperative(pool = {}) where \
                             async_io` (which parks on I/O readiness instead of \
                             blocking the thread).",
                            locus_name,
                            entry.field.name,
                            pool_name,
                            deep_call,
                            pool_name,
                            pool_name,
                        ),
                    ));
                }
            }
        }
    }
}

fn check_nested_long_running_child(
    bundle: &Bundle<'_>,
    diags: &mut Vec<Diag>,
) {
    // Build a name → LocusDecl index across the bundle so we can
    // resolve params-field locus types to their target body.
    let mut local_loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Locus(l) = item {
                local_loci.insert(l.name.name.as_str(), l);
            }
        }
    }

    for program in bundle.programs.values() {
        for item in &program.items {
            let TopDecl::Locus(parent) = item else {
                continue;
            };
            if parent.is_main {
                continue;
            }
            if !locus_has_nontrivial_run(parent) {
                continue;
            }
            // Walk params fields. Each ParamDecl whose declared
            // type is a locus reference goes through the locus-
            // type-with-run check.
            for member in &parent.members {
                let LocusMember::Params(pb) = member else {
                    continue;
                };
                for pd in &pb.params {
                    let Some(ty) = &pd.ty else {
                        continue;
                    };
                    let TypeExpr::Named { path, .. } = ty else {
                        continue;
                    };
                    let segs: Vec<&str> = path
                        .segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect();
                    // Single-segment names: look up locally.
                    // Multi-segment: check against the known-
                    // long-running stdlib allowlist.
                    let target_is_long_running = if segs.len() == 1 {
                        local_loci
                            .get(segs[0])
                            .filter(|l| !l.is_main)
                            .map(|l| locus_has_nontrivial_run(l))
                            .unwrap_or(false)
                    } else {
                        is_known_long_running_stdlib(&segs)
                    };
                    if !target_is_long_running {
                        continue;
                    }
                    let target_display = segs.join("::");
                    diags.push(Diag::ty(
                        pd.span,
                        format!(
                            "locus `{}` declares params field `{}: {}` \
                             with a non-trivial `run()` body of its own. \
                             Nested cooperative children share the parent's \
                             OS thread; the child's `run()` runs to \
                             completion before the parent's `run()` begins, \
                             so a long-running child (`{}`'s accept loop \
                             never returns) starves the parent.\n\n\
                             Canonical fix: hoist both loci to siblings of \
                             a `main locus` and use a `placement {{ }}` \
                             block to put them on different pools.\n\n\
                             ```\n\
                             main locus App {{\n\
                                 params {{\n\
                                     parent: {} = {} {{ ... }};\n\
                                     {}: {} = {} {{ ... }};\n\
                                 }}\n\
                                 placement {{\n\
                                     {}: cooperative(pool = io);\n\
                                 }}\n\
                             }}\n\
                             ```\n\n\
                             See spec/runtime.md § Long-running cooperative \
                             children: placement closes Item D.",
                            parent.name.name,
                            pd.name.name,
                            target_display,
                            target_display,
                            parent.name.name,
                            parent.name.name,
                            pd.name.name,
                            target_display,
                            target_display,
                            pd.name.name,
                        ),
                    ));
                }
            }
        }
    }
}

/// F.31 Phase 5: pool identity. Each main-locus params field
/// gets one of these via the placement block (or default to
/// `Cooperative("main")`). Nested loci inherit the parent
/// tower's pool.
///
/// `Cooperative` pools are name-scoped — two loci on
/// `cooperative(pool = "io")` share an OS thread. `Pinned`
/// pools are uniquely identified by the owning field path
/// (each pinned locus spawns its own pthread, so two pinned
/// siblings — even of the same locus type — live on different
/// threads).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PoolId {
    Cooperative(String),
    /// Field-path string of the pinned locus's instantiation
    /// site, e.g. `"heartbeat"` for `main.heartbeat: pinned`.
    /// Uniqueness across pinned instances is the load-bearing
    /// property; the string is for diagnostics.
    Pinned(String),
}

impl PoolId {
    pub fn display(&self) -> String {
        match self {
            PoolId::Cooperative(name) => {
                format!("cooperative(pool = {})", name)
            }
            PoolId::Pinned(path) => format!("pinned (at `{}`)", path),
        }
    }
}

/// F.31 Phase 5 entry. Builds the per-locus-type pool map from
/// main's placement block, then walks every locus method body in
/// the bundle and flags direct `recv.foo(args)` calls whose
/// receiver resolves to a field of a locus type with a different
/// pool than the enclosing method's locus.
/// FUv0.8.2 #4 (2026-05-25): F.31 pool propagation extracted
/// as a pub helper so callers outside this module (the
/// `apply_sync_inference` finalization pass that runs before
/// codegen) can re-derive the map without re-running typecheck.
///
/// Seeds from the main locus's `placement { }` block, then
/// propagates the pool to each nested locus-typed param field.
/// First-wins on conflict — a single locus type appearing in
/// two towers with different pools is rare in v1; we pick the
/// first.
///
/// Returns an empty map for programs without a main locus
/// (free-fn-main scripts), so callers can skip the rest of
/// the analysis cheaply.
pub fn compute_pool_of_locus_type(
    bundle: &Bundle<'_>,
    top: &TopScope,
) -> BTreeMap<String, PoolId> {
    let mut main_locus: Option<&LocusDecl> = None;
    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Locus(l) = item {
                if l.is_main {
                    main_locus = Some(l);
                }
            }
        }
    }
    let Some(main) = main_locus else {
        return BTreeMap::new();
    };

    let placement_block = main.members.iter().find_map(|m| match m {
        LocusMember::Placement(pb) => Some(pb),
        _ => None,
    });
    let placement_map: BTreeMap<String, PoolId> = placement_block
        .map(|pb| {
            pb.entries
                .iter()
                .map(|e| {
                    (
                        e.field.name.clone(),
                        placement_spec_to_pool(&e.spec, &e.field.name),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let mut pool_of_locus_type: BTreeMap<String, PoolId> = BTreeMap::new();
    pool_of_locus_type.insert(
        main.name.name.clone(),
        PoolId::Cooperative("main".to_string()),
    );
    let main_params = main.members.iter().find_map(|m| match m {
        LocusMember::Params(pb) => Some(pb),
        _ => None,
    });
    if let Some(params) = main_params {
        for p in &params.params {
            let pool = placement_map
                .get(&p.name.name)
                .cloned()
                .unwrap_or_else(|| {
                    PoolId::Cooperative("main".to_string())
                });
            if let Some(ty) = &p.ty {
                if let Some(locus_name) = type_expr_locus_name(ty, top) {
                    pool_of_locus_type
                        .entry(locus_name.clone())
                        .or_insert_with(|| pool.clone());
                    walk_nested_loci(
                        &locus_name,
                        &pool,
                        top,
                        &mut pool_of_locus_type,
                    );
                }
            }
        }
    }
    pool_of_locus_type
}

fn check_placement_single_thread(
    bundle: &Bundle<'_>,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    let pool_of_locus_type = compute_pool_of_locus_type(bundle, top);
    if pool_of_locus_type.is_empty() {
        return;
    }
    // The main locus is needed downstream for the cross-pool
    // walk's `enclosing_locus`; re-locate it (cheap).
    let mut main_locus: Option<&LocusDecl> = None;
    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Locus(l) = item {
                if l.is_main {
                    main_locus = Some(l);
                }
            }
        }
    }
    let _main = main_locus;

    // 4. Walk every locus method body in the bundle and emit
    //    diagnostics for direct cross-pool calls. The check
    //    only flags `recv.foo(args)` shapes where `recv` is a
    //    field-access expression whose declared type names a
    //    locus with a known pool. Local-variable receivers,
    //    deeply-chained receivers, and stdlib/free-fn calls all
    //    fall back to OK (they need richer flow analysis we
    //    defer to v1.x).
    // F.32-0 (2026-05-24): collect locus types whose state is
    // held in `@form(...)` cells AND that carry an explicit
    // `sync = X` kwarg with X != `none`. The cross-pool
    // diagnostic is skipped only for these.
    //
    // History: 3ec6391 (2026-05-24, first cut) admitted any
    // `@form(...)` locus into this set on the assumption that
    // the form ABI serialized cell access. Bench-prep for
    // F.32-1 found the runtime (`lotus_arena.c:1869+`) has no
    // synchronization on `lotus_hashmap_set` / `_grow` — two
    // writers double-free during concurrent grow. F.32-0
    // scopes the exemption to explicitly-opted-in loci; the
    // sync disciplines themselves (α/β/γ) land in F.32-1.
    //
    // `form_bearing_loci` is a wider set (every @form locus,
    // sync or no sync) used only to tailor the diagnostic's
    // upgrade hint — receivers in this set get a "declare
    // `sync = ...` to opt in" suggestion.
    let mut cross_pool_safe_loci: BTreeSet<String> = BTreeSet::new();
    let mut form_bearing_loci: BTreeSet<String> = BTreeSet::new();
    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Locus(l) = item {
                if let Some(form) = &l.form {
                    form_bearing_loci.insert(l.name.name.clone());
                    if form_has_explicit_sync_discipline(form) {
                        cross_pool_safe_loci.insert(l.name.name.clone());
                    }
                }
            }
        }
    }

    // F.32-1∞ (2026-05-25): pre-compute sync inference for
    // every `@form(hashmap)` locus without explicit `sync = `.
    // The F.32-0 diagnostic below consults this map to name
    // the specific discipline the rule would pick, instead of
    // suggesting a generic "choose one of serialized/striped".
    let inferred_sync = crate::sync_inference::infer_sync_for_bundle(
        bundle,
        top,
        &pool_of_locus_type,
    );

    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Locus(l) = item {
                let caller_pool = pool_of_locus_type.get(&l.name.name);
                for member in &l.members {
                    if let Some(body) = locus_member_body(member) {
                        let mut cx = PoolCheckCx {
                            enclosing_locus: l,
                            caller_pool,
                            pool_of_locus_type: &pool_of_locus_type,
                            cross_pool_safe_loci: &cross_pool_safe_loci,
                            form_bearing_loci: &form_bearing_loci,
                            inferred_sync: &inferred_sync,
                            top,
                            diags,
                        };
                        walk_block_pool(body, &mut cx);
                    }
                }
            }
        }
    }
}

fn placement_spec_to_pool(
    spec: &hale_syntax::ast::PlacementSpec,
    field_name: &str,
) -> PoolId {
    use hale_syntax::ast::PlacementSpec;
    match spec {
        PlacementSpec::Cooperative { pool } => {
            let name = pool
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "main".to_string());
            PoolId::Cooperative(name)
        }
        PlacementSpec::Pinned { .. } => {
            PoolId::Pinned(field_name.to_string())
        }
    }
}

/// Resolve a type expression to a locus name, if the type
/// resolves to a `TopSymbol::Locus`. Returns `None` for
/// non-locus types or unresolved names.
fn type_expr_locus_name(ty: &TypeExpr, top: &TopScope) -> Option<String> {
    let TypeExpr::Named { path, .. } = ty else {
        return None;
    };
    if path.segments.len() != 1 {
        return None;
    }
    let name = &path.segments[0].name;
    match top.lookup(name) {
        Some(TopSymbol::Locus(_)) => Some(name.clone()),
        _ => None,
    }
}

/// Walk a locus type's params block transitively, propagating
/// the tower's pool to each nested locus-typed field. First-wins
/// on conflict.
fn walk_nested_loci(
    locus_name: &str,
    pool: &PoolId,
    top: &TopScope,
    map: &mut BTreeMap<String, PoolId>,
) {
    // We need the original LocusDecl to walk its params. The
    // bundle isn't threaded through here; instead use the
    // resolved LocusInfo's params from `top`. LocusInfo carries
    // Param `Ty` already resolved, so we walk those.
    let info = match top.lookup(locus_name) {
        Some(TopSymbol::Locus(l)) => l,
        _ => return,
    };
    for p in &info.params {
        let nested = match &p.ty {
            Ty::Named(n) => match top.lookup(n) {
                Some(TopSymbol::Locus(_)) => Some(n.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(child) = nested {
            // First-wins: if already assigned, skip to avoid
            // cycles + multi-tower conflicts.
            if !map.contains_key(&child) {
                map.insert(child.clone(), pool.clone());
                walk_nested_loci(&child, pool, top, map);
            }
        }
    }
}

/// Return the body block of a locus member that carries one
/// (lifecycle, on_failure, fn, mode). Anything else (params,
/// bus, closure decl, etc.) returns None.
fn locus_member_body(member: &LocusMember) -> Option<&Block> {
    match member {
        LocusMember::Lifecycle(lc) => Some(&lc.body),
        LocusMember::Failure(fd) => Some(&fd.body),
        LocusMember::Fn(fd) => Some(&fd.body),
        LocusMember::Mode(md) => Some(&md.body),
        _ => None,
    }
}

/// F.32-0 (2026-05-24): true when a form annotation carries an
/// explicit `sync = X` kwarg where X names a recognized sync
/// discipline (`serialized`, `striped`, or `lockfree`). The
/// cross-pool exemption applies only to such loci — the
/// substrate's runtime gives no thread-safety to plain
/// `@form(...)` cells (the 3ec6391 commit's "form ABI
/// serializes" claim was aspirational; see
/// `notes/f32-cache-aware-delivery-plan.md` § F.32-0).
///
/// Unknown / malformed `sync = X` values return false here
/// (so the cross-pool diagnostic still fires). F.32-1α/β2
/// validates the recognized values; `lockfree` (γ) is in the
/// accept set syntactically but the per-locus check rejects
/// it as deferred. This helper only gates the cross-pool
/// exemption — codegen does its own mapping to SyncMode.
fn form_has_explicit_sync_discipline(form: &FormAnnotation) -> bool {
    form.args.iter().any(|arg| {
        if arg.name.name != "sync" {
            return false;
        }
        match &arg.value {
            Expr::Ident(i) => matches!(
                i.name.as_str(),
                "serialized" | "striped" | "lockfree"
            ),
            _ => false,
        }
    })
}

/// Visitor context for the cross-pool call walk. Carried by
/// reference so the recursive Stmt/Expr traversal doesn't pay
/// a closure-capture allocation per node.
struct PoolCheckCx<'a> {
    enclosing_locus: &'a LocusDecl,
    caller_pool: Option<&'a PoolId>,
    pool_of_locus_type: &'a BTreeMap<String, PoolId>,
    /// F.32-0 (2026-05-24): locus type names that opt in to
    /// cross-pool access by declaring `@form(<name>, sync = X)`
    /// where X is a recognized discipline (`serialized` /
    /// `striped` / `lockfree`; F.32-1α/β/γ). Cross-pool method
    /// calls into receivers landing in this set skip the
    /// diagnostic — the chosen sync discipline carries the
    /// substrate's safety contract.
    ///
    /// Plain `@form(hashmap)` / `@form(vec)` / `@form(ring_buffer)`
    /// (no sync kwarg) does NOT land in this set: the runtime
    /// has no synchronization on those paths and concurrent
    /// writers corrupt the structure (`lotus_arena.c:1869+` —
    /// `lotus_hashmap_set` / `_grow` are non-atomic single-
    /// threaded code).
    cross_pool_safe_loci: &'a BTreeSet<String>,
    /// Wider companion to `cross_pool_safe_loci`: every locus
    /// type carrying any `@form(...)` annotation (with or
    /// without a sync kwarg). Used only to specialize the
    /// cross-pool diagnostic — receivers in this set get a
    /// "declare `sync = ...` to opt in" upgrade hint.
    form_bearing_loci: &'a BTreeSet<String>,
    /// F.32-1∞ (2026-05-25): sync-inference results keyed by
    /// locus type name. Present only for `@form(hashmap)`
    /// loci without explicit `sync = `. The cross-pool
    /// diagnostic reads this to name the specific discipline
    /// the rule picks (so the upgrade hint is actionable, not
    /// generic).
    inferred_sync: &'a BTreeMap<
        String,
        crate::sync_inference::InferredSync,
    >,
    top: &'a TopScope,
    diags: &'a mut Vec<Diag>,
}

fn walk_block_pool(block: &Block, cx: &mut PoolCheckCx) {
    for stmt in &block.stmts {
        walk_stmt_pool(stmt, cx);
    }
    if let Some(tail) = &block.tail {
        walk_expr_pool(tail, cx);
    }
}

fn walk_stmt_pool(stmt: &Stmt, cx: &mut PoolCheckCx) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            walk_expr_pool(value, cx);
        }
        Stmt::Assign { value, .. } => walk_expr_pool(value, cx),
        Stmt::If(i) => walk_if_pool(i, cx),
        Stmt::Match(m) => walk_match_pool(m, cx),
        Stmt::For { iter, body, .. } => {
            walk_expr_pool(iter, cx);
            walk_block_pool(body, cx);
        }
        Stmt::While { cond, body, .. } => {
            walk_expr_pool(cond, cx);
            walk_block_pool(body, cx);
        }
        Stmt::Return(opt, _) => {
            if let Some(e) = opt {
                walk_expr_pool(e, cx);
            }
        }
        Stmt::Fail { value, .. } => walk_expr_pool(value, cx),
        Stmt::Block(b) => walk_block_pool(b, cx),
        Stmt::ShmWrite { max, body, .. } => {
            walk_expr_pool(max, cx);
            walk_block_pool(body, cx);
        }
        Stmt::Recovery { args, .. } => {
            for a in args {
                walk_expr_pool(a, cx);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                walk_expr_pool(p, cx);
            }
        }
        Stmt::Send { subject, value, .. } => {
            walk_expr_pool(subject, cx);
            walk_expr_pool(value, cx);
        }
        Stmt::Expr(e) => walk_expr_pool(e, cx),
        Stmt::Yield(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Terminate(_) => {}
    }
}

fn walk_if_pool(stmt: &IfStmt, cx: &mut PoolCheckCx) {
    walk_expr_pool(&stmt.cond, cx);
    walk_block_pool(&stmt.then_block, cx);
    if let Some(eb) = &stmt.else_block {
        match eb.as_ref() {
            ElseBranch::Else(b) => walk_block_pool(b, cx),
            ElseBranch::ElseIf(nested) => walk_if_pool(nested, cx),
        }
    }
}

fn walk_match_pool(stmt: &MatchStmt, cx: &mut PoolCheckCx) {
    walk_expr_pool(&stmt.scrutinee, cx);
    for arm in &stmt.arms {
        if let Some(g) = &arm.guard {
            walk_expr_pool(g, cx);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => walk_expr_pool(e, cx),
            MatchArmBody::Block(b) => walk_block_pool(b, cx),
        }
    }
}

fn walk_expr_pool(expr: &Expr, cx: &mut PoolCheckCx) {
    if let Expr::Call { callee, args, span } = expr {
        // F.31 Phase 5: flag `self.X.foo(args)` where the
        // field X's locus type is on a different pool than
        // the enclosing locus. Only the `Field` callee
        // shape is checked; `Path2`-style stdlib/free-fn
        // calls are pool-neutral (per spec).
        if let Expr::Field { receiver, name: method, .. } = callee.as_ref() {
            if let Some(field_locus) = receiver_field_locus_type(
                receiver,
                cx.enclosing_locus,
                cx.top,
            ) {
                if let (Some(callee_pool), Some(caller_pool_val)) = (
                    cx.pool_of_locus_type.get(&field_locus),
                    cx.caller_pool,
                ) {
                    // F.32-0: receivers with an explicit
                    // sync discipline (`@form(..., sync = X)`,
                    // X != none) opt in to cross-pool calls;
                    // their chosen discipline carries the
                    // safety contract. Plain `@form(...)` is
                    // single-pool by default — the diagnostic
                    // fires with an upgrade hint.
                    if cx.cross_pool_safe_loci.contains(&field_locus) {
                        // skip the diagnostic
                    } else if callee_pool != caller_pool_val {
                        // F.32-1∞: prefer the inference-specific
                        // hint when it yields a non-None
                        // discipline (names the picked sync + the
                        // observed writer/reader pools). Fall
                        // back to the generic F.32-0 upgrade
                        // hint when the inference returns None
                        // (single-pool, or the offending call
                        // shape isn't one of the recognized
                        // `@form(hashmap)` methods so the walker
                        // observed no signal) or when the
                        // receiver isn't a hashmap (e.g. plain
                        // `@form(vec)`).
                        let inferred_hint = cx
                            .inferred_sync
                            .get(&field_locus)
                            .and_then(|inf| {
                                crate::sync_inference::render_inference_hint(
                                    &field_locus, inf,
                                )
                            });
                        let upgrade_hint = match inferred_hint {
                            Some(h) => h,
                            None => {
                                if cx.form_bearing_loci.contains(&field_locus) {
                                    format!(
                                        "\n  hint: receiver `{}` is `@form(...)`. \
                                         Cross-pool access requires an explicit sync \
                                         discipline:\n    \
                                         `@form(hashmap, sync = serialized)` — per-map \
                                         mutex (simplest, lowest throughput)\n    \
                                         `@form(hashmap, sync = striped)` — parallel \
                                         writers, cache-padded cells (F.32-1β)\n  \
                                         See `notes/f32-cache-aware-delivery-plan.md` \
                                         § F.32-0 / F.32-1.",
                                        field_locus,
                                    )
                                } else {
                                    String::new()
                                }
                            }
                        };
                        cx.diags.push(Diag::ty(
                            *span,
                            format!(
                                "cross-pool method call: `{}.{}` invokes a method \
                                 on locus `{}` placed `{}`, but the enclosing \
                                 locus `{}` is placed `{}`. Cross-pool \
                                 coordination must go through the bus, not a \
                                 direct call. See spec/types.md \
                                 § \"Single-threaded-method invariant (F.31)\".{}",
                                receiver_display(receiver),
                                method.name,
                                field_locus,
                                callee_pool.display(),
                                cx.enclosing_locus.name.name,
                                caller_pool_val.display(),
                                upgrade_hint,
                            ),
                        ));
                    }
                }
            }
        }
        walk_expr_pool(callee, cx);
        for a in args {
            walk_expr_pool(a, cx);
        }
        return;
    }
    match expr {
        Expr::Binary { left, right, .. } => {
            walk_expr_pool(left, cx);
            walk_expr_pool(right, cx);
        }
        Expr::Unary { operand, .. } => walk_expr_pool(operand, cx),
        Expr::Field { receiver, .. } => walk_expr_pool(receiver, cx),
        Expr::Index { receiver, index, .. } => {
            walk_expr_pool(receiver, cx);
            walk_expr_pool(index, cx);
        }
        Expr::Path2 { receiver, .. } => walk_expr_pool(receiver, cx),
        Expr::Tuple(items, _) | Expr::Array(items, _) => {
            for e in items {
                walk_expr_pool(e, cx);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                walk_expr_pool(&i.value, cx);
            }
        }
        Expr::Block(b) => walk_block_pool(b, cx),
        Expr::If(stmt) => walk_if_pool(stmt, cx),
        Expr::Match(stmt) => walk_match_pool(stmt, cx),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => walk_expr_pool(inner, cx),
        Expr::Approx { left, right, tolerance, .. } => {
            walk_expr_pool(left, cx);
            walk_expr_pool(right, cx);
            walk_expr_pool(tolerance, cx);
        }
        Expr::Range { lo, hi, .. } => {
            walk_expr_pool(lo, cx);
            walk_expr_pool(hi, cx);
        }
        Expr::ArrayRepeat { val, .. } => walk_expr_pool(val, cx),
        Expr::Or { inner, disposition, .. } => {
            walk_expr_pool(inner, cx);
            match disposition {
                OrDisposition::Substitute(e) => walk_expr_pool(e, cx),
                OrDisposition::Fail(e, _) => walk_expr_pool(e, cx),
                OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
            }
        }
        Expr::Literal(_, _)
        | Expr::Ident(_)
        | Expr::Path(_)
        | Expr::KwSelf(_) => {}
        // Already handled above
        Expr::Call { .. } => unreachable!(),
    }
}

/// If `receiver` is `self.X` where X is a field of
/// `enclosing_locus` whose declared type names a locus, return
/// that locus's name. Otherwise None.
fn receiver_field_locus_type(
    receiver: &Expr,
    enclosing_locus: &LocusDecl,
    top: &TopScope,
) -> Option<String> {
    let Expr::Field { receiver: inner, name, .. } = receiver else {
        return None;
    };
    if !matches!(inner.as_ref(), Expr::KwSelf(_)) {
        return None;
    }
    // Find the field on enclosing_locus's params block.
    let params = enclosing_locus
        .members
        .iter()
        .find_map(|m| match m {
            LocusMember::Params(pb) => Some(pb),
            _ => None,
        })?;
    let param = params.params.iter().find(|p| p.name.name == name.name)?;
    let ty = param.ty.as_ref()?;
    type_expr_locus_name(ty, top)
}

fn receiver_display(e: &Expr) -> String {
    match e {
        Expr::KwSelf(_) => "self".to_string(),
        Expr::Field { receiver, name, .. } => {
            format!("{}.{}", receiver_display(receiver), name.name)
        }
        Expr::Ident(i) => i.name.clone(),
        _ => "<expr>".to_string(),
    }
}

/// Bundle-wide validation for the v1.x topic-bindings feature.
/// Runs after per-locus checks because it cuts across loci. The
/// rules:
///   - At most one `main` locus per bundle. (Zero is fine — the
///     classic `fn main()` shape is still legal.)
///   - Each `bindings { Topic: <transport>; }` entry must name a
///     declared `topic`.
///   - A topic may appear at most once across all bindings.
///   - For `unix(...)` bindings without an explicit `role:` kwarg,
///     the role must be inferable from the bus block's
///     publish/subscribe declarations on this topic. Pub-only →
///     connect, sub-only → listen, both → compile error
///     ("specify `role:`").
/// Wave B: verify an adapter-binding locus satisfies the bus's
/// `__StdBusAdapter` contract (currently a single `send(subject:
/// String, bytes: Bytes)` method). Stand-alone shape — same logic
/// as `Checker::check_structural_impl` but callable from
/// `check_main_and_bindings` which doesn't construct a `Checker`.
fn check_satisfies_bus_adapter(
    top: &TopScope,
    locus_name: &str,
) -> Result<(), String> {
    const IFACE: &str = "__StdBusAdapter";
    let iface = match top.lookup(IFACE) {
        Some(TopSymbol::Interface(i)) => i,
        _ => {
            // The stdlib seed defines this interface; absence means
            // the seed wasn't loaded. Treat as OK rather than
            // failing user code with a stdlib-shape diagnostic.
            return Ok(());
        }
    };
    let locus = match top.lookup(locus_name) {
        Some(TopSymbol::Locus(l)) => l,
        _ => return Err(format!("`{}` is not a locus", locus_name)),
    };
    for im in &iface.methods {
        let lm = match locus.methods.iter().find(|lm| lm.name == im.name) {
            Some(m) => m,
            None => {
                return Err(format!(
                    "locus `{}` does not satisfy `{}`: missing method `{}`",
                    locus_name, IFACE, im.name
                ));
            }
        };
        if lm.params.len() != im.params.len() {
            return Err(format!(
                "locus `{}` method `{}` arity does not match `{}`: \
                 expected {} arg(s), locus has {}",
                locus_name,
                im.name,
                IFACE,
                im.params.len(),
                lm.params.len()
            ));
        }
        for (i, (lp, ip)) in lm.params.iter().zip(im.params.iter()).enumerate() {
            let want = &ip.1;
            if !want.assignable_from(lp) {
                return Err(format!(
                    "locus `{}` method `{}` arg #{} type mismatch: \
                     `{}` requires `{}`, locus has `{}`",
                    locus_name,
                    im.name,
                    i,
                    IFACE,
                    want.display(),
                    lp.display()
                ));
            }
        }
        if !im.ret.assignable_from(&lm.ret) {
            return Err(format!(
                "locus `{}` method `{}` return type mismatch: \
                 `{}` requires `{}`, locus returns `{}`",
                locus_name,
                im.name,
                IFACE,
                im.ret.display(),
                lm.ret.display()
            ));
        }
    }
    Ok(())
}

/// Form K4a (2026-05-20): validate the operational constraints
/// declared via the `where ...` clause on a binding entry.
///
/// Three classes of check:
///   1. **Intra-constraint consistency** — at most one scope
///      keyword per binding; `zero_copy` + `cross_machine` is a
///      contradiction.
///   2. **Transport-constraint compatibility** — does the
///      named transport satisfy each declared constraint? `unix`
///      is intra-machine, NOT zero-copy; `Adapter` is trusted
///      for scope (user-supplied transport), NOT zero-copy.
///   3. **Payload-shape compatibility** — `zero_copy` requires
///      the topic's payload to satisfy `is_flat_shapeable`.
///
/// Diagnostics are pushed to `diags`; the function returns
/// nothing (zero-or-more errors per binding).
/// F.36 Slice 2 (2026-05-28): codec(L) binding-clause typecheck.
/// When a binding entry carries a `codec(L { ... })` clause,
/// verify that:
///   1. L is a declared locus.
///   2. L has `fn encode(v: T) -> Bytes fallible(...)` where
///      T = the topic's payload type.
///   3. L has `fn decode(b: Bytes) -> T fallible(...)` where
///      T = the topic's payload type.
///   4. Both encode and decode are pure per F.36 Slice 1's
///      purity inference — codecs may be dispatched from the
///      bus reader thread / publisher's pool / consumer pools
///      concurrently, and have no coordination in scope to
///      serialize mutations to `self`.
fn check_binding_codec(
    entry: &BindingEntry,
    top: &TopScope,
    purity_map: &crate::purity::PurityMap,
    diags: &mut Vec<Diag>,
) {
    let codec = match &entry.codec {
        Some(c) => c,
        None => return,
    };
    // (1) Resolve the topic to its payload Ty.
    let topic_payload: Ty = match top.lookup(&entry.topic.name) {
        Some(TopSymbol::Topic(t)) => t.payload.clone(),
        _ => {
            // Already diagnosed by the parent "topic existence"
            // check; we can't verify the codec without a
            // payload type, so bail out silently.
            return;
        }
    };
    // (2) Resolve the codec locus.
    let locus_info = match top.lookup(&codec.locus.name) {
        Some(TopSymbol::Locus(l)) => l.clone(),
        Some(_) => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec binding for topic `{}`: `{}` is not a locus \
                     — `codec(L {{ ... }})` must name a locus that \
                     provides `encode` and `decode` methods",
                    entry.topic.name, codec.locus.name
                ),
            ));
            return;
        }
        None => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec binding for topic `{}`: unknown locus `{}`",
                    entry.topic.name, codec.locus.name
                ),
            ));
            return;
        }
    };
    // (3) Verify encode + decode methods exist with the right
    // signatures.
    let encode = locus_info.methods.iter().find(|m| m.name == "encode");
    let decode = locus_info.methods.iter().find(|m| m.name == "decode");
    let bytes_ty = Ty::Prim(hale_syntax::ast::PrimType::Bytes);

    match encode {
        None => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec `{}` for topic `{}` is missing required method \
                     `encode(v: {}) -> Bytes fallible(...)`",
                    codec.locus.name,
                    entry.topic.name,
                    topic_payload.display(),
                ),
            ));
        }
        Some(m) => {
            if m.params.len() != 1
                || !m.params[0].assignable_from(&topic_payload)
            {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `encode` must take one \
                         param of the topic's payload type `{}`; got params \
                         `{:?}`",
                        codec.locus.name,
                        entry.topic.name,
                        topic_payload.display(),
                        m.params.iter().map(|t| t.display()).collect::<Vec<_>>(),
                    ),
                ));
            }
            if !m.ret.assignable_from(&bytes_ty) {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `encode` must return \
                         `Bytes`; got `{}`",
                        codec.locus.name,
                        entry.topic.name,
                        m.ret.display(),
                    ),
                ));
            }
            if m.fallible.is_none() {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `encode` must be \
                         declared `fallible(E)` (encoding can fail; the \
                         binding-site dispatch needs a typed error \
                         channel)",
                        codec.locus.name, entry.topic.name,
                    ),
                ));
            }
        }
    }

    match decode {
        None => {
            diags.push(Diag::ty(
                codec.locus.span,
                format!(
                    "codec `{}` for topic `{}` is missing required method \
                     `decode(b: Bytes) -> {} fallible(...)`",
                    codec.locus.name,
                    entry.topic.name,
                    topic_payload.display(),
                ),
            ));
        }
        Some(m) => {
            if m.params.len() != 1
                || !m.params[0].assignable_from(&bytes_ty)
            {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `decode` must take one \
                         param of type `Bytes`; got params `{:?}`",
                        codec.locus.name,
                        entry.topic.name,
                        m.params.iter().map(|t| t.display()).collect::<Vec<_>>(),
                    ),
                ));
            }
            if !m.ret.assignable_from(&topic_payload) {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `decode` must return \
                         the topic's payload type `{}`; got `{}`",
                        codec.locus.name,
                        entry.topic.name,
                        topic_payload.display(),
                        m.ret.display(),
                    ),
                ));
            }
            if m.fallible.is_none() {
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}` for topic `{}`: `decode` must be \
                         declared `fallible(E)` (decoding can fail; the \
                         binding-site dispatch needs a typed error \
                         channel)",
                        codec.locus.name, entry.topic.name,
                    ),
                ));
            }
        }
    }
    // (4) Purity assertion. Codec methods may be invoked from
    // arbitrary threads (bus reader thread, publisher pool,
    // consumer pools) concurrently with no coordination in scope
    // to serialize mutations to self. They MUST be pure.
    for method_name in &["encode", "decode"] {
        let key = crate::purity::PurityKey::method(
            codec.locus.name.clone(),
            (*method_name).to_string(),
        );
        match purity_map.get(&key) {
            Some(crate::purity::Purity::Pure) => {}
            Some(crate::purity::Purity::Impure(reason)) => {
                let (line, hint) = render_impurity(reason);
                diags.push(Diag::ty(
                    codec.locus.span,
                    format!(
                        "codec `{}.{}` is not safe to dispatch from \
                         arbitrary threads\n\n\
                         note: codec methods must be stateless — they may \
                         be invoked from the bus reader thread, the \
                         publisher's pool, and consumer pools concurrently. \
                         No coordination is in scope to serialize mutations \
                         to `self`.\n\n\
                         note: {}\n\n\
                         help: {}",
                        codec.locus.name, method_name, line, hint,
                    ),
                ));
            }
            None => {
                // Method should have been in the map if the
                // locus was indexed; absence means the locus
                // doesn't actually have the named method (the
                // signature-mismatch branch above will have
                // already diagnosed). Quiet here to avoid
                // duplicate diagnostics.
            }
        }
    }
}

/// Render an [`Impurity`] as `(note_line, fix_hint)` strings for
/// embedding in a codec binding-site diagnostic.
fn render_impurity(
    reason: &crate::purity::Impurity,
) -> (String, &'static str) {
    use crate::purity::Impurity::*;
    match reason {
        SelfFieldWrite { field_chain, .. } => (
            format!("writes to `{}` (mutates the codec instance)", field_chain),
            "codecs are pure transformations on input data. \
             Move per-call counters out of the codec — push them \
             through the bus as observability events, or measure at \
             the adapter layer where state has lifecycle.",
        ),
        BusSend { subject_repr, .. } => (
            format!(
                "publishes to a bus topic ({}) — a side effect outside \
                 the codec's input/output channel",
                subject_repr
            ),
            "codecs translate between values and bytes; they don't \
             route messages. If you need to fire downstream events, \
             do it from the locus that owns the relationship, not \
             from the codec.",
        ),
        Violate { closure_name, .. } => (
            format!(
                "violates closure `{}` — escalates a structural failure \
                 through the parent",
                closure_name
            ),
            "codecs report failures via their `fallible(E)` return \
             channel, not via closure violations. Replace `violate` \
             with `fail SomeError {{ ... }}` in the codec body.",
        ),
        ImpureStdlibCall { fn_name, .. } => (
            format!(
                "calls `{}`, which has side effects (printing, \
                 file/process I/O, sleeping, or recovery)",
                fn_name
            ),
            "codecs must be deterministic, side-effect-free \
             transformations. Remove the offending call from the \
             codec body.",
        ),
        ImpureCalleeCall { callee_name, .. } => (
            format!(
                "calls `{}`, which is itself not pure (transitively)",
                callee_name
            ),
            "either make the called fn pure (no self-writes, no I/O, \
             no impure callees), or inline the small pure pieces \
             directly into the codec body.",
        ),
    }
}

fn check_binding_constraints(
    entry: &BindingEntry,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    if entry.constraints.is_empty() {
        return;
    }

    // (1) intra-constraint consistency.
    let scope_constraints: Vec<&SpannedBindingConstraint> = entry
        .constraints
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                BindingConstraint::IntraProcess
                    | BindingConstraint::IntraMachine
                    | BindingConstraint::CrossMachine
            )
        })
        .collect();
    if scope_constraints.len() > 1 {
        // Diagnostic cites the second one; the first is the
        // surviving "declared" scope. Pick whichever the user
        // sees first in source order; the parser preserves
        // declaration order.
        diags.push(Diag::ty(
            scope_constraints[1].span,
            format!(
                "binding for topic `{}` has multiple scope constraints \
                 (`{}` and `{}`); pick one",
                entry.topic.name,
                scope_constraints[0].kind.name(),
                scope_constraints[1].kind.name(),
            ),
        ));
    }

    let has_zero_copy = entry
        .constraints
        .iter()
        .any(|c| matches!(c.kind, BindingConstraint::ZeroCopy));
    let has_cross_machine = entry
        .constraints
        .iter()
        .any(|c| matches!(c.kind, BindingConstraint::CrossMachine));
    if has_zero_copy && has_cross_machine {
        // Find the zero_copy span for the diagnostic location.
        let span = entry
            .constraints
            .iter()
            .find(|c| matches!(c.kind, BindingConstraint::ZeroCopy))
            .map(|c| c.span)
            .unwrap_or(entry.span);
        diags.push(Diag::ty(
            span,
            format!(
                "binding for topic `{}`: `zero_copy` and `cross_machine` \
                 contradict — network transports require serialization",
                entry.topic.name
            ),
        ));
    }

    // (2) transport-constraint compatibility.
    for c in &entry.constraints {
        if let Some(msg) = transport_satisfies(&entry.transport, c.kind) {
            diags.push(Diag::ty(
                c.span,
                format!("binding for topic `{}`: {}", entry.topic.name, msg),
            ));
        }
    }

    // (3) payload-shape compatibility — `zero_copy` requires
    //     `is_flat_shapeable`. Look the topic's payload up
    //     through the resolved top scope; skip silently if the
    //     topic isn't registered (a separate diagnostic upstream
    //     will catch the missing topic).
    if has_zero_copy {
        if let Some(TopSymbol::Topic(topic)) =
            top.lookup(&entry.topic.name)
        {
            if !is_flat_shapeable(&topic.payload, top) {
                let span = entry
                    .constraints
                    .iter()
                    .find(|c| matches!(c.kind, BindingConstraint::ZeroCopy))
                    .map(|c| c.span)
                    .unwrap_or(entry.span);
                diags.push(Diag::ty(
                    span,
                    format!(
                        "binding for topic `{}` requires `zero_copy` but \
                         payload type `{}` is not flat-shapeable — it \
                         contains a String, Bytes, or fixed-size array \
                         field, whose storage is out-of-line (a pointer), \
                         so a raw memcpy would share a pointer that dangles \
                         across the zero-copy boundary. Use only fixed-size \
                         scalar fields in a `zero_copy` payload",
                        entry.topic.name,
                        topic.payload.display()
                    ),
                ));
            }
        }
    }
}

/// Returns `Some(reason)` if `transport` cannot satisfy
/// `constraint`. Returns `None` when the transport satisfies it
/// (or when the satisfaction can't be determined and trust
/// defaults to "OK" — adapter loci for scope constraints).
fn transport_satisfies(
    transport: &TransportSpec,
    constraint: BindingConstraint,
) -> Option<String> {
    use BindingConstraint::*;
    match (transport, constraint) {
        // unix: intra-machine substrate, kernel-memcpy at the
        // socket boundary.
        (TransportSpec::Unix { .. }, IntraProcess) => Some(
            "`unix` transport crosses OS process boundaries; cannot \
             satisfy `intra_process`"
                .into(),
        ),
        (TransportSpec::Unix { .. }, IntraMachine) => None,
        (TransportSpec::Unix { .. }, CrossMachine) => Some(
            "`unix` transport is host-local (AF_UNIX); cannot satisfy \
             `cross_machine`"
                .into(),
        ),
        (TransportSpec::Unix { .. }, ZeroCopy) => Some(
            "`unix` transport memcpys at the kernel boundary; cannot \
             satisfy `zero_copy`"
                .into(),
        ),

        // Adapter: user-supplied. Trust for scope constraints
        // (the adapter body knows where it routes). Reject
        // zero_copy — the Adapter contract (`fn send(subject: \
        // String, bytes: Bytes)`) requires serialization.
        (TransportSpec::Adapter { .. }, ZeroCopy) => Some(
            "`Adapter` transports cannot satisfy `zero_copy` — the \
             Adapter contract (`fn send(subject, bytes)`) requires \
             serialization to Bytes"
                .into(),
        ),
        (TransportSpec::Adapter { .. }, _) => None,

        // shm_ring: POSIX SHM ring substrate. Cross-process by
        // design (different procs mmap the same fd); host-local
        // (POSIX SHM doesn't traverse the network); satisfies
        // zero_copy intrinsically.
        (TransportSpec::ShmRing { .. }, IntraProcess) => Some(
            "`shm_ring` is cross-process by design (POSIX SHM); \
             cannot satisfy `intra_process`"
                .into(),
        ),
        (TransportSpec::ShmRing { .. }, IntraMachine) => None,
        (TransportSpec::ShmRing { .. }, CrossMachine) => Some(
            "`shm_ring` is host-local (POSIX SHM); cannot satisfy \
             `cross_machine`"
                .into(),
        ),
        (TransportSpec::ShmRing { .. }, ZeroCopy) => None,
    }
}

/// Phase 3 routing-keys (2026-05-25): cross-program checks for
/// the `fallback` policy.
///
/// * Every topic declared `on_unmatched: fallback` must have at
///   least one `where key == _` subscriber in the program.
///   Otherwise unmatched-key publishes would have nowhere to go
///   and the fallback policy is silently degraded to swallow.
/// * `where key == _` is only legal on topics that explicitly
///   declared `on_unmatched: fallback`. Using it on swallow or
///   unkeyed topics catches programmer typos.
///
/// Subscribers reference topics by either name (`subscribe K as
/// h`) or literal subject (`subscribe "k" as h of type T`). We
/// validate both forms; for literal subjects we look up the topic
/// by its wire subject string.
fn check_phase3_fallback_subscribers(
    bundle: &Bundle<'_>,
    diags: &mut Vec<Diag>,
) {
    // Collect all topics with their on_unmatched policy + wire
    // subject. Indexed by both topic name and wire subject so
    // subscribe-by-string can resolve.
    let mut by_name: BTreeMap<String, (Option<UnmatchedPolicy>, Span)> =
        BTreeMap::new();
    let mut by_wire: BTreeMap<String, (Option<UnmatchedPolicy>, Span)> =
        BTreeMap::new();
    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Topic(t) = item {
                by_name.insert(
                    t.name.name.clone(),
                    (t.on_unmatched, t.span),
                );
                let wire = t
                    .subject
                    .clone()
                    .unwrap_or_else(|| t.name.name.clone());
                by_wire.insert(wire, (t.on_unmatched, t.span));
            }
        }
    }

    // Walk every subscriber. For each `where key == _` filter,
    // resolve the topic and validate it's a fallback topic.
    // Track which fallback topics have at least one `_` sub.
    let mut fallback_has_catchall: BTreeMap<String, bool> = BTreeMap::new();
    for (name, (policy, _)) in &by_name {
        if matches!(policy, Some(UnmatchedPolicy::Fallback)) {
            fallback_has_catchall.insert(name.clone(), false);
        }
    }
    for program in bundle.programs.values() {
        for item in &program.items {
            let TopDecl::Locus(l) = item else { continue };
            for m in &l.members {
                let LocusMember::Bus(bb) = m else { continue };
                for bm in &bb.members {
                    let BusMember::Subscribe { subject, key_filter, .. } = bm
                    else {
                        continue;
                    };
                    let Some(kf) = key_filter else { continue };
                    let is_catchall = matches!(kf, KeyFilter::Unmatched { .. });
                    if !is_catchall {
                        continue;
                    }
                    let (topic_key, policy) = match subject {
                        BusSubject::Topic(i) => (
                            i.name.clone(),
                            by_name.get(&i.name).map(|x| x.0).flatten(),
                        ),
                        BusSubject::Literal { subject: s, .. } => (
                            s.clone(),
                            by_wire.get(s).map(|x| x.0).flatten(),
                        ),
                        BusSubject::QualifiedTopic(qn) => {
                            let last = qn
                                .segments
                                .last()
                                .map(|s| s.name.clone())
                                .unwrap_or_default();
                            (
                                last.clone(),
                                by_name.get(&last).map(|x| x.0).flatten(),
                            )
                        }
                    };
                    if !matches!(policy, Some(UnmatchedPolicy::Fallback)) {
                        diags.push(Diag::ty(
                            kf.span(),
                            format!(
                                "`where key == _` is only legal on \
                                 topics declared `on_unmatched: \
                                 fallback`; topic `{}` declares {}",
                                topic_key,
                                match policy {
                                    Some(UnmatchedPolicy::Swallow) =>
                                        "`on_unmatched: swallow`",
                                    Some(UnmatchedPolicy::Fail) =>
                                        "`on_unmatched: fail`",
                                    Some(UnmatchedPolicy::Fallback) =>
                                        unreachable!(),
                                    None => "no `on_unmatched` (default: \
                                             swallow)",
                                },
                            ),
                        ));
                    } else {
                        fallback_has_catchall.insert(topic_key, true);
                    }
                }
            }
        }
    }
    for (name, has) in &fallback_has_catchall {
        if *has {
            continue;
        }
        let span = by_name.get(name).map(|(_, s)| *s).unwrap_or(Span::new(0, 0));
        diags.push(Diag::ty(
            span,
            format!(
                "topic `{}` declares `on_unmatched: fallback` but \
                 no subscriber declares `where key == _`; \
                 unmatched-key publishes would have nowhere to go",
                name
            ),
        ));
    }
}

fn check_main_and_bindings(
    bundle: &Bundle<'_>,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    let mut mains: Vec<(String, Span)> = Vec::new();
    let mut bound: BTreeMap<String, Span> = BTreeMap::new();

    // For role inference: gather, per wire-subject, whether ANY
    // locus in the bundle publishes / subscribes to it. Bindings
    // reference topic-name, so map name → (publishes, subscribes).
    let (topic_publishes, topic_subscribes) = collect_topic_pub_sub(bundle);

    // F.36 Slice 2 (2026-05-28): compute the bundle-wide purity
    // map so binding-site codec checks can assert the codec's
    // encode/decode methods are pure. Done once here; threaded
    // into `check_binding_codec`. v0.1 always computes; future
    // polish could gate on "any binding has codec" to skip the
    // walk for the common case.
    let programs_vec: Vec<&Program> = bundle.programs.values().copied().collect();
    let purity_map = crate::purity::infer_purity_for_bundle(&programs_vec, top);

    for program in bundle.programs.values() {
        for item in &program.items {
            if let TopDecl::Locus(l) = item {
                if l.is_main {
                    mains.push((l.name.name.clone(), l.span));
                }
                for member in &l.members {
                    if let LocusMember::Bindings(bb) = member {
                        for entry in &bb.entries {
                            // Topic existence
                            match top.lookup(&entry.topic.name) {
                                Some(TopSymbol::Topic(_)) => {}
                                _ => {
                                    diags.push(Diag::ty(
                                        entry.topic.span,
                                        format!(
                                            "binding references unknown topic `{}`",
                                            entry.topic.name
                                        ),
                                    ));
                                }
                            }
                            // Duplicate topic across all bindings
                            if let Some(prev) = bound.get(&entry.topic.name) {
                                diags.push(Diag::ty(
                                    entry.topic.span,
                                    format!(
                                        "topic `{}` already bound (previous \
                                         binding at {:?})",
                                        entry.topic.name, prev
                                    ),
                                ));
                            } else {
                                bound.insert(entry.topic.name.clone(), entry.topic.span);
                            }

                            // Role inference validation. Substrate
                            // Unix bindings need a role (inferred or
                            // explicit); Adapter bindings carry
                            // direction inside the adapter locus's
                            // own params and are opaque here.
                            if let TransportSpec::Unix { role, .. } =
                                &entry.transport
                            {
                                if role.is_none() {
                                    let pubs = topic_publishes
                                        .contains(&entry.topic.name);
                                    let subs = topic_subscribes
                                        .contains(&entry.topic.name);
                                    if pubs && subs {
                                        diags.push(Diag::ty(
                                            entry.topic.span,
                                            format!(
                                                "binding for topic `{}` is ambiguous: \
                                                 some locus publishes it AND some locus \
                                                 subscribes to it; specify `role:` \
                                                 (e.g. `unix(\"/path\", role: listen)`)",
                                                entry.topic.name
                                            ),
                                        ));
                                    } else if !pubs && !subs {
                                        diags.push(Diag::ty(
                                            entry.topic.span,
                                            format!(
                                                "binding for topic `{}` has no publisher \
                                                 or subscriber in the bundle; nothing to \
                                                 route. Add a `bus {{ publish | subscribe }}` \
                                                 or remove the binding",
                                                entry.topic.name
                                            ),
                                        ));
                                    }
                                    // Otherwise (exactly one of pubs/subs):
                                    // role is inferable; desugar fills it in.
                                }
                            }

                            // Wave B: adapter binding checks. Verify
                            // the named symbol is a locus and that it
                            // structurally satisfies `__StdBusAdapter`
                            // (i.e. exposes `fn send(subject: String,
                            // bytes: Bytes)`). Field-init shape is
                            // codegen's job once the locus is
                            // resolved.
                            if let TransportSpec::Adapter { locus, .. } =
                                &entry.transport
                            {
                                match top.lookup(&locus.name) {
                                    Some(TopSymbol::Locus(_)) => {
                                        if let Err(msg) = check_satisfies_bus_adapter(
                                            top, &locus.name,
                                        ) {
                                            diags.push(Diag::ty(
                                                locus.span,
                                                format!(
                                                    "adapter binding for topic `{}`: {}",
                                                    entry.topic.name, msg
                                                ),
                                            ));
                                        }
                                    }
                                    Some(_) => {
                                        diags.push(Diag::ty(
                                            locus.span,
                                            format!(
                                                "adapter binding for topic `{}`: \
                                                 `{}` is not a locus — adapter \
                                                 transport spec must name a locus \
                                                 that satisfies `__StdBusAdapter`",
                                                entry.topic.name, locus.name
                                            ),
                                        ));
                                    }
                                    None => {
                                        diags.push(Diag::ty(
                                            locus.span,
                                            format!(
                                                "adapter binding for topic `{}`: \
                                                 unknown locus `{}`",
                                                entry.topic.name, locus.name
                                            ),
                                        ));
                                    }
                                }
                            }

                            // shm-ring-interop Proposal B: a
                            // `shm_ring(..., layout: Name)` binding's
                            // layout reference must resolve to a
                            // declared `ring_layout`. Absent layout =
                            // the native shape (back-compat).
                            if let TransportSpec::ShmRing {
                                layout: Some(lid), buffer_size, ..
                            } = &entry.transport
                            {
                                match top.lookup(&lid.name) {
                                    Some(TopSymbol::RingLayout(rl)) => {
                                        // The producer's compile-time
                                        // `buffer_size:` must be a multiple
                                        // of the layout's record `align`,
                                        // else a record header near the wrap
                                        // lands in (cap-len_prefix, cap) →
                                        // OOB. (The consumer enforces the
                                        // same at attach for the foreign
                                        // header's capacity.)
                                        if let Some(bs) = buffer_size {
                                            use hale_syntax::ast::RingAttrValue;
                                            let align = rl
                                                .decl
                                                .framing
                                                .as_ref()
                                                .and_then(|f| f.attrs.iter().find_map(|a| {
                                                    match (a.key.name.as_str(), &a.value) {
                                                        ("align", RingAttrValue::Int(n)) => Some(*n),
                                                        _ => None,
                                                    }
                                                }))
                                                .unwrap_or(1);
                                            if align > 1 && (*bs as i64) % align != 0 {
                                                diags.push(Diag::ty(
                                                    entry.span,
                                                    format!(
                                                        "shm_ring binding for topic `{}`: \
                                                         `buffer_size: {}` must be a \
                                                         multiple of the `{}` layout's \
                                                         record `align` ({}) — otherwise a \
                                                         record header can straddle the \
                                                         wrap boundary",
                                                        entry.topic.name, bs, lid.name, align
                                                    ),
                                                ));
                                            }
                                        }
                                        // Conformance (2026-06-06): a
                                        // layout-bound topic is read by
                                        // direct pointer-cast and written
                                        // by memcpy of the payload struct
                                        // (the bindgen-style contract — the
                                        // foreign format is fixed, so the
                                        // Hale struct must *be* the record
                                        // bytes). That's only sound if the
                                        // payload is flat-shapeable, and it
                                        // holds whether or not the binding
                                        // also asserts `where zero_copy`.
                                        if let Some(TopSymbol::Topic(topic)) =
                                            top.lookup(&entry.topic.name)
                                        {
                                            // A `BytesView` payload selects the raw-frame
                                            // mode: the consumer hands the handler a
                                            // bounded view over each record (decoded with
                                            // `std::bytes::read_*` + a discriminator), for
                                            // heterogeneous / variable-length rings. Any
                                            // other payload takes the typed-flat path,
                                            // which is read by direct cast and so must be
                                            // flat-shapeable.
                                            let is_raw_view = matches!(
                                                &topic.payload,
                                                Ty::Prim(hale_syntax::ast::PrimType::BytesView)
                                            );
                                            if !is_raw_view
                                                && !is_flat_shapeable(&topic.payload, top)
                                            {
                                                diags.push(Diag::ty(
                                                    entry.span,
                                                    format!(
                                                        "shm_ring binding for topic \
                                                         `{}` with `layout: {}` requires \
                                                         a flat-shapeable payload (read by \
                                                         direct cast) or a `BytesView` \
                                                         payload (raw-frame mode), but \
                                                         `{}` is neither — it contains \
                                                         String, Bytes, or other \
                                                         variable-size fields",
                                                        entry.topic.name,
                                                        lid.name,
                                                        topic.payload.display()
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                    Some(_) => diags.push(Diag::ty(
                                        lid.span,
                                        format!(
                                            "shm_ring binding for topic `{}`: \
                                             `layout: {}` is not a `ring_layout` \
                                             declaration",
                                            entry.topic.name, lid.name
                                        ),
                                    )),
                                    None => diags.push(Diag::ty(
                                        lid.span,
                                        format!(
                                            "shm_ring binding for topic `{}`: \
                                             unknown ring_layout `{}`",
                                            entry.topic.name, lid.name
                                        ),
                                    )),
                                }
                            }

                            // Form K4a (2026-05-20): operational-
                            // constraint validity. The `where ...`
                            // clause asserts properties of the
                            // route; the typechecker validates
                            // intra-constraint consistency,
                            // transport compatibility, and
                            // payload-shape compatibility.
                            check_binding_constraints(
                                entry, top, diags,
                            );

                            // F.36 Slice 2 (2026-05-28): pluggable
                            // codec validity. When the binding
                            // entry carries a `codec(L { ... })`
                            // clause, verify L has the encode /
                            // decode methods with the right
                            // signatures (against the topic's
                            // payload type) AND that both methods
                            // are pure per Slice 1's inference.
                            check_binding_codec(entry, top, &purity_map, diags);

                            // Form K6b (2026-05-20): shm_ring
                            // Hale-side subscribers are wired
                            // (reader thread + handler dispatch
                            // in lotus_bus_register_subscriber_shm_ring).
                            // No typecheck rejection needed; the
                            // codegen handles both publish-only
                            // and subscribe-bearing programs.
                            let _ = &topic_subscribes;
                        }
                    }
                }
            }
        }
    }
    if mains.len() > 1 {
        for (name, span) in &mains {
            diags.push(Diag::ty(
                *span,
                format!(
                    "more than one `main` locus declared (`{}` is one of {})",
                    name,
                    mains.len()
                ),
            ));
        }
    }
}

/// Walk the bundle and collect, per topic name (the binding-side
/// identifier), the set of topics that have at least one publisher
/// and the set that have at least one subscriber across all loci.
/// Used by role-inference validation in `check_main_and_bindings`.
fn collect_topic_pub_sub(
    bundle: &Bundle<'_>,
) -> (
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
) {
    let mut pubs = std::collections::BTreeSet::new();
    let mut subs = std::collections::BTreeSet::new();
    fn walk(
        items: &[TopDecl],
        pubs: &mut std::collections::BTreeSet<String>,
        subs: &mut std::collections::BTreeSet<String>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let LocusMember::Bus(bb) = member {
                            for bm in &bb.members {
                                match bm {
                                    BusMember::Publish { subject, .. } => {
                                        if let BusSubject::Topic(id) = subject {
                                            pubs.insert(id.name.clone());
                                        }
                                    }
                                    BusMember::Subscribe { subject, .. } => {
                                        if let BusSubject::Topic(id) = subject {
                                            subs.insert(id.name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, pubs, subs),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &mut pubs, &mut subs);
    }
    (pubs, subs)
}

// === Bus-graph property checks (GH #18 #4) =========================
//
// The bus topology is a typed directed graph already in the AST.
// PR A walks it for ORPHANs — a subject wired to only one end. Gated
// on a closed-world program (a `main` locus present): a library seed
// whose publishers/subscribers live in downstream consumers must not
// be flagged, since the other half is out of this bundle.
//
// Subjects are keyed by `BusSubject::canonical()` (literal string /
// topic name / qualified last segment), which is exactly the key a
// declared topic's name matches. False-positive guards: transport
// bindings (external peer), trailing-`**` wildcard coverage, and
// cross-seed (`alias::Foo`) references (the other seed owns the other
// half). A declared topic is matched by both its name and its
// `wire_subject` (a literal site may address it by the wire form).

fn check_bus_graph(
    bundle: &Bundle<'_>,
    top: &TopScope,
    diags: &mut Vec<Diag>,
) {
    // Closed-world gate: only a complete program (has `main`) has
    // both ends of every channel in-bundle.
    let has_main = bundle.programs.values().any(|p| {
        p.items
            .iter()
            .any(|i| matches!(i, TopDecl::Locus(l) if l.is_main))
    });
    if !has_main {
        return;
    }

    // The publisher/subscriber/bound/cross-seed walk is shared with
    // `bus_graph::build_bus_graph` (the static-devirt analysis) — one
    // walk, two consumers. The orphan diagnostics below use only the
    // ends + bound + cross_seed; the per-site detail is ignored here.
    let crate::bus_graph::BusWalk {
        publishers,
        subscribers,
        bound,
        cross_seed,
        ..
    } = crate::bus_graph::collect_bus_walk(bundle);

    // A subject has a publisher if some locus publishes it (exactly
    // or via wildcard), it is bound to a transport (external peer),
    // or it's referenced cross-seed. Same for subscriber.
    let has_pub = |aliases: &[&str]| {
        aliases.iter().any(|a| {
            publishers.covers(a) || bound.contains(*a) || cross_seed.contains(*a)
        })
    };
    let has_sub = |aliases: &[&str]| {
        aliases.iter().any(|a| {
            subscribers.covers(a) || bound.contains(*a) || cross_seed.contains(*a)
        })
    };

    // 1) Declared topics — matched by name and wire_subject.
    let mut declared_keys: BTreeSet<String> = BTreeSet::new();
    for (name, sym) in &top.symbols {
        let TopSymbol::Topic(info) = sym else { continue };
        // Topics that failed parent resolution carry an empty wire
        // subject and already have a diagnostic — skip.
        if info.wire_subject.is_empty() {
            continue;
        }
        declared_keys.insert(name.clone());
        declared_keys.insert(info.wire_subject.clone());
        let aliases: Vec<&str> = if info.wire_subject == *name {
            vec![name.as_str()]
        } else {
            vec![name.as_str(), info.wire_subject.as_str()]
        };
        let p = has_pub(&aliases);
        let s = has_sub(&aliases);
        if p && !s {
            let span = publishers
                .concrete
                .get(name)
                .or_else(|| publishers.concrete.get(&info.wire_subject))
                .copied()
                .unwrap_or(info.span);
            diags.push(Diag::warn(
                span,
                format!(
                    "bus topic `{}` is published but has no subscriber — \
                     the cells go nowhere. Add a `subscribe` for it, bind it \
                     to a transport, or drop the publish.",
                    name
                ),
            ));
        } else if s && !p {
            let span = subscribers
                .concrete
                .get(name)
                .or_else(|| subscribers.concrete.get(&info.wire_subject))
                .copied()
                .unwrap_or(info.span);
            diags.push(Diag::warn(
                span,
                format!(
                    "bus topic `{}` is subscribed but never published — its \
                     handler can't fire. Add a `publish` for it, bind it to a \
                     transport, or drop the subscription.",
                    name
                ),
            ));
        } else if !p && !s {
            diags.push(Diag::warn(
                info.span,
                format!(
                    "bus topic `{}` is declared but neither published nor \
                     subscribed — it's dead wiring.",
                    name
                ),
            ));
        }
    }

    // 2) Literal subjects (not a declared topic's name or wire form).
    let mut literal_keys: BTreeSet<String> = BTreeSet::new();
    for k in publishers.concrete.keys().chain(subscribers.concrete.keys()) {
        if !declared_keys.contains(k) {
            literal_keys.insert(k.clone());
        }
    }
    for k in literal_keys {
        let aliases = [k.as_str()];
        let p = has_pub(&aliases);
        let s = has_sub(&aliases);
        if p && !s {
            let span = publishers.concrete.get(&k).copied().unwrap();
            diags.push(Diag::warn(
                span,
                format!(
                    "bus subject `\"{}\"` is published but has no subscriber — \
                     the cells go nowhere. Add a `subscribe`, bind it to a \
                     transport, or drop the publish.",
                    k
                ),
            ));
        } else if s && !p {
            let span = subscribers.concrete.get(&k).copied().unwrap();
            diags.push(Diag::warn(
                span,
                format!(
                    "bus subject `\"{}\"` is subscribed but never published — \
                     its handler can't fire. Add a `publish`, bind it to a \
                     transport, or drop the subscription.",
                    k
                ),
            ));
        }
    }
}

// === Bus-graph cycles (GH #18 #4, PR B) ============================
//
// Edges of the bus graph: when locus `L` subscribes subject `S` with
// handler `H`, and `H`'s body sends to subject `D`, that's an edge
// `S →(L) D` — a cell on `S` can cause a cell on `D`. A cycle in this
// graph is a publish→subscribe→publish loop.
//
// The dispatch model splits the two outcomes:
//   - A **cross-locus** cycle (edges from ≥2 loci) hops between loci
//     via the cooperative *queue* (drained at yield) — it spins the
//     queue / livelocks → WARNING.
//   - An **intra-locus** cycle (all edges in one locus) is
//     intra-locus self-dispatch, which is **devirtualized to a direct
//     synchronous call** (spec/semantics.md), so it recurses on one
//     thread without bound → stack overflow → ERROR.
// The error stays on the provably-synchronous intra-locus case only,
// matching the error-precision discipline used elsewhere.

/// The subject a `Topic <- v` send addresses, as a canonical key
/// (matching `BusSubject::canonical`): a string literal, a bare topic
/// name, or a qualified path's last segment. None for computed
/// subjects (not statically traceable).
fn send_subject_key(e: &Expr) -> Option<String> {
    match e {
        Expr::Literal(Literal::String(s), _) => Some(s.clone()),
        Expr::Ident(id) => Some(id.name.clone()),
        Expr::Path(qn) => qn.segments.last().map(|s| s.name.clone()),
        _ => None,
    }
}

/// Collect the subjects a handler/`run()` body sends to (the targets
/// of `Topic <- value`). When `descend_cond` is false, sends nested
/// inside `if`/`match`/`for`/`while` are skipped — leaving only the
/// **unconditional** sends that fire on every execution. The
/// intra-locus error uses the unconditional set (a guarded
/// self-republish is a terminating state machine, not unbounded
/// recursion); the cross-locus warning uses all sends.
fn collect_sends_in_block(
    b: &Block,
    descend_cond: bool,
    out: &mut Vec<(String, Span)>,
) {
    for s in &b.stmts {
        collect_sends_in_stmt(s, descend_cond, out);
    }
}

fn collect_sends_in_stmt(
    stmt: &Stmt,
    descend_cond: bool,
    out: &mut Vec<(String, Span)>,
) {
    match stmt {
        Stmt::Send { subject, span, .. } => {
            if let Some(k) = send_subject_key(subject) {
                out.push((k, *span));
            }
        }
        Stmt::If(i) if descend_cond => collect_sends_in_if(i, out),
        Stmt::Match(m) if descend_cond => {
            for arm in &m.arms {
                if let MatchArmBody::Block(b) = &arm.body {
                    collect_sends_in_block(b, descend_cond, out);
                }
            }
        }
        Stmt::For { body, .. } | Stmt::While { body, .. }
            if descend_cond =>
        {
            collect_sends_in_block(body, descend_cond, out)
        }
        // A plain `{ ... }` block always executes — its sends stay
        // unconditional regardless of `descend_cond`.
        Stmt::Block(b) => collect_sends_in_block(b, descend_cond, out),
        _ => {}
    }
}

fn collect_sends_in_if(i: &IfStmt, out: &mut Vec<(String, Span)>) {
    collect_sends_in_block(&i.then_block, true, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => collect_sends_in_block(b, true, out),
        Some(ElseBranch::ElseIf(n)) => collect_sends_in_if(n, out),
        None => {}
    }
}

/// One directed edge `from → to`, tagged with the producing locus and
/// the send-site span for diagnostics.
#[derive(Clone)]
struct BusEdge {
    to: String,
    locus: String,
    span: Span,
}

type BusAdj = BTreeMap<String, Vec<BusEdge>>;

/// DFS for a cycle; returns the node sequence `[a, …, a]` of the first
/// cycle found, or None. Colors: 0 white, 1 gray (on stack), 2 black.
fn dfs_bus_cycle(
    node: &str,
    adj: &BusAdj,
    color: &mut BTreeMap<String, u8>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    color.insert(node.to_string(), 1);
    path.push(node.to_string());
    if let Some(edges) = adj.get(node) {
        for e in edges {
            match color.get(&e.to).copied().unwrap_or(0) {
                1 => {
                    let start =
                        path.iter().position(|n| n == &e.to).unwrap_or(0);
                    let mut cyc = path[start..].to_vec();
                    cyc.push(e.to.clone());
                    return Some(cyc);
                }
                0 => {
                    if let Some(c) = dfs_bus_cycle(&e.to, adj, color, path) {
                        return Some(c);
                    }
                }
                _ => {}
            }
        }
    }
    path.pop();
    color.insert(node.to_string(), 2);
    None
}

/// The set of loci whose edges realize `cyc`, plus a representative
/// send span (the first edge's).
fn cycle_loci(cyc: &[String], adj: &BusAdj) -> (BTreeSet<String>, Span) {
    let mut loci = BTreeSet::new();
    let mut span = Span::new(0, 0);
    let mut first = true;
    for w in cyc.windows(2) {
        if let Some(edges) = adj.get(&w[0]) {
            if let Some(e) = edges.iter().find(|e| e.to == w[1]) {
                loci.insert(e.locus.clone());
                if first {
                    span = e.span;
                    first = false;
                }
            }
        }
    }
    (loci, span)
}

fn check_bus_cycles(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    // Build the edge set. For each locus, map its subscribe handlers
    // by name, then for each subscribed subject add edges to whatever
    // the handler body sends.
    let mut global: BusAdj = BTreeMap::new();
    // Per-locus adjacency (only that locus's edges) for the intra
    // (synchronous) cycle check.
    let mut per_locus: BTreeMap<String, BusAdj> = BTreeMap::new();

    fn walk_loci<'a>(
        items: &'a [TopDecl],
        global: &mut BusAdj,
        per_locus: &mut BTreeMap<String, BusAdj>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    // handler name -> body
                    let mut handler_bodies: BTreeMap<&str, &Block> =
                        BTreeMap::new();
                    let mut subs: Vec<(String, String)> = Vec::new(); // (subject, handler)
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(f) => {
                                handler_bodies.insert(f.name.name.as_str(), &f.body);
                            }
                            LocusMember::Bus(bb) => {
                                for bm in &bb.members {
                                    if let BusMember::Subscribe {
                                        subject, handler, ..
                                    } = bm
                                    {
                                        subs.push((
                                            subject.canonical().to_string(),
                                            handler.name.clone(),
                                        ));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    let lname = l.name.name.clone();
                    for (subject, handler) in subs {
                        let Some(body) = handler_bodies.get(handler.as_str())
                        else {
                            continue;
                        };
                        // Cross-locus warning: every send (incl. guarded).
                        let mut sends_all = Vec::new();
                        collect_sends_in_block(body, true, &mut sends_all);
                        for (to, span) in sends_all {
                            global.entry(subject.clone()).or_default().push(
                                BusEdge { to, locus: lname.clone(), span },
                            );
                        }
                        // Intra-locus error: only unconditional sends —
                        // a guarded self-republish terminates.
                        let mut sends_uncond = Vec::new();
                        collect_sends_in_block(body, false, &mut sends_uncond);
                        for (to, span) in sends_uncond {
                            per_locus
                                .entry(lname.clone())
                                .or_default()
                                .entry(subject.clone())
                                .or_default()
                                .push(BusEdge { to, locus: lname.clone(), span });
                        }
                    }
                }
                TopDecl::Module(md) => walk_loci(&md.items, global, per_locus),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk_loci(&program.items, &mut global, &mut per_locus);
    }

    // 1) Intra-locus cycles → error (one per locus). Sound because
    //    intra-locus self-dispatch is devirtualized synchronous.
    let mut intra_loci: BTreeSet<String> = BTreeSet::new();
    for (lname, adj) in &per_locus {
        let roots: Vec<String> = adj.keys().cloned().collect();
        for root in roots {
            let mut color = BTreeMap::new();
            let mut path = Vec::new();
            if let Some(cyc) = dfs_bus_cycle(&root, adj, &mut color, &mut path) {
                let (_, span) = cycle_loci(&cyc, adj);
                diags.push(Diag::ty(
                    span,
                    format!(
                        "locus `{}` has a re-entrant synchronous bus cycle \
                         `{}`: each publish onto a topic the locus also \
                         subscribes is a direct in-thread call (intra-locus \
                         self-dispatch), so this recurses without bound and \
                         overflows the stack. Break the cycle, or route one \
                         hop through a different pool (an async enqueue).",
                        lname,
                        cyc.join(" → "),
                    ),
                ));
                intra_loci.insert(lname.clone());
                break;
            }
        }
    }

    // 2) Cross-locus cycles → warning. Exclude edges from loci that
    //    already have an intra-locus error so those don't shadow a
    //    genuine cross-locus loop.
    let mut cross_adj: BusAdj = BTreeMap::new();
    for (from, edges) in &global {
        for e in edges {
            if !intra_loci.contains(&e.locus) {
                cross_adj
                    .entry(from.clone())
                    .or_default()
                    .push(e.clone());
            }
        }
    }
    let mut reported: BTreeSet<String> = BTreeSet::new();
    let roots: Vec<String> = cross_adj.keys().cloned().collect();
    for root in roots {
        let mut color = BTreeMap::new();
        let mut path = Vec::new();
        if let Some(cyc) = dfs_bus_cycle(&root, &cross_adj, &mut color, &mut path)
        {
            let (loci, span) = cycle_loci(&cyc, &cross_adj);
            if loci.len() < 2 {
                continue;
            }
            let mut nodes: Vec<String> = cyc.clone();
            nodes.sort();
            nodes.dedup();
            let key = nodes.join("|");
            if !reported.insert(key) {
                continue;
            }
            diags.push(Diag::warn(
                span,
                format!(
                    "bus cycle `{}` across loci ({}): a cell can re-trigger \
                     its own publish, spinning the cooperative queue. Break \
                     the loop or add a terminating condition.",
                    cyc.join(" → "),
                    loci.into_iter().collect::<Vec<_>>().join(", "),
                ),
            ));
        }
    }
}

// === Bus backpressure (GH #18 #4) ==================================
//
// A producer with no flow control floods the bus without bound. A
// full "consumer can't sustain the rate" analysis is undecidable, so
// this is a deliberately narrow structural heuristic for the clearest
// case: an UNBOUNDED loop (`while true`) that publishes on some
// iteration but contains no flow-control or exit point — no
// cooperative `yield` (which lets a co-scheduled consumer drain), no
// `time::sleep`/`tick` throttle, no input-pacing blocking `recv`, and
// no `break`/`return` that could exit. Such a loop posts cells faster
// than anything can drain them — the queue and the payload arena grow
// without bound. A warning (the heuristic can't prove the OOM, only
// flag the missing backpressure). Bounded loops (`for`, `while
// cond`) are never flagged — only literal `while true`.

/// Flow-control / exit primitives whose presence anywhere in an
/// unbounded loop body rules out the flood: the loop either paces
/// itself or can leave.
fn block_has_flow_control(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_flow_control)
}

fn stmt_has_flow_control(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Yield(_) | Stmt::Break(_) | Stmt::Return(..) => true,
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            expr_has_flow_control_call(value)
        }
        Stmt::Assign { value, .. } => expr_has_flow_control_call(value),
        Stmt::Expr(e) => expr_has_flow_control_call(e),
        Stmt::Send { subject, value, .. } => {
            expr_has_flow_control_call(subject)
                || expr_has_flow_control_call(value)
        }
        Stmt::If(i) => if_has_flow_control(i),
        Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
            MatchArmBody::Block(b) => block_has_flow_control(b),
            MatchArmBody::Expr(e) => expr_has_flow_control_call(e),
        }),
        Stmt::For { body, .. } | Stmt::While { body, .. } => {
            block_has_flow_control(body)
        }
        Stmt::Block(b) => block_has_flow_control(b),
        _ => false,
    }
}

fn if_has_flow_control(i: &IfStmt) -> bool {
    block_has_flow_control(&i.then_block)
        || match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => block_has_flow_control(b),
            Some(ElseBranch::ElseIf(n)) => if_has_flow_control(n),
            None => false,
        }
}

/// A call to a throttle (`time::sleep`/`tick`) or an input-pacing
/// blocking op (`recv`/`accept`/`wait`) — both bound the publish rate.
fn expr_has_flow_control_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call { callee, args, .. } => {
            if let Expr::Path(qn) = callee.as_ref() {
                let segs: Vec<&str> =
                    qn.segments.iter().map(|s| s.name.as_str()).collect();
                if blocking_path_match(&segs).is_some()
                    || segs == ["std", "time", "sleep"]
                    || segs == ["std", "time", "tick"]
                {
                    return true;
                }
            }
            expr_has_flow_control_call(callee)
                || args.iter().any(expr_has_flow_control_call)
        }
        Expr::Binary { left, right, .. } => {
            expr_has_flow_control_call(left)
                || expr_has_flow_control_call(right)
        }
        Expr::Unary { operand, .. } => expr_has_flow_control_call(operand),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            expr_has_flow_control_call(receiver)
        }
        Expr::Index { receiver, index, .. } => {
            expr_has_flow_control_call(receiver)
                || expr_has_flow_control_call(index)
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            es.iter().any(expr_has_flow_control_call)
        }
        Expr::Struct { inits, .. } => {
            inits.iter().any(|i| expr_has_flow_control_call(&i.value))
        }
        Expr::Block(b) => block_has_flow_control(b),
        Expr::If(i) => {
            block_has_flow_control(&i.then_block)
                || matches!(i.else_block.as_deref(), Some(ElseBranch::Else(b)) if block_has_flow_control(b))
        }
        Expr::Match(m) => m.arms.iter().any(|arm| match &arm.body {
            MatchArmBody::Block(b) => block_has_flow_control(b),
            MatchArmBody::Expr(e) => expr_has_flow_control_call(e),
        }),
        Expr::Sum(e, _) | Expr::Prod(e, _) => expr_has_flow_control_call(e),
        Expr::Or { inner, .. } => expr_has_flow_control_call(inner),
        _ => false,
    }
}

/// Whether a block subtree contains a bus `Topic <- value` send.
fn block_has_send(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_send)
}

fn stmt_has_send(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Send { .. } => true,
        Stmt::If(i) => if_has_send(i),
        Stmt::Match(m) => m.arms.iter().any(|arm| {
            matches!(&arm.body, MatchArmBody::Block(b) if block_has_send(b))
        }),
        Stmt::For { body, .. } | Stmt::While { body, .. } => block_has_send(body),
        Stmt::Block(b) => block_has_send(b),
        _ => false,
    }
}

fn if_has_send(i: &IfStmt) -> bool {
    block_has_send(&i.then_block)
        || match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => block_has_send(b),
            Some(ElseBranch::ElseIf(n)) => if_has_send(n),
            None => false,
        }
}

fn is_literal_true(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Bool(true), _))
}

/// Walk a method/lifecycle body for unbounded publish-flood loops.
fn scan_flood_in_block(b: &Block, locus: &str, diags: &mut Vec<Diag>) {
    for s in &b.stmts {
        scan_flood_in_stmt(s, locus, diags);
    }
}

fn scan_flood_in_stmt(stmt: &Stmt, locus: &str, diags: &mut Vec<Diag>) {
    match stmt {
        Stmt::While { cond, body, span } if is_literal_true(cond) => {
            if block_has_send(body) && !block_has_flow_control(body) {
                diags.push(Diag::warn(
                    *span,
                    format!(
                        "locus `{}` publishes to the bus inside an unbounded \
                         `while true` loop with no flow control — no `yield`, \
                         `time::sleep`/`tick`, input-pacing `recv`, or \
                         `break`/`return`. The producer has no backpressure, \
                         so cells pile up in the queue (and the payload arena) \
                         without bound. Pace the loop (a `time::sleep`/`tick`), \
                         drive it from an input (a blocking `recv` so the \
                         publish rate follows the input rate), or `yield` to \
                         let a co-scheduled subscriber drain.",
                        locus
                    ),
                ));
                // Reported the outermost flood; don't descend further.
            } else {
                scan_flood_in_block(body, locus, diags);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            scan_flood_in_block(body, locus, diags)
        }
        Stmt::If(i) => scan_flood_in_if(i, locus, diags),
        Stmt::Match(m) => {
            for arm in &m.arms {
                if let MatchArmBody::Block(b) = &arm.body {
                    scan_flood_in_block(b, locus, diags);
                }
            }
        }
        Stmt::Block(b) => scan_flood_in_block(b, locus, diags),
        _ => {}
    }
}

fn scan_flood_in_if(i: &IfStmt, locus: &str, diags: &mut Vec<Diag>) {
    scan_flood_in_block(&i.then_block, locus, diags);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => scan_flood_in_block(b, locus, diags),
        Some(ElseBranch::ElseIf(n)) => scan_flood_in_if(n, locus, diags),
        None => {}
    }
}

// === Bus subject type-mismatch (GH #18 #4) ========================
//
// A *declared* `topic` fixes its payload type once, so every `publish
// Foo` / `subscribe Foo` site is unified by the declaration — no
// mismatch possible (and `of type` is forbidden on topic refs). The
// hole is *literal* subjects (`publish "wire.sig" of type Tick`):
// nothing ties the `of type` annotations at two sites on the same
// wire string together, so a publisher's `Tick` and a subscriber's
// `Pulse` both compile — and at runtime the subscriber decodes the
// publisher's bytes as the wrong type. That is a hard correctness
// bug, so this is an **error**.
//
// Grouping is by EXACT subject string, which deliberately sidesteps
// wildcards (`log.**` is a different string than `log.app`, so the
// two are never cross-compared — wildcard-subscriber type
// compatibility is a separate, fuzzier question).

/// A canonical, comparable rendering of a `TypeExpr` — equal strings
/// mean the same type at this layer. Also used in the diagnostic.
fn type_expr_key(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Primitive(p, _) => format!("{:?}", p),
        TypeExpr::Bounded { elem, cap, .. } => {
            format!("bounded[{}; {}]", type_expr_key(elem), cap)
        }
        TypeExpr::Named { path, generic_args, .. } => {
            let base = path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join("::");
            if generic_args.is_empty() {
                base
            } else {
                let args = generic_args
                    .iter()
                    .map(type_expr_key)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}<{}>", base, args)
            }
        }
        TypeExpr::Projection { class, inner, .. } => {
            format!("{:?}({})", class, type_expr_key(inner))
        }
        TypeExpr::Array { elem, .. } => format!("[{}]", type_expr_key(elem)),
        TypeExpr::Tuple(tys, _) => format!(
            "({})",
            tys.iter().map(type_expr_key).collect::<Vec<_>>().join(", ")
        ),
        TypeExpr::Function { params, ret, .. } => format!(
            "fn({}){}",
            params.iter().map(type_expr_key).collect::<Vec<_>>().join(", "),
            ret.as_ref()
                .map(|r| format!(" -> {}", type_expr_key(r)))
                .unwrap_or_default()
        ),
    }
}

fn check_bus_subject_types(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    // subject string -> the distinct payload types seen, each with a
    // representative site span. Insertion order preserved so the
    // first-declared type is the "expected" one in the message.
    let mut subjects: BTreeMap<String, Vec<(String, Span)>> = BTreeMap::new();

    fn record(
        subject: &BusSubject,
        ty: &Option<TypeExpr>,
        span: Span,
        subjects: &mut BTreeMap<String, Vec<(String, Span)>>,
    ) {
        // Only literal subjects carry an independent `of type`; topic
        // refs are unified by their declaration, qualified refs live
        // in another seed.
        let BusSubject::Literal { subject: subj, .. } = subject else {
            return;
        };
        let Some(ty) = ty else { return };
        let key = type_expr_key(ty);
        let entry = subjects.entry(subj.clone()).or_default();
        if !entry.iter().any(|(k, _)| k == &key) {
            entry.push((key, span));
        }
    }

    fn walk(
        items: &[TopDecl],
        subjects: &mut BTreeMap<String, Vec<(String, Span)>>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Bus(bb) = m {
                            for bm in &bb.members {
                                match bm {
                                    BusMember::Publish { subject, ty, span, .. } => {
                                        record(subject, ty, *span, subjects)
                                    }
                                    BusMember::Subscribe { subject, ty, span, .. } => {
                                        record(subject, ty, *span, subjects)
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(md) => walk(&md.items, subjects),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &mut subjects);
    }

    for (subj, types) in &subjects {
        if types.len() < 2 {
            continue;
        }
        let (expected, _) = &types[0];
        // Report each divergent type once, at its site.
        for (got, span) in types.iter().skip(1) {
            diags.push(Diag::ty(
                *span,
                format!(
                    "bus subject `\"{}\"` is used with conflicting payload \
                     types: `{}` here vs `{}` at another site. Every \
                     publish/subscribe on the same subject must carry the \
                     same payload type — a mismatch decodes the wire bytes as \
                     the wrong type at runtime. Declare a `topic` to fix the \
                     type in one place, or align the `of type` annotations.",
                    subj, got, expected,
                ),
            ));
        }
    }
}

fn check_bus_backpressure(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    fn walk(items: &[TopDecl], diags: &mut Vec<Diag>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let lname = l.name.name.as_str();
                    for m in &l.members {
                        match m {
                            LocusMember::Lifecycle(LifecycleDecl {
                                body, ..
                            }) => scan_flood_in_block(body, lname, diags),
                            LocusMember::Fn(f) => {
                                scan_flood_in_block(&f.body, lname, diags)
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(md) => walk(&md.items, diags),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, diags);
    }
}

fn collect_known_names(top: &TopScope) -> BTreeMap<String, Span> {
    let mut m = BTreeMap::new();
    for (name, sym) in &top.symbols {
        if matches!(
            sym,
            TopSymbol::Locus(_) | TopSymbol::Type(_) | TopSymbol::Perspective(_)
        ) {
            m.insert(name.clone(), sym.span());
        }
    }
    m
}

/// Stage-1 FFI (2026-05-22): predicate returning the rejection
/// reason if `ty` is not portable across the C-ABI boundary.
/// Returns `None` when the type is permitted in `@ffi` parameter
/// and return positions. See `spec/ffi.md` for the contract.
///
/// Stage 1 allows: scalar primitives (Int / Float / Bool /
/// Duration / Time), reference primitives with stable C
/// representation (String → `const char *`, Bytes → Hale
/// `[int64 len][payload]` ptr, BytesView / StringView → 16-byte
/// struct by value), and named user-type structs (layout-
/// compatible C struct by value — the library author is
/// responsible for keeping the Hale side and C side in sync;
/// future spec iteration may add a layout-assertion mechanism).
///
/// Stage 1 rejects: `Decimal` (i128 ABI is platform-variable),
/// `Uint` (Hale-internal type, no portable C mapping at v0),
/// projections / arrays / tuples / fallibles / functions / unit-
/// in-param-position. Unit (`Ty::Unit`) is allowed only as a
/// return type — the parser models `fn ...;` (no `-> T`) as
/// `ret: None`, which downstream represents as Unit; the caller
/// of this predicate already handles that path.
fn ffi_type_unportable(ty: &Ty) -> Option<&'static str> {
    match ty {
        Ty::Bounded(_, _) => Some(
            "bounded[T; N] has no portable C mapping — pass the \
             element pointer + count separately",
        ),
        Ty::Prim(p) => match p {
            PrimType::Int
            | PrimType::Float
            | PrimType::Bool
            | PrimType::String
            | PrimType::Bytes
            | PrimType::BytesView
            | PrimType::StringView
            | PrimType::BytesMut
            | PrimType::Time
            | PrimType::Duration => None,
            PrimType::Decimal => Some(
                "Decimal (i128) has platform-variable ABI; marshal as \
                 Int/Float at the Hale side instead",
            ),
            PrimType::Uint => Some(
                "Uint is Hale-internal; declare as Int in the @ffi \
                 signature",
            ),
        },
        // Unit allowed in return position; check_fn handles `ret:
        // None`. A `Ty::Unit` reaching this predicate from a param
        // came from an empty `()` type expr, which is invalid.
        Ty::Unit => Some(
            "() (unit) is not a meaningful FFI parameter type",
        ),
        // Named user-type structs are permitted at Stage 1. The
        // library author is responsible for keeping the Hale
        // struct's field order + types layout-compatible with the
        // C struct on the other side. Future spec iteration may
        // add a `@ffi_layout("c")` attribute for compile-time
        // layout assertions.
        Ty::Named(_) => None,
        Ty::Projection(_, _) => Some(
            "projection-typed values (Rich / Chunked / Recognition) \
             carry per-locus metadata and don't cross the C-ABI \
             boundary",
        ),
        Ty::Array(_, _) => Some(
            "fixed-size arrays don't cross the C-ABI boundary at \
             Stage 1; pass Bytes / a wrapper struct instead",
        ),
        Ty::Tuple(_) => Some(
            "tuples have no portable C struct layout; declare a named \
             type instead",
        ),
        Ty::Function { .. } => Some(
            "function-pointer types are not yet FFI-portable; declare \
             the wrapper at the C side and pass a struct/handle",
        ),
        Ty::Fallible { .. } => Some(
            "fallible(E) is an Hale internal channel; C functions \
             must return an error sentinel and the Hale wrapper \
             above translates",
        ),
        // Unknown comes from unresolved type names. Be permissive
        // — the named-type resolution may not have completed yet,
        // or the type may live behind an import this check can't
        // see. Codegen will catch genuinely-broken signatures at
        // LLVM-declaration emit time.
        Ty::Unknown => None,
    }
}

/// WASM plan — stdlib portability table. Returns a one-line reason +
/// browser alternative for a `std::` path unavailable under `target
/// wasm` (no syscalls in the browser sandbox), or `None` for the
/// portable surface (str/bytes/json/text/math/crypto/rand/decimal/bus/
/// diag/time/env/iter/...). Keyed on the leading namespace so every
/// call under it is covered.
fn wasm_unavailable_stdlib(segs: &[&str]) -> Option<&'static str> {
    match segs {
        ["std", "io", "tcp", ..] => Some(
            "raw TCP sockets don't exist in the browser; use a WebSocket \
             bus adapter (`ws://`) for networking",
        ),
        ["std", "io", "udp", ..] => {
            Some("raw UDP sockets don't exist in the browser")
        }
        ["std", "io", "tls", ..] => Some(
            "raw TLS isn't available; the browser performs TLS transparently \
             for `wss://` / `https://`",
        ),
        ["std", "io", "fs", ..] | ["std", "io", "file", ..] => Some(
            "filesystem access isn't available in the browser sandbox; use \
             `fetch` (via an `@ffi(\"js\")` host import) or a bus message",
        ),
        ["std", "io", "stdin", ..] | ["std", "io", "stdout", ..] => Some(
            "raw terminal I/O isn't available; use `println(...)` (the loader \
             routes it to the host console)",
        ),
        ["std", "term", ..] => {
            Some("terminal control (`std::term`) isn't available in the browser")
        }
        ["std", "process", ..] => Some(
            "OS process control (`std::process`) isn't available in the browser",
        ),
        ["std", "http", ..] => Some(
            "the `std::http` server is built on raw TCP and isn't available \
             in the browser",
        ),
        _ => None,
    }
}

struct Checker<'a> {
    top: &'a TopScope,
    known: &'a BTreeMap<String, Span>,
    diags: &'a mut Vec<Diag>,
    locals: ScopeStack,
    current_locus: Option<&'a LocusInfo>,
    in_lifecycle: bool,
    in_closure: bool,
    /// v1.x-VIOLATE (F.27): true while typechecking an
    /// `on_failure` body. Gates the rejection of `violate`
    /// inside `on_failure` (use `bubble(err)` instead).
    in_on_failure: bool,
    /// v1.x-FORM-1: when inside a `fallible(E)` fn body, holds
    /// `(success_ret, payload_E)`. Used to validate `return`
    /// against the success type, `fail <expr>;` against the
    /// payload type, and to gate the `err` implicit binding on
    /// `or`-substitute RHS scopes.
    fallible_ctx: Option<(Ty, Ty)>,
    /// WASM plan: true when the bundle declares `target wasm` /
    /// `target browser_js`. Gates the POSIX-only stdlib (no syscalls in
    /// the browser sandbox) at typecheck — see `wasm_unavailable_stdlib`.
    wasm_target: bool,
    /// M3 stage 2 (2026-07-02): true while checking an `or`
    /// expression whose value is discarded (statement position) —
    /// the Substitute arm skips the fallback-vs-success type match.
    /// Set by Stmt::Expr, consumed and cleared by the Or arm so
    /// nested `or`s in subexpressions don't inherit it.
    or_value_discarded: bool,
    /// M3 stage 3 (2026-07-02): generic fn templates declared in
    /// the program being checked (name → decl). Call sites mirror
    /// codegen's m62 inference at the Ty level — every generic
    /// param must be pinned by an arg, bindings must not conflict,
    /// args must match the substituted params — and the call types
    /// as the SUBSTITUTED return instead of Unknown.
    generic_fns: BTreeMap<String, &'a FnDecl>,
    /// M3 stage 3 tranche 2: generic TYPE templates (name → decl).
    /// Mangled monomorph literals (`Box_Int { ... }`) resolve
    /// against these — previously "unknown type" at typecheck,
    /// which made generic types unusable through the CLI (only
    /// codegen unit tests, which skip the checker, exercised
    /// them). Fields validate against the SUBSTITUTED types.
    generic_types: BTreeMap<String, &'a TypeDecl>,
}

#[derive(Default)]
struct ScopeStack {
    frames: Vec<BTreeMap<String, LocalSym>>,
}

#[derive(Debug, Clone)]
struct LocalSym {
    ty: Ty,
    /// m50: tracks whether the binding was declared with `mut`.
    /// `let x = ...` is immutable; `let mut x = ...` permits
    /// reassignment. Per spec/types.md "Mutability" + design-
    /// rationale §E. Locus state on `self` is mutable
    /// independently (locus fields aren't bindings — they're
    /// state — and lifecycle methods update them through
    /// `self.field = ...` regardless of any binding's is_mut).
    /// Fn params, loop variables, and pattern bindings default
    /// to false: the surface spec says params are immutable,
    /// loop vars rebind fresh each iteration, and pattern arm
    /// bindings exist only for the duration of the arm body.
    is_mut: bool,
}

impl ScopeStack {
    fn new() -> Self {
        Self {
            frames: vec![BTreeMap::new()],
        }
    }
    fn push(&mut self) {
        self.frames.push(BTreeMap::new());
    }
    fn pop(&mut self) {
        self.frames.pop();
    }
    fn insert(&mut self, name: &str, sym: LocalSym) {
        self.frames
            .last_mut()
            .expect("at least one scope")
            .insert(name.to_string(), sym);
    }
    fn lookup(&self, name: &str) -> Option<&LocalSym> {
        for frame in self.frames.iter().rev() {
            if let Some(s) = frame.get(name) {
                return Some(s);
            }
        }
        None
    }
}

impl<'a> Checker<'a> {
    fn check_top_decl(&mut self, decl: &'a TopDecl) {
        match decl {
            TopDecl::Locus(l) => self.check_locus(l),
            TopDecl::Fn(f) => self.check_fn(f, None),
            TopDecl::Const(c) => {
                let want = resolve_type_expr(&c.ty, self.known);
                let got = self.check_expr(&c.value);
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        c.value.span(),
                        format!(
                            "const `{}`: expected `{}`, got `{}`",
                            c.name.name,
                            want.display(),
                            got.display()
                        ),
                    ));
                }
            }
            TopDecl::Module(m) => {
                for item in &m.items {
                    self.check_top_decl(item);
                }
            }
            TopDecl::Type(_) | TopDecl::Perspective(_) => {
                // Structure already validated by resolver; field
                // types are checked when something instantiates
                // them via struct literal.
            }
            TopDecl::Interface(_) => {
                // Interface declarations are pure type-level —
                // method signatures only, no bodies. The resolver
                // collected them; the structural impl-check fires
                // at the use site (call expression where the
                // expected type is an interface).
            }
            TopDecl::Topic(t) => {
                // Topic declarations carry `payload: T; subject:
                // "...";` and now (Phase 3, 2026-05-25) optional
                // `keyed_by FIELD;` + `on_unmatched: V;`. The
                // resolver validated the payload type expression
                // already; per-use-site handler/send checks happen
                // at bus-block and send sites that reference the
                // topic. Below: Phase-3 specific static checks.

                // (5) / (6) on_unmatched policy validation:
                //   - swallow / None: nothing to check here.
                //   - fail: Send sites for this topic must carry
                //     an `or raise` / `or discard` disposition;
                //     validated at the Send site in check_send.
                //   - fallback: a program-wide `where key == _`
                //     subscriber must exist; validated in
                //     check_phase3_fallback_subscribers (bundle
                //     pass).
                let _ = t.on_unmatched;

                // (1) keyed_by field must exist on the payload
                // type and resolve to an int-shaped scalar
                // (Int, Decimal, Time, Duration, Bool, or
                // no-payload enum). For payloads that don't
                // resolve to a user-declared struct (Ty::Unknown
                // / external type / primitive), skip the check —
                // the resolver will already have flagged the
                // payload as unresolvable.
                if let Some(field_ident) = &t.keyed_by {
                    let payload_ty_name = match &t.payload {
                        TypeExpr::Named { path, .. }
                            if path.segments.len() == 1 =>
                        {
                            Some(path.segments[0].name.clone())
                        }
                        _ => None,
                    };
                    let mut found_field_ty: Option<Ty> = None;
                    if let Some(name) = &payload_ty_name {
                        if let Some(TopSymbol::Type(info)) =
                            self.top.lookup(name)
                        {
                            if let TypeKind::Struct(fields) = &info.kind {
                                if let Some(f) = fields
                                    .iter()
                                    .find(|f| f.name == field_ident.name)
                                {
                                    found_field_ty = Some(f.ty.clone());
                                }
                            }
                        }
                    }
                    if payload_ty_name.is_some() && found_field_ty.is_none() {
                        self.diags.push(Diag::ty(
                            field_ident.span,
                            format!(
                                "topic `{}`'s `keyed_by` references \
                                 field `{}`, which does not exist on \
                                 payload type `{}`",
                                t.name.name,
                                field_ident.name,
                                payload_ty_name.as_deref().unwrap_or("?"),
                            ),
                        ));
                    }
                    if let Some(fty) = found_field_ty {
                        if !is_key_eligible(&fty, &self.top) {
                            self.diags.push(Diag::ty(
                                field_ident.span,
                                format!(
                                    "topic `{}`'s `keyed_by` field \
                                     `{}` has type `{}`; routing-key \
                                     fields must be int-shaped (Int, \
                                     Decimal, Time, Duration, Bool, \
                                     or a no-payload enum)",
                                    t.name.name,
                                    field_ident.name,
                                    fty.display(),
                                ),
                            ));
                        }
                    }
                }

                // (also covers the case where) `on_unmatched` was
                // specified on a topic that doesn't declare
                // `keyed_by`: it has no meaning and is rejected
                // (catches typos / leftover from earlier drafts).
                if t.on_unmatched.is_some() && t.keyed_by.is_none() {
                    self.diags.push(Diag::ty(
                        t.span,
                        format!(
                            "topic `{}` sets `on_unmatched` but has \
                             no `keyed_by` — `on_unmatched` is only \
                             meaningful on keyed topics",
                            t.name.name
                        ),
                    ));
                }
            }
            TopDecl::Target(_) => {
                // FUv0.8.2 #7 (2026-05-25): target capability
                // block. v0.2 lands the parser + AST surface;
                // the capability-enforcement pass (rejecting
                // programs that reach beyond the declared
                // capability set) is v0.3. Today's check is
                // structural only — the resolver registered
                // the target name; no use-site checks here yet.
            }
            TopDecl::RingLayout(r) => self.check_ring_layout(r),
        }
    }

    /// shm-ring-interop Proposal B: validate a `ring_layout`'s
    /// contract — known width reprs, a recognized framing kind (with
    /// `len_prefix` for byte_records), at least one cursor with an
    /// `at` offset and a known repr/ordering/unit, and non-negative
    /// offsets. A wrong-but-well-formed layout still produces wrong
    /// *values* not OOB (the safety argument), but these catch the
    /// obvious declaration mistakes at build time.
    fn check_ring_layout(&mut self, r: &'a hale_syntax::ast::RingLayoutDecl) {
        use hale_syntax::ast::RingAttrValue;
        const WIDTHS: &[&str] = &[
            "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64",
        ];
        const ORDERINGS: &[&str] =
            &["relaxed", "acquire", "release", "acq_rel", "seq_cst"];

        if let Some(off) = r.data_at {
            if off < 0 {
                self.diags.push(Diag::ty(
                    r.span,
                    format!("ring_layout `{}`: data_at must be >= 0", r.name.name),
                ));
            }
        }
        for f in &r.scalars {
            if f.at < 0 {
                self.diags.push(Diag::ty(
                    f.span,
                    format!("ring_layout field `{}`: offset must be >= 0", f.name.name),
                ));
            }
            if !WIDTHS.contains(&f.repr.name.as_str()) {
                self.diags.push(Diag::ty(
                    f.repr.span,
                    format!(
                        "ring_layout field `{}`: unknown repr `{}` (expected one \
                         of u8/u16/u32/u64, i8/i16/i32/i64, f32/f64)",
                        f.name.name, f.repr.name
                    ),
                ));
            }
        }

        // At least one cursor, each with an `at` offset and known attrs.
        if r.cursors.is_empty() {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs at least one `cursor {{ ... }}` \
                     (the published position a consumer reads)",
                    r.name.name
                ),
            ));
        }
        for c in &r.cursors {
            let mut has_at = false;
            for a in &c.attrs {
                match a.key.name.as_str() {
                    "at" => {
                        has_at = true;
                        if let RingAttrValue::Int(n) = a.value {
                            if n < 0 {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    "cursor `at` offset must be >= 0".to_string(),
                                ));
                            }
                        } else {
                            self.diags.push(Diag::ty(
                                a.span,
                                "cursor `at` must be an integer offset".to_string(),
                            ));
                        }
                    }
                    "load" | "store" => {
                        if let RingAttrValue::Ident(id) = &a.value {
                            if !ORDERINGS.contains(&id.name.as_str()) {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    format!(
                                        "cursor `{}`: unknown memory ordering `{}` \
                                         (relaxed/acquire/release/acq_rel/seq_cst)",
                                        a.key.name, id.name
                                    ),
                                ));
                            }
                        }
                    }
                    "unit" => {
                        if let RingAttrValue::Ident(id) = &a.value {
                            if id.name != "bytes" && id.name != "slots" {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    format!(
                                        "cursor `unit`: expected `bytes` or `slots`, \
                                         got `{}`",
                                        id.name
                                    ),
                                ));
                            }
                        }
                    }
                    // `kind`, `repr` accepted as free idents (the
                    // descriptor build in PR3 maps the known ones).
                    _ => {}
                }
            }
            if !has_at {
                self.diags.push(Diag::ty(
                    c.span,
                    format!(
                        "ring_layout `{}`: cursor needs an `at OFFSET;`",
                        r.name.name
                    ),
                ));
            }
        }

        // Framing: required, recognized kind; byte_records needs a
        // len_prefix width.
        match &r.framing {
            None => self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs a `framing byte_records {{ ... }}` \
                     (or `framing slots {{ ... }}`)",
                    r.name.name
                ),
            )),
            Some(fr) => {
                if fr.kind.name != "byte_records" && fr.kind.name != "slots" {
                    self.diags.push(Diag::ty(
                        fr.kind.span,
                        format!(
                            "ring_layout `{}`: unknown framing `{}` (expected \
                             `byte_records` or `slots`)",
                            r.name.name, fr.kind.name
                        ),
                    ));
                }
                // Slots framing: a fixed-stride slot ring (the native
                // LotusRing shape). The consumer reads the geometry from
                // the foreign header, so it needs `slot_size` + `slot_count`
                // scalars (no `len_prefix` — records aren't length-framed).
                if fr.kind.name == "slots" {
                    for field in ["slot_size", "slot_count"] {
                        if !r.scalars.iter().any(|s| s.name.name == field) {
                            self.diags.push(Diag::ty(
                                fr.span,
                                format!(
                                    "framing slots: needs a `{field}` scalar — the \
                                     consumer reads the slot geometry from the \
                                     foreign header (e.g. `{field} at <off> : u64;`)"
                                ),
                            ));
                        }
                    }
                }
                if fr.kind.name == "byte_records" {
                    let len_prefix = fr.attrs.iter().find(|a| a.key.name == "len_prefix");
                    match len_prefix {
                        None => self.diags.push(Diag::ty(
                            fr.span,
                            "framing byte_records: needs `len_prefix u32;` (the \
                             record length-prefix width)".to_string(),
                        )),
                        Some(a) => {
                            if let RingAttrValue::Ident(id) = &a.value {
                                if !WIDTHS.contains(&id.name.as_str()) {
                                    self.diags.push(Diag::ty(
                                        a.span,
                                        format!(
                                            "framing byte_records: `len_prefix` repr \
                                             `{}` is not a known width",
                                            id.name
                                        ),
                                    ));
                                }
                            } else {
                                self.diags.push(Diag::ty(
                                    a.span,
                                    "framing byte_records: `len_prefix` must be a \
                                     width ident (e.g. u32)".to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // shm-ring-interop conformance (2026-06-06): cross-field
        // geometric consistency. the foreign format is fixed and
        // unchangeable, so a `ring_layout` that mis-transcribes it is
        // purely *our* bug — and several of these fields silently
        // corrupt PR3's already-shipped reader if they're wrong (a
        // cursor past `data_at`, an overlapping field, a non-power-of-
        // two `align` the reader masks with, a `pad_sentinel` too wide
        // for the `len_prefix` to hold). Catch them at compile time.
        fn repr_bytes(name: &str) -> Option<i64> {
            Some(match name {
                "u8" | "i8" => 1,
                "u16" | "i16" => 2,
                "u32" | "i32" | "f32" => 4,
                "u64" | "i64" | "f64" | "atomic_u64" => 8,
                _ => return None,
            })
        }

        // Occupied header intervals [start, end) with a label + span.
        let mut intervals: Vec<(i64, i64, String, Span)> = Vec::new();
        for f in &r.scalars {
            if f.at >= 0 {
                if let Some(w) = repr_bytes(&f.repr.name) {
                    intervals.push((
                        f.at,
                        f.at + w,
                        format!("field `{}`", f.name.name),
                        f.span,
                    ));
                }
            }
        }
        for c in &r.cursors {
            let at = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("at", RingAttrValue::Int(n)) if *n >= 0 => Some(*n),
                _ => None,
            });
            if let Some(at) = at {
                let label = c
                    .name
                    .as_ref()
                    .map(|n| format!("cursor `{}`", n.name))
                    .unwrap_or_else(|| "cursor".to_string());
                // A cursor is an atomic u64 — 8 bytes.
                intervals.push((at, at + 8, label, c.span));
            }
        }

        // (1) every header field + cursor must lie before `data_at`.
        if let Some(data_at) = r.data_at {
            for (start, end, label, span) in &intervals {
                if *end > data_at {
                    self.diags.push(Diag::ty(
                        *span,
                        format!(
                            "ring_layout `{}`: {} occupies bytes [{}, {}) which \
                             overruns `data_at {}` — header fields and the cursor \
                             must lie before the data region",
                            r.name.name, label, start, end, data_at
                        ),
                    ));
                }
            }
        }

        // (2) no two header fields / cursor may overlap.
        for i in 0..intervals.len() {
            for j in (i + 1)..intervals.len() {
                let (a0, a1) = (intervals[i].0, intervals[i].1);
                let (b0, b1) = (intervals[j].0, intervals[j].1);
                if a0 < b1 && b0 < a1 {
                    self.diags.push(Diag::ty(
                        intervals[i].3,
                        format!(
                            "ring_layout `{}`: {} (bytes [{}, {})) overlaps {} \
                             (bytes [{}, {}))",
                            r.name.name, intervals[i].2, a0, a1, intervals[j].2, b0, b1
                        ),
                    ));
                }
            }
        }

        // (3) byte_records framing: `align` must be a power of two
        //     (the reader does `& ~(align-1)`); `pad_sentinel` must
        //     fit in the `len_prefix` width or wrap-detection reads a
        //     truncated sentinel and never fires.
        if let Some(fr) = &r.framing {
            if fr.kind.name == "byte_records" {
                let align_attr =
                    fr.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                        ("align", RingAttrValue::Int(n)) => Some((*n, a.span)),
                        _ => None,
                    });
                // Absent `align` defaults to 1 (byte-packed) at runtime.
                let align = align_attr.map(|(n, _)| n).unwrap_or(1);
                let align_span = align_attr.map(|(_, s)| s).unwrap_or(fr.span);
                if align < 1 || (align & (align - 1)) != 0 {
                    self.diags.push(Diag::ty(
                        align_span,
                        format!(
                            "ring_layout `{}`: framing `align {}` must be a power \
                             of two — it's the record-stride alignment the reader \
                             masks with",
                            r.name.name, align
                        ),
                    ));
                }
                let len_width =
                    fr.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                        ("len_prefix", RingAttrValue::Ident(id)) => repr_bytes(&id.name),
                        _ => None,
                    });
                // The length prefix must fit within one alignment unit, or
                // a record near the wrap boundary can land its header in
                // (cap - len_prefix_width, cap) → an OOB read/write. With
                // `len_prefix_width <= align` and `cap % align == 0`, every
                // record header is fully inside the data region.
                if let Some(w) = len_width {
                    if align >= 1 && w > align {
                        self.diags.push(Diag::ty(
                            align_span,
                            format!(
                                "ring_layout `{}`: framing `len_prefix` width ({} \
                                 bytes) exceeds `align` ({}) — the record header \
                                 could straddle the wrap boundary; set `align` to at \
                                 least the len-prefix width",
                                r.name.name, w, align
                            ),
                        ));
                    }
                }
                let pad =
                    fr.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                        ("pad_sentinel", RingAttrValue::Int(n)) => Some((*n, a.span)),
                        _ => None,
                    });
                if let (Some(w), Some((pad, span))) = (len_width, pad) {
                    let max: u64 = if w >= 8 { u64::MAX } else { (1u64 << (w * 8)) - 1 };
                    if pad < 0 || (pad as u64) > max {
                        self.diags.push(Diag::ty(
                            span,
                            format!(
                                "ring_layout `{}`: `pad_sentinel {:#x}` does not fit \
                                 in the `len_prefix` width ({} byte{}) — wrap \
                                 detection would read a truncated value and never \
                                 trigger",
                                r.name.name,
                                pad,
                                w,
                                if w == 1 { "" } else { "s" }
                            ),
                        ));
                    }
                }
            }
        }

        // --- Frontend hardening (2026-06-08) ---
        let is_byte_records =
            r.framing.as_ref().map(|f| f.kind.name == "byte_records").unwrap_or(false);
        let is_slots =
            r.framing.as_ref().map(|f| f.kind.name == "slots").unwrap_or(false);

        // Unaligned atomic cursor: an `atomic_u64` cursor whose `at` is
        // not 8-aligned makes the runtime's atomic load undefined (torn /
        // misaligned). This is the one genuinely UB-adjacent frontend gap.
        for c in &r.cursors {
            let repr = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("repr", RingAttrValue::Ident(id)) => Some(id.name.as_str()),
                _ => None,
            });
            let at = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("at", RingAttrValue::Int(n)) => Some(*n),
                _ => None,
            });
            if repr == Some("atomic_u64") {
                if let Some(at) = at {
                    if at % 8 != 0 {
                        self.diags.push(Diag::ty(
                            c.span,
                            format!(
                                "ring_layout `{}`: an `atomic_u64` cursor's `at` \
                                 offset ({}) must be 8-byte aligned — an unaligned \
                                 atomic load is undefined",
                                r.name.name, at
                            ),
                        ));
                    }
                }
            }
            // Cursor `unit` must agree with the framing kind: byte cursors
            // for `byte_records`, slot cursors for `slots`.
            let unit = c.attrs.iter().find_map(|a| match (a.key.name.as_str(), &a.value) {
                ("unit", RingAttrValue::Ident(id)) => Some(id.name.as_str()),
                _ => None,
            });
            if let (Some(unit), Some(fr)) = (unit, &r.framing) {
                let ok = (unit == "bytes" && fr.kind.name == "byte_records")
                    || (unit == "slots" && fr.kind.name == "slots");
                if !ok && (unit == "bytes" || unit == "slots") {
                    self.diags.push(Diag::ty(
                        c.span,
                        format!(
                            "ring_layout `{}`: cursor `unit {}` doesn't match \
                             `framing {}` (expected `bytes` with `byte_records`, \
                             `slots` with `slots`)",
                            r.name.name, unit, fr.kind.name
                        ),
                    ));
                }
            }
            // Typo-safety: warn on unrecognized cursor attr keys.
            for a in &c.attrs {
                if !matches!(a.key.name.as_str(),
                    "at" | "repr" | "load" | "store" | "unit" | "kind") {
                    self.diags.push(Diag::warn(
                        a.span,
                        format!(
                            "ring_layout `{}`: unknown cursor attribute `{}` \
                             (recognized: at, repr, load, store, unit, kind)",
                            r.name.name, a.key.name
                        ),
                    ));
                }
            }
        }

        // Presence: the runtime needs a source for each of these. Today a
        // layout can omit them and still typecheck, leaving the consumer
        // with nothing to read.
        if r.magic.is_none() {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs a `magic` value — without it the \
                     consumer cannot validate it attached the right segment",
                    r.name.name
                ),
            ));
        }
        if (is_byte_records || is_slots) && r.data_at.is_none() {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: framing needs `data_at` (the first \
                     record/slot offset)",
                    r.name.name
                ),
            ));
        }
        // byte_records reads the data-region capacity from a `buffer_size`
        // scalar; slots derives it from slot_size × slot_count instead.
        if is_byte_records && !r.scalars.iter().any(|s| s.name.name == "buffer_size") {
            self.diags.push(Diag::ty(
                r.span,
                format!(
                    "ring_layout `{}`: needs a `buffer_size` scalar — the consumer \
                     reads the ring's data-region capacity from it",
                    r.name.name
                ),
            ));
        }

        // Overflow policy must be one of the known kinds.
        if let Some(ov) = &r.overflow {
            const POLICIES: &[&str] = &["lap_detect", "block", "drop", "fail"];
            if !POLICIES.contains(&ov.name.as_str()) {
                self.diags.push(Diag::ty(
                    ov.span,
                    format!(
                        "ring_layout `{}`: unknown `overflow {}` (expected one of \
                         lap_detect, block, drop, fail)",
                        r.name.name, ov.name
                    ),
                ));
            }
        }

        // Typo-safety: warn on unrecognized framing attr keys.
        if let Some(fr) = &r.framing {
            for a in &fr.attrs {
                if !matches!(a.key.name.as_str(),
                    "len_prefix" | "align" | "pad_sentinel" | "slot_size"
                    | "record_header_bytes" | "pad_field_offset"
                    | "pad_field_width" | "pad_field_value" | "recheck"
                    | "seq_offset" | "seq_width"
                    | "kernel_ns_offset" | "kernel_ns_width"
                    | "user_ns_offset" | "user_ns_width") {
                    self.diags.push(Diag::warn(
                        a.span,
                        format!(
                            "ring_layout `{}`: unknown framing attribute `{}` \
                             (recognized: len_prefix, align, pad_sentinel, \
                             slot_size, record_header_bytes, pad_field_offset, \
                             pad_field_width, pad_field_value, recheck)",
                            r.name.name, a.key.name
                        ),
                    ));
                }
            }
        }

        // #5 (fast-protocol-I/O): validate the record_header / pad_field /
        // recheck attrs (byte_records). record_header_bytes is the fixed
        // per-record header before the payload; it must be a multiple of
        // `align` (so the reader's align_up(hdr + len, align) stride equals
        // header + align(len)) and at least len_prefix wide (the len field
        // lives at record offset 0). A pad_field (a header discriminant
        // marking a tail pad, e.g. the reference crate kind==1) must fit inside the
        // header. `recheck` only knows `post_copy`.
        if let Some(fr) = &r.framing {
            let attr_int = |key: &str| -> Option<i64> {
                fr.attrs.iter().find_map(|a| match (&a.key.name[..], &a.value) {
                    (k, RingAttrValue::Int(n)) if k == key => Some(*n),
                    _ => None,
                })
            };
            let align = attr_int("align").unwrap_or(1).max(1) as u64;
            let len_prefix_w = fr
                .attrs
                .iter()
                .find(|a| a.key.name == "len_prefix")
                .and_then(|a| match &a.value {
                    RingAttrValue::Ident(id) => Some(match id.name.as_str() {
                        "u8" | "i8" => 1u64,
                        "u16" | "i16" => 2,
                        "u32" | "i32" | "f32" => 4,
                        "u64" | "i64" | "f64" => 8,
                        _ => 0,
                    }),
                    RingAttrValue::Int(n) => Some(*n as u64),
                })
                .unwrap_or(0);
            let rh = attr_int("record_header_bytes");
            if let Some(rhv) = rh {
                let span = fr.span;
                if rhv <= 0 {
                    self.diags.push(Diag::ty(span, format!(
                        "ring_layout `{}`: record_header_bytes must be positive",
                        r.name.name)));
                } else {
                    let rhu = rhv as u64;
                    if align > 1 && rhu % align != 0 {
                        self.diags.push(Diag::ty(span, format!(
                            "ring_layout `{}`: record_header_bytes ({}) must be a \
                             multiple of align ({}) — the record stride is \
                             header + align(len)", r.name.name, rhu, align)));
                    }
                    if len_prefix_w != 0 && rhu < len_prefix_w {
                        self.diags.push(Diag::ty(span, format!(
                            "ring_layout `{}`: record_header_bytes ({}) is smaller \
                             than len_prefix ({}) — the length field at record \
                             offset 0 would not fit", r.name.name, rhu, len_prefix_w)));
                    }
                }
            }
            // pad_field: offset/width/value must be coherent and fit the header.
            let pad_off = attr_int("pad_field_offset");
            let pad_w = attr_int("pad_field_width");
            let pad_v = attr_int("pad_field_value");
            if pad_off.is_some() || pad_w.is_some() || pad_v.is_some() {
                let span = fr.span;
                match (pad_off, pad_w, pad_v) {
                    (Some(off), Some(w), Some(_)) => {
                        if !matches!(w, 1 | 2 | 4 | 8) {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: pad_field_width must be 1/2/4/8, got {}",
                                r.name.name, w)));
                        }
                        if off < 0 {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: pad_field_offset must be non-negative",
                                r.name.name)));
                        } else if let Some(rhv) = rh {
                            if rhv > 0 && (off + w) as i64 > rhv {
                                self.diags.push(Diag::ty(span, format!(
                                    "ring_layout `{}`: pad_field [{}, {}) falls outside \
                                     the {}-byte record header",
                                    r.name.name, off, off + w, rhv)));
                            }
                        }
                    }
                    _ => self.diags.push(Diag::ty(span, format!(
                        "ring_layout `{}`: a pad_field needs all of pad_field_offset, \
                         pad_field_width, pad_field_value", r.name.name))),
                }
            }
            // recheck: only post_copy is known.
            for a in &fr.attrs {
                if a.key.name == "recheck" {
                    let ok = matches!(&a.value, RingAttrValue::Ident(id) if id.name == "post_copy");
                    if !ok {
                        self.diags.push(Diag::ty(a.span, format!(
                            "ring_layout `{}`: unknown recheck mode (only `post_copy`)",
                            r.name.name)));
                    }
                }
            }
            // #5 follow-on: in-band header field offsets/widths
            // (seq/kernel_ns/user_ns) must fit inside the record header.
            for (off_key, w_key) in [
                ("seq_offset", "seq_width"),
                ("kernel_ns_offset", "kernel_ns_width"),
                ("user_ns_offset", "user_ns_width"),
            ] {
                let off = attr_int(off_key);
                let w = attr_int(w_key);
                if off.is_none() && w.is_none() {
                    continue;
                }
                let span = fr.span;
                match (off, w) {
                    (Some(o), Some(width)) => {
                        if !matches!(width, 1 | 2 | 4 | 8) {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: {} must be 1/2/4/8, got {}",
                                r.name.name, w_key, width)));
                        }
                        if o < 0 {
                            self.diags.push(Diag::ty(span, format!(
                                "ring_layout `{}`: {} must be non-negative",
                                r.name.name, off_key)));
                        } else if let Some(rhv) = rh {
                            if rhv > 0 && (o + width) as i64 > rhv {
                                self.diags.push(Diag::ty(span, format!(
                                    "ring_layout `{}`: header field [{}, {}) falls \
                                     outside the {}-byte record header",
                                    r.name.name, o, o + width, rhv)));
                            }
                        }
                    }
                    _ => self.diags.push(Diag::ty(span, format!(
                        "ring_layout `{}`: header field `{}` needs both {} and {}",
                        r.name.name, off_key.trim_end_matches("_offset"),
                        off_key, w_key))),
                }
            }
            // A declared in-band header field requires a record_header_bytes
            // (otherwise the reader's header is only the length prefix and
            // the field reads past the record — silent corruption), and no
            // two header fields (or a field and the length prefix at offset
            // 0) may overlap.
            let mut intervals: Vec<(&str, i64, i64)> = Vec::new();
            if len_prefix_w > 0 {
                intervals.push(("len_prefix", 0, len_prefix_w as i64));
            }
            let mut any_header_field = false;
            for (name, off_key, w_key) in [
                ("pad_field", "pad_field_offset", "pad_field_width"),
                ("seq", "seq_offset", "seq_width"),
                ("kernel_ns", "kernel_ns_offset", "kernel_ns_width"),
                ("user_ns", "user_ns_offset", "user_ns_width"),
            ] {
                if let (Some(o), Some(w)) = (attr_int(off_key), attr_int(w_key)) {
                    any_header_field = true;
                    if o >= 0 && matches!(w, 1 | 2 | 4 | 8) {
                        intervals.push((name, o, o + w));
                    }
                }
            }
            if any_header_field && rh.map_or(true, |v| v <= 0) {
                self.diags.push(Diag::ty(fr.span, format!(
                    "ring_layout `{}`: record_header_bytes must be declared \
                     (and positive) when in-band header fields (pad_field / \
                     seq / kernel_ns / user_ns) are used — without it the \
                     reader's header is only the length prefix and the field \
                     would read past the record",
                    r.name.name)));
            }
            for i in 0..intervals.len() {
                for j in (i + 1)..intervals.len() {
                    let (an, a0, a1) = intervals[i];
                    let (bn, b0, b1) = intervals[j];
                    if a0 < b1 && b0 < a1 {
                        self.diags.push(Diag::ty(fr.span, format!(
                            "ring_layout `{}`: header fields `{}` [{}, {}) and \
                             `{}` [{}, {}) overlap",
                            r.name.name, an, a0, a1, bn, b0, b1)));
                    }
                }
            }
        }
    }

    fn check_locus(&mut self, decl: &'a LocusDecl) {
        let info = match self.top.lookup(&decl.name.name) {
            Some(TopSymbol::Locus(info)) => info,
            _ => return,
        };
        let prev = self.current_locus.replace(info);

        // v1.x-FORM-1: verify the form annotation's shape
        // contract against the declared capacity. PR3 handles
        // shape verification; method synthesis lands in PR3b
        // (so call sites like `l.push(42)` still won't resolve
        // yet — that's expected for this PR).
        if let Some(form) = &decl.form {
            self.check_form_shape(decl, form);
        }

        // #18.6 — Hale enforces CQRS at the locus boundary:
        // methods on loci may not return locus values. Reject
        // such declarations with a span-targeted diagnostic
        // naming the canonical alternatives (accept-as-child +
        // contract reads, bus topics, delegation). See
        // spec/semantics.md § Locus method dispatch.
        self.check_no_locus_return(decl);

        // Validate that bus-subscribe handlers are declared on
        // the locus body (as fn members).
        let fn_members: BTreeMap<String, &FnDecl> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Fn(f) => Some((f.name.name.clone(), f)),
                _ => None,
            })
            .collect();
        for sub in &info.bus_subscribes {
            if !fn_members.contains_key(&sub.handler) {
                self.diags.push(Diag::ty(
                    sub.span,
                    format!(
                        "bus subscribe `{}` references handler `{}` which is \
                         not declared on locus `{}`",
                        sub.subject, sub.handler, info.name
                    ),
                ));
                continue;
            }
            // Open-question #24 MVP (2026-05-25): fallible-handler
            // check. Bus dispatch has no caller frame to address
            // a value return — a fallible handler would have
            // nowhere to send `out_err` to. So a fn that's
            // fallible-by-decl can't also be subscribed; the
            // subscription site is rejected, not the fn (one
            // fn may be referenced by zero or more handlers,
            // but each subscription points at one fn).
            if let Some(handler_fn) = fn_members.get(&sub.handler) {
                if let Some(payload_te) = &handler_fn.fallible {
                    self.diags.push(Diag::ty(
                        sub.span,
                        format!(
                            "bus subscribe `{}` references fn `{}` which \
                             declares `fallible({})` — bus-subscribed \
                             handlers can't be fallible because bus \
                             dispatch has no caller frame to address the \
                             error channel. Drop `fallible(E)` from the \
                             handler and route value-error structurally \
                             via an inline closure (a closure assertion \
                             firing into `on_failure`), or do the work in \
                             a separate fallible fn the handler calls and \
                             address the error inside the handler body \
                             with `or <disposition>`.",
                            sub.subject,
                            sub.handler,
                            crate::resolve::resolve_type_expr(
                                payload_te,
                                self.known,
                            )
                            .display(),
                        ),
                    ));
                }
            }
        }

        // F.8: contract compatibility. If this locus consumes
        // fields from coordinatees, the accept-child type
        // must expose each consumed field at a compatible
        // type. The check fires once per parent locus; the
        // child's expose-set must be a superset (by name) of
        // the parent's consume-set, with assignable types.
        if !info.contract_consume.is_empty() {
            self.check_contract_compatibility(info);
        }
        // M3 stage 4 (2026-07-02): expose-side validity. Codegen
        // treats `contract` members as pure declaration, so this is
        // the ONLY place a lying expose can be caught — an entry
        // must bind against something real on this locus (a params
        // field, a mode, or a fn member) at a matching type, or a
        // consuming parent type-checks against fiction.
        if !info.contract_expose.is_empty() {
            self.check_contract_expose_validity(info);
        }

        // F.31 (2026-05-23): validate the `placement { }` block
        // when present. The parser already enforced "main-only"
        // and required-Ident keys; here we check that each entry
        // references an actual main-locus `params` field whose
        // type is a locus. Pinned-restrictions (no accept(),
        // no closures) are checked at codegen time when
        // placement → runtime wiring fires.
        let placement_blocks: Vec<_> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Placement(pb) => Some(pb),
                _ => None,
            })
            .collect();
        if placement_blocks.len() > 1 {
            self.diags.push(Diag::ty(
                placement_blocks[1].span,
                format!(
                    "locus `{}` declares multiple `placement {{ }}` blocks; \
                     at most one is permitted",
                    info.name
                ),
            ));
        }
        // Topology Phase 1b: the (declare-only) `topology { }`
        // block that `pinned(node =)` / `pinned(l3 =)` resolve
        // against. Validate its internal consistency, then thread
        // it into placement validation for reference checking.
        let topology_blocks: Vec<_> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Topology(tb) => Some(tb),
                _ => None,
            })
            .collect();
        if topology_blocks.len() > 1 {
            self.diags.push(Diag::ty(
                topology_blocks[1].span,
                format!(
                    "locus `{}` declares multiple `topology {{ }}` blocks; \
                     at most one is permitted",
                    info.name
                ),
            ));
        }
        let topology = topology_blocks.first().copied();
        if let Some(tb) = topology {
            self.check_topology_block(info, tb);
        }
        if let Some(pb) = placement_blocks.first() {
            self.check_placement_block(info, pb, topology);
        }

        // 2026-06-01: `release(c: T)` is the death-side bookend of
        // `accept(c: T)` — it fires when an accept'd child of type
        // T completes, and declaring it marks T a "flow". Without a
        // matching `accept(c: T)` the locus never owns a T child, so
        // the release can never fire: it's a dead declaration and
        // almost always a mistake (wrong child type, or the author
        // forgot the `accept`). Reject it with a focused diagnostic.
        for member in &decl.members {
            if let LocusMember::Lifecycle(lc) = member {
                if lc.kind == LifecycleKind::Release {
                    let child_name = lc.params.first().and_then(|p| {
                        match &p.ty {
                            TypeExpr::Named { path, .. } => path
                                .segments
                                .last()
                                .map(|s| s.name.clone()),
                            _ => None,
                        }
                    });
                    match child_name {
                        Some(name) if locus_accepts(decl, &name) => {}
                        Some(name) => self.diags.push(Diag::ty(
                            lc.span,
                            format!(
                                "locus `{}` declares `release(c: {})` but has \
                                 no matching `accept(c: {})` — release is the \
                                 death-side bookend of accept and can only \
                                 fire for an accept'd child type",
                                info.name, name, name
                            ),
                        )),
                        None => {}
                    }
                }
            }
        }

        for member in &decl.members {
            self.check_locus_member(member);
        }

        self.current_locus = prev;
    }

    /// F.31: validate a `placement { field: spec; }` block on
    /// `main locus`. Each entry must:
    ///   1. Reference a declared `params` field on this locus
    ///      (the parser only enforces "main-only" and Ident
    ///      keying).
    ///   2. The referenced field must be a locus type —
    ///      placement applies only to locus instances, not
    ///      primitives or structs.
    ///   3. No duplicate field keys.
    ///
    /// Pinned-class restrictions (no `accept()`, no closures
    /// on a locus placed `pinned`) move to placement-time
    /// enforcement in Phase 3 codegen; the spec lock is here
    /// but the typecheck implementation is deferred until
    /// codegen reads placement.
    fn check_placement_block(
        &mut self,
        info: &crate::symbol::LocusInfo,
        pb: &hale_syntax::ast::PlacementBlock,
        topology: Option<&hale_syntax::ast::TopologyBlock>,
    ) {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for entry in &pb.entries {
            // (3) duplicate check
            if !seen.insert(entry.field.name.clone()) {
                self.diags.push(Diag::ty(
                    entry.span,
                    format!(
                        "placement entry: duplicate field `{}` (each \
                         field may have at most one placement spec)",
                        entry.field.name
                    ),
                ));
                continue;
            }
            // (1) field exists in this locus's params
            let param = info
                .params
                .iter()
                .find(|p| p.name == entry.field.name);
            let param = match param {
                Some(p) => p,
                None => {
                    self.diags.push(Diag::ty(
                        entry.field.span,
                        format!(
                            "placement entry: field `{}` is not declared in \
                             locus `{}`'s params block",
                            entry.field.name, info.name
                        ),
                    ));
                    continue;
                }
            };
            // (2) field's type must be a locus type. `Ty::Named(L)`
            // where L resolves to a `TopSymbol::Locus`. Unknown
            // is permissive (cross-seed or stdlib loci resolve to
            // Unknown — match the existing assignable_from rule).
            match &param.ty {
                Ty::Named(name) => {
                    let is_locus = matches!(
                        self.top.lookup(name),
                        Some(TopSymbol::Locus(_))
                    );
                    let is_unknown_external = !self.top.symbols.contains_key(name);
                    if !is_locus && !is_unknown_external {
                        self.diags.push(Diag::ty(
                            entry.field.span,
                            format!(
                                "placement entry: field `{}` has type `{}` \
                                 which is not a locus type; placement applies \
                                 only to locus instances",
                                entry.field.name,
                                param.ty.display()
                            ),
                        ));
                    }
                }
                Ty::Unknown => {
                    // Cross-seed / stdlib locus — be permissive,
                    // matching assignable_from's Unknown rule.
                }
                other => {
                    self.diags.push(Diag::ty(
                        entry.field.span,
                        format!(
                            "placement entry: field `{}` has type `{}` \
                             which is not a locus type; placement applies \
                             only to locus instances",
                            entry.field.name,
                            other.display()
                        ),
                    ));
                }
            }
            // Topology Phase 1a: `pinned(cores = ...)` static
            // validity. The spec is closed-world (literal bounds),
            // so an empty range or a duplicated set element is a
            // definite authoring error, catchable here. Whether
            // the cores exist on the deploy box stays best-effort
            // at runtime (same contract as `pinned(core = N)` on
            // a smaller CI machine).
            if let PlacementSpec::Pinned {
                affinity: PinAffinity::Cores(spec),
                ..
            } = &entry.spec
            {
                match spec {
                    CoreSpec::Range { lo, hi, inclusive } => {
                        let empty =
                            if *inclusive { hi < lo } else { hi <= lo };
                        if empty {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `cores = {}..{}{}` \
                                     selects no cores — the range is empty. \
                                     (`A..B` excludes B; use `A..=B` to \
                                     include it.)",
                                    entry.field.name,
                                    lo,
                                    if *inclusive { "=" } else { "" },
                                    hi,
                                ),
                            ));
                        }
                    }
                    CoreSpec::Set(v) => {
                        let mut sorted = v.clone();
                        sorted.sort_unstable();
                        sorted.dedup();
                        if sorted.len() != v.len() {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: duplicate core \
                                     index in `cores = {{...}}` set",
                                    entry.field.name
                                ),
                            ));
                        }
                    }
                    CoreSpec::Single(_) => {}
                }
            }

            // Topology Phase 1b: `pinned(node = N)` / `pinned(l3 =
            // name)` must reference a domain declared in the
            // `topology { }` block. Using either with no topology
            // block, or naming an undeclared node/domain, is a
            // definite authoring error (closed-world resolution).
            if let PlacementSpec::Pinned { affinity, replicas } =
                &entry.spec
            {
                // Topology Phase 1c: `replicas = K` must be >= 1.
                // `Some(0)` / negative fans out nothing (or is a
                // typo); `None` / `Some(1)` is a single instance.
                if let Some(k) = replicas {
                    if *k < 1 {
                        self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "placement entry `{}`: `replicas = {}` must \
                                 be at least 1 (it fans the locus into K \
                                 single-threaded instances)",
                                entry.field.name, k
                            ),
                        ));
                    }
                }
                match affinity {
                    PinAffinity::Node(n) => match topology {
                        None => self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "placement entry `{}`: `pinned(node = {})` \
                                 needs a `topology {{ }}` block to resolve \
                                 the node to its cores — none is declared \
                                 on `{}`",
                                entry.field.name, n, info.name
                            ),
                        )),
                        Some(tb) if tb.node_cores(*n).is_none() => {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `pinned(node = \
                                     {})` references NUMA node `{}`, which \
                                     the `topology {{ }}` block does not \
                                     declare",
                                    entry.field.name, n, n
                                ),
                            ));
                        }
                        Some(tb)
                            if tb
                                .node_cores(*n)
                                .map(|c| c.is_empty())
                                .unwrap_or(false) =>
                        {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `pinned(node = \
                                     {})` resolves to no cores — node `{}` \
                                     declares no `l3` domains",
                                    entry.field.name, n, n
                                ),
                            ));
                        }
                        _ => {}
                    },
                    PinAffinity::L3(name) => match topology {
                        None => self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "placement entry `{}`: `pinned(l3 = {})` \
                                 needs a `topology {{ }}` block to resolve \
                                 the cache domain to its cores — none is \
                                 declared on `{}`",
                                entry.field.name, name.name, info.name
                            ),
                        )),
                        Some(tb) if tb.l3_cores(&name.name).is_none() => {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "placement entry `{}`: `pinned(l3 = {})` \
                                     references cache domain `{}`, which the \
                                     `topology {{ }}` block does not declare",
                                    entry.field.name, name.name, name.name
                                ),
                            ));
                        }
                        _ => {}
                    },
                    PinAffinity::Any | PinAffinity::Cores(_) => {}
                }
            }

            // (The dead-bus-receiver check moved to
            // `check_cooperative_pool_blocking` and was corrected: a
            // non-main cooperative subscriber is dead only when its
            // `run()` ALSO makes a blocking call that starves the pool
            // thread — placement alone over-fires on event-driven
            // subscribers, which receive fine. See that fn.)

            // F.35: per-entry constraint validity.
            for c in &entry.constraints {
                match c.kind {
                    PlacementConstraint::AsyncIo => {
                        // Phase-1 macOS portability: the async_io pool
                        // backend is epoll/eventfd/ucontext-based and
                        // Linux-only. When the compiler is running on
                        // macOS, reject `where async_io` at compile time
                        // with actionable guidance — mirroring the
                        // wasm-target stdlib gating. The C runtime's
                        // async_io functions are inert stubs on macOS
                        // (LOTUS_HAVE_ASYNC_IO == 0), so this diagnostic
                        // is the clean-failure path (vs a link error).
                        if cfg!(target_os = "macos") {
                            self.diags.push(Diag::ty(
                                c.span,
                                format!(
                                    "placement entry `{}`: `async_io` pools \
                                     aren't supported on macOS yet — use a \
                                     cooperative pool (drop `where async_io`), \
                                     or build on Linux. (A kqueue/poll backend \
                                     is planned.)",
                                    entry.field.name
                                ),
                            ));
                        }
                        match &entry.spec {
                            PlacementSpec::Pinned { .. } => {
                                self.diags.push(Diag::ty(
                                    c.span,
                                    format!(
                                        "placement entry `{}`: `where async_io` \
                                         is not valid on a pinned placement. \
                                         Pinned loci own their own OS thread \
                                         and have no shared drain loop to \
                                         park on. Use `cooperative(pool = X) \
                                         where async_io` instead.",
                                        entry.field.name
                                    ),
                                ));
                            }
                            PlacementSpec::Cooperative { pool } => {
                                let pool_name = pool
                                    .as_ref()
                                    .map(|i| i.name.as_str())
                                    .unwrap_or("main");
                                if pool_name == "main" {
                                    self.diags.push(Diag::ty(
                                        c.span,
                                        format!(
                                            "placement entry `{}`: `where \
                                             async_io` is not valid on pool \
                                             `main`. The main pool runs \
                                             inline on the binary's primary \
                                             thread, with no dedicated \
                                             worker thread to integrate \
                                             epoll into. Move the field to \
                                             a named cooperative pool (e.g. \
                                             `cooperative(pool = io) where \
                                             async_io`).",
                                            entry.field.name
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        // F.35 cross-entry rule: every entry on the same named
        // cooperative pool must agree on whether the pool runs in
        // async_io mode. Mixing an async_io entry with a non-
        // async_io entry on the same pool is rejected because the
        // pool's worker drain loop is one-or-the-other.
        let mut pool_async_io: BTreeMap<String, bool> = BTreeMap::new();
        let mut pool_first_span: BTreeMap<String, hale_syntax::Span> =
            BTreeMap::new();
        for entry in &pb.entries {
            let pool_name = match &entry.spec {
                PlacementSpec::Cooperative { pool: Some(name) } => {
                    name.name.clone()
                }
                _ => continue,
            };
            if pool_name == "main" {
                continue;
            }
            let has_async_io = entry.constraints.iter().any(|c| {
                matches!(c.kind, PlacementConstraint::AsyncIo)
            });
            match pool_async_io.get(&pool_name).copied() {
                None => {
                    pool_async_io.insert(pool_name.clone(), has_async_io);
                    pool_first_span.insert(pool_name, entry.span);
                }
                Some(prev) if prev == has_async_io => {}
                Some(_) => {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "placement entry `{}`: pool `{}` has mixed I/O \
                             modes across placement entries. Every entry on \
                             a pool must either declare `where async_io` or \
                             none must; the pool's worker drain loop is \
                             one-or-the-other. (The pool first appeared at \
                             the entry whose I/O mode is the other; pick \
                             one and apply consistently.)\n\nNote: \
                             `where async_io` governs non-blocking I/O \
                             readiness — it makes blocking `recv`/`accept`/\
                             `send` park-and-resume instead of holding the \
                             thread. It does NOT affect bus delivery or \
                             handler dispatch; if a `subscribe` handler \
                             isn't firing, `async_io` is not the fix (check \
                             the locus's placement instead).",
                            entry.field.name, pool_name
                        ),
                    ));
                }
            }
        }
    }

    /// Topology Phase 1b: validate the `topology { }` block's
    /// internal consistency (declare-only, so everything is
    /// checkable statically):
    ///   - node ids are unique;
    ///   - L3-domain names are globally unique (they're referenced
    ///     by `pinned(l3 = name)` without node qualification);
    ///   - each `cores` spec is well-formed (non-empty range, no
    ///     duplicate set element — same rules as `pinned(cores)`);
    ///   - no core belongs to two L3 domains (ambiguous affinity);
    ///   - no domain core overlaps a `reserve`d core (reserved
    ///     cores are held back for the OS / main).
    /// Whether the cores exist on the deploy box stays best-effort
    /// at runtime — the machine-match is not enforced here.
    fn check_topology_block(
        &mut self,
        info: &crate::symbol::LocusInfo,
        tb: &hale_syntax::ast::TopologyBlock,
    ) {
        let _ = info;
        for spec in &tb.reserved {
            self.check_core_spec_wellformed(spec, tb.span, "reserve cores");
        }
        let reserved: BTreeSet<i64> =
            tb.reserved_cores().into_iter().collect();

        let mut node_ids: BTreeSet<i64> = BTreeSet::new();
        let mut l3_names: BTreeSet<String> = BTreeSet::new();
        let mut core_owner: BTreeMap<i64, String> = BTreeMap::new();

        for node in &tb.nodes {
            if !node_ids.insert(node.id) {
                self.diags.push(Diag::ty(
                    node.id_span,
                    format!(
                        "topology: duplicate NUMA node id `{}` — each \
                         `node N` must have a distinct id",
                        node.id
                    ),
                ));
            }
            for d in &node.domains {
                if !l3_names.insert(d.name.name.clone()) {
                    self.diags.push(Diag::ty(
                        d.name.span,
                        format!(
                            "topology: duplicate L3 domain name `{}` — \
                             domain names are referenced by `pinned(l3 = \
                             {})` and must be globally unique across nodes",
                            d.name.name, d.name.name
                        ),
                    ));
                }
                self.check_core_spec_wellformed(
                    &d.cores,
                    d.span,
                    &format!("l3 {}", d.name.name),
                );
                for c in d.cores.expand() {
                    if let Some(prev) = core_owner.get(&c) {
                        if prev != &d.name.name {
                            self.diags.push(Diag::ty(
                                d.span,
                                format!(
                                    "topology: core {} is claimed by both \
                                     L3 domains `{}` and `{}` — a core \
                                     belongs to at most one cache domain",
                                    c, prev, d.name.name
                                ),
                            ));
                        }
                    } else {
                        core_owner.insert(c, d.name.name.clone());
                    }
                    if reserved.contains(&c) {
                        self.diags.push(Diag::ty(
                            d.span,
                            format!(
                                "topology: core {} is both `reserve`d and \
                                 assigned to L3 domain `{}` — reserved cores \
                                 are held back for the OS / main and can't be \
                                 placed on",
                                c, d.name.name
                            ),
                        ));
                    }
                }
            }
        }
    }

    /// Shared well-formedness check for a `CoreSpec` literal
    /// (used by both `topology { }` domain/reserve specs and the
    /// `pinned(cores)` placement path): a range must select at
    /// least one core, a set must have no duplicate element.
    fn check_core_spec_wellformed(
        &mut self,
        spec: &CoreSpec,
        span: hale_syntax::Span,
        ctx: &str,
    ) {
        match spec {
            CoreSpec::Range { lo, hi, inclusive } => {
                let empty = if *inclusive { hi < lo } else { hi <= lo };
                if empty {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "topology `{}`: `{}..{}{}` selects no cores — \
                             the range is empty (`A..B` excludes B; use \
                             `A..=B` to include it)",
                            ctx,
                            lo,
                            if *inclusive { "=" } else { "" },
                            hi,
                        ),
                    ));
                }
            }
            CoreSpec::Set(v) => {
                let mut sorted = v.clone();
                sorted.sort_unstable();
                sorted.dedup();
                if sorted.len() != v.len() {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "topology `{}`: duplicate core index in the \
                             `{{...}}` set",
                            ctx
                        ),
                    ));
                }
            }
            CoreSpec::Single(_) => {}
        }
    }

    /// #18.6 — Hale enforces CQRS at the locus boundary:
    /// methods on loci may not return locus values. The pattern
    /// `fn factory(...) -> SomeLocus` is rejected at the
    /// declaration site.
    ///
    /// The lotus model treats loci as managed entities — they
    /// live as accepted children of a parent, expose data
    /// through `contract`, communicate cross-tower through the
    /// bus. Returning an entity from a method puts the entity
    /// into a stranger position at every call site (LoD), mixes
    /// command/query semantics (CQRS), and depends on a
    /// concretion rather than an abstraction (Dependency
    /// Inversion). Mechanically, every call leaks via the m90
    /// payload-arena routing.
    ///
    /// The five lenses (SOLID, LoD, CQRS, mechanical sympathy,
    /// the lotus model itself) converge on the same rule: a
    /// method must return data, not an entity. The compiler
    /// enforces it.
    ///
    /// Three canonical remedies in the diagnostic:
    ///   1. Parent-child: `accept(c: T)` + contract reads
    ///   2. Bus topic: publish events; receiver subscribes
    ///   3. Delegation: expose the operation directly on the
    ///      owning locus
    ///
    /// Free fns can still return loci (entity creation —
    /// `std::io::file::open(path) -> File fallible(IoError)`).
    /// Lifecycle methods / modes / failure handlers don't have
    /// return types in the value-bearing sense, so they're
    /// unaffected.
    ///
    /// Spec home: `spec/semantics.md § Locus method dispatch`.
    fn check_no_locus_return(&mut self, decl: &'a LocusDecl) {
        for member in &decl.members {
            let LocusMember::Fn(f) = member else { continue };
            // Walk the declared return type (if any) and the
            // fallible-payload type (if any). Both can carry a
            // locus.
            if let Some(ret) = &f.ret {
                self.report_locus_return(decl, f, ret, "return type");
            }
            if let Some(payload) = &f.fallible {
                // `fallible(L)` carries the error payload type.
                // A locus payload is the same antipattern as a
                // locus return — the caller would have to call
                // methods on the recovered locus, violating the
                // friendship boundary the same way.
                self.report_locus_return(
                    decl, f, payload, "fallible payload type",
                );
            }
        }
    }

    fn report_locus_return(
        &mut self,
        decl: &'a LocusDecl,
        f: &'a FnDecl,
        ty_expr: &'a TypeExpr,
        slot_label: &str,
    ) {
        let resolved = resolve_type_expr(ty_expr, self.known);
        let Ty::Named(name) = &resolved else { return };
        if !matches!(self.top.lookup(name), Some(TopSymbol::Locus(_))) {
            return;
        }
        self.diags.push(Diag::ty(
            ty_expr.span(),
            format!(
                "method `{locus}.{method}` declares {slot} `{ret}` — \
                 methods on loci may not return locus values.\n\n\
                 The lotus model treats loci as managed entities (parent-\
                 child accept, contract exposure, bus topics). Returning \
                 an entity from a method puts it in a stranger position \
                 at every call site (violating LoD), mixes \
                 command/query semantics (CQRS), and depends on a \
                 concretion rather than an abstraction. Mechanically, \
                 every call leaks via the m90 payload-arena routing.\n\n\
                 Rewrite as one of:\n\
                 1. Parent-child: declare `accept(c: {ret})` on the \
                    parent and read via `contract {{ expose ... }}`.\n\
                 2. Bus topic: publish events; receiver subscribes.\n\
                 3. Delegation: expose the operation directly on `{locus}`.\n\n\
                 Free fns can still return loci (entity creation \
                 patterns like `std::io::file::open`). See \
                 spec/semantics.md § Locus method dispatch.",
                locus = decl.name.name,
                method = f.name.name,
                slot = slot_label,
                ret = name,
            ),
        ));
    }

    /// v1.x-FORM-1: verify a `@form(<name>)` annotation's
    /// shape contract against the locus's actual capacity
    /// declaration. v1 ships shape checks for `@form(vec)`
    /// (FORM-2), `@form(hashmap)` (FORM-4), and
    /// `@form(ring_buffer)` (FORM-5).
    fn check_form_shape(&mut self, decl: &'a LocusDecl, form: &'a FormAnnotation) {
        match form.name.name.as_str() {
            "vec" => self.check_form_vec_shape(decl, form),
            "hashmap" => self.check_form_hashmap_shape(decl, form),
            "ring_buffer" => self.check_form_ring_buffer_shape(decl, form),
            "lru_cache" => self.check_form_lru_cache_shape(decl, form),
            other => {
                self.diags.push(Diag::ty(
                    form.name.span,
                    format!(
                        "unknown form `{}`; v1 recognizes: vec, hashmap, \
                         ring_buffer, lru_cache",
                        other
                    ),
                ));
            }
        }
    }

    /// v1.x-FORM-5: `@form(ring_buffer, cap = N)` requires
    /// exactly one capacity slot of kind `pool`, holding any
    /// cell type T. The `cap` annotation arg is required and
    /// must be a positive integer literal — the backing buffer
    /// is pre-allocated at locus birth and never grows.
    fn check_form_ring_buffer_shape(
        &mut self,
        decl: &'a LocusDecl,
        form: &'a FormAnnotation,
    ) {
        // Validate args: exactly one, named `cap`, positive int literal.
        let mut cap_arg: Option<&FormArg> = None;
        for arg in &form.args {
            if arg.name.name == "cap" {
                if cap_arg.is_some() {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(ring_buffer): duplicate `cap` arg".to_string(),
                    ));
                } else {
                    cap_arg = Some(arg);
                }
            } else {
                self.diags.push(Diag::ty(
                    arg.name.span,
                    format!(
                        "@form(ring_buffer): unknown arg `{}`; v1 accepts \
                         `cap = N` only",
                        arg.name.name
                    ),
                ));
            }
        }
        match cap_arg {
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(ring_buffer) requires a `cap = N` arg (fixed \
                     capacity; the buffer is pre-allocated at locus birth)"
                        .to_string(),
                ));
            }
            Some(arg) => match &arg.value {
                Expr::Literal(Literal::Int(n), _) if *n > 0 => {
                    // OK.
                }
                _ => {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(ring_buffer) `cap` must be a positive \
                         integer literal (v1 doesn't const-evaluate \
                         expressions for form args)"
                            .to_string(),
                    ));
                }
            },
        }

        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(ring_buffer) requires exactly one `pool` capacity \
                     slot; found no `capacity { ... }` block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        match cb.slots.len() {
            0 => {
                self.diags.push(Diag::ty(
                    cb.span,
                    "@form(ring_buffer) requires exactly one `pool` capacity \
                     slot; found an empty capacity block"
                        .to_string(),
                ));
            }
            1 => {
                let slot = &cb.slots[0];
                match slot.kind {
                    CapacitySlotKind::Pool => {
                        // OK.
                    }
                    CapacitySlotKind::Heap => {
                        self.diags.push(Diag::ty(
                            slot.span,
                            format!(
                                "@form(ring_buffer) requires a `pool` slot; \
                                 got `heap {} of ...`. Ring buffer recycles \
                                 fixed-capacity cells (pool discipline); \
                                 heap is the growable shape covered by \
                                 @form(vec).",
                                slot.name.name
                            ),
                        ));
                    }
                }
                if slot.as_parent_for.is_some() {
                    self.diags.push(Diag::ty(
                        slot.span,
                        "@form(ring_buffer) slot cannot also be an \
                         `as_parent_for` override; form-lowered slots own \
                         their own allocator"
                            .to_string(),
                    ));
                }
                if slot.indexed_by.is_some() {
                    self.diags.push(Diag::ty(
                        slot.span,
                        "@form(ring_buffer) slot does not take an `indexed_by` \
                         clause (that clause belongs to @form(hashmap))"
                            .to_string(),
                    ));
                }
            }
            n => {
                self.diags.push(Diag::ty(
                    cb.span,
                    format!(
                        "@form(ring_buffer) requires exactly one `pool` \
                         capacity slot; found {}",
                        n
                    ),
                ));
            }
        }
    }

    /// v1.x-FORM-6: `@form(lru_cache, cap = N)` is the one form
    /// that needs BOTH a key and a cap. It borrows the
    /// `@form(hashmap)` key surface — exactly one `pool` capacity
    /// slot with an `indexed_by <fieldname>` clause over a
    /// user-declared struct cell — AND the `@form(ring_buffer)`
    /// required `cap = N` positive-int-literal arg (the cache is
    /// pre-allocated at locus birth and evicts LRU on over-cap
    /// insert; it never grows).
    fn check_form_lru_cache_shape(
        &mut self,
        decl: &'a LocusDecl,
        form: &'a FormAnnotation,
    ) {
        // Args: exactly one `cap = N`, positive int literal.
        let mut cap_arg: Option<&FormArg> = None;
        for arg in &form.args {
            if arg.name.name == "cap" {
                if cap_arg.is_some() {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(lru_cache): duplicate `cap` arg".to_string(),
                    ));
                } else {
                    cap_arg = Some(arg);
                }
            } else {
                self.diags.push(Diag::ty(
                    arg.name.span,
                    format!(
                        "@form(lru_cache): unknown arg `{}`; v1 accepts \
                         `cap = N` only",
                        arg.name.name
                    ),
                ));
            }
        }
        match cap_arg {
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(lru_cache) requires a `cap = N` arg (fixed \
                     capacity; the cache is pre-allocated at locus birth \
                     and evicts the least-recently-used entry on over-cap \
                     insert — it never grows)"
                        .to_string(),
                ));
            }
            Some(arg) => match &arg.value {
                Expr::Literal(Literal::Int(n), _) if *n > 0 => { /* OK */ }
                _ => {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        "@form(lru_cache) `cap` must be a positive integer \
                         literal (v1 doesn't const-evaluate expressions for \
                         form args)"
                            .to_string(),
                    ));
                }
            },
        }

        // Capacity block: exactly one `pool` slot with `indexed_by`,
        // cell = user struct, indexed-by field exists on it.
        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(lru_cache) requires exactly one `pool` capacity \
                     slot with `indexed_by <fieldname>`; found no \
                     `capacity { ... }` block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        if cb.slots.is_empty() {
            self.diags.push(Diag::ty(
                cb.span,
                "@form(lru_cache) requires exactly one `pool` capacity slot \
                 with `indexed_by <fieldname>`; found an empty capacity block"
                    .to_string(),
            ));
            return;
        }
        if cb.slots.len() > 1 {
            self.diags.push(Diag::ty(
                cb.span,
                format!(
                    "@form(lru_cache) requires exactly one capacity slot; \
                     found {} slots. lru_cache is a single keyed store.",
                    cb.slots.len()
                ),
            ));
            return;
        }
        let slot = &cb.slots[0];
        match slot.kind {
            CapacitySlotKind::Pool => {}
            CapacitySlotKind::Heap => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(lru_cache) requires a `pool` slot; got `heap \
                         {} of ...`. The cache recycles a fixed cell \
                         population as entries are inserted and evicted — \
                         that's the `pool` discipline.",
                        slot.name.name
                    ),
                ));
            }
        }
        let field_ident = match &slot.indexed_by {
            Some(i) => i,
            None => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(lru_cache) slot `{}` must declare `indexed_by \
                         <fieldname>` naming the field of the cell type that \
                         serves as the cache key",
                        slot.name.name
                    ),
                ));
                return;
            }
        };
        let cell_name = match &slot.elem_ty {
            TypeExpr::Named { path, .. } if path.segments.len() == 1 => {
                path.segments[0].name.clone()
            }
            _ => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    "@form(lru_cache) cell type must be a user-declared \
                     struct (so the `indexed_by` field can resolve to a \
                     typed key); got a primitive, qualified path, or \
                     composite type"
                        .to_string(),
                ));
                return;
            }
        };
        match self.top.lookup(&cell_name) {
            Some(TopSymbol::Type(info)) => match &info.kind {
                TypeKind::Struct(fields) => {
                    if !fields.iter().any(|f| f.name == field_ident.name) {
                        self.diags.push(Diag::ty(
                            field_ident.span,
                            format!(
                                "@form(lru_cache) cell type `{}` has no field \
                                 `{}` — the `indexed_by` field must exist on \
                                 the cell struct",
                                cell_name, field_ident.name
                            ),
                        ));
                        return;
                    }
                }
                TypeKind::Enum(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(lru_cache) cell type `{}` is an enum; cell \
                             must be a struct so `indexed_by` can resolve to a \
                             typed key field",
                            cell_name
                        ),
                    ));
                    return;
                }
                TypeKind::Alias(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(lru_cache) cell type `{}` is a type alias; \
                             cell must be a struct so `indexed_by` can resolve",
                            cell_name
                        ),
                    ));
                    return;
                }
            },
            Some(TopSymbol::Locus(_)) => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    format!(
                        "@form(lru_cache) cell type `{}` is a locus. Cells \
                         are data; loci are managed entities. Store data in \
                         the cache and route entity membership through \
                         `accept(c: ...)` instead.",
                        cell_name
                    ),
                ));
                return;
            }
            _ => {
                // Cell type unresolved — separate error already
                // raised by the type resolver. Skip so we don't
                // double-report.
                return;
            }
        }
        if slot.as_parent_for.is_some() {
            self.diags.push(Diag::ty(
                slot.span,
                "@form(lru_cache) slot cannot also be an `as_parent_for` \
                 override; form-lowered slots own their own allocator"
                    .to_string(),
            ));
        }
    }

    /// v1.x-FORM-4: `@form(hashmap)` requires exactly one
    /// capacity slot, of kind `pool`, with an `indexed_by
    /// <fieldname>` clause. The slot's cell type must be a
    /// user-declared struct; the indexed-by field must exist
    /// on that struct. The field's type becomes the hashmap
    /// key type K; the cell type becomes the value type S.
    fn check_form_hashmap_shape(&mut self, decl: &'a LocusDecl, form: &'a FormAnnotation) {
        // F.32-1α/β2/γ (2026-05-24 → 2026-05-25): @form(hashmap)
        // accepts optional kwargs:
        //   sync = X  (X ∈ {none, serialized, striped, lockfree})
        //   cap  = N  (positive int literal; REQUIRED when
        //              sync = lockfree, rejected otherwise)
        //
        // Plain `@form(hashmap)` keeps the single-pool default
        // (no sync overhead; cross-pool calls typecheck-rejected
        // per F.32-0).
        let mut sync_value: Option<&str> = None;
        let mut cap_arg: Option<&FormArg> = None;
        for arg in &form.args {
            match arg.name.name.as_str() {
                "sync" => {
                    let val = match &arg.value {
                        Expr::Ident(i) => i.name.as_str(),
                        _ => {
                            self.diags.push(Diag::ty(
                                arg.span,
                                "@form(hashmap, sync = X): X must be a \
                                 bare identifier (one of `serialized`, \
                                 `striped`, `lockfree`)".to_string(),
                            ));
                            continue;
                        }
                    };
                    match val {
                        "serialized" => { sync_value = Some("serialized"); }
                        "striped"    => { sync_value = Some("striped"); }
                        "lockfree"   => { sync_value = Some("lockfree"); }
                        "none"       => { /* same as omitting */ }
                        other => {
                            self.diags.push(Diag::ty(
                                arg.span,
                                format!(
                                    "@form(hashmap, sync = {}): unknown sync \
                                     discipline; v1 accepts `serialized` \
                                     (F.32-1α), `striped` (F.32-1β2), and \
                                     `lockfree` (F.32-1γ-v1).",
                                    other
                                ),
                            ));
                        }
                    }
                }
                "cap" => {
                    cap_arg = Some(arg);
                }
                other => {
                    self.diags.push(Diag::ty(
                        arg.name.span,
                        format!(
                            "@form(hashmap): unknown arg `{}`; v1 accepts \
                             `sync = X` and (when sync = lockfree) `cap = N`",
                            other
                        ),
                    ));
                }
            }
        }
        // F.32-1γ-v1/v2: lockfree accepts `cap = N` as an
        // initial-size hint. Pre-v2 (no grow path) the cap was
        // required because the table couldn't grow; v2 ships
        // grow (2026-05-26) so cap is now optional — omitting it
        // starts the table at LOTUS_HASHMAP_INITIAL_CAP and
        // grows on demand. Other sync modes still reject cap
        // (they have their own initial size + grow policy).
        match (sync_value, cap_arg) {
            (Some("lockfree"), Some(arg)) => {
                match &arg.value {
                    Expr::Literal(Literal::Int(n), _) if *n > 0 => {
                        /* OK */
                    }
                    _ => {
                        self.diags.push(Diag::ty(
                            arg.span,
                            "@form(hashmap, sync = lockfree) `cap` must be a \
                             positive integer literal (v1 doesn't const-evaluate \
                             expressions for form args)".to_string(),
                        ));
                    }
                }
            }
            (_, Some(arg)) => {
                self.diags.push(Diag::ty(
                    arg.name.span,
                    "@form(hashmap): `cap = N` is only valid with \
                     `sync = lockfree`. Other sync modes (none, serialized, \
                     striped) grow dynamically; their initial cap is \
                     LOTUS_HASHMAP_INITIAL_CAP (8) and managed by the \
                     runtime."
                        .to_string(),
                ));
            }
            _ => { /* nothing else to check */ }
        }
        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(hashmap) requires exactly one `pool` capacity slot \
                     with `indexed_by <fieldname>`; found no `capacity { ... }` \
                     block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        if cb.slots.is_empty() {
            self.diags.push(Diag::ty(
                cb.span,
                "@form(hashmap) requires exactly one `pool` capacity slot \
                 with `indexed_by <fieldname>`; found an empty capacity block"
                    .to_string(),
            ));
            return;
        }
        if cb.slots.len() > 1 {
            self.diags.push(Diag::ty(
                cb.span,
                format!(
                    "@form(hashmap) requires exactly one capacity slot; \
                     found {} slots. Hashmap is a single keyed store.",
                    cb.slots.len()
                ),
            ));
            return;
        }
        let slot = &cb.slots[0];
        // Slot kind must be Pool (cells recycle as entries come
        // and go); Heap doesn't model the "bounded recyclable
        // population" the hashmap needs.
        match slot.kind {
            CapacitySlotKind::Pool => {}
            CapacitySlotKind::Heap => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(hashmap) requires a `pool` slot; got `heap {} \
                         of ...`. Hashmap recycles cells as entries are \
                         inserted and removed — that's the `pool` discipline. \
                         `heap` is the unordered growable shape (use @form(vec)).",
                        slot.name.name
                    ),
                ));
            }
        }
        // Slot must declare `indexed_by <fieldname>`.
        let field_ident = match &slot.indexed_by {
            Some(i) => i,
            None => {
                self.diags.push(Diag::ty(
                    slot.span,
                    format!(
                        "@form(hashmap) slot `{}` must declare `indexed_by \
                         <fieldname>` naming the field of the cell type that \
                         serves as the hashmap key",
                        slot.name.name
                    ),
                ));
                return;
            }
        };
        // The cell type must be a user-declared struct so we can
        // verify the indexed-by field exists. Primitives, enums,
        // and locus refs are rejected.
        let cell_name = match &slot.elem_ty {
            TypeExpr::Named { path, .. } if path.segments.len() == 1 => {
                path.segments[0].name.clone()
            }
            _ => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    "@form(hashmap) cell type must be a user-declared struct \
                     (so the `indexed_by` field can resolve to a typed key); \
                     got a primitive, qualified path, or composite type"
                        .to_string(),
                ));
                return;
            }
        };
        let field_ty = match self.top.lookup(&cell_name) {
            Some(TopSymbol::Type(info)) => match &info.kind {
                TypeKind::Struct(fields) => {
                    match fields.iter().find(|f| f.name == field_ident.name) {
                        Some(f) => f.ty.clone(),
                        None => {
                            self.diags.push(Diag::ty(
                                field_ident.span,
                                format!(
                                    "@form(hashmap) cell type `{}` has no field \
                                     `{}` — the `indexed_by` field must exist on \
                                     the cell struct",
                                    cell_name, field_ident.name
                                ),
                            ));
                            return;
                        }
                    }
                }
                TypeKind::Enum(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(hashmap) cell type `{}` is an enum; cell \
                             must be a struct so `indexed_by` can resolve to a \
                             typed key field",
                            cell_name
                        ),
                    ));
                    return;
                }
                TypeKind::Alias(_) => {
                    self.diags.push(Diag::ty(
                        slot.elem_ty.span(),
                        format!(
                            "@form(hashmap) cell type `{}` is a type alias; \
                             cell must be a struct so `indexed_by` can resolve",
                            cell_name
                        ),
                    ));
                    return;
                }
            },
            Some(TopSymbol::Locus(_)) => {
                self.diags.push(Diag::ty(
                    slot.elem_ty.span(),
                    format!(
                        "@form(hashmap) cell type `{}` is a locus. Cells \
                         are data; loci are managed entities. Storing an \
                         entity in a hashmap means the synthesized `.get \
                         (key)` returns a stranger to the caller, which \
                         violates the rule in `spec/semantics.md § Locus \
                         method dispatch` (same shape as a method returning \
                         a locus).\n\n\
                         Canonical alternatives for keyed-children patterns:\n\
                         1. Parent-child: declare `accept(c: {})` on the \
                            parent. Pair with a `@form(hashmap)` of cell \
                            type `type Index {{ key: String; child_idx: \
                            Int; }}` if name-based lookup is needed.\n\
                         2. Bus topic: publish commands keyed by name; \
                            subscriber dispatches into the right child.\n\
                         3. Delegation: collapse the per-child operation \
                            onto the parent (`parent.inc_named(name)`).\n\n\
                         See spec/forms.md § @form(hashmap) cell type and \
                         spec/semantics.md § Locus method dispatch.",
                        cell_name, cell_name
                    ),
                ));
                return;
            }
            _ => {
                // Cell type unresolved — separate error already
                // raised by the type resolver. Skip further checks
                // so we don't double-report.
                return;
            }
        };
        // as_parent_for and form-lowered slots don't compose:
        // the form owns the slot's allocator.
        if slot.as_parent_for.is_some() {
            self.diags.push(Diag::ty(
                slot.span,
                "@form(hashmap) slot cannot also be an `as_parent_for` \
                 override; form-lowered slots own their own allocator"
                    .to_string(),
            ));
        }
        // PR3 reads the key type `field_ty` to synthesize methods;
        // for now we just verified it resolves.
        let _ = field_ty;
    }

    /// v1.x-FORM-1: `@form(vec)` requires exactly one capacity
    /// slot, of kind `heap`, holding any cell type T. The slot
    /// name is user-chosen and not part of the contract.
    fn check_form_vec_shape(&mut self, decl: &'a LocusDecl, form: &'a FormAnnotation) {
        if !form.args.is_empty() {
            self.diags.push(Diag::ty(
                form.span,
                format!(
                    "@form(vec) takes no arguments; got {} (vec has no \
                     tuning knobs in v1 — drop the arg list)",
                    form.args.len()
                ),
            ));
        }
        let capacity = decl.members.iter().find_map(|m| match m {
            LocusMember::Capacity(cb) => Some(cb),
            _ => None,
        });
        let cb = match capacity {
            Some(cb) => cb,
            None => {
                self.diags.push(Diag::ty(
                    form.span,
                    "@form(vec) requires exactly one `heap` capacity slot; \
                     found no `capacity { ... }` block on this locus"
                        .to_string(),
                ));
                return;
            }
        };
        match cb.slots.len() {
            0 => {
                self.diags.push(Diag::ty(
                    cb.span,
                    "@form(vec) requires exactly one `heap` capacity slot; \
                     found an empty capacity block"
                        .to_string(),
                ));
                return;
            }
            1 => {
                let slot = &cb.slots[0];
                match slot.kind {
                    CapacitySlotKind::Heap => {
                        // OK: the contract is satisfied.
                        // Cell type T is whatever's declared;
                        // PR3b synthesizes methods over it.
                    }
                    CapacitySlotKind::Pool => {
                        self.diags.push(Diag::ty(
                            slot.span,
                            format!(
                                "@form(vec) requires a `heap` slot; got `pool {} \
                                 of ...`. Vec is the contiguous-growable shape; \
                                 `pool` is the unordered free-list shape — they're \
                                 different storage disciplines.",
                                slot.name.name
                            ),
                        ));
                    }
                }
                if slot.as_parent_for.is_some() {
                    self.diags.push(Diag::ty(
                        slot.span,
                        "@form(vec) slot cannot also be an `as_parent_for` \
                         override; form-lowered slots own their own allocator"
                            .to_string(),
                    ));
                }
            }
            n => {
                self.diags.push(Diag::ty(
                    cb.span,
                    format!(
                        "@form(vec) requires exactly one `heap` capacity slot; \
                         found {} slots. Vec is a single contiguous buffer.",
                        n
                    ),
                ));
            }
        }
    }

    /// M3 stage 4 (2026-07-02): every `expose` entry must bind
    /// against something real on the declaring locus — a params
    /// field, a mode (bulk/harmonic/resolution, per the
    /// mode-pull rule in semantics.md), or a `fn` member — with a
    /// type matching what's declared there (v0 equality via
    /// assignable_from, which stays permissive on Unknown).
    /// Without this, `expose value: String;` over an Int field
    /// compiles, and a parent consuming `value: String` checks
    /// clean against fiction.
    fn check_contract_expose_validity(&mut self, locus: &LocusInfo) {
        for entry in &locus.contract_expose {
            // 1. params field
            if let Some(p) =
                locus.params.iter().find(|p| p.name == entry.name)
            {
                if !entry.ty.assignable_from(&p.ty) {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "contract: locus `{}` exposes `{}: {}`, but the \
                             field is declared `{}`",
                            locus.name,
                            entry.name,
                            entry.ty.display(),
                            p.ty.display()
                        ),
                    ));
                }
                continue;
            }
            // 2. mode (exposed-mode pull: `expose bulk: T;`)
            let mode = match entry.name.as_str() {
                "bulk" => Some(ModeKind::Bulk),
                "harmonic" => Some(ModeKind::Harmonic),
                "resolution" => Some(ModeKind::Resolution),
                _ => None,
            };
            if let Some(mk) = mode {
                match locus.mode_returns.get(&mk) {
                    Some(ret) => {
                        if !entry.ty.assignable_from(ret) {
                            self.diags.push(Diag::ty(
                                entry.span,
                                format!(
                                    "contract: locus `{}` exposes `{}: {}`, \
                                     but the mode returns `{}`",
                                    locus.name,
                                    entry.name,
                                    entry.ty.display(),
                                    ret.display()
                                ),
                            ));
                        }
                    }
                    None => {
                        self.diags.push(Diag::ty(
                            entry.span,
                            format!(
                                "contract: locus `{}` exposes mode `{}` but \
                                 does not declare it",
                                locus.name, entry.name
                            ),
                        ));
                    }
                }
                continue;
            }
            // 3. fn member (vertical method surface)
            if let Some(m) =
                locus.methods.iter().find(|m| m.name == entry.name)
            {
                if !entry.ty.assignable_from(&m.ret) {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "contract: locus `{}` exposes `{}: {}`, but the \
                             method returns `{}`",
                            locus.name,
                            entry.name,
                            entry.ty.display(),
                            m.ret.display()
                        ),
                    ));
                }
                continue;
            }
            self.diags.push(Diag::ty(
                entry.span,
                format!(
                    "contract: locus `{}` exposes `{}` but has no field, \
                     mode, or method with that name",
                    locus.name, entry.name
                ),
            ));
        }
    }

    fn check_contract_compatibility(&mut self, parent: &LocusInfo) {
        let child_name = match &parent.accept_param {
            Some((_, Ty::Named(n))) => n.clone(),
            Some((_, _)) => return, // non-named child type → can't statically resolve
            None => {
                // Parent declares consume but doesn't accept any
                // child. Static error per F.8 — the consume
                // surface has nothing to bind against.
                for entry in &parent.contract_consume {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "locus `{}`: contract consumes `{}` but declares no \
                             `accept(_: ChildType)` to bind against",
                            parent.name, entry.name
                        ),
                    ));
                }
                return;
            }
        };
        let child = match self.top.lookup(&child_name) {
            Some(TopSymbol::Locus(c)) => c,
            _ => return, // unresolved child type — separate error already raised
        };
        for need in &parent.contract_consume {
            match child
                .contract_expose
                .iter()
                .find(|e| e.name == need.name)
            {
                Some(have) => {
                    if !need.ty.assignable_from(&have.ty) {
                        self.diags.push(Diag::ty(
                            need.span,
                            format!(
                                "contract: locus `{}` consumes `{}: {}`, but child \
                                 locus `{}` exposes it as `{}`",
                                parent.name,
                                need.name,
                                need.ty.display(),
                                child.name,
                                have.ty.display()
                            ),
                        ));
                    }
                }
                None => {
                    self.diags.push(Diag::ty(
                        need.span,
                        format!(
                            "contract: locus `{}` consumes `{}` but child locus \
                             `{}` does not expose it",
                            parent.name, need.name, child.name
                        ),
                    ));
                }
            }
        }
    }

    fn check_locus_member(&mut self, member: &'a LocusMember) {
        match member {
            LocusMember::Params(_) | LocusMember::Contract(_) | LocusMember::Bus(_) => {
                // Already lowered by resolver; param defaults are
                // checked against declared types implicitly when
                // the param is referenced. (Milestone-2 cut: no
                // default-vs-declared-type re-check here.)
            }
            LocusMember::Bindings(_) => {
                // Bindings are checked by a separate top-level pass
                // (validate_bindings); nothing to do here.
            }
            LocusMember::Placement(_) => {
                // F.31: placement entries are validated by a
                // dedicated top-level pass (Phase 2 — pending).
                // The parser already enforces "main-only" so the
                // block's syntactic shape is OK here.
            }
            LocusMember::Topology(_) => {
                // Topology Phase 1b: the `topology { }` block is
                // validated by the dedicated placement pass
                // (check_placement_block), alongside the
                // `pinned(node =)` / `pinned(l3 =)` references
                // that resolve against it. Nothing member-local.
            }
            LocusMember::Lifecycle(lc) => {
                self.in_lifecycle = true;
                self.locals.push();
                for p in &lc.params {
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&lc.body);
                self.locals.pop();
                self.in_lifecycle = false;
            }
            LocusMember::Mode(md) => {
                self.in_lifecycle = true;
                self.locals.push();
                for p in &md.params {
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&md.body);
                self.locals.pop();
                self.in_lifecycle = false;
            }
            LocusMember::Failure(fd) => {
                self.in_lifecycle = true;
                self.in_on_failure = true;
                self.locals.push();
                for p in &fd.params {
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&fd.body);
                self.locals.pop();
                self.in_on_failure = false;
                self.in_lifecycle = false;
            }
            LocusMember::Closure(cd) => {
                self.in_closure = true;
                self.in_lifecycle = true;
                // v1.x-VIOLATE (F.27): structural rules on the
                // closure declaration itself.
                let is_inline = cd.clauses.iter().any(|c| {
                    matches!(c, ClosureClause::Epoch(EpochSpec::Inline))
                });
                let captures: Vec<&Ident> = cd
                    .clauses
                    .iter()
                    .flat_map(|c| match c {
                        ClosureClause::Captures(names) => names.iter().collect(),
                        _ => Vec::new(),
                    })
                    .collect();
                // 1. Assertion-presence must match epoch shape.
                //    - `epoch inline`: assertion MUST be absent
                //      (inline fires only via `violate`; an
                //      assertion that never fires is dead).
                //    - Any other epoch: assertion MUST be present.
                if is_inline && cd.assertion.is_some() {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: `epoch inline` closures must \
                             omit the assertion (inline closures fire \
                             only via `violate`; the assertion has no \
                             evaluation site)",
                            cd.name.name,
                        ),
                    ));
                }
                if !is_inline && cd.assertion.is_none() {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: missing assertion. Assertion-\
                             less closures require an `epoch inline` \
                             clause (per F.27); otherwise declare the \
                             `LEFT ~~ RIGHT within TOL;` band",
                            cd.name.name,
                        ),
                    ));
                }
                // 2. `captures:` is only meaningful on inline
                //    closures.
                if !captures.is_empty() && !is_inline {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: `captures:` is meaningful only \
                             on `epoch inline` closures (the snapshot \
                             happens at `violate` fire time, which \
                             auto-epoch closures don't reach)",
                            cd.name.name,
                        ),
                    ));
                }
                // 3. Each captured field name must exist on the
                //    locus param/state surface.
                if let Some(locus) = self.current_locus {
                    for f in &captures {
                        if !locus.params.iter().any(|p| p.name == f.name) {
                            self.diags.push(Diag::ty(
                                f.span,
                                format!(
                                    "closure `{}`: `captures:` references \
                                     field `{}`, which is not declared on \
                                     locus `{}`",
                                    cd.name.name, f.name, locus.name,
                                ),
                            ));
                        }
                    }
                }
                // F.34 (v1.x-WINDOWED): `resets_per_epoch(...)` is
                // only meaningful on `epoch duration(N)` closures
                // (the field-zeroing hook fires at duration-boundary,
                // and the other epochs either don't recur — birth /
                // dissolve / inline — or recur too fast to be a
                // useful rate-budget window — tick).
                let is_duration = cd.clauses.iter().any(|c| {
                    matches!(
                        c,
                        ClosureClause::Epoch(EpochSpec::Duration(_))
                    )
                });
                let resets_pe_fields: Vec<&Ident> = cd
                    .clauses
                    .iter()
                    .flat_map(|c| match c {
                        ClosureClause::ResetsPerEpoch(names) => {
                            names.iter().collect()
                        }
                        _ => Vec::new(),
                    })
                    .collect();
                if !resets_pe_fields.is_empty() && !is_duration {
                    self.diags.push(Diag::ty(
                        cd.span,
                        format!(
                            "closure `{}`: `resets_per_epoch(...)` is \
                             meaningful only on `epoch duration(N)` \
                             closures. Other epochs either don't recur \
                             (birth / dissolve / inline) or recur too \
                             fast to be a useful rate-budget window \
                             (tick).",
                            cd.name.name,
                        ),
                    ));
                }
                if let Some(locus) = self.current_locus {
                    for f in &resets_pe_fields {
                        let Some(p) = locus
                            .params
                            .iter()
                            .find(|p| p.name == f.name)
                        else {
                            self.diags.push(Diag::ty(
                                f.span,
                                format!(
                                    "closure `{}`: `resets_per_epoch(...)` \
                                     references field `{}`, which is not \
                                     declared on locus `{}`",
                                    cd.name.name, f.name, locus.name,
                                ),
                            ));
                            continue;
                        };
                        let is_numeric = matches!(
                            &p.ty,
                            Ty::Prim(PrimType::Int)
                                | Ty::Prim(PrimType::Uint)
                                | Ty::Prim(PrimType::Float)
                                | Ty::Prim(PrimType::Decimal)
                        );
                        if !is_numeric {
                            self.diags.push(Diag::ty(
                                f.span,
                                format!(
                                    "closure `{}`: `resets_per_epoch(...)` \
                                     field `{}` has non-numeric type `{}`. \
                                     The reset hook zeros the field, which \
                                     only makes sense for Int / Uint / \
                                     Float / Decimal counters.",
                                    cd.name.name,
                                    f.name,
                                    p.ty.display(),
                                ),
                            ));
                        }
                    }
                }
                // Original assertion checks for assertion-bearing
                // closures.
                if let Some(assertion) = &cd.assertion {
                    let lt = self.check_expr(&assertion.left);
                    let rt = self.check_expr(&assertion.right);
                    if !lt.assignable_from(&rt) && !rt.assignable_from(&lt) {
                        self.diags.push(Diag::ty(
                            assertion.span,
                            format!(
                                "closure `{}`: assertion sides have incompatible types \
                                 `{}` and `{}`",
                                cd.name.name,
                                lt.display(),
                                rt.display()
                            ),
                        ));
                    }
                    if is_pure_literal(&assertion.left)
                        && is_pure_literal(&assertion.right)
                    {
                        self.diags.push(Diag::ty(
                            assertion.span,
                            format!(
                                "closure `{}`: both assertion sides are pure literals; \
                                 a closure must observe at least one runtime-varying \
                                 value (e.g. `self.x`) to audit anything",
                                cd.name.name
                            ),
                        ));
                    }
                    let _ = self.check_expr(&assertion.tolerance);
                }
                self.in_lifecycle = false;
                self.in_closure = false;
            }
            LocusMember::Fn(f) => {
                // Open-question #24 MVP (2026-05-25): user-
                // declared locus member fns may now carry
                // `fallible(E)` (the value-level error channel).
                // Substrate-facing surfaces still can't —
                // lifecycle methods (Lifecycle decls), mode
                // methods (Mode decls), and bus-subscribed
                // handlers stay non-fallible because the
                // substrate orchestrates them and has no caller
                // frame to address a value return. Lifecycle /
                // Mode are physically incapable (their AST
                // structs don't carry a `fallible` field); the
                // bus-subscribed check lives at the subscribe-
                // site loop above (search for "fallible-handler
                // check").
                //
                // Closure assertions can't *call* fallible
                // member fns inside the assertion expression —
                // `or <disposition>` is statement-position and
                // doesn't compose inside expression-shaped
                // assertion bodies; factor the value-error path
                // into a separate fn and have the closure assert
                // over pre-computed locus state instead. Not
                // checked here for v0.1 — the assertion grammar
                // already rejects most call shapes.
                self.in_lifecycle = true;
                self.check_fn(f, self.current_locus);
                self.in_lifecycle = false;
            }
            LocusMember::Const(c) => {
                let want = resolve_type_expr(&c.ty, self.known);
                let got = self.check_expr(&c.value);
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        c.value.span(),
                        format!(
                            "const `{}`: expected `{}`, got `{}`",
                            c.name.name,
                            want.display(),
                            got.display()
                        ),
                    ));
                }
            }
            LocusMember::Type(_) => {}
            LocusMember::Capacity(cb) => {
                // F.22 restriction 1: cell type must be a value-shape,
                // not a LocusRef. Loci have lifecycle; recycling
                // (Pool.release) or individual free (Heap.free) would
                // orphan the locus. The spec routes locus-membership
                // through `accept(c: SomeL)`; slots are for types.
                let mut seen: BTreeMap<String, Span> = BTreeMap::new();
                for slot in &cb.slots {
                    if let Some(prev) = seen.insert(
                        slot.name.name.clone(),
                        slot.name.span,
                    ) {
                        self.diags.push(Diag::ty(
                            slot.name.span,
                            format!(
                                "duplicate capacity slot name `{}` \
                                 (first declared at {:?})",
                                slot.name.name, prev
                            ),
                        ));
                    }
                    let elem_ty = resolve_type_expr(&slot.elem_ty, self.known);
                    let kind_word = match slot.kind {
                        CapacitySlotKind::Pool => "pool",
                        CapacitySlotKind::Heap => "heap",
                    };
                    if let Ty::Named(n) = &elem_ty {
                        if matches!(
                            self.top.symbols.get(n),
                            Some(TopSymbol::Locus(_))
                        ) {
                            self.diags.push(Diag::ty(
                                slot.span,
                                format!(
                                    "capacity slot `{} {} of {}`: cell \
                                     type cannot be a locus. Cells are \
                                     data; loci are managed entities. \
                                     Locus recycling/free would orphan \
                                     the locus's lifecycle. Route locus \
                                     membership through `accept(c: {})` \
                                     instead, and pair with a parallel \
                                     index slot (e.g. `@form(hashmap)` \
                                     keyed by name) if name-based lookup \
                                     is needed. See spec/semantics.md § \
                                     Locus method dispatch and spec/forms.md \
                                     § Cell type restrictions.",
                                    kind_word, slot.name.name, n, n
                                ),
                            ));
                        }
                    }
                    // F.22 v1.x-4: `as_parent_for ChildL` clause —
                    // validate that ChildL exists, is a locus, and
                    // has a slot with matching name/kind/elem_ty.
                    // The mechanic (handing the parent's allocator
                    // to the child at accept-time) is the v1.x-4b
                    // runtime followup; this pass just gates the
                    // surface so a malformed override fails at
                    // typecheck.
                    if let Some(child_ident) = &slot.as_parent_for {
                        let child_name = &child_ident.name;
                        match self.top.symbols.get(child_name) {
                            Some(TopSymbol::Locus(child_info)) => {
                                if !child_info
                                    .capacity_slot_names
                                    .iter()
                                    .any(|n| n == &slot.name.name)
                                {
                                    self.diags.push(Diag::ty(
                                        child_ident.span,
                                        format!(
                                            "capacity slot `{} {}` declared \
                                             `as_parent_for {}`, but `{}` \
                                             has no slot named `{}` — \
                                             override needs a matching \
                                             slot on the child",
                                            kind_word,
                                            slot.name.name,
                                            child_name,
                                            child_name,
                                            slot.name.name
                                        ),
                                    ));
                                }
                                // TODO v1.x-4b: also verify
                                // kind + elem_ty match — needs
                                // capacity-slot kind/ty info in
                                // the symbol-level LocusInfo.
                            }
                            Some(_) => {
                                self.diags.push(Diag::ty(
                                    child_ident.span,
                                    format!(
                                        "`as_parent_for {}`: `{}` is not \
                                         a locus",
                                        child_name, child_name
                                    ),
                                ));
                            }
                            None => {
                                self.diags.push(Diag::ty(
                                    child_ident.span,
                                    format!(
                                        "`as_parent_for {}`: locus `{}` \
                                         not declared",
                                        child_name, child_name
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
            LocusMember::BirthCheck(bc) => {
                // F.27 v2: validate that the cond is a Bool expr,
                // and that the referenced closure exists on the
                // enclosing locus and is epoch-inline (same rules
                // as a regular `violate NAME;`). Payload type
                // matching against captures is deferred to a
                // follow-up phase (same as the regular violate
                // checker — see Stmt::Violate handler).
                self.locals.push();
                let cond_ty = self.check_expr(&bc.cond);
                if cond_ty != Ty::Prim(PrimType::Bool) {
                    self.diags.push(Diag::ty(
                        bc.span,
                        format!(
                            "birth_check cond must be Bool, got {}",
                            cond_ty.display()
                        ),
                    ));
                }
                if let Some(payload) = &bc.payload {
                    let _ = self.check_expr(payload);
                }
                self.locals.pop();
                match self.current_locus {
                    None => {
                        self.diags.push(Diag::ty(
                            bc.span,
                            "birth_check used outside a locus context"
                                .to_string(),
                        ));
                    }
                    Some(locus) => {
                        match locus
                            .closures
                            .iter()
                            .find(|c| c.name == bc.closure_name.name)
                        {
                            None => {
                                self.diags.push(Diag::ty(
                                    bc.closure_name.span,
                                    format!(
                                        "birth_check: locus `{}` has no \
                                         closure named `{}`",
                                        locus.name, bc.closure_name.name
                                    ),
                                ));
                            }
                            Some(c) if !c.is_inline => {
                                self.diags.push(Diag::ty(
                                    bc.closure_name.span,
                                    format!(
                                        "birth_check `{}`: closure `{}` on \
                                         locus `{}` is not declared \
                                         `epoch inline`. Only epoch-inline \
                                         closures can be fired via \
                                         birth_check (same rule as `violate`)",
                                        bc.closure_name.name,
                                        bc.closure_name.name,
                                        locus.name
                                    ),
                                ));
                            }
                            Some(_) => {
                                // Closure exists and is epoch-inline.
                            }
                        }
                    }
                }
            }
        }
    }

    fn check_fn(&mut self, decl: &'a FnDecl, locus: Option<&'a LocusInfo>) {
        let prev_locus = self.current_locus;
        if locus.is_some() {
            self.current_locus = locus;
        }
        // Stage-1 FFI (2026-05-22): @ffi fn declarations validate
        // their parameter and return types against the FFI-portable
        // type set, then skip body verification (the body is a
        // synthesized empty Block, per parse_fn_decl_with_ffi).
        // Locus context is forbidden — @ffi only valid on top-level
        // free fns at Stage 1; the parser dispatch in
        // parse_top_decl enforces this, but defend in depth here.
        if let Some(ffi) = &decl.ffi {
            if locus.is_some() {
                self.diags.push(Diag::ty(
                    ffi.span,
                    "`@ffi` is only valid on top-level free fns at Stage 1, \
                     not on locus methods",
                ));
            }
            for p in &decl.params {
                let ty = resolve_type_expr(&p.ty, self.known);
                if let Some(reason) = ffi_type_unportable(&ty) {
                    self.diags.push(Diag::ty(
                        p.ty.span(),
                        format!(
                            "`@ffi` fn `{}` parameter `{}` has type {} — {}",
                            decl.name.name,
                            p.name.name,
                            ty.display(),
                            reason,
                        ),
                    ));
                }
            }
            if let Some(ret_te) = &decl.ret {
                let ret_ty = resolve_type_expr(ret_te, self.known);
                if let Some(reason) = ffi_type_unportable(&ret_ty) {
                    self.diags.push(Diag::ty(
                        ret_te.span(),
                        format!(
                            "`@ffi` fn `{}` return type {} — {}",
                            decl.name.name,
                            ret_ty.display(),
                            reason,
                        ),
                    ));
                }
            }
            self.current_locus = prev_locus;
            return;
        }
        // v1.x-FORM-1: push fallible_ctx if this fn is fallible.
        let prev_fallible = self.fallible_ctx.take();
        if let Some(payload_te) = &decl.fallible {
            let success_ret = match &decl.ret {
                Some(te) => resolve_type_expr(te, self.known),
                None => Ty::Unit,
            };
            let payload = resolve_type_expr(payload_te, self.known);
            self.fallible_ctx = Some((success_ret, payload));
        }
        self.locals.push();
        for p in &decl.params {
            let ty = resolve_type_expr(&p.ty, self.known);
            self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
        }
        self.check_block(&decl.body);
        self.locals.pop();
        self.fallible_ctx = prev_fallible;
        self.current_locus = prev_locus;
    }

    fn check_block(&mut self, block: &Block) {
        self.locals.push();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            let _ = self.check_expr(tail);
        }
        self.locals.pop();
    }

    /// Block-as-expression typecheck: walks stmts then returns the
    /// trailing expression's type. Returns `Ty::Unit` if the block
    /// has no trailing expression (caller decides whether that's an
    /// error — for if-expression arms it is).
    fn check_block_as_expr(&mut self, block: &Block) -> Ty {
        self.locals.push();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        let ty = match &block.tail {
            Some(tail) => self.check_expr(tail),
            None => Ty::Unit,
        };
        self.locals.pop();
        ty
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { is_mut, name, ty, value, .. } => {
                let got = self.check_expr_addressed(value);
                let bound = match ty {
                    Some(te) => {
                        let want = resolve_type_expr(te, self.known);
                        if !want.assignable_from(&got) {
                            self.diags.push(Diag::ty(
                                value.span(),
                                format!(
                                    "let `{}`: expected `{}`, got `{}`",
                                    name.name,
                                    want.display(),
                                    got.display()
                                ),
                            ));
                        }
                        want
                    }
                    None => got,
                };
                self.locals.insert(
                    &name.name,
                    LocalSym { ty: bound, is_mut: *is_mut },
                );
            }
            Stmt::LetTuple { is_mut, names, ty, value, .. } => {
                let got = self.check_expr_addressed(value);
                let elem_tys: Vec<Ty> = match (&got, ty) {
                    (Ty::Tuple(parts), _) if parts.len() == names.len() => {
                        parts.clone()
                    }
                    (Ty::Tuple(parts), _) => {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "let-tuple: expected {} elements, got `{}`",
                                names.len(),
                                got.display()
                            ),
                        ));
                        // Best-effort: pad / truncate so subsequent
                        // typechecking can still proceed.
                        let mut v = parts.clone();
                        v.resize(names.len(), Ty::Unknown);
                        v
                    }
                    (other, _) => {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "let-tuple: rhs is `{}`, not a tuple",
                                other.display()
                            ),
                        ));
                        vec![Ty::Unknown; names.len()]
                    }
                };
                for (n, t) in names.iter().zip(elem_tys.iter()) {
                    self.locals.insert(
                        &n.name,
                        LocalSym { ty: t.clone(), is_mut: *is_mut },
                    );
                }
            }
            Stmt::Assign { target, value, span, .. } => {
                let got = self.check_expr_addressed(value);
                let want = self.lvalue_ty(target);
                // bounded[T; N] fields cannot be whole-assigned
                // (even from another bounded of the same shape —
                // no copy semantics exist; the mutation surface is
                // push/clear).
                if matches!(want, Ty::Bounded(_, _)) {
                    self.diags.push(Diag::ty(
                        *span,
                        "bounded[T; N] fields cannot be assigned as a \
                         whole — mutate through push(...) / clear(...)"
                            .to_string(),
                    ));
                }
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        value.span(),
                        format!(
                            "assignment: target type `{}` not assignable from `{}`",
                            want.display(),
                            got.display()
                        ),
                    ));
                }
                // m50: bare-head reassignment to a non-mut local is
                // a compile-time error per spec/types.md "Mutability"
                // + design-rationale §E. Field/index segments
                // (`x.field = ...`, `x[i] = ...`) don't rebind the
                // local — they mutate state through it — so they
                // stay allowed even when the head binding is
                // immutable. `self.field = ...` is also allowed
                // because `self` is locus state, not a binding.
                if target.tail.is_empty() && target.head.name != "self" {
                    if let Some(sym) = self.locals.lookup(&target.head.name) {
                        if !sym.is_mut {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "cannot assign to `{}`: binding is \
                                     immutable. Declare with `let mut {}` \
                                     to permit reassignment.",
                                    target.head.name, target.head.name
                                ),
                            ));
                        }
                    }
                }
            }
            Stmt::Send { subject, value, span, or_disposition } => {
                self.check_send(subject, value, or_disposition.as_ref(), *span);
            }
            Stmt::If(if_stmt) => self.check_if(if_stmt),
            Stmt::Match(m) => self.check_match(m),
            Stmt::For { name, iter, body, .. } => {
                // 2026-07-02 @form iteration surface: `for e in
                // m.entries` (hashmap) / `for x in v.items` (vec).
                // `entries`/`items` are pseudo-fields the generic
                // field check would reject — when the receiver is a
                // locus, check only the receiver and bind the loop
                // var Unknown (codegen resolves the cell type and
                // rejects non-form receivers with a focused error).
                let mut handled = false;
                if let Expr::Field { receiver, name: fname, .. } = iter {
                    if fname.name == "entries" || fname.name == "items" {
                        let recv_ty = self.check_expr(receiver);
                        if let Ty::Named(ln) = &recv_ty {
                            if matches!(
                                self.top.lookup(ln),
                                Some(TopSymbol::Locus(_))
                            ) {
                                handled = true;
                            }
                        }
                    }
                }
                if !handled {
                    let _ = self.check_expr(iter);
                }
                self.locals.push();
                self.locals.insert(&name.name, LocalSym { ty: Ty::Unknown, is_mut: false });
                self.check_block(body);
                self.locals.pop();
            }
            Stmt::While { cond, body, .. } => {
                let ct = self.check_expr(cond);
                if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
                    self.diags.push(Diag::ty(
                        cond.span(),
                        format!(
                            "while condition must be Bool; got `{}`",
                            ct.display()
                        ),
                    ));
                }
                self.check_block(body);
            }
            Stmt::Return(expr, _) => {
                if let Some(e) = expr {
                    let got = self.check_expr_addressed(e);
                    // v1.x-FORM-1: returning from a fallible fn
                    // means returning the success value; payload
                    // type is checked at `fail` sites instead.
                    // Check that the returned type matches the fn's
                    // declared success return type when in a
                    // fallible body.
                    if let Some((expected_ret, _)) = &self.fallible_ctx {
                        if !expected_ret.assignable_from(&got) {
                            self.diags.push(Diag::ty(
                                e.span(),
                                format!(
                                    "return: expected `{}`, got `{}`",
                                    expected_ret.display(),
                                    got.display()
                                ),
                            ));
                        }
                    }
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) => {}
            Stmt::Terminate(span) => {
                // `terminate;` ends the *current locus's* own
                // lifecycle (the locus analogue of `return`), so it
                // only has meaning inside a locus method body —
                // there must be a `self` whose lifecycle to end. In
                // a free function there is no locus to terminate;
                // previously this fell through to a codegen
                // "no self" error with no source location. Gate it
                // here with a focused diagnostic.
                if self.current_locus.is_none() {
                    self.diags.push(Diag::ty(
                        *span,
                        "`terminate` is only valid inside a locus method \
                         — it ends the enclosing locus's own lifecycle, so \
                         there is nothing to terminate in a free function"
                            .to_string(),
                    ));
                }
            }
            Stmt::Fail { value, span } => {
                // v1.x-FORM-1: `fail <expr>;` must appear inside
                // a fallible fn body, and its payload type must
                // match the fn's declared fallible(T) payload.
                // The parser already gates statement-position
                // recognition on the in-fallible-body flag, but
                // we re-check at typecheck for completeness and
                // to produce a clear diagnostic if a Fail node
                // is constructed by other means (interpreter
                // synth, future macro, etc.).
                let payload_ty = self.check_expr_addressed(value);
                match &self.fallible_ctx {
                    None => self.diags.push(Diag::ty(
                        *span,
                        "fail: `fail <expr>;` is only valid inside a \
                         fallible fn body (declared with `fallible(T)`)"
                            .to_string(),
                    )),
                    Some((_, expected_payload)) => {
                        if !expected_payload.assignable_from(&payload_ty) {
                            self.diags.push(Diag::ty(
                                value.span(),
                                format!(
                                    "fail: expected payload type `{}`, got `{}`",
                                    expected_payload.display(),
                                    payload_ty.display()
                                ),
                            ));
                        }
                    }
                }
            }
            Stmt::Block(b) => self.check_block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    let _ = self.check_expr(a);
                }
                if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                    let _ = self.check_expr(e);
                }
            }
            // v1.x-VIOLATE (F.27): rejection-context enforcement
            // + closure-name resolution against the enclosing
            // locus + epoch-inline gate. The parser already
            // accepts `violate NAME [with EXPR];` only at
            // statement positions where it disambiguates from a
            // function call; here we enforce the structural
            // rules from F.27.
            Stmt::Violate { name, payload, span } => {
                if let Some(p) = payload {
                    let _ = self.check_expr(p);
                }
                match self.current_locus {
                    None => {
                        self.diags.push(Diag::ty(
                            *span,
                            format!(
                                "`violate {}`: free fns can't use `violate` \
                                 (no `self` to anchor the closure name). \
                                 Use `fail <payload>;` if this fn is \
                                 declared `fallible(E)`, or move the call \
                                 into a locus method body",
                                name.name,
                            ),
                        ));
                    }
                    Some(locus) if self.in_on_failure => {
                        self.diags.push(Diag::ty(
                            *span,
                            format!(
                                "`violate {}`: not allowed inside an \
                                 `on_failure` body (use `bubble(err)` to \
                                 propagate the child's failure instead — \
                                 `on_failure` is the parent-side handler, \
                                 not a place to fire `{}`'s own closures)",
                                name.name, locus.name,
                            ),
                        ));
                    }
                    Some(locus) => {
                        match locus.closures.iter().find(|c| c.name == name.name) {
                            None => {
                                self.diags.push(Diag::ty(
                                    name.span,
                                    format!(
                                        "`violate {}`: locus `{}` has no \
                                         closure named `{}`",
                                        name.name, locus.name, name.name,
                                    ),
                                ));
                            }
                            Some(c) if !c.is_inline => {
                                self.diags.push(Diag::ty(
                                    name.span,
                                    format!(
                                        "`violate {}`: closure `{}` on locus \
                                         `{}` is not declared `epoch inline`. \
                                         Only assertion-less, inline-epoch \
                                         closures can be fired via `violate`; \
                                         add `epoch inline;` to its clause \
                                         list (or use `bubble(err)` from an \
                                         `on_failure` body instead)",
                                        name.name, name.name, locus.name,
                                    ),
                                ));
                            }
                            Some(_) => {
                                // Closure exists and is epoch-inline.
                                // Payload-type validation against the
                                // closure's captures + `with` shape
                                // lands in phase 4 alongside
                                // ClosureViolation synthesis.
                            }
                        }
                    }
                }
            }
            Stmt::Expr(e) => {
                // M3 stage 2 (2026-07-02): a statement-position
                // `call() or fallback` discards its value, so the
                // fallback's type needn't match the success type —
                // `write_file(p, s) or handler(err);` with a
                // Bool-returning handler is fine (and common in
                // production code). The flag is consumed (and
                // cleared) by the Or/Substitute arm.
                if matches!(e, Expr::Or { .. }) {
                    self.or_value_discarded = true;
                }
                let _ = self.check_expr_addressed(e);
                self.or_value_discarded = false;
            }
            Stmt::ShmWrite { topic, max, binding, body, span } => {
                // The receiver must be a declared topic (its layout-bound
                // producer binding is validated at the binding site).
                if !matches!(
                    self.top.lookup(&topic.name),
                    Some(TopSymbol::Topic(_))
                ) {
                    self.diags.push(Diag::ty(
                        topic.span,
                        format!("`{}.write`: `{}` is not a declared topic",
                                topic.name, topic.name),
                    ));
                }
                let max_ty = self.check_expr_addressed(max);
                if !matches!(max_ty, Ty::Prim(PrimType::Int) | Ty::Unknown) {
                    self.diags.push(Diag::ty(
                        max.span(),
                        format!("`{}.write(max)`: max must be Int, got {}",
                                topic.name, max_ty.display()),
                    ));
                }
                // Bind the writable view over the body, then require the
                // body to end with the Int byte count to commit.
                self.locals.push();
                self.locals.insert(
                    &binding.name,
                    LocalSym { ty: Ty::Unknown, is_mut: false },
                );
                for s in &body.stmts {
                    self.check_stmt(s);
                }
                match &body.tail {
                    Some(t) => {
                        let tt = self.check_expr_addressed(t);
                        if !matches!(tt, Ty::Prim(PrimType::Int) | Ty::Unknown) {
                            self.diags.push(Diag::ty(
                                t.span(),
                                format!(
                                    "`{}.write {{ ... }}` must end with the Int \
                                     byte count to commit, got {}",
                                    topic.name, tt.display()
                                ),
                            ));
                        }
                    }
                    None => self.diags.push(Diag::ty(
                        *span,
                        format!(
                            "`{}.write {{ ... }}` must end with the Int byte \
                             count to commit (the record length)",
                            topic.name
                        ),
                    )),
                }
                self.locals.pop();
            }
        }
    }

    fn check_if(&mut self, stmt: &IfStmt) {
        let ct = self.check_expr(&stmt.cond);
        if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
            self.diags.push(Diag::ty(
                stmt.cond.span(),
                format!("if condition must be Bool; got `{}`", ct.display()),
            ));
        }
        self.check_block(&stmt.then_block);
        if let Some(else_branch) = &stmt.else_block {
            match else_branch.as_ref() {
                ElseBranch::Else(b) => self.check_block(b),
                ElseBranch::ElseIf(s) => self.check_if(s),
            }
        }
    }

    /// If-as-expression: cond checked as Bool; then/else arms checked
    /// as block-expressions; the result type is the unified arm type.
    /// Returns `Ty::Unknown` if arms disagree (with a diagnostic).
    fn check_if_as_expr(&mut self, stmt: &IfStmt) -> Ty {
        let ct = self.check_expr(&stmt.cond);
        if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
            self.diags.push(Diag::ty(
                stmt.cond.span(),
                format!("if condition must be Bool; got `{}`", ct.display()),
            ));
        }
        let then_ty = self.check_block_as_expr(&stmt.then_block);
        let else_ty = match &stmt.else_block {
            Some(b) => match b.as_ref() {
                ElseBranch::Else(blk) => self.check_block_as_expr(blk),
                ElseBranch::ElseIf(nested) => self.check_if_as_expr(nested),
            },
            None => Ty::Unit,
        };
        if then_ty.display() != else_ty.display()
            && !then_ty.assignable_from(&else_ty)
            && !else_ty.assignable_from(&then_ty)
        {
            self.diags.push(Diag::ty(
                stmt.span,
                format!(
                    "if-expression arms have mismatched types: \
                     then=`{}`, else=`{}`",
                    then_ty.display(),
                    else_ty.display()
                ),
            ));
            return Ty::Unknown;
        }
        then_ty
    }

    fn check_match(&mut self, stmt: &MatchStmt) {
        let scrut_ty = self.check_expr(&stmt.scrutinee);
        for arm in &stmt.arms {
            if let Some(g) = &arm.guard {
                let _ = self.check_expr(g);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => {
                    let _ = self.check_expr(e);
                }
                MatchArmBody::Block(b) => self.check_block(b),
            }
        }
        if !match_is_exhaustive(&scrut_ty, &stmt.arms, self.top) {
            self.diags.push(Diag::ty(
                stmt.span,
                format!(
                    "match is not exhaustive; add a `_` arm or cover all \
                     cases of `{}`",
                    scrut_ty.display()
                ),
            ));
        }
    }

    fn check_send(
        &mut self,
        subject: &Expr,
        value: &Expr,
        or_disposition: Option<&OrDisposition>,
        span: Span,
    ) {
        let payload_ty = self.check_expr(value);
        // Phase 3 routing keys (2026-05-25): the `or DISPOSITION`
        // clause on Send is legal only when the target topic
        // declares `on_unmatched: fail`. Conversely, fail topics
        // REQUIRE the clause — a fail-policy publish without an
        // or-disposition leaves the no-match err unhandled. We
        // resolve the topic by name/literal-subject and validate
        // both directions.
        let target_policy = match subject {
            Expr::Literal(Literal::String(s), _) => self
                .top
                .symbols
                .values()
                .find_map(|sym| match sym {
                    TopSymbol::Topic(ti)
                        if ti.subject == *s
                            || ti.wire_subject == *s
                            || ti.name == *s =>
                    {
                        Some(ti.on_unmatched)
                    }
                    _ => None,
                }),
            Expr::Ident(id) => match self.top.lookup(&id.name) {
                Some(TopSymbol::Topic(ti)) => Some(ti.on_unmatched),
                _ => None,
            },
            _ => None,
        };
        match (target_policy.flatten(), or_disposition) {
            (Some(UnmatchedPolicy::Fail), None) => {
                self.diags.push(Diag::ty(
                    span,
                    "publish to topic with `on_unmatched: fail` must \
                     carry an `or` disposition — e.g. \
                     `Subject <- value or raise`",
                ));
            }
            (Some(UnmatchedPolicy::Fail), Some(disp)) => {
                // v0.2 (2026-05-26): all four dispositions wired.
                //   - Raise / Discard: as v0.1 — no err-payload
                //     needed, codegen panics or no-ops.
                //   - Substitute: `err: BusUnmatchedKey` in scope
                //     on the RHS; expression evaluated for side
                //     effects (Send is a statement, no value
                //     binding to type-match).
                //   - Fail: `err: BusUnmatchedKey` in scope on
                //     the payload expr; payload type must match
                //     the enclosing fallible fn's declared err.
                match disp {
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
                    OrDisposition::Substitute(rhs) => {
                        let err_ty =
                            Ty::Named("BusUnmatchedKey".to_string());
                        self.locals.push();
                        self.locals.insert(
                            "err",
                            LocalSym {
                                ty: err_ty,
                                is_mut: false,
                            },
                        );
                        let _ = self.check_expr(rhs);
                        self.locals.pop();
                    }
                    OrDisposition::Fail(payload_expr, sp) => {
                        let err_ty =
                            Ty::Named("BusUnmatchedKey".to_string());
                        self.locals.push();
                        self.locals.insert(
                            "err",
                            LocalSym {
                                ty: err_ty,
                                is_mut: false,
                            },
                        );
                        let new_payload_ty =
                            self.check_expr_addressed(payload_expr);
                        self.locals.pop();
                        match &self.fallible_ctx {
                            None => self.diags.push(Diag::ty(
                                *sp,
                                "`or fail X`: only valid inside a \
                                 fallible fn body (declared with \
                                 `fallible(T)`). Use `or raise` to \
                                 propagate the no-match as a panic, \
                                 or `or <expr>` to side-effect a \
                                 handler.",
                            )),
                            Some((_, expected_payload)) => {
                                if !expected_payload
                                    .assignable_from(&new_payload_ty)
                                {
                                    self.diags.push(Diag::ty(
                                        payload_expr.span(),
                                        format!(
                                            "`or fail`: expected \
                                             payload type `{}`, got \
                                             `{}`",
                                            expected_payload.display(),
                                            new_payload_ty.display()
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            (_, Some(_)) => {
                self.diags.push(Diag::ty(
                    span,
                    "`or` disposition on a bus send is only legal \
                     when the target topic declares \
                     `on_unmatched: fail`",
                ));
            }
            (_, None) => {
                // Default path — unkeyed / swallow / fallback.
            }
        }
        // Subject extraction. Two static forms produce a fixed
        // wire-format subject string: a literal `"S" <- expr` and
        // a topic-ref `Foo <- expr` where Foo names a `topic`
        // decl. Anything else is a computed subject and goes
        // through the wildcard-publish path further below.
        let subject_str = match subject {
            Expr::Literal(Literal::String(s), _) => Some(s.clone()),
            Expr::Ident(id) => match self.top.lookup(&id.name) {
                Some(TopSymbol::Topic(_)) => Some(id.name.clone()),
                _ => None,
            },
            // A7 (G16): cross-seed `alias::Topic <- payload;`. The
            // typechecker can't resolve cross-seed names directly
            // (mangling happens at the codegen-side pre-pass), so
            // we use the leaf segment as the subject — mirroring
            // resolve_bus_subject's handling of QualifiedTopic in
            // subscribe/publish declarations, which also stores the
            // leaf name. The locus's bus_publishes entry for this
            // topic has payload=Unknown, so the assignability check
            // below is permissive; the codegen-side mangle resolves
            // the full path and binds the wire subject.
            Expr::Path(qn) if qn.segments.len() > 1 => {
                qn.segments.last().map(|s| s.name.clone())
            }
            _ => None,
        };
        let locus = match self.current_locus {
            Some(l) => l,
            None => {
                self.diags.push(Diag::ty(
                    span,
                    "bus send (`<-`) only valid inside a locus body".to_string(),
                ));
                return;
            }
        };
        // m94: a non-literal subject is allowed when the locus
        // declares a wildcard `publish` whose payload matches.
        // The wildcard declaration acts as the authorization +
        // type-binding for any concrete subject computed at
        // runtime that matches the pattern. Static subject-pattern
        // verification is impossible by definition; we trust the
        // declaration and let runtime dispatch route to whichever
        // subscribers (exact or wildcard) match.
        let subject_str = match subject_str {
            Some(s) => s,
            None => {
                let wildcard_match = locus.bus_publishes.iter().find(|p| {
                    p.subject.contains("**")
                        && p.payload.assignable_from(&payload_ty)
                });
                if wildcard_match.is_none() {
                    let any_wildcard = locus
                        .bus_publishes
                        .iter()
                        .any(|p| p.subject.contains("**"));
                    if any_wildcard {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "bus send (computed subject): payload `{}` does \
                                 not match any wildcard publish declaration in \
                                 locus `{}`",
                                payload_ty.display(),
                                locus.name
                            ),
                        ));
                    } else {
                        self.diags.push(Diag::ty(
                            subject.span(),
                            format!(
                                "bus send with computed subject requires a \
                                 wildcard `publish` declaration (e.g. \
                                 `publish \"log.**\" of type T`) in locus `{}`",
                                locus.name
                            ),
                        ));
                    }
                }
                return;
            }
        };
        let pub_decl = locus
            .bus_publishes
            .iter()
            .find(|p| p.subject == subject_str);
        match pub_decl {
            Some(decl) => {
                if !decl.payload.assignable_from(&payload_ty) {
                    self.diags.push(Diag::ty(
                        value.span(),
                        format!(
                            "bus send `{}`: payload `{}` not assignable to declared `{}`",
                            subject_str,
                            payload_ty.display(),
                            decl.payload.display()
                        ),
                    ));
                }
            }
            None => {
                // m94: an exact-literal subject is also valid when
                // it matches a wildcard publish declaration of the
                // right type. This lets a locus declare
                // `publish "log.**" of type LogEvent` once and
                // then send on `"log.app"` etc. literally.
                let wildcard_match = locus.bus_publishes.iter().find(|p| {
                    p.subject.contains("**")
                        && super::wildcard_match(&p.subject, &subject_str)
                        && p.payload.assignable_from(&payload_ty)
                });
                if wildcard_match.is_none() {
                    self.diags.push(Diag::ty(
                        subject.span(),
                        format!(
                            "bus send subject `{}` is not declared in locus `{}`'s bus block",
                            subject_str, locus.name
                        ),
                    ));
                }
            }
        }
    }

    fn lvalue_ty(&mut self, lv: &LValue) -> Ty {
        let mut ty = if lv.head.name == "self" {
            self.self_ty()
        } else if let Some(s) = self.locals.lookup(&lv.head.name) {
            s.ty.clone()
        } else {
            Ty::Unknown
        };
        for seg in &lv.tail {
            match seg {
                LValueSeg::Field(f) => {
                    ty = self.field_ty(&ty, &f.name).unwrap_or(Ty::Unknown);
                }
                LValueSeg::Index(idx) => {
                    let _ = self.check_expr(idx);
                    ty = match ty {
                        Ty::Array(elem, _) => *elem,
                        _ => Ty::Unknown,
                    };
                }
            }
        }
        ty
    }

    fn self_ty(&self) -> Ty {
        match self.current_locus {
            Some(l) => Ty::Named(l.name.clone()),
            None => Ty::Unknown,
        }
    }

    /// Look up a named field on a type. Resolves struct fields,
    /// locus params (when accessing a locus handle's exposed
    /// state — milestone 2 just exposes all params), and
    /// perspective params.
    /// Verify that a locus structurally implements an interface:
    /// for every method the interface declares, the locus has a
    /// method with the same name, same arity, compatible param
    /// types, and a compatible return type. Returns Err with a
    /// human-readable message on the first mismatch.
    ///
    /// Both arguments are top-symbol names. Caller has already
    /// verified that `iface_name` resolves to a TopSymbol::Interface.
    /// `locus_name` may be any TopSymbol — non-locus returns Err.
    fn check_structural_impl(
        &self,
        locus_name: &str,
        iface_name: &str,
    ) -> Result<(), String> {
        let iface = match self.top.lookup(iface_name) {
            Some(TopSymbol::Interface(i)) => i,
            _ => return Ok(()),
        };
        let locus = match self.top.lookup(locus_name) {
            Some(TopSymbol::Locus(l)) => l,
            _ => {
                return Err(format!(
                    "type `{}` cannot satisfy interface `{}` — only loci satisfy interfaces",
                    locus_name, iface_name
                ));
            }
        };
        for im in &iface.methods {
            let lm = locus.methods.iter().find(|lm| lm.name == im.name);
            let lm = match lm {
                Some(m) => m,
                None => {
                    return Err(format!(
                        "locus `{}` does not satisfy interface `{}`: missing method `{}`",
                        locus_name, iface_name, im.name
                    ));
                }
            };
            if lm.params.len() != im.params.len() {
                return Err(format!(
                    "locus `{}` method `{}` arity does not match interface `{}`: expected {} arg(s), locus has {}",
                    locus_name,
                    im.name,
                    iface_name,
                    im.params.len(),
                    lm.params.len()
                ));
            }
            for (i, (lp, ip)) in
                lm.params.iter().zip(im.params.iter()).enumerate()
            {
                let want = &ip.1;
                if !want.assignable_from(lp) {
                    return Err(format!(
                        "locus `{}` method `{}` arg #{} type mismatch: interface `{}` requires `{}`, locus has `{}`",
                        locus_name,
                        im.name,
                        i,
                        iface_name,
                        want.display(),
                        lp.display()
                    ));
                }
            }
            if !im.ret.assignable_from(&lm.ret) {
                return Err(format!(
                    "locus `{}` method `{}` return type mismatch: interface `{}` requires `{}`, locus returns `{}`",
                    locus_name,
                    im.name,
                    iface_name,
                    im.ret.display(),
                    lm.ret.display()
                ));
            }
        }
        Ok(())
    }

    fn field_ty(&self, ty: &Ty, name: &str) -> Option<Ty> {
        match ty {
            // Numeric tuple field access: `t.0`, `t.1`. Parser
            // stores the digit string as the field name, so we
            // recognize it as a usize index here.
            Ty::Tuple(parts) => {
                if let Ok(i) = name.parse::<usize>() {
                    if i < parts.len() {
                        return Some(parts[i].clone());
                    }
                }
                None
            }
            Ty::Named(n) => {
                // M3 stage 3 tranche 2: field reads on mangled
                // generic monomorph values (`b.value` where
                // b: Box_Int) resolve through the template with
                // the type args substituted.
                if self.top.lookup(n).is_none() {
                    let (template, bindings) =
                        self.resolve_generic_monomorph(n)?;
                    if let TypeDeclBody::Struct(tfields) =
                        &template.body
                    {
                        return tfields
                            .iter()
                            .find(|f| f.name.name == name)
                            .map(|f| {
                                substitute_generic_ty(
                                    &f.ty,
                                    &bindings,
                                    self.known,
                                )
                            });
                    }
                    return None;
                }
                match self.top.lookup(n)? {
                TopSymbol::Type(info) => match &info.kind {
                    TypeKind::Struct(fields) => fields
                        .iter()
                        .find(|f| f.name == name)
                        .map(|f| f.ty.clone()),
                    TypeKind::Alias(t) => self.field_ty(t, name),
                    TypeKind::Enum(_) => None,
                },
                TopSymbol::Locus(info) => {
                    if name == "children" {
                        return Some(match &info.accept_param {
                            Some((_, t)) => Ty::Array(Box::new(t.clone()), None),
                            None => Ty::Array(Box::new(Ty::Unknown), None),
                        });
                    }
                    if name == "k_max" {
                        // F.1: k_max = B / [(1-phi)c + phi*sigma].
                        // Fractional in general; Float regardless of
                        // whether B/c/sigma are Int (the divisor is
                        // a phi-weighted blend).
                        return Some(Ty::Prim(PrimType::Float));
                    }
                    // v1.x-VIOLATE (F.27): synthetic Bool flag
                    // readable from any locus method body. True
                    // while the locus is winding down after
                    // `violate`; canonical use is to gate
                    // downstream sends after escalation. Backed
                    // by `__drain_requested` at codegen.
                    if name == "draining" {
                        return Some(Ty::Prim(PrimType::Bool));
                    }
                    if let Some(p) = info.params.iter().find(|p| p.name == name) {
                        return Some(p.ty.clone());
                    }
                    info.methods
                        .iter()
                        .find(|m| m.name == name)
                        .map(method_to_fn_ty)
                }
                TopSymbol::Perspective(info) => {
                    if let Some(p) = info.params.iter().find(|p| p.name == name) {
                        return Some(p.ty.clone());
                    }
                    info.methods
                        .iter()
                        .find(|m| m.name == name)
                        .map(method_to_fn_ty)
                }
                // 2026-05-16 — method lookup on an interface-typed
                // receiver. Resolves `obj.handle(req)` when `obj`
                // has interface type, so call-site typecheck sees
                // the method's signature instead of "no field".
                // Codegen already routes the call through the fat
                // pointer's vtable (lower_method_call's
                // CodegenTy::Interface arm).
                TopSymbol::Interface(info) => {
                    info.methods.iter().find(|m| m.name == name).map(|m| {
                        Ty::Function {
                            params: m.params.iter().map(|(_, t)| t.clone()).collect(),
                            ret: Box::new(m.ret.clone()),
                        }
                    })
                }
                _ => None,
            }
            }
            Ty::Unknown => Some(Ty::Unknown),
            _ => None,
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        match expr {
            Expr::Literal(lit, _) => lit_ty(lit),
            Expr::Ident(id) => {
                if let Some(s) = self.locals.lookup(&id.name) {
                    s.ty.clone()
                } else if let Some(sym) = self.top.lookup(&id.name) {
                    match sym {
                        TopSymbol::Const(c) => c.ty.clone(),
                        TopSymbol::Fn(sig) => Ty::Function {
                            params: sig.params.iter().map(|(_, t)| t.clone()).collect(),
                            ret: Box::new(sig.ret.clone()),
                        },
                        // Locus / Type / Perspective / Interface
                        // names used in expression position resolve
                        // to the type (struct-literal, call site,
                        // or interface-typed binding).
                        TopSymbol::Locus(_)
                        | TopSymbol::Type(_)
                        | TopSymbol::Perspective(_)
                        | TopSymbol::Interface(_) => Ty::Named(id.name.clone()),
                        // Topics aren't values — they only address
                        // a bus channel. They appear legally only on
                        // the left of `<-` (handled in check_send,
                        // before check_expr ever sees the subject).
                        // Anywhere else is an error.
                        TopSymbol::Topic(_) => {
                            self.diags.push(Diag::ty(
                                id.span,
                                format!(
                                    "topic `{}` is not a value; use `{} <- expr` \
                                     to publish on it",
                                    id.name, id.name
                                ),
                            ));
                            Ty::Unknown
                        }
                        TopSymbol::RingLayout(_) => {
                            self.diags.push(Diag::ty(
                                id.span,
                                format!(
                                    "ring_layout `{}` is not a value; reference it \
                                     in a `shm_ring(..., layout: {})` binding",
                                    id.name, id.name
                                ),
                            ));
                            Ty::Unknown
                        }
                    }
                } else {
                    Ty::Unknown
                }
            }
            Expr::Path(qn) => {
                // m47-followup: 2-segment path may be an enum
                // variant construction (`EnumName::VariantName`).
                // Resolve to the enum type so let-bindings,
                // tuple/array literals, and struct fields can
                // unify against the declared shape rather than
                // falling through to Unknown (which made `let x:
                // Color = Color::Red;` fail with `expected Color,
                // got ?`).
                if qn.segments.len() == 2 {
                    let enum_name = &qn.segments[0].name;
                    let variant_name = &qn.segments[1].name;
                    if let Some(TopSymbol::Type(TypeInfo {
                        kind: TypeKind::Enum(variants),
                        ..
                    })) = self.top.symbols.get(enum_name)
                    {
                        if variants.iter().any(|v| v.name == *variant_name) {
                            return Ty::Named(enum_name.clone());
                        }
                    }
                }
                Ty::Unknown
            }
            Expr::Path2 { .. } => Ty::Unknown,
            Expr::KwSelf(span) => {
                if self.current_locus.is_none() {
                    self.diags.push(Diag::ty(
                        *span,
                        "`self` used outside a locus body".to_string(),
                    ));
                }
                self.self_ty()
            }
            Expr::Binary { op, left, right, span } => {
                let lt = self.check_expr(left);
                let rt = self.check_expr(right);
                self.binop_ty(*op, &lt, &rt, *span)
            }
            Expr::Unary { op, operand, .. } => {
                let t = self.check_expr(operand);
                match op {
                    UnaryOp::Neg | UnaryOp::BitNot => t,
                    UnaryOp::Not => Ty::Prim(PrimType::Bool),
                }
            }
            Expr::Call { callee, args, .. } => {
                // WASM plan — stdlib target-gating. Under `target wasm`
                // the browser sandbox has no syscalls, so reject a
                // POSIX-only `std::` call at compile time with guidance
                // (rather than letting it become an inert host import).
                // Typecheck runs on USER code before the stdlib is merged
                // in codegen, so this never flags the stdlib's own
                // internals — only the program's calls.
                if self.wasm_target {
                    if let Expr::Path(qn) = callee.as_ref() {
                        let segs: Vec<&str> =
                            qn.segments.iter().map(|s| s.name.as_str()).collect();
                        if let Some(why) = wasm_unavailable_stdlib(&segs) {
                            self.diags.push(Diag::ty(
                                qn.span,
                                format!(
                                    "`std::{}` is unavailable under `target wasm`: {}",
                                    segs[1..].join("::"),
                                    why
                                ),
                            ));
                        }
                    }
                }
                // Typecheck M3 stage 1 (2026-07-02): stdlib fn-name
                // validation. Within a TABLED namespace an unknown
                // name is an error with a did-you-mean; untabled
                // namespaces keep the permissive Unknown behavior,
                // so table incompleteness degrades to the status
                // quo, never to a false error.
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.first().map(|s| s.name.as_str())
                        == Some("std")
                    {
                        let segs: Vec<&str> = qn
                            .segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect();
                        if let Some(msg) =
                            crate::stdlib_surface::unknown_fn_error(&segs)
                        {
                            self.diags.push(Diag::ty(qn.span, msg));
                        }
                        // M3 stage 2 (2026-07-02): signature
                        // enforcement — arity, arg types, and the
                        // REAL return type (killing the Unknown
                        // passthrough for tabled fns). Fallible rows
                        // return Ty::Fallible, so `or 0` on an
                        // Int-success call checks the substitute
                        // against Int and codegen's must-address
                        // rule gets a typecheck twin.
                        if let Some(sig) =
                            crate::stdlib_surface::signature_for(&segs)
                        {
                            // (bounded-intrinsic block is below —
                            // stdlib paths never collide with it.)
                            if args.len() != sig.params.len() {
                                self.diags.push(Diag::ty(
                                    qn.span,
                                    format!(
                                        "`{}` takes {} argument{}, got {}",
                                        sig.display_path(),
                                        sig.params.len(),
                                        if sig.params.len() == 1 {
                                            ""
                                        } else {
                                            "s"
                                        },
                                        args.len()
                                    ),
                                ));
                            }
                            for (i, a) in args.iter().enumerate() {
                                let got = self.check_expr_addressed(a);
                                if let Some(want) = sig.params.get(i) {
                                    if !want.accepts(&got) {
                                        self.diags.push(Diag::ty(
                                            a.span(),
                                            format!(
                                                "`{}` argument {}: expected \
                                                 `{}`, got `{}`",
                                                sig.display_path(),
                                                i + 1,
                                                want.to_ty().display(),
                                                got.display()
                                            ),
                                        ));
                                    }
                                }
                            }
                            return sig.ret_ty();
                        }
                    }
                }
                // bounded[T; N] intrinsics (2026-07-02):
                // push/at/count/clear over a bounded-typed first
                // arg. Probed speculatively — when arg0 isn't
                // bounded, its diags are rolled back and the call
                // falls through to the normal paths.
                if let Expr::Ident(id) = callee.as_ref() {
                    if matches!(
                        id.name.as_str(),
                        "push" | "at" | "set" | "count" | "clear"
                            | "truncate"
                    ) && !args.is_empty()
                    {
                        let mark = self.diags.len();
                        let recv_ty = self.check_expr(&args[0]);
                        if let Ty::Bounded(elem, _cap) = recv_ty {
                            let want_args = match id.name.as_str() {
                                "push" | "at" | "truncate" => 2,
                                "set" => 3,
                                _ => 1,
                            };
                            if args.len() != want_args {
                                self.diags.push(Diag::ty(
                                    callee.span(),
                                    format!(
                                        "`{}(bounded, ...)` takes {} \
                                         argument{}, got {}",
                                        id.name,
                                        want_args,
                                        if want_args == 1 { "" } else { "s" },
                                        args.len()
                                    ),
                                ));
                            }
                            if id.name == "set" {
                                if let Some(i) = args.get(1) {
                                    let it =
                                        self.check_expr_addressed(i);
                                    if !Ty::Prim(PrimType::Int)
                                        .assignable_from(&it)
                                    {
                                        self.diags.push(Diag::ty(
                                            i.span(),
                                            format!(
                                                "set: index must be \
                                                 Int, got `{}`",
                                                it.display()
                                            ),
                                        ));
                                    }
                                }
                                if let Some(x) = args.get(2) {
                                    let xt =
                                        self.check_expr_addressed(x);
                                    let widen_ok = matches!(
                                        (elem.as_ref(), &xt),
                                        (
                                            Ty::Prim(PrimType::Float),
                                            Ty::Prim(PrimType::Int)
                                        )
                                    );
                                    if !widen_ok
                                        && !elem.assignable_from(&xt)
                                    {
                                        self.diags.push(Diag::ty(
                                            x.span(),
                                            format!(
                                                "set: element type `{}` \
                                                 does not match bounded \
                                                 element `{}`",
                                                xt.display(),
                                                elem.display()
                                            ),
                                        ));
                                    }
                                }
                                return Ty::Fallible {
                                    success: Box::new(Ty::Unit),
                                    payload: Box::new(Ty::Named(
                                        "IndexError".into(),
                                    )),
                                };
                            }
                            if id.name == "truncate" {
                                if let Some(n) = args.get(1) {
                                    let nt =
                                        self.check_expr_addressed(n);
                                    if !Ty::Prim(PrimType::Int)
                                        .assignable_from(&nt)
                                    {
                                        self.diags.push(Diag::ty(
                                            n.span(),
                                            format!(
                                                "truncate: n must be \
                                                 Int, got `{}`",
                                                nt.display()
                                            ),
                                        ));
                                    }
                                }
                                return Ty::Prim(PrimType::Int);
                            }
                            match id.name.as_str() {
                                "push" => {
                                    if let Some(x) = args.get(1) {
                                        let xt =
                                            self.check_expr_addressed(x);
                                        let widen_ok = matches!(
                                            (elem.as_ref(), &xt),
                                            (
                                                Ty::Prim(PrimType::Float),
                                                Ty::Prim(PrimType::Int)
                                            )
                                        );
                                        if !widen_ok
                                            && !elem.assignable_from(&xt)
                                        {
                                            self.diags.push(Diag::ty(
                                                x.span(),
                                                format!(
                                                    "push: element type \
                                                     `{}` does not match \
                                                     bounded element `{}`",
                                                    xt.display(),
                                                    elem.display()
                                                ),
                                            ));
                                        }
                                    }
                                    return Ty::Fallible {
                                        success: Box::new(Ty::Unit),
                                        payload: Box::new(Ty::Named(
                                            "CapacityError".into(),
                                        )),
                                    };
                                }
                                "at" => {
                                    if let Some(i) = args.get(1) {
                                        let it =
                                            self.check_expr_addressed(i);
                                        if !Ty::Prim(PrimType::Int)
                                            .assignable_from(&it)
                                        {
                                            self.diags.push(Diag::ty(
                                                i.span(),
                                                format!(
                                                    "at: index must be \
                                                     Int, got `{}`",
                                                    it.display()
                                                ),
                                            ));
                                        }
                                    }
                                    return Ty::Fallible {
                                        success: elem,
                                        payload: Box::new(Ty::Named(
                                            "IndexError".into(),
                                        )),
                                    };
                                }
                                "count" => {
                                    return Ty::Prim(PrimType::Int);
                                }
                                _ => return Ty::Unit,
                            }
                        }
                        // Not bounded: roll back speculative diags.
                        self.diags.truncate(mark);
                    }
                }
                // M3 stage 3 (2026-07-02): generic fn call
                // validation — the Ty-level mirror of codegen's m62
                // inference, with spans. Checks: arity, every
                // generic param pinned, no conflicting bindings,
                // args vs substituted params; the call types as the
                // substituted return (fallible payloads
                // substituted too).
                if let Expr::Ident(id) = callee.as_ref() {
                    if let Some(template) =
                        self.generic_fns.get(id.name.as_str()).copied()
                    {
                        if args.len() != template.params.len() {
                            self.diags.push(Diag::ty(
                                callee.span(),
                                format!(
                                    "generic fn `{}` takes {} \
                                     argument{}, got {}",
                                    id.name,
                                    template.params.len(),
                                    if template.params.len() == 1 {
                                        ""
                                    } else {
                                        "s"
                                    },
                                    args.len()
                                ),
                            ));
                        }
                        let arg_tys: Vec<Ty> = args
                            .iter()
                            .map(|a| self.check_expr_addressed(a))
                            .collect();
                        let generic_names: std::collections::BTreeSet<
                            String,
                        > = template
                            .generics
                            .iter()
                            .map(|g| g.name.name.clone())
                            .collect();
                        let mut bindings: BTreeMap<String, Ty> =
                            BTreeMap::new();
                        for (p, at) in
                            template.params.iter().zip(arg_tys.iter())
                        {
                            if let Err((gname, was, now)) =
                                unify_generic_ty(
                                    &p.ty,
                                    at,
                                    &generic_names,
                                    &mut bindings,
                                )
                            {
                                self.diags.push(Diag::ty(
                                    callee.span(),
                                    format!(
                                        "generic fn `{}`: parameter \
                                         `{}` bound to both `{}` and \
                                         `{}` by this call's arguments",
                                        id.name,
                                        gname,
                                        was.display(),
                                        now.display()
                                    ),
                                ));
                            }
                        }
                        // Every generic must be pinned — UNLESS an
                        // arg typed Unknown could have pinned it
                        // (stay permissive where inference was).
                        let any_unknown_arg = arg_tys
                            .iter()
                            .any(|t| matches!(t, Ty::Unknown));
                        if !any_unknown_arg {
                            for g in &template.generics {
                                if !bindings.contains_key(&g.name.name)
                                {
                                    self.diags.push(Diag::ty(
                                        callee.span(),
                                        format!(
                                            "generic fn `{}`: cannot \
                                             infer `{}` from this call \
                                             — every generic param \
                                             must appear in an \
                                             argument position that \
                                             pins it",
                                            id.name, g.name.name
                                        ),
                                    ));
                                }
                            }
                        }
                        // Args vs substituted params.
                        for ((p, at), a) in template
                            .params
                            .iter()
                            .zip(arg_tys.iter())
                            .zip(args.iter())
                        {
                            let want = substitute_generic_ty(
                                &p.ty,
                                &bindings,
                                self.known,
                            );
                            if !want.assignable_from(at) {
                                self.diags.push(Diag::ty(
                                    a.span(),
                                    format!(
                                        "generic fn `{}` argument \
                                         `{}`: expected `{}`, got `{}`",
                                        id.name,
                                        p.name.name,
                                        want.display(),
                                        at.display()
                                    ),
                                ));
                            }
                        }
                        let ret = match &template.ret {
                            Some(te) => substitute_generic_ty(
                                te,
                                &bindings,
                                self.known,
                            ),
                            None => Ty::Unit,
                        };
                        if let Some(fe) = &template.fallible {
                            let payload = substitute_generic_ty(
                                fe,
                                &bindings,
                                self.known,
                            );
                            return Ty::Fallible {
                                success: Box::new(ret),
                                payload: Box::new(payload),
                            };
                        }
                        return ret;
                    }
                }
                // m47-payloads: enum-variant construction with
                // args. `EnumName::Variant(..)` resolves to the
                // enum's named type. We still walk the args to
                // surface their own type errors, but don't unify
                // them against declared field types yet — codegen
                // performs that strict check, and the typechecker
                // is permissive on Unknowns elsewhere.
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2 {
                        let enum_name = &qn.segments[0].name;
                        let variant_name = &qn.segments[1].name;
                        if let Some(TopSymbol::Type(TypeInfo {
                            kind: TypeKind::Enum(variants),
                            ..
                        })) = self.top.symbols.get(enum_name)
                        {
                            if variants.iter().any(|v| v.name == *variant_name) {
                                for a in args {
                                    let _ = self.check_expr(a);
                                }
                                return Ty::Named(enum_name.clone());
                            }
                        }
                    }
                }
                // Proposal A′: validate repr-tagged field accessors. When
                // `T::member(...)` names a wire-layout struct (a type with
                // a `repr:`-tagged field), `member` must be one of its
                // fields (read: `T::field`) or `set_<field>` (write). This
                // catches a mistyped field at typecheck — otherwise the
                // accessor desugars to an unknown `std::bytes::*` call and
                // only fails at codegen. Valid accessors stay permissively
                // typed (the desugar lowers them to the pack primitives).
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2 {
                        let tname = &qn.segments[0].name;
                        let member = &qn.segments[1].name;
                        if let Some(TopSymbol::Type(TypeInfo {
                            kind: TypeKind::Struct(fields),
                            ..
                        })) = self.top.symbols.get(tname)
                        {
                            let is_wire = fields.iter().any(|f| {
                                f.tag
                                    .as_deref()
                                    .and_then(|t| {
                                        hale_syntax::desugar::tag_value(t, "repr")
                                    })
                                    .is_some()
                            });
                            if is_wire {
                                let field = member
                                    .strip_prefix("set_")
                                    .unwrap_or(member);
                                let known =
                                    fields.iter().any(|f| f.name == *field);
                                if !known {
                                    let names: Vec<&str> = fields
                                        .iter()
                                        .map(|f| f.name.as_str())
                                        .collect();
                                    self.diags.push(Diag::ty(
                                        qn.span,
                                        format!(
                                            "`{}` has no wire field `{}`. \
                                             Accessors are `{}::<field>` \
                                             (read) and `{}::set_<field>` \
                                             (write); fields: {}",
                                            tname,
                                            field,
                                            tname,
                                            tname,
                                            names.join(", ")
                                        ),
                                    ));
                                }
                                for a in args {
                                    let _ = self.check_expr(a);
                                }
                                return Ty::Unknown;
                            }
                        }
                    }
                }
                let callee_ty = self.check_expr(callee);
                let arg_tys: Vec<Ty> = args.iter().map(|a| self.check_expr(a)).collect();
                // F.20: when a fn param is an interface type, the
                // arg's locus type must structurally satisfy the
                // interface (have the required methods with
                // compatible signatures). Permissive on Unknown,
                // permissive on shape mismatch — the existing
                // checker doesn't enforce arg-vs-param positional
                // typing in general; this fires *only* when the
                // param is an interface, so we don't widen the
                // call-site checking surface beyond that.
                if let Ty::Function { params, .. } = &callee_ty {
                    for (i, (param_ty, arg_ty)) in
                        params.iter().zip(arg_tys.iter()).enumerate()
                    {
                        if let (Ty::Named(iface_name), Ty::Named(arg_name)) =
                            (param_ty, arg_ty)
                        {
                            // Look up param-named symbol; only
                            // check if it actually resolves to an
                            // interface (not a locus / type /
                            // perspective).
                            let is_iface = matches!(
                                self.top.lookup(iface_name),
                                Some(TopSymbol::Interface(_))
                            );
                            // G20 follow-up: skip the structural
                            // check when the arg is itself the same
                            // interface — interface → same-interface
                            // is identity, no fat-pointer rebuild.
                            // (Different-interface → interface
                            // subtyping is a separate design call.)
                            if is_iface && arg_name != iface_name {
                                if let Err(msg) =
                                    self.check_structural_impl(arg_name, iface_name)
                                {
                                    let span = args
                                        .get(i)
                                        .map(|e| e.span())
                                        .unwrap_or_else(|| Span::new(0, 0));
                                    self.diags.push(Diag::ty(span, msg));
                                }
                            }
                        }
                    }
                }
                let base_ret = match callee_ty {
                    Ty::Function { ret, .. } => *ret,
                    _ => Ty::Unknown,
                };
                // v1.x-FORM-1: if the callee resolves to a
                // fallible fn, wrap the result type so the
                // caller is forced to address the error.
                if let Some(payload) = self.callee_fallible_payload(callee) {
                    Ty::Fallible {
                        success: Box::new(base_ret),
                        payload: Box::new(payload),
                    }
                } else {
                    base_ret
                }
            }
            Expr::Field { receiver, name, span } => {
                // F.11 entity-collection sugar: `self.children.count`
                // (Int) and `self.children.is_empty` (Bool) read the
                // accept'd-child tracker's live count. `self.children`
                // alone is only a `for` iterand (typed `[Child]`); these
                // two accessors are the summary surface F.11 commits to.
                if let Expr::Field {
                    receiver: inner,
                    name: inner_name,
                    ..
                } = receiver.as_ref()
                {
                    if matches!(inner.as_ref(), Expr::KwSelf(_))
                        && inner_name.name == "children"
                        && (name.name == "count" || name.name == "is_empty")
                    {
                        let accepts = self
                            .current_locus
                            .map_or(false, |li| li.accept_param.is_some());
                        if !accepts {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "`self.children.{}` requires the enclosing \
                                     locus to `accept` a child type",
                                    name.name
                                ),
                            ));
                        }
                        return if name.name == "count" {
                            Ty::Prim(PrimType::Int)
                        } else {
                            Ty::Prim(PrimType::Bool)
                        };
                    }
                }
                let rt = self.check_expr(receiver);
                match self.field_ty(&rt, &name.name) {
                    Some(t) => t,
                    None => {
                        // F.22: `self.<slot>` references a capacity
                        // slot, not a field. Slots don't have a
                        // value-level type the typechecker reasons
                        // about (the cell value only appears when
                        // they're used as a method-call receiver),
                        // so return Unknown rather than diagnosing
                        // a missing field. Codegen catches misuse
                        // (slot in non-method-call position).
                        if matches!(receiver.as_ref(), Expr::KwSelf(_)) {
                            if let Ty::Named(locus_name) = &rt {
                                if let Some(TopSymbol::Locus(li)) =
                                    self.top.symbols.get(locus_name)
                                {
                                    if li
                                        .capacity_slot_names
                                        .iter()
                                        .any(|n| n == &name.name)
                                    {
                                        return Ty::Unknown;
                                    }
                                }
                            }
                        }
                        // Permissive on Unknown — stdlib paths
                        // and externally-typed values pass
                        // through. Strict when the receiver
                        // is a known type and the field
                        // doesn't exist on it: catches typos
                        // statically.
                        if !matches!(rt, Ty::Unknown) {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "no field `{}` on `{}`",
                                    name.name,
                                    rt.display()
                                ),
                            ));
                        }
                        Ty::Unknown
                    }
                }
            }
            Expr::Index { receiver, index, .. } => {
                let rt = self.check_expr(receiver);
                let _ = self.check_expr(index);
                match rt {
                    Ty::Array(elem, _) => *elem,
                    _ => Ty::Unknown,
                }
            }
            Expr::Tuple(parts, _) => {
                Ty::Tuple(parts.iter().map(|e| self.check_expr_local(e)).collect())
            }
            Expr::Array(parts, _) => {
                let elem = if let Some(first) = parts.first() {
                    self.check_expr_local(first)
                } else {
                    Ty::Unknown
                };
                for e in parts.iter().skip(1) {
                    let _ = self.check_expr(e);
                }
                Ty::Array(Box::new(elem), Some(parts.len() as u64))
            }
            Expr::ArrayRepeat { val, count, .. } => {
                // `[val; N]` — same array shape, single element
                // type repeated N times. Count is parser-validated
                // as a non-negative Int literal.
                let elem = self.check_expr_local(val);
                Ty::Array(Box::new(elem), Some(*count))
            }
            Expr::Struct { path, inits, span } => self.check_struct_literal(path, inits, *span),
            Expr::Block(b) => self.check_block_as_expr(b),
            Expr::If(s) => self.check_if_as_expr(s),
            Expr::Match(m) => {
                self.check_match(m);
                Ty::Unit
            }
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.check_expr(inner),
            Expr::Approx { left, right, tolerance, span } => {
                if !self.in_closure {
                    self.diags.push(Diag::ty(
                        *span,
                        "approximate-equality (`~~`) only valid inside a closure block"
                            .to_string(),
                    ));
                }
                let _ = self.check_expr(left);
                let _ = self.check_expr(right);
                let _ = self.check_expr(tolerance);
                Ty::Prim(PrimType::Bool)
            }
            Expr::Range { lo, hi, .. } => {
                // v0 ranges are integer iterators only. Both sides
                // must be Int. The expression itself doesn't have a
                // first-class type beyond "iterator over Int" — the
                // for-stmt handler is the only consumer that
                // recognizes it. Returning Unknown lets callers in
                // non-iterator positions still typecheck without
                // the result being used as a value.
                let _ = self.check_expr(lo);
                let _ = self.check_expr(hi);
                Ty::Unknown
            }
            Expr::Or { inner, disposition, span } => {
                let value_discarded = self.or_value_discarded;
                self.or_value_discarded = false;
                let inner_ty = self.check_expr(inner);
                // M3 stage 2 (2026-07-02): stdlib fallible
                // path-calls are dual-mode at codegen (bare = the
                // legacy direct form, `or` = fallible ABI), so the
                // Call arm types them Unknown; the precise
                // success/payload for the `or` form comes from the
                // signature table here.
                let stdlib_or = match inner.as_ref() {
                    Expr::Call { callee, .. } => match callee.as_ref() {
                        Expr::Path(qn) => {
                            let segs: Vec<&str> = qn
                                .segments
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect();
                            crate::stdlib_surface::signature_for(&segs)
                                .and_then(|sig| sig.or_types())
                        }
                        _ => None,
                    },
                    _ => None,
                };
                // Unwrap the fallible to get success + payload
                // types. If the inner isn't actually fallible,
                // the `or` clause is a no-op at best and likely
                // a user mistake.
                let (success, payload) = match (stdlib_or, inner_ty) {
                    (Some((s, p)), _) => (s, p),
                    (None, Ty::Fallible { success, payload }) => {
                        (*success, *payload)
                    }
                    (None, Ty::Unknown) => (Ty::Unknown, Ty::Unknown),
                    (None, other) => {
                        self.diags.push(Diag::ty(
                            inner.span(),
                            format!(
                                "`or` disposition expects a fallible-typed \
                                 expression on the left; got `{}` (not fallible). \
                                 Drop the `or` clause if the call can't fail.",
                                other.display()
                            ),
                        ));
                        return other;
                    }
                };
                match disposition {
                    OrDisposition::Raise(_) => {
                        // `or raise` diverges via closure
                        // violation; expression's value type is
                        // the success type.
                        success
                    }
                    OrDisposition::Discard(span) => {
                        // `or discard` — swallow error, produce
                        // Unit. Requires the underlying call's
                        // success type to be Unit (since discard
                        // doesn't carry a value).
                        if !matches!(success, Ty::Unit | Ty::Unknown) {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "`or discard` requires the underlying \
                                     call's success type to be Unit (so the \
                                     discard branch produces no value to \
                                     bind); got `{}`. Use `or <default>` or \
                                     `or raise` for value-bearing fallibles.",
                                    success.display()
                                ),
                            ));
                        }
                        let _ = payload;
                        Ty::Unit
                    }
                    OrDisposition::Fail(payload_expr, span) => {
                        // B3 / G6: `or fail X` diverges via the
                        // enclosing fallible fn's error path. The
                        // payload's type must match the enclosing
                        // fn's declared error type — not the
                        // inner call's payload. Same divergence
                        // rule as `or raise`: expression type
                        // collapses to the inner success type.
                        let _ = payload;
                        let new_payload_ty = self.check_expr_addressed(payload_expr);
                        match &self.fallible_ctx {
                            None => self.diags.push(Diag::ty(
                                *span,
                                "`or fail X`: only valid inside a fallible \
                                 fn body (declared with `fallible(T)`). \
                                 Use `or raise` to propagate the inner \
                                 payload, or `or <fallback>` to substitute \
                                 a value".to_string(),
                            )),
                            Some((_, expected_payload)) => {
                                if !expected_payload.assignable_from(&new_payload_ty) {
                                    self.diags.push(Diag::ty(
                                        payload_expr.span(),
                                        format!(
                                            "`or fail`: expected payload \
                                             type `{}`, got `{}`",
                                            expected_payload.display(),
                                            new_payload_ty.display()
                                        ),
                                    ));
                                }
                            }
                        }
                        success
                    }
                    OrDisposition::Substitute(rhs) => {
                        // The implicit `err` binding is in scope
                        // on the RHS, typed as the payload type.
                        self.locals.push();
                        self.locals.insert(
                            "err",
                            LocalSym {
                                ty: payload.clone(),
                                is_mut: false,
                            },
                        );
                        let rhs_ty = self.check_expr(rhs);
                        self.locals.pop();
                        // 2026-05-18 — locus → interface coercion at
                        // `or <substitute>` site. Mirrors the
                        // call-site and struct-literal coercion: when
                        // the fallible's success type is an interface
                        // and the substitute expression is a concrete
                        // locus that structurally satisfies it, accept
                        // the substitute. Without this, the substitute
                        // disposition was the only `or` arm that
                        // refused locus→interface flow.
                        let interface_satisfied = if let (
                            Ty::Named(iface_name),
                            Ty::Named(rhs_name),
                        ) = (&success, &rhs_ty)
                        {
                            if matches!(
                                self.top.lookup(iface_name),
                                Some(TopSymbol::Interface(_))
                            ) {
                                match self.check_structural_impl(rhs_name, iface_name) {
                                    Ok(()) => true,
                                    Err(msg) => {
                                        self.diags.push(Diag::ty(rhs.span(), msg));
                                        true
                                    }
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        // 2026-07-02 fallible handlers: the substitute
                        // may itself be a fallible call —
                        // `db() or self.convert(err)` where `convert`
                        // is `fallible(E2)`. Semantics are implicit
                        // `or raise` on the handler: its success value
                        // substitutes; its failure propagates through
                        // the ENCLOSING fn's error path, so E2 must be
                        // assignable to the enclosing fallible
                        // payload. (Sugar for the already-legal
                        // `db() or (self.convert(err) or raise)` —
                        // this closes the pond stash-bridge idiom that
                        // made jobs::Queue non-reentrant.)
                        if let Ty::Fallible {
                            success: h_success,
                            payload: h_payload,
                        } = &rhs_ty
                        {
                            if !value_discarded
                                && !success.assignable_from(h_success)
                            {
                                self.diags.push(Diag::ty(
                                    *span,
                                    format!(
                                        "`or <handler>`: handler's success \
                                         type `{}` does not match the \
                                         call's success type `{}`",
                                        h_success.display(),
                                        success.display()
                                    ),
                                ));
                            }
                            match &self.fallible_ctx {
                                None => self.diags.push(Diag::ty(
                                    *span,
                                    format!(
                                        "`or <handler>`: handler is \
                                         `fallible({})` but the enclosing \
                                         fn is not fallible — the \
                                         handler's failure has nowhere to \
                                         go. Declare the enclosing fn \
                                         `fallible({})`, or handle inside \
                                         the handler and drop its \
                                         `fallible`",
                                        h_payload.display(),
                                        h_payload.display()
                                    ),
                                )),
                                Some((_, expected_payload)) => {
                                    if !expected_payload
                                        .assignable_from(h_payload)
                                    {
                                        self.diags.push(Diag::ty(
                                            *span,
                                            format!(
                                                "`or <handler>`: handler \
                                                 fails with `{}` but the \
                                                 enclosing fn is \
                                                 `fallible({})` — the \
                                                 propagated payload must \
                                                 match",
                                                h_payload.display(),
                                                expected_payload.display()
                                            ),
                                        ));
                                    }
                                }
                            }
                            return success;
                        }
                        // Docs/spec pass find (2026-07-02): a
                        // STDLIB fallible path-call used directly
                        // as the handler compiles but silently
                        // yields the un-addressed sret ("" / 0) on
                        // the handler's OWN failure instead of
                        // propagating — the codegen handler
                        // classifier doesn't cover stdlib paths.
                        // Reject with the working spelling until
                        // it does.
                        if let Expr::Call { callee, .. } = rhs.as_ref() {
                            if let Expr::Path(qn) = callee.as_ref() {
                                let segs: Vec<&str> = qn
                                    .segments
                                    .iter()
                                    .map(|s| s.name.as_str())
                                    .collect();
                                let is_fallible_stdlib =
                                    crate::stdlib_surface::signature_for(
                                        &segs,
                                    )
                                    .map(|sig| sig.fallible.is_some())
                                    .unwrap_or(false);
                                if is_fallible_stdlib {
                                    self.diags.push(Diag::ty(
                                        *span,
                                        format!(
                                            "`or {}(...)`: a fallible \
                                             stdlib call can't be the \
                                             handler directly yet — \
                                             write the nested form `or \
                                             ({}(...) or raise)` so its \
                                             own failure has a path",
                                            segs.join("::"),
                                            segs.join("::")
                                        ),
                                    ));
                                }
                            }
                        }
                        // The substitute RHS must produce a
                        // value of the success type (or be a
                        // nested `or` that ultimately produces
                        // one). Permissive on Unknown so we
                        // don't false-positive when the
                        // typechecker can't see through a
                        // stdlib path.
                        if !interface_satisfied
                            && !value_discarded
                            && !success.assignable_from(&rhs_ty)
                        {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "`or <substitute>`: fallback type `{}` \
                                     does not match success type `{}`",
                                    rhs_ty.display(),
                                    success.display()
                                ),
                            ));
                        }
                        success
                    }
                }
            }
        }
    }

    /// Same as check_expr but used when we need a type without
    /// risking borrow conflicts with the recursion. (In practice
    /// it's identical; named to mark intent at the call sites.)
    fn check_expr_local(&mut self, expr: &Expr) -> Ty {
        self.check_expr(expr)
    }

    /// v1.x-FORM-1: check an expression that's expected to
    /// produce a regular (non-fallible) value. If the expression
    /// is fallible-typed at its outermost level, emit an
    /// `error not addressed` diagnostic and return the
    /// (would-be) success type so downstream typechecks can
    /// continue without cascading errors.
    fn check_expr_addressed(&mut self, expr: &Expr) -> Ty {
        let ty = self.check_expr(expr);
        match ty {
            Ty::Fallible { success, .. } => {
                self.diags.push(Diag::ty(
                    expr.span(),
                    "error not addressed: this expression's fallible result \
                     must be handled with an `or` clause (`or raise`, \
                     `or <fallback>`, `or handler(err)`) or a `match`"
                        .to_string(),
                ));
                *success
            }
            other => other,
        }
    }

    /// v1.x-FORM-1: if `callee` is a name reference resolving to
    /// a known fallible fn (or method on a locus / perspective),
    /// return the fn's payload type. Returns None for non-fn
    /// callees or non-fallible callees — caller uses the result
    /// to decide whether to wrap the call's return in
    /// `Ty::Fallible`.
    fn callee_fallible_payload(&mut self, callee: &Expr) -> Option<Ty> {
        match callee {
            Expr::Ident(id) => match self.top.lookup(&id.name)? {
                TopSymbol::Fn(sig) => sig.fallible.clone(),
                _ => None,
            },
            Expr::Path(qn) if qn.segments.len() == 1 => {
                match self.top.lookup(&qn.segments[0].name)? {
                    TopSymbol::Fn(sig) => sig.fallible.clone(),
                    _ => None,
                }
            }
            // v1.x-FORM-1 PR3b: method calls like `l.get(i)`. The
            // callee is a Field expression whose receiver resolves
            // to a locus/perspective; we look up the method by
            // name on that type and inspect its fallibility.
            Expr::Field { receiver, name, .. } => {
                let rt = self.check_expr_local(receiver);
                let type_name = match rt {
                    Ty::Named(n) => n,
                    _ => return None,
                };
                match self.top.lookup(&type_name)? {
                    TopSymbol::Locus(info) => info
                        .methods
                        .iter()
                        .find(|m| m.name == name.name)
                        .and_then(|m| m.fallible.clone()),
                    TopSymbol::Perspective(info) => info
                        .methods
                        .iter()
                        .find(|m| m.name == name.name)
                        .and_then(|m| m.fallible.clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether a value of type `t` can be auto-coerced to String
    /// inside a `String + <t>` expression. Mirrors the codegen
    /// `value_to_string_supports` set: every primitive that
    /// `to_string(...)` accepts, plus enums (which render as their
    /// variant name).
    fn ty_is_printable(t: &Ty) -> bool {
        match t {
            Ty::Prim(p) => matches!(
                p,
                PrimType::String
                    | PrimType::Int
                    | PrimType::Bool
                    | PrimType::Float
                    | PrimType::Decimal
                    | PrimType::Duration
                    | PrimType::Time
            ),
            // Named types: enums render via to_string at codegen.
            // The typechecker doesn't distinguish enum vs struct
            // here without more lookup work; permit and let
            // codegen reject if the type isn't actually printable
            // (struct with no Display rendering would still error
            // there).
            Ty::Named(_) => true,
            _ => false,
        }
    }

    fn binop_ty(&mut self, op: BinOp, lt: &Ty, rt: &Ty, span: Span) -> Ty {
        use BinOp::*;
        // Ergonomics arc: `String + <printable>` and the symmetric
        // form auto-coerce in codegen via value_to_string. The
        // typechecker mirrors that by short-circuiting on the
        // mixed-String add as a permitted shape that yields String.
        if matches!(op, Add) {
            let l_str = matches!(lt, Ty::Prim(PrimType::String));
            let r_str = matches!(rt, Ty::Prim(PrimType::String));
            if (l_str && Self::ty_is_printable(rt))
                || (r_str && Self::ty_is_printable(lt))
            {
                return Ty::Prim(PrimType::String);
            }
        }
        // B13 / G30: F.23 Int → Float widening in binary-op
        // position. If exactly one side is Int and the other is
        // Float, the result is Float. Decimal stays strict —
        // F.23 explicitly does NOT widen Int/Float into Decimal
        // (Decimal precision must not silently promote out from
        // monetary scale-9). Mirrors the codegen-side coercion.
        let is_int_float_mix = matches!(
            (lt, rt),
            (Ty::Prim(PrimType::Int), Ty::Prim(PrimType::Float))
                | (Ty::Prim(PrimType::Float), Ty::Prim(PrimType::Int))
        );
        match op {
            Add | Sub | Mul | Div | Mod | BitAnd | BitOr | BitXor | Shl | Shr => {
                if is_int_float_mix {
                    return Ty::Prim(PrimType::Float);
                }
                if !lt.assignable_from(rt) && !rt.assignable_from(lt) {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "binary op: incompatible operand types `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                if matches!(lt, Ty::Unknown) {
                    rt.clone()
                } else {
                    lt.clone()
                }
            }
            Eq | NotEq | Lt | Gt | LtEq | GtEq => {
                if is_int_float_mix {
                    return Ty::Prim(PrimType::Bool);
                }
                if !lt.assignable_from(rt) && !rt.assignable_from(lt) {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "comparison: incompatible operand types `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                Ty::Prim(PrimType::Bool)
            }
            And | Or => Ty::Prim(PrimType::Bool),
        }
    }

    /// M3 stage 3 tranche 2: match a mangled monomorph name
    /// (`Box_Int`, `Pair_Int_String`) against a generic type
    /// template, producing the generic→Ty bindings. The mangle
    /// joins single tokens with `_`; template base names
    /// containing `_` are handled by prefix match. None when no
    /// template matches or the token count disagrees.
    fn resolve_generic_monomorph(
        &self,
        name: &str,
    ) -> Option<(&'a TypeDecl, BTreeMap<String, Ty>)> {
        for (base, template) in &self.generic_types {
            let prefix = format!("{}_", base);
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let toks: Vec<&str> = rest.split('_').collect();
            if toks.len() != template.generics.len() {
                continue;
            }
            let mut bindings: BTreeMap<String, Ty> = BTreeMap::new();
            for (g, tok) in template.generics.iter().zip(toks.iter()) {
                bindings.insert(
                    g.name.name.clone(),
                    mangle_token_to_ty(tok, self.known),
                );
            }
            return Some((template, bindings));
        }
        None
    }

    fn check_struct_literal(
        &mut self,
        path: &QualifiedName,
        inits: &[StructInit],
        span: Span,
    ) -> Ty {
        if path.segments.len() != 1 {
            for init in inits {
                let _ = self.check_expr(&init.value);
            }
            return Ty::Unknown;
        }
        let name = &path.segments[0].name;
        // M3 stage 3 tranche 2 (2026-07-02): mangled generic
        // monomorph literal (`Box_Int { ... }`). Resolve the
        // `Base_Tok[_Tok...]` shape against a generic type
        // template, substitute the type args into the field
        // types, and validate the inits like a concrete struct.
        // (Previously "unknown type" — generic types were
        // unusable through the CLI.)
        if self.top.lookup(name).is_none() {
            if let Some((template, bindings)) =
                self.resolve_generic_monomorph(name)
            {
                if let TypeDeclBody::Struct(tfields) = &template.body {
                    let fields: Vec<(String, Ty, bool)> = tfields
                        .iter()
                        .map(|f| {
                            (
                                f.name.name.clone(),
                                substitute_generic_ty(
                                    &f.ty,
                                    &bindings,
                                    self.known,
                                ),
                                f.default.is_some(),
                            )
                        })
                        .collect();
                    return self.check_literal_fields(
                        name, &fields, "type", true, inits, span,
                    );
                }
            }
        }
        let sym = match self.top.lookup(name) {
            Some(s) => s,
            None => {
                self.diags.push(Diag::ty(
                    span,
                    format!("unknown type `{}` in struct/locus literal", name),
                ));
                for init in inits {
                    let _ = self.check_expr(&init.value);
                }
                return Ty::Unknown;
            }
        };

        // 2026-05-16 — loci + perspectives now also enforce
        // "missing field" when the param has no default
        // (`has_default: false`). Previously omitted for loci
        // because every param historically carried a default; the
        // required-param form (`name: T;`) introduced 2026-05-16
        // makes the check meaningful — otherwise `Server { port:
        // 8080 }` (missing required `handler`) would silently fall
        // through to codegen.
        let (fields, kind_label, requires_all): (Vec<(String, Ty, bool)>, &str, bool) = match sym {
            TopSymbol::Type(info) => match &info.kind {
                TypeKind::Struct(fields) => (
                    fields
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone(), f.has_default))
                        .collect(),
                    "type",
                    true,
                ),
                _ => {
                    self.diags.push(Diag::ty(
                        span,
                        format!("`{}` is not a struct type", name),
                    ));
                    return Ty::Unknown;
                }
            },
            TopSymbol::Locus(info) => (
                info.params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.has_default))
                    .collect(),
                "locus",
                true,
            ),
            TopSymbol::Perspective(info) => (
                info.params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.has_default))
                    .collect(),
                "perspective",
                true,
            ),
            _ => {
                self.diags.push(Diag::ty(
                    span,
                    format!("`{}` cannot be instantiated with `{{...}}`", name),
                ));
                return Ty::Unknown;
            }
        };

        self.check_literal_fields(
            name, &fields, kind_label, requires_all, inits, span,
        )
    }

    /// Shared literal field validation — concrete structs/loci/
    /// perspectives and (M3 stage 3 tranche 2) substituted generic
    /// monomorphs both land here.
    fn check_literal_fields(
        &mut self,
        name: &str,
        fields: &[(String, Ty, bool)],
        kind_label: &str,
        requires_all: bool,
        inits: &[StructInit],
        span: Span,
    ) -> Ty {
        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        for init in inits {
            let got = self.check_expr(&init.value);
            match fields.iter().find(|(n, _, _)| n == &init.name.name) {
                Some((_, want, _)) => {
                    // 2026-05-16 — locus → interface coercion at
                    // struct/locus literal init. Mirrors the fn-arg
                    // call-site coercion above so a stateful locus
                    // can flow into an interface-typed field (e.g.
                    // `Server { handler: MyHandler { } }` where
                    // `handler: HttpHandler`).
                    let interface_satisfied = if let (Ty::Named(iface_name), Ty::Named(arg_name)) =
                        (want, &got)
                    {
                        if matches!(
                            self.top.lookup(iface_name),
                            Some(TopSymbol::Interface(_))
                        ) {
                            match self.check_structural_impl(arg_name, iface_name) {
                                Ok(()) => true,
                                Err(msg) => {
                                    self.diags.push(Diag::ty(
                                        init.value.span(),
                                        msg,
                                    ));
                                    true
                                }
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !interface_satisfied && !want.assignable_from(&got) {
                        self.diags.push(Diag::ty(
                            init.value.span(),
                            format!(
                                "{} `{}`: field `{}` expects `{}`, got `{}`",
                                kind_label,
                                name,
                                init.name.name,
                                want.display(),
                                got.display()
                            ),
                        ));
                    }
                }
                None => {
                    self.diags.push(Diag::ty(
                        init.span,
                        format!(
                            "{} `{}` has no field `{}`",
                            kind_label, name, init.name.name
                        ),
                    ));
                }
            }
            seen.insert(init.name.name.clone(), ());
        }
        if requires_all {
            for (fname, fty, has_default) in fields {
                // bounded[T; N] fields auto-init empty and cannot
                // be spelled in a literal.
                if matches!(fty, Ty::Bounded(_, _)) {
                    if seen.contains_key(fname) {
                        self.diags.push(Diag::ty(
                            span,
                            format!(
                                "{} `{}` field `{}`: bounded[T; N] \
                                 fields cannot be initialized in a \
                                 literal — they start empty; use \
                                 push(...)",
                                kind_label, name, fname
                            ),
                        ));
                    }
                    continue;
                }
                if !seen.contains_key(fname) && !has_default {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "{} `{}`: missing field `{}`",
                            kind_label, name, fname
                        ),
                    ));
                }
            }
        }

        Ty::Named(name.to_string())
    }
}

fn lit_ty(lit: &Literal) -> Ty {
    match lit {
        Literal::Int(_) => Ty::Prim(PrimType::Int),
        Literal::Float(_) => Ty::Prim(PrimType::Float),
        Literal::Decimal(_) => Ty::Prim(PrimType::Decimal),
        Literal::String(_) => Ty::Prim(PrimType::String),
        Literal::Bool(_) => Ty::Prim(PrimType::Bool),
        Literal::Nil => Ty::Unknown,
        Literal::Duration(_) => Ty::Prim(PrimType::Duration),
        Literal::Time(_) => Ty::Prim(PrimType::Time),
        Literal::Bytes(_) => Ty::Prim(PrimType::Bytes),
    }
}
