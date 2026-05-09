# Lotus — session checkpoint

**Read this first** if you're picking up the lotus language work in a
new session. State as of m30 (fixed-size arrays + for-over-array)
— a surface-completeness pass following the m28 scheduler arc and
the m29 match-guards polish. The full substrate arc
landed: m19→m23 (region allocator with
rich/chunked/recognition strategies + per-locus arenas + bus
copy semantics), m24 (`match` codegen), m25 (bimodal
schedule-class annotation), m26 (cooperative scheduler
semantics — deferred bus dispatch + drain loop), m26b (explicit
`yield`), m27 (pinned-thread spawning via pthread_create,
run-only), m28a (full pinned lifecycle —
birth/run/drain/dissolve all on the pinned thread), m28b
(cross-thread bus mailboxes — pinned loci can subscribe and
publish; cells route via per-locus mutex+condvar mailboxes with
inline payloads; coordinated shutdown via
shutdown-flag-then-join), m28c (optional `: schedule
pinned(core = N)` for `pthread_setaffinity_np` core pinning),
m29 (match arm guards — `pat if cond -> body` lowering with
binding installed for the guard expr to reference), m30
(fixed-size arrays — `[T; N]` literal, `arr[i]` indexing,
`for x in arr` iteration, arena-backed storage). **29 of 30
examples build to native ELF — every single-binary example.**
Only `trellis-pair` (multi-binary, cross-process bus) remains.

**The bimodal scheduler is fully complete.** Cooperative loci
yield between substrate cells via the inline-payload deferred
queue; pinned loci own their thread, run their full lifecycle
(including subscribed bus handlers via per-locus mailboxes),
and can pin to a CPU core. Both layers stay arena-lock-free —
the substrate cost lives at the boundary (the queue/mailbox
mutex + the cell's two memcpy's).

**The Design / lotus is now visible at the codegen substrate.**
Same source, two execution shapes (cooperative / pinned) and
three memory shapes (rich / chunked / recognition), all
expressed as locus annotations. Substrate-invariance applied
to time was kept honestly **bimodal** — no third "greedy"
class, since cooperative already guarantees handler-atomicity
and anything beyond that means leaving the shared scheduler =
own thread = pinned. (Memory has more genuine intermediate
ground than time does, so projection class stays three-way.)

Two prior-session design decisions still drive the bus arc:
runtime owns kernel-level transports (shared memory / AF_UNIX
/ TCP / UDP), stdlib owns protocols on top (NATS / MQTT /
gRPC / TLS); cardinality (SPSC/SPMC/MPSC/MPMC) is emergent
from locus connectivity at link time, not a runtime config.
Both documented below.

This is part of the alpha-conjecture program (see
`~/notes/alpha-conjecture/CLAUDE.md`). Lotus is the language-substrate
arm — a programming language whose primitives are the framework's
coordination primitives.

## Where we are

A working compiler that **runs** lotus programs end-to-end (tree-
walking interpreter) AND **produces** native ELF binaries (LLVM via
inkwell) for a substantial subset including loci with `run()` and
parent-child `accept()` lifecycle methods. 91 tests pass across
the workspace.

```
$ lotus run examples/02-parent-child/main.lt    # interpreter path
greeting from child: hello
greeting from child: hi
greeting from child: yo

$ lotus build examples/02-parent-child/main.lt  # codegen path
built: examples/02-parent-child/main
$ ./examples/02-parent-child/main
greeting from child: hello
greeting from child: hi
greeting from child: yo
```

Phase status:
- **Phase 0** (spec stabilization) — complete
- **Phase 1** (lex / parse / typecheck) — complete; F.1–F.18 enforced
- **Phase 2 v0** (interpreter + bus router) — 21 of 22 example
  projects execute end-to-end via `lotus run` (only multi-binary
  trellis-pair waits on cross-process bus)
