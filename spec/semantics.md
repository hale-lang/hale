# Operational semantics

This document specifies what Aperio programs *do* when they
run. Where `grammar.ebnf` says what's syntactically valid and
`types.md` says what's meaningfully valid, this document says
what happens at runtime.

The semantics are described informally (engineering-grade prose),
not as formal small-step rules. Formal operational semantics are
deferred to v1+; the framework's substrate-invariance claim is
not blocked on full formalization.

## Program startup

1. The runtime initializes:
   - Region allocator, schedulers (one per CPU core; cooperative
     within each), bus router, lifecycle dispatcher.
   - Establishes the **runtime root locus** as the implicit
     parent of `main`'s implicit locus.
2. Bus topics with `bindings { Topic: ...; }` entries in the
   `main` locus are registered against their declared transports.
   Topics without a binding stay same-process via the cooperative
   queue.
3. `fn main()` is invoked.

## Function call

A `fn name(args) -> ret { body }` invocation:

1. Caller passes args by value.
2. Implicit locus is allocated for the function's scope.
3. Body executes statement-by-statement.
4. Locally bound handles are bound to local names.
5. Anonymous-child loci with ongoing-work surface attach to
   the implicit locus.
6. On `return value;`, control flows back to caller. Before
   return, the implicit locus drains and dissolves (waits for
   all children to finish; depth-first cascade per F.4).

### Implicit Int → Float widening at call sites

When a function parameter has type `Float` and the call-site
argument has type `Int`, codegen inserts an implicit
`sitofp` widening at the call site. The same rule fires at
let-binding type ascriptions: `let nf: Float = self.n;`
where `self.n: Int` succeeds. The widening is **one-way
only** — `Float → Int` narrowing remains explicit, and
`Decimal` never participates in implicit cross-type
conversion. Phase 2c (2026-05-11). See F.23 in
`spec/design-rationale.md` and the Phase 2c entry in
`spec/stdlib.md`.

## Expressions — `if` and block tails

A `{ ... }` block whose last item is an expression *without*
a trailing `;` carries that expression as its **value**. In
expression position (let-RHS, fn-call argument, if-arm body)
the value is consumed; in statement position (function body,
loop body, `Stmt::If` / `Stmt::Match` block) the trailing
expression is evaluated for side effects and the value is
discarded — semantically equivalent to having added the `;`.

`if cond { ... } else { ... }` is dual-position:

- **As statement** (`if` not at let-RHS / argument / arm-body):
  no value; trailing expressions in either arm are evaluated
  for side effects.
- **As expression** (e.g., `let x = if cond { i } else { j };`):
  the then- and else-arms' trailing expressions are
  phi-merged at the join basic block. The else branch is
  **required**; arm trailing-expression types must match;
  arms may carry their own let-bindings before the tail (the
  bindings are scoped to the arm).

`else if` chains carry through the value path —
`ElseBranch::ElseIf` recurses and the innermost arm's tail
feeds the phi at the outermost merge.

Phase 2b (2026-05-11). See F.24 in `spec/design-rationale.md`
and the Phase 2b entry in `spec/stdlib.md`.

## Binary data — Bytes and conversion

`Bytes` is the binary-safe sibling of `String`. Same
single-pointer ABI; the underlying blob is
`[i64 len][u8 data[len]]`. The `i64 len` prefix means
embedded NUL bytes survive, unlike `String`'s strlen-based
view.

Producing a `Bytes`:

- `std::io::fs::read_bytes(path) -> Bytes fallible(IoError)` (m89;
  IoError flip 2026-05-16). Caller addresses with
  `or raise` / `or fallback(err)`.
- `Stream.recv_bytes(max: Int) -> Bytes` — binary-safe TCP
  receive (Phase 2g).
- `std::bytes::from_string(s: String) -> Bytes` — copies the
  strlen-measured body into a length-prefixed blob (Phase 2g).
- `std::bytes::slice(b, lo, hi) -> Bytes` — half-open range
  copy with bound clamping (Phase 2g).

Consuming a `Bytes`:

- `len(b) -> Int` reads the length prefix.
- `std::bytes::at(b, i) -> Int fallible(IndexError)` — byte-as-Int
  (0..255). Address out-of-bounds via `or` clause (Phase 2g;
  IoError flip 2026-05-16 swapped the pre-flip `-1` sentinel for
  the fallible channel).
- `Stream.send_bytes(b)` — length-preserving TCP send (m89).
- `std::str::from_bytes(b) -> String` — copies into a
  NUL-terminated buffer; embedded NULs persist but downstream
  strlen-based String operations truncate at the first
  (Phase 2g).

All returned `Bytes` values from the path-call surface are
anchored in the lazy global payload arena, so callers can
stash the pointer past the call site without m49 deep-copy
plumbing.

## Locus instantiation

`LocusName { params }`:

1. Compute params (overrides applied to declared defaults).
2. Locus type's `accept(c)` (if instantiated inside a parent's
   lifecycle method) runs first; if it rejects, instantiation
   fails (no region allocated).
3. Region allocated as sub-region of enclosing locus's region;
   size determined by projection class.
4. `birth(args)` runs synchronously.
5. Bus subscriptions wire up.
6. Modes are reachable for invocation.
7. If `run` declared, scheduled to run on the locus's
   scheduler.
8. Expression returns the locus handle.

### Dissolve timing rules

Three shapes, three timings (m82 — "locus all the way down"):

- **Statement-position literal** (`LocusName { ... };`, no
  binding): birth → run → drain → dissolve all fire at the
  statement boundary. Fire-and-forget. The handle is discarded.
- **Let-bound literal** (`let h = LocusName { ... };`): birth
  + run + drain fire at the construction site. Dissolve is
  **deferred to the enclosing fn's scope-exit flush**. The
  user-visible binding `h` is the handle; the locus instance
  lives until `h` goes out of scope. This is what makes
  `let s = Stream { conn_fd: fd }; s.send(msg);` work — `s`
  stays valid for the method call because dissolve hasn't
  fired yet.
- **Long-lived** (locus has `bus subscribe`): always deferred,
  irrespective of binding shape — the locus must stay alive to
  receive published events between birth and the enclosing
  scope's exit.

Multiple deferred dissolves in the same scope fire in
**reverse instantiation order** at scope exit (LIFO), matching
the F.4 depth-first cascade. The reason: a later-created
locus may depend on an earlier-created one, so the later one
must dissolve first.

