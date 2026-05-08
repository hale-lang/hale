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
   Drop, deliver, error to sender, store in dead-letter queue?
   Probably: drain phase delivers in-flight; dissolve phase
   discards anything still queued. (Open; resolution awaits
   `02-parent-child` example.)

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

---

These questions are not blockers for the spec being committed.
They are the next layer of decisions the implementation will
force. Leaving them open lets the v0 grammar ship without
premature commitment.
