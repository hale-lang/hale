# DNA Architecture & Runtime Overview

In Hale, an **organism** combines your application with a self-governing runtime control plane (`dna/org`), an append-only event log and operational state store in PostgreSQL (`memory`), a NATS JetStream event broker (`nerves`), a Prometheus metrics store (`senses`), and a credential boundary (`skin`). When you submit a request, the runtime plans the work, proposes code changes in a sandboxed Git worktree, verifies the candidate with the Hale toolchain, routes the diff and evidence for review, applies approved commits, and monitors the deployed process.

```sh
hale habitat up
hale habitat graft demo
hale dna dev demo
hale dna task create --as alice "document the Echo locus in main.hl"
```

## System Components

Every DNA component maps to a specific software service or source module in your repository and local habitat:

| Component | Infrastructure / Role | Source / Habitat Location | Chapter |
| --- | --- | --- | --- |
| **Habitat** | Local or hosted control plane (Docker Compose + systemd units) that provisions and supervises organisms | `${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local` | [The Habitat](../habitat.md) |
| **Genome** | Git repository containing application source, organization config (`dna/org/`), and vendored core (`vendor/dna`); nodes pull from the forge's default branch | Repository root; host Git wrapper in `dna/host/genome.hl` | [Shaping and Governing](./shaping.md) |
| **Record** | Append-only audit log (`record_rows` table in PostgreSQL schema `dna_<identity>`), storing one signed row per governance or mutation event | `dna/core/record.hl`, `dna/host/record.hl` | [Memory and the Record](./memory.md) |
| **Memory** | Operational PostgreSQL state store (schema `dna_<identity>`, with `owner`, `spine`, and `head` DB roles) holding the daily `ledger`, organizational `graph`, and sealed `receipt` evidence | `dna/core/memory_spine.hl`, `dna/host/memory.hl`; habitat service `memory-<id>` | [Memory and the Record](./memory.md) |
| **Nerves** | NATS JetStream broker (`DNA_<ID>` stream, one user per role) relaying durable events between host, spine, heart, and reflexes after rows land in PostgreSQL | `dna/core/nerves.hl`, `dna/host/nerves.hl`; habitat service `nerves-<id>` | [The Nerves](./nerves.md) |
| **Face** | Browser web UI served in front of the edge API for viewing state and submitting intent | `dna/face/` | [The Head and the Face](./head.md) |
| **Head** | Edge HTTP and Unix socket API handling sessions, principals, and command submissions answered with receipts; never executes workflows | `dna/api/` | [The Head and the Face](./head.md) |
| **Body** | Host supervisor holding the active record lease in PostgreSQL and expressing code changes (via local restart, `ShellDeployment`, or `hale node` fleet) | `dna/core/infrastructure.hl`, `dna/host/infra.hl` | [The Heart and the Body](./heart.md) |
| **Spine** | Node runtime (`hale dna dev` or `hale dna run`) that relays unanswered requests onto NATS, projects the record into the PostgreSQL graph, enforces budgets/leases, and executes workflows | `dna/host/host.hl` supervising `dna/org/main.hl` | [The Spine](./spine.md) |
| **Legs** | Stateless worker processes (`hale dna work`) that claim workflow attempts through the head, load a context `hat`, run tools, and submit outcomes | `dna/core/legs/`, performers in `dna/org/work.hl` | [Legs, Hands, and Voice](./legs.md) |
| **Hat** | Content-addressed context bundle assembled per unit of work | `dna/core/hat.hl` | [Legs, Hands, and Voice](./legs.md) |
| **Hands** | Sandboxed tool capabilities granted by a work's output contract: Git, Forge, Hale toolchain, and record reads | `dna/core/contracts.hl`, `dna/core/tools.hl`, `dna/core/legs/hands.hl` | [Legs, Hands, and Voice](./legs.md) |
| **Voice** | Model routing layer, catalog, provider adapters, and call recording tape | `dna/core/models.hl`, `dna/core/model_map.hl`, `dna/core/tape.hl`, `dna/org/models.hl` | [Legs, Hands, and Voice](./legs.md) |
| **Heart** | The supervised Hale application publishing `<org>.app.<name>.<event>` over NATS, persisted by the host as `reading.recorded` rows | Application seed; host ingestion in `dna/host/pulse.hl` | [The Heart and the Body](./heart.md) |
| **Senses** | Prometheus metrics store scraping `/metrics` from long-running DNA components | `dna/core/senses.hl`; habitat service `senses-<id>` | [Senses and Reflexes](./senses.md) |
| **Reflexes** | Automated health daemon querying Prometheus for `instance.down` (`dna_instance_up == 0`) and triggering instance restarts | `dna/reflexes/main.hl`, node handler in `dna/host/reflex.hl` | [Senses and Reflexes](./senses.md) |
| **Skin** | Security boundary: Vault secret slots (`~/.cache/hale/vault` or `HALE_VAULT_ADDR` + `std::secret::Credential`), DB/NATS role grants, OIDC authentication, and absorbed closure violations (`violation.recorded`) | `dna/host/secrets.hl`, `dna/core/principal.hl`, `dna/oidc/stub.hl` | [The Skin](./skin.md) |

