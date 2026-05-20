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

use std::collections::BTreeMap;

use aperio_syntax::ast::*;
use aperio_syntax::{Diag, Span};

use crate::resolve::{resolve_type_expr, TopScope};
use crate::symbol::*;
use crate::ty::{is_flat_shapeable, Ty};

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

pub fn check_bundle(bundle: &Bundle<'_>, top: &TopScope) -> Vec<Diag> {
    let mut diags = Vec::new();
    let known = collect_known_names(top);
    for program in bundle.programs.values() {
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
    diags
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
                         payload type `{}` is not flat-shapeable (contains \
                         String, Bytes, or other variable-size fields)",
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

                            // Form K6a (2026-05-20): shm_ring is
                            // publish-only on the Aperio side at
                            // v1. If a same-bundle locus declares
                            // `bus subscribe` for a shm_ring-
                            // bound topic, emit a clear "not yet
                            // wired" diagnostic so the user
                            // doesn't silently get a no-op
                            // subscription. The full subscriber-
                            // side reader-thread + view-into-slot
                            // codegen lands post-K6; cross-
                            // language consumers (C readers via
                            // `lotus_shm_ring_*`) work today.
                            if matches!(
                                entry.transport,
                                TransportSpec::ShmRing { .. }
                            ) && topic_subscribes.contains(&entry.topic.name) {
                                diags.push(Diag::ty(
                                    entry.topic.span,
                                    format!(
                                        "binding for topic `{}`: \
                                         shm_ring transport doesn't yet \
                                         support Aperio-side subscribers — \
                                         the publish path is wired (Form \
                                         K4c) but subscriber codegen + \
                                         reader-thread integration lands \
                                         post-K6. Subscribe to the SHM \
                                         ring from a separate process \
                                         using the C `lotus_shm_ring_*` \
                                         primitives, or use a different \
                                         transport for this topic",
                                        entry.topic.name
                                    ),
                                ));
                            }
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
            TopDecl::Topic(_) => {
                // Topic declarations carry only `payload: T;`. The
                // resolver validated the payload type expression
                // already; per-use-site checks (handler-sig match,
                // send-payload match) happen in the bus blocks and
                // send sites that reference the topic.
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

        for member in &decl.members {
            self.check_locus_member(member);
        }

        self.current_locus = prev;
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
            other => {
                self.diags.push(Diag::ty(
                    form.name.span,
                    format!(
                        "unknown form `{}`; v1 recognizes: vec, hashmap, \
                         ring_buffer",
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

    /// v1.x-FORM-4: `@form(hashmap)` requires exactly one
    /// capacity slot, of kind `pool`, with an `indexed_by
    /// <fieldname>` clause. The slot's cell type must be a
    /// user-declared struct; the indexed-by field must exist
    /// on that struct. The field's type becomes the hashmap
    /// key type K; the cell type becomes the value type S.
    fn check_form_hashmap_shape(&mut self, decl: &'a LocusDecl, form: &'a FormAnnotation) {
        if !form.args.is_empty() {
            self.diags.push(Diag::ty(
                form.span,
                format!(
                    "@form(hashmap) takes no arguments; got {} (hashmap has no \
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
                        "@form(hashmap) cell type `{}` is a locus; cells must \
                         be value-shape types (struct), not loci with lifecycle",
                        cell_name
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
                // v1.x-FORM-2 design rule (two-channel
                // separation): locus methods can't declare
                // `fallible(E)`. Substrate-facing methods
                // communicate failure structurally via closure
                // assertions + `on_failure` routing; value-level
                // `fallible(E)` lives on free fns and stdlib-
                // synthesized methods over `@form(...)`
                // containers (application-layer storage
                // substrate). The channels meet only at the
                // implicit main locus root via
                // `lotus_root_panic`. See `spec/semantics.md`
                // § "Fallible call semantics".
                if let Some(payload_te) = &f.fallible {
                    self.diags.push(Diag::ty(
                        payload_te.span(),
                        format!(
                            "locus method `{}`: locus methods can't declare \
                             `fallible(E)`. Substrate-facing methods \
                             communicate failure structurally via closure \
                             assertions + `on_failure` routing; value-level \
                             `fallible(E)` lives on free fns and stdlib-\
                             synthesized methods over `@form(...)` \
                             containers. Three workarounds (styleguide § 7): \
                             (1) wrap as a free fn — write `fn op() -> T \
                             fallible(E)` outside the locus and call it \
                             from the method with `or <fallback>` / `or \
                             raise`; (2) error-check fn + `violate NAME;` \
                             — keep the method infallible and route \
                             structural failure through an inline closure \
                             so on_failure can pick it up; (3) sentinel-\
                             predicate — return a Bool / Option-like shape \
                             from the method and let the caller branch.",
                            f.name.name
                        ),
                    ));
                }
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
                                     type cannot be a locus (locus \
                                     recycling/free would orphan the \
                                     locus — route locus membership \
                                     through `accept(c: {})` instead; \
                                     see spec §F.22 restriction 1)",
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
            Stmt::Send { subject, value, span } => {
                self.check_send(subject, value, *span);
            }
            Stmt::If(if_stmt) => self.check_if(if_stmt),
            Stmt::Match(m) => self.check_match(m),
            Stmt::For { name, iter, body, .. } => {
                let _ = self.check_expr(iter);
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
                let _ = self.check_expr_addressed(e);
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

    fn check_send(&mut self, subject: &Expr, value: &Expr, span: Span) {
        let payload_ty = self.check_expr(value);
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
            Ty::Named(n) => match self.top.lookup(n)? {
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
            },
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
                let inner_ty = self.check_expr(inner);
                // Unwrap the fallible to get success + payload
                // types. If the inner isn't actually fallible,
                // the `or` clause is a no-op at best and likely
                // a user mistake.
                let (success, payload) = match inner_ty {
                    Ty::Fallible { success, payload } => (*success, *payload),
                    Ty::Unknown => (Ty::Unknown, Ty::Unknown),
                    other => {
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
                        // The substitute RHS must produce a
                        // value of the success type (or be a
                        // nested `or` that ultimately produces
                        // one). Permissive on Unknown so we
                        // don't false-positive when the
                        // typechecker can't see through a
                        // stdlib path.
                        if !interface_satisfied && !success.assignable_from(&rhs_ty) {
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
            for (fname, _, has_default) in &fields {
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

        Ty::Named(name.clone())
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
