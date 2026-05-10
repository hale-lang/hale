# Lifecycle methods

## Synopsis

Every locus's existence has a fixed shape: four lifecycle beats
(`birth` → `run` → `drain` → `dissolve`) plus an optional
parent-side beat (`accept`) that fires when a child is attached.
A locus declares method bodies for the beats it cares about; the
others are skipped.

## The lifecycle quartet

```text
birth → [accept(child)] → run → drain → dissolve
                       ↓
              children's lifecycles
```

| Method | Fires | Purpose |
|---|---|---|
| `birth()` | Once, after instantiation completes | Initialization |
| `accept(child: ChildType)` | Once per attached child, *before* the child's `birth` | Wire the child into parent state |
| `run()` | Once after `birth` returns | Main work |
| `drain()` | Once after `run` returns, *after* all descendants have dissolved | Wind-down |
| `dissolve()` | Once after `drain` returns | Final cleanup, before arena free |

After `dissolve` returns, the runtime frees the locus's arena
wholesale.

## Grammar

```text
lifecycle-method ::= birth-method | accept-method | run-method
                  | drain-method | dissolve-method

birth-method    ::= "birth"    "(" ")"             block
accept-method   ::= "accept"   "(" Ident ":" type-expr ")" block
run-method      ::= "run"      "(" ")"             block
drain-method    ::= "drain"    "(" ")"             block
dissolve-method ::= "dissolve" "(" ")"             block
```

Methods take no arguments other than the implicit `self` (and,
in the case of `accept`, the typed child binding).

## Semantics

### `birth()`

Runs exactly once. The locus's arena is allocated; parameters
are populated (caller-supplied values where given; declared
defaults otherwise); `self` is fully addressable. Bus
subscriptions register at the end of `birth`.

If no `birth` body is declared, the runtime inserts a no-op.

### `accept(child: T)`

Per **F.7**, runs *before* the child's `birth`. The parent gets
the chance to wire the child into its state — read the child's
exposed contract surface, store a reference, decrement
counters — before the child has executed any user code.

Multiple `accept` declarations are permitted, one per child
type. The runtime dispatches to the matching declaration based
on the constructed child's static type.

### `run()`

Runs once after `birth`. The locus is "alive" while `run`
executes; bus subscribers stay subscribed; the locus may publish.

When `run` returns, the lifecycle proceeds to `drain`.

If no `run` body is declared, the beat is skipped.

### `drain()`

Runs after `run` returns, with two runtime guarantees:

- **No new bus messages** are delivered to this locus during
  or after `drain`. Subscriptions are removed at the start of
  `drain`.
- **All descendants have already dissolved.** This is **F.4**
  — the depth-first cascade. By the time `drain` runs, every
  child the locus ever accepted has already executed its full
  lifecycle.

If no `drain` body is declared, the beat is skipped.

### `dissolve()`

Runs last. After this body returns, the runtime frees the
locus's arena. This is the right place for irreversible
"released the last hold" operations — closing files, releasing
hardware, decrementing counters visible to a parent.

If no `dissolve` body is declared, the runtime simply frees
the arena.

## F.4: depth-first dissolve cascade

When any locus dissolves, its descendants dissolve first.
Every child the locus has accepted has its full lifecycle
(`drain` → `dissolve`) executed before the parent's own
`drain` runs.

For a parent constructing two children inside `run()`:

```aperio
locus ParentL {
    accept(g: ChildL) {}
    birth()    { println("parent: birth"); }
    run()      {
        ChildL { tag: "child-a" };
        ChildL { tag: "child-b" };
    }
    drain()    { println("parent: drain"); }
    dissolve() { println("parent: dissolve"); }
}
```

Output:

```text
parent: birth
child-a: birth
child-a: drain
child-a: dissolve
child-b: birth
child-b: drain
child-b: dissolve
parent: drain
parent: dissolve
```

(In v0, child loci constructed inside a parent's `run` body
execute synchronously through their full lifecycle before the
next statement in the parent's body runs.)

## Long-lived loci

A locus that subscribes to the bus is *long-lived*: its
lifecycle is not torn down at the end of the construction
statement. It remains alive — receiving bus messages, running
handlers — until the enclosing scope's drain cascade reaches
it.

Long-lived loci are the typical shape for loci that exist for
the duration of `main` (constructed in `main`'s body, dissolved
when `main` returns).

## See Also

- [Locus declarations](./index.md)
- [Bus blocks](./bus.md)
- [Closures](./closures.md)
- [Recovery operations](../recovery/index.md)
