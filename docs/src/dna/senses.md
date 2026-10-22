# Senses and Reflexes

The **senses** are a **Prometheus** metrics server (`senses-<id>` in the local habitat) that scrapes `/metrics` endpoints exposed by long-running DNA components (`spine`, `hale node`, and `head`). The **reflexes** (`dna/reflexes`) are an automated health daemon that queries Prometheus for crashed fleet instances (`dna_instance_up == 0`) and publishes restart triggers over NATS JetStream without human or LLM intervention. Alongside automated metrics, operators and applications can record explicit **pressure** signals, **concerns**, and scheduled **upstream watches** (`source-check`) that automatically open tasks and pull requests when external dependencies change.

```sh
hale dna senses up                      # start the habitat's Prometheus store and print its URL
hale dna pressure raise api "high p99"  # record a capacity/structural pressure signal
hale dna concern raise org/api "timeout" --severity 2
```

## Prometheus Metrics & Scrape Targets

Every long-running DNA process serves a Prometheus scrape endpoint (`std::metrics`, namespace `dna`, implemented in `dna/core/senses.hl`):

| Component | Default Port | Exported Metrics |
| --- | --- | --- |
| **Spine** (body host supervisor) | `9464` | `dna_pulse_seconds`, `dna_model_calls_total` |
| **Fleet Node** (`hale node`) | `9465` | `dna_pulse_seconds`, `dna_instance_up`, `dna_instance_since_seconds` |
| **Head** (`hale dna head`) | `9466` | `dna_pulse_seconds` |

Configure metrics binding via environment variables:
- `HALE_DNA_SENSES_PORT`: Overrides the default listen port (`0` disables the metrics server).
- `HALE_DNA_SENSES_HOST`: Overrides the bind address (defaults to all interfaces so the Dockerized Prometheus container can scrape the host over `host.docker.internal`).
- **Non-fatal port conflicts**: If the configured port is already in use, the component logs `the <part> cannot serve its readings on port <port>; it runs unread` and continues running normally.

Each time series carries a `part` label, and fleet node series include `node` and `instance`:

```text
dna_instance_up{part="node",node="edge-1",instance="api-0"} 0
dna_instance_since_seconds{part="node",node="edge-1",instance="api-0"} 1790558458
dna_pulse_seconds{part="node"} 1790558460
dna_model_calls_total{part="spine"} 2
```

- `dna_pulse_seconds`: Unix timestamp (seconds) of the component's latest tick.
- `dna_instance_up`: `1` while the supervised fleet instance is running; `0` after it exits.
- `dna_instance_since_seconds`: Unix timestamp when the instance entered its current state.
- `dna_model_calls_total`: Cumulative count of `model.called` ledger rows observed by the spine.

To prevent unbounded metric cardinality, series labels never include event IDs, task IDs, or Git revisions. Per-task model spend and receipts are correlated through PostgreSQL ledger rows (`model.called` -> attempt -> task).

## The Prometheus Store (`senses-<id>`)

In local development, Prometheus runs as the `senses-<id>` service inside your machine's local habitat ([Memory and the Record](./memory.md#the-local-habitat)), configured in `<habitat>/allocations/<id>/compose.yaml`:

```yaml
  senses-<id>:
    image: prom/prometheus:v3.5.0
    network_mode: bridge
    command: ["--config.file=/etc/prometheus/senses.yml", "--storage.tsdb.path=/prometheus", "--storage.tsdb.retention.time=7d"]
    extra_hosts:
      - "host.docker.internal:host-gateway"
    # …
```

The server retains metrics for 7 days, binds to `127.0.0.1` on a host port in the `39000` range (stored in `ports.yaml`), and scrapes targets defined in `<habitat>/allocations/<id>/senses.yml` with `honor_labels: true`:

```yaml
scrape_configs:
  - job_name: spine
    honor_labels: true
    static_configs:
      - targets: ["host.docker.internal:9464"]
  # …
```

Manage the Prometheus store with `hale habitat migrate` or `hale dna senses`:

```sh
hale dna senses up [dir]        # start senses-<id> and print HALE_DNA_SENSES_URL
hale dna senses url [dir]       # print HALE_DNA_SENSES_URL if running (starts nothing)
```

When running, both commands output:

```text
HALE_DNA_SENSES_URL=http://127.0.0.1:<port>
```

