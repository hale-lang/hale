# The Bus

The Hale bus provides typed, many-to-many message passing between loci without shared pointers or channel plumbing. Loci declare which topics they publish and subscribe to inside a `bus { }` block, and the compiler and runtime wire delivery—from zero-overhead synchronous calls on the same thread to lock-free rings across threads or sockets across processes.

```hale
type Order { id: String; amount: Decimal; }

topic OrderPlaced  { payload: Order; }
topic OrderShipped { payload: Order; }

locus Warehouse {
    bus {
        subscribe OrderPlaced as on_order;
        publish   OrderShipped;
    }

    fn on_order(o: Order) {
        OrderShipped <- o;
    }
}
```

> [!NOTE]
> **Vertical Coordination via the Root**
> In Hale's [parent/child hierarchy](./parents-children.md), direct references flow strictly vertically between parents and children. The bus preserves this invariant by routing messages through topic channels owned at the runtime root: publishers send up to the topic, and the topic dispatches down to subscribers, allowing loci on separate branches to coordinate without lateral pointers.

## Declaring Topics & Payloads

A `topic` is a top-level declaration that pairs a channel name with a payload type:

```hale,fragment
type Order { id: String; amount: Decimal; }

topic OrderPlaced { payload: Order; }
```

- **Shared top-level namespace**: Topics share the module namespace with `type` and `locus` declarations. Declaring both `type Tick` and `topic Tick` is a compile error; name topics after events (`TickArrived`, `OrderPlaced`) or add a suffix (`TickTopic`).
- **Structured payload requirement**: Because bus deliveries serialize structured records, a topic's `payload` must be one of three types:
  1. A user-defined `type` (struct).
  2. An `enum` where at least one variant carries a payload.
  3. `BytesView` (for reading raw frames from an external writer).

Bare primitives (`Int`, `String`, `Decimal`) cannot be used directly as bus payloads and are rejected at compile time:

```hale,fragment
bus { publish "org.metrics" of type Int; }   // error: `Int` cannot be carried directly on the bus
```

Wrap scalar values in a single-field record (`type Metric { n: Int = 0; }`).

## Publishing & Subscribing (`bus { }`)

Every locus declares its bus inputs and outputs inside a `bus { }` block:

| Syntax | Role | Description |
|---|---|---|
| `subscribe Topic as handler;` | Inbound | Registers `fn handler(p: Payload)` to receive messages sent to `Topic`. |
| `publish Topic;` | Outbound | Authorizes the locus to send messages on `Topic`. Publishing without this declaration is a compile error. |
| `Topic <- value;` | Statement | Sends `value` to `Topic`. Produces no return value. |
| `publish "prefix.**" of type T;` | Wildcard outbound | Authorizes publishing `T` to runtime-computed subject strings matching `"prefix.**"`. |

Subscriptions are declarative: the runtime registers a locus's subscriptions when it is constructed (before `birth()`) and deregisters them automatically when it dissolves. Short-lived subscribers (such as per-connection loci) reuse subscription table slots as they dissolve, and any queued messages addressed to a dissolved subscriber are dropped rather than delivered to a subsequent occupant of that slot. (Note: `String` routing keys registered by dynamic subscribers remain interned until process exit.)

### Dynamic Wildcard Subjects

When a locus computes the target subject at runtime—such as a structured logger targeting `log.<component>`—declare a wildcard pattern in `bus { }` and send to a `String` expression:

```hale
type LogEvent { phase: String; }

locus Trace {
    params { component: String = "root"; }
    bus { publish "log.**" of type LogEvent; }

    fn note(phase: String) {
        let subject = "log." + self.component;
        subject <- LogEvent { phase: phase };
    }
}
```

Computed subject publishes enforce two checks at runtime (static `Topic <- v` sends are verified at compile time and incur zero runtime check cost):

1. **Pattern authorization**: The runtime subject string must fall under a pattern declared in the locus's `bus { }` block; otherwise the send raises `BusPublishUnauthorized`.
2. **Payload agreement**: The subject must not match a subscriber expecting a different payload type (`BusPayloadMismatch`). `hale check` also warns at compile time if a subscription sits under another locus's wildcard pattern with a mismatched payload type.

