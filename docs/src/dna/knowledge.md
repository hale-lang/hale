# The knowledge graph

What the organization learns lives in two places, and only one of
them decides.

**The record holds the decided half.** When a position proposes
knowledge — a practice for the wing it oversees, a concern about the
one above it — the proposal is filed as a receipt (its content, by
digest) and a Review is born for the Board, pinned to that digest.
You see it in the queue like any other:

```text
$ hale dna review
2 pending review(s) of 2
  k:8f1720dfed87 needs board — ratify practice `retry a mail send once before raising pressure` as a goal for org/leader/worker?
  …
$ hale dna review k:8f1720dfed87 approve --as you --comment "a good practice"
```

Your approval is one row in the record, `knowledge.ratified`, naming
the digest. That row is the authority: nothing becomes ratified
knowledge any other way, and a digest in the record always resolves
to the content that was ratified, because the content is a receipt
beside it. Who may bind what to whom is the tower rule, computed, not
tagged: a parent's idea bound to a child is a **goal**, a child's
bound upward a **concern**, one's own an **initiative**, and siblings
do not privately coordinate — a lateral proposal is refused before
any Review.

**Memory holds the live half.** The body's host tails the record —
every `knowledge.*` row from where it left off, each digest resolved
to its receipt — into a graph in Postgres, and everything that asks a
question of the graph reads it there. The store is shared, so the
organization on your workstation and the services on their nodes see
one graph, and durable, so a restart resumes from a watermark and
converges. Nothing about it is on the local filesystem. There is no
service in front of it and no in-process stand-in: memory is Postgres.

