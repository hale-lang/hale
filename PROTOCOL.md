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
- **No payload bytes, ever.** Records carry ids, sequence
  numbers, timestamps and *size classes* — never message
  contents. The one rich form (SAMPLE_RICH, §8) carries a
  latency and an exact byte count, still not the bytes. This
  was always a rate-class decision; since hale shipped its
  secrets surface (`@sealed`, `std::secret`) it is also a
  confidentiality property worth stating outright: **enabling
  observation cannot exfiltrate a payload, sealed or
  otherwise**, because the plane has nowhere to put one.
  Sealing and observation are orthogonal upstream — the
  runtime's probes are unaware of it — and this invariant is
  why that is safe. Adding a payload-bearing record kind is
  therefore a protocol break, not a minor version.

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

**Liveness is the consumer's job, and it is pid liveness —
not the header flag.** `flags` bit 0 (§3) is set at segment
creation and cleared by the emitter's clean-shutdown path, so
a SIGKILLed, panicking or `docker stop`ped emitter leaves it
set forever. A consumer reading the flag alone renders dead
processes as live indefinitely (measured: 14 phantom
processes in one iris snapshot, all leftovers of an upstream
test run). The rule is the AND of both facts — flag set
**and** `kill(pid, 0)` not `ESRCH` — which also gives the
right answer for a post-mortem dump, whose pid has by
definition exited. Residual: pid reuse reads live until the
next attach cycle; discriminating it needs a process
start-time comparison against a clock the header does not
carry, and a wrong answer there would blind a *live* process,
so it is left open deliberately. *(Amended 2026-08-11;
reference implementation `consumer/obs_attach.c::obs_alive`.)*

Upstream complements this from the emitter side: since
v0.11.24 the next observed process to start **sweeps segments
and registrations belonging to dead pids** (it skips anything
alive — blinding a running observer would be worse than
leaving a file behind). That bounds the leak; it does not
remove the consumer's obligation, because a consumer may
attach long before any new emitter starts.

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
| 0x80 | model_hash | u64 | **proto ≥ 0.2.** Model identity (§3.1) |
| 0x88 | entity_id_digest | u64 | **proto ≥ 0.3.** Canonical entity-id table identity (§3.2) |

### 3.1 Model identity — `model_hash` *(added 0.2, 2026-08-12)*

The topology artifact's `shape_hash` — the identity of the model
this binary was **compiled from** — stamped at segment creation.
It settles the one question a consumer joining a live manifest to
a source-derived artifact cannot otherwise answer: *was the
running process built from the model I am comparing it against?*
A comment-only rebuild keeps the value; a model change moves it.

**Complementary to the manifest's per-topic `shape_hash`, not a
substitute.** Model identity deliberately excludes payload field
shape, so the two move independently and both are needed:

| edit | model_hash | topic shape_hash |
|---|---|---|
| add a field to a payload | unchanged | **moves** |
| add a locus, rewire, change supervision | **moves** | unchanged |

"In sync" means both agree. A consumer checking only one of them
is blind to half the drift — which is why iris's inspector
reports them as two axes rather than one verdict.

**Absent is not zero.** `0` is a real value meaning *this emitter
has no model* — a synthetic or non-Hale emitter, and the state
this repo's `synth.c` reports. A proto 0.1 segment carries no
field at all, which means *unknown*. Consumers MUST keep the two
distinct: check `proto_minor >= 2` **and** that `header_len`
covers the field before reading it, since `header_len` is what
the emitter actually wrote. (`consumer/obs_attach.c::obs_model_hash`
returns a presence flag rather than a value for exactly this
reason.)

Minor-version discipline, since this is the first exercise of it:
the field is purely additive, so a 0.1 consumer reads a 0.2
segment correctly by ignoring it, and every existing region
offset is unchanged. That is the bar for a minor bump; anything
that moves or reinterprets an existing field is a major.

Header is written once before registration (except `flags`
and `manifest_gen`); consumers treat it as immutable after
attach apart from those two fields.

### 3.2 Entity-id table identity — `entity_id_digest` *(added 0.3, 2026-08-24)*

The identity of the canonical **entity id table** whose ids a
segment's manifest rows may carry in `aux_b` (§4). `model_hash`
is *structural* model identity and does not cover every table
those ids index, so two builds can share a `model_hash` while
numbering entities differently. A consumer recomputes this digest
from the model it holds and uses the ids **only on a match**;
otherwise it falls back to matching on names. `0` = unstamped.

Same absent-is-not-zero rule as §3.1: a proto 0.2 segment carries
no field (unknown); a 0.3 segment carrying `0` is saying it has
no canonical ids to offer.

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
  aux_b        u64     // canonical model entity id (hale >= 0.3); 0 = none.
                       //   0.4: the ONLY meaning, for every emitter
  id           u32     // per-kind id space (§7)
  name_off     u32     // into string pool
  name_len     u16
  aux_a        u16     // binding: transport enum (unix/udp/tcp/...);
                       //   2 = adapter (Hale-owned-wire ingest,
                       //   registered lazily on first use)
                       // scheduler: cpu index (0.4; was aux_b)
  kind         u8      // 0=topic, 1=locus_type, 2=binding, 3=scheduler
  flags        u8      // bit 0: networked (topics); bit 1: from .hale.topo
  _pad         u16     // zero
