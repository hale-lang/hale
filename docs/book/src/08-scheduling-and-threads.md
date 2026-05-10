# Scheduling and threads

Up to this point every locus in this book has run on a single
thread. The bus dispatched messages, lifecycles cascaded,
closures fired — all sequentially, all on one thread of
execution. That is one of the two scheduling regimes Aperio
ships, called *cooperative*. This chapter introduces the other:
*pinned* — loci that run on their own OS threads, optionally
bound to a specific CPU core, with cross-thread bus mailboxes
that let cooperative and pinned loci coordinate.

The substrate is **bimodal**: every locus is either cooperative
or pinned. There is no third mode. (The design rationale for
the bimodality, and why a "greedy" or "shared" mode was
considered and rejected, lives in `spec/design-rationale.md`
and below in *Why two modes, not three*.)

This chapter covers what pinned loci can do, when to reach for
them, the cross-thread bus, the `pinned(core = N)` affinity
annotation, the `yield` operator, and how multi-core Aperio
programs are built.

## Cooperative — the default

Every locus written so far in this book has been *cooperative*.
The annotation is implicit, but it can be declared explicitly:

```aperio
locus Worker : schedule cooperative {
    run() {
        println("cooperative: shared thread, handler-atomic");
    }
}
```

A cooperative locus runs on a shared scheduler thread alongside
all other cooperative loci in the same process. The runtime
yields between *substrate cells* — each cell is one of:

