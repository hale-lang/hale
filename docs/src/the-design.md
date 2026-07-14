# The design

> Why one shape held across all four tiers.

This guide descended four levels — a small scripting language, a
high-level application language, a concurrent-services language, a
systems language. At each level you reached for the same
primitive, the **locus**, and saw more of it. That wasn't a
teaching trick layered on top of the language. It's the language's
actual structure, and it's worth seeing whole now that you've felt
it.

## It was towering loci all along

Hale is built bottom-up from one idea: a locus is a **system** —
a thing that decomposes into sub-systems and serves a role in
some super-system. Everything structural is a locus. A `type` is
a locus that hasn't grown flow yet; an app is a locus; a service,
a connection, a collection, a parser — loci, all the way down.

The tiers of this guide are the *same* tower observed at
different depths:

- *The basics* met a locus as the shell around `main`.
- *Everyday programs* saw it as an object with state and methods.
- *Concurrent services* saw it as a lifecycle, a bus participant,
  a supervised parent.
- *Systems control* saw it as a memory region with a layout and
  an execution strategy.

None of those views contradict; each is a higher-resolution
perspective on the thing below. That's why the function you wrote
in chapter one still works in the last chapter — you were
descending into one structure, not switching languages.

## The commitments that make it hold

A locus carries a small set of structural commitments, and every
guarantee in the language falls out of them:

- **Bounded attachment.** A locus bounds how many things attach
  to it. (The capacity model you met in the systems tier.)
- **Vertical-only flow.** A locus talks up to its parent and down
  to its children — never sideways. Siblings coordinate through a
  shared parent or the bus.
- **Failure flows up.** A broken invariant routes to the parent's
  policy, recursively, to the root.
- **The root is the horizon.** Recursion stops at the current
  observable boundary — the program's root, a process edge, a
  substrate.

From vertical-only flow you get memory safety with no GC and no
borrow checker: no pointer crosses sideways, so a region frees
wholesale at dissolve. From failure-flows-up you get supervised,
let-it-crash recovery with typed policy. From bounded attachment
you get the cost model the runtime can plan against. The
constraints aren't restrictions bolted on — they're the source of
the guarantees.

## Going deeper

- **[`AGENTS.md`](https://github.com/hale-lang/hale/blob/main/AGENTS.md)** — the formal model in one
  page: nodes, hyperedges, and invariants, with the `locus ↔ Σ`
  mapping. Written for agents authoring `.hl`, but it's the
  tightest statement of the design for a human too.
- **[`spec/design-rationale.md`](https://github.com/hale-lang/hale/blob/main/spec/design-rationale.md)**
  — every numbered design decision (`F.1` … `F.36`), the
  alternatives considered, and why each commitment is shaped the
  way it is.

You now have the whole arc: a small language at the top, a
systems substrate at the bottom, one shape connecting them. Build
something — and if the decomposition into loci feels natural,
that fit is the point.
