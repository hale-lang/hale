# Claims & the Law

While [effect contracts](./effects.md) verify what an individual function or locus may do, **claims** verify architectural invariants across an entire assembled program graph. Declared in a `claims { }` block on `main locus` (or in a library seed or reusable [constitution](./constitutions.md)), claims are evaluated statically by `hale check` as hard build errors and compile to zero runtime code.

```hale,refused
type Task { id: Int; }
type Metric { n: Int; }
topic Tasks   { payload: Task; }
topic Metrics { payload: Metric; }

locus DeltaTriage {
    params { seen: Int = 0; }
    bus { subscribe Tasks as on_task; publish Metrics; }
    fn on_task(t: Task) {
        self.seen = self.seen + 1;
        Metrics <- Metric { n: t.id };
    }
}

locus GammaResearch {
    params { total: Int = 0; }
    bus { subscribe Metrics as on_metric; }
    fn on_metric(m: Metric) { self.total = self.total + m.n; }
}

group delta_wing = { DeltaTriage };
group gamma_wing = { GammaResearch };

main locus Org {
    params {
        triage: DeltaTriage = DeltaTriage { };
        research: GammaResearch = GammaResearch { };
    }
    claims {
        iso_dg: forbid reaches(delta_wing, gamma_wing);
    }
}

fn main() { Org { }; }
```

When a claim fails, `hale check` rejects the build and prints a **witness**—the minimal call and bus path violating the invariant:

```text
claim `iso_dg` violated: `delta_wing` reaches `gamma_wing` —
witness: `DeltaTriage::on_task` -(publishes "Metrics")-> `GammaResearch::on_metric`
```

