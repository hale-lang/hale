# Memory and the record

The organism remembers in two places. The **record** is its long-term
memory: every decision, in order, never rewound. It is rows in
Postgres, one signed row per event, and anyone with a clone writes to
it in their own name. **Memory** is also its working memory: the day's
work (the ledger), the graph the org chart is read from, and protected
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
| record | how the organism changed and was allowed to: mutations and their Reviews, practices proposed and ratified, grants, connections, people | Postgres, `record_rows` in the record's schema |
| ledger | what the organism did today: intents, tasks, cases, workflow executions and attempts, bills and receipts, money reserved and settled, schedules, concerns, readings, effect claims, leases | Postgres, the record's own schema |

The table is `memory_of` in `dna/core/routing.hl`, and it is the only
routing there is: every record has its ledger from its first row
([below](#the-ledger)). `hale dna history` and the projections read
both memories as one sequence.

## The record

### Where the record lives

The record is the table `record_rows` in the record's schema,
`dna_<identity>`. Every clone and every body reads and writes it
there, so nothing of it travels by git and there is nothing to sync.
Each row has a `seq`, a `kind`, an `entity`, a `body`, an `author`,
the `writer` (the role of the session that wrote it), the `prev` row's
digest, its own `digest` and its `sig`.

Candidates and deployed revisions are the genome's, not the record's:
a candidate is the branch `dna/candidate/<mutation>`, a revision a
deploy published is the branch `dna/revision/<rev>` on the forge, and
the pull of the forge's default branch lands in
`refs/hale/remote/genome`.

A row's digest is `sha256(prev|kind|entity|body)`, and the first row's
`prev` is `genesis`, so the rows are a chain. `hale dna status` says
`chain verified at <head>` when every digest checks. `hale dna
record-rows` prints the rows, oldest first, one JSON object per line:
`seq`, `kind`, `entity`, `body`, `author`.

The record's identity is the digest of its first row,
`record.genesis {genome, nonce}`: the genome's root commit and a nonce,
so two projects made alike are two records. It is the same in every
clone and different for every record. Memory's schema and the nerves'
token are both named after it (`dna_<identity>`).

The genome pins it. `hale dna init` writes it into `hale.toml` as it
composes the record:

```toml
[dna]
record = "4f1c…"   # the record's identity, written by init
```

A clone knows its record from the genome. A second clone reaches the
record through memory: its pin names the schema, and `hale dna memory
migrate` (or `hale dna upgrade` with the owner's DSN) keeps the head's
DSN there. A pin whose genome is not the one this clone's history
holds — a `hale.toml` copied from another project — is refused.
Remove the line from a copied manifest and run `init` again.

`hale dna init` appends two rows to the record it provisions:
`record.genesis`, then `record.key {key}`, the public half of the
record's key.

A clone that still holds a record an earlier toolchain kept on git
(`refs/dna/journal`) is refused by `init` and `upgrade`. Nothing reads
it and nothing carries it forward; see
[Troubleshooting](./troubleshooting.md).

`.hale/dna/` is not the record. It holds the status projection,
worktrees and the toolchain's scratch; delete it and nothing the
record holds is lost.

### Append is insert-only

Every writer appends through a memory function: `record_append` for
one row, `record_append_rows` for a batch made visible together. The
function takes the schema's advisory lock and lands the row only on
the `prev` the writer read. No role may update, delete or truncate a
row of the record. A writer that lost the race reads the new tail and
appends again; `seq` is the row's position once it lands, never a
promise made before. Two people answering at once both land.

A read of the record either happened or failed. A read that failed
(memory out of reach for a moment) is never taken for an empty or a shorter record: the reader
keeps what it had and tries again, and a verb refuses with the reason
rather than act on a record that is not there.

### Who wrote a row, and whether it counts

Every row is signed by its writer. The organism's parts (`dna`,
`spine`, `host`, a node, a leg) sign with the record's Ed25519 key,
which lives in the vault as `dna-exchange-<schema>`. A person signs
with their own ssh key (`ssh-keygen -Y sign -n hale-dna-record`), the
key git's `user.signingkey` names under `gpg.format ssh`; a person with
none configured writes an unsigned row. The organism's own rows carry
its configured author; a person's rows (a verdict, an ask, a node's
report) carry the identity of whoever ran the command, and `--as <who>`
names the person on the verbs that take it.

Whether a row counts is a check every reader makes,
`dna::row_admissible`, under the clone's `dna.trust`:

- `local`, the default and what `hale dna new` declares: every writer
  to the record is trusted.
- `signed`: a person's row counts only when a key enrolled for them
  verifies its signature. A node answers an unverified request with a
  refusal (`unverified writer`) and never relays it; the projection
  into memory moves past it without applying anything of it.

Enrol a person's key with `hale dna signers enroll <who> --key <ssh
key or .pub file>`, revoke with `hale dna signers revoke <who>`, and
list with `hale dna signers`. Enrolment and revocation are rows
(`signer.enrolled {who, key}`, `signer.revoked {who}`), and each counts
only when signed by someone the record already admits; the first
enrolment is the bootstrap.

### Sync

```sh
hale dna sync
```

The record needs no sync. `hale dna sync` applies what the record says
of this clone, which today is deleting the branch of a dropped
candidate, and says so:

```text
$ hale dna sync
record: the record is memory's: nothing to sync; 38 event(s)
```

### Candidates

Every candidate a Mutation commits is kept as a branch of the genome,
`dna/candidate/<mutation>`, and the record names its commit, so a
refused or revised change stays readable after its worktree is gone.
It is code, and travels as any branch does:

```sh
hale dna candidates                            # every kept candidate, with its Review's state
hale dna candidates m7                         # one, as a diff from its Review's base
hale dna candidates drop m7 --why "superseded by m9"
```

`drop` is a row, `candidate.dropped`; the branch goes here, and every
other clone deletes its own at its next `hale dna sync`. A candidate branch
someone pushed to a forge is deleted there like any other branch. The
commits stay in each object store until git collects them.

## Receipts and protected evidence

A receipt is evidence kept by the digest of its content: a
verification step's output, a diff document, a bill you filed. Rows
name receipts by digest. Every receipt has a class:

- `public` and `internal` bodies are
  kept in memory by digest, unsealed.
- `customer` and `confidential` bodies are **protected**. They are
  kept in memory alone, sealed there, and the record gets only the
  digest and the class (`receipt.classified`). With no memory to keep
  it, the body is withheld (`receipt.withheld`) and nothing holds it.

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
byte is erased. An unsealed body is erased from memory at once; a
protected body is erased by a node on its next tick. A copy made
outside the record cannot be recalled.

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

The clone keeps the head's line in `.hale/dna/memory.env`, so the
`hale dna` verbs you run there afterwards read and write memory as the
head without being told; a DSN in the environment still wins. The file
is the head's alone, mode 600, under `.hale/`, which git ignores; a verb
refuses it if its mode lets anyone else read it.

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

The ledger is the day's work. Memory's migration makes it with the
record, so every record has it from its first row and no row says so:

```sh
hale dna ledger rows                     # the ledger, one JSON object per line
```

### Every writer writes, memory is the gate

A verb that writes the day's work in your name writes it
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
once. A head with no memory named writes nothing: the verb says so.

`hale dna status` carries a `memory:` line on every organism:

```text
memory:     record + ledger (operations and leases live in memory)
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

A graph row memory will not take, malformed or naming nodes of the
wrong kind, whether written straight to the record or carried by a
proposal the Board ratified, is refused as a row of its own:
`graph.refused` with the row it was at and why, `by: memory`. The
projection goes on past it, so one bad row never holds back the reads
that wait for memory to catch up. Only the database failing stops the
projection, at that row, until it answers.

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

Memory gives your whole record to people inside it. Work that leaves
the organization goes to another record through a **connection**,
which says what may cross. Records meet on their nerves:

```sh
hale dna connect nats://acme.example:4222 --record 4f1c… --name acme \
    --as accountant --purpose "year-end books" --classes internal,customer
hale dna handoff acme task t7
```

- Every record publishes a **card** on its nerves (`hale dna nerves
  migrate` does it): its identity, its genome and the key it signs
  with, signed by that key. `connect` reads the card of the record you
  name (its identity is `[dna] record` in its `hale.toml`) and pins its
  key, and nothing else of it. It needs the card, not a running peer.
  It opens a Board Review, and the connection is in force once a Board
  verdict from someone other than its proposer approves it.
  `hale dna connect` alone lists connections.
- To deliver into another record's inbox you need its **inbox key**,
  the password of its nerves' `exchange` user: its operator hands it to
  yours, who keeps it in your vault as `nats-<its org>-exchange`.
- `handoff <name> task <id> | receipt <digest>` signs one envelope with
  your record's key and delivers it to the other record's inbox: the
  fact, where it came from, its history here and its purpose. Only a
  handed task crosses. A receipt crosses as its digest and class, never
  its body, and only if the connection carries its class. A retried
  handoff is one envelope. An inbox bounds what waits in it undrained,
  per sender; a full one refuses the handoff, and nothing is recorded
  here. What the other side has drained leaves its inbox, so a record
  that reads its inbox takes any number.
- The other side admits it only through a connection of its own back
  to you, under the key that connection pinned (an envelope claiming you
  under another key is dropped, or kept beside yours and never
  admitted), and accepts it with
  `hale dna handoff accept <id>`. Your `hale dna handoff sync` reads the
  acceptance back, and only then does your task settle.
- `hale dna disconnect acme --why "the engagement ended"` stops
  anything further crossing. Both records keep what already did.

Neither record reads the other's record: a connection reads one card,
and an inbox holds only the envelopes sent to it. A connection
proposed before the nerves exchange pinned no key and is proposed
again.