Keep wildcard patterns as narrow as possible (`"log.**"` rather than `"**"`). Static analysis uses wildcard prefixes to prove that a logger cannot publish to unrelated application topics (such as `app.order`) when verifying `claims { }` publisher counts.

## Delivery Guarantees, Ordering, and Lowering

### In-Process vs. Bound Delivery

A send statement `OrderShipped <- o;` requires no `or` error handler by default because the broker only accepts messages it can deliver under the topic's contract:

- **In-process topics**: Delivered to every live subscriber. Payload size is bounded only by the payload arena (e.g., a record containing a 1 MiB `String` transfers intact).
- **Bound topics ([Multi-Binary Deployment](./multi-binary.md))**: Governed by the transport binding. If a declared binding cannot be opened at startup, the process refuses to boot rather than dropping messages silently. Bound transports frame messages in **64 KiB** chunks: a payload exceeding 64 KiB still reaches in-process subscribers, but is dropped for remote peers (logging a warning to stderr once per subject).

### FIFO Ordering

All messages published by a single publisher arrive in FIFO send order across all topics on the same thread:

```hale,fragment
Prices <- p;
Orders <- o;
```

Every same-thread subscriber receives `p` before any same-thread subscriber receives `o`, regardless of whether the compiler lowers either send to a direct synchronous call or a queued delivery. No ordering is defined between publishers or subscribers executing on different threads.

### Subscriber Registration Order & Tree Reservations

A subscriber must be constructed before a publisher sends a message. In imperative bodies (`fn main()` or methods), instantiate subscribers before publishers (`hale check` warns on `"publish before its subscriber"` when both appear in the same body).

Within a **single locus literal's static tree**, the runtime handles registration order automatically:

- Before initializing any `params` field, the literal reserves all subscriptions across its entire subtree (including later sibling `params`, nested children, keyed subscribers, `pinned` fields, `cooperative(pool = ...)` fields, `replicas = K` shards, and wildcard subscribers).
- Messages published by an earlier sibling's `birth()` or `run()` are queued in order and delivered to later siblings as soon as each subscriber's `birth()` completes.
- Pre-birth reservation buffers respect each subscriber's queue bound: `bounded(N, drop_new)` retains the oldest `N` messages, `bounded(N, drop_old)` retains the newest `N`, `on_full: fail` drops excess arrivals and logs once to stderr, and unbounded subscribers buffer one queue capacity before shedding excess with a stderr warning. Set `LOTUS_BUS_LOG_RESERVE=1` to inspect reservation counts.
- At **shutdown**, the runtime joins `pinned` workers and drains their final published messages *before* dissolving sibling subscribers. Messages published by pool-placed tasks *after* their last subscriber has already dissolved are dropped (set `LOTUS_BUS_LOG_DROP=1` to log shutdown drops; use an explicit completion signal if all pool work must finish before teardown).

### Static Devirtualization (`dispatch_plan`)

When a closed-world local topic has quiet handlers on the same thread, the compiler elides the bus queue and lowers `Topic <- value;` into a direct synchronous method call:

| Lowering Flavor | Conditions | Mechanism |
|---|---|---|
| `static_direct` | Local unbound topic, same thread, quiet handler, flat scalar payload (`Int`, `Float`, `Bool`, `Decimal`, `Duration`). | Direct synchronous function call passing scalar fields. |
| `static_bucket` | Local unbound topic, same thread, quiet handler, payload contains `String`, `Bytes`, or nested structs. | Direct synchronous call with arena scratch management. |
| `dynamic` | Cross-thread delivery, topic bound in `bindings { }`, or non-quiet handler. | Full runtime bus queue / ring dispatch. |

Inspect the compiler's lowering decisions for every subject with:

```sh
hale model dump app.hl   # see the `dispatch_plan` section
```

The `dispatch_plan` is recorded in the build identity so deterministic recordings are only replayed against builds with identical dispatch lowering.

## Backpressure & Queue Bounds

