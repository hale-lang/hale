# F.40 phase 3 — plan

**Anchor:** hale-lang/hale#1212, its "Final direction" comment (2026-09-29), the phase-2 exit comment (2026-10-01), and the registry (`spec/registry.md`) at `main` ee992190. Where this plan and the issue body differ, the Final direction wins; where this plan names a row, the registry's text is the contract. Revision 2 folds the outside review of this plan (PR #1290, six findings), which changed the lifecycle oracle (§3 L1–L2), split extraction from correction (E3, P1), gave the editor step a real stage boundary (X1), fixed the target gate (P3), added the entry producer as a prerequisite (E0), and replaced the closure count with the row manifest in §10.

**What this document is.** The planning pass for phase 3, written for review before any pane starts, in the shape phases 1 and 2 were planned: where phase 2 leaves the tree, what phase 3 is and is not, the lines of work with their steps, and how the work is driven. It is written so that a driver can run it step by step from the briefs it specifies, with the model chosen per job rather than per phase (§6).

## 1. Where phase 2 leaves the tree

- One loader and one desugar sequence before the check; one snapshot per entry point (`hale_frontend::snapshot::Snapshot`) whose derived families are demanded, memoized and counted (`builds()`); the model, the artifact, the api description, the build identities, the editor's diagnostics and every editor request, and the checker's handler rows read it. Use-site identity: `Expr::Ident` is a minted `Use` site, `binding_of` resolves each use to its declaration once, and the four returned-binding walks read it.
- Registry: 44 families, 4 Canonical (`borrow_lifetime`, `model`, `seed_loading`, `working_set`), 36 Migrating with 150 legacy rows (§10 lists every one), 4 Reserved.
- Measured at the close: bench 29 of 29 within band against v0.22.0; the corpus AddressSanitizer oracle clean (91 fixtures); `hale check dna/host` 1.71 s → 1.92 s and 161 → 192 MB; `hale build dna/host` 30.4 s → 27.3 s; editor latency on `dna/host/main.hl` 53 ms → 1.9 s (the whole-seed parity's price: load 0.10 s, scope 0.02 s, the check 0.9 s, the model 2.8 s when a law demands it; a burst of edits costs one check).
- The three maintenance exercises: #1199 (handler routing) and #1210 (binding identity) moved to one producer each; #1208 (scratch-local fixpoint and teardown order) did not move at all, by plan: the lowering view and the snapshot carry no row for the entry point, the lifecycle order or allocation routing. That exercise is phase 3's measure.
- Carried from phase 2's exit and its review: the checker's two judgment migrations (unowned subscriber over the ownership graph; rules 9 and 10 over the bus graph), each measured to change diagnostics; the lowering view's own graphs (`resolved.rs` builds a scope, an ownership graph, a bus graph and handler rows over the merged program); sync inference as a pass of the sequence; SiteId joins in the model; the editor's incremental check; `document_symbols` parsing its own buffer; the harness adapter; the api binding's generated ingress (fixed in #1291).
- Known, classified divergences that phase 3 inherits rather than discovers: the placement shadow's fixture (`crates/hale-types/tests/fixtures/shadow_placement.txt`) records the checker's and the graphs' disagreement on nested inherited placement and on qualified types; the registry's `alloc_summary` row `ReclaimScope` says the checker's reclaim model is stale for scratch-local free fns (they reclaim their non-escaping allocations at return, as `scratch_local_free_fn.rs` pins; the model says `EnclosingLocus`); the `placement` row `budget_for_programs` says the resource budget ignores replicas; the `entrypoint` rows disagree on an imported `main` and a module-nested `main`.

## 2. What phase 3 is, and is not

The Final direction names what remains of F.40 after one pipeline: **lifecycle normalization** ("an explicit plan of compiler- and runtime-owned actions … a debug-build assertion checking the cascade against it, and the existing lifecycle regressions"; the first named decision: handlers run only on the queue owner's thread, so cross-thread failure delivery follows `spec/runtime.md`; #1208's second crash lives in this code), the **layer-4 fixpoints as families**, **consolidating today's wasm restrictions into one capability matrix**, and the structural laws registered with evaluators as background work. Phase 3 is those, plus the carry-overs above, delivered the way phases 1 and 2 were: one family at a time, behind a shadow, consumers switched one per commit, the old path deleted, the registry row closed with its code; and, where a row names a known defect, an explicit correction with its own decision, test and review, never a correction hidden inside a move.

Not phase 3, by the Final direction: `@evented` (F.41), the unit dialect, a `Backend` trait beyond the wasm matrix, a second emitter, footprints, storage bindings, the authority edge, the UI consumer. Not phase 3 by size: the stdlib dispatch tables (`stdlib_surface`), the identity digests (`digests`), the api surface and claims rows, the closure and sealability laws, and the laws program; §10 names each leftover row.

## 3. Lines of work

Four lines run in their own worktrees, after the prerequisites. Each step names its job shape (§6), the model that does it, its oracle, and the registry rows it closes (the manifest in §10 is the authority).

### Prerequisites (week one, before P1 and L1)

| step | work | shape · model | oracle | closes |
|---|---|---|---|---|
| E0 | **The entry row.** `entrypoint` has no producer today, and its twelve legacy definitions disagree: the checker's four finders (one defines an imported `main` by `l.imported`, one filters on a `__lib_` name, one takes the last of several with no filter, one treats a module-nested `main` as disabling rule 9), the matrix's parse-only finder, the snapshot's `has_main`, and codegen's six comparisons. The producer: one `entry` row on the snapshot (the main locus by identity; whether it is imported; whether it is nested in a module) with the decisions named for the two disagreements (an imported `main` is / is not the entry; a module-nested `main` is / is not the entry, and what rule 9's closed world then is), each a decision line with the current answers listed, settled before the pane starts, with a wording-pinned test per decision. P1 and L1 consume this row; L4 switches codegen's six comparisons. | D then S · the driver writes the decisions; Opus pane implements | diagnostics identical over the corpus and `tests/hale` except where a decision says otherwise (pinned) | `entrypoint` × 6 (the checker's four, the matrix's, the snapshot's); the codegen six close in L4 |
| L0 | **The lifecycle inventory and the first named decision.** Read-only on code: one table, one row per lifecycle action the compiler emits or the runtime performs (init, params settle, accept, birth, run start and completion, failure delivery with its execution domain, hold / settle / defer / await, drain, wait-abort, quiesce, pool join, reclaim prerequisites, dissolve cascade, arena destroy, restart, resume, teardown join, process exit): who owns it (emission site or runtime entry point), the domain it runs on today (measured from the code), prerequisites, what must stay alive across it, completion, what `spec/runtime.md` says, agree or not. Every `lotus_*` lifecycle entry point and every codegen spine site appears once; the five copies of the teardown spine are five rows that must agree with each other. The first decision goes into `spec/runtime.md` § Failure handling as a decision with a regression test name: handlers run only on the queue owner's thread; cross-thread failure delivery follows the spec. Every other disagreement is a decision line for Riley. | D · the driver fixes the table; Opus pane reads and fills it; Astra reviews the spec PR | the table is complete (row count equals the registry's sites plus the runtime entry points) | none; it is the contract |

### Line L — lifecycle normalization (layer 6)

The layer that does not exist. Today the birth sequence is the order of emit calls in `lower_locus_instantiation_inner` (a 5,723-line file), the teardown spine exists five times, the reclaim spine, the dissolve cascade and restart are each their own walk, and the hold/settle/defer/await protocol lives in the C runtime (`lotus_failure_hold` and its siblings, `lotus_arena.c` ~2061–2113), where a handler being delivered is distinguished from its having returned, and only the latter releases the payload and resumes or reclaims the child.

| step | work | shape · model | oracle | closes |
|---|---|---|---|---|
| L1 | **The action plan as a family of obligations.** `lifecycle_order`'s producer in the frontend: one row per (instance, incarnation, action) stating the obligation, not only the order: the subject (the instance by identity, and its restart incarnation), the path guard (normal; failed at params settle; failed in run; restart; drain in flight), the execution domain, the prerequisite completions (which earlier actions must have completed, not started), the retained resources (what must stay alive until which completion: the child and the failure payload until the handler completes; the arena until the last reader's completion or the pool join), the completion or cancellation transition (an action that spans execution has a start and a completion; shutdown that deliberately abandons execution has a terminal cancellation or quiescence), and the required-event multiplicity (once per incarnation; never twice). The laws are guarded by path: a child that fails during params settle never reaches run; a restart repeats the guarded subsequence; there is no unconditional birth-run-drain-dissolve sequence for every instance. Inputs exist: the ownership table, handler routing rows, placement rows, the entry row (E0). `hale check --dump-lifecycle` renders it (a development dump, never in the hashed model half). | D then S · driver fixes the obligation schema and the path guards in the brief; Opus pane implements | the plan's own laws over the corpus (acyclic prerequisites; every obligation's completion names an action that exists on that path; every cross-domain delivery names its domain); L2 is the real oracle | the family gets its producer; rows close in L4 |
| L2 | **The trace oracle evaluates the obligations.** A `LOTUS_LIFECYCLE_TRACE` debug build emits correlated events (start and completion of each spanning action, terminal cancellation or quiescence, destruction) with the instance's identity, its incarnation and the thread; the harness checks an execution against L1's plan: every required event present exactly once per incarnation (coverage and illegal duplication), every completion before the obligations that depend on it, no destruction with an outstanding reader or an undelivered or unfinished delivery, every domain claim against the thread the action ran on. Logging adds no synchronization that could mask the race under test, and a serialized log order is evidence of an execution, not a proof of a happens-before edge; progress claims (a join that completes, a wakeup that is not missed) stay with the deadline and matrix oracles. Negative controls, each failing the trace oracle independently of ASan: the pool join removed (the deferred-pool-join regression's shape), a child reclaimed before its handler completes, a required completion event omitted. | V · Opus pane; the harness shape follows `corpus_oracle.rs` | the harness passes over the 91 runnable fixtures and fails each negative control | `lifecycle_order` × 1 (`lotus_failure_hold`), when the oracle holds over the corpus and the matrix |
| L3 | **The lifecycle matrix.** A generator in the shape of `ownership_matrix.rs`: axes failure phase (params settle, birth, run, handler, drain) × tree position (root child, grandchild, accepted, interface-typed, perspective-typed) × placement (same thread, cooperative pool, pinned, cross-pool) × lifetime (flow, resident) × recovery (none, restart, quarantine, bubble, drain in flight); four oracles: leak (dissolve tag counts and `LOTUS_ARENA_RESIDENCY`), ASan with chunk pooling off, deadline (no deadlock), trace-versus-obligations (L2). `KNOWN_OPEN` lists the cells that fail today and asserts them to fail; a ~100-cell sample in CI under a minute, the full matrix nightly. | V · driver fixes the axes; Opus pane builds the generator | the sample is green with `KNOWN_OPEN` naming today's defects | none; it is the oracle |
| L4 | **Emission reads the plan**, one spine per commit: the birth order, the deferred entry teardown, the frame flush, the three `fn main` exits, the reclaim spine, the dissolve cascade, restart and resume; codegen's six `main` comparisons read the entry row. Each commit is shadowed by the corpus binaries (the trace under L2 identical before and after) and by the matrix sample. | S · Opus pane (34,900-line `codegen.rs`; a reader pane watches each commit) | trace shadow identical; matrix sample green; corpus binaries' outputs identical | `lifecycle_order` × 9, `entrypoint` × 6, `restart` × 2 |
| L5 | **The failure-delivery protocol and #1208's second crash.** Against the plan: construction-time delivery before the owner's mailbox is active, retention of the child and payload until the handler completes, progress during drain and join (a parent waiting for a child must not deadlock with a child waiting for its parent). Heap-backed payload tests, exactly-once teardown, deadlines, execution-domain assertions, ASan. The GenMC model (`verification/cascade_model.c`) states its proof boundary as the verification guide requires: the production functions and synchronization it mirrors (`lotus_failure_hold`, settle, defer, await; the mailbox), the bounded configurations, and the safety assertions it checks; condition-variable liveness is excluded there, so missed-wakeup and join-progress claims are carried by the deadline and matrix evidence, not the model. | J · driver writes the decision text; Opus pane implements; Astra reviews before merge | the matrix cells for cross-pool failure flip from `KNOWN_OPEN` to green; the GenMC model passes within its stated boundary | the regressions named in L0's table |

Size: L0 one pane-day; L1 two; L2 two; L3 two; L4 four to five; L5 two to three.

### Line E — effects as families (layer 4)

| step | work | shape · model | oracle | closes |
|---|---|---|---|---|
| E1 | **One effects fixpoint per snapshot.** `demand_effects`: the callgraph and resolved call targets, known effects with incomplete coverage kept distinct from violations (an unresolved edge never erases a known effect), purity as a column, the lower-bound variant as a second result of one walk. Consumers switched one per commit: the model's `infer_effects`, the manifest and replay's live-effects gate, the certificate evidence (computed once), the frontier law, the claims' direct-effect predicates. | S · Opus pane for the producer and the first consumer; Sonnet for the remaining switches | effect manifests, certificates and the model dump identical over the corpus and DNA seeds | `effects` × 6 |
| E2 | **Blocking and non-returning read the rows.** `blocking_free_fns` and `blocking_self_methods` read the effects rows' BLOCK class; `program_has_offthread` and `has_offthread_placement` read the placement table (after P1); the starvation and birth-order phases become laws over rows; the second `long-running` predicate goes. | S · Sonnet pane after E1 and P1 | diagnostics identical over the corpus and `tests/hale` | `blocking` × 4, `nonreturning` × 2 |
| E3a | **One allocation summary per snapshot, and the routing fixpoints extracted faithfully.** `demand_alloc_summary` (twelve builds per check become one; the check's and the LSP's entries one); the hot-path lint a law over the rows; then the codegen fixpoints as rows lowering reads, one per commit with its binary shadow: `compute_scratch_local_free_fns` with `SCRATCH_LOCAL_BUILTINS`, `SCRATCH_LOCAL_STD_NAMESPACES`, `ScratchPaths` and `current_user_fn_scratch_local` (the #1208 fact, moved as it is), `compute_nonalloc_free_fns`, `compute_elidable_methods` with `method_scratch_elidable` and `locus_arena_elidable`, the TLS publish gate's Debug-string decision. The extraction commits preserve execution and the dumps: the alloc summary dump identical, the corpus binaries' outputs identical. | S · Opus pane for the summary-once and scratch-local commits; Sonnet for the rest | summary dump identical; binaries identical; matrix sample, reclaim tests and the corpus ASan oracle green | `alloc_summary` × 13 |
| E3b | **The reclaim model corrected.** The `ReclaimScope` row names a known defect: a scratch-local free fn reclaims its non-escaping allocations at return, and the checker's model says `EnclosingLocus` for every non-sent allocation. The correction: one model, consistent with the scratch-local classification E3a moved, with the expected dump change pinned per site (which sites move from `reclaim@locus-dissolve` to reclaim at the fn's return) and any advisory wording change pinned; `spec/verification.md` § memory-bound proofs says the rule. | J · driver writes the decision; Opus pane implements; Astra reviews | the pinned dump diff and nothing else; binaries identical (the model is diagnostic) | `alloc_summary` × 1 (`ReclaimScope`) |
| E4 | **The resolved program carries the checker's types.** A typed-body table lowering reads instead of re-inferring: accumulator element types, generic argument inference and param unification, the monomorph table keyed by identity per snapshot, the form's sync mode, interface conformance for storage routing, the fallible-call law once stdlib calls are typed. | S · Opus pane (the table's shape is the driver's) | corpus binaries identical; `generic_monomorph_agreement` and the matrix green | `expression_typing` × 1, `generics` × 3, `forms` × 1, `surfaces` × 2, `bare_fallible` × 1 |

Size: E1 two pane-days; E2 one; E3a three; E3b one; E4 three.

### Line P — placement, bindings, targets (layer 5)

| step | work | shape · model | oracle | closes |
|---|---|---|---|---|
| P1 | **One placement table per snapshot, with a classified correspondence to each legacy producer.** `demand_placement`: a table keyed by instance identity (locus site, field path, replica index) with the domain, the placement block that decided it, owner-relative placement, and the witness; `DeploymentPlan` becomes its lowering view. Before the pane starts, the driver writes the correspondence: for each of the eight legacy producers (the checker's four derivations, the bus graph's and the ownership graph's copies, the model's `PlacedIn`, the desugar's read, codegen's `collect_main_placement` and `DeploymentPlan`, the resource budget's walk), which answers the table keeps, and for the known divergences the placement shadow fixture already classifies (nested inherited placement; qualified types) and the resource budget's replica accounting, the approved correction with its spec witness and its test. The gate is zero unclassified divergences; an approved correction is its own J commit with wording-pinned tests, reviewed; the extraction commits preserve execution and the external contracts (no `shape_hash` moves). Consumers switch one per commit: the checker's rules, the two graphs, `PlacedIn`, the desugar's read, `DeploymentPlan`, the resource budget. | D then S, with J commits for the corrections · driver writes the correspondence and the decisions; Opus pane | the correspondence holds over the corpus, `tests/hale` and the DNA seeds; the pinned corrections and nothing else | `placement` × 8 |
| P2 | **Binding rows.** The binding-role rule once (`binding_role_for`), the bound-topic set once, the transport capability table a row of the matrix (P3), codegen reads the binding rows for transport, adapter, codec and producer-versus-attach; the transport-loss handler named by the bindings family. | S · Sonnet pane after P1 | binding diagnostics identical; `binding_ingest` and replay tests green | `bindings` × 5, `handler_routing` × 1 |
| P3 | **One capability matrix, compared under the same effective target.** `CapabilityMatrix` (target × capability → lower or reject, with the witness) consulted by the driver before lowering on every entry point: the wasm-unavailable stdlib table, the `wasm_target` source flag, `link_wasm`'s refusals and export list, the per-site codegen skips, the `is_wasm` sites, FFI type portability, `TargetSpec`. The oracle: old and new agree under the same effective target (source-declared, then the CLI's `--target`, with precedence tests); `hale check` and `hale build` agree on target admission under equivalent source and configuration; paired cases, one program allowed natively and rejected under wasm, with pinned witnesses; a portable subset of the corpus whose results agree across targets; the wasm capability statement in `docs/src/systems/webassembly.md` generated from the matrix, with a test that the doc equals the rendering. Bringing the refusals `hale build` makes today into `hale check` is an intentional diagnostic change: a decision line, a fragment, a J commit reviewed. | D then M, with one J commit · driver fixes the matrix's shape; Sonnet pane consolidates the sites; Opus for the J commit | the paired cases and precedence tests; the portable subset identical across targets; the doc test | `target_capability` × 7, `dispatch` × 1 (`bus_payload_is_flat` as a gate column) |

Size: P1 three pane-days (one for the correspondence); P2 one; P3 two.

### Line C — carry-overs and small closes

| step | work | shape · model | closes |
|---|---|---|---|
| C1 | Sync inference a pass of the sequence: the inferred discipline a row lowering reads (no AST mutation), one `has a sync discipline` predicate over the form rows (the two predicates disagree on `sync = none`; the shadow decides, and a divergence is a named decision). | S · Opus | `desugar_sequence` × 1, `sync_inference` × 3 |
| C2 | Qualified names: the seed names the library (`lib_canonical_id`), one resolution of qualified bus subjects shared by the checker and the resolved step, `imported_fn` by identity, `construction_target` gone. | S · Sonnet | `qualified_names` × 5 |
| C3 | SiteId joins: `fn_rows` and `SupervisedRef::External` keyed by the row's site; `settles_failures`, `failure_handler_for`, `resolve_failure_route` as columns of the routing rows and the instance tree; a monomorph's rows by its template's identity (`parent_accepts_us`; round-3 finding 16) and a generic supervisor's `Failure` row substituted at instantiation or refused at compile time (finding 17); the model's `Owns` projected from the ownership table; `FunctionId`, `FnKey`, `type_expr_key` by identity. Touches `model_builder.rs` after E1 and P1 have landed theirs. | S · Opus | `handler_routing` × 5, `ownership` × 2 (`Owns`, `parent_accepts_us`), `snapshot_identity` × 3 |
| C4 | The checker's judgment migrations, with spec text first: the unowned-subscriber rule over the ownership graph (an aliased accept type stops erroring, a module-path and a generic accept type start; duplicate loci by declaration order), rules 9 and 10 and `external_subscription_handlers` over the bus graph (the walk's bound, cross-seed and wildcard facts become graph columns; the canonical subject key; handler bodies per declaration). Each is a decision line for Riley before the pane starts; each lands with wording-pinned tests and a fragment. | J · driver writes the decision and the spec paragraph; Opus pane; Astra reviews before merge | `bus_graph` × 3, `ownership` × 1 |
| C5 | The lowering view's graphs fold into the snapshot: `resolved.rs` builds a scope, an ownership graph, a bus graph and handler rows over the merged program; the snapshot's families become the lowering view's inputs through an explicit correspondence (the merged program's sites are the checked programs' sites plus the stdlib's), shadowed; the ownership table carries both relations (`accepts_ancestor`, `owner_of_site`) and the borrow-lifetime law reads `accepts_ancestor` instead of rebuilding the accept sets. `extend_fresh_factories` stays lowering-only unless the checker reads the extended set: a decision line; the row closes only on a yes. | S · Opus | `bus_graph` × 2, `ownership` × 2 (`build_ownership_graph`, `accepts`), `top_scope` × 1, `dispatch` × 1 (`from_gates`); `ownership` × 1 (`extend_fresh_factories`) conditional |
| C6 | Small closes with a guard each, one PR per batch: `law_backstops` (the spanless lowering refusals are dead under one pipeline: delete with a test that the checker refuses first); `demand`'s `document_symbols`; `collect_known_names`; `topics` (codegen reads the topic rows); `flows` (codegen reads the flow row; the accept/release survey a law); `effect_class_table` (one table keyed by identity); `bus_inert` (a structural verdict row shadowed against the Debug scan; the `user` copy goes); `intra_locus_publish_target` (lowering reads `LoweringView::intra_locus` by the call's id, shadowed); the harness adapter builds from a loaded seed. | M/S · Sonnet | `law_backstops` × 1, `demand` × 1, `top_scope` × 1, `topics` × 3, `flows` × 2, `effect_class_table` × 2, `bus_inert` × 3, `bus_graph` × 1, `desugar_sequence` × 1 |

