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

## 3. `params` block

```
params {
    B: int = 1_000_000;
    c: int = 1000;
    sigma: int = 10;
    phi: float = 1.0;
    capital_usd: decimal = 1_000_000.00d;
    inferred_param: int : inferred;
}
```

**Commits to.** Each param has a name, a type, and either a value
(compile-time-evaluable expression) or `: inferred`. The compiler
treats hand-declared values as priors and `inferred` values as
to-be-determined (statically by the compiler if possible,
otherwise at runtime via the lotus runtime's perspective-stability
machinery).

**Why.** Multi-perspective stability is the framework's commit
discipline. Hand-declared values are perspectives the author
provides; `inferred` is "no perspective yet, system finds one."
The grammar makes this explicit per parameter, so a single locus
can mix declared and inferred params without ceremony.

**Considered and rejected.**

- *Make every param a literal.* Reject; the framework explicitly
  permits inferred values, and modeling that as a special case
  in the type system is uglier than encoding it in the grammar.
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
