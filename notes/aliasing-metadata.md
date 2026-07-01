# Aliasing metadata from locus invariants

Status: stage 1 SHIPPED (2026-07-01, this branch). Stages 2–3 are
design notes for the region-aware mid-end.

Hale's semantics carry aliasing guarantees that are stronger than
what Rust's borrow checker proves — no user pointers, vertical-only
flow, single-threaded-method invariant, per-locus arenas that never
alias each other, bus payloads that are fresh copies. Until stage 1,
none of that reached LLVM: the emitted IR carried zero aliasing
metadata, so LLVM assumed any pointer might alias any other and any
call might unwind. This file tracks the staged plan for cashing the
invariants in.

## Stage 1 — attributes (shipped)

- **`noalias` returns on the allocator family** (`lotus_arena_create*`,
  `lotus_arena_create_subregion`, `lotus_arena_alloc`,
  `lotus_bus_payload_arena_alloc`, `lotus_child_struct_alloc`): every
  return is fresh memory in the malloc sense — bump allocation never
  re-hands live bytes; the chunk pool and child-struct freelist
  recycle only blocks whose previous owners are dead. Lets LLVM treat
  each allocation as a distinct object (store-forwarding / DSE across
  struct-literal init and payload copies).
- **`memory(read) nounwind willreturn` on audited pure accessors**
  (`lotus_str_len`, `lotus_bytes_len`, `lotus_bytes_data`) — LICM can
  hoist length reads out of loops, GVN can CSE repeats. Audit notes:
  strlen / one 8-byte load / pointer arithmetic only.
- **`noalias` on the implicit `__caller_arena` param** of free fns:
  the arena struct and metadata are reachable only through that
  pointer within a call; user-visible values point into chunk DATA
  bytes whose accessed ranges never overlap the metadata.
- **`nounwind` on every defined function** (whole-module sweep before
  the pass pipeline) — Hale has no unwinding; failure is
  fallible-sret or violate→process-exit.

Measured (min-of-7, interleaved A/B, 9800X3D): bus_dispatch −9.9%,
tree_fanout −4.6%, form_hashmap_set −3.9%, json_parse −1.7%,
pipeline_3stage −1.1%; call-only microbenches (fn_call, fn_modular)
jitter ±2% from code layout. No test regressions.

## Stage 2 — `noalias self` gated on a reentrancy analysis (open)

The prize is Rust's `&mut`-style `noalias` on `self` in locus
methods — it's what lets field loads stay in registers across calls.
It is NOT sound to emit blanket: two reentrancy channels can access
the same locus inside a method's dynamic extent:

1. **Synchronous bus delivery** — static-dispatch devirtualization
   (v0.9.0) lowers local quiet subjects to direct calls, and a full
   cooperative queue drains inline at the publish site. Either can
   run a handler on the SAME locus while one of its methods is on
   the stack.
2. **Pointer-carrying params** — a method taking a `LocusRef` /
   interface / TypeRef / view arg can receive a reference into its
   own locus (`self.foo(self.bar)` passes a field by pointer).

So the gate for `noalias self` on method M of locus L is:
- every param of M is a by-value scalar (Int/Float/Bool/Decimal/
  Duration — NOT Time/String/views), AND
- M's transitive call graph contains no bus publish, no queue-drain
  entry point, and no call that can reach a method of L again.

The existing `compute_elidable_methods` fixpoint already proves a
subset of this (non-allocating ⇒ no publish), but elidable+scalar
methods rarely have calls at all, so the marginal win there is
small; the analysis worth building is the middle tier — methods
that call OTHER fns proven non-reentrant-into-L. That wants a
per-locus "may-reenter" call-graph summary, which belongs in the
mid-end alongside the alloc summaries (hale-types/alloc_summary.rs
is the template).

## Stage 3 — alias scopes + TBAA (open)

- **Per-arena alias scopes:** every locus arena is a distinct
  allocation domain by construction. Emitting `!alias.scope` /
  `!noalias` metadata pairing "accesses through self's arena" vs
  "accesses through payload/other-locus pointers" would let LLVM
  interleave/vectorize cross-locus copies (bus fan-out, book
  snapshots). Needs a load/store emission chokepoint — today
  build_load/build_store are called at ~hundreds of sites; a
  wrapper that threads the current "memory domain" is the
  prerequisite refactor (and the same chokepoint TBAA needs).
- **TBAA:** Hale has no unions, no pointer casts, no user pointers —
  its TBAA tree can be more aggressive than clang's C tree (every
  named type a distinct branch; scalars distinct from all
  pointers). Same chokepoint prerequisite.
- **Runtime memory() masks:** `lotus_arena_alloc` and friends could
  carry `memory(argmem: readwrite, inaccessiblemem: readwrite)` if
  the diagnostic paths (residency logging, cap dprintf — they touch
  stderr and globals) are compiled out or gated behind a separate
  entry point. Worth doing when the diagnostics get a build-time
  flag; the conservative default mask costs LLVM the ability to
  reorder around allocation calls.
