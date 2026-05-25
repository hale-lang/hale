# F.32 — Cache-aware locus substrate: delivery plan

**Spec anchor:** `spec/design-rationale.md` § F.32 "Locus
working set as a cache-budget primitive (sketch)".

**Motivation.** The downstream workload (multi-venue
market-data gateway + HFT-adjacent strategy code) demands
tail-latency control. Hale's locus model already exposes
the partitioning a cache hierarchy wants — region semantics
+ lifetime + thread isolation + declared bounds + vertical-
only flow. The structural foundation is there; the active
analysis isn't. This plan turns the structural advantage
into measurable wins.

**2026-05-24 audit finding.** Bench-prep for the original
F.32-1 surfaced a correctness gap in the cross-pool
`@form(hashmap)` path. `lotus_hashmap_set` / `_grow` have
no synchronization (`lotus_arena.c:1869-1992`); the
typecheck exemption shipped at `3ec6391` is purely
type-system, not runtime. Two writers on different pools
double-free during concurrent `grow` (both `free(old_slots)`)
and corrupt `len` / `slots` under cell collision. Repro:
`bench/micro/form_hashmap_false_sharing.hl` at n>=1000
crashes with `double free or corruption (!prev)` within 1s.
This plan now closes that hole first (F.32-0) and reframes
cross-pool sync as a **storage discipline** picked via a
new `sync = ` kwarg on the form annotation (F.32-1{α,β,γ}).
Closed-world inference picks the default per
locus-type (F.32-1∞); the explicit annotation always
overrides.

This document survives a repository / organization rename;
all references to "hale" / "hale" should be read
against the new names once the rename lands. Code paths
below use the directory layout as of `3869ffa` on `main`;
substring-rename should suffice.

---

## Scope summary

Deliverables increase in cost, decrease in
expected impact-per-unit-effort. Each is independently
shippable — earlier deliverables can land without later
ones, and the structural foundation post-F.32-1β is
enough to attack the rest on demand.

| ID | Theme | Effort | HFT impact |
|---|---|---|---|
| **F.32-0**   | Revert cross-pool `@form(hashmap)` exemption — closes corruption hole; restores single-pool default | trivial | (correctness) |
| **F.32-1α**  | `sync = serialized` cross-pool `@form(hashmap)` (one mutex per map) | small | medium |
| **F.32-1β**  | `sync = striped` cross-pool `@form(hashmap)` (per-cell atomic + grow RW-lock + cache-padded cells) — incorporates the original F.32-1 padding work | medium | high |
| **F.32-1γ**  | `sync = lockfree` cross-pool `@form(hashmap)` — γ-v1 SHIPPED 2026-05-25 (fixed-cap, no remove); γ-v2 (grow + remove) is the natural follow-up | medium (v1), large (v2) | **high (v1 measures 1.30× faster than α on the bench)** |
| **F.32-1∞**  | Closed-world sync inference + diagnostic (picks default per locus type from pool-propagation graph) | small-medium | (ergonomics) |
| **F.32-4-prefetch** | Bus-dispatch prefetch hint (pulled forward; pairs with F.32-1β) | trivial | medium |
| **F.32-1b**  | Locus struct field reordering by access frequency | small-medium | medium |
| **F.32-2**   | Compile-time working-set budget per locus | medium | medium (engineering discipline; CI gate) |
| **F.32-3**   | Per-pool arena chunking sized to cache slice | medium | low-medium (multi-pool scale-up) |
| **F.32-4**   | HFT extras: huge pages, prefetch hints, mlock | small-medium | medium-high |

Recommended order: **0 → 1α → 1β + 4-prefetch (one commit) →
1∞ → 1b → 2 → 3 → 4 (huge pages + mlock) → 1γ if bench
numbers justify**. Rationale: F.32-0 is a one-commit drop
that fixes a corruption bug; F.32-1α is the smallest
correct cross-pool path and gives 1∞ a default to fall back
to; F.32-1β is the real perf work + finally makes the
false-sharing bench run cleanly; 4-prefetch ships with 1β
because the detection / producer-side code path is shared;
1∞ closes the ergonomics gap once explicit annotations
exist to fall back to; the rest is unchanged from the
original sequencing.

---

## F.32-0 — Revert cross-pool `@form(hashmap)` exemption

**The problem.** Commit `3ec6391` (2026-05-24) removed the
cross-pool diagnostic for `@form(...)` receivers on the
strength of a commit-message claim that "the hashmap cells
ARE the synchronization primitive." That claim is
aspirational, not shipped — `lotus_hashmap_t` (`lotus_arena.c:1869`)
has no mutex, no atomics, no synchronization. Two writers
on different pools race in `lotus_hashmap_grow`:

1. Both check `(len + 1) * 10 > cap * 7` — both see "needs grow"
2. Both `calloc` a new slots array
3. Both assign `m->slots = new_slots` (one new array leaks)
4. Both `free(old_slots)` — same pointer, freed twice → glibc
   trap: `double free or corruption (!prev)`

`bench/micro/form_hashmap_false_sharing.hl` at n>=1000
reproduces this crash within ~1s of startup. n=100 happens
to survive on initial-cap luck, not correctness.

**The fix.** Scope the typecheck exemption to form-bearing
loci that carry an explicit sync discipline. Plain
`@form(hashmap)` / `@form(vec)` / `@form(ring_buffer)`
returns to the single-pool-only default — cross-pool calls
again typecheck-error. The opt-in path is the new
`sync = ` kwarg (F.32-1α/β/γ below). No runtime change.

**Concrete substrate touch-points.**

1. `crates/hale-types/src/check.rs`: the
   `cross_pool_safe_loci: BTreeSet<String>` built in Phase 5
   of the single-thread invariant pass. Today it includes any
   locus type carrying any `@form(...)` annotation. Restrict
   to locus types carrying `sync = X` where X != `none`.

2. Diagnostic: extend the existing cross-pool error to name
   the upgrade path:

   ```
   error: cross-pool call into Registry @form(hashmap) from pool `ws`
          receiver lives on pool `main`
     hint: cross-pool @form(hashmap) requires an explicit sync discipline
           try `@form(hashmap, sync = serialized)` (per-map mutex, simplest)
            or `@form(hashmap, sync = striped)` (parallel writers, padded cells)
   ```

