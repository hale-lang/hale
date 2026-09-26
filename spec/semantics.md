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
conversion. Phase 2c. See F.23 in
`spec/design-rationale.md` and the Phase 2c entry in
`spec/stdlib.md`.

## Expressions — `if` and block tails

A `{ ... }` block whose last item is an expression *without*
a trailing `;` carries that expression as its **value**. In
expression position (let-RHS, fn-call argument, if-arm body)
the value is consumed; in statement position (loop body,
`Stmt::If` / `Stmt::Match` block) the trailing expression is
evaluated for side effects and the value is discarded —
semantically equivalent to having added the `;`.

A **fn-shaped body** (free fn, locus method, mode) with a
declared return type treats its trailing expression as an
implicit `return`: `fn double(n: Int) -> Int { let d = n * 2;
d }` returns `d` (2026-08-11 — previously the typechecker
accepted this form and codegen rejected it as a fall-through).
A body with *no* declared return type keeps statement-position
semantics: the tail is evaluated for side effects.

`if cond { ... } else { ... }` is dual-position:

- **As statement** (`if c { foo(); }` — no `else`, or any arm
  whose block has no trailing expression): no value; trailing
  expressions in either arm are evaluated for side effects.
- **As expression** (e.g., `let x = if cond { i } else { j };`):
  the then- and else-arms' trailing expressions are
  phi-merged at the join basic block. The else branch is
  **required**; arm trailing-expression types must match;
  arms may carry their own let-bindings before the tail (the
  bindings are scoped to the arm).

The two positions are distinguished by **shape, not syntactic
context** (WS3.2). A trailing `if` — the last item
of a block, with no `;` — becomes that block's tail expression
when it is **value-producing**: every arm (then, else, and each
`else if`) must end in a trailing expression. This is what lets
an `if` nest as a block value:

```hale
let x = if a { if b { p } else { q } } else { r };
//          \__ then-arm block whose tail is the inner if __/
```

The inner `if` is the then-arm block's tail, so the outer
`if`'s then-value is the inner `if`'s value. A trailing `if`
that is **not** value-producing (no `else`, or an arm with no
tail) stays a statement — it has no value to carry, and forcing
it into the value path would leave a tail-less arm with nothing
to yield.

`else if` chains carry through the value path —
`ElseBranch::ElseIf` recurses and the innermost arm's tail
feeds the phi at the outermost merge.

Phase 2b introduced `if`-as-expression; WS3.2
(2026-06-11) made a value-producing trailing `if` compose as a
block tail. See F.24 in `spec/decisions.md` and the
Phase 2b entry in `spec/stdlib.md`.

`match` is dual-position too (Gap C, 2026-07-17):

- **As statement**: arm bodies are evaluated for side effects;
  heterogeneous arm-body types are legal (values discarded).
- **As expression** (`let x = match n { 0 -> 10, _ -> 20, };`):
  every arm body's value is phi-merged at the join block, so
  all value-producing arms must agree on one type (checked at
  typecheck with the match's span; `match` expression works with
  every pattern form the statement supports — literal / binding /
  wildcard / tuple / enum-constructor patterns, guards, and
  block arm bodies with trailing expressions). F.18
  exhaustiveness applies in both positions. The only reachable
  no-arm-matched case in expression position is a match whose
  arms are all guarded and every guard is false at runtime; it
  yields the zero value of the result type (`0` / `0.0` /
  `false` / empty for pointer-shaped types), mirroring the
  statement form's silent-no-op fallthrough.

**Scrutinee-less form.** `match { cond -> body, else -> body }`
omits the scrutinee and tests guards directly:

```hale
let tier = match {
    n < 10   -> "small",
    n < 100  -> "medium",
    else     -> "large",
};
```

Arms are tried in written order and the first whose condition is
true wins. `else` is the catch-all and carries no condition.

This is **sugar**, desugared at parse into the guarded form it
replaces — `match { c -> b, else -> d }` is
`match true { _ if c -> b, _ -> d }`. Everything downstream sees an
ordinary match, so both positions, block arm bodies, the trailing
comma rule, and the all-guards-false fallthrough above behave
identically.

The form exists because first-match-wins over conditions is common
where dispatch is a ladder of tests rather than a shape match, and
the scrutinee in that case is a value the author invents only to
ignore.

