# Closures (locus members)

## Synopsis

A `closure NAME { ... }` block declared inside a locus body
introduces a structural-invariant audit. The closure body
contains an assertion (`expr ~~ expr within tolerance`) plus
optional epoch and accumulator declarations. The runtime
evaluates the assertion at the declared epoch; failure
produces a `ClosureViolation`.

## Grammar

```text
closure-decl ::= "closure" snake_case-Ident "{"
                   closure-clause+
                 "}"
closure-clause ::= assertion | epoch-clause
assertion      ::= expr "~~" expr "within" expr ";"
epoch-clause   ::= "epoch" epoch-name ";"
epoch-name     ::= "birth" | "dissolve" | "tick" | "duration" | "explicit"
```

## Example

```aperio
locus CheckerL {
    params {
        x: Int = 5;
        y: Int = 5;
    }

    closure xy_match {
        self.x ~~ self.y within 0;
        epoch dissolve;
    }
}
```

The body holds:

- One assertion: `self.x ~~ self.y within 0` — *the value of
  `self.x` should be within `0` of `self.y`*.
- An epoch declaration: `epoch dissolve;` — *evaluate at locus
  dissolve* (the default; this line is optional).

## Semantics

### `~~` (approximate equality)

The `~~` operator is permitted **only** inside a closure body's
assertion. Using it elsewhere is a parse error.

The form is `lhs ~~ rhs within tolerance`:

- `lhs`, `rhs` are numeric expressions (`Int`, `Float`,
  `Decimal`, `Duration`).
- `tolerance` is a numeric expression of the same kind.
- The runtime evaluates `|lhs - rhs| <= tolerance`. If the
  inequality holds, the closure passes.

### Epochs

The epoch declares when the runtime evaluates the assertion.
See [closures/epochs](../closures/epochs.md) for the full
semantics. Default: `dissolve`.

### Failure: `ClosureViolation`

When the assertion fails, the runtime constructs a
`ClosureViolation` value with the assertion's structured
context (locus name, closure name, left, right, tolerance,
diff) and routes it via the failure-flow rules. See
[closures/violation](../closures/violation.md).

## Accumulators

Closures may use streaming accumulators (`sum`, `count`,
`mean`) instead of snapshot expressions:

```aperio
closure drift_bounded {
    sum(self.x) ~~ 0 within 1000;
    epoch tick;
}
```

The runtime maintains per-closure-instance accumulator state
in the locus's arena. At each epoch fire, the accumulator
updates; the assertion evaluates against the post-update value.

Vocabulary:

- `sum(expr)` — running total. Numeric only.
- `count(expr)` — count of fires.
- `mean(expr)` — running mean.

## Multiple closures per locus

A locus may declare any number of `closure` blocks, each with
its own name, assertion, epoch, and accumulator state. They
fire independently; a failing closure does not prevent others
from running.

## What closures may not do

The body is *assertion only*. The compiler rejects:

- Side-effecting expressions (mutations to `self`, `<-` sends,
  function calls with effects).
- Decisions about what happens after a violation. The runtime
  routes; the closure asserts.
- `if` / `match` / loops in the assertion expression itself.
  (The runtime evaluates the single `~~` expression; control
  flow inside the assertion is not the model.)

## See Also

- [Closure assertions (semantics)](../closures/index.md)
- [Epoch semantics](../closures/epochs.md)
- [ClosureViolation propagation](../closures/violation.md)
- [Recovery operations](../recovery/index.md)
