# Mix pinned + cooperative threads

Aperio's concurrency model is bimodal: most loci default to
`: schedule cooperative` and share one scheduler thread; loci
annotated `: schedule pinned` own their own OS thread. The
two coexist in one program — cooperative loci yield to each
other at substrate cells, pinned loci run independently, and
the bus crosses thread boundaries through a per-pinned-locus
mailbox.

This recipe walks through the canonical "two loci on their own
threads, each running cooperative work within" shape.

## The schedule annotations

```aperio
locus Fast       { /* default = : schedule cooperative */ }
locus Latency    : schedule cooperative          { /* explicit */ }
locus Ingest     : schedule pinned               { /* own thread */ }
locus PinnedCore : schedule pinned(core = 3)     { /* + CPU pin */ }
```

`pinned(core = N)` additionally CPU-affinitizes the thread on
Linux. On platforms without `sched_setaffinity`, the `core`
arg is parsed and ignored.

## A worked two-thread example

Two pinned loci, each owning a thread; both publish to a
cooperative aggregator locus that runs on the main scheduler.

```aperio
type Sample { source: String; value: Int; }
topic Samples { payload: Sample; }

// --- Worker A: own thread, simulates a polling loop. -----
locus WorkerA : schedule pinned {
    bus { publish Samples; }
    run() {
        let mut i = 0;
        while i < 3 {
            Samples <- Sample { source: "A", value: i };
            std::time::sleep(80ms);
            i = i + 1;
        }
    }
}

// --- Worker B: own thread, faster cadence. ---------------
locus WorkerB : schedule pinned {
    bus { publish Samples; }
    run() {
        let mut i = 100;
        while i < 103 {
            Samples <- Sample { source: "B", value: i };
            std::time::sleep(50ms);
            i = i + 1;
        }
    }
}

// --- Aggregator: cooperative; receives from both. --------
locus Aggregator {
    params { count: Int = 0; }
    bus { subscribe Samples as on_sample; }
    fn on_sample(s: Sample) {
        self.count = self.count + 1;
        println("aggregator: got ", s.source, "/", to_string(s.value),
                " (total=", to_string(self.count), ")");
    }
}

fn main() {
    Aggregator { };              // subscriber first
    WorkerA { };                 // pinned thread #1
    WorkerB { };                 // pinned thread #2
    std::time::sleep(500ms);     // keep main alive long enough to drain
}
```

What runs where:

| Locus | Thread |
|---|---|
| `Aggregator` | main scheduler (cooperative pool) |
| `WorkerA`    | its own pthread |
| `WorkerB`    | its own pthread |

Output (timing-dependent on the exact interleave, but every
sample arrives at the aggregator):

```
aggregator: got B/100 (total=1)
aggregator: got A/0 (total=2)
aggregator: got B/101 (total=3)
aggregator: got A/1 (total=4)
...
```

## How the bus crosses threads

When a pinned locus publishes to a cooperative subscriber (or
vice versa), the runtime:

1. Resolves the subscriber's locus and notices its mailbox
   pointer (every pinned subscription gets a per-locus
   bounded ring buffer guarded by a mutex + condvar).
2. **Inline-copies the typed payload into a mailbox slot**
   before signaling — no shared-arena pointer crosses the
   thread boundary.
3. Broadcasts the condvar; the destination thread wakes,
   pops, copies the payload into **its own** arena, and
   invokes the handler.

The arenas stay single-threaded territory. Aperio commits to
"no shared mutable state across threads" structurally — the
two copies (publisher arena → mailbox → subscriber arena) are
the price.

## Pinned-vs-cooperative tradeoffs

| You want | Reach for |
|---|---|
| The default. Almost everything. | cooperative |
| Latency-sensitive work (real-time ingest, tick handling) | pinned |
| Long-running CPU-bound loops that shouldn't yield | pinned |
| Predictable cadence on a specific CPU core | `pinned(core = N)` |
| Anything else | cooperative |

The rule of thumb: pinned is for *I shouldn't share the
scheduler thread*. If sharing is fine — even for a
relatively-busy locus — cooperative is the right answer. The
substrate yields between handler invocations and between
lifecycle transitions; you don't need pinned to "let other
loci run."

## What you can't do

- **No `: schedule greedy`.** A locus that "shares the
  scheduler but never yields between handlers" would be a
  third class; the substrate refuses it. Cooperative already
  guarantees handler atomicity. If you don't want to yield
  between cells, you don't want to share the scheduler —
  use pinned.
- **No mid-handler yield in cooperative.** Within one handler
  body, the cooperative scheduler does not preempt. If you
  need to yield mid-work, factor into multiple handlers or
  use `std::time::sleep` / explicit `yield;`. (`time::sleep`
  folds in the cooperative bus drain after it returns, so a
  cooperative subscriber looping `while { sleep; ... }`
  delivers cross-thread bus traffic mid-loop without needing
  an explicit `yield;`.)
- **No shared mutable state.** No `Arc<Mutex<T>>`-shaped
  primitive. Cross-thread coordination is bus-shaped.

## Pinned-only `run()` at v1?

Earlier prototypes of pinned scheduling restricted pinned
loci to a single `run()` method (no bus, no other lifecycle
methods). The full pinned lifecycle — including the
cross-thread mailbox shown above — shipped as part of the
2026 substrate work and is the v1 surface. A pinned locus can
declare any combination of `birth` / `accept` / `run` /
`drain` / `dissolve`, plus `bus { subscribe ... publish ... }`.

## See also

- [Lifecycle & time](../concepts/lifecycle-time.md) — schedule
  classes, yield points, drain cascade.
- [The bus](../concepts/the-bus.md) §"Cross-thread bus
  semantics" — the concept-level treatment.
- [Run a topic across binaries](./multi-binary-bus.md) —
  pinned threading inside one binary is the in-process
  analogue of multi-binary deployment; same code shape, one
  scope wider.
