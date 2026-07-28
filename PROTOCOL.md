# iris observation protocol — v0 DRAFT

The shared-memory contract between an **emitter** (a Hale
process being observed — native runtime probes eventually,
the `observe` library emitter and the synthetic emitter
first) and a **consumer** (iris, `hale top`-style harnesses,
dump tooling).

Status: **DRAFT — not frozen.** Field widths and table sizes
are v0 working values; freeze happens after M1 measurement
(DESIGN.md §15), then the version byte governs forever.
iris owns this document (DESIGN.md §13); upstream native
emission implements it, not the other way around.

Design invariants the protocol must never violate:

- **One-way glass.** Consumers never block, slow, or corrupt
  the emitter. All consumer writes are confined to the
  control region (§5). Everything else is read-only by
  contract.
- **Never backpressure.** Rings overwrite oldest; drops are
  counted, not prevented.
- **Fixed-width records.** One slot size. No variable-width
  records, ever (version bump required to change this).
- **Self-describing segments.** A segment + the manifest it
  carries is decodable with no external files. `.hale.topo`
  (when it exists upstream) *populates* the manifest; it is
  not required to *read* one.

---

## 1. Discovery & registration

An emitter with observation enabled:

1. Creates the segment via `shm_open` under the name
   `/hale-obs-<pid>` (or `memfd` + registration fd-path on
   platforms without POSIX shm; v0 targets Linux, POSIX shm).
2. Atomically writes a registration file
   `$XDG_RUNTIME_DIR/hale/<pid>.json`
   (`/run/user/<uid>/hale/<pid>.json`), containing:

   ```json
   {
     "proto": "0.1",
     "pid": 1234,
     "exe": "/path/to/binary",
     "shm": "/hale-obs-1234",
     "started_mono_ns": 123456789,
     "started_wall_ns": 1753...,
     "rings": 8
   }
   ```

3. Removes the file on clean shutdown. Consumers detect
   stale registrations by pid liveness; stale files may be
   garbage-collected by any consumer.

When `$XDG_RUNTIME_DIR` is unset, the fallback directory is
`/tmp/hale-obs`. *(Amended.)*

Consumers watch the directory (inotify) and may also attach
manually by pid. Multiple consumers may attach to one
segment concurrently (see §9 — consumer state is
consumer-side, so N readers are free).

**Reference implementation:** `emitter/protocol.h` is the
executable form of this document (layouts pinned by
static_asserts), exercised end-to-end by `emitter/synth.c`
(emitter) and `emitter/peek.c` (consumer seed). Where header
and document disagree, that is a bug: fix both in one
commit.

## 2. Segment layout

All offsets are from segment base; all regions are
page-aligned. Layout is fixed at creation; the header
carries every offset, so consumers never compute layout.

```
+--------------------------+  0
| Header          (1 page) |
+--------------------------+  hdr.control_off
| Control region  (1 page) |   <- ONLY consumer-writable region
+--------------------------+  hdr.manifest_off
| Manifest        (n pages)|
+--------------------------+  hdr.modemask_off
| Mode mask       (n pages)|
+--------------------------+  hdr.counters_off
| Counter table   (n pages)|
+--------------------------+  hdr.rings_off
| Ring descriptors         |
| Ring 0 data              |
| ...                      |
| Ring R-1 data            |
+--------------------------+  hdr.total_len
```

## 3. Header

All integer fields little-endian; the magic doubles as an
endianness check.

| Offset | Field | Type | Notes |
|---|---|---|---|
| 0x00 | magic | u64 | `0x4F42534948414C45` ("HALEISBO" LE; reads "HALE-OBS") |
| 0x08 | proto_major | u16 | breaking changes |
| 0x0A | proto_minor | u16 | additive changes |
| 0x0C | header_len | u32 | |
| 0x10 | total_len | u64 | segment size |
| 0x18 | pid | u32 | |
| 0x1C | ring_count | u32 | R |
| 0x20 | ring_slots | u32 | slots per ring, power of two |
| 0x24 | ts_shift | u32 | ts_delta unit = 2^ts_shift ns |
| 0x28 | started_mono_ns | u64 | CLOCK_MONOTONIC at segment create |
| 0x30 | started_wall_ns | u64 | CLOCK_REALTIME anchor (wall-clock mapping) |
| 0x38 | control_off | u64 | |
| 0x40 | manifest_off | u64 | |
| 0x48 | manifest_len | u64 | |
| 0x50 | modemask_off | u64 | |
| 0x58 | counters_off | u64 | |
| 0x60 | counters_len | u64 | |
| 0x68 | rings_off | u64 | |
| 0x70 | flags | u64 | bit 0: emitter alive; bit 1: post-mortem dump |
| 0x78 | manifest_gen | u64 | seqlock-style generation (§4) |

