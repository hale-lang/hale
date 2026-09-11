# Working with it

The daily loop is five commands. This chapter walks one change
through them, with the output you'll see, then shows the same loop
from a teammate's clone and from GitHub.

## Start it

```sh
hale dna dev                    # organization + application here; iris on :8787
hale dna dev --observe 30       # watch a restarted program for 30 seconds (default 15)
hale dna run                    # the organization only; the fleet, or your pipeline, expresses
hale dna ui --port 8790         # the page, from the record, in any clone
```

Stop the host with Ctrl-C. The organization's memory is the record
in git, and a review left pending is still pending next time, in
every clone that syncs.

## Ask

```text
$ hale dna ask document the chat server in main.hl
task t1 born for intent i1a08c0786c5 [pending]
```

Say what you want as you would to a colleague, and name the file if
you know it. `[pending]` means the organization is working: a
sandbox copy of the repository, the model's edit under the editor's
grant, format, check, a commit there, and then the verification. It
takes seconds; the next command tells you when there is something
to look at.

## Review

```text
$ hale dna review
2 pending review(s) of 2
  m1 needs leader — apply m1 (docs): document the chat server in main.hl?
      docs · candidate bf94e503c1c0 · evidence fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 fleet=0 · disposition stage
  purpose needs board — ratify the declared purpose?
render one with `hale dna review <id>`; decide with `hale dna review <id> approve|revise|reject|abstain`
```

