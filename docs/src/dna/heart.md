# The Heart and the Body

In DNA, the **heart** is your supervised Hale application, and the **body** is the host supervisor (`dna/host` running under `hale dna dev` or `hale dna run`) that holds the active record lease in PostgreSQL and expresses approved code changes. The heart owns its own database and secrets and publishes application events on an isolated NATS subject namespace (`<org>.app.<name>.>`), while never holding credentials for DNA's internal database, vault slots, or control subjects.

```sh
hale dna dev                    # run the body and express the application locally
hale dna body                   # inspect the active body lease holder
hale dna deploy HEAD            # trigger a fleet deployment of the current commit
hale dna fleet                  # inspect instance health across all fleet nodes
```

## The Heart

### Its Events Become Readings

Your application integrates with DNA by publishing structured events over NATS JetStream. To publish an event, declare a topic with subject `app.<app>.<event>` (where `<app>` is your project name) and bind it to `vendor/dna/pond/realtime/nats` using a codec that serializes a single JSON object containing a string `id` field. Your application does not need to import `vendor/dna` itself—only pond's NATS client (`vendor/dna/pond/realtime/nats`):

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
        return Concern {
            id: std::json::find_string_field(t, "id"),
            source: std::json::find_string_field(t, "source"),
            what: std::json::find_string_field(t, "what"),
            severity: std::json::find_int_field(t, "severity") };
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
        ConcernRaised <- Concern {
            id: "c-0001",
            source: "demo/checkout",
            what: "payment retries above 5%",
            severity: 2,
        };
    }
}

fn main() {
    Demo { };
}
```

Setting `subject_prefix: std::env::var("HALE_DNA_NATS_ORG") + "."` and `jetstream: true` publishes the message to `<org>.app.demo.concern.raised` with JetStream acknowledgment. See `dna/tests/heart/heart.hl` and `dna/tests/heart_reading_test.hl` for complete test fixtures.

The node's host supervisor consumes `<org>.app.>` through the shared `heart` durable consumer and persists each valid event as a **`reading.recorded`** row (entity `<app>/<event>/<id>`, storing `app`, `event`, `id`, `subject`, and `payload`) before any handler acts on it:

```text
hale dna dev: reading <app>/<event>/<id> recorded
```

Two reading types trigger follow-up rows in the spine:
- **`concern.raised`**: Appends a `concern.requested` row routed to the source's parent ([Senses and Reflexes](./senses.md#concerns)).
- **`reflex.fired`**: Triggers an instance restart on the target fleet node and escalates repeated firings into a concern ([Reflexes](./senses.md#reflexes)).
- **All other readings**: Stored in the ledger for audit and history queries without triggering automated actions.

The host enforces four ingestion rules on every incoming application event:
1. **Payload validation**: The message body must be a single JSON object accepted by `std::json::valid_object` (at most 64 top-level keys, unique keys, max 64 KiB) containing a string `id` of 1–128 characters from `[A-Za-z0-9\-_.:]`. The `<app>` and `<event>` subject segments must obey the same character and length constraints. Invalid payloads are rejected with `the heart's <subject> is not a reading: …` and are not recorded.
2. **Idempotent deduplication**: If `<app>/<event>/<id>` already exists in the ledger, the duplicate message is rejected (`reading <app>/<event>/<id> is recorded already; refused as a duplicate`).
3. **Row-first acknowledgment**: The host acknowledges the JetStream message only after `reading.recorded` commits to PostgreSQL or is permanently rejected as invalid/duplicate. If the host crashes mid-flight, JetStream redelivers the message (up to the 7-day stream retention window).
4. **Cross-owner uniqueness**: On shared records, deduplication is enforced globally across owners by the PostgreSQL ledger ([Memory and the Record](./memory.md)).

### Its Broker Account

When `hale habitat graft` attaches an application (`application.attached`), it provisions a dedicated NATS user named `app-<name>` with its password stored in vault slot `nats-<org>-app-<name>` (`hale dna upgrade` regenerates it if missing). This account is permitted to publish only on `<org>.app.<name>.>` and cannot subscribe to or publish on any other application's subjects or `<org>.dna.>`. No application may be named `reflexes`, as `<org>.app.reflexes.>` is reserved for [the reflex daemon](./senses.md#reflexes).

