# Open questions — Hale v0

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

24. **`fallible(E)` on user-declared locus member fns.**
    **SHIPPED (2026-05-25): the blanket "no fallible on locus
    methods" rule was lifted; see CHANGELOG + `spec/types.md`.**

    Friction signal: across multiple apps + libraries, devs
    work around the "no fallible on locus methods" rule by
    extracting a free fn that takes the locus as the first
    arg. Loses `self` ergonomics, requires explicit threading
    of the locus pointer at every call site, splits closely-
    related code across two top-level decls. F.27's
    `closure + violate + error-check fn + draining` four-piece
    pattern handles "kill the locus on this error" cleanly,
    but it doesn't address "propagate this error to my caller
    and let them decide" — that's where the friction shows up.

    **Proposed narrowing:** drop the blanket "no fallible on
    locus methods" rule. Replace with targeted rules per call
    surface:

    - **Substrate-facing surfaces — stay non-fallible.** The
      substrate orchestrates these and has no place to route
      a `fallible(E)` return; failure goes ↑:
      - **Lifecycle methods** (birth / run / drain / dissolve /
        accept / on_failure / mode entry / exit).
      - **Bus-subscribed handlers**. Verified at the
        `subscribe ... as handler` site (bus dispatch has no
        return path). The check is per-subscribe, not per-fn,
        so a fn that's fallible-by-decl just can't also be
        subscribed.
      - **Closure assertions** (tick / duration / dissolve /
        inline-violate / birth epochs). The substrate evaluates
        the assertion expression at the epoch boundary; there's
        no caller in the expression's frame to address a value
        error. Closures route failure via their own structural
        channel (the closure firing → `on_failure`), not a
        value channel.
    - **All other user-declared `fn` members** — can be
      `fallible(E)` just like free fns. Caller addresses
      with the standard surface (`or raise` / `or default` /
      `or handler(err)`); F.27's `violate` pattern stays as
      the canonical "kill the locus on this error" idiom and
      coexists with fallible member fns (caller picks).

    **Why this isn't a contract change:**

    - The substrate-orchestrated paths (lifecycle, bus,
      on_failure) keep the two-channel discipline — failure
      classification stays load-bearing where the substrate
      can't address a value return.
    - `@form(...)`-synthesized member fns are already fallible
      (`get` / `remove` / `key_at` / `entry_at` all return
      `fallible(...)`). The exception exists today; this
      generalizes it to user-declared `fn` members.
    - Cross-arena transfer reuses the existing
      `lotus_current_caller_arena` TLS routing free fns use.

    **Implementation scope estimate (revised 2026-05-25 after
    a scoping attempt):**

    - Typecheck: ~30 LOC — relax the rejection at the
      `LocusMember::Fn` arm; add a fallible-handler check at
      the bus-subscribe site (`info.bus_subscribes` loop in
      `check.rs`).
    - **Codegen: ~500 LOC** (much bigger than initially
      estimated). The locus-method declaration codepath
      (~line 11400 in `codegen.rs`) needs the same fallible-
      shape extension as free fns (sret slots, i1 return);
      `LocusInfo.user_methods` needs to grow from
      `BTreeMap<String, FunctionValue>` to a richer
      `MethodSig` that tracks fallibility per method (~6-10
      read sites across codegen need updating);
      `lower_self_method_call` + the cross-locus method call
      paths need to dispatch fallible-return shape and route
      through `or raise` / `or default` / `or handler`
      disposition lowering (the existing free-fn dispositions
      need to be parameterized over method-call vs free-fn-
      call). Tests: ~100 LOC.
    - Spec: `spec/types.md` two-channel rule section narrows
      to substrate-orchestrated paths.

    **Why the revised estimate:** the initial estimate
    (~150 LOC) was based on "reuse the free-fn fallible
    return shape." That's right for the LLVM ABI, but the
    plumbing to make the call-site dispatch KNOW a method
    is fallible (and emit the right disposition) is the
    bulk of the work — call sites currently look up
    `user_methods[name] → FunctionValue` and have no
    fallibility metadata to consult. Partial-shipping
    (typecheck-yes, codegen-error) would let users declare
    fallible member fns that crash at build time — worse
    UX than the current rule. Hold the rule until codegen
    plumbing lands as a coherent unit.

    **Open sub-questions before drafting:**

    - Can a closure assertion body **call** a fallible member
      fn (even though it can't itself be fallible)? Likely no
      — assertions are expression-shaped and `or raise` /
      `or default` / `or handler` are statement-position
      dispositions that don't compose cleanly inside an
      assertion expression. The expected pattern: factor the
      value-error path out of the closure into a regular
      member fn or free fn, and let the closure assert over
      pre-computed locus state.
    - Does this interact with cross-pool calls on
      `@form(...)`-bearing receivers? The fallible return
      value would need to cross the pool boundary; the
      caller-arena routing handles that today for synthesized
      methods, so it should generalize.

    **Empirical evidence to collect before landing:**

    - Specific friction-log call sites that motivated this.
      The proposal lands cleaner if it can quote the
      worst-offending patterns and show the post-change shape.
    - Whether the F.27 four-piece pattern is being USED in
      practice or whether devs are already routing around it
      via free fns. If the latter, the language is paying the
      cost without getting the benefit.

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

    **Resolved at the source-vendoring layer (F.19, F.25):**

    - **F.19, 2026-05-11.** Within one seed (one directory),
      every `.hl` file shares a top-level scope — same shape Go
      gets from per-package visibility. `hale build <dir>`
      bundles the directory; no `import` needed for in-seed
      cross-file refs.
    - **F.25, 2026-05-13 (v1.x-IMPORT).** Cross-seed imports
      ship: `import "<path>" as <alias>;` with required alias.
      Resolution order: entry-relative single file →
      entry-relative dir → workspace-root dir. No implicit
      `lib/` prefix. Library decls are auto-mangled with
      `__lib_<alias>_<stem>_<name>` and resolved through a
      per-build path-rename table parallel to the static stdlib
      / moa tables. See `spec/projects.md` and `spec/design-
      rationale.md` F.25.

    **Still open:** package registry, fetch, versioning,
    lockfile. v1 is vendor-the-source only — the source IS the
    dependency. A real package manager waits on concrete
    friction (version skew across projects, duplicate sources
    on disk, manual update toil); not before.

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
