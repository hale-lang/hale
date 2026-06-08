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

1. **The shipped path's dominant avoidable cost is the per-record
   `% cap` modulo** — a 64-bit division that can't be strength-reduced
   (capacity is a runtime value, not required power-of-two). It is *not*
   field-load overhead: LLVM LICM already hoists the loop-invariant
   descriptor fields into registers.
2. **The incremental wrapped-offset removes the modulo entirely** for
   *any* capacity: **+133% producer throughput** (243 → 567 M rec/s) and
   **+31% consumer** (141 → 186 M rec/s). A few lines in each hot loop;
   no new constraints, no codegen work.
3. **Full codegen specialization buys more** (the `-const` column) but
   requires power-of-two capacity (mask) and/or per-layout emitted code,
   and the residual win is small in absolute terms — every variant is
   already 140–1100 M rec/s, far above any real feed rate.

## Decision

- **Do** the incremental-offset runtime fix in the reader + producer hot
  loops (cheap, universal, removes a division). Worth it.
- **Defer** codegen specialization (OQ2) and the A1 zero-copy writable
  view: real but small residual headroom against already-enormous
  throughput; revisit only if a workload shows the ring as the
  bottleneck.

## Caveats

Absolute ns are optimistic: a hot resident buffer, a loop-invariant
payload (the `memcpy` may be partly CSE'd), and no cache misses. Under a
real feed with cold lines, memory stalls shrink the modulo's *relative*
cost — so the incremental win is "up to ~2.3 ns/record saved," large at
high rates and negligible at low. The relative `desc`/`incr`/`const`
ordering is the robust signal; the absolute rates are an upper bound.