When `hale dna dev` or a fleet node launches your application, it passes four NATS environment variables and strips all other DNA credentials:
- `HALE_DNA_NATS_URL_APP` (`nats://host:port`, without credentials)
- `HALE_DNA_NATS_USER_APP` (`app-<name>`)
- `HALE_DNA_NATS_VAULT_APP` (`nats-<org>-app-<name>`)
- `HALE_DNA_NATS_ORG` (`dna_<identity>`)

### Application API Surface & Workflow Restrictions

While application telemetry flows out to DNA over NATS, control operations from DNA to the application flow through the application's declared API surface ([The API Surface](../services/api.md)).

Worker legs are not permitted to call the application's API or trigger deployments directly: the `HeartHand` and `DeployHand` implementations provided to performers (`NoHeart` and `NoDeploy` in `dna/core/legs/hands.hl`) reject all calls. Likewise, workflow definitions containing steps that write to `heart` are rejected at admission (`hale dna definitions`):

```text
change-deliver@1  a change delivered: reviewed at the forge, applied, deployed, settled on the pulse
  …
  refused at admission: workflow change-deliver@1 step 4 (deploy) writes the heart (GH #987), which is not built yet
```

Instead, approved code changes are expressed by the host body as described in [Expression](#expression) below.

### Database Migrations (`[dna.migrations.database]`)

If your application uses its own database provisioned by the habitat, it can run schema migrations on startup or delegate migration execution to the habitat by declaring `[dna.migrations.database]` in `hale.toml`:

```toml
[dna.migrations.database]
command = ["api/api", "migrate", "--migrations", "store/migrations"]
```

When configured, `hale habitat graft`, `hale habitat migrate`, and `hale dna upgrade` execute this command with `HALE_DNA_DATABASE_DSN` set in the environment and record the exit status and output in the habitat's record. If the migration command exits non-zero, the habitat operation fails cleanly so the next run retries it.

### Detaching an Application (`hale dna application remove`)

To detach an application and revoke its NATS credentials, run:

```sh
hale dna application remove [project] [--as <who>]
```

This command appends `application.detached {name, by}` to the record, removes the application's user (`app-<app>-<key>`) from the organization's NATS account and its password from the account's secrets, deletes `nats-<org>-app-<app>` from the local vault (or instructs you to revoke it out of band when `HALE_VAULT_ADDR` is used), and restarts the nerves server holding the account to close any open connections:

```text
application remove: `<app>` detached (application.detached in the record)
nerves  app-<app> removed from the nerves allocation's configuration
nerves  its password removed from the nerves allocation's secrets
secret  nats-<org>-app-<app> removed from the vault
```

### Heart Consumer Outages (`pulse_stopped`)

If the host's pull request on the `heart` durable consumer is rejected while the NATS connection is active (for example, if the durable consumer or stream was deleted), the host logs a single `violation.recorded` row (`kind: "pulse_stopped"`, `subject: "heart"`) for the outage, recreates the durable consumer, and resumes pulling. See [Closures and Violations](./skin.md#closures-and-violations).

## The Body

A **body** is an active host supervisor process (`dna/host` started by `hale dna dev` or `hale dna run`) holding the record lease in PostgreSQL. Its holder identifier is `user@host:<clone>`. Each record admits one active body at a time (or one per owner on a shared record). See [The Spine](./spine.md#the-body-lease-and-the-fence) for lease renewal and fencing internals.

Use `hale dna body` to inspect, claim, or release the lease:

```sh
hale dna body                        # inspect current lease holder and TTL
hale dna body claim --force          # forcibly claim the lease from a stopped host
hale dna body release [--force]      # release this clone's (or another holder's) lease
```

If you start a second host while another checkout holds a live lease, the new process exits before compiling anything:

```text
hale dna run: a body for this record is live on <holder> (ticked …, lease expires in …s); a record admits one body — `hale dna body claim --force` takes it over when that body is gone
```

When the previous host is permanently gone, running `hale dna body claim --force [--as <who>]` writes `body.claimed` (`forced: true`), causing the old host (if still reachable) to stop on its next lease assertion and allowing a new `hale dna run` or `hale dna dev` to take over.

Run `hale dna profile` to inspect the detected deployment topology of your checkout:

```text
$ hale dna profile
profile:     local (detected)
record:      this clone only (no remote; `git remote add origin …` shares it)
body:        none (no body has run this record; `hale dna run` takes the lease)
head:        this clone (…); hosted head: none
fleet:       none (`[dna] fleet = "<name>"` in hale.toml names one)
memory:      the local habitat's allocation memory-<id> (compose project hale-habitat-local-<h>; `hale habitat migrate` brings it up)
trust:       local (every writer to the record is trusted)
github:      none (a bare remote: no reviews are opened, no verdicts read from it)
connections: none
```

### A Body on a Server

To provision and manage a remote host body over SSH, use `hale dna body provision`:

```sh
hale dna body provision <user@host> [--dsn <postgres://…>] [--dir <path>] [--dry-run]
hale dna body start|stop|logs [--body <user@host>]
```

`hale dna body provision` executes the following setup steps over SSH:
1. Installs the Hale toolchain version pinned in `hale.lock`.
2. Clones the repository's Git remote into `$HOME/dna/<project>` (or `--dir <path>`).
3. Runs `hale dna upgrade` to populate `vendor/dna` and draw internal vault secrets.
4. Configures PostgreSQL using either the remote machine's local habitat or `--dsn <postgres://…>` (written to the remote environment file as `HALE_DNA_MEMORY_DSN_OWNER`, never stored in the record).
5. Runs `hale habitat migrate .` once to initialize PostgreSQL and NATS.
6. Installs and starts a systemd user unit named `hale-dna-<project>-<record>` running `hale dna dev . --no-iris` with `Restart=always`.
7. Sets `dna.body` and `dna.body.dir` in your local Git config and appends a `body.provisioned` row to the record:

```text
body provision: <user@host>
    …
    dna.body = <user@host> (start/stop/logs go there); the body takes the lease when its unit starts
```

Pre-flight checks abort provisioning without writing anything if the repository has no non-local Git remote, `hale.lock` does not pin a toolchain version, SSH cannot connect, or the remote host lacks `git`, `curl`, `systemd`, or `docker compose` (when `--dsn` is omitted). Pass `--dry-run` to print the provisioning script without executing it.

Once provisioned:
- Control the remote systemd unit with `hale dna body start`, `hale dna body stop`, and `hale dna body logs` (prints the last 200 log lines).
- Populate remote vault slots over SSH stdin with `hale dna secret set <NAME> --body <user@host>` ([The Skin](./skin.md#filling-a-slot)). If a remote body starts without a model API key in its vault, it logs `body.credential_missing` until `body.credential_present` is recorded.

## Expression

While the **genome** is the committed source tree in Git, the **expression** is the running application process. When a Mutation is approved and fast-forwarded into the genome, the body expresses the change and monitors its health.

### `hale dna dev` vs. `hale dna run`

```sh
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
hale dna run [project] [--port N] [--no-iris]
```

| Command | Backing Services (`memory` & `nerves`) | Application Expression |
| --- | --- | --- |
| `hale dna dev` | Resolves `spine` credentials from the running habitat allocations (or `HALE_DNA_MEMORY_DSN_OWNER` / `HALE_DNA_NATS_URL_OWNER`). Does not start or migrate containers. | Compiles, runs, and restarts the application locally as a child process of the host. |
| `hale dna run` | Requires `HALE_DNA_MEMORY_DSN_SPINE`, `HALE_DNA_NATS_URL_SPINE`, and `HALE_DNA_NATS_ORG` in the environment; strips `owner` and `head` credentials. | Runs `dna/org` only. Expresses application changes via a configured `[dna] fleet` or `ShellDeployment` gateway. |

Both commands verify that all drawn secrets exist in the vault (`hale dna upgrade` populates missing drawn secrets), acquire the body lease, and attach the Iris inspector on `--port` (`8787` by default, disabled with `--no-iris`).

### Fast-Forward Apply & Expression Modes

When a Mutation Review settles with `approve`, `dna/org` verifies that the candidate worktree has not moved (`candidate moved after review`), the genome HEAD still matches the review base (`the genome moved since the review`), and the genome working tree is clean (`the genome has uncommitted changes`). If all checks pass, it fast-forwards the genome, records `mutation.applied`, and expresses the update using one of three mechanisms:

1. **Deployment gateway (`dna::ShellDeployment`)**: If `deployment: dna::ShellDeployment { command, seed }` is configured in `dna/org/main.hl`, `dna/org` executes `<command> express <candidate> <seed>`. An exit code of `0` marks the deployment healthy and records `expression.deployed`; any non-zero exit code triggers `<command> rollback <base> <seed>` and rolls the mutation back.
2. **Local process restart (`hale dna dev`)**: Without a `ShellDeployment`, `dna/org` appends `expression.restart_requested` naming the candidate SHA, edited seed, and fitness signals. The local `dev` host cuts `.hale/dna/current.topology` (archiving `.hale/dna/previous.topology`), compiles the new binary (reporting `build_failed` to trigger rollback if compilation fails), stops the old process (`SIGTERM` followed by `SIGKILL` after 5 seconds), starts the new binary, and records `expression.restarted`:

```text
hale dna dev: <mutation> requests a restart (apply <candidate> seed <seed> fitness <signals>)
hale dna dev: expression restarted (pid <pid>) as <shape> build <digest>
```

3. **Distributed fleet (`hale dna run` with `[dna] fleet`)**: The host translates `expression.restart_requested` into a `fleet.deploy` row consumed by `hale node` daemons ([The Fleet](#the-fleet)). If `hale dna run` has neither `[dna] fleet` nor `ShellDeployment`, it logs `no expression is under this host`.

> [!NOTE]
> Mutations that modify the organization's own code (`dna/org/`) are always expressed by the host supervisor under both `hale dna dev` and `hale dna run` by rebuilding, restarting, and observing `dna/org`.

### The Observation Window

After expressing a change, the host monitors the new process for the observation window (`--observe <secs>`, 15 seconds by default) and writes an `observation.requested` row relayed as `ExpressionObserved` until `dna/org` records `expression.observed`:

```text
hale dna dev: <mutation> observed healthy for 15s as <shape>
hale dna dev: <mutation>: the expression exited (<code>) inside the observation window
```

`dna/org` re-evaluates any fitness signals declared on the mutation (`pressure.remeasured`) and settles the deployment:
- **Healthy window**: Records `mutation.retained` and removes the sandbox worktree.
- **Process exit or failed check**: Resets the genome back to the mutation's base commit (`git reset --keep`, recording `mutation.rolled_back`) and emits a `rollback` restart request to restore the previous binary without opening a second observation window.
- **Organization self-crash**: If `dna/org` itself crashes inside its observation window after a self-update, the host supervisor directly appends `expression.crashed` and `mutation.rolled_back`, resets the genome to the base commit, and restarts the previous `dna/org` binary.

### The Fleet

To run `dna/org` on one control node while deploying application services across multiple servers, define a `fleet locus` ([Multiple Binaries](../services/multi-binary.md)) and reference it in `hale.toml`:

```hale
import "../../services/api" as api;
import "../../services/worker" as worker;

fleet locus Production {
    params {
        api: api::Api;
        worker: worker::Worker;
    }
    placement {
        api: host("edge-1"), host("edge-2");
        worker: host("edge-1");
    }
}
```

```toml
[fleets]
production = "ops/production"

[dna]
fleet = "production"
```

On each target server, clone the repository and start the node daemon for that host name:

```sh
hale node edge-1                  # [--repo <clone>] [--fleet <name>] [--tick <ms>]
```

On every tick, `hale node <name>` reads the latest `fleet.deploy` row from the record. When a deploy targets a revision not yet running on that host, the node fetches the revision branch (`dna/revision/<rev>`) from the forge, compiles and restarts every affected instance placed on `<name>`, writes PID files under `.hale/node/<name>/`, and appends status rows under author `node/<name>`:
- `instance.up`: Reports revision, topology hash, build digest, and OS PID.
- `instance.exited`: Reports exit code if an instance terminates.

```text
hale node edge-1: expressing from … (record via origin)
hale node edge-1: instance api-0 up (pid <pid>) at <rev> as <shape>
hale node edge-1: fleet.deploy #<n> expressed (<k> instance(s) started)
```

You can also trigger fleet deployments, rollbacks, and status checks manually:

```sh
hale dna deploy HEAD          # emits fleet.deploy touching all instances in the fleet
hale dna rollback <mutation>  # deploys the base commit of <mutation>
hale dna fleet                # lists every instance, host node, status, revision, and model hash
```

When an approved Mutation is expressed across a fleet under `hale dna run`, `fleet.deploy` targets only the instances whose seed directory was modified. Every targeted instance must report `instance.up` at the candidate revision within 3 minutes (otherwise `never expressed by …` triggers a `build_failed` rollback) and remain running with zero exits for the duration of the observation window (any exit logs `expression.crashed` and triggers a rollback `fleet.deploy`).

## Attaching External Codebases

An organism can also propose and verify changes in external repositories (including non-Hale repositories) that are not supervised as a live heart. Register an external repository with `hale dna codebase attach`:

```sh
hale dna codebase attach ledger-svc --repo https://forge.example/acme/ledger-svc.git \
    --build "make build" --test "make test" [--toolchain go] [--branch main] \
    [--knowledge docs/integrations]
hale dna codebase list
hale dna codebase detach ledger-svc
```

- **Record rows**: Writes `codebase.attached` or `codebase.detached` to the record without creating an `application.attached` row or NATS account.
- **`--knowledge <dir>`**: Directory on the codebase's default branch containing upstream architecture notes `<watch>.md` used by [Upstream Watches](./senses.md#a-move-becomes-a-task).
- **`--toolchain go` vs. `none`**: The default (`none`) runs only the opaque `--build` and `--test` commands inside the worker leg's worktree. Setting `--toolchain go` additionally equips the worker leg with `go_vet`, `go_test` (default `./...`), and `gofmt_check` (`gofmt -l`), and enforces `gofmt -l`, `go vet`, and `go test ./...` during verification before allowing a branch push.

To assign a task targeting an attached codebase, pass `--codebase <name>` and `--to position:<pos>`:

```sh
hale dna task create --to position:api/dev --codebase ledger-svc "make the greeting mention the leg"
```

When a leg claims the task (`hale dna work hand`, `hale dna work submit --from-worktree`), it clones the external repository on branch `leg/<codebase>/<task>`, runs the codebase's build and test commands, pushes the branch to the external forge, opens a pull request there, and submits `review:<ref>` as the attempt outcome without modifying the organism's own repository.

## Troubleshooting Common Errors

| Symptom | Cause | Resolution |
| --- | --- | --- |
| `a body for this record is live on …` | Another host currently holds the record lease | Stop the other host, or run `hale dna body claim --force` if it is no longer running |
| `the organism's secrets are not all in the vault: …` | One or more drawn secrets are missing from the vault | Run `hale dna upgrade` to regenerate missing drawn secrets |
| `no expression is under this host` | `hale dna run` started without `[dna] fleet` or `ShellDeployment` | Use `hale dna dev` for local execution, configure `[dna] fleet` in `hale.toml`, or configure `ShellDeployment` |
| `the heart's … is not a reading: …` | Application event payload is not a valid JSON object with a string `id` | Fix the application's NATS codec to emit a JSON object with a valid `id` string |
| `body provision: … nothing was written` | Missing Git remote, unpinned `hale.lock`, SSH failure, or missing remote tools | Resolve the prerequisite named in the error output and re-run `hale dna body provision` |

See [Troubleshooting](./troubleshooting.md) for a complete diagnostic reference.
