# The bus

> **α** — How do loci communicate without referring to each
> other by name?

The bus is Aperio's typed pub/sub channel: the way two loci
that don't sit in a parent ↔ child relationship still
coordinate. It's not a library, not a `std::*` namespace — it's
a first-class language primitive with grammar and typecheck
support.

This chapter covers what a topic is, how subscribe / publish
fit into a locus body, how a topic that's purely in-process
by default can be wired to a network transport at *deployment*
time without changing any code, and the optimization the
compiler runs when a topic happens to be used only within one
locus.

## Topics are first-class

Where most actor / pub-sub systems use *string subjects*,
Aperio uses **typed topic declarations**:

```aperio
type Player    { id: String; name: String; }
type MatchInfo { match_id: String; players: [Player]; }

topic JoinQueue  { payload: Player; }
topic MatchReady { payload: MatchInfo; }
```

A topic names a channel. The `payload: T` field declares the
type that flows on it. Topics are top-level declarations, the
same shape as `type` or `locus`. They live in the program's
namespace and are referenced by name, not by string.

This buys you four things:

1. **Type-checking at the publish site.** `JoinQueue <- value`
   typechecks `value` against `Player` before any code runs.
   No "I forgot to update the subject when the payload
   changed" bugs.
2. **Type-checking at the handler.** A `subscribe JoinQueue
   as on_join` requires `fn on_join(p: Player)` somewhere on
   the locus body. Wrong type → diagnostic at the locus, not
   at runtime.
3. **Refactoring works.** Rename `JoinQueue` → `PlayerJoin` and
   every reference moves with it. Subject names aren't strings
   sprinkled across the codebase.
4. **No protocol drift.** Publisher and subscriber compile from
   the same source; the type is the contract.

## Subscribing and publishing

A locus declares its bus interface in a `bus` block:

```aperio
locus Matchmaker {
    capacity { heap waiting of Player; }
    bus {
        subscribe JoinQueue  as on_join;     // inbound
        publish   MatchReady;                 // outbound authorization
    }
    fn on_join(p: Player) {
        self.waiting.push(p);
        if self.waiting.len() >= 4 {
            MatchReady <- assemble_match(self.waiting);   // <- is the send
        }
    }
}
```

Three constructs:

- **`subscribe TOPIC as HANDLER;`** — wires inbound messages
  on `TOPIC` to the handler function `HANDLER` on this locus.
  The handler is a regular `fn` somewhere on the body with
  signature `fn HANDLER(payload: T)` where `T` is the topic's
  declared payload type.
- **`publish TOPIC;`** — authorizes this locus to emit on
  `TOPIC`. Without the declaration, a `<-` send to the topic
  is a typecheck error.
- **`TOPIC <- value;`** — the send. Statement-shape only;
  produces no value. The Erlang-shape (`Pid ! Msg`)
  one-directional send.

Subscribing is *declarative*. There's no `subscribe()` function
to call at runtime; the registration happens when the locus is
constructed, before `birth()` runs. Unsubscribing happens
automatically when the locus dissolves.

## Why this preserves vertical-only flow

You may notice the bus connects two loci that aren't parent
and child. Doesn't that break the vertical-only flow rule from
the previous chapter?

It doesn't, because publishers and subscribers don't actually
see each other. They see *the topic*. The topic is a
declaration at the runtime root — structurally above every
locus that participates. Every send goes *up* through the bus
router (which lives in the substrate); every dispatch comes
*down* into the subscriber. From any participant's view, the
bus is vertical flow through a shared root, not lateral flow
to a sibling.

This is the productive shape because it gives you many-to-many
event flow without back-channels. Two loci on opposite branches
of a deeply nested tree can coordinate by both referencing the
same topic — no shared pointer, no global registry, no name
lookup at runtime.

## Bindings — same topic, different transport

Here's where the bus pays for itself. The publisher and
subscriber in the matchmaker example look identical regardless
of whether the topic is delivered in-process, over a Unix
socket, or over a protocol-layer broker like NATS. The choice
of transport is a **deployment-time** decision made in one
place — the program's `main` locus:

```aperio
main locus App {
    bindings {
        // JoinQueue: absent — same-binary cooperative queue (default).
        MatchReady: unix("/tmp/matches.sock");        // AF_UNIX
        BrokerEvt:  MyNatsAdapter { url: "nats://..." };  // user adapter
    }
    run() {
        Matchmaker { target_size: 4 };
    }
}
```

The `bindings` block is only legal in a `main`-modified locus.
Each entry pairs a topic with a transport spec. Two shapes
ship — substrate and adapter — with absence-of-entry as a third
implicit option:

- **Absence of entry** — same-binary cooperative queue (the
  runtime default). The publisher's send enqueues on a queue
  that the subscriber drains at its next yield point. There's
  no `in_memory` keyword; the default *is* the absence.
