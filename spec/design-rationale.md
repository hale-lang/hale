# Design rationale

For each major syntactic construct in the lotus grammar, this
document records:

1. **What the construct does.**
2. **What the framework commits the design to.** (The closed-graph
   evidence that constrains the design.)
3. **What the syntax commits to.** (The specific surface choices
   and what they imply.)
4. **What was considered and rejected.**

The goal is that a future reader understands not just what the
grammar parses but *why* it parses that and not something else.

---

## 0. Surface language: Go-shaped

**Commits to.** Familiar syntax for engineers; braces for blocks;
semicolons as statement terminators; `let` for binding; `fn` for
functions.

**Why.** The first authors of programs in this language are
agents and humans on Riley's team, both already fluent in Go.
Surface familiarity reduces cognitive cost and lets the
genuinely-novel parts (lifecycle, contract, projection class,
mode, closure) carry the unfamiliarity.

**Considered and rejected.**

- *Lisp-shaped (S-expressions).* Maximally machine-friendly but
  high friction for human review. Agent-first does not require
  agent-only.
- *ML-shaped (let/in, type inference everywhere).* Type
  inference is a weak match for the explicitness the framework
  wants, and ML's syntax is unfamiliar to the team.
- *Indentation-significant (Python).* The off-side rule
  interacts badly with multi-line block expressions and
  closure assertions; explicit braces are simpler.

---

## 1. ASCII-only source, names instead of Greek

**Commits to.** `phi`, `sigma`, `B`, `c`, `k_max`, `sum`, `prod`,
`approx` / `~~`. No Unicode operators in source. Renderer can
produce Greek for human display; the source is ASCII.

**Why.** The framework's named-concept registry
(`theory/notation.yaml` in the alpha-conjecture program) already
commits to: source uses names, renderer produces symbols. Lotus
inherits this. Agent-first authorship benefits from no
symbol-input friction. Tooling is simpler.

**Considered and rejected.**

- *Allow Unicode operators (`Σ`, `≈`, `φ`).* Reject because:
  source becomes editor-dependent; tooling becomes harder; no
  semantic advantage; the renderer pipeline already exists.

---

## 2. Locus declaration

```
locus AnalystL : tier 4, projection chunked {
    params { ... }
    contract { ... }
    bus { ... }
    birth(...) { ... }
    accept(child: Strategy) { ... }
    run() { ... }
    drain() { ... }
    dissolve() { ... }
    on_failure(c: Strategy, err: Error) { ... }
    mode bulk(...) -> ... { ... }
    mode harmonic(...) -> ... { ... }
    mode resolution(...) -> ... { ... }
    closure pnl_attribution { ... }
}
```

**Commits to.** A locus is the unit of declaration. Annotations
(tier, projection class) are optional; default tier is inferred
from nesting; default projection class is `chunked` if the locus
declares `accept` and the compiler cannot statically determine N.

Lifecycle members, mode declarations, failure handlers, and
closure tests all live as members of the locus body, not as
separate top-level declarations. This keeps the framework's
"every locus is its own substrate-cell" commitment syntactic:
everything about a locus is in its block.

**Why.** Every locus needs a stable identity for the compiler to
reason about. The locus *is* the unit of memory region, of
lifecycle, of closure-test scoping, of contract — making it the
syntactic block keeps these aligned. Tier and projection-class
annotations are optional because the framework's own discipline
permits inference (multi-perspective stability doesn't require
hand-declaration; learned values are commit-after-N=3).

**Considered and rejected.**

- *Locus as a struct with attached methods.* Reject because the
  framework's lifecycle states are not equivalent to methods —
  they're state-machine transitions the parent invokes via
  policy. They need their own keyword surface so the compiler
  can enforce ordering and the runtime can dispatch.
- *Tier and projection class as separate top-level decorators
  (`@tier(4) locus L`).* Reject; decorators imply meta-level
  manipulation we don't want. Annotations go inline with the
  declaration, with `:` as the annotation separator.
- *Implicit lifecycle (no keywords; just a `loop()` function).*
  Reject; explicit lifecycle states are how parent-policy-driven
  recovery becomes language-native. Without `birth`/`drain`/
  `dissolve` as distinct constructs, recovery primitives have no
  hooks.

---

## 3. `params` block (also: locus state)

```
params {
    B: int = 1_000_000;
    c: int = 1000;
    sigma: int = 10;
    phi: float = 1.0;
    capital_usd: decimal = 1_000_000.00d;
    running_sum: int = 0;
    inferred_param: int : inferred;
}
```

