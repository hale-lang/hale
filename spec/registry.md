# The graph registry

GENERATED from `crates/hale-graph/src/registry.rs` and held byte-equal by `registry_matches_spec`. Do not edit: change the table and run `HALE_REGEN_REGISTRY=1 cargo test -p hale-graph --test registry_matches_spec`. The contract this index serves is `spec/model.md` § *The graph registry*.

The families, their legacy producers, the spec rules and the frozen Debug-string sites are counted by `registry_is_well_formed`, not here: a rendered total moved with every change to any family, so two unrelated changes conflicted on one line at every rebase.

## Families

| family | layer | state | kind | producer | legacy | answers |
|---|---|---|---|---|---|---|
| `seed_loading` | Layer 1 | Canonical | desugar | `collect_checkable` | 0 | Which source units form the snapshot: the entry, every imported seed, their merge order and the spans' virtual bases. |
| `qualified_names` | Layer 1 | Canonical | desugar | `resolve_imports` | 0 | What a qualified or aliased name denotes: the library identity, the mangled declaration, the construction target, the bus subject a path names. |
| `desugar_sequence` | Layer 1 | Canonical | desugar | `desugar_before_check` | 0 | Which rewrites the program receives before checking, in which order: the declaration-shaping passes only (JSON parsers, the api surface, unit returns, construction aliases, qualified bus subjects, the omitted `run`, repr accessors). Sync inference is not a rewrite: its pick is a form row (`sync_inference`). The topic-reference and intra-locus rewrites are not desugars: they erase a written declaration reference the checker's laws and the model read, and run in lowering's resolved program, after the check. |
| `sync_inference` | Layer 1 | Canonical | derivation | `form_rows` | 0 | Which sync discipline each `@form` declaration gets: one row per declaration with the author's configuration (omitted, a written discipline, `none` included, or an argument naming none) and the effective discipline, inference's pick for a `hashmap` form left unconfigured, from the domains each of its instances is called from; two queries, explicitly configured and safe for cross-domain access. |
| `effect_class_table` | Layer 1 | Canonical | derivation | `EffectClasses` | 0 | The user effect classes of a load: one table every seed is parsed through, so a class (its name, its identity in the program's one class namespace) has one `User(i)` index in every seed; which were declared, which are composed, and the one expansion of a composed class. |
| `unit_catalogue` | Layer 2 | Canonical | derivation | `close` | 0 | Exact rational relationships between resolved unit identities, cycle consistency witnesses, and coarsest widening-compatible denominations. This is a compiler API; source declarations and expression typing do not demand it yet. |
| `top_scope` | Layer 2 | Migrating | derivation | `build_top_scope` | 1 | What every top-level name denotes: the symbol table over the merged program. |
| `expression_typing` | Layer 2 | Canonical | derivation | `check_bundle_scoped` | 0 | The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from. |
| `generics` | Layer 2 | Canonical | derivation | `unify_generic_ty` | 0 | Which monomorph a generic call instantiates and how its bindings unify. |
| `surfaces` | Layer 2 | Canonical | law | `conformance_witness` | 0 | Which surface is visible at which depth edge: contract exposure, interface conformance, perspective designation and `serves` conformance. |
| `forms` | Layer 2 | Canonical | law | `check_form_shape` | 0 | Whether a form's shape, its capacity slots and its projection class are well formed, and which operation set closes each slot. |
| `stdlib_surface` | Layer 2 | Migrating | capability | `SURFACES` | 6 | What each stdlib function is: its signature, its effect classes, whether it blocks, how it lowers, and what a value of a type can be rendered as. |
| `entrypoint` | Layer 3 | Canonical | derivation | `entry_row` | 0 | Which locus is the program's `main`, whether the world is closed, and which declarations are imported. |
| `ownership` | Layer 3 | Canonical | derivation | `resolve_owners` | 0 | Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array. |
| `bus_graph` | Layer 3 | Canonical | derivation | `build_bus_graph` | 0 | The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates. |
| `topics` | Layer 3 | Canonical | derivation | `topic_wire_subjects` | 0 | What each topic is on the wire: its subject, payload contract, routing key, bounds and shed policy; and which topic a send's subject names. |
| `bindings` | Layer 3 | Canonical | derivation | `derive_binding_rows` | 0 | Which topics are bound to which transport, in which role, with which codec, and whether the transport can carry the payload. |
| `dispatch` | Layer 3 | Migrating | derivation | `fn derive` | 1 | How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement. |
| `handler_routing` | Layer 3 | Canonical | derivation | `handler_rows` | 0 | Which `on_failure` handler a failing child's locus type reaches, and from which parent. |
| `flows` | Layer 3 | Canonical | derivation | `survey` | 0 | Which children are flows (released per completion) and which are resident; and, per locus declaration, whether its `run()` is long-running and whether it never returns. |
| `restart` | Layer 3 | Canonical | derivation | `handler_rows` | 0 | Which loci declare restart operations, which restart in place, and what the restart bound is. |
| `closures` | Layer 3 | Migrating | law | `check_locus_member` | 1 | Whether each closure clause is well formed, and which lifecycle events (`epoch`, `persists_through`, `resets_on`) it names. |
| `api_surface` | Layer 3 | Canonical | derivation | `api_surface` | 0 | The served surface: commands, reads, streams, their schemas, the roles that gate them, and the description's wire form; and the role rows: every `role` declaration, every `@gated` site and the api entry's role source, with or without an `api:` entry. |
| `sealability` | Layer 3 | Migrating | law | `check_sealed_access` | 1 | Which loci confine their state (`@sealed`), and which could. |
| `runs_under` | Layer 3 | Reserved | derivation | — | 0 | On whose authority a locus runs: the relation `runs_under(locus, principal)`, with principals declared by the program. |
| `transitions` | Layer 3 | Reserved | derivation | — | 0 | For an evented locus: the transition each handler is, input event to output set (F.41, after phase 2). |
| `effects` | Layer 4 | Canonical | derivation | `derive_effect_rows` | 0 | Which effect classes each fn and locus reaches (the callgraph fixpoint), the declared classes and their `causes:`/`depends:` DAG, and the certificate relating the two. |
| `blocking` | Layer 4 | Canonical | derivation | `blocking_path_match` | 0 | Which fns block (a cooperative worker would be held), and whether the program places anything off the main thread. |
| `alloc_summary` | Layer 4 | Canonical | derivation | `derive_alloc_summary` | 0 | Where each allocation lands and when it is reclaimed: per-fn allocation, escape, scratch eligibility, method-scratch elision, stack arrays, arena elision. |
| `borrow_lifetime` | Layer 4 | Canonical | law | `borrow_lifetime_diags` | 0 | Whether a borrowed handle outlives its holder (GH #730), decided from position over the owner structure. |
| `bare_fallible` | Layer 4 | Canonical | law | `bare_fallible_calls` | 0 | Whether a fallible call's error is addressed. |
| `nonreturning` | Layer 4 | Canonical | law | `run_statically_nonreturning` | 0 | Which `run()` bodies never return, which children are long-running, and whether the birth order or a pool starves because of it. |
| `working_set` | Layer 4 | Canonical | derivation | `compute_program_working_set` | 0 | The estimated working set per locus and program, and the locality law over it. |
| `placement` | Layer 5 | Canonical | derivation | `derive_placement` | 0 | Which thread domain each instance runs in: pools, pinned threads, replicas, affinity, and the deployment plan. |
| `target_capability` | Layer 5 | Canonical | capability | `derive_capability_matrix` | 0 | What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability. |
| `deployment` | Layer 5 | Reserved | derivation | — | 0 | A deployment as typed rows: root and horizon, component identities, instances and incarnations, resources and allocations, endpoints and routes, hosting and authority, persistence obligations (the habitat, after phase 2). |
| `lifecycle_order` | Layer 6 | Canonical | derivation | `derive_lifecycle` | 0 | The happens-before order per instance: birth sequence, params open and settle, failure delivery and its execution domain, reclaim prerequisites, drain, restart, teardown. |
| `bus_inert` | Layer 6 | Canonical | derivation | `bus_inert` | 0 | Whether the program can ever have a bus cell in flight, so drains can be elided. |
| `law_backstops` | Layer 8 | Canonical | law | `lowering_laws` | 0 | The laws that replaced lowering's own refusals of rules the spec states. |
| `model` | The law engine | Canonical | derivation | `derive_application_model_over` | 0 | The canonical semantic model of a checked bundle: fifteen entity tables, seventeen relation tables, holes, capabilities, provenance (GH #476). |
| `claims` | The law engine | Canonical | law | `claim_law_diags` | 0 | Every user law: lowered claim rows, the judged verdicts over the model and evidence, constitution identities, and the artifact's law account. |
| `view` | The law engine | Reserved | derivation | — | 0 | A named query over the tables: a node selector, a relation set and an adequacy policy, rendered by a backend (hale ui, after phase 2). |
| `snapshot_identity` | Identity | Canonical | derivation | `mint` | 0 | The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again in the resolved-program step (numbered over the user program before the intra-locus rewrite, so the sends it records are numbered on every path, and minted over the merged program with the bundle's seeds and a named seed for the bundled stdlib), idempotently (one numbering; a later mint numbers only what an earlier one did not see), with reliable provenance; and which declaration each use names (`binding_of`), resolved once by the mint. |
| `demand` | Identity | Canonical | derivation | `Snapshot` | 0 | Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, each member's own program beside the merged one, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph, handler rows and flow rows, law selection, the role rows, the arrangement, the effect rows, the model, the check in its two stages with the effects certificate report its typing produced, the lowering view), each at most once, blocking a family whose prerequisite reported errors. |
| `digests` | Identity | Migrating | digest | `IDENTITIES` | 3 | Every identity a build or an artifact carries, and what each covers: shape_hash, artifact_digest, model_hash, exec_digest, the toolchain and cache keys, source digests, and the snapshot key they were derived under. |

## Layer 1 — parse and desugar

### `seed_loading` — Canonical · desugar

**Answers.** Which source units form the snapshot: the entry, every imported seed, their merge order and the spans' virtual bases.

**Inputs.** .hl files; import directives; the workspace root (hale.toml); editor overlays (LSP)

**Producer.** `crates/hale-frontend/src/frontend.rs` · `collect_checkable`

**Also owned.** `crates/hale-frontend/src/parse_cache.rs` · `ParseCache`; `crates/hale-syntax/src/shift.rs` · `shift_program`; `crates/hale-syntax/src/shift.rs` · `shift_item`; `crates/hale-syntax/src/shift.rs` · `erase_positions`

**Consumers.** check; build; run; test; replay; bench; lsp; dna (via the CLI)

**Invariants.**

- one loader, one merge order, for every entry point
- an unresolved import is a diagnostic, never a silently smaller program
- one seed for every entry point: the editor's load (`LoadMode::Editor`) is `hale check <dir>`'s — the open file's directory, every `import` followed through the buffers (`link_checkable`) — and differs only in tolerance: a member that does not parse or will not read is recorded (`Snapshot::unparsed`, `Snapshot::unreadable`) and blocks the scope instead of failing the load, a link the import graph refuses keeps the members as they parsed with the refusal recorded (`Snapshot::unlinked`) and blocks the scope, which every request but the outline reads as the refused load (`Snapshot::linked`), and the LSP publishes an unreadable member as `seed member <name>: <os error>` against the member and the open file, never a clean seed the CLI cannot load
- parse reuse (F.40 phase 3, X1, `hale_frontend::parse_cache`): every file a load parses — a seed's own (`parse_files`, the editor's load), an imported library's two parses (through its own effect-class table, then the load's) — is parsed through the provider's cache when it carries one (`SourceProvider::parses`: the LSP's `Overlay::reusing`, one cache per server; the disk carries none). The cache holds the parser's product alone: the program or the diagnostics `parse_source_at_in` gives for the text at base 0, and the effect-class table the parse left, keyed by the path, the exact text and the table the parse started from (#345: the load's one table is an input of each file's parse). Each load recreates the rest: the file's base in its own source map, the product moved there (`hale_syntax::shift::shift_program`, exhaustive over the AST), then its own shaping and mint. Nothing shaped or minted is kept — shaping reads the whole load and its config, identities are snapshot-local — and the snapshot's key, a whole load's, is never a member's. Nothing invalidates an entry (its product is a function of its key); a path keeps its four most recently used entries

**Missing data.** n/a

**Focused tests.** crates/hale-cli/tests/imports.rs (diamond_import, three_hop_import, import_library_key); crates/hale-cli/tests/source_map.rs; crates/hale-cli/tests/lsp.rs (lsp_and_check_agree_over_a_seed_that_imports, lsp_reports_an_unreadable_seed_member_as_check_does, lsp_outline_survives_a_link_failure); crates/hale-frontend/src/snapshot.rs (a_reused_parse_is_the_parse); crates/hale-syntax/src/shift.rs (the oracle over every .hl in the tree)

**Spec.** spec/projects.md

**Guarded seams.**

- `parse_source_at_in(` may be referenced from: `crates/hale-syntax/src/lib.rs` ×2, `crates/hale-syntax/src/shift.rs` ×2, `crates/hale-frontend/src/parse_cache.rs` ×2, `crates/hale-types/src/effect_classes.rs` ×2
- `parse_in(` may be referenced from: `crates/hale-syntax/src/parser.rs` ×2, `crates/hale-syntax/src/lib.rs` ×1, `crates/hale-frontend/src/imports.rs` ×2

### `qualified_names` — Canonical · desugar

**Answers.** What a qualified or aliased name denotes: the library identity, the mangled declaration, the construction target, the bus subject a path names.

**Inputs.** import aliases; the seed cache; hale_stdlib::PATH_RENAMES; declaration names

**Producer.** `crates/hale-frontend/src/imports.rs` · `resolve_imports`

**Also owned.** `crates/hale-types/src/mangle.rs` · `resolve_construction_aliases`; `crates/hale-frontend/src/imports.rs` · `name_library`; `crates/hale-types/src/qualified_subjects.rs` · `resolve_qualified_bus_subjects`

**Consumers.** check; build; lsp (hover, definition, references)

**Invariants.**

- a name resolves once per snapshot; the checker and lowering see the same target
- a construction path spelled with a type alias is resolved once, bundle-wide, in the desugar sequence before the check (`resolve_construction_aliases`), so the checker and lowering read the same target name; the checker follows no alias of its own, so a fragment checked without the sequence is not resolved a second way
- a qualified bus subject (`subscribe` / `publish alias::Topic`, `alias::Topic <- v`, a `bindings` entry) is resolved once, in the desugar sequence before the check (`resolve_qualified_bus_subjects`), to the single-segment topic the imported declaration ends up at: the checker (`resolve_bus_subject` keeps the leaf and an `Unknown` payload only for a path no rename names), the model and lowering read the rewritten program, and the resolved-program step does not resolve it again
- a qualified path in the checker is resolved through the scope's one import table (`KnownNames::import_target`, built from the bundle's renames): a qualified type, an imported fn (`imported_fn`) and an imported enum's constructor arm (`match_is_exhaustive`) name the same declaration by the same row, compared by identity and never by the text of the merged name, and the checker holds no second scan of the rename vector for any of them
- a library's name (`AliasScopes::name_library`) is a function of its own path alone — relative to the entry's workspace root, else to the entry seed's directory (`library_basis`), encoded injectively (`library_id`) — so no two libraries of a load share a name, and neither import order, the other imports of the build nor the tree's absolute location changes the symbols a library is mangled under; a declaration's full name (`mangle::mangled`) encodes the library, the file stem and the declaration as one injective tuple, so no two declarations of a load share one either

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/import_library_key.rs; crates/hale-cli/tests/import_library_names.rs; crates/hale-cli/tests/cross_seed_arity.rs; crates/hale-codegen/tests/cross_seed_imports.rs; crates/hale-types/tests/type_alias.rs; crates/hale-types/tests/qualified_subjects.rs; crates/hale-cli/tests/import_qualified_topic.rs

**Spec.** spec/semantics.md § Cross-seed namespace resolution; spec/projects.md

**Guarded seams.**

- `resolve_construction_aliases(` may be referenced from: `crates/hale-types/src/mangle.rs` ×1, `crates/hale-types/src/desugar_sequence.rs` ×1
- `resolve_qualified_bus_subjects(` may be referenced from: `crates/hale-types/src/qualified_subjects.rs` ×1, `crates/hale-types/src/desugar_sequence.rs` ×1
- `name_library(` may be referenced from: `crates/hale-frontend/src/imports.rs` ×2

### `desugar_sequence` — Canonical · desugar

**Answers.** Which rewrites the program receives before checking, in which order: the declaration-shaping passes only (JSON parsers, the api surface, unit returns, construction aliases, qualified bus subjects, the omitted `run`, repr accessors). Sync inference is not a rewrite: its pick is a form row (`sync_inference`). The topic-reference and intra-locus rewrites are not desugars: they erase a written declaration reference the checker's laws and the model read, and run in lowering's resolved program, after the check.

**Inputs.** the merged program; --api / --env (roles); the cross-seed rename table

**Producer.** `crates/hale-types/src/desugar_sequence.rs` · `desugar_before_check`

**Also owned.** `crates/hale-types/src/resolved.rs` · `resolve_program`; `crates/hale-types/src/resolved.rs` · `rewrite_intra_locus`; `crates/hale-types/src/resolved.rs` · `resolve_rewritten`; `crates/hale-syntax/src/desugar.rs` · `desugar_intra_locus_topics`; `crates/hale-syntax/src/desugar.rs` · `desugar_topics`; `crates/hale-types/src/desugar_sequence.rs` · `bundled_stdlib`; `crates/hale-syntax/src/desugar.rs` · `desugar_omitted_run`; `crates/hale-syntax/src/desugar.rs` · `desugar_repr_accessors`

**Consumers.** check; build; run; test; replay; bench; lsp; codegen (`crates/hale-codegen/src/codegen.rs` · `build_resolved`)

**Invariants.**

- one order, run once per snapshot, before the first law is judged
- the sequence is called from the snapshot's load (`Snapshot::load`, `Snapshot::from_program`) and from `check_program` (the test entry), and from nowhere else: every entry point runs it before it mints its snapshot; the bundled stdlib goes through the same passes (`bundled_stdlib`)
- codegen never re-desugars
- the topic-reference and intra-locus rewrites are lowering's, on lowering's copy of the program, never the checked one, and each is recorded as a relation: `TopicRewrite` rows (`topic_rewrites`, and `written_topics` on the bus graph's subjects) and `IntraLocusRewrite` rows (`intra_locus`, and `direct_sends`); the intra-locus rewrite is its own stage, `rewrite_intra_locus` (over the sequence's program, its qualified bus subjects already resolved, keeping on the bus a publish into a field the placement table runs off its owner's thread; the snapshot's `intra_locus` count), which the check demands for rule 10 and lowering continues from, so the judgment and the lowering read one rewrite; `resolve_rewritten` runs the topic rewrite, appends the stdlib, mints and derives the tables
- a desugar that copies a subtree clears the copy's identities (`hale_syntax::sites::clear_ids_in_*`): a verb mints after the sequence and the resolved program mints again after its rewrites, and two sites with one id is a panic; every corpus program is minted first and resolved second by a test
- the omitted `run` is synthesized in the sequence, before the check, and marked `LifecycleDecl::synthesized`: a rule about the run's body reads the empty body and answers both spellings alike (spec semantics.md § run()); what lists the hooks the author wrote (the application model, whose function and phase tables are the artifact's identity, the allocation summary, the effect contracts, the mint's `OmittedRun` origin) reads the marker, never the absence of author text, and a synthesized `run` moves no artifact and no diagnostic

**Missing data.** n/a

**Focused tests.** crates/hale-cli/tests/api_description.rs; crates/hale-codegen/tests/framework_elision.rs; crates/hale-types/tests/topology_projection.rs (the_omitted_run_moves_no_artifact_and_no_diagnostic); crates/hale-types/tests/bus_graph.rs (a_rewritten_send_stays_on_the_resolved_graph, a_rewritten_topic_reference_stays_on_the_resolved_graph)

**Spec.** spec/semantics.md

**Guarded seams.**

