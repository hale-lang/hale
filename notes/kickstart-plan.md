# A software organization's kickstart — plan

What a new organism's record holds the moment `hale dna init` finishes, so that everything a software organization starts from is in the graph before the first ask: the knowledge it works by, the structure it works in, and the rule by which a use site pulls what applies. Today `init` writes the application as observed (`application.attached`, `structure.observed`, `responsibility.proposed`), the declared purpose, and fifteen practices as proposals (eight `design`, seven `operating`), each with a Board Review, and nothing else: no knowledge of the language, of the design the organism is an instance of, or of how to work with it, and no positions. The rest of this note is the three items that fill that in, built on two things the graph already has: a knowledge node proposed and ratified per family, and a **binding**, an idea bound to a target with a reviewed applicability.

## 1. The library: knowledge bound to language and system nodes (seed/library)

- **Nodes for what knowledge is about.** `language:hale` (and any other language a codebase is written in), `system:dna` for the design itself, and a toolchain node per version (`toolchain:hale@0.24`). A chapter of the book or a section of the spec is a knowledge node whose body is the text and whose digest is the content's, with `provenance: toolchain` and the version it ships with.
- **Bindings, not ownership.** A chapter is bound to `language:hale` or `system:dna` with the toolchain version on the binding. Nothing is "the organism's knowledge of Hale"; the library is knowledge bound to nodes, and the same bindings from anyone else's chapters to `language:go` make a Go codebase's knowledge arrive the same way, with nothing Hale-specific in the mechanism.
- **A third practice family, `using`.** How to work with the organism in the abstract: the propose, review, ratify loop; change classes and magnitudes; what to ask the Leader and what to decide alone; when to cut structure; how to read the record. Distinct from `design` (how an organization is shaped) and `operating` (how the organism runs).
- **Proposed and ratified per family.** `library/language`, `library/design` and `using` are three proposals with one Board Review each (`hale dna review library approve`), since sixteen Reviews at init is already the ceiling of what a Board reads one by one. An upgrade of the toolchain proposes the new version's set as one proposal per family and retires the old bindings when no codebase pins that version.
- **Shipped in the binary,** as the DNA core and host already are, and seeded by every `init`; `init --no-library` leaves it out (fixtures that count seeded rows, an empty record on purpose). The book goes in per chapter and the spec per section, about 400 ideas and 3.6 MB of text per toolchain version, stored as every idea is (the record's receipts, the lanes), which is what makes replay and ranking work.

## 2. The structure family: mandates and equipment on the holes the record already proposes (seed/structure)

The record already proposes its structure at birth: `hale dna init` on a repository reads it into the graph (GH #1090) and proposes the holes the structure implies (GH #1091): positions, `reviews` edges, operational roles, work items, practices as advice; review routing (GH #1087), the per-position brief (GH #1088) and evidence, verdicts and re-ingest from the CLI (GH #1089) build on those rows. An application's `init` is narrower today: it observes loci and topics and proposes responsibilities, no positions. This item enriches that one path through the same graph and proposal machinery; it adds no starter chart and no second initialization.

- **An application proposes its holes too.** The hole-proposer that runs for a repository runs for an attached application, reading the structure `init` already observed (its loci and topics, its contracts), so an application's record starts with the positions, `reviews` edges and operational roles its structure implies, as proposals. A founder alone fills several; the seeded design practices say the rest: minimal structure, growth by proposal.
- **Mandates bound to positions.** Each proposed position carries a mandate as a knowledge node bound to the position node: what it decides, what it may not, what it cites, how it writes and what it escalates. The toolchain ships the mandates for the positions the proposer knows (board, leader, architect, reviewer of a part, maintainer, operator), proposed with the position and ratified with it. The Leader's brief reads its own mandate with the charter; a reviewer's brief reads the reviewer's; the per-position AGENTS.md renders it.
- **Authority stays where it is granted.** Which change classes and magnitudes a position may ratify, which need a second, which are money, are the charter's and the law's; the record states the defaults the proposer derives for a position and the charter refines them. An API surface's `requires` names roles and informs contract relationships (a `reviews` edge for the surface's part); it never determines the organization or grants authority.
- **Standard equipment.** What a position is fitted with on fill: the API roles a surface's `requires` names, so `requires: [reviewer]` resolves to a position the record knows; the broker account; the vault names. The seeded practice `design/standard-equipment` already says a supervising part is born with its architect; this is the equipment list it lacked.
- **Holders stay empty.** Filling is a human act: `hale dna fill board <you>` is the organization's first decision.

## 3. The use-site pull rule (seed/pull)

- **Codebases point at languages.** A codebase node (item 4 of the heart plan, `notes/heart-protocol-plan.md`) carries `written_in` edges and pins its toolchain version.
- **The pull exists; its targets widen.** The hat every performer reads already carries a context package: the accepted ideas bound to the Work's target or above it, ranked inside a budget by a deterministic lexical embedding against the Work's objective, under one snapshot (GH #583, the knowledge lanes). Targets are locus paths today. This item widens them to graph nodes, exact-match for a node id and prefix-match for a path as now, and gives the hat a target set: the Work's locus path, the language nodes of the codebase the Work's target belongs to (the attached application's, `language:hale` for a Hale one), `system:dna` for an organization change, and the performer's position node for its mandate. A reviewer's brief reads the same package for the change before it. When and where a chapter arrives is decided by the site that asks, never by the seed; a hosted embedder is the same shape later.
- **Nothing is read whole by default** except ratified practices, which stay organism-wide, as today.

## Exit criteria

A fresh `hale dna init` of an application leaves the record with the library, the `using` practices and the holes its structure implies, each position with its mandate, all as proposals; `hale dna review library approve`, `using approve` and the structure's Reviews ratify them; `hale dna fill board <you>` holds. A Work's brief for a Hale codebase cites the chapter that applies and no other; a reviewer's brief cites the reviewer's mandate; a foreign codebase bound to `language:go` with chapters of its own pulls them through the same path, with no Hale-specific branch in the brief. The DNA suite holds a fixture per claim. Voice stays the repository-ingestion regression fixture (GH #1092); the todo organization below is the application-side one.

## The first user: the todo organization

A realtime todo list in Hale, started from scratch in local mode (`hale dna dev` over `dna/compose.yaml`), torn down and started over until the loop is seamless. It is an organizational test first and a CRUD-and-realtime test second, and it supplies the two pieces of acceptance evidence GH #1092 still lacks: a complete change-delivery and CLI-evidence walkthrough (deferred by #1153) and the later-commit re-ingest and diff.

1. Initialize the repository with its purpose, constraints and application structure; the record holds the library and the holes.
2. Ratify the relevant proposals and fill the working positions.
3. Give the organization one real feature to deliver.
4. An agent claims it, receives its brief (claiming and reading the brief are two recorded acts), implements it and submits candidate-specific evidence from the CLI.
5. Review routes to the position the graph names; the work settles.
6. Change an application contract at a later commit; re-ingest; verify the graph is maintained through reviewed proposals, with the diff the record states.

Alongside, an agent operates the todo app through its API (`hale api call`, MCP) while browsers observe the changes through the realtime stream. The app's todo items and the organization's development Work are separate records. The organization and process views come from the same graph.

## Order and size

seed/pull first with the smallest library (one chapter bound to `language:hale`, one to `system:dna`), since the pull rule is the mechanism the other two are proven through; then seed/library whole (the embedding, the three families, upgrade as rebind); then seed/structure (the application-side holes, the mandates, the equipment). One PR each, the `using` practices as prose in seed/library's PR. The items are named `seed/…` because K1 to K3 already name the knowledge lanes (GH #583). This lands before the todo organization dogfood, which is its first user.
