# Composition Patterns

This catalog documents the standard multi-locus architectural patterns used in production Hale services. Rather than storing loci inside hash maps, returning locus references from methods, or allocating per-message buffers, combine lifecycle hooks, keyed bus routing, and placement declarations using these compositions.

```hale
type Update   { delta: Int; }
type Snapshot { total: Int; }
topic Updates   { payload: Update; }
topic Snapshots { payload: Snapshot; }

locus Batcher {
    params { pending: Int = 0; acc: Int = 0; }
    bus {
        subscribe Updates as on_update;
        publish   Snapshots;
    }

    fn on_update(u: Update) {
        self.pending = self.pending + 1;
        self.acc = self.acc + u.delta;
        if self.pending >= 100 { self.flush(); }
    }
    run() {
        while !self.draining {
            std::time::sleep(1s);
            if self.pending > 0 { self.flush(); }
        }
    }
    fn flush() {
        Snapshots <- Snapshot { total: self.acc };
        self.pending = 0;
    }
}
```

## Pattern Catalog Summary

| # | Pattern | Problem Solved | Key Primitives |
|---|---|---|---|
| 1 | **Three-Locus Gateway & Replicated Workers** | Managing `N` dynamic per-entity children or a fixed `K`-worker fleet without a hash map of loci. | `pinned` reader, `accept` + `release`, `keyed_by` + `where key == self.id` (or `replica`). |
| 2 | **Demand-Driven Discovery** | Spawning per-entity state dynamically on first contact with zero hardcoded topology. | Bus subscription handler instantiating `Child { id: s.id };` inside an `accept` scope. |
| 3 | **Hot-Path Counters & Gauges** | Updating metrics without methods that return locus handles (rejected by CQRS ownership rules). | Pre-allocated `std::metrics::Counter` in `params`, or single-writer bus collector. |
| 4 | **Publish-Policy Gate** | Decoupling high-frequency state updates from downstream bus publish rate. | In-place field accumulation flushed by a dual volume/timer gate. |
| 5 | **View Lifetimes (Copy Out to Persist)** | Safely reading zero-copy views (`StringView`, `BytesView`, JSON spans) across buffer refills. | `std::str::clone(view)`, `to_string(view)`, or `f"{view}"` before the next `recv`/overwrite. |
| 6 | **Reused-Buffer Connection** | Zero-allocation per-frame network I/O and frame reassembly. | Pre-allocated `std::bytes::BytesBuilder` fields (`rx_buf`, `tx_buf`, `scratch`) in `params`. |
| 7 | **Pre-Render Once, Fan Out Many** | Broadcasting serialized frames to `N` subscribers without `N×` serialization work. | Render shared JSON payload once at ingest; prepend only per-connection sequence numbers. |
| 8 | **Event-Driven Ingest** | Multiplexing many blocking UDP/TCP sources onto one thread without polling sleep latency. | `cooperative(pool = ingest) where async_io` + `self.src.next() or raise` (`EPOLLIN`). |
| 9 | **Decoder, Handler, Seam** | Exhaustive, zero-boilerplate dispatch across message union sets (`A \| B`). | `seam` chaining decoder, locus hop (`self.desk`), and encoder over set types. |

---

## 1. Three-Locus Gateway & Replicated Worker Pool

Because loci are live runtime structures with arenas and lifecycles, Hale forbids storing loci inside `@form(hashmap)` collections. Instead, use the **bus as your keyed routing table** across a three-locus pipeline:

```text
pinned reader  ──▶  cooperative manager  ──▶  keyed per-entity child
(owns fd/ring,       (accept()s + release()s   (subscribe ... where
 publishes events)    one child per key)        key == self.id)
```

1. **Pinned reader**: Owns the blocking socket or ring on a dedicated `pinned` thread and publishes decoded events onto the bus.
2. **Cooperative manager**: Subscribes to entity-creation events and spawns one child per entity via `accept(c: Child)`. Always declare `release(c: Child)` on the manager so completed children reclaim their arenas immediately rather than accumulating as residents.
3. **Keyed per-entity child**: Subscribes to a `keyed_by` topic using `where key == self.id` (supports scalar and hash-gated `String` keys):

```hale,fragment
topic Update { payload: Tick; keyed_by id; }

// Inside the per-entity child locus:
bus { subscribe Update as on_update where key == self.id; }
```