Header is written once before registration (except `flags`
and `manifest_gen`); consumers treat it as immutable after
attach apart from those two fields.

## 4. Manifest

The decoder ring. Packed records carry integer ids; the
manifest maps ids to names, kinds, and shapes. Emitters MUST
publish a manifest entry before the first record referencing
its id.

Layout: manifest header, then `entry_cap` entries, then
string pool to the end of the region. *(Amended from "entry
table + string pool" by the v0 reference implementation —
consumers need a count and pool location.)*

```
ManifestHdr (16 B):
  entry_count  u32 (atomic; release on append)
  entry_cap    u32
  pool_off     u32     // from manifest_off
  pool_used    u32 (atomic)
```

Entry field order is chosen for natural alignment (u64s
first; 30 B of fields + 2 B pad). *(Amended: the original
doc order packed to 30 B with misaligned u64s.)*

```
ManifestEntry (32 B):
  shape_hash   u64     // payload-shape content hash (topics); 0 otherwise
  aux_b        u64     // binding: owning topic_id; scheduler: cpu index
  id           u32     // per-kind id space (§7)
  name_off     u32     // into string pool
  name_len     u16
  aux_a        u16     // binding: transport enum (unix/udp/tcp/...)
  kind         u8      // 0=topic, 1=locus_type, 2=binding, 3=scheduler
  flags        u8      // bit 0: networked (topics); bit 1: from .hale.topo
  _pad         u16     // zero
```

- **Topic identity across binaries** (fusion join key):
  `shape_hash` = content hash of the qualified topic name +
  payload shape. Two emitters built from the same
  declaration agree; iris fuses on (name, shape_hash).
- **Dynamic registration.** The native emitter writes the
  manifest once at startup (from `.hale.topo`). The library
  emitter learns topics at runtime and appends. Append
  protocol: write the entry and strings, then increment
  `manifest_gen` (release). Consumers re-scan on generation
  change (acquire). Entries are never mutated or removed.
- Pool exhaustion: if the manifest region fills, further
  registrations set a `manifest_overflow` counter (§6);
  records with unknown ids render as `unknown:<id>`.

## 5. Control region — the only consumer-writable page

| Field | Type | Notes |
|---|---|---|
| observer_count | u32 | atomic; incremented on attach, decremented on detach; 0 = dormant (probes skip all ring writes; counters-only) |
| sample_n | u32 | 1-in-N for SAMPLED-RICH mode |
| reserved | — | rest of page zero |

The mode mask (§7) is also consumer-written but lives in its
own region for size. Emitters read control fields relaxed;
staleness of a few events is acceptable by design.

Consumers MUST map all regions except control + mode mask
read-only (`PROT_READ`), and SHOULD map those two as the
only writable views. The emitter never trusts consumer
writes beyond these regions (defensive bounds on every read
of consumer-writable fields).

## 6. Counter table

Line 0 is a global line; then one cache-line (64 B) of u64
counters per manifest entry of kind topic or binding, in
manifest entry order counting only those kinds (index map
derived from manifest scan). A **late-registered**
topic/binding takes the next line index at registration time
— i.e. line index is fixed by its position among
topic/binding entries, exactly as a startup registration
would be. *(Amended: global-line-first and the late-entry
rule made explicit by the v0 implementation.)*

```
topics:    published, delivered, bytes, dropped_records
bindings:  sent, delivered, bytes, queue_depth (gauge),
           send_block_ns, retries, seq_high_water
```

Plus one global line: `ring_drops_total`,
`manifest_overflow`, `records_total`. All plain atomic
increments/stores from the emitter, relaxed loads from
consumers. Gauges (queue_depth) are last-write-wins.

## 7. Id spaces & mode mask

- `topic_id`, `locus_type_id`: u20 (record-packed), assigned
  by the emitter (native: from `.hale.topo` ordering;
  library: registration order). **Id spaces are per-kind**
  (topic 3 and locus-type 3 are unrelated; records disambiguate
  by ekind). Ids are per-segment; only (name, shape_hash) is
  stable across processes. Locus *instance* ids are a further
  dynamic space seeded by LOCUS_BIRTH records.
- **Mode mask**: one byte per topic_id, `modemask_off +
  topic_id`. The region is sized to the id *capacity*
  (`entry_cap`), not the full u20 space — consumers bound-check
  topic ids against the region length. *(Amended.)* Values:

  ```
  0 OFF | 1 COUNTERS | 2 PACKED (default) | 3 SAMPLED-RICH | 4 FIREHOSE
  ```

  Consumer-written, emitter-read (relaxed, per publish; MAY
  cache and refresh on manifest_gen or every K events).
  FIREHOSE SHOULD be set on at most one topic; emitters MAY
  clamp.

## 8. Records

**One slot size: 16 bytes.** Two words.

