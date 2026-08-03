# Anchor retirement — reclaiming replaced heap clones

Status: SHIPPED for @form(hashmap) sync=none (2026-07-03) — set
overwrite, remove, and string keys all retire; flush at USER-method
scratch destroy (never in form-synthesized methods — a caller-held
cell copy must survive its own activation, and that placement was
also the per-set overhead). Validated: 4M-set churn over 16 keys with
fresh key+value strings per set = 4.8 MB flat RSS (was 207 MB —
~50 B/set, the audited on_mark shape).

SMALL-BLOCK FIX (2026-07-17): the reuse freelist stored its node IN
the dead block (size@0, next@8), so blocks < 16 bytes could not carry
it and were DROPPED at flush — short replaced values/keys (a "12.3",
a "sig.4") leaked ~50-128 B per set. A downstream service measured
this as ~128 B/frame linear on a churned recorded-state map (v0.11.1).
Fix: blocks < 16 recycle OUT-OF-BAND via their shell {blob,size,next}
on `retire_free_small` (no write into the block → sound for any size);
`lotus_str_clone` drops its 16-byte floor so the recorded size equals
the true block size and small/large reuse both match. Validated: a
1M-set churn of sub-16-byte values over 100 keys stays at the RSS
floor (was ~40 MB), ASan clean, 5×30k acceptance bench flat. Full
suite green; the earlier ≥16 in-band path is unchanged. GOTCHA that cost a
segfault: lotus_hashmap_t is mirrored FIELD-FOR-FIELD by an inline
LLVM struct in locus/decl.rs — new C fields go at the TAIL of both.
SELF-FIELD STORES (Gap A, 2026-07-17): compound
`self.f = Struct{...}` replaces now retire the old struct's String
clones via a per-field post-memcpy fixup
(`lotus_str_field_replace_fixup`), and `lotus_str_assign_in_place`
retires the abandoned buffer on its grow path. Validated: 1M
whole-struct replaces (2 fresh clones each) = RSS flat
(alloc_model_rss.rs::self_field_struct_replace_churn), 200k mixed
alias/RMW/grow churn ASan+UBSan clean.

FOUND EN ROUTE — same-arena skip broke value semantics: the clone
skip let `self.g = self.f` (non-fitting path) and struct literals
embedding a `self.<field>` read SHARE the source slot's blob; the
source's next in-place overwrite mutated the aliased slot (probe:
g printed f's new bytes). That aliasing also made retire unsound
(freeing a blob the other slot still holds). Fix = SINGLE-OWNER
rule: at self-storage store sites, a same-arena incoming pointer
that isn't the slot's own old pointer force-copies
(`lotus_str_copy_owned` / `lotus_bytes_copy_owned`, no skips).
Fresh clones, statics, and RMW round-trips keep the zero-copy
paths. Regression tests: tests/self_field_alias.rs. NOTE: reads of
self heap fields return RAW arena pointers (no clone-on-read) —
any future anchor site must preserve single-owner or retire breaks.

CELL SINGLE-OWNER + BYTES-GROW RETIRE (2026-07-18): two former
residuals shipped together.

 1. Bytes grow: `lotus_bytes_assign_in_place`'s doesn't-fit branch
    now retires the abandoned blob (lotus_arena_retire_bytes, size
    from the len prefix — under-reports after a fit-path shrink,
    same caveat as String), and Bytes allocation
    (lotus_bytes_clone / lotus_bytes_copy_owned) consults the
    freelist via ALIGNMENT-AWARE pops: align-8 Bytes blocks and
    align-1 String blocks share one list; a candidate only matches
    if its address satisfies the request's alignment (String
    requests take either kind, Bytes requests skip odd-addressed
    blocks). NOTE the shrink-collapse means a single oscillating
    field cannot self-serve its own grows (recorded size < the
    physical block it needs next) — the reclaim pays off through
    OTHER same-arena allocations of matching sizes, not the
    growing field itself.
 2. Cell single-owner: the @form cell-store anchor walk
    (hashmap set / lru put) now (a) walks a stack SNAPSHOT of the
    value struct — the in-place store-back used to re-point a
    self-storage source (`m.set(self.rec)`) at the cell's own
    blob, re-aliasing them — and (b) clones String/Bytes leaves
    through `lotus_*_clone_cell_owned`, which force-copy a
    same-arena input instead of skip-sharing it (statics still
    pass through; cross-arena values clone as before). Empirical
    note that cost a debugging detour: literal builds and
    hashmap-get both round-trip through method scratch, so
    `m.set(Rec { val: e2.val })` shapes were ALREADY de-aliased —
    the reachable pre-fix repro was the whole-struct self-field
    store `m.set(self.rec)`, proven by same-length in-place
    mutation visibility (tests/hashmap_cell_alias.rs; fixture
    70-cell-single-owner runs the mixed churn under the ASan
    corpus oracle).

