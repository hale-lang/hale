# Handoff: compiler/runtime findings from the iris observer build (2026-07-27)

Source: the iris session that ported `fuse` (multi-segment shm
fusion + HTTP/SSE serve) to Hale — `iris-observer` branch
`observer` @ `1964d81`, `consumer/fuse-hl/`. Everything below
was found by building and soak-testing that program against a
~500k records/s workload. Toolchain: `hale 0.11.9` CLI was
stale; all results below use a fresh `cargo build -p hale-cli`
(`target/debug/hale`).

Priorities: **P1 is the only correctness bug.** P2/P3 are
analysis/API gaps with working workarounds.

---

## P1 — `@form(vec).set` fallible path: per-call leak + ~1000× slowdown

**Symptom.** Each `v.set(i, val) or discard` on a
`@form(vec)` locus:
1. **leaks ~33 bytes** into a region that is NOT reclaimed
   when the calling child locus dissolves (suspect: the
   fallible-ABI sret/error plumbing allocating in the form
   owner's region, or a per-call arena that never resets);
2. **costs ~1µs/call** — roughly 1000× the inlined `.get`
   (forms.md says get/set/pop/push are "inlined directly at
   codegen"; measured behavior says `.set` still pays a heavy
   path).

**Repros** (self-contained, no FFI):
`/home/riley/code/hale-lang/iris-observer/consumer/fuse-hl/upstream-repro/`

| file | what it does | result |
|---|---|---|
| `repro.hl`  | 3M per-iteration child loci, each builds a 1KB string | flat, ~1M children/s — child-region reclaim works |
| `repro2.hl` | 100k children × 1k parent-form `.get or default` (100M reads, rows carry a String) | flat, 4.9MB peak, 0.07s — reads clean |
| `repro3.hl` | children doing `.get` + `.set or discard` pairs on a 1024-entry parent vec | **2M sets → 70MB peak, 2.06s** (~33B and ~1µs per set). At the original 100M-set scale it ran >2min and was killed |

Run: `target/debug/hale build reproN.hl && /usr/bin/time -v ./reproN`

**Why it matters.** fuse-hl's event path does ~1M sets/s →
~1.4–3MB/s RSS growth in a long-running observer (confirmed:
idle fuse is perfectly flat; growth scales only with event
volume). Until fixed, fuse-hl needs periodic restarts. The
set latency also taxes the drain loop that must outrun
emitters' rings.

**Corroborating field data.** fuse-hl leaked ~2.2MB/s in its
first (pre-batching) shape and ~1.4MB/s after all FFI/alloc
suspects were eliminated; the residual matches the repro3
per-set cost times the measured set rate.

---

## P2 — `verify` unbounded-allocation analysis: three false-positive shapes

`hale check` on fuse-hl is clean (0 errors), but
`hale verify` trips **26 advisories** that are wrong for this
program's shape. The advisory text itself taught us the
correct idioms (it caught a real 2MB/s leak in the first
draft — the analysis is genuinely valuable); these are the
patterns it can't yet see:

1. **Const-bounded init loops** read as unbounded:
   `while i < NET_SLOTS * WINDOW { self.inflight.push(...) }`
   in `birth()` (both consts). Constant-folding the bound
   would clear it.
2. **Per-iteration child loci**: allocations inside a locus
   constructed and dissolved every loop iteration (`Cycle` in
   `App::run`) are flagged as accumulating "until the locus
   dissolves" — which is the very next statement. The
   analysis doesn't model child lifetime. (This idiom is the
   documented *fix* the advisory suggests, so it currently
   recommends a shape it then flags.)
3. **Domain-bounded form growth**: `.push` into forms whose
   size is bounded by external structure (attached segments
   ≤16, topics ≤64) — probably needs an annotation rather
   than analysis.

There is **no suppression/acknowledge annotation**, so any
program with these shapes permanently fails the discipline
gate. Either sharpen 1–2 or add an ack mechanism (per-site,
so the gate keeps its teeth).

Repro: `hale verify /home/riley/code/hale-lang/iris-observer/consumer/fuse-hl/`

---

## P3 — `std::io::tcp::Stream.send` blocks with no timeout surface

A stalled SSE client (half-dead conn, throttled tab) blocks
`send` indefinitely and thereby stalled fuse-hl's fusion loop
— observed as emitter-side ring overruns whenever a consumer
socket stopped draining. Worked around in C glue with
`setsockopt(SO_SNDTIMEO, 200ms)` on the taken-over fd.

Suggestion: a `send_timeout_ms` param on `Stream` (mirroring
the conn loop's 5s recv timeout), or a
`std::io::tcp::set_send_timeout(fd, ms)` sibling of the
`set_recv_timeout` that http.hl's takeover docs already
reference. Servers that take over connections will all hit
this.

---

## Smaller notes (no action urgently needed)

- **FFI ergonomics that worked well**: `Bytes` param as a
  C-fillable batch buffer (`lotus_bytes_len/data`) — one FFI
  call per 512 events replaced 5–6 calls/event and fixed real
  event loss. Arena-String returns per ffi.md worked as
  documented. Native `>>`/`&` on Int handled all protocol
  bit-unpacking (no glue needed).
- **Diagnostics were excellent** throughout: `drain` reserved
  lifecycle keyword, vec `.set` fallibility, `let mut`,
  `std::math::int_to_float` naming — every miss was a
  one-shot fix.
- **F.10** (`codegen-unknown-cross-seed-type-in-signature`)
  remains the standing gate on building spike code pulled
  from iris `main`.
- The wasm32 target passed its render-loop spike with ~100×
  headroom (`iris-observer/examples/wasm-flower/`); no wasm
  bugs found.

---

## P4 (feature, not bug) — native bus observation emission

The runtime's bus layer (`lotus_bus_load_config` /
`LOTUS_BUS_CONFIG`, subject→UDP-multicast planes) is the
single point through which an entire deployed system's
messages flow — and per iris DESIGN §13, runtime emission is
the hale side's half of the observation contract. Emitting
iris protocol records there lights up every binary of a
stack with zero app changes:

