# The spine

The spine is the one surface every other part calls: a person asks,
a leg claims and settles, a head reads, and every answer the spine
gives is a row. It is not one process. It is the program every node
runs: the host that `hale dna run` or `hale dna dev` starts, and the
organization (`dna/org`) the host starts beside it. It runs on the
body, with memory's full spine role and the body lease. A node holds
nothing: its state is the stores and its code is the genome, so a
node that can no longer prove its lease, or whose nerves fail, stops
rather than act, and a fresh one picks up from the stores. The spine
never holds a device's session: a browser reaches it through [the
head](./head.md), and a clone through rows in the record.

## A node

Run one on your own machine with `hale dna dev`, which also brings up
memory and the nerves from `dna/compose.yaml` and runs the
application under the same host. `hale dna run` runs a node on its
own and takes memory and the nerves from its environment (see
[Memory](./memory.md#working-memory) and
[The nerves](./nerves.md#migrate-and-drop)).

On every tick, once a second, each node:

1. asserts the body lease, and stops if it is no longer its own;
2. relays onto the nerves every request row the record has not
   answered yet ([The nerves](./nerves.md#relaying-after-the-row-lands)),
   first, so an answer never waits behind the rest;
3. syncs the record with its remote;
4. projects the record into memory's graph, carries out a ledger
   adoption or abandonment the record asks for, and erases the
   protected bodies the record says were redacted;
5. checks the forge for a new genome ([below](#the-genome-pull));
6. asserts the lease again, since the sync may have outlasted it;
7. tells the heads every row it landed, and answers the restart
   requests the record holds.

A record admits one body per owner at a time ([below](#the-body-lease-and-the-fence)),
and over a shared record every owner's body is a node of the same
spine. Nothing coordinates them; the stores keep them from doing
anything twice: the projection moves a row at a time by
compare-and-swap, an adoption is claimed by the row that asked for it,
and an erasure is by digest.
When the record has a remote, a node projects only what the remote
holds after that tick's sync, so a stamped row is always one every
clone can receive.

### Claims by id

Every act a node takes that must not happen twice is claimed in
memory first, by id, with an expiry: one row of memory's `claims`
table per key, taken with one conditional write. Of two nodes racing
for a key, one lands and the other finds it taken. The expiry is the
database's clock, so nodes whose clocks disagree still agree on when a
claim ends. A node that dies holding a claim leaves it to expire, and
the next node finishes the act. The record shows each claim a node
acted on:

```text
claim.taken plan/i7 {"holder": …, "token": 1, "until": …}
claim.released plan/i7 {"holder": …}
```

A node names itself in its claims by the body's holder,
`user@host:<clone>` (`HALE_DNA_NODE`, which the host sets).

### The genome pull

Nodes take a new genome by pulling it; nothing pushes it to them.
Every five minutes (`HALE_DNA_GENOME_POLL`, in seconds; `0` never) a
node fetches the forge's default branch. When it moved, the node
stops cleanly, gives the body lease back and exits with code 75; its
unit starts it again, and at start it builds the new genome. A genome
that does not build is a `node.build_failed` row: the node goes back
to the last genome that built and keeps serving it. `hale dna status`
says what each node runs on its `genome:` line.

## The API: ask, claim, settle, read

| call | who makes it | what the spine does |
|---|---|---|
| **ask** | a person (`hale dna task create`, the face), the Leader's optimize proposal | admits the ask as a workflow execution, or refuses it, as rows |
| **claim** | a leg, through the head (`hale dna work next`) | hands it one outstanding attempt under a lease |
| **settle** | a leg, through the head (`hale dna work submit`) | checks the lease and records the attempt's outcome |
| **read** | a head, a leg | the record, memory's graph, and one Work's hat |

An ask is a row first (`intent.requested`), written by whoever asked,
from any clone; a node relays it to the organization. The
organization admits an intent once by its id, so a relay repeated
after a lost answer is the same execution. Before it admits one, it
checks in order:

- the Board's gate: an empty ask is refused;
- the owner: over a shared record, only the organization that owns
  the position the ask is for admits it;
- the budget: on an exhausted window the ask is refused
  ([below](#the-budget-the-one-gate));
- the plan claim `plan/<intent>`, when the Leader plans
  ([below](#the-leader-and-planning)).

Then it writes `intent.offered` and admits the ask as a workflow
execution (`workflow.admitted`, with `task.born` as its one-line
summary). A refusal is an `intent.refused` row with the reason. What
happens next, step by step, is [One task, end to end](./workflow.md).
A schedule's occurrence is not an ask: it is admitted as an execution
directly, under its own key ([Schedules](./schedules.md)).

A claim and a settlement are a leg's, and go through the head's API
under the leg's lease: `dna.attempt.claim`, `dna.attempt.renew`,
`dna.attempt.allowance`, `dna.attempt.outcome`, `dna.attempt.release`.
The head writes the leg's rows into the ledger, and a node relays the
outcome and the allowance ask to the organization, which settles them.
The verbs a leg runs are [Legs, hands and voice](./legs.md); the API
and its transport are [The head and the face](./head.md).

## The Leader and planning

The generated organization says `planned: true`, so the Leader reads
every ask before it is admitted. The Leader is the organism's
architect: it proposes, and the Board decides. Its brief is its
charter (`dna/org/charter.hl`), the purpose (`dna/org/purpose.hl`),
the law as the genome holds it, and the practices the Board has
ratified for `org`.

Planning is a model call, so it is spend, and it is claimed first.
The node whose claim on `plan/<intent>` lands asks the Leader; a node
that finds the claim taken answers `planning elsewhere: <holder> holds
plan/<intent>` and writes nothing, and the ask comes back to it with
the next relay until it is answered or the claim expires (300
seconds). While the Leader's word is out, the ask is in planning, and
an ask offered again meanwhile is not asked of the Leader twice.

The Leader answers what the ask is: its kind (`organism`,
`appendage`, `product` or `person`), the class of change, the target,
and for a person's job, whom and under what obligation. The admission
binds that word; a plan never widens what you asked. Then:

| the ask | admitted as |
|---|---|
| a person's job | `ask-person`: one human leaf, a case handed to the person the Leader named |
| anything else | `ask-edit`: one edit leaf, a candidate prepared for Review |
| `hale dna task create --judgment` | `ask-judge`: one judgment leaf, a leg's; no plan is asked |

A Leader answer that names no kind and no class leaves the defaults
standing: an `application` change at the ask's own target. Without a
Leader every ask is an application change.

The Leader also decides the Reviews inside the grant the Board gave
it ([Shaping and governing it](./shaping.md)), and on a cadence it
walks the machinery in the optimize pass ([Schedules](./schedules.md)).

## The workflow engine

Everything the organism does for you is one execution of a workflow,
run by one engine and written into the record as it happens.

### Definitions

A definition is data, bound whole before anything runs. It is ordered
steps, and each step names the **one store it writes**: `record`,
`forge`, `genome`, `heart`, `graph`, `nerves`, `memory`, `vault` or
`host`, one word per step. A step is where a fact is written; reading
something, or waiting for another writer, belongs to the step whose
fact it serves. A step's members are **leaves** (one unit of work: an
objective, the capability words it requires, how many attempts it may
take) or **children** (another definition, run as an execution of its
own under this step).

Your own definitions go in `dna/org/own_workflows.hl`:

```hale
import "vendor/dna" as dna;

fn own_workflows(catalog: dna::WorkflowCatalog) -> String {
    let d = catalog.define("close-month", 1, "record record", "close the month");
    if len(d) > 0 { return d; }
    let a = catalog.leaf("close-month", 1, 0, "reconcile", dna::WorkRequest { objective: "reconcile the bank feed", requires: "analysis" }, 2);
    if len(a) > 0 { return a; }
    let b = catalog.leaf("close-month", 1, 1, "post", dna::WorkRequest { objective: "post the journal entries", requires: "human" }, 1);
    if len(b) > 0 { return b; }
    return catalog.occurs("close-month", 1, "skip", "period");
}
```

`close-month` has two steps, each writing the record. `reconcile` may
be tried twice; `post` runs only once step 0 has completed. `occurs`
says how the definition behaves when a schedule points at it: `skip`
(the default) lets an open execution finish and records the skipped
occurrence, `overlap` runs the next one beside it, and `period` is the
input a schedule's `--args` must name ([Schedules](./schedules.md)).

A definition binds whole or is refused at admission, with nothing
started: a name that does not exist, a child that would recurse, a
member key outside `[a-z0-9-]+`, a step that names no store or two, a
step on a part the organization cannot write yet, or a tree past the
limits (depth 8, 32 steps, 32 members to a step, 8 attempts to a leaf,
256 units of work).

### The catalog

`hale dna new` writes `dna/org/workflows.hl`, which returns DNA's
baseline plus your own. `hale dna upgrade` rewrites it to the current
shape and never touches `dna/org/own_workflows.hl`. The baseline is
the vendored toolchain's, so an upgrade supersedes a definition by
revision: a task born under a revision finishes under it, and the
next one binds the newest.

`hale dna definitions` lists the catalog, with each step's store and
what an admission would refuse a definition for (`--json` for the
catalog's own document). It leads with, and fails on, a definition of
your own that the catalog refused. On a new project:

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

The baseline:

| id | steps (store) | admitted today |
|---|---|---|
| `ask-edit`, `ask-person`, `ask-judge` | one leaf (record) | every ask, as planned above |
| `practice-ratify` | ratify · hat (record) | every proposal the Board decides |
| `concern-escalate` | raise (record) | every concern |
| `optimize-walk` | walk (record) | the optimize pass |
| `ask-triage` | offered · classify · plan (record) | refused: the organization performs no step `offered` |
| `change-deliver` | candidate (record) · review (forge) · verdict (record) · apply (genome) · deploy (heart) · settle (record) | refused: it writes the heart |
| `deploy-observe` | deploy (heart) · settle (record) | refused: it writes the heart |
| `secret-rotate` | rotate (vault) · recorded (record) | refused: it writes the vault |
| `body-provision` | provision · start (host) · lease (memory) · observed (record) | refused: a person provisions a host with `hale dna body provision` |

### How an execution runs

Every part of an execution has an id you will see in `hale dna
history`, and they are the record's, so a restart finds the same
ones:

| id | what it is |
|---|---|
| `t1` | the root task |
| `t1.s0.b` | a child task: the parent, the step, the member key |
| `t1/wf1/s0` | a step (`wf1`: the definition's revision) |
| `t1/wf1/s0/a` | a unit of work |
| `t1/wf1/s0/a/a0`, then `/a1` | an attempt, and its retry |

A step registers its whole set of members (`step.registered`) before
anything in it is dispatched. Each leaf admits an attempt
(`attempt.admitted`), the attempt is claimed once, and it is
delivered to a performer of the kind its `requires` selects:

- `organism`: the organization performs the step itself;
- `edit`: the editor, as a Mutation prepared for Review;
- `human` or `person`: a person, as a case handed on the Board's
  queue;
- otherwise capability first: `judgment` to an agent, then a person;
  `analysis` to a service, then an agent. The generated organization
  hands the agent kind to [the legs](./legs.md).

The performer's answer is the attempt's outcome (`attempt.outcome`),
the leaf settles on it (`work.settled`), and the step completes only
when every member it registered has settled. The next step activates
behind that completion, and when the last one completes the execution
settles (`workflow.settled`). A performer may answer later, in any
order; a reply that repeats one already recorded changes nothing, and
a reply from anyone but the performer the attempt was admitted to is
nobody's.

### Failure and retry

A leaf that fails with attempts left is tried again under the next
attempt id. One that fails its last attempt fails its step, the step
fails its execution, and a child's failure fails the step that invoked
it, up to the root. Nothing later activates. A member still out under
a failed step keeps its responsibility: its outcome is recorded when
it comes, and the execution above it settles failed only after it has.
Cancelling a root fences everything below it; the engine carries it
as a message (`WorkflowCancelRequested`), and no `hale dna` verb sends
it.

The record is the execution. An organization restarted over it
rebuilds only what is unfinished, under the original ids: nothing is
admitted again, and an attempt whose outcome is recorded never runs
again. An attempt whose claim a dead process left open is reconciled
before anything is delivered again: a performer that can say it acted
answers from its own record, one that truthfully did not act is
delivered the attempt again (`effect.redelivered`), and one that
cannot say leaves it waiting for a person:

```sh
hale dna effect resolve attempt:t1/wf1/s0/a/a0 --outcome failed
```

A transition is exactly once in the record: one claim, one outcome,
one settlement. Whether an external effect happened once is its
performer's to say.

## The budget: the one gate

The organization's spend is one policy in the model catalog,
`dna/org/models.hl`:

```hale,fragment
fn org_budget() -> dna::BudgetPolicy {
    return dna::BudgetPolicy { window: "day", allowance_micros: 25000000 };
}
```

An allowance in micro-dollars per `day`, `week`, or `none` (one
allowance for the record's lifetime); `0` is unmetered. The
organization owns the one `Budget`, which accounts every `model.called`
row at its time and is rebuilt from the record at start, so a restart
loses nothing.

Every model-backed spend passes one gate, `Budget.admit`: the
editor's attempt and the Leader's Review before their calls, a leg's
attempt when the leg asks for its allowance, an ask, and the optimize
pass. On an exhausted window it refuses, and the refusal is a row;
an ask's reads:

```text
refused: budget exhausted (spent 25000000 of 25000000 micro-dollars this day in 41 call(s))
```

`budget.exhausted` is written once for the window and the Board is
told; a pending Review waits for the Board, which needs no model. The
next window starts clean.

Otherwise the gate admits the smaller of the work's own cost ceiling
and what the window has left. A leg asks for its allowance before its
first model call (`hale dna work allowance`) and makes no call whose
known price is past what is left of it. Nothing is reserved, so legs
admitted against one remainder can together pass the window's
allowance, by at most what they were granted plus one call each; the
organization settles an attempt that spent past its grant as failed,
naming the overrun, and journals its calls all the same.

The model catalog and its keys are [Legs, hands and
voice](./legs.md). Money a grant delegates is a separate budget
([Shaping and governing it](./shaping.md)).

## The body lease and the fence

A record admits one body at a time, one per owner over a shared
record. Before the host builds or runs anything it takes the body
lease, at the record's remote when there is one, so a host on another
clone is refused by the remote itself and exits 3. The holder is
`user@host:<clone>`, so the same clone restarting takes its lease
straight back.

The lease lives 30 seconds. The host asserts it at the top of every
tick, before it relays, restarts or applies anything, and again the
moment before it starts a process. Beside the host runs the **body
fence**, a small process that renews the lease every ten seconds with
every git call bounded, and never waits on the host. When the lease is
someone else's, when the fence cannot prove it within five seconds of
its expiry, or when the host is gone, the fence kills the organization
and the expression with every process they started. A body that cannot
reach its remote executes nothing past the lease.

On an adopted ledger the lease is a row of memory's `claims` table
(`body`, or `owner/<owner>` over a shared record). Its token is an
epoch the host hands its organization, and memory refuses an append
under a lease that was taken over, released or expired: `fenced`,
whatever the organization still believes.

```sh
hale dna body                    # who runs this record
hale dna body claim --force      # take it from a body that is gone, as a row in your name
hale dna body release            # give it up
```

A body on a server, and how you provision one, is [The heart and the
body](./heart.md).

## Where reflexes run

Not in the spine. The reflexes are a program of their own beside the
senses, and a firing reaches the organism as an application event.
The spine records it as a reading row before anything acts on it; the
fleet node the firing names restarts the instance and records what it
did;
and a second firing for the same target within the window becomes a
concern the organization raises, for a workflow to answer. The spine
records and routes; it never plans on a reading. See [Senses and
reflexes](./senses.md).

## The pieces in dna/core

The organization's substrate, `dna::Dna`, is assembled from family
loci in `dna/core`, each keeping one family of rows; they are listed
in [DNA, the building block](./dna.md).