- A bus handler invocation (`on_X(payload)` runs to completion;
  the next cell can be a different locus's handler).
- A lifecycle transition (`birth` / `run` / `drain` /
  `dissolve`).
- A `time::sleep` call (yields cooperatively).
- An explicit `yield;` statement (covered below).

Within a cell — within a single handler body or lifecycle method
body — execution is **atomic** with respect to other cooperative
loci. A handler does not need to lock against another handler's
state because the substrate guarantees no other cell runs until
this one returns.

This is the right model for most Aperio programs. The substrate
is doing real work: keeping handler bodies atomic, freeing the
programmer from concurrency hazards inside handlers, scheduling
fairly across loci.

## Pinned — own a thread

When cooperative scheduling is the wrong shape — when you need
true parallelism, latency-critical paths, work that cannot
yield often — annotate a locus as pinned:

```aperio
locus PinnedJob : schedule pinned {
    birth() {
        println("pinned.birth: setting up on the pinned thread");
    }
    run() {
        time::sleep(30ms);
        println("pinned.run: doing work on the pinned thread");
    }
    drain() {
        println("pinned.drain: finishing pending work");
    }
    dissolve() {
        println("pinned.dissolve: tearing down on the pinned thread");
    }
}

fn main() {
    PinnedJob { };
    println("main: spawned PinnedJob; will join at scope exit");
}
```

A pinned locus owns its own OS thread. The runtime spawns a
`pthread` at the locus's instantiation; the locus's full
lifecycle (`birth` → `run` → `drain` → `dissolve`, in order,
each only if declared) runs on that thread; when the lifecycle
completes, the thread exits; the spawning scope joins the
thread before the locus's arena is torn down.

The output of the program above demonstrates true parallelism —
the `main: spawned PinnedJob` line lands between the pinned
locus's `birth` print and its `run` print, because while the
pinned thread is sleeping, the main thread is doing other work:

```text
pinned.birth: setting up on the pinned thread
main: spawned PinnedJob; will join at scope exit
pinned.run: doing work on the pinned thread
pinned.drain: finishing pending work
pinned.dissolve: tearing down on the pinned thread
```

A pinned locus's handler body, like a cooperative one, is
atomic — but the boundary is now per-thread. A pinned locus can
run *in parallel with* a cooperative locus or with another
pinned locus, because they live on different threads.

## Cross-thread bus mailboxes

Pinned loci can subscribe to and publish on the same bus as
cooperative ones. The runtime arranges the cross-thread plumbing
automatically:

```aperio
type Tick {
    n: Int;
    label: String;
}

locus PinnedSubscriber : schedule pinned {
    bus {
        subscribe "tick" as on_tick of type Tick;
    }

    birth() {
        println("pinned subscriber: birth on pinned thread");
    }

    fn on_tick(t: Tick) {
        println("pinned subscriber: got tick #", t.n, " (", t.label, ")");
    }

    dissolve() {
        println("pinned subscriber: dissolve on pinned thread");
    }
}

locus Publisher {
    bus {
        publish "tick" of type Tick;
    }

    birth() {
        "tick" <- Tick { n: 1, label: "first" };
        "tick" <- Tick { n: 2, label: "second" };
        "tick" <- Tick { n: 3, label: "third" };
    }
}
```

Mechanics under the hood:

- Each pinned subscriber has a per-locus **mailbox**: a bounded
  ring buffer guarded by a mutex + condvar.
- When the cooperative `Publisher`'s `<-` runs, the bus dispatch
  path notices the subscriber's registration carries a mailbox
  pointer, copies the typed payload **inline** into a mailbox
  slot, and broadcasts the condvar.
- The pinned thread is sitting in a drain loop on its mailbox,
  blocked on the condvar. It wakes, pops the cell, copies the
  payload from the inline slot into **its own arena**, and
  invokes the handler.
- The handler runs on the pinned thread; the publisher's
  cooperative scheduler is unaffected.

The substrate cost lives at the *layer boundary* — the mailbox
lock, the inline payload's two memcpy moves. Each arena stays
single-threaded territory. Cooperative on one side of the
mailbox; pinned on the other; no third "shared-arena" mode.

### Coordinated shutdown

When `main` returns or a `drain` cascade reaches a pinned
subscriber, the runtime signals shutdown on the subscriber's
mailbox. The pinned thread's drain loop observes a "no more
cells, shutdown requested" condition, breaks its loop, runs its
declared `drain` and `dissolve` bodies, and exits the thread.
The spawning scope joins the pthread before destroying the
locus's arena.

In-flight cells in the mailbox flush before the loop returns —
shutdown does not race past unprocessed messages.

## CPU affinity: `pinned(core = N)`

For latency-critical work, the runtime can bind a pinned
locus's thread to a specific logical CPU:

```aperio
locus CoreZeroWorker : schedule pinned(core = 0) {
    run() {
        time::sleep(20ms);
        println("worker: ran on core 0 (or fallback if unavailable)");
    }
}

locus CoreOneWorker : schedule pinned(core = 1) {
    run() {
        time::sleep(20ms);
        println("worker: ran on core 1 (or fallback if unavailable)");
    }
}

fn main() {
    CoreZeroWorker { };
    CoreOneWorker { };
    println("main: spawned two pinned workers on different cores");
}
```

`schedule pinned(core = N)` asks the runtime to call
`pthread_setaffinity_np` on the spawned thread, binding it to
logical CPU `N`. This is the *extreme case* the bimodal
scheduler vision allowed for: ordinary pinned loci let the OS
scheduler pick a core; `pinned(core = N)` loci own the core.

The semantics are **best-effort**:

- If the requested core does not exist (the binary is running
  on a CI box with fewer cores than the source declares), the
  runtime falls back to ordinary OS scheduling — the locus
  still runs, it just is not core-bound.
- If `pthread_setaffinity_np` returns an error (lack of
  permission, etc.), same fallback.
- The runtime never refuses to start a binary because of an
  affinity request it cannot fulfill.

This matches the substrate's "best-effort, predictable" stance:
a workload that benefits from core pinning gets it when the OS
allows; a workload that runs in a constrained environment still
runs.

## `yield`: explicit cooperative-cell boundary

Inside a cooperative locus's body, the substrate inserts cell
boundaries automatically (handler exit, lifecycle transition,
bus dispatch, `time::sleep`). For long-internal-loop bodies
that want to let pending bus events fire mid-body, there is an
explicit escape hatch: `yield;`.

```aperio
locus TickProducer {
    bus {
        publish "ticks" of type Tick;
    }

    run() {
        // Without `yield`, all 3 publishes would enqueue first
        // and all 3 logs would fire at scope-exit drain. With
        // `yield`, each batch flushes between publishes — same
        // observable ordering as today, but EXPLICIT about
        // when the substrate cell boundary is.
        "ticks" <- Tick { n: 1 };
        yield;
        println("--- after first yield ---");
        "ticks" <- Tick { n: 2 };
        yield;
        println("--- after second yield ---");
        "ticks" <- Tick { n: 3 };
    }
}
```

`yield;` lowers to a bus-queue drain at that point — every
enqueued substrate cell pops and runs before the surrounding
code continues. It is a no-op on pinned loci (which do not
share a cooperative queue).

`yield` is the *rare* tool. The substrate's automatic cell
boundaries handle almost every case. Reach for `yield` when
you have a long inner loop and want pending events to fire
mid-body for ordering reasons.

## When to choose pinned over cooperative

A short decision guide:

| Reach for | When |
|---|---|
| **`schedule cooperative`** (or omit) | The default. Most loci. Handler bodies run to completion atomically; the substrate yields between cells. |
| **`schedule pinned`** | True parallelism is needed: a long-running computation that should not yield, a hard real-time path, work that already has its own thread of control. |
| **`schedule pinned(core = N)`** | Latency-critical work: a market-data ingest path, a high-priority audit loop, a benchmark harness where core affinity matters. |

In practice, an Aperio program is mostly cooperative loci with
a small number of pinned loci at the edges where they are
load-bearing — typically one pinned locus per CPU-bound or
latency-critical responsibility, with cross-thread bus
mailboxes connecting them to the cooperative interior.

## Building a multi-core program

A multi-core Aperio program is a collection of loci where some
are pinned to different cores. There is nothing special about
the program's outer structure — it is the same `main` + locus
constructions you have used throughout this book — but the
schedule annotations create real OS-level parallelism:

```aperio
type Sample {
    value: Int;
}

// One pinned locus per core, each doing an isolated piece of
// computation. The cooperative coordinator collects results
// over the bus.

locus Worker0 : schedule pinned(core = 0) {
    bus {
        subscribe "work" as on_work of type Sample;
        publish   "result" of type Sample;
    }

    fn on_work(s: Sample) {
        // CPU-bound work runs on core 0.
        let computed = s.value * 2;
        "result" <- Sample { value: computed };
    }
}

locus Worker1 : schedule pinned(core = 1) {
    bus {
        subscribe "work" as on_work of type Sample;
        publish   "result" of type Sample;
    }

    fn on_work(s: Sample) {
        let computed = s.value * 2;
        "result" <- Sample { value: computed };
    }
}

locus ResultCollector {
    params {
        received: Int = 0;
    }

    bus {
        subscribe "result" as on_result of type Sample;
    }

    fn on_result(s: Sample) {
        self.received = self.received + 1;
        println("got result #", self.received, ": ", s.value);
    }
}

locus Coordinator {
    bus {
        publish "work" of type Sample;
    }

    birth() {
        // Both Worker0 and Worker1 subscribe to "work"; the
        // bus delivers each publish to every subscriber, so
        // each Sample is processed twice — once on core 0,
        // once on core 1.
        "work" <- Sample { value: 10 };
        "work" <- Sample { value: 20 };
        "work" <- Sample { value: 30 };
    }
}

fn main() {
    ResultCollector { };
    Worker0 { };
    Worker1 { };
    Coordinator { };
}
```

The pieces:

- **Two pinned workers**, each on its own core. Each subscribes
  to `"work"` and publishes on `"result"`. They do CPU-bound
  computation in parallel, on real cores.
- **A cooperative collector** that subscribes to `"result"` and
  prints. The collector runs on the cooperative scheduler;
  cross-thread mailboxes deliver each worker's result to it.
- **A cooperative coordinator** that publishes the input
  stream. Both workers see every input (the bus fans out
  per-subject).

When this runs, both pinned workers process every published
`Sample` in parallel. The result collector receives the
combined output on the cooperative thread, prints in arrival
order. The OS schedules the workers on their pinned cores; the
cooperative scheduler runs the collector and the coordinator
on its shared thread.

Scaling out is the same shape with more workers and more cores.
A program targeting eight cores declares eight pinned worker
loci, each annotated with a different `core = N`, all
subscribing to the same input subject and publishing on the
same output subject. The bus does the routing; the OS does the
parallelism; the substrate does the bookkeeping.

> **A note on workload partitioning.** The example above has
> every worker process every input — both workers see all
> three samples and produce six results. That is the bus's
> fan-out semantics. For partitioned-work patterns where each
> input goes to *one* worker, the application layer needs to
> route — typically by having the coordinator select which
> subject to publish on (`"work.0"`, `"work.1"`) and each
> worker subscribe to its own subject. Substrate-level
> partitioned-fan-out is a v1.x roadmap item.

## Why two modes, not three

The bimodality is load-bearing. A "third mode" was considered
and rejected during the language's design:

> A *greedy* class: cooperative-but-don't-yield-between-cells
> either. Reject because cooperative already guarantees
> handler-atomicity, so the only thing greedy could add was
> "don't yield BETWEEN cells either" — which is leaving the
> cooperative scheduler entirely. The place you go when you
> leave is your own thread. That's pinned.

The same argument applies to a hypothetical *shared-arena*
pinned class. Each arena stays single-threaded territory; the
substrate cost lives at the layer boundary (the mailbox);
trying to share an arena across threads would either force
locking inside arenas or break the per-locus arena invariant.
Bimodality preserves the invariant. Cooperative on one side,
pinned on the other; no third mode.

## What this chapter does not cover

- **Pinned threads with explicit thread-pool sizing.** v0
  spawns one OS thread per pinned locus. A future
  `: schedule pinned(pool = N)` annotation could bind a group
  of loci to a thread pool of fixed size — useful when the
  program has more pinned loci than cores. Roadmap.
- **Thread priorities.** v0 inherits the OS default priority.
  Real-time priority annotations (`pinned(rt = priority)`)
  are roadmap.
- **NUMA awareness.** v0's CPU affinity is logical-core only.
  NUMA-aware allocation (binding a pinned locus's arena to
  the same NUMA node as its core) is roadmap.

The next chapter, **[Cross-process](./09-cross-process.md)**,
goes one level beyond: when even multiple threads in one
process is the wrong shape — when a workload needs separate
OS processes, possibly on separate machines — Aperio's bus
extends transparently.
