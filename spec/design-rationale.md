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

**Commits to.** In the transport-driven hot-load model (the
aspirational path — see `semantics.md` § "Perspective hot-load"; the
*shipped* perspective is the in-process contract + slot), a
perspective is a serializable parameter bundle within a shared
compiled-in schema. Both producer (fitter) and consumer (applier)
compile from the same Hale source, so the type *is* the contract;
the bus carries only parameter values.

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

---

## The locked design commitments have moved

The **F-series** — the locked design commitments (what each construct
commits the design to, what was considered and rejected, and which are
superseded or still sketches) — is an append-only decision log, now in
its own file: [`decisions.md`](./decisions.md). This file keeps the
*current* conceptual rationale; `decisions.md` is the decision/history
layer.
