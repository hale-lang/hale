# DNA: a governed application

**DNA** is the part of a Hale application that decides how the
application itself changes: what may be attempted, who reviews it,
what evidence counts, and what is never applied without a human.
It is ordinary Hale source. The core (`dna/core` in the hale
repository) ships inside the toolchain; a project materializes it
and owns its own assembly on top.

```sh
hale dna new demo          # a greenfield application with its DNA
hale dna init .            # attach the DNA to an existing application
hale dna upgrade           # re-materialize vendor/dna for this toolchain
```

## What `init` produces

| path | owner | what |
|---|---|---|
| `vendor/dna/*.hl` | toolchain | the DNA core, pinned in `hale.lock` as `[dna] toolchain` |
| `dna/assembly.hl` | project | the `Genome`: the `Dna` constructor with local defaults, and the baseline `Review` |
| `dna_constitution.hl` (in the app's seed) | project | the groups and `constitution Project` |
| `dna/purpose.hl` | project | the declared purpose the first Review ratifies |
| `.hale/dna/journal.jsonl` | the organism | the Journal, seeded from the compiler's model |
| `.hale/dna/baseline.topology` | the organism | the artifact it was seeded from |

The application's `main locus` gains the imports, a `genome` param,
`adopt Project;` and the two membrane bindings;
`hale.toml` gains `[claims] base = "Project"` and a `local`
environment so `hale check --matrix` judges the entrypoint against
the law. Nothing existing is rewritten, and re-running keeps every
file.

There is no configuration file. Every "setting" is a constructor
argument in `dna/assembly.hl`:

```hale,fragment
core: dna::Dna = dna::Dna {
    journal: dna::FileJournal { path: ".hale/dna/journal.jsonl" },
    boundary: dna::AutonomyBoundary { child: "demo", grant: dna::Grant { classes: "refactor docs", max_magnitude: 4, review: "pre" } },
    review_policy: dna::HumanBeforeApply { },
    deployment: dna::NoDeployment { },
    membrane: dna::LocalHumanMembrane { who: "operator" }
};
```

`hale check` sees all of it, which is the point: the law in
`dna/constitution.hl` is evaluated over the assembly you actually
constructed, not over a policy you wrote in prose.

## The seeded Journal

`init` cuts the application's topology artifact and writes one
event per fact the compiler can vouch for, with provenance:

- `application.attached` — the entrypoint, the artifact's digests, the toolchain;
- `structure.observed` — one per locus (params, methods, publishes, subscribes, supervision, instances), topic, binding, effect class and claim — provenance `observed`;
- `responsibility.proposed` — one guess per locus, from what it reacts to and emits — provenance `inferred`, `ratified: false`, never anything stronger;
- `law.deferred` — a clause the application cannot certify yet (see below);
- `review.requested` — the baseline review: ratify `dna/purpose.hl`, pinned to the sha256 of its text.

The chain is the same the core's `FileJournal` writes (each row's
digest covers the previous digest), so the organism rehydrates it
at birth and continues it.

## The law an application can carry

`constitution Project` lives in the application's own seed, because
a constitution names groups its adopting entrypoint must declare
(`organism`) and whoever imports the application (its tests) must
see the law and its vocabulary together. It starts from the core's
own law: a mutation
is applied only through the assembly's gate, performers never
apply, credentials stay sealed. The assembly-scoped clause
(`apply_gated`, quantified from the `Genome`) always holds. The
application-wide clause (`organism_gated`) is generated **active
only when the baseline artifact has no unresolvable edges**: a
`forbid reaches` over an application with an indirect call fails
closed, and a law that fails the application on day one is not a
law anyone keeps. Otherwise it is written out commented with the
reason, and the deferral is journaled. Resolve the edges and
uncomment it.

In Phase 1 the deployment gateway is `NoDeployment`: every
mutation stops at `staged`. That is the constructor, and it is
also the law: `phase1_read_only: forbid reaches(genome,
effects(genome_apply))` holds against the assembly as constructed,
so swapping the gateway in `dna/assembly.hl` is a law change a
reviewer sees, not a constructor detail.

