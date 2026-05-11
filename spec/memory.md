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
is how `02-parent-child`'s `Coord.accept(g: GreeterL)` fires for
each `GreeterL { ... }` instantiated in the coordinator's `run()`
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

**m49 closes the free-fn gap.** Every non-main free fn now takes
an implicit `__caller_arena: ptr` first param at the LLVM ABI.
At body entry the callee opens a subregion of `__caller_arena`
via `lotus_arena_create_subregion(__caller_arena)`; the body's
allocations route through that subregion (a new tier between
`current_self`'s arena and `arena.global` in the codegen-side
allocation routing). At return, the body branches to a unified
`fn.exit` epilogue that deep-copies the return value into
`__caller_arena` (identity for value types; `lotus_str_clone`
for String; recursive walk for Tuple), destroys the subregion
wholesale, and emits `build_return`. `main` keeps the
program-wide `arena.global` it always had — it's the single fn
without a caller. Heap-typed returns of Array, TypeRef-struct,
or has-payload-Enum are rejected at v0.1 — none currently
appear as free-fn returns; ship as a follow-up when a workload
demands. This delivers the spec's "every free function has its
own implicit locus" memory boundary at the codegen substrate.
Bound handles in free fn bodies still attach to the enclosing
deferred-dissolve frame (lifecycle parity with main and lifecycle
methods); the implicit-locus *handle-rooting* semantic — fn
return waits for in-fn-bound children to dissolve as if the fn
were itself a locus — remains a future-work item, not exercised
by any current example.

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
- **Recognition** parents: same code path as chunked at
  v0 — sub-region allocation with free-list bookkeeping. The
  spec's pre-allocated bitmap-cell pool is a perf optimization
  (avoids `malloc` per accept) deliberately deferred until a
  workload exercises it. The annotation parses + resolves +
  routes correctly; the `Recognition` arm is *behaviorally*
  equivalent to `Chunked` until the optimization lands. The
  surface contract (parent owns a pool of fixed-size cells, no
  dynamic allocation in steady state) is exercised by
  `examples/14-projection-classes`.

C runtime ABI as of m22:

```
ptr  lotus_arena_create(void)
ptr  lotus_arena_create_subregion(ptr parent)   // m22
ptr  lotus_arena_alloc(ptr arena, i64 size, i64 align)
void lotus_arena_destroy(ptr arena)             // auto-detects sub-region
                                                // and returns slot to
                                                // parent's free-list
```

`lotus_arena_destroy` is unified across kinds — it inspects the
arena's optional `parent` pointer and slot, and returns the slot
to the parent's free-list when present. Callers always emit a
single destroy call regardless of how the arena was created.
This keeps the codegen side simple: it doesn't have to remember
which create variant was used.

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
