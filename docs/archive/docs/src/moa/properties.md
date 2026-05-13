# The four properties

MOA is defined by four properties. They are co-constraining: each
implies the others, and the architecture is what falls out when all
four hold. State-bearing Aperio apps that violate any of them fight
the substrate, the way `goto`-style control flow fights structured
programming or unbalanced `malloc`/`free` fights region-based memory.

## 1 — State lives at one memory-owner per concern

Every piece of state in an Aperio app has exactly one canonical
locus that's the source of truth for it. That locus is the
**memory-owner** for the concern. It declares the F.22 capacity
that holds the state, owns the arena (slot 0) that backs it, and
is the canonical publisher of every delta about it.

There is no shared state. Observers of a memory-owner's state read
it via copied deltas on the typed lateral bus. Pointers never cross
from one memory-owner's arena tree into another.

> **Example.** `apps/market-book/book.ap`'s `BookL` owns the 8-level
> limit-order ladder. No other locus reads ladder cells directly;
> every observer subscribes to `book.snapshot.*` or `book.delta` and
> reconstructs the ladder from copied messages.

## 2 — Two disciplines declared per memory-owner

Each memory-owner declares its behavioral shape through two
declarations on the locus body:

**Capacity** declares storage. F.22 `capacity { pool X of T; heap
Y of T; }` slots plus the locus's projection class (rich / chunked
/ recognition) together name how the state is laid out and how it
is freed.

**Ingest** declares consumption. A doc-comment `/// ingest:
discard | save | transform — <rationale>` above each `subscribe`
line in the bus block classifies how the locus responds to that
subject.

Together, the two disciplines let a reader infer the locus's
behavioral shape from declarations alone — without reading the
method bodies. They are the locus's contract with the substrate,
parallel to how `contract { expose / consume }` is the locus's
contract with parent and children.

> **Example.** `BookL`'s bus block carries three `/// ingest:
> transform` annotations (the snapshot-folding subscriptions) and
> one `/// ingest: save` (the in-snapshot flag flip). A reader sees
> immediately which deltas mutate ladder state and which only
> change a flag.

## 3 — Orchestrators carry no state

Above each memory-owner sits a thin **orchestrator** layer of loci
that route events, manage lifecycle of their memory-owning children,
and present a small surface to siblings. Orchestrators:

- Use projection class `rich` (small fixed child set; freed
  wholesale at parent dissolve).
- Declare no F.22 capacity slots.
- Publish only *intents* (`agent.intent.camera`) and *control
  signals* (`control.pause`) — never deltas of owned state.

If an orchestrator finds itself accumulating state — a cache, a
counter, a registry — it has stopped being an orchestrator; promote
the concern to its own memory-owner child or refactor.

> **Example.** `apps/market-book/main.ap` is canonical orchestrator
> shape: routes argv, instantiates one `MdGatewayL` and two `BookL`s,
> kicks the synthetic feed, runs assertions through the books'
> contract surface. Holds no state of its own.

## 4 — Bus is the only inter-concern channel

Vertical contracts (`expose` / `consume`) handle within-concern
parent/child wiring; the typed lateral bus handles cross-concern
coordination. The framework's vertical-only-flow rule plus
copy-at-boundary together mean pointers never cross from one
memory-owner's arena hierarchy into another. Deltas cross by value.

This makes the routing graph a tree at every observation moment:
for any subject family, there is exactly one canonical publisher.
The data-flow graph of an MOA app is audit-able by inspection — for
any concern, you can answer "who is the source of truth for X?" in
one hop.

> **Example.** `book.*` has one publisher (`MdGatewayL`). The two
> `BookL` instances and any future observer subscribe; no `BookL`
> ever publishes `book.*` itself, even though it could
> syntactically. The conservatism is what makes the bus topology a
> coherent audit surface.

## Why the four compose

The four properties are not a checklist — they co-constrain each
other:

- Single-memory-owner-per-concern forces deltas-as-communication;
  no one else *can* read the state directly.
- Deltas-as-communication forces capacity-discipline; the
  memory-owner must hold the state's full lineage to produce
  correct deltas.
- Capacity-discipline forces ingest-discipline as its dual; storage
  without consumption is dead weight; consumption without declared
  storage discipline is illegible.
- Capacity + ingest together force the orchestrator / memory-owner
  separation; mixing them creates conflicting responsibilities.

So MOA is *what's coherent* given the framework's primitives. Other
shapes are *possible* in the language but fight the substrate.

## Two memory-owner kinds — recording vs projection

The ingest classifier reveals a second-order split worth naming:

**Recording memory-owners** save deltas verbatim. Their state IS
the log of received deltas. Replay-from-zero is trivial — same
input → same state.

**Projection memory-owners** transform deltas into derived state.
Their state is a *view* of the input stream, not the stream itself.
Replay-from-zero requires explicit seed-state capture.

The distinction matters for closure tests (recording invariants
check "log is complete and ordered"; projection invariants check
"result is consistent with the driving stream") and for replay /
simulator infrastructure (recordings replay losslessly; projections
need seed-state metadata).

Most memory-owners in non-trivial apps are projections; recordings
are rarer. Some loci mix both — `BookL` is mostly projection
(snapshot folding) with one save subscription (the in_snapshot
flag flip).

## The five-step authoring process

When new work lands in an MOA-shaped app, the author proceeds:

1. **Identify the concern.** What new state or new behavior is
   involved?
2. **Find or designate the memory-owner.** Is there an existing
   memory-owner for this concern? If yes, extend it. If no, create
   one as a child of the right orchestrator — or as a top-level
   memory-owner if the orchestration concern is trivial.
3. **Declare the capacity.** What storage does the new state need?
   Arena slot 0 by default; add F.22 `pool` or `heap` slots for
   recyclable or growable state. Pick the projection class.
4. **Classify the ingest per subscription.** For each bus subject
   the memory-owner consumes, write `/// ingest: discard | save |
   transform — <rationale>` above the subscribe line.
5. **Publish deltas, not state.** Define the subject family this
   memory-owner is canonical publisher for. One subject family per
   memory-owner. Use the conventions in `moa/subjects.md`.

The substrate is the same for the agent and for the human; the
process is the same for both.
