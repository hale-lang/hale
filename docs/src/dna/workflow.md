# One Task, End to End

This chapter traces a single request from terminal submission (`hale dna task create`) through planning, sandbox editing, toolchain verification, review, fast-forward application, runtime observation, and either retention or rollback. It also covers human-assigned tasks (`ask-person` cases) and causal audit queries.

```sh
# Terminal 1: run the local supervisor
hale dna dev

# Terminal 2: submit a task, review the candidate, and inspect the audit trail
hale dna task create --as alice "document the Echo locus in main.hl"
hale dna review m1 approve --as alice --comment "looks good"
hale dna history m1
```

## Event Storage: Record vs. Ledger

Every state transition in the task lifecycle writes a durable row in PostgreSQL before any NATS message is sent. Depending on its role, each row kind is stored in one of two tables inside your organism's `dna_<identity>` schema:

| Store | Purpose | Contents |
| --- | --- | --- |
| **Record** (`record_rows`) | Long-term signed governance & audit log | Mutations, toolchain verification evidence, Reviews, verdicts, and runtime expression events |
| **Ledger** | Operational workflow & daily execution state | Intents, tasks, cases, workflow steps and attempts, effect claims, and model call receipts |

`hale dna history` queries both tables as a single causally linked sequence. See [Memory and the Record](./memory.md) for schema and role details.

## 1. Submitting an Intent

Submit a natural-language task from the terminal:

```sh
hale dna task create --as alice "document the Echo locus in main.hl"
```

`hale dna task create` writes a single `intent.requested` row to the ledger under your identity (`--as alice`) and does not publish directly to NATS. On its next 1-second tick, the active host body relays the unanswered row onto NATS JetStream, the spine admits the intent, and the CLI waits for the resulting task row (`task t1 born for intent i… […]`).

| Row Kind | Store | Description |
| --- | --- | --- |
| `intent.requested i…` | Ledger | Records the requested outcome, author (`--as`), and optional target locus (`--to`) |

Pass `--no-wait` to return immediately after writing `intent.requested`:

```text
intent i… requested in the record; the organism answers there (`hale dna status`)
```

If no body currently holds the record lease in PostgreSQL, running `hale dna task create` without `--no-wait` fails immediately before writing to the database:

```text
$ hale dna task create --as alice "document the Echo locus in main.hl"
hale dna: the organism is not running (no body holds the record's lease); start it with `hale dna run`, or ask with --no-wait to leave the ask in the record
```

## 2. Leader Planning & Workflow Admission

The spine consumes the relayed request from NATS and admits it at most once by intent ID. When `planned: true` is enabled in `dna/org/main.hl` (the default), the node acquires a distributed claim (`plan/i…`) in PostgreSQL so multiple nodes never plan the same intent twice, then invokes the Leader model to classify the work, change class, target file, and assignee.

| Row Kind | Store | Description |
| --- | --- | --- |
| `claim.taken plan/i…` | Ledger | Prevents concurrent nodes from planning the same intent twice |
| `intent.offered i…` | Ledger | Confirms the intent passed the Board, owner, and budget gates |
| `workflow.admitted t1` | Ledger | Creates the workflow execution, binding the definition, request, and Leader plan |
| `task.born t1` | Ledger | Records the one-line task summary displayed by `hale dna status` |
| `claim.released plan/i…` | Ledger | Releases the planning claim |

Based on the Leader's classification (or explicit CLI flags), the intent is admitted under one of four baseline workflow definitions:

- **`ask-edit`**: Code or documentation changes performed in a sandbox worktree.
- **`ask-person`**: Human tasks assigned to a specific person as a `case`.
- **`ask-judge`**: Judgment tasks created with `hale dna task create --judgment` (skips Leader planning).
- **`ask-position`**: Position-targeted tasks created with `hale dna task create --to position:<name>` (skips Leader planning).

If an admission gate rejects the request (for example, when the daily model budget is exhausted), the spine writes `intent.refused` and the CLI prints `refused: …`.

## 3. Step & Attempt Dispatch

The `ask-edit` workflow consists of one step (`s0`) containing one edit leaf (`e`). The workflow engine registers all members of the step, activates the step, and admits attempt `a0`:

| Row Kind | Store | Description |
| --- | --- | --- |
| `step.registered t1/wf1/s0` | Ledger | Registers all leaf/child members of step `s0` before execution begins |
| `step.activated t1/wf1/s0` | Ledger | Marks step `s0` active |
| `attempt.admitted t1/wf1/s0/e/a0` | Ledger | Admits attempt `0` for leaf `e` |
| `effect.requested attempt:t1/wf1/s0/e/a0` | Ledger | Acquires the execution claim for attempt `a0` |

