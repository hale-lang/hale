# Binary SHM-ring interop: `ring_layout` declarations + `std::bytes` pack primitives

**Status:** Design proposal, 2026-06-06. Pre-implementation. Driven by a
concrete first consumer (fathom ↔ the external stack). Two related but independently
landable features. Awaiting compiler-team review + staging decision.

**Authors / context:** Raised from the fathom side while scoping
interop with **an external trading stack** (a sister C++ system) whose
inter-process transport is a lock-free shared-memory broadcast ring
(its shared-memory ring-layout header — the `RingPrefix` struct). The
same need recurs for any binary wire protocol (binary venue feeds,
drop-copy streams, etc.).

---

## TL;DR

Hale already has a safe, cross-process SHM ring transport
(`shm_ring(...) where zero_copy`, `runtime/lotus_shm_ring.c`) and
injectable codecs. Two things are missing to interoperate with an
**externally-defined** binary ring + binary payloads:

1. **`std::bytes` binary-pack primitives** — read/write fixed-width
   integers and floats at byte offsets, with explicit endianness. Small,
   safe, broadly useful (every binary protocol needs it). *No SHM
   coupling — lands on its own.*

2. **A compile-time `ring_layout` declaration** that the existing
   `shm_ring` codegen parameterizes over, instead of hardcoding the
   `LRSRNG1` format. This makes a *foreign* ring layout (known at compile
   time) a first-class, **memory-safe**, codegen-lowered transport — no
   FFI, no runtime polymorphism. Hale's own `LRSRNG1` becomes the default
   instantiation, dogfooding the mechanism.

The guiding realization: **"foreign" is orthogonal to "unsafe."** The
unsafety of `mmap`+atomics is that the access is *raw and unconstrained*,
not that the format belongs to someone else. A layout known at compile
time can be lowered to exactly the same bounded, codegen-owned accessor
we already emit for `LRSRNG1`. This is the `bindgen` / `svd2rust` / SBE
pattern: *declare the foreign layout, codegen the safe typed accessor.*

---

## Why

### The immediate need

The external stack exposes its hot-path data over a POSIX-SHM lock-free SPMC ring.
Layout (`ring_layout.hpp`, verbatim constants):

```
k_ring_magic       = 0x52494E47464D5431   // "RINGFMT1"
k_ring_version     = 1
k_padding_sentinel = 0xFFFFFFFF
k_cacheline_size   = 64
k_record_alignment = 8

struct RingHeader {                  // 64 bytes
  u64 magic;       // @0
  u32 version;     // @8
  u32 buffer_size; // @12
  std::byte reserved[48];            // @16
};
struct RingPrefix {
  RingHeader header;                          // @0   (64 B)
  alignas(64) std::atomic<u64> committed;     // @64  (own cache line)
  alignas(64) std::byte buffer[1];            // @128 (the byte ring)
};
```

- **Byte-record framing** (not fixed slots): each record is
  `align_up(sizeof(u32) + payload, 8)` — a `u32` length prefix followed
  by the payload, padded to 8.
- **Monotonic byte cursor:** `committed` = total bytes ever committed.
  Publisher release-stores it; a reader acquire-loads it and walks
  records from its own `local_cursor` (in bytes) at
  `buffer[cursor % buffer_size]`.
- **Wrap:** when a record won't fit before end-of-buffer, the producer
  writes the `0xFFFFFFFF` padding sentinel as the length and wraps.
- **Overflow (lapping):** `committed < local || committed - local >
  buffer_size` → the reader fell behind and must resync.
- SPMC, single producer; cross-language by design (the external stack already has a
  Rust↔C++ SHM-transit perf test).

fathom wants to read (and possibly write) this ring from Hale.

### Why not just use what we have

| Capability | Have it? | Gap |
|---|---|---|
| SHM ring transport | ✅ `shm_ring(...)` / `lotus_shm_ring.c` | format is **hardcoded** `LRSRNG1` — slot-framed, different magic/cursor |
| Cross-process / cross-language | ✅ (C-reader test, `shm_ring_publish.rs`) | only for *our* format |
| Injectable payload codec | ✅ `CodecSpec`, `codec_encode_override` | codec controls the *payload*, not the *framing* |
| Read u32/u64/f64 at a byte offset | ❌ `std::bytes` is `at`(1 byte)/`slice`/`concat`/builder | **no binary pack/unpack** |
| Map a foreign ring layout | ❌ | layout is not declarable; only `LRSRNG1` is wired |

