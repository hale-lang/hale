# The twelve steps, with the Journal

[Working with it](./working.md) shows this session as a user sees
it. This is the same session with the Journal beside every step, on
the chat server that ships in the hale repository as the acceptance
application
(`dna/acceptance/chat-server`). The organism's editor runs on
scripted models here — the same run with a hosted model behind
`OPENAI_API_KEY` is the same commands and the same Journal, with
`adapter: hosted` in the evidence. The twelve steps are the
acceptance scenario of the DNA design; `crates/hale-cli/tests/dna_twelve_steps.rs`
runs them in CI in about ten seconds.

## 1. Attach, and lose nothing

```sh
hale check . && hale test .          # before
hale dna init .
hale check --matrix . && hale test . # after: 1 pair checked, 1 passed
git add -A && git commit -m "attach the DNA"
```

[How attaching works](./attach.md) shows what `init` wrote. The
application's own tests are what every candidate will later be
verified against, so they matter twice.

## 2. Start the organism

```text
$ hale dna run . --observe 5
hale dna run: organism chat (pid 1694293) from … under LOTUS_OBS=1
hale dna run: membrane bound at …/.hale/dna
hale dna run: iris at http://127.0.0.1:8787/  (l law · 4 review · 5 organism · m membrane)
```

## 3 and 4. Ask; a Task is born

```text
$ hale dna ask document the chat server in main.hl
task t1 born for intent i1a08c0786c5 [pending]
```

The membrane accepted the intent (`intent.offered`), the Metabolism
birthed a durable Task (`task.born`), and its Workflow made a Step
whose Work is routed back to the assembly as source-editing Work.
That Work takes seconds, so the Task's live pass settles `pending`
and the assembly settles it in the Journal when the Work is done:

```text
   17  intent.offered         i1a08c0786c5   document the chat server in main.hl
   18  task.born              t1             i1a08c0786c5: document the chat server in main.hl
   19  task.pending           t1             t1/wf1/s0=pending {t1/wf1/s0/w0=pending[routed:in-flight ] }
```

## 5 and 6. The Attempt, under its grant, in its own worktree

```text
   20  mutation.proposed      m1             task t1 application: document the chat server in main.hl () at 4319af28…
   23  mutation.worktree      m1             opened .hale/dna/worktrees/m1 at 4319af28…
   24  mutation.located       m1             main.hl under read edit fmt check @.hale/dna/worktrees/m1
   27  mutation.candidate     m1             dbbb49f550f3f021f390710803e48f1b3d1593ff
   39  model.called           m1/a0          {"adapter": "fake", "backend": "quick", …, "tool_grant": "read edit fmt check @.hale/dna/worktrees/m1", …}
   40  model.called           m1/a0          {"adapter": "fake", "backend": "deep", …}
```

The gateway opened a worktree at the genome's head. The editor's
grant was pointed at it — `read edit fmt check @<worktree>` is the
whole of what the Attempt can do, and the constitution makes
anything more a build failure. It located the file the objective
names, asked the quick tier for the rewrite and the deep tier for
the fitness signals the change should move, formatted, checked, and
the gateway committed the candidate. Two model calls under one
attempt id; each carries the grant as evidence.

## 7. Evidence, with receipts

```text
   28  evidence.base          dbbb49f5…   {"mutation_id": "m1", "step": "base", "ok": true, "code": 0, "output_digest": "e607a8e2…"}
   29  evidence.fmt           dbbb49f5…   {…"step": "fmt", "ok": true, "code": 0, …}
   30  evidence.check         dbbb49f5…   {…"step": "check", …}
   31  evidence.verify        dbbb49f5…   {…"step": "verify", …}
   32  evidence.test          dbbb49f5…   {…"step": "test", …}
   33  evidence.rollback      dbbb49f5…   {…"step": "rollback", …}
   34  evidence.diff          dbbb49f5…   {…"step": "diff", …}
   35  evidence.magnitude     dbbb49f5…   {"loci": 0, "contract_change": false, "effects_widened": false, "law_touched": false, …, "novelty": 3}
```