- `desugar_topics(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `desugar_before_check(` may be referenced from: `crates/hale-types/src/desugar_sequence.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1
- `desugar_omitted_run(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×1, `crates/hale-types/src/desugar_sequence.rs` ×1
- `desugar_repr_accessors(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×1, `crates/hale-types/src/desugar_sequence.rs` ×1
- `resolve_program(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1
- `rewrite_intra_locus(` may be referenced from: `crates/hale-types/src/resolved.rs` ×2, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1
- `resolve_rewritten(` may be referenced from: `crates/hale-types/src/resolved.rs` ×2, `crates/hale-frontend/src/snapshot.rs` ×1
- `desugar_intra_locus_topics(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×1, `crates/hale-types/src/resolved.rs` ×1

### `sync_inference` — Canonical · derivation

**Answers.** Which sync discipline each `@form` declaration gets: one row per declaration with the author's configuration (omitted, a written discipline, `none` included, or an argument naming none) and the effective discipline, inference's pick for a `hashmap` form left unconfigured, from the domains each of its instances is called from; two queries, explicitly configured and safe for cross-domain access.

**Inputs.** placement (the table, per instance); top_scope; form declarations; method call sites

**Producer.** `crates/hale-types/src/form_rows.rs` · `form_rows`

**Also owned.** `crates/hale-types/src/sync_inference.rs` · `infer_sync_for_bundle`

**Consumers.** the snapshot (one row set per snapshot, after the mint, over its scope and placement table) (`crates/hale-frontend/src/snapshot.rs` · `demand_forms`); the allocation summary (`sync_forms`: the stdlib analysis copy's, from that universe's rows) (`crates/hale-types/src/alloc_summary.rs` · `summarize_identified`); check (F.31 cross-pool verdicts: the one predicate, safe for cross-domain access) (`crates/hale-types/src/check.rs` · `check_placement_single_thread`); check (instance aliasing: a field behind a sync discipline) (`crates/hale-types/src/check.rs` · `locus_has_unsynchronized_state`); the effects certificate engine (a call into a sync-bearing form or its holder can take its lock) (`crates/hale-types/src/alloc_summary.rs` · `add_sync_forms`); sync inference (its candidates: the forms not explicitly configured) (`crates/hale-types/src/sync_inference.rs` · `infer_sync_for_bundle`); model (`sync_form`, read by the `depends` law) (`crates/hale-types/src/model_builder.rs` · `derive_application_model_over`); the lowering view (the snapshot's rows, and the merged stdlib's as written) (`crates/hale-types/src/resolved.rs` · `resolve_program`); codegen (the slot layout) (`crates/hale-codegen/src/locus/decl.rs` · `sync_mode`); lsp

**Invariants.**

- every entry point sees the same discipline for the same program
- one row per `@form` declaration, found by the identity the load minted (a monomorph by its template's) or by name: the configuration and the effective discipline are separate columns
- an explicit `sync = none` is configuration: inference does not run over it, and the row does not call it safe for cross-domain access
- one predicate per question: inference's candidates are the forms not explicitly configured, and the F.31 cross-pool exemption is safe for cross-domain access; sync inference runs once per snapshot, and the cross-pool diagnostic's hint reads its reasoning from the rows
- nothing writes the discipline into the program: the check, the effects engine, the model and lowering read the row, and a declaration with no row (a bundle's no row set holds) reads its written argument, except in lowering, whose view holds a row for every declaration it lowers and refuses one with none (F.40 phase 3's exit)
- the stdlib's analysis copy has its own universe's rows (`stdlib_bodies::forms`, once per process, found by the identities the copy was minted with), each its written configuration, since nothing infers over the copy; the allocation summary's `sync_forms` reads them and no `sync =` argument, and the effects engine adds the program's own from the snapshot's rows (`add_sync_forms`) (C3 rest)
- the readers that ask whether a form synchronizes as one question (the model's `sync_form`, the effects engine, instance aliasing) ask safe for cross-domain access alone (`FormRows::synchronizes`): an explicit `sync = none` takes no lock and is not one
- inference is per instance (the placement correspondence's K-5): an access `self.f.m()` is made from the domain of each instance of its enclosing locus and reaches that instance's own `f` (a held one by its source row, so its holders share it); the rule is applied per accessed instance and a type gets the most synchronized discipline any instance needs, never a union of domains per type; what the table does not know (a dynamic literal of unknown domains, a held instance whose source is unlinked, the instance below one) is a domain apart from every other, never main

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `sync_inference_a_hashmap_without_its_row_is_refused_at_its_declaration`

**Focused tests.** crates/hale-types/tests/form_rows.rs (the_summary_reads_the_copys_rows_and_the_engine_adds_the_programs); crates/hale-frontend/src/snapshot.rs (the_form_rows_are_one_family_by_identity); crates/hale-types/tests/placement.rs

**Spec.** spec/forms.md § Cross-pool sync disciplines; spec/semantics.md § A form's sync discipline

**Guarded seams.**

- `form_rows(` may be referenced from: `crates/hale-types/src/form_rows.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/effects.rs` ×1

### `effect_class_table` — Canonical · derivation

**Answers.** The user effect classes of a load: one table every seed is parsed through, so a class (its name, its identity in the program's one class namespace) has one `User(i)` index in every seed; which were declared, which are composed, and the one expansion of a composed class.

**Inputs.** `effect` declarations and class references per seed; the load's parse order (a seed after the seeds it imports)

**Producer.** `crates/hale-syntax/src/ast.rs` · `EffectClasses`

**Also owned.** `crates/hale-syntax/src/lib.rs` · `parse_source_at_in`; `crates/hale-types/src/effect_classes.rs` · `EffectClassTable`

**Consumers.** the load (own files, the editor's members, every imported seed after its imports; through the provider's parse cache when it carries one) (`crates/hale-frontend/src/frontend.rs` · `parse_file`); effects (contracts, phase contracts, the declared manifest) (`crates/hale-types/src/effects.rs` · `EffectClassTable::of(`); the effect rows (one table per snapshot, carried on the rows: the model's effect-class rows and atoms, and the inferred manifest's class names, read it there) (`crates/hale-types/src/effect_rows.rs` · `EffectClassTable::of(`); effects (causes) (`crates/hale-types/src/frontier.rs` · `EffectClassTable::of(`); alloc_summary (what `@effects(is: …)` carries) (`crates/hale-types/src/alloc_summary.rs` · `EffectClassTable::of(`); quantitative (user-class budgets) (`crates/hale-types/src/quantitative.rs` · `EffectClassTable::of(`); claims (lowering: class references, undeclared classes) (`crates/hale-types/src/claim_lowering.rs` · `EffectClassTable::of(`); topology (derived effect sets) (`crates/hale-types/src/topology.rs` · `EffectClassTable::of(`)

**Invariants.**

- one class, one index, per load: every seed is parsed through the load's one table (`parse_source_at_in`, `parser::parse_in`, or a parse the provider's cache made from an equal table, which it keys on: `parse_cache`), so merging seeds renumbers nothing; an imported seed is numbered after the seeds it imports (it is parsed again, through the table, once they are), the order the classes have always been numbered in
- one expansion: a composed class's mask, its atoms and whether its definition is cyclic are `EffectClassTable`'s; no analysis walks a definition itself or reads a program's table directly

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/cross_seed_effects.rs; crates/hale-cli/tests/xseed_user_effects.rs; crates/hale-types/src/effect_classes.rs (one_table_one_expansion); crates/hale-frontend/src/snapshot.rs (a_seed_is_numbered_after_the_seeds_it_imports)

**Spec.** spec/verification.md § Default-on & opt-in analyses

**Guarded seams.**

- `EffectClassTable::of(` may be referenced from: `crates/hale-types/src/effect_classes.rs` ×1, `crates/hale-types/src/effects.rs` ×3, `crates/hale-types/src/effect_rows.rs` ×1, `crates/hale-types/src/frontier.rs` ×1, `crates/hale-types/src/alloc_summary.rs` ×1, `crates/hale-types/src/quantitative.rs` ×1, `crates/hale-types/src/claim_lowering.rs` ×1, `crates/hale-types/src/topology.rs` ×1
- `effect_defs` may be referenced from: `crates/hale-syntax/src/ast.rs` ×1, `crates/hale-syntax/src/parser.rs` ×9, `crates/hale-frontend/src/frontend.rs` ×3, `crates/hale-types/src/effect_classes.rs` ×2, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-syntax/src/shift.rs` ×1

## Layer 2 — declaration graphs

### `unit_catalogue` — Canonical · derivation

**Answers.** Exact rational relationships between resolved unit identities, cycle consistency witnesses, and coarsest widening-compatible denominations. This is a compiler API; source declarations and expression typing do not demand it yet.

**Inputs.** unit declaration SiteIds; equations with declaration SiteIds and positive exact rational factors

**Producer.** `crates/hale-types/src/unit_graph.rs` · `close`

**Consumers.** conversion queries (reduced factor, exactness and equation witness) (`crates/hale-types/src/unit_graph.rs` · `conversion`); unpinned denomination queries (rational gcd and necessary input witnesses) (`crates/hale-types/src/unit_graph.rs` · `meet`)

**Invariants.**

- unit identity is the snapshot's SiteId, never a display spelling, span or NodeId equality
- factors are positive arbitrary-precision rationals; no machine overflow, rounding, runtime base unit or default loss policy
- every cycle has product one; an inconsistent catalogue returns only errors with witnessed cycles, never a partially usable closure
- a conversion between disconnected components or through an unknown unit has no answer
- denomination is the rational gcd of the input units, potentially unnamed, with an irredundant set of input witnesses; 6, 10 and 15 require three witnesses
- a denominator of one proves denomination conversion exactness only, not that a runtime range or representation width can hold the result

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/src/unit_graph.rs

**Spec.** spec/units.md

### `top_scope` — Migrating · derivation

**Answers.** What every top-level name denotes: the symbol table over the merged program.

**Inputs.** the merged program; import renames

**Producer (today's authority, migrating).** `crates/hale-types/src/resolve.rs` · `build_top_scope`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/lib.rs` · `check_bundle_opts_scoped` — `check_program` (the test entry): built here, once, for its checker and the model its laws are judged over. Beside it the model of a bundle no snapshot holds (`derive_application_model`: `claim_law_diags`, the hale-types tests, and the artifact and model-hash entries over a bare bundle; since 2.3 no verb reaches it), the certificate report of such a bundle (`effect_certificates`, for its form rows) and `resolve_program` (the bare program's test entry, once over that program for the scope and the graphs it hands the view) rebuild it; the lowering view reads its snapshot's (F.40 phase 3, C5); every verb and the LSP (its diagnostics and every request) build one per snapshot (`demand_scope`) and pass it to the checker, the model and the model's graphs. *Removed when:* phase 4, when every consumer demands the scope from a snapshot: 2.3 moved every verb and the LSP, and what still builds its own is the test entries over a bundle or a program no snapshot holds (`check_program`, `derive_application_model`, `effect_certificates`, `resolve_program`), which phase 3 kept (C5's follow-up ruling).

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_bundle_scoped`); check (type expressions: the scope's name table) (`crates/hale-types/src/check.rs` · `&top.names`); demand (every verb, the LSP's diagnostics and its requests: one scope per snapshot) (`crates/hale-frontend/src/snapshot.rs` · `build_top_scope`); model (the snapshot's scope, handed in) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); resolved program (lowering: the snapshot's scope, handed in; its topic rows, and the stdlib's bus rows and typed-body pairs answered over it) (`crates/hale-types/src/resolved.rs` · `top: &TopScope`); resolve_program (the bare program's test entry: once over that program) (`crates/hale-types/src/resolved.rs` · `build_top_scope`); lsp (definition, placement, the allocation survey: the snapshot's scope) (`crates/hale-lsp/src/lib.rs` · `demand_scope`); lsp (completion, hover, references, enforcement: the editor's scope, over the members that parsed while one does not) (`crates/hale-lsp/src/lib.rs` · `demand_editor_scope`)

**Invariants.**

- one namespace decision: module-nested declarations and imported seeds resolve the same way everywhere
- the editor's scope over a seed with a hole (`demand_editor_scope`) is the same producer over the members that parsed, counted as this family; the whole scope, the check and everything after it stay blocked, so no check runs over a partial program
- one name table: the checker resolves every type expression against the scope's own (`TopScope::names`, the declared loci, types and perspectives with each alias's expanded target and the bundle's import renames), built once with the symbols, and keeps none of its own

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/src/lifecycle/derive.rs (shared ancestry regression); crates/hale-types/tests/checks_inside_modules.rs; crates/hale-cli/tests/check_unknown_identifier.rs; crates/hale-types/tests/type_alias.rs

**Spec.** spec/semantics.md

**Guarded seams.**

- `build_top_scope(` may be referenced from: `crates/hale-types/src/resolve.rs` ×1, `crates/hale-types/src/lib.rs` ×3, `crates/hale-types/src/sync_inference.rs` ×1, `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lifecycle/derive.rs` ×1

### `expression_typing` — Canonical · derivation

**Answers.** The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from.

**Inputs.** top_scope; declarations; bodies

**Producer.** `crates/hale-types/src/check.rs` · `check_bundle_scoped`

**Also owned.** `crates/hale-types/src/resolve.rs` · `infer_literal_ty`; `crates/hale-types/src/typed_bodies.rs` · `typed_bodies`; `crates/hale-types/src/builtin_sigs.rs` · `BARE_BUILTIN_SIGS`

**Consumers.** the snapshot (one typed-body table per snapshot, packaged on demand from the check's record) (`crates/hale-frontend/src/snapshot.rs` · `demand_typed_bodies`); the check of a bundle no snapshot holds (the record packaged for the `bare_fallible` law) (`crates/hale-types/src/check.rs` · `check_bundle_reporting`); codegen (an accumulator slot's element type, the closure's typed-body row) (`crates/hale-codegen/src/codegen.rs` · `accumulator_element_type`); codegen (a bare builtin's arity and result: its signature row) (`crates/hale-codegen/src/codegen.rs` · `builtin_sig`); the cross-pool value law (`law_backstops`, at the harness's lowering view: the table's `omitted_args`; the check reads its record's) (`crates/hale-frontend/src/snapshot.rs` · `omitted_args`); every layer

**Invariants.**

- expression typing is not a layer: it is the derivation inside layer 3 that produces typed edges, and it stays Rust (final direction)
- codegen types no value the checker typed: an accumulator's element type is the closure's typed-body row, and a hole is refused at its span
- the checker's answers are carried, never re-derived: the check records them as it walks, and one typed-body table per snapshot packages the record (`demand_typed_bodies`, no second check but for a typing that reused a declaration, the snapshot family's X2 row; the check demands it once, for the `bare_fallible` law), keyed by declaration identity (a body by its declaration's site, a call by its `Call` site, a monomorph by its template's site and type arguments, never by a name string), with six columns: accumulator element types, generic calls' type arguments and unified params, the monomorph table, conformance per (locus, interface) pair, fallible calls (the callee's mark and what addresses the call), and `omitted_args` (per call that leaves arguments to their defaults, the declaration the checker resolves the callee to, a method by its receiver's type, and the first parameter it leaves; a type's field defaults, which the checker does not type, have their calls recorded too); a site the checker could not type is a hole with its reason
- the bare builtins (`len`, `to_string`, the `Int` / `Float` casts, `abs` / `min` / `max`, `starts_with` / `contains`) are typed by one signature table (`BARE_BUILTIN_SIGS`), lowering's inference written down: the checker types a call by its row where lowering lowers it and leaves it `Unknown` where lowering refuses, and lowering reads each builtin's arity and result from the same row

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/typed_body_rows.rs` · `an_accumulator_without_a_row_is_refused_at_its_expression`

**Focused tests.** crates/hale-types/tests/typed_bodies.rs; crates/hale-types/tests/codegen_fixtures_typecheck.rs; crates/hale-codegen/tests/corpus_check_build_agreement.rs

**Spec.** spec/types.md

**Guarded seams.**

- `typed_bodies(` may be referenced from: `crates/hale-types/src/typed_bodies.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1

### `generics` — Canonical · derivation

**Answers.** Which monomorph a generic call instantiates and how its bindings unify.

**Inputs.** generic declarations; call arguments; the mangled token vocabulary

**Producer.** `crates/hale-types/src/check.rs` · `unify_generic_ty`

**Also owned.** `crates/hale-types/src/check.rs` · `monomorph_table`; `crates/hale-types/src/check.rs` · `specialize_generic_bodies`; `crates/hale-types/src/check.rs` · `record_specialized_type`

**Consumers.** check (a mangled monomorph name: the table's row) (`crates/hale-types/src/check.rs` · `resolve_generic_monomorph`); codegen (a generic fn call's type arguments and specialization: the call's typed-body row, and the monomorph table's row for them) (`crates/hale-codegen/src/codegen.rs` · `generic_call_instance`); codegen

**Invariants.**

- one unification; the monomorph set is a row lowering reads
- lowering infers no generic argument: a generic fn call's type arguments are its typed-body row (inside a fn or locus specialization, the owning body's row typed for that monomorph, including params defaults in their declaring locus) and the specialization's name is the monomorph table's; a hole, or a call with no row, is refused at the call
- one monomorph table per snapshot (the typed-body table's `monomorphs`), keyed by the template's site and its type arguments, never by a name string: its producer parses a mangled name once, for each name the program spells (a written instantiation as the checker resolves it, an annotation, a struct literal's path), against the bundle's templates by identity; the checker's lookups read the row
- a generic call inside a generic fn or locus body is typed again for each of the enclosing template's monomorphs, the template's parameters bound to its arguments, and recorded under them by its source body and call identities; that walk reports nothing and queues concrete instantiations reached in annotations for the same producer to walk
- omitted function and method defaults are typed at each invocation in the caller's scope; their generic call rows retain the default's source identity and distinguish the invocation path (nested defaults included) and the caller's specialization, and lowering reads that corresponding row without inference; a supplied argument evaluates no default

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `generics_a_call_without_its_row_is_refused_at_the_call`; total: no row means no specialization: a call with no specialized row for its enclosing body or locus reads its own, and a monomorph the table does not name is not queued ahead of the source walk

**Focused tests.** crates/hale-codegen/tests/generic_monomorph_agreement.rs; crates/hale-types/tests/typed_bodies.rs

**Spec.** spec/types.md

### `surfaces` — Canonical · law

**Answers.** Which surface is visible at which depth edge: contract exposure, interface conformance, perspective designation and `serves` conformance.

**Inputs.** type, interface, contract, perspective declarations; locus members

**Producer.** `crates/hale-types/src/check.rs` · `conformance_witness`

**Also owned.** `crates/hale-types/src/check.rs` · `conformance`

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_contract_expose_validity`); check (`crates/hale-types/src/check.rs` · `check_serves_conformance`); check (`crates/hale-types/src/check.rs` · `check_reperspective`); check (interface coercion, F.20) (`crates/hale-types/src/check.rs` · `check_structural_impl`); check (a bus adapter binding's `__StdBusAdapter` contract) (`crates/hale-types/src/check.rs` · `check_main_and_bindings`); the typed-body table (the conformance column: every declared locus and interface pair, every generic locus specialization, the merged stdlib's pairs in the lowering view) (`crates/hale-types/src/typed_bodies.rs` · `typed_bodies`); codegen (storage routing: the conformance column) (`crates/hale-codegen/src/types/mod.rs` · `locus_satisfies_interface`); codegen (vtable swap)

**Invariants.**

- F.8 compatibility, F.14 and F.20 satisfaction are judged once, with a witness
- one conformance function (`conformance_witness`), its witness the first requirement unmet in the interface's method order, rendered by each caller in its own words; the bus adapter's contract is judged without the error channel, as it always was
- storage routing asks the conformance column, never the method names, and requires the checker's verdict that the locus satisfies the interface: a locus whose methods match the interface's by name and not by signature, or a generic locus's specialization, is no interface's, so its literal stays the frame's (a classified correction: the name comparison sent both to the program-lifetime payload arena; pinned in `conformance_routing_correction.rs`)

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `surfaces_a_pair_without_its_conformance_row_is_refused`; total: no row means no interface: a name that declares none is satisfied by nothing

**Focused tests.** crates/hale-types/tests/perspective_serves.rs; crates/hale-types/tests/duplicate_member.rs; crates/hale-types/tests/typed_bodies.rs; crates/hale-codegen/tests/conformance_routing_correction.rs

**Spec.** spec/types.md; spec/semantics.md

### `forms` — Canonical · law

**Answers.** Whether a form's shape, its capacity slots and its projection class are well formed, and which operation set closes each slot.

**Inputs.** @form arguments; capacity declarations; indexed_by; sync discipline (the `sync_inference` form rows)

**Producer.** `crates/hale-types/src/check.rs` · `check_form_shape`

**Consumers.** check; codegen (slot layout: the form row's `cap` and `sync` discipline) (`crates/hale-codegen/src/locus/decl.rs` · `ring_buffer_cap`); sync_inference

**Invariants.**

- the operation set a form closes over a slot is a row: it is what a storage binding (F.44) will need
- the form row carries the form's fixed capacity (`FormRow::cap`: a ring buffer's or an LRU cache's `cap =`, and a lockfree map's), and codegen's slot layout reads it and the `sync` discipline from the row (`FormRows::of`), never the form's arguments (C3 rest); the view holds a row for every `@form` declaration it lowers, the merged stdlib's as written, and a declaration with none is refused at its name (F.40 phase 3's exit)

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `forms_a_form_without_its_row_is_refused_at_its_declaration`

**Focused tests.** crates/hale-types/tests/reserved_locus_members.rs; crates/hale-codegen/tests/form_vec_bce.rs; crates/hale-types/tests/form_rows.rs (the_row_carries_the_forms_fixed_capacity)

**Spec.** spec/forms.md; spec/memory.md

### `stdlib_surface` — Migrating · capability

**Answers.** What each stdlib function is: its signature, its effect classes, whether it blocks, how it lowers, and what a value of a type can be rendered as.

**Inputs.** one table of stdlib functions (`SURFACES`: one row per function, grouped by namespace: its name, whether user code may call it, its effect classes, its signature when it has one, and how it lowers: an intrinsic id, a Hale body by name, a rename, or not at all); hale_stdlib::PATH_RENAMES; the parsed stdlib source

**Producer (today's authority, migrating).** `crates/hale-types/src/stdlib_surface.rs` · `SURFACES`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `lower_stdlib_path_call_expr` — 271 `["std", ..]` literals dispatch stdlib calls inside codegen; the registry's own comment calls this dispatch `reality`; each dispatched path's row names what its arm does (`Lower`), held equal to the arms by the parity test, but codegen does not dispatch from it yet. *Removed when:* codegen dispatches from the registry row.
- `crates/hale-codegen/src/codegen.rs` · `lower_stdlib_path_call` — the statement form: the expression dispatch with the value dropped, except for 54 more `["std", ..]` literals, in the arms a statement answers differently (Unit-only primitives, the assertions, two Hale bodies) and its fallibility refusal (34 paths). *Removed when:* same.
- `crates/hale-codegen/src/channels/mod.rs` · `try_lower_fallible_stdlib_path_call` — the fallible-call dispatch (`path or raise` on a stdlib path), a third copy of the stdlib call shapes: 150 more `["std", ..]` literals. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `value_to_string_supports` — the printable set, kept in lockstep by hand with the checker's `ty_is_printable`. *Removed when:* one predicate.
- `crates/hale-types/src/check.rs` · `ty_is_printable` — the checker's copy of the printable set. *Removed when:* one predicate.
- `crates/hale-codegen/src/codegen.rs` · `declare_builtin_closure_violation_type` — a hand-maintained mirror of the checker's injected builtin types. *Removed when:* one declaration.

**Consumers.** effects (`crates/hale-types/src/effects.rs` · `effects_for`); frontier (`crates/hale-types/src/frontier.rs` · `effects_for`); codegen; lsp (hover, completion); doc

**Invariants.**

- one row per stdlib function: the signature, the effect classes and the lowering of a path are columns of the same row, and every question the checker, the effects analysis, the catalogue and the LSP ask (lookup, the unknown-function diagnostic, the did-you-mean, the effect set, the signature) reads it; an internal row answers only the signature
- every path a dispatcher lowers has a row whose lowering is what its arms do, every intrinsic or Hale-body row has an arm, a renamed row has none, and the unlowered rows are named (parity test)
- the checker and codegen agree on every stdlib call shape (parity test) and on the printable set (corpus agreement)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/stdlib_registry_parity.rs; crates/hale-codegen/tests/stdlib_table_answers.rs; crates/hale-codegen/tests/corpus_check_build_agreement.rs; crates/hale-cli/tests/doc_effects_catalogue.rs

**Spec.** spec/stdlib.md

## Layer 3 — the locus graph

### `entrypoint` — Canonical · derivation

**Answers.** Which locus is the program's `main`, whether the world is closed, and which declarations are imported.

**Inputs.** locus declarations (is_main, imported, module nesting); the top-level `fn main`; the minted sites

**Producer.** `crates/hale-types/src/entry.rs` · `entry_row`

**Consumers.** check (rule 1's count reads the witness: the seed's own mains, module-nested ones included) (`crates/hale-types/src/check.rs` · `check_main_and_bindings`); placement (the table is seeded from the row's root, `EntryRow::root`: the entry, or the refused module-nested `main` of a seed with none, so the rules that read the table still judge it; the F.31 rule, the form rows' sync inference, the blocking check, the pinned-in-a-loop rule, instance aliasing and the model's arrangement read it) (`crates/hale-types/src/placement.rs` · `derive_placement`); check (decision 2's refusal, a lowering law: a seed whose only `main locus` is module-nested, `EntryRow::refused`, is refused once at that locus's name, by the check and by the harness's lowering view) (`crates/hale-types/src/lowering_laws.rs` · `module_nested_main_is_not_the_entry`); check (rule 9's closed world is a program with an entry, and its api exemption is the entry's `api:` binding, never an imported `main`'s, which is inert) (`crates/hale-types/src/check.rs` · `check_bus_graph`); check (each `main locus`'s own `placement { }` block is validated over the witness, deployed or not: an affinity on no named pool, two for one pool) (`crates/hale-types/src/check.rs` · `check_pool_affinity`); bus_graph (the closed world: the entry, or a top-level `fn main`) (`crates/hale-types/src/bus_graph.rs` · `build_bus_graph`); ownership (the closed world: the entry, or a top-level `fn main`, the bus graph's test; the producer hands the walk its row, and the rows of a bundle no snapshot holds, `OwnershipRows::of`, build theirs; the stdlib's rows have none) (`crates/hale-types/src/ownership_graph.rs` · `collect_ownership_walk`); effects (the async_io placement-implied advisory: the row's root's placement, the pools lowering spawns) (`crates/hale-types/src/effects.rs` · `placement_implied_diags`); check --matrix (a seed is an entrypoint when its row has an entry; the row is built over the seed's own files, since no import holds the entry, so a seed whose import does not resolve is still counted and its pair reports the import) (`crates/hale-cli/src/verbs/check/matrix.rs` · `seed_entry_kind`); --env on check and the build paths (the load refuses an environment for a seed with no entry, after the mint, before the sequence's own refusal) (`crates/hale-frontend/src/snapshot.rs` · `demand_entry`); build; dna; codegen (the deployment plan's root is the placement table's, the row's entry) (`crates/hale-codegen/src/codegen.rs` · `collect_main_placement`); codegen (`fn main` is the row's column, found by its site: the body `in_main` marks, whose `return` and assertion-failure exits key on it; a view with no row is refused before lowering, never given the row's definition, `top_level_fn_main`) (`crates/hale-codegen/src/codegen.rs` · `entry_fn`); bindings (codegen's binding term counts the entry's entries, by the declaring locus's site) (`crates/hale-types/src/binding_rows.rs` · `binds_on_main`); the lifecycle plan (the deferred main entry's spine is the entry's, by identity) (`crates/hale-types/src/lifecycle/derive.rs` · `is_entry`); the lowering view (the snapshot's row, carried to codegen) (`crates/hale-frontend/src/snapshot.rs` · `view.entry = Some(entry)`); codegen (every comparison against the main locus reads the row's entry, found in lowering's program by its site: instantiation's and the cascade's placement overrides, the deferred entry's pool join, the frame flush's main-locus head before its pinned joins, whether the root's literal hands it back so it keeps the join record of every replica of its pinned fields, and which struct carries those records (C52), the bindings prelude's loss handler, and `root_bindings`, which the shm-ring subjects and the binding codec thunks read) (`crates/hale-codegen/src/codegen.rs` · `is_entry_locus`); the api binding (GH #1106: generated from the row's root's `api:` entry (`EntryRow::root`: the entry, or a refused module-nested `main`), the `main locus` it joins as a param, never an imported library's or a second one; `--api` puts its entry there, and a seed with no root is refused, saying whether it has no `main locus` or only an imported one) (`crates/hale-types/src/desugar_sequence.rs` · `desugar_before_check`); check (the api entry's knobs and what the api leaves out, over the surface generated from the row's root: the snapshot's, `CheckInputs::api_surface`; a bundle no snapshot holds derives it from its own row) (`crates/hale-types/src/lib.rs` · `bundle_api_surface`); the role rows (`served`: the row's root carries an `api:` entry, so `owner` is among the roles an environment maps) (`crates/hale-types/src/roles.rs` · `role_rows`); --env and --matrix (an environment's constitutions are adopted into the row's root, the seed's own `main locus` the build deploys: the entry, in every seed an environment may be bound to) (`crates/hale-frontend/src/snapshot.rs` · `adopt_into_root`); model (the header's `entrypoint` names the root the arrangement is rooted at, the placement table's, so the row's root; `main` when there is none) (`crates/hale-types/src/model_builder.rs` · `derive_application_model_over`); lsp (`hale/placement` shows the row's root's params and placement block, a refused module-nested root included, never an imported library's `main locus`) (`crates/hale-lsp/src/lib.rs` · `placement_of`); hale dna init (the seed's entry and its file, by the row over the seed's own files that parse, as `seed_entry_kind` reads it: no import is resolved, and none holds the entry) (`crates/hale-cli/src/dna.rs` · `main_of`); claims (the world tier: the row's `world()`, every `main locus` the bundle declares, whose inline claims are the world law, an imported application's included, GH #733; a bundle that closes a world refuses a top-level `claims` block of the closing seed's own) (`crates/hale-types/src/claims.rs` · `enumerate_clauses`); the cross-seed rename (GH #774: an imported seed closes a world, so its constitutions' vocabulary is its own, when the row over its own files, none of them renamed yet, has an entry) (`crates/hale-types/src/mangle.rs` · `seed_declares_main`)

**Invariants.**

- the checker builds no row: the snapshot demands it before the check and hands it in (`CheckInputs::entry`); a bundle no snapshot holds (the test entries) builds it once, the form rows read the snapshot's (`demand_forms`), and the check matrix builds one over each seed's own files (no import holds the entry, and a seed whose import does not resolve is still an entrypoint)
- what lands in the row's root (`EntryRow::root`) before the mint (an environment's constitutions, `--api`'s entry, the generated api binding) finds it by the producer's row over the programs as they stand (`entry_row_in`, each program named by its index), and a reader handed programs and no bundle reads it there; an injection adds members and trailing top-level items, never a `main locus` or a module, so the snapshot's row names the same declaration after the mint
- one row per snapshot (`Snapshot::demand_entry`, the `entrypoint` count): the entry by its minted site, and every `main locus` the bundle declares as the witness, each with whether it is imported and whether it is module-nested; it reads declarations only, so no diagnostic blocks it, and a seed with a hole (no identities) blocks it with its scope
- an imported `main` is not the entry (decision 1, E0): a library's `main locus` is a declaration the importing seed does not run, and the entry is the importing seed's own; a seed whose only `main` is imported has no entry (`NoEntry::OnlyImported`). Imported is one definition, the rename pass's mark (`imported`, GH #1104 piece 5); the `__lib_` name the same pass gives is spelling, not a second test of the entry, and lowering reads the mark too (L4), so a program handed in with an unmarked `__lib_` main deploys it
- a module-nested `main` is not the entry (decision 2, E0): the entry is a top-level `main locus` of the seed's own files, so rule 9's closed world is the top-level one; a seed whose only `main` is module-nested has no entry (`NoEntry::OnlyModuleNested`), and a seed with both keeps the top-level one. Rule 1 still counts a module-nested `main` (GH #825): the count reads the witness, not the entry
- a seed whose only `main locus` is module-nested is refused (L4): one located error at that locus's name (`EntryRow::refused`, the `lowering_laws` law), so the check, the build, the editor and the harness agree and no program reaches lowering with a root that is not the entry. It is one more diagnostic, never a replacement: the placement table is seeded from `EntryRow::root`, the entry or else the refused `main`, so the placement-safety rules (the F.31 rule, the pinned-in-a-loop rule) and every other rule still judge it (GH #825; the outside review of #1293, finding 1). A seed with an entry is not refused for a module-nested `main` beside it
- with more than one candidate (rule 1's error) the entry is the last, as the checker's pool map (since replaced by the placement table) took it before the row
- the checker, the api binding and the environment, the model, the editor and the CLI read the row (F.40 phase 3, the entry's consumers), and so does lowering (L4): what reads the root before the check refuses a seed (the api binding and `--api`'s entry, an environment's constitutions, the model's `entrypoint`, the editor's placement view, the async_io advisory) reads `EntryRow::root`, the placement table's root, which is the entry in every program that reaches lowering; no legacy site is left, and the row has one definition of the main locus and one of `fn main` (`top_level_fn_main`)
- lowering reads the entry (L4) and derives no main locus and no `fn main`: the lowering view carries the row (`LoweringView::entry`); codegen's comparisons against the main locus read its entry by identity (`is_entry_locus`, `root_bindings`), never a type name or the first `is_main && !__lib_`; `fn main` is the row's column (`EntryRow::fn_main`, by its site), so `in_main` marks that declaration's body; the binding term (`binds_on_main`) counts the entry's entries by the locus's site, and the lifecycle plan's deferred main entry is the entry by identity
- the world is wider than the entry, on purpose: `EntryRow::world` is every `main locus` the bundle declares (the entry's, a module-nested one's, an imported application's), because decisions 1 and 2 say which declaration the seed runs, and an application's inline law keeps binding when another seed imports it (GH #733, `crates/hale-cli/tests/imported_main_claims.rs`); the claims read the row's column and gather no `is_main` of their own
- three readers of the `main` keyword read a property of each declaration and derive no entry fact, so they are no definition of the entry and no legacy row, whichever `main` is the entry: `check_nested_long_running_child` exempts every `main locus`, as a parent (`parent.is_main`) and as a child (`!l.is_main`), from the long-running-child rule, since a root is supervised by no parent; the ownership graph makes every `main locus` declaration a singleton (`singleton |= l.is_main`), one instance per declaration, deployed or not; and the allocation summary defers every `main locus` declaration, module-nested ones included, from eager reclamation (`if l.is_main` in its flat walk), conservatively. Each expression is a seam, so a second reader of the keyword in those files is a new row to classify
- every non-test reader of `LocusDecl::is_main` is the producer or one of the three per-declaration readers above; the binding rows copy the keyword into each row as a column (`is_main: l.is_main`), the parser's main-only member rules are syntax and are not rows, and `sync_inference`'s test helper is test code

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `entrypoint_a_view_without_the_entry_row_is_refused`; total: no entry in the row means no main locus is deployed, and no `fn main` column means the program has none, which lowering refuses as such

**Focused tests.** crates/hale-frontend/src/snapshot.rs (the_entry_row_is_the_seeds_own_top_level_main_locus); crates/hale-cli/tests/check_entry_decisions.rs; crates/hale-cli/tests/check_entry_consumers.rs (each consumer's correction on the decided shapes, beside a control); crates/hale-lsp/src/lib.rs (the_placement_view_is_the_deployed_roots); crates/hale-cli/tests/nested_main_transition.rs (after the transition, end to end: a seed whose only `main locus` is module-nested is refused by check and build, beside its own errors, and the top-level control is deployed); crates/hale-types/tests/checks_inside_modules.rs (the refusal's message and span; a module variant reports the control's messages, and the refusal as its one addition); crates/hale-cli/tests/offthread_imported_main.rs (the binding term counts the entry's entries); crates/hale-cli/tests/entry_point_placement.rs; crates/hale-types/tests/bus_graph.rs

**Spec.** spec/semantics.md § Bundle-wide rules

**Guarded seams.**

- `entry_row(` may be referenced from: `crates/hale-types/src/entry.rs` ×2, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/effects.rs` ×1, `crates/hale-cli/src/verbs/check/matrix.rs` ×1, `crates/hale-types/src/sync_inference.rs` ×1, `crates/hale-types/src/placement.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/lifecycle/derive.rs` ×1
- `is_entry_locus(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×3, `crates/hale-codegen/src/locus/instantiation.rs` ×2, `crates/hale-codegen/src/locus/dissolve.rs` ×2, `crates/hale-codegen/src/locus/decl.rs` ×1
- `world()` may be referenced from: `crates/hale-types/src/claims.rs` ×1
- `parent.is_main` may be referenced from: `crates/hale-types/src/check.rs` ×1
- `!l.is_main)` may be referenced from: `crates/hale-types/src/check.rs` ×1
- `singleton |= l.is_main` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1
- `if l.is_main {` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×1

### `ownership` — Canonical · derivation

**Answers.** Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array.

**Inputs.** locus declarations (params, accept, release); bodies (let, assign, return, field initialisers, placement entries); fresh factories (one producer); returned bindings; placement (the table: each bubbling edge's class, per instance)

**Producer.** `crates/hale-types/src/ownership.rs` · `resolve_owners`

**Also owned.** `crates/hale-types/src/ownership.rs` · `resolve_binding_facts`; `crates/hale-types/src/ownership_graph.rs` · `bubble_plans`; `crates/hale-types/src/ownership_graph.rs` · `compute_forwarding_sets`; `crates/hale-types/src/ownership_graph.rs` · `classify_owner_kind`; `crates/hale-types/src/ownership_graph.rs` · `classify_edge`; `crates/hale-types/src/ownership.rs` · `fresh_factories`

**Consumers.** codegen (`crates/hale-codegen/src/locus/instantiation.rs` · `site_owner`); codegen (a monomorph's accept rows: its template's, specialized at synthesis) (`crates/hale-codegen/src/codegen.rs` · `specialized_accepts`); codegen (whether the enclosing locus accepts the child it births) (`crates/hale-codegen/src/locus/instantiation.rs` · `parent_accepts_us`); borrow_lifetime (a bare literal's enclosing locus accepts its child: `accepts_ancestor`, over the snapshot's graph's rows) (`crates/hale-types/src/borrow_lifetime.rs` · `accepts_ancestor`); lowering view (its graph: the snapshot's rows through the correspondence, then the stdlib's; the bubble plans, `accepts` and the accept rows lowering reads) (`crates/hale-types/src/resolved.rs` · `lowering_ownership_graph`); model (dynamic births: the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `demand_ownership_graph`); a declaration's dependents (X2: a locus's births, accepts and instantiations make its neighbours through the ownership graph, the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `declaration_dependents`); check (type-check rule 20, the unowned-subscriber rule: `owner_of_site` over the snapshot's graph, handed in through `CheckInputs`) (`crates/hale-types/src/check.rs` · `check_unowned_subscriber_locus`); alloc_summary (eager-only accept sets); check and the harness (the cross-pool spawn law reads the bubble plan) (`crates/hale-types/src/lowering_laws.rs` · `cross_pool_spawn_used_as_a_value`); codegen (the handed-back column: a root some literal hands back keeps its pinned fields' join records in their instances, C52) (`crates/hale-codegen/src/codegen.rs` · `collect_main_placement`)

**Invariants.**

- a locus instantiation with no row is a CodegenError (F.39)
- ids, not names or spans: declarations are cloned and the stdlib's coordinates overlap user files
- the ownership matrix stays green with an empty KNOWN_OPEN
- the model's `Owns` edges are the placement table's `owner` column projected with the arrangement (P1 4 of 6, C3): an arranged instance is owned by the arranged instance its row names as owner; `owns.push(` has that one writer
- the model reads construction context from the ownership graph (C3): the shared walk records body births, field defaults and binding adapters with their explicit fields and literal identities. `unarranged_births` follows the defaults each construction actually evaluates; the same default can be arranged for one holder and dynamic for another, while an overridden default adds no birth. The arrangement projection (`project_arrangement`, which the model's rows are made from) supplies the literal identities its placement rows represent, including held sources, and joins each remaining birth by resolved declaration site (name fallback only for unminted bundles). No model-side params-span test or free-function walk remains; copied API-binding expressions retain their construction context despite overlapping spans
- the graph keeps each `accept` param as a row of the declaring locus's identity, its type as written (`AcceptRows`); a monomorph's accept set is its template's rows, asked for by the template's identity and specialized by the instantiation's substitution (`AcceptRows::specialize`, as `FlowRows::specialize` does for release clauses), filled at synthesis; lowering never reads a monomorph's own `accept_param` for ownership
- `fresh_factories` is read by lowering and the checker with the bundle's import renames; a factory's returned name is the declaration the snapshot resolves it to, so a fn whose returned name an inner `let` shadows is a factory of the outer binding (the #1140 shape; its escape walk still reads every binding spelling the name as the returned one, the conservative side). The carrier-arm extension (`extend_fresh_factories`) is one set both read: lowering through the owner table, and the checker's self-containment law through `extended_factory_rows`, which gives each fn the fold adds a row whose products are its arms' (F.40 phase 3, C5: the decision measured no diagnostic moving over the corpus, `tests/hale` and the DNA seeds; the fold's one addition there, in the DNA core, already had a row)
- which declaration a returned or escaping name denotes is read from the snapshot (`Snapshot::declaration_of` over `binding_of`, resolved once by the mint), never resolved again: `returned_bindings` (the binding facts and the pre-pass), `fresh_factories`, borrow_lifetime's `returned_decls` and alloc_summary's escape tags each key a binding by its declaration's SiteId; a `let` or a use the snapshot did not mint answers by name in `returned_bindings` (the conservative side) and resolves to nothing elsewhere, and every entry point mints
- the checker builds no graph: the snapshot demands it before the check and hands it in (`CheckInputs::ownership`), so a check with a law builds it once for the checker and the model together; a bundle no snapshot holds (the test entries) builds it once, through the producer (`build_ownership_graph`), and the model over that bundle reads the same build
- one graph per snapshot (F.40 phase 3, C5): the graph keeps the rows it is assembled from (`OwnershipRows`), and lowering's graph is the snapshot's rows, each site and accepting locus found in the merged program through the view's correspondence, followed by the stdlib's rows over the merged program's tail (`stdlib_ownership_rows`), the one part no snapshot holds; one procedure assembles either (`lowering_ownership_graph`)
- the tower's accept relation is one relation, `accepts_ancestor` (`OwnershipRows::accepts_ancestor`): the climb finds an owner by it, and the borrow-lifetime law asks it of a bare literal's enclosing locus, over the snapshot's graph's rows (a bundle no snapshot holds walks its own, `OwnershipRows::of`)
- rule 20 is judged by declaration identity (`owner_of_site`): the graph lists every locus declaration in declaration order, and a site carries the declaration its literal names (`child_decl`, joined by the shared resolver's declaration identity, with a name fallback for unminted input) and the child an `accept` must name to own it (`child_key`: an import or `std::` path resolved as an `accept`'s type is, a template specialized by its binding's or field's declared type); the nearest accepting ancestor owns a birth in a handler as it owns any other, but only if one accepts the child on every construction path of the enclosing locus (`construction_paths`, over the placement table: each instance row under its owner row's declaration, a template's top as a root with no ancestor, a literal directly in `fn main` included, each dynamic site under its enclosing locus or in a free fn whose callers are not followed; with the walk's own body and params-default edges, since the table does not enumerate a dynamically built locus's params subtree), and the diagnostic names a path with none. `child_ty`, `resolution` and bubble plans use the resolved child name, following imports and aliases and retaining a known generic specialization; plain records and unresolved paths are not births. Lowering's bundle carries its minted snapshot, so the declaration join keeps its identity. Lowering's `instantiated_by` still records no free-fn birth
- the holes: where the graph cannot decide, `owner_of_site` answers None and rule 20 does not fire, since unknown ownership is not proven absence: an open world (no entry, so a consumer may complete the tower), a generic template no declared type specializes, a qualified path that names no declaration, and an unanalyzable site; a literal assigned to a field of `self` is judged by the `accept` edges, since the graph records no assignment target. A construction path the placement table records as a hole cuts the other way, since an unknown path cannot prove an owner: a held instance the table does not link, a field whose initializer is no literal, a dynamic site of unknown domain and an owner row whose declaration does not resolve each count as a path with no ancestor, and so does a free fn; a held row the table links adds no path (its source row is the construction)
- a bubbling edge's class reads the placement table per instance: each row of the enclosing locus is paired with the row that owns it, every other instance with every domain of the owner; SameTower when every pair shares its domain, CrossPool when none does and the owner has one, Mixed otherwise or where an instance runs where the table cannot say. The graph reads no `placement { }` block itself
- the owner is a fact of the site, the mechanism of the instance (U-1): a Mixed edge keeps its resolved owner, takes the same-tower birth or the cross-pool post per enclosing instance where the owner is a singleton on main (`lotus_on_main_thread` at the literal), and is refused at the literal (`CodegenError::UnsupportedAt`, naming every instance and its domain) for a value use or a non-singleton owner; it is never lowered as a transient birth

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `ownership_an_instantiation_without_its_owner_row_is_refused`; total: no accept row means the enclosing locus does not accept the child, no bubble plan means no ancestor owns it, and no factory row means the fn is no fresh factory

**Focused tests.** crates/hale-types/tests/model_arrangement.rs; crates/hale-codegen/tests/owner_table.rs; crates/hale-codegen/tests/ownership_matrix.rs; crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding); crates/hale-codegen/tests/ownership_bubble.rs; crates/hale-cli/tests/check_unowned_subscriber.rs (rule 20, one test per class of the migration); crates/hale-types/tests/ownership_graph.rs (lowerings_graph_is_the_snapshots_rows_through_the_correspondence); crates/hale-types/tests/self_containing_locus.rs (a_factory_returning_a_carrier_is_reported, a_carrier_with_an_accessor_arm_is_not_a_cycle: the checker reads the extended factory set); crates/hale-cli/tests/check_borrow_lifetime.rs (the law over `accepts_ancestor`); crates/hale-types/tests/ownership_graph.rs (the O rows: nested under pinned, on the owner's pool, per-instance pairing, adapter, mixed); crates/hale-codegen/tests/ownership_bubble_mixed.rs (U-1: both instances' owner, count, retention, birth thread and teardown, both arms and ASan; the transient control; the located refusal); crates/hale-codegen/tests/ownership_bubble_crosspool.rs (O-1 under ASan)

**Spec.** spec/decisions.md F.39; spec/semantics.md § Dissolve timing rules; spec/semantics.md § Placement block (type-check rule 20); spec/semantics.md § Locus instantiation (accept bubbling: the owner per site, the delivery per instance); spec/runtime.md § Interest-based ownership (accept bubbling)

**Guarded seams.**

- `resolve_owners(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/ownership.rs` ×1
- `build_ownership_graph(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `stdlib_ownership_rows(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `lowering_ownership_graph(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `accepts_ancestor(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×3, `crates/hale-types/src/borrow_lifetime.rs` ×1
- `fresh_factories(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/ownership.rs` ×2
- `extended_factory_rows(` may be referenced from: `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/lowering_laws.rs` ×1
- `extend_fresh_factories(` may be referenced from: `crates/hale-types/src/ownership.rs` ×3
- `resolve_binding_facts(` may be referenced from: `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `returned_bindings(` may be referenced from: `crates/hale-types/src/ownership.rs` ×3
- `bubble_plans(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/lowering_laws.rs` ×1
- `owns.push(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×1
- `owner_of_site(` may be referenced from: `crates/hale-types/src/check.rs` ×1
- `demand_ownership_graph(` may be referenced from: `crates/hale-frontend/src/snapshot.rs` ×8, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1

### `bus_graph` — Canonical · derivation

**Answers.** The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates.

**Inputs.** topics; bus blocks; sends; bindings; placement (the table: every label, and so the direct-call gate)

**Producer.** `crates/hale-types/src/bus_graph.rs` · `build_bus_graph`

**Also owned.** `crates/hale-types/src/bus_graph.rs` · `dispatch_gates`; `crates/hale-types/src/bus_graph.rs` · `cycle_from`; `crates/hale-types/src/bus_graph.rs` · `external_handlers`

**Consumers.** check (rule 9: the wire rows, their bound, cross-seed and wildcard columns, over the entry row's closed world; the snapshot's graph, `CheckInputs::bus`) (`crates/hale-types/src/check.rs` · `check_bus_graph`); check (rule 10: the edges by declaration, through `cycle_from`, each edge's send joined to the intra-locus relation by its id, `CheckInputs::intra_locus`) (`crates/hale-types/src/check.rs` · `check_bus_cycles`); check (rule 7: the placed declaration's row, `external_handlers`) (`crates/hale-types/src/check.rs` · `check_cooperative_pool_blocking`); check (rules 11, 12, 19); the role law (each locus declaration's sites, `PublishRow::decl` and `SubscribeRow::decl`, the topic each names as written, `topic`, where a subscriber's handler is named, `handler_span`, and whether the declaration is imported, `LocusDeclRow::imported`) (`crates/hale-types/src/roles.rs` · `role_laws`); model (subjects, endpoints and gates: the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `demand_bus_graph`); topology; dispatch; lsp (hale/busGraph: the model's graph, so eligibility is the diagnostics pass's) (`crates/hale-lsp/src/lib.rs` · `demand_bus_graph`); codegen (a rewritten publish, found by its call's id in the relation: the probes and the reclaimed subregion) (`crates/hale-codegen/src/codegen.rs` · `intra_locus_rewrite`); lowering view (its graph: the snapshot's rows through the correspondence, each user site keyed by the topic rewrite's wire, then the stdlib's; the plan lowering reads is its gates) (`crates/hale-types/src/resolved.rs` · `lowering_bus_graph`); the no-snapshot entries (`check_bundle`, `check_bundle_opts_scoped`, `claim_law_diags`, the hale-types tests, the artifact's bundle entry, and `resolve_program` for a bare program): the producer itself, once per entry, over the entry's scope (`crates/hale-types/src/lib.rs` · `build_bus_graph`)

**Invariants.**

- one graph, over one program shape, per snapshot; rule 10's cycle graph is a query over it (`cycle_from`)
- lowering derives no graph of the user's program: its graph is the snapshot's rows (`BusRows`), each user site found in the merged program through the view's correspondence and keyed by the wire the topic rewrite gave it, followed by the stdlib's rows over the merged program's tail (`stdlib_bus_rows`), the one part no snapshot holds; the subjects and their gates are assembled from the rows by one procedure (`BusRows::subjects`) on both sides
- a bundle no snapshot holds builds its graph through the snapshot's producer (`build_bus_graph`), never through a wrapper of its own
- the checker's bus rules (7, 9, 10) compare subjects under the canonical key, the wire subject (`Subject`, `wires`): a topic published by name and subscribed by its literal subject is one subject; the gates and the model keep `BusSubject::canonical()`'s keys (`subjects`)
- an edge belongs to the locus declaration that wrote its handler (`BusEdge::decl`), never to a name: two loci of one name have their own edges
- a subject the graph cannot resolve is a hole (`holes`): it forms no edge and no rule calls it an orphan
- the intra-locus rewrite is a relation on the graph, never an erased publisher (boundary 7)
- lowering reads the relation (`LoweringView::intra_locus`) by the call's id, which the call that replaces a send keeps (`IntraLocusRewrite::send`); it never classifies a call as a rewritten publish from the call's shape
- rule 10 calls a cycle synchronous only where the relation holds every send of it (`BusEdge::send`, the snapshot's `intra_locus` stage, the one lowering continues from); a same-declaration cycle with a send the relation does not hold is carried by the queue, and the subjects' spelling never decides which
- a placement label is the placement table's answer for the type, by the name lowering keys on: `SameThread` only when every instance runs on main (a nested instance inherits its owner's domain, an adapter is pinned, an instance the table cannot place is `Unknown` and never main); the graph reads no `placement { }` block itself

**Missing data.** total: no row in the intra-locus relation means the call is the one the author wrote (the relation names every send the rewrite replaced, and the call keeps the send's id); within the graph, an unresolved subject stays a hole (`holes`), and the plan's subjects are the `dispatch` family's

**Focused tests.** crates/hale-types/tests/bus_graph.rs (lowerings_graph_is_the_snapshots_rows_through_the_correspondence); crates/hale-types/tests/lowering_correspondence.rs; crates/hale-types/tests/bus_graph.rs (the B rows: nested, imported root, qualified field, last-segment collision, two domains, adapter); crates/hale-types/tests/bus_rules_over_graph.rs; crates/hale-types/tests/bus_payload_handler.rs; crates/hale-codegen/tests/bus_devirt_differential.rs; crates/hale-codegen/tests/nested_offthread_delivery.rs (the dispatch-plan flavor of a nested subscriber, a nested publisher and an adapter's publication)

**Spec.** spec/semantics.md rules 7, 9-12, 19; spec/verification.md § Bus-graph property checks

**Guarded seams.**

- `build_bus_graph(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `collect_bus_walk(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×2
- `stdlib_bus_rows(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `lowering_bus_graph(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `dispatch_gates(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `cycle_from(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-types/src/check.rs` ×2
- `external_handlers(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-types/src/check.rs` ×1

### `topics` — Canonical · derivation

**Answers.** What each topic is on the wire: its subject, payload contract, routing key, bounds and shed policy; and which topic a send's subject names.

**Inputs.** topic declarations; send subjects; subscribe bounds; bindings (shm_ring)

**Producer.** `crates/hale-types/src/topic_identity.rs` · `topic_wire_subjects`

**Also owned.** `crates/hale-types/src/topic_identity.rs` · `TopicRows::of`; `crates/hale-types/src/topic_identity.rs` · `by_wire`

**Consumers.** check (the topic rows, built once per bundle on the TopScope) (`crates/hale-types/src/resolve.rs` · `build_top_scope`); model (the scope's topic rows: each topic's wire, and the gate merge per wire) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); codegen (the lowering view's rows: dispatch, bindings, codecs, runtime shape registration) (`crates/hale-codegen/src/codegen.rs` · `topics: &resolved.top.topics`); codegen (the shm-ring bindings: each bound topic's wire and payload, from its row) (`crates/hale-codegen/src/codegen.rs` · `collect_shm_ring_subjects`); codegen (the routing-key, on_full-fail and subscriber-bound tables, from the rows) (`crates/hale-codegen/src/codegen.rs` · `collect_routing_key_subjects`); topology (topic shapes); resolved program (the intra-locus relation's wire subjects) (`crates/hale-types/src/resolved.rs` · `topic_wire_subjects`)

**Invariants.**

- delivery joins on the subject's identity, never on the written topic name (spec/model.md rule 8)
- a literal subject at a delivery site (a literal subscription, a literal send) names only the topic that OWNS that wire subject, `TopicRows::by_wire`, never one whose declared segment or name it happens to spell; a topic reference names its declaration; a wire subject two topics carry names neither and is an error
- lowering reads the topic rows of the lowering view's scope (`Cx::topics`, over the program it lowers): a topic's wire subject, payload, routing key, bound and on_full policy come from its row, and codegen computes no wire subject of its own

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `topics_a_declared_topic_without_its_row_is_refused_at_its_name`; total: no row for a subject means it is already a wire (`topic_wire`, after the topic rewrite); a topic declaration's name, a binding entry's topic and a rewritten send's topic are required (`Cx::topic_row`)

**Focused tests.** crates/hale-codegen/tests/topic_declarations.rs; crates/hale-codegen/tests/replica_keys.rs; crates/hale-codegen/tests/serializer_shape.rs; crates/hale-types/src/topic_identity.rs (a_subject_names_its_topic_by_one_rule, a_shared_subject_names_no_topic)

**Spec.** spec/semantics.md § Topic declarations; spec/semantics.md § Phase 3: routing keys

**Guarded seams.**

- `topic_wire_subjects(` may be referenced from: `crates/hale-types/src/topic_identity.rs` ×2, `crates/hale-types/src/resolved.rs` ×1
- `TopicRows::of(` may be referenced from: `crates/hale-types/src/topic_identity.rs` ×1, `crates/hale-types/src/resolve.rs` ×1
- `by_wire(` may be referenced from: `crates/hale-types/src/topic_identity.rs` ×6, `crates/hale-types/src/check.rs` ×2

### `bindings` — Canonical · derivation

**Answers.** Which topics are bound to which transport, in which role, with which codec, and whether the transport can carry the payload.

**Inputs.** bindings blocks; topics; transport specs; purity (codecs)

**Producer.** `crates/hale-types/src/binding_rows.rs` · `derive_binding_rows`

**Consumers.** check (the binding rules walk the rows: topic, duplicate, role, adapter, ring layout, constraints, codec; the `or wait` legality check reads the bound-topic set) (`crates/hale-types/src/check.rs` · `check_main_and_bindings`); the role law (a gated handler's topic is not bound to a transport: the bound-topic set, keyed by the topic rows' wire) (`crates/hale-types/src/roles.rs` · `role_laws`); check (a binding's `where` constraints are held to its transport's guarantee: the capability module's table, read through the row's transport kind) (`crates/hale-types/src/capability/transport.rs` · `guarantee`); the transport's cell on the effective target: `RemoteTransport(kind)` × the target row's backend, read through the snapshot (verdict-neutral: the adapter's wasm refusal is a late link refusal today, a known-open cell) (`crates/hale-frontend/src/snapshot.rs` · `binding_cell`); model (main's binding thread domains: the role and the transport kind) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); bus graph (the bound-topic set, at both grains, is the rows' projection) (`crates/hale-types/src/bus_graph.rs` · `collect_bus_walk`); codegen (the prelude, the shm-ring subjects, the codec thunks and the pinned adapter loci read each entry's transport kind, role, codec and producer-versus-attach from its row, through the lowering view; the entry's own text supplies the transport's parameters; a missing row is an error) (`crates/hale-codegen/src/codegen.rs` · `root_bindings`); codegen (whether a thread crosses the bus boundary: an entry of the entry's `bindings { }`, `binds_on_main`, beside the placement table's domains) (`crates/hale-codegen/src/codegen.rs` · `program_has_offthread`); api_surface

**Invariants.**

- the transport-loss handler is named by the row: a `unix` entry's row carries the stdlib locus its transport instantiates (`loss_locus`, by role), lowering instantiates that locus, and a connect entry's is the locus whose failure the main locus's `on_failure` handles, so the handler is main's routing row for the locus the bindings row names, not one picked by a spelled name
- lowering holds the rows (`LoweringView::bindings`, the snapshot's) and finds an entry's by the id the mint kept (`BindingRows::for_entry`); it decides no transport, role, codec or producer-versus-attach itself, and an entry with no row is a `CodegenError`, not a guess
- F.36 and F.37: binding failure is structural; codec purity is a law over rows
- one row per snapshot (`Snapshot::demand_bindings`, the `bindings` count): one row per `bindings { }` entry of every locus of the bundle, an imported main's and a module-nested one's included, each with the entry's site, the topic and its wire key, the transport kind, the role, the codec, whether the bundle produces the topic and the stdlib locus a transport's loss surfaces through; the checker builds none (`CheckInputs::bindings`), and a bundle no snapshot holds builds it once
- the role is decided once, over the topic's ends read by wire subject (`desugar::role_from_ends` over the row's `publishes` and `subscribes`): the entry's own role wins, otherwise publish-only is `Connect` and subscribe-only is `Listen`, and a `unix` entry with neither is the checker's diagnostic. The checker, the model and lowering read it; the desugar's in-place fill applies the same pure rule over the topic names before the topic rewrite erases them, and agrees with it over the corpus
- what a transport carries is data beside the matrix, not a branch in the checker: the transport kind's guarantee for each `where` constraint (`capability::transport::GUARANTEES`, three rows of four cells, the former `transport_satisfies` cell for cell, its words verbatim), which does not vary by target; and whether a target realizes the transport at all is the matrix's own `RemoteTransport(kind)` row, asked through the snapshot's target row (`Snapshot::binding_cell`)
- the bound-topic set is the rows' projection (`bound_names`, `bound_subjects`): the `or wait` legality check, the role law's bound-topic rule and the bus graph's eligibility gate read it, and none walks `bindings { }` itself. An imported main's entries are in the set, as they were in each of the walks it replaces

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `bindings_an_entry_without_its_row_is_refused_at_its_topic`; total: no entry in the entry row means no `bindings { }` to lower

**Focused tests.** crates/hale-cli/tests/binding_imported_main.rs; crates/hale-codegen/tests/bindings_codec_clause.rs

**Spec.** spec/decisions.md F.36, F.37; spec/semantics.md § Operational constraints (Form K)

**Guarded seams.**

- `derive_binding_rows(` may be referenced from: `crates/hale-types/src/binding_rows.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2
- `role_from_ends(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×2, `crates/hale-types/src/binding_rows.rs` ×1

### `dispatch` — Migrating · derivation

**Answers.** How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement.

**Inputs.** bus_graph (gates, the payload_flat column among them); placement (domains: the arrangement projection, `project_arrangement`, with the ownership graph's births outside it); --no-bus-devirt

**Producer (today's authority, migrating).** `crates/hale-model/src/dispatch_plan.rs` · `fn derive`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/resolved.rs` · `from_gates` — the plan is derived twice, from two gate sets: the model's (`derive`) from the checked graph's gates, lowering's (the resolved program) from the lowering graph's, which are the same rows re-keyed by wire plus the stdlib's; by one function (`from_gates`) with one domain map (`domain_map` over the arrangement projection, keyed by the gates' spelling of a locus); held equal on the subjects the model's gates name by the law (`dispatch_plan_law.rs`: every column, the subscriber column over the model's loci and in each plan's own order). *Removed when:* phase 4, with `stdlib_surface`: one derivation needs the stdlib's rows at the snapshot, so that the model's gates and lowering's are one set (F.40 phase 3, C5 2 of 2, restated).

**Also owned.** `crates/hale-model/src/dispatch_plan.rs` · `domain_map`

**Consumers.** codegen (`crates/hale-codegen/src/codegen.rs` · `build_resolved`); codegen (`crates/hale-codegen/src/bus/dispatch.rs` · `bus_devirt`); exec_digest (the resolved program's plan) (`crates/hale-cli/src/shared/options.rs` · `resolved.plan.digest()`); model dump

**Invariants.**

- which flavour a subject gets is a plan conclusion, never a model row (spec/model.md)
- lowering reads one plan, derived once per snapshot in the resolved program; the execution digest frames that plan's digest, which covers what lowering reads (each subject, its flavor and its subscribers) and keeps a reserved 0 byte where `same_domain` sat, until the same-domain flavors (GH #464) lower by it
- both plans take their domains from one function, `domain_map`: the arranged (locus, domain) pairs minus every locus the arrangement does not fully place (a template disagreement or a birth outside it), keyed by the gates' spelling of a locus, the raw post-merge symbol, never a display name; the model feeds it its arrangement rows and placement holes, lowering the projection (`project_arrangement`) those rows are made of, the snapshot's one (`Snapshot::demand_arrangement`), so lowering demands no model
- the model's plan is lowering's on every subject the model's gates name, column for column, the domain lists and `same_domain` included; the subscriber column is compared over the model's loci and in no order (the model's gates hold subscribers sorted, lowering's in registration order; a subject the stdlib's rows also subscribe carries their loci in lowering's row only), over the corpus examples, tests/hale and the DNA mains, build and harness snapshots
- the direct tier takes all three gate legs, same-thread, quiet and the payload_flat column (`bus_graph::payload_is_flat`, codegen's flatness rule over resolved types); codegen reads the flavor and refuses a plan whose column disagrees with the lowered payload, and the codec's own flatness equals the column at every publish over the corpus

**Missing data.** total: no row means the subject dispatches dynamically: no static plan row names it, at its publish and its register alike

**Focused tests.** crates/hale-cli/tests/dispatch_plan_cli.rs; crates/hale-codegen/tests/bus_devirt_direct.rs; crates/hale-cli/tests/dispatch_payload_flat.rs (every wire payload alternative through both publish arms against the codec, the column against the codec at every publish over the corpus, the plan change recorded as a compatibility change: --dump-model's row, a pre-change recording refused by its exec digest and admitted with --allow-unverified-model, a post-change recording replayed); crates/hale-types/tests/dispatch_plan_law.rs (the model's plan is lowering's on the shared subjects over 334 views, with a control per column); crates/hale-types/tests/dispatch_plan.rs (an_imported_seeds_loci_have_their_domains: a two-seed fixture's imported loci have their domains, same-domain on main); crates/hale-model/src/dispatch_plan.rs (same_domain_is_no_part_of_the_digest)

**Spec.** spec/model.md § Derived products; spec/decisions.md F.38; spec/runtime.md § Placement classes (the dispatch plan)

**Guarded seams.**

- `DispatchPlan::derive(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×1
- `from_gates(` may be referenced from: `crates/hale-model/src/dispatch_plan.rs` ×2, `crates/hale-types/src/resolved.rs` ×1
- `domain_map(` may be referenced from: `crates/hale-model/src/dispatch_plan.rs` ×1, `crates/hale-types/src/arrangement.rs` ×1

### `handler_routing` — Canonical · derivation

**Answers.** Which `on_failure` handler a failing child's locus type reaches, and from which parent.

**Inputs.** failure declarations; declared loci and type aliases (the bundled stdlib's loci included); import renames; ownership (the supervising parent instance, in lowering)

**Producer.** `crates/hale-types/src/handler_routing.rs` · `handler_rows`

**Also owned.** `crates/hale-types/src/handler_routing.rs` · `child_locus_name`; `crates/hale-types/src/handler_routing.rs` · `resolve_locus_type`

**Consumers.** lowering view (its rows: the snapshot's through the correspondence, then the stdlib's, the indexes rebuilt over the union) (`crates/hale-types/src/resolved.rs` · `lowering_handler_routing`); codegen (the handler table, one fn per row keyed by the row's site) (`crates/hale-codegen/src/locus/decl.rs` · `handlers_of_instance`); codegen (handler bodies, by the row's site) (`crates/hale-codegen/src/locus/method.rs` · `handlers_of_instance`); codegen (concrete handler rows at locus synthesis) (`crates/hale-codegen/src/codegen.rs` · `self.handlers.specialize`); codegen (a route: the row's handler fn) (`crates/hale-codegen/src/channels/mod.rs` · `failure_handler_for`); codegen (__parent_on_failure) (`crates/hale-codegen/src/channels/mod.rs` · `resolve_failure_route`); codegen (the params-settle bracket: whether the locus has a handler) (`crates/hale-codegen/src/locus/instantiation.rs` · `settles_failures`); codegen (restart in place) (`crates/hale-codegen/src/locus/restart.rs` · `restarts_in_place`); model (supervises, over the snapshot's rows: `demand_handlers`) (`crates/hale-types/src/model_builder.rs` · `Supervises`); check (duplicate handlers, over the snapshot's rows handed in: `CheckInputs`) (`crates/hale-types/src/check.rs` · `check_duplicate_failure_handlers`); check (@supervised, over the same rows) (`crates/hale-types/src/frontier.rs` · `supervised_diags`); ownership births (resolved child, declaring template and identity) (`crates/hale-types/src/ownership_graph.rs` · `identify_child`)

**Invariants.**

- the child type is resolved once, by `child_locus_name`; lowering, the checker and the model read the same row
- a row carries its parent declaration's site (`parent_id`) and a reader holding a locus declaration asks for its rows by that identity (`handlers_of_decl`, `route_decl`): a monomorph keeps its template's id and selects concrete rows by that identity and its specialization name (`handlers_of_instance`, `route_instance`); lowering's handler fn is a column of the row, held per locus keyed by the row's site (`LocusInfo::failure_handlers`), and the handler table, the body pass and a route each join by that site, never by the row's ordinal
- a row carries the site of the declaration its child resolves to (`child_decl`, a monomorph's template's, from the same resolution as `child`: `child_locus`), qualified by the store that minted it; the model's supervision rows join parent and child to its locus table by those sites, and a child declared outside the snapshot's programs (a stdlib locus) is `SupervisedRef::External`, whose written name is its display, never a join key; only a bundle no entry point minted joins by name
- the model's failure-handler function rows are keyed by the handler's site (`handler_fn_rows`), the routing row's identity; the signature string is the row's name, which ranks it among the function rows (two handlers are two rows whatever their signatures spell), and no reader looks a handler up by it
- a generic supervisor's child types are substituted at synthesis by the handler producer (`specialize`) using the same substitution as the locus; each concrete row preserves its template handler's site and recovery ops, resolves the concrete child's declaration in the original bundle, and is indexed by template identity and specialization name. Dispatch, handler body layouts and restart-in-place attribution read those concrete rows; the declaration-level snapshot rows are unchanged
- the checker builds no rows: the snapshot demands them before the check (`CheckInputs`), and the checker's duplicate-handler rule, the `@supervised` law and the model read that one build; a bundle no snapshot holds (the test entries) builds them once, in `bundle_handler_rows`
- a build path derives the rows once (F.40 phase 4, Q1): the snapshot's (`demand_handlers`, over the checked programs, the `handler_routing` count), which the check, the model and the lowering view read. The view reads them as C5 folded the ownership and bus graphs (`lowering_handler_routing`): every user row carried across, its handler and parent sites each the merged site's `Image::Checked` (a row that is not is refused, by name); the stdlib's rows derived over the merged program's tail only (`stdlib_handler_rows`, its child types resolved against the whole merged program); and the indexes, the failure column and the span-keyed bounds rebuilt over the union, the stdlib's entered after the user's, as the walk over the merged program entered them (the stdlib writes no recovery bound, so none of its spans, which overlap the first file's, answers for a user statement). A carried row's `child_decl` is the snapshot's (a stdlib child `SiteRef::stdlib`); no lowering reader reads it, and lowering's joins (`is_row_of`, `handlers_of_decl`, `specialize`, `instance_key`) read site indexes the merged mint kept
- a route's parent instance is the lowering frame's (`resolve_failure_route`): a runtime pointer (the supervising parent's, else `current_self`, else `params_init_self`), which no snapshot row holds; its handler is the row's, and the one join in it, the supervising parent a field literal records matched to the child, is by the child's identity as the rows key a concrete locus (`HandlerRouting::instance_key`: its declaration's site, a monomorph's with the specialization `specialize` registered), never by name; a declaration no mint numbered is keyed by its name (C3 rest)

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `handler_routing_a_handler_without_its_row_is_refused`; total: no route means the parent declares no handler for the child, whose failure takes the unhandled route, and no handler row means the locus settles no failures

**Focused tests.** crates/hale-codegen/tests/lifecycle_flow.rs (on_failure_dispatch_by_child_type); tests/hale/on_failure_per_child_type_test.hl; crates/hale-types/tests/violate.rs; crates/hale-types/tests/handler_routing_probes.rs

**Spec.** spec/semantics.md § failure; spec/runtime.md (failure delivery)

**Guarded seams.**

- `handler_rows(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1
- `child_locus_name(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/ownership_graph.rs` ×2, `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/flows.rs` ×1
- `child_locus(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×3
- `resolve_locus_type(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×3, `crates/hale-types/src/ownership_graph.rs` ×2
- `DeclaredNames::of(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×2, `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/flows.rs` ×1
- `stdlib_handler_rows(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `lowering_handler_routing(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/resolved.rs` ×1

### `flows` — Canonical · derivation

**Answers.** Which children are flows (released per completion) and which are resident; and, per locus declaration, whether its `run()` is long-running and whether it never returns.

**Inputs.** release declarations; accept declarations; run bodies; declared loci and type aliases (handler_routing's resolver); import renames

**Producer.** `crates/hale-types/src/flows.rs` · `survey`

**Consumers.** check --flows (each flow type as written, with its clauses: the snapshot's rows) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_flows`); check (a daemon-shaped locus that accepts a child type it releases no clause for: a law over the rows) (`crates/hale-types/src/check.rs` · `check_accept_release`); check (the run rows: the long-running-child rule's long-running column, the starvation and birth-order laws' never-returns column; the snapshot's rows handed in, `CheckInputs::flows`, shared by all four) (`crates/hale-types/src/check.rs` · `run_of`); lowering view (the snapshot's rows, handed in: lowering reads them by locus name, so they need no correspondence) (`crates/hale-types/src/resolved.rs` · `let flows = flows.clone()`); a declaration's dependents (X2: a flow child and its `release` owners are neighbours, `Snapshot::declaration_dependents`) (`crates/hale-frontend/src/snapshot.rs` · `fn dependency_index`); the lifecycle plan (an accepted flow is torn down by the reclaim its run's end runs, a resident by its owner's cascade: the snapshot's rows over the checked programs) (`crates/hale-frontend/src/snapshot.rs` · `fn demand_lifecycle`); codegen (run elision, run-end reclaim and the release call: `Cx::is_flow`, one row read) (`crates/hale-codegen/src/codegen.rs` · `is_flow`); codegen (the generic-instantiation queue: each locus specialization it creates asks the row for its template's clauses, under the substitution its synthesis applies) (`crates/hale-codegen/src/codegen.rs` · `specialize(`)

**Invariants.**

- a release clause's child is resolved once, by `child_locus_name` (handler_routing's resolver: aliases, generic instantiations, qualified paths), into the row (`FlowClause::locus`); lowering's flow-ness is a row read (`flows::is_flow`), never a comparison of its own
- the flow facts cover the specializations lowering creates: a clause whose type mentions its owner's type parameters names no locus by itself and carries its template (`FlowClause::template`: the owner's identity, its parameters in order, the type as written); `FlowRows::specialize` answers for one specialization by resolving the template's type under the substitution lowering's synthesis applied, so `Manager<Worker>`'s `release(c: T)` makes `Worker` a flow exactly as a concrete `release(c: Worker)` does
- the checker's accept/release rule judges over the rows: the release clauses a locus declares are the rows' clauses inside its declaration
- every locus declaration, a module's included, has a run row (`RunRow`: the declaration's name and span, which `FlowRows::run_of` finds it by, and two columns, long-running and never-returns, the `nonreturning` family's two definitions); the checker surveys no rows: its four readers share the snapshot's, handed in (`CheckInputs::flows`)
- one survey per snapshot (F.40 phase 4, Q1): `flows` is a counted family (`Snapshot::demand_flows`, over the checked programs), and the check, the lifecycle plan, the lowering view, the editor's dependents relation and `check --flows` read that one. The view needs no correspondence: lowering reads the rows by locus name (`is_flow`, `specialize`), the stdlib declares no `release` clause and no type alias, and the resolver's declared loci hold the stdlib's over either program, so the merged program's survey was the checked one's in every clause row; its run rows for the stdlib's loci have no lowering reader. A bundle no snapshot holds (the test entries `check_bundle`, `check_bundle_opts_scoped`, `resolve_program`) surveys for itself

**Missing data.** total: no row means the child is resident: no `release` clause names it, so its owner's cascade tears it down (`Cx::is_flow`)

**Focused tests.** crates/hale-codegen/tests/release_reclaims_flow.rs; crates/hale-codegen/tests/release_two_parents.rs; crates/hale-codegen/tests/release_generic_owner.rs; tests/hale/release_generic_owner_test.hl; crates/hale-types/src/flows.rs (a_clause_names_the_locus_lowering_names, a_template_clause_names_the_specialization_s_argument); crates/hale-types/tests/demand_gate.rs (a_build_surveys_the_flows_once_and_lowering_reads_that_survey)

**Spec.** spec/semantics.md § release(c) and flow children

**Guarded seams.**

- `flows::survey(` may be referenced from: `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lifecycle/derive.rs` ×1

### `restart` — Canonical · derivation

**Answers.** Which loci declare restart operations, which restart in place, and what the restart bound is.

**Inputs.** closure and birth-check declarations; handler_routing (the rows' recovery ops)

**Producer.** `crates/hale-types/src/handler_routing.rs` · `handler_rows`

**Also owned.** `crates/hale-types/src/handler_routing.rs` · `recovery_ops`

**Consumers.** codegen (which loci get restart points: __restart_<L>, __resume_<L>) (`crates/hale-codegen/src/locus/restart.rs` · `can_fail`); codegen (the `for N` retry bound) (`crates/hale-codegen/src/codegen.rs` · `retry_bound_at`); codegen (__restart_<L>, __resume_<L>); model

**Invariants.**

- a recovery op is a row with a witness, per (parent, child)
- every `for` bound a recovery statement writes is an entry of the rows, keyed by the statement's span (`HandlerRouting::retry_bound_at`): a literal is its value, any other expression the site of the expression written there, which lowering lowers once where the statement runs. The model's `retry_bound` is the last literal of a handler's entries, so the bound modelled and the bound lowered are one fact
- which loci a failure can originate in (a closure of any epoch, `inline` ones and so every `violate` included, or a `birth_check`) is a column of the rows per locus declaration (`HandlerRouting::can_fail`), over the merged program lowering walks; lowering emits restart points exactly where it answers yes. A monomorph is no declaration and is not in the column, so a generic locus that declares a closure gets no restart points (known open)

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `restart_a_bound_without_its_row_is_refused_at_the_statement`; total: no row in the failure column means no failure can originate in the locus, so it gets no restart points (`can_fail`; a monomorph is no declaration and is in no column, known open), and no handler restarting it in place means none does

**Focused tests.** crates/hale-codegen/tests/restart_in_place_params.rs; crates/hale-codegen/tests/restart_bound.rs; crates/hale-types/tests/handler_routing_probes.rs (the failure column, the bounds); crates/hale-codegen/tests/restart_spine_ir.rs

**Spec.** spec/semantics.md § supervision

**Guarded seams.**

- `recovery_ops(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×2
- `can_fail(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-codegen/src/locus/restart.rs` ×1
- `retry_bound_at(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-codegen/src/codegen.rs` ×1

### `closures` — Migrating · law

**Answers.** Whether each closure clause is well formed, and which lifecycle events (`epoch`, `persists_through`, `resets_on`) it names.

**Inputs.** closure declarations; lifecycle_order (the event alphabet)

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_locus_member`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `emit_accumulator_reset_for_event` — the recovery events a closure names are matched ad hoc: `persists_through(...)` takes any identifier (`parse_recovery_event_name`), `locus/decl.rs` copies the names as strings, and this function compares them with the event being lowered, which is only ever `restart`, `restart_in_place` or `quarantine`, so a clause naming an event the locus never reaches is a silent no-op; `resets_on(...)` is read by nothing. The epoch is not ad hoc: its names are a closed enum (`EpochSpec`) the parser enforces. *Removed when:* the clause joins the lifecycle table and an unreachable event is a law violation with a witness.

**Consumers.** check; codegen

**Invariants.**

- closures are a consumer of the layer-6 alphabet (RFC §2)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/closure_resets_per_epoch.rs; crates/hale-types/tests/violate.rs

**Spec.** spec/semantics.md § closures

### `api_surface` — Canonical · derivation

**Answers.** The served surface: commands, reads, streams, their schemas, the roles that gate them, and the description's wire form; and the role rows: every `role` declaration, every `@gated` site and the api entry's role source, with or without an `api:` entry.

**Inputs.** @export, @gated, serve declarations; role declarations; the api binding; roles (--env)

**Producer.** `crates/hale-syntax/src/api_gen.rs` · `api_surface`

**Also owned.** `crates/hale-types/src/roles.rs` · `role_rows`; `crates/hale-types/src/roles.rs` · `role_laws`; `crates/hale-frontend/src/snapshot.rs` · `demand_role_rows`

**Consumers.** check --matrix (the roles an environment maps: the pair's snapshot's role rows, `RoleRows::declared_roles`, read once the pair's check has demanded them, not gated on the typing; a pair the check refuses has no snapshot and is reported by its refusal alone, with no role coverage) (`crates/hale-cli/src/verbs/check/matrix.rs` · `demand_role_rows`); check --dump-api (the snapshot's surface, the one its binding serves) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `api_surface`); check (the api entry's rules over the snapshot's surface, `CheckInputs::api_surface`, never a second derivation) (`crates/hale-types/src/check.rs` · `check_api_binding`); check (the ten role rules, a law over the role rows, `CheckInputs::roles`, joined to the bus graph's sites, the binding rows' bound topics and the topic rows' wire keys) (`crates/hale-types/src/check.rs` · `role_laws`); build (the description the binding serves); describe / call / watch / admin; ui (reserved); bundle (reserved)

**Invariants.**

- form, not params (I1): the description names what the program is, never where one copy listens
- perspective-invariant (I5)
- the role rows have one producer (F.40 phase 4, A4: `roles::role_rows`), over the programs after the sequence, with or without an `api:` entry, demanded once per snapshot (`Snapshot::demand_role_rows`, the `api_surface` count), not gated on the typing; a bundle no snapshot holds builds its own. Their columns: each `role` declaration (`RoleDeclRow`: the name and its span, each `includes` with its span, the declaration's span and site), two declarations of one name kept as two rows; each `@gated` site (`GateRow`: its kind, a free fn, a locus fn, a perspective fn, a contract member with its direction or a `publish` member by its identity, the declaration it sits on, the locus with its ordinal among the bundle's locus declarations, the bus graph's `decls` index, and whether it is imported, the member, the role and its span, the site's span); the role source (`RoleSource`: the last `roles:` clause, the entry it is named in, what it names and that locus's `fn holds` as written); and whether the row's root is served
- the role rules are a law over the rows (`roles::role_laws`), one function per rule, reading what another family owns from its owner: which handler of which declaration subscribes to which topic and which sites publish it from the bus graph, the bound topics from the binding rows, a topic's wire key from the topic rows; the role source's `fn holds` is read as written, since the rule is stated over the written signature and the scope holds resolved types
- the roles an environment maps are a projection of the rows (`RoleRows::declared_roles`, `vocabulary`); the binding is generated inside the sequence, before the rows exist, so the surface computes its list with the one function in `hale-syntax` (`api_gen::role_vocabulary`, which `declared_roles` reads too), held equal to the rows' projection over the corpus, `tests/hale` and the DNA seeds

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/api_description.rs; crates/hale-types/tests/api_binding_check.rs; crates/hale-types/tests/role_rows.rs

**Spec.** spec/model.md § The description; spec/semantics.md § The api binding (GH #1106); spec/types.md § Roles and `@gated` (GH #1109)

### `sealability` — Migrating · law

**Answers.** Which loci confine their state (`@sealed`), and which could.

**Inputs.** locus declarations; field accesses

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_sealed_access`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/sealability.rs` · `survey` — the `--sealable` survey seals every locus, re-runs a partial check and PARSES THE DIAGNOSTIC MESSAGE TEXT to decide. *Removed when:* the survey reads the sealed-access rows.

**Consumers.** check; check --sealable; claims (require sealed)

**Invariants.**

- a diagnostic's wording is never an input to a derivation

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/sealed_locus.rs

**Spec.** spec/verification.md § Secrets — confine, classify, claim

### `runs_under` — Reserved · derivation

**Answers.** On whose authority a locus runs: the relation `runs_under(locus, principal)`, with principals declared by the program.

**Inputs.** principal declarations (a later dialect); ownership (an undeclared locus inherits its owner's principal)

**Producer.** none: reserved, computes nothing.

**Consumers.** claims (`only reaches(effects(secret(..))) via { runs_under(P) }`); placement (policy projections per principal); dna (positions instantiate principals)

**Invariants.**

- static, like ownership; a locus cannot run under a principal its owner does not hold
- the column is reserved so the tower's table never changes twice

**Missing data.** n/a

**Spec.** RFC #1212, the authority comment

### `transitions` — Reserved · derivation

**Answers.** For an evented locus: the transition each handler is, input event to output set (F.41, after phase 2).

**Inputs.** @evented declarations; handler signatures

**Producer.** none: reserved, computes nothing.

**Consumers.** bus_graph (exact cycles); effects (rate bounds); lifecycle_order (finite products); replay

**Invariants.**

- at most one output per delivery, as a type; fan-out as self-events

**Missing data.** n/a

**Spec.** RFC #1212 §6 (F.41 sketch)

## Layer 4 — effects

### `effects` — Canonical · derivation

**Answers.** Which effect classes each fn and locus reaches (the callgraph fixpoint), the declared classes and their `causes:`/`depends:` DAG, and the certificate relating the two.

**Inputs.** effect annotations; the callgraph; stdlib_surface (leaf effects); effect_class_table; ffi names

**Producer.** `crates/hale-types/src/effect_rows.rs` · `derive_effect_rows`

**Also owned.** `crates/hale-types/src/frontier.rs` · `infer_effects`; `crates/hale-types/src/frontier.rs` · `infer_effect_bounds`; `crates/hale-types/src/purity.rs` · `infer_purity_for_bundle`; `crates/hale-types/src/effect_rows.rs` · `EffectRows`; `crates/hale-types/src/evidence.rs` · `derive_certificate_evidence`; `crates/hale-types/src/evidence.rs` · `derive_certificate_evidence_over`

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_decorator_stacks`); claims (certificate, causes, depends, budget); the certificate evidence (the check's effects certificate report, read by the check's laws and the artifact's) (`crates/hale-frontend/src/snapshot.rs` · `demand_effect_certificates`); check (a codec binding's purity assertion reads the purity column, demanding the rows only when a codec reaches it) (`crates/hale-types/src/check.rs` · `CheckInputs`); check (the blocking warning: which helpers hold a cooperative worker, the BLOCK class with its leaves and resolved targets, demanding the rows only once a placed field has a `run()` to walk) (`crates/hale-types/src/check.rs` · `worker_holding_fns`); model (effect labels, lower bounds and direct contributions, the last read by the reachability judgment's `effects(C)` test, and the summary the rows' walk read: the snapshot's rows, handed in) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); the effects manifest (`--dump-effects-manifest`, `--check-effects-manifest`: the snapshot's rows, cross-seed calls resolved through the renames) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_effects`); replay (the live-effects gate reads the manifest over the snapshot's rows, and refuses when they are blocked) (`crates/hale-cli/src/verbs/replay.rs` · `demand_effects`); doc

**Invariants.**

- derived ⊆ declared is the certificate; budgets are judged through evidence (spec/verification.md)
- effects run once per snapshot, not once per consumer: `Snapshot::demand_effects` runs `derive_effect_rows` once over the snapshot's allocation summary (`demand_alloc_summary`: the checked programs and the stdlib's analysis copy, cross-seed calls resolved through the import renames), counted as `effects`, blocked with the scope
- an unresolved edge is coverage, never a violation: a row's `effects` saturates to `UNCLASSIFIED` when the walk reaches what it cannot name, its `known` set is the lower bound an unresolved edge never erases, and `unknown` says the walk reached such an edge
- an indirect call is one rule for every certificate and budget reader: a call through a function value the summary resolves (the `alloc_summary` family's function-value rule) is its alternatives, each judged as the call of that fn, and one it does not resolve (`CallEdge::indirect`: a function-typed parameter, an unfollowed local or a computed callee no function value of the program can be) is a call that may do anything, never a call to nothing: the effects engine and the rows read it as `UNCLASSIFIED`, `@budget` and the quantitative dimensions as unbounded, the model as an `IndirectCall` hole, so every certificate over it is refused or uncertified (a classified correction, F.40 E5: a call through an unresolved local was a call to nothing, pinned in `indirect_calls.rs`)
- a row is keyed by its declaration's identity (`FnKey::decl`, the `snapshot_identity` family's site), its (locus, fn) name kept as the display
- a fn's direct contribution is a column (`direct`; `EffectRows::direct` answers any key, a bodyless one by what it carries): the model's function rows and absorbed paths, which the reachability judgment's `effects(C)` destination test reads, take it from the rows, and nothing outside the producer folds a body for it
- purity and the lower bound are columns of the rows: one walk answers a fn's saturating set and its lower bound (`infer_effect_bounds`), and the purity walk runs only inside the producer; the checker's codec law reads the purity column through `CheckInputs::effects`, a demand made only when a codec binding reaches the assertion, and the blocking check its BLOCK class, a demand made only once a placement entry puts a field with a `run()` on a classic pool or main, so a check of a program that does neither runs no effects fixpoint
- the effects certificate engine runs once per snapshot, in the check (`check_bundle_reporting`); the certificate evidence reads that report (`Snapshot::demand_effect_certificates`, handed to `derive_certificate_evidence_over`) and never runs the engine itself; a bundle no check ran over runs it once for itself (`effect_certificates`)

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/effect_assertions.rs; crates/hale-types/tests/effects_cross_seed_correction.rs; crates/hale-types/tests/indirect_calls.rs; crates/hale-cli/tests/effects_baseline_gate.rs; crates/hale-cli/tests/effects_manifest.rs

**Spec.** spec/verification.md § Default-on & opt-in analyses; spec/verification.md § Claims

**Guarded seams.**

- `infer_effects(` may be referenced from: `crates/hale-types/src/frontier.rs` ×4, `crates/hale-types/src/claims.rs` ×1, `crates/hale-types/src/topology.rs` ×1
- `effect_manifest_with_inference(` may be referenced from: `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-cli/src/verbs/replay.rs` ×1
- `infer_purity_for_bundle(` may be referenced from: `crates/hale-types/src/purity.rs` ×2, `crates/hale-types/src/effect_rows.rs` ×1
- `infer_effect_bounds(` may be referenced from: `crates/hale-types/src/frontier.rs` ×2, `crates/hale-types/src/effect_rows.rs` ×1
- `direct_effects(` may be referenced from: `crates/hale-types/src/effect_rows.rs` ×2
- `effect_report_grouped(` may be referenced from: `crates/hale-types/src/effects.rs` ×3, `crates/hale-types/src/check.rs` ×1
- `derive_certificate_evidence(` may be referenced from: `crates/hale-types/src/evidence.rs` ×1
- `derive_certificate_evidence_over(` may be referenced from: `crates/hale-types/src/evidence.rs` ×2, `crates/hale-types/src/judgment.rs` ×1, `crates/hale-types/src/topology.rs` ×1

### `blocking` — Canonical · derivation

**Answers.** Which fns block (a cooperative worker would be held), and whether the program places anything off the main thread.

**Inputs.** stdlib_surface (holds_cooperative_worker); effects (the rows' BLOCK class, their resolved targets and unresolved leaves); placement (the table's domains); bindings (the program's own main's entries)

**Producer.** `crates/hale-types/src/check.rs` · `blocking_path_match`

**Also owned.** `crates/hale-types/src/check.rs` · `worker_holding_fns`; `crates/hale-types/src/placement.rs` · `places_off_main`; `crates/hale-types/src/binding_rows.rs` · `binds_on_main`

**Consumers.** check (rules 7, 8); effects (@no_block); codegen (mark_pinned, no_pinned dispatch: `program_has_offthread` over the view's placement table and binding rows) (`crates/hale-codegen/src/codegen.rs` · `program_has_offthread`)

**Invariants.**

- one leaf set (GH #830) and one propagation
- whether a thread crosses the bus boundary is read off rows, once, in lowering (`program_has_offthread`): a domain of the placement table that is not main (`PlacementTable::places_off_main`: a pinned anchor, a non-main pool, an adapter binding's thread, the api binding's synthesized pool) or an entry of the entry's `bindings { }` (`BindingRows::binds_on_main`: the rows of the locus the entry row names, by its site, as lowering's prelude lowers them); the `lotus_bus_mark_pinned` call and every static dispatch's `no_pinned` flag are that one value and its negation
- an imported library's `main locus` is never deployed, so its `placement { }` block places nothing and its `bindings { }` bind nothing: a program importing one is not off-thread for it (a classified correction, pinned in `offthread_imported_main.rs`); the entry's binding makes its program off-thread, and a module-nested `main`, which the old binding term (top-level items only) missed, is no longer deployed: as a seed's only `main locus` it is refused (L4)
- the helpers that block are the effect rows' (`worker_holding_fns`, demanded through `CheckInputs::effects` only once a placed field has a `run()` to walk): a fn whose `direct` BLOCK comes from a leaf `holds_cooperative_worker` names (the BLOCK class includes `std::time::sleep`, which yields the worker, so the leaf test stays beside the row), closed over the rows' resolved targets; the check folds no call graph of its own
- the rule's horizon: the rows' propagation sees what the old name-keyed walk did not (a qualified cross-seed call, a stdlib body behind a handle method, another locus's method), but the `run()` walk consults the set only at a bare call or a `self.m()` call, so the horizon decides what a helper reaches and never which call in `run()` is looked at; the rule's diagnostics over the corpus, tests/hale and the DNA seeds are the old walk's (spec/verification.md § Concurrency & placement safety)

**Missing data.** total: no domain off main and no binding row of the entry means no thread crosses the bus boundary (`program_has_offthread`)

**Focused tests.** crates/hale-types/tests/placement.rs; crates/hale-codegen/tests/bus_devirt_no_pinned.rs; crates/hale-types/tests/checks_inside_modules.rs; crates/hale-cli/tests/offthread_imported_main.rs

**Spec.** spec/semantics.md rules 7, 8; spec/verification.md § Concurrency & placement safety; spec/semantics.md § The entry locus

**Guarded seams.**

- `places_off_main(` may be referenced from: `crates/hale-types/src/placement.rs` ×1, `crates/hale-codegen/src/codegen.rs` ×1
- `binds_on_main(` may be referenced from: `crates/hale-types/src/binding_rows.rs` ×1, `crates/hale-codegen/src/codegen.rs` ×1

### `alloc_summary` — Canonical · derivation

**Answers.** Where each allocation lands and when it is reclaimed: per-fn allocation, escape, scratch eligibility, method-scratch elision, stack arrays, arena elision.

**Inputs.** bodies; signatures (non_allocating, fallible, ffi); the callgraph; ownership (accept sets)

**Producer.** `crates/hale-types/src/alloc_summary.rs` · `derive_alloc_summary`

**Also owned.** `crates/hale-types/src/alloc_summary.rs` · `summarize_identified`; `crates/hale-types/src/alloc_routing.rs` · `derive_alloc_routing`; `crates/hale-types/src/alloc_routing.rs` · `scratch_local_free_fns`

**Consumers.** the effects certificate engine (the check's `@effects`, `@phase_effects` and placement diagnostics: the snapshot's summary, handed in) (`crates/hale-types/src/check.rs` · `CheckInputs`); effects (the rows walk the snapshot's summary and hold it, shared) (`crates/hale-frontend/src/snapshot.rs` · `demand_effects`); check (the unbounded-allocation advisory and `--dump-alloc-summary`: the snapshot's summary) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_alloc_summary`); the editor's typing stage (the advisory in the diagnostics, `Config::alloc_advisory`: the snapshot's summary) (`crates/hale-frontend/src/snapshot.rs` · `typing_stage`); lsp (hale/allocSummary: the snapshot's summary) (`crates/hale-lsp/src/lib.rs` · `demand_alloc_summary`); model (its function rows, dispatch sites and holes: the snapshot's summary's own rows) (`crates/hale-types/src/model_builder.rs` · `own_rows`); topology (the artifact's fn sort, labels, derived effects and through-stdlib contraction: the snapshot's summary, handed in) (`crates/hale-types/src/topology.rs` · `dump_topology_over`); check (the hot-path lint: a law over the program's own rows and their columns, the snapshot's summary, handed in) (`crates/hale-types/src/check.rs` · `check_hot_path_alloc`); claims (@budget: the counting engines over the snapshot's summary's own rows, handed to the evidence) (`crates/hale-types/src/evidence.rs` · `derive_certificate_evidence_over`); codegen (arena routing at an allocation) (`crates/hale-codegen/src/codegen.rs` · `current_arena_ptr`); lowering view (the routing rows' scratch-local set: the snapshot's summary's, handed in) (`crates/hale-frontend/src/snapshot.rs` · `summary.scratch_local`); codegen (a free fn's scratch: the view's non-allocating and scratch-local rows) (`crates/hale-codegen/src/codegen.rs` · `alloc_routing`); codegen (a locus's arena, and its hooks', methods' and modes' scratch: the elision rows, a monomorph's specialized) (`crates/hale-codegen/src/codegen.rs` · `locus_elision`); resource_budget (the fd sites and the fd-leak warnings: the snapshot's summary's own rows) (`crates/hale-types/src/resource_budget.rs` · `own_rows`); frontier (the `causes:` engine: the summary's own rows) (`crates/hale-types/src/frontier.rs` · `causes_inner`); a declaration's dependents (X2: the call edges of the rows and of the declaration bodies) (`crates/hale-frontend/src/dependents.rs` · `declaration_bodies`)

**Invariants.**

- one summary per snapshot: `Snapshot::demand_alloc_summary` runs `derive_alloc_summary` once over the checked programs with the stdlib's analysis copy beside them (cross-seed calls resolved through the import renames), counted as `alloc_summary`, blocked with the scope; the check's effects certificate engine reads it (`CheckInputs::alloc_summary`) and the effect rows walk it, so neither builds its own; `summarize_identified` is the one constructor, and `derive_alloc_summary` (which places the stdlib's analysis copy beside the programs itself) its one caller outside tests, so no reader builds a summary variant of its own
- the checker's reclaim model and codegen's routing agree; a stale copy is a registry violation, not a comment: `summarize_identified` classifies each free fn's frame (`FnSummary::frame`: `ScratchLocal { recursive }` or `CallersArena`; `None` for a method, a hook, a mode, `main` and an unclassified row) with the classifier lowering reads (`alloc_routing::scratch_local_free_fns`), over the declarations lowering lowers (the programs, their imported seeds under their mangled names, and the stdlib, beside the program whether or not the summary holds the analysis copy's rows) with the same renames, so the frame is lowering's row on every target (pinned in `alloc_summary_reclaim_boundary.rs`)
- that classifier runs once on a build path (F.40 phase 4, Q1): in the summary, which keeps the set it classified (`AllocSummary::scratch_local`, a set of fn names); `demand_lowering` hands the snapshot's summary's set to the view, and lowering's rows (`derive_alloc_routing`) take it rather than classifying the merged program again (pinned by a thread-local count, `scratch_local_derivations_on_this_thread`, in `demand_gate.rs`). The two runs it replaced read the same declarations but one: a program that declares a top-level item named like one of the stdlib's free fns (all `__`-prefixed) has the stdlib's twin left out of the summary, where the merged program held both
- the reclaim boundary is relative to the loop analyzed (E3b, a classified correction, pinned per target in `alloc_summary_reclaim_boundary.rs`): a `Local` allocation of a scratch-local fn that is not recursive reclaims at its return (`ReclaimScope::FnReturn`), which `ReclaimScope::accumulates_in_loop` places inside each iteration of a caller's loop and outside every iteration of the fn's own (`LoopAt`); such a fn stops the scratchless long-lived propagation as a method's per-call scratch does; `Local` is not scratch, so a `Local` of a `CallersArena` fn run once per iteration of an unbounded loop in a long-lived frame lands in that frame's arena and accumulates (`LeakReason::InCallersArena`), a locus instantiation excepted; an escaping value keeps its boundary, and a recursive fn keeps the locus boundary; the model, the budgets and `shape_hash` read none of it
- lowering routes from the view's rows (`LoweringView::alloc_routing`, `derive_alloc_routing` over `merged` with the view's renames and the summary's scratch-local set) and derives none: a free fn skips its per-call scratch when the rows call it non-allocating (FORM-3, a greatest fixpoint keyed by name), and its body allocates into its own subregion when they call it scratch-local (#1148); a locus's arena is elided, and a lifecycle hook, `fn` method or mode lowers without its per-call scratch, when its elision row says so (`LocusElision`, keyed by the locus and the member's position; a generic locus's monomorph, which lowering synthesizes, takes `AllocRouting::specialize` over the synthesized declaration); the FORM-3 classifier (`fn_body_definitely_non_allocating`) is the rows' own and lowering calls it nowhere; the two method-elision stages still classify a self field differently (stage 1, the per-member verdict, by its literal default or its ascription through aliases; stage 2, the `self.m()` sets, by a primitive ascription only), and a monomorph still has no stage-2 set, both as they were; a free fn's entry publishes its caller's arena to the caller-arena TLS when its row says so (`caller_arena_publish`: not non-allocating, and a call or a struct literal anywhere in its body, found by a structural walk, a string literal spelling `Call {` or `Struct {` counting as the Debug-string test it replaced counted it; a generic fn's monomorph takes `AllocRouting::specialize_fn`), and no body's Debug rendering decides it; the rows are #1208's classification moved as it was, so its builtin list is still a hand-kept subset of the checker's `BARE_BUILTIN_CALLEES` with no agreement test, its `std::` namespaces a per-namespace claim no stdlib_surface row states, and its qualified-path lookup checks the import renames before `PATH_RENAMES` where `resolved::lookup_qualified_path` checks them in the other order
- a body's escape tags key a binding by the declaration its escaping uses name (`Snapshot::declaration_of`), so an inner shadow of a returned name is its own, local binding (the #1140 shape), and close over `let x = y;` aliases to the declaration y names, as borrow_lifetime's `returned_decls` does; programs minted by different snapshots are summarized each with its own identities (`summarize_identified`: a bundle's programs beside the bundled stdlib's analysis copy)
- each seed's names resolve in its own scope: the programs minted with one set of identities are one scope, a body's bare free-fn name (and the locus a call's result is typed by) resolves only to a fn of its own scope, and the import renames are the bundle's names (the stdlib's analysis copy imports nothing), so a stdlib body's builtin `count(...)` is the builtin, never a user fn called `count` (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)
- the unbounded-invocation fixpoint (`AllocSummary::unbounded_invoked`) seeds from what the program reaches: beside the stdlib's analysis copy, `AllocSummary::reached` is the program's own fns, what their calls reach (the interface fan-out included) and the hooks and bus handlers of every locus they start (a struct literal of it in a reached body, or a param field of a started locus by declared type or default literal; every locus of the program's own is started), and only those seed it or call, so a stdlib loop the program never starts invokes nothing (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)
- the run-to-exit rule (a `main` and no long-lived entry: no leak sites) reads the program's own entries, never the stdlib's analysis copy's (`AllocSummary::analysis_copy` names the copy's fns, `is_own` the program's), since the copy always carries `run` hooks (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)
- a program's declaration is the row where the stdlib's analysis copy declares the same name: the copy's top-level free fn, locus or interface of a name a checked program declares stays out of the summary, so a stdlib source file checked as itself keeps its own rows (its bodies, its spans, resolved in its own scope); only stdlib source shares such a name (a classified correction, pinned in `alloc_summary_construction_correction.rs`)
- the rows carry what the hot-path lint reads: a fn's `@hot` and whether it is a mode (`FnSummary::hot`, `mode`), the order of the declarations (`decl_index`), where a site or a call is written (`in_loop`, which a `return` / `fail` payload does not reset as it resets `loop_depth`), a struct literal written as a whole statement (`AllocSite::bare_stmt`) or as the whole right side of a `self.<field> =` replace (`self_replace`, the statement's span; an in-place replace, which allocates nothing, is in `FnSummary::in_place_sites`, not `sites`), the `let` a call is the value of (`CallEdge::let_span`), how a call is spelled (`CallSpelling`), and the allocating receives (`CallEdge::allocating_recv`, the one list, which `@budget` reads too)
- the hot-path lint is a law over the rows (`check_hot_path_alloc`, over the summary the check is handed): the program's own fns, a mode aside, in declaration order, each finding where its site or call is written, a bus handler's at any depth, `@hot` an error and `@unbounded` silencing the advisory; no reader walks bodies for it, so it sees what the summary's walk sees (a publish, a bare block, `violate`, a recovery and `shm_write` included, which the lint's own walk skipped; an index expression's subscript included, walked as any operand is, as the lint's walk walked it, so a `@hot` fn keeps that rejection (a classified correction, pinned in `hot_path_alloc.rs`); a callee that is an expression of its own excluded, which it reached) and its diagnostics over the corpus, the targets and the lint's pins are the lint's, in order (pinned in `hot_path_alloc.rs`)
- a module-nested declaration is summarized like a top-level one: every declaration pass walks `module { … }` nesting (`flat_decls`; a module is a namespace, not an analysis boundary, GH #764), so a module-nested fn or locus member has a row and a call into it resolves, and the model holes out no module-nested body; the one body with no row is an `on_failure` handler, summarized as a declaration body (a classified correction, pinned in `alloc_summary_construction_correction.rs`)
- the advisory, `--dump-alloc-summary` and the editor's hale/allocSummary read the snapshot's summary and report the program's own rows judged over the whole of it (the stdlib's analysis copy and the renames included: a classified correction, pinned per target in `alloc_summary_correction.rs`); a leak site is the program's own (`AllocSummary::is_own`) and is left out of the advisory only when it has no author position (`AuthorPositions::has`: its span at or beyond `API_SYNTH_BASE`, or in a declaration the origin rows mark synthesized whose offset no source file owns), never by its owner's name; the check's warnings, the editor's diagnostics and the editor's hale/allocSummary decide with one function (`advisory_leak_sites`)
- a call through a function value resolves to the program's function values (F.40 E5): the summary marks a call through a function-typed parameter, a local its walk does not follow to a fn, or a computed callee `CallEdge::indirect`, and `resolve_function_values` rewrites it into one alternative per fn some expression of the bundle reads as a value (`fn_values`: every expression the identity walk reaches, a callee written as a name or a path and a name a binding in scope spells excluded; resolved as a `let` of it resolves) whose arity is the call's and whose declared signature can be the parameter's or the bound field's declared type, sharing a dispatch group (`CallEdge::via_value` the callee as written); a call no such value can be stays indirect, and so does every call of a bundle that reads a locus's method as a value, which this does not follow
- the model projects the program's own rows; a call into the analysis copy is the unresolved row: the model, the `@budget` engines (`budget_check`, `quantitative`), the artifact's user rows, the frontier's `causes:` engine and the resource budget (its fd sites and fd-leak warnings) read the snapshot's summary through `AllocSummary::own_rows` (the program's fns and loci; a call resolved into the stdlib's analysis copy is the unresolved call by the method's bare name, the receiver kept; a dispatch keeps its alternatives among the program's own loci, renumbered in key order, through the copy's interface is no dispatch, and through the program's own with none of its conformers left is the dead site; a function-value dispatch keeps its alternatives among the program's own fns and the leaves the program itself reads as values (`AllocSummary::copy_alternative`), its groups numbered after the interface groups, and with none left is the indirect call as written), which is the program-alone summary field for field, so the copy moves no `shape_hash`, the build and replay identity, and the copy's fd-acquiring bodies are not the program's resources (pinned per target in `alloc_summary_own_rows.rs`); what the model would gain from the copy's rows is a separate correction
- a body the program writes that is no row is a declaration body (`AllocSummary::declaration_bodies`, `DeclarationBody`): an `on_failure` handler, a params block's initializers, a constant's value, a `birth_check`, a perspective's fns, params and `stable_when`, a synthesized hook, each wherever it is declared (a `module { }`'s included, through `flat_decls`, as the rows are), and every subexpression a walk does not descend into (a callee that is no name; an index's subscript is the row's own, walked), each walked as a row is and keyed to the site of the declaration it is a member of; no judgment reads them, so the rows and every consumer's answer are what they were without them (X2, a review fix: `hale check` with every dump over the corpus, `tests/hale` and every `dna/**/main.hl` unchanged), and the declaration dependents relation reads their call edges as the declaration's own

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `alloc_summary_a_declaration_without_its_routing_rows_is_refused`; total: no row in the routing sets means the conservative answer: a fn not proven non-allocating keeps its scratch, one not scratch-local allocates into its caller's arena, and a member with no scratch entry keeps its scratch; a locus's elision rows and a free fn's caller-arena row are required, a generic fn's monomorph taking the rows' producer over its synthesized declaration

**Focused tests.** crates/hale-types/tests/hot_path_alloc.rs; crates/hale-types/tests/alloc_summary_construction_correction.rs; crates/hale-types/tests/alloc_summary_correction.rs; crates/hale-types/tests/alloc_summary_own_rows.rs; crates/hale-types/tests/alloc_summary_reclaim_boundary.rs; crates/hale-types/tests/demand_gate.rs (a_build_classifies_the_scratch_local_fns_once); crates/hale-codegen/tests/alloc_model_rss.rs; crates/hale-codegen/tests/scratch_local_free_fn.rs; crates/hale-codegen/tests/fn_nonalloc_add.rs; crates/hale-codegen/tests/method_scratch_elision.rs

**Spec.** spec/memory.md § Allocation routing; spec/verification.md § Memory-bound proofs (item 1): the reclaim boundary; spec/styleguide.md

**Guarded seams.**

- `scratch_local_free_fns(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×1
- `summarize_identified(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×4
- `allocating_recv(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×2
- `check_hot_path_alloc(` may be referenced from: `crates/hale-types/src/check.rs` ×2
- `derive_alloc_summary(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/evidence.rs` ×1, `crates/hale-types/src/judgment.rs` ×1, `crates/hale-types/src/topology.rs` ×1, `crates/hale-types/src/resource_budget.rs` ×1
- `own_rows(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×1, `crates/hale-types/src/model_builder.rs` ×1, `crates/hale-types/src/budget_check.rs` ×1, `crates/hale-types/src/quantitative.rs` ×1, `crates/hale-types/src/frontier.rs` ×1, `crates/hale-types/src/resource_budget.rs` ×2
- `derive_alloc_routing(` may be referenced from: `crates/hale-types/src/alloc_routing.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `fn_body_definitely_non_allocating(` may be referenced from: `crates/hale-types/src/alloc_routing.rs` ×12
- `unbounded_alloc_warnings(` may be referenced from: `crates/hale-types/src/lib.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

### `borrow_lifetime` — Canonical · law

**Answers.** Whether a borrowed handle outlives its holder (GH #730), decided from position over the owner structure.

**Inputs.** ownership (Borrowed rows); bodies

**Producer.** `crates/hale-types/src/borrow_lifetime.rs` · `borrow_lifetime_diags`

**Consumers.** check (`crates/hale-cli/src/verbs/check/run_impl.rs` · `borrow_lifetime_diags`); build, run, test, replay, bench (a build config's snapshot check, `Config::build_rules`) (`crates/hale-types/src/lib.rs` · `build_rule_diags`)

**Invariants.**

- runs on every entry point: it does not run in the LSP or bench today (phase 2 closes that)
- which locus accepts which child is the ownership rows' relation (`accepts_ancestor`, the snapshot's graph's rows, handed in); it builds no accept set of its own

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/check_borrow_lifetime.rs

**Spec.** spec/semantics.md § A borrow outlives its holder; spec/decisions.md F.39

**Guarded seams.**

- `borrow_lifetime_diags` may be referenced from: `crates/hale-types/src/borrow_lifetime.rs` ×3, `crates/hale-types/src/lib.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1

### `bare_fallible` — Canonical · law

**Answers.** Whether a fallible call's error is addressed.

**Inputs.** typed_bodies (the fallible_calls column: each call's callee mark and what addresses it)

**Producer.** `crates/hale-types/src/bare_fallible.rs` · `bare_fallible_calls`

**Consumers.** check, verify, build, run, test, replay, bench, the LSP (the snapshot's check, with the typing diagnostics) (`crates/hale-frontend/src/snapshot.rs` · `bare_fallible_calls`); the check of a bundle no snapshot holds (the tests' entries) (`crates/hale-types/src/check.rs` · `bare_fallible_calls`)

**Invariants.**

- one rule for user and stdlib calls: only an `or` handles a fallible call; any other position (an argument, an operand, a `match` scrutinee, a `let` initializer, a statement, a returned value) is the GH #738 error, so `hale check` refuses what `hale build` refuses, save a call through an interface-typed local or parameter: the checker types that receiver `Unknown`, so the column holds no row for the call and only the build refuses it (a typing limitation to lift)
- the law reads the column, never the signature table or the syntax tree: the checker records each fallible call's callee mark (`Declared`, `Typed`, `Stdlib`) and what addresses it as it walks
- an `or`'s handler takes the implicit `or raise` exactly where lowering does (`expr_is_fallible_call`): a `Declared` callee; any other fallible handler is refused with the nested spelling, a limitation to lift, not a rule
- the checker types a fallible value no `or` addresses as its success type, so a bare call reports the law's one error and no type mismatch
- runs on every entry point, the LSP and bench included (the snapshot's check)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/check_strict_fallible.rs; crates/hale-types/tests/typed_bodies.rs

**Spec.** spec/semantics.md § A bare fallible call is an error; spec/types.md

**Guarded seams.**

- `bare_fallible_calls(` may be referenced from: `crates/hale-types/src/bare_fallible.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1

### `nonreturning` — Canonical · law

**Answers.** Which `run()` bodies never return, which children are long-running, and whether the birth order or a pool starves because of it.

**Inputs.** flows (each locus declaration's run row: the long-running and never-returns columns); params order; placement (the table's rows for the deployed root's fields)

**Producer.** `crates/hale-types/src/flows.rs` · `run_statically_nonreturning`

**Also owned.** `crates/hale-types/src/check.rs` · `check_pool_starvation`; `crates/hale-types/src/check.rs` · `check_birth_order`

**Consumers.** check (the nested-long-running-child rule reads the long-running column) (`crates/hale-types/src/check.rs` · `check_nested_long_running_child`); check (the starvation and birth-order laws read the never-returns column) (`crates/hale-types/src/check.rs` · `never_returns`)

**Invariants.**

- two definitions, two columns of the flow rows' run row (`flows::RunRow`), named in spec/runtime.md § Typecheck enforcement: long-running is a `run()` body with a statement of its own (a nested child's `run()` completes before its parent's begins, so any body delays the parent, whether or not it returns), never-returns is a terminal `while` with no exit whose condition never flips false (only such a body starves the cells a pool runs after it); every body that never returns is long-running, not the converse, and a child whose `run()` is `std::time::sleep(1m)` keeps the long-running-child error
- the checker surveys the flow rows once per check and the three rules read the columns; none decides either question itself; a stdlib locus, whose body the checker does not see, is both when it is on the known-long-running allowlist (`KNOWN_LONG_RUNNING_STDLIB_LOCI`)

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/birth_order_trap.rs; crates/hale-codegen/tests/birth_order_trap.rs; crates/hale-codegen/tests/nested_long_running_child.rs; crates/hale-types/src/flows.rs (long_running_and_never_returns_are_two_columns)

**Spec.** spec/semantics.md; spec/runtime.md § Typecheck enforcement

**Guarded seams.**

- `run_statically_nonreturning(` may be referenced from: `crates/hale-types/src/flows.rs` ×2

### `working_set` — Canonical · derivation

**Answers.** The estimated working set per locus and program, and the locality law over it.

**Inputs.** declared shapes; @locality

**Producer.** `crates/hale-types/src/working_set.rs` · `compute_program_working_set`

**Also owned.** `crates/hale-types/src/working_set.rs` · `compute_locus_working_set`; `crates/hale-types/src/working_set.rs` · `compute_program_returns_entry_per_locus`

**Consumers.** build (`crates/hale-cli/src/verbs/build.rs` · `compute_program_working_set`)

**Invariants.**

- build-only today, and a warning without --strict; layer 7 (layout) reads it later

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-cli/tests/target_model.rs

**Spec.** spec/memory.md

## Layer 5 — placement

### `placement` — Canonical · derivation

**Answers.** Which thread domain each instance runs in: pools, pinned threads, replicas, affinity, and the deployment plan.

**Inputs.** placement and topology blocks; entrypoint (the row's root: the entry, or a seed's refused module-nested `main`); the construction templates: the root's literals, the entry's implicit construction of a root no literal builds, `fn main`'s own literals, the root's `bindings { }` adapters; the params towers each template builds; the minted sites of the snapshot and of the stdlib analysis copy; free fns and locus bodies (dynamic sites, their domains and bounds)

**Producer.** `crates/hale-types/src/placement.rs` · `derive_placement`

**Consumers.** check (rule 6, the lowering laws) (`crates/hale-types/src/lowering_laws.rs` · `pinned_features`); check (instance aliasing, #334: where each of the deployed root's params fields runs, the table's root and the domains of its rows) (`crates/hale-types/src/check.rs` · `check_instance_aliasing`); check (rule 17, the lowering laws: the root's constructions and their bounds) (`crates/hale-types/src/lowering_laws.rs` · `pinned_root_in_a_loop`); check (rule 18, the lowering laws: the root and its constructions) (`crates/hale-types/src/lowering_laws.rs` · `placement_entry_consumed`); check (rules 2-5, 13-16); check (F.31: the caller per instance, the receiver by its row's `owner_relative`) (`crates/hale-types/src/check.rs` · `check_placement_single_thread`); check (the blocking check and the starvation and birth-order laws: where each of the deployed root's fields runs, its pool's `async_io`, and the declarations it realizes) (`crates/hale-types/src/check.rs` · `root_field_placements`); check (type-check rule 20: the construction paths of a handler birth's enclosing locus, `OwnershipGraph::construction_paths` over the table handed in, derived only when that locus does not accept the child itself) (`crates/hale-types/src/check.rs` · `check_unowned_subscriber_locus`); sync_inference (accessor domains per instance) (`crates/hale-types/src/sync_inference.rs` · `infer_sync_for_bundle`); dispatch (domains: both plans' domain map, `Arrangement::domains`, over the arrangement projection) (`crates/hale-types/src/arrangement.rs` · `fn domains`); model (the arrangement: instances, owners, placed_in and affined_to, the table's rows projected through `project_arrangement`, user-only: the snapshot's projection, handed in) (`crates/hale-types/src/model_builder.rs` · `derive_application_model_over`); the intra-locus rewrite (a publish into a field off its owner's thread stays on the bus: `PlacementTable::off_owner_fields`) (`crates/hale-types/src/resolved.rs` · `rewrite_intra_locus`); codegen (the deployment plan, the table's lowering view: the root, and per root field an entry decides its schedule class, pool, NUMA node and replica cores; the pools' async_io and affinity; the pinned anchors' and pooled rows' realized declarations, an adapter among the anchors) (`crates/hale-codegen/src/codegen.rs` · `collect_main_placement`); resource budget (the threads, partitioned by the scope that creates them: the root's pinned anchors under their construction's bound, the adapters once; the worker pools, main never one) (`crates/hale-types/src/resource_budget.rs` · `budget_for_programs`); codegen (whether a thread crosses the bus boundary: a domain that is not main, `places_off_main`, over the lowering view's table, the snapshot's, handed in) (`crates/hale-codegen/src/codegen.rs` · `program_has_offthread`); codegen (the registration route: the pinned anchors whose tree holds a subscriber, by lowered name, each given a mailbox its descendants' subscriptions route to) (`crates/hale-types/src/resolved.rs` · `route_anchors`); bus_graph (every placement label and the direct-call gate: the set of each type's instances' domains) (`crates/hale-types/src/bus_graph.rs` · `type_placements`); check (a subscriber's `bounded(N, …)`, legal only where every instance runs on main: B-2, read only when a subscriber is bounded) (`crates/hale-types/src/check.rs` · `check_bounded_bus`); ownership (each bubbling edge's class: the enclosing instances paired with their owner rows) (`crates/hale-types/src/ownership_graph.rs` · `relate`); lsp (hale/placement); deployment (reserved)

**Invariants.**

- placement is keyed by instance, never by type: one row per static instance of each construction template (a key is its origin, its field path, its replica), and a type's answer is the set of its instances' domains
- the entry is a construction scope: a root no literal builds is the entry's implicit template (`Origin::Entry`, bound `Once`), and `fn main`'s own literals are templates bound by their statement's loop context; an adapter is an origin of its own, built once
- nested rows inherit their owner's domain unless a root field's entry or a binding decides it; pinned domains are per anchor and per replica, pool domains one per name with at most one affinity
- the root is the entry row's root (`EntryRow::root`): the entry, which lowering deploys, or in a seed with none the module-nested `main` the check refuses and still judges (`RootRow::is_entry` false); an imported `main` is never the root
- every root entry decides exactly one field family in each construction template, or it is a hole
- unknown is a hole, not a default: an unresolved declaration, an unenumerable initializer, a held instance (`Reuse`) and a dynamic site of unknown domain each carry their policy, and none is main
- a held instance's subtree lives in its holder's domain: the held row keeps its `Reuse` hole and its owner's domain, the source's actual rows (never the declaration's defaults) are projected under it, inherited, and each of those rows names its own source row, the one it was built as (`built_by`); where the source is not linked, nothing below the held row is asserted, and an instance there runs in an unknown domain; a question of where an instance runs skips the source's rows, a count of instances skips the held ones
- every site the table names carries the universe that minted it (`SiteRef`); lowering joins the stdlib's into its merged mint once, totally and injectively
- the checker builds no table: the snapshot demands it and hands it to the check (`CheckInputs::placement`) and to the form rows; a bundle no snapshot holds (the test entries `check_bundle`, `check_bundle_opts_scoped`, `derive_application_model`, `effect_certificates`) builds it once, over a minted bundle (an unminted one names no site and gets an empty table, which judges nothing), except that the model's test entry (`derive_application_model`, and the check's claims through it) mints a copy of an unminted bundle first, since the model's arrangement is the table's rows
- the model's arrangement is the table's rows projected (`placed_in.push(` and `affined_to.push(` have one writer): the deployed root's templates, each instance where it runs, user declarations only; the table, not the arrangement, answers every placement question. The projection is one function, `project_arrangement` (F.40 phase 3, C5), run once per snapshot over its programs, table and ownership graph (`Snapshot::demand_arrangement`, the `arrangement` count, F.40 phase 4, Q1): the model builder makes its rows and placement holes from that one (`ModelInputs::arrangement`), and lowering reads the dispatch plans' domains from it. It names each declaration by name, which is all either reads, so a snapshot holds it; a bundle no snapshot holds (the test entries `derive_application_model`, `resolve_program`) projects its own
- a consumer asks where instances run of `PlacementTable::running` (the handed-off rows skipped): a type's answer is the domains of its instances, compared per instance, never collapsed to one per type; an enclosing locus with no static instance is a hole that disables the F.31 proof, never a default to main or to pinned
- a count over the table is over templates: a domain belongs to a template, as the key that anchors it does, and has one anchor per live occurrence; a count sums the templates, each times its bound (`Unbounded` makes the count an uncertainty with its reason), takes the maximum over the alternatives of one step (one occurrence takes one), counts each replica row once and never multiplies it by K again, and counts an adapter once, never under a root construction's bound (`PlacementTable::templates`, `per_occurrence`)
- the resource budget counts the resource, read from the table: OS threads are the pinned anchors (one per replica) times their construction's bound, plus one per adapter; a pool is one worker however many instances it holds, an affinity is a column of its domain and never a thread, and `main` is never a pool; only the deployed root's rows count, so an imported or non-root `main`'s entries cost nothing; a binding's reader thread and a stdlib transport's serve thread are not placement facts and are named as not counted
- lowering's deployment plan is the table's lowering view, read once before lowering (`collect_main_placement` over `LoweringView::placement`): no field name or written type decides a schedule class, a pool or a type set; a user site the table names is the node of the same index in the merged program, and the deciding entry's written selector says only whether one core is emitted alone or as a set
- subscriptions follow the tower (U-6): a nested instance's subscriptions register with its anchor's route, the pinned anchor's mailbox or the pool anchor's pool, never the program-wide queue; the route exists before the anchor's params are initialized and outlives every registration routed to it (each descendant deregisters in its dissolve, on the anchor's thread, and the join retires what is left before it destroys the mailbox)
- a pinned anchor's subtree initializes on the anchor's thread: the instantiating thread creates the route and the thread, the thread initializes the params (every nested construction, registration, birth and inline run(), a yield draining the anchor's mailbox) and reports ready, and the instantiating thread waits for it, servicing its own mailbox as a yield there would, before the literal completes; an override written at the literal is the instantiating code's and is evaluated there, and a locus it builds as the field's value is built on the anchor's thread; the init's temporaries are dissolved at its end, on the anchor's thread
- a pool anchor's subtree initializes on the pool's worker, as the anchor's first job there: the instantiating thread posts the params' initialization to the pool and waits for it as for a pinned anchor, and the worker runs every nested construction, registration, birth and inline run(), and the params' settle, a yield draining the pool's queue; the job never parks (on an async_io pool it runs on the worker's own stack), so the roots of one pool initialize in post order; a worker blocked on the instantiating thread's decision runs the posted initialization in place; the anchor's own birth() stays on the instantiating thread and its run() is posted behind the job
- the model's arrangement projects the table but covers less than the table, and the table answers every placement question: a literal `fn main` builds besides the root is not arranged (M-7: its birth is a hole of the model's dynamic births, the ownership family's legacy row, and arranging it moves with that row), nor is a row realizing a stdlib declaration (U-4: `Realizes` names an entry of `entities.loci`, which the hashed half renders, so arranging one moves `shape_hash`; carrying them is a model schema change of its own)
- F.38: placement is semantics-free, so a backend may Approximate it
- placement is a choice point: v1's declared placement is the single candidate
- the authored blocks are validated before they are rows: `check_pool_affinity` judges each `main locus`'s own `placement { }` block (an affinity on no named pool, two affinities for one pool), deployed or not, over the entry row's witness (`mains`), and that judgment is what lets the table keep one affinity per pool domain (rule 16); the per-block pool-to-affinity map it keeps is the block's well-formedness, not a placement fact, so it derives no row and no consumer reads it

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `placement_an_entry_without_its_root_row_is_refused_at_the_entry`; total: no root where the entry row names no `main locus` means nothing is deployed; no entry deciding a field means it runs where its owner does; no realized declaration (a hole) means the row joins no thread set; and no anchor in the route set means it has no nested subscriber

**Focused tests.** crates/hale-types/tests/placement_golden.rs (static instance rows, root, template bounds, domains and holes, plus the dynamic domain and hole projection read by consumers, over the corpus, tests/hale, DNA and placement fixtures; each seed by id, shared row groups by hash; HALE_SHADOW_REGEN regenerates); crates/hale-types/tests/placement.rs; crates/hale-types/tests/placement_pairings.rs; crates/hale-codegen/tests/pool_affinity.rs; crates/hale-codegen/tests/placement_where_async_io.rs; crates/hale-types/tests/placement_table.rs (the table through the frontend's load: the correspondence's coverage cases 1 to 16, the two universes joined into lowering's mint, the table's laws over every clean fixture, and a check that demands it once); crates/hale-types/tests/form_rows.rs (sync inference per instance, K-5); crates/hale-codegen/tests/nested_offthread_delivery.rs (the receiving thread of a nested subscriber, recorded inside its handler and inside its birth() and run(), in both devirtualization arms; the startup handshake, a nested child waiting during its anchor's initialization for a delivery through the anchor's route, or for a reply from a subscriber on the instantiating thread (and its control with that thread's drain off); a temporary of a pinned default, dissolved at the init's end; the same startup under a pool anchor (the review's program beside pinned, a child two levels down, two pool roots on one pool, an async_io pool, the round trip through main and its control, a worker blocked on a held failure running the init); the route's IR, for both threads, and its teardown under ASan); crates/hale-types/tests/intra_locus_pool_safety.rs (the intra-locus rewrite keeps a publish into a field off its owner's thread on the bus, through the lowering view); crates/hale-types/tests/model_arrangement.rs (the model's arrangement as the table's rows: M-1, M-3, M-5, M-7, M-9, U-4 and U-5, each against the old build's ids and hashes); crates/hale-types/tests/resource_budget.rs (the budget's rows R-1 to R-7 and checkpoint 4 through the frontend's load: replicas, one worker per pool, affinity, imported roots, constructions times their bounds and the uncertainty, adapters once, main never a pool, the dump's counted and not-counted lines); crates/hale-codegen/tests/placement_occurrences.rs (checkpoint 1: one template, one domain, its bound multiplying the budget; two live occurrences of one literal on two anchor threads, each nested subscriber on its own, in both devirtualization arms)

**Spec.** spec/semantics.md § Placement block (F.31); spec/decisions.md F.31, F.35, F.38; spec/runtime.md § Placement classes (m28b: subscriptions follow the tower)

**Guarded seams.**

- `derive_placement(` may be referenced from: `crates/hale-types/src/placement.rs` ×2, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/sync_inference.rs` ×1
- `bundle_placement(` may be referenced from: `crates/hale-types/src/placement.rs` ×1, `crates/hale-types/src/lifecycle/derive.rs` ×1
- `collect_main_placement(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×2
- `route_anchors(` may be referenced from: `crates/hale-types/src/resolved.rs` ×2
- `placed_in.push(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×1
- `affined_to.push(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×1
- `project_arrangement(` may be referenced from: `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `check_pool_affinity(` may be referenced from: `crates/hale-types/src/check.rs` ×2

### `target_capability` — Canonical · capability

**Answers.** What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability.

**Inputs.** --target; a source `target` declaration; stdlib_surface; FFI signatures

**Producer.** `crates/hale-types/src/capability.rs` · `derive_capability_matrix`

**Also owned.** `crates/hale-types/src/capability/uses.rs` · `derive_capability_uses`; `crates/hale-types/src/capability/uses.rs` · `admission_diags`

**Consumers.** check; build; docs (systems/webassembly.md § What wasm32 can do, and spec/ffi.md's refused-namespace table: regions rendered from the cells) (`crates/hale-types/src/capability.rs` · `render_markdown`); the effective-target row, on the snapshot: the check's target and its conflict refusal, the editor's alike (`crates/hale-frontend/src/snapshot.rs` · `demand_target`); hale build's backend and artifact naming, from the effective target (`crates/hale-cli/src/verbs/build.rs` · `compile_target(row.effective)`); hale run and replay refuse a program whose effective target is wasm32 (`crates/hale-cli/src/shared/options.rs` · `refuse_unexecutable`); the use rows, on the snapshot: the check's input beside the row (`crates/hale-frontend/src/snapshot.rs` · `demand_capability_uses`); the admission law: every use's cell for the effective target, in the check of every entry point (`crates/hale-types/src/check.rs` · `admission_diags`); hale check and hale build: every link input (--link, each package's [ffi] link) held to LinkLibrary before any tool, located at its manifest line or flag (`crates/hale-cli/src/shared/options.rs` · `link_refusals`); a build handed link libraries (the harness, a library build): LinkLibrary read off the lowering view's column before lowering and before any tool (`crates/hale-codegen/src/codegen.rs` · `Capability::LinkLibrary`); lowering: every behaviour and obligation emitted or omitted per target, read off the lowering view's column (`LoweringView::cells`) (`crates/hale-codegen/src/codegen.rs` · `self.cells.`); the wasm32 link: the module's fixed exports are ExportSurface's lowering data (`crates/hale-codegen/src/codegen.rs` · `Lowering::Exports(fixed)`); the check of an @ffi or @export signature: the FfiType cells for its ABI on the effective target (`crates/hale-types/src/check.rs` · `ffi_type_refusal`); the teardown spines: the pool join, wait-abort and ingress quiesce each spine owes, in the lifecycle plan's order (`crates/hale-codegen/src/codegen.rs` · `emit_teardown_obligations`)

**Invariants.**

- Approximate is legitimate only in layers 5 and 7; everywhere else a target lowers or rejects, with the row's witness
- the docs' target statement is generated, never hand-maintained
- every (target class, capability), (target class, invocation) and (target class, obligation) pair has exactly one written cell: no default arm, the musl column written out like the others
- behaviours and obligations are distinct types: an obligation is omitted only on a premise (a Reject behaviour, a Refused invocation, a registered proof) that holds on its own target, and a Lower behaviour requires only Lower behaviours
- a capability refusal depends only on the program, the configuration and the target; a failure that depends on the machine running the compiler stays a toolchain error
- a target's class comes from its (arch, os, env), never from a triple's name; the effective target is a function of the snapshot key's configured target and its sources
- one effective target for analysis and emission alike (T1(b)): an explicit `--target`, else a written `target wasm`/`browser_js` declaration (wasm32), else the host; a `--target` of another class than a written declaration's is refused at the declaration, and a declaration `--wrap-main` injects never selects
- every use is read off resolved identities (a call, a method through its receiver's type, a cross-seed alias, a construction and the lifecycle it implies), never a `std::` spelling of the call; a type-only mention is not a use
- the program's own sources are the horizon: a use is refused once, at its first site in them, naming the capability, the target and its witness chain; a callee beyond it is refused at the call that crosses into it
- an unresolved requirement is never an admission on a target that rejects anything in its family: a hole is refused there and recorded elsewhere
- a policy refusal comes before any tool is probed: a link input the target refuses is refused by the check and by the build before clang, wasm-ld or zig is looked up, located at its input (T4)
- emission configuration through TargetSpec only, every capability through the matrix: codegen's remaining wasm-ness reads are the emission choices and the link path, and every behaviour and obligation it emits per target is a read of the lowering view's cells, which the view takes from the effective target and lowering refuses options of another class against
- the matrix selects a target's lifecycle obligations and the lifecycle plan orders them, alike in every teardown spine (the quiesce, then the wait-abort, then the pool join)
- the portable subset prints the same bytes natively and under node; the comparison proves agreement for its programs and admits nothing outside them

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-types/src/capability/laws.rs` · `every_pair_has_exactly_one_cell`

**Focused tests.** crates/hale-types/tests/wasm_target_gating.rs; crates/hale-codegen/tests/wasm_target.rs; crates/hale-cli/tests/target_model.rs; crates/hale-cli/tests/wasm_package_csrc.rs (T4: a package's [ffi] link refused by check and build alike at the manifest's line; --link named as the flag; the refusal before any tool, on a PATH with no clang); crates/hale-cli/tests/target_precedence.rs (the precedence table: source x --target, check, build and the editor agreeing per cell; wasm-flower built for its declared target; run refusing a declared program; the agreement test: every wasm-relevant program's located refusals equal on check, build and the editor, with and without --target wasm32); crates/hale-types/src/capability/laws.rs (the matrix's laws: one cell per pair, anchored witnesses, premises, requires; KNOWN_OPEN empty since T2, T3 and T5 landed, a new entry held to today's answer); crates/hale-types/src/target.rs (async_io_follows_the_libc: TargetSpec::has_async_io is the runtime's shape, the AsyncIoPool cell what a program may ask for; they differ on wasm32 alone); crates/hale-types/tests/shadow_capability.rs (the checker rows against their cells over the corpus, tests/hale, the DNA seeds and the wasm programs, on three columns: 0 divergences); crates/hale-codegen/tests/shadow_capability_lowering.rs (the codegen rows, the thread behaviours and @ffi("js") against their cells, both targets built, a harness build refused by the admission before lowering; 7 classified divergences, all the design's: 4 declared export-only programs and 3 declared @ffi("js") programs a harness lowers natively anyway, where codegen still says `program has no fn main()` and the native link fails); crates/hale-cli/tests/shadow_capability_cli.rs (run, replay, record and --wrap-main against their cells; 0 divergences); crates/hale-types/tests/capability_uses.rs (the use producer's acceptance cases: a stdlib call, a construction with its lifecycle, a handle's method, a wrapper refused once, module-nested and on_failure bodies, a hole, declaration rows, an export-only program, an exported run(); T2's pinned, pool, async_io and transport refusals at the entry or binding; T3's stub namespaces; T5's @ffi("js") on a native target, called or not; each type-only variant admitted); crates/hale-types/tests/capability_doc_matches.rs (both document regions equal the rendered matrix); crates/hale-codegen/tests/target_lifecycle_cells.rs (the five teardown spines on both targets, each owing what its target's cells select, in the plan's order); crates/hale-codegen/tests/portable_subset.rs (the design's programs and the playground's examples print the same bytes natively and under node; skipped, naming what is missing, without node, clang or wasm-ld); crates/hale-codegen/tests/wasm_import_backstop.rs (T7: every module the wasm tests build imports only the loader's writers, read from the loader's source, plus its declared @ffi("js") names, or a known-open name from its stated callers (dprintf and fflush, the unabsorbed-violation report, until the loader has a stderr writer); a control module importing an undefined @ffi("c") name reported by name; each known-open name still imported; no observation probe emitted for wasm32, against the native build of the same program; skipped, naming what is missing, without clang or wasm-ld)

**Spec.** spec/decisions.md F.35; spec/ffi.md § The `target` declaration + stdlib gating; docs/src/systems/webassembly.md

**Guarded seams.**

- `derive_capability_matrix(` may be referenced from: `crates/hale-types/src/capability.rs` ×5, `crates/hale-types/src/capability/transport.rs` ×1, `crates/hale-types/src/capability/laws.rs` ×12, `crates/hale-types/src/capability/uses.rs` ×2, `crates/hale-types/src/target.rs` ×1, `crates/hale-cli/src/shared/options.rs` ×1
- `derive_capability_uses(` may be referenced from: `crates/hale-types/src/capability/uses.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×1
- `target_row(` may be referenced from: `crates/hale-types/src/capability.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×1

### `deployment` — Reserved · derivation

**Answers.** A deployment as typed rows: root and horizon, component identities, instances and incarnations, resources and allocations, endpoints and routes, hosting and authority, persistence obligations (the habitat, after phase 2).

**Inputs.** placement; bindings; admitted artifacts; a deployment dialect (later)

**Producer.** none: reserved, computes nothing.

**Consumers.** fleet (today's JSON as an input adapter); dna (the habitat); ui (reserved)

**Invariants.**

- habitat owns the hosting allocation and its obligations; the tenant keeps its own root and authority
- desired, admitted and observed stay distinct

**Missing data.** n/a

**Spec.** RFC #1212, the implementation plan §8; GH #262

## Layer 6 — lifecycle order

### `lifecycle_order` — Canonical · derivation

**Answers.** The happens-before order per instance: birth sequence, params open and settle, failure delivery and its execution domain, reclaim prerequisites, drain, restart, teardown.

**Inputs.** placement (the instance templates: the static towers, replicas apart, and the dynamic sites with their domains and bounds; the domains); handler_routing (the owner's handler for each child, and whether it restarts); flows (an accepted flow is reclaimed at its run's end, a resident by its owner's cascade); bus_graph (which instances subscribe); entrypoint (which `main locus` is the entry, whose deferred teardown is the main entry's spine); the declarations each template realizes (run, the closures' epochs, birth_check, violate sites, accept, on_failure, the params bracket); the decision lines (`hale_types::lifecycle::DECISION_LINES`) and the inventory rows they name; the runtime protocol (lotus_failure_hold / await / defer_reclaim; a run's retention, queued or started, lotus_run_cancel_only / lotus_run_cancel_queued and deferred storage release); the delivery's domain (lotus_failure_post: the owner's domain recorded where its params open, a delivery off it posted there and awaited, its cell's hold on the child; lotus_pinned_join and the pool join's wait for the joined domain's end)

**Producer.** `crates/hale-types/src/lifecycle/derive.rs` · `derive_lifecycle`

**Also owned.** `crates/hale-types/src/lifecycle.rs` · `LifecyclePlan`; `crates/hale-types/src/lifecycle/trace.rs` · `Expected`; `crates/hale-types/src/lifecycle/project.rs` · `expected`; `crates/hale-types/src/lifecycle/spine.rs` · `birth_order`; `crates/hale-types/src/lifecycle/spine.rs` · `reclaim_order`; `crates/hale-types/src/lifecycle/spine.rs` · `cascade_order`; `crates/hale-types/src/lifecycle/spine.rs` · `cascade_fields`; `crates/hale-types/src/lifecycle/spine.rs` · `recovery_order`; `crates/hale-types/src/lifecycle/spine.rs` · `entry_order`

**Consumers.** the lowering view (the snapshot's plan, carried to the emitters) (`crates/hale-frontend/src/snapshot.rs` · `view.lifecycle = Some(lifecycle)`); codegen (the process rows of every teardown spine, in the plan's order as the target's cells select them: a spine's head before its frame's pre-drain, the pre-drain and the rows after it in a fn main exit's flush) (`crates/hale-codegen/src/codegen.rs` · `emit_process_rows`); codegen (fn main's three exits, the fall-through, the test failure and `return`, through one helper: the exit spine's process rows, the frame, the process exit tail C24) (`crates/hale-codegen/src/codegen.rs` · `emit_main_exit`); codegen (a main-locus entry's teardown in the plan's order, `LifecyclePlan::entry_order`: the eager spine and the frame flush emit its head before its own pinned joins, the flush ahead of the first of them wherever its entry order puts it, and refuse a cascade the plan places earlier) (`crates/hale-codegen/src/codegen.rs` · `head_before_pinned_joins`); codegen (emission reads the order, spine by spine, through `LifecyclePlan::spine`); codegen (the restart and the resume: __restart_<L>, __resume_<L>) (`crates/hale-codegen/src/locus/restart.rs` · `define_restart_fns`); closures (the event alphabet); transitions (reserved); deployment (reserved)

**Invariants.**

- each order the emitters read (birth, reclaim, cascade, recovery, process, entry, readiness) is computed once per plan by the spine index (`SpineIndex`, built in `build_resolved` beside the plan), never per literal or per teardown: the plan's own readers give the same answers by the same per-instance code, `lifecycle_plan.rs` holds the two equal over every corpus plan and their pair sets equal to a reference, and `birth_spine_ir.rs` bounds the readers' passes over a plan by a constant
- equivalent parent execution contexts are represented once per owner (instantiating domain, queue domain, handler state); occurrence bounds still sum every owner, and domain claims retain every distinct context without enumerating ancestry paths
- a handler runs on its owner's domain (decision L0-1, L5's fourth part): a failure raised off it is posted there with the copied violation and the child retained, and the failing thread waits for the handler's return servicing its own queue; the domain runs it wherever it services its queue (main's drain, a pinned mailbox's drains and yields, a pool worker between cells, every servicing wait, and the pinned and pool joins, which run nothing else); a domain that has ended runs nothing more, and a delivery to it runs where it was raised; the owner's reclaim waits for the posted delivery and for the failing cell's hold on the child, except that it never waits for a delivery only its own thread can run: reached inside a handler for a child whose delivery is held for that thread (a replaced failing sibling), it follows that delivery, after the running handler returns; wasm32 keeps the in-place call (failure_delivery_domain, the matrix's C36 cells, jp_late_failure_*, l08_sibling_replaced_kept, cascade_model's phases 3 and 4)
- a spec/implementation disagreement is settled as a named decision, never by extraction picking a side
- an obligation is keyed by its source site (the declaration and P1's construction template); the runtime mints the instance and its incarnation, the table never does
- every obligation ends in exactly one of its named terminal alternatives; lifetime (what stays alive until which event) and progress (what makes it reach a terminal) are separate fields
- each rule says whether it is shipped, adopted, known open at an inventory row, or pending on a named condition
- the runtime's protocol is checked by its trace, not trusted: a trace build reports the hold, the settle and each held delivery, and the trace oracle holds them to the plan's edges (delivered after the owner's settle, before its birth), with negative controls that remove or reorder a step and fail it, over the lifecycle fixtures, every runnable example (its departures named in the corpus oracle's PLAN_KNOWN_OPEN: one, a statement-position literal's teardown the plan orders before its owner's reclaim but not before the end of the body that built it, C13; the closure with no `epoch` clause whose dissolve-epoch rows the producer did not derive is closed, C37) and the lifecycle matrix's program cells (a sample of 60 by default; all 121 under HALE_MATRIX=full)
- a run posted to a pool ends in exactly one of decision line 19's named terminals (L5): it is retained against its child's teardown, whose Reclaim begins by canceling the child's queued runs (`lotus_run_cancel_only`, past the reclaim latch on every reclaim path, before storage release), so a run queued on any pool finds its child whole or ends NotStarted(Acknowledged), and physical release waits for the child's started runs, each holding its child and owned descendants from admission until it returns; a queued main-thread handler defers physical release until it returns, while drain and dissolve remain before replacement construction; child Reclaim completion precedes owner Reclaim completion; a post refused at shutdown ends NotStarted(Shutdown(PoolShutdown)), a cell the pools' teardown frees NotStarted(Shutdown(PoolTeardown)), and a run post that cannot allocate aborts; under replay the ordering gate drops a canceled run before comparing it with the recording and holds a live one without admitting it, in a hold buffer that is its consumer thread's and is freed at the thread's exit
- one plan per snapshot, over the placement table's instance templates: a held row and the rows projected under it are their source's and owe nothing of their own, a hole owes nothing, and a dynamic literal's own fields are instances of their field literals in its domains, a field literal reached through several constructions of its owner's declaration one template that keeps each one's owner, domains and bound (its bound their sum, a nested field's the product of its owner's with its own placement), and a body or accepted literal one template whose owners are every template of its enclosing locus, each occurrence on the domain its enclosing occurrence runs it on (its bound the table's, which covers every enclosing scope); no check builds the plan
- a failure's rows are guarded, one set per source an instance can raise (a birth-epoch closure, the birth_check, a violate in run(), in a handler or in drain(), a dissolve-epoch closure, a closure with no `epoch` clause among them: its epoch is `ClosureDecl::epoch`'s, the rule the checker and lowering read): its delivery in place, the held alternative where the owner's params can still be open, and the recovery decision and the restart, performed or refused under teardown; a path through the plan picks one
- existence, each edge and each domain claim carry their own rule and status, so a row shipped to exist can carry a claim known open or pending (line 3); a domain is claimed only where the placement table and the rule resolve one for every occurrence: one domain, or the set where a template's occurrences are built under parents on different domains (each occurrence held to one of them), never one parent's alone
- a failure's delivery claims the owner occurrence's domain, shipped (decision L0-1: in place there, or posted there and awaited), and a held one the thread settling the owner, a pool-placed owner's worker included (line 1); where the owner's domain has ended before the failure is raised (a static instance's drain or dissolve on main after the pool join or its pinned anchor's join, under a pool worker or a pinned thread) it claims the raising thread under the shutdown rule (join progress); the joins' progress is shipped (C18, R20); a queued run's cancellation claims the reclaiming thread (R19a: main for a static instance)
- the plan the trace oracle holds a lifecycle fixture's, a matrix cell's or a runnable example's run to is the producer's for its program, rendered along the run's path (`lifecycle::project::expected`): no hand-written expectation stands beside it, the negative controls' own plans aside and the hand-written plans of the fixtures the producer does not derive yet (`UNDERIVED`: a field replaced while its failure is posted, C29), and a run's known departures are named per inventory row in the known-open tables
- occurrences of one declaration whose runs end differently in one execution are counted from the run's path: a teardown's cancellation is owed once per run the path cancels however many templates owe it, edges out of it are held only where every occurrence is canceled, and a run that never started is held to no domain (its not-started end is named where it was canceled or refused); a main locus built more than once enters its eager spine at each teardown, so those process rows name the literal (`Obligation::per_occurrence_of`) and are owed once per occurrence, and the first join owes no later construction's run
- a row whose every terminal is a not-started one is owed by no subject: a restart refused under teardown (RD, C42), a run a resumed locus does not declare (line 13, C48)
- a pinned or pool anchor's params initialize and settle on its own domain (C49/C50); each static nested field's instantiating domain follows that initialization, and the anchor's own birth runs there too (line 3, L4 5: a pool anchor's as a second job its instantiating thread posts after its registrations and waits for, the held decision staying on that thread; claimed there only where the main locus is built once, since a later construction's posted birth finds the pools joined and runs in place on the instantiating thread); a nested run inside a pool init is inline and owes no queue admission or queued-run cancellation
- an emitter reads its spine's obligations for an instance template from the plan (`LifecyclePlan::spine`, `birth_spine`, `birth_order`, `reclaim_order`, `cascade_order`), on the path no failure takes, each after every row its entry edges reach and ties in the producer's order; the steps it emits for a spine are exactly those, which the trace build shows per instance and spine over every fixture program with exact named departures in SPINE_KNOWN_OPEN (C29's alternate field-replacement spine remains to be derived), the Reclaim on the spine the plan holds it on and a run's Cancellation on its Reclaim's, read on the shutdown path (`shutdown_spine`)
- the process edges are the plan's (L4 3): the producer states every teardown spine's process rows (the eager main locus's C13, the deferred main entry's C19, `fn main`'s fall-through, test-failure and `return` exits C21–C23) as the ingress quiesce, the wait-abort and, with pools, the pool join with its progress, the exits then their frame's pre-drain (without pools the pre-drain before the wait-abort), and the edges between them; emission reads that order (`process_order`) and the capability matrix selects which rows a target emits (line 16); a run takes one of `fn main`'s exits, and a main locus a fn other than `main` builds owes its head before `fn main`'s exit
- one teardown order on every spine (L4 5): a main locus's head (the ingress quiesce, the wait-abort, with pools the pool join) precedes the join and the drain of every pinned anchor its teardown joins, with a pool or without (line 7's edges, which `entry_order` reads), so the deferred spine's flush emits the head before the first of the entry's own pinned joins (C14) as the eager spine does; a pinned field's join precedes its owner's drain (line 12), and a root handed back to its caller keeps its pinned fields' join records in their instances and joins them in its own cascade, wherever the caller ends it, the building frame's flush owning the join only when that frame owns the root at its exit (C52)
- a `Run` is owed exactly where lowering calls `run()` (C53): the producer and lowering read one test, `lifecycle::run_is_called` over `lifecycle::body_is_empty` (a body, or a flow's run, whose wrapper reclaims it), and a pinned locus's thread takes the step whatever the body; an empty `run() { }`, written or not, owes none
- the birth spine is emitted from the plan (L4 1): an instantiation's steps from params settle to the run's start (accept, registration, `birth()` with its birth-epoch closures and `birth_check`, readiness, the run's start) come in the order `birth_order` reads from the plan's rows and edges for its declaration, the registrations on the instantiating thread and the rest there or, for a pinned locus, on its thread; the emitter refuses a plan that puts a registration after the birth; a pinned locus's `birth_check` runs on its thread before `run()` (C38)
- an instance's header is initialized before anything can reach it (C54, a classified correction): its children list, restart, quarantine and drain latches, recpool fields, failure route, owner links, masks and reclaim slots, and a handed-back root's join records for its pinned fields' replicas (C52, cleared before any replica's literal writes its own) are stored right after its arena, before the owner singleton publishes it, before its pointer reaches the observer, the runtime or a child, and before any param is built, on every instantiation path, the cross-pool create cell's included (`emit_instance_header`, lowering's own preamble, which the plan does not name); nothing after the params stores a header field again, so a child the params bubble to it stays accepted and is torn down with it; a restart keeps the header but the two latches the failure raised (C42)
- readiness is the plan's (line 6): delivery to a subscriber is eligible once its birth and checks complete; what is published to it before is parked in order, never dropped (including a pinned subscriber, whose mailbox remains current for already-born nested subscribers), bounded at the queue's cap for a publisher on another thread, and dropped with the subscriber if it is quarantined first; no direct call delivers to it during its birth (a rewritten send keeps its exact receiver and uses its registered route while that receiver is held, and a baked direct publish uses runtime dispatch while any window is open)
- the reclaim spine is emitted from the plan (L4 2): every teardown funnels into `emit_locus_arena_destroy`, which emits an instance's reclaim in the order `reclaim_order` reads for its declaration: the owned children's reclaims before physical release (line 14), guarded logical entry, cancellation of still-queued runs, waiting for admitted runs and deferred descendants in the retained release callback (line 19, R19), the arena's release, then the struct's; the emitter refuses cancellation before entry, a wait before cancellation, or any storage release before its dependencies; a field owned through a pool-placed owner's params runs and subscribes on that pool (line 3, C12)
- the dissolve cascade is emitted from the plan (L4 2): an owner's teardown around its fields comes in the order `cascade_order` reads (the fields' drains, the owner's drain, its dissolve, the fields' dissolves, the reclaim), over the fields in the order `cascade_fields` reads (declaration order, line 12); a field typed by an interface or a perspective drains before its owner like any field (its slot holds the impl's drain and rest halves, C32), and a pinned locus's thread drains its fields before its own drain() (C9)
- restart and resume are emitted from the plan (L4 4): a restart's steps come in the order `recovery_order` reads from the plan's rows and edges for its declaration (the recovery decision once the handler has returned, the Restart after its completion (line RD), then the incarnation the restart begins: its rows owed once per incarnation, `birth()` with its birth-epoch closures, then `run()` only where the template declares one (line 13, C48)); a restart tears nothing down, the instance is the same one (C42); `__restart_<L>` emits the entry and the birth, the decision and the run stay with the caller on the spine that decides, and `__resume_<L>` starts a run only where the plan owes one (a flow's run end, its reclaim, aside). Departures the trace shows are named in SPINE_KNOWN_OPEN: a held run() failure's phase-0 resume the producer does not derive (C43), a held failure's restart performed by the resume at settle where the producer holds it on PoolRun (C42), and a restart asked for under the owner's teardown performed (C42, the adopted refusal's mechanism L5's)

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `lifecycle_order_a_view_without_the_plan_is_refused`; total: no template of a declaration in the plan (one only the stdlib or an imported seed builds) means it reads each spine's order from every template of the plan, then in the producer's order, and owes no readiness

**Focused tests.** crates/hale-types/src/lifecycle/derive.rs (shared ancestry regression); crates/hale-codegen/tests/reclaim_spine_ir.rs (reclaim ordering through retained callbacks, contract teardown halves, pinned field drains and declaration order); crates/hale-codegen/tests/birth_spine_ir.rs (the birth spine's IR per shape: a root child, a nested child, each replica, a pinned child with its birth_check before its run, a pool-placed child born on its pool's worker as a posted job, a cross-pool bubble accepted before birth); crates/hale-codegen/tests/header_before_params.rs (the header before the params: children a params default bubbles to a main locus, a field owner in its owner's arena and a pool-placed owner stay accepted and are torn down; a dirtied stack is never read; a pinned owner's on_failure at settle reads its own latch clear); crates/hale-codegen/tests/restart_spine_ir.rs (the restart and resume spines' IR per shape: a root child, a nested child, a pinned child, each replica, a pool-placed child, restart_in_place under a generic supervisor, a locus with no run() resumed without one); crates/hale-types/tests/lifecycle_plan.rs (a_restart_reads_decision_entry_birth_then_run); crates/hale-codegen/tests/topic_phase2.rs (a_births_own_publishes_are_delivered_after_it_in_order; sends directly in birth or in its helpers retain receiver identity and wait for readiness; pinned birth bursts and managed payloads run under ASan); crates/hale-types/tests/lifecycle_plan.rs (the plan through the snapshot: demanded once and built by no check; its laws over every corpus program the snapshot scopes, edges naming its own rows and acyclic on events, birth and teardown owed once per template, every line a decision line, every delivery to an owner of known domain naming its domain; the rows lines 1, 3, 7, 12, 13, 14, 18, 19 and RD are about; a cross-domain delivery's claim, posted or under the shutdown rule; a main locus built twice; started-run retention through physical reclaim; statement-position subscribers torn down at frame exit); crates/hale-codegen/tests/lifecycle_flow.rs; crates/hale-codegen/tests/reclamation_spine.rs; crates/hale-codegen/tests/main_locus_deferred_pool_join.rs; crates/hale-codegen/tests/teardown_pinned_join_order.rs; crates/hale-codegen/tests/frame_flush_ir.rs (the frame flush per fn main exit, with and without a pool: the head's rows, then the pre-drain and the rows after it, the entries subscription-less pinned first then reverse push; a deferred main entry's head before its pinned joins, in a fn other than main and in fn main's frame; a returned root's pinned field joined by its owner's teardown in the caller, and the control a frame keeps; another fn's flush pre-drains and aborts no wait); crates/hale-codegen/tests/reclaim_cancel_ir.rs (queued cancellation precedes the storage callback on every spine, and its hold wait dominates physical releases); crates/hale-codegen/tests/replay_canceled_run.rs (a canceled run replays clean; a held run keeps its retention; the hold buffer is freed by each thread that held, under ASan with no suppression); crates/hale-types/src/lifecycle.rs (the schema's laws: every decision line binds a kind, the Pending lines are the named ones, the doc table is the data); crates/hale-codegen/tests/lifecycle_fixtures.rs (a fixture per decision line under tests/fixtures/lifecycle/; KNOWN_OPEN pins today's outcome where it differs from the adopted one; the trace oracle holds each run to the producer's plan for the fixture's program, rendered along the run's path and its line's known-open rules, the line-19 fixtures' included, or, for a fixture in UNDERIVED (one field replacement, C29), to its hand-written plan; TRACE_KNOWN_OPEN names today's departures, CONTROLS fail it; every_spine_emits_the_plans_obligations_in_order holds each instance's emitted steps on the eight instance spines (the deferred entry's and the deferred main entry's among them) and reclaim/cancellation on every spine to the plan's order); crates/hale-types/tests/lifecycle_plan.rs (every_declaration_reads_one_birth_order_over_the_corpus, the_reader_orders_each_spine_by_the_plans_edges); crates/hale-types/src/lifecycle/trace.rs (the trace's parser and oracle); crates/hale-codegen/tests/corpus_oracle.rs (corpus_traces_keep_the_lifecycle_laws: every runnable example, traced, held to the trace's laws and to its program's plan along its run's path, the path stated where the run is not the default one (a failure, a branch not taken, a default its literals replace); PLAN_KNOWN_OPEN names today's departures); crates/hale-codegen/tests/lifecycle_matrix.rs (failure phase × tree position × domain, a generated program per cell held to its outcome, the producer's plan for the program rendered along the cell's run, ASan on the sample and the let-bound differential; KNOWN_OPEN names today's failing cells, none since the producer claims the shutdown rule's domain; HALE_MATRIX=full runs every cell); crates/hale-codegen/tests/target_lifecycle_cells.rs (the five teardown spines on both targets: each owes what its target's cells select, in the order the plan's process rows state, before the cascade; the call counts pinned per shape, variant and target); crates/hale-codegen/tests/failure_delivery_domain.rs (a pinned child's handler on its owner's thread, outside the owner's window; an owner's reclaim never under a delivery in flight; a pool-placed owner's held and posted deliveries on its worker, by thread id; all under ASan in both dispatch modes); crates/hale-codegen/tests/cascade_model.rs (the GenMC model compiled with every control, run natively once: the in-place control fails the domain assertion)

**Spec.** spec/runtime.md (failure delivery; pool join rule b); spec/runtime.md § Lifecycle obligations (the decision lines, adopted and shipped told apart); spec/runtime.md § The lifecycle trace (a debug aid, not a contract); spec/runtime.md § Lossless recording mode (the replay hold and a canceled run); spec/semantics.md § lifecycle

**Guarded seams.**

- `derive_lifecycle(` may be referenced from: `crates/hale-types/src/lifecycle/derive.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1
- `process_order(` may be referenced from: `crates/hale-types/src/lifecycle/spine.rs` ×3, `crates/hale-codegen/src/codegen.rs` ×1
- `entry_order(` may be referenced from: `crates/hale-types/src/lifecycle/spine.rs` ×3, `crates/hale-codegen/src/codegen.rs` ×1
- `emit_main_exit(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×4
- `recovery_order(` may be referenced from: `crates/hale-types/src/lifecycle/spine.rs` ×2, `crates/hale-codegen/src/locus/restart.rs` ×1

### `bus_inert` — Canonical · derivation

**Answers.** Whether the program can ever have a bus cell in flight, so drains can be elided.

**Inputs.** the user program's declarations (topics, perspectives, bus and bindings blocks, accepts); the names the user program spells (`hale_syntax::names`); the stdlib's bus-taint column (which `std::` namespaces reach bus surface)

**Producer.** `crates/hale-types/src/bus_inert.rs` · `bus_inert`

**Also owned.** `crates/hale-types/src/bus_inert.rs` · `bus_tainted_namespaces`; `crates/hale-syntax/src/names.rs` · `for_each_spelled`

**Consumers.** the resolved program (a row of the lowering view, over the user program before the stdlib merge) (`crates/hale-types/src/resolved.rs` · `bus_inert::bus_inert(`); codegen (drain elision: the row, read) (`crates/hale-codegen/src/codegen.rs` · `resolved.bus_inert`); codegen (`crates/hale-codegen/src/bus/runtime.rs` · `emit_bus_drain`)

**Invariants.**

- a drain elision is a conclusion of structural rows, never of a rendering: the declarations the user program carries, and the names it spells (`hale_syntax::names`, an exhaustive walk) against the stdlib's bus-taint column (`bus_tainted_namespaces`, a fixpoint over the stdlib's declarations by the same walk); lowering reads the verdict (`LoweringView::bus_inert`) and derives none
- the test is by name and over-approximates (a local spelled like a tainted namespace keeps the drains); a query over the message graph that follows which stdlib subscribers a program actually reaches would elide more, and is a separate change with its own shadow

**Missing data.** total: no absence is possible: the verdict is a value of every view, and `false` keeps every drain

**Focused tests.** crates/hale-codegen/tests/drain_elision.rs; crates/hale-codegen/tests/log_routing.rs; crates/hale-types/src/bus_inert.rs (the_verdict_reads_declarations_and_names)

**Spec.** spec/runtime.md § drain

**Guarded seams.**

- `bus_inert(` may be referenced from: `crates/hale-types/src/bus_inert.rs` ×2, `crates/hale-types/src/resolved.rs` ×1

## Layer 8 — lowering

### `law_backstops` — Canonical · law

**Answers.** The laws that replaced lowering's own refusals of rules the spec states.

**Inputs.** the AST; the placement table; the binding rows; the ownership graph; the typed-body table's `omitted_args`; the entry row

**Producer.** `crates/hale-types/src/lowering_laws.rs` · `lowering_laws`

**Consumers.** the check (every verb and the LSP) (`crates/hale-types/src/check.rs` · `lowering_laws`); the harness's lowering view (`Config::harness`), which is not gated on the check (`crates/hale-frontend/src/snapshot.rs` · `lowering_laws`); codegen (the backstop: a value use of a literal the plan posts to another thread, which the cross-pool value law judges, refused as its missing judgment) (`crates/hale-codegen/src/locus/instantiation.rs` · `emit_crosspool_bubble_spawn`)

**Invariants.**

- a law is judged once, with a span
- lowering judges no shape a law in `lowering_laws` covers: the check runs the laws among its rules, and the harness's lowering view demands them before it lowers, so those refusals reach no entry point unlocated (C7)
- rule 6 is judged per pinned instance, by the locus it realizes (an override literal's, a stdlib locus's), over the placement table's rows: a `pinned` entry's field and each replica, and an adapter inline in `bindings { }` (C7, 1)
- rule 17 is judged per root construction over the placement table: a literal of the root declaration (as resolved) written inside a loop body, whose template holds a row a `pinned` entry decides (C7, 2)
- rule 18 is judged per entry of the placement table's root over the inits its constructions supply, or the params default when one leaves the field or none builds the root (C7, 3)
- a cross-pool spawn is judged from the ownership graph's resolved birth rows and bubble plan: `bare_statement` records whether that literal is a discarded expression statement; a value use in a locus's own member bodies (birth checks and closure assertions included; an argument default's literal is not one, it is judged where it is expanded) is refused at the literal. The law does not rewalk literals or join by their written final segments (C7, 4; C3)
- the cross-pool value rule holds at every expansion of a default, through one relation, the ownership graph's `expansions`: each literal lowering builds from a default, with the locus it is lowered under (the plan's key there) and the root in that locus's own body that starts the expansion. A params default is expanded where a literal leaves its field unsupplied; a fn's or a locus method's argument default at each call that leaves the argument out (`omitted_args`; a call that supplies it expands nothing), so it is judged per invocation, in the caller's locus; chains of either kind are followed, and a literal the plan posts to another thread expands no params default under its context. A params default's literal is refused at the literal, once per context; an argument default's at the root, once per root, so a caller on the owner's thread is not refused. A `const`'s value and a type's field default are lowered at every use under a locus no row names, so they are refused at the position whenever some locus's plan posts the child (C3 rest and its review)
- lowering's backstop is an error, not a judgment: a cross-pool construction in value position that reaches the post (`emit_crosspool_bubble_spawn`, whose instance is born on the owner's thread and has no value here) is a `CodegenError` in every build profile, located at the literal, naming this law as the judgment that is missing; it never lowers a null value (C3 rest, the review of #1351)
- self-containment (GH #813, #870) is judged over every locus's params defaults, keyed (locus, supplied fields) as lowering expands them, through every literal anywhere in a default (each branch of an `if` or `match`, each statement of a block) and every fresh-factory product: a cycle is refused at the param that closes it, and lowering keeps no re-entry guard (C7, 5)
- decision 2's refusal (F.40 phase 3, L4): a seed whose only `main locus` is module-nested has no entry (`EntryRow::refused`) and is refused once, at that locus's name, so the harness lowers no program whose root is not the entry; it is one more diagnostic, and the other laws still judge that `main` over the table it seeds

**Missing data.** required: a missing row is a `CodegenError`, pinned by `crates/hale-codegen/tests/missing_rows.rs` · `law_backstops_a_value_use_the_law_did_not_judge_is_refused_at_the_literal`

**Focused tests.** crates/hale-types/tests/placement.rs; crates/hale-cli/tests/check_lowering_laws.rs (`hale check` and `hale build`); crates/hale-cli/tests/nested_main_transition.rs (decision 2's refusal at `hale check` and `hale build`); crates/hale-frontend/src/snapshot.rs (the_lowering_view_carries_the_entry_row: the refusal blocks the harness's view too); crates/hale-codegen/tests/harness_lowering_laws.rs (the harness, which skips the check); crates/hale-codegen/tests/deferred_slot_per_iteration.rs (rule 17 at the harness); crates/hale-codegen/tests/placement_factory_default.rs (rule 18 at the harness); crates/hale-types/tests/ownership_graph.rs (the cross-pool spawn law, every expansion context of a default included); crates/hale-types/tests/self_containing_locus.rs; crates/hale-codegen/tests/self_containing_locus.rs

**Spec.** spec/semantics.md rules 6, 17, 18 and § accept bubbling; spec/types.md § A locus may not contain itself by value; GH #813, #870, #876

**Guarded seams.**

- `emit_crosspool_bubble_spawn(` may be referenced from: `crates/hale-codegen/src/locus/instantiation.rs` ×2

## The law engine

### `model` — Canonical · derivation

**Answers.** The canonical semantic model of a checked bundle: fifteen entity tables, seventeen relation tables, holes, capabilities, provenance (GH #476).

**Inputs.** a checked bundle; top_scope; bus_graph; ownership; handler_routing; placement; effects; alloc_summary; topics; bindings

**Producer.** `crates/hale-types/src/model_builder.rs` · `derive_application_model_over`

**Also owned.** `crates/hale-types/src/model_builder.rs` · `ModelInputs`; `crates/hale-types/src/lib.rs` · `derive_application_model`

**Consumers.** demand (every verb and the LSP: the claims, over the snapshot's scope and graphs) (`crates/hale-frontend/src/snapshot.rs` · `derive_application_model_over`); a bundle no snapshot holds (the test entry's) (`crates/hale-types/src/lib.rs` · `derive_application_model_over`); claims (a caller not on the snapshot) (`crates/hale-types/src/judgment.rs` · `derive_application_model`); topology (`hale check`'s artifact and both gates: the snapshot's model) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `dump_topology_over`); topology (a bundle no snapshot holds) (`crates/hale-types/src/topology.rs` · `derive_application_model`); model dump (the check's snapshot) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_model`); the build identity: the model hash and the obs ids (build, run, replay: the snapshot's model) (`crates/hale-cli/src/shared/options.rs` · `demand_model`); fleet (admits the artifact, never the model)

**Invariants.**

- one constructor; no artifact → model, no plan → model, no hand-authored model
- hale-model is rebuilt on hale-graph (phase 1.1a): its seed, source and provenance ids and its provenance store are the graph core's, re-exported under the model's paths; its canary allows that one dependency and no other
- demand-gated: a no-claims check builds no model (GH #476 criterion 1); demand_gate.rs pins it as per-family accounting over `Snapshot::builds` (the `demand` family): the LSP's diagnostics path builds none, `hale check` of a program with claims builds one, which `--dump-model` reuses
- the model builds none of the families it reads beside the program (2.3): the scope with its topic rows, the bus graph, the ownership graph, the handler rows, the effect rows (with the stdlib-merged summary their walk read), the form rows and the placement table arrive as `ModelInputs`, each demanded once from the snapshot over the checked programs; the allocation summary it still re-runs for itself is listed under its family
- the arrangement is the placement table's rows projected, under three identity contracts kept apart: shape identity (the arrangement is outside the shape half, so no change to it moves `shape_hash`); observation entity ids (they stamp subjects, locus declarations and the deployed root's bindings, never instances); and arrangement-instance correspondence (`LocusInstanceId` is the index in path order, stable across no change of the arrangement; a consumer joins instances by path, and a replica index is the replica row's own, never a descendant's)

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/demand_gate.rs; crates/hale-model/tests/architecture.rs; crates/hale-types/tests/topology_projection.rs; crates/hale-types/tests/model_arrangement.rs (the arrangement's three identity contracts, each pinned against the build before the switch)

**Spec.** spec/model.md

**Guarded seams.**

- `derive_application_model(` may be referenced from: `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/judgment.rs` ×1, `crates/hale-types/src/topology.rs` ×2
- `derive_application_model_over(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

### `claims` — Canonical · law

**Answers.** Every user law: lowered claim rows, the judged verdicts over the model and evidence, constitution identities, and the artifact's law account.

**Inputs.** model; claim and constitution declarations; evidence (certificates, budgets); effects

**Producer.** `crates/hale-types/src/judgment.rs` · `claim_law_diags`

**Consumers.** check / verify; admission (`hale topology graph`, `hale fleet`: an artifact's law account is validated against itself, with the family's own functions; it recomputes the law digest and the evidence inputs digest, checks every reference against the artifact's catalogs, re-renders every stated form with the model's spelling (`hale_model::claim_form`) and recomputes a row's verdict and the document's with the model's aggregation (`certificate_row_verdict`, `document_verdict`); its decoder is the artifact schema's reader, since a payload carries no model ids and cannot rebuild a `ClaimIr`, held to the emitter by a round trip over every artifact the corpus emits, `law_account_round_trip.rs`, F.40 phase 4, A5) (`crates/hale-cli/src/topology_law.rs` · `validate_law_account`); the check's law-selection diagnostics (the snapshot's selection, handed in) (`crates/hale-types/src/check.rs` · `laws.diags`); the check's laws stage (judged over the snapshot's model, after its typing stage; its lowering reads the snapshot's selection) (`crates/hale-frontend/src/snapshot.rs` · `demand_laws`); topology (law section: the law rows, the constitution identities projected from the adoption, and the environment label, all the snapshot's selection, handed in) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_law_selection`); check --matrix (each pair's identity comparison: the roots its snapshot's selection adopted, `LawSelection::identities`, the projection the artifact reads; a pair the check refuses has no snapshot, so no constitution of its takes part) (`crates/hale-cli/src/verbs/check/matrix.rs` · `adopted_roots`); fleet; dna (dna_law.rs wording); model diff

**Invariants.**

- law selection runs once per snapshot (F.40 phase 4, A2): the `law_selection` cell (`Snapshot::demand_law_selection`) selects over the programs after the sequence, for the configuration's environment (its label and the constitutions it injected, passed as data, never a binding on the thread), and the check's selection diagnostics, the laws stage's lowering and the artifact's law rows, constitution identities (`LawSelection::identities`, a projection of the adoption) and environment label read that one selection, as does the environment matrix's identity comparison, from the snapshot its pair's check read (F.40 phase 4, A3: a pair loads its seed once, and a pair the check refuses, which has no snapshot, compares nothing); it is not gated on the typing, so a program that does not typecheck still answers it; a bundle no snapshot holds selects once per entry (`bundle_law_selection`)
- structural compiler laws are evaluated through model_query with shared witness rendering; the judgment path stays for user claims (final direction)
- a registered rule without an evaluator fails the compiler's own build
- a non-holds verdict is never silent
- admission evaluates no law; it refuses an artifact whose sections disagree (`spec/verification.md`)

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/claim_diags_snapshot.rs; crates/hale-cli/tests/law_selection_reaches_the_artifact.rs; crates/hale-cli/tests/law_account_round_trip.rs; crates/hale-cli/tests/dna_law.rs; crates/hale-types/tests/one_reachability_engine.rs

**Spec.** spec/verification.md § Claims; spec/model.md § Adding a judgment family

### `view` — Reserved · derivation

**Answers.** A named query over the tables: a node selector, a relation set and an adequacy policy, rendered by a backend (hale ui, after phase 2).

**Inputs.** the resolved program's tables; a view declaration (its own surface)

**Producer.** none: reserved, computes nothing.

**Consumers.** ui (reserved); bundle (reserved)

**Invariants.**

- a view is a query, not a claim: it yields a projection with an adequacy verdict
- adequate_for(RelationSet): a view may not show an absence its relations cannot see; a hole renders as a hole with its reason

**Missing data.** an unknown is a hole with a stated policy

**Spec.** RFC #1212, the UI comment

## Identity

### `snapshot_identity` — Canonical · derivation

**Answers.** The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again in the resolved-program step (numbered over the user program before the intra-locus rewrite, so the sends it records are numbered on every path, and minted over the merged program with the bundle's seeds and a named seed for the bundled stdlib), idempotently (one numbering; a later mint numbers only what an earlier one did not see), with reliable provenance; and which declaration each use names (`binding_of`), resolved once by the mint.

**Inputs.** seed_loading; desugar_sequence

**Producer.** `crates/hale-types/src/snapshot.rs` · `mint`

**Also owned.** `crates/hale-syntax/src/sites.rs` · `SiteKind`; `crates/hale-types/src/snapshot.rs` · `resolve_uses`; `crates/hale-types/src/snapshot.rs` · `declaration_of`; `crates/hale-types/src/snapshot.rs` · `number`

**Consumers.** every table; the shadow facility (compares through an explicit correspondence, never raw id equality); lsp (a later incremental future); the resolved program (codegen's input is minted over the merged program); the model's test entry (a bundle nothing minted is minted over clones of its programs, since the arrangement is the placement table's rows) (`crates/hale-types/src/lib.rs` · `derive_application_model`)

**Invariants.**

- addresses are not identities (declarations are cloned); spans are not (the stdlib's coordinates overlap user files; desugars share spans)
- snapshot-local uniqueness and provenance are the requirement; persistent identity across editor revisions is a separate problem
- canonical ids need real equality and hashing; the AST's structural NodeId equality stays separate
- the identity's types are hale_graph::ids (SeedId, SiteId; phase 1.1a); hale_types::snapshot::mint numbers every site the AST walk hale_syntax::sites reaches with one counter: after the entry point's last desugar, and again, idempotently, in the resolved-program step over the merged program, numbering only what an earlier mint did not see (phase 1.1b); before the intra-locus rewrite that step only numbers the user program (`snapshot::number`: the rewrite moves a send's id onto its call and records it), which makes no snapshot of it; every entry point calls it after its last desugar with its source map and the bundle carries the result (every verb and the LSP through the snapshot's load, the test harness through `Snapshot::from_program`, with no source map, and the bare program's view, `resolved::resolve_program`, before its rewrite, so its correspondence has checked identities to join to); the lowering view mints with the bundle's seeds, the stdlib's sites under the named seed snapshot::STDLIB_SEED (its spans overlap the first file's); a generated site records the desugar that made it (Snapshot::origins); codegen's generic instantiation keeps the template's id, and the F.39 pre-pass numbers nothing: an unminted Struct or Call is an error
- every identifier expression is a `Use` site and every name a declaration with no site of its own binds (a fn's or a hook's parameter, a match pattern's binding, a tuple `let`'s name, a `shm_write` binding) a `Binder` site, their ids on their `Ident`s (use-site identity, phase 2)
- `binding_of` is one resolution per use, keyed by identity, never by name or span: the mint resolves each `Use` site, and each `Assign` site's head, to the `Let`, `For` or `Binder` site it names under the checker's scoping (`check::ScopeStack`), once per snapshot (the load's, the lowering view's, the bundled stdlib's analysis copy's once per process: `demand_gate` pins it); a use that names no local binding has no row; every reader asks `Snapshot::declaration_of` and resolves nothing itself, and every entry point mints (`check_program` too)
- a check of a bundle no entry minted (`Bundle::new` over parsed programs, a library caller's or a test's) mints a copy of its programs once, with its source map, before any family is derived (`with_identities`, the no-snapshot adapter of `check_bundle` and `check::check_bundle`): its bus graph's sends and the intra-locus rewrite's relation, whose own numbering keeps those ids, name a send by one id, so rule 10's join answers on that path as on the snapshot's; a send with no id reaching the join is refused as an internal failure naming the send, never judged as queued
- an analysis row (`FnKey`) is its declaration's identity: `decl`, the site the universe that minted it numbered (the stdlib's analysis copy is its own universe), for every fn-like member (a free fn, a locus method, a lifecycle hook, a mode; a failure handler, a perspective fn, a constant and a synthesized hook as declaration bodies); the (locus, fn) pair is the row's display name and leads its order. A call reaches a row through the summary's one resolution (`AllocSummary::resolve`), and every reader holding a declaration builds its key from that declaration's site; a monomorph's rows are its template's. A key with no identity is a declaration no mint numbered, or frontier's one fallback for a subscription's handler the summary holds no row for (C3 rest)
- `FunctionId` is the model IR's name-ordered id (spec/model.md law 2: a function row's id is the rank of its canonical name, and `validate`'s sorted names hold); the row carries the declaration it is (`Function::decl`), never rendered and never hashed, and every builder join from an analysis row to its function reads that column (`fn_of_site`). A name answers only the resolution of author text (a group selector's member, a claim's function, an unresolved call's callee by its shape) and a key no mint numbered: `fn_id.get(` is the model builder's and the claim lowering's alone (C3 rest)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/fn_key_identity.rs; crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding); crates/hale-codegen/tests/owner_table.rs; crates/hale-types/tests/snapshot.rs (each_use_resolves_to_the_declaration_in_scope); crates/hale-types/tests/demand_gate.rs (each_snapshot_resolves_its_uses_once); crates/hale-syntax/tests/sites.rs

**Spec.** spec/decisions.md F.39, F.40

**Guarded seams.**

- `mint(` may be referenced from: `crates/hale-types/src/resolved.rs` ×2, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/stdlib_bodies.rs` ×1, `crates/hale-types/src/alloc_summary.rs` ×1, `crates/hale-types/src/sync_inference.rs` ×1
- `type_expr_identity(` may be referenced from: `crates/hale-types/src/check.rs` ×7
- `fn_id.get(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×8, `crates/hale-types/src/claim_lowering.rs` ×1
- `FnKey::method(None` may be referenced from: `crates/hale-types/src/frontier.rs` ×1

### `demand` — Canonical · derivation

**Answers.** Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, each member's own program beside the merged one, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph, handler rows and flow rows, law selection, the role rows, the arrangement, the effect rows, the model, the check in its two stages with the effects certificate report its typing produced, the lowering view), each at most once, blocking a family whose prerequisite reported errors.

**Inputs.** seed_loading; desugar_sequence; snapshot_identity; the config (target, api, api roles, environment, the check's rules); editor overlays (LSP); a consumer's request

**Producer.** `crates/hale-frontend/src/snapshot.rs` · `Snapshot`

**Also owned.** `crates/hale-frontend/src/snapshot.rs` · `demand_entry`; `crates/hale-frontend/src/snapshot.rs` · `demand_scope`; `crates/hale-frontend/src/snapshot.rs` · `demand_editor_scope`; `crates/hale-frontend/src/snapshot.rs` · `demand_bus_graph`; `crates/hale-frontend/src/snapshot.rs` · `demand_ownership_graph`; `crates/hale-frontend/src/snapshot.rs` · `demand_handlers`; `crates/hale-frontend/src/snapshot.rs` · `demand_flows`; `crates/hale-frontend/src/snapshot.rs` · `demand_law_selection`; `crates/hale-frontend/src/snapshot.rs` · `demand_role_rows`; `crates/hale-frontend/src/snapshot.rs` · `demand_arrangement`; `crates/hale-frontend/src/snapshot.rs` · `demand_effects`; `crates/hale-frontend/src/snapshot.rs` · `demand_effect_certificates`; `crates/hale-frontend/src/snapshot.rs` · `demand_model`; `crates/hale-frontend/src/snapshot.rs` · `demand_typing`; `crates/hale-frontend/src/snapshot.rs` · `demand_laws`; `crates/hale-frontend/src/snapshot.rs` · `demand_check`; `crates/hale-frontend/src/snapshot.rs` · `demand_lowering`; `crates/hale-frontend/src/snapshot.rs` · `from_program`; `crates/hale-frontend/src/snapshot.rs` · `SnapshotKey`; `crates/hale-frontend/src/snapshot.rs` · `declarations`; `crates/hale-frontend/src/snapshot.rs` · `declaration_dependents`; `crates/hale-frontend/src/dependents.rs` · `DependencyIndex`; `crates/hale-frontend/src/snapshot.rs` · `reusing_typing`; `crates/hale-frontend/src/snapshot.rs` · `typing_reuse`; `crates/hale-frontend/src/typing_reuse.rs` · `ReuseKey`; `crates/hale-frontend/src/typing_reuse.rs` · `plan`

**Consumers.** check (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_check`); the checker's rules (the handler rows: duplicate handlers, `@supervised`; the entry row: rule 1 and rule 9; the placement table: the F.31 rule per instance, the blocking check, pinned-in-a-loop, rule 20's construction paths; the bus graph: rules 7, 9 and 10, and the role law's sites; the ownership graph: rule 20; law selection: the law-selection diagnostics; the role rows: the role rules; the sequence's surface: the api entry's rules), demanded before the check runs; the effect rows' purity column for a codec binding, demanded only when one reaches the assertion (`crates/hale-frontend/src/snapshot.rs` · `CheckInputs`); topology (`--dump-topology`, `--check-topology`, `--check-topology-shape`: one artifact of the snapshot's model) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `dump_topology_over`); the api description (`--dump-api`: the surface the snapshot's sequence generated the binding for) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `api_surface`); the model dump (`--dump-model`: the check's own model when it judged a law) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_model`); the build identity (build, run, replay: the model hash and the obs ids from the snapshot's model, the plan digest from its lowering view) (`crates/hale-cli/src/shared/options.rs` · `model_identity`); build (`crates/hale-cli/src/verbs/build.rs` · `demand_lowering`); run <file> and run <dir> (`crates/hale-cli/src/verbs/run.rs` · `demand_lowering`); test (a build config for the host, the dev profile) (`crates/hale-cli/src/verbs/test.rs` · `demand_lowering`); replay (the identity admitted before lowering) (`crates/hale-cli/src/verbs/replay.rs` · `demand_lowering`); bench (the driver an overlay on the bench file) (`crates/hale-cli/src/verbs/bench.rs` · `demand_lowering`); the test harness (`tests/support/build.rs` of the codegen crate: `Snapshot::load` of text or a directory, `Snapshot::from_program` for a program a test made; lowering not gated on a check); lsp (the first publication: one snapshot per publish pass, its typing stage) (`crates/hale-lsp/src/lib.rs` · `demand_typing`); lsp (the typing stage reuses the seed's last typed snapshot, `State::typed`) (`crates/hale-lsp/src/lib.rs` · `reusing_typing`); lsp (the second publication: the whole check, for the files the laws add to) (`crates/hale-lsp/src/lib.rs` · `demand_check`); lsp (every request loads the snapshot the diagnostics load, `editor_snapshot`, one per request) (`crates/hale-lsp/src/lib.rs` · `editor_snapshot`); lsp (definition, placement, the allocation survey) (`crates/hale-lsp/src/lib.rs` · `demand_scope`); lsp (completion, hover, references, enforcement) (`crates/hale-lsp/src/lib.rs` · `demand_editor_scope`); lsp (hale/busGraph) (`crates/hale-lsp/src/lib.rs` · `demand_bus_graph`); model (the effect rows, demanded with the graphs it reads) (`crates/hale-frontend/src/snapshot.rs` · `demand_effects`); the effects manifest (`--dump-effects-manifest`, `--check-effects-manifest`) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_effects`); replay (the live-effects gate) (`crates/hale-cli/src/verbs/replay.rs` · `demand_effects`); the check's laws (their certificate evidence reads the typing's effects certificate report) (`crates/hale-frontend/src/snapshot.rs` · `demand_effect_certificates`); topology (the artifact's law evidence reads the same report) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_effect_certificates`); lsp (documentSymbol: the open file's member program, its own declarations as written) (`crates/hale-lsp/src/lib.rs` · `snap.member(`)

**Invariants.**

- every editor request reads the snapshot: the outline reads the open file's member program (`Snapshot::member`, the file as it parsed, before the merge and the sequence), so it answers while another member does not parse or an import does not resolve (the editor's load keeps the members of a seed whose link it refused, every family blocked) and lists only what the file itself declares; a file that does not parse has no outline
- the check is two stages and their composition (F.40 phase 3, X1): `demand_typing` is everything that needs no model — the scope's and the typing's diagnostics finished (`finish_check_diags`: the user's spelling, no repeat), then the build rules where the config asks (`Config::build_rules`: a build's and the editor's) and the allocation advisory where it asks (`Config::alloc_advisory`: the editor's; `hale check` runs its own beside its reports, under its flag); `demand_laws` is the laws judged over the model when the typing's own diagnostics denote one and the program has a claim surface, finished after the typing's own (`finish_check_diags_after`), and empty otherwise; `demand_check` is the typing stage's diagnostics followed by the laws', so every entry point's set is one pass's over both by construction, and only the position of a build rule or an advisory relative to a law differs from the single pass it replaced (both stages before; the laws last). Each stage is counted once (`Snapshot::builds`, the `STAGES` beside the families)
- the editor publishes the two stages apart (X1, `check_and_publish`): the first publication is every file of the seed with the typing stage, placed as `hale check` places it (each diagnostic spelled, suppressed — `retain_owned_advisories`, a stdlib-origin span — and placed on its own), with every file the seed's last publication covered and this one does not published empty (`State::published`); the second is the whole check (`demand_check`, the laws after the typing), sent only for the files whose list it changes, so a file's final list is its first followed by the laws placed in it, every file's last publication is `hale check`'s for it, and a seed with no broken law gets one publication; a seed the snapshot does not check (a refused load, a hole, the stdlib cache) gets its one. Before each publication, and before the laws are judged, the pass asks whether a document event is queued behind it: if one is, the buffers it read are superseded, what is unsent is discarded with the rest of the pass, `published` keeps only what was sent, and the pass's files join the next pass (`State::pending`)
- a prerequisite runs once: every family is a `OnceCell` of its snapshot, and a family that reads another demands it rather than building its own; `Snapshot::builds` counts each family's demands on the snapshot (its producer's runs in the snapshot's own cell), and no count exceeds one on any consumer on the snapshot. It does not count a producer run inside another family's cell, nor a family it has no row for. The lowering view derives none of the snapshot's families again: its scope is the snapshot's; its ownership and bus graphs (F.40 phase 3, C5) and its handler rows (F.40 phase 4, Q1) are the snapshot's rows read through the view's correspondence with the stdlib's after them; its flow rows are the snapshot's `flows` cell, read by locus name; its dispatch domains are the snapshot's `arrangement` cell, the projection the model reads; and its routing rows take the scratch-local set the allocation summary classified (`AllocSummary::scratch_local`), the classifier's one run. So on a build path the scope, each graph, the handler rows, the flow rows and the arrangement are each derived once per snapshot, as `builds()` says, and the scratch-local classification once, which a thread-local count says (`demand_gate.rs`); the bare-program test entry (`resolve_program`) derives its own
- a family nobody requested is not computed: the no-claims editor path builds no model (GH #476 criterion 1); the bus graph (rules 7, 9 and 10) and the ownership graph (rule 20) it builds are the check's, once each
- the model's inputs are families (2.3): `demand_model` demands the scope, the bus graph, the ownership graph, the handler rows, the effect rows and the arrangement over the checked programs (the `bus_graph`, `ownership`, `handler_routing`, `effects` and `arrangement` counts), each once
- the checker's inputs are families (2.3): the typing demands the handler rows, the flow rows, the entry row, the ownership graph, law selection and the role rows before the checker runs and hands them in (`CheckInputs`), with the surface the sequence generated the binding from (`Snapshot::api_surface`), so a check with a law builds the handler rows and the ownership graph once for the checker and the model together, and selects its laws once for the checker, the laws stage and the artifact (F.40 phase 4, A2: the `law_selection` count), and the role rules read the one set of role rows (A4: the `api_surface` count); the producers read declarations and bodies, not types, so they are total over a program that does not typecheck
- a family whose prerequisite reported errors is `Blocked { family, because }`, not computed: an editor seed with a member that did not parse or would not read has no scope (the editor's requests read `demand_editor_scope`, the scope over the members that parsed with the hole named, and never a scope of their own), a program that does not typecheck has no model, a program whose check reported an error has no lowering view; a ready result may still hold typed holes
- the lowering view (`LoweringView`, the `lowering_view` count) is a family: `demand_lowering` demands the check and the lifecycle plan (which it carries to the emitters, `LoweringView::lifecycle`), then runs `resolve_rewritten` once over the intra-locus stage (`demand_intra_locus`, the `intra_locus` count, which the check demanded too) with the snapshot's identities, source map, renames and api config; `build_resolved` reads it by reference; the view's correspondence (`hale_types::correspondence`, F.40 phase 3, C5) places every site of the merged program it lowers as a checked site (the same index, kind and span), the call the intra-locus rewrite put in a send's place (the send, by the rewrite's relation) or the stdlib analysis copy's site in the same walk position, and every checked site as the image of one merged site or the subject of a send a rewrite erased (`TopicRewrite::erased`, `IntraLocusRewrite::erased`); a site it cannot place refuses the view, and the law holds over the corpus, `tests/hale` and the DNA seeds (`lowering_correspondence`); every build path (build, run, test, replay, bench) demands it from a `Snapshot::load`, and the test harness from `Snapshot::load` over an overlay buffer or a directory (or from `Snapshot::from_program` for a program a test made), whose config (`Config::harness`) does not gate lowering on the check
- `Snapshot::from_program` is the frontend's entry for tests that construct a view directly: no `crates/*/src` file calls it (its seam's allowance is empty), so every verb, and the codegen crate, reach a snapshot through `Snapshot::load`; a bare program has no source map and its sites are seeded by ordinal
- a changed entry, load mode, target, config, overlay or source text is a distinct snapshot (`SnapshotKey`, computed after the load from what it read; a bare program is its own load); two snapshots share no result but the typing reuse below, which a second key names
- the editor's typing stage is incremental by declaration and equals the full one by construction (F.40 phase 3, X2, `Snapshot::reusing_typing`, `hale_frontend::typing_reuse`): the snapshot key stays the whole seed's; a second key (`ReuseKey`: the entry, the load mode, the target, the config digest, the import renames, one row per alias and declaration in the load's stable order) names the reuse, and a previous snapshot whose second key differs, or whose typing never ran, offers nothing. The check's per-declaration passes (the checker's walk of a top-level declaration and the reveal rule's, `check::check_bundle_by_declaration`, `DeclChecked`) keep their result per declaration; a new snapshot of the seed reuses it for every declaration that is unchanged (the previous one moved by the distance its start moved is the new one, position for position), is no dependent of a changed one through the families (`declaration_dependents`, in the previous snapshot and the new one) and has no diagnostic position outside itself, moved the same distance, and runs the passes for the rest; a changed declaration is one whose bodies alone changed (equal once positions are erased and bodies set aside), and any other change (a declaration added, removed, renamed or reordered, an edit to what a declaration declares, a changed declaration the families do not place, a changed set of programs) checks the seed whole (`TypingReuse::Whole`, with the reason). Everything else the typing stage runs (the scope, the bundle-wide rules, the build rules, the advisory) runs whole, so the incremental stage's diagnostics are the full stage's, ordered and deduplicated as one pass leaves them; the laws stage stays whole. Reuse is opt-in and only the LSP opts in (`State::typed`, one typed snapshot per seed, kept across a hole), so `hale check` runs the whole check. The check records what it typed as it walks, so a typing that reused a declaration holds no record of its bodies, and the typed-body table of such a snapshot (`demand_typed_bodies`) is packaged from a whole check; the editor never lowers
- diagnostic spellings use one indexed rename table (`stdlib_bodies::Demangler`) per snapshot (`Snapshot::demangler`, F.40 phase 3, X3); import rows occur once per alias and declaration in a stable load order, and the indexed demangler preserves longest-name-first row application, including overlapping names, alias precedence and replacements that create a later row's name. The editor reuses this index for typing and final diagnostics; flow displays, model absorption displays and certificate evidence use the same demangler
- the bundle is a borrowed view (`Snapshot::bundle`), built per call, never stored
- a declaration's dependents are the families' (F.40 phase 3, X2, `Snapshot::declaration_dependents`, `hale_frontend::dependents`), never the text's: the unit is a top-level declaration of a snapshot program (`Snapshot::declarations`, a module one, named by its own minted site); for a fn or a locus the answer is itself, every declaration whose resolved calls reach it (the callgraph whose targets the effect rows copy, read from the allocation summary's call edges: the rows' and the declaration bodies', which are every body the reveal rule reads that is no row, each its declaration's own; a call to a fn with no row by its bare name; its readers closed transitively), and its neighbours in the ownership graph (births, accepts, instantiations), the bus graph (one subject's publishers, subscribers and topic), the placement table (what an instance or a dynamic site realizes, where its literal sits, its owner's and its enclosing scope's, joined by site) and the flow rows (a flow child and its `release` owners); a name joins every declaration that declares it, so a shared name answers both; a placement hole that leaves its literal's declaration unresolved makes the declaration it sits in a dependent of every one, and so does a call the summary records no edge for (by its span, over the site walk: a call in a position no summary body covers, today a closure's clauses and a type's field defaults, neither of which the reveal rule reads); any other declaration, one with no site, and a site that names none is `Dependents::Whole`. The relation's domain is an edit to a declaration's bodies: over the example corpus, every body edit of a mutation (emptied, an ill-typed `let`, a print, a call added) re-derives typing diagnostics only in declarations the relation names before the edit or after it, and an edit to a declared surface reaches a reader no family names, which is why such an edit is the seed's

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/demand_gate.rs (a_build_derives_the_scope_and_each_graph_once_lowering_included: the lowering view derives no scope, graph, handler rows, flow rows or arrangement of its own, F.40 phase 3, C5 and phase 4, Q1; a_build_classifies_the_scratch_local_fns_once; law_selection_runs_once_per_snapshot_on_every_verb_path, phase 4, A2); crates/hale-types/tests/declaration_dependents.rs (a_declarations_dependents_cover_what_a_fresh_check_rederives, an_edit_to_a_declared_surface_escapes_the_families); crates/hale-cli/tests/lsp.rs (the_check_is_its_typing_stage_followed_by_its_laws_stage, lsp_and_check_agree_over_a_seed_with_laws, lsp_publishes_the_typing_stage_first_and_the_laws_replace_it, the_incremental_typing_stage_is_the_full_one, the_incremental_typing_stage_is_the_full_one_over_the_dna_host, the_incremental_typing_stage_is_the_full_one_through_handler_and_initializer_calls, lsp_publishes_what_check_reports_after_a_helper_edit_through_handler_and_initializer_calls); crates/hale-cli/tests/lsp_latency.rs (lsp_latency_first_and_final_publication: the editor's latency, first and final publication apart; a measurement, ignored by default); crates/hale-frontend/src/snapshot.rs (a_changed_input_is_a_distinct_snapshot_and_shares_no_result, a_file_that_does_not_parse_blocks_the_scope_and_its_dependents, the_editor_scope_covers_the_members_that_parsed_and_names_the_hole, a_member_that_will_not_read_blocks_the_scope, a_program_that_does_not_typecheck_blocks_the_model, a_check_with_errors_blocks_the_lowering_view, the_lowering_view_is_resolved_once_after_the_check); crates/hale-lsp/src/lib.rs (a_request_builds_only_the_families_it_reads, a_request_over_a_seed_with_a_hole_answers_from_the_members_that_parsed, the_laws_replace_the_typing_stage_unless_a_newer_event_supersedes_them, a_pass_reuses_the_seeds_last_typed_snapshot)

**Spec.** spec/decisions.md F.40; RFC #1212 § phase 2 (demand and readiness)

**Guarded seams.**

- `Snapshot::load(` may be referenced from: `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1, `crates/hale-cli/src/verbs/build.rs` ×1, `crates/hale-cli/src/verbs/run.rs` ×1, `crates/hale-cli/src/verbs/test.rs` ×1, `crates/hale-cli/src/verbs/replay.rs` ×1, `crates/hale-cli/src/verbs/bench.rs` ×1, `crates/hale-lsp/src/lib.rs` ×1
- `Snapshot::from_program(` may be referenced from: 

### `digests` — Migrating · digest

**Answers.** Every identity a build or an artifact carries, and what each covers: shape_hash, artifact_digest, model_hash, exec_digest, the toolchain and cache keys, source digests, and the snapshot key they were derived under.

**Inputs.** the model half; the artifact; sources; BuildOptions; compiler sources; the snapshot key

**Producer (today's authority, migrating).** `crates/hale-graph/src/identity.rs` · `IDENTITIES`

**Legacy producers (permitted until removal).**

- `crates/hale-cli/src/shared/options.rs` · `exec_digest` — the replay identity: HALE_TOOLCHAIN_SHA256 + version + options fingerprint + plan digest + sources; its logical source paths fall back to file names; build and run fingerprint `debug` differently, so a build's recording never replays. *Removed when:* one stated coverage, with tests that a covered change moves it.
- `crates/hale-cli/src/shared/stale.rs` · `compute_codegen_src_hash` — the stale-binary hash: codegen.rs, lotus_arena.c and every stdlib .hl seed, walked identically at build and run time through the shared walk. *Removed when:* one identity per snapshot; the stale check reads it.
- `crates/hale-frontend/src/snapshot.rs` · `b.sources` — per-file FNV digests, set by the snapshot, rooted at hale.toml for every load mode, the editor's and its requests' included. *Removed when:* one source map per snapshot.

**Also owned.** `crates/hale-types/src/topology_projection.rs` · `project_shape_hash`; `crates/hale-cli/src/shared/options.rs` · `model_identity`; `crates/hale-types/src/topology.rs` · `dump_topology_over`; `crates/hale-model/src/claim_ir.rs` · `semantic_digest`; `crates/hale-model/src/application.rs` · `analysis_coverage_digest`; `crates/hale-model/src/dispatch_plan.rs` · `digest`; `crates/hale-cli/build.rs` · `toolchain_digest`; `crates/hale-iris/build.rs` · `main`; `crates/hale-iris/src/lib.rs` · `toolchain_hash`; `crates/hale-dna/src/digest.rs` · `digest_of_pairs`; `crates/hale-types/src/evidence.rs` · `analysis_inputs_digest`; `crates/hale-frontend/src/frontend.rs` · `source_map`; `crates/hale-model/src/obs_ids.rs` · `digest`; `crates/hale-frontend/src/snapshot.rs` · `SnapshotKey`; `crates/hale-frontend/src/snapshot.rs` · `digest`; `crates/hale-frontend/src/source.rs` · `overlay_digest`; `crates/hale-frontend/src/snapshot.rs` · `load`; `crates/hale-types/src/claims.rs` · `constitution_digest`; `crates/hale-cli/src/fleet.rs` · `fnv`; `crates/hale-codegen/src/codegen.rs` · `compile_cached_runtime_object_with`

**Inventory of identities** (`crates/hale-graph/src/identity.rs` · `IDENTITIES`). One row per value compared for equality as an identity. *Leaves out* names an input a reader would expect, with the stated reason; a reason that begins "a defect" or "a gap" names the step that closes it.

| identity | identifies | computed | fold | covers | leaves out | producer | on mismatch | versioned by | frozen |
|---|---|---|---|---|---|---|---|---|---|
| `shape_hash` | two programs have the same structural model: the hashed half of the topology artifact, rendered from the model alone | artifact emission (`hale check --dump-topology`) and every build (`model_identity`) | Fnv64 | ModelHalf | SourcePaths: by design: a structural identity, so a moved comment or a renamed file does not churn it; UserSources: by design: it names what the program is, not the text it was written in; LawRows: by design: the law rows are a section of their own, under `law_digest` | `crates/hale-types/src/topology_projection.rs` · `project_shape_hash` | the artifact is refused (`verify_shape_hash`), a replay is refused, a baseline gate fails | the artifact schema (`TOPOLOGY_SCHEMA`, 1.19) | the 1,793 pinned values of `topology_projection`, the nine of `model_arrangement`, and DNA, which requires schema 1.19 and recomputes it |
| `model_hash` | a binary was built from a model with this `shape_hash`: the same value, stamped into the recording and observation headers | hale build, run, replay (`model_identity`, from the snapshot's model) | Fnv64 | ModelHalf | BuildOptions: by design: the model is not the build; `exec_digest` carries the options | `crates/hale-cli/src/shared/options.rs` · `model_identity` | replay is refused | moves with `shape_hash`: the artifact schema | the iris protocol's header layout |
| `artifact_digest` | the whole topology artifact, results and provenance included, is the one a producer emitted: an unkeyed tripwire, not a signature | artifact emission | Fnv64 | ModelHalf, Model, LawRows, UserSources, SourcePaths, AnalysisVersion | CompilerSources: by design: it is a checksum of the document; the compiler that wrote it is named by nothing in it | `crates/hale-types/src/topology.rs` · `dump_topology_over` | the artifact is refused | the artifact schema; a section moved by I3 changes values and never the section's shape | DNA requires schema 1.19 exactly and recomputes it |
| `law_digest` | the artifact's law rows and issues are the ones emitted: a fold of their canonical JSON | artifact emission | Fnv64 | LawRows | — | `crates/hale-types/src/topology.rs` · `dump_topology_over` | the artifact is refused: a row edited under a stale digest | the artifact schema | DNA reads the law section under schema 1.19 |
| `claim_table_digest` | an evidence sidecar was derived from this claim table: its rows, origins, laws and provenance store | evidence derivation (`ClaimIrTable::semantic_digest`) | Fnv64 | LawRows | ModelHalf: by design: the sidecar names the model by `model_shape`; two programs of one topology with different `@effects` classes must not accept each other's evidence, which is why the law table is hashed beside it | `crates/hale-model/src/claim_ir.rs` · `semantic_digest` | the evidence is refused (`InvalidProvenanceRecord`) | the crate's own FNV hasher, never `DefaultHasher`; a change of what the table holds moves it | — |
| `analysis_coverage_digest` | an evidence sidecar was derived over a model whose analysis coverage (the analyzable, analyzed, summarized and ownership bits) is this one | evidence derivation | Fnv64 | Model | SourcePaths: by design: coverage bits per entity, by canonical entity order | `crates/hale-model/src/application.rs` · `analysis_coverage_digest` | the evidence is refused (`InvalidProvenanceRecord`) | a change of the bits it folds moves it; `ANALYSIS_SEMANTICS_VERSION` covers the analyses' meaning | — |
| `dispatch_plan_digest` | two builds lowered the bus with the same dispatch plan: each subject, its flavor and its subscribers | hale build, run, replay (the lowering view's plan; the default plan under `no_bus_devirt`) | Fnv64 | DispatchPlan | DispatchPlan: the same-domain column is a reserved zero byte that no lowering reads; GH #464 makes it a column again | `crates/hale-model/src/dispatch_plan.rs` · `digest` | none of its own: it moves `exec_digest`, so a replay is refused | a change of what lowering reads moves it, and with it every recording's `exec_digest` | — |
| `exec_digest` | a build and a recording were made from the same build inputs: the toolchain, the options, the plan and the user's sources | hale build, run, replay | Sha256 | CompilerSources, RuntimeC, StdlibSeeds, Manifests, RustcVersion, GitCommit, CompilerVersion, BuildOptions, DispatchPlan, UserSources, SourcePaths | BuildOptions: a defect, corrected in I2: `debug` is fingerprinted as set by `hale build` and never by `run` or `replay`, so a built binary's recording never replays; the `[ffi]` pickup happens in `build` only, and `replay` accepts `--env` and never resolves it; SourcePaths: a defect, corrected in I3: paths are framed relative to the entry's parent and fall back to the bare file name, so a directory build's recording never matches its file and two imports of one file name share an identity; the frame follows absolute paths' order | `crates/hale-cli/src/shared/options.rs` · `exec_digest` | replay refused | moves with every compiler commit (the toolchain half frames the commit) | the default options fingerprint string (`build_env.rs`'s test) |
| `toolchain_digest` | the replay identity's compiler half: a recording was made by a compiler built from these sources, by this rustc, at this commit | compiler build (`HALE_TOOLCHAIN_SHA256`, hale-cli/build.rs) | Sha256 | CompilerSources, RuntimeC, StdlibSeeds, Manifests, RustcVersion, GitCommit | UserSources: by design: the toolchain is not the program; `exec_digest` frames the sources beside it; BuildOptions: by design: options are a build's, `exec_digest` frames them | `crates/hale-cli/build.rs` · `toolchain_digest` | replay refused (through `exec_digest`) | moves with every compiler commit; a framing of the one shared selection (`identity_files`) | — |
| `codegen_src_hash` | the stale-binary warning's hash: the binary was built from the codegen, runtime and stdlib files now on disk | compiler build (`HALE_CODEGEN_SRC_HASH`, hale-cli/build.rs) and every check, verify, build, run, test, dna and inputs invocation in a development checkout (`compute_codegen_src_hash`) | DefaultHasher | CompilerSources, RuntimeC, StdlibSeeds | CompilerSources: a gap, closed in I4: it folds one of codegen's source files (`codegen.rs`) and none of the other covered crates'; RuntimeC: a gap, closed in I4: it folds one of the runtime's C files (`lotus_arena.c`), no other C file or header; Manifests: a gap, closed in I4 | `crates/hale-cli/src/shared/stale.rs` · `compute_codegen_src_hash` | a warning on stderr | none: its selection is `stale_hash_paths`; a change of coverage is a change of that function | — |
| `compiler_src_hash` | the DNA host cache's compiler half: the same file selection as `toolchain_digest`, folded without the rustc version or the commit, because the cache is keyed by what this binary would build | compiler build (`HALE_COMPILER_SRC_HASH`, hale-iris/build.rs) | Fnv64 | CompilerSources, RuntimeC, StdlibSeeds, Manifests | RustcVersion: by design: two binaries of one source build one host; GitCommit: by design: the commit names no source the files do not; BuildOptions: a gap, closed in I5: the build knobs the cache's `hale build` subprocess inherits from the environment (`HALE_DEV`, `LOTUS_NO_DEBUGINFO`, a sanitizer) are in no key | `crates/hale-iris/build.rs` · `main` | the cache directory is new; the host is rebuilt | moves with every compiler source edit; the shared selection (`identity_files`) | — |
| `toolchain_hash` | the DNA host cache's key: the compiler's sources, the stdlib and the embedded iris and DNA trees are the ones the cached host was built from | hale iris, hale dna (per invocation) | Fnv64 | CompilerVersion, CompilerSources, RuntimeC, StdlibSeeds, Manifests, EmbeddedDna, EmbeddedIris | BuildOptions: a gap, closed in I5: the environment's build knobs are in no key, so a host cached under a sanitizer or `HALE_DEV` is served to a run without it; StdlibSeeds: a gap, closed in I5: the stdlib is folded twice, once in `compiler_src_hash` and once as the embedded `AP_FILES`; RustcVersion: by design: see `compiler_src_hash` | `crates/hale-iris/src/lib.rs` · `toolchain_hash` | the cache directory is new; the host is rebuilt | moves with the version, the compiler's sources and every embedded byte | — |
| `embedded_dna_digest` | a binary embeds this DNA source set: the files of the listed directories, by path and content | compiler build (`HALE_DNA_EMBEDDED_DIGEST`, hale-dna/build.rs), and over any checkout's tree (`digest_of_tree`) | Sha256 | EmbeddedDna | EmbeddedDna: by design: the directories are listed non-recursively, with the extensions each contributes (`EMBEDDED_DIRS`); a file outside them is not embedded | `crates/hale-dna/src/digest.rs` · `digest_of_pairs` | a warning on stderr (the stale-DNA check); `hale dna status` reports the difference | a framing tag (`hale-dna-embedded-v1`) | persisted in users' trees (`vendor/dna` provenance): its value never moves with a refactor |
| `analysis_inputs_digest` | evidence was produced by the same analyses: the stdlib source they absorb, the compiler version, the path renames and the surface registry; independent of the program by design | artifact emission and admission (recomputed by the binary that reads it) | Fnv64 | AnalysisVersion, StdlibSeeds, CompilerVersion, PathRenames, SurfaceRegistry | UserSources: by design: it names the analyses, never the program; CompilerSources: by design: the analyses' Rust is versioned by `ANALYSIS_SEMANTICS_VERSION`, bumped by hand when a certificate's meaning moves | `crates/hale-types/src/evidence.rs` · `analysis_inputs_digest` | the artifact is refused: evidence produced under another analysis snapshot | a hand-bumped constant (`ANALYSIS_SEMANTICS_VERSION`, 7), and the compiler version | — |
| `source_digest` | a source file has this text: the per-file digest of the snapshot's source map | snapshot load (`source_map`, hale-frontend) | Fnv64 | UserSources | SourcePaths: the path rides beside it in the source map, not in the digest; SourcePaths: a defect, corrected in I3: a file outside the workspace root keeps its absolute path, so an artifact is specific to the machine that built it | `crates/hale-frontend/src/frontend.rs` · `source_map` | the evidence is refused; the artifact's digest differs | the artifact schema | the `sources` section's shape (schema 1.19) |
| `obs_entity_id_digest` | two observed id tables number the entities identically: the `(kind, name, id)` rows a build stamped, which an external consumer recomputes from its own copy of the model | hale build (codegen stamps it into the obs header), and by a consumer from its model | Fnv64 | Model | SourcePaths: by design: keyed by (kind, name), because no artifact carries a site identity; keying it by site would break the consumer and churn on unrelated edits | `crates/hale-model/src/obs_ids.rs` · `digest` | no match, no join: a consumer refuses the ids | the iris protocol's version (0.3) | nine literal values in `model_arrangement`; `PROTOCOL.md` §3.2 |
| `snapshot_key` | two snapshots were loaded from the same inputs and share a result: the entry, the load mode, the target and the three component digests | snapshot load | Structural | SourcePaths, Target, SnapshotConfig, EditorBuffers, UserSources | CompilerSources: by design: the key is snapshot-local; no binary or recording carries it; BuildOptions: by design: codegen options are not a load's | `crates/hale-frontend/src/snapshot.rs` · `SnapshotKey` | the snapshots are distinct and share no result | its fields: a component added is a field added | — |
| `snapshot_config_digest` | the load's configuration is the same: the target and its spec, the api and roles, the environment and the check's switches | snapshot load (`Config::digest`) | Fnv64 | Target, EnvironmentRoles, SnapshotConfig | — | `crates/hale-frontend/src/snapshot.rs` · `digest` | the snapshots are distinct | length-framed fields: a switch added is a field added | — |
| `snapshot_overlay_digest` | the editor buffers a load read over the disk are the same | snapshot load (`SourceProvider::overlay_digest`) | Fnv64 | EditorBuffers | — | `crates/hale-frontend/src/source.rs` · `overlay_digest` | the snapshots are distinct | zero for the disk itself; length-framed fields | — |
| `snapshot_sources_digest` | a load read the same source text: every unit's path and text, imports included | snapshot load | Fnv64 | UserSources, SourcePaths | SourcePaths: a defect, corrected in I3: the path is hashed absolute, so one program checked out at two roots has two keys | `crates/hale-frontend/src/snapshot.rs` · `load` | the snapshots are distinct | none: the order is the load's | — |
| `constitution_digest` | two constitutions have the same normalized closure: their bases (deduplicated) and entries, rendered as forms | law selection, once per snapshot, projected for the artifact and the matrix (`LawSelection::identities`) | Fnv64 | Constitutions | SourcePaths: by design: the closure's text, not where it was declared | `crates/hale-types/src/claims.rs` · `constitution_digest` | the matrix reports two environments that adopt different closures | none: a change of the rendering moves it | — |
| `fleet_shape_hash` | a fleet plan's model text is the one printed: the plan artifact's own shape | fleet plan emission | Fnv64 | FleetPlan | — | `crates/hale-cli/src/fleet.rs` · `fnv` | none: nothing in the workspace compares it, it is printed | none | — |
| `runtime_object_key` | a cached runtime object was compiled from this C source with these flags by this C compiler | hale build (`compile_cached_runtime_object_with`) | DefaultHasher | RuntimeC, CCompiler | CCompiler: the host `clang` hashes as it always has, so a cached object stays valid; only another compiler or another release moves the key | `crates/hale-codegen/src/codegen.rs` · `compile_cached_runtime_object_with` | the object is recompiled | a hand-bumped constant (`RT_CACHE_VERSION`) | — |

**Consumers.** replay (admission); topology / fleet (admission); dna (schema 1.19, semantics 2, shape_hash, artifact_digest); the runtime obs header; the DNA host cache

**Invariants.**

- external contracts are frozen through extraction: additive and unhashed sections are free; hash and replay identity change only through explicit versioned transitions with an exact diagnostic (#476's rule)
- a build's identities read one snapshot (2.3, `model_identity`): the model hash (P26) is the snapshot model's `shape_hash`, read from the model (`project_shape_hash`, the value its artifact stamps, never scraped from a rendered artifact), the obs ids are that model's entities, and the plan digest `exec_digest` frames is its lowering view's plan; beside them the snapshot key (`SnapshotKey`: the entry, the load mode, the target, the config digest, the overlay digest, the digest of the source text read) names the load all three were derived from. The key is snapshot-local: no binary or recording carries it
- a semantic producer moving between crates never makes a later edit invisible to cache or replay identity: the replay identity and the cache key fold one selection, every identity-covered crate (the CLI among them until hale-frontend owns its semantic work) and the manifest files; the stale-binary hash is a cheap warning over codegen.rs, the runtime and the stdlib seeds by design

**Missing data.** n/a

**Focused tests.** crates/hale-cli/tests/obs_model_hash.rs; crates/hale-cli/tests/model_diff.rs; crates/hale-cli/tests/replay_cli.rs; crates/hale-cli/tests/stale_dna_warning.rs; crates/hale-cli/tests/source_map.rs

**Spec.** spec/model.md § Identity and versioning

## Spec rules and their evaluators

A registered rule without an evaluator fails the compiler's own build, and `registry_rules_match_spec.rs` reads each list below from the spec and fails on a rule one side lacks or a title that differs. `reads` is what the evaluator reads: the rows of the named families, or the declaration it judges.

| list | rules |
|---|---|
| `spec/semantics.md` § Type-check rules | 20 |
| `spec/semantics.md` § Slot restrictions (v1) | 3 |
| `spec/verification.md` § Structural & design rules | 7 |

| rule | list | title | gist | family | evaluator | reads | state |
|---|---|---|---|---|---|---|---|
| semantics/placement/1 | `spec/semantics.md` § Type-check rules | `placement { }` is `main locus` only. | `placement { }` is main-locus-only | `placement` | `crates/hale-syntax/src/parser.rs` · ``placement` block is only valid inside` | the declaration | Canonical |
| semantics/placement/2 | `spec/semantics.md` § Type-check rules | Keys reference main-locus `params` field names. | keys name main-locus params fields | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | `top_scope` | Canonical |
| semantics/placement/3 | `spec/semantics.md` § Type-check rules | Field values are locus types. | field values are locus types | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | `top_scope` | Canonical |
| semantics/placement/4 | `spec/semantics.md` § Type-check rules | At most one placement entry per field. | at most one entry per field | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | the declaration | Canonical |
| semantics/placement/5 | `spec/semantics.md` § Type-check rules | Pool names use snake_case Idents. | pool names are identifiers; `main` always exists | `placement` | `crates/hale-syntax/src/parser.rs` · `parse_placement_block` | the declaration | Canonical |
| semantics/placement/6 | `spec/semantics.md` § Type-check rules | Locus-pinning compatibility. | pinned-class restrictions (no accept(), no closure whose epoch is birth or dissolve, the default) on every pinned instance, a placement entry's or an adapter binding's | `placement` | `crates/hale-types/src/lowering_laws.rs` · `pinned_features` | `placement` | Canonical |
| semantics/placement/7 | `spec/semantics.md` § Type-check rules | Dead bus receiver (error). | dead bus receiver on a cooperative pool is an error | `blocking` | `crates/hale-types/src/check.rs` · `check_cooperative_pool_blocking` | `placement`, `bus_graph`, `effects` | Migrating |
| semantics/placement/8 | `spec/semantics.md` § Type-check rules | Blocking syscall on a cooperative pool (warning). | a blocking syscall on a cooperative pool is a warning | `blocking` | `crates/hale-types/src/check.rs` · `check_cooperative_pool_blocking` | `placement`, `bus_graph`, `effects` | Migrating |
| semantics/placement/9 | `spec/semantics.md` § Type-check rules | Orphan bus topic (warning). | orphan bus topic (closed world) | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_graph` | `bus_graph`, `entrypoint`, `top_scope` | Migrating |
| semantics/placement/10 | `spec/semantics.md` § Type-check rules | Bus cycles. | bus cycles: a queued cycle warns, an unconditional intra-locus cycle of direct calls is an error | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_cycles` | `bus_graph`, `desugar_sequence` | Migrating |
| semantics/placement/11 | `spec/semantics.md` § Type-check rules | Bus backpressure (warning). | bus backpressure heuristic | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_backpressure` | the declaration | Migrating |
| semantics/placement/12 | `spec/semantics.md` § Type-check rules | Bus subject type-mismatch (error). | one literal subject, one payload type | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_subject_types` | the declaration | Migrating |
| semantics/placement/13 | `spec/semantics.md` § Type-check rules | Empty / degenerate `pinned(cores = …)` (error). | degenerate `pinned(cores = ..)` is an error | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | the declaration | Canonical |
| semantics/placement/14 | `spec/semantics.md` § Type-check rules | `topology { }` consistency + `pinned(node/l3)` resolution (error). | topology consistency and node/l3 resolution | `placement` | `crates/hale-types/src/check.rs` · `check_topology_block` | the declaration | Canonical |
| semantics/placement/15 | `spec/semantics.md` § Type-check rules | `replicas = K` (error on `K < 1`; pinned-only). | `replicas = K`: K >= 1, pinned only | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | the declaration | Canonical |
| semantics/placement/16 | `spec/semantics.md` § Type-check rules | Pool affinity (2026-08-12). | pool affinity agrees per pool | `placement` | `crates/hale-types/src/check.rs` · `check_pool_affinity` | `entrypoint` | Migrating |
| semantics/placement/17 | `spec/semantics.md` § Type-check rules | A `pinned` placement forbids a loop (error). | a pinned locus is not instantiated in a loop | `placement` | `crates/hale-types/src/lowering_laws.rs` · `pinned_root_in_a_loop` | `placement` | Canonical |
| semantics/placement/18 | `spec/semantics.md` § Type-check rules | Every entry is consumed by exactly one instantiation (error). | every placement entry is consumed exactly once | `placement` | `crates/hale-types/src/lowering_laws.rs` · `placement_entry_consumed` | `placement` | Canonical |
| semantics/placement/19 | `spec/semantics.md` § Type-check rules | Uncarriable bus payload (error). | a bus payload is carriable | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_payload_carriable` | `top_scope` | Migrating |
| semantics/placement/20 | `spec/semantics.md` § Type-check rules | Unowned subscriber (error). | a subscriber born in a bus handler is owned | `ownership` | `crates/hale-types/src/check.rs` · `check_unowned_subscriber_locus` | `ownership`, `placement` | Canonical |
| semantics/slots/1 | `spec/semantics.md` § Slot restrictions (v1) | Slot element type must be a value-shape, not a LocusRef. | a capacity slot's cell type is not a locus: a span-targeted typecheck error, and again at codegen (`locus/decl.rs`) as defense in depth | `forms` | `crates/hale-types/src/check.rs` · `check_locus_member_at` | `top_scope` | Canonical |
| semantics/slots/2 | `spec/semantics.md` § Slot restrictions (v1) | Slot pointers don't cross the bus. | structurally enforced, no check: a slot name is a locus member, not a typeable identifier (`self.<slot>` is typed `Unknown`, `check_expr_at`), so it cannot appear as a payload struct field | `forms` | `crates/hale-types/src/check.rs` · `check_expr_at` | the declaration | Canonical |
| semantics/slots/3 | `spec/semantics.md` § Slot restrictions (v1) | Duplicate slot names rejected. | two slots of one name are a typecheck error, and again at codegen (`locus/decl.rs`) | `forms` | `crates/hale-types/src/check.rs` · `check_locus_member_at` | the declaration | Canonical |
| verification/structural/cqrs-no-locus-return | `spec/verification.md` § Structural & design rules | CQRS / no-locus-return | a locus `fn` whose return type or `fallible(T)` payload names a user-declared locus (error) | `surfaces` | `crates/hale-types/src/check.rs` · `check_no_locus_return` | `top_scope` | Canonical |
| verification/structural/stdlib-error-type-shadow | `spec/verification.md` § Structural & design rules | Stdlib error-type shadow | a user `type` named like a stdlib error type whose shape differs, when a fallible stdlib call reaches that error type (error) | `stdlib_surface` | `crates/hale-types/src/resolve.rs` · `check_stdlib_error_shadowing` | `top_scope` | Migrating |
| verification/structural/codec-purity | `spec/verification.md` § Structural & design rules | Codec purity | a bus codec whose `encode` / `decode` is not pure (error), read from the purity column of the effect rows | `bindings` | `crates/hale-types/src/check.rs` · `check_main_and_bindings` | `effects` | Canonical |
| verification/structural/ring-layout-contract | `spec/verification.md` § Structural & design rules | `ring_layout` contract | a foreign-ring layout declaration that is internally ill-formed (error); `check_ring_layout`, and `check_main_and_bindings` for a binding's `layout:` reference | `bindings` | `crates/hale-types/src/check.rs` · `check_ring_layout` | the declaration | Canonical |
| verification/structural/ring-layout-geometry | `spec/verification.md` § Structural & design rules | `ring_layout` geometry | a cross-field inconsistency in a `ring_layout` (overlap, overrun, a `buffer_size` that is not a multiple of the record alignment) (error); `check_ring_layout` and `check_main_and_bindings` | `bindings` | `crates/hale-types/src/check.rs` · `check_ring_layout` | the declaration | Canonical |
| verification/structural/foreign-ring-payload-shape | `spec/verification.md` § Structural & design rules | Foreign-ring payload shape | a `layout:`-bound topic whose payload is neither flat-shapeable nor `BytesView` (error) | `bindings` | `crates/hale-types/src/check.rs` · `check_main_and_bindings` | `top_scope` | Canonical |
| verification/structural/cell-slot-of-origin | `spec/verification.md` § Structural & design rules | Cell slot-of-origin | releasing a `Cell<T>` into a different `(locus, slot)` than it was acquired from (error, at codegen) | `forms` | `crates/hale-codegen/src/codegen.rs` · `try_lower_capacity_slot_method_call` | the declaration | Canonical |

## The shadow facility's allowance

The shadow facility (`hale-graph`'s `shadow` module) runs a new derivation beside the old one while a family migrates. Its call sites outside tests are this allowance only: a source file under `crates/*/src` that references the facility, other than the facility's own module, is listed here with its count, or fails `registry_guard.rs`. None today: F.40 phase 3 deleted every shadow, and the facility's users are tests.

## Frozen Debug renderings

Every Debug rendering with no prose around it (a `?}` placeholder in a formatting macro whose template holds no space) in `hale-syntax`, `hale-types`, `hale-model`, `hale-codegen`, `hale-frontend`, `hale-cli` and `hale-lsp`, with the number of invocations that collapse to the fragment. A message with prose around its `{:?}` is not listed: it is read by a person. A site that *decides* derives a fact from a Debug string and is permitted only until its family's table replaces it; a new site fails the guard.

| path | invocation | count | verdict |
|---|---|---|---|
| `crates/hale-cli/src/build_env.rs` | `format!( "target={:?};cpu={:?};dev={};debug={}", o.target, o.target_cpu, o.dev_profile, o.` | 1 | decides (`digests`) |
| `crates/hale-cli/src/build_env.rs` | `format!(";lto={l:?}")` | 1 | decides (`digests`) |
| `crates/hale-cli/src/verbs/misc.rs` | `println!("{:#?}", prog)` | 1 | renders |
| `crates/hale-codegen/src/codegen.rs` | `format!("{:?}", other)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("pinned({:?})", affinity)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", ExitCode::SUCCESS)` | 2 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", affinity)` | 1 | decides (`placement`) |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", c.kind)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", p)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", r)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", s.placement)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", site.escape)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", site.kind)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{:?}", site.reason)` | 1 | renders |
| `crates/hale-lsp/src/lib.rs` | `format!("{code:?}")` | 1 | renders |
| `crates/hale-syntax/src/json_gen.rs` | `format!("{:?}", f)` | 1 | renders |
| `crates/hale-syntax/src/parser.rs` | `format!("{:?}", err)` | 21 | renders |
| `crates/hale-types/src/check.rs` | `format!("{:?}", p)` | 2 | decides (`snapshot_identity`) |
| `crates/hale-types/src/check.rs` | `format!("{:?}({})", class, type_expr_text(inner))` | 1 | renders |
| `crates/hale-types/src/check.rs` | `format!("{:?}({})", class, type_expr_identity(inner, known))` | 1 | decides (`snapshot_identity`) |
| `crates/hale-types/src/lib.rs` | `format!("{:?}", d.kind)` | 1 | renders |
| `crates/hale-types/src/model_builder.rs` | `format!("{:?}:{}", d.kind, d.display)` | 1 | renders |
| `crates/hale-types/src/model_builder.rs` | `format!( "projection:{:?}({})", class, type_descriptor(inner) )` | 1 | decides (`snapshot_identity`) |
| `crates/hale-types/src/desugar_sequence.rs` | `format!("{:?}", d)` | 1 | renders |
| `crates/hale-types/src/purity.rs` | `format!("{:?}", op)` | 1 | renders |
| `crates/hale-types/src/purity.rs` | `format!("{:?}", subject)` | 1 | renders |
| `crates/hale-types/src/secret_reveal.rs` | `format!("{:?}", fd)` | 1 | decides (`effects`) |
| `crates/hale-types/src/secret_reveal.rs` | `format!("{:?}", lc.kind)` | 1 | decides (`effects`) |
| `crates/hale-types/src/secret_reveal.rs` | `format!("{:?}", m)` | 1 | decides (`effects`) |
| `crates/hale-types/src/secret_reveal.rs` | `format!("{:?}", other)` | 3 | renders |
| `crates/hale-types/src/stdlib_names.rs` | `format!("{:?}", d)` | 1 | decides (`stdlib_surface`) |
