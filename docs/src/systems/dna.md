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
