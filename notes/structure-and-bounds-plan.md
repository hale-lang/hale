# Structure as batches, onboarding in five commands, and bounds that mean something — plan

The process harness skeleton is on main and its from-scratch cycle is green three times running (`scripts/dna-cycle/cycles.sh 3`). Three things still keep it from being a small organization a person stands up in minutes: every structural change is its own Review, so a team costs a dozen verdicts; onboarding takes about fifteen commands, one of them a script run from a hale checkout; and the operations the habitat names beyond opening and filling a position are not built. Beside that, the compiler's memory-bound survey is advisory everywhere and DNA answers it with 1,298 `@unbounded` decorators, most of which suppress nothing.

This note is the plan for all of it and the order it lands in.

## The batch is the unit of structural change

A **batch** is a list of operations (open a position, fill, vacate, assign, merge, split, adopt or retire a pack, expose, depend, externalize, internalize), validated as a whole and proposed as one Board Review, applied all or nothing.

- **One grammar.** A batch is written in the step language the scenario runner already reads (`open api/dev | <mandate>`, `fill api/dev ada`, `vacate api/dev ada`, `expect holds api/dev ada`). The parser moves from the test into `dna/core` and serves both. A scenario is a batch with `expect` lines.
- **Validated before it is proposed.** `hale dna apply <file> [--dry-run]` simulates the batch on the graph and the holds the record states, and checks the invariants on the end state: every hold on a stated position, every position with a mandate, no `under` naming a node the graph does not state, a part's reviewer never its dev. Order inside a batch does not matter; the end state does.
- **Read as a change to the organization.** `--dry-run` prints the org chart before and after; the same difference, as data, is what the Board's Review renders and what the UI draws.
- **One Review.** The organization proposes the batch as one family Review in the group `changes`, its members each step's proposal. Ratifying applies every member in one append. A batch whose ground moved (a position it fills was retired, a name it opens was taken) is refused at ratification as stale and never half-applied.
- **Governance.** A batch is a governance proposal: it requires the Board, and the Board may ratify what it asked for.

A **pack** is a named, versioned batch together with the knowledge it brings (practices, mandates, workflow definitions), published as a library family so `upgrade` and supersession already apply. **Bootstrap** is `init` with a pack. **Advice** from the Leader, later, is a proposed batch the Board can preview and ratify.

## Every mutation is a head command; the UI is a client

A structural change from the UI is not a second code path. Each operation lands as an `rpc Commands::*` member of the head's generated surface (`HeadCommands`) whose handler writes the same request row its CLI verb writes; the organization answers that row whoever sent it; the Board decides through `Commands::verdict`. Authority is the surface's `requires:`. The generated clients serve the legs, the CLI and the habitat UI alike.

