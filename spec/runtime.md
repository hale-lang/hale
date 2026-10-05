# Runtime

What every compiled Hale binary always ships with. Always-
loaded; not optional; no `import` needed; the substrate every
Hale program depends on.

This document distinguishes the **runtime** (always there) from
the **standard library** (`stdlib.md`, importable but bundled).
Go's distinction between `runtime` and other stdlib packages is
the model: runtime is automatic; stdlib is explicit.

> **Naming note:** The language is **Hale**; the runtime/
> substrate concept is called **lotus**, and the C-runtime
> symbols stay `lotus_*` (per project memory). When this doc
> says "lotus" it means the substrate; "Hale" means the
> language proper.

## What's in the runtime

### Memory

- **Region allocator.** Per-locus arenas, hierarchical, freed
  on dissolution. Bump allocation within a region; no per-object
  metadata; no GC. The framework's lotus structure provides the
  scope; the allocator just respects it.
- **Per-method scratch.** A locus method body
  (lifecycle / user-fn / mode) opens a per-call subregion of
  `self.__arena` at entry and destroys it at every return —
  *unless* the body provably allocates nothing and returns a
  by-value scalar (or Unit), in which case the scratch is elided
  (2026-06-28; an optimization with no observable effect — there's
  nothing to reclaim, so skipping the subregion just removes a
  `malloc`/`free` per call). Transient allocations made inside the
  body — `to_string`,
  `String` concat, `std::str::*` / `std::json::*` / `std::bytes::*`
  results, format-string composition — route through the
  scratch via `current_arena_ptr()` and get reclaimed at method
  exit. Heap-typed `self.X = expr` stores deep-copy into
  `self.__arena` before the store so persisted state outlives
  the scratch destroy. Heap return values are deep-copied into
  the caller's arena via a fn-local snapshot of
  `lotus_caller_arena_or_global()` taken at the method's entry
  block. Callers publish their `current_arena_ptr()` via
  `lotus_set_caller_arena` immediately before each method call
  (same TLS contract as stdlib primitives). Without this,
  long-running `run()` loops accumulated every transient
  allocation into the locus's lifetime arena — a real
  workload measured multiple MB/sec of growth on a hot
  message-dispatch path, OOM within minutes under a typical
  container cap. See
  `spec/memory.md` "Phase-4 per-method scratch reclaim" for
  the full design (invariants, cost model, interaction with
  the cross-seed-segv routing).
- **Per-projection-class allocation strategy.** Rich → simple
  arena; chunked → arena with per-coordinatee sub-regions;
  recognition → recpool, sub-mode-typed at the declaration
  site (see "Recognition pool allocators" below). Selected
  at compile time per locus.
- **Free-list within parent for bookkeeping reclamation.** When
  a coordinatee dissolves, its bookkeeping slot in the parent's
  arena is reclaimed via a per-arena free-list (chunked-class
  loci) or periodic defrag (high-churn loci). Reclamation is
  **per-arena**, **bounded**, **deterministic** — never stop-
  the-world. Coordinatee sub-regions remain pristine arenas
  freed wholesale on dissolution.
- **F.22 capacity-slot allocators.** Each `pool X of T;` /
  `heap Y of T;` declaration on a locus adds a per-instance
  allocator. The C runtime ships two symbol families:

  | Family | Surface | Backing |
  |---|---|---|
  | `lotus_pool_*` | `create(cell_size, cell_align) -> pool*`, `acquire(pool) -> cell*`, `release(pool, cell)`, `destroy(pool)` | Linked list of chunks; each chunk is one malloc holding N contiguous cells. Free-list threads through the cells themselves (each free cell stores the next-free pointer at its base). Chunks grow geometrically (initial sized so one chunk fits in a host page when stride permits, else 16 cells; doubling; capped at 4096). Cell stride = max(cell_size, sizeof(void*)) aligned to cell_align. v1.x-17: initial chunk cell count is `max(16, page_size / cell_stride)` capped at 4096 — sysconf(_SC_PAGESIZE) queried once and cached; falls back to 4 KiB on systems where sysconf returns implausible values. |
  | `lotus_heap_*` | `create(cell_size, cell_align) -> heap*`, `alloc(heap) -> cell*`, `free(heap, cell)`, `destroy(heap)` | Doubly-linked live list with intrusive header (prev/next pointers) sitting just before each cell. `free()` unlinks in O(1); `destroy()` walks the list and frees every still-live cell wholesale. |

  Both allocator families are type-erased at the C ABI (sizes
  + aligns are i64 args). Cell alignment is 8 bytes uniformly
  in v0; loci with cells requiring >8-byte alignment (e.g.
  AVX-aligned types) are not supported. Per F.22 §"Slot
  lifetime", slot init runs after slot 0 / arena and destroy
  runs in reverse before slot 0 / arena. See
  `spec/semantics.md` "Capacity slot lifecycle and dispatch
  (F.22)" for the language-level surface and `spec/memory.md`
  "Capacity slots (F.22)" for the lotus-substrate framing.

