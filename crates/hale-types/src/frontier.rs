//! GH #265 — the frontier items the issue flagged as beyond phase 1
//! but reachable on this substrate:
//!
//!   * **`@effects(causes: {…})`** — cross-actor causal reasoning.
//!     The call graph stops at a publish; the BUS graph continues.
//!     "This HTTP handler, by publishing `orders`, can transitively
//!     cause a `fs.write` in the audit subscriber" is checkable
//!     because Hale's message graph is declared. Erlang/Akka
//!     structurally cannot do this — they have no declared edges.
//!   * **`@supervised`** — every locus in a subtree has a declared
//!     failure policy. Supervision coverage as a *checked* property
//!     is something the actor world has wanted statically for
//!     decades; Hale's declared tree makes it a tree walk.
//!   * **`@secret` taint** — no `@secret`-derived value reaches a
//!     publish or a log sink. Coarse (parameter-granular, not
//!     value-flow-complete), which the issue explicitly wanted
//!     assessed rather than deferred to "v2 horizon".
//!   * **Manifest effect inference** — the `.hale.effects` manifest
//!     can carry INFERRED sets, not just declared ones, because a
//!     manifest is a report rather than a type. (Effect rows on
//!     function *types* remain the deferred slippery slope.)
//!   * **Symbolic cost** — a structural cost expression in input
//!     sizes for fns that are already bounded. Not WCET: the
//!     first-filter shape hard-real-time triage needs.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::alloc_summary::{self, AllocSummary, Callee, FnKey};
use crate::bus_graph::BusGraph;
use crate::callgraph;
use crate::stdlib_surface::{self, EffectSet};

// ===================== inferred effect sets =====================

/// The effect set a fn actually performs, transitively — inferred,
/// never declared. Used by the manifest (a report) and by the
/// causality check (which needs each subscriber's set).
pub fn infer_effects(
    summary: &AllocSummary,
    key: &FnKey,
    ffi: &BTreeSet<String>,
) -> EffectSet {
    fn walk(
        summary: &AllocSummary,
        key: &FnKey,
        ffi: &BTreeSet<String>,
        seen: &mut BTreeSet<FnKey>,
        steps: &mut u32,
    ) -> EffectSet {
        if !seen.insert(key.clone()) {
            return EffectSet::PURE;
        }
        *steps += 1;
        if *steps > callgraph::MAX_STEPS {
            return EffectSet::UNCLASSIFIED;
        }
        let Some(fs) = summary.fns.get(key) else {
            return EffectSet::PURE;
        };
        let mut acc = EffectSet::PURE;
        // #345: what THIS fn declares it carries. The callee arm below
        // unions a leaf's `carries` when something calls it, but a fn's
        // own classification was invisible to its own inferred set — so
        // a bus subscriber declaring `is: {money}` contributed nothing
        // to `causes_diags`, which infers each subscriber's effects
        // from here. A user class silently did not travel over the bus,
        // while every built-in did.
        if let Some(c) = summary.carries.get(key) {
            acc = acc.union(*c);
        }
        if !fs.sites.is_empty() {
            acc = acc.union(EffectSet::ALLOC);
        }
        for site in &fs.effect_sites {
            acc = acc.union(match site.kind {
                alloc_summary::EffectSiteKind::Publish(_) => {
                    EffectSet::PUBLISH
                }
                alloc_summary::EffectSiteKind::Spawn(_) => EffectSet::ALLOC,
            });
        }
        for edge in &fs.calls {
            match &edge.callee {
                Callee::Resolved(k) => {
                    // #345: what this leaf DECLARES it carries.
                    if let Some(c) = summary.carries.get(k) {
                        acc = acc.union(*c);
                    }
                    if k.locus.is_none() && ffi.contains(&k.fn_name) {
                        acc = acc.union(EffectSet::SYSCALL);
                    }
                    acc = acc.union(walk(summary, k, ffi, seen, steps));
                }
                Callee::Unresolved(name) => {
                    // #353: a call through a FUNCTION-TYPED PARAMETER.
                    // It lands here looking like an unknown free fn, so
                    // it contributed nothing — and that is how an
                    // indirect call voided every certificate:
                    // `@no_syscall` on a fn whose body is `return f(v);`
                    // passed while the program performed the syscall,
                    // and `@budget(alloc_per_call = 0)` leaked the same
                    // way.
                    //
                    // The target is not knowable from this fn alone, so
                    // it is UNCLASSIFIED — "may do anything" — which is
                    // the same fail-closed treatment an unclassified
                    // stdlib leaf gets. Resolving it exactly needs the
                    // binding from the call site (see the tier-1 note in
                    // the issue); until then, closed beats open.
                    if fs.fn_params.iter().any(|p| p == name) {
                        acc = acc.union(EffectSet::UNCLASSIFIED);
                        continue;
                    }
                    // #382 receiver-typing: a method call on a
                    // receiver the summarizer STILL cannot type (an
                    // index result, a match value, a foreign
                    // expression — the common shapes now resolve)
                    // is a method of some bundle locus reached
                    // through an opaque expression. Same fail-closed
                    // treatment as an indirect call: it may do
                    // anything. (#392 interface dispatch never lands
                    // here: fanned out when conformers exist, dead
                    // code when none do.)
                    if edge.opaque_method_call() {
                        acc = acc.union(EffectSet::UNCLASSIFIED);
                        continue;
                    }
                    let segs: Vec<&str> = name.split("::").collect();
                    if let Some(e) = stdlib_surface::effects_for(&segs) {
                        if !e.is_unclassified() {
                            acc = acc.union(e);
                        }
                    }
                }
            }
        }
        acc
    }
    let mut seen = BTreeSet::new();
    let mut steps = 0u32;
    walk(summary, key, ffi, &mut seen, &mut steps)
}

