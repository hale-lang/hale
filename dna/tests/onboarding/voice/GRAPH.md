# The repository as a graph (draft)

What `hale dna init` should ingest from this repository, exactly enough to
be checked against. Voice is one recursive hypergraph. The org chart and
the deployed process model are two perspectives over it, not its trunk;
the process model settled into its shape because of the goals and
constraints, and changes when they do. This document lists the node kinds,
the hyperedge kinds, the nodes and edges the repository holds today, the
holes `init` should propose to fill, and the two perspectives as queries.

Nothing here is authored twice: every node below is a thing that already
exists in the repository, and every edge a reference it already makes.

## The two conventions

Most of the graph is in the repository's structure already: compose
services, manifests, CI jobs, the specs. What the structure cannot say is
written in markdown, in two forms, so ingest never guesses.

- **A marked list item.** A list item that opens with a code span naming
  its kind (`axiom`, `derived`, `practice` or `law`) declares a node. Its
  bold sentence is its name. The repository files an `axiom` item links to
  are what it shaped. `DIRECTION.md` uses it.
- **A declaring table.** A table whose columns name an edge's roles
  declares one edge per row. `Served by | Consumed by | Over` declares where
  processes meet at the row's contract (its first link); `Deployment | Runs`
  declares what a deployment runs. In a cell, a name in code is a process or
  a seed of this repository, a link is the file it points to, and plain
  words are a party outside it. When `Over` names a process, that process
  carries the contract. The README uses both.

## Node kinds

