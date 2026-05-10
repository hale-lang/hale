# Runtime

What the lotus binary always ships with. Always-loaded; not
optional; no `import` needed; the substrate every Aperio program
depends on.

This document distinguishes the **runtime** (always there) from
the **standard library** (`stdlib.md`, importable but bundled).
Go's distinction between `runtime` and other stdlib packages is
the model: runtime is automatic; stdlib is explicit.

## What's in the runtime

### Memory

- **Region allocator.** Per-locus arenas, hierarchical, freed
  on dissolution. Bump allocation within a region; no per-object
  metadata; no GC. The framework's lotus structure provides the
  scope; the allocator just respects it.
- **Per-projection-class allocation strategy.** Rich → simple
  arena; chunked → arena with per-coordinatee sub-regions;
  recognition → fixed-size pre-allocated pool. Selected at
  compile time per locus.
- **Free-list within parent for bookkeeping reclamation.** When
  a coordinatee dissolves, its bookkeeping slot in the parent's
  arena is reclaimed via a per-arena free-list (chunked-class
  loci) or periodic defrag (high-churn loci). Reclamation is
  **per-arena**, **bounded**, **deterministic** — never stop-
  the-world. Coordinatee sub-regions remain pristine arenas
  freed wholesale on dissolution.

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
  cascading.
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
locus AnalystL         : schedule cooperative { ... }   // default
locus MarketDataIngest : schedule pinned      { ... }
```

| Class | Yield discipline | Resource |
|---|---|---|
| **Cooperative** (default) | Yields between substrate cells (handler exit, lifecycle transition, bus dispatch, `time::sleep`, explicit `yield`). Handler bodies are atomic. | Shares a scheduler thread with other cooperative loci. |
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
  different channels to different transports (a market-data
  channel to UDP multicast; a control channel to NATS; a
  test channel to in-memory). The router maintains per-channel
  transport bindings established at deployment time from
  config.
- **Transport adaptation interface.** The runtime defines the
  `Adapter` interface (built-in; standardized in stdlib); any
  transport implementation conforming to it can be plugged in.
  No specific transport ships with the runtime itself.

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
  values, tolerance, diff. Distinct from structural failures
  (panic). See design-rationale §F.9.
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

### Failure & panic handling

- **Panic = framework failure.** Any unrecovered panic in a
  locus body becomes a failure event the parent observes via
  `on_failure`. The default at the root is process exit with
  full stack trace.
- **No exceptions.** Failures are values; recovery is
  parent-policy. Mirrors Erlang's let-it-crash + supervisor.

### Time

- **Monotonic + wall-clock.** `time::now()` and
  `time::monotonic()` are runtime-provided. `time::monotonic()`
  returns a `Duration` (i64 nanoseconds since an unspecified
  reference); only meaningful for elapsed-time differences.
  Backed by `clock_gettime(CLOCK_MONOTONIC)` on both interpreter
  and codegen paths. `time::now()` (wall-clock) is reserved for
  observation and waits on richer `Time` typing.
  Mocking is available for tests via `time::mock_clock(...)`
  (stdlib).
- **Monotonic-only scheduling.** Every scheduling primitive in
  lotus — `time::sleep`, `time::tick`, the cooperative
  scheduler's deadline queue — is grounded on the monotonic
  clock. NTP slewing, leap seconds, and wall-clock jumps cannot
  warp scheduling decisions. `time::sleep` retries on EINTR
  using the kernel's reported remaining time, so a delivered
  signal does not shorten the total sleep. `CLOCK_REALTIME` is
  reserved for `time::now()` (wall-clock observation only) and
  has no scheduling role.
- **Implementation invariant.** Both interpreter and codegen
  paths lower `time::sleep(d)` to
  `clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem)` with EINTR
  retry. The same primitive on both paths means observable
  scheduling behavior is identical regardless of the
  compilation route — important for a system targeting
  trading-grade clock semantics where the substrate cannot
  drift between development and production.

### I/O — minimal

- **stdout / stderr** for `print` / `println`. That's it for
  runtime-level I/O. Files, networking, etc. live in stdlib.

### Process control

- **Exit codes.** `main()` returning `()` exits 0; returning
  `int` exits with that code. Panics exit non-zero.
- **Signal handling.** SIGINT / SIGTERM trigger `drain` →
  `dissolve` on the root locus. Stdlib provides finer-grained
  control if needed.

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
