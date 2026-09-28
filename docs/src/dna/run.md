# The host, the nerves, the nodes

```sh
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
hale dna run [project] [--port N] [--no-iris] [--observe <secs>]
hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]
hale dna ui [project] [--port N]
```

## The host

`run` and `dev` are one **stateless host** — a Hale program,
`dna/host`, that ships in the toolchain beside the core and that
`hale dna` execs with the project resolved. Every child it starts —
the organization, the application, iris, on a node an instance — runs
detached through `sh`, with its pid, its exit code and its log as
files beside it, and the host echoes the logs to the terminal a tick
at a time. It cuts a fresh artifact
of the application (`.hale/dna/current.topology`), builds the
organization and execs it under `LOTUS_OBS=1` from the project root
with `HALE_BIN` set to the toolchain that started it, waits for it to
read its facts from [the nerves](#the-nerves), attaches iris to inspect
it, and then supervises. Each tick it relays the record's unanswered
requests onto the nerves (first, so an answer never waits behind the
rest), syncs the record, mirrors GitHub when configured, re-projects
`status.json`, and answers the organization's restart requests. It holds no Task state; when
the organization exits, the host reaps iris and exits with its code.

The host is also a node of memory's **spine**, beside any number of
others: on each tick — once a second — it projects the record into the
knowledge graph and the org chart, carries out an adoption the record
asks for, and erases the protected evidence the record says was
redacted, all under the record's spine role
(`HALE_DNA_MEMORY_DSN_SPINE`). Nothing coordinates the nodes; the
stores do. `dev` applies memory's schema
first, with `HALE_DNA_MEMORY_DSN_OWNER` or the database
`dna/compose.yaml` brings up, and hands the host the spine's DSN
alone; `run` takes the spine's DSN from its environment and keeps any
owner's or head's DSN away from the host. The host checks the
schema's version before it starts anything on it. See
[Operating](./operating.md#memory). The nerves go the same way: `dev`
creates the organization's stream with the owner's NATS URL
(`HALE_DNA_NATS_URL_OWNER`, or the server `dna/compose.yaml` brings
up) and hands the host the spine's URL and the organization's token;
`run` takes those two from its environment
(`hale dna nerves migrate` prints them).

Under **`dev`** the application runs under the same host, and a
restart request for it is answered here: rebuild, restart, watch the
window, report — [Apply, express, observe](./apply.md). A restart
request for the organization's own seed (`dna/org`, when the org
chart changed) is answered under both verbs: the host rebuilds and
restarts the organization and watches it.

Under **`run`** the application is expressed elsewhere. With `[dna]
fleet` in `hale.toml`, the host deploys through the record and
watches the nodes' reports; with a deployment gateway wired in the
organization, no host is asked; with neither, the host says so and
nothing is expressed.

```text
$ hale dna run . --no-iris
hale dna run: organization (pid 1874960) from … under LOTUS_OBS=1
hale dna run: the organization reads its facts from the nerves (DNA_4F…)
hale dna run: the fleet `production` (…/fleet.plan.json) is the expression; `hale node <name>` runs its nodes
```

## The organism is built once per content fingerprint

An organization's seed is a handful of project files plus the whole
vendored core, and emitting its IR takes a few seconds — the same few
seconds on every start and every restart of an organism nobody has
edited. So the host builds a seed **once per content fingerprint** and
copies the binary after that. A warm start is the copy and the boot,
not the build — measured from `hale dna run` to the organization
listening, on one project on a quiet machine:

```text
first start, or after any edit    3.7s
nothing changed since             0.15s
```

The fingerprint is over every file the build reads — the compiler
lists them itself, `hale inputs <seed>` — plus the project's manifest
and lock, the toolchain (version, and the `hale` binary's own size and
mtime, so a compiler rebuilt in place is a different fingerprint),
the environment variables that change codegen, and the seed's path.
Edit anything under `dna/org`, or upgrade `vendor/dna`, and the next
start builds. The binaries live in `$XDG_CACHE_HOME/hale/dna-build`
(or `~/.cache/hale/dna-build`), 32 of them, least-recently-used
pruned; deleting that directory costs one rebuild and nothing else.
`HALE_DNA_NO_BUILD_CACHE=1` builds from scratch every time.

## One body per record

A record admits one body at a time. Before the host builds or runs
anything it takes the body lease — at the record's remote when
there is one, so a second host on another clone is refused by the
remote itself:

```text
$ hale dna run . --no-iris
hale dna run: a body for this record is live on riley@srv:/srv/chat (ticked 2s ago, lease expires in 28s); a record admits one body — `hale dna body claim --force` takes it over when that body is gone
```

The holder is `user@host:<clone>`, so the same clone restarting takes
its lease straight back. Beside the host runs the **body fence**, a
small process of its own that renews the lease every ten seconds,
with every git call it makes cut off after eight, and all of them cut
off at the deadline the lease allows. It never waits on
the host: when the lease is someone else's, or it cannot prove the
lease within five seconds of its expiry, or the host is gone, it
kills the organization and the expression, with every tool they
started — even one whose organization already crashed. Everything the
body starts begins a session of its own, and a tool stays in its
organization's session however it is regrouped or reparented, so the
fence stops the whole session, on Linux and macOS alike. Everything the
body starts also carries a mark in `HALE_DNA_BODY`, a second net for a
tool that left the session on purpose (`setsid`), read through `/proc`
on Linux; a Mac shows no other process's environment, and the host says
at startup that such a tool is out of reach. The fence writes why to
`.hale/dna/body.fence.status`. The host reads that file at the top of
every tick, before it relays anything onto the nerves or restarts
anything, and again the moment before it starts a process, and exits
3 when the lease is not its own. So a host stuck in a hung sync or a
long build cannot keep its organization running on a lease it lost,
and a body that cannot reach the remote executes nothing past thirty
seconds.

`hale dna body` says who runs the record. When a body is gone but its
lease is still live, `hale dna body claim --force` releases it as a
row in your name (`body.claimed`, `forced: true`); that body stops
the next time it asserts, and the next `hale dna run` takes the
lease. `hale dna profile` prints the combination the organism is —
record, body, head, fleet, memory, trust — detected from the
pieces, never from a stored label; `status` carries the same two
lines at its foot.

## A body on a server

The first combination beyond local is a local head with a remote
body: the organization on a server, your clone as the head, the
record over the git remote between them.
`hale dna body provision` makes the body over ssh:

```text
$ hale dna body provision riley@srv --dsn postgres://dna:…@db:5432/dna
body provision: riley@srv
    installing hale 0.20.0
    cloned into $HOME/dna/chat
    memory: the DSN given
    unit hale-dna-chat-3f9c2a1b7d04 enabled and started (systemctl --user)
    dna.body = riley@srv (start/stop/logs go there); the body takes the lease when its unit starts
```

In order: the toolchain `hale.lock` pins, the record's remote cloned,
Postgres from `dna/compose.yaml` or the DSN you give (written to the
body's env file as `HALE_DNA_MEMORY_DSN_OWNER`, for the body's `hale
dna dev` to migrate with before it hands its host the spine's DSN
alone), and a systemd
user unit that supervises the host and restarts it on failure, so a
failure flows up one more level before it reaches you. It stops
before writing anything when ssh cannot reach the host, when the
record has no remote the body could reach, or when the host lacks
git, curl, systemd or (without a DSN) docker compose. `--dry-run`
prints the exact script. Afterwards `hale dna body start`, `stop` and
`logs` reach the unit, and `hale dna profile` says `remote body`.

Credentials go where the body runs, and never into the record:

```text
$ hale dna secret set ANTHROPIC_API_KEY --body riley@srv
value for ANTHROPIC_API_KEY on riley@srv, on one line:
secret set: ANTHROPIC_API_KEY is in /home/riley/.config/hale-dna/chat-3f9c2a1b7d04.env on riley@srv (secret.rotated ANTHROPIC_API_KEY; the value is nowhere in the record)
```

The value is read from stdin — never from the command line — and
lands in a file only that user can read; the host loads it for the
organization. Without `--body` it goes on
this machine. A body that starts with no credential for its model
says so at once, on the board and in `status`, instead of on a task
hours later.

## The nerves

Facts travel between the organism's parts over **NATS JetStream**
(pond's client and bus adapter, vendored with the core). Every
request is a row first: `hale dna task create`, a verdict, `concern
raise`, `pressure raise` and `practice propose` write their row into
the record, and **never publish anything themselves**. The host is
the one that publishes. While it runs it is its program's `main locus`,
with the organization's topics bound to the NATS adapter, and on every
tick it relays each request row the record has not answered. It relays
the row again every 30 seconds until the answer is in the record:

| topic | subject | from the row |
|---|---|---|
| `ReviewVerdict` | `dna.review.verdict` | `review.verdict` (`hale dna review <id> <verdict>`, the page) |
| `IntentOffered` | `dna.intent.offered` | `intent.requested` (`hale dna task create`, the page) |
| `ConcernRaised` | `dna.concern.raised` | `concern.requested` (`hale dna concern raise`, an application's own `concern.raised` event) |
| `PressureRaised` | `dna.pressure.raised` | `pressure.requested` (`hale dna pressure raise`, the page) |
| `PracticeRequested` | `dna.practice.requested` | `practice.requested` (`hale dna practice propose`) |
| `KnowledgeNodeRequested`, `…BindingRequested`, `…EdgeRequested` | `dna.knowledge.*.requested` | the face's knowledge commands |
| `ExpressionObserved` | `dna.expression.observed` | `observation.requested` (the host itself, after an observation window) |

On the server every subject is under the organization's token —
`dna_<id>.dna.intent.offered` — in one stream per organization,
`DNA_<ID>`, which keeps what was said for a week. The organization
reads it through its durable consumer, so a fact published while it
was restarting reaches it when it is back. Each part holds its own
credential: the owner creates the stream, the spine (the host and the
organization) publishes and reads, the head only subscribes.
`dna/nats.conf` configures `dna/compose.yaml`'s server that way, with
placeholder passwords for now.

A verdict is admitted by the Review that owns it — authority,
independence from the author, and the candidate digest are checked
there, never by the transport. Intent goes through the Board. The
observation report is judged by the substrate against the Mutation's
state. Pressure is counted by source. The nerves are how the outside
gets *in*; nothing about a decision lives in them. A fact the
transport loses costs a relay, never the fact: the row is still
unanswered, and the host publishes it again. When the stream stops
acknowledging, the host writes `nerves.lost` and exits for its unit to
start it again with a fresh connection. Over a shared record every
owner's facts travel in a space of their own, so each organization
hears only its owner's.

### The heart's events

The application is the heart: what the organism exists to keep
beating. Its one hookup to DNA is its events. It declares each one the
way a Hale program declares any topic, under a subject of its own,
`app.<app>.<event>`, and binds it to pond's NATS adapter with a codec
that writes one JSON object carrying an `id` the application chooses:

```hale,fragment
import "vendor/pond/realtime/nats" as nats;

type Usage { id: String = ""; project: String = ""; output_tokens: Int = 0; }
topic UsageRecorded { payload: Usage; subject: "app.voice.usage.recorded"; }

main locus Api {
    params { nerves: nats::NatsConn = nats::NatsConn { }; }
    placement { nerves: pinned; }
    bindings { UsageRecorded: nats::NatsAdapter { } codec(UsageJson { }); }
    // ...
}
```

It imports nothing of DNA. It connects as the `app` user
(`HALE_DNA_NATS_URL_APP`, which `hale dna nerves migrate` prints), with
the organization's token and a dot (`HALE_DNA_NATS_ORG`) as its
connection's `subject_prefix`, and `jetstream: true`, so the
organization's stream acknowledges each event. Today you hand it those
two yourself: neither `dev` nor a node passes them to the application
it starts. The host reads every application's events through one
durable, `heart`, and records each as a `reading.recorded` row,
`<app>/<event>/<id>`, before anything acts on it:

```text
hale dna run: reading voice/usage.recorded/resp_01j8z3q4v7m2k9x0 recorded
```

A reading is a signal, never a fact: it acts on nothing itself, and
what the spine does with one (an application's concern, a reflex's
firing) is a row of its own. The event
is acknowledged to the stream only once its row has landed, so a host
that stops first loses nothing, and the event comes again. The same
event and id arriving again is refused (`recorded already; refused as
a duplicate`). A body that is not a JSON object with a string `id` (as
`std::json::valid_object` admits one: at most 64 top-level members,
unique keys) is not a reading at all, and the host says why. Whatever the organism does
to the application goes the other way: through the API the application
exposes, from a leg.
`dna/tests/heart` is the smallest application that does this, and
`dna/tests/heart_reading_test.hl` runs it against a live organism.

### The senses and the reflexes

Every long-running part serves its readings for the senses to scrape:
the host (the spine) on 9464, a node on 9465, the head on 9466, or on
`HALE_DNA_SENSES_PORT`. Every series is labelled with its `part`, and a
node's with the node:

```text
dna_instance_up{part="node",node="edge-1",instance="api-0"} 0
dna_instance_since_seconds{part="node",node="edge-1",instance="api-0"} 1790558458
dna_pulse_seconds{part="node"} 1790558460
dna_model_calls_total{part="spine"} 2
```

No label is per event, per task or per revision, because a series, once
served, is never retired. A task's model calls are found through its
`model.called` rows.

The store is Prometheus, the `senses` service of your `dna/compose.yaml`.
It scrapes what `dna/senses.yml` names and keeps a week of it. `hale dna
dev` brings it up with the rest of compose, and `hale dna senses up`
brings it up on its own and prints its read URL. A part never learns
the store's address, and a reading never acts.

The reflexes (`dna/reflexes`) read the store. They hold its URL and the
nerves' `reflexes` user, which may publish their firings and nothing
else (no application's user may). The face's `start.sh` starts them
when it attaches a project. Their one rule, `instance.down`, fires when
a node reads an instance down. The firing is an application event,
`reflex.fired` on `app.reflexes.reflex.fired`, and the heart lands it as
a reading. The node it names restarts that instance and records
`reflex.acted`:

```text
hale node edge-1: reflex instance.down:edge-1:api-0:1790558458 (instance.down on api-0): restarted
```

A second firing for the same instance within ten minutes
(`HALE_DNA_REFLEX_WINDOW_SECS`) becomes a concern the organization
raises, and so does a restart that did not take. An instance that keeps
falling over is something a workflow answers to, not something the body
keeps restarting quietly. `dna/tests/senses_reflex_test.hl` runs the
whole loop on the seed's compose.

## The nodes

A node is the host's counterpart on a machine that runs instances.
It owns a clone and a name, ticks like the host, and expresses what
the record's latest `fleet.deploy` says: fetch the revision, check
it out, build and restart the instances the plan assigns to it that
the deploy touches, and append `instance.up` and `instance.exited`
in its own name. `.hale/node/<name>/` holds its pid files and
artifacts. It never decides — [Operating the
fleet](./operating.md).

## The surface

`hale dna ui` reads, and takes no command — the record's commands are
the api head's gated topics. It serves only under `dna.principal = oidc`
(GH #989), a verified sign-in in front of every page; with no principal
source it refuses to start, trusted-local being a test fixture's mode. It is a Hale program from the toolchain cache serving one
page and a small API. Every request runs one offline verb of `hale
dna` in the project root and returns what it printed; the forms send
a verdict, an intent or a pressure signal the way the CLI does — a
row a node relays — and do not wait. It reads nothing itself and decides nothing, so with or
without a host it shows what the CLI shows. Iris stays the inspector:
attached to the organization's process it renders the org as the
live topology it is, as it would any Hale binary. The face
(`dna/face`, served by [the head](#the-head)) is where the page is
headed: one shell over the same record and status projection, with
workspaces for the organization, its knowledge and practices, and the
workflow's definitions and executions. Its commands are the verbs of
this book.

On a domain, the surface is a hosted head and asks who you are. Set
the principal source to your identity provider and map the subjects
you trust to member names:

```text
git config dna.principal oidc
git config dna.oidc.issuer https://accounts.google.com
git config dna.oidc.client 1234.apps.googleusercontent.com
git config dna.oidc.redirect https://dna.example.com/auth/callback
git config --add dna.oidc.member "109876543210=riley"
git config dna.oidc.board riley
hale dna secret set HALE_DNA_OIDC_SECRET
```

The head's socket (the api binding on `dna/api`, one per record under
`$XDG_RUNTIME_DIR/hale/dna/<id12>.sock` — the record id's first twelve
characters — or the record's own
`.hale/dna`, `LOTUS_API` overriding it) knows a peer by its Unix
credentials instead, and the record says what that peer may do: map
the uid to a person the record knows, in the record's own local config,

```text
git config --local --add dna.unix.member "uid:1000=riley"
```

(never a global or an included file; the head reads it again whenever
the record moves), and a position the graph says that person holds is
a role the peer holds: `role api_dev;` is `position:api/dev`, and
`owner`, which reads the full description, is the board.
Generic clients (`hale call`, `hale mcp --app`, `hale admin`) see
exactly that principal's slice.

The record's own commands (`dna/api`) are gated topics on this same
socket: `owner` is the board, `reviewer` is `position:reviewer`, and
`position` is any position the peer holds. `git config --local --add
dna.unix.member "uid:<n>=<person>"` is how a peer becomes a person
these gates can name. The face reaches the same gates over the api
binding's own HTTP transport: `POST …/commands` goes to it under the
caller's bearer — the ID token you signed in with, or a service's
token — and the gates name the person `dna.oidc.member` maps that
subject to (a fixture's trusted-local session's launch token names the
head's own account). Revoking a mapping (`git config --local --unset`) takes
effect when the record next moves, since the source re-reads the
mapping with the edges then.

Then nothing is served without signing in, a verdict from the page is
recorded in the name the subject maps to — with the board's authority
if `dna.oidc.board` lists that name — and a subject you have not mapped
gets no session at all. The head reads the issuer's discovery document
and its signing keys (JWKS) once, when it starts, and verifies every ID
token against them: ES256 only, so an issuer that signs RS256 alone is
refused at start. A caller that is not a browser presents its ID token
as `Authorization: Bearer <token>`, and its commands are recorded in
the name its subject maps to. The head speaks plain HTTP; put TLS in
front of it.

**A service reads through the head.** A program with no person behind
it — an application that enforces the limits a project ratified —
gets its own token from the issuer with the `client_credentials`
grant, under its own client id and a secret kept in the vault, and
presents it as `Authorization: Bearer <token>` on the project's API
routes. Map it in the record, beside your members:

```text
git config --local --add dna.oidc.service "voice-api=voice"
```

It reads the project's read routes under its own name (`service:voice`,
never a person's), gets no session and no cookie, and asks nothing: any
command or lookup it sends is refused, since a command is a person's
and a service holds no position. A subject is a person or a service,
never both; a person's ID token is refused as a bearer at the head,
since a person signs in. The vault entry holds the secret alone, with
no trailing newline, and the token has to be ES256 for the head's
client, as the stub issues it.

**Local mode is OIDC too (GH #989).** `dna/face/start.sh` starts a stub
identity provider (`dna/oidc`) on the loopback beside the head, under a
key and a client secret made for that launch (0600, never on a command
line), and you sign in through it as yourself: the subject `local-sub`,
mapped to `$USER`. Only the browser that opened the URL the head prints
(it carries the head's launch token, 0600 in its state directory) may
sign in, so another account on the machine cannot. An issuer on the
loopback is anyone's who can bind the port, so a project pins its key
(`dna.oidc.key`, which the face writes at attach), and a head whose
issuer does not publish the pinned key refuses to start. There is no head without a principal source: a head started
with neither `dna.principal = oidc` nor a fixture's
`HALE_DNA_TRUSTED_LOCAL=1` refuses to start. The trusted-local session
— a launch token in `.hale/dna/head.token`, `http://…/?token=…` — is a
test fixture's mode, never a project's.

### The head

`dna/face/start.sh [project]` starts the head,
`dna/api/project_service`: one loopback process the browser talks
to, which serves the face, keeps a registry of your projects, and
proxies the Record reads and commands to a per-project API child.
The head listens on 8792, the API child's reads on 8793, the stub
provider on 8794 and the API child's commands, its api binding's own
HTTP transport, on 8795 (`--port`, `--api-port`, `--oidc-port`,
`--commands-port`).
Given no project it starts detached, and the browser's Projects
workspace is where you begin: create a project (`hale dna new`,
run for you), initialize an existing checkout (`hale dna init`), or
attach one that already has a record. Every button is a CLI verb
the head runs **detached** under `timeout … sh -e` with its pid,
exit code and log as files, and answers with a receipt that settles
on your next request — so closing the browser or restarting the
head interrupts nothing, and a verb that talks to a remote (a sync,
a publish, a probe) reports `outcome_unknown` with its log when it
passes its deadline, never a failure it cannot prove.
The browser does not poll the head: the head tells it when to look
again, over an event stream (`GET /api/hale/v1/head/events`, GH #986),
when a receipt is journaled, a run or a child starts or ends, or the
organism lands a row — which a node tells it over the nerves
([Operating](./operating.md#the-nerves)).

The head keeps its files under
`${HALE_DNA_HEAD_STATE:-${XDG_STATE_HOME:-~/.local/state}/hale/dna/head}`:
the registry, the receipt journal, one directory per run, and the
pid files of its children — the API child and a local body started
with `hale dna run|dev … --no-iris`. A body the head started
outlives it; the next head over the same directory
re-adopts them. Secrets never enter the head: `dna.secret.set` names
a *source* — a `0600` one-line file under
`${XDG_CONFIG_HOME:-~/.config}/hale-dna/sources/<NAME>`, consumed once
the verb succeeded, or an environment variable the run's shell reads —
and the value is in no request, journal line or log. The head signs
you in through the stub provider `start.sh` starts, and configures a
project it attaches for that provider (`dna.principal`,
`dna.oidc.issuer`, `dna.oidc.client`, and your `dna.oidc.member`); a
project served under another issuer is refused at attach. Every read
and command it proxies to the project's API child carries your ID
token, which the child verifies itself.