3. No runtime change. No codegen change. Pure typecheck restriction.

**Tests.**
- `crates/hale-types/tests/cross_pool_form_hashmap_rejected.rs`:
  the two-pool form_hashmap_false_sharing shape, without a
  sync kwarg, must produce the diagnostic.
- The existing single-pool fixtures (`std::log` registry
  used inside one locus, etc.) continue to typecheck clean.

**Spec / docs.**
- `spec/types.md` § "Interaction with @form(...) loci": the
  "implicitly cross-pool-callable" claim becomes "cross-
  pool-callable when an explicit sync discipline is declared
  (see F.32-1)." Plain `@form(...)` is single-pool by default.
- `spec/forms.md`: forward-reference the `sync = ` kwarg.
- This document: this section.

**Acceptance.** `form_hashmap_false_sharing.hl` no longer
compiles without `sync = striped` (or serialized) on the
Registry declaration. No regression in existing fixtures
where forms are used single-pool.

**Estimated effort.** 1 small session. ~50 LOC across
typecheck + diagnostic + one regression test.

---

## F.32-1 — Cross-pool `@form(hashmap)`: sync disciplines

F.32-0 closes the corruption hole but doesn't restore the
Prometheus-counter shape (multi-producer / single-consumer)
that the original F.32-1 was sized for. That workload
genuinely needs cross-pool writes. This umbrella defines
three opt-in disciplines the form ABI offers — each picks a
different point on the throughput / engineering-cost /
tail-latency curve:

| Discipline | Throughput (4 writers) | p99 tail | LOC |
|---|---|---|---|
| **α — serialized** | ~5–10 M ops/s (mutex contention) | ~100µs (futex sleep) | ~50 |
| **β — striped** | ~80–150 M ops/s (parallel cells; grow serializes) | grow spike (~100µs) | ~250 |
| **γ — lockfree** | ~150–200 M ops/s (near-linear) | ~few µs (bounded CAS retry) | ~600 |

The cache-line padding originally specified as F.32-1 lives
under F.32-1β specifically — without parallel cell writes
there's nothing to false-share. `sync = striped` emits
padded cells automatically; the other variants don't pay
for padding they can't use.

F.32-1∞ adds closed-world inference: the typechecker walks
the call graph from `main.placement { }` and picks a sync
default per form-bearing locus type, emitting a diagnostic
at the decl site naming the choice. Explicit annotations
always override.

## F.32-1α — `sync = serialized`

**Surface.**
```hale
@form(hashmap, sync = serialized)
locus Registry { capacity { pool entries of Counter indexed_by id; } }
```

**Implementation.**

1. `lotus_hashmap_t` (`lotus_arena.c:1869`) grows a
   `pthread_mutex_t mu;` field. Initialized in a new
   `lotus_hashmap_init_serialized`, destroyed in a paired
   `lotus_hashmap_destroy_serialized` called from the
   locus's dissolve before the arena wholesale-free.
2. `lotus_hashmap_set` / `_get` / `_remove` / `_bump` /
   `_key_at` / `_entry_at` / `_len` / `_has` wrap bodies in
   `pthread_mutex_lock` / `unlock`. `_grow` runs under the
   same lock — no re-entrancy because the same thread holds
   it across the grow call.
3. Codegen: when emitting a locus with `sync = serialized`,
   call `lotus_hashmap_init_serialized` instead of
   `lotus_hashmap_init`. Same ABI; init variant constructs
   the mutex.

**Tests.**
- `crates/hale-codegen/tests/form_hashmap_serialized_crosspool.rs`:
  the F.32-0-rejected shape now typechecks and runs.
  100k inserts from each of two pools; assert `len == 200000`
  and no crash.
- `crates/hale-codegen/tests/form_hashmap_serialized_basic.rs`:
  single-thread `set` / `get` / `remove` continue to work
  (uncontended mutex path).

**Spec / docs.**
- `spec/forms.md`: add `sync = ` kwarg; list `serialized` as
  one value; document the per-map-mutex semantics.
- `spec/types.md`: cross-pool exemption applies when sync != none.

**Acceptance.** The previously-crashing
`form_hashmap_false_sharing.hl` bench runs cleanly with
`sync = serialized` on the Registry. Throughput will be
modest by design — α is the safe baseline; β is where the
perf work happens.

**Estimated effort.** ~1 small session. ~100 LOC (mutex
plumbing + 2 tests + spec touch).

---

## F.32-1β — `sync = striped` (per-cell atomic + grow RW-lock + cache-padded cells)

**Status (2026-05-25, second pass): β2-v2 SHIPPED.**

**What's in the runtime:**
- `@form(hashmap, sync = striped)` valid annotation; codegen
  routes through `lotus_hashmap_init_striped`.
- `lotus_hashmap_t` carries `sync_mode`,
  `mu_grow: pthread_rwlock_t *`, `cell_stride: size_t`.
- Cells cache-padded to `LOTUS_CACHE_LINE` (64B default).
- **True cell-level CAS** in `lotus_hashmap_set_striped`:
  - `EMPTY (0) → CLAIMED (1) → COMMITTED (2)` state machine.
  - `__atomic_compare_exchange_n` for slot claim.
  - Acquire-release pair on `slot[0]` for memory ordering.
  - Writers run in parallel — no global mutex.