**Commits to.** Each param has a name, a type, and either a value
(compile-time-evaluable expression) or `: inferred`. The compiler
treats hand-declared values as priors and `inferred` values as
to-be-determined (statically by the compiler if possible,
otherwise at runtime via the lotus runtime's perspective-stability
machinery).

**`params` is also the locus's state.** Following Ruby's `@foo`
pattern, lotus collapses the params-vs-state distinction. The
declared params are simultaneously:

1. *Birth-time defaults*: overridable at instantiation
   (`AggregatorL { running_sum: 100 }`).
2. *Runtime mutable state*: accessible and reassignable via
   `self.foo` throughout the locus's lifetime, from any
   lifecycle method, mode block, closure, or member function.

There is no separate `state { ... }` block. A locus's state is
its params; its params are its state.

**Why.** Multi-perspective stability is the framework's commit
discipline. Hand-declared values are perspectives the author
provides; `inferred` is "no perspective yet, system finds one."
Collapsing params and state means the same surface is both the
declared-perspective surface and the running-state surface; no
artificial barrier between them. Aligned with how Erlang
processes hold state (one mutable bundle per process) and how
Ruby instance variables work (`@foo` is both a parameter and an
instance variable).

**Considered and rejected.**

- *Separate `state { ... }` block from `params { ... }`.*
  Reject; introduces an artificial distinction between
  "declared at birth" and "mutable at runtime" that the
  framework doesn't make. The framework's substrate-cell view
  treats locus state as one thing.
- *Make every param a literal (immutable).* Reject; loses the
  Ruby-style ergonomics; a long-running locus needs mutable
  state and forcing a separate state mechanism is ceremony.
- *Use option types instead of `inferred`.* Reject; an `Option<T>`
  param can be present-or-absent at runtime, but `inferred` is a
  compile-time / runtime determination promise. Different
  semantics, different syntax.

---

## 4. `contract` block

```
contract {
    expose position_size: decimal;
    expose pnl: decimal;
    consume book: Resolution<MarketBook>;
    consume volume: Bulk<VolumeEvent>;
}

contract: inferred ;

contract {
    expose explicit_field: int;
    expose inferred ;
    consume inferred ;
}
```

**Commits to.** `expose` declares fields visible to coordinators
above; `consume` declares typed dependencies on coordinatees
below. Either may be `inferred`. A contract may be entirely
explicit, entirely inferred, or mixed.

**Why.** This is the contract-graded visibility commitment from
the design conversation. The contract is what mediates access
between L's region and C's sub-region — physical layout is
hierarchical, logical access is contract-mediated. Making this
syntactic keeps the visibility rule auditable: a reader knows
exactly what a locus exposes and what it depends on without
having to read the whole body.

`inferred` lets the compiler synthesize the contract from the
locus body (compile-time inference) or learn it from runtime
observation (NN-style inference). The framework's discipline
guards against bad inference: a learned contract must satisfy
closure tests and respect substrate-derivation anchoring.

**Considered and rejected.**

- *Contract derived only from the body, no syntactic block.*
  Reject; explicit contracts are an audit surface. Even if the
  compiler can infer them, declaring them tracks the author's
  intent and lets later changes to the body be checked against
  the original commitment.
- *Single keyword (no expose/consume distinction).* Reject;
  vertical-only flow needs the up/down direction marked. A
  symmetric contract obscures the asymmetry of the design.

---

## 5. `bus` block

```
bus {
    subscribe "trellis.book.alpha.intent" as on_intent of type StratIntent;
    subscribe "trellis.kappa.updates" as on_kappa of type KappaUpdate;
    publish "trellis.drift" of type DriftReport;
}
```

**Commits to.** External typed message bus is a first-class
declarative surface. The grammar names subscriptions and
publications without committing to a specific bus implementation
(NATS, Unix sockets, shared memory, UDP multicast). The runtime
binds the bus block to the actual transport at link / startup
time.

**Why.** Trellis's first program needs UDP multicast input from
grease. Future programs will need NATS, Kafka, or other transports.
Declaring the bus interface in source means the language can
typecheck the messages flowing in/out without committing to a
specific runtime. This also enables the perspective-shipping
contract between analyst and executor binaries — both compile
from the same lotus source, both have type-level agreement.

**Considered and rejected.**

- *Functions as message handlers, no `bus` block.* Reject; without
  a syntactic bus surface, the compiler can't typecheck the
  outbound subjects (which are runtime-resolved string paths).
  The block makes them static.
- *Bus declarations at top level instead of inside locus.*
  Reject; the bus interface is a property of a specific locus
  (its inbound flow filter), not the whole program.

---

## 6. Lifecycle blocks

