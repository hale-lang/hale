# Lifecycle & time

> **α** — How does a locus come into being, run, and dissolve?
> And what does "concurrent" mean here?

A locus isn't a static record. It moves through five named
states from construction to teardown, and the runtime
guarantees the ordering. Concurrency in Aperio is not
`async`/`await`; it's the *cooperative scheduling* of many
loci through their lifecycles, coordinated by bus events and
yield points.

This chapter covers the five lifecycle methods, the two
schedule classes, the cooperative yield model, drain cascade,
the rules for when an unbound locus dissolves vs. stays
alive, and why there's no `async` keyword.

## The five lifecycle methods

Every locus type has five available lifecycle methods. None
are required; the compiler supplies defaults for any you omit.

```aperio
locus GameSession {
    birth()           { /* once at construction */ }
    accept(c: Player) { /* per child arrival */ }
    run()             { /* steady-state work */ }
    drain()           { /* prepare to dissolve */ }
    dissolve()        { /* teardown */ }
}
```

**`birth()`** runs once, synchronously, at the very start of
the locus's life. By the time it returns, the locus's region
is allocated, its `params` are initialized, and its bus
subscriptions are wired. `birth` is where you acquire
resources: open files, listen on sockets, allocate large
buffers. State you mutate in `birth` is visible to every
subsequent method via `self`.

If `birth()` panics or routes an error upward, the region is
freed, no `dissolve` runs, and the parent's `on_failure`
receives the structural-failure event.

**`accept(c)`** runs **before** child `c`'s region is
allocated. It's the parent's gatekeeper: the parent can read
`c`'s declared params and contract surface, and either accept
(return normally) or reject (route through `on_failure`). If
accept rejects, the child instantiation expression fails and
no resources are committed.

**`run()`** is the steady-state body. It may loop, wait for
bus events, time-sleep, publish, do work. It's a cooperative
function: it runs to completion *or* yields at a cooperative
yield point and lets the scheduler hand control to another
locus. If `run` returns naturally, the locus proceeds to
drain.

If `run` is omitted, the locus has no steady-state loop; it
still receives bus events (its handlers run whenever messages
arrive) and stays alive until the enclosing scope dissolves
it.

**`drain()`** runs when the locus is asked to shut down. It
*cascades depth-first*: every child of this locus drains first
(synchronously), then this locus drains. During drain, new
child accepts are refused, new bus messages aren't accepted,
but in-flight handler invocations complete. The default
`drain` is a no-op — the runtime's draining-state guard is
already enough for many loci.

**`dissolve()`** runs after drain completes. User-supplied
cleanup runs here. After `dissolve` returns, the locus's
region is freed wholesale. The default `dissolve` is also a
no-op (the region cleanup happens regardless).

Together these five form a *state machine* the runtime
enforces. You can't accept after drain has begun. You can't
run before birth completed. The compiler and runtime jointly
guarantee the ordering — you don't have to defensively code
against impossible transitions.

## Default lifecycle methods

A locus that omits a lifecycle method gets a compiler-supplied
default:

| Method | Default behavior |
|---|---|
| `birth()` | no-op |
| `accept(c)` | register `c` in `self.children`; no policy |
| `run()` | empty steady-state; locus waits for events or signals |
| `drain()` | refuse new work, wait for in-flight |
| `dissolve()` | free the region wholesale |
| `on_failure(c, err)` | `bubble(err)` |

A locus with only `params` and `birth` is fully valid — that
was the `Greeter` from
[Your first locus](../getting-started/first-locus.md). The
compiler fills in the rest.

## Schedule classes

The lifecycle methods of multiple loci execute under a
**scheduler**. Aperio commits to a bimodal scheduling model:

```aperio
locus Matchmaker : schedule cooperative { ... }   // default
locus DataIngest : schedule pinned      { ... }
locus Bursty     : schedule pinned(core = 3) { ... }
```

Two classes, with no third option:

- **`cooperative`** (default) — Shares a scheduler thread
  with other cooperative loci. Yields at substrate-cell
  boundaries: between handler invocations, between lifecycle
  transitions, on bus dispatch, on `time::sleep`, on explicit
  `yield`. Handler bodies are *atomic* — no preemption inside
  one.
- **`pinned`** — Owns its own OS thread. No yielding to
  siblings inside the same scheduler; the locus runs as long
  as it has work and the OS thread runs it. Cross-thread bus
  traffic crosses through a per-locus lock-protected mailbox.
  Optionally CPU-affinitized via `pinned(core = N)`.

There is **no greedy or third class**. A locus that "shares
the scheduler thread but doesn't yield between handlers" would
be a structural compromise — cooperative already guarantees
handler-atomicity, so the only additional thing it could do
is *refuse to yield between cells*, which means "I don't
share." That's what `pinned` is.

The rule of thumb: cooperative is the default for almost
everything; pinned is for latency-critical work that genuinely
shouldn't share the scheduler thread (real-time data ingest,
high-frequency tick handling).

A worked example of two pinned loci publishing to a
cooperative aggregator — including how the bus crosses
thread boundaries — lives at
[Mix pinned + cooperative threads](../how-tos/threading.md).

## Cooperative yield points

Inside a cooperative locus, the substrate yields between
"substrate cells" — atomic units of locus work. The yield
points:

1. **Handler exit.** After a bus handler returns, the
   scheduler may pick up another locus.
