# Conventions

This section is the prescriptive read on how to write idiomatic
Aperio: the axiom the whole language flows from, the six-pattern
catalog every program decomposes into, naming, composition, the
rule that governs how the catalog grows, and the anti-patterns
that come from reaching for an old habit instead of the
substrate's primitive.

The content here is derived from
`notes/agent-onboarding/aperio-styleguide.md`, the
agent-loaded source of truth. When the two diverge, the notes
file is canonical — flag the drift.

## How to read this section

- **[Types vs loci](./axiom.md)** — the source axiom: types
  are for shapes, loci are for flow. Read first; everything
  else assumes it.
- **[Pattern catalog](./patterns.md)** — the six shapes every
  piece of Aperio code falls into. Read second; everything
  after it refines a pattern.
- **[Naming](./naming.md)**, **[Composition](./composition.md)**,
  **[Rolling the design](./rolling.md)** — surface rules that
  apply across the catalog.
- **[Anti-patterns](./anti-patterns.md)** — the failure modes
  that show up when one of the above is being avoided.
- **[Seeds and exports](./seeds.md)** — what a seed is and
  what crosses its boundary.
- **[v0 friction](./v0-friction.md)** — current language gaps
  with documented workarounds; shrinks as the language fills
  in.
