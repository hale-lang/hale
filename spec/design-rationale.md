# Design rationale

For each major syntactic construct in the Hale grammar, this
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

## Foundational axiom: types are for shapes, loci are for flow

Hale commits to a clean two-primitive split at the
declaration level:

- **`type`** — a static record. Pure shape. Fields, names,
  layout. Returnable by value, equal by value, no projection
  modes, no contracts, no birth/run/dissolve.
- **`locus`** — dynamic flow. Lifecycle (birth → accept → run
  → drain → dissolve), contracts (expose / consume), bus
  participation (publish / subscribe), projection (resolution
  / harmonic / bulk views over the same instance).

If a thing has lifecycle, it is a locus. If it is pure data,
it is a type. There is no third category at v0; the split is
clean.

**Recursive principle.** Loci are the fundamental building
block at every layer of an Hale program: an app is a locus;
a library namespace is a locus (empty `params { }`, only
methods — the namespace-lotus pattern); a long-running
service is a locus; a goroutine-equivalent is a locus; a bus
subscriber is a locus; an HTTP-handler is a locus; a cache /
pool / pipeline / queue is a locus. **Inside any locus,
behavior is itself a locus tower one layer down.** The
recursion bottoms at primitive operations (arithmetic, single
field reads, primitive calls). Everything above the floor is
loci nested in loci.

This axiom underlies most of the per-construct rationale that
follows: every section answers some shape of *"why is this
piece of locus syntax in the language?"* The answer, in every
case, is that flow needs lifecycle / contracts / projection /
recovery, and locus is the syntactic surface those four
attach to. Type declarations need none of that — they are
pure shape — so they have a separate, much simpler surface
(see also `spec/types.md`).

Full design note: `notes/hale-types-vs-loci.md`.

---

## 0. Surface language: Go-shaped

**Commits to.** Familiar syntax for engineers; braces for blocks;
semicolons as statement terminators; `let` for binding; `fn` for
functions.

**Why.** The first authors of programs in this language are
agents and humans collaborating, often already fluent in Go.
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

**Why.** The ancient texts' named-concept registry already
commits to: source uses names, renderer produces symbols. Hale
inherits this. Agent-first authorship benefits from no
symbol-input friction. Tooling is simpler.

**Considered and rejected.**

- *Allow Unicode operators (`Σ`, `≈`, `φ`).* Reject because:
  source becomes editor-dependent; tooling becomes harder; no
  semantic advantage; the renderer pipeline already exists.

---

## 2. Locus declaration

```
locus Fitter : tier 4, projection chunked {
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
pattern, Hale collapses the params-vs-state distinction. The
declared params are simultaneously:

1. *Birth-time defaults*: overridable at instantiation
   (`Aggregator { running_sum: 100 }`).
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
    subscribe "fitter.observation" as on_observation of type Observation;
    subscribe "fitter.kernel.updates" as on_kernel of type KernelUpdate;
    publish "fitter.drift" of type DriftReport;
}
```

**Commits to.** External typed message bus is a first-class
declarative surface. The grammar names subscriptions and
publications without committing to a specific bus implementation
(NATS, Unix sockets, shared memory, UDP multicast). The runtime
binds the bus block to the actual transport at link / startup
time.

**Why.** The running example needs UDP multicast input. Future
programs will need NATS, Kafka, or other transports. Declaring
the bus interface in source means the language can typecheck the
messages flowing in/out without committing to a specific runtime.
This also enables the perspective-shipping contract between
fitter and applier binaries — both compile from the same Hale
source, both have type-level agreement.

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

**Why.** Modes are a substrate primitive from the ancient texts —
the commitment that one kernel has three projections. Making them
syntactic means the compiler can:

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

**Why.** Cyclic-closure is a substrate primitive from the ancient
texts. Making it syntactic enables:

- Compile-time verification that the cycle exists (both sides of
  the `~~` reference defined values; the runtime accumulates
  both within the same scope).
- Runtime band-checking with named epochs.
- Recovery-event-aware accumulation (epoch resets / persists).

The `~~` operator is reserved for closure assertions only (per
precedence.md); using it elsewhere is a parse error.

**Cycle-existence rule.** A closure assertion must observe at
least one runtime-varying value. An assertion whose left and
right are both pure literals (no identifiers, no `self`, no
calls) is a compile error: the result is fixed at compile
time and the closure can't audit anything. This is the first
narrowing of the cycle-existence rule; the deeper version
(left and right reach a common producer through some causal
chain) requires the typechecker to track param-to-param
dataflow and lands in a later iteration.

Field references inside closure assertions resolve through
the strict locus surface (params + methods + `self.children`
+ `self.k_max`); a typoed field name like `self.greting` is
a compile error rather than a silent `Ty::Unknown` slip.

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
perspective Kernel<T> {
    params {
        scale_row: [decimal; 8];
        sigma_factor: decimal;
        regime_id: int;
    }
    stable_when {
        // Held to be stable when ≥3 perspectives have validated
        // and the closure tests at the producing locus pass.
        return num_validated >= 3 && closure_status == ok;
    }
    serialize_as KernelV1;
}
```

**Commits to.** A perspective is a serializable parameter bundle
within a shared compiled-in schema. Both producer (fitter) and
consumer (applier) compile from the same Hale source, so the
type *is* the contract; the bus carries only parameter values.

**Why.** This is the fitter/applier split: one process fits
parameters from observations; another applies them at high
frequency. The serialization format isn't a separate concern —
it's the perspective type. Compile-time type agreement between
binaries means no protocol-versioning handshake; the schema
version is the source-code version they both compile from.

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
for free. Hale's locus-lifecycle methods give the deterministic
free-points. The contract block gives the access discipline.
Together they give region-based memory management without the
inference problems that have historically made region-based MM
hard (Tofte-Talpin region inference is hard; here, the hierarchy
is explicit in the source).

The projection class is a **perspective-resolution
commitment** — a declaration of what observation granularity
the locus serves to perspectives one tower up. Storage
strategy is downstream of that commitment, not the commitment
itself:

- **`rich`** — fine-grained. Perspectives address individual
  children by name; each child carries its own state worth
  observing in detail. Storage consequence: per-locus arena
  per child, low churn, freed on dissolution.
- **`chunked`** — mid-grained. Perspectives operate over
  chunks or ranges; the parent commits to "I'll serve
  observations at chunk resolution." Storage consequence:
  per-locus arena with per-coordinatee sub-regions, freed on
  each coordinatee dissolution. The sub-region shape is what
  makes chunk-level observation cheap.
- **`recognition`** — aggregate. Perspectives operate over
  the population, not individuals — "represent as a curve,"
  "represent as a histogram," "represent as a count." At
  this resolution, individual child types stop mattering;
  the perspective consumes the population's structural shape.
  Storage consequence: pre-allocated fixed pool (or shared
  slab); cell stride derived from the accept-method type
  union; no dynamic allocation in steady state.

The compiler picks the allocator based on the locus's
declared resolution. The resolution choice is what's
load-bearing; the allocator is its implementation.

**Considered and rejected.**

- *Garbage collection.* Reject; latency-sensitive systems
  can't afford GC pauses, and Hale's locus structure
  obviates the need.
- *Rust-style ownership/borrow checker.* Reject; Hale's
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

**Why.** Floating-point arithmetic is wrong for money and any
other fixed-precision domain. Hale makes `decimal` a primitive
distinct from `float`, with semantics matching the
`shopspring/decimal` Go library. Decimal literals use the `d`
suffix (`1.50d`).

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
let h = Hello { greeting: "hi" };
Hello { };  // unbound; locus dissolves at statement end
```

The compiler distinguishes locus instantiation from struct
construction by what `Hello` is declared as. The semantic
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

When the handle is **unbound** (`Hello { };` as a statement-
expression), the rule depends on whether the locus has any
**ongoing-work surface** beyond birth:

- **Ephemeral.** Only `birth` + `params` (or just `params`).
  The locus dissolves at the enclosing statement boundary.
  Hello-world's `Hello { };` is the canonical case.
