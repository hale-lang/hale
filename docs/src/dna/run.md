# The host, the membrane, the nodes

```sh
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
hale dna run [project] [--port N] [--no-iris] [--observe <secs>]
hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]
hale dna ui [project] [--port N]
```

## The host

`run` and `dev` are one **stateless host**. It cuts a fresh artifact
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
