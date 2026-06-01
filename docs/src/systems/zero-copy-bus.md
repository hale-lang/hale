# Zero-copy & the high-frequency bus

> **Coming from Rust / C++?** This is the shared-memory ring
> buffer you'd otherwise build by hand with `mmap` and atomics.
> For same-machine routes north of ~100k msg/s — market data, tick
> streams — the per-message copy at the locus boundary shows up in
> the latency budget. A `shm_ring` binding writes the payload
> straight into a POSIX shared-memory slot the subscriber reads
> from. No kernel memcpy at the boundary. And it's still the same
> `subscribe`/`publish` code.

## The default copies; sometimes you can't afford it

Every ordinary bus delivery copies the payload into the
subscriber's arena — that's what keeps lifetimes independent and
the memory model sound (see [Memory & lifetime](./memory.md)).
For the vast majority of topics that copy is free in the noise.
For the hottest same-host routes it isn't, and you opt into a
zero-copy path explicitly.

## A `shm_ring` binding

In `main`'s [`bindings { }`](../services/multi-binary.md) block:

```hale
main locus App {
    bindings {
        L2Updates: shm_ring("/l2-updates",
                            slot_count:  1024,
                            on_overflow: fail)
                  where intra_machine, zero_copy;
    }
}
```

Publisher and subscriber `mmap` the same `/dev/shm` object and
coordinate through the ring's slot indices. The publisher writes
its payload directly into a slot; the subscriber reads from the
same memory. No copy crosses the boundary.

The `subscribe L2Updates as on_update;` handler is the *same line
of source* it would be over a Unix socket — the substrate picks
the zero-copy lowering from the binding, not from the locus code.

## The `where` clause is a checked contract

`where intra_machine, zero_copy` is two things at once: your
assertion about the route, and a contract the compiler validates.

- **Scope** — `intra_process`, `intra_machine`, or
  `cross_machine` (pick one). `zero_copy` with `cross_machine` is
  rejected: the network always serializes.
- **Behavior** — `zero_copy` is rejected on transports that can't
  honor it (`unix(...)` memcpies through the socket buffer; user
  adapters serialize through `send(subject, bytes)`).

## Zero-copy needs a flat payload

A payload you can drop into a shared slot must be **flat-shapeable**:
every leaf is a fixed-layout primitive (`Int`, `Float`, `Bool`,
`Decimal`, `Time`, `Duration`), a fixed-size array of those, or a
struct whose fields are all flat-shapeable. `String`, `Bytes`,
and unbounded arrays carry heap pointers that don't translate to
a shared slot, so the compiler rejects them on a zero-copy topic.
Use a fixed-size byte array (`[Byte; 256]`) for bounded text on
these routes.

## Overflow is your decision

A `shm_ring` binding must declare `on_overflow:` — slot
exhaustion needs a policy the substrate can't guess:

- **`block`** — the publisher spins until a slot frees.
  Right for control-plane data that must not be lost.
- **`drop`** — overwrite the next slot; slow consumers miss
  messages. Right for stale-is-worthless feeds.
- **`fail`** — panic with a clear diagnostic. Process-level
  visibility into back-pressure.

## The same shape, one tier down

Notice this is the same move as everything else at this level: an
operational requirement (zero-copy delivery) declared at the
deployment seam, validated by the compiler, consumed by codegen
to pick a lowering — while the locus body stays the synchronous,
portable code you wrote three tiers ago. You reach under the hood
without rewriting the program.

Next: calling into native libraries — [Binding C](./binding-c.md).
