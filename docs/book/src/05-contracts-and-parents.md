# Contracts and parents

[Chapter 4](./04-locus-lifecycle.md) introduced the parent-child
relationship at the lifecycle level: how a child locus instantiated
inside a parent's body becomes that parent's child, how `accept`
runs before `birth`, how the depth-first cascade (**F.4**) ensures
children dissolve before their parents.

This chapter goes one layer deeper. The parent-child relationship
has two parts: a **capacity** the parent can hold, and an
**interface** the parent presents to its children. Both are
declared at the source level and enforced by the type checker.

- Capacity is described by four parameters — `B`, `c`, `sigma`,
  `phi` — from which the compiler derives `k_max`, the maximum
  number of children the parent can attach.
- Interface is described by a `contract` block on each side — the
  child *exposes* fields, the parent *consumes* them.

The two together implement the substrate's load-bearing **F.14:
three-way interface** (locus + parent + contract), where the
contract itself is a first-class entity, not just a name shared
between two loci.

## Capacity parameters

A parent locus declares four numeric parameters that describe its
capacity:

```aperio
locus CoordinatorL {
    params {
        B: Int = 100;        // budget
        c: Int = 1;          // attach cost per coordinatee
        sigma: Int = 1;      // summary cost per coordinatee
        phi: Float = 1.0;    // formality
    }
    // ...
}
```

Their meanings:

- **`B`** — *budget*. The total resource the parent has to spend
  on coordinating children.
- **`c`** — *cost per attached coordinatee*. The fixed overhead
  of holding a child active.
- **`sigma`** — *summary cost*. The marginal cost of formally
  summarizing a child's state through the contract.
- **`phi`** — *formality*. A `Float` between 0.0 and 1.0
  describing how thoroughly the parent enforces the contract:
  `0.0` = barely (mostly informal access), `1.0` = fully (every
  child interaction goes through the contract surface).

These are not arbitrary configuration values. They are the
**ancient texts' named-concept registry** made executable. The runtime
uses them to compute `k_max`.

## `self.k_max`

Every locus that declares the four capacity parameters as numeric
fields gets `self.k_max` as a built-in computed field of type
`Float` (per **F.16**):

```text
self.k_max = B / [(1 − phi) · c + phi · sigma]
```

`k_max` is the maximum number of children the parent can attach.
The capacity parameters are mutable (`self.B = ...`,
`self.phi = ...` are valid inside the locus body), so `k_max`
floats with them — a coordinator that adjusts `phi` at runtime to
formalize its interface sees `k_max` move correspondingly.

A worked numerical example, from the design rationale:

```text
B=100, c=10, sigma=1, phi=0.5
  → k_max = 100 / [0.5 · 10 + 0.5 · 1]
         = 100 / 5.5
         ≈ 18.18
```

This is the small-`k` mixed-formality regime: a coordinator that
can attach roughly eighteen children when it spreads its budget
half between informal attachment overhead and formal contract
summaries.

The runtime errors cleanly on a missing capacity param or a zero
denominator (rather than producing `NaN` or `Inf`); the type
checker enforces that `k_max` is `Float` on every locus that
declares the four params.

A closure (introduced in [chapter 7](./07-closures.md)) can audit
against `k_max` directly:

```aperio
closure within_capacity {
    self.children.length ~~ 0 within self.k_max;
}
```

— "the number of children should be within `k_max` of zero," i.e.
not exceed `k_max`. This is the substrate's signature equation
made into an executable language primitive.

## Contract blocks

A contract declares the typed surface that crosses between a child
and a parent. Both sides participate:

```aperio
locus GreeterL {
    params {
        greeting: String = "hello";
    }

    contract {
        expose greeting: String;
    }
}

locus CoordinatorL {
    params {
        B: Int = 100;
        c: Int = 1;
        sigma: Int = 1;
        phi: Float = 1.0;
    }

    contract {
        consume greeting: String;
    }

    accept(g: GreeterL) {
        println("greeting from child: ", g.greeting);
    }

    run() {
        GreeterL { greeting: "hello" };
        GreeterL { greeting: "hi" };
        GreeterL { greeting: "yo" };
    }
}

fn main() {
    CoordinatorL { };
}
```

The mechanics:

- **Child `expose X: T`** — the child declares that it makes a
  field named `X` of type `T` visible to its parent.
