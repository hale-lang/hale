# Memory-Owner Architecture

> The substrate that apps build on. Path prefix `moa::*`. Parallel to
> `std::*`; not part of stdlib; conceptually one layer below
> application code and one layer above the language itself.

## What MOA is

Memory-Owner Architecture (MOA) is the application-level pattern
that falls out when an Aperio app applies the framework's substrate
commitments coherently rather than fighting them. It is not a new
language feature; it is the shape that the existing primitives —
vertical-only contracts, lateral typed bus, copy-at-boundary,
F.22 capacity slots, projection class — make coherent when composed.

The four load-bearing properties are stated on the next page; every
pattern in `patterns/` and every library piece in `reference/` is a
specialization of those four into a recognizable shape.

## Why it is a substrate, not a style

MOA ships as a real layer of code at `/moa/` in the repository,
parallel to stdlib's `/crates/aperio-codegen/runtime/stdlib/`.
Substrate definitions resolve under the `moa::*` path prefix; their
declarations are bundled into every emitted Aperio binary.

A separate top-level prefix (rather than living under `std::*`)
makes the load-bearing claim visible: `std::*` is the language's
libraries — things every Aperio program *might* use. `moa::*` is
the architectural substrate — things any *stateful* Aperio app
*does* use, because they encode the shape state takes.

The split also creates a stable target for app-to-app interop:
two MOA apps built against the same `moa::*` surface and sharing
the bus subject conventions in `subjects.md` interoperate by
construction. Adding a third app means subscribing to its
substrate-typed events; no protocol negotiation, no
schema-matching, no glue.

## What lives in this subtree

- **`introduction.md`** — this page.
- **`properties.md`** — the four load-bearing properties.
- **`patterns/`** — recognized compositions:
  - `broadcast-snapshot.md` — public delta stream + on-demand
    snapshot ping. The default shape for req/resp under MOA.
  - `config-loader.md` — small CLI memory-owner read by the
    orchestrator at startup.
  - `private-streams.md` — per-recipient response streams; carve-out
    for privacy / volume / per-client state.
- **`reference/`** — the substrate's user-facing API:
  - `types.md` — `LocusId`, `BraidId`, `Tick`, `RuntimeEvent`.
  - `snapshotable.md` — `moa::Snapshotable` structural interface.
  - `clock.md` — `moa::Clock` tick generator.
  - `recorder.md` — `moa::Recorder` bus-traffic recorder.
  - `replayer.md` — `moa::Replayer` event playback.

## Reading order

If you have ten minutes and want the shortest viable path:
- `quickstart.md` — the one-page summary; five-step authoring,
  three patterns, anti-patterns.

If you have never composed a stateful Aperio app:
1. `quickstart.md` — orient.
2. `properties.md` — the four-property statement.
3. `patterns/broadcast-snapshot.md` — the most common shape.
4. `patterns/config-loader.md` — how `main()` stays an orchestrator.
5. `walkthroughs/market-book.md` — worked example.

If you are designing a new Aperio app right now:
1. `properties.md` — re-read the four properties.
2. Skim `patterns/` for shapes you recognize from your design.
3. Walk the five-step authoring process at the bottom of
   `properties.md`.
4. Reach for `reference/` modules when their concerns match yours
   (need a clock? `clock.md`. Want to record traffic for replay?
   `recorder.md`).
5. Before merge, walk `audit-checklist.md`.

If you are auditing existing code:
- `audit-checklist.md` — seven sections, ~30 min walkthrough.
- `glossary.md` — when terminology is unclear.

If you are confused by what's actually available vs. pending:
- `roadmap.md` — v0 / v0.x / v1.x / v2 status of every surface.

## Cross-references

- `notes/aperio-types-vs-loci.md` — the foundational axiom MOA
  builds on (types are for shapes, loci are for flow).
- `notes/aperio-seed.md` — the per-directory seed model that MOA
  apps compose under.
- `spec/styleguide.md` — the pattern catalog of six shape
  primitives; MOA disciplines how those compose.
- `notes/agent-onboarding/app-dev-brief.md` — read-this-first
  brief; routes state-bearing apps here.
- `spec/design-rationale.md` §F.22 — capacity slots, the storage
  discipline MOA depends on.
- `moa/MOA.md` (in the repo) — the foundational architecture doc;
  this docs subtree is a navigable view onto the same material.
