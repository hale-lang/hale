# Semiring traversals: one memo, four walkers (analysis)

Status: **analysis only — nothing implemented.** Written 2026-08-12,
verified against `main` @ `2cf0e1a`.

Prompted by Iago Leal's ["A Tale of Dynamic
Programming"](https://iagoleal.com/posts/dynamic-programming/), read against
`hale-types`' call-graph analyses. Most of that article does not apply to a
compiler (see §6 — read it before importing any of its vocabulary). The part
that does applies sharply, and it names three things that are already true of
our code:

| # | Finding | Cost | Unblocks |
|---|---|---|---|
| S1 | `count_dim` re-walks per reaching path; `stack_depth` (same file, 40 lines up) memoizes | small | removes a class of `MAX_STEPS` saturations that surface as uncertifiable programs |
| S2 | `FactVisitor` is a semiring with ⊕ defaulted to ⊗ | small–medium | one engine for boolean/counting/tropical facts instead of per-verb walkers |
| S3 | Witness paths are declared impossible from a fixpoint in `callgraph.rs`, and `model_graph.rs` already does it | medium | the deferred #382 provenance-bearing model |

S1 is the one worth doing on its own merits regardless of what happens to S2/S3.

---

## §1 The frame, briefly

The transferable content of the article is one sentence: **the value at a
state depends on the state, not on the history that reached it** — Bellman's
principle of optimality — and that property is what licenses memoization,
fixpoint iteration, and topological single-pass evaluation. Everything else in
the article (contraction mappings, discount factors, policy iteration) is
about *choosing* actions to minimize cost, which no analysis in `hale-types`
does. We take worst case over all paths; there is no policy.

The second transferable idea is the article's aggregation function `ρ` over a
monad `M`: one traversal, parameterized by what you accumulate. We built that
independently as `FactVisitor` (`crates/hale-types/src/callgraph.rs:52`).

---

## §2 S1 — `count_dim` is `stack_depth` minus its memo

`grep -rn memo crates --include='*.rs'` returns exactly one traversal in
non-test source: `stack_depth`, `quantitative.rs:178`, memo threaded at `:183`
and populated at `:213` behind a correct guard at `:210`:

> Memoize only cycle-free results (a path-dependent Unbounded must not poison
> a diamond reached another way).

`count_dim` (`quantitative.rs:221`) walks the same graph with no memo. The two
sit in adjacent arms of the same `match dim` at `quantitative.rs:453`:

```rust
QuantDim::StackBytes => { … let mut memo = BTreeMap::new(); stack_depth(…, &mut memo, …) }
_                    => { …                                  count_dim(…            ) }
```

`callgraph::walk` (`:124`) has the same shape, and its header (`:37`) states
the per-path revisiting is *"correct for per-call counting."*

### Why the memo is sound here

It yields the right number. It is not required to. `count_dim` implements:

```
count(f) = Σ sites(f)
         ⊕ Σ_{e: f→g, ungrouped}      scale(e, count(g))
         ⊕ Σ_{groups} max_{e ∈ group} scale(e, count(g))
```

where `scale(e, q) = if e.loop_depth > 0 { if q == 0 { 0 } else { Unbounded } } else { q }`.

Three facts make `count(f)` a pure function of `f`:

1. **Loop scaling is applied at the edge** (`quantitative.rs:~333`), not
   inside the callee. `count(g)` is the same value however `g` was entered.
2. **`dim`, `fanout_of`, `carrier_mask`, `summary` are fixed** for the whole
   invocation — same situation as `frames` for `stack_depth`. The memo key is
   `FnKey` alone.
3. **Path-dependence enters in exactly one place**: `path.contains(callee) →
   contrib.add(Unbounded)` (`:~313`). Any node whose value was truncated by
   cycle-cutting therefore carries `Unbounded` upward, so excluding
   `Unbounded` from the memo excludes precisely the path-dependent results.

That is the same argument `stack_depth`'s guard already rests on, so the guard
transfers verbatim:

```rust
if !path.contains(key) && total != Qty::Unbounded {
    memo.insert(key.clone(), total);
}
```

