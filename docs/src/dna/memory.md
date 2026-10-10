# Memory and the record

The organism remembers in two places. The **record** is its long-term
memory: every decision, in order, never rewound. It is git, under
`refs/dna/*` in your repository, and anyone with a clone writes to it
in their own name. **Memory** is its working memory: the day's work
(the ledger), the graph the org chart is read from, and protected
evidence. It is Postgres, on the private tier, and every client opens
it under a role of its own. A record write that loses a race goes
round again until it lands; memory that stops answering stops the
organism admitting anything until it answers. Neither is ever reached
from a device: a browser talks to [the head](./head.md), and the head
reads for it.

## Three memories, one home per row

There is a third memory too: the codebase itself, the genome, which
holds what the organism *is*. Every row kind lives in exactly one of
the other two:

| memory | holds | lives in |
|---|---|---|
| record | how the organism changed and was allowed to: mutations and their Reviews, practices proposed and ratified, grants, connections, people, the ledger's adoption | git, `refs/dna/journal` |
| ledger | what the organism did today: intents, tasks, cases, workflow executions and attempts, bills and receipts, money reserved and settled, schedules, concerns, readings, effect claims, leases | Postgres, the record's own schema |

The table is `memory_of` in `dna/core/routing.hl`. Which one applies
is a fact of the record, never a build's opinion: a new organism is
on **routing 0**, every row in the record, until you adopt the ledger
([below](#the-ledger)). `hale dna history` and the projections read
both memories as one sequence.

## The record

### What is under refs/dna

| ref | what it holds |
|---|---|
| `refs/dna/journal` | the chain of rows, one commit per row |
| `refs/dna/receipts/<sha256>` | evidence bodies, by the digest of their content |
| `refs/dna/identity` | the record's identity, published so another record can connect to it |
| `refs/dna/candidates/<mutation>` | the candidate commit each Mutation ended in |
| `refs/dna/revisions/<rev>` | a revision a deploy asked for, so every node can fetch it |
| `refs/dna/lease/<key>` | a lease on routing 0 (the body's, a Mutation's) |
| `refs/dna/exchange/<identity>` | a mailbox: the rows a connected record wrote for this one ([below](#between-records)) |
| `refs/dna/remote/journal`, `refs/dna/remote/genome` | what the remote held at the last fetch |

A fresh project has the first three:

```text
$ git log --oneline refs/dna/journal | head -3
77bc90b review.requested review:k:fffbc971da28
85c2c83 knowledge.proposed sha256:fffbc971da28fafdbd6f3aa6ca5bcd4eb3e2e3626ed8140ef687f896d3540e50
0182c0f review.requested review:k:52ccd337cd20
```

Each commit's tree holds `journal.jsonl`, every row so far, one JSON
object per line: `seq`, `kind`, `entity`, `body`, `author`. Its
subject is `<kind> <entity>`. A row's digest is its commit and its
predecessor is the parent, so the commit graph is the chain.
`hale dna status` says `chain verified at <head>` when the commit
count is the row count.

The record's identity is its first commit, the same in every clone
and different for every record. Memory's schema and the nerves' token
are both named after it (`dna_<identity>`).

`.hale/dna/` is not the record. It holds the status projection,
worktrees and the toolchain's scratch; delete it and nothing the
record holds is lost.

### Append is compare-and-swap, through the Record interface

A writer builds the next commit on the head it read and moves the ref
only if the ref is still there. One that lost the race reads the new
tail and appends again; `seq` is the row's position once it lands,
never a promise made before. Two people answering at once both land.

Everything that touches the record stands on one interface, `Record`
in `dna/core/record.hl`, whose vocabulary never names git: chains of
rows, bodies kept by digest, cells swapped by version (a lease, the
published identity), pointers (a candidate, a revision), families
shared with a remote, the signer of a row, and the `dna.*` settings.
`GitRecord` is the implementation over git plumbing, the only file in
the core and the host that spells `git` for the record; the core's
`GitJournal`, `GitReceipts` and `GitLeases`, the host's sync and body
lease, and the mailboxes between records are built on it. `MemRecord`,
the implementation fixtures use, shows the race:

```hale
import "vendor/dna" as dna;

fn main() {
    let r = dna::MemRecord { };
    // alice and bob both read the chain while it was empty
    let seen = r.head("journal").id;
    let a = r.append("journal", seen, dna::Row { kind: "note.written", entity: "n1", body: "{}", author: "alice" }, false, "");
    let b = r.append("journal", seen, dna::Row { kind: "note.written", entity: "n2", body: "{}", author: "bob" }, false, "");
    println("alice: ", a.ok, " bob: ", b.error);
    // bob reads the new tail and appends again: nothing is lost
    let again = r.append("journal", r.head("journal").id, dna::Row { kind: "note.written", entity: "n2", body: "{}", author: "bob" }, false, "");
    println("bob again: ", again.ok, ", rows: ", r.count("journal"));
}
```

```text
alice: true bob: stale
bob again: true, rows: 2
```

A read of the record either happened or failed. A read that failed
(git unable to run on a loaded machine, a remote out of reach for a
moment) is never taken for an empty or a shorter record: the reader
keeps what it had and tries again, and a verb refuses with the reason
rather than act on a record that is not there.

### Who wrote a row, and whether it counts

Authorship is git's. The organism's own rows carry its configured
author; a person's rows (a verdict, an ask, a node's report) carry the
identity of whoever ran the command, and `--as <who>` names the person
on the verbs that take it.

Whether a row counts is a check every reader makes,
`dna::row_admissible`, under the clone's `dna.trust`:

- `local`, the default and what `hale dna new` declares: every writer
  to the record is trusted.
- `signed`: a row counts only when its commit carries a signature
  git verifies against its keyring or allowed-signers file. A node
  answers an unverified request with a refusal naming the commit
  (`unverified writer`) and never relays it; the projection into
  memory moves past it without applying anything of it.

### Sync

```sh
hale dna sync
```

`sync` fetches the remote's record into `refs/dna/remote/journal`,
reconciles, and pushes. Local ahead: push. Remote ahead:
fast-forward. Diverged: the local-only rows are re-appended on top of
the remote's head, bodies and authors unchanged, and pushed. The
reconciled chain is built beside the ref and swapped in with one
compare-and-swap, so a reader's view only grows and a clone that
appended meanwhile is not overwritten: the swap fails and the
reconcile goes round. Receipts and candidates travel both ways;
mailboxes are received. The remote is `dna.remote` in git config, or
`origin`. A running node syncs on every tick, and a plain clone has no
record until it syncs. With no remote there is nothing to do:

```text
$ hale dna sync
record refs/dna/journal: no remote: the record is local; 38 event(s)
```

Under `dna.trust = signed` a reconcile rebuilds only what this clone
may sign. A row it signed is re-signed, its commit naming the
original (`Rebuilt-From: <commit>`); a row never signed stays
unsigned; a row signed with another key refuses the whole sync before
anything moves:

```text
hale dna: the record diverged, and local event … (…, by …) was signed with …, not this clone's; a reconcile would re-sign it as this clone's, so the record was not changed and every local row is kept at refs/dna/journal. Have that row's writer sync first — a writer rebuilds its own rows — then sync again to fast-forward; or fetch and fast-forward once the remote holds it
```

A fast-forward is never refused.

### Candidates

Every candidate a Mutation commits is kept under
`refs/dna/candidates/<mutation>` and travels with the record, so a
refused or revised change stays readable after its worktree is gone:

```sh
hale dna candidates                            # every kept candidate, with its Review's state
hale dna candidates m7                         # one, as a diff from its Review's base
hale dna candidates drop m7 --why "superseded by m9"
```

`drop` is a row, `candidate.dropped`; the pointer goes here, at the
remote, and at every other clone on its next sync. The commits stay
in each object store until git collects them.

## Receipts and protected evidence

A receipt is evidence kept by the digest of its content: a
verification step's output, a diff document, a bill you filed. Rows
name receipts by digest. Every receipt has a class:

- `public` and `internal` bodies are blobs under
  `refs/dna/receipts/<sha256>` and travel with `sync`.
- `customer` and `confidential` bodies are **protected**. They are
  kept in memory alone, sealed there, and the record gets only the
  digest and the class (`receipt.classified`). With no memory to keep
  it, the body is withheld (`receipt.withheld`) and nothing holds it.
  Nothing protected travels with `sync`.

Memory seals a protected body under the **receipt key**, which you
give at migration as `HALE_DNA_RECEIPT_KEY` in the owner's environment
(sixteen characters at least). It is written once into the
owner-only table `memory_keys`; a different key later is refused,
because the bodies sealed under the first would no longer open. No
process that runs the organism holds it. Three functions, run as the
schema's owner, are the whole surface of the sealed table:
`receipt_file` and `receipt_read`, which heads and the spine may call,
and `receipt_erase`, the spine's alone, which deletes a body and keeps
its digest so that nobody can file it again.

**A dump of the database is as sensitive as the evidence.** It carries
the key and the ciphertext together; protect and retain dumps as you
would the bodies themselves.

The verbs:

| verb | what it does |
|---|---|
| `hale dna receipt` | lists the protected evidence in the record |
| `hale dna receipt file <path> [--class internal\|customer\|confidential]` | files a document as evidence |
| `hale dna receipt disclose <digest> --to <who> --purpose <p>` | authorizes a reader, as a row |
| `hale dna receipt show <digest> --purpose <p>` | reads a protected body in your name |
| `hale dna receipt hold <digest> --why <w>` | stops redaction until `release-hold` |
| `hale dna receipt redact <digest> --why <w> --policy <p>` | removes the body and keeps the digest |

Each takes `--as <who>` and says what it wrote, such as `receipt
<digest> filed (customer, kept in memory; receipt.classified, by
alice)`. A read is refused unless a `receipt.disclosed` row names both
the reader and the purpose, and an answer is given only once its
`receipt.read` row is in the record: every read of a protected body
is a row in the reader's name. A redaction is in the record before a
byte is erased. A git receipt's ref goes at once, here and at the
remote, and every other clone drops it at its next sync; a protected
body is erased by a node on its next tick. The blob stays in each
object store until git collects it, and a copy made outside the
record cannot be recalled.

## Working memory

Memory is Postgres, and only Postgres. There is no service in front
of it: a process that reads or writes memory opens Postgres itself,
as a role, and the role's grants are the trust boundary.

Memory is per record. Each record has its own schema, `dna_<identity>`,
and two roles of its own, `dna_<identity>_spine` and
`dna_<identity>_head`, so one Postgres can hold many records and no
role reads another record's evidence. A role's password is drawn into
the vault as `postgres-<role>`, and its DSN names that entry instead
of carrying the password; the driver presents it when it connects.

### Three DSNs, three jobs

| variable | who holds it | what it may do |
|---|---|---|
| `HALE_DNA_MEMORY_DSN_OWNER` | `hale dna memory migrate`, `hale dna dev`, `hale dna upgrade`, a provisioned body's env file | apply the schema |
| `HALE_DNA_MEMORY_DSN_SPINE` | the host that runs the organism, and the organization it starts | read and write the record's tables, protected evidence through the functions; no DDL |
| `HALE_DNA_MEMORY_DSN_HEAD` | a head: the CLI, the face, the API | read; take or fence a claim; write the ledger through its gate; file and read protected evidence |

Apply the schema with the owner's DSN and the verb prints the other
two:

```text
$ hale dna memory migrate
HALE_DNA_MEMORY_DSN_SPINE=postgres://dna_…_spine@…/dna?sslmode=…&vault=postgres-dna_…_spine
HALE_DNA_MEMORY_DSN_HEAD=postgres://dna_…_head@…/dna?sslmode=…&vault=postgres-dna_…_head
```

With no `HALE_DNA_MEMORY_DSN_OWNER` it brings up the `knowledge-db`
service of `dna/compose.yaml` (it needs `docker compose` on `PATH`)
and migrates that. The migration is one transaction and can be run
again at any time. It writes a schema version, and every
store checks it when it opens: a host whose memory is at another
version refuses to start, naming both versions and
`hale dna memory migrate`, and a migration refuses a schema a newer
toolchain wrote.

`hale dna dev` migrates first and hands the host the spine's DSN
alone. `hale dna run` migrates nothing: give it
`HALE_DNA_MEMORY_DSN_SPINE`, and it keeps any owner's or head's DSN
away from the host. Without the spine's DSN the host says so and runs
with no memory, projecting and admitting nothing; on a record that has
adopted the ledger, `run` refuses to start at all.

### Shared records

Over a record several organizations share, each owner's heads write as
that owner's role. Give the owners' keys to the migration
(`HALE_DNA_OWNER_KEYS`, `<owner>=<key> …`) and it prints one more line
per owner, `HALE_DNA_MEMORY_DSN_HEAD_<OWNER>=<dsn>`; each owner's heads
take theirs as `HALE_DNA_MEMORY_DSN_HEAD`. An owner's role writes only
in its own members' names, and who is a member is the graph's
([below](#the-graph)).

## The ledger

Moving the day's work into memory is an explicit, one-way step that a
node carries out:

```sh
hale dna ledger                          # the routing, the memory named here, the cutover
hale dna ledger rows                     # the ledger, one JSON object per line
hale dna ledger adopt
hale dna ledger abandon --why "back to one memory"
```

On a new organism:

```text
$ hale dna ledger status
routing:    0 (every row in the record)
memory:     none named here (HALE_DNA_MEMORY_DSN_HEAD)
```

`adopt` syncs the record and appends `ledger.adopting`. On its next
tick a node claims the ask, copies every operational row of the
record into the ledger keyed by its commit (interrupted, it is run
again, never repaired by hand), carries its body lease into memory,
and appends `ledger.adopted` naming the checkpoint. From that commit
on, an operational row never goes into git. `abandon` appends
`ledger.abandoning`; a node empties the ledger and appends
`ledger.abandoned`, and the organism is on the record alone again. The
record's own rows are never removed, so nothing is lost either way.

Run an organization with its ledger adopted. On a git-backed record
every row of the day's work is a commit, and an execution costs tens
of them.

### Every writer writes, memory is the gate

Once adopted, a verb that writes the day's work in your name writes it
straight into the ledger as your head's role, through memory's insert
function, `ledger_append`. The row lands, or it is refused with the
reason, there and then; there is nothing to wait on:

```text
$ hale dna task done acme:t4 --as alice
hale dna: `task.done acme:t4` was not written to the ledger: task acme:t4 is handed to bob, not to alice; a completion is admitted in the assignee's name
```

The gate refuses a row when:

- its kind is not the ledger's (it goes to the record instead);
- the person it names has retired;
- the calling role does not write as that author;
- it accepts a transfer outside the owner the task was offered to;
- it completes a task in any name but the assignee's;
- it names a lease that is not live at that epoch (`fenced`);
- it was decided at a revision the ledger has moved past
  (`stale_revision`), or claims an entity already claimed.

A write carries a request id, so one retried after a lost answer lands
once. A head with no memory named writes nothing once the ledger is
adopted: the verb says so and never falls back to git.

`hale dna status` carries a `memory:` line on every organism:

```text
memory:     the record alone (routing 0); `hale dna ledger adopt` moves the day's work to the ledger
```

When memory stops answering the same line says `THE LEDGER IS
UNREACHABLE (…)`: what you read is the last projection, and nothing
is admitted until it answers.

## The graph

The graph is what memory understands of the record. Every node of
[the spine](./spine.md) projects the record into it on its tick:
ratified knowledge and its bindings, the code's structure as `init`
observed it, the concerns and pressure raised per source, the
repository's nodes and edges (`graph.node`, `graph.edge`), and who
retired. Each record row is one transaction that moves memory's stamp
by compare-and-swap before the row's effects, so any number of nodes
over one record converge on one graph with each row applied once, and
a projection interrupted mid-row leaves nothing of it. Nothing else
writes the graph.

There is one org chart, and it is this graph: its positions, the
organizations that own them, and who holds them, all `holds` edges.
Read its two perspectives with:

```sh
hale dna show org               # every position, what it sits under, who holds it, what it signs
hale dna show processes         # every process, where it meets another, what runs it
```

Both read memory under the head's role (`HALE_DNA_MEMORY_DSN_HEAD`;
without it they refuse and say so) and take `--json`. When memory is
behind the record they say how far on stderr (`memory has projected N
of the record's M rows`) and show the graph as of that row. How positions are filled and routed is [The head and the
face](./head.md) and [Shaping and governing it](./shaping.md).

### Reading the repository again

`hale dna init` reads the repository once. `hale dna ingest [--at
<rev>]` reads it again at a commit (`HEAD` by default) with the same
ingest, and files what differs from the graph the record states as one
Board Review, listed under `ingest`: nodes and edges added, bodies
changed, and what is gone as retirements.

```text
ingested 3f2a…c1: 6 added, 0 changed, 3 retired; the Board ratifies it as k:… (`hale dna review` lists it under `ingest`)
```

Every operation of a surface the repository serves is a node of kind
`tool`, `tool:<Surface>::<op>`, naming its contract and the roles it
requires: the tools an organization's positions may come to use, found
in the graph. A `graph.ingested` row names the commit either way. The
same difference at the same commit is proposed once, and once the
Board ratifies it, reading that commit again says
`the graph the record states is the repository's; nothing to review`.

### What a Work is told, and by which node

An idea is bound to a **target**, and a target is a locus path or a node
of this graph. A path (`org`, `org/<child>`) reaches itself and
everything under it. A node id (`language:hale`, `system:dna`,
`position:leader`) reaches exactly that node: `language:hale` is not
`language:hale-x`, and a prefix of an id is nothing. The package a Work's
hat carries is read for a *set* of targets, in one query, and an idea
that several of them reach arrives once. The hat's set is the Work's
path; the languages the attached application is written in (its
`written_in` edges); `system:dna` instead of the languages when the Work
is a change to the organization itself; and the performer's position, so
a mandate bound to the position arrives with the Work. The hat's JSON
names them as `targets`, and they are part of its digest. A reviewer reads
the same way: `hale dna review <id>` lists, under `knowledge for …`, the
ideas ratified for the node the Review is bound to (or `system:dna` for a
change to the organization), when memory is there to ask.

`init` writes the two nodes everything hangs from, `language:hale` and
`system:dna`, and for an attached application the node
`application:<name>` with a `written_in` edge to the language. It also
proposes the toolchain's library: the book per chapter and the spec per
section, one idea each, generated from the tree when the toolchain is
built (a new chapter joins the library by existing). The language's
chapters and sections are bound to `language:hale`; the DNA chapters
(`docs/src/dna`, `docs/src/parts`, `spec/dna.md`, `spec/model.md`) to
`system:dna`. The Board does not review hundreds of ideas one by one: the
library is two **families**, `library/language@<version>` and
`library/design@<version>`, and each is one Review whose body names the
ideas it ratifies. Approving a family ratifies every idea in it and binds
each to its node; rejecting it refuses them all. Nothing arrives in a
Work's brief until the Board decides:

```sh
hale dna review                      # two library families, listed beside the practices
hale dna review library approve      # ratify both families, and so bind every idea
hale dna review <family id>          # the ideas a family ratifies, by name
hale dna init --no-library           # a record with none of it (also `new --no-library`)
```

What a brief pulls from the library is what is bound to the node it is
about, ranked, within the package's budget: a Work on the language reads
the few chapters and sections of `language:hale` that matter to its ask,
not the whole book, and a change to the organization reads the ones bound
to `system:dna`. An idea bound to everyone would crowd out what the work
needs, which is why a chapter is bound to the node it is about and not to
`org`.

When the toolchain moves, `hale dna upgrade` proposes the new version's
library as new families. Each idea names the digest it replaces, and
approving the family retires the old ones: until then an application keeps
the bindings of the version it was seeded at.

An idea whose target is a node the record has no node for is refused,
naming it.

## Between records

Sync copies your whole record to people inside it. Work that leaves
the organization goes to another record through a **connection**,
which says what may cross:

```sh
hale dna connect git@example.com:acme/record.git --name acme --as accountant \
    --purpose "year-end books" --classes internal,customer
hale dna handoff acme task t7
```

- `connect` reads the other record's identity through its git remote,
  and nothing else of it; a service URL (`http://…`, `https://…`) is
  refused. It opens a Board Review, and the connection is in force
  once a Board verdict from someone other than its proposer approves
  it. `hale dna connect` alone lists connections.
- `handoff <name> task <id> | receipt <digest>` writes one envelope
  into this record's mailbox in the other record,
  `refs/dna/exchange/<this record's identity>`: the fact, where it came
  from, its history here and its purpose. Only a handed task crosses.
  A receipt crosses as its digest and class, never its body, and only
  if the connection carries its class. A retried handoff is one
  envelope.
- The other side admits it only through a connection of its own back
  to you, and accepts it with `hale dna handoff accept <id>`. Your
  `hale dna handoff sync` reads the acceptance back, and only then
  does your task settle.
- `hale dna disconnect acme --why "the engagement ended"` stops
  anything further crossing. Both records keep what already did.

Neither record reads the other's journal. This clone's cache for a
connection holds the other record's identity and this record's own
mailbox there, nothing else.
