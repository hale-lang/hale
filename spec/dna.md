# DNA

DNA is the part of a Hale application that governs how the application
changes. It ships as a library inside the toolchain (`vendor/dna`, the
`dna/core` seed of the hale repository) and as the `hale dna` commands.
This file specifies what the library and the commands promise; the
design is GH #521 and its successor #566; `docs/src/dna/` is the guide.

## Three memories

An organism has three memories, and every row kind belongs to exactly
one (GH #646).

| Memory | Holds | Lives in | Written by |
|---|---|---|---|
| **Structure** | what the organism *is*: positions, routes, schedules and grants as authored, the law, the app, the models catalog | the codebase | the editor, under review |
| **Record** | how the organism *changed* and was allowed to: mutations and their reviews, practices proposed and ratified, authority contracted or revoked, connections, provisioning, people, the adoption of the ledger | git, `refs/dna/*`, one signed commit per row, cloned and synced | the Board, the organism's pipeline, a head from its clone |
| **Ledger** | what the organism *did today*: intents, tasks, assignments, decisions, bills and receipts, money reserved and settled, schedules fired, concerns, handoffs, effect claims, liveness | memory: Postgres, in the record's own schema `dna_<identity>` | every writer under its own role — the nodes as the spine's, each head as its own — through memory's insert function, the gate (GH #1026) |

The routing table is `memory_of` in the core (`routing.hl`): the
`case`, `intent`, `task`, `decision`, `completion`, `exception`,
`schedule`, `receipt`, `handoff`, `pressure`, `instance`, `effect`,
`report`, `lease` and `claim` families are the Ledger's, as are
`concern.requested` / `.raised` / `.refused`, `grant.reserved` /
`.released` / `.fenced` / `.reservation_refused` (money;
`grant.refused` — a grant born wider than its ceiling — is authority
and the record's), `spend.reserved` / `.settled` / `.compensated`,
`body.claimed` / `.released` / `.credential_*`, `model.called`,
`knowledge.consulted`, `optimize.refused`, `budget.exhausted`, and a
workflow execution's own facts (`workflow.admitted` / `.refused` /
`.ask_refused` / `.settled`, `step.registered` / `.activated` /
`.completed` / `.failed`, `work.settled`, `attempt.admitted` /
`.outcome`). Every other kind, and any
kind a build does not know, is the record's. **Routing is versioned and
is a fact of the record**: version 0 is every kind in the record;
version 1 is the table; an organism is on the version its last
`ledger.adopted` row names, and on 0 again after `ledger.abandoned`.
There is no dual-write: a kind has one home, and a writer that cannot
reach it fails rather than writing elsewhere.

`RoutedJournal` is the `Journal` an organism and the host stand on:
the record's journal and the Ledger's read as one merged sequence (a
row's `seq` as read is its merged position; rows name each other by
id), appended to by kind under the record's routing. The Ledger is
`MemoryLedger` (`dna/core/memory_spine.hl`) over `PqLedger` (Postgres,
the record's `dna_<identity>` schema, one insert conditioned on the
tail, the claim kinds `effect.requested`, `spend.reserved` and
`task.born` unique by `(kind, entity)`), opened on first use as the
role its process's environment names (**Memory**, below). There is no
service in front of memory (GH #985): a part that reads or writes it
opens Postgres itself, and the role's grants are the trust boundary.
The organization's Ledger carries the body lease and the epoch the
host handed it (`HALE_DNA_LEASE`, `HALE_DNA_LEASE_TOKEN`): an append
under a lease that is not held, is held at another token, or has
expired lands nothing (`fenced: …`).

**Adoption** is explicit, per organism and one-way. `hale dna ledger
adopt`, with the record synced, appends `ledger.adopting {routing,
by}` to the record: a request, which the spine carries out on its
tick (**The spine**, below). It copies every operational row of the
record up to that row into the Ledger keyed by its commit (so a copy
interrupted anywhere is rerun, never repaired), records the cutover,
carries the running body's lease into memory at its token (unless
memory already holds a lease under that key), and appends
`ledger.adopted {routing, checkpoint, rows, by}`: the checkpoint is
the `ledger.adopting` row's commit, the record head the copy was taken
at, and from that commit on operational kinds never go into the
record. Historical operational rows stay in git, read-only; readers of
history (`history`, the projections, the export) read both memories as
one. `hale dna ledger abandon --why` appends `ledger.abandoning {by,
why}`; the spine empties the Ledger and appends `ledger.abandoned {by,
why}`, and the organism is on routing 0 again with nothing lost.
Until a body with memory runs, an adoption or abandonment waits in the
record. `hale dna run` on an adopted record refuses to start without
`HALE_DNA_MEMORY_DSN_SPINE` (#646 decision 3: a body without its
memory admits nothing); `dev` applies memory's schema and hands the
host the spine's DSN. `hale dna ledger [status]` says the routing,
whether memory is named here, and the Ledger's row count and cutover;
`hale dna ledger rows` prints the Ledger's rows as JSON lines (`seq`,
`kind`, `entity`, `body`, `author`, `writer` — the role that wrote it, so a head writing as `host` is told apart from the body — `prev`, `digest`), read under the
head's role or the spine's.

**Writers write directly; the store is the gate (GH #1026).** A head —
the CLI, `hale dna ui`, the API — writes a Ledger row itself, straight
into memory under its own role, and a node writes the organism's rows
under the spine's; nothing sits in front of memory and no process
admits anything. Every Ledger row lands through memory's insert
function, `ledger_append(expected, kind, entity, body, author, lease,
epoch, request)`, a definer function run as the schema's owner, and the
calling role is who wrote it (`session_user`, kept in the row's
`writer` column beside `lease` and `epoch`). It refuses, returning the
reason the verb prints:

1. a kind that is not the Ledger's (`ledger_kind`, the routing table
   above as data): it is appended to the record instead;
