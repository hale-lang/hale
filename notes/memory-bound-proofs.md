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
> **The model was corrected by measurement** (see below) — `spec/memory.md`
> overstates free-fn reclaim. The type-free `+`-as-concat over-report from
> step 1 was *removed* (it flagged every `i + 1`); String-concat detection
> is deferred to a type-aware stage.

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

## `@form(hashmap)` interaction (the issue's open question)

A `@form(hashmap, cap = N)` is a *bounded* sink for the cells themselves
(fixed cap), but a cell with a `String`/array field is unbounded if that
field grows per update (leak class 2). So the pass composes: the cell
count is bounded by `cap`; the per-cell field allocations must themselves
be bounded (a fixed-width scalar field is; an appended-to `String` field
is not). The `@form` cap is an input to the solver, not a free pass.

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
   the corpus is zero.
4. **`@form` composition** — the cap-as-input + per-cell-field bounds.

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
