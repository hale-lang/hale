# The canonical semantic model

The typed intermediate representation every structural consumer
reads. GH #476 built it; this document is its contract.

`spec/verification.md` specifies what the compiler *checks* and what
the topology artifact *serialises*. This one specifies the value
sitting between them: what a model contains, what it is allowed to
not know, the laws it must satisfy, and the rules a new consumer
has to obey.

The crate is `hale-model` — typed semantic tables with no dependency on the
AST, the checker, or codegen. Its identity and provenance mechanics
(`SeedId`, `SourceId`, `ProvenanceId`, `Provenance`, `SourceUnit`,
`ProvenanceTable`) are `hale-graph`'s, the graph core F.40 rebuilds
it on, re-exported under the model's own paths.

## The architectural law

```text
checked Bundle                      -->  ApplicationModel
Bundle + Model                      -->  ClaimIrTable
Bundle + Model + ClaimIrTable       -->  EvidenceTable
Model + ClaimIrTable [+ Evidence]   -->  Judged verdicts
ApplicationModel                    -->  model-half projection (artifact)
dispatch gates + arrangement        -->  DispatchPlan (the model holds it projected)
admitted Artifact                   -->  ComponentModel --> fleet ModelGraph
```

Reachability proof reports are judgment outputs kept outside
`ApplicationModel`. The existing successor projection supplies their graph
facts; finite closed-set/path checkers and Lean validate the certificates.
They do not derive a second model or certify extraction/lowering. The exact
export, replay and completeness boundaries are in
[verification.md](./verification.md#reachability-proof-export-gh-1327).

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

## The graph registry (F.40)

The law above was written for the model's consumers: artifact, fleet
and claim code cannot walk source for a modeled fact. It never reached
the producers, codegen or the CLI, which is why the bus graph is built
eight times per build, ownership has three accounts, and a fixpoint
over the call graph could land inside codegen without tripping
anything (GH #1208). F.40 (`spec/decisions.md`) extends the law to
**every semantic fact the compiler acts on**, in every crate, and
makes it checkable:

> One authoritative producer per semantic fact within a resolved
> compilation snapshot. Consumers ask for the fact by name; they do not
> re-encode its meaning from the AST, from another table's raw rows,
> or from a Debug string.

The registry is the index of that law. `spec/registry.md` — rendered
from `crates/hale-graph/src/registry.rs` and held byte-equal by a test
— lists every **semantic family** with its contract:

- what it **answers**, in one sentence, and what it **reads**;
- its **producer**: the authoritative one, or today's, or none yet;
- its **legacy producers**, each with the condition under which it is
  deleted — the exact inventory of what is permitted while the family
  migrates;
- its **consumers**, named, so a new reader is a registry change
  before it is a code change;
- its **invariants** and **missing-data policy**: whether a missing
  row is a compiler error or a hole with a stated policy; and, for a
  family lowering reads, which of two things each of lowering's reads
  is: **required** (the snapshot always provides the row, so a missing
  one is a `CodegenError` naming the family and the row, pinned by a
  test that removes it from a lowering view) or **total** (absence is
  itself the fact, and the registry says what no row means), never a
  default or a second derivation; `lowering_reads_are_classified`
  holds every family lowering reads to one of the two;
- its **focused tests**, the ones a contributor runs to know a change
  to that family is right;
- where the **spec** states the contract.

Each family is in one of three states, so the inventory can be exact
without banning what exists today: **Reserved** (a future family or
consumer, computing nothing), **Migrating** (the canonical producer is
under development and the legacy list is complete), or **Canonical**
(one production producer; consumers cannot reconstruct its meaning).
The registry also lists the spec's numbered rules with the code that
evaluates each — a registered rule with no evaluator fails the
compiler's own build — and freezes the `format!("{:?}", ..)` sites
that decide a fact from a Debug string, so no new one can appear.

The guard (`crates/hale-graph/tests/registry_guard.rs`) makes the
wrong workflow fail: a derivation-shaped function the registry does
not name, a reference to a family's guarded seam from an unlisted
file, a new Debug-string scan, a cited site that no longer exists.
The right workflow is the one the registry describes: change the
authoritative derivation or rule, update its contract tests, exercise
the registered interactions and consumers, and never patch a second
interpretation into codegen or the CLI.

Reserved entries exist so the tables never change twice: the
`runs_under(locus, principal)` relation column on the tower, the
`view` family and its `adequate_for` law, the evented `transitions`,
the `deployment` families, and `ui` and `bundle` as consumers beside
the LSP, `--dump-topology` and the description. A reservation creates
no production demand.

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
hale_types::model_builder::derive_application_model_over(&Bundle, &ModelInputs) -> ApplicationModel
```

It runs over a *checked* bundle. A model of an ill-typed program
describes nothing, so consumers that judge gate on the bundle
having no non-`Claim` errors first.

It builds none of the families it reads beside the program:
`ModelInputs` hands it the top scope with its topic rows, the bus
graph, the ownership graph and the handler rows, each built once
over the checked programs. Every verb demands them, and the model,
from its snapshot (`Snapshot::demand_model`, F.40 phase 2.3), and
so does every test: no entry builds them for a bundle no snapshot
holds.

**There is no other way in.** In particular:

- **No artifact → model.** The topology artifact is a *projection*,
  not a serialisation. It carries the model half (hashed by
  `shape_hash`) plus typed law, capability and adequacy sections,
  but several tables — `costs`, `locus_instances`, the analyses,
  the provenance interner — have no wire form. `hale topology
  graph` and the fleet tier *admit* an artifact by validating its
  JSON against the same rules; they do not reconstruct an
  `ApplicationModel`.
- **No fleet → model.** Fleet composition derives a fleet seed's
  `deployment` rows and reads artifacts; it does not elaborate a fleet
  into an application model. What it builds instead is instructive:
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

### The description: the model's first wire form (GH #1107, #1417)

One slice of the model does have a wire form, and it is deliberately a
*form* slice: the **description** of a served surface
(`spec/api.md` § The description; the format is
`spec/api-description.schema.json`). It is rendered from the `surface`
family: one row per `rpc` line of an `api` block or `@rpc` handler (the
surface, the member, the request, response and error types with their
contract shape hashes (§ The shape of a type), whether the error is the
server error, the pools and the required roles), from which the
surface's contract digest is folded and every description is written,
per exposure, from the rows dispatch reads. The type declarations
supply the field schemas the model does not hold. It does not consult
the model's other tables: it describes the same declarations the model
is built from, so the two cannot disagree about which locus a row
names.

Three properties are the contract:

- **Form, not params (I1, not I2).** The program-wide **inventory**
  (`hale check --api`) names what the program offers and never who
  holds a role: it is a deployment inventory, not an authorization
  statement for any caller. A served **exposure** adds the listener it
  was given and the caller it established, and says so: the identity of
  an exposure is its surface, its digest and its name.
- **Perspective-invariant (I5).** Every entry is a form claim over
  the program's declarations, so a `perspective` swap, a placement
  change or a second deployment describe themselves in the same
  bytes; two builds of one source produce one description, and a
  running exposure serves the document the compiler's rows write for
  the caller it names (the Unix `{"describe": true}` and
  `GET /.description` answer it whole).
- **Boundary honesty.** The document carries its notes in its own
  text: `requires` is a boundary check at the serve site, never a proof
  over internal call paths (unless the program states one: a group
  `{ role R }` names the handlers a holder of `R` may call, and a claim
  over it is checked over the call and bus paths; docs/src/claims.md §
  `group`), and a request accepted runs to completion
  whatever becomes of the caller's wait. A client renders the notes;
  it does not present a check as a certificate.

Its shape is pinned by `tests/api-contract/` (the R0 fixtures) and by
`crates/hale-cli/tests/fixtures/api_description/`; the OpenAPI 3.1,
JSON Schema and MCP forms are projections of a surface's rows
(`hale check --api --surface NAME`). It is not a reversible wire form of
the model — the boundary above stands — and it admits nothing: a
consumer reads it, never builds a model from it. The model holds the
surfaces and their rows (the `surfaces` and `surface_rows` tables,
below).

## Sorts — the entity tables

`Entities` holds seventeen tables. A row's **id is its index** in its
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

A `functions` row's id is the rank of its `name`; the row also
carries `decl`, the site its snapshot minted for the declaration. A
join from an analysis row (an allocation-summary or effect row, keyed
by that site) to its function reads `decl`, never the name. `decl`
is never rendered and never hashed, so it moves no dump and no
digest.

| table | what a row is |
|---|---|
| `functions` | free fn, method, lifecycle hook, mode, failure handler, locus parameter default, data-field default, argument default |
| `loci` | a locus *declaration* |
| `locus_instances` | a statically exact instance in the main arrangement (`App.w`, `App.workers[3]`) |
| `topics` | a declared topic |
| `subjects` | a wire subject or pattern — **address identity** |
| `payloads` | a payload contract — deliberately a different sort from the subject |
| `phases` | lifecycle phases |
| `seeds` | imported seeds |
| `thread_domains` | where arranged instances run: `main`, a pool, a pinned anchor, a binding's reader |
| `bindings` | transport bindings, with a `role` |
| `groups` | resolved claim groups |
| `types`, `interfaces` | type and interface declarations |
| `effect_classes` | declared **or merely referenced** user effect classes — `declared: false` is "referenced, never declared". Built-ins are a separate fixed vocabulary (`BUILTIN_EFFECT_CLASSES`) and have no row |
| `declarations` | the declaration universe, for coverage laws |
| `surfaces` | an `api` block, or a seed's default surface its `@rpc` handlers feed, with its contract digest (`spec/api.md`, GH #1417) |
| `surface_rows` | one row of a surface: the member as a caller names it, the handler's function, the request, response and error types (each its spelling, contract shape and hash), whether a failure is the server error, the pools the handler's locus runs on and the required roles as written |

The surface family has one law of its own beside canonical order (by
surface name; rows by surface, then member): a surface's digest is the
fold of its rows (`hale_model::surface::surface_digest`, the framing
of `spec/api.md` § The contract digest), so a row added, removed or
changed without the digest moving is not a model. Neither table enters
`shape_hash`: a program with no surface hashes as it did, and one with
surfaces gains these rows and no hash.

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

### The shape of a type

A type has a **shape**, a string, and a **shape hash**, the 64-bit
FNV-1a fold of the string's bytes (`hale_graph::identity::Fnv64`:
offset basis `0xcbf29ce484222325`, prime `0x100000001b3`, each byte
xor-ed in and then multiplied), written as sixteen lowercase hex
digits. One renderer (`hale_types::topic_identity::Shapes`) renders it
in two forms; the payload contract reads one and the surface rows the
other, and neither renders a shape of its own.

Both forms tag a field the same way when its type is flat:

| field type | tag |
|---|---|
| `Int`, `Uint`, an identity, a range (`spec/units.md`) | `i` |
| `Float` | `f` |
| `Bool` | `b` |
| `Decimal` (`Decimal<9>`) | `d` |
| `Decimal<S>`, `S` other than 9 | `d<S>` (`Decimal<18>` is `d18`: the scale is part of the contract) |
| `Time` | `t` |
| `Duration` | `u` |
| `String`, `StringView` | `s` |
| `Bytes`, `BytesView`, `BytesMut` | `y` |
| a quantity or a point | `q(<denomination>)`, `q(<denomination> point)`, `q(<denomination> point <origin>)` (`spec/units.md` § Layout and the wire) |

A struct's shape is its fields in declaration order, each
`<field>:<tag>`, joined by `;`. A **flat struct** is one whose every
field has a tag of the table; it has one shape whichever form reads it,
so one shape hash: `type OrderReceipt { order: OrderId; notional:
Money; }` is `order:i;notional:q(cent)`, `bb4f99639cf069af`, in both.

**The observation form** is the topic's observation identity (the
`topics` section of the artifact, `PROTOCOL.md` §4, mirrored byte for
byte by the runtime's `obs_fnv`, which folds the wire subject, `:` and
this string) and the `payloads` table's contract. Only a bare,
non-generic declared struct has one, and any field whose type is not
in the table is tagged `struct`, name-free, so the hash never depends
on a declaring binary's local type names. A payload with no observation
shape (an enum, a scalar, an array, a builtin record, a type the seed
cannot see) is the opaque contract `opaque:<type>`. This form is a wire
identity and does not move.

**The contract form** is what a surface row's request, response and
error are, which the contract digest folds (`spec/api.md` § The
contract digest). Every type has one, and it is deep, so a field
changed inside a nested struct, or a variant added to an enum, moves
the hash of every type that reaches it:

- a declared struct: its fields, each `<field>:<tag>`, joined by `;`;
- a builtin record (`ClosureViolation`, `IndexError`, … the records
  the compiler declares, `hale_types::builtin_types`): its fields as a
  struct's, `ClosureViolation` being `locus:s;closure:s;diff:i`
  (`36c7f0561125943e`); a declaration of the name in the program
  replaces it, as it does in the checker;
- an enum: `=enum(` its variants in declaration order joined by `|`,
  a variant written `<Name>` or `<Name>(<tag>,…)` `)`: `type Side =
  enum { Buy, Sell(Int, String) }` is `=enum(Buy|Sell(i,s))`;
- an alias, and a scalar that is neither an identity, a range, a
  quantity nor a point (`distinct Float`): the shape of what it stands
  for;
- any other type: `=` and its tag (`=i` for an `OrderId` a row names
  by itself, `=q(cent)` for a `Money`).

A field's tag in the contract form is the table's, or:

| field type | tag |
|---|---|
| a declared struct, a builtin record, an enum | `#` and its own contract shape hash: `body:#e661911904a01160` for a `body: Inner` with `type Inner { a: Bool; }` |
| one of those already being rendered (a recursive type) | `rec(<k>)`, `k` the number of enclosing named types out to it: `type Node { v: Int; kids: [Node]; }` is `v:i;kids:[rec(1)]` |
| an alias, and a scalar of the kind above | the tag of what it stands for |
| `[T]`, `[T; N]` | `[<tag>]`, `[<tag>;<N>]` (`_` for a size that is not a literal) |
| `bounded[T; N]` | `bounded[<tag>;<N>]` |
| a tuple | `(<tag>,…)` |
| a type the program declares nothing for | `opaque(<type as written>)` |

The nested type's slot is its hash, not its fields inlined, so a
type's contract shape is a function of its own fields and of the
hashes of the types it names; and because a flat struct's fields are
all tagged by the shared table, its contract shape is its observation
shape and **no flat struct's hash differs between the two forms**. A
struct with a compound field is the one kind whose two hashes differ:
`type Outer { id: Int; body: Inner; }` is `id:i;body:struct` as a
payload and `id:i;body:#e661911904a01160` as a row's type.

### The arrangement

`locus_instances`, `realizes`, `owns`, `placed_in`, `thread_domains`
and `affined_to` are the **placement table's rows, projected** (F.40
phase 3, the `placement` family); the builder walks no declaration of
its own for them.

- **What is arranged.** The instances of the root lowering deploys
  (the entry row's entry, never an imported `main`): one
  template per literal of the root, or the entry's implicit
  construction of a root no literal builds. Each instance is arranged
  where it runs: a held instance under its holder, as the rows its
  source actually built. A field is arranged as the declaration the
  table says it realizes, so an aliased field is the aliased locus and
  a contract-typed field the implementation that was built.
- **Paths and replicas.** A path is the fields from the root, with
  the replica index after the replicated field (`App.f[2].k`). A path
  has no construction component: it is arranged when every template
  and alternative enumerates it and agrees on the declaration and the
  domain; otherwise it is left out with everything under it, each
  declaration realized there a `RuntimeInheritedPlacement` hole naming
  the path. An unenumerable subtree in any template or alternative is
  disagreement, even if the other templates agree.
  `LocusInstance::replica` is the replica row's own index;
  an instance under a replica carries the index in its path and
  `None` in its `replica`.
- **Domains.** `thread_domains` are the domains the arranged instances
  run in (`main`, `pool:<name>`, `pinned:<anchor path>`) and one
  `binding:<topic>` per entry of the deployed root's `bindings { }`.
  `affined_to` gives a domain its resolved CPU set (a `cores` range or
  set, or a `node` / `l3` affinity resolved against the root's
  `topology { }`): a pool's is the set its one worker may run on, a
  pinned replica's its own thread's. A CPU set is a column of a domain,
  never a domain; two entries naming one pool give one row, and a
  domain with no affinity has none.
- **What is not arranged.** The projection is user-only and partial
  by design: a row realizing a stdlib declaration is left out with
  its subtree (its declaration is no entity of this model, and adding
  one would be shape), with no hole for it; an adapter of the root's
  `bindings { }` is no instance; a literal `fn main` builds besides the
  root is a birth outside the arrangement, a `RuntimeInheritedPlacement`
  hole on its declaration. Every placement question the checker and
  lowering ask is answered by the table, never by the arrangement.

Dynamic-birth provenance comes from the ownership graph (F.40 phase 3,
C3). Its walk records whether a locus-body literal belongs to a params
initializer and records free-function births alongside the owning-locus
sites. The model does not classify births by comparing source spans or
walk free functions again. In particular, expressions copied from API
bindings into generated params retain their source spans and their
params-default provenance. They do not add dynamic-placement holes;
connection peers created in bodies still do. A birth's resolved child
joins the model's declaration table by site, so a qualified stdlib name
cannot create a hole on a user locus with the same final name segment.

Three identity contracts hold the projection, each separately:

1. **Shape identity.** The arrangement is outside the shape half:
   `shape_hash` reads none of these tables, so no change to the
   arrangement moves it.
2. **Observation entity ids.** `obs_entity_ids` stamps subjects,
   locus declarations and bindings, never instances; the bindings are
   the deployed root's.
3. **Arrangement-instance correspondence.** `LocusInstanceId` is the
   index in path order, so an added or removed path renumbers every
   later instance. No numeric instance id is stable across builds; a
   consumer that holds an instance across builds joins by path.

## Relations

`Relations` holds seventeen tables. The invariant that matters is
**grain**.

Call and publish rows are **site-grained**: they carry an authored
`site` ordinal, two calls to one callee are two rows because the
callee executes twice, and alternatives of one interface dispatch
share that ordinal because one dispatch runs one conformer. A
consumer that collapses either into a set is computing reachability
where the language means executions. A call through a function value
resolves to the program's function values of its type (F.40 E5;
spec/verification.md), and the model has no function-value dispatch
kind: each alternative is a `Direct` row at a site of its own, so a
count over them is a sum where one execution runs one of them (the
`@budget` engines, which read the analysis's dispatch group, take the
max), and the site ordinals after such a call move when the set of
function values does. The same holds for alternatives inside a
`stdlib_absorption` interior.

`costs` is **mixed-grain** and carries no `site` ordinal at all;
its rows are distinguished by `(function, dimension, provenance)`.
See below.

| table | grain / notes |
|---|---|
| `member_of`, `phase_of`, `declared_in`, `realizes` | structure |
| `owns` | instance parent → child |
| `calls` | **site**; carries `dispatch`, `in_loop`, `unbounded` |
| `constructions`, `initializes` | typed locus/type constructor targets, supplied fields and exact omitted-default evaluations; separate from authored call sites |
| `dead_interface_calls` | uninhabited dispatch |
| `publishes` | **site**; carries `key_domain`, `in_loop`, `disposition` |
| `declares_publish` | the endpoint grain (`bus { publish T; }`) |
| `subscribes` | carries `key_predicate` — the filter half of delivery |
| `placed_in`, `affined_to` | placement: an instance's domain, a domain's CPU set (§ The arrangement) |
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

Fifteen kinds: `IndirectCall`, `UntypedReceiver`, `ComputedSubject`,
`UnknownKeyDomain`, `OpenInterface`, `RuntimeInheritedPlacement`,
`DynamicEndpoint`, `ExternalOpaque`, `UnsupportedArtifactSemantics`,
`UnanalyzedBody`, `UnmodeledRecovery`, `UnmodeledFailureDispatch`,
`UnmodeledInitialization`, `UnmodeledLifecycleContext`, `GenericFlowTemplate`.

`UnmodeledRecovery` is a function-grain behavioral gap. It hides `CALLS`,
`PUBLISHES`, `EFFECTS` and `COSTS`, and retains a recovery-statement witness.
Unanchored default/foreign recovery contexts conservatively affect every
function until callers can be bound. Schema 1.24 introduced its
`unmodeled_recovery` artifact residue.

`UnmodeledFailureDispatch` hides `CALLS`, `EFFECTS` and `COSTS`. It preserves
the distinction between invoking existing handler bodies and the inventory
of authored publish sites: `Publish.function` identifies the body containing
a send, independent of how that body is invoked. A missing handler body,
computed send or other independent publish gap remains unresolved. Failure
dispatch alone introduces no new authored send, so it cannot withdraw a
closed route-policy judgment by manufacturing an unknown publisher.
`HoleKind::implicit_execution_families` supplies the required masks for these
implicit-execution kinds; validation rejects a narrowed recovery mask or a
failure-dispatch mask that invents publish-site uncertainty. These are
whole-function gaps with no invented call-site ordinal.

The allocation-summary producer caches a reverse reachability forest from
its existing `violates` rows through resolved call edges and constructor
invocations of omitted locus parameter and data-field defaults, once per summary, using the
shared graph engine. Initializer nodes use a source-universe-qualified owner
and field identity; they are not callable functions or authored call edges.
Each constructor retains its own supplied fields, so an overridden default
adds no invocation edge. The existing body walk supplies each field's calls
and nested constructors; aliases and imports use the shared nominal resolver. Program projections preserve that forest and
its foreign source classification when library bodies are contracted away.
Affected user functions carry failure-dispatch gaps; there is no invented
executable edge to a handler. One reverse search over default-call coverage
retains each seed's family mask; a join with actual recovery keeps its broader
gap. Absence from the explicit-failure forest does not establish coverage of
automatic closure checks, other declaration-body execution, or runtime
admission. Ordinary locus-parameter effects and costs use the execution
account below.

Schema 1.25 adds the distinct `unmodeled_failure_dispatch` residue. An older
schema cannot carry that vocabulary. This addition does not reinterpret the
existing recovery hole or canonical-model semantics. Analysis semantics 15
also binds constructor/default structural-failure coverage. Schema 1.26 and
analysis semantics 16 extend ordinary locus-parameter execution as described
below.

Budget evidence withdraws a would-be `Holds` when either implicit-execution
gap is reachable through canonical calls (or possible delivery for fan-out).
The finite search retains an actual failure/recovery witness; an unrelated
handler's gap does not withdraw a closed function's budget. Reachability
retains these unordered behavioral gaps while exploring known selected edges:
a concrete counterexample or the existing saturation result still wins, and a
fruitless search with a relevant gap is `Uncertified`, with no closed-set proof.
Other authored unresolved-call holes keep their existing stop rule. This is a
coverage gate, not a new cost count.

Model semantics version 6 adds closure assertion bodies and exact explicit
checkpoint accounts, retaining scoped comparison failures and unresolved caller
contexts. Version 5 adds call-site argument-default execution with callable
ownership and supplied-argument counts. Version 4 adds type-owned data-field defaults and typed
constructor targets. Version 3 includes locus parameter-default execution bodies
and distinct constructor evaluation edges. Model semantics version 2 allowed failure handlers to be analyzed and
summarized under the same coverage laws as other executable functions.
Version 1 required them to remain unanalyzed. The current model validator
refuses older and unknown model semantics. A handler with a canonical
summary has no blanket `UnanalyzedBody` hole; any unresolved sites retain
their ordinary scoped holes. Artifact semantics version 3 and analysis
semantics version 12 bind this change for downstream consumers and proofs.

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

`Capabilities` is the **positive** completeness account: twelve
independent booleans (`exact_calls`, `exact_publishes`,
`exact_subscribes`, `exact_key_filters`, `exact_ownership`,
`exact_placement`, `exact_routes`, `exact_effects`,
`exact_cardinality`, `exact_delivery_guarantees`, `exact_costs`,
`exact_flow_classification`),
each vouching a `RelationSet`.

`exact_publishes` vouches for the direct authored-site inventory. It does not
prove invocation order, multiplicity, admission or completion; call/effect/cost
coverage remains independently necessary for claims about that execution.

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

- **`recovery`** — typed recovery observations from the user's checked
  bodies and the separate stdlib typing walk (`RecoveryInventory`). Each
  optional inventory distinguishes unavailable from available-and-empty.
  The source-site mint's existing lexical-expression walk records whether it
  saw recovery syntax. Proven stdlib syntax absence supplies an empty
  inventory without forcing a whole-stdlib typing walk. Positive or
  unavailable presence still uses the actual typed observations; no handler
  signature or missing typed row substitutes for this absence proof.
  `RecoveryAction` retains an optional model body anchor, handler status,
  specialization symbol, typed operation and receiver type, symbolic operand,
  retry bound, and source witnesses. A symbolic root is the current locus,
  an immutable parameter by body and ordinal (with its arity), or unresolved;
  a field path reads fields at the statement. Mutable aliases, saved field
  values, computed values and call results are unresolved, even when their
  type is known. Immutable aliases of whole immutable locus roots retain
  that root. This prevents an
  alias captured before field replacement from becoming a live field lookup.
  Literal bounds retain signed values; dynamic bounds retain their expression
  provenance. Evaluated defaults retain separate, outermost-first evaluation
  witnesses for each invocation. Their body and operand root remain unresolved
  until those evaluation contexts are bound; the default expression's offsets
  remain foreign rather than indexing caller text. Unresolved evaluation sites
  remain `None`, not invented source locations. The outer user invocation
  retains provenance when its minted site is available. Nested bare site ids do not carry
  their source universe, so they remain unavailable rather than aliasing a
  same-numbered user site. Generic walks retain their concrete monomorph symbol without
  using a display or Debug rendering as machine identity. Unmapped contexts
  remain explicit. The stdlib side never joins user ids and has foreign
  provenance. Validation rejects dangling references, malformed parameter
  ordinals, empty names/paths, illegal literal/dynamic bound operations and
  source-universe confusion.

  The recovery inventory also carries optional call-operand bindings joined by
  caller, authored invocation identity and callee declaration. A saved field
  value never becomes a live ownership path. `bind_recovery_call` and
  `bind_recovery_operand` substitute static arranged-instance IDs under a
  supplied invocation premise. They establish no failure dispatch, guard
  feasibility, execution order or containment verdict. Missing contexts and
  default arguments are explicit operand-scoped gaps. These internal facts
  are not part of topology artifact identity or a closed recovery capability.
  They must not be used as artifact proof inputs without a corresponding
  versioned binding.

  `failure_entries` retains conditional handler-entry contexts for every arranged
  logical instance. The arrangement keeps all contributing construction keys,
  even when their declaration and domain agree. Fresh field rows record whether
  the construction uses its literal's field holder or its declaring params
  initializer. Lowering requires this column where the table represents a fresh
  field, checks its index, and checks that the selected context source agrees.
  The ownership family's shared `supervision_override` query selects the source;
  no allocation-owner edge alone supplies a supervisor.

  Held projections retain their original `built_by` source. Moving a parent does
  not change its children's original supervisor; moving a child does not give it
  a new one. A unique original-source correspondence can name an arranged parent;
  multiple logical aliases remain explicit instance/supervisor gaps. Each context
  retains a root-to-field construction witness path and an alternative guard bit.
  Missing roots, unconstructed catalogs, missing sources, unmapped handlers and
  unbound generic specializations remain unknown. A known unhandled route is
  separate from unavailable routing facts. Handler identities come from the
  shared routing rows by declaration, joined to the canonical function table.

  `recovery_handler_invocation` substitutes that conditional context into a
  handler frame: its actual constructed-field supervisor is `self`, and its
  failed-child argument names the selected logical instance. Every contributing
  context must be accounted for. These facts do not establish a live runtime
  instance, incarnation, feasible guard, failure trigger, params settlement,
  scheduling/admission or unchanged field state. They are not exported proof
  inputs, change no recovery completeness capability and upgrade no verdict.

  `RecoveryQueries` validates and indexes these model tables once. Its
  conditional boundary probe follows every construction alternative with the
  shared graph engine. A search state keeps the construction context, function,
  receiver and parameter instance IDs; recursion cannot merge sibling operands
  or produce an infinite chain of identical frames. Call witnesses retain the
  canonical relation index and typing alternative. Recovery operands bind only
  through the existing substitution queries and must agree with the receiver's
  locus declaration. Field state, omitted operands, absent call accounts,
  specializations without an execution binding, implicit entries, contracted
  stdlib execution and deliveries remain scoped gaps. An action's implicit
  lifecycle behavior remains a gap even when its receiver is known.

  The probe can identify a possible represented receiver outside a supplied
  arranged-instance set; that source witness wins over other uncertainty. A
  fruitless search retains the first gap. An explicit search limit returns
  saturation, never absence. `NoRepresentedEscape` describes only the inspected
  source graph and is not a containment verdict: runtime admission, incarnation,
  guard feasibility, recovery effects and native correspondence remain separate
  obligations. The model dump reports at most the first target per logical
  instance, with a 512-state inspection limit. These observations do not change
  model/artifact identity, evidence inputs, capabilities or existing verdicts.

  Evaluated default calls remain separate from counted call rows. The model
  retains their outer source witness and causally evaluating function when
  available; nested numeric ids do not qualify a source universe. One shared
  reverse graph closure checks possible recovery reachability, including nested
  defaults and opaque calls. Relevant unresolved execution leaves scoped
  `UnmodeledRecovery` gaps. Temporary default-walk undo keeps executable
  observations; declaration-only and locus name probes supply no such facts.

  These observations are **not** a behavioral recovery relation. Conditional
  parameter substitution does not establish runtime instance identity, field
  state, guards, admitted handler invocations or operation execution order.
  There is no recovery completeness
  capability or containment judgment over them yet. `model_query::recovery_observations`
  returns selected-body actions together with all unanchored observations
  and inventory availability; it never silently drops the latter to certify
  a smaller world. The current topology artifact does not export this
  inventory. Authored failure handlers now have canonical function summaries,
  call, effect, publish, cost and phase rows. Each joins by its declaration
  identity, so display spellings are never a join key. Duplicate handler
  signatures remain checker errors. Ordinary unresolved
  call sites retain their scoped holes. This closes the same syntactic
  call/effect horizon as other bodies. Calls through locus type aliases use the canonical
  locus resolver and retain the template declaration of a generic target.
  Function-valued fields retain value dispatch when their declared type is
  a function alias or a generic parameter. Indexed array receivers use the
  summary's existing element-type facts: declared arrays and uniform literals
  retain their element's method target. The index expression is still evaluated
  and contributes its own calls and capabilities. Unknown element types retain
  their opaque-receiver gap. A receiver typed by a lexical generic
  parameter still has no single concrete dispatch target and retains an
  opaque-receiver hole; its spelling cannot bind to a same-named global
  locus. This does not establish which runtime
  recovery instances or guarded calls execute. Recovery
  operations retain `UnmodeledRecovery` gaps for their implicit lifecycle
  execution and costs, so observing their explicit helper calls cannot
  certify the hidden work. A named receiver with no established
  locus declaration may be an interface; it is not evidence of a non-locus
  value. A future behavioral consumer must add its relation/capability,
  scoped typed holes, artifact/schema contract and adequacy gate first.
- **`dispatch_plan`** — the program's one dispatch plan (below),
  projected onto the subjects the program's own bus sites name and
  the loci the model declares: each kept row's subscriber column
  holds only those loci, sorted; every other column is the plan's.
  The stdlib's `log.**` row appears only where the program names
  `log.**` itself, and never with the stdlib's sinks.
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
- **`DispatchPlan`** — (GH #476 Change 8) derived once per program
  from one gate set, the bus graph's sites keyed by wire subject with
  the bundled stdlib's after them, and the arrangement's thread
  domains. It decides how a BUS SUBJECT dispatches — dynamic, static
  bucket, or static direct — not how calls in general lower.
  Lowering lowers it and the model holds it projected
  (`Analyses::dispatch_plan`); which flavour a subject gets is a plan
  *conclusion*, never a model row.

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
| `coverage_digest` | per locus: name + `analyzable`. Per function: name, `analyzed`, `summarized`, a **failure-handler discriminator** — not the full `FunctionKind`, so a move among `Free`/`Method`/`Hook`/`Mode` is not in that byte — plus **canonical locus/type/callable owner** and optional default **kind, field and ordinal** | coverage changes, a member moves between owners, or its default identity changes |
| `model_shape` (in `EvidenceTable`) | the `shape_hash` the sidecar was derived beside | the model half does |
| obs `model_hash` | **the emitted `shape_hash`** — the build's snapshot's model's model-half digest (`project_shape_hash`, the value the artifact of that model stamps), read from the model, never from a rendered artifact | the model half does. It is the runtime exposure of `TopologyShapeV1`, **not** a full-model identity |
| obs `entity_id_digest` | the exact stamped id table (kind, name, id) | the numbering does. It exists because `model_hash` does *not* cover every table the ids index — arrangement bindings are not in the artifact at all, and an unused topic's wire subject rides an unhashed section, so two builds could share a `model_hash` while numbering entities differently |
| `TOPOLOGY_SCHEMA` | the artifact's decoding contract | a field becomes required, a section changes meaning, or a family moves (1.18 added the per-locus `contracts` section) |
| `TOPOLOGY_DIFF_SCHEMA` | the `hale model diff` document's decoding contract | a diff section changes meaning or a field becomes required |
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

## Diffing two artifacts

`hale model diff <a.topology> <b.topology> [--json|--text]` is the
semantic difference between two topology artifacts (GH #527). It
is defined over **artifacts**, not in-process models: both inputs
are byte-reproducible, digest-verified documents a caller can keep
as evidence, so the diff is replayable by anyone holding them.
Each input passes the renderer's admission gates — canonical
top-level order, `artifact_digest`, `semantics`, recomputed
`shape_hash`, exact `TOPOLOGY_SCHEMA` — and an artifact that fails
any of them is refused, never diffed.

The output is a versioned document (`TOPOLOGY_DIFF_SCHEMA`, now
`1.0`) naming both inputs by path, `artifact_digest` and
`shape_hash`, with these sections:

- **`declarations`** — one row per (kind, name) across loci,
  topics, fns, groups and claims: `persisted`, `moved` (same name
  and kind, a different source unit), `renamed`, `split`, `joined`,
  `added`, `removed`, or `ambiguous`. Rows carry both sides' source
  sites (`unit`, `span`), with units taken relative to each
  artifact's common source root so two checkouts of one program
  compare.
- **`contracts`** — per paired locus, facet by facet, as set
  differences (`removed` / `added`): `sealed`, `params`, `methods`,
  `publishes`, `subscribes` (with the subscription's queue bound
  and shed policy), `supervises`, and the `ownership` (who owns its
  instances) and `placement` (which thread domains) — by owner and
  domain, not by instance path, so a param rename does not echo
  here. Plus, per paired topic, its `subject` and payload `shape`
  when they moved: a wider payload is a contract change for every
  locus on the topic. These facets are read from the artifact's
  `contracts` and `topics` sections (schema 1.18+), the per-locus
  regrouping of facts the model already holds. A param's type is
  rendered in author spelling (`dna::Leader`, never the mangled
  cross-seed symbol), like every other name in the artifact.
- **`effects`** — per paired fn, classes `gained` / `dropped`; per
  fn present on one side only with a non-empty effect set, a row
  with `change: added` (its classes as `gained`) or `change:
  removed` (as `dropped`) — an added locus whose handler reaches a
  declared class widens what the program does, and the paired rows
  alone would miss it; and per fn-grained certificate (`lowered`
  rows keyed by subject and form), `added` / `removed` / `result`
  changed.
- **`law`** — per paired claim, `form` / `result` / `verdict` /
  `family` changes; claims added and removed with their result;
  `adequacy` changes per family; the overall `verdict` when it
  moved.
- **`classification`** — `identical` (same `artifact_digest`);
  `source-only` (same `shape_hash`, no contract or effect rows:
  comments, moved lines, re-spelt law); `contract` (same
  `shape_hash`, but a contract facet or an effect set moved — a
  payload that gained a field, a param added — which the hash does
  not cover); or `model-shape`.

**Matching is deterministic and names what it compared.** The
`matching` section of every document states the rules; they are:

1. Declarations pair by (kind, name). Loci are matched first.
2. A removed and an added declaration of one kind pair as a
   **rename** only when each is the other's *unique* shape-
   signature match. A locus's signature is its contract facets
   minus the instance paths (which embed its name); a topic's is
   subject and payload shape; a fn's is owner, kind and effect
   classes; a group's is its members; a claim's is its rendered
   form. More than one candidate on either side is reported as
   `ambiguous`, with the candidates, and the declarations stay
   `removed` / `added`.
3. An accepted locus or group rename is read through wherever the
   name recurs on the A side — fn owners (`Old::m` pairs with
   `New::m`), group members, param types, supervised children,
   claim forms — so a rename does not echo as a contract or law
   delta in every declaration that mentions it.
4. A **split** is one removed locus whose local method set is
   exactly partitioned by two or more added loci; a **join** is the
   mirror. Anything short of an exact partition is not a split.

Nothing is guessed. The rule about joins that make wrong
hypotheses feel evidenced applies with full force to a tool whose
output is a review view: a wrong "renamed" row would hide a
removal and an addition behind one reassuring line.

## Placement judgments

`require separate_domains(all A, all B)` projects each group's locus
declarations through `realizes` to every represented instance, then
through `placed_in` to its execution domain. `model_query::instance_domains`
is the shared query; it preserves instance and domain identity, source
provenance, missing placement rows, unrepresented declarations, and
selected declarations' placement/population holes as distinct facts.
Selecting a declaration does not implicitly select its descendants.

The `placement` judgment family declares `OWNS | PLACED | CARDINALITY`.
It refuses invalid selectors and empty populations, reports a concrete
shared-domain pair as `violated`, and fails closed as `uncertified` on
relevant unknowns when no collision is known. The complete verdict
contract is in `spec/verification.md`. Analysis semantics version 9
adds this family; topology schema 1.20 adds its required adequacy entry
and the typed `require_separate_domains` law with `left` and `right`
group references. Artifact admission validates these operands, their
rendered form, and the evidence account using the existing law reader.
It does not reconstruct instance placement or evaluate a second model.

## Resource judgments

`bound queue_slots <= N on instances G` joins the group's selected locus
declarations to every arranged instance and to every subscription registration
of that declaration. `model_query::queue_reservations` owns this query;
replicas, repeated constructions and distinct subscription sites retain their
identity and source provenance. Descendants are selected only when their
declarations are members of the group. Keys and execution domains do not
discount declared capacity.

The `resource` family reads `OWNS | SUBSCRIBES | CARDINALITY`. A finite
reservation is the sum of the selected registrations' capacities. The
arithmetic does not wrap at a machine integer boundary. A known unbounded
registration or a known subtotal above the limit is `violated`; a relevant
population/subscription hole prevents `holds` otherwise. Invalid selections
and selected declarations with an exact zero population are `invalid`.
An instantiated locus with no subscriptions contributes zero slots.

Topology schema 1.22 adds the typed `queue_slots` law (`group`, `limit`)
and the required `adequacy.resource` entry. Older schemas cannot carry this
law. Admission validates the law account without reconstructing a model.
This is a declared subscription reservation, independent of runtime queue
occupancy, payload storage, topic-wide quotas and in-flight processing.

## Flow classification and retained children

`LocusDecl::flow` records the shared completed `flow_instances` family's
classification at declaration grain:

- `Resident`: no release clause classifies this concrete declaration as
  a flow.
- `Flow`: a nonempty, canonical list of `FlowRelease` witnesses names the
  classifying owners, their concrete specializations when applicable,
  and the release clauses' provenance.
- `GenericTemplate`: this declaration has type parameters; the model
  asserts no single concrete classification for all its specializations.

The snapshot supplies the completed inventory to `ModelInputs`; the
model does not discover generic instances or rederive membership. A
missing inventory blocks model construction. Owners in the application
join by `LocusDeclId`; owners in the separate stdlib analysis universe
remain external, and their offsets remain foreign provenance even if
numerically equal offsets exist in application text. `validate` rejects
empty flow witness lists, duplicate or unsorted witnesses, dangling owner
or provenance IDs, and empty external/specialization identities.

`model_query::flow_classes` projects selected declaration IDs into flows,
residents and non-concrete templates, preserving release witnesses. It
uses no arrangement counts, source traversal or lifecycle plan.

This is type-wide classification: even an uninstantiated owner's release
clause can classify a child. It does not identify the accepting owner of
a particular birth. A resident classification alone does not mean that
an instance survives until owner teardown: scope and ownership also
matter. A flow classification does not prove that `run` returns, that
field completion and owner cancellation are safe on every path, or that
live children or retained bytes have a finite bound. Those judgments
require birth, ownership and retention facts at their own grains.

`require flow(all G)` checks this declaration-grain policy through the
`flow_policy` judgment family. It projects locus declarations from `G`;
no selected loci, an absent group or a refused selection is invalid.
Selected generic templates are uncertified. A known concrete resident
is a counterexample even alongside a selected template. A withdrawn
capability with no typed account of its gaps is uncertified before
using absence of membership as a resident counterexample.

The family reads `FLOW_CLASSIFICATION` (bit 13). The positive capability
is `exact_flow_classification`; each `GenericTemplate` requires a
`GenericFlowTemplate` hole at its declaration hiding exactly that family.
Such a hole on a concrete declaration is invalid. A template withdraws
global adequacy for this family without poisoning claims over unrelated
concrete types. Other judgment families do not read this bit.

Topology schema 1.23 adds `require_flow`, `adequacy.flow_policy` and the
classification capability. Optional hashed `flow_classes` rows record
non-default `flow` and `template` tags, sorted by locus display name;
omitted loci have the concrete `resident` default in this schema. The
field is independent of law selection. It is absent when every locus is
resident, preserving those shape hashes. Flow or template classifications
now participate in shape identity; affected recordings need re-recording.
Admission validates tag vocabulary, identity, order, duplicates, schema,
capability contradictions and law/evidence binding without evaluating a
second flow judgment. This account was introduced with analysis semantics 11
and model semantics 1, adding facts without reinterpreting existing rows.

## Storage and admission configuration

`LocusDecl::storage` holds declaration-level configuration from the
allocation summary's `LocusShape::projection` row. The row says whether
an accept hook is declared and which storage policy is effective:
explicit annotation, or chunked for an acceptor and rich otherwise.
Recognition records the literal cap and selected mode. The declaration's
provenance covers this nested value. A missing summary row becomes
`None`, never an uncapped policy; `model_query::storage_policies` preserves
that distinction.

Lowering consumes the same rows through the view's required
`storage_projections` table. Concrete generic instances inherit their
template's literal policy under the shared closed inventory's names;
lowering neither rereads annotations nor reinfers defaults. Missing
tables, declaration rows and monomorph rows are named `alloc_summary`
refusals. `LocusShape::recognition_cap()` projects the recorded class,
keeping the allocation-summary text format unchanged.

These are configuration facts. An unshipped mode is not an enforced
allocator, and a cap alone does not bound all retained children, owner
populations, historical slab bytes, queued construction requests or
restart overlap. No storage-bound judgment consumes this column yet;
it is not a new artifact law or projection field. A consuming judgment
must add its required relations, completeness, source witnesses and
artifact/schema account together.

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
reachable-but-undescribed.

Field-level documentation is the rustdoc — every public field
carries a doc comment, and a test fails if one lands without. It is
published with the book at **`/api/hale_model`**, so reading a row
does not require cloning the compiler.

**`application`** — the model itself and its sidecars.
`ApplicationModel`, `ModelHeader`, `ModelHashKind`, `Entities`,
`Relations`, `Analyses`, `LabelRow`, `WeightRow`, `ModelError`.
Recovery observations: `RecoveryInventory`, `RecoveryFailureEntry`,
`RecoveryFailureContext`, `RecoveryConstructionSource`, `RecoveryFailureDispatch`,
`RecoveryFailureGap`, `RecoveryAction`, `RecoveryOperation`,
`RecoveryReceiverType`, `RecoveryOperand`, `RecoveryRoot`, `RecoverySpecialization`,
`RecoveryBound`.
Absorption: `StdlibAbsorption`, `AbsorbedNode`, `AbsorbedEvent`,
`AbsorbedTarget`, `AbsorbedHoleKind`. Evidence: `EvidenceTable`,
`EvidenceRow`, `CertificateEvidence`, `VerdictIr`. Plus
`DispatchGate`.

**`entity`** — the fifteen sorts. `Function`, `FunctionKind`,
`LocusDecl`, `LocusStorage`, `StorageProjection`, `RecognitionMode`, `FlowClass`, `FlowOwner`, `FlowRelease`, `LocusParam`, `LocusInstance`, `Topic`, `Subject`,
`PayloadContract`, `Phase`, `Seed`, `ThreadDomain`, `Binding`,
`BindingRole`, `TransportKind`, `Group`, `TypeDecl`,
`InterfaceDecl`, `EffectClassDecl`, `EffectClassDefinition`,
`Declaration`, `DeclKind`.

**`surface`** — the surface family (GH #1417): `Surface`,
`SurfaceRow`, `RowType`, and the digest framings
(`surface_digest_input`, `stream_digest_input`) with their folds.

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
`DispatchFlavor`. Conclusions derived from the dispatch gates and
the arrangement, never authored facts.

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


### Constructor-default execution

A locus parameter default is an executable body with
`FunctionKind::ParameterDefault`, a real parameter declaration site, a locus
owner, and typed `Initializer { kind: Parameter, field, ordinal }` metadata. It is not a
callable name or a generated native function. Its canonical name is
`Locus::params::field`. Locus groups include these owned executable bodies;
phase roots still select only the phase relation.

A data-field default has `FunctionKind::FieldDefault`, its real field
declaration site, `type_owner: TypeDeclId`, and `InitializerKind::Field`.
Its canonical name is `Type::fields::field`. It has no locus owner or
`member_of` row; no fake locus or free function is introduced. The built-in
`program` group includes main-seed field bodies, even when unused, and a
`lib::*` selector includes the imported seed's field bodies. Imported bodies
are not silently included in `program`; execution from a selected caller
still reaches them through constructors.

`constructions(function, owner, site, supplied, in_loop, unbounded)` records
source constructors in their own site namespace. `owner` is a typed
`ConstructorTarget::Locus` or `ConstructorTarget::Type`.
`initializes(from, to, owner, field, site, member, in_loop, unbounded)` evaluates exactly the omitted
defaults, in declaration order. These are separate from authored `Call` rows,
call ordinals, dispatch alternatives and call-operand bindings. An explicitly
supplied field has no default evaluation. Validation recomputes the entire
omission account from constructor overrides and default definitions, including
source witnesses and loop facts; evidence coverage uses the same validator.

The existing body walk supplies each default's calls, constructors, effects
and local stack charges once. The executable row participates in ordinary
call resolution; the editor's declaration-body copy is refreshed afterward.
`model_query::executions` exposes both invocation relations with typed
`ExecutionSite` namespaces. Reachability, quantitative paths, communication,
causal and dependency consumers share that projection. Helper effects remain
on the helper's body: an initialization edge can reach that helper, and an
`avoiding` mask can still stop there. Closed/path graph certificates use these
actual vertices and edges.

Known stdlib defaults enter the existing absorption graph. An entry carries
typed initialization metadata, and its site is a constructor ordinal; it
joins no authored-call dispatch group. Interior default evaluations retain
the same distinction, real default-body vertices and helper edges. Module
barriers match the actual stdlib owners. The contracted re-emergence account
also follows these executions. Fan-out costs use separate call, constructor
and delivery namespaces, summing distinct omitted defaults.

Per-call costs include each evaluated default. Distinct omitted defaults are
sequential contributions, not alternatives of an interface dispatch. Stack
charges come from the summary's original body walk, including locals inside
expression blocks. Inline defaults start with zero frame overhead;
initializer local and helper charges are summed conservatively in the
caller. This retains
the existing local-sizing/ABI assumptions rather than establishing an
independent physical-stack or retained-byte theorem.

A default absent from both the canonical bodies and the known absorption horizon has an
`UnmodeledInitialization` gap hiding calls, effects and costs. It invents no
publish site: direct publishing from defaults is prohibited, while existing
helper bodies retain their own authored publish inventory. These gaps are
scoped through invocation edges and preserve concrete known violations.

Schema 1.27 exports the hashed `sorts.types` universe and unified
`relations.initializers` definitions. Constructor and initialization rows
carry `owner: { kind: "locus" | "type", name }` and a `member` ordinal.
`law.types` binds raw/display type identities to that hashed universe;
function coverage carries `field_default` and `owner_type` for field bodies.
Admission checks exclusive ownership, default definitions, constructor
omissions and typed target agreement. It cannot interchange a record and a
locus. Schema 1.26's locus-only `parameter_defaults` account remains readable;
older schemas reject the new vocabulary. This account was introduced in model
semantics 8 and analysis semantics 21. Explicit closure checkpoints
have the separate execution account below. Automatic closure epochs and
runtime lifecycle execution still require their own source/coverage accounts;
this constructor-default projection does not establish those obligations.
The source summary joins each resolved constructor to automatic assertion
declarations by the real owner identity, preserving the source universe. The
pending epoch work is an `UnmodeledInitialization` gap at that construction's
function, hiding calls, effects and costs. It is not an ordinary call or a
synthetic per-call frame charge. Reachable evaluated defaults retain their
own construction obligations; supplied defaults do not execute. Tick/duration
obligations conservatively retain the unresolved cell-admission context,
including when this analysis has not established that a cell can occur.
Model semantics 7 requires a complete calls/effects/costs gap at each
represented automatic-assertion construction and its source provenance.
Schema 1.30 admission checks the corresponding constructor/epoch/residue
account. Earlier explicit-checkpoint accounts remain readable. An artifact
bound to the current analysis inputs cannot evade automatic-assertion residue
by relabeling its envelope as schema 1.29; artifacts with older analysis inputs
already require a new dump.

Record constructor overrides are now part of the hashed model half. Renaming
a supplied field therefore changes `shape_hash`, even when the field has no
default. A field's representation alone remains outside this identity;
per-topic payload hashes continue to bind the wire layout separately.


### Argument-default execution

A callable's parameter default is an executable `ArgumentDefault` body with
`InitializerKind::Argument`, the actual parameter binding site, and an
`argument_owner: FunctionId`. Its name is `callable::args::parameter`.
A method or mode default retains the callable's locus owner and `member_of`
row; a free-function default has its callable owner and no locus owner.
Defaults cannot own other defaults. This identity describes inline evaluation,
not an emitted native function or a callable source symbol.

`Call::supplied` records the explicit argument count of a named invocation.
`argument_evaluations(from, to, callee, site, argument, parameter, in_loop,
unbounded)` enters every omitted default at that actual call. Supplied
arguments skip their defaults. The evaluation's callee, source call ordinal,
loop facts and provenance must agree with the call, and its target must belong
to the named callable. Model and evidence admission share the validator that
recomputes the exact omission set. Constructor and argument evaluations keep
separate namespaces; arguments do not become constructor rows, authored calls
or dispatch alternatives.

The shared summary's resolved call and source arity supply these facts.
Function-value and interface dispatch carry no named-default expansion;
their arguments must be supplied. Function bodies and argument evaluation
remain separate executions. A contract or graph selector on a function body
starts inside that body; the caller's contract includes its evaluated
arguments. The `program` and imported-seed selections also include declared
argument-default bodies, as they include other declared executable defaults.
A default's own generic scope is retained; an unresolved symbolic receiver
withdraws coverage rather than becoming a zero-effect leaf.

Effects, per-call costs, structural-failure coverage and graph queries follow
the same evaluation edges. Distinct omitted arguments add their costs, across
parameters and calls. Inline defaults have zero native-frame overhead and
retain local/helper charges. Helpers remain separate vertices, including
under `avoiding`. Foreign defaults enter the existing absorption graph with
typed argument metadata and the actual call ordinal, distinct from both
constructor entries and ordinary call dispatch.

Schema **1.28** adds `argument_default`/`owner_function` coverage, function-owned
initializer definitions, hashed `argument_calls` and `argument_evaluations`
tables. These tables bind supplied counts and evaluations to the represented
call graph. Older schemas reject the new vocabulary; the reader retains the
1.26 locus-only and 1.27 typed constructor accounts. Artifact admission is a
structural check of this exported account, not reconstruction of an
`ApplicationModel` or evaluation of a second language semantics.


### Explicit closure-checkpoint execution

A closure with an assertion contributes one `ClosureAssertion` executable body,
identified by its closure declaration site and owned by its declaring locus.
Its source name is `locus::closures::name`. It is not entered in callable-name
lookup. `closure_definitions` records the actual owner, declared epoch and
closure declaration ordinal, including assertion-less inline declarations in
that order. Inline closures introduce no assertion body. Duration thresholds
are retained beside assertion operands by the same source body walk.

An authored `check_closures()` statement has a `closure_checks` row with its
actual call identity, a dedicated checkpoint ordinal, implicit-self owner,
repetition facts and source provenance. It evaluates each explicit-epoch
assertion of that owner, in closure declaration order. `closure_evaluations`
records those edges separately from authored calls, constructor defaults and
argument defaults. Its exact account is validated before model/evidence use;
a missing assertion, lost check, wrong owner, duplicate or changed loop fact
cannot silently become empty behavior. Checkpoints with no explicit assertions
remain represented no-ops. Function selectors retain the assertion/helper
boundaries; a graph witness renders `checks closure` at the evaluation edge.

The shared effects, allocation, quantitative and graph consumers follow this
execution relation. Conservative stack charges account for assertion operands
inside the generated epoch wrapper without asserting a native frame per
assertion. Distinct evaluated assertions contribute separately to counts.
Known operand effects can supply a concrete forbidden path even when the
comparison's failure branch remains unknown.

The comparison can report or dispatch a structural failure. Until that
conditional execution is modeled, user assertions retain a scoped
`UnmodeledFailureDispatch` gap; a barrier excluding the assertion excludes
its gap too. This is not an arithmetic proof that the assertion holds.
A checkpoint inside an omitted default uses its caller's implicit self. An
unbound caller context retains an explicit execution gap and categorical
reporters refuse certification, rather than borrowing the callee's declared
locus. Supplying the argument skips that default and its contextual checkpoint.

Schema **1.29** exports `closure_assertion` function coverage and the three
closure tables in the hashed model half. Each definition binds its exact
function identity, owner, name, epoch and declaration ordinal; checks and
evaluations preserve separate checkpoint ordinals and repetition flags.
Artifact admission checks their structural account without reconstructing an
ApplicationModel or interpreting laws. Source SiteIds remain in the canonical
model; artifacts use stable per-body ordinals, so source motion does not change
this structural identity. Earlier envelopes reject this vocabulary. Schema
1.28 argument accounts and earlier supported constructor accounts remain
readable. Automatic birth, dissolve, tick and duration execution is separate
work; retaining an epoch declaration does not establish its invocation path.


### Source phase membership

`AllocSummary::source_phases` is produced in the existing declaration
inventory, with the real function key, typed locus owner, phase name,
substrate/source-slice role and source span. The provenance/seed view
consumes this inventory rather than surveying phases again. Ordinary phase
certificates, the canonical `phase_of` relation and artifact projection
therefore select the same bodies. Imported owners retain their identity
universe; lexical spelling does not substitute for owner identity.

Automatic assertion bodies belong to their birth, dissolve, tick or duration
epoch. This source membership exists without an authored hook. Explicit
assertions have no independent phase: their execution edges inherit the
checkpoint caller's phase. A method named `tick` keeps its method role while
a tick assertion keeps its substrate role; both are sources of the named
phase. Modes, failure handlers and module-nested phase contracts use the
same inventory.

Model semantics 11 requires exactly one source phase for each hook, method,
mode, failure handler and birth check. Automatic assertions require their matching epoch
phase; explicit assertions and inline default bodies have no independent
phase. Dropping an authored phase relation therefore cannot manufacture a
bodyless phase certificate. Schema 1.34
admission checks the corresponding epoch/phase/role account, including
downgraded envelopes bound to current analysis inputs. It selects phase
bodies through the exported phase relation and locus ownership, not a
`Locus::phase` callable-name convention. Only a lifecycle phase with no
represented source bodies can receive a synthetic Holds certificate. A
missing certificate over a real source phase stays Uncertified.

This relation selects source roots for `during` and phase certificates.
Construction scheduling, retained lifetime, primitive wrapper operations and
conditional failure remain separate execution/coverage obligations. The
constructor gaps introduced by model semantics 7 remain required.
Each `birth_check` clause has its own minted declaration identity and
`BirthCheck` executable body, named `Locus::birth_checks::N` by clause order.
The predicate is evaluated once and the payload remains in its conditional
branch. These bodies are inline, with no fabricated native frame, and share
the existing call/default/effect summaries. They are not callable-name entries.
The editor retains their enclosing declaration dependency view.

The `birth_checks` relation records the executable body, typed locus owner,
inline closure name and clause ordinal. Validation requires exactly one
definition per birth-check body, its birth-phase binding, and scoped
`UnmodeledFailureDispatch` coverage for the possible conditional rejection.
The predicate is not modeled as an unconditional explicit `violate` or a
closure comparison. Its actual helper calls remain graph edges, so known
forbidden effects still violate before uncertainty is considered.

Resolved constructors of a locus with birth checks also require an
`UnmodeledInitialization` gap at the actual constructor witness. Shared
`BirthCheckInvocation` source obligations feed ordinary effects, canonical
coverage and foreign-call absorption. They do not invent an execution edge,
stack charge or scheduling/lifetime fact. The checker and lowering use the
same clause body identity. Actual constructor/lifecycle invocation, generated
wrapper work and whole birth/teardown behavior remain unfinished.


### Self lifecycle declarations and constructor entry

`AllocSummary.lifecycle_hooks` is produced in the declaration inventory, not
from lifecycle plans or emitted artifacts. Every locus has a row, including
one with no self lifecycle body. Its birth/run/drain/dissolve declarations
retain real declaration identities, stages, source spans, source-empty facts
and synthesized markers. Parent `accept`/`release` admission is a separate
invocation account. Synthesized empty run bodies remain visible to lowering
without becoming authored canonical executable bodies.

Declaration metadata and executable-body selection are separate. When a
checked seed declares a stdlib name, its body shadows the analysis copy's
executable row. The full declaration inventory still retains both source
universes: the merged lowering view contains the bundled clone and requires
its own hook identities, empty-body facts and synthesis markers. The
`own_rows` and `program_rows` projections retain only program-universe hook
metadata; retaining a bundled declaration must not restore its shadowed
body, calls or effects. Parent protocol declarations follow the same rule.

The analysis API also accepts unminted inputs, as its `FnKey` contract
permits. Their lifecycle owners retain the resolved name and source universe
with no declaration identity; hook effects remain visible. This case cannot
produce native lifecycle rows: lowering requires real owner and body
identities and refuses missing ones.

The program-alone `own_rows` projection removes foreign constructor bindings
and implicit body entries together with the source bodies it excludes.
`program_rows` retains these source facts for the canonical model's absorption
and invocation-context accounts. Neither projection derives facts from an
artifact or an emitted lifecycle plan.

`lifecycle_definitions` records each authored self hook with its typed locus
owner, stage and source-empty fact. Each nonempty self stage has a guarded
may-entry in `lifecycle_evaluations`, keyed by constructor ordinal and stage,
independently of authored call and argument/checkpoint ordinals. The guards
are `construction` for birth, `run_admission` for run and `teardown` for drain
and dissolve. Each entry preserves the actual constructor witness, original
source identity and repetition facts. The complete definition and
constructor-entry accounts are validated before judgment or evidence use.
Empty hooks have no fabricated entry when the native call is elided.

Effect and reachability consumers follow these entries and retain known
body/helper effects of all four self stages. These are possible source paths:
the graph does not prove that the guard succeeds or that a path is a feasible
runtime trace. They are not caller-frame edges for stack-cycle
or quantitative accounting. With the exact complete self-hook edge account,
qualitative source calls and effects are represented. Constructor invocation
context remains scoped `UnmodeledLifecycleContext` coverage of **costs**:
no successful admission, stack placement,
retained lifetime or generated-wrapper guarantee follows from the entry.
Run/drain/dissolve may-entries do not establish immediate execution. Known
violations in represented work still take precedence over context uncertainty.
Phase filters select source roots; transitive may-entries retain their own
stages and do not become members of the caller's phase.

Foreign self-stage entry uses a distinct absorption entry namespace and typed
lifecycle events; it is never encoded as an authored call or an initializer.
Interior hook stages retain source metadata, and coverage validation requires
invocation-context residue at the entry's user function. Numeric cost evidence
refuses unresolved lifecycle execution rather than charging an arbitrary
callee frame to its constructor. These facts do not establish complete
runtime refinement or physical resource ceilings.

The lowering view projects this source inventory through the existing source
correspondence. Native self-stage selection uses the real source declaration and
source-empty fact, including generic templates, and refuses missing or
mismatched rows. Self-stage emptiness is not independently rediscovered after
lowering rewrites. Initial construction, pinned-thread birth, cross-pool birth
and restart birth all consume the same hook selection. Lifecycle-plan ordering
continues to govern emission; the canonical model is not derived from a plan.

### Source lifecycle contexts

`Snapshot::demand_lifecycle_contexts` caches construction and ownership contexts
from the original bundle, placement, ownership and completed concrete flow
membership. Each source template retains all owner contributions, its
instantiating and queue domains, construction bound and source lifecycle
statement stage. `lifecycle_order` consumes these rows to derive obligations;
source context derivation does not read that plan. A source construction bound
counts occurrences and does not establish a simultaneous live population or a
physical storage ceiling. Unknown domains and recursive-owner cuts remain
explicit, with typed ownership gaps. Summed construction counts that exceed
the representation become unbounded rather than saturated. A representative
parent context preserves execution-domain distinctions; it is not a proof of
exact runtime ancestry. These source rows do not yet discharge the canonical model's
lifecycle invocation-context gaps.

The canonical model retains these source facts in
`Analyses.lifecycle_contexts`, including templates outside the arranged entity
horizon. Declaration bindings distinguish represented user declarations,
foreign declarations and unrepresented user declarations. A foreign source
identity is never joined numerically to a user entity or resolved into user
provenance. User locus entities retain their original declaration ID for this
join, separately from their display spelling and name-span provenance. The source-neutral identity, domain and construction-position
schema is shared with placement and planning in `hale_model::construction`.

This internal inventory is not yet an invocation/completeness relation and is
not consumed by a claim, a graph certificate or the topology artifact. An
absent inventory means unavailable, and an available inventory does not prove
runtime admission, exact ancestry, incarnation identity, safe reclaim or a
physical population/byte ceiling. Such judgments require their own scoped
coverage and evidence bindings before these rows can certify them.

A context retains the original constructor literal ID where one exists.
Implicit entry and binding templates invent no authored constructor site;
unknown field literals stay absent. Specialization keys identify declarations
and must not be parsed to reconstruct type arguments or target layouts.

Known generation markers retain named synthetic provenance. Generated JSON/API
offsets are never used as context witnesses into overlapping user source.


### Source default evaluation contexts

The `default_evaluation` family retains the ownership walk's original literal
and root-call identities while following the verified typing walk's omission
and evaluation-path rows. A literal in a params or argument default is evaluated
under its root caller's construction context. An ordinary free-function body
and an adapter binding have no caller locus. The same default declaration
can therefore have rows under a locus and under no locus, with distinct roots.
This context does not bind `self` reads: a locus parameter default has a
separate binding to the instance being initialized. Failure supervision and
accepting runtime instances also remain separate facts.
`None` for a source identity is unavailable; a source span is only a witness.
Overlapping generated witness spans do not merge distinct identified roots.

The source account also retains unavailable identities or default declarations,
missing call classifications, recursive
default-call cutoffs, coalesced repeated source contexts, parameter defaults
beyond a cross-pool post, and per-use
positions whose uses are not related to particular loci. Per-use candidates
include the no-locus context and cannot establish that a particular use occurs.
A missing context row does not prove no execution. Rows are source candidates,
not invocation counts or runtime occurrence IDs. The omission column's
classification gaps do not by themselves classify a primitive's effects. Its
callee identities describe user defaults only; a primitive or foreign callee
retains an unrepresented-default boundary rather than acquiring a user
declaration through a coincident numeric ID.

The snapshot caches these rows once. The checker and harness lowering
backstop read the same rows and possible cross-pool owner routes. A possible
post is enough to prohibit using the constructed value. It does not prove
that the post occurs: local and orphan alternatives still evaluate defaults
locally. Definite posts alone cut default expansion at the handoff boundary;
possible posts are retained separately and cannot erase those evaluations.
The locus-only
`OwnershipGraph::expansions` API projects the same source computation. The
producer reads the verified expression walk and ownership graph, without
reading a lifecycle plan or deriving an application model. This source
inventory establishes no admission, accepting runtime instance, failure
supervisor, lifetime, native frame or completeness capability. Parent
accept/release execution and its claim coverage remain separate required work.

### Parent protocol execution

The source execution account retains parent `accept` and `release`
declarations from the allocation summary's existing declaration walk. Each
row carries the real owner and callable identity, resolved child symbol,
source emptiness and synthesis status. Native hook selection reads these
facts through the merge correspondence; a missing inventory or mismatched
hook identity is a compiler refusal.

A contextual source edge retains both the triggering call/construction and
the nested constructor. Parent work from an inline default belongs to the
caller's triggering root. It is not an unconditional successor of the shared
default body. Ordinary free functions have no caller locus merely because a
locus called them. Construction context remains distinct from parameter
`self`, failure supervision and runtime ancestry.

Known parent bodies participate in source effect traversal. The account
retains admission/release opportunity, scheduling, frame and lifetime
uncertainty; it establishes no occurrence count, native caller frame,
physical retention bound or feasible temporal trace. The portable model
currently preserves this work as scoped initialization coverage at its root.
The guarded portable parent-protocol relations and their source-account
validation are described below. Runtime occurrence, admission, scheduling
and lifetime correspondence remain separate obligations; graph absence cannot
use this source account as proof of complete execution coverage.


### Parent protocol source contexts and guarded entry

Model semantics 12 adds `protocol_definitions`, `protocol_contexts` and
`protocol_evaluations`. These are derived from the shared `protocol_execution`
source family, never lifecycle plans or artifact reconstruction. Definitions
retain each authored accept/release body's real function and locus identity,
source stage, resolved child symbol, interface boundary and empty-body fact.
The source inventory also supplies native selection through the existing
lowering correspondence. Synthesized and source-empty bodies create no
invented execution.

A context preserves both the triggering caller root and the actual nested
constructor. Call roots are projected through original authored-site identity
to the model's grouped call ordinal; source-summary call indexes are not
portable ordinals. Construction and call namespaces remain distinct. A nested
constructor must be reachable through the root's initializer evaluations,
including omitted arguments and record fields; ordinary callees do not
inherit their caller's construction context. Parameter `self` remains separate.
Root and constructor witnesses and endpoint repetition facts are retained.
Intermediate default-path repetition remains unresolved.

Each context retains its resolved child symbol separately from the original
constructor template. `Cell_Int` does not become `Cell`: specialization
bindings come from the existing lifecycle source inventory, joined by the
original constructor identity and declaration binding. Nongeneric constructors
retain their already-resolved nominal target. No generated-name parsing or
lowering resurvey supplies this relation.

Each context has exactly its matching nonempty nominal parent entries.
`accept` is guarded by `admission_opportunity`; `release` by
`release_opportunity`. Validation checks definitions against source phases,
constructor ownership and source identity, initializer reachability, exact
entry sets and function-scoped `UnmodeledInitialization` coverage. This is a
source may-entry account. It does not prove successful admission, a runtime
occurrence/incarnation, callback scheduling, a native caller frame, retained
lifetime, population or physical bytes. The declared accepting parent is a
source candidate, not a proved runtime ancestry relation.

The canonical execution query and reachability engine follow these entries.
Known forbidden paths take precedence over unresolved invocation coverage.
Path certificates describe that graph proposition; absence certificates and
quantitative results still require adequate scope. Parent opportunities never
become ordinary authored calls or numeric frame charges. Interface matching,
foreign and unrepresented bodies and remaining source horizons keep their
scoped gaps rather than fabricated nominal edges.

Schema 1.35 exports these accounts and a `protocol_constructors` source-symbol
account. The latter binds resolved child specializations to real constructor
function/ordinal pairs independently of selected callback contexts. Admission checks their exact shapes,
references, constructor/root namespaces, initializer paths, repetition, guard,
coverage and matching entry sets directly over JSON. A document with authored
parent callbacks requires the new account; deleting a table or downgrading
its schema is refused. Older self-stage-only accounts retain their existing
supported horizon. Artifact admission checks consistency, not source
extraction or runtime correspondence. It cannot detect coordinated deletion
of all independent source facts; proof trust still includes extraction.
Fleet composition and topology rendering project the admitted guarded edges.
This account was introduced in analysis semantics 26. The twelve-part claims
upgrade remains broader than this execution account.

### Receiver confinement and declaration horizons

The shared allocation/execution summary produces receiver-confinement sites
from record declarations, independently of the executable-body horizon.
Its existing declaration-field producer also supplies canonical stdlib record
shapes when those bodies are absent. Each source universe keeps its nominal
shadowing: local aliases, enums, loci or other type declarations cannot inherit
a foreign record's shape by name. Private stdlib aliases do not become
program-scope aliases through this account.

Record fields retain their template-parameter identities. A type parameter
of a function, locus or record does not become a nominal data record because
their spellings match. Nested writes through an unresolved parameter
or managed handle retain a confinement gap. Replacing a local value record's
own field does not mutate a locus reached through that field.

These facts supply purity through the same summary as calls, defaults,
lifecycle and parent execution. They import no executable rows, defaults or
callee bindings, and cannot close an unknown callback or missing execution
account. Analysis semantics 30 binds this classification. This change adds no
portable model table and proves no runtime lifetime or retained-resource bound.

### Self lifecycle source closure and runtime context (model semantics 13)

The source summary checks every constructor against its lifecycle declaration
inventory. Every nonempty authored self stage needs its exact target body,
constructor ordinal, source identity, span and repetition flags. Missing
inventories, bodies or edges keep categorical effects unknown. This account
uses the original source facts; it reads no lowering plan or artifact.

The model separately validates that all such definitions and entries are
present. `UnmodeledLifecycleContext` has exactly the `costs` mask and must bind
a represented constructor/hook source entry. Foreign entries retain their
own namespace, body stages and source witnesses. A missing context row, a
detached row, or an altered edge fails validation. Topology schema 1.36 and
analysis semantics 31 distinguish this account from earlier evidence.

A pure represented hook can therefore satisfy a source effect prohibition;
known forbidden work inside any potential stage remains a violation. Unknown
calls, structural failure dispatch, automatic assertion epochs and parent
protocol gaps retain their independent call/effect coverage. This closes no
runtime admission, completion, native frame, lifetime, incarnation, retained
resource, or liveness obligation. Numeric budgets still refuse the unresolved
lifecycle context. Potential lifecycle work remains separate from synchronous
caller-frame traversal.

### Constructor predicate source entries

Birth checks and automatic closure assertion operands participate in source
reachability from each construction that may evaluate them. An operand that
performs a forbidden effect produces a violation with the predicate and helper
path. The predicate entry keeps its constructor ordinal, clause kind, epoch and
loop context in a namespace separate from calls and explicit checkpoints.

These are possible source entries. They establish neither successful admission,
execution order, a native stack frame nor eventual evaluation. Conditional
failure dispatch, recovery, scheduling and lifetime coverage remain explicit;
a pure operand alone does not certify a constructor or its quantitative budget.
Imported and standard-library predicate bodies use the same execution facts.
Topology schema 1.37 includes `relations.constructor_checks`; admission requires
exactly the represented constructors' birth predicates and automatic assertions.
Model semantics 14 and analysis semantics 32 distinguish this account from older
evidence, including source-bound reachability proof replay.

Foreign predicate metadata retains conditional failure uncertainty at the actual
interior vertex. A valid `avoiding` gate can exclude it. Known operand paths
remain traversable and a concrete forbidden effect wins over uncertainty.
Each enclosing source entry separately retains runtime cost coverage; compact
artifacts conservatively project interior failure uncertainty to that entry.
The compact projection therefore cannot recover the source model's interior
barrier precision. Native scheduling and failure dispatch remain unproved.
