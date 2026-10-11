# The nerves

The nerves carry signals between the organism's parts: an ask, a
verdict or a leg's outcome the record already holds, and an
application's event on its way to becoming a row. They are a NATS
JetStream server on the private tier, with one stream per organization
and one user per role, each allowed only its own subjects. They carry
signals, never facts: a request is a row before it is sent, an event
is a row before anything acts on it, and a message the stream loses
costs a relay, never the row. When the stream stops acknowledging, the
connection's delivery closure is violated, the node records it and
stops, and its unit starts it again. The nerves hold neither the
record nor memory, and no device reaches them.

## Row first

Nothing a person runs publishes anything. `hale dna task create`, a
verdict, `hale dna concern raise`, `hale dna pressure raise`,
`hale dna practice propose` and `hale dna schedule declare` each write
their row into the record, from any clone, and return. A **node**
([The spine](./spine.md#a-node)) is the one that publishes: while it
runs it is its program's `main locus`, with every DNA topic it
publishes bound to pond's NATS adapter, and on every tick it relays
each request row it holds that the record has not answered.

The row is the fact and the answer is a row. A message never admits
anything; the part that reads it decides, and writes what it decided.

## The server

`dna/compose.yaml` runs the nerves as its `nerves` service
(`nats:2`, on the loopback, a host port in 42xx taken free when the
seed was made), configured by `dna/nats.conf`. Beyond one machine,
run a server of your own with JetStream on (`-js`, or `jetstream {}`
in its configuration), give it `dna/nats.conf` and
`dna/nats.secrets.conf`, and name it with `HALE_DNA_NATS_URL_OWNER`.

`dna/nats.conf` is tracked and holds no password: each user's password
is a variable of `dna/nats.secrets.conf`, which the bootstrap writes
from the vault, mode 600 and ignored by git. For a project named
`refproj` the users are:

```text
    # creates the organization's stream; held by no process that runs it
    { user: owner, password: $NATS_OWNER_PASSWORD }
    # the spine: a node's host publishes the organization's facts, and the
    # organization reads them through its durable consumers (`spine`, an
    # owner's `spine_<owner>`, and `heart` for the applications' events).
    # `*` stands for the organization's token: this server carries one
    # organization. A node also tells the heads every row it lands.
    { user: spine, password: $NATS_SPINE_PASSWORD,
      permissions: { publish: ["*.dna.>", "*.*.dna.>", "*.head.>", "*.*.head.>", "$JS.API.CONSUMER.CREATE.*.*", "$JS.API.CONSUMER.INFO.*.*", "$JS.API.CONSUMER.MSG.NEXT.*.*", "$JS.ACK.*.*.>"], subscribe: ["_INBOX.>"] } }
    # the application `refproj` the record attaches: publishes on its own
    # subjects alone, reads nothing of DNA's (#987)
    { user: app-refproj, password: $NATS_APP_REFPROJ_PASSWORD,
      permissions: { publish: ["*.app.refproj.>"], subscribe: ["_INBOX.>"] } }
    # the reflexes (#988): publish their firings on their own subjects,
    # read nothing
    { user: reflexes, password: $NATS_REFLEXES_PASSWORD,
      permissions: { publish: ["*.app.reflexes.>"], subscribe: ["_INBOX.>"] } }
    # the head: subscribes, publishes nothing
    { user: head, password: $NATS_HEAD_PASSWORD,
      permissions: { publish: { deny: [">"] }, subscribe: ["*.>"] } }
```

## Accounts

| user | held by | may |
|---|---|---|
| `owner` | `hale dna nerves migrate`, `hale dna dev` | create, update or delete the stream; no process that runs the organism holds it |
| `spine` | a node's host and the organization it starts | publish and read the organization's facts (`<org>.dna.>`), read the applications' events through the durable `heart`, tell the heads what landed (`<org>.head.>`) |
| `head` | a head (the face) | subscribe; publish nothing |
| `reflexes` | the reflexes | publish their firings on `<org>.app.reflexes.>` alone |
| `app-<name>` | the application the record attaches | publish on `<org>.app.<name>.>` alone; read nothing of DNA's |

Each password is the vault's, `nats-<org>-<role>` (the application's
is `nats-<org>-app-<name>`), drawn by `hale dna init` and drawn anew by
every `hale dna upgrade`, which restarts a server compose is running so
it reads them. A part that connects names its user and its vault
entry, and pond's client presents the password on the one line that
writes `CONNECT`: no URL, argument or environment variable carries it.
A role the vault holds no password for is refused; nothing connects
with a default.

The application's account is provisioned when the record attaches the
application and revoked by `hale dna application remove`
([The heart and the body](./heart.md)). An application named
`reflexes` holds no account: those subjects are the reflexes' alone.
[The skin](./skin.md) has every secret the organism holds.

## Migrate and drop

The stream is created with the owner's URL, and the verb prints what
every other part is handed:

```text
$ hale dna nerves migrate
HALE_DNA_NATS_ORG=dna_…
HALE_DNA_NATS_URL_SPINE=nats://…
HALE_DNA_NATS_URL_HEAD=nats://…
HALE_DNA_NATS_URL_REFLEXES=nats://…
HALE_DNA_NATS_URL_APP=nats://…
HALE_DNA_NATS_USER_APP=app-refproj
HALE_DNA_NATS_VAULT_APP=nats-dna_…-app-refproj
HALE_DNA_NATS_VAULT_REFLEXES=nats-dna_…-reflexes
```

`HALE_DNA_NATS_ORG` is the organization's token, the same name memory
gives the record's schema. Every URL is the server alone,
`nats://host:port`; the three `_APP` lines appear while the record
attaches an application. With no `HALE_DNA_NATS_URL_OWNER` the verb
brings up compose's `nerves` service and uses that. It refuses a
server without JetStream (`the server does not run JetStream (start
it with -js)`) and a vault that lacks a role's password, naming
`hale dna upgrade`. Run it again at any time: an existing stream is
brought to this toolchain's configuration.

`hale dna dev` does this first and hands the host the spine's URL and
the token alone. `hale dna run` creates nothing: give it
`HALE_DNA_NATS_URL_SPINE` and `HALE_DNA_NATS_ORG`, and it strips any
owner's or head's URL before the host sees them. Without them the host
says so and runs, and the organization hears nothing the record asks:

```text
hale dna run: no nerves: HALE_DNA_NATS_URL_SPINE and HALE_DNA_NATS_ORG are not set (`hale dna dev` creates the stream from HALE_DNA_NATS_URL_OWNER or dna/compose.yaml and hands the host the spine's); the organization hears nothing the record asks
```

An application the organism starts (an instance a fleet node starts,
the application `dev` runs) inherits the application's URL, user,
vault entry and the token, and none of the organism's other
credentials: the spine's, owner's, head's and reflexes' URLs and every
memory DSN are taken out of what it inherits.

`hale dna nerves drop` deletes the stream, and everything it held,
with the owner's URL; it prints `dropped DNA_…`, or `absent DNA_…` when
there was none. It never falls back to compose: with no
`HALE_DNA_NATS_URL_OWNER` it drops nothing and says so. Drop it beside
memory's schema when an organization is gone for good.

## Subjects and the stream

Everything an organization carries is under its token:

| subject | what travels on it |
|---|---|
| `<org>.dna.<family>.<event>` | DNA's own facts: the declared subject of a `dna::` topic, such as `dna.intent.offered` |
| `<org>.app.<app>.<event>` | an application's events, under its own name ([The heart and the body](./heart.md)) |
| `<org>.head.<event>` | what a node tells the heads (`head.row.landed`), outside `dna.` so no organization reads it |

The connection puts the token on what leaves and takes it off what
arrives, so a topic keeps its declared subject. One server can carry
several organizations this way, each under its own token.

Each organization has one stream, `DNA_<ID>` (its token upper-cased),
over `<org>.>`, on disk, keeping what it was told for a week. It is
not a queue that forgets once read: every publish is acknowledged by
the stream, and each reader keeps its own place in it through a
durable consumer:

- `spine`: the organization reads its facts, filtered to
  `<org>.dna.>`. A fact published while it was down or restarting
  reaches it when it is back.
- `heart`: the node's host reads every application's events,
  `<org>.app.>`, at most 64 unacknowledged at once. There is one for
  the organization, so each event goes to one host.

At start the host waits up to 20 seconds for the organization to be
reading, and says which:

```text
hale dna run: the organization reads its facts from the nerves (DNA_…)
hale dna run: the organization did not come to read the nerves within 20s; the facts wait in the stream
```

## Relaying after the row lands

These are the request rows a node relays, the topic each goes out on,
and the rows that answer it:

| request row | topic (subject) | answered by |
|---|---|---|
| `intent.requested` | `IntentOffered` (`dna.intent.offered`) | `intent.offered`, `intent.refused` |
| `review.verdict` | `ReviewVerdict` (`dna.review.verdict`) | `review.settled`, `review.refused`, `review.signed` |
| `concern.requested` | `ConcernRaised` (`dna.concern.raised`) | `concern.raised`, `concern.refused` |
| `pressure.requested` | `PressureRaised` (`dna.pressure.raised`) | `pressure.raised`, `pressure.refused` |
| `practice.requested` | `PracticeRequested` (`dna.practice.requested`) | `practice.proposed`, `practice.refused` |
| `hold.requested` | `HoldRequested` (`dna.hold.requested`) | `hold.proposed`, `hold.refused` |
| `position.requested` | `PositionOpenRequested` (`dna.position.requested`) | `position.proposed`, `position.refused` |
| `graph.requested` | `GraphReviewRequested` (`dna.graph.requested`) | `graph.proposed`, `graph.refused` |
| `schedule.requested` | `ScheduleRequested` (`dna.schedule.requested`) | `schedule.answered` |
| `knowledge.node.requested`, `.binding.`, `.edge.` | `KnowledgeNodeRequested`, `…BindingRequested`, `…EdgeRequested` | the matching `.proposed` or `.refused` |
| `observation.requested` | `ExpressionObserved` (`dna.expression.observed`) | `expression.observed`, `observation.refused` |
| `attempt.outcome_requested` | `WorkSubmit` (`dna.work.submit`) | `attempt.outcome`, `attempt.outcome_refused` |
| `attempt.allowance_requested` | `WorkAllowanceAsk` (`dna.work.allowance`) | `attempt.allowance_granted`, `attempt.allowance_refused` |

A request is answered when a later row of an answering kind names it.
Until then the node relays it again every 30 seconds, whatever the
transport said about the last publish: a publish is confirmed by its
answer in the record, never by the stream. A concern and a pressure
signal share an entity with every other from their source, so each
carries a `request` id and its answer names that id.

Under `dna.trust = signed` a node relays only rows whose commit
carries a signature git verifies. It answers an unverified one with a
refusal naming the commit (`unverified writer`) and never publishes it.
Over a shared record a node relays an ask only for a position its
owner holds; the rest stay in the record for the owner's node.

## Consumed once, by id

Delivery is at least once, so every reader consumes by id:

- The organization admits an intent once by its id: offered again, it
  answers with the execution already admitted and writes nothing. A
  verdict repeated at a settled Review is answered as already settled.
  A practice, a concern or a pressure request is answered once by its
  request id; a concern delivered twice is one concern, and the count
  that turns concerns into a proposal counts requests, never
  redeliveries.
- A leg's outcome and allowance ask are keyed by the attempt and the
  lease: one outcome under one lease.
- An application's event lands as a `reading.recorded` row,
  `<app>/<event>/<id>`, before anything acts on it. A second arrival
  of the same entity, a redelivery or a replay, is refused as a
  duplicate and lands nothing. The host acknowledges the message to
  the stream only once its row has landed, or it was refused for good
  (a duplicate, or not a reading at all); one whose row could not be
  written comes again. An event not landed within the stream's week
  ages out unrecorded.

## Delivery is audited

A connection keeps every publish until the stream acknowledges it and
writes it again after a reconnect. A publish still unacknowledged
past its window violates the connection's `delivery` closure. The
violation collapses to the connection's owner, which never restarts the
connection in place:

- the host appends `violation.recorded` (`adapter_undeliverable`,
  subject `nerves`, its holder and why), stops its organization and
  the expression, gives the body lease back (`body.released`,
  `why: nerves`) and exits 75, the organism's one restart code; its
  unit starts it again with a fresh connection;
- the organization's own connection collapsing makes it exit 75 too,
  and the host that supervises it ends with it.

The new node relays every request still unanswered, because the row,
not the publish, is the fact.

The heart's durable is audited the same way: a pull on `heart` refused
while the connection holds (the durable or the stream gone) is one
`violation.recorded` row (`pulse_stopped`) per outage, and the
connection makes the durable again and pulls on.

## Rows landing

For every row its view of the organism gains, a node publishes
`head.row.landed` (the row's `seq`, `kind` and `entity`), at most 256
on one tick. The face's head subscribes as the `head` user and tells
the browser to read again, so the face does not poll for the
organism's rows ([The head and the face](./head.md)).

## Shared records

Over a record several organizations share, every owner runs a body and
an organization, and each owner is a space of its own on the stream:
its host publishes under `<org>.<owner>.` and its organization reads
through the durable `spine_<owner>`, filtered to
`<org>.<owner>.dna.>`. No organization pulls a fact another owner's
host published. The host names its owner from the clone's `dna.owner`
and hands it to the organization as `HALE_DNA_OWNER`. The applications'
events stay on one `heart` durable for the whole organization.

## Stopping

SIGTERM drains a node: it stops its organization, which drains the
same way, gives the body lease back and ends. `hale dna run` and
`hale dna dev` give it 30 seconds for that (`LOTUS_DRAIN_GRACE_MS`, in
milliseconds, when the environment does not already set one); past
the grace the runtime ends it.
