# Cross-process: opening multiple lotuses

[Chapter 8](./08-scheduling-and-threads.md) introduced
multi-thread Aperio programs — pinned loci on separate OS
threads, cross-thread bus mailboxes, CPU-core affinity. This
chapter goes one level beyond: when even multiple threads in
one process is the wrong shape — when a workload needs separate
OS processes, possibly on separate machines — the same bus
extends transparently.

So far every Aperio program has lived in one binary, opening one
[lotus](../reference/glossary.md#lotus). This chapter covers
the multi-binary case: separate Aperio programs that share a bus
across processes, each opening its own lotus, communicating via
the same named subjects you already know from
[chapter 6](./06-the-bus.md).

The substrate-level claim is striking and worth stating upfront:
**source code does not change** when a bus subject moves from
in-process to cross-process. A `subscribe "chat.message"
of type Message` declaration in a locus reads identically
whether the publisher is another locus in the same binary,
another locus in a different binary on the same machine, or
another locus on a different machine entirely. What changes is
the *deployment* — which transport binds the subject and what
role each process plays.

## When you reach for it

Multiple binaries make sense when:

- **Different fault domains.** A `restart` in the analyst process
  must not restart the executor. They are separate operating-
  system processes, each opening its own lotus.
- **Different schedule regimes.** One process runs at high
  frequency in `bulk` mode; another runs slowly in `resolution`
  mode. Each process gets its own scheduling.
- **Different machines.** Geographic separation, redundancy,
  scale-out — the bus carries the same subjects across the
  network as it does across local pipes.

For everything else, one binary opening one lotus is the simpler
shape. Don't reach for cross-process unless you need it.

## Shared schema, separate binaries

The chat-fanout example (`examples/chat-fanout/`) is the
canonical multi-binary Aperio program. It contains three source
files:

```text
chat-fanout/
    server.ap          // the server binary's main
    client.ap          // the client binary's main
    shared.ap          // types compiled into both binaries
```

The shared file declares the types that travel on the bus:

```aperio
// shared.ap
type Message {
    sender: String;
    body: String;
    ts: Time;
}

type Session {
    user: String;
    joined_at: Time;
    message_count: Int;
}
```

Both binaries `import "chat-fanout/shared";`. The compiler emits
the same struct layout for `Message` and `Session` in each
binary's compiled output. **Schema agreement is by compilation,
not by runtime negotiation** — the wire format is exactly the
in-memory layout, and both sides agree because both sides compiled
the same source.

This is the substrate's stance on cross-process schemas: the
schema is the source. There is no Protobuf, no JSON Schema, no
OpenAPI document to keep in sync — the `type` declarations *are*
the schema.

## Building the binaries

Each binary's main file is built separately:

```bash
aperio build examples/chat-fanout/server.ap
aperio build examples/chat-fanout/client.ap
```

Two ELF binaries land alongside their source: `server` and
`client`. Each statically embeds the bundled
[lotus](../reference/glossary.md#lotus) C runtime, the
shared types, and its own loci. Neither binary needs the other to
run; they share state only through the bus at runtime.

## The cross-process bus configuration

For in-process bus, no configuration is needed — the runtime
includes an in-memory router by default. For cross-process,
each binary needs to know *which* subjects travel across the
boundary, *what* transport to use, and *which* end of the
connection it is.

In v0, this is provided via the `LOTUS_BUS_CONFIG` environment
variable, pointing at a small line-oriented config file:

```text
chat.message   = unix:///tmp/chat-message.sock   : listen
chat.broadcast = unix:///tmp/chat-broadcast.sock : connect
```

Each line has the shape:

```text
<subject> = <transport-url> : <role>
```

where:

- **`<subject>`** matches a bus subject the binary subscribes
  to or publishes on.
- **`<transport-url>`** names the transport. `unix://<path>` is
  the v0 surface; this chapter focuses on it.
- **`<role>`** is `listen` or `connect`.

### `listen` vs `connect`

The two roles are server vs client, in the conventional sense:

- A **`listen`**-role process binds the transport endpoint and
  accepts incoming connections from peers. For a Unix socket,
  this is `bind()` + `listen()` + `accept()` in a background
  thread.
- A **`connect`**-role process opens the transport endpoint as
  a client, with retry. If the listener is not yet up,
  `connect` retries until it is.

For a given subject, exactly one process should be in the
`listen` role; one or more processes can be in the `connect`
role. The `listen` side is the publisher when the subject's flow
is producer→consumer; the `connect` side is the publisher when
the flow is collector→aggregator. Either direction works — the
wire is bidirectional.

### Multi-peer fanout

A single config file may contain multiple `: connect` lines for
the same subject pointing at different listeners. The publisher
fans out: every `connect` peer receives a copy of every published
message on that subject.

```text
evt = unix:///tmp/peer-a.sock : connect
evt = unix:///tmp/peer-b.sock : connect
```

A locus publishing on `"evt"` with the above config delivers each
message to both peer-a and peer-b. (This was end-to-end-verified
in the m69 fanout test in `bus_subscriber.rs`.)

### Subjects without a config line

A subject the binary uses but does not list in
`LOTUS_BUS_CONFIG` falls back to in-process dispatch. This is
how the same source can run as one binary or several: in single-
binary mode, no config; the bus stays in-memory. In multi-binary
mode, only the subjects that cross processes need entries.

A binary started with no `LOTUS_BUS_CONFIG` at all behaves
identically to a single-process program — every subject is
in-memory.

## The wire format

When a subject crosses a process boundary, its payload must be
serialized to bytes on the publisher side and deserialized on
the subscriber side. The substrate's stance on wire format is
load-bearing and worth stating precisely:

> **Compile-time agreement, no runtime negotiation.** Both sides
> compiled the same `type` declarations from the same source.
> The wire format is fully determined by those declarations; no
> headers, no version bytes, no schema description travels on
> the wire.

The serialization rules (m70):

| Field type | Wire form |
|---|---|
| `Int`, `Float`, `Bool`, `Time`, `Duration` | 8 bytes, little-endian |
| `Decimal` | 16 bytes |
| `String` | 8-byte LE length prefix + UTF-8 bytes (no NUL) |
| Nested struct, enum, array as a field | not yet supported in v0; defer to post-v1 |

Field order on the wire is declaration order; there is no
padding, no field tags. A `Greeting { text: String, sender:
String }` is exactly two length-prefixed UTF-8 strings,
back-to-back.

A subscriber reading the wire allocates string bytes from a
**lazy global payload arena** (`lotus_bus_payload_arena_alloc`)
that lives for the life of the process. This arena hands the
deserialized strings to the subscriber's handler with the same
shape they would have had in-process. The handler does not see
any difference between an in-memory copy and a wire copy.

### Schema evolution

There is no on-the-wire versioning in v0. If you change a
`type`'s field set, both the publisher and subscriber binary
must be recompiled and redeployed together. This is intentional
— versioning is the topic of perspective evolution, deferred to
the post-v1 `serialize_as TypeV1` mechanism (open-question #13).

For v1, plan deployments such that producer and consumer binaries
that share a subject also share their build of the shared
schema.

## In-process vs cross-process: same shape

The single property worth stating again at the end:

> **The handler signature does not change.** A locus's
> `subscribe "evt" as on_evt of type Evt` declaration looks the
> same whether the publisher of `"evt"` is in the same binary
> or a different one. The handler runs on the subscriber's
> arena either way; the payload is a copy either way; the
> publisher's locus does not block waiting for delivery either
> way.

What changes between in-process and cross-process is the
substrate's bookkeeping — the wire serializer, the lazy payload
arena, the listen/connect socket pair. None of that is visible
to user code.

## What this chapter does not cover

- **NATS / TCP / UDP multicast transports** — listed in the
  `deployment.yaml` files in the example directory as the
  production transports. v0 implements `unix://` end-to-end;
  the others are scaffolded for production deployment but are
  not the focus of this book.
- **Higher-level config (`deployment.yaml`)** — the
  example-tree config files describe a richer YAML shape with
  glob patterns and per-transport options. v0 consumes the
  simpler `LOTUS_BUS_CONFIG` line format; the YAML form is the
  intended future surface.
- **Perspective versioning (`serialize_as TypeV1`)** — the
  schema-evolution mechanism mentioned briefly above. Lands
  with the perspectives chapter,
  [chapter 11](./11-perspectives.md), and is full v1.x roadmap
  work.

The next chapter, **[Generics](./10-generics.md)**, returns to
the source-level surface: how `Result<T, E>`, `Option<T>`, and
the substrate's `Numeric` bound let a single locus or function
work over a family of types without runtime type inspection.
