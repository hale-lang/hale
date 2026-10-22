# Part V: DNA (Autonomous & Governed Applications)

Part V covers **DNA** (`vendor/dna` and `hale dna`), Hale's framework and runtime for building **self-governing, auditable software systems** where human operators and AI agents collaborate under compiler-enforced rules.

## Infrastructure Architecture Behind the Metaphor

DNA uses anatomical names for its components. Each component maps directly to a concrete piece of systems infrastructure:

| DNA Term | Concrete Software Component | Role |
|---|---|---|
| **Genome** (`dna/org/`) | Git repository + `.hl` source code | Declares the application, workflow steps, roles, models, and constitutional laws. |
| **Memory & Record** | **PostgreSQL** (`dna_<id>` schema) + **Git refs** (`refs/dna/*`) | Append-only event ledger, operational state projections, and Ed25519-signed evidence receipts. |
| **Nerves** | **NATS JetStream** (`DNA_<ID>` stream) | Durable pub/sub event bus connecting the coordinator, application, workers, and health monitors. |
| **Spine** | Workflow engine (`dna/org` under `hale dna run` / `dev`) | Evaluates task state machines, enforces LLM/compute budgets, and manages task leases. |
| **Heart & Body** | Supervised application (`heart`) + host supervisor (`body`) | Runs the target application, streams app events over NATS, and deploys verified code changes. |
| **Senses & Reflexes** | **Prometheus** (`senses`) + recovery daemon (`dna/reflexes`) | Scrapes metrics, triggers automatic `instance.down` restarts, and runs scheduled upstream watches. |
| **Skin** | **Vault** / `~/.cache/hale/vault` + OIDC + violation handler | Confines secrets in `@sealed` loci, authenticates operators, and records runtime closure violations. |
| **Legs, Hands & Voice** | Worker processes (`hale dna work`), sandboxed tools, LLM router | Executes tasks in isolated Git worktrees using scoped tools (`GitHand`, `GhForge`, `go_test`) and LLMs. |
| **Head & Face** | Edge RPC API (`dna/api`) + Web UI (`:8792`) | Serves the operator CLI/web dashboard, Board decision queue, 3-view code reviews, and GitHub PR sync. |

## Chapters in Part V

| Section | Chapters |
|---|---|
| **Overview & Walkthrough** | [Architecture of a DNA System](../dna/index.md) · [DNA Quickstart](../dna/getting-started.md) · [End-to-End Task Lifecycle](../dna/workflow.md) |
| **Core Runtime Subsystems** | [Record & State Store (PostgreSQL & Git)](../dna/memory.md) · [Workflow Coordinator (`spine`)](../dna/spine.md) · [Event Broker (`nerves` / NATS JetStream)](../dna/nerves.md) · [Application Supervisor & Deployment (`heart` & `body`)](../dna/heart.md) · [Telemetry, Health Reflexes & Upstream Watches (`senses`)](../dna/senses.md) · [Secrets, Vault & Violation Absorption (`skin`)](../dna/skin.md) |
| **Workers, UI & Governance** | [Worker Agents, Sandboxed Tools & LLM Routing (`legs`)](../dna/legs.md) · [Recurring Schedules](../dna/schedules.md) · [Edge API, Web UI, Board & Reviews (`head` & `face`)](../dna/head.md) · [Governance, Positions, Grants & Packs](../dna/shaping.md) |
| **Customization & Reference** | [The `dna::Dna` Locus & Custom Topologies](../dna/dna.md) · [DNA Engineering Standard](../dna/standard.md) · [DNA Troubleshooting](../dna/troubleshooting.md) · [DNA CLI & Schema Reference](../dna/reference.md) |
