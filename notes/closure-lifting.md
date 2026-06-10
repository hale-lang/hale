# Closure-assertion lifting (GH #18 item 3)

Status: **scoped — and the tractable slice turned out to be already handled
by an existing typecheck rule, so no code shipped.** Written 2026-06-10. The
last unstarted GH #18 verification candidate. The issue: Hale closures are
runtime invariant checks; many can be proven statically, so the runtime
check is dead weight — lift the provable subset, leave the rest as runtime
asserts. Opportunistic, gates nothing.

## Reality check: Hale closures are `~~ within` band checks

The issue's example — *"a closure like `count >= 0` over a never-decremented
counter"* — is a **comparison** assertion. Hale doesn't have those. A Hale
closure is:

```hale
closure within_band {
    sum(self.delta) ~~ 0 within 100;   // |left - right| <= tolerance
    epoch tick;
}
```

an **approximate-equality band check** — `|left - right| <= tolerance` —
evaluated per epoch over *runtime / accumulated* quantities (`sum(...)`
accumulators, `self.field` reads).

## Finding: the constant case is already forbidden

The obvious tractable slice was "fold constant closure assertions: lift the
always-true ones, error on the always-false ones." **It's redundant.**
`check.rs` (the "closure" typecheck arm, ~line 6029) already rejects any
closure whose assertion observes no runtime-varying value:

> `closure 'X': both assertion sides are pure literals; a closure must
> observe at least one runtime-varying value (e.g. self.x) to audit
> anything`

Verified this fires on both pure literals (`1.0 ~~ 2.0 within 0.1`) **and**
const arithmetic (`2.0 - 1.0 ~~ 0.5 within 0.1`) — so a fully-constant
closure (satisfiable or not) is already a compile error. There are no valid
all-constant closures to lift, and the always-false bug-catch is subsumed:
Hale rejects the *whole class* as pointless, not just the unsatisfiable
members. A constant-fold "lifting" pass would be dead code that only fires
alongside an existing type error. **Not shipped** (a prototype was written
and backed out once this was confirmed).

## What's actually left: case 3 (symbolic, deferred)

Every closure that *survives* typecheck observes a runtime value, so the
only liftable closures are ones provable from **how that runtime value is
produced** — e.g. proving `sum(self.delta) ~~ 0 within 100` always holds
from the producer arithmetic feeding `self.delta`. That's the issue's
"symbolic execution of closure bodies": reason about the accumulator + the
self-field writes that feed it. It would reuse the dataflow infra
(`alloc_summary`'s self-field-write tracking + the closure accumulator
model) and a check→codegen handoff to skip the runtime check for proven
closures.

## Honest recommendation: don't build case 3 (yet)

This is the **lowest-leverage** #18 item, and the finding makes it lower
still: the easy win is already done by an existing rule, and the remaining
win is a substantial symbolic-execution effort whose payoff is removing a
runtime check from a *niche* feature (closures) on the *provable* subset of
an *already-runtime-only* set. The cost/benefit doesn't favor it now. Revisit
only if (a) closures see materially heavier use, and (b) a profiled workload
shows closure checks on a hot path. Until then, item #3 is **complete as
far as it's worth taking** — the constant case is handled, the symbolic case
is scoped and deliberately parked.