```
birth() { ... }
accept(child: Strategy) { ... }
run() { ... }
drain() { ... }
dissolve() { ... }
```

**Commits to.** Five named lifecycle states, declared as
parameterized blocks within a locus. `birth` is invoked once at
locus instantiation; `accept` is invoked on each coordinatee
attachment; `run` is the steady-state loop; `drain` halts new
work but lets in-flight finish; `dissolve` frees the region.

**Why.** Failure-recovery is parent-policy-driven, and recovery
primitives (`restart`, `quarantine`, etc.) need named states to
operate over. The compiler enforces the state machine. Missing
transitions get compiler-supplied defaults (e.g., default `dissolve`
frees the region; default `drain` waits for inflight messages).

**Considered and rejected.**

- *Implicit lifecycle (just `init`, `loop`).* Reject; recovery
  needs distinct named transitions.
- *Lifecycle as trait/interface (`impl Lifecycle for L`).*
  Reject; we don't have traits in v0, and the lifecycle is so
  central to the language that making it a stdlib trait would
  bury it. Keywords keep it visible.

---

## 7. `mode` declarations

```
mode bulk(input: [Book]) -> VolumeProfile { ... }
mode harmonic(input: [Book]) -> StrategyProfile { ... }
mode resolution(input: [Book]) -> SingleDecision { ... }
```

**Commits to.** Three modes (bulk / harmonic / resolution) are
language-native; user defines any subset; compiler emits one
implementation per declared mode.

**Why.** Modes are a framework primitive (paper 4 ME-5a/5b/7;
the framework's commitment that one kernel has three projections).
Making them syntactic means the compiler can:

- Generate optimized code per mode (vectorization for bulk,
  per-class projection for harmonic, lazy / tail-only access for
  resolution).
- Verify mode-projection invariants (e.g., bulk + harmonic
  reconstruction equals identity within tolerance — a closure
  test in itself).
- Allow callers to request a specific mode by name.

**Considered and rejected.**

- *Modes as functions (`fn bulk(...)`).* Reject; modes are tied
  to a specific kernel/locus, and the locus block needs to know
  about them for type-system reasons (the kernel's signature is
  shared across modes).
- *More than three modes.* Reject; the framework explicitly
  commits to three. Other shapes are not modes.
- *Fewer than three required.* The grammar allows zero, one, two,
  or three to be declared. The framework permits a locus that
  doesn't operate in resolution mode (e.g., a pure aggregator
  has only bulk).

---

## 8. `on_failure` handler

```
on_failure(c: Strategy, err: Error) {
    match err {
        Error::Timeout(_) -> restart_in_place(c);
        Error::Corruption(_) -> quarantine(c) for 30s;
        Error::Capital(_) -> bubble(err);
        _ -> dissolve(c);
    }
}
```

**Commits to.** A locus declares one `on_failure` handler that
the runtime invokes when a coordinatee fails. The handler
receives the failed coordinatee and the error, and chooses among
recovery primitives.

**Why.** The framework's failure-traversal commitment is
vertical-only (failures flow up to the parent), and the parent
makes the policy decision. Locus-attached `on_failure` is the
syntactic home for that policy. Per-coordinatee-class overrides
live in the contract (`consume` member with a per-class
`on_failure`); not yet in v0 grammar but reserved.

**Considered and rejected.**

- *Multiple `on_failure` handlers (per error type).* Reject for
  v0; one handler with `match` is sufficient and avoids handler-
  selection ambiguity.
- *Implicit failure handling (default `dissolve`).* The compiler
  *does* default to `dissolve(c)` if `on_failure` is omitted,
  but the keyword is mandatory whenever any non-default policy
  is wanted. The default is conservative (simplest cleanup).

---

## 9. `closure` blocks

```
closure pnl_attribution {
    sum(intent.pnl) ~~ sum(book.realized_pnl) within 0.05d;
    epoch tick;
    persists_through(restart_in_place, quarantine);
    resets_on(dissolve, replace);
}
```

**Commits to.** A closure test is a structural audit declared at
a locus. The first non-clause in the body is the assertion: two
expressions and a tolerance. Subsequent clauses control epoch
boundaries and recovery interaction.

**Why.** Cyclic-closure is a framework primitive (paper 4 ME-13;
R12 cyclic-closure pattern). Making it syntactic enables:

- Compile-time verification that the cycle exists (both sides of
  the `~~` reference defined values; the runtime accumulates
  both within the same scope).
- Runtime band-checking with named epochs.
- Recovery-event-aware accumulation (epoch resets / persists).

The `~~` operator is reserved for closure assertions only (per
precedence.md); using it elsewhere is a parse error.

**Considered and rejected.**

