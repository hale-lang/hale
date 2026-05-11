# Runtime

## Synopsis

The Aperio runtime is a small C library, statically linked
into every binary. It provides the per-locus arena allocator,
the bus router, the cooperative scheduler, the cross-thread
mailbox plumbing, and the cross-process transport adapters.
User-facing source code never invokes runtime symbols
directly; the codegen emits calls into the runtime as part of
lowering language constructs.

## Substrate symbols

The runtime exports symbols with a `lotus_` prefix
(intentional — the substrate concept *is* the lotus). The
prefix is preserved across the language rename for substrate
symbols; it is not user-facing.

| Subsystem | Symbol prefix | Role |
|---|---|---|
| Region allocator | `lotus_arena_*` | Per-locus arenas, subregions, wholesale free |
| Bus router | `lotus_bus_*` | Subject registration, dispatch, fanout |
| Cross-process transport | `lotus_transport_*` | listen/connect, send/recv on Unix sockets |
| String runtime | `lotus_str_*` | UTF-8 length, slice, concat |
| Mailbox | `lotus_mailbox_*` | Per-pinned-locus bounded ring buffer |

## Schedule classes

Per **F.x bimodality**, every locus is cooperative or pinned.

### `: schedule cooperative` (default)

A cooperative locus runs on the shared scheduler thread. The
runtime yields between *substrate cells*:

| Cell type |
|---|
| Bus handler invocation |
| Lifecycle transition (`birth` / `run` / `drain` / `dissolve`) |
| `time::sleep` call |
| Explicit `yield;` statement |

Within a cell — within a single handler body or lifecycle
method body — execution is **atomic** with respect to other
cooperative loci. Bus dispatch enqueues; cells run between
publisher cells, not during them.

### `: schedule pinned`

A pinned locus owns its own OS thread. The runtime spawns a
`pthread` at instantiation; the locus's full lifecycle runs on
that thread, in order; the spawning scope joins the thread
before the locus's arena is torn down.

Pinned loci can subscribe and publish on the bus; cross-thread
dispatch routes through per-locus mailboxes (mutex + condvar
+ ring buffer). The substrate cost lives at the layer
boundary — the mailbox lock and the inline payload's two
memcpy moves.

### `: schedule pinned(core = N)`

Pinned with CPU affinity. The runtime calls
`pthread_setaffinity_np` on the spawned thread, binding it to
logical CPU `N`. Best-effort: if the requested core does not
exist or affinity is denied, the runtime falls back to
ordinary OS scheduling — the locus still runs.

## The bus router

The router maintains:

- **Per-subject subscription lists.** For each subject, a list
  of (subscriber locus handle, handler fn pointer, mailbox
  pointer if pinned).
- **A cooperative dispatch queue.** Cells enqueued by `<-`
  pop and run between publisher cells.
- **A remote fanout table.** For subjects with cross-process
  config, the set of `connect`-role transport peers.

### Dispatch semantics

When `<-` runs:

1. The router looks up the subject's subscriber list.
2. For each in-process cooperative subscriber: copy the
   payload from the publisher's arena into the subscriber's
   arena; enqueue a cell for the subscriber.
3. For each in-process pinned subscriber: copy the payload
   inline into the subscriber's mailbox; broadcast the
   condvar.
4. For each remote `connect` peer: serialize via the m70 wire
   format and write to the transport.

Step 1 is direct — no enqueue — for synchronous-dispatch
transports; the queue applies to the cooperative scheduler
specifically.

### Long-lived subscribers

A locus that subscribes registers at the end of its `birth()`
body. The subscription persists until the locus's `drain`
removes it (at the start of `drain`). Between birth and drain,
the locus is reachable via the bus.

## Cross-thread mailbox mechanics

Each pinned subscriber gets its own mailbox: a bounded ring
buffer protected by a mutex + condvar.

When a cooperative publisher's `<-` reaches a pinned
subscriber:

1. Acquire the mailbox mutex.
2. Copy the payload **inline** into a free mailbox slot.
3. Increment the slot count.
4. Broadcast the condvar.
5. Release the mutex.

The pinned thread is blocked in a drain loop, waiting on the
condvar:

1. Wake up from condvar broadcast.
2. Pop a slot from the mailbox.
3. Copy the payload from the inline slot into the
   subscriber's arena.
4. Invoke the handler with the in-arena payload.
5. Loop, or block again on the condvar if the mailbox is
   empty.

### Coordinated shutdown

When a pinned subscriber is asked to drain:

1. The runtime sets a shutdown flag on the mailbox.
2. The drain loop observes "no more cells, shutdown
   requested," breaks the loop.
3. Runs the locus's declared `drain` body (if any) on the
   pinned thread.
4. Runs the locus's declared `dissolve` body (if any).
5. Exits the pthread.
6. The spawning scope joins the pthread before destroying the
   locus's arena.

In-flight cells in the mailbox flush before the loop exits.

## Cross-process transports

v0 ships:

- **`unix://path`** — Unix domain socket, SOCK_SEQPACKET.
  Roles: `listen` (`bind` + `accept` in a background thread),
  `connect` (connect-with-retry).

Roadmap (declared in deployment.yaml, not yet implemented):

- **NATS** — reliable, ordered, control-plane.
- **UDP multicast** — line-rate, lossy-acceptable, broadcast.
- **TCP** — point-to-point reliable.

The transport adapter API is `lotus_transport_*`. Adding a new
transport is a runtime extension, not a language change.

## Built-in functions

| Name | Purpose |
|---|---|
| `print(args...)` | Format and write to stdout, no trailing newline |
| `println(args...)` | Format and write to stdout, newline-terminated |
| `eprint(args...)` | Format and write to stderr (fd 2), no trailing newline |
| `eprintln(args...)` | Format and write to stderr (fd 2), newline-terminated |
| `len(s)` | Byte length of a `String` |
| `to_string(v)` | Render any printable primitive (Int/Float/Bool/Decimal/Duration/Time/enum) as a `String` |
| `time::sleep(d)` | Sleep on `CLOCK_MONOTONIC` with EINTR retry |
| `time::monotonic()` | Read `CLOCK_MONOTONIC` |
| `yield;` | Cooperative cell boundary |
| `check_closures();` | Fire all explicit-epoch closures on the calling locus |

The `e`-prefixed print variants route to stderr via `dprintf(2, ...)`,
sidestepping the cross-libc `stderr` FILE* macro shape.

`String + <printable>` is auto-coerced (since the 2026-05-11
ergonomics arc): `"port=" + port` works without explicit
`to_string`. The coercion is symmetric (`port + " is the port"`)
and chained (`"a=" + a + " b=" + b`); the non-String operand is
rendered through `value_to_string` and then concatenated.

Recovery primitives (`restart`, `restart_in_place`,
`quarantine`, `bubble`) are valid only inside `on_failure`
bodies.

## See Also

- [Memory model](./memory.md)
- [Bus dispatch](./bus/index.md)
- [Deployment](./deployment.md)
- [Locus declarations](./loci/index.md)
