# Lifecycle & time

> How does a locus come into being, run, and dissolve?
> And what does "concurrent" mean here?

A locus isn't a static record. It moves through five named
states from construction to teardown, and the runtime
guarantees the ordering. Concurrency in Hale is not
`async`/`await`; it's the *cooperative scheduling* of many
loci through their lifecycles, coordinated by bus events and
yield points.

This chapter covers the five lifecycle methods, the two
schedule classes, the cooperative yield model, drain cascade,
the rules for when an unbound locus dissolves vs. stays
alive, and why there's no `async` keyword.

## The lifecycle methods

Every locus type has six available lifecycle methods. None
are required; the compiler supplies defaults for any you omit.

```hale
locus GameSession {
    birth()           { /* once at construction */ }
    accept(c: Player) { /* per child arrival */ }
    release(c: Player) { /* per child completion */ }
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

**`release(c)`** is the death-side bookend, symmetric to
`accept`. It runs when an accept'd child `c` *completes* —
after `c` has drained, before it dissolves — so the parent gets
one consistent place to observe each child finishing and to read
its final state. Declaring `release(c: T)` also does something
structural: it marks `T` a **flow** (see [Ending early](#ending-early-terminate-and-flow-children)
below), so a `T` child is reclaimed the moment its own `run()`
completes rather than living until the parent dissolves. It's
policy only — it does not free anything.

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
| `release(c)` | not supplied — absence means `c`'s type is a *resident*, not a flow (see below) |
| `run()` | empty steady-state; locus waits for events or signals |
| `drain()` | refuse new work, wait for in-flight |
| `dissolve()` | free the region wholesale |
| `on_failure(c, err)` | `bubble(err)` |

A locus with only `params` and `birth` is fully valid — that
was the `Greeter` from
[Your first locus](../getting-started/first-locus.md). The
compiler fills in the rest.

## Placement (F.31)

The lifecycle methods of multiple loci execute under a
**scheduler**. Hale commits to a bimodal model — cooperative
pools (shared OS threads) and pinned threads (one OS thread
per locus) — with placement declared at `main locus` rather
than on the locus itself:

```hale
main locus App {
    params {
        matchmaker:  Matchmaker   = Matchmaker { };
        ingest:      DataIngest   = DataIngest { };
        bursty:      Bursty       = Bursty { };
    }
    placement {
        matchmaker:  cooperative(pool = main);
        ingest:      pinned;
        bursty:      pinned(core = 3);
    }
}
```

Two classes, with no third option:

- **`cooperative(pool = X)`** — Shares pool `X`'s OS thread
  with other cooperative loci placed on the same pool. Yields
  at substrate-cell boundaries: between handler invocations,
  between lifecycle transitions, on bus dispatch, on
  `time::sleep`, on explicit `yield`. Handler bodies are
  *atomic* — no preemption inside one. Default for any
  main-locus `params` field not mentioned in `placement { }`,
  with pool `main` (the program's main OS thread).
- **`pinned`** — Owns its own OS thread. No yielding to
  siblings; the locus runs as long as it has work and the OS
  thread runs it. Cross-thread bus traffic crosses through a
  per-locus lock-protected mailbox. Optionally CPU-affinitized
  via `pinned(core = N)`.

There is **no greedy or third class**. A locus that "shares a
pool's thread but doesn't yield between handlers" would be a
structural compromise — cooperative already guarantees handler-
atomicity, so the only additional thing it could do is *refuse
to yield between cells*, which means "I don't share." That's
what `pinned` is. (If you want to isolate a cooperative locus
from siblings without dedicating it a full thread, place it on
its own quiet pool: `cooperative(pool = quiet)`.)

**Placement keys on main-locus `params` field names**, not
locus type names. This lets two siblings of the same locus
type sit on different pools — exactly the parallelism case
(per-venue gateways pinned to distinct cores, etc.).

**Nested loci inherit their parent's pool.** Placement entries
apply only to top-level `main locus` `params` fields. A locus
instantiated inside another locus's body shares its parent's
pool. To run a nested locus on a different pool, hoist it to
a main-locus sibling.

**Cooperative pool inference.** Pool names appear only in
`cooperative(pool = X)` references in the `placement { }`
block. The runtime spawns one OS worker per inferred pool name
beyond `main`. No `threads { }` declaration block at v1 —
pools are named purely by use site.

**Green-I/O via `where async_io` (F.35).** A cooperative pool
that handles many concurrent I/O-bound connections can opt into
a green-I/O drain loop by tagging the placement entry:

```hale
placement {
    worker: cooperative(pool = ws_workers) where async_io;
}
```

The pool's worker integrates an epoll instance, and blocking
I/O syscalls inside locus methods on this pool (`recv_bytes`,
`accept_one`, `send_bytes`, ...) park-and-resume instead of
holding the OS thread. User code stays synchronous-shaped —
`stream.recv_bytes(N)` is the same line of source either way.
Detailed treatment in [How to: threading § Green-I/O cooperative
pools](../how-tos/threading.md#green-io-cooperative-pools-where-async_io).

The rule of thumb: cooperative-on-main is the default for
almost everything; pinned is for latency-critical work that
genuinely shouldn't share a pool thread (real-time data
ingest, high-frequency tick handling); separate cooperative
pools are for partitioning long-running siblings that would
otherwise serialize on the same thread.

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
   The sleep is sliced into intervals of at most 100ms, and the
   substrate drains the cooperative bus queue after each slice —
   so a cooperative subscriber whose `run()` loops with
   `time::sleep(...)` delivers cells posted by other threads
   (unix-bound reader threads, pinned publishers) mid-loop
   without an explicit `yield;`. The slicing matters for the
   keep-alive idiom: a `main`-thread `while true { sleep(60s); }`
   still services the bus ~10×/s instead of stalling every
   `main`-pool handler for 60s at a stretch. Total wall-clock is
   preserved (slices sum to `d`); sleeps ≤100ms are one slice, so
   the common case is unchanged, and the drain is idempotent —
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

```hale
fn main() {
    Greeter { name: "Hale" };           // statement position
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

```hale
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

## Ending early: `terminate` and flow children

The dissolve rules above are about loci bound in a *function*
scope. The other case is an **`accept`'d child of a long-lived
parent** — a server that accepts one child per connection. By
default such a child's region is reclaimed only when the
*parent* dissolves; for a daemon whose parent never dissolves,
that means the child's region is never reclaimed, and connections
accumulate (see
[Keeping memory bounded](../how-tos/keeping-memory-bounded.md#reclaim-per-connection-state-with-flow-children)).
Hale gives a child two in-grain ways to end *with its own flow*,
neither of which is a manual free — both invoke the normal
`drain → dissolve → region-free` teardown:

**`terminate;`** is the locus analogue of `return`. Inside one
of a locus's own methods, it ends *that locus's* lifecycle: it
exits the method immediately (code after it doesn't run, just
like `return`), and when the method's `run()` completes the
runtime tears the locus down. Use it when a locus decides it's
finished — a connection that reads a close frame, a session that
times out.

```hale
locus Worker {
    run() {
        let job = self.next();
        if job.poisoned { terminate; }   // end myself now
        self.process(job);
    }
}
```

**Flow children** make this automatic. A child whose parent
declares `release(c: Child)` is a *flow*: its `run()` **is** its
lifetime, and when `run()` returns the runtime reclaims it — no
explicit `terminate;` needed. A connection child whose `run()`
is a recv loop that returns on EOF reclaims on that plain
`return`. The parent's `release(c)` fires on the way out (after
drain, before dissolve) for a last look at the finished child.

```hale
locus Conn {
    params { conn_fd: Int = -1; }
    run() {
        let stream = std::io::tcp::Stream { conn_fd: self.conn_fd, owns_fd: false };
        loop {
            let chunk = stream.recv(4096);
            if len(chunk) == 0 { return; }   // client closed → Conn reclaimed
            // ... handle chunk
        }
    }
}

locus Server {
    accept(c: Conn)  { }
    release(c: Conn) { }   // ← marks Conn a flow: reclaim on run() completion
}
```

A child whose type **no** parent `release`s is a **resident**:
its `run()` returning means "ready", and it lives — receiving bus
events — until the parent dissolves. That's the right shape for a
fixed cohort of long-lived workers a parent spawns at boot. The
distinction is always explicit (the presence of `release`), never
inferred: the same "`run()` returned" event means "reclaim me" for
a flow and "I'm ready" for a resident.

(At program shutdown, a parent dissolving still reclaims any
children it hasn't already — the drain cascade is the backstop;
flow reclamation is the *early* path that keeps a daemon bounded
while it runs.)

## Why no async / await

Other languages put concurrency in `async`/`await`: a function
declares it might block; a caller `awaits` it; the runtime
suspends and resumes via state-machine compilation.

Hale doesn't have `async`/`await` (the keywords are
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
in it implicitly; Hale just makes them syntactic.

## Next

The next chapter, [Perspective & observation](./perspective.md),
covers Hale's mechanism for *serializable observation* — how
a locus exposes a versioned, schema-shared view of itself that
can travel across process boundaries.
