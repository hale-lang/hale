# The Application Model

Alongside its code-generation IR (basic blocks, instructions, and registers), the Hale compiler frontend derives an **`ApplicationModel`**: a canonical relational graph of the program's loci, ownership, call sites, bus edges, placements, effects and costs. Claims and topology export read this model. Lowering views and plans consume projections of the same authoritative source families.

```hale
type Held = Holder;

main locus App {
    params {
        h: Held = Holder { };          // arranged as App.h (realizing Holder)
        r: Router = RouterV1 { };      // arranged as App.r (realizing RouterV1)
        workers: Worker = Worker { };
    }
    placement {
        h: pinned;                     // App.h and its subtree run on pinned:App.h
        workers: pinned(cores = 0..4, replicas = 4);
    }
}
```

Inspect the derived model for any seed or file using the CLI:

```sh
hale model dump app.hl
```

## Source construction contexts

The internal model retains source construction contexts in
`Analyses.lifecycle_contexts`: templates, owner and domain contributions,
constructor-site identities, and explicit ownership gaps. Planning reads the
same cached source family. User declarations bind through their original IDs;
foreign identities and provenance remain in their own source universe. Known
generated sites retain named synthetic provenance rather than borrowing a user
source line.

These rows describe possible source constructions. Admission, completion,
incarnation identity and exact runtime ancestry need additional execution
coverage. Existing lifecycle invocation gaps remain visible. The inventory is
currently available for internal inspection; topology artifacts and graph
certificates retain their existing scope. Specialization keys identify concrete
declarations, while layout bounds require structural type and target facts.

## Single Canonical Derivation (`I7`)

A semantic fact about a program—which functions a method calls, which handlers a bus publish reaches, or which supervisor owns a child instance—has one authoritative producer in the compiler:

- **Shared in-process truth**: `hale check`'s claim verdict and the topology artifact read the same model. Lowering views and dispatch plans read shared derived source families; each fact has one producer across these projections.
- **Eight distinct artifacts**: The compiler keeps eight representations strictly separate:
  1. **Source** (`.hl` files)
  2. **Plan** (resolved AST and types)
  3. **Model** (`ApplicationModel` relational tables)
  4. **Lowered law** (`ClaimIr`)
  5. **Evidence** (witness paths and counterexamples)
  6. **Artifact** (`ComponentModel` topology JSON)
  7. **Lowering plan** (`DispatchPlan`)
  8. **Execution evidence** (runtime telemetry and replay traces)
- **The Graph Registry (`spec/registry.md`)**: Every semantic fact family (ownership, bus graph, dispatch, placement, effects, allocation, and lifecycle order) is registered in `spec/registry.md` with its single authoritative producer, permitted legacy producers, consumers, and tests. Compiler CI checks registered producer locations, legacy-call counts, and reviewed classifications for bare `Debug` renderings.

## Explicit Horizons: `Hole`s, Capabilities, and Verdicts (`I8`)

A static graph that only records discovered edges cannot distinguish *"no edge exists"* from *"the analysis could not see past an indirect call."* To prevent unresolvable code from silently passing negative claims (`forbid reaches(...)` or `@effects(none: { ... })`), `ApplicationModel` records every analysis boundary as a typed **`Hole`** row:

```rust
Hole {
    at:     Function(cache_lookup),
    kind:   IndirectCall,
    hides:  CALLS | EFFECTS | COSTS,
    reason: "call through `f`",
}
```

Conversely, the model records positive **`Capability`** rows to certify which fact families are closed and complete, rather than inferring completeness from the absence of warnings.

### The Four Verification Verdicts

Because the model tracks both edges and `Hole`s, every claim or contract check produces one of four verdicts:

| Verdict | Build Outcome | Meaning & Action |
|---|---|---|
| `holds` | Pass | The property is proved over a complete, hole-free subgraph. |
| `violated` | **Error** | Disproved; the compiler prints a concrete witness counterexample path. Fix the program edge or update the law. |
| `uncertified` | **Error** | The statement is well-formed, but traversal encountered a `Hole` hiding `CALLS`, `EFFECTS`, or `COSTS`. Replace the opaque construct (e.g., function pointer or out-of-arrangement birth) so static analysis can close the graph. |
| `invalid` | **Error** | The claim itself is malformed (such as referencing an unknown identifier or an empty group without `may_be_empty`). |

## Whole-Program Properties

Holding the entire assembled program in `ApplicationModel` enables checks that per-function analysis cannot express:

| Property | Annotation / Claim | How `ApplicationModel` Verifies It |
|---|---|---|
| **Cross-bus transitive effects** | `@effects(none: { fs })`, `forbid reaches(A, effects(syscall))` | Follows edges across `Topic <- payload;` into every matching subscriber handler in the closed topic graph. |
| **Backward causal influence** | `@effects(depends: { TopicA, TopicB })` | Walks the message and call graph backward to verify the complete set of upstream topics that can transitively trigger or influence a locus. |
| **Fan-out amplification** | `@budget(fanout = 8)` | Counts the maximum transitive subscriber deliveries caused by a single call across runtime replica registrations and routing keys. |

## Quantitative Graph Semantics

Quantitative checks such as `@budget(fanout = N)` require counting executions rather than set reachability. `ApplicationModel` enforces two rules to keep quantitative counts exact:

### 1. Site-Grained Rows vs. Reachability Sets

Call and publish tables store one row **per syntactic call or publish site**, not one row per `(caller, callee)` pair:

- If a handler calls a publishing helper twice, the model records two call rows and counts two publishes.
- If a locus subscribing to `Quote` is placed with `pinned(..., replicas = 3)`, the model records three runtime subscriber registrations.
- If a publish targets a `keyed_by` topic with a known key, the model counts only the subscriptions matching that key rather than the union of all subscribers on the topic.

### 2. End-to-End Scenario Aggregation

When computing worst-case maxima over branching keys or polymorphic interface conformers, the analyzer evaluates each coherent **scenario** (a single routing key or single concrete interface conformer) end-to-end before taking the maximum:

```text
max(immediate) + union(downstream)    !=   max(immediate + downstream)
sum over sites of max over branches   !=   max over branches of sum
```

Aggregating early (left-hand side) would combine the immediate cost of one routing key with the downstream cost of a different key, or sum two mutually exclusive interface conformers as if a single dispatch invoked both. Evaluating complete scenarios first (right-hand side) keeps bounds tight.

> [!NOTE]
> **Function Pointer Conservatism**
> Calls through function-pointer values (`fn(T) -> U`) can target any function of matching signature passed as a value in the program. The model currently records each candidate as a call site at that position, resulting in a conservative over-count (never an under-count).

## Instance Arrangement & Placement Projection

The model's instance tables—recording which locus instances exist, their parent-child ownership tree, their assigned thread domain, and their CPU core affinity (`affined_to`)—are projected directly from the compiler's placement table:

- **Type aliases and interfaces**: Aliased fields (`h: Held = Holder { }`) resolve to the concrete locus (`Holder`), and interface-typed fields (`r: Router = RouterV1 { }`) resolve to the concrete implementation constructed in the initializer (`RouterV1`).
- **Replica paths**: Replicated fields expand to indexed instance paths (`App.workers[0]` .. `App.workers[3]`), and any children nested beneath a replica include the replica index in their path.
- **CPU affinity (`affined_to`)**: Core sets (`cores = 0..4`, NUMA nodes, L3 domains) are recorded as an `affined_to` attribute on each pinned replica thread or cooperative pool worker.
- **Arrangement boundaries**:
  - **Standard-library loci**: Fields of stdlib types (such as `std::io::udp::Reader`) are placed by the compiler in the thread table, but omitted from user declaration arrangement tables.
  - **Imported `main locus` declarations**: An imported library's `main locus` is never treated as the program's root arrangement.
  - **Out-of-arrangement births**: Any locus instantiated directly inside `fn main()` outside the root `main locus` is recorded as a dynamic birth `Hole`.
  - **API surfaces and hubs**: Generated transport adapters for [`api::serve`](./services/api.md) and hubs appear in the model's `dispatch_plan` report with their I/O pool placement, while dynamically accepted client connections carry dynamic-placement holes.