- `NET_SEND` / `NET_DELIVER` (`w1 = binding:16 | seq:48`) at
  the UDP transport send/deliver, using the transport's
  per-subject seq — this is what iris seq-matches into
  cross-process edges with real latency. Consumer-side fuse
  is already multicast-aware (one send matches every
  listener's deliver).
- `BUS_PUBLISH` / `BUS_DELIVER` (`w1 = locus:20 | seq:44`,
  amended 2026-07-27) at dispatch, locus from the current
  locus context — gives per-locus activity attribution.
- `LOCUS_BIRTH`/`DISSOLVE`/`RESTART` from the runtime's
  lifecycle paths — real supervision trees, no manual
  `birth_ev` calls.
- Segment setup: the `observe/` glue in iris
  (`iris-observer/observe/glue.c`) is the reference; the ring
  hot path is `std::ring::__spsc_emit` (hale#247), already a
  lotus primitive. Emission must be mode-gated
  (`OBS_MODE_*` mask, observer-count-gated) so the
  unobserved cost stays a branch.
- Worth deciding at the same time: re-emitting structural
  state (live loci, in-flight births) on the
  `observer_count` 0→1 transition, so a late-attaching
  observer reconstructs the tree — today it can't
  (PROTOCOL §13 candidate).

PROTOCOL.md on the iris `observer` branch is the contract;
the interim per-app path (`lib/observe` `*_from` variants)
works today but carries no transport seqs, so edges stay
counter-only until native emission lands.

---

## Dispositions (compiler side, 2026-07-27, PR hale#263)

**P1 — FIXED, and your two symptoms were one bug.** Vec elements
are pointer-storage; `.set` deep-copied the new element into the
form owner's program-lifetime arena and orphaned the REPLACED
one. The orphans were the ~33 B/set; the growing arena made the
deep-copy's containment chunk-walk progressively slower, which
was the ~1µs (so the leak *caused* the slowdown — they compound).
Fix: replaced elements (+ non-surviving String fields) retire
immediately onto the arena reuse freelist and the deep-copy
allocates through it. Your repro3: 2.06 s / 70 MB → **0.01 s /
7.8 MB flat** (2M sets); with-set and no-set RSS are now
identical; ASan-clean. fuse-hl should hold flat at 1M sets/s
with the set cost near `.get`'s — no more periodic restarts.
One contract note (forms.md): a `.get` value is invalidated by
a later `set` to the same slot — the standard single-owner rule;
your get→modify→set idiom is fine.

**P2 — `hale verify` on fuse-hl now reports 0 findings** (was
26): (1) const-expression ceilings fold (`NET_SLOTS * WINDOW`
ranks bounded); (2) per-iteration eager children are modeled —
the analysis no longer flags the idiom its own advisory
recommends (let-bound / subscription-bearing / `while true`
loci keep the conservative verdict); (3) the 13 per-message
`.set` advisories dissolved with P1 (set is no longer an
accumulation channel). For genuinely domain-bounded growth
(segments ≤16, topics ≤64): the acknowledge mechanism has
existed all along — `@unbounded` on the enclosing fn or
lifecycle hook — and both advisory texts now name it.

**P3 — already shipped; docs fixed.**
`std::io::tcp::set_send_timeout(fd, 200ms)` exists and works
(also the tls sibling for handles). Your `SO_SNDTIMEO` C glue
can be deleted. The takeover chapter now tells servers to arm
it before push loops.

**Smaller notes**: F.10 was fixed 2026-07-27 in hale#250 /
v0.11.10+ (diamond imports through two libs) — re-try your
spike-code builds on a fresh toolchain; the positive
FFI/diagnostics/wasm notes were passed along. Also relevant to
fuse-hl since your build: v0.11.11 shipped `std::compress`
(gzip/zstd) + `std::tar`, and the #255 backpressure contract
(`or wait`, bounded topics, `bounded(N, drop_old)` subscribers
— the SSE slow-client shape).

**P4 — SHIPPED as hale#264 (native observation emission).**
`LOTUS_OBS=1` on any hale binary publishes the PROTOCOL v0.1
segment and emits from the runtime's own choke points:
BUS_PUBLISH/BUS_DELIVER (all dispatch flavors; deliver is
enqueue-time at v0), NET_SEND/NET_DELIVER with per-binding
monotonic seqs, LOCUS_BIRTH/DISSOLVE (cooperative AND pinned
paths), RESTART at reconnect — plus the §13 candidate you
flagged: live-locus birth REPLAY on the observer_count 0→1
edge, so your late-attaching fuse reconstructs the tree.
Verified against your own peek.c (pub=dlv 1:1, manifest names,
births with parent attribution). Knobs: LOTUS_OBS_RINGS=8,
LOTUS_OBS_SLOTS=4096. v0 seams for the freeze conversation
(flagged in lotus_obs.c's header): enqueue-time deliver,
registration-order manifest ids (.hale.topo ordering comes with
that metadata section), publisher-locus attribution on
BUS_PUBLISH is 0/unattributed, pinned births render
parent=root. Your interim lib/observe path keeps working
unchanged — native emission is purely additive.
