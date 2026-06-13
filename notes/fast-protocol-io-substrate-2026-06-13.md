# Fast protocol I/O belongs in the substrate, not the library

**Date:** 2026-06-13. **Status:** **SHIPPED 2026-06-13.** Every item below
landed; this note is kept as the design record. Per item:

| # | Item | PR |
|---|---|---|
| 1 | `recv_stamped_into` — kernel RX timestamps (one recvmsg) | #138 |
| 2 | `TCP_NODELAY` + tcp sockopt surface | #135 |
| 3 | `std::io::MirrorRing` — double-mmap wrap-free buffer (+ `BytesMut` raw-window reads) | #141 |
| 4 | `std::bytes::find_byte` + block-XOR masking | #139 |
| 5 | `ring_layout` `record_header` + `post_copy` recheck | #140 |
| 5+ | in-band header-field delivery (`std::shm::last_record_*`) | #143 |
| 6 | recv plaintext-alloc audit + zero-alloc gate | #142 |
| 7 | alloc/syscall gate counters (`std::diag`) | #137 |

The original scoping proposal follows unchanged below (it still describes
the design rationale; the syntax that shipped differs in spots — e.g. the
`record_header` knobs landed as flat `byte_records` framing attributes, and
header fields are surfaced via thread-local getters rather than a
payload-struct prefix; see `spec/` for the shipped surface).

**Provenance.** Prompted by a teardown of `sorcery-labs/ws-fast` (MIT, Rust,
~6.9k LOC) — a sync, caller-driven WebSocket *ingress* crate for HFT
market-data. We compared it against `pond/websocket` to ask "can we beat it?"
The useful answer turned out to be a substrate question, not a library one,
and it generalizes well past WebSocket. Credit to ws-fast for the design; the
techniques below are largely theirs, recast as Hale substrate work.

---

## Thesis

When you decompose where a state-of-the-art latency-focused protocol stack
gets its speed, **very little of it is protocol-specific cleverness.** Most of
it is I/O-floor capability — syscall discipline, kernel timestamps, a
wrap-free receive buffer, bounded TLS plaintext delivery, a shared-memory
fan-out ring, fast byte primitives. Those are exactly the things a *substrate*
should own. If they live in `std::*` + codegen + runtime, then `pond/http`,
`pond/pq`, `pond/websocket`, and every future protocol get them for free, and
the libraries stay thin — which is the whole Hale bet.

A speed story that lives in a library (the ws-fast model) is the anti-Hale
outcome. The Hale outcome is a fast substrate that makes the library trivial.

This note enumerates the gaps, sketches the stdlib shape and the codegen/
runtime work each implies, ranks them by leverage ÷ cost, and — importantly —
states the **non-goals**, because "make all protocols fast" overpromises in
two specific places.

---

## Where ws-fast's speed actually comes from

Categorized as **(A)** substrate gap absent from Hale today, **(B)** a
codegen/runtime property Hale's model targets but must be shown to achieve, or
**(C)** genuine library policy that should stay in the library.

| Technique | Cat. | Generalizes to all protocols? |
|---|---|---|
| One `recvmsg`/poll + kernel RX timestamps (`SO_TIMESTAMPNS`/`SCM_TIMESTAMPNS`) | A | Yes — pure stdlib add |
| Mirror ring: `memfd_create` + 2×`MAP_FIXED` so a wrapped record is one contiguous slice | A | Yes, as an **opt-in** buffer |
| Bounded / zero-alloc TLS plaintext delivery | A | Yes — every TLS protocol |
| `TCP_NODELAY`, nonblocking connect, `MSG_NOSIGNAL` | A | Yes — every TCP protocol |
| Block / word-at-a-time XOR masking; word-scan length/delimiter decode | A | Masking is WS-ish; word-scan helps all parsers |
| SPMC shm ring + lap detection | A (Hale *has* it) | Yes, modulo the record-header gap (§5) |
| Zero steady-state allocation (arena scratch + reused buffers) | B | Yes — `pond/websocket` recv is *already* zero-alloc |
| Branch-light single-pass frame scanner; one `PongSlot`; cork = one TLS record | C | No — this part is correctly library code |

Confirmed against the current stdlib surface (`spec/stdlib.md`,
`crates/hale-codegen/src/stdlib/`): there is **no** `recvmsg`, no
`SO_TIMESTAMPING`/`SCM_*`, no `mmap`/mirror primitive, and not even a
`TCP_NODELAY` constant in `std::io::sockopt` (you cannot disable Nagle from
Hale today). The documented recv pattern is already a reused `BytesBuilder`
accumulator (`spec/stdlib.md` § "Builders vs Bytes"), so the zero-alloc
property (B) is real when code is written to the grain — the lever there is
codegen maturity + a way to *prove* it (§7), not a new primitive.