The `LRSRNG1` and `RINGFMT1` formats are structurally different:

| | `LRSRNG1` (ours) | `RINGFMT1` (the external stack) |
|---|---|---|
| magic | `0x4C5253524E4731` | `0x52494E47464D5431` |
| header | 128 B (magic@0, slot_size@8, slot_count@16, seqno@24, consumer_seqno@64) | 64 B + cursor@64 + data@128 |
| published cursor | `seqno` (slot count) @24 | `committed` (byte count) @64 |
| record model | **fixed-size slots** | **variable-length byte records** (`u32` len + 8-align) |
| wrap | `seqno % slot_count` | byte cursor + `0xFFFFFFFF` pad sentinel |

So the injectable codec (payload) does **not** bridge this — the
slot-vs-byte framing and cursor semantics live in the C runtime. Today
the only routes to the external stack's ring are: adopt our format on
the external side, or drop to `@ffi("c")` glue. This proposal adds a third, better
one: **declare the foreign layout and let codegen lower it safely.**

---

## Proposal A — `std::bytes` binary-pack primitives

A safe, bounded, endianness-explicit read/write surface for fixed-width
scalars at byte offsets. Independent of SHM; useful for any binary
protocol. Lands first.

### Read (from a `Bytes` value or a byte view)

```hale
// Unsigned + signed integers, little- and big-endian. Offset is a byte
// index; width is implied by the function. Bounds-checked: a read whose
// [off, off+width) exceeds the buffer raises BoundsError.
std::bytes::read_u8 (b: Bytes, off: Int) -> Int fallible(BoundsError);
std::bytes::read_u16_le(b, off) -> Int fallible(BoundsError);
std::bytes::read_u16_be(b, off) -> Int fallible(BoundsError);
std::bytes::read_u32_le(b, off) -> Int fallible(BoundsError);
std::bytes::read_u32_be(b, off) -> Int fallible(BoundsError);
std::bytes::read_u64_le(b, off) -> Int fallible(BoundsError);
std::bytes::read_u64_be(b, off) -> Int fallible(BoundsError);
std::bytes::read_i8 / i16_{le,be} / i32_{le,be} / i64_{le,be}  // sign-extended
std::bytes::read_f32_le(b, off) -> Float fallible(BoundsError);
std::bytes::read_f64_le(b, off) -> Float fallible(BoundsError);
std::bytes::read_f64_be(b, off) -> Float fallible(BoundsError);
```

Notes / decisions to make:
- **`u64` → `Int`.** Hale `Int` is i64; a true `u64` with the top bit set
  wraps to negative. the external stack's cursors/ids fit i63 in practice, but the
  general primitive should say so. Options: (1) document the wrap and add
  `read_u64` returning the raw bit pattern as i64; (2) gate on a `Uint`
  type if/when one exists (FFI spec already reserves `Uint`). Recommend
  (1) now.
- **Fallibility vs hot path.** Bounds-checked + `fallible(BoundsError)`
  matches Hale's structural-failure discipline and is the safe default.
  For the ring fast path (millions of reads/s) a later
  `read_*_unchecked` (caller asserts the slice was length-validated once)
  is a reasonable optimization — explicitly out of scope for v1.
- **Endianness.** Provide both; x86-native binary structs (the external stack) are
  LE, so `_le` is the common case. A bare `read_u32`/`read_u64` aliasing
  host-endian is a convenience worth considering but invites portability
  bugs — recommend explicit `_le`/`_be` only.

### Write (append to a `std::bytes::builder`)

```hale
b.append_u8(n: Int);
b.append_u16_le(n) / _be ;  append_u32_le / _be ;  append_u64_le / _be ;
b.append_i*_{le,be};  append_f32_le;  append_f64_{le,be};
b.append_pad(to_align: Int);   // zero-fill to the next `to_align` boundary
```

### Write-in-place (for zero-copy ring producing)

