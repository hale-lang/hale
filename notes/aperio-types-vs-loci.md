# Aperio design note — types vs loci, recursive lotus

> Captured 2026-05-10. Articulates a long-implied axiom that
> directs Aperio source design.

## The axiom

> **Types are for shapes. Loci are for flow.**

That is:

- A `type` declaration is a static record — fields, names,
  layout. Pure shape, no lifecycle. Returnable by value, equal
  by value, no projection modes, no contracts, no birth/run/
  dissolve.
- A `locus` declaration is dynamic flow — params (configurable
  initial state), lifecycle (birth → accept → run → drain →
  dissolve), contracts (expose / consume), bus participation
  (publish / subscribe), projection (resolution / harmonic /
  bulk views over the same instance).

If a thing has lifecycle, it is a locus. If it is pure data, it
is a type. There is no third category at v0; the split is
clean.

## The recursive principle

> **Loci are the fundamental building block at every layer of
> the codebase.**

Concretely:

- An app is a locus.
- A library namespace is a locus (empty params, only methods —
  the namespace-lotus pattern, validated in
  `MorphemeRewriterL`).
- A long-running service is a locus.
- An async task / goroutine / spawned thread is a locus.
- A bus subscriber is a locus.
- An HTTP route handler is a locus (subscriber on a
  path-derived subject).
- A configuration / connection / cache / pool / pipeline / queue
  is a locus.
- **And inside any of those: their behavior is itself a locus
  tower.** A cache's lookup flow, a pipeline's stages, a
  service's request handling — each is a recursive lotus.

The stopping rule for the recursion is the same as the start:
*if there is emergent structure with lifecycle and coordination,
that's a locus*. When you've decomposed down to leaf operations
(arithmetic, single field reads, primitive calls), you've hit
the floor. Everything above the floor is loci nested in loci.

## What this does NOT change

- Aperio's two-primitive split (types and loci). This note
  doesn't add a third primitive; it just sharpens the
  contract for the existing two. The same principle in
  miniature governs new entries to the pattern catalog — see
  "Rolling the design" in `spec/styleguide.md`. New primitives
  earn their slot by sharpening the existing contract, not by
  inventing a third category.
