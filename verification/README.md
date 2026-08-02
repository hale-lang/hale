# Substrate race-completeness (GH issue #18 item 2)

Model-check the lotus runtime's concurrent primitives under **every**
thread interleaving the C11 memory model permits — to catch races and
use-after-frees that TSan only hits along paths a test happens to
exercise. This directory holds the proof-of-concept that settles the
tooling question and validates the approach on the first primitive.

## Tool: GenMC (decided)

The issue's open question was *TLA+ vs Loom vs hand-rolled*. The
substrate is **C** (pthreads + C11 atomics), which decides it:

- **GenMC** — a stateless model checker that runs the *actual C* under
  all interleavings of the C11 model, catching data races and
  use-after-free automatically. **Chosen.** The primitives are written
  in clean C11 atomics with no syscalls on the hot path, so they model
  directly.
- **Loom** — Rust-only. Would require porting each primitive to Rust
  and keeping two implementations in sync. **Rejected** (the substrate
  is C; we want to check the code that ships).
- **TLA+** — an *abstract* spec, not the C; smaller state space, good
  as documentation, but the spec-to-code mapping is unverified.
  **Complementary, not primary** — write it later if a primitive wants
  a formal design record.

GenMC v0.17.0 builds against the project's LLVM 18. See
`build_genmc.sh`.

## What's here

| Model | Mirrors | Status |
|-------|---------|--------|
| `lockfree_hashmap_model.c` | `lotus_hashmap_*_lockfree` in `crates/hale-codegen/runtime/lotus_arena.c` — the enter/exit writer-counter protocol, single-grower grow phase, writers-in-flight drain, and EMPTY→CLAIMED→COMMITTED set state machine | ✅ verified: **42 executions, no errors** |
| `mailbox_model.c` | `lotus_mailbox_*` / `lotus_mpsc_ring_*` in `lotus_arena.c` — the pinned-locus mailbox: a **lock-free Vyukov MPSC ring + signal-when-parked wake** (the `_Atomic int parked` flag + the seq_cst fences in post/drain_one, plus the missed-wakeup handshake) | ✅ verified: **10 executions, no errors** |
| `coop_pool_model.c` | `lotus_coop_pool_*` / `lotus_mpsc_ring_*` in `lotus_arena.c` — the cooperative pool's classic consumer path after Phase-1b swapped the cell-buffer-under-mutex queue for the same lock-free Vyukov MPSC ring + signal-when-parked wake; the coop-pool twin of `mailbox_model.c` (byte-identical ring + wake handshake) | ✅ verified (mailbox-twin ring + wake) |
| `bus_queue_model.c` | `lotus_bus_queue_*` in `lotus_arena.c` — the cooperative-pool queue's `g_bus_has_pinned`-gated **conditional lock** on enqueue/drain (concurrent enqueues under the lock; drain snapshots under the lock) | ✅ verified: **2 executions, no errors** |
| `arena_subregion_model.c` | `lotus_arena_create_subregion` / `lotus_arena_destroy` in `lotus_arena.c` — the per-parent `subregion_lock` guarding the child-slot freelist (`free_list` / `free_count` / `next_slot`): concurrent create (pop-or-bump) + destroy (push) on the same parent must never hand the same slot to two live children. The per-thread chunk pool itself is `__thread` (no cross-thread surface); this is the real "arena locks" surface. | ✅ verified: **6 executions, no errors** |

## Coverage gaps and drift (audited 2026-08-02)

The models are **transcriptions**, not the shipping code, and nothing
detects when the two diverge. A model that still passes proves a
property of whatever `lotus_arena.c` looked like when the model was
written. Audited by extracting each mirrored function's
synchronization skeleton (atomics, fences, mutexes) at the model's
commit and at HEAD:

| Model | Skeleton drift |
|---|---|
| `arena_subregion_model.c` | none — the transcribed protocol is unchanged |
| `lockfree_hashmap_model.c` | new untranscribed atomics in `lotus_hashmap_iter_next` / `iter_batch` (iteration under concurrent writers) |
| `mailbox_model.c` | new untranscribed atomics in `lotus_mpsc_ring_peek_nonempty` / `size_approx` |
| `coop_pool_model.c` | `lotus_coop_pool_enable_async_io` lost its atomic |
| `bus_queue_model.c` | enqueue body moved into `bus_queue_enqueue_inner`; protocol shape preserved, but it gained a **third** conditional-unlock path for the bounded-capacity branch (#255) that the model predates |

Also untranscribed: `lotus_bus_queue_enqueue_st`, the **zero-
synchronization** enqueue used when codegen proves no thread crosses
the bus. Its safety is a *compiler* property (`no_pinned ==
!program_has_offthread`), not a runtime protocol, so GenMC cannot
speak to it. `crates/hale-codegen/tests/bus_devirt_no_pinned.rs` pins
it by example — two specific programs — rather than structurally. The
residual risk is `program_has_offthread` becoming incomplete: the
runtime has a **second** writer of `g_bus_has_pinned`
(`lotus_coop_pool_start_all`, gated on `g_coop_pool_count > 0`) whose
correspondence to the compile-time predicate nothing asserts.

`spsc_ring_model.c` runs (the runner globs `*_model.c`) but has no row
in the table above.

None of this means a model is wrong. It means "verified" in the table
below is a claim about a past revision, and re-transcription is owed
before it can be read as a claim about HEAD.

## How to run

```sh
verification/build_genmc.sh            # one-time: build GenMC (LLVM 18 + cmake)
GENMC=/tmp/genmc/build/bin/genmc verification/run_genmc.sh
```

`run_genmc.sh` exits non-zero if any model reports a race / UAF /
assertion violation, so it works as a CI gate (see "CI" below).

## Why trust a passing model: the negative control

A model that passes only matters if it would *fail* on a real bug. The
PoC confirms GenMC has teeth: deleting the grower's drain-wait (the
`while (writers_in_flight > 0)` spin) — so a writer can still touch
`old_slots` while grow frees it — makes GenMC report a **safety
violation (use-after-free)** within the first executions. The protocol
is exactly what prevents that, and the checker proves it does, across
all interleavings.

The verified property set, all automatic under GenMC:

- **no data race** — no conflicting non-atomic accesses;
- **no use-after-free** — no op reads/writes `slots` after grow frees
  the old table (the reason the enter/drain protocol exists);
- **no lost insert / corruption** — hand-written invariants in the
  harness `main()` (both distinct keys survive, `len` agrees).

## Crucial caveat: the model is a transcription

Each `*_model.c` is a **hand-written transcription** of the
synchronization core, not the production code — it strips payloads,
hashing, tombstones, and the striped/serialized modes, keeping exactly
the atomics and orderings that decide thread-safety. **If the
production atomics or memory orderings change, the model must change
with them**, or the proof is about stale code. Treat a model edit as
mandatory whenever you touch the corresponding `lotus_*` atomics. The
top of each model names the functions it mirrors.

State space is kept small on purpose (2 writers, `cap=2` forcing one
grow) — exhaustive checking is exponential in threads/operations, and
the grow-during-write interleaving is the whole race surface. Larger
configurations are for a deeper sweep, not the gate.

## A note on condition variables

GenMC does not model `pthread_cond_*` (condition variables are a
liveness mechanism; they're commented out of its runtime `pthread.h`).
The mailbox and coop-pool ring are **lock-free** — a Vyukov bounded
MPSC ring with no mutex on the handoff; the only condvar left is the
signal-when-parked *wake* handshake that nudges a parked consumer.
There the condvar governs only *sleep-vs-spin* — the **safety**
properties (no lost/duplicated cell, no use-after-free) are independent
of it. So those models replace the `cond_wait`/`signal` wake with a
lock-guarded spin that preserves the exact parked/wake predicate, and
check safety under all interleavings. Missed-wakeup *liveness* is out
of scope for GenMC and would need a different tool (e.g. a TLA+ spec of
the wake handshake).

## Roadmap

Five primitives modeled: lockfree hashmap, mailbox, the coop-pool ring,
bus queue, and the arena subregion-slot lock. That covers the inventory in the issue thread
— the last entry, **"chunk pool / arena locks,"** resolved to the
`arena_subregion_model.c` above: the chunk pool proper is `__thread`
(thread-local, no cross-thread interleaving to check — its one historical
race, the env-driven prefill lazy-init, is a `pthread_once` and raceless
by construction), so the meaningful surface is the parent arena's
child-slot freelist lock, now modeled. No primitive with a cross-thread
synchronization surface is left unmodeled; add a model alongside any new
one.

**CI gate (live).** The `genmc` job in `.github/workflows/tests.yml`
builds GenMC (cached on `build_genmc.sh`, ~3 min on a cold cache) and
runs `run_genmc.sh` on every push/PR, so a race / UAF / assertion
violation in any model fails the build. New models are picked up
automatically. `build_genmc.sh` + `run_genmc.sh` are also the local
recipe.