The ring **producer** path writes fields directly into a slot/record's
mapped memory (no intermediate builder → no copy). That needs a
*writable view at an offset*, which `std::bytes` doesn't currently model
(values are immutable; the builder only appends). Two options:

- **(A1)** A `BytesMut`/writable-view type the codegen hands the codec for
  the reserved slot, with `write_u32_le(view, off, n)` etc. mirroring the
  readers. Cleanest; needed for true zero-copy.
- **(A2)** Codec builds into a `builder`, codegen `memcpy`s the snapshot
  into the slot (one copy). Simpler; gives up zero-copy on the write
  side.

Recommend shipping the readers + builder-append first (A2 is enough for a
working producer), and adding the writable view (A1) when the zero-copy
write path matters.

### Optional follow-on — layout-declared payload structs

The payload analog of `ring_layout` below: declare a POD struct layout
(`@repr(c)` / field offsets) once and get generated typed field
accessors, instead of hand-writing `read_u32_le(b, 12)` per field. This
is the `bindgen`/`zerocopy`/SBE-codegen layer. Strictly additive on top
of the pack primitives — call it out as a future Proposal A′, not v1.

---

## Proposal B — compile-time `ring_layout` declaration

Make the SHM ring **layout** a source-level declaration that the existing
`shm_ring` codegen consumes, instead of hardcoding `LRSRNG1` in
`lotus_shm_ring.c`. A `ring_layout` captures everything the runtime needs
to attach, validate, iterate, and (optionally) publish — all of which the
runtime already does for `LRSRNG1`; this just parameterizes it.

### Surface

> **Status (2026-06-06): the `ring_layout` declaration LANDED (PR1).**
> The concrete grammar below (members keyword-led + `;`-terminated;
> `cursor`/`framing` as nested attr blocks; attr values are idents or
> ints) parses, resolves, and typechecks — the layout *contract*
> (known width reprs, recognized framing kind with `len_prefix` for
> `byte_records`, ≥1 cursor with an offset, known cursor
> reprs/orderings/unit) is enforced in `hale-types::check`.
>
> **PR2 (LANDED): the `layout:` binding kwarg.**
> `shm_ring(..., layout: Name)` parses and resolves to a declared
> `ring_layout` (unknown / non-layout names diagnose); absent
> `layout:` keeps the native ring, so existing bindings are unchanged.
> Grammar: `spec/grammar.ebnf` (`ring_layout_decl`, `shm_ring_kwarg`).
>
> **PR3 (LANDED): the read-only `byte_records` consumer.**
> A subscriber on a `layout:`-bound topic now actually reads the
> foreign ring. Runtime (`lotus_shm_ring.c`):
> `lotus_shm_ring_open_layout` (attach read-only, validate
> magic/version, read buffer_size) +
> `lotus_bus_register_subscriber_shm_ring_layout` + a `byte_records`
> reader thread (modular `[len_prefix][payload]` walk, `pad_sentinel`
> tail-pad skip, `align_up` stride; lossy-but-safe lap resync).
> Codegen (`emit_bus_register_shm_ring`) branches on the resolved
> layout, emitting a flat 16-entry descriptor global + the
> layout-aware register call; the native LRSRNG1 path is untouched
> (back-compat verified). Field roles read by convention from the
> layout (`version`/`buffer_size` scalars). Validated by a C
> mock-producer ↔ consumer round-trip (`shm_ring_layout.rs`,
> roundtrip + wrap, clean under ASan/UBSan) and a codegen IR-shape
> test (`shm_ring_layout_codegen.rs`). Spec: `semantics.md`
> § "Foreign rings", `runtime.md`. **Read path complete.**
> Still out of scope: the producer path for foreign layouts (M3),
> `slots` framing, historical replay, multi-cursor back-pressure.

The external stack's ring becomes a declaration:

```hale
ring_layout ForeignRing {
    magic        0x52494E47464D5431;
    version_at   8  : u32;          // validated == expected_version
    expected_version 1;
    buffer_size_at 12 : u32;        // ring capacity, read from header
    data_at      128;               // first record offset

    cursor {
        kind     published;         // monotonically-increasing publish cursor
        at       64;
        repr     atomic_u64;
        load     acquire;           // consumer side
        store    release;           // producer side
        unit     bytes;             // cursor counts bytes (vs slots)
    }

    framing byte_records {
        len_prefix    u32;          // record = [u32 len][payload]
        align         8;
        pad_sentinel  0xFFFFFFFF;   // len value meaning "skip to wrap"
    }

    overflow lap_detect;            // committed - local > buffer_size → resync
}
```

