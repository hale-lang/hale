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

## Results so far (2026-09-09)

Two rounds with model reviewers, neither the outsider this test
wants; recorded here so the human round starts from what is known.

**Round 1 (this walkthrough, two fresh reviewers, opposite order).**
Both decided from View B in an estimated 2–4 minutes (the law
flipping to violated is one line) and wrote their revision comments
from View A in 10–15. View B missed the payload shape change and
never named `outbound_email`; View A caught everything but could only
infer the law outcome. Fixed after the round: topic shape rows, no
rename echo in ownership/placement, a legend, labeled offsets.

**Round 2 (independent, stricter: six cases, two safe controls, a
Latin square, frozen packets and ground truth, unsafe approvals /
detections / calibrated holds / completed decisions scored
separately).** Source and combined views tied on every measure —
0/4 unsafe approvals, 4/4 detections, 2/2 safe approvals, 6/6
completed decisions. View B alone: 0/4 unsafe approvals, 2/4
detections (both law-backed), 4 calibrated holds, 2/6 completed. Its
two blind spots were pairs whose semantic text was byte-identical:
a rename versus the same rename plus `+1 → +100` inside a handler
(behaviour, which a structural diff will never carry), and a frozen
payload gaining a field versus an inert param (fixed since: the
shape row).

**What that says.** For small, fully visible changes the semantic
diff does not beat a complete source diff at *deciding*; it decides
faster where a law's verdict moved, it summarizes structure, and it
is honest about what it cannot see — reviewers held rather than
approved. It is not a standalone approval interface, and Track D's
review (D4) should render the source diff, the semantic diff and the
evidence table together, as the issue already says. The human round
should test that combined view against the source diff alone, on a
change large enough that the source is not fully readable in one
sitting — the regime the semantic diff exists for.

