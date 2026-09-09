# The kill test: is a semantic diff visibly better than a git diff for deciding?

This is the falsification test Track D (GH #529) puts before its
apply path, following brained's lesson: a one-day test that can stop
the work, before the work. **If the answer is no, Track D is reordered
or stopped.** The person answering should not care about Hale.

## The setup

`before/` and `after/` are the same small application — a support
desk — before and after a proposed change. A candidate (an agent, a
colleague) describes the change as:

> Tickets can now arrive by email, support engineers get an SLA and can
> reply to customers, and triage is renamed dispatch.

The reviewer's job is the one Track D asks of a human: **approve,
revise, or reject the candidate, and say why.** Nothing else.

Run `dna/kill-test/run.sh`. It prints two views of the same change:

- **View A** — the git diff. What every reviewer has today.
- **View B** — the semantic diff (`hale model diff`): what changed in
  the *model* of the program — declarations added, removed, renamed,
  moved; each locus's contract (params, what it publishes and
  subscribes, supervision, ownership, placement); effects gained or
  dropped per function; laws added, removed, or whose verdict changed.

`run.sh --iris` also opens the same View B as perspective [4] in
iris, with the law view on `l`.

## The protocol

1. Give the reviewer View A only. Start a clock. Ask for a verdict
   and the reasons. Note the time and the reasons verbatim.
2. Give the reviewer View B (and iris, if they want it). Same
   question. Note the time and the reasons.
3. Then reveal what the change actually does to the model (below)
   and ask which view would have let them see it.

Swap the order for a second reviewer, so the views are not always
seen second.

## What the change does, which the reviewer has to catch

Five things, of different kinds, chosen because a git diff shows
each as "some lines changed" and a model diff shows each as what it
is:

| what | View B row | why it matters to a reviewer |
|---|---|---|
| a second publisher of `Tickets` (`EmailIntake`) | `+ locus EmailIntake`, `! claim one_intake: result holds -> violated` | the law `one_intake` is now **violated**; the candidate broke an invariant the team wrote down |
| `Support` gains the `outbound_email` effect (through `Mailer`) | `! fn Support::on_queue gains … publish`, `+ locus Mailer`, `+ topic Outbound` | a customer-facing side effect appeared where there was none |
| the Product → Support contract changes | `! locus Support params: +sla_hours: Int` · `! topic QueueStats shape: -open:i; +open:i;oldest_hours:i` | a parent-facing contract moved |
| `Triage` becomes `Dispatch` | `~ locus Triage -> Dispatch (renamed; shape unchanged)` | half the git diff is this rename; it changes nothing |
| a new law `mail_from_support_only` | `+ claim mail_from_support_only … [holds]` | the candidate added law, which needs a different reviewer than code |

## Scoring

Record, per reviewer and per view:

- **time to verdict**;
- **the verdict** (approve / revise / reject) and **the reasons**;
- **coverage**: which of the five rows above the reasons mention;
- **false confidence**: anything asserted that is wrong (e.g. "the
  rename changes behavior", "no new side effects");
- **what was missing**: a question the reviewer asked that neither
  view answered.

Known limits of View B going in (found by a dry run with model
reviewers, 2026-09-09): an effect class reached through a bus edge
(`Support` → `Outbound` → `Mailer`'s `outbound_email`) shows only as
`gains publish` on the sender, never by the class's name; wire
*values* (`owner: "triage"` becoming `"dispatch"`) are outside the
model; and a field that is added but never read looks the same as
one that is used. Whether those matter to the reviewer's decision is
part of what the test measures.

The test passes if View B is *visibly* better on coverage and false
confidence for a reviewer who does not know Hale — not marginally,
visibly. Time is secondary. If View B only helps someone who already
reads Hale, or the reviewer's questions are ones neither view
answers, write that down: it is the finding, and it reorders the
track.
