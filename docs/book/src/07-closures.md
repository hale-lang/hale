# Closures

A **closure** in Aperio is the substrate's *audit primitive*: a
declaration that some property of the locus's state holds, evaluated
at a specific moment in the locus's lifecycle. When the property
holds, the locus's lifecycle proceeds normally. When it fails, the
runtime emits a typed `ClosureViolation` that propagates upward
through the parent's `on_failure` per **F.9**.

This is the rune by which a locus *watches itself*. Most languages
implement audit out-of-band — assertions in tests, schemas in
proxies, monitoring dashboards in production. Aperio puts the
audit *inside* the locus, evaluated at compile-known moments by
the runtime.

(Aperio's "closure" is not the closure-of-a-function found in
ML-family languages. The name reflects its role: closing over
the locus's state and the moment at which the closure of that
state is audited.)

## A first closure

```aperio
locus CheckerL {
    params {
        x: Int = 5;
        y: Int = 5;
    }

    closure xy_match {
        self.x ~~ self.y within 0;
    }
}

fn main() {
    CheckerL { x: 5, y: 5 };
    println("collapsed cleanly.");
}
```

The `closure xy_match { ... }` block declares a single closure
named `xy_match` on `CheckerL`. The body holds an assertion:
`self.x ~~ self.y within 0` reads "the value of `self.x` should be
within `0` of the value of `self.y`" — they must be equal in this
case.

The `~~` operator is the *approximate equality* operator. It is
permitted **only** inside a closure assertion clause; using it
elsewhere is a parse error. The companion `within <expr>` clause
specifies the tolerance: `0` for exact equality, any numeric
expression for fuzzy equality.

Output of the program above:

```text
collapsed cleanly.
```

The closure passed at dissolve (5 and 5 are within 0 of each
other), so the locus *collapsed*: clean dissolution, no
observable side effect from the closure itself.

## The three outcomes (F.9)

When a closure fires, exactly one of three things happens:

1. **Collapse.** The closure passed; the locus dissolves
   normally. No special effect; the parent (if any) sees a
   normal child-dissolution event.
2. **Absorb.** The closure failed; the parent's
   `on_failure(child, err: ClosureViolation)` body runs; the
   parent reads the structured violation and returns without
   re-raising. From the parent's perspective the child's
   dissolution is now a clean collapse.
3. **Bubble.** The closure failed and no parent handles it (or
   the parent explicitly calls `bubble(err)`). The
   `ClosureViolation` propagates to the grandparent. If it
   propagates past `main`, the process exits non-zero with the
   violation report on stderr.

This is **F.9**. Three outcomes, total. There is no "ignored
silently" path; a violation either collapses, is absorbed, or
bubbles. (Recovery primitives like `restart_in_place` and
`quarantine` are full alternatives to `bubble` in
`on_failure`; see [chapter
11](./11-recovery-and-supervision.md).)

## `ClosureViolation`

When a closure fails, the runtime constructs a
`ClosureViolation` value with the assertion's structured
context. v0 fields:

| Field | Type | Meaning |
|---|---|---|
| `locus` | `String` | Name of the locus that failed |
| `closure` | `String` | Name of the failing closure |
| `left` | (numeric) | LHS of the `~~` assertion |
| `right` | (numeric) | RHS of the assertion |
| `tolerance` | (numeric) | The `within` clause value |
| `diff` | (numeric) | `left - right` when both are numeric |

A parent's `on_failure` body reads these fields directly:

```aperio
locus AuditL {
    on_failure(c: CheckerL, err: ClosureViolation) {
        println("AuditL absorbed: ", err.closure,
                " on ", err.locus,
                " (diff=", err.diff, ")");
        // Returning without re-raising = absorption.
    }

    run() {
        CheckerL { x: 5, y: 5 };  // collapses silently
        CheckerL { x: 5, y: 7 };  // err.diff = -2; absorbed
    }
}
```

`ClosureViolation` is a built-in type. Loci that handle
failures do not need to declare it — its name is in scope
everywhere `on_failure` is permitted.

## Epochs

Every closure has an *epoch*: the moment in the locus's life
when it is evaluated. The default is `dissolve`. There are five
epochs in v0:

```aperio
closure xy_match {
    self.x ~~ self.y within 0;
    epoch dissolve;     // the default; can be omitted
}
```

| Epoch | Fires |
|---|---|
| `birth` | Once, after the locus's `birth()` body returns. |
| `dissolve` | Once, as the locus dissolves. (Default.) |
| `tick` | After every substrate cell — each bus handler invocation, and after `run()` returns. |
| `duration` | At fixed intervals (e.g., `every 5s`). |
| `explicit` | Only when user code calls `evaluate(closure_name)`. |

Each epoch audits a different question:

- **`birth`** — *was the locus born into a valid state?*
- **`dissolve`** — *did the locus end in a valid state?*
- **`tick`** — *does the invariant hold AT EACH STEP?*
- **`duration`** — *does the invariant hold over rolling
  intervals?*
- **`explicit`** — *evaluate when the program decides; for
  one-off check-points or tests.*

A tick-epoch example:

```aperio
locus Counter {
    params {
        n: Int = 0;
    }

    closure under_cap {
        // |n - 0| must stay <= 5.
        self.n ~~ 0 within 5;
        epoch tick;
    }

    bus {
        subscribe "increment" as on_inc of type Sample;
    }

    fn on_inc(s: Sample) {
        self.n = self.n + s.value;
    }
}
```

After every `on_inc` call, the runtime evaluates `under_cap`. If
`self.n` ever leaves the range `[-5, 5]`, the closure fails and
a `ClosureViolation` reaches the parent's `on_failure`.

The substrate's bookkeeping for epochs is automatic — the user
declares the epoch on the closure, and the runtime arranges for
the right firing schedule.

## Accumulators

Snapshot assertions (`self.x ~~ self.y within 0`) compare two
values at one moment. *Accumulators* extend the surface to
streaming aggregates: `sum`, `count`, `mean` evaluated over a
rolling window of values seen at each epoch fire.

```aperio
closure drift_bounded {
    sum(self.x) ~~ 0 within 1000;
    epoch tick;
}
```

This reads: *the running sum of `self.x` (sampled at each
tick) must stay within 1000 of zero*. At every tick, the
runtime adds the current `self.x` to the closure's per-instance
running total and then evaluates the assertion against that
total.

Accumulator vocabulary in v0:

- **`sum(expr)`** — running total of `expr` evaluated at each
  fire. Numeric only (`Int`, `Float`, `Decimal`, `Duration`).
- **`count(expr)`** — count of fires; `expr` is evaluated for
  side-effect / type-check but not summed.
- **`mean(expr)`** — running mean.

Accumulators have per-closure state and live in the locus's
arena. Like every other allocation, they are freed when the
locus dissolves.

## Audit, not control flow

Closures are an audit channel, not a control mechanism. A
closure cannot:

- Set a value or mutate state. The body is *assertion only*;
  side-effecting expressions are a compile-time error.
- Decide what happens after a violation. The runtime does that
  — it constructs the `ClosureViolation` and routes it via
  `on_failure`. The closure's job ends at the assertion.
- Block the locus's normal work. Evaluation happens at
  declared epochs; between them the closure is silent.

The discipline this enforces: any property the locus must hold
is written *as itself*, not as a chain of `if` checks
distributed through the body. The audit is centralized; the
work is centralized; they do not interleave.

## What this chapter does not cover

- **`on_failure` deeply** — what `restart`, `restart_in_place`,
  `quarantine`, and `bubble` do, when each applies, what the
  recovery costs are. See
  [chapter 11](./11-recovery-and-supervision.md).
- **`self.children` as a scrutinee** — closures auditing
  populations of children (`closure within_capacity {
  self.children.length ~~ 0 within self.k_max; }`). Appears
  alongside recovery in chapter 11.
- **`evaluate(closure_name)`** — the explicit-epoch trigger —
  surfaces in chapter 11 too.

The next chapter, **[Cross-process](./08-cross-process.md)**,
shifts gears. Up to here every Aperio program lived in one
binary. The next chapter introduces what happens when an Aperio
program opens multiple lotuses across separate processes — the
wire format, the deployment-time transport binding, and the
boundary at which "in-memory copy" becomes "copy across a Unix
socket."
