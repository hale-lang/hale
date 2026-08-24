# The canonical semantic model

The typed intermediate representation every structural consumer
reads. GH #476 built it; this document is its contract.

`spec/verification.md` specifies what the compiler *checks* and what
the topology artifact *serialises*. This one specifies the value
sitting between them: what a model contains, what it is allowed to
not know, the laws it must satisfy, and the rules a new consumer
has to obey.

The crate is `hale-model` — 106 public types, no dependency on the
AST, the checker, or codegen.

## The architectural law

```text
Source --check/derive--> Model --judge(ClaimIr)--> Evidence
                           |
                           +--project--> Artifact
                           +--derive---> DispatchPlan
```

**Derive a modeled semantic fact once.** Downstream consumers
project or query it; they do not walk the AST, the artifact JSON,
or codegen state to rediscover it.

Eight concepts stay distinct and un-conflated: source, plan,
**model**, `ClaimIr`, evidence, artifact, lowering plan, and
execution evidence. Each answers a different question, and keeping
them separate is what lets each one be simple — and what keeps a
second authority for one question from existing at all.

## What a model is

A model is **known facts plus an explicit account of what it does
not know**, closed at a horizon:

- typed **entity tables** — not one homogeneous node kind;
- typed **relation tables** — `calls`, `owns` and `publishes` are
  different rows with different fields and different witness
  renderings, never interchangeable string edges;
- typed **holes** — unresolved residue as data, each naming the
  relation families it hides;
- positive **capabilities** — what is exact, *stated*, so a
  consumer can ask "is this model adequate for my question?"
  without reverse-engineering the absence of strings;
- **provenance on every row** — source-neutral: a `SourceId` and a
  byte span, or a named synthetic origin.