The deferred-dissolve mechanism is fn-level, not block-level,
in v0. Loops that bind a locus per iteration accumulate
dissolves until fn exit. Per-iteration cleanup uses a helper
free fn whose return is the per-iteration boundary (see
`handle_one_connection` in `stdlib/io_tcp.ap`).

### Method-returning-locus heap allocation (m90)

When a method declares `-> Some` and instantiates a `Some`
in its body, the instance is allocated via the lazy global
payload arena (program-lifetime), **not** the caller's stack
or the callee's arena. Both the eager dissolve and the
deferred-frame push are suppressed at the instantiation site;
the locus semantically "moves" to the caller and lives for
the program.

This is the codegen-side fix for "second method call on a
returned locus reads stale state" — the first read sees
still-valid memory, the second sees overwritten state. Heap
allocation gives the returned handle program-lifetime safety
at the cost of leaking the locus instance + its arena until
process exit. Acceptable trade-off for v1 (matches `Bytes`
lifetime semantics).

A return-slot ABI (caller passes a struct out-pointer +
adopts the locus into its own deferred-dissolves frame) would
tighten this without leaking — deferred to v1.x. Covers
both `return Some { ... };` and `let s = Some { }; ...;
return s;` because `current_user_fn_ret` is set during either
literal's lowering.

## Capacity slot lifecycle and dispatch (F.22)

A locus's `capacity { pool X of T; heap Y of T; ... }` block
declares **slots 1..N** — additional storage disciplines
beyond slot 0 (the locus's own Arena, implicit). Slot order in
the declaration is significant.

### Slot lifetime ordering

Slot init runs at instantiation, in declaration order, **after
slot 0 (arena) is set and before the locus's own field
initializers run**:

1. Slot 0 (arena) — fresh `lotus_arena_create()`, or a
   sub-region of the parent's arena if the parent's projection
   class is Chunked / Recognition and accepts this locus.
2. For each declared slot in declaration order: call
   `lotus_pool_create(size_of(T), 8)` or `lotus_heap_create(
   size_of(T), 8)`. Store the returned allocator pointer in
   the slot's `__slot_<name>: ptr` field.
3. Locus's user fields (params + their defaults / overrides).
4. Synthetic flags (`__restart_count`, `__quarantined`, etc.).

Slot destroy runs at dissolve, in **reverse declaration order**,
**before slot 0**:

1. Drain + dissolve closures + user `drain()` / `dissolve()`.
2. For each slot in reverse declaration order: call
   `lotus_pool_destroy(allocator)` or `lotus_heap_destroy(
   allocator)`.
3. Slot 0 arena destroyed via `lotus_arena_destroy(arena)`.

Reverse-order destroy matches F.4's reverse-instantiation
cascade rule for let-bound loci; the same principle applies
to slots within a locus.

### Slot restrictions (v1)

1. **Slot element type must be a value-shape, not a LocusRef.**
   Loci have lifecycle; cell recycling (Pool.release) or
   individual free (Heap.free) would orphan the locus. Use
   `accept(c: Child)` for locus membership; slots are for
   value-shaped types. Enforced at typecheck (with a
   span-targeted diagnostic) and again at codegen as defense
   in depth.
2. **Slot pointers don't cross the bus.** Structurally
   enforced: slot names aren't typeable identifiers, so they
   cannot appear as bus payload struct fields. No runtime
   check is needed; the type system makes the case unreachable.
3. **Duplicate slot names rejected.** Two slots sharing a
   name (even across separate `capacity { ... }` blocks on
   the same locus, though v1 grammar admits only one block
   per locus in practice) fail at both typecheck and codegen.

### Method-shaped slot dispatch

The user-facing surface is `self.<slot>.<method>(args)`. The
parser and typechecker both recognize `self.<slot>` as a
slot reference rather than a missing field; the codegen
intercepts the method-call shape and routes directly to the
matching C primitive:

| Slot kind | acquire / borrow | release / return |
|---|---|---|
| `pool X of T` | `self.X.acquire() -> Cell<T>` (no args) | `self.X.release(c)` (one Cell<T> arg) |
| `heap Y of T` | `self.Y.alloc() -> Cell<T>` (no args) | `self.Y.free(c)` (one Cell<T> arg) |

Calling a pool method on a heap slot (or vice versa) is a
build-time diagnostic that names the right method for the
slot kind. The `Cell<T>` cell type is documented in
`types.md`; struct cells support `cell.field` read/write (v1.x-2)
and `Cell<T>` carries slot-of-origin so cross-slot release is
a hard error (v1.x-5). Primitive cells (`Cell<Int>` etc.) still
reject field access with a focused diagnostic — direct
load/store through a primitive Cell handle (e.g. `*cell`) is
the natural next surface but no current workload demands it.

Slot access outside a method-call receiver position (e.g.,
`let x = self.entries;` to hold a slot handle as a value) is
not supported at v1 — slots have no value-level CodegenTy
that survives outside the dispatch path. Codegen errors with
"no field on locus self" if the standalone access slips past
typecheck. v1.x can lift this if a workload demands first-
class slot-handle values.

### Slot 0 parent-override

When a locus is accepted by a parent whose projection class is
**Chunked** or **Recognition**, the child's slot 0 (arena) is
allocated either as a sub-region of the parent's arena (Chunked,
via `lotus_arena_create_subregion`) or out of the parent's
recpool (Recognition with the matching sub-mode, via
`lotus_recpool_fixed_acquire` / `lotus_recpool_slab_acquire`).
The child is freed wholesale when the parent dissolves.
**Rich**-class parents do not sub-region-allocate; accepted
children get their own top-level arenas. See `memory.md`
Per-projection-class allocation table.