Hale's existing format becomes the *default* declaration (dogfooding):

```hale
ring_layout LotusRing {            // == today's LRSRNG1, now declared
    magic 0x4C5253524E4731;
    data_at 128;
    cursor { kind published; at 24; repr atomic_u64; load acquire; store release; unit slots; }
    cursor consumer { at 64; repr atomic_u64; }     // back-pressure cursor
    framing slots { slot_size <param>; slot_count <param>; }
    overflow { policies drop, block, fail; }
}
```

### Binding a topic to a layout

The `shm_ring(...)` binding gains a layout reference; absent one, it
defaults to `LotusRing` (100% back-compat):

```hale
bindings {
    // read the external stack's ring:
    ForeignTick: shm_ring("/foreign.mdgw.ticks", layout: ForeignRing) where zero_copy;
    // unchanged today's form still means layout: LotusRing
    Tick:      shm_ring("/lotus.ticks", slot_count: 4096, on_overflow: drop) where zero_copy;
}
```

Payload encode/decode stays the existing injectable `CodecSpec`, now
implemented with the Proposal-A pack primitives.

### Lowering

`lotus_shm_ring.c` already implements every primitive this needs —
`shm_open`+`mmap`, magic/version validation, `atomic_load_explicit(...,
acquire)` of the cursor, `atomic_fetch_add(..., release)` to commit, wrap
handling. The work is to **parameterize** that C (or generate it) from
the `ring_layout` fields rather than the hardcoded `LRSRNG1` constants:

- **attach:** `mmap`, check `magic` == declared, `version` == declared,
  read `buffer_size` from its declared offset.
- **iterate (consumer):** acquire-load the cursor at its offset; while
  `local < committed`: at `framing == byte_records`, read the `u32` len
  at `data_at + local % buffer_size`, skip `pad_sentinel`, hand the codec
  a (bounded) view of `[off+4, off+4+len)`, advance `local` by
  `align_up(4+len, align)`; check `overflow` each step. At `framing ==
  slots`, the existing slot-index path.
- **publish (producer):** reserve `align_up(4+payload, align)` (byte
  ring) or claim next slot; codec writes the payload; release-store /
  fetch-add the cursor.

The codegen surface that already routes `Topic <- v` to
`lotus_bus_publish_shm_ring` and registers the subject→ring + codec
(`emit_bindings_prelude`, `lower_send`, `lotus_bus_register_codec`) is
reused unchanged; only the ring descriptor it registers becomes
layout-derived.

---

## Safety analysis

This is the crux of *why compile-time-declared beats runtime/FFI*, and
why it does **not** widen the unsafe surface.

- **The codegen still owns every raw operation.** The user never writes
  pointer arithmetic, `mmap`, or an atomic. They write a declaration; the
  compiler emits the bounded accessor — exactly as it does today for
  `LRSRNG1`. The trusted/unsafe surface is the *ring-layout lowering*, a
  single compiler component, not per-use app code.
- **All access is bounded to the mapping — given the checks below.**
  Record offsets stay within `[data_at, data_at+buffer_size)`; the pack
  primitives are bounds-checked (overflow-safe: a near-`INT64_MAX`
  offset can't wrap the guard). The headline guarantee — **a wrong
  declaration produces wrong values, never out-of-bounds memory
  access** — holds *because* the boundary against a non-conforming /
  hostile foreign producer is closed by:
    - **frontend** (`check_ring_layout` / the binding check):
      `len_prefix_width <= align`, `buffer_size % align == 0` (producer
      `buffer_size:`), an 8-aligned `atomic_u64` cursor, and presence of
      `magic` / `data_at` / a `buffer_size` scalar;
    - **attach** (`lotus_shm_ring_open_layout`): reject a foreign
      `buffer_size` that isn't a multiple of `align`;
    - **per record** (the `byte_records` reader): the len-prefix read is
      clamped to `pos + len_prefix_width <= cap`; the framed `len` must
      equal the bound payload's fixed `value_size` (else resync, never
      dispatch — so the handler never reads past a short record); the
      record-fit test is overflow-safe against a hostile `len`.

  Without those, a hostile producer *could* drive an OOB read at the
  wrap; with them it cannot. This is still strictly safer than
  `@ffi("c")` glue, which has no such enforced boundary.
