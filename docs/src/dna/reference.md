# Reference

## The CLI

```text
hale dna init [app-dir]      generate the organization (dna/org) for an existing application
hale dna new <name>          a greenfield application with its organization
hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain (and write a catalog for an organization from before it)
hale dna models [project]    the catalog (dna/org/models.hl): every backend, one small request to each
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
                             the organization AND the application under one host: rebuild and
                             restart the application on an apply, watch the window, report back
hale dna run [project] [--port N] [--no-iris] [--observe <secs>]
                             the organization only; the fleet ([dna] fleet), or a deployment
                             gateway, expresses the application
hale dna ui [project] [--port N]
                             the surface in a browser, from the record alone
hale dna status [project] [--json]
                             the status projection, from the record
hale dna ask [--to <locus>] [--no-wait] <intent…>
                             offer intent; over the membrane here, into the record otherwise
hale dna review              the pending Reviews
hale dna review <id> [--iris] render a Review: source diff, semantic diff, evidence (offline)
hale dna review <id> approve|revise|reject|abstain [--as <reviewer>] [--authority <a>]
                             [--comment <c>] [--digest <sha>] [--no-wait]
hale dna history [<entity>]  walk the record by causal links (offline)
hale dna sync [project]      fetch, reconcile and push the record (refs/dna/*)
hale dna board [project]     the Board's queue: verdicts needed, escalations, proposals, reports
hale dna report [project]    file a report from the record since the last one
hale dna pressure [raise <source> <what…>]
                             pressure raised and answered; `raise` publishes one signal
hale dna github sync         mirror pending Reviews to pull requests, read reviews back as verdicts
hale dna fleet [project]     what the fleet expresses: every instance, node, revision, hash, state
hale dna deploy <revision>   express a genome revision through the fleet's nodes
hale dna rollback <mutation> express the base a Mutation was applied on, again
hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]
                             run the instances a plan assigns to this node, from the record
hale fleet check [plan.json] [--in <dir>] [--if-declared]
                             compose and check; every declared fleet when no plan is named
```

Environment the host sets on the organization: `LOTUS_OBS=1`,
`HALE_BIN` (the toolchain it runs for verification), and on a
restart `HALE_DNA_RESTART_FOR` / `HALE_DNA_EXPRESSION`. A node sets
`HALE_DNA_NODE` and `HALE_DNA_INSTANCE` on each instance.
`HALE_DNA_ONESHOT` makes a generated application's `run()` return
after its first cycle (for tests). `ANTHROPIC_API_KEY` /
`OPENAI_API_KEY` are what `init` looks for when it writes the
catalog, and the `HostedCredential` sources it names. Git config: `dna.remote` (default
`origin`), `dna.github` (`owner/repo`), `dna.github.board` (logins).

## The record's vocabulary

One commit per event on `refs/dna/journal`; `journal.jsonl` in the
tree, one JSON object per line: `seq`, `kind`, `entity`, `body`,
`author`.

| kind | entity | body |
|---|---|---|
| `application.attached` | the seed | the entrypoint, the artifact's digests, the toolchain |
| `structure.observed` | `locus:X`, `topic:X`, `claim:X`, … | the compiler's model of it, `provenance: observed` |
| `responsibility.proposed` | `locus:X` | an inferred one-line responsibility, `ratified: false` |
| `law.deferred` | a clause | why `init` could not certify it |
| `intent.requested` | the intent id | an ask from a clone with no organization: outcome, from, to |
| `intent.offered` / `intent.refused` | the intent id | the outcome asked for / the refusal |
| `task.born` | `t<n>` | `<intent>: <outcome>` |
| `task.pending` / `task.done` / `task.failed` | `t<n>` | the Workflow detail, or the Work and performer that settled it |
| `mutation.proposed` | `m<n>` | `task t<n> <class>: <objective> (<target>) at <base>` |
| `mutation.worktree` | `m<n>` | `opened <path> at <base> …` / `removed` |
| `mutation.located` | `m<n>` | the files and the grant they were found under |
| `mutation.candidate` | `m<n>` | the candidate commit |
| `mutation.review` / `.stage` / `.escalate` / `.release` / `.deny` | `m<n>` | the boundary's disposition |
| `mutation.topology` | `m<n>` | the diff names a plan or the manifest: re-classed for the Board |
| `mutation.applied` | `m<n>` | the candidate commit |
| `mutation.retained` / `.rolled_back` / `.rejected` / `.revise` / `.refused` / `.failed` | `m<n>` | why |
| `effect.requested` / `effect.result` | an idempotency key | the gateway's record: `worktree.open:<id>`, `commit:<id>:<step>`, `apply:<candidate>`, `rollback:<id>:<base>` |
| `evidence.base` / `.fmt` / `.check` / `.verify` / `.test` / `.fleet` / `.replay` / `.rollback` / `.diff` | the candidate commit | `{step, ok, code, output_digest, bytes}`; the receipt is `refs/dna/receipts/<output_digest>` |
| `evidence.magnitude` | the candidate commit | the vector |
| `review.requested` | `review:<id>` | question, authority, candidate, base, shape, disposition, evidence, magnitude, diff digests, fitness signals |
| `review.verdict` | `<id>` | a verdict appended from a clone or from GitHub, in the reviewer's name |
| `review.settled` / `review.refused` | `<id>` | `<verdict> by <reviewer>` / the reason |
| `review.reasoned` | `<id>` | the deciding verdict's comment: a person's note, or the Leader's reasoning in full |
| `expression.restart_requested` | `m<n>` | `apply <candidate> seed <s> fitness …` or `rollback <base> seed <s> after …` |
| `expression.restarted` | `m<n>` | the shape and build the new expression reports |
| `expression.deployed` | `m<n>` | what a deployment gateway expressed, and its judgement |
| `expression.observed` / `expression.crashed` | `m<n>` | the window's outcome; `crashed` names the instance and node on a fleet |
| `fleet.deploy` | `m<n>` or a short revision | plan, revision, seed, the instances touched, reason |
| `instance.up` / `instance.exited` | the instance id | node, revision, model hash, build, pid / node, revision, code — authored `node/<name>` |
| `github.pr` / `github.commented` | `m<n>` | the pull request opened / the settlement commented |
| `pressure.raised` | a source | `<what> x<n>` |
| `pressure.remeasured` | `m<n>` | the Task, the declared fitness signals, the outcome |
| `appendage.proposed` / `appendage.candidate` | a source | the organ proposed / the organization mutation that proposes it |
| `report.filed` | `r<n>` | the summary since the last report |
| `model.called` | `<work>/a<n>` or a review id | the model evidence (never the prompt) |

