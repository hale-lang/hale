# The workflow

Everything the organization does for you is one execution of a
workflow, run by one engine, written into the record as it happens.
An ask from the terminal, a schedule firing at two in the morning, a
job handed to a person, an edit the editor makes: the same shapes, the
same rows, the same rules on a restart. This chapter is what those
shapes are and what they promise. You do not need it to use the
organization; you need it the first time you ask *what is it doing
now, and what happens if I stop it*.

## An ask becomes an execution

```text
$ hale dna ask document the chat server in main.hl
task t1 born for intent i1a08c0786c5 [pending]
```

Three things happened. The membrane admitted the intent
(`intent.offered`). Where the organization plans, the Leader read it
first and said what kind of work it is and for whom; while that word
is out the state is `[planning]`, and an intent offered again
meanwhile is neither gated nor journaled twice. Then the ask was
admitted as a workflow: `workflow.admitted` names the Task (`t1`), the
definition it runs under, and the ask as its request, with the
Leader's word bound into its inputs; `task.born` is the one-line
summary the tooling lists. From here the engine owns it.

The two definitions an ask runs under are the organization's own: one
edit leaf for a change to the code, one human leaf for a person's job.
Which one, and for whom, is the Leader's plan; without a Leader every
ask is an edit.

## Definitions

A definition is written in code, in `dna/org/workflows.hl`, and bound
whole before anything runs. It is ordered steps, and each step names
the **one store it writes** — `record`, `forge`, `genome`, `heart`,
`graph`, `nerves`, `memory`, `vault` or `host` — one word per step, so
the words are also the step count. A step is where a fact is written:
reading something, or waiting for another writer, belongs to the step
whose fact it serves. A step is a set of members; a member is a **leaf** (one unit of work, with an objective,
the capability words it requires, and how many attempts it may take)
or a **child** (another definition, run as its own execution under
this step). The canonical example the engine's proofs use:

```hale,fragment
let c = self.core.catalog;
c.define("close-month", 1, "record record", "close the month");
c.leaf("close-month", 1, 0, "a", dna::WorkRequest { objective: "reconcile the bank feed", requires: "analysis" }, 1);
c.child("close-month", 1, 0, "b", "collect-receipts", 1);
c.leaf("close-month", 1, 1, "d", dna::WorkRequest { objective: "post the journal entries", requires: "analysis" }, 1);
c.define("collect-receipts", 1, "record record", "");
c.leaf("collect-receipts", 1, 0, "b1", dna::WorkRequest { objective: "list missing receipts", requires: "analysis" }, 1);
c.child("collect-receipts", 1, 1, "c", "chase-supplier", 1);
c.define("chase-supplier", 1, "record", "");
c.leaf("chase-supplier", 1, 0, "c1", dna::WorkRequest { objective: "email the supplier", requires: "analysis" }, 2);
let id = self.core.run_workflow(dna::WorkflowAsk { id: "month-end", definition: "close-month", revision: 1, from: "books" });
```

`close-month` has two steps: the first runs leaf `a` and the child
`collect-receipts` together; the second runs `d` only after both have
settled. The child's second step runs a grandchild. `c1` may be tried
twice. A definition binds whole or is refused: a name that does not
exist, a child that would recurse, a member key outside `[a-z0-9-]+`,
a step that declares no store or two (`record+forge`), a step on a part
not built yet (the heart, the vault), or a tree wider than the limits
(depth 8, 32 steps, 32 members to a step, 8 attempts to a leaf, 256
units of work) is a refusal at admission, in the record
(`workflow.refused`), with nothing started.

## The catalog

DNA ships a catalog, and `hale dna new` writes `dna/org/workflows.hl`
to return it plus your own; `hale dna definitions` lists it. Each
baseline definition is a chain in which every step writes one store:

| id | steps (store) | runs today |
| --- | --- | --- |
| `practice-ratify` | ratify · hat (record) | every knowledge proposal the Board decides |
| `concern-escalate` | raise (record) | every concern |
| `optimize-walk` | walk (record) | the optimize pass |
| `ask-triage` | offered · classify · plan (record) | refused: the ask path still admits `ask-edit` / `ask-person` after the leader plans |
| `change-deliver` | candidate (record) · review (forge) · verdict (record) · apply (genome) · deploy (heart) · settle (record) | refused: the heart is not built |
| `deploy-observe` | deploy (heart) · rollback (genome) · settle (record) | refused: the heart is not built |
| `secret-rotate` | rotate (vault) · recorded (record) | refused: the vault is not built |
| `body-provision` | provision · start (host) · lease (memory) · observed (record) | refused: a person provisions a host with `hale dna body provision` |

