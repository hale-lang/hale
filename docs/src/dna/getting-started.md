# Getting Started with DNA

This guide walks through provisioning a new DNA organism, configuring its local infrastructure in the habitat, populating secret slots in the vault, starting the local supervisor (`hale dna dev`), ratifying the initial Board reviews, and submitting your first task.

```sh
hale habitat up
hale habitat graft demo
cd demo
hale dna dev
```

## Prerequisites

Before creating an organism, verify that your machine has:

- **`hale` on `PATH`**: The Hale compiler and DNA CLI.
- **`git`**: Candidate branches and base commits live in your Git repository. `hale habitat graft` initializes a repository automatically if none exists.
- **`docker compose` on `PATH`**: Runs the organism's private backing services—PostgreSQL (`memory`), NATS JetStream (`nerves`), and Prometheus (`senses`)—inside your machine's local habitat.
- **An LLM backend (optional but recommended)**: By default, the habitat's own models: graft [voice](https://github.com/hale-lang/voice) into the habitat and every organism's model map is pointed at it, each with a key the habitat draws, so nobody mints one ([The Habitat](../habitat.md#the-habitats-models)). Otherwise, a hosted API key already in the vault when you graft (Anthropic or OpenAI), a local `ollama` server on `PATH` (`127.0.0.1:11434`), or a coding CLI harness (`claude` or `codex`) on `PATH`. Without a model, the organism still runs, but the Leader cannot plan tasks or decide delegated reviews, so every review waits for human Board approval.
- **A Git remote (optional)**: Required only when sharing a record across teammates or deploying to remote fleet nodes.

## Provisioning an Organism

Every organism on a machine is provisioned as a child of the local **habitat** ([The Habitat](../habitat.md)), which manages shared Docker Compose infrastructure. Run `hale habitat up` to initialize and start the habitat, then run `hale habitat graft <name>` to create a new application and its DNA organization in a fresh directory:

```text
$ hale habitat up
$ hale habitat graft demo
ok: 1 file(s) typechecked
created demo/hale.toml
created demo/main.hl
created demo/tests/main_test.hl
created demo/.gitignore
wrote   vendor/dna (81 file(s) written, 0 unchanged; hale.lock pins toolchain 0.21.0, embedded dna ca4012a0cda7226d)
cut     …/demo/.hale/dna/baseline.topology (schema 1.21, shape ddd87794e8adeed5, verdict clean)
created …/demo/dna/org/purpose.hl
created …/demo/dna/org/charter.hl
created …/demo/dna/org/law.hl
created …/demo/dna/org/models.hl
models  found   no model key in the vault (`hale dna secret set <NAME>`), no ollama on PATH, claude on PATH
models  no hosted backend (no model key in the vault) · desk = llama3 (ollama at 127.0.0.1:11434, not found)
models  editor, agent, leader: quick = harness (claude), deep = harness (claude) · private = desk · budget 25.00 USD a day (`hale dna models` probes them)
models  the habitat's models fill the model map: a models provider it grafted (voice) is every tenant's model backend (`hale habitat models <organism>`)
created …/demo/dna/org/work.hl
created …/demo/dna/org/workflows.hl
created …/demo/dna/org/own_workflows.hl
created …/demo/dna/org/main.hl
memory  the local habitat's allocations: `hale habitat migrate …/demo` brings this organism's memory, nerves and senses' store up and migrates them (docker compose on PATH), and `hale dna dev` runs on them; `hale dna run` needs HALE_DNA_MEMORY_DSN_SPINE, HALE_DNA_NATS_URL_SPINE and HALE_DNA_NATS_ORG
kept    …/demo/main.hl (the application is not modified; the organization oversees it from dna/org)
edited  …/demo/hale.toml ([claims] no_base, [environments.local], [environments.org])
pinned  …/demo/hale.toml ([dna] record = 4f1c9a07d2be…, the identity of the record born here)
memory  schema applied; the record's tables, roles and functions are in place
born    record 4f1c9a07d2be (record.genesis; record.key 8a3e51c0b7d4…)
seeded  the record (6 observed event(s): application.attached, structure.observed, responsibility.proposed; then the purpose proposed and the application's graph)
graph   2 node(s), 0 edge(s): 1 purpose, 0 axiom, 0 process, 1 seed, 0 contract, 0 noun, 0 deployment, 0 gate, 0 document, 0 witness, 0 position, 0 work, 0 organization, 0 definition; 6 proposal(s) (graph.node, graph.edge)
seated  the head's socket knows uid 1000 as … (dna.unix.member); the record declares dna.trust = local, where they hold every position
seeded  purpose (proposed for the Board: `hale dna review` lists it under `purpose`)
seeded  design (11 practice(s) proposed, one Board Review each: `hale dna review` lists them under `design`)
seeded  operating (7 practice(s) proposed, one Board Review each: `hale dna review` lists them under `operating`)
seeded  using (7 practice(s) proposed, one Board Review each: `hale dna review` lists them under `using`)
memory  its allocation's superuser password in the vault, and its file beside the allocation in the local habitat (mode 600)
nerves  every role's password drawn into the vault, and the nerves allocation's secrets written (mode 600)
secret  oidc-client-dna-local drawn into the vault
secret  forge-token: a slot in the vault for a person to fill (`hale dna secret set`)
nerves  the organization's stream created

next steps:
    hale check --matrix …/demo   # every entrypoint against its law
    hale dna dev …/demo          # the organization and the application under one host, iris attached
    the first Review (`purpose`) ratifies the declared purpose (dna/org/purpose.hl): `hale dna review purpose approve --as <you>`
```

Key details from the `graft` output:

- **Model probing (`models`)**: `graft` inspects your machine's vault and `PATH` to generate `dna/org/models.hl`. In the output above, `claude` was found on `PATH` and no API key was present in the vault, so `quick` and `deep` tiers default to the `claude` harness and no hosted backend is written at all: there is no placeholder key to fill. When the habitat holds a models provider (voice), the graft also points the organism's model map at it (`models  demo's model map points at the habitat's models …`), and a Work the map resolves is asked there with the organism's own key.
- **Local identity (`seated`)**: Local Git config (`dna.unix.member`) maps your OS UID to your username, and `dna.trust = local` grants that local user authority over every organizational position.
- **Remote body profile**: Grafting a new application refuses an existing target directory. Pass `--profile remote-body --remote <url> [--body <user@host>]` to configure a shared remote record and remote host body instead (see [The Heart and the Body](./heart.md)).

### Attaching to an Existing Application

To add DNA governance and runtime supervision to an existing Hale repository, run `hale habitat graft` with `--attach`:

```sh
hale habitat graft . --attach
```

`--attach` performs the following steps:

1. Initializes a Git repository if one does not exist and writes `vendor/dna`.
2. Runs `hale check` to extract the application's baseline topology. If the existing application fails typechecking, `graft` aborts before writing any `dna/` files.
3. Leaves your application source files untouched (`kept …`), adds the `local` and `org` environments to `hale.toml`, and runs `hale fmt` across `dna/` and your application seed.
4. Inspects the repository graph (via `hale check --api` when an application seed is present at the root, or the repository graph otherwise). The application becomes the root `seed` node, and every HTTP/RPC API surface or hub stream it serves becomes a `contract` node.
5. Proposes **holes** (`hale dna review` group `holes`) for the organizational positions implied by the graph (`board`, plus the application's `dev` and `reviewer` positions) along with each position's **mandate** (a policy text defining what the position decides, what it is forbidden from deciding, what it must cite, how it writes, and what it escalates).
6. Computes each position's standard **equipment** from contract role annotations (`requires: [editor]` in an API surface or `spec/<Stem>.description.json` generated by `hale api describe`), assigning `reviewer` to the seed's reviewer position and all other required roles to its `dev` position (`hale dna fill` displays these roles). Equipment roles describe what tools a position holder is fitted with; they never create positions or grant authority on their own.

## Generated Files & Habitat Allocations

`hale habitat graft` creates the following files and database structures:

| Path / Resource | Purpose | User-Editable? |
| --- | --- | --- |
| `dna/org/purpose.hl` | Single-sentence declaration of the codebase's purpose, ratified in the first Board Review. | Yes |
| `dna/org/charter.hl` | The Leader's architectural brief, read alongside the purpose, law, and ratified design practices during planning and review. | Yes |
| `dna/org/main.hl` | Entrypoint defining the Board, Leader, `AutonomyBoundary` grant, and `dna::Dna` substrate. | Yes |
| `dna/org/law.hl` | Compile-time architectural claims enforced by `hale check`. You may add stricter claims, but do not weaken baseline rules. | Yes (extend only) |
| `dna/org/models.hl` | Model catalog, tier routing, and daily spend budget generated from detected local tools. | Yes |
| `dna/org/work.hl` | Worker performer definitions executed by legs (`hale dna work`). | Yes |
| `dna/org/workflows.hl` | Baseline workflow catalog; overwritten whenever you run `hale dna upgrade`. | No |
| `dna/org/own_workflows.hl` | Custom workflow definitions; preserved across `hale dna upgrade`. | Yes |
| `vendor/dna` | Vendored DNA core library matching the `hale` binary digest (`embedded dna ca4012a0cda7226d`). Git-ignored; refreshed by `hale dna upgrade`. | No |
| `.hale/dna/` | Ephemeral runtime state: baseline topology artifacts, Unix sockets, sandbox worktrees, logs, and `.hale/dna/memory.env`. Git-ignored; safe to delete. | No |
| `record_rows` (`dna_<identity>`) | Append-only PostgreSQL table storing signed record rows and receipts. | No |

`hale.toml` is updated with two environments (`[environments.local]` for the application and `[environments.org]` for `dna/org`), `[claims] no_base = true` so each entrypoint is checked against its own architectural claims, and `[dna] record = "<identity>"` pinning the record's genesis digest. See [DNA, the Building Block](./dna.md) for the full contents of each generated file.

### Local Habitat Services

No Docker Compose files are added to your repository checkout. Instead, PostgreSQL (`memory-<id>`), NATS JetStream (`nerves-<id>`), and Prometheus (`senses-<id>`) run as **allocations** inside your machine's local habitat Compose project (`hale-habitat-local-<h>`) under `${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local/allocations/<id>/`, where `<id>` is the 12-character prefix of `[dna] record`.

Because allocations are keyed by record identity:
- Multiple clones or Git worktrees of the same organism share the same PostgreSQL database, NATS stream, and Prometheus server.
- Stopping `hale dna dev` leaves the backing containers running in the habitat.
- The habitat also runs one Forgejo, its forge: the graft made your organism a user there and a repository for its code, and `hale dna forge sync` sends the code out through it and brings the reviews on it back ([The forge mirror](./head.md#the-forge-mirror-hale-dna-forge-sync)).
- After updating the `hale` toolchain, run `hale habitat migrate` to idempotently update schemas and bring up any new habitat services (including `senses-<id>`):

```sh
hale habitat migrate .
```

Commit the generated project files (Git automatically ignores `vendor/dna` and `.hale/`):

```sh
cd demo
git add -A && git commit -m "the organism"
```

## Verifying the Workspace

Use `hale check --matrix` to verify both the application entrypoint and `dna/org` against their respective laws, then run the test suite:

```text
$ hale check --matrix .
=== ./. @ local ===
ok: 1 file(s) typechecked
=== ./dna/org @ org ===
ok: 8 file(s) typechecked

ok: 2 (entrypoint, environment) pair(s) checked
$ hale test .
ok   ./tests/main_test.hl

1 passed, 0 failed
```

Every candidate mutation proposed by the organism will automatically run `hale check`, `hale verify`, and `hale test` before opening a Review.

## Managing Secrets in the Vault

`hale habitat graft` automatically generates random passwords for internal PostgreSQL and NATS roles and creates empty **slots** for user-supplied API keys. Secrets live in `HALE_VAULT_DIR` (or `~/.cache/hale/vault` by default) as mode `0600` files.

Run `hale dna secrets` to check which secrets are present or missing (values are never printed):

```text
$ hale dna secrets
the organism's secrets (the local vault, …):
  present  postgres-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0_spine  memory: the spine's role
  present  postgres-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0_head  memory: the head's role
  present  postgres-owner-db39b9dd4369  memory: its allocation's superuser in the local habitat
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-owner  the nerves: the owner's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-spine  the nerves: the spine's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-head  the nerves: the head's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-reflexes  the nerves: the reflexes's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-app-demo  the nerves: the application `demo`'s account
  present  oidc-client-dna-local  the skin: the head's OIDC client secret (the local stub's)
  MISSING  forge-token  the forge's token — a person supplies it: `hale dna secret set FORGE_TOKEN`
1 missing
```

No model key is listed: the habitat's models need none from you. Once the habitat has keyed the organism, its slot shows up as `present  model-HABITAT_<KEY>  the model map's key for think.quick, … (the habitat's models)`.

To populate a user-supplied slot (`FORGE_TOKEN`, `OIDC_CLIENT_SECRET`, or a model key a hosted backend in `dna/org/models.hl` names), run `hale dna secret set <NAME>` and enter the secret on stdin:

```text
$ hale dna secret set FORGE_TOKEN
value for FORGE_TOKEN (this machine), on one line:
secret set: FORGE_TOKEN is in its slot of the vault, …/forge-token (secret.rotated FORGE_TOKEN; the value is nowhere in the record). A part reads it from the vault where it uses it
```

> [!IMPORTANT]
> DNA never reads model API keys from shell environment variables (such as `export OPENAI_API_KEY=…`). A model key must be written into its vault slot via `hale dna secret set`. `FORGE_TOKEN` is only required when mirroring Reviews to GitHub pull requests and can be left unset for local development. See [The Skin](./skin.md).

## Starting the Runtime (`hale dna dev`)

Start the organization and your application together under the local host supervisor:

```sh
hale dna dev
```

Leave `hale dna dev` running in its terminal. On startup, `hale dna dev` executes five steps in order:

1. **Resolves spine credentials** for the organism's PostgreSQL (`memory`) and NATS (`nerves`) services in the local habitat. `hale dna dev` does not start containers or run schema migrations itself—if the habitat services are down or unmigrated, it exits and instructs you to run `hale habitat migrate`.
2. **Acquires the body lease** in PostgreSQL so this checkout is the single active host supervising the record.
3. **Compiles and launches `dna/org`** with the `spine` credentials and waits up to 20 seconds for it to subscribe to the NATS stream.
4. **Compiles and launches your application**, attaching the Iris inspector on port `8787`.
5. **Enters the 1-second tick loop**: syncs record state, relays unanswered request rows onto NATS JetStream, and rebuilds/restarts the application whenever an approved mutation is applied.

```text
hale dna dev: body lease taken as … (token 1)
hale dna dev: memory: the graph, the ledger and protected evidence under the record's spine role
hale dna dev: organization (pid …) from … under LOTUS_OBS=1
hale dna dev: the organization reads its facts from the nerves (…)
hale dna dev: expression demo (pid …) under LOTUS_OBS=1
```

If no model credential is present in the vault, the host logs a warning (`hale dna dev: no credential for the model: …`) and displays a notice on `hale dna board` until the slot is filled.

Useful flags for `hale dna dev`:
- `--observe <secs>`: Duration of the post-restart health observation window (default: `15` seconds).
- `--port <N>`: Port for the Iris inspector (default: `8787`).
- `--no-iris`: Disables the Iris inspector.
- Stop the supervisor at any time with `Ctrl-C`; all pending Reviews and tasks remain durable in PostgreSQL.

## Inspecting Runtime Status

In a second terminal, run `hale dna status`:

```text
$ hale dna status
embedded dna: ca4012a0cda7226d (hale 0.21.0)
organism:   not running — reading the Journal
journal:    38 event(s), chain verified at 9ddd807957fc
expression: attached Demo (shape ddd87794e8adeed5) · current shape not cut · build not built
intents:    0 offered, 0 refused
tasks:      none
reviews:    32 pending of 32
  k:05cecb18af4e [pending] needs board — ratify the design practice `design/software-delivery`: For an appendage or a product: process boundaries first (what runs, fail…
  …
mutations:  0 (none applies before a human's verdict on the exact candidate)
profile:    local (detected)
body:       none (no body has run this record; `hale dna run` takes the lease)
memory:     record + ledger (operations and leases live in memory)
genome:     no node has recorded the genome it runs
```

Read-only inspection commands (`hale dna status`, `hale dna history`, `hale dna review`, and `hale dna board`) query PostgreSQL directly using the `head` DSN stored in `.hale/dna/memory.env` and work even when `hale dna dev` is stopped. When `hale dna dev` is running, the `organism:` and `body:` lines display the active holder and lease status.

## Ratifying Initial Board Reviews

`hale habitat graft` seeds `dna/org/purpose.hl` and proposes initial governance items for the Board (you) to ratify:

```hale,fragment
const PURPOSE: String = "refproj: keep the application correct, reviewable and explainable; every change is staged, reviewed, and never applied by the organism itself.";
```

Run `hale dna review` to list all pending Reviews grouped by category:

```text
$ hale dna review
32 pending review(s) of 32
  purpose — the declared purpose, the Board's to ratify first:
      k:d2564afcfe0d — ratify the declared purpose?
      decide one with `hale dna review <id> …`, or all pending with `hale dna review purpose approve|reject`
  design — 11 seeded practice(s), each its own Review:
      k:05cecb18af4e — ratify the design practice `design/software-delivery`: For an appendage or a product: process boundaries first (what runs, fail…
      …
  operating — 7 seeded practice(s), each its own Review:
      k:33be310ddba8 — ratify the operating practice `operating/one-store-per-step`: a workflow step writes to exactly one store, by that store's one writer,…
      …
  using — 7 seeded practice(s), each its own Review:
      k:8c1f6a4e2d70 — ratify the using practice `using/propose-review-ratify`: nothing is in force until the Board says so. A change to the organi…
      …
  holes — 6 hole(s) the repository leaves, each its own Review:
      k:d51a712f169e — ratify the position `board` under the purpose: it ratifies what this organization becomes
      …
      decide one with `hale dna review <id> …`, or all pending with `hale dna review holes approve|reject`
render one with `hale dna review <id>`; decide with `hale dna review <id> approve|revise|reject|abstain`
```

With `hale dna dev` running, ratify the declared purpose and the seeded practice families (`design`, `operating`, and `using`):

```sh
hale dna review purpose approve --as alice
hale dna review design approve --as alice
hale dna review operating approve --as alice
hale dna review using approve --as alice
```

Each command appends a `review.verdict` row signed by `--as <name>` and waits for the running spine node to settle the Review (`review k:… settled: approve by alice`). If no body holds the record lease, `hale dna review` refuses to write unless you pass `--no-wait` to queue the verdict for the next run.

To ratify every pending group—including `holes`—in one command, run `hale dna review all approve --as alice`. To also assign yourself to every position opened by `holes` (`board`, `<app>/dev`, and `<app>/reviewer`) during initial setup, apply the `founder` pack when grafting:

```sh
hale habitat graft . --attach --with founder --set founder=alice --as alice
hale dna review all approve --as alice      # with `hale dna dev` running
```

See [A Pack at the Graft](./shaping.md#a-pack-at-the-graft) for details on how packs stage proposals against open position holes.

## Submitting Your First Task

With `hale dna dev` running, request a code change from the terminal:

```sh
hale dna task create --as alice "document the Echo locus in main.hl"
```

The CLI writes the request row, waits for the spine to admit the workflow, and prints `task t1 born for intent i… […]`. Continue to [One Task, End to End](./workflow.md) to trace how this task executes, verifies, reviews, and deploys.
