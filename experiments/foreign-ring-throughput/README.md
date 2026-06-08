# Foreign-ring throughput microbench (Proposal B)

Gates the post-M3a performance question: is the descriptor-driven
`byte_records` framing worth specializing (design-doc OQ2), or building
the A1 zero-copy producer view?

## Run

```sh
./run.sh
```

(Links `crates/hale-codegen/runtime/lotus_shm_ring.c` and the in-file
modelled framing variants — see `bench.c` for what each path measures.)

## What it measures

Per 80-byte flat record, three framing variants on each side:

- **`*-desc`** — runtime descriptor (offsets/width/align/capacity loaded
  from memory) with `local % cap` for the wrap. This is the shipped
  PR3/M3a path.
- **`*-incr`** — runtime descriptor, but the wrapped offset is kept
  **incrementally** (`pos += step; if (pos >= cap) pos -= cap`) instead
  of `% cap`. Records never straddle the wrap (the pad guarantees it),
  so this is exact — and needs **no** power-of-two capacity.
- **`*-const`** — layout constants compile-time-folded, power-of-two
  capacity reduced to a mask, fixed-width loads/stores. The ceiling a
  full codegen specialization would approach.

Plus the real shipped `lotus_bus_publish_shm_ring_layout` (incl. its
subject `strcmp`) and the native `LRSRNG1` publish as reference points.

## Findings (2026-06-08, -O2, x86-64)

```
PRODUCER framing:   pub-desc 4.11   pub-incr 1.76   pub-const 0.91  ns/op
CONSUMER walk:      read-desc 7.09  read-incr 5.39  read-const 1.73 ns/op
shipped publish 4.7 ns/op  ·  native publish 6.3 ns/op
```

1. **In isolation, the framing's dominant avoidable cost is the
   per-record `% cap` modulo** — a 64-bit division that can't be
   strength-reduced (capacity is a runtime value, not required
   power-of-two). It is *not* field-load overhead: LLVM LICM already
   hoists the loop-invariant descriptor fields into registers.
2. **The incremental wrapped-offset removes the modulo** for *any*
   capacity, and in the isolated framing loop that is large: producer
   243 → 567 M rec/s, consumer 141 → 186 M rec/s. A few lines per hot
   loop, no new constraints, no codegen work.
3. **But end-to-end the win is small.** A/B of the *real* publish path
   (`pub-shipped`, old vs incremental runtime, same machine) moves only
   **~5% at an 80-byte payload (4.70 → 4.47 ns/op)** and is **in the
   noise at 16 bytes**. The isolated framing delta does not translate:
   in the real path the division overlaps with the `memcpy` + atomic
   release-store + subject `strcmp` under out-of-order execution, so
   removing it frees little wall-clock.
4. Full codegen specialization (the `-const` column) buys more in
   isolation but needs power-of-two capacity / per-layout code, and
   would hit the same end-to-end ceiling.

## Decision

- The incremental-offset runtime fix is a **correct, low-risk
  division-removal** worth keeping — but a *minor* win (~5% publish at
  80 B), not the headline the isolated framing numbers suggest.
- **Defer** codegen specialization (OQ2) and the A1 zero-copy writable
  view: the end-to-end A/B shows the framing isn't the bottleneck (the
  memcpy + atomic + lookup are), so neither is justified by throughput.
- The real lesson: **measure end-to-end, not just the hot inner loop** —
  an isolated framing microbench overstated this optimization ~25×.

## Caveats

Isolated-framing absolute ns are optimistic (hot resident buffer,
loop-invariant payload, no cache misses). The `pub-shipped` A/B is the
trustworthy end-to-end number; the `desc`/`incr`/`const` framing columns
show where the *framing* cost goes, not the end-to-end win.
