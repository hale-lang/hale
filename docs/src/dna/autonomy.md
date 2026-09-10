# Autonomy: the vector and the rules

Autonomy is a **boundary grant**, given by a parent to one child,
and never widened by the child. In the generated Genome the parent
is you and the child is the application:

```hale,fragment
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor docs", max_magnitude: 4, review: "pre" }
},
review_policy: dna::HumanBeforeApply { },
```

A `Grant` says which change classes the child may apply
(`classes`), a ceiling on the sum of the magnitude vector
(`max_magnitude`), and when review happens (`pre`, `post`,
`sampled`, `audit`). `may_expand_self` exists on the type so a test
can prove it stays false: a request to widen the grant that does
not come from the parent — whatever parent it names — is refused
and audited.

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
irreversible external blast. Those go to Review whatever the
evidence says.

## The disposition

`dispose(grant, class, magnitude, evidence)` is a pure function
(`dna/core/types.hl`), and the boundary journals every assessment
in its audit:

1. a hard boundary → **review**;
2. the class is not in the grant → **escalate** (the current parent
   cannot decide);
3. the vector's sum exceeds `max_magnitude` → **escalate**;
4. the evidence score is below `sum + 2 × novelty` → **stage**
   (verified, not applied);
5. `review: "pre"` → **review**; otherwise → **release**.

Evidence is counted, not rated: `check_clean` 1, `verify_clean` 1,
`tests_pass` 2, `replay_ok` 2, `rollback_rehearsed` 2, and 2 per
independent review. A first change in a fresh lineage needs 3 + 6 =
9 points and cannot have them, which is deliberate: nothing is
released on its first day.

In this phase **every disposition but a release still blocks on a
human** — the verdict is the assurance the grant cannot supply —
and the disposition is recorded (`mutation.review`,
`mutation.stage`, `mutation.escalate`) so the reviewer sees why
they were asked. A candidate that does not even check is
`mutation.deny` and never a candidate.

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
sufficient evidence — check, verify, the rollback rehearsal, and a
lineage that has accepted something before — is a **release**: it
is applied first (`mutation.release`, the boundary's audit says so),
the host restarts and observes, and the Review follows with the
question `post-review: applied m4 (refactor): …?`. A `reviewer`
suffices for a refactor; a maintainer is still needed when law or
effects moved. A rejection after the fact rolls the candidate back
and asks the old expression back.

That is the whole shape of autonomy here: nothing about it is a
runtime flag. The policy is a locus the Genome constructs, the
grant is data the parent owns, and `hale check` sees both.

## What a child cannot do

- **Expand its own grant.** `boundary.expand(requested_by, parent,
  …)` is refused when the requester is the child, or is not the
  parent it names; the refusal is an audit row.
- **Widen its effects quietly.** An added handler that reaches a
  declared effect class is an effect row in the semantic diff, a
  hard boundary in the vector, and a Review whatever the grant
  says.
- **Apply.** The performers group — including the source editor —
  is forbidden by law from reaching `genome_apply`; only the
  assembly's gate does, and only after a settled Review.

## Pressure, and appendages

Pressure is a typed fact (`PressureRaised { source, what, count }`).
The assembly journals every raise (`pressure.raised`), and when one
source has raised it `appendage_threshold` times (default 3) it
journals `appendage.proposed` and notifies the membrane: *an organ
for `what` is proposed, not created*. Nothing is grown. Growing is
a Mutation like any other — asked for as intent, edited under a
grant, verified, reviewed. The organism never adds a limb to itself
because a metric said so.

After a Mutation is retained or rolled back, the pressure that
motivated it is re-measured against the fitness signals the
proposal declared (`pressure.remeasured`). In this phase the
measurement is what the observation window saw; a project's own
fitness signals are the next thing to wire in.

## Workflow definitions are versioned

A Task binds the Workflow definition revision it was born with
(`Metabolism.workflow_revision`). Evolving the definition
(`evolve_workflow`) changes future adoption only: an active Task
finishes under its own revision — `Settled.revision` says which —
and the next Task adopts the new one. Adoption and migration stay
two different, auditable things.
