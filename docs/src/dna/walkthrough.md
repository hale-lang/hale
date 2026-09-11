# The twelve steps, with the record

[Working with it](./working.md) shows a session as a user sees it.
This is the same session with the record beside every step, on the
demo application `hale dna new` makes, with the editor on scripted
models — the same run with a hosted model behind `OPENAI_API_KEY` is
the same commands and the same record, with `adapter: hosted` in the
evidence. The twelve steps are the acceptance scenario of the DNA
design; `crates/hale-cli/tests/dna_twelve_steps.rs` runs them on the
chat server in CI.

## 1. Generate, and lose nothing

```sh
hale check . && hale test .          # before
hale dna init .
hale check --matrix . && hale test . # after: 2 pairs checked, 1 passed
git add -A && git commit -m "the organization"
```

[What init makes](./attach.md) shows what was written. The
application's own tests are what every candidate is verified
against, so they matter twice.

## 2. Start the organization

```text
$ hale dna dev . --observe 5
hale dna dev: organization (pid 1874960) from … under LOTUS_OBS=1
hale dna dev: membrane bound at …/.hale/dna
hale dna dev: expression chat (pid 1874991) under LOTUS_OBS=1
```

## 3 and 4. Ask; a Task is born

```text
$ hale dna ask document the chat server in main.hl
task t1 born for intent i1a08c0786c5 [pending]
```

The Board admitted the intent (`intent.offered`), the Metabolism
birthed a durable Task (`task.born`), and its Workflow made a Step
whose Work is routed back to the substrate as source-editing Work.
That Work takes seconds, so the Task's live pass settles `pending`
and the substrate settles it in the record when the Work is done.
In the session below the change was proposed directly, so the
lineage starts at step 5.

## 5 and 6. The Attempt, under its grant, in its own worktree

```text
    7  mutation.proposed      m1             task t1 docs: document the chat server in main.hl (main.hl) at 232dc8f1…
    8  effect.requested       worktree.open:m1   232dc8f1…
    9  effect.result          worktree.open:m1   ok
   10  mutation.worktree      m1             opened .hale/dna/worktrees/m1 at 232dc8f1…
   11  model.called           m1/a0          {"adapter": "fake", "backend": "quick", …, "tool_grant": "read edit fmt check @.hale/dna/worktrees/m1", …}
   12  model.called           m1/a0          {"adapter": "fake", "backend": "deep", …}
   13  effect.requested       commit:m1:a0   docs: document the chat server in main.hl
   14  effect.result          commit:m1:a0   ok
   15  mutation.candidate     m1             bf94e503c1c002f277248b14b6afd1910bb8ce6f
```

The gateway opened a worktree at the genome's head. The editor's
grant was pointed at it — `read edit fmt check @<worktree>` is the
whole of what the Attempt can do, and the law makes anything more a
build failure. It planned the files the objective names, asked the
quick tier for the rewrite and the deep tier for the fitness signals
the change should move, formatted, checked, and the gateway
committed the candidate. Two model calls under one attempt id; each
carries the grant as evidence.

## 7. Evidence, with receipts

```text
   16  evidence.base          bf94e503…      {"mutation_id": "m1", "step": "base", "ok": true, "code": 0, "output_digest": "b676facc…"}
   17  evidence.fmt           bf94e503…      {…"step": "fmt", "ok": true, "code": 0, …}
   18  evidence.check         bf94e503…      {…"step": "check", …}
   19  evidence.verify        bf94e503…      {…"step": "verify", …}
   20  evidence.test          bf94e503…      {…"step": "test", …}
   21  evidence.fleet         bf94e503…      {…"step": "fleet", …}
   22  evidence.rollback      bf94e503…      {…"step": "rollback", …}
   23  evidence.diff          bf94e503…      {…"step": "diff", …}
   24  evidence.magnitude     bf94e503…      {"loci": 0, "contract_change": false, "effects_widened": false, "law_touched": false, …, "novelty": 3}
```