- *Closure tests as library calls (`assert_closure(...)`).*
  Reject; without language support the compiler can't verify
  cycle existence statically, and recovery-event handling
  becomes a runtime convention rather than a language feature.
- *Inline closure assertions instead of named blocks.* Reject;
  named tests are auditable (the audit log references closure
  names) and reusable across loci that share a structural
  cycle.

---

## 10. `perspective` declarations

```
perspective TradeKernel<T> {
    params {
        kappa_row: [decimal; 8];
        sigma_factor: decimal;
        regime_id: int;
    }
    stable_when {
        // Held to be stable when ≥3 perspectives have validated
        // and the closure tests at the producing locus pass.
        return num_validated >= 3 && closure_status == ok;
    }
    serialize_as TradeKernelV1;
}
```

**Commits to.** A perspective is a serializable parameter bundle
within a shared compiled-in schema. Both producer (analyst) and
consumer (executor) compile from the same lotus source, so the
type *is* the contract; the bus carries only parameter values.

**Why.** This is the analyst/executor split discussed in the
design conversation. The serialization format isn't a separate
concern — it's the perspective type. Compile-time type agreement
between binaries means no protocol-versioning handshake; the
schema version is the source-code version they both compile
from.

`stable_when` is a function-level boolean expression that the
runtime evaluates to decide whether a perspective is ready to
ship. This puts multi-perspective-stability into the source.

**Considered and rejected.**

- *Perspectives as tagged structs.* Reject; the `stable_when`
  function and the `serialize_as` annotation are intrinsic to
  the perspective concept. A tagged struct loses these.
- *Run-time-only perspective negotiation (versioning protocol).*
  Reject; the perspective is the contract, the source is the
  schema, no negotiation needed.

---

## 11. Region-based memory, no GC

(No grammar surface — this is semantic. Documented for posterity.)

**Commits to.** Each locus has a private memory region. C's
region is a sub-region of L's region. Allocation within a region
is locus-scoped; dissolution frees the region wholesale. No
garbage collector; no borrow checker.

**Why.** The framework's recursion property gives the hierarchy
for free. The lotus-lifecycle methods give the deterministic
free-points. The contract block gives the access discipline.
Together they give region-based memory management without the
inference problems that have historically made region-based MM
hard (Tofte-Talpin region inference is hard; here, the hierarchy
is explicit in the source).

Allocation strategy varies by projection class:

- `rich`: per-locus arena, low churn, freed on dissolution.
- `chunked`: per-locus arena with per-coordinatee sub-regions,
  freed on each coordinatee dissolution.
- `recognition`: pre-allocated fixed pool, no dynamic allocation
  in steady state.

The compiler picks the allocator based on the locus's projection
class.

**Considered and rejected.**

- *Garbage collection.* Reject; trading systems can't afford
  GC pauses, and the lotus structure obviates the need.
- *Rust-style ownership/borrow checker.* Reject; lotus
  hierarchical regions provide the ownership structure
  implicitly. Ownership tracking is unnecessary.
- *Reference counting.* Reject; same as GC but worse latency
  characteristics. Region dissolution is wholesale and
  deterministic.

---

## 12. Sum / product reductions in the grammar

`sum(expr)`, `prod(expr)` are language-native primary expressions
rather than stdlib functions.

**Why.** Closure assertions reference `sum` constantly. Putting
the reduction operators in the grammar means:

- They have well-defined precedence (higher than any binary op).
- The compiler knows they're aggregations and can reason about
  them in closure verification.
- Capacity computations (Σ over coordinatees) are syntactically
  unambiguous.

Other reductions (`min`, `max`, `count`) are stdlib. `sum` and
`prod` are special because they appear in framework-primitive
expressions.

---

## 13. Time and duration as language-native types

```
let t: time = `2026-05-08T12:00:00Z`;
let d: duration = 5s;
let timeout: duration = 100ms + 50us;
```

**Why.** Trading and any closure-test-with-band system needs
time and duration as first-class. Making them lexical avoids the
"is `5s` a string or a duration" ambiguity and prevents
unit-confusion bugs.

---

## 14. `decimal` as a primitive type

**Why.** Floating-point arithmetic is wrong for money. The
grease codebase already uses `shopspring/decimal`; lotus
inherits the commitment by making `decimal` a primitive
distinct from `float`. Decimal literals use the `d` suffix
(`1.50d`).

---

## 15. Generics and projection-class generics

`Rich<T>`, `Chunked<T>`, `Recognition<T>` are not stdlib types;
they're language-native generic constructors. The compiler
recognizes them and selects the appropriate allocator /
implementation based on which projection-class wrapper a value
carries.

