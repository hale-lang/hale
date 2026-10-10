# One task, end to end

This chapter follows one request from the terminal to a change that is
kept, or rolled back, with the record's rows beside each step. It then
covers the cases where the work is a person's, and how you read it all
back.

You need an organism running: `hale dna dev` in one terminal
([Getting started](./getting-started.md)). The commands below go in
another.

## Where the rows go

Every step below writes rows. Each row kind has one home:

- **the record**: `refs/dna/journal` in your repository, one commit per
  row. Mutations, evidence, Reviews and expressions live here, synced
  to every clone.
- **the ledger**: the day's work (intents, tasks, the workflow
  engine's steps and attempts, effects, model calls) in memory.

Until you run `hale dna ledger adopt`, the record takes every row,
ledger kinds included. The tables below name each row's home once the
ledger is adopted. `hale dna history` reads both as one sequence, so
nothing below depends on which one you are on.
[Memory and the record](./memory.md) has the two memories.

## 1. Ask

```sh
hale dna task create --as alice "document the Echo locus in main.hl"
```

The verb writes one row in your name and does nothing else: it never
publishes. The body beside the organization relays the row onto the
nerves on its next tick, and the organization answers in the record.
The verb waits for that answer and prints the Task it made:
`task t1 born for intent i… […]`.

| row | home | what it is |
| --- | --- | --- |
| `intent.requested i…` | ledger | your ask: the outcome, who asked, and `--to` when you named a locus |

`--no-wait` returns once the row is written, and `hale dna status`
shows the answer later:

```text
intent i… requested in the record; the organism answers there (`hale dna sync`, then `status`)
```

With no organism running in
this clone and no remote to reach one through, the ask is refused
before anything is written:

```text
$ hale dna task create --as alice --no-wait "document the Echo locus in main.hl"
hale dna: the organism is not running here (no body in this clone holds the record's lease) and the repository has no remote to reach one through; start it with `hale dna run`
```

## 2. Planned, and admitted as a workflow

The organization reads the ask off the nerves and admits it once, by
its id. The generated organization plans first (`planned: true` in
`dna/org/main.hl`): the Leader says what kind of work the ask is, which
class of change, which file, and for whom. With memory named, the node
claims the plan before it asks, so two nodes never plan one ask twice.

| row | home | what it is |
| --- | --- | --- |
| `claim.taken plan/i…` | ledger | this node plans the ask (only with memory) |
| `intent.offered i…` | ledger | the ask passed the Board's gate, the owner's and the budget's |
| `workflow.admitted t1` | ledger | the execution: its definition, the ask as its request, the Leader's word bound in its inputs |
| `task.born t1` | ledger | the one-line summary the tooling lists |
| `claim.released plan/i…` | ledger | the plan's claim given back |

A change is admitted under the `ask-edit` definition, a person's job
under `ask-person`, `task create --judgment` under `ask-judge`, and
`task create --to position:<name>` under `ask-position`, with no plan
asked. An ask the gates refuse (an exhausted budget, for
one) is `intent.refused`, and the verb prints `refused: …`.

## 3. The engine runs the step

`ask-edit` is one step with one leaf, the edit. The engine registers
the step, activates it, and admits one attempt at the leaf.

| row | home | what it is |
| --- | --- | --- |
| `step.registered t1/wf1/s0` | ledger | the step's whole set of members, before anything runs |
| `step.activated t1/wf1/s0` | ledger | the step may run |
| `attempt.admitted t1/wf1/s0/e/a0` | ledger | attempt 0 at the leaf `e` |
| `effect.requested attempt:t1/wf1/s0/e/a0` | ledger | the one claim on running that attempt |

The ids are the record's, not a process's: after a restart the same
attempt has the same id. [The spine](./spine.md) has the engine, its
retries and its failures.

## 4. The editor proposes a change

An edit leaf is performed by the organization's own editor, in a
sandbox worktree, under a grant of four tools: read, edit, format and
check, pointed at that worktree.

| row | home | what it is |
| --- | --- | --- |
| `mutation.requested m1` | record | which attempt asked for the Mutation, so a redelivery never edits twice |
| `mutation.proposed m1` | record | the change: its Task, class, objective, target, base commit |
| `effect.requested` / `effect.result` `worktree.open:m1` | ledger | the worktree opened, once |
| `mutation.worktree m1` | record | the sandbox, at the genome's head |
| `mutation.located m1` | record | the files found, and the grant they were found under |
| `model.called m1/a0` | ledger | each model call, with its evidence and cost; the prompt is a receipt |
| `effect.requested` / `effect.result` `commit:m1:…` | ledger | the candidate committed in the worktree |
| `mutation.candidate m1` | record | the candidate commit |

When the edit does not check, the editor tries again with the
diagnostics in the prompt, each try under its own attempt id.

## 5. Evidence

The candidate is proven before anyone is asked. Each step's output is
a receipt under `refs/dna/receipts/`, named by its hash; the row
carries the exit code and the digest.

