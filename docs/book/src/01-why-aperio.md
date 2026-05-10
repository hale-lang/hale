# Why Aperio?

Aperio is a compile-time language whose primitives are coordination
primitives. Its type system, its memory model, and its failure model
all derive from one underlying object — a *lotus*: a tree of
[loci](../../reference/src/glossary.md#locus) communicating along a
closed graph, each owning a region of memory whose lifetime is the
locus's own.

This chapter explains the choice. What it means for a language to
be built that way, what kinds of bugs it removes by construction,
and what classical alternatives Aperio replaces. There is no code
yet; the second chapter introduces the first runnable program.

## Aperio and lotus

The two names are load-bearing and distinct:

- **Aperio** (Latin *I open / I reveal*) — the language. The thing
  you write source code in. The compiler and toolchain (`aperio
  build`, `aperio run`).
- **a lotus** — the runtime data structure an Aperio program *is*.
  An Aperio source file describes what shape a lotus will take when
  the program runs; the running program is a lotus inhabiting that
  shape.

Throughout this book, *Aperio* refers to the language at the source
level and *lotus* refers to the runtime structure. The split is
worth keeping straight — a few later chapters depend on the
distinction being clear.

## Substrate-up, not feature-up

Most general-purpose languages assemble themselves the same way:
syntax, type system, memory model, concurrency primitives. Each is
a layer; coordination patterns (actors, supervisors, message
queues, schemas) live above all of them as libraries or
frameworks.

Aperio inverts this. Its substrate — the lotus's coordination
primitives — is the language's foundation, not its target. The
locus, the lifecycle, the bus, and the closure are not library
types you import; they are constructs the type checker enforces.

In practice this means several patterns that classical languages
treat as conventions become *invariants the compiler holds for
you*. A child locus cannot outlive its parent's arena. A bus
subject cannot be subscribed against the wrong type. A closure
declares an audit at a specific epoch, and the runtime guarantees
that epoch is reached. There is no way to write a malformed lotus
that compiles.

## Bugs eliminated by construction

The substrate-up choice closes specific classes of bug at the
compiler boundary. The four most consequential:

### Memory leaks have nowhere to go

Each locus owns an
[arena](../../reference/src/glossary.md#arena) — a contiguous
region of memory that comes into existence with the locus and is
freed wholesale when the locus dissolves. Allocations made inside
the locus's body live in that arena; when the locus departs, so
does its arena.

There is no mechanism by which a locus's allocation can outlive
the locus that allocated it. The runtime does not have a "this
value escaped" path; the *shape of escape* is not expressible.
This is the substrate's substitute for a garbage collector or a
borrow checker — the locus boundary is the lifetime boundary, by
construction.

### No lateral failure routing

When a locus's
[closure](../../reference/src/glossary.md#closure) fails or its
work goes wrong, the runtime emits a `ClosureViolation`. That
violation propagates *upward* — to the locus's parent, who chooses
one of four recovery primitives: `restart`, `restart_in_place`,
`quarantine`, or `bubble`. Bubbling continues upward.

This is **F.8** — vertical-only-flow. It rules out an entire class
of distributed-systems bug in which a sibling unexpectedly
"handles" a failure and silently absorbs it. There is no
subscriber-handles-failure shortcut on the bus, no
peer-reaches-into-peer back channel. Every failure has a finite,
visible path: up, until it lands in a parent who handles it, or
until it exits the program.

### Audit invariants are part of the program

In most languages, structural invariants ("this counter and that
counter should match," "every issued ticket has been redeemed")
live in tests or runbooks. They are external; if they exist at
all, they exist in a different artifact from the production code.

In Aperio, a closure is a syntactic construct in the locus that
declares the invariant *and the moment it must hold*: at birth, at
dissolve, every tick, every duration, or when explicitly invoked.
The runtime enforces it. A failed closure is a typed
`ClosureViolation`, distinguishable at the source level from a
panic.

This is **F.9**. Audit moves from an out-of-band concern into the
program itself, evaluated at compile-known epochs.

### Drain has a deterministic order

When a parent locus drains, its children drain *first*. Children's
children drain first relative to those, and so on, depth-first
through the tree. This is **F.4**.

Most concurrent runtimes leave drain ordering to chance — actors
finish in whatever order their mailboxes empty; threads exit when
their work happens to complete. Aperio guarantees that by the time
a parent's `drain()` body runs, every child the parent has ever
accepted has already drained and dissolved. A parent can rely on
its descendants' state being fully wound down without a
"have-they-finished?" check.

## What Aperio replaces

Several patterns common in classical systems languages become
unnecessary in Aperio because the substrate provides what they
were built to handle:

| Classical pattern | Aperio's substitute |
|---|---|
| Garbage collection | Per-locus arenas freed at dissolve |
| Borrow checker | Lifetime = locus existence; escape is unrepresentable |
| Actor systems + supervision trees | Loci + `on_failure` with four finite recovery primitives |
| External assertions / runtime monitoring | Closures evaluated at compile-known epochs |
| Out-of-band schemas (Protobuf, OpenAPI) | Typed bus subjects + deployment-bound transports |
| Manual shutdown wiring | F.4 depth-first drain cascade, automatic |
| Custom retry / restart logic | `restart`, `restart_in_place`, `quarantine`, `bubble` |

Aperio is not built for every kind of program. It is built for
programs whose shape is a lotus — long-running, coordinated,
audited systems with structured failure handling. For programs
that fit, the language and the substrate co-design eliminates an
entire layer of glue code.

## Where this book goes from here

The remaining chapters introduce the substrate one primitive at a
time, in a layered tutorial:

- **[Hello, locus](./02-hello-locus.md)** — the smallest runnable
  Aperio program. A single locus, one lifecycle method, the
  toolchain end-to-end.
- **[Types and values](./03-types-and-values.md)** —
  builtins, user-defined records, the language's value model.
- **[Locus lifecycle](./04-locus-lifecycle.md)** — the four
  beats every locus lives through (`birth` / `run` / `drain` /
  `dissolve`).
- **[Contracts and parents](./05-contracts-and-parents.md)** —
  the parent-child relationship; F.7 and F.14.
- **[The bus](./06-the-bus.md)** — typed pub-sub; F.8
  vertical-only-flow; transport-bound-at-deployment.
- **[Closures](./07-closures.md)** — F.9 collapse / absorb /
  bubble; structural-invariant auditing.
- **[Cross-process](./08-cross-process.md)** — opening
  multiple lotuses; the wire format.
- **[Generics](./09-generics.md)** — monomorphization, the
  `Numeric` bound, `Result<T,E>` and `Option<T>`.
- **[Perspectives](./10-perspectives.md)** — projection
  classes; F.2.
- **[Recovery and supervision](./11-recovery-and-supervision.md)**
  — `on_failure` deeply; the four recovery primitives.
- **[trellis-pair](./12-trellis-pair.md)** — the capstone: a
  multi-binary production-shaped Aperio program built from every
  substrate primitive introduced.

By the end you will have written a multi-binary cross-process
Aperio program with audited closures, typed bus dispatch, generic
types, and structured failure cascades — assembled from
first-principle primitives, with no surprise machinery
underneath.