The last two are the load-bearing ones. A graph that records only
what it found cannot distinguish "there is no such edge" from "I
could not see". Every judgment in the language rests on that
distinction — see [Unknown is not absent](#unknown-is-not-absent).

## Obtaining a model

There is exactly **one** constructor:

```rust
hale_types::model_builder::derive_application_model(&Bundle) -> ApplicationModel
```

It runs over a *checked* bundle. A model of an ill-typed program
describes nothing, so consumers that judge gate on the bundle
having no non-`Claim` errors first.

**There is no other way in.** In particular:

- **No artifact → model.** The topology artifact is a *projection*,
  not a serialisation. It carries the model half (hashed by
  `shape_hash`) plus typed law, capability and adequacy sections,
  but several tables — `costs`, `locus_instances`, the analyses,
  the provenance interner — have no wire form. `hale topology
  graph` and the fleet tier *admit* an artifact by validating its
  JSON against the same rules; they do not reconstruct an
  `ApplicationModel`.
- **No plan → model.** Fleet composition admits deployment plans
  and reads artifacts; it does not elaborate a plan into an
  application model. What it builds instead is instructive:
  `ComponentModel`, a separate string-level structure reconstructed
  from artifact JSON — vertex names, `(from, to)` call pairs,
  name-level publish and subscribe relations, and the component's
  own residue. It is deliberately weaker than an `ApplicationModel`
  because that is all the wire form carries, and it is the concrete
  price of the boundary below.
- **No hand-authored model format.** Tests that need a shape the
  builder cannot yet infer derive a real model and then edit its
  tables. That is deliberate: it keeps the builder the only thing
  that decides what a model of a program is.

The consequence is worth stating plainly, because it bounds every
out-of-process consumer: **anything outside the compiler process
reads the artifact, not the model**, and the artifact is lossy by
construction. Giving the model a reversible wire form is a real
piece of future work, not a small one — it would need every table
to serialise, the provenance table to survive, and an admission
path that cannot be tricked into producing a model no program
denotes.

## Sorts — the entity tables

`Entities` holds fifteen tables. Each row has an id (a newtype
index into its own table), a raw canonical `name`, an author-facing
`display`, and provenance.

| table | what a row is |
|---|---|
| `functions` | free fn, method, lifecycle hook, mode, failure handler |
| `loci` | a locus *declaration* |
| `locus_instances` | a statically exact instance in the main arrangement (`App.w`, `App.workers[3]`) |
| `topics` | a declared topic |
| `subjects` | a wire subject or pattern — **address identity** |
| `payloads` | a payload contract — deliberately a different sort from the subject |
| `phases` | lifecycle phases |
| `seeds` | imported seeds |
| `thread_domains` | pools and their placement |
| `bindings` | transport bindings, with a `role` |
| `groups` | resolved claim groups |
| `types`, `interfaces` | type and interface declarations |
| `effect_classes` | built-in and user-declared effect classes |
| `declarations` | the declaration universe, for coverage laws |

Two distinctions in that table are load-bearing:

- **`subjects` vs `topics`.** A subject is the delivery identity. A
  literal `"t" <- v` send carries `declared_topic: None` even when
  its text matches a declared topic's wire subject, because after
  lowering the runtime cannot tell the two spellings apart. Any
  query that decides delivery joins on `SubjectId`; `declared_topic`
  is a syntactic link to a declaration and decides nothing.
- **`loci` vs `locus_instances`.** A declaration says what may
  exist; an instance is a statically exact birth in the
  arrangement. `LocusDecl::params` records what a locus may hold
  even where no instance is born; `LocusInstance::replica` is the
  0-based index the runtime pins, not a count.

## Relations

`Relations` holds seventeen tables. The invariant that matters is
**grain**.

Call and publish rows are **site-grained**: two calls to one callee
are two rows, because the callee executes twice. Alternatives of
one interface dispatch share an authored `site` ordinal, because
one dispatch runs one conformer. A consumer that collapses either
into a set is computing reachability where the language means
executions.

| table | grain / notes |
|---|---|
| `member_of`, `phase_of`, `declared_in`, `realizes` | structure |
| `owns` | instance parent → child |
| `calls` | **site**; carries `dispatch`, `in_loop`, `unbounded` |
| `dead_interface_calls` | uninhabited dispatch |
| `publishes` | **site**; carries `key_domain`, `in_loop`, `disposition` |
| `declares_publish` | the endpoint grain (`bus { publish T; }`) |
| `subscribes` | carries `key_predicate` — the filter half of delivery |
| `placed_in`, `affined_to` | placement |
| `binds` | topic ↔ transport, with `role` |
| `supervises` | supervision edges |
| `group_members`, `group_selectors` | resolved vs authored |
| `costs` | **site**; per-call `alloc` / `block` / `frame_bytes` |

`costs` is site-grained for the same reason as publishes: a
per-call budget is a statement about *one invocation*, so whether
a site sits inside a loop is the difference between a finite count
and an unbounded one.

## Holes

A hole is unresolved residue, as data:

```rust
Hole { at: EntityRef, kind: HoleKind, hides: RelationSet,
       authored_site: Option<u32>, reason: String, provenance }
```

Ten kinds: `IndirectCall`, `UntypedReceiver`, `ComputedSubject`,
`UnknownKeyDomain`, `OpenInterface`, `RuntimeInheritedPlacement`,
`DynamicEndpoint`, `ExternalOpaque`, `UnsupportedArtifactSemantics`,
`UnanalyzedBody`.

`hides` names the relation families the hole withdraws.
`allowed_hole_families` is a **shape law**, validated: each
`(entity, kind)` pair has a required minimum and a permitted
maximum mask. An unfollowable call hides `CALLS ∪ EFFECTS ∪ COSTS`
— all three are required, because a call whose target the caller
chooses is exactly where an effect walk *and* a quantitative bound
must refuse.

### Holes are reachability-scoped

**A hole withdraws a proof only where it is relevant to the
question being asked.** A hole on an unrelated topic says nothing
about a purely local claim, and a global "is there any hole?" scan
turns one dynamically-born locus into an unbounded answer for the
whole program.

Relevance has two grains, and consumers need both:

- **Endpoint-scoped** — is what lies beyond *this* endpoint, in
  *this* direction, fully modeled? Answered once, in
  `model_query::endpoint_incomplete`, which reads both the holes
  and the typed routes that leave the application. A `connect`
  binding is a downstream boundary; a `listen` binding is an
  upstream one; neither emits a hole, so a consumer reading only
  `holes` fails open on both.
- **Reachability-scoped** — residue in a function nothing can
  execute withdraws nothing. An unfollowable call in dead code
  cannot invoke anything.

### Unknown is not absent

The rule the language runs on, stated once:

> A concrete path beats a hole. A hole beats a false proof of
> absence.

Its most common violation is subtle. Where a query returns three
states — *exactly none*, *exactly n*, *not knowable* — collapsing
the last two reads as "no such thing" and certifies. Population,
effect sets, subscriber counts and key domains all have this
shape, and the collapse reads as a proof of absence.

## Capabilities and adequacy

`Capabilities` is the **positive** completeness account: eleven
independent booleans (`exact_calls`, `exact_publishes`,
`exact_subscribes`, `exact_key_filters`, `exact_ownership`,
`exact_placement`, `exact_routes`, `exact_effects`,
`exact_cardinality`, `exact_delivery_guarantees`, `exact_costs`),
each vouching a `RelationSet`.

The account is positive on purpose. Absence of recorded unknowns is
not proof of completeness, so an unvouched family reads `degraded`
whether or not a hole happens to exist. `validate` cross-checks the
two: a capability claiming exactness over a family some hole hides
is a `CapabilityContradiction`, and **the canonical
`unresolved_relation_mask` is the authority** for what a hole or an
absorption residue hides. A producer's private bookkeeping agreeing
with itself is not enough; every consumer is entitled to rely on
`validate()`.

**Adequacy** is derived, per judgment family: `exact` when
capabilities vouch every relation family that family's projection
consumes, else `degraded`. Each family declares that set in
`JudgmentFamily::required_relations`, and the declaration must name
what the queries **actually read** — extending an engine's reads
extends the declaration in the same change.

## Provenance

Every row carries a `ProvenanceId` into a table of either a
`Source { source, span }` or a named `Synthetic { origin }`. Two
rules:

- The model never sees the AST. Spans are bytes and `SourceId`s.
- Foreign offset spaces stay foreign. A diagnostic whose span lives
  in stdlib parse space or another seed carries a discriminator, so
  numeric overlap with a bundle file cannot misfile stdlib evidence
  as application code.

## Analyses

`Analyses` carries products the model does not derive itself but
that no other table holds:

- **`dispatch_gates`** — the bus graph's per-subject
  devirtualization gates.
- **`stdlib_absorption`** — what a merged-summary walk sees *inside*
  stdlib bodies reachable from a user fn.

Absorption is kept as a **graph**, not a flattened summary, and its
grain matters twice over. One `StdlibAbsorption` row is one
authored entry site (with `entry_group` marking alternatives of one
dispatch); its `nodes` form an interior graph whose call events
carry their own dispatch groups and can target either another
interior node or a user re-emergence. An interior can publish —
`std::log::Logger.info` computes `log.<path>` and sends from inside
its own body — so a consumer reading only `relations.publishes`
misses real publishers.

The contracted `ViaStdlib` rows in `relations.calls` are the
*endpoint pair* for the same paths. They collapse several entry
sites into one, so a consumer counting executions reads the
absorption account and skips them; counting both double-counts.

## The laws

`ApplicationModel::validate()` enforces eighteen error kinds. They
group into four families:

**Structural.** `NotCanonical` (every table sorted and deduplicated
by its canonical key — the model's identity depends on it),
`DanglingId`, `DanglingProvenance`, `InvalidProvenanceRecord`.

**Residue.** `EmptyHole` (a hole hiding nothing is not residue),
`CapabilityContradiction`, `UnrepresentedUnknown` (an unknown the
tables imply but no hole records).

**Semantic.** `InvalidBound`, `RealizesDisagrees`,
`RealizesIncomplete`, `ReplicaIndicesNotContiguous` (the replicas of
one field form a contiguous 0-based set — codegen bakes those
indices and keyed delivery reads them), `BindingSubjectDisagrees`,
`IllegalFallback`, `FallbackUncovered`, `KeyContract`,
`DeclaredTopicDisagrees`, `EndpointPayloadDisagrees`.

**Coverage.** `CoverageLaw` — the ownership partition and the
analysis-coverage bits, in one shared validator that both the model
and the evidence sidecar call. A model whose ownership account is
corrupted cannot certify anything, digests notwithstanding.

## Derived products

Three values are derived *from* a model and are not part of it:

- **`ClaimIrTable`** — the lowered law: one typed row per clause,
  with its ordinal, origin, judgment family, typed operands
  (each reference carrying raw identity, author spelling and
  resolution state), and the `GroupSelection` status carried from
  selection rather than re-inferred.
- **`EvidenceTable`** — the certificate sidecar, deliberately
  *outside* the model, because a model must not carry a cached
  prior judgment of itself. It ties to the model by
  `model_shape`, `law_digest`, `inputs_digest` and
  `coverage_digest`; a judgment refuses evidence whose ties
  disagree rather than replaying it.
- **`DispatchPlan`** — the typed lowering plan (GH #476 Change 8),
  combining dispatch gates with the arrangement. Which lowering
  flavour a call gets is a plan *conclusion*, not a model row.

## Identity and versioning

Six identities, each answering a different question. Conflating any
two of them is a trap.

| identity | covers | moves when |
|---|---|---|
| `shape_hash` | the model half only | the topology changes; **not** when law or comments do |
| `artifact_digest` | the whole serialised document | any byte changes |
| `law_digest` | the lowered law table | a law row changes |
| `inputs_digest` | analysis inputs *outside* the model | stdlib source, path renames, or `ANALYSIS_SEMANTICS_VERSION` change |
| `coverage_digest` | the `analyzed` / `analyzable` bits | coverage changes |
| `model_hash` | model identity for the obs stream | the model does |
| `TOPOLOGY_SCHEMA` | the artifact's decoding contract | a field becomes required, a section changes meaning, or a family moves |
| `ANALYSIS_SEMANTICS_VERSION` | the evidence producer's *semantics* | its **results** move |

The last one has a rule that is easy to get wrong.
`EvidenceTable::validate` compares digests; it does not hash the
implementation. So a sidecar produced by an older toolchain can
share the source, the model shape, the law digest and the coverage
digest while carrying a verdict the current compiler disagrees
with. **Whenever a producer change alters results — including
correcting a bug — the version moves in the same change.** It went
3 → 6 across GH #476 Change 5h alone.

Similarly for the schema: making a field *required* is a
decoding-contract change, not an additive one. An artifact written
before the change passes the schema gate and is then refused for
omitting something its schema never demanded.

## Adding a judgment family

The discipline below is contract, not style. Each rule states a
property a consumer of this model is entitled to rely on.

1. **Ask each question in one place.** The shared queries in
   `model_query` exist because the same rule was settled in one
   engine and left stale in its sibling four times: delivery joined
   on wire identity here and on the syntactic topic link there;
   uncertainty was endpoint-scoped in one check and global in
   another.
2. **Declare `required_relations` as what you read**, then ship the
   `adequacy` entry and the schema transition together. A family a
   row can declare, over which the document cannot state exactness,
   is an internally contradictory artifact.
3. **Never be silent.** The check path appends diagnostics, not
   verdicts. A non-`holds` row with no diagnostic compiles clean
   while the artifact records `law_failed` — and leaves the row
   with no evidence for admission to find. Every non-holds verdict
   carries its reason, including the "this subject is outside the
   analyzable universe" case.
4. **Scope residue to the question.** Relevant *reachable* residue
   withdraws a proof; residue anywhere in the document authorises
   nothing.
5. **Preserve the three states.** *None*, *n*, *not knowable* are
   three answers. Collapsing the last two into the first is a
   fail-open.
6. **Respect grain.** Sites are executions. Sets are reachability.
   A quantitative law wants the former.
7. **Aggregate last.** Cost a whole scenario — one key, one
   conformer — end to end *before* taking any maximum.
   `max(a) + union(b)` is not `max(a + b)`, and `Σ max` is not
   `max Σ`.
8. **Join on identity, never spelling.** `SubjectId`, not
   `declared_topic`; `Function::name` (raw canonical), not
   `Function::display` (demangled author spelling).
9. **Move the semantics version when results move.**

## Known boundaries

Stated so nobody mistakes them for oversights:

- The model is **in-process only** — see
  [Obtaining a model](#obtaining-a-model). No wire form, no
  reconstruction from an artifact.
- `CostDimension::Block` and `FrameBytes` are recorded, but the
  quantitative engines still *measure* over their own summary; the
  model carries the facts and the judgment reads the engines'
  evidence.
- Frame sizes are estimated from declared shapes, not measured from
  codegen. The bound is a structural over-approximation and the
  diagnostic says so.
- Absorption interiors are explored to a frontier. Beyond it the
  account is `Truncated`, which withdraws rather than assumes.
