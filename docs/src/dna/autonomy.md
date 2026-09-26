# Autonomy: the vector and the rules

Autonomy is a **boundary grant**, given by the Board to one position,
and never widened by the position. In the generated organization the
Board is you and the position is the Leader, over the codebase:

```hale,fragment
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor docs", max_magnitude: 4, review: "pre" }
},
review_policy: dna::OrgPolicy { },
```

A `Grant` says which change classes may be decided inside it
(`classes`), a ceiling on the sum of the magnitude vector
(`max_magnitude`), and when review happens (`pre`, `post`,
`sampled`, `audit`). `may_expand_self` exists on the type so a test
can prove it stays false: a request to widen the grant that does not
come from the parent — whatever parent it names — is refused and
audited.

## The magnitude vector

Every candidate is measured on a **vector**, never reduced to a
score, because a confidence number would let evidence cancel a
boundary it must never cancel:

| dimension | read from |
|---|---|
| `loci` | declarations added, removed, moved, renamed in the semantic diff |
| `contract_change` | a parent-facing contract facet moved (params, publications, a topic's payload shape) |
| `effects_widened` | an effect class newly reached, on a paired fn or an added one |
| `law_touched` | a claim added, removed, or whose result moved |
| `placement_change`, `ownership_crossed` | those facets in the diff |
| `state_migration` | a persisted locus's params changed |
| `external_blast` | a *declared* effect class newly reached (0 none … 3 customer/financial) |
| `irreversible` | state migration or a removal |
| `novelty` | 3 until the lineage has applied anything, then 1 |

Some dimensions are **hard boundaries** the grant can never cover:
touching law, widening effects, crossing ownership, or an
irreversible external blast. Those go to the Board whatever the
evidence says.

## The evidence

Counted, not rated: `check_clean` 1, `verify_clean` 1, `tests_pass`
2, `replay_ok` 2, `rollback_rehearsed` 2, and 2 per independent
review. Two more facts gate before any of that counts:
`fleet_clean` — every plan the workspace declares composes and holds
its claims with the candidate's artifacts (true when it declares
none) — and `topology_changed` — the candidate's diff names a plan
or the manifest. A candidate that does not check is `mutation.deny`;
one that breaks the fleet is `mutation.deny` with the witness in
its receipt; one that changes the fleet's shape is re-classed
`topology` (`mutation.topology`) whatever it was asked as, which
makes it the Board's.

## The disposition

`dispose(grant, class, magnitude, evidence)` is a pure function
(`dna/core/types.hl`), and the boundary journals every assessment in
its audit:

1. a hard boundary → **review**;
2. the class is not in the grant → **escalate** (the current parent
   cannot decide);
3. the vector's sum exceeds `max_magnitude` → **escalate**;
4. the evidence score is below `sum + 2 × novelty` → **stage**
   (verified, not applied);
5. `review: "pre"` → **review**; otherwise → **release**.

A first change in a fresh lineage needs 3 + 6 = 9 points and cannot
have them, which is deliberate: nothing is released on its first
day. Every disposition but a release blocks on a verdict, and the
disposition is recorded (`mutation.review`, `mutation.stage`,
`mutation.escalate`) so the reviewer sees why they were asked.

## Who is asked

`OrgPolicy` turns a disposition into a required authority: `board`
when the vector touched law, widened effects or crossed ownership;
`board` for the Board classes — `organization`, `constitutional`,
`process-policy`, `topology`; `leader` for everything else inside
the grant. A `Review` requiring `leader` is answered by the Leader
position with a model, and by anyone of higher rank who gets there
first.

## Post-review autonomy

Swap the policy and loosen the grant for a class you trust:

```hale,fragment
review_policy: dna::PostReviewRefactors { },
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor", max_magnitude: 12, review: "post" }
},
```

