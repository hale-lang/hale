# The `dna::Dna` Locus and Organization Architecture

Every DNA organism is assembled as a standard Hale program in `dna/org/main.hl` that instantiates the `dna::Dna` locus from `vendor/dna`. Because the entire organization—its positions, autonomy grants, verification pipeline, spend budget, and NATS JetStream (`nerves`) bindings—is expressed as locus construction in Hale source code, `hale check` statically verifies the organization's topology and constitutional claims (`dna/org/law.hl`) before any process starts.

```hale,fragment
import "vendor/dna" as dna;
import "vendor/dna/pond/realtime/nats" as nats;

main locus Org {
    params {
        core: dna::Dna = dna::Dna {
            journal: dna::RoutedJournal { record: dna::RecordJournal { repo: "." } },
            work: dna::WorkSystem { agent: dna::LegRelay { name: "legs" }, agent_reconciler: dna::RelayReplay { } },
            budget: dna::Budget { policy: org_budget() },
            boundary: dna::AutonomyBoundary {
                child: "refproj",
                grant: dna::Grant { child: "refproj", classes: "refactor docs", max_magnitude: 4, review: "pre" }
            },
            review_policy: dna::OrgPolicy { },
            board: dna::Board { who: "board" },
            gateway: dna::MutationGateway {
                leases: dna::MemoryLeases { repo: "." },
                workspaces: dna::IsolatedWorktrees { repo: ".", root: ".hale/dna/worktrees" },
                repo: dna::LocalGit { repo: "." }
            },
            verification: dna::HaleVerification { receipts: dna::MemoryReceipts { repo: "." }, scratch: ".hale/dna/scratch", repo: ".", seed: "." },
            editor: dna::SourceEditor { name: "editor", models: editor_models() },
            genome_seed: ".",
            catalog: workflows(),
            planned: true
        };
```

---

## Where `vendor/dna` Comes From

The `hale` compiler binary embeds the complete DNA runtime source. Running `hale habitat graft` or `hale dna upgrade` materializes the user-imported packages into `vendor/dna/` inside your project (git-ignored and regenerated automatically on a fresh clone when a body starts):

| Path in `vendor/dna/` | Contents | Import Statement |
| --- | --- | --- |
| `*.hl` | Core DNA substrate: `dna::Dna` and all organizational loci and interfaces | `import "vendor/dna" as dna;` |
| `pond/` | Postgres driver (`pq`) and NATS JetStream client and bus adapter | `import "vendor/dna/pond/realtime/nats" as nats;` |
| `legs/` | Worker CLI verbs, performers (`ModelPerformer`, `Person`, `FixedAnswer`), and `Hands` | `import "vendor/dna/legs" as legs;` |
| `README.md` | Toolchain version and embedded SHA-256 source digest | — |

### Provenance Digests and Host Binaries

Because two builds of the same version number during development can embed different DNA source trees, DNA tracks provenance by the SHA-256 digest of the embedded source rather than the version string alone:

```sh
hale dna --embedded-digest                      # print the 64-hex-digit digest embedded in this binary
hale dna --embedded-digest --from-tree <dir>    # compute the digest that a checkout at <dir> would embed
```