F.22 names this as "projection class governs parent-override
of slot 0." **Slot 1..N parent-override** (`pool entries of Int
as_parent_for Child;`) shipped via v1.x-4 (surface) + v1.x-4b
(runtime mechanic, commit `d50ab79`): the borrow-mask
`__slot_borrowed_mask` field carries one bit per slot, set when
the slot was borrowed from a parent's matching slot at accept
time; the dissolve pass skips destroy on borrowed slots so the
parent retains ownership of the underlying allocator.

## Lifecycle method invocation

### `birth()`

Runs once, synchronously, after region allocation and before
the locus is "live" for any other purpose. Failure during
birth: region freed, parent's `on_failure(self,
StructuralFailure { ... })` invoked.

### `accept(c)`

Runs **before** child c's region is allocated (per F.7).
Receives c's declared params (not its running state). Can:

- Return normally (accept) — child proceeds to allocation +
  birth.
- Panic / return error (reject) — child instantiation fails.

After accept returns normally, child registers in
`self.children` (per F.11).

### `run()`

Runs continuously until drain is requested or run returns
naturally. Cooperative — yields at every bus dispatch, every
`time::sleep`, every explicit yield point. The scheduler may
run other loci while this run is yielded.

If run() returns naturally, the locus exits run-state and
proceeds to drain.

If run() panics, parent's `on_failure(self, StructuralFailure
{ ... })` invoked.

### `drain()`

Runs once, when the locus is asked to drain. Drain *cascades
depth-first* (per F.4): drain runs on all children first,
synchronously; then runs on self.

During drain:
- New child accepts are refused.
- In-flight handler invocations complete.
- New bus messages are not accepted; in-flight messages on
  bus subscriptions are delivered.
- Closure tests at `tick` epoch may fire (if not already
  fired).

Default drain: no-op (just transitions state from running to
drained).

### `dissolve()`

Runs once, after drain completes. Executes user-supplied
cleanup code if any. Then:

- Closure tests at `dissolve` epoch fire (per F.9). Failure
  records explosion flag.
- Region freed wholesale.
- If exploded, parent's `on_failure(self, ClosureViolation
  { ... })` invoked alongside region release.
- Otherwise, parent sees normal child-dissolution.

Default dissolve: free region.

### `on_failure(c, err)`

Runs when a child of self fails (any failure type:
StructuralFailure, ClosureViolation, etc.). Receives the
child handle and the typed error.

The handler may:
- Return normally (absorb): treat as collapsed; parent
  forgets about the child.
- Call `restart(c)`: re-instantiate the child with the same
  params.
- Call `restart_in_place(c)`: re-init in place (preserve
  arena).
- Call `quarantine(c)`: keep child in a halted state with
  arena preserved; future inspection possible.
- Call `bubble(err)`: pass the failure to self's parent.
- Call `dissolve(c)` explicitly: free child's region.

Default on_failure: `bubble(err)`. The runtime root's default
is process exit with stack trace.

## Mode invocation

`self.bulk()` / `self.harmonic()` / `self.resolution()` invoke
mode declarations. Modes are:

- Synchronous functions taking the receiver as implicit
  argument.
- Read/write the locus's arena directly (no copies).
- Compiled to per-projection-class implementations.

Mode invocation from outside the locus (e.g., `child.bulk()`
from a parent) is permitted iff `bulk` is contract-exposed
on the child; goes through the contract's typed surface (per
F.14).

## Topic declarations

A `topic Foo { payload: T; }` declaration names a typed pub/sub
channel at top level. Subscribers, publishers, and send sites
reference the topic by name; the payload type travels with the
declaration instead of being repeated at every `subscribe ...
of type T` site.

```aperio
type Tick { n: Int; }
topic Ticks { payload: Tick; }

locus Counter {
    params { count: Int = 0; }
    bus { subscribe Ticks as on_tick; }    // no `of type T`
    fn on_tick(t: Tick) { self.count = self.count + 1; }
}

