# Claims & the law

The [effects chapter](./effects.md) taught one function to make a
promise. This chapter is about the promises no single function can
make.

"Nothing in the Delta wing reaches the Gamma wing." "Exactly one
locus publishes settlement commands." "Every path from intake to the
ledger passes through authorization." "One task invokes the model at
most once." These are laws of the *assembled system* — spread them
across function annotations and completeness depends on remembering
every entry point, and the requirement itself has no name and no
source line a reviewer can point at.

A **claim** is a named sentence over the whole program graph,
declared on the main locus, checked by `hale check`, and lowered to
zero runtime code. When a claim is false, the compiler returns a
countermodel: the path, the un-granted edge, the uncovered topic,
the competing writers, or the excessive count.

## The vocabulary: groups

Claims quantify over **groups** — declared sets of loci, functions,
and imported declarations:

```hale,fragment
group delta_wing = { delta::*, DeltaStore };
group gamma_wing = { gamma::Research };
group probes     = { } may_be_empty;
```

Groups are checked vocabulary, not text. A misspelt member is an
**error with a did-you-mean**, never an empty set that happens to
satisfy every prohibition. An empty group is a **vacuity error**
unless it says `may_be_empty` — a `forbid` trivially satisfied by an
empty domain proves nothing while reading as law. And a group whose
members have no executable surface (pure-data loci) is refused by
any claim that walks functions, for the same reason.

The only pattern is `alias::*` — trailing-only enumeration of an
imported seed's declarations, resolved through the same machinery
that resolves `alias::Name` everywhere else.

## The law lives in `main`

The main locus already owns the assembled world: `params` says who
exists, `placement` where they run, `bindings` where the process
boundary is. `claims` completes the family with what must remain
true of the whole:

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

This program **fails to check** — the metrics publish crosses the
boundary — and the diagnostic returns the route:

```text
claim `iso_dg` violated: `delta_wing` reaches `gamma_wing` —
witness: `DeltaTriage::on_task` -(publishes "Metrics")-> `GammaResearch::on_metric`
```

Claims are **errors**, never warnings. Weakening one — deleting a
`forbid`, widening a grant, raising a bound — is a visible source
diff, which is exactly the review event the block exists to create.

## The verbs

**`forbid reaches(A, B)`** — no path from A to B, composing calls
with message dispatch by default. `via { calls }` or `via { bus }`
restricts the relation; omitting `via` is the conservative full
composition.

**`only edges A -> B { … }`** — the boundary form: every *direct*
edge from A into B must match a granted line, and each grant is a
reviewable topic name:

```hale,fragment
grant: only edges gamma_wing -> delta_wing {
    publish t::ResearchDigest;
};
```

Call edges are never grantable — a permitted cross-domain dependency
should be a named bus edge, not an invisible method call. Note the
division of labor: `only edges` constrains the direct boundary;
transitive routes through third parties are `forbid reaches`
territory.

**`require` / `cover` / `count`** — the positive family, because a
claims system with only prohibitions rewards the empty program:

```hale,fragment
wired: require subscribes(some delta_wing, topic t::Tasks);
no_orphans: cover topic in seed(t): subscribed_by(some positions);
single_writer: count publishers(topic t::Tasks) == 1;
```

`require` demands a declared bus end exists; `cover` quantifies over
every topic an imported seed declares (naming each uncovered one);
`count` handles cardinality — `== 1` is the single-writer invariant,
and a violation names the competing writers.

**`bound C <= N on paths from G`** — the [budget](./effects.md)
semiring behind a claims surface. Classify the capability once
(`@effects(is: {llm})` on the model call), then bound it per
invocation: two helpers each calling the model once is a count of
two, a carrier inside a loop is unbounded, and an edge the compiler
cannot resolve refuses to certify rather than counting as zero.

**`during <phase>`** — restrict a `forbid`'s sources to one
lifecycle phase or method: `forbid reaches(positions, effects(llm))
during birth` is the quiet-boot claim. A phase that names nothing in
the group is an error, not a vacuous pass.

**`avoiding <group>`** — the interposition form. "A reaches B only
through the gate" is literally "no path exists once the gate is
masked out":

```hale,fragment
gated: forbid reaches(public_api, ledger) avoiding authorization_gate;
```

The gate must be disjoint from the endpoints — masking the target
would hold vacuously, masking a source silently drops roots, and
both are rejected.

## The data plane: indexed families

Control-plane isolation is not data isolation: two wings with no
call or message route can still share a helper that reads the wrong
store. User effects supply the data vocabulary, and **indexed
families** keep it structured:

```hale,fragment
domain wing = { delta, gamma };
effect knowledge(wing);

@effects(is: {knowledge(delta)})
fn read(key: String) -> Idea { ... }
```

Every instantiation (`knowledge(delta)`, `knowledge(gamma)`) behaves
as an ordinary declared class, `knowledge(*)` covers all of them,
and a misspelt index is an undeclared-class error with a hint. Then
the data rule is one line, independent of the control-plane rule:

```hale,fragment
data_iso: forbid reaches(delta_wing, effects(knowledge(gamma)));
```

A serious boundary usually wants both sentences.

## The model in version control

The checked model exports as the **topology artifact**:

```text
hale check app --dump-topology > .hale.topology
hale check app --check-topology .hale.topology
```

The artifact carries the sorts, the call/publish/subscribe
relations, the groups, the effect labels, the *unknowns* (every
place static certification stopped and failed closed), and each
claim's result — in author spelling, under a schema version and a
`shape_hash` over the model half. A committed baseline separates two
review questions: *does the program still satisfy the law?* and
*did the graph change in a way reviewers should see?* — and it makes
three kinds of pull request structurally distinct: implementation
under the same law, topology change under the same law, and a law
change.

## Where the trust lives

Evaluating a claim is the easy half; deriving the graph is the trust
root. The rule the whole surface obeys:

> Uncertainty may add possible edges. It may never delete an edge
> and report success.

An indirect call, a receiver the compiler cannot type, a computed
publish subject — each fails closed with a diagnostic that says what
to fix, rather than certifying by silence. The claim language is
deliberately fixed (no user inference rules, no embedded prover):
reachability, existence, coverage, cardinality, interposition, and
bounded cost, each decidable, each returning a small countermodel.

The workflow this enables — write the law first, let countermodels
drive the implementation — has a full treatment in
[Claim-Driven Development in Hale](https://hale-lang.org/articles/claim-driven-development-in-hale/).
The reference semantics live in the
[verification spec](https://github.com/hale-lang/hale/blob/main/spec/verification.md).

The program owns the law. The compiler owns the proof.
