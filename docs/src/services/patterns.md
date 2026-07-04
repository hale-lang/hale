# Composition patterns

The [shape catalog](../../AGENTS.md) names the six building blocks —
app locus, namespace lotus, service locus, spawned child, shape
type, free fn. This chapter is the next layer up: five *compositions*
of those blocks that recur in real Hale services, distilled from
production use. Reach for one of these when a problem feels like it
needs a new language feature — usually it doesn't, it needs one of
these shapes.

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
  and lives until the manager dissolves — unbounded on a daemon).
- Each **child** subscribes with a key filter (`subscribe Update as
  on_update where key == self.id`) so the bus routes only its own
  entity's messages to it.

This gives you per-entity state and lifecycle without a map of loci —
the bus *is* the routing table, keyed.

## 2. Demand-driven discovery

A special case of the gateway with **zero hardcoded topology**: the
manager doesn't know its children up front. A subscription *triggers*
the `accept()`:

```hale
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
bubbles to the [nearest accepting ancestor](./parents-children.md)
— v0.9.2.)

## 3. Hot-path counters & gauges (and the CQRS rejection)

You will want to write `let n = self.metrics.incr("hits")` on a hot
path. Hale **rejects** locus methods that return locus values
(GH #18.6 / the "CQRS" shape) — a method call that hands back a live
locus reference breaks the closed-world ownership the substrate
relies on. The rejection without a replacement strands you, so here
is the migration:

- **Pre-allocated handles at boot.** Declare the counter/gauge loci
  as `params` of the owner, instantiated once at birth. The hot path
  mutates a field in place (`self.hits = self.hits + 1`) — no method
  returning a locus, no per-call allocation.
- **Bus-routed single-writer store.** For shared metrics, publish a
  `MetricUpdate { name, delta }` to a single collector locus that
  owns the store and applies updates in its handler. One writer, no
  contention, and the closed-world rewrite keeps the publish
  synchronous. This is the shape `pond/metrics`' `MetricsCollector`
  uses.

Either way the hot path does an in-place field write or a publish —
never a method that returns a locus.

## 4. The publish-policy gate

When you produce data faster than you want to publish it (telemetry,
book snapshots), gate the publish behind a `tick()` with a
time-or-volume trigger rather than publishing per-update:

```hale
fn on_update(u: Update) {
    self.pending = self.pending + 1;
    self.acc = self.acc + u.delta;          // accumulate in place
    if self.pending >= 100 { self.flush(); } // volume trigger
}
fn tick() {                                  // time trigger (scheduled)
    if self.pending > 0 { self.flush(); }
}
fn flush() {
    "snapshot" <- Snapshot { total: self.acc };
    self.pending = 0;
}
```

The accumulation is in-place; only the flush crosses the bus. This
keeps the high-frequency path allocation-free and bounds publish
volume independently of input volume.

## 5. View lifetime — copy out to persist

The zero-copy span/JSON APIs (`StringView`, `BytesView`,
`std::json::*_span`) hand you a **view into a buffer you don't own**.
That view is valid only until the next operation that overwrites the
buffer — the next `recv`, the next ring read. Holding it across that
boundary reads freed/overwritten memory:

```hale
let name = std::json::find_string_field(msg, "name");  // view into recv buf
self.read_msg();                                       // ← overwrites the buffer
println(name);                                         // ✗ dangling view
```

The rule: **a view is valid until the next recv/overwrite; copy out
to persist.** Materialize it before the boundary:

```hale
let name = std::str::clone(std::json::find_string_field(msg, "name"));
self.read_msg();
println(name);   // ✓ owns its own copy
```

Forgetting this is now **panic-guarded** (a stale-view access exits
with a diagnostic rather than reading garbage), so you'll see a clear
"view used after its buffer was overwritten" message instead of a
silent corruption — but the fix is always to clone out before the
overwriting call.
