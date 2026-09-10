# What it will and won't do

Plainly, so you can decide whether it fits.

## It will

- Take a request in a sentence and turn it into a proposed change to
  one source file, made in a sandbox copy of your repository.
- Format, check, verify and test the candidate, diff it against
  your program's structure, rehearse rolling it back, and keep every
  result with a receipt.
- Show you the source diff, the structural diff and the evidence
  together, pinned to one commit, in the terminal or in iris.
- Apply exactly the commit you approved, rebuild, restart the
  program, watch it, and keep the change or roll it back.
- Record all of it, in order, in a history you can walk and that
  survives restarts.

## It won't

- **Apply anything without a verdict.** In this phase every change
  blocks on a human. Post-review autonomy exists for refactors you
  opt into, and even then a rejection rolls the change back.
- **Migrate live state.** A restart is a restart. State your
  program keeps only in memory is gone across a change, exactly as
  it would be across any deploy.
- **See behaviour in the structural view.** The semantic diff is
  good at structure and rules and blind to what a handler computes;
  a `+1` becoming `+100` is invisible to it. That is why the source
  diff is always beside it and never optional. Read the source.
- **Touch git, the network, or a deployment from the editor.** The
  editing step holds four tools — read, edit, format, check — inside
  its sandbox. The compiler forbids more, with a witness path if
  someone tries.
- **Grow itself.** Persistent pressure on one part of the program
  produces a *proposal* for a new component in the history and a
  note to you. Nothing is added until you ask for it and review it.
- **Measure your fitness signals yet.** The proposal names the
  signals it expects to move; the observation window, for now,
  checks that the program stays up.
- **Run without git, or without a running program.** Changes are
  commits and rollbacks are resets; the organism answers you through
  the live process.

## What the evidence supports

This design was gated on a falsification test before the apply path
was built: reviewers deciding from the structural diff alone, the
source diff alone, and both, on planted changes. Source and combined
decided every case; the structural view alone completed the
rule-backed cases and correctly held on the rest. The combined view
is what you get, and the claim made for the structural view is only
what it earned: it decides law and shape, and it tells you when it
cannot decide. The write-up is `dna/kill-test/WALKTHROUGH.md` in the
hale repository.
