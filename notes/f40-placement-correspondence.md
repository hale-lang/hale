# F.40 phase 3, P1 — the placement correspondence

**What this is.** The correspondence P1 requires before its producer is built (hale-lang/hale#1212; the phase-3 plan's § 3 Line P, step P1, and § 6). It does five things. It fixes the schema of the one placement table and the stage that produces it. It reads each of the eight legacy producers the registry lists under `placement`, and states which answers the table keeps. It classifies every known divergence and gives its spec witness and the test that will pin it. It orders the consumer switches. Last, it proposes answers to the open decisions (§ 7), records what the implementation has to get right (§ 8), and states the checkpoints the producer PRs are verified against (§ 9). The decisions it applies are the wave-2 decisions § P1: instance identity includes field path and replica (and here, after the outside review of #1296, the construction the instance comes from); budgets count the resource; and a runtime test checks the receiving thread of the nested off-thread subscriber.

The step is read-only on code. It was measured at `main` 13e5f352 (origin/main after #1293), and line numbers refer to that tree. One thing was run: the existing shadow, `cargo test --release -p hale-types --test shadow_placement`, which passes (21 classified rows, none unexplained, none stale). Everything else is **by reading**, and is marked so wherever a claim is about execution rather than code. Running a probe program through `hale` was not available in this session. § 4's test is what settles the execution claims.

**Abbreviations.** `chk` is `crates/hale-types/src/check.rs`; `bg` `crates/hale-types/src/bus_graph.rs`; `og` `crates/hale-types/src/ownership_graph.rs`; `mb` `crates/hale-types/src/model_builder.rs`; `rb` `crates/hale-types/src/resource_budget.rs`; `ent` `crates/hale-types/src/entry.rs`; `res` `crates/hale-types/src/resolved.rs`; `si` `crates/hale-types/src/sync_inference.rs`; `ds` `crates/hale-syntax/src/desugar.rs`; `fs` `crates/hale-frontend/src/snapshot.rs`; `cg` `crates/hale-codegen/src/codegen.rs`; `inst` `crates/hale-codegen/src/locus/instantiation.rs`; `brt` `crates/hale-codegen/src/bus/runtime.rs`; `dep` `crates/hale-codegen/src/deployment.rs`; `A` `crates/hale-codegen/runtime/lotus_arena.c`; `sem` `spec/semantics.md`; `rt` `spec/runtime.md`; `ty` `spec/types.md`; `dec` `spec/decisions.md`.

**Classes.** Each divergence gets one class. The class describes the change a consumer sees when it stops reading its legacy producer and reads the table.

- **known old bug.** The legacy answer contradicts the spec, and the table answers what the spec says. Every known old bug listed here is an **approved correction** under the wave-2 decisions, either directly (the shadow fixture's rows, the budget's accounting) or as the same principle reaching another consumer. Each lands as its own J commit with wording-pinned tests.
- **intentional correction.** The legacy answer was a deliberate stance that a decision now changes. It also lands as a J commit.
- **unresolved disagreement.** The right answer is not decided. It is listed in § 7 with a proposed answer, pending the owner, and its consumer does not switch until the owner decides.
- **regression.** The table answers differently from a legacy producer, no row here explains it, and the investigation of its witness (§ 8) finds the producer wrong. That is a producer bug. It stops the step and is never classified away.

Where a row says **agreement**, the table and the legacy producer give the same answer over everything the shadow sees, and the switch is an S commit.

## 1. The table

### Schema

```rust
/// Which store minted a site. The two stores number independently, each from
/// seed 0 and index 0, so a `SiteId` names a site only with its universe
/// (§ 1, the input universe).
pub enum SiteUniverse {
    User,                                  // the snapshot's identities: the seed's files and the imports linked in
    StdlibAnalysis,                        // the stdlib analysis copy's (`stdlib_bodies::identities()`)
}

/// A site the table names: the universe that minted it and its id there.
/// Every site in the table is one; no bare `SiteId` leaves the producer.
/// Equality, ordering and hashing compare the universe first.
pub struct SiteRef {
    pub universe: SiteUniverse,
    pub id: SiteId,
}

/// Provenance (kind, span, origin) of a site, from the store its universe
/// names and from no other.
pub fn provenance<'a>(site: SiteRef, user: &'a Snapshot, stdlib: &'a Snapshot) -> Option<&'a Site> {
    match site.universe {
        SiteUniverse::User => user.site(site.id),
        SiteUniverse::StdlibAnalysis => stdlib.site(site.id),
    }
}

/// One per snapshot: `Snapshot::demand_placement`, a new FAMILIES entry
/// "placement", demanded after the desugar sequence and the mint over the
/// snapshot's identities (see "The stage" below). `stdlib` is the analysis
/// copy of the bundled stdlib and its own identities (`stdlib_bodies`).
/// Nothing outside a minted snapshot calls it.
pub fn placement_table(bundle: &Bundle, top: &TopScope, entry: &EntryRow, stdlib: &StdlibDecls) -> PlacementTable;

pub struct PlacementTable {
    pub root: Option<RootRow>,             // None: lowering deploys no main locus
    pub domains: Vec<Domain>,              // indexed by DomainId
    pub instances: BTreeMap<InstanceKey, InstanceRow>,
    pub dynamic: Vec<DynamicSite>,         // loci instantiated outside the static tower
    pub holes: Vec<Hole>,                  // what the producer could not decide, with its policy
}

/// Static instance identity: the scope that constructs the tree, the field
/// path from its top, the replica. A key names a template, never a runtime
/// occurrence (see "Templates, occurrences, incarnations" below).
pub struct InstanceKey {
    pub origin: Origin,
    pub path: Vec<Step>,                   // the fields from the origin's top; [] is the top itself
    pub replica: Option<u32>,              // Some(i) on a `replicas = K > 1` field and on every row nested under it
}

pub enum Origin {
    Construction(SiteRef),                 // a root literal (`App { … }`): one template per literal site, the root at []
    Binding(SiteRef),                      // an adapter literal in the root's `bindings { }` entry: built once, in the bindings prelude (cg:9813)
}

pub struct Step {
    pub field: String,
    pub alternative: Option<SiteRef>,      // the literal taken, when the field's initializer chooses among literals; None when it has one
}

pub struct RootRow {
    pub decl: MainLocus,                   // EntryRow::lowering_root, never EntryRow::entry
    pub is_entry: bool,                    // false when lowering deploys a module-nested main (NoEntry::OnlyModuleNested)
    pub constructions: Vec<Construction>,  // every literal of the root declaration, each a template
}

pub struct Construction {
    pub literal: SiteRef,                  // the root literal; `Origin::Construction` of every key under it
    pub bound: Bound,                      // how many occurrences of this template can be live at once
}

pub struct InstanceRow {
    pub realizes: DeclRef,                 // the declaration actually built (an override literal's, not the field's declared type)
    pub literal: Option<SiteRef>,          // the literal that builds it (default or override); None = a hole
    pub owner: Option<InstanceKey>,        // None only at an origin's top ([] path)
    pub domain: DomainId,
    pub decided_by: Decision,
    pub owner_relative: OwnerRelative,     // SameAsOwner | OffOwner
    pub guarded: bool,                     // on or under a step with an `alternative`: live only in occurrences that took it
}

pub struct DeclRef {
    pub site: SiteRef,                     // the locus declaration; for a monomorph, the template's
    pub args: Vec<Ty>,                     // the substitution; empty unless generic
    pub lowered: String,                   // the name lowering keys on (`__StdIoTcpListener`, `Cache_Int_String`)
}

pub enum Decision {
    Entry { block: SiteRef, entry: SiteRef }, // the root's `placement { }` entry for this field
    Binding { entry: SiteRef },            // an adapter locus inline in `bindings { }`: pinned-equivalent (cg:10494-10508)
    Inherited { from: InstanceKey },       // nested: the owner's domain (sem:3511-3542, rt:364-371)
    Default,                               // a root field with no entry: pool main (sem:3544-3552)
}

pub enum DomainKind {
    Main,                                  // the program's main thread; adds no thread
    Pool { name: String, async_io: bool, affinity: Option<CoreSet> }, // one worker per name
    Pinned { anchor: InstanceKey, affinity: Option<CoreSet>, numa_node: Option<i64> }, // one thread per anchor (per replica); the anchor's origin is its creating scope
}

pub struct DynamicSite {                   // a locus literal in a method body, `accept`ed child, let-bound locus
    pub literal: SiteRef,
    pub enclosing: Enclosing,              // Locus(DeclRef) | Fn(SiteRef): a free function or `fn main` can enclose a literal (§ 8)
    pub domains: BTreeSet<DomainId>,       // the domains the enclosing scope runs in; empty = unknown, never defaulted to main
    pub bound: Bound,                      // Once | AtMost(n) | Unbounded(reason)
}
```

### The stage

**After shaping and minting.** The frontend's load runs the desugar sequence over the loaded programs and then mints them (`fs:597-631`). `placement` is demanded from that snapshot, as `entrypoint` is (`fs:856`), so every site in the table is one a mint has already numbered: a `User` site the snapshot minted, or a `StdlibAnalysis` site the analysis copy minted (the input universe, below). The producer is never called where sync inference runs today. `apply_sync_inference` runs per program **before** the sequence and the mint (`fs:590-596`), over a `Bundle::new` it builds itself, whose identity snapshot is empty (`lib.rs:438-454`). Its `entry_row` serves the root only as `MainLocus.site: Option<SiteId>`, `None` on a bundle nothing minted (`ent:41-44`). The table's `SiteRef`s are not optional and cannot be built there, and a placeholder id would alias a real declaration.

**Sync inference moves after placement, and nothing cycles.** Placement reads no `sync` argument. The readers of `sync` are the checker (`chk:4626`, `chk:10825`, `chk:17912`), `alloc_summary.rs:1396`, `frontier.rs:1155`, `mb:485` and lowering, and none of them is a placement producer. Sync inference reads placement. The one ordering that holds both is sequence → mint → entry → placement → the inferred discipline. Today's apparent cycle is only that the checker and lowering read the `sync =` the pre-pass wrote into the AST. C1's form row, as amended, removes it: the explicit configuration (what the source wrote) and the effective discipline (what inference decides from placement) are separate columns, and a reader that asks "has a sync discipline" reads the effective one. So K-5's switch is **C1's**, stacked on P1's producer commit, and not P1 PR 3's.

**Until C1 lands, the pre-mint computation is its own contract.** It stays what it is: a per-program map from locus declaration name to pool, built over the single-program bundle, with no `SiteId` in it. It is typed as such (`PreMintPoolMap`), and it is never converted into table rows. Its correspondence into the table is verified rather than assumed. The shadow gains a column that, for every `@form(hashmap)` locus over the corpus, `tests/hale` and `dna/`, compares the discipline inferred from the pre-mint map with the discipline K-5's rule infers from the table, joined by declaration name within the program. Because the pre-pass runs per file, a multi-file seed's per-file map can lack the root that the table has. Each such divergence is a fixture row with an id, like any other.

**The input universe.** The snapshot mints the programs the frontend loaded: the seed's files and the imports linked into them (`fs:628-631`). The bundled stdlib is not among them. The typecheck bundle never holds it (`stdlib_bodies.rs:16-20`). The analyses read a separate copy, minted alone (`stdlib_bodies.rs:35-44`). Lowering appends the bundled stdlib to the user program inside `resolve_program` and mints the merged program, numbering the stdlib's sites past the user's (`res:238-276`). So there are three mints. Two of them, the snapshot's and the analysis copy's, reach the table; the merged one is lowering's.

**The two universes collide numerically.** `mint` numbers every site from one counter that starts at 0, and numbers seeds from 0 (`snapshot.rs:185-232`). In the user snapshot, seed 0 is the first source unit. The analysis copy is minted with no source map, so its one seed, the program named `STDLIB_SEED`, is also seed 0 (`snapshot.rs:220-222`), and its counter also starts at 0. A user declaration and a stdlib locus can therefore carry the same `SiteId`, and the re-review's probe found one: against 500 user locus declarations, `SiteId 0:99` named a stdlib locus in the analysis copy and a user locus in the snapshot. `STDLIB_SEED` is a seed's display string (`snapshot.rs:168`), recorded in one store's `seeds`, not a reserved numeric seed. Reading it presupposes knowing which store to ask, and a bare id does not say. (The merged mint numbers the stdlib's seed after the source units, a third numeric seed for the same string.)

**Every site carries its minting universe.** Every site the table names is a `SiteRef`, its universe beside its id, wherever the table can reference either universe: declaration identities (`DeclRef.site`, and so `InstanceRow.realizes` and `Enclosing::Locus`), literal identities (`InstanceRow.literal`, `Construction.literal`), alternative identities (`Step.alternative`), dynamic-site identities (`DynamicSite.literal`, `Enclosing::Fn`), origins (`InstanceKey.origin`, through `Origin`) and steps (`Step`). The decisions' sites (`Decision::Entry`, `Decision::Binding`) are always `User`, since only the seed's root has a placement block or bindings, and are `SiteRef`s too, so that no bare `SiteId` leaves the producer. The root's `MainLocus` stays the entry row's type: the entry row is computed over the user snapshot alone, and the producer qualifies its site as `User` when it reads it. Provenance lookup (span, kind, origin) dispatches on the universe (`provenance`): a `User` site is looked up in the snapshot's identities, a `StdlibAnalysis` site in `stdlib_bodies::identities()`, and never the other way.

**Three identities, three roles.** One stdlib declaration has three identities, and none stands in for another:

- **Analysis identity:** the analysis copy's `SiteId`, inside `stdlib_bodies::identities()`. The analyses and the producer's resolution of a qualified field read it. It means something only in that store.
- **Placement identity:** `SiteRef`. The table's keys and rows hold it, and every consumer of the table compares it: the shadow, sync inference after C1, the model projection, the budget, L1's rows. A stdlib site's `SiteRef` is its analysis identity qualified by `StdlibAnalysis`.
- **Lowering identity:** `DeclRef.lowered`, the name lowering keys on: the bundled declaration's own `__Std…` name (res:226-228), the same in both copies because both are clones of one parsed program (`bundled_stdlib`).

**Lowering joins once, in the lowering view.** The lowering view (PR 2) resolves every `SiteRef` it reads into the merged mint. A `User` site joins directly: `resolve_program` keeps the ids the bundle minted (`res:209-216`; minting is idempotent). A `StdlibAnalysis` declaration joins by `lowered`, the declaration-to-lowering join. A `StdlibAnalysis` literal or alternative has no name to join by. It joins by position: both copies are clones of `bundled_stdlib` that no pass touches between the clone and the mint (`res:246-264` asserts it of the merged tail's items), so the two walks visit the same sites in the same order. The view pairs them in that order, asserts each pair's kind and span equal, and asserts that the pairing agrees with `lowered` on every declaration. It asserts the resolution is total and injective over the refs it reads: each resolves exactly once. A ref the merged program lacks, or two refs resolving to one merged site, is a compiler bug that refuses the build and names the row.

**A name join at one seam never repairs a collision at another.** Re-keying by `lowered` corrects only what lowering reads, and only for declarations. The producer, the shadow and every other consumer above read the table's ids directly, and a literal, an alternative or a dynamic site has no name to re-key. So the universe is in the id from the moment the producer writes a row, and no consumer reconstructs it.

**Lowering-generated sites are not inputs.** The table is over the checked program, and no row names a node that a lowering rewrite creates. A consumer that needs one joins through that rewrite's relation (`IntraLocusRewrite`).

Until the producer resolves a qualified field through the analysis copy, a stdlib-typed field's `realizes` is a hole (invariant 6). It is never a row with an invented id. K-1, B-3 and M-2 are promised only on that resolution.

**Tested on the entry paths that exist.** PR 1's cases run through the frontend's load as every verb does (load, sequence, mint, demand), not only through a bundle the test mints itself. One of them holds a qualified stdlib field and a multi-file seed, and asserts the stdlib row's universe is `StdlibAnalysis`, not its seed's display string (§ 3 case 12). C1's switch is tested through the actual sync-inference entry path, the frontend load: `two_owners_on_two_pools_infer_none` and the sync-inference shadow are driven from `Snapshot::load`.

### Invariants

1. **One row per static instance of each construction template.** A construction template is one literal of the root declaration. Its static tower is the root's params fields as that literal builds them (its overrides, else the declaration's defaults) and, recursively, their params fields, each with the literal that built it. Two literals of the root are two templates even when their trees agree, so a key's origin decides every row under it: `App { gw: Gateway { router: RouterV2 { } } }` and `App { gw: Gateway { router: RouterV3 { } } }` give `gw.router` two keys, each with one `realizes`, one `literal` and one set of generic arguments. A field whose initializer chooses among literals (an `if` or `match` whose arms are literals) gives one step per literal, each with its `alternative`, and every row on or under it is `guarded`. An initializer whose literals cannot be enumerated (a call) is a hole on that field. An adapter literal in the root's `bindings { }` is an origin of its own (`Origin::Binding`), not a row of any construction template, because the bindings prelude builds it once however often the root is constructed. Keys are unique. Replica rows are exactly `0..K` for a `replicas = K > 1` entry, and every row nested under replica `i` carries `Some(i)`.
2. **Nested rows inherit.** A row's domain is its owner's domain unless the row is decided by an `Entry`, which only a root field can be (rule 1, sem:3216), or by a `Binding`.
3. **Pinned domains are per instance.** Two rows share a pinned domain only if one is nested under the other, so each replica is its own domain. Pool domains are per name: every row on pool `X` shares one domain, and that domain carries at most one affinity (rule 16, sem:3400).
4. **The root is `lowering_root`, never `entry`.** The table describes what lowering deploys. Until L4 has lowering read the entry, that root can be a module-nested `main` that is not the entry (`ent:96-113`, `ent:185`). `RootRow::is_entry` records the difference, so a consumer bound to the entry (rule 9's closed world, `--env`, `--matrix`) can tell. An imported `main` is never the root: a seed whose only `main` is imported has an empty table.
5. **Every root entry decides exactly one field family in each construction template** (rule 18, sem:3438): the field's one row, or, under `replicas = K`, its K replica rows, all decided by the one entry. An entry that decides none is a hole with a stated policy. It is never dropped silently.
6. **Unknown is a hole, not a default** (registry, `placement` § Missing data). A field whose realized declaration cannot be resolved still gets its domain from its owner or its entry. Only `realizes` holds the hole.
7. **F.38** (dec:3219). No column of the table changes who receives a message. Domains decide threads and routes, never delivery sets.

### Templates, occurrences, incarnations

Three identities are easy to run together, and the table holds only the first.

- **A template** is static: an origin (a construction literal of the root, or an adapter's binding entry), a path of steps from it, a replica index. It is what `InstanceKey` names, and what every safety rule and lowering decision is stated over.
- **An occurrence** is one execution of a template's literal: `fn main() { App { }; }` has one, a factory called in a loop has as many as the loop runs. The table does not key occurrences. It counts them, through `Construction::bound` and `DynamicSite::bound`, and every occurrence of a template has the same domains, decisions and realized declarations, because they are all decided statically.
- **An incarnation** is a runtime identity: which live object an observation came from. It belongs to the runtime and the observation stream (P2 and the model's arrangement read it), never to the table. A consumer that needs one joins it to a template; the table never mints one.

### How live bounds combine

A count over the table (threads, pinned anchors, arenas) is a sum over the templates that can be live at once. The rule:

- **Across templates, sum.** Two construction templates, or two dynamic sites, can be live together. The table proves nothing about disjoint lifetimes, so a program that tears one root down before building the next is still counted as if both were live, and the budget says so.
- **Within one template, multiply by its bound.** A template counts once per occurrence that can be live: `Once` is 1, `AtMost(n)` is `n`, `Unbounded` makes the whole count an uncertainty with the template's reason.
- **Across the alternatives of one step, take the maximum.** One occurrence takes exactly one alternative, so its guarded subtrees are exclusive. The maximum is over each alternative's own subtree count.
- **Replicas are already rows.** `replicas = K` is K rows in the template, so it is counted by the sum over rows and never multiplied again.
- **A binding origin is `Once`.** The bindings prelude runs once per process, in `lower_program` (`cg:9813`), so an adapter's subtree is counted once and never multiplied by any root construction's bound.
- **Count domains, not rows.** A count of threads is over distinct `DomainId`s, each counted in the one scope that creates it (§ 2.8), so a domain is never counted twice by two terms. Domains anchored under the exclusive alternatives of one step combine by the maximum like any other subtree count (§ 9, checkpoint 4).

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
| resource budget | threads and pools (`rb:152`) | threads = the distinct pinned domains, partitioned by the origin of their anchor: root-created anchors under their construction's bound, adapter anchors once (§ 2.8); pools = non-main pool domains |

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
| K-5 | sync inference (`si:123-216`) reads the type map for the **enclosing** locus's pool (`si:164-166`) and takes the union of the pools that access each `@form(hashmap)` type (`si:197-206`) | first-wins per enclosing type | per instance | agreement **only if** the switch computes accessor domains per accessed instance and then takes the maximum discipline over a type's instances. A plain union of instance domains per type would infer `serialized` where today's answer, `none`, is right: two `@form` fields, each touched by its own owner on its own pool, need no synchronization. The switch is C1's, after the mint (§ 1, the stage), and its commit pins that | ty:1131-1137 (the 2026-07-15 handoff note at `chk:4773-4787`) | `sync_inference` shadow, driven from the frontend load: the inferred discipline is identical for every `@form(hashmap)` locus in the corpus, plus `two_owners_on_two_pools_infer_none` |
| K-6 | qualified, aliased, generic, module-qualified and contract-typed fields that are **nested** | aliased and contract-typed: no row (`TopSymbol::Type` and `Interface` are not `Locus`). Generic: the template name (`Cache`), not the monomorph | rows with `realizes` resolved | known old bug, **by reading**; § 3's cases confirm before the switch | ty:137 (aliases are transparent in params); ty:743-747 (generic loci); sem:506-521 (a contract-typed field holds the impl that was built) | § 3 cases 5-8 |
| K-7 | the entry itself: a `main locus` with only claims (no constructing literal), a library seed checked alone, a `fn main` that builds loci by verb | no row for the entry; the map is seeded from the root's type and its fields only | `Origin::Entry`, bound `Once`: a main locus's tower enumerates under it, `fn main`'s literals are constructions under it, a library seed alone roots at its lowering root | agreement for the checker's answers; the rows are new coverage (§ 10, 1) | `ent:96-113`; sem:3411-3437 | § 3 cases 13-15 |
| K-8 | a held instance (`Hole::Reuse`) whose type has locus fields, under a holder placed off main | the declared type's subtree under the holder, in the holder's domain | the source's actual rows projected under the held row, never the declaration's defaults, inherited, each row's `built_by` naming its own source row; no domain question reads the source's rows | agreement where the source keeps the defaults (§ 10, 8). Where the producer cannot link the source (a parameter, a name bound twice), the source template stays on main beside the held row: a correction | — | `placement_table.rs` · `a_held_instances_subtree_lives_in_its_holders_domain` (§ 3 case 16), `a_held_subtree_projects_its_sources_overrides` |
| K-9 | a held instance whose source the producer cannot link (a parameter, a name bound twice) | the declared type's default subtree under the holder, in the holder's domain | the `Reuse` hole on the held row and no subtree | known old bug → correction (§ 10, 9): the legacy fabricates rows an override may contradict. A consumer switch treats the hole as unknown: it disables a proof or an optimization and never defaults to main or to pinned | — | `placement_table.rs` · `a_held_row_whose_source_is_unlinked_asserts_no_subtree` |

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
| B-7 | an adapter locus in `bindings { }` (its instance is pinned on its own thread) | `SameThread` | a pinned domain anchored at the adapter | known old bug → approved correction, **conditional on § 4's adapter test** (an adapter publishing to a main-thread subscriber, the receiving thread recorded). Corrected in the graph-switch PR (§ 10, 2) | rt:364-371; GH #1032's shape | `bus_graph.rs` · `an_adapter_is_not_same_thread`; § 4's adapter case |

**Readers that are not this row.** `has_offthread_placement` (`bg:691-722`) answers "does any entry put a thread off main". It is one term of codegen's `program_has_offthread` (`cg:1149`, `cg:1223-1228`). It walks every block, so an imported root's entries count, which is conservative and safe. The table narrows it to the root's rows: an intentional correction, and **E2's** to switch (plan E2; registry lines 883-884). The narrowing must be pinned with the runtime flag. `g_bus_has_pinned` stays 0 when the only pinned entry belongs to an imported root, because no thread is spawned.

### 2.3 The ownership graph: `collect_placements`

**The question.** The same per-type label (`og:792-854`, a verbatim copy of `bg:621` with `named_type` at `og:856` for `single_named_type`). It is read by `classify_edge` (`og:746-783`), which compares the owner's label with the enclosing locus's label and gives `SameTower`, `CrossPool` or `Open`. Lowering's bubble plans read the edge class (`cg:3408-3451`). `SameTower` allocates the child in the owner's arena from the enclosing thread. `CrossPool` marshals an asynchronous post to the owner's thread, where the instance is legal only as a bare statement (sem:308-317).

**Not in the shadow.** The fixture compares the checker with the bus graph only. This copy has the same divergences, but its consumer differs and the stakes are higher: an arena is per thread and unsynchronized.

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| O-1 | an enclosing locus nested under a pinned root field instantiates `I { }`, and `I` bubbles to the root, which accepts `I` | both labels `SameThread` → `SameTower`: the child is allocated in the root's (main's) arena **from the pinned thread** | `CrossPool` | known old bug → approved correction (B-1's principle reaching this consumer). The site becomes fire-and-forget: a value use that compiles today becomes the existing compile error. J commit | sem:308-317; rt § Interest-based ownership; sem:3511-3542 | `ownership_graph.rs` · `a_nested_enclosing_under_a_pinned_field_is_cross_pool`; an ASan run of the same program in `ownership_bubble_crosspool.rs` |
| O-2 | an enclosing locus nested under a pool-`io` root field, bubbling to an owner on pool `io` (or to that root field itself) | enclosing `SameThread`, owner `CrossPool(io)` → `CrossPool`: a post to the thread the code already runs on, and a value use is refused | `SameTower` | known old bug → approved correction; it **admits** a value use that is refused today | as O-1 | `ownership_graph.rs` · `a_nested_enclosing_on_its_owners_pool_is_same_tower` |
| O-3 | one enclosing type with instances in two domains (under `main` and under pinned `p`) | first-wins: one class for both | the instances disagree | **unresolved disagreement** → § 7 U-1. Lowering emits one code path per `(enclosing type, child type)` (`cg:3408-3423`), so per-instance classes cannot be expressed. `Open` is not a safe answer: `bubble_plans` makes no plan for an `Open` edge (`og:258-275`), so the site falls through to transient handling, which changes who accepts, retains, counts, releases and tears down the child. The owner is resolved either way; a second instance of the enclosing type elsewhere does not stop the accepting ancestor owning it. Recommendation: keep the resolved owner apart from the delivery mechanism and never fall back to transient (§ 7 U-1) | — | `ownership_bubble_mixed.rs` (§ 7 U-1's tests) |
| O-4 | `Pinned == Pinned` for two different pinned instances | `SameTower` | `CrossPool` | unreachable today: rule 6 (sem:3232) forbids `accept()` on a pinned locus, so an owner is never in a pinned domain of its own. The table states it as a law rather than relying on the coincidence | sem:3232 | `placement_table.rs` · `no_accepting_owner_is_anchored_in_a_pinned_domain` |
| O-7 | an adapter locus in `bindings { }` as the enclosing or owning side of an edge | `SameThread` → `SameTower` | the adapter's pinned domain: `CrossPool` | known old bug → approved correction, as B-7: conditional on § 4's adapter test, corrected in the graph-switch PR (§ 10, 2) | as B-7 | `ownership_graph.rs` · `an_adapter_edge_is_cross_pool` |

### 2.4 The model: `PlacedIn`

**The question.** The arrangement per instance, with replicas: `LocusInstance`, `Realizes`, `PlacedIn`, `Owns` and `ThreadDomain` (`mb:2934-3275`). It is the closest of the eight to the table. Instances are keyed by path (`App.f`, `App.f[2].k`), replicas fan out (`mb:3061-3110`), nested rows inherit (`mb:3092-3098`), and domains are named `main`, `pool:X`, `pinned:<path>` and `binding:<topic>` (`mb:3092-3098`, `mb:3199-3202`).

**What it computes.** The root is the **first** `is_main` in `ast.loci` (`mb:3124-3127`), in bundle program order with modules flattened and imported `__lib_` mains included. Declarations come only from the user seeds (`mb:2954-2979`; `mb:2612` says `ast.loci` covers only the user seeds). Field types are read by last segment (`mb:3025-3027`), and a field whose type is not a user locus is skipped (`mb:3057`). `affined_to` is in the model's relation set (`hale-model/src/application.rs:152`), but the builder never pushes a row: the column is always empty.

**Three contracts, kept apart.** Filtering out stdlib instances does not by itself keep the model's ids where they are, because M-1, M-3 and M-5 move instances too. Each contract is stated on its own, with what is expected to change.

1. **Shape identity.** The arrangement is outside the model's shape half (`mb:2934-2940`), so no `PlacedIn` change moves `shape_hash` unless it adds a declaration, topic or binding to that half. P1's projection adds none. Projecting stdlib instances would: their `Realizes` targets are not among `entities.loci`, which hold user declarations only (`mb:2612`), and adding them is shape. Contract: `shape_hash` identical over the corpus, `tests/hale` and `dna/`.
2. **Observation entity ids and their digest.** `obs_ids.rs` stamps subjects, locus declarations and bindings (`obs_entity_ids`, `obs_ids.rs:93-130`), never instances, so adding, removing or renumbering a `LocusInstance` leaves them alone. Bindings are the exception that reaches here: the arrangement reads them from the root it picked (`mb:3166-3170`). M-1 re-roots the arrangement, so for a bundle whose arrangement was rooted at an imported main, the imported root's binding rows go and the seed's root's come, and the `BindingId`s and `entity_id_digest` change with them. Contract: ids and digest identical over the corpus except M-1's pinned cases, where the change is listed.
3. **Arrangement-instance correspondence.** `LocusInstanceId` is the index in path-sorted order (`mb:3222-3231`), so any added or removed path renumbers every later id. P1 makes no claim that numeric ids stay stable. Expected changes, each pinned in `model.rs` by a table of path → old id → new id built from both builds:
   - **M-1** changes the root, so the root row and its whole subtree change paths, and every id can move.
   - **M-3** removes the user `Listener` wrongly arranged at a stdlib-typed field, with every row it nested, and the later ids shift down.
   - **M-5** adds the aliased and contract-typed user fields that were omitted, with their nested rows, and the later ids shift up.
   - **Construction templates.** A `LocusInstance` path has no construction component (§ 1), so one arrangement row corresponds to the table's keys with the same steps across every template. The row is exact when those keys agree on `realizes` and `domain`, and otherwise it is a model hole naming the templates that disagree.

   A consumer that holds an instance across builds joins by path, never by index. If numeric ids must stay stable for a consumer, that is a model change of its own (ids derived from the path rather than from its position in the sort), not a P1 projection.

| id | shape | legacy | table | class | spec witness | pinned by |
|---|---|---|---|---|---|---|
| M-1 | a bundle holding an imported `__lib_App` whose program sorts before the seed's own | the arrangement is rooted at `__lib_App` | rooted at `lowering_root` | intentional correction (fixture F-R's principle) | `ent:1-31` | `model.rs` · `the_arrangement_is_rooted_at_the_deployed_main` (`--dump-model` pinned) |
| M-2 | stdlib-typed fields (`std::io::tcp::Listener`; `BytesBuilder` nested) | no instance | rows exist in the table | **unresolved disagreement** → U-4. Projecting them adds stdlib declarations to the shape half (contract 1). Recommendation: a user-only projection in P1, exposed as partial coverage (each omitted row a model hole), with the full table authoritative for safety and no claim that ids are stable (contract 3) | — | `model.rs` · `stdlib_instances_are_holes_of_the_arrangement` |
| M-3 | a user `locus Listener` and a field typed `std::io::tcp::Listener` | the field is arranged as the user's `Listener` | `realizes` = the stdlib declaration | known old bug | registry § Invariants | § 3 case 4 |
| M-4 | `affined_to` | empty | one row per domain with an affinity | **unresolved disagreement** → U-5. It changes `--dump-model` and not the shape | sem:3400 | — |
| M-5 | generic, aliased and contract-typed fields | the template name, the alias name (no instance), or the contract name (no instance) | resolved | known old bug, **by reading** | ty:137, ty:743-747, sem:506-521 | § 3 cases 5-8 |
| M-7 | the entry, and `fn main`'s literals | no instance for the entry; the arrangement is the root's tree | the entry is `Origin::Entry` (bound `Once`); the arrangement path has no construction component (§ 1), so the rows correspond as for any template | agreement for the arrangement's rows; contract 3 applies to any path the entry adds (§ 10, 1) | `ent:96-113` | `model.rs` · `the_entry_adds_no_arrangement_row_of_its_own` |
| M-8 | a held instance whose type has locus fields (`App.h.roles.k`) | the declared type's subtree arranged under the holder, placed in the holder's domain | the paths of the source's actual rows projected under the held row, never the declaration's defaults; the source template's paths are not projected, since the instance is arranged where it runs | agreement where the source keeps the defaults (§ 10, 8) | — | `placement_table.rs` · `a_held_instances_subtree_lives_in_its_holders_domain` (§ 3 case 16), `a_held_subtree_projects_its_sources_overrides` |
| M-9 | a held instance whose source the producer cannot link | the declared type's default subtree arranged under the holder | no path below the held row | known old bug → correction (§ 10, 9). A consumer switch treats the hole as unknown: it disables a proof or an optimization and never defaults to main or to pinned | — | `placement_table.rs` · `a_held_row_whose_source_is_unlinked_asserts_no_subtree` |

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
| G-4 | adapter loci in `bindings { }` | pinned-equivalent with no placement entry | a row keyed under `Origin::Binding`, with `Decision::Binding` and a pinned domain anchored at itself | agreement. The row exists for C7's inline-adapter judgment and for R-6 | registry `law_backstops` | `placement_table.rs` · `an_inline_adapter_is_a_pinned_binding_row` |

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
| R-5 | the root built at several sites or in a loop (a factory called in a loop is legal, sem:3432-3436) | counted once | each construction template's threads × its `bound`, summed over the templates (§ 1, how live bounds combine); `Unbounded` renders as an uncertainty, and a ceiling against an uncertain count fails with that reason | known old bug → approved correction ("dynamic instantiation needs a bound or an uncertainty") | sem:3411-3437 | `resource_budget.rs` · `a_root_built_in_a_called_loop_is_uncertain` |
| R-6 | adapter loci in `bindings { }` (pinned-equivalent threads, `cg:10494-10508`) | 0 | 1 thread per adapter anchor, counted once whatever the root's bound | **unresolved disagreement** → U-3. The decision covers placement, not bindings; recommendation: count them, since they are threads | — | `resource_budget.rs` · `one_adapter_and_one_pinned_child_are_two_threads`, `an_adapter_counts_once_under_several_root_constructions` |
| R-7 | `cooperative` or `pool = main` spelled | `main` is counted as a pool, and only when spelled | `main` adds no worker | **unresolved disagreement** → U-2. Recommendation: pools are worker pools; `main` is rendered on its own line. A declared ceiling can only loosen | rt:354-362 | `resource_budget.rs` · `the_main_pool_is_not_a_worker` |

**The thread count is a partition by construction scope.** Each pinned domain is counted once, in the scope that creates its anchor, and the domain table holds each domain once, so no domain is in two terms:

- **Root-created anchors.** A pinned domain whose anchor has `Origin::Construction(c)`: a root field placed `pinned`, one per replica. Counted under construction `c`'s bound, and summed over the constructions (§ 1, how live bounds combine).
- **Adapter anchors.** A pinned domain whose anchor has `Origin::Binding(e)`: one per adapter literal in the root's `bindings { }`. The bindings prelude builds it once, so it is counted once, and its bound is never the root's.
- **Nested rows add nothing.** A row under either kind of anchor inherits its domain (invariant 2) and spawns no thread.

So `threads = Σ_c bound(c) × |pinned anchors under c| + |adapter anchors|`, every term over distinct `DomainId`s. Two tests pin the partition. `one_adapter_and_one_pinned_child_are_two_threads`: one adapter in `bindings { }` and one root field placed `pinned` give 2, not 3 (the old formula counted the adapter's pinned domain among the pinned domains and again as an adapter binding). `an_adapter_counts_once_under_several_root_constructions`: two construction templates, one `AtMost(3)` and one `Once`, each with one pinned field, and one prelude adapter give `3 + 1 + 1 = 5`, not `(3 + 1) × 2`.

Threads the runtime spawns outside placement are not placement facts. Examples are binding reader threads (one domain per binding, `mb:3199-3202`) and the serve threads of the GH #233 stdlib transports (`cg:10494-10498`). The budget renders them as an explicit "not counted" line until P2's binding rows can count them.

### 2.9 Placement readers outside the eight rows

| reader | what it asks | who switches it |
|---|---|---|
| sync inference (`lib.rs:438-454`, a single-program bundle with no identities, run per program before the sequence and the mint, `fs:590-596`) | the type map, for the enclosing pool | C1, stacked on P1's producer commit: inference moves after the mint and reads the table (K-5's rule) into C1's effective-discipline column (§ 1, the stage). Until then the pre-mint map stays as its own typed contract, verified against the table by a shadow column. The test-local copy (`si:505-563`) goes with the switch |
| `has_offthread_placement` / `program_has_offthread` (`bg:691`, `cg:1149`, `cg:1223-1228`) | any thread off main | E2 (§ 2.2) |
| the blocking check's three derivations | per root field: pool, `async_io`, pinned | E2 |
| `placement_implied_diags` (`effects.rs:538-590`) | which types run on an `async_io` pool (root fields, single segment, every top-level `is_main`) | registry line 418: reads `lowering_root`, then the entry at L4. When it moves to the table, its set gains nested members of `async_io` pools: classified then, not in P1 |
| LSP `hale/placement` (`hale-lsp/src/lib.rs:2409-2450`) | the root's params and entries, for display | registry line 419: L4. Showing per-instance domains is optional and not in P1 |
| ownership decisions, `placed_fields` (`ownership.rs:1751-1760`, read at `ownership.rs:3095-3130`) | whether a field has an entry (`Decision::Placement`) | a syntax fact; stays |
| `check_pool_affinity` | validation of every declared block | stays (§ 2.1) |

## 3. Coverage the shadow must run

**The shadow loads through the snapshot.** Today's shadow parses each program alone, with no stdlib merge, no imports and no desugar (`shadow_placement.rs:22-25`). The corpus it walks (`hale-corpus/src/lib.rs:76-152`) is the example fixtures, the CLI fixtures, the stdlib and the Rust-embedded programs. That excludes `tests/hale` and `dna/`. P1's oracle is "the corpus, `tests/hale` and the DNA seeds", and imported roots exist only with imports. So the P1 shadow loads every seed through `Snapshot::load` (seeds under `tests/hale` and `dna/`; corpus programs as single-file seeds), demands `placement`, and compares the table's projection with **every** legacy producer: the checker's map, `enclosing_field_placement` at each receiver, the bus graph, the ownership graph, `PlacedIn`, the desugar's set, `DeploymentPlan` and the budget. Each comparison is a column of the fixture. Every divergence row cites an id from § 2. The gate stays at zero unexplained and zero stale.

**Measured by the producer** (§ 10): the shadow loads 2026 seeds and compares 48,914 rows, of which 13,185 diverge. 13,071 of those were classified before the rulings, and the rulings classify the other 114 (23 entry roots, 91 adapters).

**The cases.** Each is a unit test of the table in `crates/hale-types/tests/placement_table.rs` (a new file, which joins its area binary in `Cargo.toml`). Each also runs through the shadow, so its legacy answers are recorded beside the table's.

1. **Two instances of one type in different domains.** `a: W` pinned, `b: W` on pool `io`, `c: W` default, each nesting `k: K`. Six rows, three domains for `W`, and `K` inheriting each. Legacy: one domain per type (K-3, B-6, O-3).
2. **Nested inheritance, three deep.** A root field on pool `io`, nesting `M`, nesting a subscriber `S`. `S` is on `io` (B-1, O-2). The same tree under `pinned` gives `S` the pinned domain of the root field's instance, and under `replicas = 3` it gives three domains with `[i]` in each nested key.
3. **Overrides.** `App { gw: Gateway { router: RouterV2 { } } }` against `Gateway`'s default `RouterV1`, and a contract-typed param (`j: Counter = Churner { }`). `realizes` names the literal's declaration, `literal` names the override's site, and the domain is inherited (M-5, K-6; sem:506-521, sem:4086-4093).
4. **Qualified types and a last-segment collision.** A root field `l: std::io::tcp::Listener` on pool `io`, beside a user `locus Listener` used elsewhere (K-1, B-3, B-5, M-3).
5. **Aliased types.** `type Held = Holder;` with `h: Held = Holder { }` placed `pinned`, and an alias nested one level down. Expected: one row realizing `Holder`. The case first pins whether rule 3 admits the field: `chk:9822-9833` reads the resolved `Ty`, which by reading is `Holder`. If rule 3 admits it, G-3 and K-6 are confirmed; if it refuses, the rows reclassify as a checker gap (K-6, G-3, M-5).
6. **Generic specialization.** `c: Cache<Int, String> = Cache { cap: 2 }` placed `pinned` with a subscription, and `d: Cache<Int, Int>` on pool `io`. Two rows, one template site, two substitutions, two `lowered` names. Confirms G-2 by building both (a compile-only check that `pinned_locus_types` holds `Cache_Int_String` after the switch).
7. **Module-nested and module-qualified.** A root field `k: m::K = m::K { }`, and a seed whose only `main` is inside `module app { … }`. `RootRow::is_entry == false`, rows present, because lowering deploys it (`ent:96-113`; the nested-main transition test `nested_main_transition.rs`).
8. **Imported roots.** A seed importing a library whose `main locus` has pinned entries, with and without the seed's own `main`. The import gives no rows (K-2, B-4, M-1, R-4). With the seed's main, the rows are the seed's. Run from `tests/hale` through the snapshot, since a parse-only shadow cannot see it.
9. **Adapter bindings.** An inline adapter in `bindings { }`: a row under `Origin::Binding` with `Decision::Binding` and a pinned domain anchored at itself, beside a root field placed `pinned`: two pinned domains, disjoint. The same program with the root built at two sites keeps one adapter row and gains a second root field row (G-4, R-6, § 2.8's partition).
10. **Dynamic sites.** A locus literal in a root method in a loop, an `accept`ed child, and the root built by a factory called in a loop. These produce `DynamicSite` rows with the enclosing domains and the bound, and the factory's literal is one construction template with an `Unbounded` bound (R-5).
11. **Two constructions of one root with different nested overrides.** `App { gw: Gateway { router: RouterV2 { } } }` in one function and `App { gw: Gateway { router: RouterV3 { } } }` in another, with `gw` placed `pinned` and `RouterV3` generic. Two `Construction` rows, and two keys for each of `gw` and `gw.router`, which differ only in their origin: each `gw.router` row has its own `realizes`, `literal` and `args`, and each `gw` its own pinned domain. A third construction `App { gw: if c { Gateway { router: RouterV2 { } } } else { Gateway { } } }` gives two `alternative` steps at `gw`, every row under them `guarded`, and its budget contribution is the larger of the two subtrees, not their sum. The case first pins whether rule 3 and the checker admit a literal-armed `if` as a param initializer; if they refuse it, the third construction is dropped from the case and `alternative` stays for `match` arms only if those are admitted, else it is removed from the schema.
12. **Two universes, one numeric id** (the acceptance test for § 1's input universe). A seed with a user locus declaration minted with the same numeric `SiteId` as a stdlib locus in the analysis copy. The seed is padded with user locus declarations until one lands on a stdlib locus's id, as the re-review's probe did (`0:99` with 500 declarations), and the case first asserts the collision is present, so it cannot pass vacuously. The root holds a qualified stdlib field (`l: std::io::tcp::Listener`) beside user fields, one of them typed by the colliding user locus, and the stdlib locus has a params field with a default literal. Asserted:
    - the two declarations' `DeclRef.site` have equal `id` and different `universe`, and their canonical declaration keys stay distinct: two `realizes`, two entries in a set keyed by `DeclRef`, and no row of one answering for the other;
    - default-literal provenance resolves into the right store: the stdlib locus's default literal through `provenance` into the analysis copy, naming a stdlib span, and the user locus's into the snapshot, naming the user file;
    - the lowering correspondence resolves each relevant site (both declarations, both default literals, the root's field literals) into the merged mint exactly once;
    - checking the seed's display string alone is insufficient, and the case says so beside the assertion: `seeds[id.seed] == STDLIB_SEED` holds only when the store is already known, and asked of the snapshot, seed 0 is the first user file. The case asserts the universe.
13. **A claims-only `main locus`.** A `main locus` with claims and no constructing literal. The table has an `Origin::Entry` key, bound `Once`, and the locus's tower enumerates under it (K-7, M-7; § 10, 1).
14. **A library seed alone.** A seed whose only `main locus` is a library's, checked alone. It roots at its lowering root: the entry key is that root's, and the rows are present (K-7).
15. **A `fn main` building loci by verb.** `fn main` constructs loci by literal and by verb. Each literal is a construction under `Origin::Entry`, and the loci it builds are rows under it, not dynamic sites (K-7).
16. **A held instance under a holder placed off main.** `h` is placed `pinned`, `fn main` does `let r = Roles { }; App { h: Holder { roles: r } }`, and `Roles` has a field `k: K`. The held row `h.roles` is a `Reuse` hole naming `r`, in `h`'s domain. `h.roles.k` is a row under it, inherited, and both name the rows of `fn main`'s `Roles { }` template as `built_by`. The legacy checker and the arrangement agree with the table (K-8, M-8; § 10, 8).

## 4. The runtime test the correction needs

**What the reading predicts.** The fixture's deferred note says to "probe obs_intra_tree_publish … for a direct call across threads". That program cannot show it. Its only publisher of `ToChild` is the pinned owner `Pub` itself. The desugar rewrites that publish into a call on `Pub`'s own thread (`ds:1100-1129`), which is correct, and its other subject has a pinned publisher, which fails the gate's publisher leg (`bg:498-505`). Reading the registration path shows a hazard that is **wider than the direct-call gate**:

- **The registration route.** A nested subscriber registers through the cooperative branch with no mailbox (`inst:3808-3816`, `None`). The only route it gets is a pool: `current_cooperative_pool` at lowering, or `lotus_coop_pool_current()` at runtime (`brt:434-479`), and on a pinned thread or main that is null. A pinned owner has a mailbox (`inst:3930-3990`), but its nested children do not use it.
- **A nested subscriber under a pinned owner.** Its cells go to the program-wide queue, which drains only on its owner thread, main (`A:6639`, `A:7311`). By reading, its handler runs on **main** in every flavor. That breaks ty:1123-1125 and races with the pinned owner's own code.
- **A nested subscriber under a pool owner.** Its entry carries the pool, so the deferred path is right. But when the gate also passes (all labels `SameThread`, a quiet handler, a flat payload, a single handler), lowering picks the direct-inline flavor. Its accessor returns NULL for an entry with a pool (`A:19841-19852`), so by reading the delivery is **dropped**.
- **A nested publisher under a pinned owner, with a quiet subscriber on main.** The publisher leg reads `SameThread`, so the handler runs synchronously on the pinned thread (`A:19776-19777`). That is GH #253's race in a new shape.

Deliver records are no witness here. `lotus_obs_bus_deliver` is called at post time on the **publisher's** thread in every flavor (`A:19780`, and the fanout sites listed at `A:11053-19641`), so the ring a record lands on names the publisher. The test needs a witness for the thread that runs the handler.

**The test.** `crates/hale-codegen/tests/nested_offthread_delivery.rs`, joining the `replay_obs` area (`tests/replay_obs.rs`, beside `obs_intra_tree_publish`), built with `harness::unique_bin` and `build_opts::options()`.

**The witness is the thread id, recorded inside the handler.** Every receiving handler calls `pthread_self()` itself, as `bus_adapter_inbound::adapter_subscription_runs_on_the_adapters_own_thread` already does (`@ffi("c") fn pthread_self() -> Int`, `bus_adapter_inbound.rs:392`, compared at `:414`). The fixture is synchronized throughout, so no assertion rests on timing and nothing reads another domain's fields:

- **Handshake.** Each anchor a case names records `pthread_self()` first thing in its `run()` and publishes `Ready { who, tid }`: the pinned `Owner`, the pool owner (one worker per pool, so its `run()` is on the worker), and the root for main. A `Collector` on main gathers them, and the `Feeder` publishes only after the collector has every expected `Ready`.
- **Report.** Every receiving handler publishes `Seen { who, seq, tid }` with its own `pthread_self()` for each cell it receives. The collector's handler prints, so its own subject is never a direct call and is drained on main.
- **Exact counts.** The collector asserts that each `seq` in `1..=5` arrives exactly once from each expected receiver, and that every `Seen.tid` equals the `Ready.tid` of the domain the case expects. It then asks each receiver for its count with a `Count` request that the receiver answers from its own domain, by message; the answer must be 5. No one reads `got` across threads.
- **Bounded deadlines, no sleep windows.** Every wait is a loop on a condition with a 10 s deadline on `std::time::monotonic_ns`. On expiry the program prints `TIMEOUT <what it waited for, how many it saw>` and exits non-zero. A dropped delivery fails by name; nothing depends on how long a sleep lasted.
- **Both arms.** Each case runs with devirtualization on and with `BuildOptions::no_bus_devirt = true` (`cg:649`), and asserts the same counts and the same receiving domain in each.

**The quiet variant.** The direct-call gate admits only quiet handlers, and any call makes a handler not quiet (`bg:751-766`, `handler_is_quiet` at `bg:827`). So a handler that calls `pthread_self()` never reaches a direct flavor, and the two hazards that live there (B's drop and C's synchronous call) would go unseen. Cases B and C therefore run two subscriber variants. The witnessing variant pins the receiving domain on the deferred route. The quiet variant (`self.got = self.got + 1; self.last = c.seq;`) reaches the gate. Its counts go through the same `Count` round trip, polled until 5 or the deadline. Its thread is pinned at plan level: its subject's flavor in the lowering view's `DispatchPlan` is not `static_direct`. That flavor is the only lowering that runs a handler on the publisher's thread (`A:19776-19777`), and on the deferred route the quiet handler runs where the witnessing one was seen to run (the gate's own soundness argument, `bg:737-749`).

The cases:

- **A — pinned owner, nested subscriber, publisher on main.** `Owner` is placed `pinned` and nests `Kid`, which subscribes to `Tick { payload: P }`; a root sibling `Feeder` on main publishes five cells after the handshake. Expected: five `Seen`, each with `Owner`'s `Ready.tid`, and `Count` = 5, in both arms. The reading predicts every `Seen.tid` is main's today.
- **B — pool owner, nested subscriber, publisher on main.** The same program with `Owner` on `cooperative(pool = io)`, a single subscriber type and a flat payload. Expected: the witnessing variant's five `Seen` carry the pool owner's `Ready.tid`; the quiet variant's `Count` is 5; both in both arms. The reading predicts the quiet variant's `TIMEOUT` with 0 seen in the devirtualized arm today.
- **C — pinned owner, nested publisher, subscriber on main.** Expected: the witnessing variant's five `Seen` carry the root's `Ready.tid` (main); the quiet variant's `Count` is 5 and its flavor is not `static_direct`; both in both arms. The plan assertion is compile-only and gates in CI.

**ThreadSanitizer is additional evidence, not the witness.** An `#[ignore]` arm in the shape of `form_hashmap_lockfree_tsan.rs` (`LOTUS_TSAN=1`) runs the quiet variants of A to C and asserts no `WARNING: ThreadSanitizer` line.

**A negative control proves the oracle can fail.** One fixture runs a handler on a known wrong thread: a bound adapter's subscription, which runs on the adapter's own `run()` thread (GH #1032's shape), checked against an expectation of main. The ordinary assertion must report the mismatch, and the test asserts that it does. Under the TSan arm, the same control adds an unsynchronized read of the handler's field from main, and the test asserts a TSan warning. So a clean TSan run is evidence, not silence.

The test lands in the J commit that switches the bus graph (PR 5 below). It is red on the parent commit, and the PR body quotes that red run. If A or B stays red after the label correction, the registration route is the remaining cause. The fix is in lowering: a nested subscriber registers with its tower's route, the owner's mailbox when the domain is pinned and the pool when it is a pool. The table gives lowering that domain per instance. Whether that fix belongs to the same PR is U-6, and what it has to order is in § 8.

## 5. Consumer switch order

Six PRs follow this document. Each consumer switches in a commit of its own, and each correction is a J commit of its own, with wording-pinned tests and the decision it applies named in the body. One PR is in CI at a time and at most one beyond (the WIP rule). Every PR body is the usual document: What lands, Architecture with the invariants kept, Tests, Deferred. A user-visible change has its fragment in `unreleased/<pr>.md` in a second push.

| # | PR | shape | carries | oracle |
|---|---|---|---|---|
| 1 | **The producer.** `crates/hale-types/src/placement.rs` · `placement_table`, `Snapshot::demand_placement` and a `FAMILIES` entry. The shadow rewritten as § 3 describes, with the fixture extended and every row cited by id. `placement_table.rs` with § 3's twelve cases. The registry names the producer. No consumer reads it. | S · Opus | the producer commit; the shadow commit; the cases commit; the registry commit | shadow: zero unexplained or stale over the corpus, `tests/hale` and `dna/`; `cargo nextest run --release --workspace`. **L1 stacks on this PR's producer commit.** `hale_types::lifecycle` (#1300) copies the key field for field, so when P1's type lands its `SourceSite`/`DeclRef` take `SiteRef` (with `Origin`, `Step.alternative` and `Template::Dynamic`'s literal), and no bare `SiteId` survives in a lifecycle row. |
| 2 | **The lowering view.** The desugar's read (D, agreement), then `DeploymentPlan` as the table's lowering view, with replicas folded in and the type sets from `realizes.lowered`. Each of G-2 and G-3 that § 3 confirmed lands first as its own J commit with a runtime test. | S, with J for confirmed G rows · Opus | registry: `collect_off_owner_thread_fields`, `collect_main_placement` and `DeploymentPlan` leave the legacy list, and the seam `collect_main_placement(` goes | dispatch-plan digest and build identity identical over the corpus; `IntraLocusRewrite` relation identical; `LOTUS_ASAN=1 … --test corpus_oracle -- --ignored`; the ownership-matrix sample |
| 3 | **The checker.** F.31 reads `domain` and `owner_relative` (K-1, J). Pinned-in-a-loop and entry-consumed read the root rows (S). Instance aliasing reads them too (intentional correction, J). `enclosing_field_placement` goes. `compute_pool_of_locus_type` stays only as the pre-mint map sync inference reads until C1 (§ 1, the stage). | J + S · Opus for K-1, Sonnet for the rest | the K-1 test; the shadow column comparing the pre-mint map's inferred discipline with the table's; `an_imported_seeds_aliasing_is_not_reported`; the registry's checker row narrowed to the pre-mint map, and the seam `compute_pool_of_locus_type(` narrowed to sync inference's one call; a fragment for the qualified-field diagnostic; sem § Placement block (rule 3 names qualified stdlib loci) and the docs chapter `docs/src/services/concurrency.md` | diagnostics identical over the corpus and `tests/hale` except the pinned K-1 cases |
| 4 | **`PlacedIn`.** The model's arrangement projected from the table: M-1 (J), M-3 (J), M-5 (J), and the M-2 and M-4 projections as U-4 and U-5 decide. | J · Sonnet after PR 3 | `--dump-model` pins; § 2.4's three contracts, each stated in the body: `shape_hash` identical, obs ids and digest identical except M-1's listed cases, and the instance-id remapping tables for M-1, M-3 and M-5; the registry row removed; a fragment | `hale check --dump-model` over the corpus differs only in the pinned cases |
| 5 | **The two graphs.** The bus graph (B-1, B-3, B-4, B-5: J), `check_bounded_bus` (B-2: J, a new error), the ownership graph (O-1 and O-2: J; O-3 as U-1 decides, with `ownership_bubble_mixed.rs`) and the registration route if U-6 puts it here. `collect_placements` and `collect_subscriber_placements` go. | J · Opus | § 4's test (red on the parent, quoted); the B and O tests; an ASan run of the ownership cases; registry rows removed; a fragment naming the dispatch-plan digest moves (a recording of an affected program from before this PR no longer admits for replay); sem § Nested instantiation, rt § Placement classes (subscriptions follow the tower), dec F.38's note if wording changes; the docs chapter | suites; `ownership_matrix` sample; `LOTUS_ASAN=1` corpus oracle; `cargo test -p hale-cli --test dna_native_suite`; `hale test tests/hale` |
| 6 | **The resource budget.** R-1, R-4 and R-5 (J), R-2 and R-3 (pinned agreement), R-6 and R-7 as U-3 and U-2 decide. | J · Sonnet | `resource_budget.rs` tests; `spec/verification.md:2284-2292` and `docs/src/verification.md:154` (what is counted, the uncertainty, the "not counted" line); `notes/resource-budgets.md`; the registry row removed, which closes `placement` × 8; a fragment | `--dump-resource-budget` over the corpus differs only in the pinned cases |

**PR 5 as it lands (P1 3 of 6).** Four commits, in this order. (1) § 4's test, `nested_offthread_delivery.rs` in the `bus_topics` area, measured on its parent: A's and B's nested receivers run on **main** in both arms and both variants, and C's and the adapter case's are received on main; the quiet receiver's subject is `static_direct` in all four. B's registration carried no pool, so the reading's predicted drop did not happen: a nested instantiation starts with no current pool, so its registration took the registering thread's (none, on main). The TSan arm is not written. (2) U-6, the registration route: a pinned anchor whose tree subscribes (the lowering view's `route_anchors`, read from the table) gets a mailbox, created before its params are initialized; every subscription registered in its params, at any depth, carries it; a root field on a worker pool publishes its pool by name the same way; the join retires what is still routed to the mailbox before destroying it. A, B and the witnessing variants go green in both arms. The quiet variants of A and B stay red in the devirtualized arm, now as a drop: the legacy gate still bakes their subject as the direct-inline call, whose accessor skips an entry with a route. (3) The bus graph reads the table: `PlacementTable::domains_by_type` gives each declaration, by lowered name, the domains of its rows (a held instance's source rows skipped) and of its dynamic sites, unknown where a site's domain is unknown or where a template the entry builds may be an unlinked held row's source (K-9); `Placement` is that set's label (`Unknown` joins the enum: never main); the gate needs every publisher and subscriber type `SameThread`; `check_bounded_bus` reads the same labels, lazily, and refuses `Pinned` and `CrossPool` only (B-2); `collect_subscriber_placements` and the legacy-against-legacy shadow are gone, and the table's shadow has no bus column (109 B lines went to agreement). B-1, B-3, B-4, B-5, B-6 and B-7 are pinned in `bus_graph.rs`, B-2 in `placement.rs`; § 4's quiet flavors are `static_bucket` in all four cases and its `KNOWN_OPEN` tables are empty. A bundle no snapshot holds (the tests' entry, the artifact's bundle entry) gets its table from `placement::bundle_placement`, which mints a copy. Over the corpus, `tests/hale` and the DNA seeds, check, topology and every built output are identical; one `--dump-model` line moves (`dna/tests/workflow_recovery_test.hl`: `dna.execution.refused` `static_direct` → `static_bucket`, its publisher `Executions` also built in the imported `Dna` that seed never instantiates, a dynamic site of unknown domain, § 8). (4) The ownership graph reads the table: each bubbling edge pairs every row of the enclosing locus with the row that owns it (the nearest above it realizing the owner) and every other instance (a literal in a body, a row under no owner row) with every domain the owner runs in; `SameTower` when every pair shares its domain, `CrossPool` when none does and the owner has one domain, and the new `Mixed` otherwise or where an instance runs where the table cannot say. U-1 as decided: a `Mixed` edge keeps its resolved owner; for a singleton owner (a `main locus`, on main) a bare `I { };` tests `lotus_on_main_thread()` at the literal and lowers both arms, the same-tower bubble and the cross-pool post; a value use, or a non-singleton owner, is refused at the literal with `CodegenError::UnsupportedAt`, naming every enclosing instance and its domain; no `Mixed` edge is lowered transient. `collect_placements` closes, and the table's shadow has no ownership column. O-1 (nested under a pinned field: `CrossPool`, the child posted) and O-2 (nested under the owner's own pool: `SameTower`, the owner threaded) are pinned in `ownership_graph.rs` with the per-instance pairing (two owners in two domains each owning their own nested child: `SameTower`), O-1 also under ASan in `ownership_bubble_crosspool.rs`; O-3 by `ownership_bubble_mixed.rs` (both instances accepted, counted, retained, born on the owner's thread and dissolved once, in both arms and under ASan, the transient control and the located refusal); O-4 by the table's laws test; O-7 moves no edge: an adapter's own tower bubbles to it `SameTower`, and a `bindings { }` entry is no instantiation edge, so nothing bubbles past an adapter. Over every seed, 19 bubbling sites, and the only one whose class moves is `ownership_bubble_mixed.rs`'s own. Found on the way: the intra-locus rewrite (case b) turns a root's publish into a direct call to its one field of the subscriber's type, dropping the delivery to every other instance of that type (a nested one included), a type-level read in the desugar that its switch to the table (PR 2's row D) owns; and a child born by a cross-pool post leaks its arena record (216 bytes) at teardown under ASan, with or without a mixed edge.

PR 2 precedes the J PRs for two reasons. It is the only one whose oracle is "nothing executes differently". And PR 5's registration route reads the table's per-instance domain through the lowering view PR 2 builds. E2 (`has_offthread_placement` and the blocking derivations), C1 (sync inference after the mint, K-5) and C7 (the placed-`Unknown` judgment) need only PR 1.

## 6. What the fixture and the registry contradict

- **The registry's checker row undercounts.** It says the checker holds four derivations, "two more inline in the blocking and single-thread checks". Measured: three inline in the blocking check, none in the single-thread check, and four more readers elsewhere (pinned-in-a-loop, entry-consumed, instance aliasing, affinity validation), plus `check_bounded_bus` reading the bus graph's map (§ 2.1).
- **The fixture's deferred probe names a program that cannot show the hazard** (§ 4). By reading, the hazard is also wider than the label the fixture classifies. The nested subscriber's registration carries no route, so a pinned owner's nested subscriber is delivered on main in every flavor, and a pool owner's can be dropped by the direct-inline flavor.
- **The fixture classes the nested rows `known-old-bug`, and the decisions call them approved corrections.** Both are right at different ends. The bug is in the bus graph, and the correction is what its consumers see when they switch. This document uses "known old bug → approved correction" for those rows.
- **The ownership graph's copy is in neither the shadow nor the fixture.** The registry calls it verbatim, and it is: the logic is identical. But its consumer allocates across arenas, so its divergences need rows of their own (O-1 to O-4). PR 1's shadow adds its column.
- **The budget's legacy row says "counts placement entries, ignores replicas".** That is true, and it also counts every locus's block (imported roots too), reads only one module level, counts `main` when spelled, and never counts adapter threads (§ 2.8).
- **The registry lists `model (placed_in, affined_to)` as a consumer, but the builder never produces `affined_to`** (M-4).
- **The shadow's corpus excludes `tests/hale` and `dna/`, and parses each program alone.** So it cannot see imported roots, which the oracle requires (§ 3).

## 7. Decisions for the driver before the pane starts

Each answer below is **proposed, pending the owner**. Until the owner confirms one, its rows stay `unresolved disagreement` and their consumer does not switch. Once confirmed, an answer's rows become intentional corrections under it, each a J commit naming the decision.

- **U-1 (O-3). Proposed, pending the owner: reject the transient fallback.** Not `Open`. An `Open` edge gets no bubble plan (`og:258-275`), and a site with no plan is lowered as a transient instantiation (`cg:3408-3451`, "everything else … stays transient"). For a child whose owner has been resolved, that changes its lifetime, not just its route: the owner no longer accepts it, holds it, counts it among its children, releases it, or tears it down. A known accepting ancestor does not stop owning a child because another instance of the enclosing type runs in a different domain. Recommended, in order of preference:
  1. **Keep the owner and the mechanism apart.** The resolved owner (`OwnerResolution::Ancestor`) is a fact of the site and is kept whatever the domains say. The delivery mechanism (same-tower allocation, or a cross-pool post) is chosen per enclosing instance.
  2. **Carry the decision per instance.** The enclosing instance carries its owner's domain relative to its own, as the non-singleton plan already carries the owner pointer in `__owner_for_<I>` (`cg:3424-3441`). The birth seam reads that field and takes the same-tower or the cross-pool path at runtime, through a handoff whose two arms are each verified by the tests below. Alternatively, lowering specializes the enclosing method per domain where the instances' domains are static, which the table gives it.
  3. **Otherwise refuse.** Where neither can be emitted (a value use of `I { }` at a site one of whose instances is cross-pool, which sem:308-317 makes fire-and-forget), the program is refused with a located diagnostic at the `I { }` literal naming both enclosing instances and their domains.

  A transient fallback for a resolved owner would be a language decision of its own, with observable-behaviour tests, not a placement correction. Tests, in `crates/hale-codegen/tests/ownership_bubble_mixed.rs`, each in both devirtualization arms, under ASan, and with `LOTUS_NO_OWNERSHIP_BUBBLE=1` as the differential control: a root that accepts `I`, and two `Worker` instances whose method constructs `I`, one under `main` and one under a pinned field. For **both** instances it asserts that the root accepts the child, that the root's child count rises by exactly one per construction, that the child is retained while its constructor has returned, that a `release` frees it once, and that teardown runs exactly once per child at the root's end. The pinned arm also records `pthread_self()` at the child's birth and asserts it is the root's thread (sem:308-317).
- **U-2 (R-7). Proposed, pending the owner: count worker pools on their own, and show main explicitly.** `cooperative_pools` counts the named worker pools, one worker per named pool however many instances it holds (R-2). `main` is never a worker pool. The budget renders it on a line of its own, whether or not a program spells `pool = main`. This is an explicit accounting change, not a correction of a miscount: the PR and its fragment say so, and a declared ceiling can only loosen under it.
- **U-3 (R-6). Proposed, pending the owner: count each actual adapter anchor once.** An adapter literal in `bindings { }` is one pinned thread, counted once in § 2.8's partition and never under a root's bound. Threads outside placement stay visible rather than dropped: binding reader threads and the GH #233 transport serve threads are rendered on a "not counted" line, until P2's binding rows count them. The budget never describes its thread total as a bound on all of the process's threads. It is the placement threads plus the adapter anchors, and its rendering says that.
- **U-4 (M-2). Proposed, pending the owner: a temporary user-only projection.** `PlacedIn` projects user declarations only in P1, as explicit partial coverage. Each omitted stdlib row is a model hole, and the full table, not the arrangement, is authoritative for every safety rule. The projection does not make instance ids stable; M-1, M-3 and M-5 move them anyway (§ 2.4, contract 3). Adding stdlib instances is a separate decision, because it changes the shape half (contract 1).
- **U-5 (M-4). Proposed, pending the owner: yes, project `affined_to` in PR 4.** It is projected from each domain's resolved affinity (the pool's, or the pinned anchor's per replica), not from the written entries, so two entries naming one pool give one row (invariant 3). A CPU set is a column of a domain, never a thread: a pool's affinity is the set its one worker may run on, and a pinned replica's is its own thread's, matching R-3. It changes no shape (contract 1). The `--dump-model` change is pinned in `model.rs`, and the PR body states it.
- **U-6 (§ 4). Proposed, pending the owner: yes, in the graph-switch PR (PR 5), as its own J commit before the bus-graph commit.** Otherwise the runtime test cannot be green in the PR that adds it. The commit establishes the parent's route before any nested subscription uses it (§ 8, the anchor and its descendants), and keeps the anchor's mailbox and pool alive until every child has unregistered and the work outstanding against them is done. It is tested in both devirtualization arms, by § 4's test and by a teardown case: a pinned anchor whose nested subscriber is dissolved while cells for it are queued, run under ASan with `LOTUS_NO_CHUNK_POOL=1`.
- **K-6, G-2, G-3, M-5 (by reading).** Each is a known old bug **only once § 3's case confirms the shape reaches the producer**. A case that the checker refuses first is reported as a checker gap and reclassified before its PR. It is never left unclassified.

## 8. Implementation notes

**A pinned anchor and its descendants.** A pinned domain has one anchor, the row its `Entry` (or its binding) decides. Its descendants inherit the domain (invariant 2) and spawn no thread. Lowering's per-type sets are derived from the actions each instance requires, not from the written field types:

- **A thread** is spawned per anchor row, one per replica, and never for a descendant.
- **A mailbox** is required by an anchor whose subtree subscribes anywhere. Today `pinned_locus_types` gives a struct its mailbox field only when the pinned type itself subscribes (`decl.rs:448-462`), so an anchor whose only subscribers are nested has no mailbox for them to share.
- **The anchor's address.** The shared-mailbox correction (U-6) routes a descendant's registration to its anchor's mailbox. The descendant therefore needs, at runtime, a pointer to its anchor or that mailbox, threaded at birth as the non-singleton bubble plan threads `__owner_for_<I>` (`cg:3424-3441`).
- **The order.** Nested field subscriptions register while the enclosing instance's params are initialized: each child's instantiation registers its own subscriptions (`inst:3808-3816`) during the parent's params init (`inst:2136-2831`), and the pinned parent's mailbox is created only afterwards (`inst:3925-3940`). So either the route is allocated, or published, before params init begins, or the child's binding is deferred until it exists, under a stated readiness and lifetime protocol. Either way it meets the wave-2 construction-readiness decisions, and the protocol is stated in U-6's commit, not left to lowering's order.

**Replica indices stay on their own row.** The key carries the replica index of the replicated root field on every row nested under it (invariant 1), because identity needs it: `f[0].k` and `f[1].k` are two instances. The model's own `replica` column is narrower. The model's `validate` requires `None` on every path whose last component is not a replica (`hale-model/src/application.rs:1469-1489`): `App.workers[0].leaf` is a `leaf`, not replica 0 of anything. So the projection writes the index into the path of every descendant and into `LocusInstance.replica` only on the replica row itself. One placement entry decides a field family that fans out into K rows (invariant 5); a consumer that counts entries reads the family, and one that counts threads reads the rows.

**Unresolved coverage is not a migration regression.** The shadow of PR 1 reads more than today's (`tests/hale`, `dna/`, imports, the merged stdlib), so it will list divergences § 2 does not. A newly listed divergence is neither auto-classified nor auto-blamed on the producer. It is investigated with its witness, the program and the answers of both sides, before fault is assigned. If the legacy side is wrong, it joins § 2 with an id. If the producer is wrong, it is a regression and stops the step. Either classification needs a reviewed explanation, in the fixture row and the PR body, and a regression test that pins the answer.

**Dynamic sites need a policy for what the static tree does not reach.**

- **Enclosing scopes.** `DynamicSite.enclosing` names a locus declaration today, which cannot express the sites that matter here. It becomes `Locus(DeclRef) | Fn(SiteRef)`, so a free function, the user's or the stdlib's, can enclose a literal.
- **A bare `fn main`.** With no `main locus`, `root` is `None` and there are no static rows. Every locus literal is a dynamic site. A literal directly in `fn main` runs on main, so its domain set is `{Main}`.
- **Free factories.** A factory's literals take the union of its callers' domains where the call graph resolves every caller, and are unknown otherwise.
- **Domains absent from the static root tree.** A site whose enclosing scope has no static instance (a locus only ever built dynamically) has an unknown domain set.

Unknown is never defaulted to main. It disables the proofs and optimizations that need a domain, and each consumer states what it does with the hole:

- **The direct-call gate.** Unknown is not `{Main}`, so the subject stays on the deferred path.
- **The intra-locus rewrite.** An unknown field is treated as off its owner's thread, so no publish becomes a direct call.
- **Sync inference.** An accessor with an unknown domain is a domain of its own, so two accessors infer a synchronized discipline.
- **The ownership graph.** No same-tower proof. The site takes U-1's per-instance handoff, or U-1's located refusal for a value use.
- **The F.31 rule.** It compares known domains only. A receiver whose domain is unknown is a hole on that receiver, not a pass, and C7's placed-`Unknown` judgment decides whether it is refused.
- **The resource budget.** The unknown site makes its count an uncertainty, with the hole's reason.
- **The model.** A hole.
- **Lowering.** Tolerates it unchanged: entries are root-only, so no dynamic site has a placement decision, and a dynamic locus is born in its enclosing thread's domain at runtime.

## 9. Implementation checkpoints

These are verification requirements, not new decisions. A producer or consumer PR that touches the area meets its checkpoint, and its body names the test that shows it.

1. **Domains are templates.** A `DomainId` belongs to a template, as the key that anchors it does (§ 1, templates, occurrences, incarnations). A pinned domain anchored by a construction template has one anchor per live occurrence of that template, and so one thread per live occurrence. Required:
   - a thread observation (§ 4's `Seen.tid`, an observation in P2's stream) is compared with the anchor of the occurrence that produced it, never with one tid recorded for the domain;
   - the budget multiplies the template's anchors by its bound (§ 2.8's `bound(c) × |pinned anchors under c|`);
   - no structure maps a `DomainId` to one physical thread id shared by occurrences.

   Tested by a root built at one site twice, both live, with one pinned field nesting a subscriber: each occurrence's handshake records its own `Ready.tid`, every `Seen.tid` equals its own occurrence's anchor, the two anchors' tids differ, and the budget counts 2.
2. **Unknown-domain synchronization is proved conservatively.** The static inventory has one row per site or declaration, but the runtime can execute one accessor site, or one declaration's instances, from several domains. Such a site is never inferred single-threaded because its inventory has one row: a dynamic site contributes every domain its enclosing scope runs in, and an unknown domain set counts as a domain apart from every other (§ 8, sync inference). C1's tests include a dynamically instantiated `@form(hashmap)` owner called from two domains (main and a pinned field), whose inferred discipline is synchronized, never `none`.
3. **U-1's lifetime tests discriminate.** A test that passes under both the resolved-owner lowering and the transient one shows nothing. Required:
   - the differential control (`LOTUS_NO_OWNERSHIP_BUBBLE=1`) changes the expected ownership behaviour: under it the test asserts the transient outcome for at least one of acceptance, child count, retention, release and teardown, so the main arm's assertions are shown to be able to fail;
   - child counts are sampled only after the test has synchronized with the cross-pool birth. The pinned arm's birth is a post to the root's thread (sem:308-317), so the root's count is read after a message the birth itself sends on completion, never right after the constructing call returns and never after a sleep.
4. **Alternative bounds and domain counts stay consistent.** Guarded alternatives of one step take a maximum, and independent constructions take a sum (§ 1, how live bounds combine). That holds for every count over the table, distinct domains and pinned anchors included: domains anchored under exclusive alternatives are never summed, and replica rows, already K rows, are never multiplied by K again. Tested by a nested alternative under replicas, in `placement_table.rs` and `resource_budget.rs`:
   - a root field placed `pinned(replicas = 3)` whose own initializer chooses between two literals: six rows, three per alternative, six `DomainId`s, a thread count of 3, not 6 and not 9;
   - the same field with the alternative one step down (`w.inner`, between a leaf and a subtree of two): every row under `w[i].inner` carries `Some(i)` and is `guarded`, the instance count is `3 × (1 + max(1, 2))`, and the thread count stays 3.

   Both shapes depend on case 11's admission of a literal-armed initializer and are dropped with it.

## 10. What the producer found

The producer (branch `f40/wave2-p1-producer`, PR pending) measured the tree and found shapes the design did not foresee. The driver ruled on each. A ruling is recorded as one paragraph, with the design rows it adds where it names them.

1. **The entry is an implicit construction.** `Origin` gains `Entry`: the program's entry, built once, bound `Once`. A `main locus`'s tower enumerates under it, the literals in `fn main` are constructions under it, and a library seed checked alone roots at its lowering root. The rows are K-7 (§ 2.1) and M-7 (§ 2.4). Coverage gains three cases in § 3: a claims-only `main locus` (13), a library seed alone (14), and a `fn main` building loci by verb (15).

2. **An adapter's instance is pinned on its own thread.** The legacy bus graph and ownership graph label it `SameThread`, which is a known old bug (new rows B-7 in § 2.2 and O-7 in § 2.3). The correction is conditional on § 4's runtime test: an adapter publishing to a main-thread subscriber, the receiving thread recorded. It lands in the graph-switch PR, with the test red on its parent.

3. **The shadow fixture's granularity.** One line per (producer, design row, declaration, witness seed), with a count, under 200 KB. A line per divergence (13,185 over 2026 seeds) is not committed.

4. **Schema deviations the tree forced.** `Decision::Entry { decl, entry }` replaces `Entry { block, entry }`, because the placement block is not a minted site, so only the declaration and the entry are named. `InstanceRow::realizes` is optional, because invariant 6 needs a hole where the producer cannot say what was built. An adapter's literal is its binding entry's site (`Origin::Binding` and `Decision::Binding` carry the same site).

5. **Fields built from an existing instance, and dynamic sites.** A field initialized from an existing instance is a `Hole::Reuse` row that names the source expression. A locus built only dynamically keeps its params subtree unenumerated. A dynamic site of unknown domain is a hole, counted and not pinned: about 86,000 over the 2026 seeds, mostly library seeds and stdlib files loaded as programs.

6. **Cases 6 and 11, checkpoint 4, and K-6.** Case 6 is unreachable at the producer, since the checker refuses a generic locus as a params field. G-2 is a checker gap, not a correction. Rule 18 refuses a conditional at a placed field and refuses an `if` whose arms build two declarations. A choice among literals of one declaration on an unplaced field is admitted (DNA's `Head.bearer`), so `Step::alternative` stays, and checkpoint 4's second shape (the alternative one step down) is the testable one; its first shape, at the placed field, is not reachable. K-6 holds only at the root, because the checker's nested walk reads the resolved type.

7. **`compute_pool_of_locus_type` stays the ninth legacy row** until the checker-switch PR. It is read as a producer beside the eight in § 2, and it is not removed before that PR.

8. **A held instance's subtree lives in its holder's domain** (K-8 in § 2.1, M-8 in § 2.4). A field initialized from an existing instance (`Hole::Reuse`, item 5) moves that instance into its holder's domain on the handoff. Its own locus fields are enumerated under the holder's path as the source's actual rows, never the declaration's defaults: each source row (its realized declaration, its literal with any override, its holes, and its descendants) is projected under the held row, with the holder's domain and `decided_by: Inherited`. The `Reuse` hole stays on the held row and names the source. Each of these rows names, as `InstanceRow::built_by`, its own source row, the one it was built as, on the domain where it was built; the retention question reads that. A question of where an instance runs skips those source rows (`PlacementTable::handed_off`), and a count of instances skips the held ones. Under this row the probe's two divergences become agreement: `checker:K`, pinned against main, and `model:App.h.roles.k`, which only the arrangement had. The probe is case 16 in § 3. The producer links a source only through a local bound once, immutably, to a locus literal in the template's own scope. Where it cannot (a parameter, a name bound twice), nothing is enumerated below the held row (item 9), and the source template stays on main beside it: that residue is K-8 beside B-1 or O-1. Three B-6 and three O-3 divergences over the DNA seeds were hiding this shape: `WebAssets` in two seeds and one `ui::Ui`, each held under a server on pool `io` after passing through a parameter from `fn main`'s template. They are reclassified. No K-3 line hid it. An instance that several holders in different domains hold (`PushSlots`) stays K-3, B-6 and O-3. The re-review found that the first cut enumerated the declared type's defaults under the held row and linked `built_by` afterwards, so an override in the source (`Roles { k: B { } }` where `k: A` by default) gave a held row realizing `A` with no `B` subtree under it: a fabricated row with no hole. The projection replaces that. `a_held_subtree_projects_its_sources_overrides` pins a held `h.roles.k` realizing `B`, with `h.roles.k.child` in the holder's domain and `handed_off()` exactly the three source rows. The source's override is a contract-typed field, so its rows are K-6 and M-5 to the legacy producers, and B-1 and O-1 under the holder.

9. **An unlinked held source asserts no subtree** (K-9 in § 2.1, M-9 in § 2.4). Where the producer cannot link a held row's source (item 8: a parameter, a name bound twice), the table records the `Reuse` hole on the held row and asserts nothing below it: `built_by` is empty, no row is enumerated under it, and `handed_off()` lists nothing for it. The legacy producers assert the declared type's default subtree under the holder. That is a known old bug: they fabricate rows that an override in the source may contradict, the re-review's finding. Both divergences of `held_unlinked.hl` (`fn start(r: Roles)` hands `r` into a pinned holder) are corrections under this row. The first is `checker:K`, pinned in the legacy map and main in the table, whose only `K` row is the one in `fn main`'s template. The second is `model:App.h.roles.k`, which only the arrangement has. The consumer-switch PRs treat the hole conservatively: an instance below an unlinked held row runs in an unknown domain, which disables a proof or an optimization and never defaults to main or to pinned. The row is pinned by `a_held_row_whose_source_is_unlinked_asserts_no_subtree`. The regenerated fixture drops the K-8 lines this subtree had produced under DNA's `ui::Ui`, whose held rows pass through `serve`'s parameter. `CodeExchange`, `PendingSignIns` and `Sessions` become plain K-7 lines, and the lines of `NoGovernanceAuthority` and `NoKnowledgeCommands` go.

## 11. The family's close (P1 6 of 6)

**What is left.** The family's legacy list, read with every part in: main's text (#1315 merged) less what parts 3, 4 and 5 close. Every row has closed, each in the PR that switched its consumer:

| legacy row | closed by |
|---|---|
| `check.rs` · `compute_pool_of_locus_type`, the F.31 map and the copy sync inference built before the mint | #1315, `d32d6899`. C1 (#1301) had left the map in place: the form rows read it over the snapshot's entry row. #1315 switched both, and the function is gone from main |
| `check.rs` · `enclosing_field_placement` | #1315, `4653f870` |
| `bus_graph.rs` · `collect_subscriber_placements` | #1319 (part 3), `bc9e132d` |
| `ownership_graph.rs` · `collect_placements` | #1319 (part 3), `6318c5db` |
| `model_builder.rs` · `PlacedIn` | part 4 (`f40/wave2-p1-4`), `eff92841` |
| `desugar.rs` · `collect_off_owner_thread_fields` | part 4, `9e92dc7d` |
| `resource_budget.rs` · `budget_for_programs` | part 5 (`f40/wave2-p1-5`), `7e0e1083` |
| `codegen.rs` · `collect_main_placement`, `deployment.rs` · `DeploymentPlan` | part 5, `74af1946` |

Part 6 stacks on part 5, which descends from neither #1315 nor part 4. So its tree still holds four of these producers, and the registry names each with the PR that closes it. The family cannot be Canonical in this tree: `compute_pool_of_locus_type` is a derivation-shaped definition that `registry_guard` requires to be registered, and a Canonical family lists no legacy producer. Once the parts are under it, the four rows go with their PRs and nothing else stands between the family and Canonical.

**Two coverage limits of the model's arrangement remain.** Neither is a producer of placement. The arrangement is a projection of the table, and the table answers every placement question (part 4). Each stays open, with its blocker:

- **M-7: a literal `fn main` builds besides the root.** The table has these templates (`PlacementTable::entry_literals`). The arrangement projects only the deployed root's templates (part 4), so a `fn main` that builds `Worker { }` beside `App { }`, or a bare `fn main` with no root, arranges nothing for that literal. Measured over the corpus, `tests/hale` and the DNA seeds, 719 of the 1,444 seeds whose table is not empty build such a template (656 have a root). Projecting them is reachable: the path of a template's row starts at its top's declaration, as `PlacementTable::path_of` already writes it, and contract 3 holds as for any template (one path for every template that reaches it, arranged where they agree, a hole where they do not). It is blocked twice over. First, the projection is part 4's code, which this branch does not descend from. Second, those literals are births that the model's `RuntimeInheritedPlacement` holes account for today, and those holes are the ownership family's legacy row of the model's dynamic births (C3 2 of 2, `f40/wave2-c3-2`). Moving that row onto the graph's rows is a classified correction that stopped on a shadow difference (`tests/hale/api_binding_run_test.hl`). Arranging M-7's literals has to remove their births from those holes in the same change, so it follows that row's move, not part 6. The `--dump-model` pin and the instance-id remapping table belong to that change: the instances added sort into the path order and renumber every later id.
- **U-4 (M-2): stdlib instances.** A row realizing a stdlib declaration (`__StdBytesBytesBuilder`, `__StdProcessChild`, `__StdHttpServer`, `__StdIoTcpListener` and others) cannot be arranged without moving `shape_hash`. `Realizes` names a `LocusDeclId` in `entities.loci`, and `project_model_half` renders the set of `entities.loci` into the hashed half (`topology_projection.rs`), so adding a stdlib declaration there moves the hash of every program with a stdlib-typed field. Carrying them needs a model schema change of its own, a declaration entity outside the hashed half or a `Realizes` that names one, which is the separate decision U-4 named. Until then the arrangement stays user-only, as decided.