- **Parent `consume X: T`** — the parent declares that it
  requires a field named `X` of type `T` from its children.
- **Compatibility check** — for each pairing between this parent
  and this child, the type checker verifies that the child's
  exposed `X` has a type the parent's consumed `X` accepts. (For
  v0, "accepts" is type equality. Future versions may admit
  covariant or contravariant relationships.)
- **`accept(g: ChildType)`** — the parent's `accept` body
  receives the child as a typed value, and reads consumed fields
  from it via field access (`g.greeting`).

If a child does not expose what a parent consumes, the
construction is a compile-time error. A child can be attached
only to a parent whose consumed surface it satisfies. This is
the typing-rule expression of **F.8** — vertical-only-flow at
the contract level.

## F.14: the three-way interface

The contract is a *first-class entity*, not merely a name shared
between locus declarations. **F.14** spells this out:

1. **The locus** owns its arena, its state, and its translation
   *implementations* — the code that produces contracted values
   from internal state.
2. **The parent** receives translated values through the
   contract; it cannot see the locus's internal state directly,
   only what the contract surfaces.
3. **The contract** declares the typed surface. It bounds what
   the locus's translation implementations are permitted to
   return — every function the locus injects into its arena that
   satisfies a contract entry must return a type the contract
   permits.

The split is interface vs implementation, made structural:

| Role | Entity |
|---|---|
| Interface | the contract |
| Implementation | the locus's translation function (or the param itself) |
| Observer | the parent |

What this gives Aperio:

- **No backdoor.** A locus cannot route around the contract by
  exposing some internal state directly to its parent; the
  contract is the source of truth for what crosses the D/D−1
  boundary.
- **Multiple implementations of the same field can coexist.** A
  contract field `volume: Decimal` can have a "rich" translation,
  a "chunked" translation, and a "recognition" translation — three
  different implementations all returning `Decimal`. The parent
  asks for whichever it wants; the contract bounds them all.
  (Multi-implementation syntax is deferred to a future version;
  for v0 the commitment is the typing rule. See
  [chapter 11](./11-perspectives.md) for projection classes,
  which make multi-implementation natural.)
- **Vertical-only flow at the query level.** A grandparent at
  D−2 cannot reach past its child at D−1 directly into the
  grandchild's arena at D. Every cross-boundary read goes
  through one contract at a time.

For v0, contract fields default to *the param itself as the
implementation*: declaring `expose greeting: String` and having
a param named `greeting: String` is the default-implementation
case. User-defined fns can add additional implementations as
long as they return the contract's typed surface; for v0 there
is one implementation per field, the param.

## What this looks like at runtime

Putting it together, the `02-parent-child` example flow:

1. `main` constructs a `CoordinatorL`.
2. `CoordinatorL`'s `birth` runs (none declared, skipped).
3. `CoordinatorL`'s `run` body executes:
   - `GreeterL { greeting: "hello" }` is constructed as a child.
     The type checker has already verified at compile time that
     `GreeterL` exposes the `greeting: String` that
     `CoordinatorL` consumes.
   - **F.7 ordering**: the parent's `accept(g: GreeterL)` runs
     first, reading `g.greeting` (which is "hello" via the
     default-implementation rule).
   - The greeter's `birth` would run next (none declared,
     skipped). Then `drain`, `dissolve` (none, none).
   - The same dance for "hi" and "yo".
4. After `run` returns, `CoordinatorL`'s `drain` runs (none,
   skipped).
5. `dissolve` runs, the arena is freed, the program ends.

Output:

```text
greeting from child: hello
greeting from child: hi
greeting from child: yo
```

## What's not in this chapter

- **The `~~` operator and closures** — `closure
  within_capacity { ... }` was teased above as a way to audit
  `k_max` at runtime. The full closure surface is
  [chapter 7](./07-closures.md).
- **Multi-implementation contract fields** — `@projection
  rich fn greeting() -> String { ... }` style annotations land
  alongside projection classes in
  [chapter 11](./11-perspectives.md).
- **`self.children`** — iterating a parent's currently-attached
  children — appears starting in
  [chapter 7](./07-closures.md), where it is the usual
  scrutinee for `closure` audits over a population.

The next chapter, **[The bus](./06-the-bus.md)**, introduces
the substrate's *other* coordination axis — typed pub-sub on
named subjects, with **F.8** vertical-only-flow making the
graph of communication closed.
