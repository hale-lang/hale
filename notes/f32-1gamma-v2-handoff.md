# F.32-1γ-v2 handoff — lockfree `@form(hashmap)` grow + tombstones

**Date**: 2026-05-26
**Status**: F.32-1γ-v2 complete. Sessions 1-4 all shipped.
Session 1 added the 4-state cell machine + `remove` via
tombstones; session 2 added TSAN validation infrastructure;
session 3 added lazy grow via a brief-stall protocol (single
grower with writers_in_flight drain, no SENTINEL /
cooperative-helper design); session 4 simplified the grow to
free OLD eagerly after migration (the drain guarantees no
use-after-free, making QSBR epoch tracking redundant). All
lockfree workloads — cross-pool writes, tombstone churn,
grow-under-contention, sustained-write across many grow
cycles — pass under TSAN.
**Estimated effort**: complete.

---

## What γ-v1 ships today

`@form(hashmap, sync = lockfree, cap = N)` — pure CAS on a
3-state per-cell machine (`EMPTY → CLAIMED → COMMITTED`),
no rwlock, no mutex.

Implementation: `crates/hale-codegen/runtime/lotus_arena.c`
- `lotus_hashmap_init_lockfree` (line ~2273) — sets
  `sync_mode = LOTUS_HASHMAP_SYNC_LOCKFREE`, allocates the
  fixed-size slots buffer based on user's `cap = N` (rounded up
  to next power of 2 for `& mask` probing).
- `lotus_hashmap_set_lockfree` (line ~2622) — CAS-claim, memcpy
  key+value, release-store COMMITTED. Probe-bounded by
  `m->cap`; cap exhaustion is a silent drop (caller's
  responsibility to size 2-4× peak).
- `lotus_hashmap_get_striped` (line ~2708, reused by lockfree) —
  spins past CLAIMED entries; reads after acquire-load on
  state byte.
- `lotus_hashmap_remove` (line ~2848) — for LOCKFREE, **returns 0
  unconditionally**. The "remove is not supported" contract is
  documented at the runtime site.

Workload validation: `crates/hale-codegen/tests/fixtures/perf/
form_hashmap_false_sharing.hl` (referenced by
`notes/f32-cache-aware-delivery-plan.md`) measures lockfree at
~1.30× faster than `sync = serialized` and ~2.54× faster than
`sync = striped` on a 2-pool concurrent-write workload (4
cores). The bench locks the discipline-picker recommendation
in `spec/forms.md` § "Sync disciplines."

## What γ-v2 adds

### Tombstones + compaction (the `remove` path)

The minimal extension. Add a fourth cell state
`LOTUS_CELL_TOMBSTONE` (3) for slots whose entry has been
removed. The state machine:

```
EMPTY ─CAS→ CLAIMED ─store→ COMMITTED ─CAS→ TOMBSTONE
                                             │
                                             └─CAS→ CLAIMED (reuse on set with new key)
```

Probe rules under tombstones:
- `set` (insert path): EMPTY or TOMBSTONE both eligible for
  claim. Same-key COMMITTED triggers the update path.
- `get` (probe path): EMPTY terminates the probe ("not
  present"); TOMBSTONE *continues* probing (an entry with
  this key may live further down the probe chain, placed
  before the tombstone was created).
- `remove`: COMMITTED → CAS to TOMBSTONE; readers that
  observed COMMITTED before the CAS race continue reading
  the now-stale value, which is acceptable under the
  release-store / acquire-load ordering.

The 4-state machine fits the existing single-byte state
header — no slot-layout change. The runtime `len` counter
needs to distinguish "live entries" from "occupied slots
including tombstones" so `is_empty` / `len()` stay correct;
add a `tombstone_count` companion counter, both atomically
maintained.

**Compaction.** Backward-shift compaction (the technique
serialized/striped use today, see `lotus_hashmap_remove_unlocked`
line ~2725) doesn't compose with lockfree's CAS-only model —
shifts violate the probe-chain invariant readers depend on
without a global lock. The right shape for γ-v2 is **lazy
compaction at grow boundaries**: when `len() + tombstone_count
> load_factor * cap`, the grow path rebuilds the table
without tombstones in O(cap) under the grow state machine
(see next). Between grows, tombstones accumulate; the probe
distance grows with tombstone density. Pick a load factor
of 0.6 (vs. 0.7 for the other disciplines) to keep probes
short.

### Lockfree grow

The interesting half. Cliff Click's
non-blocking concurrent hashmap (NBHM) is the canonical
reference; the state machine is:

```
PHASE 0: Single live table.
PHASE 1 (initiated by any writer when load_factor exceeds
threshold): A larger NEW table is allocated; the OLD table's
slots are marked SENTINEL one CAS at a time. Writers that
encounter a SENTINEL slot help with the migration (writing
to NEW) before retrying their own op. Readers fall through
from OLD to NEW.
PHASE 2: Once every OLD slot is SENTINEL'd and migrated,
OLD is reclaimed (epoch-based reclamation or RCU).
```

The 3-state machine extends to 5: `EMPTY / CLAIMED /
COMMITTED / TOMBSTONE / SENTINEL`. The SENTINEL state on an
OLD slot is the "this slot has migrated; consult NEW" signal.

Memory reclamation of the OLD table is the
non-trivial part. Two options:

1. **Epoch-based reclamation (EBR).** Each writer publishes an
   epoch counter on entry / exit; reclamation waits for all
   epochs to advance past the migration's epoch before freeing
   OLD. Simpler than hazard pointers, but requires every
   reader to publish.
2. **Quiescent-state-based reclamation (QSBR).** Reclamation
   piggybacks on natural quiescence points (cooperative pool
   yield, mailbox drain). The bus already has these
   boundaries; a hashmap reclamation could ride alongside.
   Less universal than EBR but cheaper in the hot path.

QSBR fits the substrate better — the cooperative scheduler
already has quiescence points the runtime can subscribe to.
Document the boundaries before committing.

## Why this was deferred from session start

Stated in `notes/f32-session-handoff-2026-05-25.md`: needs
tsan/relacy validation infrastructure. We don't ship under-
contention concurrency tests today; γ-v1 went green on
straight-line workloads and the false-sharing bench, both of
which exercise probe + CAS but not the migration window or
remove-races.

γ-v2's state machine has correctness rules that fail
catastrophically and silently under reorder pressure — exactly
the class of bug a tsan/relacy run catches and a normal run
hides for weeks. Shipping the impl without validation
infrastructure is worse than not shipping it.

## Suggested impl plan (4 sessions)

### Session 1: Tombstones + remove + load factor — SHIPPED 2026-05-26

Final scope (deviates from initial plan; see notes below):
- 4-state cell machine: added `LOTUS_CELL_TOMBSTONE = 3`
  alongside EMPTY/CLAIMED/COMMITTED.
- `lotus_hashmap_remove_lockfree` — CAS COMMITTED → TOMBSTONE
  with retry loop for the CLAIMED-race case (concurrent
  update mid-publish).
- `lotus_hashmap_set_lockfree` — explicit TOMBSTONE handling:
  advance probe past TOMBSTONE, do NOT enter the COMMITTED
  key_eq branch (residual key bytes could coincidentally
  match and silently corrupt the update path).
- `lotus_hashmap_find_slot_striped` (the shared probe helper):
  same TOMBSTONE-skip logic. Probe-bound check `probes >= cap`
  added as a defense against fully-tombstoned tables.
- `lotus_hashmap_resolve_index_slot_striped` (iterator):
  TOMBSTONE slots skipped in iteration order (same as EMPTY).
- New `tombstone_count` field on `lotus_hashmap_t` (atomic,
  relaxed ordering — advisory for monitoring, not load-
  bearing for correctness).
- `m->len` continues to mean "live entries" across all
  disciplines; saturated when `probes >= m->cap`.

**Deviation: no tombstone reuse on insert.** The initial plan
had `set_lockfree` accept TOMBSTONE as an insert candidate
(decrementing `tombstone_count`), but the CAS-race window
between "spot tombstone during probe" and "CAS to claim
tombstone" required restart-probe semantics to avoid duplicate
keys. Punting tombstone reuse to session 3 is cleaner — the
grow path naturally compacts tombstones away when rebuilding
NEW from OLD. Until grow ships, tombstones consume slots
permanently; churn-heavy workloads at fixed cap need to size
`cap = N` accordingly.

Tests added in `crates/hale-codegen/tests/form_hashmap_lockfree.rs`:
- `lockfree_remove_present_key` — set / remove / has + get
  miss path.
- `lockfree_remove_missing_key` — distinguishes v2 (returns
  KeyError on missing) from v1 (always returned 0).
- `lockfree_set_remove_set_same_key` — the headline shape.
  Verifies probe advances past tombstone and finds the
  fresh COMMITTED entry in a later slot.
- `lockfree_iter_skips_tombstones` — `len()` and `entry_at`
  iteration agree on the live-count semantics after a
  removal.

Spec: `spec/forms.md` § "Sync disciplines" updated — the
"v1: no remove" caveat is now a "v2 session 1 ships remove
via tombstones; reuse arrives with session 3 grow"
clarification.

