# Concurrency & Placement

In Hale, every locus is single-threaded internally, and concurrency is configured declaratively on the `main locus` via a `placement { }` block. You write synchronous, race-free locus methods without `async`/`await` function coloring, and the deployment configuration assigns each top-level component to a shared cooperative pool, an event-driven `async_io` pool, or one or more dedicated `pinned` OS threads.

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
        gateway: pinned(core = 1);
        metrics: cooperative(pool = io) where async_io;
        ui:      cooperative(pool = render);
    }
}

fn main() { App { }; }
```

## Placement Modes

Placement entries are declared on `main locus` and keyed by the parameter **field name** (allowing two fields of the same locus type to be placed on different threads). Any top-level field omitted from `placement { }` defaults to `cooperative(pool = main)`.

| Placement Mode | OS Threads | Scheduling & I/O Behavior | Typical Use Case |
|---|---|---|---|
| `cooperative(pool = main)` *(default)* | Shares the process's main thread | Cooperative scheduling; handlers run to completion; `std::time::sleep` slices into `≤100ms` intervals to drain queued bus work. | Application coordination, low-rate control logic, CLI entry points. |
| `cooperative(pool = X)` | 1 worker thread per unique pool name `X` | Same cooperative semantics as `main` on a dedicated worker thread. Optional `cores = 0..=1` sets CPU affinity for pool `X`. | Isolating background subsystems (logging, metrics aggregation, rendering). |
| `cooperative(pool = X) where async_io` | 1 epoll worker thread per pool `X` | Blocking network/timer calls (`recv`, `accept`, `send`, `sleep`) transparently park and resume coroutines on readiness (`EPOLLIN`/`EPOLLOUT`). | High-concurrency network servers, WebSocket hubs, multi-source UDP/TCP ingest. |
| `pinned` / `pinned(core = N)` / `pinned(cores = 4..8)` | 1 dedicated OS thread | Exclusive thread ownership; never yields to other top-level loci. Supports single core, range (`4..8`, `4..=7`, `{4, 5}`), `node = N`, or `l3 = name`. | Latency-critical gateways, blocking hardware/C FFI loops, CPU-bound stages. |
| `pinned(..., replicas = K)` | `K` dedicated OS threads (`0..K-1`) | Spawns `K` independent single-threaded instances of the locus, distributing cores round-robin if `cores`/`node`/`l3` is specified. | Parallel worker fleets sharded via `subscribe T as h where key == replica`. |

> [!NOTE]
> **Starting a `main locus`**
> A `main locus` defines the deployment topology, and `fn main() { App { }; }` instantiates it. A library seed with a `main locus` and no `fn main()` passes `hale check` so other seeds can import it, but `hale build` and `hale run` require `fn main()`.

## Cooperative Pools & Automatic Sleep Slicing

Within any cooperative pool (`main` or `cooperative(pool = X)`), only one handler or step runs at a time, and handler bodies execute to completion without preemption. Loci yield control at well-defined boundaries: after a bus handler returns, on bus dispatch, on an explicit `yield`, or during `std::time::sleep`.

To prevent a long timer from starving co-resident loci on the same pool, `std::time::sleep` automatically slices sleeps into intervals of at most **100 ms** and drains pending bus deliveries on that pool (including the sleeping locus's own handlers) between slices:

```hale,fragment
run() {
    while !self.draining {
        self.send_heartbeat();
        std::time::sleep(30s);   // sliced into <=100ms chunks; co-resident bus
                                 // handlers on this pool continue firing
    }
}
```

- Sleep slicing drains queued **bus deliveries** on the pool, but does not start another locus's queued `run()` mid-sleep.
- On a `pinned` thread, `std::time::sleep` slices drain that locus's own mailbox.
- On a `where async_io` pool, `std::time::sleep` parks the coroutine on the epoll timer rather than slicing.

## Hardware Affinity & `topology { }`

On Linux, `pinned` and `cooperative` placements can bind threads to specific CPU cores, L3 cache domains, and NUMA nodes. (On macOS or when specified core indices exceed host CPU count, affinity is a best-effort no-op and threads run unpinned.)

Declare host hardware layout once in a `topology { }` block on `main` and reference domains symbolically in `placement { }`:

```hale
locus Matcher { }
locus Region  { }

