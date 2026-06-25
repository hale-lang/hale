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

**Memory-bound proofs** *(opt-in).* "Bounded per epoch" only means
something for a long-lived process, so a script that allocates and
exits owes nothing and pays nothing. Annotate a long-lived locus
`@bounded` (or run the whole-program `--warn-unbounded-alloc` survey)
and the compiler's escape/loop dataflow flags allocations that escape a
per-message handler or unbounded loop and **accumulate until the locus
dissolves** — with loop-ranking that *proves* a `while v < N` counter
bounded. Advisory today; a hard `@bounded` contract is the intended end
state once in-scope false positives reach a durable zero.

**Resource budgets** *(opt-in).* Static counts of file descriptors, OS
threads, cooperative pools, and bus subjects, with a
`--check-resource-budget budget.toml` ceiling gate for CI and fd-leak
detection.

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
