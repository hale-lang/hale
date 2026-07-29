//! R1 (2026-07-29) — the shared call-graph walking substrate.
//!
//! `hale-types` grew four independent call-graph traversals
//! (`alloc_summary`'s fixpoint, `budget_check`'s counting DFS,
//! `purity`'s worklist, check.rs's blocking finder), none of which
//! carried a witness path. Issue #265 (categoric effect assertions)
//! needs exactly one engine: path-carrying reachability over the
//! resolved call graph, parameterized by what a leaf contributes.
//! This module is that engine, extracted from `budget_check`'s DFS
//! (its semantics are the reference: ancestor-stack recursion
//! detection, loop-context propagation, per-reaching-path diamond
//! counting, a step cap) — `budget_check` is the first ported
//! customer, and #265's effect classes are intended to be the next.
//!
//! Two entry points:
//!
//!  * [`walk`] — a fact-accumulating DFS. The [`FactVisitor`]
//!    decides what an allocation site, an unresolved call, or a
//!    recursive edge contributes; the engine handles traversal
//!    order, loop context, cycle detection, and the step cap. The
//!    `emit` flag on every visitor method distinguishes the real
//!    walk (record diagnostics) from silent probes (loop-callee
//!    pre-walks: contribute facts, discard diagnostics) — the same
//!    split `budget_check` made with its throwaway offender vec.
//!  * [`witness_path`] — the #265 primitive: the first call chain
//!    from a root to a site/edge matching a predicate, returned as
//!    the human-renderable `root -> f -> g [leaf]` path that
//!    `@budget`'s fixpoint could never produce.

use hale_syntax::Span;

use crate::alloc_summary::{
    AllocSite, AllocSummary, CallEdge, Callee, FnKey, FnSummary,
};

/// Runaway-graph guard, inherited from `budget_check`: a diamond-heavy
/// graph revisits shared callees once per reaching path (correct for
/// per-call counting), so pathological shapes could blow up. Past the
/// cap the walk reports saturation rather than spinning. Real call
/// graphs are nowhere near this.
pub const MAX_STEPS: u32 = 20_000;

/// What one traversal event contributes to the accumulated fact.
///
/// Facts accumulate via `identity` + `combine`; `saturated` is the
/// conservative worst-case element used when the step cap trips. For
/// `budget_check` the fact is a saturating count; for #265 effect
/// classes it will be a lattice mask + quantity.
///
/// `emit` is false during silent probes (the pre-walk of a callee
/// inside a loop): compute and return the fact as usual, but do not
/// record diagnostics/offenders.
pub trait FactVisitor {
    type Fact: Clone;

    fn identity(&self) -> Self::Fact;
    fn combine(&self, a: Self::Fact, b: Self::Fact) -> Self::Fact;
    /// The element returned when the step cap trips.
    fn saturated(&self) -> Self::Fact;
    /// Does this fact contribute anything? Drives the loop-call
    /// policy: a loop callee whose probe `is_zero` produces no event.
    fn is_zero(&self, f: &Self::Fact) -> bool;

    /// An allocation site in `key`'s own body (`site.loop_depth > 0`
    /// = every-iteration).
    fn site(
        &mut self,
        key: &FnKey,
        site: &AllocSite,
        emit: bool,
    ) -> Self::Fact;
    /// An unresolved (external / stdlib / FFI) callee.
    fn unresolved(
        &mut self,
        key: &FnKey,
        edge: &CallEdge,
        name: &str,
        in_loop: bool,
        emit: bool,
    ) -> Self::Fact;
    /// A cycle on the current ancestor path (recursion).
    fn recursion(
        &mut self,
        key: &FnKey,
        edge: &CallEdge,
        callee: &FnKey,
        emit: bool,
    ) -> Self::Fact;
    /// A resolved callee inside a loop whose silent probe (`sub`)
    /// was non-zero: its work repeats every iteration.
    fn loop_call(
        &mut self,
        key: &FnKey,
        edge: &CallEdge,
        callee: &FnKey,
        sub: Self::Fact,
        emit: bool,
    ) -> Self::Fact;
}

/// Fact-accumulating DFS from `root`. Traversal semantics are
/// `budget_check::count_fn`'s, verbatim:
///
///  * sites first, then call edges, in summary order;
///  * a resolved callee NOT in a loop folds its walk in directly
///    (offender spans point into the callee's body);
///  * a resolved callee IN a loop is probed silently; only a
///    non-`is_zero` probe produces a `loop_call` event;
///  * an ancestor-path callee is `recursion`;
///  * past [`MAX_STEPS`] the walk returns `saturated()`.
pub fn walk<V: FactVisitor>(
    summary: &AllocSummary,
    root: &FnKey,
    visitor: &mut V,
) -> V::Fact {
    let mut path = Vec::new();
    let mut steps = 0u32;
    walk_inner(summary, root, visitor, &mut path, &mut steps, true)
}