Execution and attempt IDs are deterministic in PostgreSQL, so a process restart resumes the exact same attempt ID without re-running completed work. See [The Spine](./spine.md).

## 4. Sandbox Editing

The organization's built-in editor performs the `edit` leaf inside an isolated Git worktree (`.hale/dna/`) using a restricted tool grant of four operations: `read`, `edit`, `format`, and `check`.

| Row Kind | Store | Description |
| --- | --- | --- |
| `mutation.requested m1` | Record | Binds the mutation to the requesting attempt ID so redeliveries never duplicate edits |
| `mutation.proposed m1` | Record | Records the mutation's task, change class, objective, target, and base commit |
| `effect.requested` / `effect.result` `worktree.open:m1` | Ledger | Opens the sandbox Git worktree once |
| `mutation.worktree m1` | Record | Records the sandbox worktree path and base HEAD |
| `mutation.located m1` | Record | Records the target files and tool grant |
| `model.called m1/a0` | Ledger | Logs each LLM call, token cost, and content-addressed prompt receipt |
| `effect.requested` / `effect.result` `commit:m1:…` | Ledger | Commits the edited files to the candidate branch in the sandbox worktree |
| `mutation.candidate m1` | Record | Records the candidate commit SHA |

If the edited code fails `hale check`, the editor retries under a new attempt ID (`a1`, `a2`, …) with the compiler diagnostics included in its prompt.

## 5. Toolchain Verification & Evidence Receipts

Before any reviewer is asked to evaluate `m1`, the spine runs the full verification suite against the candidate worktree. Each check stores its full output as a content-addressed receipt in PostgreSQL and records the exit code and SHA-256 digest in `record_rows`:

| Row Kind | Store | Description |
| --- | --- | --- |
| `evidence.base` | Record | Cuts the base topology artifact from the current genome HEAD |
| `evidence.fmt`, `evidence.check`, `evidence.verify`, `evidence.test` | Record | Runs `hale fmt --check`, `hale check`, `hale verify`, and `hale test` on the candidate |
| `evidence.fleet` | Record | Verifies composition of every fleet declared in the workspace against the candidate |
| `evidence.rollback` | Record | Rehearses stepping the worktree back to the base commit and forward again |
| `evidence.diff` | Record | Computes the structural semantic diff between base and candidate |
| `evidence.magnitude` | Record | Scores the change magnitude across structural axes |

## 6. Autonomy Boundary & Review Routing

Once verification completes, the organization evaluates the candidate against the `AutonomyBoundary` grant configured in `dna/org/main.hl`:

```hale,fragment
            boundary: dna::AutonomyBoundary {
                child: "refproj",
                grant: dna::Grant { child: "refproj", classes: "refactor docs", max_magnitude: 4, review: "pre" }
            },
```

- **Delegated to the Leader**: Changes whose class (`refactor` or `docs`) and magnitude (`<= 4`) fall inside the grant, provided they do not touch `dna/org/law.hl`, widen effect classes, cross ownership boundaries, or modify `dna/org/` itself.
- **Escalated to the Board**: All `application` changes, changes exceeding `max_magnitude`, changes touching laws, effects, or `dna/org/`, and any change when no model is available.

| Row Kind | Store | Description |
| --- | --- | --- |
| `mutation.<disposition> m1` | Record | Boundary decision: `review`, `stage`, `escalate`, `release`, or `deny` |
| `review.requested review:m1` | Record | Opens the Review with the pinned candidate SHA, required authority, diffs, and evidence digests |
| `review.routed review:m1` | Record | Identifies required signers from the organizational graph in PostgreSQL |

Because the edit leaf's job is to produce a verified candidate in review, the `ask-edit` workflow execution settles as soon as the Review is opened:

| Row Kind | Store | Description |
| --- | --- | --- |
| `attempt.outcome t1/wf1/s0/e/a0` | Ledger | Records the attempt outcome |
| `work.settled`, `step.completed` | Ledger | Marks the leaf and step `s0` settled |
| `workflow.settled t1` | Ledger | Marks workflow execution `t1` complete |

Run `hale dna review` to see pending Reviews, or `hale dna review m1` to inspect the candidate's source diff, structural semantic diff, and verification receipt table. See [The Head and the Face](./head.md).

## 7. Recording a Review Verdict

If the Review is routed to the Leader, the Leader evaluates the three views (source diff, semantic diff, and evidence) via a recorded model call. If the Review requires human authority, submit your verdict from the CLI:

```sh
hale dna review m1 approve --as alice --comment "fine"
```

