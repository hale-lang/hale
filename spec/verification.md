# Static verification surface

This page is the canonical catalog of the compile-time **checks** the
toolchain runs beyond ordinary type-checking — the structural and
semantic guarantees a program earns at `hale build` / `hale check`
time. It describes shipped behavior; the verification roadmap that drove
these checks — now delivered — is recorded in GitHub issue #18 (closed).

Two severity levels exist:

- **error** — fails the build (`Diag::is_error()` is true).
- **warning** — surfaced but non-fatal; the only non-error diagnostic
  Hale emits. Used where the flagged shape is a real smell but can be
  legitimate, so the call is left to the author.

Most checks run in the bundle-level passes of
`crates/hale-types/src/check.rs` (`check_bundle`); a few resolve-time
ones run in `crates/hale-types/src/resolve.rs`; cell slot-of-origin is
a codegen-time check. Each entry names the enforcing pass.

## Concurrency & placement safety

The bus + cooperative-pool model is the substrate; these checks keep a
program's placement coherent with how the runtime dispatches.

| Check | Catches | Severity | Enforced by |
|---|---|---|---|
| **Single-threaded-method invariant** | a *direct* cross-pool method call (`self.field.method()` where `field` is placed on a different pool) — it would run the callee's method on the wrong thread | error | `check_placement_single_thread` |
| **Dead bus receiver** | a non-`main` cooperative locus that subscribes to the bus *and* makes a blocking call in `run()` — the blocking call monopolizes the pool thread so the dispatch never delivers and its handlers never fire | error | `check_cooperative_pool_blocking` |
| **Blocking call on a cooperative pool** | a blocking `run()` (`recv`/`accept`, `process::run`) on a pool that isn't `where async_io` — it holds the pool's OS thread and stalls co-scheduled loci. Follows the call graph: blocking reached through a helper fn or `self.method` is flagged too | warning | `check_cooperative_pool_blocking` |
| **Cooperative pool starvation** | two or more loci on one cooperative pool (not `where async_io`) whose `run()` bodies statically never return (terminal `while` with no exit — `while true`, `while !self.draining`, or a never-assigned Bool flag) — the pool runs each `run()` to completion in birth order, so the later `run()` bodies never start. Covers fields with no placement entry (they default to pool `main`) and the main locus's own `run()`, which begins only after params-init | warning | `check_cooperative_pool_blocking` |
| **Nested long-running child** | a non-`main` locus holding a params field of a locus type whose `run()` doesn't return — the canonical fix is hoisting it to a `main` sibling with its own placement | error | `check_nested_long_running_child` |
| **Unowned subscriber locus** | a bus-subscribing locus instantiated *non-owned* inside another locus's method/handler body — it dissolves at that scope's exit, so its subscription can never fire (overridable with `--allow-unowned-subscriber`) | error | `check_unowned_subscriber_locus` |

The dead-receiver error is deliberately **direct-call-only** (its
call-graph surface is not widened), while the blocking *warning* is
interprocedural — the high-stakes diagnostic stays precise. See
`spec/semantics.md` type-check rules 7–8 and
`docs/src/services/concurrency.md`.

## Bus-graph property checks

