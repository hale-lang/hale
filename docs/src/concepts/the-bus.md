# The bus

> How do loci communicate without referring to each
> other by name?

The bus is Hale's typed pub/sub channel: the way two loci
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
Hale uses **typed topic declarations**:

```hale
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

```hale
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

```hale
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

```hale
locus MyAdapter {
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
        // recv loop on the adapter's dedicated thread.
        // For inbound, call
        // std::bus::__local_dispatch(subject, bytes) once
        // you've received and identified a payload.
    }
    dissolve() {
        // close the connection
    }
}

main locus App {
    bindings {
        Tick: MyAdapter { url: "nats://prod:4222" };
    }
}
```

Adapters instantiated inline in a `bindings { }` entry get
their own OS thread implicitly — the substrate places them
pinned-equivalent so `run()` can block on a recv loop without
holding up any cooperative pool. They are not main-locus
`params` fields and so don't appear in `placement { }`;
F.31's `placement { }` block governs only the placement of
main-locus `params` field loci. `send` runs synchronously in
the publisher's thread; `__local_dispatch` looks up the
subject's serializer to convert wire-bytes into the in-memory
payload shape, then fans into local subscribers.

The substrate stays neutral on protocol semantics — reliability
guarantees, ordering, retries, backpressure, fan-in policy all
live in the adapter body where they belong. NATS-at-most-once
and MQTT-QoS-2 and a custom broker with transactional ack all
satisfy the same `__StdBusAdapter` contract.

### Shared-memory zero-copy delivery (`shm_ring`)

For high-frequency same-machine routes where the memcpy cost
shows up in the latency budget — real-time market data, tick
streams, anything north of ~100k msg/s on one host — a binding
can declare `shm_ring` as the transport and assert `zero_copy`
in a `where` clause:

```hale
bindings {
    L2Updates: shm_ring("/l2-updates",
                        slot_count:  1024,
                        on_overflow: fail)
              where intra_machine, zero_copy;
}
```

The publisher writes the payload directly into a POSIX
shared-memory ring buffer slot; the subscriber reads from the
same memory; no kernel memcpy at the locus boundary. Both
processes `mmap` the same `/dev/shm` object and coordinate via
the ring's slot indices.

The `where` clause is two things at once:

- **A user assertion** that the route satisfies the named
  constraints. The dev team is declaring "this should be intra-
  machine + zero-copy."
- **A typecheck contract** the substrate validates: the
  transport must support the constraint, the payload must be
  shape-compatible, and the scope keywords must be internally
  consistent.

Constraint vocabulary:

- **Scope** — `intra_process` (same OS process), `intra_machine`
  (cross-process, same host), `cross_machine` (network in
  scope). A binding declares at most one; mixing two is
  rejected as ambiguous.
- **Behavior** — `zero_copy` (no memcpy at locus boundary). The
  typechecker rejects `zero_copy` combined with `cross_machine`
  (network always serializes) and against transports that can't
  satisfy it (`unix(...)` rejects `zero_copy` — the kernel
  memcpies through the socket buffer; user-supplied adapters
  reject `zero_copy` — the `fn send(subject, bytes)` contract
  requires serialization).

`zero_copy` further requires the topic's **payload type to be
flat-shapeable**: every leaf is a fixed-layout primitive
(`Int`, `Float`, `Bool`, `Decimal`, `Time`, `Duration`), a
fixed-size array of flat-shapeables, or a struct whose fields
are all flat-shapeable. `String`, `Bytes`, `BytesView`,
`StringView`, and unbounded arrays carry heap pointers / length
prefixes that don't translate to a shared-memory slot, so the
typechecker rejects them on a zero-copy topic. Use a fixed-size
byte array (`[Byte; 256]` etc.) for bounded text/binary payloads
on these routes.

The back-pressure policy (`on_overflow:`) is required on every
`shm_ring` binding — slot exhaustion needs a policy decision
the substrate can't guess for you. Three policies:

- **`block`** — publisher spins until a slot frees. Right for
  control-plane topics where latency tolerates backpressure
  but data must not be lost. v1 has no timeout; deadlocks if
  the consumer dies.
- **`drop`** — publisher unconditionally overwrites the next
  slot. Slow consumers silently miss messages. Right for
  stale-is-worthless feeds (market data tickers, telemetry).
- **`fail`** — publisher panics with a clear stderr diagnostic
  and `_exit(1)` when the ring is full. Process-level
  visibility into back-pressure events.

Same pattern as F.35's `where async_io` on placement entries —
operational requirements declared at the deployment seam,
validated by the typechecker, consumed by codegen to pick the
lowering strategy. The locus body's `subscribe T as on_msg`
handler is the same line of source whether the message arrived
via shared-memory zero-copy or via a unix-socket memcpy.

## Hierarchical topics + wildcards

Topics can declare a parent and inherit a dotted wire-subject
hierarchy:

```hale
topic Events { payload: Event;   subject: "events"; }
topic Login  : Events { payload: Login;  subject: "login"; }
topic Logout : Events { payload: Logout; subject: "logout"; }
```

`Login`'s wire subject is `"events.login"`. `Logout` is
`"events.logout"`. The hierarchy is purely a *subject naming*
convention — each topic is still its own typed declaration.

Subscribers can use **`**` wildcards** to catch a whole subtree:

```hale
locus AuditLog {
    bus { subscribe "events.**" as on_event; }
    fn on_event(payload: Bytes) { /* log every event */ }
}
```

Where the literal-subject form (`"events.**"`) accepts any
matching topic by wire subject, the typed-topic form
(`subscribe Events as ...`) keeps the strict-type discipline.

## Routing keys + `on_unmatched`

A topic can declare a **routing key** — a field on the payload
type that selects which subscriber receives each message:

```hale
type CmdMsg { sym_id: Int; qty: Int; }