Only the bottom row of the table is library work, and it is small. Everything
above it is substrate.

---

## The proposal — seven items, ranked by leverage ÷ cost

### 1. `std::io::tcp::recv_stamped_into` — kernel RX timestamps  ·  HIGH leverage, LOW cost

One `recvmsg(2)` that captures the kernel's `SCM_TIMESTAMPNS` (Linux) and a
userspace stamp, on top of the existing `recv_into` destination. Multi-value
return is the only wrinkle; follow the **existing** errno-style thread-local
idiom already used by `std::io::udp::recv_with_source` /
`last_source_host()` — don't invent tuple returns.

```hale
// recv into the builder as today; populate a thread-local stamp cache.
std::io::tcp::recv_stamped_into(fd: Int, buf: BytesBuilder, max: Int) -> Int fallible(IoError);
std::io::tcp::last_recv_kernel_ns() -> Int;   // 0 when the platform delivered none (macOS TCP)
std::io::tcp::last_recv_user_ns()   -> Int;   // clock_gettime(CLOCK_REALTIME) at recvmsg return
```

Runtime: a `lotus_tcp_recv_stamped` that builds the `msghdr`/cmsg buffer, does
one `recvmsg`, and walks control messages **defensively** (refuse zero-length
or short cmsg headers; do not use `CMSG_NXTHDR` macros — some libcs infinite-
loop on a zero-length cmsg; ws-fast's `socket.rs` learned this). One setsockopt
(`SO_TIMESTAMPNS`) at socket setup. Benefits: every protocol gets wire-arrival
time with no extra syscall. This is the first thing to ship.

### 2. Socket-option completeness, starting with `TCP_NODELAY`  ·  HIGH leverage, TRIVIAL cost

`std::io::tcp` has no `set_option_*` at all (only `std::io::udp` does) and
`sockopt` has no `TCP_NODELAY`. Add the constant and a `std::io::tcp::set_nodelay(fd, on: Bool)`
(or extend the generic `set_option_int` to TCP). Nagle on a request/response
or market-data socket is a latency killer; not being able to turn it off is an
embarrassing gap. While here: `SO_BUSY_POLL` (Linux) is worth a constant for
the spin-poll crowd.

### 3. `std::io::MirrorRing` — wrap-free receive buffer (OPT-IN)  ·  HIGH leverage, MEDIUM cost

The double-mmap "magic ring": reserve `2*cap` `PROT_NONE`, then map one
`memfd`/`shm` object twice at offsets `0` and `cap` with `MAP_FIXED`, so any
window ≤ `cap` that straddles the physical seam reads as one contiguous slice.
A parser never copies or special-cases the wrap.

```hale
locus MirrorRing { params { capacity: Int; } }   // capacity: power of two, page multiple
// readable()/writable() hand out BytesView/BytesMut over the live ring;
// commit(n)/consume(n) advance the cursors. Views obey the existing F.30
// view-lifetime rule: valid until the next mutation. recv_into can target it.
```

Runtime: `lotus_mirror_ring_*` (`memfd_create`+`ftruncate`+reservation+2×
`MAP_FIXED`; `shm_open`+`shm_unlink` on macOS). Codegen: `BytesView`/`BytesMut`
coercion at the slice boundary, reusing the StringView/BytesView machinery
that already exists. **Opt-in only** — see non-goals; this is wrong as a
default recv buffer.

### 4. `std::bytes` masked-XOR + word-scan primitives  ·  MEDIUM leverage, LOW cost

```hale
std::bytes::xor_mask_into(dst: BytesBuilder, src: BytesView, key32: Int) -> ();  // 4-byte key, word-at-a-time
std::bytes::find_byte(b: BytesView, off: Int, needle: Int) -> Int;               // word-at-a-time scan, -1 if absent
```

These lower to vectorized loops. Codegen already inlines leaf byte primitives
(the JSON SIMD pass, `range_eq`, `byte_at_unchecked`), so this is the same
shape of work. WebSocket masking is the obvious consumer; `find_byte` helps
every length/delimiter-framed parser (HTTP header CRLF, etc.).

### 5. `ring_layout`: a per-record header + post-copy lap re-check  ·  MEDIUM leverage, MEDIUM cost

`ring_layout` (Proposal B) already consumes foreign SPMC rings — and ws-fast's
bus (`ws_fast_bus.h`) is exactly such a ring with a published C ABI. But it
**cannot be consumed today**, for two precise reasons (byte-level detail in the
appendix):

- `byte_records` assumes the only per-record overhead is the length prefix:
  stride = `align_up(len_prefix + len, align)`. ws-fast's record header is
  **32 bytes** (`len, kind, opcode, seq, kernel_ns, user_ns`), so its stride is
  `32 + align8(len)`. A Hale reader computes the wrong stride and desyncs after
  one record, and has no way to read `seq`/timestamps.
- Hale's `lap_detect` (per `spec/semantics.md` ~L1140) is a **pre-read** resync.
  ws-fast's correctness rests on a **post-copy** re-check (copy → acquire fence
  → reload cursor → re-verify the window still exceeds the stride) that proves
  the copied bytes were not clobbered *during* the copy by the free-running
  writer. Without it, a fast foreign producer can hand a torn record to the
  handler.