Every step's output is a blob under `refs/dna/receipts/` by its
sha256, and the event on the candidate carries the exit code and the
digest. The base artifact is cut from the genome's seed at the same
moment as the candidate's, so the semantic diff is against what the
candidate actually changed. The fleet step composes every plan the
workspace declares from the candidate's own artifacts. The rollback
step is a rehearsal: the worktree is stepped back to the base and
forward again, so "reversible" is a fact, not a hope.

## 8. The boundary decides; the Review blocks

```text
   25  mutation.stage         m1             docs candidate bf94e503…
   26  review.requested       review:m1      {"question": "apply m1 (docs): document the chat server in main.hl?", "subject_digest": "bf94e503…", "required_authority": "leader", …}
```

The grant says `refactor docs`; a `docs` change is inside it, and a
first change in a fresh lineage does not have the evidence to
release, so the disposition is `stage` and the Review is the
Leader's. Every disposition but a release still blocks on a verdict,
and the request carries everything a reviewer decides on.

## 9. The same Review, everywhere

```text
$ hale dna review
2 pending review(s) of 2
  m1 needs leader — apply m1 (docs): document the chat server in main.hl?
      docs · candidate bf94e503c1c0 · evidence fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 fleet=0 · disposition stage
  purpose needs board — ratify the declared purpose?
```

`hale dna review m1` renders the source diff, the semantic diff, the
evidence table and the magnitude — [The Review in
detail](./review.md) shows it in full. The page, iris's organism
panel, a teammate's clone after `hale dna sync`, and a pull request
when GitHub is configured all show the same Review from the same
record.

## 10. Approve: exactly this candidate

```text
$ hale dna review purpose approve --as riley --comment "ratified"
review purpose settled: approve by riley
$ hale dna review m1 approve --as riley --comment "fine"
review m1 settled: approve by riley
```

```text
   27  review.settled         purpose        approve by riley
   28  review.settled         m1             approve by riley
   29  effect.requested       apply:bf94e503…   m1
   30  effect.result          apply:bf94e503…   ok
   31  mutation.applied       m1             bf94e503c1c002f277248b14b6afd1910bb8ce6f
   32  expression.restart_requested m1       apply bf94e503… seed . fitness docs_coverage +
```

The substrate checked the worktree's head against the pinned commit,
took the Mutation's lease, and applied through the gateway under the
candidate's own idempotency key. `git log` now reads:

```text
bf94e50 docs: document the chat server in main.hl
232dc8f the organization
```

## 11. Express it, watch it, measure the pressure

```text
hale dna dev: m1 requests a restart (apply bf94e503… seed . fitness docs_coverage +)
hale dna dev: expression restarted (pid 1875137) as 3c9b9327e480d349 build 7a489ad3e72c
hale dna dev: m1 observed healthy for 5s as 3c9b9327e480d349
```

```text
   33  expression.restarted   m1             3c9b9327e480d349 build 7a489ad3e72c
   34  expression.observed    m1             healthy 3c9b9327e480d349 up for 5s
   35  pressure.remeasured    m1             task t1 fitness docs_coverage +: healthy up for 5s
```

The host rebuilt, restarted the application with the Mutation's id
in its environment, and watched the window. Then it reported on the
membrane, and the organization re-measured the originating pressure
against the fitness signals the proposal declared. On a fleet the
same step is a `fleet.deploy` row, `instance.up` from every touched
node, and the window over all of them.

## 12. Retained

```text
   36  mutation.retained      m1             observed healthy as 3c9b9327e480d349
```

```text
$ hale dna status
…
mutations:  1 (none applies before a human's verdict on the exact candidate)
  m1 [retained] docs: document the chat server in main.hl (main.hl) · task t1 · candidate bf94e503c1c0
```

A rejected change leaves `git log` unmoved and the running
expression unchanged, and the Mutation with its candidate, its
Attempt and its evidence stays in the record. `hale dna history m1`
is the whole lineage above, in one listing, from any clone.