If the container is stopped, `hale dna senses url` and `hale dna dev` note that the store is down and point you to `hale habitat migrate` without blocking startup. Only `dna/reflexes` reads `HALE_DNA_SENSES_URL`.

## Pressure

A **pressure** signal records that a component or subsystem is hitting capacity or architectural limits:

```sh
hale dna pressure                       # list raised pressure signals, proposals, and re-measurements
hale dna pressure raise <source> <what…>
```

Running `hale dna pressure raise` (or submitting the pressure form on `hale dna ui`) writes a `pressure.requested` row with a unique `request` ID and returns. A spine node relays the request over NATS until `dna/org` records `pressure.raised`.

`dna/org` counts unique pressure raises per `<source>` in the record:
1. **Threshold proposal (`appendage_threshold` = 3)**: When a single `<source>` reaches 3 pressure signals, `dna/org` writes `appendage.proposed` (*an organ for `<what>` is proposed, not created*) and notifies the Board.
2. **Candidate generation (`initiative`)**: When `initiative` is enabled on `dna::Dna` (the default), `appendage.proposed` automatically triggers an `organization` Mutation on `dna/org/` (`appendage.candidate`), which is verified and routed to the human Board for Review. No organizational change takes effect without Board approval.
3. **Post-deployment re-measurement**: Whenever a Mutation is retained or rolled back, `dna/org` re-evaluates the motivating pressure against the proposal's declared fitness signals and records `pressure.remeasured` ([The Observation Window](./heart.md#the-observation-window)).

`hale dna pressure` displays the 10 most recent signals alongside all `appendage.proposed`, `appendage.candidate`, and `pressure.remeasured` rows:

```text
pressure: <n> signal(s) raised
  <seq>  <source>  <what> x<count>
  …
```

## Concerns

A **concern** is an operational signal raised by a child component (`org/trio/worker`) and routed to its parent locus (`org/trio`). Concerns enter the record in two ways, both producing a `concern.requested` row:

