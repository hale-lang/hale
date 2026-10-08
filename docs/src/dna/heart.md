# The heart and the body

The **heart** is the application: what the organism exists to keep
beating. Its one job is to do its work and say what it did, as events
on its own subjects. The project owns it, as ordinary Hale source. It
fails by crashing inside an observation window, or by its pulse
stopping. It holds its own store, its own secrets, a publish-only
broker account on its own subjects and whatever API it chooses to
expose; it never holds the record, DNA's database, DNA's vault
entries or DNA's subjects.

The **body** is the hosts the organism runs on. Its job is to run the
spine under the record's lease and express the genome. The operator
owns it. It fails by losing its lease, and then it stops. It holds the
spine's full database role and the lease; it never holds a device's
session.

This page covers both, and the step between them: how an applied
change becomes the running application.

## The heart

### Its events become readings

The application's one hookup to DNA is its events. It declares each
one as a topic of its own, under the subject `app.<app>.<event>`, where
`<app>` is its project's name, and binds it to pond's NATS adapter with
a codec that writes one JSON object carrying an `id` the application
chooses. It imports nothing of DNA; pond's client comes from
`vendor/dna/pond/realtime/nats`.

Here is a project named `demo` raising a concern about one of its own
parts:

```hale
import "vendor/dna/pond/realtime/nats" as nats;

/// A concern about one of the application's own parts.
type Concern {
    id: String = "";
    source: String = "";
    what: String = "";
    severity: Int = 1;
}

topic ConcernRaised { payload: Concern; subject: "app.demo.concern.raised"; }

type CodecError { kind: String = ""; }

fn quoted(s: String) -> String {
    return "\"" + std::str::replace(std::str::replace(s, "\\", "\\\\"), "\"", "\\\"") + "\"";
}

/// One JSON object on the wire, as the heart reads an event.
locus ConcernJson {
    fn encode(c: Concern) -> Bytes fallible(CodecError) {
        return std::bytes::from_string("{\"id\":" + quoted(c.id) + ",\"source\":" + quoted(c.source)
                + ",\"what\":" + quoted(c.what) + ",\"severity\":" + to_string(c.severity) + "}");
    }
    fn decode(b: Bytes) -> Concern fallible(CodecError) {
        let t = std::str::from_bytes(b);
        return Concern { id: std::json::find_string_field(t, "id"), source: std::json::find_string_field(t, "source"),
                what: std::json::find_string_field(t, "what"), severity: std::json::find_int_field(t, "severity") };
    }
}

main locus Demo {
    params {
        nerves: nats::NatsConn = nats::NatsConn {
            url: std::env::var("HALE_DNA_NATS_URL_APP"),
            user: std::env::var("HALE_DNA_NATS_USER_APP"),
            credential: std::secret::Credential { vault: std::env::var("HALE_DNA_NATS_VAULT_APP") },
            subject_prefix: std::env::var("HALE_DNA_NATS_ORG") + ".",
            jetstream: true
        };
    }
    placement { nerves: pinned; }
    bus { publish ConcernRaised; }
    bindings { ConcernRaised: nats::NatsAdapter { } codec(ConcernJson { }); }
    run() {
        ConcernRaised <- Concern { id: "c-0001", source: "demo/checkout", what: "payment retries above 5%", severity: 2 };
    }
}

fn main() {
    Demo { };
}
```

The connection's `subject_prefix` is the organization's token and a
dot, so the event travels as `<org>.app.demo.concern.raised`, and
`jetstream: true` has the organization's stream acknowledge it.
`dna/tests/heart/heart.hl` is the smallest application that does
this with a usage event, and `dna/tests/heart_reading_test.hl` runs it
against a live organism.

