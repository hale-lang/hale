# Getting started

This chapter makes an organism, shows what it wrote, fills its vault,
starts it on your machine, and walks you through the first Review and
the first task.

## What you need

- The `hale` binary on your `PATH`.
- git. The record lives in your repository, so `new` and `init` make
  one when there is none.
- `docker compose` on your `PATH`. `hale dna dev` brings up the
  organism's private services with it: Postgres for memory, NATS for
  the nerves, Prometheus for the senses.
- A model, for the organism to do work: a model key you put in the
  vault, `ollama` on your `PATH`, or a coding harness (`claude`, else
  `codex`) on your `PATH`. Without one the organism still runs; the
  Leader cannot decide anything, and every Review waits for you.
- For a team, or for nodes: a git remote. Alone on one machine, none
  is needed.

## Make one

`hale dna new <name>` makes a greenfield application with its organism
in a new directory:

```text
$ hale dna new demo
ok: 1 file(s) typechecked
created demo/hale.toml
created demo/main.hl
created demo/tests/main_test.hl
created demo/.gitignore
wrote   vendor/dna (81 file(s) written, 0 unchanged; hale.lock pins toolchain 0.21.0, embedded dna ca4012a0cda7226d)
cut     …/demo/.hale/dna/baseline.topology (schema 1.19, shape ddd87794e8adeed5, verdict clean)
created …/demo/dna/org/purpose.hl
created …/demo/dna/org/charter.hl
created …/demo/dna/org/law.hl
created …/demo/dna/org/models.hl
models  found   no model key in the vault (`hale dna secret set <NAME>`), no ollama on PATH, claude on PATH
models  frontier = gpt-4o · fast = gpt-4o-mini (OPENAI_API_KEY, not in the vault: hosted backends are not permitted until it is) · desk = llama3 (ollama at 127.0.0.1:11434, not found)
models  editor, agent, leader: quick = harness (claude), deep = harness (claude) · private = desk · budget 25.00 USD a day (`hale dna models` probes them)
created …/demo/dna/org/work.hl
created …/demo/dna/org/workflows.hl
created …/demo/dna/org/own_workflows.hl
created …/demo/dna/org/main.hl
created …/demo/dna/compose.yaml
memory  dna/compose.yaml: `hale dna dev` brings its Postgres and NATS up, applies memory's schema and creates the nerves' stream (docker compose on PATH); `hale dna run` needs HALE_DNA_MEMORY_DSN_SPINE, HALE_DNA_NATS_URL_SPINE and HALE_DNA_NATS_ORG
created …/demo/dna/senses.yml
kept    …/demo/main.hl (the application is not modified; the organization oversees it from dna/org)
edited  …/demo/hale.toml ([claims] no_base, [environments.local], [environments.org])
seeded  refs/dna/journal (8 event(s): application.attached, structure.observed, responsibility.proposed, the purpose proposed)
seated  the head's socket knows uid 1000 as … (dna.unix.member); the record declares dna.trust = local, where they hold every position
seeded  purpose (proposed for the Board: `hale dna review` lists it under `purpose`)
seeded  design (8 practice(s) proposed, one Board Review each: `hale dna review` lists them under `design`)
seeded  operating (7 practice(s) proposed, one Board Review each: `hale dna review` lists them under `operating`)
seeded  using (7 practice(s) proposed, one Board Review each: `hale dna review` lists them under `using`)
edited  …/demo/.gitignore (/dna/nats.secrets.conf, /dna/postgres.secrets)
memory  the compose database's superuser password in the vault; dna/postgres.secrets written from it (mode 600, untracked)
nerves  every role's password drawn into the vault; dna/nats.secrets.conf written (mode 600, untracked)
secret  oidc-client-dna-local drawn into the vault
secret  forge-token: a slot in the vault for a person to fill (`hale dna secret set`)
secret  model-OPENAI_API_KEY: a slot in the vault for a person to fill (`hale dna secret set`)

next steps:
    hale check --matrix …/demo   # every entrypoint against its law
    hale dna dev …/demo          # the organization and the application under one host, iris attached
    the first Review (`purpose`) ratifies the declared purpose (dna/org/purpose.hl): `hale dna review purpose approve --as <you>`
```