1. **From the CLI**: Run `hale dna concern raise <source> <what…> [--severity N]` (default severity `1`). Writes `concern.requested` directly to PostgreSQL.
2. **From the running application**: The application publishes a JSON event `{id, source, what, severity}` on `<org>.app.<app>.concern.raised` ([The Heart](./heart.md#its-events-become-readings)). The host records the reading `<app>/concern.raised/<id>`, and the spine appends `concern.requested` (keyed by the reading entity) before acknowledging the NATS message. The `source` field must be a `/`-separated path of `[A-Za-z0-9\-_.]` characters containing `<app>` as one of its segments; concerns claiming another application's namespace are rejected.

The spine deduplicates each request ID and admits it as a `concern-escalate` workflow execution, recording `concern.raised <source>` with the `occurrence` count, target `parent`, and workflow `task` ID (a different concern reusing an existing request ID is rejected with `concern.refused`).

### Escalation to Knowledge (`concern_threshold` = 3)

When a single `<source>` accumulates 3 concerns in the record (`concern_threshold`), the `concern-escalate` step automatically proposes a knowledge entry bound to `<parent>` and records `concern.proposed <source>` naming its digest. This opens a Board Review ([The Head and the Face](./head.md)); once ratified, the knowledge entry is automatically included in the context `hat` of future tasks targeting `<parent>` ([Shaping and Governing](./shaping.md)). If `<source>` has no parent segment (no `/` in its path), escalation fails at the threshold with `knowledge.refused concern:<source>`.

## Reflexes

The reflex daemon (`dna/reflexes`) is a standalone process on the private-services tier that holds only two credentials:
- `HALE_DNA_SENSES_URL`: Read-only HTTP URL of the Prometheus store.
- `HALE_DNA_NATS_URL_REFLEXES`, `HALE_DNA_NATS_VAULT_REFLEXES`, and `HALE_DNA_NATS_ORG`: The NATS `reflexes` account, permitted to publish exclusively on `<org>.app.reflexes.>` (`hale dna nerves urls` prints all three values).

`dna/reflexes` enforces a single built-in rule: **`instance.down`**. Every 5 seconds (`HALE_DNA_REFLEX_TICK_MS`), it queries Prometheus for any fleet instance reporting `dna_instance_up == 0`. For each outage, it publishes one event on `<org>.app.reflexes.reflex.fired`:

```json
{"id": "instance.down:<node>:<instance>:<since>", "rule": "instance.down", "node": "<node>", "target": "<instance>", "action": "restart", "since": 1790558458, "fired_at": 1790558463}
```

The organism handles the firing through three row-backed stages:

1. **Reading ingestion**: The host supervisor consumes the NATS event and writes `reading.recorded` (`reflexes/reflex.fired/<id>`), rejecting duplicate firings for the same outage ID.
2. **Target node restart**: On its next tick, the named `hale node <node>` reads any `reflex.fired` row targeting it that lacks a matching `reflex.acted` row, restarts that single instance at the latest deployed revision, and appends `reflex.acted` (`restarted`, `already up`, or the failure reason):

```text
hale node edge-1: reflex instance.down:edge-1:api-0:1790558458 (instance.down on api-0): restarted
```

3. **Concern escalation on repeat crashes**: When the host records a firing, it checks recent firings for the same `<target>`. If a second firing occurs within 10 minutes (`HALE_DNA_REFLEX_WINDOW_SECS`)—or if the node's restart attempt failed (`reflex-failed:<id>`)—the host raises a concern (`source: "reflexes/<target>"`, `request: "reflex:<id>"`) once per window so the spine escalates the recurring crash.

When you start the web face (`hale dna head` or `dna/face/start.sh`) attached to a project under local sign-in, it automatically builds, starts, and stops `dna/reflexes` using `hale dna senses url` and `hale dna nerves urls` (if Prometheus or NATS is down, it logs `face: no reflexes: …` naming `hale habitat migrate` and runs the face without reflexes). See `dna/tests/senses_reflex_test.hl` for the full integration test.

## Upstream Watches

An **upstream watch** monitors an external dependency (a changelog page, documentation page, RSS/Atom feed, or OpenAPI specification) on a recurring schedule and notifies the owning position when the upstream changes. Apply the `standard` pack to propose an upstream watch:

```sh
hale dna pack apply standard --set integration=pay-changelog --set position=payments/dev \
    --set source=https://pay.example/changelog --set kind=changelog --set every=1d
hale dna pack apply standard --set integration=pay-api --set position=payments/dev \
    --set source=integrations/pay.hl --set kind=openapi --set every=1d
```

Each command opens a Board Review proposing two items bound to `<position>`:
- The practice `standard/upstream-watched/<integration>`:

```text
The upstream pay-api is watched for its position: … source=integrations/pay.hl kind=openapi every=1d step=source-check
```

- The knowledge entry `standard/upstream-source/<integration>` naming the upstream source.

| Practice Parameter | Description |
| --- | --- |
| `source=` | For `changelog`, `docs`, or `feed`: an `https://`, `http://`, or `file://` URL. For `openapi`: the committed `.hl` surface file generated by `hale api import` ([Noticing When the Service Moves](../services/api.md#noticing-when-the-service-moves)). |
| `kind=` | `changelog`, `docs` (HTML/text page), `feed` (RSS or Atom), or `openapi`. |
| `every=` | Schedule cadence (e.g., `1h`, `1d`). |
| `step=` | `source-check` (the workflow definition executed on each scheduled occurrence). |

Once ratified by the Board, the practice registers a recurring schedule of `source-check` convened by `<position>` (visible in `hale dna schedule`; if `<position>` has no holder at ratification, schedule creation is refused until filled—see [Schedules](./schedules.md)). To modify a watch, amend its practice; to disable it, run `retire-practice`.

On each scheduled run, `source-check` normalizes the upstream content—stripping HTML markup/scripts/styles/comments for `changelog`/`docs`, extracting one line per entry for `feed`, or running `hale api drift --document` for `openapi`—and compares it against the previous occurrence's snapshot:
- **First occurrence**: Captures the baseline snapshot and writes no reading or concern.
- **Unchanged snapshot**: Writes nothing.
- **Changed snapshot**:
  1. Records a `reading.recorded` row (`sources/<integration>.changed/<task>`) containing the unified diff (and `drift` summary lines for `openapi`) backed by a receipt digest:

```text
drift Pay added create_refund
--- standard/upstream-watched/pay-api 3a71c0d2e9b4
+++ standard/upstream-watched/pay-api 92c69fb5be46
@@ -30,6 +30,15 @@
…
+    "/refunds": {
```

  2. Raises a single concern from `<position>/<integration>` routed to `<position>` referencing the reading and receipt digest.
- **Unreachable source**: If fetching fails or returns a non-`2xx` status, the occurrence fails. One or two consecutive failures are ignored as transient blips; the 3rd consecutive failure records `sources/<integration>.unreachable/<task>` without raising a concern.

### A Move Becomes a Task

When an upstream watch practice also specifies `codebase=<name>` (via `--set codebase=<name>` on `hale dna pack apply standard`, referencing an attached codebase from [Attaching External Codebases](./heart.md#attaching-external-codebases)), any detected upstream change automatically creates a task for `<position>`:

1. **Architecture note ingestion (`<watch>.md`)**: Store an architecture note explaining how your codebase uses the upstream service at `<knowledge-dir>/<watch>.md` on the codebase's default branch (`--knowledge` on `hale dna codebase attach`, defaulting to `docs/integrations/`). When the upstream moves, the organism reads `<watch>.md` and proposes it (if new or modified) as knowledge bound to `<position>`.
2. **Automatic task creation**: The organism files a task under `<position>` whose brief bundles the `sources/<integration>.changed/<task>` reading, the upstream diff, and `<watch>.md`.
3. **Worker leg execution**: A worker leg holding `<position>` claims the task, inspects the client code in a worktree of the attached codebase, and submits one of two outcomes (along with receipts for the note, diff, and every file read):
   - **A pull request** on the attached codebase's forge updating the client code to match the upstream change.
   - **A no-impact brief** (a response starting with `NO IMPACT:` explaining why no client code changes are needed), which opens an internal Board Review rather than a forge pull request.
4. **Settlement (`task.delivered` vs. `refused`)**:
   - If the pull request is merged on the forge (or the `NO IMPACT:` Review is approved), the task settles as **delivered** (`task.delivered`). All PR review comments—and any diff between the leg's commit and the final merged commit if teammates edited the PR branch—are saved as receipts on the verdict row.
   - If the pull request is closed without merging (or the `NO IMPACT:` Review is rejected), the task settles as **refused**.

### End-to-End Example (`sdlc_exit_test.hl`)

To set up automated upstream watching and PR generation on a Go client repository:

```sh
hale dna codebase attach client-go --repo https://forge.example/acme/client-go.git \
    --build "go build ./..." --test "go test ./..." --toolchain go
hale api import ./pay.yaml --as Pay --out integrations/pay.hl   # commit this file
hale dna pack apply standard --set integration=pay --set position=payments/dev \
    --set source=integrations/pay.hl --set kind=openapi --set every=1h \
    --set codebase=client-go --as ada
```

The test fixture `dna/tests/sdlc_exit_test.hl` (using the sample Go module in `dna/tests/sdlc_exit_repo`) exercises this entire loop end-to-end across both an OpenAPI upstream (producing a verified Go PR passing `gofmt -l`, `go vet`, and `go test`, followed by human branch edits and merge) and a changelog upstream (settled via a `NO IMPACT:` brief).

## Troubleshooting Common Errors

| Symptom | Cause | Resolution |
| --- | --- | --- |
| `the <part> cannot serve its readings on port …; it runs unread` | Another process is already bound to the metrics port | Set `HALE_DNA_SENSES_PORT` to a free port (or `0` to disable), or stop the conflicting process |
| `hale dna senses url: the organism's senses' store in the local habitat is not up …` | The `senses-<id>` habitat container is not running | Run `hale habitat migrate <organism>` or `hale dna senses up` |
| `reflexes: the store at … did not answer (…)` | Prometheus container is down or unreachable | Run `hale dna senses up` |
| `reflexes: the vault holds no credential …` | Missing `nats-<org>-reflexes` password in the vault | Run `hale dna upgrade` to regenerate missing drawn secrets |
| A concern never escalates into a knowledge proposal | The concern's `<source>` path has no parent segment (no `/`) | Format `<source>` as `<parent>/<child>` so the proposal has a parent locus to bind to |
| A `source-check` occurrence fails with `refused` | Invalid parameters in the `standard/upstream-watched/<name>` practice text | Amend the practice text to fix the invalid parameter named in the failure output |
| `schedule.refused standard/upstream-watched/<name>`, `nobody holds position:<p>` | Position `<p>` had no assigned holder when the watch practice was ratified | Assign a holder with `hale dna fill <p> <holder>`, then declare the schedule manually: `hale dna schedule declare <name> --every <cadence> --definition source-check --convener <p> --args '{"practice": "<digest>", "name": "<name>"}'` |
