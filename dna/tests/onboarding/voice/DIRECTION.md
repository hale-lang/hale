# Direction

Voice is one inference endpoint over local engines and hosted providers. A
caller names a model and presents a project key. Voice decides which seat
serves it, enforces the project's limits and budget, and records what the
request used. The full design is in
[hale-lang/voice#1](https://github.com/hale-lang/voice/issues/1). This file
records the direction settled so far. It is the first thing in the repository
because the contract comes before the code.

Each decision below is marked by kind, so a tool reading this repository
does not have to guess: `axiom` is a decision, declared and irreducible;
`derived` is a consequence of the axioms, kept as advice; `practice` is a
rule about how work is done, advice until ratified; `law` is a practice
proposed as law. The mark is the code span that opens a list item, and the
item's bold sentence is its name. The files an axiom links to are what it
shaped.

## The contract

The API is specified in [`openapi.yaml`](./spec/openapi.yaml) (the coordinator) and
[`node.yaml`](./spec/node.yaml) (voice's own node, locally), and the connection between them in
[`protocol.yaml`](./spec/protocol.yaml) (AsyncAPI). They are the source of truth;
code conforms to them.

- `axiom` **The caller API is an open standard.** The core endpoint is
  [Open Responses](https://www.openresponses.org/specification), version
  `2026-04-24`. Its shapes are taken by reference from the upstream document,
  vendored in [`spec/vendor/open-responses/`](./spec/vendor/open-responses/).
- `axiom` **Voice extends the standard only in the ways it allows.** Extra fields live
  in one optional `voice` object. New stream events carry the `voice:` prefix.
  Caller tags use the standard `metadata` field, which voice stores untouched.
- `axiom` **Chat Completions is a compatibility surface.** It exists so clients that
  speak only the older format work unchanged.
- `axiom` **Every request belongs to one project and one key.** Tokens and spend are
  tracked per project and broken down per key. A project's budget is a hard
  cap on spend per period.
- `axiom` **Responses report what actually served them.** Model, quantization, node,
  seat, account, tokens, price and timing come back on every response and every usage record.
  Hale's DNA accounts from those records.
- `axiom` **Callers never name a node.** They may name the account that pays, for
  when billing depends on it. The coordinator proxies and never redirects
  ([`openapi.yaml`](./spec/openapi.yaml): `voice.account`, `Served`).

## Configuration

- `axiom` **Nothing is discovered.** Everything that serves is configured by
  an operator, and the API ([`openapi.yaml`](./spec/openapi.yaml)) is the
  first-class way to do it; the UI is one client of it. A node declares what
  it offers ([`node.yaml`](./spec/node.yaml),
  [`protocol.yaml`](./spec/protocol.yaml)); nothing is found by looking.
- `axiom` **Only the api is exposed.** NATS and the nodes are on the private
  network, so a node talks to the coordinator over NATS
  ([`protocol.yaml`](./spec/protocol.yaml)), and the process model publishes
  nothing else ([`compose.yaml`](./compose.yaml)).
- `derived` **A node** is one machine running voice's node software, on the private
  network with NATS. An operator creates it on the
  coordinator, which issues an enrollment token, and starts the node with
  that token and the api's address. The node registers itself, subscribes to
  its own NATS subject, and reports its state to the api.
- `axiom` **The node decides what is possible; the coordinator decides what runs.**
  A node's operator lists, in a file on the machine
  ([`node.yaml`](./spec/node.yaml)), the engines it offers
  (the static engine, or a CLI with the login it runs under) and its hard
  limits. The node declares them when it connects. **A seat** is one of
  those engines, spending one account, and is configured on the coordinator,
  which sends each node its seats as an assignment
  ([`protocol.yaml`](./spec/protocol.yaml)). The node refuses a seat outside
  what it offers.
- `axiom` **Local controls only restrict.** On the machine, an operator can pause the
  node or a seat, or lower its in-flight cap, and nothing more
  ([`node.yaml`](./spec/node.yaml)).
- `axiom` **The protocol is the seam.** Any program that speaks
  [`protocol.yaml`](./spec/protocol.yaml) and the api's node plane can be a
  node. Voice ships the api, the coordinator
  and its own node.
- `axiom` **One admin UI**, in this repository ([`ui/`](./ui/README.md),
  [`DESIGN.md`](./ui/DESIGN.md)), served by the api from its own origin,
  administers every node. The backend is Hale; the UI is not, so the api is
  the trust boundary ([`openapi.yaml`](./spec/openapi.yaml)): every rule is
  enforced there, and the UI is an untrusted client.
- `derived` **An account** is what pays and what has limits. Several seats may spend
  one account (the same login on two machines) and share its limits. Voice
  never holds the credential; the engine does, on its node.
- `derived` **The catalog** of models is configured on the coordinator. Seats refer to
  its ids, so one model routes across every seat that serves it.
- `derived` **Projects and keys** name the accounts they may spend. A request may name
  one account; it is then served on that account or refused, never moved to
  another. A key for client work names the work account, so everything it
  sends is billed there.

Only what a process needs before its API answers is set at boot: addresses,
the data directory, tokens.

Admin authentication is middleware in front of the admin APIs, not part of
them. The MVP runs without it, on loopback; OIDC drops in later with no change
to the endpoints. Project keys and nodes' enrollment tokens are enforced from
the start, because the MVP exists to prove them.

## Agents

- `axiom` **No bypass flags.** A seat that runs an agent CLI never passes the flag
  that skips its permissions. Each CLI runs under its own policy (allow
  lists, its sandbox mode), inside a container as the outer boundary, with
  the worktree as the only thing it may write ([`node/`](./node/)).
- `axiom` **Logins stay with the official binaries.** Voice never extracts a
  token or calls a provider's backend on a subscription's behalf; the CLI
  holds its login on its node ([`node/`](./node/)).

## Process model

The coordinator is two roles. **Api** instances serve callers, the admin
plane and nodes' connections; they hold no state of their own and scale out.
One **brain** turns the fleet's reported state into a route table and
publishes it over NATS; api instances route from the latest snapshot, and
a node accepts or refuses each request, so the table need not be exact.
Postgres holds all durable state: configuration, reservations and counters,
the fleet as reported, and the ledger.

Requests move between the pieces as Hale bus topics, keyed by node and by
request, bound to NATS from the MVP on: an api instance publishes `serve` on
the node's subject, and the node streams the answer to the reply subject
that request named. No instance holds a node's connection, so nothing is
forwarded. The [README](./README.md) documents the architecture.

- `axiom` **The MVP runs the real process model.** The api and the brain are
  separate processes from the start (one api, one [brain](./brain/)), so the
  process model is the real one from the first slice
  ([`compose.yaml`](./compose.yaml)). The brain reaches the rest only through
  Postgres, and the process boundary makes that a fact rather than a
  convention.
- `axiom` **State never travels as events; it stays in Postgres.** What
  crosses NATS is a snapshot or a request
  ([`internal.yaml`](./spec/internal.yaml)); reservations and settlement are
  transactions ([`store.md`](./spec/store.md)).

## Phases

The API stays the same through every phase. Only what stands behind it
changes.

1. **MVP.** One coordinator and one node on one machine. The node runs a
   static seat that answers every request with its configured text. Every
   piece is set up through the APIs: a model, an account, the node, its seat,
   a project and a key. This proves configuration, routing, keys, projects,
   limits, budgets, usage records and streaming end to end without a model.
2. **v1.** Seats wrap the operator's own agent CLIs, several accounts at
   once: `claude -p` on each Claude account, `codex exec`, and others. They
   report the usage and quota windows the CLIs return, so usage across every
   account is visible in one place. The CLIs are a starting point, not a
   design constraint.
3. **Later.** Local engines (llama.cpp server, MLX, vLLM) join as seats, with
   routing by memory fit, load and weights on disk.

The first useful version is the one that lets its operator route their own
tooling through voice and see how many tokens each project uses.

## Delivery

- `axiom` **Real development is delivered through a DNA organization.** This
  repository holds the design and the chalk lines: the contracts, the
  process model running from `docker compose up`, a stub of the api, and
  skeletons of the brain and the node. The MVP itself is delivered by a DNA
  organization built from this repository, with the org chart and the
  process model as two perspectives over one graph
  ([`GRAPH.md`](./GRAPH.md), hale-lang/hale#1092). Until that tooling
  exists, work here stays at the level of contracts, stubs and skeletons.

## Open questions

- **Anthropic Messages.** Claude Code and the Anthropic SDKs speak the
  Messages format. Routing them through voice needs a Messages endpoint beside
  Open Responses.
- **Default data class.** Requests default to `internal`. A seat that
  forwards to a hosted provider must be allowed to carry `internal`, or the
  default must change.
- **Several logins per CLI on one node.** How each CLI keeps them apart:
  [hale-lang/voice#2](https://github.com/hale-lang/voice/issues/2).
- **Caller-defined tools.** A backend that cannot return tool calls should
  refuse caller tools with a clear error.
- **License.** Not chosen yet.
- The open questions in issue #1 on residency, coordinator placement and
  fair-share limits still stand.