## Models

The core ships one adapter over an OpenAI-compatible chat endpoint,
behind the same `ModelBackend` interface as its fakes, in two
shapes the law can tell apart:

- `HostedModel` — a prompt leaves the process. `complete` carries
  the `external_model` effect class, so a claim can keep customer
  data away from it structurally; the API key is read from the
  environment into a **sealed** `HostedCredential` that presents it
  on the wire and never returns it. Without a credential the model
  is not a permitted backend, and the router refuses before the
  wire.
- `LocalModel` — the same wire to a local endpoint (an inference
  server on this host), no credential, no `external_model`, any
  data class.

Every call publishes `ModelCalled` and the assembly journals it
as `model.called`: adapter and endpoint, the credential's
fingerprint, requested and reported model, parameters as sent,
prompt and context digests (never the prompt), the knowledge
bindings used, tool grant, response digest, tokens, wall time,
cost, validation, retry lineage, data class, and the refusal when
there was one. `hale dna history w1/a0` shows an attempt's calls.

## Running it

```sh
hale dna run            # in the project; --port N for iris, --no-iris to skip it
```

`run` is a **stateless host**. It cuts a fresh artifact of what is
about to run (`.hale/dna/current.topology`), builds, execs the
organism under `LOTUS_OBS=1` from the project root, waits for the
membrane sockets, and attaches iris with the law view on the fresh
artifact, the review view diffing it against the baseline `init`
cut (which is the application as `init` found it, so the first run's
review shows exactly what the DNA added), and the membrane panel. It holds no Task state: an intent
offered through the membrane lands in the organism's Journal, and
when the organism exits the host reaps iris and exits with the
organism's code. The Journal is the record either way.

## Talking to it

```sh
hale dna status [--json]        # the status projection, from the Journal
hale dna ask "write the changelog"
hale dna review purpose approve --as riley --comment "ratified"
hale dna history [t1]           # the Journal, or one entity's causal history
```

`status` reads the Journal: tasks born and settled, pending Reviews
and why (the authority they need, the question), staged mutations,
the expression identity (the artifact `init` attached, the artifact
that would run now, the build digest), the chain's integrity, and
whether an organism is currently bound to its membrane. It works
offline and says so.

`ask` publishes a typed `IntentOffered` through the embedded
membrane client and reads the organism's answer back from the
Journal: the Task it birthed, or the refusal. `review` does the same
with a `Verdict` on a pending Review, using the candidate digest the
Journal recorded for the request; the Review still checks it, the
reviewer's authority and independence, and the Journal records
`review.settled` or `review.refused` with the reason. `history`
walks the Journal from an intent, Task or Review id through the
rows that link to it. None of these decide anything: the host
publishes and reads.

## Where a mutation edits

Phase 2 (Track D) lets an Attempt change the genome. Two gateways in
the core carry that, each with the effect class the law reasons
about. Neither holds a Journal of its own: the assembly hands its
one Journal to every gated call, so the gateway's events sit in the
same chain as everything else and cannot be wired with a private one.

- `IsolatedWorktrees { repo, root }` — one `git worktree` per
  Mutation, named by its id under `.hale/dna/worktrees`, detached at
  the base commit, with the repository's `vendor/` copied in (it is
  toolchain-managed and ignored by git); removed when the Mutation
  dissolves. Carries `worktree_io`. The base is recorded as the commit *and* the
  `exec_digest` of the checked build it was expressed as.
- `LocalGit { repo }` — `commit_all`, `diff`, `contains`, and
  `apply`. `apply` fast-forwards the repository to the exact reviewed
  candidate (a non-descendant is merged) and is the one method
  carrying `genome_apply` beside `repo_write`, so the constitution
  can forbid every path to it except through the assembly's gate.

