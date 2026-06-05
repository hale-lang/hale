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

## Roadmap

PoC done (lockfree hashmap). Remaining substrate primitives, by
model-checking value (see the inventory in the issue thread):

1. **Mailbox** (`lotus_arena.c`, pinned-locus bus) — mutex + condvar; a
   canonical pattern, good second model to validate the methodology on
   lock-based code.
2. **Bus queue** (cooperative pool) — `g_bus_has_pinned`-gated mutex;
   producers + drainer.
3. **Chunk pool / arena locks** — lower interleaving risk; model if a
   regression appears.

**CI wiring** (follow-up): GenMC takes a few minutes to build, so a CI
gate should build it once (cached on the LLVM-18 toolchain image) in a
dedicated job, then run `run_genmc.sh`. Until that lands, the PoC is
run locally / on demand. The `build_genmc.sh` + `run_genmc.sh` pair is
the CI recipe.
