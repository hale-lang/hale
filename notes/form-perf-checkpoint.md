# @form perf checkpoint

Tracks the FORM-3 perf gate (10% of hand-written C) as the
substrate iterates. The `bench/` harness is the source of truth
for current numbers; this file is the narrative the harness
output doesn't carry — what changed, what the diagnosis was,
what's still open.

## 2026-05-13 — first bench, post-FORM-4

The bench harness landed (parallel-process activity in
`bench/`) and surfaced concrete ratio data against Go / Node /
Python siblings. Headline numbers from the 5-sample median
run, Aperio vs Go:

| Bench               | Aperio   | Go ratio | Status |
|---------------------|----------|----------|--------|
| loop_overhead       | 20.4 ms  | 0.94×    | within 10% gate ✓ |
| form_vec_push       | 2.89 ms  | 0.96×    | within 10% gate ✓ |
| form_vec_get        | 2.36 ms  | 0.016×   | **62× behind** — pre-fix |
| fn_call             | 188 ms   | 0.04×    | 25× behind (m49 ABI) |
| locus_instantiation | 3.07 ms  | 0.006×   | 167× behind (arena/locus) |
| bus_dispatch        | 2.48 ms  | 0.019×   | 53× behind |
| stream_aggregator   | 4.28 ms  | 0.005×   | 200× behind (composite) |

**Diagnosis split.** Two distinct perf shapes surfaced:

1. **form_vec_get's 62× is layout-correct but codegen-pattern
   wasteful.** form_vec_push at 0.96× proves the inline vec
   layout is right (struct GEP + memcpy of elem bytes is what
   hand-written C does). The 62× on `get` comes from the
   fallible-call codegen surface — specifically, the FORM-2
   PR5 codegen constructed `IndexError` *unconditionally* on
   every call:
   - arena_alloc for IndexError struct
   - 3 stores populating kind / index / len
   - lotus_vec_len function call (just to fill err.len)

   All dead on the happy path. ~50 cycles of waste per call.

2. **fn_call / locus_instantiation / bus_dispatch are
   layout-conditioned by The Design.** The m49 arena-subregion-
   per-call calling convention and the arena-per-locus
   lifecycle are substrate commitments, not codegen-pattern
   accidents. Closing those gaps is calling-convention design
   work; separate from FORM-3.

## 2026-05-13 — lazy-error fix landed

Moved `emit_index_error_alloc` / `emit_key_error_alloc` into
dedicated err basic blocks inside
`try_lower_form_vec_fallible_method` and
`try_lower_form_hashmap_fallible_method`. The happy path now
branches over the alloc + stores entirely. Also dropped the
unconditional `lotus_vec_len` pre-call — `len` is now read
inline via struct GEP into the vec's `len` field, and only on
the err path (where its value populates `IndexError.len`).

Two consecutive `cond_br` on the same `is_err` SSA (one in the
dispatcher, one in `lower_or_expr`'s consumption of the
result) compile down under SimplifyCFG / GVN.

| Bench         | Before  | After   | Go ratio before | Go ratio after | Δ |
|---------------|---------|---------|-----------------|----------------|---|
| form_vec_get  | 2.36 ms | 1.61 ms | 0.016× (62× back) | 0.024× (42× back) | **−32%** |
| form_vec_push | 2.89 ms | 3.02 ms | 0.96×             | 0.90×              | noise |
| loop_overhead | 20.4 ms | 20.4 ms | 0.94×             | 0.94×              | unchanged |

Real measurable win. Tests: 656 / 0 (unchanged).

## What's still open for the FORM-3 gate

The remaining ~42× on form_vec_get is dominated by:

- **The `lotus_vec_get` C-function-call boundary.** LLVM can't
  inline across the TU boundary without LTO. Hand-written C
  inlines the bounds check + load into the call site; Aperio
  pays a function call.
- Possibly some calling-convention overhead inherited from m49
  even though the synth-method dispatch path doesn't itself
  subregion.

Two candidate next steps:

1. **Inline `lotus_vec_get`'s logic directly in LLVM IR at
   codegen time.** The C function is:
   ```c
   int lotus_vec_get(lotus_vec_t *v, size_t es, int64_t i, void *out) {
       if (i < 0 || (size_t)i >= v->len) return 0;
       memcpy(out, v->buf + i * es, es);
       return 1;
   }
   ```
   This is ~5 IR instructions: load len, icmp i ≥ len OR i < 0,
   cond_br to oob_bb / load_bb, in load_bb load buf, GEP buf +
   i\*es, load value, store to out. Bypasses the function call
   entirely. Same shape as Go's `v[i]`.

2. **LTO build.** Cheaper to set up (add a flag) but harder to
   trust across release / debug; depends on the linker. Less
   surgical.

(1) is the right shape per the design philosophy — synth
methods on `@form(...)` are not user fns and shouldn't pay a
user-fn-shaped ABI. Reserved for a follow-up if the FORM-3
gate becomes load-bearing.

## What this does NOT fix

The m49 ABI gaps (fn_call 25×, locus_instantiation 167×,
bus_dispatch 53×, stream_aggregator 200×) are unchanged by
this milestone. Those are substrate calling-convention design
work, gated on whether real apps measure the cost. Worth
benching apps before redesigning the substrate.
