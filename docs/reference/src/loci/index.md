# Locus declarations

## Synopsis

A locus is the unit of presence at runtime — every Aperio
program's running structure is a tree of loci. A `locus L { ... }`
declaration introduces a *locus type* that can be instantiated
at any statement position; instantiation produces a *locus
handle* whose lifetime is bound to the enclosing scope.

## Grammar

```text
locus-decl ::= "locus" PascalCase-Ident generic-params? annotations? "{"
                 locus-member*
               "}"

annotations ::= ":" annotation ("," annotation)*
annotation  ::= tier-ann | projection-ann | schedule-ann

tier-ann       ::= "tier" Int
projection-ann ::= "projection" projection-class
schedule-ann   ::= "schedule" schedule-class

projection-class ::= "rich" | "chunked" | "recognition"
schedule-class   ::= "cooperative" | "pinned" | "pinned" "(" "core" "=" Int ")"

locus-member ::= params-block
              | contract-block
              | bus-block
              | closure-decl
              | lifecycle-method
              | fn-member
              | mode-decl
              | on-failure-decl
```

A locus body may declare any number of each member type, in
any order.

## Members

| Member | See |
|---|---|
| `params { ... }` | [params](./params.md) |
| `contract { expose ... ; consume ... ; }` | (covered with declarations below) |
| `bus { subscribe ... ; publish ... ; }` | [bus blocks](./bus.md) |
| `closure NAME { ... }` | [closures](./closures.md) |
| `birth() / accept() / run() / drain() / dissolve()` | [lifecycle methods](./lifecycle.md) |
| `fn name(args) -> ret { ... }` | [fn members](./fn-members.md) |
| `mode bulk(...) / harmonic(...) / resolution(...)` | (mode declarations; see design rationale §7) |
| `on_failure(child: T, err: ClosureViolation) { ... }` | [recovery](../recovery/index.md) |

## Annotations

```aperio
locus CoordinatorL : tier 4, projection chunked, schedule cooperative {
    // ...
}
```

- **`tier`** — depth hint (advisory; not yet enforced in v0).
- **`projection`** — allocator strategy. See
  [perspectives](../types/perspectives.md#projection-classes).
- **`schedule`** — execution regime. `cooperative` is the
  default; `pinned` runs the locus on its own pthread.
  `pinned(core = N)` adds CPU affinity. See
  [runtime — scheduling](../runtime.md).

All annotations are optional. The default for a locus that
declares `accept` is `: projection chunked` if N is not
statically determinable; for all loci the default schedule is
`cooperative`.

## Contract blocks

Per **F.8** + **F.14**, contracts declare the typed surface
between a locus and its parent. Both sides participate:

```aperio
locus GreeterL {
    params { greeting: String = "hi"; }
    contract {
        expose greeting: String;
    }
}

locus CoordinatorL {
    params {
        B: Int = 100; c: Int = 1; sigma: Int = 1; phi: Float = 1.0;
    }
    contract {
        consume greeting: String;
    }
    accept(g: GreeterL) {
        println(g.greeting);
    }
}
```

The typechecker verifies that, for each parent's `consume X: T`,
the attached child has an `expose X: T` whose type is
compatible. (For v0, "compatible" is type equality.)

## Locus handles

Constructing a locus produces a *locus handle*. The handle
carries:

- A pointer to the locus's arena.
- The current values of its parameters.
- Bookkeeping for active subscriptions, child population, and
  closure state.

Locus handles are not first-class values — they are a runtime
notion the substrate manages. Source-level code interacts with
the locus through:

- Field access on `self` from within the locus body.
- Field access on a typed binding (e.g. `g.greeting` from a
  parent's `accept(g: GreeterL)` body).

## Capacity parameters

When a locus declares the four numeric capacity parameters
(`B`, `c`, `sigma`, `phi`) as `Int` / `Int` / `Int` / `Float`
fields, it gains `self.k_max: Float` as a built-in computed
field per **F.16**:

```text
self.k_max = B / [(1 - phi) * c + phi * sigma]
```

`k_max` is the maximum coordinatees the locus may attach. The
runtime errors cleanly on missing params or zero denominator.

See [params](./params.md) and `spec/design-rationale.md` for
the full F.1 / F.16 semantics.

## See Also

- [Lifecycle methods](./lifecycle.md)
- [Params](./params.md)
- [Closures](./closures.md)
- [Bus blocks](./bus.md)
- [Fn members](./fn-members.md)
- [Recovery operations](../recovery/index.md)
