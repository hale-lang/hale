# Worker Processes, Tools, and Model Routing (Legs, Hands, and Voice)

In DNA, a **leg** (`hale dna work`) is a stateless worker process that claims one task attempt from the edge API (`head`), reads the attempt's structured context package (**hat**), executes sandboxed tools (**hands**) and LLM requests (**voice**), and submits the outcome back under a time-bounded lease. Legs hold no persistent state between tasks, hold no direct Postgres roles, and cannot settle their own work—the organization spine validates and settles every submitted outcome.

```sh
# 1. Authenticate against the local OIDC stub (or print the browser login URL)
eval "$(hale dna work login --as alice)"

# 2. Enqueue a judgment task and run a single worker cycle as position:agent
hale dna task create --judgment assess whether the storage migration is safe to ship
hale dna work loop --as position:agent --parallel 1 --once
```

## Architecture Overview

| Component | Engineering Role | Source Location |
| --- | --- | --- |
| **Legs** (`hale dna work`) | CLI verbs, performer dispatch, and supervised worker loop | `dna/core/legs` (vendored as `vendor/dna/legs`), `dna/org/work.hl` |
| **Hat** | Deterministic, structured per-Work context package fetched per attempt | `dna/core/hat.hl` |
| **Hands** (`legs::Hands`) | Sandboxed execution interfaces for Git, GitHub (`gh`), Hale/Go toolchains, and lease management | `dna/core/legs/hands.hl`, `dna/core/contracts.hl`, `dna/core/tools.hl` |
| **Voice** (`ModelBackend`) | LLM client router, credential vault bindings, budget gate, and record/replay tape | `dna/org/models.hl`, `dna/core/models.hl`, `dna/core/tape.hl` |

The leg binary is compiled once per project from `dna/org/work.hl` and `vendor/dna/legs` into `.hale/dna/legs/<key>`. When running an MCP server via `hale mcp`, the leg verbs are exposed as the `hale_dna_work` tool and the current Work's hands as `hale_dna_hand` (under MCP, `loop` only runs with `--once` or `--drain`).

---

## Step-by-Step Manual Execution

Before running worker commands, start the organism ([The heart and the body](./heart.md)) and the edge API server ([The head and the face](./head.md)).

All `hale dna work` subcommands connect to `--api` (defaulting to `HALE_DNA_API`, or `http://127.0.0.1:8793`). Read requests use HTTP (`Authorization: Bearer $HALE_DNA_ID_TOKEN` when OIDC is enabled), while mutations (`HeadCommands`) are sent over the local Unix domain socket advertised by the head, where the OS kernel authenticates your UID, or over the head's HTTP commands port.

1. **Authenticate (OIDC heads):**
   When connected to the local stub OIDC issuer started by `hale dna head`, `login` reads the client secret from the vault and prints an `export HALE_DNA_ID_TOKEN=…` statement without exposing the secret:
   ```sh
   eval "$(hale dna work login --as alice)"
   ```
   Against an external production OIDC issuer, `login` refuses to mint a token, prints the head's `/auth/login` URL for browser sign-in, and exits `1`.

2. **Claim, inspect, submit, and settle an attempt step by step:**
   ```sh
   hale dna task create --judgment assess whether the storage migration is safe to ship
   hale dna work next --as position:agent
   hale dna work brief --attempt <attempt> --render text --plain
   hale dna work submit --as position:agent --attempt <attempt> --token <n> --result "assessment: safe to ship"
   hale dna work settle --attempt <attempt> --token <n> --wait 30
   ```
   - `next` claims the next matching attempt and returns the lease metadata: `attempt`, `work`, `task`, `holder`, fencing `token`, and expiration timestamp (`until`).
   - `brief --render text --plain` fetches and formats the Work's hat for human reading.
   - `submit` records the outcome under the lease (`attempt.outcome_requested`).
   - `settle` polls until the spine settles the outcome (`settled` with disposition, or `refused` with a reason), recording rows from `attempt.claimed` through `attempt.outcome` ([One task, end to end](./workflow.md)).

---

## `hale dna work` CLI Reference

