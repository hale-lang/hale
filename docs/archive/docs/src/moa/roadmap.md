# v0 → v1.x roadmap

What MOA ships today, what's pending, and what each pending piece
unblocks. Use this page to set expectations: MOA is real
substrate at v0, but several pieces are declared-but-not-bundled
or skeleton-only pending compiler-session pickup. The
"declarative artifact" status is captured in each file's header
comment for direct discovery; this page summarizes for
orientation.

## Currently live (v0)

These are bundled into every Aperio binary and reachable under
`moa::*` paths today:

| Surface | Reachable as | Source |
|---|---|---|
| `LocusId` | `moa::LocusId` | `moa/types.ap` |
| `BraidId` | `moa::BraidId` | `moa/types.ap` |
| `Tick` | `moa::Tick` | `moa/types.ap` |
| `RuntimeEvent` | `moa::RuntimeEvent` | `moa/types.ap` |

Together these are the substrate payload types — domain-agnostic
records every MOA app uses for cross-app introspection and
runtime observation.

## Pending wiring (v0.x)

These are declared as valid Aperio source in `moa/` but not yet
bundled into `MOA_AP_SOURCE`. Each is a two-line compiler-session
pickup: one `include_str!` entry in the `concat!()` block; one
`(&["moa", "<Name>"], "__Moa<Name>")` row in
`MOA_PATH_RENAMES`. The file headers in `moa/*.ap` carry the
exact lines to add.

| Surface | Reachable as (target) | Source | What it unblocks |
|---|---|---|---|
| `Snapshotable` interface | `moa::Snapshotable` | `moa/snapshotable.ap` | F.20 polymorphic dispatch on the broadcast-snapshot pattern; lets a generic dispatcher accept any snapshotable memory-owner. |
| `Clock` locus | `moa::Clock` | `moa/clock.ap` | A canonical tick substrate for any timed locus; centralizes the swap point between real and mocked clocks. |
| `Recorder` locus | `moa::Recorder` | `moa/recorder.ap` | First-class bus-traffic recording for debug + simulator workflows. |
| `Replayer` locus | `moa::Replayer` | `moa/replayer.ap` | The playback companion to `Recorder`; skeleton only, see v1.x below. |

Pickup ordering doesn't matter beyond what each file's header
documents (no cross-dependencies between the four).

## v1.x — body fill-in

These have working surfaces today but skeleton or limited
implementations:

### `Replayer.run()` body

Needs one of two unblocks:
- **Heap-slot iteration primitives** — `events.first()`,
  `events.next(cell)` — let `Replayer` walk a `Recorder`'s heap
  slot directly. Requires cross-locus heap-slot access; F.22 v1.x
  roadmap item.
- **Serialized event-blob format** — `Recorder` emits a `Bytes`
  blob; `Replayer` consumes it. Decouples the pair; trades
  in-process access for a serialization layer.

Either path lands when a concrete simulator workload exercises
it.

### `Recorder` parameterized subject patterns

Today the subscribe subject is hardcoded to `runtime.event.**`.
A parameterized variant would let `Recorder` mirror any subject
family. Requires bus-subscribe parameterization at the language
level (subjects are currently `STRING_LIT`, not expressions).
v1.x grammar surface.

### `Clock` mocked variant

`moa::MockClock` (or similar) for deterministic simulation —
same `clock.tick` subject family, subscriber loci can't tell
which is publishing. Lands when a simulator workload demands
the swap.

### F.22 `Cell<T>` field IO

Already shipped for struct cells in v1.x-2; primitive-cell
field IO (e.g. `pool of Int` with `cell.value = 42`) is the
next increment. Unblocks the `BookL` ladder-array → capacity
lift in market-book, and similar patterns elsewhere.

### Ingest classification as grammar

The v1 doc-comment convention (`/// ingest: discard | save |
transform — <rationale>`) could become a grammar surface
(`ingest { ... }` block) if a workload exercises runtime audit
or if reviewers find the discipline hard to enforce by reading.
Until either fires, comments stay.

## v1.x — patterns yet to be documented

Three patterns are sketched in conversation but not yet
documented:

- **Dispatcher** (when routing IS state) — a memory-owner whose
  state is a routing table; saves registrations, transforms
  lookups, publishes to chosen targets. Earns its place when a
  concrete workload needs membership-based, policy-based, or
  lookup-based routing. The bus itself is the router for most
  cases.
- **Session establishment** — a long-lived variant of
  request/response where clients announce a stable id at birth,
  and the owner saves session registrations in a `heap sessions
  of SessionRef` slot. Layers on broadcast-snapshot when the
  three carve-out conditions (privacy / volume / per-client
  state) apply. Sketched in conversation; doc lands when the
  IDE work exercises it.
- **Cross-process MOA** — same patterns over `std::bus::tcp` or
  equivalent. The substrate's copy-not-pointer rule makes this
  free at the design level; the operational concerns
  (deployment, transport binding, perspective hot-load) need
  pages. Lands when fitter-applier-style cross-process apps
  reach beyond v0.

## v2 — speculative

Long-term directions that aren't on any milestone schedule but
are worth naming:

- **Compile-time MOA enforcement.** If memory-owners are
  identifiable at typecheck (capacity declaration present;
  ingest classification present), the compiler could refuse to
  compile state-bearing apps that violate MOA. Today this is
  reviewer-enforced via `audit-checklist.md`.
- **Substrate-aware visualization.** MOA's well-defined topology
  (one publisher per family; delta + snapshot envelopes; ingest
  classification) makes a generic visualizer feasible. An app
  that subscribes to `runtime.event.**` from any MOA app should
  be able to render its locus tower and its bus topology
  without per-app code. The IDE work touches this.
- **Cross-language MOA bridges.** Foreign codebases that
  expose MOA-compatible bus subjects (the conventions in
  `moa/subjects.md`) interoperate with Aperio apps without
  bespoke protocol code. The codebase-onboarder's reverse-MOA
  extraction (from foreign code to MOA shape) is the dual.

These are not commitments; they're directions the discipline
supports if the workload demands.

## Cross-references

- `moa/README.md` (in the repo) — current bundled / pending
  status with file-by-file inventory
- `introduction.md` — MOA's place in the layering
- `properties.md` — the four properties; what's tested by the
  current substrate vs. what's discipline-only
- `audit-checklist.md` — the review-time enforcement surface
  while compile-time enforcement is a v2 idea
- `spec/design-rationale.md` §F.22 — capacity slots, the main
  v1 → v1.x roadmap that MOA depends on