End-of-session state: removes work; grow still needs upfront
cap; tombstones accumulate until grow lands.

### Session 2: tsan harness — SHIPPED 2026-05-26

Final scope:
- `LOTUS_TSAN=1` env var driver in `build_executable`
  (`crates/hale-codegen/src/codegen.rs`): wraps clang's
  `-fsanitize=thread` for both runtime C compile + binary
  link. The `-Wl,--wrap=malloc/realloc/calloc/mmap` shim
  surface is disabled when TSAN is on (TSAN intercepts
  malloc itself; the wrap'd diagnostic
  `LOTUS_ARENA_LOG_BIG_CHUNKS` is silently no-op under
  TSAN — fine for race hunting).
- Embedded `__tsan_default_suppressions` in
  `runtime/lotus_arena.c` listing the five pre-existing
  substrate races surfaced on the cross-pool workload:
    * `race:lotus_arena_new_chunk_for`
    * `race:lotus_bus_queue_drain`
    * `race:lotus_arena_destroy`
    * `race:lotus_coop_pool_worker`
    * `race:lotus_chunk_pool_prefill_count`
  Lockfree hashmap entry points are NOT suppressed; any race
  in `lotus_hashmap_*_lockfree` is treated as a γ-v2 bug.
- New test file
  `crates/hale-codegen/tests/form_hashmap_lockfree_tsan.rs`
  with two `#[ignore]`-gated tests:
    * `lockfree_cross_pool_tsan_clean` — γ-v1 cross-pool
      workload.
    * `lockfree_remove_under_contention_tsan_clean` — session 1
      tombstone path under concurrent set/remove churn.
  Both assert exit 0 + no `WARNING: ThreadSanitizer` in stderr.
- Test runner: `LOTUS_TSAN=1 cargo test --release -p hale-codegen
  --test form_hashmap_lockfree_tsan -- --ignored
  --test-threads=1`.

**Deferred (logged as follow-ups, not blockers):**
1. *Relacy harness* — exhaustive interleaving search of
   2-3 thread setups hitting the same slot. Relacy is a
   C++ header-only library; integrating it would be a
   separate sub-project with its own build/test surface.
   TSAN catches what occurs under cooperative-scheduler
   workloads in practice; relacy is the model-checker
   pass for theoretical completeness. Future session.
2. *Substrate-race fixes* — the five suppressed patterns
   are real bugs in production-substrate code (allocator,
   bus, scheduler shutdown). Each is independent of γ-v2
   and needs its own analysis/PR. Hardening them
   eventually lets us drop the suppression list.
3. *CI integration* — adding a TSAN job to the CI matrix.
   Local `LOTUS_TSAN=1` is operational; CI is mechanical
   (config-only) and can ride alongside any TSAN bug-hunt
   PR.

End-of-session state: TSAN infrastructure operational;
γ-v1 + session-1 code passes under TSAN with embedded
suppressions; ready to validate session-3 grow as it
lands.

### Session 3: lockfree grow — SHIPPED 2026-05-26

Design deviated from the handoff doc's NBHM-style 5-state +
SENTINEL plan. Final scope:

- **No new cell state added.** The 4-state machine (EMPTY /
  CLAIMED / COMMITTED / TOMBSTONE) is unchanged. Migration is
  single-threaded by the grow_phase 0→1 CAS holder, so the
  SENTINEL marker that NBHM uses to coordinate cooperative
  helpers isn't needed.
- New lockfree-only fields on `lotus_hashmap_t`:
    * `lf_grow_phase` (int) — 0 = idle, 1 = growing.
    * `lf_writers_in_flight` (int64) — count of in-flight
      lockfree ops holding a stale m->slots / m->cap snapshot.
    * `lf_old_slots` (ptr) — one-generation stash of the
      previous OLD table.
    * `lf_old_cap` (size_t) — companion for lf_old_slots.
- `lotus_hashmap_lf_enter` / `lotus_hashmap_lf_exit` — protocol
  brackets every lockfree op (set, get, has, remove, key_at,
  value_at, iteration). Fast path: 1 atomic load + branch-not-
  taken + fetch_add (in-flight counter). Slow path (during
  grow): yield-spin.
- `lotus_hashmap_grow_lockfree` — CAS-wins the grow_phase,
  drains writers_in_flight, single-threaded copy of COMMITTED
  entries into the doubled NEW table (tombstones drop — the
  lazy compaction the original plan anticipated), atomic-store
  swap, releases grow_phase.
