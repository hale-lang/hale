# A software organization's kickstart — plan

What a new organism's record holds the moment `hale dna init` finishes, so that everything a software organization starts from is in the graph before the first ask: the knowledge it works by, the structure it works in, and the rule by which a use site pulls what applies. Today `init` writes the application as observed (`application.attached`, `structure.observed`, `responsibility.proposed`), the declared purpose, and fifteen practices as proposals (eight `design`, seven `operating`), each with a Board Review, and nothing else: no knowledge of the language, of the design the organism is an instance of, or of how to work with it, and no positions. The rest of this note is the three items that fill that in, built on two things the graph already has: a knowledge node proposed and ratified per family, and a **binding**, an idea bound to a target with a reviewed applicability.

## 1. The library: knowledge bound to language and system nodes (K1)

- **Nodes for what knowledge is about.** `language:hale` (and any other language a codebase is written in), `system:dna` for the design itself, and a toolchain node per version (`toolchain:hale@0.24`). A chapter of the book or a section of the spec is a knowledge node whose body is the text and whose digest is the content's, with `provenance: toolchain` and the version it ships with.
- **Bindings, not ownership.** A chapter is bound to `language:hale` or `system:dna` with the toolchain version on the binding. Nothing is "the organism's knowledge of Hale"; the library is knowledge bound to nodes, and the same bindings from anyone else's chapters to `language:go` make a Go codebase's knowledge arrive the same way, with nothing Hale-specific in the mechanism.
- **A third practice family, `using`.** How to work with the organism in the abstract: the propose, review, ratify loop; change classes and magnitudes; what to ask the Leader and what to decide alone; when to cut structure; how to read the record. Distinct from `design` (how an organization is shaped) and `operating` (how the organism runs).
- **Proposed and ratified per family.** `library/language`, `library/design` and `using` are three proposals with one Board Review each (`hale dna review library approve`), since sixteen Reviews at init is already the ceiling of what a Board reads one by one. An upgrade of the toolchain proposes the new version's set as one proposal per family and retires the old bindings when no codebase pins that version.
- **Shipped in the binary,** as the DNA core and host already are; `init --no-library` leaves it out. The record's size grows by the library's text once; the Knowledge lanes index it as they index any ratified node.

## 2. The structure family: positions, mandates, equipment, shapes (K2)

- **Positions as nodes with mandates.** A default chart: board, leader, architect, reviewer per part, maintainer, operator, release owner. Each position's mandate is a knowledge node bound to the position node: what it decides, what it may not, what it cites, how it writes and what it escalates. The Leader's brief reads its own mandate with the charter; a reviewer's brief reads the reviewer's.
- **Authority defaults.** Which change classes and magnitudes each position may ratify, which need a second, which are money: rows the record states and the charter refines, where today the charter and the law carry them alone.
- **Standard equipment.** What a position is fitted with on fill: the API roles a surface's `requires` names, so `requires: [reviewer]` resolves to a position the record knows; the broker account; the vault names. The seeded practice `design/standard-equipment` already says a supervising part is born with its architect; this is the equipment list it lacked.
- **Holders stay empty.** Filling is a human act: the seed proposes the chart, and `hale dna fill board <you>` is the organization's first decision.
- **A shape at init.** `init --shape solo|team|platform` picks one chart (a founder alone; a small product team; platform plus product), proposed as the `structure` family with one Review.

## 3. The use-site pull rule (K3)

- **Codebases point at languages.** A codebase node (item 4 of the heart plan, `notes/heart-protocol-plan.md`) carries `written_in` edges and pins its toolchain version.
- **A brief follows bindings from what it is working on.** A Work's brief follows Work to codebase to language to bound knowledge and renders what the Knowledge lanes rank relevant; a reviewer's brief does the same for the change before it; an organization change pulls what is bound to `system:dna`; a position's brief pulls its mandate. When and where a chapter arrives is decided by the site that asks, never by the seed.
- **Nothing is read whole by default** except ratified practices, which stay organism-wide, as today.

## Exit criteria

A fresh `hale dna init` of an application leaves the record with the library, the `using` practices and a shape's structure as proposals; `hale dna review library approve`, `using approve` and `structure approve` ratify them; `hale dna fill board <you>` holds. A Work's brief for a Hale codebase cites the chapter that applies and no other; a reviewer's brief cites the reviewer's mandate; a foreign codebase bound to `language:go` with chapters of its own pulls them through the same path, with no Hale-specific branch in the brief. The DNA suite holds a fixture per claim.

## Order and size

K3 first with the smallest library (one chapter bound to `language:hale`, one to `system:dna`), since the pull rule is the mechanism the other two are proven through; then K1 whole (the embedding, the three families, upgrade as rebind); then K2 (the chart, mandates, equipment, shapes). One PR each, the `using` practices as prose in K1's PR. This lands before the todo organization dogfood, which is its first user.
