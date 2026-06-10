# Static verification surface

This page is the canonical catalog of the compile-time **checks** the
toolchain runs beyond ordinary type-checking — the structural and
semantic guarantees a program earns at `hale build` / `hale check`
time. It describes shipped behavior; the roadmap of *candidate* future
checks lives in GitHub issue #18, not here.

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

## Not yet offered

Item 4 (bus-graph property checks) is fully landed. The remaining GitHub
issue #18 candidates are partial or unstarted, and none is a default gate
— don't assume them in a build:

- **Memory-bound proofs (item 1)** — the analysis exists (per-method
  allocation summary + call-graph escape/loop dataflow) and emits
  unbounded-accumulation warnings, but **opt-in** via `hale check
  --warn-unbounded-alloc`; not on by default. Type-aware String-concat
  sites + loop-ranking are unfinished. See `notes/memory-bound-proofs.md`.
- **Resource-budget tracking (item 5)** — a static **count** of pinned
  threads / cooperative pools / bus subjects via `hale check
  --dump-resource-budget`; informational only, no ceiling gate or fd-leak
  detection yet. See `notes/resource-budgets.md`.
- **Closure-assertion lifting (item 3)** — unstarted. Closures still
  verify their invariants at *runtime*.

Nothing here yet *proves* allocation, fd, or thread bounds as a
build-failing gate; the item-1 warnings are advisory.

Item 2 (race-completeness for substrate primitives) is a *substrate*
quality bar, not a user-facing check: it model-checks the runtime's own
concurrent primitives under all C11 interleavings. A proof-of-concept
has landed (the lockfree hashmap's enter/drain/grow protocol, verified
exhaustively with GenMC) — see `verification/`. It is not yet a CI gate
across every primitive.
