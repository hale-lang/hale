# What it will and won't do

Plainly, so you can decide whether it fits.

## It will

- Take a request in a sentence — from a terminal, another clone,
  GitHub, or the page — and turn it into a proposed change to your
  source, made in a sandbox copy of your repository.
- Format, check, verify and test the candidate, compose the fleet
  it would deploy, diff it against your program's structure,
  rehearse rolling it back, and keep every result as a receipt in
  the record.
- Show the source diff, the structural diff and the evidence
  together, pinned to one commit, in the terminal, on the page, in
  iris, or as a pull request.
- Decide inside a grant the Board gave, with a model whose reasoning
  is on the record, and ask the Board for everything else.
- Apply exactly the commit that was approved, express it — here, on
  every node the plan assigns, or through your pipeline — watch it,
  and keep the change or roll it back everywhere it went.
- Grow its own org chart the same way: a proposal, a candidate, a
  Board review, a restart.
- Record all of it, in order, on a branch of your repository that
  syncs like code.

## It won't

- **Apply anything without a verdict.** The Leader's verdict is a
  model's, inside a grant only the Board writes; the Board's is a
  person's. Post-review autonomy exists for refactors you opt into,
  and even then a rejection rolls the change back.
- **Migrate live state.** A restart is a restart. State a process
  keeps only in memory is gone across a change, exactly as it would
  be across any deploy.
- **See behaviour in the structural view.** The semantic diff is
  good at structure and rules and blind to what a handler computes;
  a `+1` becoming `+100` is invisible to it. That is why the source
  diff is always beside it and never optional. Read the source.
- **Let the editor touch git, the network, or a deployment.** The
  editing position holds four tools — read, edit, format, check —
  inside its sandbox. The compiler forbids more, with a witness path
  if someone tries.
- **Grow without the Board.** Persistent pressure produces a
  proposal, and with initiative a candidate; nothing joins the org
  chart until the Board approves the commit.
- **Diff the fleet as a whole.** A Review's semantic diff is the
  edited seed's. The deploy row names the instances it reaches; a
  fleet-level structural diff is not there yet.
- **Raise pressure from your metrics on its own.** `hale dna
  pressure raise` is the spelling; nothing wires a service's metrics
  to it yet.
- **Run without git.** The record is a branch, changes are commits,
  rollbacks are resets. Nodes and teammates need a remote to share
  it through.

## What the evidence supports

This design was gated on a falsification test before the apply path
was built: reviewers deciding from the structural diff alone, the
source diff alone, and both, on planted changes. Source and combined
decided every case; the structural view alone completed the
rule-backed cases and correctly held on the rest. The combined view
is what you get — and what the Leader gets — and the claim made for
the structural view is only what it earned: it decides law and
shape, and it tells you when it cannot decide. The write-up is
`dna/kill-test/WALKTHROUGH.md` in the hale repository.
