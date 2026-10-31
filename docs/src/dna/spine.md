# The Spine

The **spine** is the workflow execution engine, budget coordinator, and record projector that runs on every DNA node. It consists of the host supervisor started by `hale dna dev` or `hale dna run` (`dna/host/host.hl`) paired with the compiled organization binary (`dna/org/main.hl`), operating under PostgreSQL's `spine` role and the active body lease. Spine nodes hold no persistent state in process memory: if a node loses its database lease or NATS connection, it terminates immediately and a replacement node reconstructs all active workflows from PostgreSQL.

```sh
hale dna dev                    # run the spine and application together locally
hale dna definitions            # inspect the workflow catalog and store bindings
hale dna body                   # check which host currently holds the spine lease
```

## A Node

Run a spine node locally with `hale dna dev` (which uses the local habitat's PostgreSQL and NATS allocations and supervises your application in the same process tree) or on a dedicated server with `hale dna run` (which reads `HALE_DNA_MEMORY_DSN_SPINE`, `HALE_DNA_NATS_URL_SPINE`, and `HALE_DNA_NATS_ORG` from the environment; see [Memory](./memory.md#working-memory) and [The Nerves](./nerves.md#migrate-and-drop)).

On every 1-second tick, each spine node executes seven steps in order:

1. **Asserts the body lease** in PostgreSQL, terminating immediately if the lease is no longer held by this node.
2. **Relays unanswered request rows** from `record_rows` onto NATS JetStream ([The Nerves](./nerves.md#relaying-after-the-row-lands)) before doing any other work so user commands never queue behind background maintenance.
3. **Reads new record and ledger rows** and deletes local Git branches (`dna/candidate/<mutation>`) for any candidates marked `candidate.dropped`.
4. **Projects new record rows into the PostgreSQL graph** and erases encrypted bodies of any protected receipts marked `receipt.redacted`.
5. **Polls the Git remote for genome updates** ([below](#the-genome-pull)).
6. **Re-asserts the body lease** before acting on external outputs, ensuring previous steps did not outlast the lease TTL.
7. **Publishes `head.row.landed` notifications** to connected heads and processes any pending `expression.restart_requested` rows.

Each record admits one active body at a time (or one body per organization owner on a shared record; see [The Body Lease and the Fence](#the-body-lease-and-the-fence)). Across a shared record, multiple owner nodes operate on the same PostgreSQL schema without inter-node locks: graph projection advances one row at a time via compare-and-swap, and receipt erasure is idempotent by content digest.

### Distributed Claims (`claims` Table)

Any node operation that must not execute concurrently across nodes acquires a time-bounded lock in PostgreSQL's `claims` table using a conditional insert keyed by claim ID. Lease expiration is evaluated against the PostgreSQL server clock so clock skew between hosts never causes split-brain execution. If a node crashes while holding a claim, the row expires and the next node resumes the operation.

Claim acquisitions and releases are logged in the ledger:

```text
claim.taken plan/i7 {"holder": …, "token": 1, "until": …}
claim.released plan/i7 {"holder": …}
```

Nodes identify themselves in claims using `user@host:<clone>` (passed to child processes as `HALE_DNA_NODE`).

### The Genome Pull

Spine nodes update their code by polling the forge rather than receiving push deployments. Every 5 minutes (`HALE_DNA_GENOME_POLL` in seconds; set `0` to disable), the node fetches the remote repository's default branch:
- If the default branch advanced, the node releases its body lease and exits cleanly with code `75`.
- The supervising systemd unit restarts the node, which compiles the updated genome before re-acquiring the lease.
- If the new genome fails to compile, the node records `node.build_failed`, checks out the last known-good revision, and continues serving. `hale dna status` displays each node's active commit on the `genome:` line.

## Core Operations: Ask, Claim, Settle, and Read

All interactions with the spine flow through four operations:

| Operation | Caller | Spine Action |
| --- | --- | --- |
| **Ask** | CLI (`hale dna task create`), web face, or Leader optimize pass | Validates admission gates and admits a workflow execution (or records `intent.refused`) |
| **Claim** | Worker leg via head API (`hale dna work next`) | Grants a time-bounded lease on one pending workflow attempt |
| **Settle** | Worker leg via head API (`hale dna work submit`) | Verifies the attempt lease and records the attempt's outcome |
| **Read** | Head API, CLI, or worker leg | Queries `record_rows`, the `ledger`, the organizational `graph`, or a work unit's `hat` |

When an `intent.requested` row is relayed to `dna/org`, the spine deduplicates it by intent ID and evaluates four admission gates in order:
1. **Board gate**: Rejects empty intent descriptions.
2. **Owner gate**: On a shared record, verifies that this node's organization owns the target position.
3. **Budget gate**: Rejects the intent if the current spend window is exhausted ([below](#the-budget-the-one-gate)).
4. **Planning claim (`plan/<intent>`)**: Acquires the planning claim when Leader planning is enabled ([below](#the-leader-and-planning)).

Once admitted, the spine writes `intent.offered`, `workflow.admitted`, and `task.born` (or `intent.refused` with the rejection reason). Scheduled workflows bypass the intent gate and are admitted directly under their schedule occurrence key (see [Schedules](./schedules.md)).

Worker legs claim, renew, request budget allowances, and submit outcomes through the head's RPC endpoints (`dna.attempt.claim`, `dna.attempt.renew`, `dna.attempt.allowance`, `dna.attempt.outcome`, `dna.attempt.release`). See [Legs, Hands, and Voice](./legs.md) and [The Head and the Face](./head.md).

## The Leader and Planning

By default, `dna/org/main.hl` sets `planned: true`, routing every incoming ask through the **Leader** before workflow admission. The Leader acts as the organization's planner and bounded reviewer, reading:
- `dna/org/charter.hl` (the Leader's architectural brief)
- `dna/org/purpose.hl` (the repository's declared purpose)
- `dna/org/law.hl` (compile-time architectural claims)
- All Board-ratified practices bound to `org`

Because planning invokes an LLM and consumes budget, the node first acquires the `plan/<intent>` claim (300-second TTL). If another node already holds `plan/<intent>`, the losing node logs `planning elsewhere: <holder> holds plan/<intent>` and waits for the next relay or claim expiration.

The Leader classifies the request (`organism`, `appendage`, `product`, or `person`), determines the change class and target path, and selects the assignee and obligation for human tasks. The spine then binds the plan (which may narrow, but never widen, the original ask) to a workflow definition:

| Request Shape | Admitted Workflow Definition | Execution Plan |
| --- | --- | --- |
| Classified by Leader as human work (`person`) | `ask-person` | Single human leaf creating a handed `case` for the named assignee |
| Classified by Leader as code/config work | `ask-edit` | Single edit leaf producing a verified candidate Mutation for Review |
| `hale dna task create --judgment` | `ask-judge` | Single judgment leaf dispatched to a worker leg (skips Leader planning) |
| `hale dna task create --to position:<name>` | `ask-position` | Single patch leaf dispatched to the leg holding `<name>` (skips Leader planning) |

If the Leader returns no classification or no Leader model is configured, the ask defaults to an `application` change under `ask-edit` at the ask's target. Beyond planning, the Leader also decides Reviews that fall within its `AutonomyBoundary` grant ([Shaping and Governing](./shaping.md)) and executes periodic optimization walks ([Schedules](./schedules.md)).

## The Workflow Engine

### Workflow Definitions (`dna/org/own_workflows.hl`)

A workflow definition is a static DAG of ordered steps validated before execution begins. Every step declares the **single store it writes** (`record`, `forge`, `genome`, `heart`, `graph`, `nerves`, `memory`, `vault`, or `host`). Each step contains one or more members:
- **Leaf**: A single unit of work specifying an `objective`, required capability tags (`requires`), and maximum retry attempts.
- **Child**: A nested workflow definition executed as a sub-workflow of the parent step.

Define custom workflows in `dna/org/own_workflows.hl`:

```hale
import "vendor/dna" as dna;

fn own_workflows(catalog: dna::WorkflowCatalog) -> String {
    let d = catalog.define("close-month", 1, "record record", "close the month");
    if len(d) > 0 { return d; }
    let a = catalog.leaf(
        "close-month",
        1,
        0,
        "reconcile",
        dna::WorkRequest { objective: "reconcile the bank feed", requires: "analysis" },
        2,
    );
    if len(a) > 0 { return a; }
    let b = catalog.leaf(
        "close-month",
        1,
        1,
        "post",
        dna::WorkRequest { objective: "post the journal entries", requires: "human" },
        1,
    );
    if len(b) > 0 { return b; }
    return catalog.occurs("close-month", 1, "skip", "period");
}
```

In this example:
- `close-month` revision `1` has two sequential steps (`"record record"`), both writing to `record`.
- Step `0` (`reconcile`) requires `"analysis"` and permits up to `2` attempts.
- Step `1` (`post`) requires `"human"` (`1` attempt) and activates only after step `0` settles.
- `catalog.occurs("close-month", 1, "skip", "period")` configures scheduled concurrency (`skip` skips new occurrences while an execution is still open; `overlap` runs occurrences concurrently) and requires scheduled triggers to supply `"period"` in `--args` ([Schedules](./schedules.md)).

A workflow definition is rejected at admission—before any step starts—if it:
- References an unknown child workflow or introduces a cycle.
- Uses member keys outside `[a-z0-9-]+`.
- Declares zero or multiple stores for a step, or targets a store (`heart`, `vault`, `host`) that workflows are not permitted to write.
- Exceeds engine structural bounds: max depth `8`, max `32` steps per workflow, max `32` members per step, max `8` attempts per leaf, or max `256` total work units.

### The Workflow Catalog & `hale dna definitions`

`hale habitat graft` generates `dna/org/workflows.hl`, which combines the built-in DNA workflow catalog with `own_workflows`. Running `hale dna upgrade` updates `dna/org/workflows.hl` to the current toolchain revision while leaving `dna/org/own_workflows.hl` untouched. Because workflow executions bind the definition's revision number (`@1`, `@2`, …) at admission, in-flight tasks finish under the revision they started with while new tasks bind the upgraded revision.

Run `hale dna definitions` (or `hale dna definitions --json`) to inspect all registered definitions, their step stores, and any admission refusal reasons:

```text
$ hale dna definitions
catalog dna/org/workflows.hl
ask-edit@1  an ask: one change, prepared for review
  0. e  record
  occurs: skip while open, takes objective
…
change-deliver@1  a change delivered: reviewed at the forge, applied, deployed, settled on the pulse
  0. candidate  record
  1. review  forge
  2. verdict  record
  3. apply  genome
  4. deploy  heart
  5. settle  record
  refused at admission: workflow change-deliver@1 step 4 (deploy) writes the heart (GH #987), which is not built yet
…
```

| Baseline Definition | Steps & Target Stores | Admission Status |
| --- | --- | --- |
| `ask-edit`, `ask-person`, `ask-judge`, `ask-position` | 1 leaf (`record`) | Admitted for user and Leader tasks |
| `practice-ratify` | `ratify` · `hat` (`record`) | Admitted for Board proposal ratifications |
| `concern-escalate` | `raise` (`record`) | Admitted when a concern is raised |
| `optimize-walk` | `walk` (`record`) | Admitted on the scheduled optimize pass |
| `ask-triage` | `offered` · `classify` · `plan` (`record`) | Refused: `offered` is handled inline prior to workflow admission |
| `change-deliver` | `candidate` (`record`) · `review` (`forge`) · `verdict` (`record`) · `apply` (`genome`) · `deploy` (`heart`) · `settle` (`record`) | Refused: step 4 writes `heart` |
| `deploy-observe` | `deploy` (`heart`) · `settle` (`record`) | Refused: step 0 writes `heart` |
| `secret-rotate` | `rotate` (`vault`) · `recorded` (`record`) | Refused: step 0 writes `vault` |
| `body-provision` | `provision` · `start` (`host`) · `lease` (`memory`) · `observed` (`record`) | Refused: hosts are provisioned via `hale dna body provision` |

### Execution Hierarchy & Attempt Dispatch

Every entity created by the workflow engine has a deterministic ID visible in `hale dna history`:

| ID Format | Entity |
| --- | --- |
| `t1` | Root workflow task |
| `t1.s0.b` | Child workflow task (`<parent>.<step>.<member-key>`) |
| `t1/wf1/s0` | Workflow step (`wf1` = workflow definition revision) |
| `t1/wf1/s0/a` | Leaf unit of work (`a` = member key) |
| `t1/wf1/s0/a/a0`, `t1/wf1/s0/a/a1` | Attempt `0` and retry attempt `1` |

When a step activates:
1. The engine writes `step.registered` listing every member of the step before dispatching any work.
2. Each leaf admits an attempt (`attempt.admitted`), which is routed according to its `requires` capability:
   - `organism`: Executed directly by the spine.
   - `edit`: Executed by the sandbox editor as a candidate Mutation.
   - `human` or `person`: Handed to a human assignee on the Board queue as a `case`.
   - `judgment`: Routed to an agent leg (`hale dna work`), falling back to a person.
   - `analysis`: Routed to a service performer, falling back to an agent leg.
3. When the assigned performer submits a result, the engine records `attempt.outcome` and `work.settled` (ignoring duplicate replies or replies from any actor other than the assigned performer).
4. Once every registered member of the step has settled, the step writes `step.completed` and activates the next step. When the final step completes, the execution writes `workflow.settled`.

### Retries, Failure Propagation, and Crash Recovery

- **Retries & failure bubbling**: If a leaf attempt fails and retries remain, the engine admits the next attempt (`a1`, `a2`, …). When a leaf exhausts its final attempt, its enclosing step fails, which fails the parent workflow up to the root and prevents any subsequent steps from activating. Any sibling attempts still running under a failed step are allowed to report their outcomes before the parent workflow settles as failed.
- **Cancellation**: Sending an internal `WorkflowCancelRequested` message fences the root execution and all descendant steps/attempts (no `hale dna` CLI command emits this message).
- **Crash recovery**: On startup, the spine reconstructs only unfinished executions from PostgreSQL under their original IDs. Settled attempts never re-run. If an attempt's claim was left open by a crashed performer:
  - Performers that track their own idempotency return their recorded outcome.
  - Performers that verify no side effect occurred receive the attempt again (`effect.redelivered`).
  - Performers that cannot determine whether an external effect ran pause the attempt until a human resolves it:

```sh
hale dna effect resolve attempt:t1/wf1/s0/a/a0 --outcome failed
```

## The Budget: The One Gate

Model spend across the entire organization is governed by a single policy in `dna/org/models.hl`:

```hale,fragment
fn org_budget() -> dna::BudgetPolicy {
    return dna::BudgetPolicy { window: "day", allowance_micros: 25000000 };
}
```

`allowance_micros` specifies the maximum spend in micro-dollars (`25000000` = $25.00 USD) per `window` (`"day"`, `"week"`, or `"none"` for a lifetime cap; `0` disables metering). On startup, the spine reconstructs current window spend from all `model.called` rows in the ledger.

Every model-backed operation must pass `Budget.admit` before calling a model:
- Leader intent planning and delegated Review verdicts
- Sandbox editor attempts
- Worker leg allowance requests (`hale dna work allowance`)
- Scheduled optimize passes

When the window allowance is exhausted, `Budget.admit` rejects new requests, writes `budget.exhausted` once for the window, and notifies the Board (pending Reviews fall back to human Board members):

```text
refused: budget exhausted (spent 25000000 of 25000000 micro-dollars this day in 41 call(s))
```

When budget remains, `Budget.admit` grants the smaller of the work unit's cost ceiling and the remaining window balance. Worker legs request an allowance before their first model call and stop calling models once the next call's known price would exceed their grant. Because grants are not pre-reserved across concurrent legs, parallel workers can collectively overshoot the window cap by at most their individual grants plus one call; any attempt that exceeds its own granted allowance is settled as failed with an overrun diagnostic while still recording all `model.called` rows.

See [Legs, Hands, and Voice](./legs.md) for model catalog configuration and [Shaping and Governing](./shaping.md) for delegated monetary budgets.

## The Body Lease and the Fence

Each record permits only one active host body at a time (or one per owner on a shared record). Before compiling or launching any child processes, `hale dna dev` / `hale dna run` acquires the `body` claim (`owner/<owner>` on shared records) in PostgreSQL with a 30-second TTL. If another checkout holds a live lease, PostgreSQL rejects the claim and the second host exits with code `3`. Because the holder identity is `user@host:<clone>`, a restarting process on the same checkout reclaims its own lease immediately.

Lease safety is enforced at three layers:
1. **Host tick checks**: The host verifies it still holds the lease at the start of every 1-second tick and immediately before spawning any child process.
2. **The body fence process**: Alongside the host runs a lightweight watchdog process (**body fence**) that renews the lease every 10 seconds with bounded timeouts independently of the host. If the lease is taken over by another holder, cannot be renewed within 5 seconds of expiration, or the parent host process exits, the fence immediately kills `dna/org`, the running application, and all descendant processes.
3. **Database epoch fencing (`fenced`)**: Each lease acquisition increments an epoch token passed to `dna/org`. PostgreSQL rejects any write carrying an expired or superseded epoch token with `fenced`.

Manage the body lease from the CLI:

```sh
hale dna body                    # display current lease holder and expiration
hale dna body claim --force      # forcibly take over the lease from a dead host
hale dna body release            # release this checkout's active lease
```

See [The Heart and the Body](./heart.md) for remote server provisioning (`hale dna body provision`).

## Reflex Routing & Core Loci

The automated health daemon (`dna/reflexes`) runs outside the spine as a separate process querying Prometheus. When `dna/reflexes` publishes an `instance.down` event on NATS, the spine records it as a `reading.recorded` row, the target fleet node restarts the crashed instance, and a second firing for the same target within the observation window is escalated by the spine into a `concern` workflow. See [Senses and Reflexes](./senses.md).

Internally, the `dna::Dna` locus is composed from domain-specific family loci in `dna/core/`, documented in [DNA, the Building Block](./dna.md).