/// Render an effect set as sorted class names — the manifest's
/// stable form.
///
/// Built-ins only; a user class has no static name. Prefer
/// [`render_effects_named`] anywhere the seed's intern table is in
/// reach, or a set containing a user class renders as if it were
/// empty — which reads as "causes nothing".
pub fn render_effects(e: EffectSet) -> Vec<String> {
    render_effects_named(e, &[])
}

/// [`render_effects`] with the seed's user effect-class table, so
/// `User(i)` bits render as the name the author declared.
pub fn render_effects_named(e: EffectSet, names: &[String]) -> Vec<String> {
    if e.is_unclassified() {
        return vec!["unclassified".to_string()];
    }
    let mut out = Vec::new();
    for (mask, name) in [
        (EffectSet::SYSCALL, "syscall"),
        (EffectSet::BLOCK, "block"),
        (EffectSet::PUBLISH, "publish"),
        (EffectSet::TIME, "time"),
        (EffectSet::ENTROPY, "entropy"),
        (EffectSet::ENV, "env"),
        (EffectSet::ALLOC, "alloc"),
        (EffectSet::SECRET_USE, "secret_use"),
    ] {
        if e.contains(mask) {
            out.push(name.to_string());
        }
    }
    for (i, n) in names.iter().enumerate() {
        if (i as u32) < EffectClass::USER_CAPACITY {
            let bit = EffectSet(
                1 << (EffectClass::BUILTIN_BITS as u64 + i as u64),
            );
            if e.contains(bit) {
                out.push(n.clone());
            }
        }
    }
    out
}

// ================= cross-actor causality =========================

/// `@effects(causes: {…})`: the classes this fn can cause ANYWHERE
/// in the system, following bus edges. A publish to subject `T`
/// causes everything `T`'s subscribers do — the reasoning the call
/// graph alone cannot reach, available because the message graph is
/// declared.
pub fn causal_effects(
    summary: &AllocSummary,
    graph: &BusGraph,
    key: &FnKey,
    ffi: &BTreeSet<String>,
) -> (EffectSet, Vec<String>) {
    let mut acc = infer_effects(summary, key, ffi);
    let mut via: Vec<String> = Vec::new();
    // Subjects this fn (transitively) publishes to.
    let mut subjects: BTreeSet<String> = BTreeSet::new();
    collect_published_subjects(summary, key, &mut subjects, &mut BTreeSet::new());
    for subj in &subjects {
        let Some(info) = graph.subjects.get(subj) else { continue };
        for sub in &info.subscribers {
            let skey = FnKey::method(sub.locus.clone(), sub.handler.clone());
            let sub_eff = infer_effects(summary, &skey, ffi);
            if !sub_eff.is_unclassified() && sub_eff.0 != 0 {
                via.push(format!(
                    "`{}` -> subject `{}` -> `{}::{}`",
                    key.display(),
                    subj,
                    sub.locus,
                    sub.handler
                ));
            }
            acc = acc.union(sub_eff);
        }
    }
    (acc, via)
}

fn collect_published_subjects(
    summary: &AllocSummary,
    key: &FnKey,
    out: &mut BTreeSet<String>,
    seen: &mut BTreeSet<FnKey>,
) {
    if !seen.insert(key.clone()) {
        return;
    }
    let Some(fs) = summary.fns.get(key) else { return };
    for site in &fs.effect_sites {
        if let alloc_summary::EffectSiteKind::Publish(Some(s)) = &site.kind {
            out.insert(s.clone());
        }
    }
    for edge in &fs.calls {
        if let Callee::Resolved(k) = &edge.callee {
            collect_published_subjects(summary, k, out, seen);
        }
    }
}

