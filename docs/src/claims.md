# Claims & the law

The [effects chapter](./effects.md) taught one function to make a
promise. This chapter is the complete reference for the promises no
single function can make: **claims** — named sentences over the
assembled program graph, declared on the main locus, evaluated by
`hale check` as errors, and lowered to zero runtime code.

The chapter is organized as a reference: the grammar first, then
each declaration and verb with its exact semantics, the evaluation
model, every diagnostic the surface can produce, and the topology
artifact. If you want the workflow story instead — write the law
first, let countermodels drive implementation — read
[Claim-Driven Development in Hale](https://hale-lang.org/articles/claim-driven-development-in-hale/).

## The worked example

One program exercises most of the surface. Two wings, one boundary
temptation:

```hale
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

This program fails to check, and the diagnostic returns the route:

```text
claim `iso_dg` violated: `delta_wing` reaches `gamma_wing` —
witness: `DeltaTriage::on_task` -(publishes "Metrics")-> `GammaResearch::on_metric`
```

Everything below is the precise definition of what just happened,
plus every other sentence the surface can state.

## The grammar

The complete surface, as it appears in
[`spec/grammar.ebnf`](https://github.com/hale-lang/hale/blob/main/spec/grammar.ebnf):

```text
group_decl        = "group" , IDENTIFIER , "=" , "{" ,
                    [ group_member , { "," , group_member } , [ "," ] ] ,
                    "}" , [ "may_be_empty" ] , ";" ;
group_member      = IDENTIFIER , { "::" , IDENTIFIER } , [ "::" , "*" ] ;

domain_decl       = "domain" , IDENTIFIER , "=" , "{" ,
                    IDENTIFIER , { "," , IDENTIFIER } , [ "," ] ,
                    "}" , ";" ;
effect_family_decl
                  = "effect" , IDENTIFIER , "(" , IDENTIFIER , ")" , ";" ;

claims_block      = "claims" , "{" , { claim_entry } , "}" ;
                  (* inside main locus: the world tier;
                     at top level: the library tier (#392) *)
claim_entry       = IDENTIFIER , ":" , claim_form , ";" ;
claim_form        = forbid_form | only_edges_form | bound_form
                  | require_form | cover_form | count_form ;

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

require_form      = "require" , ( "subscribes" | "publishes" ) ,
                    "(" , "some" , IDENTIFIER , "," , "topic" ,
                    topic_ref , ")" ;

cover_form        = "cover" , "topic" , "in" , "seed" , "(" ,
                    IDENTIFIER , ")" , ":" , "subscribed_by" ,
                    "(" , "some" , IDENTIFIER , ")" ;

count_form        = "count" , ( "publishers" | "subscribers" ) ,
                    "(" , "topic" , topic_ref , ")" ,
                    ( "==" | "<=" | ">=" ) , INT_LIT ;

claim_set         = IDENTIFIER
                  | "effects" , "(" , effect_class_ref , ")" ;
topic_ref         = IDENTIFIER , { "::" , IDENTIFIER } ;
effect_class_ref  = IDENTIFIER , [ "(" , ( IDENTIFIER | "*" ) , ")" ] ;
```

Every introducer — `group`, `domain`, `claims`, `forbid`, `reaches`,
`via`, `during`, `avoiding`, `only`, `edges`, `bound`, `require`,
`cover`, `count`, `seed`, `may_be_empty`, and the predicate words —
is a **contextual keyword**: recognized in position, still usable as
an ordinary identifier everywhere else. The exceptions are words
that were already hard keywords (`bus`, `publish`, `subscribe`,
`on`, `in`), which simply appear as themselves.

## `group` — the vocabulary

```hale,fragment
group delta_wing = { delta::*, DeltaStore, helper_fn };
group gamma_wing = { gamma::Research };
group probes     = { } may_be_empty;
```

A group names **declared program elements**: loci, free functions,
and imported declarations. Membership is spelled out, never
pattern-matched.

**Member forms.**

| form | meaning |
|---|---|
| `Name` | a locus or free fn declared in the bundle. If both a locus and a fn share the name, both join. |
| `alias::Name` | an imported declaration. Canonicalized to the merged declaration at the mangle stage — the same path qualified topic references take — never by name-suffix matching. |
| `alias::*` | every locus and free fn the seed imported as `alias` declares, enumerated through the same rename table `alias::Name` resolves through. Trailing-only, single level: `a::*::b` and `a::b::*` (nested) are rejected. |

Types, topics, and constants matched by a glob are silently skipped
— they are not path vertices, so they contribute nothing a
`reaches` walk could use. A glob still counts as *resolved* if the
alias exists at all.

**Guards.** Every one of these is an error, not a silent
degradation:

- an unknown single-segment member — with a did-you-mean over
  declared loci and fns;
- a qualified member matching no import (`does not resolve`);
- a glob over an alias this bundle never imports;
- a nested glob (parse error: the glob is trailing-only, like `**`
  in bus subjects);
- a group declared twice;
- a group that resolves to **zero declarations**, unless it says
  `may_be_empty` — a claim quantifying over an empty group holds
  vacuously, which is a fail-open wearing formal clothing;
- a group whose declarations have **no executable surface** (all
  pure-data loci), used at a `forbid` or `only edges` endpoint or a
  `bound` source — *projection vacuity*: the vocabulary is
  non-empty but the fn-grained walk would prove nothing, so the
  claim refuses instead of passing.

**Projection.** Claims evaluate at the function grain. A locus in a
group projects to **all** of its methods, lifecycle hooks
(`birth`, `accept`, `release`, `run`, `drain`, `dissolve`), and
modes. Projection only ever *adds* sources and sinks — the
conservative direction. A free fn projects to itself.

## `domain` and effect families — the data-plane vocabulary

```hale,fragment
domain wing = { delta, gamma };

effect llm;
effect knowledge(wing);

@effects(is: {knowledge(delta)})
fn read(key: String) -> Idea { ... }
```

`domain` declares a **closed index set**. `effect NAME(domain);`
declares an **indexed family**: every instantiation
(`knowledge(delta)`, `knowledge(gamma)`) is interned as an ordinary
declared class, and `NAME(*)` as a composed class whose mask is the
union of every instantiation. Because the reduction lands on the
ordinary class machinery, everything composes with no new rules:

- instantiations appear anywhere a class name does —
  `@effects(is:/only:/none:/causes:)` lists, composed-class
  definitions (`effect sensitive = { knowledge(delta) };`), claim
  sinks (`effects(knowledge(delta))`, `effects(knowledge(*))`),
  `bound`, and `@budget` keys;
- a **misspelt index** (`knowledge(delt)`) interns a name nothing
  declared and is rejected with a did-you-mean over declared
  instantiations;
- an `@effects(only: {knowledge(delta), …})` contract computes its
  complement from the live class universe over **atomic** classes,
  so a domain member added later lands *outside* every existing
  contract — the fail-closed direction, inherited rather than
  re-derived;
- instantiations travel across seed boundaries through the merged
  class table like any user class.

**Rules.** The domain must be declared **earlier in the same file**
as the family (this is what lets every instantiation be expanded
eagerly). An empty domain is a parse error (every family over it
would be vacuous). A duplicate domain is a parse error. Family
instantiations count against the same effect-mask capacity as every
class — a family over a large domain that overflows the mask is
rejected at the declaration, fail-closed.

**The companion budget.** `@budget(<user class> = N)` bounds calls
to declared carriers of the class along any path, with the standard
per-call rules: a carrier inside a loop is unbounded, an
unresolvable edge is unbounded, an undeclared class name is an
error with a did-you-mean, a duplicate dimension is a parse error,
and a *built-in* class as a key is a parse error (the counted
built-ins keep their own spellings: `alloc_per_call`, `publish`,
`block_points`, `stack_bytes`, `fanout`).

## `claims { }` — placement and naming

The block has two homes, one per tier:

- **Inside `main locus`** — the WORLD tier. Main is the
  closed-world gate: bundle-wide sentences cannot be evaluated
  before the whole application graph exists, and
  one-main-per-bundle makes the claims root unique. Inside any
  other locus it is a parse error.
- **At top level in a LIBRARY seed** — the library tier (see
  [Library-tier claims](#library-tier-claims) below): a seed
  swears about itself and its own boundary, and the block travels
  with the import. A seed that declares `main locus` writing the
  top-level form is a check error — world law belongs in main.

A main locus may carry several `claims { }` blocks; their entries
concatenate.

Every entry is `name: form;`. The **name is the contract of
record** — it is what the diagnostic, the CI check, the review
policy, and the topology artifact cite — and it must be unique
across all blocks (a duplicate is an error). Claims gate
`hale check` as **errors**; there is no advisory tier, because a
warning that reads "tenant isolation is false" is a law that
doesn't bind. Weakening a claim requires a source diff — deleting
a `forbid`, widening a grant list, raising a bound — and that diff
is the review event.

## `forbid reaches(SRC, DST)` — absence under closure

```hale,fragment
iso: forbid reaches(delta_wing, gamma_wing);
iso_calls: forbid reaches(delta_wing, gamma_wing) via { calls };
data_iso: forbid reaches(delta_wing, effects(knowledge(gamma)));
quiet_boot: forbid reaches(positions, effects(llm)) during birth;
gated: forbid reaches(public_api, ledger) avoiding authorization_gate;
```

**Sets.** `SRC` must be a group. `DST` is a group or
`effects(<class>)` — the class may be a built-in, a user class, a
composed class, a family instantiation, or a family star
(`effects(...)` in *source* position is an error).

**The relation.** The walk starts from every fn in SRC's projection
and composes two edge kinds:

- **`calls`** — resolved call-graph edges: free-fn calls, `self`
  methods, handle methods on typed receivers, interface-slot
  defaults, calls into imported seeds, and calls through the
  Hale-source stdlib (its bodies are merged into the walk, so a
  chain through a stdlib locus resolves rather than stopping at
  the boundary).
- **`bus`** — for each publish site *in the visited fn's own body*
  with a statically-known subject, an edge to every subscriber
  handler of that subject — including subscribers whose `**`
  wildcard pattern covers it (a `log.**` sink is an edge from
  every `log.x` publish; more edges is the conservative
  direction).

`via { calls }` or `via { bus }` restricts the composition;
omitting `via` selects both — the conservative default. `via { }`
with no relations is a parse error. Note the precise meaning of
`via { bus }`: it composes *declared wiring* — a publish reached
only through a call chain needs `calls` in the relation to be
seen.

**Hits.** A visited fn is a violation if it is in DST's projection
(group form) or if its **direct** effects intersect the class mask
(effects form: its `is:` classification, its allocation sites, its
publish sites, its classified stdlib and FFI leaf calls). Roots
are tested too: a declaration in *both* groups is a zero-length
path and reports as a violation — boundary confusion is surfaced,
not skipped.

**Vacuity.** An empty DST group (via `may_be_empty`) forbids
nothing: the claim holds. An empty SRC likewise. Both are only
reachable through the explicit opt-out; everything else fails the
group guards first.

**`during <phase>`.** Restricts SRC's projection to the members of
`<phase>` in the model's **phase relation**: lifecycle hooks
(`birth`, `accept`, `release`, `run`, `drain`, `dissolve`) and
modes (`bulk`, `harmonic`, `resolution`) are hook-phases the
runtime drives; any method or handler name is its own source-slice
phase. Free fns have no phases and drop out. If the filter empties
a non-empty projection, that is an error ("phase names nothing in
group"), not a vacuous pass. The relation is exported in the
topology artifact (`phases`), which is what makes a `during` row
independently re-derivable; it is still a source slice, not a
temporal logic.

**`avoiding <group>`.** Masks the named group's vertices out of
the walk entirely — neither traversed nor tested. This is the
**interposition form**: "every path from A to B passes through G"
is literally "no path from A to B exists while G is masked". The
mask must be disjoint from both endpoints: masking the target
would hold vacuously, masking a source silently drops roots, and
either overlap is an error.

Modifiers stack in any order:
`forbid reaches(a, b) via { calls } during birth avoiding gate;`.

**The witness.** One minimal countermodel per violated claim: the
path from a source root to the hit, calls rendered `->`, bus hops
rendered `-(publishes "subject")->`, every name in author spelling
(cross-seed symbols demangled).

**Where to edit.** The witness names *who*; secondary diagnostics
point at *where* — the call that crosses the boundary (or, for a
bus hop, the publish site and the subscription declaration) and
the forbidden destination's declaration, in the effect system's
root + leaf shape. A hop whose source lives inside a stdlib body
renders by name alone: stdlib source parses in its own offset
space, and a span from there would point at the wrong line of your
file.

## `only edges A -> B { … }` — the boundary form

```hale,fragment
grant: only edges gamma_wing -> delta_wing {
    publish t::ResearchDigest;
};
```

Every **direct** edge from A into B must match a granted line;
every un-granted edge is reported (all of them, not just the
first — the grant list is the review surface, so the full diff
matters).

- A grant names a declared topic. `publish T` and `subscribe T`
  admit **the same edge** — the verb documents which end's
  declaration a reviewer should go read.
- **Call edges are never grantable.** A direct call from A into B
  is always an un-granted edge: a permitted cross-domain
  dependency should be a named bus edge, not an invisible method
  call.
- A subscriber *outside* B is not an A→B edge — the shared
  `log.**` sink needs no grant.
- An edge on a **literal or wildcard subject** cannot be granted
  (grants name declared topics) and therefore fails closed if it
  lands in B — restructure onto a declared topic.
- Direction matters: `only edges A -> B` says nothing about B→A
  edges. And it is a *direct-boundary* property, not an exception
  to `forbid`: a granted edge still creates a path, so a blanket
  `forbid reaches(A, B)` over the same direction would reject it,
  and a boundary enumeration does not exclude transitive routes
  through a third group. Write the sentence the architecture
  means.

## `bound C <= N on paths from G` — capability budgets

```hale,fragment
one_call: bound llm <= 1 on paths from positions;
```

`C` must be a **user-declared** class (a built-in here is an error
pointing at the `@budget` spellings); composed classes and family
instantiations work through their masks. The quantity is the
**per-invocation aggregate** — a call-tree sum, exactly
`@budget`'s semantics: if a handler calls two helpers and each
reaches the model once, the count is two. It does not become one
because each root-to-leaf chain contains one call. Bus hops are
followed: a publish contributes every subscriber's count.

Unbounded — and therefore a violation of any finite bound — are: a
carrier reachable inside a loop, a carrier under recursion, an
indirect call, an unresolvable receiver, a computed publish
subject. An operation that *might* hide a carrier must prevent
certification rather than count as zero.

The violation carries the measured count and a representative
heaviest chain; the unbounded case names which condition made it
uncountable.

## `require` — existence

```hale,fragment
wired: require subscribes(some delta_wing, topic t::Tasks);
feeds: require publishes(some delta_wing, topic t::Done);
```

At least one **locus** member of the group must *declare* the
subscription (or publication) in its `bus { }` block — "wired" is
a declaration property, checked over the declared bus ends. The
violation names the group and the topic. The topic reference must
resolve to a declared topic (qualified refs canonicalize at the
mangle stage; unknown names error with a did-you-mean).

## `cover` — bounded universals

```hale,fragment
no_orphans: cover topic in seed(t): subscribed_by(some positions);
```

Every topic declared by the seed imported as `t` (the claiming
file's own import alias — there is no global seed namespace) must
have a subscriber in the group. The violation lists **every**
uncovered topic. An alias that names no imported topic vocabulary
is an error: an empty coverage domain would hold vacuously.

## `count` — cardinality

```hale,fragment
single_writer: count publishers(topic t::Tasks) == 1;
consumed: count subscribers(topic t::Done) >= 1;
capped: count subscribers(topic t::Audit) <= 2;
```

Counts **distinct declared loci** on the chosen end of the topic.
`== 1` is the single-writer invariant. A violation reports the
actual count and names the participating loci. These are counts
over declarations, not a runtime census of replicated instances —
exact instance claims belong to deployment elaboration, when it
lands.

## Library-tier claims

A library seed states its own law in a **top-level** `claims { }`
block — no `main locus` required:

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

Two properties make this the *library* tier:

- **The law travels.** Import the seed and its claims come along,
  re-evaluating in **every closing build** over the merged world.
  Checked standalone, the seed satisfies itself; when an app
  quietly wires a second subscriber onto `Charges`, the library's
  own `single_settle` refuses the build — reported with seed
  attribution (``claim `pay::single_settle` violated``, never a
  mangled symbol) and pointing at the library's own claim line.
  Getting stricter as the world grows is the point: the claim
  quantifies over whatever the merge added.
- **The tier is the enforcement surface.** A seed swears about
  *itself and its own boundary* — it can only name what it can
  see: its own decls, its own imports. World-quantification stays
  main's, and a seed that declares `main locus` writing the
  top-level form is a check error. A dependency cannot brick a
  downstream build with world-claims because it has no way to
  state one.

Claim names stay per-seed (`pay::single_settle` and your own
`single_settle` coexist); group and topic references inside a
traveling block canonicalize across the seed boundary exactly as
group declarations do.

## The evaluation model

What the sentences quantify over:

```text
sorts:      fns, loci, topics (+ subjects)
relations:  calls        resolved call edges, stdlib bodies merged
            publishes    fn -> statically-known subject
            subscribes   subject -> (locus, handler), wildcards included
labels:     effect classes (declared carriers via is:), groups
weights:    carrier-site counts (bound, @budget)
```

Groups name declarations; each verb projects them onto the sorts
its relation needs; evaluation is fn-grained. Each claim produces
one of three results — `holds`, `violated`, or `invalid` (a
vocabulary or reference error prevented evaluation) — and the
result set is what the artifact records.

**The fail-closed rules.** Deriving the graph is the trust root,
and the direction is fixed: *uncertainty may add possible edges;
it may never delete an edge and report success.* Concretely, any
judgment that traverses calls refuses to certify over:

- an **indirect call** through a function-typed parameter;
- a **method call on a receiver the compiler cannot type**. The
  summarizer types bare vars, `self` fields, chained fields
  (through locus and struct field maps), struct-literal receivers,
  known-fn call results, uniform `if/else` values, `or`-unwrapped
  values, single-slot collection `.get(i)` results, and
  `for`-binders over array fields, capacity slots, array params,
  array literals, and the implicit accepted-children collection.
  What remains — an index result, a `match` value, a foreign
  expression — fails closed, with a diagnostic that says how to
  fix it: bind the receiver to a typed field or local. The same
  rule applies in the effect and budget walkers, so a fn-level
  certificate and a bundle-level claim always agree;
- a **computed publish subject** (it could route to any
  subscriber);
- a walk exceeding the step ceiling.

**Interface dispatch fans out.** A method call on an
interface-typed value — `route.handler.handle(ctx)`, the stdlib
router's own shape — is not an unknown: the world is closed, so
the implementor set is enumerable. The summarizer fans the one
written edge out to every conforming locus (structural
name-and-arity conformance over the declarations — a superset of
the checker's typed conformance, safe because over-approximation
only adds edges). Reachability and effect judgments walk every
alternative; counting judgments (`bound`, `@budget`, the
quantitative dims) take the **max** over one dispatch site's
alternatives, because an invocation dispatches to exactly one
target — a sum would count phantom calls no execution performs.

An interface *no* locus conforms to is different again: an
interface value only ever arises by coercing a conforming locus,
so in a closed world an uninhabited interface has no values and
its call sites are **dead** — they contribute nothing to any
judgment (the router's `m.before(cur)` over an empty middleware
list is the everyday case). The artifact records each such site
(`uninhabited_interface_call:<interface>.<callee>`) inside the
hashed model half, so a conformer appearing in a later build
changes `shape_hash`.

## Every diagnostic

The complete catalog, grouped by stage. Parse errors:

| condition | shape |
|---|---|
| `claims { }` outside `main locus` | "only valid inside `main locus`" |
| unknown claim verb | lists the six verbs |
| `via { }` with no relations / unknown relation | "must name at least one relation" / "the composable relations are `calls` and `bus`" |
| nested glob in a group member | "the glob is trailing-only" |
| negative bound or count | "must be non-negative" |
| empty or duplicate `domain` | "has no members" / "declared more than once" |
| family over a domain not in this file | "declare `domain X = { … };` above the family" |
| effect-mask overflow (incl. family expansion) | rejected at the declaration, fail-closed |
| duplicate `@budget` dimension / built-in class as budget key | "state it once" / points at the built-in spellings |

Vocabulary and reference errors (evaluation refuses; the claim's
result is `invalid`):

| condition | shape |
|---|---|
| unknown group member | "names no declared locus or fn" + did-you-mean |
| unresolved qualified member / topic ref | "does not resolve — no imported declaration matches" |
| glob over an unknown alias | "names no import alias" |
| duplicate group / duplicate claim name | "declared more than once" |
| empty group without `may_be_empty` | "holds vacuously — say `may_be_empty`" |
| projection vacuity at an endpoint | "projects to no executable … vertices" |
| unknown group in a claim | + did-you-mean over declared groups |
| `effects(...)` in source position | "sources must be declared groups" |
| undeclared effect class / misspelt family index | "never declared" + did-you-mean |
| built-in class in `bound` | points at the `@budget` spellings |
| unknown topic in a grant / `require` / `count` | + did-you-mean over declared topics |
| `cover` over an alias with no topics | "the coverage domain would be empty" |
| `during` phase naming nothing in the group | "a claim over an empty phase holds vacuously" |
| `avoiding` overlapping an endpoint | "masking an endpoint makes the claim weaker than it reads" |

Violations (the claim's result is `violated`):

| claim | what the diagnostic carries |
|---|---|
| `forbid` | the minimal countermodel path, bus hops named |
| `only edges` | every un-granted edge + the granted list |
| `bound` | the measured count + representative chain, or the unbounded reason |
| `require` | the group and the missing declaration |
| `cover` | every uncovered topic |
| `count` | the actual count + the participating loci |
| any call-traversing claim | "cannot be certified" for indirect calls, untypeable receivers, computed subjects — with the repair named |

## One law, many entrypoints

`claims { }` is only legal inside `main locus`, and that rule is
load-bearing: claims are closed-world statements, and `main` is the
only place a world is closed. But that constrains *evaluation*, not
*authoring*. Copy-pasting a law into twenty main loci means the copy
somebody forgets fails open, silently.

A **constitution** is a named claimset declared once, outside any
main, and adopted by each entrypoint:

```hale
constitution Core {
    tenant_iso: forbid reaches(billing, research);
    one_writer: count publishers(topic Settled) == 1;
}

main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims {
        adopt Core;
        local_rule: require publishes(some billing, topic Settled);
    }
}
```

Every clause is still evaluated **here**, in this entrypoint's closed
world. One text, N evaluations, N worlds — nothing about the
soundness argument changes.

### Environments compose it

```hale
constitution Dev extends Core {
    no_real_payments: forbid reaches(app, payment_provider);
}
```

An environment may **add** laws, never drop them. Development wanting
*stricter* law than production is the safe direction and a genuinely
useful one — "nothing reaches the real payment provider", satisfied by
the dev entrypoint wiring a stub. The dangerous direction is the
reverse: a law in production that development lacks means development
never exercised production's architecture, and the first violation
surfaces at the production build.

Composition is **union, and nothing else**. A derived constitution
cannot replace an inherited clause:

```hale
constitution B extends A { rule: count publishers(topic T) == 9; }
//                         ^ error if `A` also declares `rule`
```

That looks like a restriction and is actually the point. If override
were allowed, weakening would be expressible and would read exactly
like ordinary composition at the adoption site — and telling
strengthening from weakening means proving one sentence implies
another, which fails open when it gets that wrong. With union only, a
stricter bound is simply a second named claim that coexists with the
inherited one. Both are checked; the stricter one does the work.
Weakening isn't rejected, it's **unexpressible**.

Two mechanics follow. Claim names are one flat namespace, so two
constitutions declaring one name is an error naming both origins —
and so is a local clause shadowing an adopted one. A **diamond** (two
constitutions extending a common base) contributes the shared base
exactly once, deduped by origin rather than by name, so a real
two-origin collision still surfaces.

### Binding a constitution to a deployment target

`adopt Core;` in source means *always, everywhere*. But an entrypoint
deployed to two environments must satisfy both claimsets, and it
cannot write two conflicting `adopt` lines. So the environment binding
lives in `hale.toml`, where the deployment facts already are:

```toml
[claims]
base = "Core"                 # carried by EVERY environment

[environments.dev]
constitution = "Dev"          # …and dev adds this
entrypoints  = ["apps/prober", "apps/dashboard"]

[environments.prod]
constitution = "Prod"
entrypoints  = ["apps/prober"]
```

The base is what makes "an environment may add law, never drop it"
true of the mechanism rather than of convention — every evaluation
carries it by construction. A workspace with environments must decide
explicitly: `base = "…"` or `no_base = true`. An environment adding
nothing of its own says `source_only = true`. In each case an omission
would be indistinguishable from a typo.

```sh
hale check apps/prober --env prod    # adopts Prod for this run
hale check --matrix                  # every (entrypoint, environment) pair
```

`--env` injects the environment's constitution exactly as if the
source had written `adopt` — same evaluation, same closed world,
union with whatever the source already adopts. So one entrypoint gets
two verdicts, one per environment, without the source knowing where it
will be deployed.

`--matrix` checks every pair. **An entrypoint listed in no environment
is an error**, not a skip — that is the one hole composition cannot
close by construction, since no single compilation can see that a
sibling was left out. A seed with no `main locus` is not an entrypoint
and is not demanded of the manifest.

### Groups must be declared by every adopting entrypoint

A constitution applied to every entrypoint cannot name any one
application's internals. It names **shared vocabulary** — but that
vocabulary has to exist in each adopting entrypoint, and an
*undeclared* name is an error, not an empty set:

```
type error: claim `iso` names group `payment_provider`, which is never
declared. Add `group payment_provider = { … };` at the top level.
```

`may_be_empty` applies only to a group that IS declared and resolves
to zero members. So an entrypoint that genuinely lacks a component
declares the vocabulary and says so:

```hale
group payment_provider = { } may_be_empty;
```

The usual way to satisfy this is for the entrypoints to import the
same seed that declares both the constitution and its groups — then
the vocabulary arrives with the law. An entrypoint that deliberately
lacks a component writes the empty group explicitly, which is a line
a reviewer can see rather than an absence they must infer.

### Provenance, not annotation

Once environments can add laws, two kinds of sentence live in one
block with opposite lifetimes: **product laws**, true everywhere, and
**environment rails** like "nothing reaches the real payment
provider", deliberately false in production. They read identically.

So the artifact records which constitution each clause came from
rather than asking you to mark them:

```json
{"name": "tenant_iso", "form": "…", "result": "holds", "source": "Core"}
```

A clause written in the main itself has no `source`. The distinction
is structural and cannot drift from reality the way a hand-applied
marker can.

## Checking a repository with many seeds

`hale check` operates on **one seed** and does not recurse — a
directory is one compilation unit, and that is the right unit for a
closed-world check. The consequence is that a repository with several
seeds needs something to enumerate them, or a claim is enforced only
where somebody remembered to point `check`.

```sh
hale check --workspace .        # every seed under `.`, each on its own
hale verify --workspace .       # same, with advisories gated too
```

Every seed runs even if an earlier one fails — a runner that stopped
at the first failure would report a subset of the truth. The summary
names which seeds failed, and the exit status is the worst of them, so
a usage error is not masked by an ordinary check failure elsewhere.

`vendor/`, `target/` and dot-directories are skipped: a seed you do
not own is not yours to gate.

What this does **not** do is connect seeds to each other. Each stays
its own closed world with its own model. Two binaries that publish and
subscribe the same topic are not linked by this command — nothing
about a deployment is visible from source alone, and inventing those
edges would certify a system nobody deploys. Per-seed artifact flags
(`--dump-topology`, `--check-topology*`, the effects and budget
equivalents) are therefore rejected in combination with `--workspace`:
N seeds are N models, and there is no single artifact to emit or gate.

## The topology artifact

The checked model exports, diffs, and gates:

```text
hale check app --dump-topology > .hale.topology
hale check app --dump-topology=.hale.topology     # or write it directly
hale check app --check-topology .hale.topology    # exact snapshot gate
hale check app --check-topology-shape .hale.topology   # model-only gate
```

The two gate flags take their operand either way (`--flag value` or
`--flag=value`), and a missing operand is a usage error rather than a
silent no-op — as is an unknown flag or a second target.

`--dump-topology` is the exception: its destination is the
`=<path>` form **only**, and a bare `--dump-topology` writes to
stdout. Its operand is optional, so "take the next token" has no
safe reading — `hale check --dump-topology app.hl` would be asking
whether `app.hl` is the destination or the target, and guessing
wrong overwrites your source.

Dumping does **not** change what the command means: a program whose
claims fail still exits non-zero with its witnesses, it just prints
the artifact on the way.

### Rendering the artifact

A committed artifact can be drawn (experimental surface, pre-1.0):

```text
hale topology graph .hale.topology                          # SVG to stdout
hale topology graph .hale.topology --format mermaid
hale topology graph .hale.topology --view claim --claim apart -o claim.svg
```

Views: `system` (loci, functions with phase/effect chips, topics,
publish/subscribe/call edges), `code` (functions and calls only),
`bus` (endpoints only), `claim` (system view with the named claim's
groups highlighted and its verdict on a card), `residue` (unresolved
holes rendered as first-class nodes — never silently omitted; other
views note them on a card). A `--config file.json` can retitle,
`focus`, `hide`, or `highlight` — presentation choices only, never
new semantics.

The renderer admits before it draws, in the fleet loader's order:
the whole-body `artifact_digest` must verify (a hand-edited
artifact is refused, not rendered under a stale identity), the
model `semantics` must match the build, and the schema minor must
be one the adapter actually covers (1.4+). It does *not* require a
clean verdict — violations are worth drawing.

The renderer is an **artifact client**: it reads exactly the JSON a
third party reads, never your source, and its output is
deterministic by construction — fixed text metrics (no font
measurement), stable IDs derived from artifact names, no
timestamps. Rendering the same artifact twice is byte-identical,
and *moving source lines doesn't move a pixel* (provenance spans
change; the model shape doesn't) — which is what makes generated
diagrams safe to commit and regression-test.

### One verdict vocabulary

Bundle claims and fn-grained certificates (`@effects`, `@budget`,
`@phase_effects`) are the same kind of statement at different
granularity, so they report in the same words. Four states, told
apart by what you should do about each:

| verdict | meaning | repair |
|---|---|---|
| `holds` | proved | nothing |
| `violated` | disproved — a counterexample exists | fix the program; the witness says where |
| `uncertified` | well-formed but not provable here — the graph has an unknown (an indirect call, an untypeable receiver, a computed subject) | resolve the unknown edge, or accept that this law can't be checked here |
| `invalid` | the statement itself is malformed — an unknown group member, an undeclared effect class | fix the claim |

`violated` and `uncertified` used to be one value, because **unknown
⇒ violation**: an indirect call fails closed rather than certifying
an absence nothing established. That rule is unchanged and both
still fail the build. What changed is that the artifact records
which happened, because the repairs are different — and because
composing models across binaries needs a propagated unknown to make
a claim `uncertified` rather than report it as disproved when
nothing disproved it.

The document's own `verdict` is `clean` only if every row — claims
and lowered alike — reports `holds`. `uncertified` does not pass: a
law that could not be checked has not been satisfied. It says
nothing about whether the program typechecks, because it doesn't
have to — an artifact is only emitted for a program that does.

### Identity vs. integrity

Two different hashes, and conflating them is a trap:

- **`shape_hash`** is an *identity*. It covers the model half only,
  so claim renames, moved comments, and the `topics`/`provenance`
  sections don't churn it. That's what makes
  `--check-topology-shape` usable in CI.
- **`artifact_digest`** is an *integrity* check. It covers the
  entire body — model, topics, provenance and claim results alike —
  and is the final key, so everything before it is exactly what was
  hashed.

You need the second as soon as anything trusts an artifact it didn't
produce. A gate that greps `shape_hash` out of a committed baseline
can be defeated by editing that one line; and a cross-binary
consumer joining endpoints on the `topics` rows would otherwise be
joining on data outside the hash it verified. Both baseline gates
reject a file whose digest doesn't match its contents, as corrupt
rather than as a mismatch.

An artifact with no `artifact_digest` (anything before schema 1.3)
reports as *unverifiable* rather than as valid — a consumer may
choose to accept it, but must never read "nothing to check" as
"checked and intact".

The artifact shape (schema `1.11`):

```text
{
  "schema": "1.11",
  "shape_hash": "<fnv1a-64 over the model half>",
  "sorts":     { "loci": […], "fns": […], "topics": […] },
  "relations": {
    "calls": [ {"from", "to",
                "loop"?: true, "unbounded"?: true,
                "via_interface"?: "<iface>"} ],
    "calls_via_stdlib": [ {"from", "to", "loop"?: true} ],
    "publishes": […], "subscribes": […]
  },
  "groups":    { "<name>": [members as declared] },
  "labels":    { "<fn>": [declared effect classes] },
  "phases":    { "<fn>": {"phase", "kind": "hook"|"method"} },
  "seeds":     { "<alias>": [member decls] },
  "effects":   { "<fn>": [derived effect classes] },
  "supervision": [ {"locus", "child", "err", "ops": […],
                  "retry_bound"?} ],
  "unknowns":  [ {"fn": …, "reasons": ["indirect_call" |
                  "untyped_receiver_call:<callee>" |
                  "uninhabited_interface_call:<iface>.<callee>" |
                  "computed_publish"]} ],
  "provenance": { "calls": [+span], "publishes": [+span],
                  "subscribes": [+span],
                  "decls": {name: span, topics included},
                  "supervision": [+span] },
  "topics":    [ {"name", "subject", "shape", "payload_hash"} ],
  "claims":    [ {"name", "form", "result": <verdict>,
                  "ordinal", "source"?} ],
  "lowered":   [ {"subject", "form", "result": <verdict>} ],
  "law":       { "law_digest", "inputs_digest",
                 "rows": [ {"ordinal", "name", "origin",
                            "family", "verdict",
                            "law": {"kind", …typed operand refs,
                                    each with name/display/
                                    resolved and provenance…},
                            "certs"?: [ {"ordinal", "form",
                                         "result",
                                         "evidence"?} ],
                            "evidence"?: [ {"message",
                                            "file"?, "span"?} ],
                            "file"?, "span"?} ] },
  "capabilities": { "exact_calls": bool, … },
  "adequacy":  { "<family>": "exact" | "degraded" },
  "verdict":   "clean" | "law_failed"
}
```

Everything renders in author spelling (cross-seed symbols
demangled). `shape_hash` covers the **model half** — sorts,
relations (with weights and the through-stdlib contraction),
groups, labels, phases, seeds, derived effects, unknowns — and
excludes claim *results* and *provenance*: one topology under a
different law keeps one shape, and moving code changes every span
but no identity, while any graph, vocabulary, carrier, phase, or
new fail-closed or dead-dispatch site changes it.
Two gates, named for what they compare:

- **`--check-topology`** — an exact artifact snapshot: law, model
  *and* provenance. Strictest, and the right default when you want
  every change to the file reviewed. Note that it therefore fires on
  a claim rename or a comment that shifts spans, even though the
  model is identical.
- **`--check-topology-shape`** — the model identity (`shape_hash`)
  only. Renames and source motion pass; a changed graph,
  vocabulary, carrier, phase, or new fail-closed site fails. Use
  this when baseline churn from provenance is costing more than it
  catches.

Either way the gate separates two review questions: *does the
program still satisfy the law?* and *did the graph change in a way
reviewers should see?*

The pieces worth knowing:

- **Weights.** A call row marked `"loop": true` sits inside a
  loop; `"unbounded": true` inside a loop with no compile-time
  trip bound. `bound` replay reads these. A `"via_interface"` row
  is one fanned-out dispatch alternative — alternatives sharing
  (from, interface, method) fold with **max**, one dispatch
  invokes one target.
- **`calls_via_stdlib`.** The evaluator walks stdlib bodies; the
  artifact serializes user rows. Every user→user path whose
  interior is stdlib collapses to one contracted edge (loop flag
  conservative: true if *any* such path crosses a loop), so
  reachability over the artifact matches reachability as
  evaluated.
- **`phases` / `seeds` / `effects`.** What `during`, `cover`, and
  effect-class endpoints evaluate against, exported — the rows
  that used to be compiler-certified-only.
- **`provenance`.** Bundle-global byte-offset spans (`[start,
  end]`) for every user edge and decl — the "where to edit" data,
  unhashed by design.
- **`lowered`.** Every fn-grained certificate — each `@effects`
  assert, each `@phase_effects` phase contract, each `@budget` —
  as the claim form it is pointwise sugar for, with the verdict of
  the same evaluation that gates the build: `forbid
  reaches({F}, effects(money))`, `bound alloc <= N on paths from
  {F}`, `only effects {…} on {L} during birth`. One schema of
  record: the artifact carries all law, bundle-quantified and
  fn-grained, in one place. Unhashed like the claim results.
- **`topics`.** The per-topic OBSERVATION identity: the wire
  subject, the canonical payload shape, and `payload_hash` —
  exactly the `(name, shape_hash)` pair the runtime manifest fuses
  on (iris PROTOCOL §4), so a recording/WAL segment names the
  checked topology it ran under. `subject` is deliberately RAW
  (it is the byte-exact join key; a subject-less imported topic
  registers under its mangled local name — declare `subject:` on
  shared topics to fuse across binaries). Unhashed by ruling:
  payload field shape does not affect claim evaluation, so it is
  not part of the model identity — the artifact document is the
  reference between the two identities, not a fusion of them.

v2 scope: every claim verb replays independently over the exported
relations. Still compiler-certified: `bound` over **built-in**
classes (site counting through the stdlib interior, deliberately
not serialized) and any walk past the step ceiling.

## What claims are not

Not tests — a test executes one case; a claim quantifies over
every represented path, and the two compose (test-first for
behavior, claim-first for the architecture the behavior lives in).
Not design-by-contract — a pre/postcondition surrounds one
operation; a claim's witness may cross files, seeds, and message
boundaries. Not a runtime policy engine — claims lower to no code,
inspect no traffic, and authorize no requests; what is not
knowable statically (a computed subject, an un-elaborated
deployment) is exposed as a boundary, never silently approximated
in the unsafe direction.

And the fixed division underneath all of it:

> **The program owns the law. The compiler owns the proof.**

Reference semantics:
[`spec/verification.md § Claims`](https://github.com/hale-lang/hale/blob/main/spec/verification.md).
The workflow treatment:
[Claim-Driven Development in Hale](https://hale-lang.org/articles/claim-driven-development-in-hale/).
