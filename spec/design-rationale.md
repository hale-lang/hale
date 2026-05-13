# Design rationale

For each major syntactic construct in the Aperio grammar, this
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

Aperio commits to a clean two-primitive split at the
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
block at every layer of an Aperio program: an app is a locus;
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

The axiom also explains a load-bearing decision in the
codebase-onboarder (`notes/codebase-onboarding-design.md`):
when extracting Aperio source from a foreign codebase,
**cross-tower agreement = locus identity**. A node-name that
appears in ≥ 2 of {operational, harmonic, domain} towers is
emitted as a locus in the absorbed source; a node-name that
appears in only one tower is a type, a free fn, or a comment.
Cross-tower coincidence does the inventing — the absorber
never synthesizes loci that aren't already implied by the
foreign code's own emergent structure.

Full design note: `notes/aperio-types-vs-loci.md`.

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

**Why.** The ancient texts' named-concept registry already
commits to: source uses names, renderer produces symbols. Aperio
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
pattern, Aperio collapses the params-vs-state distinction. The
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
fitter and applier binaries — both compile from the same Aperio
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
consumer (applier) compile from the same Aperio source, so the
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
for free. Aperio's locus-lifecycle methods give the deterministic
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
  can't afford GC pauses, and Aperio's locus structure
  obviates the need.
