# The graph registry

GENERATED from `crates/hale-graph/src/registry.rs` and held byte-equal by `registry_matches_spec`. Do not edit: change the table and run `HALE_REGEN_REGISTRY=1 cargo test -p hale-graph --test registry_matches_spec`. The contract this index serves is `spec/model.md` § *The graph registry*.

The families, their legacy producers, the spec rules and the frozen Debug-string sites are counted by `registry_is_well_formed`, not here: a rendered total moved with every change to any family, so two unrelated changes conflicted on one line at every rebase.

## Families

| family | layer | state | kind | producer | legacy | answers |
|---|---|---|---|---|---|---|
| `seed_loading` | Layer 1 | Canonical | desugar | `collect_checkable` | 0 | Which source units form the snapshot: the entry, every imported seed, their merge order and the spans' virtual bases. |
| `qualified_names` | Layer 1 | Canonical | desugar | `resolve_imports` | 0 | What a qualified or aliased name denotes: the library identity, the mangled declaration, the construction target, the bus subject a path names. |
| `desugar_sequence` | Layer 1 | Migrating | desugar | `desugar_before_check` | 1 | Which rewrites the program receives before checking, in which order: the declaration-shaping passes only (JSON parsers, the api surface, unit returns, construction aliases, qualified bus subjects, the omitted `run`, repr accessors). Sync inference is not a rewrite: its pick is a form row (`sync_inference`). The topic-reference and intra-locus rewrites are not desugars: they erase a written declaration reference the checker's laws and the model read, and run in lowering's resolved program, after the check. |
| `sync_inference` | Layer 1 | Migrating | derivation | `form_rows` | 1 | Which sync discipline each `@form` declaration gets: one row per declaration with the author's configuration (omitted, a written discipline, `none` included, or an argument naming none) and the effective discipline, inference's pick for a `hashmap` form left unconfigured, from the domains each of its instances is called from; two queries, explicitly configured and safe for cross-domain access. |
| `effect_class_table` | Layer 1 | Canonical | derivation | `EffectClasses` | 0 | The user effect classes of a load: one table every seed is parsed through, so a class (its name, its identity in the program's one class namespace) has one `User(i)` index in every seed; which were declared, which are composed, and the one expansion of a composed class. |
| `top_scope` | Layer 2 | Migrating | derivation | `build_top_scope` | 1 | What every top-level name denotes: the symbol table over the merged program. |
| `expression_typing` | Layer 2 | Canonical | derivation | `check_bundle_scoped` | 0 | The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from. |
| `generics` | Layer 2 | Migrating | derivation | `unify_generic_ty` | 2 | Which monomorph a generic call instantiates and how its bindings unify. |
| `surfaces` | Layer 2 | Canonical | law | `conformance_witness` | 0 | Which surface is visible at which depth edge: contract exposure, interface conformance, perspective designation and `serves` conformance. |
| `forms` | Layer 2 | Migrating | law | `check_form_shape` | 1 | Whether a form's shape, its capacity slots and its projection class are well formed, and which operation set closes each slot. |
| `stdlib_surface` | Layer 2 | Migrating | capability | `signature_for` | 6 | What each stdlib function is: its signature, its effect classes, whether it blocks, and what a value of a type can be rendered as. |
| `entrypoint` | Layer 3 | Migrating | derivation | `entry_row` | 31 | Which locus is the program's `main`, whether the world is closed, and which declarations are imported. |
| `ownership` | Layer 3 | Migrating | derivation | `resolve_owners` | 4 | Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array. |
| `bus_graph` | Layer 3 | Migrating | derivation | `build_bus_graph` | 2 | The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates. |
| `topics` | Layer 3 | Canonical | derivation | `topic_wire_subjects` | 0 | What each topic is on the wire: its subject, payload contract, routing key, bounds and shed policy; and which topic a send's subject names. |
| `bindings` | Layer 3 | Canonical | derivation | `derive_binding_rows` | 0 | Which topics are bound to which transport, in which role, with which codec, and whether the transport can carry the payload. |
| `dispatch` | Layer 3 | Migrating | derivation | `fn derive` | 2 | How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement. |
| `handler_routing` | Layer 3 | Migrating | derivation | `handler_rows` | 1 | Which `on_failure` handler a failing child's locus type reaches, and from which parent. |
| `flows` | Layer 3 | Canonical | derivation | `survey` | 0 | Which children are flows (released per completion) and which are resident. |
| `restart` | Layer 3 | Migrating | derivation | `handler_rows` | 2 | Which loci declare restart operations, which restart in place, and what the restart bound is. |
| `closures` | Layer 3 | Migrating | law | `check_locus_member` | 1 | Whether each closure clause is well formed, and which lifecycle events (`epoch`, `persists_through`, `resets_on`) it names. |
| `api_surface` | Layer 3 | Migrating | derivation | `api_surface` | 3 | The served surface: commands, reads, streams, their schemas, the roles that gate them, and the description's wire form. |
| `sealability` | Layer 3 | Migrating | law | `check_sealed_access` | 1 | Which loci confine their state (`@sealed`), and which could. |
| `runs_under` | Layer 3 | Reserved | derivation | — | 0 | On whose authority a locus runs: the relation `runs_under(locus, principal)`, with principals declared by the program. |
| `transitions` | Layer 3 | Reserved | derivation | — | 0 | For an evented locus: the transition each handler is, input event to output set (F.41, after phase 2). |
| `effects` | Layer 4 | Canonical | derivation | `derive_effect_rows` | 0 | Which effect classes each fn and locus reaches (the callgraph fixpoint), the declared classes and their `causes:`/`depends:` DAG, and the certificate relating the two. |
| `blocking` | Layer 4 | Migrating | derivation | `blocking_path_match` | 4 | Which fns block (a cooperative worker would be held), and whether the program places anything off the main thread. |
| `alloc_summary` | Layer 4 | Migrating | derivation | `derive_alloc_summary` | 1 | Where each allocation lands and when it is reclaimed: per-fn allocation, escape, scratch eligibility, method-scratch elision, stack arrays, arena elision. |
| `borrow_lifetime` | Layer 4 | Canonical | law | `borrow_lifetime_diags` | 0 | Whether a borrowed handle outlives its holder (GH #730), decided from position over the owner structure. |
| `bare_fallible` | Layer 4 | Migrating | law | `bare_fallible_calls` | 1 | Whether a fallible call's error is addressed. |
| `nonreturning` | Layer 4 | Migrating | law | `run_statically_nonreturning` | 2 | Which `run()` bodies never return, which children are long-running, and whether the birth order or a pool starves because of it. |
| `working_set` | Layer 4 | Canonical | derivation | `compute_program_working_set` | 0 | The estimated working set per locus and program, and the locality law over it. |
| `placement` | Layer 5 | Migrating | derivation | `derive_placement` | 7 | Which thread domain each instance runs in: pools, pinned threads, replicas, affinity, and the deployment plan. |
| `target_capability` | Layer 5 | Migrating | capability | `derive_capability_matrix` | 7 | What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability. |
| `deployment` | Layer 5 | Reserved | derivation | — | 0 | A deployment as typed rows: root and horizon, component identities, instances and incarnations, resources and allocations, endpoints and routes, hosting and authority, persistence obligations (the habitat, after phase 2). |
| `lifecycle_order` | Layer 6 | Migrating | derivation | `derive_lifecycle` | 9 | The happens-before order per instance: birth sequence, params open and settle, failure delivery and its execution domain, reclaim prerequisites, drain, restart, teardown. |
| `bus_inert` | Layer 6 | Canonical | derivation | `bus_inert` | 0 | Whether the program can ever have a bus cell in flight, so drains can be elided. |
| `law_backstops` | Layer 8 | Migrating | law | — | 1 | The checker rules lowering re-judges because `build_executable` never runs the checker: self-containment, cross-pool bare statements, placement entries, pinned loci in loops. |
| `model` | The law engine | Canonical | derivation | `derive_application_model_over` | 0 | The canonical semantic model of a checked bundle: fifteen entity tables, seventeen relation tables, holes, capabilities, provenance (GH #476). |
| `claims` | The law engine | Migrating | law | `claim_law_diags` | 3 | Every user law: lowered claim rows, the judged verdicts over the model and evidence, constitution identities, and the artifact's law account. |
| `view` | The law engine | Reserved | derivation | — | 0 | A named query over the tables: a node selector, a relation set and an adequacy policy, rendered by a backend (hale ui, after phase 2). |
| `snapshot_identity` | Identity | Migrating | derivation | `mint` | 3 | The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again in the resolved-program step (numbered over the user program before the intra-locus rewrite, so the sends it records are numbered on every path, and minted over the merged program with the bundle's seeds and a named seed for the bundled stdlib), idempotently (one numbering; a later mint numbers only what an earlier one did not see), with reliable provenance; and which declaration each use names (`binding_of`), resolved once by the mint. |
| `demand` | Identity | Canonical | derivation | `Snapshot` | 0 | Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, each member's own program beside the merged one, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph and handler rows, the effect rows, the model, the check in its two stages with the effects certificate report its typing produced, the lowering view), each at most once, blocking a family whose prerequisite reported errors. |
| `digests` | Identity | Migrating | digest | `model_shape_hash` | 9 | Every identity a build or an artifact carries, and what each covers: shape_hash, artifact_digest, model_hash, exec_digest, the toolchain and cache keys, source digests, and the snapshot key they were derived under. |

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

### `desugar_sequence` — Migrating · desugar

**Answers.** Which rewrites the program receives before checking, in which order: the declaration-shaping passes only (JSON parsers, the api surface, unit returns, construction aliases, qualified bus subjects, the omitted `run`, repr accessors). Sync inference is not a rewrite: its pick is a form row (`sync_inference`). The topic-reference and intra-locus rewrites are not desugars: they erase a written declaration reference the checker's laws and the model read, and run in lowering's resolved program, after the check.

**Inputs.** the merged program; --api / --env (roles); the cross-seed rename table

**Producer (today's authority, migrating).** `crates/hale-types/src/desugar_sequence.rs` · `desugar_before_check`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `build_executable_with_options` — codegen's adapter for a bare program is the test harness's snapshot, not a second pipeline: it builds `Snapshot::from_program` (shaped by the one load, with no source map and no check before lowering, `Config::harness`) and demands the lowering view; its seam allows only the definition, so no non-test caller bypasses the verbs' snapshot (tests are not scanned by the seam guard). *Removed when:* the harness builds from a loaded seed, as the verbs do.

**Also owned.** `crates/hale-types/src/resolved.rs` · `resolve_program`; `crates/hale-types/src/resolved.rs` · `rewrite_intra_locus`; `crates/hale-types/src/resolved.rs` · `resolve_rewritten`; `crates/hale-syntax/src/desugar.rs` · `desugar_intra_locus_topics`; `crates/hale-syntax/src/desugar.rs` · `desugar_topics`; `crates/hale-types/src/desugar_sequence.rs` · `bundled_stdlib`; `crates/hale-syntax/src/desugar.rs` · `desugar_omitted_run`; `crates/hale-syntax/src/desugar.rs` · `desugar_repr_accessors`

**Consumers.** check; build; run; test; replay; bench; lsp; codegen (`crates/hale-codegen/src/codegen.rs` · `build_resolved`)

**Invariants.**

- one order, run once per snapshot, before the first law is judged
- the sequence is called from the snapshot's load (`Snapshot::load`, `Snapshot::from_program`) and from `check_program` (the test entry), and from nowhere else: every entry point runs it before it mints its snapshot; the bundled stdlib goes through the same passes (`bundled_stdlib`)
- codegen never re-desugars
- the topic-reference and intra-locus rewrites are lowering's, on lowering's copy of the program, never the checked one, and each is recorded as a relation: `TopicRewrite` rows (`topic_rewrites`, and `written_topics` on the bus graph's subjects) and `IntraLocusRewrite` rows (`intra_locus`, and `direct_sends`); the intra-locus rewrite is its own stage, `rewrite_intra_locus` (over the sequence's program, its qualified bus subjects already resolved; the snapshot's `intra_locus` count), which the check demands for rule 10 and lowering continues from, so the judgment and the lowering read one rewrite; `resolve_rewritten` runs the topic rewrite, appends the stdlib, mints and derives the tables
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
- `build_executable_with_options(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×1

### `sync_inference` — Migrating · derivation

**Answers.** Which sync discipline each `@form` declaration gets: one row per declaration with the author's configuration (omitted, a written discipline, `none` included, or an argument naming none) and the effective discipline, inference's pick for a `hashmap` form left unconfigured, from the domains each of its instances is called from; two queries, explicitly configured and safe for cross-domain access.

**Inputs.** placement (the table, per instance); top_scope; form declarations; method call sites

**Producer (today's authority, migrating).** `crates/hale-types/src/form_rows.rs` · `form_rows`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/alloc_summary.rs` · `summarize_identified` — the allocation summary reads a written `sync =` argument for `sync_forms`, the only answer for the stdlib's analysis copy, which no snapshot's rows hold; the effects engine adds the rows' (`add_sync_forms`). *Removed when:* the stdlib's forms are rows of the snapshot (the stdlib merged once).

**Also owned.** `crates/hale-types/src/sync_inference.rs` · `infer_sync_for_bundle`

**Consumers.** the snapshot (one row set per snapshot, after the mint, over its scope and placement table) (`crates/hale-frontend/src/snapshot.rs` · `demand_forms`); check (F.31 cross-pool verdicts: the one predicate, safe for cross-domain access) (`crates/hale-types/src/check.rs` · `check_placement_single_thread`); check (instance aliasing: a field behind a sync discipline) (`crates/hale-types/src/check.rs` · `locus_has_unsynchronized_state`); the effects certificate engine (a call into a sync-bearing form or its holder can take its lock) (`crates/hale-types/src/alloc_summary.rs` · `add_sync_forms`); sync inference (its candidates: the forms not explicitly configured) (`crates/hale-types/src/sync_inference.rs` · `infer_sync_for_bundle`); model (`sync_form`, read by the `depends` law) (`crates/hale-types/src/model_builder.rs` · `derive_application_model_over`); the lowering view (the snapshot's rows, and the merged stdlib's as written) (`crates/hale-types/src/resolved.rs` · `resolve_program`); codegen (the slot layout) (`crates/hale-codegen/src/locus/decl.rs` · `sync_mode`); lsp

**Invariants.**

- every entry point sees the same discipline for the same program
- one row per `@form` declaration, found by the identity the load minted (a monomorph by its template's) or by name: the configuration and the effective discipline are separate columns
- an explicit `sync = none` is configuration: inference does not run over it, and the row does not call it safe for cross-domain access
- one predicate per question: inference's candidates are the forms not explicitly configured, and the F.31 cross-pool exemption is safe for cross-domain access; sync inference runs once per snapshot, and the cross-pool diagnostic's hint reads its reasoning from the rows
- nothing writes the discipline into the program: the check, the effects engine, the model and lowering read the row, and a declaration with no row (the stdlib's, merged for lowering) reads its written argument
- the readers that ask whether a form synchronizes as one question (the model's `sync_form`, the effects engine, instance aliasing) ask safe for cross-domain access alone (`FormRows::synchronizes`): an explicit `sync = none` takes no lock and is not one
- inference is per instance (the placement correspondence's K-5): an access `self.f.m()` is made from the domain of each instance of its enclosing locus and reaches that instance's own `f` (a held one by its source row, so its holders share it); the rule is applied per accessed instance and a type gets the most synchronized discipline any instance needs, never a union of domains per type; what the table does not know (a dynamic literal of unknown domains, a held instance whose source is unlinked, the instance below one) is a domain apart from every other, never main

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/form_rows.rs; crates/hale-frontend/src/snapshot.rs (the_form_rows_are_one_family_by_identity); crates/hale-types/tests/placement.rs

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

### `top_scope` — Migrating · derivation

**Answers.** What every top-level name denotes: the symbol table over the merged program.

**Inputs.** the merged program; import renames

**Producer (today's authority, migrating).** `crates/hale-types/src/resolve.rs` · `build_top_scope`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/lib.rs` · `check_bundle_opts_scoped` — `check_program` (the test entry): built here, once, for its checker and the model its laws are judged over. Beside it the model of a bundle no snapshot holds (`derive_application_model`: `claim_law_diags`, the hale-types tests, and the artifact and model-hash entries over a bare bundle; since 2.3 no verb reaches it), the certificate report of such a bundle (`effect_certificates`, for its form rows) and the lowering view (once, for the ownership graph and the bus graph) rebuild it; every verb and the LSP (its diagnostics and every request) build one per snapshot (`demand_scope`) and pass it to the checker, the model and the model's graphs. *Removed when:* every consumer demands the scope from a snapshot (2.3).

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_bundle_scoped`); check (type expressions: the scope's name table) (`crates/hale-types/src/check.rs` · `&top.names`); demand (every verb, the LSP's diagnostics and its requests: one scope per snapshot) (`crates/hale-frontend/src/snapshot.rs` · `build_top_scope`); model (the snapshot's scope, handed in) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); resolved program (lowering) (`crates/hale-types/src/resolved.rs` · `build_top_scope`); lsp (definition, placement, the allocation survey: the snapshot's scope) (`crates/hale-lsp/src/lib.rs` · `demand_scope`); lsp (completion, hover, references, enforcement: the editor's scope, over the members that parsed while one does not) (`crates/hale-lsp/src/lib.rs` · `demand_editor_scope`)

**Invariants.**

- one namespace decision: module-nested declarations and imported seeds resolve the same way everywhere
- the editor's scope over a seed with a hole (`demand_editor_scope`) is the same producer over the members that parsed, counted as this family; the whole scope, the check and everything after it stay blocked, so no check runs over a partial program
- one name table: the checker resolves every type expression against the scope's own (`TopScope::names`, the declared loci, types and perspectives with each alias's expanded target and the bundle's import renames), built once with the symbols, and keeps none of its own

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/checks_inside_modules.rs; crates/hale-cli/tests/check_unknown_identifier.rs; crates/hale-types/tests/type_alias.rs

**Spec.** spec/semantics.md

**Guarded seams.**

- `build_top_scope(` may be referenced from: `crates/hale-types/src/resolve.rs` ×1, `crates/hale-types/src/lib.rs` ×3, `crates/hale-types/src/sync_inference.rs` ×1, `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

### `expression_typing` — Canonical · derivation

**Answers.** The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from.

**Inputs.** top_scope; declarations; bodies

**Producer.** `crates/hale-types/src/check.rs` · `check_bundle_scoped`

**Also owned.** `crates/hale-types/src/resolve.rs` · `infer_literal_ty`; `crates/hale-types/src/typed_bodies.rs` · `typed_bodies`

**Consumers.** the snapshot (one typed-body table per snapshot, packaged on demand from the check's record) (`crates/hale-frontend/src/snapshot.rs` · `demand_typed_bodies`); codegen (an accumulator slot's element type, the closure's typed-body row) (`crates/hale-codegen/src/codegen.rs` · `accumulator_element_type`); every layer

**Invariants.**

- expression typing is not a layer: it is the derivation inside layer 3 that produces typed edges, and it stays Rust (final direction)
- codegen types no value the checker typed: an accumulator's element type is the closure's typed-body row, and a hole is refused at its span
- the checker's answers are carried, never re-derived: the check records them as it walks, and one typed-body table per snapshot packages the record (`demand_typed_bodies`, no second check but for a typing that reused a declaration, the snapshot family's X2 row; a check that never asks builds none), keyed by declaration identity (a body by its declaration's site, a call by its `Call` site, a monomorph by its template's site and type arguments, never by a name string), with five columns: accumulator element types, generic calls' type arguments and unified params, the monomorph table, conformance per (locus, interface) pair, fallible calls; a site the checker could not type is a hole with its reason

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/typed_bodies.rs; crates/hale-types/tests/codegen_fixtures_typecheck.rs; crates/hale-codegen/tests/corpus_check_build_agreement.rs

**Spec.** spec/types.md

**Guarded seams.**

- `typed_bodies(` may be referenced from: `crates/hale-types/src/typed_bodies.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

### `generics` — Migrating · derivation

**Answers.** Which monomorph a generic call instantiates and how its bindings unify.

**Inputs.** generic declarations; call arguments; the mangled token vocabulary

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `unify_generic_ty`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `unify_generic_param_bindings` — a Ty-level mirror of the checker's unification, by its own comment. *Removed when:* codegen reads the call's typed-body row (its type arguments, and the monomorph table's name for them); blocked until the checker types the bare builtins a generic call's argument can be (`len`, `abs`, `min`, `max`, `to_string`, the numeric casts), which it types `Unknown` today, so `first(len(s))` is a hole the base lowers.
- `crates/hale-codegen/src/codegen.rs` · `infer_generic_fn_args` — codegen infers generic arguments again from lowered types. *Removed when:* same.

**Also owned.** `crates/hale-types/src/check.rs` · `monomorph_table`; `crates/hale-types/src/check.rs` · `specialize_generic_fns`

**Consumers.** check (a mangled monomorph name: the table's row) (`crates/hale-types/src/check.rs` · `resolve_generic_monomorph`); codegen

**Invariants.**

- one unification; the monomorph set is a row lowering reads
- one monomorph table per snapshot (the typed-body table's `monomorphs`), keyed by the template's site and its type arguments, never by a name string: its producer parses a mangled name once, for each name the program spells (a written instantiation as the checker resolves it, an annotation, a struct literal's path), against the bundle's templates by identity; the checker's lookups read the row
- a generic call inside a generic fn's body is typed again for each of the fn's monomorphs, the template's parameters bound to its arguments, and recorded under them; that walk reports nothing

**Missing data.** a missing required row is a compiler error

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

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/perspective_serves.rs; crates/hale-types/tests/duplicate_member.rs; crates/hale-types/tests/typed_bodies.rs; crates/hale-codegen/tests/conformance_routing_correction.rs

**Spec.** spec/types.md; spec/semantics.md

### `forms` — Migrating · law

**Answers.** Whether a form's shape, its capacity slots and its projection class are well formed, and which operation set closes each slot.

**Inputs.** @form arguments; capacity declarations; indexed_by; sync discipline (the `sync_inference` form rows)

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_form_shape`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/decl.rs` · `ring_buffer_cap` — codegen reads the form's `cap =` argument again for the slot layout (a ring buffer's or LRU cache's capacity, a lockfree map's fixed capacity); the `sync` discipline it reads from the form row. *Removed when:* codegen reads the form rows.

**Consumers.** check; codegen (slot layout); sync_inference

**Invariants.**

- the operation set a form closes over a slot is a row: it is what a storage binding (F.44) will need

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/reserved_locus_members.rs; crates/hale-codegen/tests/form_vec_bce.rs

**Spec.** spec/forms.md; spec/memory.md

### `stdlib_surface` — Migrating · capability

**Answers.** What each stdlib function is: its signature, its effect classes, whether it blocks, and what a value of a type can be rendered as.

**Inputs.** the stdlib registry; hale_stdlib::PATH_RENAMES; the parsed stdlib source

**Producer (today's authority, migrating).** `crates/hale-types/src/stdlib_surface.rs` · `signature_for`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `lower_stdlib_path_call_expr` — 271 `["std", ..]` literals dispatch stdlib calls inside codegen; the registry's own comment calls this dispatch `reality`. *Removed when:* codegen dispatches from the registry row.
- `crates/hale-codegen/src/codegen.rs` · `lower_stdlib_path_call` — the statement-form twin of the expression dispatch: 191 more `["std", ..]` literals. *Removed when:* same.
- `crates/hale-codegen/src/channels/mod.rs` · `lower_fallible_call` — the fallible-call dispatch, a third copy of the stdlib call shapes (150 literals). *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `value_to_string_supports` — the printable set, kept in lockstep by hand with the checker's `ty_is_printable`. *Removed when:* one predicate.
- `crates/hale-types/src/check.rs` · `ty_is_printable` — the checker's copy of the printable set. *Removed when:* one predicate.
- `crates/hale-codegen/src/codegen.rs` · `declare_builtin_closure_violation_type` — a hand-maintained mirror of the checker's injected builtin types. *Removed when:* one declaration.

**Consumers.** effects (`crates/hale-types/src/effects.rs` · `effects_for`); frontier (`crates/hale-types/src/frontier.rs` · `effects_for`); codegen; lsp (hover, completion); doc

**Invariants.**

- the checker and codegen agree on every stdlib call shape (parity test) and on the printable set (corpus agreement)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/stdlib_registry_parity.rs; crates/hale-codegen/tests/corpus_check_build_agreement.rs; crates/hale-cli/tests/doc_effects_catalogue.rs

**Spec.** spec/stdlib.md

## Layer 3 — the locus graph

### `entrypoint` — Migrating · derivation

**Answers.** Which locus is the program's `main`, whether the world is closed, and which declarations are imported.

**Inputs.** locus declarations (is_main, imported, the __lib_ prefix, module nesting); the minted sites

**Producer (today's authority, migrating).** `crates/hale-types/src/entry.rs` · `entry_row`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/entry.rs` · `lowering_root` — the row's provisional column: the `main locus` lowering deploys today, `collect_main_placement`'s choice copied exactly (the first `is_main && !__lib_` over the flat declarations, module-nested ones included), so the placement-safety rules (the F.31 rule, the blocking check and the pinned-in-a-loop rule, through the placement table) guard the threads lowering spawns while a seed whose only `main` is module-nested has no entry. *Removed when:* L4: lowering reads `entry`, and the column goes.
- `crates/hale-codegen/src/codegen.rs` · `collect_main_placement` — `is_main && !__lib_` over flat declarations. *Removed when:* one definition of the entry, as a row.
- `crates/hale-codegen/src/locus/instantiation.rs` · `let is_main_locus` — `is_main_locus` compares type names at instantiation (and twice more in dissolve.rs). *Removed when:* same.
- `crates/hale-codegen/src/locus/dissolve.rs` · `let is_main_locus` — the same comparison in the cascade. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `is_main_entry` — the deferred entry teardown compares the entry's locus name with `main_locus_name` to decide whether it joins the pools (#1208). *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `emit_bindings_prelude` — the connect-transport loss handler is looked up in the locus named by `main_locus_name`. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `in_main` — whether lowering is inside `fn main` is a flag set while main's body is emitted (and cleared around a generic fn lowered from inside it); the frame flush's main-exit wait-abort and `return`-from-main's teardown key on it. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `collect_shm_ring_subjects` — the shm-ring subjects are read through `root_bindings`, which takes the first `is_main && !__lib_` over the flat declarations, `collect_main_placement`'s choice made again. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `synthesize_codec_thunks_for_main_bindings` — the binding codec thunks are synthesized for the entries `root_bindings` reads, the first `is_main && !__lib_` over the flat declarations, the same choice made again. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `let has_socket_binding` — whether the program has a socket binding (so the cooperative queue is locked) asks the TOP-LEVEL `is_main && !__lib_` declarations only, where `collect_main_placement` walks the flat declarations: a module-nested root's bindings are not seen. *Removed when:* same.
- `crates/hale-types/src/check.rs` · `check_placement_entry_consumed` — rule 18's scope is the LAST `is_main && !__lib_` over every declaration, module-nested ones included (lowering takes the first; the two differ only under rule 1's error). *Removed when:* reads `lowering_root`, since the rule guards what lowering emits; reads the entry with L4.
- `crates/hale-types/src/check.rs` · `check_instance_aliasing` — instance aliasing relates the placed fields of the LAST `is_main` declaration's static params tower, with no filter (an imported `main` included). *Removed when:* same.
- `crates/hale-types/src/check.rs` · `check_pool_affinity` — validates EVERY `is_main` declaration's own placement block (an affinity with no named pool, two affinities for one pool), deployed or not: validation of each declaration, which derives no entry fact. *Removed when:* none for the entry: it leaves this inventory when it walks the row's witness (`mains`) instead of the declarations (L4).
- `crates/hale-types/src/check.rs` · `let api_bound` — `check_bus_graph`'s orphan lint is lifted when ANY `is_main` declaration carries an `api:` binding: a module-nested one, or an imported one whose api entry is inert (GH #1104 piece 5). *Removed when:* reads the entry, whose binding is the one that binds (L4).
- `crates/hale-types/src/desugar_sequence.rs` · `let at = programs` — the `--api` injection target in `desugar_before_check`: the first program holding a top-level `is_main`, an imported one included. *Removed when:* reads the entry: the binding goes on the entry, and a seed with none is refused (L4).
- `crates/hale-syntax/src/api_gen.rs` · `inject_api_entry` — the injected `api:` entry goes on that program's first `is_main` at any depth, module-nested and imported ones included. *Removed when:* same.
- `crates/hale-syntax/src/api_gen.rs` · `api_surface` — the api surface is built around the first `is_main && !imported` carrying an `api:` entry, at any depth (a module-nested `main` included). *Removed when:* same.
- `crates/hale-syntax/src/api_gen.rs` · `declared_roles` — `owner` joins the declared roles when any `is_main && !imported` declaration at any depth carries an `api:` entry. *Removed when:* same.
- `crates/hale-frontend/src/snapshot.rs` · `inject_adopt` — an environment's constitution is adopted into EVERY top-level `is_main` of the program (an imported one included, a module-nested one not), and a program with none refuses it. *Removed when:* reads the entry: an environment binds law to the entry (L4).
- `crates/hale-types/src/claims.rs` · `has_main = true` — world-tier claims are gathered from every `is_main` at any depth, and `has_main` refuses a top-level `claims` block in a seed that closes. *Removed when:* same.
- `crates/hale-types/src/model_builder.rs` · `let main_decl` — the model's arrangement root is the first `is_main` among the model's loci, with no filter. *Removed when:* reads the entry (L4).
- `crates/hale-types/src/model_builder.rs` · `let entrypoint = ast` — the model's `entrypoint` name is the first `is_main` among its loci, else `main`. *Removed when:* same.
- `crates/hale-types/src/bus_graph.rs` · `let has_entry_point` — the bus graph's closed world is any top-level `is_main` or top-level `fn main`, an imported `main` included; deliberately broader than rule 9's. *Removed when:* reads the entry, beside the `fn main` entry point (L4).
- `crates/hale-types/src/ownership_graph.rs` · `let has_entry_point` — the ownership DAG's closed world, the bus graph's test made again. *Removed when:* same.
- `crates/hale-types/src/mangle.rs` · `seed_declares_main` — whether an imported seed declares a `main locus` (GH #774): any top-level `is_main` over the seed's files. *Removed when:* reads the imported seed's own entry row (L4).
- `crates/hale-types/src/effects.rs` · `placement_implied_diags` — the async_io pool's locus types come from every top-level `is_main` declaration's placement, an imported one included and a module-nested one not. *Removed when:* reads `lowering_root`, since the pool is one lowering spawns; reads the entry with L4.
- `crates/hale-lsp/src/lib.rs` · `placement_of` — the editor's placement view shows every top-level `is_main` declaration's placement. *Removed when:* same.
- `crates/hale-cli/src/dna.rs` · `main_of` — `hale dna init` takes the LAST top-level `is_main` over the seed's files, parse-only (no import is resolved, so no mark exists). *Removed when:* reads the entry row over the seed's own files, as `seed_entry_kind` does (L4).
- `crates/hale-types/src/check.rs` · `if parent.is_main` — `check_nested_long_running_child` exempts a `main locus`, as a parent and as a child, from the long-running-child rule: a property of each declaration, which derives no entry fact. *Removed when:* none for the entry: it leaves this inventory when it reads the row's witness (`mains`) instead of the keyword (L4).
- `crates/hale-types/src/ownership_graph.rs` · `entry.singleton |= l.is_main` — every `main locus` declaration is a singleton in the ownership graph: a property of each declaration, which derives no entry fact. *Removed when:* same.
- `crates/hale-types/src/alloc_summary.rs` · `let mut eager_only_loci` — every top-level `main locus` declaration is excluded from eager reclamation, conservatively: a property of each declaration, which derives no entry fact. *Removed when:* same.

**Consumers.** check (rule 1's count reads the witness: the seed's own mains, module-nested ones included) (`crates/hale-types/src/check.rs` · `check_main_and_bindings`); placement (the table is seeded from the lowering root; the F.31 rule, the form rows' sync inference, the blocking check and the pinned-in-a-loop rule read it) (`crates/hale-types/src/placement.rs` · `derive_placement`); check (rule 9's closed world is a program with an entry) (`crates/hale-types/src/check.rs` · `check_bus_graph`); check --matrix (a seed is an entrypoint when its row has an entry; the row is built over the seed's own files, since no import holds the entry, so a seed whose import does not resolve is still counted and its pair reports the import) (`crates/hale-cli/src/verbs/check/matrix.rs` · `seed_entry_kind`); --env on check and the build paths (the load refuses an environment for a seed with no entry, after the mint, before the sequence's own refusal) (`crates/hale-frontend/src/snapshot.rs` · `demand_entry`); build; dna; codegen

**Invariants.**

- the checker builds no row: the snapshot demands it before the check and hands it in (`CheckInputs::entry`); a bundle no snapshot holds (the test entries) builds it once, the form rows read the snapshot's (`demand_forms`), and the check matrix builds one over each seed's own files (no import holds the entry, and a seed whose import does not resolve is still an entrypoint)
- one row per snapshot (`Snapshot::demand_entry`, the `entrypoint` count): the entry by its minted site, and every `main locus` the bundle declares as the witness, each with whether it is imported and whether it is module-nested; it reads declarations only, so no diagnostic blocks it, and a seed with a hole (no identities) blocks it with its scope
- an imported `main` is not the entry (decision 1, E0): a library's `main locus` is a declaration the importing seed does not run, and the entry is the importing seed's own; a seed whose only `main` is imported has no entry (`NoEntry::OnlyImported`). Imported is one definition, the rename pass's mark (`imported`, GH #1104 piece 5); the `__lib_` name the same pass gives is spelling, not a second test of the entry (the provisional lowering root copies lowering's name test, until L4)
- a module-nested `main` is not the entry (decision 2, E0): the entry is a top-level `main locus` of the seed's own files, so rule 9's closed world is the top-level one; a seed whose only `main` is module-nested has no entry (`NoEntry::OnlyModuleNested`), and a seed with both keeps the top-level one. Rule 1 still counts a module-nested `main` (GH #825): the count reads the witness, not the entry
- the decisions bind what reads the entry (rule 9's closed world, `--env`, `--matrix`), not the placement-safety rules: until lowering reads the entry (L4) it deploys a module-nested `main` as its root and spawns its pinned threads, so the placement table, which the F.31 rule and the pinned-in-a-loop rule read, is seeded from the row's provisional `lowering_root`, and a seed whose only `main` is module-nested has no entry and its cross-pool call is still refused (the outside review of #1293, finding 1)
- with more than one candidate (rule 1's error) the entry is the last, as the checker's pool map (since replaced by the placement table) took it before the row; the lowering root is the first non-`__lib_` `main`, nested or not, as lowering takes it
- the legacy sites use three definitions of the main locus today, and lowering's `in_main` a fourth, of fn main; the row has one
- every non-test reader of `LocusDecl::is_main` is the producer or a legacy row here; the parser's main-only member rules are syntax and are not rows, and `sync_inference`'s test helper is test code

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-frontend/src/snapshot.rs (the_entry_row_is_the_seeds_own_top_level_main_locus); crates/hale-cli/tests/check_entry_decisions.rs; crates/hale-cli/tests/nested_main_transition.rs (the nested-main transition end to end: refused, and deployed, as a top-level main); crates/hale-cli/tests/entry_point_placement.rs; crates/hale-types/tests/bus_graph.rs

**Spec.** spec/semantics.md § Bundle-wide rules

**Guarded seams.**

- `entry_row(` may be referenced from: `crates/hale-types/src/entry.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/effects.rs` ×1, `crates/hale-cli/src/verbs/check/matrix.rs` ×1, `crates/hale-types/src/sync_inference.rs` ×1

### `ownership` — Migrating · derivation

**Answers.** Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array.

**Inputs.** locus declarations (params, accept, release); bodies (let, assign, return, field initialisers, placement entries); fresh factories (one producer); returned bindings

**Producer (today's authority, migrating).** `crates/hale-types/src/ownership.rs` · `resolve_owners`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/ownership_graph.rs` · `build_ownership_graph` — which accepting ancestor owns a method-body birth, keyed (enclosing locus, child type) by name, the child type resolved by `child_locus_name`; built once in the resolved program for lowering and once per snapshot over the checked programs for the check and the model (`demand_ownership_graph`). *Removed when:* phase 3, one ownership table with both relations, when the lowering view's graphs fold into the snapshot: the check still runs over the checked programs and the resolved program is built only on build paths (`resolve_program`), so the model's graph and lowering's are two builds over two program forms (at the phase-2 close).
- `crates/hale-types/src/model_builder.rs` · `Owns` — the model's params-field tree from main, a third ownership account. *Removed when:* projected from the one table.
- `crates/hale-types/src/ownership.rs` · `extend_fresh_factories` — the carrier-arm fixpoint that widens the factory set. *Removed when:* phase 3, as a judgment migration: the carrier fold widens the set lowering reads and the checker reads the unextended set, so giving the checker the extended set changes which bindings it checks as factory-returned (lowering-only at the phase-2 close; deferred in its exit comment).
- `crates/hale-types/src/borrow_lifetime.rs` · `accepts` — the borrow-lifetime law rebuilds the accept sets from the AST for itself. *Removed when:* reads `accepts_ancestor`.

**Also owned.** `crates/hale-types/src/ownership.rs` · `resolve_binding_facts`; `crates/hale-types/src/ownership_graph.rs` · `bubble_plans`; `crates/hale-types/src/ownership_graph.rs` · `compute_forwarding_sets`; `crates/hale-types/src/ownership_graph.rs` · `classify_owner_kind`; `crates/hale-types/src/ownership_graph.rs` · `classify_edge`; `crates/hale-types/src/ownership.rs` · `fresh_factories`

**Consumers.** codegen (`crates/hale-codegen/src/locus/instantiation.rs` · `site_owner`); codegen (a monomorph's accept rows: its template's, specialized at synthesis) (`crates/hale-codegen/src/codegen.rs` · `specialized_accepts`); codegen (whether the enclosing locus accepts the child it births) (`crates/hale-codegen/src/locus/instantiation.rs` · `parent_accepts_us`); borrow_lifetime (`crates/hale-types/src/borrow_lifetime.rs` · `borrow_lifetime_diags`); model (dynamic births: the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `demand_ownership_graph`); a declaration's dependents (X2: a locus's births, accepts and instantiations make its neighbours through the ownership graph, the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `declaration_dependents`); check (type-check rule 20, the unowned-subscriber rule: `owner_of_site` over the snapshot's graph, handed in through `CheckInputs`) (`crates/hale-types/src/check.rs` · `check_unowned_subscriber_locus`); alloc_summary (eager-only accept sets)

**Invariants.**

- a locus instantiation with no row is a CodegenError (F.39)
- ids, not names or spans: declarations are cloned and the stdlib's coordinates overlap user files
- the ownership matrix stays green with an empty KNOWN_OPEN
- the graph keeps each `accept` param as a row of the declaring locus's identity, its type as written (`AcceptRows`); a monomorph's accept set is its template's rows, asked for by the template's identity and specialized by the instantiation's substitution (`AcceptRows::specialize`, as `FlowRows::specialize` does for release clauses), filled at synthesis; lowering never reads a monomorph's own `accept_param` for ownership
- `fresh_factories` is read by lowering and the checker with the bundle's import renames; a factory's returned name is the declaration the snapshot resolves it to, so a fn whose returned name an inner `let` shadows is a factory of the outer binding (the #1140 shape; its escape walk still reads every binding spelling the name as the returned one, the conservative side). The carrier-arm extension (`extend_fresh_factories`) is folded in for lowering only
- which declaration a returned or escaping name denotes is read from the snapshot (`Snapshot::declaration_of` over `binding_of`, resolved once by the mint), never resolved again: `returned_bindings` (the binding facts and the pre-pass), `fresh_factories`, borrow_lifetime's `returned_decls` and alloc_summary's escape tags each key a binding by its declaration's SiteId; a `let` or a use the snapshot did not mint answers by name in `returned_bindings` (the conservative side) and resolves to nothing elsewhere, and every entry point mints
- the checker builds no graph: the snapshot demands it before the check and hands it in (`CheckInputs::ownership`), so a check with a law builds it once for the checker and the model together; a bundle no snapshot holds (the test entries) builds it once, in `bundle_ownership_graph`, and the model over that bundle reads the same build
- rule 20 is judged by declaration identity (`owner_of_site`): the graph lists every locus declaration in declaration order, and a site carries the declaration its literal names (`child_decl`, the first of a shared name) and the child an `accept` must name to own it (`child_key`: an import or `std::` path resolved as an `accept`'s type is, a template specialized by its binding's or field's declared type); the nearest accepting ancestor owns a birth in a handler as it owns any other, but only if one accepts the child on every construction path of the enclosing locus (`construction_paths`, over the placement table: each instance row under its owner row's declaration, a template's top as a root with no ancestor, a literal directly in `fn main` included, each dynamic site under its enclosing locus or in a free fn whose callers are not followed; with the walk's own body and params-default edges, since the table does not enumerate a dynamically built locus's params subtree), and the diagnostic names a path with none. `child_ty` and `resolution` stay keyed by the literal's last segment, as lowering and the model read them, and lowering's `instantiated_by` still records no free-fn birth
- the holes: where the graph cannot decide, `owner_of_site` answers None and rule 20 does not fire, since unknown ownership is not proven absence: an open world (no entry, so a consumer may complete the tower), a generic template no declared type specializes, a qualified path that names no declaration, and an unanalyzable site; a literal assigned to a field of `self` is judged by the `accept` edges, since the graph records no assignment target. A construction path the placement table records as a hole cuts the other way, since an unknown path cannot prove an owner: a held instance the table does not link, a field whose initializer is no literal, a dynamic site of unknown domain and an owner row whose declaration does not resolve each count as a path with no ancestor, and so does a free fn; a held row the table links adds no path (its source row is the construction)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/owner_table.rs; crates/hale-codegen/tests/ownership_matrix.rs; crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding); crates/hale-codegen/tests/ownership_bubble.rs; crates/hale-cli/tests/check_unowned_subscriber.rs (rule 20, one test per class of the migration)

**Spec.** spec/decisions.md F.39; spec/semantics.md § Dissolve timing rules; spec/semantics.md § Placement block (type-check rule 20)

**Guarded seams.**

- `resolve_owners(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/ownership.rs` ×1
- `build_ownership_graph(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `fresh_factories(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/check.rs` ×1
- `resolve_binding_facts(` may be referenced from: `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `returned_bindings(` may be referenced from: `crates/hale-types/src/ownership.rs` ×3
- `bubble_plans(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `owner_of_site(` may be referenced from: `crates/hale-types/src/check.rs` ×1
- `demand_ownership_graph(` may be referenced from: `crates/hale-frontend/src/snapshot.rs` ×4
- `bundle_ownership_graph(` may be referenced from: `crates/hale-types/src/lib.rs` ×3, `crates/hale-types/src/check.rs` ×1

### `bus_graph` — Migrating · derivation

**Answers.** The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates.

**Inputs.** topics; bus blocks; sends; bindings; placement (for gates)

**Producer (today's authority, migrating).** `crates/hale-types/src/bus_graph.rs` · `build_bus_graph`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/lib.rs` · `bundle_bus_graph` — the check and the model of a bundle no snapshot holds (the test entries: `check_bundle`, `check_bundle_opts_scoped`, `claim_law_diags`, the hale-types tests, the artifact's bundle entry) build the bus graph here, once per entry, beside the ownership graph and the handler rows, and the check's intra-locus relation beside it (`bundle_intra_locus`, the stage over the bundle's programs merged); every verb reads its snapshot's. *Removed when:* those callers hold a snapshot.
- `crates/hale-types/src/resolved.rs` · `build_bus_graph` — built once in the resolved program, over the desugared program, for lowering; the snapshot builds a second over the checked programs for the check, the model and hale/busGraph (`demand_bus_graph`). *Removed when:* phase 3, one graph per snapshot, when the lowering view's graphs fold into the snapshot: the check still runs over the checked programs, not the resolved one, so lowering's graph (with the intra-locus and topic rewrites recorded on it) and the snapshot's are built over two program forms (at the phase-2 close).

**Also owned.** `crates/hale-types/src/bus_graph.rs` · `dispatch_gates`; `crates/hale-types/src/bus_graph.rs` · `cycle_from`; `crates/hale-types/src/bus_graph.rs` · `external_handlers`

**Consumers.** check (rule 9: the wire rows, their bound, cross-seed and wildcard columns, over the entry row's closed world; the snapshot's graph, `CheckInputs::bus`) (`crates/hale-types/src/check.rs` · `check_bus_graph`); check (rule 10: the edges by declaration, through `cycle_from`, each edge's send joined to the intra-locus relation by its id, `CheckInputs::intra_locus`) (`crates/hale-types/src/check.rs` · `check_bus_cycles`); check (rule 7: the placed declaration's row, `external_handlers`) (`crates/hale-types/src/check.rs` · `check_cooperative_pool_blocking`); check (rules 11, 12, 19); model (subjects, endpoints and gates: the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `demand_bus_graph`); topology; dispatch; lsp (hale/busGraph: the model's graph, so eligibility is the diagnostics pass's) (`crates/hale-lsp/src/lib.rs` · `demand_bus_graph`); codegen (a rewritten publish, found by its call's id in the relation: the probes and the reclaimed subregion) (`crates/hale-codegen/src/codegen.rs` · `intra_locus_rewrite`)

**Invariants.**

- one graph, over one program shape, per snapshot; rule 10's cycle graph is a query over it (`cycle_from`)
- the checker's bus rules (7, 9, 10) compare subjects under the canonical key, the wire subject (`Subject`, `wires`): a topic published by name and subscribed by its literal subject is one subject; the gates and the model keep `BusSubject::canonical()`'s keys (`subjects`)
- an edge belongs to the locus declaration that wrote its handler (`BusEdge::decl`), never to a name: two loci of one name have their own edges
- a subject the graph cannot resolve is a hole (`holes`): it forms no edge and no rule calls it an orphan
- the intra-locus rewrite is a relation on the graph, never an erased publisher (boundary 7)
- lowering reads the relation (`LoweringView::intra_locus`) by the call's id, which the call that replaces a send keeps (`IntraLocusRewrite::send`); it never classifies a call as a rewritten publish from the call's shape
- rule 10 calls a cycle synchronous only where the relation holds every send of it (`BusEdge::send`, the snapshot's `intra_locus` stage, the one lowering continues from); a same-declaration cycle with a send the relation does not hold is carried by the queue, and the subjects' spelling never decides which

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/bus_graph.rs; crates/hale-types/tests/bus_rules_over_graph.rs; crates/hale-types/tests/bus_payload_handler.rs; crates/hale-codegen/tests/bus_devirt_differential.rs

**Spec.** spec/semantics.md rules 7, 9-12, 19; spec/verification.md § Bus-graph property checks

**Guarded seams.**

- `build_bus_graph(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `collect_bus_walk(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×2
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

**Missing data.** an unknown is a hole with a stated policy

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

**Consumers.** check (the binding rules walk the rows: topic, duplicate, role, adapter, ring layout, constraints, codec; the `or wait` legality check and the api gates read the bound-topic set) (`crates/hale-types/src/check.rs` · `check_main_and_bindings`); check (a binding's `where` constraints are held to its transport's guarantee: the capability module's table, read through the row's transport kind) (`crates/hale-types/src/capability/transport.rs` · `guarantee`); the transport's cell on the effective target: `RemoteTransport(kind)` × the target row's backend, read through the snapshot (verdict-neutral: the adapter's wasm refusal is a late link refusal today, a known-open cell) (`crates/hale-frontend/src/snapshot.rs` · `binding_cell`); model (main's binding thread domains: the role and the transport kind) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); bus graph (the bound-topic set, at both grains, is the rows' projection) (`crates/hale-types/src/bus_graph.rs` · `collect_bus_walk`); codegen (the prelude, the shm-ring subjects, the codec thunks and the pinned adapter loci read each entry's transport kind, role, codec and producer-versus-attach from its row, through the lowering view; the entry's own text supplies the transport's parameters; a missing row is an error) (`crates/hale-codegen/src/codegen.rs` · `root_bindings`); api_surface

**Invariants.**

- the transport-loss handler is named by the row: a `unix` entry's row carries the stdlib locus its transport instantiates (`loss_locus`, by role), lowering instantiates that locus, and a connect entry's is the locus whose failure the main locus's `on_failure` handles, so the handler is main's routing row for the locus the bindings row names, not one picked by a spelled name
- lowering holds the rows (`LoweringView::bindings`, the snapshot's) and finds an entry's by the id the mint kept (`BindingRows::for_entry`); it decides no transport, role, codec or producer-versus-attach itself, and an entry with no row is a `CodegenError`, not a guess
- F.36 and F.37: binding failure is structural; codec purity is a law over rows
- one row per snapshot (`Snapshot::demand_bindings`, the `bindings` count): one row per `bindings { }` entry of every locus of the bundle, an imported main's and a module-nested one's included, each with the entry's site, the topic and its wire key, the transport kind, the role, the codec, whether the bundle produces the topic and the stdlib locus a transport's loss surfaces through; the checker builds none (`CheckInputs::bindings`), and a bundle no snapshot holds builds it once
- the role is decided once, over the topic's ends read by wire subject (`desugar::role_from_ends` over the row's `publishes` and `subscribes`): the entry's own role wins, otherwise publish-only is `Connect` and subscribe-only is `Listen`, and a `unix` entry with neither is the checker's diagnostic. The checker, the model and lowering read it; the desugar's in-place fill applies the same pure rule over the topic names before the topic rewrite erases them, and agrees with it over the corpus
- what a transport carries is data beside the matrix, not a branch in the checker: the transport kind's guarantee for each `where` constraint (`capability::transport::GUARANTEES`, three rows of four cells, the former `transport_satisfies` cell for cell, its words verbatim), which does not vary by target; and whether a target realizes the transport at all is the matrix's own `RemoteTransport(kind)` row, asked through the snapshot's target row (`Snapshot::binding_cell`)
- the bound-topic set is the rows' projection (`bound_names`, `bound_subjects`): the `or wait` legality check, the api gates and the bus graph's eligibility gate read it, and none walks `bindings { }` itself. An imported main's entries are in the set, as they were in each of the walks it replaces

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/binding_imported_main.rs; crates/hale-codegen/tests/bindings_codec_clause.rs

**Spec.** spec/decisions.md F.36, F.37; spec/semantics.md § Operational constraints (Form K)

**Guarded seams.**

- `derive_binding_rows(` may be referenced from: `crates/hale-types/src/binding_rows.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2
- `role_from_ends(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×2, `crates/hale-types/src/binding_rows.rs` ×1

### `dispatch` — Migrating · derivation

**Answers.** How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement.

**Inputs.** bus_graph (gates); placement (domains); the flat-payload predicate; --no-bus-devirt

**Producer (today's authority, migrating).** `crates/hale-model/src/dispatch_plan.rs` · `fn derive`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/resolved.rs` · `from_gates` — the resolved program derives lowering's plan with an empty domain map (#464's widening is a separate optimization); the model derives its own with the arrangement's domains for `same_domain`. *Removed when:* phase 3, one plan: lowering's plan takes the arrangement's domains only with #464's widening, an optimization with its own bench gate not yet taken on, so the two plans still differ by their domain maps (at the phase-2 close).
- `crates/hale-codegen/src/bus/wire.rs` · `bus_payload_is_flat` — the third leg of the direct-call gate exists only in codegen. *Removed when:* a gate column.

**Consumers.** codegen (`crates/hale-codegen/src/codegen.rs` · `build_resolved`); codegen (`crates/hale-codegen/src/bus/dispatch.rs` · `bus_devirt`); exec_digest (the resolved program's plan) (`crates/hale-cli/src/shared/options.rs` · `resolved.plan.digest()`); model dump

**Invariants.**

- which flavour a subject gets is a plan conclusion, never a model row (spec/model.md)
- lowering reads one plan, derived once per snapshot in the resolved program; the execution digest frames that plan
- the model's plan agrees with it on the flavor of every subject both carry (shadowed over the corpus at phase 1.5)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/dispatch_plan_cli.rs; crates/hale-codegen/tests/bus_devirt_direct.rs

**Spec.** spec/model.md § Derived products; spec/decisions.md F.38

**Guarded seams.**

- `DispatchPlan::derive(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×1
- `from_gates(` may be referenced from: `crates/hale-model/src/dispatch_plan.rs` ×2, `crates/hale-types/src/resolved.rs` ×1

### `handler_routing` — Migrating · derivation

**Answers.** Which `on_failure` handler a failing child's locus type reaches, and from which parent.

**Inputs.** failure declarations; declared loci and type aliases (the bundled stdlib's loci included); import renames; ownership (the supervising parent instance, in lowering)

**Producer (today's authority, migrating).** `crates/hale-types/src/handler_routing.rs` · `handler_rows`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/channels/mod.rs` · `resolve_failure_route` — the parent instance is the lowering context's (supervising parent, then self, then params-init self); the handler is the row's. *Removed when:* phase 3, when the instance is a row of an instance tree: the snapshot has no instance-tree family, so the parent instance is still the lowering context's (at the phase-2 close).

**Also owned.** `crates/hale-types/src/handler_routing.rs` · `child_locus_name`

**Consumers.** codegen (the handler table, one fn per row keyed by the row's site) (`crates/hale-codegen/src/locus/decl.rs` · `handlers_of_decl`); codegen (handler bodies, by the row's site) (`crates/hale-codegen/src/locus/method.rs` · `handlers_of_decl`); codegen (a route: the row's handler fn) (`crates/hale-codegen/src/channels/mod.rs` · `failure_handler_for`); codegen (__parent_on_failure) (`crates/hale-codegen/src/channels/mod.rs` · `resolve_failure_route`); codegen (the params-settle bracket: whether the locus has a handler) (`crates/hale-codegen/src/locus/instantiation.rs` · `settles_failures`); codegen (restart in place) (`crates/hale-codegen/src/locus/restart.rs` · `restarts_in_place`); model (supervises, over the snapshot's rows: `demand_handlers`) (`crates/hale-types/src/model_builder.rs` · `Supervises`); check (duplicate handlers, over the snapshot's rows handed in: `CheckInputs`) (`crates/hale-types/src/check.rs` · `check_duplicate_failure_handlers`); check (@supervised, over the same rows) (`crates/hale-types/src/frontier.rs` · `supervised_diags`)

**Invariants.**

- the child type is resolved once, by `child_locus_name`; lowering, the checker and the model read the same row
- a row carries its parent declaration's site (`parent_id`) and a reader holding a locus declaration asks for its rows by that identity (`handlers_of_decl`, `route_decl`): a monomorph keeps its template's id and so reads its template's rows, with no scan of the templates; lowering's handler fn is a column of the row, held per locus keyed by the row's site (`LocusInfo::failure_handlers`), and the handler table, the body pass and a route each join by that site, never by the row's ordinal
- a row carries the site of the declaration its child resolves to (`child_decl`, a monomorph's template's, from the same resolution as `child`: `child_locus`), qualified by the store that minted it; the model's supervision rows join parent and child to its locus table by those sites, and a child declared outside the snapshot's programs (a stdlib locus) is `SupervisedRef::External`, whose written name is its display, never a join key; only a bundle no entry point minted joins by name
- the model's failure-handler function rows are keyed by the handler's site (`handler_fn_rows`), the routing row's identity; the signature string is the row's name, which ranks it among the function rows (two handlers are two rows whatever their signatures spell), and no reader looks a handler up by it
- a generic supervisor's rows are its template's, with the child resolved once from the template's written type, unsubstituted: a monomorph reads them by its template's identity as they are. Substituting the row at instantiation, or refusing the shape at compile time, is round 3's finding 17, a correction outside these moves
- the checker builds no rows: the snapshot demands them before the check (`CheckInputs`), and the checker's duplicate-handler rule, the `@supervised` law and the model read that one build; a bundle no snapshot holds (the test entries) builds them once, in `bundle_handler_rows`

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/lifecycle_flow.rs (on_failure_dispatch_by_child_type); tests/hale/on_failure_per_child_type_test.hl; crates/hale-types/tests/violate.rs; crates/hale-types/tests/handler_routing_probes.rs

**Spec.** spec/semantics.md § failure; spec/runtime.md (failure delivery)

**Guarded seams.**

- `handler_rows(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1
- `child_locus_name(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/ownership_graph.rs` ×4, `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/flows.rs` ×1
- `child_locus(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×3
- `DeclaredNames::of(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/flows.rs` ×1

### `flows` — Canonical · derivation

**Answers.** Which children are flows (released per completion) and which are resident.

**Inputs.** release declarations; accept declarations; declared loci and type aliases (handler_routing's resolver); import renames

**Producer.** `crates/hale-types/src/flows.rs` · `survey`

**Consumers.** check --flows (each flow type as written, with its clauses) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `flows::survey(`); check (a daemon-shaped locus that accepts a child type it releases no clause for: a law over the rows) (`crates/hale-types/src/check.rs` · `check_accept_release`); resolved program (the lowering view's rows, over the merged program) (`crates/hale-types/src/resolved.rs` · `flows::survey(`); a declaration's dependents (X2: a flow child and its `release` owners are neighbours, `Snapshot::declaration_dependents`) (`crates/hale-frontend/src/snapshot.rs` · `flows::survey(`); the lifecycle plan (an accepted flow is torn down by the reclaim its run's end runs, a resident by its owner's cascade: the snapshot's rows over the checked programs) (`crates/hale-frontend/src/snapshot.rs` · `flows::survey(`); codegen (run elision, run-end reclaim and the release call: `Cx::is_flow`, one row read) (`crates/hale-codegen/src/codegen.rs` · `is_flow`); codegen (the generic-instantiation queue: each locus specialization it creates asks the row for its template's clauses, under the substitution its synthesis applies) (`crates/hale-codegen/src/codegen.rs` · `specialize(`)

**Invariants.**

- a release clause's child is resolved once, by `child_locus_name` (handler_routing's resolver: aliases, generic instantiations, qualified paths), into the row (`FlowClause::locus`); lowering's flow-ness is a row read (`flows::is_flow`), never a comparison of its own
- the flow facts cover the specializations lowering creates: a clause whose type mentions its owner's type parameters names no locus by itself and carries its template (`FlowClause::template`: the owner's identity, its parameters in order, the type as written); `FlowRows::specialize` answers for one specialization by resolving the template's type under the substitution lowering's synthesis applied, so `Manager<Worker>`'s `release(c: T)` makes `Worker` a flow exactly as a concrete `release(c: Worker)` does
- the checker's accept/release rule judges over the rows: the release clauses a locus declares are the rows' clauses inside its declaration

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/release_reclaims_flow.rs; crates/hale-codegen/tests/release_two_parents.rs; crates/hale-codegen/tests/release_generic_owner.rs; tests/hale/release_generic_owner_test.hl; crates/hale-types/src/flows.rs (a_clause_names_the_locus_lowering_names, a_template_clause_names_the_specialization_s_argument)

**Spec.** spec/semantics.md § release(c) and flow children

**Guarded seams.**

- `flows::survey(` may be referenced from: `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×2

### `restart` — Migrating · derivation

**Answers.** Which loci declare restart operations, which restart in place, and what the restart bound is.

**Inputs.** closure and birth-check declarations; handler_routing (the rows' recovery ops)

**Producer (today's authority, migrating).** `crates/hale-types/src/handler_routing.rs` · `handler_rows`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/restart.rs` · `locus_declares_failures` — which loci a failure can come from (and so get restart points) is a walk over closure and birth-check members in codegen. *Removed when:* a column of the restart rows.
- `crates/hale-codegen/src/codegen.rs` · `RecoveryModifier::For` — the `for N` bound is lowered from the statement's own expression: lowering reads the bound from the statement; the row's `retry_bound` is the model's. *Removed when:* lowering reads the row.

**Also owned.** `crates/hale-types/src/handler_routing.rs` · `recovery_ops`

**Consumers.** codegen (__restart_<L>, __resume_<L>); model

**Invariants.**

- a recovery op is a row with a witness, per (parent, child)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/restart_in_place_params.rs; crates/hale-codegen/tests/restart_bound.rs

**Spec.** spec/semantics.md § supervision

**Guarded seams.**

- `recovery_ops(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×2

### `closures` — Migrating · law

**Answers.** Whether each closure clause is well formed, and which lifecycle events (`epoch`, `persists_through`, `resets_on`) it names.

**Inputs.** closure declarations; lifecycle_order (the event alphabet)

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_locus_member`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/closure.rs` · `EpochSpec::Birth` — event names are matched ad hoc in codegen; a clause naming an event the locus never reaches is a silent no-op. *Removed when:* the clause joins the lifecycle table and an unreachable event is a law violation with a witness.

**Consumers.** check; codegen

**Invariants.**

- closures are a consumer of the layer-6 alphabet (RFC §2)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/closure_resets_per_epoch.rs; crates/hale-types/tests/violate.rs

**Spec.** spec/semantics.md § closures

### `api_surface` — Migrating · derivation

**Answers.** The served surface: commands, reads, streams, their schemas, the roles that gate them, and the description's wire form.

**Inputs.** @export, @gated, serve declarations; the api binding; roles (--env)

**Producer (today's authority, migrating).** `crates/hale-syntax/src/api_gen.rs` · `api_surface`

**Legacy producers (permitted until removal).**

- `crates/hale-syntax/src/api_gen.rs` · `generate_api` — run by check with no roles and by build with roles, so check's description and model never carry what build --env bakes in; codegen runs it again. *Removed when:* one surface per snapshot, with the configuration as an input.
- `crates/hale-types/src/check.rs` · `check_api_roles` — role declarations, includes and gates are judged over the AST. *Removed when:* a law over the surface rows.
- `crates/hale-cli/src/verbs/check/matrix.rs` · `role_coverage` — re-runs the loader and reads pre-desugar programs. *Removed when:* reads the rows.

**Consumers.** check --dump-api (the snapshot's surface, the one its binding serves) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `api_surface`); build (the description the binding serves); describe / call / watch / admin; ui (reserved); bundle (reserved)

**Invariants.**

- form, not params (I1): the description names what the program is, never where one copy listens
- perspective-invariant (I5)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/api_description.rs; crates/hale-types/tests/api_binding_check.rs

**Spec.** spec/model.md § The description; spec/semantics.md § The api binding (GH #1106)

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

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_decorator_stacks`); claims (certificate, causes, depends, budget); the certificate evidence (the check's effects certificate report, read by the check's laws and the artifact's) (`crates/hale-frontend/src/snapshot.rs` · `demand_effect_certificates`); check (a codec binding's purity assertion reads the purity column, demanding the rows only when a codec reaches it) (`crates/hale-types/src/check.rs` · `CheckInputs`); model (effect labels, lower bounds and direct contributions, the last read by the reachability judgment's `effects(C)` test, and the summary the rows' walk read: the snapshot's rows, handed in) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); the effects manifest (`--dump-effects-manifest`, `--check-effects-manifest`: the snapshot's rows, cross-seed calls resolved through the renames) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_effects`); replay (the live-effects gate reads the manifest over the snapshot's rows, and refuses when they are blocked) (`crates/hale-cli/src/verbs/replay.rs` · `demand_effects`); doc

**Invariants.**

- derived ⊆ declared is the certificate; budgets are judged through evidence (spec/verification.md)
- effects run once per snapshot, not once per consumer: `Snapshot::demand_effects` runs `derive_effect_rows` once over the snapshot's allocation summary (`demand_alloc_summary`: the checked programs and the stdlib's analysis copy, cross-seed calls resolved through the import renames), counted as `effects`, blocked with the scope
- an unresolved edge is coverage, never a violation: a row's `effects` saturates to `UNCLASSIFIED` when the walk reaches what it cannot name, its `known` set is the lower bound an unresolved edge never erases, and `unknown` says the walk reached such an edge
- a row is keyed by the fn's name (`FnKey`) until the `snapshot_identity` family's declaration rows carry it
- a fn's direct contribution is a column (`direct`; `EffectRows::direct` answers any key, a bodyless one by what it carries): the model's function rows and absorbed paths, which the reachability judgment's `effects(C)` destination test reads, take it from the rows, and nothing outside the producer folds a body for it
- purity and the lower bound are columns of the rows: one walk answers a fn's saturating set and its lower bound (`infer_effect_bounds`), and the purity walk runs only inside the producer; the checker's codec law reads the purity column through `CheckInputs::effects`, a demand made only when a codec binding reaches the assertion, so a check of a program that binds no codec runs no effects fixpoint
- the effects certificate engine runs once per snapshot, in the check (`check_bundle_reporting`); the certificate evidence reads that report (`Snapshot::demand_effect_certificates`, handed to `derive_certificate_evidence_over`) and never runs the engine itself; a bundle no check ran over runs it once for itself (`effect_certificates`)

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/effect_assertions.rs; crates/hale-types/tests/effects_cross_seed_correction.rs; crates/hale-cli/tests/effects_baseline_gate.rs; crates/hale-cli/tests/effects_manifest.rs

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

### `blocking` — Migrating · derivation

**Answers.** Which fns block (a cooperative worker would be held), and whether the program places anything off the main thread.

**Inputs.** stdlib_surface (holds_cooperative_worker); the callgraph; placement

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `blocking_path_match`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `blocking_free_fns` — a name-keyed callgraph fixpoint for the BLOCK class, beside the effects fixpoint's own BLOCK propagation. *Removed when:* one fixpoint (the effects rows).
- `crates/hale-types/src/check.rs` · `blocking_self_methods` — the method half of the same fixpoint; no cross-locus hop. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `program_has_offthread` — codegen's predicate: its placement term calls the bus graph's `has_offthread_placement` (which walks modules); its bindings term scans top-level items only, so a module-nested main's socket binding is missed. *Removed when:* codegen reads the placement rows.
- `crates/hale-types/src/bus_graph.rs` · `has_offthread_placement` — the placement half of the same predicate, walking modules; a component of codegen's, not a second copy. *Removed when:* one placement table.

**Consumers.** check (rules 7, 8); effects (@no_block); codegen (mark_pinned, no_pinned dispatch)

**Invariants.**

- one leaf set (GH #830) and one propagation

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/placement.rs; crates/hale-codegen/tests/bus_devirt_no_pinned.rs

**Spec.** spec/semantics.md rules 7, 8

### `alloc_summary` — Migrating · derivation

**Answers.** Where each allocation lands and when it is reclaimed: per-fn allocation, escape, scratch eligibility, method-scratch elision, stack arrays, arena elision.

**Inputs.** bodies; signatures (non_allocating, fallible, ffi); the callgraph; ownership (accept sets)

**Producer (today's authority, migrating).** `crates/hale-types/src/alloc_summary.rs` · `derive_alloc_summary`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/alloc_summary.rs` · `ReclaimScope` — the checker's reclaim model, stale for scratch-local fns since #1208. *Removed when:* one model.

**Also owned.** `crates/hale-types/src/alloc_summary.rs` · `summarize_identified`; `crates/hale-types/src/alloc_routing.rs` · `derive_alloc_routing`

**Consumers.** the effects certificate engine (the check's `@effects`, `@phase_effects` and placement diagnostics: the snapshot's summary, handed in) (`crates/hale-types/src/check.rs` · `CheckInputs`); effects (the rows walk the snapshot's summary and hold it, shared) (`crates/hale-frontend/src/snapshot.rs` · `demand_effects`); check (the unbounded-allocation advisory and `--dump-alloc-summary`: the snapshot's summary) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_alloc_summary`); the editor's typing stage (the advisory in the diagnostics, `Config::alloc_advisory`: the snapshot's summary) (`crates/hale-frontend/src/snapshot.rs` · `typing_stage`); lsp (hale/allocSummary: the snapshot's summary) (`crates/hale-lsp/src/lib.rs` · `demand_alloc_summary`); model (its function rows, dispatch sites and holes: the snapshot's summary's own rows) (`crates/hale-types/src/model_builder.rs` · `own_rows`); topology (the artifact's fn sort, labels, derived effects and through-stdlib contraction: the snapshot's summary, handed in) (`crates/hale-types/src/topology.rs` · `dump_topology_over`); check (the hot-path lint: a law over the program's own rows and their columns, the snapshot's summary, handed in) (`crates/hale-types/src/check.rs` · `check_hot_path_alloc`); claims (@budget: the counting engines over the snapshot's summary's own rows, handed to the evidence) (`crates/hale-types/src/evidence.rs` · `derive_certificate_evidence_over`); codegen (arena routing at an allocation) (`crates/hale-codegen/src/codegen.rs` · `current_arena_ptr`); codegen (a free fn's scratch: the view's non-allocating and scratch-local rows) (`crates/hale-codegen/src/codegen.rs` · `alloc_routing`); codegen (a locus's arena, and its hooks', methods' and modes' scratch: the elision rows, a monomorph's specialized) (`crates/hale-codegen/src/codegen.rs` · `locus_elision`); resource_budget (the fd sites and the fd-leak warnings: the snapshot's summary's own rows) (`crates/hale-types/src/resource_budget.rs` · `own_rows`); frontier (the `causes:` engine: the summary's own rows) (`crates/hale-types/src/frontier.rs` · `causes_inner`); a declaration's dependents (X2: the call edges of the rows and of the declaration bodies) (`crates/hale-frontend/src/dependents.rs` · `declaration_bodies`)

**Invariants.**

- one summary per snapshot: `Snapshot::demand_alloc_summary` runs `derive_alloc_summary` once over the checked programs with the stdlib's analysis copy beside them (cross-seed calls resolved through the import renames), counted as `alloc_summary`, blocked with the scope; the check's effects certificate engine reads it (`CheckInputs::alloc_summary`) and the effect rows walk it, so neither builds its own; `summarize_identified` is the one constructor, and `derive_alloc_summary` (which places the stdlib's analysis copy beside the programs itself) its one caller outside tests, so no reader builds a summary variant of its own
- the checker's reclaim model and codegen's routing agree; a stale copy is a registry violation, not a comment
- lowering routes from the view's rows (`LoweringView::alloc_routing`, `derive_alloc_routing` over `merged` with the view's renames) and derives none: a free fn skips its per-call scratch when the rows call it non-allocating (FORM-3, a greatest fixpoint keyed by name), and its body allocates into its own subregion when they call it scratch-local (#1148); a locus's arena is elided, and a lifecycle hook, `fn` method or mode lowers without its per-call scratch, when its elision row says so (`LocusElision`, keyed by the locus and the member's position; a generic locus's monomorph, which lowering synthesizes, takes `AllocRouting::specialize` over the synthesized declaration); the FORM-3 classifier (`fn_body_definitely_non_allocating`) is the rows' own and lowering calls it nowhere; the two method-elision stages still classify a self field differently (stage 1, the per-member verdict, by its literal default or its ascription through aliases; stage 2, the `self.m()` sets, by a primitive ascription only), and a monomorph still has no stage-2 set, both as they were; a free fn's entry publishes its caller's arena to the caller-arena TLS when its row says so (`caller_arena_publish`: not non-allocating, and a call or a struct literal anywhere in its body, found by a structural walk, a string literal spelling `Call {` or `Struct {` counting as the Debug-string test it replaced counted it; a generic fn's monomorph takes `AllocRouting::specialize_fn`), and no body's Debug rendering decides it; the rows are #1208's classification moved as it was, so its builtin list is still a hand-kept subset of the checker's `BARE_BUILTIN_CALLEES` with no agreement test, its `std::` namespaces a per-namespace claim no stdlib_surface row states, and its qualified-path lookup checks the import renames before `PATH_RENAMES` where `resolved::lookup_qualified_path` checks them in the other order
- a body's escape tags key a binding by the declaration its escaping uses name (`Snapshot::declaration_of`), so an inner shadow of a returned name is its own, local binding (the #1140 shape), and close over `let x = y;` aliases to the declaration y names, as borrow_lifetime's `returned_decls` does; programs minted by different snapshots are summarized each with its own identities (`summarize_identified`: a bundle's programs beside the bundled stdlib's analysis copy)
- each seed's names resolve in its own scope: the programs minted with one set of identities are one scope, a body's bare free-fn name (and the locus a call's result is typed by) resolves only to a fn of its own scope, and the import renames are the bundle's names (the stdlib's analysis copy imports nothing), so a stdlib body's builtin `count(...)` is the builtin, never a user fn called `count` (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)
- the unbounded-invocation fixpoint (`AllocSummary::unbounded_invoked`) seeds from what the program reaches: beside the stdlib's analysis copy, `AllocSummary::reached` is the program's own fns, what their calls reach (the interface fan-out included) and the hooks and bus handlers of every locus they start (a struct literal of it in a reached body, or a param field of a started locus by declared type or default literal; every locus of the program's own is started), and only those seed it or call, so a stdlib loop the program never starts invokes nothing (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)
- the run-to-exit rule (a `main` and no long-lived entry: no leak sites) reads the program's own entries, never the stdlib's analysis copy's (`AllocSummary::analysis_copy` names the copy's fns, `is_own` the program's), since the copy always carries `run` hooks (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)
- a program's declaration is the row where the stdlib's analysis copy declares the same name: the copy's top-level free fn, locus or interface of a name a checked program declares stays out of the summary, so a stdlib source file checked as itself keeps its own rows (its bodies, its spans, resolved in its own scope); only stdlib source shares such a name (a classified correction, pinned in `alloc_summary_construction_correction.rs`)
- the rows carry what the hot-path lint reads: a fn's `@hot` and whether it is a mode (`FnSummary::hot`, `mode`), the order of the declarations (`decl_index`), where a site or a call is written (`in_loop`, which a `return` / `fail` payload does not reset as it resets `loop_depth`), a struct literal written as a whole statement (`AllocSite::bare_stmt`) or as the whole right side of a `self.<field> =` replace (`self_replace`, the statement's span; an in-place replace, which allocates nothing, is in `FnSummary::in_place_sites`, not `sites`), the `let` a call is the value of (`CallEdge::let_span`), how a call is spelled (`CallSpelling`), and the allocating receives (`CallEdge::allocating_recv`, the one list, which `@budget` reads too)
- the hot-path lint is a law over the rows (`check_hot_path_alloc`, over the summary the check is handed): the program's own fns, a mode aside, in declaration order, each finding where its site or call is written, a bus handler's at any depth, `@hot` an error and `@unbounded` silencing the advisory; no reader walks bodies for it, so it sees what the summary's walk sees (a publish, a bare block, `violate`, a recovery and `shm_write` included, which the lint's own walk skipped; an index expression's subscript included, walked as any operand is, as the lint's walk walked it, so a `@hot` fn keeps that rejection (a classified correction, pinned in `hot_path_alloc.rs`); a callee that is an expression of its own excluded, which it reached) and its diagnostics over the corpus, the targets and the lint's pins are the lint's, in order (pinned in `hot_path_alloc.rs`)
- a module-nested declaration is summarized like a top-level one: every declaration pass walks `module { … }` nesting (`flat_decls`; a module is a namespace, not an analysis boundary, GH #764), so a module-nested fn or locus member has a row and a call into it resolves, and the model holes out no module-nested body; the one body with no row is an `on_failure` handler, summarized as a declaration body (a classified correction, pinned in `alloc_summary_construction_correction.rs`)
- the advisory, `--dump-alloc-summary` and the editor's hale/allocSummary read the snapshot's summary and report the program's own rows judged over the whole of it (the stdlib's analysis copy and the renames included: a classified correction, pinned per target in `alloc_summary_correction.rs`); a leak site is the program's own (`AllocSummary::is_own`) and is left out of the advisory only when it has no author position (`AuthorPositions::has`: its span at or beyond `API_SYNTH_BASE`, or in a declaration the origin rows mark synthesized whose offset no source file owns), never by its owner's name; the check's warnings, the editor's diagnostics and the editor's hale/allocSummary decide with one function (`advisory_leak_sites`)
- the model projects the program's own rows; a call into the analysis copy is the unresolved row: the model, the `@budget` engines (`budget_check`, `quantitative`), the artifact's user rows, the frontier's `causes:` engine and the resource budget (its fd sites and fd-leak warnings) read the snapshot's summary through `AllocSummary::own_rows` (the program's fns and loci; a call resolved into the stdlib's analysis copy is the unresolved call by the method's bare name, the receiver kept; a dispatch keeps its alternatives among the program's own loci, renumbered in key order, through the copy's interface is no dispatch, and through the program's own with none of its conformers left is the dead site), which is the program-alone summary field for field, so the copy moves no `shape_hash`, the build and replay identity, and the copy's fd-acquiring bodies are not the program's resources (pinned per target in `alloc_summary_own_rows.rs`); what the model would gain from the copy's rows is a separate correction
- a body the program writes that is no row is a declaration body (`AllocSummary::declaration_bodies`, `DeclarationBody`): an `on_failure` handler, a params block's initializers, a constant's value, a `birth_check`, a perspective's fns, params and `stable_when`, a synthesized hook, each wherever it is declared (a `module { }`'s included, through `flat_decls`, as the rows are), and every subexpression a walk does not descend into (a callee that is no name; an index's subscript is the row's own, walked), each walked as a row is and keyed to the site of the declaration it is a member of; no judgment reads them, so the rows and every consumer's answer are what they were without them (X2, a review fix: `hale check` with every dump over the corpus, `tests/hale` and every `dna/**/main.hl` unchanged), and the declaration dependents relation reads their call edges as the declaration's own

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/hot_path_alloc.rs; crates/hale-types/tests/alloc_summary_construction_correction.rs; crates/hale-types/tests/alloc_summary_correction.rs; crates/hale-types/tests/alloc_summary_own_rows.rs; crates/hale-codegen/tests/scratch_local_free_fn.rs; crates/hale-codegen/tests/fn_nonalloc_add.rs; crates/hale-codegen/tests/method_scratch_elision.rs

**Spec.** spec/memory.md § Allocation routing; spec/styleguide.md

**Guarded seams.**

- `summarize_identified(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×4
- `allocating_recv(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×2
- `check_hot_path_alloc(` may be referenced from: `crates/hale-types/src/check.rs` ×2
- `derive_alloc_summary(` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/evidence.rs` ×1, `crates/hale-types/src/judgment.rs` ×1, `crates/hale-types/src/topology.rs` ×1, `crates/hale-types/src/resource_budget.rs` ×1
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
- its accept-set walk and its returned-binding walk are ownership residues, listed under `ownership` (borrow_lifetime.rs `accepts`, `returned_decls`)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/check_borrow_lifetime.rs

**Spec.** spec/semantics.md § A borrow outlives its holder; spec/decisions.md F.39

**Guarded seams.**

- `borrow_lifetime_diags` may be referenced from: `crates/hale-types/src/borrow_lifetime.rs` ×3, `crates/hale-types/src/lib.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1

### `bare_fallible` — Migrating · law

**Answers.** Whether a fallible call's error is addressed.

**Inputs.** expression types (Fallible); stdlib_surface (dual-mode calls)

**Producer (today's authority, migrating).** `crates/hale-types/src/bare_fallible.rs` · `bare_fallible_calls`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `check_expr_addressed` — the checker judges user fallibility while bare_fallible judges stdlib dual-mode calls the checker types as Unknown: two producers split by callee kind. *Removed when:* one law once stdlib calls are typed.

**Consumers.** check (`crates/hale-cli/src/verbs/check/run_impl.rs` · `bare_fallible_calls`); build, run, test, replay, bench (a build config's snapshot check, `Config::build_rules`) (`crates/hale-types/src/lib.rs` · `build_rule_diags`)

**Invariants.**

- runs on every entry point (not the LSP or bench today)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/check_strict_fallible.rs

**Spec.** spec/semantics.md § fallible

**Guarded seams.**

- `bare_fallible_calls(` may be referenced from: `crates/hale-types/src/bare_fallible.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1

### `nonreturning` — Migrating · law

**Answers.** Which `run()` bodies never return, which children are long-running, and whether the birth order or a pool starves because of it.

**Inputs.** run bodies; params order; placement

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `run_statically_nonreturning`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `check_nested_long_running_child` — a second `long-running` predicate (a hand table naming std::http::Server) that disagrees with the first. *Removed when:* one predicate.
- `crates/hale-types/src/check.rs` · `check_cooperative_pool_blocking` — the starvation and birth-order phases live inside the blocking check. *Removed when:* laws over rows.

**Consumers.** check

**Invariants.**

- one definition of long-running

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/birth_order_trap.rs; crates/hale-codegen/tests/birth_order_trap.rs

**Spec.** spec/semantics.md

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

### `placement` — Migrating · derivation

**Answers.** Which thread domain each instance runs in: pools, pinned threads, replicas, affinity, and the deployment plan.

**Inputs.** placement and topology blocks; entrypoint (the lowering root, never the entry); the construction templates: the root's literals, the entry's implicit construction of a root no literal builds, `fn main`'s own literals, the root's `bindings { }` adapters; the params towers each template builds; the minted sites of the snapshot and of the stdlib analysis copy; free fns and locus bodies (dynamic sites, their domains and bounds)

**Producer (today's authority, migrating).** `crates/hale-types/src/placement.rs` · `derive_placement`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/bus_graph.rs` · `collect_subscriber_placements` — per type, first wins. *Removed when:* same.
- `crates/hale-types/src/ownership_graph.rs` · `collect_placements` — a verbatim copy of the previous. *Removed when:* same.
- `crates/hale-types/src/model_builder.rs` · `PlacedIn` — the model's arrangement, per instance with replicas. *Removed when:* projected from the table.
- `crates/hale-types/src/resource_budget.rs` · `budget_for_programs` — counts placement entries, ignores replicas. *Removed when:* reads the table.
- `crates/hale-syntax/src/desugar.rs` · `collect_off_owner_thread_fields` — a desugar-time placement read. *Removed when:* reads the table.
- `crates/hale-codegen/src/codegen.rs` · `collect_main_placement` — codegen's DeploymentPlan, keyed by field name and locus type name. *Removed when:* codegen reads the table.
- `crates/hale-codegen/src/deployment.rs` · `DeploymentPlan` — the plan type lowering reads today. *Removed when:* becomes the layer-5 table.

**Consumers.** check (rules 2-5, 13-18); check (F.31: the caller per instance, the receiver by its row's `owner_relative`) (`crates/hale-types/src/check.rs` · `check_placement_single_thread`); check (the blocking check's three phases, blocking calls, pool starvation and the birth-order trap: where each of the deployed root's fields runs, its pool's `async_io`, and the declarations it realizes) (`crates/hale-types/src/check.rs` · `root_field_placements`); check (rule 17, pinned in a loop: the root's pinned rows, and each construction whose bound is built in a loop) (`crates/hale-types/src/check.rs` · `check_pinned_locus_in_loop`); check (type-check rule 20: the construction paths of a handler birth's enclosing locus, `OwnershipGraph::construction_paths` over the table handed in, derived only when that locus does not accept the child itself) (`crates/hale-types/src/check.rs` · `check_unowned_subscriber_locus`); sync_inference (accessor domains per instance) (`crates/hale-types/src/sync_inference.rs` · `infer_sync_for_bundle`); dispatch (domains); model (placed_in, affined_to); codegen (pools, mailboxes, affinity); lsp (hale/placement); deployment (reserved)

**Invariants.**

- placement is keyed by instance, never by type: one row per static instance of each construction template (a key is its origin, its field path, its replica), and a type's answer is the set of its instances' domains
- the entry is a construction scope: a root no literal builds is the entry's implicit template (`Origin::Entry`, bound `Once`), and `fn main`'s own literals are templates bound by their statement's loop context; an adapter is an origin of its own, built once
- nested rows inherit their owner's domain unless a root field's entry or a binding decides it; pinned domains are per anchor and per replica, pool domains one per name with at most one affinity
- the root is `lowering_root`, never the entry; an imported `main` is never the root
- every root entry decides exactly one field family in each construction template, or it is a hole
- unknown is a hole, not a default: an unresolved declaration, an unenumerable initializer, a held instance (`Reuse`) and a dynamic site of unknown domain each carry their policy, and none is main
- a held instance's subtree lives in its holder's domain: the held row keeps its `Reuse` hole and its owner's domain, the source's actual rows (never the declaration's defaults) are projected under it, inherited, and each of those rows names its own source row, the one it was built as (`built_by`); where the source is not linked, nothing below the held row is asserted, and an instance there runs in an unknown domain; a question of where an instance runs skips the source's rows, a count of instances skips the held ones
- every site the table names carries the universe that minted it (`SiteRef`); lowering joins the stdlib's into its merged mint once, totally and injectively
- the checker builds no table: the snapshot demands it and hands it to the check (`CheckInputs::placement`) and to the form rows; a bundle no snapshot holds (the test entries `check_bundle`, `check_bundle_opts_scoped`, `derive_application_model`, `effect_certificates`) builds it once, over a minted bundle (an unminted one names no site and gets an empty table, which judges nothing)
- a consumer asks where instances run of `PlacementTable::running` (the handed-off rows skipped): a type's answer is the domains of its instances, compared per instance, never collapsed to one per type; an enclosing locus with no static instance is a hole that disables the F.31 proof, never a default to main or to pinned
- F.38: placement is semantics-free, so a backend may Approximate it
- placement is a choice point: v1's declared placement is the single candidate

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/placement.rs; crates/hale-types/tests/placement_pairings.rs; crates/hale-codegen/tests/pool_affinity.rs; crates/hale-codegen/tests/placement_where_async_io.rs; crates/hale-types/tests/placement_table.rs (the table through the frontend's load: the correspondence's coverage cases 1 to 16, the two universes joined into lowering's mint, the table's laws over every clean fixture, and a check that demands it once); crates/hale-types/tests/shadow_placement.rs (the table against every legacy producer the snapshot reaches, over the corpus, tests/hale, the DNA seeds and the coverage fixtures, every divergence classified under the correspondence's rows and pinned per producer, rows and declaration); crates/hale-types/tests/form_rows.rs (sync inference per instance, K-5)

**Spec.** spec/semantics.md § Placement block (F.31); spec/decisions.md F.31, F.35, F.38

**Guarded seams.**

- `derive_placement(` may be referenced from: `crates/hale-types/src/placement.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/check.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/sync_inference.rs` ×1
- `collect_main_placement(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×2

### `target_capability` — Migrating · capability

**Answers.** What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability.

**Inputs.** --target; a source `target` declaration; stdlib_surface; FFI signatures

**Producer (today's authority, migrating).** `crates/hale-types/src/capability.rs` · `derive_capability_matrix`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `wasm_unavailable_stdlib` — a hand-kept slice-pattern table keyed by leading namespace, consulted only when the SOURCE declares `target wasm` (never from `--target wasm32`), and only for call forms. *Removed when:* one CapabilityMatrix consulted by the driver before lowering.
- `crates/hale-types/src/check.rs` · `wasm_target` — the source-declaration flag the table is gated on. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `link_wasm` — link-time refusals (link_libs) and the export list. *Removed when:* same.
- `crates/hale-codegen/src/locus/instantiation.rs` · `lotus_replay_start_ingress` — one of the per-site wasm skips; on wasm the eager main-locus spine emits the pool join where every other spine omits it, and every spine but the deferred entry emits wait-abort. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `is_wasm` — 31 sites read it: 14 are emission choices (a TargetSpec query, never a cell), the rest decide a behaviour, the link path or an obligation, each classified in the lowering shadow's site inventory. *Removed when:* emission configuration through TargetSpec only; every capability through the matrix.
- `crates/hale-types/src/check.rs` · `ffi_type_unportable` — FFI portability per type. *Removed when:* a capability row.
- `crates/hale-types/src/target.rs` · `TargetSpec` — has_async_io is true for wasm32; the checker sees the target only under `hale build`. *Removed when:* the matrix is the one statement, on every entry point.

**Consumers.** check; build; docs (systems/webassembly.md § What wasm32 can do, and spec/ffi.md's refused-namespace table: regions rendered from the cells) (`crates/hale-types/src/capability.rs` · `render_markdown`); the effective-target row, on the snapshot (consulted by nothing yet) (`crates/hale-frontend/src/snapshot.rs` · `demand_target`)

**Invariants.**

- Approximate is legitimate only in layers 5 and 7; everywhere else a target lowers or rejects, with the row's witness
- the docs' target statement is generated, never hand-maintained
- every (target class, capability), (target class, invocation) and (target class, obligation) pair has exactly one written cell: no default arm, the musl column written out like the others
- behaviours and obligations are distinct types: an obligation is omitted only on a premise (a Reject behaviour, a Refused invocation, a registered proof) that holds on its own target, and a Lower behaviour requires only Lower behaviours
- a capability refusal depends only on the program, the configuration and the target; a failure that depends on the machine running the compiler stays a toolchain error
- a target's class comes from its (arch, os, env), never from a triple's name; the effective target is a function of the snapshot key's configured target and its sources

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/wasm_target_gating.rs; crates/hale-codegen/tests/wasm_target.rs; crates/hale-cli/tests/target_model.rs; crates/hale-types/src/capability/laws.rs (the matrix's laws: one cell per pair, anchored witnesses, premises, requires, KNOWN_OPEN still today's answer); crates/hale-types/tests/shadow_capability.rs (the checker rows against their cells over the corpus, tests/hale, the DNA seeds and the wasm programs, on three columns: 0 divergences); crates/hale-codegen/tests/shadow_capability_lowering.rs (the codegen rows, the thread behaviours and @ffi("js") against their cells, both targets built; 7 classified divergences, all the design's: 3 PoolJoin, 4 @ffi("js") native links); crates/hale-cli/tests/shadow_capability_cli.rs (run, replay, record and --wrap-main against their cells; 1 classified divergence, T1's --wrap-main spelling); crates/hale-types/tests/capability_doc_matches.rs (both document regions equal the rendered matrix)

**Spec.** spec/decisions.md F.35; spec/ffi.md § The `target` declaration + stdlib gating; docs/src/systems/webassembly.md

**Guarded seams.**

- `wasm_unavailable_stdlib(` may be referenced from: `crates/hale-types/src/check.rs` ×2
- `derive_capability_matrix(` may be referenced from: `crates/hale-types/src/capability.rs` ×2, `crates/hale-types/src/capability/transport.rs` ×1, `crates/hale-types/src/capability/laws.rs` ×12
- `target_row(` may be referenced from: `crates/hale-types/src/capability.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

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

### `lifecycle_order` — Migrating · derivation

**Answers.** The happens-before order per instance: birth sequence, params open and settle, failure delivery and its execution domain, reclaim prerequisites, drain, restart, teardown.

**Inputs.** placement (the instance templates: the static towers, replicas apart, and the dynamic sites with their domains and bounds; the domains); handler_routing (the owner's handler for each child, and whether it restarts); flows (an accepted flow is reclaimed at its run's end, a resident by its owner's cascade); bus_graph (which instances subscribe); the declarations each template realizes (run, the closures' epochs, birth_check, violate sites, accept, on_failure, the params bracket); the decision lines (`hale_types::lifecycle::DECISION_LINES`) and the inventory rows they name; the runtime protocol (lotus_failure_hold / await / defer_reclaim)

**Producer (today's authority, migrating).** `crates/hale-types/src/lifecycle/derive.rs` · `derive_lifecycle`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/instantiation.rs` · `lower_locus_instantiation_inner` — the birth sequence is the order of emit calls in a 4,900-line function; its eager teardown spine is one of two copies. *Removed when:* an explicit action plan (compiler- and runtime-owned actions with domain, prerequisites, liveness, completion) read by emission.
- `crates/hale-codegen/src/codegen.rs` · `emit_deferred_entry_teardown` — the deferred teardown spine, the second copy; #1208's pool join was added here after four other sites already had it. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `emit_frame_teardown` — the frame flush order (drain, wait-abort, pinned-first, reverse push). *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `lower_program` — fn main's fall-through exit quiesces ingress and joins the pools before its flush, a third copy of the join-before-free order (the wait-abort comes from the flush's `in_main` gate). *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `main_test_fail_bb` — fn main's test-failure exit repeats the quiesce and the join before its frame teardown, a fourth copy. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `lower_return_inner` — a `return` from fn main repeats the quiesce and the join before its teardown, a fifth copy. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `__reclaim_` — the reclaim spine. *Removed when:* same.
- `crates/hale-codegen/src/locus/dissolve.rs` · `emit_locus_arena_destroy` — the cascade (field drains, field dissolves, arena destroy). *Removed when:* same.
- `crates/hale-codegen/src/locus/restart.rs` · `define_restart_fns` — restart and resume. *Removed when:* same.

**Also owned.** `crates/hale-types/src/lifecycle.rs` · `LifecyclePlan`; `crates/hale-types/src/lifecycle/trace.rs` · `Expected`; `crates/hale-types/src/lifecycle/project.rs` · `expected`

**Consumers.** codegen (emission reads the order); closures (the event alphabet); transitions (reserved); deployment (reserved)

**Invariants.**

- handlers run only on the queue owner's thread, so cross-thread failure delivery follows spec/runtime.md (a typed bus message): the first named decision, with its own regression test
- a spec/implementation disagreement is settled as a named decision, never by extraction picking a side
- an obligation is keyed by its source site (the declaration and P1's construction template); the runtime mints the instance and its incarnation, the table never does
- every obligation ends in exactly one of its named terminal alternatives; lifetime (what stays alive until which event) and progress (what makes it reach a terminal) are separate fields
- each rule says whether it is shipped, adopted, known open at an inventory row, or pending on a named condition
- the runtime's protocol is checked by its trace, not trusted: a trace build reports the hold, the settle and each held delivery, and the trace oracle holds them to the plan's edges (delivered after the owner's settle, before its birth), with negative controls that remove or reorder a step and fail it, over the lifecycle fixtures, every runnable example and every cell of the lifecycle matrix
- one plan per snapshot, over the placement table's instance templates: a held row and the rows projected under it are their source's and owe nothing of their own, a hole owes nothing, and a dynamic literal's own fields are instances of their field literals in its domains; no check builds the plan
- a failure's rows are guarded, one set per source an instance can raise (a birth-epoch closure, the birth_check, a violate in run(), in a handler or in drain(), a dissolve-epoch closure): its delivery in place, the held alternative where the owner's params can still be open, and the recovery decision and the restart, performed or refused under teardown; a path through the plan picks one
- existence, each edge and each domain claim carry their own rule and status, so a row shipped to exist can carry a claim known open (decision L0-1 at C36) or pending (line 3); a domain is claimed only where the placement table and the rule resolve one

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/lifecycle_plan.rs (the plan through the snapshot: demanded once and built by no check; its laws over every corpus program the snapshot scopes, edges naming its own rows and acyclic on events, birth and teardown owed once per template, every line a decision line, every delivery to an owner of known domain naming its domain; the rows lines 1, 3, 7, 12, 13, 14, 18 and RD are about); crates/hale-codegen/tests/lifecycle_flow.rs; crates/hale-codegen/tests/reclamation_spine.rs; crates/hale-codegen/tests/main_locus_deferred_pool_join.rs; crates/hale-codegen/tests/teardown_pinned_join_order.rs; crates/hale-types/src/lifecycle.rs (the schema's laws: every decision line binds a kind, the Pending lines are the named ones, the doc table is the data); crates/hale-codegen/tests/lifecycle_fixtures.rs (a fixture per decision line under tests/fixtures/lifecycle/; KNOWN_OPEN pins today's outcome where it differs from the adopted one; the trace oracle holds each run to its line's plan, TRACE_KNOWN_OPEN names today's departures, CONTROLS fail it; the producer's plan, rendered along each run's path, judges every run as the line's plan does, but UNDERIVED's, which it does not derive yet); crates/hale-types/src/lifecycle/trace.rs (the trace's parser and oracle); crates/hale-codegen/tests/corpus_oracle.rs (corpus_traces_keep_the_lifecycle_laws: every runnable example, traced); crates/hale-codegen/tests/lifecycle_matrix.rs (failure phase × tree position × domain, a generated program per cell held to its outcome, its trace plan, ASan on the sample and the let-bound differential; KNOWN_OPEN names today's failing cells; HALE_MATRIX=full runs every cell; the producer's plan judges each run as the generated one does, to the word)

**Spec.** spec/runtime.md (failure delivery; pool join rule b); spec/runtime.md § Lifecycle obligations (the decision lines, adopted and shipped told apart); spec/runtime.md § The lifecycle trace (a debug aid, not a contract); spec/semantics.md § lifecycle

**Guarded seams.**

- `derive_lifecycle(` may be referenced from: `crates/hale-types/src/lifecycle/derive.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

### `bus_inert` — Canonical · derivation

**Answers.** Whether the program can ever have a bus cell in flight, so drains can be elided.

**Inputs.** the user program's declarations (topics, perspectives, bus and bindings blocks, accepts); the names the user program spells (`hale_syntax::names`); the stdlib's bus-taint column (which `std::` namespaces reach bus surface)

**Producer.** `crates/hale-types/src/bus_inert.rs` · `bus_inert`

**Also owned.** `crates/hale-types/src/bus_inert.rs` · `bus_tainted_namespaces`; `crates/hale-syntax/src/names.rs` · `for_each_spelled`

**Consumers.** the resolved program (a row of the lowering view, over the user program before the stdlib merge) (`crates/hale-types/src/resolved.rs` · `bus_inert::bus_inert(`); codegen (drain elision: the row, read) (`crates/hale-codegen/src/codegen.rs` · `resolved.bus_inert`); codegen (`crates/hale-codegen/src/bus/runtime.rs` · `emit_bus_drain`)

**Invariants.**

- a drain elision is a conclusion of structural rows, never of a rendering: the declarations the user program carries, and the names it spells (`hale_syntax::names`, an exhaustive walk) against the stdlib's bus-taint column (`bus_tainted_namespaces`, a fixpoint over the stdlib's declarations by the same walk); lowering reads the verdict (`LoweringView::bus_inert`) and derives none
- the test is by name and over-approximates (a local spelled like a tainted namespace keeps the drains); a query over the message graph that follows which stdlib subscribers a program actually reaches would elide more, and is a separate change with its own shadow

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/drain_elision.rs; crates/hale-codegen/tests/log_routing.rs; crates/hale-types/src/bus_inert.rs (the_verdict_reads_declarations_and_names)

**Spec.** spec/runtime.md § drain

**Guarded seams.**

- `bus_inert(` may be referenced from: `crates/hale-types/src/bus_inert.rs` ×2, `crates/hale-types/src/resolved.rs` ×1

## Layer 8 — lowering

### `law_backstops` — Migrating · law

**Answers.** The checker rules lowering re-judges because `build_executable` never runs the checker: self-containment, cross-pool bare statements, placement entries, pinned loci in loops.

**Inputs.** the AST; the lowering context

**Producer.** none yet: the family has no authoritative producer today; the legacy list is the whole inventory.

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/instantiation.rs` · `CodegenError::Unsupported` — spanless refusals at lowering for rules the checker already states (rule 6's checker evaluator landed in phase 0; the backstop stays for harness builds that skip the checker); for a placed locus the checker types as Unknown, for an `accept()` with no parameter (the checker keys on `accept_param`, codegen on the method name), and for an adapter locus instantiated inline in a `bindings { }` block (which lowering pins without a placement entry), it is the only evaluator. *Removed when:* phase 3, when one pipeline guarantees the checker ran before lowering and the refusals become dead: every verb checks before it lowers, but the test harness's adapter `build_executable_with_options` builds through a harness snapshot that does not gate lowering on a check (`Config::harness`), and over three hundred test files build through it (at the phase-2 close).

**Consumers.** codegen harness builds

**Invariants.**

- a law is judged once, with a span

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/self_containing_locus.rs; crates/hale-codegen/tests/self_containing_locus.rs

**Spec.** spec/semantics.md rules 6, 17, 18; GH #813, #876

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
- the model builds none of the families it reads beside the program (2.3): the scope with its topic rows, the bus graph, the ownership graph, the handler rows and the effect rows (with the stdlib-merged summary their walk read) arrive as `ModelInputs`, each demanded once from the snapshot over the checked programs; the allocation summary and the placement it still re-runs for itself are listed under their families

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/demand_gate.rs; crates/hale-model/tests/architecture.rs; crates/hale-types/tests/topology_projection.rs

**Spec.** spec/model.md

**Guarded seams.**

- `derive_application_model(` may be referenced from: `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/judgment.rs` ×1, `crates/hale-types/src/topology.rs` ×2
- `derive_application_model_over(` may be referenced from: `crates/hale-types/src/model_builder.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

### `claims` — Migrating · law

**Answers.** Every user law: lowered claim rows, the judged verdicts over the model and evidence, constitution identities, and the artifact's law account.

**Inputs.** model; claim and constitution declarations; evidence (certificates, budgets); effects

**Producer (today's authority, migrating).** `crates/hale-types/src/judgment.rs` · `claim_law_diags`

**Legacy producers (permitted until removal).**

- `crates/hale-cli/src/topology_law.rs` · `validate_law_account` — the CLI recomputes the law digest and re-judges certificate and document verdicts when it admits an artifact: a second law authority. *Removed when:* admission validates ties and reads verdicts; it re-derives none.
- `crates/hale-cli/src/verbs/check/matrix.rs` · `constitution_identities` — re-runs the loader, the scope and the bus graph to re-derive identities the artifact already carries. *Removed when:* reads the artifact's section.
- `crates/hale-types/src/claims.rs` · `constitution_identities` — the identity derivation the matrix calls. *Removed when:* one derivation, projected.

**Consumers.** check / verify; the check's laws stage (judged over the snapshot's model, after its typing stage) (`crates/hale-frontend/src/snapshot.rs` · `demand_laws`); topology (law section); fleet; dna (dna_law.rs wording); model diff

**Invariants.**

- structural compiler laws are evaluated through model_query with shared witness rendering; the judgment path stays for user claims (final direction)
- a registered rule without an evaluator fails the compiler's own build
- a non-holds verdict is never silent

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/claim_diags_snapshot.rs; crates/hale-cli/tests/law_selection_reaches_the_artifact.rs; crates/hale-cli/tests/dna_law.rs; crates/hale-types/tests/one_reachability_engine.rs

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

### `snapshot_identity` — Migrating · derivation

**Answers.** The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again in the resolved-program step (numbered over the user program before the intra-locus rewrite, so the sends it records are numbered on every path, and minted over the merged program with the bundle's seeds and a named seed for the bundled stdlib), idempotently (one numbering; a later mint numbers only what an earlier one did not see), with reliable provenance; and which declaration each use names (`binding_of`), resolved once by the mint.

**Inputs.** seed_loading; desugar_sequence

**Producer (today's authority, migrating).** `crates/hale-types/src/snapshot.rs` · `mint`

**Legacy producers (permitted until removal).**

- `crates/hale-model/src/ids.rs` · `FunctionId` — model ids are ranks in a sorted string order (`L::f`, `(name, kind)`, path strings). *Removed when:* same.
- `crates/hale-types/src/effects.rs` · `FnKey` — analysis keys are (locus name, fn name). *Removed when:* same.
- `crates/hale-types/src/check.rs` · `type_expr_key` — rule 12 compares stringified TypeExprs. *Removed when:* same.

**Also owned.** `crates/hale-syntax/src/sites.rs` · `SiteKind`; `crates/hale-types/src/snapshot.rs` · `resolve_uses`; `crates/hale-types/src/snapshot.rs` · `declaration_of`; `crates/hale-types/src/snapshot.rs` · `number`

**Consumers.** every table; the shadow facility (compares through an explicit correspondence, never raw id equality); lsp (a later incremental future); the resolved program (codegen's input is minted over the merged program)

**Invariants.**

- addresses are not identities (declarations are cloned); spans are not (the stdlib's coordinates overlap user files; desugars share spans)
- snapshot-local uniqueness and provenance are the requirement; persistent identity across editor revisions is a separate problem
- canonical ids need real equality and hashing; the AST's structural NodeId equality stays separate
- the identity's types are hale_graph::ids (SeedId, SiteId; phase 1.1a); hale_types::snapshot::mint numbers every site the AST walk hale_syntax::sites reaches with one counter: after the entry point's last desugar, and again, idempotently, in the resolved-program step over the merged program, numbering only what an earlier mint did not see (phase 1.1b); before the intra-locus rewrite that step only numbers the user program (`snapshot::number`: the rewrite moves a send's id onto its call and records it), which makes no snapshot of it; every entry point calls it after its last desugar with its source map and the bundle carries the result (every verb and the LSP through the snapshot's load, the test harness through `Snapshot::from_program`, with no source map); the lowering view mints with the bundle's seeds, the stdlib's sites under the named seed snapshot::STDLIB_SEED (its spans overlap the first file's); a generated site records the desugar that made it (Snapshot::origins); codegen's generic instantiation keeps the template's id, and the F.39 pre-pass numbers nothing: an unminted Struct or Call is an error
- every identifier expression is a `Use` site and every name a declaration with no site of its own binds (a fn's or a hook's parameter, a match pattern's binding, a tuple `let`'s name, a `shm_write` binding) a `Binder` site, their ids on their `Ident`s (use-site identity, phase 2)
- `binding_of` is one resolution per use, keyed by identity, never by name or span: the mint resolves each `Use` site, and each `Assign` site's head, to the `Let`, `For` or `Binder` site it names under the checker's scoping (`check::ScopeStack`), once per snapshot (the load's, the lowering view's, the bundled stdlib's analysis copy's once per process: `demand_gate` pins it); a use that names no local binding has no row; every reader asks `Snapshot::declaration_of` and resolves nothing itself, and every entry point mints (`check_program` too)
- a check of a bundle no entry minted (`Bundle::new` over parsed programs, a library caller's or a test's) mints a copy of its programs once, with its source map, before any family is derived (`with_identities`, the no-snapshot adapter of `check_bundle` and `check::check_bundle`): its bus graph's sends and the intra-locus rewrite's relation, whose own numbering keeps those ids, name a send by one id, so rule 10's join answers on that path as on the snapshot's; a send with no id reaching the join is refused as an internal failure naming the send, never judged as queued

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding); crates/hale-codegen/tests/owner_table.rs; crates/hale-types/tests/snapshot.rs (each_use_resolves_to_the_declaration_in_scope); crates/hale-types/tests/demand_gate.rs (each_snapshot_resolves_its_uses_once); crates/hale-syntax/tests/sites.rs

**Spec.** spec/decisions.md F.39, F.40

**Guarded seams.**

- `mint(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×2, `crates/hale-types/src/stdlib_bodies.rs` ×1, `crates/hale-types/src/alloc_summary.rs` ×1, `crates/hale-types/src/sync_inference.rs` ×1

### `demand` — Canonical · derivation

**Answers.** Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, each member's own program beside the merged one, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph and handler rows, the effect rows, the model, the check in its two stages with the effects certificate report its typing produced, the lowering view), each at most once, blocking a family whose prerequisite reported errors.

**Inputs.** seed_loading; desugar_sequence; snapshot_identity; the config (target, api, api roles, environment, the check's rules); editor overlays (LSP); a consumer's request

**Producer.** `crates/hale-frontend/src/snapshot.rs` · `Snapshot`

**Also owned.** `crates/hale-frontend/src/snapshot.rs` · `demand_entry`; `crates/hale-frontend/src/snapshot.rs` · `demand_scope`; `crates/hale-frontend/src/snapshot.rs` · `demand_editor_scope`; `crates/hale-frontend/src/snapshot.rs` · `demand_bus_graph`; `crates/hale-frontend/src/snapshot.rs` · `demand_ownership_graph`; `crates/hale-frontend/src/snapshot.rs` · `demand_handlers`; `crates/hale-frontend/src/snapshot.rs` · `demand_effects`; `crates/hale-frontend/src/snapshot.rs` · `demand_effect_certificates`; `crates/hale-frontend/src/snapshot.rs` · `demand_model`; `crates/hale-frontend/src/snapshot.rs` · `demand_typing`; `crates/hale-frontend/src/snapshot.rs` · `demand_laws`; `crates/hale-frontend/src/snapshot.rs` · `demand_check`; `crates/hale-frontend/src/snapshot.rs` · `demand_lowering`; `crates/hale-frontend/src/snapshot.rs` · `from_program`; `crates/hale-frontend/src/snapshot.rs` · `SnapshotKey`; `crates/hale-frontend/src/snapshot.rs` · `declarations`; `crates/hale-frontend/src/snapshot.rs` · `declaration_dependents`; `crates/hale-frontend/src/dependents.rs` · `DependencyIndex`; `crates/hale-frontend/src/snapshot.rs` · `reusing_typing`; `crates/hale-frontend/src/snapshot.rs` · `typing_reuse`; `crates/hale-frontend/src/typing_reuse.rs` · `ReuseKey`; `crates/hale-frontend/src/typing_reuse.rs` · `plan`

**Consumers.** check (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_check`); the checker's rules (the handler rows: duplicate handlers, `@supervised`; the entry row: rule 1 and rule 9; the placement table: the F.31 rule per instance, the blocking check, pinned-in-a-loop, rule 20's construction paths; the bus graph: rules 7, 9 and 10; the ownership graph: rule 20), demanded before the check runs; the effect rows' purity column for a codec binding, demanded only when one reaches the assertion (`crates/hale-frontend/src/snapshot.rs` · `CheckInputs`); topology (`--dump-topology`, `--check-topology`, `--check-topology-shape`: one artifact of the snapshot's model) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `dump_topology_over`); the api description (`--dump-api`: the surface the snapshot's sequence generated the binding for) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `api_surface`); the model dump (`--dump-model`: the check's own model when it judged a law) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_model`); the build identity (build, run, replay: the model hash and the obs ids from the snapshot's model, the plan digest from its lowering view) (`crates/hale-cli/src/shared/options.rs` · `model_identity`); build (`crates/hale-cli/src/verbs/build.rs` · `demand_lowering`); run <file> and run <dir> (`crates/hale-cli/src/verbs/run.rs` · `demand_lowering`); test (a build config for the host, the dev profile) (`crates/hale-cli/src/verbs/test.rs` · `demand_lowering`); replay (the identity admitted before lowering) (`crates/hale-cli/src/verbs/replay.rs` · `demand_lowering`); bench (the driver an overlay on the bench file) (`crates/hale-cli/src/verbs/bench.rs` · `demand_lowering`); the test harness (`Snapshot::from_program`, lowering not gated on a check) (`crates/hale-codegen/src/codegen.rs` · `demand_lowering`); lsp (the first publication: one snapshot per publish pass, its typing stage) (`crates/hale-lsp/src/lib.rs` · `demand_typing`); lsp (the typing stage reuses the seed's last typed snapshot, `State::typed`) (`crates/hale-lsp/src/lib.rs` · `reusing_typing`); lsp (the second publication: the whole check, for the files the laws add to) (`crates/hale-lsp/src/lib.rs` · `demand_check`); lsp (every request loads the snapshot the diagnostics load, `editor_snapshot`, one per request) (`crates/hale-lsp/src/lib.rs` · `editor_snapshot`); lsp (definition, placement, the allocation survey) (`crates/hale-lsp/src/lib.rs` · `demand_scope`); lsp (completion, hover, references, enforcement) (`crates/hale-lsp/src/lib.rs` · `demand_editor_scope`); lsp (hale/busGraph) (`crates/hale-lsp/src/lib.rs` · `demand_bus_graph`); model (the effect rows, demanded with the graphs it reads) (`crates/hale-frontend/src/snapshot.rs` · `demand_effects`); the effects manifest (`--dump-effects-manifest`, `--check-effects-manifest`) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_effects`); replay (the live-effects gate) (`crates/hale-cli/src/verbs/replay.rs` · `demand_effects`); the check's laws (their certificate evidence reads the typing's effects certificate report) (`crates/hale-frontend/src/snapshot.rs` · `demand_effect_certificates`); topology (the artifact's law evidence reads the same report) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_effect_certificates`); lsp (documentSymbol: the open file's member program, its own declarations as written) (`crates/hale-lsp/src/lib.rs` · `snap.member(`)

**Invariants.**

- every editor request reads the snapshot: the outline reads the open file's member program (`Snapshot::member`, the file as it parsed, before the merge and the sequence), so it answers while another member does not parse or an import does not resolve (the editor's load keeps the members of a seed whose link it refused, every family blocked) and lists only what the file itself declares; a file that does not parse has no outline
- the check is two stages and their composition (F.40 phase 3, X1): `demand_typing` is everything that needs no model — the scope's and the typing's diagnostics finished (`finish_check_diags`: the user's spelling, no repeat), then the build rules where the config asks (`Config::build_rules`: a build's and the editor's) and the allocation advisory where it asks (`Config::alloc_advisory`: the editor's; `hale check` runs its own beside its reports, under its flag); `demand_laws` is the laws judged over the model when the typing's own diagnostics denote one and the program has a claim surface, finished after the typing's own (`finish_check_diags_after`), and empty otherwise; `demand_check` is the typing stage's diagnostics followed by the laws', so every entry point's set is one pass's over both by construction, and only the position of a build rule or an advisory relative to a law differs from the single pass it replaced (both stages before; the laws last). Each stage is counted once (`Snapshot::builds`, the `STAGES` beside the families)
- the editor publishes the two stages apart (X1, `check_and_publish`): the first publication is every file of the seed with the typing stage, placed as `hale check` places it (each diagnostic spelled, suppressed — `retain_owned_advisories`, a stdlib-origin span — and placed on its own), with every file the seed's last publication covered and this one does not published empty (`State::published`); the second is the whole check (`demand_check`, the laws after the typing), sent only for the files whose list it changes, so a file's final list is its first followed by the laws placed in it, every file's last publication is `hale check`'s for it, and a seed with no broken law gets one publication; a seed the snapshot does not check (a refused load, a hole, the stdlib cache) gets its one. Before each publication, and before the laws are judged, the pass asks whether a document event is queued behind it: if one is, the buffers it read are superseded, what is unsent is discarded with the rest of the pass, `published` keeps only what was sent, and the pass's files join the next pass (`State::pending`)
- a prerequisite runs once: every family is a `OnceCell` of its snapshot, and a family that reads another demands it rather than building its own; `Snapshot::builds` counts each family's demands on the snapshot (its producer's runs in the snapshot's own cell), and no count exceeds one on any consumer on the snapshot. It does not count what is rebuilt outside those cells: the lowering view's `resolve_rewritten` builds its own top scope, ownership graph, bus graph and handler rows, so on a build path `build_top_scope` runs more often than `builds()` says. Those rebuilds are the legacy rows `check_bundle_opts_scoped` (top_scope), `build_ownership_graph` (ownership) and the resolved program's `build_bus_graph` (bus_graph), and the resolved program's reference in the `handler_rows(` seam (handler_routing)
- a family nobody requested is not computed: the no-claims editor path builds no model (GH #476 criterion 1); the bus graph (rules 7, 9 and 10) and the ownership graph (rule 20) it builds are the check's, once each
- the model's inputs are families (2.3): `demand_model` demands the scope, the bus graph, the ownership graph, the handler rows and the effect rows over the checked programs (the `bus_graph`, `ownership`, `handler_routing` and `effects` counts), each once; lowering's graphs are the lowering view's own, over the resolved program, until the check runs over it
- the checker's inputs are families (2.3): the typing demands the handler rows, the entry row and the ownership graph before the checker runs and hands them in (`CheckInputs`), so a check with a law builds the handler rows and the ownership graph once for the checker and the model together; the three producers read declarations and bodies, not types, so they are total over a program that does not typecheck
- a family whose prerequisite reported errors is `Blocked { family, because }`, not computed: an editor seed with a member that did not parse or would not read has no scope (the editor's requests read `demand_editor_scope`, the scope over the members that parsed with the hole named, and never a scope of their own), a program that does not typecheck has no model, a program whose check reported an error has no lowering view; a ready result may still hold typed holes
- the lowering view (`LoweringView`, the `lowering_view` count) is a family: `demand_lowering` demands the check, then runs `resolve_rewritten` once over the intra-locus stage (`demand_intra_locus`, the `intra_locus` count, which the check demanded too) with the snapshot's source map, renames and api config; `build_resolved` reads it by reference; every build path (build, run, test, replay, bench) demands it from a `Snapshot::load`, and the test harness from `Snapshot::from_program`, whose config (`Config::harness`) does not gate lowering on the check
- a changed entry, load mode, target, config, overlay or source text is a distinct snapshot (`SnapshotKey`, computed after the load from what it read; a bare program is its own load); two snapshots share no result but the typing reuse below, which a second key names
- the editor's typing stage is incremental by declaration and equals the full one by construction (F.40 phase 3, X2, `Snapshot::reusing_typing`, `hale_frontend::typing_reuse`): the snapshot key stays the whole seed's; a second key (`ReuseKey`: the entry, the load mode, the target, the config digest, the import renames as a set) names the reuse, and a previous snapshot whose second key differs, or whose typing never ran, offers nothing. The check's per-declaration passes (the checker's walk of a top-level declaration and the reveal rule's, `check::check_bundle_by_declaration`, `DeclChecked`) keep their result per declaration; a new snapshot of the seed reuses it for every declaration that is unchanged (the previous one moved by the distance its start moved is the new one, position for position), is no dependent of a changed one through the families (`declaration_dependents`, in the previous snapshot and the new one) and has no diagnostic position outside itself, moved the same distance, and runs the passes for the rest; a changed declaration is one whose bodies alone changed (equal once positions are erased and bodies set aside), and any other change (a declaration added, removed, renamed or reordered, an edit to what a declaration declares, a changed declaration the families do not place, a changed set of programs) checks the seed whole (`TypingReuse::Whole`, with the reason). Everything else the typing stage runs (the scope, the bundle-wide rules, the build rules, the advisory) runs whole, so the incremental stage's diagnostics are the full stage's, ordered and deduplicated as one pass leaves them; the laws stage stays whole. Reuse is opt-in and only the LSP opts in (`State::typed`, one typed snapshot per seed, kept across a hole), so `hale check` runs the whole check. The check records what it typed as it walks, so a typing that reused a declaration holds no record of its bodies, and the typed-body table of such a snapshot (`demand_typed_bodies`) is packaged from a whole check; the editor never lowers
- the bundle is a borrowed view (`Snapshot::bundle`), built per call, never stored
- a declaration's dependents are the families' (F.40 phase 3, X2, `Snapshot::declaration_dependents`, `hale_frontend::dependents`), never the text's: the unit is a top-level declaration of a snapshot program (`Snapshot::declarations`, a module one, named by its own minted site); for a fn or a locus the answer is itself, every declaration whose resolved calls reach it (the callgraph whose targets the effect rows copy, read from the allocation summary's call edges: the rows' and the declaration bodies', which are every body the reveal rule reads that is no row, each its declaration's own; a call to a fn with no row by its bare name; its readers closed transitively), and its neighbours in the ownership graph (births, accepts, instantiations), the bus graph (one subject's publishers, subscribers and topic), the placement table (what an instance or a dynamic site realizes, where its literal sits, its owner's and its enclosing scope's, joined by site) and the flow rows (a flow child and its `release` owners); a name joins every declaration that declares it, so a shared name answers both; a placement hole that leaves its literal's declaration unresolved makes the declaration it sits in a dependent of every one, and so does a call the summary records no edge for (by its span, over the site walk: a call in a position no summary body covers, today a closure's clauses and a type's field defaults, neither of which the reveal rule reads); any other declaration, one with no site, and a site that names none is `Dependents::Whole`. The relation's domain is an edit to a declaration's bodies: over the example corpus, every body edit of a mutation (emptied, an ill-typed `let`, a print, a call added) re-derives typing diagnostics only in declarations the relation names before the edit or after it, and an edit to a declared surface reaches a reader no family names, which is why such an edit is the seed's

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/demand_gate.rs; crates/hale-types/tests/declaration_dependents.rs (a_declarations_dependents_cover_what_a_fresh_check_rederives, an_edit_to_a_declared_surface_escapes_the_families); crates/hale-cli/tests/lsp.rs (the_check_is_its_typing_stage_followed_by_its_laws_stage, lsp_and_check_agree_over_a_seed_with_laws, lsp_publishes_the_typing_stage_first_and_the_laws_replace_it, the_incremental_typing_stage_is_the_full_one, the_incremental_typing_stage_is_the_full_one_over_the_dna_host, the_incremental_typing_stage_is_the_full_one_through_handler_and_initializer_calls, lsp_publishes_what_check_reports_after_a_helper_edit_through_handler_and_initializer_calls); crates/hale-cli/tests/lsp_latency.rs (lsp_latency_first_and_final_publication: the editor's latency, first and final publication apart; a measurement, ignored by default); crates/hale-frontend/src/snapshot.rs (a_changed_input_is_a_distinct_snapshot_and_shares_no_result, a_file_that_does_not_parse_blocks_the_scope_and_its_dependents, the_editor_scope_covers_the_members_that_parsed_and_names_the_hole, a_member_that_will_not_read_blocks_the_scope, a_program_that_does_not_typecheck_blocks_the_model, a_check_with_errors_blocks_the_lowering_view, the_lowering_view_is_resolved_once_after_the_check); crates/hale-lsp/src/lib.rs (a_request_builds_only_the_families_it_reads, a_request_over_a_seed_with_a_hole_answers_from_the_members_that_parsed, the_laws_replace_the_typing_stage_unless_a_newer_event_supersedes_them, a_pass_reuses_the_seeds_last_typed_snapshot)

**Spec.** spec/decisions.md F.40; RFC #1212 § phase 2 (demand and readiness)

**Guarded seams.**

- `Snapshot::load(` may be referenced from: `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1, `crates/hale-cli/src/verbs/build.rs` ×1, `crates/hale-cli/src/verbs/run.rs` ×1, `crates/hale-cli/src/verbs/test.rs` ×1, `crates/hale-cli/src/verbs/replay.rs` ×1, `crates/hale-cli/src/verbs/bench.rs` ×1, `crates/hale-lsp/src/lib.rs` ×1
- `Snapshot::from_program(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×1

### `digests` — Migrating · digest

**Answers.** Every identity a build or an artifact carries, and what each covers: shape_hash, artifact_digest, model_hash, exec_digest, the toolchain and cache keys, source digests, and the snapshot key they were derived under.

**Inputs.** the model half; the artifact; sources; BuildOptions; compiler sources; the snapshot key

**Producer (today's authority, migrating).** `crates/hale-types/src/topology.rs` · `model_shape_hash`

**Legacy producers (permitted until removal).**

- `crates/hale-cli/src/shared/options.rs` · `exec_digest` — the replay identity: HALE_TOOLCHAIN_SHA256 + version + options fingerprint + plan digest + sources; its logical source paths fall back to file names; build and run fingerprint `debug` differently, so a build's recording never replays. *Removed when:* one stated coverage, with tests that a covered change moves it.
- `crates/hale-cli/build.rs` · `toolchain_digest` — the replay identity: `hale_graph::identity::identity_files`, every identity-covered crate (`COVERED_CRATES`, hale-cli among them) and the manifest files (`Cargo.lock`, the ts-shim manifest), walked through the one shared walk. *Removed when:* phase 3, one stated coverage with `exec_digest`: the CLI's `build` verb still owns semantic work (its snapshot's config from the flags, the `[ffi]` pickup, the identity it stamps; see `identity_files`), so the replay identity still walks the compiler sources at build time, the CLI's crate among them (at the phase-2 close).
- `crates/hale-cli/src/shared/stale.rs` · `compute_codegen_src_hash` — the stale-binary hash: codegen.rs, lotus_arena.c and every stdlib .hl seed, walked identically at build and run time through the shared walk. *Removed when:* one identity per snapshot; the stale check reads it.
- `crates/hale-iris/build.rs` · `identity_files` — the DNA toolchain cache key's compiler-source half: the replay identity's selection, every identity-covered crate and the manifest files; hale-cli is covered because the cache builds a host through its `build` verb, whose Rust still owns its snapshot's config from the flags, the `[ffi]` pickup and the identity it stamps (the load and the pre-check sequence are hale-frontend's since 2.2b), until that work moves into hale-frontend. *Removed when:* the cache key is derived from the snapshot identity.
- `crates/hale-iris/src/lib.rs` · `toolchain_hash` — the cache key itself (version, compiler sources and manifests, stdlib, embedded iris and DNA trees). *Removed when:* the cache key is derived from the snapshot identity.
- `crates/hale-dna/src/digest.rs` · `EMBEDDED_DIRS` — DNA's embedded-source identity, its own directory list. *Removed when:* one inventory of what each identity covers.
- `crates/hale-types/src/evidence.rs` · `analysis_inputs_digest` — the evidence inputs digest (semantics version, stdlib source, compiler version, renames, the surface registry). *Removed when:* same.
- `crates/hale-frontend/src/snapshot.rs` · `b.sources` — per-file FNV digests, set by the snapshot, rooted at hale.toml for every load mode, the editor's and its requests' included. *Removed when:* one source map per snapshot.
- `crates/hale-model/src/obs_ids.rs` · `fn digest` — the observed entity-id digest, keyed by (kind, name). *Removed when:* keyed by snapshot identity.

**Consumers.** replay (admission); topology / fleet (admission); dna (schema 1.19, semantics 2, shape_hash, artifact_digest); the runtime obs header; the DNA host cache

**Invariants.**

- external contracts are frozen through extraction: additive and unhashed sections are free; hash and replay identity change only through explicit versioned transitions with an exact diagnostic (#476's rule)
- a build's identities read one snapshot (2.3, `model_identity`): the model hash (P26) is the snapshot model's `shape_hash`, read from the model (`project_shape_hash`, the value its artifact stamps, never scraped from a rendered artifact), the obs ids are that model's entities, and the plan digest `exec_digest` frames is its lowering view's plan; beside them the snapshot key (`SnapshotKey`: the entry, the load mode, the target, the config digest, the overlay digest, the digest of the source text read) names the load all three were derived from. The key is snapshot-local: no binary or recording carries it
- a semantic producer moving between crates never makes a later edit invisible to cache or replay identity: the replay identity and the cache key fold one selection, every identity-covered crate (the CLI among them until hale-frontend owns its semantic work) and the manifest files; the stale-binary hash is a cheap warning over codegen.rs, the runtime and the stdlib seeds by design

**Missing data.** n/a

**Focused tests.** crates/hale-cli/tests/obs_model_hash.rs; crates/hale-cli/tests/model_diff.rs; crates/hale-cli/tests/replay_cli.rs; crates/hale-cli/tests/stale_dna_warning.rs; crates/hale-cli/tests/source_map.rs

**Spec.** spec/model.md § Identity and versioning

## Spec rules and their evaluators

A registered rule without an evaluator fails the compiler's own build.

| rule | gist | family | evaluator | state |
|---|---|---|---|---|
| semantics/placement/1 | `placement { }` is main-locus-only | `placement` | `crates/hale-syntax/src/parser.rs` · ``placement` block is only valid inside` | Canonical |
| semantics/placement/2 | keys name main-locus params fields | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | Canonical |
| semantics/placement/3 | field values are locus types | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | Canonical |
| semantics/placement/4 | at most one entry per field | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | Canonical |
| semantics/placement/5 | pool names are identifiers; `main` always exists | `placement` | `crates/hale-syntax/src/parser.rs` · `parse_placement_block` | Canonical |
| semantics/placement/6 | pinned-class restrictions (no accept(), no closure whose epoch is birth or dissolve, the default) at the placement entry | `placement` | `crates/hale-types/src/check.rs` · `is placed `pinned` but` | Migrating |
| semantics/placement/7 | dead bus receiver on a cooperative pool is an error | `blocking` | `crates/hale-types/src/check.rs` · `check_cooperative_pool_blocking` | Migrating |
| semantics/placement/8 | a blocking syscall on a cooperative pool is a warning | `blocking` | `crates/hale-types/src/check.rs` · `check_cooperative_pool_blocking` | Migrating |
| semantics/placement/9 | orphan bus topic (closed world) | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_graph` | Migrating |
| semantics/placement/10 | bus cycles: a queued cycle warns, an unconditional intra-locus cycle of direct calls is an error | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_cycles` | Migrating |
| semantics/placement/11 | bus backpressure heuristic | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_backpressure` | Migrating |
| semantics/placement/12 | one literal subject, one payload type | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_subject_types` | Migrating |
| semantics/placement/13 | degenerate `pinned(cores = ..)` is an error | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | Canonical |
| semantics/placement/14 | topology consistency and node/l3 resolution | `placement` | `crates/hale-types/src/check.rs` · `check_topology_block` | Canonical |
| semantics/placement/15 | `replicas = K`: K >= 1, pinned only | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | Canonical |
| semantics/placement/16 | pool affinity agrees per pool | `placement` | `crates/hale-types/src/check.rs` · `check_pool_affinity` | Migrating |
| semantics/placement/17 | a pinned locus is not instantiated in a loop | `placement` | `crates/hale-types/src/check.rs` · `check_pinned_locus_in_loop` | Migrating |
| semantics/placement/18 | every placement entry is consumed exactly once | `placement` | `crates/hale-types/src/check.rs` · `check_placement_entry_consumed` | Migrating |
| semantics/placement/19 | a bus payload is carriable | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_payload_carriable` | Migrating |
| semantics/placement/20 | a subscriber born in a bus handler is owned | `ownership` | `crates/hale-types/src/check.rs` · `check_unowned_subscriber_locus` | Canonical |

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
| `crates/hale-types/src/check.rs` | `format!("{:?}", kind)` | 1 | decides (`blocking`) |
| `crates/hale-types/src/check.rs` | `format!("{:?}", p)` | 1 | decides (`snapshot_identity`) |
| `crates/hale-types/src/check.rs` | `format!("{:?}({})", class, type_expr_key(inner))` | 1 | decides (`snapshot_identity`) |
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
