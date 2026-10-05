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

```hale,fragment
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
opt into. (A `pinned` locus owns its thread, so its sleeps stall
no one else; its slices drain its own mailbox. On a `where
async_io` pool a sleep parks instead of slicing.)

## Placement lives on `main`

You declare placement once, against the top-level loci, in
`main`:

```hale
locus Gateway       { }
locus MetricsServer { params { port: Int = 80; } }
locus Renderer      { }

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

fn main() { App { }; }
```

- `cooperative(pool = X)` puts the locus on pool `X`'s thread.
  The runtime spawns one OS worker per pool name it sees. A pool
  can take the same affinity forms as `pinned` —
  `cooperative(pool = io, cores = 0..=1)` binds the pool's worker
  thread to those cores (declare it on one entry; others naming
  the pool inherit it).
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
locus Matcher { }
locus Region  { }

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

fn main() { App { }; }
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

## Parallelism: `replicas = K`

To run a locus in parallel, you don't get a multi-worker pool —
that would break the single-consumer invariant everything rests
on (one cooperative pool is one thread; the lock-free rings and
bus devirtualization assume it). Instead you fan it into **K
single-threaded instances**:

```hale,fragment
placement {
    // 8 workers, replica i on core 4+i, each its own thread
    workers: pinned(cores = 4..12, replicas = 8);
}
```

Each replica is a full instance on its own core, still
single-threaded — parallelism comes from more units, not from
sharing a thread, so every invariant survives per replica. With
more replicas than cores the assignment wraps round-robin; with
no `cores` the K instances are OS-scheduled. `replicas` composes
with the topology targets — `pinned(node = 0, replicas = 4)` fans
4 workers across node 0's cores, each with its arena on node 0.

The replicas are **workers, not handles**: there's no
`workers[i]` to call. They pull work — by subscribing a bus topic
or running their own loop. A plain subscription registers all K,
so the topic *broadcasts* to every replica. To **shard** instead,
key the topic and subscribe with the replica's own index:

```hale,fragment
type Conn { fd: Int; shard: Int; }
topic NewConn { payload: Conn; keyed_by shard; }

locus Worker {
    bus { subscribe NewConn as on_conn where key == replica; }
    fn on_conn(c: Conn) { /* only shard == my index arrives */ }
}
// placement { workers: pinned(cores = 4..12, replicas = 8); }
// listener publishes: NewConn <- Conn { fd: fd, shard: fd % 8 };
```

`replica` is the instance's 0-based index (a non-replicated
instance is replica 0), so K lives in exactly one place — the
placement entry — and each connection lands on one worker. This
is the webserver fan-out shape: a pinned listener accepting,
K workers each owning their shard. `replicas` is pinned-only;
`cooperative(..., replicas = K)` is rejected (K loci on one pool
would share a thread, which isn't parallel).

Placement keys on the *field name*, not the locus type, so two
instances of the same locus type can live on different threads —
the parallelism case (one gateway per core, say).

Why on `main` and not on the locus? Because where something runs
is a property of the *deployment*, not the code. The same
`Gateway` locus is pinned in production and cooperative in a
test, with no edit to `Gateway` itself. Library authors say what
a locus *is*; the binary author says *where it runs*.

## Counting the threads

The compiler resolves placement per instance, into one table that
both the checks and the build read: each instance of `main`'s tower
gets a thread domain, nested instances share their owner's, and the
binary spawns exactly what the table says. So the thread count is
something you can read before you run:

- a `pinned` field is one thread, and `replicas = K` is K;
- each construction of `main` that can be live at once spawns its own
  pinned threads, so a `main` built at two sites, or by a function
  called twice, counts its pinned fields twice;
- an adapter in `bindings { }` is one thread, built once, however
  often `main` is constructed;
- a cooperative pool is one worker, however many loci run on it; the
  main thread is the program's own, not a pool.

`hale check --dump-resource-budget` prints the count, and
`--check-resource-budget` gates it in CI. A `main` built in a loop
has no static bound, so its count is *uncertain*, with the reason,
and a thread ceiling refuses it. A binding's reader thread and the
serve thread a stdlib transport starts are not placement, and the
dump names them as not counted.

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
        reg:   std::metrics::Registry = std::metrics::Registry { namespace: "gw" };
        ticks: std::metrics::Counter  = std::metrics::counter(
                   self.reg, "ticks", std::metrics::labels_empty());
    }
    // run() calls self.ticks.inc() etc. — all on the pinned thread
}
```

Hoisting them to siblings instead would put them on a *different*
thread, and the gateway calling them directly would then be a
cross-pool method call — which the compiler rejects (see below).
Nesting is the supported pattern for "many loci, one pinned
thread."

A nested locus's bus handlers run there too. If a helper nested
under the pinned gateway (or under a locus on `pool = io`)
subscribes to a topic, its handler runs on the gateway's thread (or
on `io`'s worker), wherever the publisher is: the runtime routes
the helper's subscriptions to its anchor's thread.

