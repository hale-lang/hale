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

## Reading someone else's ring

A `shm_ring` binding speaks Hale's *own* ring format. But sometimes
the ring already exists — written by another program in another
language, with its own binary layout. Instead of hand-writing FFI
or forking the runtime, you *declare* that layout and point a
binding at it:

```hale
ring_layout ForeignRing {
    magic 0x52494E47464D5431;        // expected header magic at offset 0
    version 1 at 8 : u32;            // header field `version`, must equal 1
    buffer_size at 12 : u32;         // ring capacity, read from the header
    data_at 128;                     // first record starts here
    cursor published {               // the producer's published byte cursor
        at 64; repr atomic_u64; load acquire; unit bytes;
    }
    framing byte_records {           // records are [u32 length][payload]
        len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF;
    }
    overflow lap_detect;
}

main locus App {
    bindings {
        Ticks: shm_ring("/foreign.ticks", on_overflow: drop,
                        layout: ForeignRing) where zero_copy;
    }
}
```

A subscriber on `Ticks` now reads that foreign ring directly: the
runtime attaches it read-only, checks the magic and version, and
walks the length-prefixed records, handing each payload to your
`on_tick` handler with no copy. Your handler code is identical to
any other `shm_ring` subscriber — the layout only changes how the
substrate finds and frames the bytes.

A binding with no `layout:` keeps Hale's native ring, so nothing
you wrote before changes.

The same binding works the other way too. If a locus in your
program *publishes* the topic, it becomes the ring's producer: Hale
creates the segment, writes the header the layout describes, and
frames each `Ticks <- Tick { ... }` as a length-prefixed record
another program (or another language) can read. Give the binding a
`buffer_size:` to size the ring:

```hale
Ticks: shm_ring("/foreign.ticks", on_overflow: drop,
                layout: ForeignRing, buffer_size: 65536) where zero_copy;
```

So the same declared layout lets Hale sit on either side of a
foreign ring — consume what another process writes, or produce what
another process reads — with the locus body unchanged. Two caveats
at this version: a subscriber sees records published after it
attaches (no replay of history), and if it falls more than a full
buffer behind it resyncs rather than read a torn record.

### Mixed record types: a raw `BytesView` payload

The examples above bind a fixed payload struct — every record on the
ring is the same shape. Real feeds are often heterogeneous: a header
plus one of several record types, selected by a discriminator, with
varying length. Bind such a topic to a **`BytesView`** payload and the
subscriber receives a bounded view over each record to decode itself:

```hale
topic Recs { payload: BytesView; }

locus Reader {
    bus { subscribe Recs as on_rec; }
    fn on_rec(v: BytesView) {
        let kind = std::bytes::read_u8(v, 0) or 0;
        match kind {
            1 => { /* decode an L1 record with std::bytes::read_* */ }
            2 => { /* decode an L2 record */ }
            _ => { }
        }
    }
}
```

No fixed size is assumed (a differently-sized valid record isn't
dropped), and you decode with the `std::bytes::read_*` pack readers and
a discriminator branch. This is the path for reading real external
mixed-record rings; the typed-struct binding stays the fast path for a
homogeneous ring.

Producing such a ring is symmetric — build a record with a
`BytesBuilder` and send the bytes:

```hale
fn emit_l2(level: L2) {
    let b = std::bytes::BytesBuilder { initial_cap: 64 };
    b.append_u8(2);                // discriminator
    b.append_u32_le(level.price);
    b.append_u32_le(level.qty);
    Recs <- b.view();              // framed at its own length
}
```

`Recs <- bytes` frames `[len_prefix len][bytes]` where `len` is the
value's actual byte length, so each record carries its own size.

## The same shape, one tier down

Notice this is the same move as everything else at this level: an
operational requirement (zero-copy delivery) declared at the
deployment seam, validated by the compiler, consumed by codegen
to pick a lowering — while the locus body stays the synchronous,
portable code you wrote three tiers ago. You reach under the hood
without rewriting the program.

Next: calling into native libraries — [Binding C](./binding-c.md).