**What would invalidate it:** moving loop scaling inside the callee, making
any of `dim`/`mask`/`fanout_of` vary mid-walk, or introducing a
path-dependent quantity that can still be `Finite`. All three are worth a
comment on the memo so a future change notices.

### Why it matters beyond speed

A k-diamond graph has 2^k reaching paths and 2k nodes. That is why
`MAX_STEPS = 20_000` (`callgraph.rs:41`) exists, and the saturation is not
free:

- `count_dim` → `Qty::Unbounded`
- `callgraph::walk` → `visitor.saturated()`
- `model_graph::reaches` → `Reach::Uncertified`

All three are conservative and fail closed, which is right. But claims are
errors-gating-check by design with no advisory tier, so a large-but-legal
program stops certifying for a reason that is an artifact of traversal
strategy rather than of the program. "Your diamond density exceeded the
walker" is a hard failure that memoization mostly deletes.

### Suggested shape

1. Add `memo: &mut BTreeMap<FnKey, Qty>` to `count_dim`, guard copied from
   `stack_depth`, allocated at the `quantitative.rs:463` call site.
2. Then hoist it: `count(f)` is root-independent too, so one memo per `dim`
   can be shared across all budgeted roots in the bundle rather than
   allocated per root. Do this as a second commit — it is a strictly larger
   claim and deserves its own test.
3. `callgraph::walk` is the same fix but needs S2 first, because a memo there
   is only sound if the `Fact` domain is honest about ⊕ vs ⊗ (a visitor whose
   `combine` is *not* associative would break under re-association). Suggest
   landing `count_dim` first and treating `walk` as follow-on.

### Tests to pin

- A synthetic k-diamond bundle (k ≈ 12) with a known exact count: currently
  saturates to `Unbounded`, should certify. This is the regression that
  proves the point and should fail before the change.
- The corpus baseline must be **byte-identical** — this is a performance
  change, not a semantics change. Any baseline movement means the soundness
  argument above is wrong somewhere.
- A recursive bundle: still `Unbounded`, and the memo must not contain the
  recursive `FnKey` afterwards (assert on the memo directly).
- A diamond where one arm is recursive and the other is not — the case the
  `stack_depth` guard comment was written for. Both orders of traversal.

---

## §3 S2 — `FactVisitor` is a semiring missing a required ⊕

`callgraph.rs:52` defines `identity` / `combine` / `saturated`, and #392 added
`join_alternatives` (`:105`). Those last two are ⊗ (sequential composition)
and ⊕ (choice). But `join_alternatives` defaults to `combine`, documented as
"over-approximate, never under-counting" — one operation doing double duty,
imprecision absorbed as conservatism. Every real visitor then overrides it:
`budget_check.rs:142` uses `add` for ⊗, `:154` uses `max` for ⊕.

The abstraction knows the distinction; the trait does not require it. Making
⊕ required and non-defaulted gives one engine over:

| verb | ⊕ (choice) | ⊗ (sequence) |
|---|---|---|
| `forbid reaches`, `@no_ffi`, `@no_block`, `@no_recursion` | `∨` | `∧` |
| `@budget(stack_bytes)` | `max` | `+` |
| `@budget(alloc_per_call / publish / fanout)` | `max` | `+` |
| witness path (§4) | keep the winner's predecessor | concat |

