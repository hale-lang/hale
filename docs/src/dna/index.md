# The organism, part by part

The product is the **organism**: your Hale application, plus the parts
that let it be governed, changed, run and watched. Someone asks for an
outcome; the organism turns it into work, proposes a change in a
sandbox, proves it with the toolchain, asks whoever holds the authority,
applies exactly what was approved, runs it, and watches that it works.
Every decision is a row in a record that lives in your git repository.

This page is the map. Each part has one job, one place in the tree, and
one chapter that covers it.

## How to read this section

- Start with [Getting started](./getting-started.md): make an
  organism, fill its vault, start it, answer the first Review.
- Then [One task, end to end](./workflow.md): one request from the
  terminal to a change kept or rolled back, with the record's rows
  beside each step.
- The part chapters follow, one per group of parts. Read the one you
  need when you need it.
- [Shaping and governing it](./shaping.md) is how you change what the
  organism is: its purpose, practices, grant and law.
- [DNA, the building block](./dna.md) comes last on purpose. DNA
  (`vendor/dna`, the `dna::Dna` locus, the `hale dna` commands) is what
  every part is assembled from. It is the inside view, and you do not
  need it to run an organism.
- [Troubleshooting](./troubleshooting.md) and the
  [Reference](./reference.md) are for looking things up.

## The parts

| part | its one job | where it lives | chapter |
| --- | --- | --- | --- |
| **genome** | what the organism is made of: the application's source, the org chart and catalogs under `dna/org`, and the vendored core in `vendor/dna`; a node pulls it from the forge's default branch | your repository; the host's git for it is `dna/host/genome.hl` | [Shaping](./shaping.md) |
| **record** | long-term memory: every decision, in order, never rewound, as one commit per row under `refs/dna/*` | `dna/core/record.hl`, `dna/host/record.hl` | [Memory and the record](./memory.md) |
| **memory** | working memory: the ledger of the day's work, claims, the graph, protected evidence; Postgres, a role per client | `dna/core/memory_spine.hl`, `dna/host/memory.hl` | [Memory and the record](./memory.md) |
| **nerves** | carry facts between parts after their row has landed: NATS JetStream, one stream per organization, one user per part | `dna/core/nerves.hl`, `dna/host/nerves.hl`, `dna/nats.conf` | [The nerves](./nerves.md) |
| **face** | where the organism shows itself to people and takes their intent: a browser surface the head serves | `dna/face/` | [The head and the face](./head.md) |
| **head** | the edge API: sessions, principals, commands answered with a receipt; it never performs work | `dna/api/` | [The head and the face](./head.md) |
| **body** | the hosts the organism runs on, and the lease that says which body runs the record | `dna/core/infrastructure.hl`, `dna/host/infra.hl` | [The heart and the body](./heart.md) |
| **spine** | the program every node runs: it relays the record's requests, projects the record into memory, admits and settles work, runs the workflows | `dna/host/host.hl` over the organization in `dna/org/main.hl` | [The spine](./spine.md) |
| **legs** | the workers: each claims an attempt through the head, wears its hat, hands the outcome back, and holds nothing between tasks | `dna/core/legs/`, performers in `dna/org/work.hl` | [Legs, hands and voice](./legs.md) |
| **hat** | one content-addressed context per unit of work | `dna/core/hat.hl` | [Legs, hands and voice](./legs.md) |
| **hands** | the tools a Work's output contract gives a performer, and what runs them: git, the forge, the toolchain, the record's reads | `dna/core/contracts.hl`, `dna/core/tools.hl`, `dna/core/legs/hands.hl` | [Legs, hands and voice](./legs.md) |
| **voice** | the one seam to any model: the catalog, the model mapping, the adapters, the tape | `dna/core/models.hl`, `dna/core/model_map.hl`, `dna/core/tape.hl`, the catalog in `dna/org/models.hl` | [Legs, hands and voice](./legs.md) |
| **heart** | the application itself; its pulse is the events it publishes on its own subjects, each landed as a reading row | your application's seed; the host lands its events in `dna/host/pulse.hl` | [The heart and the body](./heart.md) |
| **senses** | the readings every long-running part serves, kept in one store | `dna/core/senses.hl`, the store's config in `dna/senses.yml` | [Senses and reflexes](./senses.md) |
| **reflexes** | reactions that need no plan: an instance that is down is restarted | `dna/reflexes/main.hl`; the node acts in `dna/host/reflex.hl` | [Senses and reflexes](./senses.md) |
| **skin** | the trust boundary: the vault, roles and accounts, OIDC, sealed credentials, closures recorded as violations | `dna/host/secrets.hl`, `dna/core/principal.hl`, `dna/oidc/stub.hl` | [The skin](./skin.md) |

Schedules, which say when a workflow runs and who convenes it, have
their own chapter: [Schedules](./schedules.md).

## Two rules

Every part keeps two rules. They are why nothing is half-written and
why a restart loses nothing.

**One store per step.** A workflow step writes to exactly one store,
by that store's one writer, and the next step reads what the previous
one made durable. A workflow definition names the store of each step
in one word (`record`, `forge`, `genome`, `heart`, `graph`, `nerves`,
`memory`, `vault` or `host`), and a step that names none, or two, is
refused before anything runs. A write whose subject moved since it was
read is refused and decided again. There is no transaction spanning
two stores, and none is needed. [The spine](./spine.md) has the
engine.

**Row first.** Every live signal is a row before it is sent. When you
ask for something, the command writes a row and stops; a node relays
that row onto the nerves until the record holds the answer. Events
name a row by id, arrive at least once, and are consumed once by that
id. An event from outside, such as the application's own, is a
signal, not a fact: it lands as a `reading.recorded` row before
anything acts on it. [The nerves](./nerves.md) has the relay.