`MutationGateway` puts every operation behind the Journal and a
lease: journaled before dispatch under an idempotency key
(`worktree.open:<id>`, `commit:<id>:<step>`, `apply:<candidate>`),
the result appended once, so a crashed and retried apply reads the
recorded result and never commits twice; and fenced by the
Attempt's lease on `mutation:<id>`, so a stale fencing token is
refused rather than applied. `hale dna history m1` shows the
worktree, candidate and apply events of a Mutation.

### The Attempt that edits

`SourceEditor` is the performer that changes source. Its hands are
`WorktreeTools { root, hale_bin }`: `read` and `edit` of
worktree-relative paths (an absolute path or a `..` segment is
refused and counted), and `fmt` and `check`, which run the toolchain
against the worktree (effect class `toolchain_run`). That is the
whole grant. The editor holds no repository, no worktree gateway, no
deployment and no Knowledge — not as a runtime check but by
construction, and a constitution says so:

```hale,fragment
group editors = { dna::SourceEditor, dna::WorktreeTools };
group knowledge = { dna::Knowledge };
constitution Editing {
    editors_never_commit: forbid reaches(editors, effects(repo_write));
    editors_never_touch_worktrees: forbid reaches(editors, effects(worktree_io));
    editors_never_apply: forbid reaches(editors, effects(genome_apply));
    editors_never_learn: forbid reaches(editors, knowledge);
}
```

A wiring that hands the editor a `LocalGit` fails `hale check` with
the witness path (`dna/tests/law/editor_confined_fail`).

One `perform` is two model calls under one attempt id — the quick
tier rewrites the target file to the objective, the deep tier says
which fitness signals the change should move — then `fmt`, then
`check`. The result is a `MutationProposal` in `WorkResult.result`:
the files changed, the rationale, the expected fitness signals, and
whether the proposal formats and checks; the toolchain's exit codes
and the two backends are the `evidence`. A proposal that does not
check is a failed Attempt carrying the diagnostics, never a
candidate. The candidate commit is the gateway's business, not the
editor's.

### Evidence with receipts