**Commits to.** A user can write code parametric in projection
class:

```
fn process<P: ProjectionClass, T>(input: P<T>) -> P<Result> { ... }
```

The compiler picks the body specialization based on `P`'s
concrete instantiation.

**Why.** This is the "same source, different generated allocator"
commitment. Without language-level projection-class generics,
the user would have to write three nearly-identical functions
for `Rich<T>`, `Chunked<T>`, `Recognition<T>`. With them, one
function compiles to three.

---

## A. Locus instantiation and handles

(Added in v0.1.1, after the hello-world example surfaced this.)

A locus is instantiated using struct-literal syntax:

```
let h = HelloL { greeting: "hi" };
HelloL { };  // unbound; locus dissolves at statement end
```

The compiler distinguishes locus instantiation from struct
construction by what `HelloL` is declared as. The semantic
difference is significant:

- A struct literal allocates the value and assigns its fields.
- A locus instantiation allocates a *region* inside the
  enclosing locus's region, invokes `birth()` synchronously,
  and returns a typed handle.

`birth()` runs to completion before the instantiation expression
returns. If `birth()` panics, the runtime emits a failure event
that the parent's `on_failure` handles (or defaults to process
exit at the runtime root).

When the handle is bound (`let h = ...`), the locus lives until
`h` goes out of scope (at which point default `drain` and
`dissolve` are invoked).

When the handle is **unbound** (`HelloL { };` as a statement-
expression), the rule depends on whether the locus has any
**ongoing-work surface** beyond birth:

- **Ephemeral.** Only `birth` + `params` (or just `params`).
  The locus dissolves at the enclosing statement boundary.
  Hello-world's `HelloL { };` is the canonical case.
- **Long-lived.** Has `run`, *or* has bus subscriptions, *or*
  has mode declarations callable from outside, *or* otherwise
  exposes a surface that can be invoked post-birth. The locus
  becomes an *anonymous child of the enclosing scope*; its
  work proceeds until the enclosing scope dissolves it
  (typically via SIGINT-triggered drain cascade). Examples:
  01's `TickerL { n: 3 };` (run), 05's `EchoL { };` (bus
  subscriptions).

The rule generalizes: a locus is long-lived iff it can do
something *after* birth completes. If birth is all there is, it
dissolves at the statement that birthed it.

This means every function scope is itself an implicit locus
(see §D below). Anonymous children of a scope dissolve before
the scope returns — same rule as bound handles, just without
a name.

Multiple bindings of a handle are not yet specified. v0 punts;
expected: handles are move-only (Rust-shaped), so `let h2 = h;`
transfers ownership and `h` is no longer usable. Reference
counting is rejected (no GC, no ARC).

## B. The `self` keyword

(Added in v0.1.1.)

Inside a lifecycle block (`birth`, `accept`, `run`, `drain`,
`dissolve`), a mode block (`mode bulk`, etc.), or a closure
block, the keyword `self` refers to the enclosing locus.
`self.greeting` accesses the `greeting` param; `self.position`
accesses a contract-exposed field; etc.

Outside these contexts (in free `fn` bodies, in `const` decls,
in top-level expressions), `self` is a parse error.

Considered and rejected:

- *Implicit access to params by name.* `greeting` instead of
  `self.greeting` from inside a lifecycle. Reject; risks
  collision with locals; agent-first prefers explicit.
- *`this` instead of `self`.* Reject; `self` is more aligned
  with Rust / Python and avoids the C++/Java baggage.

## D. Function scope as implicit locus (and lifecycle methods are not)

(Added in v0.1.2 from 01-locus-with-run; refined in v0.1.3
from 02-parent-child.)

**Free `fn` functions have implicit loci.** Every `fn main()`,
`fn helper()`, etc. has an **implicit locus** at its scope.
Locally-bound handles and anonymous children of the function
body are children of this implicit locus. The function returns
when:

- The function body's last statement completes, AND
- All children of the function's implicit locus have dissolved.

For `fn main() { TickerL { ... }; }`, the implicit `main` locus
has one anonymous child (the ticker). `main` cannot return until
the ticker's `run()` has completed (or been drained). This makes
"main returns when its work is done" the natural semantics
without requiring explicit `wait()` or `join()` calls.

**Lifecycle methods do not have their own implicit locus.**
`birth`, `accept`, `run`, `drain`, `dissolve`, `on_failure` are
not regular functions — they run *as the locus*. Children
instantiated inside a lifecycle method attach to the enclosing
locus, not to a fresh implicit scope.

