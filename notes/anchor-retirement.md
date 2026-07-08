# Anchor retirement — reclaiming replaced heap clones

Status: SHIPPED for @form(hashmap) sync=none (2026-07-03) — set
overwrite, remove, and string keys all retire; flush at USER-method
scratch destroy (never in form-synthesized methods — a caller-held
cell copy must survive its own activation, and that placement was
also the per-set overhead); clones floor at 16 bytes so every blob
can carry a freelist node. Validated: 4M-set churn over 16 keys with
fresh key+value strings per set = 4.8 MB flat RSS (was 207 MB —
~50 B/set, the audited on_mark shape). Full suite green; pond +
a downstream app corpus builds; a downstream service smoke passes. GOTCHA that cost a
segfault: lotus_hashmap_t is mirrored FIELD-FOR-FIELD by an inline
LLVM struct in locus/decl.rs — new C fields go at the TAIL of both.
Remaining: compound self.field-store retire (assign_in_place covers
the direct-String case; struct-store leaves the old field clones),
synced maps (needs an epoch scheme — cross-thread readers), vec
cells, run-loop direct sets (no activation boundary — pending list
just holds; no worse than before). The TP-3 class
from the stage-5 audit: 53 corpus sites where a hashmap `set` or a
compound `self.field = Struct{...}` store anchors a fresh String
clone into the locus arena and the PREVIOUS clone for the same slot
is never freed (arenas don't free per-allocation). a downstream service was
hand-fixed with key-reuse idioms; dashboard/prober/websocket still
leak, and every future app will. Same mechanism as the 2026-05-25
a market-data bigcell OOM.

## Why the obvious fixes are unsound

- **In-place buffer reuse** (write the new bytes into the old
  clone's buffer): a reader in the CURRENT activation may hold the
  old pointer (`let old = m.get(k); m.set(k, …); use(old.name)`) —
  it would see the new bytes. Visible to legal programs.
- **Immediate freelist** (retire the old clone for the next alloc):
  same hazard, deferred — the held pointer's bytes survive until a
  LATER allocation in the same activation reuses the block, then
  corrupt.

## The sound design: retire at the ACTIVATION boundary

No raw pointer legally survives an activation: locals die with the
method scratch; anything persisted goes through `self`-storage,
which re-anchors its OWN copy. That is the exact argument that
makes per-call scratch destruction sound — so it also makes this
sound:

1. **retire**: when an anchor site REPLACES a slot's old heap
   pointer (hashmap-set anchor, compound-store field anchor), the
   old pointer goes onto the arena's PENDING list — bytes untouched.
   Gates: `lotus_arena_contains_ptr(arena, old)` (never retire
   another arena's block or a .rodata literal), and old != new
   (the same-arena RMW skip already returns the same pointer).
2. **flush**: at the activation boundary — method-scratch destroy /
   handler exit — pending blocks move to the arena's size-classed
   REUSE freelist (same intrusive-node discipline as the
   child-struct recycler: node header in the dead block's bytes).
3. **reuse**: `lotus_arena_alloc` consults the freelist first
   (size-matched pop, bounded probe), bump-allocates on miss.

Steady state for a bounded-key hashmap under continuous set:
every replaced clone is reused one activation later — O(live)
memory, not O(sets).

## Block sizing

String clones are `[i64 len][bytes][NUL]`; the retire site derives
the block size from the len prefix the same way the clone
allocation did. Only String/Bytes retire in v1 (TypeRef compound
fields recurse to their own String leaves via the anchor walk).

## Rollout

- v1 wires the hashmap-set anchor (the audit's hottest class:
  marks/wireskew/last_message shapes) + the
  anchor_struct_fields_in_place replace site.
- Validation: an RSS bench (steady-state set loop over a bounded
  key domain with fresh-parsed values — flat vs linear), plus the
  full suite and the alloc_model_rss empirical tests.
- The unbounded-alloc analysis keeps flagging these sites until the
  verdict model learns "anchor sites retire" — flip that only after
  the RSS bench proves the runtime behavior (no false bounded).
