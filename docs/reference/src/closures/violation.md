# ClosureViolation propagation

## Synopsis

When a closure assertion fails, the runtime constructs a
`ClosureViolation` value with structured context, then routes
it via the failure-flow rules per **F.9** (collapse / absorb
/ bubble) and **F.8** (vertical-only-flow upward). The
violation is a typed event distinguishable from a panic at the
source level.

## The `ClosureViolation` type

`ClosureViolation` is a built-in type. Loci that handle
failures do not need to declare it.

| Field | Type | Meaning |
|---|---|---|
| `locus` | `String` | Name of the locus that failed |
| `closure` | `String` | Name of the failing closure |
| `left` | numeric | LHS value of the `~~` assertion |
| `right` | numeric | RHS value |
| `tolerance` | numeric | The `within` clause value |
| `diff` | numeric | `left - right` (where both are numeric) |

## Flow

The runtime's failure-routing algorithm:

1. The closure fails. The runtime builds a `ClosureViolation`
   from the assertion's structured context.
2. The locus *explodes*: its lifecycle does not collapse
   normally; instead the violation is dispatched.
3. The parent of the failing locus is consulted. If the parent
   has declared `on_failure(child_binding: ChildType, err:
   ClosureViolation)`, that body runs.
4. The parent's `on_failure` body chooses one of:
   - **Return without calling a primitive** → *absorb*. The
     parent has acknowledged the failure; the child's
     dissolution is now a clean collapse from the parent's
     perspective.
   - **Call `restart(child_binding)`** or
     **`restart_in_place(child_binding)`** → re-run the child's
     `birth` per **F.9** semantics; subject to the cap-2
     budget per locus lifetime.
   - **Call `quarantine(child_binding)`** → the child is
     stickily silenced (its `run` is skipped, bus
     subscriptions are silenced); `drain` and `dissolve` still
     fire as cleanup.
   - **Call `bubble(err)`** → propagate the violation upward
     to the grandparent.
5. If the parent has no `on_failure` declaration, the violation
   propagates upward implicitly (equivalent to `bubble`).
6. If the violation reaches `main`'s implicit locus and is not
   handled, the process exits non-zero with the violation
   report on stderr.

## Source-level handling

```aperio
locus AuditL {
    on_failure(c: CheckerL, err: ClosureViolation) {
        println("AuditL absorbed: ", err.closure,
                " on ", err.locus,
                " (diff=", err.diff, ")");
        // returning without re-raising = absorb
    }

    run() {
        CheckerL { x: 5, y: 5 };  // collapses silently
        CheckerL { x: 5, y: 7 };  // err.diff = -2; absorbed
    }
}
```

## Recovery primitives at a glance

| Primitive | Effect | Re-run birth? | Cap |
|---|---|---|---|
| `restart(c)` | Re-run `birth` on same memory | yes (state preserved) | 2 |
| `restart_in_place(c)` | Factory-reset, then re-run `birth` | yes (defaults restored) | 2 |
| `quarantine(c)` | Sticky stop | no | n/a |
| `bubble(err)` | Propagate upward | no | n/a |
| (return without calling) | Absorb | no | n/a |

The `restart` and `restart_in_place` primitives share a single
cap-2 budget: a locus can use at most 2 attempts total in any
combination.

See [recovery operations](../recovery/index.md) for the full
semantics.

## Distinguishability from panics

`ClosureViolation` is a typed substrate event. Other runtime
errors — array index out of bounds, division by zero, missing
inferred parameter — are *panics* and follow a different path
(immediate process exit with diagnostic). They do not reach
`on_failure`.

A future revision may unify the two paths or add a typed
`PanicEvent`; for v0 they are distinct.

## See Also

- [Closure assertions](./index.md)
- [Closures (locus members)](../loci/closures.md)
- [Recovery operations](../recovery/index.md)
