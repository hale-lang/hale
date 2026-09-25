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
  `verified` when the commit count at the head it loaded is the row
  count, and names that head.
- **Append is compare-and-swap.** A writer builds the next commit on
  the head it read and updates the ref with that head as the expected
  old value. A writer that lost the race reloads and re-appends at
  the new tail; `seq` is the position in the record, never a promise
  made before the append. A lost race is retried until the row lands,
  not given up on. Two people answering at once lose nothing.
- **A reader's view of the record only ever grows.** The chain never
  loses a row (a reconcile swaps in a whole chain rather than rewinding
  the ref), so a read that comes back with fewer rows than the reader
  holds — or with no chain at all — is a read that failed, not a
  shorter record: `git` unable to run on a loaded machine, the
  repository out of reach for a moment. The reader keeps what it had,
  counts the failure, says "nothing moved", and tries again at its next
  refresh. This matters more than it sounds: how often a source has
  raised a concern, which Review is open and which Task is in flight
  are not numbers the organization keeps but counts it makes by reading
  its own record, so an organization that let one failed read empty its
  view would start every one of them again from nothing — and say
  nothing about it. The same goes for the head itself: a read of it
  either happened (the chain's head, or no chain at all) or failed, and
  a failed one is never taken for "no record" — a command refuses
  with the reason instead of acting on a record that is not there.
- **Authorship is git's.** The organization's events carry its
  configured author; a person's facts — a verdict, an intent through
  the CLI, a host's crash accounting, a node's reports — carry the
  identity of whoever ran the command. Commit signing, where the
  repository requires it, applies unchanged.
- **Receipts** are blobs under `refs/dna/receipts/<sha256>`, by the
  digest of their content: a verification step's output, a diff
  document. Events name receipts by digest; `git cat-file -p` reads
  one. A customer or confidential body is never one of these: memory
  keeps it, sealed under a key that lives in memory, and the record
  holds only its digest and its class. With no memory to keep it, the
  body is withheld (`receipt.withheld`) — the record still has its
  digest and class, and nothing holds the body. `hale dna receipt`
  lists them; `hale dna receipt disclose <digest> --to <who>
  --purpose <p>` authorizes a reader; `hale dna receipt show <digest>
  --purpose <p>` reads one in your name, and the read is a row.
  Nothing protected travels with `sync`. A body can be redacted under a stated policy:
  `hale dna receipt redact <digest> --why <why> --policy <policy>`
  records who, why and under what policy, and the body goes — a git
  receipt's ref at once, a protected body when the spine erases it on
  its next tick — while the record keeps the digest. Memory keeps the
  digest of what it erased too, so a redacted body is refused if
  anyone files it again. `receipt hold` stops redaction until `receipt
  release-hold`. Each clone drops a redacted receipt at its next sync;
  the object lingers until git collects it, and a copy made outside
  the record cannot be recalled.
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
push the remote refuses is fetched and reconciled again. A sync that
cannot read either head refuses and changes nothing. The
reconciled chain is built beside the ref and swapped in with one
compare-and-swap, so the record never loses a row it held a moment
before: a reader's view only grows, a writer that reloads finds its
own rows, and a clone that appended meanwhile is not overwritten (the
swap fails and the reconcile goes round). Receipts and
candidates travel by refspec both ways; mailboxes are received. The
remote is `dna.remote` in git config, or `origin`. The host does this
every second; a plain clone has no record until it syncs.

Under `git config dna.trust signed`, a reconcile rebuilds only what
this clone may sign. A row this clone signed is re-signed and its
commit names the original (`Rebuilt-From: <commit>`); a row that was
never signed is rebuilt unsigned; a row signed with another key
refuses the whole sync before anything moves:

```text
$ hale dna sync
hale dna: the record diverged, and local event 8c1f… (intent.requested i-4, by riley)
was signed with key SHA256:zzu3…, not this clone's; a reconcile would re-sign it as
this clone's, so the record was not changed and every local row is kept at
refs/dna/journal. Have that row's writer sync first — a writer rebuilds its own
rows — then sync again to fast-forward; or fetch and fast-forward once the remote
holds it
```

Nothing is discarded: the rows are still on `refs/dna/journal` in
this clone, and a fast-forward is never refused. Under the default
`local` trust every row is rebuilt as before.

## Candidates

A candidate is the commit a Mutation's worktree ends in. The record
keeps a pointer to each under `refs/dna/candidates/<mutation>`, shared
with the record, so a refused or revised change stays readable as a
diff after its worktree is gone:

```sh
hale dna candidates                 # every kept candidate, with its Review's state
hale dna candidates m7              # one, as a diff from its Review's base
hale dna candidates drop m7 --why "superseded by m9"
```

`drop` is a row, `candidate.dropped <mutation> {by, why}`: the pointer
goes here and at the remote, and every other clone drops it at its
next sync, the way a redacted receipt goes. The commits stay in each
object store until git collects them.

## The record's own API

Everything above is one interface in the core, `Record`, with a
vocabulary that never names git: chains of rows, bodies by digest,
cells swapped by version, pointers, families received and shared,
the signer of a row, the `dna.*` settings. `GitRecord` is the one
implementation over git plumbing, and the only file in the core and
the host that spells `git` for the record; `MemRecord` is the one
fixtures use. The organism's `GitJournal`, `GitReceipts` and
`GitLeases`, the host's sync and body lease, and the mailboxes between
records all stand on it — which is what lets the operational memory
move to a store without the record changing shape.

## Requests are rows

From any clone, `hale dna task create` appends `intent.requested` and
a verdict appends `review.verdict`, each in the appender's git
identity; neither publishes anything. The host beside the organization
relays unanswered rows onto [the nerves](./run.md#the-nerves), and
again every 30 seconds until they are answered — every writer trusted by
default (`dna.trust = local`), or only signed commits git verifies
(`dna.trust = signed`; an unverified row is refused in the record and
never relayed) — the organization answers
(`intent.offered`, `task.born`, `review.settled`, `review.refused`),
and the answers come back the same way. A row is answered when a
later row of the answering kind names its entity. GitHub is the same
idea with pull requests as the surface — [The Review in
detail](./review.md).

## What is not the record

`.hale/dna/` holds only what is scratch: the status projection
`status.json`, one worktree per open Mutation
under `worktrees/<id>/`, the toolchain's inputs and outputs under
`scratch/`, and the artifacts of what was attached and what is
running (`baseline.topology`, `current.topology`,
`previous.topology`). Delete it and nothing the record holds is
lost.

## The ledger

The record is one of three memories. The Structure is the codebase;
the record is how the organism changed and was allowed to; the
**ledger** is what it did today — intents, tasks, decisions, bills,
money reserved and settled, schedules fired, concerns, effect claims,
liveness — in memory, Postgres, in the record's own schema. Every row
kind has exactly one home (`dna::memory_of`), and the organism's
routing version is a fact of its record, never a build's opinion.

A new organism starts on routing 0: every row in the record, as
before. Moving the day's work to the ledger is an explicit, one-way
step, and the body carries it out:

```sh
hale dna ledger                # routing, the memory named here, the cutover
hale dna ledger adopt          # asks a node to adopt
hale dna ledger abandon --why "back to one memory"
```

`adopt` appends `ledger.adopting` to the record. On its next tick a
node (see [Operating](./operating.md#the-spine-is-every-node)) claims
the ask, copies every operational row of the record into the ledger
keyed by its commit (rerun after any interruption; nothing is copied
twice), carries its own body lease into memory at the token it holds,
and appends `ledger.adopted` naming the checkpoint. From that commit
on, an operational row never goes into git: whoever writes it — the
organization, a head — writes it straight into the ledger. The record
keeps every row it ever held; `hale dna history` reads both memories
as one.

Once adopted, the leases move too: the mutation leases the gateway
takes and the body lease `hale dna run` holds are rows of memory's
`claims` table, swapped by their token, and the fence renews a row rather
than a ref. A host that lost its lease presents a stale token and is
refused. Over a shared record (the owners map) the body lease is one
row per owner, `owner/<owner>`, so every owner's body runs side by
side; the lease's token is an epoch the host hands its organization,
and a write under a lease that was taken over, released or expired is
refused `fenced`. If memory becomes unreachable, the fence keeps the
lease it last proved until just before it expires and then stops the
organism; `hale dna status` says the ledger is unreachable and that
nothing is admitted until it answers.

### A head writes the ledger directly

Once adopted, a verb that writes the day's work in your name writes it
straight into the ledger, as your head's role in memory, and memory's
insert function is the gate: the row lands, or it is refused with the
reason, there and then. The verb says which:

```text
$ hale dna receipt hold sha256:9d0e… --why "audit" --as sam
receipt sha256:9d0e… held (receipt.held, by sam)

$ hale dna task done acme:t4 --as alice
hale dna: `task.done acme:t4` was not written to the ledger: task acme:t4 is handed to bob, not to alice; a completion is admitted in the assignee's name
```

The gate checks the row against memory as it stands: a person who
retired (`person.retired`, which every node projects into memory) is
refused, a completion for a task handed to someone else is refused, a
transfer is accepted only by a member of the owner it was offered to, a
claim already taken is refused, an author your head's role does not
write as is refused, and a decision read at a revision the ledger has
since moved past is refused as stale — the verb reads again and decides
again. Over a shared record your head writes as your owner's role (see
[Operating](./operating.md#shared-records-and-owners-roles)), which
writes only in your owner's members' names. A write carries a request
id, so one retried after an answer that was lost lands once.

A head with no memory named writes nothing once the ledger is adopted:
the row's home is the ledger, and the verb says so, naming the
checkpoint — it never falls back to git. Before adoption (routing 0)
operational rows go into the record as they always did. The host's own
operational writes go the same way, as `host`.

`hale dna ledger rows` prints the ledger as memory holds it, one JSON
object per line, for a head that names memory
(`HALE_DNA_MEMORY_DSN_HEAD`).

Evidence keeps its treatment across the two memories: a bill's rows
are the ledger's, its body is where it always was, and a redaction
after adoption removes a body filed before it. The books export reads
the ledger with `hale dna ledger rows` beside the record and never runs
SQL of its own.