The steps the organization performs itself are leaves that require
`organism`: the engine hands each to the organization, which writes the
step's rows and reports it — at once, or when what it waits on arrives
(the leader's answer, memory catching up). When the Board decides a
proposal — a practice you proposed, one seeded at the record's birth,
the declared purpose, a concern's — the decision becomes an execution you
can follow in `history`: the ratification (or the decline), then the
hat that now reads it (`practice.read`). A concern is one step: its row
names the parent it is routed to, and at the threshold it is proposed.

**The record is the floor; the ledger is the runtime.** Every step is a
handful of rows of the day's work. On a git-backed record — what a new
organization starts on, one person working alone — each row is a commit,
so an execution costs tens of them: the Board deciding a repository's 35
seeded proposals waits minutes for the ratifications, where an adopted
ledger (`hale dna ledger adopt`, [The record](./record.md)) takes the same
rows in about a second. Run an organization with its ledger adopted.
`hale dna review <group> approve` writes every verdict of the group
before it waits for the answers, once.

`hale dna upgrade` brings the toolchain's newer baseline, a new
revision of a definition: a Task already running finishes under the
revision it was born with, and the next one binds the new one.

Every part has an identity you will see in `history`: the root Task
`t1`; a child Task `t1.s0.b` (the parent, the step, the key); a step
`t1/wf1/s0`; a unit of work `t1/wf1/s0/a`; an attempt `t1/wf1/s0/a/a0`,
then `/a1` on a retry. They are the record's, not the process's: a
restart finds the same ids.

## How it runs

A step registers its whole required set (`step.registered`) before
anything in it is dispatched, then activates. Each leaf admits an
attempt (`attempt.admitted`), the attempt is claimed once
(`effect.requested` under `attempt:<id>`) and delivered to a
performer of the kind the leaf's `requires` selects: `analysis` goes
to a service, then an agent; `judgment` to an agent, then a person;
an edit to the editor; a person's job to the person. The performer's
answer is the attempt's outcome (`attempt.outcome`), the leaf settles
on it (`work.settled`), and the step completes only when every
member it registered has settled, never because the right number of
answers arrived. The next step activates behind that committed
completion. When the last step completes the execution settles
(`workflow.settled`), and a child's settlement answers the parent's
step under the key it was registered as.

Delayed answers are ordinary: a performer may say "later" and the
step waits, reachable, until the reply comes, in any order. A reply
that repeats one already recorded changes nothing. A reply from a
performer other than the one the attempt was admitted to is nobody's.

**Failure.** A leaf that fails within its allowance is tried again
under a new attempt id; one that fails its last attempt fails its
step, the step fails its execution, and a child's failure fails the
step that invoked it, up to the root. Nothing later activates. A leaf
still out under a failed step keeps its responsibility: its outcome is
recorded when it comes, it reopens nothing, and the execution above
it settles failed only after it did.

**Cancellation.** Cancelling a root fences everything below it, at
any depth: what was admitted records its outcome, nothing further is
admitted or claimed, the tree drains and reclaims from the leaves up,
and a late reply for a fenced attempt is still recorded as that
attempt's outcome. The engine carries cancellation as a message
(`WorkflowCancelRequested`); the CLI has no verb for it yet.

## People in it

A leaf whose work is a person's becomes a **case**: its own handed
Task, `<task>.s<i>.<key>`, admitted as `case.admitted` and handed as
`task.handed`, exactly at the terms the acceptance practice of the day
sets. It appears in `hale dna board` and closes the way every handed
Task does — `hale dna task done <id> --as <you>`, with the evidence or
the authorized exception the practice requires, or `task decide` for a
decision someone else made. The completion is a row in the person's
name; the engine reads it from the record and the attempt settles on
it, so the step the case was under moves on. Nothing else settles a
case: a restart, a redelivery, the Leader — the case waits for its
person. The first valid completion is the outcome; a second is a
duplicate and settles nothing twice.

## Edits in it

A leaf whose work is a change to the code becomes a Mutation. The
record says which attempt asked for it (`mutation.requested`, before
`mutation.proposed`), so a redelivery after a restart is answered from
that Mutation and never edits twice. The Mutation takes the road every
change takes — the sandbox, the candidate, the evidence, the Review —
and the leaf's attempt is done once the candidate is in review. The
Review itself is outside the workflow: your verdict, the apply, the
window are the loop of [Working with it](./working.md), and a
`revise` or `reject` is the Review's, not the execution's.

## What a restart keeps

The record is the execution. An organization restarted over it, a new
process over the same git record, or the same record reconstructed
from its two memories, rebuilds only what is unfinished, under the
original ids: no execution is admitted again, no step registers or
activates twice, no completed member runs again. Every admitted root
not yet settled, or settled but with work still owed under it, is
asked for again; an intent that was offered but never admitted before
the stop is noted once as `intent.unrecovered` and never re-offered,
because work may already have run for it.

An attempt the old process had claimed is reconciled before anything
is delivered again. A performer whose adapter kept its own record of
the invocation answers from that record, and the effect is not
performed twice. One that truthfully has not acted is delivered the
attempt again, once, under the same id, and the record says so
(`effect.redelivered`). One that cannot say leaves the attempt
waiting, visibly, for a person to resolve. Which of these your
performers are is theirs to declare; the default can say nothing.

## What is promised

The baseline is one test over the example above, run under every
supported mode — immediate, delayed and out-of-order replies, a
duplicate, a stale reply, a failed grandchild, a cancellation; one
memory in process, a git record with a restart mid-flight, two
memories reconstructed mid-flight, a leased record fenced by a
takeover, and a real process killed at three points and resumed —
with one oracle over the record: the member sets, each step once,
every unit of work once on its admitted attempt, one claim per
attempt, the adapter's invocation count, the order of the rows, and
every resident reclaimed. What it establishes is stated as the
supported semantics in the spec (`spec/dna.md`, *Workflow execution:
the baseline*), and what it does not cover is not promised.

Two lines worth knowing. A transition is exactly-once in the record —
one claim, one outcome, one settlement — but whether the *external*
effect happened once is the adapter's to say, as above. And a
borrowed handle a performer holds has to outlive it; the engine does
not check lifetimes for you.

## Reading it

`hale dna status` counts executions asked and settled beside the
Tasks; `hale dna history t1` walks one execution's rows by their
causal links, children and attempts included, and sums what its
model calls cost, by position and by backend (the execution and each
attempt carry the same `usage` on the API); `hale dna board` shows
the cases waiting on people. The face (`dna/face`, served by the
head) is where these become workspaces — definitions, executions,
the people's queue — over the same rows; the verbs in this chapter are the
commands it issues, and creating a task is what `hale dna ask` is
the terminal spelling of.