```

- **`aux_b` is the canonical model entity id — resolved 2026-09-04
  at proto 0.4** (hale#525, handoff-14 P31). Since v0 this document
  and iris's reference emitters (`emitter/synth.c`, `observe/`) had
  used the field as *binding → owning topic_id, scheduler → cpu
  index*; hale's native emitter, from proto 0.3, wrote the canonical
  model entity id there for every kind, guarded by
  `entity_id_digest` (§3.2), with `0` meaning "no canonical id".
  The two were not distinguishable from the value alone.

  Resolution: v0's meaning is **retired**. Every emitter writes the
  entity id or 0; the scheduler cpu index moves to `aux_a`; the
  binding → topic pairing is dropped (the counter line and the
  binding name carry it). No layout change — which is exactly why
  it still needed a minor: the *meaning* moved, and a consumer
  depends on the meaning, not the offset.

  Consumer rule: use the ids only when `entity_id_digest != 0`
  matches the model you hold (§3.2). At `proto_minor >= 4` a
  nonzero `aux_b` was never anything else; a 0.3 segment from an
  iris emitter carries the old meaning and a zero digest, so the
  digest gate alone is correct for it too.

  How it happened is worth keeping: two implementations, one spec,
  and the spec was not consulted before claiming a v0 field.
  Upstream's rationale recorded the field as "written as 0 by every
  path", true of its own emitter and false of this document's.

- **Topic identity across binaries** (fusion join key):
  `shape_hash` = content hash of the wire subject + payload
  shape. Two emitters built from the same declaration agree;
  iris fuses on (name, shape_hash).

  **Exact definition** *(pinned with the hale team,
  hale#399; the reference implementation is
  `hale_types::topic_identity`, which both hale codegen and
  the topology artifact call; `lotus_obs.c::obs_fnv` is the
  emitter-side mirror)*:

  ```
  shape_hash = FNV-1a/64 over:
      wire_subject bytes, one ':' byte, shape bytes
    offset basis 0xcbf29ce484222325, prime 0x100000001b3

  wire_subject = parent-joined dot-path of declared
    `subject:` values, child-last; a topic without
    `subject:` contributes its declared name AS WRITTEN.
    Only explicitly declared subjects are stable across
    binaries — a name fallback carries the declaring
    binary's local (possibly mangled) spelling and will not
    fuse. Shared topics MUST declare `subject:`.

  shape = for a payload written as a bare, non-generic
    named struct: the struct's fields in declaration order
    as "<field>:<tag>" joined by ";". Tags:
      i  Int/Uint        f  Float       b  Bool
      d  Decimal         t  Time        u  Duration
      s  String/StringView
      y  Bytes/BytesView/BytesMut
      struct  anything else (nested structs deliberately
              name-free: the hash must never depend on a
              declaring binary's local type names)
    Any other payload form hashes the EMPTY shape (the ':'
    separator is still hashed).
  ```

  Test vectors (wire contract; breaking any is a protocol
  break):

  ```
  subject "Tasks", payload struct
    { id: Int; label: String; }:
      shape        "id:i;label:s"
      shape_hash   0xf7d174542aa33437
  subject "Tasks", empty shape:
      shape_hash   0xf3573379dcc4dcd5
  parented: topic Org (subject "org"), child topic
    Metrics : Org (subject "metrics"):
      wire_subject "org.metrics"
  ```

  The hashed subject is the JOINED wire subject — the same
  string publish-side manifest rows key by. *(Amended: the
  original native emitter registered shapes under the
  unjoined declared subject, so a parented topic's manifest
  row hashed the empty shape; fixed with the pinning.)*

  The compiler-side topology artifact exports each topic's
  `(name, subject, shape, payload_hash)` in an unhashed
  `topics` section — the join document: a recording/WAL
  segment carrying `(name, shape_hash)` matches a row and
  names the exact checked topology it ran under. The two
  identities stay separate namespaces by ruling (payload
  shape does not affect claim evaluation, so it is not part
  of the model `shape_hash`); the artifact references, never
  fuses.

  The `topics` section arrived at schema 1.2; the artifact is
  at **1.9** as of hale v0.16.0+ (1.3 integrity digest, 1.4
  verdict vocabulary, 1.6 evaluation, 1.8 source maps, 1.9
  sealing in the hashed model). Consumers should read the
  `schema` key rather than pin a version — the section is
  additive and 1.2's rows still parse.

  **Verify `artifact_digest` before joining on `topics`.**
  Schema 1.3 added it precisely because this join was
  unverified: `shape_hash` covers the *model half only*, so
  the `topics` rows iris joins on sit outside it, and an
  artifact could be edited to agree with a forged shape hash.
  `artifact_digest` is FNV-1a/64 over the entire body,
  emitted as the final key so verification is a prefix hash —
  no re-serialization, no canonicalization step. iris trusts
  artifacts it did not produce, so it owes that check.
  *(Amended 2026-08-11.)*
- **Dynamic registration.** The native emitter writes the
  manifest once at startup (from `.hale.topo`). The library
  emitter learns topics at runtime and appends. Append
  protocol: write the entry and strings, then increment
  `manifest_gen` (release). Consumers re-scan on generation
  change (acquire). Entries are never mutated or removed.

  *(Amended 2026-08-12.)* In the absence of `.hale.topo` the
  native emitter also registers on first use — a topic's row is
  created by its first probe. Two consequences consumers may
  rely on, and emitters must preserve:

  **Absence is meaningful.** A topic that never carries a
  message has no row, so "declared in the source topology but
  absent from the manifest" is a sound reading of *never
  mentioned at runtime*. iris's inspector builds a verdict on
  exactly this (`inspect/`).

  **Registration must be flavor-uniform.** That reading holds
  only if every dispatch flavor registers alike. It did not: an
  intra-subtree publish was rewritten to a direct handler call
  before lowering and registered nothing, making "compiled away"
  and "never published" the same observation (iris handoff-11
  P23, fixed upstream in `5567bf2` — the desugared site now
  probes like every other flavor). Recorded here because it is a
  contract, not an implementation detail: **a new dispatch
  flavor that skips registration silently converts live topics
  into apparently-dead ones**, and nothing in the segment can
  reveal the difference.
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
- ~~Exact shape_hash definition~~ — **closed** (hale#399):
  pinned in §4 with test vectors; reference implementation
  `hale_types::topic_identity`, exported by the topology
  artifact (schema 1.2, current 1.9) as the recording↔topology
  join. Both pinned vectors re-verified against hale v0.16.0+
  on 2026-08-11 (`0xf7d174542aa33437`; parented `org.metrics`).
- ~~Late attach loses structure~~ — **closed upstream**
  (hale v0.11.18): the birth replay on `observer_count` 0→1
  was probe-driven, so a quiet process (long read loop, pinned
  readers) never noticed the transition. A detached heartbeat
  thread now drives the gate check every 250 ms under
  `LOTUS_OBS=1`, bounding post-attach replay latency at ~250 ms
  with zero probe traffic. Consumer-visible cost: the heartbeat
  claims one SPSC ring slot, so a `rings=N` segment leaves N-1
  for app threads (§14).
- Whether `sample_n` is global (current) or per-topic
  (another mode-mask-sized table).
- macOS: POSIX shm name limits and `memfd`-equivalent —
  parked until the platform work upstream settles (#231).
  Movement to watch: hale#445 replaced "native = whatever host
  compiled the compiler" with a real `TargetSpec` (canonical
  triples, per-target linker/runtime facts, `hale
  --list-targets` with support tiers). That is the seam a
  second shm implementation would hang off; nothing for iris
  to do until a non-Linux target reaches a tier that matters.
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

- ~~**`aux_b` collision (§4)**~~ — **closed 2026-09-04 at proto
  0.4** (hale#525): v0's meaning retired, every emitter writes the
  entity id or 0, scheduler cpu index → `aux_a`.

## 14. Turning it on (native emitter)

The protocol says nothing about how an emitter is enabled —
that is upstream's surface — but iris ships the acceptance
loop, so the prerequisites belong here. For the native hale
runtime (v0.11.15+):

| Env | Default | What it buys |
|---|---|---|
| `LOTUS_OBS=1` | off | The whole plane: segment, registration, manifest, counters, local records (BUS_*, lifecycle, supervision). **Never alters the wire.** |
| `LOTUS_OBS_WIRE=1` | off | The 16-byte `[magic][origin\|seq]` wire header. **Required fleet-wide for cross-process edges** — without it NET records carry `(0, local-seq)`: countable, not pairable. |
| `LOTUS_OBS_RINGS` | 8 | Rings per segment. |
| `LOTUS_OBS_SLOTS` | 4096 | Slots per ring (power of two). |

Two prerequisites that cost real field time when missed:

- **Edges need `LOTUS_OBS_WIRE=1` on every node, all ≥
  v0.11.15.** This is deliberate (hale#277, iris handoff-4
  P16): before the opt-in, an observed sender's header made a
  stale peer's deserialize fail on every datagram, *silently*
  — enabling observation partitioned a fleet with one old
  binary. With the opt-in, `LOTUS_OBS=1` alone is byte-for-byte
  identical to an unobserved run. The cost is that observation
  and pairing are now two switches, and a fleet with only the
  first reads origin 0 everywhere and forms no edges. That is
  correct behavior, not a bug — check the env before filing
  one.
- **The replay heartbeat claims a ring.** A fleet pinning
  `LOTUS_OBS_RINGS` to exactly its thread count should add
  one, or an app thread shares a ring and overruns earlier.
