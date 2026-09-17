# Reference

## The CLI

```text
hale dna init [app-dir]      generate the organization (dna/org) for an existing application
hale dna new <name> [--profile local|remote-body --remote <url> [--body <user@host>]]
                             a greenfield application with its organization; the profile sets pieces
hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain (and write a catalog for an organization from before it)
hale dna models [project]    the catalog (dna/org/models.hl): every backend, one small request to each
hale dna knowledge [project] [--port N]
                             the knowledge service in the foreground (HALE_DNA_KNOWLEDGE_DSN: postgres://…, or memory)
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
                             the organization AND the application under one host: rebuild and
                             restart the application on an apply, watch the window, report back
hale dna run [project] [--port N] [--no-iris] [--observe <secs>]
                             the organization only; the fleet ([dna] fleet), or a deployment
                             gateway, expresses the application
hale dna ui [project] [--port N]
                             (under `git config dna.principal oidc`: a hosted head behind sign-in at
                             dna.oidc.issuer; dna.oidc.client, dna.oidc.redirect, dna.oidc.member
                             "<subject>=<name>", dna.oidc.board, HALE_DNA_OIDC_SECRET)
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
hale dna queue [submit]      the requests kept here while the service could not be reached; send them
hale dna ledger [status | adopt | abandon --why <w>]
                             the operational memory: where the day's work lives, and the one-way move of it into the store
hale dna candidates [<mutation> | drop <mutation> --why <w>]
                             the candidates the record keeps; one as a diff; stop keeping one
hale dna profile [project]   the organism's combination, detected from its pieces
hale dna body                who runs this record (the body lease); `claim --force` takes it from a
                             body that is gone; `release [--force]` gives it up; rows in your name
hale dna body provision <user@host> [--dsn <url>] [--dir <path>] [--dry-run]
                             a body over ssh: the pinned toolchain, the record cloned, Postgres from
                             compose or the DSN, a systemd user unit; writes nothing it cannot finish
hale dna body start|stop|logs [--body <user@host>]
                             the body's unit, over ssh
hale dna receipt [disclose <digest> --to <who> --purpose <p> | show <digest> --purpose <p>]
                             protected evidence: kept by the knowledge service alone; disclosure
                             and every read are rows in the reader's name
hale dna receipt hold|release-hold <digest> --why <w> | redact <digest> --why <w> --policy <p>
hale dna receipt file <path> [--class internal|customer|confidential] [--as <who>]
hale dna practice [propose <name> --text <text> [--because <why>] [--supersedes <digest>] [--as <who>]]
hale dna task done <id> [--as <who>] [--note …] [--evidence <digest> | --exception <why> --authorized-by <who>]
hale dna task authorize <id> --exception <why> [--as <authorizer>]
hale dna task decide <id> --decided-by <party> --via <channel> --evidence <digest> [--note …] [--as <reporter>]
hale dna connect [<record-url> --name <n> --as <position> --purpose <p> --classes <internal,customer,confidential> [--by <who>]]
hale dna disconnect <n> --why <why> [--by <who>]
hale dna handoff [<n> task <id> | <n> receipt <digest> [--note …] [--as <who>] | accept <id> [--note …] [--as <who>] | sync]
                             a hold refuses redaction; a redaction removes the body, keeps the digest
hale dna schedule [pause <id> | resume <id>]
                             the schedules the org chart declared, as the record has them;
                             pause and resume are rows in your name
hale dna secret set <NAME> [--body <user@host>]
                             a credential from stdin into ~/.config/hale-dna/<project>-<record>.env there or
                             here; `secret rotate <NAME>`; the record gets `secret.rotated` only
hale dna board [project]     the Board's queue: verdicts needed, escalations, proposals, reports
hale dna report [project]    file a report from the record since the last one
hale dna concern raise <source> <what…> [--severity N]
                             a concern from a locus path about the part above it; three become a proposal
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

The **memory** column is where the row lives once the organism has
adopted the ledger (`hale dna ledger adopt`, routing 1): `record` is
git, `ledger` is the store behind the knowledge service. Before
adoption every kind is the record's, and the two are read as one
sequence either way — see [The record](./record.md).

| kind | memory | entity | body |
|---|---|---|---|
| `application.attached` | record | the seed | the entrypoint, the artifact's digests, the toolchain |
| `structure.observed` | record | `locus:X`, `topic:X`, `claim:X`, … | the compiler's model of it, `provenance: observed` |
| `responsibility.proposed` | record | `locus:X` | an inferred one-line responsibility, `ratified: false` |
| `law.deferred` | record | a clause | why `init` could not certify it |
| `intent.requested` | ledger | the intent id | an ask from a clone with no organization: outcome, from, to |
| `intent.offered` / `intent.refused` | ledger | the intent id | the outcome asked for, and who asked (`… (from alice)`, a schedule, an optimizer) / the refusal |
| `intent.unrecovered` | ledger | the intent id | offered before a restart with no Task born; never re-offered, because work may already have run |
| `candidate.dropped` | record | the mutation | `by`, `why`: the candidate's pointer is no longer kept (applied at every clone's sync) |
| `ledger.adopting` / `ledger.adopted` / `ledger.abandoned` | record | `ledger` | the move of the day's work into the store: `ledger` (the service), `routing`, `checkpoint` (the record head the copy was taken at), `rows`, `by` |
| `task.born` | ledger | `t<n>` (`<owner>:t<n>` over a shared record) | `<intent>: <outcome>` |
| `task.planned` | ledger | `t<n>` | the plan the Task is worked under |
| `task.handed` | ledger | `t<n>` | handed to a person: `work`, `assignee`, `by`, `narrative`, `obligation`, `acceptance`, `evidence_required` |
| `task.reassigned` | ledger | `t<n>` | the assignment moved: `to`, and who moved it |
| `task.resumed` | ledger | `t<n>` | re-entered after a restart, under the plan already recorded — never replanned |
| `task.pending` / `task.done` / `task.failed` | ledger | `t<n>` | the Workflow detail, or the Work and performer that settled it |
| `mutation.proposed` | record | `m<n>` | `task t<n> <class>: <objective> (<target>) at <base>` |
| `mutation.worktree` | record | `m<n>` | `opened <path> at <base> …` / `removed` |
| `mutation.located` | record | `m<n>` | the files and the grant they were found under |
| `mutation.candidate` | record | `m<n>` | the candidate commit |
| `mutation.review` / `.stage` / `.escalate` / `.release` / `.deny` | record | `m<n>` | the boundary's disposition |
| `mutation.topology` | record | `m<n>` | the diff names a plan or the manifest: re-classed for the Board |
| `mutation.applied` | record | `m<n>` | the candidate commit |
| `mutation.apply_retried` | record | `m<n>` | the apply ran again on a review that was already settled |
| `mutation.retained` / `.rolled_back` / `.rejected` / `.revise` / `.refused` / `.failed` | record | `m<n>` | why |
| `effect.requested` / `effect.result` | ledger | an idempotency key | the gateway's record: `worktree.open:<id>`, `commit:<id>:<step>`, `apply:<candidate>`, `rollback:<id>:<base>` |
| `evidence.base` / `.fmt` / `.check` / `.verify` / `.test` / `.fleet` / `.replay` / `.rollback` / `.diff` | record | the candidate commit | `{step, ok, code, output_digest, bytes}`; the receipt is `refs/dna/receipts/<output_digest>` |
| `evidence.magnitude` | record | the candidate commit | the vector |
| `review.requested` | record | `review:<id>` | question, authority, candidate, base, shape, disposition, evidence, magnitude, diff digests, fitness signals |
| `review.verdict` | record | `<id>` | a verdict appended from a clone or from GitHub, in the reviewer's name |
| `review.settled` / `review.refused` | record | `<id>` | `<verdict> by <reviewer>` / the reason |
| `review.reasoned` | record | `<id>` | the deciding verdict's comment: a person's note, or the Leader's reasoning in full |
| `org.reviewed` | record | the organization | the organization's own pass over itself, and what it answered |
| `optimize.refused` | ledger | the organization | that pass did not run: the budget for the window is spent |
| `expression.restart_requested` | record | `m<n>` | `apply <candidate> seed <s> fitness …` or `rollback <base> seed <s> after …` |
| `expression.restarted` | record | `m<n>` | the shape and build the new expression reports |
| `expression.deployed` | record | `m<n>` | what a deployment gateway expressed, and its judgement |
| `expression.observed` / `expression.crashed` | record | `m<n>` | the window's outcome; `crashed` names the instance and node on a fleet |
| `fleet.deploy` | record | `m<n>` or a short revision | plan, revision, seed, the instances touched, reason |
| `instance.up` / `instance.exited` | ledger | the instance id | node, revision, model hash, build, pid / node, revision, code — authored `node/<name>` |
| `github.pr` / `github.commented` | record | `m<n>` | the pull request opened / the settlement commented |
| `pressure.raised` | ledger | a source | `<what> x<n>` |
| `pressure.remeasured` | ledger | `m<n>` | the Task, the declared fitness signals, the outcome |
| `appendage.proposed` / `appendage.candidate` | record | a source | the organ proposed / the organization mutation that proposes it |
| `report.filed` | ledger | `r<n>` | the summary since the last report |
| `grant.contracted` | record | a child | authority narrowed, and what it leaves: `to`, `epoch` |
| `grant.refused` | record | a child | a grant born wider than its ceiling: authority |
| `grant.reservation_refused` | ledger | a child | a spend the window would not admit, naming the field: money |
| `spend.reserved` | ledger | the allocation (op) | a spend admitted: child, amount, currency, counterparty, route, ceiling, epoch, at, funder, account — reserved once by the store's claim (GH #668) |
| `spend.settled` | ledger | the allocation | one attempt's actual consumption: child, attempt, spent; every attempt is retained |
| `spend.compensated` | ledger | the allocation | money that came back, authorized by name: child, attempt, amount, by |
| `grant.reserved` / `grant.released` | ledger | a child | a reservation and its settlement from before GH #668 (`op` in the body); read as above |
| `grant.fenced` | ledger | a child | an admission refused because the grant's epoch moved since |
| `receipt.classified` | ledger | a digest | a protected body the knowledge service keeps: class, by, store |
| `receipt.withheld` | ledger | a digest | a protected body no service could keep: class, by, why |
| `receipt.disclosed` | ledger | a digest | a reader authorized: recipient, purpose, by |
| `receipt.read` / `receipt.read_refused` | ledger | a digest | a read in the reader's name, or its refusal: by, purpose, class |
| `receipt.filed` | ledger | a digest | an internal document filed as evidence: by, name, bytes, class, store |
| `knowledge.proposed` / `knowledge.ratified` / `knowledge.declined` / `knowledge.refused` | record | the practice's digest | a practice through its review: class, target, and the verdict that settled it |
| `knowledge.retired` | record | the practice's digest | superseded by a later version, when that one is ratified |
| `knowledge.consulted` | ledger | `m<n>` or the work | what this piece of work looked up, and the digests it read |
| `person.retired` | record | `<who>` | someone left: by, the successor their handed Tasks went to |
| `practice.requested` / `practice.proposed` / `practice.refused` | record | a request id | a person's practice proposal: requested in the record (name, text, by, because, supersedes), proposed by the organization (name, digest, review_id, by, because, supersedes), or refused (why) |
| `exception.authorized` | ledger | `t<n>` | an exception to a Task's acceptance condition, authorized in the authorizer's own name: task, why, by, practice |
| `completion.linked` / `completion.excepted` | ledger | `t<n>` | a person's completion under its acceptance condition: the evidence linked (task, evidence, by, practice), or an exception someone else authorized (task, why, authorized_by, by, practice) |
| `connection.proposed` / `connection.closed` | record | `connection:<n>` | a connection to another record: name, url, peer (its genesis), position, purpose, classes, by, review_id; closed: by, why |
| `handoff.received` | ledger | `handoff:<id>` | an envelope in the receiving record's mailbox `refs/dna/exchange/<origin identity>`, never its journal: handoff, origin_record, origin_url, origin_author, origin_row, lineage, purpose, position, via, kind, subject, class, fact, note |
| `handoff.published` / `handoff.refused` | ledger | `handoff:<id>` | in the origin record: connection, peer, kind, subject, class, purpose, peer_row, by, note; refused: why |
| `handoff.accepted` / `handoff.accepted_by_peer` | ledger | `handoff:<id>` | an acceptance in the receiving record's journal (by, note, connection, and the envelope's origin, lineage, purpose and fact), sent back as an envelope into the origin's mailbox; its admission in the origin (connection, peer, handoff, accepted_by, note) |
| `task.transfer_requested` / `task.transfer_accepted` | ledger | `t<n>` | a Task handed across a connection (`handoff`, `peer`), or offered to another owner of a shared record (`owner`, `to`, `assignee`); settled only on the receiver's acceptance |
| `decision.reported` | ledger | `t<n>` | a decision someone outside made, reported by the assignee: reporter, decider, channel, evidence, scope, obligation, practice, policy, accepted, why, note |
| `receipt.held` / `receipt.hold_released` | ledger | a digest | a hold that refuses redaction, and its release: by, why |
| `receipt.redacted` | ledger | a digest | the body removed, the digest kept: by, why, policy, class, store |
| `grant.revoked` | record | a child | the parent revoked the grant, recorded before it takes effect and restored at birth: by, parent, epoch |
| `concern.requested` / `concern.raised` | ledger | a source | a concern raised from a locus path about the part above it: what, severity, by; several concerns share one source, so a request carries its own `request` id and its answer is one object (`what`, `severity`, `occurrence`, `request`) — a concern's words are never read as the metadata around them — and one request is one concern, however often it is delivered |
| `concern.refused` | ledger | a source | one the organization would not admit, and why |
| `concern.proposed` | record | a source | three raises became a proposal: the practice's digest, or `refused`, after `<n>` raise(s) |
| `body.claimed` / `body.released` | ledger | the holder | who is running this record, by the lease's token: token, forced, from, by |
| `body.provisioned` | record | `<user>@<host>` | a machine made able to run it: dir, toolchain, knowledge (`compose` or `dsn`), by |
| `body.credential_missing` / `body.credential_present` | ledger | `model` | whether the model's key is set where the body runs: any_of, holder |
| `secret.rotated` | record | the variable's name | a credential set or rotated: where (`local` or the body), by — never the value |
| `schedule.declared` / `schedule.refused` | ledger | the schedule id | a schedule the org chart declares, or why it would not be admitted (a bad cron, a grant it exceeds) |
| `schedule.fired` / `schedule.skipped` | ledger | the schedule id | the Task it made (`task`), or why it did not fire — the last one is still open |
| `schedule.paused` / `schedule.resumed` | ledger | the schedule id | paused and resumed by hand, in your name |
| `budget.exhausted` | ledger | `budget` | the window's model allowance is spent: what was spent, of what, and when the window turns |
| `model.called` | ledger | `<work>/a<n>` or a review id | the model evidence; the prompt and context are receipts under its digests (`bodies`), none for a customer-class call |

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
| `refs/dna/candidates/<mutation>` | a Mutation's candidate, kept whatever its Review decided |
| `refs/dna/exchange/<identity>` | a connected record's mailbox for this one |
| `.hale/dna/` | sockets, `status.json`, `worktrees/<id>/`, `scratch/`, the artifacts as attached / running / before the last restart |
| `.hale/node/<name>/` | on a node: `<instance>.pid`, `<instance>.topology` |
| `<plan>.plan.json` | the fleet plan (schema 1.2: `seed`, `node` on an instance) |

## The core, by file

`vendor/dna/` after `init` (`dna/core/` in the hale repository):

| file | what |
|---|---|
| `assembly.hl` | `Dna` (the substrate), `Board`, `OrgPolicy` and the other review policies, `NoDeployment` / `ShellDeployment` / `LocalApplyDeployment` |
| `org.hl` | `Leader`, `SourceReader` |
| `journal.hl` | `Journal`, `MemJournal`, `Receipts` (`FileReceipts`), `Coordination` (`MemLeases`), the effect idempotency helpers |
| `record.hl` | `Record` — the record's own API — with `GitRecord` (the one file that spells `git` for the record) and `MemRecord`; `GitJournal`, `GitReceipts`, `GitLeases` over it |
| `routing.hl` | the three memories: `memory_of` (the routing table), `RoutedJournal` (the record and the ledger read as one), `ServiceLedger` (the ledger over the knowledge service), `ServiceLeases` (leases in the store, swapped by token; `GitLeases` moves to it with the routing) |
| `infrastructure.hl` | `Infrastructure` (a body's database, supervisor, credentials) and `Transport` (how a head reaches a body), with their memory implementations; the host's `infra.hl` is the reference one — compose, a systemd user unit, the env file, over ssh or this machine's shell |
| `exchange.hl` | `Exchange` (one record's mailbox in another: deliver once, delivered?, received) and `MemExchange`; the host's `connections.hl` exchanges through the peer's service or as mailbox refs, by the connection's url |
| `forge.hl` | `Forge` (a code-review host), `MemForge`, `NoForge`; the host's `forge_github.hl` is `GitHubForge` over `gh` and the `FileForge` fixtures use |
| `process.hl` | `Task`, `Workflow`, `Step`, `Work`, `Attempt`, `Metabolism` |
| `work_system.hl` | `WorkSystem`, routing perspectives, the performers |
| `review.hl` | `Review`, `AutonomyBoundary`, authority ranks |
| `models.hl` | `ModelRouter`, `OpenAiChat`, `AnthropicMessages`, `HarnessModel`, `LocalModel`, `FakeModel`, `HostedCredential` (with its `scheme`), `Confinement` (`Bubblewrap`, `NoConfinement`), `probe` |
| `budget.hl` | `BudgetPolicy`, `Budget` (the substrate's one counter) |
| `tape.hl` | `RecordedModel` (record and replay over any backend) |

Beside the core, `dna/knowledge` (the `KnowledgeStore` interface, `Pq`, `Mem`, `apply_record`), `dna/knowledge/service` (the service `hale dna knowledge` and `hale dna dev` run) and `dna/pond` (pond's `db` and `pq`, pinned):
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
