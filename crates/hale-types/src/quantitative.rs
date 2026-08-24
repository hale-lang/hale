//! GH #265 step 5 — the quantitative layer.
//!
//! `@budget(alloc_per_call = N)` proved the compiler can *count* an
//! effect. This generalizes counting to the other dimensions the
//! issue names, on the same call-graph substrate:
//!
//!   * **`@budget(stack_bytes = N)`** — with acyclicity established
//!     (`@no_recursion` / the `recursion` class), the call graph
//!     under a root is a DAG, so worst-case stack depth is a
//!     longest-path computation over per-frame sizes. Not WCET, but
//!     the structural bound the embedded / DO-178 audience needs:
//!     "this handler's call tree cannot exceed 4 KB of stack."
//!   * **`@budget(block_points = N)`** — how many blocking
//!     operations one call may reach. `0` is `@no_block`; `1` is the
//!     useful middle ("this may wait once, on its own socket").
//!   * **`@budget(publish = N)`** — publishes per call. `1` is the
//!     issue's `@replies` (exactly-once reply per delivery), falling
//!     out as a count rather than a bespoke analysis.
//!   * **`@budget(fanout = N)`** — transitive *amplification*: the
//!     number of subscriber deliveries one call can cause, read off
//!     the bus graph. This is the backpressure/DoS property — a
//!     handler that publishes to a 200-subscriber subject amplifies
//!     200×, which no per-fn count would reveal.
//!
//! Frame sizes are ESTIMATED from declared shapes (params, locals,
//! payload buffers), not measured from codegen — the bound is a
//! structural over-approximation, and the diagnostic says so. A
//! recursive path is unbounded and reported as such (which is why
//! `@no_recursion` composes with this rather than duplicating it).

use std::collections::BTreeMap;

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::verdict::Verdict;
use crate::alloc_summary::{self, AllocSummary, Callee, FnKey};
use crate::callgraph;

/// How many subscriber DELIVERIES one publish site can cause.
///
/// Round 2: this used to be `Fn(&str) -> u64` — a subject-text
/// lookup that counted covering `subscribes` rows. A `Subscribe` row
/// is DECLARATION-grained: one subscription declared by one handler.
/// Three arranged replicas of one `Sink` are three runtime
/// registrations and three deliveries, and the old count said one,
/// so `@budget(fanout = 1)` certified a publish that dispatched
/// three cells. Keyed subscriptions failed the other way: two
/// mutually-exclusive filters on one subject were both charged,
/// because address coverage is only half of delivery.
///
/// So the question is asked per SITE — `(publishing fn, site
/// ordinal, subject text)` — and answered against the model's own
/// delivery join and instance population. `None` means the answer
/// is not knowable (a dynamic population, an unknown key, an
/// external route), which is unboundedness, not one.
pub type FanoutOf<'a> =
    dyn Fn(&FnKey, u32, &str) -> Option<u64> + 'a;

/// Unit rendering for a dimension's diagnostic.
fn dim_unit(d: QuantDim) -> &'static str {
    match d {
        QuantDim::StackBytes => "bytes of stack",
        QuantDim::BlockPoints => "blocking operation(s)",
        QuantDim::Publish => "publish(es)",
        QuantDim::Fanout => "subscriber delivery/deliveries",
        // #382 phase 3: calls to declared carriers of the class.
        QuantDim::UserClass(_) => "call(s) to a declared carrier",
    }
}

/// Dimension name for diagnostics — user classes render through the
/// seed's intern table.
fn dim_display(d: QuantDim, names: &[String]) -> String {
    match d {
        QuantDim::UserClass(i) => names
            .get(i as usize)
            .cloned()
            .unwrap_or_else(|| "<user effect class>".to_string()),
        other => other.as_str().to_string(),
    }
}

/// A measured quantity that saturates at "unbounded" (a cycle, or a
/// loop-nested contributor for the per-call dimensions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Qty {
    Finite(u64),
    Unbounded,
}

