# Operational semantics

This document specifies what Hale programs *do* when they
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
`handle_one_connection` in `stdlib/io_tcp.hl`).

### `terminate`

`terminate;` (2026-05-30) ends the current locus's lifecycle
from inside one of its own methods — the locus analogue of
`return` (which ends a fn). It is only valid inside a locus
method body. It does **not** free anything directly: it sets
the locus's `__drain_requested` latch and exits the current
method like `return;`. When the method's `run()` coro completes
with the latch set, the runtime runs the locus's normal
`drain → dissolve → arena reclaim` — i.e. `terminate` *invokes*
the declarative teardown early; it is never a manual free.

Its purpose is **per-child reclamation on completion** for an
`accept`'d child whose lifetime is its own flow rather than its
parent's. An accept'd child on a daemon parent is otherwise
reclaimed only when the parent dissolves (never, for a daemon),
so per-connection children accumulate. A connection child whose
`run()` is a recv/park loop ending on EOF can `terminate;` (or
just `return;` once run-completion-reclaim for declared flows
lands) so its arena is reclaimed the moment its flow ends, while
the parent and the rest of the program keep running. Reclamation
is idempotent (the arena-destroy latch), so a `terminate` that
races the parent's eventual dissolve is torn down exactly once.

The reclaim runs on the coro's own pool worker, after `run()`
returns, while the locus's arena is still valid — never seizing
a still-executing frame. (A child that `terminate`s mid-`run()`
exits `run()` immediately, like `return`; code after `terminate`
in the same method does not execute.)

**Validity (typecheck, 2026-06-01).** `terminate;` in a free
function is a typecheck error — there is no enclosing locus whose
lifecycle to end. It is accepted in any locus method body
(lifecycle method or member `fn`).

**From a bus handler (2026-06-01).** `terminate;` is no longer
limited to `run()`. A subscriber can end its own life from inside
a bus handler (e.g. `on_close` receives a shutdown message and
calls `terminate;`). The reclaim runs when the handler returns —
the dispatch path checks the `__drain_requested` latch after each
handler and runs the spine on the handler's own worker. This is
the resident-subscriber analogue of the connection child that
`terminate`s from its `run()` recv loop.

### `release(c)` and flow children

`release(c: Child) { ... }` (2026-05-30) is the death-side
bookend, symmetric to `accept(c: Child)`. Declaring it on a
parent has two effects:

1. **It marks `Child` a *flow*.** A flow child is reclaimed when
   its `run()` *completes* — a plain `return` (or running off the
   end of `run()`), no explicit `terminate;` required. This is
   the connection model: `run()` is the connection's flow (a
   recv/park loop that returns on EOF), and the child's arena is
   reclaimed the moment that flow ends. A child whose type is NOT
   declared in any parent's `release` is a *resident*: its `run()`
   returning means "ready" (it lives on as a subscriber), and it
   is reclaimed only when the parent dissolves. The same
   `run()`-returns event thus means "reclaim me" for a flow and
   "ready" for a resident — disambiguated by the parent's
   declaration, never guessed.
2. **It fires on each completion.** When a flow child completes
   (via run-completion OR `terminate;`), the runtime calls
   `parent.release(owner, child)` — **after** the child drains,
   **before** it dissolves — so the parent observes the
   completion and reads the child's final settled state (the
   mirror of `accept(c)`, which reads it fresh). `release` is
   policy only: it does not free. The owner is the accept'ing
   parent, recorded at accept time; `release` does not fire if a
   flow-typed locus is instantiated outside an accept context
   (no owner).

`release` has the same shape as `accept` — one typed child
param — and the same fn signature `(parent_self, child_self)`.

**Validity (typecheck, 2026-06-01).** A `release(c: T)` with no
matching `accept(c: T)` on the same locus is a typecheck error: a
locus that never accepts a `T` child can never release one, so
the declaration is dead (almost always a wrong child type or a
forgotten `accept`).

**Parent-dissolve reclaim (2026-06-01).** When a parent that
`accept`s children dissolves, it reclaims each accept'd child it
still tracks — running the child's full teardown (drain →
dissolve → arena reclaim) before the parent's own arena (which
backs the children's subregions) is freed. This is what makes the
"resident reclaimed only when the parent dissolves" rule above
*observable*: a resident's `dissolve()` body (fd close, flush)
runs at the parent's graceful shutdown, rather than the child
being silently swallowed by the parent's wholesale arena free.
Flow children that already self-reclaimed mid-life are no longer
tracked, so they are not torn down twice; the per-child teardown
is idempotent regardless (an `__arena`-null latch). The cascade is
single-threaded-safe: cooperative pool workers are joined before
the dissolve cascade runs at program exit, so no worker can be
mid-dispatch to a child being reclaimed. (A parent reclaims only
children it *tracks* — i.e. one whose body iterates
`self.children`; a dispatcher that accepts but never iterates
holds no per-child handle and relies on flow/`terminate` reclaim.)

### Locus method dispatch

**Methods on loci may not return locus values.** This is the
load-bearing rule for locus method dispatch in Hale. The
compiler rejects any `fn` member of a locus whose declared
return type (or fallible-payload type) names a user-declared
locus.

#### Why this rule

Five design principles converge on the same constraint, which
is why the rule is shaped this narrowly:

- **CQRS** — queries return data; entities (loci) are managed
  structurally, not returned. A method returning a locus mixes
  command and query semantics in one call.
- **Law of Demeter** — a method that returns an entity puts
  that entity into a stranger position at every call site. The
  only ways to use it are LoD violations (call methods on a
  stranger) or pass-through (forward to another callee). Both
  shapes signal the method shouldn't have existed.
- **Dependency Inversion** — depending on a returned entity is
  depending on a concretion. The bus and `contract`-exposed
  fields are the abstraction surfaces for cross-locus
  coordination.
- **Single Responsibility** — a locus whose only purpose is to
  be the return value of a factory method has no responsibility
  of its own; it's a method-dispatch wrapper around state that
  lives elsewhere.
- **Mechanical sympathy** — every "method returns locus"
  call site triggers per-call allocation through the m90
  payload-arena routing (program-lifetime, never freed). The
  pattern leaks by construction. Removing the shape removes
  the allocation.

These aren't five rules layered on top of each other — they're
five lenses pointing at one structural error. The compiler
enforces it once.

#### The factory train wreck

The motivating violation is the cross-tower factory pattern:

```hale
locus Counter {
    store: Store;   // borrowed reference back to caller's state
    key: String;
    fn inc() { self.store.touch(self.key); }
}

locus Registry {
    fn counter(name: String) -> Counter {       // ← rejected
        return Counter { store: self.store, key: name };
    }
}

// caller:
self.reg.counter("ticks").inc();   // would leak per call
```

The `counter()` method declaration is the rejection site. The
diagnostic names three canonical alternatives:

