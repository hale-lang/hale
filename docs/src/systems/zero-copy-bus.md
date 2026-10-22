# Zero-Copy & High-Frequency Bus

For high-throughput, same-machine message routes (100k+ messages/sec), copying payloads across locus boundaries can dominate latency. Binding a topic to `shm_ring` maps a POSIX shared-memory ring buffer (`/dev/shm`) into both publisher and subscriber processes so payloads are written and read directly in place without changing locus `publish` or `subscribe` code.

```hale
type Update { px: Int; qty: Int; }
topic L2Updates { payload: Update; }

main locus App {
    bindings {
        L2Updates: shm_ring("/l2-updates",
                            slot_count:  1024,
                            on_overflow: fail)
                   where intra_machine, zero_copy;
    }
}

fn main() { App { }; }
```

## Shared-Memory Ring Bindings (`shm_ring`)

Ordinary bus deliveries copy the payload into delivery-scoped storage to keep sender and receiver lifetimes independent (see [Memory & Lifetime](./memory.md)). When you bind a topic to `shm_ring` inside `main`'s `bindings { }` block, the publisher and subscriber `mmap` the same `/dev/shm` segment and coordinate via atomic slot cursors. The publisher writes its payload directly into a ring slot, and the subscriber reads from that slot without a kernel or user-space `memcpy`.

### Checked `where` Contracts

The `where` clause on a binding is statically verified by `hale check`:

| Constraint | Valid Combinations | Rejected Combinations |
|---|---|---|
| Route Scope | `intra_process`, `intra_machine`, `cross_machine` | Combining `zero_copy` with `cross_machine` (network transports always serialize). |
| Transport Capability | `shm_ring(...) where zero_copy` | Combining `zero_copy` with `unix(...)` or custom adapter loci (which copy or serialize bytes). |

### Flat-Shapeable Payload Rules

Because pointers cannot be dereferenced across separate processes sharing an `mmap` segment, a `zero_copy` topic requires a **flat-shapeable** payload:

- **Allowed types**: Fixed-layout primitives (`Int`, `Float`, `Bool`, `Decimal`, `Time`, `Duration`), inline fixed-size scalar arrays (`[Int; 8]`, `[Float; 4]`, `[Bool; N]`, `[Decimal; N]`, `[Duration; N]`), and nested `type` structs composed exclusively of flat-shapeable fields.
- **Rejected types**: `String`, `Bytes`, `StringView`, `BytesView`, unbounded arrays, and arrays of structs or `Time`. For variable-length or heterogeneous messages on a shared-memory ring, use a [raw `BytesView` payload](#mixed-record-types-bytesview-payloads) over a declared `ring_layout`.

### Overflow Policies (`on_overflow`)

Every `shm_ring` binding must declare an explicit `on_overflow` policy:

| Policy | Behavior When Ring Is Full | Typical Use |
|---|---|---|
| `block` | Publisher spins until the consumer frees a slot. During in-process shutdown, the reader continues draining until worker pools join; any publish still blocked after the reader stops raises `BusWaitAborted`. | Lossless control-plane or audit streams. |
| `drop` | Publisher overwrites the oldest slot; slow consumers skip overwritten items. | Real-time market data or telemetry where stale ticks are worthless. |
| `fail` | Process aborts immediately with a backpressure diagnostic. | Strict latency SLAs where ring saturation indicates a fatal sizing or consumer stall. |

## Per-Record vs. Batch Handlers (`Drain<T>`)

By default, the runtime invokes a subscriber's handler once per message:

```hale
type Update { px: Int; qty: Int; }
topic L2Updates { payload: Update; }

locus Book {
    params { total: Int = 0; }
    bus { subscribe L2Updates as on_update; }
    fn on_update(u: Update) {
        self.total = self.total + u.px;
    }
}

fn main() { Book { }; }
```

On high-rate feeds, per-record handler call overhead and scratch setup can become a bottleneck. Changing the handler parameter type from `T` to `Drain<T>` switches the subscription to **batch mode** without changing the `subscribe` declaration:

```hale,fragment
type Tick { px: Int; qty: Int; }
topic Quotes { payload: Tick; }

locus Agg {
    params { total: Int = 0; }
    bus { subscribe Quotes as on_quotes; }
    fn on_quotes(feed: Drain<Tick>) {
        for t in feed {
            self.total = self.total + t.px;
        }
    }
}
```

In batch mode:
- The runtime invokes `on_quotes` once per available batch of ring slots.
- `for t in feed` compiles to a tight inline loop that reads `t.px` directly from the mapped shared-memory slot without copying.
- The ring's consumer cursor advances once at the end of the batch rather than once per record.
- `Drain<T>` is valid only as a batch handler's parameter type and `for`-loop iterable; it is not a general value type and is not yet supported on foreign (`layout:`) rings.

## Foreign Shared-Memory Rings (`ring_layout`)

When interfacing with an existing shared-memory ring produced or consumed by a non-Hale process (such as a C++ feed handler), declare its binary header, cursor, and record framing with a top-level `ring_layout` block and reference it via `layout:` on the `shm_ring` binding:

```hale
type Tick { px: Int; qty: Int; }
topic Ticks { payload: Tick; }

ring_layout ForeignRing {
    magic 0x52494E47464D5431;        // expected 64-bit header magic at byte 0
    version 1 at 8 : u32;            // header version field at offset 8 must equal 1
    buffer_size at 12 : u32;         // ring data capacity read from header offset 12
    data_at 128;                     // byte offset where payload records begin
    cursor published {               // producer's published byte cursor
        at 64; repr atomic_u64; load acquire; unit bytes;
    }
    framing byte_records {           // records framed as [u32 length][payload]
        len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF;
    }
    overflow lap_detect;
}

locus Tape {
    params { last: Int = 0; }
    bus { subscribe Ticks as on_tick; }
    fn on_tick(t: Tick) { self.last = t.px; }
}

main locus App {
    params { tape: Tape = Tape { }; }
    bindings {
        Ticks: shm_ring("/foreign.ticks", on_overflow: drop,
                        layout: ForeignRing) where zero_copy;
    }
}

fn main() { App { }; }
```

### Consuming and Producing Foreign Rings

A `ring_layout` binding works in both directions:
- **As a subscriber**: The runtime attaches to the segment read-only, validates `magic` and `version`, and walks records starting from the point of attachment (historical records prior to attach are not replayed; if the consumer falls more than a full buffer behind, it resyncs to the latest cursor rather than reading a torn record).
- **As a publisher**: If a locus in your program publishes `Ticks`, Hale creates the shared-memory segment, writes the declared header, and frames each published message according to the layout. Pass `buffer_size:` on the binding to specify the ring capacity:

```hale,fragment
Ticks: shm_ring("/foreign.ticks", on_overflow: drop,
                layout: ForeignRing, buffer_size: 65536) where zero_copy;
```

> [!NOTE]
> Fixed-stride rings use `framing slots` instead of length-prefixed `framing byte_records`. Omitting `layout:` uses Hale's native ring format.

### Mixed Record Types (`BytesView` Payloads)

External feeds often multiplex multiple record shapes of varying lengths onto a single ring using a discriminator byte. To handle heterogeneous frames, declare the topic payload as `BytesView` and bind it to a `layout:` ring (without `where zero_copy`, as each frame is copied into a scratch buffer for safe bounds-checked parsing):

```hale
topic Recs { payload: BytesView; }

locus Reader {
    bus { subscribe Recs as on_rec; }
    fn on_rec(v: BytesView) {
        let kind = std::bytes::read_u8(v, 0) or 0;
        match kind {
            1 -> { /* decode L1 record via std::bytes::read_* */ },
            2 -> { /* decode L2 record */ },
            _ -> { },
        }
    }
}
```

> [!IMPORTANT]
> A `BytesView` topic must be bound to a `shm_ring` with a `layout:` declaration; subscribing or publishing to an unbound `BytesView` topic (or one without `layout:`) is a compile error.

To publish variable-length frames from a buffer, construct a `BytesBuilder` and publish its view (`Recs <- b.view();`), which prefixes the frame with its actual byte length:

```hale,fragment
locus Feed {
    bus { publish Recs; }
    fn emit_l2(level: L2) {
        let b = std::bytes::BytesBuilder { initial_cap: 64 };
        b.append_u8(2);
        b.append_u32_le(level.price);
        b.append_u32_le(level.qty);
        Recs <- b.view();
    }
}
```

### In-Place Slot Writing (`Topic.write`)

To avoid building a temporary `BytesBuilder` and copying it into the ring on a hot producer path, use `Topic.write(max) { w => ... }` to reserve up to `max` bytes directly in the ring slot and write fields in place:

```hale,fragment
locus FastFeed {
    bus { publish Recs; }
    fn emit_l2(level: L2) {
        Recs.write(24) { w =>
            std::bytes::write_u8(w, 0, 2)               or raise;
            std::bytes::write_u32_le(w, 1, level.price) or raise;
            std::bytes::write_u32_le(w, 5, level.qty)   or raise;
            9 // commits 9 bytes as the record's actual length
        };
    }
}
```

The `std::bytes::write_*` functions are bounds-checked (`fallible(IndexError)`) and return the byte offset immediately past the written value. The slot reservation and commit are lexically scoped to the closure block, preventing `w` from escaping or remaining uncommitted.

### Wire-Layout Field Tags (`repr:"..."`)

Instead of maintaining manual byte offsets in `std::bytes::read_*` and `write_*` calls, tag struct fields with `repr:"..."` metadata to generate bounds-checked, zero-overhead accessors automatically:

```hale
type L2 {
    kind:  Int `repr:"u8"`;       // 1 byte  @ offset 0
    price: Int `repr:"u32_le"`;   // 4 bytes @ offset 1
    qty:   Int `repr:"u32_le"`;   // 4 bytes @ offset 5
}
```

For padded foreign layouts, pin a field's byte offset explicitly using `at=<offset>` (for example, `` `repr:"u32_le,at=4"` ``). The compiler synthesizes `Type::field(v)` readers and `Type::set_field(w, val)` writers that desugar directly to the underlying `std::bytes` primitives at the computed offsets:

```hale,fragment
fn on_rec(v: BytesView) {
    let p = L2::price(v) or raise;
    ...
}

fn emit(level: L2) {
    Recs.write(9) { w =>
        L2::set_kind(w, 2)            or raise;
        L2::set_price(w, level.price) or raise;
        L2::set_qty(w, level.qty)     or raise;
        9
    };
}
```

### Per-Record Headers & Wire Timestamps (`std::shm`)

When a foreign ring prefixes each record with a fixed metadata header (configured via `record_header_bytes` and optional `pad_field_*` wrap markers in `ring_layout`), handlers can inspect the current record's sequence number and producer wire timestamp via `std::shm`:

```hale
fn on_rec(v: BytesView) {
    let seq = std::shm::last_record_seq();
    let wire_ns = std::shm::last_record_kernel_ns();
}
```

Both functions return `0` if the active `ring_layout` does not declare the corresponding header field. Adding `recheck post_copy` to the `ring_layout` re-validates the record header after copying payload bytes, ensuring a producer lapping the ring cannot surface a torn header to the handler.
