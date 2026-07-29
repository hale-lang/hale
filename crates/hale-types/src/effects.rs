//! #265 (2026-07-29) — categoric effect assertions, phase 1.
//!
//! `@budget(alloc_per_call = N)` proved the shape: an opt-in
//! contract at a root the author cares about, inferred everywhere
//! else, enforced as a hard error over the resolved call graph.
//! This module generalizes it from "allocation count" to *effect
//! classes*, on the same substrate ([`crate::callgraph`]) — and
//! adds what the fixpoint never had: the diagnostic names the
//! **call chain to the violation**, not just the fn.
//!
//! Phase 1 ships the three assertions whose leaf sets are closed
//! and unambiguous:
//!
//!   * `@no_recursion` — call-graph acyclicity under the root (the
//!     static-stack precondition; also what makes a symbolic cost
//!     bound possible later).
//!   * `@no_ffi` — no `@ffi` fn transitively reachable ("pure
//!     managed Hale", the `forbid(unsafe)` analog).
//!   * `@no_block` — no blocking stdlib operation reachable. This
//!     is the contract an `async_io`-placed handler needs: a
//!     blocking call there stalls the pool's single worker (the
//!     Crumb batch-5 sleep bug, as a *checkable* property).
//!
//! Deliberately NOT here (see the issue's build order): `@no_panic`
//! is a different analysis (disposition coverage + index-op
//! selection, not leaf reachability); `@no_syscall` /
//! `@deterministic` wait on the full `lotus_*` frontier
//! classification (the `EffectSet` column in
//! [`crate::stdlib_surface`] is where that lands); the quantitative
//! layer (stack bytes, fan-out) and phase/placement-implied
//! contracts follow.

use std::collections::BTreeSet;

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::alloc_summary::{self, AllocSummary, FnKey};
use crate::callgraph::{self, Probe};

/// Blocking stdlib operations — the closed leaf set for
/// `@no_block`. These are the calls that park or block the calling
/// thread for an unbounded (or caller-controlled) time. On an
/// `async_io` pool the parking ones yield the worker; on every
/// other placement they hold it — and a handler asserting
/// `@no_block` is asserting it does neither.
///
/// Matched against the qualified path the alloc summary records for
/// an unresolved callee (`std::io::tcp::recv`), and against the
/// bare trailing name for method-call syntax on a stdlib handle
/// (`s.recv(...)`), the same two shapes `budget_check`'s recv set
/// matches.
fn blocking_leaf(name: &str) -> Option<&'static str> {
    let bare = name.rsplit("::").next().unwrap_or(name);
    let label = match bare {
        "sleep" if name.starts_with("std::time") || name == "sleep" => {
            "std::time::sleep blocks/parks for the full duration"
        }
        "recv" | "recv_bytes" | "recv_into" | "recv_with_source"
        | "recv_stamped_into" => {
            "a blocking receive — waits for data or the socket timeout"
        }
        "accept" | "accept_one" => {
            "accept blocks until a connection arrives"
        }
        "connect" => "connect blocks through the TCP/TLS handshake",
        "wait" | "try_wait" if name.contains("process") => {
            "waiting on a subprocess blocks"
        }
        "next" if name.contains("udp") || name.contains("Reader") => {
            "Reader.next() blocks/parks until a datagram arrives"
        }
        _ => return None,
    };
    Some(label)
}

/// Is this an `@ffi`-declared fn in the bundle?
fn ffi_names(programs: &[&Program]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in programs {
        for item in &p.items {
            if let TopDecl::Fn(f) = item {
                if f.ffi.is_some() {
                    out.insert(f.name.name.clone());
                }
            }
        }
    }
    out
}

/// Render `root -> hop -> hop [leaf]` for a witness chain.
fn chain(root: &FnKey, steps: &[callgraph::WitnessStep]) -> String {
    callgraph::render_witness(root, steps)
}

