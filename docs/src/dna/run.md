# Running it

```sh
hale dna run [project] [--port N] [--no-iris] [--observe <secs>]
```

`run` is a **stateless host**. It cuts a fresh artifact of what is
about to run (`.hale/dna/current.topology`), builds, and execs the
organism under `LOTUS_OBS=1` from the project root with `HALE_BIN`
set to the toolchain that started it. It waits for the membrane
sockets, attaches iris — the law view on the fresh artifact, the
review view diffing it against the `init` baseline, the organism
panel, the membrane form — and then supervises: it re-projects the
Journal into `.hale/dna/status.json` once a second, and answers the
organism's restart requests (see [Apply, restart, observe](./apply.md)).
It holds no Task state. When the organism exits, the host reaps iris
and exits with the organism's code.

```text
$ hale dna run . --no-iris
…
hale dna run: organism chat (pid 1694293) from /tmp/dna-docs-session/chat under LOTUS_OBS=1
hale dna run: membrane bound at /tmp/dna-docs-session/chat/.hale/dna
```

With iris attached the line before that names the URL; `l` is the
law view, `4` the review view, `5` the organism, `m` the membrane
form. See [Iris](../systems/iris.md).

## Talking to it

Four commands, and none of them decides anything. The host
publishes and reads; the organism decides; the Journal is the record
both consult.

```sh
hale dna status [--json]              # the projection, from the Journal (works offline)
hale dna ask "document the chat server in main.hl"
hale dna review                       # pending Reviews; `review <id>` renders one
hale dna history [<entity>]           # the Journal, or one entity's causal history
```

**`status`** reads the Journal: tasks born and settled, pending
Reviews and why (the authority they need, the question), mutations
with their disposition and candidate, the expression identity (the
artifact `init` attached, the artifact that would run now, the
build digest), the chain's integrity, and whether an organism is
currently bound to its membrane. Offline it says so:

```text
organism:   not running — reading the Journal
```

Running, the same projection is what iris's organism panel renders:

```text
$ hale dna status
organism:   running (membrane bound)
journal:    41 event(s), chain verified
expression: attached ChatServer (shape 3853f0f14bbf1639) · current shape 8517c3db7499d3b3 · build 68c91b13f399
intents:    1 offered, 0 refused
tasks:      1
  t1 [done] i1a08c0786c5: document the chat server in main.hl
reviews:    2 pending of 2
  m1 [pending] needs maintainer — apply m1 (application): document the chat server in main.hl?
  purpose [pending] needs maintainer — ratify the declared purpose?
mutations:  1 (none applies before a human's verdict on the exact candidate)
  m1 [escalate] application: document the chat server in main.hl · task t1 · candidate dbbb49f550f3
```

**`ask`** publishes a typed `IntentOffered` through the embedded
membrane client and reads the organism's answer back from the
Journal: the Task it birthed, or the refusal.

```text
$ hale dna ask document the chat server in main.hl
task t1 born for intent i1a08c0786c5 [pending]
```

`[pending]` is honest: the Task's Work is routed to the assembly's
editor and takes a few seconds; `status` shows `[done]` once the
Journal says so.

**`review`** is the human's end of the loop and has its own
chapter: [Reviewing](./review.md).

**`history`** walks the Journal from an intent, Task, Mutation or
Review id through the rows that link to it — a Task's Mutation, a
Mutation's candidate, the candidate's receipts, the Review, the
verdict, the apply, the restart, the observation. It is the audit
trail, and it works offline.

## The membrane

The organism binds three typed topics on unix sockets under
`.hale/dna/`:

| topic | subject | who publishes |
|---|---|---|
| `ReviewVerdict` | `dna.review.verdict` | `hale dna review <id> <verdict>`, iris's membrane form |
| `IntentOffered` | `dna.intent.offered` | `hale dna ask`, iris's membrane form |
| `ExpressionObserved` | `dna.expression.observed` | `hale dna run`, after an observation window |

A verdict is admitted by the Review that owns it — the reviewer's
authority, independence from the author, and the candidate digest
are checked there, never by the transport. Intent goes through the
membrane gate (`LocalHumanMembrane.offer`). The observation report
is judged by the assembly against the Mutation's state. The sockets
are how the outside gets *in*; nothing about a decision lives in
them.

## What the organism leaves behind

```text
.hale/dna/
  journal.jsonl                the Journal (the authority)
  baseline.topology            the artifact `init` cut
  current.topology             the artifact of what is running now
  previous.topology            the artifact before the last restart
  status.json                  the projection, re-written every second while running
  hale-dna.*.sock              the three membrane sockets
  worktrees/<id>/              one git worktree per open Mutation
  evidence/<sha256>.txt        one receipt per verification step
  evidence/<id>.base.topology.json, <id>.candidate.topology.json, <id>.diff.json, <id>.diff.txt
```

Delete the directory and you delete the organism's memory. The
genome is untouched: it is your repository.