The node's host reads every application's events through one durable,
`heart`, and lands each as a **`reading.recorded`** row, entity
`<app>/<event>/<id>`, before anything acts on it. The row's body keeps
`app`, `event`, `id`, `subject` and `payload` (the event's JSON). The
host says so as it goes:

```text
hale dna dev: reading <app>/<event>/<id> recorded
```

A reading is a signal, never a fact. It acts on nothing itself; what
the spine does with one is a row of its own. Today that is two
things: an event named `concern.raised` becomes a `concern.requested`
row ([Senses and reflexes](./senses.md#concerns)), and a reflex's
firing is read with the others for its target
([Reflexes](./senses.md#reflexes)). Every other reading is kept, and
nothing reacts to it.

The rules the host applies:

- **What is a reading.** The body is one JSON object as
  `std::json::valid_object` admits it (at most 64 top-level members,
  unique keys), at most 64 KiB, with a string `id` of letters, digits
  and `-_.:`, at most 128 bytes. The application's and the event's
  names follow the same rule. Anything else is not a reading: the host
  says why (`the heart's <subject> is not a reading: …`) and records
  nothing.
- **Duplicates.** The same entity arriving again, a redelivery or a
  replay, is refused and lands nothing: `reading <app>/<event>/<id> is
  recorded already; refused as a duplicate`. Two hosts pulling the one
  durable land it once.
- **Row first in delivery too.** The event is acknowledged to the
  stream only once its row has landed, or it was refused for good. A
  host that stops in between loses nothing: the event comes again,
  within the stream's week.
- **Shared records.** Over a shared record that keeps its rows in git,
  only the ledger can decide an id across owners, so readings wait in
  the stream until `hale dna ledger adopt` ([Memory and the
  record](./memory.md)); the host says so once.

### Its broker account

The application the record attaches has a broker account of its own,
named by its project: user `app-<name>`, its password the vault's
`nats-<org>-app-<name>`. It may publish on `<org>.app.<name>.>` and
nowhere else: no other application's subjects, nothing of DNA's, and it
reads nothing. `hale dna init` records `application.attached` (with the
`name`) and draws the account; `hale dna upgrade` draws it again if it
is missing. An application named `reflexes` holds no account, since
those subjects are [the reflexes'](./senses.md#reflexes).

The application is handed four variables and none of the organism's
other credentials: `HALE_DNA_NATS_URL_APP` (the server, `nats://host:port`,
no password), `HALE_DNA_NATS_USER_APP`, `HALE_DNA_NATS_VAULT_APP` (the
vault entry its `Credential` names) and `HALE_DNA_NATS_ORG`. `hale dna
dev` hands them to the application it starts, a node to each instance,
and `hale dna nerves migrate` prints them ([The nerves](./nerves.md)).

### Its API

What the organism does *to* the application goes the other way:
through the API the application exposes ([The API
surface](../services/api.md)), never through its events.

A leg is not handed that API. The hands in
`dna/core/legs/hands.hl` include a `HeartHand` and a `DeployHand`, and
the ones a performer gets, `NoHeart` and `NoDeploy`, refuse every call
and say so. The workflow catalog agrees: a definition with a step that
writes the heart is refused at admission, as `hale dna definitions`
prints:

```text
change-deliver@1  a change delivered: reviewed at the forge, applied, deployed, settled on the pulse
  …
  refused at admission: workflow change-deliver@1 step 4 (deploy) writes the heart (GH #987), which is not built yet
```

Today the body expresses a change, as the rest of this page describes.

### Removing it

```sh
hale dna application remove [project] [--as <who>]
```

This writes `application.detached {name, by}` to the record, which is
where membership lives, and revokes the account: the user leaves
`dna/nats.conf`, its password leaves `dna/nats.secrets.conf` and the
vault, and compose's `nerves` is restarted, so a connection it holds is
closed and none opens again.

```text
application remove: `<app>` detached (application.detached in the record)
nerves  app-<app> removed from dna/nats.conf
nerves  its password removed from dna/nats.secrets.conf
secret  nats-<org>-app-<app> removed from the vault
```

With a real vault (`HALE_VAULT_ADDR`) the last line says the entry is
revoked out of band instead.

### When the pulse stops

If the heart's pull on its durable is refused while the connection
holds (the durable, or its stream, is gone), the host records one
`violation.recorded` row, kind `pulse_stopped`, subject `heart`, for
the outage. The connection makes the durable again and the organism
goes on. See [closures and violations](./skin.md#closures-and-violations).

## The body

A body is a host running the organism for a record: the `dna/host`
program under `hale dna run` or `hale dna dev`, which is a node of the
spine. Its holder name is `user@host:<clone>`, so the same clone
restarting takes its lease straight back. A record admits one body at
a time (one per owner over a shared record). The lease itself, its
renewal and the fence that kills a body that lost it are
[the spine's](./spine.md); this is how you see and move it.

```sh
hale dna body                        # who runs this record
hale dna body claim --force          # take the lease from a body that is gone
hale dna body release [--force]      # give it up
```

`hale dna body` prints one line, `body: …`: `none (no body has run
this record; …)`, `live on <holder>, ticked …, lease expires in …s`,
`stale: …` once the lease has expired, or `released by <holder>`.

A second host started while a body is live stops before it builds
anything:

```text
hale dna run: a body for this record is live on <holder> (ticked …, lease expires in …s); a record admits one body — `hale dna body claim --force` takes it over when that body is gone
```

When that body really is gone, `body claim --force` releases its lease
as a row in your name (`body.claimed`, `forced: true`, `--as <who>` to
name someone else). That body stops the next time it asserts the
lease, and the next `hale dna run` takes it. `body release` gives up
this clone's lease (`body.released`); with `--force` it releases
another holder's.

`hale dna profile` prints the combination the organism is, detected
from its pieces and never stored:

```text
$ hale dna profile
profile:     local (detected)
record:      this clone only (no remote; `git remote add origin …` shares it)
body:        none (no body has run this record; `hale dna run` takes the lease)
head:        this clone (…); hosted head: none
fleet:       none (`[dna] fleet = "<name>"` in hale.toml names one)
memory:      compose (dna/compose.yaml, under `hale dna dev`)
trust:       local (every writer to the record is trusted)
github:      none (a bare remote: no reviews are opened, no verdicts read from it)
connections: none
```

### A body on a server

```sh
hale dna body provision <user@host> [--dsn <postgres://…>] [--dir <path>] [--dry-run]
hale dna body start|stop|logs [--body <user@host>]
```

`body provision` makes a body over ssh, in order: the toolchain
`hale.lock` pins, the record's remote cloned (into `$HOME/dna/<project>`
unless `--dir` names another place), `vendor/dna` and the record brought
up with `hale dna upgrade` and `hale dna sync`, memory from
`dna/compose.yaml` or the DSN you give, and a systemd user unit,
`hale-dna-<project>-<record>`, that runs `hale dna dev . --no-iris` with
`Restart=always`. On one server the body is the organization and the
application under one host. A DSN goes into the body's env file as
`HALE_DNA_MEMORY_DSN_OWNER`, never into the record.

It writes nothing when the record has no remote, or one local to this
machine; when `hale.lock` pins no toolchain; when ssh cannot reach the
host; or when the host lacks git, curl, systemd or (without `--dsn`)
docker compose. `--dry-run` prints the exact script and writes nothing.
Afterwards it records `dna.body` and `dna.body.dir` in this clone's git
config and a `body.provisioned` row, and says:

```text
body provision: <user@host>
    …
    dna.body = <user@host> (start/stop/logs go there); the body takes the lease when its unit starts
```

`body start`, `stop` and `logs` reach that unit (the last 200 lines for
`logs`), on `dna.body` or on the `--body` you name.

Credentials go where the body runs, with `hale dna secret set <NAME>
--body <user@host>` ([The skin](./skin.md#filling-a-slot)). A body
that starts with no model key in its vault says so at once: a
`body.credential_missing` row, a line on the board and on `status`'s
`body:` line, until a `body.credential_present` follows.

## Expression

The genome is what the organism is; the **expression** is the
application actually running. An approved change moves the genome, and
something then has to express it and watch it.

### `hale dna dev` and `hale dna run`

```sh
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
hale dna run [project] [--port N] [--no-iris]
```

Both start the one host. `--port` is iris's port (8787 by default) and
`--no-iris` starts none. The host refuses to start while the vault
lacks a secret the organism draws (`hale dna upgrade` draws it), then
takes the body lease, then builds and supervises.

- **`dev`** is the whole organism on this machine. It applies memory's
  schema and creates the nerves' stream as their owners (the servers
  `dna/compose.yaml` brings up, or `HALE_DNA_MEMORY_DSN_OWNER` and
  `HALE_DNA_NATS_URL_OWNER` when you name your own), brings up the
  senses' store when compose is where the servers come from, hands the
  host only the spine's DSN and URL, and runs the application under the
  same host.
- **`run`** is the organization alone. It migrates nothing: give it
  `HALE_DNA_MEMORY_DSN_SPINE`, `HALE_DNA_NATS_URL_SPINE` and
  `HALE_DNA_NATS_ORG` (the migrate verbs print them), and it strips the
  owner's and the head's DSN and URL before the host sees them. The
  application is expressed elsewhere: by a fleet, by a deployment
  gateway, or not at all.

[The spine](./spine.md) describes the host's tick and
[The nerves](./nerves.md) the relay; what follows is the part that
touches the application.

### Apply

When a Mutation's Review settles `approve`, the organization applies
**exactly the reviewed candidate** through its gateway, a fast-forward
or nothing. It refuses, touching nothing, when the candidate's worktree
moved after the review (`mutation.refused: candidate moved after
review`), when the genome moved since the review (`the genome moved
since the review`), or when the genome has uncommitted changes (`the
genome has uncommitted changes`). Clearing the last one and approving
again runs the whole gate again (`mutation.apply_retried`). [One task,
end to end](./workflow.md) walks the rows.

Then it records `mutation.applied` and expresses it, one of three ways.

**A deployment gateway.** With `deployment: dna::ShellDeployment {
command, seed }` in `dna/org/main.hl`, the organization runs `<command>
express <candidate> <seed>` in its own handler. Exit 0 means up and
healthy; anything else is the observation that rolls back, through
`<command> rollback <base> <seed>`. The record says
`expression.deployed`, and no host is asked.

**The host, under `dev`.** Without a gateway the organization appends
`expression.restart_requested`, naming the candidate, the seed the change
edited and the fitness signals the proposal declared. The host cuts a
fresh artifact of the application (`.hale/dna/current.topology`,
keeping the old one as `previous.topology`), rebuilds, stops the old
process (SIGTERM, then SIGKILL after five seconds), starts the new one,
and records `expression.restarted` in its own name:

```text
hale dna dev: <mutation> requests a restart (apply <candidate> seed <seed> fitness <signals>)
hale dna dev: expression restarted (pid <pid>) as <shape> build <digest>
```

A candidate that does not build is reported `build_failed`, and the
organization rolls back.

**A fleet, under `run`.** With `[dna] fleet` set, the host answers the
restart request with a deploy row instead ([below](#the-fleet)). With
neither a fleet nor a gateway, `run` says `no expression is under this
host` and expresses nothing.

A change to the organization's own seed (`dna/org`) is answered by the
host under both verbs: it rebuilds and restarts the organization, and
watches it.

### The observation window

The new expression must stay up for the window: `--observe <secs>`,
15 seconds by default. At its end the host writes its report as a row,
`observation.requested`, and relays it as `ExpressionObserved` until the
organization answers with `expression.observed`:

```text
hale dna dev: <mutation> observed healthy for 15s as <shape>
hale dna dev: <mutation>: the expression exited (<code>) inside the observation window
```

The organization judges only an applied Mutation, and records
`pressure.remeasured` against the fitness signals the proposal
declared. Then:

- **healthy**: `mutation.retained`, and the worktree is dissolved;
- **anything else**: the genome goes back to the Mutation's base
  (`git reset --keep`, through the gateway, `mutation.rolled_back`), and
  the old expression is asked back with a restart request marked
  `rollback`, which is answered without opening another window.

If the *organization* exits inside its own window, nobody is left to
decide, so the host does: it appends `expression.crashed` and
`mutation.rolled_back`, resets the genome to the base and restarts the
base organization.

A restart is a restart. The old process ends and the new one starts
from its constructors; state that must survive a change is carried the
way it would be across any deploy.

### The fleet

The organization can run on one machine while the application runs on
many. The fleet is the plan Hale already checks ([Multiple
binaries](../services/multi-binary.md)), with plan schema 1.2 letting an
instance name the `seed` it is built from and the `node` that runs it:

```json
{"schema": "1.2", "name": "production", "instances": [
  {"id": "api-0", "seed": "services/api", "node": "edge-1", "labels": ["api"]},
  {"id": "api-1", "seed": "services/api", "node": "edge-2", "labels": ["api"]},
  {"id": "worker-0", "seed": "services/worker", "node": "edge-1", "labels": ["worker"]}]}
```

```toml
[fleets]
production = "ops/production.plan.json"

[dna]
fleet = "production"
```

On each machine, a clone of the repository and one node:

```sh
hale node edge-1                  # [--repo <clone>] [--fleet <name>] [--tick <ms>]
```

A node syncs the record every tick and reads the latest `fleet.deploy`.
When that names a revision it does not express, it fetches and checks
it out, then builds and restarts each instance the plan assigns to it
that the deploy touches, and reports each in its own name (`node/<name>`):
`instance.up` with the revision, model hash, build digest and pid, and
`instance.exited` with the code when one goes. It decides nothing.
`.hale/node/<name>/` holds its pid files.

```text
hale node edge-1: expressing from … (record via origin)
hale node edge-1: instance api-0 up (pid <pid>) at <rev> as <shape>
hale node edge-1: fleet.deploy #<n> expressed (<k> instance(s) started)
```

A deploy is a row. The revision is pushed to
`refs/dna/revisions/<rev>` first, so every node can fetch it; the row
carries the plan, the revision, the seed the change edited, the
instances touched (every instance with a node whose seed is that
directory) and the reason. By hand:

```sh
hale dna deploy HEAD          # fleet.deploy #<n>: production at <rev> touching api-0 api-1 worker-0
hale dna rollback <mutation>  # the base that Mutation was applied on, again
hale dna fleet                # every instance, its node, up or exited, its revision and model hash
```

Under `hale dna run` with a fleet, an approval deploys the candidate to
the instances whose seed the change edited, then waits: every touched
instance must report `instance.up` at the revision (three minutes to
settle), and then none may exit for the observation window. All up and
quiet is `healthy`. One exiting is `expression.crashed` naming the
instance and its node, and the organization's rollback is another deploy
row touching the same instances. An instance that never came up is
`never expressed by …`, reported as `build_failed`.

## How it breaks

| symptom | cause | what to do |
| --- | --- | --- |
| `a body for this record is live on …` | another host holds the lease | stop it, or `hale dna body claim --force` if it is gone |
| `the organism's secrets are not all in the vault: …` | a drawn secret is missing | `hale dna upgrade` |
| `no expression is under this host` | `run` with no fleet and no gateway | use `dev`, set `[dna] fleet`, or wire a `ShellDeployment` |
| `the heart's … is not a reading: …` | the event is not one JSON object with a string `id` | fix the application's codec |
| `readings wait in the stream: …` | a shared record not yet on the ledger | `hale dna ledger adopt` |
| `body provision: … nothing was written` | no reachable remote, no pinned toolchain, or a missing tool on the host | what the line names |

[Troubleshooting](./troubleshooting.md) has the rest, part by part.
