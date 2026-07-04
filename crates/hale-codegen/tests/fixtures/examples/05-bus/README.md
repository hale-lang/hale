# 05-bus

Typed pub-sub on a transport-agnostic bus. Three loci wired
through an in-memory router, self-driving from birth: `SenderL`
publishes a `Greeting` on `demo.greeting` at birth; `EchoL`
subscribes to `demo.greeting`, prints it, and sends an
`Acknowledgment` on `demo.ack`; `AckLogL` subscribes to
`demo.ack` and logs it. No external transport feeds the program.

```
type Greeting        { text: String; sender: String; }
type Acknowledgment  { received: String; }

locus EchoL {
    bus {
        subscribe "demo.greeting" as on_greeting of type Greeting;
        publish   "demo.ack"               of type Acknowledgment;
    }

    fn on_greeting(g: Greeting) {
        println("got: ", g.text, " from ", g.sender);
        "demo.ack" <- Acknowledgment { received: g.text };
    }
}

locus AckLogL {
    bus {
        subscribe "demo.ack" as on_ack of type Acknowledgment;
    }

    fn on_ack(a: Acknowledgment) {
        println("ack: ", a.received);
    }
}

locus SenderL {
    bus {
        publish "demo.greeting" of type Greeting;
    }

    birth() {
        "demo.greeting" <- Greeting { text: "hello", sender: "sender-1" };
    }
}

fn main() {
    EchoL { };
    AckLogL { };
    SenderL { };  // born last, so the subscribers are ready
}
```

Plus `deployment.yaml` mapping subjects to transports.

## What runs

1. Process starts. For each declared bus channel, the runtime
   binds the appropriate transport adapter (in-memory router
   for v0; `nats` / `udp_multicast` in production) and
   registers the channel.
2. `main()` invoked. `EchoL`, `AckLogL`, `SenderL` instantiate
   in that order as anonymous children of `main`'s implicit
   locus.
3. `EchoL` and `AckLogL` register their subscriptions at birth:
   `on_greeting` for `demo.greeting`, `on_ack` for `demo.ack`.
   Order matters in v0 — the subscribers must be born before
   `SenderL` so they're ready when its `birth()` publishes.
4. `SenderL` is born last. Its `birth()` sends
   `"demo.greeting" <- Greeting { text: "hello", sender:
   "sender-1" }`.
5. The router delivers to `EchoL.on_greeting`, which prints
   `got: hello from sender-1` and sends `"demo.ack" <-
   Acknowledgment { received: "hello" }`.
6. The router delivers to `AckLogL.on_ack`, which prints
   `ack: hello`.
7. `main` has no `run()` body, so it returns immediately after
   instantiation. In production the SIGINT-triggered drain
   cascade (per F.4) would dissolve the tree; v0 just exits.

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
  needed; v0 uses explicit `<-` sends).
- **`publish SUBJECT of type T`** — declares the locus may
  emit messages of type T on SUBJECT. Compiler verifies all
  `SUBJECT <- msg` sends have matching type.
- **`SUBJECT <- msg` send operator** — emits a message on the
  subject. In scope inside a locus that has a matching
  `bus { publish ... ; }` declaration; a compile error
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

2. **`<-` send operator.** A `SUBJECT <- msg` send is in scope
   inside a locus that declares `publish ...` in its bus block.
   The compiler verifies the subject and type at each send site
   against the declarations. Documented in tokens.md and
   design-rationale §F.12.

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