The payoff is not elegance, it is that the *memoization argument in §2 becomes
a property of the algebra* rather than a fact re-derived per walker. A fact
domain with associative ⊗, associative-commutative-idempotent ⊕, and a top
element is memoizable and fixpoint-iterable by construction. Today we have
four traversals (`alloc_summary`'s fixpoint, `budget_check`'s DFS, `purity`'s
worklist, `check.rs`'s blocking finder — per `callgraph.rs:4`) and only one of
their descendants got the memo.

Note the one place where idempotence genuinely fails: **counting**. `+` is not
idempotent, so counting is not a "did I already see this node" reachability
problem and Kleene iteration to a fixpoint does not converge over ℕ with
cycles. That is *not* an objection to memoizing (§2 is about caching a
node-local recurrence, not about iterating to a fixpoint) but it does mean the
engine cannot use one convergence strategy for all verbs. See §5.

---

## §4 S3 — witnesses from a fixpoint are not impossible; we already ship one

`callgraph.rs:28` says `witness_path` produces the chain *"that `@budget`'s
fixpoint could never produce."*

True of a fixpoint over a **scalar** domain. Not true in general — and
`model_graph::reaches` (`:269`) already does the polynomial version: `search`
maintains a `parent` map and reconstructs the path by walking back
(`:308–316`). Those are Bellman-Ford predecessor pointers.

The general construction is to lift the fact domain to `(value, witness)`
pairs: ⊕ keeps the winner's witness, ⊗ concatenates. This is the standard
provenance-semiring answer to "I want the fixpoint's cost and the DFS's
explanation," and it is why `model_graph` gets a counterexample path without
enumerating paths.

So the two modules disagree about what is possible, and `model_graph` is
right. Worth resolving explicitly, because the deferred #382 item that every
reviewer named as the next milestone — the normalized provenance-bearing model,
witnesses with spans — **is** this construction. We should not build it a
second time in a different shape.

Concretely: `Fact = (Qty, Option<WitnessPath>)` with ⊕ = "take the larger
`Qty`, keep its path" would give `@budget` diagnostics their call chain
without the per-path walk, which is the stated reason `witness_path` exists
as a separate primitive at all.

---

## §5 Backward induction: we compute the precondition and don't dispatch on it

The article's acyclic case: topologically sort, one pass, exact, no iteration,
no convergence assumption needed.

`@no_recursion` establishes exactly that the call graph under a root is a DAG.
`quantitative.rs:22` already leans on it in prose — *"with acyclicity
established … worst-case stack depth is a longest-path computation"* — but
nothing dispatches on the fact. Every walker rediscovers cycles per path via
`path.contains`.

The available branch:

- **acyclic** (`@no_recursion` holds, or cheaply verified): topological order,
  single pass, exact for every verb including the non-idempotent counting
  ones, no step cap needed.
- **cyclic**: Kleene iteration for the idempotent verbs (converges by finite
  lattice height, no γ needed — see §6); `Unbounded` for the counting ones,
  which is what we already report.

This subsumes S1 rather than competing with it: memoization *is* lazy
topological order. S1 is the small version that needs no new fact plumbing;
§5 is what it grows into if the acyclicity fact gets threaded to the walkers.

---

## §6 What NOT to import from the article

Recorded because the vocabulary is seductive and three-quarters of it is a
trap here.

- **No discount factor.** Our domains are finite-height (`Bool`; `Qty =
  Finite(n) | Unbounded` with a top). Termination comes from monotonicity over
  finite height, not from contraction on a metric space. There is no γ, no
  Banach argument, and nothing to tune. If a design doc ever proposes a
  discount factor for a Hale analysis, it has mistaken the analogy for the
  thing.
- **No policy, no argmin, no policy iteration.** Bellman minimizes over
  actions. We take the worst case over all paths. Our ⊕ is `max` / `∨` / `+`,
  never `min`. Roughly half the article — value iteration with γ, policy
  improvement, MDPs, expectation aggregation — has no counterpart, and
  reaching for it would invert the direction of soundness (an
  under-approximation certifies programs it shouldn't).
- **"Dynamic programming" is the wrong name for what we do.** The right
  framing for `hale-types` is abstract interpretation over a lattice: may-
  analysis, over-approximate, `Unbounded` as ⊤, fail closed. The DP literature
  is worth reading for the semiring-parameterization and the memoization
  license, and for nothing else.

---

## Open questions

1. Does any current visitor have a non-associative `combine`? If so, memoizing
   `walk` (S1 step 3) is unsound for it and the trait should say so.
2. `join_alternatives`' default is a silent over-approximation. Is there a
   verb where summing dispatch alternatives instead of maxing them has ever
   changed a verdict on the corpus? If not, making ⊕ required is a free
   tightening; if so, it is a baseline movement that needs its own note.
3. `model_graph`'s `HolePolicy::PathWins` (`:165`) and `callgraph`'s
   `saturated()` are two different answers to "the walk gave up." Should
   saturation be a hole? A hole is at least *locatable*; `Qty::Unbounded` from
   a step-cap trip is indistinguishable from a genuine unbounded recursion in
   the diagnostic, which seems worse than it needs to be.
