# Recurring and Cron Schedules

A **schedule** binds a recurring interval (`--every`) or UTC cron expression (`--cron`) to a catalog workflow definition (`--definition`) and a convening position (`--convener`). Schedules never store free-form prompt text; each occurrence admits a standard execution of the target workflow definition under a deterministic idempotency key (`sched:<id>@<time>`). The organization's `Scheduler` locus (`dna/core/scheduler.hl`) evaluates schedules on each wall-clock tick and records every declaration, admission, skip, miss, or refusal as an explicit row in the record.

```sh
# Declare a nightly UTC cron schedule for the ask-edit workflow
hale dna schedule declare nightly \
    --cron "0 2 * * *" \
    --definition ask-edit \
    --convener editor \
    --args '{"objective": "reconcile the records of the day"}'

# List all declared schedules and their occurrence counts
hale dna schedule
```

## Inspecting Schedules (`hale dna schedule`)

Running `hale dna schedule` reads the record directly and works offline from any clone, whether or not the organism is running.

On a newly grafted project before any practices are ratified:

```text
$ hale dna schedule
schedules: none declared (a ratified practice declares one, or `hale dna schedule declare`)
```

Once schedules are declared, each schedule prints its state (`live` or `paused`), cadence, target workflow definition, convener, occurrence counters, and most recent Task ID:

```text
schedules: <n>
  <id> [live|paused] every <n>ms|cron `<expr>` (UTC) — <definition>, convened by <position> · occurred N, skipped N, missed N · last <task>
```

- **occurred:** Total workflow executions admitted under this schedule's key.
- **skipped:** Occurrences skipped because the previous execution was still open.
- **missed:** Occurrences whose time window elapsed while the organism was stopped.
- **last:** The Task ID of the most recently admitted execution.

---

## Declaring a Schedule (`hale dna schedule declare`)

