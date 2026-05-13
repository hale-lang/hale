# Memory-Owner Architecture (MOA)

> Captured 2026-05-11. Names the architectural discipline that falls
> out when an Aperio app applies the framework's substrate
> commitments — vertical-only contracts, lateral typed bus,
> copy-at-boundary, F.22 capacity slots, projection class — coherently
> instead of fighting them. The discipline already implicit in
> `apps/market-book/`, `std::log`, and the codebase-onboarder
> family; this note states it.

## The axiom

> **State lives at one memory-owner per concern.**
> **Everything above is routing.**

That is:

- Every piece of state in an Aperio app has exactly one canonical
  locus that owns it. That locus is the *memory-owner* for that
  concern.
- Above each memory-owner sits a thin *orchestrator* layer of loci
  that route events, manage lifecycle, and present a small surface
  to siblings. Orchestrators carry no state.
- Observers of a memory-owner's state read it via *copied deltas* on
  the typed bus. Sharing is impersonated by copying; pointers never
  cross from one memory-owner's arena hierarchy into another.

There is no "shared state" in an MOA-shaped app. There is one
canonical author per concern and N typed observers downstream.

## The four properties

These four are co-constraining — each implies the others, and the
architecture is what falls out when all four hold.

**1. State lives at one memory-owner per concern.** Every concern
has exactly one locus that's the source of truth for it. The
memory-owner declares the F.22 capacity that holds the state, owns
the arena (slot 0) that backs it, and is the canonical publisher of
every delta about it. See `apps/market-book/book.ap` `BookL` — the
sole owner of the 8-level limit-order book; every observer reads
state via `book.snapshot.*` or `book.delta` deltas.

**2. Two disciplines declared per memory-owner.** *Capacity*
declares how state is stored: F.22 `capacity { pool X of T; heap Y
of T; }` slot declarations + projection class (rich / chunked /
recognition). *Ingest* declares how state is updated: a doc-comment
`// ingest: discard | save | transform — <rationale>` above each
`subscribe` line in the bus block. Together they let a reader infer
the locus's behavioral shape from declarations alone, without
reading the bodies. They are the locus's contract with its
substrate, parallel to how `contract { expose / consume }` is its
contract with parent and children.

**3. Orchestrators carry no state.** Above each memory-owner sits
a thin orchestrator (rich projection class, small arena,
routing/lifecycle only). Orchestrators don't publish deltas — only
*intents* (`agent.intent.camera`) and *control signals*
(`control.pause`). They carry no F.22 capacity slots. Their arenas
are small and freed wholesale at orchestrator dissolve.
`apps/market-book/main.ap` is the canonical orchestrator: it routes
argv, instantiates `MdGatewayL` + two `BookL`s, kicks the feed; it
holds no state.

**4. The bus is the only inter-concern channel.** Vertical
contracts (`expose` / `consume`) handle within-concern parent/child
wiring; the typed lateral bus handles cross-concern coordination.
The framework's vertical-only-flow rule plus copy-at-boundary
together mean pointers never cross from one memory-owner's arena
hierarchy into another. Deltas cross by value. The system has no
shared mutable state, anywhere.

## Why the four compose into one shape

They aren't a checklist — they co-constrain each other:

- Single-memory-owner forces deltas-as-communication; no one else
  *can* read the state directly.
- Deltas-as-communication forces capacity-discipline; the
  memory-owner must hold the state's full lineage to produce
  correct deltas.
- Capacity-discipline forces ingest-discipline as its dual; storage
  without consumption is dead weight; consumption without declared
  storage discipline is illegible.
- Capacity + ingest together force the orchestrator / memory-owner
  separation; mixing them in one locus creates conflicting
  responsibilities — the orchestrator's small arena fights the
  memory-owner's heavy capacity slots; the orchestrator's many
  routing concerns fight the memory-owner's single subject family.

So MOA is *what's coherent* given the framework's primitives. Other
shapes — orchestrators that grow state, sibling state-sharing via
shared pointer, multi-concern memory-owners — are *possible* in the
language but *fight* the substrate. The framework punishes you the
way Java punishes `goto`-style control flow or C punishes
unbalanced `malloc` / `free`. The language permits it; the
substrate makes it expensive.

