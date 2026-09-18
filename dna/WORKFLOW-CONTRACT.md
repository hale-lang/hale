# dna — the recursive workflow execution contract

A design note for the recursive workflow baseline (delivery cards 00–19).
It fixes the ownership, identity, barrier, persistence, retry and
compatibility rules the later cards build on. It describes **intended**
behaviour; `spec/dna.md` describes what ships, and changes only when a
card ships the behaviour with its test.

Each rule carries one status:

- **[decided]** — a design decision; later cards implement it.
- **[existing]** — current behaviour, kept as is.
- **[needs proof]** — depends on native lifetime behaviour that card 03
  must demonstrate before cards 04 onward build on it.
- **[proven: …]** — demonstrated by the named native fixture.

## 1. The primitive

A workflow is one or more ordered steps. A step holds one or more
required members. A member is either a leaf Work or a child workflow,
recursively. The next step starts only after the previous step's
completion barrier is satisfied. Applications supply the activities,
inputs, performers and policies; DNA preserves composition, identity,
completion and continuation across immediate replies, delayed replies
and restart. **[decided]**

Out of scope: a DSL or configuration language, an arbitrary DAG
engine, a business workflow library, a scheduler product, a
coding-agent integration. **[decided]**

## 2. Ownership

The names and their ownership meanings are kept. **[existing]**

| Locus | Owns | Answers to |
|---|---|---|
| `Metabolism` | the Task pool; mediates delegated settlement | the assembly (`Dna`) or the application |
| `Task` | one bounded intention; binds one workflow recipe and revision | `Metabolism` |
| `Workflow` | the ordered steps of one execution; activates each next step | its `Task` |
| `Step` | its required-member set and its barrier | its `Workflow` |
| `Work` | one logical piece of work across its Attempts | its `Step` |
| `Attempt` | one performer on one request | its `Work` (the owner of the performers runs it) |

A child workflow is a child `Task`, owned by `Metabolism` like every
Task, and correlated to its spawning Step by data (`parent_task`,
`spawning_step`, member key). **[existing]**

Journal facts are the durable authority. The live loci are a
projection of those facts and are rebuilt from them on restart; a
recovery reconstructs this same model, never a separate scheduler.
**[decided]**

## 3. Definitions and executions

A **definition** is plain data: a workflow definition id and revision,
its ordered steps, and each step's members. A leaf member carries its
own `WorkRequest` content (objective, requires, data class, target,
context digest, output contract, cost ceiling). A child member names a
child definition. Definitions are flat records that reference each
other by id; they never nest live loci. **[decided]**

An **execution** is one admission of a definition. At admission the
finite recipe is resolved and bound: the whole expanded tree with every
member's content, the definition ids and revisions, and the inputs. The
bound recipe is recorded, not just a revision number, so a later change
to a definition affects only later admissions. **[decided]**

**Admission limits.** Admission checks the expansion against limits the
assembly supplies (`AdmissionLimits`); an application or assembly may
raise or lower them. The documented defaults are **[decided]**:

| Limit | Default | What it bounds |
|---|---|---|
| `max_depth` (root = 1) | 8 | how deep child workflows nest |
| `max_steps` | 32 | steps in one workflow |
| `max_members` | 32 | members in one step |
| `max_attempts` | 8 | the largest retry allowance a leaf may bind |
| `max_works` | 256 | leaf Works in one bound recipe, all levels |

These are capacity policy, not language or implementation ceilings: no
hard limit is known for any of them. They bound the work one admission
may create, the size of the recipe recorded with it and the depth of
the expansion. The limits applied are bound into the execution with its
recipe, so a later change of defaults never reinterprets an admitted
execution. If persistence (card 05) meets a real ceiling — a journal
row size, a transport limit — it is named there as such, separately
from these defaults. `max_attempts` caps what a leaf's retry allowance
may be; the retry policy itself (how many attempts a leaf wants, which
performer each attempt uses) is the application's. Exceeding any limit
refuses the whole admission, naming the limit, the value and the
member or workflow that exceeded it; nothing is admitted.
**[decided]**

A definition that refers to itself, directly or through children, is
refused at admission as unbounded. **[decided]**

## 4. Identity

Every execution entity has one id, built from its owner's id. Member
keys are application-given, `[a-z0-9-]+`, unique within their step.
**[decided]**

| Entity | Id |
|---|---|
| root Task | `t<n>` (`<owner>:t<n>` over a shared record) **[existing]** |
| child Task | `<parent task>.s<i>.<member key>` |
| Workflow | `<task>/wf<revision>` **[existing]** |
| Step | `<workflow>/s<i>` **[existing]** |
| Work | `<step>/<member key>` |
| Attempt | `<work>/a<n>`, `n` from 0 **[existing]** |

