# Run a topic across binaries

[The bus](../concepts/the-bus.md) introduces topics as
deployment-bindable channels: the same `subscribe` /
`publish` code works whether the topic is delivered
in-process or over a transport. This recipe walks through the
actual mechanics — declaring topics in a shared seed, wiring
transports in `main locus { bindings { ... } }`, and running
two binaries that exchange messages.

## What ships

- **Absence of binding** — same-binary cooperative queue. The
  default when a topic isn't listed in `bindings { }`. There's
  no `in_memory` keyword; the default *is* the absence.
- **`unix("/path")` or `unix("/path", role: listen|connect)`** —
  AF_UNIX framed-byte transport. Substrate-provided. When
  `role:` is omitted, the typechecker infers it from the bus
  block (publish-only → connect; subscribe-only → listen).
- **`MyAdapter { ... }`** — user-supplied protocol-layer
  adapter (any locus satisfying `__StdBusAdapter`). Covers
  NATS, MQTT, TCP-with-framing, custom JSON-over-WebSocket —
  anything the substrate doesn't ship directly.

Unix sockets are the v1 substrate cross-process transport.
For protocol-layer transports, see the adapter section at the
bottom and [the bus concept page](../concepts/the-bus.md) for
the contract.

## Layout

We'll wire two binaries that share a `Tick` topic. The
publisher publishes ticks; the subscriber prints them. The
shared topic + payload type live in a third seed both binaries
import.

```
beats/                       ← workspace root (cd here to build)
  shared/
    topics.hl                ← topic Tick { payload: TickPayload; }
  publisher/
    main.hl                  ← import "shared" as shared; main locus
  subscriber/
    main.hl                  ← import "shared" as shared; main locus
```

## The shared seed

```hale
// beats/shared/topics.hl
type TickPayload { n: Int; label: String; }
topic Tick { payload: TickPayload; }
```

That's it. The topic decl is one place; both binaries see the
same wire shape because they compile from the same source.

**Type identity is path-based, not alias-based.** Either binary
could `import "shared" as s;` or `import "shared" as topics;` —
the mangler keys off the lib's canonical path (workspace-root-
relative), not the importer's alias. Both binaries see the same
internal symbol for `TickPayload` regardless of how each
imports it. That's what makes the shared-DTO pattern work
naturally — wire bytes match because layouts match, AND the
in-language type identity matches across consumers.

## The publisher

```hale
// beats/publisher/main.hl
import "shared" as shared;

locus Producer {
    bus { publish shared::Tick; }
    run() {
        let mut i = 1;
        while i <= 5 {
            shared::Tick <- shared::TickPayload {
                n: i,
                label: "pub"
            };
            std::time::sleep(100ms);
            i = i + 1;
        }
    }
}

main locus PublisherApp {
    bindings { Tick: unix("/tmp/beats.sock"); }
    run() {
        Producer { };
    }
}
```

The `bindings` block is **only legal in a `main`-modified
locus** (`main locus PublisherApp`, not bare `locus
PublisherApp`). A non-main locus carrying `bindings { }` is a
parse error.

Role is inferred from the bus block: this binary only
*publishes* `Tick`, so the role resolves to `connect` (the
write-side). No `role:` kwarg needed.

## The subscriber

```hale
// beats/subscriber/main.hl
import "shared" as shared;

locus Consumer {
    bus { subscribe shared::Tick as on_tick; }
    fn on_tick(t: shared::TickPayload) {
        println("got tick #", t.n, " from ", t.label);
    }
}

main locus SubscriberApp {
    bindings { Tick: unix("/tmp/beats.sock"); }
    run() {
        Consumer { };
        std::time::sleep(2000ms);   // keep alive long enough to receive
    }
}
```

This binary only *subscribes* `Tick`, so the role resolves to
`listen` (the server side). At `main`'s prelude, the runtime
binds the AF_UNIX socket and spawns a reader thread; inbound
payloads flow into the locus's normal handler dispatch.

If a binary both publishes AND subscribes the same topic, role
inference can't pick — specify it explicitly:

```hale
bindings { Tick: unix("/tmp/beats.sock", role: listen); }
```

## Build and run

From `beats/`:

```sh
hale build subscriber/
hale build publisher/

./subscriber/subscriber &       # start the listener first
sleep 0.1                       # give it a moment to bind the socket

./publisher/publisher
```

Expected output (from the subscriber):

```
got tick #1 from pub
got tick #2 from pub
got tick #3 from pub
got tick #4 from pub
got tick #5 from pub
```

## Bundle-wide rules

The compiler enforces:

1. **At most one `main` locus per bundle.** Zero is fine (a
   classic `fn main()` shape is still legal); two main loci
   is a compile error.
2. **Each `bindings` entry's topic must be declared.** A
   binding for an undeclared topic name is a compile error.
3. **A topic may appear at most once across all bindings.**
4. **`bindings` only inside a `main`-modified locus.** Any
   other location is a parse error.

## What `unix(...)` actually wires

At program startup, `main`'s prelude calls
`lotus_bus_register_remote(subject, "unix:///tmp/beats.sock", role)`.
The C runtime:

