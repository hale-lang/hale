# The Application Model

Alongside its code-generation IR (basic blocks, instructions, and registers), the Hale compiler frontend derives an **`ApplicationModel`**: a canonical relational graph of what the program *is*—its loci, ownership tree, call sites, bus pub/sub edges, thread placements, effect classes, and quantitative costs. Every static analysis pass, `claims { }` verifier, bus dispatch planner (`DispatchPlan`), and topology artifact exporter queries this single model rather than re-walking the AST.

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

## Single Canonical Derivation (`I7`)

A semantic fact about a program—which functions a method calls, which handlers a bus publish reaches, or which supervisor owns a child instance—has one authoritative producer in the compiler:

- **Shared in-process truth**: `hale check`'s verdict on a `claims { }` law, the lowered `DispatchPlan` used by codegen, and the verdict written to the topology artifact all read the same `ApplicationModel` tables in memory.
- **Eight distinct artifacts**: The compiler keeps eight representations strictly separate:
  1. **Source** (`.hl` files)
  2. **Plan** (resolved AST and types)
  3. **Model** (`ApplicationModel` relational tables)
  4. **Lowered law** (`ClaimIr`)
  5. **Evidence** (witness paths and counterexamples)
  6. **Artifact** (`ComponentModel` topology JSON)
  7. **Lowering plan** (`DispatchPlan`)
  8. **Execution evidence** (runtime telemetry and replay traces)
- **The Graph Registry (`spec/registry.md`)**: Every semantic fact family (ownership, bus graph, dispatch, placement, effects, allocation, and lifecycle order) is registered in `spec/registry.md` with its single authoritative producer, permitted legacy producers, consumers, and tests. Compiler CI fails if any crate introduces an unregistered derivation walk, adds a caller to a legacy producer, or formats model rows via raw `Debug` output.

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

## In-Process `ApplicationModel` vs. Out-of-Process Artifacts

`ApplicationModel` exists only in memory inside the compiler process and is constructed exclusively by checking source code.

Out-of-process tools—such as `hale fleet check` when composing multiple binaries—read the **topology artifact**, which serializes a compact `ComponentModel` projection under a cryptographic `shape_hash` alongside lowered laws, capabilities, and adequacy summaries. The artifact is intentionally a one-way projection: out-of-process consumers run the shared reachability engine over `ComponentModel` and verify artifact digests rather than reconstructing a full compiler `ApplicationModel`.

### Further Reading

- Formal specification of sorts, relations, holes, capabilities, and identity rules: [`spec/model.md`](https://github.com/hale-lang/hale/blob/main/spec/model.md)
- Authoritative producer/consumer registry: [`spec/registry.md`](https://github.com/hale-lang/hale/blob/main/spec/registry.md)
- Rust crate API reference: [`/api/hale_model`](api/hale_model/index.html)
