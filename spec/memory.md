# Memory model

This document specifies lotus's memory model: how regions are
allocated, organized, and freed; how locus structure constrains
memory layout; how access between loci is mediated.

## Foundational rule

**A locus owns a region. The region's lifetime is the locus's
lifetime. When the locus dissolves, the region is freed
wholesale.**

There is no garbage collector. There is no borrow checker. The
lotus structure provides the memory hierarchy for free; the
allocator just respects it.

## Hierarchy

Regions form a tree. The runtime root locus has a top-level
region. Each child locus's region is a *sub-region* of its
parent's region.

```
  root region
  ├── main's implicit-locus region
  │   ├── coordinator region
  │   │   ├── leaf-1 region
  │   │   ├── leaf-2 region
  │   │   └── leaf-3 region
  │   └── attribution-service region
  │       └── ...
  └── runtime services regions
```

A region's allocations are bounded above by its own size; sub-
regions allocated within count against the parent's budget.

Regions cannot escape their hierarchy: a value allocated in
leaf-1's region cannot be referenced by leaf-2 (lateral access
is blocked at the type system level; physical layout makes it
moot anyway since wholesale dissolution would invalidate cross-
region references).

## Per-projection-class allocation

A locus's projection class (rich / chunked / recognition)
determines its allocator strategy. Same syntactic source, three
generated implementations:

| Projection class | Allocator | Coordinatee model |
|---|---|---|
| **Rich** (proj_rich, N≈4–10) | Per-locus bump arena. | Coordinatees attached as small array; full state per coordinatee; freed wholesale on locus dissolution. Low churn expected. |
| **Chunked** (proj_chunked, N≈10–30) | Per-locus arena with **per-coordinatee sub-regions**. Each accept allocates a sub-region; each dissolution frees one wholesale. Bookkeeping slots reclaimed via free-list. | Typed-message-header per coordinatee; moderate churn supported. |
| **Recognition** (proj_recognition, N≈100–500) | Pre-allocated fixed pool. No dynamic allocation in steady state. | Summary-only per coordinatee; many supported; minimal per-coordinatee state. |

The compiler picks based on the locus's declared projection
class. A locus with no explicit `projection` annotation defaults
to `chunked` if it accepts coordinatees, `rich` if it doesn't.

## Lifetime rules

### Bound handles

```
let h = LocusL { ... };
// h is bound. Locus lives until `h` goes out of scope.
// Then: drain() runs (cascades), dissolve() runs, region freed.
```

### Unbound expressions

Per design-rationale §A:

- **Ephemeral** (only `birth` + `params`, no ongoing-work surface):
  dissolves at the enclosing statement boundary.
- **Long-lived** (has `run`, bus subscriptions, mode declarations
  callable from outside, or any post-birth work surface):
  becomes anonymous child of enclosing scope; lives until scope
  dissolves.

### Lifecycle methods

Lifecycle methods (`birth`, `accept`, `run`, `drain`, `dissolve`,
`on_failure`) **do not have their own implicit locus** (per F.6).
They run *as the locus*. Locals in their bodies are bound to the
lifecycle method invocation; child loci instantiated in their
bodies attach to the enclosing locus, not to the lifecycle
method's scope.

### Free `fn` functions

`fn main()`, `fn helper()`, etc. — every free function has its
own implicit locus (per §D). Bound handles and anonymous
children are children of that implicit locus; the function
returns when:

- Body's last statement completes, AND
- All children of the implicit locus have dissolved.

## Bookkeeping reclamation (per-arena defrag)

Per F.3: within a parent's arena, dissolved-coordinatee
bookkeeping slots are reclaimed via a **per-arena free-list**
(chunked-class loci) or **periodic defrag** (high-churn).
Reclamation is:

- **Per-arena** — never crosses arena boundaries.
- **Bounded** — reclamation work is O(slots-being-reclaimed),
  not O(heap).
- **Deterministic** — no stop-the-world, no opaque tracing.

Coordinatee sub-regions remain pristine arenas freed wholesale
on dissolution; only the parent's *bookkeeping* about
coordinatees (registry slot, dispatch entry) needs free-list
reclamation.

## Drain cascade

Per F.4: `drain()` always cascades depth-first.

1. Recursively call `drain()` on each child first.
2. Wait for all children to finish draining.
3. Drain self.

There is no separate `drain_cascade()` syntax — `drain()` is
*always* cascading. SIGINT triggers `drain()` on the runtime
root, cascading through the whole process tree.

This implies that during drain:
- New child accepts are refused.
- In-flight messages on bus subscriptions are delivered;
  no new messages accepted.
- Any in-flight handler invocations complete.
- After in-flight work completes, dissolve runs; region freed.

## Mode-projections share the arena

Per F.5: a locus's three modes (bulk / harmonic / resolution)
operate on the same locus state via the same arena. Generated
code reads/writes one region across three implementations. No
duplicate allocation, no copy.

The compiler verifies that the modes don't write-conflict
(resolution-mode mutating state that bulk-mode also writes
during overlapping evaluation is a compile-time error if the
writes would race).

## Inter-locus access

### Vertical-only at the memory level

