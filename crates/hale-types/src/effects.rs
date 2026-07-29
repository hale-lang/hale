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
    if roots.is_empty() {
        return Vec::new();
    }

    let summary = alloc_summary::summarize_programs(programs);
    let ffi = ffi_names(programs);
    let mut diags = Vec::new();
    for (key, asserts, span) in &roots {
        for a in asserts {
            match a {
                EffectAssert::NoBlock => {
                    check_leaf(
                        &summary, key, *span, "@no_block", &mut diags,
                        &mut |probe| match probe {
                            Probe::Unresolved(name, _) => {
                                blocking_leaf(name).map(|why| {
                                    format!("{} — {}", name, why)
                                })
                            }
                            Probe::Site(_) | Probe::Resolved(..) => None,
                        },
                        "Move the blocking call off this path (a reader \
                         locus on its own pool, or a bus message that \
                         triggers the work), or drop `@no_block`.",
                    );
                }
                EffectAssert::NoFfi => {
                    check_leaf(
                        &summary, key, *span, "@no_ffi", &mut diags,
                        &mut |probe| match probe {
                            // An `@ffi` fn resolves (empty body, real
                            // summary entry), so it appears as a
                            // RESOLVED callee — not on the unresolved
                            // frontier. Match both: a bundle-local
                            // extern, and (defensively) an unresolved
                            // name that happens to be declared @ffi.
                            Probe::Resolved(k, _) => {
                                if k.locus.is_none()
                                    && ffi.contains(&k.fn_name)
                                {
                                    Some(format!(
                                        "{} is an `@ffi` fn",
                                        k.fn_name
                                    ))
                                } else {
                                    None
                                }
                            }
                            Probe::Unresolved(name, _) => {
                                let bare =
                                    name.rsplit("::").next().unwrap_or(name);
                                if ffi.contains(bare) || ffi.contains(*name) {
                                    Some(format!("{} is an `@ffi` fn", name))
                                } else {
                                    None
                                }
                            }
                            Probe::Site(_) => None,
                        },
                        "An `@ffi` call is a trust boundary the managed \
                         contract excludes — route the foreign work through \
                         a locus this fn doesn't reach, or drop `@no_ffi`.",
                    );
                }
                EffectAssert::NoRecursion => {
                    if let Some(cyc) = find_recursion(&summary, key) {
                        diags.push(Diag::ty(
                            *span,
                            format!(
                                "`@no_recursion` violated: `{}` reaches a \
                                 recursive cycle. {}. A recursive path has \
                                 no static stack bound — restructure to an \
                                 explicit worklist/loop, or drop \
                                 `@no_recursion`.",
                                key.display(),
                                cyc
                            ),
                        ));
                    }
                }
            }
        }
    }
    diags
}

/// Shared shape for the reachability assertions: find the first
/// witness chain to a matching leaf and report it.
fn check_leaf(
    summary: &AllocSummary,
    key: &FnKey,
    span: Span,
    label: &str,
    diags: &mut Vec<Diag>,
    pred: &mut dyn FnMut(&Probe<'_>) -> Option<String>,
    advice: &str,
) {
    let Some(path) = callgraph::witness_path(summary, key, pred) else {
        return;
    };
    let leaf_span = path.last().map(|s| s.span).unwrap_or(span);
    diags.push(Diag::ty(
        span,
        format!(
            "`{}` violated: `{}` reaches {}. {}",
            label,
            key.display(),
            chain(key, &path),
            advice
        ),
    ));
    diags.push(Diag::ty(
        leaf_span,
        format!("the {} violation is reached here", label),
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