Every step's output is on disk under its own sha256 in
`.hale/dna/evidence/`, and the event on the candidate carries the
exit code and the digest. The base artifact is cut from the genome's
seed at the same moment as the candidate's, so the semantic diff is
against what the candidate actually changed. The rollback step is a
rehearsal: the worktree is stepped back to the base and forward
again, so "reversible" is a fact, not a hope.

## 8. The boundary decides; the Review blocks

```text
   36  mutation.escalate      m1             application candidate dbbb49f550f3…
   37  review.requested       review:m1      {"question": "apply m1 (application): document the chat server in main.hl?", "subject_digest": "dbbb49f5…", "required_authority": "maintainer", …}
   38  task.done              t1             t1/wf1/s0/w0 by editor: m1: review
```

The grant says `refactor docs`; an `application` change is outside
it, so the boundary's disposition is `escalate`. Every disposition
still blocks on a human in this phase — the verdict is the
assurance the grant cannot supply — and the request carries
everything a reviewer decides on.

## 9. The same Review, in the terminal and in iris

```text
$ hale dna review
2 pending review(s) of 2
  m1 needs maintainer — apply m1 (application): document the chat server in main.hl?
      application · candidate dbbb49f550f3 · evidence fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 · disposition escalate
  purpose needs maintainer — ratify the declared purpose?
```

`hale dna review m1` renders the source diff, the semantic diff,
the evidence table and the magnitude — the next chapter,
[The Review in detail](./review.md), shows it in full. Iris's organism panel
shows the same pending Review from the same projection, and its
membrane form sends the same typed verdict.

## 10. Approve: exactly this candidate

```text
$ hale dna review m1 approve --as riley --comment "the rooms stay the only way"
review m1 settled: approve by riley
```

```text
   41  review.settled         m1             approve by riley
   42  effect.requested       apply:dbbb49f5…   m1
   43  effect.result          apply:dbbb49f5…   ok
   44  mutation.applied       m1             dbbb49f550f3f021f390710803e48f1b3d1593ff
   45  expression.restart_requested m1       apply dbbb49f5… fitness guests_greeted +
```

The assembly checked the worktree's head against the pinned
commit, took the Mutation's lease, and applied through the gateway
under the candidate's own idempotency key. `git log` now reads:

```text
dbbb49f application: document the chat server in main.hl
4319af2 attach the DNA
d084733 the chat server
```

## 11. Express it, watch it, measure the pressure

```text
hale dna run: m1 requests a restart (apply dbbb49f5… fitness guests_greeted +)
hale dna run: organism restarted (pid 1694392) as 8517c3db7499d3b3 build b1ac50c8c3ef
hale dna run: m1 observed healthy for 5s as 8517c3db7499d3b3
```

```text
   46  expression.restarted   m1             8517c3db7499d3b3 build b1ac50c8c3ef
   47  expression.observed    m1             healthy 8517c3db7499d3b3 up for 5s
   48  pressure.remeasured    m1             task t1 fitness guests_greeted +: healthy up for 5s
```

The host rebuilt, restarted the organism with the Mutation's id in
its environment — the new expression journaled `expression.restarted`
itself, at birth — relaunched iris with the diff from the previous
artifact, and watched the window. Then it reported on the membrane,
and the organism re-measured the originating pressure against the
fitness signals the proposal declared.

## 12. Retained

```text
   49  mutation.retained      m1             observed healthy as 8517c3db7499d3b3
   50  mutation.worktree      m1             removed
```

```text
$ hale dna status
…
mutations:  2 (none applies before a human's verdict on the exact candidate)
  m1 [retained] application: document the chat server in main.hl · task t1 · candidate dbbb49f550f3
  m2 [rejected] application: document the chat server in main.hl · task t2 · candidate 5e1ba1ea0276
```

`m2` is a second intent, rejected: `git log` did not move, the
running expression did not change, and the Mutation with its
candidate, its Attempt and its evidence stays in the Journal.
`hale dna history m1` is the whole lineage above, in one listing.