- **`listen` side** — `bind()`s the socket, spawns a pthread
  reader, fans incoming framed payloads into the local
  subscriber set via `lotus_bus_local_dispatch`.
- **`connect` side** — opens a write fd lazily on first
  publish, sends length-prefixed frames.

Framing is length-prefix per topic (the bus transport
contract is "deliver one whole message" regardless of
transport). You don't see the frames — the substrate handles
them.

## UDP transport (unicast + multicast)

The substrate also ships a `udp://host:port` transport for the
**runtime** side (the `LOTUS_BUS_CONFIG` env-var path; the
source-level `bindings { Topic: udp(...); }` syntax is a
separate follow-up). Both unicast and multicast IPv4 ride the
same scheme — the address class picks the mode:

```ini
# LOTUS_BUS_CONFIG file format
Tick = udp://127.0.0.1:5000:listen        # unicast subscribe
Tick = udp://192.168.1.5:5000:connect     # unicast publish
Book = udp://239.255.77.77:5000:listen    # multicast subscribe
Book = udp://239.255.77.77:5000:connect   # multicast publish
```

Multicast addresses (`224.0.0.0/4` — `224.0.0.0` through
`239.255.255.255`) trigger `IP_ADD_MEMBERSHIP` on the listen
side and rely on the kernel's multicast routing tree on the
connect side. Unicast addresses just `bind` + `sendto`.
Publishers use `sendto` identically in both modes; the
distinction lives entirely in the subscriber-side setup.

**Lossy delivery.** UDP transports give publishers the same
"sendto returned" durability the other transports do — the
substrate's contract to the publisher is "I have your
message, you're done." Subscriber-side gap recovery is a
deployment concern, not a runtime one: apps that need
gap-free multicast wire a repeater between the feed and
the subscription (the MoldUDP shape NASDAQ ITCH uses).
Workloads where occasional loss is acceptable (telemetry
fan-out, level-2 book streams) skip the repeater.

The UDP transport spawns the same per-subject reader-thread
shape as `unix://` — recvfrom loop → deserialize → local
dispatch. At program exit, `shutdown(SHUT_RDWR)` unblocks the
reader thread, then `pthread_join`, then `close` releases
the fd. The Linux 3.9+ `SO_REUSEPORT` is set on the
subscriber socket so multiple processes on the same host can
receive the same multicast group.

## When the topic is also bound, the closed-world opt is skipped

The compiler normally rewrites a topic that's used only
within one locus type into a direct method call (the
"closed-world optimization"). A bound topic is never
optimized — the binding may publish to remote subscribers
the compiler can't see. This means adding a binding is
*always* a real bus traversal; the optimization quietly
stops applying.

## Mixing in-process and remote subscribers

A topic can have a binding **and** in-process subscribers.
Inbound payloads from the socket and locally-published
payloads fan out to the same handler set:

```hale
main locus App {
    bindings { Tick: unix("/tmp/beats.sock"); }
    run() {
        Consumer { };                  // local subscriber
        OtherConsumer { };             // another local subscriber
        // Remote publishers writing to /tmp/beats.sock also reach both.
    }
}
```

## What about TCP, NATS, MQTT, etc.?

The substrate doesn't ship them directly — that's the
adapter path. Protocol-layer transports are user loci that
satisfy `__StdBusAdapter`:

```hale
locus MyNatsAdapter {
    params { url: String = "nats://localhost:4222"; }
    birth() { /* open connection */ }
    fn send(subject: String, bytes: Bytes) {
        /* publish via your protocol */
    }
    run() {
        /* recv loop on the adapter's own thread.
         * For each inbound message, call
         * std::bus::__local_dispatch(subject, bytes);
         * the runtime looks up the subject's deserializer and
         * fans into local subscribers. */
    }
    dissolve() { /* close connection */ }
}

main locus App {
    bindings { Tick: MyNatsAdapter { url: "nats://prod:4222" }; }
}
```

Adapters instantiated inline in a `bindings { }` entry get
their own OS thread implicitly — they're not a main-locus
`params` field, so they don't appear in `placement { }`, but
their run-loops need a dedicated thread by construction. The
substrate places them pinned-equivalent regardless of any
explicit placement.

The application-code shape (publishers, subscribers, handlers)
is identical to the `unix(...)` case. Only the binding line
changes. Reliability semantics, retry, ordering, and
backpressure are the adapter's concern — the substrate stays
neutral on protocol choice because NATS, MQTT, and AMQP
genuinely disagree on those points.

See [The bus → Writing your own adapter](../concepts/the-bus.md#writing-your-own-adapter)
for the full contract.

## See also

- [The bus](../concepts/the-bus.md) — concept-level treatment
  of topics, hierarchical subjects, and the vertical-flow
  reconciliation.
- [Project layout](./project-layout.md) — cross-seed imports
  and the `hale build` flow.
- [Structured logging](./logging.md) — `log.**` is a
  hierarchical topic that can be bound the same way to ship
  log events to a centralized aggregator binary.