Schedules can be declared either by ratifying a practice that carries a schedule (see [Practice-Driven Schedules](#practice-driven-schedules)) or by requesting one via the CLI:

```sh
hale dna schedule declare nightly \
    --cron "0 2 * * *" \
    --definition ask-edit \
    --convener editor \
    --args '{"objective": "reconcile the records of the day"}'
```

| Flag / Argument | Description |
| --- | --- |
| `<id>` | Unique schedule identifier (must not be empty or contain `@` or spaces). |
| `--every <n>ms\|s\|m\|h\|d` | Fixed interval cadence (e.g., `90s`, `15m`, `1d`). Mutually exclusive with `--cron`. |
| `--cron <expr>` | Five-field UTC cron expression (`minute hour day-of-month month day-of-week`) supporting `*`, `a`, `a-b`, `*/n`, `a-b/n`, and comma-separated lists. Mutually exclusive with `--every`. |
| `--definition <id>` | Target workflow definition in the catalog (run `hale dna definitions` to list available definitions and required inputs). |
| `--convener <position>` | Position that convenes each execution (a bare name like `editor` is normalized to `position:editor`). |
| `--args <json>` | JSON object supplying all required inputs for `--definition` (defaults to `{}`). |
| `--as <who>` | Author of the request (defaults to your current user). |

### Request Lifecycle and Authorization

`hale dna schedule declare` writes a `schedule.requested` row under a fresh request ID and relays it over NATS JetStream (`nerves`) to the organization's `ScheduleRequested` subscription (which receives relayed subject messages automatically without requiring an explicit entry in `dna/org/main.hl`'s `bindings { }`).
- **When the organism is running:** The CLI waits up to 60 seconds for the matching `schedule.answered` row (`declared: true`, or `declared: false` with `why`) and prints:
  ```text
  schedule nightly declared: ask-edit on cron `0 2 * * *` (UTC), convened by position:editor
  ```
  or `schedule declare: refused: <why>`.
- **When no organism is running:** The CLI syncs the request to the record and exits:
  ```text
  schedule nightly asked of the organization in the record by <who> (<request>); it declares it when it hears it
  ```

**Who may declare a schedule:** Only a holder of `position:board` or a holder of the schedule's own `--convener` position may declare a schedule. Once Postgres memory records who holds the Board, requests from any user holding neither seat are answered with `declared: false`. If no Board holder or memory is configured yet, the requester is permitted as a fallback.

**Idempotency and updates:** Re-declaring an identical schedule is a no-op. Declaring an existing schedule ID with modified parameters writes a new `schedule.declared` row that supersedes the previous terms immediately.

### Validation and Refusal Conditions

Whether a schedule originates from `schedule declare` or a ratified practice, the organization validates it at declaration time (never waiting until the first trigger) and writes a `schedule.refused` row if any check fails:
- The `<id>` is empty or contains `@` or spaces.
- Neither `--every` nor `--cron` is provided, or both are provided.
- The `--cron` expression is syntactically invalid.
- The `--convener` is not formatted as `position:<name>`, is vacant, or lacks access to the stores written by the workflow definition.
- The `--definition` does not exist in the catalog, fails catalog expansion, or contains a step the organization cannot perform.
- The `--args` value is not a JSON object or omits a required input parameter of the definition. Inspect required inputs with `hale dna definitions`:
  ```text
  ask-edit@1  an ask: one change, prepared for review
    0. e  record
    occurs: skip while open, takes objective
  ```

---

## Convener Positions and Graph Projection

The **convener** is the position under whose authority each scheduled workflow execution is admitted (`workflow.admitted` records `convener: "position:<name>"`).

- **Built-in spine positions (`leader` and `editor`):** Have access to every store written by the organization's workflows and can convene schedules without a human holder.
- **Graph positions (`position:<name>`):** Convene through their active holders (a person or an owning organization's members) as recorded in the Postgres memory graph. If no memory is configured, graph conveners are refused.
- **Vacant positions:** A position with no ratified holder cannot convene schedules. The declaration is refused with a `hole` marker on `schedule.refused`:
  ```text
  nobody holds position:<p>: it convenes nothing until someone does (`hale dna fill <p> <holder>`)
  ```

When a schedule is declared, the memory graph also projects a `definition:<id>` node and a `convenes` edge from `position:<name>` carrying the cadence (`every 1d`, `every 90s`, `cron <expr>`). Running `hale dna show processes` lists each active rhythm at the end of its output as `<position> convenes <definition> <cadence>` ([Memory and the record](./memory.md)).

---

## Occurrence Lifecycle

The organization evaluates schedules on wall-clock ticks (`self.core.request_tick(…)`). Each occurrence is identified by its timestamp: an integer multiple of `--every` from the Unix epoch, or a matching UTC minute for `--cron` (evaluated once per minute). No occurrence at or before the schedule's declaration timestamp is ever due.

When a tick falls within an occurrence window, the scheduler submits the latest revision of `--definition` with `--args` under the deterministic ask ID `sched:<id>@<time>`. Because schedule state is reconstructed from the record on startup, re-evaluating an occurrence across restarts never creates duplicate executions.

| Occurrence Outcome | Recorded Row | Behavior |
| --- | --- | --- |
| **Admitted** | `workflow.admitted` (`request: "sched:<id>@<time>"`) | Starts a new workflow execution with `--args`. |
| **Skipped (in-flight)** | `schedule.skipped <id> {occurrence, task}` | Previous execution is still open and the workflow definition's overlap policy is `skip` (the default; definitions declared with `overlap` run concurrently—see [The spine](./spine.md)). |
| **Missed (downtime)** | `schedule.missed <id> {first, last, count, why}` | One or more occurrences elapsed while the organism was stopped. Missed occurrences are **never** run late; one summary row is written per contiguous downtime gap. |
| **Refused** | `schedule.refused <id> {occurrence, why}` | Workflow admission was refused at trigger time (for example, if the convener became vacant). |

---

## Pausing and Resuming (`pause` and `resume`)

```sh
hale dna schedule pause nightly
hale dna schedule resume nightly
```

`pause` and `resume` write `schedule.paused` and `schedule.resumed` rows in your name from any clone, taking effect on the organism's next tick:

```text
schedule nightly paused (schedule.paused; nothing occurs until it is resumed, and nothing is missed)
schedule nightly resumed (schedule.resumed)
```

While a schedule is `paused`, its occurrences neither fire nor count as `missed`. When resumed, the schedule continues from the next future occurrence (and across restarts, downtime misses are only counted after the `schedule.resumed` timestamp). Pausing an already paused schedule or resuming an already live schedule prints a notice and writes no row.

---

## Practice-Driven Schedules

### 1. Structured Schedule Metadata (`operating/optimize-cadence`)

A ratified practice can declare a schedule via its `schedule` metadata field. Every project grafted by the toolchain seeds the `operating/optimize-cadence` practice proposal:

```text
$ hale dna review
…
  operating — 7 seeded practice(s), each its own Review:
…
      k:fffbc971da28 — ratify the operating practice `operating/optimize-cadence`: walk the machinery on a cadence: the optimize pass is an execution of op…
```

The underlying receipt in memory contains the schedule specification:

```json
{
  "author": "org",
  "kind": "practice",
  "name": "operating/optimize-cadence",
  "provenance": "design",
  "schedule": "{\"id\": \"optimize\", \"every_ms\": 86400000, \"definition\": \"optimize-walk\", \"args\": \"{}\", \"convener\": \"position:leader\"}",
  "target": "org",
  "text": "walk the machinery on a cadence: the optimize pass is an execution of optimize-walk, convened by the leader once a day, which proposes one small change or records that the state is clean. The cadence is this practice's; a different one is an amendment the Board ratifies.",
  "toolchain": "0.21.0"
}
```

When the Board ratifies this practice (`hale dna review <id> approve` or `hale dna review operating approve`), the ratification step automatically declares the `optimize` schedule (`every 1d`, `optimize-walk`, convened by `position:leader`). If the schedule declaration itself is refused, the practice ratification still succeeds and records a separate `schedule.refused` row (`its schedule was refused (<why>)`).

On each daily occurrence, `optimize-walk` checks the budget gate (recording `optimize.refused` if the budget window is exhausted), inspects structural signals in the record, asks the Leader to propose at most one small organizational improvement, and records the result as `org.reviewed`. To change the cadence, propose a superseding amendment to `operating/optimize-cadence` ([Shaping and governing it](./shaping.md)).

### 2. Inline Key-Value Terms in Practice Text (`every=` and `step=`)

A practice can also declare a schedule directly inside its body text using `<key>=<value>` tokens:
- Including `every=1d step=source-check` in a practice's text declares—upon ratification—a schedule whose ID is the practice's name, running the `source-check` workflow every day, convened by the position the practice is bound to.
- Every occurrence launched by a practice passes the practice's identity (`practice`, its digest, and `name`) in its inputs. The workflow step reads any remaining parameters directly from the ratified practice text, so amending the practice updates the schedule's behavior without duplicating configuration. Upstream watches use this mechanism ([Upstream watches](./senses.md#upstream-watches)).

---

## Schedule Record Rows

| Row Kind | Written By | Payload Summary |
| --- | --- | --- |
| `schedule.requested` / `schedule.answered` | `hale dna schedule declare` / Organization | Schedule request terms and the organization's response (`declared: Bool`, `why: String`) keyed by request ID. |
| `schedule.declared` | Organization | Active schedule specification: `every_ms`, `cron`, `definition`, `args`, `convener`, `from`. |
| `schedule.refused` | Organization | Declaration or occurrence refusal (`why`, plus `hole` when a convener position has no holder). |
| `schedule.skipped` | Organization | Occurrence skipped because the prior execution is still open (`{occurrence, task}`). |
| `schedule.missed` | Organization | Contiguous range of occurrences missed while the organism was down (`{first, last, count, why}`). |
| `schedule.paused` / `schedule.resumed` | Operator (`hale dna schedule pause\|resume`) | Manual pause and resume events recorded in the operator's name. |

---

## Troubleshooting Schedules

| Diagnostic / Symptom | Cause & Resolution |
| --- | --- |
| `schedule declare: refused: <why>` (or `schedule.refused` row) | The declaration failed validation. Fix the reported condition (invalid cron, missing input in `--args`, unknown definition, or unauthorized requester) and re-run `hale dna schedule declare`. |
| `nobody holds position:<p>: it convenes nothing until someone does` | The convening position has no ratified holder. Assign one with `hale dna fill <p> <holder>` and ratify the hold Review, then re-run `hale dna schedule declare` (refused declarations are not retried automatically). |
| `who holds position:<p> is the graph's, and no memory is named to ask` | A non-builtin convener requires Postgres memory to resolve position holders. Run `hale habitat migrate` (or `hale dna memory migrate`). |
| `asked for the schedule, but the organization journaled no answer for <request> in time` | No running organism processed the request within 60 seconds. The `schedule.requested` row remains in the record and will be evaluated when the organism starts. |
| `missed N` in `hale dna schedule` | The organism was not running when those occurrences came due. Missed occurrences are never executed late; the schedule will fire at its next scheduled time. |