1. **Parent-child + contract reads.** `Counter` becomes an
   accepted child of `Registry`; `Registry` reads counter
   state through the contract:

   ```hale
   locus Counter {
       params { name: String; value: Int = 0; }
       contract { expose value: Int; }
       fn inc() { self.value = self.value + 1; }
   }

   locus Registry {
       accept(c: Counter) { /* default registration */ }
       fn inc(name: String) {
           // iterate self.children, find the matching counter,
           // call c.inc(). Vertical method dispatch on owned
           // child — friend access, no LoD violation.
       }
   }
   ```

2. **Bus topic (mediator).** Counters publish events; Registry
   subscribes:

   ```hale
   topic Inc { name: String };
   Counter::Inc { name: "ticks" } -> Inc;
   ```

   Closed-world rewrite (when its preconditions hold) collapses
   the bus round-trip to a direct dispatch — same cost as
   delegation, without the typed-handle loss.

3. **Delegation.** `Registry` exposes the operation directly:

   ```hale
   self.reg.inc("ticks");
   ```

   Loses the typed handle but doesn't allocate. Acceptable when
   the caller has only a few counters to touch.

#### Owned-child + contract is the canonical "B's data feeds A" shape

When locus A needs to read and update derived state computed
from its own input, the canonical pattern is **B as an
owned-child field of A**, with the update going through a
vertical command (`self.b.compute(...)`) and reads going through
a vertical contract exposure or method call on the child:

```hale
locus Segment {
    params { /* accumulator state */ }
    fn clear() { /* reset */ }
    fn push(t: Float, v: Float) { /* update */ }
    fn slope() -> Float { /* compute */ }
    fn intercept() -> Float { /* compute */ }
}

locus LeadingEdge {
    params {
        // ring buffer fields
        seg: Segment = Segment { };   // owned-child field
    }
    fn fit() {                         // command, returns nothing
        self.seg.clear();
        // replay ring contents into self.seg
    }
    fn slope() -> Float {              // query, returns data
        self.fit();
        return self.seg.slope();       // vertical method on owned child
    }
}
```

The `54-geom-leading-edge` example fixture demonstrates this
shape end-to-end. The earlier "factory return" form
(`fn fit() -> Segment`) was the pattern this rule rejects.

#### What's not rejected

The rule fires only on `fn` members of a locus. It does not
catch:

- **Free fns returning loci** — entity creation patterns like
  `std::io::file::open(path: String) -> File fallible(IoError)`
  are constructors, not factory methods on existing loci.
- **Methods returning primitives, records, or fallible-of-those.**
  `BytesBuilder.finish() -> Bytes` is fine; `LeadingEdge.slope()
  -> Float` is fine.
- **Methods returning nothing.** Commands stay commands.
- **Namespace-lotus pattern.** `__StdLangLang.parse(src) -> Int`
  is fine — the locus's methods return data, not loci.
- **Lifecycle / mode / failure handler bodies.** These don't
  have value-bearing return types.

#### Migration

There is no opt-out annotation. The rule is the language's
structural axiom for locus methods — programs that violate it
are mis-designed, and the diagnostic names the canonical
alternatives. Migrating from the factory shape to one of the
three alternatives is a refactor, not a switch flip.

The runtime m90 routing (see § Method-returning-locus heap
allocation below) survives only to cover the few remaining
shapes where a locus value transits through the m90 path
indirectly (e.g., interface returns from free fns). With
factory methods stopped at the declaration site, the dominant
trigger of the m90 leak goes away by construction.

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

```hale
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

```hale
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

```hale
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

  **Subscribers (Form K6b, 2026-05-20).** Hale-side
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

  **Birth-order trap (single-binary + `block`).** Hale
  births child loci in `params`-declaration order. In a
  single-binary deployment where a Producer's `birth()`
  immediately publishes onto a `block`-policy ring, the
  Consumer locus MUST be declared *before* the Producer in
  the parent's `params` block — otherwise the Producer's
  first overflow blocks on a `consumer_seqno` that no live
  reader will ever advance, and the process hangs. Order
  the consumer first or move the publishing into a `run()`
  body that runs after all child births. (Cross-binary
  deployments aren't affected — the subscriber lives in a
  different process and exists before the publisher
  process starts.)

**Foreign rings via `ring_layout` (Proposal B, 2026-06-06).**
The shm_ring transport above reads/writes the *native* Lotus
ring (the `LRSRNG1` header + equal-sized slots). To read a ring
defined by *another* program — an externally-defined binary
broadcast ring — a `ring_layout` declaration describes that
ring's binary shape, and a binding references it with the
`layout:` kwarg:

```hale
ring_layout ForeignRing {
    magic 0x52494E47464D5431;        // expected header magic at offset 0
    version 1 at 8 : u32;            // header field `version`: expect 1
    buffer_size at 12 : u32;         // ring data capacity, read from header
    data_at 128;                     // first-record byte offset
    cursor published {               // the published byte cursor
        at 64; repr atomic_u64; load acquire; unit bytes;
    }
    framing byte_records {           // records are [u32 len][payload]
        len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF;
    }
    overflow lap_detect;
}

