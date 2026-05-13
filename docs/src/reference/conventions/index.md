# Conventions

This section is the prescriptive read on how to write idiomatic
Aperio: the axiom the whole language flows from, the six-pattern
catalog every program decomposes into, naming, composition, the
rule that governs how the catalog grows, and the anti-patterns
that come from reaching for an old habit instead of the
substrate's primitive.

The content here is derived from `spec/styleguide.md`, the
normative source of truth. When the two diverge, `spec/styleguide.md`
is canonical — flag the drift.

## How to read this section

- **[Types vs loci](./axiom.md)** — the source axiom: every
  named structural thing is a locus; types are loci-in-waiting
  on the locus gradient. Read first; everything else assumes
  it.
- **[Design philosophy](./design-philosophy.md)** — the full
  "everything is a locus" framing, the three axes (capacity /
  projection / form), the form-annotation surface, and the
  locked-in v1 form behavior. Read after the axiom for the
  why behind the patterns.
- **[Pattern catalog](./patterns.md)** — the six shapes every
  piece of Aperio code falls into. Read after the philosophy;
  everything after refines a pattern.
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