/// `@effects(causes: {…})` — check the declared causal set against
/// what the fn can actually cause through bus edges.
pub fn causes_diags(
    programs: &[&Program],
    graph: &BusGraph,
) -> Vec<Diag> {
    // The seed's user effect-class table, so an excess class renders
    // as `money` rather than as nothing at all.
    let names: Vec<String> = programs
        .iter()
        .map(|p| &p.effect_names)
        .find(|n| !n.is_empty())
        .cloned()
        .unwrap_or_default();
    let mut roots: Vec<(FnKey, Vec<EffectClass>, Span)> = Vec::new();
    for p in programs {
        for item in &p.items {
            let mut push = |key: FnKey, fd: &FnDecl| {
                for a in &fd.effects {
                    if let EffectAssert::Causes(cs) = a {
                        roots.push((key.clone(), cs.clone(), fd.name.span));
                    }
                }
            };
            match item {
                TopDecl::Fn(fd) => {
                    push(FnKey::free_fn(fd.name.name.clone()), fd)
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            push(
                                FnKey::method(
                                    l.name.name.clone(),
                                    fd.name.name.clone(),
                                ),
                                fd,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if roots.is_empty() {
        return Vec::new();
    }
    let summary = alloc_summary::summarize_programs(programs);
    let ffi: BTreeSet<String> = programs
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| match i {
            TopDecl::Fn(f) if f.ffi.is_some() => Some(f.name.name.clone()),
            _ => None,
        })
        .collect();
    let defs: Vec<Option<Vec<EffectClass>>> = programs
        .iter()
        .map(|p| &p.effect_defs)
        .find(|d| !d.is_empty())
        .cloned()
        .unwrap_or_default();
    let mut diags = Vec::new();
    for (key, declared, span) in &roots {
        let (actual, via) = causal_effects(&summary, graph, key, &ffi);
        let mut allowed = EffectSet::PURE;
        for c in declared {
            allowed = allowed.union(class_mask_with(*c, &defs));
        }
        // Only report classes reached THROUGH the bus (the causal
        // surface); direct effects are the `none:` form's job.
        let direct = infer_effects(&summary, key, &ffi);
        let caused_only = EffectSet(actual.0 & !direct.0);
        let excess = EffectSet(caused_only.0 & !allowed.0);
        if excess.0 != 0 {
            diags.push(Diag::ty(
                *span,
                format!(
                    "declared causal set violated: `{}` can transitively \
                     cause {} through the bus, which its \
                     `@effects(causes: …)` does not declare.{} Add the \
                     class to the declaration, or route the publish to a \
                     subject whose subscribers don't perform it.",
                    key.display(),
                    render_effects_named(excess, &names).join(", "),
                    if via.is_empty() {
                        String::new()
                    } else {
                        format!(" Path: {}.", via.join("; "))
                    }
                ),
            ));
        }
    }
    diags
}


fn class_mask(c: EffectClass) -> EffectSet {
    match c {
        EffectClass::Syscall => EffectSet::SYSCALL,
        EffectClass::Block => EffectSet::BLOCK,
        EffectClass::Publish => EffectSet::PUBLISH,
        EffectClass::Time => EffectSet::TIME,
        EffectClass::Entropy => EffectSet::ENTROPY,
        EffectClass::Env => EffectSet::ENV,
        EffectClass::Alloc => EffectSet::ALLOC,
        EffectClass::SecretUse => EffectSet::SECRET_USE,
        EffectClass::Ffi => EffectSet::SYSCALL,
        EffectClass::Spawn | EffectClass::Recursion => EffectSet::PURE,
        // #345: user classes occupy the bits above the built-ins.
        // Saturating past the ceiling would be UNSOUND in the open
        // direction: a class with no bit unions as PURE, so
        // `@effects(none: {overflowed})` certifies a fn that calls a
        // declared source of it. The declaration is rejected up front
        // (`check_effect_capacity`) so this arm is unreachable for a
        // program that typechecks; it stays saturating rather than
        // panicking for callers that analyse un-checked ASTs.
        EffectClass::User(i) => {
            if (i as u32) < EffectClass::USER_CAPACITY {
                EffectSet(1 << (EffectClass::BUILTIN_BITS as u64 + i as u64))
            } else {
                EffectSet::PURE
            }
        }
    }
}

/// #354: a class's mask, following `effect io = { a, b };` definitions.
///
/// A COMPOSED class has no bit of its own — its mask is the union of
/// its members'. That single fact gives both useful directions with no
/// new analysis: forbidding `io` tests against `syscall|block` and so
/// catches either, and a fn that reaches a syscall has that bit set and
/// therefore satisfies "carries `io`".
///
/// `defs` is index-parallel to the program's `effect_names`. Recursion
/// is depth-bounded by `seen`, which also rejects a definition cycle by
/// resolving it to PURE rather than looping — the cycle itself is
/// diagnosed separately, at declaration.
pub fn class_mask_with(
    c: EffectClass,
    defs: &[Option<Vec<EffectClass>>],
) -> EffectSet {
    fn go(
        c: EffectClass,
        defs: &[Option<Vec<EffectClass>>],
        seen: &mut Vec<u16>,
    ) -> EffectSet {
        if let EffectClass::User(i) = c {
            if let Some(Some(members)) = defs.get(i as usize) {
                if seen.contains(&i) {
                    return EffectSet::PURE; // cycle; diagnosed at decl
                }
                seen.push(i);
                let mut acc = EffectSet::PURE;
                for m in members {
                    acc = acc.union(go(*m, defs, seen));
                }
                seen.pop();
                return acc;
            }
        }
        class_mask(c)
    }
    go(c, defs, &mut Vec::new())
}

// ===================== @supervised ==============================

/// `@supervised` on a locus: every locus in its subtree (its params
/// children, transitively) must have a declared failure policy —
/// an `on_failure` handler somewhere up the tree. Supervision
/// coverage, checked.
pub fn supervised_diags(programs: &[&Program]) -> Vec<Diag> {
    // locus name -> (has on_failure, child locus type names, span)
    let mut info: BTreeMap<String, (bool, Vec<String>, Span)> =
        BTreeMap::new();
    let mut supervised_roots: Vec<(String, Span)> = Vec::new();
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(l) = item else { continue };
            let has_failure = l
                .members
                .iter()
                .any(|m| matches!(m, LocusMember::Failure(_)));
            let mut children = Vec::new();
            for m in &l.members {
                if let LocusMember::Params(pb) = m {
                    for prm in &pb.params {
                        if let Some(TypeExpr::Named { path, .. }) = &prm.ty {
                            if path.segments.len() == 1 {
                                children
                                    .push(path.segments[0].name.clone());
                            }
                        }
                    }
                }
            }
            info.insert(
                l.name.name.clone(),
                (has_failure, children, l.name.span),
            );
            if l.supervised {
                supervised_roots.push((l.name.name.clone(), l.name.span));
            }
        }
    }
    let mut diags = Vec::new();
    for (root, span) in &supervised_roots {
        let mut uncovered: Vec<String> = Vec::new();
        let mut stack = vec![(root.clone(), false)];
        let mut seen = BTreeSet::new();
        while let Some((name, covered_above)) = stack.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some((has_failure, children, _)) = info.get(&name) else {
                continue;
            };
            let covered = covered_above || *has_failure;
            // A locus with children but no policy anywhere above it
            // is the uncovered case — a failure there has nowhere to
            // go.
            if !covered && !children.is_empty() {
                uncovered.push(name.clone());
            }
            for c in children {
                stack.push((c.clone(), covered));
            }
        }
        if !uncovered.is_empty() {
            diags.push(Diag::ty(
                *span,
                format!(
                    "`@supervised` violated: `{}`'s subtree has loci with \
                     no failure policy in scope — {}. Every locus under a \
                     supervised root needs an `on_failure` handler on \
                     itself or an ancestor, or a failure there has nowhere \
                     to go. Add `on_failure` to the root to cover the \
                     whole tree.",
                    root,
                    uncovered.join(", ")
                ),
            ));
        }
    }
    diags
}

// ===================== @secret taint =============================

/// The `@secret` **lint** (GH #436).
///
/// Emits WARNINGS for the leaks it can actually see: a `@secret`
/// param mentioned in a publish or a log / file sink in the same fn
/// body. Those are true positives worth reporting and it keeps
/// reporting them.
///
/// It is deliberately NOT a certificate, and #436 stopped it claiming
/// to be one. "Must not reach a sink" is a whole-world property, and
/// this is a local walker over one body: it does not follow calls, it
/// does not track aliases, and the fragment it walks is narrower
/// still (`then` branches but not `else`, no `match`, no `let`, no
/// assignment). Everything outside that fragment vanishes from the
/// result rather than surfacing as `uncertified` — which is the
/// fail-open shape the rest of the stack exists to avoid.
///
/// The traversal is deliberately left narrow here. Widening it would
/// newly fail programs that compile today, and a lint that grows
/// teeth in a point release is a userspace break even when every new
/// finding is a real bug. The widened walk lives in
/// [`secret_taint_strict`] behind `--strict-secret`.
///
/// For a guarantee rather than a lint, confine the secret to a
/// `@sealed` locus, classify its one operation with an effect class,
/// and state the law in `claims` — see `spec/verification.md`
/// § "Secrets".
pub fn secret_taint_diags(programs: &[&Program]) -> Vec<Diag> {
    let mut diags = Vec::new();
    for p in programs {
        for item in &p.items {
            let fns: Vec<&FnDecl> = match item {
                TopDecl::Fn(fd) => vec![fd],
                TopDecl::Locus(l) => l
                    .members
                    .iter()
                    .filter_map(|m| match m {
                        LocusMember::Fn(fd) => Some(fd),
                        _ => None,
                    })
                    .collect(),
                _ => continue,
            };
            for fd in fns {
                let secrets: BTreeSet<String> = fd
                    .params
                    .iter()
                    .filter(|pm| pm.secret)
                    .map(|pm| pm.name.name.clone())
                    .collect();
                if secrets.is_empty() {
                    continue;
                }
                check_secret_block(&fd.body, &secrets, &mut diags);
            }
        }
    }
    diags
}

fn check_secret_block(
    b: &Block,
    secrets: &BTreeSet<String>,
    diags: &mut Vec<Diag>,
) {
    for st in &b.stmts {
        match st {
            Stmt::Send { value, span, .. } => {
                if expr_mentions(value, secrets) {
                    diags.push(Diag::warn(
                        *span,
                        "lint: a `@secret` value reaches a bus publish — \
                         the payload crosses a process boundary and may \
                         be observed or logged downstream. Send a derived \
                         non-secret (an id, a hash) instead. This lint \
                         sees one fn body and follows no calls; for a \
                         checked guarantee, confine the secret to a \
                         `@sealed` locus and claim over its effect class."
                            .to_string(),
                    ));
                }
            }
            Stmt::Expr(e) => {
                if let Expr::Call { callee, args, span, .. } = e {
                    let is_sink = match callee.as_ref() {
                        Expr::Ident(i) => {
                            i.name == "println" || i.name == "print"
                        }
                        Expr::Path(p) => p
                            .segments
                            .last()
                            .map(|s| {
                                s.name == "write_bytes"
                                    || s.name == "write_file"
                                    || s.name == "write_line"
                            })
                            .unwrap_or(false),
                        _ => false,
                    };
                    if is_sink
                        && args.iter().any(|a| expr_mentions(a, secrets))
                    {
                        diags.push(Diag::warn(
                            *span,
                            "lint: a `@secret` value reaches a log / file \
                             sink — secrets must not be written where they \
                             can be read back. Log an identifier instead. \
                             This lint sees one fn body and follows no \
                             calls; for a checked guarantee, confine the \
                             secret to a `@sealed` locus and claim over \
                             its effect class."
                                .to_string(),
                        ));
                    }
                }
            }
            Stmt::If(i) => check_secret_block(&i.then_block, secrets, diags),
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                check_secret_block(body, secrets, diags)
            }
            _ => {}
        }
    }
}