```
word0 (u64):  id:20 | ekind:5 | size_class:8 | ts_delta:31
word1 (u64):  ekind-dependent (seq, aux, full timestamp, ...)
```

- `id`: topic_id or locus-instance id per ekind.
- `size_class`: log2 bucket of payload bytes (0 = n/a).
- `ts_delta`: units of 2^ts_shift ns since this ring's last
  EPOCH record. On overflow the emitter MUST emit EPOCH.
- `ekind` (5 bits, 32 kinds; v0 assignment):

  ```
  0  EPOCH            word1 = full CLOCK_MONOTONIC ns
  1  BUS_PUBLISH      word1 = locus:20 | seq:44 (publisher instance;
                      0 = unattributed — library emitters without
                      locus context). Amended 2026-07-27: locus
                      attribution packed into the high bits so
                      consumers can render per-locus activity
                      (which petal is emitting/reacting); mirrors
                      NET_SEND's binding:16|seq:48 packing. 44-bit
                      in-process seq wraps at ~17T msgs/topic.
  2  BUS_DELIVER      word1 = locus:20 | seq:44 (consumer instance;
                      same packing and 0-means-unattributed rule)
  3  NET_SEND         word1 = origin_id:16 | seq:48
  4  NET_DELIVER      word1 = origin_id:16 | seq:48
                      Amended 2026-07-27 (field finding): the
                      pair is the SENDER's — origin_id is the
                      sending process's binding/stream id and
                      seq its per-(origin, subject) send
                      counter, carried ON THE WIRE and echoed
                      verbatim by the receiver's NET_DELIVER.
                      A receiver-local receive counter can
                      never match under multicast (N senders
                      sum into one count) and cannot show
                      loss; the wire seq's gaps ARE the loss.
                      Consumers match deliveries to sends on
                      (topic, origin_id, seq).
  5  LOCUS_BIRTH      id = instance; word1 = parent:32 | type:20
  6  LOCUS_DISSOLVE   id = instance; word1 = reason enum
  7  RESTART          id = instance; word1 = attempt:16 | policy:8
  8  SUPERV_TRANS     id = instance; word1 = transition enum (on_failure, absorb, escalate, ...)
  9  PLACEMENT        id = instance; word1 = scheduler/cpu
  10 ARENA_MARK       id = instance; word1 = bytes high-water
  11 BINDING_UP       id = binding
  12 BINDING_DOWN     id = binding; word1 = errno/reason
  13 SAMPLE_RICH      header slot; followed by 1 extra slot of payload metadata
  14 DROP_MARK        word1 = records dropped since last mark
  15 CONT             continuation slot (see SAMPLE_RICH)
  16 LOCUS_ENTER      id = instance; word1 = trigger topic_id:20 | seq-low:44   [reserved, not emitted in v0]
  17 LOCUS_EXIT       id = instance; word1 = 0                                  [reserved, not emitted in v0]
  18–31 reserved
  ```

  SAMPLE_RICH is the one two-slot form: a header slot + one
  continuation slot (latency ns, payload bytes exact). The
  continuation slot's word0 has `ekind=15` (CONT) so a
  consumer landing mid-stream can resynchronize by skipping
  CONT slots. No other multi-slot forms; adding one is a
  minor-version event.

- Locus **instance ids**: u20 per-segment monotonic
  allocation, tied to LOCUS_BIRTH; wraparound emits a
  generation EPOCH note (open item, §13).

## 9. Rings

One SPSC ring per scheduler. Descriptor array at
`rings_off` (one per ring, 64 B aligned):

```
RingDesc:
  data_off      u64    // slots array, ring_slots * 16 B
  head          u64    // producer write cursor, monotonic, never wraps
  dropped       u64    // overwritten-unread estimate is consumer-side; this counts emit-side drops (OFF-mode races etc.)
  sched_id      u32
  current_locus u32    // gauge: locus instance id now running on this scheduler; 0 = idle
```

`tag_b` (current_locus) is the **external-sampling join**: the
emitter stores it (relaxed) at every locus switch — cheap and
exact under cooperative scheduling. A consumer running a
sampling profiler (perf_event_open; sample data never enters
the segment) joins `(tid → sched_id, timestamp)` against this
gauge to attribute native callstacks to loci (DESIGN §10).
Consumers read it relaxed; a sample landing exactly on a
switch may misattribute by one event — acceptable for
sampling by construction. Span-exact attribution is the
LOCUS_ENTER/EXIT upgrade path (§8, reserved).

- **Producer:** issue a **release fence**, write both words
  of slot `head & (ring_slots-1)`, then publish `head+1`
  with a release store. The pre-write fence is the producer
  half of the Boehm-seqlock pair (finding 3 below); it is a
  compiler barrier on x86 and one `dmb ish` on ARM. No CAS,
  no consumer interaction. If the ring is "full" it simply
  keeps writing — overwrite-oldest is the cursor arithmetic,
  not a branch.
