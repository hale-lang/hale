# Runtime

What the lotus binary always ships with. Always-loaded; not
optional; no `import` needed; the substrate every lotus program
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
  arena becomes available for the next accept.

### Lifecycle

- **Lifecycle dispatcher.** Invokes `birth → run → drain →
  dissolve` per locus; invokes `accept` on coordinatee
  attachment; invokes `on_failure` on child failure with the
  parent's policy.
- **State machine enforcement.** A locus can't accept after
  drain has begun, can't run before birth completed, etc. The
  runtime tracks state; transitions are rejected if they
  violate ordering.
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

### Bus message router

- **Subject → handler dispatch.** Declared `bus subscribe
  "..." as fn` declarations are wired by the runtime at
  startup; inbound messages on declared subjects route to the
  declared handler.
- **Outbound publish.** Declared `bus publish "..."` allows
  emit from any handler return; the runtime routes to the
  configured transport.
- **Transport adaptation.** The runtime defines a `Transport`
  trait (TBD: name; we don't have traits in v0 — call it a
  built-in interface). Stdlib provides implementations
  (NATS, UDP multicast, Unix socket, in-memory). The runtime
  doesn't ship with any specific transport; stdlib does.

### Closure-test infrastructure

- **Accumulator engine.** For each `closure name { ... }`, the
  runtime maintains accumulators for the left and right sides
  of `~~`, scoped per epoch.
- **Epoch management.** `epoch tick`, `epoch duration(...)`,
  `epoch explicit` are runtime-managed: tick increments on
  configurable signal; duration on time elapsed; explicit on
  user `epoch_advance()` call.
- **Band checking + reporting.** At each epoch boundary, the
  runtime checks the closure band and emits a typed
  `ClosureReport` event the application can subscribe to via
  bus.
- **Recovery-event interaction.** `persists_through(...)` and
  `resets_on(...)` clauses are honored at recovery time; the
  accumulator is preserved or zeroed per declaration.

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
  `time::monotonic()` are runtime-provided. Mocking is
  available for tests via `time::mock_clock(...)` (stdlib).

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
  team-specific bindings (e.g. grease's typed messages) live
  as third-party packages. Marshalling helpers in stdlib.
- **Hot-reload of code (not just perspectives).** Erlang
  supports module-level hot reload. Lotus's perspective
  hot-reload is more granular and addresses most of the use
  case; full code hot-reload may not be needed.
- **Determinism mode for tests.** Discussed in `testing.md`;
  runtime needs to support deterministic scheduling when
  requested. The cooperative scheduler makes this easier than
  M:N would have — single-scheduler test mode is fully
  deterministic by construction.
