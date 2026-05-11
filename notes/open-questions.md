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
   Recognition}.

   **v1 extension (per The Design — minimal surface):** add
   one more built-in bound, `Numeric` (Int / Float / Decimal /
   Duration). Unlocks generic accumulators, generic comparison
   operators, and tolerance values in generic closures. No
   other bounds for v1: substrate-invariance argues for the
   smallest surface that unblocks the next concrete workload.
   Eq/Ord/Display can come post-v1 if a workload demands.

   Generic interactions:
   - **+ bus payloads:** no special handling. Generic types
     monomorphize at codegen; `Result<Int, String>` becomes a
     concrete struct with a mangled name; the bus copies it
     like any other concrete payload.
   - **+ closures:** generic closures over `T` work if T has
     the operations the assertion needs. Tolerance requires
     `T: Numeric`; without Numeric, tolerance must be a
     literal. F.9 closures audit runtime-varying state — the
     comparison must be defined for T.

3. **Refinement types for k_max bounds?**
   **Resolved (delivery plan, commitment 3):** Deferred. k_max is
   computed at compile time from constant params and checked at
   runtime where dynamic. Refinement types add compile-time
   complexity for marginal runtime benefit; runtime check is
   sufficient given the framework discipline already enforces
   correctness.

4. **Decimal semantics.** **Resolved:** matches `shopspring/decimal`
   semantics for FFI compatibility with existing fixed-precision
   ecosystems.

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
   **Resolved (per The Design — runtime/stdlib split):**
   deployment-config maps subjects to transport URLs. Source
   stays transport-agnostic. Runtime owns kernel-level
   transports (shared memory, AF_UNIX, TCP, UDP); stdlib owns
   protocol adapters (NATS, MQTT, gRPC, TLS). The binary takes
   a startup config (file or CLI flags) that routes each
   declared subject to a transport URL. Source-level transport
   annotations would couple coordination semantics to deployment
   topology — exactly what the abstraction prevents.
   **Substrate progress:** m57 ships the AF_UNIX kernel-level
   transport (`lotus_transport_create / send / recv / destroy`
   over SOCK_SEQPACKET in the C runtime). m58 ships the
   deployment-config subject→transport binding: the runtime
   parses `$LOTUS_BUS_CONFIG` at boot via
   `lotus_bus_load_config`, registering each `subject=url:role`
   line through `lotus_bus_register_remote`. Publisher-side
   fanout is wired into `lotus_bus_dispatch`. m59 ships the
   subscriber side: LISTEN-role registration spawns a per-
   subject pthread that owns recv-loop + local dispatch, so
   the cross-process bus is now bidirectional end-to-end.
   Per-payload serializer + multi-binary orchestration is m60.

9. **What happens if the same subject is declared by two loci in
   the same binary?** Compile error or runtime fan-out?
   **Resolved (per The Design — emergent cardinality):**
   runtime fan-out. Multiple publishers + multiple subscribers
   = MPMC; all subscribers receive every published message
   regardless of source locus. Subjects are coordination
   points, not single-owner channels. Cardinality is emergent
   from connectivity, not a runtime configuration.

10. **How do bus messages cross the locus region boundary?**
    **Resolved (already implemented per spec/memory.md):**
    copy. Vertical-only-flow at the memory level says lateral
    references = copies. The bus adapter `memcpy`s the payload
    into the subscriber's arena (m20 ships this for in-memory
    transport). For cross-process: each transport defines its
    own wire format; lotus's contract stays "the receiver's
    arena gets a fresh copy of the payload struct." Codegen
    emits a default per-payload-type serializer; transport
    adapters override when needed. The wire is just a longer
    copy path — semantics don't change at the arena boundary.
    **Substrate progress (m60):** codegen synthesizes
    `__serialize_<T>` / `__deserialize_<T>` per bus payload
    type and routes send/recv through them. Bodies are
    identity (memcpy of sizeof(T)) at v0.1 — the shape is in
    place; a future wire-format milestone replaces the bodies
    (field-by-field little-endian, length-prefixed Strings,
    schema versioning) without touching call sites.

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

16. **`reorganize(...)` semantics.**
    **Resolved (per The Design — vertical-only-flow):**
    `reorganize` is `restart_in_place` lifted from a single
    locus's state to its substructural role. The parent's
    failure invalidates the parent's *configuration* but not
    its *role* in the tree: parent's params reset to declared
    defaults, children are re-attached to the new instance,
    nothing migrates laterally. (Lateral migration between
    siblings would violate vertical-only-flow.) Implementation
    deferred until a workload exercises it; semantic locked.

17. **Ordering of `drain` and `dissolve` after a failure.**
    **Resolved (per The Design — coherent vocabulary):**
    `drain` and `dissolve` are lifecycle methods, not recovery
    operations. m55 removes them from the `RecoveryOp` enum.
    To end a locus's role on failure, use `bubble(err)`:
    failure propagates up through vertical-only-flow, runs
    the locus's drain → dissolve → arena_destroy lifecycle as
    a side effect of teardown. The recovery vocabulary is
    `restart` / `restart_in_place` / `quarantine` / `bubble` +
    `reorganize` (per #16) — five primitives, no overlap.

## Bus handler shape

**Bus handler default-param policy.**
**Resolved (per The Design — coordination primitives have
fixed shapes):** bus handlers take exactly one payload param;
defaults on it are rejected at codegen. Reasons: (a) the
payload param is always provided by dispatch, so a default
would never fire — dead syntax; (b) extras would break the
fixed-arity dispatch contract. Codegen emits a clear error
pointing at this constraint. Modes (m54) and locus `fn`
methods (m34) accept defaults as normal.

## Imports / modules

18. **How do imports resolve paths?**
    Filesystem? Package registry? Both? Probably both,
    filesystem-first like Go's vendor + go.mod.

    **Partially resolved (F.19, 2026-05-11):** within one
    seed (one directory), every `.ap` file shares a top-level
    scope — same shape Go gets from per-package visibility.
    `aperio build <dir>` bundles the directory; no `import`
    needed for in-seed cross-file refs. **Cross-seed imports
    (one app reaching into another, or a package registry)
    remain deferred** — the `module` keyword is reserved with
    no semantics yet.

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

When to pull forward: when fitter-applier-pair (or a comparable
substantive program) shows that single-implementation-per-field
is hitting limits — most likely when projection-class-specific
projections of the same contracted value start being needed at
runtime.

## Tooling

20. **Editor / IDE support.** Tree-sitter grammar derived from
    the EBNF would give editor support cheaply. Probably the
    next artifact after the spec stabilizes.

21. **Compiler implementation language.** Rust (good for
    compilers, mature toolchain) or Go (closer to some teams'
    surface syntax)? Resolved: Rust for the compiler proper.

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
