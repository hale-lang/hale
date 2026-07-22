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
  using the instance as a value is rejected at compile time.
- **State machine enforcement.** A locus can't accept after
  drain has begun, can't run before birth completed, etc. The
  runtime tracks state; transitions are rejected if they
  violate ordering.
- **`drain()` cascades depth-first.** Calling `drain()` on a
  locus first recursively drains all its children (depth-first),
  waits for them, then drains itself. SIGINT triggers `drain()`
  on the runtime root, cascading through the whole process
  tree. No separate cascade syntax — `drain()` is always
  cascading. For locus-typed param fields specifically
  (F.29), the codegen walks `LocusRef` fields in declaration
  order at the cascade-teardown sites (ephemeral scope-exit
  and deferred-flush) and calls each child's drain BEFORE the
  outer locus's own drain. The subsequent dissolve cascade
  runs the outer's `closures → dissolve` body next, then per
  child `closures → dissolve → arena_destroy`, then outer's
  arena_destroy. Pinned-thread tail still skips the cascade
  per the v1 trade-off. An `accept`'d child is reclaimed on its
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
  before dissolving its `params` fields**, so a worker still
  executing a pool-placed field's `run()` can never touch that
  field's arena after it's freed (the alternative — freeing first
  — is a use-after-free; the alternative join-without-(a) is a
  hang). Together these let a program whose `main` run() returns
  while a classic-pool server child is live shut down cleanly
  rather than hanging or segfaulting.
- **Recovery primitives.** `restart`, `restart_in_place`,
  `quarantine`, `reorganize`, `bubble`, `dissolve`, `drain` —
  all language keywords; runtime implements the actual
  effects.
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
handoff).

Each bus delivery to a subscriber on an `async_io` pool runs its
handler on a coroutine (a struct + a 64 KiB stack). Rather than
allocate and free that pair per delivery, the pool keeps a bounded
per-worker free-list (cap 64) of completed coro slots and reuses
them — a warm fan-out skips the per-dispatch stack allocation
entirely (2026-07-16). The free-list is worker-thread-local (no
lock) and drained at pool teardown, so a busy async pool retains up
to 64 × 64 KiB (~4 MiB) of coro stacks at steady state. Transparent
to user code — a pure allocation optimization, no behavior change.

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
condvar+memcpy machinery. The single-threaded-method invariant
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
instantiation / scope-exit boundaries).

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
loci spawn a pthread at instantiation; the locus's full
declared lifecycle (birth → run → drain → dissolve, each only
if declared) executes on that thread, in order. Main thread
continues immediately after spawn. At scope exit (deferred-
dissolve flush), `pthread_join` blocks until the pinned
thread has finished its lifecycle and returned; the main
thread's only remaining work for a pinned entry is the join
plus the locus's arena destroy wholesale (drain / dissolve are
SKIPPED on the main side — they ran on the pinned thread).

m28a synthesizes a per-locus `__pinned_main_<LocusName>`
function whose signature matches pthread's start-routine
contract directly (`ptr (ptr)`); pthread_create gets that
function pointer with `self_ptr` as its argument. No C-side
adapter, no thread_args struct. The synthesized body simply
calls each declared lifecycle method in sequence, then
returns null.

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
that declares `bus subscribe` allocates its own
`lotus_mailbox_t` at instantiation: a bounded ring buffer with
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
m28b's mailbox post-and-continue) or closures (cross-thread
violation routing). Codegen errors clearly if those are
present.

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
`bindings { Topic: <transport>; }` block. Two transport shapes
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
- The serve loop (`lotus_bus_unix_serve`) accepts a peer,
  dispatches its messages, and on peer EOF **re-arms** — closes
  the dead connection and loops back into `accept()` for the
  next peer (GH #233 step 2; peer EOF is not connection loss,
  and a rolling restart of the connect-side binary just works).