- **`unix("/path")` or `unix("/path", role: listen|connect)`** —
  AF_UNIX framed-byte transport. Substrate-provided: the
  runtime's `lotus_transport_*` owns the delivery contract
  directly. When `role:` is omitted, the typechecker infers it
  from the bus block (`publish` only → `connect`, `subscribe`
  only → `listen`); if both publish and subscribe touch the
  topic in the same bundle, you specify `role:` explicitly.
- **`MyAdapter { ... }`** — a user-supplied locus that
  satisfies `__StdBusAdapter` (currently a single method,
  `fn send(subject: String, bytes: Bytes)`). Protocol-layer
  transports — NATS, MQTT, raw-TCP-with-framing, custom
  JSON-over-WebSocket — ship as ordinary loci in user code or
  downstream packages. The grammar tells the two apart by the
  head's case: lowercase keyword `unix` vs capitalized locus
  name.

The point isn't the transport list — it's that the **publisher
code and subscriber code don't change** when you flip the
binding. A locus that subscribes to `JoinQueue` doesn't know
whether the publisher is in the same process or on the other
side of a Unix socket or arriving via NATS. The deployment
seam is the only place that knows.

This is what makes the same locus code reusable across test
(in-memory), single-binary (in-memory), and multi-binary
(unix / adapter) deployments. The library writer doesn't
choose; the application writer does.

For the end-to-end mechanics — two binaries, a shared
topic, what the build invocations look like, what `unix(...)`
actually wires up — see
[Run a topic across binaries](../how-tos/multi-binary-bus.md).

### Writing your own adapter

For protocol-layer transports the substrate doesn't ship in
itself, you write the adapter as an ordinary locus. The
contract is a single method:

```aperio
locus MyAdapter : schedule pinned {
    params {
        url: String = "nats://localhost:4222";
    }
    birth() {
        // open the connection in your own state
    }
    fn send(subject: String, bytes: Bytes) {
        // ship one outbound payload via your protocol
    }
    run() {
        // pinned recv loop — for inbound, call
        // std::bus::__local_dispatch(subject, bytes) once
        // you've received and identified a payload
    }
    dissolve() {
        // close the connection
    }
}
```

The `: schedule pinned` annotation gives the adapter its own
pthread so `run()` can block on a recv loop without holding up
the cooperative scheduler. `send` runs synchronously in the
publisher's thread; `__local_dispatch` looks up the subject's
serializer to convert wire-bytes into the in-memory payload
shape, then fans into local subscribers.

The substrate stays neutral on protocol semantics — reliability
guarantees, ordering, retries, backpressure, fan-in policy all
live in the adapter body where they belong. NATS-at-most-once
and MQTT-QoS-2 and a custom broker with transactional ack all
satisfy the same `__StdBusAdapter` contract.

## Hierarchical topics + wildcards

Topics can declare a parent and inherit a dotted wire-subject
hierarchy:

```aperio
topic Events { payload: Event;   subject: "events"; }
topic Login  : Events { payload: Login;  subject: "login"; }
topic Logout : Events { payload: Logout; subject: "logout"; }
```

`Login`'s wire subject is `"events.login"`. `Logout` is
`"events.logout"`. The hierarchy is purely a *subject naming*
convention — each topic is still its own typed declaration.

Subscribers can use **`**` wildcards** to catch a whole subtree:

```aperio
locus AuditLog {
    bus { subscribe "events.**" as on_event; }
    fn on_event(payload: Bytes) { /* log every event */ }
}
```

Where the literal-subject form (`"events.**"`) accepts any
matching topic by wire subject, the typed-topic form
(`subscribe Events as ...`) keeps the strict-type discipline.

## The closed-world optimization

If a topic is only used inside one locus type — same locus
publishes and subscribes, no binding to an external transport —
the compiler can prove that every send necessarily routes back
to a handler on the same locus instance. In that case, the
desugar pass rewrites the `<-` send into a direct method call.
The bus is elided.

This means you can use topics freely for internal event flow
inside a complex locus *without* paying the bus dispatch cost.
When a workload later sprouts a second subscriber or gets a
deployment binding, the optimization stops applying
automatically and the bus path comes back. The user-visible
code doesn't change.

## Cross-thread bus semantics

Most loci default to `: schedule cooperative` and share a
single scheduler thread. Bus dispatch between cooperative
subscribers is a fast in-process enqueue.

A locus annotated `: schedule pinned` owns its own OS thread.
Bus traffic to or from a pinned locus crosses a thread
boundary via a lock-protected mailbox. The semantics are
identical from the user's view — `Topic <- payload;` still
works the same way — but the substrate adapts. Schedule
classes are covered in [Lifecycle & time](./lifecycle-time.md).

## Next

The next chapter, [Capacity & storage](./capacity-storage.md),
covers what *else* a locus can hold besides its `params` —
bounded storage slots, projection classes, and the form
library that gives you growable buffers, hashmaps, and ring
buffers without parametric collection types.
