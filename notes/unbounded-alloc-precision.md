# Precision refinements for `--warn-unbounded-alloc` (GH #18 item 1)

Status: **scoped** (2026-06-11). The path to making the memory-bound
warning precise enough to eventually be **default-on** — replacing the
rejected `@unbounded` escape-hatch idea (an escape hatch lets people opt
out of the substrate discipline the check exists to enforce; the right
move is to make the warning *right*, not silenceable). See the default-on
decision in [[memory-bound-proofs]] and the escape-hatch discussion that
killed `@unbounded`.

## What the warning actually does today (grounded)

`leak_sites` / `final_verdict` (`alloc_summary.rs`) flags a Struct / Array /
`[v; N]` / Bytes literal when **either**:

- it's inside an **unbounded loop** (`while true`, runtime `for`-iter, a
  non-const-counter `while`), **or**
- its owner fn is **unbounded-invoked** (a per-message bus handler, or
  reached through a call in an unbounded loop) **and** its reclaim scope is
  `EnclosingLocus`.

`EnclosingLocus` is **everything except a bus `Sent`** — so `Local`,
`Returned`, and `StoredToSelf` all qualify. There is **no escape filter and
no lifetime filter.** That is the imprecision.

## The framing correction (vs. my first sketch)

"Replace-vs-append reduces false positives" is **wrong** as stated. In a
bump arena a `self.f = new_value` *replace* also leaks resident memory —
the old value's bytes are never reclaimed until the locus dissolves (bump
pointers don't go backwards). So a replace in a *long-running* locus is a
**true positive**, not noise. The real false-positive sources are
different, and replace-vs-append earns its keep for *message precision* and
*runtime-fix targeting*, not for silencing.

## The actual false-positive sources

1. **Transient local scratch in a handler.** `fn on_msg(m) { let t =
   Struct{…}; use(t); }` — `t` never escapes. The method-scratch
   sub-region is reclaimed at method return (the leak that *was* there was
   fixed — see [[compiled-corpus-oracle-harness]]), so `t` does **not**
   accumulate across messages. But `final_verdict` flags it anyway
   (`EnclosingLocus`, owner unbounded-invoked). **This is the biggest false
   positive class.**
2. **Accumulation in a short-lived locus.** The same `self.x = alloc` is a
   leak in a long-running service and *fine* in a per-connection
   `accept`'d child that `release`s (its whole region is reclaimed when the
   connection ends) or a one-shot batch `main` (reclaimed at exit). The
   check is blind to the owner's lifetime.

A long-running-locus *replace* (the moving-average `self.window = [...]`)
is **not** in this list — it genuinely leaks resident memory and should
stay flagged. The author's "it's a bounded 4-window" belief is wrong about
the runtime; the fix is to route it over the bus, use a fixed slot, or the
runtime sidecar below — not to suppress.

## R1 premise — CONFIRMED (via codegen, definitively)

