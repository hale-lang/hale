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
`spawning_step`, member key). **[existing]** In the resident runtime the
Step asks for the child over the bus and the Task owner creates it from
its own handler: a subscriber born in a handler must be that handler's
own child, and a Step accepts only its Work (§9, F.19).
**[proven: `workflow_lifetime_test.hl` case 4b]**

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

An admission's recipe binds its task under exactly this workflow
identity: the projection refuses an admission whose recipe binds the
task under any other — another revision's, or a name of no such form —
and a restored execution runs under the identity its recipe bound,
read from the record, never re-derived from the task and revision.
**[proven: `workflow_recovery_test.hl`, card 13]**

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
   It does not undo external effects. The fence is durable: the
   executor reads the execution's cancellation (its own or an
   ancestor's `workflow.settled`) at the same captured reading its
   execution claim is exact at, so an attempt not yet claimed under a
   cancelled execution starts no new work whatever notification lagged,
   and one claimed before still records its outcome. A step that had
   failed and is draining when the cancellation lands keeps its
   failure and forwards the fence to its live members; the fence is
   never a second step outcome, and it is the one message a member acts
   on for a cancellation, so a member that retires or settles on it
   ends exactly once (a resident that ends with a second message queued
   for it is #703). **[proven for one execution, card 10:
   a cancellation while step 0's leaf is out settles the execution
   cancelled, the leaf cancelled, births no step 1, and the leaf's late
   reply is recorded and reopens nothing; a cancellation that lands
   between an attempt's admission and its request to run leaves it
   unclaimed and unrun; a cancellation during a failed step's drain
   fences the pending sibling and the failure stands; the fence reaches
   down: a root cancelled while its grandchild's leaf is out settles
   the child and the grandchild cancelled and the leaf under them —
   card 11]**
6. Logical failure and physical reclamation are separate. An
   outstanding responsibility stays reachable until its outcome or an
   explicit fence. There is no invented timeout.