- `fill`, `position open`, `ingest` and the model mapping are CLI-only today; they become `Commands::fill`, `Commands::open_position`, `Commands::ingest`, `Commands::set_model_rule` and `Commands::set_model_category`.
- A batch has a preview read (the org-chart difference as JSON) and a command, `Commands::apply_batch`.
- Every operation's acceptance includes a test through the generated client, and the from-scratch cycle drives one path through the head's API rather than the CLI.
- The screens belong to the habitat UI (GH #1462): this work delivers the commands, the previews, the generated clients and their documentation; the UI wires its affordances when it is rebased. Habitat operations (graft, extract, absorb, re-parent) and administration stay out of the UI until saplings.

## The operations

Each operation is a batch step kind, a head command and a scenario; a scenario of every operation not built yet is in the runner's known-open table, asserted to fail, so the whole set is executable from the start and each operation that lands turns its entry over.

In order: vacate; assign (a position writes or reviews a part or a contract); adopt and retire a pack; merge and split positions (holds and bound knowledge carried over, mandates and bindings divided); expose and depend (on the contract and tool nodes `hale dna ingest` states); externalize and internalize. The habitat's operations span several records and come with saplings, after this plan.

## Packs and starter teams

`hale dna pack list | show | apply`, `apply` being a batch Review. Three packs ship first:

- **a dev team**: a dev and a reviewer per part, review practices, the routed-ask defaults;
- **a small startup**: the Board, the Leader, a product position and one dev team;
- **a team that ships no code**: content or marketing, with `Report` contracts, schedules and receipts.

Retiring a pack restores what it superseded.

## The nursery

Packs and saplings are published in a **nursery**, and a nursery is a git repository. It needs no service to run: a record is already git; a version is a tag; provenance is a commit; an upgrade proposed against what the host has ratified since is a diff between two tags; and a company that wants a private nursery forks one or starts its own.

- **The layout.** `nursery.toml` at the root indexes what the repository offers: for each entry its name, kind (`pack` or `sapling`), version, path, content digest, the toolchain it needs and a one-line summary. An entry may point at another repository and a revision instead of a path, so a sapling can grow into a repository of its own and stay listed.
- **One official nursery,** `hale-lang/nursery`. The starter packs are its first entries. The library the toolchain embeds today stays as the built-in baseline, readable with no network.
- **Adding a source is governance.** `hale dna nursery add <git-url>` proposes the source as a binding the Board ratifies, since what a nursery offers is proposed into the organization. `hale dna pack list` reads every ratified nursery and the baseline.
- **Pinned by content.** Applying an entry records the nursery, the commit and the entry's digest in the proposal, so what the Board ratified is exactly what was applied. A moved tag is a new version, never a silent change. Signed tags come later; the Review's preview is the check until then.
- **Discovery later.** A static index on the site can be generated from the official nursery for browsing and search; the repository stays the source of truth.

Saplings and grafting come after this plan; the nursery's format carries them from the start, so the first sapling is an entry, not a migration.

## Onboarding

From about fifteen commands to about five: `init --with <pack>`, one verdict, `dev`, `head`, `work login`.

- **`hale dna head`**, a toolchain verb: the head's sources are embedded in the toolchain as the host's are and built from the vendored copy, so no checkout and no `dna/face/start.sh`.
- **The head derives its memory DSN** from what `memory migrate` records, instead of an operator copying it.
- **`hale dna init --with <pack>`** proposes the pack planned on the holes init opens (they count as ratified for the plan, and the pack's ratification waits for theirs), so the pack may fill them and one `review all approve` ratifies the starting organization; plain `init` is unchanged. The `founder` pack (`fill * <founder>`) seats one person everywhere.
- **A starter model mapping** from what `init` discovers: a default rule and a category for each discovered backend, so a leg has a model without hand configuration.
- **The cycle is rewritten onto this path** and stays green three times running, printing how many commands it took.
- A hosted head's `record_status` and `record_history` need an owning-service visibility provider; that is its own design and waits.

## Memory refuses and moves on

A ratified graph proposal memory will not project (a malformed edge, a node id it refuses) is recorded as `graph.refused` with why, and the projection continues past it, as a refused hold already is. Today one such row stops the projection at that row and everything after it waits, so every hat read waits on a watermark that never comes. Batches multiply proposals; this lands first.

## Bounds that mean something

The memory-bound survey (`spec/verification.md`, memory-bound proofs) is advisory and on by default; `@unbounded` on a fn silences its sites. DNA carries 1,298 of them, 847 outside its tests. Of those: about 68% suppress nothing (they travel with refactors because `hale verify` already exits 1 on 309 inherited advisories, so nobody can tell a needed one from a ritual one); about 22% answer the survey's false positives; 6–8% mark real growth that an input bounds, with no way to say which.

The aim: an `@unbounded` remains only for growth nobody understands yet, with an issue; growth with a known bound says what bounds it; DNA's `hale verify` is green, then a required gate.

1. **Which decorators suppress nothing, judged across programs.** The survey judges a whole program, and DNA's core is compiled into about eight (host, organism, api, ui, the tests). `hale check --report-unbounded-use` lists, per program, the `@unbounded` decorators that silenced at least one site; `scripts/unbounded-unused.sh` runs it over every DNA target and the stdlib and lists those no program needs. A program warns of a decorator that suppresses nothing only for a fn its own seed declares; a shared library's is judged by the aggregate.
2. **Cleanup, first pass.** The listed decorators are removed in one mechanical change, landed when no DNA branch is in flight; every DNA target's allocation pins move once, by source spans alone.
3. **A value accumulated in a loop and returned** is charged once per invocation to the frame that receives it, not as growth of the fn's locus, unless the receiver keeps it (about 141 sites). The survey also recognizes the same shape however the concatenation is spelled; today it needs a literal or a declared `String` in the chain.
4. **A replaced `String` field** (`self.f = <new value>`) is modelled as retiring the value it replaces, the store-latest refinement the spec names, as distinct from appending (about 40 sites).
5. **One command per process.** The host answers one verb and exits except under `hale dna dev`, but the survey judges all of it long-lived, which is 285 of its warnings. The long-running loop moves into a program of its own; the verb program runs to exit and owes the survey nothing, as a script already does.
6. **Cleanup, second pass**, of what 3 to 5 made unnecessary.
7. **`@bounded_by(<name>)`** states what bounds a real growth (`@bounded_by(record)`, `@bounded_by(open_requests)`): it silences the survey as `@unbounded` does, appears in `--dump-alloc-summary`, and is what a test asserting growth proportional to that input checks. The name is free-form.
8. **Verify, green and gated.** With DNA's advisories answered, a CI job runs `hale verify` on core, host, operations and api, and is required. The house standard (`docs/src/dna/standard.md`) says: no `@unbounded` without an issue; `@bounded_by` for stated growth.

Every compiler change moves the allocation and placement pins; each change says which moved and why, as today.

## Beside it

- **CI flakes, fixed where they start.** `senses_reflex_test` takes a free port by probing it and releasing it before the child binds, so a parallel slice can take it between; `api_runtime::the_restart_program_runs_clean_under_asan` races under load.
- **Language gaps the standard surfaced**, noted and not scheduled: typed record rows rather than JSON strings, collections of values, enums, units for money and quantities, head reads as served surfaces, a typed heart. Several would retire whole classes of `@unbounded` and string plumbing; each wants its own design.

## Order

At most one change in CI and one more beside it; a compiler change pairs with a DNA one, except the two cleanup passes, which land alone.

| | DNA | compiler |
|---|---|---|
| 1 | the held branches; memory refuses and moves on | the issue; which decorators suppress nothing |
| then | | cleanup, first pass |
| 2 | batch apply, its preview and command; the runner on its parser | returned accumulators; replaced fields |
| 3 | the known-open scenarios; vacate; packs adopt and retire; the nursery and its starter packs; `Commands::fill`, `Commands::open_position` | one command per process |
| then | | cleanup, second pass |
| 4 | onboarding: `head`, the derived DSN, `init --with`, the starter mapping, the cycle rewritten | `@bounded_by` |
| 5 | assign, merge and split, expose and depend, externalize and internalize | verify green and gated; the standard updated |

## Decisions taken

Each was taken by recommendation and stands until overruled:

- the batch grammar is the scenario runner's step language;
- `init --with` plans the pack on init's pending holes rather than folding them into one Review: a hole's graph rows are not steps a batch writes, and each hole stays decidable on its own (taken when it was built, spec/dna.md);
- batches land before the onboarding changes, which build on them;
- the first packs are a dev team, a small startup and a team that ships no code;
- a nursery is a git repository indexed by `nursery.toml`; the official one is `hale-lang/nursery`;
- the held `claims/no-silent-loss` and `claims/routing-coverage` branches land;
- the host's long-running loop becomes its own program rather than gaining an annotation;
- `@bounded_by` takes a free-form name;
- `hale verify` becomes a required gate once DNA is green under it;
- the UI work delivered here is the commands, previews and clients; the habitat UI builds the screens.