impl Qty {
    fn add(self, o: Qty) -> Qty {
        match (self, o) {
            (Qty::Finite(a), Qty::Finite(b)) => Qty::Finite(a.saturating_add(b)),
            _ => Qty::Unbounded,
        }
    }
    fn max(self, o: Qty) -> Qty {
        match (self, o) {
            (Qty::Finite(a), Qty::Finite(b)) => Qty::Finite(a.max(b)),
            _ => Qty::Unbounded,
        }
    }
    fn exceeds(self, cap: u64) -> bool {
        match self {
            Qty::Finite(n) => n > cap,
            Qty::Unbounded => true,
        }
    }
    fn render(self) -> String {
        match self {
            Qty::Finite(n) => n.to_string(),
            Qty::Unbounded => "an unbounded number of".to_string(),
        }
    }
}

/// Estimated stack frame size for one fn, from its declared shape.
/// Deliberately coarse and OVER-approximating: every param and local
/// is charged, aggregates by their declared width where known. The
/// contract is "the real frame is no larger than this," which is what
/// makes the resulting bound safe to assert on.
fn frame_bytes(fd: &FnDecl) -> u64 {
    const SLOT: u64 = 8; // one register-width slot
    const CALL_OVERHEAD: u64 = 32; // return addr + saved regs + align
    let mut n = CALL_OVERHEAD;
    n += fd.params.len() as u64 * SLOT;
    // Locals: one slot each, plus the known-wide shapes.
    fn count_block(b: &Block, n: &mut u64) {
        for st in &b.stmts {
            match st {
                Stmt::Let { ty, .. } => {
                    *n += match ty {
                        // A Bytes/String local carries a pointer;
                        // the buffer is arena/heap, not stack.
                        Some(TypeExpr::Named { path, .. })
                            if path
                                .segments
                                .last()
                                .map(|s| s.name == "Decimal")
                                .unwrap_or(false) =>
                        {
                            16
                        }
                        _ => 8,
                    };
                }
                Stmt::If(i) => {
                    count_block(&i.then_block, n);
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    count_block(body, n)
                }
                _ => {}
            }
        }
    }
    count_block(&fd.body, &mut n);
    n
}