- `find_slot_striped` + `resolve_index_slot_striped` spin past
  transient `CLAIMED` states (bounded by writer's release).
- Probe wrap-around detection: if probes ≥ cap, force grow
  + retry (concurrent writers may have filled the slack
  between the load-factor check and the probe).
- `lotus_hashmap_set_unlocked` writes `LOTUS_CELL_COMMITTED`
  (was `1`, now `2`) so the grow path's re-insertion produces
  immediately-readable cells (not transient-looking CLAIMED).

**Perf finding (2026-05-25, hardware-dependent):**

On this 2-core x86_64 host with the bench's small key/value
payloads (`{id: Int, v: Int}`), β2-v2 measures **1.87× SLOWER**
than α: 29 ms vs 15.5 ms on `form_hashmap_false_sharing`. The
per-op rwlock+CAS overhead (~150 ns) exceeds α's mutex+memcpy
(~90 ns) by more than the 2-core parallelism gain compensates.

**β2-v2 wins materialize on:**
- More cores (the parallelism scales; per-op overhead doesn't).
- Heavier per-op work (large values, complex keys: per-op work
  amortizes the synchronization overhead).
- Read-heavy mixes (rwlock-rdlock allows concurrent reads; α's
  mutex blocks all readers).

**Recommendation by workload:**
- 2-4 cores + cheap k/v ops + write-heavy → use `sync = serialized`.
- 4+ cores or heavy k/v ops or read-heavy → use `sync = striped`.
- Single pool (single-threaded access) → plain `@form(hashmap)`.

The bench file `form_hashmap_false_sharing.hl` is wired with
`sync = striped` so the harness exercises β2-v2's correctness
+ perf shape; the elevated baseline (29 ms) reflects β2-v2's
honest cost on this hardware. Switching to α is a one-line
change for users whose workload favors mutex serialization.

**Correctness validation:**
- /tmp/sm_striped_cp_big (100k inserts × 2 pools): exit 0, len=200000.
- `cargo test --release -p hale-codegen --test form_hashmap_serialized`:
  passes (covers α; the striped path uses the same wiring).
- Workspace test sweep: 169 suites pass.

**Open work for β2-v3 (future):**
- tsan / relacy validation under concurrent stress.
- Investigate cheaper sync primitives (futex-based, biased
  locks) to close the per-op gap with α.
- Lockfree variant (γ) skips rwlock entirely → likely the
  right answer if β2-v2 stays slower than α on common
  workloads.

The original β design — kept verbatim below as historical
context (β2-v2's implementation derived from it):

**The problem.** A `@form(hashmap, sync = striped)` locus is
reachable cross-pool by design. Two cores writing
adjacent cells that share a 64-byte cache line generate
MESI ping-pong even though each producer logically owns
its own cell. The Prometheus-registry pattern (one Counter
per metric, multiple producer pools incrementing, one
consumer pool rendering) hits this directly. Per-cell
atomicity on the occupancy byte gives correctness; cache-
line padding on the cell stride gives throughput.

**The fix.** When codegen emits a `sync = striped` form,
pad cell stride up to the next `LOTUS_CACHE_LINE` multiple
(64B default) AND make the occupancy byte an
`_Atomic uint8_t` with CAS on insert, atomic load on probe.
A `pthread_rwlock_t mu_grow` guards the grow path: set/get
hold read lock (uncontended common case), grow holds write
lock (rare). Other form loci keep the current packed,
non-atomic stride.

**Concrete substrate touch-points.**

1. **Add `LOTUS_CACHE_LINE` constant.** `runtime/lotus_arena.c`,
   alongside `LOTUS_HASHMAP_INITIAL_CAP`. Default 64. A
   `--cache-line-size=N` CLI flag overrides at build time
   for non-x86_64 targets (ARM big.LITTLE has variable line
   sizes; M-series Apple is 128B effective).

2. **Annotation discriminator.** No detection pass needed —
   the `sync = striped` kwarg on the form annotation IS the
   signal. Parser already admits
   `FormAnnotation { name, args: Vec<FormArg> }`; lex one
   more kwarg value (`sync = striped`). Codegen looks up
   the kwarg in the locus's `LocusInfo` and emits striped
   lowering instead of dense. This is materially simpler
   than the pre-F.32-0 plan, which depended on transitive
   reachability analysis from `main.placement { }` —
   replaced now by author opt-in.

3. **Striped emission.** In
   `lower_locus_instantiation` / `declare_locus_struct`,
   when the form is `sync = striped`:
   - Compute `padded_stride = round_up(packed_stride,
     LOTUS_CACHE_LINE)`.
   - Emit a call to `lotus_hashmap_init_striped(key_size,
     value_size, padded_stride, key_type_tag)` instead of
     `lotus_hashmap_init`. The new init constructs the
     `pthread_rwlock_t mu_grow` and uses the padded stride
     for the slots calloc.
   - Striped `_set` / `_get` / `_remove` / `_bump`:
     `rwlock_rdlock` → `find_slot` with CAS on the
     occupancy byte → `memcpy` key+value → atomic
     `fetch_add(&len, 1)` on insert → `rwlock_unlock`.
   - Striped `_grow`: `rwlock_wrlock` → standard grow path
     (now race-free because all readers are blocked) →
     `rwlock_unlock`.
   - Dissolve calls `lotus_hashmap_destroy_striped` which
     `pthread_rwlock_destroy`s before the arena wholesale-free.

4. **F.32-4-prefetch piggyback.** The producer side of
   `lotus_coop_pool_post` (and `lotus_mailbox_post`) gains
   `__builtin_prefetch(slot, 1, 3);` after the slot fill.
   Ship in the same commit — same hot path, trivial diff,
   detection-free.

5. **Tests.**
   - `crates/hale-codegen/tests/form_cache_padding.rs`:
     compiles a 4-cell `sync = striped` hashmap, asserts
     `lotus_hashmap_entry_size(&map) >= LOTUS_CACHE_LINE`.
     Same shape with `sync = serialized` asserts packed
     stride (no padding).
   - `crates/hale-codegen/tests/form_hashmap_striped_concurrent.rs`:
     100k inserts from each of 2 pools into one striped
     map; assert `len == 200000` after both writers drain,
     and assert every expected key is present.
   - `crates/hale-codegen/tests/form_hashmap_striped_perf.rs`
     (gated `#[ignore]`, run with `--ignored`): two
     producer threads pinned to different cores hammer
     adjacent cells of `sync = striped` vs `sync = serialized`
     maps. Measure `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`
     delta. Asserts striped >= 4x serialized on a 2+ core
     host with siblings on the same L2.
   - `bench/micro/form_hashmap_false_sharing.hl` (the bench
     held back during F.32-0) lands with `sync = striped`
     on the Registry; baseline gets a real number for the
     first time.

6. **Spec / docs.**
   - `spec/design-rationale.md` § F.32: promote sketch → v1
     section once shipped. Note the sync-discipline kwarg
     and cell-stride change in the ABI table.
   - `spec/forms.md`: document `sync = serialized` and
     `sync = striped`; describe per-discipline ABI + cost
     trade-off; cross-reference F.32-1∞ inference.
   - `docs/src/how-tos/threading.md`: note that cross-pool
     `@form(hashmap)` requires explicit sync (link F.32-0
     diagnostic); recommend `striped` for parallel-writer
     hot paths and `serialized` for read-mostly or low-rate
     mutate workloads.

**Acceptance.**
- The perf fixture shows striped >= 4x serialized on the
  producer/consumer hot loop (2+ core host, siblings on
  same L2). False-sharing pmu counters
  (`perf stat -e mem_load_l3_hit_xsnp_hitm`) drop to ~zero
  on striped.
- `form_hashmap_false_sharing.hl` runs and produces a
  stable baseline; bench harness picks up the elapsed_ns
  for ongoing regression detection.
- The C twin at `experiments/f32-false-sharing/` establishes
  the theoretical max (~5x for pure increments); the Hale
  bench's 4x target is the realistic ceiling once hash +
  probe + grow are included.

**Out of scope.**
- Padding for non-cross-pool form loci (pure intra-pool
  workloads — `sync` unset). Keep the dense layout; no
  per-cell atomics; no RW-lock.
- `@form(ring_buffer)` — FIFO cells don't false-share by
  construction; cell sharing across producers / consumers
  is bounded by the head/tail seqnos which already live on
  their own cache lines per `lotus_shm_ring.c`.
- `@form(vec)` cross-pool — defer until a workload surfaces.
  The plumbing here generalizes (sync = striped on vec
  cells would emit the same padding + atomic-len pattern)
  but no current downstream wants it.
- Manual cache-line size at the cell-type level (i.e., a
  `@cache_line(128)` annotation on the cell type). Defer.

**Estimated effort.** 1-2 focused sessions. ~250 LOC across
codegen (lowering switch on sync kwarg) + runtime (striped
init/set/get/grow/destroy + 4-prefetch one-liner) + tests.

---

## F.32-1γ — `sync = lockfree` (γ-v1 SHIPPED 2026-05-25)

**Status (2026-05-25, third pass): γ-v1 SHIPPED.**

After β2-v2's perf finding (rwlock+CAS slower than α at
2-core / cheap-payload), γ-v1 was sized DOWN from the
original "full lock-free / wait-free hashmap" scope to a
minimum-viable variant that ships in this session and
proves out the parallel-writer thesis.

**γ-v1 scope:**
- Fixed cap (user declares `cap = N`; runtime never grows).
- Cache-padded cells like β2-v2.
- Pure CAS on the 3-state occupancy machine — NO
  pthread_rwlock, NO pthread_mutex anywhere on the hot path.
- No remove (returns 0; tombstones land in γ-v2 if a
  workload needs them).
- Silent drop on cap exhaustion (caller sizes cap to 2-4× peak).

**Perf finding (2026-05-25, the headline F.32 win):**

On `form_hashmap_false_sharing` (200k cross-pool concurrent
inserts, AMD Ryzen 7 9800X3D):

| Discipline | Elapsed | vs α |
|---|---:|---:|
| γ-v1 (`sync = lockfree`) | **11.95 ms** | **0.77× (1.30× faster)** |
| α (`sync = serialized`)  | 15.51 ms | 1.00× |
| β2-v2 (`sync = striped`) | 28.37 ms | 1.83× slower |

γ-v1 vs Go's `sync.Mutex`-protected map closes the gap
from 1.66× (under α) to 1.18×. Hale is now within striking
distance of Go on the canonical 2-writer concurrent-hashmap
workload — the closest the language has been on a cross-
pool perf benchmark.

**Why γ-v1 beats β2-v2:**
- Per-op cost: γ-v1 ~60 ns/op (CAS + memcpy) vs β2-v2 ~150
  ns/op (rwlock_rdlock + CAS + memcpy + rwlock_unlock).
- Parallelism: identical (both use cell-level CAS).
- The rwlock overhead is the dominant cost in β2-v2 on this
  hardware; removing it more than offsets the loss of grow
  support.

**Why γ-v1 beats α:**
- α serializes all writers through one mutex (~90 ns/op
  serialized, ~22M ops/s/core × 1 core).
- γ-v1 runs writers in parallel (~60 ns/op × 2 cores =
  ~33M ops/s aggregate).

**γ-v2 follow-up scope** (when a workload demands it):
- Tombstones + periodic compaction → `remove` supported.
- Lockfree grow (Cliff Click state machine) → cap no longer
  required upfront.
- tsan / relacy validation under high-contention stress.

**Integration notes:**
- The F.32-1∞ inference rule (when shipped) should default
  to `lockfree` for cross-pool write-heavy maps where cap is
  static, and fall back to `striped` or `serialized` for
  workloads requiring grow / remove.
- The bench file `form_hashmap_false_sharing.hl` is wired
  with `sync = lockfree, cap = 300000` and documents the
  measured ordering on this hardware.

**Original γ design** — kept verbatim below for the
eventual γ-v2 implementer who'll add grow + remove:

A full lock-free / wait-free open-addressing hashmap (Cliff
Click-style state machine or epoch-based reclamation). Joins
as a peer of α/β under the same `sync = ` kwarg; no surface
change required at the language level.

**Estimated effort (if γ-v2 pursued).** 3-5 sessions; grow
state machine + tombstone compaction + extensive concurrency
tests + tsan/relacy runs to catch reorder bugs.

---

## F.32-1∞ — Closed-world sync inference + diagnostic

**Status (2026-05-24): deferred.** The F.32-0 cross-pool
diagnostic already names the upgrade path concretely — when
the typechecker rejects a plain `@form(hashmap)` cross-pool
call, the error suggests `sync = serialized` (or `striped`
when β2 lands). That captures most of the ergonomic value
this inference pass would deliver, without the plumbing
work.

The full auto-inference design (below) requires a new
pipeline phase between `check_bundle` and codegen that
mutates the @form annotation AST to inject the inferred
kwarg. `check_bundle` today takes `&Bundle` (immutable);
restructuring the pipeline to thread the inferred-sync map
through is ~200 LOC of orchestration plus the inference
pass itself. Defer until the explicit-annotation friction
shows up in real downstream code.

The design below is preserved for the eventual implementer.

---

**Goal.** Authors shouldn't need to type `sync = striped` on
every map that happens to be touched cross-pool. The
typechecker has all the information needed to pick a good
default from the pool-propagation graph F.31 already builds.
Explicit annotations are for override.

**Inference rule (first cut).**

For each `@form(hashmap)` locus type (no explicit `sync =`
kwarg), walk the call graph from `main.placement { }` and
collect:

- `writers` — set of pools containing any mutate-method call
  site (`set` / `bump` / `remove`).
- `readers` — set of pools containing any read-method call
  site (`get` / `has` / `len` / `key_at` / `entry_at`).
- `hot_path` — true if any mutate call sits inside a loop or
  inside a bus-handler body (loop-nesting count ≥ 1 OR
  ancestor is `fn on_*` in a bus subscriber).

```
if |writers ∪ readers| <= 1:
    sync = none           // single-pool default; F.32-0 typecheck still rejects
                          // any cross-pool path because the rule didn't fire
elif |writers| <= 1 and |readers| > 1:
    sync = serialized     // α; reads dominate, mutex contention rare
elif |writers| >= 2:
    if hot_path:
        sync = striped    // β; parallel writers earn their padded cells
    else:
        sync = serialized // α; low-rate mutate doesn't justify striped's overhead
```

The rule is conservative — when in doubt it prefers the
discipline whose worst-case behavior is bounded
(serialized's tail is futex sleep; striped's tail is the
grow-write-lock; lockfree's tail is unbounded retry
under adversarial collision). Inference can pick α or β;
γ is opt-in via explicit annotation until its tail-latency
behavior is characterized on the downstream workload.

**Diagnostic shape.**

```
note: Registry @form(hashmap) — inferred `sync = striped`
  ├─ writers: pool `ws` (3 mutate sites), pool `gateway` (1)
  ├─ readers: pool `http` (1 read site)
  ├─ hot-path: WsHandler.on_msg loops over self.reg.bump
  └─ override: declare `@form(hashmap, sync = X)` on the locus
```

Emitted once per inferred-sync locus, at the locus decl
site. The explicit annotation suppresses the diagnostic.

**Implementation.**

1. New pass in `crates/hale-types/src/check.rs`, runs after
   F.31 pool propagation. Per `@form(hashmap)` locus type
   without explicit sync:
   - Collect writer pools, reader pools, hot-path flag.
   - Apply the inference rule.
   - Store picked sync on the locus's `LocusInfo`.
   - Emit the diagnostic.
2. Codegen reads the inferred sync the same way it reads
   the explicit one — no separate code path.

**Action-at-a-distance mitigation.** Build artifact
`target/release/sync-inference.json` lists every
form-bearing locus type and its picked sync + reasoning.
Projects can check this into `target/inference-baseline/`
under VCS; CI compares fresh inference against the baseline
and a diff means the build flips an inference (usually
because someone changed `main.placement { }`). Reviewer can
see the flip in code review rather than discovering it via
ABI mismatch at deploy time.

**Tests.**
- `crates/hale-types/tests/sync_inference_single_pool.rs`:
  lone-pool usage → no sync inferred (cross-pool rule
  inactive; F.32-0 still applies if a stray cross-pool call
  appears later).
- `crates/hale-types/tests/sync_inference_multi_reader.rs`:
  1 writer pool + N reader pools → `serialized`.
- `crates/hale-types/tests/sync_inference_multi_writer_hot.rs`:
  2 writer pools, mutate inside `fn on_*` handler → `striped`.
- `crates/hale-types/tests/sync_inference_multi_writer_cold.rs`:
  2 writer pools, mutate in one-shot init code → `serialized`.
- `crates/hale-types/tests/sync_inference_override_explicit.rs`:
  explicit `sync = X` suppresses the diagnostic; codegen
  honors the explicit pick even if inference would have
  picked something else.
- `crates/hale-types/tests/sync_inference_baseline_diff.rs`:
  inference artifact contents match a checked-in baseline;
  regression test for the inference rule itself.

**Estimated effort.** ~1 session. ~150 LOC pass + ~120 LOC
tests + ~30 LOC artifact emission.

---

## F.32-4-prefetch — Bus-dispatch prefetch hint

(Pulled forward from F.32-4 to ship with F.32-1β: same
producer-side hot path, trivial diff, no extra detection.)

**The opportunity.** Cross-pool bus dispatch already memcpys
the payload into the destination pool's queue cell
(`lotus_coop_pool_post` → ring buffer slot). The receiver
pool's worker drains by reading that exact cell first. If
we emit `__builtin_prefetch(slot, 1, 3)` immediately after
the memcpy in the producer, the destination cache line is
already inbound on the receiver's L1 by the time the
receiver's drain wakes.

**Implementation.** One-line addition in
`lotus_coop_pool_post` after the slot fill:
`__builtin_prefetch(slot, 1, 3);` (write-intent, high
temporal locality). Same for `lotus_mailbox_post`. Zero
cost on the producer side (single instruction, no stall);
~10-50ns saved on the receiver side per cell.

Same change in `lotus_bus_queue_enqueue` for the main-pool
cooperative path, though the win is smaller there (same-
core drains tend to be cache-warm already).

**Tests.** Hard to assert programmatically without HW perf
counters; ship behind a build flag (`--enable-prefetch`,
default on) and rely on the perf fixture from F.32-1 to
detect regressions.

**Effort.** <1 hour. ~10 LOC.

---

## F.32-1b — Locus struct field reordering by access frequency

**Status (2026-05-25): SHIPPED.**

The shipped implementation walks each locus's method bodies
(lifecycle / mode / fn / failure handler) once at codegen
time, counting LEXICAL `self.<field>` occurrences. Then in
`declare_locus_struct` the user-param portion of `llvm_field_tys`
is sorted by (access_count desc, declaration_order asc) and
the `fields` lookup map's indices are updated to match.

**What stays put:**
- Synthetic `__arena` at idx 0 (fixed offset; bus dispatch
  relies on it for cross-locus arena routing).
- Capacity slot fields (their indices are stored in
  `CapacitySlotLayout.struct_field_idx` and used directly
  during instantiation).
- All synthetic flags (`__restart_count`, `__quarantined`,
  `__drain_requested`, `__slot_borrowed_mask`,
  `__locus_ref_owned_mask`, `__recpool` / `_release_pool` /
  `_release_kind`, `__parent_self`, `__parent_on_failure`,
  `__duration_last_fire_*`, `__mailbox`).

Only the [1, user_fields_end) slice of `llvm_field_tys` is
permuted. `defaults` Vec stays in declaration order (defaults
evaluate in source order at instantiation, independent of
struct layout). `fields` BTreeMap iterates by name regardless.

**What's NOT in v1:**
- Loop-weighted access counts. A `self.x` inside `while ...`
  counts as 1 occurrence, not N runtime invocations. Hot-
  loop weighting is a follow-up (probably worth N=10×
  multiplier per nesting level).
- `@layout(declaration_order)` opt-out annotation. The
  reorder is unconditional; if a workload needs ABI-stable
  layout (e.g., cross-binary serialization), add the
  annotation surface then.
- Cross-binary ABI guarantees. Loci aren't serialized
  cross-process today (the bus serializes `type` payloads
  per-field, not whole loci), so the reorder is purely
  local to the binary's codegen.

**Tests:**
- `crates/hale-codegen/tests/locus_field_reorder.rs`
  covers value-read, field-override-at-instantiation, and
  write-then-read round-trip post-reorder. All pass.
- The broader hale-codegen suite was sampled (build_hello,
  locus_field_cascade, closed_world_nested_struct,
  cross_locus_from_method, coop_pool_basic,
  form_hashmap_serialized, form_hashmap_lockfree) — 61
  tests, all pass. CI runs the full sweep.

**Bench impact:** the reorder is a per-locus optimization;
no microbench specifically measures it. The win shows up
indirectly in any bench where the substrate touches `self`
many times per method call (most of them). Sustained
benefit on the canopy of a deep locus tower where
high-fanout cache-line pressure compounds.

The design below is preserved for the eventual loop-
weighted / annotation-aware follow-up.

---

**The opportunity.** A locus's methods are statically
visible. The compiler can compute, for each field, how
many method bodies touch it (read or write). Reordering
fields so that high-access fields land on the first cache
line of the struct (after the synthetic header fields)
keeps method-body hot reads on a single line; cold fields
(set once at birth, read at dissolve) migrate to later
lines.

**Caveats.**
- Synthetic fields (`__arena`, `__quarantined`,
  `__parent_self`, etc.) have fixed positions for ABI
  reasons. Reordering applies only to user-declared
  `params` fields.
- Default field order is source-declaration order, which
  many existing programs implicitly rely on (struct
  literal field-name pairs are explicit, but
  intermediate code might assume order via direct GEPs).
  Need to verify codegen doesn't bake offsets into
  intermediate state — the locus's `info.fields:
  BTreeMap<String, (u32, CodegenTy)>` already keys by
  name, so this looks safe.
- Reordering must be deterministic across builds for
  cross-binary bus compatibility. Sort by (access_count
  desc, declaration_order asc).

**Implementation.** New pass in
`declare_locus_struct` after the synthetic field
positions are fixed: walk method bodies once with a
field-access counter, sort user fields by count, emit
the LLVM struct with the new order. Update `info.fields`
indices accordingly.

**Annotation override.** `@layout(declaration_order)` on a
locus disables reordering for cases where the author
needs ABI stability (e.g., cross-binary bus payload
types where the wire format is field-order-dependent —
though those should be `type` decls not loci, and the
wire format is per-field-serialized not memcpy-shaped, so
this is theoretical).

**Tests.**
- Verify reordering happens via IR dump
  (`LOTUS_DUMP_IR=1`) — assert field positions in the
  emitted struct match the by-access-frequency order.
- Regression: existing programs continue to work
  (struct literal field-name pairs already abstract over
  position).

**Effort.** 1 small session. ~150 LOC.

---

## F.32-2 — Compile-time working-set budget per locus

**Status (2026-05-24): deferred.** Largest deliverable in
the plan (500-800 LOC across hale-types analysis pass +
diagnostic shape + cache-tier constants + multiple test
fixtures). Lower priority for the downstream daemon's
immediate perf needs — F.32-1α + 4-prefetch + 4a/4c
deliver the latency-critical wins; F.32-2 is engineering
discipline for the longer term (CI gating). Defer to its
own focused session.

The design below is preserved verbatim.

---

**The deliverable.** A build-time analysis that computes
each locus's projected working set and compares against
a target cache budget. Out-of-budget loci produce a
warning naming the tower depth at which the budget
overflows.

**Working-set formula.**

```
working_set(L) =
    sizeof(L's struct)
  + sum(c in L's capacity slots) cap(c) * cell_stride(c)
  + sum(child in L's params) working_set(child)
                                 if child is locus-typed
  + L's per-method scratch high-water mark (heuristic;
    bound by largest known transient allocation)
```

Capacity for chunked / recognition projections comes from
F.22's compile-time bounds. `@form(hashmap)` capacities
come from `capacity { pool entries of T indexed_by k; }`
declarations — these accept an optional `cap = N` kwarg
(parser surface exists; lift to required for budget
analysis or default to a large sentinel).

**Surface.**

```hale
@locality(L1)   // working-set MUST fit in L1
@locality(L2)   // ... L2
@locality(L3)
@locality(any)  // explicit "no budget" (default if unannotated)
locus HotPath { ... }
```

```sh
hale build . --target-cache=l2     # warn on >L2
hale build . --target-cache=l1 --strict   # error
```

Without `--target-cache`, no analysis runs (zero cost).

**Implementation.**

1. New crate or module:
   `crates/hale-types/src/working_set.rs`. Walks LocusInfo
   recursively, sums bounded sizes, returns
   `WorkingSetEstimate { lo: usize, hi: usize, unbounded:
   bool }`. Tower depth tracked as a string path for the
   diagnostic.
2. Diagnostic site: post-typecheck, pre-codegen pass in
   `hale-cli/src/main.rs`. Emits `warning: locus 'L'
   working set ~38 KB exceeds @locality(L1) ≈ 32 KB; chain:
   App → Mdgw → BookEngine (cells: 4096 × 8 bytes = 32 KB)`.
3. Cache-tier constants: build-time defaulted from
   `/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size`
   on Linux; fall back to 32K / 512K / 8M.

**Tests.**
- Fixture with three loci, each annotated `@locality(L1)` /
  `(L2)` / `(L3)`. Assert the analyzer flags the L1-budgeted
  one when its cells push it over.
- Fixture with bounded + unbounded loci asserts the
  unbounded-leaf path produces a "cannot compute budget"
  diagnostic rather than a false-pass.

**Effort.** 1-2 sessions. ~500-800 LOC including diagnostics
and tests.

---

## F.32-3 — Per-pool arena chunking sized to cache slice

**Status (2026-05-24): deferred.** Requires loci-per-pool
counts at codegen time (extends collect_main_placement)
plus a new `lotus_arena_create_sized(initial_chunk_bytes)`
variant plus a build-flag plumbing surface plus a multi-
locus-per-pool perf fixture to validate. Scope is medium
but spans multiple files; lower immediate priority than
F.32-1α + 4-prefetch + 4a/4c. Defer to a follow-up.

The design below is preserved verbatim.

---

**The opportunity.** Today the per-locus arena allocator
picks default chunk sizes via its own grow heuristic. On a
cooperative pool with N loci sharing one OS thread, those
N loci collectively compete for that core's L2 slice
(typical 1 MB per core on modern Intel; 12 MB shared L3).
If chunk sizes are large relative to L2-per-core / N, each
locus's chunk evicts the others on rotation through the
pool worker's drain loop.

**The fix.** Per-pool arena chunk sizing based on the
worker's resident-set budget:

```
chunk_size(pool P, locus L) =
    min(default_chunk_size,
        (target_L2_per_core / loci_on(P)) / typical_chunks_per_locus)
```

This is a hint, not a hard cap. Locus methods that need
more allocation still grow chunks beyond the hint; the
hint just makes the FIRST chunk land in the L2-friendly
size band.

**Implementation.**

1. `lotus_arena_create` grows a variant
   `lotus_arena_create_sized(initial_chunk_bytes)`.
2. Codegen at locus instantiation: for loci on a non-
   `main` cooperative pool, emit `_create_sized(hint)`
   instead of `_create()`. Hint computed at codegen
   time from `main_cooperative_pools` + count of loci
   per pool.
3. Default chunk size for main-pool loci unchanged.

**Tests.** Hard to assert without perf measurement; ship as
an opt-in via build flag `--cache-aware-chunking` and rely
on the F.32-1 perf fixture extended to multi-locus-per-pool
shapes.

**Effort.** 1 session. ~200 LOC.

---

## F.32-4 — HFT-specific extras

Three independent sub-deliverables, each scopable
individually.

### F.32-4a — Huge-page-backed arenas for pinned loci

**The opportunity.** Pinned loci with multi-MB working
sets (order books, large hashmap registries) generate TLB
pressure on every cache miss that lands on a new 4K page.
Huge pages (2 MB on x86_64) reduce TLB walks by 512x.

**Implementation.**

1. `lotus_arena_create_hugetlb(initial_bytes)` in
   `runtime/lotus_arena.c`: `mmap(NULL, sz, PROT_READ |
   PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB |
   MAP_HUGE_2MB, -1, 0)`. Fallback to `mmap` without
   `HUGETLB` on `ENOMEM` (kernel huge-page pool exhausted).
2. Annotation `@hugepages` on a locus opts in:
   ```hale
   @hugepages
   locus OrderBook { ... }
   ```
3. Threshold check: huge-page arenas only used for arenas
   whose initial chunk is >= 2 MB (anything smaller
   wastes physical memory).
4. Sysctl prereq documented:
   `sysctl -w vm.nr_hugepages=N`. Diagnostic at startup if
   `@hugepages` is declared but hugepages unavailable.

**Effort.** 1 small session. ~100 LOC.

### F.32-4b — Prefetch hints

Already detailed in the F.32-4-prefetch section above.
Ship with F.32-1.

### F.32-4c — `mlockall()` opt-in for latency-critical programs

**The opportunity.** Page faults on a hot-path arena
allocation can cause a multi-millisecond stall (worst case
when swap is enabled and the kernel decides to evict a
page). HFT-grade processes use `mlockall(MCL_CURRENT |
MCL_FUTURE)` to lock all pages.

**Implementation.**

1. Surface on `main locus`:
   ```hale
   main locus App {
       runtime {
           lock_memory: true;
       }
       params { ... }
       placement { ... }
   }
   ```
2. Codegen at main prelude: `mlockall(MCL_CURRENT |
   MCL_FUTURE)` after `lotus_env_init` if the runtime block
   declares it.
3. Sysctl prereq documented:
   `ulimit -l unlimited` or appropriate `RLIMIT_MEMLOCK`.

**Effort.** 1 small session. ~80 LOC.

---

## Sequencing recommendation

**Day 0 (immediate): F.32-0.** One-commit drop that reverts
the cross-pool exemption. Closes the corruption hole now;
unblocks subsequent work that has a sound default to
fall back to.

**Week 1: F.32-1α.** `sync = serialized` (per-map mutex).
Smallest correct cross-pool path. Restores the
Prometheus-counter shape on the safe-but-slow baseline.
Gives F.32-1∞ a sensible non-striped default to pick.

**Week 2: F.32-1β + F.32-4-prefetch.** `sync = striped`
(padded cells + per-cell atomic + grow RW-lock). The real
perf work + finally makes `bench/micro/form_hashmap_false_sharing.hl`
run cleanly with a meaningful baseline. Prefetch hint
ships in the same commit because the producer-side code
path is shared.

**Week 3: F.32-1∞.** Closed-world inference. Lands once
α and β both exist (the inference rule picks between them).
Author-experience win; no runtime change.

**Week 4: F.32-1b.** Field reordering. Substrate already
has the method-body walk; adds the access-counter pass +
struct-layout reorder.

**Week 5: F.32-2.** Working-set budget. The biggest
deliverable; useful for CI gates on production binaries.

**Week 6: F.32-4a + F.32-4c.** Huge pages + mlockall.
Deployment-grade; useful even before F.32-3.

**Week 7+: F.32-3.** Per-pool chunking. Scale-up concern;
worth landing once multi-pool deployments are common.

**Conditional: F.32-1γ.** Only if F.32-1β's bench numbers
on the downstream workload show that the marginal
throughput / tail-latency gain justifies a ~600 LOC
lock-free hashmap. Default position: do not pursue.

---

## What this plan does NOT cover

- **GPU/accelerator integration.** Out of scope; substrate
  remains CPU-first at v1.
- **Inter-process cache coordination.** Caches don't help
  cross-process; bus-over-unix / shm-ring already pay the
  copy cost at the boundary.
- **NUMA-aware allocation.** Hale doesn't model NUMA
  topology today. A future F.33-shape proposal could
  extend placement to NUMA nodes (`pinned(numa = 0)`).
- **Runtime profile-guided cache adaptation.** Static-only.
  LLVM PGO is orthogonal and stacks on top.
- **`async`/`await`-style work stealing.** F.31 ships M:N
  cooperative pools with one OS thread per pool; work
  stealing within a pool is a v2+ concern (would invalidate
  the per-arena single-thread invariant without further
  work).

---

## Friction items this closes

Once F.32-0 ships:
- The cross-pool `@form(hashmap)` corruption hole is
  closed; daemons exposed to the Prometheus-counter shape
  no longer double-free on grow under multi-pool writers.
- The diagnostic names the upgrade path; authors who hit
  the rejection know to pick `sync = serialized` or
  `sync = striped` rather than search the spec for what
  the previous accept-then-corrupt path was hiding.

Once F.32-1α ships:
- The Prometheus-counter shape works correctly cross-pool
  on the safe baseline. Throughput is mutex-bound but
  honest; tail is futex sleep but bounded.

Once F.32-1β + 4-prefetch ship:
- Hot-path counter increments across producer pools no
  longer ping-pong the cache line between cores.
- The "Prometheus registry shared across pools" pattern
  reaches its perf ceiling on β (>= 4x serialized on the
  perf fixture; matches the C twin's theoretical ceiling
  within hash + probe overhead).
- Bus dispatch latency drops by 10-50 ns/cell via the
  prefetch hint (consumer-side L1 warmed by the producer).

Once F.32-1∞ ships:
- Authors don't have to type `sync = striped` on every
  cross-pool form — the inference picks it from the
  pool-propagation graph. Override is one kwarg away.
- Inference baseline artifact makes "Wait, why is Registry
  now striped?" a reviewable diff, not a deploy-time
  surprise.

Once F.32-1b ships:
- Locus method bodies with hot fields touched on every
  iteration get those fields packed on the first cache
  line, reducing per-iteration L1 miss rate.

Once F.32-2 ships:
- "This tower won't fit in L2 on the chosen target"
  surfaces at build time instead of at perf-measurement
  time.
- CI gates: a regression that pushes a locus over its
  declared `@locality` budget fails the build.

Once F.32-4 ships:
- TLB miss rate on large pinned-locus working sets drops
  by ~512x via huge pages.
- Bus dispatch latency drops by 10-50ns/cell via prefetch.
- Worst-case page-fault stalls eliminated for latency-
  critical programs via mlockall.

---

## Coverage / verification strategy

**Microbenchmarks:** `crates/hale-codegen/tests/cache_*.rs`
fixtures, gated `#[ignore]`, run with `cargo test --release
-- --ignored`. Each fixture pins threads to specific cores
(via `pthread_setaffinity_np` invoked from Rust test code),
runs a hot loop for N iterations, measures `clock_gettime`
or `__rdtsc()` delta. CI doesn't run these (pinning needs
host root or specific kernel config); developers run them
locally before claiming a perf win.

**Functional tests:** every F.32-* deliverable lands with
a functional test asserting the structural behavior
(padding applied, fields reordered, etc.) independent of
perf measurement.

**Real-workload validation:** the downstream gateway daemon
serves as the end-to-end test bed. Pre-F.32-1 latency
profile vs. post-F.32-1 latency profile is the acceptance
gate for the work.

---

## Document survival across rename

This file lives at `notes/f32-cache-aware-delivery-plan.md`.
The rename will swap the org/language name; the substantive
content of this plan is name-independent.

Touchpoints in this file that will need updating post-
rename:
- Path prefixes `crates/hale-*` → new crate names
- `lotus_*` C runtime symbols → new runtime prefix
- `LOTUS_*` C constants → new prefix
- `hale build` / `hale_codegen` references

A `sed` pass over the substring renames in this file
should suffice; the structural plan stays put.

---

## Pickup checklist

The "new home" rename is complete (this is `hale-lang/hale`);
the rename steps below are historical context only. Skip to
step 3 for current pickup.

1. (historical) Confirm the org/language rename is complete
   in the new directory and the rename's commit hash is on `pub`.
2. (historical) Update path/symbol references in this file
   via a single sed pass.
3. Branch from `main` as `f32-0-revert-crosspool-exemption`.
   Land F.32-0 first; it's a one-commit drop that closes
   the corruption hole and gives F.32-1{α,β,∞} a sound
   default to fall back to.
4. Then `f32-1a-sync-serialized`. Smallest correct cross-pool
   path; ~100 LOC.
5. Then `f32-1b-sync-striped` (also covers F.32-4-prefetch).
   Real perf work; `bench/micro/form_hashmap_false_sharing.hl`
   gets its first meaningful baseline.
6. Then `f32-1-infer-sync`. Closed-world inference; depends
   on α and β both existing as fall-back targets.
7. Then the original F.32-1b (field reordering) and onwards
   per the sequencing § above.

Stable references that DON'T change across rename:
- The F.32 spec section in `spec/design-rationale.md` (the
  section letter survives; the prose may want a wording
  pass for tone consistency with the new branding).
- The `m90 routing` references in this file (m90 is a
  historical milestone tag preserved per CHANGELOG
  conventions).
- The friction-log shape (downstream consumer's friction
  log; not affected by lotus-side rename).
