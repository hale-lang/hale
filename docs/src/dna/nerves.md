# The Nerves

The **nerves** are the organism's **NATS JetStream** broker (`nats:2`), running on the private-services tier with a dedicated durable stream per organization (`DNA_<ID>`) and per-component user accounts (`owner`, `spine`, `head`, `reflexes`, and `app-<name>`). The broker carries signals between the host, spine, head, reflexes, and supervised applications after state transitions have been committed to PostgreSQL. If the NATS stream fails to acknowledge a publish within its delivery window, the node's delivery closure triggers, records a `violation.recorded` row, releases the body lease, and exits with code `75` so its supervisor restarts it cleanly.

```sh
hale dna nerves migrate         # provision or update the NATS JetStream stream
hale dna nerves urls            # print per-role NATS URLs and vault slot names
```

## Row-First Delivery Model

User-facing CLI commands (`hale dna task create`, `hale dna review`, `hale dna concern raise`, `hale dna pressure raise`, `hale dna practice propose`, and `hale dna schedule declare`) never connect or publish to NATS directly. Instead, each command writes a request row to PostgreSQL (`record_rows` or the `ledger`) and returns or waits for an answer row.

Only a running spine **node** ([The Spine](./spine.md#a-node)) publishes to NATS. The node binds its `dna::` bus topics to pond's NATS adapter and, on every tick, relays any request rows in PostgreSQL that have not yet received an answering row. Because the database row is the source of truth, a dropped NATS message only delays a relay—it never loses a request or admits an action without a row.

## Broker Configuration & Accounts

In local development, the broker runs as a `nerves-<id>` container inside your machine's local habitat ([Memory and the Record](./memory.md#the-local-habitat)): a `nats:2` server bound to `127.0.0.1` on a host port in the `42000` range (stored in `ports.yaml`). An organism grafted with the habitat's defaults has no server of its own: its organization is an **account** on the habitat's NATS server. An organism grafted with `--own nerves` gets a server of its own, holding its one account. For multi-host deployments, run a NATS server with JetStream enabled, deploy the generated files, and pass `HALE_DNA_NATS_URL_OWNER` during migration.

Each organization a server holds is an account of its own (`ORG_<KEY>`, the record's first 12 hex digits), so nothing of one organization reaches another on a shared server. Its users are named by role and record key, and their passwords are variables of the account's secrets file (`<key>.secrets.conf`, written from the vault with mode `0600`), which the account includes first. The server's `nats.conf` includes every account in its `nerves.d/` directory and is rewritten whenever one comes or goes. For a project named `refproj` whose record key is `3f2a9c0b7d11`, the generated account reads:

```text
include ./3f2a9c0b7d11.secrets.conf
ORG_3F2A9C0B7D11: {
  jetstream: enabled
  users = [
      # creates the organization's stream; held by no process that runs it
      { user: owner-3f2a9c0b7d11, password: $NATS_3F2A9C0B7D11_OWNER_PASSWORD }
      # the spine: a node's host publishes the organization's facts, and the
      # organization reads them through its durable consumers
      { user: spine-3f2a9c0b7d11, password: $NATS_3F2A9C0B7D11_SPINE_PASSWORD,
        permissions: { publish: ["*.dna.>", "*.*.dna.>", "*.head.>", "*.*.head.>", "$JS.API.CONSUMER.CREATE.*.*", "$JS.API.CONSUMER.INFO.*.*", "$JS.API.CONSUMER.MSG.NEXT.*.*", "$JS.ACK.*.*.>"], subscribe: ["_INBOX.>"] } }
      # the application `refproj` the record attaches: publishes on its own
      # subjects alone, reads nothing of DNA's
      { user: app-refproj-3f2a9c0b7d11, password: $NATS_3F2A9C0B7D11_APP_REFPROJ_PASSWORD,
        permissions: { publish: ["*.app.refproj.>"], subscribe: ["_INBOX.>"] } }
      # the reflexes: publish their firings on their own subjects, read nothing
      { user: reflexes-3f2a9c0b7d11, password: $NATS_3F2A9C0B7D11_REFLEXES_PASSWORD,
        permissions: { publish: ["*.app.reflexes.>"], subscribe: ["_INBOX.>"] } }
      # the head: subscribes, publishes nothing
      { user: head-3f2a9c0b7d11, password: $NATS_3F2A9C0B7D11_HEAD_PASSWORD,
        permissions: { publish: { deny: [">"] }, subscribe: ["*.>"] } }
  ]
}
```

### NATS User Accounts

| NATS User | Held By | Vault Slot | Permitted Operations |
| --- | --- | --- | --- |
| `owner` | `hale habitat graft`, `hale habitat migrate`, `hale dna nerves migrate`, `hale dna nerves drop` | `nats-<org>-owner` | Create, update, or delete the JetStream stream; never held by a running daemon |
| `spine` | Node host supervisor and `dna/org` | `nats-<org>-spine` | Publish and consume `<org>.dna.>` via durable `spine`, consume `<org>.app.>` via durable `heart`, publish `<org>.head.>` notifications |
| `head` | Edge API / web face (`hale dna head`) | `nats-<org>-head` | Subscribe to `<org>.>`; forbidden from publishing |
| `reflexes` | Health daemon (`dna/reflexes`) | `nats-<org>-reflexes` | Publish on `<org>.app.reflexes.>` only; cannot read any stream |
| `app-<name>` | Attached application (`<name>`) | `nats-<org>-app-<name>` | Publish on `<org>.app.<name>.>` only; cannot read any DNA subjects |

`hale habitat graft` generates random passwords for all NATS roles in the vault, and `hale dna upgrade` rotates them and restarts the `nerves-<id>` container so the updated credentials take effect immediately. When a component connects, pond's NATS client reads the password directly from its vault slot when writing the `CONNECT` frame—passwords never appear in URLs, CLI arguments, or environment variables.

The `app-<name>` account is created when `application.attached` is recorded and revoked when you run `hale dna application remove` ([The Heart and the Body](./heart.md)). When `hale habitat graft <dir> --attach` is run on a repository root that uses Docker Compose services, it provisions `app-<project>` for those services. No application may be named `reflexes`, as `<org>.app.reflexes.>` is reserved for the reflex daemon.

### Supervising Repository Docker Compose Services

If your repository root contains a `compose.yaml`, `hale dna dev` and `hale dna run` automatically manage those services as a Docker Compose project named `dna_<id>-<project>` once `dna/org` connects to NATS. The host waits for the containers to report healthy, restarts any service that exits abnormally, and stops them on shutdown.

Compose receives only the application's NATS credentials (`app-<project>`) and never sees DNA's internal database or spine credentials:

```yaml
services:
  api:
    network_mode: host            # the broker where the host reaches it
    environment:
      HALE_DNA_NATS_URL_APP: ${HALE_DNA_NATS_URL_APP:-}
      HALE_DNA_NATS_USER_APP: ${HALE_DNA_NATS_USER_APP:-}
      HALE_DNA_NATS_ORG: ${HALE_DNA_NATS_ORG:-}
      HALE_VAULT_DIR: /run/secrets  # the secret below, a vault of one entry
      HALE_DNA_NATS_VAULT_APP: nats-app
    secrets: [nats-app]
secrets:
  nats-app:
    file: ${HALE_DNA_NATS_SECRET_APP:-/dev/null}
```

- **Secret mounting (`HALE_DNA_NATS_SECRET_APP`)**: Points to the mode `0600` vault file for `app-<project>`, which Docker Compose mounts read-only at `/run/secrets/nats-app`.
- **Host networking (`network_mode: host`)**: Allows Linux containers to reach the loopback-bound habitat broker at the same address the host uses.
- **Orphan cleanup via the body fence**: Each container is labeled with the active body's token and tracked in `.hale/dna/compose.owner.<token>_<pid>`. If the host crashes, the body fence removes only the containers belonging to that failed body epoch without disrupting a replacement body's containers.
- **Disabling Compose management**: Set `HALE_DNA_COMPOSE=off` to manage the repository's Compose stack manually. See `dna/tests/onboarding/todo` for a complete working example.

## Migrate and Drop

Run `hale dna nerves migrate` to create or update the organization's JetStream stream and print the environment variables for each component role:

```text
$ hale dna nerves migrate
HALE_DNA_NATS_ORG=dna_…
HALE_DNA_NATS_URL_SPINE=nats://…
HALE_DNA_NATS_URL_HEAD=nats://…
HALE_DNA_NATS_URL_REFLEXES=nats://…
HALE_DNA_NATS_URL_APP=nats://…
HALE_DNA_NATS_USER_APP=app-refproj
HALE_DNA_NATS_VAULT_APP=nats-dna_…-app-refproj
HALE_DNA_NATS_VAULT_REFLEXES=nats-dna_…-reflexes
```

- **`HALE_DNA_NATS_ORG`**: The organization token (`dna_<identity>`), matching the PostgreSQL schema name.
- **Automatic habitat startup**: If `HALE_DNA_NATS_URL_OWNER` is unset, `hale dna nerves migrate` starts the `nerves-<id>` allocation in the local habitat and configures the stream there. It rejects NATS servers running without JetStream (`the server does not run JetStream (start it with -js)`) or missing vault passwords (prompting you to run `hale dna upgrade`).
- **`hale dna nerves urls`**: Prints the same environment variables for a running broker without starting containers or modifying streams.
- **Host credential scoping**: `hale dna dev` resolves the `spine` URL and token from the running habitat allocation (if down, it exits and names `hale habitat migrate`). `hale dna run` requires `HALE_DNA_NATS_URL_SPINE` and `HALE_DNA_NATS_ORG` in its environment and strips any `owner` or `head` URLs; if unset, the host logs a warning and runs without NATS (`hale dna run: no nerves: …; the organization hears nothing the record asks`). Supervised application instances inherit only `HALE_DNA_NATS_URL_APP`, `HALE_DNA_NATS_USER_APP`, `HALE_DNA_NATS_VAULT_APP`, and `HALE_DNA_NATS_ORG`, with all DNA database DSNs and other NATS URLs stripped from their environment.
- **`hale dna nerves drop`**: Deletes the `DNA_<ID>` stream and all stored messages (`dropped DNA_…` or `absent DNA_…`). Requires `HALE_DNA_NATS_URL_OWNER` to be explicitly set and never defaults to the local habitat allocation.

## Subjects and the Stream

Every NATS subject is prefixed with the organization token (`<org>`):

| Subject Pattern | Traffic Carried |
| --- | --- |
| `<org>.dna.<family>.<event>` | Internal DNA workflow and governance events declared on `dna::` topics (e.g., `dna.intent.offered`) |
| `<org>.app.<app>.<event>` | Telemetry and concern events published by an attached application ([The Heart and the Body](./heart.md)) |
| `<org>.head.<event>` | Row-landed notifications (`head.row.landed`) sent from spine nodes to edge heads |

Pond's NATS adapter automatically prepends `<org>.` on publish and strips it on receive, allowing multiple organizations to share a single NATS server without colliding on topic names.

Each organization owns one file-backed JetStream stream named `DNA_<ID>` (the uppercased `<org>` token) capturing `<org>.>` with a 7-day retention window. Readers track their offset via two durable consumers:
- **`spine`**: Consumed by `dna/org`, filtered to `<org>.dna.>`. Events published while `dna/org` is restarting remain queued in the stream until it reconnects.
- **`heart`**: Consumed by the node's host supervisor, filtered to `<org>.app.>` with up to `64` unacknowledged messages in flight. Because there is one `heart` durable per organization, each application event is delivered to one host.

On startup, the host waits up to 20 seconds for `dna/org` to begin pulling from `spine`:

```text
hale dna run: the organization reads its facts from the nerves (DNA_…)
hale dna run: the organization did not come to read the nerves within 20s; the facts wait in the stream
```

## Relaying After the Row Lands

On every tick, a spine node checks PostgreSQL for unanswered request rows and publishes them to NATS. A request row remains eligible for relay every 30 seconds until a corresponding answering row is written to PostgreSQL:

| Unanswered Request Row | NATS Topic (Subject) | Answering Row Kind(s) |
| --- | --- | --- |
| `intent.requested` | `IntentOffered` (`dna.intent.offered`) | `intent.offered`, `intent.refused` |
| `review.verdict` | `ReviewVerdict` (`dna.review.verdict`) | `review.settled`, `review.refused`, `review.signed` |
| `concern.requested` | `ConcernRaised` (`dna.concern.raised`) | `concern.raised`, `concern.refused` |
| `pressure.requested` | `PressureRaised` (`dna.pressure.raised`) | `pressure.raised`, `pressure.refused` |
| `practice.requested` | `PracticeRequested` (`dna.practice.requested`) | `practice.proposed`, `practice.refused` |
| `hold.requested` | `HoldRequested` (`dna.hold.requested`) | `hold.proposed`, `hold.refused` |
| `position.requested` | `PositionOpenRequested` (`dna.position.requested`) | `position.proposed`, `position.refused` |
| `graph.requested` | `GraphReviewRequested` (`dna.graph.requested`) | `graph.proposed`, `graph.refused` |
| `schedule.requested` | `ScheduleRequested` (`dna.schedule.requested`) | `schedule.answered` |
| `knowledge.node.requested`, `.binding.`, `.edge.` | `KnowledgeNodeRequested`, `…BindingRequested`, `…EdgeRequested` | Matching `.proposed` or `.refused` row |
| `observation.requested` | `ExpressionObserved` (`dna.expression.observed`) | `expression.observed`, `observation.refused` |
| `attempt.outcome_requested` | `WorkSubmit` (`dna.work.submit`) | `attempt.outcome`, `attempt.outcome_refused` |
| `attempt.allowance_requested` | `WorkAllowanceAsk` (`dna.work.allowance`) | `attempt.allowance_granted`, `attempt.allowance_refused` |

Because multiple concerns or pressure signals from the same source share an entity path, each `concern.requested` and `pressure.requested` row carries a unique `request` ID matched by its answering row.

- **Signed trust enforcement (`dna.trust = signed`)**: Nodes verify the SSH signature on human request rows before relaying; unverified requests receive an immediate refusal row (`unverified writer`) and are never published to NATS.
- **Shared-record routing**: On a shared record, a node only relays `intent.requested` rows targeting positions owned by its organization.

## Idempotent Consumption & Delivery Auditing

### At-Least-Once Delivery & Deduplication by ID

Because JetStream and the 30-second relay loop deliver messages at least once, every consumer deduplicates by identifier:
- **Intents, verdicts, practices, concerns, and pressure**: Admitted once by request/intent ID. Duplicate deliveries return the existing execution or settled Review without writing duplicate rows, and threshold counters (such as `concern_threshold` and `appendage_threshold`) count unique request IDs rather than NATS redeliveries.
- **Worker leg outcomes and allowances**: Keyed by `(attempt_id, lease_token)` so each lease settles at most once.
- **Application events (`reading.recorded`)**: Persisted as `<app>/<event>/<id>` before any handler runs. Duplicate arrivals of the same entity ID are rejected as duplicates. The host acknowledges the JetStream message only after the `reading.recorded` row commits to PostgreSQL (or is permanently rejected as malformed/duplicate); unacknowledged messages are redelivered for up to the stream's 7-day retention window.

### Delivery Closure Auditing

Pond's NATS connection buffers outbound messages until acknowledged by JetStream and retransmits them across reconnects. If a publish remains unacknowledged past its delivery timeout, the connection's `delivery` closure fires and collapses to its owning process rather than silently dropping messages:

- **Host connection failure**: The host appends `violation.recorded` (`adapter_undeliverable`, subject `nerves`), stops `dna/org` and the supervised application, releases the body lease (`body.released`, `why: nerves`), and exits with code `75` so systemd restarts the node with a fresh connection.
- **Organization connection failure**: `dna/org` exits with code `75`, causing the supervising host to shut down and restart alongside it.
- **Heart consumer failure (`pulse_stopped`)**: If pulling from the `heart` durable fails while the NATS connection is otherwise live (for example, if the durable consumer or stream was deleted), the host logs one `violation.recorded` (`pulse_stopped`) row for the outage, recreates the durable consumer, and resumes pulling.

## Head Notifications, Shared Records, and Shutdown

- **Push notifications to heads (`head.row.landed`)**: Whenever a node observes new rows in PostgreSQL, it publishes up to `256` `head.row.landed` messages (`seq`, `kind`, `entity`) per tick on `<org>.head.row.landed`. Edge heads (`hale dna head`) subscribe as the `head` user and push updates to connected browsers without polling ([The Head and the Face](./head.md)).
- **Shared multi-owner streams**: When multiple organizations share a record, each owner's host sets `HALE_DNA_OWNER` (from `dna.owner` in Git config), publishes internal facts under `<org>.<owner>.dna.>`, and consumes them through a dedicated durable consumer named `spine_<owner>`. Application events remain on the single shared `heart` durable across the organization.
- **Graceful shutdown (`SIGTERM`)**: Sending `SIGTERM` drains the node and `dna/org`, releases the body lease, and exits cleanly within `LOTUS_DRAIN_GRACE_MS` (default `30000` ms / 30 seconds).