/// Per-fn frame sizes for the bundle, keyed like the call graph.
///
/// `pub(crate)` since Change 5h: the model builder records these as
/// `FrameBytes` cost sites so the migrated `@budget(stack_bytes)`
/// judgment counts over the model. ONE authority — the estimate is
/// computed here and called, not re-derived there.
pub(crate) fn frame_map(programs: &[&Program]) -> BTreeMap<FnKey, u64> {
    let mut out = BTreeMap::new();
    for p in programs {
        for item in &p.items {
            match item {
                TopDecl::Fn(fd) => {
                    out.insert(
                        FnKey::free_fn(fd.name.name.clone()),
                        frame_bytes(fd),
                    );
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            out.insert(
                                FnKey::method(
                                    l.name.name.clone(),
                                    fd.name.name.clone(),
                                ),
                                frame_bytes(fd),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Longest root-to-leaf stack path (DAG longest path). A cycle makes
/// it unbounded — the reason `@no_recursion` is this dimension's
/// natural companion.
fn stack_depth(
    summary: &AllocSummary,
    frames: &BTreeMap<FnKey, u64>,
    key: &FnKey,
    path: &mut Vec<FnKey>,
    memo: &mut BTreeMap<FnKey, Qty>,
    steps: &mut u32,
) -> Qty {
    if let Some(q) = memo.get(key) {
        return *q;
    }
    if path.contains(key) {
        return Qty::Unbounded;
    }
    *steps += 1;
    if *steps > callgraph::MAX_STEPS {
        return Qty::Unbounded;
    }
    let own = Qty::Finite(*frames.get(key).unwrap_or(&64));
    let Some(fs) = summary.fns.get(key) else {
        return own;
    };
    path.push(key.clone());
    let mut deepest = Qty::Finite(0);
    for edge in &fs.calls {
        if let Callee::Resolved(callee) = &edge.callee {
            let sub = stack_depth(summary, frames, callee, path, memo, steps);
            deepest = deepest.max(sub);
        }
    }
    path.pop();
    let total = own.add(deepest);
    // Memoize only cycle-free results (a path-dependent Unbounded
    // must not poison a diamond reached another way).
    if !path.contains(key) && total != Qty::Unbounded {
        memo.insert(key.clone(), total);
    }
    total
}

/// Count a per-call quantity over the call graph. Loop-nested
/// contributors saturate to Unbounded, matching `@budget`'s
/// per-call semantics.
fn count_dim(
    summary: &AllocSummary,
    key: &FnKey,
    dim: QuantDim,
    fanout_of: &FanoutOf<'_>,
    carrier_mask: crate::stdlib_surface::EffectSet,
    path: &mut Vec<FnKey>,
    steps: &mut u32,
) -> Qty {
    let Some(fs) = summary.fns.get(key) else {
        return Qty::Finite(0);
    };
    let mut total = Qty::Finite(0);
    // Syntactic sites (publish / fanout). Publish sites consume
    // source-order ordinals, the same space the model's `Publish`
    // rows are keyed by — that ordinal is how a fan-out question
    // finds the site it is about.
    let mut pub_site: u32 = 0;
    for site in &fs.effect_sites {
        *steps += 1;
        if *steps > callgraph::MAX_STEPS {
            return Qty::Unbounded;
        }
        if let alloc_summary::EffectSiteKind::Publish(subj) = &site.kind {
            let ordinal = pub_site;
            pub_site += 1;
            let per: Option<u64> = match dim {
                QuantDim::Publish => Some(1),
                QuantDim::Fanout => match subj.as_ref() {
                    Some(s) => fanout_of(key, ordinal, &s.text),
                    // A computed subject can address any endpoint.
                    None => None,
                },
                _ => Some(0),
            };
            match per {
                None => total = total.add(Qty::Unbounded),
                Some(0) => {}
                Some(n) => {
                    total = total.add(if site.loop_depth > 0 {
                        Qty::Unbounded
                    } else {
                        Qty::Finite(n)
                    });
                }
            }
        }
    }
    // #353: an INDIRECT call through a function-typed parameter. Its
    // target is not knowable from this fn, so it may allocate, block
    // or publish any number of times — the quantity is Unbounded, not
    // zero. `@budget(alloc_per_call = 0)` on a fn whose body is
    // `return f(v);` passed while the callee allocated, which is the
    // budget half of the same certificate hole as the effect classes.
    for edge in &fs.calls {
        // #382: an untypeable-receiver method call gets the same
        // unbounded treatment as an indirect call.
        if edge.indirect || edge.opaque_method_call() {
            total = total.add(Qty::Unbounded);
        }
    }
    // Frontier leaves (block points).
    if dim == QuantDim::BlockPoints {
        for edge in &fs.calls {
            if let Callee::Unresolved(name) = &edge.callee {
                let segs: Vec<&str> = name.split("::").collect();
                if let Some(eff) = crate::stdlib_surface::effects_for(&segs) {
                    if eff.contains(crate::stdlib_surface::EffectSet::BLOCK) {
                        total = total.add(if edge.loop_depth > 0 {
                            Qty::Unbounded
                        } else {
                            Qty::Finite(1)
                        });
                    }
                }
            }
        }
    }
    // Resolved callees. #392: fanned-out interface-dispatch
    // alternatives share a `dispatch_group`; a dispatch invokes
    // exactly one of them, so a group contributes the MAX over its
    // alternatives where a real call sequence would sum.
    path.push(key.clone());
    let mut group_max: BTreeMap<u32, Qty> = BTreeMap::new();
    for edge in &fs.calls {
        *steps += 1;
        if *steps > callgraph::MAX_STEPS {
            path.pop();
            return Qty::Unbounded;
        }
        if let Callee::Resolved(callee) = &edge.callee {
            let mut contrib = Qty::Finite(0);
            if path.contains(callee) {
                contrib = contrib.add(Qty::Unbounded);
            } else {
                // #382 phase 3: a call to a declared CARRIER of the
                // budgeted user class counts one site (loop-nested is
                // unbounded, like every per-call contributor).
                if matches!(dim, QuantDim::UserClass(_)) {
                    let is_carrier = summary
                        .carries
                        .get(callee)
                        .map_or(false, |c| c.0 & carrier_mask.0 != 0);
                    if is_carrier {
                        contrib = contrib.add(if edge.loop_depth > 0 {
                            Qty::Unbounded
                        } else {
                            Qty::Finite(1)
                        });
                    }
                }
                let sub = count_dim(
                    summary, callee, dim, fanout_of, carrier_mask,
                    path, steps,
                );
                contrib = contrib.add(if edge.loop_depth > 0 {
                    match sub {
                        Qty::Finite(0) => Qty::Finite(0),
                        _ => Qty::Unbounded,
                    }
                } else {
                    sub
                });
            }
            match edge.dispatch_group {
                Some(g) => {
                    let e = group_max
                        .entry(g)
                        .or_insert(Qty::Finite(0));
                    *e = e.max(contrib);
                }
                None => total = total.add(contrib),
            }
        }
    }
    for q in group_max.into_values() {
        total = total.add(q);
    }
    path.pop();
    total
}

/// Check every quantitative `@budget(<dim> = N)` clause in the
/// bundle. `fanout_of` maps a subject to its subscriber count (from
/// the bus graph); callers without a graph pass a `|_| 1`.
pub fn quantitative_diags(
    programs: &[&Program],
    fanout_of: &FanoutOf<'_>,
) -> Vec<Diag> {
    quantitative_report(programs, fanout_of).0
}

/// #392 §8: every quantitative `@budget(<dim> = N)` contract as a
/// lowered claim row with its verdict — same evaluation as the
/// diagnostics, so the two cannot disagree.
pub fn certificate_rows(
    programs: &[&Program],
    fanout_of: &FanoutOf<'_>,
) -> Vec<crate::effects::LoweredCertificate> {
    quantitative_report(programs, fanout_of).1
}

fn quantitative_report(
    programs: &[&Program],
    fanout_of: &FanoutOf<'_>,
) -> (
    Vec<Diag>,
    Vec<crate::effects::LoweredCertificate>,
    Vec<(usize, usize)>,
) {
    let mut roots: Vec<(FnKey, Vec<(QuantDim, u64)>, Span)> = Vec::new();
    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Fn(fd) if !fd.quantities.is_empty() => roots.push((
                    FnKey::free_fn(fd.name.name.clone()),
                    fd.quantities.clone(),
                    fd.name.span,
                )),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            if !fd.quantities.is_empty() {
                                roots.push((
                                    FnKey::method(
                                        l.name.name.clone(),
                                        fd.name.name.clone(),
                                    ),
                                    fd.quantities.clone(),
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
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let summary = alloc_summary::summarize_programs(programs);
    let frames = frame_map(programs);
    let names = crate::effects::effect_names_of(programs);
    let declared = crate::effects::declared_of(programs);
    let defs = crate::effects::defs_of(programs);
    let mut diags = Vec::new();
    let mut rows = Vec::new();
    // Where each row's own diagnostics begin (Change 5h) — the
    // grouped report hands them to the evidence sidecar without a
    // second evaluation.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (key, dims, span) in &roots {
        for (dim, cap) in dims {
            // Closed at the end of this iteration. An undeclared
            // class `continue`s before a row exists, and its
            // diagnostic must NOT fall into the previous row's
            // group — it is a lowering issue, and the judgment
            // makes such a row Invalid from the class reference
            // alone.
            let base = diags.len();
            // #382 phase 3: a user-class dimension must name a
            // DECLARED class — the misspelt-class rule, applied to
            // budget keys.
            if let QuantDim::UserClass(i) = dim {
                if !declared.contains(i) {
                    let bad = names
                        .get(*i as usize)
                        .cloned()
                        .unwrap_or_default();
                    let mut near: Vec<&String> = names
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| declared.contains(&(*j as u16)))
                        .map(|(_, n)| n)
                        .filter(|n| crate::effects::close(n, &bad))
                        .collect();
                    near.sort();
                    let hint = match near.first() {
                        Some(n) => format!(" Did you mean `{}`?", n),
                        None => String::new(),
                    };
                    diags.push(Diag::ty(
                        *span,
                        format!(
                            "`{}` budgets effect class `{}`, which is \
                             never declared. Add `effect {};` at the \
                             top level.{}",
                            key.display(),
                            bad,
                            bad,
                            hint
                        ),
                    ));
                    continue;
                }
            }
            let measured = match dim {
                QuantDim::StackBytes => {
                    let mut path = Vec::new();
                    let mut memo = BTreeMap::new();
                    let mut steps = 0u32;
                    stack_depth(
                        &summary, &frames, key, &mut path, &mut memo,
                        &mut steps,
                    )
                }
                _ => {
                    let mask = match dim {
                        QuantDim::UserClass(i) => {
                            crate::frontier::class_mask_with(
                                EffectClass::User(*i),
                                &defs,
                            )
                        }
                        _ => crate::stdlib_surface::EffectSet::PURE,
                    };
                    let mut path = Vec::new();
                    let mut steps = 0u32;
                    count_dim(
                        &summary, key, *dim, fanout_of, mask, &mut path,
                        &mut steps,
                    )
                }
            };
            ranges.push((base, base));
            rows.push(crate::effects::LoweredCertificate {
                subject: key.display(),
                form: format!(
                    "bound {} <= {} on paths from {{{}}}",
                    dim_display(*dim, &names),
                    cap,
                    key.display()
                ),
                result: if measured.exceeds(*cap) {
                        Verdict::Violated
                    } else {
                        Verdict::Holds
                    },
            });
            if !measured.exceeds(*cap) {
                continue;
            }
            let extra = match dim {
                QuantDim::StackBytes => {
                    " Frame sizes are estimated from declared shapes and \
                     over-approximate; a recursive path is unbounded (pair \
                     with `@no_recursion`)."
                }
                QuantDim::Fanout => {
                    " Fan-out counts transitive subscriber deliveries — a \
                     publish to a many-subscriber subject amplifies."
                }
                QuantDim::UserClass(_) => {
                    " Counts calls to declared carriers of the class \
                     along any path; a carrier inside a loop is \
                     unbounded per call."
                }
                _ => {
                    " A contributor inside a loop is unbounded per call."
                }
            };
            diags.push(Diag::ty(
                *span,
                format!(
                    "budget exceeded: `{}` declares `@budget({} = {})` but \
                     the compiler measures {} {}.{}",
                    key.display(),
                    dim_display(*dim, &names),
                    cap,
                    measured.render(),
                    dim_unit(*dim),
                    extra
                ),
            ));
            if let Some(last) = ranges.last_mut() {
                last.1 = diags.len();
            }
        }
    }
    (diags, rows, ranges)
}

/// #476 Change 5h: every quantitative `@budget(<dim> = N)` contract
/// as a lowered certificate WITH the engine's own diagnostics.
///
/// Measuring stays this engine's question; the evidence sidecar
/// carries what it measured, and the VERDICT becomes the
/// judgment's — the duplicate authority #476 removes.
pub fn certificate_groups(
    programs: &[&Program],
    fanout_of: &FanoutOf<'_>,
) -> Vec<(crate::effects::LoweredCertificate, Vec<Diag>)> {
    let (diags, rows, ranges) = quantitative_report(programs, fanout_of);
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| {
            let (from, to) =
                ranges.get(i).copied().unwrap_or((0, 0));
            (row, diags[from..to].to_vec())
        })
        .collect()
}