/// The public entry: check every effect assertion in `programs`.
/// Returns hard-error diagnostics (opt-in: you asked for the
/// contract, so a violation fails the build) — empty when every
/// contract holds.
/// GH #265: **placement-implied contracts** — the check that needs
/// no annotation at all. A locus placed `cooperative(pool = X) where
/// async_io` runs its methods on that pool's single worker; a
/// BLOCKING operation reachable from one of its handlers holds that
/// worker, so every other locus on the pool stalls behind it. The
/// placement IS the assertion, so the compiler enforces it without
/// the author writing `@no_block`.
///
/// Reported as a WARNING, not an error: the placement may be
/// deliberate (a lone locus on a pool it owns), and the escape hatch
/// is explicit — the diagnostic names both fixes. This is the
/// Crumb batch-5 bug (a JS handler's `await sleep(400)` holding the
/// engine pool) as a compile-time finding.
fn placement_implied_diags(
    programs: &[&Program],
    summary: &AllocSummary,
) -> Vec<Diag> {
    use crate::stdlib_surface::EffectSet;
    // main-locus params fields placed on an async_io pool → the locus
    // TYPE names whose methods must not block.
    let mut async_io_types: Vec<(String, String)> = Vec::new(); // (type, pool)
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(l) = item else { continue };
            if !l.is_main {
                continue;
            }
            let mut field_ty: std::collections::BTreeMap<String, String> =
                Default::default();
            for m in &l.members {
                if let LocusMember::Params(pb) = m {
                    for prm in &pb.params {
                        if let Some(TypeExpr::Named { path, .. }) = &prm.ty {
                            if path.segments.len() == 1 {
                                field_ty.insert(
                                    prm.name.name.clone(),
                                    path.segments[0].name.clone(),
                                );
                            }
                        }
                    }
                }
            }
            for m in &l.members {
                if let LocusMember::Placement(pb) = m {
                    for e in &pb.entries {
                        let is_async = e.constraints.iter().any(|c| {
                            matches!(c.kind, PlacementConstraint::AsyncIo)
                        });
                        if !is_async {
                            continue;
                        }
                        let PlacementSpec::Cooperative { pool } = &e.spec
                        else {
                            continue;
                        };
                        if let Some(t) = field_ty.get(&e.field.name) {
                            async_io_types.push((
                                t.clone(),
                                pool.as_ref()
                                    .map(|i| i.name.clone())
                                    .unwrap_or_else(|| "main".to_string()),
                            ));
                        }
                    }
                }
            }
        }
    }
    if async_io_types.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in programs {
        for item in &p.items {
            let TopDecl::Locus(l) = item else { continue };
            let Some((_, pool)) = async_io_types
                .iter()
                .find(|(t, _)| *t == l.name.name)
            else {
                continue;
            };
            for m in &l.members {
                let LocusMember::Fn(fd) = m else { continue };
                // An explicit assertion means the author is already
                // engaged with this — don't double-report.
                if !fd.effects.is_empty() {
                    continue;
                }
                let key =
                    FnKey::method(l.name.name.clone(), fd.name.name.clone());
                let mut pred = |probe: &Probe<'_>| match probe {
                    Probe::Unresolved(name, _) => {
                        let segs: Vec<&str> = name.split("::").collect();
                        let eff = crate::stdlib_surface::effects_for(&segs)?;
                        if eff.contains(EffectSet::BLOCK) {
                            Some(name.to_string())
                        } else {
                            None
                        }
                    }
                    Probe::Site(_) | Probe::Resolved(..) => None,
                };
                let Some(path) =
                    callgraph::witness_path(summary, &key, &mut pred)
                else {
                    continue;
                };
                out.push(Diag::warn(
                    fd.name.span,
                    format!(
                        "`{}` is placed on the async_io pool `{}`, whose \
                         single worker it shares — but it reaches {}. A \
                         blocking call here stalls every other locus on \
                         `{}` until it returns. Either move the blocking \
                         work to its own pool, or assert the intent with \
                         `@no_block` (to have the compiler enforce it) — \
                         the placement is the contract.",
                        key.display(),
                        pool,
                        chain(&key, &path),
                        pool
                    ),
                ));
            }
        }
    }
    out
}

