# voice

[![ci](https://github.com/hale-lang/voice/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/hale-lang/voice/actions/workflows/ci.yml)

One inference endpoint in front of many model backends. A caller names a
model and presents a project key. Voice picks a seat that can serve it,
enforces the project's limits and budget, and records what the request used,
per project, key and paying account.

Voice is written in [Hale](https://github.com/hale-lang/hale). The design is
in [hale-lang/voice#1](https://github.com/hale-lang/voice/issues/1), and the
settled direction in [`DIRECTION.md`](./DIRECTION.md).

**Status:** the specification, and the api and the node on their way
from a stub to the real thing: configuration (the catalog, accounts, nodes
and seats, projects, budgets, limits and keys, with the change log), the
caller's view of it, the node plane, the brain and its route table, and
inference served by a node's static seat over NATS, streamed or not, are
real; usage and the admin event stream still answer a canned happy path
(below). The contract comes first.

## The api

[`api/`](./api/) is a Hale program that serves every endpoint in
`openapi.yaml`. The ones made real are rows of an `api` surface
([`api/surface.hl`](./api/surface.hl)), each a handler on a receiver over
the configuration store; a thin router matches method and path, decodes the
body into the row's request and writes the contract's JSON
([`api/endpoints.hl`](./api/endpoints.hl), [`api/wire.hl`](./api/wire.hl)).

| Served for real | Endpoints |
|---|---|
| health | `GET /healthz`, `GET /readyz` |
| the caller plane | `GET /v1/models`, `GET /v1/models/{model}`, `GET /v1/capabilities` (a project key required) |
| inference | `POST /v1/responses`, `POST /v1/chat/completions`, served by a node's seat over NATS, streamed or not (as SSE with `"stream": true`) |
| the catalog | `GET`, `POST /admin/v1/models`; `GET`, `PATCH`, `DELETE /admin/v1/models/{model}` |
| projects, budgets, limits | `/admin/v1/projects`, `.../{project}`, `.../{project}/budget`, `.../{project}/limits` |
| keys | `GET`, `POST .../keys`; `GET`, `PATCH`, `DELETE .../keys/{key}` |
| accounts | `GET`, `POST /admin/v1/accounts`; `GET`, `PATCH`, `DELETE /admin/v1/accounts/{account}` |
| nodes and seats | `/admin/v1/nodes`, `.../{node}`, `.../{node}/token`, `.../{node}/seats`, `.../{node}/seats/{seat}` |
| the change log | `GET /admin/v1/changes`, written by every admin write above |
| the node plane | `/node/v1/register`, `assignment`, `heartbeat`, `seats`, `capabilities`, `local`, `quota`, `draining` (an enrollment token required) |

Everything above reaches the configuration and the fleet's reports through
the `Store` interface ([`store/store.hl`](./store/store.hl)). Given
`--database postgres://...` (or `VOICE_DATABASE_URL`), the api keeps them
in Postgres (`PgStore`, [`store/pg_store.hl`](./store/pg_store.hl)), in the
tables of [`spec/store.md`](./spec/store.md), each admin write and the
change it records in one transaction; `api migrate` brings the schema up to
date first ([`store/migrations/`](./store/migrations/), store.md's SQL in
order). Several api instances may share one database: a write that reads
a resource first holds it (a Postgres advisory lock on its path, in the
write's transaction) from the read to the commit, and a node's assignment
version moves on in SQL, so no instance's write is lost or reuses a
version (`scripts/race.sh` checks it with two instances). A store failure
answers a fixed message with an id; what Postgres said goes to the api's
stderr only, and nothing of the database URL is printed but its host and
port. Grafted into a DNA habitat, voice's genome needs a `database`
(the root [`hale.toml`](./hale.toml), `[dna] needs`), and when neither
`--database` nor `VOICE_DATABASE_URL` is given the api and the brain use
the one the habitat provisioned, `HALE_DNA_DATABASE_DSN`: a DSN with no
password, whose `vault=` names the vault entry pq reads it from when it
authenticates. Its own NATS, grafted, is the habitat's `broker`
([`lib/broker`](./lib/broker/broker.hl)): `HALE_DNA_BROKER_URL`, `_USER`
and `_VAULT` replace `--nats` and name its account, and every subject
moves under the prefix `HALE_DNA_BROKER_SUBJECTS` grants (`voice.node.*`
becomes `heart.<id>.node.*`, and the same for `reply` and `route`), the
request inboxes under `HALE_DNA_BROKER_INBOX` when it is handed over and
under the prefix when not; standalone, the subjects are `voice.*` as
always. The nodes have their own account there
(`HALE_DNA_BROKER_NODE_USER`, its password in the vault entry
`HALE_DNA_BROKER_NODE_VAULT`), which a node is handed in its
registration's answer over its enrollment-authenticated call: the api
writes that answer itself on the node's connection, the password revealed
into that write alone, escaped as a JSON string there, and the answer
marked `no-store`. That account is one per heart, shared by every node today: a node
holding it can read every node's requests and answer for another node
(its subjects are `heart.<id>.node.>` and the replies). Per-node users,
each held to its own subjects, come later. A hand-over is whole or
refused: any of the broker's variables, or of the head's, set without the
rest stops the api (and the brain) at start, rather than running it
standalone; so does a vault without the heart's own broker password. `HALE_DNA_HEAD_URL`, `HALE_DNA_APPLICATION_ID`,
`HALE_DNA_OIDC_ISSUER`, `HALE_DNA_OIDC_CLIENT` and
`HALE_DNA_OIDC_CLIENT_VAULT` replace `--head`, `--application`,
`--issuer`, `--client` and `--client-vault` when they are set, and the
habitat migrates the database itself (`[dna.migrations.database]` in
`hale.toml`: `api migrate`, run at every graft, `hale habitat migrate` and
upgrade, its own ledger making a run at the head a no-op that exits 0):
neither the api nor the brain migrates on its own. `E2E_BROKER=grafted scripts/e2e.sh` runs the
whole e2e on a NATS whose one account may use `heart.zz.>` alone. Without a database the store is in the process
(`MemoryStore`), lost on restart, which is what `hale test api` runs on. Budgets are
configuration: nothing is charged against them yet.

**Rate limits** are enforced on one admission path, their limits from one
of two sources, never both ([`api/admission.hl`](./api/admission.hl)).
Without `--head` they are the limits the admin plane stored: the
project's, its model-scope ones for the model asked for, and the key's.
With `--head`, voice runs under a DNA organization, and a project's limit
is its ratified practice `operating/limits/<project>`, which its Board
decides ([`api/governance.hl`](./api/governance.hl)). A reader of its own,
a pinned locus off the request path, reads the ratified practices of the
application `--application` names (its id at the head, which the head's
`/api/hale/v1/applications` lists) through the head as a service (a
`client_credentials` token from `--issuer` under `--client`, its secret
the vault's `oidc-service-<client>` or `--client-vault`), every
`--limits-every-s` seconds (or `VOICE_LIMITS_EVERY_S`, which the flag
overrides; 30 by default), each call one attempt under a 30 s deadline
([`api/deadline.hl`](./api/deadline.hl)). A request reads only the limits
last read, never the head. An answer from the head must give its length
or its chunks, and one cut short does not land, so no project goes
unlimited because a page was cut. Of a project's ratified practices, the
one in force is the one no other supersedes; a project whose practices are
ambiguous (two in force, or a cycle of supersessions) keeps the limit held
for it, or, with none, its keyed requests are `503`, and the other
projects are read as they are. `GET .../limits` then shows the
ratified limit with `source: practice` and the practice it came from,
`PUT .../limits` answers `409 conflict`, and until a read has landed a
keyed request is `503 metering_unavailable`; after that, the last limits
read stand while a read does not land, and the reader logs why.

A limit's `requests` per `window_seconds` are counted, each limit's
counter kept under its scope, its window and its kind, and a request past
any is `429 rate_limited` with `Retry-After` and the limit in
`voice.limit`; an admitted request carries the `x-ratelimit-*-requests`
headers of the limit with the fewest left. The counters are each api
instance's own. Token limits are shown, not yet enforced: they wait on
the reservation in [`spec/store.md`](./spec/store.md). `concurrent` is
shown and not enforced: an instance serves one request at a time (its
HTTP server is serial), so a limit on requests in flight could never
refuse one.

```sh
api/api --head http://127.0.0.1:8792 --application <id> --issuer http://127.0.0.1:8794 --client voice-api
```

[`api/tests/admin_limits.sh`](./api/tests/admin_limits.sh) runs it end to
end against a DNA organization, from the Board's ratification to the
caller's 429, a superseding limit, and a head stopped with the limits
read last standing and no answer waiting on it (`VOICE_DNA_FIXTURE=1`,
with a hale whose record is in memory, v0.26.0 on).

A node is `pending` until it registers, then `up` while it heartbeats and
`down` after three missed intervals; each seat shows the state its node
reported, and the problems the coordinator sees (an engine the node does
not offer, a config its engine requires, an archived account, a model not
in the catalog). A change to a node or its seats is a new assignment,
published to the node on NATS. An inference request is admitted (its key,
model, account and data class), then offered to each seat that could take
it in turn: a seat that refuses, does not answer, or fails before output is
an attempt in `voice.attempts`, and the next is offered the request. Each
attempt answers on a reply subject of its own, and a seat the api stops
waiting for is sent `cancel`. When every seat fails, the `502` carries the
request id and the attempts in `error.voice`. A seat whose node holds a
local cap of 0 is not offered. Chat Completions messages are converted to
the standard's input items (content parts, tool calls and tool results),
and what has no standard form is refused `400 unsupported`. The api serves
its callers one at a time, and opens a NATS connection for each attempt.
With `"stream": true` the answer is Server-Sent Events over a chunked
body (hale's `std::http::SseWriter`): `/v1/responses` sends the standard
event stream, a `voice:attempt` event before the next seat's first when a
seat fails before output, the terminal event with `voice`, then `data:
[DONE]`; `/v1/chat/completions` sends `chat.completion.chunk`s, the last
with `usage` and `voice`, then `data: [DONE]`. The stream opens at the
first event a seat sends, or at the first keep-alive (below), so a request
every seat refuses is still answered whole. A seat that fails once the
stream has begun ends it with the evidence: an `error` event and
`response.failed` carrying the response so far with `voice`, or a last
chunk finished `error` with an `error` object and `voice`; a truncated
answer never ends as a complete one. A seat is waited for between messages,
not for the whole answer: 120 s until its first output (`--seat-first-s`; a
slow model, or an engine that answers at the end), then 30 s between events
(`--seat-idle-s`). While a seat that accepted is waited for, the stream gets
an SSE comment (`: keep-alive`) whenever 15 s pass with nothing sent, so a
proxy does not cut it. A caller that goes away is found by the next write
that fails, an event's or a keep-alive's. The first write after a caller
closes is accepted (the caller answers it with a reset), so each keep-alive
is followed a second later by an empty comment (`:`), the write that fails:
a caller that leaves while the seat is quiet is found within about 16 s, and
the stream ends there and the seat is sent `cancel`. A request body is at
most 12 MiB; a larger one is answered `413 request_too_large` in the
contract's envelope before it is read, and a chunked one (hale's Server
reads `Content-Length` bodies only) is refused `400`.

Usage and the admin event stream answer their happy path:
a canned body from [`api/canned/`](./api/canned/), one file per
`operationId`. Each is replaced by a row as it is made real.

```sh
hale build api
api/api --port 8080          # --host, --canned DIR
curl -X POST localhost:8080/admin/v1/models -H 'content-type: application/json' -d '{"id": "echo-1"}'
curl -X POST localhost:8080/admin/v1/projects -H 'content-type: application/json' -d '{"id": "work", "name": "Work"}'
curl -X POST localhost:8080/admin/v1/projects/work/keys -H 'content-type: application/json' -d '{"name": "laptop"}'   # its "secret"
curl localhost:8080/v1/models -H 'Authorization: Bearer vk-...'
curl localhost:8080/admin/v1/changes
hale test api
```

Every canned body, and a live call to every endpoint, validates against the
spec (`spec/validate`).

### The surface's bundle

[`api/bundle/`](./api/bundle/) is `hale api export --surface Voice` of the
api, committed: the surface's description, its OpenAPI, JSON Schema, MCP
and protobuf forms, and its contract digest, and nothing else. CI exports
it again and fails on any drift (`hale api export --surface Voice --check
api/bundle api`), and on any file in `api/bundle/` the export does not
write.
After a change to a row, export it again and commit the result.

The bundle is the surface's own shape, not the REST wire's: a row is
`POST /call/<member>`, and its types say what the codec of hale 0.25.0 can
carry. [`api/gaps.json`](./api/gaps.json) lists every place
where a row's type differs from the contract schema it stands for, each
with what closes it, and `spec/validate/bundle.mjs` (`npm run bundle`)
fails when the gaps found and the gaps committed differ either way. It
fails closed: every exported component is mapped to the contract, reached
inside a mapped one, or listed as not compared with why (path and query
records, hale's own refusal, the node's whole-body report), and holds no
field its reason does not name; a scalar of another type, a narrowing (an
enum on a plain string, an integer where the contract says number), a
field a request row requires and the contract does not, and enum values
that differ are never classed away (a field an answer always sends is
what the contract allows; a row's optional field, a presence pair, may
be left out or `null` where the contract's may be left out); and
`spec/validate/negative/` (`npm run bundle-negative`) holds changes it must
refuse:

| Kind | Closed by |
|---|---|
| `tri-state` | hale: a PATCH field that may be left out (kept), `null` (cleared) or a value; a presence pair (`has_x` beside `x`) says two of the three, so the row keeps `set_<field>` |
| `map` | hale: a free-form JSON object (a seat's `config`, a change's `before` and `after`) and a map whose values are scalars (`Health.checks`); a hale map's values are records |
| `json-text` | hale: a free-form JSON value inside a typed object (an engine's `config_schema`), so a node's `capabilities` ride as JSON text |
| `path` | voice: a path or query parameter carried in the row's request |
| `required` | voice: a row field with a default is not required on the surface |
| `wire` | voice: the REST wire's shape differs from the row (`api/wire.hl` writes it) |
| `open` | voice: the contract admits several shapes there (`oneOf`/`anyOf`); the row carries one, as the reader does |

After a change, `npm run bundle -- --write` writes the gaps again; the diff
is what to review. Friction met along the way is in
[`FRICTION.md`](./FRICTION.md).

## The node

[`node/`](./node/) is voice's own node. It reads and checks its
capabilities file at start (refusing a kind it cannot run) and serves its
local API from `node.yaml` (status, capabilities, local controls). It
registers on the api's node plane with its enrollment token, applies the
assignment that comes back (refusing a seat outside what it offers, and
holding one its local controls pause), reports every seat's state and
heartbeats with the assignment version it applied. On NATS it takes
`assign`, and answers `serve` at once with `accepted` or `refused`, then
serves the request on its seat with the static engine, streaming the
standard's events and the result to the reply subject the request named
(`protocol.yaml`); a `cancel` stops it. It serves one request at a time,
and refuses a `serve` that arrives meanwhile, so the request goes to
another seat. Core NATS delivers an `assign` at most once, so a node reads
its assignment each time it is on NATS again, and the api sends the
assignment again to a node whose heartbeat says it is behind.

The node's calls to the api's node plane run on a thread of their own
(`Plane`), apart from the NATS session (`Session`): the api serves one
caller at a time, so a heartbeat can wait for as long as a request holds
the api, and a session waiting there could not answer the `serve` that
fails that request over to it. The static engine's `fail_after_ms` makes a
seat fail every request after that long, for drills.

```sh
hale build node
node/node --id laptop --api http://127.0.0.1:8080 --token-vault node-laptop-enrollment --data node-data --capabilities deploy/node/capabilities.json --port 8081
hale test node
```

The enrollment token comes from the vault entry `--token-vault` (or
`VOICE_ENROLLMENT_TOKEN_VAULT`) names, read through `std::secret` and
revealed only into the request it authorizes, which is how a grafted node
is handed it. `--token vn-...` and `VOICE_ENROLLMENT_TOKEN` still work
standalone, and the node warns that the token is in the open. A database
URL with its password in it warns the same way; `?vault=ENTRY` in its
place names the password's vault entry.

[`scripts/e2e.sh`](./scripts/e2e.sh) runs the whole path on one machine:
NATS (in a container of its own, or the one `NATS_URL` names), the api and
a node, the node created and its seat configured through the admin plane,
a response and a chat completion served and validated against the spec,
a local pause that takes the seat out of service, and a seat change that
reaches the node over NATS. CI runs it (`end to end`).

## Running the process model

[`compose.yaml`](./compose.yaml) runs voice as the MVP will: Postgres, NATS,
the api, the brain and a node, each its own process, from one image built
with hale's released toolchain ([`Dockerfile`](./Dockerfile)).

```sh
docker compose up --build
curl localhost:8080/healthz        # the api; see "The api" for a key
curl localhost:8081/node/v1/status # the node's local API
```

The node `local` serves once it exists on the api and is started with its
token: create it (`POST /admin/v1/nodes` with `{"id": "local"}`), give it a
seat, and restart it with `VOICE_ENROLLMENT_TOKEN=vn-... docker compose up
-d node`. The `migrate` service brings the schema up to date before the
api and the brain start, and the api keeps its configuration in Postgres,
so it survives a restart of the api. The brain writes the route table to
Postgres every second and tells NATS its new version.

## The brain

[`brain/`](./brain/) is its own program, and only one works at a time: it
holds a Postgres advisory lock, and a second brain stands by, trying the
lock every second, and takes over when the first one's session ends: its
connection drops, its host stops answering TCP keepalives (about 11 s), or
it says nothing for 10 s, as a stopped process does (`idle_session_timeout`,
so voice needs Postgres 14 or newer; the brain says so and stops on an
older one). The brain's NATS notice runs on a thread of its own, so an
address that never answers costs the tick nothing.
Each second the one that works:

- reclaims what dead api instances held (an instance whose heartbeat is
  three intervals old, checked again inside its transaction): their
  requests in flight are settled as `abandoned`, their reservations
  released, and the instance is marked reclaimed (spec/store.md,
  "Reclaim"); an instance whose reclaim fails is left for the next tick;
- in one transaction, writes each node's `up` or `down` from its heartbeat
  and the route table (`route_table`, spec/store.md, "The route table"),
  its version from Postgres's clock and never below the one before it;
- publishes that version on `voice.route`
  ([`internal.yaml`](./spec/internal.yaml)), saying once if it cannot.

A tick at the bounds (256 seats each serving 16 of 256 models) computes the
table in milliseconds. `GET /admin/v1/routes` reads the table from Postgres. The api does
not route from the table yet: it picks a seat from the fleet's state in the
store, the same state the brain reads, until it serves requests
concurrently. Each api instance heartbeats its row in `api_instance` when
it has a database.

The store, its rows and its migrations are the seed
[`store/`](./store/), which the api and the brain both import.

```sh
hale build brain
brain/brain --database postgres://voice:voice@127.0.0.1:5432/voice --nats nats://127.0.0.1:4222
hale test brain     # VOICE_TEST_DATABASE_URL=... for the Postgres half
```

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
| [`spec/internal.yaml`](./spec/internal.yaml) | Voice's own NATS subjects: the route table's version from the brain. | `brain` | `api` | NATS |

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
from. It is one row in Postgres, rewritten every second; NATS carries only
the notice of its new version.

- **Contents.** Per model, the seats that can serve it (up, on a node that
  is up, with an account that has headroom), each with its free capacity
  and a rank: accounts with the most headroom first, then the least loaded
  seats. Per model, the accounts that are exhausted and until when.
- **Written** by the brain to Postgres every second, with a version that
  only grows, and its version **published** on `voice.route`. Api
  instances read the table when a newer version is announced and keep it
  in memory; a new instance reads it when it starts.
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
simply exercised by one process. The brain's notice of a new route table
rides the same connection (`spec/internal.yaml`). See Hale's
[across binaries](https://hale-lang.org/docs/services/multi-binary).

**What does not go on the bus: state.** The reservation and the settlement
are transactions, and a broker cannot make them atomic, so they stay in
Postgres. The route table is state too, so it is a row in Postgres; what
is published is only its version, and losing a notice costs a second. Usage is written once, to the ledger; publishing it to a
broker as well would be a second write that can disagree with the first. A
consumer that wants usage as it happens reads the ledger by cursor.

**The organization's events are not usage's second write either.** Run
attached to an organization (Hale's DNA), the api tells it each request a
seat served or began, streamed or not, as an event: a `UsageEvent` on
`app.voice.usage.recorded` ([`api/events.hl`](./api/events.hl)), its `id`
the request's, its `status` how it ended (`served`, `incomplete`,
`cancelled` when the caller left, `failed` when the seat failed after
output; a token count the seat did not report is left out). It goes out on the organization's own nerves, a
JetStream stream, not voice's NATS: the attach hands the api its server,
its user, the vault entry of that user's password and the organization's
token (`HALE_DNA_NATS_URL_APP`, `HALE_DNA_NATS_USER_APP`,
`HALE_DNA_NATS_VAULT_APP`, `HALE_DNA_NATS_ORG`), and the events are told
from the teller ([`api/teller.hl`](./api/teller.hl)), a pinned locus of
its own: a request hands its event over and goes on, so no answer waits
on the nerves. The teller queues at most 1024 events, keeps one
connection, publishes each under its id and waits for the stream's
acknowledgement, each step bounded at 2 s; a publish that does not land
is tried again after a back-off (to 5 s), and when the queue is full the
oldest event is dropped and counted, the running total logged at most
once a second while drops go on and once more after the last. The
organization records each event once, as a reading under its id, and acts on nothing it says. Without `HALE_DNA_NATS_ORG`
nothing is published and no connection is made.

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
  across instances and across restarts.
- **The route table.** One row the brain rewrites every second.
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
as a one-shot `migrate` step that the api and the brain wait on
(two at once are safe: `api migrate` holds an advisory lock of its own
around pond's, which creates its tracking table before it locks), never from
the api or the brain themselves, so neither ever starts against a schema it
does not expect. Today the store and its migrations live in the api
(`api migrate`); they move to a store seed the api and the brain share when
the brain reads the store.

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
| The brain | Serving continues on the last route table, which goes stale until a brain works again; a standby takes over within about 11 s. |
| Postgres | Nothing is served and nothing is configured until it is back. |

## Logs

Every line the api, the brain and the node write goes through `std::log`:
each program's `log.hl` logs under its logger (`voice.api`,
`voice.brain`, `voice.node`; the vendored Postgres driver and migrator
under `pq` and `migrate`), a message and its fields as logfmt (`k=v`), and
its `main` holds the one sink, voice's `RecordSink`
([`lib/logfmt`](./lib/logfmt/logfmt.hl)), which writes each event as one
logfmt record: the event's time, its level, its logger and its message,
then its fields. Info goes on stdout, warnings and errors on stderr; the
record is the same on both. Each line is data a collector parses, as the
standard pack's first clause asks (GH #1591, "Logs are structured"), and
the attach manifest declares it ([`dna/applications/voice.toml`](./dna/applications/voice.toml)).
Voice prints nothing outside the sink, and CI refuses a bare `print`,
`println`, `eprint` or `eprintln` anywhere else outside the tests
directories. One line is not voice's: the
Hale runtime's `perror` when a client hangs up while the api writes
(`lotus_tcp_send_str: write: Broken pipe` on stderr), which FRICTION.md
lists for hale.

```text
time=2026-10-19T04:12:09Z level=info logger=voice.api msg=serving url=http://127.0.0.1:8080 canned=api/canned
time=2026-10-19T04:12:09Z level=warn logger=voice.api msg="NATS unreachable" nats=nats://127.0.0.1:4222 error=...
```

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
- It shares nothing with the api except Postgres. The process boundary
  enforces that.
- Each tick it reads the fleet's reported state (seat states, heartbeats,
  quota windows, configuration), writes the route table to Postgres and
  publishes its version, and reclaims what dead api instances held. Per-tick work runs in a method, so each tick's memory is
  reclaimed.

**Which topics bind, and which use the client.** A `keyed_by` topic over
a binding is one subject that every subscriber receives and filters, so
`bindings { }` carries only what is truly a broadcast: the route table's
version on `voice.route`. The per-node and per-request subjects (`serve`, `assign`,
`cancel` to a node; a request's events and result back) are published and
subscribed through the NATS client directly, with the subjects
`spec/protocol.yaml` names, so a node receives only its own requests and an
instance only its own answers. That is what the per-node NATS permissions
rely on.

Running a second api instance is starting one; the subscriptions are
already there.

**Node**

- The plane registers over the node plane, posts seat states and
  heartbeats, and reads the assignment when the session asks; the session
  holds the NATS connection (`pond/realtime/nats`) and serves. Each is
  pinned to its own thread, because the client's reads and the node
  plane's calls block, and they talk over the bus.
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