main locus App {
    bindings {
        Ticks: shm_ring("/foreign.ticks", on_overflow: drop,
                        layout: ForeignRing) where zero_copy;
    }
}
```

The `layout:` reference must resolve to a declared `ring_layout`
(else a typecheck diagnostic). A binding with no `layout:` is the
native ring, unchanged.

*Slot rings (`framing slots`).* The example above is a variable-length
`byte_records` ring. A `slots` framing describes a fixed-stride slot
ring instead — the shape of the native Lotus ring itself. The geometry
(`slot_size`, `slot_count`) is read from the foreign header rather than
fixed in the layout, the cursor is the published seqno (1-based; `unit
slots`), and slot *S* lives at `data_at + (S mod slot_count) ×
slot_size`. A consumer skips a seqno the producer has already lapped
(`published − S ≥ slot_count`) rather than read a torn slot — matching
the native reader. This makes the native `LRSRNG1` ring expressible as a
`ring_layout`, read through the same abstraction as a foreign one:

```hale
ring_layout LotusRing {
    magic 0x4C5253524E4731;          // "LRSRNG1"
    slot_size  at 8  : u64;          // geometry read from the header
    slot_count at 16 : u64;
    data_at 128;                     // first slot (after the 2-cache-line header)
    cursor published { at 24; repr atomic_u64; load acquire; unit slots; }
    framing slots { }
    overflow lap_detect;
}
```

The producer side for a foreign `slots` ring (a Hale writer) is not yet
offered; `slots` is a consumer framing at this version.

*The layout contract.* A `ring_layout` is validated at typecheck
(`hale-types::check`), so a malformed layout fails the build, not
the read. The rules:

- Each scalar `repr` must be a known fixed width — `u8`/`u16`/
  `u32`/`u64`, `i8`/`i16`/`i32`/`i64`, `f32`/`f64`.
- A `cursor` block needs an `at` offset, a known `repr`
  (`atomic_u64`), a known `load` memory ordering (`relaxed`/
  `acquire`/`release`/`acq_rel`/`seq_cst`), and a `unit` of
  `bytes` or `slots`. At least one cursor is required.
- `framing` kind is `byte_records` or `slots`. `byte_records`
  requires a `len_prefix` (and reads capacity from a `buffer_size`
  scalar); `slots` requires `slot_size` and `slot_count` scalars (the
  consumer reads the slot geometry from the foreign header, and derives
  capacity as their product).
- All offsets are non-negative.
- A `ring_layout` is a declaration, not a value — referencing its
  name in expression position is an error.

*Cross-field conformance.* Because the foreign format is fixed and
unchangeable, a layout that mis-transcribes it is the program's own
bug — and several of these fields silently corrupt the reader if
wrong, so they are caught at compile time:

- Every header scalar and the cursor (an 8-byte atomic) must lie
  *before* `data_at` — a field whose `[at, at+width)` overruns the
  data region is rejected.
- No two header fields (or a field and the cursor) may overlap.
- `byte_records` `align` must be a power of two — it is the
  record-stride alignment the reader masks with.
- `pad_sentinel` must fit in the `len_prefix` width; otherwise wrap
  detection reads a truncated value and never fires.
- `len_prefix` width must be `<= align`, and a producer's
  compile-time `buffer_size:` must be a multiple of `align` — else a
  record header could land in `(cap - len_prefix_width, cap)` and read
  or write past the data region.
- An `atomic_u64` cursor's `at` must be 8-byte aligned (an unaligned
  atomic load is undefined); `magic`, `data_at` (for `byte_records`),
  and a `buffer_size` scalar must all be present.

*Payload conformance at the binding.* A `layout:`-bound topic's
payload picks the consumer mode:

- A **flat-shapeable struct** → *typed mode*: the record is read by a
  direct pointer-cast (and, on the producer side, written by a `memcpy`
  of the payload struct — the foreign record bytes *are* the Hale
  struct, bindgen-style). The framed `len` must equal the struct's
  fixed size or the record is resynced (the OOB guard above). Enforced
  whether or not the binding also asserts `where zero_copy`.
- A **`BytesView`** → *raw-frame mode*: for heterogeneous /
  variable-length rings (e.g. a discriminated-union feed). The
  consumer can't assume a fixed size, so `value_size` is 0 (the
  size gate is off) and the handler receives a bounded `BytesView`
  over each record — it decodes with `std::bytes::read_*` + a
  discriminator branch. The framed-size bounds checks against the ring
  still apply; the record payload is copied into a Bytes-shaped scratch
  blob (the pack readers need that prefix, and the mapping is
  read-only), so raw-frame mode is not zero-copy. The producer side is
  symmetric: `Recs <- bytes` (a `Bytes` or `BytesView` value, e.g. built
  with a `BytesBuilder`) frames `[len_prefix len][bytes]` where `len` is
  the value's actual byte length, so a producer can emit
  heterogeneous / variable-width records — the runtime publish path is
  size-generic. For a *zero-copy* write, `Topic.write(max) { w => ... ;
  len }` reserves up to `max` bytes, binds a writable `BytesMut` view `w`
  over the slot (written with the `std::bytes::write_*` family, the
  bounds-checked mirror of the readers), and commits the byte count the
  body's tail yields — the producer writes record fields directly into
  the mapped ring with no intermediate buffer. The reserve and commit are
  scoped to the block, so the view can't escape and the commit can't be
  forgotten.
- A struct field may carry a Go-style backtick metadata tag after its
  type (`price: Int `repr:"u32_le"`;`) — free-form `key:"value"` metadata
  stored on the field. A `repr:"<wire-type>"` key makes the struct a
  binary layout: `Type::field(v)` reads that field from a `Bytes` /
  `BytesView` and `Type::set_field(w, x)` writes it into a `BytesMut`, at
  the field's offset (computed in declaration order over the tagged
  fields, or pinned with `,at=N`). These desugar to the matching
  `std::bytes::read_*` / `write_*` call, so they share the primitives'
  bounds-checking and cost.
- A `json:"<key>"` tag is the second tag consumer: a struct with at least
  one `json:` tag gets a generated `Type::from_json(s) -> Type
  fallible(JsonError)` that parses the object in a single pass (driving
  the `std::json` object cursor), dispatching each key to the matching
  field and reading the value by the field's declared scalar type
  (`Int` / `Float` / `Bool` / `String`). The key is the tag value, else
  the field name; unmatched keys (and nested objects/arrays under them)
  are skipped. A missing field raises `JsonError { kind, field }` unless
  the field declares a literal default (`= "USD"`), which fills it.
  `from_json` is `fallible`, so callers must address it. Nested-struct
  and array fields are a follow-up. Further tag keys remain reserved for
  future consumers (validation, db mapping).

Any other payload (with `String`, `Bytes`, or variable-size fields and
not itself `BytesView`) is rejected.

*Out-of-bounds safety.* The guarantee is that a wrong layout — or a
non-conforming / hostile foreign producer — yields **wrong values,
never an out-of-bounds access**. It holds given the checks above
plus the runtime's boundary defenses: the consumer rejects, at
attach, a foreign `buffer_size` that isn't a multiple of `align`;
each record's len-prefix read is clamped within the data region; and
the framed `len` must equal the bound payload's fixed size before the
handler is invoked (a short record is resynced, never dispatched), so
the handler cannot read past a record near the wrap. The bound checks
are overflow-safe against a hostile `len` or offset.

The member token positions (`acquire`, `atomic_u64`, `bytes`, and
words that collide with keywords like `release`) are layout
*words*, not Hale type expressions — bare identifiers (or
keyword-spelled words) checked against the sets above, never
resolved as types.

*Consumer (read).* A subscriber on a layout-bound topic registers
via `lotus_bus_register_subscriber_shm_ring_layout(subject, name,
desc, self, handler)`, where `desc` is a flat 16-entry descriptor
codegen builds from the resolved layout. The runtime attaches the
foreign segment read-only (it never creates it — the foreign
producer owns the ring), validates the magic and `version`, reads
`buffer_size` for the data-region capacity, then runs a
`byte_records` reader thread: acquire-load the published byte
cursor, and for each record walk `data_at + local % capacity`,
read the `len_prefix`, skip a `pad_sentinel` tail-pad to the wrap,
hand the payload view to the handler, and advance by
`align_up(len_prefix + len, align)`. Field *roles* are read by
convention from the layout — a scalar named `version` (with an
expected value) is the version check; one named `buffer_size` is
the capacity source.

*Producer (write).* If the bundle *publishes* a layout-bound topic,
it is the ring's single producer (SPMC): the prelude CREATES the
segment via `lotus_bus_register_shm_ring_layout(subject, name,
desc, capacity)` — sizing it `data_at + capacity`, writing the
header (magic, `version`, `buffer_size = capacity`) and zeroing the
cursor — and each `Topic <- value` routes through
`lotus_bus_publish_shm_ring_layout`, the exact inverse of the
reader: reserve `align_up(len_prefix + payload, align)`, write a
`pad_sentinel` and wrap if the record would straddle the end, write
the length prefix + a `memcpy` of the (flat) payload, then
release-store the cursor. Capacity comes from the binding's
`buffer_size:` kwarg (bytes; a per-transport default applies when
omitted). A layout binding that is only *subscribed* in this bundle
creates nothing — it attaches the foreign producer's ring.

*Limitations (v1).* A subscriber reads records published *after*
it attaches (no historical replay) — so an in-process producer must
not publish before the consumer's reader thread has started (a
non-issue for an external long-running producer like an external market-data feed). Lap
handling is lossy + safe: if the producer runs more than `capacity`
bytes ahead, the missed bytes are gone, so the reader resyncs to the
producer's cursor (a commit boundary) and resumes rather than
reading a torn record. Handlers run on the reader thread (same
constraint as the native subscriber). The `slots` framing kind, the
zero-copy writable producer view (A1 — the producer copies the
payload once today), and multi-cursor back-pressure are post-v1.

**In-memory delivery is absence-of-entry.** A topic with no
binding entry is delivered same-process via the cooperative
queue. There is no `in_memory` variant — the runtime default
covers the case and explicit syntax would be ceremony.

**Operational constraints (Form K, 2026-05-20).** A binding
entry may carry an optional `where` clause listing
constraint keywords the dev team asserts the route must
satisfy:

```hale
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
(synchronous instead of cooperative-deferred) — *provided
publisher and subscriber share an execution context* (see the
placement carve-out below; that condition is what keeps the
rewrite observably transparent).

