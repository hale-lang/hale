# 03-closure-test

A locus with a single closure assertion that demonstrates the
two failure modes of dissolution: **collapse** (clean) and
**explosion** (audit failure).

```
locus CheckerL {
    params {
        x: int = 5;
        y: int = 5;
    }

    closure xy_match {
        self.x ~~ self.y within 0;
    }
}

fn main() {
    CheckerL { x: 5, y: 5 };  // collapses
}
```

## Collapse vs. explosion

When a locus dissolves, the runtime evaluates each declared
closure at its declared epoch boundary. The default epoch is
`dissolve`, so closures fire as part of dissolution.

| Closure result | Dissolution mode | What the parent sees |
|---|---|---|
| All pass | **Collapse** | Normal child-dissolution event. |
| Any fail | **Explosion** | `on_failure(self, ClosureViolation { ... })` typed event. |

A collapsed locus is "books-balanced from the parent's
perspective" — its work is done, its closures held, no
bookkeeping needs to absorb. An exploded locus is "books-
unbalanced" — the discrepancy must be accounted for somewhere,
and the parent is the natural place because the framework's
vertical-only-flow puts the parent in policy authority.

This separates **structural failures** (panics, runtime errors)
from **audit failures** (closure violations). Both flow up to
the parent's `on_failure`, but they're distinct typed events.
The parent's handler chooses the response per failure type:

```
on_failure(c: CheckerL, err: Error) {
    match err {
        Error::ClosureViolation(v) -> {
            // The child's books didn't balance. Absorb the
            // discrepancy into our own running totals, OR
            // bubble further, OR recover.
            log_violation(v);
            // treating-as-collapse means we just return:
            return;
        }
        _ -> bubble(err);
    }
}
```

If `main`'s implicit locus has no `on_failure`, the failure
bubbles to the runtime root, which exits the process with a
non-zero code and a `ClosureViolation` report.

## What runs (collapse case)

1. `main()` invoked.
2. `CheckerL { x: 5, y: 5 }` instantiates as anonymous child of
   `main`'s implicit locus.
3. `birth()` runs (default, no-op).
4. No `run()` declared, so steady-state is empty.
5. Locus enters dissolution. Runtime evaluates `xy_match`:
   `5 ~~ 5 within 0` — passes.
6. Locus collapses cleanly. Parent (`main`'s implicit) sees
   normal child-dissolution.
7. `main()` returns. Process exits 0.

## What would run (explosion case)

If `CheckerL { x: 5, y: 7 }`:
1. Same up through dissolution.
2. Runtime evaluates `xy_match`: `5 ~~ 7 within 0` — fails.
3. Locus enters exploded state.
4. Dissolution proceeds: region freed, lifecycle ends.
5. Parent's `on_failure(self, ClosureViolation { ... })` invoked.
   `main`'s implicit locus has no handler, so default `bubble`
   propagates to runtime root.
6. Runtime root prints a structured violation report:
   ```
   ClosureViolation in CheckerL/xy_match
     epoch:    dissolve
     left:     5
     right:    7
     within:   0
     diff:     2
     locus_id: ...
   ```
7. Process exits with non-zero status.

## Primitives this exercises (new vs. 02)

- **`closure name { ... }` block** — declares an audit
  invariant. The runtime maintains the closure as part of the
  locus's structure.
- **`~~` operator inside a closure block** — the approximate-
  equality assertion. Reserved to closure context only (per
  precedence.md).
- **`within tolerance`** — the band the runtime accepts as
  "close enough." For integer comparison with `within 0`, exact
  match required.
- **Implicit `epoch dissolve`** — when `epoch` clause is
  omitted, default is to evaluate the closure at dissolve.
- **Self-referential closure assertion** — `self.x ~~ self.y`
  reads two locus params (which are also state). No external
  iteration required.
- **Collapse vs. explosion failure modes** — distinct dissolution
  outcomes that drive different parent-policy paths.

## What writing this surfaced (for the spec)

Three commitments locked in this commit:

1. **`params` is both initial state and runtime state.**
   Following the user's pointer to Ruby's `@foo` pattern,
   lotus collapses the params/state distinction. Defaults
   serve as birth-time defaults; the same fields are mutable
   throughout the locus's lifetime via `self.x = ...`. No
   separate `state` block. Updated design-rationale §3.

2. **Collapse vs. explosion as the two dissolution modes.**
   A locus dissolves with all closures passing → collapse;
   any closure failure at dissolve → explosion. Explosion
   surfaces as a typed `ClosureViolation` event the parent
   receives via `on_failure`. Distinct from structural failure
   (panic). Added §F.9.

3. **Default epoch is `dissolve`.** Closures with no `epoch`
   clause evaluate at dissolution. Other epochs (`tick`,
   `duration(d)`, `birth`, `explicit`) are still in the
   grammar; their semantics will land in `04-modes` or
   `04.5-epochs` when the example forces them.

## What this still does *not* exercise

- Iteration over `self.children` — deferred to 04
- `epoch tick` / `epoch duration(...)` / `epoch explicit` —
  deferred
- `persists_through(...)` / `resets_on(...)` clauses —
  deferred
- Multi-closure interaction in one locus — deferred
- `on_failure` handlers that absorb vs. bubble — deferred
  (will likely surface in a later closure example)
- Mode declarations — 04
- Bus interface — 05

## Next on the ladder

`04-modes` — a locus that exposes the same kernel under
bulk / harmonic / resolution modes; introduces `self.children`
iteration; closures over child collections become natural
(e.g., `sum(c.value for c in self.children)`).