Recurring workflow triggers and conveners are covered in [Schedules](./schedules.md).

## Documentation Map

- **[Getting Started](./getting-started.md)**: Provision an organism with `hale habitat graft`, populate its vault slots, start `hale dna dev`, ratify the initial Board reviews, and submit a task.
- **[One Task, End to End](./workflow.md)**: Trace a single task from terminal submission through sandbox editing, toolchain verification, review, fast-forward merge, observation window, and retention or rollback.
- **Component Guides**: Read the individual chapters in the table above for operational details on storage, NATS subjects, workflows, workers, deployment, metrics, and secrets.
- **[Shaping and Governing](./shaping.md)**: Configure the organism's purpose, practices, autonomy grants, and compile-time architectural laws (`dna/org/law.hl`).
- **[DNA, the Building Block](./dna.md)**: Reference for the internal `vendor/dna` library, the `dna::Dna` locus, and generated source files.
- **[Troubleshooting](./troubleshooting.md)** and **[CLI & Row Reference](./reference.md)**: Lookup tables for common errors, CLI flags, environment variables, and record row schemas.

## Core Execution Invariants

Every component in the DNA runtime enforces two persistence rules so that crashes and restarts never leave partial state or lost work:

1. **One store per workflow step**: Every workflow step declares a single target store (`record`, `forge`, `genome`, `heart`, `graph`, `nerves`, `memory`, `vault`, or `host`) and writes exclusively through that store's designated writer. The next step reads what the previous step made durable. Workflow definitions that name zero or multiple stores for a step are rejected before execution. Concurrent writes whose target state moved since being read are rejected and re-evaluated, avoiding distributed transactions across stores. See [The Spine](./spine.md).
2. **Row-first persistence**: Every state transition or user command writes a durable row to PostgreSQL before any live notification is sent. CLI commands write a request row and wait; a running node relays unanswered rows onto NATS JetStream until an answering row lands in the record. NATS messages carry row IDs, deliver at least once, and are deduplicated idempotently by ID. External application events from the `heart` are persisted as `reading.recorded` rows before any workflow or reflex acts on them. See [The Nerves](./nerves.md).

## Trust Tiers & Credential Isolation

Components are partitioned into eight trust tiers defined by which credentials they receive and which resources they cannot access:

| Tier | Component / Actor | Credentials & Access Granted | Access Explicitly Denied |
| --- | --- | --- | --- |
| **Devices** | User's web browser | Session cookie from the head | Private keys, Git access, shell access, NATS broker |
| **Identity & Forge** | OpenID provider; GitHub (`gh` via core `Forge` interface) | User identity assertions; genome/record Git pushes and pull request verdicts | Operational runtime credentials or databases |
| **The Head** | Edge API (`dna/api/`) and web UI (`dna/face/`) | User sessions; principal-to-position mappings; local record checkout; PostgreSQL `head` role (claim fencing, reads, ledger appends on behalf of users); subscribe-only NATS user | PostgreSQL `spine` role; NATS publish permissions |
| **The Body** | Spine runtime (`hale dna run` or `hale dna dev`) | PostgreSQL `spine` role; NATS `spine` user; active body lease | PostgreSQL and NATS `owner` and `head` credentials |
| **The Body's Legs** | Worker processes (`hale dna work`) | Lease on the claimed attempt; model API keys from the local machine's vault | Direct PostgreSQL role (all reads and writes go through the head API) |
| **The Heart** | Supervised Hale application | Application-owned database and secrets; NATS `app-<name>` user restricted to publishing on `<org>.app.<name>.>` | DNA record, DNA PostgreSQL schema, DNA vault entries, `<org>.dna.>` NATS subjects |
| **Private Services** | PostgreSQL (`memory`), NATS (`nerves`), Vault, Prometheus (`senses`), `dna/reflexes` | Loopback-only listeners (`127.0.0.1`); reflexes receive Prometheus read URL and NATS `reflexes` publish-only user | Direct network reachability from external user devices |
| **External APIs** | Hosted LLM providers | Single prompt and API key per HTTP request | Any repository files or state not explicitly included in the prompt |