## Trust tiers

Each part sits in one tier. A tier is defined by what it is handed and
what it is never handed.

| tier | who | is handed | is never handed |
| --- | --- | --- | --- |
| devices | a person's browser | a session cookie from the head | keys, git, a shell, the broker |
| identity and the forge | an OpenID provider; the forge (GitHub through `gh`, behind the core's `Forge` interface) | who a person is; where the genome and the record are pushed, and the verdicts on pull requests | anything operational |
| the head | the edge API and the face it serves | sessions; the mapping from a principal to positions; its own checkout of the record; memory's head role (take and fence a claim, read, write the ledger in its people's names); a broker user that may subscribe and never publish | the spine's database role |
| the body | the spine, run by `hale dna run` or `dev` on each node | memory's spine role; the nerves' spine user; the body lease | the owner's and the head's credentials, for memory and for the nerves |
| the body's legs | `hale dna work` | the lease on the attempt it claimed; the model keys in the vault of the machine it runs on | a database role: everything goes through the head |
| the heart | your application | its own store and secrets; a broker user that may publish only on its own subjects, `<org>.app.<name>.>` | the record, DNA's database, DNA's other credentials, DNA's subjects |
| private services | memory (Postgres), nerves (NATS), the vault, the senses' store (Prometheus), the reflexes | servers that listen on `127.0.0.1` only; the reflexes hold the store's read URL and a publish-only user | reachability from a device |
| external | a model's API | one prompt and one key, per call | anything it was not sent |

The local vault is one directory per user. Any program that user runs
could read any entry by name, so what keeps a part to its own
credential is what it is handed, not what it could read.
[The skin](./skin.md) covers the vault and the accounts.

## What it will and won't do

Plainly, so you can decide whether it fits.

### It will

- Take a request in a sentence, from the terminal
  (`hale dna task create`) or from the face, and turn it into one
  execution of a workflow the record keeps.
- Make a proposed change in a sandbox worktree of your repository.
  The editor holds four tools there: read, edit, format and check.
- Prove the candidate: its base artifact, format, check, verify,
  tests, the fleet it would deploy, a rehearsed rollback, and the
  structural diff. Each result is a receipt in the record.
- Show the source diff, the semantic diff and the evidence together,
  pinned to one commit: in the terminal, in iris, on the read-only
  page `hale dna ui` serves, and as a pull request when GitHub is
  configured.
- Let the Leader, a model, decide inside a grant the Board gave, with
  its reasoning on the record, and ask the Board for everything else.
- Apply exactly the approved commit, express it (a restart under
  `hale dna dev`, a deploy row every node of a fleet answers, or your
  own deployment command), watch it for a window, and keep it or roll
  it back.
- Grow its own org chart through the same door: a change to the
  organization is the Board's to review, and applying it restarts the
  organization.
- Hand a person's job to that person as a Task, and wait for them.
- Run work as workflow definitions written in code, and resume every
  execution after a restart under the same ids.
- Record all of it in order: in git under `refs/dna/*`, and, once you
  adopt the ledger, the day's work in memory.

### It won't

- **Apply anything without a verdict.** The Leader's verdict is a
  model's, inside a grant only the Board writes; the Board's is a
  person's.
- **Let the editor commit, open worktrees, apply, or read memory's
  knowledge.** The law in `dna/org/law.hl` forbids each, and a wiring
  that breaks it fails to build, with a witness path.
- **See behaviour in the structural view.** The semantic diff sees
  structure and rules and is blind to what a handler computes. That is
  why the source diff is always beside it.
- **Grow without the Board.** Persistent pressure becomes a proposal;
  nothing joins the org chart until the Board approves the commit.
- **Migrate live state.** A restart is a restart. State the
  application keeps only in memory is gone across a change, as it
  would be across any deploy.
- **Diff the fleet as a whole.** A Review's semantic diff is the edited
  seed's. The deploy row names the instances it reaches.
- **Raise pressure from your metrics on its own.**
  `hale dna pressure raise` writes a pressure signal; nothing raises
  one from a service's metrics.
- **React beyond one reflex.** The reflexes have one rule: an instance
  its node reads as down is restarted. They do not roll back or back
  off.
- **Take commands on `hale dna ui`.** That page reads the record and
  refuses every form. Commands go through the head, which serves the
  face.
- **Run a definition whose step writes the heart, the vault or a
  host.** Such a definition is refused at admission, and
  `hale dna definitions` says why. A leg's deploy and heart hands
  refuse too.
- **Read a model key from the environment.** A model key is a vault
  slot, `model-<NAME>`, filled with `hale dna secret set`.
- **Run without git.** The record is refs in your repository, changes
  are commits, rollbacks are resets. Teammates and nodes share it
  through a remote.
- **Promise an external effect happened once.** A transition happens
  once in the record. Whether a performer's effect ran once across a
  restart is its adapter's to say; one that cannot say leaves the
  attempt for a person (`hale dna effect resolve`).
- **Close a person's job for them.** A case waits for its person's
  `hale dna task done`, through any restart.
- **Cancel an execution from the terminal.** The engine can cancel;
  no `hale dna` verb asks it to.

### What the evidence supports

Before the apply path was built, reviewers decided planted changes
from the structural diff alone, the source diff alone, and both. The
source diff and the combined view decided every case; the structural
view alone decided the rule-backed cases and correctly held on the
rest. So the combined view is what you get, and what the Leader gets.
The write-up is `dna/kill-test/WALKTHROUGH.md` in the hale repository.

The contract behind every statement on these pages is
`spec/dna.md`.