fn walk_inner<V: FactVisitor>(
    summary: &AllocSummary,
    key: &FnKey,
    visitor: &mut V,
    path: &mut Vec<FnKey>,
    steps: &mut u32,
    emit: bool,
) -> V::Fact {
    let fs: &FnSummary = match summary.fns.get(key) {
        Some(fs) => fs,
        // Unresolved target with no summary — nothing visible.
        None => return visitor.identity(),
    };

    let mut total = visitor.identity();

    for site in &fs.sites {
        *steps += 1;
        if *steps > MAX_STEPS {
            return visitor.saturated();
        }
        let f = visitor.site(key, site, emit);
        total = visitor.combine(total, f);
    }

    path.push(key.clone());
    for edge in &fs.calls {
        *steps += 1;
        if *steps > MAX_STEPS {
            path.pop();
            return visitor.saturated();
        }
        let in_loop = edge.loop_depth > 0;
        match &edge.callee {
            Callee::Resolved(callee_key) => {
                if path.contains(callee_key) {
                    let f =
                        visitor.recursion(key, edge, callee_key, emit);
                    total = visitor.combine(total, f);
                    continue;
                }
                if in_loop {
                    let sub = walk_inner(
                        summary, callee_key, visitor, path, steps, false,
                    );
                    if !visitor.is_zero(&sub) {
                        let f = visitor
                            .loop_call(key, edge, callee_key, sub, emit);
                        total = visitor.combine(total, f);
                    }
                } else {
                    let sub = walk_inner(
                        summary, callee_key, visitor, path, steps, emit,
                    );
                    total = visitor.combine(total, sub);
                }
            }
            Callee::Unresolved(name) => {
                let f =
                    visitor.unresolved(key, edge, name, in_loop, emit);
                total = visitor.combine(total, f);
            }
        }
    }
    path.pop();
    total
}

/// One step of a witness path: the fn whose body contains the hop,
/// and the span of the site/edge that continues (or ends) the chain.
#[derive(Debug, Clone)]
pub struct WitnessStep {
    pub in_fn: FnKey,
    pub span: Span,
    /// Human label: the callee display name for interior hops, or
    /// the predicate's own label for the leaf.
    pub label: String,
}

/// What [`witness_path`] tests at each traversal event.
pub enum Probe<'a> {
    /// An allocation site in the current fn's body.
    Site(&'a AllocSite),
    /// An unresolved callee by name (the classified frontier:
    /// stdlib path-calls, unknown externals).
    Unresolved(&'a str, &'a CallEdge),
    /// A RESOLVED callee, tested before descending into it. An
    /// `@ffi` fn is resolved (it has a summary entry with an empty
    /// body), so `@no_ffi`-style predicates match here rather than
    /// on the unresolved frontier. Returning a label stops the walk
    /// with the callee as the leaf; returning None descends.
    Resolved(&'a FnKey, &'a CallEdge),
}

/// The #265 primitive: the FIRST chain of calls from `root` to a
/// site/unresolved-callee satisfying `pred` (which returns the leaf's
/// label), as renderable steps. `None` when nothing reachable
/// matches. Depth-first in summary order, ancestor cycles skipped —
/// deterministic for stable summaries.
pub fn witness_path(
    summary: &AllocSummary,
    root: &FnKey,
    pred: &mut dyn FnMut(&Probe<'_>) -> Option<String>,
) -> Option<Vec<WitnessStep>> {
    let mut path = Vec::new();
    let mut steps = 0u32;
    witness_inner(summary, root, pred, &mut path, &mut steps)
}

fn witness_inner(
    summary: &AllocSummary,
    key: &FnKey,
    pred: &mut dyn FnMut(&Probe<'_>) -> Option<String>,
    path: &mut Vec<FnKey>,
    steps: &mut u32,
) -> Option<Vec<WitnessStep>> {
    let fs = summary.fns.get(key)?;
    for site in &fs.sites {
        *steps += 1;
        if *steps > MAX_STEPS {
            return None;
        }
        if let Some(label) = pred(&Probe::Site(site)) {
            return Some(vec![WitnessStep {
                in_fn: key.clone(),
                span: site.span,
                label,
            }]);
        }
    }
    path.push(key.clone());
    for edge in &fs.calls {
        *steps += 1;
        if *steps > MAX_STEPS {
            break;
        }
        match &edge.callee {
            Callee::Resolved(callee_key) => {
                if path.contains(callee_key) {
                    continue;
                }
                if let Some(label) =
                    pred(&Probe::Resolved(callee_key, edge))
                {
                    path.pop();
                    return Some(vec![WitnessStep {
                        in_fn: key.clone(),
                        span: edge.span,
                        label,
                    }]);
                }
                if let Some(mut tail) =
                    witness_inner(summary, callee_key, pred, path, steps)
                {
                    let mut out = vec![WitnessStep {
                        in_fn: key.clone(),
                        span: edge.span,
                        label: callee_key.display(),
                    }];
                    out.append(&mut tail);
                    path.pop();
                    return Some(out);
                }
            }
            Callee::Unresolved(name) => {
                if let Some(label) = pred(&Probe::Unresolved(name, edge))
                {
                    path.pop();
                    return Some(vec![WitnessStep {
                        in_fn: key.clone(),
                        span: edge.span,
                        label,
                    }]);
                }
            }
        }
    }
    path.pop();
    None
}

/// Render a witness chain as `root -> hop -> hop [leaf-label]` — the
/// diagnostic shape #265 specifies.
pub fn render_witness(root: &FnKey, steps: &[WitnessStep]) -> String {
    let mut s = root.display();
    for (i, st) in steps.iter().enumerate() {
        if i + 1 == steps.len() {
            s.push_str(&format!(" [{}]", st.label));
        } else {
            s.push_str(&format!(" -> {}", st.label));
        }
    }
    s
}
