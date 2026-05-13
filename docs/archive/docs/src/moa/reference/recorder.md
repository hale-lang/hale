# `moa::Recorder`

Recording memory-owner. Subscribes to `runtime.event.**` (m94
wildcards) and saves every observed event into an F.22 heap slot
for later replay or inspection. Pairs with `moa::Replayer` for the
record-then-replay debugging surface.

## Signature

```aperio
locus Recorder {
    params {
        max_events: Int = 1024;
        event_count: Int = 0;
    }

    capacity {
        heap events of RuntimeEvent;
    }

    bus {
        /// ingest: save — appends every observed runtime event
        subscribe "runtime.event.**" as on_event of type RuntimeEvent;
    }

    fn count() -> Int;  // current recorded-event count
}
```

## Usage

Instantiate near the entry point of a debugging session; the
Recorder begins capturing as soon as its `birth()` completes:

```aperio
fn main() {
    let r = moa::Recorder { max_events: 4096 };

    // ... the watched app runs, emits runtime.event.* ...

    println("captured ");
    println(r.count());
    // (replay path lands when moa::Replayer's body ships in v1.x)
}
```

## What it captures

`Recorder` subscribes to the entire `runtime.event.**` family.
Events flow in this family when a runtime-introspection layer (the
debugger / simulator / audit tool) is publishing them. Specifically:

- `runtime.event.bus_send` — a watched app's `<-` send
- `runtime.event.lifecycle` — birth/run/drain/dissolve transitions
- `runtime.event.closure_violation` — closure assertion failures
- `runtime.event.scheduler` — scheduler-dispatch events

Each is wrapped in a `moa::RuntimeEvent` envelope. The Recorder
saves the envelope, not the underlying payload — see "Payload
size" below.

## Capacity

The `events` heap slot uses the F.22 `lotus_heap_*` substrate:
geometric chunk growth (initial 16 cells, doubling, capped at
4096 per chunk). Cells are freed wholesale when the Recorder's
arena dissolves; per-event individual free is not exercised in
the v1 implementation but the heap surface supports it.

The `max_events` param sets the upper bound. Events beyond the
cap are dropped silently; `count()` stops advancing. The drop
behavior is intentional — back-pressure is not a recorder's
concern.

## Payload size

The `moa::RuntimeEvent` envelope carries `payload_size` (the byte
size of the original event), not the payload itself. This is the
v1 compromise: cross-locus copy of arbitrary payload types
through a generic recorder would require runtime type-erasure
machinery that v1 doesn't ship. At v1.x, a typed-recorder variant
(or a `Bytes`-payload envelope) can capture full payloads when a
workload demands.

For the most common debug case — "what events fired, in what
order, on what subjects" — the envelope alone is sufficient.

## Ingest classification

`runtime.event.**` ingest is `save`: every event is recorded
verbatim into the heap slot. The Recorder is a **recording**
memory-owner under the MOA dichotomy (see `../properties.md`).
Replay-from-zero is trivial: same input → same recorded state.

## Limitations (v0)

- **Hardcoded subject pattern.** The wildcard subscribe is to
  `runtime.event.**` because Aperio's bus subscriptions use
  STRING_LIT subjects, not parameters. A v1.x variant accepting
  a subject-pattern param would let Recorder mirror any subject
  family (e.g. `book.**`). Until then, applications wanting to
  record a non-runtime family wrap Recorder in a forwarder locus
  that republishes onto `runtime.event.<concern>.*`.
- **Per-cell read access** through F.22 `cell.field` (shipped
  v1.x-2) works for inspection, but **iteration across the heap
  slot's live cells** does not yet have a v1 primitive.
  Streaming-replay implementations wait on a heap-walk addition
  (`first()` / `next(cell)`).
- **No serialization to disk.** All recorded events live in the
  Recorder's arena; they vanish when the locus dissolves. A
  serialized-export variant lands when a debugging workflow
  demands cross-session replay.

## v0 wiring status

`moa::Recorder` is declared in `moa/recorder.ap` but not yet
bundled into `MOA_AP_SOURCE`. The wiring is two single-line edits
in `crates/aperio-codegen/src/codegen.rs` (see the file header in
`moa/recorder.ap`).

## Cross-references

- `replayer.md` — the playback companion
- `types.md` — the `moa::RuntimeEvent` payload
- `../patterns/broadcast-snapshot.md` — a related pattern; Recorder
  is the canonical recording memory-owner, while broadcast-snapshot
  shows a projection memory-owner serving similar observers
- `spec/design-rationale.md` §F.22 — capacity slots
