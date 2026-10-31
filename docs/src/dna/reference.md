# DNA & Habitat Reference

This reference catalogs every CLI command (`hale dna`, `hale habitat`, `hale node`), environment variable, Git configuration key, record/ledger row kind, `hale dna status --json` field, and file/state location. The normative specification is [`spec/dna.md`](https://github.com/hale-lang/hale/blob/main/spec/dna.md).

```sh
hale dna --help
hale habitat --help
hale node --help
```

---

## CLI Reference

Most `hale dna` commands accept an optional `[project]` directory path and default to the current working directory.

### Habitat & Provisioning (`hale habitat`)

The local habitat provisions and manages every organism's infrastructure (Postgres memory, NATS JetStream nerves, and Prometheus senses store) under `${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local`.

```text
usage: hale habitat [--head <url>] up [--no-body] [--image <tag>] [--profile local] [--backends own|operator] [--name <name>]
       hale habitat up --host <name> [--user <user>] [--port <n>] [--name <name>] [--image <tag>] [--backends own|operator]
       hale habitat image build [<checkout>]
       hale habitat [--head <url>] graft <dir> [--attach] [--no-library] [--no-pack]
                          [--profile local|remote-body [--remote <url>] [--body <user@host>]]
                          [--with <pack> [--set <param>=<value>]… [--as <who>]]
       hale habitat [--head <url>] graft <sapling> [--host <name>] [--no-library] [--no-pack] [--set <param>=<value>]… [--no-head]
       hale habitat [--head <url>] place <key>/<role> <host>
       hale habitat [--head <url>] model-map <organism> <mode>.<size> <model> [--endpoint <url>] [--key-vault <slot>] [--tools]
       hale habitat [--head <url>] model-probe <organism> <mode>.<size>
       hale habitat [--head <url>] models <organism>|--all [--plan]
       hale habitat [--head <url>] apply [<host>] [--fresh]
       hale habitat [--head <url>] nursery add <dir|git-url> | refresh | list | ratify <sapling>
       hale habitat [--head <url>] forge <organism> forgejo|github|file|none [--repo <owner/name>] [--key-vault <slot>] [--no-head]
       hale habitat [--head <url>] handover [organism]
       hale habitat [--head <url>] host add <name> [--user <user>] [--port <n>]
       hale habitat [--head <url>] host probe <name>
       hale habitat [--head <url>] host remove <name> [--plan]
       hale habitat [--head <url>] organism remove <name> [--plan]
       hale habitat [--head <url>] brief
       hale habitat [--head <url>] status
       hale habitat down [--forget [--plan]]
       hale habitat migrate [organism]
       hale habitat sweep <root> | --test-vaults [<root>]
```

| Subcommand | Description |
| --- | --- |
| `up` | Initializes the habitat on first run from the embedded genome (the only organism born without a parent; proposes the `habitat` and `standard` nursery packs), brings up all recorded allocations via the `compose` provider, migrates schemas behind the current toolchain, records placement (`local` host, policy, head, and face) and the head's URL (`head.url` and `habitat.profile`), and starts the habitat's `body` and `face` services in its Docker Compose project (built from the embedded `Dockerfile` and `hale.pin`, or from a prebuilt image with `--image <tag>` / `HALE_HABITAT_IMAGE`; `--no-body` provisions without starting `body` or `face`). `--backends own` (default) provisions Docker Compose services; `--backends operator` adopts `HALE_DNA_MEMORY_DSN_OWNER` and `HALE_DNA_NATS_URL_OWNER`. Idempotent. |
| `up --host <name>` | Runs the habitat on another machine (Docker and nothing else) over your own SSH login (`--user`, `--port`, or the SSH config's; `HALE_HABITAT_SSH_CONFIG` for `ssh -F`): sends or pulls the toolchain image there, runs `up` there in it, waits on the head and face at `http://<name>:<port>`, and keeps a pointer here (`remote.json`, `head.url`, the head's launch token). Every verb here reaches the head through it; `status` asks the head, `down` and `graft <dir>` run there over SSH (a directory here shipped there as the organism's home), `organism <name> dna <verb>` runs an organism's verbs there, `migrate` and `handover` are refused. Run again, it finds the same habitat by its record's identity and changes nothing. |
| `image build [<checkout>]` | Builds the habitat toolchain image from `<checkout>` (`.` by default) using its `Dockerfile` (`hale-habitat:<pin>`) and prints the tag on stdout. |
| `graft <dir>` | Births a new organism as a child of the habitat (`record.genesis` names the habitat record as parent), allocates required roles (`[dna] needs` in `hale.toml`: `memory` and `nerves` always, `senses` unless omitted, plus optional `database` for a Hale heart passed as `HALE_DNA_DATABASE_DSN`), and proposes the `standard` pack (`--no-pack` skips it). Pass `--attach` for an existing application or repository, `--profile remote-body --remote <url> [--body <user@host>]` for a remote body, and `--with <pack> [--set k=v]… [--as <who>]` to plan a nursery pack on top of the initial `holes` Reviews. |
| `graft <sapling>` | Fetches a pinned sapling by bare name into `<habitat>/saplings/<name>`, verifies its file digest, grafts it as a child of the habitat, proposes its packs (`--set` values forwarded), and records the sapling name, revision, and digest. |
| `nursery add \| refresh \| list \| ratify` | `add <dir\|git-url>` trusts a nursery (`Commands::add_nursery`). `refresh` re-reads all nurseries (pinning newly seen saplings and filing newer revisions of existing saplings for the Board without moving grafted organisms). `ratify <sapling>` pins a filed revision. `list` displays all trusted nurseries and saplings. |
| `handover [organism]` | Prints `NAME=value` environment lines handed to the organism's application heart (referencing secrets by vault slot name only): `HALE_DNA_APPLICATION_ID`, `HALE_DNA_HEAD_URL`, `HALE_DNA_OIDC_ISSUER`, `HALE_DNA_OIDC_CLIENT`, `HALE_DNA_OIDC_CLIENT_VAULT`, and per-role `HALE_DNA_DATABASE_DSN` or `HALE_DNA_BROKER_URL`, `_USER`, `_VAULT`, `_SUBJECTS`, `_NODE_USER`, `_NODE_VAULT`. |
| `host add \| probe \| remove <name>` | Board commands on the habitat head (`Commands::add_host`, `::probe_host`, `::remove_host`). `add` probes `<name>` over SSH (`--user`, `--port 22`), trusts the first-seen host key, and prints the habitat's public key line for `~/.ssh/authorized_keys` (marking the host `pending-key`). `probe` re-probes capabilities (`compose`, `systemd-user`, OS, arch) and marks authorized hosts `ready`. `remove` is planned first (`Commands::plan_remove_host`): on a host with allocations, the plan lists each allocation and what stops or is deleted there, and nothing is touched until the host's name is typed back at a terminal (`Commands::remove_host` checks it); `--plan` prints the plan and exits. |
| `organism remove <name> [--plan]` | Removes an organism the habitat grafted, planned first (`Commands::plan_remove_organism`): its record and memory, genome repository, Forgejo repository and account, allocations by host, compose services and projects, vault slots and its rows; nothing is touched until its name is typed back at a terminal (`Commands::remove_organism` checks it, with the plan's digest). |
| `brief` | Prints the head's `Commands::brief` over its socket: the Board's charter, the rules a pairing agent works by, and the live state (hosts, allocations with their pulse, organisms, saplings, Reviews and sapling revisions waiting for the Board). It reads the record alone, never the vault. |
| `status` | Prints the habitat's genome, body, face URL, the head's MCP endpoint (`mcp://127.0.0.1:<port>`) while the face runs, and every allocation (organism, service status, memory/nerves schema versions vs. toolchain, and last heart migration result). |
| `down [--forget [--plan]]` | Stops the habitat body, face, and all Compose services, and removes nothing. `--forget` removes the habitat whole, planned like terraform: the plan groups its record and memory, each grafted organism, the compose projects and volumes, the vault slots (names only), its Forgejo and accounts, its hosts and what runs on them over SSH, each line with its size or count and whether it is recoverable, and nothing is touched until the habitat's name is typed back at a terminal (no `--yes`; refused without a terminal). `--plan` prints the plan and exits. The line each host's `authorized_keys` took cannot be removed by the habitat and is noted. |
| `migrate [organism]` | Starts the organism's allocations and migrates its Postgres memory schema and NATS JetStream stream to the current toolchain version, saving the head DSN in `.hale/dna/memory.env` (and recording versions in `allocation.json`). Uses `HALE_DNA_MEMORY_DSN_OWNER` / `HALE_DNA_NATS_URL_OWNER` when set. |
| `sweep <root> \| --test-vaults [<root>]` | Tears down test habitat Compose projects (containers, networks, volumes) listed under `<root>/.dna-compose-projects` or stale test vaults under `<tmp>/hale-test-vaults-<uid>`, never touching the developer's `$HOME` habitat. |

---

### Genome & Toolchain (`hale dna`)

See [Getting started](./getting-started.md) and [DNA, the building block](./dna.md).

```text
hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain, and propose this version's practices and library
                             (each changed practice, and the library as new families, supersede the active one once the Board approves)
hale dna profile [project]   the organism's combination, detected from its pieces: record, body, head, fleet, knowledge, trust
hale dna --embedded-digest [--from-tree <dir>]
                             the digest of the DNA source this binary embeds (nothing else on stdout);
                             with a checkout, what that tree would embed — a mismatch means the binary
                             predates the working tree and a mutation run against it proves nothing
```

### The Record & Cross-Record Connections

See [Memory and the record](./memory.md).

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

### Memory (Postgres), Ledger, and Receipts

See [Memory and the record](./memory.md).

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

### Nerves (NATS JetStream)

See [The nerves](./nerves.md).

```text
hale dna nerves migrate [dir]
                             create the organization's NATS JetStream stream with the owner's URL (HALE_DNA_NATS_URL_OWNER,
                             or the organism's nerves allocation, brought up) and print its token and each role's URL
hale dna nerves urls [dir]   the token and each role's URL of nerves that are up; nothing brought up or created
hale dna nerves drop [dir]   delete the organization's stream, and everything it held, with the owner's URL
```

### The Spine

See [The spine](./spine.md).

```text
hale dna run [project] [--port N] [--no-iris]
                             build and run the organization (dna/org), a node of it: relay the record's requests
                             to it over the nerves (NATS); iris inspects its process
hale dna definitions [project] [--json]
                             the workflow catalog (dna/org/workflows.hl): each definition's revisions and every step's store
hale dna effect resolve <key> an effect whose outcome is unknown after a restart: --outcome ok|failed, in your name
```

### Heart, Body, Codebases, Fleet, and Nodes

See [The heart and the body](./heart.md).

```text
hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]
                             the organization AND the application under one host: rebuild and restart
                             the application on an apply, watch the window, report back
hale dna application remove [project] [--as <who>]
                             the attached application removed (`application.detached` in the record) and its
                             broker account revoked: its user, its password and its vault entry
hale dna codebase [list | attach <name> --repo <url> --build <cmd> --test <cmd> [--branch <name>] | detach <name>]
                             a foreign codebase attached with no application (`codebase.attached` in the record): a leg
                             works a branch of its repository, verifies with the two commands, and its review opens there
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
                             (`[dna] fleet = "<name>"` in hale.toml names the fleet seed; `hale node <name>` runs a node)
```

Fleet node agent (`hale node --help`):

```text
usage: hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]

The agent that expresses a fleet's instances on one machine, from
the record: it reconciles the instances the fleet places on this
host (`host("<name>")`) with what is running here. With HALE_DNA_NATS_URL_APP
and HALE_DNA_NATS_ORG set (`hale dna nerves migrate` prints them), it
hands both to every instance, which says what it says onto the
nerves itself.
```

### Senses and Reflexes

See [Senses and reflexes](./senses.md).

```text
hale dna senses up [dir]     bring up the organism's senses' store in the local habitat and print its read URL
hale dna senses url [dir]    the read URL of a senses' store that is up; nothing brought up
hale dna pressure [raise <source> <what…>]
                             pressure raised and answered; `raise` writes one signal into the record, which a node relays
hale dna concern raise <source> <what…> [--severity N]
                             a concern from a locus path about the part above it; persistent ones become knowledge proposals
```

### The Skin (Secrets & Vault)

See [The skin](./skin.md).

```text
hale dna secret set <NAME> [--body <user@host>]
                             a credential from stdin (never argv, never the record) into its slot of the vault
                             (a model key's, FORGE_TOKEN, or OIDC_CLIENT_SECRET) on the body or here; `secret rotate <NAME>`; the record gets `secret.rotated <NAME>` only
                             a secret the seed declares ([dna.secrets.<name>], a slot) by its own name; a family's member
                             with `--member <label>` (also `secret remove <name> --member <label>`); a drawn one is never set,
                             and `secret rotate <name>` draws it anew
hale dna secret show <name> [--member <label>]
                             the value of a secret the seed declares with person = "read" (any other is refused)
hale dna secrets [dir]       every secret the organism requires, and whether the vault holds it (never a value)
hale dna declared-secrets [dir]
                             the secrets <dir>/hale.toml declares, a line each: name, kind, person, programs, host commands, family
hale dna declared-provides [dir]
                             what <dir>/hale.toml provides the habitat's organisms, a line each: capability, service, path, models_path, key
hale dna declared-uis [dir]
                             the web UIs <dir>/hale.toml declares, a line each: name, label, service, path, login
```

### Worker Legs and Voice (Models)

See [Legs, hands and voice](./legs.md).

```text
hale dna work <verb> …       a leg's verbs against the head's API (--api, --as position:<name>): next, brief, login,
                             renew, allowance, submit (--from-worktree), settle, release, friction, hand, run — the project's performers (dna/org/work.hl);
                             loop --parallel N is a worker: N children, each its own holder; loop --drain ends one
hale dna models [project]    the catalog (dna/org/models.hl): every backend, and one small request to each
hale dna models map | rule <selector> <size> [--permit <sizes>] | category <mode>.<size> <model> [--tools] [--price-in N] [--price-out N] [--endpoint <url>] [--key-vault <slot>]
                             which model a Work gets, as rows in your name (--as <who>): a rule says the size a kind of work needs
                             (default, position:<name>, contract:<Name>, task:<id>; quick standard deep), a category which model fills it
```

### Schedules

See [Schedules](./schedules.md).

```text
hale dna schedule [pause <id> | resume <id>]
                             the schedules the record declares (a definition on an interval or a cron, and who
                             convenes it) and their occurrences; pause and resume are rows in your name (--as <who>)
hale dna schedule declare <id> (--every <n>ms|s|m|h|d | --cron <expr>) --definition <id> --convener <position> [--args <json>]
                             a schedule asked of the organization, which declares it or refuses it; an occurrence
                             is an execution of the definition
```

### Head, Browser Face, Board, and Reviews

See [The head and the face](./head.md).

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
hale dna forge sync          mirror pending Reviews to the allocated forge and read its reviews back as verdicts
                             (the habitat's Forgejo by default; `hale habitat forge <organism> github --repo owner/repo` for GitHub, via `gh`; dna.github.board logins,…)
hale dna report [project]    file a report from the record since the last one (report.filed)
```

### Tasks and Human Workflows

See [One task, end to end](./workflow.md).

```text
hale dna task create [--to <locus>|position:<name>] [--as <who>] [--judgment] [--size quick|standard|deep] [--codebase <name>] [--no-wait] <outcome…>
                             ask for an outcome (--judgment: an assessment, a leg's to perform; --to position:<name>: a change the leg holding that position makes): a row in the record, which a node relays to the organism; prints the Task born or the refusal
hale dna task done <id>      a person reports a handed Task done (--as <who>, --note …); `task reassign <id> --to <who>`
                             under an acceptance practice requiring evidence: --evidence <digest>, or --exception <why> --authorized-by <who>
hale dna task authorize <id> --exception <why>   authorize an exception, in your name (not the assignee's)
hale dna task decide <id>    report a decision someone else made (--decided-by <party> --via <channel> --evidence <digest>, --as <reporter>)
hale dna retire <who>        a person retires: the handed Tasks they hold move to --to <successor>, as rows
```

### Shaping the Organization (`fill`, `position`, `apply`, `pack`, `practice`)

See [Shaping and governing it](./shaping.md).

```text
hale dna fill <position> <holder> [project] [--as <who>]
                             ask the organization to propose who holds a position, for the Board
hale dna ingest [project] [--at <rev>]
                             read the repository's graph again at a commit (HEAD): what differs from the record, tools
                             for every served operation included, proposed as one Board Review (`review ingest approve`)
hale dna position open <name> --mandate "<text>" [--text <what it is>] [--under <part>] [--as <who>]
                             open a position at run time: its node and its mandate, one Board Review (`review positions approve`)
hale dna apply <file> [--dry-run] [--as <who>] [--on-holes]
                             several changes to the structure (`open`, `fill`, `fill *`, `vacate`, `practice`, `law`, `idea`, `assign`, `unassign`, `close`, `merge`,
                             `split`, `expose`, `depend`, `externalize`, `internalize`, `retire-practice`) as one Board Review, applied all or nothing
                             (`review changes approve`); --dry-run prints the org chart before and after and proposes nothing; --on-holes
                             plans it on the holes the graft proposed that the Board has not decided, and ratifies it after them
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

---

## Environment Variables and Git Configuration

### Operator Environment Variables

The table below lists environment variables set by operators; internal child-process variables are managed automatically by the CLI and host. The complete normative list is maintained in [`spec/dna.md#environment`](https://github.com/hale-lang/hale/blob/main/spec/dna.md#environment) and [`spec/runtime.md#diagnostic--tuning-env-vars`](https://github.com/hale-lang/hale/blob/main/spec/runtime.md#diagnostic--tuning-env-vars).

| Variable | Purpose |
| --- | --- |
| `HALE_DNA_MEMORY_DSN_OWNER` | External Postgres owner DSN (used instead of the local habitat allocation) for schema migrations (`hale habitat migrate`, `hale dna memory migrate`, `hale dna upgrade`); `dev`, `head`, and `show` derive role DSNs from it without migrating. |
| `HALE_DNA_NATS_URL_OWNER` | External NATS owner URL (used instead of the local habitat allocation) for stream creation (`hale habitat migrate`, `hale dna nerves migrate`); `dev` and `nerves urls` derive role URLs from it. |
| `HALE_DNA_MEMORY_DSN_SPINE` | Postgres spine role DSN required by `hale dna run` (printed by `hale dna memory migrate`). |
| `HALE_DNA_NATS_URL_SPINE`, `HALE_DNA_NATS_ORG` | Spine NATS URL and organization stream token required by `hale dna run` (printed by `hale dna nerves migrate`). |
| `HALE_DNA_MEMORY_DSN_HEAD` | Postgres head role DSN used exclusively by the head API server (printed by `hale dna memory migrate`). |
| `HALE_DNA_RECEIPT_KEY` | Encryption key (16+ characters) used to seal `customer` and `confidential` evidence in Postgres memory; passed once to `hale dna memory migrate`. |
| `HALE_DNA_OWNER_KEYS` | Space-separated `<owner>=<key>` pairs defining organization owners on a shared record during `hale dna memory migrate`. |
| `HALE_DNA_API`, `HALE_DNA_ID_TOKEN` | Default head HTTP URL and OIDC bearer token used by `hale dna work` when `--api` is omitted. |
| `HALE_DNA_GENOME_POLL` | Interval in seconds (default `300`) between polls of the remote Git repository for genome updates. |
| `HALE_DNA_NO_BUILD_CACHE` | When set, forces fresh compilation of every seed instead of reusing cached binaries. |
| `HALE_DNA_COMPOSE=off` | Prevents a repository's body from automatically starting and stopping its `compose.yaml` services. |
| `HALE_VAULT_DIR`, `HALE_VAULT_ADDR`, `HALE_VAULT_TOKEN` | Vault location: local filesystem directory (`HALE_VAULT_DIR`), or HTTP Vault API URL and token (`HALE_VAULT_ADDR`, `HALE_VAULT_TOKEN`). |

### Repository Git Configuration Keys (`.git/config`)

| Git Config Key | Purpose |
| --- | --- |
| `dna.remote` | Git remote name used for genome synchronization (defaults to `origin`). |
| `dna.trust` | `local` (trusts all writers to the record) or `signed` (requires verified cryptographic signatures on relayed rows). |
| `dna.owner` | Organization identity that this body represents on a multi-owner shared record. |
| `dna.body` | Default `<user@host>` SSH target set by `body provision` or `hale habitat graft --body` (used by `body start`, `stop`, `logs`, and `secret set`). |
| `dna.unix.member` | `uid:<n>=<name>` mapping from local OS UID to person identity on the head's Unix socket. |
| `dna.principal` | Set to `oidc` to enable OpenID Connect authentication on the head and `hale dna ui`. |
| `dna.oidc.issuer`, `dna.oidc.client`, `dna.oidc.redirect` | OIDC issuer URL, client ID, and callback redirect URI. |
| `dna.oidc.member` | `<subject>=<name>` mapping from OIDC `sub` claim to a person in the record (multi-valued). |
| `dna.oidc.service` | `<subject>=<service>` mapping from OIDC `sub` claim to a read-only service principal (multi-valued). |
| `dna.oidc.board` | Comma-separated member names authorized to act with `board` authority. |
| `dna.oidc.key` | Base64 `SubjectPublicKeyInfo` pinned signing key required for loopback OIDC issuers. |
| `dna.github.board` | Comma-separated logins whose reviews at the forge carry `board` authority (the repository is the forge allocation's, `hale habitat forge`). |

---

## Record and Ledger Row Vocabulary

Every event row contains a `kind`, `entity`, `body`, and `author`. In Postgres memory (schema `dna_<record-identity>`), rows are routed by `dna/core/routing.hl` (`memory_of`) into either `record_rows` (**record**: governance, structure, mutations, and reviews) or `ledger_rows` (**ledger**: day-to-day task execution, leases, and readings), and read back as a single unified sequence ([Memory and the record](./memory.md)).

### 1. Record, Graph, Evidence, and Cross-Record Connections

| Row Kind | Table | Description |
| --- | --- | --- |
| `structure.observed` | `record` | Compiler topology model of an application locus or topic captured at graft time. |
| `graph.node` / `graph.edge` / `graph.retired` | `record` | Addition or retirement of a node or hyperedge in the repository knowledge graph. |
| `graph.ingested` | `record` | Summary of `hale dna ingest` at a commit (`added`, `changed`, `retired`, and proposal `review_id`). |
| `graph.requested` / `graph.proposed` / `graph.refused` | `record` | Request to open an ingest/graph Review, the resulting Review, or a projection refusal (`by: memory`). |
| `model.rule` | `record` | Model size routing rule (`hale dna models rule`, `task create --size`). |
| `model.category` | `record` | Model backend assignment, `--tools` capability flag, and pricing for `<mode>.<size>` (`hale dna models category`). |
| `responsibility.proposed` | `record` | Inferred one-line locus responsibility proposal awaiting ratification. |
| `candidate.dropped` | `record` | Removes a retained candidate branch locally and across clones on sync. |
| `receipt.filed` | `ledger` | Internal document filed as evidence. |
| `receipt.classified` / `receipt.withheld` | `ledger` | Protected evidence (`customer` / `confidential`) sealed in Postgres memory, or withheld when no memory was available. |
| `receipt.disclosed` | `ledger` | Grants a named reader access to a protected receipt for a stated purpose. |
| `receipt.read` / `receipt.read_refused` | `ledger` | Audit row recording an authorized read (or refusal) of protected evidence. |
| `receipt.held` / `receipt.hold_released` | `ledger` | Retention hold preventing redaction of a receipt digest, and its release. |
| `receipt.redacted` | `ledger` | Permanent erasure of a receipt body while retaining its digest and policy note. |
| `connection.proposed` / `connection.closed` | `record` | Proposal to connect to an external record (`hale dna connect`), and connection closure (`hale dna disconnect`). |
| `handoff.published` / `handoff.refused` | `ledger` | Fact published across a connection (`hale dna handoff`), or refusal reason. |
| `handoff.received` | `ledger` | Incoming handoff envelope in the receiving record's exchange inbox. |
| `handoff.accepted` / `handoff.accepted_by_peer` | `ledger` | Acceptance of a received handoff locally, and confirmation synced back to the origin record. |
| `task.transfer_requested` / `task.transfer_accepted` | `ledger` | Cross-record or cross-owner Task transfer offer and acceptance. |

### 2. The Spine (Intents, Workflows, Leases, and Budgets)

| Row Kind | Table | Description |
| --- | --- | --- |
| `intent.requested` | `ledger` | Task intent recorded from a clone when no organism body is running (`--no-wait`). |
| `intent.offered` / `intent.refused` | `ledger` | Intent admitted by the spine, or refused with a reason. |
| `intent.unrecovered` | `ledger` | Intent offered before a restart before a `task.born` row was committed (never re-offered automatically). |
| `task.born` | `ledger` | Task created from an admitted intent or settled Review. |
| `workflow.admitted` / `workflow.refused` | `ledger` | Workflow execution admitted (definition, revision, inputs, bound recipe) or refused. |
| `workflow.ask_refused` | `ledger` | Workflow request refused before minting a Task. |
| `workflow.settled` | `ledger` | Workflow execution completed with disposition `done`, `failed`, or `cancelled`. |
| `step.registered` / `step.activated` / `step.completed` / `step.failed` | `ledger` | Lifecycle transitions of an individual workflow step. |
| `attempt.admitted` / `attempt.outcome` | `ledger` | Work attempt admitted by the spine, and its final settled disposition, result, and evidence. |
| `work.settled` | `ledger` | Unit of Work settled, referencing the winning attempt ID. |
| `effect.requested` / `effect.result` | `ledger` | Exclusive idempotency claim on an external effect key, and its resolved outcome. |
| `effect.relayed` | `ledger` | Attempt queued by `dna::LegRelay` for external worker legs to claim via the head. |
| `effect.redelivered` | `ledger` | Claimed attempt re-dispatched after a restart. |
| `claim.taken` / `claim.released` | `ledger` | Node execution claim taken prior to acting, and its release. |
| `lease.taken` / `lease.renewed` / `lease.released` | `ledger` | Runtime lease transitions with fencing tokens. |
| `budget.exhausted` | `ledger` | Model spend budget window exhausted. |
| `model.called` | `ledger` | LLM invocation audit record (adapter, backend, model, tokens, micro-dollar cost, latency). |
| `grant.contracted` / `grant.revoked` / `grant.refused` | `record` | Autonomy grant narrowed, revoked, or clamped at startup because it exceeded its `ceiling`. |
| `grant.reservation_refused` / `grant.fenced` | `ledger` | Monetary spend refused by window limit or stale grant epoch. |
| `spend.reserved` / `spend.settled` / `spend.compensated` | `ledger` | Monetary spend reserved against a grant, settled with actual cost, or refunded/compensated. |
| `optimize.refused` | `ledger` | Scheduled `optimize-walk` pass skipped (e.g., due to an exhausted budget window). |
| `org.reviewed` | `record` | Outcome of the Leader's `optimize-walk` pass over organizational signals. |

### 3. Worker Legs (`hale dna work`)

| Row Kind | Table | Description |
| --- | --- | --- |
| `attempt.claimed` | `ledger` | Attempt lease claimed by a leg position holder at the head. |
| `attempt.outcome_requested` / `attempt.outcome_refused` | `ledger` | Outcome submitted by a leg under its lease, or refusal by the spine. |
| `attempt.allowance_requested` | `ledger` | Leg request for an attempt's model spend allowance. |
| `attempt.allowance_granted` / `attempt.allowance_refused` | `ledger` | Budget gate response for a leg attempt, editor pass, or Leader Review evaluation (`review:<id>`). |
| `attempt.released` | `ledger` | Lease voluntarily released by a leg without an outcome. |
| `attempt.unresolved` | `ledger` | Attempt by an `uncertain` performer whose outcome failed to settle or whose lease expired; awaits `hale dna effect resolve`. |
| `friction.filed` | `record` | Operational friction reported by a worker (`hale dna work friction`). |

### 4. Code Mutations and Reviews

| Row Kind | Table | Description |
| --- | --- | --- |
| `mutation.requested` / `mutation.proposed` | `record` | Mutation requested by a Work/attempt, and its proposed change class, objective, target, and base commit. |
| `mutation.worktree` / `mutation.located` / `mutation.candidate` | `record` | Scratch worktree path, located target files and grant, and resulting candidate commit SHA. |
| `mutation.review` / `.stage` / `.escalate` / `.release` / `.deny` | `record` | Autonomy boundary `dispose()` outcome for the candidate. |
| `mutation.topology` | `record` | Candidate re-classified to `topology` (Board authority) because its diff touches a fleet seed or `hale.toml`. |
| `mutation.applied` / `mutation.apply_retried` | `record` | Candidate commit applied to the genome branch, or retried. |
| `mutation.retained` / `mutation.rolled_back` | `record` | Applied mutation kept after passing its observation window, or rolled back to base on failure. |
| `mutation.rejected` / `mutation.revise` / `mutation.refused` / `mutation.failed` | `record` | Candidate rejected or sent back by a reviewer, refused at apply time, or failed verification. |
| `evidence.<step>` | `record` | Receipt digest for a verification step (`base`, `fmt`, `check`, `verify`, `test`, `fleet`, `replay`, `rollback`, `diff`). |
| `evidence.magnitude` | `record` | Computed 10-dimensional semantic magnitude vector of the candidate. |
| `review.requested` | `record` | Opens a Review (question, required authority, candidate digest, disposition, evidence, and diffs). |
| `review.routed` / `review.signed` | `record` | Required signer positions and CI gates from `hale dna route`, and individual signer approvals prior to final settlement. |
| `review.verdict` / `review.command_decided` | `record` | Verdict submitted by a reviewer (CLI, UI, or GitHub PR sync). |
| `review.settled` / `review.refused` / `review.reasoned` | `record` | Final settlement of the Review, verdict refusal reason, and reviewer comment. |
| `gate.observed` | `record` | CI check run status at the candidate commit observed from the forge. |
| `github.pr` / `github.commented` | `record` | The pull request a Review opened at the forge (its url, number and forge) and the settlement commented back by `hale dna forge sync`. |

### 5. Heart, Body, Codebases, and Fleet

| Row Kind | Table | Description |
| --- | --- | --- |
| `application.attached` / `application.detached` | `record` | Application seed attached to the organism, or detached via `hale dna application remove`. |
| `codebase.attached` / `codebase.detached` | `record` | Foreign repository attached via `hale dna codebase attach` (repo URL, build/test commands, default branch) or detached. |
| `task.delivered` / `task.refused` | `ledger` | Final verdict on an attached codebase task (merged PR / approved brief vs. closed PR / unapproved brief, with PR diffs and comments attached as receipts). |
| `expression.restart_requested` | `record` | Spine request for the host to rebuild and restart the application at a newly applied revision. |
| `expression.restarted` / `expression.deployed` | `record` | Confirmation that the new runtime expression started or deployed. |
| `expression.observed` / `expression.crashed` | `record` | Post-deploy observation window outcome (clean survival vs. crash with exit code). |
| `observation.requested` / `observation.refused` | `record` | Host observation report submission. |
| `fleet.deploy` | `record` | Genome revision targeted for fleet expression (`hale dna deploy`). |
| `instance.up` / `instance.exited` | `ledger` | Per-instance process status reported by `hale node <name>`. |
| `node.started` / `node.build_failed` | `record` | Node agent startup at a genome revision, or build failure keeping the prior revision running. |
| `body.claimed` / `body.released` | `ledger` | Fenced body lease acquisition or release. |
| `body.provisioned` | `record` | Remote host provisioned via `hale dna body provision`. |
| `body.credential_missing` / `body.credential_present` | `ledger` | Whether the body's vault holds the required model credentials. |

### 6. Senses, Reflexes, Skin, and Schedules

| Row Kind | Table | Description |
| --- | --- | --- |
| `reading.recorded` | `ledger` | Application telemetry or upstream watch event recorded before any reflex acts. |
| `reflex.acted` | `ledger` | Automated action taken by a node in response to a firing reflex. |
| `pressure.requested` / `pressure.raised` / `pressure.remeasured` | `ledger` | Pressure signal submitted (`hale dna pressure raise`), recorded, and re-evaluated after a change. |
| `concern.requested` / `concern.raised` / `concern.refused` | `ledger` | Upward structural concern raised from a locus path (`hale dna concern raise`). |
| `concern.proposed` | `record` | Knowledge proposal automatically opened after 3 concerns (`concern_threshold`). |
| `appendage.proposed` / `appendage.candidate` | `record` | New organizational position proposed after 3 pressure signals (`appendage_threshold`), and its generated `dna/org` mutation candidate. |
| `secret.rotated` | `record` | Audit record noting that a named secret slot was set or rotated (never contains the secret value); a family's member is `<name>--<label>`. |
| `secret.removed` | `record` | A member of a declared secret family taken out of the vault (`<name>--<label>`), never its value. |
| `violation.recorded` | `record` | Runtime closure fault absorbed by a supervisor (`adapter_undeliverable`, `lease_unsettled`, `pulse_stopped`). |
| `schedule.requested` / `schedule.answered` | `ledger` | Schedule declaration request and organization response. |
| `schedule.declared` / `schedule.refused` | `ledger` | Active schedule specification, or declaration/occurrence refusal. |
| `schedule.skipped` / `schedule.missed` | `ledger` | Occurrence skipped due to an in-flight execution, or missed during organism downtime. |
| `schedule.paused` / `schedule.resumed` | `ledger` | Manual schedule pause or resume (`hale dna schedule pause\|resume`). |

### 7. People, Positions, Batches, and Practices

| Row Kind | Table | Description |
| --- | --- | --- |
| `task.handed` / `task.reassigned` / `case.admitted` / `task.done` | `ledger` | Human task assignment, reassignment, leaf case admission, and completion report (`hale dna task done`). |
| `completion.linked` / `completion.excepted` / `exception.authorized` | `ledger` | Evidence digest linked to a completed task, or third-party exception authorization (`hale dna task authorize`). |
| `decision.reported` | `ledger` | External decision recorded via `hale dna task decide`. |
| `person.retired` | `record` | Person retired from the organization (`hale dna retire`), transferring open tasks to `--to <successor>`. |
| `hold.requested` / `hold.proposed` / `hold.refused` | `record` | Position holder assignment (`hale dna fill`) requested, opened as a Board Review, or refused. |
| `batch.proposed` | `record` | Multi-step structural batch (`hale dna apply` / `hale dna pack apply`) opened under `changes`, recording steps and any `planned_on` hole Reviews. |
| `position.requested` / `position.proposed` / `position.refused` | `record` | Runtime position and mandate proposal (`hale dna position open`). |
| `practice.requested` / `practice.proposed` / `practice.refused` | `record` | Practice proposal lifecycle (`hale dna practice propose`). |
| `knowledge.proposed` / `.ratified` / `.declined` / `.refused` / `.retired` | `record` | Knowledge and practice Review outcomes and version supersession. |
| `knowledge.consulted` / `practice.read` | `ledger` / `record` | Audit rows recording which knowledge nodes and ratified practices were included in a Work's hat. |
| `report.filed` | `ledger` | Summary report generated by `hale dna report`. |

---

## `hale dna status --json` Schema

`hale dna status --json` (and `GET /api/status` on `hale dna ui`) outputs a single-line JSON projection of the organism's state:

```json
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
    }
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

- **`organism`:** `"running (this clone's body holds the lease)"` when the local body holds the active lease, or `"not running — reading the Journal"`.
- **`journal.chain`:** `"verified"` or `"BROKEN"`.
- **`tasks[]`:** Objects with `id`, `outcome`, `state`, `since`, and optional `detail`.
- **`reviews[]`:** Settled Reviews add `settled` and `reasoned`; code Mutation Reviews add `mutation_id`, `change_class`, `seed`, `disposition`, `base_commit`, `candidate_commit`, `candidate_shape`, `evidence`, `magnitude`, `diff_text`, `diff_json`, and `author`.
- **`mutations[]`:** Objects with `id`, `candidate`, `disposition`, `class`, `task`, `objective`, optional `detail` and `worktree`, and `events`.
- **`expression.last_restart_request` / `last_observed`:** Objects with `mutation`, `what`, and `seq`.

---

## Where things live

### Project Checkout Files (`hale habitat graft`)

| Path | Description |
| --- | --- |
| `main.hl`, `tests/main_test.hl` | Starter application and test (created only when grafting a new application directory). |
| `hale.toml` | Package manifest defining `[environments.local]` (`.`), `[environments.org]` (`dna/org`), and `[dna] record`. |
| `hale.lock` | Lockfile pinning `[dna] toolchain = "<version>"`. |
| `.gitignore` | Ignores compiled binaries, `/vendor/`, and `/.hale/`. |
| `vendor/dna/` | Toolchain-managed DNA core library (git-ignored; updated via `hale dna upgrade`). |
| `dna/org/main.hl` | Organization entrypoint locus (`Org`), positions, `dna::Grant`, and NATS bindings. |
| `dna/org/law.hl` | Organization constitutional claims (`constitution Org`). |
| `dna/org/purpose.hl` | Project purpose statement (`PURPOSE`). |
| `dna/org/charter.hl` | Leader architectural brief (`CHARTER`). |
| `dna/org/models.hl` | Model backend catalog and `org_budget()`. |
| `dna/org/work.hl` | Worker leg performer catalog (`performers()`). |
| `dna/org/workflows.hl` | Baseline workflow catalog (managed by `hale dna upgrade`; do not edit by hand). |
| `dna/org/own_workflows.hl` | Custom project workflow definitions. |

### Runtime State, Databases, and Scratch Directories

| Location | Contents |
| --- | --- |
| **Postgres (`dna_<record-identity>`)** | `record_rows` (signed append-only governance/mutation rows), `ledger_rows` (operational rows), `receipts` (content-addressed evidence; protected classes sealed under `HALE_DNA_RECEIPT_KEY`), `leases` (fenced leases including the body lease), and the knowledge/org graph under spine and head database roles. |
| **`hale.toml` (`[dna] record`)** | The record's immutable identity (SHA-256 digest of its `record.genesis` row). |
| **`.hale/dna/memory.env`** | Mode `0600` file (git-ignored) storing the derived Postgres head role DSN (`HALE_DNA_MEMORY_DSN_HEAD` and `HALE_DNA_MEMORY_FROM`), written by `hale habitat migrate` and `hale dna memory migrate`. |
| **`refs/heads/dna/candidate/<mutation>`** | Git branch retaining each Mutation's candidate commit regardless of Review outcome (until dropped via `hale dna candidates drop`). |
| **`refs/hale/remote/genome`** | Last fetched tip of the remote forge's default branch on a fleet node. |
| **`.git/config`** | Local repository settings (`dna.trust`, `dna.unix.member`, `dna.oidc.*`, `dna.github.*`). |
| **`.hale/dna/`** | Local clone scratch directory: `status.json`, `org.log`, `baseline.topology`, `embedded.digest`, seed build cache (`build/`), mutation sandboxes (`worktrees/`, `scratch/`), compiled leg binary (`legs/`), interactive human worktrees (`work/<attempt>/`), and temporary ingest checkouts (`ingest/<sha>/`). |
| **`.hale/node/<name>/`** | Fleet node instance PID files and build artifacts. |
| **NATS JetStream (`nerves`)** | One JetStream stream per organization, named after its uppercase stream token. |
| **Local Habitat (`~/.local/share/hale/habitat/local`)** | Long-lived Docker Compose project (`hale-habitat-local-<h>`) running `memory-<id>`, `nerves-<id>`, and `senses-<id>` per organism (`<id>` = first 12 hex digits of the record identity). Per-allocation configs (`compose.yaml`, `ports.yaml`, `postgres.secrets`, `nats.conf`, `nats.secrets.conf`, `senses.yml`, `allocation.json`) live under `allocations/<id>/`. |
| **Vault (`~/.cache/hale/vault`)** | Mode `0600` secret slot files under `HALE_VAULT_DIR` (or `$XDG_CACHE_HOME/hale/vault`, else `~/.cache/hale/vault`), or an external HTTP Vault server at `HALE_VAULT_ADDR`. |
| **`HALE_DNA_HEAD_STATE`** | Project head operational state and receipt logs (defaults to `${XDG_STATE_HOME:-$HOME/.local/state}/hale/dna/head`). |