- **Recognition pool allocators (v1.x-3).** A locus with
  `: projection recognition(cap=N, <sub_mode>)` allocates one
  recpool at instantiation; child loci accepted by that parent
  draw their arena from the pool instead of `lotus_arena_create
  _subregion`. Two symbol families ship in v1:

  | Family | Surface | Backing |
  |---|---|---|
  | `lotus_recpool_fixed_*` | `create(cap, cell_bytes) -> recpool*`, `acquire(recpool) -> arena*`, `release(recpool, arena)`, `destroy(recpool)` | One contiguous block of `cap × cell_stride` bytes. Each cell carries an INLINE `lotus_arena_t` + `lotus_arena_chunk_t` header at its front followed by `cell_bytes` of payload — the cell IS the child's arena. Bitmap (`uint64_t[ceil(cap/64)]`) tracks occupancy; acquire scans the lowest unset bit via `__builtin_ctzll`. Release clears the bit so the slot is reusable. The returned arena has `fixed_size=1`, so `lotus_arena_alloc` returns NULL on overflow (caller routes to the closure-violation channel). |
  | `lotus_recpool_slab_*` | `create(cap, slab_bytes) -> recpool*`, `acquire(recpool) -> arena*`, `release(recpool, arena)`, `destroy(recpool)` | One `lotus_arena_t` with an initial chunk of `slab_bytes` and `fixed_size=1` so it never grows. Every `acquire` returns the SAME arena pointer — children share the bump space and per-child release is a no-op. The whole slab frees at parent dissolve via `lotus_arena_destroy(slab_arena)`. `cap` is recorded but not enforced at the C layer (codegen's birth-cap check bounds concurrent children; the slab is a memory budget). |

  Both families return `lotus_arena_t*` from `acquire` so child
  body code stays projection-class-agnostic per the F.22
  architectural invariant — the same `arena_alloc` path handles
  fresh, subregion, fixed-cell, and shared-slab children. The
  codegen dispatch at child dissolve picks the matching
  `release` fn via the synthetic `__recpool_release_kind`
  discriminator (0 = regular `arena_destroy`, 1 = fixed_cell
  release, 2 = shared_slab release). v1 ships `fixed_cell` and
  `shared_slab`; `spillover` and `summary_only` parse + AST
  through but reject at typecheck with a `v1.x pending`
  diagnostic (the spillover malloc-fallback machinery and the
  `summary_only` "no child arena allocation" type-system rule
  are separate work).

### Lifecycle

- **Lifecycle dispatcher.** Invokes `birth → run → drain →
  dissolve` per locus; invokes `accept` on coordinatee
  attachment; invokes `on_failure` on child failure with the
  parent's policy.
- **Interest-based ownership (accept bubbling)**.
  `accept(c: I)` collects not only a *direct* child but the
  nearest such acceptor for an `I{}` instantiated anywhere in the
  subtree: when a locus instantiates `I{}` and its direct
  enclosing locus does not `accept(I)`, ownership *bubbles* to the
  nearest enclosing ancestor that does (innermost-wins).
  Resolution is entirely static — there is no polymorphic locus
  instantiation, so the closed-world instantiation graph fixes
  every owner edge at compile time; no runtime ancestor walk.
  Backward-compatible by construction: innermost-wins selects the
  direct parent whenever it accepts, so no existing parent↔child
  edge changes; bubbling only *adds* an owner where a child would
  otherwise be a transient throwaway. An `I{}` with no accepting
  ancestor stays transient — ownership is opt-in via `accept`, and
  the absence of an owner is never an error. Same-tower bubbling
  costs nothing beyond the direct-parent case (the owner pointer is
  a constant for a singleton owner, or threaded down the birth
  chain for multiple owner instances — giving each owner instance
  its own isolated collection — then the ordinary accept path). A
  cross-pool owner (e.g. a `main locus` registry collecting
  entities spawned on a worker pool) is served by an async handoff
  over the bus queue: the child is born on the owner's thread and
  reclaimed by the owner's same-thread cascade, so a cross-pool
  `I{}` is **fire-and-forget** — it may only be a bare statement;
  using the instance as a value is rejected at compile time. Which
  of the two an `I{}` is follows the placement table per instance
  of the enclosing locus (F.40 phase 3, P1): the enclosing instance
  is paired with the owner instance above it, and a locus nested
  under a field placed off main runs on that field's thread. When
  an enclosing locus has instances on both sides of its owner's
  thread and the owner is a `main locus`, a bare `I{};` tests
  `lotus_on_main_thread()` at the literal and takes the same-tower
  birth on main, the handoff off it; a value use there, or such an
  edge to an owner with several instances, is refused at the
  literal.
- **Order by construction, with latches.** A locus can't run
  before its birth completed, can't be torn down twice, etc.
  There is no runtime state machine: the order is the order the
  compiler emits, and the latches (`__arena` NULL,
  `__drain_requested`, `__quarantined`, the held-failure node's
  state) keep a step from running twice (§ "Lifecycle
  obligations", line 14).
- **`drain()` cascades depth-first.** Calling `drain()` on a
  locus first recursively drains all its children (depth-first),
  waits for them, then drains itself. SIGINT and SIGTERM raise
  the process's draining flag and call no `drain()`: the `run()`s
  that read `self.draining` return, and the ordinary teardown
  follows (§ "Process control", § "Lifecycle obligations",
  line 15). No separate cascade syntax — `drain()` is always
  cascading. For locus-typed param fields specifically
  (F.29), the codegen walks `LocusRef` fields in declaration
  order at the cascade-teardown sites (ephemeral scope-exit
  and deferred-flush) and calls each child's drain BEFORE the
  outer locus's own drain. The subsequent dissolve cascade
  runs the outer's `closures → dissolve` body next, then per
  child `closures → dissolve → arena_destroy`, then outer's
  arena_destroy. **The walk is recursive, to the leaves**: a
  child's own `LocusRef` fields drain before the child does
  and are dissolved (and their arenas destroyed) before the
  child's arena, which holds their structs. Every level's
  gate is that level's own ownership mask, so a subtree handed
  in from outside is skipped wherever it appears and is torn
  down once, by its real owner.
  A param field typed by a **contract** rather than by the
  child's locus — an `interface` slot, a `perspective(P)`
  handle — carries an owned child on the same terms, and the
  cascade reaches it: the declared type names no impl, so the
  instantiation records the child's teardown in a synthetic
  per-field slot, a pair of its drain and the rest of its spine
  (dissolve → arena reclaim), and the cascade runs each half
  through it where a `LocusRef` field's runs, under the same
  ownership-mask gate: the drain with the other fields' drains,
  before the outer's drain, the rest after the outer's dissolve.
  The consequence users can check is arena residency: no locus
  arena, at any depth and behind any field type, survives its
  owner. A pinned locus's thread drains its own fields before
  its `drain()`; their dissolve cascade runs after its join.
  The cascade walks the fields in the order the lifecycle plan
  places them (§ "Lifecycle obligations", line 12: declaration
  order). Each field's dissolve completes before the next starts;
  physical release may wait for an active run. The owner retains
  those children until its own storage can be released.
  An `accept`'d child is reclaimed on its
  OWN run-completion / `terminate` when it is a flow (see
  "Per-child reclamation" below) rather than waiting for the
  parent's cascade.
- **Per-child reclamation**. An `accept`'d child's
  `run()` is posted to its pool as a coro, run through a
  synthesized `__coop_pool_run_<L>` wrapper. When that run()
  completes, the wrapper reclaims the child — drain → (for a
  flow) the parent's `release(owner, self)` → dissolve →
  arena/recpool release — iff the child is a **flow** (some
  declared locus has `release(c: ThisType)`) OR it set the
  `__drain_requested` latch via `terminate;`. A non-flow
  ("resident") child whose run() merely returns is NOT reclaimed
  (it lives to parent dissolve). The reclaim runs on the child's
  own pool worker while its arena is valid; `emit_locus_arena_
  destroy` is idempotent (NULLs `__arena`), so a later
  parent-dissolve of the same locus no-ops. At pool shutdown a
  coro may still be PARKED (a listener in `accept()`); the
  **wakeable park** handles it: a per-pool wake `eventfd` in the
  pool's epoll lets `shutdown_all` unblock a worker sitting in
  `epoll_wait(-1)` (the condvar broadcast can't), so the worker
  returns from the drain and the join completes instead of
  hanging. The parked coros are then *abandoned* — their stacks
  freed without resuming them — because a forever-loop `run()`
  (`while true { accept }`) cannot be cooperatively unwound
  without the loop checking `self.draining` (a future
  refinement), and the process is exiting anyway. Per-child
  reclamation proper (terminate / flow run-completion) never
  needs this: there the coro returns from `run()` on its own.
- **Classic-pool blocking-accept shutdown.** A
  *classic* (non-`async_io`) pool worker blocked in a blocking
  `accept(2)` inside a locus's `run()` (e.g. `std::http::Server`
  or `std::io::tcp::Listener` placed on a plain
  `cooperative(pool = X)`) can't be woken by the wake `eventfd`
  (there is no epoll on a classic pool) — so two rules keep its
  teardown clean: (a) the classic `accept` polls the listen fd
  with a short timeout and checks its pool's shutdown flag, so it
  returns a sentinel `-1` once `shutdown_all` is signalled and the
  stdlib accept loops (`Server`/`Listener`) break out of their
  forever loop; and (b) the **main locus joins all pool workers
  before dissolving its `params` fields** — whether it dissolves
  eagerly or, being a subscriber, at the exit of the fn that
  instantiated it, `main` or another (`fn main() { start() }`
  with the locus built in `start`; GH #1148) — so a worker still
  executing a pool-placed field's `run()` can never touch that
  field's arena after it's freed (the alternative — freeing first
  — is a use-after-free; the alternative join-without-(a) is a
  hang). Together these let a program whose `main` run() returns
  while a classic-pool server child is live shut down cleanly
  rather than hanging or segfaulting.
- **Teardown delivery contract (GH #253, 2026-07-27).** A
  dissolving parent joins its own **pinned** children — mailbox
  shutdown, `pthread_join`, then a bus drain — BEFORE cascading
  its field children's drain/dissolve, and the fn-exit flush
  likewise joins subscription-less pinned entries before any
  cooperative entry's teardown. Consequence: events a pinned
  worker publishes in its final moments (up to `run()` return)
  are **delivered** to sibling subscribers, in any declaration
  order — a parent whose `run()` returns immediately no longer
  silently drops its workers' last publishes (the hale-bun
  install-fanout shape). This is the pinned mirror of the
  pool-worker join rule above, and it also closes the lifetime
  hazard of destroying the owner's arena (which holds the pinned
  children's self structs) while their threads still ran. What
  still drops, by contract: a publish made **after** its last
  subscriber has dissolved — e.g. from a pool-placed publisher
  whose queued work outlives the process teardown, or from a
  `dissolve()` body publishing to a subscriber that tore down
  earlier in the same cascade. Those cells are discarded via the
  deregister-on-dissolve invariant (never dispatched to freed
  memory); `LOTUS_BUS_LOG_DROP=1` surfaces them. A workload that
  must not lose such events coordinates completion explicitly
  ("exit when done" — poll a tally / await a completion event)
  rather than relying on teardown ordering. **Deferred-path
  completion (Crumb batch-4, 2026-07-28):** the contract holds
  regardless of which teardown path the parent takes. Previously a
  bus subscription on the main locus made it long-lived → deferred,
  and the deferred flush processed the parent (cascading its
  subscriber fields) BEFORE joining its own pinned children — the
  exact inversion the contract forbids, triggered by the one-line
  addition of a `subscribe` on the root, dropping every in-flight
  result silently. A deferred parent's own pinned entries are now
  re-ordered after its own frame entry, so the reverse-order flush
  joins + drains them while every subscriber field is alive —
  identical semantics to the eager path.
- **Recovery primitives.** `restart`, `restart_in_place`,
  `quarantine`, `reorganize`, `bubble`, `dissolve`, `drain` —
  all language keywords; runtime implements the actual
  effects.

### Scheduler — multi-scheduler cooperative

Lotus uses a **multi-scheduler cooperative** model (closest
existing analog: Erlang BEAM, *not* Go's M:N). The reasons are
framework-discipline:

- **Lateral-access prohibition is physical, not just typed.**
  Within a single cooperative scheduler, sibling loci cannot
  run concurrently — only one locus is executing at a time per
  scheduler. There is no thread of execution that could attempt
  a lateral memory reference. The compile-time type rule
  ("vertical-only flow") is reinforced by the substrate.
- **Substrate-cell atomicity is naturally aligned.** Cooperative
  yield points — between message-handler invocations, between
  lifecycle phases, on bus dispatch — are exactly where the
  substrate-cell boundary lives. No preemption inside a
  substrate-cell because the runtime can't preempt at all;
  it only switches at yield points.
- **Per-scheduler region allocators.** Each scheduler is
  single-threaded, so its allocator state is naturally
  per-scheduler with no synchronization. Lock-free by
  construction.
- **Failure-traversal is a call-stack walk on one scheduler.**
  No cross-thread synchronization for parent-catches-child
  failure when both are on the same scheduler.

Concurrency comes from running **multiple cooperative schedulers
in parallel** (one per CPU core, by default). Loci belong to a
specific scheduler; cross-scheduler communication uses the bus
just like cross-process communication. Loci may be migrated
between schedulers transparently for load balancing because all
their communication is bus-mediated already.

Specifically:

- **One scheduler per CPU core** at startup, configurable.
- **Cooperative yield points**: between handler invocations,
  between lifecycle transitions, on bus message dispatch, on
  explicit `yield` (rare, for long-running computations).
  Plain fn exit is NOT a yield point — and since 2026-07-02 a
  proven-non-allocating fn's exit provably skips the queue drain
  (a non-allocating body cannot have published; payload copies
  allocate). A cooperative compute-only loop that leaned on
  helper-call returns for delivery never had that guarantee and
  must use an explicit `yield;`.
- **No preemption within a scheduler.** A locus's handler runs
  to completion or an explicit yield.
- **Cross-scheduler is bus.** No shared memory; no locks.
- **Failure-traversal**: if parent and child are on the same
  scheduler, failure-traversal is a stack walk. If different
  schedulers, the failure is delivered as a typed bus message
  to the parent's scheduler, which dispatches to `on_failure`.

### Placement classes (per-locus execution strategy)

Just as **projection class** governs a locus's memory strategy,
**placement class** governs its execution strategy. Placement
is a *deployment seam*, not an intrinsic property of the locus
(see `spec/decisions.md` § F.31). Placement entries
live in a `placement { }` block on `main locus` only, parallel
to `bindings { }` for bus topology:

```hale
main locus App {
    params {
        gateway_a:   Gateway = Gateway { venue: "venue-a" };
        gateway_b: Gateway = Gateway { venue: "venue-b" };
        metrics:          MetricsServer = MetricsServer { port: 9100 };
        ui:               Renderer = Renderer { };
    }
    placement {
        gateway_a:   pinned(core = 1);
        gateway_b: pinned(core = 2);
        metrics:          cooperative(pool = io);
        ui:               cooperative(pool = render);
        // unspecified main-locus params → cooperative(pool = main)
    }
}
```

Placement remains honestly **bimodal**: either a locus shares
a cooperative pool (an OS thread running a cooperative drain
loop) or it owns its own OS thread. There is no third position.

| Class | Yield discipline | Resource |
|---|---|---|
| **`cooperative(pool = X)`** (default for unspecified main-locus params, with `X = main`) | Yields between substrate cells (handler exit, lifecycle transition, bus dispatch, `time::sleep`, explicit `yield`). `time::sleep` slices into ≤100ms intervals and folds in `lotus_bus_queue_drain` for the locus's pool after each slice, so cells posted by other threads deliver mid-loop even during a long keep-alive sleep. Handler bodies are atomic. | Shares pool `X`'s OS thread with other cooperative loci placed on the same pool. |
| **`pinned`** / **`pinned(core = N)`** / **`pinned(cores = A..B \| A..=B \| {a, b, c})`** / **`pinned(node = N)`** / **`pinned(l3 = name)`** / **`pinned(..., replicas = K)`** | No yield to siblings; owns its OS thread. Bus events to/from cross-thread boundaries via formal mailbox post. | Dedicated OS thread. `core = N` pins it to one CPU; `cores = ...` (Phase 1a) sets its mask to a core *set*; `node = N` / `l3 = name` (Phase 1b) set the mask to a NUMA node / cache domain from `topology { }` (and bind the arena there); `replicas = K` (Phase 1c) fans into K single-threaded instances, one per core. The OS schedules freely within the mask. Linux-only; best-effort no-op elsewhere. |

**Pool inference rule.** The cooperative pool set is inferred
from `cooperative(pool = X)` references in the `placement { }`
block. The runtime spawns one OS worker thread per inferred
pool name beyond `main` (which is always the program's main
thread). No separate `threads { }` declaration block at v1 —
when per-pool attributes (priority, affinity, realtime hint)
become useful, the block lands then as a typed extension. Pool
`main` exists in every program regardless of whether
`placement { }` references it.

**Nested-instantiation inheritance.** Placement entries apply
only to top-level `main locus` `params` fields. Loci
instantiated nested in another locus's body (in `birth` /
`run` / lifecycle methods, or as let-bound children) inherit
the parent's pool. There is no way to spell "this nested child
runs on a different pool than its parent" — that would require
the nested-instantiation expression to carry placement, which
would re-mix the deployment and intrinsic layers F.31 separates.

**Topology block (Phase 1b).** A `main locus` may
also declare a `topology { }` block — a **declare-only**
description of the host's core partition, a sibling deployment
seam to `placement { }` / `bindings { }`:

```hale
topology {
    reserve cores 0..2;              // held back for the OS / main
    node 0 {
        l3 fast { cores 4..8; }      // a CCD / shared-L3 group
        l3 slow { cores 8..12; }
    }
    node 1 {
        l3 heavy { cores 12..16; }
    }
}
```

A `placement { }` entry then targets a domain: `pinned(node =
0)` sets the thread's affinity mask to node 0's core set (the
union of its L3 domains — here `{4..12}`), and `pinned(l3 =
fast)` sets it to the named domain's cores (`{4..8}`). The
compiler resolves the domain to a concrete core set at compile
time (closed-world: node ids and domain cores are literals),
reusing the same cpuset affinity mechanism as `pinned(cores =
...)`. Validation (unique node ids, globally-unique L3 names,
non-overlapping domains, no domain/reserved overlap, and every
`pinned(node/l3)` referencing a declared domain) is static. L3
domain names go through the identifier rule, so a hard keyword
(e.g. `bulk`) can't name a domain.

**Replicas (Phase 1c).** `pinned(..., replicas = K)` is the
parallelism sugar: it fans the field into **K single-threaded
instances**, replica `i` pinned to one core of the affinity set
(round-robin — `pinned(cores = 4..12, replicas = 8)` puts replica
`i` on core `4 + i`; more replicas than cores wraps; with no
affinity the K instances are OS-scheduled). This is deliberately
*not* a multi-worker pool — a cooperative pool is one consumer
thread, and the lock-free rings, bus devirtualization, and
single-threaded-method guarantee all rest on that invariant.
Parallelism instead comes from more single-threaded units, each
its own single consumer, so every invariant survives. `replicas`
is **pinned-only** (K cooperative loci on one pool would share a
thread, which isn't parallel) and composes with the topology
targets (`pinned(node = 0, replicas = 4)` fans across node 0's
cores with each replica's arena bound to node 0). Codegen emits K
instantiations at the field's init site; all K register their bus
subscriptions (a subscribed topic fans out to every replica) and
all K are joined + dissolved at parent teardown via the deferred-
dissolve frame. The replicas are non-addressable — there is no
`field[i]` surface; they are workers that pull from the bus or run
their own loop.

**The dispatch plan (GH #476 Change 8).** Which lowering a
subject's dispatch receives — `dynamic` (runtime hash lookup),
`static_bucket` (compile-time subject id, dispatch still queued),
`static_direct` (synchronous direct calls to every subscriber) —
is a CONCLUSION derived from the canonical model, not a fact
declared anywhere in the program. `DispatchPlan::derive(
&ApplicationModel)` owns that derivation: the bus graph's
per-subject eligibility gates decide the flavor (a single ladder,
`DispatchFlavor::of`, which the backend calls rather than
re-deciding; `static_direct` takes three legs: every publisher and
subscriber same-thread, every handler quiet, and the gate's
`payload_flat` column, a payload struct whose every field is an
inline scalar — a direct-eligible subject with a managed payload is
`static_bucket`), and the Change-8 arrangement supplies each row's
publisher/subscriber **thread domains** and `same_domain` — "every
publish site and every subscriber of this subject sit in one
domain", the precondition the future placement-driven flavors
(GH #464) need, withheld rather than guessed whenever the model
cannot place a locus's WHOLE population — a locus with both an
arranged instance and a dynamic birth is incomplete, and its
arranged instance does not answer for the instances the model
admits it cannot see. A locus's domains are found by the gates'
own spelling of it, the canonical post-merge name, never the
display name, so a locus of an imported seed has its domains like
any other (before F.40 phase 3's C5 they were found by the display
name, which demangles an imported locus and never met the gates'
spelling, and every such row printed no domain). Plan subjects are
WIRE subjects; `hale model dump` prints the plan and the
same-domain count.

The gates' placement leg reads the placement table (F.40 phase 3,
P1): a type is same-thread only when every instance of it runs on
main, so `static_direct` needs every publisher and every subscriber
type to be. An instance nested under a root field placed off main
runs on that field's thread, an adapter in `bindings { }` on its
own, and an instance the table cannot place (a literal in a scope
whose domain is unknown) is never taken for main; each keeps its
subject on a queued flavor. The same answer decides where a
subscriber's `bounded(N, …)` is legal (main-queue registrations
only, `spec/decisions.md` F.37).

**Binding roles and replica indices in the model.** A binding's
role is the authored `role:` kwarg when present and otherwise the
inferred one — publish-only is `connect`, subscribe-only is
`listen`, and an ambiguous or unused binding is a typecheck
diagnostic, never a default. One rule serves both the desugar and
the canonical model (`hale_syntax::desugar::binding_role_for`);
before GH #476 Change 8 the desugar ran its half AFTER topic
references were rewritten to literal subjects, so the publish and
subscribe ends were always empty and no role was ever inferred —
every binding had to spell `role:` or codegen refused it. Loss
behavior follows the role, because the runtime does: the connect
side is the publish side, where a send failure marks the entry lost
and `or wait` parks through the reconnect window; the listen side
re-arms on peer EOF and a link it cannot serve is structural.
A `pinned(..., replicas = K)` field contributes K instances whose
model rows carry their own 0-based INDEX — the same `i` codegen
bakes and a keyed subscriber registers under — and the model
refuses a replica set that is not contiguous from 0. Replica-ness
belongs to the LAST path component: the arrangement walks into each
replica, so what a replica owns (`App.workers[0].leaf`) is an
ordinary child that inherits its owner's domain and carries no
index of its own.

A subject named by a `bindings { }` entry is never devirtualized:
an external peer is (or may be) the real counterparty, so the
dispatch has to go through the transport the binding realizes.
That exemption is keyed at BOTH grains — the topic decl name a
binding entry writes and the wire subject a desugared program
dispatches on — because the backend builds its graph after topic
desugaring, and a decl-name-only gate silently let a bound subject
be bucketed past its own adapter.

The plan is part of what a build **is**: its digest is framed into
the executable identity (`exec_digest`) alongside the toolchain
hash, the compiler version, the build options, and every source
byte. The plan framed there is the one the backend lowers: derived
once from the bus graph over the merged, topic-desugared program
the backend walks, with the same ladder, and without the
arrangement (its `same_domain` column is not consulted by any
flavor today). Two builds of identical sources that lower dispatch
differently — notably the all-dynamic `LOTUS_NO_BUS_DEVIRT=1`
control arm — therefore have different identities, and a recording
made under one is refused against the other rather than replayed
against a bus that behaves differently.

**Thread + memory co-location.** A `pinned(node = N)` /
`pinned(l3 = name)` locus binds not just its thread but its
*memory*: its arena is created via
`lotus_arena_create_labeled_on_node`, which flags the arena's
NUMA node, and every chunk that arena grows is `mmap`'d
(page-aligned) and bound to the node with the `mbind` syscall
(`MPOL_BIND`) before first touch — so pages fault in on the
node regardless of which thread touches them first (the locus
struct is instantiated on `main` but runs on its own pinned
thread). Sub-regions inherit the node, so a node-pinned locus's
**method scratch** — the dominant per-invocation allocation —
lands on its node too. `mbind` is invoked as a raw syscall, so
this adds **no libnuma dependency**; the whole feature is
zero-cost for programs that don't opt in (an unbound arena, the
default, takes the ordinary malloc / chunk-pool path
byte-for-byte). Best-effort and Linux-only, exactly like
`pinned(core = N)`: an `mbind` the box can't honor (node absent,
capability denied) falls back to first-touch, and on non-Linux
hosts the arena allocates normally. (Huge-page-backed chunks
and node binding don't currently combine — a node-bound arena
uses regular pages; a follow-up.)

**Single-threaded-method invariant.** A locus's methods may
be invoked only on the OS thread that owns its placement's
pool. Cross-pool method calls and lateral field accesses go
through the bus's existing copy-and-condvar dispatch
machinery, which already crosses thread boundaries safely.
The typechecker walks the static call graph from each
top-level placement entry, propagates pool ownership through
method receivers, and rejects calls that cross pools without
going through the bus. This is the substrate enforcement that
makes M:N safe — without it, multi-pool deployments would
silently race on locus arenas (which are unsynchronized bump
allocators).

The single shared **bus payload arena** is the deliberate
exception to "one owning thread per arena": it is reachable
concurrently from any pool, because some stdlib primitives
allocate their result there directly rather than into a
per-locus arena (e.g. `std::io::tls::recv_bytes`, which always
targets it regardless of the caller's per-thread scratch). That
arena therefore carries an internal lock on its bump — two
pinned loci calling such a primitive at once do not corrupt each
other's allocations. Per-locus arenas keep the lock-free bump;
only this one shared arena pays for the lock.

#### Why no "greedy" class

A natural temptation is to want a third option: "shares a
pool's thread but doesn't yield." That would be a bimodality
violation. Cooperative already guarantees handler-level
atomicity — no preemption within a substrate cell — so the only
thing such a class could add over cooperative is "don't yield
*between* cells either." But that means leaving the shared
pool entirely. The place you go when you leave is your own
thread. That's pinned.

Latency-critical work, or anything that genuinely shouldn't
share with neighbors on its pool, is signaling that it belongs
on its own pool (placed `cooperative(pool = some_quiet_pool)`)
or on its own thread (placed `pinned`). The first option is
new in F.31: pools partition the cooperative substrate so
"shouldn't share with siblings" no longer forces pinned — but
the underlying bimodality (cooperative-pool vs pinned-thread)
holds.

#### `time::sleep` drain semantics

The codegen lowering of `std::time::sleep(d)` slices the request
into intervals of at most **100ms** and ends each slice's
EINTR-retry loop with an inline call to `lotus_bus_queue_drain`
against the program-wide cooperative queue (plus a pinned-mailbox
drain). The total wall-clock sleep is preserved (the slices sum to
`d`); sleeps of 100ms or less take exactly one slice, so the common
case is unchanged. A cooperative subscriber looping

```hale
run() {
    while !self.bail {
        std::time::sleep(100ms);
        // ... loop body sees handlers fired during the sleep
    }
}
```

receives cells posted by other threads — unix-bound reader
threads, pinned publishers via `lotus_bus_local_dispatch`, etc.
— right when each sleep returns, without an explicit `yield;`.
The drain is idempotent, so existing code with `sleep; yield;`
stays correct.

**Which path delivers to whom.** The queue drained *here* is the
program-wide *in-process cooperative* queue, drained on the `main`
thread — so this sleep-loop drain delivers in-process cells to a
cooperative subscriber on the `main` pool. It is **not** the only
delivery path: cross-process topics (`udp://` / `unix://` bound via
`LOTUS_BUS_CONFIG`) are delivered by the transport reader thread,
which dispatches directly into the subscribed handler set and
reaches a cooperative locus on *any* pool — **provided that pool's
thread is free to run the dispatch.** So a non-`main` cooperative
subscriber receives reliably as long as it doesn't monopolize its
pool thread with a blocking call. (Pinned loci are a third path: a
per-locus mailbox drained at each `sleep`/`yield`.) The failure mode
is a cooperative subscriber whose `run()` *blocks* — the dispatch
can't run and its handlers never fire; that blocking-and-subscribing
combination is what the dead-bus-receiver rule rejects (see
"Type-check rules" in `spec/semantics.md`), **not** non-`main`
placement on its own.

**Long sleeps no longer starve main-pool handlers.**
Before the slicing, the drain happened only *after the whole sleep
returned*, so the natural keep-alive idiom

```hale
run() { while true { std::time::sleep(60s); } }   // on the main thread
```

starved every `pool = main` bus handler for 60s at a time: a
main-pool subscriber registers with `coop_pool == NULL`, so the
wire/reader-thread dispatch path lands its cells on the global
cooperative queue (`g_bus_queue_for_remote`, the same object only
`main` drains). A 60s blocking sleep on `main` left those cells
unserviced for the full 60s — indistinguishable from "the handler
never fires." Slicing keeps the queue serviced ~10×/s during any
sleep, so a main-pool handler that republishes onto a topic an
async-pool subscriber listens to (e.g. a udp-reader-fed producer
forwarding to per-connection writers) now flows promptly regardless
of how long `main` is asleep.

The pinned-mailbox path is unchanged: pinned subscribers wake
on `lotus_mailbox_post`'s condvar broadcast regardless of what
the cooperative scheduler is doing.

#### Owner-executed handlers (2026-07-15, downstream handoff)

Two runtime rules restore the single-threaded-locus invariant
(F.31) *dynamically* — the compiler already enforces it for
direct calls, but two bus paths used to violate it under load
(reproduced as a SIGSEGV in a 10k msg/s ingest bench):

1. **The global cooperative queue is drained only by its owner
   thread** (`main`, recorded at queue creation). The scope-exit
   flush emitted at the end of every fn/method body — and the
   sleep-slice / `yield` drains — call `lotus_bus_queue_drain` on
   whatever thread ran the body; on any thread but the owner the
   call is now a no-op. Previously a pinned publisher's flush
   would execute a main-pool subscriber's handler on the
   publisher's thread, concurrently with `main`'s own drains (the
   locked drain releases the queue mutex before each handler
   invocation) — two threads inside one locus.
2. **Payload deserialization happens on the subscriber's owner
   thread.** The non-flat dispatch paths deserialize each
   published payload into the subscriber's arena (Task-11 arena
   routing); for a target owned by a different thread this write
   used to happen on the *publisher's* thread — an unlocked
   cross-thread write into a foreign arena. Now a cross-thread
   publish enqueues the *wire bytes* plus the deserialize fn in
   the cell, and the owner materializes (deserializes into its
   own arena) at drain, just before invoking the handler.
   Same-thread targets keep the deserialize-at-dispatch fast
   path, so single-pool programs are unchanged.

Consequences: a main-pool subscriber's handlers run **only on
`main`** (at sleep-slice / yield / scope-exit drains — worst-case
~100ms after a cross-thread publish, per the slicing above);
pool subscribers run only on their pool's worker; pinned
subscribers only on their own thread. Flat (pointer-free POD)
payloads are exempt from rule 2 — a verbatim byte copy writes no
arena. Delivery and FIFO order per subscriber are unchanged.

**Item B from the 2026-05-21 friction log** (a cooperative
publisher's `<-` to a pinned subscriber) is **resolved as of
2026-06-01** — the earlier "drains only at dissolve in some
configurations" was a *sequencing* effect, not a lost wakeup.
The mailbox condvar path is correct: a pinned subscriber whose
`run()` **returns** proceeds into the blocking
`lotus_mailbox_drain_one`, and a cooperative publisher's `<-`
wakes it via the `not_empty` broadcast — confirmed by
`coop_to_pinned_mid_program::pinned_returning_run_drains_mailbox`.

The residual constraint is inherent to a single pinned thread:
a pinned subscriber with a **long-running `run()`** that never
returns or yields cannot drain its mailbox *during* `run()` —
the one thread is busy in the loop. Such a `run()` must reach a
cooperative yield point (`time::sleep` / `yield`) for the
TLS-cached `lotus_mailbox_drain_pending` to service the mailbox
(the `coop_to_pinned_mid_program` sleep-loop test), or let
`run()` return so the post-run blocking drain takes over. This
is a property of the bimodal model — "pinned owns its thread,
no yielding between cells" — not a bug; a pinned subscriber that
must receive bus traffic while busy should yield in its loop.
Cross-binary flows (unix / shm_ring with their own reader
threads) land in `g_bus_queue` and benefit from the sleep-folded
drain above.

#### Long-running cooperative children: placement closes Item D

Pre-F.31, declaring a long-running cooperative child as a
`params` field (`metrics_server: std::http::Server = ...`)
serialized parent and child onto the main thread: the child's
`run()` body never returned, so the parent's `run()` never
started. The workaround was the **sibling-in-main pattern** —
hoisting the child to a top-level `main locus` param so it
ran "alongside" the parent rather than "inside" it.

Under F.31 the sibling-in-main pattern IS the canonical
shape, and it composes cleanly because `placement { }` lets
each sibling pick its own pool:

```hale
main locus App {
    params {
        gateway:  Gateway              = Gateway { };
        metrics:  std::http::Server    = std::http::Server { port: 9100 };
    }
    placement {
        gateway:  pinned(core = 1);
        metrics:  cooperative(pool = io);
    }
}
```

Parent's `run()` no longer serializes against `metrics`'s
accept loop — they sit on different OS threads. To shut down
gracefully when one finishes (e.g. a duration-bounded gateway
exits), call `metrics.shutdown()` from the finishing locus's
thread; the C-iii interruptible-accept work makes this the
supported pattern.

The nested-as-child shape (long-running cooperative locus as
a `params` field of a non-`main` locus) remains structurally
serialized — that's a consequence of the nested-instantiation
inheritance rule (children share their parent's pool by
construction). Nested long-running children are an antipattern
under F.31: hoist to main-locus siblings.

**Typecheck enforcement.** The compiler rejects
the antipattern at typecheck. A non-main locus with a non-trivial
`run()` body holding a `params` field of a locus type whose own
`run()` is also non-trivial — including `std::http::Server` and
the other entries on the known-long-running stdlib allowlist —
gets a hard error pointing at the canonical sibling-in-main +
placement fix. The runtime starvation that motivated this rule
is silent (the parent's `run()` simply never executes), so the
type-side rejection is load-bearing: it converts a class of
hard-to-diagnose runtime bugs into a clear compile-time signal.

The rule asks whether a `run()` is **long-running**, which is not
whether it **never returns**, and the two are two definitions, two
columns of each locus's run row (`hale_types::flows::RunRow`, read
through the flow rows):

- **long-running**: the `run()` body has a statement of its own. A
  nested child's `run()` runs to completion before its parent's
  begins, so any body delays the parent whether or not it returns: a
  child whose `run()` is `std::time::sleep(1m)` is long-running and
  draws this error. This rule reads it.
- **never returns**: the `run()` body's last statement is a `while`
  with no exit whose condition never flips false (`while true`,
  `while !self.draining`, or a Bool params flag no member assigns,
  whose default keeps the loop live). Only such a body starves the
  cells a cooperative pool runs after it: the pool-starvation warning
  and the birth-order trap read it.

Every body that never returns is long-running; the converse does not
hold. A stdlib locus, whose body the checker does not see, is both
when it is on the known-long-running allowlist.

#### `where async_io` — green-I/O cooperative pools (F.35)

The sibling-in-main fix puts each long-running child on its own
OS thread, which caps concurrent connections at one-per-pool. To
scale beyond that without spawning a thread per connection, a
placement entry may declare `where async_io`:

```hale
main locus App {
    params {
        listener: std::websocket::Server = std::websocket::Server { ... };
        worker:   WsWorker               = WsWorker { ... };
    }
    placement {
        listener: cooperative(pool = ws_accept)  where async_io;
        worker:   cooperative(pool = ws_workers) where async_io;
    }
}
```

`where async_io` opts the pool into green-I/O scheduling: the
pool's worker drain loop integrates an epoll instance, and
blocking I/O syscalls inside locus methods on this pool park-
and-resume instead of blocking the OS thread. The user code
inside the locus is unchanged — `recv_bytes(stream)` reads the
same line of source whether the pool is `async_io` or not; the
substrate picks the right lowering at the syscall boundary.

**One worker thread per named cooperative pool — a promise, not
an implementation detail** (Crumb batch-3, 2026-07-28). Every
named cooperative pool (`async_io` or classic) has exactly one
OS worker thread for the lifetime of the program. Consequences a
consumer may depend on: every `run()`, bus handler, and parked-
coro resume for loci on the pool executes on that one thread
(coroutines interleave on it; they never migrate), so a
thread-affine C library — a JS engine, SQLite in serialized
mode, a GUI toolkit — placed on a named pool is entered from a
single thread by construction, and `@export` re-entry from
foreign code called on that thread stays on it. Concurrency
within a pool is cooperative only. If a scale-out mechanism ever
lands, it will be a new opt-in placement form; `cooperative(pool
= name)` keeps the single-worker guarantee.

Because parking yields the shared worker, N reader loci that each
park on their own fd — the F.35 one-reader-per-signal shape —
are serviced concurrently by a single pool. Two invariants make
that multiplexing correct: (1) the drain loop starts a queued
`run()` the moment the running coro *parks*, not only when it
completes, so a long-lived reader never starves the readers
queued behind it; and (2) each coro's caller-arena — the
thread-local that decides where its stdlib allocations (recv
result blobs, string builders) land — is snapshotted across the
park and restored on resume, so a coro that resumes after a
sibling ran (and perhaps dissolved) never allocates through an
arena the sibling has since torn down. Every blocking-recv
primitive on the pool honors invariant (1) by parking rather than
blocking `recvfrom`/`read` (the `std::io::udp` recv family joined
the `tcp`/`tls` siblings here in the 2026-07-15 downstream
handoff, and `std::time::sleep` joined them 2026-07-28 (Crumb
batch-5) via a timer-only park — a deadline with no fd, serviced
by the same expiry sweep — so sleeping coros overlap on one
worker and a `sleep` inside a handler never blocks the pool; the
timer half of an event loop now costs what the design always
claimed).

Each bus delivery to a subscriber on an `async_io` pool runs its
handler on a coroutine (a struct + a 64 KiB stack). Rather than
allocate and free that pair per delivery, the pool keeps a bounded
per-worker free-list (cap 64) of completed coro slots and reuses
them — a warm fan-out skips the per-dispatch stack allocation
entirely (2026-07-16). The free-list is worker-thread-local (no
lock) and drained at pool teardown, so a busy async pool retains up
to 64 × 64 KiB (~4 MiB) of coro stacks at steady state. Transparent
to user code — a pure allocation optimization, no behavior change.

**A handler's payload is its own until it returns, across parks**
(GH #781, 2026-09-19). The storage a delivery's payload lives in
belongs to the coroutine that runs the handler, for the whole
invocation: the cell's inline payload bytes are copied into the coro
when it starts, a spilled heap payload and the wire path's
per-delivery subregion transfer to it, and all three are released
once the handler *returns* — however many parks later. So a handler
that parks on a `sleep`, a socket read, or a subprocess drain and
then reads its payload parameter again reads what it read at entry;
concurrent deliveries to the same subscriber never share payload
storage. This did not hold before: the drain dequeued each cell into
a stack local and handed the handler a pointer into it, so a parked
handler returned to a frame the next dequeue had already reused, and
every parked delivery read the LAST published value (the same
mechanism leaked a >512-byte spilled payload per park). Pinned
subscribers were never affected — a mailbox cell outlives the
handler it dispatches, which has no coro to park on.

Typecheck rules:

- All placement entries on the same named cooperative pool must
  agree on `where async_io`. The pool's drain loop is one-or-the-
  other.
- `where async_io` is rejected on `pinned` entries. Pinned loci
  own their own OS thread and have no shared drain loop to park
  on.
- `where async_io` is rejected on pool `main`. The main pool
  runs inline on the binary's primary thread, with no dedicated
  worker to integrate epoll into.

See `spec/decisions.md § F.35` (forthcoming) for the
green-I/O substrate design + perf-axis trade-offs.

(Compare: rich / chunked / recognition projection classes are
genuinely three-way because N≈10, N≈30, and N≈300 are
different cost regimes at scale — memory has more genuine
intermediate ground than time does. Placement, even with M:N
pool partitioning, stays bimodal.)

#### Cross-class bus semantics

- **Cooperative → cooperative, same pool**: handler enqueues
  on the pool's queue; runs at the next substrate cell on
  that pool's drain loop. Sender never blocks.
- **Cooperative → cooperative, different pools**: cross-thread
  post via the destination pool's queue; the destination pool's
  drain thread wakes on the condvar broadcast (same machinery
  as the cooperative→pinned path). Sender never blocks.
- **Any → pinned**: cross-thread post via the pinned locus's
  lock-protected mailbox. Sender never blocks.
- **Pinned → any**: cross-thread post; pinned publisher doesn't
  block waiting for delivery acknowledgement.

#### Implementation status (m26 + m27 + m28a + m28b + m28c; F.31 pending)

m25 wired the annotation through parse / typecheck / codegen.
**m26 ships cooperative semantics; m27 ships pinned threads
(run-only); m28a lifts pinned to full lifecycle; m28b lights up
cross-thread bus mailboxes — pinned loci can subscribe and
publish, with cells routed across threads via per-locus
mailboxes; m28c adds optional CPU-core affinity via
`pthread_setaffinity_np`.**

**F.31 (2026-05-23, Phase 1-5 + Phase 4a shipped):** the
placement-at-main surface and M:N cooperative pools. The
per-locus `: schedule` annotation is removed; the placement
choice moves to a `placement { }` block on `main locus`.
Cooperative subscribers can be partitioned across N pools
(N OS threads, each running its own
`lotus_coop_pool_worker` drain loop against its own per-pool
ring buffer); cross-pool bus dispatch reuses the m28b
condvar+memcpy machinery. A pool's worker thread can take the
same affinity forms as `pinned` — `cooperative(pool = X,
core/cores/node/l3 = …)`, applied right after the worker spawns
(2026-08-12; entries naming one pool must agree, and the main
pool takes none). The single-threaded-method invariant
— a locus's methods may be invoked only from its pool's thread
or via the bus — ships as a typecheck rule (Phase 5).

**Phase 4 v1 limit (handler-only cross-pool delivery).** The
runtime ships pool-aware **bus dispatch** for now: a subscriber
whose enclosing locus is placed on a non-`main` cooperative
pool gets its handler invoked on that pool's worker thread.
Lifecycle methods (`birth` / `run` / `dissolve` / `accept`)
still run on the main thread for cooperative-pool loci —
the codegen does NOT yet relocate them to the pool worker.
For state mutated only inside bus handlers this is enough to
honor the single-threaded-method invariant (the handler is
the only writer of locus state on the pool thread). State
touched by both lifecycle bodies and handlers on a non-main
pool is a Phase 4b concern; the typechecker doesn't flag it
today because lifecycle methods are not user-callable in the
`recv.method()` shape that Phase 5 checks. Plan: Phase 4b
moves lifecycle dispatch onto the pool worker via the same
queue mechanism (post "run_init" / "drain_exit" cells at
instantiation / scope-exit boundaries). The pool-placed field's
`run()` is posted to its worker (below), and its **subtree**
initializes there (§ "m27 + m28a", the pool side): everything
nested under it is built, registered and born on the worker, and
a nested cooperative child's `run()` runs there inline. The
placed field's own `accept` and `birth()` still run on the
instantiating thread (§ "Lifecycle obligations", line 3).

**Runtime pool inheritance for in-method-body instantiation
(2026-05-29).** A locus instantiated *inside a method or
bus-handler body that is itself executing on a pool worker*
inherits that pool **at runtime**. Codegen has no static
placement name for such a locus (placement keys on main-locus
`params` fields only), so the run-posting site and the
subscription-registration site resolve the pool as: the
compile-time-known pool when the locus IS a placed main-locus
field, else the pool whose worker is currently on-CPU
(`lotus_coop_pool_current()`, which reads the per-thread
`g_current_pool_tls`; NULL on the main thread). When the
resolved pool is non-NULL the child's `run()` is posted to it
(its own cell — and, on an `async_io` pool, its own parkable
coro, so a blocking `recv` in the child's `run()` parks that
child's coro rather than the spawning handler's) and its bus
subscriptions are tagged with that pool so dispatch routes to
the right worker. This is **gated on the child being owned
beyond the spawning scope** — `accept`'d, an owned param
field, or returned. A handler-local `let`-bound long-lived
locus is *not* owned (its deferred dissolve fires at the
handler's scope exit), so posting its `run()` would execute
after it's dissolved; those keep the prior behavior
(synchronous `run()`, global-queue subscription). The
canonical N-dynamic-children shape (per-connection handlers,
per-tenant workers) is therefore an `accept`'d child whose
`run()` holds the recv loop — it multiplexes on an `async_io`
pool by construction. See `spec/memory.md` § "Owned
param-field child allocation" for the companion arena rule
that makes the owned child's full subtree outlive the
spawning frame.

**Adapter loci instantiated inline in `bindings { Topic:
AdapterLocus { ... }; }` are NOT main-locus `params` fields**
and so receive no `placement { }` entry. Their `run()` recv-
loops need a dedicated thread by construction; the substrate
places them pinned-equivalent implicitly (same m90 routing +
pthread spawn that pre-F.31 fired via the adapter's
`: schedule pinned` annotation). The annotation goes away but
the behavior is preserved automatically — the bindings-inline
shape unambiguously signals "transport adapter with a recv
loop."

The implementation notes below describe pre-F.31 shapes (m25
through m28c). They remain accurate for the cooperative-only-
on-main-pool case, which is the v1-compatible default when
no `placement { }` block is declared.

**m26 (cooperative):** Each `<-` enqueues `(handler, self,
payload_copy)` cells onto a program-wide FIFO queue
(`@lotus.bus_queue.global`) instead of running handlers
inline. The scheduler drain loop pops cells one at a time
and invokes the handler — handler-atomic per substrate cell,
with cooperative yields BETWEEN cells rather than nested call
frames. Handlers may publish more events; drain continues
until empty.

Drain runs at the start of every `flush_dissolve_frame` —
before any long-lived locus dissolves — so subscribers process
pending cells while still alive. Plus an explicit `yield;`
statement (m26b) drains at user-placed points inside long
internal loops. v0 limitation: cells enqueued DURING a
dissolve are leaked.

The C runtime gained the queue surface:
```
ptr  lotus_bus_queue_create(void)
void lotus_bus_queue_enqueue(ptr q, ptr handler, ptr self, ptr payload)
void lotus_bus_queue_drain(ptr q)
void lotus_bus_queue_destroy(ptr q)
```
m20's "memcpy payload into subscriber's arena" step happens at
ENQUEUE time (publisher's frame).

**m27 + m28a (pinned threads + full lifecycle):** Pinned-class
loci spawn a pthread at instantiation; the locus's params and
its full declared lifecycle (birth → run → drain → dissolve,
each only if declared) execute on that thread, in order.

A pinned locus's subtree initializes on its thread. The
instantiating thread creates the locus's mailbox, if it has
one, and then its thread. The thread makes the mailbox current
and initializes the locus's params: every nested construction,
its subscriptions, its `birth()` and a cooperative child's
inline `run()`, and the params bracket and its settle. The
initialization has a scope of its own: a temporary locus a
default builds (`Helper { }.value()`) is dissolved when the
initialization ends, on the pinned thread, not when the
instantiating function's scope does. So every
lifecycle body of a locus nested under a pinned one runs on the
pinned thread, the domain it is placed in, and a yield inside
one (`std::time::sleep`, an `await`) drains the pinned locus's
mailbox, as a yield on main drains main's queue. The
instantiating thread waits until the params are initialized
(`lotus_pinned_start_await_ready`), so nothing observes the tree
before it is built. While it waits it services its own mailbox
exactly as a yield on it would: on main it drains main's queue,
on a pinned thread that thread's mailbox (a yield on a pool
worker drains neither, and neither does this wait). So a nested
body that waits during the initialization for a reply from a
subscriber on the instantiating thread gets it. Then it finishes the instantiation (the
synthetic fields, the failure route, the locus's own
subscriptions) and releases the thread (`lotus_pinned_start_go`)
into `birth()` and the rest of its lifecycle, and continues. An
override written at the literal (`Worker { started:
pthread_self() }`) is the instantiating code's: it is evaluated
on the instantiating thread, before the pinned thread starts. A
locus an override builds as the field's value is part of the
subtree, and is built on the pinned thread. At scope exit (deferred-
dissolve flush), `pthread_join` blocks until the pinned
thread has finished its lifecycle and returned; the main
thread's only remaining work for a pinned entry is the join
plus the locus's arena destroy wholesale (drain / dissolve are
SKIPPED on the main side — they ran on the pinned thread).

m28a synthesizes a per-locus `__pinned_main_<LocusName>`
function whose signature matches pthread's start-routine
contract directly (`ptr (ptr)`); pthread_create gets that
function pointer with the locus's start block as its argument:
the locus, the start gate, and each value of the instantiating
function that the params' initialization reads (the
instantiating thread runs nothing but its own mailbox's handlers
while they are read, and those reach their own subscribers, not
its frame). No C-side
adapter. The synthesized body makes the mailbox current, runs
the params' initialization (`__pinned_init_<LocusName>`),
reports ready, waits for its release, calls each declared
lifecycle method in sequence, then returns null.

**The pool side.** A root field placed on `cooperative(pool = X)`
initializes its subtree on X's worker the same way, as the first
job of that field. Its params' initialization is lowered into
`__pool_init_<LocusName>`, the start block is the pinned one, and
the instantiating thread posts the initialization to X as one job
(`lotus_pool_start_post`) and waits for it as for a pinned locus
(`lotus_pool_start_await_ready`, which services its own mailbox
the same way). The worker runs every nested construction, its
subscriptions (routed to X), its `birth()` and a cooperative
child's inline `run()`, and the params bracket and its settle, so
a failure a nested child raises during the initialization is
delivered on the worker at settle. The instantiating thread then
finishes the instantiation (the synthetic fields, the field's own
subscriptions and its `birth()`, still on the instantiating
thread) and posts the field's `run()` to X behind the job, as
before. An override written at the literal is evaluated on the
instantiating thread before the post, as for a pinned locus. A
delivery to the subtree during the initialization is X's own: it
runs on the worker, at a yield of the initialization or after it.
A yield inside the initialization (`std::time::sleep`, `yield;`)
drains X's queue on the worker, as a yield on a pinned thread
drains its mailbox; a yield inside a cell that drain runs drains
nothing, and outside an initialization a yield on a pool worker
drains nothing, as before. The job never parks: on an `async_io`
pool it runs on the worker's own stack, not a coroutine, so a
`sleep` or a socket wait inside it blocks the worker, and the
initialization is complete before the worker starts another cell.
The roots of one pool therefore initialize in post order, each
complete before the next is posted. Ordinarily the job
waits behind cells already queued on X: a field placed after
a sibling on the same classic pool whose `run()` never returns is
never initialized, and the instantiation waits for it (the
sibling's `run()` already holds the worker against everything
else on X). No wait may be on itself (§ "Lifecycle
obligations", line 1): when the worker is itself blocked on the
instantiating thread, waiting for the decision on a held failure
that thread gives only at settle, it runs the posted
initialization in place, still on the worker, and waits on. The
constructor offers this pending initialization independently of
queue capacity and tries to enqueue without blocking. Once the
worker claims the pending initialization, the constructor proceeds
to the readiness wait even if the queue remains full; it need not
enqueue a second copy. The pending slot and any queued copy each
hold a reference, and only one path runs the initialization. A
target without threads initializes on the instantiating thread.

**m28b stage 1 (inline-payload queue):** Bus queue cells now
carry an inline `[u8; 512]` payload buffer (with `pthread_mutex_t`
guarding the cell array) instead of a pointer to subscriber-arena
memory. The publisher memcpy's into the cell at enqueue; the
drain (running on the subscriber's thread) memcpy's from the
cell into the subscriber's arena before invoking the handler.
This makes the queue the single point of cross-thread
synchronization: each per-locus arena stays single-threaded
territory, the boundary between layers is where the lock lives.
Per spec/memory.md, "every locus boundary copies the payload"
still holds — just with two memcpy's per cell instead of one.

**m28b stage 2 (cross-thread mailboxes):** Each pinned locus
that declares `bus subscribe`, or nests one that does, allocates
its own `lotus_mailbox_t` at instantiation: a bounded ring buffer with
`pthread_mutex_t` + `pthread_cond_t` + a shutdown flag, sharing
the same inline-payload cell shape as the global queue. The
locus's struct grows a `__mailbox: ptr` field to hold it.

The bus entry table grows from `{subject, self, handler}` to
`{subject, self, handler, mailbox}`. Cooperative subscribers
register with `mailbox = NULL`; pinned subscribers register
with their mailbox pointer. At dispatch time, the `bus_dispatch`
fn loads `entry.mailbox` and branches: null → enqueue on the
global cooperative queue (handler runs on the cooperative
thread); non-null → `lotus_mailbox_post` on the pinned
subscriber's mailbox (handler runs on the pinned thread).

**Subscriptions follow the tower.** A locus nested under a root
field placed off main runs on that field's thread (§ Placement
classes), and so do its bus handlers: its subscriptions register
with its anchor's route, not the global queue. Under a pinned
anchor that is the anchor's mailbox, which the anchor has whenever
anything in its tree subscribes, whether or not it subscribes
itself; under an anchor on `cooperative(pool = X)` it is pool `X`.
The route exists before the subscriptions that use it: the anchor's
mailbox is created before its params are initialized, which is
where every nested instance registers. A pinned anchor's params
initialize on its own thread (m27 + m28a, above), so a nested
instance that waits during its initialization for a delivery
through the mailbox is served by the thread the mailbox belongs
to. An anchor on a pool initializes its params on the pool's
worker (the pool side, above), so a nested instance's bodies and
its handlers run on the one worker, and a delivery during its
initialization runs there at a yield or after it, never beside
it. The route outlives the subscriptions: each
nested instance deregisters in its own `dissolve()`, on the
anchor's thread, before the join below returns, and the join
retires any registration still routed to the mailbox before
destroying it.

The synthesized `__pinned_main_<Locus>` body grows a mailbox
loop between `run()` and `drain()`: it calls
`lotus_mailbox_drain_one`, which blocks on the condvar until
either a cell arrives (returns 1, after dispatching the
handler) or shutdown is signaled with empty queue (returns 0,
breaking the loop). Pending cells flush before the loop
returns 0 even after shutdown — the order check is "queue
empty AND shutdown."

Coordinated shutdown: at the deferred-dissolve flush, the main
thread calls `lotus_mailbox_shutdown` on the pinned locus's
mailbox (sets the flag + broadcasts the condvar), then
`pthread_join`. The pinned thread observes the empty+shutdown
condition, breaks its loop, runs `drain()` and `dissolve()`,
and exits — main joins, then destroys the mailbox and the
arena.

Per The Design / lotus, this is the canonical "any → pinned"
bus path: publisher and subscriber sit in different layers of
the lotus, the substrate cost lives at the layer boundary
(the mailbox lock + the inline payload's two memcpy's), and
each arena stays single-threaded territory. Bimodality holds.

Still gated: pinned loci cannot declare `accept()` (children
of pinned would need cross-thread cascade-dissolve
coordination, which is meaningful new infrastructure beyond
m28b's mailbox post-and-continue) or a closure whose epoch is
`birth` or `dissolve` — dissolve being the default with no
`epoch` clause — (cross-thread routing inside the cascade). Tick, duration, explicit and inline closures fire on
the pinned thread and are supported. The typechecker refuses
the two gated shapes at the placement entry or adapter binding
(adapters also run on their own pinned thread; rule 6); codegen
keeps a backstop for builds that skip the checker.

**m28c (CPU-core affinity):** When a pinned locus declares
`: schedule pinned(core = N)`, codegen emits a call to
`lotus_set_core_affinity(tid, N)` immediately after
`pthread_create` succeeds. The C-side helper wraps
`pthread_setaffinity_np` (with a `cpu_set_t` zeroed and bit N
set) so codegen doesn't have to know the cpu_set_t layout
(opaque + size-variable across glibc versions). Best-effort
semantics: if the requested core is unavailable (e.g., CI box
with fewer cores than the source declares) or the syscall is
denied, the runtime silently falls back to ordinary OS
scheduling rather than refusing to run the binary. The
underlying bimodality is unchanged — `pinned(core = N)` is a
refinement WITHIN the pinned mode, not a third position.

**Topology Phase 1a (cpuset affinity):**
`pinned(cores = A..B)` / `pinned(cores = A..=B)` /
`pinned(cores = {a, b, c})` generalize the single core to a
core **set**: the thread's affinity mask is the whole set and
the OS schedules it freely within it — a range carves out an
isolation domain rather than picking one CPU. Bounds are
integer literals (placement is a closed-world deployment
seam), so the compiler expands the spec statically — sorted,
deduplicated — into a constant array and emits one call to
`lotus_set_core_affinity_set(tid, cores, count)` after
`pthread_create`. Range inclusivity follows expression
ranges: `..` excludes the upper bound, `..=` includes it. The
typechecker rejects a spec that selects no cores (`4..4`,
`8..=4`) and a duplicated set element; whether the cores
exist on the deploy box stays best-effort at runtime — the
C helper skips out-of-range indices and applies the mask only
if at least one valid core remains. CPU affinity is
Linux-only: on other hosts (macOS) both helpers are compiled
as no-ops and the loci run unpinned. `pinned(core = N)`
continues to route through the single-core helper unchanged.

Linker dependency: clang invocation now passes `-lpthread`
unconditionally; small fixed cost in the resulting binary
(libpthread is on every modern Linux).

### Bus message router

The runtime's bus is **transport-agnostic**. From the
framework's perspective, a transport is the bus kernel projected
through a parameter regime: NATS and UDP multicast and TCP and
Unix sockets are the same primitive (typed pub-sub) at different
(B, c, σ, φ) values. The runtime knows about subjects, channels,
and modes; specific transports come from stdlib (`std::bus::*`).

- **Subject → handler dispatch.** Declared `bus subscribe
  "..." as fn` declarations are wired by the runtime at
  startup; inbound messages on declared subjects route to the
  declared handler.
- **Outbound publish.** Declared `bus publish "..."` allows
  emit from any handler return; the runtime routes to the
  configured transport.
- **Multi-transport dispatch.** A single binary may bind
  different channels to different transports (a real-time event
  channel to UDP multicast; a control channel to NATS; a
  test channel to in-memory). The router maintains per-channel
  transport bindings established at deployment time from
  config.
- **Transport adaptation interface.** Cross-host transports
  (NATS, MQTT, TCP-with-framing, custom) plug in via
  `interface std::bus::Adapter` — a contract for user-supplied
  loci that ship messages on whatever protocol they choose. The
  contract definition lives in `runtime/stdlib/bus.hl`; concrete
  adapter implementations live in user code or downstream
  packages, NOT in std. The substrate-provided `unix(...)`
  transport is in the runtime itself (substrate-guaranteed
  atomic delivery via SOCK_SEQPACKET) and doesn't go through
  the Adapter interface.

**v1.x source surface.** Subjects are now declared as typed
top-level `topic Foo { payload: T; subject: "..."; }` decls
(with optional `: Parent` for hierarchical wire subjects);
deployment-time bindings live in the `main` locus's
`bindings { Topic: <transport>; }` block — the program's OWN entry
main: a `main locus` that arrives through `import` (a test seed
importing the application it tests) keeps its `bindings { }` inert,
so a test binary never binds, or unlinks, the sockets of the running
application (GH #529 D7). It is also not counted against the
one-`main`-locus rule, so a program that imports such a seed may
declare a `main locus` of its own (GH #1059). Two transport shapes
ship: substrate-provided `unix("/path", role: ...)` and
user-supplied adapter loci named directly on the right-hand
side (any locus satisfying `__StdBusAdapter` —
`fn send(subject: String, bytes: Bytes)` — qualifies).
In-memory delivery is absence-of-entry. Adapter bindings let
protocol-layer transports (NATS, MQTT, TCP-with-framing,
custom JSON-over-WebSocket) live in user code without the
language having to enumerate protocol variants. See
`spec/semantics.md` "Topic declarations → Phase 2" for the
full surface, including the closed-world topology
optimization that elides bus dispatch for unambiguous
intra-locus and single-hop parent→child tower patterns when no
binding is declared.

**Binding realization failure (GH #227) + transports-as-loci
(GH #233 steps 1–2, 2026-07-22).** Source-level
`bindings { T: unix(...) }` entries are sugar: codegen
instantiates a stdlib transport locus
(`__StdBusUnixListenTransport` / `__StdBusUnixConnectTransport`,
`runtime/stdlib/bus.hl`) as a cooperative child at the main
prelude — converging with the adapter path. The locus is the
control plane; the data plane stays in C:

- `birth()` calls `lotus_bus_transport_realize(subject, path,
  role)` synchronously on the boot path (unix `socket + bind +
  listen` via `lotus_transport_listener_create` for listen,
  connect-with-retry for connect). Realization failure routes
  into `lotus_bus_binding_fail(subject, url)` — the
  structural-failure shape (stderr diagnostic + `exit(1)`), the
  same seat as `lotus_root_panic`. The listen locus's birth then
  spawns the serve thread (`lotus_bus_transport_spawn_server`).
  The transport loci are deliberately NOT pinned: a pinned locus
  runs birth on its spawned thread, which would make realization
  asynchronous; the serve thread belongs to the C data plane.
- The serve loop (`lotus_bus_unix_serve`) serves MANY peers
  (2026-09-09, DNA F.13): it polls the listener beside every
  accepted connection (up to 64), admits connections as they
  arrive, keeps a framed seq space per peer, closes only the peer
  that hangs up, and — once the listener is shut at exit — drains
  every connected peer to EOF before leaving. Before this it
  accepted one peer,
  dispatches its messages, and on peer EOF **re-arms** — closes
  the dead connection and loops back into `accept()` for the
  next peer (GH #233 step 2; peer EOF is not connection loss,
  and a rolling restart of the connect-side binary just works).
- `dissolve()` calls `lotus_bus_transport_reclaim`: sets the
  serve loop's `closing` flag, shuts down both fds to unblock a
  parked accept/recv, joins the serve thread, destroys the
  transport. The husk entry stays in the remote table for
  `lotus_bus_remote_destroy_all` to free uniformly.
- **When that dissolve runs (GH #893).** The transport's struct
  is allocated for the program's lifetime (the payload arena, so
  it outlives `fn main`'s subregion) but it is OWNED by `fn
  main`: the prelude registers it on main's deferred-dissolve
  frame, before any user statement, so it is that frame's first
  entry and the reverse-order flush tears it down LAST — after
  every user locus has dissolved (a `dissolve()`-body publish
  still reaches the wire), after the main-exit ingress quiesce
  and cooperative-pool join the exit path sequences ahead of the
  flush, and before the global arena destroy and
  `lotus_bus_queue_destroy`. `lotus_bus_remote_destroy_all` then
  finds the entry already reclaimed — transport NULL, serve
  thread joined — and frees only the husk. Program-lifetime
  ALLOCATION and no OWNER are separate questions; a transport
  answers the first without the second.

Publish fanout is untouched — realized entries land in the same
`g_bus_remote_entries` table the fanout walks.

**Connection-loss supervision (GH #233 steps 3–4).** Publish
fanout marks a locus-served connect entry `lost` on send
failure (skipping it thereafter — down-window publishes drop,
never falsely succeed) and pushes it onto a mutex'd pending
list (`lotus_bus_transport_mark_lost`). The top of
`lotus_bus_queue_drain` — owner thread, the only place failure
handlers may run — drains the list
(`lotus_bus_drain_lost_transports`): each handle goes to the
codegen-registered dispatcher (`lotus_bus_set_loss_handler`,
emitted only when main declares the matching `on_failure`) or
straight to the structural exit
(`lotus_bus_transport_lost_fallback`). The dispatcher invokes
`main.on_failure(main_self, transport_self, link_lost_violation)`
and, when the handler bumped `__restart_count` via
`restart (t)`, calls `lotus_bus_transport_reconnect` (re-runs
connect-with-retry against the entry's stored path; success
clears `lost`). Codegen support: `lotus.main.self` global
(stored at main-locus instantiation),
`lotus_bus_transport_bind_self` (entry ↔ locus self, emitted
after each connect instantiation).

Env-configured routes (`LOTUS_BUS_CONFIG`) have no source-level
declaration to hang a locus on and keep the direct C path:
`lotus_bus_register_remote(subject, url, role)` returns an i32
status (0 ok / -1 unrealizable — scheme/addr validation,
socket/bind/listen, connect-retry timeout, thread spawn), with
listener-side work done synchronously at registration (udp:
parse + bind + multicast join via
`lotus_bus_udp_listener_setup`). On failure the entry is popped
(no dead slot for fanout to silently skip) and
`lotus_bus_load_config` routes into `lotus_bus_binding_fail`.
Config-route unix listeners share `lotus_bus_unix_serve`, so
they re-arm identically. Normative contract: spec/semantics.md,
"The publish contract".

**Adapter dispatch.** At codegen, an adapter binding
instantiates the adapter locus into the program-lifetime
payload arena (same m90 routing the `-> LocusRef(L)` return
path uses), resolves the locus's `send` method's fn pointer,
and registers the (self, send_fn) pair with the runtime via
`lotus_bus_register_remote_adapter`. The runtime stores both
in `lotus_bus_remote_entry_t`'s adapter slot. Outbound fanout
packages the wire bytes as an Hale-level `Bytes` value
(built via `lotus_bytes_from_buf` against the lazy global
payload arena) and indirect-calls
`send_fn(self, subject, bytes)`. No vtable lookup is needed
at the runtime layer — codegen resolved the method at
binding-emit time.

**Adapter inbound (m105).** Adapters receiving wire-bytes
from their protocol layer call `std::bus::__local_dispatch(
subject, bytes)`; the primitive backs onto
`lotus_bus_dispatch_wire`, which looks up the subject's
registered deserialize fn in `g_bus_entries` (same table the
publish-side fanout consults), reconstructs the struct-layout
bytes, and fans into local subscribers via
`lotus_bus_local_dispatch`. Symmetric to the unix reader-
thread path; out-of-band recv loops (any code holding wire
bytes for a bound subject) can use this too.

**SPSC observation ring (GH #244).** A single-producer
fixed-slot ring over CALLER-PROVIDED memory, exposed as lotus
primitives (`lotus_spsc_init` / `_emit` / `_note_drop` /
`_set_tag_b` / `_read`) and the raw all-Int Hale surface
`std::ring::__spsc_*`. Built for observation planes (the iris
observer attaches to these rings inside an shm segment,
read-only, from a foreign process), so the layout is a STABLE
documented contract:

- Descriptor, 64 B, 64-aligned, caller-placed: `u64 data_off`
  (slot array offset from the segment base), `u64 head`
  (producer cursor — monotonic, never wraps, published with a
  release store; slot index is `head & (ring_slots-1)`,
  ring_slots a power of two), `u64 dropped` (producer-side drop
  accounting; the ring itself never blocks — overwrite-oldest by
  construction), `u32 tag_a`, `u32 tag_b` (user tags; tag_b has
  a relaxed-store setter for gauge use), 32 B reserved (zero).
- Slots: `ring_slots × 16 B`, two u64 words, written plain
  after a RELEASE FENCE and before the head release-store. The
  fence pairs with an acquire fence on the read side (below) —
  the Boehm seqlock recipe. Without the pair, a relaxed slot
  load may observe a future record while the h2 re-read returns
  a stale head, delivering a mixed record past the discard;
  GenMC exhibits it (the stress soak cannot — TSO masks it).
- Read side (any process, no Hale runtime required): snapshot
  h1 (acquire) → copy `[cursor, min(h1, cursor+max))` →
  ACQUIRE FENCE → re-read h2 (acquire) → discard records with
  index `<= h2 - ring_slots`
  and count them as overruns. The `<=` is load-bearing: the
  producer's in-flight (unpublished) write for record `h` is
  already clobbering slot index `h - ring_slots`, so the live
  window given a published head `h` is `(h - ring_slots, h]`.
  Verified concurrently in `tests/spsc_driver.c` and modeled in
  `verification/spsc_ring_model.c` (which also refutes the
  strict-`<` boundary).

Consumer cursors and overrun counters live OUTSIDE the shared
segment (caller-owned); external readers never write the ring.
This ring is the convergence target for iris's observation
protocol (its PROTOCOL.md pre-freeze sketch is this layout with
`tag_a`/`tag_b` as `sched_id`/`current_locus`).

**SHM ring substrate (Form K5).** POSIX shared-
memory ring backing the zero-copy bus route. Six C primitives in
`runtime/lotus_shm_ring.c`, linked unconditionally so user
programs that bind a topic to a zero_copy route resolve cleanly:

- `lotus_shm_ring_open(name, slot_size, slot_count)` — open or
  attach (creates if it doesn't exist; validates header on
  attach). Returns a per-process handle.
- `lotus_shm_ring_claim(ring)` — publisher gets a pointer to
  the next slot. v1 is single-producer; never fails.
- `lotus_shm_ring_commit(ring)` — release-orders the slot
  writes before atomic-incrementing the published seqno.
- `lotus_shm_ring_published(ring)` — acquire-load the seqno
  for subscriber-side polling.
- `lotus_shm_ring_read_slot(ring, seqno)` — subscriber gets a
  pointer to the slot for `seqno`. Returns NULL if not yet
  committed OR wrapped past (slow consumer).
- `lotus_shm_ring_close(ring)` — unmap + close fd; unlink the
  SHM object if this handle created it.

Layout in SHM: a 64-byte cache-aligned header
(`lotus_shm_ring_header_t` — magic, slot_size, slot_count,
atomic seqno) followed by N slots of `slot_size` bytes each.
Header magic + sizes are validated on attach to catch ABI
mismatches across binaries pinned to the same ring name.

**Foreign-layout consumer (Proposal B).** Two more
primitives read an *externally*-defined ring described by a
`ring_layout` (see semantics.md § "Foreign rings"), rather than
the native LRSRNG1 shape:

- `lotus_shm_ring_open_layout(name, desc)` — attach an existing
  foreign segment READ-ONLY (never creates), `fstat` for the map
  length, validate `magic`/`version`, read `buffer_size` for the
  data-region capacity. `desc` is a `lotus_shm_layout_t` built
  from a flat 16-entry uint64 descriptor codegen emits.
- `lotus_bus_register_subscriber_shm_ring_layout(subject, name,
  desc_words, self, handler)` — open via the above and spawn a
  `byte_records` reader thread that walks `[len_prefix][payload]`
  records (modular over `capacity`, skipping `pad_sentinel`
  tail-pads, advancing by `align_up`). Shares the native
  subscriber registry + `atexit` teardown; a layout subscriber is
  marked `is_layout` and torn down via `lotus_shm_ring_close_layout`.

The producer side (Proposal B M3a) mirrors these:

- `lotus_shm_ring_create_layout(name, desc, capacity)` — CREATE +
  own the segment (size `data_at + capacity`, write the
  magic/`version`/`buffer_size` header, zero the cursor). Attaches
  read-write without re-init if it already exists.
- `lotus_bus_register_shm_ring_layout(subject, name, desc_words,
  capacity)` — create via the above + register a producer (one per
  subject). `lotus_bus_publish_shm_ring_layout(subject, value,
  size)` frames one `byte_records` record (the inverse of the
  reader: reserve `align_up`, `pad_sentinel` at the wrap, write the
  length prefix + payload, release-store the cursor). The producer
  rings are closed + `shm_unlink`'d at `atexit`.

Field reads/writes are host-native endianness (the foreign producer and Hale are
both little-endian x86-64). v1 is `byte_records` only; the `slots`
framing kind and a zero-copy writable producer view are post-v1.

v1 scope: single-producer, multi-consumer; in-memory delivery is
in scope (POSIX shm_open works intra-machine cross-process).
Multi-producer (CAS-based claim), back-pressure / timeout
modes, and named-ring registry are post-v1. The Hale-side
`fallible(ClaimError)` signature is reserved for those; v1's
`claim()` never actually fails.

**Lifecycle / cleanup.** Both
`lotus_bus_register_shm_ring` (publisher) and
`lotus_bus_register_subscriber_shm_ring` (subscriber) register
a single `atexit` hook on first call. The hook:

1. Signals every subscriber reader thread to stop via an
   atomic `should_stop` flag.
2. `pthread_join`s each reader thread, ensuring no in-flight
   handler is interrupted.
3. Frees the subscriber state allocated by the registration
   call.
4. `lotus_shm_ring_close`s every ring opened in this process
   — which `shm_unlink`s the ones this process created
   (`owns_unlink=1`), keeping `/dev/shm/` clean across
   restarts.

The atexit hook runs on a clean process exit (return from
main, `exit(3)`), which includes a SIGINT / SIGTERM that drains
to its end (GH #1039). Termination that does not return from main
— SIGKILL, a signal the program does not observe, a drain that
outlives its grace, `_exit` — bypasses atexit and leaves the SHM
namespace entry behind until reboot or manual `shm_unlink`.

**Constraint: subscriber handlers must not call `exit()`.** The
handler runs on the reader thread; calling `exit()` from inside
the handler invokes atexit on the reader thread, which then
attempts to `pthread_join` itself (undefined behavior). Use
`_exit()` if a handler needs to terminate the process
immediately.

Codegen surface for K5 lands in K4 (route-selection +
slot-locus synthesis) and K6 (subscriber view + epoch guard).
K5 ships the substrate; user code can't reach these symbols
directly without going through the slot-locus surface a
zero_copy binding produces.

### Closure-test infrastructure

- **Default epoch is `dissolve`.** Closures with no `epoch`
  clause evaluate at the locus's dissolution. Other epochs:
  `epoch tick`, `epoch duration(...)`, `epoch birth`,
  `epoch explicit` — runtime-managed per declaration.
- **Accumulator engine.** For each `closure name { ... }`, the
  runtime maintains accumulators for the left and right sides
  of `~~`, scoped per epoch (when accumulation is needed; not
  needed for one-shot self-referential closures like
  `self.x ~~ self.y within 0`).
- **Band checking + reporting.** At each epoch boundary, the
  runtime evaluates left and right expressions, checks the
  band, and emits a typed `ClosureReport` event the application
  can subscribe to via bus.
- **Collapse vs. explosion.** A closure-pass at any epoch is
  silent. A closure-fail flips an "exploded" flag on the locus.
  At the failing epoch (held while the parent's params are
  open; § "Lifecycle obligations", line 9), the parent's
  `on_failure(self, ClosureViolation { ... })` is invoked with
  a typed event carrying closure name, epoch, left/right
  values, tolerance, diff. Distinct from hard substrate
  failures (OOM, divide-by-zero, null-deref from
  miscompilation) — those terminate the process directly
  without the ClosureViolation routing path. See
  decisions §F.9.
- **Recovery-event interaction.** `persists_through(...)` and
  `resets_on(...)` clauses are honored at recovery time; the
  accumulator is preserved or zeroed per declaration. The
  exploded flag itself persists across `restart_in_place` and
  `quarantine` (per default; future `clear_violation_on(...)`
  clause may override).

### Perspective infrastructure

- **The global slot.** Each `perspective P` has one program-global
  `{ data, vtable }` slot (`__persp.<P>`). Every holder of
  `perspective(P)` dispatches through it — a load plus a predicted
  indirect call, near-direct cost. A program that declares no
  perspectives pays nothing.
- **Live swap (`reperspective`).** Re-points the slot at a new
  `serves P` impl with a single atomic store, redirecting every
  call site at once. State-preserving across impls of one footprint:
  the `{ data, vtable }` split means `data` — the live, arena-backed
  state — is untouched and only the vtable changes. When the
  perspective declares a bus surface, the swap also re-points its
  subscriptions on that same `data`. (See `spec/semantics.md`
  § Perspectives.)
- **Wire hot-load (aspirational).** Transport-driven redeploy —
  decode a serialized perspective against the compiled-in schema,
  gate on `stable_when`, atomically install with no torn read — is
  specified but not yet shipped. See `spec/semantics.md`
  § "Perspective hot-load".

### Failure handling

- **Failure = `ClosureViolation` propagation.** Any `closure`
  assertion that fails in a locus body produces a
  `ClosureViolation` record routed to the parent's
  `on_failure(child, err)` handler per **F.9**. The parent
  picks one of `restart` / `restart_in_place` / `quarantine` /
  `reorganize` / `bubble`, or absorbs (returns without calling
  any). A violation that bubbles past the root exits the
  process non-zero with the violation report on stderr.
- **No source-level panic / exceptions.** Hale has no
  `panic(msg)`, `assert(cond)`, `throw` / `catch`, or
  implicitly-propagating exception machinery. Failure is
  either structural (closure violation; parent-policy
  recovery, Erlang let-it-crash with the parent locus as the
  supervisor) or value-level via `fallible(E)` (v1.x-FORM-1
  addressing protocol; every fallible call MUST be addressed
  by an `or` clause at the immediate caller). The two
  channels are orthogonal at every frame except the implicit
  main locus's root, where a value error escaping past every
  enclosing `fallible` frame triggers `lotus_root_panic` —
  the runtime's only value-error escape valve. See
  `spec/semantics.md` § "Process exit".
- **Decision L0-1 (F.40 phase 3, 2026-10-01): handlers run only
  on the queue owner's thread.** A child's failure is delivered
  to its owner's `on_failure` on the owner's execution domain,
  never on a thread outside the owner's domain. When owner and
  child share a thread, the child's thread is the owner's domain,
  and the handler is called inline. The owner's execution
  domain is the thread that drains the owner's queue: `main` for
  a locus on the main pool, the pool's worker for a locus placed
  on a cooperative pool, and the locus's own thread for a pinned
  one (§ "Owner-executed handlers").
  - When the child fails on that same thread, the handler is
    called in place (§ Scheduler, "Failure-traversal": a stack
    walk).
  - When the child fails on another thread, the failure travels
    to the owner's queue as a typed cell (§ Scheduler: "the
    failure is delivered as a typed bus message to the parent's
    scheduler, which dispatches to `on_failure`"). This is the
    path a cross-thread publish takes, and the cross-pool accept
    handoff takes it too.
  - The failing child and its copied violation stay alive until
    the handler has returned. The child waits for the handler's
    decision before it restarts, carries on or is reclaimed,
    exactly as a held failure is waited for today
    (`lotus_failure_await`).
  - **Delivery makes progress while the owner joins its
    children.** The owner must remain able to complete
    outstanding failure decisions until its dependent children
    have quiesced, and shutdown must never silently discard a
    failure cell whose child is awaiting it. A child's failure
    may come after the owner's last drain: from a pinned child
    the owner is joining, or from a pool child while the pool
    workers are joined. The owner then has to be able to run that
    cell even though it is inside the join. Otherwise the owner
    waits for the child's thread while the child waits for the
    owner's decision. This decision adopts the requirement; F.40
    phase 3's L1 (the obligation) and L5 (the implementation)
    supply the mechanism.
  - **Restart during drain is converted to cancellation.** A
    restart the handler asks for after the owner has entered
    teardown, or while the process drains, is not performed. The
    child ends as if the handler had returned without asking: it
    reaches its ordinary run end, where it is reclaimed if it is
    a flow or terminated child and otherwise kept for its owner's
    cascade. For the process drain this is shipped
    (`emit_restart_requested`,
    `crates/hale-codegen/src/locus/restart.rs`, refuses a restart
    while `lotus_process_draining_flag` is up). For an owner's
    teardown nothing checks it yet.
  - A failure held while the owner's params are open is still
    delivered when they settle (`spec/semantics.md` §
    "on_failure(c, err)"). One subcase is open: an owner placed
    on a cooperative pool. This decision names the pool's worker
    as that owner's domain. `spec/semantics.md` names "the thread
    settling the parent", which today is the instantiating
    thread. The subcase awaits the construction-time decision
    (inventory Decisions line 1), and this bullet and
    `spec/semantics.md` are brought into agreement when it lands.
  - Transport loss already follows this rule. Its dispatcher runs
    from the top of `lotus_bus_queue_drain`, "owner thread, the
    only place failure handlers may run" (§ "Bus message
    router").

  **What the runtime does today, where it differs.**
  `emit_on_failure_call` (`crates/hale-codegen/src/channels/mod.rs`)
  asks `lotus_failure_hold` first. The runtime holds a failure only
  while the parent's params are open; once they have settled, the
  handler is called in place, on whatever thread raised the
  failure. A failure raised off the owner's thread therefore runs
  the owner's handler beside the owner's own code, on a second
  thread inside one locus. Today that happens:
  - on a pinned child's thread, for a `violate` in its `run()` or
    a closure after `run()` returns. The second failure in
    `tests/hale/pinned_restart_test.hl` is this case: `App`'s
    handler writes `self.fired` on the pump's thread while
    `App.run()` reads it on `main`;
  - on a pool worker, for a `violate` in a pool-placed child's
    `run()`;
  - on a subscriber's queue owner, for a closure that fires after
    one of the subscriber's handlers.

  `notes/f40-lifecycle-inventory.md` lists these sites as rows
  C36–C40. No test asserts the thread a handler runs on.

  A fourth site is misrouted, not misplaced. It is a
  dissolve-epoch closure that fails under a flow child's
  run-completion reclaim. The reclaim spine
  (`synthesize_reclaim_fns`, `crates/hale-codegen/src/codegen.rs`)
  lowers the closure with the reclaimed child as `current_self`.
  `resolve_failure_route` therefore asks the child's own type for
  a handler of its own type and normally finds none. The owner's
  handler is not selected at all, and the violation takes the
  bare report-and-exit. The defect is "owner handler not
  selected", not "owner handler called on a foreign thread". It
  becomes the second kind only if the route is fixed without this
  decision: the reclaim then calls the owner's handler in place on
  the reclaiming thread. Inventory row C25 records the site;
  inventory Decisions line 4 chooses the route.

  Teardown pumps no owner queue while it joins. A dissolving
  parent joins a pinned child with a blocking `pthread_join`
  (`emit_deferred_entry_teardown`,
  `crates/hale-codegen/src/codegen.rs`) and drains the bus only
  after the join returns. `lotus_coop_pool_shutdown_all` joins
  each pool worker with no drain in between. Neither join is a
  hazard while the handler is called in place. With delivery
  through the owner's queue, both are the wait cycle that the
  progress requirement above rules out (inventory rows C18 and
  R20).

  **Regression (added with the implementation, F.40 phase 3
  L5).** `lifecycle_flow
  failure_delivery_domain::a_childs_failure_runs_on_its_owners_thread`
  (`crates/hale-codegen/tests/failure_delivery_domain.rs`).
  The owner records `pthread_self` in its own `run()` and again in
  its handler; the test asserts the two are equal for:
  - a pinned child's `violate` after the owner settled;
  - a pool-placed child's `violate` in `run()`;
  - a pool subscriber's tick-epoch closure after a handler;
  - a pinned child failing during the owner's params loop. This
    case is held and delivered at settle, and is the control;
  - a pinned child failing during the params loop of an owner
    placed on a cooperative pool. The construction-time decision
    (inventory Decisions line 1) fixes which thread the case
    expects, and the case lands with that decision.

  In each of these cases the owner is running, not in teardown,
  and the case asserts that the restart the handler asks for
  takes effect.

  A flow child whose dissolve-epoch closure fails under its
  run-completion reclaim has a case of its own. The case asserts:
  - the correct owner receives the violation exactly once;
  - the handler runs on the owner's domain;
  - the child and the closure's captured payload are live until
    the handler returns.

  The case asserts no restart. A dissolve-epoch failure has no
  restart unless a separate decision introduces one.

  Two further cases run under a deadline: a hang fails the test
  rather than stalling the suite.
  - a pinned child raises a failure after the owner has entered
    teardown;
  - a pool child raises a failure after the owner has entered
    teardown.

  A handshake forces the order, never a sleep: the child raises
  only after it observes the owner's teardown. The pinned child
  sees its mailbox shut down. The pool child sees its pool's
  shutdown flag, which a classic-pool accept already returns on.
  Each of the two cases asserts:
  - the handler ran on the owner's domain;
  - the handler completed exactly once;
  - the owner's teardown completed exactly once;
  - the child and the violation's payload were live until the
    handler returned.

  **Restart during drain** has a case of its own. It uses the same
  handshake: a pinned child fails after the owner has entered
  teardown, and the handler asks for a restart. The case asserts
  the conversion to cancellation described above:
  - `birth()` ran once;
  - the child reached its ordinary end;
  - the child was torn down exactly once.

  The test runs under ASan with heap-backed child fields.

### Lifecycle obligations

The lifecycle is a table of obligations (F.40 phase 3, L1): each
action the compiler emits or the runtime performs is owed by a
domain to an instance, is placed in order by the events it waits
for, and ends in one of its named terminal outcomes. The rows are
`hale_types::lifecycle` (`crates/hale-types/src/lifecycle.rs`). An
obligation is keyed by its source site, the declaration built and
the construction template that builds it (P1's instance key); the
runtime numbers the live instance and its incarnation, one per
`birth()`, and the table never does. A restart begins the next
incarnation; a restart asked for and not performed begins none.

This section holds the decisions the lifecycle inventory
(`notes/f40-lifecycle-inventory.md`, Decisions 1–19) asked for,
one paragraph per line, and the requirements adopted beside them. It sits apart from § "Failure handling" because most of the
lines are about order and teardown, not failure; decision L0-1
stays there. Each paragraph states the rule and whether it is
shipped. Where the adopted rule is not yet what the code does, the
paragraph names the inventory row where they differ, and the
fixture under `crates/hale-codegen/tests/fixtures/lifecycle/` that
pins today's outcome; `lifecycle_fixtures.rs` lists it in its
`KNOWN_OPEN` table and fails once the outcome changes, so the
entry has to go with the fix. Each fixture also runs under the
lifecycle trace (§ "The lifecycle trace"), held to the plan the
table's producer (`hale_types::lifecycle::derive`) derives for its
program, on its line's rules (three of line 19's, whose shapes the
producer does not derive yet, to a hand-written plan).
The six started-run retention fixtures use the derived plan, including
the edge from each run's end to its reclaim's completion. A posted run
may overlap drain and dissolve; an inline run ends before drain. The
producer also keeps statement-position subscribers alive until frame
exit, where their teardown runs, rather than assigning them an eager
teardown at the literal. An owner's Reclaim entry follows its children's
Dissolve completion; its Reclaim completion follows their Reclaim
completion. This permits retained storage while preserving physical
release from children to owner.
A departure the trace shows and the outcome cannot (a missing step,
a step on the wrong thread) is in the same file's
`TRACE_KNOWN_OPEN` table. A line still waiting on a condition
says so and records today's behaviour. The same rules are evidenced
across shapes by the lifecycle matrix
(`crates/hale-codegen/tests/lifecycle_matrix.rs`): a generated
program for each failure phase, tree position and domain, held to
its outcome, the producer's plan for it and AddressSanitizer, with the cells
that fail today, the inventory row each fails at (two, for a cell
that shows two known defects), and the departures each shows, in
its `KNOWN_OPEN` table.

- **Line 1, construction-time delivery.** Construction, readiness
  and failure delivery are one protocol, and its settlement is
  defined by events: the owner's last param is stored, a held
  failure is admitted to delivery, its handler completes, the child
  resumes. A failure raised while the owner's params are open is
  held; at settle the held failures are delivered in the order they
  arrived, before the owner's `birth()`, and each child waits for
  its decision. The child and the copied violation are retained
  until the handler completes. Shipped for an owner whose params
  settle on its own domain (`l01_held_failure_settle.hl`). No
  mechanism carrying this protocol may wait on itself: a held
  failure whose child and owner share a pool's worker
  (`l01_neg_same_pool_held.hl`), and an instantiating thread that
  settles while the worker holding the failed child needs its
  queue for a sibling's run (`l01_neg_it_waits_worker_queue.hl`),
  both complete today; so does an instantiating thread waiting for
  a pool-placed field's initialization on a worker that is itself
  waiting for that thread's decision, since the worker runs the
  initialization in place (§ "m27 + m28a", the pool side).
  **Pending:** an owner placed on a cooperative pool. Decision L0-1
  names the pool's worker as that owner's domain, and
  `spec/semantics.md` § "on_failure(c, err)" names the thread
  settling the parent; for a root field placed on a pool both are
  now the worker, where its params open and settle
  (`l01_pool_owner_settle.hl` pins the delivery, not its thread).
- **Line 2, the tick closures after a posted `run()`.** **Pending:**
  the decisions choose no option. Today the tick and duration
  closures of a locus whose `run()` is posted to a pool run on the
  instantiating thread right after the post, while `run()` is still
  running on the worker (inventory row C12,
  `l02_tick_after_posted_run.hl`). The options stand: run them in
  the posted wrapper after `run()` returns, or drop the post-run
  tick for a posted `run()`.
- **Line 3, where lifecycle methods run on a pool.** **Pending:**
  the decisions choose no option. Today a pool-placed locus's
  `accept` and `birth()` run on the instantiating thread, its
  `run()` on the pool's worker, and its `dissolve()` on the
  teardown thread (`l03_pool_birth_domain.hl`); § "Placement
  classes", Phase 4 v1 limit, says otherwise. Everything nested
  under it is built, registered, born and run inline on the worker
  (§ "m27 + m28a", the pool side).
- **Line 4, the failure route bound at birth, in every spine.**
  Every spine that evaluates a child's closures reads the failure
  route the child bound at its birth, so one instance has one
  route, and the owner and the payload are retained until the
  delivery completes. Not yet shipped in two places. The reclaim
  spine resolves the route with the reclaimed child as the parent
  (inventory row C25, `l04_dissolve_route_reclaim.hl`), and a
  dissolve cascade lowered in `fn main`, where no locus is `self`,
  resolves none (row C31, `l04_dissolve_route_cascade.hl`); in both,
  a dissolve-epoch violation takes the report and the structural
  exit, and the owner's handler is never selected.
- **Line 5, `accept`'s position.** `accept(c)` runs once the
  child's region exists and its params are built, and before its
  subscriptions and its `birth()`. Its result is not read: it
  admits, and cannot reject. An admission interface is separate
  work. Shipped (`l05_accept_position.hl`).

  Lines 5, 6 and 8 are the birth spine, and the compiler emits it
  from the lifecycle plan (F.40 phase 3, L4): an instantiation's
  steps from params settle to the run's start (accept, registration,
  `birth()` with its birth-epoch closures and `birth_check`,
  readiness, the run's start) come in the order the plan's rows and
  edges place them for its declaration, on the instantiating thread
  or, from the birth on, a pinned locus's own thread. A cross-pool
  bubble's child (§ "Lifecycle", interest-based ownership) is the
  same spine on its owner's thread: the create cell's dispatcher
  stitches it to its owner (`accept`, the children's tracker), then
  runs its `birth()`.
- **Line 6, registration before birth, and readiness.** A new
  instance's subscriptions are registered before its `birth()`, so
  `birth()` may publish to them. Shipped. Delivery to the instance
  becomes eligible once its `birth()` (and its `birth_check`) has
  completed: what is published to it before then, its own
  `birth()`'s sends included, waits in the order it was published,
  is never dropped, and is delivered afterwards. Shipped
  (`l06_readiness_main.hl`, `l06_readiness_pool.hl`, and the pinned
  capacity regression in `topic_phase2.rs`). For each subscriber,
  the runtime opens a window
  before the first registration (`lotus_bus_hold_delivery`) and
  parks each cell posted to it there; its readiness step
  (`lotus_bus_ready`, right after the birth) posts them, ahead of
  any published after. A publisher on another thread waits once
  the window holds a queue's worth. The thread running birth does
  not wait on its own window. Nor does the subscriber's own
  consumer, the worker of the pool it is placed on or the pinned
  thread that drains its mailbox: readiness posts the parked cells
  into that consumer's queue, so it parks past the bound, as a
  self-publish never blocks on its own full queue
  (`topic_phase2.rs`, with a cross-domain control that still
  waits). Nor does a consumer that a producer is blocked on,
  since that producer may be a readiness step posting parked
  cells, or wait on one. Either still parks behind every earlier
  cell, so order holds. A pinned subscriber transfers that
  exemption to its thread before birth; its mailbox stays current
  so already-born nested subscribers can continue receiving. The closed-world optimization that turns a send
  into a direct call (`spec/semantics.md` § "Topic declarations →
  Phase 2") preserves the intended receiver while deferring delivery:
  rewritten sends, including those in helpers reached from birth,
  use that receiver's registered route while its window is open.
  Baked broadcast publishes use runtime dispatch while any window
  is open. A helper cannot bypass readiness or broaden a local send
  into a broadcast.
  Not yet true in two cases (inventory rows C51
  and R51). A subscribing locus that only stdlib or imported code
  builds, so that the lifecycle plan holds no template of it, opens
  no window, and what is published to it during its `birth()` is
  delivered then. And a `birth()` that publishes to its own
  subscription without bound parks every cell, since its own thread
  cannot wait for the window to drain: whether such a birth meets a
  bound or a refusal is undecided.
- **Line 7, waits that only teardown ends.** Every teardown spine
  aborts the `or wait`s it would otherwise wait on before it joins
  the workers they block, and an aborted publish is not a success:
  it raises `BusWaitAborted`. Shipped in all five spines: each runs
  the ingress quiesce, then the wait-abort, then the pool join (the
  edges between the process rows the lifecycle plan states for every
  teardown spine, which emission reads), so a pool-placed
  publisher waiting for space on a queue only `main` drains takes the
  raise path and the join returns (`l07_pool_or_wait_teardown.hl`,
  and one fixture per other spine: `l07_or_wait_deferred_main_entry`,
  `_main_fall_through`, `_main_return`, `_main_test_failure`). Where
  a spine owes no pool join (no pool, or a target that rejects every
  pool), `fn main`'s exits keep the wait-abort after their frame's
  pre-drain, so a handler that drain runs may still wait.
- **Line 8, a birth failure's shape.** A failure in `birth()` (a
  birth-epoch closure, `birth_check`) or in `run()` (`violate`, a
  closure) is a `ClosureViolation`, and the failing child is kept
  for its owner's supervision: its region stays, the handler reads
  it, and a restart reuses it. There is no `StructuralFailure`.
  Shipped (`l08_birth_failure_kept.hl`). A pinned locus's
  `birth_check` runs on its own thread, after `birth()` and before
  `run()`; a check's failure its owner is still holding is decided
  before `run()` starts, as for every other locus (inventory row
  C38, shipped by L4's birth spine; the lifecycle matrix's pinned
  birth cells, whose delivery is decision L0-1's in-place one until
  L5).
- **Line 9, when a violation reaches the owner.** At the failing
  epoch, not at dissolve; held while the owner's params are open
  (line 1). Shipped (`l09_delivery_at_epoch.hl`).
- **Line 10, dissolve-epoch closures, then `dissolve()`.** In every
  spine a locus's dissolve-epoch closures run before its
  `dissolve()`, so a violation there is delivered while the user's
  cleanup has not yet run. Shipped (`l10_dissolve_closures_first.hl`;
  today its violation also shows line 4's cascade route).
- **Line 11, a let-bound literal.** `birth()` and `run()` happen at
  the construction site; `drain()` and `dissolve()` happen together
  at the enclosing scope's exit. Shipped (`l11_let_bound_drain.hl`).
- **Line 12, owned fields drain before their parent.** A locus's
  owned locus fields drain before it does, each in its own domain:
  a pinned locus's fields drain on its thread before its own
  `drain()`, and nothing is called unconditionally on a parent's
  pinned thread from outside it. A field the locus was handed and
  does not own acquires no drain obligation. An owner's fields drain
  in their declaration order. Each dissolve completes before the next
  starts; physical release can be deferred while a run holds the
  child. Every child's storage is released before its owner's.
  Shipped, and emitted from the lifecycle plan (F.40 phase 3, L4's
  dissolve cascade): a pinned locus's thread drains its fields
  before its `drain()` (inventory rows C9, C18;
  `l12_pinned_fields_drain.hl` and the lifecycle matrix's pinned
  grandchild cells), and a field typed by an interface or a
  perspective drains with the others, before its owner's drain,
  through the drain half of the teardown its instantiation records
  (inventory row C32; the matrix's interface-field and
  perspective-slot cells). Before, a pinned locus's fields were
  dissolved after the join without a drain, and a contract-typed
  field's whole spine ran after its owner's `dissolve()`.
  An owned field's lifetime is its owner's, so a pinned field's
  thread is joined in its owner's teardown. Not yet true for a root
  returned from the fn that built it (inventory row C52,
  `l12_returned_root_pinned_anchor.hl`): its pinned field is joined
  when that fn returns, while the caller still holds the root, so a
  publish to the field afterwards is dropped. Where such an anchor is
  joined, and where its thread id lives once the building fn's frame
  is gone, is undecided.

  Lines 12 and 19 are an instance's own teardown, the dissolve
  cascade and the reclaim, and the compiler emits both from the
  lifecycle plan (F.40 phase 3, L4). The cascade's steps around an
  owner's fields (the fields' drains, the owner's drain, its
  dissolve-epoch closures and `dissolve()`, the fields' dissolves,
  the reclaim) and the order of the fields come from the plan's rows
  and edges for its declaration; so do the reclaim's (the owned
  children's reclaims, guarded logical entry, cancellation of the
  runs still queued for the instance, waiting for admitted runs and
  deferred descendants, the arena's release, the struct's). Logical
  reclaim hands physical release to a callback that preserves the
  runtime's run and owner holds. The
  emitter refuses an order it cannot emit rather than reorder it.
  The trace build holds each instance's emitted reclaim, on the spine
  the plan holds it on, and a queued run's cancellation, on its
  reclaim's spine, to the plan's order over every fixture, with exact
  departures listed in `SPINE_KNOWN_OPEN`. C29's field replacement
  calls the shared reclaim spine; the producer still describes that
  field's normal cascade and needs an alternate path for replacement.
  The
  process-level teardown steps around them (the pool shutdown and
  join, the wait-abort, the ingress quiesce, the pinned joins) are
  not this order.
- **Line 13, resume.** A child resumed after a held handler goes
  through the same placement and admission as a first run, so a
  pool-placed child's `run()` is posted to its pool; under shutdown
  the resumed run may end in line 19's not-started outcome. Not yet
  shipped (inventory row C43): the resume calls `run()` inline on
  the settling thread (`l13_resume_pool_child.hl`). A locus that
  declares no `run()` owes none on any incarnation, and the trace
  shows none. A `Run` is owed exactly where the compiler calls
  `run()`, by one test both read (inventory row C53): a `run()` with
  a body, a flow's even when empty (its run wrapper reclaims it when
  it returns), and a pinned locus's on its thread whatever the body;
  an empty `run() { }`, written or not, is not called and owes none.
  Shipped, and so is the resume's half (inventory row C48, L4): the
  resume starts the resumed incarnation's `run()` only where the
  plan owes it one, so a locus owed none enters no `Run` when
  resumed, as its first incarnation enters none
  (`l01_neg_same_pool_held.hl`); a flow's run
  end, which is its reclaim, is entered as its first incarnation
  enters it.

  Line 13 and restart during drain (below) are a failure's recovery,
  and the compiler emits it from the lifecycle plan (F.40 phase 3,
  L4): a restart's steps come in the order the plan's rows and edges
  give its declaration. The recovery decision is read once the
  handler has returned; the restart follows it, putting back the
  params as built for a `restart_in_place` and lowering the latch
  the failure raised, and begins the next incarnation, which owes
  again what is owed once per incarnation: `birth()` with its
  birth-epoch closures, then `run()` where the locus declares one.
  Nothing is torn down between them; the instance is the same one.
  The decision is carried out on the spine that reads it: the run
  gate of the instantiation, the posted run's loop, a pinned locus's
  thread, or the resume at settle. The trace build holds the resume
  and the restart to the plan over every fixture, with three
  departures named: a held `run()` failure's resume, which the plan
  does not yet state (C43); a held failure's restart performed by
  the resume at settle where the plan places it on the posted run
  (C42); and the restart under an owner's teardown (below).
- **Line 14, order.** Order follows the steps the compiler emits;
  latches and pending-release records keep teardown from running
  twice (§ "Lifecycle", "Order by construction"). Shipped
  (`l14_reclaim_exactly_once.hl`), and verified: the trace build
  (§ "The lifecycle trace") checks every fixture's run, and every
  runnable example's, against laws that hold whatever the plan (an
  instance is reclaimed once, and only after it was born; no step is
  taken on an instance nothing built; every step entered ends), and
  each adopted line's fixture against its plan.
- **Line 15, signals.** SIGINT and SIGTERM raise the process's
  draining flag, from a watcher thread; nothing on the signal path
  calls a lifecycle method. The `run()`s that read `self.draining`
  return, and the ordinary teardown follows, `drain()` and
  `dissolve()` once each. Shipped (`l15_sigint_flag.hl`).
- **Line 16, a target without threads.** The capability matrix
  selects which of the three process-wide obligations a target owes
  (its `PoolJoin`, `WaitAbort` and `IngressQuiesce` cells), and the
  plan orders the ones selected, the same in every spine. Shipped:
  wasm32 owes no pool join (its premise is that wasm32 rejects every
  pool other than `main`) and no ingress quiesce (it rejects the
  listen transports), and owes the wait-abort in all five spines,
  since the local capacity wait is admitted there and no proof yet
  shows no waiter is live at teardown; the host owes all three
  (`l16_eager_spine_pool_join.hl` for the native half,
  `crates/hale-codegen/tests/target_lifecycle_cells.rs` per spine on
  both targets). The compiler emits them from the plan: each spine's
  process rows, in the plan's order, its head before the frame and,
  at `fn main`'s three exits (one emitter for the fall-through, the
  test failure and `return`), the frame's pre-drain and the rows the
  plan places after it in the frame's flush
  (`crates/hale-codegen/tests/frame_flush_ir.rs`).
- **Line 17, the pinned join set and order.** **Pending,
  conditionally:** the deferred spine's rule (subscription-less
  pinned children first, pinned subscribers in their slots) is the
  baseline for every spine, with the eager spine's change pinned,
  provided GH #253's final-publish guarantees (§ "Lifecycle",
  Teardown delivery contract) hold across the eager, deferred and declaration
  permutations; a conflict is resolved before the line is settled.
  The fixtures add one fact the condition has to meet: a main
  locus's own pinned subscriber field is joined before a
  cooperative sibling field's `dissolve()` publishes to it under
  both spines, because a locus's own pinned entries are moved after
  its frame entry (`l17_pinned_join_eager.hl`,
  `l17_pinned_join_deferred.hl`). The join order is the compiler's,
  not yet the plan's, and the plan and the two spines disagree on one
  shape: for a main locus with a pinned field in a program with
  pools, the plan places the main locus's head (the ingress quiesce,
  the wait-abort, the pool join) before every field's drain, the
  eager spine emits it so, and the deferred spine joins the pinned
  field first (inventory row C14), at the exit of `fn main` or of
  whichever fn built the main locus. The line's settlement decides
  which.
- **Line 18, the pre-drain.** Every teardown spine drains the bus
  before its first step. The pre-drain is a delivery point, not a
  witness that anything has quiesced. Not yet emitted by the eager
  spine (inventory row C13). No program shows the difference
  today: a body that may have published drains at its own exit,
  and a main locus's ingress quiesce ends in a drain
  (`l18_eager_pre_drain.hl` guards the outcome). The trace build
  shows the missing step: the eager spine's run has no `PreDrain`
  before its first teardown step.
- **Line 19, a run's admission and its terminal outcome.** A run
  posted to a pool is attempted by the caller and then admitted or
  rejected; "attempted" is what the caller knows, not a third
  outcome. An admitted run is executed or canceled. Each ends in
  one named outcome:

  | path | terminal outcome |
  |---|---|
  | rejected before admission | not started, with the shutdown reason |
  | admitted, canceled before start | not started, with an acknowledgement |
  | started, returned | completed |
  | started, abandoned by an asynchronous shutdown | canceled after start; the worker's quiescence is witnessed separately, by the pool join |

  Admission is decided on the pool's queue: a post that finds room
  in the ring is admitted, and one that meets a full ring once the
  pool's shutdown is set is rejected, not started for the pool's
  shutdown (`PoolShutdown`). A cell admitted after the worker's last
  empty-and-shutdown check is never dequeued: its child's reclaim
  cancels it (not started, with an acknowledgement), or, when no
  reclaim reached the child first, the pools' teardown frees it, not
  started for that teardown (`PoolTeardown`). Nothing is silent: every
  run that does not start is named on one of these paths, and a run
  post that cannot allocate aborts, as the lifecycle runtime's other
  allocations do. Run admission is separate from the admission of a
  failure decision, which shutdown never refuses while its child
  waits (join progress, below). Whatever the outcome, the child is
  torn down exactly once. A started run abandoned by an async pool's
  shutdown ends canceled after start, named where the worker frees
  its parked coroutine, and the pool join's completion is the
  separate witness of the worker's quiescence; the trace build
  records both (inventory row R20a, `l19_parked_started_coroutine.hl`).
  No release build observes a run's terminal, so the name lives
  there. An admitted run is retained against its child's teardown:
  from admission until the worker starts it or a teardown cancels it,
  the run holds its child. The child's Reclaim begins by canceling
  the runs still queued for it, on whatever pool, each ending not
  started with an acknowledgement, before its arena (or an elided
  arena's struct) is released: every reclaim path makes the call,
  past its latch, so a queued run finds the child whole or finds its
  run canceled, never a released arena. The ticket's lock
  linearizes the cancellation against admission: a reclaim that
  cancels first wins, and a worker that takes the run first converts
  the ticket into the run's hold on the child. A child torn down by
  its owner on the worker its `run()` was posted to is reclaimed
  without that run starting, and so is one whose run waits on another
  pool's worker; each is torn down once. A run that has started holds
  its child until it returns: the child's Reclaim, past the
  cancellation, waits for its started runs before the arena is
  released, so a placed field reassigned while its run is running on
  another pool keeps the old child's memory until that run returns.
  An async pool's parked run that its pool's shutdown abandons
  releases its hold where its coroutine is freed. The wait is not a
  join. The reclaim's drain and dissolve still run beside the run, as
  before, and the hold guards the memory only. A run executing on the
  reclaiming thread is not waited for. Such a run is the one whose
  end reclaims its own child, which happens after `run()` returned.
  Outside a live handler, the wait services the reclaiming thread's
  queue as a yield does. Inside a queued main-thread handler, drain,
  dissolve and queued-run cancellation still happen at the reclaim
  site, but physical release is queued until that handler returns.
  The queue guard stays set throughout the handler body: a free
  function's tail drain cannot start the next handler early. At the
  boundary, a release callback can wait and service replies normally.
  A coroutine on an async pool parks while it
  waits, so its worker runs its other cells and coroutines. Main
  drains its bus queue between short sleeps, and a pinned thread
  drains its mailbox the same way (`l19_started_run_retained.hl` and
  `l19_started_run_retained_async.hl`, both also under
  AddressSanitizer; `l19_started_run_publishes_back.hl` and its
  `_async` twin). The handler variants
  (`l19_handler_replaces_started_run.hl` and its `_async` twin) check
  dissolve-before-replacement, handler completion order, a handler-local
  temporary and a started run reading a nested form's storage. Both
  dispatch modes run these and the run-body controls under ASan.
  Removing the handler boundary restores the original deadlock under
  the fixture's deadline.

  Admission to a child's shared reclaim spine is an atomic claim on
  that instance. It precedes arena reads and logical teardown, and stays
  claimed while physical release is deferred. A started run ending in
  `terminate` or automatic flow reclamation on another worker cannot
  reclaim the retired child again: it returns from the reclaim entry,
  ends its run hold, and lets the thread that owns retirement complete
  release. Failure-handler deferral happens before claiming, so its
  later callback can enter the spine. A constructor resets the claim
  for each new instance, including recycled storage. The handler
  retention regression exercises termination and flow completion under
  ASan on classic and async pools, in both dispatch modes.

  Physical release waits before freeing forms, children trackers,
  recognition pools, arenas or recyclable structs. While an owner's
  run can still read its descendants, their logical teardown collects
  physical-release callbacks under that owner. The owner waits first,
  releases those descendants, then releases its own storage. Retired
  fields retain an explicit owner link after their field slots change.
  Nested handler drains during a release cannot start another release
  callback beneath it; pending requests are coalesced. Unowned
  handler-local stack instances retain synchronous release.
  Before the hold, admission freed the ticket and
  nothing held the child. Such a reclaim released the arena under the
  running run, a heap-use-after-free under AddressSanitizer in both
  dispatch modes. A run whose child is never reclaimed until its
  pool joins is still ordered against the teardown by the join.
  Shipped (F.40 phase 3,
  L5): the cancellation is named in the trace build on the thread
  that reclaims, inside the Reclaim's bracket, and the release build
  runs the same path (`l19_queued_run_canceled.hl`;
  `l19_cross_pool_queued_run_canceled.hl`, a run queued on pool
  `side` for a child its owner replaces on main, also run under
  AddressSanitizer; the lifecycle matrix's pool cells, which assert
  the named terminal). Before it, the run started
  on the freed struct (a heap-use-after-free under AddressSanitizer;
  an accepted child was torn down twice), or, for a subscriber, was
  freed unrun with no terminal named. A run refused at shutdown and a
  cell freed unrun when the pools are torn down are named the same
  way, by the trace build where the runtime ends them, on the release
  build's path (L5; before it both were silent). The refused run was
  never admitted, so its Run's terminal stands alone on the posting
  thread; the freed cell's is bracketed by the teardown's
  cancellation. The post's ABI stays `void`: the caller does not
  learn the outcome, the trace names it. The regressions: a full ring
  (`l19_full_ring.hl`: after the first `App` literal's teardown has
  joined the pools, each later one's placed field posts its run to
  a pool with no worker; the first 64 cells are canceled by their
  children's reclaims, and once they fill the 64-cell ring every
  later post is refused), an empty ring after the last check
  (`l19_empty_ring_last_check.hl`; with the reclaim's cancel removed
  by the trace build's negative control, the pools' teardown names
  the cell instead), self-post overflow (`l19_self_post_overflow.hl`,
  every admitted run completes today), a resumed run
  (`l19_resumed_run_at_shutdown.hl`), and the parked started
  coroutine.
- **Restart during drain.** A restart the handler asks for after
  its owner has entered teardown, or while the process drains, is
  not performed. The recovery decision (what the handler asked
  for) and its execution outcome (performed, or not started for a
  shutdown reason) are recorded separately. The owner's entry into
  teardown and the permission to restart are ordered: a restart
  executed after that entry is refused. The child ends as if the
  handler had returned without asking: it reaches its ordinary run
  end, which keeps a failed child its owner holds (§ "Lifecycle",
  Per-child reclamation; GH #1069), so the cancellation is never an
  unconditional reclaim. Shipped for the process drain
  (`emit_restart_requested` refuses a restart while
  `lotus_process_draining_flag` is up). Not yet shipped for an
  owner's teardown (inventory row C42): a pool-placed child that
  fails while `fn main`'s exit joins the pools is restarted
  (`rd_restart_during_teardown.hl`).
- **Join progress.** An owner keeps completing the outstanding
  failure decisions of its children until the children it waits
  for have quiesced, and a pool worker that supervises children
  on another pool owes the same. Shutdown never silently discards a
  failure cell whose child awaits it. The joins do not acquire a
  general queue drain beside `pthread_join`: whatever the joining
  thread runs while it waits has its own reentrancy and admission
  rules. Today a late failure during a pinned join or during the
  pool join completes (`jp_late_failure_pinned_join.hl`,
  `jp_late_failure_pool_join.hl`), but only because its handler
  runs in place on the child's thread, outside decision L0-1 (the
  trace build shows the delivery completing on the child's pinned
  thread or pool worker, not on `main`, inventory row C36); once
  delivery follows L0-1, the joins, which pump no queue (inventory
  rows C18, R20), are the wait cycle this rule rules out. A late
  failure whose destination queue is full has no regression yet
  (L5).
- **Bubble climbs the tree.** `bubble(err)` passes the failure to
  the grandparent's `on_failure`, and on up until a handler absorbs
  it; only past the root is it the report and the non-zero exit
  (`spec/semantics.md` § "bubble(err)"). Not yet shipped (inventory
  row C47): every `bubble` is lowered as the root's report and exit,
  wherever it is raised, and no grandparent's handler is called.
  F.40 phase 3's L4 corrects it with the failure-delivery spine.

#### The lifecycle trace

A debug aid, not a contract. A build made with
`HALE_LIFECYCLE_TRACE=1` (`BuildOptions::lifecycle_trace`; native
host targets only) compiles the runtime with `LOTUS_LIFECYCLE_TRACE`
and emits each lifecycle step between an entry call and its
completion event, so the program writes one line per obligation
event on stderr:

```text
lc <seq> <Kind> <Point> spine=<Spine> dom=<domain> type=<T> inst=<n> inc=<n>
```

`Kind` is an `ObligationKind`, `Point` is `Entered`, `Completed` or
`Terminal(<terminal>)`, and `Spine` is the spine that emitted the
step (`-` where the site cannot name it: a failure delivered in
place, a restart's re-birth). A `Restart` carries the spine of the
step that decided it (the posted run's loop, `PoolRun`; a pinned
locus's thread, `PinnedMain`; the run gate of an instantiation,
`Instantiation`; the resume at settle, `Settle`), and a `Resume` the
settle's. The domain is the thread the event ran on: `main`,
`pool:<name>` for a cooperative pool's worker, `pinned:<n>` for a
pinned locus's thread, `thread:<n>` for any other. `type`, `inst` and
`inc` name the subject; a process-level obligation (the pool join, a
wait-abort, a pre-drain) prints `-` for all three.

The runtime mints the subject, and this is the one place a runtime
subject comes from: the first event naming a struct gives it the
next instance number and incarnation 0, `Restart Entered` begins
its next incarnation, and `Reclaim Completed` retires the number,
so a struct recycled at the same address is a new instance. The
events: `ParamsSettle` (the bracket's entry, and its settle),
`Accept`, `Birth`, `Run` (and `Run Terminal(CanceledAfterStart)` for
a started run whose parked coroutine a pool worker abandons at
shutdown, with that `Cancellation`; a run that never starts ends
`Terminal(NotStarted(...))`: `Acknowledged` when its child's
`Reclaim` cancels it, with that `Cancellation`, inside the reclaim;
`Shutdown(PoolShutdown)` when the post is refused at shutdown, on
the posting thread; `Shutdown(PoolTeardown)`, with a
`Cancellation`, when the pools' teardown frees its cell), `FailureDelivery` (entered
where the failure is raised, completed when the handler returns,
in place or at settle), `ConstructionDelivery` (a held failure,
from the hold to its handler's return at settle), `Restart`,
`Resume` (a held failure's resume at settle: its decision to restart,
start `run()` or end, each traced as its own step after it), `Drain`,
`Dissolve` (the dissolve-epoch closures and `dissolve()`),
`Reclaim` (past the `__arena` latch: the queued runs' cancellation,
then the arena's release), `PreDrain`,
`WaitAbort`, `PoolJoin` and `PinnedJoin`. Readiness, subscription and
the run's admission have no events yet.

The subject table publishes each initialized identity with a
release/acquire ready flag: a thread finding a claimed slot waits
until its instance number and initial metadata are ready. Later
metadata updates and the sequence counter use relaxed atomics.
On one thread `seq` order is program order, and
if event a happens before event b then `seq(a) < seq(b)`, so a `seq`
order that contradicts a required edge is a real violation, and one
that agrees with it is evidence of an execution, not a proof. The
subject publication and the write itself (one `write(2)` per line)
perturb the execution; a traced run does not establish the ordering
of an untraced run. `hale_types::lifecycle::trace` parses the lines back
into `Event`s and checks them against what a run owes.

`LOTUS_LIFECYCLE_SKIP`, read by a trace build's runtime at start, is
a comma list of steps a negative control removes: a kind's name
skips that step and both its events where it is emitted (and, for
`ConstructionDelivery`, holds no failure, so the handler runs in
place while the params are open; for `Cancellation`, a reclaim
cancels no queued run, which only a fixture whose cells no worker
will dequeue may use); `<Kind>.<Point>` drops that one
line and nothing else. A build without the knob emits nothing of
the trace and its IR is the same. `lifecycle_fixtures.rs`'s
`CONTROLS` use it so that, for every obligation kind a fixture's plan
holds a run to, a run with that step removed or reordered fails the
oracle, and with the violation that says why: a removed pool join
lets a worker's teardown begin before its `run()` has ended, a
removed hold delivers a failure before its owner's settle, an omitted
completion leaves a dependent step entered with its prerequisite
unreached, and the host's own order (join, then abort the waits)
fails line 7's edge.

### Native observation emission (iris P4, 2026-07-27)

With `LOTUS_OBS=1` the runtime publishes an iris-protocol
observation segment (`/hale-obs-<pid>` + registration file per
PROTOCOL v0.4 — the contract's executable form is
`crates/hale-codegen/runtime/obs_protocol.h` in this repo since
2026-09-09 (GH #527 B1); `lotus_obs.c` is compiled with it prepended
and pins its layouts to it, and the Rust test decoder is checked
against its text. 0.2, 2026-08-12
downstream handoff: the header gains `model_hash` at `0x80` — the
topology artifact's `shape_hash`, stamped by the CLI from the model
of the snapshot it typechecks, so a consumer can establish the RUNNING
binary was built from the model it joins against; 0 = unstamped
harness build. And the per-binding backpressure cells reserved
since v0 are now written: `queue_depth` (cell 3, a last-write-wins
gauge of the kernel send-queue occupancy sampled at send time),
`send_block_ns` (cell 4, accumulated transport-send duration), and
`retries` (cell 5, reconnects) — counters-tier, so a consumer
falling behind shows as depth climbing and block time accruing
BEFORE anything drops) and the
**Canonical model entity ids (GH #476 Change 8, proto 0.3).**
Manifest rows name entities and number them in registration order,
which is a fact about the run, not about the program. Each row's
`aux_b` — in the entry layout since v0, written as 0 by every path
until now — carries the canonical `ApplicationModel` entity id for
that row, stamped by the CLI at build time: `MK_TOPIC` rows carry
the `SubjectId` (the manifest fuses publishers by wire subject, so
the address, not the topic decl, is the identity), `MK_LOCUS_TYPE`
rows the `LocusDeclId`, `MK_BINDING` rows the `BindingId`. Values
are `index + 1`; **0 keeps meaning "unstamped"** (a harness build,
or an entity the model does not name — a stdlib subject, say).

Those ids index tables that `model_hash` does NOT fully cover:
`shape_hash` is structural model identity, and the arrangement's
binding rows are not in the topology artifact at all, so two builds
can share a `model_hash` while numbering entities differently. The
header therefore publishes the id table's own identity at `0x88`:
`entity_id_digest`, a digest over the exact `(kind, name, id)` rows
the build stamped. **A consumer recomputes it from the model it
holds and uses the ids only on a match** — a detectable refusal
instead of a silent misattribution. 0 = unstamped. A consumer that
joined by name still can.

The stamp is all-or-nothing: the mapping table grows with the
program (no cap), and if it cannot, the process says so on stderr
and publishes NO ids and NO digest rather than a partial table —
partial canonicalization is indistinguishable from "unstamped" and
would put a consumer back on name matching without telling it.

**proto 0.4 (2026-09-04, GH #525 / iris handoff-14 P31).** No
layout change. Iris's PROTOCOL §4 and its reference emitters had
used `aux_b` since v0 as *binding → owning topic id, scheduler →
cpu index*, so from 0.3 a nonzero `aux_b` meant two things
depending on which emitter wrote the segment. 0.4 retires the v0
meaning: every emitter writes the canonical entity id or 0, the
scheduler cpu index moves to `aux_a`, and the binding → topic
pairing is dropped (the counter table and the binding name carry
it). A consumer still gates on `entity_id_digest != 0`; at minor
≥ 4 it may additionally trust that a nonzero `aux_b` was never
anything else. This was the last open item before the protocol
freezes at v0.

**Identity is stamped before anything can register.** The prelude
publishes `model_hash`, `exec_digest`, and the entity ids ahead of
the bindings prelude and the config loader, because registering a
binding creates the observation segment and segment creation
snapshots the identity fields — a program with a `bindings { }`
block used to publish `model_hash 0` for its whole life. Eager
recording/replay init still runs after binding realization, so a
backend with no replay class refuses at its own seam, naming
itself, instead of being pre-empted by a generic identity
refusal.

The
runtime's own choke points emit records: `BUS_PUBLISH` /
`BUS_DELIVER` at every dispatch flavor (dynamic, static-devirt,
cross-thread wire; deliver is enqueue-time at v0),
`NET_SEND` / `NET_DELIVER` at the transport fanout/reader with
per-binding monotonic seqs (what iris seq-matches into
cross-process edges), `LOCUS_BIRTH` / `LOCUS_DISSOLVE` (both
cooperative and pinned lifecycle paths; dissolve rides the
`emit_locus_arena_destroy` chokepoint) and `RESTART` at
reconnect. Cost contract: env unset = one predictable branch
per probe (`g_obs_state`); enabled-but-unobserved
(`observer_count == 0`) = counters only, no ring writes; ring
emission is SPSC via the #247 primitive with one ring per
emitting thread (TLS assignment; overflow threads count into
`ring_drops_total`). On the observer-count 0→1 rising edge the
live-locus table replays as `LOCUS_BIRTH` records so a
late-attaching observer reconstructs the tree. Knobs:
`LOTUS_OBS_RINGS` (default 8), `LOTUS_OBS_SLOTS` (default
4096), `LOTUS_OBS_WIRE` (default off — see NET seq semantics:
`LOTUS_OBS` alone never alters the wire; the cross-process edge
header rides the wire only when the operator opts the whole fleet
in with `LOTUS_OBS_WIRE=1`). v0 notes: manifest ids are registration-order; publisher
locus attribution on BUS_PUBLISH is unattributed (0); pinned
births render parent=root. `lotus_obs.c` is its own TU; the
arena TU's probes are weak-guarded so helper binaries compiling
`lotus_arena.c` alone  still link.

Field-hardening (iris handoff 2, 2026-07-27, v0.11.13): NET_SEND
now fires on the UDP multicast fanout path (it `continue`s before
the stream-transport probe, so multicast publishers emitted only
deliver-side records — the seq matcher had nothing to pair);
LOCUS_BIRTH is emitted BEFORE a locus's field-default init and
carries real parentage (the parent registers before its children
look it up — post-birth placement rendered every tree flat), and
pinned children register on the spawning thread so a park before
their first probe doesn't hide them; BUS_PUBLISH stamps the
publishing locus from a codegen-set TLS (the C dispatch doesn't
know `self`); topic shape_hash is `fnv(subject, canonical payload
structure)` — field names + coarse type tags in declared order,
never the declaring type's local name — so two binaries sharing a
subject fuse into one manifest row; and each ring re-emits EPOCH
every 1024 records so a high-rate ring that wraps its anchor never
reconstructs timestamps from a stale base (the ~2^64 ns readings).

NET seq semantics (iris handoff 3, v0.11.14): `NET_SEND` and
`NET_DELIVER` carry `w1 = origin:16 | seq:48` where **origin is
the SENDER's per-process identity and seq is the SENDER's
per-binding counter** — the receiver echoes both verbatim from
the wire, so a send and its delivers pair on `(origin, seq)`
across segments (the cross-process edge). Before this, the
receiver stamped its LOCAL delivery counter, which sums across
senders on a multicast subject — the send seq never equalled the
deliver seq and iris rendered zero edges. (Stream transports are
unicast — one sender per connection — so origin 0 + the framed
wire seq already pairs correctly.)

Edge-emission correctness (iris handoff 4, v0.11.15):

- **The record id field IS the topic id** for `NET_SEND` /
  `NET_DELIVER` (as for the bus records) — it is the consumer's
  join key onto the fused topic row. It was hardcoded 0, so no
  NET event could be associated with any topic and edges were
  structurally impossible regardless of `(origin, seq)`. The
  probe now resolves the id from the subject (in hand at every
  emit site); the per-binding counter line still keys off the
  binding id.
- **The published counter is never attribution-gated.** Counters
  are the dormant-mode contract (enabled-but-unobserved = counters
  only), so a genuine publish counts even when its locus can't be
  attributed. handoff-3 gated the counter (and record) behind a
  positive publisher-TLS that a cross-pool or free-fn publish never
  set, zeroing the fleet's published counter. Inbound wire
  re-dispatch is now excluded by NEGATIVE marking instead: the
  reader brackets its re-dispatch (`lotus_obs_begin/end_redispatch`)
  and the publish probe consumes the mark; genuine publishes are the
  unmarked default and always count, with best-effort locus
  attribution (0 when unknown). The keyed dispatch flavors, which
  had no publish/deliver probe at all, now emit both.
- **`LOTUS_OBS` never alters the wire.** The `(origin, seq)` edge
  identity is a wire change a pre-header receiver cannot parse — an
  observed sender would silently drop every datagram at a stale
  peer, partitioning a mixed-version fleet invisibly. So the UDP
  self-describing header (`[u64 magic][u64 origin|seq]`) and the
  framed-transport origin word ride the wire ONLY when the operator
  opts in with `LOTUS_OBS_WIRE=1`; with `LOTUS_OBS` alone the wire
  is byte-for-byte identical to an unobserved run. Cross-process
  edges therefore require `LOTUS_OBS_WIRE=1` fleet-wide; counters
  and local records need only `LOTUS_OBS=1`. The receiver always
  peels a header it recognizes (self-describing), so old→new and
  unobserved peers are unaffected either way.

Flavor completeness + quiet-process replay (iris handoff 5,
v0.11.18):

- **The fully-devirtualized direct dispatch now carries probes.**
  The single-quiet-subscriber same-thread flavor (the
  `static_direct` bucket walk with the handler baked as a direct
  call, and its multi-handler C sibling) emitted NO probes at all —
  a subject on that path never registered its topic, counted
  nothing, and produced no BUS records. Both direct flavors now
  publish once + deliver per matched target like every other
  flavor. Dormant cost is preserved by the `lotus_obs_live` gate:
  codegen checks the flag ONCE per function entry (the flag is
  final before any user publish can run — obs resolves at the
  first probe, always a locus birth), so LLVM hoists the branch
  and an unobserved publish loop is instruction-identical to the
  probe-free lowering. The same per-fn check now also gates the
  publisher-attribution TLS note. **The AST-level rewrite got the
  same sentence on 2026-08-11 (downstream handoff P23):** the
  closed-world intra-locus/tower desugar — one rewrite earlier
  than these codegen flavors, so the earlier fix never reached it
  — now emits `BUS_PUBLISH` (publisher-attributed) +
  `BUS_DELIVER` (subscriber-attributed) at the direct handler
  call, behind the same per-fn gate. Its first probe registers
  the topic's manifest row, so an intra-tree subject with traffic
  is indistinguishable from its bus-dispatched sibling, and
  manifest absence uniformly means "never mentioned at runtime"
  on every flavor.
- **Observer attach no longer needs probe traffic.** The 0→1
  birth replay was driven from inside probes, so a probe-quiet
  process (a main parked in a read loop with pinned raw-fd
  readers; hot paths on the direct flavor) never noticed an
  observer and stayed silent — segment registered, zero records.
  When `LOTUS_OBS=1`, a detached heartbeat thread drives the
  replay check every 250ms under the obs lock (teardown takes the
  same lock around the unmap, so the heartbeat never races it);
  replay latency after attach is bounded at ~250ms even for a
  process that never probes again.

BUS record w1 layout (iris handoff 7, v0.11.21): `BUS_PUBLISH` /
`BUS_DELIVER` pack `w1 = locus:20 (bits 44..63) | seq:44 (low)` —
PROTOCOL §8, executable reference `iris emitter/protocol.h`
(`obs_bus_w1`). The emitter had these fields transposed (locus
low, seq shifted) since handoff-3; attribution was computed
correctly and packed unreadably, and the contract tests stayed
green because they decoded with the emitter's own layout. The
tests now vendor protocol.h's decode, so emitter and consumers
cannot disagree silently.

Adapter ingest completeness (iris handoff 8, v0.11.22): the
Hale-owned-wire ingest (`std::bus::__local_dispatch` →
`lotus_bus_dispatch_wire_inbound`) now carries the full probe trio
the C reader threads have. The wrapper peels the self-describing
obs wire header when the producer emitted one (magic-guarded — a
headerless or non-Hale producer is byte-for-byte unaffected, and
headered bytes reaching a Hale adapter previously failed
deserialization outright), emits `NET_DELIVER` echoing the wire
`(origin, seq)` (or `(0, local per-subject seq)` when headerless —
countable, not pairable), and the plain `dispatch_wire` fanout
gains the per-target `BUS_DELIVER` its keyed sibling received in
v0.11.18. Adapter bindings register lazily in the manifest
(`aux = 2`). Dynamically-ingested planes now form producer→consumer
edges and attribute subscriber loci exactly like statically
configured listens.

Attribution ordering + the adapter inbound path (iris handoff 6,
v0.11.20):

- **`lotus_obs_live` is resolved in a constructor**, before
  `main`. The fn-entry gate hoist's soundness argument requires
  the flag to be final before ANY function entry — previously it
  was set at the first probe (a locus birth inside main's body),
  so a publish lowered into `fn main` itself would snapshot a
  stale dormant flag forever. Constructor resolution makes the
  flag genuinely process-constant. The field-shaped ordering case
  (publishers deep in steady-state loops, observer attaching much
  later) is pinned in `obs_fleet_contract.rs`.
- **`std::bus::__local_dispatch` (the adapter-inbound ingest) is
  redispatch-MARKED.** An adapter locus dispatching received wire
  bytes is a delivery, exactly like the reader-thread path — the
  unmarked entry stamped a spurious `locus=0` BUS_PUBLISH per
  inbound message and inflated the published counter, so a fleet
  ingesting via the std adapter surface read as "everything
  publishes unattributed." Lowers to
  `lotus_bus_dispatch_wire_inbound`, which brackets the dispatch
  with the P15 negative marking. Per-subscriber BUS_DELIVER and
  the topic registration are unchanged.

### Lossless recording mode (GH #296 Phase 1)

`LOTUS_OBS_RECORD=<path>` opts a run into **recording**: the
observation plane stops being a sampler and becomes a flight
recorder. Implies `LOTUS_OBS=1`; the recording counts as an
attached observer, so ring emission is on from the first probe
with nothing mmapping the segment. Opt-in on the same terms as
everything else here — unset, the lowering and the wire are
byte-for-byte the unobserved build (the flag resolves in the same
constructor as `lotus_obs_live` and gates behind it everywhere).

Three deltas from observer mode, all in the service of one rule —
**a recording never drops** (a dropped record is a replay that
diverges silently):

- **Overwrite-oldest becomes block-the-producer.** An in-process
  drain thread appends every ring record to `<path>` and
  publishes per-ring cursors; a producer whose ring is full
  against the cursor waits (50µs naps) instead of clobbering the
  oldest slot. Recording-on hot cost while the ring has room is
  one relaxed load + compare per record. A live observer prefers
  losing records to stalling the program; a recorder makes the
  opposite trade, and the stall is the contract.
- **A thread that cannot get a ring fails the run** (exit 70,
  diagnostic names `LOTUS_OBS_RINGS`) instead of going silently
  dark forever (the `-2` disposition observer mode keeps).
  Recording defaults `LOTUS_OBS_RINGS` to the 64 maximum, so this
  means more than 64 emitting threads. Block or fail — never drop.
- **A drain-side write failure fails the run** (exit 74) rather
  than truncating silently. The file ends with a trailer written
  at teardown after a final full sweep; a reader that does not
  find the trailer holds a truncated recording and must say so.

**Durable recording / crash-prefix recovery (Phase 5).** Named
precisely: this is a durable flight recorder, NOT a write-ahead
log — the application is never gated on the recording reaching
stable storage, so a crash can lose records the application
already executed past. (A load-bearing WAL grade — persist,
fence, then permit delivery — is a possible later mode; nothing
here claims it.) What IS guaranteed: the drain appends whole
frames in stream order and flushes every sweep, so the on-disk
file is an exact prefix of the stream at all times, and the
header identity (`model_hash`, `exec_digest`, policy flags) is
stamped **eagerly** by the drain as soon as the ctor-sequenced
setters have run, not only at finalize. A run that dies without
teardown therefore leaves an artifact that is attributable and
exact up to one torn frame at the tail. Default recording rides
the page cache (survives a process crash, not power loss).
`LOTUS_OBS_RECORD_DURABLE=1` is the power-loss grade: every
flushed sweep synchronizes the file (`fdatasync` on Linux,
`F_FULLFSYNC` on macOS), the parent directory is synced
at file creation (a fresh NAME is not durable until its directory
entry is), the finalize trailer itself is synchronized before
close (a clean exit followed by power loss must not demote a
finalized recording to a truncated one), and the grade is
recorded in the header's policy flags (bit 1).

**Consume records (private recorder namespace).** `BUS_DELIVER`
is enqueue-time by design and lands on the *publisher's* ring —
correct for fanout accounting, structurally unable to say in what
order a consumer actually ran its handlers. Under recording, every
dequeue-driven handler invoke (main-queue drain, coop-pool drain,
pinned mailbox drain, async coro start) stamps a consume record on
the *consuming* thread right before the handler runs, carrying the
delivery identity: the target locus in the record's id field and
the full 64-bit `msg_id` in `w1`. Its ring position is the
per-consumer delivery order — the thing a replay serves back.
Pairing rule: per queue, the k-th consume is the k-th queued
delivery. The synchronous direct-dispatch flavors deliberately emit
no consume record: their handler runs at the `BUS_DELIVER`
position, which *is* the consumption point.

Recorder events (`CONSUMER`/`CONSUME`/`ENQ`/`JOURNAL`) live on
**process-private per-thread rings** in their own event namespace,
never in the public observation segment: iris protocol ekinds
8/9/11/12 mean SUPERV_TRANS/PLACEMENT/BINDING_UP/BINDING_DOWN, and
an observer attaching to a recording process must never decode
recorder bookkeeping as lifecycle transitions. In the recording
file, private-ring entries carry the high bit in their ring field,
so the two namespaces can never be confused.

**Delivery identity (Phase 2).** Every queued delivery carries a
deterministic `pub_id = consumer_id:16 | per-publisher-thread
seq:48` (full width — the review-round guards fail loudly rather
than wrap), re-derivable by a re-executed run without global
coordination. Consumer ids are stable across runs, unlike pthread
ids: main = 1, cooperative pool workers = 16 + registration
index, pinned locus threads = 64 + obs instance id; threads with
no stable identity (ingress readers) get run-unique anonymous
ids, never a shared fallback. Payload bytes are captured once per
publish — **every dispatch flavor, queued or synchronous direct**
— under a **stable subject hash** (manifest topic ids are
registration-order and racing publishers register in either
order). Flags: bit 0 = external wire ingress; bit 1 = raw
in-process struct bytes — an ABI snapshot (String/Bytes fields
are pointers, padding is uninitialized) that consumers must
compare by size only; canonical per-topic recording codecs are
the staged fix. Wire captures are canonical bytes, unflagged.
The direct-dispatch flavors — the devirtualized same-thread call
that replaces the enqueue, in both its baked-inline and helper
forms — record through the same writer, at the same publish site,
in the same framing. They used to capture nothing, on the
reasoning that a closed-world same-thread call carries no
external input and re-execution re-derives its payloads: it does
re-derive them, which is precisely why `--diff` can compare them,
and a fully direct-dispatched workload recorded none to compare.
The capture sits behind the same recording gate as the publish
probe, so an unrecorded run writes nothing and pays nothing.

**Input journal (Phase 3).** Under recording, every user-facing
nondeterministic read is journaled per consumer as ONE unified
stream carrying the read's kind and its **exact encoded
arguments** (length-framed; replay memcmps them — hashes proved
collision-prone), so a changed env name, arg index, rand bound,
or read length is the first *named* divergence, never a silently
substituted value. **Env values are withheld by default**: names,
existence, and lengths are recorded; the value itself requires
`LOTUS_OBS_RECORD_ENV=full`, the artifact header says which
policy applied, and a withheld read replays as a named withheld
divergence. The interposed set:
`std::time::now`, `std::time::monotonic[_ns]` (now a named
primitive, `lotus_time_monotonic_ns` — it used to be inline
`clock_gettime` IR that nothing could interpose),
`std::rand::next_int` (per call — the global RNG's mutex makes
seed-replay unsound across threads), `std::os::getrandom`, and
the `std::env` surface. Internal runtime clocks and config
`getenv`s are deliberately not journaled.

**File format v0.3 (PRE-STABLE).** 96-byte header (magic
`HALEREC0`, version, pid, ring geometry, epoch anchors,
`model_hash`, a 32-byte framed-SHA-256 `exec_digest`, and policy
flags), tagged entries — tag 0: 24-byte ring record; tags 1/2/3:
payload / journal / meta blobs (32-byte header + bytes padded to
8; meta carries the run-stable topic and public-ring identity
maps) — and a 16-byte trailer (`HALEEND0` + entry count). A
recording is *clean* only when the ENTIRE artifact validates:
exact parse to the trailer position and a matching entry count,
enforced by the CLI reader and independently by the runtime
loader (one open + fstat + private mmap, checked arithmetic,
per-kind value shapes) — trailer magic at EOF alone proves
nothing. Identity fields are stamped eagerly by the drain and
re-stamped at finalize (a prelude binding registration can probe —
creating the header — before the stamps run; the two stamps
agree). A trailer-less artifact is a **crash-truncated
recording**: refused by default, but admissible as a PREFIX with
`hale replay --allow-truncated` (`LOTUS_REPLAY_ALLOW_TRUNCATED=1`)
— the loaders stop at the first incomplete frame, report the
parsed extent and dropped tail bytes, and replay that prefix;
execution past the tape's end falls back live and is counted, per
the degrade-never-refuse rule. Under `--diff` a truncated baseline
compares as a prefix — and the runtime verdict agrees with the
comparator: recorded-history-exhausted events (journal reads past
the tape, deliveries past the consume stream, consumers the
recording never saw) count into a separate
`post_prefix_live_fallback` status key, never into the divergence
totals; mismatches BEFORE exhaustion stay divergences either way.
A trailer-FINALIZED artifact keeps the exact full-parse + count
checks: with a finalize present, any truncation is corruption.

**Replay (`hale replay <recording> <program.hl>`).** Admission,
strongest check first: the recording's `exec_digest` — a framed
SHA-256 over the toolchain source hash (compiler + runtime +
stdlib implementation, via the stale-CLI build hash), the CLI
version, build options, and every source file's full path,
length, and contents — must match the recompiled program exactly;
the build options are the ones the compiling command was GIVEN
(`hale run` and `hale replay` take `hale build`'s option flags —
2026-09-20, GH #904; `run` compiled with the defaults and
fingerprinted the defaults), so a recording made under `hale run
--dev` is admitted by `hale replay --dev` and by no default
replay. The build options include the environment's build knobs that
change the binary (see *Build-time and toolchain environment*:
`LOTUS_ASAN`, `LOTUS_TSAN`, `LOTUS_UBSAN`, `LOTUS_LTO`,
`LOTUS_DISABLE_PREFETCH`, `LOTUS_NO_BUS_DEVIRT`,
`LOTUS_NO_OWNERSHIP_BUBBLE`, `HALE_NO_TS_SHIM`, `HALE_TS_SHIM_A`,
`HALE_ZIG`, `HALE_TARGET_GLIBC`, `HALE_TARGET_SYSROOT`,
`LOTUS_OPENSSL_PREFIX`), so a recording made under one is admitted by
no replay built without it; what only narrates or times a build, the
warnings, the linker and the cache directory are not part of it;
`shape_hash` is structural compatibility only, the secondary
check. An unstamped recording is refused without
`--allow-unverified-model`. Stated residue: the digest cannot see
the LLVM/libc toolchain outside the hale binary; a post-link
binary digest is the staged stronger form. **Safe by default:**
replay re-executes real side effects, so the typed effect rows
gate admission and fail CLOSED on `syscall`, `ffi`,
`unclassified` ("may do anything") — refused without an explicit
`--allow-live-effects` (coarse by class granularity —
over-refusal is the safe direction; per-primitive replay classes
are the staged refinement). `bindings` blocks are **no longer**
in that list: under replay the wire is hermetic (below), so a
bound topic cannot reach the live world. `LOTUS_REPLAY=<path>`
then serves journaled reads back per consumer in recorded order;
a read past (or mismatching) the recorded history falls back live
and is counted — **replay degrades, never refuses** (RFC Q6) —
with a divergence summary at exit and a machine-readable verdict
(`LOTUS_REPLAY_STATUS`) the CLI consumes: journal misses, order
holds, unconsumed journal entries, and unconsumed deliveries all
count, and any of them fails `--diff`. Each consumer's queued deliveries are re-consumed
in the RECORDED order (Phase 4): dequeued cells that arrive ahead
of their recorded turn are held per-consumer and released in
order, with a bounded hold (1s) after which the oldest held cell
is released and the miss counted, so a genuinely divergent replay
reports rather than deadlocks. A run its child's reclaim canceled
in the queue (decision line 19) is dropped before the gate compares
it, as the recording dropped it, with no consume; a live run the
gate holds keeps its retention on the child and is admitted only
when it is dispatched. The hold belongs to its consumer thread for
the thread's life: a pool worker or a pinned thread frees it when
it exits at the pools' or the owner's join, and a cell found still
held then ends as one the pools' teardown frees undequeued, never
dropped with the hold.

**Async pools replay (Phase 6).** The nondeterminism of a `where
async_io` pool is its drain's SCHEDULING: which cell starts when,
which parked coro resumes when (epoll readiness order), and when a
timed park expires relative to both. Under recording, every such
decision is stamped on the pool worker's private ring as a step
(`ASYNC_START`/`ASYNC_RESUME`/`ASYNC_EXPIRE`; coros are named by
birth ordinal — the index of the START that created them, stable
across runs because start order is enforced). Under replay the
drain is DRIVEN by that stream instead of the clock: START steps
reuse the phase-4 cell gate (start order is consume order); RESUME
steps wait for the named coro's readiness, parking early-ready
coros aside until their turn; EXPIRE steps resume the named coro
with the timed-out sentinel immediately — the recording already
proves the deadline fired at this point in the sequence, so
replayed sleeps fast-forward rather than re-waiting. A step
unsatisfiable within the hold bound counts an `async-schedule`
divergence and is skipped (degrade, never deadlock) — and a
skipped START **retires** both its birth ordinal and its consume
slot (review round 2: ordinals belong to the recorded START slots,
not to whichever cell happened to start next — without retirement
one missing delivery shifts every later coroutine into an earlier
recorded identity, and the pinned consume expectation makes every
later START mismatch; with it, later steps that name a retired
slot skip immediately and the divergence stays local).

**Coverage, named (review round).** A dry tape hands the pool back
to the live drain — never silently. The artifact carries a
capability bit (header flag 4: this runtime records async
schedules); the states are distinct and reviewable: a pre-phase-6
artifact gets a one-shot coverage note ("recording predates
async-schedule support"); a truncated tape's remaining schedule is
the stated coverage boundary (excluded from the divergence
totals); a FINALIZED capable artifact whose tape runs dry while
the re-execution keeps doing async work counts each such action as
`async_post_tape`, and schedule steps left unconsumed at exit
count as `unconsumed_async_steps` — both machine-readable status
keys and divergence rows. `--diff` compares the per-consumer async
step streams (kind, value) bidirectionally, exactly like the
consume and journal streams — "byte-identical output" alone is
weaker than "the schedule matched". Boundary, stated: the SCHEDULE replays; the DATA of
unjournaled I/O does not — a coro resumed at its recorded turn
re-executes its recv against the live world (syscall-class, gated),
and bindings ingress arrives via the Phase-5 injector.

**Hermetic wire + ingress injection (Phase 5).** Hermeticity is
a **binding-kind capability, not a blanket assumption**: the
native `unix://` and `udp://` transports (and the transport-locus
form) are suppressed and injectable; a backend with **no replay
class fails closed** — `shm_ring` in particular is refused by the
CLI (backend named) and independently at the runtime's shm-open
seam, so a replayed or fed process never creates, opens, or
mutates live shared memory. Adapter loci are user logic and
re-execute as such (governed by their inferred effects).

For the covered backends, under replay a binding is **suppressed
at realization**: the entry exists (a husk, `transport` NULL —
the same shape the reclaim path already leaves), but no socket is
created, nothing binds, nothing connects, and outbound fanout to
bound subjects sends nothing (the publish is still recorded and
compared under `--diff`).

In the listeners' stead, injection runs as an explicit **boot
phase**: codegen emits one `lotus_replay_start_ingress()` call at
the main locus's boot/run boundary — params children born,
bindings realized or suppressed, every boot subscription
registered, `run()` not yet entered — where the runtime snapshots
the subscription registry ON the main thread (injector workers
never read the live registry) and spawns **one worker per
recorded ingress source** (the `pub_id`'s consumer bits), each
carrying its source's recorded consumer identity so a `--diff`
verify recording aligns per-consumer streams with the original
instead of collapsing every listener onto one injector identity.
Fresh anonymous claims are floored above the recorded range.
The snapshot IS the coverage boundary: a
subscriber the program only creates during `run()` (spawned or
accepted) is not visible to injection — such tape entries classify
as `late_subscription_uncovered` (the injector join rescans the
live registry at teardown to distinguish them from genuinely
absent subscribers), a stated boundary rather than a silent
`unmatched`. Injection is keyed by **full identity**: each tape record carries
its complete subject string and the subject's canonical payload
shape (FNV-32 alone is collision-prone and is kept only for
reporting); a record whose shape does not match the live
program's shape for that subject is refused as *incompatible*,
never fed as a plausible wrong value. Only messages the original
application **accepted** enter the tape — the wire capture runs
after the deserializer says yes, so identity allocation agrees
between record and replay and rejected traffic is never re-fed.

**Pacing.** Strict replay injects one message, then waits (bounded
by the phase-4 hold) until that message's recorded consumes have
all been re-consumed — unpaced injection can shed at bounded
queues (`bounded(N, drop_old/drop_new)`, `on_full` policies)
before the dequeue-side order gate ever sees the cell, which no
amount of holding can undo. Feed mode is deliberately unpaced.

**Accounting.** Every tape entry ends in exactly one class:
injected, rejected-by-deserializer, unmatched-subject,
incompatible-shape, or unprocessed-at-shutdown (the injector is
joined at teardown and its remainder classified); injector start
failure is its own class. Under strict replay every non-injected
class is a divergence, in the machine-readable status
(`ingress_*`, `injector_start_failure` keys) and the exit
summary.

**Feed mode (`hale replay --feed`, `LOTUS_REPLAY_FEED=<path>`) —
same recorded ingress, changed code, live nondeterministic
environment.** Stated that precisely because "same inputs" would
overclaim: feed reproduces recorded **binding ingress** only —
time, randomness, env, and argv stay live. The recording is
consumed as an input tape: recorded ingress injected, wire
hermetic, and nothing else of replay applies — no journal
serving, no order enforcement, no model admission (a model-hash
mismatch is reported informationally; feeding a tape to changed
code is the point), no `--diff`/`--at` (there is no recorded
schedule to compare against). The **effects gate still applies**:
`--feed` bypasses identity admission, never effect safety — a
program whose frontier reaches `syscall`/`ffi`/`unclassified`
still requires `--allow-live-effects` alongside `--feed`.
Mutually exclusive with `LOTUS_REPLAY`. Injected deliveries carry
the new run's own identity. The exit report classifies every tape
entry, and an unfed remainder (unmatched, incompatible,
unprocessed, start failure) **fails the run by default** —
`--allow-unmatched-feed` is the explicit acceptance of a partial
feed.

**`--diff` (strict replay only — feed rejects it, above).**
`--diff` records the replay (under the original's env policy) and
compares bidirectionally: per-consumer queued consume streams
(target locus + msg_id), per-consumer PUBLIC bus streams aligned
by subject and stable consumer id via the meta maps — which is
what makes synchronous direct dispatch visible — payloads both
ways (topic, flags, bytes; raw metadata by declared size), and
per-consumer journal streams (kind, exact args, withheld state,
value; grouped per consumer because the drain interleaves
concurrent threads' entries in incidental order). Any runtime
divergence fails `--diff` through the verdict. `--at <n>` stops
(SIGSTOP) at the nth consume process-wide (meaningful for one
consumer); `--at <consumer-id>:<ordinal>` is the stable
multi-consumer form. Replay implies observation (identity rides
the obs machinery).

**What a match reports — coverage, per category (GH #728).** A
match is only as strong as the categories the recording carries
observations for, so the success report names all five and states,
for each, either the count it compared or that the category was
**not exercised** and why: public bus events (with their consumer
count), payloads, queued consumes, async schedule steps, journal
reads. A category with no recorded observations is never printed
as a compared zero — "0 consumes across 0 consumers" read as a
verified queued delivery schedule when the workload
direct-dispatched every delivery and no queued schedule existed
to verify (and the same template asserted payload identity for a
recording with no payload blobs). Direct dispatch stays valid and
is named as such: its deliveries are compared, as public bus
events, and its payloads are compared with them — a
direct-dispatched publish records its payload blob like any other
(above), so `payloads` is a compared category for such a
workload, not an unexercised one. The payload line says *what* was
compared, per the blobs the recording carries (GH #842): an
in-process flat payload's blob is metadata only (flag bit 1,
above), so for those the comparator checks declared size and
publish identity and never the contents, and the report reads
`payloads: N (sizes and identities; contents not canonicalised)`;
wire captures are canonical bytes and read `canonical bytes
identical`; a recording carrying both counts each. A flipped field
inside an in-process payload is therefore NOT a divergence
`--diff` can see today — the per-topic canonical codec that would
make it one is a separate feature item (GH #947), not part of this
report. Coverage is derived from the recording the comparator
walks and gated on the same async-capability bit, so the report
cannot claim a category `diff` skipped. `--json` (strict replay
with `--diff`) prints the same verdict machine-readably —
`result`, `ring_records`, `recorded_prefix_only`, and per category
`compared`, `count`, `consumers`, `not_exercised_because`, and for
`payloads` the same split as `contents_canonicalised` and
`metadata_only`. Success
semantics are unchanged: a divergence in any compared category
still fails, and a diverged verdict carries the reason with no
per-category counts (the comparison stopped at the first
difference).

Two honest limits, stated rather than implied: the recorded
interleaving is reproduced per consumer, not globally — cross-
consumer wall-clock alignment is not a replay property; and the
injected tape covers **bindings** ingress — raw user-level socket
reads are syscall-class live effects (gated), and adapter loci
re-execute their own protocol logic. Fleet-scale replay (multiple
binaries against one composed tape, with #262's plan admission
for replay-under-a-different-plan) is the next milestone.

### Time

- **Monotonic + wall-clock.** `time::now()` and
  `time::monotonic()` are runtime-provided. `time::monotonic()`
  returns a `Duration` (i64 nanoseconds since an unspecified
  reference); only meaningful for elapsed-time differences.
  Backed by `clock_gettime(CLOCK_MONOTONIC)`. `time::now()` (C7,
  pond follow-up) returns
  wall-clock seconds since the Unix epoch as `Int` via
  `clock_gettime(CLOCK_REALTIME)`; observation only — NTP
  slewing and leap seconds can warp the value, so
  `time::monotonic` stays the basis for scheduling. Richer
  `Time`-typed wall-clock (with calendar arithmetic) is
  deferred until a consumer surfaces a concrete date-shape
  need. Mocking is available for tests via
  `time::mock_clock(...)` (stdlib).
- **Monotonic-only scheduling.** Every scheduling primitive in
  Hale — `time::sleep`, `time::tick`, the cooperative
  scheduler's deadline queue — is grounded on the monotonic
  clock. NTP slewing, leap seconds, and wall-clock jumps cannot
  warp scheduling decisions. `time::sleep` retries on EINTR
  using the kernel's reported remaining time, so a delivered
  signal does not shorten the total sleep. `CLOCK_REALTIME` is
  used by `time::now()` for wall-clock observation only and
  has no scheduling role.
- **Implementation invariant.** `time::sleep(d)` lowers to
  `clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem)` with EINTR
  retry — important for a system targeting high-precision clock
  semantics.

### I/O — minimal

- **stdout / stderr** for `print` / `println`. That's it for
  runtime-level I/O. Files, networking, etc. live in stdlib.
- **Errno surface helpers** (2026-05-16, used by the fallible
  `std::io::fs::*` / `std::io::tcp::*` wrappers):
  - `lotus_get_errno() -> i32` — surfaces the current platform
    `errno` to LLVM. Each fallible wrapper calls this
    immediately after the failing primitive (POSIX errno is
    sticky until the next syscall sets it).
  - `lotus_io_error_kind(errno_val: i32) -> *const char` —
    maps errno to a stable kind-tag string (`"not_found"`,
    `"permission_denied"`, `"is_dir"`, `"already_exists"`,
    `"would_block"`, `"connection_refused"`, `"timeout"`,
    `"host_unreachable"`, `"broken_pipe"`, `"interrupted"`,
    ..., catch-all `"io"`). Returns a static-table pointer;
    caller must not free.

### Text + string primitives (v1.x adds)

- `lotus_str_parse_float(s) -> double` / `lotus_str_can_parse_float(s) -> int`
  — v1.x-16. Strict trailing-NUL parse; 0.0 on failure paired
  with a bool predicate. Mirrors the parse_int contract.
- `lotus_text_base64_decode(s) -> Bytes*` — v1.x-16. Standard
  alphabet, whitespace tolerated, non-alphabet / wrong padding
  returns empty Bytes. Inverse of `lotus_text_base64_encode`.
- `lotus_str_builder_new()` / `_append(b, s)` / `_len(b) -> i64` /
  `_finish(b) -> char*` — v1.x-15. Doubling-realloc malloc
  buffer. N appends are amortized O(N). `finish()` copies into
  the bus payload arena (program-lifetime) and frees the
  builder.
- `lotus_bytes_builder_new(i64 initial_cap) -> ptr` /
  `_append(ptr handle, ptr chunk) -> i64 status` /
  `_len(ptr handle) -> i64` /
  `_finish(ptr handle) -> Bytes*` /
  `_shift_front(ptr handle, i64 n)` /
  `_clear(ptr handle)` /
  `_snapshot(ptr handle) -> Bytes*` /
  `_view(ptr handle) -> Bytes*` /
  `_free(ptr handle)` — C10 / Phase 0 / Phase-2 (1)
  (2026-05-19, pond/websocket follow-up). Binary-safe sibling
  of the str-builder family. Append reads the chunk's
  `[i64 len]` prefix instead of `strlen`; finish emits a
  length-prefixed Bytes blob with no trailing NUL.
  In-place ops: `shift_front` memmoves the tail to the head
  and drops n bytes (capacity preserved). `clear` sets len=0
  (capacity preserved). `snapshot` copies the current
  `[0..len)` into a fresh Bytes blob in the bus payload
  arena, builder unchanged. `view` returns a non-owning Bytes
  pointer aliasing the builder's inline `[i64 len][u8 data]`
  region — zero allocation, zero copy; lifetime valid until
  the next mutation on the source builder. `free` disposes
  the malloc-backed buffer.

  **F.30 type promotion.** The Hale-visible
  method surface returns `BytesView` / `StringView`
  (typecheck-distinct from `Bytes` / `String`). The view-to-
  owned upgrade paths (`std::bytes::clone`, `std::str::clone`)
  are backed by `lotus_bytes_clone(arena, src)` (new) and
  `lotus_str_clone(arena, src)` (existing m49).

  **F.30b view layout + epoch guard (2026-05-22 PM compaction).**
  The `_view` / `_text_view` C primitives return a 16-byte
  by-value struct — no arena allocation in the hot path. Pre-
  compaction was a 24-byte struct heap-allocated per call,
  and that allocation was the dominant residual chunk-
  allocation trigger in long-running recv loops:

  ```c
  #define LOTUS_VIEW_EPOCH_STATIC ((int64_t)-1)

  typedef struct lotus_view {
      void   *src;     // builder ptr (epoch >= 0, real view)
                       // OR static data ptr (epoch == -1,
                       //   static-lifetime view from
                       //   lotus_view_from_static_data or
                       //   the null-handle path of
                       //   builder_view / builder_text_view).
      int64_t epoch;   // stamped mutation_epoch, or the
                       //   static sentinel.
  } lotus_view_t;
  ```

  The `{void*, int64_t}` layout fits SysV AMD64's "two
  INTEGER eightbytes ≤ 16 bytes" return-by-value rule —
  both registers (`rax`, `rdx`) carry the view; arg-by-value
  passes in two integer arg registers. The underlying data
  pointer is *recomputed* at unpack time from
  `((lotus_bytes_builder_t*)v.src)->buf` (Bytes-shape:
  `buf - 8`; C-string shape: `buf`), so the view itself
  doesn't need to store it.

  `lotus_bytes_builder_t` gains an `int64_t mutation_epoch`
  field bumped by every mutating op (`append`, `append_slice`,
  `shift_front`, `clear`, `advance`). Codegen at view-coerce
  sites emits a call to `lotus_bytes_view_data` /
  `lotus_str_view_data`, which compares the stamped epoch
  against the live epoch and calls `lotus_view_stale_panic`
  (noreturn — stderr + `_exit(1)`) on mismatch. The 5b
  literal-default coercion calls `lotus_view_from_static_data`
  to construct the view in-register with the static
  sentinel; the helpers skip the epoch check on that branch
  and return `v.src` directly as the underlying data pointer.

  **Memory layout (Phase-2 (1)).** The builder header is
  `{cap, buf, mutation_epoch}`; the data area is preceded
  inline by an 8-byte length prefix matching the Bytes ABI:

  ```
  malloc'd region: [int64_t len][u8 data[cap]][NUL]
                                ^
                                buf
  ```

  `view(b)` returns a 16-byte `lotus_view_t` whose `src`
  field is the builder pointer; the read-site helper
  recomputes the data pointer as `b->buf - 8` (Bytes-shape,
  suitable for `lotus_bytes_len` / `lotus_bytes_at` /
  `lotus_bytes_data`). Append / append_slice / shift_front /
  clear / advance all update the inline prefix in sync with
  the data mutation AND bump `mutation_epoch`. Cost: one
  extra pointer dereference per len access vs the prior
  `{cap, len, buf*}` shape, plus a one-load epoch check at
  every view-coerce site. Zero arena allocation per view()
  call (the dominant residual chunk-alloc trigger pre-
  compaction). `lotus_str_builder_t` (for `std::str::*`)
  keeps the prior layout — no view surface there yet.

  These primitives are no longer the user-facing surface;
  they're the C externs called by the
  `std::bytes::BytesBuilder` stdlib locus
  (`crates/hale-codegen/runtime/stdlib/bytes_builder.hl`).
  See `spec/decisions.md` § F.28 for the rationale
  and the locus's method shape. The locus-side calls reach
  these via internal `std::bytes::builder::__*` path-call
  dispatch.

  **ABI notes.** `_new` takes `int64_t
  initial_cap` (previously zero-arg, hardcoded 64) — values
  `<= 0` are treated as the legacy default. `_append`
  returns `int64_t status` (1=ok, 0=fail on realloc-NULL
  or null-handle) — previously void; the status return is
  what the locus's `append` method checks before routing
  through `violate alloc_failed` per F.27.

  **Builder handles are NOT layout-compatible with regular
  Bytes blobs.** The struct shape is
  `{ size_t cap; size_t len; char *buf; }` (24 bytes);
  Bytes blobs are `[int64_t len][u8 data[]]`. So
  `lotus_bytes_at(builder, i)` / `lotus_bytes_len(builder)`
  read the wrong slots if a builder handle is passed as a
  Bytes value. The Hale-level enforcement (`BytesBuilder`
  as its own locus type) makes that mistake impossible to
  express; this note is the C-side mirror — anyone calling
  these primitives directly from C / Rust must keep the
  distinction.
- `lotus_tcp_recv_into(fd, builder, max_bytes) -> i64` /
  `lotus_tls_recv_into(handle, builder, max_bytes) -> i64` /
  `lotus_udp_recv_into(fd, builder, max_bytes) -> i64` —
  2026-05-19 (Phase 1, pond/websocket follow-up).
  Caller-provided destination at the syscall layer. Reads
  directly into the builder's tail; grows on insufficient
  headroom; bumps the builder's len by the count read.
  Return semantics mirror POSIX read(2): `> 0` bytes
  appended, `= 0` peer closed cleanly (TCP) / zero-length
  datagram (UDP), `< 0` error. EINTR retried internally.
  **A `SO_RCVTIMEO` timeout is distinguished from a fatal
  error: `-2` = "would-block / timed out, retryable"; `-1`
  = fatal** (TCP: `EAGAIN`/`EWOULDBLOCK`; TLS: `SSL_read`
  → `SSL_ERROR_WANT_READ`/`WANT_WRITE`). The `-2` only
  arises when the caller has set a recv timeout (opt-in via
  `set_recv_timeout`), so it's backward-compatible — a caller
  that treats all `< 0` as error keeps working; a liveness
  loop checks for `-2` to run its ping/pong instead of
  tearing the connection down. No allocation in
  `g_bus_payload_arena`. No allocation in `g_bus_payload_arena` —
  closes the residual ~80% of the pond/websocket recv-loop
  leak that Phase 0's in-place builder ops surfaced (the
  syscall layer's own `[i64 len][body]` blob per call).
  Helpers `lotus_bytes_builder_reserve(handle, n)` +
  `lotus_bytes_builder_advance(handle, n)` factor the
  grow + offset-bump so `lotus_tls.c` (separate translation
  unit) can implement its recv_into without seeing the
  builder struct layout.
- `lotus_str_lower(s) -> char*` / `lotus_str_upper(s) -> char*`
  — ASCII case folding. One-pass byte-level fold; non-ASCII
  bytes pass through unchanged. Allocates in the bus payload
  arena. Used by `http_request_header` for RFC 7230
  case-insensitive lookup.
- `lotus_str_trim(s) -> char*` — strip ASCII whitespace
  (space / tab / CR / LF) from both ends. Arena-anchored.
- `lotus_str_replace(s, needle, rep) -> char*` — greedy
  non-overlapping substring replace. Two-pass (count, then
  fill) so the output is right-sized in one arena alloc.
  Empty needle is a no-op.
- `lotus_str_repeat(s, n) -> char*` — n copies of s
  concatenated; n <= 0 returns empty. Single arena alloc.
- `lotus_str_pad_left(s, width, pad) -> char*` /
  `lotus_str_pad_right(s, width, pad) -> char*` — width-aligned
  output using the first byte of `pad` (default space) as
  the fill char. No truncation: `len(s) >= width` returns
  s unchanged.

### Process control

- **Exit codes.** `main()` returning `()` exits 0; returning
  `int` exits with that code. Panics exit non-zero.
- **Signal handling.** SIGINT / SIGTERM begin the whole-process
  drain (`semantics.md` § "Drain cascade (whole-process)", GH
  #1039): the handler writes one byte to a pipe, a watcher thread
  raises the exported `lotus_process_draining_flag` (which every
  `self.draining` read loads, beside the locus's own
  `__drain_requested`), wakes every `async_io` pool so timed parks
  that began before the drain expire, and then waits out the grace
  (`LOTUS_DRAIN_GRACE_MS`, default 5000); past it the watcher
  prints one line — the signal, and what the drain was still waiting
  on (each started cooperative pool: its mode, whether its worker is
  mid-iteration and in which locus, its queued cells; the live
  instances that read `draining`, by locus name) — restores the
  signal's default action and re-raises the signal, so the process
  dies by it. The pool's `running_label` and the instance names are
  kept for that line alone; nothing else reads them. A second signal does the same at once.
  Installed by the main prelude only in a program that reads
  `draining`; caught with `SA_RESTART`, never blocked, so spawned
  subprocesses keep the default disposition.
- **SIGPIPE globally ignored** (added 2026-05-17, C2). The
  prelude installs `signal(SIGPIPE, SIG_IGN)` once at
  `lotus_io_init` so writes to a closed pipe (subprocess stdin,
  closed TCP socket, etc.) surface as `EPIPE` through the
  IoError channel instead of synchronously killing the parent.
  Applies process-wide — no opt-out.
- **Subprocess lifecycle** (added 2026-05-17, C2 — see
  [`spec/stdlib.md` § std::process](stdlib.md) for the API
  surface). Every spawned child gets its own process group via
  `setpgid(0, 0)` in the post-fork prelude. Chosen over
  `prctl(PR_SET_PDEATHSIG, SIGKILL)` for POSIX portability
  (macOS / BSD parity); a controlled `Child.dissolve()` covers
  the orderly-shutdown path. `Child.dissolve()` closes the
  three pipe fds and kill-escalates idempotently (SIGTERM →
  100 ms grace → SIGKILL → waitpid; `ESRCH` / `ECHILD` count
  as success) so an unwaited child doesn't leak zombies on
  scope exit. The `std::process::run` synchronous form drains
  stdout + stderr via interleaved `poll()` so the child can
  write to either stream without deadlocking; 16 MiB cap per
  stream.

### stdout buffering

stdout is **line-buffered** for the lifetime of the program,
regardless of whether it's attached to a TTY or a pipe. The
main prelude calls `setvbuf(stdout, NULL, _IOLBF, 0)` once
before any user code runs.

The default libc behavior (fully-buffered when stdout isn't a
TTY) silently dropped output for any program that printed then
blocked on a syscall — `println("READY"); accept_loop();` made
"READY\n" invisible to a piped consumer until the buffer
filled or the program exited. Test oracles, supervisors waiting
for a READY handshake, and log tailers all hung. Line-buffering
matches Python `python -u` discipline and Go's default; `\n`-
terminated `println` calls flush immediately under any stdout
target.

stderr is line-buffered by POSIX already; the runtime doesn't
touch it.

## What's NOT in the runtime (lives in stdlib instead)

- Specific bus transports (NATS, UDP, etc.)
- File I/O
- Networking (sockets, HTTP)
- JSON / protobuf / msgpack encoding
- Most collections beyond what the language has built-in
- Math beyond `sum` / `prod` (which are language-native)
- Statistics
- Linear algebra
- String manipulation beyond literal handling
- Time arithmetic beyond comparison and arithmetic
- Logging / metrics / tracing

These are bundled with the toolchain (no separate install) but
require explicit `import std::...`.

## Form-vec runtime (v1.x-FORM-1)

The `@form(vec)` form lowers to a contiguous growable buffer
implemented in C. See `spec/forms.md` for the form contract and
synthesized method set; this section documents the runtime
shape.

### C struct layout

Each `@form(vec)` locus's heap slot lowers to an inline
struct:

```c
typedef struct {
    size_t cap;   // allocated capacity (elements)
    size_t len;   // number of valid elements
    char  *buf;   // contiguous element array
} lotus_vec_<T>_t;
```

The `<T>` suffix is conceptual — codegen monomorphizes per
cell type T, but the runtime primitives operate on the
common prefix layout via `void *` casts. All `lotus_vec_*_t`
typedefs share the `{cap, len, buf}` prefix.

### Primitive functions

Defined in `crates/hale-codegen/runtime/lotus_arena.c`
(v1.x-FORM-1 PR4):

| Function                                              | Behavior |
|-------------------------------------------------------|----------|
| `void lotus_vec_init(void *v)`                        | Zero-init: cap=0, len=0, buf=NULL |
| `void lotus_vec_push(void *v, size_t es, const void *x)` | Append; doubles cap on overflow |
| `int  lotus_vec_get(void *v, size_t es, int64_t i, void *out)` | Bounds-checked read; returns 1=OK, 0=out-of-bounds |
| `int  lotus_vec_set(void *v, size_t es, int64_t i, const void *x)` | Bounds-checked in-place write; returns 1=OK, 0=out-of-bounds (does not extend the vec) |
| `int  lotus_vec_pop(void *v, size_t es, void *out)` | Returns 1=OK, 0=empty |
| `int64_t lotus_vec_len(void *v)`                      | Element count |
| `int  lotus_vec_is_empty(void *v)`                    | 1=empty, 0=non-empty |
| `void lotus_vec_destroy(void *v)`                     | `free(buf)`; called at locus dissolve |
| `void lotus_vec_sort_int(void *v)`                    | In-place ascending sort of an `int64_t`-cell vec via `qsort` |
| `void lotus_vec_sort_float(void *v)`                  | In-place ascending sort of a `double`-cell vec; NaN treated as equal-to-anything |
| `void lotus_vec_sort_string(void *v)`                 | In-place ascending sort of a `char *`-cell vec under `strcmp` ordering |
| `void lotus_vec_sort_by(void *v, size_t es, int (*cmp)(const void *, const void *, void *), void *cookie)` | `qsort_r` wrapper; cmp is a codegen-synthesized per-(cell_type, direction) trampoline |

`es` (elem_size) is the cell type's size in bytes — codegen
passes `sizeof(T)` at each call site.

### Growth policy

- Initial: cap=0, no allocation at locus birth.
- First push: allocates a 4-element buffer.
- Each overflow: doubles cap; `realloc`s. Old contents copied
  by realloc; previous buf freed.
- Shrink: not implemented in v1. Buf released at dissolve.

### Failure shapes

- `lotus_vec_get` / `lotus_vec_set` / `lotus_vec_pop` return 0
  on contract break (out-of-bounds / empty). Codegen wraps this
  into the `Ty::Fallible { success: T, payload: IndexError }`
  surface (Unit-success for `set`) via a small adapter that
  synthesizes the `IndexError` struct from the bool + the call
  args (shipped v1.x-FORM-2 PR5/6; `set` added 2026-05-16).
- `lotus_vec_push` OOM routes through the substrate-trap →
  closure-violation channel per the two-channel rule (shipped
  v1.x-FORM-2 PR6).
- Sort family (`sort`, `sort_by`, `sort_desc_by`, added
  2026-05-16) is infallible from the language surface; `sort_*`
  C wrappers do not return a status code. If the user-supplied
  comparator in `sort_by` faults (a fallible call inside the
  comparator body raised through `or raise`), the fault
  propagates and `qsort_r` stops mid-sort — the vec is left
  with every element still present, ordering partially applied.

## Form-hashmap runtime (v1.x-FORM-4)

The `@form(hashmap)` form lowers to an intrusive open-addressing
hash table implemented in C. See `spec/forms.md` for the form
contract and synthesized method set; this section documents the
runtime shape.

### C struct layout

Each `@form(hashmap)` locus's pool slot lowers to an inline
struct:

```c
typedef struct {
    size_t cap;          // power-of-two slot count
    size_t len;          // live entry count
    size_t key_size;     // sizeof(K), set at init
    size_t value_size;   // sizeof(S), set at init
    int    key_type_tag; // 0 = Int, 1 = String
    char  *slots;        // cap * (1 + key_size + value_size) bytes
} lotus_hashmap_t;
```

Each slot is `1 + key_size + value_size` bytes:

```
[occupied: u8] [key: key_size bytes] [value: value_size bytes]
```

`occupied = 0` means empty. Backward-shift deletion (no
tombstones) — probes terminate at the first empty slot.

The C ABI is type-erased: codegen passes `key_size` /
`value_size` at init time, and per-call sites pass raw `void *`
key/value pointers. Codegen GEPs the indexed-by field on the
caller's side to derive the key pointer before each `set`.

### Primitive functions

Defined in `crates/hale-codegen/runtime/lotus_arena.c`
(v1.x-FORM-4 PR4):

| Function | Behavior |
|----------|----------|
| `void lotus_hashmap_init(void *m, size_t key_size, size_t value_size, int key_type_tag)` | Allocate `cap=8` slots, zero them; freeze key/value sizes and key-type tag |
| `void lotus_hashmap_set(void *m, const void *key, const void *value)` | Insert or replace; grow at load factor 0.7 |
| `int  lotus_hashmap_get(void *m, const void *key, void *out_value)` | Bounds-checked read; returns 1=OK, 0=missing_key |
| `int  lotus_hashmap_has(void *m, const void *key)` | 1=present, 0=missing |
| `int  lotus_hashmap_remove(void *m, const void *key)` | 1=removed, 0=missing |
| `int64_t lotus_hashmap_len(void *m)` | Live entry count |
| `int  lotus_hashmap_is_empty(void *m)` | 1=empty, 0=non-empty |
| `void lotus_hashmap_destroy(void *m)` | `free(slots)`; called at locus dissolve |

### Key types and hashing

| `key_type_tag` | Type | Hash function | Equality |
|---|---|---|---|
| `0` (LOTUS_HASHMAP_KEY_INT) | `int64_t` | Knuth multiplicative (`k * 0x9E3779B97F4A7C15`) | `==` on i64 |
| `1` (LOTUS_HASHMAP_KEY_STRING) | `const char *` (NUL-terminated) | FNV-1a over the bytes | `strcmp == 0`, with pointer-identity fast path |

Other key types (Bytes, custom structs, enum tags) are not
supported at v1; codegen rejects `@form(hashmap)` with a
focused diagnostic when the indexed-by field's resolved type
doesn't map to one of these two tags.

### Growth policy

- Initial: `cap=8`, slots calloc'd at locus birth via
  `lotus_hashmap_init`.
- Growth: when `(len + 1) > 0.7 * cap`, double cap and rehash
  every live entry through the normal `set` path (the probe
  sequence changes with the new mask, so we don't copy raw
  bytes between tables).
- Shrink: not implemented in v1.
- Cap is always a power of two so hash-to-index folds to
  `& mask`.

### Deletion policy

Backward-shift deletion (no tombstones). After clearing the
target slot, the runtime walks forward through the cluster and
shifts any entry whose natural position is "before" the freed
slot in the probe sequence. The cluster boundary is the first
empty slot encountered. This keeps probe chains tight and lets
`find_slot` terminate correctly without a separate tombstone
marker.

### Failure shapes

- `lotus_hashmap_get` / `lotus_hashmap_remove` return 0 on
  contract break (missing key). Codegen wraps this into the
  `Ty::Fallible { success: S, payload: KeyError }` surface via
  the same machinery `@form(vec)` uses for `IndexError`,
  synthesizing the `KeyError { kind: "missing_key" }` payload
  at the call site (shipped v1.x-FORM-4 PR5).
- `lotus_hashmap_set` OOM during the slot calloc / realloc
  routes through the substrate-trap → closure-violation
  channel per the two-channel rule.

## Form-ring-buffer runtime (v1.x-FORM-5)

`@form(ring_buffer, cap = N)` lowers a pool capacity slot to an
inline `lotus_ring_buffer_t` and synthesizes a fixed-capacity
FIFO surface (push / pop / len / is_full). The cap is baked in
at `lotus_ring_buffer_init` from the form annotation arg; the
backing buffer is malloc'd once at locus birth and never grows.

### C struct layout

```c
typedef struct {
    size_t cap;        // fixed at init; never changes
    size_t head;       // index of oldest element (next pop)
    size_t len;        // current element count, 0..=cap
    size_t elem_size;  // bytes per element
    char  *buf;        // cap * elem_size bytes
} lotus_ring_buffer_t;
```

Codegen emits the matching LLVM inline struct on the locus's
pool slot; the slot's struct field IS the ring buffer (no
indirection). Element-size is `sizeof(T)` from the cell type's
LLVM `size_of`.

### Primitive functions

Defined in `crates/hale-codegen/runtime/lotus_arena.c`
(v1.x-FORM-5):

| Function | Behavior |
|----------|----------|
| `void lotus_ring_buffer_init(void *rb, size_t cap, size_t elem_size)` | `malloc(cap * elem_size)`; head=len=0 |
| `int  lotus_ring_buffer_push(void *rb, const void *src)` | 1=pushed, 0=full; wraps modulo cap |
| `int  lotus_ring_buffer_pop(void *rb, void *out)` | 1=popped, 0=empty; advances head |
| `int64_t lotus_ring_buffer_len(void *rb)` | Current element count |
| `int  lotus_ring_buffer_is_full(void *rb)` | 1=full (len==cap), 0=not |
| `void lotus_ring_buffer_destroy(void *rb)` | `free(buf)` at locus dissolve |

### Failure shapes

- `push` returns 0 when full → codegen converts to Bool false at
  the language surface (`fn push(x: T) -> Bool`).
- `pop` returns 0 when empty → codegen lazily allocates an
  `EmptyError { kind: "empty" }` payload on the err path,
  surfaced to the caller's `or` clause.
- OOM during init (cap × elem_size too large to malloc) leaves
  `buf == NULL`; subsequent push/pop see a 0-cap buffer and
  refuse / fail. Routing OOM through the closure-violation
  channel is deferred to a future hardening pass — the v1
  contract is "fixed cap; if init can't allocate, the buffer
  is permanently empty."

## Native codegen defaults

What the compiler emits for a native `hale build`:

- **Host-CPU tuning + O3 by default.** Native builds tune to the
  host CPU (`target-cpu`/`target-features` from the build
  machine) and run LLVM's aggressive (O3) pipeline — both the
  module passes and the backend codegen level. This unlocks
  autovectorization across all generated code (e.g. AVX-512 on a
  capable host). **Consequence:** a native binary is **not
  portable across microarchitectures** — it may use instructions
  absent on an older CPU.
- **`--target-cpu native | baseline`.** `native` (default) is the
  host-tuned build above. `baseline` pins a portable
  **`x86-64-v3`** target (AVX2 + BMI2 + FMA) for **distributed
  artifacts** that must run on any modern x86-64 CPU. The
  emitted module self-describes its subtarget via per-function
  `target-cpu`/`target-features` attributes, so the choice is
  carried into bitcode (it survives LTO).
- **`LOTUS_LTO=1` — opt-in full-LTO.** Read at *build time*.
  Emits the Hale module as LLVM bitcode and compiles the lotus C
  runtime TUs with `-flto`, so the final `clang -flto -O3
  -fuse-ld=lld` link inlines the runtime hot paths (arena
  bump-allocator, string helpers, shm_ring framing) **across the
  TU boundary** into the Hale-generated callers — a boundary
  that's otherwise opaque. Worth a few percent on
  allocation/coordination-heavy code; neutral on
  already-vectorized loops (the host tuning is preserved under
  LTO via the function attributes above). **Off by default:** the
  LTO link is ~3–4× slower and requires `lld` on PATH. Native,
  non-sanitizer builds only; `wasm32` and sanitizer builds keep
  the ordinary non-LTO link. The `-Wl,--wrap` malloc/syscall
  shims (and the `LOTUS_ARENA_LOG_BIG_CHUNKS` /
  `std::diag::syscall_count` features that ride them) are
  preserved under LTO — `lld` resolves `--wrap` before LTO
  codegen.
- **`wasm32` is unaffected** — it stays `generic`/O2 (the browser
  bundle is size/compat-sensitive).

## Build-time and toolchain environment

Codegen reads no environment variable. Every knob that changes what a
build emits is a field of `hale_codegen::BuildOptions`, and the one
function that turns the process environment into those fields is
`build_options_from_env` (`crates/hale-cli/src/build_env.rs`); every
command that compiles (`hale build`, `run`, `test`, `replay`) starts
from it, so a variable means the same thing to all four. A caller of
the library (a test, another tool) sets the field and touches no
environment, and must choose a `cache_dir`: `BuildOptions` has no
`Default`, only `BuildOptions::new(cache_dir)`, because the runtime-object
cache is the one setting with no right answer to guess; `crates/hale-codegen/tests/codegen_reads_no_environment.rs`
fails if a read appears in the crate.

A boolean variable is on for `1`, `true` or `TRUE` and off for anything
else. A variable marked *set* is on by being set to any value at all.
The rows with no field are read by the rest of the toolchain, not by a
build.

| Variable | `BuildOptions` field | Effect | Default |
|---|---|---|---|
| `HALE_DEV` (*set*) | `dev_profile` | The `--dev` profile: LLVM O1 module pipeline and Less machine codegen, trading runtime speed for build latency. | off |
| `LOTUS_LTO` | `lto` | `1`, `true` or `full`: full LTO; `thin`: ThinLTO; anything else, off. Native, non-sanitizer builds only (see *Native codegen defaults*). | off |
| `LOTUS_ASAN` | `asan` | Build with AddressSanitizer and skip the O3 module pipeline, so a leak or use-after-free report carries accurate frames. | off |
| `LOTUS_TSAN` | `tsan` | Build with ThreadSanitizer for the runtime compile and the link; the `-Wl,--wrap` shims are left out. | off |
| `LOTUS_UBSAN` | `ubsan` | Build with address and undefined-behavior sanitizers, aborting on the first UB (`-fno-sanitize-recover=all`). | off |
| `LOTUS_DUMP_IR` (*set*) | `dump_ir_beside_output` | Write the pre-optimization LLVM IR to `<output>.ll`. A library caller may instead name the path in `dump_ir`. | off |
| `LOTUS_NO_BUS_DEVIRT` | `no_bus_devirt` | Force the all-dynamic bus lowering: every subject's dispatch plan is empty. The differential harness's control arm. | off |
| `LOTUS_NO_OWNERSHIP_BUBBLE` | `no_ownership_bubble` | Force the pre-bubble ownership lowering: no bubble plans, no forwarding sets, no threading fields. | off |
| `LOTUS_DISABLE_PREFETCH` | `disable_prefetch` | Compile the runtime without its prefetch hints. | off |
| `LOTUS_DI_TRACE` (*set*) | `di_trace` | Narrate debug-location decisions on stderr. | off |
| `HALE_DISPATCH_TRACE` | `dispatch_trace` | Print the flavor the bus dispatch plan chose for each subject, with its gate's `payload_flat` column, and the codec's flatness at each publish to a literal subject, on stderr. | off |
| `HALE_LIFECYCLE_TRACE` | `lifecycle_trace` | The lifecycle trace: one line per obligation event on stderr (§ "The lifecycle trace"). A debug build; native host targets only. | off |
| `HALE_TIME` (*set*) | `time_phases` | Print per-phase wall times of the build on stderr. | off |
| `HALE_CC_WARNINGS` | `cc_warnings` | Let the runtime's C warnings through instead of `-w`. For work on the runtime itself. | off |
| `HALE_NO_LLD` | `no_lld` | Link with the default linker even when `ld.lld` is on PATH (Linux only; lld is otherwise used). | off |
| `HALE_NO_TS_SHIM` | `no_ts_shim` | Behave as if `libhale_ts_shim.a` was never built: a `std::ts` program gets the located refusal, other programs link without it. | off |
| `HALE_TS_SHIM_A` | `ts_shim` | The tree-sitter shim staticlib to link, ahead of `<hale binary dir>` and the workspace `target/` dirs. | unset |
| `HALE_ZIG` | `zig` | The `zig` binary a cross build (`--target`) compiles and links with. | `zig` on PATH |
| `HALE_TARGET_GLIBC` | `target_glibc` | The glibc version a cross build's Linux gnu binary asks for. | `2.31` |
| `HALE_TARGET_SYSROOT` | `target_sysroot` | The sysroot a cross build finds OpenSSL, zlib and the shim in. | `<cache>/hale/sysroot/<triple>` |
| `XDG_CACHE_HOME`, `HOME` | `cache_dir` | Where compiled runtime objects are cached, content-addressed: `$XDG_CACHE_HOME/hale/runtime`, else `~/.cache/hale/runtime`. An empty value is skipped. | else `<tmp>/hale-runtime-cache-<pid>`, a directory of that process's own |
| `LOTUS_OPENSSL_PREFIX`, `OPENSSL_ROOT_DIR` | `openssl_prefix` | macOS: a Homebrew OpenSSL prefix (the first whose `include/openssl/ssl.h` exists) for the link. | the standard brew locations |
| `LOTUS_NO_DEBUGINFO` | none: the CLI supplies no `debug` sources | Opt out of DWARF line tables for the Hale code (the runtime C always carries `-g`). | off |
| `HALE_BIN` | none | The `hale` binary a child process runs as its toolchain: `hale dna` sets it, to the binary it is running as, for the hosts and fixtures it starts. | the running binary |
| `HALE_IMPORT_DEBUG` (*set*) | none | Trace import resolution on stderr, per call. | off |
| `HALE_MCP_ROOT` | none | `hale mcp`: every path a tool call names must resolve under this directory. | unset: no restriction |
| `HALE_MODEL_TRACE` | none | Print the model builder's and the fleet lowering's demand-proof line on stderr (`1` turns it on). | off |
| `HALE_REPLAY_TEST_HOLD` | none | A test hook: `hale replay` waits, between admitting a recording and starting it, until this path exists, so a test can replace the recording at a chosen moment. | unset |
| `HALE_SKIP_STALE_CHECK` | none | Any value but empty or `0` skips the check that the binary is not older than the workspace it was built from. | off |
| `HALE_STALE_DNA_ROOT` | none | The tree the stale-DNA check compares the embedded DNA against; the regression test's way to hand it a tree it may edit. | the workspace the binary was built from |
| `HALE_TEST_JOBS` | none | How many workers `hale test` runs when `-j` is not given. | one per available core |
| `HALE_TEST_KEEP_VAULT` | none | `1` keeps each test file's vault directory (mode 0700 under `<tmp>/hale-test-vaults-<uid>`) after its run and prints where, instead of removing it. | remove |
| `HALE_WARM_SKIP_IRIS` | none | `scripts/warm-dna-cache.sh` skips building the observer (`1`), for jobs that never run `hale iris`. | off |

## Diagnostic + tuning env vars

Every variable the runtime (`lotus_arena.c`, `lotus_obs.c`) and the
standard library read at run time, and the ones the CLI sets for the
process it starts. Unset (the default) keeps the runtime quiet and
its behavior as described in this document.

| Env var | Default | Effect |
|---|---|---|
| `LOTUS_ARENA_LOG_BIG_CHUNKS=<N>` | off | Logs every arena chunk + libc allocator (malloc / realloc / calloc / mmap) >= `<N>` bytes to stderr with size, monotonic seqno, and 8-frame backtrace. Use `1` (= 1 MiB) as a shortcut; any positive decimal byte count works (e.g. `4096`). Each event labeled by source: `arena_big_chunk`, `malloc_big`, `realloc_big`, `calloc_big`, `mmap_big`. Note: only fires on the fresh-malloc path; chunks recycled from the per-thread pool bypass this hook — use `LOTUS_ARENA_LOG_CHUNK_ATTACH` for the full picture. |
| `LOTUS_ARENA_LOG_CHUNK_ATTACH=<N>` | off | Logs every chunk attachment to ANY arena — both fresh-malloc (`chunk_attach_malloc`) AND per-thread-pool-recycled (`chunk_attach_pool`) paths — when `cap >= N`. Use `1` for "log every chunk attachment". Each event additionally prints `arena=<ptr> kind=<root\|sub> label=<resolved>`: `root` means the chunk attached to a top-level locus-lifetime arena (a leak class if it grows); `sub` means a subregion (method scratch / free-fn body) that will recycle to the pool on destroy. `label` walks the subregion→root chain and looks up the root in the residency registry — requires `LOTUS_ARENA_RESIDENCY=1` to populate the label map. Filter `kind=root label=<name>` to isolate the actual arena-growers. Shares the `LOTUS_ARENA_LOG_BIG_MAX_EVENTS` cap with the big-chunks logger. |
| `LOTUS_ARENA_LOG_BIG_MAX_EVENTS=<N>` | 200 | Caps the log at `<N>` events per process. Default 200. Set to 0 for unlimited (useful when watching low-rate sub-MiB allocation patterns over a long window). |
| `LOTUS_CHUNK_POOL_STATS=1` | off | Dumps per-thread chunk-pool hit / miss / store / overflow counters to stderr at process exit. Diagnostic for "pool isn't recycling" symptoms — pairs hits vs misses, stores vs overflows. The atexit handler runs on the main thread; counters are `__thread` so the dump is that thread's view. |
| `LOTUS_GLIBC_ARENA_MAX=<N>` | unset (glibc's own) | Calls `mallopt(M_ARENA_MAX, <N>)` at startup. Caps glibc's per-thread malloc arena count. `1` forces a single arena (max contention, min virtual-address fragmentation); higher `<N>` trades contention for parallelism. Useful belt-and-suspenders against the per-thread arena heap-segment proliferation glibc default tuning can produce on long-running daemons. Unset keeps glibc's default. |
| `LOTUS_BUS_PAYLOAD_ARENA_CAP=<N>` | 64 MiB | Overrides the lazy-global bus payload arena's byte cap (default 64 MiB). When the cap fires, `lotus_arena_alloc` returns NULL and the existing alloc-fail paths (`empty_global` / `alloc_failed` violation) surface degraded service rather than OOM-killing the process. |
| `LOTUS_DRAIN_GRACE_MS=<ms>` | 5000 | How long a SIGINT / SIGTERM drain may take before the runtime re-raises the signal with its default action, ending the process by it (default 5000). Only in a program that reads `self.draining`; see `semantics.md` § "Drain cascade (whole-process)" (GH #1039). |
| `LOTUS_BUS_CALL_ARENA_STATS=1` | off | At exit, prints `[bus call arenas] opened=N live_bytes=L peak_bytes=P` to stderr: how many per-call arenas adapter `send`s and keyed adapter deliveries opened, the chunk bytes still held (0 unless a call is in flight), and the most held at once. The call arenas are not residency targets, so this is how a test sees them (GH #1038). |
| `LOTUS_ARENA_RESIDENCY=1` | off | Registers every top-level arena (locus `__arena`s, `g_bus_payload_arena`, the program-wide global) into a side-table at creation time with a 24-frame construction backtrace. `std::process::dump_arena_residency()` walks the live set and emits one line per arena to stderr — bytes / chunks / parent / label, sorted by bytes desc — with the construction backtrace. Subregions (method scratch) are skipped; they destroy at method exit and don't accumulate residency. Atexit also dumps, but post-dissolve fires after all loci tear down — useful only for the global arena's final state. Long-running daemons should call `dump_arena_residency` from a heartbeat / checkpoint tick so locus arenas are sampled while still alive. |
| `LOTUS_CHUNK_POOL_PREFILL=<N>` | 32 | Per-thread chunk-pool pre-fill on first touch. Default 32 (= 2 MiB resident per scheduler thread). Set 0 to disable. Bumps the pool's steady-state floor so brief bursts don't drain to zero and miss into malloc; the trade-off is per-thread resident memory. |
| `LOTUS_TSAN=1` | off (build time) | Read at *build time* (by `hale build`, not at runtime; see *Build-time environment*). When set, the emitted clang command passes `-fsanitize=thread` for both the C runtime compile and the binary link, and skips the `-Wl,--wrap=malloc/realloc/calloc/mmap` shim surface (TSAN intercepts malloc itself; the wrap'd `LOTUS_ARENA_LOG_BIG_CHUNKS` diagnostic is silently no-op under TSAN). The resulting binary runs ~5-15× slower; use only for race-hunting workloads. The C runtime embeds an empty `__tsan_default_suppressions` hook at link time so no external suppression file is needed; all originally-flagged substrate races (bus queue drain, arena destroy, coop pool worker, env-var lazy-init) have been fixed and the suppression list is empty. Opt-in tests live behind `#[ignore]` and the env var (see `crates/hale-codegen/tests/form_hashmap_lockfree_tsan.rs`). |
| `LOTUS_LTO=1` | off (build time) | Read at *build time*. Opt-in full-LTO native build: the Hale module is emitted as bitcode and the lotus runtime TUs compile with `-flto`, so the `clang -flto -O3 -fuse-ld=lld` link inlines the runtime hot paths (arena / string / shm_ring) across the TU boundary into the Hale callers. A few percent on allocation/coordination-heavy code, neutral on vectorized loops (host tuning preserved via per-function `target-features`). Off by default — ~3–4× slower link, requires `lld`. Native non-sanitizer only; `--wrap` shims survive (lld resolves them before LTO codegen). See *Native codegen defaults* above. |
| `LOTUS_BUS_LOG_UNMATCHED=1` | off | Surfaces silent no-key-match drops in `lotus_bus_local_dispatch_keyed` (Phase 3 routing keys). When set, each publish that matches no `where key == ...` subscriber for the topic emits a single stderr line citing subject, key, and the per-topic subscriber counts (specific vs unkeyed). Off by default — the silent-drop is correct for `on_unmatched: swallow` topics in steady state, but during bring-up the lack of any signal is load-bearing on debug cycles. Implied by `LOTUS_BUS_LOG_DROP=1`. |
| `LOTUS_BUS_LOG_DESERIALIZE_DROP=1` | off | Surfaces silent drops in the udp:// reader thread when (a) no deserializer is registered for the inbound subject, or (b) the deserializer returns `<= 0` (size mismatch, bounded-read failure). Emits one stderr line per drop naming the subject, the payload size, and (when applicable) the deserializer's return value. Off by default; the silent-skip on cross-routed multicast noise is the correct steady-state behavior. Same env-gated pattern as `LOTUS_BUS_LOG_UNMATCHED` for keyed-dispatch misses. Implied by `LOTUS_BUS_LOG_DROP=1`. |
| `LOTUS_BUS_QUEUE_CAP=<N>` | 8192 | Caps the cooperative bus dispatch queue, each per-pinned-locus mailbox, and each cooperative pool's queue at `N` cells (default 8192; floor 64; rounded up to a power of two; read once). **v0.9.0 footprint change:** the pinned mailbox and cooperative-pool queues are now lock-free MPSC rings (Vyukov bounded ring + signal-only-when-parked wake), and a fixed-size lock-free ring **pre-allocates its cap up front** rather than growing to it — so each pinned subscriber mailbox and each cooperative pool now costs ~4.3 MB resident at the default cap (vs the prior grow-as-needed). With the typical handful of pinned loci / pools this is a few-to-low-tens of MB; **lower `LOTUS_BUS_QUEUE_CAP` for pinned-/pool-heavy programs** to shrink it (the rings honor it identically). When a producer hits the cap it *back-pressures* instead of growing without bound (GH #125) — every message is still delivered. The mechanism: a **single-threaded** producer on the cooperative queue **inline-drains** it to free space; a **cross-thread** producer to a full ring **blocks** (a fenced producers-waiting handshake) until the single consumer drains a slot; a handler self-publishing to its own full ring spills to a consumer-thread-local overflow list (it can't block on itself). The cross-*cooperative*-pool *shared* queue path (multiple drainers, no single consumer) is the remaining non-lock-free path — a follow-on. Lower the cap to tighten the bound / footprint; raise it to reduce drain bursts at the cost of resident memory. |
| `LOTUS_UNIX_STREAM=1` | off (`SOCK_SEQPACKET` on Linux) | GH #231: forces the unix bus transport into framed `SOCK_STREAM` mode on Linux (the Darwin default — macOS has no AF_UNIX `SOCK_SEQPACKET`). Wire format per message: `[u64 LE payload len][u64 LE seq][payload]`; the seq is per-connection monotonic from 1, reset per accepted peer, and the receiver counts gaps (`seq_gaps` counter — GH #236's loss-computability primitive). Set for EVERY process on a socket: framed and SEQPACKET ends don't interoperate (a mismatch trips the 8 MB length sanity cap with a diagnostic naming the likely cause). Primary use: Linux CI/test coverage of the macOS code path. |
| `LOTUS_BUS_COUNTERS_DUMP=1` | off | GH #236 (observability groundwork): prints one stderr line per remote binding at teardown — `[bus counters] subject=... kind=... role=... sent= delivered= bytes_sent= bytes_delivered= send_failures= dropped_lost= waits= rearms= reconnects=`. The counters are plain relaxed atomics bumped at the transport choke points (fanout send, serve-loop dispatch, re-arm, reconnect) and exist as the substrate for the iris observer; the dump is the operator/test surface until an in-process consumer ships. `dropped_lost` counts publishes made while a connect binding was in the lost/reconnecting window (GH #233 — drops the publish contract makes deliberate and visible). `waits` counts publishes that parked in `or wait` through that window instead (GH #255 phase 1). `buffered_early` / `dropped_early` (GH #468) count boot-window wire messages buffered before any matching subscriber registration existed, and the subset dropped (buffer eviction at the 64-msg/1 MiB per-binding cap, a deserialize failure at flush, or a subject that never gained a subscriber by teardown). Socket-buffer occupancy is intentionally not a counter: it's a poll-time `SIOCOUTQ` query against the live fd. |
| `LOTUS_BUS_LOG_DROP=1` | off | Broad superset for diagnosing "publish appears to succeed but handler doesn't fire" symptoms. Implies `LOTUS_BUS_LOG_UNMATCHED` + `LOTUS_BUS_LOG_DESERIALIZE_DROP` AND covers additional silent-drop sites the narrower vars miss: `lotus_bus_dispatch`'s serialize-fn-returns-<=0 case, the local-fanout (`lotus_bus_dispatch_wire` + `lotus_bus_local_dispatch`) zero-matching-subscribers case, per-entry deserialize-returns-<=0 on the local-fanout path, and the no-post-target case (mailbox / coop_pool / global queue all NULL on a matched entry). Each line names the call site, subject, and relevant size / index info so a bus-heavy repro can identify exactly which silent-skip is firing. Reach for this first when investigating bus-drop friction; the narrower vars stay supported for their specific bring-up scenarios. |
| `LOTUS_BUS_QUIESCE_MS` | 500 | GH #468: bound on the main-exit ingress quiesce (default `500`, `0` disables it). At every main-exit point, before pools join and loci dissolve, LISTEN binding fds half-close and their readers drain kernel-accepted data to true EOF through the still-intact registry; this bounds how long exit waits for a wedged reader. Exceeding the bound is loud (stderr names it) and drops whatever stayed undrained. Test-only siblings `LOTUS_BUS_TEST_BOOT_HOLD_MS` / `LOTUS_BUS_TEST_READER_STALL_MS` deterministically stretch the boot-registration window / the reader's descheduled window (the loss canaries in `binding_ingest_468.rs` use them); never set them in production. |
| `LOTUS_ARENA_CHUNK_BYTES_OVERRIDE=<N>` | 64 KiB | Overrides the arena chunk size (F.32-3): a power of two in [4 KiB, 16 MiB], anything else ignored; read once. Smaller chunks keep each locus's hot chunk in cache across pool rotations in a multi-locus-per-pool deployment. Overridden chunks bypass the per-thread chunk pool. |
| `LOTUS_NO_CHUNK_POOL=<x>` | off (on under an ASan build) | Any non-empty value not starting with `0` turns the per-thread chunk pool off, so a destroyed arena's chunks are freed instead of recycled with their bytes intact and a use-after-free reads memory a sanitizer can see (GH #816). |
| `LOTUS_HUGE_PAGES=1` | off | F.32-4a: chunks of at least 2 MiB are mapped with `MAP_HUGETLB \| MAP_HUGE_2MB` instead of malloc'd. A value starting with `1`, `t` or `T` turns it on. |
| `LOTUS_LOCK_MEMORY=1` | off | F.32-4c: `mlockall(MCL_CURRENT \| MCL_FUTURE)` at startup (Linux), to remove page-fault stalls on hot-path arena allocation. Needs `RLIMIT_MEMLOCK` or root; on failure it prints a diagnostic on stderr and runs unlocked. A value starting with `1`, `t` or `T` turns it on. |
| `LOTUS_BUS_CONFIG=<path>` | unset | The bus config file (the peer and route table `hale node` writes) that `lotus_bus_load_config` reads at startup. Unset or unreadable, the program is single-process. |
| `LOTUS_BUS_UDP_RCVBUF=<N>` | the kernel's | `SO_RCVBUF`, in bytes, for the udp bus readers. Ignored unless a positive `int`. |
| `LOTUS_BUS_TEST_BOOT_HOLD_MS=<ms>` | 0 | Test only: stretches the boot-registration window of a listening binding (see `LOTUS_BUS_QUIESCE_MS`). Never set in production. |
| `LOTUS_BUS_TEST_READER_STALL_MS=<ms>` | 0 | Test only: stretches the window in which a binding's reader is descheduled. Never set in production. |
| `LOTUS_TEST_PINNED_START_NO_DRAIN=1` | off | Test only: disables the instantiating thread's queue drain while it waits for pinned or pool initialization. The startup request/reply negative controls use it to expose the resulting deadlock. Any non-empty value not starting with `0` enables it. Never set in production. |
| `LOTUS_LIFECYCLE_SKIP=<steps>` | unset | Test only, and read only by a lifecycle-trace build (`HALE_LIFECYCLE_TRACE=1`; a release runtime has no such code): a comma list of steps a negative control removes, a kind (`PoolJoin`) or one event line (`Reclaim.Completed`). See *The lifecycle trace*. |
| `HALE_MATRIX=full` | the sample | Test only, read by the test suite (`ownership_matrix.rs`, `lifecycle_matrix.rs`), never by a program: `full` runs every cell of the generated ownership and lifecycle matrices instead of the default deterministic sample. |
| `LOTUS_OBS=1` | off | Native observation emission (iris): the process creates its observation segment and its probes emit. Implied by `LOTUS_OBS_RECORD` and `LOTUS_REPLAY`. See *Native observation emission*. |
| `LOTUS_OBS_RINGS=<N>` | 8 (64 when recording) | Rings in the observation segment, 1 to 64; a value outside that falls back to the default. |
| `LOTUS_OBS_SLOTS=<N>` | 4096 | Slots per ring: a power of two, at least 64; anything else falls back to the default. |
| `LOTUS_OBS_WIRE=1` | off | Puts the `(origin, seq)` edge identity on the wire: the udp magic prefix and the framed transport's origin widening. That is a wire-format change a pre-header receiver cannot parse, so the whole fleet opts in together; it is what cross-process edges need (iris handoff-4 P16). |
| `LOTUS_OBS_RECORD=<path>` | unset | Lossless recording (GH #296): write the run's tape to `<path>`; implies observation. See *Lossless recording mode*. |
| `LOTUS_OBS_RECORD_DURABLE=1` | off | Synchronize every flushed drain sweep of the recording (`fdatasync` on Linux, `F_FULLFSYNC` on macOS); the default rides the page cache (survives a process crash, not power loss). |
| `LOTUS_OBS_RECORD_ENV=full` | withheld | Record the value of every environment read, not just its name, existence and length; the artifact header says which policy applied. |
| `LOTUS_REPLAY=<path>` | unset | Replay a recording (`hale replay` sets it): the run is judged against the tape. |
| `LOTUS_REPLAY_FEED=<path>` | unset | Feed mode (`hale replay --feed`): the recording is input, not law. Mutually exclusive with `LOTUS_REPLAY`. |
| `LOTUS_REPLAY_ALLOW_TRUNCATED=1` | off | Accept a recording with no clean-finalize trailer (a crashed writer); `--allow-truncated`. |
| `LOTUS_REPLAY_ALLOW_UNVERIFIED=1` | off | Replay although the recording's model identity differs from this build's; `--allow-unverified`. |
| `LOTUS_REPLAY_FEED_ALLOW_UNMATCHED=1` | off | In feed mode, unfed tape is a failure unless this is set. |
| `LOTUS_REPLAY_STATUS=<path>` | unset | Where the replayed process writes the verdict counters (journal misses, order divergences) the CLI reads. |
| `LOTUS_REPLAY_AT=<n>` | unset | Stop the replayed run (SIGSTOP) at the n'th consume; `hale replay --at <n>`. |
| `LOTUS_REPLAY_AT_CONSUMER=<id>` | unset | With `LOTUS_REPLAY_AT`, count the n'th consume of that consumer instead of the process-wide n'th; `--at <consumer-id>:<n>`. |
| `LOTUS_REPLAY_FD=<fd>` | unset | An already-open, validated descriptor for the recording, passed by `hale replay` so no path is re-resolved between admission and replay. |
| `LOTUS_API=<path>` | the baked path | Overrides the unix socket path of the program's `api` binding at run time. |
| `LOTUS_API_ROLES=<table>` | the baked `[environments.<env>.roles]` | Overrides the api binding's roles table at run time; with no table baked in, every gate refuses until this says otherwise. |
| `HALE_LOG=<level>` | unset: no filtering | `std::log` drops events below this level (`debug`, `info`, `warn`, `error`, case-insensitive). Anything unrecognized, and an unset variable, filters nothing, so a typo cannot discard the logs it was set to see. |
| `HALE_VAULT_DIR=<dir>` | `$XDG_CACHE_HOME/hale/vault`, else `~/.cache/hale/vault`, else `/tmp/hale-vault` | The local vault directory a `vault:` source of `std::secret` reads (a file per name). |
| `HALE_VAULT_ADDR=<url>` | unset | When set, a `vault:` source resolves over the vault's HTTP API at `<url>/v1/secret/<name>` instead of the local directory. |
| `HALE_VAULT_TOKEN=<token>` | unset | The bearer token for `HALE_VAULT_ADDR`; empty, the lookup fails closed. |

Every top-level arena is created via `lotus_arena_create_labeled(name)` and carries an immutable human-readable label string. The codegen passes the locus name (e.g. `WsClient`, `__lib_metrics_metrics_MetricMap`); the program-wide global is labeled `lotus.arena.global`; `g_bus_payload_arena` labels itself. The label is the load-bearing identifier in the residency dump; backtraces resolve via `-rdynamic` for cases where the label alone isn't enough.

The arena-chunk pool and -wrap=malloc family ship in every
binary unconditionally; the env vars are zero-cost when unset
(one int read + one branch per allocation). The `-rdynamic`
link flag is similarly unconditional so backtrace symbols
resolve without addr2line.

## Runtime size budget

The runtime should be small enough that a hello-world program
binary is < 1 MB statically linked, and < 100 KB if dynamic
linking against libc. This is a target, not a guarantee.

The framework's discipline enables this: no GC, no metadata
overhead per allocation, region-based MM compiles to bump
allocators. Comparable to C in size, with ergonomics closer to
Erlang.

## Open questions for runtime

- **Async / await integration.** Reserved keywords, no v0
  semantics. The lifecycle state machine + cooperative yield
  points subsume most of what async is for; explicit
  async/await may not be necessary.
- **FFI to existing languages.** Generic FFI in stdlib;
  team-specific bindings (e.g. domain-specific typed messages)
  live as third-party packages. Marshalling helpers in stdlib.
- **Hot-reload of code (not just perspectives).** Erlang
  supports module-level hot reload. Lotus's perspective
  hot-reload is more granular and addresses most of the use
  case; full code hot-reload may not be needed.
- **Determinism mode for tests.** Resolved for the single-pool
  case: no mode is needed, because single-pool execution is
  deterministic by construction, and that is now a stated,
  pinned guarantee rather than an accident — see
  `spec/testing.md` § Determinism. What remains open is
  deterministic *re-execution* of multi-pool programs, which is
  the record/replay track (GH #296) — reproducing a recorded
  per-consumer order, never constraining live scheduling.
