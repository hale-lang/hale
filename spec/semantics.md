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
   - Reads deployment config (e.g., `deployment.yaml`) for
     transport bindings.
   - Establishes the **runtime root locus** as the implicit
     parent of `main`'s implicit locus.
2. Bus channels declared in the program are wired to transport
   adapters per deployment config.
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

- `std::io::fs::read_bytes(path) -> Bytes` (m89).
- `Stream.recv_bytes(max: Int) -> Bytes` — binary-safe TCP
  receive (Phase 2g).
- `std::bytes::from_string(s: String) -> Bytes` — copies the
  strlen-measured body into a length-prefixed blob (Phase 2g).
- `std::bytes::slice(b, lo, hi) -> Bytes` — half-open range
  copy with bound clamping (Phase 2g).

Consuming a `Bytes`:

- `len(b) -> Int` reads the length prefix.
- `std::bytes::at(b, i) -> Int` — byte-as-Int (0..255) with
  -1 sentinel for OOB (Phase 2g).
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
`__handle_one_connection` in `stdlib/io_tcp.ap`).

### Method-returning-locus heap allocation (m90)

When a method declares `-> SomeL` and instantiates a `SomeL`
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
both `return SomeL { ... };` and `let s = SomeL { }; ...;
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
   `accept(c: ChildL)` for locus membership; slots are for
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
`types.md`; at v1 it is opaque round-trip only (no read /
write through the cell — pending Map / Vec stdlib in v1.x).

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
allocated as a sub-region of the parent's arena via
`lotus_arena_create_subregion`. The child is freed wholesale
when the parent's arena dissolves. **Rich**-class parents do
not sub-region-allocate; accepted children get their own
top-level arenas. See `memory.md` Per-projection-class
allocation table.

This is existing v0 behavior; F.22 names it as "projection
class governs parent-override of slot 0" so future slot-1..N
parent-override (deferred to v1.x as `as_parent_for`) sits
on consistent vocabulary.

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

## Fallible call semantics (v1.x-FORM-1)

The runtime observes exactly one form of failure: closure
violation. Value-level `fallible(T)` returns are an
*addressing protocol* between immediate caller and callee —
they don't constitute a separate runtime mechanism. See
`notes/agent-onboarding/aperio-design-philosophy.md` § 2.

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

- **`or raise`**: raises a closure violation carrying `p`
  through the existing `bubble` / `on_failure` machinery.
  The closure violation is uniform-opaque to handlers — they
  may inspect `p` as diagnostic data but the payload is
  consumed at the `or` site if surgical recovery is needed.
- **`or <fallback>`**: binds `err` to `p` in scope and
  evaluates `<fallback>`. Its result is the expression's
  value. Type must match the success type.

Chains are right-associative: `a() or b() or raise` reduces
the value to the success type level by level.

### Process exit

If a closure violation bubbles past root with no `on_failure`
catching it, the process exits with the violation's payload as
the structured error report (same path as F.9). This is the
*only* way the runtime ends a program against its will.

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
