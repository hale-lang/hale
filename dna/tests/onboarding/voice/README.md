# voice

[![ci](https://github.com/hale-lang/voice/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/hale-lang/voice/actions/workflows/ci.yml)

One inference endpoint in front of many model backends. A caller names a
model and presents a project key. Voice picks a seat that can serve it,
enforces the project's limits and budget, and records what the request used,
per project, key and paying account.

Voice is written in [Hale](https://github.com/hale-lang/hale). The design is
in [hale-lang/voice#1](https://github.com/hale-lang/voice/issues/1), and the
settled direction in [`DIRECTION.md`](./DIRECTION.md).

**Status:** the specification, and a stub of the api that answers every
endpoint with a canned happy-path response (below). The contract comes first.

## The stub api

[`api/`](./api/) is a Hale program that serves every endpoint in
`openapi.yaml` and answers each with its happy path: a canned body from
[`api/canned/`](./api/canned/), one file per `operationId`, all drawn from
one consistent example (a `work` project, `static` and `claude-work`
accounts, a `laptop` node). Inference endpoints answer with the event stream
when the request sets `"stream": true`. Nothing is stored and nothing is
checked; it is there to build clients and the UI against, and each endpoint
is replaced by its real handler as the MVP is built.

```sh
hale build api
api/api --port 8080          # --host, --canned DIR
curl localhost:8080/v1/models
hale test api                # every route answers its happy path
```

Every canned body validates against the spec. Friction met along the way is
in [`FRICTION.md`](./FRICTION.md).

## The node (skeleton)

[`node/`](./node/) is the start of the real node, not a stub. It reads
and checks its capabilities file at start (refusing a kind it cannot run),
serves its local API from `node.yaml` (status, capabilities, local
controls) and retries the coordinator. The protocol (assignments, seat
states, serving) and the static engine come next.

```sh
hale build node
node/node --id laptop --data node-data --capabilities deploy/node/capabilities.json --port 8081
hale test node
```

## Running the process model

[`compose.yaml`](./compose.yaml) runs voice as the MVP will: Postgres, NATS,
the api, the brain and a node, each its own process, from one image built
with hale's released toolchain ([`Dockerfile`](./Dockerfile)).

```sh
docker compose up --build
curl localhost:8080/v1/models      # the api
curl localhost:8081/node/v1/status # the node's local API
```

Today the api is the canned stub and the brain and node are skeletons, so
nothing reads Postgres or NATS yet, and the node registers with the stub's
canned answer.

## The brain (skeleton)

[`brain/`](./brain/) is its own program from the MVP on. The skeleton ticks
and names what each tick will do; the store, the advisory lock and the route
table come with the MVP. `hale build brain && brain/brain --database URL`.

## The admin UI

[`ui/`](./ui/) holds voice's own admin UI, one application for all of
voice, served by the api from its own origin. A first slice runs against
the stub: tokens, components and five screens. The backend is Hale; the UI is
not, because Hale's front-end story is not far enough along to build on. So
**the api is the trust boundary**: the compiler's claims about the system
graph stop at the api's edge, the UI is an untrusted client like any other,
and every rule is enforced by the api. The seam, the proposed stack and the
screens are in [`ui/README.md`](./ui/README.md). It builds against the stub,
and follows changes through the admin event stream (`GET /admin/v1/events`).

## The contract

| Document | What it specifies | Served by | Consumed by | Over |
|---|---|---|---|---|
| [`openapi.yaml`](./spec/openapi.yaml) | The coordinator: the caller plane (`/v1`), the admin plane (`/admin/v1`) and the node plane. | `api` | callers, `ui`, `node` | HTTP |
| [`node.yaml`](./spec/node.yaml) | A node's own admin API (`/node/v1`), where its seats are configured. | `node` | the machine's operator | HTTP |
| [`protocol.yaml`](./spec/protocol.yaml) | The connection between a node and the coordinator (AsyncAPI). | `node` | `api` | NATS |
| [`spec/vendor/open-responses/`](./spec/vendor/open-responses/) | The [Open Responses](https://www.openresponses.org/specification) standard, `2026-04-24`, which the caller plane is. | | [`openapi.yaml`](./spec/openapi.yaml), [`protocol.yaml`](./spec/protocol.yaml) | |
| [`spec/store.md`](./spec/store.md) | Postgres: the tables and the transactions, the state laws as SQL. | `postgres` | `api`, `brain` | SQL |
| [`spec/internal.yaml`](./spec/internal.yaml) | Voice's own NATS subjects: the route table from the brain. | `brain` | `api` | NATS |

The code conforms to these documents, not the other way round. The last
three columns are where the processes meet: the process that serves the
contract, the ones that consume it (a name in code is a process or a seed of
this repository, a link another contract, plain words a party outside it),
and what it travels over. A tool reads the table as it is written.

## The pieces

```
     callers                                operator, UI
        |                               +-------------------+
        | /v1                           | /admin/v1         | /node/v1
        v                               v                   |
  +----------------- api (x N), exposed ----------------+   |
  |  gateway -> admission -> claim -> serve             |   |
  |                                                     |   |
  |  settle, ledger        node plane: register, reports|   |
  +-----------------------------------------------------+   |
          |                     |           ^               |
          | SQL                 |           | HTTP, from    |
          v                     |           | the node      |
  +--------- Postgres --------+ |           |               v
  |  configuration            | | +---------+ node (x N) ---+-----+
  |  route table, counters    | | |  session -> seats -> engines  |
  |  ledger                   | | |  capabilities, local          |
  +---------------------------+ | +-------------------------------+
          ^                     |                 |
          | SQL                 |                 |
  +------- brain (x 1) -------+ |                 |
  |  reads state, writes      | |                 |
  |  the route table          | |                 |
  +---------------------------+ |                 |
          |                     |                 |
          v                     v                 ^
  ======== nats, private network ===================================
  serve, assign, cancel to each node on its own subject;
  each request's events and result back on its reply subject
```

- **The api** is the endpoint, and the only exposed process. Every
  instance serves the caller plane, the admin plane and the node plane,
  admits and routes requests, and settles them. Instances hold no state of
  their own beyond their open caller connections, so they scale out behind
  a load balancer and restart one at a
  time.
- **The brain** keeps the route table. It reads what the api instances write
  (seat states, heartbeats, quota windows, configuration) and decides
  which seats should take the next requests for each model. There is exactly
  one, it is not on the request path, and it talks to nothing but Postgres.
- **A node** runs on each machine that has engines, on the private network.
  It offers the engines its operator listed on the machine, runs the seats
  the coordinator assigns it within them, registers and reports over the
  api's node plane, and takes requests over NATS on its own subject. Any
  program that speaks [`protocol.yaml`](./spec/protocol.yaml) and the node plane
  can be a node; voice ships its own.
- **Postgres** is the shared state: configuration, the route table,
  counters and the ledger. Nodes have no database.
- **NATS** carries messages between processes on the private network:
  `serve` from an api instance to a node, each request's events and result
  back to the instance that asked, assignments and cancels to nodes, and the
  brain's notice that the route table changed. It carries no state.

What the specs call the **coordinator** is the api and the brain together;
callers and nodes cannot tell how many api instances stand behind the
address.

**The MVP runs the real process model:** one api instance, one brain, one
node, Postgres and NATS, as five processes. The brain shares state with the
api only through Postgres, and the process boundary enforces it. Scaling out
later adds api instances and nothing else: the paths between instances are
already exercised by the one. See [In Hale](#in-hale).

### Nouns

- **Model:** a catalog entry: an id callers name, its context, output
  ceiling and price. Configured on the coordinator.
- **Account:** what pays and what has limits: a subscription, a metered API
  key, or nothing for a local engine. Configured on the coordinator. Voice
  never holds the credential.
- **Node:** one machine running voice's node software. Created on the
  coordinator, which issues its enrollment token.
- **Engine:** what a node offers: a kind (`static`, `claude-cli`, ...) with
  the login it runs under. Listed by the node's operator in a capabilities
  file on the machine, with the node's hard limits, because only the
  machine holds the login. Two logins for one CLI are two engines.
- **Seat:** one of a node's engines, spending one account, serving some
  models. Configured on the coordinator and sent to the node as its
  assignment; the node refuses a seat outside what it offers. Several seats
  may spend one account and share its limits.
- **Project and key:** who is asking. Every request belongs to one project
  and one key, and keys name the accounts they may spend.

Nothing is discovered. A node offers exactly the engines listed on it and
runs exactly the seats assigned to it, and the coordinator admits only nodes
it issued a token to. A node's operator can pause it or a seat, or lower its
in-flight cap, on the machine; local controls only restrict.

**One admin UI.** The api serves the admin UI from its own origin, and every
node is administered through it: seats, caps and state live on the
coordinator. A node's own API is only for the machine it runs on, and for
when it cannot reach the coordinator.

## A request, end to end

1. **Admit.** The api reads the key and finds its project. It checks that
   the model is visible to the key, the data class is one the project may
   send, and the named account, if any, is one the key may spend.
2. **Reserve,** in one transaction. The api reserves the estimated input
   plus the requested maximum output against every limit that applies
   (project, key, model and account) and against the project's budget, at
   the model's current price, and records the request as in flight on this
   instance. If a limit does not fit, nothing is held and the answer is
   `429`. The transactions are in [`spec/store.md`](./spec/store.md).
3. **Pick a seat** from the route table in memory: the best-ranked seat
   for the model among the permitted accounts, with room by the api's own
   count. If the model's accounts are all exhausted, the answer is
   `503 no_capacity` with the earliest reset. If every seat is busy, the api
   waits for the next snapshot, until the request's deadline.
4. **Serve.** The api publishes `serve` on the node's subject as a request,
   naming a reply subject for this one, and the node answers `accepted` or
   `refused` at once. `refused`, or no answer, means the next candidate;
   the node, not the table, is the truth about its own capacity. The body
   is the standard Open Responses request; a Chat Completions request is
   converted first. The node's engine runs, and its standard streaming
   events come back on the reply subject, to the instance that asked, and
   are relayed as they arrive, as server-sent events or collected into one
   response. See [Events](#events).
5. **Retry.** If the seat fails before anything reached the caller, the api
   takes the next candidate, on a different seat and within the named
   account if there is one. The move is recorded as an attempt. After
   output has begun, a failure fails the response.
6. **Settle,** in one transaction. The node's `result` carries the model
   that actually ran, cache writes, timing and, from the CLI engines, the
   account's quota windows. The api settles the reservation on actual
   usage and writes the usage record. The response's `voice` object reports
   what served it and what it cost.

The coordinator never redirects and never stores prompts or responses. Content
passes through one api instance and one node in memory; what is kept is the
numbers and the caller's `metadata`.

## The route table

The brain turns the state of the fleet into a ranked table the api routes
from. It is a snapshot on NATS, not rows in Postgres.

- **Contents.** Per model, the seats that can serve it (up, on a node that
  is up, with an account that has headroom), each with its free capacity
  and a rank: accounts with the most headroom first, then the least loaded
  seats. Per model, the accounts that are exhausted and until when.
- **Published** by the brain on `voice.route` whenever it changes and every
  second regardless, with a version. Api instances keep the latest in
  memory; a new instance has a table within a second of starting.
- **Not exact, and it need not be.** Two instances can pick the same last
  slot; the node accepts one and refuses the other, which moves on. An
  instance decrements its own copy when it sends, so it does not pick a
  seat twice before the next snapshot, and ranks carry a little jitter so
  instances do not all chase the same seat.
- **Leases.** Every api instance heartbeats a row. When one stops, the
  brain releases the reservations of its in-flight requests and records
  them as abandoned.

The brain being down does not stop serving: the last table in memory still
routes, bounded by each node's own refusals; it only goes stale.

## Events

Voice is evented through Hale's bus, with NATS carrying the topics that
cross processes. Everything that moves between the pieces of a request is a
typed topic:

- `serve`, `assign` and `cancel`, keyed by node: from an api instance to
  the node, on the node's own subject.
- the node's events and its `result`, keyed by request: from the node to
  the api instance that asked, on the reply subject that request named.

Each subscriber names its own key (`where key == ...`), so nothing filters
traffic in a handler, and no api instance holds a node: whichever instance
routes a request talks to the node directly, and the answer comes straight
back to it. Nothing is forwarded between instances.

**These topics are bound to NATS from the MVP on**, in `main`'s
`bindings { }` block through pond's NATS adapter
([`pond/realtime/nats`](https://github.com/hale-lang/pond/tree/main/realtime/nats)),
in the api and in the node alike. With one api instance the same path is
simply exercised by one process. The brain's route table rides the same
connection (`spec/internal.yaml`). See Hale's
[across binaries](https://hale-lang.org/docs/services/multi-binary).

**What does not go on the bus: state.** The reservation and the settlement
are transactions, and a broker cannot make them atomic, so they stay in
Postgres. The route table is the one thing published that looks like
state, and it is a derived snapshot: losing it costs a second. Usage is written once, to the ledger; publishing it to a
broker as well would be a second write that can disagree with the first. A
consumer that wants usage as it happens reads the ledger by cursor.

**Why the broker is in the MVP:** a single api instance would not need it,
but the MVP's job is to lay the process model the rest is built on, and a
path first exercised when the second instance appears is a path that has
never run. NATS is core NATS, not JetStream: nothing durable travels on it,
so nothing needs to be retained.

## State

State lives in one of three places, and each piece has exactly one home.

### Postgres

- **Configuration.** The catalog, accounts and their limits, projects,
  budgets and limits, keys (only a digest of the secret), nodes (only a
  digest of the enrollment token), and pending policy changes. Every admin
  write is a transaction and appends a row to a change log (who, when, what
  was there before), so configuration has a history
  (`GET /admin/v1/changes`).
- **Counters and reservations.** Rate-limit windows, each project's spend
  in its current period, and every request in flight with what it holds.
  Every api instance updates them with conditional writes, so a limit holds
  across instances and across restarts. The route table is not here; it is
  a snapshot on NATS.
- **The fleet as reported.** Api instances' heartbeats, each node's
  registration, heartbeats, declared capabilities and local controls, its
  seats' states, and each account's quota windows as an engine last
  reported them.
- **The ledger.** One usage record per request: project, key, requested and
  served model, node, seat, account, tokens, price, timing, attempts and the
  caller's `metadata`. It is append-only. Usage summaries are queries over
  it, and in a DNA deployment it is the record DNA accounts from.

### Process memory

An api instance holds its open caller connections and its NATS
subscriptions, and may cache configuration, keyed by a version it checks.
Nothing in memory is the only copy of anything, so losing a process loses
only its own in-flight requests.

### A node's data directory

The node's capabilities file, written by its operator, and its local
controls. Everything else about what it runs comes from the coordinator, so
it needs no database.

### Migrations

The schema is versioned with
[`pond/migrations`](https://github.com/hale-lang/pond/tree/main/migrations):
migrations registered in Hale code, each applied in one transaction under a
Postgres advisory lock, with golang-migrate's dirty-state repair. They run
as a one-shot `migrate` step that the api and the brain wait on, never from
the api or the brain themselves, so neither ever starts against a schema it
does not expect. The schema and its migrations live in the store seed the
api and the brain share.

### Why Postgres, and not also Redis

The ledger grows without end and is read by aggregate queries (usage grouped
by project, key, account, day). Configuration writes need transactions. Api
instances need shared counters and an atomic claim. DNA reads the ledger.
Postgres does all of it, including the claim, so a second store would only
add something to run and keep consistent; there is also no Redis client in
pond. Hale reaches Postgres through
[`pond/pq`](https://github.com/hale-lang/pond/tree/main/pq), with schema
changes through
[`pond/migrations`](https://github.com/hale-lang/pond/tree/main/migrations).

### Metering is not optional

A request reserves in Postgres before it runs and writes its usage in the
same transaction that settles it. If Postgres is unreachable, nothing is
served (`503 metering_unavailable`): a budget is a circuit breaker, and an
unmetered request would go around it. There is no batch to lose; a request
that dies with its api instance is recorded as abandoned by the brain.

## Security

- **Callers** present project keys. The coordinator stores digests, never
  secrets.
- **Nodes** present their enrollment token on every node-plane call, and
  connect to NATS with their own credentials, confined to their own subject
  and the reply subjects they are handed. NATS and the nodes are on the
  private network; only the api is exposed. Revoking a node refuses its
  token, and the coordinator publishes nothing further to its subject.
- **Admins** authenticate through middleware in front of the admin APIs:
  OIDC later, nothing in the MVP, which listens on loopback only.
- **Provider credentials** stay with the engines on their nodes. A CLI
  engine's login is named in the node's capabilities file and never
  declared; voice never reads it.
- **The coordinator cannot widen a node.** It assigns seats only within the
  engines and limits the node declared, and the node refuses anything else,
  so a compromised coordinator cannot make a machine run what its operator
  did not offer.
- **Content** is never written down. A seat sees only the data classes its
  operator allowed it.
- **TLS** is terminated in front of the api. Hale's TLS reads block their
  thread and cannot park on an async pool yet, so the api speaks plain HTTP
  behind the terminator, which is also the load balancer. NATS is not
  exposed, so it needs no terminator.

## When things fail

| What fails | What happens |
|---|---|
| A seat | The request moves to another seat if nothing reached the caller; otherwise the response fails, with the attempt recorded. |
| A node | Missed heartbeats mark it down: its in-flight requests fail over as above and its seats leave the route table. When it is back it registers again and gets its assignment. |
| An account's quota | The account gets no slots until its window resets. Requests that named it fail with `503 no_capacity` and `Retry-After`. |
| An api instance | Its callers' requests fail; nodes are unaffected, since none is bound to it. The brain releases its reservations when its heartbeat stops. |
| The brain | Serving continues on the last route table in memory, which goes stale until the brain is back; a new api instance has no table until then. |
| Postgres | Nothing is served and nothing is configured until it is back. |

## In Hale

Voice is three Hale programs, the api, the brain and the node, from the MVP
on, plus shared seeds for the protocol's message types and for the store
(schema and queries, used by both the api and the brain). The shapes come
from Hale's
[styleguide](https://github.com/hale-lang/hale/blob/main/spec/styleguide.md).

**Api**

- The gateway serves both planes on `std::http`. Each caller request is an
  accepted child with `release`, so its memory is reclaimed when the request
  ends. A streaming request takes over its connection to write server-sent
  events itself.
- The node plane is ordinary handlers: register, assignment, heartbeat and
  the reports, each a write to the store.
- A request child publishes `serve` on the node's topic and subscribes to
  the events and result topics with its own request id as the key; the
  binding to NATS makes the reply subject that request's own.
- The store owns the Postgres connections: a pool for reads and a dedicated
  connection for transactions, since `pq`'s pool does not do transactions.
  Because `pq` blocks, the store is pinned, away from the pool that serves
  requests.

**Brain**

- Its own program, with its own store and Postgres connection. It takes a
  Postgres advisory lock before doing anything, so only one brain works at a
  time; a second brain process waits on the lock as a standby and takes over
  when the first one's connection drops.
- It shares nothing with the api except Postgres and the route table it
  publishes. The process boundary enforces that.
- Each tick it reads the fleet's reported state (seat states, heartbeats,
  quota windows, configuration), computes the route table and publishes it
  if it changed (and every second regardless), and reclaims what dead api
  instances held. Per-tick work runs in a method, so each tick's memory is
  reclaimed.

**Which topics bind, and which use the client.** A `keyed_by` topic over
a binding is one subject that every subscriber receives and filters, so
`bindings { }` carries only what is truly a broadcast: the route table on
`voice.route`. The per-node and per-request subjects (`serve`, `assign`,
`cancel` to a node; a request's events and result back) are published and
subscribed through the NATS client directly, with the subjects
`spec/protocol.yaml` names, so a node receives only its own requests and an
instance only its own answers. That is what the per-node NATS permissions
rely on.

Running a second api instance is starting one; the subscriptions are
already there.

**Node**

- The session registers over the node plane, then holds the NATS
  connection (`pond/realtime/nats`, which reconnects and resubscribes on
  its own) and posts heartbeats. It is pinned, because the client's reads
  block.
- The capabilities file is read at start and declared at registration; the
  assignment that comes back, and every `assign` after, is reconciled
  against it, and seat states are posted back.
- Each seat is a locus running its engine: the static engine answers in
  process, and CLI engines run their CLI as a supervised child process.
- The admin API serves `/node/v1` on loopback: status, capabilities, local
  controls.

## Deployment

- **MVP:** one machine, five processes on loopback: Postgres, NATS, one
  api, one brain and one node. The node offers the static engine, and a seat on
  it answers every request with its configured text. Everything is set up
  through the APIs, in this order: a model, an account, a node (which yields
  a token), the node started with that token and a capabilities file, its
  seat, a project and a key.
- **Personal:** the api, the brain, Postgres and NATS on a machine that
  stays up, with a node on each machine whose CLIs are logged in, one engine
  per login and a seat on each.
- **Scaled out:** several api instances behind a load balancer that also
  terminates TLS and sends traffic only to instances whose `/readyz` is ok,
  the brain with a standby on another machine, and nodes anywhere.

| Deployment | Runs |
|---|---|
| MVP ([`compose.yaml`](./compose.yaml)) | `postgres`, `nats`, `api`, `brain`, `node` |
| Personal | `api`, `brain`, `postgres` and `nats` on one machine; a `node` on each machine with logins |
| Scaled out | `api` behind a load balancer, `brain` with a standby, `postgres`, `nats`, and `node` anywhere |