> [!IMPORTANT]
> **Instance Identity Across Builds (`path` vs. `id`)**
> Numeric instance IDs are assigned in lexicographic path order, so adding or renaming a field renumbers subsequent IDs. Always join or diff instances across builds by their canonical **instance path** (e.g., `App.workers[2]`), never by numeric ID. Instance ordering does not affect the program's `shape_hash` or runtime observation IDs.

## Flow classification and retained children

The in-process model records each locus declaration as a concrete flow,
a concrete resident, or a generic template whose specializations need
their own classification. Flow rows keep the release clauses that
classify them, including the concrete specialization of a generic
owner. The model, lifecycle planner and code generator share the same
completed inventory. `hale model diff` reports classification changes in
the `flow` contract facet.

This classification is type-wide. A release clause on an unused owner
can classify a child as a flow; it does not identify which owner accepts
a particular child. It also does not prove that a run terminates or
that live children or retained bytes are bounded. Those questions need
birth, ownership and retention facts beyond the static arrangement.

## In-Process `ApplicationModel` vs. Out-of-Process Artifacts

`ApplicationModel` exists only in memory inside the compiler process and is constructed exclusively by checking source code.

The shared declaration inventory is distinct from executable-body selection.
Checking a stdlib source seed keeps its own executable bodies while retaining
the bundled clone's declaration identities for lowering. That metadata does
not add the shadowed clone's calls or effects to the program model.