`HaleVerification { hale_bin, evidence_dir, recording }` runs the
toolchain over a candidate's worktree — `hale fmt --check`, `hale
check --json --dump-topology`, `hale verify --json`, `hale test`,
`hale model diff <base> <candidate>` (the base artifact cut from the
genome's seed at the same moment, so the diff is against what the
candidate actually changed), and `hale replay --feed` when the
project has a recording — and keeps every result as
content-addressed evidence: the step's output goes to
`.hale/dna/evidence/<sha256>.txt`, and an `evidence.<step>` event
on the candidate commit carries the exit code and that digest
(`hale dna history <candidate>` lists them). The `Evidence` the
boundary and the Review read (`check_clean`, `verify_clean`,
`tests_pass`, `replay_ok`) is therefore a set of facts with
receipts. A project with no tests has no test evidence, only an
exit code; a candidate that does not check has no artifact, no
diff, and a default magnitude — and the report says so.

`assess_structure` turns the semantic diff into the **magnitude
vector** of #521, never a score: affected loci, parent-facing
contract change, effects widened, law touched, placement or
ownership change, state migration, external blast radius (a
declared effect class newly reached), reversibility, and novelty
against the accepted lineage (coarse for now: unprecedented until
the lineage has applied anything). It is journaled as
`evidence.magnitude` beside the tool receipts. For that vector to
see an added locus that reaches a declared class, `hale model
diff` now reports one-sided fns with effects as rows of their own
(`+ fn Mailer::on_ping reaches mail`).

### The Review that blocks

`Dna.mutate(task, class, objective, target, now)` is the pipeline up
to the human: open a worktree at the genome's head (the gateway,
journaled and fenced), point the editor's grant at it and let it
propose, commit the candidate, verify it with receipts, and put it
under a `Review` — a child of the assembly, keyed on the mutation's
id, **pinned to the exact candidate commit**. Every disposition the
boundary computes is recorded (`mutation.review`, `mutation.stage`,
`mutation.escalate`), and in Phase 2 every one of them still blocks
on a human: the verdict is the assurance the grant cannot supply. A
candidate that does not check is `mutation.deny` and never a
candidate.

The `review.requested` event carries what a reviewer decides on: the
question, the required authority, the base and candidate commits,
the candidate's shape hash, the disposition, the evidence steps, the
magnitude vector, and where the semantic diff lives. So the review
renders **offline**:

```sh
hale dna review            # the pending Reviews, one line each
hale dna review m1         # source diff · semantic diff · evidence table · magnitude
hale dna review m1 --iris  # the same diff in iris [4], beside the status and the membrane form
```

A verdict of `approve` is also the apply: see "Apply, restart,
observe" below.

The source diff is git's (base to candidate); the semantic diff is
`hale model diff --text` read from its receipt; the evidence table is
one row per toolchain step with its exit code and the digest of its
receipt. The kill test (`dna/kill-test/`) is why all three are shown
together: the semantic view decides law and structure, the source
view decides handler behaviour, and neither is offered alone.

A verdict names the candidate the reviewer looked at:

```sh
hale dna review m1 approve --as riley --comment "fine"
hale dna review m1 approve --digest <sha>     # state it explicitly
```

The Review, not the transport, admits it: a digest other than the
pinned candidate is refused (`review.refused`: `digest mismatch`), as
is an authority that does not satisfy the requirement or a reviewer
who authored the candidate. The settlement is a Journal event
(`review.settled`: `approve by riley`), and the mutation's status
moves to `reviewed` or `rejected`.

The Journal is the authority for the Review's existence too: an
organism started after the request — or restarted mid-review —
re-births every pending mutation Review from `review.requested`
events at birth (`rehydrated` in `Dna.status()`), so the answer can
always be given. The purpose Review is the Genome's own static child
and is not rehydrated.

### Apply, restart, observe

Approval applies **exactly the reviewed candidate**. When a
mutation's Review settles `approve`, the assembly checks that the
worktree's head is still the commit the reviewer looked at — a
candidate that moved after the review is refused by digest
(`mutation.refused`) — takes the Mutation's lease, and applies
through the gateway: fast-forward, or a merge of the pinned commit,
journaled under the candidate before dispatch, so a crashed and
retried apply reads the recorded result and never commits twice. A
merge that does not complete is aborted; there is no half-applied
state. Rejection or revision journals `mutation.rejected` /
`mutation.revise` and leaves genome and expression untouched; the
Mutation, its Attempts and its evidence stay in lineage.

The applied genome is not yet the running expression. The assembly
journals `expression.restart_requested` (with the fitness signals
the proposal declared) and `hale dna run` answers it: cut a fresh
artifact, rebuild, terminate the old organism, start the new one
with `HALE_DNA_RESTART_FOR=<id>`, which the new expression journals
as `expression.restarted` at birth, and relaunch iris with the diff
from the previous artifact to the current one. Then the
**observation window** (`--observe <secs>`, default 15): the
expression must stay up. At the end the host reports on the
membrane — a third typed topic, `ExpressionObserved` — and the
organism decides: `healthy` retains the Mutation
(`mutation.retained`) and dissolves its worktree; anything else
rolls the genome back to the Mutation's base (`git reset --keep`,
journaled as `mutation.rolled_back`) and asks for the old expression
back. Only an applied Mutation is judged; a late report changes
nothing.

If the expression exits inside the window there is nobody left to
decide, so the host accounts for it explicitly: `expression.crashed`
and `mutation.rolled_back` are appended by the host (the Journal has
one writer at a time, and the organism is gone), the base is
rebuilt and restarted. If the candidate does not even express
(build fails), the host reports `build_failed` and the running
organism rolls back the same way.

`hale dna status` shows every mutation by id with its disposition
(`proposed`, `review`, `stage`, `reviewed`, `applied`, `retained`,
`rolled_back`, `rejected`, `refused`, `failed`) and the expression's
restarts; `hale dna history m1` is the whole lineage of one
Mutation, receipts included.

### The twelve steps, in CI

`dna/acceptance/chat-server` is the site's chat server kept in the
repository as the small application the acceptance scenario governs
(`crates/hale-cli/tests/dna_twelve_steps.rs`, about ten seconds):

1. `hale dna init` attaches the DNA; `hale check --matrix` and the
   app's own tests still pass.
2. `hale dna run` holds the organism and its membrane (iris reads
   the same status projection the test asserts on).
3. `hale dna ask "document the chat server in main.hl"`.
4. The membrane accepts the intent and births Task `t1`
   (`intent.offered`, `task.born`).
5. Its Workflow makes a Step whose Work is routed back to the
   assembly as source-editing Work; the editor's Attempt locates the
   file under its grant (`mutation.located`, and every model call's
   evidence carries the grant).
6. The Attempt proposes the Mutation in its own worktree
   (`mutation.worktree`, `mutation.candidate`).
7. The receipts: `base`, `fmt`, `check`, `verify`, `test`,
   `rollback` (rehearsed in the worktree), `diff`, `magnitude`.
8. The boundary's disposition (`escalate` here: `application` is
   outside a `refactor docs` grant) and the blocking Review.
9. `hale dna review` and `hale dna review m1` render it; `hale dna
   status --json` is what iris shows.
10. `approve` applies the exact candidate; a second intent rejected
    leaves `git log` where it was.
11. The host rebuilds and restarts; the new expression journals
    itself; the window is observed and the originating pressure
    re-measured (`pressure.remeasured`, against the fitness signals
    the proposal declared).
12. `mutation.retained`; `hale dna history m1` is the whole lineage.

Two things the run found. An imported `main locus` used to bind its
`bindings { }` in whatever program imported it — the application's
own tests, run by the DNA's verification, took the organism's
membrane sockets from under it; bindings now belong to the
program's own entry main only. And behind a bound organism's
off-thread bus, a routed Work cannot await its answer (FRICTION
F.15): the Task's live pass settles `pending`, and the assembly
settles the durable Task in the Journal when the Work is done.

### The four extensions

`dna/tests/extensions_test.hl` runs them in one process:

- **Evolve the Workflow definition** (`Metabolism.evolve_workflow`)
  while a Task is active: the Task finishes under the revision it
  was born with (`Settled.revision`); the next Task adopts the new
  one.
- **Post-review autonomy** for a reversible internal refactor:
  under a `review: "post"` grant with `PostReviewRefactors`, a
  candidate inside the grant with no hard boundary and sufficient
  evidence is applied first (`mutation.release`, the boundary's
  audit says so) and reviewed after; a rejection rolls it back.
- **Widening the child's effects** crosses a hard boundary: review,
  never release, whatever the grant; and the child cannot expand its
  own grant (`expand` from the child is refused and audited).
- **A second persistent pressure** (`PressureRaised` from one source
  past the threshold) journals `appendage.proposed` and notifies the
  membrane; nothing is grown. An organ is asked for as intent and
  becomes a Mutation under review like any other.

The dogfood run on a real, larger application (the track names its
candidate) is a maintainer's session, not CI: the same commands, a
hosted model behind `OPENAI_API_KEY`, and the human in the loop.

## In iris

`hale dna run` hands iris three sources: the observation segment
(what the expression does), the artifact (what the genome says),
and the organism's **status projection**, a JSON file the host
re-projects from the Journal once a second. Perspective **[5]**
renders that projection — tasks and their state, pending Reviews
and why, staged mutations, model calls, the expression identity —
and tints the DNA lineage tower on the canvas (Task, Workflow,
Step, Work, Attempt), keyed on the core's type names rather than on
your application's. Imported loci are observed under their
author-facing names (`dna::Dna`, not a mangled symbol), so the
tower is visible and joins with the artifact.

## The membrane

The organism binds two typed topics on unix sockets under
`.hale/dna/`: `dna.review.verdict` and `dna.intent.offered`. Iris
(`hale iris --membrane .hale/dna`) and `hale dna ask` publish on
them; the Review checks the reviewer's authority, and intent goes
through the membrane gate. See [Iris](./iris.md).