By default, topic queues grow dynamically. For high-rate producers or slow consumers, you can bound capacity on the **topic** (publisher backpressure) or on individual **subscriptions** (consumer shedding):

| Bound Location | Syntax | Behavior at Capacity |
|---|---|---|
| **Topic declaration** (Publisher contract) | `bounded(1024); on_full: fail;` | Broker rejects the send when full; every publish site must handle the failure with `or raise`, `or discard`, or `or wait`. |
| **Subscribe clause** (Consumer shedding) | `bounded(64, drop_old)` | Silently drops the oldest queued message to keep the newest 64 (ideal for latest-state snapshots). |
| **Subscribe clause** (Consumer shedding) | `bounded(64, drop_new)` | Silently drops newly arriving messages until queue space opens. |

```hale,fragment
topic Frame {
    payload: Reading;
    bounded(1024);
    on_full: fail;
}
```

```hale,fragment
subscribe Frame as on_frame bounded(64, drop_old);
```

A subscriber bound smaller than the topic bound sheds excess messages locally before the topic bound is reached.

> [!NOTE]
> Subscriber-level `bounded(...)` clauses apply only to cooperative subscribers on `main`. Subscribers placed on a non-`main` `cooperative(pool = ...)` or `pinned` thread (and children nested under them) already receive messages through fixed-size inter-thread rings that apply backpressure directly to producers.

## Keyed Routing (`keyed_by`)

By default, a topic broadcasts every message to every subscriber. When managing per-entity or sharded subscribers (such as one locus per connection, symbol, or tenant), declare `keyed_by <field>` on the topic and `where key == <expr>` on the subscription so the runtime routes matching messages directly:

```hale
type Tick { symbol_id: Int; price: Decimal; }

topic Quote { payload: Tick; keyed_by symbol_id; }

locus Feed {
    params { symbol_id: Int = 0; }

    bus {
        subscribe Quote as on_quote where key == self.symbol_id;
    }

    fn on_quote(t: Tick) {
        // receives only ticks where t.symbol_id == self.symbol_id
    }
}
```

### Routing Key Rules

- **Supported key types**: The `keyed_by` field must exist on the payload struct and have type `Int`, `Bool`, `Time`, `Duration`, a payload-less `enum`, `Decimal`, or `String`. (`String` keys use a hash gate so non-matching keys cost a single integer comparison; for compound keys, pack values into a `Decimal` or format a composite `String` key.)
- **Filter expressions (`where key == EXPR`)**: `EXPR` may be a literal, a `const`, `self.<field>`, or the keyword `replica` (the instance's 0-based replica index when using `pinned(..., replicas = K)`). Applying a `where` filter to an unkeyed topic is a compile error.
- **Captured at construction before `birth()`**: The runtime reads `self.<field>` when registering the subscription *before* `birth()` executes. Always pass the key in the locus literal (`Feed { symbol_id: 7 }`). Mutating `self.symbol_id` inside `birth()` or during `run()` does not re-target an existing subscription; to switch keys, dissolve the locus and construct a new instance.

### Unmatched Key Policies (`on_unmatched`)

Configure how a keyed topic handles messages whose key matches no active subscriber using `on_unmatched:`:

| Policy | Syntax | Behavior when No Subscriber Matches |
|---|---|---|
| **Swallow** *(default)* | `on_unmatched: swallow;` | Drops the message silently. Set `LOTUS_BUS_LOG_UNMATCHED=1` to log unmatched drops during debugging. |
| **Fail** | `on_unmatched: fail;` | Makes publishing fallible; every send site must specify `Topic <- msg or raise;` or `Topic <- msg or discard;`. |
| **Fallback** | `on_unmatched: fallback;` | Delivers unmatched messages to catch-all subscribers registered with `where key == _`. Requires at least one catch-all subscriber in the program. |

```hale
type Tick { symbol_id: Int; price: Decimal; }

topic Quote { payload: Tick; keyed_by symbol_id; on_unmatched: fallback; }

locus Unrouted {
    bus { subscribe Quote as on_stray where key == _; }
    fn on_stray(t: Tick) { }
}
```

Next: [Concurrency & Placement](./concurrency.md).