The runtime routes each message directly to the matching child rather than broadcasting to all `N` children and filtering with `if u.id == self.id` inside the handler.

### Variant: Fixed Replicated Worker Pool (`replicas = K`)

When you need fixed data-parallelism across `K` CPU cores rather than dynamic per-entity lifecycles, combine `pinned(..., replicas = K)` with `where key == replica`:

```hale,fragment
type Conn { fd: Int; shard: Int; }
topic NewConn { payload: Conn; keyed_by shard; }

locus Worker {
    bus { subscribe NewConn as on_conn where key == replica; }
    fn on_conn(c: Conn) { /* handles shard == replica */ }
}

// In main locus:
// placement { workers: pinned(cores = 4..12, replicas = 8); }
```

## 2. Demand-Driven Discovery

When the set of active entities is unknown at startup, trigger child creation directly from a bus subscription handler:

```hale,fragment
// Inside the manager locus (which declares `accept(c: Child)` and `release(c: Child)`):
bus { subscribe "entity.first_seen" as on_seen of type Seen; }

fn on_seen(s: Seen) {
    // Bare statement instantiation inside a parent method attaches the child
    // via the enclosing `accept(c: Child)` hook (or bubbles to the nearest accepting ancestor).
    Child { id: s.id };
}
```

Combined with `release(c: Child)`, children spawn on first contact and free their arenas as soon as their `run()` completes.

## 3. Hot-Path Counters & Gauges (No Locus Return)

Hale rejects locus methods that return locus references (such as `let c = self.metrics.counter("hits")`) because returning interior locus handles breaks closed-world arena ownership. Use one of two patterns instead:

