# Bus dispatch and routing

## Synopsis

The bus is Aperio's typed pub-sub primitive. Loci declare
subscribe / publish entries on named subjects; the runtime
delivers each published payload to every active subscriber on
that subject. Subjects may be in-process, cross-thread,
cross-process, or cross-machine; the source code does not
change.

## Subjects

A subject is a string-literal name like `"demo.greeting"` or
`"trellis.observation"`. Conventionally hierarchical and
dotted, but the language imposes no structure beyond *a string
literal*.

Every subject carries exactly one type. The compiler verifies
all `subscribe` and `publish` declarations naming the same
subject across the program use the same type.

## Dispatch

### In-process

When the publisher and all subscribers run in the same
process, dispatch is direct: the runtime walks the subject's
subscriber list, copies the payload from the publisher's arena
into each subscriber's arena (per **bus copy semantics**), and
arranges for the subscriber's handler to run.

For cooperative subscribers, the handler runs on the shared
scheduler thread; the publisher's locus does not block waiting
for delivery — publish-and-continue is the dispatch model.
Substrate cells (handler invocations) are queued and run
between publisher cells.

For pinned subscribers, the runtime delivers via the pinned
locus's per-locus mailbox (see [runtime](../runtime.md)).

### Cross-process

When a subject is bound to a remote transport via
`LOTUS_BUS_CONFIG` (or the higher-level `deployment.yaml`,
roadmap), the publisher serializes the payload to the m70 wire
format and dispatches to each `connect`-role transport peer.
Each peer's runtime deserializes the payload into its
subscriber's arena.

See [deployment](../deployment.md) for the configuration
surface.

## Bus copy semantics

When `<-` runs, the payload is copied from the publisher's
arena into each subscriber's arena. Consequences:

- **The subscriber owns its copy.** The handler may keep
  references to (parts of) the payload for as long as the
  subscriber's lifecycle lasts.
- **The publisher does not block.** Once `<-` returns, the
  publisher is free to mutate or discard the original payload;
  the subscriber's copy is independent.
- **In-process and cross-process look identical at the source
  level.** The arena-to-arena copy in-process and the
  byte-stream copy across a transport boundary present the
  same observable shape.

## Wire format (m70)

For cross-process subjects:

| Field type | Wire form |
|---|---|
| `Int`, `Float`, `Bool`, `Time`, `Duration` | 8 bytes, little-endian |
| `Decimal` | 16 bytes |
| `String` | 8-byte LE length prefix + UTF-8 bytes (no NUL) |
| Nested struct, enum, array as a field | not yet supported in v0 |

Field order on the wire is declaration order; there is no
padding, no field tags, no header. **Compile-time agreement,
no runtime negotiation** — both sides compiled the same `type`
declarations.

A subscriber reading the wire allocates string bytes from a
**lazy global payload arena** (`lotus_bus_payload_arena_alloc`)
that lives for the life of the process.

## F.8: vertical-only-flow

Per **F.8**:

- The graph of communication is closed by the union of all
  declared subscribe / publish entries. There is no way for a
  message to reach a locus that did not declare a subscription.
- Failures do not flow along the bus. A `ClosureViolation`
  propagates upward through the parent's `on_failure`, not
  laterally.

## Subject types in v0

| Subject type | Status |
|---|---|
| `Int`, `Float`, `Decimal`, `Bool`, `String`, `Time`, `Duration`, `Bytes` as struct fields | Supported |
| Enums (no-payload and with payload) as the subject's top-level type | Supported (within m70 limits) |
| Nested struct as a field | Not yet supported (v0 wire format) |
| Tuples as the subject's top-level type | Not yet supported |
| Arrays as a field | Not yet supported (v0 wire format) |

The bus's payload type is conventionally a user-defined
record (`type T { ... }`), not a primitive.

## See Also

- [Bus blocks (locus member)](../loci/bus.md)
- [Statements (the `<-` operator)](../statements/index.md)
- [Runtime](../runtime.md)
- [Deployment](../deployment.md)