- **Phase 3 milestone 30** (arrays) — complete. New
  `LotusType::Array(elem, N)`; fixed-size `[T; N]` lowers to
  arena-allocated `[N x T]` storage. `arr[i]` indexing, `for x
  in arr` iteration, and arrays-as-fn-params all work; element
  type is inferred from the literal's first element. Empty
  array literals + variable-size arrays remain rejected (need
  a type ascription / element-type carrier the literal-only
  path doesn't carry). Per The Design / lotus, the arena's
  wholesale-free shape is the reason arrays are fixed-size in
  v0: dynamic Vec would need either reallocation under a
  separate growth policy or a fundamentally different lifetime
  story. New `examples/21-arrays/` covers indexing, for-loop,
  and arrays as fn parameters.
- **Phase 3 milestone 28c** (pinned CPU-core affinity) —
  complete. `: schedule pinned(core = N)` syntax parses through
  to a `pthread_setaffinity_np` call right after pthread_create.
  ScheduleClass::Pinned grew to `Pinned(Option<i64>)`; the
  parser recognizes optional `(core = N)` after `pinned`; the
  C-runtime helper `lotus_set_core_affinity` wraps the syscall
  behind a stable signature so codegen doesn't have to know the
  cpu_set_t layout. Best-effort: if the requested core doesn't
  exist or the call is denied, the runtime silently falls back
  to ordinary OS scheduling. New `examples/20-pinned-core/`
  pins two workers to cores 0 and 1. Per The Design / lotus,
  this is a refinement WITHIN pinned, not a third mode —
  bimodality holds.
- **Phase 3 milestone 28b** (cross-thread bus mailboxes) —
  complete. Pinned loci can now declare `bus subscribe` and
  publish to cross-thread subjects; the gate is fully lifted
  (only `accept()` and closures remain pinned-incompatible —
  both require cross-thread cascade/violation routing that's
  separate from the mailbox post-and-continue m28b
  delivers). Stage 1 refactored bus queue cells to carry
  inline payloads (with a `pthread_mutex_t`) so the queue is
  the single point of cross-thread synchronization — each
  per-locus arena stays single-threaded territory. Stage 2
  added `lotus_mailbox_t` (a bounded ring buffer with
  mutex+condvar+shutdown flag), grew the bus entry struct
  to `{subject, self, handler, mailbox}`, taught
  `bus_dispatch` to route by `entry.mailbox`
  (null → cooperative global queue; non-null → pinned
  mailbox), and grew the synthesized
  `__pinned_main_<Locus>` body with a mailbox loop between
  `run()` and `drain()`. Coordinated shutdown:
  deferred-dissolve flush calls `lotus_mailbox_shutdown` →
  pthread_join → arena/mailbox destroy. Per The Design /
  lotus, the substrate cost lives at the layer boundary
  (the mailbox lock + the inline payload's two memcpy's),
  not inside either layer's arena. Bimodality holds. New
  `examples/19-pinned-bus/` exercises a cooperative
  publisher feeding a pinned subscriber across threads.
  v0 limit: payloads above 512 bytes drop silently;
  trellis-grade typed messages are well under this.
- **Phase 3 milestone 28a** (pinned full lifecycle on the pinned
  thread) — complete. m27's "run-only" gate is lifted: pinned
  loci can now declare birth / run / drain / dissolve, and the
  full sequence executes on the pinned thread, in order. Codegen
  synthesizes a per-locus `__pinned_main_<LocusName>(self_ptr)
  -> ptr` whose signature matches pthread's start-routine
  contract directly; `pthread_create` gets that function pointer
  with `self_ptr` as its argument. The C-side `lotus_thread_entry`
  adapter and the `(fn, self_ptr)` args struct are gone — the
  generated thread_main calls each declared lifecycle method in
  sequence, returns null. `flush_dissolve_frame` short-circuits
  drain / dissolve for pinned entries (those already ran on the
  pinned thread); main thread's only remaining work is the
  pthread_join + arena_destroy. v0 m28a still gates: pinned
  loci cannot declare `accept()` (cross-thread cascade
  dissolves), bus subscribe / publish (cross-thread mailbox),
  or closures. Those wait on m28b. New
  `examples/18-pinned-lifecycle/` exercises the full lifecycle
  with a 30ms sleep in `run()` so the main thread races past
  before the pinned thread reaches `run`'s body — proves real
  parallelism + correct ordering of all four methods on the
  pinned thread.
- **Phase 3 milestone 27** (pinned threads, run-only) —
  complete. Pinned-class loci spawn a real pthread at
  instantiation: codegen arena-allocates a `(run_fn, self_ptr)`
  tuple, calls `pthread_create` with the C-runtime adapter
  `lotus_thread_entry` as the start routine, and defers
  `pthread_join` to the deferred-dissolve flush via a new
  optional `thread_id_alloca` field on frame entries (parallel
  to cooperative long-lived's None-tagged entries). pthread_join
  blocks until run() returns; arena destroy follows.
  Linker now passes `-lpthread` unconditionally. v0 m27 scope:
  pinned loci can declare ONLY `run()` — no birth/drain/dissolve,
  no bus subscribe/publish. Codegen errors clearly otherwise.
  Full pinned lifecycle on the pinned thread + cross-thread
  bus mailbox (the "any → pinned" post-and-continue side of
  cross-class semantics) wait on m28.
  `examples/16-schedule-classes/` updated to actually exercise
  the new substrate: PinnedWorker.run() does a 50ms
  `time::sleep` so the main thread's println races deterministically.
  Output ordering "cooperative ... / main: spawned both / pinned
  ran on its own pthread" demonstrates the parallelism.
- **Phase 3 milestone 26b** (explicit `yield` primitive) —
  complete. `yield` lifted from reserved keyword to a real
  statement. Codegen lowers `yield;` to a call to
  `lotus_bus_queue_drain` at this point — pending substrate
  cells fire mid-body. Interpreter treats it as a no-op
  (single-threaded synchronous dispatch — no queue to drain).
  Per spec/runtime.md cooperative yield points: "explicit
  `yield` (rare, for long-running computations)" — the
  implicit yield points (handler exit, lifecycle transition,
  bus dispatch) cover most cases; `yield` is the escape hatch
  for long-internal-loop bodies. New `examples/17-yield/`
  exercises the primitive end-to-end.
- **Phase 3 milestone 26** (cooperative scheduler semantics) —
  complete. Bus dispatch is now deferred: each `<-` enqueues
  `(handler, self, payload_copy)` cells onto a program-wide
  FIFO queue (`@lotus.bus_queue.global`) instead of running
  handlers inline. The C-runtime drain loop pops cells one at
  a time and invokes the handler — handler-atomic per substrate
  cell, with cooperative yields between cells rather than
  nested call frames. Handlers may publish more events; drain
  continues until empty. Drain runs at the start of every
  `flush_dissolve_frame` so cooperative subscribers process
  pending cells before they themselves dissolve. v0 limitation:
  cells enqueued during dissolves are leaked (subscriber gone).
  trellis-demo + 05-bus output unchanged from sync-nested days
  — interleaving naturally produces the same observable order
  for these examples (kernel multipliers all 1.0; 05-bus is a
  linear two-stage chain). Spec/runtime.md updated;
  spec-aligned per "cooperative yield points: between handler
  invocations, between lifecycle transitions, on bus dispatch."
- **Phase 3 milestone 25** (schedule-class annotation
  infrastructure, bimodal) — complete. New keywords
  `schedule`, `cooperative`, `pinned` in lexer (no `greedy` —
  see preamble); `LocusAnnotation::Schedule(ScheduleClass)` in
  AST; parser recognizes the `: schedule X` annotation alongside
  `tier N` and `projection X`; typechecker stores it on
  `Annotations`; codegen resolves it onto
  `LocusInfo.schedule_class` (default cooperative). Runtime
  today still runs everything synchronously — no semantic
  branch on the class yet. m26 will introduce deferred bus
  dispatch + a scheduler loop on the main thread; m27 spawns
  dedicated threads for pinned loci.
  `examples/16-schedule-classes/` exercises both classes;
  spec/runtime.md gets a "Schedule classes" section
  documenting the surface, the explicit bimodality reasoning
  ("Why no greedy class"), and the implementation status.
- **Phase 3 milestone 24** (`match` expressions) — complete.
  Match statements lower to LLVM as a chain of test-blocks +
  body-blocks, falling through to the next arm on mismatch.
  Patterns supported: `Literal` (Int / Bool / Duration / Float /
  Decimal), `Wildcard`, and `Binding(x)` (binds the scrutinee to
  `x` for the arm body, with shadow/restore of any prior local
  with the same name). `Tuple` / `Constructor` patterns + arm
  guards remain interpreter-only. F.18 exhaustiveness is
  enforced upstream by the typechecker, so the post-arms
  fallthrough block is unreachable for well-typed programs.
  Match arm bodies handle `Call` exprs by routing through
  `lower_stmt` (so `println` / void-returning user fns work
  identically to statement-position calls). New
  `examples/15-match/` exercises Int + Bool + Binding patterns.
- **Phase 3 milestones 22 + 23** (per-projection-class arena
  strategies) — complete. Each locus's projection class
  resolves from `: projection <class>` annotation or per-spec
  default rule (chunked if accept declared, rich otherwise) at
  declare-locus-struct time. m22 wires chunked parents through
  `lotus_arena_create_subregion`: each accepted child gets a
  sub-region carved from the parent's bookkeeping space, with
  slot indices reused via a free-list when children dissolve.
  m23 lights up the recognition annotation behind the same
  sub-region path — the pre-allocated bitmap-cell pool
  optimization is deliberately deferred until a workload
  exercises it, and that gap is documented in
  `spec/memory.md`. New `examples/14-projection-classes/`
  exercises all three classes end-to-end.
- **Phase 3 milestone 20** (locus-owned arenas + bus copy
  semantics) — complete. Every locus struct now carries a
  synthetic `__arena: ptr` field at struct slot 0; instantiation
  fills it via `lotus_arena_create()`; the per-locus arena is
  wholesale-freed via `lotus_arena_destroy` after `dissolve()`
  runs (both the ephemeral path and the deferred long-lived
  flush). Allocations route through three tiers: an explicit
  override (used during locus-instantiation field init so
  composite-default literals land in the new locus's arena), the
  enclosing locus's arena field (when `current_self` is set), or
  the program-wide arena (`@lotus.arena.global`, used in `main`
  and free fns). Bus dispatch implements the spec's "typed
  message crossing a locus boundary is a copy, not a pointer"
  rule: each `<-` passes the payload's compile-time size to
  `lotus.bus_dispatch`, which allocates `size` bytes in each
  matching subscriber's arena (loaded from `self_ptr + 0`,
  the fixed arena-field offset), memcpy's the payload, and
  passes the COPY to the subscriber's handler. Trellis-demo's
  `self.current_kernel = msg` pattern now actually works under
  per-locus arenas — subscriber's stored copy outlives publisher
  locus dissolution.
- **Phase 3 milestone 19** (region allocator substrate) —
  complete. The codegen path now links a small C arena runtime
  (`crates/lotus-codegen/runtime/lotus_arena.c`, bundled into the
  compiler via `include_str!`) into every emitted binary. ABI:
  `lotus_arena_create()` / `lotus_arena_alloc(arena, size, align)`
  / `lotus_arena_destroy(arena)`. An arena is a linked list of
  bump chunks (default 64 KiB; oversized requests get a fresh
  chunk sized to fit); allocation is pointer-bump in the head
  chunk, destruction walks + frees wholesale.
- **Phase 3 milestone 18** (codegen subset). **17 of 18 example
  projects build to native ELF — every single-binary example.**
  Modes (lowered as locus methods named bulk/harmonic/resolution;
  callable via `self.<mode>()`), built-in `self.children` array
  (fixed-cap 16, embedded after user fields on every locus that
  declares accept; appended at accept dispatch + counter bumped),
  `for child in self.children { ... }` lowered as an indexed
  loop with the var bound as a LocusRef-typed local, and locus
  literals in expression position so `let _l1 = LeafL { ... }`
  works. Interpreter parity: replaced the m10 dedup-pop with a
  `dissolved: Cell<bool>` flag on LocusHandle so ephemeral
  handles stay in parent.children (for `for child in
  self.children`) but the parent's later cascade skips
  already-dissolved children.
- **Phase 3 next** — `trellis-pair` (multi-binary, cross-process
  bus + entry-point selection) is now the only example
  remaining. The substrate is in good shape: full bimodal
  scheduler with cross-thread bus, per-projection-class arenas,
  cooperative deferred dispatch + explicit yield, pinned threads
  with full lifecycle + mailboxes + core affinity. trellis-pair
  needs `lotus build --bin <Locus>` entry-point selection plus a
  cross-process bus transport (decided last session: shared-
  memory ring buffer, per the runtime/stdlib transport split
  documented below). It also exercises pieces still
  interpreter-only: module / `import` resolution, `perspective`
  declarations with `is_stable()`, and tick-epoch closures.

## Transport layering (decided 2026-05-08)

Runtime / stdlib split for bus transports:

- **Runtime owns kernel-level IO primitives** + thin `Transport`
  adapters that wrap them: shared memory (`shm_open` + `mmap` +
  atomic indices), Unix domain sockets (`AF_UNIX`), TCP/UDP
  (`AF_INET` + multicast). Direct syscall plumbing wired into
  the bus router. `io_uring` / `epoll` / `kqueue` integration
  also lives here when the cooperative scheduler lands.
- **Stdlib owns protocols on top of those primitives**:
  `std::bus::nats` (NATS frames over TCP), `std::bus::mqtt`,
  `std::bus::http_sse`, `std::bus::grpc`. TLS lives in
  stdlib too (`std::tls`); serialization (json/protobuf/msgpack)
  in `std::encoding`.

This matches `spec/runtime.md`'s "transport-agnostic" framing —
runtime defines the `Adapter` interface, specific protocols
plug in from stdlib. The new clarification is that the runtime
*also* directly exposes the kernel primitives those protocols
need, rather than forcing every adapter to vendor its own
syscall wrappers.

## Producer/consumer cardinality is emergent (insight, 2026-05-08)

The standard MPSC / SPSC / SPMC / MPMC taxonomy doesn't
describe a transport configuration — it describes
**locus connectivity** on a subject. Count the loci with
`publish "X"` and the loci with `subscribe "X"` at link time:

| Publishers on X | Subscribers on X | Required machinery |
|---|---|---|
| 1 | 1 | SPSC — wait-free, no claim ticket |
| 1 | N | SPMC — Disruptor's natural shape |
| N | 1 | MPSC — fan-in queue with producer claim |
| N | N | MPMC — atomics on both sides |

In trellis-demo all subjects are SPSC / SPMC; **no MPMC
machinery needed**. That's a real speedup vs a uniform
"every subject is MPMC" runtime — SPSC rings are 5-10x faster
than MPMC ones.

The current `BusRouter` doesn't exploit this — it's uniform
MPMC-shaped. When the substrate gets more serious, per-subject
specialization is exactly the kind of optimization the
framework's coordination primitives unlock that a general
pub-sub library can't: **the locus surface carries the shape
information; the substrate gets to specialize.** Connects to
F.14 (three-way interface: locus + parent + contract) — the
contract surface declares data flow shape; bus declarations
declare connectivity shape; together that's enough to pick
the cheapest correct primitive.

## Codegen milestone arc (Phase 3 progress)

Each milestone below is one focused commit + a CHECKPOINT/README
refresh. The arc moved fast: nineteen milestones (m0–m18)
landed across two sessions in 2026-05-08, taking the codegen
path from "no-op stub" to "every single-binary example is a
build target." Each load-bearing piece was intentionally narrow:

```
m0  Phase 3 milestone 0: lotus build → native ELF via LLVM      (77b977f)
m1  Codegen milestone 1: Int / Float / Bool params + println    (5c9b6f7)
m2  Codegen milestone 2: let + Int/Float arithmetic + cmp       (5224d53)
m3  Codegen milestone 3: let mut + assignment                   (03c2f55)
m4  Codegen milestone 4: if / while / break / continue          (cae8c9a)
m5  Codegen milestone 5: time::sleep on CLOCK_MONOTONIC         (929efa2)
m6  Codegen milestone 6: multi-fn programs                      (9955bea)
m7  Codegen milestone 7: locus runtime ABI    ← load-bearing    (206fbd0)
m8  Codegen milestone 8: accept() + parent-child wiring         (d5afffd)
m9  Codegen milestone 9: time::monotonic() + Duration arith     (cdd7353)
m10 Codegen milestone 10: drain() / dissolve() lifecycle        (3ba3e05)
m11 Codegen milestone 11: user `type` decls + struct literals   (5cb4882)
m12 Codegen milestone 12: bus router (subscribe + <- + deferral)(5645eaa)
m13 Codegen milestone 13: self.method() calls                   (b036c7f)
m14 Codegen milestone 14: Decimal + return-from-main exit code  (b036c7f)
m15 Codegen milestone 15: closures (collapse-only path)         (9bf21c1)
m16 Codegen milestone 16: Time + composite defaults + heap lits (e33e8ee)
                          ⇒ trellis-demo builds to native ELF
m17 Codegen milestone 17: on_failure routing (absorb / bubble)  (4bf84e3)
                          ⇒ 03b / 03c build to native ELF
m18 Codegen milestone 18: modes + self.children + for + locus  (d48df6b)
                          literal in expression position
                          ⇒ 04-modes builds; 17/18 single-binary
                            examples are build targets
m19 Codegen milestone 19: region allocator substrate           (ea4892b)
                          ⇒ libc malloc removed; lotus_arena_*
                            backs every type-literal + ClosureViolation
                            allocation; same example ladder still passes
m20 Codegen milestone 20: locus-owned arenas + bus copy        (d511670)
                          ⇒ __arena field on every locus struct
                            (slot 0), lifecycle-bound; bus dispatch
                            copies payloads between publisher /
                            subscriber arenas per spec
m22 Codegen milestone 22: chunked-class sub-regions            (010db7a)
                          ⇒ chunked parents allocate accepted
                            children via lotus_arena_create_subregion;
                            free-list bookkeeping reuses slot
                            indices as children dissolve
m23 Codegen milestone 23: recognition-class stub               (010db7a)
                          ⇒ recognition annotation parses /
                            resolves / dispatches; behaviorally
                            equivalent to chunked at v0; bitmap-
                            pool optimization deliberately deferred
                          + examples/14-projection-classes
m24 Codegen milestone 24: match expressions                    (bb948c6)
                          ⇒ Literal / Wildcard / Binding patterns
                            in codegen; Tuple / Constructor +
                            guards remain interpreter-only;
                            F.18 exhaustiveness still enforced at
                            typecheck
                          + examples/15-match
m25 Codegen milestone 25: schedule-class annotation infra      (bbe2731 +
                                                                763edf8)
                          ⇒ `: schedule cooperative | pinned`
                            parses, typechecks, resolves on
                            LocusInfo; default cooperative; no
                            runtime semantic branch yet.
                            Bimodal-only: greedy dropped on
                            review as a bimodality violation.
                          + examples/16-schedule-classes
m26 Codegen milestone 26: cooperative scheduler semantics      (9c0ba40)
                          ⇒ bus dispatch deferred via process-
                            wide FIFO queue (lotus_bus_queue_*);
                            drain runs at flush_dissolve_frame
                            entry so subscribers process cells
                            before they dissolve; cells enqueued
                            during dissolves are leaked (v0)
m26b Codegen milestone 26b: explicit `yield` primitive         (6760a44)
                          ⇒ yield lifted from reserved to real;
                            codegen lowers to lotus_bus_queue_drain;
                            interpreter no-op
                          + examples/17-yield
m27 Codegen milestone 27: pinned threads (run-only)            (cc57ee4)
                          ⇒ pthread_create at pinned instantiation;
                            run() executes on its own thread;
                            deferred pthread_join at scope exit;
                            -lpthread linked unconditionally;
                            v0 scope: pinned loci must be run-only
                            (no other lifecycle, no bus)
m28a Codegen milestone 28a: pinned full lifecycle              (c70b551)
                          ⇒ pinned loci can declare birth/run/
                            drain/dissolve, all run on the pinned
                            thread in order; synthesized per-locus
                            __pinned_main_<Locus> matches pthread
                            start-routine signature directly (no C
                            adapter, no args struct); flush skips
                            drain/dissolve for pinned entries
                            (already ran on the thread)
                          + examples/18-pinned-lifecycle
m28b/1 m28b stage 1: inline-payload bus queue + mutex          (8f8d20d)
                          ⇒ queue cells carry [u8; 512] inline
                            payload; pthread_mutex_t guards cell
                            array; drain copies inline →
                            subscriber arena before invoking
                            handler. Prereq for cross-thread
                            bus: queue is the single sync point;
                            arenas stay single-threaded.
m28b/2 m28b stage 2: per-pinned mailbox + dispatch routing     (fe296ae)
                          ⇒ lotus_mailbox_t (mutex+condvar+
                            shutdown flag); bus entry grows
                            mailbox field; dispatch routes by
                            entry.mailbox null/non-null;
                            synthesized __pinned_main_<Locus>
                            grows mailbox loop between run()
                            and drain(); coordinated shutdown
                            via shutdown-flag-then-join
                          + examples/19-pinned-bus
m28c   Codegen milestone 28c: pinned(core=N) affinity          (5b10337)
                          ⇒ ScheduleClass::Pinned(Option<i64>);
                            parser optional (core=N); C-side
                            lotus_set_core_affinity wraps
                            pthread_setaffinity_np; codegen
                            calls it after pthread_create when
                            core is set; best-effort fallback
                          + examples/20-pinned-core
m29    m29: match arm guards in codegen                        (0398d42)
                          ⇒ pattern → guard_bb (binding install
                            + guard eval + cond branch) → body;
                            falls through to next arm on false;
                            extends m24 surface
                          + examples/15-match (extended)
m30    m30: arrays — literal + indexing + for-over-array       (2bc3fbb)
                          ⇒ LotusType::Array(elem, N); fixed-
                            size [T; N] only; arena-backed
                            storage; arr[i] indexing; for x in
                            arr lowers to indexed loop; arrays
                            pass through fn params (as ptrs)
                          + examples/21-arrays
m30b   m30 follow-up: indexed local-array assignment           (78ea6e7)
                          ⇒ `arr[i] = v` lowers via GEP-into-
                            local-array-storage + store; rest
                            of LValue surface unchanged
                          + examples/22-moving-average (real
                            flex: bus-driven sliding-window
                            mean over a [Int; 4] state array)
m31    m31: integer ranges in for-loop iterators               (2e7cb06)
                          ⇒ Expr::Range { lo, hi, inclusive }
                            in AST; parser tail-attaches at
                            lowest precedence; for-stmt
                            handlers (interp + codegen) special-
                            case Range as a counted loop; range
                            outside iterator position rejects
                          + examples/23-ranges
m32    m32: default fn param values (free fns)                 (d211c60)
                          ⇒ Defaults must form a suffix; caller
                            may omit trailing args; default expr
                            evaluates at the call site in the
                            caller's scope. Locus methods still
                            reject — m32 is free-fn-only.
                          + examples/24-default-params
m33    m33: import resolution for multi-file projects          (pending)
                          ⇒ CLI's parse_with_imports walks the
                            entry's `import "..."` directives,
                            recursively parses each, dedups by
                            canonical path, merges items into
                            one logical Program. Paths resolve
                            relative to importing file's dir
                            with .lt extension implicit. Cycles
                            short-circuit. Both `lotus run` and
                            `lotus build` use the merged Program
                            for single-file targets.
                          + examples/25-imports
```

The architectural pivots are **m7** (locus → LLVM struct,
lifecycle methods take `self_ptr`, `self.X` via GEP) and **m8**
(accept's child param as `LotusType::LocusRef(String)`,
parent-aware child instantiation, F.7 dispatch ordering).
Everything before m7 was scalar-only fn-bodies; everything after
m7 builds on the struct ABI.

## What runs vs. what builds

| Primitive | Interpreter | Codegen |
|---|---|---|
| `fn main()` entry | ✅ | ✅ |
| Int / Float / Bool / String literals + params | ✅ | ✅ |
| `let` bindings | ✅ | ✅ |
| Arithmetic, comparisons, logical ops | ✅ | ✅ |
| `self.X` reads (in lifecycle methods) | ✅ | ✅ (runtime GEP+load) |
| Locus instantiation + `birth()` | ✅ | ✅ (ephemeral only) |
| Mixed-type println (single printf) | ✅ | ✅ |
| `let mut` + assignment (incl. compound `+=` etc.) | ✅ | ✅ |
| `if` / `else` / `else if` / `while` + `break` / `continue` | ✅ | ✅ |
| `time::sleep` on CLOCK_MONOTONIC + EINTR retry | ✅ | ✅ |
| `time::monotonic()` + Duration ± Duration / cmp | ✅ | ✅ |
| User-defined fns called from main / each other | ✅ | ✅ |
| `run()` lifecycle method | ✅ | ✅ |
| `self.X = ...` mutation in lifecycle methods | ✅ | ✅ |
| `accept()` lifecycle method (F.7 ordering) + child `g.X` reads | ✅ | ✅ |
| `drain()` / `dissolve()` lifecycle methods (F.4 cascade) | ✅ | ✅ |
| User `type` decls + struct literals + field reads | ✅ | ✅ |
| Locus `fn` members (called from bus dispatch, etc.) | ✅ | ✅ |
| Bus router (`<-` send + subscribe dispatch) | ✅ | ✅ |
| Long-lived locus deferred drain/dissolve (subscribers) | ✅ | ✅ |
| `self.method()` calls inside lifecycle / fn bodies | ✅ | ✅ |
| `Decimal` type + arithmetic + comparisons (f64 v0) | ✅ | ✅ |
| `return n;` from main → process exit code | ✅ | ✅ |
| Closures: collapse on pass, exit-non-zero on fail | ✅ | ✅ |
| Closures: parent absorb / bubble routing (F.9) | ✅ | ✅ |
| Built-in `ClosureViolation` type (locus/closure/diff fields) | ✅ | ✅ |
| Modes (`mode bulk()` etc.) + self-method dispatch | ✅ | ✅ |
| `self.children` (fixed-cap array on accept-declaring loci) | ✅ | ✅ |
| `for child in self.children { ... }` iteration | ✅ | ✅ |
| Locus literals in expression position (`let l = L { }`) | ✅ | ✅ |
| Time literals + Time as a typechecked primitive | ✅ | ✅ (string-spelling v0) |
| Composite locus param defaults | ✅ | ✅ |
| Nested field reads (self.x.y, expr-receiver-of-Field) | ✅ | ✅ |
| Heap-allocated user-type literals (escape via bus) | ✅ | ✅ |
| Contracts (typecheck only — F.8) | ✅ | ✅ (skipped at codegen) |
| `match` (Literal / Wildcard / Binding patterns) | ✅ | ✅ |
| `match` arm guards (`pat if cond -> body`) | ✅ | ✅ |
| `match` (Tuple / Constructor patterns) | ✅ | — |
| Array literals `[T; N]` + indexing | ✅ | ✅ |
| `for x in arr` over fixed-size arrays | ✅ | ✅ |
| Indexed local-array assignment `arr[i] = v` | ✅ | ✅ |
| `for i in lo..hi` / `lo..=hi` range loops | ✅ | ✅ |
| Default fn param values (free fns; suffix-only rule) | ✅ | ✅ |
| Default values on locus methods | ✅ | — |
| `import "..."` resolution (multi-file projects) | ✅ | ✅ |
| Schedule-class annotation (`: schedule cooperative \| pinned`) | — | ✅ (resolved on LocusInfo) |
| Cooperative scheduler (deferred bus + drain loop) | — | ✅ |
| Explicit `yield` primitive | ✅ (no-op) | ✅ (drains queue) |
| Pinned threads (full lifecycle: birth/run/drain/dissolve) | — | ✅ |
| Pinned + cross-thread bus mailbox | — | ✅ |
| Region allocator — per-locus arenas, bus copy semantics | — | ✅ |
| Region allocator — chunked sub-regions + free-list | — | ✅ |
| Region allocator — recognition bitmap-pool | — | — (chunked-equivalent stub) |
| Recovery primitives (bubble) | ✅ | ✅ |
| Recovery primitives (restart / quarantine / reorganize) | parsed | — |

## Locked design commitments (F.1–F.18)

Spec source: `spec/design-rationale.md`. Summary:

- **F.1** k_max = B / [(1−φ)c + φσ] is the framework equation.
- **F.2** `ProjectionClass` as built-in any-of-three constraint.
- **F.3** Per-arena defrag/free-list, no whole-program GC.
- **F.4** `drain()` always cascades depth-first.
- **F.5** Mode projections share the locus's arena.
- **F.6** Lifecycle methods are not implicit loci.
- **F.7** `accept()` runs before child birth.
- **F.8** Contract compatibility type-checked across coordinator /
  coordinatee.
- **F.9** Collapse vs. explosion + parent on_failure routing
  (absorb / bubble).
- **F.10** Mode keywords accepted post-dot as member names.
- **F.11** `self.children` typing and lifecycle.
- **F.12** Bus send is `<-`; subscribe is declarative.
- **F.13** Bus subscription handler signature.
- **F.14** Three-way interface: locus + parent + contract.
- **F.15** Predefined type names are PascalCase, not keywords.
- **F.16** `self.k_max` as built-in computed field (F.1 executable).
- **F.17** Strict field-access; method types on locus / perspective.
- **F.18** Match exhaustiveness checked at typecheck.

## Files to read for orientation

In order:

1. `README.md` — overview, status, F-table, example list, toolchain.
2. `spec/design-rationale.md` — why each construct is shaped the way
   it is. Source of truth for F.1–F.18.
3. `spec/grammar.ebnf` — formal grammar.
4. `spec/tokens.md` — lexical structure.
5. `spec/precedence.md` — operator precedence table.
6. `spec/memory.md` — memory model + the "Codegen ABI (v0)" section
   documenting the locus struct lowering, F.7 dispatch ordering,
   and ephemeral-only constraint (added in m7, extended in m8).
7. `spec/runtime.md` — runtime semantics + the "Time" section
   documenting the monotonic-only-scheduling discipline (m5, m9).
8. `examples/hello-world/main.lt` → `examples/10-stateful-locus/`
   → `examples/trellis-demo/main.lt` — the example ladder.
   06-10 are the codegen-arc demos; trellis-demo exercises the
   full interpreter pipeline.
9. `crates/lotus-syntax/src/lib.rs` — public API of the parser/AST.
10. `crates/lotus-types/src/lib.rs` — typechecker entry + unit
    tests that lock the F.x rules.
11. `crates/lotus-runtime/src/lib.rs` + `eval.rs` + `bus.rs` +
    `builtins.rs` — interpreter, dissolve cascade, bus router,
    `time::sleep` / `time::monotonic` via libc::clock_*.
12. `crates/lotus-codegen/src/codegen.rs` — current LLVM lowering.
    The biggest single file in the workspace; the locus runtime
    ABI is what makes it interesting. Worth a careful read if
    extending codegen.
13. `crates/lotus-codegen/runtime/lotus_arena.c` — the lotus
    region allocator (m19) AND cooperative scheduler queue (m26)
    AND pthread adapter (m27). Bundled into the compiler via
    `include_str!`, written next to each generated `.o` file at
    link time, compiled + linked into the final binary. The
    surface every `arena_alloc` / `bus_queue_*` /
    `lotus_thread_entry` call site in codegen.rs targets.
14. `crates/lotus-cli/src/main.rs` — CLI dispatch (lex / parse /
    check / run / build).
15. `~/.claude/plans/witty-foraging-lightning.md` — the original
    delivery plan to team-wide internal v1.0 (~18–30 months total).
16. `notes/open-questions.md` — tracked deferrals, including the
    spec-vs-impl gap on immutable-binding compile-time
    enforcement (§23).

For broader program context:

- `~/notes/alpha-conjecture/CLAUDE.md` — the master project guide.
  Lotus is one substrate-arm among several; paper 4 is the program's
  foundational anchor (read its memory file too).
- `~/notes/alpha-conjecture/lotus/` — the design-time meta-framework
  that lotus-the-language is the compile-time projection of.

## Strategic preferences locked in

These are user (Riley) directions saved into auto-memory at
`~/.claude/projects/-home-riley-notes-alpha-conjecture/memory/`:

- **Greenfield cleanup as we go** — pre-ship code is greenfield;
  drop "preserved old behavior" / fallback patterns; clean up
  rather than accumulate compatibility cruft. (See
  `feedback_greenfield_cleanup.md`.)
- **Stay focused on lotus** for the foreseeable session — don't
  swing back to paper-4 / theory work without explicit redirect.
- **LLVM is the codegen target** — committed; toolchain installed
  (llvm-18 + clang + lld + libpolly-18-dev). inkwell 0.5 +
  llvm-sys 180.0.0 against system LLVM.
- **Trellis informs but doesn't dictate** — production trellis-pair
  (analyst/executor as separate binaries) is the eventual real-world
  use case, but we're not building specifically toward it. It's a
  milestone we'll hit when the pieces are right; for now,
  `examples/trellis-demo/` is the single-process surrogate that
  exercises the full pipeline.

## User context (Riley)

Junior partner at small finance firm. Deep software-architecture
expertise via brain3 (production deployment at the firm,
brained.dev). The trellis trading system is the natural first
real-world use case for lotus.

## Recent commit history (newest first)

```
c4ec399 codegen: remove dead `_ =>` arm in lower_stmt
0398d42 m29: match arm guards in codegen
5b10337 Codegen milestone 28c: pinned(core=N) CPU-core affinity
fe296ae m28b stage 2: cross-thread bus mailboxes for pinned loci
8f8d20d m28b stage 1: inline-payload bus queue + mutex
c70b551 Codegen milestone 28a: pinned full lifecycle on the pinned thread
1cb4aaa CHECKPOINT.md: session-resume reference
cc57ee4 Codegen milestone 27: pinned threads (run-only)
6760a44 m26b: explicit `yield` primitive
9c0ba40 Codegen milestone 26: cooperative scheduler semantics
763edf8 m25 cleanup: drop greedy from schedule classes (bimodality)
bbe2731 Codegen milestone 25: schedule-class annotation infrastructure
bb948c6 Codegen milestone 24: match expressions
010db7a Codegen milestones 22 + 23: per-projection-class arena strategies
d511670 Codegen milestone 20: locus-owned arenas + bus copy semantics
ea4892b Codegen milestone 19: region allocator substrate
79e839c CHECKPOINT.md: capture transport layering + cardinality insight
b18febb CHECKPOINT.md: update milestone-arc preamble
601c0b7 CHECKPOINT.md: backfill m18 commit hash
d48df6b Codegen milestone 18: modes + self.children + for-loops
4bf84e3 Codegen milestone 17: on_failure routing (absorb / bubble)
e33e8ee Codegen milestone 16: trellis-demo builds to native ELF
9bf21c1 Codegen milestone 15: closures (collapse-only path)
b036c7f Codegen milestones 13 + 14: self.method, Decimal, return-from-main
5645eaa Codegen milestone 12: bus router lowering
5cb4882 Codegen milestone 11: user `type` decls + struct literals
3ba3e05 Codegen milestone 10: drain() / dissolve() lifecycle
cdd7353 Codegen milestone 9: time::monotonic() + Duration arithmetic
d5afffd Codegen milestone 8: accept() lifecycle + parent-child wiring
206fbd0 Codegen milestone 7: locus runtime ABI
9955bea Codegen milestone 6: multi-fn programs
929efa2 Codegen milestone 5: time::sleep on CLOCK_MONOTONIC
```

89 commits ahead of origin/master at checkpoint time.

## Next steps in priority order

Substrate is now in good shape. The remaining v1 work is one
substantial substrate piece (m28) followed by the application
exercise (trellis-pair) that proves it. Everything else is
polish.

**1. m28 — pinned full lifecycle + cross-thread bus mailbox.**
The other half of the bimodal cut. Pinned loci can today only
declare `run()`; m28 lifts that restriction. Specifically:

- Pinned loci can declare birth / drain / dissolve, all running
  on the pinned thread.
- Pinned loci can declare bus subscribe / publish.
- Cross-thread bus dispatch ("any → pinned" per
  `spec/runtime.md::Schedule classes`) posts to a per-pinned-
  locus mailbox via mutex; the pinned-thread event loop polls
  the mailbox between cells. Pinned → any goes through the
  existing program-wide queue (drained on main-thread side).
- Coordinated shutdown: signal pinned thread → drain its
  mailbox → run its drain/dissolve → pthread_join.
- Optional: `: schedule pinned(core=N)` syntax for explicit
  `sched_setaffinity` core pinning.

This is a meaningful threading milestone with real cross-thread
synchronization. Worth designing carefully — particularly the
arena ownership model for cross-thread payload copies. (Today
m20 memcpy's at enqueue time on the publisher's frame; for
cross-thread, the publisher writes into the pinned subscriber's
arena which is otherwise that thread's exclusive territory →
needs either arena-level locking or a cross-thread bounce
buffer.)

**2. trellis-pair** (multi-binary, cross-process bus +
entry-point selection). The only remaining example in the
ladder. Two pieces:
- `lotus build --bin <locus>` entry-point selection
- Cross-process bus transport. Decided last session:
  shared-memory ring buffer (most production-shaped; matches
  the existing in-process LMAX disruptor), per the
  runtime/stdlib transport split documented above.

**Polish (any time):**

- Tuple / Constructor patterns in match (needs tuple values
  in codegen first)
- Default param values on locus methods (free fns work as of
  m32; locus methods need a richer dispatch plumb because
  bus-dispatch / mode-dispatch / `self.method()` calls all
  have different arity stories)
- Recovery primitives execution (restart / quarantine /
  reorganize — interpreter parses, neither runs)
- Recognition-class real bitmap-pool (currently chunked-
  equivalent stub per spec/memory.md)
- Decimal precision tightening (printf %g vs Display)
- Free-fn implicit-locus arenas (spec is fuzzy on
  return-value-copy semantics)

**Long-deferred:**

- Generic instantiation (records args, no substitution)
- Module / import resolution (parsed only)
- Tree-sitter grammar derivation from EBNF
- LSP server
- Self-hosting (Phase 6, distant)

## Toolchain state

System has:

- `llvm-config` 18.1.3 at `/usr/bin/llvm-config`
- `clang` 18.1.3 at `/usr/bin/clang`
- `lld` at `/usr/bin/lld`
- `libpolly-18-dev` (required by llvm-sys for static link)
- `gcc` 13.x

Cargo workspace builds clean. `cargo test --workspace --tests` passes
all 91 tests (the locus-with-run test runs 3×500ms sleeps so the
runtime + codegen integration buckets clock ~1.5s each).

## How to verify the checkpoint

```
cd ~/code/lotus-lang
cargo test --workspace --tests           # 91 passed
cargo run --bin lotus -- run examples/trellis-demo/main.lt
cargo run --bin lotus -- build examples/hello-world/main.lt
./examples/hello-world/main              # prints "hello, world"
rm examples/hello-world/main             # clean up artifact
cargo run --bin lotus -- build examples/01-locus-with-run/main.lt
./examples/01-locus-with-run/main        # tick 0..2 over 1.5s
rm examples/01-locus-with-run/main       # clean up artifact
cargo run --bin lotus -- build examples/02-parent-child/main.lt
./examples/02-parent-child/main          # 3× "greeting from child: ..."
rm examples/02-parent-child/main         # clean up artifact
cargo run --bin lotus -- build examples/06-mutable-counter/main.lt
./examples/06-mutable-counter/main       # prints "n=2"
rm examples/06-mutable-counter/main      # clean up artifact
cargo run --bin lotus -- build examples/07-control-flow/main.lt
./examples/07-control-flow/main          # prints "sum=29 stopped at n=9"
rm examples/07-control-flow/main         # clean up artifact
cargo run --bin lotus -- build examples/08-monotonic-sleep/main.lt
./examples/08-monotonic-sleep/main       # prints tick 0..2 + done; ≥150ms
rm examples/08-monotonic-sleep/main      # clean up artifact
cargo run --bin lotus -- build examples/09-functions/main.lt
./examples/09-functions/main             # prints square(7)=49 / fib(12)=144 / ...
rm examples/09-functions/main            # clean up artifact
cargo run --bin lotus -- build examples/10-stateful-locus/main.lt
./examples/10-stateful-locus/main        # prints total=160 / step=30
rm examples/10-stateful-locus/main       # clean up artifact
cargo run --bin lotus -- build examples/11-drain-dissolve/main.lt
./examples/11-drain-dissolve/main        # parent: birth, child-a/b drain+dissolve, parent: drain+dissolve
rm examples/11-drain-dissolve/main       # clean up artifact
cargo run --bin lotus -- build examples/12-user-types/main.lt
./examples/12-user-types/main            # p.x=3 p.y=4, q.x=13 q.y=8, alice says hello (priority 7)
rm examples/12-user-types/main           # clean up artifact
cargo run --bin lotus -- build examples/05-bus/main.lt
./examples/05-bus/main                   # got: hello from sender-1, ack: hello
rm examples/05-bus/main                  # clean up artifact
cargo run --bin lotus -- build examples/13-decimal-and-exit/main.lt
./examples/13-decimal-and-exit/main      # bid/ask/spread/mid/fee printed
rm examples/13-decimal-and-exit/main     # clean up artifact
cargo run --bin lotus -- build examples/03-closure-test/main.lt
./examples/03-closure-test/main          # collapsed cleanly.
rm examples/03-closure-test/main         # clean up artifact
cargo run --bin lotus -- build examples/trellis-demo/main.lt
./examples/trellis-demo/main             # 5x intent + 3x kernel hot-load
rm examples/trellis-demo/main            # clean up artifact
cargo run --bin lotus -- build examples/03b-closure-absorbed/main.lt
./examples/03b-closure-absorbed/main     # AuditL absorbs the violation, exits 0
rm examples/03b-closure-absorbed/main
cargo run --bin lotus -- build examples/03c-closure-bubbled/main.lt
./examples/03c-closure-bubbled/main      # bubble → exits non-zero
rm examples/03c-closure-bubbled/main
cargo run --bin lotus -- build examples/04-modes/main.lt
./examples/04-modes/main                 # bulk=60, harmonic=3, resolution=30
rm examples/04-modes/main
cargo run --bin lotus -- build examples/14-projection-classes/main.lt
./examples/14-projection-classes/main    # rich/chunked/recognition: total=6
rm examples/14-projection-classes/main
cargo run --bin lotus -- build examples/15-match/main.lt
./examples/15-match/main                 # zero/two/other; status: live/dormant; got value=42
rm examples/15-match/main
cargo run --bin lotus -- build examples/16-schedule-classes/main.lt
./examples/16-schedule-classes/main      # cooperative + main + (50ms) + pinned on its own pthread
rm examples/16-schedule-classes/main
cargo run --bin lotus -- build examples/17-yield/main.lt
./examples/17-yield/main                 # logged tick 1/2/3 with `--- after first/second yield ---`
rm examples/17-yield/main
cargo run --bin lotus -- build examples/18-pinned-lifecycle/main.lt
./examples/18-pinned-lifecycle/main      # main: spawned + pinned.birth/run/drain/dissolve on pinned thread
rm examples/18-pinned-lifecycle/main
cargo run --bin lotus -- build examples/19-pinned-bus/main.lt
./examples/19-pinned-bus/main            # cooperative publisher feeds 3 ticks to pinned subscriber
rm examples/19-pinned-bus/main
cargo run --bin lotus -- build examples/20-pinned-core/main.lt
./examples/20-pinned-core/main           # two pinned workers on cores 0 and 1 (best-effort)
rm examples/20-pinned-core/main
cargo run --bin lotus -- build examples/21-arrays/main.lt
./examples/21-arrays/main                # nums[i] reads + sum_of + dot product over [Int; N]
rm examples/21-arrays/main
cargo run --bin lotus -- build examples/22-moving-average/main.lt
./examples/22-moving-average/main        # 6 samples → smoothed averages 25/75/150/250/350/450
rm examples/22-moving-average/main
cargo run --bin lotus -- build examples/23-ranges/main.lt
./examples/23-ranges/main                # triangular(10)=45, factorial(5)=120, factorial(7)=5040, square>50 at i=8
rm examples/23-ranges/main
cargo run --bin lotus -- build examples/24-default-params/main.lt
./examples/24-default-params/main        # greet/pow with omitted trailing args
rm examples/24-default-params/main
cargo run --bin lotus -- build examples/25-imports/main.lt
./examples/25-imports/main               # multi-file: types.lt + notional.lt + main.lt → "GOOG notional = 17050"
rm examples/25-imports/main
```

If all twenty-nine work, the checkpoint is intact.