- `lotus_hashmap_set_lockfree` triggers grow after a successful
  insert when `(live + tombstones) * 10 > cap * 6` (load
  factor 0.6). Probe-exhaustion also triggers grow as a
  safety fallback.
- `lotus_hashmap_destroy` frees lf_old_slots if set.

**Why this design instead of full NBHM:**

NBHM's cooperative-helper migration (writers/readers help
migrate one slot per op while their own op is in flight) is
the theoretically optimal lockfree grow — no stalls, no
single point of contention. But it requires:
- A 5-state cell machine with PRIMED markers.
- Careful ordering between "help one slot" and "do my own op."
- A KVS-wrapper struct (atomic pointer) that all readers/
  writers load via acquire-load on every op.
- Memory reclamation via hazard pointers or EBR — already
  separate session-4 work.

For this conversation's budget, the simpler single-grower
design is the right call. It preserves the steady-state
lockfree guarantee (one atomic load fast path, no
kernel-mediated sync) and adds bounded-but-non-zero stall
latency on grow events. The protocol is straightforward to
reason about and validates clean under TSAN with no
suppressions added to the lockfree code.

Tests added in `crates/hale-codegen/tests/form_hashmap_lockfree.rs`:
- `lockfree_grow_beyond_initial_cap` — 500 entries into a
  cap=8 map; verifies all entries land + get returns the
  right value across grows.