The spine verifies three conditions before settling the Review:
1. The verdict matches the candidate commit pinned by the Review (or explicitly supplied via `--digest`).
2. The signer (`--as alice`) holds the required position authority.
3. The signer is not the author of the candidate commit.

| Row Kind | Store | Description |
| --- | --- | --- |
| `review.verdict m1` | Record | Records the submitted verdict under the signer's identity |
| `review.settled m1` | Record | Marks the Review decided (e.g., `approve by alice`) |
| `review.refused m1` | Record | Recorded instead of `settled` when a verdict fails authority, digest, or self-review checks |
| `review.reasoned m1` | Record | Records the signer's `--comment` or the Leader's reasoning |

Other verdict options:
- **`revise`** / **`reject`**: Settles the Review (`mutation.revise` or `mutation.rejected`) without modifying the genome or restarting the application.
- **`abstain`**: Logs the abstention row while leaving the Review open.

## 8. Applying the Candidate Commit

When a Review settles with `approve`, the spine verifies that:
- The sandbox worktree HEAD still matches the reviewed candidate commit.
- The genome's HEAD still matches the base commit the Review was evaluated against.
- The genome checkout has no uncommitted modifications.

If all checks pass, the spine fast-forwards the genome to the candidate commit under an effect claim keyed by the candidate digest (preventing duplicate merges on retry):

| Row Kind | Store | Description |
| --- | --- | --- |
| `effect.requested` / `effect.result` | Ledger | Guards the fast-forward merge by candidate digest |
| `mutation.applied m1` | Record | Confirms the genome HEAD is now at the candidate commit |
| `expression.restart_requested m1` | Record | Requests runtime expression (`apply <candidate> seed <seed> fitness <signals>`) |

If any pre-merge check fails, nothing is applied and `mutation.refused` is recorded with the exact cause (`candidate moved after review`, `the genome moved since the review`, or `the genome has uncommitted changes`). If the working tree had uncommitted changes, commit or stash them and run `hale dna review m1 approve` again.

## 9. Expressing, Observing, and Retaining the Change

Under `hale dna dev`, the host supervisor rebuilds the application binary, stops the previous process, launches the new binary, and monitors it for the observation window (`--observe`, 15 seconds by default):

```text
hale dna dev: m1 requests a restart (apply … seed . fitness …)
hale dna dev: expression restarted (pid …) as …
hale dna dev: m1 observed healthy for 15s as …
```

| Row Kind | Store | Description |
| --- | --- | --- |
| `expression.restarted m1` | Record | Records the new process PID, topology shape, and build digest |
| `observation.requested m1` | Record | Host's health report at the end of the observation window |
| `expression.observed m1` | Record | Spine's acknowledgment of the observation report |
| `pressure.remeasured m1` | Ledger | Re-evaluates fitness signals declared on the mutation proposal |
| `mutation.retained m1` | Record | Marks the mutation permanently kept (`observed healthy as …`) and cleans up the worktree |

When running under `hale dna run` with a multi-node fleet configured, approval emits a `fleet.deploy` row instead, and the observation window spans every fleet instance affected by the edited seed. See [The Heart and the Body](./heart.md).

## 10. Automatic Rollback on Observation Failure

If the new application process exits or fails health checks during the observation window, the spine automatically rolls the genome back to the mutation's base commit and requests a restart of the previous binary:

```text
hale dna dev: m1: the expression exited (…) inside the observation window
```

| Row Kind | Store | Description |
| --- | --- | --- |
| `expression.observed m1` | Record | Records the crash or observation failure |
| `pressure.remeasured m1` | Ledger | Measures fitness signals against the failed window |
| `effect.requested` / `effect.result` `rollback:m1:…` | Ledger | Resets the genome branch back to the base commit once |
| `mutation.rolled_back m1` | Record | Confirms the genome is restored to the base commit |
| `expression.restart_requested m1` | Record | Requests restarting the previous known-good binary (`rollback <base> …`) |

All candidate commits, verification receipts, and rollback rows remain in the record and can be inspected via `hale dna status`:

```text
mutations:  1 (none applies before a human's verdict on the exact candidate)
  m1 [retained] …: … · task t1 · candidate …
```

## Human Tasks (`ask-person` Cases)

When the Leader classifies an intent as human work, the spine admits it under `ask-person`, creating a handed **case** task (`t1.s0.p`) assigned to the designated person:

| Row Kind | Store | Description |
| --- | --- | --- |
| `case.admitted t1.s0.p` | Ledger | Records parent attempt, objective, assignee, obligation, active acceptance practice, and evidence requirement |
| `task.born t1.s0.p`, `task.handed t1.s0.p` | Ledger | Tracks the open case in `hale dna status` and `hale dna board` |