- **Validated at attach.** `magic` + `version` mismatch, or a
  `buffer_size` that isn't `align`-aligned / overruns the mapping →
  clean failure (don't map an incompatible ring).
- **The trust boundary is the same as every schema binding.** You assert
  the declared layout matches the producer — identical to `bindgen`
  (`#[repr(C)]`), `svd2rust` (memory-mapped registers), or an SBE schema.
  Get an offset wrong and you read garbage; you do not corrupt memory.
  This is a well-understood, accepted contract.
- **Atomicity/ordering is declared, not improvised.** The cursor's
  `repr`/`load`/`store`/ordering are part of the layout, so the codegen
  emits the correct `memory_order_acquire`/`release` intrinsics — the
  same ones `lotus_shm_ring.c` uses — rather than leaving ordering to
  hand-written FFI.

Net: compile-time `ring_layout` keeps the "codegen encapsulates the
unsafe primitives" model that already makes `shm_ring` safe, and merely
stops hardcoding one instance of it. It is **not** the runtime
foreign-attach capability (below), which would be the thing that punches
through the safety model.

---

## Non-goals

- **Runtime/dynamic foreign-ring attach.** Mapping a layout *not known at
  compile time* (negotiated at runtime, or one binary tapping many
  formats) is a genuinely different, heavier capability and the only one
  that reintroduces unconstrained access. Explicitly out of scope —
  YAGNI until a real "format unknown until runtime" case exists.
- **An arbitrary ring DSL.** Do **not** try to parameterize every
  conceivable ring. Cover the two formats that exist (`LotusRing`
  slot-framed, `ForeignRing` byte-record-framed) plus the common Aeron-ish
  shape, and stop. The failure mode to avoid is a config-soup DSL where
  every ring is a special case.
- **Replacing `@ffi("c")`.** FFI remains the escape hatch for calling a
  partner system's *own* reader, or formats you refuse to declare.
- **A general `Uint`/128-bit story.** Out of scope; `u64`→`Int` wrap is
  documented (see Proposal A).

---

## Driving use case & validation plan

**fathom ↔ the external stack.** fathom (Hale) reads the external stack's `ForeignRing` (market
data / feed), and optionally publishes into the external stack's `ingress` ring.
Same-host only (SHM). The payload structs are the external stack's POD messages,
decoded via an injectable codec built on the Proposal-A primitives.

**Validation (mirrors how fathom de-risked its grease UDP integration —
a loopback against a faithful mock before any live wiring):**
1. Pack primitives: unit tests, round-trip every width/endianness; fuzz
   against bounds.