| row | home | what it is |
| --- | --- | --- |
| `evidence.base` | record | the base artifact, cut from the genome at the same moment |
| `evidence.fmt`, `evidence.check`, `evidence.verify`, `evidence.test` | record | the toolchain on the candidate, your tests included |
| `evidence.fleet` | record | every fleet plan the workspace declares, composed with the candidate |
| `evidence.rollback` | record | the worktree stepped back to the base and forward again |
| `evidence.diff` | record | the semantic diff, base to candidate |
| `evidence.magnitude` | record | how big the change is, axis by axis |

## 6. The boundary decides who reviews

The grant the Board gave the Leader is in `dna/org/main.hl`:

```hale,fragment
            boundary: dna::AutonomyBoundary {
                child: "refproj",
                grant: dna::Grant { child: "refproj", classes: "refactor docs", max_magnitude: 4, review: "pre" }
            },
```

A `docs` or `refactor` change inside that grant is the Leader's to
review. A change outside it, an `application` change, one that
touches the law, widens effects or crosses ownership, and a change to
the organization itself are the Board's.

| row | home | what it is |
| --- | --- | --- |
| `mutation.<disposition> m1` | record | what the boundary decided: `review`, `stage`, `escalate`, `release` or `deny` |
| `review.requested review:m1` | record | the Review: the question, the authority it requires, the pinned candidate, the evidence, the diffs |
| `review.routed review:m1` | record | who must sign, from the graph in memory; with no memory named, the fallback and why |

The ask's leaf is done once its candidate is in review, so the
execution settles now; the Review goes on without it.

| row | home | what it is |
| --- | --- | --- |
| `attempt.outcome t1/wf1/s0/e/a0` | ledger | the attempt's outcome |
| `work.settled`, `step.completed` | ledger | the leaf settled; every member of the step has |
| `workflow.settled t1` | ledger | the execution is done |

`hale dna review` now lists `m1`, with who it needs, the candidate and
the evidence codes. `hale dna review m1` renders its three views: the
source diff, the semantic diff and the evidence table.
[The head and the face](./head.md) reads a Review in full.

## 7. A verdict

When the Review is the Leader's, the Leader reads the same three views
and decides, with a model call on the record. When it is yours:

```sh
hale dna review m1 approve --as alice --comment "fine"
```

Like the ask, the verb writes a row and waits for the Review's
answer: `review m1 settled: approve by alice`. The Review checks the
verdict names the candidate the Review pinned (or the one you give
with `--digest`), that its authority meets what the Review requires,
and that you are not the candidate's author.

| row | home | what it is |
| --- | --- | --- |
| `review.verdict m1` | record | your verdict, in your name |
| `review.settled m1` | record | the verdict that decided it, e.g. `approve by alice` |
| `review.refused m1` | record | instead of settling: why a verdict was not admitted |
| `review.reasoned m1` | record | the deciding verdict's comment, or the Leader's reasoning |

`revise` and `reject` settle the Review and touch nothing:
`mutation.revise` or `mutation.rejected`, and the genome and the
running application stay as they were. `abstain` is recorded and
leaves the Review open.

## 8. Apply

An approval is the apply. The organization checks that the worktree's
head is still the candidate, that the genome is still the base the
Review was against, and that the genome has nothing uncommitted; then
it fast-forwards the genome to the candidate, under an effect keyed by
the candidate itself, so a retried apply never commits twice.

| row | home | what it is |
| --- | --- | --- |
| `effect.requested` / `effect.result` | ledger | the apply, keyed by the candidate |
| `mutation.applied m1` | record | the genome is at the candidate |
| `expression.restart_requested m1` | record | `apply <candidate> seed <seed> fitness <signals>`: express it |

When a check fails nothing is applied, and the record says which:
`mutation.refused` with `candidate moved after review`,
`the genome moved since the review`, or
`the genome has uncommitted changes`. After the last, commit or stash,
and approve again. `git log` shows an applied change as an ordinary
commit.

## 9. Express, watch, keep

Under `hale dna dev`, the host rebuilds the application, stops the old
process, starts the new one and records it. It then watches for the observation window (`--observe`, 15 seconds by
default) and writes what it saw as a row, which it relays until the
organization answers. In the host's terminal:

```text
hale dna dev: m1 requests a restart (apply … seed . fitness …)
hale dna dev: expression restarted (pid …) as …
hale dna dev: m1 observed healthy for 15s as …
```

| row | home | what it is |
| --- | --- | --- |
| `expression.restarted m1` | record | the new process's shape and build |
| `observation.requested m1` | record | the host's report on the window |
| `expression.observed m1` | record | the organization took the report |
| `pressure.remeasured m1` | ledger | the fitness signals the proposal declared, measured again |
| `mutation.retained m1` | record | `observed healthy as …`: kept |

Under `hale dna run` with a fleet, the same approval is a `fleet.deploy`
row that every node answers for its own instances, and the window
covers every instance the change touched.
[The heart and the body](./heart.md) covers the fleet and deployment
commands.

## 10. Or rolled back

Anything but a healthy window rolls the change back. The genome goes
back to the Mutation's base, through the gateway, and the old
application is asked for again:

```text
hale dna dev: m1: the expression exited (…) inside the observation window
```

| row | home | what it is |
| --- | --- | --- |
| `expression.observed m1` | record | what went wrong |
| `pressure.remeasured m1` | ledger | the fitness signals, measured against what the window saw |
| `effect.requested` / `effect.result` `rollback:m1:…` | ledger | the reset to the base, once |
| `mutation.rolled_back m1` | record | the genome is back at the base |
| `expression.restart_requested m1` | record | `rollback <base> …`: the old application back |

The Mutation, its candidate, its attempts and its evidence all stay in
the record. `hale dna status` lists it with its state:

```text
mutations:  1 (none applies before a human's verdict on the exact candidate)
  m1 [retained] …: … · task t1 · candidate …
```

A Mutation reads `retained`, `rolled_back`, `rejected`, or, while it is
in flight, its disposition.

## When the work is a person's

When the Leader's plan says an ask is a person's job, it is admitted
under `ask-person`, and its one leaf becomes a **case**: a handed Task
of its own, `t1.s0.p`, for the person the plan named.

| row | home | what it is |
| --- | --- | --- |
| `case.admitted t1.s0.p` | ledger | the case: its parent attempt, objective, assignee, obligation, the acceptance practice in force, whether evidence is required |
| `task.born t1.s0.p`, `task.handed t1.s0.p` | ledger | the rows `status` and `board` read |

Nothing settles a case but its person. It waits through any restart,
and the step above it waits with it. The case appears on
`hale dna board` and in `hale dna status`. The person closes it:

```sh
hale dna task done t1.s0.p --as alice --note "wrote the section"
```

That writes `task.done` in their name and prints
`task t1.s0.p done by alice: wrote the section — human-reported, no evidence linked`.
Only the assignee can close a handed Task; anyone else is told to
reassign it first. The completion is the attempt's outcome, and the
engine settles the leaf, the step and the execution as in step 6.

When the acceptance practice bound at hand-off requires evidence, a
note alone is refused. The person links a receipt, filed first:

```sh
hale dna receipt file report.pdf --as alice
hale dna task done t1.s0.p --as alice --evidence sha256:…
```

That writes `completion.linked` before `task.done`. Or someone else
authorizes an exception in their own name, and the person closes the
Task with exactly that exception:

```sh
hale dna task authorize t1.s0.p --exception "the vendor sent no report" --as bob
hale dna task done t1.s0.p --as alice --exception "the vendor sent no report" --authorized-by bob
```

The first writes `exception.authorized`; the second
`completion.excepted`, then `task.done`. The assignee cannot authorize
their own exception.

A decision someone outside made, by mail or in a meeting, is reported
by the assignee with its evidence, and closes the Task when the bound
practice accepts it:

```sh
hale dna task decide t1.s0.p --decided-by carol --via email --evidence sha256:… --as alice
```

That writes `decision.reported`. The rest move a Task between people:

- `hale dna task reassign t1.s0.p --to bob --as alice` writes
  `task.reassigned` and prints `task t1.s0.p reassigned from alice to bob by alice`.
  The Task stays handed.
- `hale dna retire alice --to bob` moves every handed Task alice holds
  to bob and records `person.retired`. From then on no new work reaches
  alice. It is refused while alice holds work and names no successor:
  pending work is accounted for, never dropped.

Each of these rows is in the name of whoever ran the verb (`--as`),
and each is refused when that person has retired.

A judgment (`task create --judgment`) goes to a leg instead: an agent
claims the attempt with `hale dna work` and hands the outcome back.
[Legs, hands and voice](./legs.md) follows that path.

## Reading it back

`hale dna history <entity>` walks the record by causal links from one
id: an ask (`i…`), a Task (`t1`), a Mutation (`m1`) or a Review. It
prints the record's header, the number of rows it found, what the
entity's model calls cost by position and by backend, and then each row:
its number, kind, entity and the start of its body. Given one attempt's
id (`t1/wf1/s0/e/a0`), it also prints what each of that attempt's model
calls sent, from the receipt. It works offline, from any clone.

`hale dna status` has one line per Task and per Mutation, with its
state.

`hale dna report` files a summary of everything since the last report,
as a row for the Board:

```text
$ hale dna report
report r39 filed: since #0: proposed 0 · reviewed 16 · applied 0 · retained 0 · rolled back 0 · rejected 0 · escalated 16 · pressure 0 · proposals 0 · model calls 0 (0 µ$) · settled: none
```

That is a new organism's first report: twenty-three Reviews asked of the
Board, nothing proposed yet.

## After a restart

The record is the execution. An organization restarted over it picks
up every execution that is not finished, under the same ids: nothing
admitted twice, no step run twice, a case still its person's. An ask
offered but never admitted before the stop is noted once as
`intent.unrecovered` and never offered again. An effect whose outcome
no one can establish waits for a person:
`hale dna effect resolve <key> --outcome ok|failed`.
[The spine](./spine.md) has the promises in full.