```
locus CoordinatorL {
    accept(g: GreeterL) {
        println(g.greeting);  // reads child's exposed state
    }
    run() {
        // GreeterL { ... } here: child of CoordinatorL,
        // NOT of run()'s scope. accept() will be invoked.
        GreeterL { greeting: "hi" };
    }
}
```

This distinction matters because the framework's "locus is the
unit of region" commitment means lifecycle methods can't have
their own region — they ARE the locus.

The implicit-function-locus model also underwrites SIGINT
handling: SIGINT triggers `drain()` on the runtime root locus
(which contains main); the drain cascades to main's implicit
locus, which cascades to its children. See F.4 (drain cascade).

## C. Default lifecycle methods

(Added in v0.1.1.)

When a locus omits a lifecycle keyword, the compiler supplies
a default:

- `birth()` default: no-op.
- `accept(c)` default: register the coordinatee in the locus's
  registry; no policy.
- `run()` default: empty steady-state — wait for messages or
  signals, dispatch to handlers as declared.
- `drain()` default: stop accepting new work; wait for
  in-flight to complete.
- `dissolve()` default: free the locus's region wholesale.
- `on_failure(c, err)` default: `bubble(err)`. The runtime
  root's default `on_failure` is process exit with stack
  trace.

A locus with only `params` and `birth` (like the hello-world
program) is fully valid; the compiler fills in the rest.

## E. `mut` keyword and immutable-by-default bindings

(Added in v0.1.2.)

Bindings are **immutable by default**. `let x = 0;` produces
an immutable binding; reassignment `x = 1;` is a compile-time
error. `let mut x = 0;` produces a mutable binding; reassignment
is permitted.

This matches Rust. Considered and rejected:

- *Mutable by default (Go).* Simpler surface; loses the
  discipline of marking mutation. Lotus's framework
  alignment prefers explicit-mutation marking.
- *No mutation; recursion only.* Pure but awkward; while-loops
  with counters are natural and the `mut` annotation is cheap.
- *Allow rebinding via shadowing.* Confusing; doesn't help with
  loops (inner `let i = i + 1` shadows in inner scope; outer
  loop never advances).

Mutability is a per-binding property, not a per-type property.
A `let mut x: int` is mutable; the `int` type itself is not
"mutable" or "immutable." This avoids the type-level mutability
machinery seen in some languages.

## F. Locked design commitments (v0.1.2 — from delivery plan)

These commitments came from the delivery-plan turn and lock in
specific decisions for the language's evolution toward v1.0:

### F.1 Optimize for runtime perf, never sacrifice behavior

When choosing between two options, the **faster runtime** wins —
provided correctness, framework discipline, and cyclic-closure
invariants hold. Compile-time perf is secondary; we accept
expensive compile passes (e.g., per-projection-class
monomorphization) if they produce faster runtime code.

This is an architectural directive, not a syntactic feature.
It informs every implementation decision: codegen strategies
prefer runtime speed, allocator design prefers runtime
determinism, scheduler design prefers runtime throughput.

### F.2 `ProjectionClass` as the "any-of-three" constraint

`ProjectionClass` is a built-in type-system primitive analogous
to Go's `any`. The constraint `<T: ProjectionClass>` requires
T to be `Rich`, `Chunked`, or `Recognition`. No full trait
system in v0; the constraint is built into the compiler.

This resolves the open question of "how does a generic bound
work without traits" without adding type-system surface. If
later versions need richer constraints (e.g., custom-defined
projection classes), the trait system can grow then.

### F.3 Per-arena defrag, no whole-program GC

Within a parent's arena, dissolved-coordinatee bookkeeping
slots are reclaimed via a **free-list** (for chunked-class
loci) or **periodic defrag** (if churn is high). The
reclamation is **per-arena**, **bounded**, and
**deterministic** — it cannot stop the world.

Coordinatee sub-regions remain pristine arenas freed wholesale
on dissolution; only the parent's *bookkeeping* about
coordinatees (registry slots, dispatch entries) needs free-list
reclamation. This keeps the no-GC commitment intact while
solving the "long-running parent with churning children leaks
bookkeeping" problem from `notes/open-questions.md`.

### F.4 `drain()` cascades depth-first

Calling `drain()` on a locus:

1. Recursively calls `drain()` on each child first (depth-first
   traversal).
2. Waits for all children to finish draining.
3. Drains itself.

There is no separate `drain_cascade()` syntax — `drain()` is
*always* cascading.

SIGINT triggers `drain()` on the runtime root locus, cascading
through the entire process tree. This gives clean shutdown for
free: from the user's perspective, "Ctrl-C and the program exits
cleanly" is the default.

### F.6 Lifecycle methods are not implicit loci

