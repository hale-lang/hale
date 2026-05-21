# Memory model

This document specifies Aperio's memory model: how regions are
allocated, organized, and freed; how locus structure constrains
memory layout; how access between loci is mediated.

> **Naming note:** The language is **Aperio**; the runtime
> substrate is called **lotus** ("lotus structure provides the
> memory hierarchy" below refers to the substrate concept, not
> the language). C-runtime symbols stay `lotus_*`.

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

A locus's projection class (rich / chunked / recognition) is
a **perspective-resolution commitment** — a declaration of
what observation granularity the locus serves to perspectives
one tower up. Rich serves named-child observation (fine
resolution); chunked serves chunk-level observation (mid
resolution); recognition serves population-level observation
(aggregate resolution — "represent as a curve," "as a
histogram," "as a count"). The allocator strategy below is a
*consequence* of the resolution commitment, not the
commitment itself — same syntactic source, three generated
implementations chosen to make observation at the declared
resolution cheap:

| Projection class | Allocator | Coordinatee model |
|---|---|---|
| **Rich** (proj_rich, N≈4–10) | Per-locus bump arena. | Coordinatees attached as small array; full state per coordinatee; freed wholesale on locus dissolution. Low churn expected. |
| **Chunked** (proj_chunked, N≈10–30) | Per-locus arena with **per-coordinatee sub-regions**. Each accept allocates a sub-region; each dissolution frees one wholesale. Bookkeeping slots reclaimed via free-list. | Typed-message-header per coordinatee; moderate churn supported. |
| **Recognition** (proj_recognition, N≈100–500) | Sub-mode-typed recpool selected at the locus declaration site (`fixed_cell`, `shared_slab`, `spillover`, or `summary_only`). v1 ships `fixed_cell` (bitmap-tracked fixed cells; child's arena lives inline in the cell) and `shared_slab` (one bump arena shared across all children; per-child release is a no-op). See **Recognition sub-modes** below. | Summary-only per coordinatee; many supported; minimal per-coordinatee state. |

The compiler picks based on the locus's declared projection
class. A locus with no explicit `projection` annotation defaults
to `chunked` if it accepts coordinatees, `rich` if it doesn't.
Recognition is **explicit-only** — there is no implicit
recognition fallback, and v1.x-3 made the sub-mode commitment
required at the declaration site (bare `: projection recognition`
is a parse error). Same forcing-function discipline as the
2026-05-12 two-channel rule: the substrate doesn't pick a default
for you.

### Recognition sub-modes (v1.x-3)

A locus annotated `: projection recognition(cap=N, <sub_mode>)`
commits to a storage discipline for its accepted children at
the declaration site. The cell stride (`K` in the table below)
is *not* a user knob — it is derived at codegen time from the
union of accept-method param types on the parent locus, taken
as the max. The contract the author writes is "how many
children, what discipline"; the layout is the compiler's job.
v1 ships two sub-modes; the other two parse + typecheck but
reject at the resolver with a "v1.x pending" diagnostic.

| Sub-mode | Commitment | Backing | Per-child release |
|---|---|---|---|
| `fixed_cell` | "Each child fits in a cell sized for the accept-type union. Cap of N children. Overflow is a hard runtime error." | `lotus_recpool_fixed_*`. One contiguous block of `N × stride` bytes; each cell holds an inline `lotus_arena_t` + chunk header + payload. Bitmap-tracked acquire/release. The cell IS the child's arena. | Clears the bitmap bit. Slot is reusable. |
| `shared_slab` | "All children share a single bump arena sized for the accept-type union × cap. The whole slab frees at parent dissolve." | `lotus_recpool_slab_*`. One `lotus_arena_t` with a single fixed-size chunk; `fixed_size=1` so it never grows. Every acquire returns the SAME arena pointer — sibling allocations interleave. | No-op. Slab freed wholesale at parent dissolve. |
| `spillover` *(v1.x pending)* | "Each child fits in a cell; overflow malloc-fallback with one-time warning. Graceful degradation under load." | Future: `lotus_recpool_spillover_*`. Per-cell `fixed_cell` plus a heap-allocated fallback. | TBD. |
| `summary_only` *(v1.x pending)* | "Children carry zero per-instance state; all allocations live in the parent's arena." | Future: type-system rule prohibiting child arena allocation; parent's `__arena` is the only storage. | No-op. |

The arena handle returned by `lotus_recpool_fixed_acquire` /
`lotus_recpool_slab_acquire` is a `lotus_arena_t*` so child
body code stays projection-class-agnostic per the F.22
architectural invariant. Overflow on the child's
`arena_alloc` returns NULL (the arena's `fixed_size=1` flag is
honored); v1.x-3 wires that to a hard NULL return — routing
through `lotus_root_panic` for value-error escalation is a
future polish.

The codegen dispatch at child dissolve picks the matching
`release` fn via a synthetic `__recpool_release_kind: i64`
discriminator on every locus struct (0 = regular
`lotus_arena_destroy`, 1 = `lotus_recpool_fixed_release`, 2 =
`lotus_recpool_slab_release`). Set at the parent's accept
step; consumed at child dissolve. Uniform layout so the
dissolve path doesn't branch on whether the locus opted into
the recognition surface.

## Capacity slots (F.22)

A locus's storage discipline is an **N-tuple of capacity slots**.
Slot 0 is the locus's own Arena (everything above this section).
Slots 1..N are user-declared in a `capacity { ... }` block:

```
locus Foo {
    capacity {
        pool entries of Int;        // slot 1: cell-recycling of Int-sized
        heap registry of Command;   // slot 2: growable, individual free
    }
}
```

Each non-Arena slot kind is a commitment the locus makes about
its own state, not a hidden implementation detail:

| Slot kind | Commitment | Backing | Lifetime |
|---|---|---|---|
| **Arena** (slot 0, implicit) | "I'm scratch — everything I touch dies with me." | Single bump arena per locus. | Wholesale-free at locus dissolve. |
| **Pool of T** (slots 1..N) | "I hold a bounded shape of recyclable state." | Chunked free-list (`lotus_pool_*`). Cells acquired / released; chunks grow geometrically (16 → 32 → 64, capped at 4096) when the free-list empties. | Wholesale-free at locus dissolve. |
| **Heap of T** (slots 1..N) | "I hold growable state bounded by my own lifetime." | Doubly-linked live list with intrusive header (`lotus_heap_*`). Cells alloc / free individually. | Wholesale-free at locus dissolve (live list walked, every still-live cell freed). |

Slot init runs in declaration order after slot 0; slot destroy
runs in reverse declaration order before slot 0. Cell alignment
is 8 bytes (the v0 substrate's universal scalar alignment); cell
size comes from T's LLVM struct layout. Restriction: a cell
type cannot be a `LocusRef` — locus membership goes through
`accept(c: Child)`, not slots. See `spec/semantics.md`
"Capacity slot lifecycle and dispatch (F.22)" for the user-
facing method-shaped surface and full restriction list.

**Slot 0 parent-override** is governed by projection class
(table above): Chunked / Recognition parents sub-region-allocate
their accepted children's slot 0; Rich parents do not. F.22
names this existing v0 behavior so future **slot 1..N parent-
override** (`pool entries of Int as_parent_for Child;`) sits
on consistent vocabulary. Slot 1..N parent-override is deferred
to v1.x — the first workload that demands a parent-owned Pool
shared across accepted children will unlock the syntax.

**Naming note.** The Recognition projection class's recpool
(see § "Recognition sub-modes (v1.x-3)" above —
`lotus_recpool_fixed_*` / `lotus_recpool_slab_*` per the chosen
sub-mode) is the *slot 0 storage strategy for recognition-
classed loci*. F.22's `pool` slot is a user-declared slot at
1..N with chunked-+-free-list backing and no projection-class
entanglement. The two systems may unify in v1.x once F.22
slots 1..N stabilize; until then they are structurally
distinct mechanisms that happen to share the word "pool."

## Lifetime rules

### Bound handles

```
let h = Locus { ... };
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

Sub-mode-typed at the locus declaration site
(`recognition(cap=N, <sub_mode>)`); v1 ships two sub-modes:

```c
/* fixed_cell — bitmap-tracked cells; the cell IS the child's
 * arena. Per-child release clears the bit. The compiler
 * computes cell_bytes from the parent's accept-method type
 * union; user code only spells cap. */
struct lotus_recpool_fixed {
    size_t cap_count, cell_bytes, cell_stride, bitmap_words;
    uint64_t *bitmap;
    char *cells;   /* cap_count * cell_stride bytes;
                    * each cell holds an inline arena
                    * header + chunk header + payload */
};

/* shared_slab — one shared arena; every acquire returns the
 * same pointer; per-child release is no-op. Slab size also
 * derived from the accept-type union × cap. */
struct lotus_recpool_slab {
    size_t cap_count, slab_bytes;
    lotus_arena_t *slab_arena;   /* fixed_size=1 */
};
```

Allocation is a bitmap search (fixed_cell) or a regular arena
bump (shared_slab); no dynamic memory in steady state for
either. See `crates/aperio-codegen/runtime/lotus_arena.c` for
the canonical implementation and § "Recognition sub-modes
(v1.x-3)" above for the per-sub-mode contract.

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

**Arena alignment contract (2026-05-20).** `lotus_arena_alloc(a,
size, align)` returns a pointer whose address (not the within-
chunk offset) is aligned to `align`. The chunk header
(`{next, used, cap}` = 24 bytes on x86_64 LP64) sits before the
data region, so a naive "align the offset" approach yields
8-byte-aligned pointers even when callers ask for 16. The
correct shape: align the cursor address `(c+1) + c->used` to
`align`, then convert back to a within-chunk offset. The codegen
side passes `align = 16` from `arena_alloc` to cover the widest
scalar type (i128 / Decimal) — earlier the codegen passed 8 and
the C side only aligned the offset, leading to `movaps` segfaults
on Decimal stores into struct fields (fathom F7-segfault / F4
root cause, 2026-05-20). Both layers are necessary: the codegen
must ask for the natural alignment of the widest scalar it can
emit, and the C arena must honor that alignment at the pointer
level, not the offset level.

## Codegen ABI (v0)

The native-codegen path (`aperio build`) lowers each locus to an
LLVM struct one field per declared param, and each lifecycle
method to an LLVM function whose first parameter is a pointer to
that struct. Field reads / writes via `self.X` lower to
`getelementptr` + `load` / `store` against the `self_ptr`. This
is the substrate the region allocator + scheduler will sit on top
of when they land — the ABI is the load-bearing contract; the
allocator and dispatcher refine *where* the struct is allocated
and *how* methods get scheduled, not the struct's shape.

```
locus Greeter {
    params { greeting: String = "hi"; }
    contract { expose greeting: String; }
}
locus Coord {
    params { factor: Int = 1; }
    contract { consume greeting: String; }
    accept(g: Greeter) { ... }
    run()              { ... }
}
```

lowers to:

```
%locus.Greeter = type { ptr }              ; greeting
%locus.Coord   = type { i64 }              ; factor

declare void @Coord.accept(ptr %self, ptr %child)
declare void @Coord.run(ptr %self)
```

Statement-level instantiation `T { ... };` lowers to: `alloca`
on the caller's stack, store each field (call-site override or
declared default), then dispatch lifecycle methods in the F.7
order:

1. **If we're inside a parent locus's lifecycle method AND that
   parent has an `accept(child: T)` method matching the locus
   being instantiated** → call `parent.accept(parent_self, child_ptr)`.
2. Call `T.birth(child_ptr)` if declared.
3. Call `T.run(child_ptr)` if declared.
4. Call `T.drain(child_ptr)` if declared.
5. Call `T.dissolve(child_ptr)` if declared.

`accept` runs *before* the child's own `birth`, per F.7. This
is how `02-parent-child`'s `Coord.accept(g: Greeter)` fires for
each `Greeter { ... }` instantiated in the coordinator's `run()`
body. Inside `accept`, `self.X` GEPs through the parent's struct
and `g.X` GEPs through the child's struct — different `getelementptr`
chains, same lowering machinery.

`drain` / `dissolve` run last, in that order, before the alloca
dies. The F.4 depth-first cascade is implicit in v0's
synchronous-instantiation model: any descendants instantiated
inside this locus's `run()` body have already gone through their
own full birth → run → drain → dissolve sequence (each via this
same lowering, recursively) before `run()` returns. So when this
locus's `drain()` fires, all descendants are already gone — no
explicit cascade walk is needed at the substrate level. When the
cooperative scheduler lands and loci can be long-lived, the
cascade becomes explicit; the lifecycle-method ABI doesn't
change.

v0 codegen is **ephemeral-only**: every alloca is on the caller's
stack and freed when the enclosing fn returns. Long-lived loci
and the parent-child region hierarchy described above (each
child's region nested in the parent's) wait on the cooperative
scheduler + region allocator work.

Constraints v0 codegen enforces (will relax as more lands):

- Lifecycle methods supported: `birth`, `accept`, `run`,
  `drain`, `dissolve`.
- `birth`, `run`, `drain`, `dissolve` take no user-declared
  params (only implicit `self`); `accept` takes exactly one
  param, the typed child reference. All lifecycle methods
  return `void`.
- Locus param defaults must be literals (Int / Float / Bool /
  String / Duration). Non-literal defaults compile under the
  interpreter but not via `aperio build`.
- Contracts are typecheck-only at this layer — they're accepted
  in the AST and skipped by codegen. The expose / consume
  surfaces are still type-checked across coordinator / coordinatee
  per F.8 by the typechecker pass.
- Locus members beyond `params`, `contract`, the five-method
  lifecycle set, `bus { subscribe / publish }` declarations, and
  `fn` members (used as bus handlers) are rejected at declare
  time. Modes, closures, failure handlers, and nested
  consts/types wait on later milestones.

The struct ABI + accept + drain/dissolve dispatch is what makes
`01-locus-with-run`, `02-parent-child`, `10-stateful-locus`, and
`11-drain-dissolve` compile to native ELF identically to their
interpreter behavior.

### Bus router (m12)

The bus router lowers as **one global subscription table per
program**, sized at compile time from the total `bus subscribe`
declaration count. Layout:

```
%lotus.bus_entry = type { ptr, ptr, ptr }   ; subject, self, handler
@bus.entries = internal global [N x %lotus.bus_entry] zeroinitializer
@bus.count   = internal global i64 0
```

Each `bus subscribe "S" as h ...` declaration on a locus
contributes one slot; registration happens when the locus is
instantiated, BEFORE its `birth()` runs:

```
@bus.entries[bus.count] = { @.str.S, %self_ptr, @<Locus>.h }
bus.count += 1
```

`<-` lowers to a call into the generated dispatch fn:

```
define void @lotus.bus_dispatch(ptr %subject, ptr %payload) {
   ; for i in 0..bus.count:
   ;   if strcmp(bus.entries[i].subject, %subject) == 0:
   ;     bus.entries[i].handler(bus.entries[i].self, %payload)
}
```

Subject equality uses libc `strcmp` (subjects are NUL-terminated
global strings). Handler functions are called through a
type-erased function pointer — every bus handler has the same
LLVM signature `void (ptr self, ptr payload)`, with payload
typing enforced by the typechecker upstream.

#### Long-lived locus deferral

A locus with any `bus subscribe` declaration is **long-lived**:
its drain/dissolve must NOT fire at the end of its instantiation
expression (which would dangle its `self_ptr` in the bus table
before later publishes can reach it). Instead, each fn body /
lifecycle method body opens a deferred-dissolve frame; long-lived
loci instantiated inside push their `(self_ptr, locus_name)` onto
the frame; at body exit (just before `ret`) the frame is flushed
in reverse instantiation order, calling drain → dissolve on each.

Ephemeral loci (no subscribe) at *statement-position* keep the
original semantics: drain → dissolve fires at end of
`lower_locus_instantiation`, inside the same lifecycle body
that instantiated them. m82 changed the *let-bound* case:
`let h = LocusName { ... }` now defers dissolve to the
enclosing fn's scope-exit flush instead of the struct-literal
boundary, so the user-visible binding stays valid for
subsequent method calls. Long-lived loci (with `bus subscribe`)
continue to defer regardless of binding shape. See
`spec/semantics.md` "Dissolve timing rules" for the full rule.

The F.4 cascade still falls out structurally — children
dissolve before their parent, regardless of which mechanism
handles each.

### Region allocator substrate (m19)

The codegen path links a small C arena runtime
(`crates/aperio-codegen/runtime/lotus_arena.c`, bundled into
the compiler via `include_str!`) into every emitted binary.
Public ABI:

```
ptr  lotus_arena_create(void)                              // new arena
ptr  lotus_arena_alloc(ptr arena, i64 size, i64 align)     // bump
void lotus_arena_destroy(ptr arena)                        // wholesale free
```

An arena is a linked list of bump chunks (default 64 KiB each;
oversized requests get a fresh chunk sized to fit). Allocation is
a pointer-bump in the head chunk; on overflow, a new chunk is
malloc'd and pushed to the front. Destruction walks the list and
frees every chunk wholesale — no per-object free, ever.

### Locus-owned arenas + bus copy semantics (m20)

Every locus struct carries a synthetic `__arena: ptr` field at
**struct slot 0**. Initialized at instantiation time (right after
the `alloca`) via `lotus_arena_create()` and torn down via
`lotus_arena_destroy(self.__arena)` after the user's `dissolve`
method runs (in both the ephemeral path and the deferred
long-lived-locus flush at body exit). Per spec: "A locus owns a
region. The region's lifetime is the locus's lifetime."

The arena field's fixed-offset placement is load-bearing for the
bus path: `lotus.bus_dispatch` is type-erased — it sees only
`ptr self` from the subscription table — so its only way to find
the subscriber's arena at runtime is a fixed-offset load. Slot 0
makes that a constant GEP.

Allocation routing inside codegen has three tiers, in order:

1. **`current_arena_override`** — set during locus-instantiation
   field init so composite-literal defaults / overrides land in
   the new locus's arena (rather than the parent's arena where
   the default expression literally executes).
2. **`current_self`'s arena field** — when we're inside a
   lifecycle method body (or any fn with a `current_self`
   binding), allocations go to the locus's own arena.
3. **`@lotus.arena.global`** — fallback for `main` and free fns,
   which have no enclosing locus. Initialized in main's prelude;
   destroyed at every `ret` from main.

Bus dispatch implements the spec's copy-not-pointer semantic:

```
void lotus.bus_dispatch(ptr subject, ptr payload, i64 size):
   for i in 0..bus.count:
     if strcmp(bus.entries[i].subject, subject) == 0:
       sub_self  = bus.entries[i].self
       sub_arena = load (sub_self + 0)
       copy      = lotus_arena_alloc(sub_arena, size, 8)
       memcpy(copy, payload, size)
       bus.entries[i].handler(sub_self, copy)
```

Each `<-` call site passes the payload's compile-time-known
struct size as a third arg. The subscriber's handler receives a
pointer into the subscriber's own arena, valid until that
subscriber dissolves — independently of when the publisher's
locus dissolves. This unblocks `self.current_kernel = msg`
patterns where the subscriber stores a payload reference across
multiple bus events (fitter-applier-demo's central pattern).

m20 deliberately keeps free fns + main on the program-wide arena
(no per-call arena yet) and doesn't yet specialize per projection
class — chunked-class per-coordinatee sub-regions land in m22,
the recognition-class fixed pool in m23.

**Phase-3 hard byte-cap on `g_bus_payload_arena` (2026-05-19;
safety net).** The arena now refuses to grow past
`LOTUS_BUS_PAYLOAD_ARENA_CAP` (default 64 MiB, env-overridable
for capacity-planning experiments). When the cap fires
`lotus_arena_alloc` returns NULL; one diagnostic line goes to
stderr identifying the cap event and the arena's name; subsequent
allocations against the capped arena keep returning NULL.
Existing callers — BytesBuilder `snapshot()` / `finish()` via the
alloc-fail sentinel + violate routing, recv_bytes returning
empty Bytes, `lotus_bytes_create` returning NULL through
`empty_global` — already surface NULL as degraded service, so the
cap converts a slow OOM into structural failure that surfaces
through the F.27 channel. This is the floor for a long-running
program leaking into the payload arena, not the fix; the fix is
per-subscriber arena routing for m70 + `__caller_arena` threading
for the stdlib primitives that land here.

**Phase-3 Task 11 intra-process bus per-subscriber routing
(2026-05-20).** Extends Task 9's per-sub arena pattern to the
intra-process `<-` path. Previously `lotus_bus_dispatch` enqueued
the publisher's struct bytes verbatim into each subscriber's
queue cell — payload String / Bytes pointers stayed aliased to
the publisher's locus arena. For long-running publishers (mdgw
normalizer class) that meant an unbounded leak in the publisher's
locus arena (the per-arena cap from Task 10 doesn't apply to
locus-owned arenas), bounded only by the publisher's eventual
dissolve — which for a daemon's root locus never happens.

The fix: when the codegen has synthesized a wire codec for the
payload type (the common case — every `<-` payload type gets
one), the dispatcher serializes the publisher's struct to wire
bytes once, then routes through `lotus_bus_dispatch_wire`. The
wire path's per-subscriber TLS routing rebuilds the struct in
each subscriber's own `__arena`; payload pointers end up bounded
by the subscriber's lifecycle. Cost: one serialize + N
deserializes per publish (N = matching subscribers). For
cooperative-only programs with no remote subs, the
previously-skipped serialize work is now paid on every publish.
For programs with both local and remote subs, the serialize cost
is amortized (same wire_buf feeds both).

A payload-typeless subject (no codegen-synthesized wire codec —
the `serialize_fn` arg is NULL) falls back to the legacy
verbatim enqueue, preserving the pre-Task-11 v1 behavior. This
escape hatch is intentional: it lets a hot-path subject opt out
of the round-trip cost when the publisher controls all
subscribers and can guarantee the payload's pointer-aliasing
discipline.

**Phase-3 Task 9 m70 per-subscriber arena routing (2026-05-20).**
`lotus_bus_dispatch_wire` no longer parks deserialized String /
Bytes pointers in the program-lifetime g_bus_payload_arena.
Instead it iterates the matching subscribers, sets the TLS
caller-arena (Task 8 indirection) to each subscriber's own
`__arena` (via the m20 fixed-offset slot-0 GEP), and deserializes
the wire bytes per-subscriber into that arena. The payload
pointers in the enqueued struct_buf now alias the subscriber's
own arena, bounded by the subscriber's lifecycle — no
program-lifetime deposit, no eventual OOM.

Cost: deserialize is invoked once per matching subscriber rather
than once total. Acceptable for typical fan-out (1–3 subs per
subject); high-fan-out subjects pay a real bill that could be
optimized via deserialize-once-then-clone-per-sub if a workload
demands it.

Closes the original Phase-2 (4) investigation's finding ("not
reclaimable under current semantics"): the answer was never to
reclaim the global arena but to skip it entirely — the m20 spec
("each subscriber's arena outlives the payload pointer") now
holds by construction because the deserialize-time allocator IS
the subscriber's arena.

**Phase-2 (4) `g_bus_payload_arena` reclaim investigation
(2026-05-19; superseded by Phase-3 Task 9).**
The handoff posed: "should `lotus_bus_dispatch_wire`'s
`g_bus_payload_arena` deposit reclaim per dispatch since m20
memcpy's into subscriber arena anyway?" The answer is no, and
the reason exposes a load-bearing constraint.

m20's `memcpy(copy, payload, size)` is a flat struct copy: the
publisher's payload bytes (size = compile-time-known struct size)
land in the subscriber's arena. The struct's String / Bytes /
TypeRef fields are POINTERS inside that struct; the memcpy copies
the pointers, not the pointed-to bytes. For cross-process wire
dispatch (`lotus_bus_dispatch_wire` → deserialize → struct_buf →
`lotus_bus_local_dispatch`), the deserialized String / Bytes data
lives in `g_bus_payload_arena`. After m20's struct memcpy, the
subscriber's copy still aliases that arena.

Handler-side assignment (`self.foo = payload.string_field`) is a
pointer store — `lotus_str_clone` is invoked only at *free-fn
return* boundaries, not at struct-field assignment. So if the
handler retains payload fields on its own struct, the retention
extends the `g_bus_payload_arena` deposit's lifetime to the
subscriber's entire lifetime. Reclaiming per dispatch would dangle
the subscriber's retained pointers.

Enabling per-dispatch reclaim requires changing handler-side
String / Bytes assignment to clone-on-store from payload, OR
introducing a per-dispatch arena that's reset only after every
subscriber's handler has run — neither is a small change. For
v1 the arena grows unbounded for high-message-rate cross-process
subscribers; bounding it is forward work, not a follow-up to F.28
/ F.29 / F.27. Documented here so the next surface that asks
"can we reclaim per dispatch?" finds the previously-investigated
answer.

**Phase-4 per-method scratch reclaim (2026-05-21).** Locus
method bodies (lifecycle `birth` / `run` / `accept` / `drain` /
`dissolve`, user-fn members, mode bodies) now open a per-call
scratch subregion of `self.__arena` on entry, route transient
allocations through it via `current_arena_ptr()`, and destroy
the subregion at every return point. Before this, every
allocation made by a long-running `run()` loop (JSON parse
strings, format-string concats, metric-label entries, every
stdlib primitive that lands on `lotus_caller_arena_or_global`)
landed in `self.__arena` directly — bounded only by the
locus's lifetime, which for a daemon's root locus or any
event-loop service is the entire process. fathom's kraken
mdgw measured 2.4 MB/sec growth on the L2 hot path before the
fix, OOM-killed at the 2 GB container cap every ~13 minutes
(see fathom's `lib/venues/kraken/FRICTION.md` "mdgw RSS
grows monotonically"). Post-fix the scratch resets each method
call so transient allocations have a bounded lifetime
matching the call's frame.

Two correctness invariants make this safe:

  1. Heap-typed `self.X = expr` stores deep-copy `expr` into
     `self.__arena` BEFORE the store, so the persisted
     pointer outlives the scratch destroy. `String`, `Bytes`,
     `TypeRef`, `Tuple`, `Array`, `Interface`, and
     payload-bearing `Enum` are heap-typed; `BytesView` /
     `StringView` / `LocusRef` / scalars / cells pass through.
     The copier reuses `emit_return_value_deep_copy`. Bytes
     now uses `lotus_bytes_clone` (was a pass-through under
     the previous program-lifetime assumption — broken once
     payloads can live in scratch).
  2. Heap-typed return values from a method are deep-copied
     into the caller's arena before the scratch destroy. The
     caller publishes its `current_arena_ptr()` via
     `lotus_set_caller_arena` immediately before each method
     call (mirroring the stdlib primitive contract). The
     callee snapshots `lotus_caller_arena_or_global()` at the
     method's entry block into a fn-local alloca and uses
     THAT snapshot — NOT a fresh TLS read at exit — as the
     deep-copy destination. The snapshot dance avoids a
     subtle bug where any nested method call inside the body
     would clobber TLS, leaving the epilogue to deep-copy
     into the wrong arena (whichever nested callee was
     called last).

Cost: two mallocs (subregion arena struct + initial 64 KiB
chunk) and two frees per method call. For typical short
methods this is roughly 100–400 ns of overhead per call; on
fathom's 70 L2/sec hot path with dozens of methods per frame
that's ~7 µs per second of aggregate overhead — invisible
next to the JSON parse / decimal arithmetic the methods do.
An `lotus_arena_reset` primitive (subregion reuse: keep
chunks, set `used=0`) could amortize this to ~5–10 ns per
call but is deferred — the leak's the load-bearing bug; the
fast-path optimization can follow.

Locus instantiation routing is unchanged. Child locus structs
allocate via their own routes (parent arena if parent accepts
them, lazy-global if the fn returns the child type, otherwise
stack alloca) — `lower_locus_instantiation` doesn't read
`current_arena_ptr()`. So a method body that does `let _w =
ChildLocus { };` still gets the child instantiation routed to
the parent's arena, not scratch, and the deferred-dissolve
mechanism continues to govern child teardown.

The free-fn path (`lotus_arena_create_subregion(__caller_arena)`
at entry, destroyed at exit, with allocations routed to
`__caller_arena` directly post-cross-seed-segv-fix) is
unchanged. Free fns called from inside a method body get
`__caller_arena = method's scratch` — anything they alloc
lives in the same scratch and gets reclaimed at the outer
method's exit. The cross-seed-segv test pattern (foreign
vec push from inside a free fn) continues to work because
those tests call from `main`, not from a method body. A
method body that pushes a heap value into a foreign locus's
vec without the wrapping locus owning it would still
dangle — same boundary the cross-seed-segv fix originally
documented; the fix here doesn't widen or narrow it.

**m49 closes the free-fn gap.** Every non-main free fn takes
an implicit `__caller_arena: ptr` first param at the LLVM ABI.
`main` keeps the program-wide `arena.global` it always had —
it's the single fn without a caller. Heap-typed returns of
Array, TypeRef-struct, or has-payload-Enum are rejected at
v0.1 — none currently appear as free-fn returns; ship as a
follow-up when a workload demands.

**Allocation routing (post-2026-05-18 cross-seed-segv fix,
commit 907837a).** Free-fn-body allocations now route to
`__caller_arena` directly. The earlier m49 design routed them
through a per-call subregion (`lotus_arena_create_subregion(
__caller_arena)`); that proved unsound because the codegen has
no escape analysis, so any value alloc'd in the subregion and
stored on a longer-lived structure (canonically: pushed onto a
`@form(vec)` on a foreign locus arg) dangled at fn-exit. The
fix routes allocations directly to `__caller_arena`. The
subregion is still created / destroyed at entry / exit so the
cleanup hooks for `fail E { ... }` payloads still have a
short-lived arena to anchor in, but the per-call performance
tier the subregion was meant to provide is deferred — it
needs escape analysis to ship safely. The fn-exit deep-copy
into `__caller_arena` is now a same-arena memcpy in the common
case (correct, marginally wasteful; can be elided in a
follow-up).

**Subregion elision for non-allocating bodies (FORM-3,
2026-05-13).** Codegen classifies each user fn at declare time
via a conservative syntactic walk
(`fn_body_definitely_non_allocating`). A body is non-
allocating iff every expression in it lowers to a known-non-
arena-touching shape: literals (incl. String — global static),
identifier reads, KwSelf, field/index reads (excluding
range-index slices), numeric/bool/bitwise Binary (Add excluded
since it could be String concat without type info threaded
in), Unary, If with non-allocating arms. For fns that pass the
classifier, the subregion `create` + `destroy` are skipped
entirely and the return-value memcpy epilogue is skipped — the
return value is either a primitive or a pointer to a region
stable across the fn frame (String-literal global, caller-passed
pointer, field read of one of those). Closes the bench's
per-call cost for leaf fns (`fn_call` went 188 ms → 37.1 ms =
5×, ratio vs Go 0.04× → 0.21×; `form_vec_push` reached 1.00× =
Go parity). The `__caller_arena` LLVM param is still passed
even to non-allocating fns (kept uniform per-fn ABI); the
optimization is purely on the body side. Fallible fns always
pay the full subregion lifecycle because `fail E { ... }`
allocates the payload struct into the subregion. Post-907837a
the elision benefit is narrower than its m49-era framing: with
allocating-body allocations now routed to caller-arena directly
(see "Allocation routing" above), the deep-copy epilogue is a
same-arena memcpy and the subregion lifecycle is mostly
overhead for cleanup hooks — the optimization still skips
both, just with a smaller per-call cost being avoided.

This delivers the spec's "every free function has its own
implicit locus" memory boundary at the codegen substrate.
Bound handles in free fn bodies still attach to the enclosing
deferred-dissolve frame (lifecycle parity with main and
lifecycle methods); the implicit-locus *handle-rooting*
semantic — fn return waits for in-fn-bound children to
dissolve as if the fn were itself a locus — remains a
future-work item, not exercised by any current example.

### Per-projection-class arena strategies (m22 + m23)

Each locus's projection class is resolved at codegen-declare
time: from an explicit `: projection rich|chunked|recognition`
annotation when present, otherwise per the spec/memory.md
default rule (chunked if the locus declares accept; rich
otherwise — recognition is explicit-only).

The class drives the *child arena allocation* strategy when this
locus accepts coordinatees:

- **Rich** parents: each child gets a fresh top-level arena via
  `lotus_arena_create()`. Independent allocation lifetime;
  parent does no bookkeeping. v0 default for non-coordinator
  loci.
- **Chunked** parents: each accepted child gets a sub-region via
  `lotus_arena_create_subregion(parent_arena)`. The parent
  tracks a slot index per child; on child dissolve, the slot
  returns to a per-arena free-list so peak slot space stays
  O(concurrent children alive), not O(total children ever
  accepted). Per F.3.
- **Recognition** parents (v1.x-3): the sub-mode commitment
  spelled at the declaration site picks the allocator family.
  `fixed_cell` routes children through
  `lotus_recpool_fixed_acquire` (bitmap-tracked cells, inline
  arena per cell); `shared_slab` routes children through
  `lotus_recpool_slab_acquire` (every child shares one bump
  arena). Cell stride for either sub-mode is derived at
  codegen time from the parent's accept-method param type
  union — not a user-supplied byte budget. At parent instantiation the recpool is
  allocated via the matching `_create` fn and stashed on the
  synthetic `__recpool: ptr` struct field; at parent dissolve
  it's torn down via `_destroy`. The child's arena teardown
  is dispatched at the C ABI level: a discriminator on the
  child struct picks `lotus_arena_destroy` (kind=0, regular),
  `lotus_recpool_fixed_release` (kind=1), or
  `lotus_recpool_slab_release` (kind=2). The surface contract
  ("parent owns a pool, no dynamic allocation in steady
  state for `fixed_cell`; one bump for `shared_slab`") is
  exercised by `examples/14-projection-classes`.

C runtime ABI as of v1.x-3:

```
ptr  lotus_arena_create(void)
ptr  lotus_arena_create_subregion(ptr parent)   // m22
ptr  lotus_arena_alloc(ptr arena, i64 size, i64 align)
void lotus_arena_destroy(ptr arena)             // auto-detects sub-region
                                                // and returns slot to
                                                // parent's free-list
ptr  lotus_recpool_fixed_create(i64 cap, i64 bytes)   // v1.x-3
ptr  lotus_recpool_fixed_acquire(ptr pool)            // v1.x-3
void lotus_recpool_fixed_release(ptr pool, ptr arena) // v1.x-3
void lotus_recpool_fixed_destroy(ptr pool)            // v1.x-3
ptr  lotus_recpool_slab_create(i64 cap, i64 bytes)    // v1.x-3
ptr  lotus_recpool_slab_acquire(ptr pool)             // v1.x-3
void lotus_recpool_slab_release(ptr pool, ptr arena)  // v1.x-3
void lotus_recpool_slab_destroy(ptr pool)             // v1.x-3
```

`lotus_arena_destroy` is unified across the regular + subregion
shapes — it inspects the arena's optional `parent` pointer and
slot, and returns the slot to the parent's free-list when
present. The recpool variants are NOT unified with
`lotus_arena_destroy` — their backing storage layout is
distinct (inline-in-cell for fixed_cell; shared-bump for
shared_slab) and routing through `lotus_arena_destroy` would
corrupt the recpool's bookkeeping. The codegen dispatch
discriminator (`__recpool_release_kind`) is what keeps the
right release function reachable at child dissolve.

## Future work

- **Hot-load preservation across perspective updates.** When a
  perspective is hot-loaded, the receiving locus's arena state
  is preserved across the swap; the new perspective's translation
  functions replace the old. v0 specifies the perspective hot-
  load mechanism (runtime.md); the memory-level interaction is
  TBD.
- **Region size hints.** Initial chunk sizes per locus are
  taken from declared params. Per The Design's locus-as-region
  invariant, the load-bearing property is *lifetime* (wholesale
  free at dissolve), not *fixed size*. The C-runtime arena
  grows linked-list chunks on demand: when the head chunk
  can't fit a request, a fresh chunk is allocated and pushed
  on the front. Declared params are sizing hints, not
  ceilings — a locus that out-allocates its declared budget
  doesn't panic, it just adds chunks. Compaction across
  long-lived chunked loci stays deferred (see below).
- **Compaction passes.** For long-running chunked-class loci
  with high churn, periodic compaction may be needed. Currently
  free-list reclamation is sufficient for v0; compaction passes
  are deferred.