Proposed: a `record_header N { name at off : repr; ... }` block inside
`framing byte_records` (reusing the segment-header scalar mechanism), so stride
= `record_header_bytes + align(len)` and the named fields are delivered
alongside the payload view; plus an `overflow lap_detect { recheck post_copy; }`
knob. This single change is where "consume ws-fast" and "fix the substrate"
become the *same* work — and it also enriches Hale's own producer side.

### 6. Audit + bound `std::io::tls` plaintext allocation  ·  MEDIUM leverage, MEDIUM–HIGH cost

Every TLS protocol pays whatever `std::io::tls`'s OpenSSL binding allocates per
record. ws-fast measured rustls at exactly one alloc + one copy per app-data
record and *pinned it with a test*. We should measure `lotus_tls.c`'s behavior,
drive `recv_into` toward bounded/zero per-record allocation, and gate it (§7).
`SSL_MODE_RELEASE_BUFFERS` is already set (good); the open question is the
per-record plaintext copy.

### 7. Test-time guarantees: the rigor that *keeps* the wins  ·  MEDIUM leverage, MEDIUM cost

The wins above regress silently without gates. ws-fast's real moat is its test
harness, and the disciplines port even if the code doesn't:

- **Allocation gate** — a global-allocator shim that counts alloc/dealloc, with
  a `window { ... }` helper asserting *zero* (or an exact pinned count) inside
  a steady-state region. Hale has `--warn-unbounded-alloc` at compile time;
  this is the runtime/test-time complement ("this loop did zero allocs").
- **Syscall gate** — assert "exactly one `recvmsg` per poll" (ws-fast re-execs
  under `strace -c -e trace=recvmsg`; an interposed counter works too).
- **Conformance-as-pinned-regression** — adopt ws-fast's autobahn rule: pin the
  per-case verdict, and treat a *new pass* as a regression-until-justified, not
  just a new failure. It caught two real bugs for them.

---

## Non-goals (where "all protocols get the benefit" overpromises)

1. **The mirror ring is not the default recv buffer.** Double-mapping every
   socket costs 2× address space + `memfd_create`+`ftruncate`+3×`mmap` *per
   connection*. Fine for one HFT session; VMA-pressure suicide for a
   100k-connection server. Ship it opt-in (§3); leave the `BytesBuilder`
   accumulator as the default.

2. **We are not chasing hand-tuned-Rust ns-per-syscall on a single socket.**
   Every Hale syscall crosses the managed FFI boundary with arena-snapshot
   semantics; ws-fast's `recvmsg` is a direct libc call. Substrate work shrinks
   the per-call overhead but does not erase it. And kernel timestamps measure
   wire arrival — if a cooperative pool then schedules the handler behind other
   work, end-to-end latency includes jitter the stamp can't see. The target is
   "fast enough that the protocol library is thin," not "win the one-socket
   microbench." (For genuine HFT, the answer is a `pinned` pool owning the poll
   loop — which Hale already supports — not a faster cooperative default.)

3. **The substrate ships primitives, not per-protocol fast paths.** No
   `recvmsg`-for-WebSocket. The parse hot loop stays in the library and is
   written to the grain — and a *general* parser (fragmentation, HTTP/2,
   compression negotiation) inherently pays more branches than a deliberately
   narrow one like ws-fast's. The I/O floor generalizes cleanly; the parse loop
   generalizes only as *primitives*.

4. **No hidden async runtime, no compression in the hot path** — these are
   ws-fast's non-goals too, and for the same reason (zero-copy + predictability).

---

## Sequencing