The bus topology is a typed directed graph in the source; these walk
it. (GitHub issue #18 item 4.)

| Check | Catches | Severity | Enforced by |
|---|---|---|---|
| **Orphan topic / subject** | a declared `topic` or literal subject wired to only one end — published with no subscriber, subscribed with no publisher, or used by neither | warning | `check_bus_graph` |
| **Cross-locus bus cycle** | a publish→subscribe→publish loop spanning ≥2 loci — the cell hops via the cooperative queue and can spin / livelock | warning | `check_bus_cycles` |
| **Intra-locus re-entrant cycle** | an *unconditional* self-republish loop within one locus — intra-locus self-dispatch is a direct synchronous call, so it recurses on one thread without bound (stack overflow) | error | `check_bus_cycles` |
| **Bus backpressure** | a publish inside an unbounded `while true` loop with no flow-control or exit point (`yield` / `sleep`/`tick` / input-pacing `recv` / `break`/`return`) — floods the bus without bound | warning | `check_bus_backpressure` |
| **Subject type-mismatch** | two sites on the same literal subject string declaring different `of type` payloads — a subscriber would decode the wrong type | error | `check_bus_subject_types` |
| **Routing-key fallback rules** | an `on_unmatched: fallback` topic with no `where key == _` subscriber, or a `where key == _` filter on a non-fallback topic | error | `check_phase3_fallback_subscribers` |
| **Topic parent-chain cycle** | a topic hierarchy that loops (`topic A : B; topic B : A`) | error | `finalize_topic_chain` (resolve) |

Orphan detection is **closed-world gated** (it runs only when a `main`
locus is present), and suppressed by transport bindings, `**` wildcard
coverage, cross-seed (`alias::Topic`) references, and self-pub/sub —
so library seeds and external peers aren't falsely flagged. The
intra-locus cycle error counts only *unconditional* sends as edges: a
self-republish guarded by `if`/`match`/loop is a terminating state
machine, not unbounded recursion, and is left alone. See
`spec/semantics.md` type-check rules 9–10.

## Claims — domain requirements as checked sentences (GH #382, phase 1)

Bundle-level, **named** sentences over the program graph, declared in
the main locus and evaluated in `hale check` as **errors** — never
advisories (an advisory claim reads as law and doesn't bind, the #354
fail-open shape). The motivating property is multi-tenant isolation:
"no path from domain A to domain B" as one declaration with a name a
contract can cite, instead of per-fn `@effects` contracts scattered
across every position with completeness by hope.

```
group delta_wing = { delta::*, DeltaStore };
group gamma_wing = { gamma::Research };

main locus Org {
    params { ... }
    claims {
        iso_dg: forbid reaches(delta_wing, gamma_wing);
        iso_calls: forbid reaches(delta_wing, gamma_wing) via { calls };
        no_spend: forbid reaches(gamma_wing, effects(money));
    }
}
```

- **Groups are declared vocabulary, never patterns.** A `group` names
  a set of declared program elements (loci, free fns, imported
  decls). An unknown member is an **error, not an empty set** — the
  misspelt-effect-class lesson applied at the group layer — with a
  did-you-mean. An empty group is a **vacuity error** unless it opts
  out with `may_be_empty`: a `forbid` trivially satisfied by an empty
  quantification domain is a fail-open wearing formal clothing. The
  only glob is `alias::*` — trailing-only enumeration of an imported
  seed's decl set via the same rename table codegen resolves
  `alias::Name` through, deliberately mirroring the trailing-`**`
  rule for bus subjects. Qualified members (`alias::Name`)
  canonicalize at the mangle stage (#334's path), never by
  name-suffix matching.
- **`forbid reaches(SRC, DST)`** — absence under closure: no path
  from any element of SRC to any element of DST. Evaluation is
  fn-grained: a locus member projects to all of its methods,
  lifecycle hooks, and modes, which only ever *adds* sources and
  sinks (the conservative direction). Edges are the resolved call
  graph (stdlib bodies merged, handle-method calls resolved) and the
  declared bus graph (a publish site composes with every subscriber
  of its subject, wildcard subscribers included). `via { calls }` /
  `via { bus }` restricts which relations compose; the default is
  the **full composition** — more edges, conservative. `bus`
  composes publish sites in the visited fn's own body; the default
  (with `calls`) is the sound transitive closure.
- **`effects(<class>)`** in target position: the declared carriers
  of an effect class (an `@effects(is: {…})` frontier entry or a
  classified leaf), composed-class masks included. An undeclared
  class in claim position is an error with a did-you-mean.
- **Witnesses.** A violation renders a minimal countermodel path in
  author spelling — `` `delta::Triage::on_task` -(publishes
  "org.metrics")-> `gamma::Research::on_metric` `` — cross-seed
  symbols demangled. One witness per claim (the minimal
  countermodel, not an enumeration).
- **Unknown ⇒ violation.** An indirect call (function-typed
  parameter, #353) or a computed publish subject on a path from a
  `forbid` source cannot be certified and is reported as a
  violation, exactly as `@no_syscall` treats the same shapes.
- **Placement.** `claims { }` is only legal inside `main locus`
  (parse error elsewhere): main is the closed-world gate, so
  bundle-wide claims cannot be evaluated anywhere earlier, and
  one-main-per-bundle makes the claims root unique. Claim names are
  the contract-of-record and must be unique.

The remaining verbs (#382 phases 2–5):

- **`only edges A -> B { publish T; subscribe T; }`** — isolation
  with an exhaustive grant enumeration: every DIRECT edge from A to
  B must match a granted line, and every un-granted edge is
  reported (the grant list is the review surface, so the full diff
  matters). `publish T` and `subscribe T` admit the same bus edge —
  the verb names which end's declaration is the reviewable line.
  Call edges are never grantable: a direct call across the boundary
  is always an un-granted edge. Transitive paths through third
  parties are `forbid reaches` territory; a subscriber outside B
  (the `log.**` sink shape) is not an A→B edge and needs no grant.
- **`bound C <= N on paths from G`** — `@budget`'s per-call
  semiring behind a claims surface: total sites of user class C
  reachable per invocation (a call-tree SUM, exactly
  `@budget(C = N)`'s semantics — two calls to a carrier are two
  sites). A recursion cycle, loop-nested carrier, indirect call, or
  computed publish subject is unbounded and violates. The witness
  carries the count and a representative chain. Built-ins keep
  their `@budget` spellings.
- **`require subscribes|publishes(some G, topic T)`** — existence
  over the DECLARED bus ends (the `bus { }` blocks — "wired" is a
  declaration property).
- **`cover topic in seed(a): subscribed_by(some G)`** — bounded
  universal: every topic the seed imported as `a` declares has a
  subscriber in G. Every uncovered topic is named. A seed with no
  topics is an error at the claim (an empty coverage domain holds
  vacuously).
- **`count publishers|subscribers(topic T) ==|<=|>= N`** — the
  cardinality family over distinct loci; `== 1` is the invariant
  behind every single-writer pattern, and a violation names the
  competing writers.
- **`during P`** on `forbid` — restricts sources to the named
  lifecycle phase / method of each source locus (`during birth` is
  the quiet-boot claim). A phase naming nothing in the group is an
  error, not a vacuously-holding claim.
- **`avoiding G`** on `forbid` — masks G's vertices out of the
  walk, which makes it the interposition form: "every path from A
  to B passes through the gate" is `forbid reaches(A, B) avoiding
  gate`.

**Indexed effect families** (#382 phase 3): `domain wing = { delta,
gamma };` declares a closed index domain; `effect knowledge(wing);`
declares a family. Every instantiation `knowledge(delta)` interns
as an ordinary declared class and `knowledge(*)` as an
auto-populated composed class over all of them — the whole feature
is a reduction onto shipped machinery (masks, `only:` complements,
cross-seed remap, did-you-mean), so a misspelt index is an
undeclared-class error and a domain member added later lands
OUTSIDE every existing `only:` contract (#354's fail-closed,
inherited rather than re-derived). The domain must be declared
earlier in the same file as the family. Companion:
`@budget(<user class> = N)` bounds calls to declared carriers of a
class along any path, with the same loop/indirect unboundedness
rules as every per-call dimension.

**The topology artifact** (#382 phase 2): `hale check <t>
--dump-topology` emits the serialized model — sorts (loci, fns,
topics), relations (calls, publishes, subscribes), the declared
**groups**, the effect **labels** (declared carriers), and the
**unknowns** (fns with indirect calls or computed publish subjects
— the places evaluation failed closed), all in author spelling —
plus every named claim's normalized form and result, under a
schema version and a `shape_hash` (FNV-1a/64 over the canonical
model half, which includes groups/labels/unknowns; claim RESULTS
are excluded, so one topology under different law keeps one
shape). `--check-topology <path>` diffs against a committed
baseline and fails with a regenerate hint — the `.hale.effects`
precedent: an unreviewed topology or law change fails CI the way
an API break does. v1 scope, stated honestly: this is enough to
independently re-evaluate the reachability-class claims and audit
where certification stopped; it is not yet the complete normalized
verification model (no per-edge spans, weights, phase relation, or
seed sort — that export is the architectural milestone tracked on
#382). The derivation (source → model) remains the trust root.

Still later (#382): library-tier claims that travel with imports,
and the annotations-lower-to-claim-IR unification (§8 of the
issue) once the claim IR has survived real use.

## Structural & design rules

| Check | Catches | Severity | Enforced by |
|---|---|---|---|
| **CQRS / no-locus-return** | a locus `fn` member whose return type (or `fallible(T)` payload) names a user-declared locus type — returning a managed entity from a method is a Law-of-Demeter / CQRS / Dependency-Inversion violation that also leaks via payload-arena routing | error | `check_no_locus_return` |
| **Stdlib error-type shadow** | a user-declared `type IoError` / `ParseError` / `CryptoError` / `IndexError` / `KeyError` / `EmptyError` whose shape doesn't match the stdlib's, when that error type is reached by a fallible stdlib call | error | `check_stdlib_error_shadowing` (resolve) |
| **Codec purity** | a bus codec whose `encode` / `decode` method isn't pure (codecs may be dispatched off-thread) | error | `check_main_and_bindings` + `purity::infer_purity_for_bundle` |
| **`ring_layout` contract** | a foreign-ring layout declaration that's internally ill-formed — unknown scalar/`len_prefix` repr, missing `framing` (or `byte_records` without a `len_prefix` / `buffer_size`, or `slots` without `slot_size` / `slot_count`), no cursor / a cursor without an `at`, unknown cursor ordering or unit, a missing `magic` / `data_at`, or a `shm_ring(..., layout: N)` whose `N` doesn't resolve to a declared `ring_layout` | error | `check_ring_layout` + `check_main_and_bindings` |
| **`ring_layout` geometry** | a *cross-field* inconsistency that would let a record header land out of bounds or silently corrupt the reader: a header scalar or the cursor overrunning `data_at`, two fields overlapping, a non-power-of-two `align`, a `pad_sentinel` too wide for the `len_prefix`, a `len_prefix` width `> align`, a non-8-aligned `atomic_u64` cursor, or (producer side) a `buffer_size:` that isn't a multiple of `align` | error | `check_ring_layout` + `check_main_and_bindings` |
| **Foreign-ring payload shape** | a `layout:`-bound topic whose payload is neither flat-shapeable (typed mode — read by direct cast, needs a fixed byte layout) nor `BytesView` (raw-frame mode — a bounded view per record, for heterogeneous rings); e.g. a struct with `String` / `Bytes` / variable-size fields. Enforced regardless of `where zero_copy` | error | `check_main_and_bindings` |
| **Cell slot-of-origin** | releasing a `Cell<T>` into a different `(locus, slot)` than it was acquired from | error | codegen |

CQRS is GitHub issue #18 item 6; its three sanctioned remedies
(parent-child + contract, bus mediator, delegation) are named in the
diagnostic. See `spec/semantics.md § Locus method dispatch`.

## Default-on & opt-in analyses

One GitHub issue #18 analysis runs **by default**: item 4 (bus-graph property
checks — *errors*, fail the build). The rest, including item 1 (memory-bound),
are **opt-in** (behind a flag) or deferred. Only item 4 is a build gate; don't
assume the others in a build:

- **Memory-bound proofs (item 1)** — **opt-in**, two ways. The proof is
  opt-in by design: "bounded per epoch" only means something for a
  long-lived process (a daemon, a bus handler, a persistent locus), so a
  script that allocates and exits owes it nothing and pays nothing by
  default — the same descent-curve stance as the `@locality` cache-tier
  budgets (annotation/flag-gated, never automatic). The two opt-in surfaces:
  - **`@bounded locus L { … }`** — the in-source opt-in. A locus annotated
    `@bounded` is checked on every `hale check` (no flag), and a
    `@unbounded fn`/`@unbounded run { … }` inside it is the greppable
    carve-out that silences one body's sites. This is the descent marker:
    the locus that took on long-lived state asks for the proof on itself.
    *(Currently advisory warnings; the intended end state is a hard
    **error** contract once the precision refinements — store-latest vs.
    append, `@form(cap)` composition — drive in-scope false positives to
    zero.)*
  - **The whole-program advisory survey — DEFAULT-ON since 2026-07-02**
    (the M3 stage-5 flip, after a full-corpus audit triaged all 402
    warnings: every true positive preserved, every residual false positive
    in a documented accepted class — see
    notes/unbounded-alloc-audit-2026-07-02.md). Flags every site
    regardless of `@bounded` (a `@unbounded` fn is still suppressed);
    run-to-exit programs (a `main` with no `run` loop and no bus handler)
    warn nothing — a script owes the proof nothing. Warnings print but
    never fail the build. **`--no-warn-unbounded-alloc`** is the opt-out;
    `--warn-unbounded-alloc` is accepted-and-ignored (the former opt-in
    spelling).
  `--dump-alloc-summary` prints the raw per-fn summary. A per-method allocation summary + call-graph
  escape/loop dataflow — with **escape-awareness** (a non-escaping local in
  a per-message handler is reclaimed at the per-delivery method-scratch
  destroy, so it isn't flagged), call-result escape tagging, and
  **loop-ranking** (a `while v < N` const counter is proven bounded) — flags
  a value allocated in a per-message handler / unbounded loop that escapes
  and **accumulates until the locus dissolves**. A whole-value replace
  (`self.f = Struct{…}`) genuinely leaks (the arena bump-allocates a fresh
  value each time); the fix is **in-place mutation** (`self.f.x = v` /
  `self.a[i] = v`), a capacity-bounded `@form` (`ring_buffer` / `lru_cache`
  / a `capacity` slot), the bus (reclaims per dispatch), or a per-iteration
  child locus. It also flags an **insert into a growing collection** —
  `v.push(x)` / `m.set(x)` where the receiver's declared type is a
  `@form(vec)` / `@form(hashmap)` locus — in an unbounded context; the
  backing buffer grows with population and frees only at dissolve. A
  `@form(ring_buffer)` / `@form(lru_cache)` is cap-bounded and excluded.
  (Detection reads *declared* receiver types — params, typed `let`s, locus
  param fields — not inferred ones.) Zero corpus false positives. Type-aware
  String-concat sites and untyped-receiver collection inserts remain
  deferred. See `notes/memory-bound-proofs.md`.
- **Hot-path allocation contract — `@budget(alloc_per_call = N)`** (2026-07-16).
  The dual of `@unbounded`: where `@unbounded` acknowledges intentional
  unbounded allocation, `@budget` declares an *opt-in per-call ceiling* and
  the compiler **enforces it as a hard error**. On a `fn` (free or method),
  `@budget(alloc_per_call = N)` asserts the fn performs at most `N` arena
  allocations per call. The check reuses the item-1 allocation summary +
  call graph: it counts the arena-allocating literals / `@form` inserts it
  can see, **transitively through resolved (bundle-local) callees**, plus
  the known-allocating `recv` family (`recv` / `recv_bytes` /
  `recv_with_source` — the same set the hot-path lint flags); a
  loop-nested allocation, or a call to an allocating fn inside a loop, or
  recursion, is **unbounded per call**. `N = 0` is the zero-alloc
  certificate — the strongest form, for a per-datagram handler or decode
  helper the runtime calls on the hot path with a guarantee it touches no
  arena. Opaque calls other than the `recv` family are outside what the
  budget sees (the same boundary the escape analysis draws); pair the
  contract with `recv_into` + a reused `BytesBuilder`. fn-only; mutually
  exclusive with `@unbounded`. A violation reports the measured count and
  points at every offending allocation with the fast-path fix.
- **Effect assertions — `@effects(...)` and its `@no_*` sugar**
  (GH #265, 2026-07-29). `@budget`'s discipline generalized from
  allocation *count* to effect *classes*: an opt-in contract at a root
  the author cares about, inferred everywhere else, enforced as a hard
  error. **One surface, one engine, one classified frontier** — not a
  family of independent flags.

  The general form is `@effects(...)`:

  ```hale
  @effects(none: {syscall, block})   fn decode(b: Bytes) -> Msg { ... }
  @effects(none: {time})             fn backoff(n: Int) -> Int { ... }
  @effects(publish: {OrderFill})     fn route(o: Order) { ... }
  ```

  `none: {…}` forbids effect classes; `publish: {…}` declares the
  allowed publish set (exact, because the topic set is closed). The
  classes are `syscall`, `block`, `time`, `entropy`, `env`, `ffi`,
  `publish`, `spawn`, `recursion`.

  The `@no_*` family is **documented sugar**, desugared at parse time
  so the checker has exactly one shape to interpret (a flag can never
  drift from the general form):

  | sugar | means |
  |---|---|
  | `@no_syscall` | `@effects(none: {syscall})` |
  | `@no_block` | `@effects(none: {block})` |
  | `@no_ffi` | `@effects(none: {ffi})` |
  | `@no_publish` | `@effects(none: {publish})` |
  | `@no_spawn` | `@effects(none: {spawn})` |
  | `@no_recursion` | `@effects(none: {recursion})` |
  | `@deterministic` | `@effects(none: {time, entropy, env})` |

  All stack with each other and with `@hot` / `@budget(...)`, so the
  full hot-path certificate is one line:
  `@no_block @no_syscall @deterministic @no_recursion @hot
  @budget(alloc_per_call = 0)`.

  **Where each class's truth comes from.** `syscall` / `block` /
  `time` / `entropy` / `env` are queries against the **fully
  classified stdlib registry** — all 327 surface entries carry an
  `EffectSet` in `hale-types::stdlib_surface`, with zero unclassified
  residue (pinned by a test). The classification distinguishes
  *reading* an effect source from operating on a *supplied value*:
  `time_from_unix(n)` is deterministic while `monotonic_ns()` is not;
  `http::parse_request` is pure while `http::get` is blocking I/O.

  **Incompleteness fails closed, in both of its forms.** An entry
  present but *unclassified* is treated as may-do-anything and
  violates every assertion. So is a `std::` path with **no registry
  row at all** — the two used to be asymmetric, and that asymmetry
  was a soundness hole: an unclassified row failed closed while a
  whole unregistered namespace read as pure, silently certifying
  calls into it. "Absent" and "unknown" are the same claim, and
  neither can be certified.

  The **language builtins that write to a stream** (`println`,
  `print`, `eprintln`, `eprint`) are syscall-class. They are not
  `std::` paths, so they once sat outside the frontier entirely and
  a `@no_syscall` fn could print freely — while the diagnostic for
  `std::io::fs::*` described the syscall class as covering "stdio".
  Writing to a stream is a `write(2)`; it can block, and a hot-path
  certificate that permits it is not certifying what it claims.

  `ffi` matches
  bundle-local `@ffi` declarations. `publish` and `spawn` are
  **syntactic** — `Topic <- v` and `Child { … }` are effects the
  language expresses directly, recorded as effect *sites* on the
  summary rather than as call edges. `recursion` is a graph property.

  **Reachability follows handles, not just paths.** A call made on a
  value — `reader.slurp()`, `resolver.get(…)` — is a real edge, and
  the analysis resolves the receiver's declared type to walk into
  the method body. This includes the part of the standard library
  that is **written in Hale** (`hale-stdlib`): those bodies are fed
  to the callgraph, so their effects are *inferred from the
  implementation* rather than declared in a table that can drift.
  Witness paths through a stdlib locus are rendered in the public
  spelling (`std::cli::Resolver::get`), never the internal mangled
  name.

  This matters more than it sounds: the locus-with-methods shape is
  the idiomatic way to do I/O in Hale — the same shape a violation
  diagnostic recommends as the fix. Moving an effect behind a locus
  the asserting fn still calls does not make it unreachable, and the
  checker must not confuse the two.

  **Diagnostics carry the witness path** — the call chain from the
  asserting root to the offending leaf, which `@budget`'s fixpoint
  structurally could not produce:

  ```
  effect assertion violated: `on_tick` must not reach `block`, but reaches
    on_tick -> helper -> nap [std::time::sleep — a blocking operation …].
  ```

  plus a second diagnostic at the leaf itself.

  **Boundaries:** opaque callees outside the classified frontier are
  not seen (the same soundness boundary the escape analysis and
  `@budget` draw); `@ffi` labels are trusted, not verified; a computed
  publish subject cannot be proven in-set and is reported. `@no_panic`
  remains a separate track — disposition coverage + index-op
  selection, not leaf reachability.
- **Quantitative budgets — `@budget(<dim> = N)`** (GH #265 step 5,
  2026-07-29). `@budget` counts more than allocations now; dimensions
  compose in one clause, comma-separated:
  - `stack_bytes` — worst-case stack depth as a **DAG longest path**
    over estimated frame sizes. Acyclicity is the precondition, which
    is why this pairs with `@no_recursion`: a cycle reports
    *unbounded*.

    **What the estimate rests on** (#326, examined 2026-08-03). Frames
    are estimated from declared shapes: 32 bytes of call overhead, 8
    per parameter, 8 per local. That unit is close to right *because
    of Hale's memory model, not by luck* — fixed arrays, structs and
    string/bytes buffers are arena-allocated, so a local is a pointer
    and almost nothing but scalars is ever on the stack. The same
    estimator in C would be wrong by orders of magnitude. The premise
    is pinned by tests (`hale-codegen/tests/stack_budget_premises.rs`)
    precisely because it is load-bearing: if a shape ever became
    stack-allocated, the estimate would silently under-count by the
    size of that shape.

    Inlining, the other obvious worry, cuts the safe way: the model
    charges `CALL_OVERHEAD` per level of call depth and inlining
    removes levels, so it strictly reduces the term the model spends
    most of its budget on.

    **What the estimate does NOT cover.** Storage the optimizer
    introduces is invisible to any source-level model — register
    spills above all. We build `-O3` with `target-cpu=native`, so on
    an AVX-512 host a spilled vector register is **64 bytes** against
    a model whose unit is 8, and a vectorized loop can consume
    hundreds of bytes with no source local to explain it. Inlining
    amplifies this by merging live ranges, which is what produces
    spills — the risk is merged *pressure*, not merged locals.

    So: the bound is a **structural** estimate over declared shapes,
    not a machine-level guarantee and not WCET. It is sound against
    everything the frontend can see and silent about everything the
    backend adds. Treat it as a bound on *program shape* — call depth
    and declared locals — and not as a promise about bytes of hardware
    stack. Settling the latter needs post-codegen measurement
    (LLVM's `.stack_sizes` section), which is not wired up; `hale
    check`'s diagnostic already says "estimated from declared shapes",
    and this entry now matches that honesty rather than exceeding it.
  - `block_points` — how many blocking operations one call may reach
    (`0` is `@no_block`; `1` is "may wait once, on its own socket").
  - `publish` — publishes per call. `@budget(publish = 1)` **is** the
    exactly-once-reply contract the issue sketched as `@replies`,
    falling out as a count rather than a bespoke analysis.
  - `fanout` — transitive subscriber **deliveries** one call causes,
    read off the bus graph. This is the amplification/backpressure
    property no per-fn count reveals: a handler publishing to a
    200-subscriber subject amplifies 200×.

  A contributor inside a loop saturates to unbounded, matching
  `alloc_per_call`'s per-call semantics.
- **Phase-indexed effects — `@phase_effects(...)` on a locus**
  (GH #265 step 6, 2026-07-29). The lifecycle model expresses what a
  function-level effect system cannot:
  `@phase_effects(birth: {alloc}, run: {})` **is** the DO-178 "no
  dynamic memory after initialization" discipline, stated directly
  rather than assembled from two unrelated flags. Each phase names
  the classes it may perform (`alloc`, plus the `@effects` classes);
  a phase omitted is unconstrained, a phase with `{}` forbids
  everything. Phases resolve to lifecycle hooks (`birth`, `run`,
  `drain`, `dissolve`, `accept`, `release`) or to any member fn /
  handler by name.
- **`@no_panic` — disposition coverage** (GH #265, 2026-07-29).
  Deliberately *not* an effect class: this is a syntactic property of
  a body, not a query over the classified frontier. A fn asserting
  `@no_panic` must have no reachable trap — no explicit `violate`, no
  fallible expression dispositioned `or raise` (which propagates
  rather than handles), no trapping index form. `or discard`, a
  substitute value, and `or handler(err)` all satisfy it.
- **The conformance loop — checking the checker** (GH #265 step 7,
  2026-07-29). Static classification is a *claim*; the running
  binary is the oracle. `crates/hale-codegen/tests/
  effects_conformance.rs` compiles programs carrying assertions,
  runs them, and samples the runtime's own counters
  (`std::diag::syscall_count` / `heap_alloc_count`) around the
  certified call: a fn certified `@no_syscall` that performs a
  syscall, or `@budget(alloc_per_call = 0)` that allocates, is **a
  caught soundness bug in the analysis itself** — the one class of
  defect that "the checker says what I expect" testing cannot find.
  A negative control asserts the oracle detects the effect when it
  genuinely happens, so the conformance checks can never pass
  vacuously. Same philosophy as GenMC-in-CI, applied to effects.
- **The `.hale.effects` manifest + CI gate** (GH #265 step 7). The
  manifest is a **behavioural fingerprint**: every fn's declared
  contracts *and* its INFERRED effect set (`does={…}`), stable-sorted
  for diffs. `hale check <target> --dump-effects-manifest` writes it;
  `--check-effects-manifest <path>` diffs against a committed
  baseline and **fails the build** when the program's effects change,
  printing which fn gained or lost what. That catches the case
  annotations cannot: a handler that quietly starts doing filesystem
  I/O shows up as `+ Api::emit … does={syscall,publish,alloc}` in
  review even though no annotation changed. Regenerate deliberately
  with the dump flag when the change is intended.
- **Corpus-wide conformance** (GH #265 step 7). Beyond the per-test
  runtime oracle, a sweep over the whole in-tree `.hl` corpus asserts
  three properties everywhere real code lives: no reachable stdlib
  call lands on an **unclassified** registry row (an unclassified
  leaf silently weakens every assertion over it, so frontier
  completeness must not rot as the corpus grows); inference is
  **deterministic** (the same program inferred twice yields identical
  sets — otherwise manifests diff spuriously); and every corpus
  program carrying an assertion **satisfies** it.
- **The `.hale.effects` manifest** (GH #265 step 7). `effect_manifest`
  / `render_effect_manifest` emit the whole program's declared
  contracts in a stable, sorted line format alongside `.hale.topo`.
  Declaration-only at v1 (inferring a full effect set per fn is the
  effect-rows-on-function-types slippery slope the issue defers);
  what it buys today is that an effect **regression** — a handler
  that quietly lost a contract — shows up as a one-line diff in
  review, the way an API break shows in a `.d.ts` diff.
- **Cross-actor causality — `@effects(causes: {…})`** (GH #265,
  2026-07-29). The call graph stops at a publish; the **bus graph
  continues**. Because Hale's message graph is declared over a closed
  topic set, "this handler, by publishing `orders`, can transitively
  cause a filesystem write in the audit subscriber" is a checkable
  property — the compiler walks publish sites → subject → each
  subscriber's inferred effect set. The diagnostic names the causal
  path (`Api::handle -> subject Orders -> Audit::on_order`). Only
  effects reached THROUGH the bus are reported; direct effects are
  the `none:` form's job. Actor systems without a declared message
  graph structurally cannot offer this.
- **Backward causality — `@effects(depends: {…})`** (#330,
  2026-07-31). The dual of `causes:`. `causes:` walks the bus graph
  forward; nothing walked it backward, so an independence claim
  between two parts of a bus graph was unenforceable — a dependence
  routed through one republishing intermediary is invisible in every
  declaration on the depending locus, whose `bus {}` block names only
  the innocent subject it directly subscribes to. `depends:` is a
  COMPLETE declaration of the subjects that may transitively reach any
  of the locus's handlers, and the diagnostic names the path
  (`subject SumLookup -> Launderer -> subject Recalled ->
  StatedCarry`). **Locus-level**: dependence enters through
  subscriptions, which are declared per-locus, so a fn-level
  `depends:` is a parse error rather than a silent no-op. Opt-in on
  measured grounds — over a real application (428 topics, 114 loci)
  transitivity adds nothing beyond the `bus {}` block for 87% of loci,
  so a mandatory form would be redundant far more often than
  informative. The closure is over the **bus graph**: influence
  travelling outside it (see shared state below) is not part of it.
- **User-declared effect classes — `effect NAME;` and
  `@effects(is: {…})`** (#345, 2026-08-01). A program may name its own
  effect classes and have them propagated by the same engine, with the
  same witness paths:

  ```
  effect money;

  @effects(is: {money})
  fn charge(cents: Int) -> Int { … }

  @effects(none: {money})
  fn price(n: Int) -> Decimal { … }   // violates if it reaches charge
  ```

  Grounded exactly like a built-in. The objection to user effects is
  that they have no frontier — that `@no_money` could only mean "no fn
  somebody remembered to annotate". But effects worth checking are
  about interaction with the outside, and that IS the frontier: money
  moves when the processor is called, the ledger row is written, the
  settlement is published. So `is:` adds rows to the classification
  that already exists rather than introducing a second kind. **The
  compiler owns propagation; the program owns classification** — the
  same split the stdlib registry has, with a different owner.

  Classes are interned as indices and occupy the free bits above the
  ten built-ins — `EffectSet` is a u64, so 54 are available.
  Overflow is an **error at the declaration**, not a saturating
  no-op: a class with no bit unions as PURE, so `none: {…}` on it
  would certify a fn that reaches a declared source. The analysis
  fails closed everywhere else and must here too.
  Classes cross seed boundaries. Each seed interns its own names from
  zero, so the merge unions the tables and rewrites each seed's
  indices before concatenating items — without that, two seeds' class
  0 share a bit and a `none:` on one is checked against the other.
- **An indirect call fails closed** (#353, 2026-08-03). A call
  through a function-typed parameter reaches the call graph as an
  unresolved callee, indistinguishable from a call to an unknown free
  fn — which contributed nothing. So `@no_syscall` on a fn whose body
  is `return f(v);` typechecked while the program performed the
  syscall, and `@budget(alloc_per_call = 0)` leaked identically. Every
  certificate the language offers ran through one hole, function
  pointers being the first genuinely open-world construct in the
  language. An indirect call is now treated as **may do anything**:
  unclassified for the effect classes, unbounded for the quantitative
  dimensions. Deliberately conservative rather than exact — Hale is
  whole-program and closed-world, so the target set IS enumerable and
  exact resolution is possible, but a certificate wrong in the safe
  direction beats one wrong in the other, so the conservative form
  lands first and precision becomes an improvement rather than a
  correctness fix.
- **Closed effect contracts — `@effects(only: {…})`** (#354,
  2026-08-03). The dual of `none:`. `none:` forbids a listed set and
  permits everything else, which makes it **rot**: expressing "this
  handler only allocates" requires enumerating every other class, and
  adding a class to the language silently widens every such contract —
  the annotation still reads "only alloc" and no longer means it.
  Nothing fails; the certificate quietly weakens. `only:` states the
  permitted set and is checked against the **complement computed at
  check time** from the live class universe (the ten built-ins plus
  every declared user class). Nothing is written down that can go
  stale, so a class declared after the contract was written is outside
  it automatically. Rendered separately from `none=` in the manifest —
  a reader must be able to tell a closed contract from an open one,
  because they age differently.
- **Composed effect classes — `effect NAME = { A, B };`** (#354,
  2026-08-03). A class may be DEFINED as the union of others. A
  composed class owns **no bit**; its mask is its members', which
  yields both useful directions with no additional analysis:
  forbidding `io` tests against `syscall|block` and catches either,
  and a fn that reaches a syscall carries `io`. This also repairs a
  fail-open in `@deterministic`, which desugars to a hardcoded
  `none: {time, entropy, env}` and therefore could not see a
  user-declared class: `effect wallclock = { time };` puts the time
  bit in the class's mask, so the existing contract catches it with no
  new mechanism — and it stays opt-in, since an atomic class like
  `money` is correctly *not* swept into determinism. A definition
  cycle resolves to no effect at all, so every contract naming it
  would hold vacuously; cycles are rejected.
- **Synchronized access is blocking and non-deterministic** (#340/#341,
  2026-08-01). A `sync`-bearing form takes a lock, so a call reaching
  one contributes `block`, and a read through one defeats
  `@deterministic` — another pool can change the value between two
  calls with identical arguments. Attributed because **placement is
  not static**: once placement can be swapped at runtime, whether a
  mutex ever contends is undecidable at compile time, so a certificate
  reading "never blocks, we are single-pool today" would be
  invalidated by a later swap. A form with no `sync` discipline takes
  no lock and stays certifiable.
- **Supervision coverage — `@supervised`** (GH #265, 2026-07-29). A
  locus marked `@supervised` asserts that every locus in its subtree
  has a failure policy in scope — an `on_failure` on itself or an
  ancestor. A locus with children and no policy above it is reported
  by name: a failure there has nowhere to go. The declared ownership
  tree makes this a tree walk; it is the static supervision-coverage
  property the actor world has wanted for decades.
- **Coarse secret taint — `@secret` params** (GH #265, 2026-07-29).
  A parameter declared `@secret name: T` must not reach a bus publish
  or a log / file sink. Deliberately **coarse** — parameter-granular,
  not value-flow-complete — which the issue asked be assessed rather
  than deferred to a "v2 horizon"; the choke-pointed sinks make even
  this catch the mistake that matters (a key or token in a log line
  or on the wire). Constant-time / full information flow remains out
  of scope: value-dependent control flow and microarchitecture are a
  different shape of analysis.
- **Inferred effect sets + symbolic cost** (GH #265, 2026-07-29).
  `frontier::infer_effects` computes the transitive effect set of ANY
  fn — no declaration required — which is what feeds the causality
  check and lets the `.hale.effects` manifest report inferred sets
  alongside declared contracts. (Effect rows on function *types*
  remain the deferred slippery slope; a manifest is a report, not a
  type.) `cost_expression` renders a structural cost —
  `O(n^k)` in nesting depth with a step estimate — explicitly **not
  WCET**: the first-filter triage shape, meaningful only for a fn
  already proven bounded.
- **Placement-implied contracts — the assertion you don't write**
  (GH #265, 2026-07-29). A locus placed
  `cooperative(pool = X) where async_io` shares that pool's single
  worker; a blocking operation reachable from one of its handlers
  holds the worker and stalls every other locus on the pool. Since
  the placement already declares the intent, the compiler enforces it
  **with no annotation at all**: an unannotated handler on an
  async_io pool that reaches a blocking call gets a warning naming
  the chain and both fixes (move the work to its own pool, or assert
  `@no_block` to have it enforced as an error). Advisory rather than
  hard, because a lone locus owning its pool may block deliberately;
  writing an explicit assertion suppresses the advisory (the author
  is engaged, and the enforced error replaces it). This is the class
  of bug that shipped as a downstream latency mystery — a sleeping
  handler holding an engine pool — now visible at compile time.
- **Hot-path allocation lint — default-on advisory** (2026-07-16). Two
  loop-scoped anti-patterns get a **warning** (never a build failure), so
  the allocation-free shape is the path of least resistance rather than
  expert folklore: (1) a **locus** (its own arena / heap buffer) or a
  `std::bytes::BytesBuilder` instantiated inside a loop — hoist it to a
  reused field; (2) an **allocating `recv`** (the `recv` family) in a loop
  — use `recv_into` with a reused `BytesBuilder`. Both accumulate in the
  method scratch until the enclosing method returns, and a `run()` read
  loop never returns. Loop-scoped keeps the signal clean (per-iteration is
  the unambiguous case); a plain value struct/type literal is not flagged
  (only loci and heap-bearing builders), and a per-invocation
  instantiation outside a loop reclaims at method exit. This is the
  conservative default advisory; `@budget` is the strict opt-in contract
  built on the same intent.

  Gap D extensions (2026-07-17): (3) a locus / `BytesBuilder`
  instantiated **anywhere in a bus handler** (not just a loop) — a
  handler runs per message, so a per-call instance is the
  ~4.5 KB/frame class; hoist it to a reused field. (4) **`accept`
  without `release` on a daemon-shaped locus** — declaring
  `release(c: C)` marks `C` a flow child (reclaimed when its `run()`
  completes); without it every accepted child is RESIDENT until the
  parent dissolves, so a parent whose `run()` loops forever (literal
  `while true` — the deliberately narrow daemon signal) grows
  O(accepted children). Run-to-exit accept examples stay silent.
- **`@hot` — hot-path certification** (Gap D, 2026-07-17). The layered
  escalation between the default advisory and `@budget`'s counted
  ceiling: `@hot fn` certifies "this is a 10k/s-class path" and (a)
  **promotes the hot-path lint's findings inside that fn to hard
  errors** (prefixed `@hot:`), and (b) enables two stricter,
  perf-only hints that would nag as defaults: `.snapshot()` /
  `.finish()` in a loop or handler (each call copies the builder's
  full contents — prefer the zero-copy `.view()` / `.text_view()`),
  and a whole-struct replace of a direct self-field (post-Gap-A the
  replaced String clones retire, so this is no longer a leak — but
  each store still pays a clone + retire per heap field where
  in-place scalar mutation is allocation-free). fn-only; stacks with
  a following `@budget(...)`:
  `@hot @budget(alloc_per_call = 0) fn send(...)`.
- **Anchor-retirement verdict flip** (Gap D, 2026-07-17). The item-1
  survey's model learned what Gap A's runtime now does: a whole-field
  `self.<f> = Struct { ... }` replace of a struct whose fields are all
  scalar / `String` reclaims at the enclosing method's activation
  boundary (the struct bytes memcpy in place; replaced String clones
  retire and recycle — RSS-validated flat over 1M replaces), so such a
  site invoked unboundedly is no longer reported. The conservative
  verdict stays for: structs with `Bytes` / nested compound / array
  fields (those leaves don't retire yet), stores directly inside a
  `run()`-loop (no activation boundary — pending retires never
  flush), and scratchless owners.
- **Resource-budget tracking (item 5)** — fully shipped, opt-in. A static
  **count** of pinned threads / cooperative pools / bus subjects /
  fd-acquisition sites (fd-opening calls *and* held-fd `Listener` /
  `Stream` instantiations) via `hale check --dump-resource-budget`; a **CI
  ceiling gate** `--check-resource-budget <file.toml>` (fails the build
  when a count exceeds a declared ceiling); and **fd-leak detection**
  `--warn-resource-leak` (an fd-acquiring call whose result is stored
  resident in an unbounded context). See `notes/resource-budgets.md`.

  The ceiling file is TOML; every key is optional (an absent key leaves
  that resource unconstrained, an unknown key is an error):

  ```toml
  pinned_threads    = 4
  cooperative_pools = 2
  bus_subjects      = 16
  fd_open_sites     = 8
  ```
- **Closure-assertion lifting (item 3)** — scoped, deliberately parked.
  The tractable case (constant assertions) is already handled: typecheck
  rejects any closure whose assertion observes no runtime-varying value
  (pure literals *or* const arithmetic), so there are no constant closures
  to lift. The only liftable closures are ones provable from producer
  arithmetic (symbolic execution) — low-leverage for a niche feature, not
  built. Closures still verify their (runtime-observing) invariants at
  *runtime*. See `notes/closure-lifting.md`.

The item-1 whole-program survey and the hot-path lint are advisory
(warnings). The one build-failing *allocation* gate is opt-in:
`@budget(alloc_per_call = N)` on a fn — you ask for the ceiling, and a
violation is a hard error. fd and thread bounds remain advisory / CI-gated
(item 5), not automatic build failures.

Item 2 (race-completeness for substrate primitives) is a *substrate*
quality bar, not a user-facing check: it model-checks the runtime's own
concurrent primitives under all C11 interleavings with GenMC, run as a
standing CI gate (the `genmc` job). Every substrate primitive with a
cross-thread synchronization surface is now modeled: the lockfree
hashmap's enter/drain/grow protocol, the pinned-locus mailbox monitor,
the cooperative-pool bus queue's conditional lock, and the arena
subregion-slot freelist lock. (The per-thread chunk pool needs no model
— it is `__thread`, with no cross-thread access.) See `verification/`.
