# Working with it

The daily loop is five commands. This chapter walks one change
through them, with the output you'll see.

## Start it

```sh
hale dna run                    # iris on :8787; --port N to move it, --no-iris to skip it
hale dna run --observe 30       # watch a restarted program for 30 seconds (default 15)
```

The host holds the program up, keeps `hale dna status` current, and
does the rebuilding and restarting. Stop it with Ctrl-C; the
organism's memory is on disk, and a review left pending is still
pending next time.

## Ask

```text
$ hale dna ask document the chat server in main.hl
task t1 born for intent i1a08c0786c5 [pending]
```

Say what you want as you would to a colleague, and name the file if
you know it. `[pending]` means the organism is working: it has made
a sandbox copy of your repository, asked its model for the change,
formatted and checked it, committed the result there, and is now
verifying it. That takes a few seconds. You don't wait on it — the
next command tells you when there is something to look at.

## Review

```text
$ hale dna review
2 pending review(s) of 2
  m1 needs maintainer — apply m1 (application): document the chat server in main.hl?
      application · candidate dbbb49f550f3 · evidence fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 · disposition escalate
  purpose needs maintainer — ratify the declared purpose?
render one with `hale dna review <id>`; decide with `hale dna review <id> approve|revise|reject|abstain`
```

`m1` is the change. `evidence` is the exit code of each check (0 is
clean). `disposition escalate` is why you are being asked: the
organism's grant covers `refactor docs`, and this is an
`application` change, so it is outside what it may decide alone.
Open it:

```text
$ hale dna review m1
review m1 [pending]: apply m1 (application): document the chat server in main.hl?
  needs maintainer · candidate dbbb49f550f3f021f390710803e48f1b3d1593ff
  mutation m1 (application) by editor · disposition under the grant: escalate · shape 8517c3db7499d3b3
  magnitude: novelty 3

source diff (git 4319af28e30e .. dbbb49f550f3):
   main.hl | 1 +
  --- a/main.hl
  +++ b/main.hl
  @@ -110,3 +110,4 @@ main locus ChatServer {
   fn main() { ChatServer { }; }
  +// documented by the organism: the rooms are the only way to the signer

semantic diff (hale model diff, baseline .. candidate):
  classification: source-only  (shape_hash 8517c3db7499d3b3 -> 8517c3db7499d3b3)
  no semantic differences

evidence (fmt=0 check=0 verify=0 test=0 diff=0 rollback=0):
  step       ok     code  receipt        bytes
  base       yes    0     e607a8e25cfc   26
  fmt        yes    0     e3b0c44298fc   0
  check      yes    0     e3b0c44298fc   0
  verify     yes    0     e3b0c44298fc   0
  test       yes    0     184a3407ce31   67
  rollback   yes    0     a06effb044d9   96
  diff       yes    0     0dfce6838a75   70257

decide: hale dna review m1 approve|revise|reject|abstain [--as <you>] [--comment <c>] [--digest dbbb49f550f3]
```

Three parts, always together:

- **The source diff** is the change, as git shows it. Read this the
  way you read any pull request; it is the only view that shows
  what the code will *do*.
- **The semantic diff** is what changed in the program's structure
  and rules: components added or removed, who talks to whom, which
  effects are reached, whether a rule went from holding to
  violated. Here: nothing — a comment is `source-only`. When a rule
  flips, this is where it says so in one line.
- **The evidence** is what the toolchain established: format,
  check, verify, the tests, a rollback rehearsal, the diff. A `NO`
  in that column is a reason to stop reading.

`magnitude` is how big the change is on several axes at once —
components touched, contracts changed, effects widened, rules
touched, reversibility, how new this kind of change is. Here only
`novelty` shows: nothing has been accepted into this program yet.

`hale dna review m1 --iris` opens the same diff in iris beside the
program's live state, and the verdict can be sent from there too.

## Decide

```text
$ hale dna review m1 approve --as riley --comment "the rooms stay the only way"
review m1 settled: approve by riley
```

- `approve` applies the change.
- `revise` and `reject` do not; the program and the repository stay
  as they were, and the proposal stays in the history with your
  comment.
- `abstain` records that you looked and leaves it open.

`--as` is your name on the record. The organism checks three things
before it accepts a verdict: that it names the candidate you looked
at (the commit in the review; if the sandbox changed underneath,
the verdict is refused and you review again), that you have the
authority the review needs (`maintainer` by default), and that you
are not the one who wrote it.

## Watch

Back in the host's terminal:

```text
hale dna run: m1 requests a restart (apply dbbb49f5… fitness guests_greeted +)
hale dna run: organism restarted (pid 1694392) as 8517c3db7499d3b3 build b1ac50c8c3ef
hale dna run: m1 observed healthy for 5s as 8517c3db7499d3b3
```

Approval applied the candidate to your repository as a commit —
`git log` shows it — then the host rebuilt the program, restarted
it, and watched it for the observation window. It stayed up, so the
change is kept. Had it crashed, the host would have rolled the
repository back to where it started and restarted the old program;
had you rejected it later, the same.

```text
$ hale dna status
…
tasks:      1
  t1 [done] i1a08c0786c5: document the chat server in main.hl
mutations:  1 (none applies before a human's verdict on the exact candidate)
  m1 [retained] application: document the chat server in main.hl · task t1 · candidate dbbb49f550f3
```

A change is `retained`, `rolled_back`, `rejected`, or — while it is
still in flight — `review`, `stage` or `escalate`.

## Look back

```text
$ hale dna history m1
```

is the whole story of one change: the ask that started it, the
sandbox, the model calls (what was asked and what it cost, never the
prompt), each check with its receipt, the review, your verdict, the
apply, the restart, the outcome. `hale dna history t1` starts from
the request instead. The receipts themselves are files under
`.hale/dna/evidence/`, named by their hash.

## Two things to know

- **A key.** The default settings call a hosted model and read the
  key from `OPENAI_API_KEY`. Without it, an ask produces a task that
  fails with `credential not present`. [Shaping it](./shaping.md)
  covers local and scripted models.
- **The purpose review** stays pending until you answer it. It does
  not block anything; it is the organism asking you to say what the
  program is for.
