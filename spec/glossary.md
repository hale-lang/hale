# Codename legend

The spec and code comments carry a few families of short internal tags
from Hale's development history. Most are **navigation breadcrumbs, not
part of the language** — you can ignore them and read the surrounding
prose as current behavior. This page says which is which.

## `F.<N>` — design-decision tags

`F.1`, `F.22`, `F.32-1γ-v2`, … Each names one **locked design
commitment**: what a construct commits the design to, what was
considered and rejected, and whether it's shipped, superseded, or still
a sketch. The full records live in the decision log,
[`decisions.md`](./decisions.md). A reference like "§ F.31" is a link
into that log — not something you need to know to *use* the language.

## `m<N>` — milestone tags

`m26`, `m82`, `m105`, … Internal build milestones from the development
history (roughly: a unit of implementation work). They appear in code
comments and occasional spec asides as provenance — "shipped in m27,"
"the m105 adapter-inbound path." They are **not language surface**: the
behavior each describes is specified in the relevant spec file, and the
ship order is in [`../CHANGELOG.md`](../CHANGELOG.md). Safe to skip. (A
few carry an inline gloss where it aids reading — e.g. m26/m27
cooperative-then-pinned schedulers, m82 "locus all the way down,"
m96 the tree-sitter grammar.)

## `WS<N>`, `v1.x-*` — workstream tags

`WS1`, `WS3`, `v1.x-FORM-4`, `v1.x-WINDOWED`, … Names for larger
threads of work. Same status as milestones: history/provenance, not
language surface. Ignorable.

## Not codenames — live language identifiers

Some short symbols look like codes but are **real, current parts of the
language**, defined normatively in the spec — keep them in mind, don't
skip them:

- **`k_max`** and the accept-budget formula
  `k_max = B / [(1 − φ)·c + φ·σ]` — the bound on how many children a
  locus may `accept`. Defined in [`types.md`](./types.md); the ASCII
  source spellings `B`, `c`, `sigma`, `phi` are reserved identifiers
  (see [`tokens.md`](./tokens.md)).
- **`bulk` / `harmonic` / `resolution`** — the three execution modes
  (real keywords, not codenames).

> Rule of thumb: an `F.`/`m`/`WS`/`v1.x-` tag is history you can
> ignore; everything else in the spec is the current contract.
