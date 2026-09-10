# DNA: a governed application

Your program can change itself. You hold the door.

**DNA** gives a Hale application a way to take a request, propose a
change to its own source in a sandbox, prove the change checks and
tests, show you exactly what it did, and — only after you say yes —
apply it, restart itself, and watch that it still works. Every step
is written down where you can read it later.

```sh
hale dna init .                                  # attach it to your application
hale dna run                                     # start it
hale dna ask "document the chat server in main.hl"
hale dna review m1                               # read the change and the evidence
hale dna review m1 approve --as riley            # your call
```

## The loop

1. **You ask.** In a sentence. The organism turns it into a task.
2. **It proposes.** In a sandbox copy of your repository, with a
   model, it edits the file, formats it, checks it.
3. **It proves.** The candidate is checked, verified, tested, and
   diffed against your program's structure. Every result is kept
   with a receipt.
4. **You review.** The diff, what changed in the program's shape
   and rules, and the checks — pinned to one commit. Approve,
   revise, or reject.
5. **It applies and watches.** Approval becomes a commit. The
   program rebuilds, restarts, and is watched for a while. If it
   stays up, the change is kept. If not, it is rolled back.

## What you keep

- **The decision.** Nothing is applied without your verdict.
- **The source.** A change is always shown as a source diff, not a
  summary of one.
- **The story.** `hale dna history m1` is everything that happened
  to one change, in order: the ask, the sandbox, the checks, the
  review, the apply, the restart, the outcome.
- **The way back.** A change that crashes the program, or that you
  reject after the fact, is rolled back to the commit it started
  from.

## What it looks like

```text
$ hale dna ask document the chat server in main.hl
task t1 born for intent i1a08c0786c5 [pending]

$ hale dna review
2 pending review(s) of 2
  m1 needs maintainer — apply m1 (application): document the chat server in main.hl?
      application · candidate dbbb49f550f3 · evidence fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 · disposition escalate

$ hale dna review m1 approve --as riley --comment "the rooms stay the only way"
review m1 settled: approve by riley

hale dna run: organism restarted (pid 1694392) as 8517c3db7499d3b3 build b1ac50c8c3ef
hale dna run: m1 observed healthy for 5s as 8517c3db7499d3b3
```

## Where to go

- [Getting started](./getting-started.md) — attach it, run it,
  answer the first review.
- [Working with it](./working.md) — the daily loop.
- [Shaping it](./shaping.md) — the purpose, how much autonomy,
  which models.
- [What it will and won't do](./limits.md) — the plain limits.
- [Troubleshooting](./troubleshooting.md) — the messages you'll
  meet.

The mechanism — the Journal, the gateways, the review's pin, the
autonomy rules — is in the *under the hood* chapters that follow.
You don't need them to use it.