- `hale --version` prints the first 16 hex digits (`embedded dna: ca4012a0cda7226d`).
- Materialization writes the full 64-character digest to `vendor/dna/README.md` and `.hale/dna/embedded.digest`, and pins `[dna] toolchain` in `hale.lock`.
- `hale dna status` compares the running binary's digest against `.hale/dna/embedded.digest`. If they differ, `status` prints `` ; vendor/dna was materialized from <digest> — run `hale dna upgrade` `` ([Keeping the toolchain's part current](./shaping.md#keeping-the-toolchains-part-current)).

In addition to `vendor/dna/`, the `hale` binary embeds the sources for the host daemon (`dna/host`, executed by `hale dna dev`, `hale dna run`, and `hale node`), the one-shot CLI verb runner (`dna/host/verb`, executed by `status`, `review`, `apply`, `fill`, etc.), the head API (`dna/api`), and the browser face (`dna/face`). These programs are compiled once into the toolchain cache on first use and executed directly in their own processes.

---

## The `dna::Dna` locus

In `dna/org/main.hl`, the `main locus Org` declares three parameters:
1. `core: dna::Dna` — the organizational substrate.
2. `leader: dna::Leader` — the architect/planner position that plans tasks and decides Reviews within its grant.
3. `nerves` — the pinned NATS JetStream connection that bridges bus topics to the organization's stream.

### Required and Generated `dna::Dna` Parameters

| Field | Default Generated Value | Role in the Substrate |
| --- | --- | --- |
| `journal` | `dna::RoutedJournal { record: dna::RecordJournal { repo: "." } }` | Unified reader/writer over `record_rows` and `ledger_rows` in Postgres memory ([Memory and the record](./memory.md)). |
| `work` | `dna::WorkSystem { agent: dna::LegRelay { name: "legs" }, agent_reconciler: dna::RelayReplay { } }` | Dispatches admitted attempts: relays `agent` work to external legs (`hale dna work`) and reconciles leases on restart ([Legs, hands and voice](./legs.md)). |
| `budget` | `dna::Budget { policy: org_budget() }` | Enforces the organization-wide model spend policy defined in `dna/org/models.hl` ([The spine](./spine.md)). |
| `boundary` | `dna::AutonomyBoundary` wrapping `dna::Grant` | Defines which change classes and semantic magnitudes the Leader may approve without escalating to the Board ([Grant and autonomy](./shaping.md#grant-and-autonomy)). |
| `review_policy` | `dna::OrgPolicy { }` | Maps candidate dispositions and hard boundaries to required authority ranks ([Who is asked](./shaping.md#who-is-asked)). |
| `board` | `dna::Board { who: "board" }` | Human governance authority where intents enter and escalations surface. |
| `gateway` | `dna::MutationGateway` (`MemoryLeases`, `IsolatedWorktrees`, `LocalGit`) | Manages mutation leases, isolated scratch worktrees (`.hale/dna/worktrees`), and Git commits/rollbacks. |
| `verification` | `dna::HaleVerification` (`MemoryReceipts`) | Runs `fmt`, `check`, `verify`, `test`, `fleet`, and `model diff` on candidates and stores output receipts by digest. |
| `editor` | `dna::SourceEditor { name: "editor", models: editor_models() }` | Built-in editing position that reads, edits, formats, and checks code inside a scratch worktree. |
| `genome_seed` | `"."` | Root seed directory targeted by non-`organization` code changes. |
| `catalog` | `workflows()` | Workflow definitions admitted by the spine (`dna/org/workflows.hl` + `dna/org/own_workflows.hl`). |
| `planned` | `true` | Routes incoming intents through the Leader for classification and decomposition before creating mutations. |

### Optional `dna::Dna` Parameters

| Field | Default | When to Override |
| --- | --- | --- |
| `deployment` | `dna::NoDeployment { }` | By default, applying a candidate writes `expression.restart_requested` for the host to rebuild and restart. Set `dna::ShellDeployment { command, seed }` to run a custom deploy/rollback shell command instead ([The heart and the body](./heart.md)). |
| `ceiling` | None | Sets an upper `dna::AutonomyBoundary` that clamps the Leader's `boundary.grant` on every evaluation. |
| `initiative` | `true` | When a source raises pressure 3 times (`appendage_threshold`), automatically proposes an `organization` mutation adding a new `dna::Leader`. Set `false` to record `appendage.proposed` without generating a candidate ([How the organization grows](./shaping.md#how-the-organization-grows)). |
| `org_seed` | `"dna/org"` | Seed directory targeted by `organization`-class changes. |

### Leader Wiring, Nerves Bindings, and Wall-Clock Ticks

Alongside `core`, `dna/org/main.hl` wires the `leader` locus and binds incoming bus topics to `nats::NatsAdapter`:

```hale,fragment
        leader: dna::Leader = dna::Leader {
            name: "leader",
            models: leader_models(),
            receipts: dna::MemoryReceipts { repo: "." },
            source: dna::SourceReader { repo: "." },
            // …
            charter: charter(),
            purpose: purpose()
        };
```

```hale,fragment
    claims { adopt Org; }
    placement { nerves: pinned; }
    // …
    bindings {
        dna::ReviewVerdict: nats::NatsAdapter { };
        dna::IntentOffered: nats::NatsAdapter { };
        dna::ExpressionObserved: nats::NatsAdapter { };
        dna::PressureRaised: nats::NatsAdapter { };
        dna::ConcernRaised: nats::NatsAdapter { };
        dna::PracticeRequested: nats::NatsAdapter { };
        dna::HoldRequested: nats::NatsAdapter { };
        dna::PositionOpenRequested: nats::NatsAdapter { };
        dna::KnowledgeNodeRequested: nats::NatsAdapter { };
        dna::KnowledgeBindingRequested: nats::NatsAdapter { };
        dna::KnowledgeEdgeRequested: nats::NatsAdapter { };
        dna::WorkSubmit: nats::NatsAdapter { };
        dna::WorkAllowanceAsk: nats::NatsAdapter { };
    }
```

At the end of `Org.run()`, the main loop calls `self.core.request_tick(now_ms)` on the wall clock to drive scheduled workflows ([Schedules](./schedules.md)).

---

## Internal Record Families Inside `dna::Dna`

Within `dna/core/`, each domain of the substrate is factored into its own file (`org.hl`, `editing.hl`, `workspace.hl`, `verification.hl`, `memory_record.hl`, `review.hl`, `budget.hl`, `work_system.hl`, `workflow_definition.hl`, and `assembly.hl`).

State projections and row emissions inside `dna::Dna` are partitioned into **record families**—child loci initialized automatically during `dna::Dna.birth()` and bound to `self.journal`:

```hale,fragment
        self.evidence.bind(self.journal, self.verification.receipts);
        self.org_source.bind(self.journal);
        self.practices.bind(self.journal);
        self.concerns.bind(self.journal);
        self.spending.bind(self.journal);
        self.positions.bind(self.journal, self.verification.receipts);
        self.cases.bind(self.journal);
        self.attempts.bind(self.journal);
        self.organization_source_review_facts.bind(self.org_source.facts);
        self.scheduler.bind(self.journal);
        self.knowledge_record.bind(self.journal, self.verification.receipts);
```

| Family Field | Locus | Source File | Responsibility |
| --- | --- | --- | --- |
| `evidence` | `EvidenceStore` | `dna/core/evidence.hl` | Evidence classification, retention holds, redaction, model prompt/context receipts, and sealed protected bodies. |
| `scheduler` | `Scheduler` | `dna/core/scheduler.hl` | Declared schedules, record replay watermark, and occurrence evaluation on each tick. |
| `knowledge_record` | `KnowledgeRecord` | `dna/core/knowledge_record.hl` | Knowledge proposals, Board ratifications, retirements, and graph node/binding/edge mutations. |
| `org_source` | `OrganizationSourceRecord` | `dna/core/org_source.hl` | Staged mutations to `dna/org/`, exact verdict tracking, and refusal handling. |
| `practices` | `PracticeRecord` | `dna/core/practices.hl` | Practice requests, lifecycle rows, and `practice-ratify` executions. |
| `concerns` | `ConcernRecord` | `dna/core/concerns.hl` | Concerns, pressure signals, and escalation thresholds (`concern_threshold = 3`, `appendage_threshold = 3`). |
| `spending` | `SpendRecord` | `dna/core/spending.hl` | Model call cost evidence, budget gate evaluation, and monetary grant reservations/settlements. |
| `positions` | `PositionRecord` | `dna/core/positions.hl` | Position holder proposals and organization membership (`hale dna fill`). |
| `steps` | `StepRecord` | `dna/core/steps.hl` | Built-in workflow catalog steps awaiting a Review verdict, Leader response, or memory projection. |
| `cases` | `CaseRecord` | `dna/core/cases.hl` | Human-assigned Task leaf cases (admitted, held, refused, completed). |
| `attempts` | `AttemptRecord` | `dna/core/attempts.hl` | Attempt leases, pending leg outcomes, and code edits awaiting Review. |

---

## Files and Record Rows Created by `hale habitat graft`

- **New application (`hale habitat graft <name>`):** Scaffolds a minimal Hale application (`main.hl` and `tests/main_test.hl`) in an empty directory and grafts it as a child of the local habitat.
- **Existing application or repository (`hale habitat graft <dir> --attach`):** Attaches to an existing Hale seed (locating its `main locus`) or multi-service repository without modifying any application source files. Re-running `hale habitat graft` preserves existing files (`kept … (already exists)`) and never reseeds an existing record.

### Generated Project Files

| Path | Written By | Owner | Purpose |
| --- | --- | --- | --- |
| `main.hl`, `tests/main_test.hl` | `graft` (new app only) | Project | Starter application (`Ping` topic, `Echo` locus, main locus) and test. |
| `hale.toml` | `graft` | Project | Package manifest with `[claims] no_base = true`, `[environments.local]`, `[environments.org]`, and `[dna] record`. |
| `hale.lock` | `graft`, `upgrade` | Toolchain | Pins `[dna] toolchain = "<version>"`. |
| `.gitignore` | `graft` | Project | Ignores build artifacts, `/vendor/`, and `/.hale/`. |
| `vendor/dna/` | `graft`, `upgrade` | Toolchain | Materialized DNA core, `pond/`, `legs/`, and `README.md`. |
| `.hale/dna/embedded.digest` | `graft`, `upgrade` | Toolchain | Full 64-hex-digit digest of the materialized DNA source. |
| `.hale/dna/baseline.topology` | `graft` | Toolchain | Initial compiler topology dump (`hale check --dump-topology`) used to seed the record. |
| `dna/org/main.hl` | `graft` | Project | Organization entrypoint locus (`Org`). |
| `dna/org/law.hl` | `graft` | Project | Organization constitution (`constitution Org`). |
| `dna/org/purpose.hl` | `graft` | Project | Declared project purpose (`PURPOSE`). |
| `dna/org/charter.hl` | `graft` | Project | Leader architectural brief (`CHARTER`). |
| `dna/org/models.hl` | `graft` | Project | Model backend catalog and `org_budget()`. |
| `dna/org/work.hl` | `graft` | Project | Leg performer catalog (`performers()`). |
| `dna/org/workflows.hl` | `graft`, `upgrade` | Toolchain | Baseline workflow catalog (regenerated on `upgrade`). |
| `dna/org/own_workflows.hl` | `graft` | Project | Custom project workflow definitions (never overwritten). |

> [!NOTE]
> Infrastructure configuration files (`compose.yaml`, `ports.yaml`, `nats.conf`, `nats.secrets.conf`, `postgres.secrets`, `senses.yml`) never live in the Git checkout. They are managed as local habitat allocations under `${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local/allocations/<id>/` ([Memory and the record](./memory.md#the-local-habitat)).

### Manifest (`hale.toml`) and Lockfile (`hale.lock`)

`hale habitat graft` configures two independent environments in `hale.toml` so `hale check --matrix .` checks both the application and `dna/org` under their respective constitutions:

```toml
# hale habitat graft: the two entrypoints and where each deploys (`hale check --matrix`).
# The organization adopts its law (dna/org/law.hl) itself; the application keeps its own.
[claims]
no_base = true

[environments.local]
source_only = true
entrypoints = ["."]

[environments.org]
source_only = true
entrypoints = ["dna/org"]
```

### Initial Record Rows Seeded at Graft

When `hale habitat graft` provisions the Postgres record schema, it appends `record.genesis` (naming the habitat record as parent) and `record.key`, followed by:

| Row Kinds | Description |
| --- | --- |
| `application.attached` | Records the application's `main locus`, topology artifact digest, shape hash, and toolchain version. |
| `structure.observed` | One row per locus and topic observed in `.hale/dna/baseline.topology`. |
| `responsibility.proposed` | One unratified responsibility proposal per observed locus. |
| `graph.node`, `graph.edge` | Nodes for `language:hale`, `system:dna`, `application:<name>` (`written_in` edge), the root seed, and every served API surface or stream contract (`hale check --api`). |
| `knowledge.proposed`, `review.requested` (`library`) | Unless `--no-library` is passed: proposes every chapter of the Hale book (`library/language@<version>`, bound to `language:hale`) and DNA design docs (`library/design@<version>`, bound to `system:dna`) as two family Reviews (`hale dna review library approve`). |
| `knowledge.proposed`, `review.requested` (governance) | Proposes the `purpose` Review, the 25 seeded practices/mandates (`design`, `operating`, `using`), and the structural `holes` (`board`, `<app>/dev`, `<app>/reviewer`, and their mandates). |

Outside the record, `hale habitat graft` also initializes a Git repository (if needed), configures `dna.unix.member` and `dna.trust = local` in `.git/config`, generates internal service credentials in the vault, creates empty vault slots for human-supplied secrets, and formats all `.hl` files with `hale fmt`. Passing `--profile remote-body --remote <url> [--body <user@host>]` additionally configures the `origin` Git remote and `dna.body`.

---

## A repository instead of one application

When you run `hale habitat graft <dir> --attach` on a multi-service repository that has no single root Hale application entrypoint, the graft treats the directory as a **repository**:
- `hale.toml` receives only `[environments.org]` (no root application entrypoint or `.hale/dna/baseline.topology`).
- The graft inspects the repository to seed the initial knowledge graph:
  - **Purpose:** Extracted from the title and first paragraph of `README.md`.
  - **Processes & Seeds:** Derived from services in `compose.yaml` and Hale seed directories.
  - **Contracts & Docs:** Discovered under `spec/` and markdown documentation.
  - **Gates & Witnesses:** Extracted from CI job definitions and `FRICTION.md` entries.
- Writes `dna/org/structure.hl` with `operational_roles()` (defaulting to `""`; edit before grafting to include roles from `"support accounts billing on-call"` under each deployment).
- From this graph, the graft proposes the required positions and mandates under `holes`: `board`, a `reviewer` for every part that serves or consumes a contract, a `dev` and `work` item wherever a CI gate guards a part, and an `operator` under each deployment.

### Markdown Conventions Read by `hale habitat graft` and `hale dna ingest`

You can declare architectural axioms, contract wiring, and deployment mappings directly in repository Markdown files (`README.md`, `spec/*.md`), which `hale habitat graft` (and later `hale dna ingest`) parses into graph nodes and edges:

1. **Marked List Items (Axioms and Practices):**
   Start a bullet item with an inline code tag (`` `axiom` `` for a design decision, or `` `derived` ``, `` `practice` ``, or `` `law` `` for a rule proposed as a practice) followed immediately by a **bold name**. Any file links in an `axiom` item record which files the axiom shaped:
   ```markdown
   - `axiom` **Only the api is exposed.** NATS and the nodes are on the
     private network ([`protocol.yaml`](./spec/protocol.yaml)).
   ```

2. **Contract and Deployment Tables:**
   - A Markdown table with `Document`, `Served by`, `Consumed by`, and `Over` columns declares which process or seed (`code` span) serves and consumes each linked contract file (plain text names external callers):
     ```markdown
     | Document | Served by | Consumed by | Over |
     |---|---|---|---|
     | [`openapi.yaml`](./spec/openapi.yaml) | `api` | callers, `ui`, `node` | HTTP |
     ```
   - A Markdown table whose first column is `Deployment` with a `Runs` column declares deployment environments and the processes they run.

If a marked list item or declaring table violates these rules (for example, an `axiom` missing a bold name, a contract row with no link, a `code` span naming a nonexistent seed/process, or a name containing `|`), `hale habitat graft` stops before seeding the record and reports the exact offending line so you can fix the Markdown and re-run the graft.
