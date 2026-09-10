# The host and the membrane

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

## The commands are projections

`status`, `review`, `history` read the Journal and the receipts;
`ask` and a verdict publish one typed fact through the embedded
membrane client and read the organism's answer back from the
Journal. None of them decides anything, and all but the two that
publish work offline. [Working with it](./working.md) is their
user-facing side.

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
