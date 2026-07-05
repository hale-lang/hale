# Concurrency & placement

> **Coming from Go?** Concurrency isn't `go f()` scattered through
> the code. Loci run concurrently by default; *where* each one
> runs — a shared cooperative pool (like a scheduler's worker) or
> its own dedicated OS thread — is declared in one place, the
> `placement { }` block on `main`. It's a deployment decision, not
> something baked into the locus. And there's no `async`/`await`:
> the lifecycle and the bus already give you what coloring
> functions would.

## Two ways a locus can run

Hale's concurrency is deliberately **bimodal** — two choices, no
third:

- **Cooperative** — the locus shares an OS thread with other
  cooperative loci on the same *pool*. It yields between units of
  work (after a handler, on a bus dispatch, on `time::sleep`, on
  an explicit `yield`). Handler bodies run to completion without
  interruption, so within one cooperative locus there's no
  data race to worry about. This is the default.
- **Pinned** — the locus owns its own OS thread and doesn't yield
  to neighbors. For latency-critical or CPU-bound work that
  shouldn't share.

## Long sleeps don't freeze the pool

A cooperative pool runs one locus at a time, so a locus that sits
in a long `time::sleep` could, in principle, starve every other
locus sharing its pool — a 30-second keep-alive timer on the
`main` pool would block bus handlers for 30 seconds. It doesn't.
`std::time::sleep` slices any sleep into short intervals (≤100ms)
and drains the pool's pending bus work between slices, so
neighbors keep getting dispatched while one locus naps:

```hale
run() {
    while true {
        self.send_heartbeat();
        std::time::sleep(30s);   // sliced — co-resident handlers
                                 // still fire every ≤100ms
    }
}
```

The sleeping locus still wakes after the full duration; it just
doesn't hold the thread hostage in the meantime. You write
`sleep(30s)` and the slicing is invisible — there's nothing to
opt into. (A `pinned` locus owns its thread, so its sleeps affect
no one and aren't sliced.)

## Placement lives on `main`

You declare placement once, against the top-level loci, in
`main`:

```hale
main locus App {
    params {
        gateway: Gateway       = Gateway { };
        metrics: MetricsServer = MetricsServer { port: 9100 };
        ui:      Renderer      = Renderer { };
    }
    placement {
        gateway: pinned(core = 1);          // own thread, pinned to core 1
        metrics: cooperative(pool = io);    // shares the "io" pool
        ui:      cooperative(pool = render);
        // anything unlisted defaults to cooperative(pool = main)
    }
}
```

- `cooperative(pool = X)` puts the locus on pool `X`'s thread.
  The runtime spawns one OS worker per pool name it sees.
- `pinned` / `pinned(core = N)` gives the locus its own thread,
  optionally pinned to a CPU core.
- `pinned(cores = 4..8)` (or `4..=7`, or `{4, 5, 6, 7}`) pins
  the thread to a core *set* instead of one core: the OS
  schedules it freely within the set, so a range carves out an
  isolation domain ("this locus lives on these cores, away from
  everything else") without hand-picking a single CPU. Ranges
  follow the usual rules — `..` excludes the upper bound, `..=`
  includes it.
- `pinned(node = 0)` / `pinned(l3 = fast)` target a NUMA node or
  cache domain *by name* instead of raw core numbers — see the
  `topology { }` block below.
- Unmentioned top-level loci default to `cooperative(pool =
  main)` — the program's main thread.

Core affinity (`core =`, `cores =`, and the `node =` / `l3 =`
forms below) is a Linux optimization and best-effort: indices
that don't exist on the box are skipped, and on other platforms
(macOS) the thread simply runs unpinned. Your program behaves
identically either way — affinity only affects *where* the
scheduler may run the thread.

## Describing the machine: `topology { }`

Raw core numbers work, but on a big box you'd rather say "put
this on the fast cache domain" than memorize which cores share
an L3. A `topology { }` block on `main` describes the host's
core partition once, and placement entries target it by name:

```hale
main locus App {
    topology {
        reserve cores 0..2;              // hands-off for the OS / main
        node 0 {
            l3 fast { cores 4..8; }      // a CCD / shared-L3 group
            l3 slow { cores 8..12; }
        }
        node 1 {
            l3 heavy { cores 12..16; }
        }
    }
    params {
        matcher: Matcher = Matcher { };
        region:  Region  = Region  { };
    }
    placement {
        matcher: pinned(l3 = fast);   // affinity = the `fast` domain, {4..8}
        region:  pinned(node = 0);    // affinity = node 0's cores, {4..12}
    }
}
```

- `pinned(node = N)` masks the thread to node `N`'s cores — the
  union of the node's L3 domains.
- `pinned(l3 = name)` masks it to that one cache domain, so
  cooperating loci sharing an L3 keep their cross-locus bus
  traffic hot in that cache.
- `reserve cores` holds cores back for the OS / main; a domain
  may not claim a reserved core.

The block is **declare-only** and checked at compile time: node
ids must be unique, L3 names must be unique (so `pinned(l3 =
name)` is unambiguous), a core belongs to at most one domain,
and every `pinned(node/l3)` must name a domain you declared.
L3-domain names are ordinary identifiers, so a reserved word
(like `bulk`) can't be a domain name — pick a plain name.

**Thread *and* memory co-location.** `pinned(node = N)` binds
more than the thread: the locus's arena — and its per-call
method scratch — is allocated on that NUMA node's memory (via
`mbind`), so its working set lives next to the thread that uses
it. That's the point of NUMA targeting: cross-node memory access
is what kills big-box performance, and a node-pinned locus
avoids it on both axes. `pinned(l3 = fast)` binds the arena to
the node containing that cache domain. Like affinity, memory
binding is a Linux optimization and best-effort — it falls back
to normal allocation where the node can't be honored, and it
costs nothing (no extra dependency, the ordinary allocation
path) for loci that don't ask for a node.

Placement keys on the *field name*, not the locus type, so two
instances of the same locus type can live on different threads —
the parallelism case (one gateway per core, say).

Why on `main` and not on the locus? Because where something runs
is a property of the *deployment*, not the code. The same
`Gateway` locus is pinned in production and cooperative in a
test, with no edit to `Gateway` itself. Library authors say what
a locus *is*; the binary author says *where it runs*.

## Nested loci inherit their pool

Placement entries apply only to top-level `main` loci. A locus
instantiated inside another locus's body runs on its parent's
pool. To put a component on its own pool, hoist it to a top-level
sibling in `main` and give it a placement entry. (This is the
canonical fix for "my long-running child starved its parent" —
make it a sibling, not a nested child.)

This inheritance is also how you **co-locate work on a `pinned`
thread**. There's no `pinned(pool = X)` for sharing a pinned
thread — `pinned` owns its thread exclusively. So when a pinned
locus needs helpers on its thread (counters, a metrics registry, a
signal store — anything it calls directly), you *nest* them: make
them `params` of the pinned locus, and they inherit its thread.
Param defaults make this ergonomic — a default can itself
instantiate the helper:

```hale
locus Gateway {              // placed pinned in main
    params {
        reg:   Registry = Registry { };
        ticks: metrics::Counter = metrics::counter(self.reg, "ticks");
    }
    // run() calls self.ticks.inc() etc. — all on the pinned thread
}
```

Hoisting them to siblings instead would put them on a *different*
thread, and the gateway calling them directly would then be a
cross-pool method call — which the compiler rejects (see below).
Nesting is the supported pattern for "many loci, one pinned
thread."

## The bus crosses threads for you

When a cooperative locus on one pool publishes to a subscriber on
another pool — or to a pinned locus on its own thread — the
runtime handles the hand-off: it copies the payload across the
thread boundary and wakes the destination. The sender never
blocks. From your code's point of view, `Topic <- value;` is the
same line whether the subscriber is on the same thread or a
different one. The substrate adapts; the source doesn't.

## High-concurrency I/O: `where async_io`

A single pinned thread handles one blocking connection at a time.
To serve *many* concurrent connections on one thread without a
thread-per-connection explosion, tag a cooperative pool with
`where async_io`:

```hale
placement {
    workers: cooperative(pool = ws) where async_io;
}
```

The pool's worker runs an event loop (epoll under the hood), and
blocking I/O calls inside loci on that pool — `recv`, `accept`,
`send` — *park and resume* instead of holding the thread. Your
locus code stays synchronous-shaped: `stream.recv(4096)` is the
same call either way; the substrate picks the parking lowering at
the syscall boundary. This is how you get async-style throughput
without async-style function coloring.

## The compiler checks your placement

Two placement mistakes are caught for you, because both the
placement and the locus's shape are known at compile time:

- **A subscriber that blocks its own delivery is an error.** A
  cooperative locus on a non-`main` pool *receives bus cells fine*
  as long as its pool thread is free to run the dispatch — an
  event-driven subscriber (handlers plus a `sleep` loop, or `where
  async_io`) works. But if such a subscriber's `run()` makes a
  **blocking** call, it monopolizes the pool thread, the dispatch
  never runs, and its handlers never fire. *That* combination —
  non-`main` cooperative subscriber **with a blocking `run()`** —
  is the error; the compiler points you at `pinned` (own thread +
  mailbox) or keeping `run()` non-blocking. (Placement alone is
  fine; it's the blocking call that kills delivery.)
- **A blocking call on a cooperative pool is a warning.** Even when
  the locus *isn't* a subscriber, a blocking `run()` (a blocking
  `recv`/`accept`, a subprocess `run`) on a pool that isn't `where
  async_io` holds the pool's thread and stalls everything else
  scheduled there. The compiler warns and suggests `pinned` (own
  thread) or `where async_io` (parks). For blocking I/O gateways,
  `pinned` is the prescribed shape. This warning follows the call
  graph: a `run()` that blocks indirectly — through a helper fn or a
  `self.method` it calls — is flagged too, naming the offending call.
  (The dead-receiver *error* above stays direct-call-only, so it
  never widens onto an indirect path.)
- **An orphan bus topic is a warning.** In a complete program (one
  with a `main` locus), a topic or subject wired to only one end —
  published with nobody subscribed, or subscribed with nobody
  publishing — is flagged, as is a declared topic used by neither.
  It's suppressed when the other end is plausibly external: a
  transport `binding`, a wildcard (`log.**`) covering the subject, a
  cross-seed (`alias::Topic`) reference, or the same locus being both
  ends. Library code (no `main`) isn't checked — its peers live
  downstream.
- **A bus cycle is flagged.** If a handler for one topic publishes
  another in a loop (`a → b → a`), the cell can re-trigger its own
  publish. A cycle *across* loci spins the cooperative queue — a
  warning. A cycle *within* one locus is worse: intra-locus publishes
  are direct synchronous calls, so the loop recurses on the thread
  until the stack overflows — an error. (Only an *unconditional*
  self-republish errors; one guarded by an `if` is a terminating
  state machine and is left alone.)
- **An unthrottled publish loop is a warning.** A `while true` loop
  that publishes with no `yield`, `time::sleep`/`tick`, input-pacing
  `recv`, or `break`/`return` floods the bus — the producer has no
  backpressure, so cells pile up without bound. Pace the loop, drive
  it from an input, or `yield` to let the subscriber drain. (Bounded
  loops are never flagged; any flow-control point clears it.)
- **A subject payload type-mismatch is an error.** If two sites
  publish/subscribe the same literal subject string with different
  `of type` payloads, a subscriber would decode the wrong type at
  runtime — rejected. (Declared `topic`s are already unified by their
  declaration, so this only affects ad-hoc literal subjects.)

It also enforces the **single-threaded-method invariant**: a locus's
methods may only be called on the thread that owns its pool, so a
*direct* method call across pools (`self.other.foo()` where `other`
is placed on a different pool) is a compile error — it would run
`other`'s method on the wrong thread.

One escape is deliberately **not** traced: a call made through a
*handler function pointer* rather than a direct method reference —
the canonical case being a `std::http::Server` handler that reads a
locus living on another pool. The static call-graph walk can't see
through the pointer, so it's allowed. That's load-bearing (it's how
a `/metrics` endpoint on the `io` pool reads a registry nested on a
pinned gateway), but it's on *you* to keep that access safe —
typically a read of stable, append-only state, not a mutation that
would race the owning thread.

Next: how loci nest and own each other — [Parents &
children](./parents-children.md).