A human case waits indefinitely across restarts until its assignee resolves it:

| CLI Command | Ledger Rows Written | Behavior |
| --- | --- | --- |
| `hale dna task done <id> --as <user> --note "<text>"` | `task.done` | Completes a case that does not require evidence; only the current assignee may close it |
| `hale dna receipt file <file> --as <user>` + `hale dna task done <id> --as <user> --evidence sha256:…` | `completion.linked`, `task.done` | Files a receipt and closes a case whose acceptance practice requires evidence |
| `hale dna task authorize <id> --exception "<why>" --as <other>` + `hale dna task done <id> --as <user> --exception "<why>" --authorized-by <other>` | `exception.authorized`, `completion.excepted`, `task.done` | Closes an evidence-requiring case under an exception authorized by someone other than the assignee |
| `hale dna task decide <id> --decided-by <who> --via <channel> --evidence sha256:… --as <user>` | `decision.reported`, `task.done` | Records an external decision (e.g., email or meeting) with its receipt |
| `hale dna task reassign <id> --to <new> --as <user>` | `task.reassigned` | Transfers an open case to another person |
| `hale dna retire <user> --to <successor>` | `person.retired` | Transfers all open cases held by `<user>` to `<successor>` and blocks future assignments to `<user>` |

The face offers the same two steps for a case that needs no evidence. They are the head's rows `Commands::take_task` and `Commands::complete_task`, the face's **Take this Task** and **Report done**:

- **Take this Task** makes an unassigned case yours (`task.reassigned` from no one to you).
- **Report done** closes a case that is yours, or unassigned, with what you did as text (`task.done`, `by <you>: <text>`).

### With no model

An organism whose models catalog is empty still works; this is the solo path.

- **What counts as no model:** none is mapped (`hale dna models category`), and the catalog reaches none: its hosted backends have no key and its local server is not running. A canned or recorded model counts as a model.
- **Asks:** with no model, every ask is a person's job. No plan is asked, and the ask is handed unassigned, never given to the editor.
- **Leg work:** a judgment, or Work routed to a position, is a leg's only while there is a model. With none it is a person's case.
- **Timers:** no timer moves work to a leg.

A person takes each case and reports it done as text, and its execution settles.

Examples of closing a case with evidence or an authorized exception:

```sh
# Close with a filed evidence receipt
hale dna receipt file report.pdf --as alice
hale dna task done t1.s0.p --as alice --evidence sha256:…

# Close with an exception authorized by another team member
hale dna task authorize t1.s0.p --exception "the vendor sent no report" --as bob
hale dna task done t1.s0.p --as alice --exception "the vendor sent no report" --authorized-by bob

# Record an external decision
hale dna task decide t1.s0.p --decided-by carol --via email --evidence sha256:… --as alice
```

> [!NOTE]
> `hale dna retire <user>` is rejected if `<user>` holds any open tasks and `--to <successor>` is omitted, ensuring pending human work is never orphaned. Any command run with `--as <retired-user>` is also rejected. For agent-performed judgment tasks (`hale dna task create --judgment`), see [Legs, Hands, and Voice](./legs.md).

## Auditing History & Board Reports

- **`hale dna history <entity>`**: Walks the causal graph in PostgreSQL starting from an intent (`i…`), task (`t1`), mutation (`m1`), Review, or attempt (`t1/wf1/s0/e/a0`). Prints the record header, total matching rows, model call cost breakdown by position and backend, and every causally linked row. When given an attempt ID, it also prints the full prompt sent for each model call from its receipt. Runs offline from any clone.
- **`hale dna status`**: Displays a one-line summary for every active task and mutation.
- **`hale dna report`**: Generates and records a Board summary (`report r…`) covering all activity since the previous report:

```text
$ hale dna report
report r39 filed: since #0: proposed 0 · reviewed 16 · applied 0 · retained 0 · rolled back 0 · rejected 0 · escalated 16 · pressure 0 · proposals 0 · model calls 0 (0 µ$) · settled: none
```

## Restart & Recovery Behavior

Because workflow state lives in PostgreSQL rather than process memory, restarting `hale dna dev` or `hale dna run` resumes all unfinished workflow executions under their existing IDs without re-admitting intents or re-running settled attempts.

- **Unrecovered intents**: Any intent that was offered (`intent.offered`) but never admitted before the process stopped is marked once as `intent.unrecovered` and is not re-offered.
- **Indeterminate external effects**: If a worker crashed while executing an external side effect whose adapter cannot verify completion, the attempt pauses until an operator resolves it manually:

```sh
hale dna effect resolve <key> --outcome ok|failed
```

See [The Spine](./spine.md) for the complete recovery protocol.