/// The `@secret` **strict** pass (GH #436) — `hale check --strict-secret`.
///
/// Opt-in, because it newly fails programs that compile today. It is
/// the same idea as the lint with the two holes closed:
///
/// 1. **Every control-flow form is walked**, not just `then` branches
///    — `else`, `else if`, `match` arms, loops, bare blocks. Moving a
///    publish from `then` to `else` no longer hides it.
/// 2. **Aliases propagate.** `let alias = token;` taints `alias`, so
///    a one-line rename no longer launders.
///
/// And, unlike the lint, it **fails closed**. A tainted value that
/// reaches anything this walker cannot model — a call to a fn whose
/// body it does not follow, a field store, a return — is reported as
/// `uncertified` rather than passing silently. That will be loud on
/// real code, which is the honest signal: this is still one body's
/// worth of reasoning, and the whole-world guarantee lives in
/// `@sealed` + effect classes + `claims`, not here.
pub fn secret_taint_strict(programs: &[&Program]) -> Vec<Diag> {
    let mut diags = Vec::new();
    for p in programs {
        for item in &p.items {
            let fns: Vec<&FnDecl> = match item {
                TopDecl::Fn(fd) => vec![fd],
                TopDecl::Locus(l) => l
                    .members
                    .iter()
                    .filter_map(|m| match m {
                        LocusMember::Fn(fd) => Some(fd),
                        _ => None,
                    })
                    .collect(),
                _ => continue,
            };
            for fd in fns {
                let mut tainted: BTreeSet<String> = fd
                    .params
                    .iter()
                    .filter(|pm| pm.secret)
                    .map(|pm| pm.name.name.clone())
                    .collect();
                if tainted.is_empty() {
                    continue;
                }
                strict_block(&fd.body, &mut tainted, &mut diags);
            }
        }
    }
    diags
}

