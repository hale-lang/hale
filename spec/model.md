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
checked Bundle                      -->  ApplicationModel
Bundle + Model                      -->  ClaimIrTable
Bundle + Model + ClaimIrTable       -->  EvidenceTable
Model + ClaimIrTable [+ Evidence]   -->  Judged verdicts
ApplicationModel                    -->  model-half projection (artifact)
ApplicationModel                    -->  DispatchPlan
admitted Artifact                   -->  ComponentModel --> fleet ModelGraph
```

**Derive a modeled semantic fact once.** Downstream consumers
project or query it; they do not walk the AST, the artifact JSON,
or codegen state to rediscover it.

Note what that does *not* say. `ClaimIrTable` is not a projection
of the model alone — `lower_claims` takes the bundle too, because
clause enumeration, constitution adoption and annotation surfaces
are read from source. `EvidenceTable` is likewise derived from all
three, and for the certificate and budget families it is an INPUT
to judgment rather than its result: those engines measure, and the
judgment decides over what they measured. And the fleet tier shares
selected ALGORITHMS and admitted contracts, not the value: it
decodes a `ComponentModel` from artifact JSON and composes its own
`ModelGraph`.

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
- **provenance on the fact-bearing rows** — entities, relations,
  holes, labels and weights; source-neutral, and scoped precisely
  below.

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

`Entities` holds fifteen tables. A row's **id is its index** in its
own table, wrapped in a newtype (`FunctionId`, `SubjectId`, …) —
not a field stored on the row.

Identity is **per table**, and deliberately not uniform. Many rows
carry a raw canonical `name` plus an author-facing `display`
(`functions`, `loci`, `topics`, `groups`); several do not.
`LocusInstance` is identified by `path` (`App.workers[3]`) with a
`decl` and a `replica` index; `Subject` by its wire `pattern`;
`PayloadContract` by its shape and hash; `Binding` by its typed
subject, transport and role; `Phase`, `Seed` and `ThreadDomain`
carry a name with no separate display. Consumers should read the
table's own definition rather than assume a common shape.

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
| `effect_classes` | declared **or merely referenced** user effect classes — `declared: false` is "referenced, never declared". Built-ins are a separate fixed vocabulary (`BUILTIN_EFFECT_CLASSES`) and have no row |
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

Call and publish rows are **site-grained**: they carry an authored
`site` ordinal, two calls to one callee are two rows because the
callee executes twice, and alternatives of one interface dispatch
share that ordinal because one dispatch runs one conformer. A
consumer that collapses either into a set is computing reachability
where the language means executions.

`costs` is **mixed-grain** and carries no `site` ordinal at all;
its rows are distinguished by `(function, dimension, provenance)`.
See below.

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
| `costs` | **mixed**; per-call `alloc` / `block` / `frame_bytes` |

`costs` needs care, because its two grains have different
semirings:

- **`Alloc` and `Block` are OCCURRENCE rows.** One row per
  allocation site or blocking call, distinguished by provenance,
  carrying `in_loop`. A per-call budget is a statement about *one
  invocation*, so whether an occurrence sits inside a loop is the
  difference between a finite count and an unbounded one.
- **`FrameBytes` is a FUNCTION-level row.** Exactly one per
  analysed function, `in_loop` always false, consumed by a
  longest-stack-path computation rather than summed over
  executions. A frame is reused across iterations; multiplying it
  as if it were an occurrence is a category error.

Neither carries an authored site ordinal — `CostSite` is
`(function, dimension, amount, in_loop, provenance)`.

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

- **Endpoint-scoped** — is the possible-delivery STRUCTURE beyond
  *this* endpoint, in *this* direction, fully modeled?
  `model_query::endpoint_incomplete` answers exactly that and no
  more: downstream it reads `SUBSCRIBES | KEY_FILTERS | DELIVERY`
  holes, upstream `PUBLISHES | DELIVERY`, plus the typed routes
  that leave the application (a `connect` binding is a downstream
  boundary, a `listen` binding an upstream one; neither emits a
  hole, so a consumer reading only `holes` fails open on both).

  **It is not the whole completeness question**, and a consumer
  must not treat it as such. It deliberately says nothing about
  subscriber CARDINALITY or locus population, because those do not
  affect which effect classes a publish reaches — another instance
  of one locus runs the same handler. A judgment that counts
  *deliveries* rather than reachability needs its own account, and
  `@budget(fanout)` has one: subject/topic-grained `CARDINALITY`
  residue, and locus-grained `OWNS | CARDINALITY` residue through
  `population_of`, whose three outcomes (exactly zero, exactly n,
  not knowable) must stay distinct.

  The rule generalises: **scope every family in your
  `required_relations`, not just the ones this shared query
  covers.** `endpoint_incomplete` is one question asked once; it is
  not every question.
- **Reachability-scoped** — residue in a function nothing can
  execute withdraws nothing. An unfollowable call in dead code
  cannot invoke anything.

### Unknown is not absent

The rule the language runs on, stated once:

> A reachable relevant hole always defeats a proof of absence.

Note what that does *not* say. Whether a concrete counterexample
found *later* in a walk outranks a hole found earlier is a
**per-tier policy**, not a universal rule, and the shared
reachability engine exposes both:

- `HolePolicy::Halt` — stop at the first reachable hole. The
  application checker takes this: the repair is to make that edge
  resolvable, and the diagnostic names the edge.
- `HolePolicy::PathWins` — keep walking known edges, so a concrete
  counterexample outranks the refusal and the hole decides only if
  no path is found. The fleet checker takes this: a cross-binary
  path is worth more than "cannot tell".

Both are sound; they differ in what is more useful to report. A new
judgment picks one deliberately — inheriting the wrong one silently
changes what an application law means.

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

Entity, relation, hole, label and weight rows carry a
`ProvenanceId` into a table with **three** variants:

- `Source { source, span }` — a `SourceId` and a byte range
  **relative to that source unit's own content**, not to any
  bundle-global space. A consumer renders a global span by adding
  the unit's base;
- `Synthetic { origin }` — a named origin for a fact no source line
  produced;
- `ForeignSpan { span }` — a span in a FOREIGN offset space
  (stdlib parse space, another seed). It is the discriminator that
  keeps stdlib evidence from being resolved against application
  files, where numeric overlap would misfile it.

Two rules:

- The model never sees the AST. Spans are bytes and `SourceId`s.
- Foreign offset spaces stay foreign, by construction rather than
  by convention — hence the third variant.

The guarantee is scoped to those row families. Analysis products
are coarser: a `StdlibAbsorption` carries `entry_provenance` for
its authored entry site, and its interior nodes and events do not
each carry their own.

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
dispatch); its `nodes` are `AbsorbedNode`s forming an interior
graph, each carrying `AbsorbedEvent`s in walk order — a `Call`
(with its own dispatch group, targeting an `AbsorbedTarget`: another
interior node or a user re-emergence), a `Publish`, or one of the
three residues: `CallHole`, `PublishHole`, `Truncated`. An interior
can publish —
`std::log::Logger.info` computes `log.<path>` and sends from inside
its own body — so a consumer reading only `relations.publishes`
misses real publishers.

The contracted `ViaStdlib` rows in `relations.calls` are the
*endpoint pair* for the same paths. They collapse several entry
sites into one, so a consumer counting executions reads the
absorption account and skips them; counting both double-counts.

An interior publish to a computed subject is a **`PublishHole`,
and it carries the wildcard publish patterns its locus declared.**
The language admits a computed subject only under such a
declaration and the publish site enforces it, so those patterns
*bound* the hole: one declared `io.tcp.**` cannot explain a
publisher of `app.order`.

This is what keeps `exact_publishes` from being the only answer
available. That capability is one bit for the whole program, so
reading it alone means a single stdlib I/O call — `std::io::tcp`
logs to a runtime-chosen subject — withdraws the publisher account
for every topic, degrading every family. A subject-specific
question instead asks whether any residue can reach *that*
subject. Three kinds cannot be bounded and still answer yes
everywhere: an unfollowable interior call, a truncated frontier,
and an interior publish whose subject expression resolves to no
subject row (the absorption records the send's subject
*expression*, so a computed one appears as the variable's name —
comparing that as though it were a wire subject would certify
straight through the publish it stands for).

Overlap between two patterns is not the same question as whether a
subject matches a pattern: a subscription may itself be declared on
`log.**`. Two `**` patterns overlap when one root is a prefix of
the other on a segment boundary.

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

Three values sit beside a model without being part of it. Only one
is derived from the model ALONE — see the layering above:

- **`ClaimIrTable`** — the lowered law, from `(bundle, model)`:
  one typed row per clause,
  with its ordinal, origin, judgment family, typed operands
  (each reference carrying raw identity, author spelling and
  resolution state), and the `GroupSelection` status carried from
  selection rather than re-inferred.
- **`EvidenceTable`** — the certificate sidecar, from
  `(bundle, table, model)`. Deliberately *outside* the model,
  because a model must not carry a cached prior judgment of
  itself. For the certificate and budget families it is an INPUT
  to judgment rather than its output: those engines measure, and
  the judgment decides over what they measured.

  It is tied to what it answers by **five** independent checks, not
  four: `model_shape`, `law_digest`, `inputs_digest`,
  `coverage_digest`, and **exact equality of the source-unit list**
  — path plus content digest — across the evidence, the model and
  the law table. The fifth is not implied by the others: without
  it, locally valid offsets could be rendered against a different
  source snapshot. It ties to the model by
  `model_shape`, `law_digest`, `inputs_digest` and
  `coverage_digest`; a judgment refuses evidence whose ties
  disagree rather than replaying it.
- **`DispatchPlan`** — from the model alone (GH #476 Change 8),
  combining dispatch gates with the arrangement. It decides how a
  BUS SUBJECT dispatches — dynamic, static bucket, or static
  direct — not how calls in general lower. Which flavour a subject
  gets is a plan *conclusion*, never a model row.

## Identity and versioning

**Eight identities and two version constants.** They answer
different questions and cover different data; conflating any two is
a trap, and the coverage below is what the digest functions
actually hash — not what their names suggest.

| identity | covers | moves when |
|---|---|---|
| `shape_hash` | the model HALF of the artifact (`TopologyShapeV1`) — sorts, relations, weights, the through-stdlib contraction, endpoint identity. Claim RESULTS excluded | the topology changes; **not** when law, comments or provenance do |
| `artifact_digest` | the whole serialised document | any byte changes |
| `law_digest` | every law row (ordinal, name, origin, typed law, provenance id) **and the law provenance STORE** — its source units and every record | an operand changes, *or* a law's span moves, *or* the source snapshot does |
| `inputs_digest` | analysis inputs OUTSIDE the model: `ANALYSIS_SEMANTICS_VERSION`, the Hale stdlib source, **the compiler package version**, the import-rename table, and **the stdlib surface-classification registry** (namespaces, fn names, effect masks, open prefixes) | any of those drift — most do not rely on anyone remembering |
| `coverage_digest` | per locus: name + `analyzable`. Per function: name, `analyzed`, `summarized`, a **failure-handler discriminator** — not the full `FunctionKind`, so a move among `Free`/`Method`/`Hook`/`Mode` is not in that byte — and **canonical owner** | coverage changes, *or* a member moves between owners |
| `model_shape` (in `EvidenceTable`) | the `shape_hash` the sidecar was derived beside | the model half does |
| obs `model_hash` | **the emitted `shape_hash`** — `model_shape_hash` renders the artifact and reads that field out | the model half does. It is the runtime exposure of `TopologyShapeV1`, **not** a full-model identity |
| obs `entity_id_digest` | the exact stamped id table (kind, name, id) | the numbering does. It exists because `model_hash` does *not* cover every table the ids index — arrangement bindings are not in the artifact at all, and an unused topic's wire subject rides an unhashed section, so two builds could share a `model_hash` while numbering entities differently |
| `TOPOLOGY_SCHEMA` | the artifact's decoding contract | a field becomes required, a section changes meaning, or a family moves |
| `ANALYSIS_SEMANTICS_VERSION` | the evidence producer's SEMANTICS | its **results** move |

Two rules that are easy to get wrong.

`EvidenceTable::validate` compares digests; **it does not hash the
implementation.** A sidecar produced by an older toolchain can
share the source, the model shape, the law digest and the coverage
digest while carrying a verdict the current compiler disagrees
with. So whenever a producer change alters results — *including
correcting a bug* — `ANALYSIS_SEMANTICS_VERSION` moves in the same
change. It went 3 → 6 across GH #476 Change 5h alone.

And making an artifact field **required** is a decoding-contract
change, not an additive one: an artifact written before the change
passes the schema gate and is then refused for omitting something
its schema never demanded. That is a `TOPOLOGY_SCHEMA` transition.

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
8. **Join on the right identity for the question.** For DELIVERY,
   the identity is `SubjectId`: `declared_topic` is a syntactic
   link and a literal send carries `None` there. But
   `declared_topic` is not vestigial — it decides
   declaration-sensitive questions (typed endpoint identity,
   `require publishes` / `subscribes`), which is why the schema
   keeps both. Use the wire identity for what the runtime does,
   the declaration link for what the author wrote. Separately:
   `Function::name` is the raw canonical identity and
   `Function::display` the demangled author spelling; joins take
   the former.
9. **Move the semantics version when results move.**

## The type inventory

Everything above describes how the model *behaves*. This is what it
*contains*: every public type, so nothing in the crate is
reachable-but-undescribed. Field-level documentation is the rustdoc
— each public field carries a doc comment, and a test fails if one
lands without.

**`application`** — the model itself and its sidecars.
`ApplicationModel`, `ModelHeader`, `ModelHashKind`, `Entities`,
`Relations`, `Analyses`, `LabelRow`, `WeightRow`, `ModelError`.
Absorption: `StdlibAbsorption`, `AbsorbedNode`, `AbsorbedEvent`,
`AbsorbedTarget`, `AbsorbedHoleKind`. Evidence: `EvidenceTable`,
`EvidenceRow`, `CertificateEvidence`, `VerdictIr`. Plus
`DispatchGate`.

**`entity`** — the fifteen sorts. `Function`, `FunctionKind`,
`LocusDecl`, `LocusParam`, `LocusInstance`, `Topic`, `Subject`,
`PayloadContract`, `Phase`, `Seed`, `ThreadDomain`, `Binding`,
`BindingRole`, `TransportKind`, `Group`, `TypeDecl`,
`InterfaceDecl`, `EffectClassDecl`, `EffectClassDefinition`,
`Declaration`, `DeclKind`.

**`relation`** — the seventeen relation row types. Structure:
`MemberOf`, `PhaseOf`, `DeclaredIn`, `Realizes`, `Owns`. Behaviour:
`Call`, `DispatchKind`, `DeadInterfaceCall`, `Publish`,
`Subscribe`, `DeclaresPublish`. Placement: `PlacedIn`, `AffinedTo`,
`CoreSet`. Wiring: `TopicBinding`. Supervision: `Supervises`,
`SupervisedRef`, `SupervisionPolicy`. Groups: `GroupMember`,
`GroupSelector`, `SelectorForm`. Cost: `CostSite`,
`CostDimension`.

**`keys`** — delivery's typed vocabulary. `KeyDomain`, `KeyValue`,
`KeyPredicate`, `TopicKey`, `KeyOnUnmatched`, `PublishDisposition`,
`Capacity`, `ShedPolicy`, `TopicBound`, `TopicOnFull`,
`BindingLossBehavior`.

**`claim_ir`** — lowered law. `ClaimIrTable`, `ClaimRow`,
`ClaimIr`, `ClaimOrigin`, `ClaimIrError`, `LoweringIssue`.
Operands: `NameRef`, `GroupRef`, `TopicIrRef`, `PhaseIrRef`,
`SeedIrRef`, `EffectClassRef`, `BusSelector`, `GrantIr`, `SetIr`,
`CountCmpIr`, `QuantDimIr`.

**`hole`** — `Hole`, `HoleKind`, `RelationSet`.

**`capability`** — `Capabilities`.

**`provenance`** — `Provenance`, `ProvenanceTable`, `SourceUnit`.

**`ids`** — the newtype ids and `EntityRef`. A row's id is its
index in its own table.

**`dispatch_plan`** — `DispatchPlan`, `SubjectPlan`,
`DispatchFlavor`. Conclusions derived from the model, never
authored facts.

**`obs_ids`** — `ObsEntityId`, `ObsEntityKind`, the join back from
an observed record to a model row.

Three of these are easy to mistake for each other, and the
difference is load-bearing:

- `GroupMember` is resolved membership; `GroupSelector` is the
  authored selector list. `{ lib::* }` and `{ lib::A, lib::B }` can
  resolve identically and must still hash differently.
- `Publish` is a send expression; `DeclaresPublish` is the declared
  endpoint. A locus that declares an end and never sends still
  publishes in the endpoint sense.
- `Call`'s `DispatchKind` is the *fact* of the mechanism;
  `DispatchFlavor` in the lowering plan is the *conclusion* drawn
  from it.

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