(See §D for full discussion.) Lifecycle methods (`birth`,
`accept`, `run`, `drain`, `dissolve`, `on_failure`) run *as*
the locus, not in their own scope. Children instantiated
inside a lifecycle method attach to the enclosing locus.

### F.7 `accept()` runs before child birth

When a child locus is instantiated inside a parent's lifecycle
method, the parent's `accept()` runs **before** the child's
region is allocated and `birth()` runs. This lets `accept()`
inspect the child's declared params + contract surface and
reject the child (panic, return error) before any resources
are committed.

If `accept()` rejects the child, the child instantiation
expression fails. The child's region is never allocated; its
`birth()` never runs.

If `accept()` accepts the child, the child's region is allocated
as a sub-region of the parent's, and `birth()` runs. The
instantiation expression returns the child handle.

For an unbound child instantiation, the child dissolves at the
enclosing statement boundary (per §A) if it has no `run`, or
becomes an anonymous child of the parent if it has `run`. The
parent's lifetime bounds the child's.

### F.8 Contract compatibility is type-checked

When a parent declares `consume X: T` and a child declares
`expose X: T`, the compiler verifies at compile time that the
child's expose-surface is a superset of the parent's
consume-surface, and that the types match. A mismatch (missing
field, wrong type, missing trait bound) is a compile-time
error.

This is the framework's contract-graded visibility commitment
expressed as a type rule. The full typing rule lives in
`spec/types.md` (Phase 0 deliverable).

### F.9 Collapse vs. explosion as dissolution modes

(Added in v0.1.4 from 03-closure-test.)

A locus that dissolves with all closures passing **collapses** —
clean dissolution from the parent's perspective. A locus that
dissolves with at least one closure failing at the dissolve
epoch **explodes** — the discrepancy is surfaced to the parent.