Out of scope for v1 (fall through to bus dispatch unchanged):

- Multi-hop towers (`Outer → Middle → Leaf`).
- Plural / vec / capacity-slot children — broadcast semantics
  don't match the singleton-rewrite shape.
- Child-publishes-parent-subscribes — needs a parent-reference
  mechanism that doesn't exist in v1.
- A parent with multiple direct fields of the subscriber type
  (ambiguous receiver).
- **Off-thread subscribers (F.31 placement).** When the
  subscriber is a main-locus field placed on a cooperative pool
  other than `main`, or on a pinned thread, the direct call would
  run the handler on the *publisher's* thread instead of the
  subscriber's pool worker — breaking the single-threaded-pool
  invariant and dropping the pool context that any locus the
  handler instantiates must inherit (an accept'd child's `run()`
  would otherwise go synchronous, and its `subscribe`s would
  register on the global queue rather than the pool). Such
  publishes stay on the bus dispatch path, which posts to the
  subscriber's pool via `lotus_coop_pool_post`. `cooperative` with
  no pool (or `pool = main`) keeps the subscriber on the
  publisher's thread, so it remains eligible.

A bound topic is never optimized: the binding may publish to
remote subscribers that aren't visible at compile time.

### Phase 3: routing keys (v0.1 proposal, 2026-05-25)

Phase 3 extends topic declarations with a per-message **routing
key** so the bus can shard dispatch by key value at the
`(subject, key)` granularity, rather than fanning every published
message to every subscriber on the subject. Motivated by the
fathom `apps/mdgw/kraken` workload (handoff
`handoff-compiler-hashmap-bigcell-leak-2026-05-25.md`): one
reader thread publishes book frames for N symbols; N per-symbol
loci each want only their own symbol's frames. Without routing
keys, every BookSignal would receive every L2Data frame and have
to filter in user code — O(N × messages) dispatches; per-symbol
state corruption pressure if filtering is forgotten.

Routing keys are also reusable for any "many similar loci sharing
one publisher" pattern (per-tenant request streams, per-account
ledger updates, etc.).

**Surface — three pieces.**

```hale
type L2Data {
    sym_id:   Int;                          // i64 routing key field
    bids:     [BookLevel; 100];
    asks:     [BookLevel; 100];
}

topic KrakenL2 {                            // (1) topic-decl additions
    payload:      L2Data;
    subject:      "kraken.l2";
    keyed_by      sym_id;                   //  ←  new
    on_unmatched: swallow;                  //  ←  new (default if absent)
}

locus BookSignal {
    params { sym_id: Int = 0; ... }
    bus {                                   // (2) subscribe-clause filter
        subscribe KrakenL2 as on_l2
                  where key == self.sym_id; //  ←  new
    }
    fn on_l2(d: L2Data) { /* d.sym_id == self.sym_id, statically */ }
}

main locus Mdgw {                           // (3) per-instance bindings
    params {
        btc: BookSignal = BookSignal { sym_id: 1 };
        eth: BookSignal = BookSignal { sym_id: 2 };
        sol: BookSignal = BookSignal { sym_id: 3 };
    }
}
```

**Width is inherited from the `keyed_by` field's type.** No
separate width annotation. Acceptable field types and their bus
storage at v0.1:

| Field type | Bus storage | Compare cost |
|---|---|---|
| `Bool` | u64 (zero-extended) | one i64 cmp |
| `Int` | u64 | one i64 cmp |
| `Time`, `Duration` | u64 (ns since epoch) | one i64 cmp |
| no-payload `enum` | u64 (i32 tag zero-extended) | one i64 cmp |
| `Decimal` | u128 (i64 pair) | two i64 cmps |

The bus runtime stores both halves of a u128 uniformly
(`key_lo: u64, key_hi: u64`) — narrower types zero-extend. Apps
that need compound keys (`(sym_id, venue, side)`) pack them into
a `Decimal` field themselves; the language does not bake compound-
key derivation at v0.1.

**`where key == EXPR` — what EXPR can be.**

The RHS is evaluated at the subscribing locus's instantiation (the
point where its `params` defaults are resolved), and the resulting
key value is captured into the bus registry alongside the
handler's self pointer. v0.1 restricts EXPR to:

1. An integer / decimal literal: `where key == 42`
2. A const identifier resolving to a scalar of the topic's key type
3. A `self.<field>` path read, where `<field>` is a `params`-block
   field of the subscribing locus

