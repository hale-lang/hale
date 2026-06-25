# Memory-bound proofs (GH #18 item 1)

Status: **step 1 (the dataflow scaffold) landed; bound-proving not started.**
Written 2026-06-09. The next unstarted GH #18 candidate per the issue's own
ordering, and the one that builds the shared compile-time dataflow
infrastructure that items #3 (closure-lifting) and #5 (resource-budgets)
reuse.

> **Step 1 landed (#100):** `crates/hale-types/src/alloc_summary.rs` — the
> per-method allocation summary + call graph, behind `hale check
> --dump-alloc-summary`. Records allocation sites (escape-tagged, loop
> depth), call edges (resolved/unresolved), loops (bounded/runtime), entry
> multiplicity. The reusable IR for steps 2-4 (and items #3/#5).
>
> **Step 2 landed (the reclamation model):** each site now carries a
> `ReclaimScope` + a `SiteVerdict` (`OncePerInvocation` /
> `PerIterationReclaim` / `AccumulatesBoundedLoop` / `AccumulatesUnbounded`),
> the dump flags the last as `LEAK PRECURSOR`, and an RSS test
> (`alloc_model_rss.rs`) ties the verdict to measured peak RSS in both
> directions (the scope's "no false bounded" teeth).
>
> **Step 3 landed (the bound solver):** call-graph multiplicity
> propagation (`unbounded_invoked` — bus handlers, fixed-point over call
> edges in unbounded loops) folds into a `final_verdict`, so an allocation
> in a fn *invoked* unboundedly (a per-message handler, or a helper called
> in a hot loop — the JSON leak class) is flagged even when its own body
> has no loop. `leak_sites()` → `unbounded_alloc_diags()` emits a located,
> actionable warning, opt-in via `hale check --warn-unbounded-alloc`.
> **Corpus: zero false positives** — the only 4 flags are genuine
> per-message-handler accumulations (e.g. `22-moving-average`'s
> `Window::on_sample` rebuilds a 4-elem array per message into `self`).
> **Loop-ranking landed:** `while v < N { … v += c … }` is proven
> const-bounded when `N` is a const literal and `v` is const-initialized
> and only ever incremented by positive consts (whole-fn scan). Such a
> loop ranks `WhileCounter` (a const trip bound), so an in-loop alloc is
> `accumulates×const`, not `ACCUMULATES-UNBOUNDED` — `while i < 100 {
> alloc }` no longer over-flags. Sound: any reset / runtime increment /
> runtime or non-const init / `self.field` counter stays unbounded (never
> a false "bounded"; four soundness cases pinned in
> `while_counter_ranking_is_sound`). Corpus unchanged (still 4 real
> per-handler flags). *Still deferred:* the `@unbounded` escape valve and
> type-aware String-concat sites.
>
> **Reverted to opt-in (2026-06-25).** The warning was promoted to
> default-on (#122) but later reversed: per Hale's descent-curve stance,
> a bound *per epoch* only means something for a long-lived process, so a
> script that allocates and exits owes the proof nothing and pays nothing
> by default. The warning is now opt-in via `--warn-unbounded-alloc`
> (advisory; never fails the build), mirroring how `@locality` cache-tier
> budgets are flag/annotation-gated. `--no-warn-unbounded-alloc` is an
> accepted no-op for back-compat. The opt-in semantics are pinned by
> `hale-cli` `unbounded_alloc_opt_in.rs`.
>
> **Phase B landed — `@bounded` / `@unbounded` (2026-06-25).** The
> in-source opt-in: `@bounded locus L { … }` opts that locus into the
> proof on every `hale check` (no flag); `@unbounded` on a `fn` or a
> lifecycle hook (`@unbounded run { … }`) is the greppable carve-out that
> silences one body's sites, in scoped mode AND under the survey flag.
> `unbounded_alloc_diags(programs, include_all)` gates scope:
> `include_all=false` reports only `@bounded`-locus sites (default check),
> `true` is the whole-program survey (`--warn-unbounded-alloc`).
> Implementation: `bounded: bool` on `LocusDecl`, `unbounded: bool` on
> `FnDecl` + `LifecycleDecl`; `AllocSummary.{bounded_loci, unbounded_fns}`;
> parser handles both as bare `@`-flags. **Severity is warning** — the
> hard *error* contract for `@bounded` waits on the precision phases
> (store-latest vs. append, `@form(cap)`) reaching zero in-scope FP.
> Pinned by `hale-syntax` parser tests + `hale-cli` `bounded_annotation.rs`.
> *Next: Phase C* — replace-vs-append refinement (RSS-validated).
>
> **Default-on deliberately held, then shipped, then reverted.** The
> 2026-06-10 reasoning below is retained for context; the conclusion
> (kept opt-in) is once again the shipped behavior. The 4 flags are
> *true* positives but include legitimately bounded-by-design patterns —
> `22-moving-average`'s `Window::on_sample` keeps a fixed 4-elem window;
> per the reclamation model the *replaced* arrays aren't freed until
> dissolve, so the warning is correct but the author would reasonably
> accept it. Default-on would emit **unsuppressable** warnings on 3 shipped
> example fixtures with no way to acknowledge intent. Prerequisites before
> revisiting: (1) the `@unbounded` escape valve (acknowledge an intended
> accumulation → suppress its warning), and ideally (2) a replace-vs-append
> refinement so a store-latest (`self.x = …`, bounded) is distinguished
> from an append (`self.list.push(…)`, growing). Then default-on with a
> `--no-warn-unbounded-alloc` opt-out + triaged examples.
>
> **The model was corrected by measurement** (see below) — `spec/memory.md`
> was corrected to match (free-fn no per-call reclaim). The type-free
> `+`-as-concat over-report from step 1 was *removed* (it flagged every
> `i + 1`); String-concat detection is deferred to a type-aware stage.

## The empirical reclamation model (measured, not assumed)

The scope warned the reclamation model is the whole ballgame. RSS
experiments (3M-iteration loops, `std::process::rss_bytes()`) established:

| program | RSS | reading |
|---|---|---|
| no-alloc loop | ~5 MB | runtime floor |
| struct allocated directly in the loop | ~99 MB | accumulates |
| struct allocated inside a **non-inlinable free fn** called per iter, only an `Int` returned | ~99 MB | **also accumulates** |

The third row is the key finding: **free-fn returns do NOT reclaim per
call** — a value allocated inside a called free fn accumulates in the
caller's region exactly as if inlined. This **contradicts `spec/memory.md`
§"Free fn functions"** ("the function returns when … region freed"). So
the model attributes a value allocation's reclaim to its enclosing
**locus** (dissolve), not the fn return. Conservative and correct (no
false "bounded"). Only bus sends get a per-iteration boundary
(`reclaim@bus-dispatch`).

> **Resolved (2026-06-09):** the spec was corrected to match shipped
> behavior — `spec/memory.md` §"Value allocations vs. the free-fn implicit
> locus" now states that value allocations bump into the enclosing locus's
> region and reclaim only at its dissolve, not at fn return. The model
> encodes that.

## Goal

Prove at compile time that a locus's arena — or a hot path through it — is
bounded by a static constant per epoch, independent of runtime input.
Where a bound can't be proven, surface it: *"unbounded — annotate
`@unbounded` or rework,"* not a silent OOM at 3am.

## The leak classes this catches (all observed, not hypothetical)

1. **Factory-style returns that pin for program lifetime** — a method
   returns a heap value that escapes into a long-lived owner; called in a
   loop, the owner's arena grows without bound. (GH #18 #6 / CQRS already
   bans the *locus*-returning shape; this is the data analog.)
2. **Collection cells with growing fields** — a `@form(hashmap)` /
   `@form(vec)` cell whose element has a `String`/array field that grows
   per update.
3. **Hot-path allocation in an observe/ingest loop** — String concat, or
   a parse-per-message, inside an unbounded `while`/handler, allocating
   into an arena that isn't reclaimed per iteration.
4. **Worked example (this session):** `let q = Type::from_json(body)` in a
   200k-iteration loop sat at **346 MB** — each parsed struct (and, before
   the inline rewrite, the cursor's per-step intermediate structs)
   accumulated in the enclosing arena because nothing freed per iteration.
   A bound pass would have flagged the loop at build time: *"allocates per
   iteration into a non-reclaimed arena; iteration count is runtime
   input → unbounded."*

## The key insight: this is an escape/lifetime analysis, not alloc-counting

The naive framing ("sum allocation sizes × invocation counts") misses the
actual mechanism. Hale frees arenas **wholesale at scope exit** — a method
that allocates locally and returns nothing heap-bearing is *bounded by
construction* no matter how much it allocates, because its subregion is
freed at return (empirically: a fallible fn returning a flat struct, 200k×
in a loop, stayed at ~18 MB; the same loop calling a parser that *escaped*
intermediates hit 346 MB). So the question per allocation site is **does
this allocation escape its scope?**

- **Local** (freed at method/scope exit): bounded by the *peak* within one
  scope activation — i.e. by the method's own loops, not by how often the
  method runs.
- **Escaping** (returned, stored in a locus field, pushed into a
  collection, routed to the payload arena): persists past the scope, so it
  accumulates in the owner across invocations.

The proof obligation is therefore: **every escaping-allocation path either
lands in a bounded sink (a fixed-cap `@form`, an overwritten field) or is
gated by a statically-bounded loop/recursion.** Unbounded escape into an
unbounded-invocation context (a per-message handler, an unbounded loop) is
the leak.

## Shape

Three layers, the first two of which #3/#5 share:

1. **Per-method allocation summary** (shared infra). For each `fn` /
   lifecycle / handler body, a summary: the set of allocation sites
   (arena allocs — struct/String/Bytes literals, concat, `@form` push,
   slice/copy, codec routing), each tagged **local** vs **escaping**
   (escape = flows to a `return`, a `self.field` store, a collection
   `push`, or a bus `<-`), plus the loops/recursive calls that multiply
   them.
2. **Call graph + invocation context** (shared infra). Walk from the
   entry points — `run()`, bus-handler `fn`s, lifecycle hooks — composing
   method summaries. Classify each context's invocation multiplicity:
   one-shot (`birth`), bounded loop (const trip count), or **unbounded**
   (a handler called per message, `while true`, recursion without a
   ranking function).
3. **Bound solver.** For each escaping allocation reached under an
   unbounded context, require a bounded sink or a bounded gating loop;
   else emit *"unbounded allocation: <site> escapes into <unbounded
   context>; annotate `@unbounded` or route into a bounded sink."* Local
   allocations need only their intra-scope loops bounded.

## Loops + recursion (the hard core)

- **Bounded loop** (`for i in 0..N` with const N, or `[T; N]` iteration):
  multiply the body's local peak by N; escaping allocs × N must hit a
  bounded sink.
- **Unbounded loop** (`while cond`, `while true`, `for x in <runtime
  collection>`): local allocs are fine *iff* the arena reclaims per
  iteration (a per-iteration subregion); escaping allocs are the leak.
  Needs the reclamation model below.
- **Recursion**: needs a ranking function (decreasing measure) to bound
  depth, else `@unbounded`. Mutual recursion → SCCs in the call graph.

## The reclamation model it depends on

The analysis is only as good as its model of *when arenas free*. Hale's
rules (per `spec/memory.md`): a locus arena frees at dissolve; a method
may open a subregion freed at return; the payload arena routes bus sends.
The pass must encode: which allocation sites land in which arena, and when
that arena reclaims relative to the enclosing loop/invocation. This model
is the foundation — get it wrong and the bounds are noise. It should be
written down as a first-class table (site-kind → arena → reclaim point)
and validated against the corpus (the inverse check: every program the
pass says is bounded should show flat RSS under the
`high_volume_walk_bounded_rss`-style harness).

## Bounded sinks live on the capacity/projection axis, NOT `@form` (2026-06-25)

An earlier draft framed this as "`@form(hashmap, cap = N)` composition" —
the `@form` cap as a bound the solver reads. **That is wrong**, and the
spec says so plainly: `@form` is an *access discipline* (vec / hashmap /
ring_buffer — picks a lowering + synthesizes methods), and
`spec/forms.md` is explicit that *"discipline goes on the capacity slot,
not in the annotation."* The `cap = N` that appears on
`@form(hashmap, …, cap = N)` is *"an optional initial-size hint"*
(forms.md), **not a bound** — the map still lazily grows. So `@form`
bounds nothing; reading it for a bound is a category error.

The bound lives on the **capacity/projection axis**, in three tiers:

| Construct | Bound character |
|---|---|
| `: projection recognition(cap = N, fixed_cell)` | **Hard static cap.** N cells; overflow is a *runtime error*. `spec/memory.md` calls it "the one real cap." The clean bounded sink. |
| `capacity { pool X of T; }` | Recyclable cells (chunked free-list, geometric growth capped at 4096/chunk). Bounded **iff** acquire/release is balanced — high-water tracks peak population, not a static N. |
| `capacity { heap Y of T; }` / `@form(vec)` / `@form(hashmap)` | Individually freed at dissolve; grows with total population. Unbounded unless population is bounded. |

### The organizing insight: bound model is per storage slot

A locus's storage is an N-tuple of slots (`spec/memory.md` §"capacity
slots"). The proof must route each *escaping* allocation to the slot it
lands in and apply that slot's bound model:

- **Slot 0 — the locus's own bump Arena.** Where `self.field = X{}`
  value allocations land. Bump-allocated, reclaimed only at dissolve, so
  a per-iteration store-latest **accumulates** — measured, see the Phase
  C finding below. This slot is the `@bounded`/`@unbounded` story and the
  store-latest (A/B) fork. No capacity construct governs it.
- **Slots 1..N — capacity pool/heap + projection.** Where collections /
  accepted-child entities live. Bound = the table above: `recognition`
  cap is a hard static bound; `pool` is bounded-by-balanced-release;
  `heap`/`vec`/`hashmap` grow with population.

This also lines up with the entity-vs-data taxonomy: `recognition`
(accept'd entities, hard `fixed_cell` cap) vs `@form`/`capacity` (data).

### Phase C finding — store-latest is unbounded (measured 2026-06-25)

The note had hypothesized a "replace-vs-append refinement" treating a
store-latest (`self.x = …`) as *bounded*. **RSS measurement falsified
it.** A `run()` loop doing a whole-value field replace each iteration
(3M trips, runtime bound) over a baseline of ~55 MB:

| variant | RSS | grows? |
|---|---|---|
| `self.recent[i] = v` (in-place indexed) | 55 MB | no — baseline |
| `self.recent = [i,i,i,i]` (fixed `[Int;4]` replace) | 188 MB | **+133 MB** |
| `self.h = Holder{…}` (flat all-scalar struct replace) | 96 MB | **+41 MB** |

Whole-value field replacement bump-allocates a fresh value each iteration
into slot 0; the replaced value is **not** reclaimed until dissolve —
even for a fixed-size `[Int;4]`. The current model already flags exactly
the growing cases and leaves in-place writes alone, so **the model is
sound and there is no false positive to fix.** The "store-latest is
bounded" refinement is **dropped as unsound.** Two follow-on directions
remain open (the slot-0 fork): **(A)** lock the soundness with RSS
regression tests + redirect precision effort to the capacity/projection
axis; or **(B)** a *codegen* change making fixed-size value-type field
assignment an in-place memcpy (no bump-alloc) so store-latest becomes
genuinely bounded, then teach the model. A is analysis-faithful and
cheap; B is a larger codegen-correctness effort that supersedes A's
store-latest assertion.

## Scope boundaries

- **In:** arena-growth bounds per epoch, the escape/lifetime classification,
  bounded/unbounded loop + recursion ranking, the `@unbounded` escape, the
  diagnostic with the offending site + context.
- **Out (for v1):** exact byte counts (a *symbolic* "bounded by a const"
  is the bar, not "≤ 4096 bytes"); full WCET/timing; cross-binary /
  transport (explicitly library territory per the issue).
- **`@unbounded`**: the escape valve — an explicit, greppable
  acknowledgement that a path is intentionally unbounded (a cache, an
  accumulator the operator sizes). Mirrors the issue's stated shape.

## Staging

1. **Shared dataflow scaffold** — the call graph over entry points + the
   per-method allocation-site summary (sites + escape tagging + loop/
   recursion structure). No bound-proving yet; just the IR and a
   `--dump-alloc-summary` to validate it against known programs. *This is
   the reusable infra for #3 and #5 — design it once, here.*
2. **The reclamation model** — the site→arena→reclaim table, validated by
   the bounded-RSS harness on the corpus (no false "bounded").
3. **Bound solver + `@unbounded`** — the escape-into-unbounded-context
   obligation, bounded loops/sinks, the diagnostic. Warning first (like
   the bus-graph checks shipped), error once the false-positive rate on
   the corpus is zero. *(Shipped. Phase A made it opt-in via
   `--warn-unbounded-alloc`; Phase B added the `@bounded` locus opt-in +
   `@unbounded` carve-out — see the status block at the top.)*
4. **Capacity/projection-aware slot bounding** *(was mislabeled "`@form`
   composition")* — route each escaping allocation to the storage slot it
   lands in and apply that slot's bound model: `recognition(cap = N)` is a
   hard static bound; a `pool` slot is bounded-by-balanced-release; a
   `heap` slot / `vec` / `hashmap` grows with population; slot 0 (the bump
   Arena) accumulates a store-latest. Then compose per-cell field bounds
   (a fixed-scalar cell field is bounded; an appended-to `String` field is
   not). The bound is read from the **capacity block + projection class**,
   never from `@form` (an access discipline that bounds nothing). See
   "Bounded sinks live on the capacity/projection axis" above.
   *Pre-req decision: the slot-0 store-latest (A/B) fork.*

## Risks

- **False positives sink it.** A bounds pass that cries wolf on correct
  code gets `@unbounded`-spammed into uselessness. Ship as a **warning**,
  measure the false-positive rate on the whole fixture corpus + the real
  apps, and only promote to error at zero. The bus-graph checks (#4) are
  the template: narrow, structural, no false positives.
- **The reclamation model is the whole ballgame** — if it mis-models when
  an arena frees, every bound is wrong. Validate it empirically against
  RSS before trusting it.
- **Largest lift in #18.** It's real dataflow (call graph, escape
  analysis, loop ranking), unlike the single-AST-walk checks that landed.
  Stage it; don't try to prove everything at once.

## Precedent

SPARK Ada (flight-critical, proves absence of runtime errors incl. storage
exhaustion); AbsInt aiT (WCET via value/loop-bound analysis). Both lean on
explicit annotations where inference can't reach — the `@unbounded` valve
is the same admission.

## Why now

The session's JSON work made the leak classes concrete (the 346 MB loop)
and exercised Hale's arena lifetime behavior directly (subregion-free-at-
return, escape-accumulation, the `high_volume_walk_bounded_rss` harness) —
the exact substrate this pass must model. The reclamation model (staging
step 2) can be seeded from what that work already established.