2. **Lifecycle transitions.** Between `birth` → `run` → `drain`
   → `dissolve`.
3. **Bus dispatch.** A `<-` send enqueues for the subscriber;
   the subscriber's handler runs at its scheduler's next
   yield point.
4. **`time::sleep(d)`.** Yields for at least `d` real time.
   After the underlying `clock_nanosleep` returns, the
   substrate drains the cooperative bus queue inline — so a
   cooperative subscriber whose `run()` loops with
   `time::sleep(...)` delivers cells posted by other threads
   (unix-bound reader threads, pinned publishers) mid-loop
   without an explicit `yield;`. The drain is idempotent;
   existing `sleep; yield;` code stays correct.
5. **Explicit `yield;`** — a statement-level construct that
   lets you insert a cooperative yield inside a long internal
   loop. Still useful for loops that don't sleep but want to
   surface queued cells at a known checkpoint.

Between yield points, the cooperative locus has the scheduler
thread exclusively. No other locus's code runs on that
thread until the current one yields. This makes most data
races at the application layer structurally impossible:
within a single cooperative locus, there is no parallelism to
race against.

## Drain cascade

Drain has one rule and one rule only:

> `drain()` always cascades depth-first.

When `drain()` is called on a locus L:

1. The runtime walks L's children depth-first, calling
   `drain()` on each (which recursively walks their children).
2. After every child has drained, L's own `drain()` body runs.
3. After L's drain completes, L's `dissolve()` runs.

There is no separate `drain_cascade()` syntax — drain is
always cascading. This rule is what makes SIGINT handling
trivial: the signal handler calls `drain()` on the runtime
root locus, the whole tree cascades, every locus shuts down
in dependency order, and the process exits cleanly. From the
user's perspective, "Ctrl-C and the program exits cleanly" is
the default.

In flight during drain:
- New child accepts are refused.
- In-flight bus messages on subscriptions are delivered;
  no new messages accepted.
- Closure tests at `tick` epoch may fire (if not already).
- Closure tests at the `dissolve` epoch will fire as part of
  the dissolve sequence.

## Dissolve timing rules

When does a locus *actually* dissolve? Three shapes:

```aperio
fn main() {
    Greeter { name: "Aperio" };           // statement position
    let s = Stream { fd: connect(...) };  // let-bound
    Counter { };                          // anonymous w/ subscriptions
}
```

1. **Statement-position literal** (no binding, no bus
   subscriptions, no `run()` body to outlive birth): runs
   birth → drain → dissolve immediately at the statement
   boundary. Fire-and-forget. The handle is discarded.
2. **Let-bound literal** (`let s = ...`): birth + run + drain
   fire at construction, but **dissolve defers to the
   enclosing function's scope-exit flush**. The binding stays
   valid for method calls between construction and dissolve.
3. **Long-lived** (the locus has `bus subscribe` declarations,
   or a `run()` body that hasn't returned): always defers to
   scope exit, regardless of binding. The locus must stay
   alive to receive published events between birth and the
   enclosing scope's exit.

The user-visible rule: **`let`-binding keeps the locus alive
for the scope.** Statement-position is fire-and-forget unless
the locus has post-birth work (subscriptions, run-loop). Two
quick examples:

```aperio
fn main() {
    let c = Counter { };       // Counter alive for main's scope
    Echoer { };                // Echoer alive for main's scope
                               //   (has bus subscriptions → long-lived)
    Pulse { iters: 4 };        // Pulse: run() to completion, dissolve immediately
    println(c.sum);            // c still valid here
}                              // Counter + Echoer drain + dissolve at scope exit
```

Multiple deferred dissolves in the same scope fire in
**reverse instantiation order** at scope exit (LIFO),
matching the depth-first cascade rule.

## Why no async / await

Other languages put concurrency in `async`/`await`: a function
declares it might block; a caller `awaits` it; the runtime
suspends and resumes via state-machine compilation.

Aperio doesn't have `async`/`await` (the keywords are
reserved). Why?

Because the substrate already gives you what `async` is for —
without the function-coloring problem.

- **Cooperative yield points** play the role of `await`. A
  bus handler running on the cooperative scheduler is exactly
  an async-style task. It runs, yields between handlers, and
  is resumed when its next message arrives. The scheduler
  handles the dispatch.
- **Lifecycle methods** play the role of structured
  concurrency. `birth` / `run` / `drain` / `dissolve` are the
  spawning and joining of a "task" — but with typed state and
  a parent supervisor.
- **The bus** plays the role of channels. Typed pub/sub
  between loci, with the runtime handling dispatch ordering.
- **Pinned scheduling** plays the role of "spawn on a thread
  pool." A pinned locus owns an OS thread; bus traffic
  crosses thread boundaries through a mailbox.

The function-coloring problem in async languages — the fact
that calling an async function from a sync function requires
special machinery — disappears because there are no async
*functions*. There are *loci*, which are structurally aware
of when they should yield. The yield is at the locus
boundary, not inside a sync-vs-async function call site.

The cost is that you can't write code that *looks* like
synchronous-with-occasional-blocking. You write *loci that
communicate*, which is a different shape. For most systems,
the locus shape is more honest — your code already had loci
in it implicitly; Aperio just makes them syntactic.

## Next

The next chapter, [Perspective & observation](./perspective.md),
covers Aperio's mechanism for *serializable observation* — how
a locus exposes a versioned, schema-shared view of itself that
can travel across process boundaries.