This is the framework's distinction between *structural failure*
(panic; the locus crashed) and *audit failure* (the locus
completed normally but its books didn't balance).

Mechanically:

1. When a closure fires (at its declared epoch boundary) and
   fails, the runtime flips an "exploded" flag on the locus.
   The locus continues running otherwise (it might still pass
   later epochs, but the explosion flag persists for that
   closure's failure).
2. At dissolve, if any closure-failure has been recorded, the
   parent's `on_failure(self, ClosureViolation { ... })` is
   invoked. The `ClosureViolation` carries: the closure name,
   the epoch at which it fired, the left/right values, the
   tolerance, and the diff.
3. If the parent's handler returns without re-raising
   (effectively absorbing), the locus is treated as collapsed —
   the parent has accepted the imbalance into its own books.
4. If the parent bubbles, the failure propagates up another
   level.
5. If no parent handler exists (e.g., the runtime root), the
   process exits non-zero with a structured violation report.

**Why this distinction matters.** Books at the parent's level
*must* balance, even if a child's didn't. The parent has to know
about the imbalance to absorb it correctly. Exploding the failure
upward — with typed information about exactly which closure broke
and by how much — gives the parent's handler the data it needs
to take corrective action. Conflating audit failure with
structural failure (everything is just "panic") loses this
information.

**Recovery primitives interact.** A parent's `on_failure` for a
ClosureViolation can:

- `ignore` (return without action) — treat as collapse
- `absorb(violation)` — fold the discrepancy into self's state
- `bubble(err)` — pass to grandparent
- `restart(child)` — re-instantiate the failed locus
  (semantics: the closure violation invalidates this child's
  state; restart gives a fresh attempt)

### F.10 Mode keywords as member names

(Added in v0.1.5 from 04-modes.)

The keywords `bulk`, `harmonic`, `resolution` are reserved at
the lexer level (per `tokens.md`). They appear at top-level in
the grammar as part of `mode_decl`. They also need to appear
post-`.` and post-`::` for member access — `self.bulk()` is the
natural mode-invocation syntax.

The grammar's `postfix_op` non-terminal admits a `member_name`
production that accepts either an `IDENTIFIER` or one of the
three mode keywords. This permits `self.bulk()` while keeping
`bulk` as a reserved word at top-level.

Considered and rejected:

- *Rename modes to non-keywords (e.g., `BulkMode`).* Reject;
  ruins framework-vocabulary alignment.
- *Use a separate mode-invocation syntax (`self::bulk()` or
  `mode_bulk(self)`).* Reject; less ergonomic than method-style.
- *Make modes not-callable-by-name; only invoked via runtime
  context.* Reject for v0; users want to be able to invoke
  specific modes for debugging and direct queries. Future
  versions might add an "implicit mode selection" mechanism on
  top.

This is the same approach Rust takes with raw identifiers
(though Rust uses `r#` prefix for any keyword; lotus permits
just the mode keywords post-dot, less surface area).

### F.11 `self.children` typing and lifecycle

(Added in v0.1.5 from 04-modes.)

A locus that declares `accept(c: ChildType)` exposes
`self.children` as a typed iterable — specifically, `[ChildType]`
(slice / list) of the currently-attached coordinatees.

Membership in `self.children`:

- A child enters `self.children` *after* `accept(c)` returns
  normally (i.e., accepts the child). Per F.7, `accept()` runs
  before child registration; if `accept()` rejects, the child
  is never added to `self.children`.
- A child exits `self.children` when it dissolves — clean
  collapse or explosion both remove the child from the
  collection. (For explosions, the parent's
  `on_failure(self, ClosureViolation)` is invoked alongside
  removal.)

Iteration with `for c in self.children` is O(N) for chunked-
class loci. The cost reflects the projection class:

- Rich (proj_rich): `self.children` is a small array; iteration
  is trivial.
- Chunked (proj_chunked): `self.children` is a chunked array
  with sub-region pointers; iteration is O(N) with one indirect
  load per child.
- Recognition (proj_recognition): `self.children` is a fixed
  pre-allocated pool with summary statistics — iteration may
  be discouraged by the compiler in favor of summary access
  (e.g., `self.children.count` vs `count = ...; for c in
  self.children { count = count + 1; }`).

**v0 limitation: single-accept-type only.** A locus with
`accept(c: ChildType)` for a single ChildType has well-typed
`self.children`. A locus that accepts multiple types
(`accept(c: TypeA)` and `accept(c: TypeB)` overloads — not
yet in grammar) would need `self.children` as a sum type
(`[TypeA | TypeB]`) and is deferred to a future version.

### F.12 `publish` builtin and bus-block scoping

(Added in v0.1.6 from 05-bus.)

The `publish(subject, msg)` builtin is in scope inside any
locus that declares a matching `publish SUBJECT of type T`
in its `bus { ... }` block. The compiler verifies at each call
site that the subject is declared and the type matches.

Out of scope in loci with no publish declarations. Calling
`publish` outside a locus body, or with an undeclared subject,
or with a mismatched type, is a compile-time error.

The runtime emits the message on whatever transport is bound
to the subject in the deployment configuration (per
`std::bus::Adapter` / runtime bus router). Author writes typed
declarations; deployment config selects transports;
typechecking happens at compile time; routing happens at
runtime.

### F.13 Bus subscription handler signature

(Added in v0.1.6 from 05-bus.)

A handler named in `subscribe SUBJECT as HANDLER of type T` is
a function defined elsewhere on the locus body, with signature:

```
fn HANDLER(payload: T) { ... }
```

It returns nothing (`-> ()`); to emit responses, the body calls
`publish(...)` explicitly. This gives the handler full control
over how many responses to emit, on which subjects, and under
what conditions — more flexible than the return-value-as-publish
pattern (which v0 doesn't have but a future version might add
for the simple single-response case).

### F.5 Mode-projections share the locus's arena

A locus may declare any subset of `mode bulk`, `mode harmonic`,
`mode resolution`. All declared modes operate on the same
underlying locus state and share the **same arena**. The
arena cascade (parent arena ⊃ child arena) gives mode-projection
sharing for free — there is no duplicate allocation, no copy,
no separate per-mode region.

This was implicit in §11 (region-based memory) but worth
making explicit: when you write three mode blocks, the runtime
generates three implementations that all read/write the same
arena. The compiler is responsible for verifying that the modes
don't conflict (e.g., resolution-mode mutating state that
bulk-mode also writes is a compile-time error if the writes
race).

## 16. What's deferred

The grammar in v0 does **not** specify:

- **Trait / interface system.** No `trait` keyword in v0
  (reserved). Generics are bound only by projection class for
  now. A full trait system is a future extension.
- **Effect / capability system.** Substrate-derivation anchor
  tracking is currently a runtime concern enforced by closure
  tests. A future version may move it into the type system as
  effect annotations.
- **Async / await.** Reserved keywords. Concurrency in v0 is
  expressed via the lifecycle state machine + bus interface;
  explicit async functions await a future spec.
- **Macros.** Reserved keyword `macro`. No grammar surface yet.
- **Pattern guards beyond `if`.** Match arms support `if` guards;
  more sophisticated pattern logic awaits a future version.
- **First-class modules.** `module IDENTIFIER { ... }` is in the
  grammar but module loading semantics are not specified.

Each of these is a known extension point. Closing them off in v0
keeps the spec tractable; opening them later is a non-breaking
addition.
