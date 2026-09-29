# Composition patterns

The [shape catalog](https://github.com/hale-lang/hale/blob/main/AGENTS.md) names the seven building blocks —
app locus, namespace locus, service locus, spawned child, `@form`
collection with a domain facade, shape type, free fn. This chapter
is the next layer up: the *compositions* of those blocks that recur
in real Hale services, distilled from production use. Reach for one
of these when a problem feels like it needs a new language feature —
usually it doesn't, it needs one of these shapes.

## 1. The three-locus gateway

The canonical answer to "I have N dynamic, keyed children with their
own lifecycles" (and to the rejection of putting loci in a hashmap):

```
pinned reader  ──▶  cooperative manager  ──▶  keyed per-entity child
(owns the fd,        (accept()s a child       (subscribe ... where
 publishes events)    per new key)             key == self.id)
```

- A **pinned** locus owns the blocking input (socket, ring) on its
  own thread and publishes decoded events onto the bus.
- A **cooperative manager** subscribes to "new entity" events and
  `accept()`s one child per key. Declare `release(c: Child)` so each
  child is reclaimed when its flow ends (otherwise it's a resident
  and lives until the manager dissolves — unbounded on a daemon; the
  compiler warns on that shape).
- The **topic declares its routing field** and each child subscribes
  with a key filter, so the bus delivers only that entity's traffic:

  ```hale,fragment
  topic Update { payload: Tick; keyed_by id; }
  // in the child:
  bus { subscribe Update as on_update where key == self.id; }
  ```

  Keys can be scalars or `String` — a symbol id and a symbol *name*
  both work (String keys are hash-gated, so non-matching traffic
  costs one integer compare per entry).

This gives you per-entity state and lifecycle without a map of loci —
the bus *is* the routing table, keyed. Filtering in the handler
instead (`if u.id == self.id`) is the anti-pattern this composition
deletes: it delivers every message to every child and discards
N-1 of N.

## 2. Demand-driven discovery

A special case of the gateway with **zero hardcoded topology**: the
manager doesn't know its children up front. A subscription *triggers*
the `accept()`:

```hale,fragment
// manager
bus { subscribe "entity.first_seen" as on_seen of type Seen; }
fn on_seen(s: Seen) {
    // First message for this key → spawn its child now.
    // Bare instantiation inside a parent method attaches the child:
    // it triggers the enclosing accept(c) gatekeeper. `accept` is a
    // lifecycle hook the runtime invokes, never a method you call.
    Child { id: s.id };
}
```

The topology grows from the data. Combined with `release`, children
appear on first contact and vanish when their flow ends — the
process shape mirrors the live workload with no configuration. (If
the manager doesn't itself `accept` this child type, the child
bubbles to the [nearest accepting ancestor](./parents-children.md).)

## 3. Hot-path counters & gauges (and the CQRS rejection)

You will want to write `let n = self.metrics.incr("hits")` on a hot
path. Hale **rejects** locus methods that return locus values
(the "CQRS" shape) — a method call that hands back a live
locus reference breaks the closed-world ownership the substrate
relies on. The rejection without a replacement strands you, so here
is the migration:

- **Pre-allocated handles at boot.** Declare the counter/gauge loci
  as `params` of the owner, instantiated once at birth —
  `std::metrics::counter(...)` as a param default, as
  [Concurrency & placement](./concurrency.md) shows. The hot path
  mutates in place (`self.ticks.inc()`, or a plain `self.hits =
  self.hits + 1`) — no method returning a locus, no per-call
  allocation.
- **Bus-routed single-writer store.** For shared metrics, publish a
  `MetricUpdate { name, delta }` to a single collector locus that
  owns the store and applies updates in its handler. One writer, no
  contention.

Either way the hot path does an in-place field write or a publish —
never a method that returns a locus.

## 4. The publish-policy gate

When you produce data faster than you want to publish it (telemetry,
book snapshots), gate the publish behind a time-or-volume trigger
rather than publishing per-update:

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
        self.acc = self.acc + u.delta;           // accumulate in place
        if self.pending >= 100 { self.flush(); } // volume trigger
    }
    run() {                                      // time trigger
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

The accumulation is in-place; only the flush crosses the bus. This
keeps the high-frequency path allocation-free and bounds publish
volume independently of input volume.

## 5. View lifetime — copy out to persist

The zero-copy APIs (`StringView`, `BytesView`, a builder's `.view()`
/ `.text_view()`, the `std::json::*_span` cursors) hand you a **view
into a buffer you don't own**. That view is valid only until the next
operation that overwrites the buffer — the next `recv`, the next ring
read. Holding it across that boundary reads bytes that are no longer
the ones you looked at:

```hale,fragment
let name = self.rx_buf.text_view();   // view into the receive buffer
self.read_msg();                      // ← refills rx_buf
println(name);                        // ✗ stale view
```

The rule: **a view is valid until the next recv/overwrite; copy out
to persist.** Materialize it before the boundary:

```hale,fragment
let name = std::str::clone(self.rx_buf.text_view());
self.read_msg();
println(name);   // ✓ owns its own copy
```

A builder's views are **panic-guarded**: reading one after its
`BytesBuilder` changed exits with a diagnostic rather than reading
garbage — `violation: StringView read after source BytesBuilder
mutated …` — instead of a silent corruption. The fix is always to
copy out before the overwriting call.

## 6. The reused-buffer connection

Every production connection locus converges on the same three
fields: buffers held as `params`, reused every frame, never
instantiated in a handler:

```hale,fragment
locus WsConn {
    params {
        rx_buf:  std::bytes::BytesBuilder = ...;  // frame reassembly
        tx_buf:  std::bytes::BytesBuilder = ...;  // send assembly
        scratch: std::bytes::BytesBuilder = ...;  // unmask / inflate
    }
    fn on_data(...) {
        std::io::tcp::recv_into(self.fd, self.rx_buf, 65536);
        // parse from self.rx_buf.view() — zero-copy
    }
}
```

The buffer argument is a `std::bytes::BytesBuilder` and nothing
else — `recv_into` fills the builder's tail in place, so a `Bytes`
value or a plain `Int` is not a smaller version of the same thing.
Pass one and `hale check` says so, at the argument:

```text
`std::io::tcp::recv_into` argument 2: expected
`std::bytes::BytesBuilder`, got `Int`
```

A builder created *inside* the handler is a fresh heap buffer per
message that reclaims only at method return (the compiler warns).
Read with `.view()` / `.text_view()` and clear the buffer at the
*start* of the next cycle, not the end of this one, so views handed
out stay valid between frames. The full hot-path discipline lives in
the [styleguide §4](https://github.com/hale-lang/hale/blob/main/spec/styleguide.md)
and [Performance](../systems/performance.md).

## 7. Pre-render once, fan out many

When one event reaches N consumers, render the shared payload once
at ingest; each consumer adds only its per-consumer delta:

```hale,fragment
// ingest: render once
self.update_json = render_update(t);          // one render
Tick <- Update { json: self.update_json };    // fan out
// each connection: prepend its own seq only
fn on_update(u: Update) { self.send_frame("" + self.seq + u.json); }
```

N× a tiny prepend beats N× a full render. Pair it with the
**two-flavor emitter** convention: a `std::json::Builder` version
for cold paths (readable, allocating) and a raw-concat `_pre`
version for the hot fan-out — keeping both documents which callers
are hot.

## 8. Event-driven ingest

One reader locus per source, parked on readiness — never a poll
loop with `set_recv_timeout`:

```hale
locus Reader {
    params {
        port: Int = 0;
        src: std::io::udp::Reader =
            std::io::udp::Reader { addr: "127.0.0.1", port: self.port, cap: 2048 };
    }
    run() {
        while !self.draining {
            let frame = self.src.next() or raise;   // parks on EPOLLIN
            // decode `frame`, publish what it says
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

Each reader's `next()` parks its coroutine on EPOLLIN; N readers
share one pool worker with microsecond wakes (measured ~4 µs p50).
Poll-scanner sleeps accumulate tail-latency debt that looks like a
runtime problem but is the sleep schedule. Blocking I/O that can't
park (a TLS handshake, say) goes on `pinned` instead — see
[Concurrency & placement](./concurrency.md).