### Line X — the editor's price

Phase 2 made the editor answer `hale check` exactly and pay its price (1.9 s on the DNA host). `demand_check` today returns the typing diagnostics, the model's law judgments and the build rules together, demanding the model synchronously for a claim-bearing program; two notifications after that return would shorten nothing. The step therefore starts with the API.

| step | work | shape · model | target |
|---|---|---|---|
| X1 | **A real stage boundary, then two publications.** The snapshot splits the check into `demand_typing` (parse, scope, typing, the build rules, the allocation advisory: everything that needs no model) and `demand_laws` (the model and the claims' judgments), with `demand_check` the composition the CLI keeps, so parity holds by construction: the editor's final publication equals the CLI's set, ordered and deduplicated as today, with today's suppression rules. The editor publishes after `demand_typing` and again after `demand_laws` (the second replaces the first per file); a publication for a superseded document version is discarded; dependency files whose findings disappeared are cleared, as today. Parsed reuse: a per-(path, text digest) cache of the unshifted, unshaped parse product, from which the snapshot recreates its source mapping and mints its identities (never a shaped or minted member reused across revisions; the whole-snapshot key is not a per-member key). First and final publication are measured separately on a claim-bearing DNA seed, with the parity tests landing before the implementation. | P, with a D commit for the API · driver fixes the two families' contents; Opus pane; the latency script from the exit | `dna/host/main.hl`: first publication ≤ 1.0 s on open and on edit; the final publication no slower than today; hello-world unchanged |
| X2 | **Incremental check by declaration** (stretch; starts only if X1 lands under target): a changed declaration re-checks itself and its dependents through the families' identities, the rest reused. The snapshot key stays; a second key names the reuse. | D then P · driver designs the dependency rows; Opus pane | final publication ≤ 300 ms on an edit to the DNA host |

## 4. Order, parallelism and file ownership

1. **Week one:** E0 and L0 (the entry row and the lifecycle inventory, both decision-bearing, the driver's first), E1 and C6 in their worktrees; P1's correspondence written by the driver while E0 lands; P1 starts when E0 has merged.
2. **Week two:** L1–L3 (the plan, the oracle, the matrix) while E3a, P2 and P3 run; C1–C3.
3. **Week three:** L4 (the spines, one a day) and L5; E2, E3b and E4; C4 and C5 after their decisions; X1.
4. **Close:** exercises re-measured (#1208 must move: the lifecycle fact in a row, the scratch-local fact in a row), the bench against v0.22.0, the timing and latency tables, the registry's state against §10, the exit comment, a prerelease tag on Riley's ask.

Shared files, and which PR establishes each surface (the registry is not the only one):

| file | steps that touch it | order and the owner of the surface |
|---|---|---|
| `crates/hale-frontend/src/snapshot.rs` (the demanded families) | E0 (`entry`), E1 (`demand_effects`), P1 (`demand_placement`), L1 (`demand_lifecycle`), E3a (`demand_alloc_summary`), X1 (`demand_typing` / `demand_laws`) | the family pattern exists (phase 2); each step adds its own cell and `FAMILIES` entry in a commit of its own, and rebases over the others |
| `crates/hale-types/src/model_builder.rs` | E1 (`infer_effects` goes), P1 (`PlacedIn` projected), C3 (`fn_rows`, `SupervisedRef::External`, `Owns`) | E1 first, then P1, then C3 |
| `crates/hale-types/src/check.rs` | E0 (the four finders), P1 (the placement rules), E2 (blocking), C6 (`collect_known_names`), C4 (the judgment migrations) | E0, P1, E2, C6, C4 |
| `crates/hale-codegen/src/codegen.rs` and `locus/*` | E3a (the fixpoints and `current_user_fn_scratch_local`), P1 (`DeploymentPlan`), C6 (topics), L4 (the spines and the `main` comparisons) | E3a, P1, C6, then L4; two lines never hold open commits in `codegen.rs` at once |
| `crates/hale-types/src/resolved.rs` (the lowering view) | E3a, E4, C5, L1 (the plan as a view input) | each adds a field in its own commit; C5 last |
| `crates/hale-graph/src/registry.rs` and `spec/registry.md` | every step | a step's registry edit is its last commit; the integration owner rebases daily |

WIP cap stays: one PR in CI and one beyond; panes work ahead on stacked branches, as in phase 2, and a stacked PR is opened against `main` only when its base has merged.

## 5. Exit criteria and measurement

The RFC's acceptance lines phase 3 is responsible for, each with its evidence:

| result | evidence |
|---|---|
| zero decisions in lowering for the migrated families | each family in §10 marked "phase 3" is Canonical, which requires its final production derivation removed, not only its rows closed; a missing required row a `CodegenError` with a test per family |
| one derivation per migrated graph | the guard; `builds()` counts match the producers that ran |
| the lifecycle obligations hold | L2's oracle over the corpus with its negative controls failing; the matrix sample green with `KNOWN_OPEN` naming only cells L5 leaves, and the reason per cell |
| the runtime verified against the plan | L2 over the corpus and the matrix; the GenMC cascade model within its stated boundary |
| the LSP reports every layer-4 diagnostic | the parity fixtures extended with an effects and an allocation finding |
| the wasm statement generated from the matrix | the doc test; the paired-target cases |
| every shadow deleted | the shadow facility's call sites outside tests are the registry's allowance only |
| the three exercises | re-measured by the phase-1 method; #1208's copies 5/5/3/5 → 1 and its reading region halved is the target |
| no attributable regression | the bench against v0.22.0; the timing and latency tables beside phase 2's, first and final publication separately |

## 6. Driving the plan: the right model for the right job

Phase 2 showed what each kind of step needs. The plan classifies every step by its job shape, and the shape, not the phase, picks the model and the review.

| shape | what it is | oracle | who does it | who drives it |
|---|---|---|---|---|
| **M** mechanical | a fully specified transformation the guards check: registry re-horizons and seam counts, signature threading, a verbatim move, a deletion once a guard proves no caller, spec and doc sync, fragments | build, registry guards, existing tests | Sonnet pane | any driver, from the brief alone |
| **S** shadowed move | a derivation becomes a family producer, or a consumer switches to the rows; known divergences are classified before the pane starts (the shadow fixtures' existing classifications and the registry's "known defect" rows are the list); the gate is zero *unclassified* divergences; an unclassified one stops the step and is reported, never papered over | shadow identical except the classified list; suites; matrix sample; ASan where allocation moves | Opus pane for the producer and the first consumer; Sonnet for later consumers once the pattern exists in the tree | the integration owner for the producer; any driver for consumer switches |
| **J** judgment migration | the move changes a diagnostic or a fact by design (a classified correction): a spec decision is written first, the tests pin the wording and the expected fact or dump change, the fragment says what changed | wording-pinned tests; the decision in the spec; an outside review before merge | the driver writes the decision; Opus pane implements | the integration owner, with Riley's decision |
| **D** design | a table or plan that does not exist yet (the entry row, the lifecycle obligations, the capability matrix, the typed-body table, the two check families, the dependency rows) | the table's own laws over the corpus; the oracle that reads it | the driver fixes the row schema, invariants and oracle in the brief; Opus pane implements | the integration owner |
| **V** verification | a matrix generator, a trace oracle with its negative controls, a GenMC model with its stated boundary | the oracle passes over the corpus, names today's defects, and fails its negative controls | Opus pane, with the existing generator or harness as the template | the integration owner fixes the axes and the controls |
| **P** performance | a measured target on the latency or timing table | the table, first and final separately where the step has stages | Opus pane with the measurement script | the integration owner sets the target |
| **R** review | every PR, within minutes of opening (Astra, on GitHub) | findings fixed on the branch with a test before merge | Astra | the integration owner answers or fixes |

Rules the phases taught, kept:

- **A brief is complete or the pane stops.** The brief names what to read first (files and line ranges), the facts measured on the tree, the classified divergences it may see, the change per commit with its title, the verification commands to run and paste, the stop rule, and "commit verified state first" for a session that runs long. A pane never decides a judgment migration or a correction; it reports.
- **The driver owns the architecture, the decisions and the shadows.** A pane delivers thin slices; the row schemas, the invariants, the correspondence tables, the decision text and the exit measurement are the driver's. Side findings are one Deferred line in the PR body, never a new issue.
- **Every PR is a document** (What lands, Architecture with the invariants kept, Tests, Deferred), carries its fragment in a second push, is held while its review runs, and merges on green by the driver. One PR in CI and one beyond.
- **No attribution lines, no build in the main checkout, every worktree its own target directory, no binary ever tracked** (`gitignore_has_no_binary_rules`: build with `-o`, and never add an ignore rule for a built binary).
- **Measure before and after**, under the same conditions, on a quiet machine, with the scripts kept where a reboot cannot take them.

A Sonnet driver can run line C and every M-shaped and later-consumer S-shaped step of E and P from this document and the briefs it specifies; the D, J, V and P steps and every producer stay with the integration owner on the larger model. Astra's review covers every PR either way.

## 7. What phase 3 leaves

- Phase 4 lines, each row named in §10: `stdlib_surface` (the stdlib call literals become registry-driven dispatch; the printable predicate and the builtin declarations once), `digests` (one stated coverage per identity, derived from the snapshot identity), `api_surface` and `claims` (one surface per snapshot with the configuration as input; admission reads verdicts), `closures` and `sealability` (the lifecycle alphabet law; the sealability survey), and the laws program (every structural rule a registered law with an evaluator through `model_query`; rule 6 first).
- Extensions with their own reviews: F.41 `@evented` (on L's lifecycle envelope and E1's effects), the unit dialect, the `Backend` trait beyond P3, the UI consumer, deployment.

## 8. Decisions for Riley before the lines start

1. **The lifecycle decision** as the Final direction states it (handlers run only on the queue owner's thread; cross-thread failure delivery follows `spec/runtime.md`): confirmed as written, or amended. L0's inventory may surface more; each comes back as a line.
2. **The entry decisions (E0):** an imported `main` is or is not the entry; a module-nested `main` is or is not the entry (and rule 9's closed world under each).
3. **The corrections phase 3 makes explicit:** the reclaim model for scratch-local fns (E3b), the placement divergences the shadow fixture classifies and the resource budget's replica accounting (P1), the build-only target refusals reaching `hale check` (P3), the two checker judgment migrations (C4), the sync-discipline predicate (C1). Each proceeds on a yes; a no keeps the old answer as a narrower row.
4. **A prerelease tag after phase 2** (explicit ask only).
5. **Tracking:** the Final direction says phases 3 onward reopen as their own issues; this plan is a PR by request so the review poll finds it. Once reviewed, it can become the issue's body, or the issue can point here.

## 9. Risks

- **The temporal half of layer 6** is a state-space question; L limits itself to the structural obligations, the trace oracle with its negative controls, the deadline oracle, and a GenMC model with a stated boundary, and leaves exactness for evented loci to F.41.
- **`codegen.rs`** (34,900 lines) is where E3a, P1, C6 and L4 land; one spine per commit, a reader pane per commit, the trace shadow, and never two lines with open commits in it at once.
- **The matrix's cost**: a full lifecycle matrix with ASan is nightly work; the CI sample must stay under a minute (the ownership matrix's precedent).
- **Diagnostic wording** is pinned by hundreds of tests and by DNA; every S step's shadow includes `tests/hale` and the DNA seeds, and every correction's wording is a J commit.
- **Corrections hidden in moves** were the phase-2 lesson (#1289's exemption): the classified list in each brief and the J shape for every correction are the control.

## 10. The row manifest

Every legacy row at ee992190, the step that closes it, or the leftover it is. The counts are computed from this table, not asserted elsewhere.

| family | rows | phase 3 closes (step) | leftover |
|---|---|---|---|
| `alloc_summary` | 14 | 13 (E3a), 1 (E3b: `ReclaimScope`) | |
| `entrypoint` | 12 | 6 (E0), 6 (L4) | |
| `lifecycle_order` | 10 | 1 (L2: `lotus_failure_hold`), 9 (L4) | |
| `digests` | 9 | | 9 (phase 4) |
| `stdlib_surface` | 8 | | 8 (phase 4) |
| `placement` | 8 | 8 (P1, `budget_for_programs` included) | |
| `target_capability` | 7 | 7 (P3) | |
| `bus_graph` | 6 | 3 (C4), 2 (C5), 1 (C6: `intra_locus_publish_target`) | |
| `effects` | 6 | 6 (E1) | |
| `handler_routing` | 6 | 5 (C3), 1 (P2) | |
| `ownership` | 6 | 2 (C3), 1 (C4), 2 (C5); 1 conditional (C5: `extend_fresh_factories`) | the conditional row if the checker keeps the unextended set |
| `bindings` | 5 | 5 (P2) | |
| `qualified_names` | 5 | 5 (C2) | |
| `blocking` | 4 | 4 (E2) | |
| `api_surface` | 3 | | 3 (phase 4) |
| `bus_inert` | 3 | 3 (C6) | |
| `claims` | 3 | | 3 (phase 4) |
| `generics` | 3 | 3 (E4) | |
| `snapshot_identity` | 3 | 3 (C3) | |
| `sync_inference` | 3 | 3 (C1) | |
| `topics` | 3 | 3 (C6) | |
| `desugar_sequence` | 2 | 1 (C1), 1 (C6) | |
| `dispatch` | 2 | 1 (P3), 1 (C5: `from_gates`, one plan through the correspondence) | |
| `effect_class_table` | 2 | 2 (C6) | |
| `flows` | 2 | 2 (C6) | |
| `nonreturning` | 2 | 2 (E2) | |
| `restart` | 2 | 2 (L4) | |
| `surfaces` | 2 | 2 (E4) | |
| `top_scope` | 2 | 1 (C5), 1 (C6) | |
| `bare_fallible` | 1 | 1 (E4) | |
| `demand` | 1 | 1 (C6) | |
| `expression_typing` | 1 | 1 (E4) | |
| `forms` | 1 | 1 (E4) | |
| `law_backstops` | 1 | 1 (C6) | |
| `closures` | 1 | | 1 (phase 4) |
| `sealability` | 1 | | 1 (phase 4) |

Totals: 150 rows; phase 3 closes 124 and 1 conditionally; 25 are intentional leftovers for phase 4, each named above; no row is unassigned. A family is Canonical at the exit only when its rows are closed and its final production derivation is gone; a family with a leftover row stays Migrating with that row as its inventory.
