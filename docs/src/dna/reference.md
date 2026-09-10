# Reference

## The CLI

```text
hale dna init [app-dir]      attach the DNA to an existing application
hale dna new <name>          a greenfield application with its DNA
hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain
hale dna run [project] [--port N] [--no-iris] [--observe <secs>]
                             build, run under LOTUS_OBS with iris attached, hold the membrane;
                             rebuild and restart on the organism's request, watch the window, report back
hale dna status [project] [--json]
                             the organism's status projection, from the Journal
hale dna ask [--to <locus>] <intent…>
                             offer intent over the membrane; prints the Task born or the refusal
hale dna history [<entity>]  walk the Journal by causal links (works offline)
hale dna review              the pending Reviews
hale dna review <id> [--iris] render a Review: source diff, semantic diff, evidence (works offline)
hale dna review <id> approve|revise|reject|abstain [--as <reviewer>] [--authority <a>] [--comment <c>] [--digest <sha>]
                             send a verdict over the membrane; the Review decides
```

Environment the host sets on the organism: `LOTUS_OBS=1`,
`HALE_BIN` (the toolchain the organism runs for verification),
and on a restart `HALE_DNA_RESTART_FOR` / `HALE_DNA_EXPRESSION`.
`HALE_DNA_ONESHOT` makes a `hale dna new` application's `run()`
return after its first cycle (for tests). `OPENAI_API_KEY` is the
default `HostedCredential` source in the generated Genome.

## The Journal's vocabulary

One JSON line per event: `seq`, `kind`, `entity`, `body`, `prev`,
`digest`, where `digest = sha256(prev | kind | entity | body)` and
the first `prev` is `genesis`. `hale dna status` reports the chain
as `verified` or `BROKEN`.

| kind | entity | body |
|---|---|---|
| `application.attached` | the seed | the entrypoint, the artifact's digests, the toolchain |
| `structure.observed` | `locus:X`, `topic:X`, `claim:X`, … | the compiler's model of it, `provenance: observed` |
| `responsibility.proposed` | `locus:X` | an inferred one-line responsibility, `ratified: false` |
| `law.deferred` | a clause | why `init` could not certify it |
| `intent.offered` / `intent.refused` | the intent id | the outcome asked for / the refusal |
| `task.born` | `t<n>` | `<intent>: <outcome>` |
| `task.pending` / `task.done` / `task.failed` | `t<n>` | the Workflow detail, or the Work and performer that settled it |
| `mutation.proposed` | `m<n>` | `task t<n> <class>: <objective> (<target>) at <base>` |
| `mutation.worktree` | `m<n>` | `opened <path> at <base> …` / `removed` |
| `mutation.located` | `m<n>` | the target file and the grant it was found under |
| `mutation.candidate` | `m<n>` | the candidate commit |
| `mutation.review` / `.stage` / `.escalate` / `.release` / `.deny` | `m<n>` | the boundary's disposition |
| `mutation.applied` | `m<n>` | the candidate commit |
| `mutation.retained` / `.rolled_back` / `.rejected` / `.revise` / `.refused` / `.failed` | `m<n>` | why |
| `effect.requested` / `effect.result` | an idempotency key | the gateway's own record: `worktree.open:<id>`, `commit:<id>:<step>`, `apply:<candidate>`, `rollback:<id>:<base>` |
| `evidence.base` / `.fmt` / `.check` / `.verify` / `.test` / `.replay` / `.rollback` / `.diff` | the candidate commit | `{step, ok, code, output_digest, bytes}` — the receipt is `.hale/dna/evidence/<output_digest>.txt` |
| `evidence.magnitude` | the candidate commit | the vector |
| `review.requested` | `review:<id>` | question, authority, candidate, base, shape, disposition, evidence, magnitude, diff paths, fitness signals |
| `review.settled` / `review.refused` | `<id>` | `<verdict> by <reviewer>` / the reason |
| `expression.restart_requested` | `m<n>` | `apply <candidate> fitness …` or `rollback <base> after …` |
| `expression.restarted` | `m<n>` | the shape and build the new expression reports |
| `expression.observed` / `expression.crashed` | `m<n>` | the window's outcome; `crashed` is appended by the host |
| `pressure.raised` | a source | `<what> x<n>` |
| `pressure.remeasured` | `m<n>` | the Task, the declared fitness signals, the outcome |
| `appendage.proposed` | a source | the organ proposed, not created |
| `model.called` | `<work>/a<n>` | the model evidence (never the prompt) |

## `status.json`

The projection `hale dna status --json` prints and iris renders:
`organism`, `journal { path, revision, chain }`, `expression {
attached, current, build_digest, toolchain, restarts,
last_restart_request, last_observed }`, `intents`, `tasks[]`,
`reviews[]` (a mutation's Review carries `mutation_id`,
`change_class`, `disposition`, `base_commit`, `candidate_commit`,
`candidate_shape`, `evidence`, `magnitude`, `diff_text`,
`diff_json`, `author`), `mutations[]` (`id`, `task`, `class`,
`objective`, `disposition`, `candidate`, `events`),
`law_deferred`, `model_calls`.

## The files

| path | what |
|---|---|
| `vendor/dna/*.hl` | the core (toolchain-owned, git-ignored, pinned in `hale.lock`) |
| `dna/assembly.hl` | the Genome |
| `dna/purpose.hl` | the declared purpose |
| `<seed>/dna_constitution.hl` | the law |
| `.hale/dna/journal.jsonl` | the Journal |
| `.hale/dna/baseline.topology`, `current.topology`, `previous.topology` | artifacts: as attached, as running, before the last restart |
| `.hale/dna/status.json` | the projection |
| `.hale/dna/hale-dna.review.verdict.sock`, `…intent.offered.sock`, `…expression.observed.sock` | the membrane |
| `.hale/dna/worktrees/<id>/` | one worktree per open Mutation |
| `.hale/dna/evidence/` | receipts by sha256; `<id>.base.topology.json`, `<id>.candidate.topology.json`, `<id>.diff.json`, `<id>.diff.txt` |

## The core, by file

`vendor/dna/` after `init` (`dna/core/` in the hale repository):

| file | what |
|---|---|
| `assembly.hl` | `Dna` (the assembly), the membrane, review policies, deployment gateways |
| `journal.hl` | `Journal`, `MemJournal`, `FileJournal`, coordination leases, the effect idempotency helpers |
| `process.hl` | `Task`, `Workflow`, `Step`, `Work`, `Attempt`, `Metabolism` |
| `work_system.hl` | `WorkSystem`, routing perspectives, the performers |
| `review.hl` | `Review`, `AutonomyBoundary` |
| `models.hl` | `ModelRouter`, `HostedModel`, `LocalModel`, `FakeModel`, `HostedCredential` |
| `knowledge.hl` | semantic memory: ideas, edges, bindings |
| `workspace.hl` | `IsolatedWorktrees`, `LocalGit`, `MutationGateway` |
| `editing.hl` | `WorktreeTools`, `SourceEditor` |
| `verification.hl` | `HaleVerification`, `assess_structure` |
| `topics.hl` | the typed topics, including the three membrane topics |
| `types.hl` | `Intent`, `WorkRequest`, `Grant`, `Magnitude`, `Evidence`, `Disposition`, `Mutation`, `dispose` |

Friction the DNA has logged against the language and the toolchain,
with reproducers, is `dna/FRICTION.md` in the hale repository.