Under a pinned locus this holds from the start. The gateway's
params are built on the gateway's own thread, so a helper's
`birth()` and, for a helper with a `run()`, its `run()` happen
there too, before the gateway's own `birth()`. A helper whose
`run()` waits for a message through the gateway's mailbox gets it:
a `std::time::sleep` on that thread drains the mailbox. The
literal that builds the gateway returns once its params are built.
While it waits, the thread running the literal keeps handling its
own messages, as a `std::time::sleep` there would, so a helper
whose `run()` asks a locus on that thread for something and waits
for the answer gets it.
An override written in that literal (`Gateway { started:
std::time::monotonic_ns() }`) is your code, not the gateway's, and
is evaluated where the literal is; a helper the override builds
(`Gateway { reg: std::metrics::Registry { namespace: "edge" } }`)
is still the gateway's, built on its thread.

A locus placed on a pool (`cooperative(pool = io)`) gets the same
start on `io`'s worker: its params, and every helper nested under
it with that helper's `birth()` and inline `run()`, are built on
the worker as the locus's first job there, and the literal returns
once they are. A helper that waits in that `run()` for a message to
itself gets it: a `std::time::sleep` there lets the worker handle
`io`'s queued messages. The pool locus's own `birth()` runs on the
worker too, after its helpers are built and before any message
reaches it, while the literal waits for it; its `run()` is then
posted to the worker, as before. So the locus's `birth()` and its
handlers run on one thread and never at the same time. Two locus fields on one pool are built in turn,
one after the other. If the worker is waiting for this constructor
to settle a held failure, it can perform the initialization during
that wait, even when its message queue is full. This breaks the
startup dependency and runs the initialization once on the worker.
Otherwise the build waits behind whatever the worker is
already running, so a locus placed after a sibling whose `run()`
never returns on the same pool (without `where async_io`) is never
built, and the program stops there.

## The bus crosses threads for you

An adapter named in `bindings { }` runs on its own pinned thread.
The same restrictions as an explicit `pinned` placement apply:
it cannot declare `accept()` or a closure with a `birth` or
`dissolve` epoch (including an omitted epoch). The checker reports
these at the binding, as it reports them at a `pinned` placement
entry, with a note at each place that makes it so: the binding (or
the entry), the locus's declaration, and the `accept()` or the
closure's assertion. Inline closures are supported on that thread.

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

```hale,fragment
placement {
    workers: cooperative(pool = ws) where async_io;
}
```

The pool's worker runs an event loop (epoll under the hood), and
blocking I/O calls inside loci on that pool — `recv`, `accept`,
`send` — *park and resume* instead of holding the thread. Your
locus code stays synchronous-shaped: `stream.recv(4096) or ""` is the
same call either way; the substrate picks the parking lowering at
the syscall boundary. This is how you get async-style throughput
without async-style function coloring.

A bus handler on such a pool keeps its payload across a park: park on
a read, a `sleep`, or a subprocess drain in the middle of a handler,
and the payload parameter still reads what it read at entry — it is
that delivery's own until the handler returns, no matter how many
other deliveries the pool starts in between.

## The compiler checks your placement

These placement mistakes are caught for you, because both the
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
  `recv`/`accept`, a line read from stdin or a file, an `http`
  request, a subprocess `run`) on a pool that isn't `where async_io`
  holds the pool's thread and stalls everything else scheduled there.
  The compiler warns and suggests `pinned` (own thread) or `where
  async_io` (parks). For blocking I/O gateways, `pinned` is the
  prescribed shape. This warning follows the call
  graph: a `run()` that blocks indirectly — through a helper fn or a
  `self.method` it calls — is flagged too, naming the offending call.
  (The dead-receiver *error* above stays direct-call-only, so it
  never widens onto an indirect path.)

  "Blocking" here means exactly what the effect system means by it:
  the stdlib calls classified `block`, the same set `@no_block` and
  the `.hale.effects` manifest (`hale check --dump-effects-manifest`)
  read. There is one
  deliberate exception — `std::time::sleep`, which the compiler
  chunks into ≤100 ms slices and drains the pool's bus queue between
  them. A sleeping locus keeps its pool serviced, which is why
  "handlers plus a `sleep` loop" is the event-driven shape both of
  these diagnostics point you at.
