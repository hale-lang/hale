# Reference

The CLI, the environment, the record's vocabulary, the status
projection and the files, in one place. The chapters explain; this
page lists. [spec/dna.md](https://github.com/hale-lang/hale/blob/main/spec/dna.md)
is the contract behind all of it.

## The CLI

`hale dna --help` prints every verb. Here the same lines are grouped by
the part each verb drives, word for word. Most verbs take the project
directory as an optional argument and default to the current one.

### Starting: DNA and the genome

[Getting started](./getting-started.md) and
[DNA, the building block](./dna.md).

```text
hale dna init [app-dir] [--no-library] [--with <pack> [--set <param>=<value>]… [--as <who>]]
                             attach the DNA to an existing application; the record starts with the language and system
                             nodes and the toolchain's library proposed, one Review per family (--no-library leaves out the nodes and the
                             library, not the practices); --with proposes a pack from the toolchain's nursery as one batch beside them,
                             planned on the holes init opens so it may fill them (`--with founder --set founder=<you>` seats you in
                             every one): one `review all approve` ratifies the starting organization, seats included
hale dna new <name> [--no-library]
                             a greenfield application with its DNA
hale dna new <name> [--profile local|remote-body --remote <url> [--body <user@host>]]
                             the profile sets the pieces (a remote, a body host); the combination is always detected
hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain, and propose this version's practices and library
                             (each changed practice, and the library as new families, supersede the active one once the Board approves)
hale dna profile [project]   the organism's combination, detected from its pieces: record, body, head, fleet, knowledge, trust
hale dna --embedded-digest [--from-tree <dir>]
                             the digest of the DNA source this binary embeds (nothing else on stdout);
                             with a checkout, what that tree would embed — a mismatch means the binary
                             predates the working tree and a mutation run against it proves nothing
```

The habitat is its own command, the one that brings an organism's
memory, nerves and senses' store up and migrates them. `hale habitat
--help`:

```text
usage: hale habitat up [--foreground] [--profile local]
       hale habitat status
       hale habitat down
       hale habitat migrate [organism]
       hale habitat sweep <root>

The local habitat holds every organism's infrastructure on this machine:
one docker compose project, named for the habitat, with a service per
allocation (memory-<id>, nerves-<id>, senses-<id>, <id> the organism's
record identity), under ${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local.
It is long-lived: nothing tears it down when work ends, and two checkouts
of one organism are the same tenant.

up                   birth the habitat the first time, from the genome the
                     toolchain ships (its positions the nursery's `habitat`
                     pack, its law the `standard` pack and its document's
                     `law` items), the one organism born without a parent;
                     then bring every recorded allocation up, migrating any
                     whose recorded schema version is behind this
                     toolchain's, and start the habitat's body: a systemd
                     user unit named for the habitat, or, with
                     --foreground, in this process. Idempotent. The hosted
                     profile (backends the platform runs) is not built.
status               the habitat, its genome and body, and each
                     allocation: its organism, whether each role's service
                     runs, and the schema version it records against this
                     toolchain's.
down                 stop the habitat's body and every service of its
                     project; their data is kept, and `up` brings them back.
migrate [organism]   bring the organism's allocations up and migrate them:
                     memory's schema and the nerves' stream move to this
                     toolchain's, and the head's DSN is kept in the
                     checkout's .hale/dna/memory.env. With
                     HALE_DNA_MEMORY_DSN_OWNER or HALE_DNA_NATS_URL_OWNER
                     set, that server of your own is migrated instead.
                     Idempotent. The organism defaults to the current
                     directory. Each role's version is recorded in the
                     allocation's allocation.json.

sweep <root>         take down every compose project run from under <root>,
                     a test's habitat: its containers, networks and
                     volumes, confirmed gone by the project's label. The
                     projects are named in <root>/.dna-compose-projects
                     until they are, so a teardown that did not finish is
                     retried. Never the developer's habitat (under $HOME),
                     nor the one XDG_DATA_HOME names outside <root>. Exit 1
                     names what survived; nothing is done without docker.

Nothing else migrates or provisions: `hale dna dev`, `run`, `head`, `show`
and `work` run on what is up and migrated, and name this command when it
is not.
```

### The record

[Memory and the record](./memory.md).

```text
hale dna status [project] [--json]
                             the organism's status projection, from the Journal
hale dna history [<entity>]  walk the Journal by causal links (works offline)
hale dna record-rows [project]  the record's rows, one JSON object per line
hale dna sync [project]      apply what the record says of this clone (the record itself is memory's: nothing to sync)
hale dna candidates [<mutation> | drop <mutation> --why <w>]
                             the candidates the record keeps, whatever the review decided; one as a diff; stop keeping one
hale dna connect <nerves-url> --record <identity> --name <n> --as <position> --purpose <p> --classes <internal,customer,…>
                             propose a connection to another record (a Board Review); `hale dna connect` lists them;
                             `hale dna disconnect <n> --why <w>` closes one
hale dna handoff <n> task <id> | receipt <digest>
                             write one fact into the connected record, with origin, lineage and purpose;
                             `handoff` lists, `handoff accept <id>` accepts one received, `handoff sync` reads acceptances back
```

### Memory

[Memory and the record](./memory.md).

```text
hale dna memory migrate [dir]
                             apply memory's schema with the owner's DSN (HALE_DNA_MEMORY_DSN_OWNER, or the organism's
                             memory allocation in the local habitat, brought up) and print the record's spine and head DSNs
                             (HALE_DNA_MEMORY_DSN_SPINE, …_HEAD); `hale habitat migrate` does it with the nerves and senses
hale dna ledger rows         the operational memory's rows as JSON lines: where the day's work lives;
                             a head's write (a task done, a receipt filed …) goes straight into the Ledger
                             under the head's role: done when it lands, or refused by memory with the reason
hale dna receipt [disclose <digest> --to <who> --purpose <p> | show <digest> --purpose <p>]
hale dna receipt hold|release-hold <digest> --why <w> | redact <digest> --why <w> --policy <p>
hale dna receipt file <path> [--class internal|customer|confidential]   file a document as evidence
                             a hold refuses redaction until released; a redaction removes the body and keeps the digest, as a row
                             (a body kept in memory is erased by the body on its tick, and filing it again is refused)
                             protected evidence (customer, confidential): kept in memory alone, sealed there;
                             disclosure and every read are rows in the reader's name (--as <who>)
hale dna show org|processes [--json] [project]
                             the org chart and the process model, as queries over memory
```

### The nerves

[The nerves](./nerves.md).

```text
hale dna nerves migrate [dir]
                             create the organization's NATS JetStream stream with the owner's URL (HALE_DNA_NATS_URL_OWNER,
                             or the organism's nerves allocation, brought up) and print its token and each role's URL
hale dna nerves urls [dir]   the token and each role's URL of nerves that are up; nothing brought up or created
hale dna nerves drop [dir]   delete the organization's stream, and everything it held, with the owner's URL
```

### The spine

[The spine](./spine.md).

```text
hale dna run [project] [--port N] [--no-iris]
                             build and run the organization (dna/org), a node of it: relay the record's requests
                             to it over the nerves (NATS); iris inspects its process
hale dna definitions [project] [--json]
                             the workflow catalog (dna/org/workflows.hl): each definition's revisions and every step's store
hale dna effect resolve <key> an effect whose outcome is unknown after a restart: --outcome ok|failed, in your name
```

### The heart and the body

[The heart and the body](./heart.md).

```text
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
                             the organization AND the application under one host: rebuild and restart
                             the application on an apply, watch the window, report back
hale dna application remove [project] [--as <who>]
                             the attached application removed (`application.detached` in the record) and its
                             broker account revoked: its user, its password and its vault entry (GH #989)
hale dna body                who runs this record (the body lease); `body claim --force` takes it from a body that is gone;
                             `body release [--force]` gives it up — both are rows in your name (--as <who>)
hale dna body provision <user@host> [--dsn <postgres://…>] [--dir <path>] [--dry-run]
                             over ssh: the toolchain hale.lock pins, the record's remote cloned, Postgres from
                             the body's local habitat or the DSN, migrated once (`hale habitat migrate .`), a systemd
                             user unit supervising the host; writes nothing
                             when ssh or the toolchain is unavailable. Then `body start|stop|logs [--body <user@host>]`
hale dna fleet [project]     what the fleet expresses: every instance, its node, revision, model hash, state
hale dna deploy <revision>   express a genome revision through the fleet's nodes (fleet.deploy)
hale dna rollback <mutation> express the base a Mutation was applied on, again
                             (`[dna] fleet = "<name>"` in hale.toml names the plan; `hale node <name>` runs a node)
```

A node is its own command. `hale node --help`:

```text
usage: hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]

The agent that expresses a fleet plan's instances on one machine,
from the record: it reconciles what the plan says this node runs
against what is running here (GH #566 F5). With HALE_DNA_NATS_URL_APP
and HALE_DNA_NATS_ORG set (`hale dna nerves migrate` prints them), it
hands both to every instance, which says what it says onto the
nerves itself (GH #986).
```

### Senses and reflexes

[Senses and reflexes](./senses.md).

```text
hale dna senses up [dir]     bring up the organism's senses' store in the local habitat and print its read URL
hale dna senses url [dir]    the read URL of a senses' store that is up; nothing brought up
hale dna pressure [raise <source> <what…>]
                             pressure raised and answered; `raise` writes one signal into the record, which a node relays
hale dna concern raise <source> <what…> [--severity N]
                             a concern from a locus path about the part above it; persistent ones become knowledge proposals
```

### The skin

[The skin](./skin.md).

```text
hale dna secret set <NAME> [--body <user@host>]
                             a credential from stdin (never argv, never the record) into its slot of the vault
                             (a model key's, FORGE_TOKEN, or OIDC_CLIENT_SECRET) on the body or here; `secret rotate <NAME>`; the record gets `secret.rotated <NAME>` only
hale dna secrets [dir]       every secret the organism requires, and whether the vault holds it (never a value)
```

### Legs and voice

[Legs, hands and voice](./legs.md).

```text
hale dna work <verb> …       a leg's verbs against the head's API (--api, --as position:<name>): next, brief, login,
                             renew, allowance, submit (--from-worktree), settle, release, friction, hand, run — the project's performers (dna/org/work.hl);
                             loop --parallel N is a worker: N children, each its own holder; loop --drain ends one
hale dna models [project]    the catalog (dna/org/models.hl): every backend, and one small request to each
hale dna models map | rule <selector> <size> [--permit <sizes>] | category <mode>.<size> <model> [--tools] [--price-in N] [--price-out N]
                             which model a Work gets, as rows in your name (--as <who>): a rule says the size a kind of work needs
                             (default, position:<name>, contract:<Name>, task:<id>; quick standard deep), a category which model fills it
```

### Schedules

[Schedules](./schedules.md).

```text
hale dna schedule [pause <id> | resume <id>]
                             the schedules the record declares (a definition on an interval or a cron, and who
                             convenes it) and their occurrences; pause and resume are rows in your name (--as <who>)
hale dna schedule declare <id> (--every <n>ms|s|m|h|d | --cron <expr>) --definition <id> --convener <position> [--args <json>]
                             a schedule asked of the organization, which declares it or refuses it; an occurrence
                             is an execution of the definition (GH #1143)
```

### The head and the face

[The head and the face](./head.md).

```text
hale dna head [project] [--port P] [--api-port P] [--oidc-port P] [--commands-port P] [--source-drafts]
                             the head: the project service serving the browser face (8792), its API child, the stub
                             OpenID provider and, with a project, the reflexes — built once into the toolchain cache,
                             no checkout needed (from a checkout, dna/face/start.sh); with a project, the head's
                             memory DSN derived from memory that is up, never migrated (`hale dna head --help`)
hale dna ui [project] [--port N]
                             under `git config dna.principal oidc` a hosted head: sign-in through dna.oidc.issuer,
                             subjects mapped by dna.oidc.member, the secret the vault's oidc-client-<client>;
                             with no principal source it refuses to start (trusted-local is a test fixture's mode)
                             the DNA surface in a browser, from the record in memory: the Board's queue, the Reviews
                             with their three views, the fleet, the history; verdicts, intent and pressure from forms
hale dna board [project]     the Board's queue: what needs its verdict, escalations, proposals, reports
hale dna review              the pending Reviews
hale dna review <id> [--iris] render a Review: source diff, semantic diff, evidence (works offline)
hale dna review <family> approve|reject
                             decide every pending Review of a group in turn: purpose, design, operating, using, library, holes, practices, holds, positions, changes, ingest (a library family is one Review for all its ideas)
hale dna review all approve|reject
                             every group with a pending Review, in the order a new organization is ratified, each waited for before the next
hale dna review <id> approve|revise|reject|abstain [--as <reviewer>] [--authority <a>] [--comment <c>] [--digest <sha>] [--no-wait]
                             write a verdict into the record, which a node relays; the Review decides
hale dna route [--json] (<path>… | --diff <range>)
                             who must sign a change set, and the gates it is judged against
hale dna github sync         mirror pending Reviews to pull requests and read their reviews back as verdicts
                             (git config dna.github owner/repo; dna.github.board logins,…; needs `gh`)
hale dna report [project]    file a report from the record since the last one (report.filed)
```

### Tasks and people

[One task, end to end](./workflow.md).

```text
hale dna task create [--to <locus>|position:<name>] [--as <who>] [--judgment] [--size quick|standard|deep] [--no-wait] <outcome…>
                             ask for an outcome (--judgment: an assessment, a leg's to perform; --to position:<name>: a change the leg holding that position makes): a row in the record, which a node relays to the organism; prints the Task born or the refusal
hale dna task done <id>      a person reports a handed Task done (--as <who>, --note …); `task reassign <id> --to <who>`
                             under an acceptance practice requiring evidence: --evidence <digest>, or --exception <why> --authorized-by <who>
hale dna task authorize <id> --exception <why>   authorize an exception, in your name (not the assignee's)
hale dna task decide <id>    report a decision someone else made (--decided-by <party> --via <channel> --evidence <digest>, --as <reporter>)
hale dna retire <who>        a person retires: the handed Tasks they hold move to --to <successor>, as rows
```

### Shaping

[Shaping and governing it](./shaping.md).

```text
hale dna fill <position> <holder> [project] [--as <who>]
                             ask the organization to propose who holds a position, for the Board
hale dna ingest [project] [--at <rev>]
                             read the repository's graph again at a commit (HEAD): what differs from the record, tools
                             for every served operation included, proposed as one Board Review (`review ingest approve`)
hale dna position open <name> --mandate "<text>" [--text <what it is>] [--under <part>] [--as <who>]
                             open a position at run time: its node and its mandate, one Board Review (`review positions approve`)
hale dna apply <file> [--dry-run] [--as <who>] [--on-holes]
                             several changes to the structure (`open`, `fill`, `fill *`, `vacate`, `practice`, `assign`, `unassign`, `close`, `merge`,
                             `split`, `expose`, `depend`, `externalize`, `internalize`, `retire-practice`) as one Board Review, applied all or nothing
                             (`review changes approve`); --dry-run prints the org chart before and after and proposes nothing; --on-holes
                             plans it on the holes init proposed that the Board has not decided, and ratifies it after them
hale dna vacate <position> <holder> [--dry-run] [--as <who>]
                             a holder leaves a position: a one-step batch, one Board Review (`review changes approve`)
hale dna pack list [--from <dir|git-url[#rev]>]
hale dna pack show <name> [--set <param>=<value>]… [--from …]
hale dna pack apply <name> [--set <param>=<value>]… [--from …] [--dry-run] [--as <who>] [--on-holes] [project]
hale dna pack retire <name> [--dry-run] [--as <who>] [project]
hale dna nursery add <dir|git-url> [--dry-run] [--as <who>] [project]
hale dna nursery list [project]
                             a nursery the Board trusts, proposed as one batch step; the ones it trusts (a git nursery's
                             packs are adopted only once it is trusted)
                             packs from a nursery (the toolchain's, a directory, or a git repository at a revision): a batch
                             with blanks, adopted as one Board Review named for the pack (`review changes approve`)
hale dna practice propose <name> --text <text> [--because <why>] [--supersedes <digest>]
                             propose a practice for the Board to ratify (a knowledge Review); `hale dna practice` lists them
```

## The environment

The variables you set yourself. Everything else the CLI and the body
set for the processes they start.

| variable | what it does |
|---|---|
| `HALE_DNA_MEMORY_DSN_OWNER` | a Postgres of your own instead of the organism's memory allocation in the local habitat: the owner role that migrates memory (`hale habitat migrate`, `memory migrate`, `upgrade`); `dev`, `head` and `show` derive from it and migrate nothing |
| `HALE_DNA_NATS_URL_OWNER` | a NATS server of your own instead of the organism's nerves allocation: the owner user that creates the stream (`hale habitat migrate`, `nerves migrate`); `dev` and `nerves urls` derive from it |
| `HALE_DNA_MEMORY_DSN_SPINE` | the spine role's DSN, which `hale dna run` needs; `memory migrate` prints it |
| `HALE_DNA_NATS_URL_SPINE`, `HALE_DNA_NATS_ORG` | the spine's nerves URL and the organization's token, which `hale dna run` needs; `nerves migrate` prints them |
| `HALE_DNA_MEMORY_DSN_HEAD` | the head role's DSN, for a head and nothing else; `memory migrate` prints it |
| `HALE_DNA_RECEIPT_KEY` | sixteen characters or more: the key protected evidence is sealed under, given to `memory migrate` once |
| `HALE_DNA_OWNER_KEYS` | `<owner>=<key> …`: the owners of a shared record, for `memory migrate` |
| `HALE_DNA_API`, `HALE_DNA_ID_TOKEN` | the head a leg's `hale dna work` verbs talk to without `--api`, and the bearer token it presents |
| `HALE_DNA_GENOME_POLL` | seconds between polls of the record's remote for a changed genome (300) |
| `HALE_DNA_NO_BUILD_CACHE` | build every seed, instead of reusing a build of the same sources |
| `HALE_DNA_COMPOSE=off` | a repository's body leaves its compose services to you instead of bringing them up and down |
| `HALE_VAULT_DIR`, `HALE_VAULT_ADDR`, `HALE_VAULT_TOKEN` | where the vault is: a local directory, or a vault's HTTP API and its token |

Every `HALE_DNA_*` variable, with its default and effect, is in the
spec's [Environment](https://github.com/hale-lang/hale/blob/main/spec/dna.md#environment)
table; a test fails when the tree names one the table does not. The
vault's three are in the runtime's
[environment table](https://github.com/hale-lang/hale/blob/main/spec/runtime.md#diagnostic--tuning-env-vars).

The record's settings live in the clone's git config:

| key | what it does |
|---|---|
| `dna.remote` | the remote the record syncs with (`origin` when unset) |
| `dna.trust` | `local` (every writer to the record is trusted) or `signed` (a relayed row needs a verified signature) |
| `dna.owner` | on a shared record, the organization this body is |
| `dna.body` | `<user@host>`, set by `body provision` (or `new --body`): where `body start`, `stop`, `logs` and `secret set` go without `--body` |
| `dna.unix.member` | `uid:<n>=<name>`: who a local uid is, for the head's socket |
| `dna.principal` | `oidc` for a hosted head |
| `dna.oidc.issuer`, `dna.oidc.client`, `dna.oidc.redirect` | the issuer, the head's client id, its redirect URI |
| `dna.oidc.member` | `<subject>=<name>`, one per person: who a subject is |
| `dna.oidc.board` | the member names that act with Board authority |
| `dna.oidc.key` | the pinned signing key of an issuer on the loopback |
| `dna.github`, `dna.github.board` | the `owner/repo` Reviews are mirrored to, and the logins whose reviews count as the Board's |

## The record's vocabulary

Every row has a kind, an entity, a body and an author. The **memory**
column is where the row lives: `record` is `record_rows` and `ledger`
is `ledger_rows`, both in the record's schema in memory (Postgres).
The two are read as one sequence ([Memory and the record](./memory.md)). The table
in `dna/core/routing.hl` (`memory_of`) decides. A reader that meets a
kind it does not know keeps walking.

### The record and memory

| kind | memory | what it is |
|---|---|---|
| `structure.observed` | record | the compiler's model of one of the application's parts, at `init` |
| `graph.node` / `graph.edge` / `graph.retired` | record | a node of the repository's graph; a hyperedge of it; one leaving it (also inside a ratified graph proposal's receipt) |
| `graph.ingested` | record | `hale dna ingest` read the repository at a commit (`added`, `changed`, `retired`, the `review_id` of its proposal) |
| `graph.requested` / `graph.proposed` / `graph.refused` | record | `hale dna ingest` asks the running organization to open its Review; opened, or not. `graph.refused <row id>` with `by: memory` is a graph row memory would not project; the projection goes on past it |
| `model.rule` | record | the size a selector's Work needs, and the sizes a more specific selector may move it to (`hale dna models rule`, `task create --size`) |
| `model.category` | record | which model fills `<mode>.<size>`, whether it returns tool calls, and its price (`hale dna models category`) |
| `responsibility.proposed` | record | a one-line responsibility inferred for a part, not yet ratified |
| `candidate.dropped` | record | a kept candidate is no longer kept, here and at every clone's sync |
| `receipt.filed` | ledger | an internal document filed as evidence |
| `receipt.classified` / `receipt.withheld` | ledger | a protected body memory keeps sealed, or one withheld because no memory could keep it |
| `receipt.disclosed` | ledger | a reader authorized, for a purpose |
| `receipt.read` / `receipt.read_refused` | ledger | a read in the reader's name, or its refusal |
| `receipt.held` / `receipt.hold_released` | ledger | a hold that refuses redaction, and its release |
| `receipt.redacted` | ledger | the body removed, the digest kept |
| `connection.proposed` / `connection.closed` | record | a connection to another record, for the Board; and its closing |
| `handoff.published` / `handoff.refused` | ledger | a fact written into a connected record, or why not |
| `handoff.received` | ledger | an envelope in the receiving record's inbox (a kind of the exchange's envelopes since GH #1590, never a row) |
| `handoff.accepted` / `handoff.accepted_by_peer` | ledger | an acceptance in the receiving record, and its admission back in the origin |
| `task.transfer_requested` / `task.transfer_accepted` | ledger | a Task offered across a connection or to another owner, and accepted |

### The spine

| kind | memory | what it is |
|---|---|---|
| `intent.requested` | ledger | an ask from a clone with no organization running |
| `intent.offered` / `intent.refused` | ledger | the outcome an ask was admitted for, or the refusal |
| `intent.unrecovered` | ledger | an intent offered before a restart that no admission names; never re-offered |
| `task.born` | ledger | the work an intent or a settled Review made |
| `workflow.admitted` / `workflow.refused` | ledger | an execution admitted (definition, revision, inputs, the bound recipe), or refused |
| `workflow.ask_refused` | ledger | an ask refused before any Task was minted |
| `workflow.settled` | ledger | an execution settled `done`, `failed` or `cancelled` |
| `step.registered` / `step.activated` / `step.completed` / `step.failed` | ledger | a step's required set, then its life |
| `attempt.admitted` / `attempt.outcome` | ledger | one attempt and the request it was admitted with; its disposition, result and evidence |
| `work.settled` | ledger | a unit of work settled, naming the attempt it settled on |
| `effect.requested` / `effect.result` | ledger | the exclusive claim on an effect key, and its outcome |
| `effect.relayed` | ledger | an attempt a leg relay answered pending: a leg's to claim at the head |
| `effect.redelivered` | ledger | a claimed attempt dispatched again after a restart |
| `claim.taken` / `claim.released` | ledger | a node's claim by id before it acts, and its release |
| `lease.taken` / `lease.renewed` / `lease.released` | ledger | the runtime's lease on its record, with its fencing token |
| `budget.exhausted` | ledger | the window's model allowance is spent |
| `model.called` | ledger | a model call and its evidence |
| `grant.contracted` / `grant.revoked` / `grant.refused` | record | authority narrowed, taken back, or born wider than its ceiling |
| `grant.reservation_refused` / `grant.fenced` | ledger | a spend the window would not admit; an admission refused because the grant's epoch moved |
| `spend.reserved` / `spend.settled` / `spend.compensated` | ledger | a spend admitted; one attempt's actual consumption; money that came back, authorized by name |
| `optimize.refused` | ledger | the organization's pass over itself did not run |
| `org.reviewed` | record | that pass's own answer |

### Legs

| kind | memory | what it is |
|---|---|---|
| `attempt.claimed` | ledger | a leg's lease on an admitted attempt, taken at the head |
| `attempt.outcome_requested` / `attempt.outcome_refused` | ledger | the outcome a leg handed back under its lease, or why the owner would not settle it |
| `attempt.allowance_requested` | ledger | a leg's ask for its attempt's spend |
| `attempt.allowance_granted` / `attempt.allowance_refused` | ledger | the budget's one gate answering it (also the editor's, and `review:<id>` for the Leader's) |
| `attempt.released` | ledger | a leg gave its lease back without an outcome |
| `attempt.unresolved` | ledger | an `uncertain` performer's attempt with no known outcome, until a person resolves it |
| `friction.filed` | record | what got in a position's way |

### Changes and Reviews

| kind | memory | what it is |
|---|---|---|
| `mutation.requested` | record | which Work and attempt asked for the Mutation |
| `mutation.proposed` | record | a change proposed for a Task: class, objective, target, base |
| `mutation.worktree` / `mutation.located` / `mutation.candidate` | record | the sandbox; the files found and the grant; the candidate commit |
| `mutation.review` / `.stage` / `.escalate` / `.release` / `.deny` | record | what the autonomy boundary decided |
| `mutation.topology` | record | the diff names a plan or the manifest: re-classed for the Board |
| `mutation.applied` / `mutation.apply_retried` | record | the candidate applied to the genome; the apply run again |
| `mutation.retained` / `mutation.rolled_back` | record | kept after its observation window, or the genome back at the base |
| `mutation.rejected` / `mutation.revise` | record | refused after review, or sent back |
| `mutation.refused` / `mutation.failed` | record | not applied, or did not survive its own verification |
| `evidence.<step>` | record | a verification step's output by digest: `base`, `fmt`, `check`, `verify`, `test`, `fleet`, `replay`, `rollback`, `diff` |
| `evidence.magnitude` | record | the measured magnitude of the change |
| `review.requested` | record | the Review: question, authority, candidate, disposition, evidence, diffs |
| `review.routed` / `review.signed` | record | what a routed Review requires; an approval it admitted that settled nothing |
| `review.verdict` | record | a verdict in the reviewer's name, from a clone or a forge |
| `review.command_decided` | record | a Review decided by a command |
| `review.settled` / `review.refused` | record | the verdict that decided it, or why one was not admitted |
| `review.reasoned` | record | the deciding verdict's comment |
| `gate.observed` | record | a gate's check run at the candidate, as the forge reported it |
| `github.pr` / `github.commented` | record | the pull request a Review opened, and the settlement commented back |

### The heart and the body

| kind | memory | what it is |
|---|---|---|
| `application.attached` / `application.detached` | record | the application the organism oversees, and its removal |
| `expression.restart_requested` | record | the organization asks for a change to be expressed |
| `expression.restarted` / `expression.deployed` | record | what the new expression reports, or what a deployment gateway expressed |
| `expression.observed` / `expression.crashed` | record | the observation window's outcome |
| `observation.requested` / `observation.refused` | record | the host's observation report as a row |
| `fleet.deploy` | record | a genome revision expressed through the fleet's nodes |
| `instance.up` / `instance.exited` | ledger | a node's report on one instance of the plan |
| `node.started` / `node.build_failed` | record | a node runs a genome; or a genome did not build, and it stayed on the last |
| `body.claimed` / `body.released` | ledger | who is running this record, by the lease's token |
| `body.provisioned` | record | a machine made able to run it |
| `body.credential_missing` / `body.credential_present` | ledger | whether the model's key is in the vault where the body runs |

### Senses and reflexes

| kind | memory | what it is |
|---|---|---|
| `reading.recorded` | ledger | an application's event, landed as a reading before anything acts on it |
| `reflex.acted` | ledger | what a node did about a reflex's firing |
| `pressure.requested` / `pressure.raised` | ledger | a signal from a source, and its answer |
| `pressure.remeasured` | ledger | the declared fitness signals, measured again after a change |
| `concern.requested` / `concern.raised` / `concern.refused` | ledger | a concern from a part about the part above it, and its answer |
| `concern.proposed` | record | three raises became a proposal |
| `appendage.proposed` / `appendage.candidate` | record | an organ the organization proposes for itself, and the Mutation that proposes it |

### The skin

| kind | memory | what it is |
|---|---|---|
| `secret.rotated` | record | a credential set or rotated: its name and where, never the value |
| `violation.recorded` | record | a closure a part absorbed and went on from: `adapter_undeliverable`, `lease_unsettled`, `pulse_stopped` |

### Schedules

| kind | memory | what it is |
|---|---|---|
| `schedule.requested` / `schedule.answered` | ledger | a schedule asked of the organization, and its answer |
| `schedule.declared` / `schedule.refused` | ledger | a schedule declared, or refused with the reason (also an occurrence refused) |
| `schedule.skipped` / `schedule.missed` | ledger | an occurrence skipped while the last is open; occurrences that passed with no tick |
| `schedule.paused` / `schedule.resumed` | ledger | paused and resumed by hand |

### People, knowledge and practices

| kind | memory | what it is |
|---|---|---|
| `task.handed` / `task.reassigned` | ledger | a Task handed to a person; the assignment moved |
| `case.admitted` | ledger | a person's leaf as its own handed Task |
| `task.done` | ledger | a person reports a handed Task done |
| `completion.linked` / `completion.excepted` | ledger | the evidence a completion links, or the exception it carries |
| `exception.authorized` | ledger | an exception to a Task's acceptance, in the authorizer's name |
| `decision.reported` | ledger | a decision someone else made, reported by the assignee |
| `person.retired` | record | someone left, and who took their work |
| `hold.requested` / `hold.proposed` / `hold.refused` | record | a holder asked for a position (`hale dna fill`), proposed to the Board, or refused |
| `batch.proposed` | record | a batch `hale dna apply` proposed as one Review in `changes`, with its steps (and `planned_on`, the pending Reviews it was planned on); `graph.requested` under the same id asks the organization to open it |
| `position.requested` / `position.proposed` / `position.refused` | record | a position asked at run time (`hale dna position open`), proposed with its mandate as one Board Review, or refused |
| `practice.requested` / `practice.proposed` / `practice.refused` | record | a person's practice proposal: asked, proposed for review, or refused |
| `knowledge.proposed` / `.ratified` / `.declined` / `.refused` | record | a practice through its Review |
| `knowledge.retired` | record | a practice superseded by a later version |
| `knowledge.consulted` | ledger | what a piece of work looked up |
| `practice.read` | record | what the hat read once a proposal was ratified |
| `report.filed` | ledger | a report from the record since the last one |

## `hale dna status --json`

The projection `status` prints, as one JSON document on one line;
`hale dna ui` serves the same at `/api/status`. Here it is for a fresh project with nothing
running, indented, with twenty-two of its twenty-three pending Reviews cut:

```text
$ hale dna status --json
{
  "organism": "not running — reading the Journal",
  "journal": {
    "identity": "4f1c9a07d2be…",
    "revision": 38,
    "chain": "verified",
    "chain_head": "77bc90bc6b4a31e4fb45b98d4da500309d6e6100"
  },
  "expression": {
    "attached": {
      "artifact": ".hale/dna/baseline.topology",
      "artifact_digest": "aaa7945205031d36",
      "main": "Refproj",
      "name": "refproj",
      "provenance": "observed",
      "schema": "1.20",
      "shape_hash": "6aacbffe834a9fcd",
      "toolchain": "0.21.0",
      "verdict": "clean"
    },
    "current": null,
    "build_digest": null,
    "toolchain": "0.21.0",
    "restarts": 0,
    "last_restart_request": null,
    "last_observed": null
  },
  "intents": {
    "offered": 0,
    "refused": []
  },
  "tasks": [],
  "reviews": [
    {
      "id": "k:05cecb18af4e",
      "state": "pending",
      "question": "ratify the design practice `design/software-delivery`: For an appendage or a product: process boundaries first (what runs, fail…",
      "required_authority": "board",
      "subject_digest": "sha256:05cecb18af4e5fc83bd62025ee3477b831ff2a9299c497116dc4ffaf90ccd7a1",
      "refusals": []
    },
    …
  ],
  "mutations": [],
  "law_deferred": [],
  "model_calls": {
    "total": 0,
    "recent": []
  },
  "pressure": "not journaled in Phase 1",
  "projected_at": 1790702205
}
```

`organism` is `running (this clone's body holds the lease)` when a body
here holds the lease. `chain` is `verified` or `BROKEN`. As the
organism works, the arrays fill:

- a task: `id`, `outcome`, `state`, `since`, and `detail` when there is
  one;
- a settled Review adds `settled` (and `reasoned`); a Mutation's Review
  adds `mutation_id`, `change_class`, `seed`, `disposition`,
  `base_commit`, `candidate_commit`, `candidate_shape`, `evidence`,
  `magnitude`, `diff_text`, `diff_json` and `author`;
- a Mutation: `id`, `candidate`, `disposition`, `class`, `task`,
  `objective`, `detail` and `worktree` when set, and its `events`;
- `last_restart_request` and `last_observed`: `mutation`, `what`,
  `seq`.

## Where things live

### A generated project

`hale dna new <name>` writes these; `hale dna init` writes the same
around an application you already have.

| path | what |
|---|---|
| `main.hl`, `tests/main_test.hl` | the application and its first test (`new` only) |
| `hale.toml` | the manifest: an environment for the application (`.`) and one for the organization (`dna/org`) |
| `hale.lock` | the toolchain the DNA was materialized for (`[dna] toolchain`) |
| `.gitignore` | ignores the build artifact, `/vendor/` and `/.hale/` |
| `vendor/dna/` | the DNA core, copied from the toolchain; git-ignored, `hale dna upgrade` copies it again |
| `dna/org/main.hl` | the organization: its positions, the Leader's grant, the budget's owner |
| `dna/org/law.hl` | its law |
| `dna/org/purpose.hl` | the declared purpose |
| `dna/org/charter.hl` | the Leader's brief |
| `dna/org/models.hl` | the model catalog and the budget |
| `dna/org/work.hl` | the performers a leg runs |
| `dna/org/workflows.hl` | the workflow catalog; rewritten by `upgrade`, not yours to edit |
| `dna/org/own_workflows.hl` | your own workflow definitions, beside the baseline |

No file in the project runs or configures a service: memory, the nerves
and the senses' store are the organism's allocations in the local
habitat, below.

### The organism's state

| where | what |
|---|---|
| memory, `record_rows` | the record: one signed row per event, insert-only |
| memory, receipts | receipts, by the digest of their content (protected classes sealed) |
| memory, leases | leases with fencing tokens, swapped by token; the body lease is one |
| `hale.toml` `[dna] record` | the record's identity: the digest of its first row |
| `.hale/dna/memory.env` | the head's DSN and where it came from (`HALE_DNA_MEMORY_FROM`), kept by `hale habitat migrate` and `hale dna memory migrate`; mode 600, head role only, git-ignored |
| `refs/heads/dna/candidate/<mutation>` | a Mutation's candidate, a branch of the genome kept whatever its Review decided |
| `refs/hale/remote/genome` | the forge's default branch as the node's last pull fetched it |
| `.git/config` | the record's settings (`dna.trust`, `dna.unix.member`, … above) |
| `.hale/dna/` | this clone's scratch: `status.json`, the organization's log `org.log`, the topology artifacts, `embedded.digest`, the seed build cache `build/`, sandboxes `worktrees/`, `scratch/`, the built leg `legs/`, a person's kept worktrees `work/<attempt>`, an ingest's checkout `ingest/<sha>/` (removed after) |
| `.hale/node/<name>/` | on a node: its instances' pid files and artifacts |
| memory | Postgres, schema `dna_<record identity>`: the ledger, the graph and protected evidence, under the spine's and the head's roles |
| the nerves | one JetStream stream per organization, named after its token in capitals |
| the local habitat | `${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local`: one compose project per machine, `hale-habitat-local-<h>`, with the services `memory-<id>`, `nerves-<id>` and `senses-<id>` per organism (`<id>` the first twelve hex digits of the record's identity); each organism's `allocations/<id>/` holds `compose.yaml`, `ports.yaml`, `postgres.secrets`, `nats.conf`, `nats.secrets.conf` and `senses.yml`. Long-lived; `hale habitat migrate` brings an organism's allocations up |
| the vault | `HALE_VAULT_DIR`, else `$XDG_CACHE_HOME/hale/vault`, else `~/.cache/hale/vault`; one file per secret, mode 600. `HALE_VAULT_ADDR` names a vault's HTTP API instead |
| `HALE_DNA_HEAD_STATE` | the project head's state (`${XDG_STATE_HOME:-$HOME/.local/state}/hale/dna/head` when unset) |

The record and the day's work are both in memory; `hale dna ledger
status` says where things stand.
`hale dna secrets` lists every vault entry the organism needs.
