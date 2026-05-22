# Runtime

What every compiled Aperio binary always ships with. Always-
loaded; not optional; no `import` needed; the substrate every
Aperio program depends on.

This document distinguishes the **runtime** (always there) from
the **standard library** (`stdlib.md`, importable but bundled).
Go's distinction between `runtime` and other stdlib packages is
the model: runtime is automatic; stdlib is explicit.

> **Naming note:** The language is **Aperio**; the runtime/
> substrate concept is called **lotus**, and the C-runtime
> symbols stay `lotus_*` (per project memory). When this doc
> says "lotus" it means the substrate; "Aperio" means the
> language proper.

## What's in the runtime

### Memory

- **Region allocator.** Per-locus arenas, hierarchical, freed
  on dissolution. Bump allocation within a region; no per-object
  metadata; no GC. The framework's lotus structure provides the
  scope; the allocator just respects it.
- **Per-method scratch (2026-05-21).** Each locus method body
  (lifecycle / user-fn / mode) opens a per-call subregion of
  `self.__arena` at entry and destroys it at every return.
  Transient allocations made inside the body — `to_string`,
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
  arena_destroy. Pinned-thread tail and `parent_accepts_us`
  still skip the cascade per the v1 trade-off.
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
- **No preemption within a scheduler.** A locus's handler runs
  to completion or an explicit yield.
- **Cross-scheduler is bus.** No shared memory; no locks.
- **Failure-traversal**: if parent and child are on the same
  scheduler, failure-traversal is a stack walk. If different
  schedulers, the failure is delivered as a typed bus message
  to the parent's scheduler, which dispatches to `on_failure`.

### Schedule classes (per-locus execution strategy)

Just as **projection class** governs a locus's memory strategy,
**schedule class** governs its execution strategy.
Substrate-invariance applied to time the way projection class
applies it to space — but kept honestly **bimodal**: either you
share a scheduler thread or you own one. There is no third
position.

Annotation:

```
locus Fitter          : schedule cooperative { ... }   // default
locus DataIngest      : schedule pinned      { ... }
```

| Class | Yield discipline | Resource |
|---|---|---|
| **Cooperative** (default) | Yields between substrate cells (handler exit, lifecycle transition, bus dispatch, `time::sleep`, explicit `yield`). Handler bodies are atomic. See "`time::sleep` yield-point caveat" below. | Shares a scheduler thread with other cooperative loci. |
| **Pinned** | No yield to siblings; owns its scheduler. Bus events to/from cross thread boundaries via formal mailbox post. | Dedicated OS thread, optionally pinned to a CPU core. |

#### Why no "greedy" class

A natural temptation is to want a third option: "shares the
scheduler thread but doesn't yield." That would be a bimodality
violation. Cooperative already guarantees handler-level
atomicity — no preemption within a substrate cell — so the only
thing such a class could add over cooperative is "don't yield
*between* cells either." But that means leaving the shared
scheduler entirely. The place you go when you leave is your own
thread. That's pinned.

Latency-critical work, or anything that genuinely shouldn't
share with siblings, is signaling that it belongs in a *deeper
layer of the lotus* — its own thread, formal cross-boundary
posts, fewer neighbors. That's a layering decision, not a third
scheduling regime. Two classes, no third position, by design.

#### `time::sleep` yield-point caveat (v1.x polish 2026-05-20)

The "Yield discipline" row above lists `time::sleep` as a yield
point. That holds for the cooperative scheduler's queue — sleeps
in the same cooperative locus do let the runtime drain
in-flight handler invocations before the loop body resumes.

It does NOT today drain the cross-thread mailbox bus dispatch
path. A `main locus` looping `while { std::time::sleep(100ms);
... }` will not deliver inbound handler invocations queued from
a `unix(...)`-bound (or other cross-thread-routed) `subscribe`
declaration. The handlers post into the mailbox; the cooperative
scheduler never observes the dispatch until an explicit `yield;`
runs the drain.

**Working pattern:** add an explicit `yield;` after the sleep
in any loop body that consumes cross-thread-bound bus events.

```aperio
run() {
    while !self.bail {
        std::time::sleep(100ms);
        yield;   // drain cross-thread bus dispatch into handlers
        // ... loop body sees the latest dispatches
    }
}
```

A future runtime change may fold cross-thread mailbox draining
into the sleep yield point. Until then, the explicit `yield;`
is the documented form.

