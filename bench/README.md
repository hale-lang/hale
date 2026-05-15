# bench — Aperio performance harness

Baseline scaffold for Layer 3 (performance) per `spec/testing.md`.
Microbenches isolate substrate primitives; one app-level bench
exercises a small mixed workload end-to-end. The runner is
shell + jq, no extra build dependencies.

Each `.ap` bench ships with sibling `.go`, `.js`, and `.py`
equivalents implementing the same shape as closely as possible.
The harness runs all of them, reports per-language medians plus
a `ratio_vs_aperio` column, but **only Aperio regressions** gate
exit code (per `spec/testing.md` Layer 3: "a regression in
aperio-vs-X ratio is a developer signal, not a CI gate").

## Quickstart

```
# From repo root, with the CLI built (cargo build --release -p aperio-cli):
./bench/run.sh                       # run all + comparatives, exit 1 on Aperio regression
./bench/run.sh --iters=10            # more samples per bench (default 5)
./bench/run.sh --bench=loop_overhead # one bench at a time
./bench/run.sh --update-baselines    # overwrite baselines.json with new Aperio medians
./bench/run.sh --no-build            # skip rebuild step
./bench/run.sh --no-comparative      # Aperio only, skip go/node/python siblings
./bench/run.sh --json                # emit only the JSON report to stdout
```

Each invocation writes a timestamped JSON report under
`bench/results/` (gitignored).

## Three kinds of microbench

The benches under `bench/micro/` split into **three categories**
answering different questions. Mix them with care — they're not
interchangeable.

### Overhead microbenches — "what does the substrate cost when unused?"

Strip allocation work down to nothing so the substrate machinery
runs alone. These deliberately measure Aperio's worst case: the
arena lifecycle gets no chance to amortize against work it would
normally accompany.

- **`loop_overhead`** — empty while loop. No arena work at all.
- **`fn_call`** — `fn noop(x) -> Int { return x; }` called 10M
  times. m49's per-call subregion runs against a body that
  doesn't allocate, so the boundary cost is paid for nothing.
- **`locus_instantiation`** — `Empty {}` 100k times,
  statement-position. Arena create + struct init + arena destroy
  with zero allocations in between.
- **`bus_dispatch`** — 10k typed messages through the bus. The
  per-message payload memcpy + queue enqueue is the design (per
  `memory.md` "pointers don't cross loci; values do") but the
  bench measures it isolated.
- **`form_vec_push`** — 500k pushes only. Isolated growth path.
- **`form_vec_get`** — 200k indexed reads only. Isolated read
  path.
- **`form_hashmap_set`** — 1M Int-keyed inserts into a
  `@form(hashmap)` locus. Tests hash + slot probe + entry
  memcpy + occasional grow/rehash.
- **`form_hashmap_get`** — 150k Int-keyed lookups against a
  pre-populated `@form(hashmap)`. Cliffs at n=200k (set+get
  doubles per-iter work vs pure-write set).

Expect Aperio to be **slow** here. These benches surface
codegen overhead the compiler team can target (e.g. eliding
arena subregions when a fn provably doesn't allocate) and
violations of spec performance commitments (e.g. `form_vec_get`
60× behind Go violates `spec/forms.md` FORM-3's "within 10%
of C" target).

### Amortized microbenches — "does the design pay off when used as intended?"

Match the shape the design optimizes for: many allocations
inside one arena, wholesale-free at dissolve. The substrate cost
is paid once across N units of work, not per unit.

- **`vec_amortized`** — push N + fold N, single timed region.
- **`fn_scratch_work`** — 100 fn calls × 1000-element local
  `@form(vec)` per call. The m49 subregion gets a real workout.
- **`coord_with_churn`** — chunked-class parent accepting K
  Worker children. Tests F.3 free-list reclamation + chunked
  sub-region allocator. (K capped at 20 by v1 codegen's
  accept() accumulation cliff at k≈25 — see comment in source.)

These are where Aperio's region model is supposed to win. The
ratio against Go is the right signal — if amortized benches show
Aperio competitive with Go and overhead benches don't, the
design is real but the compiler is leaving per-op cost on the
table.

### Coordinated-workload microbenches — "does the design win in multi-locus shapes?"

The shape Aperio is *built for*: multiple loci deep, lateral
siblings, coordinating via vertical-only flow or bus. The design
predicts these should out-throughput dynamic-language alternatives
because region memory + cooperative scheduling + bus-mediated
typed messages avoid the per-allocation GC tracking those
languages pay.

- **`tree_fanout`** — depth=2 with K=20 lateral siblings.
  Coordinator's accept() calls `worker.compute()` for each
  child; results aggregate into parent state. Tests hierarchical
  region memory + cross-locus method dispatch. **First bench
  where Aperio decisively beats Node (8.6×) and Python (20.5×).**
- **`pipeline_3stage`** — depth=3 sequential. Source → Filter →
  Sink via two bus subjects. Tests multi-stage bus coordination
  + the cooperative scheduler's drain semantics. Currently
  bottlenecked by per-event bus-dispatch overhead — same shape
  that limits `bus_dispatch`.

### App benches — `bench/app/`

- **`stream_aggregator`** — publisher fires N typed events; a
  long-lived aggregator subscribes and maintains running stats.
  Cross between bus_dispatch and a real workload.

## Layout

```
bench/
├── README.md                this file
├── run.sh                   shell + jq harness
├── baselines.json           checked-in Aperio medians + tolerance bands
├── results/                 per-run JSON reports (gitignored)
├── micro/
│   │
│   │   # Overhead microbenches (isolate substrate cost)
│   ├── loop_overhead.{ap,go,js,py}
│   ├── fn_call.{ap,go,js,py}
│   ├── locus_instantiation.{ap,go,js,py}
│   ├── bus_dispatch.{ap,go,js,py}
│   ├── form_vec_push.{ap,go,js,py}
│   ├── form_vec_get.{ap,go,js,py}
│   ├── form_hashmap_set.{ap,go,js,py}
│   ├── form_hashmap_get.{ap,go,js,py}
│   │
│   │   # Amortized microbenches (match design's optimization target)
│   ├── vec_amortized.{ap,go,js,py}
│   ├── fn_scratch_work.{ap,go,js,py}
│   ├── coord_with_churn.{ap,go,js,py}
│   │
│   │   # Coordinated-workload microbenches (multi-locus shapes)
│   ├── tree_fanout.{ap,go,js,py}
│   └── pipeline_3stage.{ap,go,js,py}
├── app/
│   └── stream_aggregator.{ap,go,js,py}
└── c-twins/                 (placeholder) hand-written C equivalents
                             for FORM-3 10%-gate comparisons.
```

## Conventions

Every bench (any language) self-times the work-of-interest with
a monotonic clock and prints exactly one `elapsed_ns=N` line on
stdout. The harness additionally captures `maxrss_kb` externally
via `/usr/bin/time -v`. The runner takes N samples per bench
(default 5), records the median, and writes both per-sample
arrays and the median into the JSON report.

**Monotonic clocks used:**
- Aperio: `std::time::monotonic()` → Duration ns
- Go: `time.Since(t0).Nanoseconds()`
- Node: `process.hrtime.bigint()`
- Python: `time.monotonic_ns()`

**Regression gate (Aperio only).** A bench fails when:

```
current_median > baseline_elapsed_ns * (1 + tolerance)
```

Faster-than-baseline is never a regression. The default tolerance
is **0.30** — sub-10ms benches routinely jitter ±20% under OS
noise. Tighten per-bench in `baselines.json` once a metric
stabilizes. Comparative numbers (go/node/python) are emitted in
the report but **never** trigger exit 1.

**Toolchain detection.** The harness checks for `go`, `node`,
and `python3` on PATH at startup. Each comparative language is
silently skipped if its toolchain is missing.

## Adding a bench

1. Decide which category: overhead microbench (isolate a primitive
   under worst-case conditions), amortized microbench (real
   workload shape), or app bench (mixed-workload end-to-end).
2. Drop a `.ap` file in `bench/micro/` or `bench/app/`. Each one
   must:
   - Self-time the work-of-interest with two `std::time::monotonic`
     calls and `t1 - t0` arithmetic.
   - Print `elapsed_ns=` followed by the duration value on its
     own line.
3. Add sibling `.go`, `.js`, `.py` files implementing the same
   shape as closely as the language permits. Each must also
   print one `elapsed_ns=N` line. The harness picks them up
   automatically by filename stem.
4. Run `./bench/run.sh --update-baselines` to seed.
5. Commit the new sources + the updated `baselines.json`.

## Reading the report

```json
{
  "generated_at": "...",
  "iters": 5,
  "benches": [
    {
      "name": "fn_scratch_work",
      "kind": "micro",
      "status": "ok",
      "elapsed_ns_median": 469208,
      "elapsed_ns_samples": [...],
      "maxrss_kb_median": 3088,
      "maxrss_kb_samples": [...],
      "baseline_elapsed_ns": 469208,
      "baseline_maxrss_kb": 3088,
      "note": null,
      "comparatives": {
        "go":     { "elapsed_ns_median": 397163,  "ratio_vs_aperio": 0.8465, ... },
        "node":   { "elapsed_ns_median": 918277,  "ratio_vs_aperio": 1.9571, ... },
        "python": { "elapsed_ns_median": 2578210, "ratio_vs_aperio": 5.4948, ... }
      }
    }
  ]
}
```

`ratio_vs_aperio = lang_elapsed / aperio_elapsed`. A value of
**0.5** means the other language is 2× faster than Aperio; **2.0**
means Aperio is 2× faster than the other language; **1.0** is
parity.

## Known constraint — accumulation ceiling

Several Aperio substrate paths segfault under v1 codegen
somewhere between 100k and 1M iterations of a tight loop. The
microbench Aperio iteration counts are tuned **below** those
ceilings; comments inside each `.ap` document the threshold
they hit. The sibling `.go/.js/.py` benches mirror the same
iteration count so the ratio stays apples-to-apples.

The **`coord_with_churn`** bench hits the steepest cliff:
parent's `accept(child)` in a loop fails at k≈25 regardless of
projection class. Caps the bench at K=20, where the timing
signal is small but measurable. When the substrate
accumulation fix lands, raise iteration counts in all four
language files together.

## Future work

- **C twins for FORM-3.** `spec/forms.md` commits `@form(vec)`
  to within 10% of a hand-written C equivalent on push+get.
  Land C sources in `bench/c-twins/` and add a comparison column
  to the runner's report.
- **More comparative langs.** Erlang (BEAM) is the natural fourth
  comparator since Aperio's runtime model is BEAM-shaped. Rust
  is a fifth for the "non-GC compiled" point of comparison.
- **`aperio bench` CLI.** Per `spec/testing.md`, this surface is
  planned but not shipped. The shell harness here is the current
  stand-in.