topic Cmd {
    payload: CmdMsg;
    subject: "cmd";
    keyed_by sym_id;
    on_unmatched: fail;
}

locus AaplWorker {
    params { my_sym_id: Int = 1; }   // 1 = AAPL by convention
    bus { subscribe Cmd as on_cmd where key == self.my_sym_id; }
    fn on_cmd(c: CmdMsg) { /* AAPL-only */ }
}
```

The `keyed_by sym_id` clause names the payload field the bus
inspects at dispatch time. Each subscriber adds a `where key
== EXPR` predicate; only the subscriber whose predicate
matches the message's key field receives the call. This is
the canonical alternative to *N* sibling subscribers each
filtering inside their handler body.

**Acceptable key types** at v0.1: `Bool`, `Int`, `Time`,
`Duration`, no-payload `enum`, and `Decimal`. All store as a
u64 (or u128 for `Decimal`) on the bus and compare in one or
two `i64` ops at dispatch. `String` and `Bytes` are not
admitted — variable-width keys don't fit the fixed-cost
dispatch model. Convention for string-keyed shapes: hash the
string to an `Int` at publish time, or assign sequential
integer ids at registration.

The `on_unmatched` clause picks what happens when no
subscriber's `where` predicate matches:

- **`swallow` (default)** — the message is silently dropped.
  Cheap, but the publisher has no visibility into misses.
- **`fail`** — the publish-side `<-` expression becomes
  fallible-required. Callers must address it with `or raise`
  (re-fail via the closure-violation channel) or `or discard`
  (suppress this miss specifically). Useful when "no
  subscriber" is a structural bug worth surfacing.
- **`fallback`** — the topic must include at least one
  catch-all subscriber written as `where key == _`. That
  subscriber receives every message whose key didn't match a
  more specific predicate.

See `spec/semantics.md § Phase 3: routing keys` for the full
rule set + the `BusUnmatchedKey` error type. The
[fallible-bus-adapters](../../spec/design-rationale.md#f33-fallible-user-supplied-bus-adapters-sketch)
sketch (F.33) extends the same `or` machinery to transport
failures on user-supplied adapters.

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

Most loci default to `cooperative(pool = main)` placement and
share the main OS thread. Bus dispatch between cooperative
subscribers on the same pool is a fast in-process enqueue.

A locus placed `pinned` (in `main`'s `placement { }` block)
owns its own OS thread. Bus traffic to or from a pinned locus
crosses a thread boundary via a lock-protected mailbox. With
multiple cooperative pools (`cooperative(pool = io)`), bus
traffic across pools follows the same condvar+memcpy
machinery. The semantics are identical from the user's view —
`Topic <- payload;` still works the same way — but the
substrate adapts. Placement is covered in
[Lifecycle & time](./lifecycle-time.md).

## Next

The next chapter, [Capacity & storage](./capacity-storage.md),
covers what *else* a locus can hold besides its `params` —
bounded storage slots, projection classes, and the form
library that gives you growable buffers, hashmaps, and ring
buffers without parametric collection types.
