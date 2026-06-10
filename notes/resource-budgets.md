# Resource-budget tracking (GH #18 item 5)

Status: **count slice + fd-leak detection landed.** Written 2026-06-10. The
verification candidate that "complements #1" and reuses the shared dataflow
infra built for memory-bound proofs (#100/#101/#103). The issue frames it as
a *linter* — a useful "is this in range" signal + a CI gate ("this PR raised
the fd ceiling — intentional?"), not a proof.

> **Leak detection landed (the gap below is closed).** `alloc_summary`'s
> `CallEdge` now carries the **escape of the call's result** (the call-
> result analog of an allocation site's escape), so an fd-acquiring call
> (`std::io::file::open` / `tcp::connect`/`listen`/`accept`) whose result is
> stored resident (`self`) in an unbounded context (a per-message handler /
> a call in an unbounded loop) is flagged — the fd accumulates. Opt-in via
> `hale check --warn-resource-leak`; `resource_budget::resource_leak_diags`.
> Zero false positives on the corpus. This result-escape tagging also
> upgrades **item #1** (a factory call whose result escapes in a loop is now
> visible in the dump as `result=escaping=…`).

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

## The gap for leak detection (now closed)

Hale fds are held by loci (`Stream`/`File`) that auto-close on dissolve.
So an fd "leak" isn't an open without a close — it's an fd-opening call
whose **result escapes** (stored to `self`) so the holding locus stays
resident, *in an unbounded context*. That's the same escape-into-unbounded-
context obligation as #1 — but `alloc_summary` originally tagged escape only
on **direct allocation sites**, not on **call results** (an fd-open is a
`Call`, recorded as a `CallEdge`). **`CallEdge` now carries a result escape**
(threaded from the same `escape` context that tags alloc sites, including
the `let x = open(); … self.f = x;` indirection via the name pre-pass), so
the leak check filters resource-acquiring calls with a `self`-store result
in an unbounded context. A `Local` (let-scoped, dissolved per iteration)
holder is bounded and not flagged.

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
2. **Leak detection** — result-escape tagging on resource-acquiring calls
   + the unbounded-context filter — **landed** (`--warn-resource-leak`).
3. **Budget gate** — **landed.** `hale check <prog> --check-resource-budget
   <file.toml>` reads a TOML ceiling file and exits non-zero if any count
   exceeds its declared ceiling (the CI payoff: "this PR raised the
   thread/subject count — bump the ceiling if intentional"). Unknown keys
   error (typo protection); a resource with no declared ceiling is
   unconstrained. Format:
   ```toml
   pinned_threads    = 4
   cooperative_pools = 2
   bus_subjects      = 16
   ```
4. **fd counts** — **landed.** `fd_open_sites` tallies fd-opening call
   sites (`std::io::file::open` / `tcp::connect`/`listen`/`accept`) by
   reusing `alloc_summary`'s call edges (unambiguous qualified paths →
   zero FP); shows in the dump + gated by the ceiling. *Remaining:* direct
   held-fd locus instantiations (`tcp::Listener { }`) aren't counted — the
   alloc summary keeps only a struct's last path segment, so matching by
   name alone would risk colliding with a user type named `Listener` /
   `Stream` / `File`; counting those needs path-qualified struct matching.
   Now counted too (matched on the qualified path, so a user type named `Listener` doesn't collide) — fd acquisition = open-calls + held-fd-locus instantiations. Stage complete.

## Risks

- **Counts drift without a gate.** A bare `--dump` is informational; the
  value is the gate (stage 3). Until then it's a manual check.
- **Leak detection's escape analysis** must avoid false positives on the
  common bounded case (open + dissolve per iteration) — same discipline as
  #1 (warning-first, zero-FP on the corpus before error).