fn strict_sink_name(callee: &Expr) -> bool {
    match callee {
        Expr::Ident(i) => i.name == "println" || i.name == "print",
        Expr::Path(p) => p
            .segments
            .last()
            .map(|s| {
                s.name == "write_bytes"
                    || s.name == "write_file"
                    || s.name == "write_line"
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// Report every tainted expression that escapes into something this
/// walker cannot follow. `what` names the shape for the diagnostic.
fn strict_escape(
    e: &Expr,
    tainted: &BTreeSet<String>,
    what: &str,
    span: Span,
    diags: &mut Vec<Diag>,
) {
    if !expr_mentions(e, tainted) {
        return;
    }
    diags.push(Diag::ty(
        span,
        format!(
            "uncertified: a `@secret` value reaches {what}, which this \
             check does not follow — it cannot certify that the secret \
             is contained. Confine it to a `@sealed` locus and let a \
             claim over its effect class carry the guarantee."
        ),
    ));
}

fn strict_block(
    b: &Block,
    tainted: &mut BTreeSet<String>,
    diags: &mut Vec<Diag>,
) {
    for st in &b.stmts {
        match st {
            Stmt::Send { value, span, .. } => {
                if expr_mentions(value, tainted) {
                    diags.push(Diag::ty(
                        *span,
                        "a `@secret` value reaches a bus publish — the \
                         payload crosses a process boundary and may be \
                         observed or logged downstream. Send a derived \
                         non-secret (an id, a hash) instead."
                            .to_string(),
                    ));
                }
            }
            // The alias hole: `let alias = token;` taints `alias`.
            Stmt::Let { name, value, span, .. } => {
                if expr_mentions(value, tainted) {
                    // A call in the initializer is opaque to us; the
                    // binding is tainted AND the call is unmodelled.
                    if matches!(value, Expr::Call { .. }) {
                        strict_escape(
                            value, tainted, "a call this check does not \
                            follow", *span, diags,
                        );
                    }
                    tainted.insert(name.name.clone());
                }
            }
            Stmt::Assign { value, span, .. } => {
                strict_escape(value, tainted, "a stored location", *span, diags)
            }
            Stmt::Return(Some(e), span) => {
                strict_escape(e, tainted, "this fn's return", *span, diags)
            }
            Stmt::Expr(e) => {
                if let Expr::Call { callee, args, span, .. } = e {
                    if strict_sink_name(callee) {
                        if args.iter().any(|a| expr_mentions(a, tainted)) {
                            diags.push(Diag::ty(
                                *span,
                                "a `@secret` value reaches a log / file \
                                 sink — secrets must not be written where \
                                 they can be read back. Log an identifier \
                                 instead."
                                    .to_string(),
                            ));
                        }
                    } else {
                        for a in args {
                            strict_escape(
                                a,
                                tainted,
                                "a call this check does not follow",
                                *span,
                                diags,
                            );
                        }
                    }
                }
            }
            // A destructuring bind taints every name it introduces.
            // Position-precise would be better; conservative is the
            // safe direction and this is the walk that must not miss.
            Stmt::LetTuple { names, value, span, .. } => {
                if expr_mentions(value, tainted) {
                    if matches!(value, Expr::Call { .. }) {
                        strict_escape(
                            value,
                            tainted,
                            "a call this check does not follow",
                            *span,
                            diags,
                        );
                    }
                    for n in names {
                        tainted.insert(n.name.clone());
                    }
                }
            }
            // The traversal hole: every branch, not just `then`.
            Stmt::If(i) => strict_if(i, tainted, diags),
            Stmt::Match(m) => {
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Block(blk) => {
                            strict_block(blk, tainted, diags)
                        }
                        // An EXPRESSION arm was skipped entirely, so
                        // `match k { 0 -> print(secret), … }` walked
                        // straight past.
                        MatchArmBody::Expr(e) => strict_escape(
                            e,
                            tainted,
                            "a `match` arm expression",
                            m.span,
                            diags,
                        ),
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                strict_block(body, tainted, diags)
            }
            Stmt::Block(blk) => strict_block(blk, tainted, diags),
            _ => {}
        }
    }
}

fn strict_if(
    i: &IfStmt,
    tainted: &mut BTreeSet<String>,
    diags: &mut Vec<Diag>,
) {
    strict_block(&i.then_block, tainted, diags);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => strict_block(b, tainted, diags),
        Some(ElseBranch::ElseIf(inner)) => strict_if(inner, tainted, diags),
        None => {}
    }
}

/// Does this expression mention a tainted name?
///
/// EXHAUSTIVE by construction: the `match` has no `_` arm, so adding
/// an `Expr` variant fails the build here rather than silently
/// creating a laundering route. The original had a catch-all and
/// listed six forms, so a tuple, an index, a block tail, an `or`
/// substitute — anything else — carried a secret straight past both
/// the lint and the strict walk.
fn expr_mentions(e: &Expr, names: &BTreeSet<String>) -> bool {
    let any = |es: &[Expr]| es.iter().any(|x| expr_mentions(x, names));
    match e {
        Expr::Ident(i) => names.contains(&i.name),
        Expr::Binary { left, right, .. } => {
            expr_mentions(left, names) || expr_mentions(right, names)
        }
        Expr::Unary { operand, .. } => expr_mentions(operand, names),
        Expr::Call { callee, args, .. } => {
            expr_mentions(callee, names) || any(args)
        }
        Expr::Struct { inits, .. } => {
            inits.iter().any(|si| expr_mentions(&si.value, names))
        }
        Expr::Field { receiver, .. } => expr_mentions(receiver, names),
        Expr::Path2 { receiver, .. } => expr_mentions(receiver, names),
        Expr::Tuple(items, _) | Expr::Array(items, _) => any(items),
        Expr::ArrayRepeat { val, .. } => expr_mentions(val, names),
        Expr::Index { receiver, index, .. } => {
            expr_mentions(receiver, names) || expr_mentions(index, names)
        }
        Expr::Range { lo, hi, .. } => {
            expr_mentions(lo, names) || expr_mentions(hi, names)
        }
        Expr::Approx { left, right, tolerance, .. } => {
            expr_mentions(left, names)
                || expr_mentions(right, names)
                || expr_mentions(tolerance, names)
        }
        Expr::If(i) => stmt_mentions(&Stmt::If((**i).clone()), names),
        Expr::Match(m) => {
            stmt_mentions(&Stmt::Match((**m).clone()), names)
        }
        Expr::Block(b) => block_mentions(b, names),
        Expr::Or { inner, disposition, .. } => {
            expr_mentions(inner, names)
                || disposition_mentions(disposition, names)
        }
        Expr::Sum(body, _) | Expr::Prod(body, _) => {
            expr_mentions(body, names)
        }
        Expr::Literal(..) | Expr::KwSelf(_) | Expr::Path(_) => false,
    }
}

/// A block mentions a name if any statement or its tail does.
fn block_mentions(b: &Block, names: &BTreeSet<String>) -> bool {
    b.stmts.iter().any(|st| stmt_mentions(st, names))
}

fn stmt_mentions(st: &Stmt, names: &BTreeSet<String>) -> bool {
    match st {
        Stmt::Let { value, .. } => expr_mentions(value, names),
        Stmt::LetTuple { value, .. } => expr_mentions(value, names),
        Stmt::Assign { value, .. } => expr_mentions(value, names),
        Stmt::Return(Some(e), _) => expr_mentions(e, names),
        Stmt::Expr(e) => expr_mentions(e, names),
        Stmt::Send { value, .. } => expr_mentions(value, names),
        Stmt::If(i) => {
            expr_mentions(&i.cond, names)
                || block_mentions(&i.then_block, names)
                || match i.else_block.as_deref() {
                    Some(ElseBranch::Else(b)) => block_mentions(b, names),
                    Some(ElseBranch::ElseIf(inner)) => {
                        stmt_mentions(&Stmt::If((*inner).clone()), names)
                    }
                    None => false,
                }
        }
        Stmt::Match(m) => m.arms.iter().any(|a| match &a.body {
            MatchArmBody::Expr(x) => expr_mentions(x, names),
            MatchArmBody::Block(b) => block_mentions(b, names),
        }),
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            block_mentions(body, names)
        }
        Stmt::Block(b) => block_mentions(b, names),
        _ => false,
    }
}

fn disposition_mentions(
    d: &OrDisposition,
    names: &BTreeSet<String>,
) -> bool {
    // Exhaustive for the same reason as `expr_mentions`.
    match d {
        OrDisposition::Substitute(e) => expr_mentions(e, names),
        // `or fail <payload>` carries a value out of the fn.
        OrDisposition::Fail(payload, _) => expr_mentions(payload, names),
        OrDisposition::Raise(_)
        | OrDisposition::Discard(_)
        | OrDisposition::Wait(_) => false,
    }
}

// ===================== symbolic cost =============================

/// A structural cost expression: a constant plus per-loop terms in
/// the loop's bound. NOT WCET (no microarchitecture) — the
/// first-filter shape hard-real-time triage needs, and only
/// meaningful for a fn already proven bounded.
pub fn cost_expression(summary: &AllocSummary, key: &FnKey) -> String {
    let Some(fs) = summary.fns.get(key) else {
        return "unknown".to_string();
    };
    let base = 1 + fs.sites.len() as u64 + fs.calls.len() as u64;
    let loops = fs.loops.len();
    if loops == 0 {
        format!("O(1) — ~{} structural steps", base)
    } else {
        format!(
            "O(n^{}) — ~{} steps × {} nested loop level(s); bound is \
             structural, not timing",
            loops, base, loops
        )
    }
}

/// RFC #330 — `@effects(depends: {A, B})` on a locus.
///
/// The backward dual of `causes:`. `causes:` walks the bus graph
/// FORWARD because a call graph stops at a publish; nothing walked it
/// the other way, so an independence claim between two parts of a bus
/// graph was unenforceable. A dependence routed through one
/// republishing intermediary is invisible in every declaration on the
/// depending locus — its `bus {}` block names only the innocent
/// subject it directly subscribes to.
///
/// `depends:` is a COMPLETE declaration, like `publish:` and
/// `causes:`: every subject that can transitively reach any of the
/// locus's handlers must be named. Omitting one is the error.
///
/// Reachability, not dataflow. The claim is "no value from subject S
/// can reach this locus at all", which is the conservative form — a
/// locus that subscribes to a laundered republish of S depends on S
/// whether or not it reads the field.
pub fn depends_diags(programs: &[&Program], graph: &BusGraph) -> Vec<Diag> {
    // locus -> subjects it directly subscribes to
    let mut subs_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    // subject -> loci that publish it
    let mut pubs_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (subj, info) in &graph.subjects {
        for s in &info.subscribers {
            subs_of.entry(s.locus.as_str()).or_default().push(subj.as_str());
        }
        for p in &info.publishers {
            pubs_of.entry(subj.as_str()).or_default().push(p.locus.as_str());
        }
    }

    // #340: forms carrying a `sync` discipline. A locus holding one
    // is an input channel with no bus edge — another pool writes it,
    // this locus reads it, and nothing in the message graph records
    // that. Inferred from the form's own declaration; no annotation.
    let sync_forms: BTreeSet<String> = programs
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| match i {
            TopDecl::Locus(l)
                if l.form.as_ref().is_some_and(|f| {
                    f.args.iter().any(|a| a.name.name == "sync")
                }) =>
            {
                Some(l.name.name.clone())
            }
            _ => None,
        })
        .collect();

    let mut diags = Vec::new();
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(ld) = item else { continue };
            let Some(dep) = &ld.depends else { continue };
            for m in &ld.members {
                let LocusMember::Params(pb) = m else { continue };
                for prm in &pb.params {
                    let Some(TypeExpr::Named { path, .. }) = &prm.ty else {
                        continue;
                    };
                    let Some(seg) = path.segments.last() else { continue };
                    if !sync_forms.contains(&seg.name) {
                        continue;
                    }
                    diags.push(Diag::ty(
                        dep.span,
                        format!(
                            "declared dependency set is incomplete: \
                             `{}` holds `{}` as `{}`, a form carrying a \
                             `sync` discipline — shared state another \
                             pool can write. That is an input channel \
                             outside the bus graph, and `depends:` closes \
                             over the message graph only.",
                            ld.name.name, seg.name, prm.name.name
                        ),
                    ));
                }
            }

            // Backward BFS from this locus, remembering how each
            // subject was reached so the diagnostic can name the path
            // rather than only the verdict.
            let mut seen_subj: BTreeMap<&str, Option<(&str, &str)>> =
                BTreeMap::new(); // subject -> (via locus, from subject)
            let mut seen_locus: BTreeSet<&str> = BTreeSet::new();
            seen_locus.insert(ld.name.name.as_str());
            let mut queue: Vec<(&str, Option<(&str, &str)>)> = subs_of
                .get(ld.name.name.as_str())
                .map(|v| v.iter().map(|s| (*s, None)).collect())
                .unwrap_or_default();

            while let Some((subj, via)) = queue.pop() {
                if seen_subj.contains_key(subj) {
                    continue;
                }
                seen_subj.insert(subj, via);
                for pl in pubs_of.get(subj).into_iter().flatten() {
                    if !seen_locus.insert(*pl) {
                        continue;
                    }
                    for up in subs_of.get(*pl).into_iter().flatten() {
                        queue.push((*up, Some((*pl, subj))));
                    }
                }
            }

            for (subj, via) in &seen_subj {
                if dep
                    .subjects
                    .iter()
                    .any(|d| crate::effects::topic_ref_matches(d, subj))
                {
                    continue;
                }
                let path = match via {
                    Some((locus, into)) => format!(
                        " Path: subject `{}` -> `{}` -> subject `{}` -> `{}`.",
                        pretty(subj),
                        pretty(locus),
                        pretty(into),
                        ld.name.name
                    ),
                    None => format!(
                        " It is subscribed directly by `{}`.",
                        ld.name.name
                    ),
                };
                diags.push(Diag::ty(
                    dep.span,
                    format!(
                        "declared dependency set violated: `{}` can \
                         transitively depend on `{}` through the bus, which \
                         its `@effects(depends: …)` does not declare.{} Add \
                         the subject to the set, or route the input through \
                         a subject this locus doesn't reach.",
                        ld.name.name,
                        pretty(subj),
                        path
                    ),
                ));
            }
        }
    }
    diags
}

/// Merged cross-seed symbols reach here mangled
/// (`__lib_lib_relay_main_Recalled`). Show the author the name they
/// wrote, not the resolver's.
fn pretty(sym: &str) -> String {
    match sym.strip_prefix("__lib_") {
        Some(rest) => rest.rsplit('_').next().unwrap_or(rest).to_string(),
        None => sym.to_string(),
    }
}
