# The host, the membrane, the nodes

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
with `HALE_BIN` set to the toolchain that started it, waits for the
membrane sockets, attaches iris, and then supervises. Each tick it
syncs the record, mirrors GitHub when configured, relays the
membrane rows in the record, re-projects `status.json`, and answers
the organization's restart requests. It holds no Task state; when
the organization exits, the host reaps iris and exits with its code.

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
hale dna run: membrane bound at …/.hale/dna
hale dna run: the fleet `production` (…/fleet.plan.json) is the expression; `hale node <name>` runs its nodes
```

## The organism is built once per content fingerprint

An organization's seed is a handful of project files plus the whole
vendored core, and emitting its IR takes a few seconds — the same few
seconds on every start and every restart of an organism nobody has
edited. So the host builds a seed **once per content fingerprint** and
copies the binary after that. A warm start is the copy and the boot,
not the build — measured from `hale dna run` to the membrane sockets
appearing under `.hale/dna`, on one project on a quiet machine:

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
started (even one whose organization already crashed: everything the
body starts carries its mark in `HALE_DNA_BODY`, and the fence finds
it by that mark through `/proc` on Linux or `ps -E` on macOS — on a
machine offering neither it says so at startup and reaches only what
hangs off a living organization), and writes why to
`.hale/dna/body.fence.status`. The host reads that file at the top of
every tick, before it relays anything onto the membrane or restarts
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
record, body, head, fleet, knowledge, trust — detected from the
pieces, never from a stored label; `status` carries the same two
lines at its foot.

## A body on a server

The first combination beyond local is a local head with a remote
body: the organization on a server, your clone as the head, the
record over the git remote as the membrane between them.
`hale dna body provision` makes the body over ssh:

```text
$ hale dna body provision riley@srv --dsn postgres://dna:…@db:5432/dna
body provision: riley@srv
    installing hale 0.20.0
    cloned into $HOME/dna/chat
    knowledge: the DSN given
    unit hale-dna-chat-3f9c2a1b7d04 enabled and started (systemctl --user)
    dna.body = riley@srv (start/stop/logs go there); the body takes the lease when its unit starts
```

In order: the toolchain `hale.lock` pins, the record's remote cloned,
Postgres from `dna/compose.yaml` or the DSN you give, and a systemd
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
organization and the knowledge service. Without `--body` it goes on
this machine. A body that starts with no credential for its model
says so at once, on the board and in `status`, instead of on a task
hours later.

## The membrane

The organization binds four typed topics on unix sockets under
`.hale/dna/`:

| topic | subject | who publishes |
|---|---|---|
| `ReviewVerdict` | `dna.review.verdict` | `hale dna review <id> <verdict>`, the page, iris; the host, relaying a `review.verdict` row |
| `IntentOffered` | `dna.intent.offered` | `hale dna ask`, the page, iris; the host, relaying an `intent.requested` row |
| `ExpressionObserved` | `dna.expression.observed` | the host, after an observation window |
| `PressureRaised` | `dna.pressure.raised` | `hale dna pressure raise`, the page, a metrics relay |

A verdict is admitted by the Review that owns it — authority,
independence from the author, and the candidate digest are checked
there, never by the transport. Intent goes through the Board. The
observation report is judged by the substrate against the Mutation's
state. Pressure is counted by source. The sockets are how the
outside gets *in*; nothing about a decision lives in them, and the
same facts as rows in the record reach the same loci through the
host's relay.

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

`hale dna ui` is a Hale program from the toolchain cache serving one
page and a small API. Every request runs one offline verb of `hale
dna` in the project root and returns what it printed; the forms send
a verdict, an intent or a pressure signal the way the CLI does and
do not wait. It reads nothing itself and decides nothing, so with or
without a host it shows what the CLI shows. Iris stays the observer:
attached to the organization's process it renders the org as the
live topology it is, with the status projection as a third source
beside the segment and the artifact.

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

Then nothing is served without signing in, a verdict from the page is
recorded in the name the subject maps to — with the board's authority
if `dna.oidc.board` lists that name — and a subject you have not mapped
gets no session at all. The head speaks plain HTTP; put TLS in front of
it. Without `dna.principal`, the surface trusts whoever can reach it,
as it always has.

### The cockpit head

`iris/cockpit/start.sh [project]` starts the cockpit's own head,
`dna/api/project_service`: one loopback process the browser talks
to, which serves the shell, keeps a registry of your projects, and
proxies the Record reads and commands to a per-project API child.
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

The head keeps its files under
`${HALE_IRIS_HEAD_STATE:-${XDG_STATE_HOME:-~/.local/state}/hale/iris/head}`:
the registry, the receipt journal, one directory per run, and the
pid files of its children — the API child, a local body started
with `hale dna run|dev … --no-iris`, and the observer (`hale iris
--membrane`) it can start once the membrane is up. A body or observer
the head started outlives it; the next head over the same directory
re-adopts them. Secrets never enter the head: `dna.secret.set` names
a *source* — a `0600` one-line file under
`${XDG_CONFIG_HOME:-~/.config}/hale-dna/sources/<NAME>`, consumed once
the verb succeeded, or an environment variable the run's shell reads —
and the value is in no request, journal line or log. The head is
trusted-local: it acts as `USER`, and a project whose
`dna.principal` is `oidc` is refused at attach.