- `dissolve()` calls `lotus_bus_transport_reclaim`: sets the
  serve loop's `closing` flag, shuts down both fds to unblock a
  parked accept/recv, joins the serve thread, destroys the
  transport. The husk entry stays in the remote table for
  `lotus_bus_remote_destroy_all` to free uniformly.

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
main, `exit(3)`). Signal-driven termination (SIGTERM, SIGKILL,
`_exit`) bypasses atexit and leaves the SHM namespace entry
behind until reboot or manual `shm_unlink`. A future v1.x
SIGINT/SIGTERM handler can fold into this same teardown.

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
  At dissolve, if exploded, the parent's
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
- **Signal handling.** SIGINT / SIGTERM trigger `drain` →
  `dissolve` on the root locus. Stdlib provides finer-grained
  control if needed.
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

## Diagnostic + tuning env vars

A small set of env vars toggle runtime instrumentation and
glibc tuning hooks. All are opt-in; unset (the default) keeps
the runtime quiet.

| Env var | Effect |
|---|---|
| `LOTUS_ARENA_LOG_BIG_CHUNKS=<N>` | Logs every arena chunk + libc allocator (malloc / realloc / calloc / mmap) >= `<N>` bytes to stderr with size, monotonic seqno, and 8-frame backtrace. Use `1` (= 1 MiB) as a shortcut; any positive decimal byte count works (e.g. `4096`). Each event labeled by source: `arena_big_chunk`, `malloc_big`, `realloc_big`, `calloc_big`, `mmap_big`. Note: only fires on the fresh-malloc path; chunks recycled from the per-thread pool bypass this hook — use `LOTUS_ARENA_LOG_CHUNK_ATTACH` for the full picture. |
| `LOTUS_ARENA_LOG_CHUNK_ATTACH=<N>` | Logs every chunk attachment to ANY arena — both fresh-malloc (`chunk_attach_malloc`) AND per-thread-pool-recycled (`chunk_attach_pool`) paths — when `cap >= N`. Use `1` for "log every chunk attachment". Each event additionally prints `arena=<ptr> kind=<root\|sub> label=<resolved>`: `root` means the chunk attached to a top-level locus-lifetime arena (a leak class if it grows); `sub` means a subregion (method scratch / free-fn body) that will recycle to the pool on destroy. `label` walks the subregion→root chain and looks up the root in the residency registry — requires `LOTUS_ARENA_RESIDENCY=1` to populate the label map. Filter `kind=root label=<name>` to isolate the actual arena-growers. Shares the `LOTUS_ARENA_LOG_BIG_MAX_EVENTS` cap with the big-chunks logger. |
| `LOTUS_ARENA_LOG_BIG_MAX_EVENTS=<N>` | Caps the log at `<N>` events per process. Default 200. Set to 0 for unlimited (useful when watching low-rate sub-MiB allocation patterns over a long window). |
| `LOTUS_CHUNK_POOL_STATS=1` | Dumps per-thread chunk-pool hit / miss / store / overflow counters to stderr at process exit. Diagnostic for "pool isn't recycling" symptoms — pairs hits vs misses, stores vs overflows. The atexit handler runs on the main thread; counters are `__thread` so the dump is that thread's view. |
| `LOTUS_GLIBC_ARENA_MAX=<N>` | Calls `mallopt(M_ARENA_MAX, <N>)` at startup. Caps glibc's per-thread malloc arena count. `1` forces a single arena (max contention, min virtual-address fragmentation); higher `<N>` trades contention for parallelism. Useful belt-and-suspenders against the per-thread arena heap-segment proliferation glibc default tuning can produce on long-running daemons. Unset keeps glibc's default. |
| `LOTUS_BUS_PAYLOAD_ARENA_CAP=<N>` | Overrides the lazy-global bus payload arena's byte cap (default 64 MiB). When the cap fires, `lotus_arena_alloc` returns NULL and the existing alloc-fail paths (`empty_global` / `alloc_failed` violation) surface degraded service rather than OOM-killing the process. |
| `LOTUS_ARENA_RESIDENCY=1` | Registers every top-level arena (locus `__arena`s, `g_bus_payload_arena`, the program-wide global) into a side-table at creation time with a 24-frame construction backtrace. `std::process::dump_arena_residency()` walks the live set and emits one line per arena to stderr — bytes / chunks / parent / label, sorted by bytes desc — with the construction backtrace. Subregions (method scratch) are skipped; they destroy at method exit and don't accumulate residency. Atexit also dumps, but post-dissolve fires after all loci tear down — useful only for the global arena's final state. Long-running daemons should call `dump_arena_residency` from a heartbeat / checkpoint tick so locus arenas are sampled while still alive. |
| `LOTUS_CHUNK_POOL_PREFILL=<N>` | Per-thread chunk-pool pre-fill on first touch. Default 32 (= 2 MiB resident per scheduler thread). Set 0 to disable. Bumps the pool's steady-state floor so brief bursts don't drain to zero and miss into malloc; the trade-off is per-thread resident memory. |
| `LOTUS_TSAN=1` | Read at *build time* (by the codegen's `build_executable`, not at runtime). When set, the emitted clang command passes `-fsanitize=thread` for both the C runtime compile and the binary link, and skips the `-Wl,--wrap=malloc/realloc/calloc/mmap` shim surface (TSAN intercepts malloc itself; the wrap'd `LOTUS_ARENA_LOG_BIG_CHUNKS` diagnostic is silently no-op under TSAN). The resulting binary runs ~5-15× slower; use only for race-hunting workloads. The C runtime embeds an empty `__tsan_default_suppressions` hook at link time so no external suppression file is needed; all originally-flagged substrate races (bus queue drain, arena destroy, coop pool worker, env-var lazy-init) have been fixed and the suppression list is empty. Opt-in tests live behind `#[ignore]` and the env var (see `crates/hale-codegen/tests/form_hashmap_lockfree_tsan.rs`). |
| `LOTUS_LTO=1` | Read at *build time*. Opt-in full-LTO native build: the Hale module is emitted as bitcode and the lotus runtime TUs compile with `-flto`, so the `clang -flto -O3 -fuse-ld=lld` link inlines the runtime hot paths (arena / string / shm_ring) across the TU boundary into the Hale callers. A few percent on allocation/coordination-heavy code, neutral on vectorized loops (host tuning preserved via per-function `target-features`). Off by default — ~3–4× slower link, requires `lld`. Native non-sanitizer only; `--wrap` shims survive (lld resolves them before LTO codegen). See *Native codegen defaults* above. |
| `LOTUS_BUS_LOG_UNMATCHED=1` | Surfaces silent no-key-match drops in `lotus_bus_local_dispatch_keyed` (Phase 3 routing keys). When set, each publish that matches no `where key == ...` subscriber for the topic emits a single stderr line citing subject, key, and the per-topic subscriber counts (specific vs unkeyed). Off by default — the silent-drop is correct for `on_unmatched: swallow` topics in steady state, but during bring-up the lack of any signal is load-bearing on debug cycles. Implied by `LOTUS_BUS_LOG_DROP=1`. |
| `LOTUS_BUS_LOG_DESERIALIZE_DROP=1` | Surfaces silent drops in the udp:// reader thread when (a) no deserializer is registered for the inbound subject, or (b) the deserializer returns `<= 0` (size mismatch, bounded-read failure). Emits one stderr line per drop naming the subject, the payload size, and (when applicable) the deserializer's return value. Off by default; the silent-skip on cross-routed multicast noise is the correct steady-state behavior. Same env-gated pattern as `LOTUS_BUS_LOG_UNMATCHED` for keyed-dispatch misses. Implied by `LOTUS_BUS_LOG_DROP=1`. |
| `LOTUS_BUS_QUEUE_CAP=<N>` | Caps the cooperative bus dispatch queue, each per-pinned-locus mailbox, and each cooperative pool's queue at `N` cells (default 8192; floor 64; rounded up to a power of two; read once). **v0.9.0 footprint change:** the pinned mailbox and cooperative-pool queues are now lock-free MPSC rings (Vyukov bounded ring + signal-only-when-parked wake), and a fixed-size lock-free ring **pre-allocates its cap up front** rather than growing to it — so each pinned subscriber mailbox and each cooperative pool now costs ~4.3 MB resident at the default cap (vs the prior grow-as-needed). With the typical handful of pinned loci / pools this is a few-to-low-tens of MB; **lower `LOTUS_BUS_QUEUE_CAP` for pinned-/pool-heavy programs** to shrink it (the rings honor it identically). When a producer hits the cap it *back-pressures* instead of growing without bound (GH #125) — every message is still delivered. The mechanism: a **single-threaded** producer on the cooperative queue **inline-drains** it to free space; a **cross-thread** producer to a full ring **blocks** (a fenced producers-waiting handshake) until the single consumer drains a slot; a handler self-publishing to its own full ring spills to a consumer-thread-local overflow list (it can't block on itself). The cross-*cooperative*-pool *shared* queue path (multiple drainers, no single consumer) is the remaining non-lock-free path — a follow-on. Lower the cap to tighten the bound / footprint; raise it to reduce drain bursts at the cost of resident memory. |
| `LOTUS_UNIX_STREAM=1` | GH #231: forces the unix bus transport into framed `SOCK_STREAM` mode on Linux (the Darwin default — macOS has no AF_UNIX `SOCK_SEQPACKET`). Wire format per message: `[u64 LE payload len][u64 LE seq][payload]`; the seq is per-connection monotonic from 1, reset per accepted peer, and the receiver counts gaps (`seq_gaps` counter — GH #236's loss-computability primitive). Set for EVERY process on a socket: framed and SEQPACKET ends don't interoperate (a mismatch trips the 8 MB length sanity cap with a diagnostic naming the likely cause). Primary use: Linux CI/test coverage of the macOS code path. |
| `LOTUS_BUS_COUNTERS_DUMP=1` | GH #236 (observability groundwork): prints one stderr line per remote binding at teardown — `[bus counters] subject=... kind=... role=... sent= delivered= bytes_sent= bytes_delivered= send_failures= dropped_lost= rearms= reconnects=`. The counters are plain relaxed atomics bumped at the transport choke points (fanout send, serve-loop dispatch, re-arm, reconnect) and exist as the substrate for the iris observer; the dump is the operator/test surface until an in-process consumer ships. `dropped_lost` counts publishes made while a connect binding was in the lost/reconnecting window (GH #233 — drops the publish contract makes deliberate and visible). Socket-buffer occupancy is intentionally not a counter: it's a poll-time `SIOCOUTQ` query against the live fd. |
| `LOTUS_BUS_LOG_DROP=1` | Broad superset for diagnosing "publish appears to succeed but handler doesn't fire" symptoms. Implies `LOTUS_BUS_LOG_UNMATCHED` + `LOTUS_BUS_LOG_DESERIALIZE_DROP` AND covers additional silent-drop sites the narrower vars miss: `lotus_bus_dispatch`'s serialize-fn-returns-<=0 case, the local-fanout (`lotus_bus_dispatch_wire` + `lotus_bus_local_dispatch`) zero-matching-subscribers case, per-entry deserialize-returns-<=0 on the local-fanout path, and the no-post-target case (mailbox / coop_pool / global queue all NULL on a matched entry). Each line names the call site, subject, and relevant size / index info so a bus-heavy repro can identify exactly which silent-skip is firing. Reach for this first when investigating bus-drop friction; the narrower vars stay supported for their specific bring-up scenarios. |

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
- **Determinism mode for tests.** Discussed in `testing.md`;
  runtime needs to support deterministic scheduling when
  requested. The cooperative scheduler makes this easier than
  M:N would have — single-scheduler test mode is fully
  deterministic by construction.
