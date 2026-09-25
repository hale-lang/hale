# Operating the fleet

The organization can run on your workstation while the code it
governs runs on a fleet. The fleet is described by the plan Hale
already checks; each machine runs a node that expresses what the
record says; a deploy is a row in the record; and an approval's
window is over every instance the change touched.

## The plan

```json
{"schema": "1.2", "name": "production", "instances": [
  {"id": "api-0", "seed": "services/api", "node": "edge-1", "labels": ["api"]},
  {"id": "api-1", "seed": "services/api", "node": "edge-2", "labels": ["api"]},
  {"id": "worker-0", "seed": "services/worker", "node": "edge-1", "labels": ["worker"]}]}
```

An instance names the **seed** it is built from and the **node** that
runs it. Two instances of one seed on two nodes are two replicas.
Routes, groups and claims are the fleet's own — see [Multiple
binaries](../services/multi-binary.md) — and `hale fleet check`
composes the plan from artifacts cut from the seeds, so a change to
one service that breaks a claim over all of them is caught before
anyone reviews it.

```toml
[fleets]
production = "ops/production.plan.json"

[dna]
fleet = "production"          # the arrangement the DNA expresses
```

Services that are not Hale — a database, a queue — are environment:
they appear in the plan as endpoints the Hale services bind to, and
the DNA never edits them.

## The nodes

On each machine, a clone of the repository and one node:

```sh
git clone git@example.com:you/chat.git && cd chat
hale node edge-1                          # --repo <clone> --fleet <name> --tick <ms>
```

```text
hale node edge-1: expressing from /srv/chat (record via origin)
hale node edge-1: instance api-0 up (pid 4123) at 232dc8f18dde as 3c9b9327e480
hale node edge-1: fleet.deploy #41 expressed (2 instance(s) started)
```

A node syncs the record every tick, reads the latest deploy, and
when that names a revision it does not express yet, fetches it,
checks it out, builds the instances the plan assigns to it that the
deploy touches, restarts them, and reports each one into the record
in its own name: `instance.up` with the revision, the model hash and
the build digest; `instance.exited` with the code when one goes.
That is all it does. It never decides.

## Deploy, and what the record shows

```text
$ hale dna deploy HEAD
fleet.deploy #41: production at 232dc8f18dde touching api-0 api-1 worker-0

$ hale dna fleet
fleet production (ops/production.plan.json)
last deploy: deploy by operator 232dc8f18dde (232dc8f18dde) touching 3 — the whole genome
  api-0        edge-1     up       rev 232dc8f18dde model 3c9b9327e480
  api-1        edge-2     up       rev 232dc8f18dde model 3c9b9327e480
  worker-0     edge-1     up       rev 232dc8f18dde model 91ac4e0b7d21
```

`hale dna fleet` is read from the record, from any clone: every
instance of the plan, its node, whether it is up, the revision and
model hash it last came up at, and the deploy that asked. Two
replicas reporting two different hashes is something you can see.

## An approval, on the fleet

Under `hale dna run` with `[dna] fleet` set, an approval does what
`dev` does on one machine, over the fleet:

```text
hale dna run: m1 requests a restart (apply bf94e503c1c0… seed services/api fitness …); deploying to the fleet `production`
hale dna run: m1: fleet.deploy apply bf94e503c1c0 touching api-0 api-1
hale dna run: m1: 2 instance(s) observed healthy for 15s as 8517c3db7499
```

The host writes a deploy row naming the candidate and the instances
whose seed the change edited — `api-0` and `api-1`, not the worker —
and waits: every touched instance must come up at the revision, and
then none may exit for the observation window. All up and quiet:
`healthy`, and the change is retained. One falls over:

```text
hale dna run: m1: instance api-1 on edge-2 exited (137) inside the observation window
hale dna run: m1: fleet.deploy rollback to 232dc8f18dde touching api-0 api-1
```

