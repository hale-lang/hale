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

## The membrane over the record

From a clone with no organization, `hale dna task create` appends
`intent.requested` and a verdict appends `review.verdict`, each in
the appender's git identity. The host beside the organization relays
unanswered rows onto the membrane once — every writer trusted by
default (`dna.trust = local`), or only signed commits git verifies
(`dna.trust = signed`; an unverified row is refused in the record and
never relayed) — the organization answers
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
hale dna ledger adopt          # asks the body to adopt
hale dna ledger abandon --why "back to one memory"
```

`adopt` appends `ledger.adopting` to the record. The body that holds
the spine lease (see [Operating](./operating.md#the-spine-lease)), on
its next tick, copies every operational row of the record into the
ledger keyed by its commit (rerun after any interruption; nothing is
copied twice), carries its own body lease into memory at the token it
holds, and appends `ledger.adopted` naming the checkpoint. From that
commit on, an operational row never goes into git: the organization
appends it to the ledger itself, and everyone else asks for it. The
record keeps every row it ever held; `hale dna history` reads both
memories as one.

Once adopted, the leases move too: the mutation leases the gateway
takes and the body lease `hale dna run` holds are rows of memory's
lease table, swapped by their token, and the fence renews a row rather
than a ref. A host that lost its lease presents a stale token and is
refused. Over a shared record (the owners map) the body lease is one
row per owner, `owner/<owner>`, so every owner's body runs side by
side; the lease's token is an epoch the host hands its organization,
and a write under a lease that was taken over, released or expired is
refused `fenced`. If memory becomes unreachable, the fence keeps the
lease it last proved until just before it expires and then stops the
organism; `hale dna status` says the ledger is unreachable and that
nothing is admitted until it answers.

### A head's writes are requests

The ledger has one writer: the spine's role. A head — the CLI in your
clone, `hale dna ui`, the read API — never writes it. Once adopted, a
verb that writes the day's work in your name records a **request**
instead: a `ledger.requested` row in the record, saying what to write,
in whose name, and the ledger revision the decision was read at. The
verb tells you so, with the request's digest:

```text
$ hale dna receipt hold sha256:9d0e… --why "audit" --as sam
hale dna: `receipt.held sha256:9d0e…` is requested of the ledger as 3f1c2a9e0b77 (this organism's operations live there since 232dc8f18dde); the body admits it on its next tick — `hale dna history sha256:9d0e…` shows the outcome
receipt sha256:9d0e… held (receipt.held, by sam)
```

The spine admits each request once, on its tick, keyed on the
request's digest, so a request seen twice lands once. It checks the
request there, in the spine, against the ledger as it stands: a person
who retired (`person.retired`) is refused, a completion for a task
handed to someone else is refused, a transfer is accepted only by a
member of the owner it was offered to, a claim already taken is
refused, and a decision read at a revision the ledger has since moved
past is refused as stale. A request memory could not take for any
other reason stays undecided and is tried again on the next tick. Over a shared record the request must also carry a valid
signature of its owner's key (see
[Operating](./operating.md#shared-records-and-owners-keys)). An
admitted request lands in the ledger in the person's name; a refused
one is a `ledger.request_refused` row in the record with the reason.
`hale dna history` and `hale dna status` show which, once the spine
has run. The spine writes under the spine lease it holds: one that lost
it lands nothing, and the next holder decides the request.

Because a request is a row of the record, a head needs no memory to
make one: the record is the pager. A clone with no DSN named requests
a write at the tail as any other, and `sync` carries the request to the
body. A write decided at a ledger revision — `task done`, a
completion — needs that revision read, so without memory its verb
refuses ("the ledger's revision was not read") and requests nothing;
with memory it is requested with the revision, and says so with the
request's digest and that revision. Before
adoption (routing 0) there is nothing to request — operational rows go
into the record as they always did. The host's own operational writes
go the same way, signed as its owner.

`hale dna ledger rows` prints the ledger as memory holds it, one JSON
object per line, for a head that names memory
(`HALE_DNA_MEMORY_DSN_HEAD`).

Evidence keeps its treatment across the two memories: a bill's rows
are the ledger's, its body is where it always was, and a redaction
after adoption removes a body filed before it. The books export reads
the ledger with `hale dna ledger rows` beside the record and never runs
SQL of its own.
