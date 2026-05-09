# Open questions — lotus-lang v0

Decisions deferred during the v0 grammar draft. Each is a real
question that will surface when the first program is written or
when the type-system / operational-semantics docs are drafted.

## Type system

1. **How are projection-class generics monomorphized?**
   **Resolved (delivery plan, commitment 1):** Compile-time
   monomorphization. Per the runtime-perf-over-compile-perf
   commitment, we accept the compile-time cost for runtime speed.
   Each concrete `Rich<Foo>` / `Chunked<Foo>` / `Recognition<Foo>`
   gets its own machine code.

2. **What does a trait-less generic constraint look like?**
   **Resolved (delivery plan, commitment 2):** `ProjectionClass`
   is a built-in "any-of-three" constraint, analogous to Go's
   `any`. `<T: ProjectionClass>` requires T ∈ {Rich, Chunked,
   Recognition}. No full trait system in v0; can grow later if
   needed.

3. **Refinement types for k_max bounds?**
   **Resolved (delivery plan, commitment 3):** Deferred. k_max is
   computed at compile time from constant params and checked at
   runtime where dynamic. Refinement types add compile-time
   complexity for marginal runtime benefit; runtime check is
   sufficient given the framework discipline already enforces
   correctness.

4. **Decimal semantics.** **Resolved:** matches shopspring/decimal
   semantics for direct FFI compatibility with grease.

## Memory / runtime

5. **How is the parent's bookkeeping for a coordinatee freed when
   the coordinatee dissolves?** **Resolved (delivery plan,
   commitment 5):** per-arena free-list (chunked-class loci) or
   periodic defrag (high-churn). Reclamation is per-arena,
   bounded, deterministic — never stop-the-world. Coordinatee
   sub-regions are pristine arenas freed wholesale on
   dissolution.

6. **What happens to in-flight messages on `dissolve`?**
   **Resolved (05-bus):** drain phase delivers in-flight messages
   before any new messages are accepted; dissolve phase discards
   anything still queued. SIGINT triggers drain on root → cascade
   → leaf loci stop accepting new inbound, finish their in-flight
   handlers, then dissolve.

7. **How is locus-scoped memory shared across mode projections?**
   **Resolved (delivery plan, commitment 7):** modes share the
   locus's arena via the arena cascade. No double allocation;
   no copy. Compiler verifies modes don't write-conflict.

## Bus interface

8. **How does the runtime bind `bus subscribe "..."` to a
   transport at link time?**
   Probably: the binary takes a config arg specifying transport
   (NATS URL, UDP multicast group, etc.), and the runtime maps
   subjects to transport channels.

9. **What happens if the same subject is declared by two loci in
   the same binary?** Compile error or runtime fan-out?
   Probably runtime fan-out (matching grease's behavior).

10. **How do bus messages cross the locus region boundary?**
    Probably copy (vertical-only-flow at the memory level: lateral
    references = copies). The bus adapter copies into the locus's
    arena.

## Closure tests

11. **What does `epoch tick` actually mean?**
    Each accept/dissolve event is a tick? Each runtime tick? Some
    other periodic boundary? Need to specify.

12. **How are closure failures reported?**
    Crash? Bubble to parent? Log + alert? Per-locus policy?
    Probably: per-closure policy, declared in the closure block
    (extension to v0 grammar).

## Perspectives

13. **What does `serialize_as TypeV1` actually do?**
    Just a type alias? Generates a wire format? A versioning
    annotation for forward/backward compatibility? Probably the
    last; need to specify the serialization protocol.

14. **How does the runtime know which perspective is "current" on
    the consumer side?**
    Probably: latest-wins with epoch numbering; consumers track
    last-seen epoch.

## Recovery / failure

15. **What happens if `bubble(err)` reaches a locus with no
    `on_failure` handler?**
    Probably: process exit; the framework's vertical-only-flow
    means failures bubble to the OS at the root.

16. **`reorganize(...)` semantics.** Move children from failed
    parent to a sibling? Spawn a new sibling and migrate? The
    grammar reserves the keyword; semantics are TBD.

17. **Ordering of `drain` and `dissolve` after a failure.**
    Always drain-then-dissolve? Or can policy skip drain?
    Probably: policy choice; defaults to drain-then-dissolve.

## Imports / modules

18. **How do imports resolve paths?**
    Filesystem? Package registry? Both? Probably both,
    filesystem-first like Go's vendor + go.mod.

19. **Is there a standard library?**
    Yes (eventually). Reductions other than sum/prod, time
    arithmetic, decimal arithmetic, common collections,
    bus-transport adapters all live there.

## Structural direction (deferred to v0.5+)

**Arena-stored translation functions / three-way interface.**
Conceptual move surfaced in design conversation: the locus +
parent + contract is genuinely three entities, with the contract
mediating between L's translation implementations (in its arena)
and the parent's reads. Translation functions are bounded above
by the contract's typed surface (F.14 typing rule); multiple
implementations per contract field can coexist (rich/chunked/
recognition projections of the same value); cost reflects
projection class.

For v0: the typing rule is locked (F.14); current `params`
provide default-implementation-per-contract-field. Multi-
implementation syntax (per-projection-class translations,
`@projection rich fn ...` annotations, runtime injection of
new translations) is deferred until an example forces it.

When to pull forward: when trellis-pair (or a comparable
substantive program) shows that single-implementation-per-field
is hitting limits — most likely when projection-class-specific
projections of the same contracted value start being needed at
runtime.

## Tooling

20. **Editor / IDE support.** Tree-sitter grammar derived from
    the EBNF would give editor support cheaply. Probably the
    next artifact after the spec stabilizes.

21. **Compiler implementation language.** Rust (good for
    compilers, mature toolchain) or Go (closer to the surface
    syntax, easier for the team)? Probably Rust for the
    compiler proper, with Go bindings for FFI to grease.

22. **First reference implementation.** ANTLR4 frontend would
    give a fast path to a parser; LLVM backend gives native
    code. Both are well-trodden.

## Implementation gaps vs. spec

These are spec commitments the implementation has not yet caught
up to. Not bugs — the spec is forward content; the implementation
fills in incrementally — but tracking them avoids silent drift.

23. **Immutable-binding compile-time enforcement.**
    **Resolved (m50).** Spec (`design-rationale.md` §E,
    `types.md` "Mutability") commits: `let x = 0; x = 1;` is
    a compile-time error; only `let mut x` permits
    reassignment. m50 lands the enforcement in
    `crates/lotus-types/src/check.rs`: `LocalSym` now carries
    `is_mut: bool`; `Stmt::Let` / `Stmt::LetTuple` propagate
    the AST flag; fn params, loop vars, and pattern bindings
    default to `false`; `Stmt::Assign` raises a diagnostic when
    the target is a bare-head local (no `.field` / `[i]`
    segments, head ≠ `self`) bound without `mut`. Field /
    index reassignment through an immutable head stays allowed
    (the head isn't being rebound — state is being mutated
    through it). `self.field = ...` in lifecycle methods stays
    allowed unconditionally (locus state is mutable
    independently of any binding).

---

These questions are not blockers for the spec being committed.
They are the next layer of decisions the implementation will
force. Leaving them open lets the v0 grammar ship without
premature commitment.
