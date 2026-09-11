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
wrote   vendor/dna (14 file(s) written, 0 unchanged; hale.lock pins toolchain 0.19.2)
cut     …/chat/.hale/dna/baseline.topology (schema 1.19, shape 3c9b9327e480d349, verdict clean)
created …/chat/dna/org/purpose.hl
created …/chat/dna/org/law.hl
created …/chat/dna/org/models.hl
models  found   OPENAI_API_KEY set, no ollama on PATH
models  frontier = gpt-4o · fast = gpt-4o-mini (OPENAI_API_KEY) · desk = llama3 (ollama at 127.0.0.1:11434, not found)
models  leader, editor, agent: deep = frontier, quick = fast, private = desk · budget 25.00 USD a day (`hale dna models` probes them)
created …/chat/dna/org/main.hl
created …/chat/dna/compose.yaml
knowledge dna/compose.yaml: `hale dna dev` brings its Postgres up and runs the knowledge service against it (docker compose on PATH); `hale dna run` needs HALE_DNA_KNOWLEDGE_DSN
kept    …/chat/main.hl (the application is not modified; the organization oversees it from dna/org)
edited  …/chat/hale.toml ([claims] no_base, [environments.local], [environments.org])
seeded  refs/dna/journal (7 event(s): application.attached, structure.observed, responsibility.proposed, review.requested)
```

`init` refuses a program that does not check, and it does not touch
your application. What appeared:

| what | where | yours? |
|---|---|---|
| **The purpose.** One sentence: what this codebase is for. The first review asks you to ratify it. | `dna/org/purpose.hl` | yes — edit it |
| **The organization.** The Board, the Leader and its grant, the gateways. Ordinary Hale source; `hale check` validates it. | `dna/org/main.hl` | yes — edit it |
| **The catalog.** Which model each position calls, and the budget, written from what your machine had. `hale dna models` probes it. | `dna/org/models.hl` | yes — edit it |
| **The knowledge graph's environment.** Its Postgres, for `hale dna dev` through docker compose. | `dna/compose.yaml` | yes — edit it |
| **The law.** What no position may ever do, enforced by the compiler against the wiring you actually built. Add to it; don't weaken it. | `dna/org/law.hl` | yes — extend it |
| **The toolchain's part.** The DNA itself, pinned to your `hale` version. Ignored by git; `hale dna upgrade` refreshes it. | `vendor/dna/` | no |
| **The record.** Everything the organization does, one commit per event, plus receipts and leases. Not files: refs in your repository. | `refs/dna/*` | no — but it's git |
| **Scratch.** Sockets, sandboxes, the toolchain's inputs, the status projection. Ignored by git; delete it and nothing is lost. | `.hale/dna/` | no |

Your `hale.toml` gained two environments — the application and the
organization are two entrypoints, each checked against its own law
— and `[claims] no_base = true`, because they deliberately share
none.

Commit it, and if you have a remote, push the record with the code:

```sh
git add -A && git commit -m "the organization"
git push origin main 'refs/dna/*:refs/dna/*'
```

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
hale dna dev: membrane bound at …/.hale/dna
built: …/./chat
hale dna dev: expression chat (pid 1874991) under LOTUS_OBS=1
chat: 1 ping(s) echoed; membrane open
```

Leave this terminal open; it is the host. It rebuilds and restarts
the application when a change is approved and watches it. Under
`hale dna run` only the organization runs here, and the application
is expressed wherever it lives — [Operating the fleet](./operating.md).
In another terminal:

```text
$ hale dna status
organism:   running (membrane bound)
journal:    7 event(s), chain verified
expression: attached Chat (shape 3c9b9327e480d349) · current shape 3c9b9327e480d349 · build 812f3c9bd9e4
intents:    0 offered, 0 refused
tasks:      none
reviews:    1 pending of 1
  purpose [pending] needs board — ratify the declared purpose?
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

That is the whole review mechanism, on the smallest possible thing.
`--as` is your name on the record; a verdict from the terminal
carries the Board's authority unless you say otherwise. Every change
from now on goes through the same door: something is proposed,
someone with the authority is asked, they answer with their name on
it, and the answer is a commit.

Next: [Working with it](./working.md).