## Two memory-owner kinds — recording vs projection

The ingest classifier reveals a second-order split:

> **Recording memory-owners** save deltas verbatim. Their state IS
> the log of received deltas. Replay-from-zero is trivial — same
> input → same state.

> **Projection memory-owners** transform deltas into derived state.
> Their state is a *view* of the input stream, not the stream
> itself. Replay-from-zero requires explicit seed-state capture.

`apps/market-book/book.ap` `BookL` is canonical projection — incoming
`snapshot.*` and `delta` events fold into a current 8-level ladder;
downstream subscribers see the *result*, not the log. A
conversation-history locus saving every message verbatim would be
canonical recording.

The distinction matters for closure tests (recording invariants
check "log is complete and ordered"; projection invariants check
"result is consistent with the driving stream") and for replay /
simulator infrastructure (recordings replay losslessly; projections
need seed-state metadata).

Most memory-owners in non-trivial apps are projections; recordings
are rarer. Some loci mix both — see `apps/market-book/book.ap`
where `snap_end` is `save` (records the snapshot-complete signal)
while `snap_begin` / `snap_level` / `delta` are `transform` (fold
into ladder state).

## Ingest classification — the discipline at v1

Every subscription handler in a memory-owner does exactly one of:

- **discard** — the delta is irrelevant, duplicate, or filtered out;
  no state change, no downstream publish.
- **save** — record the delta into owned state verbatim. The locus
  is *a recording* of these deltas. Provenance preserved.
- **transform** — apply the delta as a derived mutation; possibly
  emit downstream deltas as a result. The locus is *a projection*,
  not a recording.

At v1 the classification is a doc-comment above each `subscribe`
line in the locus's `bus` block:

```aperio
bus {
    /// ingest: save — appends to messages slot; provenance preserved
    subscribe "agent.message.response" as on_agent_response of type Message;

    /// ingest: transform — folds tower-node into last-tower diff state
    subscribe "source.tower.node.added" as on_node_added of type TowerNode;

    /// ingest: discard — placeholder for future filtering
    subscribe "runtime.event.scheduler" as on_scheduler of type RuntimeEvent;
}
```

The doc-comment is the audit surface. Grep for `// ingest: save`
finds every recording handler; grep for `// ingest: transform`
finds every projection handler. Future v1.x grammar could promote
this to enforced syntax (`ingest { ... }` block) if a workload
demands runtime audit; until then, comments do the work.

## The five-step authoring process