1. **#1 (`recv_stamped`) and #2 (`TCP_NODELAY`/sockopt)** — additive, cheap, no
   API disruption, immediate value to `pond/http` + `pond/pq` +
   `pond/websocket`. Land first.
2. **#4 (byte primitives)** — cheap, isolated.
3. **#3 (MirrorRing) and #5 (ring_layout record header)** — the two structural
   items; #5 unblocks foreign-ring interop (incl. consuming ws-fast).
4. **#6 (TLS audit) and #7 (gates)** — ongoing; #7 should land alongside #1 so
   the first wins are pinned from day one.

## How we'd know it worked

- A `fast echo` microbench in `experiments/` reporting ns/visit and allocs/
  visit, run on a pinned host.
- The §7 gates green on `std::io` + `pond/websocket`.
- `pond/websocket` and `pond/http` rebuilt on the new primitives, showing the
  recv-path win — and `pond/websocket`'s known send-path gaps closed in the
  same pass (reuse a cork buffer instead of a fresh `BytesBuilder` per frame;
  pre-buffer `getrandom` entropy instead of one syscall per frame; block-XOR
  masking via #4 instead of byte-by-byte `from_int`).
- A `ring_layout WsFastBus` (appendix) consuming a live ws-fast segment
  zero-copy with in-band kernel timestamps — the concrete interop proof.

---

## Appendix — the exact `ring_layout` gap vs `ws_fast_bus.h`

**ws-fast segment** (little-endian, 64-bit): `magic@0` (`"WSFBUS01"` =
`0x5753464255533031`, written last at init), `version@8:u32`, `capacity@16:u64`
(power of two), `generation@24:u64`, `write_cursor@128` (atomic u64, *alone on
its cache line* — the 128-byte isolation matters on both 64 B x86 and 128 B
Apple lines), `data@256`.

**ws-fast record** (8-byte aligned, never wraps; stride = `32 + ((len+7)&~7)`):

| off | field | repr | note |
|---|---|---|---|
| 0  | `len`       | u32 | payload bytes, pad excluded |
| 4  | `kind`      | u8  | 0 Data, 1 Padding (skip), 2–4 reserved |
| 5  | `opcode`    | u8  | WS opcode: 1 Text, 2 Binary |
| 6  | `_pad`      | u16 | |
| 8  | `seq`       | u64 | `(generation << 48) \| counter`; generation change = splice marker |
| 16 | `kernel_ns` | u64 | recvmsg kernel RX stamp, 0 if none |
| 24 | `user_ns`   | u64 | userspace stamp at recvmsg return |
| 32 | `payload`   |     | `(len+7)&~7` bytes |

**Today's `byte_records`** can express the *segment* (cursor offset is settable
→ put it at 128 for cache-line isolation ✓; `buffer_size` scalar = capacity ✓;
`data_at 256` ✓) but **not** the record: it models overhead as a bare
`len_prefix u32`, giving stride `align_up(4 + len, 8)` ≠ `32 + align8(len)`, and
offers no way to surface `seq`/`kernel_ns`/`user_ns`.

**Sketch of the extension** (illustrative — names/spelling for the team to
settle):

```hale
ring_layout WsFastBus {
    magic 0x5753464255533031;
    version    1  at 8  : u32;
    buffer_size   at 16 : u64;            // capacity
    data_at 256;
    cursor published { at 128; repr atomic_u64; load acquire; unit bytes; }

    framing byte_records {
        record_header 32 {                // NEW: fixed per-record header + named fields
            len       at 0  : u32;        // the framing length field (payload bytes)
            kind      at 4  : u8;
            opcode    at 5  : u8;
            seq       at 8  : u64;
            kernel_ns at 16 : u64;
            user_ns   at 24 : u64;
        }
        align 8;
        pad_when kind == 1;               // NEW: padding marked by a header field
                                          // (hale-native producers may keep pad_sentinel on len instead)
    }
    overflow lap_detect { recheck post_copy; };   // NEW: copy → acquire fence → reload cursor → re-verify
}
```

Delivered-record access: the handler receives the payload `BytesView` plus
generated accessors for the named header fields (`rec.seq()`, `rec.kernel_ns()`,
`rec.opcode()`), so a Hale market-data consumer reads ws-fast's output
zero-copy *with* wire-arrival timestamps and never makes a syscall to get them.

References: `ws_fast_bus.h` (the ABI + the literate reader proof),
`src/bus.rs` (Rust side + layout test against the header), `src/ring.rs`
(MirrorRing), `src/socket.rs` (recvmsg + cmsg walk), `tests/{no_alloc,
syscall_gate,bus_layout}.rs` (the rigor bar).
