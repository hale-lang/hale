# DNA: a governed application

**DNA** is the part of a Hale application that decides how the
application itself changes: what may be attempted, who reviews it,
what evidence counts, what is applied, and what is never applied
without a human. It ships inside the `hale` binary as ordinary Hale
source, and a project owns its own copy of the decisions.

```sh
hale dna init .                      # attach it to an application
hale dna run                         # hold the organism, its membrane, and iris
hale dna ask "document the chat server in main.hl"
hale dna review m1                   # what a human decides on
hale dna review m1 approve --as riley
```

That is the whole loop. An intent goes in through the **membrane**,
becomes a durable Task, and a source-editing Attempt proposes a
change in its own worktree. The toolchain verifies the candidate
and leaves receipts. A blocking **Review** shows a human the source
diff, the semantic diff and the evidence, pinned to the exact
candidate. Approval applies exactly that candidate, the host
rebuilds and restarts the expression, an observation window judges
it, and the change is retained or rolled back. Every step is an
event in a hash-chained **Journal** that `hale dna history` walks.

## Three words

- **Genome** — the source, the law, and the assembly that says
  which stores, models and gateways exist. What the program *is*.
- **Expression** — the running process built from the genome, with
  a model hash iris can join to its instances. What the program
  *does*.
- **Experience** — the Journal: what happened, in order, with
  receipts. The authority when the two disagree.

The names are chosen to keep three things apart that most
"self-improving software" blurs: a change to the genome is not a
change to the expression until a rebuild, and neither is true until
the Journal says so.

## What it is not

DNA is not an agent that edits your program while you sleep. The
kill test that gates this design (`dna/kill-test/` in the hale
repository) established two things about model-generated changes:
a semantic diff decides law and structure well, and it cannot see
handler behaviour — a `+1` becoming `+100` is invisible to it. So a
Review always shows the source diff beside the semantic one, and in
this phase every change blocks on a human. The grant a project gives
its organism is a *boundary*, expressed in the law, not a mood.

What DNA does do, mechanically and every time:

- the editing Attempt can read, edit, format and check inside one
  worktree, and the constitution makes anything more a build
  failure with a witness path;
- a candidate is verified with `fmt`, `check`, `verify`, `test`,
  `model diff` and a rollback rehearsal, each output kept under its
  own sha256;
- the verdict names a candidate commit; a candidate that moved after
  the reviewer looked is refused by digest;
- an apply is journaled before it is dispatched, so a crashed and
  retried apply reads its own record and never commits twice;
- a rollback is `git reset --keep` to a base the Journal recorded,
  never a guess.

## Where to go next

- [Attaching it](./attach.md) — what `init` generates and why the
  law lives where it does.
- [Running it](./run.md) — the host, the membrane, status, ask,
  history.
- [A change, end to end](./walkthrough.md) — one real session, with
  its output.
- [Reviewing](./review.md) — the three views, the verdict, the pin.
- [Apply, restart, observe](./apply.md) — what approval does and
  what happens when it goes wrong.
- [Autonomy and policy](./autonomy.md) — grants, the magnitude
  vector, post-review, pressure.
- [Models and credentials](./models.md) — hosted, local, scripted,
  sealed.
- [Reference](./reference.md) — the Journal's vocabulary, the CLI,
  the files.