- **Pre-allocate handles in `params` at birth**: Instantiate `std::metrics::Registry` and `std::metrics::Counter` as `params` defaults on the owning locus (see [Concurrency & Placement](./concurrency.md#pool-inheritance--co-locating-helpers)). On the hot path, mutate in place via `self.ticks.inc()` or `self.hits = self.hits + 1;`.
- **Single-writer bus collector**: For metrics shared across pools, publish a `MetricUpdate { name, delta }` value to a dedicated collector locus that owns the registry and updates counters in its handler.

## 4. Publish-Policy Gate

When an upstream source produces updates faster than downstream consumers need them (such as order-book ticks or telemetry), accumulate updates in place and flush behind a dual volume-and-time trigger:

```hale
type Update   { delta: Int; }
type Snapshot { total: Int; }
topic Updates   { payload: Update; }
topic Snapshots { payload: Snapshot; }

locus Batcher {
    params { pending: Int = 0; acc: Int = 0; }
    bus {
        subscribe Updates as on_update;
        publish   Snapshots;
    }

    fn on_update(u: Update) {
        self.pending = self.pending + 1;
        self.acc = self.acc + u.delta;           // in-place scalar accumulation
        if self.pending >= 100 { self.flush(); } // volume trigger
    }
    run() {                                      // periodic timer trigger
        while !self.draining {
            std::time::sleep(1s);
            if self.pending > 0 { self.flush(); }
        }
    }
    fn flush() {
        Snapshots <- Snapshot { total: self.acc };
        self.pending = 0;
    }
}
```

## 5. View Lifetimes — Copy Out to Persist

Zero-copy views (`StringView`, `BytesView`, `BytesBuilder.view()`, `BytesBuilder.text_view()`, and `std::json::*_span` cursors) borrow directly from an underlying receive or builder buffer. **A view is valid only until the next operation that mutates or overwrites that buffer:**

```hale,fragment
// INVALID: view held across buffer refill
let name = self.rx_buf.text_view();
self.read_msg();                      // overwrites self.rx_buf
println(name);                        // runtime violation: stale view!
```

To retain data across a buffer refill or handler boundary, copy it into an owned `String` before the next read:

```hale,fragment
// VALID: copy out before refilling rx_buf
let name = std::str::clone(self.rx_buf.text_view());
self.read_msg();
println(name);
```

- `std::str::clone(view)`, `to_string(view)`, `"name=" + view`, and `f"{view}"` all allocate an owned `String` copy.
- Views derived from `BytesBuilder` are **panic-guarded** at runtime: reading a `StringView` or `BytesView` after its source `BytesBuilder` has been mutated terminates the process with `violation: StringView read after source BytesBuilder mutated` rather than silently reading corrupted bytes.

## 6. Reused-Buffer Connection

Connection loci that parse and assemble frames at high frequency should pre-allocate `std::bytes::BytesBuilder` buffers in `params` and reuse them across frames rather than instantiating builders inside handler methods:

```hale,fragment
locus WsConn {
    params {
        rx_buf:  std::bytes::BytesBuilder = ...;  // inbound frame reassembly
        tx_buf:  std::bytes::BytesBuilder = ...;  // outbound frame assembly
        scratch: std::bytes::BytesBuilder = ...;  // unmask / decompression scratch
    }
    fn on_data(...) {
        std::io::tcp::recv_into(self.fd, self.rx_buf, 65536);
        // parse zero-copy from self.rx_buf.view()
    }
}
```

- `std::io::tcp::recv_into` requires a `std::bytes::BytesBuilder` as its second argument and appends directly into the builder's capacity in place.
- Clear the builder at the **start** of the next frame cycle (rather than the end of the current cycle) so any views handed out during frame processing remain valid until the next frame begins.

## 7. Pre-Render Once, Fan Out Many

When broadcasting an event to `N` client connections, serialize the shared payload once at ingest and have each connection prepend only its client-specific header or sequence number:

```hale,fragment
// At ingest: serialize shared payload once
self.update_json = render_update(t);
Tick <- Update { json: self.update_json };

// In each connection locus: prepend per-connection sequence only
fn on_update(u: Update) {
    self.send_frame("" + self.seq + u.json);
}
```

For emitters used on both cold and hot paths, provide two functions: a `std::json::Builder` version for cold paths and a raw-concatenation `_pre` version for hot fan-out paths.

## 8. Event-Driven Ingest (`where async_io`)

To ingest from multiple sockets without burning a thread per socket or adding polling sleep latency (`set_recv_timeout`), place reader loci on a `cooperative(pool = ...) where async_io` pool:

```hale
locus Reader {
    params {
        port: Int = 0;
        src: std::io::udp::Reader =
            std::io::udp::Reader { addr: "127.0.0.1", port: self.port, cap: 2048 };
    }
    run() {
        while !self.draining {
            let frame = self.src.next() or raise;   // parks coroutine on EPOLLIN (~4 us wake)
            // decode `frame` and publish to bus
        }
    }
}

main locus App {
    params { r0: Reader = Reader { port: 9000 }; r1: Reader = Reader { port: 9001 }; }
    placement {
        r0: cooperative(pool = ingest) where async_io;
        r1: cooperative(pool = ingest) where async_io;
    }
}

fn main() { App { }; }
```

Each `self.src.next()` parks its coroutine on `EPOLLIN`, allowing `N` readers to share a single worker thread with microsecond wake latency. (Operations that cannot park on epoll, such as blocking TLS handshakes, should be placed on `pinned` threads instead.)

## 9. Decoder, Handler, Seam

When an ingress boundary decodes wire bytes into one of several message types, handles each variant, and encodes a reply, use a [`seam`](./seams.md) over [set types](../everyday/records.md#sets--one-of-several-types) so the compiler generates and verifies exhaustive dispatch:

```hale,fragment
type Inbound  = NewOrder | Cancel;
type Outbound = Ack | Reject | WireError;

locus Wire {
    fn decode(b: String) -> Inbound fallible(WireError) { … }
    fn encode(o: Outbound) -> String { … }
}

locus Desk {
    fn new_order(n: NewOrder) -> Outbound { … }
    fn cancel(c: Cancel) -> Outbound { … }
}

locus Gateway {
    params { feed: Feed = Feed { }; wire: Wire = Wire { }; desk: Desk = Desk { }; log: Log = Log { }; }
    seam {
        self.feed.next() -> self.wire.decode -> self.desk -> self.wire.encode -> self.log.write;
        or end;
    }
}
```

- `self.desk` is a **locus hop**: the compiler routes each member of `Inbound` (`NewOrder`, `Cancel`) to the unique `Desk` method accepting that parameter type, failing the build if any member is unhandled or ambiguous.
- Because `WireError` is a member of `Outbound`, `decode` failures automatically route forward to `self.wire.encode` and become wire replies; unhandled errors are routed via `or end;` or `or violate <closure>;`.

Next: [Seams](./seams.md).
