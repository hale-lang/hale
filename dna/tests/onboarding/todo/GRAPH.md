# The repository as a graph (draft)

What `hale dna init` should ingest from this repository, exactly enough to be
checked against. Todo is small: one process that serves a contract, a page
that watches it, and a deployment that runs the two. This document lists the
nodes and edges the repository holds today, the holes `init` should propose
to fill, what the first run actually produced and where it fell short (under
**Open**), and the two perspectives as queries. The conventions (marked list
items, declaring tables) are voice's: see `../voice/GRAPH.md`.

Nothing here is authored twice: every node is a thing the repository already
has, and every edge a reference it already makes.

## The nodes, as they stand

**purpose**: the README's title and first paragraph: a todo list that keeps one
list and tells everyone who is watching the moment it changes, to people and
scripts through one api, to agents through MCP, to browsers through a stream.

**axiom** (the `axiom` items of the README's Constraints, in its order):

1. One api.
2. The browser only reads.
3. An agent operates it as a caller, not as a maintainer.
4. Roles are read and write.
5. The list tells the organization what it notices, and nothing else.

**process**: `api`, `ui` (the compose services).

**seed**: `api` (its `hale.toml`). `ui` is not one: it has no manifest, so it is
a process with no code of its own in the graph (see Open).

**contract**: `spec/todo.md`. The files beside it (`Todo.openapi.json`,
`Todo.mcp.json`, `Todo.description.json`, `Todo.json-schema.json`,
`Todo.proto`, `DIGEST`) are its machine forms and are not nodes (see Open).

**noun**: none. The contract is prose; no schemas are read from it.

**deployment**: `compose` (built).

**practice**: `The machine forms of the contract are generated, never written`
(the README's `practice` item), proposed as advice.

**gate**: `ci/api`, `ci/ui` (the jobs of `.github/workflows/ci.yml`).

**document**: `README.md`, `GRAPH.md`, `spec/todo.md` (the dry run's copy did
not yet hold `GRAPH.md`, so it counted two).

**witness**: none. There is no `FRICTION.md` yet.

## The edges, as they stand

`unfold`:
- `purpose` to each of the 5 axioms.
- `api` (process) to seed `api`.
- *(holes: the positions under the processes and the deployment, and the board
  under `purpose`; below.)*

`meets`: `spec/todo.md`, over HTTP and WebSocket: served by `api`; consumed by
callers, an agent (plain words, outside the repository) and `ui`.

`runs`: `compose` to `api`, `ui`.

`constrains` (an axiom to the nodes among the files it links): from the links
in the README, 1 to seed `api` and `spec/todo.md`; 2 to `spec/todo.md` (`ui/`
is no node); 3 and 4 to `spec/todo.md`; 5 to seed `api`. The run wrote 5
`constrains` edges, one per axiom (journal events 14, 17, 20, 23, 26); the
listing truncates an edge's members, so which nodes each names is expected,
not read. An axiom whose links reach no node (the first draft's links were
`spec/`, `ui/` and a `.json`) writes no edge.

`gates`: `ci/api` to seed `api`. `ci/ui` gates nothing: the job's one step names
a file under `ui/`, and `ui` is not a seed.

`witnesses`, `binds`, `holds`, `reviews`: none. The last two are the holes.

## The holes: what `init` should propose

A *part* is a process of the repository, or a seed no process unfolds into.

- **A `board`** under `purpose`.
- **A reviewer per part that serves or consumes a contract**: `api` (serves
  `todo.md`) and `ui` (consumes it). Each carries its `reviews` edges: the
  part's seed, if it has one, and the contract. `api/reviewer` signs the api's
  code and `todo.md`; `ui/reviewer` signs `todo.md`.
- **A dev and a work item per part a gate guards**: `api` (done when `ci/api`
  passes). Not `ui`: no gate guards a seed of it.
- **An operator per deployment**: `compose/operator`, which signs the compose
  file.
- **Operational roles**: none by default.
- **Practices**: the one the README marks, as advice.

That is 6 proposals from the repository (the board, 3 positions, 1 work item, 1
practice) and the declared purpose's, plus the 15 practices DNA proposes for
every organization (8 `design`, 7 `operating`).

## The two perspectives, as queries

**The org chart** once the proposals are ratified (every position unfilled):

```
purpose
  board
  api         dev, reviewer(todo.md)
  ui          reviewer(todo.md)
  compose     operator
```

**The process model**:

```
callers, an agent, ui  --HTTP and WebSocket: todo.md-->  api
compose runs { api, ui }
```

## Checkable

`hale dna init` on this repository, at this commit, produced (dry run, on a
scratch copy of the directory made a repository of its own, with the project
named for the copy): 1 purpose, 5 axioms, 2 processes, 1 seed, 1 contract, 0
nouns, 1 deployment, 2 gates, 2 documents (3 with this file), 0 witnesses, 0 positions,
0 work;
14 edges (5 `unfold` from the purpose, 1 `unfold` from `api`, 5 `constrains`, 1
`meets`, 1 `runs`, 1 `gates`); 7 proposals from the repository plus the 15
stock practices. A later commit that adds a contract, a seed or a decision
changes those counts, and the diff between two ingests is the review.

## Open

Where the first run and this document disagree, or where the shape is the
repository's to change once the kickstart lands.

- **No application is attached.** Run on the repository root, `init` is in its
  repository mode: it records the graph and the purpose but not
  `application.attached`, so no application named `todo` exists for the record,
  and the broker account `app-todo` (publish-only on `<org>.app.todo.>`) that
  `HALE_DNA_NATS_USER_APP` names is not drawn for it. The sense in
  `api/nerves.hl` is mute until it is. Attaching an application is
  `hale dna init <app-dir>`, which puts `dna/` under that directory; this
  repository keeps the seed in `api/` and the organization at the root.
- **The exported contract is not read.** `init` takes `spec/*.yaml`, `*.yml` and
  `*.md` as contracts and `components.schemas` as nouns. `hale api export`
  writes JSON and a `.proto`, so `spec/todo.md` is a short hand-written index
  (a test holds its digest to `spec/DIGEST`), the repository has no nouns
  (`Todo`, `TodoItem`, `Change` and the rest never reach the graph) and the
  `refers` edges between them cannot exist.
- **`ui` is a process with no seed**, so it has no dev, no work item, and
  `ci/ui` guards nothing. A `package.json` (or a `hale.toml`) under `ui/` would
  make it one; the page is deliberately buildless.
- **The repository's own practices are one.** The README's `practice` item is
  the only one marked; the stock `design/*` and `operating/*` practices are
  DNA's and arrive with every organization.
- **Nothing proposes the equipment or mandates** the kickstart plan's second
  item describes (a library bound to `language:hale`, to `system:dna`; the
  application-side holes): the first run proposed positions, one work item and
  practices only.
- **The org's name is the directory's.** The application name a record
  attaches by (`app-<name>`) is the project's directory name; the sense reads
  it off the account name, so the repository may be cloned under any name, but
  the heart reads events under `app.<name>.` only for that name.
- **Side effects of a dry run** worth knowing before a second one: `init`
  writes passwords into the user's vault (`postgres-owner-<project>`,
  `nats-<org>-*`) as well as into the repository (`vendor/dna`, `dna/`,
  `.gitignore`, `hale.toml`).
