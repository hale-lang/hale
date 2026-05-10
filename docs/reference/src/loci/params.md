# Params

## Synopsis

A locus's `params { ... }` block declares its *parameter
struct* — the configurable values it carries throughout its
existence. Per **F.3**, the parameter struct is also the
locus's mutable state bundle; fields are mutable through
`self.x = ...`.

## Grammar

```text
params-block ::= "params" "{" param-decl* "}"
param-decl   ::= snake_case-Ident ":" type-expr param-init? ";"
param-init   ::= "=" expr | ":" "inferred"
```

## Example

```aperio
locus FitterL {
    params {
        // Capacity parameters
        B: Int = 1000;
        c: Int = 10;
        sigma: Int = 1;
        phi: Float = 1.0;

        // Locus state
        latest_kernel: Kernel = Kernel {
            scale: 1.0d,
            valid_after: `2026-01-01T00:00:00Z`,
            perspective_id: 0,
        };
        published_count: Int = 0;
    }
}
```

## Semantics

### Default values

Each parameter declares a default value with `= expr`. The
expression must be a *compile-time-evaluable* form — literals,
struct/enum/tuple/array constructors composed of literals,
arithmetic on literals, named constants — not runtime values.

Construction may override any subset of parameters; unspecified
ones use their declared defaults:

```aperio
FitterL { B: 2000 };   // overrides B; everything else default
```

### `: inferred` parameters

Per **F.3**, a parameter may declare `: inferred` instead of a
default value:

```aperio
params {
    weight: Float : inferred;
}
```

This indicates the runtime fills in the value (typically via
the perspective hot-load mechanism); the author does not supply
it. Construction must omit the parameter; the runtime errors if
the inferred value is not available at instantiation time.

### Mutability

Per **F.3**, parameter fields are mutable through `self`:

```aperio
fn on_event(e: Event) {
    self.published_count = self.published_count + 1;
}
```

This is the locus's mutable state bundle. There is no separate
"locus state" construct — `params` carries both
configuration-at-construction and ongoing state.

### Self-reference rules

- Inside the locus body: `self.x` reads the current value;
  `self.x = expr` writes it.
- From outside the locus (e.g. a parent's `accept(g: GreeterL)`
  body): the parent can read fields the child has declared
  `expose` in its contract; reading other fields is a
  compile-time error per **F.17**.

## Capacity parameters

When the four numeric capacity parameters (`B`, `c`, `sigma`,
`phi`) are declared with their canonical types (`Int`, `Int`,
`Int`, `Float`), the locus gains `self.k_max: Float` as a
built-in computed field per **F.16**:

```text
self.k_max = B / [(1 - phi) * c + phi * sigma]
```

The capacity parameters are mutable; `k_max` floats with them.
A locus that adjusts `phi` at runtime sees `k_max` move
correspondingly.

If any of the four params are missing, `self.k_max` is not
available; referencing it is a compile error. If the
denominator evaluates to zero at runtime, the runtime emits a
typed error rather than producing `NaN` or `Inf`.

## Locus state shape at runtime

The parameter struct is the locus's runtime memory layout. The
codegen emits an LLVM struct with one field per param, in
declaration order, with alignment per the field types. The
locus handle is a pointer to this struct.

Allocations made in the locus body (string concatenations,
heap-typed values) live in the locus's arena alongside the
parameter struct.

## See Also

- [Locus declarations](./index.md)
- [Lifecycle methods](./lifecycle.md)
- [Memory model](../memory.md)