Two invocations of the same definition are two child Tasks with two
different ids, because their parent, step or member key differs.

A result is valid only if it names its expected entity:

- a Work outcome names `work_id` and `attempt_id`, and the attempt is
  the Work's current admitted attempt;
- a child settlement names the child `task_id`, its `parent_task` and
  its `spawning_step`, and that child is a registered member of that
  step.

A repeated delivery of an accepted result is a no-op. A result for an
unknown member, the wrong step, or a superseded attempt is recorded as
such and changes nothing. A retry is a new Attempt with a new number; a
retransmission of the same admitted attempt keeps its id. After a
restart, a reply for the still-current attempt is accepted. Owner and
lease fencing (`HALE_DNA_LEASE`, epochs) is separate from attempt
identity. **[decided]**

The routed retry path already gives the performer, the Attempt record,
its history and model evidence one attempt number.
**[proven: `routed_attempt_identity_test.hl`, card 01, #685]**

## 5. The barrier

1. A step registers its complete required-member set, durably, before
   any member is dispatched.
2. A step completes once, only when every registered member has a
   committed `done` outcome.
3. `pending` is not terminal.
4. A required member's terminal failure fails the step at once; later
   steps never activate. The other admitted members follow the bound
   drain policy: by default they keep their responsibility until their
   own outcome, which is recorded and cannot reopen the step.
5. Cancellation stops further admission and fences outstanding members.
   It does not undo external effects.
6. Logical failure and physical reclamation are separate. An
   outstanding responsibility stays reachable until its outcome or an
   explicit fence. There is no invented timeout.

**[decided]**

The current `Step` counts child settlements without checking their
disposition or spawning step. The new execution path replaces that
count with the exact-member join. **[existing, to be replaced by cards
06 and 11]**

## 6. The canonical example

Every later card's tests use this one example.

```
root  t1  definition close-month rev 1
  s0  (t1/wf1/s0)
      a   leaf   "reconcile the bank feed"          Work t1/wf1/s0/a
      b   child  definition collect-receipts rev 1  Task t1.s0.b
          s0  (t1.s0.b/wf1/s0)
              b1  leaf  "list missing receipts"     Work t1.s0.b/wf1/s0/b1
          s1  (t1.s0.b/wf1/s1)
              c   child  definition chase-supplier rev 1   Task t1.s0.b.s1.c
                  s0  (t1.s0.b.s1.c/wf1/s0)
                      c1  leaf "email the supplier"  Work t1.s0.b.s1.c/wf1/s0/c1
  s1  (t1/wf1/s1)
      d   leaf   "post the journal entries"         Work t1/wf1/s1/d
```

**When D may start.** Only after all of the following are committed,
in this order of dependency: `c1` done; step `t1.s0.b.s1.c/wf1/s0`
complete; Task `t1.s0.b.s1.c` settled done; step `t1.s0.b/wf1/s1`
complete (so `b1` and `t1.s0.b/wf1/s0` completed before it activated);
Task `t1.s0.b` settled done; `a` done; step `t1/wf1/s0` complete; step
`t1/wf1/s1` activated. `a` and `b` may finish in either order. D's
request is dispatched only after its step's activation is committed.

**When C fails.** If `c1` fails terminally after its retries, step
`t1.s0.b.s1.c/wf1/s0` fails, Task `t1.s0.b.s1.c` settles failed, step
`t1.s0.b/wf1/s1` fails, Task `t1.s0.b` settles failed, and step
`t1/wf1/s0` fails. Root step `s1` never activates and D is never
requested. `a`, if still outstanding, keeps its responsibility under
the drain policy; its later outcome is recorded and does not reopen
`t1/wf1/s0`. The root Task settles failed once, after `a` settles.

**Valid results in the example.** `WorkDone` for `t1/wf1/s0/a` with its
current attempt; `TaskSettled` for `t1.s0.b` with `parent_task: t1` and
`spawning_step: t1/wf1/s0`. Not valid: a settlement of `t1.s0.b`
naming step `t1/wf1/s1`; `WorkDone` for `t1/wf1/s0/a/a0` after `a1` was
admitted; a second `done` for `c1`.

**What restart preserves.** The bound recipe for `t1` and all its
descendants; every registered member set; every admitted attempt and
recorded outcome; every step activation and completion. After a
restart, only unfinished responsibilities are rebuilt, with their
original ids; completed members do not run again; each step activates
at most once; the root settles last.

**Who retries.** The `Work` owning each leaf (`a`, `b1`, `c1`, `d`)
admits its next attempt under the bound retry policy. A child workflow
is not retried as a whole unless the application's policy says so.

## 7. Retries and execution

One retry owner: the durable Work lifecycle admits each attempt under
the bound routing/retry policy. The new executor path runs exactly one
admitted attempt per request and never retries on its own. The current
`WorkSystem` loop, which retries internally, stays for legacy callers
until card 18. **[decided]**

A settled attempt is never executed again. Durable dispatch may be
redelivered; a duplicate request for a running attempt attaches to its
pending outcome. If a process stops after an external effect and
before its outcome is saved, recovery reconciles through the existing
effect records (`effect.requested`, `effect.result`) or the adapter's
documented idempotent replay; otherwise the Work stays visibly
unresolved. Success is never manufactured and an uncertain effect is
never blindly repeated. **[decided]**

## 8. Durable transitions

Proposed fact families (card 05 fixes the exact names and codecs). All
are Ledger rows under split routing and Record rows on routing 0, and
are added to `memory_of` explicitly. `task.*` names are not reused.
**[decided]**

| Fact | Entity | Carries |
|---|---|---|
| `workflow.admitted` | Task | engine `wf1`, bound recipe, inputs |
| `workflow.refused` | Task | the violated bound or validation |
| `step.registered` | Step | the required-member set |
| `step.activated` | Step | activation id |
| `attempt.admitted` | Attempt | Work, attempt number, bound request |
| `attempt.outcome` | Attempt | disposition, result and evidence refs |
| `work.settled` | Work | the accepted attempt, disposition |
| `step.completed` / `step.failed` | Step | the deciding members |
| `workflow.settled` | Task | disposition |

Transition rule: validate against the current projection, append with
exact compare-and-append, and dispatch or announce only after the
append is acknowledged. On a stale revision, refresh and evaluate the
transition again. An append refusal means no dispatch and no
announcement. **[decided]**

One positive discriminator: `workflow.admitted` with `engine: wf1`
marks a new-engine execution, written atomically with its bound recipe
before anything runs. A Task without it keeps the legacy recovery path.
A partial new admission never falls through to legacy `requires: edit`
recovery. **[decided]**

Associations to Mutations and Reviews are by id, never by position in
the merged journal. **[proven for the legacy path:
`recovery_association_test.hl`, card 02, #686]**

## 9. Lifetime and persistence wiring

**Lifetime.** Execution loci are resident and event-driven. A waiting
instance is an `accept`ed child whose type no parent `release`s, so it
stays alive after its `run()` and its creating handler return. It
advances from its own bus handlers, keyed by its id, and ends with
`terminate;` from a handler once its responsibility is finished. No
sleeping handler or polling loop stands in for continuation. Its owner
learns the outcome over the bus, not through `release`. **[needs proof]**

The trap: `release(c: T)` anywhere makes every `T` a flow, program-wide.
Today `Attempt`, `Work`, `Step`, `Workflow` and `Task` each have a
`release` hook, so they dissolve at the end of their `run()` (F.5,
F.15). Card 03 proves the resident shape on isolated types and records
the migration those five types need. **[existing]**

**Persistence wiring.** An execution locus does not hold a journal: an
interface-typed value cannot flow into a child's field (F.3). It
proposes each transition to the one committer over the bus, keyed by
its own id, and acts only on the committer's answer:

```
child  --TransitionProposed{key, kind, entity, body}-->  committer
committer: validate, exact append
committer  --TransitionCommitted{key, revision}-->  child  -> dispatch
committer  --TransitionRefused{key, why}-->        child  -> no dispatch
```

The committer is the journal's owner: `Dna` in an assembled organism.
A standalone `Metabolism` is its own committer over a default memory
journal. Exactly one committer answers in a program; which one is
chosen at construction. The memory-backed assembly gives process-local
guarantees only; durable restart needs a persistent journal.
**[needs proof]**

## 10. Compatibility

Until card 18 **[decided]**:

- `Dna.ask`, `Metabolism.offer` and `Knobs` keep their current
  behaviour; the new engine is assembled explicitly.
- Legacy records keep their legacy recovery (fixed for split memories
  in card 02).
- The meaning of human completion (`hale dna task done`, evidence,
  exceptions) and of code-review completion is unchanged.
- No historical journal row is rewritten.

After card 18, new work uses one engine; old records are read through a
narrow legacy adapter. `Knobs` becomes an adapter that produces an
equivalent definition.

## 11. Proof status

| Claim | Status |
|---|---|
| routed attempt identity | proven, card 01 (#685) |
| Mutation association by id across memories | proven, card 02 (#686) |
| a handler-born child survives its `run()` and its handler | needs proof, card 03 |
| a delayed keyed reply reaches it and it advances to nested work | needs proof, card 03 |
| completion and parent shutdown reclaim it, without leaks | needs proof, card 03 |
| a duplicate terminal message cannot advance it twice | needs proof, card 03 |
| child → committer → acknowledgement → dispatch, and refusal | needs proof, card 03 |
| the shape works in a program that imports the DNA core | needs proof, card 03 |
