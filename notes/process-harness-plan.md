# The process harness and a changing organization — plan

The todo loop runs green from scratch on main: `init`, ratification, filling, one ask carried by the organization's own editor through plan, candidate, Review, local CI and apply, and a clean teardown, on canned models. Two of the kickstart walkthrough's six steps are still outside it (`notes/kickstart-plan.md`, The first user): a performer holding a position claims a Patch Work, reads its brief and submits a candidate (step 4), and a later commit is re-ingested into reviewed graph proposals (step 6). Behind both is the same gap: the organism's structure comes only from `init`, and nothing but its own editor can be handed work.

This note is the design that closes it, and the order it lands in. A thin **skeleton** runs end to end first; everything else waits until it does, and then proceeds in parallel behind its interfaces.

## The harness: hat and hands

A Work is one step of a process, and a code change is one kind of output among several. The baseline's definitions owe two today, `Patch` and `Assessment`; `output_contract` is otherwise a free string, which test fixtures fill with names such as `Report` or `approval` that nothing describes. The organism's harness is therefore a **process harness**, not a coding one, and an output contract becomes something declared (`dna/core/contracts.hl`), not a string a definition makes up. For one Work it is two things.

- **The hat** is the assembled context: the position's mandate and charter, the practices and knowledge drawn for the Work, the objective, the history, the output contract and how it is judged (its gates, the position that reviews it, its change class), the workspace for a Work that changes files (the repository, the base, what the grant may touch), and the model chosen for it with the rule that chose it. It stays structure, sealed by its digest, as today.
- **The hands** are the toolset composed for that Work: the tools to act with, and the validators of its output contract, run before the outcome is submitted. A validator in the harness is pre-flight. The owner runs the same validators on submit, and only the owner's verdict counts: the store is the gate.

One composer builds the harness for every performer: the organization's own editor, a leg working autonomously, and a person driving an agentic session of their own through the head. Today the hat is assembled in two places (the editor's in `dna/core/assembly.hl`, the head's `HatReader` in `dna/operations/context.hl`) and the editor carries its own tool grant; they become one. Tools execute on the performer's side, in the leg or the person's session, where the workspace is. A performer reads its harness only through the head.

### Output contracts

Each output contract declares its shape, its validators and its default hands. A `Patch` is validated by `hale check`, `hale test`, `hale fmt --check` and the gates its route names, and comes with the git and toolchain hands; an `Assessment` by its shape and its citations of ratified practices; a `Report` by its required sections and evidence digests; an `approval` by a signature within the holder's authority. A pack can ship new contracts with their validators and hands.

### The tool registry

The organism's tools are graph rows, `tool:<name>`, each carrying a spec in the shape of pond's `agent/tools` (`ToolSpec`: name, description, input schema), its effect class and the authority it needs, where it runs, and whether it acts, validates, or both. Edges place it: a position is `equipped_with` a tool, a contract is `validated_by` one, a tool is `provided_by` a provider and `reaches` an effect class. A Work's hands are its position's equipment, narrowed to the Work's grant, plus its contract's validators. Equipping a position is a reviewed operation like any other.

One registry is rendered three ways: as MCP tools for a person's agent (`hale mcp`), as tool definitions in a model's request, and as the `Hands` a leg holds. There is no separate infrastructure server; `hale mcp` rendering the registry is it, filtered by the caller's equipment and grant.

The toolchain ships two families.