locus Pub {
    bus { publish Ticks; }                  // no `of type T`
    run() {
        Ticks <- Tick { n: 1 };             // identifier subject, not "Ticks"
    }
}
```

Type-check rules:

1. Every subscriber's handler signature must match `Topic.payload`
   exactly — a static error cites both sites if they diverge.
2. The send-expression's type at a topic-ref `<-` site must match
   `Topic.payload`.
3. The `of type T` clause is forbidden on topic-ref subscribe /
   publish; the topic carries the payload type.
4. A topic identifier outside subscribe / publish / send-subject
   position (e.g. `let x = Foo;`) is a type error — topics are not
   values, they only address bus channels.

`topic` is a contextual keyword: lexes as `IDENTIFIER` except in
top-level declaration position, so existing names (struct fields
called `topic`, local variables named `topic`) continue to work.

Lowering: codegen and runtime work against the legacy
string-subject form. A desugaring pass between typecheck and
codegen rewrites `BusSubject::Topic(Foo)` → `BusSubject::Literal {
subject: "Foo" }` and fills in the elided payload type, so the
downstream pipeline (cooperative queue, mailbox post, transport
fanout) is unchanged from the string-subject path. The wire-format
subject for a topic named `Foo` is the bare string `"Foo"`.

Coexistence: the literal-string form (`subscribe "S" as h of
type T;`) is still accepted because the log namespace lotus
relies on wildcard publish (`publish "log.**" of type LogEvent;`)
+ runtime-computed subject strings (`subj <- LogEvent { ... }`
where `subj` is `"log." + self.full_path`), and the topic-decl
form has no equivalent at v1. The two forms can be mixed within
one program; they only collide if a topic name and a literal
subject share the same wire-format string, which the type
checker catches via the standard duplicate-symbol diagnostic.

**Canonical form for new code:** prefer the topic-decl form
(`topic Foo { payload: T; subject: "wire.subject"; }` +
`subscribe Foo as h;`). Reach for the literal-string form only
when you need a wildcard subscription or a runtime-computed
publish subject — those are the cases the topic system doesn't
cover at v1.

### Phase 2: hierarchy, subjects, bindings, closed-world optimization

Phase 2 extends topic declarations with three orthogonal pieces:

**1. Hierarchical topics + wire subject.** A topic may declare a
parent and an own-subject segment. The materialized "wire subject"
is the dot-joined chain of segments root-to-leaf:

```aperio
topic Events { payload: Event; subject: "events"; }
topic Login : Events { payload: Login; subject: "login"; }
// Login's wire subject is "events.login".
```

Defaults: own-subject defaults to the topic's name (verbatim), so
top-level `topic Ticks { payload: Tick; }` keeps Phase-1's
behavior of wire subject `"Ticks"`. Parent must reference a
declared topic; cycles + missing parents are typecheck errors. Two
distinct topics that produce the same wire subject are also
errors — path-shaped routing would be ambiguous.

The desugar pass rewrites `BusSubject::Topic(Login)` to
`BusSubject::Literal { subject: "events.login" }` so codegen and
the bus runtime see only the wire form.

**2. `main` locus + `bindings { }` block.** A locus prefixed with
`main` is the binary's entry-point holder and is the only place a
`bindings { }` member is legal. Bindings choose a transport per
topic; the same library compiles to in-process or external in
different binaries by varying the main locus.

```aperio
main locus App {
    bindings {
        // Beat: not bound — same-binary cooperative queue (default).
        Login:  unix("/tmp/login.sock");                  // role inferred
        Events: unix("/tmp/events.sock", role: listen);   // explicit override
        Remote: MyNatsAdapter { url: "nats://..." };       // adapter locus
    }
}
```

Transport surface:

- `unix("/path")` or `unix("/path", role: connect|listen)` —
  AF_UNIX framed-byte transport. Substrate-provided: the
  runtime's `lotus_transport_*` owns the delivery contract
  directly. `role: listen` spawns a reader thread that fans
  recv'd payloads into the local handler set; `role: connect`
  opens a write-side transport that publish-site dispatch sends
  to. When `role:` is omitted, the typechecker infers it from
  the bus block (`publish` only → connect, `subscribe` only →
  listen); if both publish and subscribe touch the topic, the
  binding is rejected with a "specify `role:`" diagnostic.

- `LocusName { field: value, ... }` — user-supplied
  protocol-layer adapter. Any locus that declares
  `fn send(subject: String, bytes: Bytes)` satisfies the
  `__StdBusAdapter` contract and may appear on the right-hand
  side of a binding. The bus router dispatches outbound payloads
  for the bound topic through the adapter's `send` method;
  framing, retry, ordering, and connection management are the
  adapter body's concern. The adapter's own `params` block
  carries protocol configuration (broker URL, credentials,
  timeouts, point-to-point role for p2p shapes). The grammar
  distinguishes substrate vs adapter by the head's case
  (lowercase keyword `unix` vs capitalized locus name).
  Inbound dispatch from an adapter into the local handler set
  awaits the `__bus_local_dispatch` opening (deferred).

- `shm_ring("/name", slot_count: N, on_overflow: <policy>)` —
  POSIX SHM ring substrate backing the zero-copy route. Name
  is the shm_open object name; slot_count defaults to 128 when
  not specified. `on_overflow` is REQUIRED — see "Back-pressure"
  below. Satisfies `intra_machine` and `zero_copy` constraints
  intrinsically. Slot size is derived at codegen from the
  topic's payload type (which must satisfy `is_flat_shapeable`
  — variadic fields rejected). Substrate-provided: the
  runtime's `lotus_shm_ring_*` primitives in
  `runtime/lotus_shm_ring.c` own the lifecycle.

  At codegen, each shm_ring binding emits a
  `lotus_bus_register_shm_ring(subject, slot_size,
  slot_count, name)` call into main's prelude (alongside
  the existing `lotus_bus_register_remote` for unix
  bindings). Subsequent `Topic <- value` (Send) statements
  on the bound topic short-circuit to
  `lotus_bus_publish_shm_ring(subject, &value, sizeof(value))`
  — the C runtime owns claim + memcpy + commit. This is the
  one-memcpy path: 1.6x faster than the m28b two-memcpy
  baseline per `experiments/k2-zero-copy/bench.c`.

  Explicit `let slot = topic.claim(); slot.field = ...;
  slot.commit();` surface (the slot-as-locus design in
  [[slot-locus-design]]) for the zero-memcpy path is
  post-v1; the implicit `<-` path covers the common case.

  **Subscribers (Form K6b, 2026-05-20).** Aperio-side
  `bus subscribe` for shm_ring-bound topics is wired.
  Codegen emits a `lotus_bus_register_subscriber_shm_ring(...)`
  call at the subscriber locus's birth lifecycle; the C
  runtime opens the ring, spawns a dedicated reader thread
  per binding, and dispatches each newly-committed slot to
  the user's `fn on_foo(p: T)` handler with `p` pointing
  directly into the ring slot (no memcpy on the subscriber
  side).

  **Threading constraint.** The handler runs on the reader
  thread, NOT the cooperative scheduler. Handlers must be
  thread-safe and avoid touching shared scheduler state.
  Users who need cooperative dispatch should use
  `unix(...)` instead. Future versions may add an optional
  cooperative-queue routing mode at the binding level.

  **Staleness.** v1 ships without a stamped-epoch read
  guard — handlers must finish fast enough that the ring
  doesn't wrap past the slot they're reading. If a slot
  has wrapped at the moment the reader thread fetches it,
  the slot is skipped silently (lotus_shm_ring_read_slot
  returns NULL). Post-v1 work will generalize F.30b's
  stamped-epoch guard for per-field read checks.

  **Back-pressure (Form K7, 2026-05-20).** `on_overflow:`
  is required on every shm_ring binding — there's
  intentionally no default. Three policies:

  - `block` — publisher's `claim()` spins with 100µs
    nanosleeps until the consumer's release-stored
    `consumer_seqno` advances enough for a free slot.
    No timeout in v1; deadlocks if the consumer dies.
    Right for control-plane topics where latency tolerates
    backpressure but data must not be lost.
  - `drop` — publisher's `claim()` returns the next slot
    unconditionally (pre-K7 behavior). Slow consumers
    silently miss messages. Right for stale-is-worthless
    feeds (market data tickers, telemetry).
  - `fail` — publisher's `claim()` returns NULL when the
    ring is full; the `publish_shm_ring` wrapper panics
    with a clear stderr diagnostic and `_exit(1)`.
    Process-level visibility into back-pressure events.
    Graceful caller-side handling via fallible-`<-` is a
    K7b follow-up; today, fail = process exits.

  The consumer's reader thread release-stores the cursor
  after each batch of dispatches; the cursor lives on its
  own cache line (separate from the producer's `seqno`)
  so the two sides don't pingpong each other's writes.

**In-memory delivery is absence-of-entry.** A topic with no
binding entry is delivered same-process via the cooperative
queue. There is no `in_memory` variant — the runtime default
covers the case and explicit syntax would be ceremony.

**Operational constraints (Form K, 2026-05-20).** A binding
entry may carry an optional `where` clause listing
constraint keywords the dev team asserts the route must
satisfy:

```aperio
bindings {
    L2Updates: unix("/sock") where intra_machine, zero_copy;
}
```

Constraints split into two orthogonal axes:

- **Scope** — where the bus may reach. `intra_process` (same
  OS process), `intra_machine` (cross-process, same machine;
  SHM-capable), `cross_machine` (network in scope). Hierarchy:
  `intra_process ⊂ intra_machine ⊂ cross_machine`.
- **Behavior** — `zero_copy` (no memcpy at locus boundary;
  requires the payload type to satisfy `is_flat_shapeable`).

The typechecker validates three classes of constraint issue
(Form K4a, 2026-05-20):

1. **Intra-constraint consistency.** At most one scope
   keyword per binding (`intra_machine` + `intra_process` is
   rejected as ambiguous). `zero_copy` + `cross_machine` is
   rejected as a contradiction — network transports require
   serialization.

2. **Transport-constraint compatibility.** Each declared
   constraint is checked against the binding's transport
   variant:
   - `unix(...)` satisfies `intra_machine`. Rejects
     `intra_process` (sockets cross processes),
     `cross_machine` (AF_UNIX is host-local), and `zero_copy`
     (kernel memcpy at the socket boundary).
   - Adapter loci: trusted for scope constraints (the adapter
     body knows where its protocol routes). Rejected for
     `zero_copy` — the Adapter contract
     (`fn send(subject, bytes)`) requires serialization.

3. **Payload-shape compatibility.** `zero_copy` requires the
   topic's payload to satisfy `is_flat_shapeable` — every
   leaf must be a fixed-layout primitive, fixed-size array of
   flat-shapeable, or struct whose fields are all
   flat-shapeable. String, Bytes, BytesView, StringView, and
   unbounded arrays are variadic and fail the predicate.

Slot-locus codegen and the `shm_ring(...)` transport variant
that actually satisfies `zero_copy` land in subsequent K
sub-tasks. Until then, asserting `zero_copy` on any binding
produces a clear diagnostic naming the transport limitation.
Existing bindings without a `where` clause continue to work
unchanged.

Bundle-wide rules:

1. At most one `main` locus per bundle. Zero is fine — the
   classic bare `fn main()` shape is still legal.
2. Each `bindings` entry's topic must name a declared `topic`.
3. A topic may appear at most once across all bindings.
4. Bindings only legal in a `main`-modified locus. The parser
   rejects them in any other locus position.
5. Every binding's role must be either explicit (`role:`
   kwarg) or unambiguously inferable from the bus block.

Codegen emits one runtime registration call per binding entry
into `fn main`'s prelude, right after the bus queue is published:
- Unix bindings call
  `lotus_bus_register_remote(subject, url, role)`.
- Adapter bindings first instantiate the adapter locus with
  program-lifetime allocation (same m90 routing the
  `-> LocusRef(L)` return path uses), resolve the locus's `send`
  method's function pointer, then call
  `lotus_bus_register_remote_adapter(subject, self, send_fn)`.

Subjects use the desugared wire form (so a binding for
hierarchical `Login` registers as `"events.login"`). Topics with
no binding entry get no register call and stay same-process via
the cooperative queue.

**3. Closed-world topology optimization.** When a topic has no
binding and the publisher / subscriber relationship is statically
unambiguous, the desugar pass rewrites the publisher's
`Stmt::Send` into a direct method call. Two shapes qualify:

- **Intra-locus (same-type):** publisher locus type == subscriber
  locus type. Every Send happens inside an instance of the same
  locus that hosts the handler. Rewrite: `Foo <- v` →
  `self.handler(v)`.
- **Intra-tower (parent → child):** publisher locus type P has
  exactly one direct singleton field (declared in `params { }`)
  whose type names the subscriber locus type S. Every Send in
  P's body statically routes to that one child. Rewrite:
  `Foo <- v` → `self.<field>.handler(v)`.

Common preconditions for both shapes:

- No `bindings { Topic: ... }` entry exists for this topic.
- Exactly one locus type publishes the topic.
- Exactly one locus type subscribes the topic.

When eligible, the publish→queue→drain→dispatch path collapses
to a synchronous method call. The `subscribe` / `publish`
entries stay declared (still type-check) but the bus runtime
never sees traffic on the optimized subject. This is a pure-perf
rewrite — observable behavior is identical modulo timing
(synchronous instead of cooperative-deferred).

Out of scope for v1 (fall through to bus dispatch unchanged):

- Multi-hop towers (`Outer → Middle → Leaf`).
- Plural / vec / capacity-slot children — broadcast semantics
  don't match the singleton-rewrite shape.
- Child-publishes-parent-subscribes — needs a parent-reference
  mechanism that doesn't exist in v1.
- A parent with multiple direct fields of the subscriber type
  (ambiguous receiver).

A bound topic is never optimized: the binding may publish to
remote subscribers that aren't visible at compile time.

## Bus subscription dispatch

A `bus { subscribe SUBJECT as HANDLER of type T; }` declaration
wires:

1. The runtime registers HANDLER as the receiver for SUBJECT
   on the bound transport.
2. Inbound messages on SUBJECT are decoded as `T`, then
   HANDLER(payload) is invoked.
3. HANDLER runs in the locus's scheduler context. It may
   call `publish(SUBJECT, msg)` to emit responses (subject
   to `bus { publish ... ; }` declarations).
4. HANDLER yields naturally on completion; scheduler returns
   to other loci.

If HANDLER panics:
- The current message is dropped.
- `on_failure(self, BusHandlerFailure { subject, payload, err })`
  invoked on the parent if any.
- The subscription itself is *not* removed; future messages
  continue to dispatch.

### Payload type — primitives + nested structs + String

The wire format supports primitives (`Int`, `Float`, `Bool`,
`Decimal`, `Duration`, `Time`, `String`), `Bytes`, and
**nested user struct types** (`type T { ... }`) recursively
composed. A bus payload may carry a struct whose fields are
primitives, Strings, Bytes, or other nested structs, at any
depth. Serialize walks the field tree in declaration order;
deserialize allocates each nested struct in the lazy global
payload arena and recurses. Arrays, tuples, and enums as bus
payload fields are post-v1 polish.

## Closure-test evaluation

For each `closure NAME { LEFT ~~ RIGHT within TOL; epoch
... }`:

1. At each declared epoch boundary, runtime evaluates LEFT
   and RIGHT in the locus's scope.
2. Computes `|LEFT - RIGHT|`.
3. If `<= TOL`: closure passes silently.
4. If `> TOL`: flips the locus's "exploded" flag; emits a
   typed `ClosureViolation` event.

Epoch boundaries:

- `epoch dissolve` (default): fires once, as part of dissolve
  sequence.
- `epoch tick`: fires on each runtime tick (configurable
  cadence).
- `epoch duration(d)`: fires every `d` of monotonic time.
- `epoch birth`: fires once, after birth completes.
- `epoch explicit`: fires only when user code calls
  `epoch_advance(NAME)`.
- `epoch inline` (F.27, v1.x-VIOLATE): never fires
  automatically; fires only when user code executes
  `violate NAME;`. The closure body has no assertion (no LEFT /
  RIGHT / TOL to evaluate). See "Inline closure violation"
  below.

## Inline closure violation

(F.27, v1.x-VIOLATE.) Inline closures provide a pull-only
structural-failure channel for locus method bodies that catch a
value error and want to escalate it. The declaration carries no
assertion; the optional `captures:` clause names locus fields
whose values are snapshotted into the ClosureViolation payload
at fire time.

```aperio
closure fatal_io { captures: last_error; epoch inline; }
```

`violate NAME;` (optionally `violate NAME with EXPR;`) fires the
closure synchronously at the call site:

1. Runtime synthesizes a `ClosureViolation` value carrying:
   - `locus`, `closure` — string names of the failing locus and
     the inline closure (always present, both runtimes).
   - Under `aperio run` only: one field per name in the
     closure's `captures:` clause, holding the snapshot of
     `self.<field>` taken at the fire point. Under `aperio
     build` the LLVM `ClosureViolation` struct has a fixed
     shape and these convenience fields are not materialized.
     The portable access pattern is to read frozen child state
     through the child handle in `on_failure(c, err)` — see
     "Reading the audit state" below.
   - If `with EXPR` was given (interpreter), a `payload` field
     with EXPR's value. Codegen evaluates EXPR for side effects
     (and to detect typecheck errors on the payload type) but
     does not materialize a `payload` field on the compiled
     `ClosureViolation`.
   - The assertion-shape fields (`left`, `right`, `tolerance`,
     `diff`) are NOT populated for inline violations.
2. The locus's exploded flag is set (same as the auto-epoch
   path; downstream observers can't tell from the flag whether
   the fire was auto-epoch or inline).
3. The synthetic `__drain_requested` field on the locus is
   set. Readable from user code as `self.draining`.
4. The parent's `on_failure(child, ClosureViolation { ... })`
   handler runs — same routing as for auto-epoch closure
   violations.

### Reading the audit state

The portable access pattern in `on_failure(c, err)` is to read
the child's frozen locus state through the child handle:

```aperio
on_failure(c: Child, err: ClosureViolation) {
    log::error(err.closure, " ", c.last_error, " fd=", c.conn_fd);
}
```

`violate` is divergent — the method body's remaining statements
do not execute, so the child's locus state is frozen at the
violate moment. `c.last_error` reads exactly the value the
violate site observed. This works identically in both runtimes.

Under the interpreter, `err.<capture_name>` is also available
as a convenience (the interpreter materializes captures fields
on the `ClosureViolation` struct). Compiled code does not
materialize these fields. Source that reads `err.last_error`
will typecheck (`ClosureViolation` admits unknown fields
permissively at field-access time) but will fail to link / run
under `aperio build` — prefer `c.last_error` for portability.

The `violate` statement is divergent: the typechecker treats it
as `Never`, the same as `fail` in fallible fn bodies and
`bubble` in `on_failure`. No statement after `violate` in the
same block is reachable; the typechecker does not require a
trailing `return` on a `violate` branch.

### `birth_check` synthesis hook

(F.27 v2, 2026-05-20.) A declarative form for construction-time
invariants:

```aperio
locus L {
    params { x: Int = 0; }
    closure invariant_broken { captures: x; epoch inline; }
    birth() { /* set up state */ }
    birth_check { self.x < 0 } -> violate invariant_broken;
}
```

After `birth()` body completes and birth-epoch closures have
fired, each declared `birth_check` clause's `cond` expression is
evaluated. A `true` result fires the named closure with the
locus's fully-constructed state — every field reads its
declared post-birth value, so the on_failure handler's
capture-snapshot sees coherent state. Multiple clauses evaluate
in declaration order; the first to fire short-circuits the
rest.

Why a separate clause vs. calling `violate NAME;` inside the
birth body: a violate mid-birth leaves the locus partially
constructed (some fields set, others at defaults) when the
on_failure handler reads captures. `birth_check` runs the body
to completion before the check fires, so the post-birth invariant
of "every field has its declared value" holds at violation time.
The runtime-routing semantics are otherwise identical to a
regular `violate` (drain_requested set, parent on_failure
absorbs or process exits non-zero with diagnostic). The codegen
emits the check + violate routing INLINE at the instantiation
site, branching to a continuation block on absorbed violations
rather than returning from the caller's fn — the absorbed-then-
continue contract matches what users expect when wrapping the
instantiation in a parent that handles the failure.

The check expression is read-only against `self.X` fields; the
closure name must resolve to a declared epoch-inline closure on
the same locus, same constraint as a regular violate.

### `self.draining`

While the locus is draining, the synthetic `self.draining`
field reads `true` from any locus method body. The canonical
use is to suppress downstream sends after escalation:

```aperio
let r = expr or self.handle_io(err);
if !self.draining { Result <- r; }
```

`self.draining` is the only synthetic field exposed by name to
user code; `__drain_requested` is internal-only.

### Rejection contexts

`violate` is rejected at typecheck in:

- **Free fn bodies.** No `self` to resolve the closure name
  against. A free fn helper called from a locus method body
  cannot violate transitively: `violate` is lexically scoped to
  the locus method body it appears in.
- **`on_failure` body.** Use `bubble(err)` — `on_failure` is the
  parent-side handler for child failures; re-firing a self-
  closure from there mixes the two channels.

Allowed everywhere else that has `self`: named locus method
bodies, bus-handler methods (`subscribe X as foo` → `fn foo`),
`run()`, lifecycle methods (`birth()`, `dissolve()`, `drain()`),
mode-method bodies. The same body shape gets the same primitive.

## Perspective hot-load

For each `perspective P { ... }` instance currently active:

1. New perspective arrives via bus (or explicit
   `load_perspective(P, bytes)` call).
2. Runtime decodes against P's compiled-in schema. Type-
   mismatch → reject; emit `PerspectiveDecodeError`.
3. Validates `stable_when` predicate. If false → reject;
   emit `PerspectiveNotStable`.
4. Atomically swaps the active perspective:
   - Pause all readers (readers within the locus see the
     pre-swap perspective).
   - Replace.
   - Resume.
5. Emit `PerspectiveLoaded` event.

Old perspective is freed only after the swap completes; no
torn read possible.

## Recovery primitives

### `restart(child)`

1. Schedule child for dissolution.
2. Once dissolved, instantiate a new child with the same
   declared params.
3. New child's birth runs; old child's state is gone.

### `restart_in_place(child)`

1. Set child's "restarting" flag.
2. Wait for current handler / mode invocation to complete
   (cooperative yield point).
3. Reset locus to post-birth state, preserving the arena.
4. Re-run birth().
5. Mark restart complete.

Useful for transient failures that don't invalidate the
locus's structural commitments (e.g., the locus's k_max is
fine; just had a bad message).

### `quarantine(child) [for d]`

1. Pause child (no new messages dispatched, no new accepts
   permitted, no run scheduling).
2. Preserve arena and state.
3. If `for d` clause given, automatically restart after `d`.
4. Otherwise wait until parent explicitly resolves.

### `reorganize(child, ...)`

Reserved syntax. Semantics TBD; expected: relocate child's
sub-children to a sibling. Not in v0.

### `bubble(err)`

Re-raise the error to self's parent. Equivalent to:

```
on_failure(c, err) { bubble(err); }  // for self's failures
```

Fully traverses the lotus tower upward until a handler
absorbs.

### `dissolve(child)`

Force-dissolve child immediately. Skips drain; closure tests
at non-dissolve epochs do not fire on this path. Used for
forced-shutdown scenarios.

## Drain cascade (whole-process)

SIGINT or SIGTERM:

1. Signal handler in the runtime root locus calls
   `drain(self)` on itself.
2. Drain cascades depth-first to all children of root.
3. Each child cascades to its children, etc.
4. Leaves drain first; in-flight messages complete; bus
   subscriptions stop accepting.
5. Each parent waits for all its children to drain before
   draining itself.
6. Root drains last.
7. Runtime tears down schedulers, bus router, allocator.
8. Process exits 0.

## Closure-failure cascade

A closure violation at any epoch:

1. Runtime emits `ClosureViolation` event.
2. Locus's exploded flag is set.
3. Subsequent epochs may also fail; flag persists.
4. At dissolve, parent's `on_failure(self, ClosureViolation
   { ... })` invoked.
5. Parent's policy decides: absorb, recover, bubble.
6. If bubbled, propagates to grandparent; recursively until
   absorbed or reaching root (process exit).

`epoch inline` closures (F.27) take the same cascade path with
one addition: at step 2 they also set `__drain_requested`, so
the locus enters drain at the next cooperative yield rather
than continuing on its current epoch. The drain initiation is
the only divergence from the auto-epoch cascade; routing to
parent's `on_failure` at step 4 is identical.

## Scheduler dispatch

Per `runtime.md`: multi-scheduler cooperative.

1. N schedulers, one per CPU core (configurable).
2. Each scheduler holds a queue of runnable loci.
3. Scheduler picks a locus, runs it until cooperative yield.
4. At yield, scheduler picks next.
5. Cross-scheduler communication via bus (typed messages).
6. Loci may be migrated between schedulers transparently for
   load balancing.

## Failure-traversal flow

Failures flow upward:

1. Child failure → child's parent's `on_failure`.
2. Parent's handler decides absorb / restart / bubble /
   quarantine / dissolve.
3. If bubble, → grandparent's `on_failure`.
4. If reaches runtime root, process exits with structured
   error report.

Failures never flow laterally (sibling-to-sibling) — the
framework's vertical-only-flow expressed at the runtime layer.

## Fallible call semantics (v1.x-FORM-1; PR6 reframe)

Aperio carries two **orthogonal** failure channels:

- **Closure-violation channel** — structural failure of a
  locus's closure (its assertion / invariant) fires
  `Signal::Bubble(ClosureViolation)` and routes through the
  existing `bubble` / `on_failure` machinery. See **F.9**.
  This is the *substrate-facing* channel: it expresses
  "a locus's promised invariant broke" and propagates
  vertically through the locus tower per the failure-
  propagation-upward mechanic.
- **Value-error channel** — value-level `fallible(T)` returns
  are an *addressing protocol* between immediate caller and
  fallible callee. They don't constitute a separate runtime
  mechanism at intermediate frames; they propagate by sret
  + path-indicator through the static call stack, addressed
  at each level by a required `or` clause. This is the
  *application-facing* channel: it expresses "this
  call-by-call computation might fail; address it inline."

The two channels meet at exactly one place: the implicit main
locus's root boundary (see "Process exit" below). Everywhere
else, the channels are independent. See
`notes/agent-onboarding/aperio-design-philosophy.md` § 2.

### Where each channel lives (declaration sites)

The two channels are realized through different declaration
sites. The mapping is canonical, not advisory:

- **`fallible(E)` may be declared on:**
  - **Free fns** — pure application-layer computations whose
    failure shape matters call-by-call.
  - **Stdlib-synthesized methods on `@form(...)` containers**
    (`@form(vec).get`, `@form(vec).pop`, future `@form(...)`
    methods). Application-layer storage substrate: the
    container's role is application-layer data, not locus-
    structural participation in the substrate's lifecycle.
- **`fallible(E)` may NOT be declared on:**
  - **User-declared locus methods.** Substrate-facing surface
    — methods on user-declared loci participate in the
    substrate's lifecycle (bus subscriptions, modes,
    contract reads). Their failures are *structural events*
    that belong on the closure-violation channel. A locus
    method that needs to expose application-layer failure
    semantics wraps a fallible free fn:

    ```aperio
    fn parse_message(b: Bytes) -> Message fallible(ParseError) { ... }

    locus Reader {
        fn handle_input(b: Bytes) -> () {
            let m = parse_message(b) or default_message();
            // ...
        }
    }
    ```

    The typechecker rejects `fn ... fallible(E)` on a locus
    member with a diagnostic naming this rule. The forcing
    function is productive: it surfaces *which channel* the
    failure lives in at the declaration site, where the
    design intent is set, rather than at the use site where
    the channel choice is ambiguous.

The rule is **two-channel separation as a design enforcement**,
not a temporary limitation. Adding fallible to user-declared
locus methods would create a third pathway with overlapping
semantics — the same shape The Design counsels against
(`spec/forms.md` discussion of parametric vs form-shaped
collections is the parallel argument at the type layer).

### `fail` statement

`fail <expr>;` inside a fallible fn body:

1. Evaluates `<expr>` to a value `v` (typed as the fn's
   declared payload type E).
2. Exits the enclosing fallible fn body via the error path.
3. The caller's `Expr::Call` sees the result as
   `FallibleErr(v)` — a tagged value the immediate caller's
   `or` clause is required to address.

`fail` outside a fallible body is a typecheck error;
statement-position recognition is also parser-gated to a
fallible-body scope (so `let fail = 0;` outside such a body
stays admissible).

### `or` disposition

`<expr> or <disp>` evaluates `<expr>`. If the result is a
non-error value, that value is the expression's value
(disposition is a no-op). If the result is `FallibleErr(p)`:

- **`or raise`** — propagate the error one frame up the
  static call stack. Inside a fallible(E) fn, this writes `p`
  into the enclosing fn's error sret slot and exits via the
  enclosing fn's error path; the enclosing caller's `or`
  clause then addresses the error in turn. The closure-
  violation channel is **not** entered. (An application may
  later promote a value error to a closure violation
  explicitly, but no such syntax exists in v1.)
- **`or <fallback>`** — binds `err` to `p` in scope and
  evaluates `<fallback>`. Its result is the expression's
  value. Type must match the success type. `<fallback>` may
  itself be a call (`or handler(err)`); the identifier `err`
  in the fallback expression resolves to the typed payload.
- **`or discard`** (added 2026-05-16) — swallows the error
  and produces Unit. The underlying call's success type MUST
  be Unit; the typechecker rejects `or discard` on
  value-bearing calls with a message pointing at
  `or <default>` or `or raise`. Sugar for the previously-
  idiomatic `or noop(err)` pattern with a no-op handler fn.
- **`or fail <payload>`** (added 2026-05-17, B3 / G6) —
  symmetric to `or raise`, but the caller picks a fresh
  payload of the enclosing fallible fn's declared error type
  instead of forwarding the inner call's payload verbatim.
  Lets a caller translate one error shape into another
  inline (`std::str::parse_int(s) or fail AppErr { msg: "bad
  number" }`) rather than bouncing through a helper fn. Same
  divergence rule: chain value type collapses to the inner
  success type. Typechecker rejects outside a fallible fn
  body with a hint to use `or raise` or `or <fallback>`.

Chains are right-associative: `a() or b() or raise` reduces
the value to the success type level by level.

### Process exit

The runtime ends a program against its will via one of two
boundary events at the implicit main locus's root:

1. **Closure-violation escape (F.9).** If a closure violation
   bubbles past every `on_failure` handler back to the
   runtime root, the process exits with the violation's
   payload as the structured error report.
2. **Value-error escape (PR6).** If an `or raise` reaches
   the implicit main locus's body with no enclosing
   `fallible(E)` frame to absorb it, the value error escapes
   the locus's body. The runtime panics via
   `lotus_root_panic(payload, size, typename)` — today
   dprintf to stderr (`"Aperio panic: unhandled <T>
   escaping main locus"`) + `exit(1)`. Architecturally this
   is the seat for a future routing-through-main-locus-
   `on_failure` extension: when (if) the main locus declares
   `on_failure`, the runtime will route the synthesized
   ClosureViolation through that handler before falling out
   to the dprintf+exit fallback. Until then the boundary
   collapses both channels to the same exit shape.

Both paths preserve the framework's vertical-only-flow: every
failure exits through the top of the recursion, never
laterally.

## Cross-seed namespace resolution (v1.x-IMPORT)

A file may declare `import "<path>" as <alias>;` at the top.
References to library decls go through the alias as
`alias::Name`. Resolution is two-step:

1. **Parse / merge.** The CLI resolves each import's path (per
   `spec/projects.md` "Resolution order"), parses every `.ap`
   file in the resolved target, applies the auto-mangler with
   the user's alias + each file's stem, and merges the mangled
   items into the importing program's item list. A per-build
   path-rename table is built mapping `["<alias>", "<Name>"]`
   to `__lib_<alias>_<stem>_<Name>`.