`expression.crashed` names the instance and its node, the
organization rolls the genome back to the base, and the rollback is
another deploy row every node answers. Both replicas come back at
the base. `hale dna history m1` has the instance's name in it.

```sh
hale dna rollback m1        # the same rollback row, by hand, later
```

## Your own pipeline instead

If the fleet already has a deployer — a script, Nomad, Kubernetes —
wire it in as the deployment gateway in `dna/org/main.hl`:

```hale,fragment
deployment: dna::ShellDeployment { command: "ops/deploy.sh", seed: "services/api" },
```

The command is called as `ops/deploy.sh express <candidate> <seed>`
on an approval and `ops/deploy.sh rollback <base> <seed>` after a
bad window or a rejection. It owns expressing *and* judging: exit 0
from `express` means up and healthy, anything else is the
observation that rolls back. No host is asked; the organization
expresses through the command in its own handler and the record
says `expression.deployed`.

## Memory

Memory is Postgres: the ledger, the knowledge graph and protected
evidence, each record in a schema of its own, `dna_<identity>`, where
the identity is the sha of the record's first commit. Nothing stands
in front of it — a process that reads or writes memory opens Postgres
itself, as a role, and the role's grants are the trust boundary.

Three DSNs, three jobs:

| Variable | Who holds it | What it may do |
|---|---|---|
| `HALE_DNA_MEMORY_DSN_OWNER` | `hale dna memory migrate`, `hale dna dev`, `hale dna upgrade`, and a provisioned body's env file (whose `hale dna dev` migrates) | apply the schema, and nothing else |
| `HALE_DNA_MEMORY_DSN_SPINE` | the host that runs the organism, and the organization it starts | read and write the record's tables, through the functions for protected evidence; no DDL |
| `HALE_DNA_MEMORY_DSN_HEAD` | a head: the CLI, `hale dna ui`, the read API | read; take or fence a lease; file and read protected evidence through the functions |

Apply the schema with the owner's DSN, and the command prints the
other two:

```text
$ HALE_DNA_MEMORY_DSN_OWNER=postgres://dna:dna@db.internal:5432/dna hale dna memory migrate
HALE_DNA_MEMORY_DSN_SPINE=postgres://dna_9f3c…_spine:dna_9f3c…_spine@db.internal:5432/dna?sslmode=prefer
HALE_DNA_MEMORY_DSN_HEAD=postgres://dna_9f3c…_head:dna_9f3c…_head@db.internal:5432/dna?sslmode=prefer
```