- *Rust-style ownership/borrow checker.* Reject; Aperio's
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
other fixed-precision domain. Aperio makes `decimal` a primitive
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
`spec/types.md` (Phase 0 deliverable).

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
(though Rust uses `r#` prefix for any keyword; Aperio permits
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
  Reject; subscription is structural in Aperio (declared in
  `bus`, dispatched by the runtime), so a receive operator
  has no statement position to occupy. Aperio's subscriptions
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
`std::text::Sink` shipped as a tagged-locus antipattern (one
locus with `dest: String` branching on every method) because
there was no interface mechanism; `std::log::StdoutSink` had to
couple through the bus for the same reason. Structural interfaces
let `StdoutSink` / `StringSink` / `FileSink` coexist as
separate loci with one shared surface, eliminating the inner
dispatch entirely. The Go-shape (structural, no `impl`
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

**Implementation status (Phase A + Phase B).**

- **Phase A — parser / AST / resolver / typechecker (shipped
  2026-05-10).** The `interface` keyword parses, `InterfaceDecl`
  lands in the AST, `TopSymbol::Interface` registers in the
  bundle scope, the typechecker enforces the structural-impl
  rule at every call site where a fn declares an interface-typed
  param. Mismatches produce typed diagnostics (missing method,
  arity, param type, return type).
- **Phase B — codegen vtable dispatch (shipped 2026-05-11).**
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
  arg. End-to-end coverage in
  `crates/aperio-codegen/tests/interface_dispatch.rs`.
- **Phase B follow-ups (deferred).** Returning an interface
  value from a fn, storing one in a locus param/field, putting
  interfaces in arrays/tuples — all need fat-pointer deep-copy
  across arena boundaries (the data pointer inside the fat
  pointer would dangle without it). `emit_return_value_deep_copy`
  currently rejects Interface returns with a pointer at this
  follow-up. Heterogeneous storage (`Vec<Sink>` of mixed-impl)
  needs the same follow-up plus array-of-interface lowering.
  The `std::text::Sink` stdlib migration is unblocked at the
  language level and ships in a separate milestone to keep this
  commit's blast radius bounded.

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
when the n-dim growth arc has its first concrete demo (mdgw or
triangulator).

### F.19 Per-directory seed model

A directory of `.ap` files compiles as one **seed**: every
top-level decl (locus, type, free fn, perspective, const) in
any file in the directory is visible to every other file in
the same directory, in one shared scope. `aperio build <dir>`,
`aperio run <dir>`, and `aperio check <dir>` accept directory
targets and bundle every `.ap` file under them; `aperio build
<file.ap>` keeps working for one-file apps.

File order in the merged bundle is **alphabetical by filename**
(deterministic). Resolution is order-free — the typechecker
flattens all top-level decls into one bundle scope before name
lookup, so a fn declared in `z.ap` is callable from `a.ap`
without ceremony.

There is no per-file visibility (no `pub`, no Go-style
uppercase-exported convention). Anything declared at the top
level is visible to every file in the seed. Cross-seed
imports — one `apps/myapp` reaching into another `apps/lib` —
remain deferred (the `module` keyword is reserved with no
semantics; see `notes/open-questions.md` Q18).

**Why.** Single-file apps grew unwieldy quickly (ferryman hit
~2,300 lines before this milestone landed). The friction log
entry `notes/aperio-friction.md` 2026-05-10 single-file-app-
monolith captured the canonical case. The implementation cost
was small — the typechecker's `Bundle` already accepted multiple
programs, and `aperio run` / `aperio check` already handled
directory targets; only `aperio build` had a hard "single .ap
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
- *Build-system manifest file (`aperio.toml` listing files).*
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

**Slot 1..N parent-override (v1.x-4 + v1.x-4b, SHIPPED).** A
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
intent — `notes/aperio-friction.md`
`dense-locus-storage-bloat` is the canonical writeup.
F.22 names the substrate distinction so the same locus can
hold "what dies with me" (Arena) and "what I recycle / grow
during my life" (Pool, Heap), in language the locus *writes
down* rather than smuggles in via runtime convention.
Operationalizes The Design's multi-dimensional capacity
principle at Aperio's substrate.

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

- `crates/aperio-codegen/runtime/lotus_arena.c` — adds
  `lotus_pool_*` and `lotus_heap_*` symbol families.
- `crates/aperio-syntax/src/ast.rs` — `LocusMember::Capacity`
  variant carrying `Vec<CapacitySlot { name, kind, elem_ty }>`.
- `crates/aperio-codegen/src/codegen.rs` —
  `declare_locus_struct` extends the struct layout with one
  field per declared slot; `lower_locus_instantiation`
  initializes each slot after slot 0; `flush_dissolve_frame`
  walks slots in reverse before slot-0 arena destroy.
- `crates/aperio-codegen/src/codegen.rs` — `lower_expr` for
  `Expr::Field { Self, name }` checks the slot table before
  the field table, so `self.entries` resolves to a slot handle
  type rather than erroring.

**Pickup pointers for implementation.** This session locked
the spec; implementation tasks are tracked at the friction
plan level (`crates/aperio-codegen/runtime/lotus_arena.c`
gets Pool + Heap primitives first; codegen surface follows).

### F.23 Int → Float widening at let/arg sites (Phase 2c, 2026-05-11)

Codegen inserts an implicit `sitofp` widening at two surfaces:

- **let-binding type ascription** — `let nf: Float = self.n;`
  with `self.n: Int` succeeds. The ascription tells the lowerer
  to coerce the RHS at the binding site.
- **fn-arg coercion** — when the parameter type is `Float` and
  the call-site argument type is `Int`, the argument widens at
  the call site. Same rule applies to user-declared fns and to
  stdlib path-calls (`std::math::sqrt(n)` with `n: Int` works
  without `2.0` literals).

**Strictly one-way.** `Float → Int` narrowing remains explicit;
`Decimal` never participates in implicit cross-type conversion;
other numeric pairs (Int↔Decimal, Float↔Decimal) still reject.

**Why.** The friction-log entry
`notes/aperio-friction.md` 2026-05-10 `float-surface-gaps`
documented the cost of forcing every Float-heavy library to
carry parallel Int+Float counters and explicit conversion
plumbing. Phase 2c also shipped `std::math::{sqrt, exp, log,
floor, ceil, pow}`; the widening makes those libm primitives
ergonomic by removing the per-callsite `to_float()` ceremony.

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

### F.24 Block-tail expression / `if` as expression (Phase 2b, 2026-05-11)

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

**Why.** The friction-log entry
`notes/aperio-friction.md` 2026-05-10 `if-needs-block-value`
documented the canonical case: index selection, default
fallbacks, and ternary-ish expressions all need a small
conditional value, and the pre-2b workaround
(`let mut x = i; if cond { x = j; }`) is verbose and obscures
intent.

The shape is form-completeness within the expression-evaluation
substrate: match-arm direct expressions
(`MatchArmBody::Expr(Expr)`) and function-body returns already
produced values; if-blocks were the lone holdout. Phase 2b
closes the form-asymmetry — same shape as Rust, which is what
the friction-log entry asked for.

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

### F.25 Cross-seed imports — vendored source, alias-required (v1.x-IMPORT, 2026-05-13)

An importer references a vendored library by literal path with
a required alias:

```aperio
import "lib/moa" as moa;
import "../shared" as shared;
```

Cross-seed references read as `alias::Name`. The library is a
directory of `.ap` files (per-dir seed per F.19) copied into
the importer's source tree; v1 has no package manager, no
registry, no fetch, no versioning, no lockfile. The source IS
the dependency.

Resolution order is three-step (first hit wins):

1. `<importer-dir>/<path>.ap` — single-file lib.
2. `<importer-dir>/<path>/` — directory bundle.
3. `<workspace-root>/<path>/` — workspace fallback (workspace
   root = upward `Cargo.toml` search).

Library decls are auto-mangled at parse-time with prefix
`__lib_<alias>_<file_stem>_<name>` and registered into a
per-build path-rename table parallel to the static
`STDLIB_PATH_RENAMES` and `MOA_PATH_RENAMES` tables. The user
never writes the mangled form; `alias::Name` resolves through
the table at codegen.

**Why.** F.19 (per-directory seed model) shipped 2026-05-11 and
fixed the single-file-app-monolith friction at the intra-seed
layer. Cross-seed sharing remained deferred — the `module`
keyword was reserved with no semantics. Friction accumulated
in two shapes:

- MOA was bundled unconditionally into every binary (via
  `MOA_AP_SOURCE` in codegen) whether or not the app
  referenced any `moa::*` type. Apps that didn't need MOA
  still paid for it.
- Cross-app helper patterns (tagged-accumulator, directory
  walks, JSON glue) lived in the std seed because there was
  no library home. The std seed grew to absorb friction that
  should live in user libraries.

v1.x-IMPORT (this milestone) opens user libraries as a
first-class shape and unblocks both: MOA can stop being
unconditionally bundled (a follow-up cleanup); user helpers
can graduate from copy-paste / std-seed-bloat to a vendored
shared lib.

**Vendor-the-source as the v1 commitment.** A real package
manager (registry + fetch + semver + lockfile) is several
months of work. The friction this milestone unblocks is "can
libraries exist at all" — not "can we deduplicate dependencies
across projects." Vendoring is how C, early Go, and many other
languages bootstrapped library ecosystems before package
managers existed. Aperio's file-based dir-seed model is
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
- *Package registry / lockfile / `$APERIO_PATH`.* Reject for v1
  (see "Vendor-the-source" above). All deferred.
- *Hand-mangled `pub`-style prefixes on every library decl.*
  Reject; the auto-mangler does it at parse-time. Users would
  have to author `__MyLib*` prefixes by hand otherwise — exactly
  the shape std and moa carry today, but now done automatically.

**Implementation entry points.** See `spec/imports.md` § "Implementation
entry points" for the file paths and primary functions.

## 16. What's deferred

The grammar in v0 does **not** specify:

- **Trait system.** No `trait` keyword in v0 (reserved). The
  Go-style structural `interface` form (F.20) shipped 2026-05-11
  with Phase A typecheck + Phase B codegen vtable dispatch;
  loci structurally satisfy interfaces with no `impl I for L`
  declaration. Rust-style traits with explicit impl blocks,
  trait bounds on generics, and coherence remain a future
  extension. Generics today are bound only by projection class.
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