A locus L can read into a coordinatee C's region via the
contract surface (per F.7, F.8, F.11, F.14). C cannot read
into L's region except via the contract's `consume` declarations.
Siblings cannot read each other (vertical-only flow expressed
at the memory layer).

Practically:

- L → C: typed contract field access (`c.greeting`); routed
  through C's translation function (per F.14); cost reflects
  C's projection class.
- C → L: only what L declares in `consume`; goes back through
  the contract; never direct address.
- C1 ↔ C2 (siblings): not permitted. Lateral coordination
  flows through the parent (typed message via bus, or
  contract-mediated).

### Cross-locus copies, not pointers

A typed message crossing a locus boundary is a **copy**, not a
pointer. The bus message arrives in the receiver's arena; the
sender's state is independent. This is required because:

- Sender and receiver may be in different schedulers (per the
  cooperative scheduler model in `runtime.md`).
- Sender may dissolve before receiver finishes processing the
  message; pointer-based access would dangle.

The framework's vertical-only-flow expresses itself at the
memory level as: pointers don't cross loci; values do.

## Region escape rules (forbidden patterns)

The compiler rejects these at compile time:

1. **Returning a sub-region pointer from a longer-lived scope.**
   `let r = make_region_in_child(); use(r);` where `r` outlives
   the child is a region-escape error.
2. **Storing a child reference in a parent or sibling that
   outlives the child.** Triggers a region-lifetime check.
3. **Sibling-to-sibling reference.** No type rule permits this;
   the compiler emits a clear "vertical-only flow" error.

## Edge cases

### Failure during birth

If a locus's `birth()` panics or otherwise fails, the region is
freed wholesale; no `dissolve` runs (since dissolve assumes
birth completed). The parent's `on_failure` receives the
failure event.

### Failure during accept

Per F.7, `accept()` runs before child region allocation. If
accept rejects (panics, returns error), the child region is
never allocated — no cleanup needed.

### Failure during run

Mid-`run()` panic triggers the parent's `on_failure(self,
StructuralFailure { ... })`. Parent decides recovery
(`restart`, `quarantine`, `bubble`, `dissolve`); region is
freed per the recovery primitive's rules.

### Closure violation at dissolve

Per F.9, a closure violation at the `dissolve` epoch is an
**audit failure** (explosion), not a structural failure. The
locus's region is freed regardless; the parent receives
`on_failure(self, ClosureViolation { ... })` with typed event
data.

### Quarantine

A quarantined coordinatee retains its region (the parent has
chosen to preserve it for inspection). Region is freed only
when explicitly `dissolve(child)`d or `restart`ed. Quarantine
is the one case where a "dissolved-from-the-system" coordinatee
keeps its region alive.

## Allocator implementation outline

(Informative; specifies expected behavior, not the literal
implementation.)

### Rich

```
struct RichArena {
    bump_ptr: *mut u8,
    end_ptr: *mut u8,
    coordinatees: [Option<Coordinatee>; MAX_RICH_N],
}
```

Single bump arena. Allocations are pointer-bumps. Dissolution
resets bump_ptr to start.

### Chunked

```
struct ChunkedArena {
    parent_bump: BumpAllocator,
    coordinatee_subregions: Vec<SubRegion>,
    bookkeeping_freelist: FreeList<usize>,
}

struct SubRegion {
    bump_ptr: *mut u8,
    end_ptr: *mut u8,
    coordinatee_id: u32,
}
```

Each accepted coordinatee gets a sub-region (pristine bump
arena). Bookkeeping (`coordinatee_id`, dispatch slot) lives in
the parent's bump area; reclaimed via free-list when
coordinatee dissolves.

### Recognition

```
struct RecognitionPool {
    cells: [RecognitionCell; POOL_SIZE],
    occupied_bitmap: [u64; POOL_SIZE / 64],
}

struct RecognitionCell {
    summary: RecognitionSummary,
    occupied: bool,
}
```

Pre-allocated pool of fixed-size cells. Allocation is a bitmap
search; deallocation is a bitmap clear. No dynamic memory in
steady state.

## What the compiler emits

For each locus, the compiler generates:

1. A region-allocation function (per projection class) called
   at birth.
2. A drain handler that walks children depth-first.
3. A dissolution handler that releases the region wholesale.
4. Per-mode implementations that read/write the locus's region
   in-place.
5. Translation function entries (per F.14) accessible through
   the arena.

The runtime provides the underlying bump allocators, free-list
machinery, scheduler integration, and lifecycle dispatcher.

## Future work

- **Hot-load preservation across perspective updates.** When a
  perspective is hot-loaded, the receiving locus's arena state
  is preserved across the swap; the new perspective's translation
  functions replace the old. v0 specifies the perspective hot-
  load mechanism (runtime.md); the memory-level interaction is
  TBD.
- **Region size tuning.** Initial region sizes per locus are
  set by the compiler from declared params. Runtime growth /
  shrinkage of regions is currently not supported (a locus
  exceeding its region is a runtime panic). Future versions may
  add growable regions for chunked-class loci.
- **Compaction passes.** For long-running chunked-class loci
  with high churn, periodic compaction may be needed. Currently
  free-list reclamation is sufficient for v0; compaction passes
  are deferred.
