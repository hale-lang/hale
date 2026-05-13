# Aperio design note — types vs loci, recursive lotus

> Captured 2026-05-10. This articulates a long-implied axiom and
> extends it into a directive for both Aperio source design and
> the codebase-onboarder's cross-tower model.

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

## The codebase-onboarder implication

> **The core emergent structure of any codebase is a lotus / locus
> tower.**

This is the foundational thesis of the codebase-onboarder:
foreign code already exhibits the lotus shape; the tool just
makes it visible. The three towers (import-graph, operational,
domain) are projections of one underlying locus tower; they
agree on what a locus is, just describe it from different
epistemic angles.

### Cross-tower agreement = locus identity

> **Nodes that synchronize coordination across towers should be
> directly represented as loci in the absorbed source.**

When the three extractors point at the same node — i.e., a
single source entity surfaces in:

- **Operational tower** as a `func main()` / handler / spawn-target
  / long-loop owner, AND
- **Import-graph tower** as the source of certain dependencies
  (it imports `net/http`, `log`, etc.), AND
- **Domain tower** as a named-thing with a motion-form
  (`Manager` → managing, `Service` → serving)

…then that node IS a locus in the codebase. The agreement
across three orthogonal projections is precisely the *signal*
of locus-ness. A node that appears in only one tower (say, a
file with imports but no functions or types) is structural
glue, not a locus. A node that appears in all three with
coherent roles is the kind of thing the codebase wants you to
treat as a unit.

This is the rule for the **cross-tower join layer** (m103a):
do not invent loci. Do not synthesize "this looks important so
it's a locus." Cross-tower coincidence does the inventing for
you. The join layer's job is to detect agreement across towers
and emit the corresponding locus declarations in the absorbed
Aperio source.

### Internal behavior is its own tower

When a locus is identified in the absorbed source, its internal
behavior — methods, lifecycle, child loci — is itself
extracted as a lotus tower one layer down. A `Service` struct
in Go isn't just one Aperio locus; it's one Aperio locus whose
*body* is a tower of methods that themselves project onto the
three modes:

- Resolution (what calls what; long-running loops; spawns),
- Harmonic (which fields it reads, what it imports
  internally),
- Bulk (what concepts the methods name).

The recursion bottoms out at primitive operations. The depth is
not a problem to plan around; it is a property of well-
structured code. A codebase whose loci nest 8 levels deep is
not "over-engineered" — it is honest about its own
emergent structure.

## What this changes in the spec corpus

This note crystallizes axiom-level guidance that's been
implicit. Concrete edits queued (not yet applied):

- **`spec/design-rationale.md`** — add a "Types vs Loci" section
  citing this note. The split has been operationally clear in
  codegen (`type` records have no lifecycle, `locus` does) but
  hasn't been stated as a design axiom.
- **`docs/src/grimoire/06-the-same-shape.md`** — extend the
  "every axis is the same shape" section with the recursive-
  inward point. Currently the chapter lists axes (memory,
  lifecycle, contracts, schedulers, transports, modes,
  perspectives, closure tests) but doesn't surface the
  *recursion* — that the lotus shape continues *inside* every
  locus's body.
- **`notes/codebase-onboarding-design.md`** — extend the
  "Three lotus perspectives" section with the
  cross-tower-agreement rule. Currently the design names
  per-tower extraction rules but doesn't articulate the
  cross-tower join semantics.

## What this changes in the codebase-onboarder roadmap

The cross-tower join layer (m103a) was previously sketched as
"link goroutine call-sites to fn decls; HTTP handlers to
HandleFunc wiring; motion-forms to operational loci." This note
makes its *purpose* explicit: **emit a locus declaration for
every node where the three towers agree.**

Practical join algorithm:

1. For each file, collect all (file, node-name, role) tuples
   from each tower.
2. Group tuples by `(file, node-name)`.
3. A node-name with **≥ 2 tower roles** is a candidate locus.
4. A node-name with **exactly 1 tower role** is structural
   data — emit as a `type`, a free fn, or a leaf comment.
5. Naming: prefer the operational tower's name (entrypoint /
   handler / spawn-target). Suffix with `L` per the apps-are-
   loci convention.
6. Lifecycle wiring: operational tower says which method is
   `main`/`birth`/`run`/`dissolve`. The motion-form from the
   domain tower goes in a `// motion: <form>` comment until
   Aperio gets a first-class metadata slot.

Intuition: ≥ 2 tower agreement is sufficient for "this thing has
flow." Three-tower agreement is sufficient for "and the
codebase explicitly names it." One-tower presence is
"structural artifact, not its own thing."

## What this does NOT change

- The current per-tower extraction logic — file-level for v0
  is fine; the join layer rolls things up.
- The data layer — three independent JSONs is the right
  intermediate shape. The join layer reads them; doesn't
  rewrite them.
- Aperio's two-primitive split (types and loci). This note
  doesn't add a third primitive; it just sharpens the
  contract for the existing two. The same principle in
  miniature governs new entries to the pattern catalog — see
  "Rolling the design" in `spec/styleguide.md`. New primitives
  earn their slot by sharpening the existing contract, not by
  inventing a third category.

## See also

- `moa/MOA.md` — Memory-Owner Architecture; the composition
  discipline that builds on this axiom. The recursive lotus
  this note names becomes a practical authoring rule when an
  app has state to organize: state lives at one memory-owner
  per concern, with capacity + ingest disciplines declared per
  memory-owner, and the bus carrying typed deltas between
  concerns. The architecture ships as a top-level substrate
  with its own `moa::*` path prefix.
