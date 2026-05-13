# Private response streams

A carve-out from the broadcast default (see `broadcast-snapshot.md`).
Use when one of three conditions makes broadcast unworkable; layer
on top of the broadcast pattern rather than replacing it.

## The shape

Instead of every observer receiving every response, each observer
gets its own private response subject. The memory-owner publishes
to the per-recipient subject when serving that recipient's request.

Subject families:

- `<owner>.request` — shared inbox. Many clients publish; the
  owner subscribes once.
- `<owner>.response.<client_id>` (or
  `<owner>.response.<correlation_id>`) — per-recipient outbox.
  The owner publishes here when serving the request that named
  this id. Each client subscribes only to its own id-suffixed
  subject.

The owner declares the response family with an m94 wildcard:

```aperio
bus {
    publish "<owner>.response.**" of type ResponseMsg;
}
```

User-code routing then targets a specific suffix at publish time.

## When the carve-out applies

Three reasons broadcast is wrong and private streams are right:

### 1 — Privacy / authorization

Observer X must not see observer Y's data. Per-client suffixes
enforce isolation: even if X subscribed to the wildcard pattern,
the runtime's subject-match check rejects it (or, at v1.x, an
auth layer rejects the subscription registration).

> **Example.** A trading system serving order-flow per customer.
> Customer A's fills must not appear on customer B's bus.

### 2 — Volume

Broadcasting everything to everyone is too expensive at scale.
Per-recipient streams let the bus router filter at registration
time; subscribers only receive their own slot's traffic.

> **Example.** A market-data fan-out serving 10,000 subscribers
> at 100,000 events/sec. Broadcasting 10^9 events/sec to every
> subscriber wastes 99.99% of the work.

### 3 — Per-client owner state

The owner needs to maintain per-observer state — where the observer
is in the stream, what it has acknowledged, what its filter
preferences are. Then it *has* a per-client concern, and the
per-client subject family is the honest representation.

> **Example.** A streaming subscription service tracking each
> subscriber's playback cursor. The owner's state is a
> `heap subscribers of Subscriber` slot; each entry maps client
> id to its cursor position.

## Why this is a carve-out, not the default

Private streams cost more than broadcast on every axis except the
three above:

- **More subjects.** Each client adds a subject to the bus router;
  registration / dispatch overhead grows linearly.
- **More state at the owner.** The owner must track client ids
  (where do they come from? when are they collected? what happens
  on client disconnect?). Lifecycle becomes a real concern.
- **More protocol.** Clients must announce their id before the
  owner can route to them. The announcement is itself state the
  owner must maintain.

Default to broadcast. Reach for private streams only when one of
the three conditions makes broadcast genuinely wrong.

## Worked sketch

A streaming service with per-client cursors:

```aperio
type Subscriber {
    id: String;
    cursor: Int;
}

locus StreamerL {
    params {
        client_count: Int = 0;
    }

    capacity {
        heap subscribers of Subscriber;
    }

    bus {
        publish "stream.response.**" of type StreamMsg;

        /// ingest: save — appends client to subscribers heap
        subscribe "stream.subscribe" as on_subscribe of type SubscribeReq;

        /// ingest: transform — serves request, publishes per-client
        subscribe "stream.request"   as on_request   of type StreamReq;
    }

    fn on_subscribe(r: SubscribeReq) {
        let cell = self.subscribers.alloc();
        cell.id = r.client_id;
        cell.cursor = 0;
        self.client_count = self.client_count + 1;
    }

    fn on_request(r: StreamReq) {
        // serve r.client_id by publishing on its private subject
        // (subject computed from r.client_id; m94 wildcard surface
        // authorized publication on stream.response.**)
        // ...
    }
}
```

`StreamerL` is a recording memory-owner for the subscribers heap
(saves verbatim) AND a projection memory-owner for the cursor
state (transforms requests into cursor advances). The blended
shape is one of the cases where save and transform mix within one
locus.

## Combining with broadcast

Most real systems use both:

- Broadcast for the bulk data plane: `<concern>.delta`,
  `<concern>.snapshot.*`. Every observer follows.
- Private streams for per-client query/response or auth-gated
  surfaces: `<concern>.response.<client_id>`. Each observer
  subscribes only to its own slot.

The two patterns coexist on one bus router without conflict, since
the bus dispatches by subject and the subject namespaces don't
overlap.

## Cross-references

- `broadcast-snapshot.md` — the default pattern this is a
  carve-out from
- `properties.md` — the four MOA properties; private streams
  preserve all four if the per-client concern is genuine
- `moa/subjects.md` — subject naming conventions, including the
  m94 wildcard surface that private streams depend on
