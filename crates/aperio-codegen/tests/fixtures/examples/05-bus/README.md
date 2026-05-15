# 05-bus

Typed pub-sub on a transport-agnostic bus. An echo service:
subscribes to `demo.greeting`, publishes `demo.ack` in response.

```
type Greeting        { text: string; sender: string; }
type Acknowledgment  { received: string; }

locus EchoL {
    bus {
        subscribe "demo.greeting" as on_greeting of type Greeting;
        publish   "demo.ack"               of type Acknowledgment;
    }

    fn on_greeting(g: Greeting) {
        println("got: ", g.text, " from ", g.sender);
        publish("demo.ack", Acknowledgment { received: g.text });
    }
}

fn main() {
    EchoL { };
}
```

Plus `deployment.yaml` mapping subjects to transports.

## What runs

1. Process starts. Runtime reads `deployment.yaml`. For each
   declared bus channel, the runtime instantiates the
   appropriate transport adapter (`std::bus::in_memory`,
   `std::bus::nats`, etc.) and registers the channel.
2. `main()` invoked.
3. `EchoL { }` instantiates as anonymous child of `main`'s
   implicit locus.
4. EchoL's bus subscription wires up: the runtime registers
   `on_greeting` as the handler for `demo.greeting` on the
   bound transport.
5. EchoL's `birth()` runs (default, no-op).
6. EchoL has a bus subscription, so it's long-lived (per the
   updated §A rule). `main`'s implicit locus has one
   long-lived anonymous child.
7. Inbound `demo.greeting` messages on the bound transport
   trigger `on_greeting(g)`. Each call prints the greeting and
   publishes `demo.ack`.
8. SIGINT triggers `drain()` on the runtime root. Cascade
   reaches `main`'s implicit locus → reaches EchoL.
9. EchoL's drain unbinds bus subscriptions (no new messages
   accepted; in-flight handlers complete). Then dissolve.
10. `main()` returns. Process exits.

## Transport binding

Same source, different transport per channel — picked at
deployment time, not at compile time.

```yaml
# deployment.yaml
channels:
  "demo.*":
    transport: in_memory
```

For production, swap to NATS or UDP multicast or whatever fits
the channel's parameter envelope:

```yaml
"demo.greeting":
  transport: nats
  url: "nats://localhost:4222"

# or
"demo.greeting":
  transport: udp_multicast
  group: "239.1.2.3"
```

The runtime's bus router (per `runtime.md`) maintains per-channel
transport bindings; messages flow through whatever transport the
deployment selected. The locus source doesn't change.

## Primitives this exercises (new vs. 04)

- **`type Foo { ... }` declarations** — struct types for typed
  bus payloads. `Greeting` and `Acknowledgment` are plain
  structs with named fields.
- **`bus { subscribe ... ; publish ... ; }` block** — declares
  the locus's bus interface. Subscribes wire incoming messages
  to handlers; publishes declare outbound subjects + types.
- **`subscribe SUBJECT as HANDLER of type T`** — handler is a
  named member function on the locus, takes one arg of type
  T, returns nothing (or a typed value if multi-publish is
  needed; v0 uses explicit `publish(...)`).
- **`publish SUBJECT of type T`** — declares the locus may
  emit messages of type T on SUBJECT. Compiler verifies all
  `publish(SUBJECT, msg)` calls have matching type.
- **`publish(subject, msg)` builtin** — runtime function for
  emitting a message. In scope inside a locus that has a
  matching `bus { publish ... ; }` declaration; out of scope
  otherwise.
- **Long-lived unbound locus via bus subscription.** EchoL has
  no `run()` but does have a bus subscription. The updated §A
  rule extends the unbound-locus lifetime to "while
  subscriptions are active" for any locus with bus interfaces.
- **Transport binding via deployment config.** Source declares
  subjects; deployment maps subjects to transports.

## What writing this surfaced (for the spec)

Three things, all resolved in this commit:

1. **Long-lived unbound locus rule extended.** v0.1.2's §A
   covered (a) unbound + only birth = dissolves at statement
   boundary, and (b) unbound + has run = anonymous child of
   enclosing scope. v0.1.6 extends: (c) unbound + has bus
   subscriptions = anonymous child of enclosing scope.
   Generalization: any locus that *can do work after birth*
   (run, mode invocations from outside, bus subscriptions) is
   long-lived if unbound. A locus that's purely ephemeral
   (only birth + params) dissolves at statement boundary.
   Updated §A.

2. **`publish` runtime builtin.** The publish function is
   in scope inside a locus that declares `publish ...` in its
   bus block. The compiler verifies the subject and type at
   each call site against the declarations. Documented in
   tokens.md "Built-in identifiers" and design-rationale §F.12.

3. **Bus subscription handler signature.** A handler named in
   a `subscribe ... as HANDLER` declaration is a fn on the
   locus body taking one argument of the subscribed type. v0
   handlers return nothing (`-> ()`); explicit publishes
   inside the body emit responses. Future versions may permit
   `-> T` for auto-publish on a single response channel.
   Documented in §F.13.

## What this still does *not* exercise

- `bus subscribe ... as fn` with explicit return-type-as-publish
  (`fn on_x(x) -> Y`). Reserved for later.
- Closure tests over bus message rates / counts (e.g.,
  "messages received ~~ messages handled within 0"). Natural
  follow-up for fitter/applier.
- Multi-channel coordination (one locus subscribing to N
  channels, publishing on M).
- Error handling on bus dispatch (handler panic, transport
  failure).
- Request-response semantics (some transports support, some
  don't; the TransportEnvelope tracks this).

## Open question 6 finally resolved

`notes/open-questions.md` Q6 ("What happens to in-flight
messages on dissolve?") gets its answer here: drain phase
delivers in-flight messages before any new messages are
accepted; dissolve phase discards anything still queued.
SIGINT triggers drain on root → cascade → EchoL drains: stops
accepting new inbound; lets in-flight handler invocations
finish; then dissolves.

## Next on the ladder

`fitter-applier-pair` — analyst and executor binaries on a shared
schema, communicating via UDP multicast. The full first
program. Exercises everything we've built so far plus
perspective serialization, the analyst↔executor cyclic-closure
check, multi-binary deployment.