For the development workflow of writing architectural claims before implementation, see [Claim-Driven Development in Hale](https://hale-lang.org/articles/claim-driven-development-in-hale/).

---

## Syntax Cheat Sheet

### Vocabulary Declarations

| Declaration | Syntax | Description |
|---|---|---|
| **Group** | `group g = { LocusA, fn_b, lib::Item, lib::*, role R, std::log::* } [may_be_empty];` | Names a set of loci, free functions, imported seed members, API role handlers, or Hale-source stdlib modules. |
| **Built-in `program` Group** | `program` *(no declaration needed)* | Implicitly selects every locus and free function declared in the closing entrypoint's own seed (excluding imports). |
| **Domain** | `domain wing = { delta, gamma };` | Declares a finite, closed set of partition or tenant identifiers. |
| **Indexed Effect Family** | `effect knowledge(wing);` | Expands a domain into per-member effect classes (`knowledge(delta)`, `knowledge(gamma)`) and a union wildcard (`knowledge(*)`). |

### Claim Sentences (`claims { <name>: <sentence>; }`)

| Claim Family | Sentence Syntax | What It Proves |
|---|---|---|
| **Reachability Prohibition** | `forbid reaches(SRC, DST)`<br>`  [via { calls, bus }]`<br>`  [during <phase>]`<br>`  [avoiding <gate>];` | No path in the chosen edge relation (`calls`, `bus`, or both) connects group `SRC` (optionally filtered to `<phase>`) to group or `effects(C)` `DST` without passing through `<gate>` (`<group>` or `std::<mod>::*`). |
| **Direct Boundary Allowlist** | `only edges A -> B {`<br>`  publish T; subscribe U;`<br>`};` | Every direct edge from group `A` into group `B` is a bus edge on one of the explicitly listed topics (`call` edges are never allowed). |
| **Capability Budget** | `bound C <= N on paths from G;` | Every execution path starting from group `G` invokes carriers of user effect class `C` (or `secret_use`) at most `N` times across calls and bus hops. |
| **Bus Wiring Existence** | `require subscribes(some G, topic T);`<br>`require publishes(some G, topic T);` | At least one locus in group `G` declares a subscription (or publication) for topic `T` in its `bus { }` block. |
| **State Encapsulation** | `require sealed(all G);` | Every locus in group `G` is annotated with `@sealed`. |
| **Effect Attribution** | `require attributed(all C);` | Every function that directly performs built-in effect class `C` also declares a user-defined purpose class via `@effects(is: {...})`. |
| **Execution Domain Separation** | `require separate_domains(all G1, all G2);` | Every arranged locus instance in `G1` runs on a distinct execution domain (pool or pinned thread) from every arranged instance in `G2`. |
| **Lossless Bus Routing** | `require no_silent_loss(topic T);`<br>`require no_silent_loss(all G);` | No boundary on topic `T` (or on any route touched by group `G`) is configured to silently drop messages (`swallow`, `drop_old`, `drop_new`, `or discard`, transport `drop`). |
| **Seed Topic Coverage** | `cover topic in seed(alias): subscribed_by(some G);` | Every topic declared in the seed imported as `alias` has at least one subscriber in group `G`. |
| **Keyed Routing Partition** | `cover keys(topic T [in LO..=HI]): delivered_to(exactly_one G);` | Every permitted key on keyed topic `T` is delivered to **exactly one** arranged subscriber instance in group `G`. |
| **Endpoint Cardinality** | `count publishers(topic T) (== \| <= \| >=) N;`<br>`count subscribers(topic T) (== \| <= \| >=) N;` | The number of distinct declared loci publishing or subscribing to topic `T` satisfies the comparison against `N`. |
| **Constitution Adoption** | `adopt ConstitutionName;` | Includes all claim sentences from `ConstitutionName` (and its `extends` ancestors) into this `claims` block. |

---

## Formal Grammar

From [`spec/grammar.ebnf`](https://github.com/hale-lang/hale/blob/main/spec/grammar.ebnf):

```text
group_decl        = "group" , IDENTIFIER , "=" , "{" ,
                    [ group_member , { "," , group_member } , [ "," ] ] ,
                    "}" , [ "may_be_empty" ] , ";" ;
group_member      = IDENTIFIER , { "::" , IDENTIFIER } , [ "::" , "*" ]
                  | "role" , IDENTIFIER ;

domain_decl       = "domain" , IDENTIFIER , "=" , "{" ,
                    IDENTIFIER , { "," , IDENTIFIER } , [ "," ] ,
                    "}" , ";" ;
effect_family_decl
                  = "effect" , IDENTIFIER , "(" , IDENTIFIER , ")" , ";" ;

claims_block      = "claims" , "{" , { claim_entry } , "}" ;
                  (* inside main locus: the world tier;
                     at top level: the library tier *)
claim_entry       = ( IDENTIFIER , ":" , claim_form , ";" )
                  | ( "adopt" , IDENTIFIER , ";" ) ;
claim_form        = forbid_form | only_edges_form | bound_form
                  | require_form | cover_form | route_form
                  | count_form ;

forbid_form       = "forbid" , "reaches" , "(" , claim_set , "," ,
                    claim_set , ")" ,
                    { via_clause | during_clause | avoiding_clause } ;
via_clause        = "via" , "{" , via_relation ,
                    { "," , via_relation } , [ "," ] , "}" ;
via_relation      = "calls" | "bus" ;
during_clause     = "during" , IDENTIFIER ;
avoiding_clause   = "avoiding" , IDENTIFIER ;

only_edges_form   = "only" , "edges" , IDENTIFIER , "->" ,
                    IDENTIFIER , "{" , { edge_grant } , "}" ;
edge_grant        = ( "publish" | "subscribe" ) , topic_ref , ";" ;

bound_form        = "bound" , effect_class_ref , "<=" , INT_LIT ,
                    "on" , "paths" , "from" , IDENTIFIER ;

require_form      = require_endpoint | require_sealed
                  | require_attributed | require_no_silent_loss
                  | require_separate_domains ;
require_separate_domains
                  = "require" , "separate_domains" , "(" ,
                    "all" , IDENTIFIER , "," , "all" , IDENTIFIER , ")" ;
require_endpoint  = "require" , ( "subscribes" | "publishes" ) ,
                    "(" , "some" , IDENTIFIER , "," , "topic" ,
                    topic_ref , ")" ;
require_sealed    = "require" , "sealed" ,
                    "(" , "all" , IDENTIFIER , ")" ;
require_attributed = "require" , "attributed" ,
                    "(" , "all" , IDENTIFIER , ")" ;
require_no_silent_loss
                  = "require" , "no_silent_loss" , "(" ,
                    ( ( "topic" , topic_ref ) | ( "all" , IDENTIFIER ) ) ,
                    ")" ;

cover_form        = "cover" , "topic" , "in" , "seed" , "(" ,
                    IDENTIFIER , ")" , ":" , "subscribed_by" ,
                    "(" , "some" , IDENTIFIER , ")" ;

route_form        = "cover" , "keys" , "(" , "topic" , topic_ref ,
                    [ "in" , key_bound , "..=" , key_bound ] , ")" ,
                    ":" , "delivered_to" , "(" , "exactly_one" ,
                    IDENTIFIER , ")" ;
key_bound         = [ "-" ] , INT_LIT ;
count_form        = "count" , ( "publishers" | "subscribers" ) ,
                    "(" , "topic" , topic_ref , ")" ,
                    ( "==" | "<=" | ">=" ) , INT_LIT ;

claim_set         = IDENTIFIER
                  | "effects" , "(" , effect_class_ref , ")" ;
topic_ref         = IDENTIFIER , { "::" , IDENTIFIER } ;
effect_class_ref  = IDENTIFIER , [ "(" , ( IDENTIFIER | "*" ) , ")" ] ;
```

All claim introducers (`group`, `domain`, `claims`, `forbid`, `reaches`, `via`, `during`, `avoiding`, `only`, `edges`, `bound`, `require`, `cover`, `count`, `seed`, `may_be_empty`, and predicate names) are **contextual keywords**: they are recognized only in grammar position and remain valid as ordinary identifiers elsewhere. Only existing language keywords (`bus`, `publish`, `subscribe`, `on`, `in`) are reserved everywhere.

---

## `group` — the vocabulary

```hale,fragment
group delta_wing = { delta::*, DeltaStore, helper_fn };
group gamma_wing = { gamma::Research };
group probes     = { } may_be_empty;
```

A `group` names a set of declared program elements (loci, free functions, and imported declarations).

### Group Member Forms

| Member Form | Resolution Semantics |
|---|---|
| `Name` | A locus or free function declared in the current seed. If a locus and a free function share `Name`, both are included. |
| `alias::Name` | A declaration imported under `import "..." as alias;`, canonicalized through the compiler's symbol table. |
| `alias::*` | Every locus and free function declared in the seed imported as `alias` (types, topics, and constants are ignored). Single-level and trailing-only (`a::*::b` and `a::b::*` are rejected). |
| `role R` | Every API surface handler method callable by a principal holding role `R` (see [Role groups](#role-groups)). |
| `std::m::*` | Every locus and free function in the Hale-source stdlib module `std::m` (e.g., `std::log::*`). Native-lowered stdlib modules (`std::math`) are rejected. Allowed only as a `forbid reaches` target or `avoiding` gate. |
| `program` *(built-in)* | Implicitly available in every claim without declaration: includes every locus and free function in the closing entrypoint's own seed (excluding imports). Redeclaring `group program` is an error. |

### Function-Grained Projection and Compile-Time Guards

Claims evaluate at function granularity. Including a locus in a group projects to **all** of its member `fn`s, lifecycle hooks (`birth`, `accept`, `release`, `run`, `drain`, `dissolve`), and modes (`bulk`, `harmonic`, `resolution`). A free function projects to itself.

To prevent claims from passing vacuously due to typos or empty sets, `hale check` rejects:

- Unknown single-segment names or undeclared roles (with a did-you-mean hint).
- Unresolved `alias::Name` members or `alias::*` globs over unimported aliases.
- A `role R` member that no `api` surface row requires, unless the group is marked `may_be_empty`.
- Duplicate group declarations.
- Any group that resolves to **zero declarations**, unless explicitly suffixed with `may_be_empty`.
- **Projection vacuity**: A non-empty group whose members contain zero executable methods/hooks/modes (e.g., pure-data loci) when used as a `forbid` or `only edges` endpoint or a `bound` source.

### Role groups

While `requires: [reader]` on an `api` surface row enforces authorization at the transport boundary, a `role <name>` group member lets you prove what code paths an authorized caller can trigger inside the program:

```hale
role reader;
role editor includes reader;

type Change { what: String; }
type Query { n: Int; }
topic TodoChanged { payload: Change; }

locus Todos {
    params { n: Int = 0; }
    bus { publish TodoChanged; }
    fn add(q: Query) -> Int {
        self.n = self.n + q.n;
        TodoChanged <- Change { what: "added" };
        return self.n;
    }
    fn list(q: Query) -> Int { return self.n; }
}

locus Feed {
    params { seen: Int = 0; }
    bus { subscribe TodoChanged as on_change; }
    fn on_change(c: Change) { self.seen = self.seen + 1; }
}

api Todo {
    rpc Todos::add requires: [editor];
    rpc Todos::list requires: [reader];
}

group read_side = { role reader };
group changes   = { Feed };

main locus App {
    params { todos: Todos = Todos { }; feed: Feed = Feed { }; }
    claims {
        a_reader_publishes_nothing: forbid reaches(read_side, changes);
    }
}
fn main() { App { }; }
```

`role R` selects the specific handler method (not the entire locus) of every surface row across the program that a caller holding `R` is permitted to invoke:

- Because `editor includes reader`, `role editor` selects both `Todos::add` and `Todos::list`, whereas `role reader` selects only `Todos::list`.
- Any surface row that requires no role (`requires: []`) is callable by all principals and is included in every `role` group.

---

## `domain` and effect families — the data-plane vocabulary

```hale,fragment
domain wing = { delta, gamma };

effect llm;
effect knowledge(wing);

@effects(is: {knowledge(delta)})
fn read(key: String) -> Idea { ... }
```

- `domain <name> = { <id1>, <id2>, ... };` declares a non-empty, closed set of identifiers. It must appear earlier in the same file than any `effect` family referencing it.
- `effect <family>(<domain>);` interns an atomic effect class for each domain member (`knowledge(delta)`, `knowledge(gamma)`) and a composed wildcard class `knowledge(*)` equal to the union of all members.
- Indexed classes work everywhere ordinary effect classes work: `@effects(is:/only:/none:/causes:)`, composed effects (`effect sensitive = { knowledge(delta) };`), claim targets (`effects(knowledge(delta))`, `effects(knowledge(*))`), `bound`, and `@budget(knowledge(delta) = N)`.
- Adding a new identifier to a `domain` automatically places that new instantiation outside existing `@effects(only: {...})` allowlists.

---

## `claims { }` Placement and Scoping Tiers

A `claims { }` block can appear in two places:

1. **World Tier (inside `main locus`)**: Evaluates over the complete, closed application graph. A `main locus` may contain multiple `claims { }` blocks; their entries are concatenated. Inline claims resolve group names within the seed that declares the claim (so importing an application seed into a test preserves the application's own group bindings).
2. **Library Tier (at top level in a library seed with no `main locus`)**: Allows a library seed to state invariants over its own declarations and boundary (see [Library-Tier Claims](#library-tier-claims)). A seed that declares a `main locus` cannot also have a top-level `claims { }` block.

Every claim entry is written `<name>: <form>;`. Claim names must be unique across all blocks in a seed, and every failed claim is a hard compile error.

---

## Claim Sentences Reference

### 1. Reachability Prohibition (`forbid reaches`)

```hale,fragment
iso:        forbid reaches(delta_wing, gamma_wing);
iso_calls:  forbid reaches(delta_wing, gamma_wing) via { calls };
data_iso:   forbid reaches(delta_wing, effects(knowledge(gamma)));
quiet_boot: forbid reaches(positions, effects(llm)) during birth;
gated:      forbid reaches(public_api, ledger) avoiding authorization_gate;
logs_only:  forbid reaches(program, effects(stdio)) avoiding std::log::*;
```

- **Endpoints (`SRC`, `DST`)**: `SRC` must be a group. `DST` may be a group or `effects(<class>)` (where `<class>` is a built-in class, user class, composed class, domain family instantiation, `family(*)`, or `client(Surface)` / `client(*)`). Using `effects(...)` in `SRC` is an error. If a declaration belongs to both `SRC` and `DST`, it immediately violates the claim as a zero-length path.
- **`via { calls, bus }`**: Selects which edges the reachability walk traverses (omitting `via` traverses both):
  - `calls`: Direct calls, `self` methods, handle calls on typed receivers, interface slot defaults, calls into imported seeds, and calls through Hale-source stdlib bodies.
  - `bus`: Connects each statically-known publish site in a visited function to every subscribing handler for that topic or matching `**` wildcard subject (including stdlib loggers like `std::log::Logger` reaching `StdoutSink` -> `println` when `StdoutSink` is instantiated in the program). Note that `via { bus }` alone only follows publish statements directly inside `SRC` functions; omit `via` (or write `via { calls, bus }`) to follow calls that lead to a publish.
- **`during <phase>`**: Filters `SRC`'s projection to a specific lifecycle hook (`birth`, `accept`, `release`, `run`, `drain`, `dissolve`), mode (`bulk`, `harmonic`, `resolution`), or method/handler name. Free functions drop out; if the filter leaves zero functions in `SRC`, compilation fails with an error.
- **`avoiding <gate>`**: Removes all vertices in `<gate>` (a declared group or `std::<mod>::*` such as `std::log::*`) from the graph walk. This expresses **mandatory interposition**: proving that `A` cannot reach `B` when `G` is removed proves that every path from `A` to `B` passes through `G`. `<gate>` must be disjoint from both `SRC` and `DST`.

### 2. Direct Boundary Allowlists (`only edges A -> B`)

```hale,fragment
grant: only edges gamma_wing -> delta_wing {
    publish t::ResearchDigest;
};
```

Requires every **direct** edge from group `A` into group `B` to match one of the listed topic grants, reporting all un-granted edges if violated:

- `publish T;` and `subscribe T;` grant the exact same `A -> B` bus edge on declared topic `T`.
- **Direct call edges from `A` to `B` are never grantable** and always violate `only edges A -> B`.
- Bus edges on raw string literals or wildcard subjects without a declared topic cannot be granted and fail closed if they land in `B`.
- `only edges A -> B` checks only direct `A -> B` edges; it does not restrict `B -> A` edges or multi-hop paths through a third group `C`.

### 3. Capability Budgets (`bound C <= N on paths from G`)

```hale,fragment
one_call: bound llm <= 1 on paths from positions;
```

Bounds the per-invocation call-tree sum of a user-declared effect class `C` (or `secret_use`) along any path starting from group `G`, following both call edges and bus hops:

- If a function in `G` calls two helpers sequentially and each invokes an `llm` carrier once, the path cost is `2`.
- Any carrier reachable inside a loop, recursion, unresolved indirect call, untyped receiver, or computed publish subject has an **unbounded** cost and violates any finite bound `N`.
- For built-in dimensions on a single function (`alloc_per_call`, `stack_bytes`, `block_points`, `publish`, `fanout`), use [`@budget`](./effects.md#quantitative-budgets-budget).

### 4. Wiring, Encapsulation, and Attribution (`require`)

```hale,fragment
wired:      require subscribes(some delta_wing, topic t::Tasks);
feeds:      require publishes(some delta_wing, topic t::Done);
confined:   require sealed(all vaults);
attributed: require attributed(all syscall);
```

- **`require subscribes(some G, topic T)` / `require publishes(some G, topic T)`**: Verifies that at least one locus in `G` declares `subscribe T` or `publish T` in its `bus { }` block.
- **`require sealed(all G)`**: Verifies that every locus in `G` carries `@sealed` (so its `params` fields cannot be read or mutated outside its own methods).
- **`require attributed(all C)`**: Verifies that every function directly performing built-in effect class `C` carries a user-declared effect classification `@effects(is: {...})`.

### 5. Execution-Domain Separation (`require separate_domains`)

```hale,fragment
isolate_io: require separate_domains(all workers, all gateways);
```

Verifies that every arranged locus instance in `workers` executes on a different runtime domain from every arranged locus instance in `gateways`:

- **Domain identity**: Each named cooperative pool is one domain; each `pinned` locus instance (or replica) is its own domain; an unplaced child locus inherits its parent's domain (so two unplaced children of the same pinned parent share a domain).
- **Group selection**: Groups select locus declarations directly (not child subtrees; include child declarations in the group if they should be checked). Both groups must select at least one locus with a non-zero instance count (empty groups or zero-instance loci are rejected as `invalid`, even with `may_be_empty`).
- If two instances share a domain, the claim is `violated` and prints both instance paths and their shared domain. If an instance's placement cannot be statically resolved, the claim reports `uncertified`.

### 6. Lossless Bus Configuration (`require no_silent_loss`)

```hale,fragment
payments_arrive: require no_silent_loss(topic Payments);
money_is_safe:   require no_silent_loss(all money);
```

Verifies that no modeled boundary on topic `Payments` (or on any bus route published or subscribed by a member of `money`, including routes covered by wildcard patterns like `"pay.**"`) is configured to silently discard messages:

| Route Boundary | Configuration Checked | Violates `no_silent_loss` When Set To | Non-Silent Alternatives |
|---|---|---|---|
| **Keyed Topic Routing** | Topic's `on_unmatched:` policy | `swallow` *(default on keyed topics)* | `on_unmatched: fail;` or `on_unmatched: fallback;` (with a `where key == _` subscriber) |
| **Subscriber Queue** | Subscription's `bounded(N, ...)` policy | `drop_old` or `drop_new` | Unbounded subscription queue |
| **Publish Call Site** | Send statement's `or` handler | `or discard` | Plain send (on non-failing topic), `or raise`, or `or wait` |
| **Transport Binding** | External transport loss policy | `drop` | `listen` binding, or `connect` binding where every send uses `or wait` |

Example of a route that violates `no_silent_loss`:

```hale,refused
type Payment { account: Int = 0; cents: Int = 0; }

topic Payments {
    payload: Payment;
    subject: "payments";
    keyed_by account;
}

locus Ledger {
    params { booked: Int = 0; }
    bus { subscribe Payments as on_payment bounded(64, drop_old) where key == 1; }
    fn on_payment(p: Payment) { self.booked = self.booked + p.cents; }
}

locus Gateway {
    bus { publish Payments; }
    fn pay(account: Int, cents: Int) {
        Payments <- Payment { account: account, cents: cents };
    }
}

group money = { Ledger, Gateway };

main locus App {
    params { ledger: Ledger = Ledger { }; gate: Gateway = Gateway { }; }
    claims {
        payments_arrive: require no_silent_loss(topic Payments);
        money_is_safe:   require no_silent_loss(all money);
    }
    run() { self.gate.pay(1, 500); }
}
fn main() { App { }; }
```

```text
claim `payments_arrive` violated: route `Payments` is configured to discard silently — topic `Payments` keyed by `account` is `on_unmatched: swallow`; subscriber `Ledger::on_payment` on `Payments` is `bounded(64), on_full: drop_old`
```

To satisfy the claim, configure the topic with `on_unmatched: fail;`, remove `bounded(64, drop_old)` from the subscriber, and handle the fallible send with `or raise`:

```hale,fragment
topic Payments {
    payload: Payment;
    subject: "payments";
    keyed_by account;
    on_unmatched: fail;
}
// Ledger:  subscribe Payments as on_payment where key == 1;
// Gateway: Payments <- Payment { account: account, cents: cents } or raise;
```

> [!NOTE]
> **When `no_silent_loss` Reports `uncertified`**:
> - A send routes its refusal into a custom inline handler (`or println("dropped")` or `or fail ...`) whose behavior the model cannot verify.
> - A `connect` transport binding is used without `or wait` on every send.
> - An unmodeled adapter hole or a runtime-computed publish subject (`self.subject <- v`) whose declared `publish` pattern covers the route prevents complete static inspection.

### 7. Seed Topic Coverage (`cover topic in seed`)

```hale,fragment
no_orphans: cover topic in seed(t): subscribed_by(some positions);
```

Verifies that every topic declared in the seed imported under alias `t` has at least one subscribing locus in group `positions`. Reports all uncovered topics on failure.

### 8. Keyed Routing Partitions (`cover keys`)

```hale,fragment
one_worker: cover keys(topic Orders in 0..=3): delivered_to(exactly_one workers);
whole_type: cover keys(topic Orders): delivered_to(exactly_one workers);
```

Verifies that for every permitted key on a `keyed_by` topic, **exactly one** arranged subscriber registration in the named group receives that key (neither zero recipients nor overlapping recipients):

```hale,refused
type Order { shard: Int = 0; qty: Int = 0; }
topic Orders { payload: Order; subject: "orders"; keyed_by shard; }

locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_order where key == replica; }
    fn on_order(o: Order) { self.seen = self.seen + o.qty; }
}

locus Audit {
    params { n: Int = 0; }
    bus { subscribe Orders as on_any; }
    fn on_any(o: Order) { self.n = self.n + 1; }
}

group workers = { Worker };

main locus App {
    params { w: Worker = Worker { }; audit: Audit = Audit { }; }
    placement { w: pinned(cores = 0..4, replicas = 4); }
    bus { publish Orders; }
    claims {
        one_worker: cover keys(topic Orders in 0..=5): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 1, qty: 2 }; }
}
fn main() { App { }; }
```

Because `w` has 4 replicas (`0..=3`) and the claim requires `0..=5`, keys `4..=5` have no recipient in `workers`:

```text
claim `one_worker` violated: not every key of `Orders` reaches exactly
one registration of `workers` — uncovered: no registration of `workers`
receives keys 4..=5 (the publish is silently swallowed; its recipients
lie outside the group: `Audit::on_any`)
```

Note that `Audit` (which subscribes to all keys of `Orders`) does not cause an overlap violation because `Audit` is outside `workers`. Changing the interval to `in 0..=3` satisfies the claim.

- **Key Domain**: `in LO..=HI` specifies an inclusive integer range (negative bounds like `in -3..=3` are valid). When `in LO..=HI` is omitted, the domain is the full key type (`Int`, `Bool`, etc.), which requires a `where key == _` fallback subscriber on an `on_unmatched: fallback` topic to cover unmatched keys.
- **Fail-Closed Conditions for `cover keys`**:

| Condition | Verdict | Reason |
|---|---|---|
| Dynamic key filter (`where key == self.id`) on an instantiated subscriber | `uncertified` | Key filter is not a literal or `replica` constant. |
| Subscribing locus spawned dynamically outside `main` arrangement | `uncertified` | Instance population is not statically bounded. |
| Unmodeled hole hiding subscriptions, counts, or filters on `T` | `uncertified` | Subscriber set is incomplete. |
| No `in LO..=HI` interval and no known publish site (or an unresolved computed-subject publisher on `T`'s wire) | `uncertified` | Permitted key domain cannot be established without `in LO..=HI`. |
| Topic `T` does not declare `keyed_by` | `invalid` | Topic is unkeyed. |
| `in LO..=HI` used on a non-`Int` key field (e.g., `Bool` or `String`) | `invalid` | Integer interval requires an `Int` key field. |
| Target group has no locus members | `invalid` | Free functions cannot hold subscriptions. |

### 9. Endpoint Cardinality (`count publishers` and `count subscribers`)

```hale,fragment
single_writer: count publishers(topic t::Tasks) == 1;
consumed:      count subscribers(topic t::Done) >= 1;
capped:        count subscribers(topic t::Audit) <= 2;
```

Counts the number of **distinct declared loci** that publish or subscribe to the topic. To count replicated runtime instances across a multi-binary deployment, use `count` inside a `fleet locus` ([Multi-Binary Services](./services/multi-binary.md#multi-binary-verification-fleet-locus)).

---

## Library-Tier Claims

A library seed (a directory with no `main locus`) can declare a top-level `claims { }` block over its own loci, functions, and topics:

```hale,fragment
type Charge { amount: Int; }
topic Charges { payload: Charge; }

locus Wire {
    params { seen: Int = 0; }
    bus { subscribe Charges as on_charge; }
    fn on_charge(c: Charge) { self.seen = self.seen + c.amount; }
}

group settlers = { Wire };

claims {
    single_settle: count subscribers(topic Charges) <= 1;
    wired: require subscribes(some settlers, topic Charges);
}
```

Library-tier claims **travel with the import**:

- When the library seed is checked standalone (`hale check lib/pay`), its claims are evaluated against the library's own declarations.
- When an application imports `lib/pay as pay`, `pay`'s claims are re-evaluated over the **merged application graph**. If the application adds a second subscriber to `pay::Charges`, `hale check` fails the application build citing ``claim `pay::single_settle` violated`` and pointing to the library's claim declaration.

---

## Evaluation Model,Verdicts, and Fail-Closed Rules

Every claim and lowered function certificate (`@effects`, `@budget`, `@phase_effects`) evaluates to one of four verdicts:

| Verdict | Meaning | Required Action |
|---|---|---|
| `holds` | Proved over all modeled paths. | None (passes build). |
| `violated` | Disproved; a concrete counterexample path or configuration exists. | Fix the code or configuration identified by the witness. |
| `uncertified` | Well-formed claim, but the program graph contains an unresolvable edge (indirect call, untyped receiver, computed publish subject). | Refactor the unresolvable call/publish into a statically typed form. |
| `invalid` | The claim or group declaration itself is malformed (e.g., undeclared group member or empty group without `may_be_empty`). | Fix the claim or group syntax. |

Both `violated`, `uncertified`, and `invalid` fail `hale check`.

### How Indirect Calls, Receivers, and Interfaces Are Resolved

1. **Function Pointers & Indirect Calls**: A call through a local `let f = target_fn; f()` resolves directly to `target_fn`. A call through a function-typed parameter or variable fans out conservatively to every function referenced as a value in the program with a matching signature. Only an indirect call with zero matching candidates reports `uncertified`.
2. **Method Receivers**: The analyzer resolves types for local variables, `self` fields, chained struct/locus fields, struct literals, function return values, uniform `if`/`else` expressions, `or`-unwrapped expressions, single-slot `.get(i)` calls, and `for` loop variables over arrays or capacity slots. Calling a method on an untyped complex receiver reports `uncertified` with a hint to bind the receiver to a typed local variable.
3. **Interface Dispatch Fan-Out**: Calling a method on an interface type (`handler.handle(ctx)`) fans out to all loci in the closed world that structurally satisfy the interface. Reachability walks visit all alternatives; quantitative bounds (`bound`, `@budget`) take the **maximum** across alternatives at a single call site.
4. **Interface Field Narrowing**: When an interface call is made through a locus field (`self.h.dep.apply(x)`), the compiler narrows the target set to only the concrete loci actually stored into that field across struct literals and default parameters in the closed application (unless the field is assigned dynamically or the seed has no `main locus`).
5. **Uninhabited Interfaces**: If zero loci in the closed world satisfy an interface, call sites on that interface are dead code, contribute no edges, and are recorded in the topology artifact under `unknowns` (`uninhabited_interface_call:<iface>.<callee>`).

---

## Diagnostics Catalog

### Parse Errors

| Condition | Diagnostic Summary |
|---|---|
| `claims { }` inside a non-`main` locus | `"only valid inside main locus"` |
| Unknown claim verb | Lists the valid claim verbs (`forbid`, `only`, `bound`, `require`, `cover`, `count`, `adopt`). |
| `via { }` empty or unknown relation | `"must name at least one relation"` / `"the composable relations are calls and bus"` |
| Nested glob in group (`a::*::b` or `a::b::*`) | `"the glob is trailing-only"` |
| Empty key interval in `cover keys` (`3..=0`) | `"the key interval 3..=0 is empty"` |
| Negative bound or count | `"expected an integer bound after <="` / `"expected an integer after the comparison"` |
| Empty or duplicate `domain` | `"has no members"` / `"declared more than once"` |
| Effect family over a `domain` not declared earlier in the file | `"declare domain X = { … }; above the family"` |
| Effect-mask overflow (> 54 user atomic classes) | Rejected at the `effect` declaration (`fail-closed`). |
| Duplicate `@budget` key or built-in effect class used as user budget key | `"state it once"` / points to built-in `@budget` dimension names. |

### Vocabulary & Reference Errors (`invalid` / `uncertified`)

| Condition | Verdict & Diagnostic Summary |
|---|---|
| Unknown group member or undeclared `role R` | `invalid`: `"names no declared locus or fn"` / `"names no declared role"` + did-you-mean |
| `role R` not required by any surface row (without `may_be_empty`) | `invalid`: `"no surface row requires role R"` |
| Unresolved `alias::Name` or `alias::*` | `invalid`: `"does not resolve — no imported declaration matches"` / `"names no import alias"` |
| Duplicate group or duplicate claim name | `invalid`: `"declared more than once"` |
| Empty group without `may_be_empty` | `invalid`: `"holds vacuously — say may_be_empty"` |
| Projection vacuity (group has no executable methods/hooks/modes) | `invalid`: `"projects to no executable … vertices"` |
| Unknown group or undeclared effect class in a claim | `invalid`: `"never declared"` + did-you-mean |
| `effects(...)` used in `SRC` position of `forbid reaches` | `invalid`: `"sources must be declared groups"` |
| Built-in effect class used in `bound` (other than `secret_use`) | `invalid`: points to `@budget` dimension spellings |
| Unknown topic in grant, `require`, or `count` | `invalid`: did-you-mean over declared topics |
| `cover topic in seed(alias)` where `alias` declares no topics | `invalid`: `"the coverage domain would be empty"` |
| `no_silent_loss(all G)` where `G` neither publishes nor subscribes | `invalid`: `"names no route"` |
| `separate_domains` over a group with no loci or zero-population locus | `invalid`: cannot certify separation without arranged instances |
| `during <phase>` matching zero functions in `SRC` | `invalid`: `"a claim over an empty phase holds vacuously"` |
| `avoiding <gate>` overlapping `SRC` or `DST` | `invalid`: `"masking an endpoint makes the claim weaker than it reads"` |
| Unresolvable call, receiver, filter, or dynamic population | `uncertified`: `"cannot be certified: …"` + remediation hint |

### Claim Violations (`violated`)

| Claim Verb | Witness Reported in Diagnostic |
|---|---|
| `forbid reaches` | Minimal counterexample path (`->` for calls, `-(publishes "subject")->` for bus hops) with source spans. |
| `only edges` | Every un-granted `A -> B` edge alongside the declared grant list. |
| `bound` | Measured call-tree count + heaviest chain, or the specific unbounded condition (loop, recursion, etc.). |
| `require subscribes / publishes / sealed / attributed` | The group or class and the missing declaration or annotation. |
| `require separate_domains` | Both colliding instance paths, their shared execution domain, and source locations. |
| `require no_silent_loss` | Every silent boundary setting (topic `on_unmatched`, subscriber queue policy, send `or discard`, binding `drop`). |
| `cover topic in seed` | Every uncovered topic in the imported seed. |
| `cover keys` | Every uncovered key/interval, or the overlapping subscriber registrations sharing a key. |
| `count` | Actual count of distinct declared loci and their names. |

---

## One law, many entrypoints

In a multi-binary system, copying a `claims { }` block across entrypoints risks drift. A **constitution** ([Constitutions](./constitutions.md)) declares a named set of claims once at the top level so multiple `main locus` entrypoints can `adopt` it:

```hale
type Settlement { n: Int; }
topic Settled { payload: Settlement; }

locus Billing {
    bus { publish Settled; }
    fn settle(n: Int) { Settled <- Settlement { n: n }; }
}

locus Ledger {
    params { total: Int = 0; }
    bus { subscribe Settled as on_settled; }
    fn on_settled(s: Settlement) { self.total = self.total + s.n; }
}

locus Research {
    params { notes: Int = 0; }
    fn note() { self.notes = self.notes + 1; }
}

group billing  = { Billing };
group research = { Research };

constitution Core {
    tenant_iso: forbid reaches(billing, research);
    one_writer: count publishers(topic Settled) == 1;
}

main locus App {
    params {
        b: Billing = Billing { };
        l: Ledger = Ledger { };
        r: Research = Research { };
    }
    claims {
        adopt Core;
        local_rule: require publishes(some billing, topic Settled);
    }
}

fn main() { App { }; }
```

### Extending Constitutions (`extends`)

A constitution can extend one or more base constitutions using `extends`:

```hale
constitution Dev extends Core {
    no_real_payments: forbid reaches(app, payment_provider);
}
```

Composition is strictly **additive (union-only)**: a derived constitution can add new named claims, but cannot override or weaken an inherited claim name:

```hale
constitution B extends A { rule: count publishers(topic T) == 9; }
//                         ^ compile error if `A` already declares `rule`
```

Diamond inheritance (`D extends B, C` where both `B` and `C` extend `A`) deduplicates `A`'s clauses by declaration origin automatically.

### Binding a constitution to a deployment target

To apply environment-specific constitutions (such as `Dev` vs. `Prod`) without modifying source files, configure `[claims]` and `[environments.<name>]` in `hale.toml`:

```toml
[claims]
base = "Core"                 # Enforced in EVERY environment (or set `no_base = true`)

[environments.dev]
constitution = "Dev"          # Merged with `base` for dev checks
entrypoints  = ["apps/prober", "apps/dashboard"]

[environments.prod]
constitution = "Prod"         # Use `source_only = true` if prod adds no extra constitution
entrypoints  = ["apps/prober"]
```

Check a single environment, run the full `(entrypoint, environment)` matrix, or inject a constitution ad-hoc from the CLI:

```sh
hale check apps/prober --env prod              # Checks apps/prober with Core + Prod
hale check --matrix                            # Checks all (entrypoint, environment) pairs in hale.toml
hale check apps/prober --adopt std::law::Standard  # Injects a constitution for one run
```

- **`std::law::Standard`**: Built-in standard library constitution that enforces structured logging (`logs_structured: forbid reaches(program, effects(stdio)) avoiding std::log::*;`), rejecting raw `println` calls in application code.
- **Required Group Vocabulary in Adopting Entrypoints**: Every group referenced by an adopted constitution (other than the built-in `program` group) must be declared in the adopting entrypoint (or imported from a shared policy seed). If an entrypoint intentionally omits a component, declare an explicit empty group:

```hale
group payment_provider = { } may_be_empty;
```

### Checking Multi-Seed Workspaces (`--workspace`)

Because `hale check <dir>` checks a single seed (compilation unit), use `--workspace` to check every seed and `*_test.hl` file in a repository:

```sh
hale check --workspace .
hale verify --workspace .
```

`--workspace` evaluates every seed and test file even if an earlier one fails, skipping `vendor/`, `target/`, and dot-directories. Because each seed has its own model, single-seed artifact flags (`--dump-topology`, `--check-topology*`) cannot be combined with `--workspace`.

---

## The Topology Artifact (`.hale.topology`)

`hale check` can export the verified program model, inferred effects, and claim verdicts as a deterministic, cryptographically digested JSON artifact (`.hale.topology`):

```sh
# Export topology to stdout or a file (note: file path requires `=`)
hale check app --dump-topology > .hale.topology
hale check app --dump-topology=.hale.topology

# CI Gates:
hale check app --check-topology .hale.topology        # Exact snapshot gate (model + law + spans)
hale check app --check-topology-shape .hale.topology  # Model identity gate (`shape_hash` only)
```

| Gate Flag | What It Compares | When to Use |
|---|---|---|
| `--check-topology <file>` | Whole-file `artifact_digest` (model, claims, topics, and byte-offset provenance spans). | Strict review gate where any source motion, claim rename, or model change requires updating the baseline. |
| `--check-topology-shape <file>` | Model `shape_hash` (loci, functions, call/bus relations, groups, phases, derived effects, unknowns) plus `artifact_digest` integrity verification. | CI gate that ignores line-number shifts, comment edits, and claim renames while catching any structural or effect change. |

### Visualizing Diagrams (`hale topology graph`)

Render deterministic SVG or Mermaid architecture diagrams directly from a committed `.hale.topology` file without touching source files:

```sh
hale topology graph .hale.topology                          # SVG to stdout (default `system` view)
hale topology graph .hale.topology --format mermaid
hale topology graph .hale.topology --view claim --claim apart -o claim.svg
```

Available `--view` modes:

- `system`: Loci, functions (with phase and effect badges), topics, and call/publish/subscribe edges.
- `code`: Functions and call-graph edges only.
- `bus`: Bus topics, publishers, and subscribers only.
- `claim`: Highlights the groups and verdict of the claim specified by `--claim <name>`.
- `residue`: Highlights unresolved analysis holes as first-class nodes.

### Semantic Model Diffing (`hale model diff`)

Compare two topology artifacts semantically to review structural, contract, effect, and claim changes between revisions:

```sh
hale model diff before.topology after.topology --text
hale model diff before.topology after.topology          # Outputs versioned JSON diff
```

```text
hale model diff  before.topology  ->  after.topology
classification: model-shape  (shape_hash 560bb0… -> 578951…)
legend: + added  - removed  ~ renamed  > moved  * split/joined  ? ambiguous  ! changed in place
classes: identical · source-only (no model or contract change) · contract (a contract or effect moved, shape_hash unmoved) · model-shape
declarations:
  ~ locus Worker -> Crew  (renamed; shape unchanged)
  + locus Pager  (main.hl bytes 248..253)
  + topic Alerts  (main.hl bytes 71..77)
contracts:
  ! locus App params: +limit: Int
  ! locus App publishes: +Alerts
effects:
  ! fn Crew::on_e gains publish
law:
  + claim quiet: count publishers(topic Out) == 0  [violated]
```

### Topology Artifact Schema (`1.21`)

```json
{
  "schema": "1.21",
  "semantics": 1,
  "shape_hash": "<fnv1a-64 over the model half>",
  "sorts":     { "loci": ["..."], "fns": ["..."], "topics": ["..."] },
  "sealed":    [ "<@sealed loci>" ],
  "relations": {
    "calls": [ {"from": "...", "to": "...", "loop": true, "unbounded": true, "via_interface": "<iface>"} ],
    "calls_via_stdlib": [ {"from": "...", "to": "...", "loop": true} ],
    "publishes": ["..."],
    "subscribes": ["..."]
  },
  "groups":    { "<name>": ["<members as declared>"] },
  "labels":    { "<fn>": ["<declared effect classes>"] },
  "phases":    { "<fn>": {"phase": "...", "kind": "hook|method"} },
  "seeds":     { "<alias>": ["<member decls>"] },
  "effects":   { "<fn>": ["<derived effect classes>"] },
  "supervision": [ {"locus": "...", "child": "...", "err": "...", "ops": ["..."], "retry_bound": 3} ],
  "unknowns":  [ {"fn": "...", "reasons": ["indirect_call | untyped_receiver_call:<callee> | uninhabited_interface_call:<iface>.<callee> | computed_publish"]} ],
  "endpoint_identity": [ {"verb": "...", "fn": "...", "wire": "...", "topic": "..."} ],
  "sources":   [ {"id": 0, "path": "...", "digest": "..."} ],
  "provenance": { "calls": ["..."], "publishes": ["..."], "subscribes": ["..."], "decls": {}, "supervision": ["..."] },
  "topics":    [ {"name": "...", "subject": "...", "shape": "...", "payload_hash": "..."} ],
  "endpoints": [ {"verb": "publish|subscribe", "subject": "...", "via": "site|declaration"} ],
  "declares_publish": [ {"locus": "...", "subject": "..."} ],
  "contracts": [ {"locus": "...", "sealed": false, "params": [], "methods": [], "publishes": [], "subscribes": [], "supervises": [], "instances": []} ],
  "bindings":  [ "<declared bindings: topic, transport, role>" ],
  "observe":   [ {"param": "...", "kind": "...", "backend": "...", "host": "...", "port": 0, "path": "..."} ],
  "claims":    [ {"name": "...", "form": "...", "result": "holds|violated|uncertified|invalid", "ordinal": 0, "source": "Core"} ],
  "lowered":   [ {"subject": "...", "form": "...", "result": "holds", "ordinal": 0} ],
  "law":       { "law_digest": "...", "inputs_digest": "...", "fn_universe": [], "loci": [], "groups": [], "topics": [], "subjects": [], "effect_classes": [], "legacy": [], "issues": [], "rows": [] },
  "capabilities": { "exact_calls": true },
  "adequacy":  { "reachability": "exact", "boundary": "exact", "endpoint": "exact", "bound": "exact", "certificate": "exact", "causes": "exact", "depends": "exact", "budget": "exact", "placement": "exact" },
  "evaluation": { "environment": "prod", "roots": [], "closure": [] },
  "verdict":   "clean | law_failed",
  "artifact_digest": "<digest over entire document>"
}
```

Key artifact sections:

- **`shape_hash` vs. `artifact_digest`**: `shape_hash` hashes only the structural model half (`sorts`, `relations`, `groups`, `labels`, `phases`, `seeds`, `effects`, `supervision`, `unknowns`, `endpoint_identity`). `artifact_digest` hashes the entire JSON document to prevent tampering. Both `hale topology graph` and `hale fleet check` verify `artifact_digest`, `shape_hash`, `law_digest`, and `inputs_digest` before admitting an artifact.
- **`calls_via_stdlib`**: Collapses any user-to-user call path traversing standard library internals into a single contracted edge so external artifact verifiers can replay reachability without stdlib source.
- **`lowered`**: Records every function-level `@effects`, `@budget`, and `@phase_effects` contract in its equivalent relational claim form alongside top-level `claims`.

---

## Related Reference

- **Specification**: [`spec/verification.md § Claims`](https://github.com/hale-lang/hale/blob/main/spec/verification.md).
- **Reusable Claimsets**: [Constitutions](./constitutions.md).
- **Program Model**: [The Model](./the-model.md).
- **Multi-Binary Fleet Claims**: [Multi-Binary Services](./services/multi-binary.md#multi-binary-verification-fleet-locus).