2. an author the calling role does not hold. `memory_principals`,
   owner-only, maps each role to whom it writes as: `*` any author and
   no person checks (the spine's role, and the schema's owner); `@` any
   person (the head's role over a record with one owner); an owner's
   name, that owner's members (each owner's head role over a shared
   record, **The owners' roles**, below); `host` for any head;
3. a person the record retired (`org_retired`, below);
4. a `task.transfer_accepted` for a Task not offered to an owner, or in
   the name of someone who is not the member of the owner it is offered
   to;
5. `task.done`, `task.failed`, `completion.linked` and
   `completion.excepted` in any name but the Task's assignee's;
6. a row naming a lease — the body's, which its organism carries — whose
   claim is not live at that token in `claims` right now: `fenced`, with
   whose it is and at what epoch;
7. a revision a decision was read at that the Ledger has moved past
   (`stale_revision`; a `task.transfer_accepted` lands at the tail
   whatever its `expected`, GH #1052, since check 4 reads the transfer
   against the Ledger as it stands), and a claim kind whose entity is
   already taken (`claimed`).

Checks 3 to 5 are not the role's own business, so they apply to every
role but a `*`. Appends are serialized on the schema, and the row's
digest extends the chain as `event_digest` does. A write carries a
request id minted when it is made (`ledger_requests`): a write retried
after an answer that was lost is answered as it landed, once. A head's
verb says it is done when the row lands, and says why when it is
refused; nothing is pending and there is nothing to wait on. A head with
no memory named writes nothing, naming the checkpoint the organism's
operations live behind since — never into git instead. A write decided
at a revision needs the Ledger's revision read, and without memory its
verb refuses (`the ledger's revision was not read`). Nothing on a head
is authoritative, and no head executes work offline (#646 decision 2).
On routing 0 an operational row goes into the record, as it always did.
An adoption and an abandonment are the record's `ledger.adopting` and
`ledger.abandoning` rows above, carried out by a node under a claim on
the asking row (**Claims by id**, below).

**The org chart in memory (GH #1026).** The checks above read what the
spine projects: `org_members(person, owner)` is the genome's owners map
(`dna/org/owners`), replaced whole in one transaction whenever its text
changes; `org_retired(person, seq)` is the record's `person.retired`
rows, projected in the same per-row transaction as the graph (and
emptied with it on a rebuild). The head reads both, so a verb that
checks before writing — a retired person, a transfer's owner — says so
in its own words; the store's check is the gate, the head's the
message.

**The owners' roles (GH #1026).** Over a shared record each owner's
heads write the Ledger as that owner's role, `<schema>_head_<owner>`,
which the migration creates for every owner the owners' keys name
(`HALE_DNA_OWNER_KEYS`, `<owner>=<key> …`, in the owner's environment
at migration) and prints as `HALE_DNA_MEMORY_DSN_HEAD_<OWNER>=<dsn>`;
the plain head role of a shared record reads and writes as no one. A
role is who a head is: memory holds no signature bytes, and the keys
stay the record's until the vault holds them (#989). **The host is a
principal**: it writes its own rows as `host`, under the spine's role
or an owner's, and its `body.*` rows carry `owner` in their body when
the record names one, so a row by `host` still says whose body it was.
Who a person is remains the head's to establish — its issuer under
`dna.principal = oidc`.

**Admission, as a reader checks it (GH #1026).** A row of the record
counts only when `dna::row_admissible(record, row, trust)` says so:
under `dna.trust = signed` its commit carries a signature the record's
trust verifies (git's keyring or allowed-signers file), and an unsigned
row, or one signed with a key the record does not know, is refused;
under `local` trust every writer is trusted. The projection runs it on
every row it applies and moves its stamp past a refused row without
projecting anything of it. A Ledger row needs no reader's check: the
insert function refused it before it landed.

**Evidence and the export across two memories (stage 4, #653).**
Receipts are placed by what they evidence: `evidence.*` (a
verification step's output, a diff document — evidence of a mutation)
and `review.reasoned` are the record's; `receipt.*` — a bill filed, its
class, its disclosure, its reads, its holds, its redaction — are the
Ledger's, whatever the class, and the body itself is where it always
was (a git blob for an internal body, memory's sealed store for a
protected one; only the digest is ever a row). A redaction of a git
receipt after adoption is therefore a Ledger row, and the body it names
may be a blob filed before adoption (a head's redaction of a protected
body is a row of the record, which the spine reads to erase it): sync applies redactions and candidate drops
over both memories read as one, so the treatment holds across the
split. The books export is a supported ledger query: it reads `hale
dna ledger rows` beside the record's rows and never SQL of its own.
Recovery between the two memories needs no transaction spanning them:
every step that touches both writes its intent in the memory that owns
the decision, acts, and writes its outcome, and the restart scans the
merged journal — a task settles on its mutation's outcome, a settled
review births its task once, an effect claim is resolved by evidence
or gated as unknown, a retirement's reassignments are reapplied, a
filed body without its row is filed again — exactly as they did over
one journal, because the routed journal is that journal. **The
continuity gate** is a pre-split record built with the real git
journal — unfinished work, a reservation, a practice bound to a handed
task, a retired participant, redacted evidence, a pending handoff, a
schedule — adopted through the same steps a live organism takes, its
projections read before and after, and a redaction after adoption
removing a body filed before it (`dna_ledger.rs`).

**Handoffs between records (stage 5, #662).** A handoff crosses
between records as an envelope through an `Exchange`, the core's
contract for one record's mailbox in another: `peer_identity`,
`deliver` (idempotent by the envelope's kind and entity — its entity
is `handoff:<id>` and the id is the handoff's origin, peer, kind and
subject, so a delivery repeated after an interruption is one
envelope), `delivered` (whether the peer already holds it), `received`
(every mailbox of this record). The exchange is git's alone: mailbox
refs in a private peer cache of the other record's remote,
`refs/dna/exchange/<sender>`, beside the identity each record
publishes under `refs/dna/identity`. `hale dna connect` refuses an
`http://` or `https://` url: a connection exchanges through the other
record's git remote. The exchange keeps the contract #646 set:
**durable delivery** — a handoff published here whose envelope the
peer does not hold is delivered again, once, by `hale dna handoff sync`
from the envelope kept with its `handoff.published` row; a delivery
that cannot reach the peer at all records nothing and says to deliver
again; **the disclosure scope** is the connection's classes, checked
before anything is delivered and again before anything is admitted;
**settlement only on the receiver's acceptance** — the origin's task
moves to `transfer_accepted` when the receiver's `handoff.accepted`
envelope reaches it through the exchange, never when a delivery
returned.

**Coordination in memory (stage 2, #651).** On routing 1 every lease
is a row of memory's `claims` table, swapped by its token:
`MemoryLeases` is `Coordination` over `current(key)` and `put(key,
lease, expected)`, and a put lands only when the stored token is the
one expected (0 for an absent lease). `GitLeases` — the record's
coordination — takes a mutation lease from the record's cells on
routing 0 and from memory on routing 1, reading the routing from the
record whenever its head moved: an adopted organism's leases move with
it, and nothing in its wiring changes. The body lease moves the same
way: on routing 1 it is the row `body` (`owner/<owner>` over a shared
record), the fence renews the row every third of its life with every
call bounded, a stale token — a host that lost the lease to another —
lands nothing, `hale dna body` reads it there, and `refs/dna/lease/*`
is not written. A head takes and fences a lease under its own role.
Effect claims are unique rows of the Ledger (`(kind, entity)`), so the
contended-claim row the git rule needed does not arise. Memory lost
after activation is the case the git lease already handles: the
renewal fails, the fence keeps the lease it last proved until the
margin before its expiry and then stops the organism, and `status`
says THE LEDGER IS UNREACHABLE and that nothing is admitted until it
answers.

## Memory

Memory is Postgres, and only Postgres (GH #985). Its stores are the
core's — `dna/core/memory_store.hl` (the graph, `Pq`),
`memory_ledger.hl` (`PqLedger`, `PqLeaseStore`), `memory_protected.hl`
(`PqProtected`), `memory_embed.hl`, `memory_schema.hl` (the schema,
the roles, the grants and the functions) and `memory_spine.hl`
(`Memory`, `MemoryLedger`, `MemoryLeases`, `MemoryKnowledge`,
`MemoryVault`, `LedgerAdoption`) — and they open Postgres through
pond's driver, pinned under `dna/core/pond` and vendored beside the
core at `vendor/dna/pond`. The projection tail, `MemoryProjection`, is
not in the core: it lives in `dna/operations/memory_tail.hl`, beside
the command codecs it decodes admitted facts with, and runs on the
host's tick (**The spine**, below).

- **Memory is per record.** Each record has its own schema,
  `dna_<identity>` (the identity lower-cased, any other character `_`,
  at most 56 characters of it), and two roles of its own,
  `dna_<identity>_spine` and `dna_<identity>_head`, cut to fit
  Postgres's 63-byte names: one role granted on every record's schema
  would read every other record's evidence on the same server. Until
  the vault holds them (GH #989) a role's password is a placeholder
  equal to the role's name.
- **Three DSNs.** `HALE_DNA_MEMORY_DSN_OWNER` is the schema owner's,
  used only to migrate: by `hale dna memory migrate [dir]`, by `hale
  dna dev` before it starts the host, by `hale dna upgrade` when it is
  set, and by a provisioned body, whose env file carries it for the
  `hale dna dev` its unit runs. Without it, `memory migrate` and `dev`
  use the database `dna/compose.yaml` brings up. `hale dna memory
  migrate` prints two lines, `HALE_DNA_MEMORY_DSN_SPINE=<dsn>` and
  `HALE_DNA_MEMORY_DSN_HEAD=<dsn>`: the owner's host and database, each
  role's name and credentials. The host that runs the organism is
  handed `HALE_DNA_MEMORY_DSN_SPINE` alone, and the organization
  inherits it: `dev` sets it from the migration, and `hale dna run`
  takes it from its own environment; both take
  `HALE_DNA_MEMORY_DSN_OWNER` and `HALE_DNA_MEMORY_DSN_HEAD` out of the
  host's. With no database to migrate, `dev` says why and starts the
  host without the owner's DSN. A head — the CLI, `hale dna ui`, the API — opens memory with
  `HALE_DNA_MEMORY_DSN_HEAD`; a process given the spine's DSN uses that
  instead, and none holds both.
- **The migration** runs in one transaction under an advisory lock on
  the schema: the `vector` and `pgcrypto` extensions, the schema and
  every store's tables, the record's claim on the schema
  (`knowledge_meta.record`; a schema that names another record is
  refused), the projection protocol, the receipt functions and the
  receipt key, both roles and their grants, and
  `memory_meta.schema_version`. It is idempotent. It refuses a
  knowledge store left in `public` from before stores were scoped
  (drop its tables or the database; the projection rebuilds from the
  record), and a schema a newer toolchain migrated.
- **The version fence.** The schema version is 5 (GH #1026: 2 made the
  lease table `claims` and the graph's projection one transaction per
  record row — migrating a version-1 memory renames the table in place,
  its rows kept; 3 adds the Ledger's gate, `ledger_append`, with the
  org chart it checks and the roles' principals; GH #1085: 4 adds the
  repository's graph, `graph_nodes`, `graph_edges` and `graph_members`;
  GH #946: 5 adds `hats`, hats by digest, which moves no projection.
  Each protocol move empties the graph once for
  the projectors to rebuild). A store's `open`
  selects the record's schema and refuses one at another version, or
  at none, naming both and `hale dna memory migrate` with the owner's
  DSN; it also refuses a schema whose claim names another record (`it
  is not read`). The host checks the version on the spine's DSN before
  it starts anything, and refuses to start (exit 2) on a mismatch.
- **Grants.** A head's role — the head's, or an owner's — has `SELECT`
  on the ledger, knowledge, org-chart and meta tables (`memory_meta`,
  `ledger_rows`, `ledger_meta`, `ledger_requests`, `claims`,
  `knowledge_*`, `graph_*`, `org_members`, `org_retired`), `INSERT` and `UPDATE`
  on `claims` only, to take and fence a claim, and `EXECUTE` on
  `receipt_file`, `receipt_read` and `ledger_append`: a head writes the
  Ledger through its gate and no table directly. The spine's role reads
  and writes its schema's tables (`SELECT`, `INSERT`, `UPDATE`,
  `DELETE`, `TRUNCATE`) except the owner-only `memory_keys`,
  `memory_principals`, `protected_receipts` and `protected_redactions`,
  and has `EXECUTE` on every function; it has no DDL. Every Ledger row
  but an adoption's copy goes through `ledger_append`.
- **Connections are bounded.** Each process holds one memory handle
  (`Memory`: the Ledger and the leases), opened on first use and
  closed when the process ends; everything in a process that reads the
  Ledger or a lease takes that handle rather than building its own, and
  each `Pq` store closes its connection when it dissolves. A host holds
  a fixed handful of connections however long it runs.

**The spine (GH #1026).** The spine is not a process: it is the program
any number of identical nodes run, and a node holds nothing — its state
is the stores, its code is the genome. On each tick (`HOST_TICK`, 1 s)
every host that runs a body applies the record into the graph and the
org chart (`MemoryProjection`), carries out an adoption or abandonment
the record asks for (`LedgerAdoption`, under a claim on the asking row),
and erases the protected bodies the record redacted
(`MemoryVault.complete_redactions`). Nothing coordinates the nodes; the
stores serialize them — the projection a row at a time by
compare-and-swap, an adoption by its claim, an erasure by digest (it
is idempotent). There is no spine lease and nothing is admitted: a
writer writes, and memory's insert function is the gate (**Writers
write directly**, above). **Sync before project**: when the record has
a remote, a node projects only as far as the head the remote holds
after that tick's sync, and not at all on a tick whose sync did not
complete, so a stamped row is always a shared one — a node that
projected a row it never shared and then died would leave every other
node waiting on a commit none of them can receive. With no remote there
is one clone, and nothing to wait on. A host with no
`HALE_DNA_MEMORY_DSN_SPINE` says so and runs with no memory: nothing is
projected. What failed on a tick is said once, when it changes, and
tried again on the next.

**Claims by id (GH #1026).** Every non-idempotent act a node takes is
claimed by id, with an expiry, in memory before it is taken, so any
number of nodes may run one organism over one record with no
coordinator. The table is `claims(key, holder, token, until,
present)`; the body lease is one of its rows. `MemoryLeases.claim(key,
holder, ttl)` takes a key that is free, expired or already this
holder's, with one conditional write — of two holders racing for a key
one lands and the other finds it taken — and answers with the claim as
it stands. Expiry is the database's clock, never a node's: the write
computes `until` from Postgres's `now()` and compares against it, so
nodes whose clocks disagree agree on when a claim ends. A renewal by
its holder keeps the token; any other take raises it.
`release_claim(key, holder)` gives it back on completion, its token
kept so a late renewal is refused. A holder that dies leaves
its claim to expire and another finishes the act. Without memory (no
DSN) there is no one to race and a claim is granted. The substrate
names its node by `HALE_DNA_NODE` (the host sets it to the body's
holder, `user@host:<clone>`; otherwise this machine and process) and
claims before it spends: an ask is claimed as `plan/<intent>`, and a
human case the leader is asked to plan as `plan/case:<case>`, for
`claim_ttl` (300 s) before the leader is asked, and, once
claimed, the record is read again, so an ask the claim's previous
holder admitted meanwhile is answered with that admission and the
claim given back; a node that finds the claim taken answers `planning
elsewhere: <holder> holds plan/<intent>` and writes nothing, and the
ask comes back to it with the host's relay until it is answered or the
claim expires (a case is held and driven again by `redrive`; memory
that cannot be reached is said as such, `not planned: memory could not
be reached to claim …`). The claim is given back once the plan's
admission is recorded (the case handed). The optimize pass claims its window, `optimize/<n>` for the
cadence's length, and leaves it to expire. A claim a node acts on is a
`claim.taken <key> {holder, token, until}` row and its return a
`claim.released <key> {holder}` row (the Ledger's), so the record
shows who took what, and a second node's take after the first one's
expiry; a claim taken without memory is not recorded.

**The genome pull (GH #1026).** Nodes converge on the genome by
pulling; there is no push deploy for the organism's own nodes (the
heart's deploy is another thing). At start a host fetches the forge's
default branch (the record's remote, its `HEAD`), moves its genome
forward to it when it descends from what is checked out, and checks and
builds it — the application's artifact, the organization and, under
`dev`, the application, each through the build cache. A genome that does
not check or build is a `node.build_failed <holder> {sha, why, by,
owner}` row; the node returns its checkout to the last genome that built
(`.hale/dna/genome.built`), builds that, and serves it. A node that is up
records `node.started <holder> {sha, by, owner}`. On its tick, every
`HALE_DNA_GENOME_POLL` seconds (300 by default; 0 never), it fetches
again, into `refs/dna/remote/genome` (never `FETCH_HEAD`, which any
other fetch in the clone rewrites); when the forge's head is neither what it runs nor a sha that did
not build here, it stops cleanly — the organization and the expression
drain on SIGTERM, the body lease is given back (`body.released {why:
"genome"}`) — and exits with `NODE_RESTART` (75); its unit
(`Restart=always`) starts it again, and it builds the new genome at
start. It never replaces itself in place. A forge head that did not build
here is said once and waited out. Each poll writes
`.hale/dna/genome.json`: `sha`, `failed`, `polls`. `hale dna status`
carries a `genome:` line: the sha each node runs, from its last
`node.started`, and a newer one that did not build since.

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
  position in the record, never a promise made before the append. A
  lost race is not a failure: the append goes round with the new tail
  until it lands, within a bound only a wedged repository reaches.
- **Authorship is git's.** The organism's own events carry its
  configured author; a human's facts (a verdict, an intent through the
  CLI, a host's crash accounting) carry the git identity of whoever
  ran the command. Commit signing, where the repository requires it,
  applies unchanged.
- **Receipts** are blobs under `refs/dna/receipts/<sha256>`, filed by
  the digest of their content: a verification step's output, a diff
  document. Events name receipts by digest. A receipt has a data class
  (GH #606): a `public` or `internal` body is such a blob; a `customer`
  or `confidential` body (`protected_class`) never is. It is kept in
  memory alone, sealed there — by the organism through
  `Dna.file_evidence(text, class, by)`, by a head through `hale dna
  receipt file --class`, both through `MemoryVault` — and the record
  gets `receipt.classified <digest> {class, by, store}`. **Memory seals
  it, and no process holds the key (GH #985).** The receipt key lives
  in memory, in `memory_keys`, written once at migration from the
  owner's `HALE_DNA_RECEIPT_KEY` (sixteen characters at least); a
  different key at a later migration is refused, because bodies sealed
  under the first would no longer open. Three `SECURITY DEFINER`
  functions, run as the schema's owner, are the whole surface of the
  sealed table `protected_receipts`: `receipt_file(digest, class, body,
  at)` seals a body with pgcrypto (OpenPGP, AES-256) under the key, and
  refuses a class that is not protected, a digest that is not the
  body's, and a digest memory erased under a redaction, whoever files
  it; `receipt_read(digest)` opens one; `receipt_erase(digest, at)`
  deletes the body and keeps its digest in `protected_redactions`.
  Heads and the spine may execute `receipt_file` and `receipt_read`;
  only the spine may execute `receipt_erase`. Memory with no receipt
  key keeps no protected evidence and says so. **A dump of the database
  carries the key (`memory_keys`) and the ciphertext together, so a
  dump is as sensitive as the evidence itself**; #989 revisits where
  the key is held. A query that fails drops the connection, dials
  again and tries once more, so a database restart does not leave
  every protected read and erase failing (#637). Without memory, the
  body is withheld: `receipt.withheld <digest> {class, by, why}`, and
  nothing holds it. The record keeps the digest and the class either
  way, so `sync` never carries a protected body into a clone. Reading
  one is an act in the reader's name for a purpose (`hale dna receipt
  show <digest> --purpose <p> [--as <who>]`): it is refused unless a
  `receipt.disclosed <digest> {recipient, purpose, by}` row names both,
  a refusal appends `receipt.read_refused`, and an answer is given only
  once its `receipt.read` row is in the record — a read the record
  cannot hold discloses nothing. A body kept while the record refuses
  its `receipt.classified` row is reported so, to be filed again.
  `Dna.evidence_class(digest)` is the class a request carries when it
  puts the body in a prompt, so a hosted model refuses it. This is the
  third trust profile #606 names: the readers of a clone hold digests
  and classes, never protected bodies. Under local trust a reader's
  name is attribution, as `--as` is; a verified principal is #612's.
  **Retention (part 2).** `receipt.held <digest> {by, why}` stands
  until `receipt.hold_released`, and refuses redaction while it stands.
  A redaction (`Dna.redact_evidence(digest, by, why, policy)`, `hale
  dna receipt redact <digest> --why --policy`) appends
  `receipt.redacted <digest> {by, why, policy, class, store}` and then
  removes the body: a git receipt's ref is deleted (`Receipts.erase`);
  a protected body is erased from memory through `receipt_erase`, by
  the spine — a head's redaction of a body memory keeps records
  `receipt.redacted` (`store: memory`) and the spine erases the body on
  its tick, as it erases every body the record redacted and memory
  still holds; a withheld one had no body. **The redaction is in the
  record before a byte is erased**, appended exactly at the revision the
  hold was read at: a redaction the record refuses erases nothing, a
  hold that arrived in between refuses it, and a redaction recorded
  before an erase that failed is completed on a later tick or by
  redacting again. A body the record says was redacted is not filed
  again — `Dna.file_evidence` answers its digest and keeps nothing, and
  `hale dna receipt file` and `receipt_file` refuse it — and `hale dna
  receipt redact` appends a git receipt's redaction only at the head it
  read the hold at and, once the organism has adopted the ledger, only
  at the ledger revision it read the hold at (the request's
  `expected`), refusing "the record moved" otherwise (#636). A store
  that cannot read the body (its read failed, as opposed to finding
  none) records nothing and reports nothing erased: redact again once
  it answers (`protected_erase_step`). The record keeps the digest, so
  provenance survives and a reader learns the body is gone — a read of
  a redacted digest is refused `… was redacted: by … under …`, and
  `hale dna history` says so under a redacted prompt. Every `sync`
  deletes redacted receipts' refs from the clone and the remote, so a
  clone that fetched one drops it at its next sync; the blob stays in
  each object store until git collects it (`git gc --prune=now`), and a
  copy disclosed or cloned outside the record cannot be recalled. The
  projection retires an idea whose receipt was redacted instead of
  stopping.
- **Leases** are blobs under `refs/dna/lease/<key>` (`:` in a key
  becomes `/`), `holder`, `token`, `expires`, `present` on four lines,
  compare-and-swapped on the ref. Tokens are monotonic per key.
- **Effects are requested before they run, and the claim is exclusive
  (GH #604 rule 3).** `effect.requested <key>` is appended before a
  dispatch and `effect.result <key>` once after; a retry finds the
  request and does not dispatch again. Two writers may both append a
  request for one key; only the writer whose row is the first request
  dispatches, the other's row stays as a contended claim and it does
  not act: every gate (`worktree.open`, `commit`, `apply`, `rollback`)
  refuses `in flight` while another request holds the claim with no
  result yet (#635). A request with no result when the organism restarts is
  resolved by evidence where evidence exists (an `apply:<candidate>`
  whose genome is at the candidate is `ok`; a `worktree.open` with no
  worktree is `failed`) and marked `unknown` where it does not; the
  gate refuses an unknown effect until a person resolves it —
  `hale dna effect resolve <key> --outcome ok|failed`, one row in
  their name, the last result for the key being its status.
- **Sync.** `hale dna sync` (and the host, every tick) fetches the
  remote's record into `refs/dna/remote/journal`, reconciles, and
  pushes. Local ahead: push. Remote ahead: fast-forward. Diverged: the
  local-only events are re-appended on top of the remote's head, bodies
  and authors unchanged, `seq` their new position, then pushed; a push
  the remote refuses is fetched and reconciled again. **A fetch is a
  read** (GH #961, #1072): two at once in one clone — the host's tick
  and a verb beside it — race on git's lock of the ref they update, and
  the one that loses is tried again, up to five times, waiting 100 ms
  more before each (the other already moved the ref at least as far).
  A fetch that still fails is a read that did not happen and refuses
  the sync, the connect or the handoff with git's reason; only git's
  own "couldn't find remote ref" is a remote that holds nothing under
  the family. **The reconciled
  chain is built beside the ref and swapped in with one
  compare-and-swap**: the ref never points at a record missing a row
  it held a moment before, so a reader's position only grows, a writer
  that reloads after a lost race finds its own rows, and a clone that
  appended while the chain was built is not overwritten — the swap
  fails and the reconcile goes round. A local-only event with no row
  refuses the reconcile rather than being skipped. A fact's identity
  in the record is its content and its place among rows of the same
  content, never its `seq` (a reconcile renumbers, and a node raising
  one concern three times writes three identical rows, each a fact):
  a host relays and handles by that identity, so a renumbered fact is
  not relayed twice and a repeated one is not relayed once. **A reconcile
  rebuilds only what this clone may sign (GH #639).** Under
  `dna.trust = signed` every local-only row is read before any chain
  moves: a row this clone signed is rebuilt and re-signed, and its
  commit names the original (`Rebuilt-From: <commit>` in the message's
  body); a row never signed is rebuilt unsigned, so a reconcile upgrades
  nobody's provenance; a row signed with another key refuses the whole
  reconcile — the row and the key it was signed with are named, every
  local row and commit stays where it is, the remote is untouched, and
  the refusal says what keeps the work (that row's writer syncs first,
  since a writer rebuilds its own rows; or the remote is fast-forwarded
  once it holds the row). A fast-forward is never refused. Under `local`
  trust nothing is read and every row is rebuilt as before. **A remote with
  no record yet is the local one's push**, not "up to date": the first
  sync after an ordinary `git push origin main` carries the record, so
  no one has to push `refs/dna/*` by hand for a second clone to have
  the organization's history. Receipts and candidates travel
  by refspec both ways; mailboxes are received. The remote is `dna.remote` in git config, or
  `origin`. A plain clone has no record until it syncs.
- **Candidates are kept.** The gateway points `refs/dna/candidates/<mutation>`
  at a candidate when it commits it, and sync carries the pointers with
  the record, so a refused or revised change stays readable as a diff
  after its worktree is gone: `hale dna candidates` lists them with
  their Review's state, `hale dna candidates <mutation>` shows one from
  its Review's base, and `hale dna candidates drop <mutation> --why <why>`
  appends `candidate.dropped <mutation> {by, why}` and stops keeping it —
  here, at the remote, and at every clone on its next sync, the way a
  redaction removes a receipt. The commits stay in each object store
  until git collects them.
- **An apply expresses the candidate, tree and all.** An approval
  applies exactly the reviewed candidate, or nothing: the candidate's
  worktree head is the pinned digest, the genome's head is the base the
  review was against, and the genome has nothing uncommitted — tracked
  changes anywhere, and **every input of the candidate's build that
  the tree does not hold**. The graph is the candidate's, read by the
  compiler in the candidate's worktree (`hale inputs <seed>`: the
  seed's `.hl` files and those of every directory they import,
  transitively; the `hale.toml` of each such directory and every C
  source it declares under `[ffi]`), because a reviewed change that
  adds an import makes files an input the moment it lands; any file
  of those kinds the genome holds untracked in one of the graph's
  directories, or that the graph names outright, is dirt whether git
  ignores it or quotes it. What no build reads — a binary, a log,
  another program's source — is not dirt. A graph that cannot be
  inspected is dirt: an inspection that failed authorises nothing. A
  dirty genome is `mutation.refused` before any effect row exists,
  the work untouched; the gateway checks it once more around the git
  call and reads the head back, so an apply that reports success is
  the candidate. **Approving again** an approved and unapplied
  candidate re-runs the whole gate (`mutation.apply_retried`, then
  `review.settled` again so a waiting `hale dna review` hears it):
  the Review readmits only the same approval in full — digest,
  authority, independence, the word `approve` — and refuses any other
  verdict; a Review approved and never applied is rehydrated settled,
  so the road survives a restart.
- **A person's job (GH #596 W; GH #604 rules 4 and 5).** A plan of kind
  `person` hands the Task on: the Work is done as far as the organism
  is concerned and the record keeps `task.handed <task>` — `work`,
  `assignee` (the person the plan named), `by`, `narrative`; nothing
  is mutated. The person reports it done with `hale dna task done <id>
  [--as <who>] [--note …]`, a `task.done` row in their name and nothing
  else. **Completion is checked at admission:** a handed Task with an
  assignee is closed only in the assignee's name; anyone else is
  refused and told to reassign. `hale dna task reassign <id> --to <who>`
  appends `task.reassigned` (`from`, `to`, `by`) and the Task stays
  handed. `hale dna retire <who> [--to <successor>]` stops new work
  reaching a person: every handed Task they hold is transferred as its
  own `task.reassigned` row and `person.retired <who>` records it (`by`, `to`, `transferred`). From then
  no new work reaches them: a job the leader plans for them is handed
  to that successor, with `retired_assignee` on the `task.handed` row —
  followed on through each successor who retired in turn to the first
  person still working, unassigned when the chain ends in a retirement
  that named none or comes back on itself — and a Task is never
  reassigned to them, nor are they named anyone's successor;
  refused while they hold work and no successor is named — pending
  work is accounted for, never dropped. Refused for a Task that is not
  handed: one the organism is working, or has settled, is not a
  person's to close. **A completion meets its acceptance condition.**
  When the plan names the obligation a person's job discharges, the
  organism binds the acceptance practice in force at hand-off:
  `task.handed` carries `acceptance` (the digest of the practice
  `acceptance/<obligation>` that is ratified and not retired, "" when
  none) and `evidence_required` (its text says `evidence: required`). A
  later change of practice never changes a case already handed. Such a
  Task closes only with `--evidence <digest>`, a receipt the record
  holds, appended before `task.done` as `completion.linked` (`task`,
  `evidence`, `by`, `practice`), or with `--exception <why>
  --authorized-by <who>`, appended as `completion.excepted` (`task`,
  `why`, `authorized_by`, `by`, `practice`) — only when `<who>` has
  authorized it in their own name first: `hale dna task authorize <id>
  --exception <why> [--as <who>]` appends `exception.authorized <task>
  {task, why, by, practice}`, refused for the Task's assignee and, when
  the bound practice names who may (`exceptions by: <names>`), for anyone
  else, and refused outright when the bound practice cannot be read here
  (absent from the clone, unreadable, redacted) — an unreadable practice
  is not one that names no one; naming someone is never an authorization.
  The completion carries the exception exactly as it was authorized under
  the bound practice: a different `--exception` is refused, and needs an
  authorization of its own. A note alone is refused, and so is an exception
  without an authorizer, a self-authorized one, or both at once. Either
  may accompany any person's completion. `task.done` keeps its body; the
  projection shows a done Task's evidence or exception, and a person's
  completion with neither as human-reported, because a note never
  verifies anything.
- **A person proposes a practice (GH #602).** `hale dna practice propose
  <name> --text <text> [--because <what prompted it>] [--supersedes
  <digest>] [--as <who>]` carries a `PracticeRequest` (`request_id`,
  `name`, `text`, `by`, `because`, `supersedes`) as a
  `practice.requested` row, which a node relays onto the topic
  `dna.practice.requested` (**The nerves**). The organization proposes
  it as knowledge of kind `practice`, author `org`, bound to `org`, with
  the ordinary Board Review `k:<digest>` (decided, it is ratified by an
  execution of `practice-ratify`: GH #995, **The workflow catalog**), and
  journals
  `practice.proposed <request_id>` (`name`, `digest`, `review_id`, `by`,
  `because`, `supersedes`); a request without a name or a text is
  `practice.refused`. Nothing is in force until the Board ratifies it.
  **Roles are authorities, not identities.** The person who proposed
  a practice may ratify it: the proposal is in their name (`by`), the
  verdict carries the Board's authority, and the record keeps both, so
  one person holding several roles — the Board, and the hands doing
  the work the practice describes — proposes as one and decides as
  the other. The Review's independence rule refuses the candidate's
  *author*, which for a practice is the organization (`org`), never
  the proposer. This is the design, not an artifact; an organization
  that wants the proposer barred from ratifying their own proposal
  needs a review policy that says so, and none ships.
  The text arrives byte for byte, newlines included: the host escapes an
  argument's own backslashes and newlines in its argument list and
  unescapes every value it reads.
  `hale dna practice` lists every named practice with its state (in
  force, awaiting the Board, declined, retired), its first line, who
  proposed it and why, and requests not yet heard.
- **Cross-record handoff (GH #615).** A replica of one record is inside
  the horizon: sync carries all of it, to equally trusted readers. A
  *handoff* crosses a horizon into a separate record, and a connection is
  its only path; nothing selective leaves a record by sync. `hale dna
  connect <record-url> --name <name> --as <position> --purpose <purpose>
  --classes <internal,customer,confidential> [--by <who>]` (the url is
  the other record's git remote; an `http://` or `https://` url is
  refused as a service url) reads the other record's identity — its genesis, the root commit of its journal,
  which every record publishes as a blob under `refs/dna/identity` when it
  syncs (until a push of it to the remote as it is now has succeeded —
  `dna.identitypublished` names that remote and blob — each sync asks the
  remote and pushes it, never forced, and `sync` says when it could not)
  — and nothing else of it; it refuses this record's own, and appends `connection.proposed` (`name`, `url`, `peer`,
  `position`, `purpose`, `classes`, `by`, `review_id`) with a Board
  Review `c-<name>-<n>`. The connection is in force once a `board`
  verdict approves that Review from someone other than its proposer —
  the host then settles the Review in the approver's name — and until
  `hale dna disconnect <name> --why <why>` appends `connection.closed`.
  `hale dna handoff <name> task <id> | receipt <digest> [--note …] [--as
  <who>]` writes one `handoff.received` envelope into the other
  record's **mailbox for this record**, `refs/dna/exchange/<this record's
  identity>` — a chain of rows only this record writes, pushed there under
  compare-and-swap. Neither record reads the other's journal: this clone's
  cache for a connection (`.hale/dna/peers/<name>.git`) holds the other
  record's identity and this record's own mailbox there, nothing else. The
  envelope carries `handoff` (`h` and twelve hex
  digits of the origin genesis, kind and subject, so a fact crosses
  once), `origin_record`, `origin_url`, `origin_author`, `origin_row`
  (the source row's digest), `lineage` (the subject's rows here, as
  `kind#seq`), `purpose`, `position`, `via`, `kind`, `subject`, `class`,
  `fact`, `note`. This record then appends `handoff.published` (with
  `peer_row`, the commit it landed as) and, for a Task,
  `task.transfer_requested`. A handoff retried after a crash completes
  what the first try left (#637): an envelope already in the mailbox is
  reused, never written twice, and a published Task missing its
  `task.transfer_requested` gets it. Only a handed Task crosses. A receipt
  crosses as its digest and class; its body never leaves this record. A
  fact whose class the connection does not carry is refused at the edge
  as `handoff.refused`, with nothing written across. The receiving record
  reads its own mailboxes (`sync` fetches `refs/dna/exchange/*` from its
  remote; a row claiming another origin than the mailbox it is in is
  ignored) and admits what arrives under its own policy: `hale dna
  handoff` lists a received handoff as admitted only under a connection in
  force back to its origin record that carries its class, and `hale dna
  handoff accept <id> [--as <who>]`, only for an admitted one, appends
  `handoff.accepted` to its own journal (with the envelope's origin,
  lineage, purpose and fact) and sends a `handoff.accepted` envelope into
  the origin's mailbox for it. `hale dna handoff sync` reads, for every
  connection in force, the envelopes the other record sent to this
  record's mailbox: each acceptance of a handoff published through it is
  admitted once, as
  `handoff.accepted_by_peer` and, for a Task, `task.transfer_accepted`,
  each row checked on its own, so a crash between the two is completed at
  the next sync rather than suppressing it (#637).
  The Task settles only then, the rule retirement follows (GH #604 rule
  5). A closed connection is not read, so history stays in both records
  and nothing further is admitted.
- **An attributed external decision (GH #616).** A decision made by
  someone who does not run Hale — a manager, a client, an accountant —
  enters as a fact reported by a position inside the horizon, never as
  that party's verdict. `hale dna task decide <id> --decided-by <party>
  --via <channel> --evidence <digest> [--note …] [--as <reporter>]`
  appends one `decision.reported` row on the Task in the reporter's
  name: `reporter`, `decider`, `channel`, `evidence`, `scope` (the
  Task), `obligation`, `practice`, `policy` (the practice's digest),
  `accepted`, `why`, `note`. It is refused with nothing appended when
  the Task is not handed, when the reporter is not its assignee (GH #604
  rule 4), when the decider is the reporter, when a field is missing, or
  when the evidence is not a receipt the record holds. `hale dna receipt
  file <path> [--class internal|customer|confidential]` files one:
  internal text under `refs/dna/receipts/` with a `receipt.filed` row
  (`by`, `name`, `bytes`, `class`, `store`), a protected class sealed
  in memory alone (`receipt.classified`). Whether a report settles the Task is the
  obligation's **acceptance policy**: a practice named
  `acceptance/<obligation>`, ratified by the Board and not retired,
  whose text says `reported decisions: allowed`. The obligation is the
  class of obligation a person's job discharges; the leader's plan names
  it (`obligation:`) and `task.handed` carries it. The policy is the practice
  bound when the Task was handed (`task.handed.acceptance`), so a later
  change of practice changes no case already handed — only a Task handed
  before practices were bound reads the one in force; it is read at
  admission and its answer kept in the row, so a later change of
  practice never rewrites what was admitted. Accepted, the Task is
  `decided`, and the projection shows the report as such: who reported,
  who decided, through what, on what evidence. Otherwise the Task stays
  handed, `hale dna board` lists it under *tasks waiting* with the
  reason, and its assignee closes it with `hale dna task done`. Without
  an obligation class or a practice in force, a report never suffices.
- **Schedules (GH #610).** An ask fired on an interval or a cron, in
  the organism's own name, taking the ordinary road. The org chart
  declares them in its `birth()` — `self.core.schedule(Schedule {
  id, every_ms | cron, ask, requires })` — and the substrate's clock
  (the org program's loop, `request_tick` with a millisecond monotonic
  clock) queues their processing through the organization's keyed
  `TickRequested` handler. Synchronous `tick` is for that handler or
  isolated callers with no incoming bus work; a live loop uses
  `request_tick` so incoming work cannot enter during journal refresh.
  A declaration is a row, `schedule.declared <id>
  {action, every_ms, cron, ask, requires}`, when new or changed; a
  malformed cron (five fields, minute hour day-of-month month
  day-of-week; `*`, `a`, `a-b`, `*/n`, lists; ranges checked) is
  refused at declaration, never when it would first fire, with a
  `schedule.refused` row. An interval counts from the first tick and
  fires once per interval; a cron fires once in the UTC minute it
  names (day-of-month and day-of-week both restricted: either). A fire
  is **claimed in the record before it is admitted**: `schedule.fired
  <id> {intent, at}` first — a claim the record refuses admits nothing
  — then `ask` with `Intent { id: "s:<id>@<at>/<n>", from:
  "schedule:<id>" }`, routed as `requires` says, then
  `schedule.admitted <id> {intent, task, at}`; a refusal by the
  membrane is `schedule.refused`. A declaration after a restart
  restores the last fire from the record — its time, and for a cron
  the minute, so a cron does not fire again in the minute it fired —
  and a claimed occurrence without its admission: the Task born under
  its intent completes the admission (`recovered: true`), and one never
  born is admitted at the next tick, once. The optimize pass's fire is
  likewise a `schedule.fired {action: optimize, at}` row written before
  the pass runs. **Overlap:** a
  schedule never fires while the last Task it fired is open (born,
  pending, handed — anything but done or failed); the skip is a
  `schedule.skipped <id> {task, state, at}` row, never silent. `hale
  dna schedule pause <id>` / `resume <id>` append `schedule.paused` /
  `schedule.resumed` in your name from any clone; the organism reads
  them at its tick (a git journal is re-read from its ref at most
  every 5s) and a paused schedule never fires. `hale dna schedule`
  lists them as the record has them. At birth a schedule's state —
  its last Task, paused or not — is read back from the record. The
  optimize pass is a schedule: `optimize_every_ms` declares
  `optimize` (`action: "optimize"`) at birth and each run is a
  `schedule.fired optimize` row.
- **The optimize pass (GH #596 O).** On a cadence the org chart sets
  (`optimize_every_ms` on the substrate; 0 is never; the org program's
  loop calls `request_tick` with a millisecond monotonic clock), the substrate
  first asks the budget — the pass is model-backed work, and none is
  routed on an exhausted window: `optimize.refused <org>`, and the
  pass waits for the next window — then claims the window
  (`optimize/<n>`, the cadence's own length; **Claims by id**, below),
  so of the nodes running the organism one runs the pass, as an
  execution of `optimize-walk` (GH #995: **The workflow catalog**) whose
  one step, `walk`, reads the record's structural signals
  — asks planned and how many took the defaults, concerns raised,
  grant contractions, verdicts refused, mutations rolled back — and
  asks the leader (`OptimizeRequested`, keyed by `org_id`) to walk the
  machinery, not the work. The leader answers with one small proposal
  or none (`OrgReviewed`), and the substrate journals `org.reviewed
  <org>` either way, with the signals it read and the execution it
  answers (`task`, carried on `OptimizeRequested` and `OrgReviewed`, so
  passes that overlap never answer each other), which settles its `walk`
  step: "if the state is clean, say so". A proposal enters as an ask in the leader's name and takes
  the whole road — planned, proposed, reviewed by the Board as an
  organization change. `hale dna` runs the pass on demand through the
  substrate's `optimize()`.
- **Grants layer by containment (GH #596).** The substrate may hold a
  `ceiling`: the grant above the child's — the organism's, the Board's
  to widen. A child's boundary reads its grant through the ceiling **as
  it is now** at every assessment: the classes both allow, the smaller
  magnitude, the stricter review (`intersect_grants`). Grants contract
  on their own (two failures in a row halve the ceiling), so a child
  born within a ceiling of 100 is bound by 50 the moment its parent
  contracts, and the record says so (`grant.contracted <parent>`
  `{boundary, from, to, epoch, binds, note}`, appended before the
  contraction takes effect). An organism born over the record restores
  the last contraction of its ceiling — its epoch and `pre` review
  whatever the authored ceiling now says, and the stricter of the
  recorded and authored magnitudes — before anything is admitted, as it
  restores a revocation. A contraction whose row the record refuses
  still binds live, and is appended again before every `reserve` and
  `admits`; until the record holds it, nothing is admitted. A child born wider than its
  ceiling is the early error: `grant.refused <child>` names what is
  wider, and the ceiling binds it from birth; that row is authority,
  the record's, and is not the row a refused spend writes. Law layers by adoption:
  every position lives under the org program's main, which adopts the
  organism's law, so a department's law can only add to it.
- **Grants delegate resources (GH #605).** A `Grant` also carries
  `amounts` (per-operation ceilings, `"USD 500.00 EUR 200"`; a currency
  not named may not be spent), `window_amounts` and `window` (`day`,
  `week` or `none`), `counterparties` and `routes` (space-separated;
  none named, none allowed), `expires` (unix seconds; 0 never) and an
  `authority_epoch`. Every field defaults to none, so a grant written
  before them spends nothing. They contain like the rest: the
  currencies both grant at the smaller ceilings, the smaller window
  limit, the parties and routes both name, the earlier expiry (never is
  later than any, so a child that never expires under a ceiling that does
  is wider in time), and the
  sum of the two epochs (`intersect_grants`); `resource_gap` names what
  is wider, and the birth refusal carries it. `permits(grant, spend,
  now)` checks a `Spend { id, amount, currency, counterparty, route }`
  predicate by predicate and names the field that fails (`expires`,
  `operation`, `currency`, `amount`, `counterparty`, `route`).
  `Dna.reserve(spend)` admits a spend against the effective grant and
  appends `spend.reserved <op> {child, amount, currency, counterparty,
  route, ceiling, epoch, at, funder, account}` (GH #668: the allocation
  is the row's entity, a claim kind the Ledger's unique constraint
  reserves once, and the row names who pays — a grant's `funder`,
  `<owner>/<account>`, inherited from the ceiling when the child names
  none; a child naming another funder than its ceiling's is born wider,
  `funder … not granted above`); a reservation from before this,
  `grant.reserved <child> {op, …}`, is read the same way. The child's
  own window counts its reservations and the ceiling's window counts
  every child's under it.
  The remainder is read at a revision and the row is appended with
  `Journal.append_exact` at that revision — a writer that moved the
  record in between makes it stale, never re-appended at the tail — so
  two children cannot each spend the same remainder. A refusal is
  `grant.reservation_refused <child>` naming the field — money, the
  Ledger's, never `grant.refused`. `Dna.settle_spend(op, spent)`
  appends `spend.settled <op> {child, attempt, spent}`, one per
  attempt: every attempt's actual consumption is retained and the
  window counts their sum from then on (a second settlement is attempt
  2, not a refusal). Under contention a settlement is once: the row is
  appended with `append_exact` at the revision the last settlement was
  read at, and one another writer landed in between is read as the
  answer, never doubled. Money that came back is a compensation someone
  authorized, `Dna.compensate_spend(op, amount, by)` appending
  `spend.compensated <op> {child, attempt, amount, by}` — never an
  implied rollback. A purchase two children (or two owners of a shared
  record) fund is two reservations, each its own: one may settle and
  the other not, and the partial success is visible as such; no
  atomicity across them is assumed. A contraction advances the
  epoch, and so does `Dna.revoke_grant(by)` (the parent's only; nothing
  is left granted). A revocation is **recorded before it takes effect**,
  `grant.revoked <child> {by, parent, epoch}` — one the record refuses
  did not happen — and an organism born over the record restores it
  before anything is admitted, so a restart never restores revoked
  authority. `Dna.admits(admission)` refuses an admission made under an
  older epoch, or whose grant has expired by the time it is carried out
  (`grant.fenced`). The generated
  law adds `money_only_through_the_substrate: forbid reaches(positions,
  effects(money)) avoiding substrate`, with `effect money;` declared in
  the core; an organization's existing `dna/org/law.hl` is
  project-owned and gains the clause by hand. The money budget is
  distinct from the model budget.
- **The principal source (GH #612).** Who acts where a verdict, an
  intent or a task completion enters comes from one of two sources,
  `git config dna.principal`: `local` (the default) — the operator of
  the clone, as `--as` and the git identity say — or `oidc` — an
  identity provider. Under `oidc`, `hale dna ui` is a hosted head: it
  serves nothing without a session (`/api/*` answers 401, `/`
  redirects to `/auth/login`). A session comes from OpenID Connect's
  authorization-code flow against `dna.oidc.issuer` (https, or plain
  http to `127.0.0.1` or `localhost` exactly — the URL's host is parsed,
  userinfo refused, never matched by prefix): the head discovers the
  issuer's endpoints, refuses a discovery document whose
  `authorization_endpoint` or `token_endpoint` is not https (plain http
  only when the issuer itself is on this machine; `discovery_refusal`,
  #635), sends the browser to its authorize endpoint with a random
  `state` and `nonce` (`std::os::getrandom`), and exchanges the
  returned code at the token endpoint itself with the client's secret
  (`dna.oidc.client`, `HALE_DNA_OIDC_SECRET` read into a sealed locus,
  `dna.oidc.redirect` as the callback). That ID token came over TLS from
  the issuer's own endpoint, which authenticates it (OpenID Connect Core
  §3.1.3.7, rule 6) in place of an RS256 signature the standard library
  cannot verify; `claims_refusal` checks the issuer, the client among the
  audience, the expiry and the nonce. The subject — never an email alone
  — maps to a member through a reviewed mapping, `git config --add
  dna.oidc.member "<subject>=<name>"`; an unmapped subject gets no
  session. A sign-in's state is used once, expires in ten minutes, and is bound to
  the browser that started it by an `HttpOnly; SameSite=Lax` `dna_signin`
  cookie: a callback carrying the state from any other browser is refused
  and leaves the sign-in for the browser that started it (so a callback
  URL handed to someone else cannot sign them in); a
  session is a random 256-bit id in an `HttpOnly; SameSite=Lax` cookie
  (`Secure` when the callback is https) and lasts eight hours or until
  `/auth/logout`. With a session, a verdict acts as the member (`--as`,
  and `--authority board` when `dna.oidc.board` names them, `reviewer`
  otherwise) and an intent is asked by them (`hale dna task create --as`), the
  row's author too; the form's own `as` field is ignored. The head never
  acts in its own name. It speaks plain HTTP: TLS is a reverse proxy in
  front of it. Rows arriving by sync remain admitted under `dna.trust`
  (GH #604 rule 6); a synced row claiming a rank is not a sign-in.
- **The nerves: the record's requests reach the organism (GH #986).**
  A request is a row first, always. `hale dna task create` appends
  `intent.requested` (the body: outcome, from, to), a verdict appends
  `review.verdict` (the body: the verdict), `concern raise` appends
  `concern.requested`, `pressure raise` appends `pressure.requested`
  and `practice propose` appends `practice.requested`, each in the
  appender's git identity, from any clone. **A one-shot verb never
  publishes.** A **node** — the host under `hale dna run` or `dev`,
  which is its program's `main locus` for as long as it runs — relays
  every request row it holds that the record has not answered onto the
  nerves on its tick, admitting each under `dna.trust` (GH #604 rule 6),
  and again every 30 s while it stays unanswered. Beside a live
  organism a verb waits for the answer in the record; from a clone
  with none it syncs and says where the answer will be. **A
  request is answered by its own answer** (GH #689): an intent, a
  verdict's review and a practice request have an entity of their own,
  and a concern — whose entity is its source, shared by every concern
  from it — carries a `request` id that the organization writes into
  the `concern.raised` answering it. That row is one object — the concern's
  words, its severity, which occurrence it is, and the request it answers
  — so the id is a member of the row rather than a shape inside its text:
  a worker who writes `observed [request c1] in a log` has written text,
  not metadata, and the host compares the whole id, so an answer to `c10`
  is not an answer to `c1`. A row written before that
  rule carries no such field and is matched by counting, as this host did
  before. **One request is one concern**: a request delivered twice — the
  host relays a row again while it looks unanswered — writes one
  `concern.raised`, and the count that turns concerns into a knowledge
  proposal counts distinct requests, never redeliveries. Two requests
  raising the same words are two concerns; one id used for a second,
  different concern — other words, or the same words at another severity
  — is refused once, in `concern.refused`, which is that request's answer
  and carries its id. Pressure is answered the same way (GH #986): a
  `pressure.requested` row carries a `request` id, and the
  `pressure.raised` answering it is an object — the signal's words,
  `count` (which time its source has raised pressure) and `request`; a
  pressure row from before carries `<what> x<n>` and is read as that.
  **A publish is confirmed by its answer in the record, never by the
  transport** (GH #682): a row still unanswered 30 s after it was
  relayed is relayed again, whatever the transport said. The organism
  admits an intent once by its id — an intent offered again after its
  Task was born answers with that Task and journals nothing — a
  repeated verdict at a settled Review is answered as already settled,
  and a practice, a concern or a pressure request is answered once by
  its id. The host's own observation report is a row too:
  `observation.requested <mutation>` (`mutation_id`, `outcome`,
  `model_hash`, `detail`), relayed as `ExpressionObserved` until the
  organism answers it with `expression.observed`; a node inside its
  observation window relays while it waits for that answer. A refusal
  the host writes for an unverified row answers the request it
  refuses: for a request with an id of its own (a concern's, a
  pressure signal's) it is one object, `why` and `request`. `local`,
  the default, trusts every writer to the record — the operator's
  profile; `signed` admits a row only when its commit carries a
  signature git verifies (`git verify-commit`; git's keyring or
  allowed-signers file is the list of who may act), and a row that
  does not is refused in the record in the host's name
  (`intent.refused`, `review.refused`, `concern.refused`: "unverified
  writer") and never relayed. Under `signed`, rows this clone writes
  are signed commits. Signing is one mechanism for this edge, not the
  boundary itself: a hosted head is admitted through a reviewed mapping
  of principals to authority and acts as the user, never as itself.
  The organism's answers (`intent.offered`, `task.born`,
  `review.settled`, `review.refused`) return the same way. The body
  writes `intent.offered`, so the asker is not its author: the row's
  body names who asked — `<outcome> (from alice)`, a schedule, an
  optimizer — and a head's ask carries the person the head identified.
  A row is answered when a later row of the answering kind names its
  entity. `hale dna task create --no-wait` appends and returns, on either
  path: beside a live organism once the row is appended, from a clone
  with no organism once the row is appended and synced. "Beside a live
  organism" is the record's body lease: live, and held by this clone's
  body.

**The nerves (GH #986)** are NATS JetStream, through pond's client and
`std::bus` adapter, vendored under `dna/core/pond/realtime/nats`.

- **Bindings where the publishes are.** The host binds every DNA fact
  topic it publishes (`IntentOffered`, `ReviewVerdict`,
  `ExpressionObserved`, `PressureRaised`, `ConcernRaised`,
  `PracticeRequested`, `KnowledgeNodeRequested`,
  `KnowledgeBindingRequested`, `KnowledgeEdgeRequested`) to
  `nats::NatsAdapter`, and owns the connection, a `nats::NatsConn`
  placed `pinned`; its relay is a `NodeRelay` child whose publishes go
  out through those bindings. The generated organization binds the same
  topics and reads them through its own pinned connection. The payload
  is the bus's own encoding: both ends are Hale programs declaring the
  same topic, and nothing converts it. A one-shot verb instantiates no
  `main locus`, so it binds nothing and connects to nothing.
- **Subjects** are the topics' declared subjects under the
  organization's token — the one memory names its schema with
  (`dna_<record id>`): `<org>.dna.<family>.<event>`
  (`dna_4f….dna.intent.offered`); an application's own events will be
  `<org>.app.<app>.<event>` (#987), and what a node tells the heads is
  `<org>.head.<event>` (**Rows landing**). The connection puts the token on
  what leaves and takes it off what arrives, so a topic keeps its
  declared subject.
- **One stream per organization**, `DNA_<ID>` over `<org>.>`, on disk,
  kept a week (limits retention: what was said is kept and read from a
  position, not deleted once read). Every publish is acknowledged by the
  stream. The organization reads through its durable pull consumer
  `spine` (from the stream's start when it is first made, from where it
  left off after that), filtered to `<org>.dna.>`, so a fact published
  while it was down or restarting reaches it when it is back. **Over a
  shared record each owner is a space of its own:** every owner runs a
  body and an organization (GH #664), and an owner's host publishes
  under `<org>.<owner>.` and its organization reads through the durable
  `spine_<owner>`, filtered to `<org>.<owner>.dna.>` — so no
  organization pulls a fact another owner's host published, and refuses
  it, which the relay would take as the answer. The host names its
  owner from the clone (`dna.owner`) and hands it to the organization
  (`HALE_DNA_OWNER`); an owner's name in a subject or a durable keeps
  its letters, digits, `-` and `_`, anything else `_`.
- **Credentials, one per family**, each a URL with its user: `owner`
  creates the stream and is held by no process that runs the organism;
  `spine` publishes and reads `<org>.dna.>` and an owner's
  `<org>.<owner>.dna.>` (the host and the organization), and publishes
  `<org>.head.>` and an owner's `<org>.<owner>.head.>`; `head`
  subscribes and publishes nothing; `app`
  publishes on `<org>.app.>` (#987). `hale dna nerves migrate [dir]`
  creates or updates the stream with the owner's URL
  (`HALE_DNA_NATS_URL_OWNER`, or the `nerves` service of
  `dna/compose.yaml`, brought up) and prints `HALE_DNA_NATS_ORG` and
  one `HALE_DNA_NATS_URL_<ROLE>` per role; `hale dna dev` runs it
  before it starts the host and hands the host the spine's URL and the
  token, never the owner's nor the head's; `hale dna run` takes the
  two from its environment and has the owner's and the head's removed
  from it. `hale dna nerves drop [dir]` deletes the stream, and
  everything it held, with the owner's URL — beside dropping memory's
  schema, when an organization is torn down. `dna/nats.conf`, which
  `init` writes, is that server's configuration: the users and their
  permissions, with placeholder passwords until the vault (#989).
  `upgrade` writes it when it is missing, and says so when an older
  one's spine may not publish `<org>.head.>`.
- **Liveness.** At start the host waits, as it waited for sockets, for
  the organization to read its facts: its durable consumer has a pull
  outstanding. It says so (`the organization reads its facts from the
  nerves (DNA_<ID>)`) or that it did not within 20 s; the facts wait in
  the stream either way. A restarted or rolled-back organization is
  confirmed the same way. Without nerves (`HALE_DNA_NATS_URL_SPINE` or
  `HALE_DNA_NATS_ORG` unset) the host runs and says the organization
  hears nothing the record asks.
- **A lost connection.** The connection keeps every publish until the
  stream acknowledges it, writes it again after a reconnect, and a
  publish unacknowledged past its window violates the connection's
  `delivery` closure. The violation collapses it to its owner, which
  does not restart the connection in place: the node starts again as a
  fresh process, its connection and its organization's both new. The
  host appends `nerves.lost <holder>`
  (`why`, `by`), stops its organization and the expression, gives the
  body lease back (`body.released`, `why: nerves`) and exits
  `NODE_RESTART` (75, the genome pull's code: one restart code for the
  organism), and its unit (`Restart=always`, which restarts on any exit)
  starts it
  again. The organization's program exits 75 on its own connection's
  collapse, and the host that supervises it ends with it. The row first
  is what makes a loss recoverable: the next node relays every request
  still unanswered.
- **Rows landing.** For every row its view of the organism gains, a
  node publishes `head.row.landed` (`RowLanded`: the row's `seq`,
  `kind` and `entity`; under an owner's prefix over a shared record),
  at most 256 on one tick; its first look only takes its place. It is
  the heads' family, outside `dna.`, so no organization's durable
  pulls it; the stream keeps it like everything under `<org>.`, and
  the head's push is what it is for.
- **Stopping.** A node is its program's `main locus`, and SIGTERM
  drains it (GH #1039): its supervision reads `draining`, stops the
  organization (which drains the same way), gives the lease back and
  ends. `hale dna run` and `dev` give the node a 30 s grace for it
  (`LOTUS_DRAIN_GRACE_MS`, unless the environment already sets one).
  The core imports no NATS, because a program holding pond's
  connection drains on SIGTERM: an API that imports the core keeps the
  default action. The head holds its own connection and drains: its
  watcher stops and the server stops accepting, at once.
- **The fleet node's concerns stay local.** A node's instances are the
  application, which declares no bindings, and an environment route
  reaches Unix sockets only; `hale node` keeps the one
  `LOTUS_BUS_CONFIG` DNA writes, for `dna.concern.raised` from its own
  instances (**The application side**), and puts each concern into the
  record, which a node relays like any request. #987 replaces it.

`.hale/dna/` holds only what is not the record: the status projection,
worktrees, scratch inputs to the toolchain. Deleting it loses nothing
the record holds.

## Event kinds

Every kind names its memory. The **memory** column is the routing
table under version 1 (`memory_of` in `routing.hl`); on routing 0
every kind is the record's, and a kind no build knows is the
record's.

| kind | memory | what it is |
|---|---|---|
| `application.attached` | record | the application the organism oversees: its entrypoint, its artifact, the toolchain |
| `structure.observed` | record | the compiler's model of one of its parts, at `init` |
| `graph.node` | record | a node of the repository's graph, entity `<kind>:<name>` (**The repository's graph**) |
| `graph.edge` | record | a hyperedge of it, entity its id: kind, members `{role, node}` in order, `via`, `outside` |
| `graph.retired` | record | the node or edge the entity names leaves the graph |
| `hold.requested` | record | someone asks the organization to propose a holder for a position (`hale dna fill`) |
| `hold.proposed` / `hold.refused` | record | the organization proposed it to the Board (`digest`, `review_id`), or why not; `hold.refused <hold id>` is also memory's refusal of a hold it would not project (`why`, `row`, `by: memory`) |
| `responsibility.proposed` | record | a one-line responsibility inferred for a part, not yet ratified |
| `law.deferred` | record | a clause `init` could not certify |
| `intent.requested` | ledger | an ask from a clone with no organization running |
| `intent.offered` / `intent.refused` | ledger | the outcome an ask was admitted for, or the refusal |
| `intent.unrecovered` | ledger | an intent offered before a restart that no admission names; never re-offered |
| `review.verdict` | record | a verdict appended in the reviewer's name, from a clone or a forge |
| `task.born` | ledger | the work an intent or a settled review made |
| `task.handed` | ledger | handed to a person: assignee, obligation, acceptance |
| `case.admitted` | ledger | a human Work's case, its own handed Task (card 16): parent Task, Work and attempt, objective, assignee, obligation, acceptance, required evidence, its origin (`owner`), disposition (`to`) and performer (version 2, card 17) — before its `task.born` / `task.handed`, which say what it recorded; its completion (`task.done`, `decision.reported`) is the attempt's outcome (card 17) |
| `task.reassigned` | ledger | the assignment moved to someone else |
| `task.<state>` | ledger | a case's states, to `done` or `failed`; an admitted execution's root is settled by `workflow.settled`, its workflow's row (card 18) |
| `mutation.requested` | record | which exact Work and attempt asked for the Mutation, with the request as asked; written before `mutation.proposed` (card 14) |
| `mutation.proposed` | record | a change proposed for a Task: class, objective, target, base |
| `mutation.worktree` | record | the sandbox opened for it, and removed |
| `mutation.located` | record | the files found, and the grant they were found under |
| `mutation.candidate` | record | the commit the editor produced |
| `mutation.<disposition>` | record | what the autonomy boundary decided: `review`, `stage`, `escalate`, `release`, `deny` |
| `mutation.topology` | record | the diff names a plan or the manifest: re-classed for the Board |
| `mutation.applied` | record | the candidate applied to the genome |
| `mutation.apply_retried` | record | the apply ran again on a settled review |
| `mutation.retained` | record | kept after its observation window |
| `mutation.rolled_back` / `mutation.rejected` / `mutation.revise` | record | the genome back at the base, refused after review, or sent back for another pass |
| `mutation.refused` / `mutation.failed` | record | not applied (the candidate moved, the gate said no), or the change did not survive its own verification |
| `effect.requested` / `effect.result` | ledger | the exclusive claim on an effect key, and its outcome |
| `evidence.<step>` | record | a verification step's output, kept by the digest the row names |
| `evidence.magnitude` | record | the measured magnitude of the change |
| `review.requested` | record | the Review: question, authority, candidate, disposition, evidence, diffs |
| `review.settled` / `review.refused` | record | the verdict that decided it, or why one was not admitted |
| `review.reasoned` | record | the deciding verdict's comment — a person's note or the Leader's reasoning — right after `review.settled` (`hale dna review <id>` renders it as `why:`) |
| `candidate.dropped` | record | a kept candidate is no longer kept, here and at every clone's sync |
| `knowledge.proposed` / `.ratified` / `.declined` / `.refused` | record | a practice through its review |
| `knowledge.retired` | record | a practice superseded by a later version |
| `knowledge.consulted` | ledger | what a piece of work looked up today |
| `grant.contracted` | record | authority narrowed, and what it leaves |
| `grant.revoked` | record | the parent took the authority back |
| `grant.refused` | record | a grant born wider than its ceiling: authority |
| `grant.reservation_refused` | ledger | a spend the window would not admit, naming the field: money |
| `grant.reserved` / `grant.released` | ledger | a spend admitted against the window, and the reservation settled at what was spent |
| `grant.fenced` | ledger | an admission refused because the grant's epoch moved since |
| `budget.exhausted` | ledger | the window's model allowance is spent |
| `model.called` | ledger | a model call and its evidence |
| `optimize.refused` | ledger | the organization's pass over itself did not run |
| `node.started` / `node.build_failed` | record | a node runs a genome, by its sha; a genome did not check or build, and the node stayed on the last that did (`sha`, `why`) |
| `nerves.lost` | record | a node's connection to the nerves collapsed — a publish the stream did not acknowledge in its window (`why`, `by`); the node stops and exits 75 for its unit to start it again, and the next node relays every unanswered request again (GH #986) |
| `observation.requested` / `observation.refused` | record | the host's observation report as a row (`mutation_id`, `outcome`, `model_hash`, `detail`), relayed until `expression.observed` answers it; refused, unrelayed, when the row is not verified under `signed` trust (GH #986) |
| `claim.taken` / `claim.released` | ledger | a node took a claim by id before acting — `plan/<intent>`, `plan/case:<case>`, `optimize/<window>` — and gave it back: `holder`, and for a take its `token` and `until` |
| `attempt.claimed` | ledger | a leg's claim on an admitted, outstanding attempt, taken at the head (GH #946): the lease (`holder`, `token`, `until`), the principal that took it (`principal_mode`, `principal_name`: whose outcome the lease admits), the attempt's task, work and performer kind, and the command that took it |
| `attempt.outcome_requested` | ledger | the outcome a leg handed back under its lease: the disposition, the result (`result`, `result_ref`), the receipts it filed (`evidence_ref`), the calls it made (`calls`), the hat it wore (`hat_digest`, `hat_head`, `hat_watermark`, `prompt_digest`, `renderer`) and the command (`request`); relayed until `attempt.outcome` or `attempt.outcome_refused` answers it |
| `attempt.outcome_refused` | ledger | why the owner would not settle a leg's outcome (`why`, `holder`, `token`, `request`) |
| `attempt.released` | ledger | a leg gave its lease back without an outcome (`holder`, `token`, `why`, and the command); the attempt is another leg's to claim (GH #946) |
| `friction.filed` | record | what got in a position's way (`position`, `attempt_id`, `text`, and the command), filed on the attempt it was met on, else on the position; nobody admits it, it is a fact (GH #946) |
| `org.reviewed` | record | that pass's own answer |
| `person.retired` | record | someone left, and who took their work |
| `body.claimed` / `body.released` | ledger | who is running this record, by the lease's token |
| `body.provisioned` | record | a machine made able to run it |
| `body.credential_missing` / `body.credential_present` | ledger | whether the model's key is set where the body runs |
| `secret.rotated` | record | a credential set or rotated — the name and the place, never the value |
| `schedule.declared` / `schedule.refused` | ledger | a schedule the org chart declares, or one that would not be admitted |
| `schedule.fired` / `schedule.skipped` | ledger | the Task a schedule made, or why it did not fire |
| `schedule.paused` / `schedule.resumed` | ledger | paused and resumed by hand, in your name |
| `receipt.classified` / `receipt.withheld` | ledger | a protected body memory keeps sealed, or one withheld because no memory could keep it |
| `receipt.disclosed` | ledger | a reader authorized, for a purpose |
| `receipt.read` / `receipt.read_refused` | ledger | a read in the reader's name, or its refusal |
| `receipt.held` / `receipt.hold_released` | ledger | a hold that refuses redaction, and its release |
| `receipt.redacted` | ledger | the body removed, the digest kept |
| `concern.requested` / `concern.raised` | ledger | a concern from a part about the part above it; the request carries its own `request` id; the answer is one object (`what`, `severity`, `occurrence`, `request`, and since GH #995 the `parent` it is routed to and the concern-escalate execution that raised it, `task`), so a concern's words are never read as metadata, and one request is one concern, however often it is delivered |
| `concern.refused` | ledger | one the organization would not admit |
| `practice.read` | record | what the hat read once a proposal was ratified, at which head (`digest`, `target`, `ratified_at`, `hat`, `head`, `included`, `task`): practice-ratify's `hat` step (GH #995) |
| `concern.proposed` | record | three raises became a proposal — at concern-escalate's `raise` step once the source crossed the threshold, including the execution a birth admits for a source over it, unproposed |
| `pressure.requested` / `pressure.raised` | ledger | a signal from a source; the request carries its own `request` id and a node relays it until answered (GH #986); the answer is one object (`what`, `count`, `request`) and is written once per request — a row from before is `<what> x<n>` |
| `pressure.remeasured` | ledger | the declared fitness signals, measured again after the change |
| `appendage.proposed` | record | an organ the organization proposes for itself |
| `expression.restart_requested` | record | the organization asks for a change to be expressed |
| `expression.restarted` / `expression.deployed` | record | the shape and build the new expression reports, or what a deployment gateway expressed and judged |
| `expression.observed` / `expression.crashed` | record | the observation window's outcome; `crashed` names the instance and node on a fleet |
| `fleet.deploy` | record | a genome revision expressed through the fleet's nodes |
| `instance.up` / `instance.exited` | ledger | a node's report on one instance of the plan |
| `github.pr` / `github.commented` | record | the pull request a Review opened, and the settlement commented back |
| `ledger.adopting` / `ledger.adopted` / `ledger.abandoned` | record | the move of the day's work into the ledger, its checkpoint, and its undoing |

Their bodies are documented in the guide's reference chapter; the set
grows by ordinary change, and a reader that meets an unknown kind must
keep walking.

## Workflow definitions

A recursive workflow can be written as data (`dna/core/workflow_definition.hl`;
the execution contract it serves is `dna/WORKFLOW-CONTRACT.md`). A
`WorkflowCatalog` holds definitions: `define(id, revision, stores, title)`
declares a workflow whose ordered steps each write the one store its word
names — `stores` holds one word per step, `record`, `forge`, `genome`,
`heart`, `graph`, `nerves`, `memory`, `vault` or `host`, so the step
count is the word count (GH #995). A step is where a fact is written: reading, or waiting on
another writer, is part of the step whose fact it serves, never a step of
its own; `leaf(workflow,
revision, step, key, work, attempts)` adds a required member carrying
the content of one Work request and its retry allowance; `child(workflow,
revision, step, key, child, child_revision)` adds a required member that
invokes another definition. Each returns `""` or why it was refused (a
revision defined twice, a member for an undefined workflow).
Definitions refer to each other by id and revision, in two flat
collections.

Before anything is built, `expand` checks every definition reachable
from the root once, counting each one's Works and nesting height from
its children's and saturating at the limits: a catalog whose expansion
would be enormous is refused from those counts, in one pass over the
definitions, without materializing a node of it. The check also holds
for the root, which is depth 1, and for the limits themselves (a limit
below 1 admits nothing and is refused).

`expand(root_task, id, revision, limits)` binds one execution: the whole
finite tree as `BoundNode`s (tasks, steps, works) with their ids — a
child Task is `<parent>.s<i>.<key>`, a Workflow `<task>/wf<revision>`,
a Step `<workflow>/s<i>`, a Work `<step>/<key>` — each Work carrying its
authored request content and allowance (`bound_request` makes its
`WorkRequest`), and each Task its definition, parent, spawning step,
depth and definition path. It refuses the whole expansion, binding
nothing and naming what broke, when a definition is missing (by id and
revision), a workflow has no steps, a step has no members, a member
names a step outside its workflow's steps, a step declares no store
(`-`), two (`record+forge`: `declares two stores (record, forge)`) or a
word that is no store, a step writes or reads a part that is not built
yet (named with its issue: the heart, GH #987; the vault, GH #989; a
host the organization provisions, which only `hale dna body provision`
reaches today), a member key is not
`[a-z0-9-]+` or repeats in its step, a member is neither a leaf nor a
child, a workflow invokes itself directly or through its children, or
an `AdmissionLimits` value is exceeded: `max_depth` (8),
`max_steps` (32), `max_members` (32), `max_attempts` (8, the largest
allowance a leaf may bind) and `max_works` (256), the defaults in
parentheses, supplied by the caller and returned with the `Expansion`.
Two invocations of one definition are two executions with their own ids;
a later revision changes nothing an earlier revision expands to.
A definition id is `[a-z0-9][a-z0-9-]*`, checked when it is defined and
when it is read, so an id can never carry the delimiters the definition
path is written with. `encode()` writes every definition and member as
one JSON document (`format: dna.workflow-definitions/2`: each workflow's
`steps` and its `stores`) and `decode(text)` reads one back, refusing a
document that does not close, another format, an id outside the grammar,
an already defined revision, a document that defines one revision twice,
or a workflow whose `stores` do not name one per step, and adding
nothing then. `latest(id)` is the newest revision the catalog holds: what
a new Task binds.

`encode_bound()` writes one bound execution as a recipe (`format:
dna.workflow-recipe/1`; an admission nests it as an object of its own,
never escaped into a string) carrying the node count it was written with, and
`decode_bound(text)` reads one back. It refuses another format, a missing
node array, a count that disagrees with what the document carries, a node
without an identity or of no known kind, a node whose number was written
as text or whose text was written as a number, and a document that does not close: every object,
array and string closed by its own delimiter, with nothing after the
value. That is a closure scan rather than a JSON grammar validator — it
does not check escape sequences or number syntax — which is what reading
back one encoder's own output needs; a document from outside the system
wants a real parser in front of it. A whole final node also ends in `}`, so the last
character says nothing about the document. Every refusal binds nothing:
half a recipe is not a smaller execution. Nothing yet executes a bound
expansion.

## The workflow catalog (GH #995)

DNA ships a catalog of definitions, each a chain in which every step
writes one store (`dna::baseline_definitions()`, in
`dna/core/workflow_definition.hl`). A project's `dna/org/workflows.hl` is
generated — written by `hale dna new` and rewritten to the current shape
by every `hale dna upgrade`, whatever an earlier toolchain wrote — and
returns it plus the project's own (`fn workflows() -> dna::WorkflowCatalog`),
which live in `dna/org/own_workflows.hl` (`fn own_workflows(catalog)`:
project-owned, written only where there is none, never rewritten;
what it answers when a definition is refused is kept on the catalog,
`refused`, which `hale dna definitions` leads with and fails on, never a
smaller catalog; an upgrade that rewrites a workflows.hl holding
definitions of its own names each line it dropped, for the project to
move into own_workflows.hl),
and the generated organization admits from it (`catalog: workflows()` on
its `dna::Dna`; a body given no catalog takes the baseline at birth). The
baseline is the vendored toolchain's, so an upgrade supersedes it by
revision: a Task born under a revision finishes under it — its recipe was
bound at admission — and a new Task binds the newest (`latest`).

| id | steps (the store each writes) |
| --- | --- |
| `ask-triage` | `offered` · `classify` · `plan` (record each) |
| `change-deliver` | `candidate` (record) · `review` (forge) · `verdict` (record) · `apply` (genome) · `deploy` (heart) · `settle` (record) |
| `practice-ratify` | `ratify` · `hat` (record each) |
| `deploy-observe` | `deploy` (heart) · `settle` (record) |
| `concern-escalate` | `raise` (record) |
| `optimize-walk` | `walk` (record) |
| `secret-rotate` | `rotate` (vault) · `recorded` (record) |
| `body-provision` | `provision` · `start` (host) · `lease` (memory) · `observed` (record) |
| `ask-edit`, `ask-person` | the ask's one leaf: an edit prepared for review, or a person's job (record) |

Every step writes a fact of its own. What #995 listed as steps that write
none is folded into the step whose fact it serves: practice-ratify's
verdict is the Review's row, so `ratify` waits for it and writes the
ratification or the decline; memory's projection writes the idea and its
binding, so `hat` waits for it and records the row it was fenced to; the
request and the Review are written by whoever proposes (a person's
request, a concern, the record's birth); a concern's routing is a field of
its `concern.raised` row and its proposal is written in the same step at
the threshold; the optimize pass's walk reads what its `org.reviewed` row
records; deploy-observe's pulse is what `settle` waits for, and its
rollback is the failure edge, not a step: the happy path writes nothing
to the genome.

A step the organization performs itself is one leaf that `requires:
"organism"`. The engine relays its attempt to the assembly serving the
scope (`OrganismRelay`, kind `organism`: `StepRequested`, answered
pending), which performs the step and reports the attempt's outcome
(`Dna.on_step_requested`). A step ensures its rows — performed again
after a redelivery it writes none twice — and one that waits (a verdict,
the leader's word, memory's projection) is kept and performed again when
what it waits on may have arrived: a verdict, the leader's answer, the
tick. The organization performs the steps this toolchain knows, by
definition and key: `practice-ratify/ratify`, `practice-ratify/hat`,
`concern-escalate/raise`, `optimize-walk/walk`. A definition with an
organism step it does not know — a path not moved onto the catalog yet
(`ask-triage`, whose ask is still admitted as `ask-edit` or `ask-person`
after the leader plans it), or a revision from a newer toolchain — is
refused at admission naming the step, never failed when it runs, as a
definition on a part not built is refused naming the part; both are
`workflow.refused` rows and nothing else. The organization performs a
step only of a definition admitted directly, so one that invokes a
definition with an organism step anywhere below it is refused at
admission too. A ratification the catalog refuses is answered once per
incarnation — one `workflow.refused` row — and asked again after a
restart, so a catalog fixed in between ratifies it, and nothing retries
at every reconciliation. `hale dna definitions` lists
what an admission would refuse each definition for.

- **practice-ratify** carries every knowledge proposal the Board
  decided: when a verdict settles a proposal's Review, the organization
  admits one execution for it (`ratify:<digest>`), whoever proposed it —
  a person (`hale dna practice propose`, proposed by the organization as
  before), a concern, the record's birth (the purpose, the seeded
  practices, a repository's holes and practices), a holder asked for.
  `ratify` writes `knowledge.ratified` (and `knowledge.retired` for what
  it supersedes, or `knowledge.refused`) under an approval, and
  `knowledge.declined` otherwise, which fails the chain; it is done only
  when the whole consequence landed, so a retirement the record refused
  is written again. `hat` fails the chain, saying so, when no memory is
  named (nothing would come to wait for); otherwise it waits for
  memory's projection to hold the idea ratified, then reads the target's
  package at memory's head and writes `practice.read <digest>`
  (`target`, `ratified_at`, `hat`, `head`, `included`, `task`): what the
  hat read, at which head. No verdict path writes a ratification of its
  own — a Review's verdict or a decision by command (`review.command_decided`)
  alike; a settled verdict whose ratification never ran is admitted the
  same way at the next reconciliation.
- **concern-escalate** is a concern: `hale dna` concerns, and the
  organization's own, are admitted as `concern:<source>:<request>` (a
  redelivery of the request is the same execution; the same request
  with other words is `concern.refused`, as before). Its one step,
  `raise`, writes `concern.raised <source>` (`what`, `severity`,
  `occurrence`, `request`, the `parent` it is routed to, `task`) and,
  once the source crossed the threshold, proposes it as knowledge by the
  rule that always did. A source over the threshold with no proposal and
  no execution running for it is escalated once at birth by an execution
  that raises nothing more (its raises are the record's).
- **optimize-walk** is the optimize pass: the pass's budget and window
  claim are its admission's gate (`optimize.refused` as before); `walk`
  reads the machinery's signals, asks the leader once per incarnation and
  is done at its `org.reviewed` row, which names the execution (`task`).

**The record is the local-mode floor; the ledger is the runtime.** Every
step is a few rows of the day's work (`step.*`, `attempt.*`,
`work.settled`, `effect.*`). Until `hale dna ledger adopt`, each is a
commit on `refs/dna/journal`, so an execution costs tens of commits: a
Board deciding a repository's 35 seeded proposals waits some 15 to 18
seconds for their ratifications on a git-backed record (measured), where
memory takes the same rows in about a second. A git-backed record is what one person working
alone starts on; an organization runs with its ledger adopted.
`hale dna review <group> approve|reject` writes every verdict of the
group first and then waits for the answers once, bounded by the group's
size.

**The declared purpose is a proposal like any other.** `hale dna init`
proposes it in the record's one seed call (host verb `record-seed`, or
`graph-ingest` for a repository: kind `purpose`, author `org`, for
`org`, `provenance: declared`) with the Board's Review under the
group `purpose`, which `hale dna review` lists first and `hale dna
review purpose approve` decides; the generated organization holds no
Review of its own for it.

`hale dna definitions [project] [--json]` lists the catalog
`dna/org/workflows.hl` returns, built beside a one-line main as the model
catalog's probe is: every definition with its revision and title, each
step with its key and store, what a newer revision
supersedes, and what a definition is refused at admission for; `--json`
is the catalog's own document. The head serves the same catalog to the
face's definitions views (`ops::ProjectWorkflowCatalog`: what
`hale dna definitions --json` reads, re-read when either file or the
vendored catalog changes; the baseline alone for a project with no
`workflows.hl`; unavailable, with the reason, when it does not build).

## Workflow facts

What a workflow execution leaves behind is written as a fact of the day's
work (`dna/core/workflow_events.hl`). Seven kinds carry an execution:
`workflow.admitted` names the Task, the definition and revision, the
inputs and their receipt, the bound recipe, the limits that were applied
and what the expansion came to; `step.registered` names a step's whole
required set before any of it is dispatched; `attempt.admitted` carries
one attempt's id and the `WorkRequest` it was admitted with;
`attempt.outcome` its disposition, result and evidence; and
`work.settled`, `step.completed` / `step.failed` and `workflow.settled`
settle each level to the one above it, a child workflow naming the parent
Task and spawning step it answers. An admission also names the engine (`wf1`), and every fact a version.
A decoder refuses a version it does not know, another engine's
admission or refusal, a fact that does not name the entity it is about,
a fact that says nothing where a number belongs (an absent revision is
not revision 0), and a fact whose own identities contradict each other:
an attempt id that is not its Work and number, an attempt carrying a
request for another Work or another attempt, a Work settling under a
Step it is not part of or on another Work's attempt, a step failing on
a member it never had, a child naming its parent but not the step that
spawned it, an admission deeper or wider than the limits it says were
applied. A fact is read whole or not at all. The identities an attempt
admission carries are written once and derived from the fact, so its id,
its number and its embedded request cannot disagree.

`step.activated` and `workflow.refused` have codecs of their own, like
the rest; a routing row is not a durable shape. A fact a transition
committed carries the scope, key and proposal id it was committed
under, appended with it, so the committed proposal can be reconstructed
from the journal after a restart and compared against a later reuse of
that id.

These kinds are the ledger's under split routing and the record's on
routing 0, like every other operational kind.

An attempt's id is its Work and its number (`<work>/a<n>`, numbered from
0 as §4 has it: the first attempt is `a0` and a retry is `a1`), so a
retry is a new id and a re-sent admission is the same one. A transition's
proposal id (§9 of `dna/WORKFLOW-CONTRACT.md`) is what the transition
does, not a counter or a clock: the same id carrying the same scope,
key, kind, entity and body is that transition proposed again, which the
committer answers from its journal; the same id carrying anything else
is a conflict, and `transition_conflict` names which part differs.

## Workflow state

The state an execution is in is read from those facts and nothing else
(`dna/core/workflow_projection.hl`). `replay(projection, kind, entity,
body)` applies one row and answers one of three things: the fact was
**applied**, it is a **replay** of what the projection already holds, or
it is **refused**. Applying a fact asks nothing of the world: the entry
point carries `@no_syscall`, which the compiler checks through everything
it reaches, and the projection holds no journal, gateway or model to
call. The contract guards syscall-class effects; having nothing to call
is what rules out the rest.

**The admitted recipe is the authority.** An admission carries the whole
bound tree under its Task, and every later fact is checked against it: a
Step is one the recipe binds for that Task at that index; a registration
names exactly the members the recipe bound under that Step, each by key,
kind and entity, none missing and none invented; a Work's retry allowance
is the one bound for it; a child is admitted only as the Task its parent
bound under that key, from a step that is registered and active, and
answers only as that Task. A child's admission carries the subtree its
parent bound under it, exactly: the same nodes, each the same in every
field, none added, none left out, none changed. It activates a binding;
it never redefines the execution its parent admitted. An attempt asks
for what the Work is: its request's content — objective, context,
bindings, output contract, data class, requires, target, cost ceiling —
is the bound Work's, on the first attempt and on every retry; who
performs it is the application's choice. A fact the recipe does not
know is refused, however well-formed.

**Every transition has its basis.** A Work settles `done` only on a
current attempt that recorded `done`; it settles `failed` only on a failed
attempt with no allowance left, or once its Step has failed or its Task is
cancelled, under which a member records its outcome and stops (the drain
policy); it settles `cancelled` only under a cancelled Task (the fence).
A retry is the next attempt, admitted only after the last one recorded a
failure and only within the allowance. A Step activates only after the
step before it completed, completes only when every member it registered
has settled `done`, and fails only on a member that failed. A Task
settles `done` only when every Step the recipe binds for it completed,
and `failed` only after a Step failed and every responsibility it admitted
has settled: the root settles last. Cancellation stops further admission
in the cancelled Task and in everything under it — no attempt, no step,
no child is admitted below a cancelled ancestor — while what was already
admitted still records its outcome and settles under the fence. The walk
to the root goes as deep as the caller's `max_depth` admitted — nothing
here has a ceiling of its own — and an ancestry that does not resolve
refuses admission rather than permitting it. A failed ancestor is not a
cancelled one: under the drain policy a member keeps its responsibility
until its own outcome.

**A fact is one row.** The row's entity is the fact's own identity, and a
`step.completed` row carries a completion. A fact whose identity is
already recorded is a replay when it is the same bytes and a conflict
otherwise, checked before the state, so a row from history is recognized
as such after the state has moved past it. The join is by identity, never
by counting: two answers from one member are one member. A Work's result
is the output of the attempt it settled on, readable only once it has
settled; what an outstanding or superseded attempt said is never the
Work's result.

## Workflow admission

An application admits a defined workflow through the assembly
(`Dna.admit_workflow(WorkflowAsk)`): which definition and revision, with
what inputs, under an identity of the caller's. The durable fact precedes
everything. The id is minted, the definition expanded under the
assembly's `admission_limits` (its `catalog` holds what the application
defined), and the admission — engine, bound recipe, limits, counts, the
ask's identity — appended with exact compare-and-append before anything
could run; only then the legacy `task.born` summary, so the tooling of
the day lists the Task. A refusal, by the catalog or by the record, is a
`workflow.refused` row under the id it minted and nothing else: no
summary, no child, no work request. `Dna.run_workflow(WorkflowAsk)` is
the admission followed by the execution's start (card 18, below).

The ask's id is the admission's identity. Asked again — relayed after a
lost answer, retried — it is one execution, found in the record by that
id. One decision, one revision: the record is read once, at a revision
the body captures; whether the ask already landed and whether a candidate
id is already an execution's or a refusal's are decided against that
reading; and the append is exact at that same revision. An exact append
protects the revision it was given, not a decision taken at an older
one, so when the record has moved — stale, or the store's unique claim on
the id — the whole decision is taken again from a fresh reading. A taken
id is skipped before any append. Two bodies contending for one Task id
cannot both take it, and one ask admitted by another body between this
body's reading and its append is found on the next reading, not
admitted twice.

Only the owner of the position admits (GH #664): the same rule `ask`
applies, applied before an id is minted, so a body over a shared record
that names no owner admits no workflow either — it answers the asker and
writes nothing, the ask being the owner's to admit. A refusal, whether
the catalog's or the store's, is written as the versioned
`workflow.refused` and the write is checked: when the record will not
take even that, the answer says the refusal went unrecorded rather than
presenting it as recorded. A refusal bound to a Task id carries the
admission's own guarantee — it is appended exactly at the revision that
found the id free, and when the record has moved it is read again: the
ask may have landed from another body, in which case that execution is
the answer, and the id may be another execution's now, in which case the
refusal is not a Task's at all. A refusal that reaches no Task — the id
taken meanwhile, every id it would mint already taken, the record moved
under it too often — is a fact of its own kind, `workflow.ask_refused`,
under the ask's own identity and a decision ordinal (`<ask>#<n>`), which
can share no identity with any Task under any owner's name; the
projection holds it beside the executions, never as one. The same ask
refused again for the same reason is the same decision, answered without
another row; refused for another reason, it is the next ordinal. That
decision, too, is one reading of the record: the decisions already taken
on the ask are counted at a captured revision, the ordinal chosen against
that count, and the append is exact at that revision, so another body
deciding on the same ask meanwhile moves the record and the decision is
taken again — its decision for the same reason becomes this body's
answer, and for another reason takes the ordinal ahead of this body's.
Nothing is appended as a changed body under a recorded identity, and a
refusal never attaches to an execution another body admitted. A restart counts admitted and refused
ids among those minted, so an id is never minted twice, and an admitted
Task is never resumed as legacy edit work, with or without its summary
row: the admission is the one positive discriminator, and its recovery
belongs to the engine's own cards.

## Workflow execution: one attempt

The durable Work owner admits an attempt (`attempt.admitted`) and then
asks for it to run through `AttemptExecutor` (`dna/core/
workflow_execution.hl`), which runs one admitted attempt once. Nothing
runs before the admission is in the record as admitted: the attempt's
identity, its performer kind and every field of its request are checked
against the recorded admission. An attempt whose outcome is recorded is
not run again; the recorded outcome is the answer, claim or no claim.
The decision to run is one reading of the record: refreshed, read at a
captured revision — the admission, whether the attempt is settled,
whether its execution is claimed — and the claim appended exactly at that
revision, with the effect claim of the existing effect records, keyed by
the attempt. When the record has moved the decision is taken again: an
outcome recorded meanwhile is the answer and the performer is not
called; a claim taken meanwhile is attached to; only a claim that lands
is followed by a performer call. A request that arrives while the
attempt is running attaches to its pending outcome instead of starting a
second performer, and a claim the record refuses runs nothing. The performer the admission names runs
once. Every reply, the one that comes back from the call and the one
that comes later, is judged first for whose it is: it must be about this
attempt (a reply that echoes an `attempt_id` must echo this one) and
from that performer (the identity the kind selects), and a reply that
is neither is not this attempt's pending answer either — it is refused
before anything is made of it, and the attempt stays outstanding under
its claim. A reply that is this attempt's and not terminal leaves it
outstanding and records nothing; a terminal one must say something the
outcome contract carries — `done`, `failed`, `declined` or `timeout` —
or it is refused without a row and without closing the claim. The
later reply is settled through the same path, under the same rules. An accepted outcome is persisted as `attempt.outcome` with an
exact append before anything is answered, and a reply for an attempt
whose outcome is already recorded changes nothing; an outcome the
record will not take is reported unrecorded, never as done, and nothing
advances on it.

The executor retries nothing (one retry owner: the Work's lifecycle) and
advances nothing (the Work settles on the outcome in a later card). It
makes no claim that external effects happen once: a performer that
acted and died before its outcome was saved is a later card's. The
`WorkSystem` retries nothing of its own (card 18): it routes each
admitted attempt to the performer of the kind the admission names.

## Legs: the hat, the claim and the outcome

The spine's whole API to a leg is three things (GH #946): the hat as a
read, `dna.attempt.claim`, and `dna.attempt.outcome`. A leg holds
nothing between tasks and has no database role; the owner still admits
and settles. Once the organism has adopted the ledger the attempt kinds
live there: the head reads them and writes a leg's rows there under its
role (`HALE_DNA_MEMORY_DSN_HEAD`; memory's insert function is the gate,
keyed by the request), and refuses the commands and the hat only when no
memory is named to it.

- **The hat** (`GET …/dna/context?id=<work>`; `dna/operations/context.hl`)
  is one Work's context as structure, never a prompt: the position's
  identity — the graph's `position:<name>` id, from the performer kind
  the attempt admitted last names, or the kind the request selects
  before one is — and its charter (the record's `graph.node` text for
  that id, `""` until the graph names it); the practices ratified for
  the Work's target, from memory under the reader's role, resolved to
  text with their ids; the knowledge bindings; the tool grant; the
  output contract; the data class; the Work's history as facts; the
  record head and memory's projection watermark it was rendered at
  (`-1` without memory, and `practices_status` says so); and the
  digest, sha256 over the canonical body with the digest itself left
  out. Rendered twice at one head it is one digest; a row that moves
  the head moves it. Two attempts of one task may wear different hats,
  correctly: what is forbidden is unrecorded variation, so the hat reads
  no clock, no environment value and no random id, and replay renders
  from the recorded hat, never the live graph. Rendering is a leg's,
  and the renderer's version is evidence of its own. The practices are
  structure — `{id, name, text, kind, author}` from one snapshot of
  memory, the watermark read in the same transaction as the ranked
  bound — never rendered lines split again; and every hat built, by the
  owner for an edit it asks (the attempt carries the hat's digest as its
  `context_digest`; the practices reach the editor as its brief, the
  ask stays the ask) or by a head for a leg, is kept in memory by its
  digest (`hats`, insert if absent).
- **The claim** is taken at the head. The filter names the performer
  kind and identity, the capabilities the leg has, the data classes it
  may see, the owners it works for and a TTL. The head picks the first
  admitted attempt of that kind that is outstanding — asked to run
  (`effect.requested attempt:<id>`), no outcome — fits the filter, and
  is not held by another leg under a live lease; takes memory's claim
  `attempt:<id>` for the performer with the TTL under the head's role
  (GH #1026: the store decides between two legs racing; without memory
  nothing races); and appends `attempt.claimed` naming the lease and
  the principal that took it. The claim is not fenced on the record
  head the leg read — memory's conditional insert is the race, so a leg
  a row behind is not turned away with `stale_subject` — and a claim
  memory took that ends in no row is given back. A leg that names no
  data class is handed nothing: no class is no class. The
  receipt returns the task and the lease as a value. Nothing fitting,
  or everything held, is a refusal: a receipt with the reason — which
  names the class or the owner that stood in the way — and no
  row, since legs ask often and an empty answer is not a fact.
- **The outcome** comes back under the lease, with the hat the leg
  wore (its digest, the head and watermark it was rendered at, the
  digest of what was rendered, the renderer's version — the five
  together or none). The head refuses — a receipt with the reason, no
  row — an attempt never admitted or never asked to run, one already
  settled or already handed back (`duplicate`: one outcome under one
  lease), a lease that is not this holder's at this token now, by the
  recorded claim and, with memory, by memory's (`stale`), and an
  outcome from a principal other than the one that took the lease;
  files the receipts (a protected body in memory alone, the rest as git
  receipts); and appends `attempt.outcome_requested`. A node relays the
  row onto the nerves (`WorkSubmit`, `dna.work.submit`) until the
  record answers it. The owner checks the same lease, journals the
  calls the leg made as `model.called` rows on the attempt id — tokens
  per task hold out of process — and settles the attempt through the
  path every reply takes: `attempt.outcome` by exact append, naming
  the request it answers and carrying `result_ref` and the hat, the
  Work and the task after it; a submission it will not settle — a
  stale lease, a second outcome after settlement — is an
  `attempt.outcome_refused` row naming why and the request, which
  answers it, so the relay stops. A `LegRelay` performer answers
  pending for the kinds a program hands to legs, so the attempt waits
  for one instead of running in process (stage 1 of the legs; the
  in-process performers go as each stage lands), and `RelayReplay` is
  its reconciler: an owner that restarts with a leg's attempt in flight
  leaves it to its claim — nothing is asked of it while the lease is
  live, and it is never `effect.result unknown` — and asks it again
  once the lease expires with no outcome (`effect.redelivered`, once per
  lease), when a leg claims it anew.
- **The verbs** (`hale dna work`, GH #946 slice 4; `dna/core/legs`,
  vendored as `vendor/dna/legs`) are a leg as API clients: `next` is
  `dna.attempt.claim` for a position (`--as position:<name>`, the
  graph's id, never a free string), `brief` the hat read — or rendered
  in the leg, `text`, `prompt` or `agent`, recording the hat digest,
  the digest of what was rendered and the renderer's version
  (`legs-render/1`) — `renew` is `dna.attempt.renew` (the lease
  extended, the token kept: another `attempt.claimed` row), `submit`
  is `dna.attempt.outcome`, `settle` that command read back, `release`
  is `dna.attempt.release` (`attempt.released`; the attempt is another
  leg's to claim), and `friction` is `dna.friction.file`
  (`friction.filed`, a fact nobody admits). Each prints one JSON
  object and holds nothing afterwards; `submit` and `release` are
  keyed on the lease, so run twice they are one act; `next` mints a
  fresh id per call (a claim by its holder renews); `renew` is counted
  by the caller. A leg's identity is a position the record knows —
  `position:<name>` from the graph or the organization's own, with
  `#<n>` for one worker of several — never free text, at the head as
  at the verb. While an outcome under a lease awaits the owner the
  lease is neither renewed nor given back, and the attempt awaits no
  other leg; without memory the record numbers the leases (the next
  claim row is the next token). Exit codes: 0 admitted, 1 refused (the
  receipt printed) or unreachable, 2 usage.
  `run` is one cycle through the project's performers
  (`dna/org/work.hl`, generated at init): a person's leg renders the
  brief and leaves the outcome to `submit`; a deterministic performer
  wins for the work kinds it takes and settles; the model performer
  (`legs::ModelPerformer`) is the catalog's router behind the
  performer interface, out of process, one task per run: the brief
  rendered as a prompt, the answer the result, and every call the
  router answered handed back as evidence (backend, reported model,
  tokens, cost, wall time under the prompt and context digests), which
  the owner journals as `model.called` rows on the attempt; a
  rate-limited call is backed off inside the attempt (bounded,
  doubling, the lease held) and each wait is a row of evidence of its
  own, so the attempt's cost in time is in the record beside its cost
  in tokens; the lease is renewed before each wait. The model is sent
  the render alone (the hat is structure), and the hat's digest is the
  context digest on every row. `--performer` names the performer
  instead of the catalog's choice; a performer never answers a kind it
  does not take, forced or not. Every performer declares an **effect
  class** — `effect_free` (answered, touched nothing), `idempotent`
  (may be run again to the same end) or `uncertain` (an agent's seat
  with tools: may have acted once already) — with no default: a
  performer declaring none is refused when the leg starts. The class
  rides on the claim (`effect_class`), where the head admits an attempt
  only if its Work's requirement allows the class — a judgment or an
  analysis admits `effect_free` and `idempotent`, an edit `idempotent`
  and `uncertain`, anything else all three; the hat says so in
  `effects` — and on the outcome, where it is evidence
  (`effect_class` on `attempt.outcome_requested`). A settle that fails
  (the outcome read back neither settled nor requested, or unreadable)
  on an `effect_free` or `idempotent` performer is `unsettled` and the
  loop runs the child again; on an `uncertain` performer the leg files
  friction naming the attempt, the request id and the lease, keeps the
  lease, answers `unresolved`, and the loop stops that slot: nothing is
  retried by a program until evidence or a person decides. The model
  leg treats a lost reply (the call may have been made) the same way:
  `unresolved` behind an `uncertain` performer, never retried or failed
  over; a `failed` outcome behind the other two. A rate-limited call was
  never made, so its backoff holds for every class. `loop --parallel N` is a worker: N
  supervised child processes, each `run` as its own holder
  `position:<name>#<n>` (two workers of one position never share a
  lease), each answer one JSON line as the child ends, started again
  at once after a task and after a per-slot doubling backoff when idle
  (nothing claimed, a person's, given back); `--once` runs each once.
  The leg is its program's main locus: SIGTERM/SIGINT drains the loop,
  which ends its children (TERM, then KILL after a grace) and reaps
  them before it ends; `--drain` (a marker beside the leg) ends a
  running loop once its children have. Nothing is held between tasks;
  a child ended mid-task leaves a lease that expires. A project
  initialised with no backend configured gets `NoModel`: its agent
  Works are a person's. The catalog and the tape (`models.hl`,
  `tape.hl`) stay in the core while the editor and leader call them
  in process. A performer is handed the brief and
  the hands as interfaces — git in scratch, the forge (it decides, a
  leg never merges), the toolchain; deploy and the heart's API refuse,
  naming #987 — and answers a performance (a result struct until #732).
  The verb is the project's own program, built beside the vendored
  seed under `.hale/dna/legs` by the host; `hale mcp` exposes it as
  `hale_dna_work`.

## Workflow execution: one step

One active step and its leaf Works are resident loci
(`dna/core/workflow_runtime.hl`), in the shape the lifetime proof
established: a `StepRun` is an accepted child no parent releases, born
from its owner's handler, alive after its `run()` and every handler
return, advancing from its own handlers keyed by its id, ending with
`terminate;`. It owns its `WorkRun`s the same way, one per leaf, born
from the handler that heard the activation land. Neither holds a
journal: each proposes every transition to the one `WorkflowRuntime`
over the bus — `TransitionProposed`, keyed by scope — holds one
proposal at a time, and acts only on that proposal's answer, and only on
`ok`. A refusal dispatches nothing.

`WorkflowRuntime` is the committer, and the executor, over one journal.
A proposal is decided at one reading: the runtime refreshes, applies
what the record holds beyond what its projection has seen and remembers
each committed proposal's id and revision, and that reading's revision
is the one the decision is taken at and the one the append is exact at
— never a second look. A proposal already committed is answered again
from what was committed, if it is the same proposal; the same id
carrying another transition is a conflict, refused with nothing
appended. Otherwise the transition is validated against the projection
without touching it (a dry run of card 06's rules), appended exactly at
that revision, and applied for real once the append has landed. When
the record has moved under the decision — another runtime committed the
same transition, or anything else — the append is stale and the
decision is taken again against the new state: the same transition
committed meanwhile is answered from its row, and one row stands. An
append the record refuses answers a refusal and moves nothing, so the
same transition proposed again is decided against the state as it is.
A proposal the state refuses lands nothing: the record is not the place
a transition is found invalid. An answer says who refused (`basis`:
`state`, `record` or `conflict`), because the residents treat them
differently. The body of a proposal carries the reference the row will
be committed under, and it must be the proposal's own scope, key and id,
whole, or the proposal is refused as a conflict with itself before the
state sees it; a committed proposal is rebuilt from its row and compared
field by field with one sent again — the same, whole, is a replay
answered from the row; anything else under that id, another key
included, is a conflict. A Work asks the runtime to run its
admitted attempt by id (`AttemptRequested`); the runtime reads the
admission from the record and runs it through the one-attempt path
above, and every reply — the one that came back from the call and the
one that came later, through `settle` — reaches the Work the same way
(`AttemptReplied`, keyed by the Work), so an immediate and a delayed
reply are one reducer and one completion path.

The barrier holds as the contract has it. A `StepRun` registers its
whole member set (`step.registered`), activates (`step.activated`), and
only then births its leaves; a member answers once, by its key
(`MemberSettled`, keyed by the step): a member that answered again and
an answer from no member of the step change nothing. Two required
leaves complete the step in either order; one immediate reply and one
delayed one leave it waiting, alive, with the outstanding Work
reachable. A member that failed fails the step at once
(`step.failed`, naming the member) and a step whose every member settled
done completes it (`step.completed`); either publishes one terminal
outcome (`StepSettled`, keyed by the step) and settles the step. A
member is its key and the Work bound under it: a settlement naming a
required key for another Work is no member's and changes nothing. After
that a sibling's later reply is recorded — its attempt's outcome and its
Work's settlement land as facts — and reopens nothing; the `StepRun`
stays until every member it dispatched has settled, then reclaims
itself, and a `WorkRun` reclaims itself once its settlement is answered.
A leaf with allowance left is retried before it fails: each attempt is
admitted as a proposal of its own, and a retry the record will not take
— the step failed meanwhile — settles the Work failed on the attempt
that failed, the drain. A refusal by the record is no outcome: a Work
or a Step whose append the record refused holds its transition, tells
nobody it settled, reclaims nothing, and proposes the same transition
again when the record resumes (`RecordResumed`, published by whoever
knows the record is writable again) or, for a Work, when its attempt's
reply reaches it again; a member has answered its step only once its
settlement is in the record. A Work whose first admission the state
refused has nothing admitted, no claim and no effect to await: under a
Step that has durably settled it retires (`MemberRetired`, keyed by the
step), which is not a settlement — it answers nothing — and the step
counts it toward its drain and reclaims itself once every admitted
responsibility settled; a first admission the record refused is
proposed again when the step settles, so the state says whether it can
still be admitted. One the state refuses while its step runs stays,
unattempted and reachable, until it is fenced. Nothing
here orders steps, spawns child workflows or survives a restart; those
are later cards.

## Workflow execution: ordered steps

One admitted execution is a `WorkflowRun`, resident like its steps: it
holds every leaf of the execution (copied at birth), births step 0, and
births step i+1 only from the handler that hears step i has drained —
`StepDrained`, keyed by the task, which a `StepRun` publishes as it
leaves, once every member it admitted has settled or retired — behind
that step's committed completion. No ordering lives in the workflow
that the record does not enforce: the projection refuses a step's
activation while the step before has not completed, and the
activation's own identity (`<step>@activated`) is one row at the
committer, so a duplicate completion cannot start the next step twice;
a drain message for a step the workflow has advanced past, or for none
it has active, changes nothing. When the last step drained completed
the workflow proposes `workflow.settled` done; a failed step drains
first, then the workflow proposes `workflow.settled` failed, and later
steps are never born. Both are proposals like any other: decided at one
reading, held when the record refuses them, proposed again on
`RecordResumed`.

A cancellation is asked of a workflow by task (`WorkflowCancelRequested`)
and is a proposal too: `workflow.settled` cancelled, which the state
takes at any point before the execution settled and refuses after.
Once it landed the workflow says so (`WorkflowSettled`, keyed by the
task) and births nothing further; its active step hears it and fences
what it dispatched (`StepFenced`, keyed by the step) — a step with no
outcome yet is `cancelled` (no row of its own; the task's row is its
basis), and a step that had failed and is draining keeps its failure:
the fence is not a second outcome. The fence is the one message a Work
acts on for a cancellation (a cancelled step settlement is for the
owner; a Work ignores it), so a Work that retires or settles on it
acts, and ends, exactly once. Its Works hear the fence: an
admitted attempt still awaiting its reply settles cancelled (the
projection allows that settlement only under a cancelled task); a first
admission the record refused is proposed again and, refused by the
state, retires; one the state had refused retires. The fence itself is
in the record, not in a notification: the executor reads the
execution's durable cancellation — the task's `workflow.settled`, or an
ancestor's, walked through the admissions — at the same reading its
execution claim is exact at, and an attempt not yet claimed under a
cancelled execution does not run, however the notification lagged,
while one claimed before the cancellation still records its outcome.
Nothing external is undone — a reply that comes later is recorded as
the attempt's outcome and reopens nothing. The workflow leaves once the
fenced step has drained. An activation the record
refuses dispatches nothing: the step holds it, no leaf is born, and the
record's resumption lands it once. Nothing here spawns child workflows
or survives a restart; those are later cards.

## Workflow execution: child workflows

One admitted Task is a `TaskRun`, resident like the rest: it holds the
bound recipe, decoded into its own catalog at birth, and from it the
leaves and children of every step, and births the one `WorkflowRun`
that runs them. The executions owner (`Executions`, the Task owner the
contract names, one per scope) accepts every `TaskRun` and never
releases one: a root asked of it (`ExecutionAsked`) with its admission
already in the record, and a child asked by its parent Task. One
execution is one Task: a Task asked twice is born once. The same
engine runs every level; there is no other path for a child.

A step that dispatches a child member asks its own Task
(`ChildRequested`, keyed by the parent task), which holds the recipe
the child is bound in; the Task cuts the subtree the recipe bound
under that child, exactly, and asks the owner to admit and run it
(`ChildAdmitRequested`, keyed by scope). A request that names a child
the recipe does not bind under that step and key is ignored. The child
proposes its own admission first — `workflow.admitted` with the
subtree, its parent, its spawning step and its member key, which the
projection accepts only for what the parent bound, from a step that is
registered, active and unsettled — and runs only once that landed; an
admission the record refused is held and proposed again when the
record resumes, or when the spawning step settles or is fenced — so
the state, not the child, says whether it can still be admitted, and
under a failed step it retires on that answer; one the state refused
retires under the settled or fenced spawning step, as a leaf does. A
fence that reaches a child while its admission answer is out is kept,
with its reason, and applied when the answer comes: an admission that
landed starts and is cancelled at once through its own settlement. A
spawning step that settles or is fenced while the answer is out owes
the child one re-decision, taken on that answer if it is a refusal by
the record: the admission is proposed again once, and the state says —
neither ordering of the two messages is assumed, and a record that
keeps refusing is not spun against. A child that settled tells the step that spawned it once
(`ChildSettled`, keyed by that step) after its row landed, and the step
counts the exact child bound under the key: a settlement for another
Task under the key, for a key that is no child of the step, or
delivered again changes nothing; a failed child fails the step at
once, as a failed leaf does. The Task leaves once its workflow has
(`WorkflowLeft`), and tells the step that spawned it so (`ChildLeft`,
keyed by that step): the step's drain waits for a child that settled
to have left, not for its settlement, so no ancestor reclaims itself
ahead of a descendant still holding something — a Work whose
settlement the record refused, say. A leave from a child that has not
answered is nobody's: a leave follows a settlement. A step fenced by its Task's cancellation fences a
running child the same way: the child's Task asks its own workflow to
cancel, which settles cancelled through the same proposal, so the
fence reaches every level below and each level drains and reclaims
from the leaves upward.

## Workflow execution: restore

Restore is not a mode. An execution asked of the executions owner over
the record a crash left — or fresh: the same ask — first asks the
runtime what the record holds about it (`ExecutionStateAsked`,
answered from the projection): one the record has settled is over —
its settlement is announced and the workflow leaves, birthing nothing
— except a cancelled one whose admitted Works have not settled:
cancelled is not cancelled-and-drained, and the current step is born
fenced, so those Works settle cancelled, the step drains, and the
workflow leaves behind it. A cancellation heard while the record is
being asked waits for the answer: an execution the record has settled
is not cancelled, and one still open is cancelled first — and then,
as whenever a cancellation lands with no step active, the record is
asked again what it still holds admitted and unsettled, and that is
drained through the current step, born fenced, before the workflow
leaves. One still open proposes its transitions exactly as a fresh one
does,
and the committer answers what the record already holds as a replay,
so every Task, Step, Work and Attempt id a restored incarnation uses is
the record's, never minted again — a completed record rebooted gains
no row and runs nothing, and a cancelled one ends the restored
execution before anything is born.

The one thing a restart adds is redelivery, and the runtime derives it
from its own incarnation, never from a request: a request for an
attempt names the attempt and the Work it is an attempt of, and the
runtime knows which attempts it claimed itself and which it already
redelivered. Every request is decided against the record at one
reading: an attempt whose outcome is recorded is answered from it and
never runs again, and a claim the dead incarnation left open under it
is closed with it, durably, before the Work is answered — on every
path that answers from a recorded outcome: a request that finds it,
one that runs into it on its own reading after seeing the attempt
unclaimed, one that attaches to a running attempt and finds it, and a
reply — and
a close the record refuses leaves the Work holding its responsibility,
told so, until a later request or reply closes it: a Work whose
request was refused without an outcome asks again when the record
resumes; one never claimed runs through the
ordinary path, which claims first; one claimed by this incarnation, or
redelivered by it, is running, and the request attaches, so a duplicate
dispatch starts no second invocation; one claimed and never answered by
a claimant this incarnation is not was claimed by an incarnation nobody
will hear from again — unless its effect resulted unknown (a restart
the existing recovery reconciled no further), which stays visibly
unresolved, nothing running on it until card 12c's reconciliation
says what happened. Otherwise its dispatch is issued again with the
same attempt id and no new claim — a transport redelivery, not a new
attempt — and that decision is a row (`effect.redelivered`) appended
exactly at the reading that saw the claim, no outcome and no fence, so
an outcome or a cancellation landing meanwhile makes it stale and the
decision is taken again. A reply from the old process for the
still-current attempt is accepted and recorded like any reply, and
whichever reply arrives second changes nothing. The admission's
performer kind is durable: the restored incarnation runs the attempt on
that kind. What an invocation that died did before it died is a later
card's, as is cross-process delivery.

## Workflow execution: recovering the tree

A restart rebuilds an execution from the record alone. The executions
owner is asked with nothing but the task's id and which performer kind
runs each leaf (`ExecutionAsked`); it asks the runtime for the
admission as the record holds it (`AdmissionAsked`), and the runtime
answers from the row, read whole, and from its projection's word that
the row was admitted (`AdmissionAnswered`): the definition and the
bound revision, the recipe, the limits. The Task is born from that
answer, so an execution restored after a restart runs what it was
admitted with — its bound recipe — whatever the catalog offers now; a
definition changed to a new revision between shutdown and restart
binds new admissions and touches no old execution. A task the record
never admitted, one admitted with a recipe that cannot be read, or one
whose admission the projection refused is not resumed: the owner says
so (`ExecutionRefused`, with why) and nothing is born or invented for
it. From there the residents re-propose through the transition code
live execution uses, and the record decides what replays and what
runs: a step the record completed is not reborn and the next step
starts once; a child the record settled announces its settlement and
leaves, and its parent's step completes on it; a member still waiting
is recovered where the record left it — its admitted attempt
redelivered, under card 12c's reconciliation — and completed members
do not rerun; the grandchild settles, then the child, then the root.
Only unfinished responsibilities exist in memory after a restart, and
every id and every required-member set is the record's.

The admission recovered is the one the projection accepted — the body
it recorded when it applied the row — never the last row in the record
that names the task: a later row the projection refused (a
re-admission under another revision, a row whose body names another
task) is not the execution, and a task the projection holds no
admission of is refused with the projection's reason for the last row
it refused, from a dry replay of that row. A child of another Task is
not recovered by asking for it: the answer says whose child it is,
under which step and key, and the child comes back through its parent,
in its bound place. The workflow identity an execution runs under is
the one its recipe bound, read from the record and never re-derived;
the projection refuses an admission whose recipe binds the task under
any workflow but `<task>/wf<revision>` (§4 of the contract), so the
recipe and the identity never say different things. Every question the
owner puts to the record carries an identity of its own
(`AdmissionAsked.ask_id`), held outstanding until answered; an answer
is taken only to a question this owner holds outstanding, for the task
it asked about, and only once — an answer nobody asked for, however
positive, and an answer under a question already answered, are strays
and birth nothing. A leaf asks the record what it holds about it before
proposing anything (`WorkStateAsked` / `WorkStateAnswered`): a settled
Work has answered and leaves; a Work with an admitted attempt resumes
that attempt under its number, id and performer kind as the record has
them — the kinds a restart is asked with bind only attempts not yet
admitted — and a Work never attempted admits its first attempt under
the asked kind. The state question carries the Work's scope and an
identity of its own (`WorkStateAsked.ask_id`), held outstanding until
answered; the answer carries both back, and the Work takes only the
answer to its one outstanding question — its scope, its identity —
once: a same-named Work in another scope's record (ids are minted
within a record) is not this Work, and a stale or unsolicited answer is
no state. A fence heard while the state question is outstanding is
remembered and decided on after the answer: an admitted attempt is
re-proposed and, admitted under the fence, settles cancelled; a Work
never attempted proposes its first admission, which the state refuses
under the cancelled Task, and retires. No settlement ever names no
attempt.

## Workflow execution: uncertain external effects

An attempt's performer may have acted before the outcome was saved:
the incarnation claimed the attempt, invoked the adapter, and died
before the outcome row landed. Recovery reconciles before it decides
on a redelivery. The work system carries an effect adapter's
reconciliation beside each of its performers, one per kind as the
performers are (`Reconciler`, default `NoEvidence`): the adapter of the
admitted kind is asked, for the identity that kind selects, about an
attempt whose claim an incarnation that died left open, and it says
whether it can tell, whether it acted, what it recorded when it did, or
that acting again is documented safe. Its answer names the kind and
identity it speaks for, and evidence that does not name the admitted
performer's is no evidence about the attempt — another adapter's
attestation that nothing acted, or its documented idempotence, does not
authorize replaying this performer. An adapter that acted hands back
the outcome it recorded, as it recorded it — judged for whose it is
exactly as any reply is, the performer it names and the attempt it
names if it names one, never relabelled — and that is the attempt's
outcome, recorded and its claim closed; nothing is performed twice. An adapter
that attests it never acted, or whose effect is documented idempotent,
has the dispatch issued again (`effect.redelivered`, saying which).
An adapter that cannot say leaves the claim resulted unknown, durably
and visibly (`effect.result unknown (reconciled after a restart: …)`,
the existing rule-3 mark, appended exactly at the reading and under
the fence; the record moving under that append — an outcome, a
resolution, a takeover, or something unrelated — has the decision
taken again at the new reading, so a resolution that landed meanwhile
is the outcome and an unrelated move just lands the unknown row
after it), and the attempt waits — asked again it still waits, nothing
is replayed blind — until a person resolves it (`hale dna effect
resolve <key> --outcome ok|failed`); then the resolution is the
attempt's outcome, recorded so the Work settles on it. The assembly's
own restart rule, which results the other effect families by evidence
or marks them unknown, leaves an attempt's claim to the runtime's
adapter. One flat leaf; no exactly-once external execution is promised
and no compensation is invented: what is promised is that recovery
never performs an effect twice on its own say-so, and never pretends to
know what it cannot.

## Workflow execution: retries under a restart, and the fence

A retry is the next numbered attempt of a Work, and it is admitted only
after the record holds the outcome that permits it — a failed attempt
with allowance left, the allowance the recipe bound — as card 06's
rules have it; its request and performer kind are the admission's,
durable, so nothing is planned again for it. Under a restart the
retry is the record's: a reboot proposes attempt zero, is answered
from its recorded failure, proposes attempt one, which the record
already holds and replays, and asks for it — redelivered if the
incarnation that claimed it died — and never mints attempt two. A late
success for attempt zero is answered from its recorded failure and
changes nothing; the Work's current attempt is attempt one. An
admission the record refuses invokes no performer and spends no retry:
the Work holds it, with its number, and proposes it again when the
record resumes.

The runtime runs under a lease that is rows of its own record
(`lease.taken`, carrying the fencing token — a per-key epoch allocated
under the exact append that takes it — the holder and the expiry;
`lease.renewed`, which moves the expiry and keeps the token;
`lease.released`; each a JSON body, so a holder reads back as it was
written and an empty holder is refused): a key, a holder, the token
the holder was granted, and the clock it judges the lease by. The
token is the row's own, never its position: a routed reader that
starts fresh reads the record's rows before the ledger's and finds the
same lease a reader that was live found, and the lease is the take
with the highest token, renewed and released by rows that name it. Ownership is read at the revision a write is exact at, so the
fence is atomic with the write by construction: a takeover is a row, it
moves the revision, and a stale holder's exact append fails and is
decided again — at the new revision the record names the new holder,
and the write is fenced. That holds for every write the runtime makes:
a transition it commits, a claim, a redelivery, an outcome it records,
a claim it closes. A holder whose lease expired — unclaimed, re-taken
by another, or re-taken under its own name by a restarted incarnation
with a new token — commits nothing, however live it feels, and answers
`fenced`, which a resident holds as it holds a refusal by the record
and proposes again once the record resumes under a live token; a
request that only attaches to a running attempt writes nothing and is
not fenced. Two takes of a free lease at once land one: the other is
stale, decided again and refused. The cell leases (`MemLeases`,
`GitLeases`) stay for what they hold — a body, a mutation — where the
operation they fence is not an append to this record. A runtime with no
lease key is unfenced: a standalone runtime over a memory record.

## Workflow execution: edit outcomes

An edit leaf of a workflow execution is performed by the assembly —
its editor, its gateway, its record — never by a router performer.
The engine reaches it through a fifth performer kind, `edit`: the
`EditRelay` publishes the attempt (`EditRequested`, keyed by the
runtime's scope, which is the organism's id) and answers pending; the
assembly performs or waits, and reports the outcome for that exact
attempt (`AttemptReported`), which the runtime settles as it settles a
late reply — judged, recorded, answered to its Work and only its Work;
a report whose outcome names another attempt than the report does, or
none, is nobody's and settles nothing.
The Work settles as any Work does, its step counts it, and the
workflow settles the root: no Work of an admitted execution settles
its Task, and a Mutation's review outcome never settles it either —
one root terminal writer.

Two output contracts, never reinterpreted as each other. `Patch` — the
default for an edit leaf — is a candidate prepared for review: the
attempt is performed once, the Mutation it produces is bound to it
(card 14), and the Work is done when the candidate is in review,
escalated or released, failed when preparation failed before a
candidate; a later verdict changes nothing about it. `Applied` (or
`Approved`) is the candidate approved and applied: the Work performs
nothing and waits on one exact candidate: the Work its request names
in `target`, as `<step>/<key>` within the workflow (`s0/e`) or a
whole Work id, resolved to the Mutation bound (card 14) to the attempt
that Work's settlement accepted — a first attempt that failed to
prepare and was retried is not it. A Work naming none fails at once,
with why, since the dependency is explicit and never a Task-wide or
Work-wide guess; one whose producer settled other than done fails,
there being no candidate. The producer's settlement and its
candidate's review and application are prerequisites that land in
either order, and each landing is an occasion to look again: a
waiting attempt is evaluated afresh — by the same resolution — when
any Work of the engine ends (`RunLives`), when a review settles, when
the record resumes, on `redrive`, and whenever the assembly is asked;
it is answered once, when its state is an outcome. The same resolution
serves an ask again after a restart, which waits once. It is answered from that candidate's durable state
alone: rejected or sent back is failed; approved and applied (or
retained) is done; approved and rolled back is failed; approved but
not applied — the apply gate refused, an application pending or
unresolved — is no outcome yet, and the Work keeps waiting, to be
answered when the application lands, on a first approval or on an
approval again. An attempt asked again — a redelivery after a restart
— is answered from the Mutation bound to it and edits nothing twice,
so the edit kind's reconciliation is a replay. A request row the
record refuses starts nothing: the attempt is held, and `redrive` puts
it again. A published report is not a durable one: the assembly keeps
every report until the record is seen to hold the attempt's outcome,
and puts a kept one again when the record resumes or on `redrive` —
the validated outcome re-persisted, never the edit performed again.

## Workflow execution: human cases

A human leaf of a workflow execution is a case for a person. The
engine reaches the assembly through the `human` performer kind: the
`HumanRelay` publishes the attempt (`CaseRequested`, keyed by the
runtime's scope, the organism's id) and answers pending, and the
attempt stays out until the person reports the case done. The
assembly admits the case as its own handed Task, `<task>.s<i>.<key>`
— the Work's step index and member key under its Task, the child-Task
form of §4 — stable across redeliveries and naming its exact parent
Work and attempt by construction. The admission (`case.admitted`,
versioned and self-sufficient: parent Task, Work and attempt,
objective, assignee, obligation, acceptance practice, required
evidence, who said so) lands first, exact at the revision read, and
only then the rows the tooling of the day reads — `task.born <case>`,
then `task.handed <case>` (or `task.transfer_requested`, for another
owner's person) — so `hale dna task done <case> --as <who>` addresses
that case as it addresses a job today, and the same gate applies.
Assignee, obligation, acceptance practice and required evidence bind
as a job's do: in an organism the leader plans for, the leader says
who and under what obligation; without one the case is unassigned. A
retired assignee's successor, a transfer to another owner, and
reassignment keep their semantics.

What the leader proposed — whom, under what obligation — is one
thing; what binds is decided from the reading the admission is exact
at: who is admissible now (a retired person's successor), which
acceptance practice is in force now, whose the assignee is. A reading
gone stale under the append repeats the decision, not only the
write, so a retirement that landed between sends the case to the
successor, never to the person retired. The admission names its
origin (`owner`) and its disposition (`to`: the owner whose person
takes it, empty for the origin's own), and the compatibility rows say
what the admission recorded, never what the body writing them is.

The hand-off settles nothing: the parent Work stays pending, and the
root is neither closed nor marked by it. Two human Works in one step
are two cases, each completable on its own. An attempt asked again —
a redelivery after a restart, a later attempt of the same Work —
finds the case by its stable identity and admits no second, and asks
the leader nothing: the terms bound at admission stand, and a case
already awaiting the leader's word is not asked for twice. A restart
that fell between the admission and either compatibility row
completes them from the admission, in that order, and never plans,
resumes or pends a case as edit work: a case is a person's, never an
executable root, never legacy edit work. Only the recorded owner
completes them: a transfer to another owner is theirs to hand once a
member of theirs accepts it (`task.transfer_accepted`, as a job's
today), and another owner's restart neither hands it nor completes
the offer. An admission the record refuses holds the attempt for
`redrive`; a compatibility row the record refuses holds the case,
completed when the record resumes or on `redrive` without a restart,
and the hand-off is announced only once it is durable.

## Workflow execution: human completion

A case is completed by its person through the tooling of the day —
`hale dna task done <case> --as <who>`, with the evidence its bound
acceptance condition wants (`completion.linked`) or an exception
someone else authorized (`completion.excepted`), or a decision
someone else made, reported by the assignee and accepted under the
bound practice (`decision.reported`, GH #616) — and that accepted
fact is the outcome of the attempt the case was admitted for (card
17). The assembly reads the completion from the record
(`case_completion`, `completion.hl`) and reports it to the runtime of
its scope as it reports an edit leaf's outcome: judged, recorded as
`attempt.outcome` and answered to its Work by the runtime's own
barrier, so the Work settles done, the step counts it, and a
completion the record already holds an outcome for changes nothing.
The report is kept until the record is seen to hold the outcome, as
an edit's is.

The completion is judged against what the admission bound — the
assignee (moved by `task.reassigned`), whether evidence is required,
which practice — never against a practice in force later: a practice
relaxed after the hand-off relaxes nothing, while a case handed after
the relaxation closes under the new one; evidence linked under
another practice is none, and a decision accepted under another
policy than the bound one, or under none, or naming another case or
none as its scope, is none. An exception
applies to the person on the completion row: recorded in their name
and authorized by someone other than them — a case reassigned to its
exception's authorizer does not close under their own authorization.
A completion the bound condition refuses — not the assignee's, no
evidence linked under the bound practice where it is required, an
exception that is not the closer's or that the closer authorized, a
decision reported by someone other than the assignee — connects
nothing and is noted once; a later valid one connects. The first
valid completion is the outcome; a second is a duplicate and settles
nothing twice. One case's completion advances nothing of another's.

It is observed wherever the assembly reads the record: live, at the
organism's tick, which examines the cases whenever the record has
rows past the revision they were last examined at — by that
revision, never by whether the tick's own refresh moved, since
another handler (a redelivery's) reads the record without examining
every case; when the record resumes; on `redrive`; at a restart of
the assembly (which reads what the record holds); and at a restart
of the execution, whose redelivery asks for the attempt again and is
answered with the outcome the record holds.
A case keeps its parent step waiting through a restart of the
execution: nothing settles it but its completion. Only the recorded
owner connects a completion — the Work runs in the origin's
execution. The tooling's own gate (`hale dna task done`, evidence,
exceptions, reported decisions) is unchanged.

## Workflow execution: the baseline

What the sections above promise is held, as one oracle, by
`dna/tests/workflow_conformance_test.hl` over the canonical three-level
example (`dna/WORKFLOW-CONTRACT.md` §6), in every supported mode, with
a unix listen binding declared so every publish is queued as in a
bound organism. A review traces each promise to it; what it does not
cover is not promised.

**Delivery.** A leaf's attempt is admitted, claimed and delivered to
its performer once per claim. A performer answers at once or later;
a later reply names the attempt and the performer, and reaches the
runtime whenever it comes — after the grandchild's step, before the
root's own leaf, in any order. A reply for an attempt whose outcome is
recorded is answered from the record and writes nothing, whether it
repeats the recorded reply or arrives for an attempt the Work has
retried past. A reply from another identity than the admitted
performer's is nobody's and is refused. A reply for an attempt the
record never admitted or never claimed is nobody's.

**Failure.** A leaf that fails its allowance fails its step; the step
fails its execution; a child's failure fails the step that invoked it,
up to the root. Nothing later activates and nothing further is
requested. A leaf still out under a failed step keeps its
responsibility: its outcome is recorded when it comes, it reopens
nothing, and the execution above it settles failed only after it
settled (the drain policy, contract §6).

**Cancellation.** A root's cancellation fences everything below it:
what was admitted records its outcome; nothing further is admitted or
claimed at any depth; the tree drains and reclaims from the leaves up;
a late reply for a fenced attempt is still recorded, as the attempt's
outcome, and reopens nothing. A root's own residents learn of a
cancellation committed by another hand through a restart or through
the fence on their next transition, not by being told.

**Restart.** The record is the execution. An incarnation booted over
it — in process, a new runtime over the same git record, a new routed
record over the same two memories, or a new process — rebuilds only
what is unfinished, under the original ids: no execution is admitted
again, no step registers or activates twice, no completed member runs
again. An attempt claimed by the incarnation that died is reconciled
before anything is delivered: a durable adapter that recorded the
effect answers from its store and the attempt is not performed again;
a performer that truthfully has not acted is delivered the attempt
again, once, under the same id, and the record says so
(`effect.redelivered`). The completed record rebooted gains no row and
asks no adapter. Every fact of an execution is routed to the ledger
(one memory), so a reconstruction of the routed record keeps their
order.

**Fencing.** Under a lease, a holder whose token is not live commits
nothing: its replies are refused and written nowhere; with the lease
back, the same replies go through and the run completes as every other.

**Reclamation.** A settled execution dissolves every resident of its
tree, leaves first; repeated completed runs leave nothing alive. What
grows is the record.

**Logical exactly-once, apart from external-effect reconciliation.**
One `effect.requested` per attempt, ever; one `attempt.outcome`; one
`work.settled` on that attempt; one settlement per execution. Whether
the external effect happened once is the adapter's to say: an adapter
whose store holds the invocation is not asked again; one that cannot
say leaves the claim resulted unknown and the attempt waits for a
person (card 12c); one documented idempotent is replayed. The record
distinguishes the cases: a redelivered attempt has its
`effect.redelivered` row and, in the adapter's store, two invocations
under one id.

**The process boundary.** `dna/tests/conformance/runner.hl` is the
public assembly (`Dna.run_workflow` over a `GitJournal`) with a durable
service adapter whose store is a file; the fixture builds it with the
`hale` under test, runs it as a separate process, kills it with
SIGKILL the moment C1's attempt is admitted, the moment it is claimed,
and the moment its adapter has recorded the effect but before the
outcome landed, and runs it again over the same record each time: the
assembly resumes the admitted root, the attempt keeps its one
admission and one claim, the redelivery is recorded once, the store is
the outcome, D runs once, the root settles last. Run once more over
the completed record, it runs nothing.

## Storage interfaces

`Journal` (ordered append with an expected revision, read by index,
chain verification), `Coordination` (leases with fencing tokens) and
`Receipts` (content-addressed store and read) are interfaces in the
core. The git-backed implementations are the ones an assembly wires
for an organism; the in-memory ones exist for tests. Protected bodies
go through `MemoryVault` to memory's sealed store instead
(`ProtectedBodies`, `PqProtected`, which files, reads and erases only
through `receipt_file`, `receipt_read` and `receipt_erase`; GH #606,
#985).

**The record has an API of its own (GH #646 stage 0).** `Record` is
the one boundary the git-backed `GitJournal`, `GitReceipts` and
`GitLeases` and the host's sync, reconcile, body lease, identity and
exchange mailboxes all stand on, with a vocabulary that never names
the tool: *chains* of rows appended under compare-and-swap at a head
(`journal`; `exchange/<identity>` for a mailbox; `remote/journal` for
what the remote held at the last receive), a row *built* beside every
chain and swapped in later, *bodies* kept by the digest of their
content, *cells* swapped by version (a lease, the published identity),
*pointers* naming an object the repository already holds
(`candidates/<mutation>`, `revisions/<rev>`), *families* received from
and shared with a remote, the *signer* of a row, and the `dna.*`
settings. `GitRecord` is the one implementation over git plumbing —
the only file in the core and the host that spells `git` for the
record; the genome's own git in `workspace.hl`, `verification.hl`,
`org.hl` and the host's `genome.hl` is the Structure, not the record.
`MemRecord` is the one for fixtures. `hale dna init` and `upgrade` seed
the record through the host (`record-seed`, `design-upgrade`): the
driver keeps no record code of its own.

**A reload never shortens what a reader holds (GH #748).** The chain
only grows — a reconcile builds beside the ref and swaps in one whole
chain, so the ref never points at a record missing a row it held a
moment before — and a reader is held to the same rule. A chain that
reads back shorter than the rows a `GitJournal` already holds, or
empty, or with no head at all, is a read of the record that failed,
never a shorter record: `git` unable to run under load, the repository
unreachable for a moment, a tool that could not be forked. Every read
is made *before* a row is dropped, and one that comes back short
leaves the rows, the head and the text as they were, counts itself
(`reads_refused`, with `last_error` naming the read), and answers
`refresh()` with "nothing moved" — so the next refresh tries again,
and an append whose reload could not read the chain ends as the i/o
failure it is rather than re-offering a stale head two hundred times.
The view a reader holds is therefore stale at worst. It used to be
emptied first and refilled from those reads, so one failed read left
an organism holding no record at all, in silence, and everything it
derives from a scan of its own record began again from nothing: how
often a source has raised a concern, which Review is open, which Task
is in flight.

**A head read tells an absent chain from a read that failed
(GH #961).** `Record.head(chain)` answers a `Head`: `ok` when the read
happened, then `absent` for a chain that does not exist or `id` for
its head; a read that did not happen is `ok: false` with `why`, and no
caller takes it for an absent chain. `GitRecord` reads `absent` only
from `rev-parse -q --verify` exiting 1 with nothing on either stream.
Every caller that compares a head it read earlier — an admission
fence, an API response's final check, a writer's compare-and-swap —
refuses on a failed read with the reason (`record_unavailable`, or an
`io:` append error), never treating it as a Record that did not move.
A CLI verb says the record could not be read rather than that there
is none, `record-head` answers `none` only for a record known absent
(the driver seeds on it), and a receipt is not written again on a
read that did not happen. Both used to be the empty string. `hale dna
sync` refuses, with the chain untouched, when either head cannot be
read, or when a round of its reconcile reads either head behind where
the previous round left it; it pulls into an empty clone only while
the ref is still absent.

Every external dependency of the DNA is declared the same way, one
interface in the core with one implementation over the real thing and
one in memory: `Infrastructure` (a body's database, supervisor and
credentials; GH #647) and `Transport` (how a head reaches a body) in
`infrastructure.hl`, `Forge` (a code-review host; GH #648) in
`forge.hl`. Their memory implementations exist; the host still drives
compose, systemd, ssh and `gh` directly until each is moved behind its
interface.

## The organization

The organism is an organization written in Hale (GH #566 F2): a
program at `dna/org` that `hale dna init` generates and `hale dna run`
runs. The application it oversees is not modified by `init` and
contains none of it; it is observed like any Hale binary. The
manifest declares two environments, the application's and the
organization's (`[claims] no_base = true`; each adopts its own law).

- **Positions are loci; routing is the bus.** `Board` (the human
  authority: intent enters through it, escalations and reports leave
  through it, it owns every grant), `Leader` (a model-backed position
  holding the project's grant: it decides the Reviews inside the grant
  by reading the source diff and the semantic diff, and its verdict is
  a model call with evidence), and the substrate `Dna` (the record,
  the gateways, verification, the editing position, the Reviews). A
  Review is announced as a typed `ReviewRequested` fact carrying what
  a deciding position needs.
- **Authorities are ranked**: `board` (4, `maintainer` is its older
  name), `leader` (3), `supervisor` (2), `reviewer` (1). A claimed
  authority satisfies a requirement of its rank or below; an unknown
  name satisfies only itself.
- **Who decides.** `OrgPolicy`: a change that touches law, widens
  effects or crosses ownership, or is of class `organization`,
  `constitutional`, `process-policy` or `topology`, requires the
  Board; a change outside the grant (disposition `escalate`) requires
  the Board; everything else inside the grant requires the Leader.
- **Owners (stage B2, GH #664).** A shared record's org chart has
  one owner per position: the firm whose controller admits intents
  for it. The map is the genome's file `dna/org/owners` (`org =
  acme`, `org/collections = north`; `acme: alice, carol` names an
  owner's members), read by `Ownership` in the org program; a
  position not named takes its nearest named ancestor's owner, and
  one under no named ancestor is unowned — admitted by no one. An
  empty map is one owner, the organization itself, and nothing about
  a single-owner organism changes. A body over a shared record says
  which owner it is (`git config dna.owner`, carried as
  `HALE_DNA_OWNER`) or the host refuses to run it; it admits an intent
  (`Intent.to`, the position it is for; "" is `org`) only for a
  position its owner holds, refusing one offered to it for another's
  by name (`intent.refused`: "not this organization's to admit"). A
  head never offers such an intent to the body beside it: `hale dna
  task create --to` writes it to the record for the owner's controller, the
  host relays only `intent.requested` rows for positions its owner
  holds, and `status` lists the rest as `[unadmitted]` with the owner.
  Changing the map is a change to the organization approved by every
  owner it affects — an owner whose holdings or members differ between
  the current map and the candidate's — each through one of its
  members: the Review carries `approvers` (`acme=alice,carol
  north=bob`), a verdict from a member of no affected owner is
  refused, one rejection settles, and approval settles only once every
  affected owner has approved. Ids and effect claims carry their owner
  (GH #666): a controller mints in its own namespace — `acme:t3`,
  `acme:m2` — and claims an apply as `acme:apply:<candidate>`, so two
  controllers never contend for one id, each restores its count from
  its own births alone, and the ledger's unique constraint on a claim
  kind decides a first claim with no coordination between them: a
  `task.born` the Ledger answers `claimed` is minted again under
  the next id, and nothing is refused. With one owner nothing is
  prefixed. Work that crosses owners is a transfer inside the one
  ledger (GH #667), #615's rule applied between owners: a plan that
  hands a job to another owner's member appends `task.transfer_requested
  <task> {owner, to, assignee, …}` in place of `task.handed`, and the
  Task waits; a member of that owner accepts it (`hale dna task accept
  <id> --as <who>`, `task.transfer_accepted`, which memory takes only
  in a member of `to`'s name) and their controller then appends
  `task.handed` in its own name, naming `transferred_from` and who
  accepted, so completion is admitted in the assignee's name as
  always. Nothing settles on the request alone. Every owner runs a
  body over a shared record, and every body projects, carries out and
  erases, none coordinating (**The spine**). Each owner's heads write
  as their owner's role, which writes only in its owner's members'
  names (**The owners' roles**); who the person is remains the head's
  to establish — its issuer under `dna.principal = oidc`. With one
  owner, the head's role writes as any person.
  The map may still name `host = <owner>` (GH #669); changing it
  affects every owner, so every owner approves, and memory does not
  read it.
- **The foundational law** (`dna/org/law.hl`, generated, extendable,
  never weakened): nothing applies except through the substrate
  (`forbid reaches(positions, effects(genome_apply)) avoiding
  substrate`); the editing position reaches neither `repo_write`,
  `worktree_io`, `genome_apply` nor the Knowledge; the Leader's
  verdict reaches the genome only through the substrate; credentials
  are sealed. The claim engine follows bus edges, so a position that
  could reach an effect through a published fact is a build failure
  with the path as its witness.
- **Hosts.** `hale dna run` builds and runs the organization with iris
  attached; `hale dna dev` runs the application under the same host
  too, rebuilds and restarts it on an apply, records
  `expression.restarted` in the host's name, watches the window and
  reports on the nerves. The host writes `org.pid` and `app.pid`
  under `.hale/dna`. Under `run` alone a restart request is logged,
  not answered: expressing an application deployed elsewhere is a
  deployment gateway's job.
- **One body per owner: the body lease, and its epoch (GH #665).**
  A record admits one body per owner at a time — one body when the
  organization is the one owner (bounded attachment; the initial
  controller model). Over a shared record (the owners map above) the
  lease is a row of memory's `claims` table per owner, `owner/<owner>`,
  taken like the single body's `body` row, so two
  firms' controllers run side by side over one record and a firm's
  second host waits on its own firm's lease alone; a shared record
  runs no body until its ledger is adopted. The lease's token is an
  epoch: it rises when the lease is taken (a takeover, a forced
  claim, an expiry) and holds through renewals, and the host hands
  the body it starts its lease and epoch (`HALE_DNA_LEASE`,
  `HALE_DNA_LEASE_TOKEN`), which the body's `MemoryLedger` checks
  before every append: a write under a lease that is not held, is held
  at another epoch, or has expired lands nothing (`fenced: …`), so a
  controller that survived a partition writes nothing after its
  replacement took over, whatever it still believes. A head's or a
  host's write is a request, which the spine admits. Failover is within an owner; no
  owner's positions pass to another on a timeout. The rest of this
  entry describes the lease as one body's; it holds per owner. A record
  admits one body of each. The
  host takes `refs/dna/lease/body` before it builds or runs anything
  — at the record's remote when it has one, so two hosts on two
  clones fence each other through the remote itself (the lease blob
  is pushed with `--force-with-lease`, git's compare-and-swap), in
  the clone otherwise. The blob is a `GitLeases` lease
  (`holder\ntoken\nexpires\npresent`), the holder
  `user@host:<clone>`, so the same clone restarting is the same
  holder and takes its lease back at once; a second clone is refused
  with who holds it and exits 3. The lease lives 30s and is renewed
  when a third of that is gone; the host **asserts it at the top of
  every tick, before it relays, restarts or applies**, and stops
  itself (exit 3, the organization with it) when the lease is
  someone else's or released. It proves the lease again, renewing it,
  **the moment before it starts a process** — the organization and
  the expression at startup and at every restart — and once more
  before relaying after the tick's sync: a build or a sync that
  outlasts a takeover starts and relays nothing, and the host exits 3.
  **The renewal and the fence do not wait on the host.** Beside the
  host runs the body fence (the host binary's `body-fence`), which
  renews the lease every 10s with every git call bounded to 8s — and
  to the deadline it last proved (the lease's expiry less 5s,
  `.hale/dna/body.deadline`), so the calls of one renewal together cannot
  outlast the lease — and
  writes what it proved to `.hale/dna/body.fence.status`, each line
  prefixed with its host's pid; the host reads its own lines there
  instead of renewing. The fence's mandate is `.hale/dna/body.fence`,
  naming its host's pid: a fence whose mandate is gone or names another
  host (a host restarted on the same clone) stops and kills nothing. When the lease is someone
  else's or released, when the fence cannot prove it within 5s of its
  expiry, or when the host is gone, the fence kills the organization
  and the expression (by their pid files), each with every process it
  started — the tree and the **session** frozen, then killed. Every
  process the host starts begins a session of its own
  (`dna::in_new_session`: `setsid(1)`, or perl's `POSIX::setsid` where
  there is none), and a tool keeps its session through everything a
  process group does not survive: `run_tool` gives each tool a group of
  its own, and a tool whose organism died is reparented, but both stay
  in the session its organism began, and only a tool that calls `setsid`
  itself leaves it (#970). Then every process carrying the body's mark
  (`HALE_DNA_BODY=<holder>#<token>#<host pid>#`, set in the environment of
  everything the host starts and inherited by every tool they start) —
  the second net, for that `setsid` tool, where the machine discloses
  environments — and says why (the
  host does the same, before it stops the fence, on every exit it takes
  once it holds the lease: a failed build, a refused start, its
  organization's exit), and the host
  exits 3 when it next looks. **The session reaches a reparented tool on
  every POSIX machine; the mark needs the machine to disclose another
  process's environment, and what a machine will give differs (#638).**
  `body_scan` says which it has: `proc` — Linux's `/proc/<pid>/environ`;
  `ps` — `ps -E`, proved at the time of asking on a child of the call's,
  since the answer is what the kernel discloses about another process
  and not whether the flag parses (macOS shows a process its own
  environment and, with SIP, no one else's — GitHub's runners do show
  it); or none, where the fence still stops the sessions — every tool
  the organization and the expression started, orphaned or not — and
  only a tool that began a session of its own is out of reach, which the
  host says at startup (#970). A host blocked in a sync, a build or an
  observation window therefore cannot keep its organism executing past
  the lease; while the remote cannot be reached the lease is kept
  unrenewed until then: a partitioned body executes nothing past its
  TTL. Taking the lease
  is a row (`body.claimed <holder> {token, forced, by}`), giving it
  up at exit another (`body.released`). `hale dna body` reads the
  lease; `hale dna body claim --force` releases a live lease that
  belongs to a body that is gone, as a `body.claimed` row with
  `forced: true` in the forcer's name (that body stops when it next
  asserts; the next `hale dna run` takes the lease); `hale dna body
  release [--force]` releases it explicitly.
- **The combination is detected, never stored.** An organism is one
  point in a space of independent capabilities — where the body
  runs, whether a hosted head exists, whether a fleet expresses it,
  which records it is connected to — declared by its pieces. `hale
  dna profile` prints one line per axis from the pieces alone: the
  record's remote, the body lease and its last tick, `[dna] fleet`,
  the knowledge store's environment, `dna.trust`, `dna.github`; the
  status projection carries a `profile:` and a `body:` line. `hale
  dna new --profile local|remote-body [--remote <url>] [--body
  <user@host>]` sets pieces (a remote, `dna.body`); the profile name
  is not written anywhere. Profiles are examples of combinations,
  not a closed set; the toolchain refuses only what breaks a
  commitment (a second body on one record).
- **The body on a server.** `hale dna body provision <user@host>
  [--dsn <url>] [--dir <path>] [--dry-run]` makes a body over ssh, in
  order: the toolchain `hale.lock` pins (installed with the site's
  installer at that version, verified, or stop), the record's remote
  cloned (fetched into a clone that exists), `vendor/dna` and the
  record brought up, memory's database from `dna/compose.yaml` or a
  DSN (which goes into the body's env file as
  `HALE_DNA_MEMORY_DSN_OWNER`, never the record: the unit's `hale dna
  dev` migrates with it and hands the host only the spine's DSN),
  and a systemd user unit `hale-dna-<project>-<record>` supervising `hale dna
  dev . --no-iris` — on one server the body is the organization and
  the application under one host — with
  `Restart=always` (GH #1026): a failure flows up one more level, and
  a node that exits to take a moved genome is started again on it
  (**The genome pull**). It stops
  before writing anything when the record has no remote or one local
  to this machine, when ssh cannot reach the host, or when the host
  lacks git, curl, systemd, or (without a DSN) docker compose. It
  records `dna.body` and `dna.body.dir` here and a `body.provisioned
  <user@host> {dir, toolchain, knowledge, by}` row. The default
  directory is `$HOME/dna/<project>` on the body, expanded by the body's
  shell (the unit's `WorkingDirectory=%h/dna/<project>` is the same
  place). The script can carry the DSN, so its file here is created
  empty with mode 600 before it is written, and removed once ssh has
  read it. `--dry-run` prints the exact script. `hale dna body
  start|stop|logs [--body …]` reach the unit over ssh; `stop` succeeds
  when the unit is no longer active, and says so. Postgres, Docker, ssh and systemd are the
  reference setup; the definition admits other implementations, and
  names where they plug in (GH #647): `Infrastructure` — a body's
  knowledge database, its supervisor (a unit installed, started,
  stopped, asked, read) and its credentials (put by name, never read
  back) — and `Transport` — how a head reaches a body (`probe`,
  `execute`) — are interfaces in the core. `ReferenceInfrastructure`
  in the host is compose, a systemd user unit and the env file, reached
  through `SshTransport` (`HALE_DNA_TRANSPORT=ssh`, the default) or
  `LocalTransport` (`local`: every script runs in this machine's shell
  under `HALE_DNA_TRANSPORT_HOME`, which is how the fixtures exercise a
  body with nothing stubbed on PATH for ssh); `MemInfrastructure` and
  `MemTransport` are the core's fixtures. The provisioning script is
  composed from the database's and the supervisor's own fragments, so
  a body without systemd gets another supervisor's unit by
  implementation, not by `if`. Nothing in the host spells `ssh`,
  `systemctl`, `journalctl` or `docker compose` outside that one
  implementation.
- **Secrets.** `hale dna secret set <NAME> [--body <user@host>]`
  reads the value from stdin — never argv (a `NAME=value` argument is
  refused), never the record — and writes `NAME=value` into
  `~/.config/hale-dna/<project>-<record>.env` (mode 600; one line per name,
  the newest) on the body over ssh's stdin, or on this machine. The
  host loads that file into its children's environment. `secret
  rotate <NAME>` is the same for a name already set. The record gets
  `secret.rotated <NAME> {where, by}` and nothing else. At start the
  host checks the credentials its catalog names (`env_var` in
  `dna/org/models.hl`): when none is set in its environment or the
  file it says so and appends `body.credential_missing model
  {any_of}`; `status` and `board` carry "no credential for the
  model" until a start finds one and appends
  `body.credential_present`.
- **What a body holds on a machine is bound to its record (#635).**
  `<record>` in the unit's and the secrets file's names is the first
  twelve hex digits of the record's identity (its journal's genesis), so
  two records whose directories share a name on one machine never share
  a unit or a credential. Everything else a body reads lives in its
  clone: the `dna.*` git configuration (the OIDC member mapping among
  it) and the knowledge schema, whose ownership is checked against the
  record (GH #613); the hosted head and the node serve the clone they
  run in.

## The organization evolves

A change of class `organization` edits the organization's own seed
(`Mutation.seed`, `dna/org`), through the same pipeline as a change
to the application: a worktree, the editing position confined to that
seed, verification of that seed (its own base artifact cut at the same
moment), the semantic diff of the organization (positions are loci,
routes are subscriptions and publications, capabilities are effect
classes), a Review that is the Board's. Applying it restarts the
organization (the restart request names the seed; the host answers
one for `dna/org` by rebuilding and restarting the organization
itself, which records `expression.restarted` at birth); the window
then judges the new organization.

Growth is initiative, not reflex: when one source raises pressure
`appendage_threshold` times, the assembly journals
`appendage.proposed` and — with `initiative` on — proposes a growth
Mutation of the organization's seed (`appendage.candidate` names
it). Nothing is grown until the Board approves the candidate commit.

`hale dna board` is the Board's queue: the Reviews only it can settle,
the proposals, the last report. `hale dna report` appends
`report.filed`, in the name of whoever asked, summarizing the record
since the previous report (proposed, reviewed, applied, retained,
rolled back, rejected, escalated, pressure, proposals, model calls and
cost, settlements). `hale dna pressure` lists the pressure raised and
answered; `hale dna pressure raise <source> <what>` appends one
`pressure.requested` row, which a node relays on `PressureRaised`
(`dna.pressure.raised`) until the organization answers it.

## The editing position

`SourceEditor.perform` plans the files an objective is about — every
listed file the objective names, else the quick tier's plan from the
listing, one file per line; only a listed file is ever a target —
edits each (one model call per file, the request naming the file as
`target`), formats and checks the seed, and, when the check fails,
tries again with the diagnostics in the prompt, up to `max_tries`
in all (default 3). Each try is an attempt id (`<work>/a<n>`), so
every try's model calls are evidence in the record. The result names
the files changed and the tries taken; a proposal that does not check
within the bound is a failed Attempt with the last diagnostics.

## Models

The organization's models are a catalog in source (GH #583 M1):
`dna/org/models.hl`, generated by `init` and owned by the project.

- **The seam.** `ModelBackend` (`identity`, `model_name`, `allows`,
  `capacity_bytes`, `complete`) is what every adapter implements;
  every call publishes `ModelCalled`, which the substrate journals as
  `model.called` with the time of the call (`at`). **The prompt is a
  receipt (GH #611):** the evidence carries the prompt and the context
  as sent, and the substrate files each as a receipt in the record
  (`refs/dna/receipts`) under the `prompt_digest` / `context_digest`
  the row names — only a body that
  hashes to its digest, and receipts are content-addressed, so an
  identical brief is stored once — recording `bodies: stored` (or
  `none` for a call with neither). An organism whose receipts are not
  the record's (a file store: a fixture, an embedded use with no
  record) files nothing and records `bodies: none (receipts are not the
  record's: file)`. A `customer`-class call files no
  body: `bodies: withheld (data class customer)`, and its digests are
  all the record keeps. `hale dna history <attempt>` renders, under each
  `model.called` row of that attempt, `prompt <digest> (<n> bytes)`, exactly
  those bytes from the receipt, and `end of prompt` (the same for the
  context) — never a re-composition; a withheld body says so, and a
  body this clone lacks says `hale dna sync` fetches it. A wider
  history (`hale dna history m1`) names the calls and leaves their
  bodies to each attempt's own, since an editor's prompt carries whole
  files. The row says `stored` only when the store gives the body back.
  Routers return
  data, never a backend. Adapters: `OpenAiChat` (the OpenAI chat
  shape: OpenAI, OpenRouter, vLLM, Ollama; `adapter: openai-chat`),
  `AnthropicMessages` (the native Messages API: `system` beside
  `messages`, `max_tokens` required and defaulted to 4096, the reply's
  text blocks joined, `anthropic-version` sent; `adapter:
  anthropic-messages`), `LocalModel` (the OpenAI shape to this
  machine; no credential, no `external_model`, any data class),
  `FakeModel` (scripted: `answer`, `answer_file`, `answers_dir`,
  `answer_role`; `fail_after` refuses after that many calls). A hosted
  adapter's `complete` carries `external_model`; its credential is a
  sealed `HostedCredential` that names its source (`env_var`) and its
  `scheme` — `bearer` (an `Authorization: Bearer` header) or
  `x-api-key` — and never returns the bytes: the credential composes
  the header, no adapter does. An API error is refused as `http
  <status> <the API's message>`; a transport failure as `http 0
  <kind> <detail>`.
- **The harness.** `HarnessModel` runs an installed coding harness
  (`command: claude`, or `codex` with `output: text` and `exec
  --skip-git-repo-check --sandbox workspace-write`, because a
  non-interactive codex is read-only by default and would answer
  without ever editing its export) per request with
  its own tools on, as a backend that `works_in_place`: for a
  source-editing request the editor makes an EXPORT of the Mutation's
  worktree (a plain directory with the same files, never `.git`, never
  `.hale`), runs the harness with the export as its cwd and the whole
  objective as its prompt, and imports the export's diff back under
  the grant — a file that differs or is new is written through the
  tools, one that is gone is removed, a change outside the grant is
  counted (`outside_grant`) and left behind; `files_changed` is
  derived from that diff, never from the harness's answer. A file the
  worktree does not have is new whatever it holds, empty included. A
  written file's parent directories are made under the grant first, so
  a harness may add a module in a directory the worktree does not have
  yet, and **an in-grant write or removal that fails is the attempt's
  failure** (`the import was incomplete: …`), never a candidate
  carrying part of the change. Then the
  same fmt, check, retry (the diagnostics in the next prompt) and
  assessment as the file-by-file flow. An answer-only request (a
  review) runs in an empty directory of its own. `complete` carries
  `external_model` and `harness_run`; evidence is one `model.called`
  row per call with the harness's own cost summary
  (`total_cost_usd`, `usage`), `tool_grant: harness @export`, and
  `confinement=<kind>` in `params`.
- **The boundary is stated exactly.** The export is not a sandbox: a
  process whose cwd is the export can still reach whatever the
  operator's account can. What DNA guarantees is that *the genome is
  unreachable from the harness process*. A `Confinement` (`kind`,
  `available`, `wrap(argv, cwd, mask)`) supplies it: `Bubblewrap`
  (Linux: the filesystem as the operator sees it — the harness's own
  home state, the toolchain, the network — with the repository and
  the worktree replaced by empty tmpfs mounts, `--die-with-parent`)
  or `NoConfinement`. **The genome is the model's own, not the
  request's**: `HarnessModel { genome, genome_env }` reads it at birth
  (the host exports `HALE_DNA_GENOME`), every call masks it, and a
  request's `mask` adds what that call knows besides — the worktree an
  editor exported. Every verb the host runs is given
  `HALE_DNA_GENOME`, not only the organization under `run` and `dev`:
  `hale dna models` probes the catalog in a process of its own, and a
  confined harness with nothing to mask is refused. An answer-only role (a review, a classification,
  the catalog's probe) carries no request mask, so a genome on the
  model itself is what makes the guarantee true for them. **A
  confinement with nothing to mask is refused**, not run, so the claim
  and the fact cannot diverge. A harness whose confinement is
  unavailable is refused (`unconfined harness not allowed`) unless the
  org chart's `allow_unconfined` says otherwise; the evidence records
  which it was, and `masked=<n>` how many paths the boundary covered. On every platform the assembly also checks that the
  repository's head and the worktree's head did not move during the
  attempt and fails the Mutation (`mutation.failed`: `the genome
  moved during the attempt`) if they did: not a boundary, but a
  bypass becomes a loud refusal. Outside the genome the harness has
  exactly the operator's account, which is what running it by hand
  has. The `Repository` interface names its `root` for the mask.
- **The tape.** `RecordedModel { dir, mode, inner }` wraps any
  backend (`dir_env` / `mode_env` name environment variables it reads
  into `dir` / `mode` at birth, so a catalog function computes no
  string: dna/FRICTION.md F.17). The key is `sha256` over the fields that identify a
  request — role, the inner's name and model, the prompt and context
  digests, the data class, the grant normalized (`… @grant`: its path
  is where it ran, not what it was) and, for a backend that works in
  place, a digest of the workspace's starting tree. An identical
  request made again in one run — a retried Work planned a second time —
  is keyed by its occurrence as well (`occurrence=n` for the n-th; the
  first keeps the plain key), so a recording that got two answers
  replays both in order, and a miss on a later occurrence says it was
  never recorded. `record` makes the
  tape's directory before it writes anything into it, and an entry is
  counted only once every file of it has landed: a patch or an entry
  that could not be written is a refusal (`cannot write the tape: …`),
  because an entry claiming a patch that is not there replays as a
  miss on a request the tape appears to hold. `record` forwards
  to `inner` (whose evidence is the call's) and writes `<key>.json`
  with every keyed field in clear and the answer, and for a workspace
  `<key>.patch`: what the backend changed there, before any import,
  so a replay reproduces the edit and everything after it runs for
  real. `replay` answers from the directory (evidence `adapter:
  recorded`, `params: tape=<key> mode=replay`), applies the patch to
  the workspace, and refuses a miss naming the request and, when an
  entry shares the prompt digest, the fields that differed (`tape
  miss <key> (role …, backend …, …): nearest <key> differs in:
  context_digest tree`). Replay is pure; record reaches whatever
  `inner` reaches. Anything that varies between runs must stay out of
  the key, and recording finds what does: the editor therefore strips
  the worktree's path from the diagnostics it feeds back. The tape is
  a checked-in fixture, not a store. It proves how the organization
  handles recorded outcomes; model quality is only ever tested by a
  fresh run.
- **The fixture.** `dna/acceptance/trio`: a gateway (the root seed),
  an api and a worker under a plan of four instances on two nodes
  with routes between them and claims across them
  (`require_publishes` on the gateway and the api, `forbid_reaches`
  gateway → worker avoiding the api). Its organization, with every
  backend a `RecordedModel` over the real adapter
  (`dna/acceptance/trio.fixture/catalog.hl`; mode from
  `HALE_DNA_TAPE`, the tape from `HALE_DNA_TAPE_DIR`), is driven by
  scripted asks end to end: a change to the gateway approved and
  deployed to both nodes, the organization grown by a supervisor
  under pressure from the worker and restarted with it, and a change
  to one service that still checks but breaks the fleet's claim
  denied — every model call answered from the tape, keyless. It is
  the acceptance for every later change to the organization;
  `HALE_DNA_TAPE=record` with a key re-records it.
- **The catalog is source.** A backend is a constructor function
  (`frontier()`, `fast()`, `desk()` …); a position's router is a
  function composed from them (`leader_models()`, `editor_models()`,
  `agent_models()`); the organization's main takes each position's
  router from the catalog and names no adapter inline. A new position
  is one more function; a provider switch is one file; `hale check`
  validates it and the law keeps the concrete types at every
  boundary.
- **Discovery.** `init` and `new` look at the environment
  (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) and `PATH` (`ollama`, whose
  first listed model becomes the desk model; `claude`, else `codex`,
  becomes `harness()`), write the catalog for what is there
  (`AnthropicMessages` with `claude-opus-5` / `claude-haiku-4-5` when
  its key is present, else `OpenAiChat` with `gpt-4o` /
  `gpt-4o-mini`, a placeholder key when none is set; with a harness
  the editor's and the agent's `quick` tier is the harness, and with
  no key every model-backed slot is), and print what each position
  was given. Nothing found is fine: a hosted backend without its key
  is not permitted, and every Review waits for the Board. `upgrade`
  writes a catalog for an organization that predates it and says what
  to point at it; it never edits the organization's main.
- **The budget has one owner.** The catalog declares `org_budget()`,
  a `BudgetPolicy` (`window`: `day`, `week` or `none`;
  `allowance_micros`, 0 = unmetered) as data. The substrate owns the
  one `Budget`: every `model.called` row is accounted at its time, the
  counters are rehydrated from the record at birth, and when a window
  is exhausted intent is refused (`intent.refused`: `budget
  exhausted (spent … of … micro-dollars this day in … call(s))`),
  `budget.exhausted` is journaled once per window (`detail`,
  `spent_micros`, `allowance_micros`, `window`, `at`), the membrane is
  told, and a Review is not announced to the model-backed positions —
  it waits for the Board, which needs no model. A `ModelRouter` counts
  what its calls cost and has no allowance of its own.
- **Tokens per task (GH #946).** The attempt id on a `model.called`
  row is the tag its cost is attributed by: `<ask>/plan` to the task
  born of the ask (the `task.born` row whose body starts `<ask>: `);
  `<work>/a<n>` to the Work's task — a wf1 attempt's admission names
  the task and the performer kind, a Mutation's request names the task
  (the editor's; an older record says it in the proposal's `task <id>`
  summary); `<review>/review` to the task the Review's request names,
  or its Mutation's, or `knowledge` for a knowledge Review
  (`k:<12hex>`); `optimize/<n>` to `org`. The probe leaves no row. A
  call of no known shape is its own and no task's. The projection
  (`dna/operations/usage.hl`, `Usage`) sums calls, input and output
  tokens and `cost_micros` under an id — the attempt itself, the ask,
  Work, Mutation or Review it is of, or the task it serves and every
  task under it (by `workflow.admitted`'s `parent_task`) — by position,
  keyed by the graph's id for it, `position:<name>` (`position:leader`
  for a plan, a verdict and the optimize pass; `position:editor` for a
  Mutation's attempts; `position:<performer kind>` for a wf1 attempt,
  until an attempt's claim names the position that took it) and by
  `<backend>/<model>`. `hale dna history <entity>` prints `usage of
  <entity>: <calls> call(s) · <in> in · <out> out · <cost> µ$` and the
  two breakdowns under the `history of` line, nothing when no call was
  made, and `usage: …` for the whole record with no entity; the API
  carries the same sums as `usage` on every execution, each of its
  attempts and every administered Task. The editor's locate and plan
  calls carry the Work's attempt id (the tape key excludes it).
- **`hale dna models`** builds the catalog beside a one-line main in
  `.hale/dna/probe`, runs its `probe_catalog()` from the project root
  with the organization's environment, and prints one line per
  backend: the catalog name, the router-facing slot, the model, and
  the answer — `ok`, elapsed, cost and the first line of the reply;
  `refused: …`; or `not permitted (no credential present)`. The
  organization is not started, the nerves not touched, and nothing
  is journaled: a probe is nobody's Attempt.

## Knowledge

The knowledge graph has two halves and one authority (GH #583 K1).
The live half is memory's, projected from the record by the spine
(GH #985).

- **The record holds the decided half.** A proposal is a document —
  canonical JSON with a fixed field order (`kind`, `text`, `author`,
  `target`, `provenance`) — filed as a receipt under its sha256, so a
  digest in the record always resolves to recoverable content.
  `Dna.propose_knowledge(idea, target)` files the receipt, appends
  `knowledge.proposed <digest>` (body: `digest`, `review_id`, `kind`,
  `author`, `target`, `class`, `at`), and births a Review pinned to
  that digest (`review_id` = `k:` + the digest's first twelve hex
  digits; `required_authority: board` — ratified knowledge is the
  organization's; the `review.requested` body carries
  `knowledge_digest`). The tower rule classifies the binding from who
  proposes and where it binds — a parent's idea bound to a child is a
  `goal`, a child's bound to a parent a `concern`, one's own an
  `initiative` — and a lateral proposal (siblings) is refused with
  `knowledge.refused` and no Review. A verdict naming another digest
  is refused by the Review; a Leader's does not satisfy the Board's
  requirement. **The authoritative event is `knowledge.ratified
  <digest>`**, appended by the assembly on `review.settled` with
  `approve` (body: `digest`, `review_id`, `outcome`, `decided_by`,
  `kind`, `target`, `class`, `at`); any other outcome appends
  `knowledge.declined`. A pending knowledge Review is re-born from the
  record at birth like a mutation's.
- **Binding changes preserve the item.** The reviewed
  `dna.knowledge.binding.bind@1` and `.unbind@1` operations address one
  `(idea, target, author)` tuple. Independent explicit grants admit a
  `knowledge.binding.requested` fact. The Body records delivery before
  creating the canonical `dna.knowledge-binding-change/1` candidate and
  its exact Review (`knowledge_binding_digest`, not `knowledge_digest`).
  The candidate pins the original command, grant and latest Record
  directive for the tuple. Approval produces `knowledge.binding.bound`
  or `.unbound`; an intervening tuple change or newly inactive bind
  subject produces a separate apply refusal. Each consequence belongs
  to the original request identity. Projection changes future package
  applicability, preserving the item receipt and earlier Work packages.
  A visible retired item permits exact unbinding. This is the reviewed
  command profile the API admits (**The API**, under *The surface*),
  not a universal Review requirement on the in-process
  `Knowledge.bind` primitive.
- **Relationship policy selects direct or reviewed admission.** The existing
  `dna.knowledge.edge.link@1` and `.unlink@1` commands retain their exact directed
  `(from, to, rel)` identity and command encoding. A `review` grant instead admits
  `knowledge.edge.requested`, pinning the latest direct or reviewed directive for
  that tuple in a canonical `dna.knowledge-edge-change/1` candidate. Its Review
  carries `knowledge_edge_digest`; the candidate is not a Knowledge node. The
  Body derives ownership from both endpoint receipts and requires its owner to
  admit all four author/target positions in this reference profile. An independent
  approval produces `knowledge.edge.reviewed_linked` or `.reviewed_unlinked` only
  while the pinned tuple basis and endpoint eligibility still hold. A changed
  basis or ineligible endpoint produces a separate apply refusal. A retired,
  ratified endpoint permits exact unlinking. Recovery follows the original
  admitted variant even if policy later changes mode. Native effects replay in
  Record order; approval alone does not establish a graph change. This policy
  does not add a universal Review requirement to the graph primitives.
- **Memory holds the live half.** `dna/core/memory_store.hl` declares
  `KnowledgeStore`: `scope` / `record`, `open`, `healthy` / `reopen`,
  `watermark` (the next record seq to apply), `begin_row(seq, prev,
  head)` / `commit_row` / `abort_row` (one record row's transaction,
  below), `rebuild(watermark, head)`, `upsert(idea, ratified_seq)` (idempotent by id, which is
  the digest), `retire`, `bind`, `unbind`, `link`, `unlink`,
  `idea(id)`, `context_ids(target, budget)` (accepted ideas bound to
  the target or any prefix of its path — goals flow down, initiatives
  stay local — in ratification order), `ranked_ids`, `count(what)`, and
  the structure and signals of K3, and the repository's graph (below).
  `Pq` is Postgres (nine tables: `knowledge_meta`, `knowledge_ideas`,
  `knowledge_bindings`, `knowledge_edges`, `knowledge_structure`,
  `knowledge_signals`, `graph_nodes`, `graph_edges`, `graph_members`,
  which the migration creates and `open` only checks; node writes are
  upserts and removals delete exact tuples; rows come back as JSON
  built by the server, since the driver's tab- and newline-separated
  rows cannot carry an ordinary paragraph; a signal counts once per
  record row, keyed on that row's sequence). **The projection is a
  consumer of the record**: `apply_record(store, journal, receipts)` in
  `dna/operations/memory_tail.hl` walks the record's rows from the
  watermark, resolves each knowledge digest to its receipt, upserts
  (`ratified` accepted with its binding; `proposed` and `declined` kept
  as such, never over a ratification). **Each record row is one
  transaction (GH #1026)**: `begin_row` moves the stamp first — the
  watermark from `n` to `n + 1`, then `graph_projection_head` from row
  `n - 1`'s digest to row `n`'s, each by compare-and-swap on its
  `knowledge_meta` row — then the row's effects, and `commit_row` lands
  them together; a failed effect lands nothing (`abort_row`), and the
  next pass begins the row again. A projector whose compare finds the
  stamp moved rolls back having written nothing and ends its pass
  (`yielded`): it blocked on the watermark's row while another
  projector held it, and that projector committed the row. So any
  number of projectors may apply one record at once, with no
  coordinator, and converge on one graph, each row applied once; after
  a crash a projector resumes at the stamp. Nothing in memory becomes
  ratified except from a `knowledge.ratified` row: git is the sole
  authority; memory can lag, never disagree. The observed and inferred
  tier (K3) lives only in Postgres and is backed up like any Postgres;
  the ratified tier rebuilds from git.
- **The record is the scope of its memory.** Memory is opened for one
  record, named by the record's identity — the sha of the journal's
  first commit, the same in every clone of the record and different
  for every record: that first commit carries a `Record-Nonce:` of 16
  random bytes, so two projects created alike in the same second (same
  author, same scaffold, same row) are two records, not one
  organization to memory and the nerves (GH #1068); a record that
  cannot draw them writes no first row. A record made before keeps its
  identity (`GitJournal.genesis()`; `scope(record)` on the
  store before `open`, `record()` to read it back). `Pq` keeps one
  Postgres schema per record, `dna_<identity>`, selected for the
  session at `open` (with `public` behind it for the vector type), so
  two records on one database each see only their own ideas, bindings,
  structure, signals and watermark; `hale dna dev`'s compose database
  is one record's anyway. The schema's `knowledge_meta` carries a
  `record` row naming its owner, written by the migration and checked
  by the migration and by every `open`: a schema that names another
  record is refused (`scope: schema … belongs to record …, not …`),
  never read. A record with no rows yet has no identity: its projection
  opens under `dna_unscoped`, where nothing is ever applied, and moves
  to the record's own schema once its first row lands.
- **The spine projects; readers read.** `MemoryProjection`
  (`dna/operations/memory_tail.hl`) applies the record into the graph
  on the host's tick, as the record's spine role, on every node (**The
  spine**, above), so the projection advances without anyone asking. **The fence compares heads by ancestry**:
  memory's stamp is a commit of the record, so every clone can place
  it. Nothing stamped, or the stamped head is this clone's row at the
  stamp: the rows after it are the delta. This clone's head an ancestor
  of the stamped head, or the stamped head a commit this clone has not
  received: another projector is ahead, and there is nothing to project
  here until the record is received (a head a clone cannot place is
  never taken for a replaced chain, or two nodes that append before they
  share would rebuild each other's projection until they synced).
  Neither, of a head this clone holds: the chain was rebuilt
  beneath the projection (a reconcile re-appends local rows with new
  digests, GH #603), so the graph is rebuilt from row 0 —
  `rebuild(watermark, head)` empties it only while the stamp is still
  the one read, so of several projectors that see one replaced chain
  one rebuilds, and a projector rebuilds at most once per pass. **A question memory cannot answer is a
  refusal, never an empty answer**: a package built from failing
  reads, or from a projection behind the record, is indistinguishable
  from "there is no knowledge here". The projection's store is opened
  on the first tick rather than at birth, and the connection is asked
  on each tick afterwards (`healthy`), because an open that succeeded
  once is not a connection that still answers: a session dropped by a
  restart or a failover is re-established (`reopen`). A reader —
  `MemoryKnowledge` in the organism, the API as a head — never
  projects. A package (`MemoryKnowledge.package_for(target, query)`:
  the ids included, their ideas with kind, text and author, the
  projection's watermark as its revision, and a digest over the target
  and the ids — what is handed over, never the revision, which varies
  between runs and would make a tape unable to answer the same request
  twice) waits, bounded at 20 s (`PACKAGE_WAIT_SECS`), for the
  projection to pass the record's last row that changes a package (an
  idea ratified or retired, a binding made or undone), and otherwise
  refuses: `memory's projection is at row <n>, before the record's
  knowledge row <m>; the spine applies it on its tick`. The organization's knowledge package is read from memory
  under the spine role. The API reads the graph under the head's role
  (**The API**, below).
- **The design, as proposals (GH #596 C).** `init` seeds the
  toolchain's practices about how a DNA organization works — the
  design principles, the evolution pattern, structure follows intent,
  standard equipment, the structural signals, signaling, the optimize
  pass, software delivery — as `knowledge.proposed` rows with receipts
  bound to `org`, **one Review per practice** (`k:<12hex>`, grouped
  `design` in `hale dna review`'s listing; `hale dna review design
  approve|reject` is a convenience that decides each pending one in
  turn, never a batch object in the record). A Review pins one digest
  and settles with one outcome, so the Board decides practice by
  practice, and nothing is ratified by the toolchain. Each receipt
  carries a stable `name` (`design/<slug>`) and the toolchain version.
  **Supersession is the Board's decision, not a side effect.**
  Declining a proposal retires nothing (the projection keeps a
  ratified idea a later `knowledge.declined` names). A proposal whose
  receipt carries `supersedes: <digest>` retires that digest when it
  is ratified — the assembly appends `knowledge.retired <old>` — and
  leaves it in force when it is declined; a `retirement` proposal
  carries only that. A replacement is ratified only while what it
  replaces is still active: an approval of a proposal whose
  `supersedes` names a digest already retired is refused
  (`knowledge.refused <digest>`, with what retired it) rather than
  ratified — otherwise two replacements of one version would both be
  served, the first never retired. `hale dna upgrade` proposes each
  practice whose current text is not the latest proposed under its
  name, superseding the active version under that name (a pending or
  declined proposal is not a predecessor), one replacement at a time:
  a practice whose latest proposal is still before the Board waits
  (`upgrade` says so), and a refused one is proposed again against
  what is active now. The projection
  retires an idea by marking it not accepted: it leaves every package
  and stays readable by digest.
- **The operating practices, as proposals (GH #994).** Beside the
  design, `init` seeds a second family, `operating/<slug>`: how the
  organism runs, which is what a leader plans within and a reviewer
  cites, where the design says how an organization is shaped — one
  store per step, row first, readings never act, legs hold nothing,
  deploy settles on pulse, the forge decides. Each is one paragraph,
  a rule a person applies. The family is seeded, listed (grouped
  `operating`; `hale dna review operating approve|reject`), decided and
  superseded exactly as the design, and `upgrade` seeds it into an
  organization whose record never held it. Its receipts carry
  provenance `design`, which marks a toolchain-seeded practice of
  either family; `HALE_DNA_DESIGN_SUFFIX` (fixtures only) appends to
  every practice of both.
- **The charter (GH #596 L).** `init` writes `dna/org/charter.hl`, a
  function returning text like `purpose`: the leader's brief, saying
  that it is the organism's architect — it proposes, the Board
  decides — and what it must know before it plans an ask or decides a
  Review. Project-owned; a change is a reviewed change to the
  organism. `upgrade` writes it for an organization from before it.
- **The leader reads its brief at both moments it thinks.** The org
  chart hands the `Leader` its `charter` and `purpose` (the program's
  own text) and a `MemoryKnowledge`; before a review and before a
  plan it composes its brief — `CHARTER`, `PURPOSE`, the `LAW` as the
  genome holds it at HEAD, and `PRACTICES` from the package for `org`
  when memory has any — and puts it above the diffs. The model
  call carries `knowledge_bindings` naming the package, so the
  evidence of every decision that read it says so.
- **`HALE_DNA_DISCOVER=off`** makes `init`'s discovery find nothing —
  no key, no local model, no harness — so a fixture on a developer's
  machine makes the organization CI makes: one whose leader has no
  model that answers, whose plans take the defaults, and which spends
  nothing. The CLI fixtures set it.
- **An ask is admitted as a workflow, in the one engine (card 18).**
  `Dna.ask(Intent)` — intent through the membrane, a schedule's, an
  optimizer's — passes the membrane's gate, the owner's (GH #664) and
  the budget's, journals `intent.offered`, and admits a workflow for
  it (`workflow.admitted`, the one positive discriminator, under the
  intent's id as the admission's identity — offered again it is the
  same execution) which the engine the substrate owns for its scope
  runs. With `planned: true` on the substrate (the generated org chart
  says so) the leader's word comes first: the substrate publishes
  `PlanRequested` for the ask, the leader answers `TaskPlanned`, and
  the admission binds that word in its inputs — the objective, the
  kind (`organism`, `appendage`, `product` or `person`), the class
  applied (an `organism` change is class `organization` whatever the
  plan called it: the Board's), the target, whom and under what
  obligation, the narrative, `planned`. A person's job is admitted
  under the `ask-person` definition (one human leaf, a case the
  substrate hands to whom the leader named, with no second word asked
  — cards 16 and 17 complete it); anything else under `ask-edit` (one
  edit leaf under the `Patch` contract, performed by the substrate's
  editor as class and target the word names — the target only when it
  names a file the genome has under the seed the class edits; a
  model's prose is not a path — else class `application` at the ask's
  own target; card 15 reports its outcome). Class `organization` for a
  child that is not the organism is a contradiction and is refused
  before anything is admitted (`intent.refused`). The execution is
  asked of the engine with the performer kind of every leaf of the
  whole bound tree, named for the exact Work — its bound id, never its
  key, which two Works of different steps or workflows may share — the
  editor's for an edit leaf, the human kind for a person's, else the
  routing policy's choice for the Work's own request under the Task
  that owns it. The tooling's answer to an ask (`hale dna task create`) is the
  execution whose admission names that ask as its request, never the
  next birth in the record, which may be another ask's. An answer that names
  no kind and no class leaves the defaults standing — class
  `application`, the ask's own target — and the admission says it did
  not parse. Without a leader the ask is an application change. An
  ask whose leader has not answered yet is in planning, and asked
  again meanwhile is not asked of the leader twice. No `task.planned`,
  `task.pending` or `task.resumed` row is written: the admission is
  the plan, and the execution's facts are the engine's rows.
- **Work survives a restart (GH #604, card 18).** The record holds an
  execution's admission before anything runs; a birth the record
  refuses stops there (`intent.refused`, `workflow.refused`), and
  nothing has run. On restart the substrate asks its engine again for
  every admitted root of its own that is not finished — settled AND
  drained: a cancellation settles the root before its admitted Works
  have settled, and a root whose tree (its own workflow's Works and
  every child's, by their admissions' `parent_task`, and every
  admitted child itself, before its first attempt or after its last
  Work settled) still owes a settlement is asked for again, so the
  engine's own recovery drains it (cards 12a, 13: the responsibilities
  still owed anywhere in the tree are counted, and a cancelled parent
  whose step holds only a child member drains the child through that
  step, born fenced). A child reborn under a cancelled ancestor reads
  that cancellation from the record at its own state question — a
  fence published before it existed reached nobody — proposes its own
  cancellation, and drains like any cancelled execution; a drained
  record is asked for nothing and written nothing. The engine restores
  each from its admission and the record's facts — under the word
  bound at admission, never replanned;
  an attempt whose outcome is recorded is answered from it and never
  runs again; a case handed stays a person's and waits. A Mutation's
  recovery is derived from every row it has, from its first — its
  request row, or its proposal when no request names it: one whose
  last word is the request, the proposal, an open worktree or a
  located file was in flight and fails, once (`mutation.failed`); one
  with a candidate, a Review or an outcome after them is left to that.
  Its id is counted at the restart and never minted again. A Task with
  no admission — a row of the shape from before card 18 — is left
  where it is: nothing resumes it, plans it or settles it. An
  `intent.offered` with no admission naming it is noted
  (`intent.unrecovered`) and never re-offered: work may already have
  run.
- **Dev relies on docker compose.** `init` writes `dna/compose.yaml`
  (the `knowledge-db` service, `pgvector/pgvector:pg16`, a named
  volume `hale-dna-<project>-knowledge`, a host port in 54xx from the
  project's name); it is part of the genome. With no
  `HALE_DNA_MEMORY_DSN_OWNER`, `hale dna dev` runs `docker compose -f
  dna/compose.yaml up -d --wait knowledge-db` and derives the owner's
  DSN from the published port; an operator's
  `HALE_DNA_MEMORY_DSN_OWNER` wins. It applies memory's schema with the
  owner's DSN and starts the host with the record's spine DSN alone
  (**Memory**). With compose not on PATH, or no compose file, it says
  exactly which it needs and the host runs with no memory. `hale dna
  run` brings up no database: beyond one machine memory is a Postgres
  of the operator's, migrated with `hale dna memory migrate`.
- **Knowledge changes later work (K2).** The substrate holds a
  `MemoryKnowledge` (`budget`, 8 by default; memory opened under the
  spine's role), the owner's line to memory; the editor never holds
  one, which the law states (`group knowledge = { dna::Knowledge,
  dna::MemoryKnowledge }`, `editors_never_learn`). When a Mutation
  opens, the substrate asks memory for the package of the change's
  target in the
  tower — `org` for an organization change, `org/<child>` for the
  application, `org/<child>/<seed>` for a seed inside it — and
  journals `knowledge.consulted <mutation>` (`target`, `hat`, `digest`,
  `revision`, `included_n`, `included`, `error`) whenever memory is
  named (`HALE_DNA_MEMORY_DSN_SPINE`), answer or not. The package
  becomes the hat the attempt wears (GH #946): built by the owner,
  kept in memory by its digest, and named on the request as its
  `context_digest`; the editor is briefed with the package's ideas
  (`SourceEditor.brief`: what a model is shown is the ask, then a
  `PRACTICES (ratified knowledge for <target>, package <digest>):`
  block, one idea per line) while the record, the commit message and
  the Mutation keep the ask itself; and the request names the package
  (`knowledge_bindings: package:<digest> <id>…`), which every model
  call of the attempt carries into its `model.called` row. No memory named is an empty package that says so, and nothing
  waits on it; memory that cannot answer, or whose projection has not
  reached the record's head within 20 s, is a package refused with the
  reason (**The spine projects; readers read**).
- **Concerns (K2).** `ConcernRaised` (`source`, `what`, `severity`) is
  a child's signal about the part above it: an application's, through
  its node (K4), or `hale dna concern raise <source> <what…>
  [--severity N]`'s, each a `concern.requested` row a node relays onto
  the nerves, and the substrate admits it as an execution of
  `concern-escalate` (GH #995: **The workflow catalog**) whose step
  journals `concern.raised <source>` (an object: `what`, `severity`,
  `occurrence`, `request`, `parent`, `task`), and
  when one source has raised it `concern_threshold` times (3) it
  becomes a knowledge proposal by that source bound to its parent
  path — a concern by the tower rule — through `propose_knowledge`,
  with `concern.proposed <source>` naming the digest, once. A source
  with no parent (no `/`) is refused with `knowledge.refused
  concern:<source>`. **Which occurrence a concern is counted from the
  record**, never from a number the organism keeps, so a restart
  continues the count where the record left it; a count that starts
  again is a record read short, not a restart (GH #748). An execution
  a restart interrupts is resumed with every other; a source over the
  threshold with no `concern.proposed` of its own and no
  concern-escalate running for it — rows that reached the record
  another way — is owed a proposal that nothing but a further concern
  would make. At birth every such source is escalated once, by an
  execution whose `raise` step finds its raises in the record
  (`rehydrate`);
  a source with no parent was refused where it was raised and is not
  refused again.
- **The repository's graph (GH #1085).** An organization and its
  product are one recursive hypergraph; the org chart and the deployed
  process model are two perspectives over it, neither its trunk. The
  record holds it as rows and memory projects it with everything else:
  one record, one projection, no second store. The vocabulary is
  `dna/operations/graph.hl`. **Node kinds**: `purpose`, `axiom`,
  `process`, `seed`, `contract`, `noun`, `deployment`, `practice`,
  `gate`, `document`, `witness`, `position`, `work`; a node's id is
  `<kind>:<name>`. **Hyperedge kinds**, arity two or more, each a list
  of members `{role, node}` whose first is its anchor:
  `unfold(parent, child)`, `meets(contract; server…, consumer…, carrier…)`,
  `names(contract; noun…)`, `refers(from, to)`,
  `constrains(axiom; shaped…)`, `runs(deployment; process…)`,
  `gates(gate; guarded…)`,
  `binds`, `witnesses(witness; about…)`, `holds(position, holder)`,
  `reviews(position, subject)` — the subject a contract or a document, a
  design document being signed too. A pair kind (`unfold`, `refers`,
  `holds`, `reviews`) is exactly its two members and is keyed by both,
  so an unfold is one edge per child; any other kind is keyed by its
  anchor, so there is one `meets` per contract and the latest row says
  who meets there. An edge's id is `<kind>:<anchor>` or
  `<kind>:<anchor>|<second>`, and no name or holder holds a `|`, so an id
  reads one way. A role may require a node kind (a
  `meets` contract is a `contract:` node, a `runs` process a
  `process:`); a `holder` is a person, not a node; `meets` also
  carries its transport (`via`) and the parties it reaches that are
  not nodes (`outside`, e.g. callers). Two kinds are never graph rows:
  a `practice` is a knowledge idea of kind `practice` — advice unless the
  Board ratifies it as law (its document says `law: true`; one ratified
  as advice stays advice) — and `binds` is its knowledge binding;
  a member names a practice as `practice:<digest>`. The rows are
  `graph.node` (entity the node's id, body
  `{kind, name, text?, source?}`), `graph.edge` (entity the edge's id,
  body `{kind, members, via?, outside?}`) and `graph.retired` (entity
  the id).
  **The tail checks a graph row whole** before memory takes it — the
  kind is the vocabulary's, the anchor comes first, every member names
  a node of the kind its role requires and no node twice, the arity is
  two or more, and the entity is the id the body implies — and a row that fails stops
  the projection at that row, `invalid graph.<kind> (row n): <why>`,
  like any invalid fact. A node or edge is replaced by a later row with
  its id (an edge's members with it) and taken out by `graph.retired`; a
  retired node's edges stay as the record left them, and the perspectives
  leave its memberships out (an edge whose anchor is gone is not shown).
  **There is one org chart, and it is the graph**: its `position` nodes
  and their `holds` edges, proposed and ratified as holes (below). The
  organization's generated `dna/org` files, the owners map and the
  organism's program are renderings of it, never its source — pending
  GH #1123, which derives them; until then they are written beside it,
  and nothing reads a position from them that the graph states.
  **Perspectives are queries over memory**:
  `KnowledgeStore.graph_perspective("org")` is the positions in the
  record's order, each with the node it unfolds from (`under`), its
  `holders` and the contracts it `reviews`;
  `graph_perspective("processes")` is the processes, every `meets` (`contract`, `via`,
  `servers`, `consumers`, `carriers`, `outside`) and every `runs`
  (`deployment`, `processes`), each as JSON the server builds.
  `graph_nodes_count(kind)` and `graph_edges_count(kind)` count them;
  `practice` counts the practices proposed or ratified and `binds` the
  ratified practices that bind something, one hyperedge each.
- **Ingest: `hale dna init` on a repository (GH #1090).** A directory
  with no Hale source of its own that is no workspace's seed is a
  repository, not one application: `init` makes the organization there
  as it would for an application — `vendor/dna`, `dna/org`, the manifest
  with `[claims] no_base` and `[environments.org]` alone (there is no
  application entrypoint) — cuts no topology artifact, and seeds the
  record with what the repository holds, as the graph, through the host
  verb `graph-ingest`, then proposes the declared purpose (GH #995)
  (`dna/operations/graph_ingest.hl`). It never derives one perspective
  from the other. A directory holding `.hl` files is a seed: with a main
  locus it is an application, attached as before with no graph ingest,
  and without one it is refused as before. Ingest reads the tracked
  files (`git ls-files`, paths as git holds them with `core.quotePath`
  off) and recognizes: the **purpose** — the `README.md`'s title and
  first paragraph, and none without a `README.md`, so an empty tree is
  an empty graph; **processes** — the compose file's services
  (`compose.yaml`, `compose.yml`, `docker-compose.*`), then the
  `compose` **deployment** that runs them (built) — processes first,
  since a perspective lists in the record's order; **seeds** —
  each directory holding `hale.toml`, `package.json`, `Cargo.toml`,
  `go.mod` or `pyproject.toml`, the outermost only (a workspace's
  members are its own), never the root; a process unfolds into the seed
  of its name; **contracts** — `spec/*.yaml`, `spec/*.yml`, `spec/*.md`
  and each directory under `spec/vendor/`; **nouns** — a YAML contract's
  `components.schemas` and a markdown contract's `CREATE TABLE`s, named
  by the contract; a noun is its name, so two contracts that name one
  merge into one node, which both `names`; then **`refers`**, from a
  contract's local `$ref: '#/components/schemas/<X>'` (never another key
  that holds such a path, never a reference that leaves the contract)
  or `REFERENCES <table>`, only to a noun the same contract names — every
  noun of a contract is written before its `refers`;
  **documents** — every `.md`; **gates** — each CI job
  (`.github/workflows/*`), named `<workflow>/<job name>`, guarding the
  seeds, contracts and compose deployment its steps name; **witnesses**
  — each `FRICTION.md` entry (a `##` heading) and each issue it links
  (`<owner>/<repo>#<n>`), about the seeds, contracts and documents it
  names. What structure cannot say is written in markdown, in two
  forms. **A marked list item**: a code span opening a list item names
  its kind — `axiom`, `derived`, `practice` or `law` — and its first
  bold span is its name; an `axiom` is a node under the purpose, and
  the repository files it links to are what it `constrains`. (`derived`,
  `practice` and `law` are practices, proposed at record birth, GH
  #1091.) **A declaring table**: a table whose columns include
  `Served by` and `Consumed by` declares one `meets` per row — the row's first
  link to a contract is its contract, `Over` its transport — and one
  whose first column is `Deployment` and has `Runs` declares a named
  deployment and what it runs. In a cell a name in code is a process,
  else a seed, else a repository path; a link is the node its target
  is (relative to the document); plain words are a party outside the
  repository; a code span inside a link is the link's text, not a name;
  an `Over` that names a process makes it the carrier. Links resolve to
  a contract, else a document, else the compose deployment, else the
  seed holding the path. Every row comes out once, a node before an edge
  that names it, and **the host checks every `graph.*` row against the
  vocabulary before it appends any** (`record-seed` does the same for
  a seed file), so an ingest the graph refuses leaves the record as it
  was. A YAML mapping's children are indented as its first child line
  is, never by an assumed two. **Input the conventions cannot take is
  refused, with the reason, never dropped**: an `axiom` item with no
  bold name; any name the vocabulary refuses (a `|`, a control byte,
  longer than 512 bytes) — a witness's title, a schema's, an axiom's;
  a meets row that links no contract; a runs row that names no
  deployment; a name in code in a declaring table that is no process,
  seed or file of the repository. Nothing is appended then, the purpose's
  Review included (init seeds it and the graph in one `graph-ingest`
  call), so the record does not exist and init can be run again once the
  document is fixed. A job's own `name:` names its gate and is not a
  word its steps say. `init` says what it read:
  `graph   <n> node(s), <m> edge(s): <count> <kind>, …; <p> proposal(s)`.
  A record that exists is not reseeded.
- **Holes as proposals (GH #1091).** What a repository cannot imply
  and a delivery needs, `init` proposes at record birth, each on its own
  with one Board Review, in the same checked seed as the graph (so a
  repository whose holes cannot be proposed leaves no record either): a
  `board` under the purpose; a `dev` and a `reviewer` under each process,
  the reviewer's proposal carrying a `reviews` edge to every contract the
  process serves (a `meets` server); under each deployment an `operator`
  and the operational roles no artifact implies — `support`, `accounts`,
  `billing`, `on-call` — proposed empty; one `work` item per process, an
  `unfold` of it, its node's `done_when` the gate that guards the
  process's seed, or none, and its text says to define one; and each item
  a document marks `derived`, `practice` or `law`, proposed as advice (a
  `law` item's question asks for law). A hole is a knowledge proposal:
  its receipt (`kind: graph`, `rows`) carries the graph rows it would add,
  each checked by the vocabulary before it is proposed; `knowledge.proposed`
  and `review.requested` (`required_authority: board`, `group: holes` or
  `practices`) are the rows a practice is proposed with. The existing
  review path ratifies it (`knowledge.ratified`); the projection, seeing
  a ratified document of kind `graph`, puts its rows into the graph at the
  row that **proposed** it (its `knowledge.proposed`), checking each again,
  so the org chart lists positions in the order the holes were proposed
  whatever order the Board ratifies them in; the proposal stays in memory
  as the idea it was, bound to nothing, so it never enters a context
  package. Ratifying them fills the org chart. `hale dna review` lists the
  groups `holes`, `practices` and `holds` together, as it does `design`
  and `operating`, and `hale dna review <group> approve|reject` decides
  each pending one in turn.
  **`hale dna fill <position> <holder> [project] [--as <who>]`** asks the
  running organization for a holder,
  as `hale dna practice propose` asks for a practice (`init` alone writes
  proposals directly, since no organism exists yet): a
  `hold.requested <id> {request_id, position, holder, by}` row in the
  asker's name, relayed as `HoldRequested` until the organization answers
  `hold.proposed` (`digest`, `review_id`) or `hold.refused` (`why`). The
  organization proposes it like any hole (`group: holds`), in the asker's
  name, so the asker may not ratify it. The CLI checks first that the
  position is one the record states or proposes and has not retired
  (`graph.retired`), that the holder is a person the record knows — who
  wrote a row in their own name, or an owner's member — and has not
  retired (`person.retired`), and the same-holder rule. **The store is the
  gate**: a hold is checked again where memory projects it, however it
  reached the record — a holder the record never knew before the row (no
  row in their name, and no owner's member) is not projected, nor is a
  retired one, and a process's `reviewer` held by its `dev`'s holder is a
  warning on stderr under `dna.trust = local` (one person holds every
  role) and not projected under any other trust. A hold the store refuses
  is a row, once: `hold.refused <hold id>` (`why`, `row`, `by: memory`).
  A person retired after their hold was projected gives every seat back
  where the retirement is projected. `fill` and `practice propose` name a
  request by its digest at the record's head (`h…`, `p…`), so two requests
  in one millisecond never share an id.
- **A repository record runs its organization alone (GH #1091).** When the
  record's seed holds no Hale source of its own, `hale dna dev` and
  `hale dna run` check and build the organization only; there is no application
  to cut, build, express or restart, and `dev` says so.
- **Review routing (GH #1087).** A change set goes to the positions that
  must sign it, read from the graph, never guessed:
  `hale dna route [--json] (<path>… | --diff <range>)` (the host verb
  `route`, reading memory as the head, over
  `graph_perspective("routing")`; the routing is
  `dna/operations/graph_route.hl`). A path is the repository's: one
  given from a subdirectory is taken from there, and the organization's
  root is the nearest directory holding `dna/org` (a seed's own
  `hale.toml` is not it). A path names a node — a contract (its file, or
  a file under its directory), else a document, else a deployment (the
  file it was read from, when that is no document: `compose.yaml`), else
  the longest seed holding it — and the edges say who signs. **A seed**: the reviewer
  (the `/reviewer` position a process unfolds into) of every process that
  unfolds into it. **A contract**: every position that `reviews` it, and
  the reviewer of every consumer — a consuming process; the processes a
  consuming seed belongs to; the positions reviewing a consuming contract
  — and the `board` where the contract is law: a practice the Board
  ratified as law is bound to it (its receipt says `law: true`); one
  ratified as advice is not law, and the board does not sign for it.
  **A document**: every position that `reviews` it. **A deployment's
  file**: the `operator` position the deployment unfolds into, against the
  gates guarding the deployment (`ci/image` gates `compose`). A
  `/dev` position never signs: a process's dev never signs for that
  process. **The evidence** a verdict is given against is the run of every
  gate guarding a node the change touches. A consumer no position reviews
  is listed as such (`unsigned`), never dropped; a path that names no
  node, or whose node no position signs (or operates), is left to the
  fallback — the Review's own authority today, the task routers of GH
  #697 when they exist, which the route names. Text lists `signed by`,
  `against` (`no gate guards it` when no gate does), `reviewed by no
  position` and `left to the fallback, <what it falls back to>`, each
  signer with its holders and why it signs; `--json` is one object:
  `signers` (`position`, `because`), `evidence` (gate ids), `unsigned`
  (`node`, `because`), `fallback` (`path`, `because`), `fallback_to`. Reviews do not yet take their signers from it; that wiring
  is its own change.
- **Perspectives: `hale dna show` (GH #1086).**
  `hale dna show org | processes [--json] [project]` is the host verb
  `show`. It takes `--json` and at most one project directory, and
  refuses any other flag, a second project or one that is not a
  directory. It reads memory under the head's role alone
  (`HALE_DNA_MEMORY_DSN_HEAD`), never the spine's, even in a process that
  holds both, scoped to the record, and prints what `graph_perspective`
  answers. Nothing is edited through it: a change moves through the
  record's rows, and the spine projects it into both perspectives. A
  record with no rows is the empty perspective, with no memory asked, and
  stderr says `hale dna init` seeds a record and `hale dna sync` fetches
  one; with rows and no memory named it refuses, and says the graph is
  memory's. The projection's stamp is read before the graph, so the rows
  it reports never overstate what the graph holds. Text
  (`dna/operations/graph_show.hl`) shows a node by the last segment of its
  name. **The org chart** is nothing when there are no positions;
  otherwise `purpose`, then a line per node positions sit under, in the
  record's order: a position under the purpose is a line of its own
  (`board`), and any other node is its name, padded in code points to the
  widest, then its positions' roles. A role is the position's name less
  its node's (`api/reviewer` under `api` is `reviewer`), followed by
  `(<what it reviews>)` and, when it has holders, `[<holders>]`; a
  position under no node is under `(unplaced)`, after the rest. **The
  process model** is a line per `meets` — its outside parties and
  consumers, the arrow `--<via>: <contract>-->`, its servers or
  `(nobody here)`, and `(carried by <carriers>)` after — then one
  `<deployment> runs { <processes> }` per `runs`. `--json` prints one object: `perspective`, `record_rows` (the
  record's length), `projected_rows` (memory's watermark), `stamp_valid`
  and `graph`, the query's answer as memory built it. How memory stands to
  the record is said on stderr, never in the perspective: behind it
  (`memory has projected <m> of the record's <n> rows`), past it (this
  clone is behind its remote: `hale dna sync`), or with a stamp that
  vouches for nothing (`hale dna memory migrate`). On voice's record with
  its holes filled the two texts are GRAPH.md's renderings, byte for byte
  (`dna/tests/graph_show_test.hl`).
- **Projections and ranking (K3).** The tail also projects the
  record's `structure.observed` rows (init's loci, topics, bindings,
  effect classes and claims) into memory by kind and name, the
  latest row winning, and its `pressure.raised` and `concern.raised`
  rows into signals counted per (kind, source, what) with the last
  row's seq (`knowledge_structure`, `knowledge_signals`), so the
  graph has what ideas bind to and what the fleet is saying. Retrieval is by binding and
  provenance first: the bounded set is the accepted ideas bound to
  the target or above it, and nothing outside it is retrieved. Inside
  it, a query — the objective, which the substrate passes from the
  ask, its first 2000 characters — ranks by similarity so the budget
  keeps the most relevant (`KnowledgeStore.ranked_ids(target, budget,
  query_vec)`; no query is ratification order). The embedding
  is lexical — `embed_text`: a hashed bag of words in 64 dimensions,
  normalized, deterministic, rendered as a pgvector literal — so `Pq`
  ranks with `<=>` over a `vector(64)` column (the migration creates
  the `vector` extension and says so when the Postgres has none).
- **The application side (K4).** An application declares the wire
  fact itself — a type of the nerves' shape (`source`, `what`,
  `severity`) on the subject `dna.concern.raised`, no import of the
  DNA — and publishes it when it observes something about the part
  above it. A node routes that subject for every instance it starts
  (`LOTUS_BUS_CONFIG=<node dir>/instance.bus.conf`, role connect) to
  its own socket `<clone>/.hale/node/concern.raised.sock`, which the
  `hale node` shim binds for it as an environment-configured listen
  route before the node starts; the node subscribes `ConcernRaised`
  with no source binding, and on each one appends `concern.requested
  <source>` (`source`, `what`, `severity`, `node`) to the record in
  its name and syncs. A node relays `concern.requested` rows onto the
  nerves like `intent.requested` and `review.verdict`, and the
  organization journals `concern.raised`. `hale dna concern raise`
  takes the same road. This environment route is the one
  `LOTUS_BUS_CONFIG` DNA writes (**The nerves**; #987 replaces it).
  A route for a subject an instance never publishes is inert.
- **The learning scenario** is the acceptance (K4), in the fixture:
  the worker on the second node observes its mail backlog and raises
  the concern three times; each travels node → record → host →
  nerves → `concern.raised`; the third makes a proposal by
  `org/trio/worker` bound to `org/trio` (a concern by the tower
  rule); the Board ratifies the exact digest (`hale dna review k:…
  approve --authority board`); the spine projects it into memory on
  its tick; and the next change to the trio consults memory
  (`knowledge.consulted m1` with the digest and the hat's digest),
  briefs the editor with the concern under the objective, and every
  `model.called` row of the attempt names the package and the digest.
  Something observed and ratified today informs the work done
  tomorrow, and the receipt says so.

## Backends by role

The assembly names what fills each role; `hale check` sees the wiring.

- **Record** — `Journal`: `GitJournal` (the branch), `MemJournal` (tests).
- **Membrane** — where humans see and decide: `Board` /
  `LocalHumanMembrane`, reached over the nerves on one machine; the record
  itself across clones (`intent.requested`, `review.verdict` rows,
  relayed by the host); GitHub, mirrored by the host when `git config
  dna.github` names `owner/repo`: every pending mutation Review becomes
  a pull request (`github.pr`; the candidate pushed to `dna/<id>`, the
  three views as the body), every GitHub review on it becomes a
  `review.verdict` row in the reviewer's login (authority `board` when
  `dna.github.board` lists the login, else `reviewer`; each review
  once, keyed by login, commit and state), every settlement goes back
  as a comment (`github.commented`) and an approval pushes the genome —
  both once the Mutation is applied (or has failed), since a Review
  settles before its Mutation is applied.
  GitHub is a projection of the record and the record wins: a review
  whose head moved is refused here and shows as refused there. **The
  forge is an implementation (GH #648):** `Forge` in the core is the
  vocabulary of a code-review host and nothing of GitHub's —
  `open_review`, `verdicts` (one per line: who, outcome, when, the
  forge's own key so each is admitted once), `comment`, `close_review`,
  `present`, `name`. `GitHubForge` (`gh`) is one implementation; the
  `FileForge` (`.hale/dna/forge/<n>.review`, its verdicts the lines of
  `<n>.verdicts`) is the fixtures'; `NoForge` is a bare remote's honest
  behaviour, and `hale dna profile`'s `github:` line names which the
  host found (`HALE_DNA_FORGE`, or `dna.github`). The membrane's pass
  is written against the interface; the rows it appends (`github.pr`,
  `github.commented`, `review.verdict` keyed by the forge's key) do
  not change with the forge. Nothing in the core or the host spells
  `gh` outside that one implementation.
- **Deployment** — `Deployment`: `NoDeployment` (a host expresses:
  `hale dna dev`), `ShellDeployment { command, seed }` (the command
  owns expressing and judging: `express <candidate> <seed>` returning 0
  means up and healthy, `rollback <base> <seed>` restores; the exit
  code is the observation, `expression.deployed` records it), and
  `LocalApplyDeployment` (tests). With a gateway wired, an approval
  expresses and judges in the organization's own handler and no host
  is asked; without one, `expression.restart_requested` asks the host.
- **Observation** — for a shell gateway, the command's exit; for a
  host, the window it watches; for a fleet, what its nodes report
  (below).

## The fleet

The genome's topology is the fleet plan Hale already checks
(`spec/verification.md`, "Fleet composition"), and the DNA expresses
one arrangement of it:

- **The plan describes the workspace's own services.** Plan schema
  1.2 lets an instance name a `seed` (relative to the plan) instead
  of an `artifact`; composition cuts the artifact from the seed first,
  so a candidate is checked as the fleet it would deploy. An instance
  may name the `node` that expresses it. `[dna] fleet = "<name>"` in
  `hale.toml` names which entry of `[fleets]` the DNA expresses.
- **The fleet is evidence.** Verification runs `hale fleet check --in
  <worktree> --if-declared` on every candidate (`evidence.fleet`;
  `fleet_clean`, true when the workspace declares no fleet). A change
  to one service that breaks a claim the fleet makes over all of them
  is `mutation.deny` ("candidate breaks the fleet") with the witness
  in the receipt. A candidate whose diff names a plan (`*.plan.json`)
  or the manifest (`hale.toml`) changes the fleet's shape: it is
  re-classed `topology` (`mutation.topology`) whatever it was asked
  as, and `OrgPolicy` sends it to the Board.
- **A deploy is a row.** `fleet.deploy` (entity: the Mutation, or the
  short revision for an operator's deploy) carries the plan name, the
  revision, the seed the change edits (`""` for the whole genome), the
  instances touched — every instance with a node whose seed is that
  directory — and the reason (`apply`, `rollback`, or the operator's).
  The revision is pushed to `refs/dna/revisions/<rev>` on the record's
  remote first, so every node can fetch it. Under `hale dna run` with
  `[dna] fleet` set, the host answers an application's
  `expression.restart_requested` with a deploy row; `hale dna deploy
  <revision>` and `hale dna rollback <mutation>` write the same row by
  hand. A rollback's row asks for the base; nothing is watched.
- **A node expresses.** `hale node <name> [--repo <clone>] [--fleet
  <name>] [--tick <ms>]` runs in a clone of the governed repository.
  Every tick it syncs the record and reads the latest `fleet.deploy`;
  when that names a revision it does not express, it fetches the
  revision, checks it out, and for each instance the plan assigns to
  the node that the deploy touches (or that is not up) cuts the
  artifact, builds the seed, restarts the process (cwd the clone,
  `LOTUS_OBS=1`, `HALE_DNA_NODE`, `HALE_DNA_INSTANCE`,
  `HALE_DNA_EXPRESSION`) and appends `instance.up` as `node/<name>`:
  node, instance, revision, model hash, build digest, pid. An instance
  that exits appends `instance.exited` with its code. The expression
  identity per instance is therefore reported by the node that
  expresses it and joined to the plan by instance id. The node decides
  nothing. `.hale/node/<name>/<instance>.pid` is the only local state.
- **The window is over every touched instance.** After an apply's
  deploy row the host waits for every touched instance's `instance.up`
  at the revision (180 s to settle), then for the observation window
  with no `instance.exited` at the revision among them. All up and
  none exited: `Observation healthy` with the model hash the instances
  reported. One exited: `expression.crashed` naming the instance and
  its node, `Observation crashed` with the same detail, and the
  organization rolls back — a rollback deploy row touching the same
  instances, which every node answers. Never up: `expression.crashed`
  "never expressed by …" and `Observation build_failed`.
- **`hale dna fleet`** renders what the fleet expresses from the
  record: every instance of the plan, its node, whether it is up, the
  revision and model hash it last came up at, and the last deploy.

## The host is a Hale program

Everything `hale dna` does beside the compiler that is DNA behaviour
rather than manifest or scaffolding is `dna/host`, a Hale seed
embedded in the toolchain beside the core and the surface, built once
into the toolchain cache. `hale dna <verb>`
resolves the project from the manifest (the root, the application's
seed, the fleet and its plan under `[dna]`) and execs the host:
`host <verb> <root> <seed> <fleet> <plan> …`, with `HALE_BIN` (the
toolchain) and `HALE_DNA_TOOLCHAIN` in the environment. The host owns the
projections (`status`, `history`, `review`, `board`, `report`,
`pressure`, `fleet`), the writers (`task create`, a verdict, `pressure raise`,
`sync`, `deploy`, `rollback`, `github sync`), the supervision (`run`,
`dev`), in which it is its program's `main locus` and binds the nerves,
and the node agent (`hale node`). It reads the record through
the core's `GitJournal`, appends in a person's or a node's name with
the ref compare-and-swapped, and starts every child — the
organization, the application, iris, an instance — through `sh`,
detached, with its pid, its exit code and its log as files under
`.hale/dna/` or `.hale/node/<name>/`, echoing the log to the terminal
a tick at a time. The compiler keeps `init` / `new` / `upgrade` (they
embed the sources), `hale fleet check` and the plan schema, and the
exec shims. The host decides nothing.

## A seed is built once per content fingerprint

The host builds the organization's seed before it starts it, and the
application's seed too under `dev`; it rebuilds both on a restart. A
build is a pure function of what it reads, so the host does it **once
per content fingerprint** and copies the binary thereafter. The cache
is `$XDG_CACHE_HOME/hale/dna-build` (or `~/.cache/hale/dna-build`), a
sibling of the toolchain cache the host and the surface are themselves
built into; it holds the 32 most recently used
binaries and prunes the rest.

The fingerprint is a SHA-256 over:

- every file the build reads, each as its path relative to the
  project and the SHA-256 of its bytes. The list is the compiler's
  own — `hale inputs <seed>`, which resolves imports exactly as a
  build does — so a seed that grows an import, or a vendored core
  that is upgraded, is a different fingerprint without anything here
  having to be kept in step;
- the project's `hale.toml` and `hale.lock`;
- the toolchain version, and the `hale` binary's own size and mtime,
  so a compiler rebuilt in place without a version bump is a
  different fingerprint;
- every environment variable that changes codegen;
- the seed's own path, because a binary carries the paths of the
  sources it was built from.

A hit is a copy and nothing else: the binary is copied beside the
seed and renamed over its place, so a reader sees the old file or the
whole new one, and a binary that is still running is replaced rather
than written through. A store is the same rename into the cache, so
two hosts building one thing at once need no lock; one of them wins
and both are right. Anything that stops the fingerprint being taken
whole — no cache home, `hale inputs` failing — is a miss, and a miss
builds exactly as it did before. `HALE_DNA_NO_BUILD_CACHE=1` makes
every build a miss.

## The embedded source has a name

`hale dna init` / `new` / `upgrade` materialize `vendor/dna` from the
DNA source **embedded in the `hale` binary**, so an organism runs the
core that binary carries and not the one in any checkout. A version
does not identify that source — two builds of one version can embed
different `dna/` source — so the toolchain names it (GH #726):
`EMBEDDED_DIGEST` is a length-framed SHA-256 over the sorted `(path,
content)` pairs of every embedded file (`dna/core`, `dna/host`,
`dna/operations`, `dna/organization_runtime`,
`dna/organization_source`, `dna/ui`, and pond's driver and NATS client
pinned under `dna/core/pond`), computed when the toolchain is built. It is:

- the second line of `hale --version` (`embedded dna: <first 16 hex>`;
  the first line stays the version alone, since a provisioning script
  reads field 2 of it);
- all 64 digits on stdout, alone, from `hale dna --embedded-digest`,
  and — with `--from-tree <dir>`, a checkout holding `dna/core` — the
  digest that checkout *would* embed, by the same algorithm. Unequal
  digests mean the binary predates the tree: what it materializes is
  the older core, and a test of an edit to the tree measures the
  wrong source. A missing directory is refused, never digested as a
  smaller set;
- the first line of `hale dna status`, with the toolchain version, and
  a refusal to stay quiet when `vendor/dna` was materialized by
  another build (`hale dna status --json` is unchanged: the
  projection is the host's);
- recorded where a materialized tree can be asked about it:
  `vendor/dna/README.md` and `.hale/dna/embedded.digest` (`hale
  <version>` then `embedded dna: <digest>`), both toolchain-owned and
  git-ignored, both refreshed by `upgrade`. It is deliberately **not**
  written into the generated organization's source: `dna/org/main.hl`
  is project-owned source that is reviewed, diffed and read by
  models, and a line that changes with every DNA edit belongs in
  neither a review nor a recorded baseline.

The build enforces its own snapshot: the digest is computed over the
on-disk tree by the build script, the crate digests what the compiler
actually embedded, and the two must agree — source edited *while* a
build runs, or a file added without being embedded, fails the build
rather than shipping a digest that names nothing.

## The surface

`hale dna ui [project] [--port N]` serves the DNA surface in a browser
from the record alone: a Hale program (`dna/ui`, embedded in the
toolchain like the core and the host, built once
into the toolchain cache) that answers every request by running one
offline verb of `hale dna` in the project root and returning what it printed
— the status projection (`/api/status`), the Board's queue
(`/api/board`), the pending Reviews and one Review's three views
(`/api/reviews`, `/api/review/<id>`), the fleet (`/api/fleet`), the
history (`/api/history[/<entity>]`), pressure (`/api/pressure`). A
verdict (`POST /api/verdict`), a task (`POST /api/task/create`) and a
pressure signal (`POST /api/pressure`) are the CLI's own verbs sent
and not waited for: rows in the record, in the name the form gives,
which a node relays. A path segment
reaching the CLI is cleaned of separators and leading dashes, so a
request cannot name a file or a flag. The surface reads nothing
itself and decides nothing; with or without an organization up, it
shows what the CLI shows. `hale dna review <id> <verdict> --no-wait`
is the same non-blocking verdict from the terminal. Iris stays the
observer: attached to the organization's process it renders the org
as the live topology it is.

The head (`dna/api/project_service`, `dna/face/start.sh [project]`),
which serves the face, is the surface's counterpart for the operator's
machine: one loopback process that serves the browser shell, keeps a registry
of projects and a journal of receipts under a state directory, and
proxies the Record's read and command routes to a per-project API
child it starts and replaces. Its own operations — create, init,
attach, detach, forget, sync, publish, forge, the local body, the
remote body, secrets by source name, the model probe, connections,
handoffs — are the CLI's verbs run detached with a
pid, an exit code and a log as files, and each answers a receipt
(`recorded → admitted|refused → running → succeeded|failed|outcome_unknown`)
under a `command-<digest>` identity that replays on an identical
retry and conflicts on a changed one. A run that passes its deadline
is `outcome_unknown` when the verb reaches beyond this machine, never
`failed`; the head appends nothing to any record itself. It is
trusted-local and refuses an `oidc` project.

The head tells the browser when to read again, and the browser does
not poll it (GH #986). `GET /api/hale/v1/head/events` is a
`text/event-stream` the head writes `event: changed` to — `data: rows`
when a row landed in the active project's organism, `data: head` when
the head's own state moved: a receipt journaled, a run or a child
started or ended, the API child answering. An event carries nothing to
render; the face reads the API, the one source of truth, again, and
once more on every reconnect. A comment line every 15 s keeps a quiet
stream open and finds one whose browser left; a write that does not
complete within a second closes its stream. At most 16 streams are
held; one more is refused 503 `busy`. The rows come from the nerves
(**Rows landing**): the head subscribes to the active project's
organization and every owner's, as the head's user, on
`HALE_DNA_NATS_URL_HEAD`, else on the server of
`HALE_DNA_NATS_URL_OWNER` (which it hands a body it starts), else on
the project's compose `nerves` service while `hale dna dev` has it up
(asked for, never brought up; asked again every 5 s). With none, only
the head's own state is pushed. The application view still reads on a
timer until #987 gives an application its events.

**The API** (`dna/api`, source-built; the head starts one per project)
is a head of memory. It reads the Knowledge graph in its own process
under the head's role (`HALE_DNA_MEMORY_DSN_HEAD`), through one handle
opened on the first read and held for the process's life; a projection
that has not reached the record's head answers
`knowledge_projection_unavailable` until the spine's tick applies it,
and without the DSN Knowledge reads are unsupported
(`knowledge_unsupported`). It admits Knowledge commands into the
record in the same process, under the explicit authority policy in the
file `HALE_DNA_KNOWLEDGE_COMMAND_POLICY` names (`dna.knowledge-authority/1`,
at most 64 KiB, decoded by `KnowledgePolicyCodec` in
`dna/operations/knowledge_policy.hl` against the record's identity and
`dna.trust`). The policy is read once, when the API starts: an edit to
the file does not change a running API's basis, and a policy that does
not decode stops the API (exit 2). `dna/api/README.md` describes the
routes.

What is deliberately not here yet: a fleet-level semantic diff (a
Review's diff is the edited seed's; the deploy row names the instances
it reaches), pressure raised from services' typed metrics (`hale dna
pressure raise` is the spelling; nothing raises it for a node), and
the organization as an instance of its own plan.
