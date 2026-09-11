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
about:

- `IsolatedWorktrees { repo, root }` — one `git worktree` per
  Mutation, named by its id under `.hale/dna/worktrees`, detached at
  the base commit; removed when the Mutation dissolves. Carries
  `worktree_io`. The base is recorded as the commit *and* the
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