When new work lands in an MOA-shaped app, the author (human or
agent) proceeds:

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
   transform — <rationale>` in the bus block.
5. **Publish deltas, not state.** Define the subject family this
   memory-owner is canonical publisher for. One subject family per
   memory-owner. Use the conventions in `subjects.md`.

The agent and human collaborate by following the same five steps.
The substrate is the same for both; the process is the same for
both.

## Common patterns

Three recurring shapes are documented as standalone patterns in
the mdbook subtree at `docs/src/moa/patterns/`:

- **Broadcast + on-demand snapshot**
  (`docs/src/moa/patterns/broadcast-snapshot.md`) — the default
  request/response shape under MOA. Public delta + snapshot
  streams; clients subscribe first and ping a public request
  channel to trigger a snapshot. No correlation ids, no
  per-recipient streams; all subscribers are equal.
- **Config-loading orchestrator**
  (`docs/src/moa/patterns/config-loader.md`) — small CLI
  memory-owner (typically `std::cli::Resolver` or a domain-specific
  variant) sits as a leaf in the orchestrator's child set. Keeps
  `main()` stateless even when argv handling is non-trivial.
- **Private response streams**
  (`docs/src/moa/patterns/private-streams.md`) — the carve-out
  for privacy / volume / per-client owner state. Per-recipient
  subject suffixes; layered on top of the broadcast default, not
  replacing it.

The library code under `moa/` implements substrate-level helpers
that several of these patterns can reach for:
- `moa::Snapshotable` (`moa/snapshotable.ap`) — F.20 interface
  that broadcast-snapshot's memory-owners structurally satisfy.
- `moa::Clock` (`moa/clock.ap`) — centralizes the tick substrate
  so timed loci subscribe instead of running their own sleep.
- `moa::Recorder` + `moa::Replayer` (`moa/recorder.ap`,
  `moa/replayer.ap`) — record-then-replay for debugging and
  simulator modes.

Each library piece carries v0 wiring instructions in its file
header; see `moa/README.md` for the current bundled/pending status.

## How this differs from neighboring architectures

Briefly, because the contrast sharpens what's distinctive:

- **Actor model (Erlang / Akka)** — shares message-passing-as-
  coordination, but flat (no parent-child region hierarchy), GC'd,
  no capacity discipline. MOA is hierarchical + region-based +
  capacity-disciplined.
- **CQRS** — shares the write-side / read-side split, but typically
  as two whole subsystems. MOA does the split per concern: each
  memory-owner is its own CQRS unit.
- **Event sourcing** — shares the deltas-as-canonical-history
  property, but typically with a persisted log and projections
  rebuilt from it. MOA uses the runtime bus as delta transport;
  persistence is orthogonal.
- **Hexagonal / Clean Architecture** — shares the layering instinct;
  the boundary it enforces is "domain vs adapters." MOA's boundary
  is "memory-owner vs everyone else," finer-grained and physically
  backed by regions, scheduling, and type-checking.

The novel composition is *hierarchical-region-disciplined-actor-
with-typed-delta-bus*. No piece is new; the combination forces
choices the pieces individually don't.

## What MOA is NOT

- **Not a third primitive.** Aperio's two-primitive split (types
  for shape, loci for flow) is unchanged. MOA is a discipline for
  composing the two existing primitives, not a new declaration
  form. See `notes/aperio-types-vs-loci.md`.
- **Not a new keyword.** Capacity uses existing F.22 syntax. Ingest
  is doc-comment convention. No grammar change ships with MOA at
  v1.
- **Not compiler-enforced at v1.** The four properties are a
  discipline the substrate *makes coherent*; a v1 Aperio compiler
  will accept code that violates MOA without complaining. v1.x may
  promote some properties to enforced (e.g., capacity-block
  required on any locus that subscribes to >N bus subjects); for
  now, audit is by reader.
- **Not a replacement for the styleguide's six patterns.** The
  styleguide's six shape-primitives (app locus, namespace lotus,
  service locus, spawned child, shape type, free fn) are *how* you
  write any individual locus. MOA is *how those primitives
  compose* when there is state. MOA sits one level above the
  catalog, not beside it.

## Cross-references

In-repo source:
- `moa/subjects.md` — bus subject naming conventions for
  interoperable MOA apps
- `moa/README.md` — file inventory and v0 wiring backlog
- `moa/types.ap` — the substrate payload types (`LocusId`,
  `BraidId`, `Tick`, `RuntimeEvent`)
- `moa/snapshotable.ap`, `moa/clock.ap`, `moa/recorder.ap`,
  `moa/replayer.ap` — library declarations (declarative
  artifacts pending v0 wiring)

Navigable mdbook:
- `docs/src/moa/introduction.md` — landing page for the docs
  subtree; same axiom from a different reading angle
- `docs/src/moa/properties.md` — the four properties as a
  navigable page
- `docs/src/moa/patterns/` — broadcast-snapshot, config-loader,
  private-streams
- `docs/src/moa/reference/` — per-type and per-locus reference
  pages

Foundational notes and spec:
- `notes/aperio-types-vs-loci.md` — the axiom MOA builds on
- `notes/aperio-seed.md` — the seed model; MOA shapes any
  stateful seed
- `spec/styleguide.md` — the pattern catalog MOA disciplines
- `notes/agent-onboarding/app-dev-brief.md` — read this first
  brief; routes state-bearing apps to MOA
- `spec/design-rationale.md` §F.22 — capacity slots, which MOA
  depends on for storage discipline

Worked examples:
- `apps/market-book/` — `MdGatewayL` = recording memory-owner,
  `BookL` = projection memory-owner, `main()` = orchestrator
