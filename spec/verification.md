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