The two roles, `dna_<identity>_spine` and `dna_<identity>_head`,
belong to the record: a role granted on every record's schema would
read every other record's evidence on the same server. Until the
vault holds their credentials (#989) each role's password is a
placeholder equal to its name, so keep the database where only the
people and machines you trust can reach it.

The migration is one transaction and can be run again at any time.
It writes a schema version (version 2), and every store checks it when
it opens: a host whose memory is at another version refuses to start,
naming both versions and `hale dna memory migrate`, and a migration
refuses a schema a newer toolchain wrote. Migrating a version-1 memory
keeps its leases (they become rows of the `claims` table) and empties
the knowledge graph, which the projectors then rebuild from the record:
the graph is derived, and version 2 applies it a row at a time.

`hale dna dev` migrates first — with `HALE_DNA_MEMORY_DSN_OWNER`, or
the database `dna/compose.yaml` brings up — and hands the host only
the spine's DSN; the owner's and the head's are taken out of its
environment. `hale dna run` migrates nothing: give it
`HALE_DNA_MEMORY_DSN_SPINE`, and it strips any owner's or head's DSN
before the host sees them. Without the spine's DSN the host says so
and runs with no memory — nothing is projected or admitted — and on
a record that has adopted the ledger, `run` refuses to start at all.

Each process holds one handle on memory, opened on first use and
closed when the process ends; a store closes its connection when it
dissolves. However long a host runs, it holds a fixed handful of
connections.

### The spine is every node

The spine is not a process. Every host that runs a body is a node of
it, any number of them, and a node holds nothing: its state is the
stores, its code is the genome. On every tick — once a second — each
node projects the record into the graph and the org chart, carries out
an adoption the record asks for, and erases the evidence the record
says was redacted. Nothing picks one of them. The stores keep them from
doing anything twice: the projection moves a row at a time by
compare-and-swap, an adoption is claimed by the row that asked for it,
and an erasure is by digest. On a shared record every owner runs a body
(each under its own lease, `owner/<owner>`), and every one of them is a
node.

Nor does a node admit anything. A head writes its rows straight into
the ledger (below), and memory's insert function decides, there and
then, whether the row lands.

When the record has a remote, a node projects only what the remote
holds after that tick's sync — and nothing on a tick whose sync did not
finish — so memory's stamp is always a commit every clone can receive.

### The genome pull

Nodes take a new genome by pulling it; nothing pushes it to them. Every
five minutes (`HALE_DNA_GENOME_POLL`, in seconds) a node fetches the
forge's default branch. When it moved, the node stops cleanly and exits
with code 75, and its unit — `Restart=always`, as `hale dna body
provision` writes it — starts it again; at start it moves to the new
genome and builds it:

```text
hale dna dev: genome: the forge's default branch is at 9908091716f6 and this node runs 63c447c47fdc; restarting to take it
…
hale dna dev: genome 9908091716f6, pulled from the forge every 300s
```

A genome that does not build does not take a node down: the node
records `node.build_failed`, goes back to the last genome that built,
and keeps serving it, and it does not restart for that genome again.
`hale dna status` says what each node runs:

```text
genome:     9908091716f6 on you@build-1:/srv/chat (8aa6251345a7 did not build)
```

Push the fix to the default branch and every node takes it within the
interval.

### Claims by id

Every act a node takes that must not be taken twice is claimed in
memory first, by id, with an expiry: the `claims` table, one row per
key, taken with one conditional write, so of two nodes racing for a key
one wins and the other finds it taken. Nothing coordinates the nodes;
the table does, on the database's clock, so nodes whose clocks
disagree still agree on when a claim expires. An organization claims an ask (`plan/<intent>`) before
its leader is asked to plan it — a model call is spend — and gives the
claim back once the plan's admission is recorded; a node that finds
the ask claimed leaves it (`planning elsewhere: <holder> holds
plan/<intent>`), and is offered it again with the host's relay. A node
that dies holding a claim leaves it to expire — five minutes for a
plan — and the next node offered the ask plans it. The optimize pass
claims its window the same way, so one node runs it per window. A node
is named in its claims by the body's holder (`HALE_DNA_NODE`, which the
host sets). The record shows each claim a node acted on and its return:

```sh
hale dna history plan/i7
# claim.taken plan/i7 {"holder": "you@build-1:/srv/chat", "token": 1, "until": …}
# claim.taken plan/i7 {"holder": "you@build-2:/srv/chat", "token": 2, "until": …}
# claim.released plan/i7 {"holder": "you@build-2:/srv/chat"}
```

The body lease is one row of the same table.

### Protected evidence and its key

A customer or confidential body is sealed inside memory under the
receipt key. Give it at migration: `HALE_DNA_RECEIPT_KEY` in the
owner's environment, sixteen characters at least. It is written once
into the owner-only table `memory_keys`; a different key later is
refused, because bodies sealed under the first would no longer open.
No process that runs ever holds it. Three functions, run as the owner,
are the whole surface of the sealed table: `receipt_file` and
`receipt_read`, which heads and the spine may call, and
`receipt_erase`, the spine's alone, which deletes a body and keeps its
digest, so a body that was redacted is refused if anyone files it
again. The spine's role reaches `memory_keys`, `protected_receipts`
and `protected_redactions` only through those functions; the head's
may select from the ledger, the graph and the meta tables, and writes
no table directly but the claims.

