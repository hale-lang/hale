# Getting started

## What you need

- A Hale application that passes `hale check`.
- A git repository around it. The record is a branch of it; changes
  are commits; rollbacks are resets.
- The `hale` binary on your `PATH`.
- For a team, or for nodes: a remote. The record syncs through it.
  Alone on one machine, none is needed.

If you don't have an application yet, `hale dna new demo` makes one
with its organization, and you can skip to [the first
run](#the-first-run).

## Generate the organization

```text
$ hale dna init .
ok: 1 file(s) typechecked
wrote   vendor/dna (14 file(s) written, 0 unchanged; hale.lock pins toolchain 0.19.2, embedded dna 770b3dfaaf001331)
cut     …/chat/.hale/dna/baseline.topology (schema 1.19, shape 3c9b9327e480d349, verdict clean)
created …/chat/dna/org/purpose.hl
created …/chat/dna/org/law.hl
created …/chat/dna/org/models.hl
created …/chat/dna/org/work.hl
models  found   OPENAI_API_KEY set, no ollama on PATH
models  frontier = gpt-4o · fast = gpt-4o-mini (OPENAI_API_KEY) · desk = llama3 (ollama at 127.0.0.1:11434, not found)
models  leader, editor, agent: deep = frontier, quick = fast, private = desk · budget 25.00 USD a day (`hale dna models` probes them)
created …/chat/dna/org/main.hl
created …/chat/dna/compose.yaml
memory  dna/compose.yaml: `hale dna dev` brings its Postgres up and applies memory's schema to it (docker compose on PATH); `hale dna run` needs HALE_DNA_MEMORY_DSN_SPINE
kept    …/chat/main.hl (the application is not modified; the organization oversees it from dna/org)
edited  …/chat/hale.toml ([claims] no_base, [environments.local], [environments.org])
seeded  refs/dna/journal (7 event(s): application.attached, structure.observed, responsibility.proposed, review.requested)
```

`init` refuses a program that does not check, and it does not touch
your application. What appeared:

| what | where | yours? |
|---|---|---|
| **The purpose.** One sentence: what this codebase is for. The first review asks you to ratify it. | `dna/org/purpose.hl` | yes — edit it |
| **The charter.** The leader's brief: it is the organism's architect, it proposes and you decide, and what it must know before it plans or judges. | `dna/org/charter.hl` | yes — edit it |
| **The organization.** The Board, the Leader and its grant, the gateways. Ordinary Hale source; `hale check` validates it. | `dna/org/main.hl` | yes — edit it |
| **The catalog.** Which model each position calls, and the budget, written from what your machine had. `hale dna models` probes it. | `dna/org/models.hl` | yes — edit it |
| **The performers.** Who performs a Work in a leg of this project: a person, a program of yours for the kinds it takes, a model. `hale dna work` runs them ([Legs](./legs.md)). | `dna/org/work.hl` | yes — edit it |
| **Memory's environment.** Its Postgres — the ledger, the knowledge graph, protected evidence — for `hale dna dev` through docker compose. | `dna/compose.yaml` | yes — edit it |
| **The law.** What no position may ever do, enforced by the compiler against the wiring you actually built. Add to it; don't weaken it. | `dna/org/law.hl` | yes — extend it |
| **The toolchain's part.** The DNA itself, as the `hale` you ran carries it. Ignored by git; `hale dna upgrade` refreshes it. | `vendor/dna/` | no |
| **The record.** Everything the organization does, one commit per event, plus receipts and leases. Not files: refs in your repository. | `refs/dna/*` | no — but it's git |
| **Scratch.** Sockets, sandboxes, the toolchain's inputs, the status projection. Ignored by git; delete it and nothing is lost. | `.hale/dna/` | no |

Your `hale.toml` gained two environments — the application and the
organization are two entrypoints, each checked against its own law
— and `[claims] no_base = true`, because they deliberately share
none.

`vendor/dna` is the DNA source your `hale` binary *carries*, not the
source of any checkout, so `embedded dna 770b3dfaaf001331` on that
first line is its provenance: a version number is not one, because
two builds of `hale 0.19.2` can embed different `dna/` source. The
digest is printed by `hale --version` (second line) and by `hale dna
status`, written into `vendor/dna/README.md` and
`.hale/dna/embedded.digest`, and `hale dna --embedded-digest` prints
it alone for a script. If you work *on* the DNA, compare it with your
checkout — `hale dna --embedded-digest --from-tree <hale-repo>` — and
rebuild when they differ: until you do, every organization you
generate runs the core your binary was built with, and a test of your
edit measures the old one. `hale dna status` says
`vendor/dna was materialized from <digest> — run hale dna upgrade`
when a project's vendored tree came from another build.

Commit it, and if you have a remote, push:

```sh
git add -A && git commit -m "the organization"
git push origin main
hale dna sync
```

`hale dna sync` carries the record — the refs under `refs/dna/`, which
`git push origin main` does not touch. The host syncs on every tick
once the organization is running, so this is only for the first push,
and a clone that already shares a record needs nothing by hand.

## Check nothing broke

```text
$ hale check --matrix .
=== ./. @ local ===
ok: 1 file(s) typechecked
=== ./dna/org @ org ===
ok: 3 file(s) typechecked

ok: 2 (entrypoint, environment) pair(s) checked

$ hale test .
ok   ./tests/main_test.hl

1 passed, 0 failed
```

Your tests matter twice from here on: every change the organization
proposes is verified against them.

## The first run

On one machine, the organization and the application under one host:

```text
$ hale dna dev
built: …/dna/org/org
hale dna dev: organization (pid 1874960) from … under LOTUS_OBS=1
hale dna dev: the organization reads its facts from the nerves (DNA_4F…)
built: …/./chat
hale dna dev: expression chat (pid 1874991) under LOTUS_OBS=1
chat: 1 ping(s) echoed; the bus is open
```

Leave this terminal open; it is the host. It rebuilds and restarts
the application when a change is approved and watches it. Under
`hale dna run` only the organization runs here, and the application
is expressed wherever it lives — [Operating the fleet](./operating.md).
In another terminal:

```text
$ hale dna status
organism:   running (this clone's body holds the lease)
journal:    35 event(s), chain verified at 5f0c2e9a41d7
expression: attached Chat (shape 3c9b9327e480d349) · current shape 3c9b9327e480d349 · build 812f3c9bd9e4
intents:    0 offered, 0 refused
tasks:      none
reviews:    15 pending of 15
  purpose [pending] needs board — ratify the declared purpose?
  k:79c636063701 [pending] needs board — ratify the design practice `design/principles`: …
  … (fourteen: eight practices of the design, six operating practices)
mutations:  0 (none applies before a human's verdict on the exact candidate)
```

`hale dna ui` serves the same thing as a page — the Board's queue,
the Reviews with their three views, the fleet, the history, and the
forms — from the record alone, with or without the host up.

## The first review

There is already something to decide: the organization wants you to
ratify the purpose `init` wrote. Read `dna/org/purpose.hl`, change
the sentence to what the codebase is actually for, and approve it:

```text
$ hale dna review purpose approve --as riley --comment "ratified"
review purpose settled: approve by riley
```

Eight more Reviews wait beside it: the **design**, the toolchain's
practices about how an organization like this one works — how it
grows, when a position is justified, what the signals mean. Each is
its own Review, because a Review pins one thing and settles with one
answer; `hale dna review` lists them under one heading per family —
`design`, how the organization is shaped, and `operating`, how the
organism runs — and you can decide them one by one or a family at
once:

```text
$ hale dna review design approve --as riley
review k:1e0855febc99 settled: approve by riley
…
```

Read them first. Nothing the toolchain proposes is ratified until you
say so, and a practice you decline never reaches anyone's package.

That is the whole review mechanism, on the smallest possible thing.
`--as` is your name on the record; a verdict from the terminal
carries the Board's authority unless you say otherwise. Every change
from now on goes through the same door: something is proposed,
someone with the authority is asked, they answer with their name on
it, and the answer is a commit.

Next: [Working with it](./working.md).
