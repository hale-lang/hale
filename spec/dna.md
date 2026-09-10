# DNA

DNA is the part of a Hale application that governs how the application
changes. It ships as a library inside the toolchain (`vendor/dna`, the
`dna/core` seed of the hale repository) and as the `hale dna` commands.
This file specifies what the library and the commands promise; the
design is GH #521 and its successor #566; `docs/src/dna/` is the guide.

## The record

The Journal is a git branch, `refs/dna/journal`, in the governed
repository:

- **One commit per event.** The commit's tree holds `journal.jsonl`,
  every event so far, one JSON object per line: `seq`, `kind`,
  `entity`, `body`, `author`. The commit's subject is `<kind> <entity>`.
- **The commit DAG is the chain.** An event's digest is its commit;
  its `prev` is the parent; the first event has no parent. Integrity
  is git's: the ref resolves, and its commit count is the event count.
- **Append is compare-and-swap on the ref.** A writer builds the next
  commit on the head it read and updates the ref with that head as the
  expected old value. A writer that lost the race reloads and, when it
  was appending at the tail, re-appends at the new tail; `seq` is the
  position in the record, never a promise made before the append.
- **Authorship is git's.** The organism's own events carry its
  configured author; a human's facts (a verdict, an intent through the
  CLI, a host's crash accounting) carry the git identity of whoever
  ran the command. Commit signing, where the repository requires it,
  applies unchanged.
- **Receipts** are blobs under `refs/dna/receipts/<sha256>`, filed by
  the digest of their content: a verification step's output, a diff
  document. Events name receipts by digest.
- **Leases** are blobs under `refs/dna/lease/<key>` (`:` in a key
  becomes `/`), `holder`, `token`, `expires`, `present` on four lines,
  compare-and-swapped on the ref. Tokens are monotonic per key.
- **Sync.** `hale dna sync` (and the host, every tick) fetches the
  remote's record into `refs/dna/remote/journal`, reconciles, and
  pushes. Local ahead: push. Remote ahead: fast-forward. Diverged: the
  local-only events are re-appended on top of the remote's head, bodies
  and authors unchanged, `seq` their new position, then pushed; a push
  the remote refuses is fetched and reconciled again. Receipts travel
  by refspec both ways. The remote is `dna.remote` in git config, or
  `origin`. A plain clone has no record until it syncs.
- **The membrane over the record.** From a clone with no organism,
  `hale dna ask` appends `intent.requested` (the body: outcome, from,
  to) and a verdict appends `review.verdict` (the body: the verdict as
  the socket membrane carries it), each in the appender's git identity;
  the host beside the organism relays unanswered rows onto the
  membrane once, and the organism's answers (`intent.offered`,
  `task.born`, `review.settled`, `review.refused`) return the same way.
  A row is answered when a later row of the answering kind names its
  entity. `hale dna ask --no-wait` appends and returns.

`.hale/dna/` holds only what is not the record: the membrane sockets,
the status projection, worktrees, scratch inputs to the toolchain.
Deleting it loses nothing the record holds.

## Event kinds

`application.attached`, `structure.observed`, `responsibility.proposed`,
`law.deferred`, `intent.requested`, `intent.offered`, `intent.refused`,
`review.verdict`, `task.born`,
`task.<state>`, `mutation.proposed`, `mutation.worktree`,
`mutation.located`, `mutation.candidate`, `mutation.<disposition>`,
`mutation.applied`, `mutation.retained`, `mutation.rolled_back`,
`mutation.rejected`, `mutation.revise`, `mutation.refused`,
`mutation.failed`, `effect.requested`, `effect.result`,
`evidence.<step>`, `evidence.magnitude`, `review.requested`,
`review.settled`, `review.refused`, `expression.restart_requested`,
`expression.restarted`, `expression.observed`, `expression.crashed`,
`pressure.raised`, `pressure.remeasured`, `appendage.proposed`,
`model.called`. Their bodies are documented in the guide's reference
chapter; the set grows by ordinary change, and a reader that meets an
unknown kind must keep walking.

## Storage interfaces

`Journal` (ordered append with an expected revision, read by index,
chain verification), `Coordination` (leases with fencing tokens) and
`Receipts` (content-addressed store and read) are interfaces in the
core. The git-backed implementations are the ones an assembly wires
for an organism; the in-memory ones exist for tests.