2. `ring_layout` read path: a Hale reader with `layout: ForeignRing`
   against a byte-ring writer producing the exact `RingPrefix` format
   (either a small C harness or the external stack's own `ring_replay` test rig) —
   assert bit-for-bit record recovery, wrap, and lap-detect.
3. `ring_layout` producer path: Hale writer → the external stack (or C) reader.
4. Regression: the existing `LRSRNG1` tests must pass unchanged with
   `LotusRing` as the default declaration (proves the parameterization
   didn't regress the hardcoded path).

---

## Staging

1. **Proposal A, readers + builder-append.** Self-contained, unblocks
   hand-written binary codecs immediately (and helps non-JSON wire
   formats generally). No SHM dependency.
   - ✅ **Readers LANDED** (2026-06-06): `read_u8`/`u16`/`u32`/`u64`
     (`_le`/`_be`), the signed `read_i*`, and `read_f32_le`/
     `read_f64_{le,be}`, each `(b, off) -> Int|Float
     fallible(IndexError)`, bounds-checked. One generic runtime helper
     (`lotus_bytes_read_uint`) backs all integer widths; floats
     bit-cast the raw bits in codegen. **Decision:** reused the
     existing **`IndexError`** (the same error `std::bytes::at`
     already raises for an out-of-range byte offset) rather than
     introducing the tentatively-named `BoundsError` — same family,
     same semantics, no parallel error type. (Resolves the
     error-naming open question toward consistency.)
   - ✅ **Builder-append writers LANDED** (2026-06-06):
     `BytesBuilder.append_u8`/`u16`/`u32`/`u64` (`_le`/`_be`), the
     signed `append_i*`, `append_f32_le`/`append_f64_{le,be}`, and
     `append_pad(to_align)`. Seed methods (`bytes_builder.hl`) over one
     runtime helper (`lotus_bytes_builder_append_scalar`) + an
     `append_pad`; floats bit-cast in codegen. Round-trip tested
     against the M1 readers. This is the A2 (build-then-copy) producer
     path — enough for a working ring producer; the zero-copy writable
     view (A1) is still future.
2. **Proposal B, read-only, `byte_records` framing.** `ring_layout`
   declaration + `layout:` on the `shm_ring` binding, consumer path only.
   First real target: read `ForeignRing`.
3. **Proposal B, producer path** + Proposal A writable view (A1) for
   zero-copy writes.
   - ✅ **Typed producer LANDED** (M3a, #61): `Topic <- Struct { ... }`
     frames a fixed-size `byte_records` record.
   - ✅ **BytesView/raw producer LANDED** (2026-06-08): `Topic <- bytes`
     (a `Bytes` or `BytesView` value) frames `[len_prefix len][bytes]`
     with `len` = the value's actual byte length, so a Hale program can
     emit heterogeneous / variable-width records — the producer mirror of
     the BytesView consumer (#72). Codegen-only (`lower_send_shm_ring`
     extracts data+len via `lotus_bytes_data`/`lotus_bytes_len`); the
     runtime publish was already size-generic. End-to-end test
     (`shm_ring_layout_bytesview_producer.rs`).
   - ✅ **A1 zero-copy writable view LANDED** (2026-06-08):
     `Topic.write(max) { w => ...; len }` reserves a slot, binds a
     writable `BytesMut` view `w`, and commits the tail length — the
     producer writes fields DIRECTLY into the mapped ring (no
     build-then-copy). New surface: the closure-scoped `write` construct
     (a bespoke parser form — Hale has no value-closures/keyword-args), a
     `BytesMut` type (reuses the 16-byte view ABI), the
     `std::bytes::write_*` family (mirrors the readers, backed by
     `lotus_bytes_write_uint`), and a runtime reserve/commit split of the
     publish path (#78). Tests: C-driver `reserve_commit` +
     `shm_ring_layout_zerocopy_write.rs` (Hale end-to-end). This was the
     last open item in the foreign-ring interop staging.
4. **Dogfood:** re-express `LRSRNG1` as the built-in `LotusRing`
   declaration; delete the hardcoded constants in favor of the default
   instantiation.
   - ✅ **`slots` framing LANDED (additive)** (2026-06-08): the layout
     path now supports `framing slots` — a fixed-stride slot ring whose
     geometry (`slot_size`/`slot_count`) is read from the foreign header,
     cursor = the published seqno, lap-skip matching the native reader. A
     `ring_layout LotusRing` expresses the native `LRSRNG1` shape, and a
     `layout: LotusRing` consumer reads a ring the native producer writes
     (C-driver `lotus_dogfood` + a Hale end-to-end test). Scope was
     **additive**: the native registration/publish path is unchanged (no
     reroute), so the hardcoded constants are *not* yet deleted — that
     (and parameterized slot geometry for a layout *producer*) is the
     remaining tail. Proves the abstraction covers the in-house ring.
5. **(Optional) Proposal A′:** layout-declared payload structs
   (`@repr(c)` → typed accessors).

Each stage is independently useful and testable.

---

## Open questions for the team

1. **Syntax & placement of `ring_layout`.** Top-level decl? A `@form`
   variant (it is, arguably, "a form whose lowering is a shared-memory
   ring")? How does it interact with the proto-locus "everything is a
   locus" direction (`notes/proto-locus-design.md`)?
2. **Parameterize C vs generate C.** Extend `lotus_shm_ring.c` to take a
   runtime layout descriptor struct, vs. codegen a specialized C/IR
   reader per `ring_layout`. Perf (the hot path wants the offsets/
   ordering as compile-time constants) argues for generation/
   specialization.
3. **`u64` cursor vs `Int`.** The `committed`/`seqno` cursors are real
   `u64`. Internally compare/advance them correctly even though the
   user-facing scalar reads are i64. Confirm the runtime keeps cursors as
   `uint64_t` and only the *payload* reads surface as `Int`.
4. **Multi-producer.** the external stack's ring is SPSC/SPMC like ours (single
   producer). Keep MP out of scope (matches `lotus_shm_ring.c` v1).
5. **Bounds-check cost.** Is `fallible(BoundsError)` per scalar read
   acceptable on the hot path, or do we want the validated-slice +
   unchecked-read fast path from v1?
6. **Writable views.** Land `BytesMut` (A1) now or defer (A2 copy-on-
   publish first)?
7. **Codec ↔ layout coupling.** Confirm the existing `CodecSpec` /
   `codec_encode_override` path composes cleanly with a `layout:`-bound
   topic (codec sees a bounded payload view; framing is the layout's job).

---

## Prior art

- **Aeron** — shared-memory log buffer with a documented layout + many
  language clients; the canonical "ring as a published wire ABI."
- **LMAX Disruptor** — the SPMC ring-buffer + published-sequence pattern
  both `LRSRNG1` and `RINGFMT1` are instances of.
- **SBE / Cap'n Proto / FlatBuffers** — schema → generated zero-copy
  typed accessors that index straight into a buffer (the Proposal-A′
  layer).
- **`bindgen` / `#[repr(C)]`, `zerocopy`, `bytemuck`** — declare a
  foreign POD layout at compile time → safe field access. Direct
  precedent for "foreign layout, compile-time, memory-safe."
- **`svd2rust`** — generates safe typed accessors for memory-mapped
  hardware registers from a description — i.e., a *foreign, fixed memory
  layout you don't control*, accessed safely via codegen. The closest
  analog to Proposal B.
- **Go `encoding/binary`, Rust `byteorder`, Python `struct`** — the
  Proposal-A pack-primitive surface.

---

## References

Hale:
- `crates/hale-codegen/runtime/lotus_shm_ring.c` — the SHM ring runtime
  (`LOTUS_SHM_RING_MAGIC` @73; header layout @86-108; `shm_open`+`mmap`
  attach/validate @136-211; acquire/release atomics in the publish/
  consume paths).
- `shm_ring(...)` binding + codec: `crates/hale-codegen/src/bus/dispatch.rs`
  (`codec_encode_override` ~297), `bus/wire.rs` (serializers),
  `shared/builtins.rs` (`lotus_bus_register_codec` ~1108), AST
  `CodecSpec` `crates/hale-syntax/src/ast.rs:549`, parse
  `parser.rs:1251`; tests `crates/hale-codegen/tests/shm_ring*.rs`
  (incl. cross-process `shm_ring_publish.rs`, `shm_ring_hale_subscriber.rs`).
- `std::bytes` today: `crates/hale-codegen/runtime/stdlib/bytes_builder.hl`
  + the `std::bytes::*` dispatch in `crates/hale-codegen/src/codegen.rs`
  (`at`/`slice`/`from_int`/`from_string`/`concat`/`clone`).
- `spec/ffi.md` — the alternative (explicit-unsafe) route this proposal
  supersedes for the declare-able case.
- `notes/proto-locus-design.md` — relevant to where `ring_layout` sits in
  the form/locus model.

The external stack (the driving consumer) — its shared-memory layer:
- the ring-layout header — the `RingPrefix` layout reproduced above.
- the frame-codec / ingress-ring / shared-memory headers — framing +
  producer/consumer.
- its short-types header — `u32`/`u64`/`i32`/`i64`.

---

## Coordination

fathom is the first consumer and will validate each stage against the external stack
(and a faithful mock, the same way it de-risked its grease UDP
integration with a loopback before live wiring). Sequencing that unblocks
fathom fastest: **Proposal A readers first** (immediately useful for any
binary codec), then **Proposal B read-only / `byte_records`** (reads
the external stack's ring). Producer + zero-copy-write + `LotusRing` dogfood can
follow.
