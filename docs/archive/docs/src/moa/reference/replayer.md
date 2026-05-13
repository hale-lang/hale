# `moa::Replayer`

Orchestrator (with small state) that reads a recorded event stream
and re-publishes events on `runtime.event.replay.**`. Pairs with
`moa::Recorder` for the record-then-replay debugging surface.

## Status: v0 skeleton

`moa::Replayer` is declared in `moa/replayer.ap` with an empty
`run()` body. The full implementation lands when one of two
unblocking paths ships in v1.x:

- **Heap-slot iteration** — primitives like `events.first()` /
  `events.next(cell)` that let Replayer walk a Recorder's heap.
  Requires either cross-locus heap-slot access or a slot-iteration
  surface; both are F.22 v1.x roadmap items.
- **Serialized event-blob format** — Recorder emits a `Bytes`
  blob; Replayer consumes it. Decouples Recorder ↔ Replayer at
  the cost of a serialization layer.

The locus's shape is captured here so the pattern is recognizable
when a workload arrives.

## Signature (target)

```aperio
locus Replayer {
    params {
        cadence_ms: Int = 0;        // 0 = as fast as possible
        events_replayed: Int = 0;
    }

    bus {
        publish "runtime.event.replay" of type RuntimeEvent;
    }

    run() {
        // walks the source event stream; re-publishes each event
        // on runtime.event.replay; sleeps cadence_ms between
        // events if non-zero. Body deferred to v1.x.
    }
}
```

## Usage (target shape, v1.x)

```aperio
let rec = moa::Recorder { max_events: 4096 };
// ... watched app runs and feeds Recorder ...
let _rep = moa::Replayer {
    source: rec,           // reference; needs cross-locus access
    cadence_ms: 0,
    events_replayed: 0,
};
```

Subscribers to `runtime.event.replay.**` consume the events the
same way they would consume live `runtime.event.**` — only the
subject-family prefix differs, so observers can distinguish live
from playback.

## Cadence behavior

- `cadence_ms: 0` (default) — replays as fast as the scheduler
  allows. Useful for stress tests and "replay the trace, check
  the closures."
- `cadence_ms: N` — sleeps `N` milliseconds between consecutive
  events. Useful for live demos, visualization replays, and
  scenarios where downstream subscribers can't keep up at full
  speed.

A future variant (`speed: Float`) could replay at the original
real-time cadence multiplied by the speed factor; lands when a
visualization workload demands it.

## MOA role

Replayer is an **orchestrator with small state**. It holds a
counter (`events_replayed`) but doesn't own any of the recorded
data — that lives in the Recorder. Replayer's job is to walk and
re-publish, not to hold.

If a workload eventually needs Replayer to own the recorded events
directly (e.g., loading from disk into Replayer's own heap slot,
bypassing Recorder), the design shifts: Replayer becomes a
recording memory-owner (saves the loaded events) plus an
orchestrator (re-publishes them). The two-locus split (Recorder +
Replayer) keeps concerns clean; the merged shape works but
combines responsibilities.

## v0 wiring status

`moa::Replayer` is declared in `moa/replayer.ap` but not yet
bundled into `MOA_AP_SOURCE`. The wiring is two single-line edits
in `crates/aperio-codegen/src/codegen.rs` (see the file header in
`moa/replayer.ap`). The skeleton lands before the body so the
locus shape can be referenced from other documents.

## v1.x roadmap

1. **Heap-slot iteration primitives** — `events.first()`,
   `events.next(cell)`, plus the cross-locus access that lets
   Replayer reach into Recorder's slot.
2. **Run body** — walks the source, publishes each event with
   `cadence_ms` delay.
3. **Cadence-as-speed-multiplier variant** — preserve original
   real-time intervals scaled by a factor.
4. **Disk-backed Replayer** — loads a serialized event stream
   from `Bytes` (Recorder gains a paired `to_bytes()` method).

## Cross-references

- `recorder.md` — the recording companion
- `types.md` — the `moa::RuntimeEvent` payload type
- `../patterns/broadcast-snapshot.md` — a related pattern; Replayer
  serves the same "let observers reconstruct" purpose as on-demand
  snapshot, but for time-series rather than current-state data
- `spec/design-rationale.md` §F.22 — capacity slots
