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
its lease straight back. The host renews the lease every ten seconds
and asserts it at the top of every tick, before it relays anything
onto the membrane or restarts anything: a host whose lease is no
longer its own stops, and takes the organization with it. A host
that cannot reach the remote keeps its lease until it expires and
then stops too — a partitioned body executes nothing past thirty
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
    unit hale-dna-chat enabled and started (systemctl --user)
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
secret set: ANTHROPIC_API_KEY is in /home/riley/.config/hale-dna/chat.env on riley@srv (secret.rotated ANTHROPIC_API_KEY; the value is nowhere in the record)
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