Remaining: nested compound / Bytes fields of a replaced struct
(String leaves only in v1; a nested compound that same-arena
skip-shares through the contains_ptr gate still aliases — no
retire for compounds, so no UAF, but shared mutation);
struct-field in-place shrink collapses the recorded capacity
(strlen / len-prefix at retire under-reports after a fit-path
shrink → reuse degrades on oscillating lengths, still sound — now
applies to Bytes too); user methods on a @form locus never flush
(the form-locus early-return); vec cells (no retire; String elements
pushed from self-storage still skip-share — benign until vec
retire exists), run-loop direct sets (no activation boundary —
pending list just holds; no worse than before). The TP-3 class
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

SYNCED MAPS via CLONE-ON-READ (2026-08-03, downstream handoff P1).
Previously listed here as "needs an epoch scheme — cross-thread
readers". It does not. The reason a `sync = serialized` map never
installed a retire descriptor is that `get` memcpy's the cell, so
its String fields come out as raw pointers into the MAP's arena and
an off-pool reader can hold one across the writer's activation
boundaries — i.e. THE LEAK WAS THE SAFETY MECHANISM. Making the
reader own its copy removes the off-thread reader entirely, and the
ordinary flush becomes sound with no epoch machinery:

 1. Every read path on a synced String-bearing map clones the
    cell's Strings into the CALLER's arena, INSIDE the critical
    section that read the cell (between an unlock and the clone a
    writer could overwrite, retire and flush the blob):
    `lotus_hashmap_{get,value_at,iter_batch}_cloned`. Covering all
    three is load-bearing — enabling retirement while leaving any
    read path handing out raw cell pointers converts the leak into
    a use-after-free, which is strictly worse.
 2. The descriptor is installed for `SyncMode::Serialized`, and the
    activation-boundary flush gate widened to match. `striped` and
    `lockfree` stay excluded: their reads bracket on lf_enter /
    rwlock rather than the map mutex and their grow path rebuilds
    slots concurrently — a separate audit.
 3. A serialized map's arena is marked `shared_concurrent`, which
    serializes the bump allocator (existing) and the retire lists
    (new `retire_lock`). Needed because pushes come from the set
    path under the MAP mutex while flushes come from each writer's
    own activation boundary with no map lock held. `retire_lock` is
    DISTINCT from `subregion_lock` because the push path allocates
    a shell through `lotus_arena_alloc`, which takes subregion_lock
    — one lock would self-deadlock. Order: retire_lock, then
    subregion_lock, never the reverse.

MEASUREMENT CAVEAT — READ BEFORE CLAIMING P1 CLOSED. The safety
properties are covered (tests/form_hashmap_synced_retire.rs, three
read paths, cross-pool churn, plus the ASan corpus oracle). The
SPACE win is NOT demonstrated: two reproducers built to the reported
shape (bounded key domain, fresh key+value Strings per set, 64-byte
values, 50k vs 400k sets, sets from a per-call method so the
activation boundary actually flushes) show byte-identical RSS before
and after — and, more to the point, show no growth with set count on
the SHIPPED v0.13.0 either, so they do not reproduce the reported
leak at all. Either the leak needs a shape neither reproducer
captures, or something since the report already addressed it. Get
the downstream team's own reproducer before recording P1 as fixed.
Note the leak DOES reproduce for sets issued directly from a long
`run()` body (66 MB -> 120 MB over 50k -> 400k), but that is the
separate known residual above — a `run()` loop is one activation, so
no flush boundary is ever crossed, and the growth there is method
scratch rather than the map.