**A dump of the database is as sensitive as the evidence.** It carries
the key (`memory_keys`) and the ciphertext together. Protect and
retain dumps as you would the bodies themselves; #989 revisits where
the key is held.

## The nerves

The organism's parts tell each other what the record asks over **NATS
JetStream** (GH #986). A request — a task asked for, a verdict, a
concern, pressure, a practice — is a row first, written by whoever
asked, from any clone. The node running the organism
(`hale dna run` or `dev`) publishes every row the record has not
answered, and again every 30 seconds until the answer is there. The
organization reads them from its stream. Nothing a person runs
publishes anything itself.

One server can carry several organizations: each has its own stream,
`DNA_<ID>`, over the subjects under its token (`dna_<id>.dna.…`, the
same name memory gives its schema), kept for a week. Four credentials,
one per family of subjects:

| Variable | Who holds it | What it may do |
|---|---|---|
| `HALE_DNA_NATS_URL_OWNER` | `hale dna nerves migrate`, `hale dna dev` | create or update the stream, and nothing else |
| `HALE_DNA_NATS_URL_SPINE` | the host that runs the organism, and the organization it starts | publish and read the organization's own facts (`<org>.dna.>`) |
| `HALE_DNA_NATS_URL_HEAD` | a head | subscribe; publish nothing |
| `HALE_DNA_NATS_URL_APP` | an application (#987) | publish on its own subjects (`<org>.app.<app>.>`) |

Create the stream with the owner's URL, and the command prints the
organization's token and the others:

```text
$ HALE_DNA_NATS_URL_OWNER=nats://owner:…@nats.internal:4222 hale dna nerves migrate
HALE_DNA_NATS_ORG=dna_9f3c…
HALE_DNA_NATS_URL_SPINE=nats://spine:…@nats.internal:4222
HALE_DNA_NATS_URL_HEAD=nats://head:…@nats.internal:4222
HALE_DNA_NATS_URL_APP=nats://app:…@nats.internal:4222
```

`hale dna dev` does this first — with `HALE_DNA_NATS_URL_OWNER`, or the
`nerves` service `dna/compose.yaml` brings up — and hands the host the
spine's URL and the token alone. `hale dna run` creates nothing: give
it `HALE_DNA_NATS_URL_SPINE` and `HALE_DNA_NATS_ORG`, and it strips an
owner's or a head's URL before the host sees them. The server needs
JetStream on (`-js`, or `jetstream {}` in its configuration);
`dna/nats.conf`, which `init` writes, is the configuration
`dna/compose.yaml` runs, with each user allowed only its own subjects.
Until the vault holds their credentials (#989) the passwords are
placeholders (`dna-<role>-dev`), so keep the server where only the
machines you trust can reach it.

The organization reads through its durable consumer, `spine`, which
keeps its place across restarts, so a fact published while it was down
reaches it when it is back. Over a shared record each owner's host and
organization have a space of their own on the stream
(`<org>.<owner>.…`, the durable `spine_<owner>`), so one owner's
organization never takes another's facts. The host waits at start for
the organization to be reading, and says so. A publish the stream does
not acknowledge in time collapses the host's connection: the host
writes `nerves.lost`, stops, and exits 75 for its unit to start it
again, and the new node relays every request still unanswered, because
the row, not the publish, is the fact. Without the spine's URL the host
says so and runs, and the organization hears nothing the record asks.

A fleet node is the one exception: its instances hand their concerns
to it on a Unix socket of its own (`.hale/node/concern.raised.sock`),
because the application declares no bindings yet. The node writes each
concern into the record, and the host relays it like any other request
(#987 moves the application onto the nerves).

## The two memories, in operation

An organism that has adopted the ledger runs on two memories at
once — the record in git, the day's work in memory — and one verb
family is how you see and move between them. [The record](./record.md)
says what lives where; this is what you type.

```sh
hale dna ledger                 # routing, the memory named here, the cutover
hale dna ledger rows            # the ledger, one JSON object per line
hale dna ledger adopt           # ask the body to move the day's work into memory
hale dna ledger abandon --why "back to one memory"
```

`hale dna ledger` on its own says which routing this record is on,
whether memory is named here, and what the ledger holds:

```text
routing:    1 (the day's work in the ledger, adopted at 232dc8f18dde)
memory:     named here (HALE_DNA_MEMORY_DSN_HEAD)
ledger:     413 row(s), cutover at 232dc8f18dde
```

`adopt` and `abandon` are asked in the record. `adopt` syncs the record
and appends `ledger.adopting`; on its next tick a node claims the ask
(`ledger/<the ask's digest>`), copies every operational row of the
record into the ledger keyed by its commit, carries its own body lease
into memory at its token, and appends `ledger.adopted` naming the
checkpoint. Interrupted anywhere, it is rerun rather than repaired by
hand. `abandon --why …` appends `ledger.abandoning`; a node empties the
ledger and appends `ledger.abandoned`, and the organism is on the
record alone again — the record's own rows are never removed from git,
so nothing is lost either way.

Once adopted, a head writes the day's work straight into the ledger, as
its own role, and memory's insert function is the gate: the row lands,
and the verb says it is done, or it is refused and the verb says why —
a retired person, a task handed to someone else, a transfer accepted
outside the owner it was offered to, an author this head does not
write as, a claim already taken. There is no request to wait on.

`hale dna status` carries a `memory:` line on every organism,
adopted or not:

```text
memory:     the record alone (routing 0); `hale dna ledger adopt` moves the day's work to the ledger
memory:     record + ledger (routing 1, adopted at 232dc8f18dde; operations and leases live in memory)
```

and when memory stops answering, that same line says so: `THE
LEDGER IS UNREACHABLE (…): what is read here is the last projection,
and nothing is admitted until it answers`.

### Shared records and owners' roles

Over a record several owners share, each owner's heads write the ledger
as that owner's role in memory, and a role is who a head is: memory
lets an owner's role write only in its own members' names. Give the
owners' keys (`HALE_DNA_OWNER_KEYS`, `<owner>=<key> …`) to the migration
and it makes a role per owner and prints its DSN:

```text
$ HALE_DNA_OWNER_KEYS="acme=… north=…" hale dna memory migrate
HALE_DNA_MEMORY_DSN_SPINE=postgres://…_spine:…
HALE_DNA_MEMORY_DSN_HEAD=postgres://…_head:…
HALE_DNA_MEMORY_DSN_HEAD_ACME=postgres://…_head_acme:…
HALE_DNA_MEMORY_DSN_HEAD_NORTH=postgres://…_head_north:…
```

An owner's heads take theirs as `HALE_DNA_MEMORY_DSN_HEAD`; the plain
head role of a shared record reads and writes as no one. Memory holds
no key: the keys stay the record's until the vault holds them (#989).
Which members an owner has is the genome's owners map, `dna/org/owners`,
which every node projects into memory, so changing it is a reviewed
change of the organization like any other. A host is a principal too:
it writes its own rows as `host`, and its `body.*` rows carry
`"owner"`, so the record still says whose body it was.

### Connections

`hale dna connect` exchanges handoffs with another record through
that record's git remote only: this record's mailbox there,
`refs/dna/exchange/<sender>`, and its identity, `refs/dna/identity`.
A service url (`http://…`, `https://…`) is refused. [The
record](./record.md) walks through a handoff.

## What is not here yet

A Review's semantic diff is the edited seed's; the deploy row names
the instances it reaches, but there is no fleet-level structural
diff. Pressure from services' typed metrics has a spelling
(`hale dna pressure raise`) and nothing raising it for a node yet.
And the organization is not itself an instance of its plan.
