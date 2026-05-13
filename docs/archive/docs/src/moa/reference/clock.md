# `moa::Clock`

Substrate clock-tick generator. Self-contained memory-owner that
publishes `clock.tick` at a configured interval. Any locus that
needs periodic activation subscribes to `clock.tick` instead of
running its own sleep loop.

## Signature

```aperio
locus Clock {
    params {
        cadence_ms: Int = 1000;  // tick interval, milliseconds
        ticks: Int = -1;         // -1 = forever; N = exactly N ticks
        seq: Int = 0;            // monotonic counter
    }

    bus {
        publish "clock.tick" of type Tick;
    }

    run() {
        // emits ticks until `ticks` reached or drained
    }
}
```

## Usage

Instantiate `Clock` once near app startup, then any timed concern
subscribes to its tick stream:

```aperio
fn main() {
    let _c = moa::Clock { cadence_ms: 100, ticks: -1 };
    let _p = PollerL { };
}

locus PollerL {
    bus {
        /// ingest: transform — advances poll state per tick
        subscribe "clock.tick" as on_tick of type moa::Tick;
    }
    fn on_tick(t: moa::Tick) {
        // poll work...
    }
}
```

## Why centralize the clock

Three reasons a single `Clock` substrate beats per-locus sleep loops:

- **One canonical time source.** Every subscriber sees the same
  monotonic timeline. No per-locus drift; no cross-scheduler
  inconsistency.
- **Simulator-friendly.** Production runs against real time;
  simulation runs against a mocked clock. The substrate is the
  swap point — application loci subscribe to `clock.tick` and
  don't know whether the publisher is real or mocked.
- **Closure-test alignment.** Closure assertions with `epoch tick`
  fire at every `Clock` tick automatically; no per-locus tick
  scheduling.

## Params

### `cadence_ms`

Tick interval in milliseconds. Default 1000 (one tick per second).
Sub-millisecond cadences are not supported at v1 — the
`std::time::sleep` substrate operates on millisecond granularity;
finer-grained scheduling waits on a v1.x runtime addition.

### `ticks`

Total ticks to emit before draining. `-1` (default) means run
forever; the locus drains when its scope dissolves. A positive `N`
emits exactly N ticks then proceeds through normal lifecycle drain.
Useful for time-bounded tests that don't want to run a separate
shutdown signal.

### `seq`

Per-clock monotonically-increasing sequence number stamped on each
`Tick` payload. Subscribers detect dropped ticks via seq gaps. The
default starts at 0; a non-zero default lets a restart resume from
a known sequence baseline (advanced usage; most apps leave it at 0).

## Tick payload

Each `clock.tick` event carries a `moa::Tick`:

```aperio
type Tick {
    now_ns: Int;
    seq: Int;
}
```

`now_ns` is `std::time::monotonic()` at the moment the tick is
emitted. `seq` is the Clock's own sequence counter. See
`types.md` for the full Tick reference.

## v0 wiring status

`moa::Clock` is declared in `moa/clock.ap` but not yet bundled
into `MOA_AP_SOURCE`. The wiring is two single-line edits in
`crates/aperio-codegen/src/codegen.rs` (see the file header in
`moa/clock.ap`). Until then, applications wanting periodic ticks
roll their own sleep loop and document the friction.

## v1.x roadmap

- **`moa::MockClock`** for deterministic simulation. Same subject
  family (`clock.tick`); subscribers don't know which is publishing.
  Lands when a simulator workload exercises the swap.
- **Sub-millisecond cadences** waiting on a runtime addition for
  finer-grained scheduling.

## Cross-references

- `types.md` — the `moa::Tick` payload
- `spec/runtime.md` — monotonic-only scheduling rules
- `../../std/time.md` — `std::time::sleep` and `std::time::monotonic`