- **Consumer:** keeps its own cursor `c` (consumer-side, not
  in shm — this is what makes N concurrent observers free).
  Snapshot algorithm:

  1. `h1 = load-acquire(head)`; if `c <= h1 - ring_slots`,
     fast-forward `c` to `h1 - ring_slots + 1`, counting the
     full jump as overrun.
  2. copy slots `[c, min(h1, c + batch))`
  3. **acquire fence** (the consumer half of the seqlock
     pair — REQUIRED of every external reader)
  4. `h2 = load-acquire(head)`
  5. any copied slot with index `<= h2 - ring_slots` was
     possibly overwritten or is being clobbered by the
     in-flight record → discard, advance `c` to
     `h2 - ring_slots + 1`, and count the **entire cursor
     advance** (including any never-copied gap) as overrun
     (rendered, per DESIGN §4).
  6. decode the survivors; merge across rings by
     reconstructed timestamp.

  Three corrections here came out of the shared primitive's
  GenMC verification (hale#244 thread, hale#247): the live
  window given published head `h` is `(h - ring_slots, h]` —
  the producer's in-flight record `h` already clobbers index
  `h - ring_slots`, so the boundary is `<=`, not `<`;
  overrun accounting must cover the full cursor advance or
  `delivered + overruns` undercounts; and the fenceless
  h1/copy/h2 validation is formally unsound (a relaxed slot
  load may read a future record's write while h2 reads
  stale — masked on TSO/x86, live on ARM). The fence pair
  guarantees observing any of record `h`'s bytes forces
  `h2 >= h`.

- Word-tearing: slots are two u64 plain stores; with the
  fence pair, a consumer can observe mixed/half-written
  words only at indices the `<=` discard window rejects.
  Model-checked, not argued: `hale/verification/
  spsc_ring_model.c` (genmc CI) asserts no *delivered*
  record is torn and delivered seqs strictly increase.

## 10. Memory ordering summary

| Site | Ordering |
|---|---|
| producer: fence → slot words → head publish | release fence before slot writes; release store on head |
| consumer: copy → fence → h2 read | acquire fence between slot copy and head re-read (REQUIRED of external readers) |
| consumer head read | acquire |
| manifest entry → manifest_gen | release inc |
| consumer manifest_gen read | acquire, then re-scan |
| counters | relaxed (monotonic; gauges last-write-wins) |
| current_locus (emitter store / consumer load) | relaxed (gauge; sampling tolerates one-event skew) |
| control/mode reads (emitter) | relaxed |

## 11. Verification obligation

The producer/consumer ring (including the overwrite window
and the two-word slot), the manifest seqlock, and the
mode-mask read races get GenMC models in the style of
`hale/verification/` before the freeze. The models live in
this repo (`verification/`) since the protocol is ours; the
hale team inherits them with the native-emitter ask.

## 12. Post-mortem dumps

A dump is the segment written to a file verbatim, prefixed
with a 16-byte dump header (`magic "HALEDUMP"`, u32 version,
u32 reserved), with header `flags` bit 1 set. Consumers open
dumps with the identical decode path — live attach and
forensics are one code path by construction (DESIGN §9).
The runtime's crash handler (upstream, eventually) or the
library emitter's abort hook (ours, M3) produces them.

## 13. Open items before freeze

- ts_delta 31 bits @ ts_shift=4 (16 ns units) ≈ 34 s between
  forced EPOCHs — confirm acceptable EPOCH overhead at
  realistic rates; tune shift.
- Locus instance-id width (u20 = 1M births per segment
  lifetime) and wraparound story.
- Ring sizing defaults (slots per scheduler) and whether
  FIREHOSE gets a dedicated ring.
- Exact shape_hash definition — needs the qualified-name +
  payload-shape canonicalization pinned down with the hale
  team so native and library emitters agree.
- Whether `sample_n` is global (current) or per-topic
  (another mode-mask-sized table).
- macOS: POSIX shm name limits and `memfd`-equivalent —
  parked until the platform work upstream settles (#231).
- Symbol demangling scheme for native-stack attribution
  (`__lib_..._Type` pattern) — needs the mangling rules
  documented by the hale team; consumer-side otherwise.
- Whether LOCUS_ENTER/EXIT (ekinds 16/17) ship with v0
  emitters or arrive as a minor-version upgrade once
  sampled flamegraphs prove insufficient.
- ~~Ring convergence (hale#244)~~ **RESOLVED (hale#247):**
  the runtime now ships the SPSC observation ring as a lotus
  primitive using this protocol's descriptor (with tag_a/
  tag_b generalization); §9 adopts it verbatim. Emitters use
  `std::ring::__spsc_*` from pure Hale; the three
  verification-found corrections are folded into §9/§10.