Out-of-process tools—such as `hale fleet check` when composing multiple binaries—read the **topology artifact**, which serializes a compact `ComponentModel` projection under an FNV-1a `shape_hash` alongside lowered laws, capabilities, and adequacy summaries. The artifact is intentionally a one-way projection: out-of-process consumers run the shared reachability engine over `ComponentModel` and verify artifact digests rather than reconstructing a full compiler `ApplicationModel`. The current exporter uses schema 1.37 and artifact semantics 3. It includes non-default flow classifications and their policy adequacy, default evaluations, and guarded self/parent lifecycle accounts. Model semantics 14 and analysis semantics 32 are separate version domains; see the [artifact format reference](./claims.md#topology-artifact-format).

### Further Reading

- Formal specification of sorts, relations, holes, capabilities, and identity rules: [`spec/model.md`](https://github.com/hale-lang/hale/blob/main/spec/model.md)
- Authoritative producer/consumer registry: [`spec/registry.md`](https://github.com/hale-lang/hale/blob/main/spec/registry.md)
- Rust crate API reference: [`/api/hale_model`](api/hale_model/index.html)


### Recovery observations

Authored failure handlers participate in the shared call and effect analysis.
Their helper calls and publish sites are visible to existing claims, and
unresolved calls keep scoped holes. Handlers are identified by declaration,
independent of display spelling. Duplicate handlers remain checker errors. Locus aliases resolve to the actual template; calls through function-valued
fields retain dispatch through aliases and generic field types. Symbolic receiver
types remain explicit unknowns. Repeated failure delivery
is conservatively unbounded; a retry literal alone does not bound how often
the handler can run. Recovery operations retain an explicit gap for implicit
lifecycle calls, sends and allocations until that execution can be modeled.

The model also records helper-call operands at the checker's existing callee
resolution point. Given an invocation, parameter substitution can carry the
arranged child instance through a helper or an immutable locus alias. A restarted
incarnation or a dynamic instance still needs its own identity account. A field's
current value, a mutable alias, an omitted argument or missing call context
remains unknown. This does not establish which handler runs or which branch
executes. Default calls that may reach recovery retain execution gaps, including
through another helper; unrelated functions keep their own scope.

Model dumps also show conditional failure-entry contexts. Fresh field
constructions name their original supervising parent and canonical handler.
Helper-call substitution can then carry that child through parameters. Every
root construction and initializer alternative is retained, even when the static
paths have the same type and placement. Moving a parent retains its children's
original supervisor; a held child does not gain its new holder as supervisor.
Ambiguous source aliases remain unknown rather than choosing one static path.

These contexts are conditional on the represented construction raising a
failure. They do not prove a branch runs, a runtime instance is live, params
settlement has completed, or a handler is admitted. Root contexts, unconstructed
catalog entries, unmapped sources and generic specializations remain explicit
gaps. A known unhandled route is distinct from unknown routing. The inventory
changes no existing claim verdict and is not exported as a recovery proof.

`hale model dump` also follows those conditional frames through helper calls
to show the first represented recovery target for each arranged instance.
Its source path retains the selected child and supervisor across calls, so two
siblings of the same type remain distinct. Missing field state, argument
bindings and implicit execution remain visible as incomplete inspection.
The dump limits each search to 512 states and reports reaching that limit.
This is a source may-path: it does not establish that a failure or branch occurs,
that the target is live, or that recovery stays inside a runtime boundary.
It lists neither every target nor the full effects of restarting a target.

Model dumps show the typed recovery operations inside handlers, methods and
helpers. Their operands distinguish the current locus, immutable parameters
by position, field paths and unresolved values. A local that shadows a
handler parameter is a different binding. An alias saved before a field is
replaced stays unresolved; it does not become a lookup of the new field.

These observations describe expressions the compiler typed. They do not
prove which instance a recovery affects, whether a conditional branch runs,
or whether a failure stays inside an ownership boundary. Evaluated defaults
retain separate invocation witnesses while their execution contexts remain
unresolved. The compiler preserves unavailable and unanchored observations
for later analysis. This inventory is not yet a containment claim or part of
the topology artifact.


### Constructor defaults

Omitting a field evaluates its default. The model represents that execution
with a constructor edge to the actual default body, followed by the body's
helper calls. Supplying the field skips that body. Locus parameter defaults
belong to their locus; data-field defaults belong to their type. A record
construction introduces no locus, spawn or lifecycle.

The `program` group includes executable data defaults declared in the main
seed. An imported seed's defaults belong to its `lib::*` selection; a selected
caller can also reach them through construction. Helper barriers such as
`avoiding` retain their ordinary meaning along these paths. Effects and
per-call budgets use the same execution facts. This covers constructor
defaults; implicit lifecycle execution remains separate coverage work.


Named calls also carry their supplied argument count. An omitted argument has
its own edge to a default body owned by the declaring function. The ordinary
edge enters the function body separately, so an explicit argument can skip a
printing default while preserving a pure body. Imported defaults keep the
actual callable owner, and graph witnesses label these evaluations as
`defaults parameter`. The same facts feed effects, budgets and fleet/diagram
projections.


### Explicit closure checkpoints

Assertions have their own executable bodies under their actual locus owner.
A checkpoint evaluates the explicit assertions in declaration order, and graph
witnesses label each edge `checks closure`. Helpers remain separate vertices,
so graph barriers keep their scope. Diagrams and fleet components retain the
same edges. Definition, checkpoint and evaluation tables are checked as one
account; they cannot omit an assertion while keeping a closed checkpoint.

The model retains a comparison's unresolved failure branch as a typed gap.
It also distinguishes an argument default's declared owner from the caller
whose closures a contextual checkpoint would evaluate. That unresolved context
refuses certification. A construction with automatic birth, dissolve, tick or
duration assertions also refuses closed certification while its epoch
scheduling, lifetime and failure context are unresolved. This gap stays at
the construction, including constructions inside evaluated defaults. These
source facts do not yet model automatic epoch invocation or establish runtime
lifecycle/refinement properties.


### Phase sources

A source phase can contain an authored hook and automatic assertion bodies.
A birth assertion belongs to `birth` even when no `birth()` hook was written;
tick and duration assertions supply their named phases. Explicit assertions
keep the phase of their checkpoint caller. Phase contracts and graph claims
read this shared membership, including inside modules and imported seeds.

Within this named-body account, a lifecycle phase with no represented roots
is vacuous. Birth-check clauses have distinct executable identities and
belong to `birth`, including their predicates and conditional payloads.
Their possible rejection retains scoped failure-dispatch uncertainty;
construction retains a separate invocation-context gap. A missing report
over real phase bodies is Uncertified. The source phase relation does not establish lifecycle
invocation, scheduling or generated wrapper behavior; their coverage remains
separate.


### Lifecycle entries

Self lifecycle definitions retain the source body identity and its locus,
stage and empty-body fact. A constructor's possible birth, run, drain and
dissolve entries form a separate relation, so their effects are visible without
borrowing an authored call ordinal. Every entry has a guard: `construction`,
`run_admission` or `teardown`. A graph path does not prove that this guard
succeeds or that the path is a feasible runtime trace.
With every declared self-stage body and constructor entry represented,
qualitative source calls and effects are closed. Admission, native stack
placement, scheduling and retained lifetime keep a separate cost-context gap.
Unknown execution inside a hook retains its ordinary source gaps.
The same source inventory selects the native self-stage body through the lowering
correspondence. A missing or mismatched declaration is a compiler refusal.

An entry does not establish its native stack, successful admission or retained
lifetime. Those questions retain scoped uncertainty; run and teardown
stages keep their own context obligations. Foreign interiors preserve typed lifecycle
entries too. Graph path certificates can describe the known body/helper path,
while absence and numeric resource proofs still require adequate coverage.

Artifacts containing authored self hooks require schema 1.34. Re-export older
artifacts from source before checking or rendering them. Changing the version
field or removing the lifecycle tables cannot supply the missing execution
account.


### Defaults and caller context

A default is evaluated where it is used. The shared source context inventory
keeps both the original default literal and the call or construction that
starts its evaluation. It distinguishes a caller construction context from a free-function
body or binding that executes under no locus. Two generated expressions with
the same witness span retain distinct source identities. A parameter default's
`self` binding to the instance being initialized is separate from this caller
context.

These are source facts for execution analysis and the lowering backstop.
Missing call classification, recursive default calls, cross-pool handoffs and
unknown uses remain explicit gaps. The inventory does not prove parent
admission, runtime ancestry or absence of execution when a row is missing.


### Parent callbacks

Parent `accept` and `release` bodies have their own source execution account.
A callback caused by a child in an omitted default is attached to the actual
caller root, while the model keeps the nested constructor's separate identity.
An ordinary free function does not acquire its caller's locus context.

The graph can show a path into a known nominal parent callback. Diagnostics
name its admission or release opportunity and point at the triggering root;
a nested constructor has a second witness. These are possible source paths,
not proof of a successful admission or a feasible scheduled runtime trace.
Known forbidden work violates a graph prohibition. Remaining invocation,
frame and lifetime uncertainty still prevents an absence certificate.

A generic child's resolved symbol and its template remain separate. The
constructor symbol account links a specialization such as `Cell_Int` to its
actual source constructor; no spelling-prefix guess supplies that binding.

Artifacts containing parent callbacks require schema 1.35 or later and their complete
portable definition/constructor/context/entry account. Missing or inconsistent tables
are refused. Topology views and fleet composition retain the admitted edges.
Generic, interface, foreign and unrepresented execution boundaries still need
adequate source and runtime correspondence; this account establishes no
resident population or physical-memory bound.