- **Infrastructure, read-only.** The record's status, history and rows; the org chart and processes; memory's knowledge; the nerves' stream health; the senses' readings; the fleet and the body; the head's status; logs. A read keeps its rules: protected evidence is read only through the principal check, and the read is a row.
- **Research.** Web search and fetch, through a provider bound like any other (its key in the habitat's vault, replaceable). It carries its own effect class, `external_read`; a data-class gate keeps a Work that carries customer data from sending it out in a query; fetched content is marked untrusted in the hat and in the evidence, so it never reaches a model as instructions; a domain policy and a budget bound it; every query is a row.

New tools emerge as the organization builds them. A part that starts serving a surface (a mail gateway, say) already has its compiler-emitted description (GH #1104), which is a tool spec: `hale dna ingest` reads it, proposes a `tool:` node per operation (reads as reads, effects with their effect classes), the Board ratifies, and positions are equipped. Discovery is a query over the registry (`hale dna tools`, an MCP tool listing); nothing is registered by hand, and nothing is usable without a Review.

## Voice and the model mapping

Every model call goes through one OpenAI-compatible API: voice (hale-lang/voice), an organism of its own hosted in the habitat. Voice serves models and nothing else. A request's `tools` are forwarded to the model and its `tool_calls` returned untouched; a seat that cannot return tool calls refuses a request that carries tools. Voice runs no harness, holds no workspace and executes no tool.

Authority is not transitive. Voice issues its own keys, and a habitat holds one set of them (more than one only where billing differs, since a voice key names the account it spends). Voice enforces its own limits and budget for the habitat as a whole and never learns a position. Each call carries `metadata` for attribution only (the attempt, the position, the holder, the organism) and an `Idempotency-Key`.

Which model a Work gets is decided on hale's side, in categories along two axes.

- **Mode belongs to the harness**: `think` is one call; `tools` is a loop of calls over the hands without a machine; `do` is that loop with a workspace. By default the mode follows the Work: a `Patch`, or hands that include a workspace, is `do`; an `Assessment` is `think`.
- **Size belongs to the model**: quick, standard or deep, with capability flags (tool calls, context length). `tools` and `do` map only to models that return tool calls.

Two tables hold the mapping. The organism's maps a selector to a category, and is portable: a pack or a recipe carries it without naming a vendor. The habitat's maps a category to a model voice serves, and is swappable: upgrading every deep reviewer is one line. Selectors resolve most-specific first: the habitat's default, the organism, the position, the workflow step or output contract, the task (an override at the ask, or the Leader's plan). An override stays inside the position's permitted set and never widens it. Data class filters which models may fill a category; it is not a category of its own. The rules are rows, so they have history, and the resolution travels in the hat with the rule that made it.

## The operations of a changing organization

The substrate already has the general mutations, each reviewed: knowledge nodes proposed, revised and retired, edges linked and unlinked, bindings bound and unbound, a hold requested, a practice proposed, the organism's own source changed, a person retired. A named operation (`docs/src/habitat.md`, The shapes change) compiles to a batch of those, carries a change class, checks its invariants before it is proposed and is one Review; the library's families and the `holes` family already ratify many rows as one. What is missing is the vocabulary: a position, for one, can today come only from `init`'s hole proposer.

Operations are tested as composable units. A **scenario** is a sequence of operations and the organization expected at its end; a step is either a person's act or the organism's proposal, the latter a Leader's answer on a canned model that the operation's validator admits or refuses. After every step the same invariants hold whatever the scenario: the record replays to the same organization, every hold in force was ratified, every position has a mandate, no edge reaches a retired node, the change class matches who ratified, and a refusal names what failed. Operations not yet built are scenarios asserted to fail, in a known-open table as the ownership matrix keeps one; an operation lands when its entry goes. The same scenario file is a test, a **recipe** a Board can adopt as one Review ("a small dev team for a CRUD application"), and a worked example the advisory function cites.

## The advisory function

The organization changes shape well only if something advises it how, and the toolchain ships that advice the way it ships the language's.

- **The know-how is the evolution pack**: playbooks for evolving an organism and a habitat. When to open, split or merge a position; when to serve, place or externalize a capability; how to sequence a team, or a small company's first teams; how to read pressure and concerns as signals. Each playbook cites the scenarios that prove it. It is bound to `system:dna`, so it reaches every change of the organism's own class. Advice on the language stays in the shipped library (`language:hale`).
- **The advisor is the Leader**, already the organism's architect: it proposes and the Board decides. A part may have an architect position of its own, as `design/standard-equipment` provides; it reads the pack through its hat. Nothing is preinstalled: the pack creates no position and grants no authority.
- **Advice is an operation.** What the advisor proposes is a named operation whose invariants admit or refuse it.
- **Two proofs.** Scenarios prove that following the advice composes into a valid organization, mechanically and on every run. Whether the advice is good is a different question, answered by an evaluation set: organization states and pressures with the family of operation expected, run live from time to time, taped and graded.

The Leader's brief also gains the organism's situation: open Reviews and the Board's queue, concerns and pressure, work in flight, vacant positions, budget use, recent changes. It is a projection of the record, bounded, with its digest on the model call, so what the Leader saw is always recoverable.

## Context follows the graph

A performer's knowledge today comes from a fixed set of targets: the Work's locus path and its ancestors, the codebase's languages, `system:dna` for an organism change, the performer's position. Context becomes a walk from the position node along declared edge kinds (the parts it is assigned to, the contracts they meet, what they depend on, the packs adopted, the holders), inside the existing budget and data-class gate, in a deterministic order. Each edge kind declares the direction knowledge flows along it, so a walk keeps the tower's rule: goals flow down, initiatives stay local, siblings never coordinate privately (H9). The operations above are what create these edges.

## The skeleton

It is done when one cycle from scratch, on canned models, is green three times running:

1. `init`, ratification, filling.
2. A position opened through the operation layer, with its mandate and a model-mapping default.
3. An ask routed to that position as a `Patch` Work, delivered three ways, each ending applied: by the organization's own editor; by a leg running its own tool loop (hat, hands, a model through an OpenAI-compatible endpoint); by a scripted person session over MCP, with the hat and hands the head serves.
4. One `Assessment` Work done by a leg in `think` mode.
5. `hale dna ingest` at a later commit, filing the graph's diff, including the tool nodes of the application's surfaces, as reviewed proposals.
6. A teardown that leaves nothing.

| | piece | done when |
|---|---|---|
| S1 | output contracts, the tool registry and the hands composer: one hat assembler; `Patch` and `Assessment`; built-in tools (read, edit, check, test, fmt, the git patch) and a thin read-only infrastructure slice (the record's status and history, the org chart) | the editor's hat and the head's come from the one composer, and the existing suite holds |
| S2 | the model mapping: categories, the rule rows, the resolver, a verb to set rules, the resolution in the hat; `OpenAiChat` sending `metadata` and an idempotency key; a scripted OpenAI-compatible server for canned runs | the hat names the model and its rule; canned calls cross real HTTP |
| S3 | the leg's tool loop in the three modes over S1's hands and S2's model: evidence per model call and per tool call, the allowance before the first call, a cost ceiling and a turn limit | a leg completes a `Patch` Work and an `Assessment` Work, canned |
| S4 | routing to positions: positions carry performer kinds; `task create --to position:<name>`; a leg claims a `Patch` Work | path B of step 3 green |
| S5 | the person's session: the head serves the hands' definition beside the hat; `hale mcp` makes them local tools | path C of step 3 green |
| S6 | the scenario runner and the first operation, opening a position; the known-open table | a dev-team scenario green; the open operations fail as listed |
| S7 | `hale dna ingest [--at <rev>]`, tool nodes from new surfaces included | step 5 in the loop |

Thin on purpose in the skeleton: a leg holds the habitat's voice key from the local vault, and the model that served a call is checked against the permitted set when the Work settles; mapping rules are set by an operator through the command line, as rows; canned runs use the scripted server on hale's side; context keeps today's fixed targets.

## After the skeleton

In parallel, behind the skeleton's interfaces:

- the operations, each leaving the known-open table: vacate, adopt and retire a pack, merge and split positions, expose, depend, externalize, internalize;
- the scenarios: a team that ships no code (`Report` contracts, schedules, receipts), then a small company's first teams; recipes;
- context as a walk over the graph;
- the Leader's situation;
- the evolution pack and the evaluation set;
- the infrastructure family whole, then research with its safety rules;
- model calls forwarded by the head (voice's key never leaves the habitat's trusted side), keys cut per attachment, the habitat UI for the mapping, voice's replay seat keyed by the content of a call;
- saplings and grafting, which span several records and come last.
