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

**The knowledge service holds the live half.** A small Hale program,
`dna/knowledge/service`, tails the record — every `knowledge.*` row
from where it left off, each digest resolved to its receipt — into a
store, and answers questions about it over HTTP. The store is
Postgres: shared, so the organization on your workstation and the
services on their nodes see one graph; durable, so a restart resumes
from a watermark and converges. Nothing about it is on the local
filesystem. In a pinch (a laptop with nothing installed, a test) the
same service runs over an in-memory store that lives as long as the
process.

## Running it

`hale dna dev` runs the service for you. `init` wrote
`dna/compose.yaml`, the graph's Postgres with a named volume per
repository, and `dev` brings it up, waits for it, derives the
connection string and starts the service beside the organization:

```text
$ hale dna dev
hale dna dev: organization (pid 41200) from …/chat under LOTUS_OBS=1
hale dna dev: membrane bound at …/chat/.hale/dna
hale dna dev: knowledge service (pid 41233) at :8791 over postgres
hale dna dev: expression chat (pid 41240) under LOTUS_OBS=1
```

`docker compose` on `PATH` is all it needs. Without it, the host says
so and runs without a knowledge service; point
`HALE_DNA_KNOWLEDGE_DSN` at a Postgres of your own
(`postgres://user:password@host:port/db`) to use one anyway, or at
`memory` for the in-process store. Beyond one machine, and under
`hale dna run`, the service is an instance in the plan against your
Postgres, like any other service; `hale dna knowledge` runs it in
the foreground.

## Asking it

```text
$ curl -s localhost:8791/
{"store": "postgres", "watermark": 58, "record": 58, "ideas": 3, "ratified": 1, "bindings": 1, "ticks": 412, "error": ""}

$ curl -s 'localhost:8791/context?target=org/leader/worker/mailer&budget=8'
{"target": "org/leader/worker/mailer", "revision": 58, "digest": "sha256:…", "included": "sha256:8f17…", "included_n": 1,
 "ideas": [{"id": "sha256:8f17…", "kind": "practice", "text": "retry a mail send once before raising pressure", "author": "org/leader", "ratified_seq": 41}]}
```

A **context package** is what a position receives: the accepted ideas
bound to its locus path or to any path above it (goals flow down;
initiatives stay where they were made), capped by a budget, with a
digest over the target, the store's revision and the ids — the thing
a model call's evidence names. A sibling's package does not carry
your wing's practice.

## How it reaches the work

The editor never talks to the service; the law says so
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
receipt says which package. With no service running, the package is
empty and says so; nothing waits.

## Concerns

A **concern** is a child's signal about the part above it. An
application raises one on the membrane, or you do:

```text
$ hale dna concern raise org/trio/worker mail backlog behind fulfilment --severity 2
concern raised by org/trio/worker: mail backlog behind fulfilment
```

Each is `concern.raised` in the record. Three from one source and it
becomes a proposal — by that source, bound to its parent, a concern
by the tower rule — and lands in your queue for ratification. A
source with no parent has nothing to bind to and is refused, saying
so. What is not yet here — retrieval by similarity, the application
publishing its own observations — is the rest of Track K.