**Does a non-escaping local literal in a per-message handler accumulate
across messages? No.** Settled by reading the codegen rather than an RSS
proxy: a locus method / bus handler **opens a method-scratch subregion at
entry** (`open_method_scratch`, `method.rs:656`) and **destroys it at exit,
per delivery** (`emit_method_scratch_destroy`, `method.rs:728` — the comment
warns that skipping the destroy "leaks one subregion per delivery").
Transients allocate into the scratch and are freed per call; escaping
values are copied out to `self` / the caller first. So `Local` is reclaimed
per invocation; the `alloc_summary` model's `Local → EnclosingLocus →
accumulates` over-flags. **R1 is a soundness fix.** The in-loop case is
unaffected (a local in a loop *within one call* accumulates until the call
returns — RSS-validated — so R1 leaves the in-loop verdict path untouched).

## The three refinements

### R1 — escape-awareness on the cross-invocation path — **LANDED**

In the **unbounded-invoked** path only, require the site to **escape its
handler** (`StoredToSelf` / `Returned`, via `Escape::persists_across_calls`)
before flagging; a non-escaping `Local` is reclaimed per invocation (method
scratch). The **in-loop** path (`site.verdict() == AccumulatesUnbounded`) is
untouched. A one-clause change in `final_verdict`.

**Corpus impact: 4 → 3 warnings.** R1 removed a genuine false positive —
`fitter.hl`'s `on_observation` builds `let p = KernelPerspective { … }`,
calls `p.is_stable()`, and discards it (the publish sends
`self.latest_kernel`, not `p`). `p` is a transient reclaimed per delivery;
it was being wrongly flagged. The three survivors are all real
`self.X = …` stores (true positives). So the earlier "4 genuine flags, zero
FPs" claim was off by one — R1 found it.

### R2 — replace-vs-append on the escaping case (message precision + runtime-fix targeting)

Among `StoredToSelf` sites, split by a syntactic test on the assignment:
`self.f = E` is **grow/append** iff `E` reads `self.f` (e.g.
`self.buf = self.buf + chunk`, a push onto `self.f`); otherwise **replace**
(`self.window = compute(m)`).

- **Append** — incorporates all prior values → unbounded by construction.
  Warn with high confidence; advice = "bound / cap / route."
- **Replace** — one live value; resident-only leak (the arena keeps old
  copies). Warn **only if the locus is long-running** (R3), with a
  replace-specific message: *"one value is live, but the old one isn't
  reclaimed until the locus dissolves — use a fixed `capacity` slot, route
  over the bus, or shorten the locus lifetime."* Replace-of-a-fixed-size
  field is the runtime-fix candidate (below).

### R3 — lifetime-awareness (kills the short-lived-locus false positives)

Classify the owner locus's lifetime from its lifecycle shape (compile-time
known):

- **Long-running / resident** — a top-level singleton, a locus with a
  non-trivial `run()`, a bus subscriber with no `release` above it.
  Accumulation = forever → **warn**.
- **Short-lived / flow** — an `accept`'d child whose parent declares
  `release` (region reclaimed per unit of work), a one-shot `main` in a
  non-service program, a per-request flow. Accumulation bounded by the
  (bounded) lifetime → **don't warn** (or downgrade to a note).

Conservative default: unknown / can't classify → treat as long-running
(warn). Never a false "bounded."

## How they compose

| site | owner lifetime | verdict after refinements |
|---|---|---|
| non-escaping `Local` in a handler | any | **silent** (R1 — reclaimed per call) |
| `Local` in an unbounded loop | any | **warn** (unchanged — accumulates within the call) |
| `StoredToSelf` *append* (`self.f = self.f + x`) | long-running | **warn** (unbounded by construction) |
| `StoredToSelf` *append* | short-lived | silent / note (R3) |
| `StoredToSelf` *replace* (`self.f = E`) | long-running | **warn**, replace-specific message (R2) |
| `StoredToSelf` *replace* | short-lived | **silent** (R3) |

The corpus's moving-average / fitter warnings are long-running `StoredToSelf`
sites, so they **stay** (true positives) — with sharper messages. What goes
away is transient handler scratch (R1) and any short-lived-locus
accumulation (R3). That's the point: after the refinements the surviving
warnings should all be true positives, which is the precondition for
default-on.

## Runtime sidecar (optional, makes replace genuinely bounded)

A **free-on-overwrite / in-place slot reuse** for a *fixed-size* heap-typed
`self` field: allocate the field's storage once, overwrite in place on
reassignment. Turns long-running fixed-size replace (the moving-average
window) from a resident leak into a true constant. Variable-size replace (a
grown String) can't reuse a fixed slot and stays a real leak. With this,
R2's replace case can drop to "bounded" for fixed-size fields. Out of scope
for the analysis work; flagged here as the principled fix the replace
diagnostic should point at.

## Staging

0. **Validate** the handler-scratch reclaim question — **DONE** (confirmed
   via codegen; see above).
1. **R1** — escape-awareness on the cross-invocation path — **LANDED**
   (corpus 4 → 3, removed a transient-local FP).
2. **R3** — lifetime classification. Removes short-lived-locus warnings.
   *(next)*
3. **R2** — replace-vs-append messaging + runtime-fix targeting.
4. **Re-evaluate default-on** — only once the surviving corpus warnings are
   all true positives. Then it's a warning that's *right*, needing no
   escape hatch.

## Risk

- **R1's premise is empirical** — if handler scratch is *not* reclaimed
  per-invocation, R1 is unsound and the model is right to flag locals;
  hence the validation gate. Don't ship R1 on the assumption.
- **Lifetime classification must stay conservative** — a misclassified
  long-running locus as "short-lived" would hide a real leak. Unknown →
  long-running.