The tail is `MemoryProjection`, and it lives in the operations
(`dna/operations/memory_tail.hl`), beside the command codecs it
decodes admitted facts with, not in the core. The stores it writes
into are the core's (`dna/core/memory_*.hl`), and they reach Postgres
through pond's driver, pinned under `dna/core/pond` and vendored into
your project at `vendor/dna/pond`. The projection runs on the host's
tick, once a second — every host that runs a body, beside any number of
others (see [Operating](./operating.md#the-spine-is-every-node)) — and
nothing else writes the graph.
Each record row is one transaction: the projector moves memory's stamp
— the row count and the last row's commit — by compare-and-swap before
the row's effects, and both land together. So a projection interrupted
mid-row leaves nothing of it, and any number of projectors over one
record converge on one graph with each row applied once: a projector
that finds the stamp moved writes nothing and leaves the row to the
one that moved it. A projector compares its clone to the stamp by
ancestry: behind it — or stamped at a commit it has not received yet —
it has nothing to project; ahead of it, it projects the rows after it;
on a chain a reconcile replaced, it rebuilds the graph from row 0.

## Running it

`hale dna dev` gives the organization its memory. `init` wrote
`dna/compose.yaml`, memory's Postgres with a named volume per
repository, and `dev` brings it up, waits for it, applies memory's
schema for the record, and starts the host on it:

```text
$ hale dna dev
hale dna dev: memory: the graph, the ledger and protected evidence under the record's spine role
hale dna dev: organization (pid 41200) from …/chat under LOTUS_OBS=1
…
```

`docker compose` on `PATH` is all it needs. Without it, point
`HALE_DNA_MEMORY_DSN_OWNER` at a Postgres of your own
(`postgres://user:password@host:port/db`). With neither, the host
says so and runs with no memory: nothing is projected, and a package
is empty and says why.

That DSN is the schema's **owner**, and nothing that runs holds it.
Before it starts anything, `dev` applies memory's schema for the
record with it — the tables, two roles of the record's own, and a
schema version — and hands the host only the spine role's DSN
(`HALE_DNA_MEMORY_DSN_SPINE`), which can read and write the tables but
not change them. `hale dna memory migrate` does the same step by hand
and prints both roles' DSNs, and `hale dna upgrade` does it when the
owner's DSN is set. A store opened on a schema at another version
refuses it, naming both versions and the command that fixes it.

One Postgres can hold many records. Memory is scoped by the record —
its identity is the sha of the record's first commit, the same in
every clone and different for every record — and each record gets its
own schema, `dna_<sha>`, and its own roles, so two projects pointed at
the same database see two graphs, each with its own watermark, and
neither role can read the other record's. A schema that names a
different record is refused, not read; so is a store in `public` from
before stores were scoped (drop its tables, and the spine rebuilds the
projection from the record).

## Asking it

A **context package** is what a position receives: the accepted ideas
bound to its locus path or to any path above it (goals flow down;
initiatives stay where they were made), capped by a budget, with the
store's revision and a digest over the target and the ids — the thing
a model call's evidence names. A sibling's package does not carry
your wing's practice.

The package is read from memory under the spine's role by
`MemoryKnowledge`, which the organization holds. It answers for the
record as it stands *now*: a practice ratified a moment ago may not be
projected yet, so the package waits — up to 20 seconds — for the
projection to reach the record's head, and refuses rather than hand
over a stale one:

```text
memory's projection is at row 57 of the record's 58; the spine applies it on its tick
```

The read API reads the same graph with the head's DSN
(`HALE_DNA_MEMORY_DSN_HEAD`), which may select from the graph and
change none of it; a projection that has not caught up is
`knowledge_projection_unavailable` there, never an empty graph.

## How it reaches the work

The editor never reads memory; the law says so
(`editors_never_learn`). Its owner does. When a change opens, the
substrate asks for the package of the change's place in the tower —
`org` for the organization's own source, `org/<app>` for the
application, `org/<app>/<seed>` for a seed inside it — writes
`knowledge.consulted` in the record, and hands the editor the ask
with the practices under it:

```text
document the Gateway locus in main.hl

PRACTICES (ratified knowledge for org/trio, package sha256:4f7f…):
- practice: retry a mail send once before raising pressure (org)
```

The record, the commit message and the Review keep the ask itself.
Every model call of that attempt names the package it was given:

```text
   23  model.called   m1/a0   {…, "knowledge_bindings": "package:sha256:4f7f… sha256:8f17…", …}
```

So what was ratified today is in the prompt tomorrow, and the
receipt says which package. With no memory named, the package is
empty and says so; nothing waits.

## What the graph knows besides ideas

The projection also carries two things the record already holds. The
code's **structure** as `init` observed it — loci, topics, bindings,
effect classes, claims — by kind and name, so an idea has something
to bind to; and the fleet's **signals**, every `pressure.raised` and
`concern.raised`, counted per source. They live in the record's
schema beside the ideas (`knowledge_structure`, `knowledge_signals`).

## Ranking inside the bound

A package is bounded first: only ideas bound to the target or above
it, never a sibling's. Inside that set, the objective ranks. The
substrate sends what the work is about as the query, and the budget
keeps the most relevant.

The embedding is deliberately small: a hashed bag of words, the same
on every machine, rendered as a pgvector literal so Postgres ranks
with `<=>`. It cannot tell synonyms apart; it can tell a mail
objective from an archive one, which is what a budget of eight over a
wing's practices needs. A hosted embedder is the same shape, text in
and a vector out, when one is worth its cost.

## Concerns

A **concern** is a child's signal about the part above it. An
application raises one by declaring the fact itself — the same shape
the membrane speaks, on the subject `dna.concern.raised`, with no
import of the DNA — and publishing it when it sees something:

```hale,fragment
type Concern { source: String = ""; what: String = ""; severity: Int = 0; }
topic WorkerConcerns { payload: Concern; subject: "dna.concern.raised"; }
// …
WorkerConcerns <- Concern { source: "org/trio/worker", what: "mail backlog behind fulfilment", severity: 2 };
```

The node the instance runs on hears it on a socket of its own and
puts it in the record; the host beside the organization relays it
onto the membrane; the organization writes `concern.raised`. Or you
raise one yourself, from anywhere with the record:

```text
$ hale dna concern raise org/trio/worker mail backlog behind fulfilment --severity 2
concern raised by org/trio/worker: mail backlog behind fulfilment
```

Each is `concern.raised` in the record, carrying which raise it is
(`occurrence`). Three from one source and it becomes a proposal — by
that source, bound to its parent, a concern by the tower rule — and
lands in your queue for ratification. A source with no parent has
nothing to bind to and is refused, saying so.

The count is the record's, not a number the organization keeps, so
restarting it changes nothing: the fourth concern is still the
fourth. What a restart *does* pick up is a proposal left owed — the
raise that reaches three is also the raise that proposes, so an
organization stopped between the two comes back to a source over the
threshold with nothing proposed for it, which no further concern may
ever arrive to fix. It proposes each of those once, at birth.

## What it adds up to

The in-repo fixture runs the whole loop, keyless, on every change to
the organization: a worker on one node observes its mail backlog and
says so three times; the third becomes a proposal; the Board ratifies
the exact digest; the next change to that application is made with
the concern under its objective, and every model call of the attempt
names the package it was given. What was observed and ratified today
informs the work done tomorrow, and the receipt says which package.
