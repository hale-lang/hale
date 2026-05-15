# The locus

> **α** — What is a locus, and why is *everything* one?

The locus is the single structural primitive Aperio gives you.
Apps are loci. Services are loci. Handlers, caches, pools,
queues, namespaces, schedulers, libraries — all loci. There is
no `class`, no `module`, no `actor`, no `package`. There's one
shape, and you compose it.

## Anatomy

A locus is a typed unit with up to seven kinds of members. None
are required; you opt in to the ones you need.

```aperio
@form(vec)                              // optional: form lowering
locus Matchmaker : projection chunked,  // optional: annotations
                   schedule cooperative {

    params {                            // declared state
        target_size: Int = 4;
    }
    contract {                          // typed surface across the boundary
        expose pending_count: Int;
    }
    bus {                               // typed pub/sub interface
        subscribe JoinQueue as on_join;
        publish   MatchReady;
    }
    capacity {                          // bounded storage discipline
        heap waiting of Player;
    }

    birth()       { /* setup */ }       // lifecycle: 5 methods
    accept(c: T)  { /* on child arrival */ }
    run()         { /* steady state */ }
    drain()       { /* prepare to dissolve */ }
    dissolve()    { /* teardown */ }

    on_failure(c: T, err: Error) { ... }    // recovery policy

    mode bulk(...)       -> ... { ... }      // optional: kernel projections
    mode harmonic(...)   -> ... { ... }
    mode resolution(...) -> ... { ... }

    closure books_balance {              // structural invariants
        sum(intent.pnl) ~~ sum(book.pnl) within 0.05d;
    }

    fn on_join(p: Player) { ... }        // member functions
}
```

You'll never use all of these in one locus. Most loci use three
or four. The point of the surface isn't completeness — it's that
every distinct *kind* of structural commitment a unit can make
has a syntactic home. State goes in `params`. What crosses the
parent ↔ child boundary goes in `contract`. What goes over the
bus goes in `bus`. Bounded storage goes in `capacity`. Failure
policy goes in `on_failure`. Invariants that must hold across
the locus's lifetime go in `closure`. Each commitment is
declared, not inferred from code.

## Walking through the surface

**`params`** is the locus's state. It's both *initialized* at
construction (`Matchmaker { target_size: 8 }`) and *mutated* at
runtime (`self.target_size = 6;` inside a method). Aperio
collapses the parameter/state distinction the way Ruby
collapses parameter/`@foo`-instance-variable. There is no
separate `state` block.

**`contract`** declares what crosses the boundary between this
locus and its parent. `expose` is what the parent can read;
`consume` is what the parent must provide (when this locus is
itself the parent of children that expose the named field). The
contract is the *only* surface the parent sees — internal state
not exposed is invisible.

**`bus`** declares typed pub/sub. `subscribe Topic as handler`
binds an incoming message stream to a handler function on the
locus body. `publish Topic` authorizes outbound sends on that
topic via `Topic <- payload;`. Subjects are first-class typed
declarations (`topic JoinQueue { payload: Player; }`), not
strings.

**`capacity`** declares bounded storage other than the locus's
implicit arena. `pool X of T;` is fixed-shape cell recycling.
`heap Y of T;` is growable storage individually freed during
the locus's lifetime. The `@form(...)` annotation on the locus
picks a high-level lowering — `@form(vec)` over a `heap` slot
synthesizes `push` / `pop` / `len` methods; `@form(hashmap)`
over a `pool` slot synthesizes keyed-store methods. You'll
choose between forms based on access pattern; you don't
write the storage code yourself.

**Lifecycle methods** are not regular `fn`s. They're
state-machine transitions the runtime invokes:

- `birth()` runs once at construction.
- `accept(c)` runs when a child locus is attached (per parent
  policy; see the next chapter).
- `run()` is the steady-state loop, if any.
- `drain()` halts new work but lets in-flight finish.
- `dissolve()` tears down the locus's region.

Every locus has all five available; the compiler supplies
defaults for any you omit (`birth` no-ops, `dissolve` frees the
region, etc.).

**`on_failure(c, err)`** is the parent's recovery policy when a
child fails. The handler chooses among `restart`, `quarantine`,
`bubble`, `dissolve`, or absorbs by returning normally. (Failure
itself is covered in detail in
[The two failure channels](./failure.md).)

**`mode bulk` / `mode harmonic` / `mode resolution`** are three
named projections of the same kernel computation — vectorized
bulk processing, per-class projection, single-decision
resolution. A locus declares whichever subset it operates in;
they share state through the same arena. You'll rarely declare
all three.

**`closure`** is a *structural invariant* that must hold across
some declared epoch (e.g., every dissolve, every tick, every
duration window). The `~~` operator means "approximately
equal within tolerance." A closure that fails routes through
`on_failure` like any other structural failure.

Closures also serve as *named structural-failure types* that
member functions can fire inline. The `epoch inline` variant
declares a closure whose only firing mode is explicit
`violate NAME` from a method body; an optional `captures: f1, f2`
clause names locus state to snapshot into the violation payload.
This shape is the bridge between the value channel and the
structural channel — covered in detail in
[The two failure channels](./failure.md). *(`epoch inline`,
`violate`, and the `captures:` clause are shipping in v1.x; the
spec change is `F.27` in `spec/design-rationale.md`.)*

## `locus` vs `type`

If you've gotten this far you may be wondering when to use a
locus vs Aperio's other declarative primitive, `type`.

```aperio
type Player { id: String; name: String; }
```

`type` is **pure shape**. A record. No lifecycle, no flow, no
state machine, no bus participation. Construct, pass around by
value, compare. The bus carries types as payloads. Your locus's
`params` are typed by types.

`type` and `locus` are not parallel categories — they're
*points on a gradient*. A type is a locus in proto-form: shape
declared, but no flow attached yet. If the thing you're
modeling starts as data and grows lifecycle (a `Cache` that's
loaded / probed / evicted; an `Order` that's submitted /
filled / cancelled), you don't bolt methods onto the `type` —
you promote it to a `locus`. There is no third primitive.

## The one-tower rule

The deepest commitment Aperio makes about modeling is this:

> Every named quantity in your model must be assignable to
> exactly one locus in one locus tower.

State that "lives between" loci — a global variable, a shared
mutable buffer, a side-channel cache nobody owns — is a signal
of modeling error, not a framework gap. When the language
seems to resist where you want to put a piece of state, the
productive move is to find the locus that *should* own it, not
to invent a workaround.

This rule exists because every other guarantee Aperio makes
depends on it. Wholesale region freeing at dissolve, vertical-
only flow, the closure-violation channel, the deterministic
cleanup cascade — all of them assume each piece of state has
exactly one owning locus. When state floats, those guarantees
unravel at the floating point.

The rule is also what enables the structural correspondence
you saw in the intro. When the mental model says "the
matchmaker holds the queue," it's because the queue belongs to
exactly one tower. The locus surface lets you write that down
directly.

[Modeling — how to think in Aperio](./modeling.md) develops
this rule into concrete patterns and points at a forthcoming
companion library that helps you make ownership decisions
explicit.

## Next

The next chapter, [Recursive composition](./recursive-composition.md),
shows how loci nest inside loci, what crosses the boundary,
and why flow is *vertical-only* — siblings never see each other
directly.
