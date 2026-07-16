//! Lever 2 (2026-07-16) — `@budget(alloc_per_call = N)`, an opt-in
//! hot-path allocation contract.
//!
//! `@unbounded` says "this fn allocates without a static bound, on
//! purpose" and silences the leak lint. `@budget` is its dual: an
//! explicit *ceiling*. `@budget(alloc_per_call = N)` on a free fn or a
//! locus method asserts the fn performs **at most `N` arena allocations
//! per call**, and the compiler enforces it as a hard error. `N = 0` is
//! the zero-alloc certificate — the strongest, most useful form: a
//! per-datagram handler or a decode helper the runtime can call on the
//! hot path with a guarantee it touches no arena.
//!
//! ## What counts
//!
//! It reuses [`crate::alloc_summary`] — the same per-fn allocation-site +
//! call-graph IR the memory-bound proofs and the leak lint consume. Per
//! call of the annotated fn, the check counts:
//!
//!   * each arena-allocating literal / collection insert the summary sees
//!     (`Struct` / array / bytes / `@form` vec/hashmap insert), in the fn
//!     body **and transitively through resolved (bundle-local) callees**;
//!   * each known-allocating stdlib `recv` (the `recv` family returns a
//!     fresh result buffer — the exact set the hot-path lint flags);
//!
//! and any of those **inside a loop** (or reached via a call inside a
//! loop) counts as *unbounded per call* — a per-call budget is a
//! statement about one invocation, and a loop runs its body many times
//! per invocation. Recursion is likewise unbounded.
//!
//! ## What it can't see
//!
//! Opaque calls other than the known-allocating `recv` set (foreign
//! receivers, most stdlib, FFI) are not counted — the budget bounds "the
//! allocations the compiler can see," and is meant to be paired with
//! `recv_into` + the hot-path lint, not to model the whole heap. This is
//! the same soundness boundary `alloc_summary`'s escape tagging draws.

use hale_syntax::ast::*;
use hale_syntax::{Diag, Span};

use crate::alloc_summary::{
    self, AllocKind, AllocSummary, Callee, FnKey, FnSummary,
};

/// A per-call allocation count that saturates at `Unbounded` (a
/// loop-nested allocation, a call in a loop, or recursion).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Count {
    Finite(u64),
    Unbounded,
}

impl Count {
    fn zero() -> Count {
        Count::Finite(0)
    }
    fn add(self, other: Count) -> Count {
        match (self, other) {
            (Count::Finite(a), Count::Finite(b)) => Count::Finite(a.saturating_add(b)),
            _ => Count::Unbounded,
        }
    }
    fn nonzero(self) -> bool {
        !matches!(self, Count::Finite(0))
    }
    fn exceeds(self, budget: u32) -> bool {
        match self {
            Count::Finite(n) => n > budget as u64,
            Count::Unbounded => true,
        }
    }
    fn render(self) -> String {
        match self {
            Count::Finite(n) => n.to_string(),
            Count::Unbounded => "an unbounded number of".to_string(),
        }
    }
}

/// A single allocation counted against a budget — its source span and a
/// human note. Collected so an over-budget error can point at every
/// offending line, not just report a total.
struct Offender {
    span: Span,
    note: String,
}

/// The known-allocating opaque stdlib calls: the `recv` family, which
/// each return a freshly-allocated result buffer. `recv_into` is the
/// zero-alloc alternative. This is exactly the set the hot-path lint
/// (`check_hot_path_alloc`) flags in a loop — the two levers agree on
/// what "allocating recv" means. Path-call form carries the full
/// `std::io::udp::recv` path; method-call form carries just the bare
/// name (the receiver types as Unknown).
fn opaque_recv_allocates(name: &str) -> bool {
    matches!(
        name,
        "std::io::tcp::recv"
            | "std::io::tcp::recv_bytes"
            | "std::io::udp::recv"
            | "std::io::udp::recv_with_source"
            | "std::io::tls::recv_bytes"
            | "recv_bytes"
            | "recv_with_source"
    )
}

fn describe_kind(kind: &AllocKind) -> String {
    match kind {
        AllocKind::StructLit(n) => format!("a `{}` instantiation", n),
        AllocKind::ArrayLit => "an array literal".to_string(),
        AllocKind::ArrayRepeat => "an array-repeat literal".to_string(),
        AllocKind::BytesLit => "a bytes literal".to_string(),
        AllocKind::CollectionInsert(form) => {
            format!("an insert into a growing `@form({})` slot", form)
        }
    }
}

/// A DFS-work ceiling. A pathological wide call graph could re-count a
/// shared callee exponentially; past this many steps we stop and report
/// the fn as uncertifiable (conservatively over budget) rather than
/// spin. Real call graphs are nowhere near this.
const MAX_STEPS: u32 = 20_000;

