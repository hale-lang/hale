# The graph registry

GENERATED from `crates/hale-graph/src/registry.rs` and held byte-equal by `registry_matches_spec`. Do not edit: change the table and run `HALE_REGEN_REGISTRY=1 cargo test -p hale-graph --test registry_matches_spec`. The contract this index serves is `spec/model.md` § *The graph registry*.

44 families: 4 canonical, 36 migrating (with 153 permitted legacy producers), 4 reserved. 19 spec rules with evaluators. 34 frozen Debug-string sites, of which 14 decide a fact.

## Families

| family | layer | state | kind | producer | legacy | answers |
|---|---|---|---|---|---|---|
| `seed_loading` | Layer 1 | Canonical | desugar | `collect_checkable` | 0 | Which source units form the snapshot: the entry, every imported seed, their merge order and the spans' virtual bases. |
| `qualified_names` | Layer 1 | Migrating | desugar | `resolve_imports` | 5 | What a qualified or aliased name denotes: the library identity, the mangled declaration, the construction target, the bus subject a path names. |
| `desugar_sequence` | Layer 1 | Migrating | desugar | `desugar_before_check` | 2 | Which rewrites the program receives before checking, in which order: the declaration-shaping passes only (JSON parsers, the api surface, sync inference, unit returns, construction aliases, the omitted `run`, repr accessors). The topic-reference and intra-locus rewrites are not desugars: they erase a written declaration reference the checker's laws and the model read, and run in lowering's resolved program, after the check. |
| `sync_inference` | Layer 1 | Migrating | derivation | `infer_sync_for_bundle` | 3 | Which sync discipline each `@form(hashmap)` slot gets when the author declared none, from the pools its methods are called from. |
| `effect_class_table` | Layer 1 | Migrating | derivation | `EffectTable` | 2 | The union of user effect classes across seeds, with `User(i)` indices remapped so one class has one index. |
| `top_scope` | Layer 2 | Migrating | derivation | `build_top_scope` | 2 | What every top-level name denotes: the symbol table over the merged program. |
| `expression_typing` | Layer 2 | Migrating | derivation | `check_bundle_scoped` | 1 | The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from. |
| `generics` | Layer 2 | Migrating | derivation | `unify_generic_ty` | 3 | Which monomorph a generic call instantiates and how its bindings unify. |
| `surfaces` | Layer 2 | Migrating | law | `check_structural_impl` | 2 | Which surface is visible at which depth edge: contract exposure, interface conformance, perspective designation and `serves` conformance. |
| `forms` | Layer 2 | Migrating | law | `check_form_shape` | 1 | Whether a form's shape, its capacity slots and its projection class are well formed, and which operation set closes each slot. |
| `stdlib_surface` | Layer 2 | Migrating | capability | `signature_for` | 8 | What each stdlib function is: its signature, its effect classes, whether it blocks, and what a value of a type can be rendered as. |
| `entrypoint` | Layer 3 | Migrating | derivation | — | 12 | Which locus is the program's `main`, whether the world is closed, and which declarations are imported. |
| `ownership` | Layer 3 | Migrating | derivation | `resolve_owners` | 8 | Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array. |
| `bus_graph` | Layer 3 | Migrating | derivation | `build_bus_graph` | 6 | The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates. |
| `topics` | Layer 3 | Migrating | derivation | `topic_wire_subjects` | 3 | What each topic is on the wire: its subject, payload contract, routing key, bounds and shed policy; and which topic a send's subject names. |
| `bindings` | Layer 3 | Migrating | derivation | `check_main_and_bindings` | 5 | Which topics are bound to which transport, in which role, with which codec, and whether the transport can carry the payload. |
| `dispatch` | Layer 3 | Migrating | derivation | `fn derive` | 2 | How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement. |
| `handler_routing` | Layer 3 | Migrating | derivation | `handler_rows` | 6 | Which `on_failure` handler a failing child's locus type reaches, and from which parent. |
| `flows` | Layer 3 | Migrating | derivation | `survey` | 2 | Which children are flows (released per completion) and which are resident. |
| `restart` | Layer 3 | Migrating | derivation | `handler_rows` | 2 | Which loci declare restart operations, which restart in place, and what the restart bound is. |
| `closures` | Layer 3 | Migrating | law | `check_locus_member` | 1 | Whether each closure clause is well formed, and which lifecycle events (`epoch`, `persists_through`, `resets_on`) it names. |
| `api_surface` | Layer 3 | Migrating | derivation | `api_surface` | 3 | The served surface: commands, reads, streams, their schemas, the roles that gate them, and the description's wire form. |
| `sealability` | Layer 3 | Migrating | law | `check_sealed_access` | 1 | Which loci confine their state (`@sealed`), and which could. |
| `runs_under` | Layer 3 | Reserved | derivation | — | 0 | On whose authority a locus runs: the relation `runs_under(locus, principal)`, with principals declared by the program. |
| `transitions` | Layer 3 | Reserved | derivation | — | 0 | For an evented locus: the transition each handler is, input event to output set (F.41, after phase 2). |
| `effects` | Layer 4 | Migrating | derivation | `infer_effects` | 6 | Which effect classes each fn and locus reaches (the callgraph fixpoint), the declared classes and their `causes:`/`depends:` DAG, and the certificate relating the two. |
| `blocking` | Layer 4 | Migrating | derivation | `blocking_path_match` | 4 | Which fns block (a cooperative worker would be held), and whether the program places anything off the main thread. |
| `alloc_summary` | Layer 4 | Migrating | derivation | `summarize_programs` | 14 | Where each allocation lands and when it is reclaimed: per-fn allocation, escape, scratch eligibility, method-scratch elision, stack arrays, arena elision. |
| `borrow_lifetime` | Layer 4 | Canonical | law | `borrow_lifetime_diags` | 0 | Whether a borrowed handle outlives its holder (GH #730), decided from position over the owner structure. |
| `bare_fallible` | Layer 4 | Migrating | law | `bare_fallible_calls` | 1 | Whether a fallible call's error is addressed. |
| `nonreturning` | Layer 4 | Migrating | law | `run_statically_nonreturning` | 2 | Which `run()` bodies never return, which children are long-running, and whether the birth order or a pool starves because of it. |
| `working_set` | Layer 4 | Canonical | derivation | `compute_program_working_set` | 0 | The estimated working set per locus and program, and the locality law over it. |
| `placement` | Layer 5 | Migrating | derivation | `compute_pool_of_locus_type` | 8 | Which thread domain each instance runs in: pools, pinned threads, replicas, affinity, and the deployment plan. |
| `target_capability` | Layer 5 | Migrating | capability | — | 7 | What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability. |
| `deployment` | Layer 5 | Reserved | derivation | — | 0 | A deployment as typed rows: root and horizon, component identities, instances and incarnations, resources and allocations, endpoints and routes, hosting and authority, persistence obligations (the habitat, after phase 2). |
| `lifecycle_order` | Layer 6 | Migrating | derivation | — | 10 | The happens-before order per instance: birth sequence, params open and settle, failure delivery and its execution domain, reclaim prerequisites, drain, restart, teardown. |
| `bus_inert` | Layer 6 | Migrating | derivation | — | 3 | Whether the program can ever have a bus cell in flight, so drains can be elided. |
| `law_backstops` | Layer 8 | Migrating | law | — | 1 | The checker rules lowering re-judges because `build_executable` never runs the checker: self-containment, cross-pool bare statements, placement entries, pinned loci in loops. |
| `model` | The law engine | Canonical | derivation | `derive_application_model_over` | 0 | The canonical semantic model of a checked bundle: fifteen entity tables, seventeen relation tables, holes, capabilities, provenance (GH #476). |
| `claims` | The law engine | Migrating | law | `claim_law_diags` | 3 | Every user law: lowered claim rows, the judged verdicts over the model and evidence, constitution identities, and the artifact's law account. |
| `view` | The law engine | Reserved | derivation | — | 0 | A named query over the tables: a node selector, a relation set and an adequacy policy, rendered by a backend (hale ui, after phase 2). |
| `snapshot_identity` | Identity | Migrating | derivation | `mint` | 4 | The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again in the resolved-program step (over the user program before the intra-locus rewrite, so the sends it records are minted on every path, and over the merged program with the bundle's seeds and a named seed for the bundled stdlib), idempotently (one numbering; a later mint numbers only what an earlier one did not see), with reliable provenance. |
| `demand` | Identity | Migrating | derivation | `Snapshot` | 1 | Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph and handler rows, the model, the check, the lowering view), each at most once, blocking a family whose prerequisite reported errors. |
| `digests` | Identity | Migrating | digest | `model_shape_hash` | 9 | Every identity a build or an artifact carries, and what each covers: shape_hash, artifact_digest, model_hash, exec_digest, the toolchain and cache keys, source digests, and the snapshot key they were derived under. |

## Layer 1 — parse and desugar

### `seed_loading` — Canonical · desugar

**Answers.** Which source units form the snapshot: the entry, every imported seed, their merge order and the spans' virtual bases.

**Inputs.** .hl files; import directives; the workspace root (hale.toml); editor overlays (LSP)

**Producer.** `crates/hale-frontend/src/frontend.rs` · `collect_checkable`

**Consumers.** check; build; run; test; replay; bench; lsp; dna (via the CLI)

**Invariants.**

- one loader, one merge order, for every entry point
- an unresolved import is a diagnostic, never a silently smaller program
- one seed for every entry point: the editor's load (`LoadMode::Editor`) is `hale check <dir>`'s — the open file's directory, every `import` followed through the buffers (`link_checkable`) — and differs only in tolerance: a member that does not parse or will not read is recorded (`Snapshot::unparsed`, `Snapshot::unreadable`) and blocks the scope instead of failing the load, and the LSP publishes an unreadable member as `seed member <name>: <os error>` against the member and the open file, never a clean seed the CLI cannot load

**Missing data.** n/a

**Focused tests.** crates/hale-cli/tests/imports.rs (diamond_import, three_hop_import, import_library_key); crates/hale-cli/tests/source_map.rs; crates/hale-cli/tests/lsp.rs (lsp_and_check_agree_over_a_seed_that_imports, lsp_reports_an_unreadable_seed_member_as_check_does)

**Spec.** spec/projects.md

### `qualified_names` — Migrating · desugar

**Answers.** What a qualified or aliased name denotes: the library identity, the mangled declaration, the construction target, the bus subject a path names.

**Inputs.** import aliases; the seed cache; hale_stdlib::PATH_RENAMES; declaration names

**Producer (today's authority, migrating).** `crates/hale-frontend/src/imports.rs` · `resolve_imports`

**Legacy producers (permitted until removal).**

- `crates/hale-frontend/src/imports.rs` · `lib_canonical_id` — library identity by path, falling back to the file name outside a workspace (can collide). *Removed when:* the snapshot's seed names the library (phase 2, when the frontend owns loading).
- `crates/hale-types/src/check.rs` · `construction_target` — one alias hop in the top scope; a no-op for every program an entry point checks, whose construction sites the desugar sequence already resolved, and the answer for a caller that checks a fragment without the sequence (`check_bundle`). *Removed when:* every checker entry runs the desugar sequence.
- `crates/hale-types/src/resolved.rs` · `resolve_qualified_bus_subjects` — rewrites qualified bus subjects in the resolved-program step's clone. *Removed when:* one resolution, shared.
- `crates/hale-types/src/resolve.rs` · `resolve_bus_subject` — the checker's resolution of the same subjects. *Removed when:* one resolution, shared.
- `crates/hale-types/src/check.rs` · `imported_fn` — an imported fn's signature by path-string vector. *Removed when:* one resolution, shared.

**Also owned.** `crates/hale-types/src/mangle.rs` · `resolve_construction_aliases`

**Consumers.** check; build; lsp (hover, definition, references)

**Invariants.**

- a name resolves once per snapshot; the checker and lowering see the same target
- a construction path spelled with a type alias is resolved once, bundle-wide, in the desugar sequence before the check (`resolve_construction_aliases`), so the checker and lowering read the same target name

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/import_library_key.rs; crates/hale-codegen/tests/cross_seed_imports.rs; crates/hale-types/tests/type_alias.rs

**Spec.** spec/semantics.md § Cross-seed namespace resolution; spec/projects.md

**Guarded seams.**

- `resolve_construction_aliases(` may be referenced from: `crates/hale-types/src/mangle.rs` ×1, `crates/hale-types/src/desugar_sequence.rs` ×1

### `desugar_sequence` — Migrating · desugar

**Answers.** Which rewrites the program receives before checking, in which order: the declaration-shaping passes only (JSON parsers, the api surface, sync inference, unit returns, construction aliases, the omitted `run`, repr accessors). The topic-reference and intra-locus rewrites are not desugars: they erase a written declaration reference the checker's laws and the model read, and run in lowering's resolved program, after the check.

**Inputs.** the merged program; --api / --env (roles); the cross-seed rename table

**Producer (today's authority, migrating).** `crates/hale-types/src/desugar_sequence.rs` · `desugar_before_check`

**Legacy producers (permitted until removal).**

- `crates/hale-frontend/src/snapshot.rs` · `apply_sync_inference` — every verb and the LSP, through the snapshot's load: sync inference per program, outside the sequence, before it. *Removed when:* phase 2: sync inference is a pass of the sequence.
- `crates/hale-codegen/src/codegen.rs` · `build_executable_with_options` — codegen's adapter for a bare program is the test harness's snapshot, not a second pipeline: it builds `Snapshot::from_program` (shaped by the one load, with no source map and no check before lowering, `Config::harness`) and demands the lowering view; its seam allows only the definition, so no non-test caller bypasses the verbs' snapshot (tests are not scanned by the seam guard). *Removed when:* the harness builds from a loaded seed, as the verbs do.

**Also owned.** `crates/hale-types/src/resolved.rs` · `resolve_program`; `crates/hale-syntax/src/desugar.rs` · `desugar_intra_locus_topics`; `crates/hale-syntax/src/desugar.rs` · `desugar_topics`; `crates/hale-types/src/desugar_sequence.rs` · `bundled_stdlib`; `crates/hale-syntax/src/desugar.rs` · `desugar_omitted_run`; `crates/hale-syntax/src/desugar.rs` · `desugar_repr_accessors`

**Consumers.** check; build; run; test; replay; bench; lsp; codegen (`crates/hale-codegen/src/codegen.rs` · `build_resolved`)

**Invariants.**

- one order, run once per snapshot, before the first law is judged
- the sequence is called from the snapshot's load (`Snapshot::load`, `Snapshot::from_program`) and from `check_program` (the test entry), and from nowhere else: every entry point runs it before it mints its snapshot; the bundled stdlib goes through the same passes (`bundled_stdlib`)
- codegen never re-desugars
- the topic-reference and intra-locus rewrites are lowering's, after the check, in `resolve_program` (with the qualified bus subjects they read), and each is recorded as a relation: `TopicRewrite` rows (`topic_rewrites`, and `written_topics` on the bus graph's subjects) and `IntraLocusRewrite` rows (`intra_locus`, and `direct_sends`); `resolve_program` otherwise appends the stdlib, mints and derives the tables
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
- `resolve_program(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1
- `desugar_intra_locus_topics(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `build_executable_with_options(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×1

### `sync_inference` — Migrating · derivation

**Answers.** Which sync discipline each `@form(hashmap)` slot gets when the author declared none, from the pools its methods are called from.

**Inputs.** placement (the pool map); top_scope; form declarations; method call sites

**Producer (today's authority, migrating).** `crates/hale-types/src/sync_inference.rs` · `infer_sync_for_bundle`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/lib.rs` · `apply_sync_inference` — injects the inferred `sync =` FormArg into the AST — the only analysis result codegen receives: every verb, the LSP and the test harness run it through the snapshot's load. *Removed when:* the inferred discipline is a row lowering reads; no AST mutation.
- `crates/hale-types/src/check.rs` · `form_has_explicit_sync_discipline` — the checker's `has a sync discipline` predicate (one caller, the F.31 single-thread check). *Removed when:* one predicate over the form rows.
- `crates/hale-types/src/sync_inference.rs` · `form_has_explicit_sync` — sync inference's own predicate, which counts `sync = none` where the checker's does not. *Removed when:* one predicate over the form rows.

**Consumers.** check (F.31 cross-pool verdicts); codegen (`crates/hale-codegen/src/locus/decl.rs` · `sync_mode`); lsp

**Invariants.**

- every entry point sees the same discipline for the same program

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/placement.rs

**Spec.** spec/forms.md; spec/semantics.md § Placement block (F.31)

**Guarded seams.**

- `apply_sync_inference(` may be referenced from: `crates/hale-types/src/lib.rs` ×4, `crates/hale-frontend/src/snapshot.rs` ×1

### `effect_class_table` — Migrating · derivation

**Answers.** The union of user effect classes across seeds, with `User(i)` indices remapped so one class has one index.

**Inputs.** effect_names / defs per program

**Producer (today's authority, migrating).** `crates/hale-frontend/src/frontend.rs` · `EffectTable`

**Legacy producers (permitted until removal).**

- `crates/hale-frontend/src/frontend.rs` · `merge_programs` — the merge remaps class indices by name. *Removed when:* the table is a declaration-layer row keyed by identity.
- `crates/hale-types/src/effects.rs` · `effect_names_of` — takes the first non-empty program's table; expansion of a class is copied five times across hale-types. *Removed when:* one expansion.

**Consumers.** check; effects; claims

**Invariants.**

- one class, one index, per snapshot

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/cross_seed_effects.rs

**Spec.** spec/verification.md § Default-on & opt-in analyses

## Layer 2 — declaration graphs

### `top_scope` — Migrating · derivation

**Answers.** What every top-level name denotes: the symbol table over the merged program.

**Inputs.** the merged program; import renames

**Producer (today's authority, migrating).** `crates/hale-types/src/resolve.rs` · `build_top_scope`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/lib.rs` · `check_bundle_opts_scoped` — `check_program` (the test entry): built here, once, for its checker and the model its laws are judged over. Beside it the model of a bundle no snapshot holds (`derive_application_model`: `claim_law_diags`, the hale-types tests, and the artifact and model-hash entries over a bare bundle; since 2.3 no verb reaches it), sync inference and the lowering view (once, for the ownership graph and the bus graph) rebuild it; every verb and the LSP (its diagnostics and every request) build one per snapshot (`demand_scope`) and pass it to the checker, the model and the model's graphs. *Removed when:* every consumer demands the scope from a snapshot (2.3).
- `crates/hale-types/src/check.rs` · `collect_known_names` — a second name table the checker keeps beside the scope. *Removed when:* one table.

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_bundle_scoped`); demand (every verb, the LSP's diagnostics and its requests: one scope per snapshot) (`crates/hale-frontend/src/snapshot.rs` · `build_top_scope`); model (the snapshot's scope, handed in) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); resolved program (lowering) (`crates/hale-types/src/resolved.rs` · `build_top_scope`); lsp (definition, placement, the allocation survey: the snapshot's scope) (`crates/hale-lsp/src/lib.rs` · `demand_scope`); lsp (completion, hover, references, enforcement: the editor's scope, over the members that parsed while one does not) (`crates/hale-lsp/src/lib.rs` · `demand_editor_scope`)

**Invariants.**

- one namespace decision: module-nested declarations and imported seeds resolve the same way everywhere
- the editor's scope over a seed with a hole (`demand_editor_scope`) is the same producer over the members that parsed, counted as this family; the whole scope, the check and everything after it stay blocked, so no check runs over a partial program

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/checks_inside_modules.rs; crates/hale-cli/tests/check_unknown_identifier.rs

**Spec.** spec/semantics.md

**Guarded seams.**

- `build_top_scope(` may be referenced from: `crates/hale-types/src/resolve.rs` ×1, `crates/hale-types/src/lib.rs` ×4, `crates/hale-types/src/sync_inference.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1

### `expression_typing` — Migrating · derivation

**Answers.** The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from.

**Inputs.** top_scope; declarations; bodies

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_bundle_scoped`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `infer_accumulator_inner_type` — codegen infers an accumulator's element type again from lowered values where the checker's type is not carried across. *Removed when:* the resolved program carries the checker's types.

**Also owned.** `crates/hale-types/src/resolve.rs` · `infer_literal_ty`

**Consumers.** every layer

**Invariants.**

- expression typing is not a layer: it is the derivation inside layer 3 that produces typed edges, and it stays Rust (final direction)
- codegen types a value only where the checker's type is not yet carried across (the accumulator case); that residue is deleted when the resolved program carries types

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/codegen_fixtures_typecheck.rs; crates/hale-codegen/tests/corpus_check_build_agreement.rs

**Spec.** spec/types.md

### `generics` — Migrating · derivation

**Answers.** Which monomorph a generic call instantiates and how its bindings unify.

**Inputs.** generic declarations; call arguments; the mangled token vocabulary

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `unify_generic_ty`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `unify_generic_param_bindings` — a Ty-level mirror of the checker's unification, by its own comment. *Removed when:* codegen reads the checker's monomorph table.
- `crates/hale-codegen/src/codegen.rs` · `infer_generic_fn_args` — codegen infers generic arguments again from lowered types. *Removed when:* same.
- `crates/hale-types/src/check.rs` · `resolve_generic_monomorph` — the template lookup parses mangled `Name_Tok` strings; tables are per program, not per snapshot. *Removed when:* keyed by identity, per snapshot.

**Consumers.** check; codegen

**Invariants.**

- one unification; the monomorph set is a row lowering reads

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/generic_monomorph_agreement.rs

**Spec.** spec/types.md

### `surfaces` — Migrating · law

**Answers.** Which surface is visible at which depth edge: contract exposure, interface conformance, perspective designation and `serves` conformance.

**Inputs.** type, interface, contract, perspective declarations; locus members

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_structural_impl`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `check_satisfies_bus_adapter` — a second copy of the structural-impl check (its own comment: same logic). *Removed when:* one conformance function.
- `crates/hale-codegen/src/types/mod.rs` · `locus_satisfies_interface` — codegen decides interface conformance by method names only, for storage routing. *Removed when:* codegen reads the conformance row.

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_contract_expose_validity`); check (`crates/hale-types/src/check.rs` · `check_serves_conformance`); check (`crates/hale-types/src/check.rs` · `check_reperspective`); codegen (vtable swap, storage routing)

**Invariants.**

- F.8 compatibility, F.14 and F.20 satisfaction are judged once, with a witness

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/perspective_serves.rs; crates/hale-types/tests/duplicate_member.rs

**Spec.** spec/types.md; spec/semantics.md

### `forms` — Migrating · law

**Answers.** Whether a form's shape, its capacity slots and its projection class are well formed, and which operation set closes each slot.

**Inputs.** @form arguments; capacity declarations; indexed_by; sync discipline

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_form_shape`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/decl.rs` · `sync_mode` — codegen reads the form's arguments again to choose the slot layout. *Removed when:* codegen reads the form rows.

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
- `crates/hale-types/src/stdlib_bodies.rs` · `summarize_with_stdlib` — the stdlib merge for analysis, parsed again per consumer (model builder, LSP, codegen each merge). *Removed when:* the stdlib is part of the snapshot, merged once.
- `crates/hale-types/src/stdlib_bodies.rs` · `summarize_with_stdlib_and_renames` — the rename-aware variant of the same merge. *Removed when:* same.

**Consumers.** effects (`crates/hale-types/src/effects.rs` · `effects_for`); frontier (`crates/hale-types/src/frontier.rs` · `effects_for`); codegen; lsp (hover, completion); doc

**Invariants.**

- the checker and codegen agree on every stdlib call shape (parity test) and on the printable set (corpus agreement)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/stdlib_registry_parity.rs; crates/hale-codegen/tests/corpus_check_build_agreement.rs; crates/hale-cli/tests/doc_effects_catalogue.rs

**Spec.** spec/stdlib.md

## Layer 3 — the locus graph

### `entrypoint` — Migrating · derivation

**Answers.** Which locus is the program's `main`, whether the world is closed, and which declarations are imported.

**Inputs.** locus declarations (is_main, imported, the __lib_ prefix)

**Producer.** none yet: the family has no authoritative producer today; the legacy list is the whole inventory.

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `check_main_and_bindings` — defines `imported main` via `l.imported`. *Removed when:* one definition of the entry, as a row.
- `crates/hale-types/src/check.rs` · `check_pinned_locus_in_loop` — finds main by a `__lib_` name filter. *Removed when:* same.
- `crates/hale-types/src/check.rs` · `compute_pool_of_locus_type` — finds main with no filter; the last one wins. *Removed when:* same.
- `crates/hale-types/src/check.rs` · `check_bus_graph` — closed world = a top-level main only; a module-nested main disables rule 9. *Removed when:* same.
- `crates/hale-cli/src/verbs/check/matrix.rs` · `seed_entry_kind` — parse-only main detection for the check matrix. *Removed when:* same.
- `crates/hale-frontend/src/snapshot.rs` · `let has_main` — has_main for an environment's entrypoint, in the snapshot's load (`check --env`, and the build paths' `--env`). *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `collect_main_placement` — `is_main && !__lib_` over flat declarations. *Removed when:* same.
- `crates/hale-codegen/src/locus/instantiation.rs` · `let is_main_locus` — `is_main_locus` compares type names at instantiation (and twice more in dissolve.rs). *Removed when:* same.
- `crates/hale-codegen/src/locus/dissolve.rs` · `let is_main_locus` — the same comparison in the cascade. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `is_main_entry` — the deferred entry teardown compares the entry's locus name with `main_locus_name` to decide whether it joins the pools (#1208). *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `emit_bindings_prelude` — the connect-transport loss handler is looked up in the locus named by `main_locus_name`. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `in_main` — whether lowering is inside `fn main` is a flag set while main's body is emitted (and cleared around a generic fn lowered from inside it); the frame flush's main-exit wait-abort and `return`-from-main's teardown key on it. *Removed when:* same.

**Consumers.** check; build; dna; codegen

**Invariants.**

- the legacy sites use three definitions of the main locus today, and lowering's `in_main` a fourth, of fn main; the row has one

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/entry_point_placement.rs; crates/hale-types/tests/bus_graph.rs

**Spec.** spec/semantics.md § Bundle-wide rules

### `ownership` — Migrating · derivation

**Answers.** Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array.

**Inputs.** locus declarations (params, accept, release); bodies (let, assign, return, field initialisers, placement entries); fresh factories (one producer); returned bindings

**Producer (today's authority, migrating).** `crates/hale-types/src/ownership.rs` · `resolve_owners`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/ownership_graph.rs` · `build_ownership_graph` — which accepting ancestor owns a method-body birth, keyed (enclosing locus, child type) by name, the child type resolved by `child_locus_name`; built once in the resolved program for lowering and once per snapshot over the checked programs for the model (`demand_ownership_graph`). *Removed when:* one ownership table with both relations; the model reads it when the check runs over the resolved program (phase 2).
- `crates/hale-types/src/model_builder.rs` · `Owns` — the model's params-field tree from main, a third ownership account. *Removed when:* projected from the one table.
- `crates/hale-types/src/ownership.rs` · `extend_fresh_factories` — the carrier-arm fixpoint that widens the factory set. *Removed when:* lowering-only today: the carrier fold widens the set lowering reads; the checker reads the unextended set (phase 2).
- `crates/hale-types/src/borrow_lifetime.rs` · `accepts` — the borrow-lifetime law rebuilds the accept sets from the AST for itself. *Removed when:* reads `accepts_ancestor`.
- `crates/hale-types/src/check.rs` · `check_unowned_subscriber_locus` — the unowned-subscriber rule over its own name-keyed locus index; skipped by `--allow-unowned-subscriber` on some verbs and hard-coded off on others. *Removed when:* a law over the table, on every entry point.
- `crates/hale-codegen/src/locus/instantiation.rs` · `parent_accepts_us` — a monomorphised parent reads its own accept param: graph rows are per template. *Removed when:* the graph keys rows by the template's identity and lowering asks by it (phase 2).
- `crates/hale-types/src/borrow_lifetime.rs` · `returned_decls` — the borrow-lifetime law resolves a returned binding for itself, by span under its own rules. *Removed when:* use-site identity: one resolution keyed by the use's SiteId (phase 2).
- `crates/hale-types/src/alloc_summary.rs` · `collect_escaping_names` — the allocation summary's escape tag resolves a returned binding for itself, by NAME at statement level (an expression-bodied match arm and an expression block are not entered), so an inner shadow of the returned name is tagged `escaping=return` (the #1140 shape). *Removed when:* use-site identity: one resolution keyed by the use's SiteId (phase 2).

**Also owned.** `crates/hale-types/src/ownership.rs` · `resolve_binding_facts`; `crates/hale-types/src/ownership_graph.rs` · `bubble_plans`; `crates/hale-types/src/ownership_graph.rs` · `compute_forwarding_sets`; `crates/hale-types/src/ownership_graph.rs` · `classify_owner_kind`; `crates/hale-types/src/ownership_graph.rs` · `classify_edge`; `crates/hale-types/src/ownership.rs` · `fresh_factories`

**Consumers.** codegen (`crates/hale-codegen/src/locus/instantiation.rs` · `site_owner`); borrow_lifetime (`crates/hale-types/src/borrow_lifetime.rs` · `borrow_lifetime_diags`); model (dynamic births: the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `demand_ownership_graph`); alloc_summary (eager-only accept sets)

**Invariants.**

- a locus instantiation with no row is a CodegenError (F.39)
- ids, not names or spans: declarations are cloned and the stdlib's coordinates overlap user files
- the ownership matrix stays green with an empty KNOWN_OPEN
- `fresh_factories` is read by lowering and the checker with the bundle's import renames, and classifies a factory's returned binding by NAME: a returned name bound twice is no factory. The binding-keyed answer (`returned_bindings`) is folded in for lowering only (`extend_fresh_factories`), so the checker's self-containment rule still refuses to count as a factory a fn whose returned name is shadowed (the #1140 shape)
- which declaration a `return r` names is resolved per body, four ways, because a use carries no identity: `returned_bindings` (by span, falling back to the name), `fresh_factories` (by name), borrow_lifetime's `returned_decls` (by span, under its own rules) and alloc_summary's `collect_escaping_names` (by name)

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/owner_table.rs; crates/hale-codegen/tests/ownership_matrix.rs; crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding); crates/hale-codegen/tests/ownership_bubble.rs

**Spec.** spec/decisions.md F.39; spec/semantics.md § Dissolve timing rules

**Guarded seams.**

- `resolve_owners(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/ownership.rs` ×1
- `build_ownership_graph(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `fresh_factories(` may be referenced from: `crates/hale-types/src/resolved.rs` ×1, `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/check.rs` ×1
- `resolve_binding_facts(` may be referenced from: `crates/hale-types/src/ownership.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `returned_bindings(` may be referenced from: `crates/hale-types/src/ownership.rs` ×3
- `bubble_plans(` may be referenced from: `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1

### `bus_graph` — Migrating · derivation

**Answers.** The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates.

**Inputs.** topics; bus blocks; sends; bindings; placement (for gates)

**Producer (today's authority, migrating).** `crates/hale-types/src/bus_graph.rs` · `build_bus_graph`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `check_bus_graph` — rule 9 runs `collect_bus_walk` itself. *Removed when:* one graph per snapshot.
- `crates/hale-types/src/check.rs` · `check_bus_cycles` — rule 10 keeps its own adjacency (`BusAdj`) instead of reading the graph. *Removed when:* a law over the graph.
- `crates/hale-types/src/check.rs` · `external_subscription_handlers` — handler discovery joined with `::` where the graph uses the last segment. *Removed when:* one subject key.
- `crates/hale-types/src/lib.rs` · `derive_application_model` — the model of a bundle no snapshot holds (the test entry's: `claim_law_diags`, the hale-types tests, the artifact's bundle entry) builds the bus graph, the ownership graph and the handler rows for itself, once each; every verb's model reads its snapshot's. *Removed when:* those callers hold a snapshot.
- `crates/hale-types/src/resolved.rs` · `build_bus_graph` — built once in the resolved program, over the desugared program, for lowering; the snapshot builds a second over the checked programs for the model and hale/busGraph (`demand_bus_graph`), and the checker's rule 9 walks the bus itself. *Removed when:* one graph per snapshot (phase 2: the check over the resolved program).
- `crates/hale-codegen/src/codegen.rs` · `intra_locus_publish_target` — lowering classifies a handler-named method call with a struct argument as a rewritten publish for the probes and the reclaimed subregion, re-deriving the relation the resolved program records. *Removed when:* lowering reads `LoweringView::intra_locus` by the call's id, shadowed against this classification over the corpus (phase 2).

**Also owned.** `crates/hale-types/src/bus_graph.rs` · `dispatch_gates`

**Consumers.** check (rules 9-12, 19); model (subjects, endpoints and gates: the snapshot's graph) (`crates/hale-frontend/src/snapshot.rs` · `demand_bus_graph`); topology; dispatch; lsp (hale/busGraph: the model's graph, so eligibility is the diagnostics pass's) (`crates/hale-lsp/src/lib.rs` · `demand_bus_graph`); bus_inert

**Invariants.**

- one graph, over one program shape, per snapshot; rule 10's cycle graph is a query over it
- the intra-locus rewrite is a relation on the graph, never an erased publisher (boundary 7)

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/bus_graph.rs; crates/hale-types/tests/bus_payload_handler.rs; crates/hale-codegen/tests/bus_devirt_differential.rs

**Spec.** spec/semantics.md rules 9-12, 19; spec/verification.md § Bus-graph property checks

**Guarded seams.**

- `build_bus_graph(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-types/src/resolved.rs` ×1
- `collect_bus_walk(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×2, `crates/hale-types/src/check.rs` ×1
- `dispatch_gates(` may be referenced from: `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1

### `topics` — Migrating · derivation

**Answers.** What each topic is on the wire: its subject, payload contract, routing key, bounds and shed policy; and which topic a send's subject names.

**Inputs.** topic declarations; send subjects; subscribe bounds; bindings (shm_ring)

**Producer (today's authority, migrating).** `crates/hale-types/src/topic_identity.rs` · `topic_wire_subjects`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `collect_topic_wire_subjects` — codegen recomputes wire subjects five times per build, plus its shm-ring, routing-key and bound tables. *Removed when:* codegen reads the topic rows.
- `crates/hale-codegen/src/codegen.rs` · `collect_shm_ring_subjects` — the shm-ring table. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `collect_routing_key_subjects` — the routing-key table. *Removed when:* same.

**Also owned.** `crates/hale-types/src/topic_identity.rs` · `TopicRows::of`; `crates/hale-types/src/topic_identity.rs` · `by_wire`

**Consumers.** check (the topic rows, built once per bundle on the TopScope) (`crates/hale-types/src/resolve.rs` · `build_top_scope`); model (the scope's topic rows: each topic's wire, and the gate merge per wire) (`crates/hale-types/src/model_builder.rs` · `ModelInputs`); codegen (dispatch, bindings, runtime registration); topology (topic shapes); resolved program (the intra-locus relation's wire subjects) (`crates/hale-types/src/resolved.rs` · `topic_wire_subjects`)

**Invariants.**

- delivery joins on the subject's identity, never on the written topic name (spec/model.md rule 8)
- a literal subject at a delivery site (a literal subscription, a literal send) names only the topic that OWNS that wire subject, `TopicRows::by_wire`, never one whose declared segment or name it happens to spell; a topic reference names its declaration; a wire subject two topics carry names neither and is an error

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-codegen/tests/topic_declarations.rs; crates/hale-codegen/tests/replica_keys.rs; crates/hale-codegen/tests/serializer_shape.rs; crates/hale-types/src/topic_identity.rs (a_subject_names_its_topic_by_one_rule, a_shared_subject_names_no_topic)

**Spec.** spec/semantics.md § Topic declarations; spec/semantics.md § Phase 3: routing keys

**Guarded seams.**

- `topic_wire_subjects(` may be referenced from: `crates/hale-types/src/topic_identity.rs` ×2, `crates/hale-types/src/bus_graph.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-codegen/src/codegen.rs` ×2
- `TopicRows::of(` may be referenced from: `crates/hale-types/src/topic_identity.rs` ×1, `crates/hale-types/src/resolve.rs` ×1
- `by_wire(` may be referenced from: `crates/hale-types/src/topic_identity.rs` ×6, `crates/hale-types/src/check.rs` ×2

### `bindings` — Migrating · derivation

**Answers.** Which topics are bound to which transport, in which role, with which codec, and whether the transport can carry the payload.

**Inputs.** bindings blocks; topics; transport specs; purity (codecs)

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `check_main_and_bindings`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `bound_topics` — the set of transport-bound topics, one of four copies (two more in the checker, one in bus_graph), disagreeing on imported mains. *Removed when:* one row.
- `crates/hale-types/src/check.rs` · `collect_topic_pub_sub` — infers the binding role without calling the desugar's `binding_role_for`, which model_builder uses. *Removed when:* one role rule.
- `crates/hale-syntax/src/desugar.rs` · `binding_role_for` — THE binding-role rule, by its own comment, applied at desugar and again by the model builder. *Removed when:* one role row.
- `crates/hale-codegen/src/codegen.rs` · `emit_bindings_prelude` — codegen decides transport, adapter, codec and producer-vs-attach at emission, and refuses a role still `None`. *Removed when:* codegen reads the binding rows.
- `crates/hale-types/src/check.rs` · `transport_satisfies` — the transport capability table. *Removed when:* a capability row in the matrix.

**Consumers.** check; model (binds); codegen; api_surface

**Invariants.**

- F.36 and F.37: binding failure is structural; codec purity is a law over rows

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-cli/tests/binding_imported_main.rs; crates/hale-codegen/tests/bindings_codec_clause.rs

**Spec.** spec/decisions.md F.36, F.37; spec/semantics.md § Operational constraints (Form K)

**Guarded seams.**

- `binding_role_for(` may be referenced from: `crates/hale-syntax/src/desugar.rs` ×1, `crates/hale-types/src/model_builder.rs` ×1

### `dispatch` — Migrating · derivation

**Answers.** How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement.

**Inputs.** bus_graph (gates); placement (domains); the flat-payload predicate; --no-bus-devirt

**Producer (today's authority, migrating).** `crates/hale-model/src/dispatch_plan.rs` · `fn derive`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/resolved.rs` · `from_gates` — the resolved program derives lowering's plan with an empty domain map (#464's widening is a separate optimization); the model derives its own with the arrangement's domains for `same_domain`. *Removed when:* one plan (phase 2).
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

- `crates/hale-codegen/src/channels/mod.rs` · `failure_handler_for` — looks the row up by ordinal in the parent's handler table (one fn per row, built in declare_locus_methods; a monomorph reads its template's rows). *Removed when:* the handler fn is a column of the row.
- `crates/hale-codegen/src/channels/mod.rs` · `resolve_failure_route` — the parent instance is the lowering context's (supervising parent, then self, then params-init self); the handler is the row's. *Removed when:* the instance is a row of the instance tree (phase 2).
- `crates/hale-codegen/src/codegen.rs` · `__StdBusUnixConnectTransport` — the transport-loss handler is picked by name through the routing table. *Removed when:* the bindings family names the transport's locus.
- `crates/hale-types/src/model_builder.rs` · `fn_rows` — the model's function rows key a failure handler by a signature string built from its params' written types. *Removed when:* keyed by the row's SiteId (phase 2).
- `crates/hale-types/src/model_builder.rs` · `SupervisedRef::External` — a child the routing rows resolve as external is recorded by its written name. *Removed when:* keyed by the row's SiteId (phase 2).
- `crates/hale-codegen/src/locus/instantiation.rs` · `settles_failures` — whether the parent has any handler, read from the lowering's handler table at the params-settle bracket. *Removed when:* a query over the routing rows (phase 2).

**Also owned.** `crates/hale-types/src/handler_routing.rs` · `child_locus_name`

**Consumers.** codegen (the handler table) (`crates/hale-codegen/src/locus/decl.rs` · `handlers_of`); codegen (handler bodies, by the row's ordinal) (`crates/hale-codegen/src/locus/method.rs` · `handlers_of`); codegen (__parent_on_failure) (`crates/hale-codegen/src/channels/mod.rs` · `resolve_failure_route`); codegen (restart in place) (`crates/hale-codegen/src/locus/restart.rs` · `restarts_in_place`); model (supervises, over the snapshot's rows: `demand_handlers`) (`crates/hale-types/src/model_builder.rs` · `Supervises`); check (duplicate handlers, over the snapshot's rows handed in: `CheckInputs`) (`crates/hale-types/src/check.rs` · `check_duplicate_failure_handlers`); check (@supervised, over the same rows) (`crates/hale-types/src/frontier.rs` · `supervised_diags`)

**Invariants.**

- the child type is resolved once, by `child_locus_name`; lowering, the checker and the model read the same row
- the checker builds no rows: the snapshot demands them before the check (`CheckInputs`), and the checker's duplicate-handler rule, the `@supervised` law and the model read that one build; a bundle no snapshot holds (the test entries) builds them once, in `bundle_handler_rows`

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/lifecycle_flow.rs (on_failure_dispatch_by_child_type); tests/hale/on_failure_per_child_type_test.hl; crates/hale-types/tests/violate.rs; crates/hale-types/tests/handler_routing_probes.rs

**Spec.** spec/semantics.md § failure; spec/runtime.md (failure delivery)

**Guarded seams.**

- `handler_rows(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/resolved.rs` ×1, `crates/hale-frontend/src/snapshot.rs` ×1, `crates/hale-types/src/lib.rs` ×1
- `child_locus_name(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×2, `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/ownership.rs` ×1
- `DeclaredNames::of(` may be referenced from: `crates/hale-types/src/handler_routing.rs` ×1, `crates/hale-types/src/ownership_graph.rs` ×1, `crates/hale-types/src/ownership.rs` ×1

### `flows` — Migrating · derivation

**Answers.** Which children are flows (released per completion) and which are resident.

**Inputs.** release declarations; accept declarations

**Producer (today's authority, migrating).** `crates/hale-types/src/flows.rs` · `survey`

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/instantiation.rs` · `release_param` — codegen classifies a flow by `release_param == L` at three sites (run elision, run-end reclaim, the release call). *Removed when:* codegen reads the flow row.
- `crates/hale-types/src/check.rs` · `check_accept_release` — the checker's own accept/release survey by type name. *Removed when:* a law over the rows.

**Consumers.** check --flows; codegen

**Invariants.**

- flows.rs states it mirrors codegen (GH #736); the row ends the mirror

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/release_reclaims_flow.rs; crates/hale-codegen/tests/release_two_parents.rs

**Spec.** spec/semantics.md § release(c) and flow children

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

### `effects` — Migrating · derivation

**Answers.** Which effect classes each fn and locus reaches (the callgraph fixpoint), the declared classes and their `causes:`/`depends:` DAG, and the certificate relating the two.

**Inputs.** effect annotations; the callgraph; stdlib_surface (leaf effects); effect_class_table; ffi names

**Producer (today's authority, migrating).** `crates/hale-types/src/frontier.rs` · `infer_effects`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/model_builder.rs` · `infer_effects` — re-run for the model (merged program, with renames). *Removed when:* phase: effects lane; one fixpoint per snapshot.
- `crates/hale-types/src/effects.rs` · `effect_manifest_with_inference` — re-run for the manifest and replay's live-effects gate (merged, no renames). *Removed when:* same.
- `crates/hale-types/src/evidence.rs` · `derive_certificate_evidence` — the certificate engines run once in the check and again for evidence when claims exist. *Removed when:* measured once.
- `crates/hale-types/src/purity.rs` · `infer_purity_for_bundle` — purity, a sibling fixpoint over the same graph, computed on every check for codec laws. *Removed when:* a column of the effects rows.
- `crates/hale-types/src/frontier.rs` · `infer_effects_lower_bound` — the lower-bound variant, a second walk. *Removed when:* one walk with two results.
- `crates/hale-types/src/claims.rs` · `direct_effects` — the claims' own direct-effect predicates. *Removed when:* read the rows.

**Consumers.** check (`crates/hale-types/src/check.rs` · `check_decorator_stacks`); claims (certificate, causes, depends, budget); model (effect labels); replay (effect manifest); doc

**Invariants.**

- derived ⊆ declared is the certificate; budgets are judged through evidence (spec/verification.md)
- effects run once per snapshot, not once per consumer

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/effect_assertions.rs; crates/hale-cli/tests/effects_baseline_gate.rs; crates/hale-cli/tests/effects_manifest.rs

**Spec.** spec/verification.md § Default-on & opt-in analyses; spec/verification.md § Claims

**Guarded seams.**

- `infer_effects(` may be referenced from: `crates/hale-types/src/frontier.rs` ×4, `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/claims.rs` ×1, `crates/hale-types/src/model_builder.rs` ×1, `crates/hale-types/src/topology.rs` ×1
- `effect_manifest_with_inference(` may be referenced from: `crates/hale-types/src/effects.rs` ×1, `crates/hale-types/src/lib.rs` ×1, `crates/hale-cli/src/verbs/replay.rs` ×1
- `infer_purity_for_bundle(` may be referenced from: `crates/hale-types/src/purity.rs` ×2, `crates/hale-types/src/check.rs` ×1

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

**Producer (today's authority, migrating).** `crates/hale-types/src/alloc_summary.rs` · `summarize_programs`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/alloc_summary.rs` · `summarize_programs_with_renames` — the rename-aware variant; the summary is built about twelve times per check with four input variants. *Removed when:* one summary per snapshot.
- `crates/hale-types/src/lib.rs` · `unbounded_alloc_warnings` — the diagnostics entry (check and the LSP's diagnostics), a different entry from the LSP's hale/allocSummary. *Removed when:* one entry.
- `crates/hale-types/src/check.rs` · `check_hot_path_alloc` — hot-path allocation lint over a hand-kept receiver list, keyed by name and `__lib_` suffix. *Removed when:* a law over the rows.
- `crates/hale-codegen/src/codegen.rs` · `compute_nonalloc_free_fns` — FORM-3 non-allocating free fns, a greatest fixpoint keyed by name. *Removed when:* codegen reads the rows (phase: effects lane).
- `crates/hale-codegen/src/codegen.rs` · `compute_scratch_local_free_fns` — which free fns may allocate in their own scratch arena; the checker's ReclaimScope model was left stale by it (#1208). *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `SCRATCH_LOCAL_BUILTINS` — the bare builtins a scratch-local fn may call: a hand-kept subset of the checker's `BARE_BUILTIN_CALLEES`, with no agreement test. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `SCRATCH_LOCAL_STD_NAMESPACES` — the `std::` namespaces whose runtime primitives the scratch-local classification takes to keep no argument, a per-namespace claim no stdlib_surface row states. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `ScratchPaths` — the scratch-local classification's own qualified-path lookup (`call_ok`: import renames first, then `PATH_RENAMES`); `resolved::lookup_qualified_path` checks them in the other order. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `current_user_fn_scratch_local` — the per-fn flag lowering sets from the scratch-local set while it emits a fn's body. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `compute_elidable_methods` — methods whose scratch arena can be elided; recomputed on the fly per method by `method_scratch_elidable`. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `method_scratch_elidable` — the on-the-fly copy; lifecycle hooks are decided only here. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `locus_arena_elidable` — arena elision per locus, with an empty interprocedural context on purpose. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `let dbg = format!("{:?}", f.body);` — the caller-arena TLS publish gate decides from the body's Debug string. *Removed when:* same.
- `crates/hale-types/src/alloc_summary.rs` · `ReclaimScope` — the checker's reclaim model, stale for scratch-local fns since #1208. *Removed when:* one model.

**Consumers.** check (unbounded allocation, hot path); lsp (hale/allocSummary); claims (@budget); codegen (arena routing at an allocation) (`crates/hale-codegen/src/codegen.rs` · `current_arena_ptr`); resource_budget

**Invariants.**

- the checker's reclaim model and codegen's routing agree; a stale copy is a registry violation, not a comment

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/hot_path_alloc.rs; crates/hale-codegen/tests/scratch_local_free_fn.rs; crates/hale-codegen/tests/fn_nonalloc_add.rs; crates/hale-codegen/tests/method_scratch_elision.rs

**Spec.** spec/memory.md § Allocation routing; spec/styleguide.md

**Guarded seams.**

- `summarize_programs` may be referenced from: `crates/hale-types/src/alloc_summary.rs` ×5, `crates/hale-types/src/lib.rs` ×1, `crates/hale-lsp/src/lib.rs` ×1, `crates/hale-types/src/budget_check.rs` ×1, `crates/hale-types/src/frontier.rs` ×1, `crates/hale-types/src/model_builder.rs` ×1, `crates/hale-types/src/quantitative.rs` ×1, `crates/hale-types/src/resource_budget.rs` ×2, `crates/hale-types/src/stdlib_bodies.rs` ×1, `crates/hale-types/src/topology.rs` ×1
- `unbounded_alloc_warnings(` may be referenced from: `crates/hale-types/src/lib.rs` ×1, `crates/hale-cli/src/verbs/check/run_impl.rs` ×1, `crates/hale-lsp/src/lib.rs` ×1

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

**Inputs.** placement and topology blocks; main's params; entrypoint; ownership (nested fields)

**Producer (today's authority, migrating).** `crates/hale-types/src/check.rs` · `compute_pool_of_locus_type`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `enclosing_field_placement` — owner-relative placement, one of four in-checker derivations (two more inline in the blocking and single-thread checks). *Removed when:* one placement table per snapshot (phase: placement lane).
- `crates/hale-types/src/bus_graph.rs` · `collect_subscriber_placements` — per type, first wins. *Removed when:* same.
- `crates/hale-types/src/ownership_graph.rs` · `collect_placements` — a verbatim copy of the previous. *Removed when:* same.
- `crates/hale-types/src/model_builder.rs` · `PlacedIn` — the model's arrangement, per instance with replicas. *Removed when:* projected from the table.
- `crates/hale-types/src/resource_budget.rs` · `budget_for_programs` — counts placement entries, ignores replicas. *Removed when:* reads the table.
- `crates/hale-syntax/src/desugar.rs` · `collect_off_owner_thread_fields` — a desugar-time placement read. *Removed when:* reads the table.
- `crates/hale-codegen/src/codegen.rs` · `collect_main_placement` — codegen's DeploymentPlan, keyed by field name and locus type name. *Removed when:* codegen reads the table.
- `crates/hale-codegen/src/deployment.rs` · `DeploymentPlan` — the plan type lowering reads today. *Removed when:* becomes the layer-5 table.

**Consumers.** check (rules 2-5, 13-18; F.31); sync_inference; dispatch (domains); model (placed_in, affined_to); codegen (pools, mailboxes, affinity); lsp (hale/placement); deployment (reserved)

**Invariants.**

- F.38: placement is semantics-free, so a backend may Approximate it
- placement is a choice point: v1's declared placement is the single candidate

**Missing data.** an unknown is a hole with a stated policy

**Focused tests.** crates/hale-types/tests/placement.rs; crates/hale-types/tests/placement_pairings.rs; crates/hale-codegen/tests/pool_affinity.rs; crates/hale-codegen/tests/placement_where_async_io.rs; crates/hale-types/tests/shadow_placement.rs (the shadow of compute_pool_of_locus_type against collect_subscriber_placements over the corpus; 20 classified divergences, all known old bugs)

**Spec.** spec/semantics.md § Placement block (F.31); spec/decisions.md F.31, F.35, F.38

**Guarded seams.**

- `compute_pool_of_locus_type(` may be referenced from: `crates/hale-types/src/check.rs` ×2, `crates/hale-types/src/lib.rs` ×1
- `collect_main_placement(` may be referenced from: `crates/hale-codegen/src/codegen.rs` ×2

### `target_capability` — Migrating · capability

**Answers.** What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability.

**Inputs.** --target; a source `target` declaration; stdlib_surface; FFI signatures

**Producer.** none yet: the family has no authoritative producer today; the legacy list is the whole inventory.

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/check.rs` · `wasm_unavailable_stdlib` — a hand-kept slice-pattern table keyed by leading namespace, consulted only when the SOURCE declares `target wasm` (never from `--target wasm32`), and only for call forms. *Removed when:* one CapabilityMatrix consulted by the driver before lowering.
- `crates/hale-types/src/check.rs` · `wasm_target` — the source-declaration flag the table is gated on. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `link_wasm` — link-time refusals (link_libs) and the export list. *Removed when:* same.
- `crates/hale-codegen/src/locus/instantiation.rs` · `lotus_replay_start_ingress` — one of the per-site wasm skips; instantiation still emits pool shutdown and wait-abort on wasm where the main exit does not. *Removed when:* same.
- `crates/hale-codegen/src/codegen.rs` · `is_wasm` — the backend configuration scattered across a dozen sites. *Removed when:* same.
- `crates/hale-types/src/check.rs` · `ffi_type_unportable` — FFI portability per type. *Removed when:* a capability row.
- `crates/hale-codegen/src/target.rs` · `TargetSpec` — has_async_io is true for wasm32; the checker sees the target only under `hale build`. *Removed when:* the matrix is the one statement, on every entry point.

**Consumers.** check; build; docs (systems/webassembly.md, generated from the matrix)

**Invariants.**

- Approximate is legitimate only in layers 5 and 7; everywhere else a target lowers or rejects, with the row's witness
- the docs' target statement is generated, never hand-maintained

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/wasm_target_gating.rs; crates/hale-codegen/tests/wasm_target.rs; crates/hale-cli/tests/target_model.rs

**Spec.** spec/decisions.md F.35; docs/src/systems/webassembly.md

**Guarded seams.**

- `wasm_unavailable_stdlib(` may be referenced from: `crates/hale-types/src/check.rs` ×2

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

**Inputs.** ownership; placement; handler_routing; flows; restart; the runtime protocol (lotus_failure_hold / await / defer_reclaim)

**Producer.** none yet: the family has no authoritative producer today; the legacy list is the whole inventory.

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
- `crates/hale-codegen/runtime/lotus_arena.c` · `lotus_failure_hold` — the hold/settle/defer/await protocol in the C runtime; verified against the table by a debug-build oracle before the table is trusted. *Removed when:* the oracle holds.

**Consumers.** codegen (emission reads the order); closures (the event alphabet); transitions (reserved); deployment (reserved)

**Invariants.**

- handlers run only on the queue owner's thread, so cross-thread failure delivery follows spec/runtime.md (a typed bus message): the first named decision, with its own regression test
- a spec/implementation disagreement is settled as a named decision, never by extraction picking a side

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/lifecycle_flow.rs; crates/hale-codegen/tests/reclamation_spine.rs; crates/hale-codegen/tests/main_locus_deferred_pool_join.rs; crates/hale-codegen/tests/teardown_pinned_join_order.rs

**Spec.** spec/runtime.md (failure delivery; pool join rule b); spec/semantics.md § lifecycle

### `bus_inert` — Migrating · derivation

**Answers.** Whether the program can ever have a bus cell in flight, so drains can be elided.

**Inputs.** bus_graph; stdlib_surface (which namespaces publish)

**Producer.** none yet: the family has no authoritative producer today; the legacy list is the whole inventory.

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/codegen.rs` · `let dbg = format!("{:?}", program.items);` — decided by searching the program's Debug string for `__Std`, `name: "std"` and tainted namespaces. *Removed when:* a query over the message graph.
- `crates/hale-codegen/src/codegen.rs` · `stdlib_bus_tainted_namespaces` — the stdlib taint fixpoint, also over Debug strings, cached per process. *Removed when:* a column of the stdlib_surface rows.
- `crates/hale-types/src/resolved.rs` · `user` — the resolved program carries the desugared user program a second time so lowering's tier-1 bus-inert scan reads the same Debug text it always did. *Removed when:* the bus-inert verdict is a row of the resolved program, computed structurally and shadowed against the scan (phase 2).

**Consumers.** codegen (`crates/hale-codegen/src/bus/runtime.rs` · `emit_bus_drain`)

**Invariants.**

- a drain elision is a conclusion of the message graph, never of a string

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/drain_elision.rs; crates/hale-codegen/tests/log_routing.rs

**Spec.** spec/runtime.md § drain

## Layer 8 — lowering

### `law_backstops` — Migrating · law

**Answers.** The checker rules lowering re-judges because `build_executable` never runs the checker: self-containment, cross-pool bare statements, placement entries, pinned loci in loops.

**Inputs.** the AST; the lowering context

**Producer.** none yet: the family has no authoritative producer today; the legacy list is the whole inventory.

**Legacy producers (permitted until removal).**

- `crates/hale-codegen/src/locus/instantiation.rs` · `CodegenError::Unsupported` — spanless refusals at lowering for rules the checker already states (rule 6's checker evaluator landed in phase 0; the backstop stays for harness builds that skip the checker); for a placed locus the checker types as Unknown, for an `accept()` with no parameter (the checker keys on `accept_param`, codegen on the method name), and for an adapter locus instantiated inline in a `bindings { }` block (which lowering pins without a placement entry), it is the only evaluator. *Removed when:* one pipeline guarantees the checker ran before lowering (phase 2), and the refusals become dead.

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
- the model builds none of the families it reads beside the program (2.3): the scope with its topic rows, the bus graph, the ownership graph and the handler rows arrive as `ModelInputs`, each demanded once from the snapshot over the checked programs; the effects, the allocation summary and the placement it still re-runs for itself are listed under their families

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

**Consumers.** check / verify; topology (law section); fleet; dna (dna_law.rs wording); model diff

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

**Answers.** The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again in the resolved-program step (over the user program before the intra-locus rewrite, so the sends it records are minted on every path, and over the merged program with the bundle's seeds and a named seed for the bundled stdlib), idempotently (one numbering; a later mint numbers only what an earlier one did not see), with reliable provenance.

**Inputs.** seed_loading; desugar_sequence

**Producer (today's authority, migrating).** `crates/hale-types/src/snapshot.rs` · `mint`

**Legacy producers (permitted until removal).**

- `crates/hale-types/src/ownership.rs` · `BindingKey` — use sites inside the returned-bindings walk are resolved by span (identifiers are not minted sites); the row is keyed by the `let`'s snapshot identity. *Removed when:* use-site identity (phase 1.1 follow-up).
- `crates/hale-model/src/ids.rs` · `FunctionId` — model ids are ranks in a sorted string order (`L::f`, `(name, kind)`, path strings). *Removed when:* same.
- `crates/hale-types/src/effects.rs` · `FnKey` — analysis keys are (locus name, fn name). *Removed when:* same.
- `crates/hale-types/src/check.rs` · `type_expr_key` — rule 12 compares stringified TypeExprs. *Removed when:* same.

**Consumers.** every table; the shadow facility (compares through an explicit correspondence, never raw id equality); lsp (a later incremental future); the resolved program (codegen's input is minted over the merged program)

**Invariants.**

- addresses are not identities (declarations are cloned); spans are not (the stdlib's coordinates overlap user files; desugars share spans)
- snapshot-local uniqueness and provenance are the requirement; persistent identity across editor revisions is a separate problem
- canonical ids need real equality and hashing; the AST's structural NodeId equality stays separate
- the identity's types are hale_graph::ids (SeedId, SiteId; phase 1.1a); hale_types::snapshot::mint numbers every site the AST walk hale_syntax::sites reaches with one counter: after the entry point's last desugar, and again, idempotently, in the resolved-program step, over the user program before the intra-locus rewrite (the rewrite moves a send's id onto its call and records it) and over the merged program, each numbering only what an earlier mint did not see (phase 1.1b); every entry point calls it after its last desugar with its source map and the bundle carries the result (every verb and the LSP through the snapshot's load, the test harness through `Snapshot::from_program`, with no source map); the lowering view mints with the bundle's seeds, the stdlib's sites under the named seed snapshot::STDLIB_SEED (its spans overlap the first file's); a generated site records the desugar that made it (Snapshot::origins); codegen's generic instantiation keeps the template's id, and the F.39 pre-pass numbers nothing: an unminted Struct or Call is an error

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding); crates/hale-codegen/tests/owner_table.rs

**Spec.** spec/decisions.md F.39, F.40

**Guarded seams.**

- `mint(` may be referenced from: `crates/hale-types/src/resolved.rs` ×2, `crates/hale-frontend/src/snapshot.rs` ×1

### `demand` — Migrating · derivation

**Answers.** Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph and handler rows, the model, the check, the lowering view), each at most once, blocking a family whose prerequisite reported errors.

**Inputs.** seed_loading; desugar_sequence; snapshot_identity; the config (target, api, api roles, environment, the check's rules); editor overlays (LSP); a consumer's request

**Producer (today's authority, migrating).** `crates/hale-frontend/src/snapshot.rs` · `Snapshot`

**Legacy producers (permitted until removal).**

- `crates/hale-lsp/src/lib.rs` · `document_symbols` — textDocument/documentSymbol parses its one buffer itself (`parse_source`, file-local spans): a syntactic outline of the open file, answered while another member of the seed does not parse; every other request reads the snapshot. *Removed when:* the snapshot keeps each member's own program beside the merged one.

**Also owned.** `crates/hale-frontend/src/snapshot.rs` · `demand_scope`; `crates/hale-frontend/src/snapshot.rs` · `demand_editor_scope`; `crates/hale-frontend/src/snapshot.rs` · `demand_bus_graph`; `crates/hale-frontend/src/snapshot.rs` · `demand_ownership_graph`; `crates/hale-frontend/src/snapshot.rs` · `demand_handlers`; `crates/hale-frontend/src/snapshot.rs` · `demand_model`; `crates/hale-frontend/src/snapshot.rs` · `demand_check`; `crates/hale-frontend/src/snapshot.rs` · `demand_lowering`; `crates/hale-frontend/src/snapshot.rs` · `from_program`; `crates/hale-frontend/src/snapshot.rs` · `SnapshotKey`

**Consumers.** check (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_check`); the checker's rules (the handler rows: duplicate handlers, `@supervised`), demanded before the check runs (`crates/hale-frontend/src/snapshot.rs` · `CheckInputs`); topology (`--dump-topology`, `--check-topology`, `--check-topology-shape`: one artifact of the snapshot's model) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `dump_topology_over`); the api description (`--dump-api`: the surface the snapshot's sequence generated the binding for) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `api_surface`); the model dump (`--dump-model`: the check's own model when it judged a law) (`crates/hale-cli/src/verbs/check/run_impl.rs` · `demand_model`); the build identity (build, run, replay: the model hash and the obs ids from the snapshot's model, the plan digest from its lowering view) (`crates/hale-cli/src/shared/options.rs` · `model_identity`); build (`crates/hale-cli/src/verbs/build.rs` · `demand_lowering`); run <file> and run <dir> (`crates/hale-cli/src/verbs/run.rs` · `demand_lowering`); test (a build config for the host, the dev profile) (`crates/hale-cli/src/verbs/test.rs` · `demand_lowering`); replay (the identity admitted before lowering) (`crates/hale-cli/src/verbs/replay.rs` · `demand_lowering`); bench (the driver an overlay on the bench file) (`crates/hale-cli/src/verbs/bench.rs` · `demand_lowering`); the test harness (`Snapshot::from_program`, lowering not gated on a check) (`crates/hale-codegen/src/codegen.rs` · `demand_lowering`); lsp (diagnostics: one snapshot per document event) (`crates/hale-lsp/src/lib.rs` · `demand_check`); lsp (every request loads the snapshot the diagnostics load, `editor_snapshot`, one per request) (`crates/hale-lsp/src/lib.rs` · `editor_snapshot`); lsp (definition, placement, the allocation survey) (`crates/hale-lsp/src/lib.rs` · `demand_scope`); lsp (completion, hover, references, enforcement) (`crates/hale-lsp/src/lib.rs` · `demand_editor_scope`); lsp (hale/busGraph) (`crates/hale-lsp/src/lib.rs` · `demand_bus_graph`)

**Invariants.**

- a prerequisite runs once: every family is a `OnceCell` of its snapshot, and a family that reads another demands it rather than building its own; `Snapshot::builds` counts each family's producer runs, and no count exceeds one on any consumer on the snapshot
- a family nobody requested is not computed: the no-claims editor path builds no model (GH #476 criterion 1), nor the graphs it reads
- the model's inputs are families (2.3): `demand_model` demands the scope, the bus graph, the ownership graph and the handler rows over the checked programs (the `bus_graph`, `ownership` and `handler_routing` counts), each once; lowering's graphs are the lowering view's own, over the resolved program, until the check runs over it
- the checker's inputs are families (2.3): the typing demands the handler rows before the checker runs and hands them in (`CheckInputs`), so a check with a law builds them once for the checker and the model together; their producer reads declarations, not types, so it is total over a program that does not typecheck
- a family whose prerequisite reported errors is `Blocked { family, because }`, not computed: an editor seed with a member that did not parse or would not read has no scope (the editor's requests read `demand_editor_scope`, the scope over the members that parsed with the hole named, and never a scope of their own), a program that does not typecheck has no model, a program whose check reported an error has no lowering view; a ready result may still hold typed holes
- the lowering view (`LoweringView`, the `lowering_view` count) is a family: `demand_lowering` demands the check, then runs `resolve_program` once over the snapshot's program, source map, renames and api config; `build_resolved` reads it by reference; every build path (build, run, test, replay, bench) demands it from a `Snapshot::load`, and the test harness from `Snapshot::from_program`, whose config (`Config::harness`) does not gate lowering on the check
- a changed entry, load mode, target, config, overlay or source text is a distinct snapshot (`SnapshotKey`, computed after the load from what it read; a bare program is its own load); two snapshots share no result, and a snapshot is dropped on any change (incremental reuse is a later future)
- the bundle is a borrowed view (`Snapshot::bundle`), built per call, never stored

**Missing data.** a missing required row is a compiler error

**Focused tests.** crates/hale-types/tests/demand_gate.rs; crates/hale-frontend/src/snapshot.rs (a_changed_input_is_a_distinct_snapshot_and_shares_no_result, a_file_that_does_not_parse_blocks_the_scope_and_its_dependents, the_editor_scope_covers_the_members_that_parsed_and_names_the_hole, a_member_that_will_not_read_blocks_the_scope, a_program_that_does_not_typecheck_blocks_the_model, a_check_with_errors_blocks_the_lowering_view, the_lowering_view_is_resolved_once_after_the_check); crates/hale-lsp/src/lib.rs (a_request_builds_only_the_families_it_reads, a_request_over_a_seed_with_a_hole_answers_from_the_members_that_parsed)

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
- `crates/hale-cli/build.rs` · `toolchain_digest` — the replay identity: `hale_graph::identity::identity_files`, every identity-covered crate (`COVERED_CRATES`, hale-cli among them) and the manifest files (`Cargo.lock`, the ts-shim manifest), walked through the one shared walk. *Removed when:* phase 2.
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
| semantics/placement/10 | bus cycles: cross-locus warning, intra-locus unconditional error | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_cycles` | Migrating |
| semantics/placement/11 | bus backpressure heuristic | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_backpressure` | Migrating |
| semantics/placement/12 | one literal subject, one payload type | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_subject_types` | Migrating |
| semantics/placement/13 | degenerate `pinned(cores = ..)` is an error | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | Canonical |
| semantics/placement/14 | topology consistency and node/l3 resolution | `placement` | `crates/hale-types/src/check.rs` · `check_topology_block` | Canonical |
| semantics/placement/15 | `replicas = K`: K >= 1, pinned only | `placement` | `crates/hale-types/src/check.rs` · `check_placement_block` | Canonical |
| semantics/placement/16 | pool affinity agrees per pool | `placement` | `crates/hale-types/src/check.rs` · `check_pool_affinity` | Migrating |
| semantics/placement/17 | a pinned locus is not instantiated in a loop | `placement` | `crates/hale-types/src/check.rs` · `check_pinned_locus_in_loop` | Migrating |
| semantics/placement/18 | every placement entry is consumed exactly once | `placement` | `crates/hale-types/src/check.rs` · `check_placement_entry_consumed` | Migrating |
| semantics/placement/19 | a bus payload is carriable | `bus_graph` | `crates/hale-types/src/check.rs` · `check_bus_payload_carriable` | Migrating |

## Frozen Debug renderings

Every Debug rendering with no prose around it (a `?}` placeholder in a formatting macro whose template holds no space) in `hale-syntax`, `hale-types`, `hale-model`, `hale-codegen`, `hale-frontend`, `hale-cli` and `hale-lsp`, with the number of invocations that collapse to the fragment. A message with prose around its `{:?}` is not listed: it is read by a person. A site that *decides* derives a fact from a Debug string and is permitted only until its family's table replaces it; a new site fails the guard.

| path | invocation | count | verdict |
|---|---|---|---|
| `crates/hale-cli/src/build_env.rs` | `format!( "target={:?};cpu={:?};dev={};debug={}", o.target, o.target_cpu, o.dev_profile, o.` | 1 | decides (`digests`) |
| `crates/hale-cli/src/build_env.rs` | `format!(";lto={l:?}")` | 1 | decides (`digests`) |
| `crates/hale-cli/src/verbs/misc.rs` | `println!("{:#?}", prog)` | 1 | renders |
| `crates/hale-codegen/src/codegen.rs` | `format!("{:?}", f.body)` | 1 | decides (`alloc_summary`) |
| `crates/hale-codegen/src/codegen.rs` | `format!("{:?}", it)` | 1 | decides (`bus_inert`) |
| `crates/hale-codegen/src/codegen.rs` | `format!("{:?}", other)` | 1 | renders |
| `crates/hale-codegen/src/codegen.rs` | `format!("{:?}", program.items)` | 1 | decides (`bus_inert`) |
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
