# Getting started

## What you need

- A Hale application with a `main locus` that passes `hale check`.
- A git repository around it. DNA proposes changes as commits and
  rolls back with git.
- A program that stays running. The organism answers you through
  the running process; a program whose `run()` returns exits, and
  takes the organism with it.
- The `hale` binary on your `PATH`.

If you don't have an application yet, `hale dna new demo` makes one
with the DNA already attached, and you can skip to [the first
run](#the-first-run).

## Attach it

```text
$ hale dna init .
ok: 1 file(s) typechecked
wrote   vendor/dna (13 file(s) written, 0 unchanged; hale.lock pins toolchain 0.19.2)
cut     .hale/dna/baseline.topology (schema 1.19, shape 3853f0f14bbf1639, verdict clean)
created dna/purpose.hl
created dna/assembly.hl
created dna_constitution.hl
edited  main.hl (imports, `genome` param, `adopt Project`, membrane bindings)
edited  hale.toml ([claims] base, [environments.local])
seeded  .hale/dna/journal.jsonl (17 event(s): application.attached, structure.observed, responsibility.proposed, review.requested)
edited  .gitignore (/vendor/, /.hale/)
```

`init` refuses a program that does not check, and it never rewrites
what you wrote — it adds. Here is what appeared, in the order you'll
care about it:

| what | where | yours? |
|---|---|---|
| **Your purpose.** One sentence: what this program is for. The first review asks you to ratify it. | `dna/purpose.hl` | yes — edit it |
| **Your settings.** How much autonomy, which review policy, which models. Ordinary Hale source; `hale check` validates it. | `dna/assembly.hl` | yes — edit it |
| **The rules.** What the organism may never do, enforced by the compiler. Add to them; don't weaken them. | `dna_constitution.hl` | yes — extend it |
| **The toolchain's part.** The DNA itself, pinned to your `hale` version. Ignored by git; `hale dna upgrade` refreshes it. | `vendor/dna/` | no |
| **The organism's memory.** Its history, its evidence, its sandboxes. Ignored by git. Delete it and the organism forgets; your code is untouched. | `.hale/dna/` | no |

Your `main.hl` gained a few lines: two imports, a `genome` param
(the organism lives inside your main locus, like any other child),
`adopt Project;` in its claims, and three socket bindings — the
door through which your verdicts and requests come in. You can read
them; you won't need to touch them.

Commit it:

```sh
git add -A && git commit -m "attach the DNA"
```

## Check nothing broke

```text
$ hale check --matrix .
=== ./. @ local ===
ok: 2 file(s) typechecked

ok: 1 (entrypoint, environment) pair(s) checked

$ hale test .
ok   ./tests/main_test.hl

1 passed, 0 failed
```

Your program still checks, now against the rules too, and its tests
still pass. Those tests matter twice from here on: every change the
organism proposes will be verified against them.

## The first run

```text
$ hale dna run
…
hale dna run: organism chat (pid 1694293) from … under LOTUS_OBS=1
hale dna run: membrane bound at …/.hale/dna
hale dna run: iris at http://127.0.0.1:8787/  (l law · 4 review · 5 organism · m membrane)
```

Your program is running, with the organism inside it and iris
watching. Leave this terminal open; it is the host, and it will
rebuild and restart the program when a change is approved. In
another terminal:

```text
$ hale dna status
organism:   running (membrane bound)
journal:    17 event(s), chain verified
expression: attached ChatServer (shape 3853f0f14bbf1639) · current shape 8517c3db7499d3b3 · build 68c91b13f399
intents:    0 offered, 0 refused
tasks:      none
reviews:    1 pending of 1
  purpose [pending] needs maintainer — ratify the declared purpose?
mutations:  0 (none applies before a human's verdict on the exact candidate)
```

## The first review

There is already something to decide: the organism wants you to
ratify the purpose `init` wrote. Read `dna/purpose.hl`, change the
sentence to what the program is actually for, and approve it:

```text
$ hale dna review purpose approve --as riley --comment "ratified"
review purpose settled: approve by riley
```

That is the whole review mechanism, on the smallest possible thing.
Every change from now on goes through the same door: something is
proposed, you are asked, you answer with your name on it, and the
answer is recorded.

Next: [Working with it](./working.md).
