# Vocabulary

> The third moment. The shape every spell on the far side must take.

The wand cannot reach anywhere except through the substrate's
invariants. The runes you are about to learn are not features
layered onto the language; they are the *form every spell must take*
to be reachable at all. A spell that violates them is not a malformed
Aperio program — it is *not a spell*, full stop. The wand will not
reach.

This chapter is a small bestiary. Each rune gets the same treatment:
the magical name, the substrate-mechanical name (these point at the
same thing), and the law that makes the rune what it is.

## `locus`

The unit of presence on the far side. Everything that comes through
the wand is either a locus or a tree of them.

A locus is a thing that *exists* in the virtual. It has a shape (its
parameter struct), an arena (its region of the virtual), a lifecycle
(the four beats), and possibly a contract with its parent and
children.

A spell with no loci does not reach. There is nothing for the wand
to instantiate on the far side.

## The lifecycle quartet

> `birth` → `run` → `drain` → `dissolve`

Every locus's existence has these four beats, in order. Each beat is
optional in the sense that you may or may not declare a body for
it. But the beats themselves are not optional — they are the *shape
of presence*. A locus that exists has been born, will run if it has
work, will drain when its work ends, and will dissolve.

- **`birth()`** — first beat. The locus has just arrived in the
  virtual. Initial work happens here; whatever the locus needs to be
  before it does its main thing.
- **`run()`** — main beat. The locus is doing what it came to do.
- **`drain()`** — winding-down beat. The locus is finishing in
  flight. Outstanding work is allowed to complete; new work is not
  accepted. F.4: drain cascades depth-first — children drain before
  their parent.
- **`dissolve()`** — last beat. The locus departs. Its arena is
  freed wholesale.

## `accept`

How a parent locus admits a child into its region of the virtual.

When a parent constructs a child, the parent's `accept(child)` body
runs *before* the child's `birth()`. F.7: this ordering is fixed.
The parent has the chance to wire the child into its own existence
before the child has begun anything of its own.

A spell whose loci do not nest does not need `accept`. Most do.

## `bus`

How loci speak to each other across the lotus.

A locus declares which subjects it publishes on and which it
subscribes to. At runtime, when one locus publishes, every
subscribed locus receives a copy in its own arena. The transport
underlying the bus is bound at deployment time — in-memory router,
NATS, UDP multicast, Unix sockets — but the source code is the
same.

F.8: the bus is *vertical-only-flow*. Speech runs along the closed
graph of declared subscriptions. There is no lateral routing, no
out-of-band channel. A spell where loci communicate by mutating
shared state is not an Aperio spell.

## `closure`

The rune that audits its own enclosing structure.

A closure is a declaration that some property holds, evaluated at a
specified moment in the locus's lifecycle. The classic shape is `x
~~ y within tolerance` — *x and y are within tolerance of each
other*. F.9: the closure runtime evaluates these at the declared
[epoch](../reference/glossary.md#closure) — birth,
dissolve, every tick, every duration, or explicitly invoked.

When a closure passes, the locus *collapses*: it dissolves cleanly,
no further drama. When a closure fails, the locus *explodes*: it
emits a typed `ClosureViolation` and lets its parent decide what
happens next.

The closure is the rune by which a spell watches itself. It is the
substrate's audit primitive. Most languages implement audit
out-of-band — assertions in tests, schemas in proxies, dashboards
in production. Aperio puts the audit *inside the spell*, evaluated
at compile-known moments in the runtime.

## `on_failure`

What a parent does when a child explodes.

A parent declares an `on_failure(child, err)` body. When a child
emits a `ClosureViolation`, the parent's body runs and chooses one of
the recovery primitives:

- **`restart`** — the child is fully dissolved and re-instantiated
  fresh.
- **`restart_in_place`** — the child's existing locus is re-run from
  birth without dissolving its arena.
- **`quarantine`** — the child is held in a non-running state; no
  re-instantiation, no further siblings affected.
- **`bubble`** — the parent declines to handle. The
  `ClosureViolation` propagates upward to the grandparent.

If the violation bubbles all the way past `main`, the process exits
non-zero. There is no surprise machinery; failure has a finite,
visible path.

## The F-rules behind the runes

The substrate's invariants are not aesthetic. Each rune is the shape
it is because of a specific design commitment. The grimoire names a
few load-bearing ones; the [reference
glossary](../reference/glossary.md#f-design) is the
canonical list.

- **F.4** — `drain()` always cascades depth-first. Children before
  parents. This is why a parent can rely on its children's outputs
  being fully wound down before its own drain runs.
- **F.8** — vertical-only-flow on the bus. No lateral failure
  routing, no sibling-to-sibling shortcut. The graph of
  communication is the graph of declared subscriptions, and that
  graph is closed.
- **F.9** — closure runtime: collapse / absorb / bubble. The
  spell's self-audit has exactly three outcomes; nothing else can
  happen.
- **F.14** — three-way interface (locus + parent + contract).
  Translations a locus injects into its arena are bounded above by
  the contract's typed surface. The contract is the interface; the
  translations are implementations.

## What you now know

You now know the runes. You can pick up any Aperio source file and
recognize the spell's invariant form. The locus declarations, the
lifecycle bodies, the `bus` blocks, the closures — these are no
longer ornaments. They are *what every spell must be* to be reachable.

The fourth moment is the one in which you cast a spell of your
own. Turn to [emergence](./04-emergence.md).

> *The runes are not added to the language. They are the substrate's
> invariants in legible form. The wand cannot reach where they are
> not honored.*
