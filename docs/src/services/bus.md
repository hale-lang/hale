# The bus

> **Coming from Go?** Topics are like channels, but typed by a
> declaration instead of by `chan T`, and many-to-many instead of
> point-to-point. You don't pass a channel into a goroutine; a
> locus declares which topics it subscribes to and publishes, and
> the runtime wires the delivery. No channel plumbing threaded
> through constructors.

You met the bus implicitly in [logging](../everyday/logging.md):
emitters publish, sinks subscribe, neither references the other.
Here you declare and use it directly.

## Topics are typed declarations

A topic names a channel and the type that flows on it:

```hale
type Order { id: String; amount: Decimal; }

topic OrderPlaced  { payload: Order; }
topic OrderShipped { payload: Order; }
```

A topic is a top-level declaration, like `type` or `locus`. It's
referenced by name — never a magic string — so the payload type
is checked at every publish and every handler, and renaming the
topic moves every use with it.

## Subscribe and publish

A locus declares its bus interface in a `bus { }` block:

```hale
locus Warehouse {
    bus {
        subscribe OrderPlaced as on_order;     // inbound
        publish   OrderShipped;                 // outbound
    }

    fn on_order(o: Order) {
        // ... pick and pack ...
        OrderShipped <- o;                       // the send
    }
}
```

- **`subscribe TOPIC as HANDLER;`** wires inbound messages to a
  handler method. The handler must exist with the matching
  signature — `fn on_order(o: Order)` — and the compiler checks
  it.
- **`publish TOPIC;`** authorizes this locus to send on the
  topic. Without it, a send is a compile error.
- **`TOPIC <- value;`** is the send. It's a statement, not an
  expression — it produces no value, like Erlang's `Pid ! Msg`.

Subscribing is declarative — there's no `subscribe()` call at
runtime. Registration happens when the locus is constructed, and
unsubscribe happens automatically at dissolve.

## One ordering rule

A subscriber must be *born before* a publisher sends, or the
message has nowhere to land. In practice: instantiate your
subscribers first in `main`. (This is the same rule you saw with
the log sink.)

## Why this doesn't break the tower

In the [parent/child model](./parents-children.md), flow is
strictly vertical — a locus only talks up to its parent and down
to its children. The bus seems to let unrelated loci talk
sideways. It doesn't, really: publishers and subscribers don't
see *each other*, they see the *topic*, which lives at the
runtime root — structurally above everyone. Every send goes up to
the bus; every delivery comes down to a subscriber. It's vertical
flow through a shared root, which is why two loci on opposite
branches of a deep tree can coordinate with no shared pointer and
no registry lookup.

This is the productive shape for events: many-to-many flow
without back-channels. A topic can have any number of publishers
and subscribers.

## You won't always pay for it

If a topic is only ever used *inside a single locus type* — the
same locus both publishes and subscribes, with no external
binding — the compiler can prove every send routes back to a
handler on the same instance, and rewrites the send into a direct
method call. The bus is elided entirely. So you can use topics
freely for a locus's own internal event flow without paying
dispatch cost; if the topic later grows a second subscriber or a
deployment binding, the real bus path comes back automatically,
and your code doesn't change.

Next: where loci actually run — [Concurrency &
placement](./concurrency.md).