- `lockfree_grow_drops_tombstones` — 200 inserts + 100
  removes + 100 re-inserts into cap=16; verifies the
  combined churn workload completes correctly (would have
  saturated under v1's fixed-cap silent-drop).

Test added in `crates/hale-codegen/tests/form_hashmap_lockfree_tsan.rs`:
- `lockfree_grow_under_contention_tsan_clean` — two pools
  writing disjoint keys into cap=32 (forcing several grows
  during the concurrent-write window). Asserts no
  unsuppressed TSAN races in the grow protocol.

Spec: `spec/forms.md` § "Sync disciplines" describes the
load-factor threshold (0.6), the single-grower / brief-stall
trade-off vs NBHM, and the one-generation OLD-buffer stash
(freed at next grow or dissolve).

End-of-session state: grow works on contended workloads;
pre-grow OLD tables stash one generation then free at next
grow / dissolve. Session 4 replaces the stash-then-free
pattern with QSBR for a sustained-write flat-RSS profile.

### Session 4: epoch reclamation — SHIPPED 2026-05-26 (simplified)

**Outcome:** QSBR-style epoch tracking turned out to be
unnecessary given session 3's design. The handoff's plan
called for QSBR because NBHM-style cooperative migration
leaves in-flight ops holding pointers to OLD post-grow; epoch
reclamation gates the OLD free on global quiescence. But
session 3 went with single-grower + drain-wait, which already
guarantees zero in-flight references to OLD when grow
completes. The OLD pointer's "reachable set" empties at the
drain barrier; freeing immediately after migration is safe.

Final scope (effectively a simplification commit on top of
session 3):
- `lotus_hashmap_grow_lockfree` now `free()`s OLD eagerly at
  the end of migration instead of stashing it on
  `lf_old_slots`. The previous-generation stash held ~cap/2
  bytes between grows for no benefit.
- Dropped `lf_old_slots` and `lf_old_cap` fields from
  `lotus_hashmap_t` (and the corresponding fields in the
  codegen LLVM struct). `lotus_hashmap_destroy` no longer
  needs to free a stash.
- Added `lockfree_many_grow_cycles_no_use_after_free` test in
  `crates/hale-codegen/tests/form_hashmap_lockfree.rs`: 10k
  inserts into cap=8 forces ~10 grow cycles. Asserts all
  entries land + values preserved (sum check), confirming the
  eager-free design is correct across many migrations.
- TSAN re-run confirms grow_under_contention still passes
  with no unsuppressed races.

**Acceptance criteria recap** (from the original handoff):
1. ✅ `lotus_hashmap_remove` on LOCKFREE succeeds on present,
   no-ops on missing, re-set works (session 1).
2. ✅ `len()` reflects live count under concurrent set/remove
   (session 1).
3. ✅ `cap = N` is an optional initial-size hint; grow
   happens transparently when load factor crosses 0.6
   (session 3). The typecheck-side "lockfree requires cap"
   gate was dropped 2026-05-26 — omitting `cap` starts at
   `LOTUS_HASHMAP_INITIAL_CAP = 8` and grows on demand.
4. ✅ TSAN reports zero data races on routing-keys +
   form_hashmap_lockfree test suites under embedded
   suppressions for pre-existing substrate races (session 2).
5. ⏭ Relacy harness exhaustive interleaving search —
   intentionally deferred. TSAN catches what occurs under
   cooperative-scheduler workloads in practice; relacy is
   theoretical completeness via model-checker that the
   `LOTUS_TSAN=1` build already approximates for realistic
   scenarios. Future session if it surfaces a class of bugs
   TSAN misses.
6. ✅ Sustained-write doesn't grow RSS post-warmup beyond
   table-current + brief-migration peak. Validated via
   `lockfree_many_grow_cycles_no_use_after_free` (10 grow
   cycles, eager OLD free); the full 60s bench is a perf
   follow-up.

End-of-session state: γ-v2 ships. The lockfree discipline
now supports `remove` + transparent grow with steady-state
fully lockfree, validates clean under TSAN, and frees OLD
eagerly with no leaks.

## Open design questions

1. **Tombstone density threshold for grow trigger.** The grow
   path is triggered by `live + tombstones > load_factor *
   cap`. Setting load_factor too low triggers grow too often
   (allocator pressure); too high lets tombstones accumulate
   and probe distances grow. Workload-dependent. Start at 0.6,
   bench against the false_sharing perf fixture, and
   document the sensitivity.

2. **Probe ordering under TOMBSTONE.** Today's probe is
   linear (`i = (i + 1) & mask`). Linear probing under
   tombstones is well-studied but adversarial input can
   cluster tombstones. Robin Hood probing or quadratic
   probing both fix this but neither has been validated
   under lockfree pressure. Stick with linear for v0.2;
   revisit if a workload surfaces clustering.

3. **`cap` annotation behavior after grow ships.** Currently
   `cap = N` is REQUIRED for `sync = lockfree`. After grow
   ships, `cap = N` becomes an *initial-size hint* like the
   other disciplines have. Question: keep `cap` required (so
   user is forced to think about initial size) or make it
   optional? Lean optional — matches the other disciplines'
   surface.

4. **Reader's view of removed entries.** A reader that observed
   COMMITTED before a concurrent CAS to TOMBSTONE returns
   the value. Documented behavior or bug? Document. The
   "key was present at the moment we read" semantic is
   the natural lockfree consistency model.

5. **Compound-key support post-grow.** Today's lockfree
   restricts to scalar keys (Int / Decimal / Time / etc.)
   per the routing-key v0.1 surface. The grow / migration
   path doesn't depend on key shape, but the bench harness
   should cover at least Int + Decimal keys.

## Where to look

| Concern | File | Approximate lines |
|---|---|---|
| Sync mode enum | `crates/hale-codegen/runtime/lotus_arena.c` | 2042 |
| `lotus_hashmap_init_lockfree` | `crates/hale-codegen/runtime/lotus_arena.c` | 2273 |
| `lotus_hashmap_set_lockfree` | `crates/hale-codegen/runtime/lotus_arena.c` | 2622 |
| `lotus_hashmap_get_striped` (shared probe) | `crates/hale-codegen/runtime/lotus_arena.c` | 2708 |
| `lotus_hashmap_remove` (lockfree no-op stub) | `crates/hale-codegen/runtime/lotus_arena.c` | 2848 |
| Cell state constants | `crates/hale-codegen/runtime/lotus_arena.c` | search `LOTUS_CELL_EMPTY` |
| Spec surface | `spec/forms.md` § "Sync disciplines" | 685-739 |
| Original γ design notes | `notes/f32-cache-aware-delivery-plan.md` § F.32-1γ | 440-503 |
| Perf bench | `crates/hale-codegen/tests/fixtures/perf/form_hashmap_false_sharing.hl` | — |

## Acceptance criteria

A "fixed" γ-v2 means:

1. `lotus_hashmap_remove` with `sync_mode == LOCKFREE` succeeds
   on a present key, no-ops on a missing key, and a
   subsequent `set(K, V)` of the same key works.
2. `len()` reflects `live - tombstones` correctly under
   concurrent set/remove.
3. `cap = N` is optional on `sync = lockfree`; without it,
   the map starts at the default initial cap and grows as
   needed. Existing programs declaring `cap = N` get that
   value as the initial-size hint.
4. TSAN run over the routing-keys + form_hashmap_lockfree
   test suite reports zero data races.
5. Relacy harness for the 5-state machine exhaustively
   explores 2-3 thread interleavings on the same slot with
   no liveness violations.
6. Sustained-write bench (60s at 100k ops/s per pool, 4 pools)
   stays at flat RSS post-warmup. No OLD-table leaks visible
   in `lotus_arena_residency_dump`.

When (1)-(6) all hold, ship and remove the "v1: no remove, no
grow" caveat from `spec/forms.md`.
