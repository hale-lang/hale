# Resource-budget tracking (GH #18 item 5)

Status: **scope + count slice landed.** Written 2026-06-10. The verification
candidate that "complements #1" and reuses the shared dataflow infra built
for memory-bound proofs (#100/#101/#103). The issue frames it as a *linter*
— a useful "is this in range" signal + a CI gate ("this PR raised the fd
ceiling — intentional?"), not a proof.

## The resources (language-visible)

- **OS threads** = **pinned loci** (`PlacementSpec::Pinned` placement
  entries on the main locus). The issue calls this "essentially free" — the
  compiler already knows the count. One pinned placement = one
  `pthread`.
- **Cooperative pools** = distinct `cooperative(pool = X)` names — each is
  one shared OS thread.
- **Bus subjects** = distinct registered subject strings
  (subscribe/publish `subject.canonical()` + `topic` decls). Each is a
  router table entry.
- **File descriptors** = held-fd loci (`std::io::tcp::Stream`/`Listener`,
  `std::io::file::File`) + fd-opening calls.
- **Arena chunks** — out of scope (an allocator-internal count, better
  served by item #1's memory bounds).

## Two flavors

1. **Static counts (ceilings / CI gate).** A per-program tally of each
   resource. Zero false positives — it's a count. A budget file (like the
   `bench` baselines) gates regressions: a PR that takes pinned threads
   3 → 8, or bus subjects 12 → 40, trips "intentional?". **This is the
   slice that landed** (threads + pools + subjects — the cheap, structural,
   top-level-walk resources).
2. **Leak detection (reuses #1's dataflow).** A resource *acquired* in an
   unbounded loop / unboundedly-invoked fn that isn't released per
   iteration = an unbounded resource leak — the exact shape of #1's
   allocation leak, but for fds/threads. `alloc_summary`'s
   `unbounded_invoked` + `in_unbounded_loop` carry over directly.

## The gap for leak detection (why it's the next stage, not this one)

Hale fds are held by loci (`Stream`/`File`) that auto-close on dissolve.
So an fd "leak" isn't an open without a close — it's an fd-opening call
whose **result escapes** (stored to `self`, returned, pushed) so the
holding locus stays resident, *in an unbounded context*. That's the same
escape-into-unbounded-context obligation as #1 — but `alloc_summary`
currently tags escape only on **direct allocation sites**, not on **call
results** (an fd-open is a `Call`, recorded as a `CallEdge` without a
result-escape tag). Closing that gap — result-escape tagging on resource-
acquiring calls — is the leak-detection stage's real work, and it upgrades
#1 too (a factory call whose result escapes in a loop). Deferred, scoped
here.

## Open question (from the issue), resolved

> *Is there a generic "tracked resource" mechanism user code could extend,
> or does the language ship a fixed set?*

**Fixed set for v1.** The four language-visible resources above are
substrate concepts (threads, pools, subjects, fds) the compiler already
models; a user-extensible "tracked resource" framework is a speculative
generalization with no concrete consumer yet. Ship the fixed set; revisit
if a real use case (e.g. a user-defined pool of GPU contexts) appears.

## Landed: the count slice

`crates/hale-types/src/resource_budget.rs` — `budget_for_programs(&[&Program])`
tallies pinned threads, cooperative pools, and bus subjects by a top-level
walk (loci → `Placement` entries; bus members + `topic` decls →
`subject.canonical()`). Surfaced via `hale check --dump-resource-budget`.
No false positives (it's a count). Validated by unit tests.

## Staging

1. **Static counts** (threads + pools + subjects) — **landed**.
2. **fd / held-fd-locus counts** — add an expr-walk tally of fd-opening
   stdlib calls + held-fd locus instantiations. Still a count (zero-FP).
3. **Budget gate** — a `resource-budgets.json`-style ceiling file + a
   `--check-resource-budget` mode that fails when a count exceeds its
   declared ceiling. The CI-gate payoff.
4. **Leak detection** — result-escape tagging on resource-acquiring calls
   (the gap above), then reuse `leak_sites` to flag an fd/thread acquired
   in an unbounded context whose holder escapes. Warning-first, like #1.

## Risks

- **Counts drift without a gate.** A bare `--dump` is informational; the
  value is the gate (stage 3). Until then it's a manual check.
- **Leak detection's escape analysis** must avoid false positives on the
  common bounded case (open + dissolve per iteration) — same discipline as
  #1 (warning-first, zero-FP on the corpus before error).