main locus App {
    topology {
        reserve cores 0..2;              // reserved for OS / main thread
        node 0 {
            l3 fast { cores 4..8; }      // shared L3 cache domain (cores 4, 5, 6, 7)
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
        matcher: pinned(l3 = fast);      // binds thread to cores 4..8 and memory to node 0
        region:  pinned(node = 0);       // binds thread to cores 4..12 and memory to node 0
    }
}

fn main() { App { }; }
```

- **Compile-time topology verification**: Node IDs must be unique, L3 domain names must be unique plain identifiers (not reserved keywords like `match`), core sets must be mutually disjoint and must not overlap `reserve cores`, and every `pinned(node = ...)` or `pinned(l3 = ...)` must reference a declared domain.
- **NUMA thread and memory co-location**: `pinned(node = N)` and `pinned(l3 = name)` bind both the OS thread affinity and the locus's memory arena (plus per-call method scratch memory) to the target NUMA node via `mbind` on Linux, eliminating cross-node memory traffic.

## Parallel Worker Fleets (`replicas = K`)

Because cooperative pools are strictly single-threaded to preserve lock-free ring invariants, data-parallel execution uses `pinned(..., replicas = K)` to spawn `K` independent single-threaded instances:

```hale,fragment
type Conn { fd: Int; shard: Int; }
topic NewConn { payload: Conn; keyed_by shard; }

locus Worker {
    bus { subscribe NewConn as on_conn where key == replica; }
    fn on_conn(c: Conn) { /* receives only messages where c.shard == replica */ }
}

// In main locus App:
// placement { workers: pinned(cores = 4..12, replicas = 8); }
// Listener publishes: NewConn <- Conn { fd: fd, shard: fd % 8 };
```

- Replicas are pull-based workers, not indexable handles (you cannot call `self.workers[i]`).
- An unfiltered `subscribe NewConn as on_conn;` broadcasts every message to all `K` replicas.
- Filtering with `where key == replica` routes each message to the single replica whose 0-based index (`0..K-1`) matches the payload's key field.
- `replicas = K` is valid only on `pinned` placements; `cooperative(..., replicas = K)` is a compile error.

### Auditing Thread Counts

The compiler resolves placement into a static thread table that you can inspect or gate in CI:

```sh
hale check --dump-resource-budget app.hl
hale check --check-resource-budget app.hl
```

- Every `pinned` field counts as `1` thread (`replicas = K` counts as `K` threads) per simultaneously live instance of `main`.
- Every named `cooperative(pool = X)` counts as `1` worker thread regardless of how many loci share pool `X` (`main` uses the process's initial thread).
- Every transport adapter in `bindings { }` counts as `1` pinned thread (constructed once per process).
- Constructing `main` inside a loop produces an `uncertain` static thread bound, which fails thread-ceiling checks.

## Pool Inheritance & Co-Locating Helpers

Only top-level fields of `main` take `placement { }` entries; any child locus constructed inside another locus's `params` or methods inherits its parent's thread or pool.

To co-locate helper loci (such as metrics counters or lookup tables) on a `pinned` locus's dedicated thread so the `pinned` locus can call their methods directly, declare them as `params` of the `pinned` locus:

```hale
locus Gateway {
    params {
        reg:   std::metrics::Registry = std::metrics::Registry { namespace: "gw" };
        ticks: std::metrics::Counter  = std::metrics::counter(
                   self.reg, "ticks", std::metrics::labels_empty());
    }
    // self.ticks.inc() runs directly on Gateway's pinned thread
}
```

### Worker Startup & Dynamic Subscribers

- **Target-thread initialization**: When a field is placed on `pinned` or `cooperative(pool = X)`, its `params` defaults (along with each nested helper's `birth()` and inline `run()`) and its own `birth()` execute on the target thread/worker before any bus messages are delivered to it. While the constructing thread waits for `birth()` to finish, it continues servicing its own bus queue and can settle held startup failures even if queues are full. Scalar override expressions in the parent literal (`Gateway { started: std::time::monotonic_ns() }`) evaluate on the caller's thread, while nested locus literals (`Gateway { reg: std::metrics::Registry { ... } }`) construct on the target thread.
- **Method-local subscribers**: A subscriber constructed inside a method (`let feed = Feed { };`) registers on the executing thread. On a `pinned` thread, its handlers run during sleeps/yields (and the `pinned` thread keeps servicing its mailbox until program shutdown before running `dissolve()`). On a `where async_io` pool, handlers run while parked. On a classic non-`async_io` cooperative pool, handlers cannot run before the method returns and dissolves `feed`, so `hale check` warns that deliveries will be dropped.
- **Transport adapters**: Any adapter locus in `bindings { }` runs on its own `pinned` thread and obeys the same restrictions as `pinned` loci: it cannot declare `accept()` or closures with `birth`/`dissolve` epochs (`epoch inline` closures are permitted).

## High-Concurrency I/O (`where async_io`)

Tagging a cooperative pool with `where async_io` backs the pool's worker thread with an epoll event loop:

```hale,fragment
placement {
    workers: cooperative(pool = ws) where async_io;
}
```

- Blocking I/O operations (`recv`, `accept`, `send`, `std::time::sleep`, subprocess drains) transparently park the calling coroutine until file descriptor readiness and resume without blocking the OS thread.
- Bus handler payload parameters remain valid across coroutine parks until the handler returns, even if the pool dispatches other messages in the meantime.
- `api::Exposure` (`api::serve(...)`) listeners and connection coroutines run on the `where async_io` pool where the exposure or server parameter is placed.

## Single-Threaded Invariant & Cross-Pool Rules

Because a locus's state and arena are owned exclusively by its assigned thread/pool, **direct method calls and arena-allocating field stores across pools are rejected at compile time**:

- **Cross-pool method calls and stores**: Calling `self.other.foo()` or storing a non-scalar field (`self.other.name = s`) when `other` runs on a different pool is a compile error—whether accessed directly, via a local variable (`let o = self.other;`), passed to a free function (`poke(self.other)`), nested inside an array/child (`self.other.helpers[i].foo()`), or typed via an alias, stdlib locus, or `interface`. Coordinate across pools by publishing a bus message instead. (Storing a primitive scalar such as `self.other.gate = 1` does not touch `other`'s arena and is permitted.)
- **Runtime guard for function pointers**: If a cross-pool access escapes static call-graph analysis via a function pointer (such as an HTTP callback reading a locus on another pool), the runtime checks the owning pool ID on access and exits with code `1`:
  ```text
  main.hl:8:9: violation: cross-pool store into `W.name` from pool `io`; this `W` runs on pool `main`: reach it through a method on the bus
  ```
- **No cross-pool instance aliasing**: Passing an already-bound locus instance (`self.a` on `main`) into a child or handler on another pool (`pool = io`) is rejected at compile time (`cross-pool alias`). By contrast, constructing a fresh locus (`let web = Web { };` in `fn main`) and passing it into an `io`-pool server *moves* ownership of `web` (and its subtree) onto the `io` pool.

## Compile-Time Placement & Bus Diagnostics

`hale check` verifies placement and bus topology statically against the assembled program graph:

| Diagnostic | Severity | Trigger Condition & Resolution |
|---|---|---|
| **Dead bus receiver** | Error | A cooperative subscriber on a non-`main` pool makes a direct blocking call (`block` effect, excluding sliced `std::time::sleep`) in `run()`, starving its own bus dispatch. Fix: place on `pinned`, add `where async_io`, or make `run()` non-blocking. |
| **Blocking call on cooperative pool** | Warning | A locus on a cooperative pool without `where async_io` makes a direct or indirect `block`-classified call in `run()`, stalling co-resident loci. Fix: use `pinned` or `where async_io`. |
| **Dropped scope-local subscriber** | Warning | A method on a classic cooperative pool (without `where async_io`) instantiates a local subscriber that dissolves before the pool can dispatch to it. |
| **Orphan bus topic** | Warning | In a complete program (with a top-level `main locus` and no `api:` binding), a topic has only publishers, only subscribers, or neither (suppressed for bound topics, wildcards, cross-seed topics, or self-pub/sub). |
| **Intra-locus / cross-locus bus cycle** | Error / Warning | An unconditional handler republish loop within a single locus is an **error** (infinite synchronous recursion); a cycle across distinct loci is a **warning** (`if`-guarded state machine cycles are permitted). |
| **Unthrottled publish loop** | Warning | A `while true` loop publishes without a pacing point (`yield`, `std::time::sleep`/`tick`, blocking input read, or `break`/`return`). |
| **Subject payload mismatch** | Error | Two `bus { }` blocks publish or subscribe to the same literal subject string with different `of type` payload types. |
| **`pinned` placement in a loop** | Error | A locus with `pinned` placement is constructed directly inside a `while`/`for` loop, `const`, type field default, or closure assertion (orphaning thread join slots). Wrap the construction in a helper function called from the loop so each call joins before returning. |
| **Placement on a factory call** | Error | A `placement { }` entry targets a parameter initialized by a function call (`a: Worker = make_worker();`) instead of a locus literal (`a: Worker = Worker { };`). |

```hale,refused
locus Worker { }
fn make_worker() -> Worker { return Worker { }; }

main locus App {
    params {
        a: Worker = make_worker();   // error: factory return cannot carry `a: pinned`
        b: Worker = Worker { };      // ok: literal carries placement
    }
    placement {
        a: pinned;
        b: pinned;
    }
}
```

Next: [Perspectives & Interfaces](./perspectives.md).