pub fn effect_diags(programs: &[&Program]) -> Vec<Diag> {
    let mut roots: Vec<(FnKey, Vec<EffectAssert>, Span)> = Vec::new();
    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Fn(fd) if !fd.effects.is_empty() => {
                    roots.push((
                        FnKey::free_fn(fd.name.name.clone()),
                        fd.effects.clone(),
                        fd.name.span,
                    ));
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            if !fd.effects.is_empty() {
                                roots.push((
                                    FnKey::method(
                                        l.name.name.clone(),
                                        fd.name.name.clone(),
                                    ),
                                    fd.effects.clone(),
                                    fd.name.span,
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let summary = alloc_summary::summarize_programs(programs);
    // The placement-implied pass runs whether or not anything is
    // annotated — that is its point.
    let mut placement = placement_implied_diags(programs, &summary);
    if roots.is_empty() {
        return placement;
    }
    let ffi = ffi_names(programs);
    let mut diags = std::mem::take(&mut placement);
    for (key, asserts, span) in &roots {
        for a in asserts {
            match a {
                EffectAssert::Forbid(classes) => {
                    for c in classes {
                        check_class(
                            &summary, key, *span, *c, &ffi, &mut diags,
                        );
                    }
                }
                EffectAssert::PublishSet(allowed) => {
                    check_publish_set(&summary, key, *span, allowed, &mut diags);
                }
            }
        }
    }
    diags
}

/// GH #265 coherence: ONE dispatcher over the effect classes. Every
/// `@no_*` flag desugars to `Forbid([class])` at parse time, so this
/// is the single place a class's meaning is defined — no flag can
/// drift from the general form.
fn check_class(
    summary: &AllocSummary,
    key: &FnKey,
    span: Span,
    class: EffectClass,
    ffi: &BTreeSet<String>,
    diags: &mut Vec<Diag>,
) {
    use crate::stdlib_surface::EffectSet;
    // The three classes that are NOT stdlib-frontier queries.
    match class {
        EffectClass::Recursion => {
            if let Some(cyc) = find_recursion(summary, key) {
                diags.push(Diag::ty(
                    span,
                    format!(
                        "effect assertion violated: `{}` reaches a recursive \
                         cycle. {}. A recursive path has no static stack \
                         bound — restructure to an explicit worklist/loop, \
                         or drop the `recursion` assertion.",
                        key.display(),
                        cyc
                    ),
                ));
            }
            return;
        }
        EffectClass::Ffi => {
            let mut pred = |probe: &Probe<'_>| match probe {
                Probe::Resolved(k, _) => {
                    if k.locus.is_none() && ffi.contains(&k.fn_name) {
                        Some(format!("{} is an `@ffi` fn", k.fn_name))
                    } else {
                        None
                    }
                }
                Probe::Unresolved(name, _) => {
                    let bare = name.rsplit("::").next().unwrap_or(name);
                    if ffi.contains(bare) || ffi.contains(*name) {
                        Some(format!("{} is an `@ffi` fn", name))
                    } else {
                        None
                    }
                }
                Probe::Site(_) => None,
            };
            report(
                summary, key, span, "ffi", &mut pred, diags,
                "An `@ffi` call is a trust boundary the managed contract \
                 excludes — route the foreign work through a locus this fn \
                 doesn't reach.",
            );
            return;
        }
        EffectClass::Publish | EffectClass::Spawn => {
            // Syntactic effects: carried by `Topic <- v` / `Child { }`,
            // not by any call. Walk the effect-site vectors.
            if let Some((chain_s, site_span)) =
                find_effect_site(summary, key, |k| match (class, k) {
                    (
                        EffectClass::Publish,
                        alloc_summary::EffectSiteKind::Publish(subj),
                    ) => Some(match subj {
                        Some(s) => format!("publishes to `{}`", s),
                        None => "publishes".to_string(),
                    }),
                    (
                        EffectClass::Spawn,
                        alloc_summary::EffectSiteKind::Spawn(name),
                    ) => Some(format!("instantiates locus `{}`", name)),
                    _ => None,
                })
            {
                diags.push(Diag::ty(
                    span,
                    format!(
                        "effect assertion violated: `{}` reaches {}. Move \
                         the effect behind a locus this fn doesn't reach, or \
                         drop the `{}` assertion.",
                        key.display(),
                        chain_s,
                        class.as_str()
                    ),
                ));
                diags.push(Diag::ty(
                    site_span,
                    format!("the `{}` effect happens here", class.as_str()),
                ));
            }
            return;
        }
        _ => {}
    }
    // The frontier-query classes.
    let (mask, what) = match class {
        EffectClass::Syscall => (
            EffectSet::SYSCALL,
            "a syscall-class operation (filesystem, socket, process, \
             terminal, stdio)",
        ),
        EffectClass::Block => (
            EffectSet::BLOCK,
            "a blocking operation — it waits, holding its thread (or, on \
             an async_io pool, its worker's turn)",
        ),
        EffectClass::Time => (EffectSet::TIME, "a clock read"),
        EffectClass::Entropy => (EffectSet::ENTROPY, "an entropy read"),
        EffectClass::Env => (EffectSet::ENV, "an environment read"),
        _ => unreachable!("handled above"),
    };
    let mut pred = |probe: &Probe<'_>| match probe {
        Probe::Unresolved(name, _) => {
            let segs: Vec<&str> = name.split("::").collect();
            let eff = crate::stdlib_surface::effects_for(&segs)?;
            if eff.is_unclassified() {
                return Some(format!("{} is not yet effect-classified", name));
            }
            if (eff.0 & mask.0) != 0 {
                Some(format!("{} — {}", name, what))
            } else {
                None
            }
        }
        Probe::Site(_) | Probe::Resolved(..) => None,
    };
    report(
        summary, key, span, class.as_str(), &mut pred, diags,
        "Move the effect behind a locus this fn doesn't reach (the \
         reader/writer-locus shape), or pass the value in as a parameter.",
    );
}

/// `@effects(publish: {A, B})` — the allowed publish set. A publish
/// to a subject outside the set is a violation; the closed topic set
/// makes this exact. A computed (non-literal) subject can't be
/// proven in-set, so it is reported too.
fn check_publish_set(
    summary: &AllocSummary,
    key: &FnKey,
    span: Span,
    allowed: &[String],
    diags: &mut Vec<Diag>,
) {
    let found = find_effect_site(summary, key, |k| match k {
        alloc_summary::EffectSiteKind::Publish(Some(subj)) => {
            if allowed.iter().any(|a| a == subj) {
                None
            } else {
                Some(format!("publishes to `{}`", subj))
            }
        }
        alloc_summary::EffectSiteKind::Publish(None) => Some(
            "publishes to a computed subject (not provably in the \
             declared set)"
                .to_string(),
        ),
        _ => None,
    });
    if let Some((chain_s, site_span)) = found {
        diags.push(Diag::ty(
            span,
            format!(
                "declared publish set violated: `{}` reaches {}. The \
                 contract allows only {{{}}} — add the subject to the set, \
                 or route the publish through a locus this fn doesn't reach.",
                key.display(),
                chain_s,
                allowed.join(", ")
            ),
        ));
        diags.push(Diag::ty(site_span, "the publish happens here".to_string()));
    }
}

/// Walk the call graph looking for a matching SYNTACTIC effect site
/// (publish / spawn) in any reachable fn body, returning the rendered
/// witness chain and the site's span.
fn find_effect_site(
    summary: &AllocSummary,
    root: &FnKey,
    matches_kind: impl Fn(&alloc_summary::EffectSiteKind) -> Option<String> + Copy,
) -> Option<(String, Span)> {
    fn walk(
        summary: &AllocSummary,
        key: &FnKey,
        path: &mut Vec<(FnKey, Span, String)>,
        seen: &mut BTreeSet<FnKey>,
        steps: &mut u32,
        matches_kind: impl Fn(&alloc_summary::EffectSiteKind) -> Option<String> + Copy,
    ) -> Option<(Vec<callgraph::WitnessStep>, Span)> {
        let fs = summary.fns.get(key)?;
        for site in &fs.effect_sites {
            if let Some(label) = matches_kind(&site.kind) {
                return Some((
                    vec![callgraph::WitnessStep {
                        in_fn: key.clone(),
                        span: site.span,
                        label,
                    }],
                    site.span,
                ));
            }
        }
        for edge in &fs.calls {
            *steps += 1;
            if *steps > callgraph::MAX_STEPS {
                break;
            }
            if let alloc_summary::Callee::Resolved(callee) = &edge.callee {
                if !seen.insert(callee.clone()) {
                    continue;
                }
                if let Some((mut tail, sp)) =
                    walk(summary, callee, path, seen, steps, matches_kind)
                {
                    let mut out = vec![callgraph::WitnessStep {
                        in_fn: key.clone(),
                        span: edge.span,
                        label: callee.display(),
                    }];
                    out.append(&mut tail);
                    return Some((out, sp));
                }
            }
        }
        None
    }
    let mut path = Vec::new();
    let mut seen = BTreeSet::new();
    let mut steps = 0u32;
    let (steps_v, sp) =
        walk(summary, root, &mut path, &mut seen, &mut steps, matches_kind)?;
    Some((callgraph::render_witness(root, &steps_v), sp))
}

/// Emit the two-diagnostic report (root + leaf) for a witness chain.
fn report(
    summary: &AllocSummary,
    key: &FnKey,
    span: Span,
    class_name: &str,
    pred: &mut dyn FnMut(&Probe<'_>) -> Option<String>,
    diags: &mut Vec<Diag>,
    advice: &str,
) {
    let Some(path) = callgraph::witness_path(summary, key, pred) else {
        return;
    };
    let leaf_span = path.last().map(|s| s.span).unwrap_or(span);
    diags.push(Diag::ty(
        span,
        format!(
            "effect assertion violated: `{}` must not reach `{}`, but \
             reaches {}. {}",
            key.display(),
            class_name,
            chain(key, &path),
            advice
        ),
    ));
    diags.push(Diag::ty(
        leaf_span,
        format!("the `{}` effect is reached here", class_name),
    ));
}

/// Detect a cycle reachable from `root` (including a self-call),
/// naming the fn that closes it. Uses the summary's resolved call
/// edges directly — recursion is a property of the graph, not of a
/// leaf predicate.
fn find_recursion(summary: &AllocSummary, root: &FnKey) -> Option<String> {
    fn walk(
        summary: &AllocSummary,
        key: &FnKey,
        path: &mut Vec<FnKey>,
        seen: &mut BTreeSet<FnKey>,
        steps: &mut u32,
    ) -> Option<String> {
        let fs = summary.fns.get(key)?;
        path.push(key.clone());
        for edge in &fs.calls {
            *steps += 1;
            if *steps > callgraph::MAX_STEPS {
                break;
            }
            if let alloc_summary::Callee::Resolved(callee) = &edge.callee {
                if path.contains(callee) {
                    let cyc: Vec<String> = path
                        .iter()
                        .skip_while(|k| *k != callee)
                        .map(|k| k.display())
                        .collect();
                    path.pop();
                    return Some(format!(
                        "cycle: {} -> {}",
                        cyc.join(" -> "),
                        callee.display()
                    ));
                }
                if seen.insert(callee.clone()) {
                    if let Some(found) =
                        walk(summary, callee, path, seen, steps)
                    {
                        path.pop();
                        return Some(found);
                    }
                }
            }
        }
        path.pop();
        None
    }
    let mut path = Vec::new();
    let mut seen = BTreeSet::new();
    let mut steps = 0u32;
    walk(summary, root, &mut path, &mut seen, &mut steps)
}