`m1` is the change. `evidence` is the exit code of each check (0 is
clean; `fleet` is the plan the workspace declares, composed with the
candidate's artifacts). `needs leader` says who decides: a `docs`
change is inside the grant the Board gave the Leader, so the Leader
decides it — a model, reading the same views you are about to,
leaving its reasoning in the record — and you are looking over its
shoulder. An `application` change, or anything that touches the
law, an effect, or the fleet's shape, `needs board`: you. Open it:

```text
$ hale dna review m1
review m1 [pending]: apply m1 (docs): document the chat server in main.hl?
  needs leader · candidate bf94e503c1c002f277248b14b6afd1910bb8ce6f
  mutation m1 (docs) by editor · disposition under the grant: stage · shape 3c9b9327e480d349
  magnitude: novelty 3

source diff (git 232dc8f18dde .. bf94e503c1c0):
   main.hl | 1 +
   1 file changed, 1 insertion(+)

  diff --git a/main.hl b/main.hl
  --- a/main.hl
  +++ b/main.hl
  @@ -28,3 +28,4 @@ main locus Chat {
   fn main() {
       Chat { };
   }
  +// documented by the organism: Echo answers every Ping

semantic diff (hale model diff, baseline .. candidate):
  classification: source-only  (shape_hash 3c9b9327e480d349 -> 3c9b9327e480d349)
  no semantic differences

evidence (fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 fleet=0):
  step       ok     code  receipt        bytes
  base       yes    0 b676facc8c41   26
  fmt        yes    0 e3b0c44298fc   0
  check      yes    0 e3b0c44298fc   0
  verify     yes    0 e3b0c44298fc   0
  test       yes    0 184a3407ce31   67
  fleet      yes    0 b33b2e832b21   277
  rollback   yes    0 89978376be81   96
  diff       yes    0 8b0b8da30d3a   3002
  receipts: refs/dna/receipts/<digest> (git cat-file -p)

decide: hale dna review m1 approve|revise|reject|abstain [--as <you>] [--comment <c>] [--digest bf94e503c1c0]
```

Three parts, always together:

- **The source diff** is the change, as git shows it. Read this the
  way you read any pull request; it is the only view that shows what
  the code will *do*.
- **The semantic diff** is what changed in the program's structure
  and rules: components added or removed, who talks to whom, which
  effects are reached, whether a rule went from holding to violated.
  Here: nothing — a comment is `source-only`. When a rule flips, this
  is where it says so in one line.
- **The evidence** is what the toolchain established: format, check,
  verify, the tests, the fleet, a rollback rehearsal, the diff. A
  `NO` in that column is a reason to stop reading.

`magnitude` is how big the change is on several axes at once —
components touched, contracts changed, effects widened, rules
touched, reversibility, how new this kind of change is. Here only
`novelty` shows: nothing has been accepted into this codebase yet.

`hale dna review m1 --iris` opens the same diff in iris beside the
program's live state.

## Decide

```text
$ hale dna review m1 approve --as riley --comment "fine"
review m1 settled: approve by riley
```

- `approve` applies the change.
- `revise` and `reject` do not; the repository stays as it was, and
  the proposal stays in the record with your comment.
- `abstain` records that you looked and leaves it open.

The Board can always answer, whoever the review was waiting on. The
Review checks three things before it accepts a verdict: that it
names the candidate you looked at (the commit in the review; if the
sandbox changed underneath, the verdict is refused and you review
again), that the authority on it satisfies what the review needs
(`board` from the terminal unless you say `--authority`), and that
you are not the one who wrote it.

## Watch

Back in the host's terminal:

```text
hale dna dev: m1 requests a restart (apply bf94e503c1c0… seed . fitness docs_coverage +)
built: …/./chat
hale dna dev: expression restarted (pid 1875137) as 3c9b9327e480d349 build 7a489ad3e72c
hale dna dev: m1 observed healthy for 5s as 3c9b9327e480d349
```

Approval applied the candidate as a commit — `git log` shows it —
then the host rebuilt the program, restarted it, and watched it for
the observation window. It stayed up, so the change is kept. Had it
crashed, the organization would have rolled the repository back to
where it started and asked for the old program back. Under `hale dna
run` with a fleet, the same approval is a deploy row every node
answers, and the window is over every instance the change touched —
[Operating the fleet](./operating.md).

```text
$ hale dna status
…
mutations:  1 (none applies before a human's verdict on the exact candidate)
  m1 [retained] docs: document the chat server in main.hl (main.hl) · task t1 · candidate bf94e503c1c0
```

A change is `retained`, `rolled_back`, `rejected`, or — while it is
still in flight — `review`, `stage` or `escalate`.

## Look back

```text
$ hale dna history m1
record refs/dna/journal — 37 event(s), chain verified
history of m1: 37 event(s)
    7  mutation.proposed      m1             task t1 docs: document the chat server in main.hl (main.hl) at 232dc8f1…
   10  mutation.worktree      m1             opened .hale/dna/worktrees/m1 at 232dc8f1…
   11  model.called           m1/a0          {"adapter": "fake", "backend": "quick", …, "tool_grant": "read edit fmt check @.hale/dna/worktrees/m1", …}
   15  mutation.candidate     m1             bf94e503c1c002f277248b14b6afd1910bb8ce6f
   16  evidence.base          bf94e503…      {"mutation_id": "m1", "step": "base", "ok": true, "code": 0, "output_digest": "b676facc…"}
   …
   28  review.settled         m1             approve by riley
   31  mutation.applied       m1             bf94e503c1c002f277248b14b6afd1910bb8ce6f
   33  expression.restarted   m1             3c9b9327e480d349 build 7a489ad3e72c
   34  expression.observed    m1             healthy 3c9b9327e480d349 up for 5s
   36  mutation.retained      m1             observed healthy as 3c9b9327e480d349
```

The whole story of one change: the ask that started it, the sandbox,
the model calls (what was asked and what it cost, never the prompt),
each check with its receipt, the review, the verdict, the apply, the
restart, the outcome. `hale dna history t1` starts from the request
instead. The receipts are blobs under `refs/dna/receipts/`, named by
their hash; `git cat-file -p` reads one.

`hale dna report` files a summary of everything since the last one —
proposed, reviewed, applied, retained, rolled back, escalated, model
calls and cost — as a row in the record, for the Board.

## From another clone

Everything above works from any clone with the remote. The record
comes with `hale dna sync` (the host syncs every second on its own),
`status`, `review <id>` and `history` read it offline, and `ask` and
a verdict go *into* it: the host beside the organization relays
them, and the answer comes back the same way.

```text
$ git clone git@example.com:you/chat.git && cd chat
$ hale dna sync
$ hale dna review m2
…
$ hale dna review m2 approve --as sam
review m2 settled: approve by sam
```

Two people answering at once is fine: the record is a git branch,
and an append that lost the race is re-appended at the new tail with
its author kept.

## On GitHub

```sh
git config dna.github you/chat
git config dna.github.board riley,sam        # whose GitHub reviews carry the Board's authority
```

With that set, the host — or `hale dna github sync` from any clone
with `gh` logged in — opens a pull request for every pending
mutation Review (the candidate on a `dna/<id>` branch, the three
views as the body), reads every GitHub review on it back as a verdict
in the reviewer's login, comments the settlement back, and pushes the
genome on approval. GitHub is a mirror of the record, never the
record: a review whose head moved is refused here and shows as
refused there.

## Two things to know

- **A key.** The generated organization calls a hosted model and
  reads the key from `OPENAI_API_KEY`. Without it, an ask produces a
  task that fails with `credential not present`, and the Leader
  cannot decide, so every review waits for you. [Shaping
  it](./shaping.md) covers local and scripted models.
- **The purpose review** stays pending until you answer it. It does
  not block anything; it is the organization asking you to say what
  the codebase is for.