- **Long-lived.** Has `run`, *or* has bus subscriptions, *or*
  has mode declarations callable from outside, *or* otherwise
  exposes a surface that can be invoked post-birth. The locus
  becomes an *anonymous child of the enclosing scope*; its
  work proceeds until the enclosing scope dissolves it
  (typically via SIGINT-triggered drain cascade). Examples:
  01's `Ticker { n: 3 };` (run), 05's `Echo { };` (bus
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

For `fn main() { Ticker { ... }; }`, the implicit `main` locus
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
locus Coordinator {
    accept(g: Greeter) {
        println(g.greeting);  // reads child's exposed state
    }
    run() {
        // Greeter { ... } here: child of Coordinator,
        // NOT of run()'s scope. accept() will be invoked.
        Greeter { greeting: "hi" };
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

## F. Locked design commitments

The F-series are the language's locked design commitments —
each one is a stable navigation tag referenced from code
comments, spec sections, and CHANGELOG entries. Section
numbering is stable; new commitments append at the end with
fresh F.N tags. Ship-dates for each commitment live in
[`../CHANGELOG.md`](../CHANGELOG.md).

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

The three classes are a **perspective-resolution gradient**,
not three flavors of allocation strategy. Rich serves
fine-grained observation (named-child resolution); chunked
serves mid-grained observation (chunk-level resolution);
recognition serves aggregate observation (population-level
resolution — "curve," "histogram," "count"). The allocator
choice in each case is *downstream* of the resolution
commitment: rich's per-child arenas make per-child
observation cheap; chunked's parent-arena sub-regions make
chunk observation cheap; recognition's recpool makes
population observation cheap. See section 11 (region-based
memory) for the storage consequences.

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
[`types.md`](./types.md).

The check fires once per parent locus that declares any
`consume` entry. The parent's `accept(c: ChildType)`
declaration tells the compiler which child to verify
against. A parent that declares `consume` but no `accept` is
itself a compile-time error — the consume surface has nothing
to bind against.

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

- *Return without re-raising* — absorption; treated as
  collapse from the grandparent's perspective.
- `bubble(err)` — re-raise the violation; if no further
  handler catches it, the process exits non-zero with the
  formatted ClosureViolation.
- `restart(child)` — re-instantiate the failed locus
  (semantics: the closure violation invalidates this child's
  state; restart gives a fresh attempt). v0 parses; the
  full restart cycle requires the region allocator and
  scheduler.

The handler receives a structured ClosureViolation value with
fields:

- `locus` — name of the locus whose closure failed
- `closure` — name of the failed closure
- `left`, `right` — the assertion's two values at evaluation
- `tolerance` — the band
- `diff` — left − right when both are numeric

Field access uses the standard `err.closure`, `err.locus`,
etc. — the parser admits these reserved-word names in member
position because post-`.` is unambiguous.

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
(though Rust uses `r#` prefix for any keyword; Hale permits
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
- Recognition (proj_recognition): the `: projection
  recognition(cap=N, <sub_mode>)` annotation commits to a
  recpool sub-mode at the declaration site (`fixed_cell`,
  `shared_slab`, `spillover`, or `summary_only`; v1 ships the
  first two — see `spec/memory.md` § "Recognition sub-modes
  (v1.x-3)"). `self.children` iterates the pool's occupied
  cells; iteration may be discouraged by the compiler in
  favor of summary access (`self.children.count` vs the
  manual count-loop) once a workload exercises the surface.

**Summary access (2026-06-01).** `self.children.count` (Int) and
`self.children.is_empty` (Bool) are the shipped summary surface —
they read the accept'd-child tracker's live count directly (a load
of `__child_count`) instead of a hand-rolled `for c in
self.children` counter. Valid only inside a method of a locus that
`accept`s a child type (typecheck-enforced); `self.children`
itself remains a `for`-iterand only (not a value). Richer entity-
collection sugar (filter/map/broadcast, `first_where`) would need
a closure-value surface Hale doesn't yet have — deferred.

**v0 limitation: single-accept-type only.** A locus with
`accept(c: ChildType)` for a single ChildType has well-typed
`self.children`. A locus that accepts multiple types
(`accept(c: TypeA)` and `accept(c: TypeB)` overloads — not
yet in grammar) would need `self.children` as a sum type
(`[TypeA | TypeB]`) and is deferred to a future version.

### F.12 Bus send is the `<-` operator

(Added in v0.1.6 from 05-bus; revised in v0.1.8 — operator
shape replaces the original `publish()` builtin.)

A locus emits a message on a declared subject with the `<-`
operator at statement position:

```
"fitter.action" <- action_value;
```

The left side names a subject declared in the locus's
`bus { publish SUBJECT of type T; ... }`; the right side is
any expression of the declared payload type. The compiler
verifies at each call site that the subject is declared and
the type matches.

`<-` is in scope inside any locus that declares at least one
matching `publish` entry in its `bus` block. Calling `<-`
outside a locus body, or with an undeclared subject, or with
a mismatched payload type, is a compile-time error.

Send is a **statement, not an expression**. There is no
`x = ("subject" <- v)`; the construct produces no value. This
matches Erlang's `Pid ! Msg` shape — single-direction, no
return — and avoids the need for a value-of-send convention.

Subscribe is **declarative, not an operator**. A subscription
is set up by the `subscribe SUBJECT as HANDLER of type T`
clause inside the `bus` block, and the named handler runs as
a regular `fn` on the locus body whenever a message arrives.
There is no runtime `<- subject` (recv) operator; reception
is attached to the locus structurally, not invoked
dynamically.

The runtime emits the message on whatever transport is bound
to the subject in the deployment configuration (per
`std::bus::Adapter` / runtime bus router). Author writes typed
declarations; deployment config selects transports;
typechecking happens at compile time; routing happens at
runtime.

Considered and rejected:

- *Keep `publish(subject, msg)` as a builtin function.*
  Reject; "publish" reads as an action, and a builtin function
  for what is structurally a message-pass operation hides the
  shape. The operator makes the dataflow visible at a glance:
  *something flows from right to left into a named channel.*
- *Bidirectional `<-` (Go-shape, both send and receive).*
  Reject; subscription is structural in Hale (declared in
  `bus`, dispatched by the runtime), so a receive operator
  has no statement position to occupy. Hale's subscriptions
  are closer to Erlang's process mailbox + `receive` block
  than to Go's channel reads.
- *`!` for send (Erlang-shape).* Reject; `!` is taken by
  logical-not in C-family syntax, and we don't want to
  overload it.

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

### F.14 Three-way interface: locus + parent + contract

(Added in v0.1.7 as a structural design direction; sharpened
through conversation but not yet given dedicated syntax.)

The contract between a locus L at depth D and its parent at
depth D-1 is **three entities, not two**:

1. **L** owns its arena, its state, and its translation
   *implementations*.
2. **The parent at D-1** receives translated values through
   the contract; cannot see L's internal state directly.
3. **The contract** is itself first-class — declares the typed
   surface that crosses the D/D-1 boundary; mediates between
   implementation and observer.

The constraint: **any function injected by L into its arena
that satisfies a contract entry must return a type permitted
by the contract.** Translation implementations cannot route
around the contract; they bound their return shapes by it.

This is the interface / implementation split, framework-aligned:

- Contract = interface (declares typed surface)
- Translation = implementation (code producing contracted values
  from local state)
- Multiple implementations of the same interface field can
  coexist (e.g., bulk vs. chunked vs. recognition projections
  of the same value); the contract bounds them all
- Cost reflects projection class; arena cascade gives
  hierarchical access without crossing contracts

What it gives us:

- Translation functions are not a backdoor. Contract is the
  source of truth for visible flow.
- Multiple projections of the same contract field are
  first-class (`ProjectionClass = any` from F.2 gets a runtime
  substrate; ask for rich → call rich translation; ask for
  recognition → call recognition translation; same contract
  type returned in both).
- Substrate-derivation discipline propagates: parent sees only
  what translation produces from L's state; anchor-isolation is
  preserved by the typing rule.
- Vertical-only flow preserved at the query level: D-1 calls
  into L's arena via cascade; never lateral; never D-2 reaching
  past D-1 directly.

For v0, the commitment is the **typing rule** only: a function
satisfying a contract entry must return the contract's typed
surface. Multiple-implementations-per-field syntax (e.g.,
`@projection rich fn greeting_rich() -> string` annotations) is
deferred to a future version when an example forces it.

For now: a locus's `params` provide a default implementation
for each contract field (read the field directly). User-defined
fns can add additional implementations as long as they return
the contract's typed surface.

### F.15 Predefined type names are PascalCase, not keywords

(Added in v0.1.8 — restructured to remove the lexical
collision between primitive type names and stdlib namespace
identifiers.)

The built-in primitive types use **PascalCase** spellings:

```
Int  Uint  Float  Decimal  String  Bool  Time  Duration  Bytes
```

These names are emitted by the lexer as ordinary `Ident`
tokens. They are **not reserved words**. The parser recognizes
them by name in **type position only** (after `:`, in `->`
return type, in generic args, in `type` declarations). In
expression and namespace position, they are unreserved — so
the lowercase names `time::sleep`, `string::to_upper`, etc.
work as regular path expressions.

Why this matters. The original design (v0.1.0) had `int`,
`time`, `decimal`, etc. as reserved keywords. This created an
unavoidable collision: the stdlib wanted `time::sleep`,
`string::format`, but `time` and `string` were keywords and
could not appear in path position. Working around the
collision required either a `try_keyword_as_name` fallback
(parser hack with action-at-a-distance behavior) or renaming
the stdlib namespaces (`stdtime::sleep` — ugly).

The PascalCase move resolves it cleanly:

- `Int`, `String`, `Time`, `Duration`: type position only.
- `int`, `string`, `time`, `duration`: ordinary identifiers
  — free for stdlib namespaces, locals, fields.
- The lexer is one-pass and context-free; the parser does
  the type-vs-namespace split positionally.

This also matches the case-convention rule from
`spec/tokens.md` (PascalCase for type names) — type names
were the only place we had been *violating* that convention,
which was itself a smell.

Considered and rejected:

- *Rename the stdlib namespaces (`stdtime`, `stdstring`).*
  Reject; ugly and team-wide-unfamiliar. Go got it right
  with `time`, `strings`; we should match.
- *Treat primitive types as keywords with a contextual
  fallback.* Reject; the fallback worked for the parser but
  is fragile (any new place a keyword can appear becomes a
  new ad-hoc decision); also user-confusing (the same word
  is a keyword sometimes and not other times).
- *Lowercase type names as identifiers (Rust-shape `i32`,
  `f64`).* Reject; the team is more familiar with PascalCase
  (Go's `int` collides with stdlib namespace too, but Go
  punts and uses `time.Time` — a workaround we don't need
  if we just capitalize the type).

Shadowing a predefined type name with a user-defined type
(`type Int = ...`) is permitted by the grammar but produces
a compiler warning. The compiler does not block shadowing
(it's sometimes useful — e.g., a project-specific `Int`
wrapper) but flags it for readers.

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

### F.16 `self.k_max` is a built-in computed field

Any locus that declares the framework parameters as numeric
params (`B`, `c`, `sigma`, `phi`) exposes `self.k_max` as a
built-in field of type `Float`. The runtime computes:

```
self.k_max = B / [(1 − phi) · c + phi · sigma]
```

from current state values — the params are mutable, so the
bound floats with them. A locus that adjusts `phi` to
formalize its interface at runtime sees `k_max` move
correspondingly.

The typechecker recognizes `k_max` on every locus type as
`Ty::Float`; the runtime errors cleanly on missing params or
zero denominator rather than producing NaN or infinity.

Read shape is uniform across receivers: `self.k_max` inside a
locus method and `g.k_max` on a borrowed LocusRef `g` lower to
the same arithmetic — codegen extracts the field-load helper so
non-self receivers reach the same path. Same rule applies to
the F.27 synthetic `draining` flag.

This makes the framework's signature equation an executable
language primitive. A closure can audit against it directly:

```
closure within_capacity {
    self.children.length ~~ 0 within self.k_max;
}
```

Numerically: B=100, c=10, sigma=1, phi=0.5 yields k_max ≈ 18.18,
the small-k regime the ancient texts predict for mixed formality.

### F.17 Strict field-access checking + method types on locus values

Every Locus, Type, and Perspective value has a known surface
of fields and methods. The typechecker enforces it strictly:
when the receiver type is statically resolvable and the
named field doesn't exist on it, that's a compile error.
Permissive only on `Ty::Unknown` receivers (stdlib paths,
externally typed values, anything the bundle can't see).

Methods on a locus or perspective — free `fn` members, mode
declarations (`bulk` / `harmonic` / `resolution`), the
implicit `is_stable() -> Bool` on every perspective — appear
in the same field-lookup namespace as params. They resolve
to `Ty::Function` so `handle.method(args)` typechecks
through the standard call machinery.

This catches typo bugs that would otherwise be silent
runtime no-ops (`self.greting` returning `Ty::Unknown` and
sneaking through every check downstream).

### F.18 Match exhaustiveness checked at typecheck

A `match` whose arms don't cover all scrutinee values is a
compile error. v0 rules:

- An unguarded arm with `_` or a bare binding pattern is a
  catch-all → exhaustive.
- For `Bool` scrutinees: covered iff both `true` and `false`
  literal arms are present (unguarded).
- For everything else: a catch-all is required.

Permissive on `Ty::Unknown` scrutinees. The runtime previously
fell through silently when no arm matched, so a typoed match
became a hidden no-op. The check makes match safe by default.

When enum-variant patterns land (deferred — requires enum
typing surface), the rule extends to "every variant covered."

### F.20 Structural interfaces (Go-shaped)

```
interface Sink {
    fn write(s: String);
    fn line(s: String);
    fn newline();
}

locus StdoutSink {
    params { }
    fn write(s: String) { print(s); }
    fn line(s: String) { println(s); }
    fn newline() { println(""); }
}

fn render(sink: Sink) {
    sink.line("hello");
}

fn main() {
    let s = StdoutSink { };
    render(s);   // implicit: StdoutSink satisfies Sink
}
```

**Commits to.** A new top-level declaration form,
`interface Name { fn ...; fn ...; }`, declaring a named set of
method signatures. A locus *structurally* satisfies an interface
iff for every method in the interface, the locus has a method
with the same name, same arity, compatible param types, and a
compatible return type. **No `impl I for L` declaration** —
satisfaction is implicit (Go's shape, not Rust's). Interfaces
admit no default methods at v0; the body is signature-only.

**Why.** Multiple friction-log entries pointed at the same gap:
`std::text::Sink` had been a tagged-locus antipattern (one
locus with `dest: String` branching on every method) because
there was no interface mechanism; `std::log::StdoutSink` had
to couple through the bus for the same reason. Structural
interfaces let `StdoutSink` / `StringSink` / `FileSink`
coexist as separate loci with one shared surface, eliminating
the inner dispatch entirely. The Go-shape (structural, no
`impl`
declaration) is the simplest mechanism that solves the friction
without adding a trait/impl-coherence design surface that v0
isn't ready for.

The structural rule keeps the type-system surface small: no new
"this locus implements that interface" relationship to track in
metadata; the resolver knows about loci and interfaces
independently, and the typechecker walks the satisfaction-check
on demand at each call site where an interface-typed param meets
a concrete arg.

**Considered and rejected.**

- *Rust-style `impl Sink for StdoutSink { ... }`.* Reject for
  v0; adds a separate declaration that has to be kept in sync
  with both sides, and introduces orphan-rule / coherence
  questions. The Go shape (structural, implicit) is simpler.
- *Default methods.* Reject for v0; one obvious-shape interface
  is enough; defaults force decisions about override resolution.
  Add when a real workload needs them.
- *Interface inheritance / extension* (`interface SuperSink :
  Sink { ... }`). Reject for v0; same scope-creep argument.

**How it lowers.** `interface Name { fn ...; ... }` parses to
an `InterfaceDecl` and registers as `TopSymbol::Interface` in
the bundle scope. The typechecker enforces the structural-impl
rule at every call site where a fn declares an interface-typed
param; mismatches produce typed diagnostics (missing method,
arity, param type, return type).

`CodegenTy::Interface(name)` represents an interface value at
the codegen layer; LLVM-level layout is a single `ptr` to an
arena-allocated `{i8* data, i8* vtable}` fat-pointer struct
(uniform single-pointer ABI matching `LocusRef`). Per (locus,
interface) pair, `ensure_vtable` synthesizes a static
`[N x ptr]` global `__vt.<locus>.<iface>` holding the locus's
methods in interface-declaration order. At call sites where a
fn declares an interface-typed param, `coerce_to_interface`
arena-allocs the fat-pointer struct and stores
`data = locus_ptr`, `vtable = __vt.<locus>.<iface>`. Method
calls on an interface receiver lower in
`lower_iface_method_call`: load data + vtable, GEP to the
method's slot (decl-order index), `build_indirect_call`
through the m80 machinery with `data` as the implicit self
arg.

Interface values are storable in locus param fields,
`@form(vec)` cell elements, and free-fn return positions
— field-store / cell-set / return-coercion sites all insert
the locus → interface coercion automatically. For free-fn
returns, the m90 locus-instantiation routing extends to fire
when the fn declares `-> Interface(I)` and the instantiated
locus satisfies I, so the underlying locus lives in the
program-lifetime payload arena rather than the fn subregion;
`emit_return_value_deep_copy` deep-copies the 16-byte
fat-pointer struct into the caller's arena.

**G20 follow-up (2026-05-23):** interface elements inside fixed
arrays, array-repeat literals, and tuples now coerce at the
construction site. The codegen routes the RHS through
`lower_expr_into(expr, hint)` when a let-binding carries a
composite ascription, propagating the element type down so
per-position `coerce_to_interface` fires before the "mixes
element types" check would reject heterogeneous LocusRefs.
`Ty::assignable_from` extends recursively through Array, Tuple,
Fallible, and Projection composites so typecheck sees the
ascription as compatible with the inferred-from-leaves type.

Still deferred: locus-routing across nested return positions.
A fn declared `-> [Greeter; N]` instantiating loci inside its
return expression still aliases the fn's stack frame —
`emit_return_value_deep_copy`'s composite extension plus the
m90 routing extension to nested-position locus instantiations
need to fire together. Same gap governs tuple-of-`LocusRef`
escape today, where the pointers in the returned tuple alias
loci in the fn's stack frame and only happen to read correctly
because the freed memory hasn't been clobbered yet.

**m90 routing of nested-field children (2026-05-24).** A
returning locus whose `params` declares fields of locus type
(e.g., `locus Mapper { params { a: AssetMap; b: VenueMap;
c: ContractTypeMap; d: ContractMap; } }`, each an
`@form(hashmap)`) used to land its outer struct in the
payload arena via m90 while leaving each child's struct in
the returning fn's stack frame (via
`alloca_in_entry_with_nulled_arena`). After the fn returned,
the outer struct held dangling pointers to children; some
happened to read back stale-but-valid stack bytes, others
corrupted to garbage `len()` values. The pattern manifested
in a downstream persist-loader's `fn load_snapshot(path:
String) -> Mapper fallible(PersistError)` shape — first two
of four `@form(hashmap)` children survived; third and fourth
corrupted.

A new transient `instantiating_into_payload_arena: bool` flag
on the codegen Cx struct propagates the routing. The
params-init loop of an m90-routed locus sets it before each
field-value lowering; the nested `lower_locus_instantiation`
consumes it (via `mem::take`) and routes its own self_ptr
to the payload arena. The flag clears unconditionally after
each `lower_expr` so a primitive-valued field (Decimal
default, etc.) doesn't leak the routing to the next iter
or to the next top-level instantiation.

Coverage:
`crates/hale-codegen/tests/locus_fallible_return_multichild.rs`
covers fallible + non-fallible shapes with 3 and 4 distinct
`@form(hashmap)` children.

Coverage: `crates/hale-codegen/tests/interface_dispatch.rs`,
`interface_return.rs`, `interface_in_form_vec.rs`,
`interface_in_composites.rs`, `sink_polymorphism.rs`.

### F.21 Cascading-dimension interface (sketch)

A second interface form, *cascading-dimension*, paired with F.20
for the substrate-aware case: arena management + arbitrary
n-dim cascading flow (the `std::lotus::Grow` family — see
`docs/src/std/roadmap.md` "Future arc — Lotus harness for
n-dim growth"). Where F.20 is "any locus matching these methods
satisfies," F.21 is "this locus participates in the cascade
along these axes, with these arena-bound translation impls per
axis." Specific shape lives in F.14 (three-way interface +
translation impls) once a workload forces the design.

**v0 status:** sketched only; not implemented. F.20 ships first
because it solves the immediate friction (Sink). F.21 ships
when the n-dim growth arc has its first concrete demo (a high-rate ingest or
triangulator).

### F.19 Per-directory seed model

A directory of `.hl` files compiles as one **seed**: every
top-level decl (locus, type, free fn, perspective, const) in
any file in the directory is visible to every other file in
the same directory, in one shared scope. `hale build <dir>`,
`hale run <dir>`, and `hale check <dir>` accept directory
targets and bundle every `.hl` file under them; `hale build
<file.hl>` keeps working for one-file apps.

File order in the merged bundle is **alphabetical by filename**
(deterministic). Resolution is order-free — the typechecker
flattens all top-level decls into one bundle scope before name
lookup, so a fn declared in `z.hl` is callable from `a.hl`
without ceremony.

There is no per-file visibility (no `pub`, no Go-style
uppercase-exported convention). Anything declared at the top
level is visible to every file in the seed. Cross-seed
imports — one `apps/myapp` reaching into another `apps/lib` —
remain deferred (the `module` keyword is reserved with no
semantics; see `notes/open-questions.md` Q18).

**Why.** Single-file apps grew unwieldy quickly (one app hit
~2,300 lines before this milestone landed). The
single-file-app-monolith pattern was the canonical friction
case. The implementation cost
was small — the typechecker's `Bundle` already accepted multiple
programs, and `hale run` / `hale check` already handled
directory targets; only `hale build` had a hard "single .hl
file" check. Lifting it cost a CLI refactor plus a merge step.
The user-visible shape mirrors Go's per-package model.

**Considered and rejected.**

- *Per-file visibility (`pub` modifier).* Reject for v0; adds
  declaration-level decoration without a forcing function in
  any current app. Apps decompose by *concern* (one file per
  concern), not by visibility ladder. If a real cross-seed
  module system adds export controls later, that's where the
  feature lives.
- *Explicit per-file `import` directives within the same dir.*
  Reject; the per-package shared-scope model is what makes
  Go's per-dir packages ergonomic. Adding intra-directory
  imports would re-introduce the friction the milestone exists
  to remove.
- *Build-system manifest file (`hale.toml` listing files).*
  Reject for v0; the directory IS the manifest. Filesystem
  enumeration covers every multi-file shape we need; a manifest
  becomes interesting if we add cross-seed imports or external
  dependencies.

### F.22 Capacity-tuple as N-D allocator surface

Every locus declares its storage discipline as an N-tuple of
**capacity slots**. Slot 0 is the locus's own Arena (the v0 1D
baseline — wholesale-free at dissolve). Loci that need richer
discipline declare additional slots in a `capacity` block:

```
locus Foo {
    capacity {
        pool entries of Int;        // slot 1: cell-recycling of Int-sized
        heap registry of Command;   // slot 2: growable, individual free
    }
    params { ... }
}
```

**Domain framing — three capacity modes, not three allocator
strategies.** Each slot kind is a *commitment the locus makes
about its own state*, not a hidden implementation detail:

- **Arena** — *"I'm scratch — everything I touch dies with me."*
  Single bump arena, wholesale-free at dissolve. The locus
  retains nothing across its own lifetime boundary.
- **Pool of T** — *"I hold a bounded shape of recyclable state."*
  Fixed-size cells; values come and go but the population is
  bounded. Map-bucket recycling, fixed-shape registries,
  per-handler scratch frames.
- **Heap of T** — *"I hold growable state bounded by my own
  lifetime."* Individual cells alloc/free during the locus's
  life; wholesale teardown at dissolve. Growable Vec backing,
  rope chunk-lists, anything whose retained size isn't known
  at birth.

Slot 0 (Arena) is implicit because the simplest commitment —
"everything dies with me" — is the case where the locus *makes
no extra promise* about its state, and shouldn't have to write
it down.

**Slot ABI.** Each declared slot adds a field to the locus
struct (`__slot_<name>: ptr`) initialized at instantiation and
torn down at dissolve. Access is method-shaped via the
locus-scoped slot handle:

```
let cell = self.entries.acquire();   // pool: borrow a cell
self.entries.release(cell);          // pool: return cell
let p = self.registry.alloc();       // heap: alloc a Command
self.registry.free(p);               // heap: free a Command
```

Slot names are identifiers resolved at typecheck against the
current locus's declared slots — *not* stringly-typed
path-calls. Mirrors F.16's "synthetic self-field" precedent:
`self.entries` reads as a member of `self`, with member-typed
methods rather than a free-fn taking a name parameter.

**Slot lifetime (F.4 ordering).** Slots are created at
instantiation in declaration order, after slot 0 initializes,
before the locus's own field initializers run. Slots are
destroyed in *reverse* declaration order at dissolve, before
the slot-0 Arena itself dissolves. Matches F.4's
reverse-instantiation cascade rule for let-bound loci.

**Slot 0 parent-override is the existing chunked/recognition
machinery.** The v0 codegen already lets a chunked-class
parent allocate a child's slot 0 (Arena) as a sub-region of
its own — `lower_locus_instantiation` checks
`parent_accepts_us && parent.projection_class in {Chunked,
Recognition}` and routes through `lotus_arena_create_subregion`.
F.22 formalizes that machinery as "projection class governs
parent-override of slot 0." No new behavior at v0 — F.22
*names* the existing capability so future slot-1..N overrides
sit on a consistent vocabulary.

**Slot 1..N parent-override.** A
parent declares `capacity { pool entries of Int as_parent_for
Child; }` and any `Child` accepted by this parent gets its
matching slot pointer replaced with the parent's at accept
time. Generalizes the chunked-class sub-region hand-off to
all slot kinds. Runtime mechanic: every locus struct carries
a synthetic `__slot_borrowed_mask: i64` with one bit per slot;
the bit is set when the slot was borrowed, and the dissolve
pass skips destroy on borrowed slots so the parent retains
ownership of the underlying allocator. Codegen-side defensive
kind+elem_ty validation rejects mismatched borrows at the
swap site; `@form(vec)` slots cannot be borrowed (rejected
with a focused diag).

**Restrictions (v0).**

1. **Slot element type must be a value-shape**, not a
   `LocusRef`. Loci have lifecycle; cell recycling
   (Pool.release) would orphan the locus, and individual heap
   free would race with the locus's own dissolve. Loci go
   under `self.children` via `accept(c: Child)` per the
   existing membership model; slots are for *types*. The
   typechecker rejects `pool X of Some` and `heap Y of Some`
   with a diagnostic pointing at this rule.

2. **Slot pointers don't cross the bus.** A slot lives in the
   locus's own address space; the wire format has no shape
   for "give me a cell of your pool back." `synthesize_serializer`
   rejects any payload type whose field would resolve to a
   slot.

3. **`Pool of T` and `Heap of T` use the same `T`-as-cell
   convention.** Element size and alignment come from
   `T`'s LLVM struct layout. The slot itself stores
   `lotus_pool_t *` / `lotus_heap_t *`; the user-visible
   `acquire` / `alloc` return `*T`-shaped pointers.

**Naming note.** F.22's `pool` slot is distinct from
the recognition projection class's recpool (see
`spec/memory.md` § "Recognition sub-modes (v1.x-3)" —
`lotus_recpool_fixed_*` for `fixed_cell` sub-mode,
`lotus_recpool_slab_*` for `shared_slab`). Both are "pools
of cells" in the substrate sense, but the Recognition
recpool is part of projection-class semantics (slot 0
storage strategy for recognition-classed loci), whereas
F.22's pool slot is a user-declared slot at 1..N with
chunked-+-free-list backing and no projection-class
entanglement. The two systems may unify in v1.x once F.22
slots 1..N stabilize.

**Why.** The 1D collapse forced every long-lived data
structure into wholesale-free semantics. Growable types — Map,
Vec, ropes — leaked per-mutation until the enclosing locus
dissolved. Workarounds (fixed-cap parallel arrays as locus
params) burned per-instance footprint and obscured the
intent — `notes/hale-friction.md`
`dense-locus-storage-bloat` is the canonical writeup.
F.22 names the substrate distinction so the same locus can
hold "what dies with me" (Arena) and "what I recycle / grow
during my life" (Pool, Heap), in language the locus *writes
down* rather than smuggles in via runtime convention.
Operationalizes The Design's multi-dimensional capacity
principle at Hale's substrate.

**Considered and rejected.**

- *Per-allocation slot annotation* (`pool_alloc(...)` /
  `heap_alloc(...)` at every callsite). Too fine-grained;
  locus-level declaration matches the lotus principle that
  *flow lives at the locus boundary*, not at every callsite.
- *Stringly-typed slot access* (`std::alloc::pool_acquire("entries")`).
  Loses compile-time check that the slot exists on the
  current locus. The `self.entries.acquire()` form catches
  typos at typecheck and reads as native locus surface, not
  as a stdlib escape hatch.
- *Slot kinds as first-class types* (`Pool<T>` as a type you
  can store in a field). Conflates storage discipline with
  shape. F.22 keeps slots as declarations; the eventual
  Map / Vec stdlib types name which slot they bind to in
  their own declarations.
- *Untyped Heap* (`heap registry;` with no element type).
  Tempting for "I want raw bytes" but loses the codegen-side
  size/align inference that Pool gets for free, and produces
  weaker typecheck diagnostics. Keep the symmetry: every
  non-Arena slot names its cell type.
- *Heap as the default*. Would invert the v0 substrate's
  cheapest-default-fastest-path. Arena stays default; Heap
  is opt-in.
- *Slots hold LocusRef cells*. Loci have lifecycle; the
  existing `self.children` mechanism is the right surface
  for "this locus has these sub-loci." Slots are for values.

**Implementation pointers.**

- `crates/hale-codegen/runtime/lotus_arena.c` — adds
  `lotus_pool_*` and `lotus_heap_*` symbol families.
- `crates/hale-syntax/src/ast.rs` — `LocusMember::Capacity`
  variant carrying `Vec<CapacitySlot { name, kind, elem_ty }>`.
- `crates/hale-codegen/src/codegen.rs` —
  `declare_locus_struct` extends the struct layout with one
  field per declared slot; `lower_locus_instantiation`
  initializes each slot after slot 0; `flush_dissolve_frame`
  walks slots in reverse before slot-0 arena destroy.
- `crates/hale-codegen/src/codegen.rs` — `lower_expr` for
  `Expr::Field { Self, name }` checks the slot table before
  the field table, so `self.entries` resolves to a slot handle
  type rather than erroring.

**Pickup pointers for implementation.** This session locked
the spec; implementation tasks are tracked at the friction
plan level (`crates/hale-codegen/runtime/lotus_arena.c`
gets Pool + Heap primitives first; codegen surface follows).

### F.23 Int → Float widening at let/arg sites

Codegen inserts an implicit `sitofp` widening at the following
surfaces:

- **let-binding type ascription** — `let nf: Float = self.n;`
  with `self.n: Int` succeeds. The ascription tells the lowerer
  to coerce the RHS at the binding site.
- **fn-arg coercion** — when the parameter type is `Float` and
  the call-site argument type is `Int`, the argument widens at
  the call site. Same rule applies to user-declared fns and to
  stdlib path-calls (`std::math::sqrt(n)` with `n: Int` works
  without `2.0` literals).
- **binary-op promotion** — when
  exactly one side of an arithmetic or comparison binop is
  `Int` and the other is `Float`, the `Int` side widens and the
  op produces `Float` (or `Bool` for comparisons). Symmetric:
  either side can be the one that widens. Lets `0.5 + n`,
  `i < 0.5`, `3 * 1.5` typecheck without sprinkling `to_float`
  helpers at every mixed call site.
- **user-type field init** — assigning
  an `Int` value into a `Float`-typed struct field at literal-
  init time widens at the store. Lets a config bundle declare
  `timeout: Float` and accept an `Int` from the caller without
  per-field casts.

**Strictly one-way.** `Float → Int` narrowing remains explicit;
`Decimal` never participates in implicit cross-type conversion;
other numeric pairs (Int↔Decimal, Float↔Decimal) still reject.

**Why.** The friction-log entry `float-surface-gaps` in
`notes/hale-friction.md` documented the cost of forcing every
Float-heavy library to carry parallel Int+Float counters and
explicit conversion plumbing. `std::math::{sqrt, exp, log,
floor, ceil, pow}` shipped alongside the widening so the libm
primitives are ergonomic without per-callsite `to_float()`
ceremony.

**Considered and rejected.**

- *Symmetric widening (Float → Decimal, Int → Decimal).*
  Reject; the Decimal substrate is intentionally invariant to
  preserve financial-math precision guarantees. A `1.50d` value
  is not the same as `1.5` (Float), and silent promotion would
  break the F.3 commitment to type-level shape distinctions.
- *Narrowing (Float → Int) at let/arg sites.* Reject; the
  rounding semantics ambiguity (round / floor / ceil / truncate)
  is non-obvious enough that explicit operator surface is the
  right answer when it lands.

### F.24 Block-tail expression / `if` as expression

A block's last item may be an expression *without* a trailing
`;`. In expression position (let-RHS, fn-call argument, if-arm
body) that trailing expression is the block's **value**; in
statement position (function body, loop body, statement-form
`if` / `match` blocks) the tail is evaluated for side effects
and discarded — pre-Phase-2b code keeps its semantics
unchanged.

`if cond { ... } else { ... }` becomes dual-position:

- **Statement form** — no value; pre-2b semantics preserved.
- **Expression form** — the then- and else-arms' trailing
  expressions are phi-merged at the join basic block. The
  `else` branch is required (Unit-typed missing-else is
  rejected); arm trailing-expression types must match; arms
  may carry their own let-bindings before the tail.

`else if` chains carry through the value path —
`ElseBranch::ElseIf` recurses and the innermost arm's tail
feeds the phi at the outermost merge.

**Why.** The friction-log entry `if-needs-block-value` in
`notes/hale-friction.md` documented the canonical case: index
selection, default fallbacks, and ternary-ish expressions all
need a small conditional value, and the statement-only-if
workaround (`let mut x = i; if cond { x = j; }`) is verbose and
obscures intent.

The shape is form-completeness within the expression-evaluation
substrate: match-arm direct expressions
(`MatchArmBody::Expr(Expr)`) and function-body returns already
produced values; if-blocks were the lone holdout. Closes the
form-asymmetry — same shape as Rust, which is what the
friction-log entry asked for.

**Considered and rejected.**

- *Trailing expression as function return value
  (`fn f() -> Int { 42 }`).* Reject for v0; the user-visible
  surface is unchanged from pre-2b (typed fns require explicit
  `return`). Adding it would be a separate spec move, not
  load-bearing for the friction this entry resolves.
- *if-without-else as expression (Unit-typed).* Reject; the
  Unit-merge form silently passes a unit-typed block-value to
  the caller, which is rarely what the writer intended.
  Requiring `else` at the value-form makes the writer's
  intent (this is a value, not a side-effect) explicit.

### F.25 Cross-seed imports — vendored source, alias-required

An importer references a vendored library by literal path with
a required alias:

```hale
import "lib/moa" as moa;
import "../shared" as shared;
```

Cross-seed references read as `alias::Name`. The library is a
directory of `.hl` files (per-dir seed per F.19) copied into
the importer's source tree; v1 has no package manager, no
registry, no fetch, no versioning, no lockfile. The source IS
the dependency.

Resolution order is three-step (first hit wins):

1. `<importer-dir>/<path>.hl` — single-file lib.
2. `<importer-dir>/<path>/` — directory bundle.
3. `<workspace-root>/<path>/` — workspace fallback (workspace
   root = upward `hale.toml` / `Cargo.toml` search).

Library decls are auto-mangled at parse-time with prefix
`__lib_<lib_id>_<file_stem>_<name>` (where `<lib_id>` is a
stable, path-derived identifier for the lib — workspace-root-
relative when known, file-name fallback otherwise) and
registered into a per-build path-rename table parallel to the
static `STDLIB_PATH_RENAMES` table. The user never writes the
mangled form; `alias::Name` resolves through the table at
codegen. Two apps importing the same lib produce the same
`<lib_id>` regardless of which aliases they chose, so DTO seeds
on a bus have symbol-identical types across consumers.

**Qualified TypeExpr rewriting (brained F.1 / iris F.10).**
The codegen-side path-rename table resolves `alias::Name`
references at call-position lowering — but expression-position
resolution runs AFTER typecheck. Type-position uses of
`alias::Name` (struct fields, fn signatures, locus params,
`@form(hashmap)` cell types) need the rewrite to happen
BEFORE the typechecker sees them, or the
"unknown type" check fires against the un-rewritten qualified
path. A pre-typecheck pass in `hale_codegen::mangle::apply_qualified_path_renames`
walks every TypeExpr in the entry program and collapses
multi-segment paths whose first segment matches an import
alias to a single-segment path carrying the mangled name.
Lets the typechecker resolve qualified-type cell types the
same way it resolves bare ones.

Codegen-side, the single-segment branch of
`type_expr_to_codegen_ty` consults `pending_type_names` as
well as `user_types` so an `apply_qualified_path_renames`-
collapsed name forward-refs cleanly when the imported lib's
TypeDecl lands later in the merged items stream than the
entry-program decl that references it. Mirrors the
multi-segment branch's pre-existing forward-ref fallback.

**Why.** F.19 (per-directory seed model) fixed the
single-file-app-monolith friction at the intra-seed layer.
Cross-seed sharing remained the next gap: cross-app helper
patterns (tagged-accumulator, directory walks, JSON glue)
had no library home, so the std seed absorbed friction that
should have lived in user libraries.

F.25 opens user libraries as a first-class shape: user helpers
can graduate from copy-paste / std-seed-bloat to a vendored
shared lib.

**Vendor-the-source as the v1 commitment.** A real package
manager (registry + fetch + semver + lockfile) is several
months of work. The friction this milestone unblocks is "can
libraries exist at all" — not "can we deduplicate dependencies
across projects." Vendoring is how C, early Go, and many other
languages bootstrapped library ecosystems before package
managers existed. Hale's file-based dir-seed model is
well-suited to it. When a real workload surfaces friction that
vendoring causes (version skew, duplicate sources on disk,
manual update toil), that's the signal to design the package
manager — not before.

**Forcing-function alias.** Bare `import "<path>";` is a parse
error. The user names the namespace at the import site so a
downstream reader doesn't reconstruct it from the path. Mirrors
v1.x-3's no-default-sub-mode rule and v1.x-FORM-2's two-channel
rule — same discipline applied at a different layer.

**Considered and rejected.**

- *Implicit `lib/` search-prefix.* Reject; the resolver should
  not invent paths the user didn't write. `import "moa"`
  resolves `moa/`; `import "lib/moa"` resolves `lib/moa/`. No
  magic prefix.
- *`pub` / `export` keywords for fine-grained visibility.*
  Reject for v1; everything top-level in an imported seed is
  exported, matching the intra-seed visibility model. Adding
  fine-grained visibility doubles the design surface (every
  decl picks a modifier; users author it; typechecker enforces
  it). Add it when a workload demonstrates a real need.
- *Re-exports.* Reject; strict barrier. If lib A imports lib B,
  B is NOT visible to A's importers. Each importer declares
  its own dependencies. Re-exports require per-library scoped
  path-rename tables — significant additional machinery for a
  feature no current friction demands.
- *Transitive resolution of imports inside imported libs.*
  Reject for v1 alongside re-exports. The resolver follows
  imports only from the entry seed; imported libs may have
  `import` lines (they parse fine) but the build does not
  follow them. Future work may add per-library scoped resolution.
- *Package registry / lockfile / `$HALE_PATH`.* Reject for v1
  (see "Vendor-the-source" above). All deferred.
- *Hand-mangled `pub`-style prefixes on every library decl.*
  Reject; the auto-mangler does it at parse-time. Users would
  have to author `__MyLib*` prefixes by hand otherwise — exactly
  the shape std and moa carry today, but now done automatically.

**Implementation entry points.** See `spec/projects.md`
§ "Implementation entry points" for the file paths and primary
functions.

### F.27 Inline closure violation

A locus method body can escalate a value error into a structural
failure by declaring an assertion-less `epoch inline` closure and
firing it with the new `violate` statement:

```hale
locus DbConnection {
    params {
        host:       String = "127.0.0.1";
        port:       Int    = 5432;
        conn_fd:    Int    = -1;
        last_error: String = "";
    }
    bus { subscribe ExecuteQuery as on_query; publish QueryResult; }

    closure fatal_io { captures: last_error; epoch inline; }

    birth()    {
        self.conn_fd = std::io::tcp::connect(self.host, self.port)
            or self.handle_io(DbError { kind: "connect_failed", detail: err.kind });
    }
    dissolve() { if self.conn_fd >= 0 { std::io::tcp::close_fd(self.conn_fd); } }

    fn handle_io(e: DbError) -> Row {
        self.last_error = e.detail;
        if e.kind == "send_failed" || e.kind == "recv_empty" {
            violate fatal_io;
        }
        return Row { data: "" };
    }

    fn on_query(q: Query) {
        let r = send_query(self.conn_fd, q) or self.handle_io(err);
        if !self.draining { QueryResult <- r; }
    }
}
```

The pattern `let r = expr or self.handle_io(err);` — paired with
`handle_io` returning the success type on recoverable cases and
calling `violate` on fatal ones — is the *error-check function*:
one named method on the locus owns both the audit-log update
(`self.last_error = ...`) and the recovery / escalation choice.
Channels stay separated (per the two-channel rule, F.22-era);
the conversion from value error to structural failure happens at
exactly one named site.

The parent's `on_failure(c: DbConnection, err: ClosureViolation)`
body reads the audit-log state through the child handle —
`c.last_error`, `c.conn_fd`, etc. — not through the
`ClosureViolation` payload. Because `violate` is divergent, the
child's locus state is *frozen* at the violate moment (the
remainder of the method body doesn't execute), so the child
handle's field reads return exactly the state at the
escalation point. The `captures:` clause is a declarative audit
hint pointing the reader at the structurally relevant state;
the portable access path is via the child handle.

**Commits to.**

1. **A new closure shape.** `closure` declarations may omit the
   assertion when an `epoch inline` clause is present. The body
   then consists only of optional clauses (`captures:`,
   `persists_through(...)`, `resets_on(...)`, `epoch inline`).
   Assertion-bearing closures cannot pair with `epoch inline`.

2. **`captures:` clause.** A `captures: f1, f2, ... ;` clause
   names locus state fields that are *structurally relevant* at
   the violate point — a declarative audit-log hint. Field names
   must reference declared locus params. The parent's
   `on_failure(c, err)` body reads each captured field through
   the child handle (`c.f1`, `c.f2`) — because `violate` is
   divergent, the child's state is frozen at the violate
   moment, so the handle reads return the snapshot the captures
   clause names. Under `hale run` the interpreter additionally
   materializes each captured field on the `ClosureViolation`
   value as a convenience (`err.f1` works); under `hale build`
   only the child-handle path is wired (the `ClosureViolation`
   carries `err.locus` + `err.closure` only). The portable
   access pattern, recommended in both runtimes, is the child-
   handle path.

3. **`epoch inline`.** A new variant alongside `tick`,
   `duration(d)`, `dissolve`, `birth`, and `explicit`. Inline
   closures do *not* fire automatically at any epoch boundary;
   they fire only via `violate <name>;`.

4. **`violate` statement.** Statement-level, recovery-primitive-
   shaped. Form: `violate IDENT;` or `violate IDENT with EXPR;`.
   Divergent (typechecker treats as `Never`, same as `fail` in
   fallible fns and `bubble` in `on_failure`). Resolves the
   identifier to a closure declared on the enclosing locus; the
   target closure must be `epoch inline`.

5. **Inline fires initiate drain.** Auto-epoch closures keep
   F.9's behavior: flip the exploded flag, locus keeps running,
   parent's `on_failure` fires at natural dissolve. Inline
   closures do that *and* request drain — at the next
   cooperative yield point the runtime transitions the locus to
   the draining state, cascading through children depth-first as
   usual. The exploded flag is set identically; the only
   difference from auto-epoch is that the locus stops accepting
   new work instead of completing its current epoch.

6. **`self.draining` reads as a synthetic Bool.** Locus method
   bodies may read `self.draining` to check whether the locus
   has entered the winding-down state. This lets the canonical
   pattern above suppress a downstream send after escalation:

   ```hale
   if !self.draining { QueryResult <- r; }
   ```

**Rejection contexts.** `violate` is rejected:

- In free fn bodies (no `self` to anchor the closure name).
- In `on_failure` body (use `bubble(err)`; `on_failure` is the
  parent-side handler for child failures, and re-firing a self-
  closure from there would mix channels).

Allowed everywhere else that has `self`: named locus method
bodies, bus-handler methods (`subscribe X as foo` → `fn foo`),
`run()`, lifecycle methods (`birth()`, `dissolve()`, `drain()`),
mode-method bodies. The same body shape gets the same primitive.

**Why.** The two-channel rule (locus methods cannot declare
`fallible(E)`) keeps recovery paths legible: parents
handle structural failures (`on_failure`), free fns and
`@form`-synthesized methods handle value errors (`fallible`).
But the bridge between them — converting a caught value error
into a structural failure inside a locus method — had no clean
primitive. The workaround was a `should_exit: Bool` flag plus a
`while !should_exit { yield; }` loop in `run()` plus a separate
`last_error` field for diagnostics: three pieces of state doing
what should be one named call.

`violate` collapses the three-piece workaround to one line by
naming the closure being violated. The closure name is the
audit-log handle (`ClosureViolation.closure`) the parent
receives, and `captures:` makes the diagnostic payload
declarative instead of folded into a free-form `Error` payload.

**Considered and rejected.**

- *A `:fatal` modifier on auto-epoch closures.* Reject; would
  conflate the audit-fire path with the inline-fire path.
  Auto-epoch closures fire at epoch boundaries; the `:fatal`
  variant would never fire automatically (since `violate` is
  the only producer). A separate `epoch inline` is clearer
  about which closures are pull-only.
- *Statement-level `terminate;` without a closure name.* Reject;
  the closure name is the structural-failure label the parent
  matches on (`match err { ClosureViolation { closure: "fatal_io",
  ... } -> ... }`). Anonymous terminate would lose the audit
  shape that F.9 established.
- *Allow `violate` from a fallible free fn.* Reject for v1; the
  value channel already has `fail`. Mixing channels in free fns
  has no demonstrated workload. (A free fn called from a locus
  method body still can't violate transitively — `violate` is
  declaration-site-only, lexically inside a locus method.)
- *Make assertions still mandatory and just ignored for inline.*
  Reject; an assertion that never fires is dead syntax. Better
  to make the assertion optional and gate it on the absence of
  `epoch inline`.
- *Pass the captures snapshot via `with <expr>` instead of a
  declarative clause.* Reject for the canonical case; the
  capture set is structural (a list of field names declared at
  closure-decl time), not value-dependent. `with <expr>` remains
  available as an additional payload for cases that need it; if
  both are present, the parent sees both as fields on the
  `ClosureViolation` payload.

**Implementation entry points.**

- `crates/hale-syntax/` — lexer (`violate` / `inline` /
  `captures` as contextual keywords), AST (`Stmt::Violate`,
  `ClosureClause::Captures`, `EpochSpec::Inline`,
  `ClosureDecl.assertion: Option<ClosureAssertion>`), parser
  arms.
- `crates/hale-types/` — typecheck divergence,
  closure-name resolution, rejection-context enforcement, `with
  <expr>` payload typing.
- `crates/hale-codegen/` — synthetic `__drain_requested: i64`
  field, `self.draining` codegen, ClosureViolation synthesis at
  the `violate` site with captures snapshot, drain initiation at
  the next cooperative yield.
- `crates/hale-runtime/` — interpreter parity for the new
  statement and assertion-less closures.

### F.28 BytesBuilder is a locus, not a primitive type

**Commitment.** The bytes-accumulator surface
(`std::bytes::builder_*`) is no longer a family of free
functions over an opaque `Bytes` handle. It's now a stdlib
locus, `std::bytes::BytesBuilder`, with method dispatch
(`b.append`, `b.len`, `b.snapshot`, `b.shift_front`, `b.clear`,
`b.finish`) and lifecycle (birth allocates the underlying
malloc-backed buffer; dissolve frees it at scope exit). The
prior free-fn surface is removed; the C primitives stay as
the locus's method-body externs.

**The bug being closed.** The builder header
(`lotus_str_builder_t { cap, len, buf* }` — a separately-
malloc'd buffer behind an internal pointer) and a `Bytes` blob
(`[i64 len][u8 data]` — single contiguous allocation) are
genuinely incompatible ABIs that cannot be unified without
giving up stable handles (the body has to be relocatable; the
Bytes blob layout forbids that). The original surface returned
a builder as `Bytes` — the only Hale type wide enough to
carry a pointer — under the convention "you should treat it as
opaque." When that convention slipped (a builder passed to
`std::bytes::at(b, i)` reading the cap field as the length),
the runtime silently misread and ran off the heap. A
downstream workload hit this with RSS exploding 2.6 GB in 5
seconds. The mechanical fix is to make builder and Bytes
statically distinct.

**Why a locus, not a parallel primitive type.** Hale's
foundational axiom says types are pure data (no flow); loci
are flow with invariants. The bytes builder is textbook flow
— allocate → append (auto-grow) → snapshot → free — with
invariants (`cap >= len`, `buf` is a valid malloc region,
`len` is the count of valid bytes). It belongs on a locus.
Introducing it as a primitive type would have closed the
typecheck footgun, but would also have added a magic
non-locus thing with lifecycle to the language — a special
case that contradicts I4 (every named structural thing is in
exactly one locus tower). The locus shape is the principled
answer; the type-discrimination win is a free byproduct.

**What the user sees.**

```hale
fn pump_frames(sock: Int) {
    let buf = std::bytes::BytesBuilder { initial_cap: 4096 };
    loop {
        let n = std::io::tcp::recv_into(sock, buf, 4096);
        if n <= 0 { break; }
        // ... peel via buf.len() / buf.shift_front(consumed)
    }
    // buf dissolves here → malloc freed, no explicit cleanup
}
```

The typechecker rejects `std::bytes::at(buf, 0)` inside that
loop — `at` takes `Bytes`, `buf` is a `__StdBytesBytesBuilder`
locus reference. Same for `len(buf)`, `slice(buf, ...)`,
anywhere a `Bytes` is expected. The discipline is mechanical.

**Discarded alternatives.**

- *Keep the free-fn surface and add a runtime tag bit to
  `lotus_bytes_*` for transparent dispatch on builder
  handles.* Rejected: every `Bytes` access pays a branch
  forever, and the static type still doesn't distinguish the
  two — readers have to consult runtime tags. Worse for
  correctness *and* worse for perf.
- *Keep the free-fn surface and add a parallel
  `lotus_bytes_builder_*` accessor family that consumers must
  use deliberately.* Rejected: discipline-only enforcement;
  same class of footgun every time a new author touches the
  code. The whole point is mechanical safety.
- *Make `BytesBuilder` a parametric type, not a locus.*
  Rejected per the foundational axiom — flow doesn't live on
  types.

**Failure routing (F.27).** `append()` routes realloc-NULL
failure through `violate alloc_failed`. The locus declares
`closure alloc_failed { captures: initial_cap; epoch inline; }`;
`append` checks the C primitive's `int64_t` status return
(1=ok, 0=fail), and routes through `violate` on 0. Owners of
the BytesBuilder bind an
`on_failure` policy to handle the violation (restart / drain
/ bubble); an unhandled violation bubbles past `main` and
exits the process non-zero with the captured payload on
stderr.

**F.27 v2: `birth_check` synthesis hook.** A
declarative invariant check that runs AFTER the locus's birth()
body completes (and after birth-epoch closures fire), at the
well-defined point where every field has its declared
post-birth value:

    locus L {
        params { x: Int = 0; }
        closure invariant_broken { captures: x; epoch inline; }
        birth() { /* set up state */ }
        birth_check { self.x < 0 } -> violate invariant_broken;
    }

If the boolean cond evaluates to true, the named closure
violates with the locus's fully-constructed state. Multiple
birth_check clauses on a locus evaluate in declaration order;
the first to fire short-circuits the rest (subsequent checks
sit in unreachable basic blocks after the violate's
terminator).

Why a synthesis hook and not just `violate` inside birth(): the
v1 form lifted the codegen restriction on
`violate` in lifecycle bodies, but `violate alloc_failed` fired
mid-birth leaves the locus PARTIALLY CONSTRUCTED — some fields
set, others at defaults — when the on_failure handler reads
the closure's captures. For BytesBuilder that worked by luck
(only `initial_cap` is captured, and it's set before birth
runs), but for any locus with multi-step birth and field
inter-dependencies the partial-construction case produces
undefined intermediate state in the violation payload. The
birth_check form sidesteps this: the body runs to completion,
the check fires at a well-defined point, the violation's
captures read coherent state.

Codegen. `emit_birth_check` (see `crates/hale-codegen/src/codegen.rs`)
emits the cond + violate routing INLINE at the instantiation
site — NOT through the standard Stmt::Violate codegen. Standard
violate's divergent return targets the CALLER's LLVM function
(e.g. Parent.run), which is wrong for birth_check: an absorbed
violation should let the caller keep running after the failing
instantiation expression, not return from the caller's whole
fn body. The inline emission writes `__drain_requested = 1`,
allocates the ClosureViolation, branches on parent_on_failure
(indirect-call vs dprintf+exit), then unconditionally branches
to a continuation block so the caller proceeds normally.

BytesBuilder migrated. `std::bytes::BytesBuilder` now uses the
birth_check form for the null-handle case
(`birth_check { self.handle == 0 } -> violate alloc_failed`),
replacing the earlier inline `if self.handle == 0 { violate
alloc_failed; }` in birth body.

**F.27 extension (superseded by F.27 v2): `violate`
in lifecycle bodies.** The codegen restriction on `violate` was
lifted for lifecycle blocks (birth / drain / dissolve / accept
/ run); they now participate in the same divergent-return +
parent-on_failure routing as regular method bodies. The
original spec rationale ("only fn-bodies can violate") was an
implementation simplification, not a structural requirement —
lifecycle bodies are void-returning fn contexts at the codegen
level, and the violate machinery's `build_return(None)` path
handles them identically once `current_user_fn_ret` is set to
`Some(None)` at lifecycle entry. Both forms remain supported in
v1.x; new locus designs should prefer the birth_check synthesis
hook for construction-time invariants (the partial-
construction hazard described above). The accepted-child
dissolve trade-off (`parent_accepts_us` skips dissolve bodies
entirely) is unchanged; dissolve violate is observable only
along the F.29 locus-field cascade path where
`emit_locus_field_dissolves` does fire the inner's dissolve.

**Caveats at v1.**

- `snapshot()` / `finish()` payload-arena alloc failures route
  through `violate alloc_failed`. The C primitives use a
  dedicated alloc-fail sentinel pointer on every failure path;
  the locus method body discriminates via
  `std::bytes::__is_alloc_fail` before returning, closing the
  prior "empty-on-success aliases empty-on-fail" hazard.

**Implementation entry points.**

- `crates/hale-codegen/runtime/stdlib/bytes_builder.hl` —
  the locus definition (params, birth, dissolve, methods).
- `crates/hale-codegen/src/codegen.rs` —
  `STDLIB_PATH_RENAMES` rewrites `std::bytes::BytesBuilder` →
  `__StdBytesBytesBuilder`; the C-primitive bridges live
  behind `std::bytes::builder::__*` paths; `recv_into` family
  extracts the locus's internal `handle` field via
  struct-GEP + load + inttoptr at the C-call boundary.
- `crates/hale-codegen/runtime/lotus_arena.c` —
  `lotus_bytes_builder_new` takes `int64_t initial_cap` (was
  zero-arg); other primitives unchanged.
- `crates/hale-runtime/src/builtins.rs` — interpreter
  parity moved to the `std::bytes::builder::__*` dispatch
  paths; the `__new` impl accepts the new `initial_cap` arg
  (currently ignored since the interpreter's `Vec` backing
  auto-grows).

**Phase-2 (1): zero-copy `view()`.** Added
`b.view() -> Bytes` returning a non-owning Bytes pointer that
aliases the builder's buffer. Pond/websocket's recv loop had a
residual leak after Phase 1 — `rx_buf.snapshot()` per peel
attempt to materialize a Bytes value for `parse_frame` to read
via `std::bytes::at` / `len`. `view()` is the same value without
the allocation.

**Memory layout (changed).** Prior layout was
`{cap, len, buf*}` with the data area separately malloc'd.
Reshaped to `{cap, buf}` where `buf` points at the data area
of a single `[i64 len][u8 data[cap]]` malloc'd region — the
8-byte length prefix lives inline immediately before the data.
`view()` returns `buf - 8`, which IS a valid Bytes pointer:
`lotus_bytes_len` / `lotus_bytes_at` / `lotus_bytes_data` all
just work. Every mutation (`append`, `shift_front`, `clear`,
`finish`) updates the inline prefix to match. The cost is one
extra pointer dereference per len access (negligible at our
scale); the win is true zero-copy view + zero-allocation peel.

**Lifetime contract.** A `view()` Bytes is valid until the
next mutation on the source builder. The aliasing property
means a captured view sees stale len if the builder grew /
shrank after capture, and a view captured after a mutation
sees the new state. At v1 (no borrow checker) this is
documented-and-trusted — same shape as the rest of Hale's
lifetime story. The canonical pattern is "view immediately,
read immediately, don't store across mutations." The
`snapshot()` path remains for cases that need a stable
Bytes (consumer holds across producer mutations).

This obviates Phase-2 ask (3a) — proposed `b.at(i)` /
`b.slice(lo, hi)` methods on BytesBuilder. With `view()`,
the existing `std::bytes::at(b.view(), i)` /
`std::bytes::slice(b.view(), lo, hi)` work directly, no new
method dispatch needed. The BytesBuilder surface stays
minimal.

Tests: `crates/hale-codegen/tests/bytes_builder_view.rs` —
5 cases covering current-contents read, aliasing across
appends, slice composition, shift_front reflection, and
clear-then-view.

### F.29 Locus-typed param fields with lifecycle cascade

**Commitment.** A locus's `params` block may declare a field
whose type is another locus (user-defined or stdlib).
Construction defaults via a locus literal:

```hale
locus WsClient {
    params {
        sock: Int = -1;
        rx_buf:   std::bytes::BytesBuilder
                = std::bytes::BytesBuilder { initial_cap: 4096 };
        last_msg: std::bytes::BytesBuilder
                = std::bytes::BytesBuilder { initial_cap: 4096 };
    }
    fn run() {
        // self.rx_buf is the held builder; method dispatch
        // works through self.<field>.<method>(...) — same
        // shape as any other locus field.
    }
}
```

The owning locus's lifecycle cascades into the child:
- **Birth.** Default-init evaluates the child literal,
  including running the child's `birth()` body. The child's
  state lives past the construction expression: child
  instantiation routes through a `parent_owns_via_field` path
  analogous to the existing `parent_accepts_us` path, so the
  child doesn't dissolve eagerly at the end of its literal.
- **Dissolve.** The parent's dissolve dispatch fires the child's
  full `drain → __dissolve_closures → dissolve → arena_destroy`
  sequence per `LocusRef`-typed field, in field-declaration order,
  **after** the parent's user dissolve body and **before** the
  parent's `arena_destroy`. The "user body first" ordering means
  the parent's dissolve body can still legitimately touch its
  child fields. Mirrors the depth-first cascade discipline of
  F.4 (drain) and F.9 (collapse vs explosion).

**Motivation.** A downstream Phase-2 ask — "in-place storage for
locus Bytes/String fields" — is what this commitment closes.
Producer locus owns the storage (a `BytesBuilder` field, in
practice); consumer reads zero-copy via the contract using
`b.view()` (F.28 Phase-2 (1)); each frame the buffer is reused
in place via `b.clear()` + `b.append(...)`. No per-message
allocation against `g_bus_payload_arena`. The "vertical
contract / DMA" idiom Hale's F.14 design was reaching for.

Equally important: this works for non-stdlib loci too. Any
user-defined service locus can compose held sub-loci via param
fields with cascade — `Cache`, `RetryPolicy`, `RateLimiter`,
whatever the application carves up. The locus axiom (everything
named and structural is a locus) gets a working composition
story.

**Discarded alternatives.**

- *Field annotation `@inplace` on Bytes/String fields.*
  Considered (one of three options in a downstream handoff). Rejected
  because the underlying need is "a locus owns growing buffer
  storage with its own lifecycle" — that's a locus, not an
  annotation on a primitive. Plus annotations don't compose with
  the rest of the locus story (no `@inplace BytesBuilder`).
- *Storage-class heuristic.* Considered (option two). Rejected
  for the same reason — silent magic where explicit composition
  is available.
- *Contract-side `expose ... @reuse`.* Considered (option three).
  Rejected: the field-owner-vs-consumer split is real, but it's
  cleanly modeled by the producer holding a `BytesBuilder` field
  and the consumer reading through `b.view()` over F.14 — no new
  contract annotation needed.

**Caveats at v1.**

- **Cascade only fires when parent dissolves through one of the
  tracked paths** — the ephemeral dispatch in
  `lower_locus_instantiation` or the `flush_dissolve_frame`
  deferred path. Both cover normal control flow (statement
  literals, let-binding scope exits, fn returns). The pinned tail
  inherits the existing "v1 trade-off: child's dissolve body
  skipped" behavior. `accept`'d children no longer do: they are
  reclaimed on their OWN completion — see "Per-child reclamation
  of accept'd children" below.
- **Diamond / cycle composition is undefined.** A locus that
  appears in two field slots of the same parent dissolves
  twice. The locus literal model already forbids reuse (each
  `Locus { }` evaluates to a fresh instance), so diamond shapes
  are rare in practice; document-and-trust at v1.
- **No depth-first drain cascade yet.** Drain still runs only
  on the outer; inner field drains are not invoked separately.
  Adding them is straightforward (mirror the dissolve cascade
  in the drain phase) and warranted when a held sub-locus
  declares its own drain — none of the stdlib loci do today.

**Implementation entry points.**

- `crates/hale-codegen/src/codegen.rs`:
  - `Codegen.instantiating_for_parent_field: bool` — flag set
    by the param-init loop before each `lower_expr` call.
  - `lower_locus_instantiation` consumes the flag via
    `mem::take` and adds `parent_owns_via_field` to the
    `defer` set; the dispatch branch suppresses eager
    dissolve at the child's instantiation site.
  - `emit_locus_field_dissolves` — new helper that iterates
    `LocusRef`-typed param fields in declaration order,
    loading each child pointer and running its
    drain/closures/dissolve/arena_destroy sequence.
  - Called from both the ephemeral dispatch in
    `lower_locus_instantiation` and the deferred-flush path
    in `flush_dissolve_frame_kind`.

Tests: `crates/hale-codegen/tests/locus_field_cascade.rs` —
3 cases: method dispatch through a field-held `BytesBuilder`,
ordering of the cascade (outer body → inner cascade → outer
arena_destroy), and a 100k-iteration construct-destroy loop
exercising the cascade path under load.

### Per-child reclamation of accept'd children (2026-05-30)

**Problem.** An `accept`'d child lives in (a subregion of) its
parent's arena and, at v1, was reclaimed only when the *parent*
dissolved. For a daemon — a server whose top locus loops forever
and never dissolves — that means *never*: a parent that accepts
one child per connection accumulates one arena per connection for
the life of the process (measured ~8.7 KB/child, linear; 6000
connections → ~58 MB and climbing). The leak is in the *primary
server shape*, not a corner.

**Why not a manual free.** The obvious fix — a `free(child)` /
`dissolve self` the developer calls when a connection closes — is
the C trap: imperative lifetime management bolted onto a
region-and-lifecycle model, easy to forget (releak) or
double-call (use-after-free). Hale's whole memory story is that
lifetime is *structural*. So the fix must *invoke* the existing
declarative teardown early, never bypass it.

**The model: a child is bound by its parent, but ends with its
own flow.** A child's lifetime is nested in the parent's (it
cannot outlive the parent — the parent-dissolve cascade is the
upper-bound backstop), but it should be reclaimed when *its* work
ends, not deferred to the parent. Two shapes, distinguished by
the parent's declaration, never guessed:

- **Flow** — the parent declares `release(c: Child)`. The child's
  `run()` *is* its flow (e.g. a connection's recv/park loop that
  returns on EOF); when `run()` completes, the child is reclaimed.
  A plain `return` reclaims a connection — no ceremony.
- **Resident** — no parent `release`s the type. `run()` returning
  means "ready"; the child lives as a subscriber until the parent
  dissolves (a fixed cohort of workers is *meant* to). The same
  `run()`-returns event means opposite things for the two shapes,
  which is exactly why the discriminator is explicit.

**The verbs.**
- `terminate;` — the locus analogue of `return`: a child ends its
  own lifecycle from inside one of its methods. It sets a latch
  and exits; the teardown runs at completion. Explicit, works for
  any child (a resident that decides to end early, a flow closing
  itself). It *invokes* drain → dissolve → reclaim — never a raw
  free — so it cannot leak or double-free.
- `release(c: Child)` — the death-side bookend, symmetric to
  `accept(c: Child)`. It both *marks the flow* and gives the
  parent a single place to observe every completion, firing after
  the child drains and before it dissolves so the parent reads
  the child's final settled state (mirror of `accept` reading it
  fresh). The parent decides both ends of a child's life — birth
  via `accept`, the death model via the presence of `release` —
  without ever holding a mutable registry of live children: the
  child flows in on the bookend, the runtime does the teardown.

**Safety.** Reclaim runs on the child's own pool worker after
`run()` returns, while its arena is valid — the parent isn't
dissolving. `emit_locus_arena_destroy` is idempotent (NULLs
`__arena`), so a child reclaimed early and then reached again by
the parent's eventual dissolve is torn down exactly once. A coro
*parked* at teardown time (shutdown, a future parent-drain) is
cancelled cooperatively — the **wakeable park**: a wake `eventfd`
unblocks the pool worker, a cancel flag makes the parked
`recv`/`accept` return its EOF sentinel, and `run()` unwinds its
own stack. The runtime never seizes a suspended frame.

**Known tails (v1).** Completion detected inside a *handler*
body (vs `run()`); slot removal from an *iterating* parent's
children buffer; the full latch gating drain+dissolve (only the
arena free is latched); a typecheck gate confining `terminate` /
`release` to locus method bodies; and graceful-shutdown reclaim
of residents.

Tests: `terminate_reclaims_child`, `release_reclaims_flow`,
`async_io_shutdown_parked`.

### F.30 BytesView — non-owning view as a distinct type

The Phase-2 `view()` / Phase-3 `text_view()` machinery hands back
a non-owning pointer that's layout-compatible with `Bytes`. The
ergonomic win is real (`std::bytes::at`, `len`, `slice` all just
work) but the type system can't distinguish a fresh arena-owned
`Bytes` blob from a non-owning view whose validity ends at the
next mutation on the source builder. Fungible call sites,
divergent lifetimes; a footgun symmetric to the pre-F.28
"passing a builder where `Bytes` was expected" hazard the locus
lift closed.

**Design move:** introduce `BytesView` as a distinct type. It
aliases `Bytes` at runtime (same `[i64 len][u8 data]` pointer
shape, zero allocation) but carries different intent at
typecheck:

- `BytesBuilder.view() -> BytesView` (was `Bytes`).
- `BytesBuilder.text_view() -> StringView` (was `String`) —
  symmetric companion for the C-string-shaped case.
- A `BytesView` coerces implicitly to `Bytes` ONLY at function-
  argument positions where the parameter is declared `Bytes` —
  i.e. read-only use sites (`std::bytes::at(v, i)`, `len(v)`,
  pattern-matching, etc.). The coercion is a no-op at runtime
  (same pointer) but the typechecker emits it as an explicit
  cast so the read-vs-store axis is visible.
- A `BytesView` does NOT coerce to `Bytes` at storage sites: a
  field declared `bytes: Bytes` or a `let x: Bytes = …`
  rejects a `BytesView` value. Callers wanting owned storage
  must call `.clone_to_bytes()` explicitly, which deep-copies
  into the caller's arena (via Task 8's TLS-routed allocator).
- `BytesView` IS allowed at storage sites whose declared type
  is `BytesView`. The pond/websocket pattern — storing a
  view in `self.last_message.bytes` with a documented
  "deferred clear" invariant — remains expressible; the type
  signature now declares the lifetime intent, and any
  maintainer reading the struct knows the field is non-owning.

**What this catches:**

- Accidental fungibility — a function whose param is declared
  `Bytes` (signaling "you can keep this") now rejects a
  `BytesView` value at the call site; the caller must
  explicitly `clone_to_bytes()` to opt into the deep copy.
- Field declarations carry lifetime intent. A struct with
  `bytes: BytesView` is self-documenting about the non-owning
  semantic; the type-vs-`Bytes` distinction is visible at
  every read site of the struct.

**What this DOES NOT catch (was deferred to F.30b — now closed):**

- Mutation of the source builder while a `BytesView` from it
  is still observable. The "future maintainer adds
  `frag_buf.append(...)` between `frag_buf.text_view()` and
  the consumer's read" hazard. Two enforcement options were
  on the table: a compile-time borrow checker (v2 territory)
  or a runtime epoch guard (v1.x). The runtime guard ships
  as F.30b below.

### F.30b BytesView / StringView mutation-while-live runtime guard

**The remaining hazard after F.30:**
The distinct-type work in F.30 catches the fungibility hazard
(stored a view where Bytes was expected). It does NOT catch the
*timing* hazard:

```hale
let v = frag_buf.text_view();
frag_buf.append(more_bytes);   // ← invalidates v silently
let txt: String = v;           // ← reads stale bytes; no diagnostic
```

The view aliases the builder's buffer; the second `append` may
realloc the buffer or shift its contents, leaving the view
observably-broken with no compile-time signal. The Hale v1
language has no borrow checker (compile-time lifetime tracking)
to close this at typecheck.

**Design move:** runtime epoch guard.

- `lotus_bytes_builder_t` gains a monotonic `int64_t mutation_epoch`
  field, bumped by every mutating op (`append`, `append_slice`,
  `shift_front`, `clear`, `advance`).
- `view()` and `text_view()` return a 16-byte by-value struct
  `lotus_view_t { src: ptr, epoch: i64 }`. `src` is overloaded:
  when `epoch >= 0` it's the source builder pointer (real view);
  when `epoch == LOTUS_VIEW_EPOCH_STATIC = -1` it's the static
  data pointer (lotus_view_from_static_data, or the null-handle
  path of builder_view / builder_text_view). The underlying
  aliasing data pointer is *recomputed* at unpack time from
  `((lotus_bytes_builder_t*)v.src)->buf` (Bytes-shape: `buf - 8`;
  C-string shape: `buf`), so the view itself doesn't store it.
- Every read-site coercion (`view_coerces_to(BytesView, Bytes)`
  / `(StringView, String)` and the `len(view)` / `println(view)`
  builtins) emits a call to `lotus_bytes_view_data` or
  `lotus_str_view_data`. The helper compares the view's
  `epoch` against the builder's live `mutation_epoch`; on
  mismatch it calls `lotus_view_stale_panic` (noreturn) — stderr
  diagnostic + `_exit(1)`. The static sentinel skips the check
  and returns `src` directly.

**Failure routing.** Stale-view detection is a runtime panic, not
a closure violation. Closures are for recoverable conditions the
owning locus can handle (e.g. alloc fail → restart / drain /
bubble). A stale view is a *programmer error* — the code that
held the view past a mutation is structurally wrong, no
restructuring at the locus level can recover it. Hard panic with
a clear diagnostic mirrors how out-of-bounds array reads behave
in the rest of the language.

**What this catches:**

- The "future maintainer adds a mutation between view() and read"
  regression — exits non-zero at the moment of misuse, with the
  view kind on stderr.
- Equivalent for `text_view() → StringView` consumers.

**What this still doesn't catch:**

- View captured before `finish()` or `dissolve()`, then read
  after. The builder's memory is freed; the view's `builder`
  pointer dangles; the epoch load is undefined behavior. Same
  shape as today's lifetime contract — separate concern, would
  need either reference counting or compile-time lifetime
  tracking to close.

**Codegen impact.** `BytesView` and `StringView` are still
distinct types at typecheck (unchanged from F.30). Their LLVM
representation is a `{ptr, i64}` struct passed and returned by
value — SysV AMD64's "two INTEGER eightbytes ≤ 16 bytes" rule
puts the view in `{rax, rdx}` on return and two arg registers
on entry, so the hot path moves through SSA without memory
traffic. The unpack-and-check helpers are mechanically inserted
at every view-accepting consumer site (the `view_coerces_to`
consultation, the `len()` builtin, the `println` arms). Other
consumers (struct field storage of `BytesView` / `StringView`,
function arguments declared as view types) are pass-through —
the view flows along until it hits a read-coercion site, where
the unpack fires.

**Allocator.** No arena allocation per `view()` / `text_view()`
call. The 16-byte by-value struct materializes in two SSA
registers; the caller's storage slot (if any) is a 16-byte
struct alloca that the existing arena routing already covers.
Read-time cost is a one-load epoch check plus a recompute of
`buf - 8` (Bytes) or `buf` (Str) from the builder pointer.

### F.31 Deployment seams live at main; intrinsic shapes live on the locus

Hale's substrate-discipline annotations sort into two
families. The split is load-bearing and was implicit before
this commitment; F.31 names it.

**Intrinsic shapes** describe what a locus *is*. They live on
the locus declaration, survive across deployments, and compose
through nesting (a child locus inside a parent has its own
intrinsic shape independent of where the parent runs):

- `capacity { pool X of T; heap Y of T; }` — F.22 storage
  discipline.
- `@form(vec | hashmap | ring_buffer)` — application-layer
  container shape.
- `: projection rich | chunked | recognition` — F.2
  perspective-resolution commitment.
- `: tier N` — structural depth in the lotus tower.

**Deployment seams** describe how a locus is wired into a
specific binary. They live in `main`-only blocks, vary across
deployments of the same library, and do not nest (only top-
level `main locus` declarations carry deployment blocks;
nested loci inherit their deployment context from the
containing tower position):

- `bindings { Topic: transport; }` — per-topic transport
  binding (Phase 2 of the topic system).
- `placement { field: pool_spec; }` — per-locus thread
  placement (this commitment).

**The diagnostic test.** When deciding whether a new
substrate annotation belongs on the locus or in `main`, ask:
*can a library author commit to this without knowing the
binary's deployment context?* If yes, it's intrinsic — a
property of the locus's identity. If no, it's a deployment
seam — a property the binary picks.

A useful corollary: anything that takes "main-only" cleanly
is a deployment seam; anything that struggles with the
main-only restriction is intrinsic. The proposed
`@form(scheduler)` annotation flunks this test — `@form(...)`
annotations are expected to work on any locus declaration,
so a main-only `@form(scheduler)` would be a form lying about
its category. The correct shape is a parallel deployment
block, not a form annotation.

### F.31a Placement-at-main + M:N cooperative pools

Schedule is a deployment concern: a `std::http::Server`
should not bake "cooperative" into its locus identity because
a consumer binary may want it pinned. F.31 makes this
explicit; the v1 surface follows from it.

**Surface.** Schedule annotations are removed from
`locus_annotation`. A new `placement { }` block on `main
locus` carries per-locus placement specifications:

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
    bindings {
        // ... per-topic transport bindings, unchanged
    }
}
```

**Keying rule.** Placement entries key on main-locus `params`
field names (snake_case), not on locus type names. This lets
two siblings of the same locus type take different placements
— exactly the parallelism case that prompted the change
(per-venue gateways on distinct cores).

**Pool inference rule.** The set of cooperative pools is
inferred from `cooperative(pool = X)` references in the
`placement` block. The compiler spawns one OS worker thread
per inferred pool name (plus the existing main thread, which
is always pool `main`). No `threads { }` declaration block at
v1 — when per-pool attributes (priority, affinity, realtime
hint) become useful, the block lands then as a typed
extension.

**Nested-instantiation rule.** Placement entries apply only
to top-level `main locus` `params` fields. Loci instantiated
nested in another locus's body (in `birth` / `run` / lifecycle
methods, or as let-bound children) inherit the parent's pool.
The single-threaded-method invariant (below) makes this
inheritance a hard rule, not a default.

**Single-threaded-method invariant.** A locus's methods may
be invoked only on the OS thread that owns its placement's
pool. Cross-pool method calls and lateral field accesses go
through the bus's existing copy-and-condvar dispatch
machinery, which already crosses thread boundaries safely.
The typechecker walks the static call graph from each
top-level placement entry, propagates pool ownership through
method receivers, and rejects calls that cross pools without
going through the bus. This is the substrate enforcement that
makes M:N safe — without it, multi-pool deployments would
silently race on locus arenas (which are unsynchronized
bump allocators).

**M:N cooperative pools.** Each cooperative pool is one OS
worker thread running its own `lotus_bus_queue_drain` loop
against its own per-pool queue. Cross-pool bus dispatch:
publisher enqueues on the subscriber's pool's queue via the
existing inline-payload-copy machinery (m28b stage 1); the
broadcast wakes the subscriber pool's drain thread. The
condvar+memcpy substrate that handles cooperative→pinned
today extends naturally to cooperative→cooperative across
pools.

**Bimodality revisited.** The pre-F.31 bimodality argument
("cooperative or pinned, no third class") lived at the locus
declaration site. Under F.31 it moves to the placement entry
site — a placement is still bimodal (`pinned` or `cooperative
(pool = X)`), but the locus itself doesn't carry that choice.
This is a strict generalization: pre-F.31 deployments expose
exactly the prior shape (one main thread + per-pinned-locus
threads) by writing the same `placement { }` entries that
were previously `: schedule` annotations.

**Considered and rejected.**

- *`@form(scheduler.X)` on `main locus`.* Tempting because of
  the family resemblance to existing form lowerings, but
  fails the F.31 intrinsic/deployment test — `@form(...)`
  annotations are expected to work anywhere a locus can be
  declared, and a main-only form would be the mechanism
  lying about its category. The bindings parallel is the
  right precedent.
- *Type-level placement keying (`Gateway: pinned`).*
  Initially looked symmetric to `bindings { Topic: ... }`
  but fails the per-venue-gateway case: two `Gateway`
  siblings need distinct placements. The structural reason:
  topics are global channels (one declaration, fan-out
  subscribers); loci are instances (each is its own running
  entity). The keying surfaces follow.
- *Pre-declared `threads { pool main; pool io; }` block.*
  Adds typo safety + future per-pool attribute space, but
  redundant at v1 when pools have no attributes. Lands when
  per-pool attributes earn it.
- *Per-pool work-stealing.* Each pool is one OS thread at
  v1 (M:1 within a pool; N pools across). Work-stealing
  inside a pool (multiple threads consuming one pool's
  queue) is a v2+ concern — the v1 design preserves the
  per-arena single-threaded invariant by construction.

**Friction items this closes.**

- `spec/runtime.md` § "Long-running cooperative children
  block parent run()" (Item D) — closed. Parent and child
  can sit in different placement entries with distinct
  pools; their `run()`s no longer serialize.
- The downstream "`std::http::Server` as child blocks parent"
  friction — closed by the same mechanism.
- The "sibling-in-main pattern" workaround becomes the
  canonical shape rather than a friction routing.

**Still open.**

- Cross-pool mailbox drain for the cooperative-publisher →
  pinned-subscriber path (`spec/runtime.md` Item B) is
  orthogonal. The placement substrate doesn't fix
  the missing condvar drain on that path; that's its own
  runtime bug.

**Adapter placement interaction.** Adapter loci instantiated
inline in a `bindings { Topic: AdapterLocus { ... }; }` entry
are NOT main-locus `params` fields, so they receive no
`placement { }` entry. Their `run()` recv-loops need a
dedicated thread by construction; the substrate places them
**pinned-equivalent implicitly** (same m90 routing + pthread
spawn that pre-F.31 fired via the adapter's
`: schedule pinned` annotation). The annotation is gone but
the behavior is preserved automatically because the bindings-
inline shape unambiguously signals "this is a transport
adapter with a recv loop." If a future workload needs adapter
placement on a cooperative pool, the binding grammar can
extend to reference a main-locus field by name (a v2
extension); v1 keeps the inline form with implicit pinned
placement.

### F.32 Locus working set as a cache-budget primitive (sketch)

**Status: design sketch.** No grammar surface, no compiler
behavior shipped. Documented here because the locus model
already exposes the structural primitives a cache-aware
optimizer would need; calling out what's possible makes the
shape easier to evaluate when a workload surfaces friction
that justifies the work.

**The observation.** Cache hierarchies care about partitioning
data by *which thread touches it* and *how big the hot
working set is*. Most languages can't reason about either at
compile time — memory is an undifferentiated heap, threading
is opaque, and lifetime is implicit. Hale's locus model
declares all three structurally:

- **Region semantics.** Each locus owns an arena (`__arena`,
  per `spec/memory.md`). Allocations made by the locus's
  methods route through that arena. The arena's bounds are
  declared by projection class + F.22 capacity slots — not
  arbitrary heap growth.
- **Lifetime.** Each locus has a declared lifecycle (birth →
  run → drain → dissolve); the arena is created at birth and
  freed wholesale at dissolve.
- **Thread isolation.** F.31 placement assigns each locus to
  exactly one OS thread (pinned own thread; cooperative pool
  shared with other loci on that pool's thread). The single-
  threaded-method invariant says one core writes the arena.
- **Cross-region access.** Vertical-only flow (F.6 / F.11)
  + bus boundaries are the only inter-locus data paths, and
  both are statically visible.

A compile-time cache analyzer running over an Hale program
sees the same partitioning the hardware caches do: per-
locus working sets, per-thread arena owners, declared cross-
thread boundaries. That's structurally novel for a general-
purpose compiled language.

**What this already buys, implicitly.**

- Single-threaded-method invariant → no MESI cross-core
  invalidation on locus state. Each locus arena is touched
  by one core; cache coherence traffic for locus-local
  reads/writes is zero.
- Pinned placement (especially `pinned(core = N)`) → the OS
  keeps the worker on a specific core → the locus's working
  set stays resident in that core's L1/L2 across handler
  fires.
- Per-method scratch (Phase 4, 2026-05-21) → tight
  allocate-touch-free loop on every method invocation →
  naturally L1-hot for the duration of one handler.
- Cross-pool bus dispatch → memcpy at the layer boundary
  into the destination pool's queue cell → the receiver
  pool's worker drains by reading that same cell first →
  already a cache-friendly transfer pattern.

None of this required calling it "cache-aware." It fell out
of the locus model.

**What an explicit cache pass would add.** Three layers, in
ascending invasiveness:

1. **Cache-line padding on known-hot field boundaries.**
   Cheapest, biggest demonstrable win. Phase 4a/4b lit up a
   producer/consumer cross-pool pattern: multiple producer
   loci on different cores write to a shared
   `@form(hashmap)` of Counter cells (the Prometheus
   registry shape). Without padding, two cells that happen
   to land on the same 64-byte line generate false-sharing
   pressure even though logically each producer writes its
   own cell. Padding the `indexed_by` row to a cache-line
   multiple eliminates the pressure. The substrate has all
   the info: the cell type's size, the form layout, and
   (post-Phase-4a) the fact that cells are cross-pool
   reachable.

2. **Compile-time working-set budget per locus.** A build
   flag (`--target-cache=L1|L2|L3`) parameterizes a static
   analysis: the compiler sums each locus's projected
   working-set bytes (arena chunk size + `@form(...)` cell
   capacities × cell stride + nested loci's budgets) and
   compares against the chosen target. Out-of-budget loci
   produce a warning naming the tower depth at which the
   budget overflows. Optional opt-in `@locality(L1)` /
   `@locality(L2)` annotation makes the intent
   declarative: "this locus is expected to fit in L1; warn
   if not." Doesn't change codegen — just surfaces "this
   tower won't fit in L2 on the chosen target" before
   anyone runs `perf stat`.

3. **Per-pool arena chunking sized to cache.** Today the
   chunk allocator picks a default chunk size. A cache-
   aware variant sizes the first chunk = `L2_per_core /
   loci_on_pool` so each cooperative-pool worker's hot loci
   don't thrash each other's L2 slices. Growth chunks land
   on cache-line multiples; small allocations within a
   chunk pack contiguously so a single chunk traversal stays
   prefetcher-friendly.

**What it can't do.** Caches aren't addressable. No CPU
exposes "pin this 32 KB region in L1." All three layers
above optimize for *probabilistic residency* via affinity +
size discipline, not deterministic placement. This is not
weaker than what real systems achieve — Intel Advisor,
LLVM PGO, hand-tuned C++ all live in the same probabilistic
world — but it's worth naming so the design doesn't get
oversold.

Runtime working-set sizes also depend on data: a
`@form(hashmap)` capped at 1 M entries doesn't usually
carry 1 M; the static bound is sound but loose. The budget
analysis is most useful when capacities are tight (chunked
projection class, recognition pool sizing) — exactly the
loci where cache residency matters most.

**Comparable landscape.** The structural-primitive overlap
with other languages:

- **C / C++ / Rust.** Per-data-structure facilities only:
  `alignas(64)` / `#[repr(align(64))]`, manual padding,
  `std::hardware_destructive_interference_size`,
  `__builtin_prefetch`. Library-level (folly, abseil,
  crossbeam) cache-line-padded primitives. No language-
  level region semantics; no per-component working-set
  visibility. Profile-guided optimization (`-fprofile-use`,
  LLVM PGO) is mature but runtime-driven, not structural.
- **Erlang / Pony actor languages.** Single-threaded-per-
  actor gives MESI-free per-actor state — same as our
  single-thread invariant. But no declared bounds on actor
  heap, no per-component budget analysis.
- **GPU shading languages (CUDA / Metal / HLSL).** Per-
  workgroup shared-memory budget IS a compile-time cache-
  tier budget — the closest analog. But it's a single tier
  (programmer-managed scratchpad, not a hardware cache),
  on a specialized memory model.
- **Real-time / DSP toolchains.** Section placement
  attributes (`__attribute__((section))`), manual cache-
  line annotation. Per-symbol, not per-component.

The combination of (region semantics + lifetime + thread
isolation + declared bounds, all visible together at
compile time, with vertical-only flow making
cross-component access statically distinguishable) is what
no general-purpose compiled language ships today. The
*individual ingredients* exist elsewhere; the integration
is the structural advantage. Whether that advantage gets
turned into a measurable speedup is a function of how much
of the cache pass above we choose to implement.

**Honest current state.** None of layers 1-3 is implemented.
The structural foundation is there; the analysis isn't.
What we have today is the implicit alignment described
above: locus boundaries happen to match the partitioning
caches reward, so well-shaped Hale programs benefit from
caches *as much as* well-shaped C/Rust programs without the
programmer reasoning about cache lines. Going beyond that —
turning the locus model into an active cache-aware
substrate — is a real workload-justified follow-up, not a
v1 commitment.

**Friction items this would close (when justified).**

- Producer/consumer false-sharing on `@form(hashmap)`-
  backed shared state across Phase 4a/4b pools (currently
  unverified; would need a perf measurement against a
  Prometheus-registry workload to quantify).
- "Tower designed but doesn't fit" surprises when a chunked
  projection's per-coordinatee sub-region overflows L2
  under realistic capacities — currently only surfaces as
  measured perf regression.

**Friction items this would NOT close.**

- Anything cross-process. Caches don't help when the
  boundary is a unix socket / shm ring.
- Variable-capacity hashmap working sets exceeding the
  static bound at runtime — the budget is a hint, not a
  cap.



### F.33 Fallible user-supplied bus adapters (sketch)

**Status: design sketch (2026-05-28).** No grammar surface, no
compiler behavior shipped. Captured here because the Phase-3
routing-keys `on_unmatched: fail` impl (2026-05-25) opened a
path that this extension fits into cleanly; documenting now so
a future session can pick it up without re-deriving the shape.
F.36 (codecs, shipped 2026-05-28) has since landed the
companion precedent: codec `encode` / `decode` methods declared
`fallible(E)` are now invoked by the bus runtime at publish /
receive sites, with the substrate threading fallibility through
the synthesized thunk ABI. F.33 reuses the same machinery on
the adapter `send` path — the implementation surface shrinks
accordingly once F.36's thunk-synthesis pattern is in tree.

**The observation.** A user-supplied bus adapter (`bindings {
Topic: MyAdapter { ... }; }` in main locus) today declares an
infallible `send`:

```hale
locus MyNatsAdapter {
    params { url: String = ""; }
    fn send(subject: String, bytes: Bytes) {
        // network — failures must be swallowed or panic'd
        // internally; no way to surface to the publisher
    }
}
```

If the underlying transport fails (broker down, queue full,
network unreachable, write timeout), the adapter has nowhere
to put the error: the contract says `send` returns nothing,
the publisher's `Topic <- value;` statement is non-fallible,
and Hale has no general "best-effort, may silently drop"
metadata. So adapters either swallow errors (publishers
unaware), retry internally (latency unpredictable), or call
`lotus_root_panic` (process death on any transport hiccup).

**The opening.** Phase-3 routing-keys shipped `on_unmatched:
fail` — a topic-level policy that makes the publish-side `<-`
expression fallible-required (`K <- value or raise / or
discard`). Same vocabulary, same disposition machinery, same
typecheck enforcement that fallible-method calls have used
since v1.x-FORM-1. Extending that mechanism to surface
transport failures from user-supplied adapters reuses the
same plumbing:

```hale
locus MyNatsAdapter {
    params { url: String = ""; retries: Int = 3; }
    fn send(subject: String, bytes: Bytes) fallible(NatsError) {
        // ... may return err
    }
}

main locus App {
    bindings { Login: MyNatsAdapter { url: "nats://..."; }; }
}

locus Worker {
    bus { publish Login; }
    fn process(c: Credentials) {
        Login <- c or handler(on_nats_err);
        //         ^^^^^^^^^^^^^^^^^^^^^^
        //         fires on NatsError from adapter.send
    }
}
```

The Send statement becomes a fallible expression when the
target topic's binding routes to an adapter whose `send` is
declared fallible. The typecheck enforces the `or` disposition
the same way `on_unmatched: fail` does.

**Two design choices to pin down at impl time.**

1. **Derived (auto) vs. explicit (`on_transport_failure: fail`
   on the topic decl).** The routing-key `on_unmatched`
   policy is *intrinsic to the topic* — "no in-process
   subscriber matched my key" is a logical condition the topic
   author can reason about up-front. Transport failure is
   different: it's *intrinsic to the binding*, and the same
   library compiles against different bindings in different
   binaries (the F.31 deployment-seam point). Two paths:

   - **Derived.** Typecheck walks `bindings → adapter → send`
     at compile time; if the bound adapter's send is fallible,
     publishes on that topic require `or`. Pro: ergonomic; the
     topic decl doesn't have to anticipate every transport.
     Con: same library compiles differently against different
     bindings — could surprise a reader who didn't check the
     main locus's bindings.

   - **Explicit on topic decl.** A new clause like
     `on_transport_failure: fail` (symmetric with
     `on_unmatched: fail`). The topic author commits up-front;
     adapters that don't match the contract are rejected at
     bindings-typecheck. Pro: contract visible at topic decl,
     symmetric with routing keys. Con: more boilerplate;
     forces topic authors to anticipate transport fallibility
     across all bindings.

   **Lean toward derived.** The deployment-seam philosophy
   (same library + different bindings → different binary
   behavior) wants the binding decision to drive the publish-
   side type. The reader who's confused by the divergence
   should be reading the bindings block anyway — that's where
   deployment shape lives.

2. **Substrate transports stay infallible at the language
   surface.** `unix(...)` and `shm_ring(...)` already swallow
   IO errors (with `on_overflow: discard|panic|...` as the
   shm_ring back-pressure escape hatch). Re-routing them
   through the fallible-publish surface would force every
   existing `<- value;` site to carry `or` — a sweeping
   breaking change with no clear win. Keep them as today; the
   new fallibility is opt-in by *user-supplied* adapters
   declaring fallible-typed send.

**Composition with routing-key `on_unmatched: fail`.** A
topic that's both keyed-with-fail-policy AND bound to a
fallible adapter has two error sources — "no in-process
match" and "transport delivery failed." The cleanest answer
is to synthesize a per-topic union err type at codegen
(`BusSendError = BusUnmatchedKey | <AdapterErr>`), have the
`or` disposition address the union, and destructure in the
handler. The codegen pattern mirrors the existing fallible-
method err-union handling.

**Sequencing.** This is a v0.3 follow-on, not v0.2 of routing
keys. v0.2 of routing keys is "ship `or handler(err)` / `or
fail <p>` for the routing-key `BusUnmatchedKey` case" — needs
`BusUnmatchedKey` stdlib type synthesis, mechanical from the
existing fallible-disposition machinery (commit message of
`bus-routing-keys-fail-fallback` calls this out). F.33
(fallible adapters) wants v0.2 already in for the err-union
composition path; without `BusUnmatchedKey` synth working,
the multi-source case can't be modeled cleanly.


### F.34 Per-epoch field reset (v1.x-WINDOWED)

**Status: shipped (2026-05-28).** Grammar + typecheck +
codegen + runtime tree-walker all live. See
`spec/grammar.ebnf § closure_clause`, `spec/semantics.md §
Per-epoch field reset`, and the
`crates/hale-codegen/tests/closure_resets_per_epoch.rs`
integration tests.

**The problem.** Closure assertions can express
point-in-time invariants (`self.x ~~ self.y within 0`) and,
post-m46, stream accumulators (`sum(self.delta) ~~ 0 within
100`). Neither shape expresses a *rate budget*: "at most N
events of type X per minute." The canonical use is mdgw
corruption-rate auditing — a parse-error counter that should
average to zero over rolling 1-minute windows. Today users
either give up the closure framing (track the counter
manually, fire violations from inline `violate` calls) or
re-implement per-window reset by snapshotting the counter at
window boundaries and computing deltas — both leak the
windowing math into application code.

**The shape.** A new closure clause
`resets_per_epoch(field1, field2, ...);` names locus fields
the runtime zeros AFTER the closure assertion fires at a
`duration(N)` epoch boundary. Ordering matters: the assertion
sees the window's accumulated value; the reset prepares the
next window.

```hale
closure low_corrupt_rate {
    self.corrupt_per_min ~~ 0 within 10;
    epoch duration(1m);
    resets_per_epoch(corrupt_per_min);
}
```

User code increments / decrements the field as the window
accumulates (`self.corrupt_per_min = self.corrupt_per_min + 1`
in the parse-error path); the substrate keeps the counter
honest about which window it belongs to.

**Why a closure clause, not a form or library locus.**

- **Forms shape storage.** `@form(vec/hashmap/ring_buffer)` are
  all container substrates. A windowed counter is
  computational / temporal, not a storage shape — adding it as
  a form would muddy the form taxonomy.

- **Library loci hide the temporal contract.** A
  `std::metrics::WindowedCounter` lib locus would work, but
  the closure assertion would degenerate to
  `self.rate.count() ~~ 0 within N` — the compiler couldn't
  reason about the temporal contract at the closure layer.
  F.27's framing has the closure as *the* compiler-visible
  structural contract; hiding the window math behind a method
  call breaks that.

- **Closure clauses are the right axis.** `resets_on(events)`
  already lives on the recovery-event axis (reset on
  restart/quarantine/dissolve/replace). `resets_per_epoch`
  mirrors the shape on the epoch-firing axis. Same grammar
  production family; same load-bearing intent ("close around
  the counter so the substrate keeps it honest").

**Typecheck restrictions.** The clause is rejected unless
paired with `epoch duration(N)` — `tick` recurs too fast to
be a useful rate-budget window, and `birth` / `dissolve` /
`inline` / `explicit` don't recur. Each named field must be
declared on the enclosing locus and have numeric type (Int /
Uint / Float / Decimal); zero is not a meaningful reset value
for booleans, strings, or structs.

**Considered and rejected.**

- *Auto-detect "rate-budget" closure shape and synthesize the
  reset.* Pattern-matching `self.X ~~ 0 within Y; epoch
  duration(Z);` as "obviously a rate budget" was tempting but
  would couple two orthogonal axes (assertion shape and reset
  semantics) — closures that legitimately accumulate without
  resetting (cumulative drift, total event count) would
  silently get the wrong behavior. The explicit clause keeps
  the user in control of which fields are window-scoped.
- *`epoch duration(N) windowed`-style flag.* Single keyword
  instead of a field list. Couldn't name which fields to
  reset, so the user would lose control over fields that
  *aren't* window-scoped but live alongside one that is. The
  field list is load-bearing.


### F.35 Green-I/O cooperative pools (`where async_io`)

**Status: shipped (2026-05-28).** Substrate plumbing in
`crates/hale-codegen/runtime/lotus_arena.c § lotus_coro_t /
lotus_coop_park_on_fd / lotus_coop_pool_drain_one_async`. User
surface in `spec/grammar.ebnf § placement_constraint`. Typecheck
+ codegen wired through `crates/hale-types/src/check.rs ::
check_placement_block` and `crates/hale-codegen/src/codegen.rs ::
async_io_pools`. Diagnostics via
`std::process::dump_pool_residency()`.

**The problem.** Hale's cooperative-pool scheduler (F.31) is
handler-atomic: each bus cell on a pool runs to completion on
the pool's worker thread before the next cell drains. That's
correct for short-lived handlers (the design assumes "handlers
do bounded work between yield points") but pathological for the
TCP / WebSocket server shape, where a handler is a per-
connection state machine that blocks on `recv` for the
connection's lifetime. With M pool threads, you get a hard cap
of M concurrent connections — every other connection queues
behind a `recv()` blocked on the kernel.

The workaround pre-F.35 was "pinned per connection" or
"per-pool single-locus + many pools." Neither scales to the
Go-shaped "many connections per OS thread" model that the
substrate's lotus framing is otherwise good at.

**The shape (option β: deployment-seam, no language-keyword
extension).** A placement entry may declare `where async_io`:

```hale
placement {
    listener: cooperative(pool = ws_accept)  where async_io;
    worker:   cooperative(pool = ws_workers) where async_io;
}
```

The pool's worker drain loop integrates an epoll instance.
Inside a locus method on this pool, blocking I/O syscalls
(`recv_bytes`, `accept_one`, `send_bytes`, ...) detect the
async_io context via a TLS pool ptr and route through
`lotus_coop_park_on_fd` instead of blocking the OS thread. The
park primitive `swapcontext`s back to the worker's drain
context; the worker services other cells (and other parked
coros' wake-ups) until epoll signals the parked fd is ready,
then `swapcontext`s back into the original coro and the syscall
retries. From the user's perspective, `recv_bytes(stream)` is
the same line of source — only the lowering differs.

**Why the deployment-seam shape (not a per-fn `@blocking`
annotation).** Same axis as `zero_copy` on bus bindings (Form K):
operational requirements are deployment choices, not author
choices. The locus body should be portable across deployments;
the deployment author picks the I/O model based on workload
shape. The same user code that serves five concurrent
connections on a default pool serves five thousand on an
`async_io` pool — with no source edit.

**Stackful vs. stackless coros.** v0.1 uses ucontext + a 64 KiB
mmap-or-malloc'd stack per in-flight handler invocation
(`lotus_coro_t`). Memory cost per concurrent connection ≈
64 KiB + the per-conn locus arena (typically 4-64 KiB) =
roughly 70 KiB per connection. 10k concurrent connections ≈
700 MB — comfortable for any modern server. A later iteration
may replace ucontext with a CPS rewrite (stackless coros á la
Rust `async fn` / Go's pre-1.4 model) if per-connection memory
becomes load-bearing; the user-facing surface (`where
async_io` + transparent `recv_bytes`) stays unchanged.

The 64 KiB stack is a hard ceiling on what handler code may put
*on the stack* while running in a coro. The runtime's own hot
paths must respect it: the bus dispatch functions
(`lotus_bus_dispatch{,_keyed}` / `lotus_bus_dispatch_wire{,_keyed}`)
formerly held `LOTUS_PAYLOAD_MAX` (64 KiB) serialize / deserialize
buffers on the stack, so a serialized publish issued from inside
a coro handler (e.g. `Topic <- value` in a bus handler on an
`async_io` pool) overflowed the coro stack — `dispatch ->
dispatch_wire` alone is 128 KiB — and segfaulted in the
`dispatch_wire` prologue (2026-05-29 fix). Those buffers are now
thread-local statics, off the coro stack; this is safe because a
coro never parks between filling and copying out a dispatch
buffer (deserialize + enqueue, no I/O) and only one coro runs per
worker at a time. Transport reader threads keep stack buffers —
they run on full-size pthreads, not coros. Any future runtime
addition on a coro-reachable path must keep large scratch off the
stack the same way.

**Considered and rejected.**

- *Per-fn `@blocking` / `@async` annotation.* Would couple the
  I/O model to the locus body's source rather than the
  deployment. A locus shipped against one deployment would need
  source edits to ship against another. Same rationale as F.31
  moving placement from per-locus annotations to `main`'s
  `placement { }` block.
- *Default-on async_io on every cooperative pool.* Changing the
  default would silently retroactively alter the meaning of
  every existing `recv_bytes` call. Programs that were correct
  with blocking semantics (e.g. a request-loop that assumes
  reads block until the next message arrives) would shift to
  cooperative behavior — handlers may interleave in surprising
  ways. Opt-in via `where async_io` keeps the change localized.
  Future v0.x may flip the default if the corpus has stabilized
  on the opt-in path; explicit migration vs. retroactive change.
- *Stackless coros via CPS rewrite in v0.1.* More invasive
  codegen (locus methods that may park need state-machine
  lowering); larger blast radius for the initial ship. ucontext
  is portable, supported by glibc / musl / BSD libc, and
  matches the cost shape of historical Go (small stacks per
  goroutine, no preemption). Stackless coros remain a future
  iteration if measured per-coro stack memory becomes the
  binding constraint.
- *Async I/O via thread pool (libuv-style).* Each blocking
  syscall would run on a dedicated I/O thread pool, with
  completion routed back to the cooperative pool via the bus.
  Works for arbitrary syscalls (read of files, getaddrinfo)
  but adds cross-thread coordination per call and loses the
  zero-context-switch property of epoll. Reserve for file I/O
  (where io_uring is the modern answer anyway) — not the
  primary network surface.
- *`pinned-async`: pinned thread with internal poll loop.*
  Pinned loci own their thread already; an "async-aware pinned"
  variant would have the locus's `run()` drive its own epoll +
  state machines. Equivalent power, more boilerplate. The
  async_io cooperative pool gives the same throughput shape
  with one pool worker fanning many connections — no per-conn
  pinned thread, no per-conn run-loop boilerplate.

**Typecheck rules.** All entries on the same named cooperative
pool must agree on `where async_io` (the drain loop is one-or-
the-other). `where async_io` is rejected on `pinned` entries
(pinned owns its thread; no shared drain to park on) and on
pool `main` (the main pool runs inline on the binary's primary
thread with no dedicated worker). See `spec/runtime.md § where
async_io` for the running-context semantics.

**Diagnostics.** `std::process::dump_pool_residency()` writes
one stderr line per cooperative pool with mode (async_io /
blocking), parked-coro count, and pending cell-queue depth.
Embed in a heartbeat tick on long-running daemons for
occupancy visibility — mirrors the `dump_arena_residency`
shape.


### F.36 Pluggable codecs on bus bindings

**Status: shipped (2026-05-28).** Grammar + typecheck + codegen
all live. Bindings carry an optional `codec(L { ... })` clause;
the binding-site assertion enforces both signature shape
(`encode(v: T) -> Bytes fallible(E)` /
`decode(b: Bytes) -> T fallible(E)` where T is the topic
payload) and method purity. At publish + receive time, codegen
substitutes per-binding `__codec_encode_thunk_<Topic>` /
`__codec_decode_thunk_<Topic>` fn ptrs (matching the m70
`lotus_serialize_fn` / `lotus_deserialize_fn` ABIs) for the
default m70 serializer ptrs at the call site — no runtime
dispatch changes; the publisher's `<-` path, local fanout via
`lotus_bus_dispatch_wire`, and remote fanout all just see a
different fn ptr. See `spec/grammar.ebnf § binding_entry`,
`crates/hale-codegen/tests/bindings_codec_clause.rs`,
`crates/hale-codegen/tests/codec_instantiation.rs`, and
`crates/hale-codegen/tests/codec_dispatch_roundtrip.rs` for the
acceptance surface. The companion machinery — compiler-
inferred method purity (`hale-types::purity`) — is the load-
bearing piece and is also reusable for several future features
(memoization, parallel evaluation of pure helpers, sort
comparator verification); F.36 is the headline use case but
not the only beneficiary.

**The problem.** The v1 bus adapter contract
(`interface __StdBusAdapter { fn send(subject: String, bytes:
Bytes); }`) is a *byte transport*, not a *codec*. The bytes are
serialized via the m70 wire format, which is internal and not
publicly specified for cross-language consumption. So Hale ↔
Hale works fine over any user-supplied transport (NATS, MQTT,
custom-broker-over-TCP), but Hale ↔ anything-not-Hale (Python
subscriber over NATS, JSON broker, existing protobuf service)
is structurally unreachable. The adapter can't re-encode
because it receives bytes, not values.

The clean answer is to split *transport* (route bytes between
processes) from *codec* (translate between in-memory values
and on-the-wire bytes). Today's `__StdBusAdapter` covers
transport; F.36 adds a parallel surface for codecs.

**The shape.** A bindings entry may declare an optional
`codec(L { ... })` clause naming a locus that provides the
encode/decode pair:

```hale
type Tick { sym: String; price: Decimal; }

locus TickJsonCodec {
    fn encode(v: Tick) -> Bytes fallible(EncodeError) {
        let b = std::bytes::BytesBuilder { };
        b.append_str("{\"sym\":\"");
        b.append_str(v.sym);
        b.append_str("\",\"price\":");
        b.append_str(std::decimal::to_string(v.price));
        b.append_str("}");
        return b.finish();
    }
    fn decode(b: Bytes) -> Tick fallible(DecodeError) {
        let w = std::json::Walker { src: std::str::from_bytes(b) };
        let sym   = w.find_string_at("sym")    or fail DecodeError { kind: "missing_sym" };
        let price = w.find_decimal_at("price") or fail DecodeError { kind: "missing_price" };
        return Tick { sym: sym, price: price };
    }
}

main locus App {
    bindings {
        TickTopic: nats("nats://localhost:4222")
                   codec(TickJsonCodec { });
    }
}
```

Codegen routes the bus publish path through `codec.encode`
instead of the m70 `__serialize_Tick`, and the receive path
through `codec.decode` instead of `__deserialize_Tick`. The
adapter's `send(subject, bytes)` contract is unchanged — the
codec runs *before* `send` on publish, *after* the receive-side
adapter on dispatch. Two layers, clean separation.

**The compiler obligation: method purity must be inferred and
asserted.** This is the load-bearing piece — and it's
*structurally pure compiler work* with no new user-facing
syntax beyond the `codec(L)` clause itself.

The bus reader thread, the publisher's pool, and any consumer
pool may invoke a codec's methods concurrently with no
serialization in scope. Codec methods must therefore be
**stateless** — no `self.X` writes, no bus publishes, no
impure stdlib calls (println, file write, sleep), and no calls
to non-pure methods transitively. The same property other
languages handle with `Sync` traits, `[Pure]` attributes, or
trust-the-user comments.

Hale's answer: the compiler computes `is_pure` as a derived
property of every method during typecheck (alongside the
existing F.31 single-threaded-method invariant check and the
F.20 structural-interface satisfaction check), then asserts
the property at the binding site. The author never writes a
`pure` annotation. The substrate just enforces purity where it
matters.

If a codec accidentally has a state-mutating method, the
diagnostic surfaces at the *binding site* (not at the method
declaration), pointing at the offending line:

```
error: codec `TickJsonCodec.encode` is not safe to dispatch
       from arbitrary threads
  --> bindings { TickTopic: nats("...") codec(TickJsonCodec { }); }

note: codec methods must be stateless — they may be invoked
      from the bus reader thread, the publisher's pool, and
      consumer pools concurrently.

note: `encode` writes to `self.call_count` at line 47:
   47 |         self.call_count = self.call_count + 1;
      |         ^^^^^^^^^^^^^^^^ this mutates the codec instance
```

The same surgical-diagnostic shape as the v0.8.3 typecheck
landings (#18.6 CQRS, #76 nested-long-running, the
`@form(hashmap)` cell-locus rejection): structural rule, error
points at the antipattern site, fix path is named in the
diagnostic.

**Why this works without a `pure` keyword.**

Purity is a *derived* property, not a *declared* one. The
compiler has full information to compute it from the method
body. The author writes natural Hale; the compiler reads the
body and marks the method `is_pure: bool` as an internal
property of the typecheck output.

Other compiler-enforced design rules in Hale already follow
this pattern: F.31's pool-placement tracking, the CQRS rule
(#18.6 — methods returning loci), F.27's closure-vs-assertion
shape verification. None of these expose a per-method
annotation to the user; the compiler just enforces the
structural rule where it matters.

Pure-method inference fits the same shape. The codec case is
the first feature that needs the property at a binding site,
but the inference machinery is reusable for any future feature
that benefits — memoization, parallel evaluation across a
collection of inputs, sort comparator verification for
sortable collections, hash function verification for
`@form(hashmap)` cells. Each gets the property for free once
the analysis exists.

**Why the locus shape (not free fns).**

An earlier design pass considered binding free fns directly:

```hale
bindings {
    TickTopic: nats("...") encoder = tick_to_json, decoder = tick_from_json;
}
```

Function types as bindings would also work, kill cliffs around
generic-interface satisfaction (just signature matching), and
free codecs from instance-lifetime concerns (free fns are
static addresses). But the locus shape wins on three counts:

1. **Organization.** A codec's encode + decode + private
   helpers naturally cluster as a unit. Splitting them into
   free fns pollutes the module namespace and makes
   accidental encoder/decoder misalignment harder for readers
   to spot.

2. **Proven safety, not asserted.** With free fns the
   thread-safety claim is "trust the author" — the function
   is just a top-level fn that *happens* to be pure. With
   locus + inferred purity, the compiler *proves* the safety
   property and surfaces violations as diagnostics.

3. **Extensibility.** Future codec variants
   (`TickProtobufCodec`, `TickMsgpackCodec`) get their own
   namespaces; helpers stay scoped; the source organization
   stays clean as the codec library grows.

The cost: the structural-interface satisfaction check at the
binding site has to bind T (the topic's payload type) against
the codec's method signatures. That's a small extension to
the F.20 structural-interface machinery — manageable, and the
purity inference is the larger lift anyway.

**Implementation surface.**

- **Grammar / AST**: one new clause on `binding_entry` —
  `codec(L { ... })`. ~30 lines.
- **Purity inference pass** (`hale-types`): dataflow walker
  over locus method bodies. Walks `Stmt::Assign` looking for
  self-rooted LHS, walks call sites looking for non-pure
  callees (transitively), consults the stdlib effect table.
  Sets `info.is_pure: bool` per method. ~250 lines.
- **Stdlib effect table** (~50 lines): names which stdlib fns
  are pure (`std::str::trim`, `std::decimal::to_string`,
  allocation primitives, arithmetic) vs not (`println`,
  `time::sleep`, `std::io::fs::write_file`, bus publishes,
  closure violations).
- **Binding-site assertion** (~50 lines): when `codec(L)` is
  bound, verify L's encode/decode methods exist with the
  right signatures (T = topic's payload type) AND are pure.
  Diagnostic shape mirrors the CQRS rule's surgical-pointer
  output.
- **Codec dispatch codegen** (~100 lines): bus serialize /
  deserialize paths route through the codec method call
  instead of m70 `__serialize_T` / `__deserialize_T`. Same
  return-slot ABI the m70 path uses today (compiler knows
  T's size at the binding site, pre-allocates the
  destination, the codec's `decode` writes into it — no
  double allocation).
- **Stdlib types** (~30 lines): `EncodeError` /
  `DecodeError` so codecs can declare fallible returns.

**Total: ~460 lines, 3-4 days.**

**Considered and rejected.**

- *Per-payload-type encoder/decoder methods (`interface
  Encodable { fn to_bytes() }`).* Each payload type can have
  at most one encoder under this shape — locks `Tick` into a
  single encoding. The codec-as-locus shape lets the same
  topic carry different codecs in different bindings (JSON
  over NATS, protobuf over the unix socket for the analytics
  worker).
- *Binding free fns (`encoder = X, decoder = Y`).* Strictly
  cheaper to ship — no purity inference needed — but loses
  organization and the proven-safety property. Free fns can
  still substitute for codec loci in a future v0.2 (since the
  purity infrastructure is the same) but the locus shape is
  the v0.1 surface.
- *Annotating codec methods with a `pure` modifier.* Pure is
  a derived property of the method body; the compiler has full
  information to compute it. Adding a `pure` keyword would
  give the author the option of *asserting* purity (which the
  compiler would still have to verify) without removing any
  compiler work — pure ceremony. Hale's enforced design rules
  consistently avoid this pattern (cf. F.31, #18.6, #76).
- *Generic codec interface (`Codec<T>`) parameterized at the
  type level.* Would lift F.20's structural-interface check
  to handle interface generics, which is a bigger lift than
  needed for the codec case. The codec-binding-site check can
  do per-binding T resolution without lifting the whole
  interface system.

**Sequencing.** The purity-inference pass is the load-bearing
piece. Build it standalone first (with the F.31 single-
threaded-method check as the test harness — purity is a
stronger version of the same "method body shape" question).
Then layer the binding-site assertion + the codec(L) grammar
+ the codegen path on top. Each step independently testable;
the purity pass benefits multiple downstream features so it's
worth investing in cleanly.



The grammar in v0 does **not** specify:

- **Trait system.** No `trait` keyword in v0 (reserved). The
  Go-style structural `interface` form (F.20) handles the
  immediate need — loci structurally satisfy interfaces with
  no `impl I for L` declaration. Rust-style traits with
  explicit impl blocks, trait bounds on generics, and coherence
  remain a future extension. Generics today are bound only by
  projection class.
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
