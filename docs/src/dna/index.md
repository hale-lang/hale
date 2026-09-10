# DNA: a governed codebase

Your codebase can change itself. You hold the door, and the door is
in git.

**DNA** gives a Hale codebase an *organization* that oversees it: a
small program, generated next to your code, with a human Board at
the top, a model-backed Leader under it holding a grant the Board
gave, and a substrate that makes changes in sandboxes, proves them
with the toolchain, shows them to whoever has the authority, applies
exactly what was approved, expresses it — on your machine, or on a
fleet of nodes — and watches that it works. Every step is a commit
on a branch of your repository, so a teammate with a clone has the
whole story, and a pull request can be the review.

```sh
hale dna new demo                      # a greenfield application with its organization
hale dna init .                        # or: generate the organization for an existing one
hale dna dev                           # the organization and the application on this machine
hale dna ask "document the chat server in main.hl"
hale dna review m1                     # the change, its structural diff, the evidence
hale dna review m1 approve --as riley  # your call — or the Leader's, inside its grant
```

## The loop

1. **Someone asks.** In a sentence, from any clone, from GitHub, or
   from the page `hale dna ui` serves. The organization turns it into
   a task.
2. **It proposes.** In a sandbox copy of the repository, with a
   model, under a grant that is read, edit, format and check and
   nothing more, it edits the files, and commits the candidate there.
3. **It proves.** The candidate is formatted, checked, verified,
   tested, composed as the fleet it would deploy, diffed against the
   program's structure, and rehearsed for rollback. Every result is a
   receipt in the record.
4. **Someone decides.** Inside the grant, the Leader — a model with
   the source diff and the structural diff, leaving its reasoning in
   the record. Outside it, or whenever the law, an effect or the
   fleet's shape moves, the Board: you. Approve, revise, reject.
5. **It applies and watches.** Approval is a commit. The change is
   expressed — the process restarted here, or every touched instance
   redeployed by its node — and watched for a window. It stays up:
   kept. One instance falls over: rolled back everywhere, with the
   instance named.

The organization itself grows the same way. Pressure that persists
becomes a proposal for a new position; the proposal is a commit the
Board reviews; approval restarts the organization with the new
position in it. The org chart and the code evolve under one loop,
in one record.

## What you keep

- **The decision.** Nothing is applied without a verdict, and every
  verdict has a name on it. The Leader's verdicts are model calls
  with evidence, inside a grant only the Board can widen.
- **The source.** A change is always shown as a source diff, never a
  summary of one.
- **The story.** `hale dna history m1` is everything that happened to
  one change: the ask, the sandbox, the model calls and what they
  cost, each check with its receipt, the verdict, the apply, the
  deploy, the outcome. It lives in `refs/dna/journal` in your
  repository and syncs like code.
- **The way back.** A change that crashes inside the window, on any
  instance, is rolled back to the commit it started from, everywhere
  it was expressed.

## What it looks like

```text
$ hale dna review
2 pending review(s) of 2
  m1 needs leader — apply m1 (docs): document the chat server in main.hl?
      docs · candidate bf94e503c1c0 · evidence fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 fleet=0 · disposition stage
  purpose needs board — ratify the declared purpose?

$ hale dna review m1 approve --as riley --comment "fine"
review m1 settled: approve by riley

hale dna dev: m1 requests a restart (apply bf94e503c1c0… seed . fitness docs_coverage +)
hale dna dev: expression restarted (pid 1875137) as 3c9b9327e480d349 build 7a489ad3e72c
hale dna dev: m1 observed healthy for 5s as 3c9b9327e480d349
```

## Where to go

- [Getting started](./getting-started.md) — generate it, run it,
  answer the first review.
- [Working with it](./working.md) — the daily loop, from any clone.
- [The organization](./organization.md) — the Board, the Leader,
  the grant, and how the org chart grows.
- [Operating the fleet](./operating.md) — nodes, deploys, rollbacks,
  the window over every instance.
- [Shaping it](./shaping.md) — the purpose, the grant, the models,
  the backends.
- [What it will and won't do](./limits.md) — the plain limits.
- [Troubleshooting](./troubleshooting.md) — the messages you'll meet.

The mechanism — the record as a git branch, the gateways, the
Review's pin, the autonomy rules — is in the *under the hood*
chapters that follow. You don't need them to use it.