**Item B from the 2026-05-21 friction log** flagged a related
shape: a cooperative publisher's `<-` cell never fires the
pinned subscriber's handler mid-program; the cell drains only
at dissolve. Same root cause — `yield;` doesn't drain the
cross-thread mailbox even though the spec wording implies it
should. The working pattern there is direct method invocation
on the receiver (`self.on_subscribe(req)`) inside the publisher's
loop, bypassing the bus for the in-binary case. Cross-binary
flows (unix / shm_ring transport with their own reader threads)
work fine because their dispatch path doesn't go through the
cooperative-pinned mailbox at all. The runtime fix is the same
as the sleep case: extend `yield;` to drain the cross-thread
mailbox. Deferred until a workload demands the bus-shaped path
specifically (today's workaround composes the same handler
body from both bus and direct entry points).

#### Long-running cooperative children block parent run() (D)

When a locus declares another locus as a `params` field —
`metrics_server: std::http::Server = ...` — the child's full
lifecycle (`birth → run → drain → dissolve`) runs synchronously
inside the parent's birth-time instantiation chain. Cooperative
children block: a child Server with `max_accepts: -1` enters its
accept loop and never returns, so the parent's run() never
starts. Documented as item D ("`std::
http::Server` as child of a parent with non-trivial `run()`
blocks parent").

This is a consequence of the cooperative scheduler's serialized
model, not a bug: cooperative siblings can't run concurrently
within one scheduler. The shipped resolutions:

  1. **Sibling-in-main pattern** — declare long-running co-
     resident loci as siblings in `main` rather than as one's
     child. Cooperative + pinned siblings coexist there (the
     pinned one runs on its own thread). To shut them down
     gracefully when one finishes (e.g. a pinned gateway exits its
     duration_s), call `metrics_server.shutdown()` from the
     finishing locus's thread — the C-iii interruptible-accept
     work makes this the supported pattern.

  2. **Pin the child** — declare the child as `: schedule
     pinned` so its run() spawns its own thread and returns
     immediately to the parent. Works when you own the child
     locus's declaration; the std:: surface ships cooperative
     by default and overriding requires a wrapper locus that
     re-instantiates the cooperative child inside its own
     pinned run() body. Practical for one-off cases; not
     ergonomic enough to recommend broadly.

A substrate change that dispatches cooperative children's run()
asynchronously (parent's run() starts immediately after the
child's birth() returns) would tighten this. Deferred — the
sibling-with-shutdown pattern covers the production case after C-iii.

(Compare: rich / chunked / recognition projection classes are
genuinely three-way because N≈10, N≈30, and N≈300 are
different cost regimes at scale — memory has more genuine
intermediate ground than time does.)

#### Cross-class bus semantics

- **Cooperative → cooperative on same scheduler**: handler
  enqueues; runs at the next substrate cell on the subscriber's
  scheduler. Sender never blocks on receiver.
- **Any → pinned**: cross-thread post via lock-protected
  mailbox. Sender never blocks.
- **Pinned → any**: same — cross-thread post; pinned doesn't
  block waiting for delivery acknowledgement.

#### Implementation status (m26 + m27 + m28a + m28b + m28c)

m25 wired the annotation through parse / typecheck / codegen.
**m26 ships cooperative semantics; m27 ships pinned threads
(run-only); m28a lifts pinned to full lifecycle; m28b lights up
cross-thread bus mailboxes — pinned loci can subscribe and
publish, with cells routed across threads via per-locus
mailboxes; m28c adds optional `: schedule pinned(core = N)`
syntax for explicit CPU-core affinity via
`pthread_setaffinity_np`.**

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
  contract definition lives in `runtime/stdlib/bus.ap`; concrete
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

**Adapter dispatch.** At codegen, an adapter binding
instantiates the adapter locus into the program-lifetime
payload arena (same m90 routing the `-> LocusRef(L)` return
path uses), resolves the locus's `send` method's fn pointer,
and registers the (self, send_fn) pair with the runtime via
`lotus_bus_register_remote_adapter`. The runtime stores both
in `lotus_bus_remote_entry_t`'s adapter slot. Outbound fanout
packages the wire bytes as an Aperio-level `Bytes` value
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

**SHM ring substrate (Form K5, 2026-05-20).** POSIX shared-
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

v1 scope: single-producer, multi-consumer; in-memory delivery is
in scope (POSIX shm_open works intra-machine cross-process).
Multi-producer (CAS-based claim), back-pressure / timeout
modes, and named-ring registry are post-v1. The Aperio-side
`fallible(ClaimError)` signature is reserved for those; v1's
`claim()` never actually fails.

**Lifecycle / cleanup (2026-05-20).** Both
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
  design-rationale §F.9.
- **Recovery-event interaction.** `persists_through(...)` and
  `resets_on(...)` clauses are honored at recovery time; the
  accumulator is preserved or zeroed per declaration. The
  exploded flag itself persists across `restart_in_place` and
  `quarantine` (per default; future `clear_violation_on(...)`
  clause may override).

### Perspective infrastructure

- **Stable-perspective tracking.** For each `perspective T`,
  the runtime tracks how many independent perspectives have
  validated; `stable_when` is invoked to determine commit
  status.
- **Hot-load.** The runtime accepts a serialized
  `T`-perspective from a transport, verifies the type
  signature against the locally-compiled `T`, and atomically
  installs it. Old perspective is preserved until the new one
  is committed (no torn read).

### Failure handling

- **Failure = `ClosureViolation` propagation.** Any `closure`
  assertion that fails in a locus body produces a
  `ClosureViolation` record routed to the parent's
  `on_failure(child, err)` handler per **F.9**. The parent
  picks one of `restart` / `restart_in_place` / `quarantine` /
  `reorganize` / `bubble`, or absorbs (returns without calling
  any). A violation that bubbles past the root exits the
  process non-zero with the violation report on stderr.
- **No source-level panic / exceptions.** Aperio has no
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
  Backed by `clock_gettime(CLOCK_MONOTONIC)` on both interpreter
  and codegen paths. `time::now()` (C7, pond follow-up) returns
  wall-clock seconds since the Unix epoch as `Int` via
  `clock_gettime(CLOCK_REALTIME)`; observation only — NTP
  slewing and leap seconds can warp the value, so
  `time::monotonic` stays the basis for scheduling. Richer
  `Time`-typed wall-clock (with calendar arithmetic) is
  deferred until a consumer surfaces a concrete date-shape
  need. Mocking is available for tests via
  `time::mock_clock(...)` (stdlib).
- **Monotonic-only scheduling.** Every scheduling primitive in
  Aperio — `time::sleep`, `time::tick`, the cooperative
  scheduler's deadline queue — is grounded on the monotonic
  clock. NTP slewing, leap seconds, and wall-clock jumps cannot
  warp scheduling decisions. `time::sleep` retries on EINTR
  using the kernel's reported remaining time, so a delivered
  signal does not shorten the total sleep. `CLOCK_REALTIME` is
  used by `time::now()` for wall-clock observation only and
  has no scheduling role.
- **Implementation invariant.** Both interpreter and codegen
  paths lower `time::sleep(d)` to
  `clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem)` with EINTR
  retry. The same primitive on both paths means observable
  scheduling behavior is identical regardless of the
  compilation route — important for a system targeting
  high-precision clock semantics where the substrate cannot
  drift between development and production.

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

  **F.30 (2026-05-20) type promotion.** The Aperio-visible
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
  (`crates/aperio-codegen/runtime/stdlib/bytes_builder.ap`).
  See `spec/design-rationale.md` § F.28 for the rationale
  and the locus's method shape. The locus-side calls reach
  these via internal `std::bytes::builder::__*` path-call
  dispatch.

  **ABI notes (2026-05-19).** `_new` takes `int64_t
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
  Bytes value. The Aperio-level enforcement (`BytesBuilder`
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
  datagram (UDP), `< 0` fatal error. EINTR retried
  internally. No allocation in `g_bus_payload_arena` —
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

### stdout buffering (2026-05-17)

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

Defined in `crates/aperio-codegen/runtime/lotus_arena.c`
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

### Interpreter parity

The interpreter (`crates/aperio-runtime/src/eval.rs`)
implements the same surface via the `SlotState::Vec` variant
backed by `Rc<RefCell<Vec<Value>>>` and the
`try_eval_form_vec_call` dispatcher (v1.x-FORM-1 PR7).
Synthesized methods route directly to this dispatcher before
normal locus-method lookup.

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

Defined in `crates/aperio-codegen/runtime/lotus_arena.c`
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

### Interpreter parity

The interpreter (`crates/aperio-runtime/src/eval.rs`)
implements the same surface via the `SlotState::Hashmap`
variant and the `try_eval_form_hashmap_call` dispatcher
(v1.x-FORM-4 PR6). Synthesized methods route directly to this
dispatcher before normal locus-method lookup. The interpreter's
backing structure does not need to match the C runtime's
data-structure choices — only the observable semantics.

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

Defined in `crates/aperio-codegen/runtime/lotus_arena.c`
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

### Interpreter parity

The interpreter implements the same surface via the
`SlotState::RingBuffer { cap, items: VecDeque<Value> }` variant
and the `try_eval_form_ring_buffer_call` dispatcher. The deque
backing differs from the C struct's circular array, but the
observable push/pop/len/is_full semantics match.

## Diagnostic + tuning env vars

A small set of env vars toggle runtime instrumentation and
glibc tuning hooks. All are opt-in; unset (the default) keeps
the runtime quiet.

| Env var | Effect |
|---|---|
| `LOTUS_ARENA_LOG_BIG_CHUNKS=<N>` | Logs every arena chunk + libc allocator (malloc / realloc / calloc / mmap) >= `<N>` bytes to stderr with size, monotonic seqno, and 8-frame backtrace. Use `1` (= 1 MiB) as a shortcut; any positive decimal byte count works (e.g. `4096`). Each event labeled by source: `arena_big_chunk`, `malloc_big`, `realloc_big`, `calloc_big`, `mmap_big`. |
| `LOTUS_ARENA_LOG_BIG_MAX_EVENTS=<N>` | Caps the log at `<N>` events per process. Default 200. Set to 0 for unlimited (useful when watching low-rate sub-MiB allocation patterns over a long window). |
| `LOTUS_CHUNK_POOL_STATS=1` | Dumps per-thread chunk-pool hit / miss / store / overflow counters to stderr at process exit. Diagnostic for "pool isn't recycling" symptoms — pairs hits vs misses, stores vs overflows. The atexit handler runs on the main thread; counters are `__thread` so the dump is that thread's view. |
| `LOTUS_GLIBC_ARENA_MAX=<N>` | Calls `mallopt(M_ARENA_MAX, <N>)` at startup. Caps glibc's per-thread malloc arena count. `1` forces a single arena (max contention, min virtual-address fragmentation); higher `<N>` trades contention for parallelism. Useful belt-and-suspenders against the per-thread arena heap-segment proliferation glibc default tuning can produce on long-running daemons. Unset keeps glibc's default. |
| `LOTUS_BUS_PAYLOAD_ARENA_CAP=<N>` | Overrides the lazy-global bus payload arena's byte cap (default 64 MiB). When the cap fires, `lotus_arena_alloc` returns NULL and the existing alloc-fail paths (`empty_global` / `alloc_failed` violation) surface degraded service rather than OOM-killing the process. |
| `LOTUS_ARENA_RESIDENCY=1` | Registers every top-level arena (locus `__arena`s, `g_bus_payload_arena`, the program-wide global) into a side-table at creation time with a 24-frame construction backtrace. `std::process::dump_arena_residency()` walks the live set and emits one line per arena to stderr — bytes / chunks / parent / label, sorted by bytes desc — with the construction backtrace. Subregions (method scratch) are skipped; they destroy at method exit and don't accumulate residency. Atexit also dumps, but post-dissolve fires after all loci tear down — useful only for the global arena's final state. Long-running daemons should call `dump_arena_residency` from a heartbeat / checkpoint tick so locus arenas are sampled while still alive. |
| `LOTUS_CHUNK_POOL_PREFILL=<N>` | Per-thread chunk-pool pre-fill on first touch. Default 32 (= 2 MiB resident per scheduler thread). Set 0 to disable. Bumps the pool's steady-state floor so brief bursts don't drain to zero and miss into malloc; the trade-off is per-thread resident memory. |

Every top-level arena is created via `lotus_arena_create_labeled(name)` and carries an immutable human-readable label string. The codegen passes the locus name (e.g. `KrakenMdgw`, `__lib_metrics_metrics_MetricMap`); the program-wide global is labeled `lotus.arena.global`; `g_bus_payload_arena` labels itself. The label is the load-bearing identifier in the residency dump; backtraces resolve via `-rdynamic` for cases where the label alone isn't enough.

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