**[decided; 1, 2, 4 and 6 proven for one step and its leaves:
`workflow_step_test.hl`, card 09 — the set registered and activated
before a leaf is born; completion in either order; waiting on a delayed
member; a failed member failing the step at once, its sibling's later
reply recorded and reopening nothing, the `StepRun` alive until that
member settled; a leaf retried within its allowance. 5 is card 06's
projection so far; 3 is card 08's.]**

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

**How a member answers (card 06).** A step registers its whole required
set with each member's kind before anything is dispatched, and a member
answers once, under the key it was registered as. A `leaf` answers when
its Work settles; a `child` answers when the execution it invoked
settles into the step that spawned it. Neither answers for the other,
and a step completes only when every key in its required set has
settled — never because the right *number* of answers arrived.
`dna/core/workflow_projection.hl` reads this from the facts alone and
reaches nothing outside itself (`@no_syscall` on its entry point). The
admitted recipe is its authority: a registration names exactly what the
recipe bound under that step, by key, kind and entity; a child is
admitted and answers only as the Task the recipe bound under its key,
and its admission carries that subtree exactly, never a node more, less
or different; an attempt asks for what its Work is, its request content
the bound Work's on every attempt. Cancellation fences everything below
the cancelled Task — nothing further is admitted under a cancelled
ancestor, at any depth the caller's limits admitted — while what was
admitted records its outcome; a failed ancestor fences nothing (the
drain policy).
Every transition has its basis in the rows before it — a Work is done on
a done attempt, failed on a failed attempt with no allowance left or
under a failed step or cancelled Task, cancelled only under a cancelled
Task; a step activates after the one before it completed; a Task is done
when every bound step completed and failed only after a step failed and
everything it admitted settled — and a row is either the fact recorded
under its identity or a conflict.
**[decided; proven: `workflow_projection_test.hl`, card 06]**

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
until card 18. **[decided; proven: `workflow_attempt_test.hl`, card 08 —
`AttemptExecutor` runs nothing before a durable admission, reuses a
recorded outcome, decides and claims the execution at one refreshed
reading of the record — an outcome durable before the claim is the
answer, never a second run — so a duplicate request attaches, judges
every reply by one rule (this attempt, the selected performer, a
disposition the contract carries), persists the outcome exactly before
answering, and reports an outcome the record refused as unrecorded]**

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
| `workflow.ask_refused` | an ask, by decision (`<ask>#<n>`) | a refusal that reached no Task: the id taken meanwhile, no free id, the record moved too often (card 07) |
| `step.registered` | Step | the required-member set |
| `step.activated` | Step | activation id |
| `attempt.admitted` | Attempt | Work, attempt number, bound request |
| `attempt.outcome` | Attempt | disposition, result and evidence refs |
| `work.settled` | Work | the accepted attempt, disposition |
| `step.completed` / `step.failed` | Step | the deciding members |
| `workflow.settled` | Task | disposition |

Card 05 fixed these as versioned codecs in
`dna/core/workflow_events.hl`, one encoder and one decoder per row,
including `step.activated` and `workflow.refused`: a routing row alone
is not a durable shape, so every kind named here has one. A decoder
refuses a version it does not know, another engine's admission or
refusal, a row missing an identity it is about, a row that says nothing
where a number belongs (absence is not zero), and a row whose own
identities contradict each other — an attempt id that is not its Work
and number, a request for another Work or attempt, a Work settling
under a Step it is not part of, a step failing on a member it never
had, an admission deeper or wider than the limits it says were applied.
A row is read whole or not at all. **[decided]**

A bound recipe travels inside `workflow.admitted` as one document
(`dna.workflow-recipe/1`) carrying the node count it was written with,
nested as an object of its own: escaped into a string it is escaped and
unescaped character by character, which is quadratic in its size.
A reader that cannot read the whole document — the wrong format, a
missing node array, a different count, a node without an identity or
with a field of the wrong type, a document that ends mid-write — binds
nothing at all. Half a recipe is not a smaller execution. **[decided]**

Transition rule: validate against the current projection, append with
exact compare-and-append, and dispatch or announce only after the
append is acknowledged. On a stale revision, refresh and evaluate the
transition again. An append refusal means no dispatch and no
announcement. **[decided]**

One positive discriminator: `workflow.admitted` with `engine: wf1`
marks a new-engine execution, written atomically with its bound recipe
before anything runs. A Task without it keeps the legacy recovery path.
A partial new admission never falls through to legacy `requires: edit`
recovery. **[decided; proven: `workflow_admission_test.hl`,
`workflow_admission_contention_test.hl`, card 07 — the admission is
appended with exact compare-and-append before the `task.born` summary,
a refusal writes no summary and requests no work, the same ask id is one
execution, a contended id is re-minted, and a restart counts admitted
ids and resumes no admitted Task as edit work]**

**The committed proposal is reconstructable from the journal alone.**
Every row a transition commits carries the scope, key and proposal id
that committed it, in the same atomic append as the fact (a
`TransitionRef`; a row no transition produced carries none, and a row
that carries part of one is refused). After a restart the committer
needs no memory of its own: for a proposal id it finds the row appended
under that id, and the fact beside it is the body that was committed,
which a later proposal reusing that id is compared against by the rule
in §9. Without this the comparison helper would have nothing to compare
against across a restart. **[decided; card 07 writes these rows]**

Associations to Mutations and Reviews are by id, never by position in
the merged journal. **[proven for the legacy path:
`recovery_association_test.hl`, card 02, #686]**

## 9. Lifetime and persistence wiring

**Lifetime.** Execution loci are resident and event-driven. A waiting
instance is an `accept`ed child whose type no parent `release`s. Born
from a bus handler, it stays alive after that handler and its own
`run()` return; it advances from its own bus handlers, keyed by its id;
it publishes its outcome and ends with `terminate;` from a handler; its
owner's dissolve reclaims one that is still waiting. No sleeping handler
or polling loop stands in for continuation. Its owner learns the
outcome over the bus, not through `release`.
**[proven: `workflow_lifetime_test.hl`, `workflow_lifetime_dna_test.hl`]**

The rules the proof pinned down, natively (hale 0.20.0, this tree):

- **`release(c: T)` anywhere makes every `T` a flow**, even on a parent
  type that is never instantiated; a flow child is reclaimed when its
  `run()` ends and a later reply reaches nobody.
  **[proven: `workflow_release_type_wide_test.hl`]**
- **A keyed subscription reads its key when it is registered, before
  `birth()` runs.** Every key a resident subscribes on is a constructor
  argument. **[proven: the lifetime fixtures construct child keys]**
- **A subscriber born in a handler must be that handler's own child**
  (`hale check` refuses otherwise; bubbling is not considered). A parent
  accepts one child type. So a Step owns its Work, and a child workflow
  is requested from the Task owner over the bus. **[proven: F.19
  reproducer; `workflow_lifetime_test.hl` case 4b]**
- A publish from `main`'s `run()` is queued and dispatched while `main`
  sleeps; a publish from a child's `run()` is dispatched in place. The
  fixtures drive each asynchronous step from `main` so no reply is
  answered inside the handler that asked. **[observed]**
- A flow-typed child that declares no `run()` was not reclaimed at
  birth. Nothing may rely on this. **[observed]**
- A local bound to a struct field follows the field: `let p =
  self.held; self.held = Empty { };` leaves `p` empty. Clear a field
  only after its last use. A payload's strings live only as long as its
  delivery; a field that keeps them stores `std::str::clone` copies.
  **[observed: `workflow_lifetime_dna_test.hl`]**

**Migration of the existing process types.** `Attempt` (released by
`Work` and `WorkSystem`), `Work` (by `Step`), `Step` (by `Workflow`),
`Workflow` (by `Task`) and `Task` (by `Metabolism`) are flows. Making any
of them resident means removing every `release` of that type and moving
every reader of a settled child from `release` to the bus, which ends
the synchronous in-tower shape (F.5) the legacy callers and fixtures use.
**Decision:** the runtime uses new resident types beside the legacy flow
types, with the same ownership meanings; the legacy types stay for the
legacy path until card 18 retires it. Names, fixed here for cards 09 on,
in `dna/core/workflow_runtime.hl`:

| Resident type | Accepts | Accepted by | Meaning |
|---|---|---|---|
| `TaskRun` | `WorkflowRun` | the Task owner (`Metabolism`'s runtime half) | one admitted execution (root or child) |
| `WorkflowRun` | `StepRun` | `TaskRun` | its ordered steps; activates each once |
| `StepRun` | `WorkRun` | `WorkflowRun` | its registered members and barrier; requests child workflows over the bus |
| `WorkRun` | — | `StepRun` | one leaf across its attempts; admits each attempt |

`StepRun` and `WorkRun` exist (card 09), `WorkflowRun` (card 10) and
`TaskRun` with the Task owner `Executions` (card 11), beside
`WorkflowRuntime`, the committer and executor they propose to. A step
asks its Task for a child member (`ChildRequested`, keyed by the parent
task); the Task cuts the subtree from its recipe and asks the owner
(`ChildAdmitRequested`, keyed by scope), which births one `TaskRun` per
Task id; the child proposes its own admission and runs behind it; a
child that settled answers its spawning step (`ChildSettled`, keyed by
that step) and, once its workflow drained and left, tells it so
(`ChildLeft`), which is what the step's drain waits for; a workflow
that left tells its Task (`WorkflowLeft`). A fence that reaches a child
while its admission answer is out is applied when the answer comes; a
record-held child admission is decided again by the state when the
spawning step settles, as a leaf's is — and a step that settles or is
fenced while the answer is out owes one re-decision, taken on a
refusal by the record, whichever of the two arrives first. A leave
from a child that has not answered is nobody's. **[proven, card 11 review: a
root cancelled by another hand as the child's admission lands, the
child settling cancelled behind it; a held child admission under a
step its sibling failed, retiring and the root failing; the tree
cancelled with the deepest Work's settlement refused once, no ancestor
reclaiming before that Work settled and each level left]** A `WorkflowRun` births step i+1 only from the
handler that hears step i drained (`StepDrained`, keyed by task, which a
step publishes as it leaves) behind its committed completion, and a
cancellation (`WorkflowCancelRequested`) is a `workflow.settled`
proposal that, landed, fences the active step through `WorkflowSettled`. A `StepRun` copies the leaves it is handed into its own
rows at birth: what an owner builds in its handler dies with the
handler.

Attempts are facts (`attempt.admitted`, `attempt.outcome`) and executor
calls, not a resident locus.

**Persistence wiring.** An execution locus does not hold a journal: an
interface-typed value cannot flow into a child's field (F.3). It
proposes each transition to the one committer over the bus, keyed by
its own id, and acts only on the committer's answer to that transition.
The envelope, amended after the review of card 03, frozen for card 05:

```
topic TransitionProposed  { payload: TransitionProposal; keyed_by scope; }
type  TransitionProposal  { scope; key; proposal_id; kind; entity; body }
topic TransitionAnswered  { payload: TransitionAnswer;   keyed_by key; }
type  TransitionAnswer    { scope; key; proposal_id; ok; revision; why; basis }
topic RecordResumed       { payload: RecordResume;       keyed_by scope; }
type  RecordResume        { scope; why }
```

`basis` (card 09) says who refused: `state` (the transition has no
basis in the projection), `record` (the append was refused, or the
record moved too often — the transition may be proposed again), or
`conflict` (the proposal disagrees with what its id committed, or with
itself). `RecordResumed` says the record is writable again; whoever
knows publishes it — the host after a reconnect, a fixture.

- **`proposal_id` names one logical transition**, stably: it is derived
  from what the transition does (the entity and the step, attempt or
  activation it moves), never from a counter or a clock, so a retry of
  the same transition carries the same id and a different transition
  never does.
- **The proposer holds one outstanding proposal at a time** and acts on
  an answer only when its `scope` and `proposal_id` match that proposal,
  consuming it once. A repeated answer, an answer to an earlier
  transition, and an answer from another committer change nothing. A
  proposer whose answer may have been lost sends the same proposal
  again.
- **The committer deduplicates by `proposal_id`**: the committed row
  records the id, so a proposal already committed is answered again
  with its original revision and not appended again, including after a
  restart. A refused proposal is not remembered and is evaluated again
  when re-sent.
- **A repeated id is a replay only if it repeats the proposal.** The
  same `scope` and `proposal_id` carrying the same `key`, `kind`,
  `entity` and `body` is the same transition sent again, and is
  answered from what was committed. The same id carrying anything else
  is a conflict: the committer refuses it, naming the id and what
  differs, and appends nothing. A proposer never reuses an id for
  another transition, so a conflict is a defect in the proposer or a
  forged proposal, not a race. The lifetime proof's lookup shows the
  replay half; card 05 implements the comparison (`transition_conflict`,
  which names the part that differs) and the durable reference beside
  each committed fact (§8) that a restart compares against. Card 09's
  runtime rebuilds the committed ids from those references as it
  catches up, and answers a repeated id from the committed row and a
  conflicting one with a refusal. **[proven, card 09]**
- **The committer validates against current state before appending**,
  and appends with exact compare-and-append. On a stale revision it
  refreshes and evaluates the transition again against the new state;
  it does not simply retry the append. The proof's committer has no
  domain state and only retries; cards 06 and 07 implement the
  re-evaluation, and card 09's `WorkflowRuntime` is the committer for
  the residents: it catches its projection up from the record and
  takes that reading's revision as the one it decides and appends at —
  never a second look, which a row landing between two looks would
  slip past — validates with a dry run of the projection, appends
  exactly, applies once the append landed, and on a stale append
  decides again. **[proven: the same transition committed by another
  runtime under the decision stands as one row; a conflicting id is
  refused; a proposal the state refuses lands nothing; an append the
  record refuses moves nothing — `workflow_step_test.hl`]**
- **Only `ok` dispatches.** On a refusal the proposer dispatches
  nothing.
- **The committer is fenced on its lease, atomically with each write**
  (card 12b): the lease is rows of the record (`lease.taken`, carrying
  a per-key epoch as the token, the holder and the expiry as JSON;
  `lease.renewed`; `lease.released`), read at the revision the write is
  exact at — the token the row's own, stable under a routed reader
  that starts fresh — so a takeover — a row — makes a
  stale holder's append fail and be decided again, and fenced. Every
  write: a commit, a claim, a redelivery, an outcome, a close. A stale
  holder answers `fenced`, which a proposer holds as it holds `record`
  and proposes again when the record resumes under a live token. A
  local ownership check before an append is not the fence and cannot
  be. **[proven, card 12b: a takeover landing after the holder's own
  check and before its append, under a transition and under a claim;
  a restarted holder under the same name with a new token; two takes
  of a free lease at once]**
- **A refusal by the record is not an outcome.** An append the record
  refused, or a record that moved too often, leaves the proposer
  holding its transition: it publishes no settlement it does not have,
  reclaims nothing, and proposes the same transition again when the
  record resumes (`RecordResumed`) or, for a Work, when its attempt's
  reply reaches it again. A member has answered its Step only once its
  settlement is in the record. A refusal by the state before anything
  was dispatched ends a Step, refused, with nothing durable lost; a
  refusal by the state of an outcome the members added up to leaves the
  Step running for the members to decide again. **[proven, card 09:
  both settlements and the completion refused once by the record, all
  three landing afterwards, no `refused` answer in between]**
- **A leaf never admitted retires under its settled Step.** A Work whose
  first admission the state refused has no attempt, no claim and no
  effect to await. Under a Step that has durably settled — the Step
  publishes its outcome only after its row landed, and the Work hears
  it — it retires (`MemberRetired`, keyed by the Step), which is not a
  settlement: it answers nothing, and only a durable Work settlement
  publishes `MemberSettled`. A first admission the record refused is
  proposed again when the Step settles, so the state — not the Work —
  says whether it can still be admitted; one it admits runs and settles
  as the drain has it. The Step counts a retired member toward its
  drain, only under its settled stage and only for the Work bound under
  the key, and reclaims itself once every admitted responsibility
  settled. A leaf the state refuses while its Step runs (misbound) is
  not retired: it stays, unattempted and reachable, until it is fenced.
  **[proven, card 09: the record-refused first admission under a Step
  that failed meanwhile retires and the Step drains; the same under a
  Step that completes is admitted on resume and settles; a misbound
  leaf under a running Step stays; a forged retirement while the Step
  runs, or naming another Work, changes nothing]**
- **A member is a key and the Work bound under it.** A settlement that
  names a required key for another Work is no member's: it changes no
  answered, done, failed or drain state. **[proven, card 09: a failed
  `b` of another execution under this step's key fails nothing and the
  real `b` lives on]**
- **The body carries the proposal's own reference.** The transition
  reference inside a proposed body must be the envelope's scope, key
  and proposal id, whole, or the proposal is refused as a conflict with
  itself before the state sees it: a row would otherwise say it was
  committed under a proposal it was not. A committed proposal is
  rebuilt from its row — the reference gives scope, key and id, the row
  the kind, entity and body — and compared field by field
  (`transition_conflict`) with the proposal sent again. **[proven, card
  09: a body without its reference and one carrying another id are
  refused; the committed transition re-sent under another key is a
  conflict; re-sent whole it is a replay]**

`scope` names the one committer that answers: `Dna` in an assembled
organism (its journal), or a standalone `Metabolism` over its own memory
journal, chosen at construction. The memory-backed assembly gives
process-local guarantees only; durable restart needs a persistent
journal.
**[proven: both lifetime fixtures — a journal refusal that dispatches
nothing; a replayed answer while work is pending; an old answer while a
newer transition is held; an answer with the right id from another
scope; a proposal re-sent after its answer was lost, answered from the
journal and dispatched once. Each case fails with the proposer's
identity check or the committer's lookup removed.]**

What the fixtures do not establish, left to the cards that own it:
domain re-evaluation on a stale revision (06, 07, 09); every stale and
duplicate child or attempt case (05, 06, 09–11); delivery across an
off-thread binding (a bound organism's sockets) and restart (12–13, 19).
The two supplied diagnostic probes print, so `hale test` reports them
failed even when their assertions hold; the silent regressions in cards
01 and 02 are the evidence, not the probes.

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
| a handler-born child survives its `run()` and its handler | proven, card 03 |
| a delayed keyed reply reaches it; it advances to nested and delegated work | proven, card 03 |
| completion and parent shutdown reclaim it; births equal dissolves at volume | proven, card 03 |
| a duplicate terminal message cannot advance it twice | proven, card 03 |
| child → committer → acknowledgement → dispatch, and refusal → no dispatch | proven, card 03 |
| transition identity: repeated, old and foreign answers ignored; re-sent proposals answered once | proven, card 03 (review) |
| the shape works in a program that imports the DNA core, beside DNA's flow types | proven, card 03 |
| `release` is type-wide | proven, card 03 |
| definitions bind whole or refuse; capacity is checked before a node is built | proven, card 04 (#691) |
| every fact is read whole or not at all; a recipe binds whole or not at all | proven, card 05 (#693) |
| the join is by identity and kind against the admitted recipe; every transition has its basis; a cancelled ancestor fences all below it | proven, card 06 (#694) |
| one admitted attempt runs once: nothing before its durable admission, a settled one reused, a running one attached to, the reply's identity checked, the outcome persisted exactly before answering, a refused append stopping the path | proven, card 08 |
| the admission precedes the summary; a refusal requests nothing and is recorded (or reported unrecorded), a Task-bound one exactly at the decision's revision and never on another body's execution, one that reached no Task as `workflow.ask_refused` under `<ask>#<n>`, decided and appended at one revision, the same decision replayed and a new one ordinal-numbered; one ask id is one execution, decided and appended at one revision; a taken id is skipped and a contended one re-minted; only the position's owner admits; an admitted Task is never legacy edit work after a restart | proven, card 07 |
| one step alive across delayed replies: the member set registered and activated before a leaf is born; two leaves completing in either order; an immediate and a delayed reply leaving it waiting and reachable; a repeated answer and a stray one changing nothing; a failed member failing it at once and a sibling's later reply recorded without reopening it; reclaimed only once every member settled; a leaf retried within its allowance; every transition a proposal decided at one reading, the same row committed meanwhile standing once, a conflict refused, an invalid or refused append moving nothing | proven, card 09 |
| ordered steps advance only behind a committed completion: the original delayed two-step reproduction passes through the new engine (four requests then done; six while waiting, eight after the held replies); A → {B,C} → D in exact order under immediate, delayed, out-of-order and mixed replies; a duplicate completion births no second step; a failed member blocks D and the execution fails once its step drained; a cancellation fences the active step, requests nothing further and a late reply reopens nothing; an activation the record refuses dispatches nothing until the record resumes | proven, card 10 |
| recursive child workflows: the canonical three-level example runs through one engine; a delayed grandchild leaf keeps root D from starting; a failed grandchild fails its step, its execution, the child's step, the child, the root's step and the root, and D never runs; a child settlement for a leaf's key, another root's child, the wrong step, or delivered again changes nothing; two roots at once and two uses of one definition never share an id; a child's admission the record refused runs nothing until the record resumes; a child request naming the wrong step or asked again births nothing; the fence reaches the grandchild's leaf; cleanup from the leaves upward | proven, card 11 |
| restore of one Work: an incarnation booted over the record the crash left asks the record what it holds, re-proposes and the committer replays, so identities are the record's; cut before the attempt's admission, after it, after the claim, after the outcome, and after completion, each deterministic boot runs to the completed record row for row (the redelivery row aside); settled work never reruns and an open claim under a recorded outcome is closed, a refused close leaving the Work holding; redelivery is derived from the incarnation's own claims, is a row appended exactly at its reading (an outcome landing under it makes it stale and nothing runs), and happens once per incarnation, so a duplicate dispatch attaches and a request under another Work's name is nobody's; the old process's reply for the still-current attempt is accepted; a cancelled record with its admitted Work unsettled births the current step fenced, settles the Work cancelled and drains before leaving, and a cancelled-and-drained one births nothing; a cancellation heard while the record is being asked waits for the answer, and a cancellation landing with no step active asks the record again and drains what it still holds admitted through the current step, born fenced, before leaving; every path that answers from a recorded outcome — a request that finds it, the ordinary execution that runs into it on its own reading, an attachment that finds it, a duplicate terminal reply — closes the open claim as a request does, and a refused close is reported on every path and holds the Work, which asks again when the record resumes; an effect that resulted unknown stays unresolved; a completed record rebooted twice gains no row and births nothing | proven, card 12a |
| retries under a restart, and the fence: attempt zero fails, attempt one is admitted and, rebooted, resumed — redelivered — with no attempt two; a late success for attempt zero changes nothing; an admission the record refuses invokes no performer and spends no retry, and lands on resume; a holder whose lease expired (unclaimed, taken by another, or re-taken under its own name with a new token) commits no transition, records no reply and claims no request, a takeover landing between its check and its append fences that append, two takes of a free lease land one, and what it held goes through once it holds a live token again; the card 01 retry-identity regression passes on the stack merged with main | proven, card 12b |
| uncertain external effects at a restart: a durable adapter that recorded the invocation is asked before any redelivery, its record becomes the outcome and the effect is not performed twice; an adapter that cannot say leaves the claim resulted unknown, durably, with no blind replay, until a person resolves it and the resolution becomes the outcome; a documented idempotent adapter is replayed; one that attests it never acted is redelivered; an adapter's record that is not this attempt's — another performer, or another attempt from the right performer — records nothing and is never relabelled; a stale unknown append is decided again, so an unrelated move lands the unknown row after it and a resolution landing meanwhile is the outcome; reconciliation is routed by the admitted performer kind and its evidence must name that kind and identity, so another performer's negative evidence authorizes no replay; the assembly's restart rule leaves attempt claims to the runtime; `effect_outcomes_test.hl` stays green | proven, card 12c |
| recovering the tree: a restart asks with the task id and the performer kinds for attempts the record has not admitted, and the runtime answers the admission from the record, so a restored execution runs its bound recipe (revision 1 while the catalog offers revision 2, which a new admission runs); a task never admitted or admitted with an unreadable recipe is refused out loud and nothing is invented; restarts after one sibling succeeded, while a grandchild waits, after the child completed and before the parent's step did, and after the parent's step completed and before the next was dispatched, each complete with every attempt claimed once, every step activated once and the root settling last; the admission recovered is the one the projection accepted, never the last row naming the task (a refused re-admission or a cross-task row is not the execution; only refused rows is refused with the projection's reason); a child asked as a root is refused and recovered through its parent; an answer is taken only to the owner's outstanding question, for its task, once — strays birth nothing; an admitted attempt resumes under the record's performer kind, the asked kinds binding only unadmitted attempts; the workflow identity is the recipe's, which the projection holds to §4; a Work's state question carries its scope and an identity and the answer is taken once by both — a same-named Work in another scope's record takes nothing from that record's answer; a fence heard while the state question is out is decided after the answer, never by a settlement naming no attempt | proven, card 13 |
| exact plan and Mutation correlation: each plan row and each Mutation's own request row (`mutation.requested`, before `mutation.proposed`) bind the Work and attempt that asked, with the request as asked; two sibling edit Works get distinct plans and Mutations; a restart reconnects each Work to its own — in one memory or two — resumes only a Work without a Mutation, under its own plan and request (capability words, data class, attempt number as recorded, never a default), fails a Mutation with only its request row without minting its id again, and keeps every row from before the card at its Task-level meaning; a plan is taken for the Work and the Task it names; the binding carries every field of the request (context digest, knowledge bindings, output contract, cost ceiling included) and a restart dispatches them; an association write the record refuses starts nothing — the plan or request is held and re-driven, never settled on | proven, card 14 |
| delivery across off-thread bindings | not yet, card 19 |

Card 03 native runs: `HALE_BIN=target/release/hale HALE_DNA_SOURCE=$PWD
target/release/hale test dna/tests/<fixture>` with hale 0.20.0 built from
this tree.
