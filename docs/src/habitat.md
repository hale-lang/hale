# The Habitat Layer: Multi-Organism Infrastructure and Isolation

A **habitat** is the root DNA organism shipped with the Hale toolchain (`spec/habitat.md`). Its role is to provision, supervise, and place infrastructure allocations (Postgres memory, NATS JetStream nerves, Prometheus senses stores, and vault roles) for child organisms. Every project organism is born as a child of the habitat via `hale habitat graft`—the habitat itself is the only organism booted without a parent (`hale habitat up`).

```sh
# 1. Start the local habitat organism, its Compose services, and its operator UI
#    (named at its birth; `local` when no --name is given, and kept after)
hale habitat up --name homelab

# 2. Graft a new application, or a sapling from a nursery the habitat trusts
hale habitat graft my-service
hale habitat nursery add https://github.com/hale-lang/voice
hale habitat graft voice

# 3. Inspect all running allocations, schema versions, and host placements
hale habitat status
```

> [!NOTE]
> **Implemented Today vs. Architectural Design:** This chapter covers both the production habitat commands shipped today (`hale habitat up`, `graft`, `migrate`, `nursery`, `host`, `handover`, `brief`, `status`, `down`) and the broader multi-record architecture (`runs_under` principal projections, cross-record `extract`/`absorb`, and financial books). See [Implementation Status](#implementation-status) at the end of this page for a complete feature matrix.

---

## How the Habitat Organism Works

Unlike an application organism that supervises a user-written application heart, the habitat's heart slot is empty: what it keeps alive are the infrastructure backends and bodies of its child organisms.

- **Fixed Genome and Law:** The habitat's genome is embedded in the `hale` binary with fixed positions (`placement`, `backends`, `identity`, `admission` from the `habitat` nursery pack) and the `standard` nursery pack as its law.
- **Root Bootstrap (`hale habitat up`):** The habitat is the sole organism born without a parent. Running `hale habitat up` supports two profiles:
  - `local` (built today): Provisions a long-lived Docker Compose project (`hale-habitat-local-<hash>`) under `${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local`, and starts the habitat's backing services alongside its own `body` (`hale dna dev`) and browser `face` (`hale dna head`) as services of that Compose project (built from the embedded `Dockerfile` and `hale.pin`, or from a prebuilt local image via `--image <tag>` / `HALE_HABITAT_IMAGE` and `hale habitat image build`). By default (`--backends own`), it manages its own Postgres, NATS, Prometheus, and Forgejo containers; with `--backends operator`, it adopts the external servers named by `HALE_DNA_MEMORY_DSN_OWNER` and `HALE_DNA_NATS_URL_OWNER`. The `body` and `face` containers mount the host's Docker socket so the habitat can run and probe every organism's services; that socket is root on the host, so a habitat is trusted as the machine's administrator is.
  - `local` on another machine (`hale habitat up --host <name> [--user <user>] [--port <n>]`): the same habitat on a LAN machine that has Docker and nothing else. Your own SSH login reaches it; the toolchain image is used if it is there already, else sent from your Docker, else pulled as the published image when your `hale` is a release's build, else built there once from your `hale`'s commit on GitHub (the machine's Docker needs its `buildx` plugin; the first build compiles the compiler and takes a while); `hale habitat up` runs there in that image, its face and head bound to the address `<name>` resolves to and recorded at `http://<name>:<port>`. This machine keeps only a pointer (the name, the host, the head's URL and launch token, the record's identity): `status` asks the head through it, `host`, `graft <sapling>`, `place`, `apply` and `model-map` go to the head as they do locally, `down` and `down --forget` run on the habitat's machine over SSH (the forget still confirmed by its name typed at your terminal, the pointer removed last), `graft <dir>` ships the directory there (it becomes the organism's home, the habitat's copy) and grafts it there, `hale habitat organism <name> dna <verb>` runs an organism's own verbs (`review`, `task create`, `work run`) in its directory there, and `migrate` and `handover` are refused. You sign in to the remote face from your own browser: the habitat's OIDC stub records its issuer under `http://<name>:<port>` too, and plain HTTP is accepted for exactly that name (and the loopback), no other host.
  - `hosted` (roadmap): Connects to an existing platform's managed Postgres, NATS, Vault, and OIDC identity provider.
- **Child Birth via `hale habitat graft`:** Every other organism is born as a child of the habitat via `hale habitat graft`. During a graft:
  - The child's `record.genesis` row records the habitat's record identity as its parent.
  - The habitat allocates the infrastructure roles declared in the child's `hale.toml` (`[dna] needs`: `memory` and `nerves` always, `senses` unless omitted, plus an optional tenant `database` for a Hale heart passed as `HALE_DNA_DATABASE_DSN`) and records those allocations in the habitat's own record.
  - The habitat decides the `scope` on `application.attached` and issues per-client database, broker, and vault credentials.
  - The habitat proposes the `standard` practices pack to the child's Board.
  - On its own backends, the habitat gives the child a user and a repository at its own Forgejo, the local profile's forge. The forge is a gate, not a store: code goes out through it as branches and verdicts come back through it. GitHub is the allocation when a Board names it (`hale habitat forge <organism> github --repo <owner/name>`).
- **Deployment and Reflexes:** For habitat-scoped applications, the habitat elaborates placement declarations into container manifests or Compose configurations, applies them using habitat-held credentials, monitors child readiness, capacity, metrics, and logs, and restarts or rescales child bodies via reflexes.

---

## Hosts, Placement Providers, and Saplings

### Hosts and Providers (`compose` and `ssh`)

In habitat commands, a **host** always refers to a physical or virtual machine where allocations can be placed. **Providers** discover hosts, allocate roles for organisms, render and apply service definitions on a target host, and observe live health (`up`, `degraded`, or `down`). Every host, allocation, and role-to-provider **policy** is stored as a row in the habitat's record and exposed on the habitat head's surface (`Commands::hosts`, `Commands::allocations`, `Commands::policy`).

| Provider / Verb | Behavior |
| --- | --- |
| **`compose` provider (`local` profile)** | Produces the `local` host (advertising `compose`) and runs all habitat and child organism allocations (`memory-<id>`, `nerves-<id>`, `senses-<id>`, `forge`, and the habitat's `body` and `face`) as Docker Compose services on the local machine. |
| `hale habitat host add <name> [--user <u>] [--port <n>]` | Probes a LAN machine over SSH using the habitat's vault-held SSH keypair, pins the first-seen host key in the record (refusing any subsequent host-key mismatch), prints the habitat's public key line to append to `~/.ssh/authorized_keys` on `<name>`, and marks the host `pending-key`. |
| `hale habitat host probe <name>` | Re-probes `<name>` over SSH. Once `~/.ssh/authorized_keys` is updated, transitions the host from `pending-key` to `ready` and records its capabilities (`docker compose`, `systemd-user`, OS, and CPU architecture). |
| `hale habitat host remove <name> [--plan]` | Removes a registered host from the habitat record, planned first: on a host with allocations the plan lists each one and what stops or is deleted there, and the removal waits for the host's name typed back at a terminal (`--plan` prints the plan and exits). |

### Saplings and Nurseries (`hale habitat nursery` / `graft <sapling>`)

While a **pack** is a reusable template of rows adopted inside an existing organism (positions, mandates, practices, knowledge), a **sapling** is a complete, prebuilt organism template published in a nursery and pinned by repository URL, Git revision, file tree SHA-256 digest, and default packs.

```sh
hale habitat nursery list
hale habitat nursery add https://example.com/nursery.git
hale habitat nursery refresh
hale habitat nursery ratify <sapling>
hale habitat nursery add https://github.com/hale-lang/voice   # voice is its own nursery
hale habitat graft voice
```

1. **Grafting by name (`hale habitat graft <sapling>`):** Pass a bare sapling name (`hale`, the one the toolchain carries, or one a trusted nursery offers, such as `voice`; `./<name>` denotes a local directory). A repository can be its own nursery: a `nursery.toml` at its root whose `[[sapling]]` says `repo = "."` (and no revision or digest) offers the repository itself, pinned at the commit the habitat reads. That is how `hale-lang/voice` is offered; the toolchain bundles no application. The habitat fetches the pinned revision into `<habitat>/saplings/<name>`, verifies its file digest against the nursery pin (aborting on any mismatch), grafts it as a child of the habitat, proposes its bundled packs, and records the sapling name, revision, and digest on the graft row. The `hale` sapling (the organism that works on the Hale toolchain) is a tree the toolchain itself carries, with no remote. `hale habitat graft hale` makes it the organism's own repository in the habitat and checks its digest the same way. Like every graft, it mirrors that repository to the habitat's Forgejo; nothing of an organism's own goes to GitHub unless its Board allocates GitHub later.
2. **A genome's codebases attach at its graft:** a `[[dna.codebase]]` list in the sapling's `hale.toml` (each with `name`, `repo`, `branch`, `build`, `test`, `toolchain`) becomes `codebase.attached` rows of the grafted organism's record, once each; nothing is cloned until a leg works one. The toolchain nursery's `hale` sapling is **the hale organism**: its genome is its own small repository, and its codebases are `hale-lang/hale` and `hale-lang/voice`.
3. **Updating sapling pins (`refresh` and `ratify`):** Running `hale habitat nursery refresh` (also run during `hale habitat up`) inspects all trusted nurseries. Newly discovered saplings are pinned automatically; if an already-pinned sapling offers a newer revision, the habitat files it for the Board without moving existing organisms. Run `hale habitat nursery ratify <sapling>` to pin the newer revision for future grafts.

---

### Pairing an Agent with a Person (MCP)

The habitat's head serves its command surface over MCP as well as over its socket and its HTTP commands port, with the same bearer and the same roles. `hale habitat status` prints the endpoint as `mcp://127.0.0.1:<port>`. An agent's host connects with the person's ID token, the bearer their face session holds:

```sh
HALE_API_TOKEN=<the person's ID token> hale mcp --app mcp://127.0.0.1:<port>
```

The agent's tools are the head's rows that person may call (`Commands__hosts`, `Commands__add_host`, …). A row they may not call is refused `unauthorized`, exactly as the face is refused, and every act is a row in their name. `hale habitat brief` (the row `Commands__brief`) is where such an agent starts: the Board's charter, the rules it works by, and the live state. [`agents/operator.md`](https://github.com/hale-lang/hale/blob/main/agents/operator.md) is the agent's own brief: the words, the rules, and which rows read, act or propose.

## Walkthrough: Deploying and Querying `voice` in a Habitat

This walkthrough covers taking a machine with `hale`, `docker compose`, `git`, and a browser from a cold start to streaming an LLM completion response through `voice`, grafted from its own repository as a nursery.

### 1. Start the Habitat and Inspect Status

```sh
hale habitat up
hale habitat status
```

`hale habitat up` initializes the root habitat organism (if not already initialized), starts its Postgres (`memory`), NATS JetStream (`nerves`), Prometheus (`senses`), and Forgejo (`forge`) containers alongside the habitat's `body` and `face` containers under a single Docker Compose project, registers the `local` host, and records the head's URL on the profile row and in `head.url` (override on any `hale habitat` or `hale dna` verb with `--head <url>` or `HALE_DNA_HEAD_URL`). `status` prints the genome, body status, active allocations, and operator face URL:

```text
face    http://127.0.0.1:<port>/ (up, ...)
```

### 2. Sign In to the Operator Face

Open the `face` URL in your browser. On a `local` habitat, the head redirects through the local OIDC identity provider, signs you in automatically as `local-sub` (mapped to your local user for an 8-hour session with no password setup required), and displays the four screens: **habitat** (the index and its name), **hardware**, **infrastructure**, and **organisms** ([The head and the face](./dna/head.md)).

### 3. Graft the `voice` Sapling (Local or LAN Host)

Trust voice's repository as a nursery (it offers itself), then graft it onto the local machine:

```sh
hale habitat nursery add https://github.com/hale-lang/voice
hale habitat nursery list
hale habitat graft voice
```

`graft` fetches the pinned revision of `voice` into `<habitat>/saplings/voice`, verifies its SHA-256 tree digest, grafts it as a child organism of the habitat, and allocates its requested roles (`memory`, `nerves`, heart `database`, and `broker`).

To place `voice` onto a remote LAN host over SSH instead (register the host with `host add` and `host probe` first):

```sh
# Register the LAN host and append the printed habitat public key to ~/.ssh/authorized_keys on rack-a.lan:
hale habitat host add rack-a.lan --user ops

# Verify SSH connectivity and host capabilities (docker compose, OS, arch):
hale habitat host probe rack-a.lan

# Graft voice onto rack-a.lan:
hale habitat graft voice --host rack-a.lan
```

### 4. Apply Allocations and Start `voice` Services

`hale habitat apply` materializes each placed allocation on its target host via that host's provider and reports live health (`up`, `degraded`, or `down`):

```sh
# Materialize placed allocations and print health pulse:
hale habitat apply
```

Where each allocation runs is a row the Board writes: `hale habitat place <key>/<role> <host>`. `hale habitat apply` makes it so. It takes each moved allocation down where it ran and brings it up on its new host with that host's provider, then prints what each allocation looks like now (up, degraded or down). An allocation on another machine is reached at that machine's name and the port it publishes there, so the DSN and the nerves' URL follow it.

A moved memory starts empty, because nothing migrates. A memory that holds its organism's record moves only with `hale habitat apply --fresh`, and the habitat's own memory does not move at all. `hale habitat graft <sapling> --host <name>` grafts an organism straight onto a host: its memory, nerves and services run there from the start, and its record is born there.

*(To move an allocation to another host before applying: `hale habitat place <key>/<role> <host>`, then `hale habitat apply`. Moving a stateful `memory` allocation across hosts requires `hale habitat apply --fresh`, which initializes a fresh store on the destination host.)*

`voice`'s image downloads the released Hale toolchain itself when it is built (its `Dockerfile`), so the first `apply` needs the network and takes a few minutes; nothing is staged by hand.

The operator face's **organisms** screen lists each of `voice`'s allocations with what its services publish: a link where the port is reachable from your machine (`api` opens `voice`'s console, its admin UI served at the api's `/`), and `loopback only on <host>` for a port bound to that host's loopback.

`voice`'s Compose definition publishes its api (port `8080`) and its NATS (port `4222`) on the listen address of the host it is placed on (`HALE_DNA_HOST_LISTEN`), so a browser or a node on another machine reaches them at the host's name; on this machine they stay on `127.0.0.1`. Every plane of the api takes a bearer, so nothing rests on loopback: a project key on `/v1`, a node's enrollment token on `/node/v1`, and the **operator token** on `/admin/v1`. Name the api's address once (`localhost:8080` when `voice` runs on this machine, `rack-a.lan:8080` when it is placed there) and check that it answers:

```sh
api=http://rack-a.lan:8080        # or http://localhost:8080 on this machine
curl $api/healthz
```

`voice` mints its operator token on its first start and keeps it in its Postgres; it is never in the environment or a log. Print it on the host running `voice` (use `docker compose ls` there to find the compose project name; its api container is `<project>-api-1`):

```sh
ssh ops@rack-a.lan docker exec <project>-api-1 voice-api admin-token > admin.token
chmod 600 admin.token
```

The console (the api's `/`, the **organisms** screen's `api` link) asks for this token once and keeps it in the browser.

### 5. Configure a Model, Project Key, and Node Seat on `voice`

Configure a model (`echo-1`), billing account, project API key, and worker node seat via `voice`'s admin API (or via the console at `$api/`, which asks for the operator token). Every admin call presents the operator token, here from a header file so it is on no command line. Using the built-in `static` engine requires no external weights or GPU:

```sh
json='content-type: application/json'
printf 'Authorization: Bearer %s\n' "$(cat admin.token)" > admin.h
curl -X POST $api/admin/v1/models   -H @admin.h -H "$json" -d '{"id": "echo-1"}'
curl -X POST $api/admin/v1/accounts -H @admin.h -H "$json" -d '{"id": "static", "name": "Static"}'
curl -X POST $api/admin/v1/projects -H @admin.h -H "$json" -d '{"id": "work", "name": "Work"}'
curl -X POST $api/admin/v1/projects/work/keys -H @admin.h -H "$json" -d '{"name": "laptop"}'
curl -X POST $api/admin/v1/nodes    -H @admin.h -H "$json" -d '{"id": "local"}'
curl -X POST $api/admin/v1/nodes/local/seats -H @admin.h -H "$json" -d '{
  "id": "static", "engine": "static", "account": "static",
  "serves": [{"model": "echo-1"}], "config": {"text": "hello from voice"}}'
```

- Save the `"secret": "vk-..."` token returned by `/admin/v1/projects/work/keys`.
- Save the `"enrollment_token": "vn-..."` returned by `/admin/v1/nodes` and start `voice`'s node container with it (use `docker compose ls` to find the compose project name):

```sh
# on the host running voice:
VOICE_ENROLLMENT_TOKEN=vn-... docker compose -p <project> up -d node
```

*(To serve a live local LLM, replace `"engine": "static"` with an `"openai-compatible"` seat pointed at a local Ollama or vLLM endpoint.)*

### 6. Point an Organism's Model Map at `voice`

Store the `vk-...` API key in a vault slot (`hale dna secret set <SLOT>` inside `voice`'s directory), then map the `think.standard` model category to `voice` and probe it:

```sh
hale habitat model-map voice think.standard echo-1 \
  --endpoint $api/v1/chat/completions \
  --key-vault <slot>
hale habitat model-probe voice think.standard
```

### 7. Send a Streaming Completion Request

Query `voice`'s OpenAI-compatible `/v1/chat/completions` endpoint directly with `curl -N`:

```sh
curl -N $api/v1/chat/completions \
  -H 'Authorization: Bearer vk-...' \
  -H 'content-type: application/json' \
  -d '{"model": "echo-1", "stream": true, "messages": [{"role": "user", "content": "hi"}]}'
```

`voice` streams back Server-Sent Event chunks (`data: {...}` with `chat.completion.chunk`) terminated by `data: [DONE]`, and records usage under `GET /admin/v1/usage` (under the operator token).

### Operational Notes

- **Stopping and restarting:** `hale habitat status` reports any degraded or stopped allocations. `hale habitat down` stops the body, the face and every service and removes nothing; `hale habitat up` brings it back online.
- **Forgetting a habitat:** `hale habitat down --forget` removes the habitat whole, plan first, like terraform. It prints everything it would remove, grouped: the habitat's record and memory (row count, organisms, newest row), each grafted organism (record, genome repository, Forgejo repository, allocations by host), the compose projects and volumes, the vault slots (by name, never a value), the Forgejo instance and its accounts, the hosts, and what it runs on each host over SSH, each line with its size or count and whether it is recoverable. Nothing is touched until you type the habitat's name back at a terminal; there is no `--yes`, and without a terminal it refuses. `--plan` prints the plan and exits. A forget that fails part way says what was done and what was not, and running it again plans from what remains. The one thing it cannot remove is the line each host's `~/.ssh/authorized_keys` took: it is noted, and the next plan shows it until you remove it there and a forget clears the note. A host that does not answer is planned as `unreachable: left on <host>`: the habitat's side goes and what is left there is named. Slots a person supplied (a GitHub token, a `--key-vault` slot) are kept. If anything changes between the plan and your confirmation, the act refuses with `plan_changed` and you plan again.
- **Removing an organism:** `hale habitat organism remove <name>` plans and confirms the same way (its record and memory, genome repository, Forgejo repository and account, allocations by host, compose services, vault slots), typed back by the organism's name; the organisms screen of the operator face offers the same.
- **Resuming interrupted grafts:** `hale habitat graft` verifies pinned file digests before grafting and resumes cleanly from its last completed step if interrupted.
- **Model map and graft log rows:** `model-map` and `model-probe` are the head's `Commands::set_model_map` and `Commands::probe_model`; a key is named only by its vault slot. The face follows a graft's log through `Commands::graft_log`, which reads only the log of a graft the habitat itself started.
- **No model:** every face screen works with the models catalog empty. Without a model, asks and leg work become handed Tasks that a person takes and reports done as text from the face; nothing waits on a model or times out into a leg ([Workflows: with no model](./dna/workflow.md#with-no-model)).
- **SSH host key verification:** `hale habitat host probe` rejects any host whose SSH host key has changed since `host add`; a host in `pending-key` state is waiting for the habitat's public key line in `~/.ssh/authorized_keys`.
- **Agent operator brief:** `hale habitat brief` prints the Board's live context package (hosts, allocations, organisms, saplings, and pending Board items). It reads the record alone, never the vault.

---

## Multi-Record Isolation, Sharing, and Identity

### Records as the Unit of Isolation

The fundamental unit of isolation in DNA is a **record**: an append-only chain of cryptographically signed rows stored in a dedicated Postgres schema (`dna_<record-identity>`) paired with isolated NATS JetStream, secret vault, and Prometheus senses allocations. A Git repository is the genome whose `hale.toml` (`[dna] record`) pins which record it belongs to.
- Multiple people can collaborate on one record, and one person can participate in multiple records.
- Every Git clone accesses the shared record in Postgres under its assigned database role (`head` or `spine`) rather than maintaining an uncoordinated local database replica.
- Every row hashes its predecessor's digest and carries its author's signature, preserving provenance whenever rows are exported or inspected.

### Deliberate Cross-Record Publication (`connect` and `handoff`)

Two organizations never merge their private records or read each other's database schemas. Instead, cross-organization collaboration works through **deliberate publication**:
- Two records establish a Board-ratified connection (`hale dna connect`), or two firms maintain private records and agree on a third shared record.
- Selected facts (tasks or evidence receipts) are published across the connection via `hale dna handoff` as signed envelopes over NATS JetStream, carrying explicit origin, lineage, and purpose metadata.
- Sharing a record's tasks, owners, or grants never exposes either participant's private context, internal administration, or financial books.

### Enforced Visibility and Sealed Protected Evidence

Because any principal with read access to `record_rows` or `ledger_rows` can read row payloads in plaintext, view-level filtering is never used for confidentiality. Sensitive payloads—along with derived artifacts such as summaries, embeddings, prompts, and worker context hats—are stored outside the row stream as **protected evidence** (`customer` or `confidential` receipts), encrypted at rest in the record's Postgres memory under `HALE_DNA_RECEIPT_KEY`, and accessed strictly through an audited, principal-checked boundary (`hale dna receipt disclose` / `show`).

### Orthogonal Deployment and Trust Axes

Every organism independently configures three operational axes:

| Axis | Supported Options |
| --- | --- |
| **Participation** | Single individual; a team sharing one record; multiple firms collaborating via a shared record (`dna.owner`) |
| **Deployment** | Local workstation (`local`); remote server body (`remote-body`); multi-node fleet (`hale node`) |
| **Principal Source** | Explicit local UID trust (`dna.trust = local`, `dna.unix.member`); verified OpenID Connect provider (`dna.principal = oidc`) |

---

## Layered Structural Operations

As an organization evolves, capabilities routinely move across process boundaries, ownership boundaries inside an organism, or record boundaries across a habitat. Every structural refactoring operation follows three invariants:
1. **One operation = one Review:** The Board votes on the high-level operation (for example, extracting a service), while the resulting low-level graph rows serve as the execution receipt.
2. **Explicit change class:** Each operation maps to a change class that determines its required ratification authority.
3. **Pre-proposal invariant checks:** Operations validate their preconditions upfront and refuse invalid cuts with a specific diagnostic before opening a Review.

| Layer | Boundary Moved | Operations |
| --- | --- | --- |
| **1. Hale Program** | Process & thread placement (code behavior unchanged) | **Serve** (place a locus subtree behind `unix`, `http`, or `nats` RPC via its `api` surface), **Unserve** (return a remote surface in-process), **Place** (move a locus between thread pools or binaries within the same organism). |
| **2. Single Organism** | Responsibility & contract ownership within one record (`hale dna apply`) | **Open** (`open`), **Fill / Vacate** (`fill`, `vacate`), **Merge** (`merge`), **Split** (`split`), **Assign** (`assign`), **Adopt / Retire Pack** (`hale dna pack apply\|retire`), **Expose** (`expose`), **Depend** (`depend`), **Externalize** (`externalize`), **Internalize** (`internalize`). |
| **3. Multi-Record Habitat** | Ownership across separate records (parent/child organisms) | **Graft** (`hale habitat graft`), **Extract** (externalize a part in the source organism and birth a child organism from the copied subgraph with provenance), **Absorb** (internalize a child organism's contract back into its parent and retire the child), **Re-parent** (move a child organism under a new parent ceiling and Board). |

Note that **Externalize** requires only a single organism: once a capability is externalized (`externalize seed:<name>`), the organism depends solely on its `contract:<path>` regardless of whether that contract is served by a sibling organism, a third-party SaaS API, or another internal team.

---

## Authority in the Compiler Graph (`runs_under`) and Financial Books

### Principal-Typed Authority (`runs_under` Design)

While the compiler today tracks locus ownership (the Σ-tower) and reachable effect classes (`@effects`), the authority architecture extends the compiler graph with a `runs_under(locus, principal)` relation (where each locus inherits its parent's principal unless narrowed, and no locus may claim a principal its owner does not hold):

1. **Static Principal Claims:** Secret slot accesses carry the principal they require, enabling compile-time constitutional claims such as *"only code running under the `reviewer` principal may reach `FORGE_TOKEN`"* ([Claims & the law](./claims.md)).
2. **Generated Backend Policies:** Vault policies, Postgres roles, NATS account ACLs, and GitHub team permissions become deterministic projections of each principal's reachable closure rendered at placement time, rather than hand-edited configuration files.
3. **Dynamic Per-Attachment Credentials:** In the record, holding a position is an appointment row (`holds`), while a live worker process or browser session acting under that seat is a time-bounded **attachment**. Short-lived credentials are minted for the specific attachment (narrowed to the attachment's scope and revoked when the attachment ends), and every runtime effect is logged against its attachment ID.

### Replaceable Providers and Financial Discipline

### Providers are replaceable

External dependencies—LLM backends, databases, OIDC issuers, communication channels, payment rails, accounting ledgers, and compute hosts—bind through Hale interfaces and perspectives ([Perspectives](./services/perspectives.md)). Each provider capability tracks three distinct states: *implemented in code*, *available on the current account*, and *authorized by grant*. Swapping a provider preserves the organization's record, authority structure, and history.

### Financial Capture and Books (Design)

Financial operations follow four structural rules:
- **Distinct pipeline stages:** Raw source receipts -> observations -> economic events -> approved ledger postings or external acknowledgements.
- **Single book authority:** Each book is designated as capture-only, managed by the organism's own posting service, or owned by an external accounting platform.
- **Exact decimal arithmetic:** Monetary quantities use `Decimal` with an explicit currency and rounding policy—never binary floating-point.
- **Five separated budget states:** Permitted budget, reserved capacity (`spend.reserved`), observed usage, booked expense, and cash settlement (`spend.settled` / `spend.compensated`) are tracked separately so a reservation is never mistaken for an expense.

---

## Implementation Status

| Capability | Status in the Repository Today |
| --- | --- |
| **Per-repository append-only signed record** | **Built:** [Memory and the record](./dna/memory.md) |
| **Deliberate cross-record publication (`connect` & `handoff`)** | **Built:** `hale dna connect` and `hale dna handoff` ([Memory and the record](./dna/memory.md)) |
| **Sealed protected evidence (`customer`, `confidential`)** | **Built:** `hale dna receipt` encrypted in Postgres memory ([Memory and the record](./dna/memory.md)) |
| **Deployment profiles (`local`, `remote-body`)** | **Built:** `hale habitat graft --profile local\|remote-body` and `hale dna profile` ([Getting started](./dna/getting-started.md)) |
| **Root habitat organism (`hale habitat up`, `status`, `down`)** | **Built** for the `local` profile (Docker Compose `body` and `face` services); `hosted` profile is **not built** |
| **Habitat child birth & role allocation (`hale habitat graft`)** | **Built:** Allocates `[dna] needs` (`memory`, `nerves`, `senses`, heart `database`), links parent genesis, and proposes `standard` pack |
| **Habitat standard & operator browser face (`/index`, `/app/`)** | **Built:** Started automatically by `hale habitat up` ([The head and the face](./dna/head.md)) |
| **Hosts, `compose` provider, and `ssh` host registration** | **Built:** `compose` provider places on `local`; `hale habitat host add\|probe\|remove` registers LAN hosts over SSH and placing allocations on them is **built** (`hale habitat place`, `apply`, `graft --host`) |
| **An agent pairing with a person (MCP, `hale habitat brief`)** | **Built:** the head's commands over `mcp::Rpc` under the person's bearer and roles (`hale mcp --app`, the endpoint `hale habitat status` prints), every act a row in their name; `agents/operator.md` |
| **Saplings & nurseries (`hale habitat graft <sapling>`, `nursery`)** | **Built:** Digest-pinned `[[sapling]]` entries, `hale habitat nursery add\|refresh\|list\|ratify`; a repository as its own nursery (`repo = "."`), as `hale-lang/voice` is |
| **Verified OIDC principal source** | **Built:** `git config dna.principal oidc` ([The head and the face](./dna/head.md), [The skin](./dna/skin.md)) |
| **Per-client backend roles (Postgres, NATS, Vault)** | **Built:** Separate `owner`, `spine`, `head`, `reflexes`, and per-app roles ([The skin](./dna/skin.md)) |
| **Monetary grant reservations & window enforcement** | **Built:** `spend.reserved`, `spend.settled`, `spend.compensated` ([The spine](./dna/spine.md)) |
| **Serving API surfaces & locus placement** | **Built:** `api::serve` over Unix/HTTP ([API](./services/api.md)) and `placement { }` ([Concurrency & placement](./services/concurrency.md)) |
| **Opening and filling positions (`position open`, `fill`, `vacate`)** | **Built:** Board-ratified graph nodes, mandates, and `holds` edges ([Positions and `fill`](./dna/shaping.md#positions-and-fill)) |
| **Atomic structural batches (`hale dna apply`)** | **Built:** `open`, `fill`, `fill *`, `vacate`, `practice`, `law`, `idea`, `assign`, `close`, `merge`, `split`, `expose`, `depend`, `externalize`, `internalize` |
| **Adopting and retiring nursery packs (`hale dna pack`)** | **Built:** `hale dna pack list\|show\|apply\|retire` and `hale dna nursery add\|list` |
| **`runs_under` principal graph, projected ACLs, attachment tokens** | **Not built** (tracked in compiler registry design) |
| **Cross-record habitat operations (`extract`, `absorb`, `re-parent`)** | **Not built** |
| **Standard equipment libraries & financial books** | **Not built** (`standard` nursery pack is **built** and proposed at graft) |
