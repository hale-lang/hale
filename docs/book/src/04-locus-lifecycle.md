# Locus lifecycle

Every [locus](../../reference/src/glossary.md#locus) lives through
the same four beats, in order:

1. **`birth`** — the locus has just come into existence.
2. **`run`** — the locus is doing whatever ongoing work it came to
   do.
3. **`drain`** — the locus is winding down: outstanding work
   completes; new work is no longer accepted.
4. **`dissolve`** — the locus departs; its
   [arena](../../reference/src/glossary.md#arena) is freed
   wholesale.

This shape is fixed. A locus declares method bodies for whichever
beats it cares about — the others are skipped — but the beats
themselves are not optional. The runtime always advances through
them in order.

This chapter walks each beat, then introduces the parent-child
relationship and the depth-first cascade (**F.4**) that connects
the lifecycles of parents and their descendants.

## Declaring lifecycle methods

A locus body lists each lifecycle method by name. None are
required, but each is bound to a specific beat:

```aperio
locus WorkerL {
    birth() {
        println("starting up");
    }

    run() {
        // main work loop
    }

    drain() {
        println("winding down");
    }

    dissolve() {
        println("goodbye");
    }
}
```

There is also a fifth method, `accept`, that runs at parent-side
lifecycle boundaries. It is covered later in this chapter
(see [Parents and children](#parents-and-children)).

## `birth()`

Runs exactly once, when the locus is instantiated. Any
initialization the locus needs — opening connections, reading
configuration into local variables, publishing a "ready" message
on the bus — happens here.

`self` is fully populated: every parameter has either its caller-
supplied value or its declared default. The arena is allocated.
The locus is ready to be addressed.

```aperio
locus HelloL {
    params {
        greeting: String = "hello, world";
    }

    birth() {
        println(self.greeting);
    }
}
```

A locus with only a `birth` body — like `HelloL` above — exists
for the duration of one `birth()` call and then dissolves. This
is the smallest viable locus shape, and it is the entire pattern
of `examples/hello-world`.

## `run()`

Runs once after `birth`, and the locus is considered alive for as
long as `run` is executing. Most loci that do ongoing work — a
ticker, a server, a long-running aggregator — put it here.

```aperio
locus TickerL {
    params {
        n: Int = 5;
        interval: Duration = 1s;
    }

    run() {
        let mut i: Int = 0;
        while i < self.n {
            println("tick ", i);
            time::sleep(self.interval);
            i = i + 1;
        }
    }
}
```

When `run()` returns, the locus naturally proceeds to `drain` and
then `dissolve`. There is no "stop running" signal a locus has to
listen for in the simple case — return from `run` and the
lifecycle continues.

A locus with no `run` body skips this beat entirely. After
`birth()`, it goes directly to `drain` (or `dissolve` if no
`drain` body is declared either).

## `drain()`

Runs after `run` returns and before `dissolve`. The locus is
winding down; this is where any cleanup that needs to *complete
work* (as opposed to *release resources*) happens — flushing
buffered output, sending a "leaving" notification on the bus,
finalizing aggregates.

The runtime gives `drain` two specific guarantees:

- **No new bus messages will be delivered to this locus** after
  `drain` begins. Subscriptions are removed.
- **All children have already drained and dissolved.** This is
  **F.4** — the depth-first cascade — covered below.

```aperio
locus ChildL {
    params { tag: String = "child"; }

    birth()   { println(self.tag, ": birth"); }
    drain()   { println(self.tag, ": drain"); }
    dissolve(){ println(self.tag, ": dissolve"); }
}
```

If a locus has neither `run` nor `drain`, the beat is skipped.

## `dissolve()`

Runs last. After this body returns, the locus's arena is freed
wholesale — every allocation made anywhere within the locus's
existence ceases to exist in the same instant.

`dissolve()` is the right place for irreversible "released the
last hold" operations: closing a file, releasing a hardware
resource, decrementing a counter visible to a parent.

It is *not* the right place for finalizing application-level
work — that goes in `drain`, where the locus is still
addressable and the runtime is still happy to deliver any
in-flight (already-routed) messages from before the
subscription was removed. Once `dissolve` runs, the locus is on
its way out the door.

A locus with no `dissolve` body simply has its arena freed
when this beat is reached; no user code runs.

## Parents and children

Loci compose. A locus instantiated *inside another locus's
lifecycle method* becomes that locus's child:

```aperio
locus CoordinatorL {
    params {
        B: Int = 100;
        c: Int = 1;
        sigma: Int = 1;
        phi: Float = 1.0;
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
```

The three `GreeterL { ... }` constructions inside `run()` attach
to `self` (the coordinator), not to the lifecycle method's local
scope. Each becomes a child of the coordinator.

> The `B`, `c`, `sigma`, `phi` parameters above are the
> coordinator's **capacity parameters** — they are how the
> compiler computes `k_max`, the maximum number of children
> allowed (per **F.1**). They appear here without explanation;
> [chapter 5](./05-contracts-and-parents.md) covers them in
> detail.

### `accept(child: ChildType)`

A parent declares an `accept` method to run code each time a
child is attached. **F.7**: `accept` runs *before* the child's
`birth`. The parent gets the chance to wire the child into its
own state before the child has executed any user code of its
own.

If a parent does not declare `accept`, children attach silently.
If a parent does, `accept` runs once per child, in attachment
order.

### Parent arena nesting

A child locus's arena is a **subregion** of its parent's. The
child cannot allocate outside its arena; the parent cannot reach
into the child's arena. Allocations live in their owner's region
and are freed when that region is freed.

This is what makes the lifetime guarantees stick: no child's
allocation can outlive the parent's arena because the child's
arena lives *inside* it.

## F.4: depth-first dissolve cascade

When any locus dissolves, its descendants dissolve first.
Specifically: the runtime walks the locus's children, dissolves
each one (which recursively dissolves their children, in turn),
and only then runs the parent's own `drain` and `dissolve`.

Concretely, for a parent with two children:

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

The output is:

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

`child-a` is constructed in `run`; in v0 children are
synchronously instantiated, so `child-a` runs its full
lifecycle (`birth` → `drain` → `dissolve`) before the next
statement in the parent's `run` body executes. By the time the
parent's `run` returns, both children have fully dissolved.
Then the parent's `drain` and `dissolve` run.

The guarantee — **F.4 depth-first cascade** — is that *every
descendant has finished dissolving before any ancestor's
`drain` runs*. A parent can rely on its children's state being
fully wound down without a "have-they-finished?" check; the
runtime makes that check for you.

This is how Aperio rules out a class of concurrent-shutdown bug
that classical actor systems have to handle with explicit
supervision protocols.

## What's not in this chapter

- **`accept`'s contract surface** — how `accept(g: ChildL)`
  reads the child's exposed contract fields (e.g. `g.greeting`)
  — is the subject of
  [chapter 5](./05-contracts-and-parents.md), where the
  three-way interface (**F.14**) is introduced.
- **Long-running children** — children whose lifecycle outlives
  a single statement in the parent's body — appear when the
  bus is introduced, in [chapter 6](./06-the-bus.md).
- **`on_failure`** — what happens when a child's lifecycle
  ends in a `ClosureViolation` rather than a clean dissolve —
  is covered in
  [chapter 12](./12-recovery-and-supervision.md).

The next chapter, **[Contracts and
parents](./05-contracts-and-parents.md)**, introduces the
parent-child *interface* — how a parent reads a child's
exposed state, why `B` / `c` / `sigma` / `phi` are the shape
they are, and what **F.14** (the three-way interface) means in
practice.