> [!NOTE]
> The local vault is a single directory per OS user (`~/.cache/hale/vault` with mode `0600` files). Because any process running under that OS user could theoretically read the filesystem, component isolation on a single machine relies on passing each process only the vault entry names it is authorized to resolve. See [The Skin](./skin.md).

## Capabilities & Non-Goals

### What DNA Does

- **Turns requests into durable workflows**: Accepts natural-language tasks from the CLI (`hale dna task create`) or web UI (`face`) and executes them as tracked workflow runs in PostgreSQL.
- **Proposes code changes in a sandbox**: Opens an isolated Git worktree where the editor agent is restricted to four tools: `read`, `edit`, `format`, and `check`.
- **Verifies candidates with sealed receipts**: Runs base artifact cutting, `hale fmt`, `hale check`, `hale verify`, `hale test`, fleet composition checks, rollback rehearsal, and semantic diffing. Every step's output is stored as a content-addressed receipt in PostgreSQL.
- **Presents unified review evidence**: Displays the source diff, structural semantic diff, and verification receipts pinned to the exact candidate commit in the CLI, Iris, `hale dna ui`, and mirrored GitHub pull requests.
- **Delegates bounded reviews to the Leader**: Allows the Leader model to approve changes that fall strictly within the Board's `AutonomyBoundary` grant (`dna/org/main.hl`), logging its reasoning in the record, while escalating everything else to human Board members.
- **Expresses, observes, and rolls back changes**: Fast-forwards the repository to the approved commit, restarts or deploys the application, monitors health over an observation window, and automatically resets to the base commit if the process exits or fails fitness checks.
- **Governs its own organization code**: Applies changes to `dna/org/` through the same review and verification pipeline (always requiring human Board approval) and restarts the organization process upon application.
- **Routes human tasks**: Assigns human work items as tracked `case` tasks and waits across restarts until the assignee completes them with `hale dna task done`.
- **Resumes cleanly across restarts**: Reconstructs all active workflow executions, steps, and attempts from PostgreSQL under their original IDs.

### What DNA Refuses to Do

- **Never applies unreviewed code**: Every code change requires a recorded `approve` verdict from either a human Board member or the Leader operating within a Board-ratified grant.
- **Never grants the editor commit or deploy access**: Compile-time architectural claims in `dna/org/law.hl` forbid the editor from committing directly to the genome, opening arbitrary worktrees, applying mutations, or reading protected knowledge. Breaking these rules fails `hale check` with a witness path.
- **Never relies on structural diff alone for runtime behavior**: Semantic diffs verify topology and architectural claims, not expression-level handler logic; source diffs are always presented alongside semantic diffs. (See `dna/kill-test/WALKTHROUGH.md` for the empirical evaluation behind this requirement.)
- **Never expands the org chart autonomously**: Repeated `pressure` signals generate an `appendage.proposed` review item, but no new component joins the organization without human Board approval.
- **Never migrates in-memory process state**: Application restarts terminate the old OS process and launch the new binary from its constructors.
- **Never computes a single cross-seed semantic diff**: A review's semantic diff covers the edited seed; `fleet.deploy` rows track which fleet instances are affected.
- **Never auto-generates pressure from application metrics**: Pressure signals must be explicitly submitted via `hale dna pressure raise` or `hale dna ui`.
- **Never executes complex remediation in reflexes**: The `dna/reflexes` daemon implements a single rule (`instance.down` triggers an instance restart). It does not perform rollbacks or backoff policies.
- **Never accepts write commands on `hale dna ui`**: `hale dna ui` is a read-only record viewer (except for the pressure signal form); interactive commands go through `hale dna head` and `face`.
- **Never admits workflows that write to `heart`, `vault`, or `host`**: Workflow definitions with steps targeting those stores—and leg `HeartHand` or `DeployHand` calls—are rejected at admission (`hale dna definitions` lists the refusal reason).
- **Never reads model API keys from shell environment variables**: Model keys must be stored in vault slots (`model-<NAME>`) via `hale dna secret set`.
- **Never runs without Git**: Candidate branches, base commits, and rollbacks rely on a Git repository.
- **Never guarantees exactly-once external side effects across crashes**: Record state transitions happen once, but external adapter calls interrupted by a crash require either an idempotent adapter or manual resolution via `hale dna effect resolve`.
- **Never auto-closes human cases**: Tasks handed to a person remain open until that assignee runs `hale dna task done` (or the person is retired to a successor via `hale dna retire`).
- **Never exposes CLI workflow cancellation**: While the workflow engine supports `WorkflowCancelRequested` internally, no `hale dna` CLI command issues cancellations.

The formal specification for the DNA runtime is maintained in `spec/dna.md`.
