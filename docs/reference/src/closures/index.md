# Closure assertions

## Synopsis

A closure is the substrate's audit primitive: a declaration in
a locus body that some property of `self` holds, evaluated at
a specific moment in the locus's lifecycle. When the property
holds, the locus's lifecycle proceeds normally; when it fails,
the runtime emits a typed `ClosureViolation` and routes it
through the parent's `on_failure` per **F.9**.

> Aperio's closure is *not* the closure-of-a-function found
> in ML-family languages. The name reflects its role: closing
> over the locus's state and asserting the closure of that
> state at audit time.

## Surface

A closure declaration lives inside a locus body:

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
```

Three components inside the body:

1. **An assertion** of the form `lhs ~~ rhs within tolerance;`.
2. An optional **epoch declaration** (`epoch dissolve;` is the
   default).
3. Optional **accumulators** (`sum`, `count`, `mean`).

See [loci/closures](../loci/closures.md) for the source-side
declaration syntax.

## The `~~` operator

Permitted only inside a closure assertion. The form is:

```text
expr "~~" expr "within" expr
```

The runtime evaluates `|lhs - rhs| <= tolerance`. All three
expressions must produce values of the same numeric type
(`Int`, `Float`, `Decimal`, `Duration`).

## Three outcomes (F.9)

When a closure fires, exactly one of three things happens:

| Outcome | When | Effect |
|---|---|---|
| **Collapse** | Closure passed | Lifecycle proceeds; locus dissolves normally |
| **Absorb** | Closure failed; parent's `on_failure` returns without re-raising | Locus dissolves cleanly from parent's perspective; violation observed but not propagated |
| **Bubble** | Closure failed; parent calls `bubble(err)` or has no `on_failure` declaration | `ClosureViolation` propagates to grandparent; if it exits past `main`, process exits non-zero |

Recovery primitives (`restart`, `restart_in_place`,
`quarantine`) are alternatives to `bubble` available inside
`on_failure`. See [recovery](../recovery/index.md).

## See Also

- [Closures (locus members)](../loci/closures.md)
- [Epoch semantics](./epochs.md)
- [ClosureViolation propagation](./violation.md)
- [Recovery operations](../recovery/index.md)
