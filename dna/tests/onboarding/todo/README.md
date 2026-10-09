# todo: a realtime todo list that people, scripts and agents share

A todo list that keeps one list and tells everyone who is watching the moment
it changes. People and scripts add, complete and remove items through one
api; an agent operates the same api through MCP; browsers only watch, over a
stream. When an item passes its due time the list says so to the organization
it lives in, which reads it as a concern.

Todo is written in [Hale](https://github.com/hale-lang/hale). It is small on
purpose: one process, one contract, one stream. It exists to be the
application a DNA organization is started around, torn down and started over
until that is seamless. The organization is the point; the list is what it
has to keep working.

## Constraints

- `axiom` **One api.** Every change to the list goes through the surface `Todo`
  of [`api/`](./api/), over HTTP for people and scripts and over MCP for an
  agent; there is no other way in. [`spec/todo.md`](./spec/todo.md) describes it.
- `axiom` **The browser only reads.** A browser holds no write path: it
  observes the stream `TodoChanged` and lists the items, and never adds,
  completes or removes. [`ui/`](./ui/) is that observer; the stream is in
  [`spec/todo.md`](./spec/todo.md).
- `axiom` **An agent operates it as a caller, not as a maintainer.** It reaches
  the list through MCP or `hale api call`, with a token that names its role.
  [`spec/todo.md`](./spec/todo.md) lists what it is offered.
- `axiom` **Roles are read and write.** `reader` lists and observes; `editor`
  also adds, completes and removes. The role is on the row of the surface, so
  it is true wherever the api is served ([`spec/todo.md`](./spec/todo.md)).
- `axiom` **The list tells the organization what it notices, and nothing else.**
  An item that becomes overdue is said once on the organism's nerves; an
  application that is not attached says nothing. It is the one thing
  [`api/`](./api/) does outside its own process.
- `practice` **The machine forms of the contract are generated, never written.**
  `spec/Todo.*` and `ui/client/todo.ts` are `hale api export` and `hale api client` of the rows,
  and a test holds them to the rows.

## The api

[`api/`](./api/) is one Hale seed, flat on purpose (a stream is bound to the
topics and loci of its own seed): the list and its surface (`todo.hl`), who a
token is (`auth.hl`), what the list says to the organization (`nerves.hl`) and
the process (`main.hl`).

```
api Todo {
    rpc Todos::add      requires: [editor];     // title, due (ISO-8601 UTC, optional)
    rpc Todos::complete requires: [editor];     // todo (the item's number)
    rpc Todos::remove   requires: [editor];     // todo
    rpc Todos::list     requires: [reader];     // open_only (optional)
    rpc Todos::overdue  requires: [reader];     // nothing
}

topic TodoChanged { payload: Change; }          // what: added | completed | removed, and the item
```

An item is `{id, title, done, due, overdue}`; `due` is empty for none. The list
holds 128 items. A list answers `{count, json}`, the items as a JSON array in
`json`: the api's codec carries no list.

Tokens come from the environment, `TODO_TOKENS=name=token,…`. A name may carry
its role, `ada:editor=t-ada` or `bo:reader=t-bo`; a bare name is an editor.

### Run it alone

```sh
hale test api                  # every rpc, every refusal, the stream, the sense
hale build api -o bin/todo-api
TODO_TOKENS=ada=t-ada,bo:reader=t-bo bin/todo-api
#   api on http://127.0.0.1:8080, mcp on 8090, stream on 127.0.0.1:8081
```

`--host`, `--port`, `--mcp` and `--stream` move the listeners.

### Drive it

```sh
hale api describe http://127.0.0.1:8080 --bearer t-ada
hale api call http://127.0.0.1:8080 Todos::add --title "milk" --due 2026-10-09T09:00:00Z --bearer t-ada
hale api call http://127.0.0.1:8080 Todos::list --bearer t-bo
hale api call http://127.0.0.1:8080 Todos::complete --todo 1 --bearer t-ada
hale watch ws://127.0.0.1:8081 TodoChanged --token t-bo      # the stream, one JSON line each
```

An agent: `claude mcp add todo -- hale mcp --app mcp://127.0.0.1:8090 --token t-ada`
(every member a tool), or any MCP host at `http://127.0.0.1:8090/mcp` with the
bearer.

The item is `todo`, not `id`, in `complete` and `remove`: `hale api call` takes
`--id` for the call itself, so a field of that name could not be set from a
flag.

### The sense

`TodoOverdue` is the list's own topic: once a second the list publishes it for
each open item whose `due` has passed, once per item. When the application is
attached, the host hands it the nerves (`HALE_DNA_NATS_URL_APP`,
`HALE_DNA_NATS_USER_APP`, `HALE_DNA_NATS_VAULT_APP`, `HALE_DNA_NATS_ORG`) and
`nerves.hl` says each one as `<org>.app.<name>.concern.raised` (`<name>` is the
project's, read from the account `app-<name>`: `todo`), with the item as the
payload: the heart reads it as a concern. Without them it says nothing.
The client is the smallest piece of core NATS that says one message and waits
for the server to have it; the application's password is revealed on the
CONNECT line and nowhere else.

## The page

[`ui/index.html`](./ui/index.html) is one static page, no framework. It lists
the items and updates on each `TodoChanged`, through the generated TypeScript
client. The token is the page's URL:

```
http://127.0.0.1:8088/?token=t-bo          # &hub=ws://host:port if the stream is not on :8081
```

A browser runs JavaScript, and the generated client is TypeScript, so the
page imports `client/todo.js`, which `tsc` makes from `client/todo.ts` (the
`ui` service of [`compose.yaml`](./compose.yaml) does it; by hand:
`tsc --target es2022 --module es2022 --lib es2022,dom client/todo.ts` in
`ui/`). Serve `ui/` with any static server. The page reads through the hub,
not the api's HTTP listener: a page on another origin cannot call the api (it
answers no CORS preflight), but a WebSocket crosses origins, and the hub
answers the `list` call too.

## The contract

| Document | What it specifies | Served by | Consumed by | Over |
|---|---|---|---|---|
| [`todo.md`](./spec/todo.md) | The calls of `Todo`, their roles, and the stream `TodoChanged`. | `api` | callers, an agent, `ui` | HTTP and WebSocket |

The machine forms of the same rows sit beside it
([`Todo.openapi.json`](./spec/Todo.openapi.json),
[`Todo.mcp.json`](./spec/Todo.mcp.json),
[`Todo.description.json`](./spec/Todo.description.json),
[`Todo.json-schema.json`](./spec/Todo.json-schema.json),
[`Todo.proto`](./spec/Todo.proto)), and [`DIGEST`](./spec/DIGEST) moves when a
member, a shape or a role does. `hale api export --surface Todo --out spec/ api`
and `hale api client --surface Todo --lang ts --out ui/client/todo.ts api`
regenerate them; `hale test api` runs their `--check`, and that `todo.md` names
the digest.

## Running the process model

| Deployment | Runs |
|---|---|
| [`compose.yaml`](./compose.yaml) | `api`, `ui` |

```sh
hale build api -o bin/todo-api
docker compose up
```

The organism's own services (the record, the nerves, the host) are not here;
`hale dna dev` brings them up.

## What the organization is for

Todo is the application; the organization is what changes it. The first
organization started around it initializes from this repository (its purpose,
the constraints above, the processes and the contract), ratifies the positions
that proposes, and delivers one real feature end to end: an agent claims it,
reads its brief, implements it and submits evidence from the command line,
review reaches the position the graph names, and the work settles. Later a
contract changes, the repository is ingested again, and the graph follows
through reviewed proposals. [`GRAPH.md`](./GRAPH.md) says what the first
ingest should produce, so a run can be checked against it.

The items on the list and the organization's own work are separate records:
the list is the application's data; the work is the organization's.