## `status.json`

The projection `hale dna status --json` prints, `hale dna ui` serves
and iris renders: `organism`, `journal { ref, revision, chain }`,
`expression { attached, current, build_digest, toolchain, restarts,
last_restart_request, last_observed }`, `intents`, `tasks[]`,
`reviews[]` (a mutation's Review carries `mutation_id`,
`change_class`, `seed`, `disposition`, `base_commit`,
`candidate_commit`, `candidate_shape`, `evidence`, `magnitude`,
`diff_text`, `diff_json`, `author`), `mutations[]` (`id`, `task`,
`class`, `objective`, `disposition`, `candidate`, `events`),
`law_deferred`, `model_calls`.

## The files

| path | what |
|---|---|
| `vendor/dna/*.hl` | the core (toolchain-owned, git-ignored, pinned in `hale.lock`) |
| `dna/org/main.hl` | the organization |
| `dna/org/law.hl` | its law |
| `dna/org/purpose.hl` | the declared purpose |
| `refs/dna/journal` | the record |
| `refs/dna/receipts/<sha256>` | receipts by content digest |
| `refs/dna/lease/<key>` | leases with fencing tokens |
| `refs/dna/revisions/<rev>` | revisions a deploy asked for |
| `.hale/dna/` | sockets, `status.json`, `worktrees/<id>/`, `scratch/`, the artifacts as attached / running / before the last restart |
| `.hale/node/<name>/` | on a node: `<instance>.pid`, `<instance>.topology` |
| `<plan>.plan.json` | the fleet plan (schema 1.2: `seed`, `node` on an instance) |

## The core, by file

`vendor/dna/` after `init` (`dna/core/` in the hale repository):

| file | what |
|---|---|
| `assembly.hl` | `Dna` (the substrate), `Board`, `OrgPolicy` and the other review policies, `NoDeployment` / `ShellDeployment` / `LocalApplyDeployment` |
| `org.hl` | `Leader`, `SourceReader` |
| `journal.hl` | `Journal`, `MemJournal`, `GitJournal`, `Receipts` (`FileReceipts`, `GitReceipts`), `GitLeases`, the effect idempotency helpers |
| `process.hl` | `Task`, `Workflow`, `Step`, `Work`, `Attempt`, `Metabolism` |
| `work_system.hl` | `WorkSystem`, routing perspectives, the performers |
| `review.hl` | `Review`, `AutonomyBoundary`, authority ranks |
| `models.hl` | `ModelRouter`, `OpenAiChat`, `AnthropicMessages`, `LocalModel`, `FakeModel`, `HostedCredential` (with its `scheme`), `probe` |
| `budget.hl` | `BudgetPolicy`, `Budget` (the substrate's one counter) |
| `knowledge.hl` | semantic memory: ideas, edges, bindings |
| `workspace.hl` | `IsolatedWorktrees`, `LocalGit`, `MutationGateway` |
| `editing.hl` | `WorktreeTools`, `SourceEditor` |
| `verification.hl` | `HaleVerification`, `assess_structure` |
| `topics.hl` | the typed topics, including the four membrane topics |
| `types.hl` | `Intent`, `WorkRequest`, `Grant`, `Magnitude`, `Evidence`, `Disposition`, `Mutation`, `dispose` |

Beside the core, `dna/host` (the host: the projections, the writers,
`run` / `dev`, the node agent — everything `hale dna` does that is DNA
behaviour rather than manifest or scaffolding), `dna/membrane` (the
client it publishes through) and `dna/ui` (the surface) ship in the
toolchain the same way; `hale dna` resolves the project and execs the
host. The compiler keeps `init` / `new` / `upgrade`, `hale fleet
check` and the plan schema. The contract the library and the commands promise is
`spec/dna.md`. Friction the DNA has logged against the language and
the toolchain, with reproducers, is `dna/FRICTION.md`.
