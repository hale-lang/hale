# Modeling — how to think in Aperio

> **α** — Given the primitives, how do you actually *use* them
> to model a real system?

This is the synthesis chapter. It takes everything from the
preceding Concepts and shows how to apply it.

Covers:

- **The one-tower rule**: every named quantity in your model
  must be assignable to exactly one locus in one locus tower.
  Floating quantities — state that "lives between" loci — are a
  signal of modeling error, not a framework gap. Where to put
  the quantity instead. (A forthcoming
  [`pond`](https://github.com/aperio-lang/pond) library,
  *memory-owner-architecture*, develops the one-tower rule into
  concrete patterns + helpers for declaring ownership and
  verifying the assignment; this chapter will link to it when
  it ships.)
- **The six idiomatic patterns**: app locus, namespace lotus,
  service locus, spawned child, shape type, free fn. Every
  well-shaped Aperio program is composed of these.
- **Anti-patterns**: the shapes that look natural from other
  languages but fight the substrate. Tagged-locus dispatch,
  "util" namespaces of unrelated helpers, methods on `type`
  records, fluent-builder chains that mutate self.
- **The friction-log discipline**: when the language doesn't
  cleanly express what you want, the productive move is to log
  it as friction — not paper over it. The pattern catalog grows
  from real friction.
- A worked example walking through modeling decisions for a
  real (but small) system, choosing among the patterns and
  forms, surfacing friction where it appears.

*This chapter is under construction. The
[`spec/styleguide.md`](https://github.com/aperio-lang/aperio/blob/main/spec/styleguide.md)
is the canonical reference in the meantime, plus the
[`AGENTS.md`](https://github.com/aperio-lang/aperio/blob/main/AGENTS.md)
pattern catalog for a condensed version.*