- **An orphan bus topic is a warning.** In a complete program (one
  with an entry: a `main` locus of its own, at the top level — a
  library's `main locus` reached through an `import` is not this
  program's entry), a topic or subject wired to only one end —
  published with nobody subscribed, or subscribed with nobody
  publishing — is flagged, as is a declared topic used by neither.
  It's suppressed when the other end is plausibly external: a
  transport `binding`, a wildcard (`log.**`) covering the subject, a
  cross-seed (`alias::Topic`) reference, or the same locus being both
  ends; and not at all when the entry carries an `api:` binding,
  whose callers are every topic's other end (an imported library's
  `main locus` and its `api:` entry don't count: that binding is
  inert here). Library code (no `main`) isn't checked — its peers live
  downstream. A topic is one subject however you spell it: publish
  it by its name and subscribe its literal `subject:` string, and
  both ends are wired. A subject the compiler can't resolve (a
  qualified path no import names) is never called an orphan — it
  isn't known to be one.
- **A bus cycle is flagged.** If a handler for one topic publishes
  another in a loop (`a → b → a`), the cell can re-trigger its own
  publish. A cycle *across* loci spins the cooperative queue — a
  warning. A cycle *within* one locus is worse: intra-locus publishes
  are direct synchronous calls, so the loop recurses on the thread
  until the stack overflows — an error. (Only an *unconditional*
  self-republish errors; one guarded by an `if` is a terminating
  state machine and is left alone.) "Within one locus" means one
  declaration: a program that declares one locus name twice is
  already an error, and the cycle check still reads the two as two
  loci, so a loop between them is reported as the cross-locus
  warning, not the error.
- **An unthrottled publish loop is a warning.** A `while true` loop
  that publishes with no `yield`, `time::sleep`/`tick`, an
  input-pacing blocking call, or `break`/`return` floods the bus —
  the producer has no backpressure, so cells pile up without bound.
  Pace the loop, drive it from an input, or `yield` to let the
  subscriber drain. (Bounded loops are never flagged; any
  flow-control point clears it.) "Input-pacing" is the same blocking
  set as the warning above, so a loop driven by a line off stdin is
  paced exactly as one driven by a blocking `recv`.
- **A subject payload type-mismatch is an error.** If two sites
  publish/subscribe the same literal subject string with different
  `of type` payloads, a subscriber would decode the wrong type at
  runtime — rejected. (Declared `topic`s are already unified by their
  declaration, so this only affects ad-hoc literal subjects.)
- **A `pinned` placement in a loop is an error.** A locus whose
  `placement { }` pins a field can't be instantiated inside a `while`
  or `for` body. `pinned` gives that field its own OS thread, and the
  record used to join it is one slot per instantiation *site* — a
  second pass over the site overwrites it, so only the last thread is
  ever joined and the earlier ones are orphaned with their memory
  still live. Placement describes a *static* topology (a core, a NUMA
  node, `replicas = K`): one thread per entry, for the program's
  life. Instantiate it once, outside the loop. The same goes for a
  locus whose `params` default builds `main`: `Holder { }` in a loop,
  where `Holder`'s default is `App { }`, builds an `App` per iteration
  and is refused at `Holder { }`. A `const`'s value, a type's field
  default and a closure's assertion are built again at every use (each
  read of the const, each literal of the type, each evaluation of the
  closure), so the compiler can't show a `main` written in one is built
  once: it is refused there outright, loop or no loop. Build it in a
  locus's `params` or a function body instead. A loop that calls a
  *function* holding the literal is fine — each call joins its own
  thread before it returns:

  ```hale,refused
  locus Worker { }
  main locus App {
      params { w: Worker = Worker { }; }
      placement { w: pinned; }
  }

  fn boot() { App { }; }          // fine: one thread per call, joined

  fn main() {
      let mut i = 0;
      while i < 3 { App { }; i = i + 1; }   // error: three orphans
      return 0;
  }
  ```
- **A placement entry a factory hands you is an error.** A placement
  attaches to the locus *literal* written for the field. A factory
  call hands back a locus that has already been born and run inside
  the factory, so there is nothing left to place — the entry would
  be silently dropped, and the field would run wherever an unplaced
  field runs. Write the literal in the field:

  ```hale,refused
  locus Worker { }
  fn make_worker() -> Worker { return Worker { }; }

  main locus App {
      params {
          a: Worker = make_worker();   // error: nothing carries `a: pinned`
          b: Worker = Worker { };      // fine: this literal is placed
      }
      placement {
          a: pinned;
          b: pinned;
      }
  }
  ```

  The same holds at the instantiation site for a field declared
  without a default: `App { a: Worker { } }` carries the entry,
  `App { a: make_worker() }` does not — wherever that `App { }` is
  written, another locus's `params` default included. A default that
  every `App { … }` overrides with a literal never runs, so it is not
  judged. If the factory did setup work, move it into the locus's own
  `params` defaults or its `birth()`.

It also enforces the **single-threaded-method invariant**: a locus's
methods may only be called on the thread that owns its pool, so a
*direct* method call across pools (`self.other.foo()` where `other`
is placed on a different pool) is a compile error — it would run
`other`'s method on the wrong thread. That holds however `other`'s
type is written: a stdlib locus by its path (`std::log::Logger`), an
alias, or an interface the field's literal implements.

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
