# Bounded bus queue + backpressure (GH #125)

Status: **v1 + cross-pool (mailbox) landed** (2026-06-11). Design + the
shipped slices for bounding the bus dispatch paths, so a producer that
outruns its consumer can't grow resident memory without limit.

> **Cross-pool backpressure landed (any → pinned).** The per-pinned-locus
> **mailbox** (`lotus_mailbox_*`) is now bounded at the same cap. It has a
> *single consumer* (the pinned locus's own thread), so a cross-thread
> producer that hits the cap **blocks on a `not_full` condvar** until the
> consumer drains a slot — the clean textbook backpressure the scope
> called for. The self-publish deadlock case (a handler publishing to its
> own mailbox during its own drain) is detected via the existing
> `g_current_pinned_mailbox` TLS and grows instead of blocking. An
> any → pinned 2M flood: ~1 GB → **54 MB**, all delivered, no deadlock.
> *Still remaining:* the cross-**cooperative**-pool path (F.31 per-pool
> queues, multiple drainers) — harder, since there's no single consumer
> thread to wait on; left growing for now.

> **v1 shipped.** `lotus_bus_queue_enqueue` (lotus_arena.c) now bounds the
> queue at `LOTUS_BUS_QUEUE_CAP` cells (default 8192 ≈ 4.5 MB, env-override,
> floor 64). Past the cap, a **single-threaded** publisher BLOCKS by
> inline-draining the queue (running the oldest handlers) to make space —
> the only way to bound a flooding producer in a cooperative pool, since the
> producer is the only thread that can drain. The multithreaded (locked)
> path still grows (cross-pool backpressure = follow-on, since inline-drain
> there would run a handler on a foreign pool's thread). #125 flood:
> **1042 MB → 54 MB**, all 2M messages still delivered.
>
> **Perf lesson (the one trap):** draining *one* cell + the loop's
> compaction per publish memmoves the whole live array every enqueue —
> O(cap), a measured **350x** regression on `bus_dispatch`. Fixed by
> inline-draining a **batch down to a ¼-cap watermark** so the compaction
> amortizes (~⅓ cell/enqueue). Net: `bus_dispatch` went **8.7 ms → 3.0 ms**
> (the bounded 4.5 MB queue is far more cache-friendly than the old 55 MB
> one) and maxrss 57 → 9.6 MB. All other bus benches unchanged.

## The problem, precisely

The cooperative bus queue (`lotus_bus_queue_t`, `lotus_arena.c:4075`) is a
growable ring of ~552-byte cells (inline payload ≤512 B + header), initial
cap 64, **doubled on full** (`lotus_bus_queue_enqueue`, line ~4177) —
**unbounded**. A producer that publishes a large batch before the consumer
runs buffers the entire backlog: `birth()` publishing 2M ticks → ~1 GB
(2M × ~552 B), measured in #125.

**Not a leak** — the per-dispatch reclaim works (a queue held at depth ~1
stays flat at 54 MB across 50k…2M messages, #125 comment). It's purely the
queue high-water mark. So this is **flow control**, not reclamation.

## The precedent: `shm_ring` `on_overflow`

`lotus_shm_ring.c:81` already defines the policy vocabulary, surfaced as a
binding kwarg (`shm_ring(..., on_overflow: <policy>)`, AST `OverflowPolicy`
Block=0 / Drop=1 / Fail=2):

- **Block** — back-pressure the publisher until space frees.
- **Drop** — drop the message, publisher proceeds (lossy streams).
- **Fail** — surface overflow as a fault.

The in-process bus should reuse this vocabulary.

## The cooperative-pool wrinkle (the crux)

Block means different things depending on where the publisher runs relative
to the consumer:

- **Cross-pool** (publisher on pool A, consumer on pool B — different
  threads, the queue's mutex already mediates): block = the publisher
  thread **waits on a condvar** until the drain frees a slot. Clean, no
  deadlock.
- **Same-pool** (publisher + consumer share the one cooperative thread, no
  preemption): the publisher *is* the only thread that can drain, so it
  can't "wait" — block = **inline-drain**: when the queue is full, the
  publish call runs queued handlers itself until a slot frees, then
  enqueues. This is how a flooding `birth()` gets bounded — there's no
  other way without preemption.

**Inline-drain re-entrancy** is the implementation risk. If the publisher
is itself a *handler* (publish-from-handler) and it overflows, inline-drain
runs **nested** handlers → stack recursion. Mitigations:
- In steady state this doesn't arise — the scheduler's main loop drains
  between handlers, so a handler that enqueues a few cells never sees a full
  queue; the queue only fills on a *flood from one call*.
- For the flood-from-`birth()` case, inline-drain is iterative (one frame
  above the drain loop) — fine.
- For the pathological "one handler publishes > cap messages in a single
  call under block," cap the nested-drain depth with a re-entrancy guard;
  beyond it, fall back to **fail** (a single thread genuinely cannot both
  block and make progress — honest failure beats deadlock). Document it.

Drop / Fail have no deadlock concern in either regime.

## Default behavior — bounded + block

Today's default (unbounded) is the wrong default for a reliable bus.
Recommend: **bounded + block by default** — a producer that outruns its
consumer is back-pressured rather than allowed to consume all memory. This
fixes #125 by construction. It's a behavior change (a flooding `birth()`
now inline-drains instead of buffering), but it preserves correctness
(every message still delivered, just paced) and only changes the timing /
memory profile — for the better.

Drop is opt-in (lossy telemetry / market-data streams, matching the
`shm_ring` broadcast shape); Fail is opt-in (a "this must never back up"
invariant).

## Syntax surface

The queue is **per-pool** (the cooperative scheduler's queue), so the cap
is a pool attribute, not per-locus or per-subject. Staging the surface:

1. **Global default + env override** (v1, no new syntax) — a compile-time
   default cap (e.g. 4096 cells ≈ 2 MB) + `LOTUS_BUS_QUEUE_CAP` /
   `LOTUS_BUS_OVERFLOW=block|drop|fail` env overrides. Mirrors the existing
   `lotus_bus_udp_bufsize` env-config pattern. Fixes #125 immediately.
2. **Per-pool override** (follow-on) — when a workload needs heterogeneous
   caps. Natural home is the per-pool config; since pools are named in
   `cooperative(pool = X)` and the queue is shared by all loci on that pool,
   a pool-level declaration is cleaner than hanging it off one
   `placement` entry. Defer until a concrete need (the global default +
   env covers the common case).

## Staging

1. **Bounded queue + block, global default + env override.** — **LANDED.**
   Cap check + same-pool inline-drain (batched to a ¼-cap watermark) +
   re-entrancy depth guard with grow-fallback, in `lotus_bus_queue_enqueue`.
   Multithreaded keeps growing (the cross-pool condvar is deferred to a
   follow-on, since inline-drain can't run a foreign pool's handler). Test:
   `bus_backpressure.rs` (a 2M flood stays < 200 MB). Fixes #125, no syntax
   change.
2. **Drop + Fail policies** (env-selectable global), reusing the
   `on_overflow` vocabulary; drop-oldest (evict head) vs drop-newest
   (reject the publish) — default drop-newest (simpler, no consumer-visible
   reordering).
3. **Per-pool / per-subject syntax** when a heterogeneous-cap use case
   appears.

## Interactions

- **Static backpressure check (GH #18 item 4, #48)** is the *compile-time*
  complement — it flags graph shapes that can back up; this is the
  *runtime* bound. The runtime fail/drop counters could feed a diagnostic
  the static check references.
- **Resource budgets (item 5)** could count/declare bus-queue caps as a
  tracked resource (a `bus_queue_bytes` ceiling).
- **Memory-bound warning (item 1)** correctly does *not* flag this — the
  queue isn't a user allocation site — which is what surfaced it as
  separate (#125).

## Risks

- **Inline-drain re-entrancy / recursion** (above) — the main implementation
  hazard; the depth guard + fail fallback is the mitigation, and it needs a
  test (a handler that publishes a large batch under block).
- **Default-cap tuning** — too small thrashes (frequent inline-drains); too
  big delays backpressure. 4096 cells (~2 MB) is a starting point; make it
  env-tunable and measure.
- **Behavior change** — a flooding `birth()` now paces instead of buffering.
  Correct, but call it out in the changelog; the GenMC bus-queue model
  (item 2, #53) should be re-run against the bounded variant.