2. **Codegen lookup.** Codegen's qualified-name resolution
   consults three tables in order — static `STDLIB_PATH_RENAMES`,
   static `MOA_PATH_RENAMES`, and the per-build import table —
   when lowering any path-qualified type expression, struct
   literal, or method receiver. The first matching table wins.

Cross-seed references in user code (`foo::Bar`) and intra-seed
references inside the imported library (bare `Bar` from a file
that uses a type declared in a sibling file) BOTH resolve to
the same mangled symbol. The mangler builds a unified rename
map across every file in the imported library before
rewriting, so `greet.ap`'s reference to a `Formatted` type
declared in `format.ap` rewrites to the same
`__lib_<alias>_format_Formatted` symbol that `format.ap`'s
decl ends up at.

Local bindings (`let`, `let mut`, fn params, lifecycle params,
for-loop vars, pattern bindings, generic params) shadow
top-level names per ordinary lexical scope; the mangler's
scope-aware walker leaves shadowed references unrewritten.

**Per-importer scoped imports (A4, 2026-05-17).** Imports
declared inside imported library files **are** followed
transitively by the resolver, but each library's imports land
under that library's own alias namespace — they do not become
visible to the top-level program. So library A importing
library B exposes A's surface to the importer; B is reachable
only through A's API surface (or by the importer re-declaring
its own `import "lib/B" as ...;`). This replaces the prior
strict barrier (which rejected transitive imports outright) and
unblocks composition without leaking dependency identity. See
`spec/projects.md` for the rationale and per-alias scoping rules.

**`aperio run` interaction.** The interpreter path consumes the
merged program but ignores the per-build path-rename table —
mirroring the existing `std::http::Request { ... }` literal
limitation. Programs that use cross-seed imports should be
built and executed via `aperio build`.

## Region lifetime guarantees

Per `memory.md`:

- A locus's region is freed atomically on dissolve.
- Sub-regions are freed before the parent's region (drain
  cascade ensures this).
- No pointer-into-a-freed-region is reachable after region
  release (compile-time-checked + region-lifetime-checked).

## What's deferred

- **Formal small-step semantics.** Engineering-grade prose for
  v0; formal operational rules in v1+ if needed for compiler
  correctness proofs.
- **Concurrency-correctness proofs.** Cooperative scheduler
  + per-locus arena makes most concurrency questions trivial,
  but full formal modeling deferred.
- **Memory-model formalization** as a happens-before relation.
  Currently informal; formal in v1+.
- **Async / await semantics.** Reserved keywords; no operational
  semantics in v0.
