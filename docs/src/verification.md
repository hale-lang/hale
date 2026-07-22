# Verification

Most languages ask you to *write* correct concurrent code and hope you
did. Hale takes a different bet: make incorrect *designs* fail to
compile, and model-check the runtime everything executes on. This page
is the honest account of what that buys you — and what it deliberately
doesn't.

## The substrate is model-checked

Hale's runtime, **lotus**, is C: pthreads and C11 atomics. Every
primitive in it with a cross-thread surface is transcribed into a model
and checked **exhaustively, under every legal interleaving**, with
[GenMC](https://github.com/MPI-SWS/genmc) — as a standing CI gate. A
race, use-after-free, or assertion failure in any model fails the build.

| Primitive | What's verified |
|---|---|
| Lock-free hashmap | the enter / drain / grow protocol |
| Mailbox monitor | the pinned-locus mutex hand-off |
| Bus queue | the cooperative-pool conditional lock |
| Arena subregion lock | the parent's child-slot freelist |

Each model carries a **negative control**: delete the synchronization
and GenMC reports the exact bug the real code prevents — proof the
check has teeth. (The per-thread chunk pool needs no model: it is
`__thread`, with no cross-thread surface.) Sanitizers catch races on
the paths your tests happen to hit; model checking catches the ones no
test reliably triggers — grow-during-drain, compact-then-grow. For a
language whose whole concurrency story is the bus, trusting the
substrate is the foundation everything else rests on.

## Your programs are data-race-free by design

Above the substrate, the language is shaped so application code can't
introduce a data race in the first place:

- **A typed bus instead of shared state.** Loci talk by publishing
  typed values to topics; the payload is *copied* into the receiver's
  region. There is no shared mutable cell to race on.
- **The single-threaded-method invariant.** Calling a locus's method
  from the wrong pool's thread is a compile error.
- **Vertical-only failure.** No lateral references between siblings; a
  failure travels up to a parent's `on_failure`, never sideways.

## Checked at build time

These run during `hale check` / `hale build`, on top of ordinary
type-checking.

**Bus-graph properties.** The bus topology is a typed graph, and the
compiler walks it. This is the analysis that is **on by default** and
fails the build:

- *orphan* topics (wired to only one end) — warning
- *cross-locus cycles* that can spin — warning
- *intra-locus re-entrant* self-publish (unbounded recursion) — **error**
- *backpressure* — an unthrottled publish in an unbounded loop — warning
- *subject type-mismatch* — two sites disagreeing on a payload type — **error**

**Design rules**, enforced as errors:

- **No locus-return** — a method may not hand back a managed locus (a
  Law-of-Demeter / CQRS / dependency-inversion violation caught in one
  rule).
- **Codec purity** — a bus codec's `encode` / `decode` must be pure;
  they may run off-thread.
- **`ring_layout` conformance** — a foreign shared-memory ring layout
  is checked for internal and cross-field consistency before a torn
  read is possible.

**Concurrency & placement**, keeping a program's placement coherent
with how the runtime dispatches:

- **Dead bus receiver** — a cooperative locus that subscribes to the
  bus *and* blocks in `run()`, so the blocking call monopolizes the
  pool thread and its handlers never fire — **error**.
- **Blocking call on a cooperative pool** — a blocking `run()`
  (`recv` / `accept` / `process::run`) on a pool that isn't
  `where async_io`; it holds the pool's thread and stalls
  co-scheduled loci — warning.
- **Cooperative pool starvation** — two or more loci on one
  cooperative pool whose `run()` bodies statically never return
  (`while true`, `while !self.draining`, a never-assigned flag): the
  pool runs each `run()` to completion in birth order, so the later
  ones never start — including the main locus's own `run()` when a
  forever-looping locus shares pool `main`. Bus handlers keep firing
  at sleep/yield drains, which makes the hang look like a healthy
  idle; the warning names every offender — warning.
- **Nested long-running child** — a non-`main` locus holding a
  params field of a locus type whose `run()` never returns; the fix
  is hoisting it to a `main` sibling with its own placement —
  **error**.
- **Unowned subscriber locus** — a bus-subscribing locus
  instantiated *non-owned* in another locus's method body, so it
  dissolves at scope exit before its subscription can fire —
  **error**.

**Memory-bound proofs** *(on by default).* Every `hale check` /
`hale build` runs the whole-program survey: the compiler's
escape/loop dataflow flags allocations that escape a per-message
handler or unbounded loop and **accumulate until the locus
dissolves** — with loop-ranking that *proves* a `while v < N`
counter bounded. Run-to-exit programs (a `main` with no `run` loop
and no bus handler) warn nothing — a script owes no bound proof.
`@unbounded fn` is the in-source carve-out for an acknowledged
site; `--no-warn-unbounded-alloc` opts a run out. Advisory today; a
hard error contract is the intended end state once the remaining
documented false-positive classes get their annotations. A separate
advisory also flags two **loop-scoped hot-path allocations** — a locus
or `BytesBuilder` instantiated per iteration, and an allocating `recv`
in a loop — steering toward a hoisted field / `recv_into`.

**Hot-path allocation budget** *(opt-in, hard error).* `@budget(alloc_per_call
= N)` on a fn is the dual of `@unbounded`: an explicit per-call
allocation ceiling the compiler enforces. It counts the arena
allocations it can see — literals, `@form` inserts, transitively
through resolved callees, plus the known-allocating `recv` family — and
**fails the build** if the fn allocates more than `N` per call (a
loop-nested allocation is unbounded per call). `N = 0` is the zero-alloc
certificate for a hot-path handler. The one allocation check that gates
the build, because you opted into it.

**Resource budgets** *(opt-in).* Static counts of file descriptors, OS
threads, cooperative pools, and bus subjects, with a
`--check-resource-budget budget.toml` ceiling gate for CI and fd-leak
detection.

## Invariants you declare, checked as it runs

The checks above are the compiler's. You can add your own with a
**closure** — a property a locus promises to keep, written as a
first-class block and audited by the runtime while the program runs:

```hale,fragment
closure balanced {
    self.debits ~~ self.credits within 0.01d;
}
```

`~~` is "approximately equal, within tolerance." When the invariant
breaks, it routes to the owner's failure handler (or, unhandled, stops
the program) — a declared property enforced by the substrate, not a
comment you hope holds. Closures are taught in full in
[When things fail](./services/failure.md#declaring-an-invariant-closure);
they're the runtime half of "verified by construction."

## What Hale does *not* claim

Hale is **not** a whole-program functional-correctness prover — that is
the world of CakeML and F\*. The guarantee here is narrower and
deliberately so: the **coordination** (the bus graph), the **substrate**
(the concurrent primitives), and **bounded resource use** are verified,
because those are the properties that must hold no matter what executes
the design — native, wasm, or a future target. Verification that
survives a change of substrate is the kind worth building on.

> The authoritative, exhaustive catalog of every compile-time check is
> [`spec/verification.md`](https://github.com/hale-lang/hale/blob/main/spec/verification.md).
> The verification roadmap that drove this work — now delivered — is
> [GitHub issue #18](https://github.com/hale-lang/hale/issues/18).