`match {` is not ambiguous with matching on a block expression:
that shape is written `match (…) { … }`.

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
- `Stream.recv_bytes(max: Int) -> Bytes fallible(IoError)` —
  binary-safe TCP receive (Phase 2g; fallible since #209 —
  EOF/timeout return empty, only genuine errors fail).
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
- `Stream.send_bytes(b)` — length-preserving TCP send (m89;
  `fallible(IoError)` with Unit success since #209).
- `std::str::from_bytes(b) -> String` — copies into a
  NUL-terminated buffer; embedded NULs persist but downstream
  strlen-based String operations truncate at the first
  (Phase 2g).

All returned `Bytes` values from the path-call surface are
anchored in the lazy global payload arena, so callers can
stash the pointer past the call site without m49 deep-copy
plumbing.

## Rendering values as text

`println`, `print`, `to_string`, `String + x`, and f-string
interpolation all render through **one** rule. The set of types
that rule accepts is called the *printable* set.

A type is printable when it is:

- a scalar primitive — `Int`, `Float`, `Bool`, `Decimal`,
  `Duration`, `Time` — or `String` / `StringView`;
- an enum (rendered `Enum::Variant`, or `Enum::Variant(a, b)`
  when the variant carries a payload). `Enum` is the
  **declaration's own name**, whether the value's type was declared
  in this seed or reached through an `import`: an imported
  `Color::Red` renders `Color::Red`, not the import alias and never
  the mangled symbol the declaration was renamed to (2026-09-20,
  GH #885; before it, every print path rendered
  `__lib_<alias>_<stem>_Color::Red` — a name that appears nowhere
  in the author's program). The alias is deliberately absent: it is
  per-importing-file (`spec/projects.md` § *Scoped imports*), and
  the rendering is a property of the value, not of the file that
  printed it;
- a `type` record whose fields are **all** printable, rendered
  `Name { field: v, other: w }` in declaration order;
- a tuple of printable elements, rendered `(a, b)`;
- a `[T; N]` or `bounded[T; N]` whose element type is `Int`,
  `Float`, `Bool`, `Decimal` or `Duration`, rendered `[a, b, c]`.

Three exclusions are load-bearing rather than incidental:

- **A locus is never printable.** A locus is flow, not shape; and
  since a `@sealed` locus confines its `params` (see GH #436),
  a printable locus would be a way to read the confined state
  back out. There is no printable path to a sealed value.
- **`Bytes` is not printable.** The useful rendering — hex, a
  length, a UTF-8 attempt — is a choice the author must make.
- **An unsized `[T]` is not printable.** There is no length to
  walk at the render site.

Rendering is defined recursively, with two rules about the nesting:

- A `String` **inside** a composite is quoted with `"`; a `String`
  rendered on its own is not. `to_string(s)` is identity, and it
  stays identity — but `User { name: a, b }` is ambiguous about
  its arity in a way `User { name: "a, b" }` is not.
- A composite longer than **32** elements is truncated with a
  trailing `…` inside the brackets. For `bounded`, the marker
  depends on the live count, not the declared capacity.

Rendering has no user-visible ordering or allocation guarantees
beyond producing the text above; results are owned by the caller's
arena like any other constructed String.

### Format specs

`f"{expr:spec}"` renders `expr` under `spec` instead of under the
default rule. The grammar is in `spec/tokens.md`. Ordering is
fixed: the value is **rendered first** (honouring `precision` or
a hexadecimal `kind`, both of which need the value) and the text is
**padded second** (`width`, `fill`, `align`).

An absent alignment resolves from the value's type — numeric values
pad on the left, everything else pads on the right — so a column of
figures lines up on the ones place and a column of names lines up
on the first letter without either being asked for.

Grammatical errors in a spec are reported by the parser;
spec/value mismatches (hexadecimal of a `String`, a precision on
an `Int`) are reported by the typechecker. Neither reaches codegen.

## Locus instantiation

`LocusName { params }`:

1. Compute params (overrides applied to declared defaults).
   `self` resolves **lexically**: inside a *default* expression —
   including the field inits of a nested locus literal written in
   that default — `self.X` reads the locus being instantiated
   (earlier-declared siblings only; a default that reads a
   later-declared sibling is a compile error, since defaults run
   in declaration order). Inside an *override* expression, `self`
   belongs to the code that wrote the literal — the enclosing
   method's locus, or the enclosing params block when the literal
   itself sits in a default (F.4 call-site rule). This holds
   regardless of where the instantiation executes (fn main, a
   params-init, or another locus's method body — 2026-07-14 fix).
2. The nearest enclosing ancestor that declares `accept(c: I)`
   for the child's interface is the **owner** (innermost-wins —
   interest-based ownership / accept bubbling; see below and
   `runtime.md`). Its `accept(c)` runs first; if it rejects,
   instantiation fails (no region allocated). With no accepting
   ancestor the child is a transient throwaway (no owner).
3. Region allocated as a sub-region of the **owner's** region
   (the accepting ancestor — not necessarily the direct parent);
   size determined by projection class.
4. `birth(args)` runs synchronously.
5. Bus subscriptions wire up.
6. Modes are reachable for invocation.
7. If `run` declared, scheduled to run on the locus's
   scheduler. For a locus placed `pinned` or on a
   cooperative pool other than `main`, "scheduled" means
   handed to that thread or worker and the instantiation
   returns immediately. For a locus that runs inline on
   the main thread — the default placement, and an
   explicit `cooperative(pool = main)` — `run()` executes
   **synchronously here**, and step 8 does not happen
   until it returns. See § Birth order is load-bearing.
8. Expression returns the locus handle.

**Accept bubbling.** The owner in step 2 need not be the direct
parent. An `I{}` instantiated anywhere in a subtree bubbles to
the nearest enclosing ancestor that declares `accept(I)`
(innermost-wins); resolution is entirely static (the closed-world
instantiation graph fixes every owner edge at compile time). The
owner may live in a different tower or on a different pool: a
cross-pool owner is served by an async handoff over the bus, so a
cross-pool `I{}` is **fire-and-forget** — it may only appear as a
bare statement, and using the instance as a value is rejected at
compile time. See `runtime.md` "Interest-based ownership (accept
bubbling)."

### Birth order is load-bearing

A parent births its `params` fields **in declaration order**, one
at a time, and step 7 above runs an inline locus's `run()`
*synchronously*. Composing those two rules: **a params field whose
`run()` does not return prevents every field declared after it
from being born at all.**

Not merely from running — from being *born*. The later locus's
`birth()` never executes, so the subscriptions it registers, the
sockets it binds, and the children it accepts never come into
existence. Nothing fails and nothing is logged; the process
completes what looks like a normal boot and then sits idle. This
is the single most expensive way to misread a Hale program, and
the symptom points nowhere near the cause — it typically presents
as "my handler never fires", which sends you looking at the bus.

```hale
main locus App {
    params {
        server: Server = Server { };   // run() { while true { ... } }
        metrics: Metrics = Metrics { };  // NEVER BORN
    }
}
```

Only the **blocker's** placement matters. Moving the *later* field
off the main thread does not rescue it — the instantiation itself
runs inline on main, so even a `pinned` sibling declared after a
blocking field waits. The four placements behave as:

| Placement of the blocking field | Blocks later births? |
|---|---|
| default (no placement entry) | **yes** |
| `cooperative(pool = main)` | **yes** |
| `cooperative(pool = io)` (any non-`main` pool) | no — `run()` is posted to a worker |
| `pinned` | no — `run()` gets its own thread |

Two remedies, both sound: declare the non-returning field **last**,
or place it off the main thread. The compiler warns whenever it can
*prove* the shape — a terminal `while` loop with no
`break`/`return`/`terminate`, in a field that runs inline on main,
with at least one field declared after it. The proof is
deliberately conservative, so a loop the compiler cannot prove
non-terminating is not reported: absence of the warning is not a
guarantee of correct ordering.

This rule is why a child's `run()` is the wrong place to put work
that must happen after its siblings exist. The **main locus's**
`run()` is the right place — it begins only after params-init
completes, so by then every child has been born.

### Dissolve timing rules

Five shapes, three timings (m82 — "locus all the way down";
expression position generalized from receivers to every
position by GH #711 / #812).

**Who reclaims a locus is decided before any code is emitted.**
One pass over the program gives every locus-producing expression
— a literal, a proven-fresh factory call, and each branch, arm
and element of an `or`, a carrier or a composite separately — the
owner the rules below name, and lowering reads that decision
rather than making one as it goes. An instantiation the pass
left undecided is refused, so the rules here hold by
construction rather than by the order things happen to be
lowered in (`spec/decisions.md` F.39).

- **Statement-position literal** (`LocusName { ... };`, no
  binding and nothing done with the value): birth → run →
  drain → dissolve all fire at the statement boundary.
  Fire-and-forget. The handle is discarded.
- **Let-bound literal** (`let h = LocusName { ... };`): birth
  + run + drain fire at the construction site. Dissolve is
  **deferred to the enclosing fn's scope-exit flush**. The
  user-visible binding `h` is the handle; the locus instance
  lives until `h` goes out of scope. This is what makes
  `let s = Stream { conn_fd: fd }; s.send(msg) or raise;` work — `s`
  stays valid for the method call because dissolve hasn't
  fired yet.
- **Expression-position literal** (a literal used as a value:
  a method receiver `LocusName { ... }.method()`, a call
  argument `serve(LocusName { ... })`, a field read
  `LocusName { ... }.field`, an operand): the expression that
  consumes it is the handle, so the literal has **the same
  timing as a let-bound one** — dissolve is deferred to the
  enclosing fn's scope-exit flush. Naming the literal with
  `let` first and using it inline are the same program, in
  every position. In particular the literal is alive for the
  entire call it is the receiver or argument of, including any
  allocation churn or drain point inside it, and for as long as
  a callee that retains it holds on to it. This holds in
  statement position too (`LocusName { ... }.method();`): only
  a literal whose value is *discarded* is torn down at the
  literal's own boundary.
- **Factory result** (`let h = make(...);`, or the call used
  directly as a value — `serve(make(...))`): the locus is built
  inside the callee and handed back, so the expression that
  consumes the handle is its owner, with **the same timing as a
  let-bound literal**. A binding owns what it names; a call
  result nothing names is owned by the enclosing fn's scope. What
  a binding names is the expression written **at that position**,
  not a factory call nested inside it: in `let x = combine(a,
  make());` — and in `return combine(a, make());`, where the
  caller is the owner — the binding (or the caller) owns
  `combine`'s result, and `make`'s is a result nothing names,
  owned by the enclosing fn's scope and reclaimed at its exit
  (GH #837). An `if`, a `match` or a block written **as a value**
  is not the value either: it hands back one arm's, and exactly
  one arm runs — so the rule applies **per path**, to each arm's
  tail expression, which is what is written at that position on
  the path that produces it. `return if c { make(1) } else {
  make(2) };` hands the caller whichever arm ran, and the frame
  that built it reclaims neither; `let x = if c { make(1) } else
  { make(2) };` reclaims whichever arm ran exactly once, at the
  enclosing fn's scope exit. A factory call anywhere else inside
  such a carrier — in the condition, in the scrutinee, in a
  statement before a block's tail — is a result nothing names,
  reclaimed at that scope's exit like any other. An **ascribed
  array or tuple** literal is likewise what the site names, never
  one of its elements: `let xs: [Thing; 2] = [make(1),
  make(2)];` names the array, and each element's result is one
  nothing names, reclaimed at the enclosing fn's scope exit
  (GH #883). This
  is the same rule in the **fallible** spelling, where the call
  is reached through `or` — `let c = std::process::spawn(argv)
  or raise;` is reclaimed exactly as `let h = make(argv);` is,
  and so is `or <substitute>`, where whichever branch produced
  the value is the branch that gives it an owner (GH #793). Two
  positions are excluded, because there the handle is *handed
  on* rather than consumed: a result written directly as a locus-
  or interface-typed **field of a locus literal**, which the
  literal owns (F.17), and a binding the enclosing fn **returns**,
  which the caller owns — dissolving either would free a value
  another owner still holds. The first of those two is an
  ownership *transfer*, not an exemption: `Router { quick:
  make(5) }` is the same program as `Router { quick: Quick { } }`
  — the field's value is reclaimed by the owner's teardown
  cascade, at the owner's timing, whether the field is written at
  the call site or as the param's **default**, and whether the
  call is bare or reached through a *diverging* `or` (`or raise`,
  `or fail`), where the factory's result is the only value the
  field can hold (GH #836). `or <substitute>` transfers on **both
  branches**: the field holds whichever branch ran and owns that
  value, so `Router { quick: make_f(5) or make2() }` reclaims
  exactly the locus that was built, once, from the owner's cascade
  — the substitute is not *also* a temporary of the frame that
  built the owner, which would flush it at that frame's exit with
  the field still pointing at it (GH #853). The claim needs every
  branch to transfer: a proven-fresh factory call of the field's
  own locus, a locus literal, or a nested `or` of those. A call in
  that position that transfers nothing is excluded exactly as an
  external handle is — one that returns a locus it did *not* build
  (one of its arguments, a handle it was given), and an `or` whose
  ok value or substitute is such a call — and the value is left to
  its real owner.
- **Long-lived** (locus has `bus subscribe`): always deferred,
  irrespective of binding shape — the locus must stay alive to
  receive published events between birth and the enclosing
  scope's exit.

Multiple deferred dissolves in the same scope fire in
**reverse instantiation order** at scope exit (LIFO), matching
the F.4 depth-first cascade. The reason: a later-created
locus may depend on an earlier-created one, so the later one
must dissolve first.

Whichever of the four shapes applies, the timing is the
timing of the **whole owned tree**, not of one level of it.
A locus held as another locus's param field never has a
teardown of its own — its instantiation is parent-owned, and
the owner's teardown cascades into it (F.29). That cascade
runs to the leaves: a grandchild's `drain()`, its `dissolve()`
body, its capacity slots and **its arena** are the owner's
responsibility just as a child's are, at exactly the moment
the owner's timing fires. Every level's arena is destroyed,
none outlives the owner, and the count of live arenas a
finished program leaves behind is zero. "Constructed by the
owner" covers both spellings of construction: a nested literal
(`Mid { leaf: Leaf { } }`) and a factory call whose result the
field takes (`Mid { leaf: make_leaf() }`, in the
diverging-`or` spelling too, and as a param default). A field
the owner did NOT construct (`Mid { leaf: shared }`, an
external handle passed in; or a call that hands back a locus
somebody else built) is excluded at whatever depth it appears,
and is torn down once by its real owner, at its owner's timing.

**What the field is DECLARED as does not change any of this.**
A param typed by a *contract* — an `interface` the child
satisfies (`params { j: Counter = Churner { } }`) or a
`perspective(P)` the child serves (`params { router:
perspective(Router) = RouterV1 { } }`) — holds a parent-owned
child exactly as a locus-typed param does, and the cascade
reaches it and everything under it. Which locus satisfies the
contract is a per-instantiation choice: a designation written
at the literal (`Gateway { router: RouterV2 { } }`) overrides
one written as the param's default, and the child torn down is
the one that was actually constructed. Both spellings of
construction reach a param typed by a contract, whichever
contract it is: `Queries { j: make_churner() }` is the same
transfer as `Queries { j: Churner { } }`, and so is `Gateway {
router: make_router_v1() }` into a `perspective(P)` param. The
child reclaimed is the impl that was actually built rather than
the one the param's default names (GH #895), and the `or`
spellings transfer here exactly as they do into a locus-typed
param — the diverging ones, where the factory's result is the
only value the field can hold, and `or <substitute>`, where the
field owns whichever branch ran. A `reperspective` swap
does not change it either — the swap replaces code and keeps
state, so the holder still owns the impl its designation built.

A locus literal written inside the initializer of a param that
**cannot hold a locus** is not an ownership transfer, because
there is no field for the owner to cascade from. In `Lonely { n:
Queries { }.total() }` the `Queries` literal is an ordinary
expression-position literal, owned by the enclosing fn's scope
and reclaimed by its scope-exit flush, exactly as it would be
written on a line of its own.

A deferred dissolve is scoped to the enclosing **fn**, not to
the enclosing block — a `let` is readable for the rest of the
fn, including after the loop that bound it — but a locus
created in a **loop** is reclaimed when its slot is reused, and
only the last one at scope exit. Control arriving at the same
instantiation a second time is the end of the previous
instance's life: it is torn down there, with the same
`drain → dissolve → arena reclaim` the scope-exit flush runs,
before the new one takes its place. This holds for every
spelling — `let h = L { };`, a fresh literal used as a value, a
`let` bound to a locus-returning factory, and a `let` whose RHS
is an `if` / `match` / block or an ascribed array or tuple, where
the reclaim is per ARM and per ELEMENT because that is where the
value is built — so a loop's residency is one instance per
site, not one per iteration. A
locus that ESCAPES the iteration is unaffected, because it
never had a slot here to begin with: a literal written directly
as another locus's param field is parent-owned, and so are an
`accept`'d child and a locus the fn returns. A handle the loop
passes *in* to an owner (`Keeper { held: h }`) keeps its own
slot and is reclaimed here — the owner's cascade steps over a
field it did not construct, as it always did (F.29).

Each instantiation reclaims its own previous instance, where
it stands, so within one iteration the reclaims run in
instantiation order — the reverse of the LIFO order the
scope-exit flush uses. Two loci bound in the same iteration are
independent; a `dissolve()` body that reads a sibling handle it
does not own sees the sibling's next instance.

**Every exit path flushes independently, and a `return` takes
nothing away from the others.** A fn — `fn main` included — may
leave through an early `return`, through a later `return`, or by
falling off the end. Each of those paths dissolves everything the
fn owns *at that point*, in reverse instantiation order, and the
paths do not interact: a `return` in the middle of a body does
not shorten the set the fall-through exit dissolves. So in

```hale
fn main() {
    let a = Noisy { tag: "a" };
    if bad_usage { return 2; }   // dissolves `a`, then exits 2
    let b = Noisy { tag: "b" };
}                                // dissolves `b`, then `a`
```

the guarded exit dissolves `a`, and the ordinary exit — the one
taken when the guard does *not* fire — dissolves `b` and then
`a`. A locus bound only inside the branch that returns is listed
on the other paths too but was never instantiated there, so those
paths skip it: it dissolves exactly once, on the one path that
built it. `return f()` evaluates `f` before any teardown begins,
and a locus `f` itself bound is dissolved by `f`'s own exit
(GH #789).

### `terminate`

`terminate;` ends the current locus's lifecycle
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

**Validity (typecheck).** `terminate;` in a free
function is a typecheck error — there is no enclosing locus whose
lifecycle to end. It is accepted in any locus method body
(lifecycle method or member `fn`).

**From a bus handler.** `terminate;` is no longer
limited to `run()`. A subscriber can end its own life from inside
a bus handler (e.g. `on_close` receives a shutdown message and
calls `terminate;`). The reclaim runs when the handler returns —
the dispatch path checks the `__drain_requested` latch after each
handler and runs the spine on the handler's own worker. This is
the resident-subscriber analogue of the connection child that
`terminate`s from its `run()` recv loop. A handler that `violate`s
sets the same latch and is reclaimed the same way — unless its owner
still holds it (below).

A child reclaimed this way may still be named by its owner — a
param field (`c: Child = Child { }`) keeps its handle. Both halves of
the owner's teardown cascade — the drain walk and the dissolve walk —
test the child's arena-destroy latch (`__arena`, slot 0 of a struct
that lives in the owner's arena) **before** the per-child body, and
step over a child already reclaimed: its `drain()` and `dissolve()`
do not run twice, and its own children, which lived in its freed
arena, are not visited again (GH #1036).

**A failed child its owner holds is kept (GH #1069).** `terminate`
is a child ending its own life; a failure is not. A child whose
`run()` or handler failed — a `violate`, a failed `birth_check` —
and that its supervisor did not restart (it absorbed the failure,
quarantined the child, or spent `restart(c) for N`) **stops**: its
`run()` has returned, and on the wrapper paths that would have
reclaimed it its bus subscriptions are dropped. But while anything
still holds it — a param field, a binding, a value the enclosing
expression uses — its memory stays until that owner's teardown,
where its `drain()` and `dissolve()` run once. The owner's handle
therefore always names a live child: `self.c.why` after an absorbed
failure reads what the child last stored. Only a child nothing else
reclaims tears itself down when it fails: an `accept`'d child (its
owner's tracker expects it gone) and a bare statement literal (its
statement's own teardown then steps over it, on the same latch).

### `release(c)` and flow children

`release(c: Child) { ... }` is the death-side
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

Several parent types may accept the same child type; each child
carries its accept'ing owner AND that owner type's `release` fn
(stored at accept dispatch), so the body that fires is the actual
owner's own, never another parent type's (2026-09-05, GH #526 F.6:
the reclaim spine used to pick the first declared `release(c: T)`
program-wide). Whether `T` is a flow remains a type-wide fact: if
any parent declares `release(c: T)`, every `T` reclaims on
run-completion, and an owner that declares no `release` simply has
no bookend called.

Whether `T` is a flow is decided over the whole program, imported
seeds included; an explanation names every `release(c: T)` clause
(GH #736).

**A resident and what a handler hands it.** A payload delivered to a
handler, and any container the handler builds while it runs, belong
to that dispatch and are reclaimed when the handler returns. A child
born from the handler that must keep them copies them in its own
`birth()` — cloning Strings, rebuilding rows — into storage it owns;
holding the handler's pointers past the dispatch is a use after
free, and the checker does not yet diagnose it (GH #712).

`release` has the same shape as `accept` — one typed child
param — and the same fn signature `(parent_self, child_self)`.

**Validity (typecheck).** A `release(c: T)` with no
matching `accept(c: T)` on the same locus is a typecheck error: a
locus that never accepts a `T` child can never release one, so
the declaration is dead (almost always a wrong child type or a
forgotten `accept`).

**Parent-dissolve reclaim.** When a parent that
`accept`s children dissolves, it reclaims each accept'd child it
still tracks — running the child's full teardown (drain →
dissolve → arena reclaim) before the parent's own arena (which
backs the children's subregions) is freed. A parent that accepts
tracks every child it accepted, whether or not any of its methods
iterates `self.children`, because this cascade is what the
tracker exists for; and the cascade runs on every path that
dissolves the parent — graceful shutdown, a reclaimed flow, and a
mid-life reassignment of the field that holds it — so a child's
bus subscriptions are deregistered before the memory they point
at is freed, never left to receive into whatever reuses it. The
order within the parent's teardown is children first, then the
parent's own capacity slots, then its arena: a child may hold a
slot borrowed from the parent (`as_parent_for`), and a borrowed
slot is never destroyed by the child. This is what makes the
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

**Mode keywords in contract names:** mode names
(`bulk` / `harmonic` / `resolution`) are admitted in expose-entry
position — `expose bulk: Float;` — making the exposed-mode pull
rule below expressible (it was a parse error before). The exposed
type is checked against the mode's declared return; expose entries
in general must bind a real params field, mode, or fn member at a
matching type (M3 stage 4).

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

When a locus is accepted by an owner (the accepting ancestor —
see "Accept bubbling," not necessarily the direct parent) whose
projection class is **Chunked** or **Recognition**, the child's
slot 0 (arena) is allocated either as a sub-region of the owner's
arena (Chunked, via `lotus_arena_create_subregion`) or out of the
owner's recpool (Recognition with the matching sub-mode, via
`lotus_recpool_fixed_acquire` / `lotus_recpool_slab_acquire`).
The child is freed wholesale when the owner dissolves.
**Rich**-class owners do not sub-region-allocate; accepted
children get their own top-level arenas. When bubbling crosses a
pool, the child is born in — and reclaimed by — the owner's
thread via an async bus handoff (`runtime.md`). See `memory.md`
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

A locus declares **at most one** `accept` (single-accept-type per
parent, `types.md` F.11). A second `accept` clause is a typecheck
error naming both clauses; the first clause remains the locus's
accept type. A parent that must own two child types gives one of
them a different owner. (2026-09-04, GH #525: a second clause used
to overwrite the first silently.)

### `run()`

Runs continuously until drain is requested or run returns
naturally. Cooperative — yields at every bus dispatch, every
`time::sleep`, every explicit yield point. The scheduler may
run other loci while this run is yielded.

If run() returns naturally, the locus exits run-state and
proceeds to drain.

A locus that declares no `run()` has an empty one: the two
spellings are the same program (GH #735, 2026-09-22). In particular
a flow child — a type some parent `release`s — is reclaimed when its
empty run completes, right after its birth, whether the empty `run()
{ }` is written or omitted; it does not live until its owner's exit
because the hook was left out. A resident is unaffected: an empty
run means "ready" either way.

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
- Return normally (absorb): treat as collapsed — the child
  stops, and a child the parent holds stays readable until the
  parent's teardown (§ `terminate`, "A failed child its owner holds
  is kept").
- Call `restart(c)`: run the child again — `birth()`, then
  `run()` — on the same instance (see § Recovery primitives).
- Call `restart_in_place(c)`: re-init in place (preserve
  arena).
- Call `quarantine(c)`: keep child in a halted state with
  arena preserved; future inspection possible.
- Call `bubble(err)`: pass the failure to self's parent.
- Call `dissolve(c)` explicitly: free child's region.

Default on_failure: `bubble(err)`. The runtime root's default
is process exit with stack trace.

**Which locus is the parent.** A child locus held in a param
field is supervised by the locus that holds it, whichever way
the literal is written: as the field's default in the parent's
`params { }`, or at the call site in the parent's literal
(`App { b: Boom { why: w } }`), in any branch of an `if` or `match`
that the field's value names, and one level at a time
through nesting (`App { w: Wrap { b: Boom { } } }` — `Boom`'s
parent is `Wrap`). Where the holder itself is built does not
matter: `let s = Sup { };` inside another locus's method gives
`Sup`'s default child to `Sup`, not to the method's locus (GH
#1074). An override expression is otherwise lowered
in the caller's context (`self.x` in it reads the caller), but
the literal it builds belongs to the parent. A locus built
elsewhere and passed in by name — a `let` binding, a factory's
return — keeps the route of the place it was built: the locus
whose method body built it, or none from a free fn or `fn main`.

**When it runs: never before the parent's params are settled.**
A child can fail while its parent is still setting params — a
cooperative child's `run()` executes inside that loop (§ Birth
order is load-bearing) and a pinned child's thread starts in it.
The failure is **held** and delivered once the parent's last
param is stored, before the parent's `birth()`; several are
delivered in the order they arrived, on the thread settling the
parent. So a handler always reads params that hold their values,
and nothing it writes is overwritten by a default stored after
it. The failing child outlives its held failure: a child whose
`run()` has already ended — a cooperative child's returns right
after its `violate` — is reclaimed only after its handler has
run, so the handler's `c` is always a live child.

The rule holds the *handler* back, not the children. The other
reading — every param settled before any child runs — would undo
§ Birth order is load-bearing: a later param's default may read
what an earlier child computed (`total: Int = self.loader.count`
after `loader`'s inline `run()` filled it), and that only works
because the children are born, and inline ones run, one at a time
in declaration order.

**A restart requested from a held handler takes effect when the
handler returns**, in the same delivery step: on the parent's
thread for a cooperative child (the one settling the parent, which
the child was running on), on the child's own thread for a pinned
one — that thread waits for the handler's decision rather than
deciding by timing. The child the handler restarts is the one it
was handed, kept alive for it. A birth-epoch failure is held like
any other: a child whose birth failed does not start `run()` until
its handler has returned, and then starts it, restarts, or stays
quarantined as the handler decided.

One failure cannot wait and is delivered at once: a
**dissolve-epoch closure's**, because the child's region is
released right after.

The bracket costs a locus nothing unless it declares
`on_failure` and holds a locus-typed field or computes a
default; one that does pays two uncontended lock round trips per
instantiation.

### Reassigning a locus-typed field (WS1#4)

Assigning a fresh locus literal to a locus-typed field —
`self.<field> = SomeLocus { … };` — is a **lifecycle
transition**, not a value store. It is lowered **break-before-make**:

1. The instance currently in the field is reclaimed — its full
   teardown spine runs (drain → dissolve → arena freed), so its
   resources are released: `@ffi` handles closed, child loci
   cascaded, region returned. This is the same teardown a child
   gets when its parent dissolves.
2. A new instance is constructed from the literal **into self's
   own arena**, owned by the field (not scope-bound) — so it
   outlives the enclosing method and is reclaimed through the
   field when self later dissolves, exactly like a field-default
   child from params-init.
3. The field is repointed at the live new instance.

The old and new instances do not coexist: the old is fully torn
down before the new is constructed. (Treating the assignment as a
plain value store — the naive lowering — would leave the field
pointing at a scope-dissolved temporary: closed handles, freed
arena, use-after-free on next use. The transition lowering exists
to prevent exactly that.)

For "same instance, reconfigure," use **in-place mutation**
(`self.<field>.<x> = v;`), which stays the cheap path and triggers
no teardown. v1 scope: the new instance inherits the parent's pool;
reassigning a *pinned*-placed field does not re-apply the pinned
placement.

**The right-hand side must be a locus LITERAL** (error). Assigning
a locus value produced somewhere else — `self.held = make_row(…);`,
or `let c = Conn { … }; self.conn = c;` — is rejected, because
ownership would be ambiguous: the field claims the instance (its
teardown reclaims it when self dissolves) and so does the frame
that produced the value (its scope exit reclaims what it built).
Nothing in the language decides between them.

That ambiguity is not hypothetical. Until this rule, a locus
returned from a free fn was routed to a program-lifetime arena and
never reclaimed, which is what made such a store *appear* to work:
the field's pointer stayed valid because nothing ever freed it —
the leak was the safety mechanism. Every attempt to give those loci
a real lifetime produced either a use-after-free or silently wrong
values.

Three remedies, all existing shapes:

- **Assign a literal** — construction in place, the lifecycle
  transition above, unambiguously owned by the field.
- **Route membership through `accept(c: L)`** — parent/child
  ownership, which is the language's answer for "this locus holds
  that one" whenever the child is not a `params` field.
- **Transfer the STATE, not the instance** — where a locus is a
  *handle* over an external resource, the module that owns the
  resource may publish a transfer operation that writes the handle's
  state into the instance the field already holds and leaves the
  source at its not-acquired sentinels. That is not a locus store at
  all: no second instance enters the field, and the disarmed source's
  own teardown releases nothing. `std::process::adopt(dest, src)`
  (GH #716) is the shipped instance — the pid and three pipe fds of a
  spawned `Child` move into the field, whatever the field held is
  released first, and the source is disarmed. The first two remedies
  cannot reach that case: the resource is acquired by a syscall inside
  a factory, so there is no literal to write and `accept()` cannot
  adopt what a free fn built.

  This is a **module-provided** operation, not a language feature.
  The language does not gain move semantics: the transfer is ordinary
  field assignment written inside the module that defines the handle,
  and it is sound only because that module knows what "one owner"
  means for its resource (which descriptors, whose teardown, in which
  order). A locus with no external resource has nothing to transfer
  and needs none of this.

**Contract-typed fields hold a borrow when assigned a handle (GH
#967).** An `interface`- or `perspective(P)`-typed field cannot be
assigned a locus literal (the literal's type is not the field's), and
the only values that reach `self.<field> = v` are interface VALUES —
a parameter, another field: a handle somebody else owns. That store
is a **borrow**, the same decision F.39 makes for a name in a field
initialiser (`Owner::Borrowed`), applied to assignment:

1. The instance the field owned until then — its default, or a
   literal it was built with — is reclaimed, break-before-make as
   above.
2. The field is marked borrowed: the holder's cascade never reclaims
   what it now points at. The handle's own owner does, at its own
   time.
3. The handle is stored.

A field that already held a borrow releases nothing when assigned
again. What the rule does not decide is lifetime: the borrowed
instance has to outlive the holder, exactly as one passed by name at
construction has to — that is the contract of every handle, and the
checker's question in GH #730. Before this rule the store was a plain
value store: the field kept describing the default it was built with,
so the holder's teardown reclaimed the handed-in instance (its owner
then read freed memory, or reclaimed it a second time) and the default
leaked.

**A borrow outlives its holder (GH #730, 2026-09-22).** A handle
stored by name into a locus-carrying param field — `LocusRef`,
`interface`, `perspective(P)` — is borrowed, never the holder's to
reclaim, so what owns it must outlive the holder. Ownership is
structural and reclamation a tree cascade, so `hale check` decides
this from position, with no annotation:

| the handle comes from | holder owned by the frame | by `self` (a field, an accepted child) | by the caller (returned) |
|---|---|---|---|
| a field of `self` | sound | sound | refused |
| a `let` of this frame | sound while the binding is in scope | refused | refused |
| a bus handler's payload | sound | refused (GH #712) | refused |
| a parameter | sound | every caller is asked | left alone |

For a parameter, every call site of the method is classified by the
same table; the first caller that hands a `let` of its own frame or a
handler payload is the witness the refusal names, and a parameter at
the caller recurses to a bounded depth. Nothing else is refused: a
chain the walk cannot follow is left alone, never guessed at. A
borrow the holder reads only in `birth()` is birth-scoped and sound —
the instantiation runs inside the frame that owns the handle — which
is the shape a resident uses to copy what it was handed before the
dispatch ends. Not decided here: a borrow across thread domains, and
a container (not a locus) a handler built and handed to a resident's
form-typed field; both are named in GH #730 and #712.

This is the same principle as the no-locus-return rule on methods
(`fn get() -> SomeLocus` is rejected): **a locus is structure, not
a value to hand around.** Ordinary `let`-bound loci — including
factory results — are unaffected; they are owned by the binding
that names them. A factory result written as a locus- or
interface-typed **field of a locus literal** — `Router { quick:
make("q") }`, in a body, a default, or another factory — is owned
by that literal: the literal is the construction site, so this is
not the ambiguous store above, and the frame that made the call
does not reclaim the result at its exit (dna/FRICTION.md F.17).

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
   (Enforced 2026-08-11 — downstream handoff; this rule was
   written but never implemented, and the reinterpreted payload
   surfaced a String field's heap pointer through an Int
   parameter from safe code. The check covers both subject forms
   — declared topics and string subjects with `of type` — plus
   arity, accepts `Drain<T>` batch handlers by their element
   type, and names the topic-as-parameter-type mistake
   (`fn on_h(msg: Hello)` where `Hello` is the topic)
   specifically. Payloads that resolve `Unknown` — cross-seed
   topics — stay permissive by the milestone-2 rule; stdlib
   paths resolve to their real nominal types since GH #470 and
   are checked like any user type.)
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

#### Computed publish subjects are confined to their declaration

A send whose subject is not a literal (`subj <- v`) requires the
enclosing locus to declare a wildcard `publish` whose payload
accepts `v`. That declaration is an **authorization**, and it is
enforced — not merely required at the declaration site:

1. **Pattern.** At the send, the computed subject must lie under one
   of the locus's declared publish patterns. A locus declaring
   `publish "io.tcp.**"` cannot publish `"app.order"`. Violating
   this raises `BusPublishUnauthorized`.
2. **Payload.** The computed subject must not reach a subscription
   declared for a different payload type. Violating this raises
   `BusPayloadMismatch`.

Both are runtime checks on the computed path only; a literal
subject is bound to its declaration at compile time and pays
nothing for either.

They are not stylistic. Without them the computed string reached
dispatch verbatim, so a subject outside the declared pattern was
delivered to whatever subscribed to it and the payload was
reinterpreted as that subscriber's type — a two-field
`LogEv { a, b }` published on `"app.order"` arrived at an `Order`
handler as `id=a qty=b`, with `hale check` reporting `ok`.

Statically, a subscription that sits under another locus's declared
wildcard pattern while expecting a different payload is a
**warning**: whether the hazard is live depends on whether that
locus ever publishes a subject reaching it (a `std::log::Logger`
declares all of `log.**` but publishes only under its own path), so
it names the risk without refusing the program.

**The pattern also bounds analysis.** Because a computed publish
cannot escape its declaration, an unresolved publish inside a locus
declaring `"io.tcp.**"` provably cannot produce a publisher of an
application topic. Subject-specific questions — how many publishers
a topic has, whether `@effects(depends:)` names the complete
inbound surface — are answered against the patterns rather than
against one program-global "some publish is unknown" bit. Residue
that carries no pattern (an unfollowable call, a truncated
frontier, a publish whose subject expression resolves to nothing)
stays unbounded and still withdraws every subject.

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
different binaries by varying the main locus. An entry may name an
imported topic — `dna::ReviewVerdict: unix(...)` binds the
declaration the library owns, never a re-declaration of it — and
resolves exactly as a qualified bus subject does (GH #527 B6).

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

**The publish contract (GH #227, 2026-07-22).** `T <- value`
succeeding means *the broker accepted the message*; what
"accepted" obligates the broker to is defined per binding, and
the broker may never accept a message it already knows it cannot
handle under that binding's guarantee. Per binding:

| Binding            | "Accepted" means                                                        |
|--------------------|-------------------------------------------------------------------------|
| none (in-process)  | dispatched to every born subscriber in this binary                      |
| `unix(...)`        | handed to the peer connection, message boundaries preserved             |
| `udp://...`        | handed to the local IP stack; lossy from there **by declaration**       |
| `shm_ring(...)`    | slot claimed + committed, under the declared `on_overflow` policy       |
| adapter locus      | whatever the adapter's own contract says (it owns delivery)             |
| `api: unix(...)`   | a command: dispatched to every born subscriber, or answered by the one handler that declares a return type; a read: answered from the locus's own pool; a stream: forwarded to every attached watcher under its bound (§ "The api binding") |

This contract is what lets `<-` be an infallible statement: the
error channel isn't missing, it's relocated to where it can be
acted on. Consequences, all normative:

- **Binding realization failure is a birth failure of the
  declaring locus.** A `bindings { }` entry (or
  `LOTUS_BUS_CONFIG` route) whose transport cannot be opened —
  socket/bind/listen/addr failure, connect-retry timeout,
  unparseable route — refuses the boot. There is no code path
  where a binding fails to open and subsequent publishes report
  success. Bindings live on the `main` locus, whose parent is
  the root, so the unhandled default is the root failure shape:
  the structural diagnostic on stderr + non-zero exit (the same
  seat `lotus_root_panic` occupies; routing through a main-locus
  `on_failure` is the planned extension of that seat). The
  runtime primitive is `lotus_bus_binding_fail(subject, url)`.
  Listener-side realization is synchronous at registration
  (socket + bind + listen; UDP: parse + bind + group join) —
  only the blocking accept / recv loop runs on the reader
  thread, so a dead binding can never fail invisibly on a
  detached thread.
- **Per-send transient errors on a lossy transport are not
  structural.** A UDP `sendto` failure is logged (once per errno
  class), not fatal: the binding's guarantee is best-effort by
  declaration, so downstream loss is within contract.
- **Peer EOF on a listen binding is not loss.** The listener is
  still bound; the serve loop re-arms and accepts the next peer
  (GH #233 step 2), so rolling restarts of connect-side
  binaries work without policy.
- **Listen-side ingress the kernel accepted is delivered
  (GH #468).** Three normative pieces, closing what used to be
  silent loss at the registry's edges:
  1. *Binding readers imply the locked queue.* Any `bindings { }`
     block makes the program off-thread for queue purposes
     (compile-time, same flag as pinned placement), and the
     runtime re-asserts it whenever it spawns a reader thread
     (covers `LOTUS_BUS_CONFIG`) — a concurrent reader enqueue
     can never race the owner's drain on the unlocked
     single-threaded path. This was the actual mechanism behind
     "a loaded run occasionally loses a mid-stream wire
     message": two binding readers and the drain on an unlocked
     queue.
  2. *The boot window buffers.* A message received between
     transport realization and the same birth's (later)
     subscriber registrations is buffered — bounded, per
     binding: 64 messages / 1 MiB, oldest-first eviction,
     counted (`buffered_early` / `dropped_early` in the
     counters dump) — and flushed FIFO the moment a matching
     registration lands. A relay-shaped program with no local
     subscriber degrades to the old drop behavior at the cap,
     counted instead of silent. Post-boot, a message with no
     registered deserializer remains the documented
     relay-shaped skip.
  3. *Exit quiesces before teardown.* At every main-exit point,
     before pools join and loci dissolve, listen fds half-close
     (new connections are refused; queued data and
     already-accepted backlog connections stay readable — the
     kernel returns queued AF_UNIX data before EOF even after
     `shutdown`), readers drain to true EOF through the
     still-intact registry, buffered boot-window residue is
     flushed, and one final local drain delivers to still-alive
     handlers. Bounded: `LOTUS_BUS_QUIESCE_MS` (default 500,
     `0` disables), loud if exceeded — a silent peer holding a
     connection open cannot stall exit. Wire data arriving
     after the half-close arrived after exit and is outside the
     contract.
- **Connect-side loss is structural, with reconnection as
  supervision policy (GH #233 steps 3–4).** A send failure on a
  source-declared connect binding marks the entry *lost*
  (fanout skips it — publishes during the window are dropped,
  never falsely "delivered") and queues a loss event dispatched
  on the owner thread at the next queue drain. Default: the
  structural exit, diagnostic naming the subject. Handled: the
  main locus declares
  `on_failure(t: std::bus::UnixTransport, err: ClosureViolation)`
  — the handler receives a synthetic `link_lost` violation, and
  `restart (t);` re-runs the connect-with-retry; on success the
  binding resumes, on failure (or a handler that declines) the
  structural exit fires. Reconnection is a supervision decision,
  not a transport feature. `LOTUS_BUS_CONFIG` connect routes sit
  outside the supervision tree (no locus to route through) and
  keep logged-only send failures.
- **Malformed `LOTUS_BUS_CONFIG` lines stay warn-and-skip**
  (the file is an operator-layered override and the diagnostic
  names the line); a well-formed line whose route cannot be
  *opened* fails the boot like a `bindings { }` entry.


#### The api binding (GH #1106)

One entry in the main locus's `bindings { }`, headed by the
contextual keyword `api`, binds the program's API rather than one
topic:

```hale
main locus App {
    bindings {
        api: unix("/run/app.sock", bound: 64, on_full: refuse);
    }
}
```

Nothing else in the source changes. The entry binds **every topic
some locus of the entrypoint's own seed subscribes** (a *command*),
**every topic such a locus publishes** (a *stream*), and **every
`expose` member of the main locus or of a param-default child of the
seed's own whose type appears once among main's params** (a *read*,
named `member` on main and `param.member` on the child). A locus that
came in through `import` is not part of the surface, however much of
it the entrypoint composes (GH #1104 piece 5): a library's internal
bus is not the application's API, and a head importing its core must
not serve the core's topics as commands. An imported *topic* a seed's
own locus subscribes or publishes is served as any other, under its
qualified name. An imported seed's `main locus` is not the entrypoint
either: renamed with its seed, it does not count toward the one-main
rule, and an `api:` entry it carries is inert — a composed head that
imports the standalone head declares its own entry to get a socket. A subscription by literal subject
(`subscribe "log.**" ...`) names no topic and is not part of the
API; a command reaches the loci that subscribe the topic by name,
not those hearing it through a parent topic. `hale run --api
<path>` (and `hale build --api <path>`, flags before the target)
synthesizes the entry above with the dev defaults, `bound: 64,
on_full: refuse`, and needs a `main locus` to put it on: a bare
`fn main` program is refused with the rule. `LOTUS_API=<path>` at
run time overrides the socket path of an entry the program
carries and never creates one, so a binary built without the entry
pays nothing. The socket file is unlinked when the listener binds
(a crashed predecessor leaves one) and again at dissolve. The
listener is born as the last param of the main locus, so it
appears once every earlier param is born; a caller that races the
boot connects with a wait.

**The two knobs.** `bound: N` and `on_full: refuse` are required
and describe the request side: at most N commands and reads
awaiting a handler's answer per binding; the N+1th caller receives
an `over_bound` receipt and nothing inside the program is touched.
`refuse` is the only request policy: a caller waiting for an answer
cannot be shed silently, and `wait` is left for when its disposition
has a consumer. Each attached watcher has a queue of its own,
`watch_bound: M` frames with `on_watch_full: drop_old | drop_new`;
the two go together, and when both are omitted a watcher gets
`bound` frames and `drop_old`. Frames a watcher's queue sheds are
counted and reported on the next frame it does receive. The third
knob is the gate's: `on_unauthorized: refuse` (the default: a
receipt, `unknown` for an item outside the caller's slice) or `drop`
(no answer), and `roles: <expr>` names the membership source (GH
#1109, below).

**The wire.** A Unix domain stream socket carrying one JSON object
per line. A request is one of

```text
{"call": "Verdict", "payload": {...}}     a command, the payload the topic's type
{"read": "billing.ledger"}                a snapshot of an exposed member
{"watch": "PriceMoved"}                   attach to a stream
{"describe": true}                        the description, the slice this caller's roles show (spec/model.md § "The description")
{"describe": "full"}                      the whole description: a read gated on `owner` (GH #1109)
```

each with an optional `"id"` the client chooses (any JSON value,
echoed verbatim). Every answer carries `"request_id"`, an integer
the binding assigns, unique for the binding's lifetime and
increasing; a line that is not a request at all is refused with
`request_id` 0, since no request was admitted. The answers:

```text
{"request_id": 7, "id": ..., "ok": true, "value": {...}}                  a command whose handler declares a return type
{"request_id": 7, "id": ..., "ok": true, "accepted": true}                a command no subscriber answers: dispatched
{"request_id": 7, "id": ..., "ok": true, "value": {...}, "as_of": "..."}  a read
{"request_id": 7, "id": ..., "ok": true, "attached": "PriceMoved"}        a watch
{"request_id": 7, "id": ..., "ok": false, "refusal": {"kind": "...", "reason": "..."}}
{"stream": "PriceMoved", "value": {...}}                                  a frame, after an attach
{"stream": "PriceMoved", "dropped": 3}                                    frames the watcher's queue shed since its last frame
```

A describe answers `{"ok": true, "value": <description>}`: the
document `hale check --dump-api` emits for the same program, byte
for byte, so a client learns the whole surface from the binding and
needs no topic name in advance. `hale describe`, `hale call`, `hale
watch`, `hale admin` and `hale mcp --app` are those clients.

The refusal kinds are `malformed` (not a JSON object, no verb, no
`payload` object on a call, or a payload that does not decode: the
reason names `missing_field` or `wrong_type` and the field),
`unknown` (no such topic or read), `not_a_command` (a stream named
in a call), `not_a_stream` (a command named in a watch),
`over_bound`, `unauthenticated` (GH #1109: the kernel would not say
who the peer is; such a peer is refused every request, gated or not,
since the binding vouches for who is calling and -1 is nobody), and
`unauthorized` (the caller lacks `owner` for `{"describe": "full"}`;
the refusal carries `"role"`). A gated command, read or stream a
caller may not use is refused as `unknown`, exactly as a name that
does not exist: what lies outside a caller's slice is not disclosed
to it, and only `full`, whose existence every caller knows, names
the role it needs. `on_unauthorized: drop` turns either refusal into
no answer at all. A refusal is a value-channel answer, never a
failure of the program. Answers arrive in the order the program produces
them, so a refusal the socket side issues itself may precede the
answer to an earlier request still with its handler; a client
correlates by `id`.

**The reply is the return type.** A subscribed handler may declare
a return type. Through the binding, the value it returns is the
reply, encoded as JSON; among the subscribers of one topic at most
one may declare a return type, else the entry is an error naming
both handlers; a command no subscriber answers is answered
`accepted` by the binding itself once dispatched. An intra-process
publish of the same topic calls the handler as before and ignores
the return, so a topic that never crosses the binding pays nothing.

**The binding authenticates; nothing else does (GH #1108).** Who is
calling is established at the api binding and nowhere else: a peer
on its Unix socket is the principal its kernel credentials name
(`mode: "unix"`, `name: "uid:<n>"`, with `uid`, `gid` and `pid` as
`SO_PEERCRED` / `getpeereid` report them; credentials the kernel
would not give are `-1`, and such a peer is **unauthenticated**,
never anyone). A message that did not come through the api binding
carries the **local principal** (`mode: "local"`, `name: "local"`,
credentials -1): a publish inside the program, and also a cell an
env-routed `LOTUS_BUS_CONFIG` transport delivered, which the program
cannot tell apart. `local` is therefore never a statement of trust,
only of provenance; a topic bound to a transport in `bindings { }`
refuses a `Context` handler outright, since a cell from another
process would reach it as `local`. A bearer token on HTTP is the
third mode and waits for the HTTP transport; it is not spelled here
until it exists. Every answer the binding writes, refusals included,
carries the principal it established:

```text
{"request_id": 7, "id": ..., "ok": true, "value": {...},
 "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
```

**The gate (GH #1109).** A role is declared vocabulary
(`spec/types.md` § "Roles and `@gated`"); `@gated(role: R)` on a
subscribed handler, an `expose` member or a `publish` member says
that a call on the topic, a read of the member or a watch of the
stream **arriving through the api binding** is refused unless the
caller's principal holds R. Commands and reads are checked per
message at the binding; a stream is checked once, when the watcher
attaches, and a stream follows the same gate as the topic's
subscribers unless its `publish` member states its own. Holding R
means the membership source answers yes for R
itself or for any role whose `includes` chain reaches R; the first
that answers is the **authorizing role**, and it is written on the
receipt as `"role"` and handed to a `Context`-taking handler as
`ctx.role` (empty for an ungated operation, and for anything that did
not cross the binding). The source is a locus satisfying
`std::api::RoleSource` (`fn holds(p: Principal, r: String) ->
Bool`, the direct question only): the one the entry names with
`roles: <expr>` — an expression the main locus evaluates as a param
default, a locus literal or one of main's own params (`self.roles`),
so the program builds its source with its own state and keeps a
handle to it, and the binding holds it as a `std::api::RoleSource`
(a borrow) — or the stdlib's `std::api::StaticRoles`, whose table
`hale build --env <name>` / `hale run --env <name>` bakes from
`[environments.<name>.roles]` in `hale.toml` and `LOTUS_API_ROLES`
overrides at run time. The table travels as one line the binding
re-splits, so it is held to one rule at check, at build and at
birth: a key is a role the program declares (an identifier), a
member is `uid:<n>`, `gid:<n>`, `user:<name>`, `group:<name>` (an
account name: letters, digits, `.`, `_`, `-`, `@`) or `*`; a table
outside that rule is a manifest error, and at birth the binding
refuses to start, saying which entry. A `gid:` member matches the
peer's primary group or one of the supplementary groups the kernel
holds for the connection (`SO_PEERGROUPS`; nothing is looked up per
request); `user:` and `group:` resolve once, at birth, per the
host's account database. Without a table every gate refuses, and
the build says so once. A principal the binding could not
authenticate (uid -1) holds no role whatever the table says, and is
refused every request. The description the
binding serves is the caller's slice: the commands, reads and
streams it may use (an ungated item always), with the schemas those
items reference; the whole document is itself a read gated on the
built-in role `owner` (`{"describe": "full"}`), and it is the
document `hale check --dump-api` emits, byte for byte. The gate is a
boundary check at ⋈ and nothing more: it says nothing about the
program's internal call paths (the description's `notes.gates` says
the same), and a `@gated` anywhere but those three sites is an error
so that no annotation promises a check that does not run. A gated
handler's topic cannot also be bound to a transport in `bindings
{ }`, for the same reason. Every subscriber of one topic, and every
publisher of one stream, states the same gate, because the binding
refuses the message, not the handler.

**The handler signature rule.** As with `Drain<T>`, the `subscribe`
line never changes; the handler's parameter list declares what the
substrate hands it. Three independent axes: the payload shape (`T`
or `Drain<T>`), identity-awareness (with or without a second
parameter `ctx: std::api::Context`), and the reply (with or without
a return type); any combination is a handler, and a parameter list
of any other shape is refused at the subscribe site. Bus dispatch
hands a handler one payload, so codegen registers a wrapper for a
handler that takes a `Context`: the handler itself stays the
subscriber by name in every analysis, the model and the manifest,
and the wrapper adds `std::api::local_context()` (the local
principal, `via: "local"`, request id 0, no role), built for that
delivery in a subregion of the locus's own arena and released when
the handler returns. Through the api binding the synthesized
subscription passes the caller the binding established, `via:
"api"`, the request id, and the authorizing role. A handler never asks whether it was reached
from outside; it reads `via`. `Context` and `Principal` are
ordinary structs (`spec/stdlib.md` § `std::api`): constructible in
a test, forwardable in a payload; provenance in `via` is what tells
a binding-produced one apart, not restricted spelling.
A batch handler (`Drain<T>`) is not reached through the binding:
the cooperative queue has no batch delivery yet, so a topic one
subscribes is left out (with a warning) and bulk requests wait on
that substrate.

**Reads are snapshots.** A binding never reads a field across
pools. An exposed member lowers to a synthesized read subject
whose handler runs on the locus's own pool and answers a JSON copy
of the member (a field, or a no-argument infallible fn's result),
so the answer is the state at that instant on that thread and a
later write does not touch it. Every read reply carries `as_of`:
the runtime keeps no per-locus sequence, so v1 defines it as
`sha256:` plus the hex digest of the answered JSON, which two reads
compare equal on exactly when the snapshot did not change and a
later command can fence on. A live view is what a stream is for.

**Codecs.** The JSON codec for every payload, return and read type
that reaches the binding is generated from the type, without
`json:` tags: `Int`, `Float`, `Bool`, `String`, and nested structs
of the same; a `json:"key"` tag renames a key here as it does for
`T::from_json`. Decoding is strict: a value of the wrong JSON kind
is `wrong_type`, a missing field without a literal default is
`missing_field`, and a handler only ever sees a decoded value. A
topic or member whose type has a field of another kind (`Decimal`,
`Time`, `Duration`, `Bytes`, arrays, enums, loci) is left out of
the API with a warning at the `api:` entry naming the field, so
adding the entry never breaks a build.

**The lowering, and what it costs.** The entry is pre-check
synthesis, no runtime change: per command an envelope type and
topic (`__api.call.<topic>`) carrying the peer, the request id and
the payload (and the topic's key, so a keyed command routes as its
topic does), and on every subscribing locus a synthesized
subscription whose handler calls the author's and publishes the
reply on `__api.reply`, keyed by peer; per read a subject
(`__api.read.<name>`) answered on the locus's pool; per stream a
forwarder into `__api.frame`. Two synthesized loci on their own
`async_io` pool own the socket: the binding (listener, request
ids, the bound, stream forwarding) and one accepted child per
connection (line reader, reply writer, watcher queue). All of it
is typechecked like the author's code, and it shows in `hale
topology` under `__api.*` subjects. The correlation store is the
membrane's shape (#684): the request row exists before dispatch,
an answer is written once, and a request is admitted once.

Transport surface:

- `unix("/path")` or `unix("/path", role: connect|listen)` —
  AF_UNIX framed-byte transport. Substrate-provided as a
  **locus** (GH #233 / F.37): the entry is sugar that
  instantiates `__StdBusUnixListenTransport` /
  `__StdBusUnixConnectTransport` (stdlib `bus.hl`) as a
  cooperative child of the main locus, converging with the
  adapter path. `birth()` realizes the transport synchronously
  on the boot path (so a dead binding fails the boot, per the
  publish contract above); the listen side's birth spawns the C
  serve thread, whose loop accepts a peer, fans recv'd payloads
  into the local handler set, and **re-arms on peer EOF**
  (closes the dead connection and accepts the next peer — a
  restarted connect-side binary reconnects without the
  subscriber noticing); `dissolve()` interrupts + joins the
  serve thread and destroys the transport. `role: connect`
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
  An adapter declared by an imported library is named through
  the import alias, `alias::Adapter { ... }`, and binds exactly
  as one declared in the program's own seed; the case rule reads
  the last segment. The same holds for a `codec(alias::Codec {
  ... })` clause (GH #1034).
  Inbound dispatch from an adapter into the local handler set is
  handled by `std::bus::__local_dispatch(subject, bytes)` (m105):
  it reconstructs the payload against the subject's registered
  deserialize fn and fans into local subscribers via
  `lotus_bus_dispatch_wire`.
  **Placement.** A bound adapter is pinned by construction (F.31):
  its `run()` gets a thread of its own, and every subscription the
  adapter declares is delivered on that thread, at its `run()`'s
  next yield (a `sleep`, a park) or after `run()` returns. `send` is
  the exception: the runtime calls it on the PUBLISHER's thread. A
  publish from inside `send` onto a topic the adapter itself
  subscribes therefore goes through the bus to the adapter's thread
  — never the closed-world direct call (§ "Closed-world topology
  optimization") — so one locus can own a socket and be both its
  `send` and its receive loop, with every write on its own thread
  (GH #1032).
  The `bytes` a `send` call receives are valid for that call: the
  runtime builds them in an arena of the call's own, freed when
  `send` returns, so publishing through an adapter holds memory only
  for the sends in flight — none per message, whether `send` returns
  at once or parks (a `sleep` or a `recv` on an `async_io` pool)
  while other publishes overlap it (GH #1038). A `send` body that
  keeps the bytes — stores them in a field, publishes them onward —
  keeps a copy, as it would any stored value.
  Whatever `send` does, every other route of the same subject —
  an env-routed `unix://` or `udp://` connect (`LOTUS_BUS_CONFIG`)
  — sends the bytes the program published:
  a `send` that publishes, or parks while another publish runs on
  its thread, cannot change what the routes after it see (GH #1058).

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

  **Subscribers (Form K6b).** Hale-side
  `bus subscribe` for shm_ring-bound topics is wired.
  Codegen emits a `lotus_bus_register_subscriber_shm_ring(...)`
  call at the subscriber locus's birth lifecycle; the C
  runtime opens the ring, spawns a dedicated reader thread
  per binding, and dispatches each newly-committed slot to
  the user's `fn on_foo(p: T)` handler with `p` pointing
  directly into the ring slot (no memcpy on the subscriber
  side).

  **Batch / drain dispatch (`Drain<T>`).** The
  dispatch mode is selected by the handler's PARAMETER TYPE,
  using the *same* `subscribe Topic as on_x;` keyword:

  - `fn on_x(t: T)` — per-record (above). The reader thread
    calls the handler once per committed slot.
  - `fn on_x(feed: Drain<T>)` — BATCH. The reader thread calls
    the handler ONCE per available batch, passing a `Drain<T>`
    handle. The handler consumes the batch with an inline
    `for t in feed { ... }` loop — there is NO per-record
    function call, and no per-call handler arena scratch. This
    is the throughput path for high-rate cross-process feeds,
    where the per-record call + scratch overhead is what loses
    to a bare consumer loop.

  `Drain<T>` is a built-in 1-arg type constructor (not a user
  generic). It is only spellable as a batch handler's single
  param and as the iterable of `for t in feed`; the loop binds
  `t` to each record read zero-copy through the ring slot —
  `t.field` accesses GEP directly into the mapped slot, exactly
  like the per-record handler's payload param. A batch handler
  registers through
  `lotus_bus_register_subscriber_shm_ring_batch(...)` (which
  spawns `shm_ring_batch_reader_thread`) instead of the
  per-record registration; the handle's runtime ABI is
  `{ void* ring, int64_t start_seqno, int64_t end_seqno }`. The
  consumer cursor is release-stored once per batch (not per
  record). Batch handlers on a `layout:`-bound (foreign) ring
  are not supported yet — use a per-record handler there.

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

  **Back-pressure (Form K7).** `on_overflow:`
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
  the consumer first, or move the publishing into the
  **main locus's** `run()`, which does begin after all
  child births. Moving it into a *child's* `run()` does
  NOT help: a cooperative child's `run()` runs inline
  during its own birth, before its later siblings are
  born — see [§ Birth order is
  load-bearing](#birth-order-is-load-bearing).
  (Cross-binary deployments aren't affected — the
  subscriber lives in a different process and exists
  before the publisher process starts.)

**Foreign rings via `ring_layout` (Proposal B).**
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

*Record headers (`record_header_bytes`).* The default `byte_records`
shape is `[len_prefix][payload]` — the prefix is the whole per-record
overhead. A real foreign producer often prepends a fixed header
(sequence number, kernel timestamps, opcode) before the payload. Set
`record_header_bytes N` on the `byte_records` framing to describe it:
the payload then starts `N` bytes into the record and the stride is
`N + align(len)` (the `len` field is still read at record offset 0 with
`len_prefix`). `N` must be a multiple of `align`. A producer that marks
a tail pad with a header *field* rather than a `len` sentinel (e.g. a
`kind` byte where `1` means padding) declares
`pad_field_offset` / `pad_field_width` / `pad_field_value`; a record
whose field equals that value is skipped to the wrap. The in-band
header scalars are surfaced to the handler by declaring their offsets
(`seq_offset` / `seq_width`, `kernel_ns_offset` / `kernel_ns_width`,
`user_ns_offset` / `user_ns_width`): the reader decodes them per record
into thread-locals the subscribe handler reads via
`std::shm::last_record_{seq, kernel_ns, user_ns}()` — the errno-style
idiom of `recv_stamped`'s `last_recv_*_ns`. The payload itself is still
delivered as the `BytesView` / typed value.
`recheck post_copy` adds a torn-read guard: each record is copied out,
an acquire fence taken, and the cursor re-read; if a free-running
producer lapped the record during the copy it is discarded rather than
handed to the handler.

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
  `from_json` is `fallible`, so callers must address it. The same tags
  drive emit: `Type::to_json(v) -> String` serializes a value back (bare
  numbers/bools, escaped strings, nested structs recursed), round-tripping
  with `from_json`; it is not fallible. A field whose
  type is another generated JSON struct is parsed recursively (the nested
  object's raw text is handed to that type's parser; a nested failure
  propagates). Array fields are **not** supported by design — Hale
  sequences are locus-owned (there is no heap-owning value collection),
  so a JSON array is read by walking the array cursor and pushing into a
  `@form(vec)` locus cell, not parsed into a struct value field (see
  `notes/value-collections.md`). Further tag keys remain reserved for
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
constraint as the native subscriber). The `slots` (fixed-stride)
framing kind ships for *consumers*, and the zero-copy writable
producer view ships as `Topic.write(max) { w => … }` (A1 — fields
written directly into the reserved slot, no intermediate copy). What
remains post-v1: a `slots` *producer* with parameterized slot
geometry, and multi-cursor back-pressure.

**In-memory delivery is absence-of-entry.** A topic with no
binding entry is delivered same-process via the cooperative
queue. There is no `in_memory` variant — the runtime default
covers the case and explicit syntax would be ceremony.

**Operational constraints (Form K).** A binding
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
(Form K4a):

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
   leaf must be a fixed-layout primitive or a struct whose
   fields are all flat-shapeable. String, Bytes, BytesView,
   StringView fail the predicate (heap-shaped / fat-pointer),
   and so do **arrays — fixed- or unbounded-size**: codegen
   stores an array field out-of-line (the field is a pointer,
   not the inline bytes), so a raw memcpy of the value would
   share a pointer that dangles across the zero-copy / shm
   boundary — a cross-process use-after-free. The binding is
   rejected at typecheck with a diagnostic naming the offending
   shape, rather than compiling to a runtime segfault. (Inlining
   array fields for flat payloads — which would let fixed-size
   arrays be `zero_copy`-eligible again — is a future codegen
   change; until then, use only fixed-size scalar fields in a
   `zero_copy` payload, or send variable data as `Bytes`/a
   `layout:`-bound `BytesView` raw frame.)

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
- **Bus adapters (GH #1032).** A locus named as an adapter in a
  `bindings { }` entry is never the publisher of a rewrite: the
  runtime calls its `send` on the publisher's thread, so a publish
  inside it may execute off the adapter's own thread, and a direct
  call would run the handler there. Its publishes take the bus,
  which posts to the adapter's mailbox.
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

### Phase 3: routing keys (v0.1 proposal)

Phase 3 extends topic declarations with a per-message **routing
key** so the bus can shard dispatch by key value at the
`(subject, key)` granularity, rather than fanning every published
message to every subscriber on the subject. Motivated by a
downstream market-data workload: one
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

topic MarketL2 {                            // (1) topic-decl additions
    payload:      L2Data;
    subject:      "market.l2";
    keyed_by      sym_id;                   //  ←  new
    on_unmatched: swallow;                  //  ←  new (default if absent)
}

locus BookSignal {
    params { sym_id: Int = 0; ... }
    bus {                                   // (2) subscribe-clause filter
        subscribe MarketL2 as on_l2
                  where key == self.sym_id; //  ←  new
    }
    fn on_l2(d: L2Data) { /* d.sym_id == self.sym_id, statically */ }
}

main locus Ingest {                           // (3) per-instance bindings
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
| `String` | u64 hash + owned copy | one i64 cmp; full compare on hash match |

The bus runtime stores both halves of a u128 uniformly
(`key_lo: u64, key_hi: u64`) — narrower types zero-extend. Apps
that need compound keys (`(sym_id, venue, side)`) pack them into
a `Decimal` field themselves; the language does not bake compound-
key derivation at v0.1.

`String` keys (2026-07-17) hash-gate the per-entry compare: the
registry stores the subscriber key's 64-bit hash plus its own
copy of the string (capture-by-value — see "Key stability"
below), the publish site hashes the payload's `keyed_by` field,
and only a hash match pays the full string compare, so a
mismatched key still costs one i64 compare per entry. `StringView`
and `Bytes` are not key-eligible. Remote fanout is unkeyed for
String keys just as for scalars — no key material crosses a
process boundary. The RECEIVE side of a binding derives the key
instead (2026-09-09, DNA F.12): codegen synthesizes one extractor
per keyed wire subject, the publish site's exact computation over
the deserialized payload, and every inbound path (unix serve loop,
boot-window flush, UDP reader, adapter inbound) dispatches keyed.
A `where key == …` subscription therefore means the same thing on
both sides of a socket; before this it received nothing over a
binding. An adapter's inbound delivery
(`std::bus::__local_dispatch(subject, bytes)`) carries wire bytes
and no key, so for a keyed subject the runtime decodes the bytes
once — through the binding's `codec(...)` when it has one — into an
arena of the delivery's own, derives the key, and delivers keyed:
matching `where key ==` subscribers and unfiltered ones hear it,
non-matching ones do not, and each subscriber still gets its own
decoded copy (GH #1041). That arena is freed when the delivery
returns and draws its memory from the thread's chunk pool, so
deriving the key costs no heap allocation in steady state. An unkeyed subject takes the
unkeyed wire path unchanged.

**`where key == EXPR` — what EXPR can be.**

The RHS is evaluated at the subscribing locus's instantiation (the
point where its `params` defaults are resolved), and the resulting
key value is captured into the bus registry alongside the
handler's self pointer. v0.1 restricts EXPR to:

1. An integer / decimal / string literal: `where key == 42`,
   `where key == "lobby"`
2. A const identifier resolving to a scalar of the topic's key type
3. A `self.<field>` path read, where `<field>` is a `params`-block
   field of the subscribing locus (for a String-keyed topic, a
   `String` field — e.g. `where key == self.name`)
4. The bare word `replica` (2026-08-12, contextual — only this
   exact RHS position): the subscribing INSTANCE's 0-based replica
   index. Each instance of a `pinned(..., replicas = K)` fan-out
   registers its own index, so K replicas shard an Int-keyed topic
   with one subscribe line and K spelled once, in the placement
   entry; a non-replicated instance is replica 0. Requires an
   Int-family key (String keys rejected — the index is a number).
   This is the bridge between the placement scale axis and the
   delivery axis, built so that placement itself stays
   semantics-free: the filter is still written on the
   subscription; the placement only decides how many indices
   exist.

A `where key == …` filter of any shape requires a KEYED topic
(2026-08-12): an unkeyed publish never runs the key match, so a
filtered subscriber would silently receive nothing — previously
accepted without a word, now a type error.

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

The key expression is evaluated once, when the subscription is
registered: at the instance's construction, before `birth()` runs.
A key a locus computes in `birth()` is therefore not the registered
one — the field's default is — and assigning the field later does
not retarget the subscription (GH #737). A key comes in as a param
at the literal.
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

**v0.2 — err-payload Send dispositions.**

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
`spec/decisions.md` § F.31 for the intrinsic-vs-
deployment axis the block sits on, and `spec/runtime.md` §
"Placement classes" for the runtime semantics.

### Syntax

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
13. **Empty / degenerate `pinned(cores = …)` (error).** A cpuset
    affinity spec that selects no cores is rejected statically: an
    exclusive range whose upper bound is `≤` its lower (`4..4`,
    `8..4`), an inclusive range that runs backwards (`8..=4`), or a
    set with a duplicated element (`{2, 4, 2}`). Bounds/elements are
    integer literals — placement is closed-world — so the selected
    core list is known at compile time and a spec that reduces to
    "no cores" (or a redundant one) is an authoring error, not a
    deploy-box question. Whether the selected cores *exist* on the
    target stays best-effort at runtime (out-of-range indices are
    skipped, exactly as `pinned(core = N)` degrades on a smaller
    machine). Linux-only affinity; a no-op on other hosts.
    (Topology Phase 1a, 2026-07-04.)
14. **`topology { }` consistency + `pinned(node/l3)` resolution
    (error).** The declare-only `topology { }` block is validated
    statically: NUMA `node` ids must be unique; L3-domain names
    must be globally unique (they're referenced by `pinned(l3 =
    name)` without node qualification); each `cores` spec must be
    well-formed (rule 13); a core may belong to at most one L3
    domain (overlap is ambiguous affinity); and a domain core may
    not overlap a `reserve`d range (reserved cores are held back
    for the OS / main). A `pinned(node = N)` / `pinned(l3 = name)`
    placement entry must reference a domain the block declares —
    using either with no `topology { }` block, or naming an
    undeclared node/domain, is an error. Resolution is
    closed-world (ids and domain cores are literals), so the
    selected core set is known at compile time. Whether those
    cores exist on the deploy box stays best-effort at runtime.
    A `pinned(node/l3)` locus gets **thread + memory
    co-location**: its thread is affinity-masked to the domain's
    cores (Phase 1a cpuset path) *and* its arena — including
    method-scratch sub-regions — is `mbind`-bound to the node, so
    its working set lives on the node its thread runs on.
    Node binding is a raw `mbind` syscall (no libnuma dependency),
    Linux-only and best-effort (falls back to first-touch when
    the node can't be honored); non-node arenas are unchanged.
    (Topology Phase 1b, 2026-07-05.)
15. **`replicas = K` (error on `K < 1`; pinned-only).** A
    `pinned(..., replicas = K)` entry fans the field into K
    single-threaded instances — replica `i` pinned to one core of
    the affinity set (round-robin), each on its own OS thread.
    `K` must be `>= 1` (`0` / negative is rejected). `replicas` is
    valid only on `pinned`: K cooperative loci on one pool would
    share a single thread (not parallel), so `cooperative(...,
    replicas = K)` is rejected at parse with guidance toward the
    pinned form. The point is that parallelism comes from *more
    single-threaded units*, never a multi-worker pool — each
    replica is its own single consumer, so the lock-free rings, bus
    devirtualization, and single-threaded-method guarantee all hold.
    Replicas compose with `node`/`l3` (each replica's arena binds to
    the target node) and are non-addressable (no `field[i]` surface;
    they are bus-subscribing or run-loop workers). All K are joined
    and dissolved at parent teardown. (Topology Phase 1c, 2026-07-05.)
    Replicas compose with KEYED DELIVERY via `where key == replica`
    (2026-08-12) — see the routing-key filter rules.
16. **Pool affinity (2026-08-12).** `cooperative(pool = X,
    core/cores/node/l3 = …)` binds pool X's worker THREAD to the
    core set — the same affinity forms `pinned` takes, resolved
    against `topology { }`, same best-effort contract. The pool has
    one worker, so entries naming one pool must agree: a second
    entry may name the pool bare (it inherits) but a different
    affinity is a type error citing both entries. Affinity without
    a named pool (or on pool `main`) is rejected — the main pool is
    the program's main thread, whose affinity belongs to the
    operator. Thread affinity only; pool workers own no arena to
    node-bind (handler scratch lives in each locus's own arena).
17. **A `pinned` placement forbids a loop (error).** A locus whose
    `placement { }` block pins any field may not be instantiated
    inside a loop body — a `while` / `for` at any nesting depth, in
    a free fn, a locus method or a lifecycle hook. `pinned` gives
    its field its own OS thread, spawned during the placing locus's
    params-init and joined at the instantiating scope's exit, and
    the join record (the deferred-dissolve slot plus the thread
    handle beside it) is one slot per instantiation *site*: a second
    pass over the site overwrites the record of the first, so only
    the LAST instance is joined and arena-destroyed and every
    earlier pinned thread is orphaned with its arena still live.
    Placement names static resources — a core, a NUMA node,
    `replicas = K` — one thread per entry for the program's life, so
    a per-iteration thread is a category error rather than a
    reclaim policy to pick. `placement { }` is main-only (rule 1),
    so the shape this rejects is the deployment root booted once per
    iteration. The fix is to instantiate it once outside the loop;
    a loop that *calls a fn* holding the literal is unaffected and
    correct (each call joins its own thread at that fn's exit), and
    the rule is positional on that literal, so it is not a rule
    about the whole call graph. Codegen keeps a matching refusal for
    embedders that bypass the checker. (GH #826, 2026-09-20.)
18. **Every entry is consumed by exactly one instantiation
    (error).** A placement entry is carried by the locus LITERAL
    lowered for its field, and by nothing else: the thread class,
    the cooperative pool and the NUMA node all ride an override that
    the next `T { }` takes. So a placed field whose value arrives
    any other way — a factory call, a fallible call, a conditional,
    a reference to an instance somebody else built — leaves the
    entry untaken, and the next field's turn through the params-init
    loop resets it. Nothing is placed and nothing is said. That
    shape is rejected at the initialiser, with the literal form
    spelled out. The entry's value is the init written at the
    instantiation site when the literal supplies one, and the
    `params` default otherwise, so both spellings are checked — and
    a default every site overrides is dead text, not a dropped
    placement. The entry is not applied after the fact because there
    is nothing left to place: the pinned path does not mark an
    instance, it spawns a thread that runs the locus's whole
    lifecycle — birth, `run()`, the mailbox loop, drain, dissolve —
    and a factory's literal has already run birth and `run()` (and
    registered its subscriptions against the global queue) before
    the value returns. Scope matches rule 17's: an imported seed's
    main locus is renamed `__lib_*`, is not the deployment root, and
    its entries never reach the plan. Codegen keeps a matching
    refusal for embedders that bypass the checker. (GH #890,
    2026-09-20.)

19. **Uncarriable bus payload (error).** An `of type T` clause on a
    `publish` / `subscribe` must name a type the bus can carry — a
    user `type`, an enum with a payload variant, or `BytesView`
    (§ *Bus subscription dispatch* → *Payload type*, below, has the
    full statement of what the wire carries). A primitive, a tuple,
    an array, `bounded[T; N]` or a no-payload enum is rejected at
    the clause's own span. Before this rule `of type Int` checked
    clean and
    could not be lowered: codegen refused the publish
    (`bus send payload must be a user-type or has-payload enum
    value`) and the subscribe (`m60 requires a TypeRef, has-payload
    Enum, or BytesView`), unlocated and from another layer. A
    payload type the bundle cannot *resolve* — a qualified path into
    a seed it does not hold, a generic instantiation, a name nothing
    declares — is left alone: the rule is about what the bus
    carries, not about which names are in scope. A `topic`'s
    `payload:` is under the same contract, but the checker does not
    desugar topics (lowering does), so that half is still diagnosed
    during lowering. (GH #876, 2026-09-20.)

### Single-threaded-method invariant

A locus's methods may be invoked only on the OS thread that
owns the locus's placement's pool. This is enforced at
typecheck via a static call-graph walk starting from each
top-level placement entry:

1. Seed each placement entry with its pool: `gateway_a`
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

## Element chains

`xs.filter(<expr>).count()` and `xs.filter(<expr>).into(target)` are
recognized **forms**, not values. A chain is rewritten to a single
loop by a post-parse pass, so typecheck and codegen both see an
ordinary `while` and neither knows chains exist.

```hale
self.users.filter(it.active).into(self.actives)
```

`it` is bound to the current element by the rewrite. Stages fuse: two
`filter`s are one pass, because nothing is produced between them.

**Why a form and not a value.** A chain that produced something at
each step would need either a lazy object or an intermediate
collection. Both are sequence values, which need an owner, which
reopens arenas and placement — the knot that made composition look
like a language-sized change. A recognized form produces nothing at
any step, so the question does not arise.

Three properties follow, and they are the contract:

1. **A chain allocates nothing.** It is therefore legal inside
   `@budget(alloc_per_call = 0)` and `@hot`. A design that returned a
   new collection would have been illegal in exactly the code Hale
   exists for.
2. **A chain is eager.** A predicate's effects are attributed to the
   predicate's own source position. A lazy chain would execute it at
   the terminal and the witness path would name the wrong line.
3. **A chain needs no lambdas.** The predicate is an argument position
   the compiler knows about, not a value, so there is no closure to
   represent — no capture modes, no escape analysis, no cross-thread
   question.

**Boundary.** Elementwise operations fuse. Whole-set operations
(`sort_into`, `reverse_into`, `group_count_into`) cannot — they need
every element before producing any — so they are terminals that
materialize into caller storage: the loop fills (or bumps) a target
the caller owns, and any reordering runs on that target after the
loop. The chain itself still allocates nothing. Over a stream source
(a bus subscription, where the driver is delivery rather than a
loop) they are rejected outright: "needs an end" is a property the
compiler can decide.

A bare method call with no stage (`v.count()`) is an ordinary form
method, not a chain, and is left alone. A chain over a non-form
receiver fails with the ordinary "no method `get`" diagnostic.

### Vocabulary

Elementwise **stages** (fuse into the one loop):

| stage | meaning |
|---|---|
| `filter(pred)` | drop elements where the `it`-predicate is false |
| `map(expr)` | rebind the element to the `it`-expression's value |
| `take(n)` | stop the whole chain once n elements have passed this point (2026-08-11) |
| `skip(n)` | drop the first n elements that reach this point (2026-08-11) |
| `enumerate()` | bind `idx` — the 0-based count of elements reaching this stage — in every later stage and the terminal (2026-08-11) |

`take` and `skip` count elements **arriving at their own position**,
so `filter(p).skip(2)` skips the first two matches, not the first
two source elements; their limits are evaluated once, before the
loop. `enumerate` is an explicit opt-in stage rather than an
always-bound name so a chain never captures a user's own `idx`
local; written after a `filter`, `idx` counts the filtered stream.
A `min`/`max` key mentioning `idx` is left unrecognized (the best
element's key is re-derived from the element at compare time, and
its enumerate count is not recoverable from it — rejecting beats a
silently wrong comparison).

**Terminals**:

| terminal | result |
|---|---|
| `count()` | Int |
| `sum()` / `sum(seed)` | `sum()` is Int; `sum(seed)` seeds the accumulator and the seed **is** its typed zero — `sum(0.0)` sums Float elements, `sum(100)` starts an Int sum at 100 (2026-08-11) |
| `into(target)` | pushes each surviving (mapped) element |
| `any(pred?)` | Bool; **empty selection ⇒ `false`** (vacuous) |
| `all(pred)` | Bool; **empty selection ⇒ `true`** (vacuous) |
| `first()` | the first surviving element — **fallible** on empty |
| `find(pred?)` | `find(p)` ≡ `filter(p).first()` |
| `min(key?)` / `max(key?)` | the **element** with the least/greatest key (`min_by_key` shape; bare form compares elements) — fallible on empty |
| `each { … }` | run the block per surviving element |
| `sort_into(target, cmp?)` | push survivors into the caller's vec, then reorder it in place — `sort()` (primitive ordering) or `sort_by(cmp)` with a named comparator (2026-08-11) |
| `reverse_into(target)` | push survivors, then swap ends inward on the caller's vec (2026-08-11) |
| `group_count_into(target, key?)` | one hashmap `bump(key)` per survivor — increment-or-init tallying into the caller's `@form(hashmap)`; the bare form keys on the element itself (2026-08-11) |

**Chain sources.** A chain rewrites to a loop that fetches each
element through the source's `get(Int) -> T fallible(IndexError)`,
so answering that name is exactly what makes a form chainable:

| source | accessor |
|---|---|
| `@form(vec)` | its synthesized `get` |
| `@form(hashmap)` via `.entries` | `entry_at`, same shape |
| `[T; N]` | `get`, over all `N` slots |
| `bounded[T; N]` | `get`, over the **live** slots (not capacity) |

The two type-level collections are a deliberate exception to the
types-have-no-methods axiom, and the only one: their operations are
otherwise grammar intrinsics (`at(f, i)`, `count(f)`), and `at`
remains the idiomatic spelling for a direct index. `get` exists so
the chain source protocol is uniform, and is identical to `at` in
signature and semantics — on `bounded` it *is* `at`.

The rewrite happens post-parse, before typecheck, so it cannot
dispatch on the source's type; a form that does not answer `get`
fails with an ordinary no-method diagnostic at the chain's site.

**Fallible terminals ride the source's own `get`.** `first` / `find`
/ `min` / `max` lower to an index search whose value is
`src.get(idx)` — an empty result is the ordinary `IndexError`, so
`or raise` / `or fallback` / `or handler(err)` all apply with no new
error machinery:

```hale,fragment
let bob = users.find(it.id == want) or User { id: 0, age: 0, name: "?" };
let oldest = users.max(it.age) or raise;
```

For the same reason they do **not** compose with `map` (the `or`
fallback would need the mapped type while `get` yields the source
element): project *after* the find, on the returned element.

**`each` takes a block, not a lambda.** The block is spliced as the
fused loop's body with `it` bound — it executes in the enclosing
scope, so there are no capture semantics to define, and `break` /
`continue` act on the fused loop (`continue` advances to the next
element):

```hale,fragment
users.filter(it.age >= 18).each {
    total = total + it.age;
}
```

**Recognition is conservative** so user facade methods sharing these
names never get hijacked: with at least one stage, every terminal is
recognized (no ordinary method chain looks like that); stage-less, a
terminal is recognized only when its argument mentions `it` — which
is unbound outside a chain, so no valid ordinary call can look like
that — or when it is `each` with a block. Bare `xs.sum()` /
`xs.first()` therefore stay ordinary method calls (stage-less first
is just `xs.get(0) or …`; a stage-less sum can be written
`xs.map(it).sum()`). The whole-set terminals (`sort_into` /
`reverse_into` / `group_count_into`) are recognized stage-less too:
`xs.sort_into(sorted)` is their natural spelling, and the compound
`*_into` names belong to this vocabulary — unlike bare `into`, no
plausible user facade carries them (2026-08-11).

Every stage and terminal above is recognized only **after a `.`**,
so a free `fn map(...)` / `fn first(...)` / `fn count(...)` is
admissible and is called as written. The exceptions are the
names the compiler claims at a BARE call site — `sum`, `prod`,
`min`, `max` among them — which a free `fn` may not take; see
[`tokens.md` § Built-in identifiers](tokens.md) for the full set,
the rule and the diagnostic. A locus method may still carry any
of them (2026-09-20, GH #863 / GH #880).

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

### Payload type — what a subject may carry

A subject's payload type — `T` in `of type T`, or a `topic`'s
`payload:` — must be one of exactly three things:

- a user `type` (`type T { ... }`);
- an `enum` with at least one variant that carries a payload,
  which travels as that enum's storage struct;
- `BytesView` (`std::bytes::BytesView`), the raw-frame path: the
  payload is not typed at all, and the handler receives a bounded
  view over each record. This is how a foreign writer's ring is
  consumed.

A delivery is a *serialized struct*, so the payload needs a field
layout: `of type Int` has none, and neither does a tuple, an
array, or a no-payload enum. An `of type` clause naming one is
rejected at typecheck, at the clause (rule 18 above); a `topic`
whose `payload:` names one is refused during lowering.

**Within** a payload, the wire format supports primitives (`Int`,
`Float`, `Bool`, `Decimal`, `Duration`, `Time`, `String`),
`Bytes`, and **nested user struct types** recursively composed. A
bus payload may carry a struct whose fields are primitives,
Strings, Bytes, or other nested structs, at any depth. Serialize
walks the field tree in declaration order; deserialize allocates
each nested struct in the lazy global payload arena and recurses.
Arrays, tuples, and enums as bus payload *fields* are post-v1
polish.

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
5. The method exits as a `return` does: loci it `let`-bound
   dissolve and its per-call scratch is freed (GH #1036; `violate`
   used to skip both).

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

While the locus is draining — after `violate` on it, or once the
process drains on SIGINT / SIGTERM (§ "Drain cascade
(whole-process)") — the synthetic `self.draining` field reads
`true` from any locus method body. The canonical uses are a
`run()` loop that ends on it (`while !self.draining { … }`) and
suppressing downstream sends after escalation:

```hale
let r = expr or self.handle_io(err);
if !self.draining { Result <- r; }
```

`self.draining` is one of the three synthetic members exposed by
name to user code (with `self.children` and `self.k_max`); its
backing field `__drain_requested` is internal-only. All three
names are reserved — a locus may not declare a member that
shadows one; see `types.md` § Reserved member names.

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

## Perspectives: contract, `serves`, and the slot (Phase 2a)

A `perspective P { ... }` is a **contract** — a set of bodyless
`fn` signatures that form a stable ABI boundary — plus a
program-global, live-rebindable **slot** that holders dispatch
through. Phase 2a ships the contract, conformance,
the slot type, and dispatch; the live swap (`reperspective`)
followed in Phase 2b + 3 (see "The live swap" below).

```hale
perspective Router {
    fn route(code: Int) -> Int;   // bodyless contract signature
    fn health() -> Int;
}

locus RouterV1 : serves Router {  // declares conformance
    fn route(code: Int) -> Int { return code + 100; }
    fn health() -> Int { return 1; }
}

locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; }  // holds the slot
    fn handle(c: Int) -> Int { return self.router.route(c); } // calls through it
}
```

**`serves` conformance (error).** A `locus L : serves P` must
provide every contract method P declares — matching arity, param
types, and return type — **and** (Phase 2c) every bus edge P's
contract declares: a `bus { subscribe/publish ... }` block in the
perspective is part of the ABI, so a serving impl must subscribe /
publish each named subject. A missing or mismatched method, a
missing bus edge, or `serves` naming an unknown / non-perspective
symbol, is a typecheck error. Live-swapping a bus-backed
perspective (re-pointing its subscriptions) is a follow-up; until
then `reperspective` on such a perspective is rejected. This is the perspective analog of interface
structural satisfaction (and reuses its shape). The synthesized
`is_stable` (from `stable_when`) is not a contract method the impl
must provide.

**Qualified contract names (2026-09-19, GH #724).** `P` may be an
**imported** perspective, named through its import alias:
`locus X : serves lib::Routing`, `perspective(lib::Routing)` and
`reperspective self.r as lib::Double` all take a qualified path. The
path resolves through the same cross-seed rename table as a
qualified type or an `alias::fn()` call, so conformance is checked
against the imported contract and every seed's holders reach the one
program-global slot. A path that resolves to nothing — a nested
alias, a typo — is a located typecheck error at the path, reported
with the alias as written. The one exception is a tool that holds a
seed *without* its imports (`hale lsp` bundles one directory's own
files): there a path behind an unresolved alias is opaque, exactly as
`lib::Grid` or `lib::f()` already is, and the contract is checked
when the whole seed is.

**The slot type `perspective(P)`.** A holder programs against
`perspective(P)`, never a concrete impl. It is a handle: at the
LLVM level a single pointer stored in the holder's field, but
dispatch does **not** read that field.

**One global slot (1-1, not 1-N).** Each perspective P has exactly
one program-global slot — a `{ data, vtable }` cell (the interface
fat-pointer layout) named `__persp.<P>`. Every holder of
`perspective(P)` funnels through it: `self.router.route(x)` loads
the global slot, indexes the vtable at the contract-method
position, and indirect-calls with `data` (the current impl's
self) as the implicit receiver. Because the interop is closed-
world and 1-1, the compiler sees every call site and there is
exactly one target — which is what makes the Phase-2b swap a
single atomic store that redirects the whole program. (Contrast
`interface`, which is a *per-value* fat pointer, many impls, no
global slot.)

**Designation.** A `perspective(P) = Impl { }` field default
*designates* the slot: it instantiates `Impl` (an owned child of
the holder, torn down normally) and stores `{ impl_self, vtable(Impl,
P) }` into the global slot. The field itself stores the impl's
self-pointer for ownership; the slot holds the same pointer plus
the vtable for dispatch.

A containing constructor may designate instead (2026-09-04, GH
#525): `App { gw: Gateway { router: RouterV2 { } } }` overrides the
holder's default with `RouterV2`, which is instantiated as the
holder's owned child and stored into the slot exactly as the
default would have been. The override must name a locus that
`serves P`; one that does not is a typecheck error naming the
missing `serves` clause. Because the slot is program-global and
1-1, an override designates the *program's* impl, not a
per-instance one — two holders constructed with different impls
race on one slot, last designation wins, the same as two holders
with different defaults.

**Cost.** Steady state is one load + one predicted indirect call
per call into a perspective — near-direct. The mechanism is
Linux/native-agnostic (no new runtime dependency); a program that
declares no perspectives pays nothing.

### The live swap: `reperspective` (Phase 2b + 3)

`reperspective self.<field> as <Impl>;` is the live redeploy. It
re-points the perspective's global slot — identified by the
`self`-field's `perspective(P)` type — at `Impl` (which must
`serve P`). Because every holder funnels through the one slot, the
swap redirects **every** call site at once: the same
`self.router.route(...)` resolves to the new impl immediately after.

- **State-preserving (Phase 3).** The slot is `{ data, vtable }`:
  `data` is the running state (an arena-backed struct), `vtable` is
  the code. They are already separate, so the swap is a single store
  of the new impl's vtable into the slot — `data` is untouched and
  the new impl's methods continue on the **same live state** (the
  note's layout-identity "zero migration"). No re-instantiation, no
  birth defaults, nothing to tear down.
- **Footprint identity (soundness).** The vtable swap is layout-safe
  only if the new impl's field offsets match the retained state, so
  the typechecker requires **every** impl of a perspective to share
  one footprint (same params, by name and type, in order). A
  footprint *change* is the `migrate` case — rejected for now with an
  actionable diagnostic rather than silently reinterpreting bytes.
- **Rebind authority.** The statement runs on the locus that owns
  the slot (`self.<field>`), never a mere caller — the ownership
  tree is the redeploy authority. The typechecker requires the field
  to be a `perspective(P)` param of the current locus and the new
  impl to `serve P`.
- **Bus edges swap too (Phase 2c-runtime).** When the perspective
  declares a bus surface, the swap also re-points its subscriptions:
  it tombstones the current impl's registrations on the shared slot
  `data` (`lotus_bus_quarantine_self`) and re-registers the new
  impl's handlers on that same `data`. A message published after the
  swap dispatches to the new handler, operating on the carried
  state. Cooperative dispatch is deferred (a publish captures the
  handler current at that moment; handlers run at drain), so the
  swap boundary is respected per message. Perspective impls are
  designated (never `placement`-pinned), so they are cooperative —
  the re-registration routes through the global queue, no mailbox
  hand-off. (Cost: tombstoned entries are skipped, not compacted — a
  bounded per-swap cost.)

## Perspective hot-load

> **Status:** Phase 2b + 3 + 2c ship the `reperspective` swap
> (above): the atomic slot re-point, state-preserving across impls
> of one footprint, re-pointing sync dispatch AND bus subscriptions.
> A footprint-changing `migrate`, and the bus-arrival / decode /
> `stable_when` / drain flow below (transport-driven redeploy from
> the wire), remain the aspirational path.

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

### `restart(child)` / `restart(child) for N`

Restarts the child on the same instance — its arena, its
subscriptions and its param values as they stand (a restart never
re-evaluates a default):

1. The failure's drain request is lowered: the child is live again.
2. `birth()` runs again, then the birth-epoch closures.
3. `run()` runs again.

When it takes effect depends on where the failure came from. A
birth-epoch closure's failure re-runs birth right after the
handler returns, before `run()` has started. A failure raised
while `run()` executes — a `violate` in it, or for a pinned child
the tick and duration closures its thread checks after it —
restarts once `run()` has returned, on the thread that ran it (a
pinned child's own thread). A failure held while the parent
was still setting params takes effect when its handler returns
(§ "on_failure(c, err)").

A restart is not taken while the process is draining: the drain
ends the child anyway.

**The retry bound.** `restart(c) for N` gives this child at most
`N` restarts. The count is per child instance and cumulative over
its lifetime, and it is compared *before* the restart, so `for N`
admits exactly `N` restarts and the failure that follows the last
one **quarantines** the child instead — the supervisor tried `N`
times and is done with it. `for 0` is meaningful and quarantines
on the first failure without restarting.

Exhaustion quarantines rather than falling through: a bounded
supervisor has stated when to stop, and stopping means the child
does not run. This is the difference from an unbounded
`restart(c)`, which stops re-running at the default cap of 2 but
leaves the child live.

`N` is an expression, evaluated at the recovery site. Without the
modifier the bound is the default 2, so an unbounded `restart(c)`
is unchanged.

The bound is recorded in the topology artifact as `retry_bound`
on the supervision row (schema 1.10), so the declared policy and
the observed restarts are comparable.

### `restart_in_place(child)`

The same as `restart(child)`, except that before `birth()` runs
again every param is put back to the value **this instance was
built with**: the value its literal gave it, or its default as that
default evaluated when the instance was built. Params are settled
once, from the literal; a restart never re-evaluates a default. So
`Worker { tag: "a" }` restarts with `tag` `"a"`, whatever its
declared default says, and a default that builds a locus or reads
the clock is not run again. A param that holds a locus (or an
interface or perspective handle) keeps its child: a restart re-runs
this instance, not its children's construction. The retry bound
and the default cap are shared with `restart`.

Useful for transient failures that don't invalidate the
locus's structural commitments (e.g., the locus's k_max is
fine; just had a bad message).

### `quarantine(child) [for d]`

1. Pause child (no new messages dispatched, no new accepts
   permitted, no run scheduling).
2. Preserve arena and state.
3. If `for d` clause given, automatically restart after `d`.
4. Otherwise wait until parent explicitly resolves.

**Not lowered:** the `for d` duration clause. `quarantine(c)`
itself ships; `quarantine(c) for d` is refused by codegen. Note
that this `for` is a *duration* before an automatic restart — a
different modifier from `restart(c) for N`, which is a retry
count.

Also not lowered: the `until` modifier on any recovery op.

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

1. The runtime raises the process drain. From then on every
   locus's `self.draining` reads `true`, on every thread.
2. Timed waits **already in progress** return early so a loop
   reaches its check: such a `std::time::sleep` returns at its next
   100 ms slice (at once, on an `async_io` pool), and a
   deadline-bounded park on an `async_io` pool — a `recv` with a
   timeout — reports its timeout. A wait begun after the drain — a
   `drain()` or `dissolve()` body pacing a flush — runs its full
   length. A receive blocked in the kernel on a pinned or classic
   thread returns at its own timeout, not earlier; an untimed wait
   (a plain `accept`) is not interrupted.
3. Each `run()` that watches `self.draining` returns; one that
   does not keeps running.
4. Leaves dissolve first, each parent after its children, the
   root last — the ordinary end-of-program teardown, which also
   runs the runtime's own (schedulers, bus router, allocator,
   the atexit hooks).
5. Process exits 0.

The drain has a **grace period**: if the process has not exited
5 s after the signal (`LOTUS_DRAIN_GRACE_MS` sets it), the runtime
prints one line naming the signal, restores the signal's default
action and re-raises it — the process dies BY the signal, as it
would with no runtime at all (a waiting parent sees it killed by
that signal; a shell reports `128 + signal`) — so a `run()` that
never reads `self.draining` cannot keep a stopped program alive. A
**second** SIGINT / SIGTERM ends the process the same way at once.

The handling is installed only in a program that reads
`draining` somewhere. A program that cannot observe a drain keeps
the default action and ends at the signal with `128 + signal`, as
if the runtime were not there. **What can answer a drain is a live
instance** of a locus whose own code reads `draining` (GH #1077):
the runtime counts each such instance from its instantiation to its
teardown. A signal that finds none live — the read sits in an
imported package the program never builds, or every instance that
could have answered has already ended — takes the signal's default
action at once, exactly as a program without the read does, instead
of waiting out the grace with nothing to drain. A `draining` read
outside any locus body (a free function handed a locus) cannot be
counted, so a program with one drains, and waits its grace, on
every signal. The signal is caught, never
blocked, so a subprocess the program spawns inherits the default
disposition. (GH #1039: before 2026-09-24 the runtime caught
neither signal, so every program ended at the signal and a
`while !self.draining` loop never saw a drain.)

**When `main`'s `run()` ends first.** A `main` locus whose `run()`
returns while a child's `run()` still loops keeps the process alive
until that `run()` returns — when the child is pinned or on a
classic cooperative pool (`main`'s own included): a pinned receive
loop, a server whose `main` has no `run()` at all. Ending such a
program is what SIGINT / SIGTERM are for; the drain reaches the
child the same way whether `main`'s `run()` is still running or has
returned. An **`async_io`** child is the exception: its `run()`
parked on a timer or an fd when `main`'s `run()` ends does not keep
the process alive — the pool shuts down, the parked coroutine is
abandoned (`runtime.md` § the wakeable park) without seeing a drain,
and the child dissolves with the rest of the tree.

**Under `hale run`.** `hale run` stands aside for the program's
drain: it ignores SIGINT (a terminal's Ctrl-C reaches the program
through the process group) and forwards a SIGTERM sent to its own
pid, then reports how the program ended (`projects.md` § "What `hale
run` starts, `hale run` ends").

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
`spec/design-rationale.md`.

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
  - **Interface method signatures** (GH #732). A call through
    the interface carries the channel exactly as a direct call
    on a fallible method does: every `or` form applies, and a
    call that does not address the error is rejected as any
    unaddressed fallible call is. See `spec/types.md`
    § "Interface types (F.20)" for which methods satisfy one.
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

### A bare stdlib call is a warning, then an error (GH #738)

Every stdlib entry point the signature table marks `fallible` can be
called with no `or`. That call keeps the **legacy form** — the
success value, or an Int status for the write fns — and the corpus
relies on it, so it has always been accepted. The ruling of
2026-09-20 stages its end, in three steps:

1. **Now:** the bare call is a *warning* naming the callee, the
   payload and the missing disposition, and the shapes that address
   it (`or raise`, `or <fallback>`, `or discard`, `or handler(err)`).
   `hale verify`, which gates on every advisory, fails on it.
2. **`hale check --strict-fallible`:** the same finding is an error,
   the shape of `--strict-secret`.
3. **The next minor:** the default becomes the error, with a
   migration note in the changelog and the book.

The typing of the bare call is unchanged through all three: `hale
build` lowers it as it always did. A handled call and a deliberately
discarded one (`or discard`) are not reported. The inventory of the
entry points concerned is the table itself (`stdlib_surface.rs`, the
rows with a payload): 94 at the time of the ruling, across
`std::io::fs`, `std::process`, `std::http::client`, `std::io::tcp`,
`std::compress`, `std::tar`, `std::bytes`, `std::str` and
`std::time`.

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

  A **diverging** fallback is exempt from the type match
  (2026-08-03, #353): a block whose last statement is
  unconditionally `break` / `continue` / `return` / `fail` /
  `terminate` never yields, so there is no value whose type
  could match, and requiring one forced callers to invent a
  substitute that is provably never used. Deliberately
  conservative — only an UNCONDITIONAL divergence qualifies,
  because a block that can fall through genuinely does need a
  value, which is what the rule protects. `v.get(i) or
  { break; }` is the shape a generic desugar needs, since it
  cannot invent a typed default for an arbitrary element type.
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
  number" }`) rather than bouncing through a helper fn. The
  payload expression sees `err` bound to the inner call's
  error value, exactly as a substitute RHS does (2026-08-11),
  so a field-carrying translation is written inline:
  `src() or fail DstError { kind: err.kind }`. Same
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

## Declarations inside `module { }`

`module NAME { <top_decl>* }` (grammar `module_decl`) groups
declarations for the reader. It introduces **no namespace of its
own**: the resolver registers a module's declarations in the
bundle's one flat top-level scope, under their **bare** names, at
any nesting depth. `module geo { type Point { x: Int; } }` makes
`Point` — not `geo::Point` — the name every use site spells, two
modules declaring the same name are the ordinary duplicate-name
error, and the module's name is not itself a value, a type, or a
path head.

The consequence is a rule, not an implementation detail:

> A declaration inside a `module { }` is **first class**. It is
> resolved, typed, mangled, lowered and linked exactly as the same
> declaration written at the top level.

This holds for each kind: a `type` (struct, enum or alias), a
`locus`, a `perspective`, an `interface`, a `topic`, a `const`, a
generic template, and a free `fn`. It holds through an `import`
in both directions: a library's module-nested declaration gets a
mangled name and a rename-table row like any other, so the
importer reaches it as `alias::Name`, and a qualified path the
importer writes *inside* a module — a type in a signature, a
call, a struct literal, an enum variant — resolves as it would at
the top level.

Two things a module still does not change, because neither is a
name lookup:

- **The entry point.** A seed's entry point is its **top-level**
  `fn main` (or the `main locus` that one instantiates). A `fn
  main` written inside a module does not start the program — and is
  **refused**, by `hale check` as well as by the build, with a
  located error at the declaration:

  ```text
  main.hl:2:8: type error: the entry point must be top-level: a `fn main` inside `module inner` does not start the program — move it out of the module, or rename it if it is an ordinary function
  ```

  Reading it as an ordinary free fn named `main` was the
  alternative, and it made a seed whose only `fn main` was one
  brace deeper check clean and then fail to build (codegen's
  spanless `program has no fn main()`). The two layers say the same
  thing now (2026-09-20, GH #911).
- **Scoping of locals.** Ordinary lexical scope is unchanged; a
  module is not a scope.

One declaration is not admissible inside a module at all:

- **`target NAME { }`** is a program-level build directive, not a
  declaration a namespace can hold, and the **parser** refuses it
  at depth:

  ```text
  main.hl:2:5: parse error: `target` is a program-level declaration; move it to the top level
  ```

  Every consumer of a target reads
  `program.items` — the checker's capability gate, the wasm-entry
  detection in `desugar`, the stdlib gating — so one declared
  inside a module used to be silently INERT: the same program that
  was gated at the top level reported `ok` one brace deeper.
  Honouring it at any depth instead would have changed what the
  build does rather than what a diagnostic says, which is why the
  refusal was taken (2026-09-20, GH #901). The grammar's
  `module_decl = "module" , IDENTIFIER , "{" , { top_decl } , "}"`
  is therefore read with `target_decl` excluded.

**Analysis is on the same line, one check at a time.** The rule
above is about resolution and lowering, and it holds without
exception. The bundle-level *checks* are being brought to it
individually: the hot-path allocation lint and the decorator-stack
check reach inside a module (GH #764, `verification.md`), while
several siblings in `check.rs` still stop at the top level and are
tracked by GH #825 — there, a declaration one brace deeper can
escape a check the identical top-level declaration would fail.
Effect inference is the same shape and fails closed: it qualifies a
module-scoped subject's name but does not summarize its body, so
the manifest reports `unclassified` ("may do anything") rather than
a narrower set.

(GH #764 for the lint; GH #884 and GH #854 for codegen's
declaration collection, the cross-seed mangler and the pre-typecheck
rename pass. Before those, the same program was accepted at the top
level and either silently unanalyzed or unbuildable one brace
deeper.)

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

**A qualified literal checks like a local one.** `alias::Type {
... }` resolves to the merged declaration before its
initializers are validated, so field names, field types,
interface and perspective coercions, and missing required
fields are all checked exactly as they are for a literal on a
locally declared type — an unknown field is the same
`type T has no field f` error, at the offending initializer's
span. A literal whose path resolves to no visible declaration
keeps the permissive `Unknown` typing. Diagnostics name the
spelling the author wrote (`alias::Type`), not the mangled
symbol. (GH #707, 2026-09-19 — downstream handoff. Until then
only the RESULT type was resolved: a misspelled field in an
imported literal was dropped in silence and the field's default
constructed instead, so `check` reported `ok` on a program the
same literal on a local type would have rejected.)

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

**Per-importer scoped imports (A4).** Imports
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
codegen path as `hale build`, and — as of WS3.3 —
the same *import* path: both the single-file and directory forms
(`hale run ./dir`) resolve cross-seed imports, build the per-build
path-rename table, and rewrite qualified `alias::Name` references
identically. A directory `run` produces the same resolved program
as the corresponding `build` and execs it. (Previously the
directory `run` form bundled files without the rename table, so
cross-seed imports only worked under `build`; that gap is closed.)

## Region lifetime guarantees

Per `memory.md`:

- A locus's region is freed atomically on dissolve.
- Sub-regions are freed before the parent's region (drain
  cascade ensures this).
- No pointer-into-a-freed-region is reachable after region
  release (compile-time-checked + region-lifetime-checked).

> Forward-looking / deferred items for this area now live in the
> decision log — see [`decisions.md` § Deferred & future
> work](./decisions.md#deferred--future-work).