| Kind | What it is | Where it comes from |
|---|---|---|
| `purpose` | The horizon: what voice is for. One node. | `README.md`, first paragraph |
| `axiom` | A goal or constraint, declared and irreducible. What the rest was shaped by. | List items marked `axiom` |
| `process` | A thing that runs as its own process. | `compose.yaml` services |
| `seed` | A Hale seed or package: a unit of code with its own gate. | A directory with `hale.toml` or a package manifest, the outermost one (a workspace's members are its own) |
| `contract` | A specification two or more processes meet at. | `spec/*.yaml`, `spec/*.md`, `spec/vendor/*` |
| `noun` | A schema inside a contract: a thing the system knows. | `components/schemas`, the store's tables |
| `deployment` | An instance of the process model, running or named. | `compose.yaml` (built); the README's `Deployment` table |
| `practice` | A rule about how work is done, as advice until ratified as law. | List items marked `derived`, `practice` or `law` |
| `gate` | A check that must pass. | Jobs in `.github/workflows/ci.yml` |
| `document` | Prose that explains or decides. | Every `.md` |
| `witness` | Evidence that something bit: a friction entry or a filed issue. | `FRICTION.md` entries, and the issues they link |
| `position` | A role with authority over some nodes. **None exist yet.** | Authored at `init` |
| `work` | A deliverable with a done-when. **None exist yet.** | Authored at `init` |

## Hyperedge kinds (arity two or more)

| Edge | Reads as | Where it comes from |
|---|---|---|
| `unfold(Σ, [children])` | This node is made of these. | `purpose` to every axiom; a process to the seed of the same name |
| `meets(contract; servers; consumers; carriers)` | These processes meet at this contract; the server implements it, the consumers depend on it, the carrier transports it. | The README's contract table |
| `names(contract, [nouns])` | This contract defines these nouns. | Its schemas, its tables |
| `refers(noun, noun, via)` | A `$ref` or foreign key. | The same |
| `constrains(axiom, [nodes])` | This decision shaped these. | The files an `axiom` item links to |
| `runs(deployment, [processes])` | This deployment runs these. | `compose.yaml`; the README's deployment table |
| `gates(gate, [nodes])` | This check guards these. | The seeds, contracts and `compose` a job's steps name |
| `binds(practice, [nodes])` | This rule applies to these. | A ratified practice's bindings |
| `witnesses(witness, [nodes])` | This evidence is about these. | The paths an entry names |
| `holds(position, holder)` | A person or agent holds this role. Empty today. | |
| `reviews(position, contract)` | This role signs changes to this contract. Empty today. | |

## The nodes, as they stand

**purpose**: one inference endpoint in front of many model backends; a
caller names a model and presents a project key, and voice picks a seat,
enforces the project's limits and budget, and records what the request
used.

**axiom** (the items `DIRECTION.md` marks `axiom`, in its order; the ones
that are consequences rather than decisions are marked `derived` there and
are practices):

1. The caller API is an open standard.
2. Voice extends the standard only in the ways it allows.
3. Chat Completions is a compatibility surface.
4. Every request belongs to one project and one key.
5. Responses report what actually served them.
6. Callers never name a node.
7. Nothing is discovered.
8. Only the api is exposed.
9. The node decides what is possible; the coordinator decides what runs.
10. Local controls only restrict.
11. The protocol is the seam.
12. One admin UI.
13. No bypass flags.
14. Logins stay with the official binaries.
15. The MVP runs the real process model.
16. State never travels as events; it stays in Postgres.
17. Real development is delivered through a DNA organization.

**process**: `postgres`, `nats`, `api`, `brain`, `node`.

**seed**: `api`, `brain`, `node` (each a `hale.toml`), `spec/validate`,
`ui` (each a `package.json`; `ui/apps/*` and `ui/packages/*` are members of
`ui`'s workspace, under its one gate).

**contract**: `spec/openapi.yaml` (caller, admin and node planes),
`spec/node.yaml`, `spec/protocol.yaml`, `spec/internal.yaml`,
`spec/store.md`, `spec/vendor/open-responses` (taken by reference).

**noun** (the ones every perspective needs; the contracts hold ~90 schemas
and 18 tables in all): model, account, node, seat, engine, capabilities,
local controls, assignment, project, key, budget, limit, request,
reservation, usage record, route table, change, policy change, admin event.

**deployment**: `compose` (built, and brought up by `ci/image`),
`personal` (named), `scaled-out` (named).

**practice**: the four items `DIRECTION.md` marks `derived` (a node, an
account, the catalog, projects and keys). The other rule-shaped bullets in
`DIRECTION.md`, `ui/DESIGN.md`, `README.md` and `ui/README.md` are
practices once marked: the contract comes before the code; a change to a
contract is a new migration, never an edit; the brain talks to nothing but
Postgres and the route table it publishes; per-node and per-request
subjects use the NATS client, only broadcasts bind; color is earned by
activity; volume is logarithmic; ground truth beside intent; tokens are
framework-free and components know none of voice's nouns; the hale
toolchain version is pinned in the action and the Dockerfile together;
keep a `FRICTION.md`.

**gate**: `ci/api`, `ci/brain`, `ci/node`, `ci/ui`, `ci/contracts`,
`ci/image`.

**document**: `README.md`, `DIRECTION.md`, `FRICTION.md`, `GRAPH.md`,
`ui/README.md`, `ui/DESIGN.md`, `spec/store.md`.

**witness**: the three `FRICTION.md` entries (a `Router.add` handler
dangles; no triple-quoted strings; no reason phrases) and the issue the
first links, hale-lang/hale#1048.

## The edges, as they stand

`unfold`:
- `purpose` to each of the 17 axioms.
- `api` to seed `api`; `brain` to seed `brain`; `node` to seed `node`.
- *(holes: the positions under every process and deployment, and the
  board under `purpose`; below.)*

`meets` (the contract as the meeting point; the README's contract table, in
its order):
- `openapi.yaml`, over HTTP: served by `api`; consumed by callers, `ui`,
  `node` (its node plane).
- `node.yaml`, over HTTP: served by `node`; consumed by the machine's
  operator.
- `protocol.yaml`, over NATS: served by `node`; consumed by `api`; carried
  by `nats`.
- `open-responses`: served by nobody here; consumed by `openapi.yaml`,
  `protocol.yaml` (by reference).
- `store.md`, over SQL: served by `postgres`; consumed by `api`, `brain`.
- `internal.yaml`, over NATS: served by `brain`; consumed by `api`; carried
  by `nats`.

`names` and `refers`: as the `$ref`s and foreign keys already state; for
instance seat to engine (a node's declared engine), seat to account, seat
to model, request to project, key, instance; usage to request.

`constrains` (an axiom to what it links to; the edges that explain the
process model):
- 1 (an open standard) to `open-responses`.
- 6 (callers never name a node) to `openapi.yaml`.
- 7 (nothing is discovered) to `openapi.yaml`, `node.yaml`,
  `protocol.yaml`.
- 8 (only the api is exposed) to `protocol.yaml` (NATS, not WebSocket),
  `compose` (what is published).
- 9 (the node decides what is possible) to `node.yaml`, `protocol.yaml`.
- 10 (local controls only restrict) to `node.yaml`.
- 11 (the protocol is the seam) to `protocol.yaml`.
- 12 (one UI, the api is the trust boundary) to `ui/README.md`,
  `ui/DESIGN.md`, `openapi.yaml`.
- 13 and 14 (no bypass flags; logins stay with the binaries), each to seed
  `node`.
- 15 (the MVP runs the real process model) to seed `brain`, `compose`.
- 16 (state never travels as events) to `internal.yaml` (the route table is
  a snapshot), `store.md` (reservation and settlement are transactions).
- 17 (delivered through DNA) to this document.

`runs`:
- `compose` to `postgres`, `nats`, `api`, `brain`, `node`.
- `personal` to `api`, `brain`, `postgres`, `nats` on one machine; `node`
  per machine with logins.
- `scaled-out` to `api` behind a load balancer, `brain` with a standby,
  `postgres`, `nats`, `node` anywhere.

`gates` (what each job's steps name):
- `ci/api` to seed `api`, `openapi.yaml` (live), seed `spec/validate`.
- `ci/brain` to seed `brain`.
- `ci/node` to seed `node`, `node.yaml` (live), seed `spec/validate`.
- `ci/ui` to seed `ui`.
- `ci/contracts` to `openapi.yaml`, `node.yaml`, `protocol.yaml`,
  `internal.yaml`, seed `spec/validate` (the canned bodies against
  `openapi.yaml`).
- `ci/image` to `compose`.

`binds`: none until a practice is ratified; each then binds the seeds or
contracts it names, e.g. "only broadcasts bind" to `api`, `node`;
"components know none of voice's nouns" to `ui`; "a contract change is a
new migration" to `store.md`.

`witnesses`:
- the `Router.add` entry, and hale-lang/hale#1048, to seed `api` (the
  `is_route` ladder in `api/endpoints.hl`).
- the triple-quoted strings entry to seed `api` (canned bodies live in
  files).

`holds`, `reviews`: none. These are the holes.

## The holes: what `init` should propose

The graph lacks exactly what a delivery needs and the repository cannot
imply. `init` proposes each of these on its own, one Board Review each,
and the Board ratifies:

- **Positions** under each process: `dev` (bound by the process's
  practices, writes) and `reviewer` (holds the contracts the process
  serves, signs; never the same holder as `dev` for that process — a
  warning where one person holds every role, `dna.trust = local`, and
  refused elsewhere). A `board` under `purpose`. An `operator` under each
  deployment. Deeper substructure (core, library, integration) when a
  process grows.
- **`reviews` edges** from each `meets`: the reviewer of a process signs
  every contract the process serves, and they come with that reviewer's
  proposal. A change to a contract also fans out to the reviewers of
  every consumer (routing, not an edge); `board` signs where the contract
  is law (none is yet).
- **Operational roles** under each deployment that no artifact implies,
  proposed empty: `support` (for a deployment's projects), `accounts`
  (account and quota management), `billing`, `on-call`. They are
  positions like any other, shown in the org chart unfilled until the
  Board names who holds them.
- **Work**: one item per process, an `unfold` of it, done when its gate
  passes: `api` (`ci/api`), `brain` (`ci/brain`), `node` (`ci/node`).
  `postgres` and `nats` have no gate, so their work's done-when is to
  define one.
- **Practices**: the four items `DIRECTION.md` marks `derived`, proposed
  as advice, and ratified as law where the Board chooses; the rest stay
  advice.

That is 35 proposals: 26 positions (the board, a dev and a reviewer under
each of the five processes, and five under each of the three
deployments), 5 work items and 4 practices.

## The two perspectives, as queries

**The org chart**: nodes are `position`s and their holders; edges are
`unfold` (which process or deployment a position is under) and `reviews`
(who signs what). Rendered for voice once the proposals are ratified,
each reviewer with the contracts its process serves:

```
purpose
  board
  postgres    dev, reviewer(store.md)
  nats        dev, reviewer
  api         dev, reviewer(openapi.yaml)
  brain       dev, reviewer(internal.yaml)
  node        dev, reviewer(node.yaml, protocol.yaml)
  compose     operator, support, accounts, billing, on-call
  personal    operator, support, accounts, billing, on-call
  scaled-out  operator, support, accounts, billing, on-call
```

Every position here is unfilled; a holder shows in brackets once the
Board names one (`dev [ada]`).

**The process model**: nodes are `process`es; edges are `meets` (which
contract, over which transport) and `runs` (which deployment). Every arrow
runs from the consumers to the server, over the transport, naming the
contract, with the carrier after it. Rendered for voice today:

```
callers, ui, node  --HTTP: openapi.yaml-->  api
the machine's operator  --HTTP: node.yaml-->  node
api  --NATS: protocol.yaml-->  node  (carried by nats)
openapi.yaml, protocol.yaml  --open-responses-->  (nobody here)
api, brain  --SQL: store.md-->  postgres
api  --NATS: internal.yaml-->  brain  (carried by nats)
compose runs { postgres, nats, api, brain, node }
personal runs { api, brain, postgres, nats, node }
scaled-out runs { api, brain, postgres, nats, node }
```

Both are views. Neither is edited directly: a change to a contract moves
through `meets` and shows up in both; a constraint added under `purpose`
moves through `constrains` and may reshape either.

## Checkable

`hale dna init` on this repository, at this commit, should yield: 1
purpose, 17 axioms, 5 processes, 5 seeds, 6 contracts, 3 deployments (1
built), 6 gates, 7 documents, 4 witnesses, 0 positions, 0 work; the edges
above; and the 35 proposals for the holes listed. A later commit that adds a
contract, a seed or a decision changes those counts, and the diff between
two ingests is the review.