/// Count the arena allocations one call of `key` performs, collecting
/// offenders for diagnostics. `path` is the current DFS ancestor stack
/// (for cycle → recursion detection); a diamond (a callee reached by two
/// distinct paths) is *not* on the path and is correctly counted once
/// per reaching call site.
fn count_fn(
    key: &FnKey,
    summary: &AllocSummary,
    path: &mut Vec<FnKey>,
    offenders: &mut Vec<Offender>,
    steps: &mut u32,
) -> Count {
    let fs: &FnSummary = match summary.fns.get(key) {
        Some(fs) => fs,
        // An unresolved / external target has no summary — nothing we can
        // see. (Callers already special-case the known-allocating recv
        // family before recursing here.)
        None => return Count::zero(),
    };

    let mut total = Count::zero();

    // Direct allocation sites in this body.
    for site in &fs.sites {
        *steps += 1;
        if *steps > MAX_STEPS {
            return Count::Unbounded;
        }
        if site.loop_depth > 0 {
            offenders.push(Offender {
                span: site.span,
                note: format!(
                    "{} inside a loop — a fresh allocation every iteration",
                    describe_kind(&site.kind)
                ),
            });
            total = total.add(Count::Unbounded);
        } else {
            offenders.push(Offender {
                span: site.span,
                note: describe_kind(&site.kind),
            });
            total = total.add(Count::Finite(1));
        }
    }

    // Call edges.
    path.push(key.clone());
    for edge in &fs.calls {
        *steps += 1;
        if *steps > MAX_STEPS {
            path.pop();
            return Count::Unbounded;
        }
        let in_loop = edge.loop_depth > 0;
        match &edge.callee {
            Callee::Resolved(callee_key) => {
                if path.contains(callee_key) {
                    // A cycle on the current path = recursion = unbounded
                    // per call.
                    offenders.push(Offender {
                        span: edge.span,
                        note: format!(
                            "recursive call to `{}` — unbounded allocation \
                             per call",
                            callee_key.display()
                        ),
                    });
                    total = total.add(Count::Unbounded);
                    continue;
                }
                if in_loop {
                    // The callee runs many times per call. If it allocates
                    // at all, that's unbounded per call.
                    let mut probe = Vec::new();
                    let sub = count_fn(callee_key, summary, path, &mut probe, steps);
                    if sub.nonzero() {
                        offenders.push(Offender {
                            span: edge.span,
                            note: format!(
                                "calls `{}` inside a loop — its allocations \
                                 repeat every iteration",
                                callee_key.display()
                            ),
                        });
                        total = total.add(Count::Unbounded);
                    }
                } else {
                    // A once-per-call callee: its allocations fold in
                    // directly (offenders point into its body).
                    total = total.add(count_fn(
                        callee_key, summary, path, offenders, steps,
                    ));
                }
            }
            Callee::Unresolved(name) => {
                if opaque_recv_allocates(name) {
                    if in_loop {
                        offenders.push(Offender {
                            span: edge.span,
                            note: format!(
                                "`{}` inside a loop allocates a fresh buffer \
                                 every iteration — use `recv_into` with a \
                                 reused `BytesBuilder`",
                                name
                            ),
                        });
                        total = total.add(Count::Unbounded);
                    } else {
                        offenders.push(Offender {
                            span: edge.span,
                            note: format!(
                                "`{}` allocates a fresh result buffer — use \
                                 `recv_into` with a reused `BytesBuilder` for \
                                 a zero-alloc read",
                                name
                            ),
                        });
                        total = total.add(Count::Finite(1));
                    }
                }
                // Any other opaque call is outside what the budget can
                // see (documented boundary).
            }
        }
    }
    path.pop();
    total
}

/// The public entry: check every `@budget(...)`-annotated fn/method in
/// `programs` against its declared per-call allocation ceiling. Returns
/// hard-error diagnostics (opt-in: you asked for the contract, so a
/// violation fails the build) — empty when every contract holds.
pub fn budget_diags(programs: &[&Program]) -> Vec<Diag> {
    let summary = alloc_summary::summarize_programs(programs);
    let mut diags = Vec::new();

    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Fn(fd) => {
                    if let Some(budget) = fd.budget {
                        let key = FnKey::free_fn(fd.name.name.clone());
                        check_one(&key, budget, fd, &summary, &mut diags);
                    }
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            if let Some(budget) = fd.budget {
                                let key = FnKey::method(
                                    l.name.name.clone(),
                                    fd.name.name.clone(),
                                );
                                check_one(&key, budget, fd, &summary, &mut diags);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    diags
}

/// The cap on how many offender pinpoints one violation emits, so a
/// wildly-over-budget fn doesn't bury the build in notes.
const MAX_OFFENDERS: usize = 6;

fn check_one(
    key: &FnKey,
    budget: u32,
    fd: &FnDecl,
    summary: &AllocSummary,
    diags: &mut Vec<Diag>,
) {
    let mut offenders = Vec::new();
    let mut path = Vec::new();
    let mut steps = 0u32;
    let count = count_fn(key, summary, &mut path, &mut offenders, &mut steps);

    if !count.exceeds(budget) {
        return;
    }

    let advice = if budget == 0 {
        "For a zero-alloc hot path: hoist per-iteration allocations to \
         reused fields, read with `recv_into` into a reused \
         `std::bytes::BytesBuilder`, and keep locus instantiation out of \
         the fn. If the allocation is intentional, drop `@budget` for \
         `@unbounded fn`."
    } else {
        "Reduce per-call allocations (hoist to reused fields, use \
         `recv_into`), raise the budget, or drop `@budget` for \
         `@unbounded fn` if the allocation is intentional."
    };

    diags.push(Diag::ty(
        fd.name.span,
        format!(
            "hot-path budget exceeded: `{}` declares \
             `@budget(alloc_per_call = {})` but the compiler counts {} \
             arena allocation(s) per call. {}",
            key.display(),
            budget,
            count.render(),
            advice,
        ),
    ));

    for off in offenders.into_iter().take(MAX_OFFENDERS) {
        diags.push(Diag::ty(
            off.span,
            format!("counts against `{}`'s alloc budget: {}", key.display(), off.note),
        ));
    }
}
