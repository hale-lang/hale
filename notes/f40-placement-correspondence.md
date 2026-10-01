# F.40 phase 3, P1 — the placement correspondence

**What this is.** The correspondence P1 requires before its producer is built (hale-lang/hale#1212; the phase-3 plan's § 3 Line P, step P1, and § 6). It does four things. It fixes the schema of the one placement table. It reads each of the eight legacy producers the registry lists under `placement`, and states which answers the table keeps. It classifies every known divergence and gives its spec witness and the test that will pin it. Last, it orders the consumer switches. The decisions it applies are the wave-2 decisions § P1: instance identity includes field path and replica; budgets count the resource; and a runtime test checks the receiving thread of the nested off-thread subscriber.

The step is read-only on code. It was measured at `main` 13e5f352 (origin/main after #1293), and line numbers refer to that tree. One thing was run: the existing shadow, `cargo test --release -p hale-types --test shadow_placement`, which passes (21 classified rows, none unexplained, none stale). Everything else is **by reading**, and is marked so wherever a claim is about execution rather than code. Running a probe program through `hale` was not available in this session. § 4's test is what settles the execution claims.

**Abbreviations.** `chk` is `crates/hale-types/src/check.rs`; `bg` `crates/hale-types/src/bus_graph.rs`; `og` `crates/hale-types/src/ownership_graph.rs`; `mb` `crates/hale-types/src/model_builder.rs`; `rb` `crates/hale-types/src/resource_budget.rs`; `ent` `crates/hale-types/src/entry.rs`; `res` `crates/hale-types/src/resolved.rs`; `si` `crates/hale-types/src/sync_inference.rs`; `ds` `crates/hale-syntax/src/desugar.rs`; `cg` `crates/hale-codegen/src/codegen.rs`; `inst` `crates/hale-codegen/src/locus/instantiation.rs`; `brt` `crates/hale-codegen/src/bus/runtime.rs`; `dep` `crates/hale-codegen/src/deployment.rs`; `A` `crates/hale-codegen/runtime/lotus_arena.c`; `sem` `spec/semantics.md`; `rt` `spec/runtime.md`; `ty` `spec/types.md`; `dec` `spec/decisions.md`.

**Classes.** Each divergence gets one class. The class describes the change a consumer sees when it stops reading its legacy producer and reads the table.

- **known old bug.** The legacy answer contradicts the spec, and the table answers what the spec says. Every known old bug listed here is an **approved correction** under the wave-2 decisions, either directly (the shadow fixture's rows, the budget's accounting) or as the same principle reaching another consumer. Each lands as its own J commit with wording-pinned tests.
- **intentional correction.** The legacy answer was a deliberate stance that a decision now changes. It also lands as a J commit.
- **unresolved disagreement.** The right answer is not decided. It is listed in § 7 with a recommendation, and its consumer does not switch until the driver decides.
- **regression.** The table answers differently from a legacy producer, and no row here explains it. That is a producer bug. It stops the step and is never classified away.

Where a row says **agreement**, the table and the legacy producer give the same answer over everything the shadow sees, and the switch is an S commit.

## 1. The table

### Schema

```rust
/// One per snapshot: `Snapshot::demand_placement`, a new FAMILIES entry
/// "placement". The producer is a pure function over a bundle, as
/// `entry::entry_row` is, so a bundle no snapshot holds (sync
/// inference's single-program bundle, `lib.rs:440-454`) builds its own.
pub fn placement_table(bundle: &Bundle, top: &TopScope, entry: &EntryRow) -> PlacementTable;

pub struct PlacementTable {
    pub root: Option<RootRow>,             // None: lowering deploys no main locus
    pub domains: Vec<Domain>,              // indexed by DomainId
    pub instances: BTreeMap<InstanceKey, InstanceRow>,
    pub dynamic: Vec<DynamicSite>,         // loci instantiated outside the static tower
    pub holes: Vec<Hole>,                  // what the producer could not decide, with its policy
}

/// Instance identity: the deployed root, the field path from it, the replica.
pub struct InstanceKey {
    pub root: SiteId,                      // the main locus declaration lowering deploys
    pub path: Vec<String>,                 // field names from the root; [] is the root itself
    pub replica: Option<u32>,              // Some(i) on a `replicas = K > 1` field and on every row nested under it
}

pub struct RootRow {
    pub decl: MainLocus,                   // EntryRow::lowering_root, never EntryRow::entry
    pub is_entry: bool,                    // false when lowering deploys a module-nested main (NoEntry::OnlyModuleNested)
    pub instantiations: Vec<(SiteId, Bound)>, // each root literal and how often it can be live
}

pub struct InstanceRow {
    pub realizes: DeclRef,                 // the declaration actually built (an override literal's, not the field's declared type)
    pub literal: Option<SiteId>,           // the literal that builds it (default or override); None = a hole
    pub owner: Option<InstanceKey>,        // None only for the root
    pub domain: DomainId,
    pub decided_by: Decision,
    pub owner_relative: OwnerRelative,     // SameAsOwner | OffOwner
}

pub struct DeclRef {
    pub site: SiteId,                      // the locus declaration; for a monomorph, the template's
    pub args: Vec<Ty>,                     // the substitution; empty unless generic
    pub lowered: String,                   // the name lowering keys on (`__StdIoTcpListener`, `Cache_Int_String`)
}

pub enum Decision {
    Entry { block: SiteId, entry: SiteId },// the root's `placement { }` entry for this field
    Binding { entry: SiteId },             // an adapter locus inline in `bindings { }`: pinned-equivalent (cg:10494-10508)
    Inherited { from: InstanceKey },       // nested: the owner's domain (sem:3511-3542, rt:364-371)
    Default,                               // a root field with no entry: pool main (sem:3544-3552)
}

pub enum DomainKind {
    Main,                                  // the program's main thread; adds no thread
    Pool { name: String, async_io: bool, affinity: Option<CoreSet> }, // one worker per name
    Pinned { anchor: InstanceKey, affinity: Option<CoreSet>, numa_node: Option<i64> }, // one thread per anchor (per replica)
}

pub struct DynamicSite {                   // a locus literal in a method body, `accept`ed child, let-bound locus
    pub literal: SiteId,
    pub enclosing: DeclRef,
    pub domains: BTreeSet<DomainId>,       // the domains of the enclosing declaration's static instances; empty = unknown
    pub bound: Bound,                      // Once | AtMost(n) | Unbounded(reason)
}
```

### Invariants

1. **One row per static instance.** The static tower is the root's params fields and, recursively, their params fields, each with the literal that built it. Keys are unique. Replica rows are exactly `0..K` for a `replicas = K > 1` entry, and every row nested under replica `i` carries `Some(i)`.
2. **Nested rows inherit.** A row's domain is its owner's domain unless the row is decided by an `Entry`, which only a root field can be (rule 1, sem:3216), or by a `Binding`.
3. **Pinned domains are per instance.** Two rows share a pinned domain only if one is nested under the other, so each replica is its own domain. Pool domains are per name: every row on pool `X` shares one domain, and that domain carries at most one affinity (rule 16, sem:3400).
4. **The root is `lowering_root`, never `entry`.** The table describes what lowering deploys. Until L4 has lowering read the entry, that root can be a module-nested `main` that is not the entry (`ent:96-113`, `ent:185`). `RootRow::is_entry` records the difference, so a consumer bound to the entry (rule 9's closed world, `--env`, `--matrix`) can tell. An imported `main` is never the root: a seed whose only `main` is imported has an empty table.
5. **Every root entry decides exactly one field row** (rule 18, sem:3438). An entry that decides none is a hole with a stated policy. It is never dropped silently.
6. **Unknown is a hole, not a default** (registry, `placement` § Missing data). A field whose realized declaration cannot be resolved still gets its domain from its owner or its entry. Only `realizes` holds the hole.
7. **F.38** (dec:3219). No column of the table changes who receives a message. Domains decide threads and routes, never delivery sets.

### What each legacy question becomes

| consumer | the question | the table's answer |
|---|---|---|
| F.31 single-thread rule, sync inference | the pool of a locus type (`chk:3701`) | per instance: `domain`; per type: the set of its instances' domains |
| F.31 receiver | where `self.f` runs relative to its owner (`chk:4971`) | `owner_relative` of the row at `owner.path + [f]` |
| pinned-in-a-loop, entry consumed, instance aliasing | the root's pinned entries; entry ↔ literal; holders' pools | `Entry` decisions of root rows; `literal`; `domain` |
| bus graph, bounded subscriber | per-type `SameThread` / `CrossPool` / `Pinned` (`bg:621`) | per subscriber or publisher type: the set of its instances' domains; *same thread* iff that set is `{Main}` |
| ownership graph | per-type placement compared owner to enclosing (`og:746`) | the domains of the owner instance and of every enclosing instance |
| model `PlacedIn` | per instance, with replicas (`mb:2997`) | the rows, projected onto `LocusInstance` / `PlacedIn` / `Owns` with the model's domain names |
| desugar | is `(owner, field)` off the owner's thread (`ds:1163`) | `owner_relative == OffOwner` |
| `collect_main_placement`, `DeploymentPlan` | main's per-field schedule class, pools, affinity, async_io, replicas, pinned types (`cg:10295`, `dep:33`) | the lowering view of the root rows and domains; the type sets from `realizes.lowered` |
| resource budget | threads and pools (`rb:152`) | threads = pinned domains × root bound + adapter bindings; pools = non-main pool domains |

## 2. The legacy producers, one by one

### 2.1 The checker: `compute_pool_of_locus_type` and `enclosing_field_placement`

**The question.** Which pool a locus type runs in (`chk:3701-3766`), and, at a receiver `self.f.m()`, which pool the field instance runs in relative to its owner (`chk:4971-4981`, read at `chk:4792-4799`).

**What it computes.** The map is seeded from `entry.lowering_root` (`chk:3711`). The root type maps to `Cooperative("main")` (`chk:3734-3737`). Each root params field maps to its entry or to `main` (`chk:3743-3749`), keyed by the field's written type through `type_expr_locus_name` (`chk:4546-4558`). That function returns `None` for any path with more than one segment, and for a name that is not a `TopSymbol::Locus`. Nested types inherit through `walk_nested_loci` (`chk:4563-4594`) over `LocusInfo` params, whose resolved `Ty` carries mangled stdlib names, so a nested stdlib locus such as `__StdBytesBytesBuilder` does get a row. The map is first-wins per type (`chk:3753`, `chk:4588`). A pinned entry maps to `PoolId::Pinned(field_name)` (`chk:4537-4539`), which every replica of the field shares. The caller's pool is the enclosing **type's** row (`chk:3913`). The callee's pool is owner-relative: the enclosing locus's own entry for the field, or else the caller's pool.

**The answers the table keeps.** The F.31 diagnostic compares the caller's domain with the receiver's domain. Placement entries exist only on the root (rule 1), so for every enclosing locus but the root the receiver co-locates and the check is silent. The table keeps that, since `owner_relative` is `SameAsOwner` off the root. The diagnostic's display (`PoolId::display`, `chk:3671-3680`) is a projection of the domain: `cooperative(pool = X)` or `pinned (at `f`)`.

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| K-1 (fixture F-Q, 6 rows) | a root field typed by a qualified path (`std::io::tcp::Listener`, `std::http::Server`) | no row (`chk:4550`), so F.31 cannot see the field | a row with the entry's domain, `realizes` = the mangled declaration | known old bug → approved correction | rule 3 (sem:3223), a stdlib locus is a locus; ty:1112-1137, the pool belongs to the instance | `placement.rs` · `a_qualified_locus_field_is_placed`: `self.listener.m()` from the root with the field on `cooperative(pool = io)` draws the F.31 error, wording pinned. Grepping `self.<f>.<m>(` over the five files behind the six rows finds no match, so no corpus program becomes refused (the shadow confirms) |
| K-2 (fixture F-R, 1 row) | the only `main` is `__lib_App` (imported) | none: the map is empty | none: the table is empty | intentional correction (approved; landed with E0, #1293) | `ent:1-31`; sem:3411-3437 (rule 17's text on the imported root) | `placement.rs` · `an_imported_seeds_main_placement_is_not_flagged` (`placement.rs:2386`) |
| K-3 | one locus type built as two instances in two domains (`a: W` pinned, `b: W` default) | `W` → the first instance's domain only | two rows, two domains | agreement for the F.31 rule: off the root the receiver co-locates, so the first-wins caller pool is never compared with a different callee. For sync inference, see K-5 | ty:1131-1137 | `placement_table.rs` · `two_instances_of_one_type_have_two_domains` |
| K-4 | `replicas = K` | one `Pinned(f)` for all K | K pinned domains | agreement: no rule compares two replicas, which are non-addressable (sem:3382-3399). The projection gives `Pinned(f)` for each | sem:3382 | `placement_table.rs` · `replicas_are_k_domains_each_nesting_its_own_tree` |
| K-5 | sync inference (`si:123-216`) reads the type map for the **enclosing** locus's pool (`si:164-166`) and takes the union of the pools that access each `@form(hashmap)` type (`si:197-206`) | first-wins per enclosing type | per instance | agreement **only if** the switch computes accessor domains per accessed instance and then takes the maximum discipline over a type's instances. A plain union of instance domains per type would infer `serialized` where today's answer, `none`, is right: two `@form` fields, each touched by its own owner on its own pool, need no synchronization. The switch commit pins that | ty:1131-1137 (the 2026-07-15 handoff note at `chk:4773-4787`) | `sync_inference` shadow: the inferred discipline is identical for every `@form(hashmap)` locus in the corpus, plus `two_owners_on_two_pools_infer_none` |
| K-6 | qualified, aliased, generic, module-qualified and contract-typed fields that are **nested** | aliased and contract-typed: no row (`TopSymbol::Type` and `Interface` are not `Locus`). Generic: the template name (`Cache`), not the monomorph | rows with `realizes` resolved | known old bug, **by reading**; § 3's cases confirm before the switch | ty:137 (aliases are transparent in params); ty:743-747 (generic loci); sem:506-521 (a contract-typed field holds the impl that was built) | § 3 cases 5-8 |

**Readers in the checker that are not this row.** The registry's row says `enclosing_field_placement` is "one of four in-checker derivations (two more inline in the blocking and single-thread checks)". Measured, the placement readers in the checker are:

- **The blocking check: three inline derivations.** Phase 1 (`chk:2731-2771`), phase 2 (`chk:2902-2929`) and the birth-order trap (`chk:3070-3092`) each read the root's entries again over single-segment field types. These are E2's to switch (the plan's E2: "the starvation and birth-order phases become laws over rows"). They read the table's per-field domain and the pool's `async_io`.
- **The single-thread check: none inline.** It calls `compute_pool_of_locus_type` and `enclosing_field_placement`.
- **Four more checks:**
  - `check_pinned_locus_in_loop` (`chk:3979-3990`) reads the root's pinned entries. It switches in P1 PR 3; agreement.
  - `check_placement_entry_consumed` (`chk:4218-4232`) reads the **last** `is_main && !__lib_`. Per registry line 402 that differs from lowering's root only under rule 1's error. It switches in P1 PR 3; agreement for accepted programs.
  - `check_instance_aliasing` (`chk:17691-17724`) reads the last `is_main` with no filter, so an imported root can be chosen (registry line 404). It switches in P1 PR 3 as an intentional correction on fixture row F-R's principle. Pinned by `an_imported_seeds_aliasing_is_not_reported`.
  - `check_pool_affinity` (`chk:3777-3845`) validates every `main` declaration's own block. It derives no placement fact and stays as it is (registry line 405).
- **One read through the bus graph.** `check_bounded_bus` reads `collect_subscriber_placements` (`chk:5611-5640`) and switches with the bus graph (B-2).

### 2.2 The bus graph: `collect_subscriber_placements`

**The question.** Whether a subscriber or publisher type runs on the owner's (main) thread, another pool, or a pinned thread (`bg:609-667`). The answer feeds `SubscriberSite::placement` (`bg:466-471`), the direct-call gate (`bg:496-514`), the LSP's `hale/busGraph`, the model's dispatch survey and `check_bounded_bus` (`chk:5611`).

**What it computes.** It walks **every** locus's `placement { }` block, module-nested and imported `__lib_` mains included (`bg:621-667`). Each entry is keyed by the **last segment** of the field's written type (`single_named_type`, `bg:730-735`), first-wins. A type no entry names is `SameThread`. Lowering builds a second instance of the graph over the merged, desugared program (`res:337-379`), so it runs with stdlib loci under mangled names that a last-segment key never matches.

**The answers the table keeps.** The direct-call gate asks one thing of placement: does every publisher and every subscriber of the subject run on the thread that drains the main queue? The table answers that with the set of domains of each type's instances. The gate holds only when every set is `{Main}`, which is stricter than today: a type with any instance off main fails the gate.

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| B-1 (fixture F-N, 14 rows) | a locus nested under a root field placed off main (`Kid` in pinned `Pub`; `Registry` in pool `io`; `BytesBuilder` in pool `ws`) | `SameThread` | the owner's domain | known old bug → approved correction | sem:3511-3542; rt:364-371; ty:1123-1125 (step 2: nested methods run on the parent's thread) | § 4's runtime test; `bus_graph.rs` · `a_nested_subscriber_under_a_placed_owner_is_not_same_thread`; the dispatch-plan flavor for § 4's programs is `static_bucket`, not `static_direct` |
| B-2 | `bounded(N, …)` on a subscriber nested under an off-main owner | accepted (the label is `SameThread`) | refused with the existing message | known old bug → approved correction, a **new error** at an existing position. J commit, wording pinned | dec:3205-3210 (bounds apply to main-queue registrations only; on pool and pinned rings they are typecheck-rejected) | `placement.rs` · `a_bounded_subscriber_nested_off_main_is_refused` |
| B-3 (fixture F-Q) | a qualified field type | the last segment `Listener`. In the checked bundle it never matches a subscriber site keyed by a declaration name. In lowering's merged bundle, where the declaration is `__StdIoTcpListener`, it does not match either | the declaration by identity | known old bug → approved correction | as K-1 | the shadow's six F-Q rows turn into agreement |
| B-4 (fixture F-R) | an imported `__lib_` root's entries | they label types (first-wins over every block) | no rows | intentional correction (approved) | `ent:1-31` | `bus_graph.rs` · `an_imported_roots_placement_labels_nothing` |
| B-5 | a last-segment collision: a user `locus Listener` beside a field typed `std::io::tcp::Listener` placed off main | the user's `Listener` is labelled with the stdlib field's placement | each declaration is labelled by its own instances | known old bug | registry `placement` § Invariants: placement is keyed by instance | § 3 case 4 |
| B-6 | `Placement::Pinned == Placement::Pinned` across two different pinned instances | equal | two domains | agreement in this graph, which only ever compares against `SameThread`. It matters in O-2 | sem:3382 | — |

**Readers that are not this row.** `has_offthread_placement` (`bg:691-722`) answers "does any entry put a thread off main". It is one term of codegen's `program_has_offthread` (`cg:1149`, `cg:1223-1228`). It walks every block, so an imported root's entries count, which is conservative and safe. The table narrows it to the root's rows: an intentional correction, and **E2's** to switch (plan E2; registry lines 883-884). The narrowing must be pinned with the runtime flag. `g_bus_has_pinned` stays 0 when the only pinned entry belongs to an imported root, because no thread is spawned.

### 2.3 The ownership graph: `collect_placements`

**The question.** The same per-type label (`og:792-854`, a verbatim copy of `bg:621` with `named_type` at `og:856` for `single_named_type`). It is read by `classify_edge` (`og:746-783`), which compares the owner's label with the enclosing locus's label and gives `SameTower`, `CrossPool` or `Open`. Lowering's bubble plans read the edge class (`cg:3408-3451`). `SameTower` allocates the child in the owner's arena from the enclosing thread. `CrossPool` marshals an asynchronous post to the owner's thread, where the instance is legal only as a bare statement (sem:308-317).

**Not in the shadow.** The fixture compares the checker with the bus graph only. This copy has the same divergences, but its consumer differs and the stakes are higher: an arena is per thread and unsynchronized.

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| O-1 | an enclosing locus nested under a pinned root field instantiates `I { }`, and `I` bubbles to the root, which accepts `I` | both labels `SameThread` → `SameTower`: the child is allocated in the root's (main's) arena **from the pinned thread** | `CrossPool` | known old bug → approved correction (B-1's principle reaching this consumer). The site becomes fire-and-forget: a value use that compiles today becomes the existing compile error. J commit | sem:308-317; rt § Interest-based ownership; sem:3511-3542 | `ownership_graph.rs` · `a_nested_enclosing_under_a_pinned_field_is_cross_pool`; an ASan run of the same program in `ownership_bubble_crosspool.rs` |
| O-2 | an enclosing locus nested under a pool-`io` root field, bubbling to an owner on pool `io` (or to that root field itself) | enclosing `SameThread`, owner `CrossPool(io)` → `CrossPool`: a post to the thread the code already runs on, and a value use is refused | `SameTower` | known old bug → approved correction; it **admits** a value use that is refused today | as O-1 | `ownership_graph.rs` · `a_nested_enclosing_on_its_owners_pool_is_same_tower` |
| O-3 | one enclosing type with instances in two domains (under `main` and under pinned `p`) | first-wins: one class for both | the instances disagree | **unresolved disagreement** → § 7 U-1. Lowering emits one code path per `(enclosing type, child type)` (`cg:3408-3423`), so per-instance classes cannot be expressed. Recommendation: mixed domains → `Open` (stay transient, as today's non-bubbling sites do) | — | `ownership_graph.rs` · `an_enclosing_type_in_two_domains_is_open` |
| O-4 | `Pinned == Pinned` for two different pinned instances | `SameTower` | `CrossPool` | unreachable today: rule 6 (sem:3232) forbids `accept()` on a pinned locus, so an owner is never in a pinned domain of its own. The table states it as a law rather than relying on the coincidence | sem:3232 | `placement_table.rs` · `no_accepting_owner_is_anchored_in_a_pinned_domain` |

### 2.4 The model: `PlacedIn`

**The question.** The arrangement per instance, with replicas: `LocusInstance`, `Realizes`, `PlacedIn`, `Owns` and `ThreadDomain` (`mb:2934-3275`). It is the closest of the eight to the table. Instances are keyed by path (`App.f`, `App.f[2].k`), replicas fan out (`mb:3061-3110`), nested rows inherit (`mb:3092-3098`), and domains are named `main`, `pool:X`, `pinned:<path>` and `binding:<topic>` (`mb:3092-3098`, `mb:3199-3202`).

**What it computes.** The root is the **first** `is_main` in `ast.loci` (`mb:3124-3127`), in bundle program order with modules flattened and imported `__lib_` mains included. Declarations come only from the user seeds (`mb:2954-2979`; `mb:2612` says `ast.loci` covers only the user seeds). Field types are read by last segment (`mb:3025-3027`), and a field whose type is not a user locus is skipped (`mb:3057`). `affined_to` is in the model's relation set (`hale-model/src/application.rs:152`), but the builder never pushes a row: the column is always empty.

**Contracts that must not move.** The arrangement is outside the model's shape half (`mb:2938-2940`), so `shape_hash` does not move. Instance ids are assigned in path-sorted order (`mb:3223-3231`), so **adding** an instance renumbers the ones after it. The PR that switches this row must show `shape_hash` and the obs ids unchanged over the corpus. U-4 is the one change that could move them.

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| M-1 | a bundle holding an imported `__lib_App` whose program sorts before the seed's own | the arrangement is rooted at `__lib_App` | rooted at `lowering_root` | intentional correction (fixture F-R's principle) | `ent:1-31` | `model.rs` · `the_arrangement_is_rooted_at_the_deployed_main` (`--dump-model` pinned) |
| M-2 | stdlib-typed fields (`std::io::tcp::Listener`; `BytesBuilder` nested) | no instance | rows exist in the table | **unresolved disagreement** → U-4. Projecting them moves instance ids. Recommendation: the projection filters to user declarations in P1, so ids hold, and adding them is a decision of its own | — | `model.rs` · `stdlib_instances_are_not_projected_yet` |
| M-3 | a user `locus Listener` and a field typed `std::io::tcp::Listener` | the field is arranged as the user's `Listener` | `realizes` = the stdlib declaration | known old bug | registry § Invariants | § 3 case 4 |
| M-4 | `affined_to` | empty | one row per domain with an affinity | **unresolved disagreement** → U-5. It changes `--dump-model` and not the shape | sem:3400 | — |
| M-5 | generic, aliased and contract-typed fields | the template name, the alias name (no instance), or the contract name (no instance) | resolved | known old bug, **by reading** | ty:137, ty:743-747, sem:506-521 | § 3 cases 5-8 |

### 2.5 The desugar's read: `collect_off_owner_thread_fields`

**The question.** Whether `(owner, field)` is placed off the owner's thread (`ds:1163-1197`). The intra-locus rewrite then refuses to turn a publish into a direct same-thread call to that field's handler (`ds:1020`, `ds:1123`).

**Where it runs.** Inside `resolve_program`, after the check (`res:217-218`), so it can read the snapshot's table: one more argument threaded into `desugar_intra_locus_topics` (`ds:995`).

**Agreement.** Entries exist only on the root, and a parent-to-child edge below the root is always same-thread. The walk also reads imported roots' blocks (`ds:1170-1191`), but a rewrite inside a `__lib_` root type is never instantiated, so nothing observable differs. The table's `owner_relative == OffOwner` gives the same set over everything the root deploys. Pinned by the shadow, with the rewrite relation (`IntraLocusRewrite`) identical over the corpus.

### 2.6 Codegen: `collect_main_placement`

**The question.** Main's per-field schedule class, pools, affinity, NUMA node, async_io pools, replicas, and the per-type sets that shape structs (`cg:10295-10509`, called at `cg:8891`).

**What it computes.** The root is the first `is_main && !__lib_` over the flat declarations (`cg:10296-10300`), which is `lowering_root` by construction (`ent:185`). The map is keyed by the **field name** of the root, and instantiation reads it only while lowering the root's params (`inst:2201-2205`, `inst:2458-2477`). Replicas are kept on `Cx::main_placement_replicas` (`cg:3741`, `cg:10388-10404`), outside the plan (`dep:20-22`), and fanned out at `inst:2531-2589`. Two type sets are keyed by the written field type: `pinned_locus_types`, which gives a struct its mailbox field (`decl.rs:448-462`), and `coop_pool_locus_types`, which synthesizes the `__coop_pool_run_<L>` wrapper. A qualified path is resolved through `mangled_for_path` (`cg:10328-10336`), which knows the stdlib's mangled names and the cross-seed renames and nothing else (`cg:6382-6388`), and a generic field is **skipped** (`cg:10324`). Adapter loci named in `bindings { }` join `pinned_locus_types` (`cg:10494-10508`).

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| G-1 | the root | first `is_main && !__lib_` | `lowering_root` | agreement (by definition) | `ent:96-113` | `entry_point_placement.rs` (existing) |
| G-2 | a generic root field placed `pinned` or on a pool | no type-set entry: by reading, a pinned monomorph subscriber gets no mailbox field, and a pooled one gets no `__coop_pool_run` wrapper (the defensive synchronous path `cg:10305-10318` warns about) | `realizes.lowered` = the monomorph | known old bug, **by reading**; § 3 case 6 confirms that the shape reaches lowering (rule 3 admits an unresolved name, `chk:9822-9833`) | ty:743-747 | case 6 |
| G-3 | an aliased (`type Held = Holder`) or module-qualified (`m::K`) root field | the alias name, or `None` from `mangled_for_path` for a seed-local module path: no type-set entry | the declaration | known old bug, **by reading**; § 3 cases 5 and 7 | ty:137 | cases 5, 7 |
| G-4 | adapter loci in `bindings { }` | pinned-equivalent with no placement entry | a row with `Decision::Binding` and a pinned domain | agreement. The row exists for C7's inline-adapter judgment and for R-6 | registry `law_backstops` | `placement_table.rs` · `an_inline_adapter_is_a_pinned_binding_row` |

### 2.7 Codegen: `DeploymentPlan`

**The question.** The value `collect_main_placement` fills (`dep:33-60`). It becomes the table's lowering view: the root rows' schedule class, NUMA node and pool, the domains' affinity and `async_io`, the replicas (folded in, so `Cx::main_placement_replicas` goes), and the two type sets from `realizes.lowered`. Its consumers keep reading `self.deployment.<field>`.

**Agreement**, except for G-2 and G-3, which are corrections. The extraction must preserve execution. Its oracle is the dispatch-plan digest, the IR-level build identity, and the corpus oracle over the corpus, all identical. Any of G-2 or G-3 that § 3 confirms lands as its own J commit **before** the switch, with a runtime test in which the shape's subscriber receives on the right thread.

### 2.8 The resource budget: `budget_for_programs`

**The question.** A static count of OS threads, cooperative pools, bus subjects and fd sites (`rb:152-189`), behind `--dump-resource-budget` and the CI ceiling `--check-resource-budget` (`hale-types/src/lib.rs:158-195`; the catalogue at `spec/verification.md:2284-2292`).

**What it computes.** It counts `+1` per `pinned` entry (`rb:204`) and inserts each pool name, `main` included when spelled (`rb:205-211`). It reads every locus's placement block at the top level and **one** module level deep (`rb:156-176`), so it counts an imported root's entries and misses a root nested two modules deep. It ignores replicas and adapter bindings. Wave-2 decision: **budgets count the resource.**

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| R-1 | `pinned(…, replicas = K)` | 1 thread | K threads | known old bug → approved correction | sem:3382-3399 ("each on its own OS thread"); rt:404 | `resource_budget.rs` · `replicated_pinned_counts_k_threads` |
| R-2 | instances on one pool | the name is deduplicated: 1 | one worker per pool domain | agreement, and the decision ("instances on one pool are not workers") is pinned so the switch cannot count rows | rt:354-362 (one worker per pool name) | `resource_budget.rs` · `three_instances_on_one_pool_are_one_worker` |
| R-3 | `cooperative(pool = io, cores = 2..4)`, and two entries naming one pool, one with an affinity | 0 threads, 1 pool | the same: affinity is a column of the pool domain, never a thread | agreement, pinned by the decision ("affinity entries are not threads") | sem:3400-3410 | `resource_budget.rs` · `pool_affinity_adds_no_thread` |
| R-4 | an imported `__lib_` root, or a module-nested non-root `main`, with pinned entries | counted | not counted: only the deployed root's rows count | known old bug → approved correction | `ent:1-31`; sem:3411-3437 | `resource_budget.rs` · `an_imported_roots_entries_cost_nothing` |
| R-5 | the root built at several sites or in a loop (a factory called in a loop is legal, sem:3432-3436) | counted once | threads × the root's `Bound`; `Unbounded` renders as an uncertainty, and a ceiling against an uncertain count fails with that reason | known old bug → approved correction ("dynamic instantiation needs a bound or an uncertainty") | sem:3411-3437 | `resource_budget.rs` · `a_root_built_in_a_called_loop_is_uncertain` |
| R-6 | adapter loci in `bindings { }` (pinned-equivalent threads, `cg:10494-10508`) | 0 | 1 thread each | **unresolved disagreement** → U-3. The decision covers placement, not bindings; recommendation: count them, since they are threads | — | `resource_budget.rs` · `an_inline_adapter_is_a_thread` |
| R-7 | `cooperative` or `pool = main` spelled | `main` is counted as a pool, and only when spelled | `main` adds no worker | **unresolved disagreement** → U-2. Recommendation: pools are worker pools; `main` is rendered on its own line. A declared ceiling can only loosen | rt:354-362 | `resource_budget.rs` · `the_main_pool_is_not_a_worker` |

Threads the runtime spawns outside placement are not placement facts. Examples are binding reader threads (one domain per binding, `mb:3199-3202`) and the serve threads of the GH #233 stdlib transports (`cg:10494-10498`). The budget renders them as an explicit "not counted" line until P2's binding rows can count them.

### 2.9 Placement readers outside the eight rows

| reader | what it asks | who switches it |
|---|---|---|
| sync inference (`lib.rs:440-454`, single-program bundle before the sequence) | the type map, for the enclosing pool | P1 PR 3: calls the producer on its bundle; K-5's rule. The test-local copy (`si:505-563`) is replaced by the producer |
| `has_offthread_placement` / `program_has_offthread` (`bg:691`, `cg:1149`, `cg:1223-1228`) | any thread off main | E2 (§ 2.2) |
| the blocking check's three derivations | per root field: pool, `async_io`, pinned | E2 |
| `placement_implied_diags` (`effects.rs:538-590`) | which types run on an `async_io` pool (root fields, single segment, every top-level `is_main`) | registry line 418: reads `lowering_root`, then the entry at L4. When it moves to the table, its set gains nested members of `async_io` pools: classified then, not in P1 |
| LSP `hale/placement` (`hale-lsp/src/lib.rs:2409-2450`) | the root's params and entries, for display | registry line 419: L4. Showing per-instance domains is optional and not in P1 |
| ownership decisions, `placed_fields` (`ownership.rs:1751-1760`, read at `ownership.rs:3095-3130`) | whether a field has an entry (`Decision::Placement`) | a syntax fact; stays |
| `check_pool_affinity` | validation of every declared block | stays (§ 2.1) |

## 3. Coverage the shadow must run

**The shadow loads through the snapshot.** Today's shadow parses each program alone, with no stdlib merge, no imports and no desugar (`shadow_placement.rs:22-25`). The corpus it walks (`hale-corpus/src/lib.rs:76-152`) is the example fixtures, the CLI fixtures, the stdlib and the Rust-embedded programs. That excludes `tests/hale` and `dna/`. P1's oracle is "the corpus, `tests/hale` and the DNA seeds", and imported roots exist only with imports. So the P1 shadow loads every seed through `Snapshot::load` (seeds under `tests/hale` and `dna/`; corpus programs as single-file seeds), demands `placement`, and compares the table's projection with **every** legacy producer: the checker's map, `enclosing_field_placement` at each receiver, the bus graph, the ownership graph, `PlacedIn`, the desugar's set, `DeploymentPlan` and the budget. Each comparison is a column of the fixture. Every divergence row cites an id from § 2. The gate stays at zero unexplained and zero stale.

**The cases.** Each is a unit test of the table in `crates/hale-types/tests/placement_table.rs` (a new file, which joins its area binary in `Cargo.toml`). Each also runs through the shadow, so its legacy answers are recorded beside the table's.

1. **Two instances of one type in different domains.** `a: W` pinned, `b: W` on pool `io`, `c: W` default, each nesting `k: K`. Six rows, three domains for `W`, and `K` inheriting each. Legacy: one domain per type (K-3, B-6, O-3).
2. **Nested inheritance, three deep.** A root field on pool `io`, nesting `M`, nesting a subscriber `S`. `S` is on `io` (B-1, O-2). The same tree under `pinned` gives `S` the pinned domain of the root field's instance, and under `replicas = 3` it gives three domains with `[i]` in each nested key.
3. **Overrides.** `App { gw: Gateway { router: RouterV2 { } } }` against `Gateway`'s default `RouterV1`, and a contract-typed param (`j: Counter = Churner { }`). `realizes` names the literal's declaration, `literal` names the override's site, and the domain is inherited (M-5, K-6; sem:506-521, sem:4086-4093).
4. **Qualified types and a last-segment collision.** A root field `l: std::io::tcp::Listener` on pool `io`, beside a user `locus Listener` used elsewhere (K-1, B-3, B-5, M-3).
5. **Aliased types.** `type Held = Holder;` with `h: Held = Holder { }` placed `pinned`, and an alias nested one level down. Expected: one row realizing `Holder`. The case first pins whether rule 3 admits the field: `chk:9822-9833` reads the resolved `Ty`, which by reading is `Holder`. If rule 3 admits it, G-3 and K-6 are confirmed; if it refuses, the rows reclassify as a checker gap (K-6, G-3, M-5).
6. **Generic specialization.** `c: Cache<Int, String> = Cache { cap: 2 }` placed `pinned` with a subscription, and `d: Cache<Int, Int>` on pool `io`. Two rows, one template site, two substitutions, two `lowered` names. Confirms G-2 by building both (a compile-only check that `pinned_locus_types` holds `Cache_Int_String` after the switch).
7. **Module-nested and module-qualified.** A root field `k: m::K = m::K { }`, and a seed whose only `main` is inside `module app { … }`. `RootRow::is_entry == false`, rows present, because lowering deploys it (`ent:96-113`; the nested-main transition test `nested_main_transition.rs`).
8. **Imported roots.** A seed importing a library whose `main locus` has pinned entries, with and without the seed's own `main`. The import gives no rows (K-2, B-4, M-1, R-4). With the seed's main, the rows are the seed's. Run from `tests/hale` through the snapshot, since a parse-only shadow cannot see it.
9. **Adapter bindings.** An inline adapter in `bindings { }`: a `Decision::Binding` row with a pinned domain (G-4, R-6).
10. **Dynamic sites.** A locus literal in a root method in a loop, an `accept`ed child, and the root built by a factory called in a loop. These produce `DynamicSite` rows with the enclosing domains and the bound, and the root's `Unbounded` bound (R-5).

## 4. The runtime test the correction needs

**What the reading predicts.** The fixture's deferred note says to "probe obs_intra_tree_publish … for a direct call across threads". That program cannot show it. Its only publisher of `ToChild` is the pinned owner `Pub` itself. The desugar rewrites that publish into a call on `Pub`'s own thread (`ds:1100-1129`), which is correct, and its other subject has a pinned publisher, which fails the gate's publisher leg (`bg:498-505`). Reading the registration path shows a hazard that is **wider than the direct-call gate**:

- **The registration route.** A nested subscriber registers through the cooperative branch with no mailbox (`inst:3808-3816`, `None`). The only route it gets is a pool: `current_cooperative_pool` at lowering, or `lotus_coop_pool_current()` at runtime (`brt:434-479`), and on a pinned thread or main that is null. A pinned owner has a mailbox (`inst:3930-3990`), but its nested children do not use it.
- **A nested subscriber under a pinned owner.** Its cells go to the program-wide queue, which drains only on its owner thread, main (`A:6639`, `A:7311`). By reading, its handler runs on **main** in every flavor. That breaks ty:1123-1125 and races with the pinned owner's own code.
- **A nested subscriber under a pool owner.** Its entry carries the pool, so the deferred path is right. But when the gate also passes (all labels `SameThread`, a quiet handler, a flat payload, a single handler), lowering picks the direct-inline flavor. Its accessor returns NULL for an entry with a pool (`A:19841-19852`), so by reading the delivery is **dropped**.
- **A nested publisher under a pinned owner, with a quiet subscriber on main.** The publisher leg reads `SameThread`, so the handler runs synchronously on the pinned thread (`A:19776-19777`). That is GH #253's race in a new shape.

Deliver records are no witness here. `lotus_obs_bus_deliver` is called at post time on the **publisher's** thread in every flavor (`A:19780`, and the fanout sites listed at `A:11053-19641`), so the ring a record lands on names the publisher. The test needs a witness for the thread that runs the handler.

**The test.** `crates/hale-codegen/tests/nested_offthread_delivery.rs`, joining the `replay_obs` area (`tests/replay_obs.rs`, beside `obs_intra_tree_publish`), built with `harness::unique_bin` and `build_opts::options()`. Each case runs in two arms, devirtualization on and `BuildOptions::no_bus_devirt = true` (`cg:649`), and must hold in both:

- **A — pinned owner, nested subscriber, publisher on main.** `Owner` is placed `pinned`; it nests `Kid`, which subscribes to `Tick { payload: P }` with a quiet handler (`self.got = self.got + 1`). A root sibling `Feeder` on main publishes five cells about 100 ms in. `Owner.run()` reads `self.k.got`, spins without yielding until about 400 ms on `std::time::monotonic`, reads it again, prints both, then sleeps and prints the final count. A pinned thread drains its mailbox only at sleep and yield. So if the handler runs on `Owner`'s thread, nothing changes inside the window, and the assertion is `before == after` and `final == 5`. The reading predicts `after - before == 5` in both arms today.
- **B — pool owner, nested subscriber, publisher on main.** The same program with `Owner` on `cooperative(pool = io)`, a single subscriber type and a flat payload. Assert `final == 5` in both arms. The reading predicts `0` in the devirtualized arm today.
- **C — pinned owner, nested publisher, quiet subscriber on main.** The plan-level assertion is compile-only and gates in CI: the subject's flavor in the lowering view's `DispatchPlan` is not `static_direct`. The execution witness is a ThreadSanitizer arm in the shape of `form_hashmap_lockfree_tsan.rs` (`#[ignore]`, `LOTUS_TSAN=1`), which asserts no `WARNING: ThreadSanitizer` line. Main has no yield-free window, because it drains at every statement boundary, so only a race detector can see where this handler ran.

The test lands in the J commit that switches the bus graph (PR 5 below). It is red on the parent commit, and the PR body quotes that red run. If A or B stays red after the label correction, the registration route is the remaining cause. The fix is in lowering: a nested subscriber registers with its tower's route, the owner's mailbox when the domain is pinned and the pool when it is a pool. The table gives lowering that domain per instance. Whether that fix belongs to the same PR is U-6.

## 5. Consumer switch order

Six PRs follow this document. Each consumer switches in a commit of its own, and each correction is a J commit of its own, with wording-pinned tests and the decision it applies named in the body. One PR is in CI at a time and at most one beyond (the WIP rule). Every PR body is the usual document: What lands, Architecture with the invariants kept, Tests, Deferred. A user-visible change has its fragment in `unreleased/<pr>.md` in a second push.

| # | PR | shape | carries | oracle |
|---|---|---|---|---|
| 1 | **The producer.** `crates/hale-types/src/placement.rs` · `placement_table`, `Snapshot::demand_placement` and a `FAMILIES` entry. The shadow rewritten as § 3 describes, with the fixture extended and every row cited by id. `placement_table.rs` with § 3's ten cases. The registry names the producer. No consumer reads it. | S · Opus | the producer commit; the shadow commit; the cases commit; the registry commit | shadow: zero unexplained or stale over the corpus, `tests/hale` and `dna/`; `cargo nextest run --release --workspace`. **L1 stacks on this PR's producer commit.** |
| 2 | **The lowering view.** The desugar's read (D, agreement), then `DeploymentPlan` as the table's lowering view, with replicas folded in and the type sets from `realizes.lowered`. Each of G-2 and G-3 that § 3 confirmed lands first as its own J commit with a runtime test. | S, with J for confirmed G rows · Opus | registry: `collect_off_owner_thread_fields`, `collect_main_placement` and `DeploymentPlan` leave the legacy list, and the seam `collect_main_placement(` goes | dispatch-plan digest and build identity identical over the corpus; `IntraLocusRewrite` relation identical; `LOTUS_ASAN=1 … --test corpus_oracle -- --ignored`; the ownership-matrix sample |
| 3 | **The checker.** F.31 reads `domain` and `owner_relative` (K-1, J). `compute_pool_of_locus_type` becomes a projection for sync inference (K-5's per-instance rule, S). Pinned-in-a-loop and entry-consumed read the root rows (S). Instance aliasing reads them too (intentional correction, J). `enclosing_field_placement` goes. | J + S · Opus for K-1 and K-5, Sonnet for the rest | the K-1 test; `two_owners_on_two_pools_infer_none` and the sync-inference shadow; `an_imported_seeds_aliasing_is_not_reported`; the registry's checker row removed and the seam `compute_pool_of_locus_type(` narrowed or gone; a fragment for the qualified-field diagnostic; sem § Placement block (rule 3 names qualified stdlib loci) and the docs chapter `docs/src/services/concurrency.md` | diagnostics identical over the corpus and `tests/hale` except the pinned K-1 cases |
| 4 | **`PlacedIn`.** The model's arrangement projected from the table: M-1 (J), M-3 (J), and the M-2 and M-4 projections as U-4 and U-5 decide. | J · Sonnet after PR 3 | `--dump-model` pins; `shape_hash` and obs ids identical over the corpus (stated in the body); the registry row removed; a fragment | `hale check --dump-model` over the corpus differs only in the pinned cases |
| 5 | **The two graphs.** The bus graph (B-1, B-3, B-4, B-5: J), `check_bounded_bus` (B-2: J, a new error), the ownership graph (O-1 and O-2: J; O-3 as U-1 decides) and the registration route if U-6 puts it here. `collect_placements` and `collect_subscriber_placements` go. | J · Opus | § 4's test (red on the parent, quoted); the B and O tests; an ASan run of the ownership cases; registry rows removed; a fragment naming the dispatch-plan digest moves (a recording of an affected program from before this PR no longer admits for replay); sem § Nested instantiation, rt § Placement classes (subscriptions follow the tower), dec F.38's note if wording changes; the docs chapter | suites; `ownership_matrix` sample; `LOTUS_ASAN=1` corpus oracle; `cargo test -p hale-cli --test dna_native_suite`; `hale test tests/hale` |
| 6 | **The resource budget.** R-1, R-4 and R-5 (J), R-2 and R-3 (pinned agreement), R-6 and R-7 as U-3 and U-2 decide. | J · Sonnet | `resource_budget.rs` tests; `spec/verification.md:2284-2292` and `docs/src/verification.md:154` (what is counted, the uncertainty, the "not counted" line); `notes/resource-budgets.md`; the registry row removed, which closes `placement` × 8; a fragment | `--dump-resource-budget` over the corpus differs only in the pinned cases |

PR 2 precedes the J PRs for two reasons. It is the only one whose oracle is "nothing executes differently". And PR 5's registration route reads the table's per-instance domain through the lowering view PR 2 builds. E2 (`has_offthread_placement` and the blocking derivations) and C7 (the placed-`Unknown` judgment) need only PR 1.

## 6. What the fixture and the registry contradict

- **The registry's checker row undercounts.** It says the checker holds four derivations, "two more inline in the blocking and single-thread checks". Measured: three inline in the blocking check, none in the single-thread check, and four more readers elsewhere (pinned-in-a-loop, entry-consumed, instance aliasing, affinity validation), plus `check_bounded_bus` reading the bus graph's map (§ 2.1).
- **The fixture's deferred probe names a program that cannot show the hazard** (§ 4). By reading, the hazard is also wider than the label the fixture classifies. The nested subscriber's registration carries no route, so a pinned owner's nested subscriber is delivered on main in every flavor, and a pool owner's can be dropped by the direct-inline flavor.
- **The fixture classes the nested rows `known-old-bug`, and the decisions call them approved corrections.** Both are right at different ends. The bug is in the bus graph, and the correction is what its consumers see when they switch. This document uses "known old bug → approved correction" for those rows.
- **The ownership graph's copy is in neither the shadow nor the fixture.** The registry calls it verbatim, and it is: the logic is identical. But its consumer allocates across arenas, so its divergences need rows of their own (O-1 to O-4). PR 1's shadow adds its column.
- **The budget's legacy row says "counts placement entries, ignores replicas".** That is true, and it also counts every locus's block (imported roots too), reads only one module level, counts `main` when spelled, and never counts adapter threads (§ 2.8).
- **The registry lists `model (placed_in, affined_to)` as a consumer, but the builder never produces `affined_to`** (M-4).
- **The shadow's corpus excludes `tests/hale` and `dna/`, and parses each program alone.** So it cannot see imported roots, which the oracle requires (§ 3).

## 7. Decisions for the driver before the pane starts

- **U-1 (O-3).** Recommended: an enclosing type whose instances span domains gets edge class `Open`, so no bubble plan is made.
- **U-2 (R-7).** Recommended: `cooperative_pools` counts worker pools only, and `main` is rendered on its own line.
- **U-3 (R-6).** Recommended: adapter bindings count as threads. Binding readers and transport serve threads are rendered as "not counted" until P2.
- **U-4 (M-2).** Recommended: `PlacedIn` projects user declarations only in P1, so instance ids hold. Adding stdlib instances is a separate decision, with the id move stated.
- **U-5 (M-4).** Recommended: project `affined_to` in PR 4, since it changes no shape.
- **U-6 (§ 4).** Recommended: the registration route lands in PR 5 as its own J commit, before the bus-graph commit. Otherwise the runtime test cannot be green in the PR that adds it.
- **K-6, G-2, G-3, M-5 (by reading).** Each is a known old bug **only once § 3's case confirms the shape reaches the producer**. A case that the checker refuses first is reported as a checker gap and reclassified before its PR. It is never left unclassified.