Every subcommand except `loop` outputs a single JSON object (`verb`, `state`, `reason`, and the receipt's `attempt`) and exits immediately. `submit` and `release` derive deterministic idempotency keys from the lease (`work-submit:<attempt>:<token>`), so repeating a command returns the original receipt; pass `--request-id` to override the key on any verb. Every verb accepts `--api`.

| Subcommand | Description | Flags |
| --- | --- | --- |
| `login` | Prints `export HALE_DNA_ID_TOKEN=…` on the local stub OIDC issuer, or prints the browser `/auth/login` URL (exit `1`) on an external issuer | `--as <person>` (defaults to the record's sole member), `--api` |
| `next` | Claims the next available attempt for a position (`Commands::claim`) | `--as`, `--kind`, `--capabilities`, `--classes` (default `"public internal"`), `--orgs`, `--ttl` (`600`), `--effect`, `--worker` |
| `brief` | Fetches the raw JSON hat or renders it for a target consumer | `--attempt` or `--work`, `--render text\|prompt\|agent`, `--plain` |
| `renew` | Extends an active lease while preserving its fencing token (`AttemptRenew`) | `--as`, `--attempt`, `--token`, `--ttl`, `--renewal <n>` (`1, 2, …` per renewal) |
| `allowance` | Requests the attempt's model spend allowance from the spine and waits (`AttemptAllowance`) | `--as`, `--attempt`, `--token`, `--wait` (`60`) |
| `hand` | Executes one of the Work's granted hands locally (file hands in the attempt's kept worktree, infra hands via the head) | `--attempt`, `--name`, `--args` (JSON object matching the hand's schema) |
| `submit` | Submits the attempt outcome under the lease (`AttemptOutcome`) | `--as`, `--attempt`, `--token`, `--from-worktree`, `--result` or `--result-file`, `--disposition` (`done`), `--narrative`, `--evidence-file`, `--receipt-file`, `--receipt-class` (`internal`), `--effect`, `--hat-digest`, `--hat-head`, `--hat-watermark`, `--prompt-digest`, `--renderer` |
| `settle` | Polls for outcome settlement from the spine (`CommandLookup`) | `--attempt`, `--token`, `--wait` (`0`) |
| `release` | Releases the lease without an outcome (`AttemptRelease`) | `--as`, `--attempt`, `--token`, `--why` |
| `friction` | Files an un-gated `friction.filed` diagnostic row (`FrictionFile`) | `--as`, `--text`, `--attempt` |
| `run` | Runs one full lifecycle: claim, brief, perform, submit, and settle | `--as`, `--kind`, `--performer person\|deterministic\|model`, `--wait` (`30`), plus all `next` filter flags |
| `loop` | Runs supervised worker children that repeatedly execute `run` | `--as`, `--parallel N` (`1..64`), `--once`, `--only <kind>`, `--idle-ms` (`2000`), `--performer`; `loop --drain` stops a running loop |

### Position Identity, Routing, and Exit Codes

- **Position identity (`--as`):** Must be a graph position ID formatted as `position:<name>` (slashes are allowed, such as `position:api/dev`). Your authenticated person must hold that position via ratified `holds` edges in memory (`hale dna fill`), or the repository must declare `dna.trust = local` (the default set by `hale habitat graft`). Over the Unix socket, the head maps your OS UID to a person via `git config --local --add dna.unix.member "uid:<n>=<person>"`.
- **Performer kinds and position routing (`--kind`):** Standard performer kinds include `agent` and `software`. Any graph position (`position:api/dev`) acts as an `agent` and also claims Works explicitly routed to that seat (`hale dna task create --to position:api/dev`). Routed tasks create a `Patch` Work that only a holder of `position:api/dev` can claim, briefed with that position's mandate and executed via the tool loop. Any `position` token passed manually in `--capabilities` is stripped by the head; seat capabilities are derived strictly from your authenticated position. When no attempt matches, the refusal message lists the outstanding kinds and the flag required to claim them.
- **Exit codes:**
  - `0`: Command admitted by the head, or read query succeeded.
  - `1`: Refusal (`state: "refused"` with `reason`), unauthorized verb, unreachable head, or a `run` ending in `unsettled` or `unresolved`.
  - `2`: CLI usage or configuration error rejected before contacting the head.

---

## Performers and Effect Classes

`dna/org/work.hl` defines the project's performer catalog (`legs::PerformerCatalog`) across three slots: `person`, `deterministic`, and `model`.

```hale,fragment
fn model() -> legs::ModelPerformer {
    return legs::ModelPerformer { router: agent_models(), effect: "uncertain" };
}

fn performers() -> legs::PerformerCatalog {
    // A person answers what they are asked; asked again, they answer again
    return legs::PerformerCatalog {
        person: legs::Person { effect: "idempotent" },
        deterministic: deterministic(),
        model: model(),
    };
}
```

| Performer Type | Accepted Kinds | Brief Render Mode | Behavior |
| --- | --- | --- | --- |
| `legs::Person` | Every kind | `text` | Prints the rendered brief; the human operator runs `submit` when finished |
| Deterministic (`legs::FixedAnswer`, `legs::CodebaseWork`, or custom) | Kinds explicitly listed by the performer (takes precedence) | `agent` | Executes deterministic Hale code (`legs::NoDeterministic { }` by default) |
| `legs::ModelPerformer` | `agent`, `service`, `software` | `prompt` / `agent` | Routes the task to an LLM backend (`legs::NoModel { }` when no backend is configured) |

### Performer Selection Rules

During `hale dna work run`, the leg selects a performer in priority order:
1. The **deterministic** performer, if it accepts the attempt's kind (`legs::FixedAnswer { kinds: "agent", effect: "effect_free", result: "…" }` is the minimal implementation; `legs::CodebaseWork` handles attached foreign repositories).
2. The **person** performer, if the work kind is `human` or if the model performer does not accept the kind (for example, when `model` is `legs::NoModel { }`, preventing unconfigured agents from burning attempts).
3. The **model** performer otherwise.

Passing `--performer person|deterministic|model` forces a specific slot, though a performer will never claim a kind it does not accept.

To run dry-run rehearsals without API keys, wire `dna::FakeModel` into `dna/org/work.hl`:

```hale
import "vendor/dna" as dna;
import "vendor/dna/legs" as legs;

fn fake() -> dna::FakeModel {
    return dna::FakeModel {
        name: "fake",
        model: "fake-1",
        answer: "assessment: the migration is additive and safe to ship",
    };
}

fn agent_models() -> dna::ModelRouter {
    return dna::ModelRouter { quick: fake(), deep: fake(), private: fake() };
}

fn model() -> legs::ModelPerformer {
    return legs::ModelPerformer { router: agent_models(), effect: "effect_free" };
}
```

### Effect Classes and Failure Recovery

Every performer in `dna/org/work.hl` **must** declare an `effect` class. There is no default; if any performer omits `effect`, `next`, `submit`, `run`, and `loop` exit `2`.

| Effect Class | Semantics | Behavior on Failed Settle or Expired Lease |
| --- | --- | --- |
| `effect_free` | Pure computation or read-only model call; no external state mutated (default for hosted models without a local harness) | Marks attempt `unsettled`, releases the lease, and retries automatically after backoff |
| `idempotent` | Safe to repeat to reach the same state | Marks attempt `unsettled`, releases the lease, and retries automatically after backoff |
| `uncertain` | May have caused external side effects (e.g., an agent harness with local tools, `claude` on `PATH`) | Marks attempt `unresolved` (`attempt.unresolved`), files friction, halts that worker slot, and waits for human resolution |

The claim records the performer's effect class, and the Work's hat specifies which effect classes the Work admits (for example, an assessment admits all three, whereas a code edit refuses `effect_free` performers). Only `next` and `submit` accept a manual `--effect` flag; `run` and `loop` always use the performer's declared class.

Because an expired `uncertain` lease halts automated retries, `uncertain` claims default to a 1-hour TTL. When an `uncertain` attempt becomes `unresolved`, a human operator must inspect its history and resolve the effect outcome manually:

```sh
hale dna effect resolve attempt:<attempt> --outcome ok|failed
```

---

## Supervised Worker Loop (`hale dna work loop`)

`hale dna work loop --as position:agent --parallel N` spawns `N` supervised worker processes (`1..64`). Each child claims work under a distinct holder identity (`position:agent#1` through `position:agent#N`) so concurrent workers never share a lease.

- **Backoff:** A worker that completes a task immediately checks for the next one. A worker that finds no matching work, leaves work for a human, or releases its lease waits for `--idle-ms` (default `2000` ms), doubling its wait on each consecutive idle pass up to 60 seconds.
- **Output:** Each child prints one JSON receipt line as it finishes a cycle; the supervisor prints a summary JSON line when the loop exits.
- **Graceful Drain:** Sending `SIGTERM` or `SIGINT` stops spawning new children and signals active children to finish (forcing termination after 3 seconds). From another terminal, running `hale dna work loop --drain` writes `.hale/dna/legs/drain`, instructing the active loop to stop claiming new attempts and exit once in-flight children complete. Any child terminated mid-task leaves its lease to expire so the spine can re-queue the attempt.
- **Concurrency Retries:** When concurrent workers advance the record simultaneously, the head may return `snapshot_changed` or `command_busy`. The leg automatically retries the command under the same `request_id` for up to 10 seconds.

---

## The Hat (Work Context Package)

The **hat** (`dna/core/hat.hl`) is a deterministic, structured representation of a single Work's execution context—never an ad-hoc prompt string. Calling `hale dna work brief` fetches the hat from the head (`GET /dna/context?id=<work>`).

### Contents of a Hat

- **Position & Governance:** The target position, its charter, and all ratified practices bound to the Work's target (with their digests/IDs).
- **Work Specification:** Objective, target, output contract (`Patch`, `Assessment`, etc.), data classification, requirements, admitted effect classes, cost ceiling, and bound knowledge nodes.
- **Hands & Validation:** The tool definitions granted by the output contract (name, description, JSON Schema of arguments), the tool grant authorizing them, and the subset of tools designated as outcome validators.
- **Model Resolution:** The selected model (`mode`, `size`, `category`, `model`, and the routing rule that selected it).
- **Execution Lineage & Provenance:** The execution history of the enclosing Task, the record head digest, the Postgres memory watermark at which the hat was read, and the hat's own SHA-256 digest.

### Lease-Gated Access Control and Deterministic Rendering

On an OIDC-enabled head, reading `/dna/context` requires holding the active lease on that Work's attempt. If you have not claimed the attempt with `next`, or if another person holds the lease, the head refuses the read with `lease_required`.

Because the hat incorporates no wall-clock timestamps, environment variables, or random IDs, reading the same Work at the same record head always produces an identical digest. The leg renders the hat into one of three formats:
- `text`: Formatted for a human operator.
- `prompt`: Formatted for a single-turn LLM call.
- `agent`: Formatted for a tool-calling agent, listing every granted hand and the required validators.

When submitting an outcome, the leg includes the renderer version (`legs-render/2`), `--hat-digest`, and `--prompt-digest` so replays can reconstruct the exact prompt from the recorded hat.

---

## Hands (Sandboxed Tool Execution)

A Work's **output contract** (`dna/core/contracts.hl`, `dna/core/tools.hl`) determines which tools (**hands**) a performer may invoke and which validators must pass before an outcome is accepted:

| Output Contract | Granted Hands (`BUILTIN_TOOLS`) | Required Validators |
| --- | --- | --- |
| `Patch` | `read`, `edit`, `check`, `test`, `fmt`, `patch` (plus `go_vet`, `go_test`, `gofmt_check` on Go codebases) | `check`, `test`, `fmt` (or Go equivalents) |
| `Assessment` | `record_status`, `record_history`, `org_chart` (read-only infrastructure queries) | None |
| Other contracts | None by default | None |

All hands execute locally on the leg's machine—never inside the organism's spine. Validators run locally before `submit`, and the spine re-runs the exact same checks upon receiving the submission.

### The `legs::Hands` Interface Catalog

Inside `dna/core/legs/hands.hl`, a performer receives a `legs::Hands` locus containing seven capability interfaces, each returning a `HandResult { ok: Bool, output: String, error: String }`:

| Hand Slot | Interface & Default Implementation | Operations |
| --- | --- | --- |
| `git` | `GitHand` (`ScratchGit`) | Clones a scratch worktree outside the primary checkout (`worktree` with `gc.auto=0`), applies unified diffs (`apply`), commits staged changes (`commit`), formats the head commit as a patch (`patch`), creates branches (`branch`), pushes to `origin` (`push`), and removes scratch directories (`clean`). |
| `forge` | `ForgeHand` (`GhForge` / `ForgeBridge`) | Opens a pull request via `gh pr create` (`open_review`) and reads the PR review decision (`verdict` -> `approve`, `revise`, or `pending`). A leg never merges PRs. |
| `toolchain` | `ToolchainHand` (`Toolchain`) | Runs `hale check <path>` (`check`), `hale test <path>` (`test`), and `hale fmt --check <path>` (`fmt_check`). |
| `go` | `GoHand` (`GoToolchain`) | For attached Go codebases: checks for `go` and `gofmt` on `PATH` (`available`), runs `go vet ./...` (`vet`), runs `go test <pkg>` (`test`, validating package patterns), and checks formatting with `gofmt -l .` (`gofmt_check`). |
| `lease` | `HeadLease` | Renews the active attempt lease (`renew`) and requests/polls the attempt's model spend allowance from the spine (`allowance`). |
| `deploy` | `DeployHand` (`NoDeploy`) | Refuses execution; deployment expression is owned by the body. |
| `heart` | `HeartHand` (`NoHeart`) | Refuses execution; application heart API calls are not yet delegated to legs. |

When completing a `Patch` Work, the leg commits its changes in the scratch clone and submits `result_ref: commit:<sha>` along with the `git format-patch` receipt. The spine applies the patch to the genome's base, re-verifies it, and opens a Review with the leg recorded as the author ([Reviews](./head.md#reviews)). For foreign repositories attached via `hale dna codebase attach`, `legs::CodebaseWork` works on a branch of the attached repository, runs its configured build/test commands, and opens a pull request on its forge.

---

## Voice: Model Catalog, Routing, and Tapes

The organization routes LLM calls through a unified `ModelBackend` interface configured in `dna/org/models.hl`. `hale habitat graft` generates this file based on the tools and keys available on the machine, and you can edit it as part of your genome:

```hale,fragment
fn frontier() -> dna::OpenAiChat {
    return dna::OpenAiChat {
        name: "deep",
        model: "gpt-4o",
        endpoint: "https://api.openai.com/v1/chat/completions",
        credential: dna::HostedCredential { key: "OPENAI_API_KEY", scheme: "bearer" },
        input_micros_per_1k: 2500,
        output_micros_per_1k: 10000,
    };
}
// …
fn agent_models() -> dna::ModelRouter {
    return dna::ModelRouter { quick: harness(), deep: harness(), private: desk() };
}
```

`leader_models()` and `editor_models()` configure the Leader and SourceEditor positions inside the spine; `agent_models()` configures the model leg; and `org_budget()` defines the organization's spend ceiling enforced by the spine ([The spine](./spine.md)).

### Supported Model Backends

| Backend Locus | Protocol / Target | Security & Effect Notes |
| --- | --- | --- |
| `dna::OpenAiChat` | OpenAI Chat Completions API (OpenAI, OpenRouter, vLLM, Ollama) | Authenticates via `scheme: "bearer"`; carries the `external_model` effect class |
| `dna::AnthropicMessages` | Anthropic native Messages API | Authenticates via `scheme: "x-api-key"`; carries the `external_model` effect class |
| `dna::LocalModel` | OpenAI-compatible HTTP endpoint on localhost | No API key, no `external_model` effect; permitted for any data classification |
| `dna::HarnessModel` | Local CLI agent harness (`claude` or `codex`) with built-in tools | Runs in an exported copy of the worktree confined under `dna::Bubblewrap` |
| `dna::RecordedModel` | Record/replay wrapper around any `ModelBackend` | Captures or replays multi-turn tool loops and patches deterministically |
| `dna::FakeModel` | Scripted static or multi-turn answers (`turns_dir`) | Keyless; used for tests, dry runs, and fixtures |

Because hosted backends (`OpenAiChat`, `AnthropicMessages`) carry the `external_model` effect class, constitutional claims in `dna/org/law.hl` can forbid customer or confidential data from reaching external providers.

### Vault Credentials and Probing (`hale dna models`)

> [!IMPORTANT]
> **Model API keys are read exclusively from the vault—never from environment variables.** A `dna::HostedCredential { key: "OPENAI_API_KEY", scheme: "bearer" }` reads the vault slot `model-OPENAI_API_KEY` fresh on every request in the exact statement that writes to the socket, and never returns the raw secret bytes. If the vault slot is empty, the router refuses the call before touching the network.

Store a model key from `stdin`:

```sh
hale dna secret set OPENAI_API_KEY
```

To verify your catalog without starting the organism or writing to the record, run `hale dna models`. It compiles `dna/org/models.hl` into `.hale/dna/probe` and sends a minimal probe request to each configured backend:

```text
$ hale dna models
catalog dna/org/models.hl
backend     slot      model                         answer
frontier    deep      gpt-4o                        not permitted (no credential present)
fast        quick     gpt-4o-mini                   not permitted (no credential present)
```

Permitted backends print `ok`, latency, cost, and the first line of the response; failing backends print `refused: <reason>`.

### Budget Gate, Retries, and the Agent Tool Loop

Before making its first LLM request, `legs::ModelPerformer` calls `allowance` to reserve spend from the spine:
- If the budget gate refuses, the leg makes no LLM call and submits `declined`.
- If a backend returns a rate-limit error without having acted, the leg retries up to 3 times (waiting 1s, 2s, 4s), renewing the attempt lease before each sleep and recording each backoff as evidence.
- Every LLM call is attached to the submitted outcome as evidence (adapter, backend, model, input/output tokens, micro-dollar cost, wall time) and journaled by the spine as `model.called`. `hale dna history <task>` totals the spend (`usage of <task>: …`).

When a Work's hat grants hands (`tools` or `do` mode), the leg runs a multi-turn **tool loop** (up to `max_turns = 16` or until the attempt's allowance is exhausted):
1. The model receives the rendered prompt and the JSON Schema tool definitions for the Work's granted hands.
2. Every tool call requested by the model is executed locally by the leg—never by the model provider or gateway—and its output is returned as the next conversation turn.
3. **Path and tool confinement:** Any tool not in the hat's grant, or any file path containing `..`, `.git`, control characters, or symbolic links escaping the scratch worktree, is rejected and returned to the model as a refusal turn.
4. **Validation (`do` mode):** For `Patch` Works running in a scratch clone, when the model finishes, the leg runs the hat's validators (`check`, `test`, `fmt`) across every modified directory containing Hale source. If validation fails, the diagnostics are fed back to the model as another turn. Once clean, the leg commits the change on the base and submits `result_ref: commit:<sha>` with the patch receipt first.
5. **Audit trail:** Each turn uses a deterministic `Idempotency-Key` (`<attempt>:call:<turn>`), and every tool invocation is logged in the `tool-calls` receipt (hand name, input/output digests, and failure reason).

To test the tool loop offline, point `dna::FakeModel { turns_dir: "<dir>" }` at a directory of `<n>.json` files, where each file specifies `content` and/or `tool_calls` (`[{ "name": "…", "arguments": {…} }]`).

### Dynamic Model Mapping (`category` and `rule`)

When routing through a unified OpenAI-compatible gateway, two record tables map each Work to a concrete model without editing `models.hl`:

```sh
hale dna models category do.deep coder-l --tools --price-in 300 --price-out 1200
hale dna models category think.quick thinker-s
hale dna models rule default quick
hale dna models rule position:api/dev deep --permit standard
hale dna task create --size standard "…"      # per-task size override
hale dna models map                           # inspect both tables
```

- **Category (`<mode>.<size>`):**
  - `mode` is derived from the Work's hands: `think` (no hands), `tools` (read-only hands, no workspace), or `do` (workspace hands). Categories for `tools` and `do` require models registered with `--tools`.
  - `size` is `quick`, `standard`, or `deep`.
- **Rule resolution order:** Most specific wins: `task:<id>` (or `--size` on `task create`, constrained by the `--permit` list of the underlying rule) -> `position:<name>` -> `contract:<Name>` -> `default` -> `standard`.
- The resolved mapping is embedded in the hat, and the leg sends that model name along with an `Idempotency-Key` and optional attribution `metadata` (`attempt`, `position`, `holder`, enabled via `send_metadata: true`). If a `tools` or `do` Work resolves to a model without `--tools`, the attempt is declined before any spend (`the model mapping refused <model> for this Work`).

### Record and Replay Tapes (`dna::RecordedModel`)

`dna::RecordedModel { dir, mode, inner }` wraps any `ModelBackend` to record or replay LLM interactions deterministically:

```hale,fragment
fn frontier() -> dna::RecordedModel {
    return dna::RecordedModel {
        name: "deep",
        dir: ".hale/dna/tape",
        dir_env: "HALE_DNA_TAPE_DIR",
        mode_env: "HALE_DNA_TAPE",
        inner: dna::AnthropicMessages {
            name: "deep",
            model: "claude-sonnet-5",
            input_micros_per_1k: 3000,
            output_micros_per_1k: 15000,
        },
    };
}
```

- **`record` mode:** Forwards requests to `inner` and writes the response, tool calls, and any harness workspace patch under a composite key derived from the request identity (role, backend, requested model, prompt and context digests, data class, grant, starting workspace tree digest for a harness, and offered tools plus prior turns for a tool loop).
- **`replay` mode:** Serves responses and applies patches directly from `dir` without API keys or network calls. A cache miss fails immediately, reporting the request key and—if an entry shares the same prompt digest—the exact fields that differed.

---

## External Harnesses and Interactive Human Sessions

### Custom External Harnesses

An external agent harness does not need to be compiled into `dna/org/work.hl`. It can drive the leg CLI directly:
1. Claim an attempt with `hale dna work next`.
2. Fetch the agent brief with `hale dna work brief --attempt <id> --render agent`.
3. Reserve budget with `hale dna work allowance --attempt <id> --token <n>` and stay within the granted micro-dollar ceiling.
4. Submit the result with `hale dna work submit … --evidence-file calls.json`, passing a JSON array of `model.called` objects (each naming its `adapter`). Exceeding the granted allowance—or spending without calling `allowance`—settles the attempt as `failed`.

### Interactive Human Sessions and Editor Assistants

A human operator (or an IDE coding assistant connected via `hale mcp`) can work on an attempt using the organization's hat and validated hands:

```sh
hale dna work next --as position:api/dev
hale dna work hand --attempt <attempt> --name read --args '{"path": "api/todo.hl"}'
hale dna work hand --attempt <attempt> --name edit --args '{"path": "api/todo.hl", "text": "…"}'
hale dna work submit --as position:api/dev --attempt <attempt> --token <n> --from-worktree --result "…"
```

File hands (`read`, `edit`, `check`, `test`, `fmt`, `patch`) operate inside a persistent per-attempt worktree cloned at `.hale/dna/work/<attempt>`. When ready, `submit --from-worktree` runs the hat's validators across all touched Hale directories, commits the change on the base, submits the commit and patch receipt for spine verification and Review, and cleans up the worktree.

---

## Troubleshooting Worker Errors

| Diagnostic / Symptom | Cause & Resolution |
| --- | --- |
| ``the head's socket lists no `Commands::claim` for <person>`` | Your OS UID is not mapped to a person, or that person holds no ratified position. Map the UID with `git config --local --add dna.unix.member "uid:<uid>=<person>"` and assign a seat with `hale dna fill`. |
| `the model performer declares no effect class` (exit `2`) | Add `effect: "effect_free"`, `"idempotent"`, or `"uncertain"` to the performer in `dna/org/work.hl`. |
| `no admitted attempt awaits a … leg with capabilities …` | No attempt of that kind is queued for legs, or none matches your `--classes` or `--capabilities`. The refusal message lists the outstanding attempts and why each was skipped. |
| `declined: the model mapping refused <model> for this Work` | A `tools` or `do` Work resolved to a model category registered without `--tools`. Update the category: `hale dna models category <mode>.<size> <model> --tools`. |
| `` `<name>` is not a hand of this Work `` / `` `<path>` is outside the worktree `` | The tool is not granted by the Work's output contract, or the path contains `..`, `.git`, control characters, or symlinks escaping the worktree. |
| `unresolved` | An `uncertain` performer failed to settle or let its lease expire. Inspect `hale dna history <attempt>` and resolve with `hale dna effect resolve attempt:<attempt> --outcome ok\|failed`. |
| `declined` | The spine's budget gate refused spend because the budget window is exhausted ([The spine](./spine.md)). |