Now a `refactor` inside the grant, with no hard boundary and
sufficient evidence, is a **release**: it is applied first
(`mutation.release`, the boundary's audit says so), expressed and
observed, and the Review follows with the question `post-review:
applied m4 (refactor): …?`. A `reviewer` suffices for a refactor; the
Board is still needed when law or effects moved. A rejection after
the fact rolls the candidate back and asks the old expression back.

That is the whole shape of autonomy here: nothing about it is a
runtime flag. The policy is a locus the organization constructs, the
grant is data the Board owns, and `hale check` sees both.

## Money, parties, routes and time

A grant can delegate more than change classes. The same containment
covers what a position may spend, with whom, through which route, and
until when:

```hale,fragment
grant: dna::Grant {
    child: "books", classes: "docs", max_magnitude: 4, review: "pre",
    amounts: "USD 500.00",          // per operation
    window_amounts: "USD 2000",     // per window
    window: "week",
    counterparties: "suppliers",
    routes: "operating-account",
    expires: 1798761600             // unix seconds; 0 never
}
```

Nothing named, nothing allowed: a grant without `amounts` spends
nothing, which is what every grant written before these fields meant.
A child's resources sit inside its ceiling's the way its classes do —
the currencies both grant at the smaller ceiling, the parties and
routes both name, the earlier expiry — and a child born wider is
refused in the record, naming the field.

A spend goes through the substrate. `reserve` checks each predicate
and the windows, and writes `grant.reserved`; `settle_spend` records
what was actually spent. A window's remainder is read at one revision
of the record and the reservation lands only at that revision, so two
children under one ceiling cannot both spend its last 500. When the
Board contracts or revokes a grant, its epoch moves, and `admits`
refuses a spend admitted before the change (`grant.fenced`).

## What a position cannot do

- **Expand its own grant.** `boundary.expand(requested_by, parent,
  …)` is refused when the requester is the child, or is not the
  parent it names; the refusal is an audit row.
- **Widen its effects quietly.** An added handler that reaches a
  declared effect class is an effect row in the semantic diff, a
  hard boundary in the vector, and the Board's whatever the grant
  says.
- **Apply.** The positions group — including the source editor and
  the Leader — is forbidden by law from reaching `genome_apply`
  except through the substrate, and the substrate applies only after
  a settled Review.
- **Move money around the substrate.** The law forbids any position
  from reaching the `money` effect except through `dna::Dna`, which
  reserves the spend against the grant first; a wiring that hands a
  position the processor itself is a build failure with a witness.

## Pressure, and growth

Pressure is a typed fact (`PressureRaised { source, what, count }`).
The substrate journals every raise (`pressure.raised`), and when one
source has raised it `appendage_threshold` times (default 3) it
journals `appendage.proposed` and notifies the Board: *an organ for
`what` is proposed, not grown*. With `initiative: true`, the proposal
becomes an `organization` mutation in a sandbox on the org's own
seed — `appendage.candidate` — verified like any other and reviewed
by the Board. Approval restarts the organization with the position
in it. Without initiative, growing is asked for as intent, like any
change.

After a Mutation is retained or rolled back, the pressure that
motivated it is re-measured against the fitness signals the proposal
declared (`pressure.remeasured`). The measurement is what the
observation window saw; a project's own fitness signals are the next
thing to wire in.

## Workflow definitions are versioned

An execution binds the workflow definition and revision it was
admitted under (`workflow.admitted`: definition, revision, the bound
recipe). Defining a new revision in the catalog changes future
adoption only: an active execution finishes under its own recipe, and
the next admission adopts the new one. Adoption and migration stay
two different, auditable things.

## Legs: the hat and the two commands

Work is performed by legs — a person, an agent harness, a worker pool —
that hold nothing between tasks and touch no database. The spine's
whole API to one is three things. **The hat** is a read,
`GET …/dna/context?id=<work>`: one Work's context as structure, never a
prompt — who the position is and its charter, the practices ratified
for the Work's target with their ids, the bindings, the tool grant,
the output contract, the data class, the Work's history as facts, and
the record head and memory's watermark it was rendered at, all under
one digest. Rendered twice at one head it is one digest; a row that
moves the head moves it. The leg renders it for its backend, and
records the hat digest, the prompt digest and its renderer's version
on the attempt, so replay renders from the recorded hat and the tape
hits. **The claim**, `dna.attempt.claim`, hands the leg an admitted
attempt of the kind it performs that fits its capabilities and data
classes, and the lease it works under — memory's claim on the attempt,
with a token and an expiry — as a value; nothing fitting, or an
attempt another leg holds, is a refusal with the reason. **The
outcome**, `dna.attempt.outcome`, hands the result back under that
lease with the calls it made and the receipts to file; the owner
journals the calls on the attempt (tokens per task hold out of
process), settles it as it settles every reply, and refuses a stale
lease or a duplicate with the reason. Positions are the graph's
`position:<name>` ids.
