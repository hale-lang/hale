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
                    if k.locus.is_none() && ffi.contains(&k.fn_name) {
                        acc = acc.union(EffectSet::SYSCALL);
                    }
                    acc = acc.union(walk(summary, k, ffi, seen, steps));
                }
                Callee::Unresolved(name) => {
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
pub fn render_effects(e: EffectSet) -> Vec<String> {
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
    ] {
        if e.contains(mask) {
            out.push(name.to_string());
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
    let mut diags = Vec::new();
    for (key, declared, span) in &roots {
        let (actual, via) = causal_effects(&summary, graph, key, &ffi);
        let mut allowed = EffectSet::PURE;
        for c in declared {
            allowed = allowed.union(class_mask(*c));
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
                    render_effects(excess).join(", "),
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
        EffectClass::Ffi => EffectSet::SYSCALL,
        EffectClass::Spawn | EffectClass::Recursion => EffectSet::PURE,
    }
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

/// Coarse secret taint: a param declared `@secret` must not flow to
/// a publish or a log/stdout sink within the fn body. Parameter-
/// granular (not full value-flow), which is the honest reach — but
/// the choke-pointed sinks make even this catch the real mistake:
/// a key or token landing in a log line or on the bus.
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
                        diags.push(Diag::ty(
                            *span,
                            "a `@secret` value reaches a log / file sink — \
                             secrets must not be written where they can be \
                             read back. Log an identifier instead."
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

fn expr_mentions(e: &Expr, names: &BTreeSet<String>) -> bool {
    match e {
        Expr::Ident(i) => names.contains(&i.name),
        Expr::Binary { left, right, .. } => {
            expr_mentions(left, names) || expr_mentions(right, names)
        }
        Expr::Unary { operand, .. } => expr_mentions(operand, names),
        Expr::Call { args, .. } => {
            args.iter().any(|a| expr_mentions(a, names))
        }
        Expr::Struct { inits, .. } => {
            inits.iter().any(|si| expr_mentions(&si.value, names))
        }
        Expr::Field { receiver, .. } => expr_mentions(receiver, names),
        _ => false,
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

    let shared_loci: BTreeSet<String> = programs
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| match i {
            TopDecl::Locus(l) if l.shared => Some(l.name.name.clone()),
            _ => None,
        })
        .collect();

    let mut diags = Vec::new();
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(ld) = item else { continue };
            let Some(dep) = &ld.depends else { continue };

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

            // #333: a `@shared` locus is an input channel with no bus
            // edge — another pool writes it, this locus reads it, and
            // nothing in the message graph records that. Report it
            // rather than letting the closure claim completeness it
            // does not have.
            for m in &ld.members {
                let LocusMember::Params(pb) = m else { continue };
                for prm in &pb.params {
                    let Some(TypeExpr::Named { path, .. }) = &prm.ty else {
                        continue;
                    };
                    let Some(seg) = path.segments.last() else { continue };
                    if !shared_loci.contains(&seg.name) {
                        continue;
                    }
                    diags.push(Diag::ty(
                        dep.span,
                        format!(
                            "declared dependency set is incomplete: \
                             `{}` holds `@shared locus {}` as `{}`, which \
                             is an input channel outside the bus graph — \
                             another pool can write state this locus \
                             reads. `depends:` closes over the message \
                             graph only, so it cannot account for that \
                             route.",
                            ld.name.name, seg.name, prm.name.name
                        ),
                    ));
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
