# Recursive composition

> How do loci nest inside loci, and why is flow
> vertical-only?

A program built from loci is a *tree*. The runtime root is at
the top; `main`'s implicit locus is one level down; the loci
that `main` instantiates are below that; their children are
below them. Every running Hale program — your `cli-demo`,
your matchmaker, your trading system — is a tower of loci,
arbitrarily deep.

This chapter covers how the nesting works, what crosses the
boundary between parent and child, and the single rule that
makes the whole structure tractable: **flow is vertical-only.**

## Parent and child

A parent locus declares interest in a child type by
implementing `accept`:

```hale
locus Matchmaker {
    params { target_size: Int = 4; }
    // ... bus / capacity / etc.

    accept(g: GameSession) {
        // runs BEFORE g's region is allocated; can reject
        // by returning early or routing through on_failure
    }
}
```

The child is brought into being by an instantiation literal:

```hale
locus Matchmaker {
    // ...
    fn on_join(p: Player) {
        self.waiting.push(p);
        if self.waiting.len() >= self.target_size {
            GameSession { players: drain_players(self) };
        }
    }
}
```

When `GameSession { ... }` is evaluated inside a parent's
method body, the runtime:

1. Runs `accept(g)` on the parent. If it returns normally,
   the child proceeds.
2. Allocates the child's region as a sub-region of the
   parent's. (Region details in
   [Capacity & storage](./capacity-storage.md).)
3. Runs `birth()` on the child synchronously.
4. Schedules `run()` to begin.

When the parent eventually drains, every child drains first
(depth-first), then the parent does. Region cleanup is
wholesale and deterministic.

## What crosses the boundary

The **contract block** is the typed surface that bridges parent
and child:

```hale
locus GameSession {
    params { players: [Player]; tick_count: Int = 0; }
    contract {
        expose tick_count: Int;       // parent can read
        expose state: SessionState;
        consume time_source: Time;    // parent must provide
    }
    // ...
}

locus Matchmaker {
    contract {
        expose pending_count: Int;
        consume time_source: Time;    // routes through to GameSession
    }
    accept(g: GameSession) {
        // g.tick_count and g.state are visible here
        // — they're contract-exposed by g.
        if g.tick_count > 1000 {
            // ...
        }
    }
}
```

The rule is strict: **the parent sees only what the child
exposes.** Internal state not named in the contract is
invisible from outside the child. Conversely, the child reads
into its parent only via `consume` entries that the parent
agrees to provide.

This is not a convention enforced by reviewers. The
typechecker rejects an attempt to read `child.private_field`
when `private_field` isn't in the contract. You don't have to
think about hiding; the structural boundary does the hiding
for you.

## Vertical-only flow

Here's the single rule the whole compositional model rests on:

> Within a locus tower, **flow is vertical only.** Parents
> read into children through the contract; children write
> upward through the contract. Siblings do not see each other
> directly. Cousins do not see each other directly. There is
> no lateral flow within a tower.

If two siblings need to coordinate, they don't reference each
other. They route through their shared parent:

```hale
locus Matchmaker {
    accept(g: GameSession) { /* ... */ }

    fn handle_game_end(g_id: String, winner: Player) {
        // siblings — the game-sessions — do not call each other.
        // The matchmaker (parent) mediates: it has both games
        // visible via self.children, and it can publish to
        // whichever subjects each needs.
    }
}
```

A parent can iterate its accept'd children with
`for child in self.children { ... }`, and read two summary
accessors without a loop: `self.children.count` (an `Int`) and
`self.children.is_empty` (a `Bool`). Both are valid only inside a
method of a locus that `accept`s a child type.

If sibling coordination is *common* enough that routing through
the parent feels like ceremony, the language is telling you the
parent is missing logic. The `Matchmaker` should be the place
that knows how games coordinate with each other — that's
exactly the role it's in.

The rule exists because the substrate's other guarantees
require it:

- **Memory safety** without a garbage collector or borrow
  checker. Wholesale region cleanup at dissolve works because
  no pointer crosses sideways. Two siblings can dissolve in
  either order without worrying about one's pointer dangling
  into the other.
- **Failure traversal**. When a child fails, the failure flows
  up to the parent's `on_failure`, never sideways. The whole
  tree's recovery policy is local; no failure can reach a
  sibling without first being absorbed (or escalated) by the
  shared parent.
- **Reasoning at scale**. When you look at a locus, you know
  every coordination path: down to its children, up to its
  parent. You never have to guess whether some sibling
  somewhere has a back-channel.

## The exception that proves the rule: the bus

You'll notice there's *one* mechanism in Hale that does
appear to let loci communicate without a direct
parent-child relationship: the bus. A subscriber on one
branch of the tree and a publisher on a completely
unrelated branch can both reference the same topic.

This is not a violation of vertical-only flow — it's the
*mediation* of lateral coordination through a substrate
that's structurally above both parties. The bus router
runs at the runtime root; topics are declared globally;
every send and every dispatch passes through a substrate
locus higher than any subscriber. The two loci don't see
each other; they see the topic, which the substrate sees.

This is how Hale reconciles "everything is a tower of
vertical relationships" with "real systems need
many-to-many event flow." The bus is covered in detail in
[The bus](./the-bus.md).

## Region nesting

A side effect of strict vertical flow is that memory nests
the same way the loci do. Each locus owns a region; each
child's region is a *sub-region* of its parent's:

```
  runtime root region
  ├── main's implicit-locus region
  │   ├── Matchmaker region
  │   │   ├── GameSession A region
  │   │   ├── GameSession B region
  │   │   └── GameSession C region
  │   └── (other top-level loci)
```

When a locus dissolves, its entire sub-tree of regions is
freed wholesale. No traversal, no per-object cleanup, no
"did I forget something?" — the cleanup is structural.

This is one of the load-bearing reasons Hale doesn't need
a garbage collector or a borrow checker. The hierarchy
*is* the ownership graph; vertical-only flow guarantees
no foreign pointer crosses the boundaries; wholesale
free-on-dissolve is sound.

## Next

The next chapter, [The bus](./the-bus.md), covers how typed
pub/sub flows through the substrate and connects loci that
have no direct parent-child relationship — without
violating the vertical-flow rule that makes the whole
structure tractable.
