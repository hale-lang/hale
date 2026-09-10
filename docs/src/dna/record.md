# The record

The Journal is a git branch, `refs/dna/journal`, in the governed
repository. Everything the organization does, everything anyone
asked or decided, is a commit on it.

```text
$ git log --oneline refs/dna/journal | head -4
18cd71b review.requested review:m1
72a8ca8 mutation.stage m1
c1a263a evidence.magnitude bf94e503c1c002f277248b14b6afd1910bb8ce6f
e9d9359 evidence.diff bf94e503c1c002f277248b14b6afd1910bb8ce6f
```

- **One commit per event.** The commit's tree holds `journal.jsonl`,
  every event so far, one JSON object per line: `seq`, `kind`,
  `entity`, `body`, `author`. The subject is `<kind> <entity>`.
- **The commit DAG is the chain.** An event's digest is its commit;
  its predecessor is the parent. `hale dna status` reports the chain
  `verified` when the ref's commit count is the row count.
- **Append is compare-and-swap.** A writer builds the next commit on
  the head it read and updates the ref with that head as the expected
  old value. A writer that lost the race reloads and re-appends at
  the new tail; `seq` is the position in the record, never a promise
  made before the append. Two people answering at once lose nothing.
- **Authorship is git's.** The organization's events carry its
  configured author; a person's facts — a verdict, an intent through
  the CLI, a host's crash accounting, a node's reports — carry the
  identity of whoever ran the command. Commit signing, where the
  repository requires it, applies unchanged.
- **Receipts** are blobs under `refs/dna/receipts/<sha256>`, by the
  digest of their content: a verification step's output, a diff
  document. Events name receipts by digest; `git cat-file -p` reads
  one.
- **Leases** are blobs under `refs/dna/lease/<key>`, compare-and-
  swapped the same way, with fencing tokens; a stale token is
  refused.
- **Revisions** a deploy asks for are pushed to
  `refs/dna/revisions/<rev>`, so a node can fetch exactly that commit.

## Sync

```sh
hale dna sync
```

Fetches the remote's record into `refs/dna/remote/journal`,
reconciles, and pushes. Local ahead: push. Remote ahead:
fast-forward. Diverged: the local-only events are re-appended on top
of the remote's head, bodies and authors unchanged, then pushed; a
push the remote refuses is fetched and reconciled again. Receipts and
leases travel by refspec both ways. The remote is `dna.remote` in
git config, or `origin`. The host does this every second; a plain
clone has no record until it syncs.

## The membrane over the record

From a clone with no organization, `hale dna ask` appends
`intent.requested` and a verdict appends `review.verdict`, each in
the appender's git identity. The host beside the organization relays
unanswered rows onto the membrane once, the organization answers
(`intent.offered`, `task.born`, `review.settled`, `review.refused`),
and the answers come back the same way. A row is answered when a
later row of the answering kind names its entity. GitHub is the same
idea with pull requests as the surface — [The Review in
detail](./review.md).

## What is not the record

`.hale/dna/` holds only what is scratch: the membrane sockets, the
status projection `status.json`, one worktree per open Mutation
under `worktrees/<id>/`, the toolchain's inputs and outputs under
`scratch/`, and the artifacts of what was attached and what is
running (`baseline.topology`, `current.topology`,
`previous.topology`). Delete it and nothing the record holds is
lost.