Higher-shape expressions (`self.a + self.b`, method calls in the
filter, cross-locus reads) are reserved for later. The
restriction keeps the static check simple ("EXPR is a let-
bindable expression with no side effects, types to the topic's
key type") and avoids surprising semantics at registration time.

**Key stability — captured by value at register.**

A routing-key subscription captures its key value at the locus's
instantiation (or restart). Subsequent mutations to fields the
filter expression references do **not** change which messages the
handler receives. If dynamic re-keying is needed, dissolve and
re-instantiate the locus. The alternative — re-evaluating the
filter on every dispatch — would break the bus's "register once,
dispatch many" cost model and introduce ordering complexity
against concurrent `self` mutation; the capture-by-value rule is
the right default, and a `re-subscribe` API is a follow-up if a
workload demands it.

**`on_unmatched: V` — policy when no subscriber's key matches.**

A keyed publish may find zero subscribers whose `where key == X`
filter matches the message's key. Topic-level config picks the
behavior; default is `swallow` (matches today's no-subscriber
semantics on unkeyed topics).

| `on_unmatched:` | Behavior |
|---|---|
| `swallow` *(default)* | Drop the message silently. Diag visible only with `LOTUS_BUS_LOG_UNMATCHED=1` env var (per-publish stderr line citing subject + key + subscriber counts). |
| `fail` | Publish becomes a fallible expression. Caller must attach an `or` disposition: `K <- value or raise` panics via `lotus_root_panic` with a `BusUnmatchedKey` marker; `K <- value or discard` silently swallows on no-match. The err-payload-carrying dispositions (`or handler(err)` / `or fail <p>`) are reserved for v0.2 — they require synthesizing `BusUnmatchedKey { subject: String, key_lo: Int, key_hi: Int }` as a stdlib type, which is a small follow-up. `or <substitute>` is permanently rejected: Send produces no value, nothing to substitute. |
| `fallback` | A catch-unmatched subscriber on the subject — `subscribe T as h where key == _` — receives the message. At least one such subscriber is required; cross-module resolve-time check rejects the topic otherwise. The `_` sentinel is legal only on `fallback` topics. |

Static checks at typecheck:

1. `keyed_by FIELD` — FIELD must be a declared field of the
   topic's payload type; FIELD's type must be one of the table
   above; the topic must not also declare a `keyed_by` via a
   parent topic with a different field.
2. `where key == EXPR` — EXPR's type must match the topic's
   keyed-by field type after width inference.
3. `where key == EXPR` is forbidden on topics without
   `keyed_by`; rejecting prevents silent-no-match bugs from
   typo'd filters.
4. `where key == _` is forbidden except on topics with
   `on_unmatched: fallback`.
5. `fail` topics: every `Topic <- value` send site must carry an
   `or` disposition clause (`or raise` / `or discard` at v0.1
   of the impl; `or handler(err)` / `or fail <p>` reserved for
   v0.2). The `or` clause attaches to the Send statement, not
   the value expression — the parser strips it off the value's
   `Expr::Or` wrapping into `Stmt::Send.or_disposition`.
   Conversely, an `or` clause on a Send to a non-`fail` topic
   is rejected.
6. `fallback` topics: at least one program-wide `where key == _`
   subscriber must exist; checked at resolve after import
   merging.

Routing keys are orthogonal to topic hierarchy (Phase 2): a
parent's `keyed_by` and `on_unmatched` are inherited by children
that don't override; children may override either independently.
A child topic re-declaring `keyed_by` must agree with the
parent's key type (subjects derived from a parent's wire prefix
share the parent's key shape — anything else makes dispatch
ambiguous on the wire).

**Runtime — `lotus_bus_entry_t` extension.**

The bus router's subscriber-entry struct
(`crates/hale-codegen/runtime/lotus_arena.c`, around line 4034)
gains a tri-state filter and a u128 key value:

```c
typedef struct {
    /* ...existing fields: subject, self_ptr, handler, etc... */
    uint8_t  key_filter_kind;     /* 0 = no filter (receive-all)
                                   * 1 = specific key
                                   * 2 = catch-unmatched (`_`) */
    uint64_t key_lo;              /* i64 key, or low half of i128  */
    uint64_t key_hi;              /* 0 for i64 / narrower types    */
} lotus_bus_entry_t;
```

Dispatch (`lotus_bus_local_dispatch_keyed`):

```c
int matched_specific = 0;
for (entry in g_bus_entries with matching subject):
    if (entry.key_filter_kind == 1
        && entry.key_lo == msg.key_lo
        && entry.key_hi == msg.key_hi) {
        fire(entry);
        matched_specific = 1;
    } else if (entry.key_filter_kind == 0) {
        fire(entry);                  /* unkeyed receive-all */
    }
if (!matched_specific) {
    for (entry in g_bus_entries with matching subject):
        if (entry.key_filter_kind == 2) fire(entry);
}
```

Walk cost is O(N_subscribers_on_subject) for the specific pass,
with a second pass only when there's no specific match (fallback
case). For workloads with thousands of keyed subscribers per
subject, a per-`(subject, key_lo, key_hi)` open-addressing index
can be added later — YAGNI until a workload demands.

Two new runtime symbols:

```c
void  lotus_bus_register_keyed(
        const char *subject, void *self,
        lotus_handler_fn handler,
        /* ...existing... */,
        uint8_t  key_filter_kind,
        uint64_t key_lo,
        uint64_t key_hi);

int   lotus_bus_dispatch_keyed(           /* returns match count;
                                           * `fail` topics check this */
        lotus_bus_queue_t *queue,
        const char *subject,
        const void *payload, size_t payload_size,
        uint64_t key_lo, uint64_t key_hi,
        lotus_serialize_fn serialize_fn);
```

Existing `lotus_bus_register` / `lotus_bus_dispatch` stay as
the unkeyed entry points (compat for unkeyed topics).

**Backward compatibility.** Topics without `keyed_by` and
subscribers without `where key ==` behave exactly as today —
the new fields default to `key_filter_kind = 0` (receive-all)
and the dispatch dispatches uniformly. Existing programs need
no source change to keep working; new programs opt in
per-topic.

**v0.2 (2026-05-26) — err-payload Send dispositions.**

On `on_unmatched: fail` topics, all four `or` disposition shapes
are now supported:

- `or raise` — no-match panics via `lotus_root_panic` (v0.1
  shape; unchanged).
- `or discard` — no-match silently swallowed (v0.1 shape;
  unchanged).
- `or <expr>` — evaluates `<expr>` for side effects on no-match,
  with `err: BusUnmatchedKey` in scope. The expression's value
  is discarded (Send is statement-level). Canonical use:
  `or log_unmatched(err)` — a free fn that takes the err
  payload and logs / metrics / etc.
- `or fail <payload>` — only legal inside an enclosing
  `fallible(E)` fn. On no-match, evaluates `<payload>` (with
  `err: BusUnmatchedKey` in scope), stores it into the
  enclosing fn's err slot, and diverts to the fn's err-exit
  path. Symmetric to `or fail` on fallible-method calls.

`BusUnmatchedKey` is a synthesized stdlib type, injected into
scope when any topic declares `on_unmatched: fail`. Layout:

```hale
type BusUnmatchedKey {
    subject: String;     // wire subject of the failing publish
    key_lo:  Int;        // low 64 bits of the unmatched key
    key_hi:  Int;        // high 64 bits (0 for i64 keys)
}
```

Codegen allocates a fresh `BusUnmatchedKey` in the current
arena on the no-match branch and binds it as `err` for the
disposition expression's lowering. Mirror of the existing
`KeyError` / `IndexError` / `IoError` synthesis pattern.

**Out of scope at v0.1 (explicit non-goals):**

- Multi-field / tuple `keyed_by` (`keyed_by (sym_id, side)`).
  Apps that need compound keys pack into a `Decimal` field in
  user code. Eligible for a v0.2 sugar once a workload's
  ergonomics surface the friction.
- Wildcard key sets (`where key in [1, 2, 3]`).
- Range predicates (`where key > 100`). Equality match is the
  workload-driven sweet spot; broader predicates would defeat
  the O(1)-dispatch cost model.
- String routing keys. String-equality match defeats the
  perf goal; only int-shaped scalars at v0.1.
- Method blocks on enums to make enum-typed routing keys
  ergonomic. Treated as a separable language feature; users at
  v0.1 use bare Int / Decimal or existing no-payload enums.
- Per-publish key override (publishing with a key value that
  isn't derived from the payload field). Tied to the payload's
  identity so wire-format consumers see a consistent key.
- Cross-process keyed dispatch (over the remote-fanout path).
  v0.1 ships keyed dispatch for the intra-process bus only;
  remote subscribers still fanout per-subject and filter in
  their own bus router after deserialize.
- `or <substitute>` on Send for SUCCESS-value substitution.
  Send produces no value to substitute. v0.2's `or <expr>`
  disposition on fail topics is a side-effect handler call
  (evaluates `<expr>` with `err` in scope, discards the value),
  not a value substitution.

## Placement block (F.31)

The `placement { }` block on `main locus` controls per-locus
thread placement, parallel to `bindings { }` for bus topology.
Placement is a deployment seam — same library, different
placement entries, different binary behavior. See
`spec/design-rationale.md` § F.31 for the intrinsic-vs-
deployment axis the block sits on, and `spec/runtime.md` §
"Placement classes" for the runtime semantics.

### Syntax

```hale
main locus App {
    params {
        gateway_kraken:   Gateway = Gateway { venue: "kraken" };
        gateway_coinbase: Gateway = Gateway { venue: "coinbase" };
        metrics:          MetricsServer = MetricsServer { port: 9100 };
        ui:               Renderer = Renderer { };
    }
    placement {
        gateway_kraken:   pinned(core = 1);
        gateway_coinbase: pinned(core = 2);
        metrics:          cooperative(pool = io);
        ui:               cooperative(pool = render);
        // unspecified main-locus params → cooperative(pool = main)
    }
}
```

### Type-check rules

> These placement/bus rules are part of the broader compile-time
> verification surface; `spec/verification.md` is the canonical
> catalog of every static check (with severities and enforcing passes).

1. **`placement { }` is `main locus` only.** Any other locus
   declaring `placement { }` is a parse error (same shape as
   `bindings { }`).
2. **Keys reference main-locus `params` field names.** The key
   on the left of each `placement_entry` must match a declared
   `params` field on the enclosing `main locus`. Unknown
   field name → typecheck error pointing at the params block.
3. **Field values are locus types.** A placement entry on a
   non-locus field (`port: Int`, `host: String`) is a typecheck
   error — placement applies only to locus instances.
4. **At most one placement entry per field.** Duplicate keys
   are a parse error.
5. **Pool names use snake_case Idents.** The set of pool names
   is inferred from `cooperative(pool = X)` references across
   all placement entries in the bundle. Pool `main` is always
   available; it refers to the program's main OS thread.
6. **Locus-pinning compatibility.** A locus placed `pinned` is
   subject to the existing pinned-class restrictions (no
   `accept(c: Child)` accept-method, no `closure` declarations
   in v1). These restrictions move from the locus declaration
   site (pre-F.31) to the placement site: the typechecker walks
   each placement entry and applies the relevant restriction to
   the named locus type. A locus that uses neither feature can
   be placed either cooperative or pinned at the deployment's
   discretion.
7. **Dead bus receiver (error).** A locus that declares
   `bus { subscribe ... }`, is placed `cooperative(pool = X)` with
   `X != main` (and not `where async_io`), **and** whose `run()`
   makes a known-blocking stdlib call is rejected. A cooperative
   locus receives cells only while its pool thread is free to run
   the dispatch (the cross-process transport reader dispatches
   into the handler set; the in-process cooperative queue is
   drained at yield points). A blocking call monopolizes the pool
   thread, so the dispatch never runs and the subscriber's handlers
   never fire. **Placement alone is not the condition** — an
   event-driven subscriber that yields (handlers plus a
   `time::sleep` loop, or `where async_io`, which parks) receives
   fine and is *not* flagged; only the blocking-and-subscribing
   combination is rejected. A subscription to a topic the locus
   also *publishes* is also spared (an intra-locus
   self-publish→subscribe is devirtualized to a direct call). Fix:
   `pinned` (own thread + a mailbox drained at sleep/yield) or keep
   `run()` non-blocking. (Corrected 2026-06-03: an earlier form
   rejected on placement alone and over-fired on event-driven
   non-main cooperative subscribers, which receive reliably.)
8. **Blocking syscall on a cooperative pool (warning).** A locus
   placed `cooperative(pool = X)` *without* `where async_io`
   whose `run()` calls a known-blocking stdlib op (tcp/tls recv,
   accept, `process::run`/`wait`) but is *not* a dead receiver
   (rule 7) gets a **warning** (not an error — hale's only
   non-fatal diagnostic). A blocking call holds the pool's OS
   thread for its whole duration, stalling every other locus
   scheduled on that pool and the pool's bus drain. The fix is
   `pinned` (its own thread — the prescribed shape for blocking
   I/O) or `cooperative(pool = X) where async_io` (parks on I/O
   readiness). It is a warning rather than an error because a
   single-purpose blocking server with nothing co-scheduled is
   legitimate. Detection is **interprocedural**: it follows the call
   graph, so a `run()` that blocks indirectly — through a helper fn
   or a `self.method` it calls (transitively) — is flagged too, with
   the diagnostic naming the offending call. It remains best-effort
   at the edges (blocking via a *handle* method like `stream.recv`,
   or across a cross-locus `self.field.method()` hop, isn't traced).
   Note rule 7 (the dead-receiver *error*) stays **direct-call-only**
   — it is not widened onto indirect paths, so the higher-stakes
   diagnostic keeps its precision.
9. **Orphan bus topic (warning).** In a closed-world program (a
   `main` locus present), a bus subject — a declared `topic` or a
   literal string — wired to only one end is flagged: *published with
   no subscriber* (the cells go nowhere) or *subscribed with no
   publisher* (the handler can't fire), and a declared topic touched
   by neither is *dead wiring*. Suppressed when the other end is
   plausibly external: a **transport binding** (`bindings { T: ... }`
   implies a cross-process peer), a **wildcard** subscriber/publisher
   covering the subject (`log.**` covers `log.app`), a **cross-seed**
   reference (`alias::Foo` — the other seed owns the other half), or
   the same locus being both publisher and subscriber. The closed-
   world gate is why this is skipped for library seeds (no `main`):
   their consumers are downstream, out of the bundle. (GH #18 #4.)
10. **Bus cycles.** An edge `S →(L) D` exists when locus `L` subscribes
    subject `S` with a handler that sends to subject `D`. A cycle in
    this graph is a publish→subscribe→publish loop, and the dispatch
    model splits it two ways:
    - A **cross-locus** cycle (edges from ≥2 loci) hops between loci
      through the cooperative *queue* (drained at yield) — it spins
      the queue / livelocks → **warning**.
    - An **intra-locus** cycle (every edge in one locus) is
      intra-locus self-dispatch, which is devirtualized to a direct
      synchronous call (rule 7), so it recurses on one thread without
      bound → stack overflow → **error**. To keep the error precise,
      only **unconditional** sends form intra-locus edges: a
      self-republish guarded by an `if`/`match`/loop is a terminating
      state machine, not unbounded recursion, and is not flagged.
      (GH #18 #4.)
11. **Bus backpressure (warning).** A locus that publishes to the bus
    inside an **unbounded** `while true` loop carrying no flow-control
    or exit point — no cooperative `yield`, no `time::sleep`/`tick`
    throttle, no input-pacing blocking `recv`, no `break`/`return` —
    has no backpressure: it posts cells faster than any subscriber can
    drain, so the queue and the payload arena grow without bound. A
    full producer-vs-consumer rate analysis is undecidable, so this is
    a deliberately narrow structural heuristic (warning): only literal
    `while true` loops are considered (bounded `for`/`while cond`
    loops never are), and any flow-control point anywhere in the loop
    body clears it. (GH #18 #4.)
12. **Bus subject type-mismatch (error).** Every publish/subscribe
    site addressing the same **literal** subject string must declare
    the same `of type` payload — otherwise a subscriber decodes the
    publisher's bytes as the wrong type at runtime. A declared `topic`
    is already unified by its declaration (and `of type` is forbidden
    on topic refs), so this closes the literal-subject gap.
    Grouping is by *exact* subject string, which excludes wildcards
    (`log.**` and `log.app` are different strings, never
    cross-compared). The fix is to declare a `topic` (one payload
    type, fixed in one place) or align the `of type` annotations.
    (GH #18 #4.)

### Single-threaded-method invariant

A locus's methods may be invoked only on the OS thread that
owns the locus's placement's pool. This is enforced at
typecheck via a static call-graph walk starting from each
top-level placement entry:

1. Seed each placement entry with its pool: `gateway_kraken`
   → pinned (own thread), `metrics` → cooperative pool `io`,
   etc.
2. For each method call expression `recv.foo(args)`, determine
   the receiver's pool from the receiver's static type and the
   surrounding pool context.
3. Cross-pool direct method calls are rejected with a focused
   diagnostic naming both pools and pointing at the
   `placement { }` entries that picked them.
4. Bus publishes (`Topic <- payload;` / `"subj" <- payload;`)
   are unrestricted — they route through the substrate's
   cross-thread dispatch machinery (the existing m28b
   condvar+memcpy mailbox path generalized to cooperative
   pools).

This invariant is the substrate enforcement that makes M:N
safe. Without it, multi-pool deployments would silently race
on locus arenas (which are unsynchronized bump allocators by
design).

### Nested instantiation

Loci instantiated nested in another locus's body (`birth` /
`run` / lifecycle methods, let-bound children, or `params`
fields of non-`main` loci) inherit their containing tower's
pool by construction:

```hale
main locus App {
    params {
        gw: Gateway = Gateway { };
    }
    placement {
        gw: pinned(core = 1);
    }
}

locus Gateway {
    params {
        // Cache instantiated nested in Gateway's params —
        // inherits Gateway's pool (the pinned thread). No
        // placement entry is permitted on `cache` at any
        // main locus.
        cache: Cache = Cache { };
    }
}
```

Placement entries on nested fields are a typecheck error —
placement is a top-level main-locus surface only. To run a
nested locus on a different pool, hoist it to a main-locus
sibling.

### Default placement

Main-locus `params` fields with no explicit `placement { }`
entry default to `cooperative(pool = main)`. The pre-F.31
shape (`: schedule cooperative` on every cooperative locus
declaration, with a single shared main thread) is exactly the
behavior a program without any `placement { }` block
receives. Existing programs that don't declare placement see
no observable change.

### Pool inference

The set of cooperative pools is the union of `X` values
appearing in `cooperative(pool = X)` references across all
placement entries, plus the implicit `main` pool. The runtime
spawns one OS worker thread per inferred pool name beyond
`main`. No `threads { }` declaration block at v1 — pools are
named purely by use site, and the runtime materializes them
on demand at startup.

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

### Per-epoch field reset (F.34, v1.x-WINDOWED)

A closure paired with `epoch duration(N)` may declare
`resets_per_epoch(field1, field2, ...);`. The named locus fields
are zeroed by the runtime **after** the assertion fires at each
duration boundary. Ordering matters: the assertion sees the
window's accumulated value; the reset prepares the next window.

```hale
closure low_corrupt_rate {
    self.corrupt_per_min ~~ 0 within 10;
    epoch duration(1m);
    resets_per_epoch(corrupt_per_min);
}
```

Restrictions enforced at typecheck:

- The closure MUST declare `epoch duration(N)`. The clause is
  rejected on `tick` / `birth` / `dissolve` / `inline` /
  `explicit` — other epochs either don't recur or recur too
  fast for a rate-budget framing.
- Each named field MUST be declared on the enclosing locus and
  MUST have numeric type (`Int`, `Uint`, `Float`, `Decimal`).
  Booleans, strings, and structs are rejected — zero is not a
  meaningful reset value for them.

User code increments / decrements the field as the window
accumulates. The closure assertion is the structural contract
(rate bounded by a per-window budget); `resets_per_epoch`
keeps the substrate honest about which window the counter
belongs to without forcing the user to maintain a `last_reset_at`
field or a parallel pre-fire hook.

## Inline closure violation

(F.27, v1.x-VIOLATE.) Inline closures provide a pull-only
structural-failure channel for locus method bodies that catch a
value error and want to escalate it. The declaration carries no
assertion; the optional `captures:` clause names locus fields
whose values are snapshotted into the ClosureViolation payload
at fire time.

```hale
closure fatal_io { captures: last_error; epoch inline; }
```

`violate NAME;` (optionally `violate NAME with EXPR;`) fires the
closure synchronously at the call site:

1. Runtime synthesizes a `ClosureViolation` value carrying:
   - `locus`, `closure` — string names of the failing locus and
     the inline closure (always present).
   - The captured fields named in the closure's `captures:`
     clause are NOT materialized on the `ClosureViolation`
     struct, which has a fixed shape. The access pattern for
     captured state is to read the frozen child through the
     child handle in `on_failure(c, err)` — see "Reading the
     audit state" below.
   - If `with EXPR` was given, EXPR is evaluated for side
     effects (and to detect typecheck errors on the payload
     type) but no `payload` field is materialized on the
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

```hale
on_failure(c: Child, err: ClosureViolation) {
    log::error(err.closure, " ", c.last_error, " fd=", c.conn_fd);
}
```

`violate` is divergent — the method body's remaining statements
do not execute, so the child's locus state is frozen at the
violate moment. `c.last_error` reads exactly the value the
violate site observed.

The `ClosureViolation` value carries only `err.locus` and
`err.closure`; it does not materialize the captured fields.
Source that reads `err.last_error` will typecheck
(`ClosureViolation` admits unknown fields permissively at
field-access time) but will fail to link / run — read captured
state through the child handle (`c.last_error`) instead.

The `violate` statement is divergent: the typechecker treats it
as `Never`, the same as `fail` in fallible fn bodies and
`bubble` in `on_failure`. No statement after `violate` in the
same block is reachable; the typechecker does not require a
trailing `return` on a `violate` branch.

### `birth_check` synthesis hook

(F.27 v2, 2026-05-20.) A declarative form for construction-time
invariants:

```hale
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

```hale
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

Hale carries two **orthogonal** failure channels:

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
`notes/agent-onboarding/hale-design-philosophy.md` § 2.

### Where each channel lives (declaration sites)

The two channels are realized through different declaration
sites. The mapping is canonical, not advisory:

- **`fallible(E)` may be declared on:**
  - **Free fns** — pure application-layer computations whose
    failure shape matters call-by-call.
  - **Stdlib-synthesized methods on `@form(...)` containers**
    (`@form(vec).get` / `.pop`, `@form(hashmap).get` /
    `.remove` / `.key_at` / `.entry_at`,
    `@form(ring_buffer).pop`). Application-layer storage
    substrate: the container's role is application-layer
    data, not locus-structural participation in the
    substrate's lifecycle.
  - **User-declared `fn` member fns on a locus**
    (open-question #24, shipped 2026-05-25 in two phases:
    MVP at `d565d6f` with value-only payloads, v0.2 at
    `98910b9` extending to heap-bearing payloads via the
    TLS caller-arena snapshot non-fallible heap-returning
    methods already use). The narrowed rule recognises that
    a `fn` member fn is *not* substrate-orchestrated — its
    callers hold a frame, can address the error channel
    inline, and the value-error path doesn't conflict with
    the closure-violation channel.
- **`fallible(E)` may NOT be declared on:**
  - **Lifecycle methods** (`birth` / `run` / `accept` /
    `drain` / `dissolve` / `on_failure`). The substrate
    orchestrates these — bus dispatch invokes the handler,
    parent invokes `accept`, runtime invokes `run` — and
    there's no caller frame in user code to address a value
    error. Physically rejected at the AST level
    (`LifecycleDecl` carries no `fallible` field).
  - **Mode methods** (`bulk` / `harmonic` / `resolution`).
    Same shape: AST doesn't carry the field.
  - **Closure assertions.** Substrate evaluates the
    assertion at the epoch boundary; there's no caller in
    the expression's frame to address a value error.
    Closures route failure via their own structural channel
    (assertion firing → `on_failure`), not a value channel.
  - **Bus-subscribed handlers.** Verified at the
    `subscribe ... as handler` site rather than at the fn
    decl: a fn that's `fallible(E)` by declaration may not
    also be referenced by a `subscribe` entry. Bus dispatch
    has no caller frame; subscribing a fallible fn would
    have nowhere to send `out_err`. The typecheck
    diagnostic fires at the subscribe site, naming the
    handler fn.

The rule is **two-channel separation at substrate-facing
surfaces**, not "no fallible on any locus method." The
load-bearing constraint is *who's the caller* — when the
substrate orchestrates a method (lifecycle, mode, closure
assertion, bus handler) there's no caller frame to address
the error channel, so `fallible(E)` would describe a
contract that cannot be satisfied. User-declared `fn`
members called from inside a method body or from another
locus's method body have an addressable caller; they carry
`fallible(E)` like free fns do, with the same `or`
disposition surface.

Example shape post-narrowing:

```hale
type ParseError { msg: String; }

locus Reader {
    // Allowed: user-declared `fn` member with fallible(E).
    // The body can `fail ParseError { msg: ... }` or call
    // other fallible functions and propagate via `or raise`.
    fn parse_message(b: Bytes) -> Message fallible(ParseError) {
        // ...
        if bad { fail ParseError { msg: "bad header" }; }
        return Message { ... };
    }

    // Allowed: lifecycle method calling a fallible member fn
    // and addressing the error inline.
    run() {
        let m = self.parse_message(b) or default_message();
        // ...
    }
}
```

The earlier v0 rule (blanket "no fallible on locus methods")
was narrowed because the friction signal across multiple
apps and libraries showed devs extracting free fns just to
get a value-error channel back — losing `self` ergonomics
and splitting closely-related code across two top-level
decls. See `notes/open-questions.md` § #24 for the
narrowing's full reasoning and the rejected alternatives.

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
   dprintf to stderr (`"Hale panic: unhandled <T>
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
   `spec/projects.md` "Resolution order"), parses every `.hl`
   file in the resolved target, applies the auto-mangler with
   a stable path-derived `<lib_id>` + each file's stem, and
   merges the mangled items into the importing program's item
   list. A per-build path-rename table is built mapping
   `["<alias>", "<Name>"]` to `__lib_<lib_id>_<stem>_<Name>` —
   the `<alias>` is the importer's local namespace choice; the
   `<lib_id>` is the lib's canonical path identity, so two
   consumers importing the same lib under different aliases see
   identical mangled symbols.

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
rewriting, so `greet.hl`'s reference to a `Formatted` type
declared in `format.hl` rewrites to the same
`__lib_<lib_id>_format_Formatted` symbol that `format.hl`'s
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

**`hale run` interaction.** `hale run` compiles through the same
codegen path as `hale build`, so a single file's imports resolve
identically. The ad-hoc directory form (`hale run ./dir`) bundles
files without threading the per-build path-rename table, so
programs with cross-seed imports should be built and executed via
`hale build`.

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