The `models` lines depend on your machine. This one had `claude` on
its `PATH` and no key in its vault, so the harness took every tier and
the hosted backends wait for their key. The `seated` line names your
login: the record's local config maps your uid to you, and
`dna.trust = local` says this is one person's record, in which you hold
every position.

`new` refuses a directory that is not empty. `--profile remote-body
--remote <url> [--body <user@host>]` starts it on a shared record and
a remote body instead; [The heart and the body](./heart.md) covers
bodies.

### Or attach one to an application you have

```sh
hale dna init .
```

`init` prints the same kind of lines. It makes a git repository when
there is none, writes `vendor/dna`, and then cuts your application's
structure with `hale check`: an application that does not check stops
it there, before any of the organization is written, so fix the
application first. It keeps your application's files (`kept … (the
application is not modified …)`), adds the two environments to your
`hale.toml` (or writes one), and ends by running `hale fmt` over
`dna/` and your application's seed. On a directory with no application
at its root, `init` seeds the record with the repository's graph
instead.

## What it wrote

| what | where | yours? |
| --- | --- | --- |
| The purpose: one sentence, what this codebase is for. The first Review asks you to ratify it. | `dna/org/purpose.hl` | yes |
| The charter: the Leader's brief, which it reads with the purpose, the law and the ratified design before it plans an ask or decides a Review. | `dna/org/charter.hl` | yes |
| The organization: the Board, the Leader and its grant, the substrate. Ordinary Hale source. | `dna/org/main.hl` | yes |
| The law: what no position may do, checked by the compiler against the wiring you built. Add to it; do not weaken it. | `dna/org/law.hl` | yes, to extend |
| The model catalog and the budget, written from what this machine had. | `dna/org/models.hl` | yes |
| The performers a leg runs. | `dna/org/work.hl` | yes |
| The workflow catalog: the baseline, rewritten by every `hale dna upgrade`. | `dna/org/workflows.hl` | no |
| Your own workflow definitions, never rewritten. | `dna/org/own_workflows.hl` | yes |
| The private services for `hale dna dev`: Postgres, NATS and Prometheus, each on `127.0.0.1`. | `dna/compose.yaml` | yes |
| The nerves' server config: one user per part, each allowed only its own subjects. No password in it. | `dna/nats.conf` | yes; `upgrade` regenerates it |
| What the senses' store scrapes. | `dna/senses.yml` | yes |
| The two files that carry passwords to the compose servers, written from the vault. Mode 600, ignored by git. | `dna/postgres.secrets`, `dna/nats.secrets.conf` | never commit |
| The core, as the `hale` you ran carries it. Ignored by git; `hale dna upgrade` refreshes it. | `vendor/dna` | no |
| Scratch: the baseline artifact, sockets, worktrees, logs. Ignored by git; deleting it loses nothing. | `.hale/dna/` | no |
| The record: every row as a commit, plus receipts. Refs in your repository, not files. | `refs/dna/*` | no, but it is git |

`hale.toml` gained two environments, one for the application and one
for the organization, and `[claims] no_base = true`, because the two
entrypoints are checked against their own law and share none. The
[DNA chapter](./dna.md) quotes every generated file.

`embedded dna ca4012a0cda7226d` on the `wrote` line is the provenance
of `vendor/dna`: the digest of the DNA source this `hale` binary
carries. `hale dna status` prints it on its first line.

Commit what `new` wrote (the ignored files stay out):

```sh
cd demo
git add -A && git commit -m "the organism"
```

## Check nothing broke

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

Your tests matter twice from here on: every change the organism
proposes is tested against them before anyone is asked.

## Fill the vault

`new` provisioned every secret the organism owns and left a slot for
each one a person supplies. The vault is a local directory,
`HALE_VAULT_DIR`, else one under the toolchain's cache; each entry is
a file of mode 600. `hale dna secrets` lists what the organism
requires and whether the vault holds it, never a value:

```text
$ hale dna secrets
the organism's secrets (the local vault, …):
  present  postgres-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0_spine  memory: the spine's role
  present  postgres-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0_head  memory: the head's role
  present  postgres-owner-demo  memory: the compose database's superuser (dna/postgres.secrets)
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-owner  the nerves: the owner's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-spine  the nerves: the spine's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-head  the nerves: the head's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-reflexes  the nerves: the reflexes's account
  present  nats-dna_db39b9dd436962c99cfa90fe5366f10a48f71dc0-app-demo  the nerves: the application `demo`'s account
  present  oidc-client-dna-local  the skin: the head's OIDC client secret (the local stub's)
  MISSING  forge-token  the forge's token — a person supplies it: `hale dna secret set FORGE_TOKEN`
  MISSING  model-OPENAI_API_KEY  the model's key OPENAI_API_KEY — a person supplies it: `hale dna secret set OPENAI_API_KEY`
2 missing
```

The long names carry the record's identity, so two organisms on one
machine never share an entry.

A model key goes into its slot with `hale dna secret set`. The value
comes from stdin, never the command line, and the record gets only the
name:

```text
$ hale dna secret set OPENAI_API_KEY
value for OPENAI_API_KEY (this machine), on one line:
secret set: OPENAI_API_KEY is in its slot of the vault, …/model-OPENAI_API_KEY (secret.rotated OPENAI_API_KEY; the value is nowhere in the record). A part reads it from the vault where it uses it
```

A name the organism has no slot for is refused. The slots are
`FORGE_TOKEN`, `OIDC_CLIENT_SECRET`, and each key your model catalog
names. The environment is not read for a model key: a key exported in
your shell does nothing until it is in the vault.

The forge's token is only needed when GitHub mirrors your Reviews.
Leave it missing for now. [The skin](./skin.md) covers the vault,
rotation, and setting a secret on a remote body.

## Start it

On one machine, the organization and the application run under one
host:

```sh
hale dna dev
```

Leave that terminal open; it is the host. In order, `dev`:

1. brings up `dna/compose.yaml` with docker compose, applies memory's
   schema as the database's owner, creates the nerves' stream as the
   broker's owner, and brings up the senses' store;
2. takes the body lease, so this clone is the one body running the
   record;
3. builds and starts the organization (`dna/org`), handing it only the
   spine's credentials, and waits for it to read the nerves;
4. builds and starts your application, and attaches iris on port 8787;
5. then ticks: syncs the record, relays new rows onto the nerves, and
   rebuilds and restarts the application when a change is applied.

What it prints comes from the host, one line per event, for example:

```text
hale dna dev: body lease taken as … (token 1)
hale dna dev: memory: the graph, the ledger and protected evidence under the record's spine role
hale dna dev: organization (pid …) from … under LOTUS_OBS=1
hale dna dev: the organization reads its facts from the nerves (…)
hale dna dev: expression demo (pid …) under LOTUS_OBS=1
```

With no model key in the vault it says so, and the board says so too,
until one is:

```text
hale dna dev: no credential for the model: none of OPENAI_API_KEY is in its slot of the vault (`hale dna secret set <NAME>`); the board says so until one is
```

`--observe <secs>` sets the observation window after a restart
(15 seconds by default), `--port N` moves iris, and `--no-iris` leaves
it out. `hale dna dev` refuses to start when the vault lacks a secret
the organism owns; `hale dna upgrade` draws it again.
[The heart and the body](./heart.md) has `dev`, `run` and the fleet in
full.

Stop the host with Ctrl-C. A Review left pending is still pending next
time.

## Look at it

From another terminal:

```text
$ hale dna status
embedded dna: ca4012a0cda7226d (hale 0.21.0)
organism:   not running — reading the Journal
journal:    38 event(s), chain verified at 9ddd807957fc
expression: attached Demo (shape ddd87794e8adeed5) · current shape not cut · build not built
intents:    0 offered, 0 refused
tasks:      none
reviews:    23 pending of 23
  k:05cecb18af4e [pending] needs board — ratify the design practice `design/software-delivery`: For an appendage or a product: process boundaries first (what runs, fail…
  …
mutations:  0 (none applies before a human's verdict on the exact candidate)
profile:    local (detected)
body:       none (no body has run this record; `hale dna run` takes the lease)
memory:     the record alone (routing 0); `hale dna ledger adopt` moves the day's work to the ledger
genome:     no node has recorded the genome it runs
```

That is the status before the first start; with `dev` up, the
`organism:` and `body:` lines name the running body. `status` works
offline, from the record alone, as do `history`, `review` and `board`.

The `memory:` line matters. A new organism keeps every row in the
record, one git commit each. With `dev` up, `hale dna ledger adopt`
moves the day's work into memory, where the same rows land in a
fraction of the time; [Memory and the record](./memory.md) explains the
move.

## The first Review

There is already something to decide. `new` declared a purpose and
proposed it to the Board, which is you:

```hale,fragment
const PURPOSE: String = "refproj: keep the application correct, reviewable and explainable; every change is staged, reviewed, and never applied by the organism itself.";
```

That is `dna/org/purpose.hl` in a project named `refproj`. The Review
pins the text `new` declared by its digest, and your verdict ratifies
those words. `hale dna review` lists every pending Review, grouped:

```text
$ hale dna review
23 pending review(s) of 23
  purpose — the declared purpose, the Board's to ratify first:
      k:d2564afcfe0d — ratify the declared purpose?
      decide one with `hale dna review <id> …`, or all pending with `hale dna review purpose approve|reject`
  design — 8 seeded practice(s), each its own Review:
      k:05cecb18af4e — ratify the design practice `design/software-delivery`: For an appendage or a product: process boundaries first (what runs, fail…
      …
  operating — 7 seeded practice(s), each its own Review:
      k:33be310ddba8 — ratify the operating practice `operating/one-store-per-step`: a workflow step writes to exactly one store, by that store's one writer,…
      …
  using — 7 seeded practice(s), each its own Review:
      k:8c1f6a4e2d70 — ratify the using practice `using/propose-review-ratify`: nothing is in force until the Board says so. A change to the organi…
      …
render one with `hale dna review <id>`; decide with `hale dna review <id> approve|revise|reject|abstain`
```

With `dev` running, answer it:

```sh
hale dna review purpose approve --as alice
```

The command writes your verdict as a row and waits for the Review's
answer, `review k:… settled: approve by alice`. `--as` is your name on
the record; a verdict from the terminal carries the Board's authority.
Without an organism running here, and no remote to reach one through,
the verdict is refused and nothing is written.

The other twenty-two are the toolchain's practices: `design`, how an
organization like this one is shaped, `operating`, how the organism
runs, and `using`, how to work with it. Each is its own Review. Read them, then decide them one
by one, or a family at once:

```sh
hale dna review design approve --as alice
hale dna review operating approve --as alice
hale dna review using approve --as alice
```

Nothing the toolchain proposes is in force until you ratify it. Every
change from here on goes through the same door: something is proposed,
someone with the authority answers with their name on it, and the
answer is a row. [The head and the face](./head.md) has Reviews in
depth.

## The first task

Ask for an outcome in a sentence, naming the file when you know it:

```sh
hale dna task create --as alice "document the Echo locus in main.hl"
```

The organism answers with the Task it made,
`task t1 born for intent i… […]`. What happens next, row by row, is
[One task, end to end](./workflow.md).
