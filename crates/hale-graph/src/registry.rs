//! The semantic-family registry (F.40, phase 0).
//!
//! One entry per semantic family the compiler derives, stating its
//! contract. The registry is the index `spec/registry.md` is rendered
//! from, and the input to the guard tests in `tests/`: a site named
//! here must exist, a derivation-shaped function must be named here,
//! a family's seams may be referenced only from the files named here.
//!
//! Every path is workspace-relative. Every symbol is a function, type
//! or constant name, or a distinctive text fragment where the site is
//! a region inside a large function. Line numbers are deliberately
//! absent: they rot, names are checked.
//!
//! Adding a family: fill every field; a `Migrating` family lists
//! every legacy producer with the condition under which it is
//! deleted; a `Canonical` family has one producer and no legacy
//! list; a `Reserved` family names no producer and computes nothing.
//! Then `HALE_REGEN_REGISTRY=1 cargo test -p hale-graph --test
//! registry_matches_spec` and review the spec diff.

/// The migration state of a family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// A future family or consumer. Creates no production demand and
    /// no table; exists so the tables never change twice.
    Reserved,
    /// The canonical producer is under development. `legacy` is the
    /// exact inventory of permitted producers, each with its removal
    /// condition.
    Migrating,
    /// One production producer; consumers cannot reconstruct its
    /// meaning.
    Canonical,
}

/// What kind of thing the family produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Rows: a fact about the program, derived from source or from
    /// earlier rows.
    Derivation,
    /// A verdict with a witness, judged over rows.
    Law,
    /// A rewrite of the program that encodes semantics.
    Desugar,
    /// An identity derived from rows or sources.
    Digest,
    /// What a target or backend can lower, and what it refuses.
    Capability,
}

/// The layer a family belongs to (RFC #1212 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// Parse and desugar: the merged program and its identities.
    Parse,
    /// Declaration graphs: types, interfaces, contracts, topics.
    Declarations,
    /// The locus graph: the tower, the message graph, typed edges.
    Locus,
    /// Effects: classes, blocking, allocation, borrows, fallibility.
    Effects,
    /// Placement: thread domains, transports, targets.
    Placement,
    /// Lifecycle order: birth, settle, failure, reclaim, drain, restart.
    Lifecycle,
    /// Lowering-time decisions that must leave lowering.
    Lowering,
    /// Laws and their engine.
    Law,
    /// Identity, digests and snapshots.
    Identity,
}

/// What a consumer does when a required row is absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// A missing required row is a compiler error; nobody guesses.
    Error,
    /// A declared unknown with a stated policy (a hole).
    Hole,
    /// Not applicable (a desugar, a digest, a reserved family).
    NotApplicable,
}

/// A place in the tree: a workspace-relative path and a symbol or
/// text fragment the file must contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub path: &'static str,
    pub symbol: &'static str,
}

/// A producer permitted while the family migrates.
#[derive(Debug, Clone, Copy)]
pub struct Legacy {
    pub site: Site,
    /// What this site derives, and how (keyed by name, span, id, or
    /// a Debug string).
    pub note: &'static str,
    /// The condition under which it is deleted.
    pub removal: &'static str,
}

/// Something that reads the family's result.
#[derive(Debug, Clone, Copy)]
pub struct Consumer {
    /// An entry point (`check`, `build`, `lsp`, ...) or a crate area.
    pub who: &'static str,
    pub site: Option<Site>,
}

/// A guarded entry symbol: the files that may reference it, each
/// with the number of references it holds today. A new reference in
/// an allowlisted file is a change to the count, so "one consumer
/// per commit" shows up as a decrement and a new re-derivation as an
/// increment.
#[derive(Debug, Clone, Copy)]
pub struct Seam {
    pub symbol: &'static str,
    pub allowed: &'static [(&'static str, usize)],
}

/// One semantic family and its contract.
#[derive(Debug, Clone, Copy)]
pub struct Family {
    pub name: &'static str,
    pub layer: Layer,
    pub state: State,
    pub kind: Kind,
    /// The question the family answers, in one sentence.
    pub answers: &'static str,
    /// What it reads: source shapes, other families, configuration.
    pub inputs: &'static [&'static str],
    /// The authoritative producer (Canonical), or the target (Migrating,
    /// when one exists today).
    pub producer: Option<Site>,
    pub legacy: &'static [Legacy],
    /// Other derivation-shaped definitions the producer owns: its
    /// helpers and variants. Registered like the producer, never a
    /// second authority.
    pub owned: &'static [Site],
    pub consumers: &'static [Consumer],
    pub invariants: &'static [&'static str],
    pub missing: Missing,
    /// Focused tests, as paths a contributor runs.
    pub tests: &'static [&'static str],
    /// Where the spec states the contract.
    pub spec: &'static [&'static str],
    pub seams: &'static [Seam],
}

/// A numbered spec rule and the code that evaluates it.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub id: &'static str,
    pub gist: &'static str,
    pub family: &'static str,
    pub evaluator: Option<Site>,
    pub state: State,
}

/// What a `format!("{:?}", ..)` site does with the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanVerdict {
    /// The string is searched to decide a fact: a derivation with no
    /// name, permitted until the named family's table replaces it.
    Decides { family: &'static str },
    /// The string only renders a label, key or message.
    Renders,
}

/// A frozen Debug rendering: a formatting-macro invocation (`format!`,
/// `write!`, `writeln!`, `println!`, `eprintln!`) whose template holds
/// a `?}` placeholder and no space. A template with prose is a
/// message; one without is a value, and a value rendered from Debug
/// and then compared, searched or hashed is a derivation with no name.
#[derive(Debug, Clone, Copy)]
pub struct DebugScan {
    pub path: &'static str,
    /// The invocation, collapsed to one line, as the scan renders it
    /// (its first 90 characters).
    pub fragment: &'static str,
    /// How many invocations in the file collapse to the fragment.
    pub count: usize,
    pub verdict: ScanVerdict,
}

const fn site(path: &'static str, symbol: &'static str) -> Site {
    Site { path, symbol }
}
const fn legacy(
    path: &'static str,
    symbol: &'static str,
    note: &'static str,
    removal: &'static str,
) -> Legacy {
    Legacy {
        site: Site { path, symbol },
        note,
        removal,
    }
}
const fn consumer(who: &'static str) -> Consumer {
    Consumer { who, site: None }
}
const fn consumer_at(who: &'static str, path: &'static str, symbol: &'static str) -> Consumer {
    Consumer {
        who,
        site: Some(Site { path, symbol }),
    }
}

// Paths, so a rename is one edit.
const CHECK: &str = "crates/hale-types/src/check.rs";
const TLIB: &str = "crates/hale-types/src/lib.rs";
const RESOLVE: &str = "crates/hale-types/src/resolve.rs";
const MODEL_BUILDER: &str = "crates/hale-types/src/model_builder.rs";
const BUS_GRAPH: &str = "crates/hale-types/src/bus_graph.rs";
const OWNERSHIP_GRAPH: &str = "crates/hale-types/src/ownership_graph.rs";
const TY_OWN: &str = "crates/hale-types/src/ownership.rs";
const TY_MANGLE: &str = "crates/hale-types/src/mangle.rs";
const TY_RESOLVED: &str = "crates/hale-types/src/resolved.rs";
const HANDLER_ROUTING: &str = "crates/hale-types/src/handler_routing.rs";
const EFFECTS: &str = "crates/hale-types/src/effects.rs";
const FRONTIER: &str = "crates/hale-types/src/frontier.rs";
const EVIDENCE: &str = "crates/hale-types/src/evidence.rs";
const ALLOC: &str = "crates/hale-types/src/alloc_summary.rs";
const PURITY: &str = "crates/hale-types/src/purity.rs";
const TOPOLOGY: &str = "crates/hale-types/src/topology.rs";
const JUDGMENT: &str = "crates/hale-types/src/judgment.rs";
const CLAIMS: &str = "crates/hale-types/src/claims.rs";
const SYNC: &str = "crates/hale-types/src/sync_inference.rs";
const TOPIC_ID: &str = "crates/hale-types/src/topic_identity.rs";
const STDLIB_SURFACE: &str = "crates/hale-types/src/stdlib_surface.rs";
const STDLIB_BODIES: &str = "crates/hale-types/src/stdlib_bodies.rs";
const CG: &str = "crates/hale-codegen/src/codegen.rs";
const CG_INST: &str = "crates/hale-codegen/src/locus/instantiation.rs";
const CG_DECL: &str = "crates/hale-codegen/src/locus/decl.rs";
const CG_DISSOLVE: &str = "crates/hale-codegen/src/locus/dissolve.rs";
const CG_RESTART: &str = "crates/hale-codegen/src/locus/restart.rs";
const CG_CHANNELS: &str = "crates/hale-codegen/src/channels/mod.rs";
const CG_WIRE: &str = "crates/hale-codegen/src/bus/wire.rs";
const CG_BUS_RT: &str = "crates/hale-codegen/src/bus/runtime.rs";
const CG_TYPES: &str = "crates/hale-codegen/src/types/mod.rs";
const CG_DEPLOY: &str = "crates/hale-codegen/src/deployment.rs";
const CG_TARGET: &str = "crates/hale-codegen/src/target.rs";
const LOTUS: &str = "crates/hale-codegen/runtime/lotus_arena.c";
const FRONTEND: &str = "crates/hale-cli/src/shared/frontend.rs";
const IMPORTS: &str = "crates/hale-cli/src/shared/imports.rs";
const OPTIONS: &str = "crates/hale-cli/src/shared/options.rs";
const BUILD_ENV: &str = "crates/hale-cli/src/build_env.rs";
const STALE: &str = "crates/hale-cli/src/shared/stale.rs";
const V_CHECK: &str = "crates/hale-cli/src/verbs/check/run_impl.rs";
const V_MATRIX: &str = "crates/hale-cli/src/verbs/check/matrix.rs";
const V_BUILD: &str = "crates/hale-cli/src/verbs/build.rs";
const V_RUN: &str = "crates/hale-cli/src/verbs/run.rs";
const V_TEST: &str = "crates/hale-cli/src/verbs/test.rs";
const V_REPLAY: &str = "crates/hale-cli/src/verbs/replay.rs";
const V_BENCH: &str = "crates/hale-cli/src/verbs/bench.rs";
const TOPO_LAW: &str = "crates/hale-cli/src/topology_law.rs";
const CLI_BUILD_RS: &str = "crates/hale-cli/build.rs";
const LSP: &str = "crates/hale-lsp/src/lib.rs";
const DESUGAR: &str = "crates/hale-syntax/src/desugar.rs";
const API_GEN: &str = "crates/hale-syntax/src/api_gen.rs";
const PARSER: &str = "crates/hale-syntax/src/parser.rs";
const M_DISPATCH: &str = "crates/hale-model/src/dispatch_plan.rs";
const M_IDS: &str = "crates/hale-model/src/ids.rs";
const M_OBS: &str = "crates/hale-model/src/obs_ids.rs";
const IRIS_BUILD_RS: &str = "crates/hale-iris/build.rs";
const IRIS_LIB: &str = "crates/hale-iris/src/lib.rs";
const DNA_DIGEST: &str = "crates/hale-dna/src/digest.rs";

/// Every semantic family, in layer order.
pub const FAMILIES: &[Family] = &[
    // ----------------------------------------------------------- Parse
    Family {
        name: "seed_loading",
        layer: Layer::Parse,
        state: State::Migrating,
        kind: Kind::Desugar,
        answers: "Which source units form the snapshot: the entry, every imported seed, their merge order and the spans' virtual bases.",
        inputs: &[".hl files", "import directives", "the workspace root (hale.toml)", "editor overlays (LSP)"],
        producer: None,
        legacy: &[
            legacy(FRONTEND, "parse_with_imports", "the file entry: parse, EffectTable::from_seed, resolve_imports, alias scoping, qualified-path renames", "phase 2: the shared frontend owns loading"),
            legacy(FRONTEND, "collect_checkable", "the directory entry for `hale check`; short-circuits a single file with no imports", "phase 2"),
            legacy(V_BUILD, "run_build", "a hand-copied directory body inside the build verb", "phase 2: one loader"),
            legacy(V_RUN, "run_program", "a hand-copied directory body inside the run verb", "phase 2: one loader"),
            legacy(LSP, "analyze_seed", "the changed file's parent directory only; resolves no `import`; the same body is inlined into `check_and_publish`", "phase 2: the LSP loads through the shared frontend with an overlay source provider"),
        ],
        consumers: &[consumer("check"), consumer("build"), consumer("run"), consumer("test"), consumer("replay"), consumer("bench"), consumer("lsp"), consumer("dna (via the CLI)")],
        invariants: &[
            "one loader, one merge order, for every entry point",
            "an unresolved import is a diagnostic, never a silently smaller program",
        ],
        missing: Missing::NotApplicable,
        tests: &["crates/hale-cli/tests/imports.rs (diamond_import, three_hop_import, import_library_key)", "crates/hale-cli/tests/source_map.rs"],
        spec: &["spec/projects.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "qualified_names",
        layer: Layer::Parse,
        state: State::Migrating,
        kind: Kind::Desugar,
        answers: "What a qualified or aliased name denotes: the library identity, the mangled declaration, the construction target, the bus subject a path names.",
        inputs: &["import aliases", "the seed cache", "hale_stdlib::PATH_RENAMES", "declaration names"],
        producer: Some(site(IMPORTS, "resolve_imports")),
        legacy: &[
            legacy(IMPORTS, "lib_canonical_id", "library identity by path, falling back to the file name outside a workspace (can collide)", "the snapshot's seed names the library (phase 2, when the frontend owns loading)"),
            legacy(CHECK, "construction_target", "one alias hop in the top scope", "one alias resolution shared by checker and lowering"),
            legacy(TY_MANGLE, "resolve_construction_aliases", "rewrites construction sites to the target name in the frontend's resolved-program step; the checker resolves the same alias for itself in `construction_target`", "one alias resolution shared by checker and lowering"),
            legacy(TY_RESOLVED, "resolve_qualified_bus_subjects", "rewrites qualified bus subjects in the resolved-program step's clone", "one resolution, shared"),
            legacy(RESOLVE, "resolve_bus_subject", "the checker's resolution of the same subjects", "one resolution, shared"),
            legacy(CHECK, "imported_fn", "an imported fn's signature by path-string vector", "one resolution, shared"),
        ],
        consumers: &[consumer("check"), consumer("build"), consumer("lsp (hover, definition, references)")],
        invariants: &["a name resolves once per snapshot; the checker and lowering see the same target"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/import_library_key.rs", "crates/hale-codegen/tests/cross_seed_imports.rs", "crates/hale-types/tests/type_alias.rs"],
        spec: &["spec/semantics.md § Cross-seed namespace resolution", "spec/projects.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "desugar_sequence",
        layer: Layer::Parse,
        state: State::Migrating,
        kind: Kind::Desugar,
        answers: "Which rewrites the program receives before checking, in which order: JSON parsers, the api surface, topic desugars, intra-locus rewrites, repr accessors, the omitted `run`, unit returns.",
        inputs: &["the merged program", "--api / --env (roles)"],
        producer: None,
        legacy: &[
            legacy(V_CHECK, "run_check_impl_labelled", "check: json_gen, sync inference, generate_api(roles = None)", "phase 2: one sequence in the shared frontend"),
            legacy(V_BUILD, "run_build", "build: json_gen, inject_api_entry, bind_build_env (roles), then sync inference; a different order from check", "phase 2"),
            legacy(V_RUN, "compile_and_exec", "run <file>: no json_gen, no api, no sync inference before the check", "phase 2"),
            legacy(V_TEST, "compile_test_binary", "test: file entry only, no desugars before the check", "phase 2"),
            legacy(V_REPLAY, "parse_file", "replay: file entry only", "phase 2"),
            legacy(V_BENCH, "run_bench_file", "bench: a synthesized text driver and no check at all", "phase 2"),
            legacy(LSP, "check_and_publish", "the LSP: json_gen and sync inference per file, then generate_api", "phase 2"),
            legacy(TY_RESOLVED, "resolve_program", "the frontend's resolved-program step re-runs json_gen, api injection, generate_api, the topic and intra-locus desugars and repr accessors on its own clone, after the check ran over the un-desugared program; the intra-locus rewrite returns what it rewrote, kept as `intra_locus` and recorded on the bus graph's subjects (`direct_sends`)", "one sequence, before the check (phase 2)"),
            legacy(TY_RESOLVED, "normalize_unit_return_annotations", "unit-return normalization in the resolved-program step", "phase 2"),
            legacy(DESUGAR, "desugar_omitted_run", "the omitted `run` is synthesized in the resolved-program step, after the stdlib merge; the checker never sees it", "the sequence runs once, before the check"),
            legacy(CG, "build_executable_with_options", "codegen resolves the program for itself when handed a bare one (the test harness builds this way); its seam allows only the definition, so no non-test caller bypasses the frontend (tests are not scanned by the seam guard)", "the frontend is the only producer (phase 2)"),
        ],
        consumers: &[consumer("check"), consumer("build"), consumer("run"), consumer("test"), consumer("replay"), consumer("lsp"), consumer_at("codegen", CG, "build_resolved")],
        invariants: &[
            "one order, run once per snapshot, before the first law is judged",
            "codegen never re-desugars",
        ],
        missing: Missing::NotApplicable,
        tests: &["crates/hale-cli/tests/api_description.rs", "crates/hale-codegen/tests/framework_elision.rs"],
        spec: &["spec/semantics.md"],
        owned: &[site(DESUGAR, "desugar_intra_locus_topics")],
        seams: &[
            Seam { symbol: "resolve_program(", allowed: &[(TY_RESOLVED, 1), (CG, 1), (V_BUILD, 1), (V_RUN, 1), (V_TEST, 1), (V_BENCH, 1), (V_REPLAY, 1)] },
            Seam { symbol: "desugar_intra_locus_topics(", allowed: &[(DESUGAR, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "build_executable_with_options(", allowed: &[(CG, 1)] },
        ],
    },
    Family {
        name: "sync_inference",
        layer: Layer::Parse,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which sync discipline each `@form(hashmap)` slot gets when the author declared none, from the pools its methods are called from.",
        inputs: &["placement (the pool map)", "top_scope", "form declarations", "method call sites"],
        producer: Some(site(SYNC, "infer_sync_for_bundle")),
        legacy: &[
            legacy(TLIB, "apply_sync_inference", "injects the inferred `sync =` FormArg into the AST — the only analysis result codegen receives, and only when a verb ran it (check, build, run <dir>, the LSP per file; never run <file>, test, replay, bench)", "the inferred discipline is a row lowering reads; no AST mutation"),
            legacy(CHECK, "form_has_explicit_sync_discipline", "the checker's `has a sync discipline` predicate (one caller, the F.31 single-thread check)", "one predicate over the form rows"),
            legacy(SYNC, "form_has_explicit_sync", "sync inference's own predicate, which counts `sync = none` where the checker's does not", "one predicate over the form rows"),
        ],
        consumers: &[consumer("check (F.31 cross-pool verdicts)"), consumer_at("codegen", CG_DECL, "sync_mode"), consumer("lsp")],
        invariants: &["every entry point sees the same discipline for the same program"],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/placement.rs"],
        spec: &["spec/forms.md", "spec/semantics.md § Placement block (F.31)"],
        owned: &[],
        seams: &[Seam { symbol: "apply_sync_inference(", allowed: &[(TLIB, 4), (V_CHECK, 1), (V_BUILD, 1), (V_RUN, 1), (LSP, 1)] }],
    },
    Family {
        name: "effect_class_table",
        layer: Layer::Parse,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "The union of user effect classes across seeds, with `User(i)` indices remapped so one class has one index.",
        inputs: &["effect_names / defs per program"],
        producer: Some(site(V_CHECK, "EffectTable")),
        legacy: &[
            legacy(FRONTEND, "merge_programs", "the merge remaps class indices by name", "the table is a declaration-layer row keyed by identity"),
            legacy(EFFECTS, "effect_names_of", "takes the first non-empty program's table; expansion of a class is copied five times across hale-types", "one expansion"),
        ],
        consumers: &[consumer("check"), consumer("effects"), consumer("claims")],
        invariants: &["one class, one index, per snapshot"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/cross_seed_effects.rs"],
        spec: &["spec/verification.md § Default-on & opt-in analyses"],
        owned: &[],
        seams: &[],
    },
    // ---------------------------------------------------- Declarations
    Family {
        name: "top_scope",
        layer: Layer::Declarations,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "What every top-level name denotes: the symbol table over the merged program.",
        inputs: &["the merged program", "import renames"],
        producer: Some(site(RESOLVE, "build_top_scope")),
        legacy: &[
            legacy(TLIB, "check_bundle_opts_scoped", "built here for the checker and never passed on: model_builder, sync inference, the resolved program (once, for the ownership graph and the bus graph) and the LSP (seven times) rebuild it", "one scope per snapshot, passed by reference (phase 2)"),
            legacy(CHECK, "collect_known_names", "a second name table the checker keeps beside the scope", "one table"),
        ],
        consumers: &[consumer_at("check", CHECK, "check_bundle_scoped"), consumer_at("model", MODEL_BUILDER, "derive_application_model"), consumer_at("resolved program (lowering)", TY_RESOLVED, "build_top_scope"), consumer_at("lsp", LSP, "build_top_scope")],
        invariants: &["one namespace decision: module-nested declarations and imported seeds resolve the same way everywhere"],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/checks_inside_modules.rs", "crates/hale-cli/tests/check_unknown_identifier.rs"],
        spec: &["spec/semantics.md"],
        owned: &[],
        seams: &[Seam { symbol: "build_top_scope(", allowed: &[(RESOLVE, 1), (TLIB, 3), (MODEL_BUILDER, 1), (SYNC, 1), (TY_RESOLVED, 1), (LSP, 7)] }],
    },
    Family {
        name: "expression_typing",
        layer: Layer::Declarations,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from.",
        inputs: &["top_scope", "declarations", "bodies"],
        producer: Some(site(CHECK, "check_bundle_scoped")),
        legacy: &[
            legacy(CG, "infer_accumulator_inner_type", "codegen infers an accumulator's element type again from lowered values where the checker's type is not carried across", "the resolved program carries the checker's types"),
        ],
        consumers: &[consumer("every layer"), ],
        invariants: &[
            "expression typing is not a layer: it is the derivation inside layer 3 that produces typed edges, and it stays Rust (final direction)",
            "codegen types a value only where the checker's type is not yet carried across (the accumulator case); that residue is deleted when the resolved program carries types",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/codegen_fixtures_typecheck.rs", "crates/hale-codegen/tests/corpus_check_build_agreement.rs"],
        spec: &["spec/types.md"],
        owned: &[site(RESOLVE, "infer_literal_ty")],
        seams: &[],
    },
    Family {
        name: "generics",
        layer: Layer::Declarations,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which monomorph a generic call instantiates and how its bindings unify.",
        inputs: &["generic declarations", "call arguments", "the mangled token vocabulary"],
        producer: Some(site(CHECK, "unify_generic_ty")),
        legacy: &[
            legacy(CG, "unify_generic_param_bindings", "a Ty-level mirror of the checker's unification, by its own comment", "codegen reads the checker's monomorph table"),
            legacy(CG, "infer_generic_fn_args", "codegen infers generic arguments again from lowered types", "same"),
            legacy(CHECK, "resolve_generic_monomorph", "the template lookup parses mangled `Name_Tok` strings; tables are per program, not per snapshot", "keyed by identity, per snapshot"),
        ],
        consumers: &[consumer("check"), consumer("codegen")],
        invariants: &["one unification; the monomorph set is a row lowering reads"],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/generic_monomorph_agreement.rs"],
        spec: &["spec/types.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "surfaces",
        layer: Layer::Declarations,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "Which surface is visible at which depth edge: contract exposure, interface conformance, perspective designation and `serves` conformance.",
        inputs: &["type, interface, contract, perspective declarations", "locus members"],
        producer: Some(site(CHECK, "check_structural_impl")),
        legacy: &[
            legacy(CHECK, "check_satisfies_bus_adapter", "a second copy of the structural-impl check (its own comment: same logic)", "one conformance function"),
            legacy(CG_TYPES, "locus_satisfies_interface", "codegen decides interface conformance by method names only, for storage routing", "codegen reads the conformance row"),
        ],
        consumers: &[consumer_at("check", CHECK, "check_contract_expose_validity"), consumer_at("check", CHECK, "check_serves_conformance"), consumer_at("check", CHECK, "check_reperspective"), consumer("codegen (vtable swap, storage routing)")],
        invariants: &["F.8 compatibility, F.14 and F.20 satisfaction are judged once, with a witness"],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/perspective_serves.rs", "crates/hale-types/tests/duplicate_member.rs"],
        spec: &["spec/types.md", "spec/semantics.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "forms",
        layer: Layer::Declarations,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "Whether a form's shape, its capacity slots and its projection class are well formed, and which operation set closes each slot.",
        inputs: &["@form arguments", "capacity declarations", "indexed_by", "sync discipline"],
        producer: Some(site(CHECK, "check_form_shape")),
        legacy: &[
            legacy(CG_DECL, "sync_mode", "codegen reads the form's arguments again to choose the slot layout", "codegen reads the form rows"),
        ],
        consumers: &[consumer("check"), consumer("codegen (slot layout)"), consumer("sync_inference")],
        invariants: &["the operation set a form closes over a slot is a row: it is what a storage binding (F.44) will need"],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/reserved_locus_members.rs", "crates/hale-codegen/tests/form_vec_bce.rs"],
        spec: &["spec/forms.md", "spec/memory.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "stdlib_surface",
        layer: Layer::Declarations,
        state: State::Migrating,
        kind: Kind::Capability,
        answers: "What each stdlib function is: its signature, its effect classes, whether it blocks, and what a value of a type can be rendered as.",
        inputs: &["the stdlib registry", "hale_stdlib::PATH_RENAMES", "the parsed stdlib source"],
        producer: Some(site(STDLIB_SURFACE, "signature_for")),
        legacy: &[
            legacy(CG, "lower_stdlib_path_call_expr", "271 `[\"std\", ..]` literals dispatch stdlib calls inside codegen; the registry's own comment calls this dispatch `reality`", "codegen dispatches from the registry row"),
            legacy(CG, "lower_stdlib_path_call", "the statement-form twin of the expression dispatch: 191 more `[\"std\", ..]` literals", "same"),
            legacy(CG_CHANNELS, "lower_fallible_call", "the fallible-call dispatch, a third copy of the stdlib call shapes (150 literals)", "same"),
            legacy(CG, "value_to_string_supports", "the printable set, kept in lockstep by hand with the checker's `ty_is_printable`", "one predicate"),
            legacy(CHECK, "ty_is_printable", "the checker's copy of the printable set", "one predicate"),
            legacy(CG, "declare_builtin_closure_violation_type", "a hand-maintained mirror of the checker's injected builtin types", "one declaration"),
            legacy(STDLIB_BODIES, "summarize_with_stdlib", "the stdlib merge for analysis, parsed again per consumer (model builder, LSP, codegen each merge)", "the stdlib is part of the snapshot, merged once"),
            legacy(STDLIB_BODIES, "summarize_with_stdlib_and_renames", "the rename-aware variant of the same merge", "same"),
        ],
        consumers: &[consumer_at("effects", EFFECTS, "effects_for"), consumer_at("frontier", FRONTIER, "effects_for"), consumer("codegen"), consumer("lsp (hover, completion)"), consumer("doc")],
        invariants: &["the checker and codegen agree on every stdlib call shape (parity test) and on the printable set (corpus agreement)"],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/stdlib_registry_parity.rs", "crates/hale-codegen/tests/corpus_check_build_agreement.rs", "crates/hale-cli/tests/doc_effects_catalogue.rs"],
        spec: &["spec/stdlib.md"],
        owned: &[],
        seams: &[],
    },
    // ------------------------------------------------------------ Locus
    Family {
        name: "entrypoint",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which locus is the program's `main`, whether the world is closed, and which declarations are imported.",
        inputs: &["locus declarations (is_main, imported, the __lib_ prefix)"],
        producer: None,
        legacy: &[
            legacy(CHECK, "check_main_and_bindings", "defines `imported main` via `l.imported`", "one definition of the entry, as a row"),
            legacy(CHECK, "check_pinned_locus_in_loop", "finds main by a `__lib_` name filter", "same"),
            legacy(CHECK, "compute_pool_of_locus_type", "finds main with no filter; the last one wins", "same"),
            legacy(CHECK, "check_bus_graph", "closed world = a top-level main only; a module-nested main disables rule 9", "same"),
            legacy(V_MATRIX, "seed_entry_kind", "parse-only main detection for the check matrix", "same"),
            legacy(OPTIONS, "bind_build_env", "has_main for the build env", "same"),
            legacy(CG, "collect_main_placement", "`is_main && !__lib_` over flat declarations", "same"),
            legacy(CG_INST, "let is_main_locus", "`is_main_locus` compares type names at instantiation (and twice more in dissolve.rs)", "same"),
            legacy(CG_DISSOLVE, "let is_main_locus", "the same comparison in the cascade", "same"),
        ],
        consumers: &[consumer("check"), consumer("build"), consumer("dna"), consumer("codegen")],
        invariants: &["nine sites use three definitions today; the row has one"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/entry_point_placement.rs", "crates/hale-types/tests/bus_graph.rs"],
        spec: &["spec/semantics.md § Bundle-wide rules"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "ownership",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array.",
        inputs: &["locus declarations (params, accept, release)", "bodies (let, assign, return, field initialisers, placement entries)", "fresh factories (one producer)", "returned bindings"],
        producer: Some(site(TY_OWN, "resolve_owners")),
        legacy: &[
            legacy(OWNERSHIP_GRAPH, "build_ownership_graph", "which accepting ancestor owns a method-body birth, keyed (enclosing locus, child type) by name, the child type resolved by `child_locus_name`; built once in the resolved program for lowering and once more by the model builder inside `hale check`, over the un-desugared bundle", "one ownership table with both relations; the model reads it when the check runs over the resolved program (phase 2)"),
            legacy(MODEL_BUILDER, "Owns", "the model's params-field tree from main, a third ownership account", "projected from the one table"),
            legacy(TY_OWN, "extend_fresh_factories", "the carrier-arm fixpoint that widens the factory set", "lowering-only today: the carrier fold widens the set lowering reads; the checker reads the unextended set (phase 2)"),
            legacy("crates/hale-types/src/borrow_lifetime.rs", "accepts", "the borrow-lifetime law rebuilds the accept sets from the AST for itself", "reads `accepts_ancestor`"),
            legacy(CHECK, "check_unowned_subscriber_locus", "the unowned-subscriber rule over its own name-keyed locus index; skipped by `--allow-unowned-subscriber` on some verbs and hard-coded off on others", "a law over the table, on every entry point"),
            legacy(CG_INST, "parent_accepts_us", "a monomorphised parent reads its own accept param: graph rows are per template", "the graph keys rows by the template's identity and lowering asks by it (phase 2)"),
        ],
        consumers: &[consumer_at("codegen", CG_INST, "site_owner"), consumer_at("borrow_lifetime", "crates/hale-types/src/borrow_lifetime.rs", "borrow_lifetime_diags"), consumer("model"), consumer("alloc_summary (eager-only accept sets)")],
        invariants: &[
            "a locus instantiation with no row is a CodegenError (F.39)",
            "ids, not names or spans: declarations are cloned and the stdlib's coordinates overlap user files",
            "the ownership matrix stays green with an empty KNOWN_OPEN",
            "`fresh_factories` is read by lowering and the checker with the bundle's import renames; the carrier fold (`extend_fresh_factories`) is lowering-only",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/owner_table.rs", "crates/hale-codegen/tests/ownership_matrix.rs", "crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding)", "crates/hale-codegen/tests/ownership_bubble.rs"],
        spec: &["spec/decisions.md F.39", "spec/semantics.md § Dissolve timing rules"],
        owned: &[site(TY_OWN, "resolve_binding_facts"), site(OWNERSHIP_GRAPH, "bubble_plans"), site(OWNERSHIP_GRAPH, "compute_forwarding_sets"), site(OWNERSHIP_GRAPH, "classify_owner_kind"), site(OWNERSHIP_GRAPH, "classify_edge"), site(TY_OWN, "fresh_factories")],
        seams: &[
            Seam { symbol: "resolve_owners(", allowed: &[(TY_RESOLVED, 1), (TY_OWN, 1)] },
            Seam { symbol: "build_ownership_graph(", allowed: &[(OWNERSHIP_GRAPH, 1), (MODEL_BUILDER, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "fresh_factories(", allowed: &[(TY_RESOLVED, 1), (TY_OWN, 1), (CHECK, 1)] },
            Seam { symbol: "resolve_binding_facts(", allowed: &[(TY_OWN, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "bubble_plans(", allowed: &[(OWNERSHIP_GRAPH, 1), (TY_RESOLVED, 1)] },
        ],
    },
    Family {
        name: "bus_graph",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates.",
        inputs: &["topics", "bus blocks", "sends", "bindings", "placement (for gates)"],
        producer: Some(site(BUS_GRAPH, "build_bus_graph")),
        legacy: &[
            legacy(CHECK, "check_bus_graph", "rule 9 runs `collect_bus_walk` itself", "one graph per snapshot"),
            legacy(CHECK, "check_bus_cycles", "rule 10 keeps its own adjacency (`BusAdj`) instead of reading the graph", "a law over the graph"),
            legacy(CHECK, "external_subscription_handlers", "handler discovery joined with `::` where the graph uses the last segment", "one subject key"),
            legacy(MODEL_BUILDER, "build_bus_graph", "rebuilt for the model", "phase 2 (the check over the resolved program)"),
            legacy(LSP, "build_bus_graph", "rebuilt for hale/busGraph without sync inference, so eligibility can disagree with the diagnostics pass", "phase 2"),
            legacy(TY_RESOLVED, "build_bus_graph", "built once in the resolved program, over the desugared program, for lowering; the model and the LSP still build their own over the un-desugared bundle, and the checker's rule 9 walks the bus itself", "one graph per snapshot (phase 2: the check over the resolved program)"),
            legacy(CG, "intra_locus_publish_target", "lowering classifies a handler-named method call with a struct argument as a rewritten publish for the probes and the reclaimed subregion, re-deriving the relation the resolved program records", "lowering reads `ResolvedProgram::intra_locus` by the call's id, shadowed against this classification over the corpus (phase 2)"),
        ],
        consumers: &[consumer("check (rules 9-12, 19)"), consumer("model"), consumer("topology"), consumer("dispatch"), consumer("lsp (hale/busGraph)"), consumer("bus_inert")],
        invariants: &["one graph, over one program shape, per snapshot; rule 10's cycle graph is a query over it", "the intra-locus rewrite is a relation on the graph, never an erased publisher (boundary 7)"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/bus_graph.rs", "crates/hale-types/tests/bus_payload_handler.rs", "crates/hale-codegen/tests/bus_devirt_differential.rs"],
        spec: &["spec/semantics.md rules 9-12, 19", "spec/verification.md § Bus-graph property checks"],
        owned: &[site(BUS_GRAPH, "dispatch_gates")],
        seams: &[
            Seam { symbol: "build_bus_graph(", allowed: &[(BUS_GRAPH, 1), (MODEL_BUILDER, 1), (LSP, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "collect_bus_walk(", allowed: &[(BUS_GRAPH, 2), (CHECK, 1)] },
            Seam { symbol: "dispatch_gates(", allowed: &[(BUS_GRAPH, 1), (TY_RESOLVED, 1)] },
        ],
    },
    Family {
        name: "topics",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "What each topic is on the wire: its subject, payload contract, routing key, bounds and shed policy; and which topic a send's subject names.",
        inputs: &["topic declarations", "send subjects", "subscribe bounds", "bindings (shm_ring)"],
        producer: Some(site(TOPIC_ID, "topic_wire_subjects")),
        legacy: &[
            legacy(CHECK, "check_send", "subject → topic by a first match over a name-ordered map; ambiguous when two topics share a wire subject", "one join on the subject row"),
            legacy(CHECK, "check_phase3_fallback_subscribers", "a second subject → topic table (`by_wire`, last writer wins)", "same"),
            legacy(RESOLVE, "resolve_bus_subject", "wire-subject derivation that differs from topic_identity on a cycle or a missing parent", "one derivation"),
            legacy(CG, "collect_topic_wire_subjects", "codegen recomputes wire subjects five times per build, plus its shm-ring, routing-key and bound tables", "codegen reads the topic rows"),
            legacy(CG, "collect_shm_ring_subjects", "the shm-ring table", "same"),
            legacy(CG, "collect_routing_key_subjects", "the routing-key table", "same"),
            legacy(MODEL_BUILDER, "topic_wire_subjects", "rebuilt for the model", "phase 2"),
        ],
        consumers: &[consumer("check"), consumer("model"), consumer("codegen (dispatch, bindings, runtime registration)"), consumer("topology (topic shapes)"), consumer_at("resolved program (the intra-locus relation's wire subjects)", TY_RESOLVED, "topic_wire_subjects")],
        invariants: &["delivery joins on the subject's identity, never on the written topic name (spec/model.md rule 8)"],
        missing: Missing::Hole,
        tests: &["crates/hale-codegen/tests/topic_declarations.rs", "crates/hale-codegen/tests/replica_keys.rs", "crates/hale-codegen/tests/serializer_shape.rs"],
        spec: &["spec/semantics.md § Topic declarations", "spec/semantics.md § Phase 3: routing keys"],
        owned: &[],
        seams: &[Seam { symbol: "topic_wire_subjects(", allowed: &[(TOPIC_ID, 2), (BUS_GRAPH, 1), (MODEL_BUILDER, 1), (TY_RESOLVED, 1), (CG, 2)] }],
    },
    Family {
        name: "bindings",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which topics are bound to which transport, in which role, with which codec, and whether the transport can carry the payload.",
        inputs: &["bindings blocks", "topics", "transport specs", "purity (codecs)"],
        producer: Some(site(CHECK, "check_main_and_bindings")),
        legacy: &[
            legacy(CHECK, "bound_topics", "the set of transport-bound topics, one of four copies (two more in the checker, one in bus_graph), disagreeing on imported mains", "one row"),
            legacy(CHECK, "collect_topic_pub_sub", "infers the binding role without calling the desugar's `binding_role_for`, which model_builder uses", "one role rule"),
            legacy(DESUGAR, "binding_role_for", "THE binding-role rule, by its own comment, applied at desugar and again by the model builder", "one role row"),
            legacy(CG, "emit_bindings_prelude", "codegen decides transport, adapter, codec and producer-vs-attach at emission, and refuses a role still `None`", "codegen reads the binding rows"),
            legacy(CHECK, "transport_satisfies", "the transport capability table", "a capability row in the matrix"),
        ],
        consumers: &[consumer("check"), consumer("model (binds)"), consumer("codegen"), consumer("api_surface")],
        invariants: &["F.36 and F.37: binding failure is structural; codec purity is a law over rows"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/binding_imported_main.rs", "crates/hale-codegen/tests/bindings_codec_clause.rs"],
        spec: &["spec/decisions.md F.36, F.37", "spec/semantics.md § Operational constraints (Form K)"],
        owned: &[],
        seams: &[Seam { symbol: "binding_role_for(", allowed: &[(DESUGAR, 1), (MODEL_BUILDER, 1)] }],
    },
    Family {
        name: "dispatch",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement.",
        inputs: &["bus_graph (gates)", "placement (domains)", "the flat-payload predicate", "--no-bus-devirt"],
        producer: Some(site(M_DISPATCH, "fn derive")),
        legacy: &[
            legacy(TY_RESOLVED, "from_gates", "the resolved program derives lowering's plan with an empty domain map (#464's widening is a separate optimization); the model derives its own with the arrangement's domains for `same_domain`", "one plan (phase 2)"),
            legacy(CG_WIRE, "bus_payload_is_flat", "the third leg of the direct-call gate exists only in codegen", "a gate column"),
        ],
        consumers: &[consumer_at("codegen", CG, "build_resolved"), consumer_at("codegen", "crates/hale-codegen/src/bus/dispatch.rs", "bus_devirt"), consumer_at("exec_digest (the resolved program's plan)", OPTIONS, "resolved.plan.digest()"), consumer("model dump")],
        invariants: &["which flavour a subject gets is a plan conclusion, never a model row (spec/model.md)", "lowering reads one plan, derived once per snapshot in the resolved program; the execution digest frames that plan", "the model's plan agrees with it on the flavor of every subject both carry (shadowed over the corpus at phase 1.5)"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/dispatch_plan_cli.rs", "crates/hale-codegen/tests/bus_devirt_direct.rs"],
        spec: &["spec/model.md § Derived products", "spec/decisions.md F.38"],
        owned: &[],
        seams: &[
            Seam { symbol: "DispatchPlan::derive(", allowed: &[(MODEL_BUILDER, 1)] },
            Seam { symbol: "from_gates(", allowed: &[(M_DISPATCH, 2), (TY_RESOLVED, 1)] },
        ],
    },
    Family {
        name: "handler_routing",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which `on_failure` handler a failing child's locus type reaches, and from which parent.",
        inputs: &["failure declarations", "declared loci and type aliases (the bundled stdlib's loci included)", "import renames", "ownership (the supervising parent instance, in lowering)"],
        producer: Some(site(HANDLER_ROUTING, "handler_rows")),
        legacy: &[
            legacy(CG_CHANNELS, "failure_handler_for", "looks the row up by ordinal in the parent's handler table (one fn per row, built in declare_locus_methods; a monomorph reads its template's rows)", "the handler fn is a column of the row"),
            legacy(CG_CHANNELS, "resolve_failure_route", "the parent instance is the lowering context's (supervising parent, then self, then params-init self); the handler is the row's", "the instance is a row of the instance tree (phase 2)"),
            legacy(CG, "__StdBusUnixConnectTransport", "the transport-loss handler is picked by name through the routing table", "the bindings family names the transport's locus"),
        ],
        consumers: &[consumer_at("codegen (the handler table)", CG_DECL, "handlers_of"), consumer_at("codegen (__parent_on_failure)", CG_CHANNELS, "resolve_failure_route"), consumer_at("codegen (restart in place)", CG_RESTART, "restarts_in_place"), consumer_at("model (supervises)", MODEL_BUILDER, "Supervises"), consumer_at("check (duplicate handlers)", CHECK, "check_duplicate_failure_handlers"), consumer_at("check (@supervised)", FRONTIER, "supervised_diags")],
        invariants: &["the child type is resolved once, by `child_locus_name`; lowering, the checker and the model read the same row"],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/lifecycle_flow.rs (on_failure_dispatch_by_child_type)", "tests/hale/on_failure_per_child_type_test.hl", "crates/hale-types/tests/violate.rs"],
        spec: &["spec/semantics.md § failure", "spec/runtime.md (failure delivery)"],
        owned: &[site(HANDLER_ROUTING, "child_locus_name")],
        seams: &[
            Seam { symbol: "handler_rows(", allowed: &[(HANDLER_ROUTING, 1), (TY_RESOLVED, 1), (CHECK, 1), (MODEL_BUILDER, 1), (FRONTIER, 1)] },
            Seam { symbol: "child_locus_name(", allowed: &[(HANDLER_ROUTING, 2), (OWNERSHIP_GRAPH, 1), (TY_OWN, 1)] },
            Seam { symbol: "DeclaredNames::of(", allowed: &[(HANDLER_ROUTING, 1), (OWNERSHIP_GRAPH, 1), (TY_OWN, 1)] },
        ],
    },
    Family {
        name: "flows",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which children are flows (released per completion) and which are resident.",
        inputs: &["release declarations", "accept declarations"],
        producer: Some(site("crates/hale-types/src/flows.rs", "survey")),
        legacy: &[
            legacy(CG_INST, "release_param", "codegen classifies a flow by `release_param == L` at three sites (run elision, run-end reclaim, the release call)", "codegen reads the flow row"),
            legacy(CHECK, "check_accept_release", "the checker's own accept/release survey by type name", "a law over the rows"),
        ],
        consumers: &[consumer("check --flows"), consumer("codegen")],
        invariants: &["flows.rs states it mirrors codegen (GH #736); the row ends the mirror"],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/release_reclaims_flow.rs", "crates/hale-codegen/tests/release_two_parents.rs"],
        spec: &["spec/semantics.md § release(c) and flow children"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "restart",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which loci declare restart operations, which restart in place, and what the restart bound is.",
        inputs: &["closure and birth-check declarations", "handler_routing (the rows' recovery ops)"],
        producer: Some(site(HANDLER_ROUTING, "handler_rows")),
        legacy: &[
            legacy(CG_RESTART, "locus_declares_failures", "which loci a failure can come from (and so get restart points) is a walk over closure and birth-check members in codegen", "a column of the restart rows"),
            legacy(CG, "RecoveryModifier::For", "the `for N` bound is lowered from the statement's own expression: lowering reads the bound from the statement; the row's `retry_bound` is the model's", "lowering reads the row"),
        ],
        consumers: &[consumer("codegen (__restart_<L>, __resume_<L>)"), consumer("model")],
        invariants: &["a recovery op is a row with a witness, per (parent, child)"],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/restart_in_place_params.rs", "crates/hale-codegen/tests/restart_bound.rs"],
        spec: &["spec/semantics.md § supervision"],
        owned: &[site(HANDLER_ROUTING, "recovery_ops")],
        seams: &[Seam { symbol: "recovery_ops(", allowed: &[(HANDLER_ROUTING, 2)] }],
    },
    Family {
        name: "closures",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "Whether each closure clause is well formed, and which lifecycle events (`epoch`, `persists_through`, `resets_on`) it names.",
        inputs: &["closure declarations", "lifecycle_order (the event alphabet)"],
        producer: Some(site(CHECK, "check_locus_member")),
        legacy: &[
            legacy("crates/hale-codegen/src/locus/closure.rs", "EpochSpec::Birth", "event names are matched ad hoc in codegen; a clause naming an event the locus never reaches is a silent no-op", "the clause joins the lifecycle table and an unreachable event is a law violation with a witness"),
        ],
        consumers: &[consumer("check"), consumer("codegen")],
        invariants: &["closures are a consumer of the layer-6 alphabet (RFC §2)"],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/closure_resets_per_epoch.rs", "crates/hale-types/tests/violate.rs"],
        spec: &["spec/semantics.md § closures"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "api_surface",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "The served surface: commands, reads, streams, their schemas, the roles that gate them, and the description's wire form.",
        inputs: &["@export, @gated, serve declarations", "the api binding", "roles (--env)"],
        producer: Some(site(API_GEN, "api_surface")),
        legacy: &[
            legacy(API_GEN, "generate_api", "run by check with no roles and by build with roles, so check's description and model never carry what build --env bakes in; codegen runs it again", "one surface per snapshot, with the configuration as an input"),
            legacy(CHECK, "check_api_roles", "role declarations, includes and gates are judged over the AST", "a law over the surface rows"),
            legacy(V_MATRIX, "role_coverage", "re-runs the loader and reads pre-desugar programs", "reads the rows"),
        ],
        consumers: &[consumer("check --dump-api"), consumer("build (the description the binding serves)"), consumer("describe / call / watch / admin"), consumer("ui (reserved)"), consumer("bundle (reserved)")],
        invariants: &["form, not params (I1): the description names what the program is, never where one copy listens", "perspective-invariant (I5)"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/api_description.rs", "crates/hale-types/tests/api_binding_check.rs"],
        spec: &["spec/model.md § The description", "spec/semantics.md § The api binding (GH #1106)"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "sealability",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "Which loci confine their state (`@sealed`), and which could.",
        inputs: &["locus declarations", "field accesses"],
        producer: Some(site(CHECK, "check_sealed_access")),
        legacy: &[
            legacy("crates/hale-types/src/sealability.rs", "survey", "the `--sealable` survey seals every locus, re-runs a partial check and PARSES THE DIAGNOSTIC MESSAGE TEXT to decide", "the survey reads the sealed-access rows"),
        ],
        consumers: &[consumer("check"), consumer("check --sealable"), consumer("claims (require sealed)")],
        invariants: &["a diagnostic's wording is never an input to a derivation"],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/sealed_locus.rs"],
        spec: &["spec/verification.md § Secrets — confine, classify, claim"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "runs_under",
        layer: Layer::Locus,
        state: State::Reserved,
        kind: Kind::Derivation,
        answers: "On whose authority a locus runs: the relation `runs_under(locus, principal)`, with principals declared by the program.",
        inputs: &["principal declarations (a later dialect)", "ownership (an undeclared locus inherits its owner's principal)"],
        producer: None,
        legacy: &[],
        consumers: &[consumer("claims (`only reaches(effects(secret(..))) via { runs_under(P) }`)"), consumer("placement (policy projections per principal)"), consumer("dna (positions instantiate principals)")],
        invariants: &["static, like ownership; a locus cannot run under a principal its owner does not hold", "the column is reserved so the tower's table never changes twice"],
        missing: Missing::NotApplicable,
        tests: &[],
        spec: &["RFC #1212, the authority comment"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "transitions",
        layer: Layer::Locus,
        state: State::Reserved,
        kind: Kind::Derivation,
        answers: "For an evented locus: the transition each handler is, input event to output set (F.41, after phase 2).",
        inputs: &["@evented declarations", "handler signatures"],
        producer: None,
        legacy: &[],
        consumers: &[consumer("bus_graph (exact cycles)"), consumer("effects (rate bounds)"), consumer("lifecycle_order (finite products)"), consumer("replay")],
        invariants: &["at most one output per delivery, as a type; fan-out as self-events"],
        missing: Missing::NotApplicable,
        tests: &[],
        spec: &["RFC #1212 §6 (F.41 sketch)"],
        owned: &[],
        seams: &[],
    },
    // ---------------------------------------------------------- Effects
    Family {
        name: "effects",
        layer: Layer::Effects,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which effect classes each fn and locus reaches (the callgraph fixpoint), the declared classes and their `causes:`/`depends:` DAG, and the certificate relating the two.",
        inputs: &["effect annotations", "the callgraph", "stdlib_surface (leaf effects)", "effect_class_table", "ffi names"],
        producer: Some(site(FRONTIER, "infer_effects")),
        legacy: &[
            legacy(MODEL_BUILDER, "infer_effects", "re-run for the model (merged program, with renames)", "phase: effects lane; one fixpoint per snapshot"),
            legacy(EFFECTS, "effect_manifest_with_inference", "re-run for the manifest and replay's live-effects gate (merged, no renames)", "same"),
            legacy(EVIDENCE, "derive_certificate_evidence", "the certificate engines run once in the check and again for evidence when claims exist", "measured once"),
            legacy(PURITY, "infer_purity_for_bundle", "purity, a sibling fixpoint over the same graph, computed on every check for codec laws", "a column of the effects rows"),
            legacy(FRONTIER, "infer_effects_lower_bound", "the lower-bound variant, a second walk", "one walk with two results"),
            legacy(CLAIMS, "direct_effects", "the claims' own direct-effect predicates", "read the rows"),
        ],
        consumers: &[consumer_at("check", CHECK, "check_decorator_stacks"), consumer("claims (certificate, causes, depends, budget)"), consumer("model (effect labels)"), consumer("replay (effect manifest)"), consumer("doc")],
        invariants: &["derived ⊆ declared is the certificate; budgets are judged through evidence (spec/verification.md)", "effects run once per snapshot, not once per consumer"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/effect_assertions.rs", "crates/hale-cli/tests/effects_baseline_gate.rs", "crates/hale-cli/tests/effects_manifest.rs"],
        spec: &["spec/verification.md § Default-on & opt-in analyses", "spec/verification.md § Claims"],
        owned: &[],
        seams: &[
            Seam { symbol: "infer_effects(", allowed: &[(FRONTIER, 4), (EFFECTS, 1), (CLAIMS, 1), (MODEL_BUILDER, 1), (TOPOLOGY, 1)] },
            Seam { symbol: "effect_manifest_with_inference(", allowed: &[(EFFECTS, 1), (TLIB, 1), (V_REPLAY, 1)] },
            Seam { symbol: "infer_purity_for_bundle(", allowed: &[(PURITY, 2), (CHECK, 1)] },
        ],
    },
    Family {
        name: "blocking",
        layer: Layer::Effects,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which fns block (a cooperative worker would be held), and whether the program places anything off the main thread.",
        inputs: &["stdlib_surface (holds_cooperative_worker)", "the callgraph", "placement"],
        producer: Some(site(CHECK, "blocking_path_match")),
        legacy: &[
            legacy(CHECK, "blocking_free_fns", "a name-keyed callgraph fixpoint for the BLOCK class, beside the effects fixpoint's own BLOCK propagation", "one fixpoint (the effects rows)"),
            legacy(CHECK, "blocking_self_methods", "the method half of the same fixpoint; no cross-locus hop", "same"),
            legacy(CG, "program_has_offthread", "codegen's predicate: its placement term calls the bus graph's `has_offthread_placement` (which walks modules); its bindings term scans top-level items only, so a module-nested main's socket binding is missed", "codegen reads the placement rows"),
            legacy(BUS_GRAPH, "has_offthread_placement", "the placement half of the same predicate, walking modules; a component of codegen's, not a second copy", "one placement table"),
        ],
        consumers: &[consumer("check (rules 7, 8)"), consumer("effects (@no_block)"), consumer("codegen (mark_pinned, no_pinned dispatch)")],
        invariants: &["one leaf set (GH #830) and one propagation"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/placement.rs", "crates/hale-codegen/tests/bus_devirt_no_pinned.rs"],
        spec: &["spec/semantics.md rules 7, 8"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "alloc_summary",
        layer: Layer::Effects,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Where each allocation lands and when it is reclaimed: per-fn allocation, escape, scratch eligibility, method-scratch elision, stack arrays, arena elision.",
        inputs: &["bodies", "signatures (non_allocating, fallible, ffi)", "the callgraph", "ownership (accept sets)"],
        producer: Some(site(ALLOC, "summarize_programs")),
        legacy: &[
            legacy(ALLOC, "summarize_programs_with_renames", "the rename-aware variant; the summary is built about twelve times per check with four input variants", "one summary per snapshot"),
            legacy(TLIB, "unbounded_alloc_warnings", "the diagnostics entry (check and the LSP's diagnostics), a different entry from the LSP's hale/allocSummary", "one entry"),
            legacy(CHECK, "check_hot_path_alloc", "hot-path allocation lint over a hand-kept receiver list, keyed by name and `__lib_` suffix", "a law over the rows"),
            legacy(CG, "compute_nonalloc_free_fns", "FORM-3 non-allocating free fns, a greatest fixpoint keyed by name", "codegen reads the rows (phase: effects lane)"),
            legacy(CG, "compute_scratch_local_free_fns", "which free fns may allocate in their own scratch arena; the checker's ReclaimScope model was left stale by it (#1208)", "same"),
            legacy(CG, "compute_elidable_methods", "methods whose scratch arena can be elided; recomputed on the fly per method by `method_scratch_elidable`", "same"),
            legacy(CG, "method_scratch_elidable", "the on-the-fly copy; lifecycle hooks are decided only here", "same"),
            legacy(CG, "locus_arena_elidable", "arena elision per locus, with an empty interprocedural context on purpose", "same"),
            legacy(CG, "let dbg = format!(\"{:?}\", f.body);", "the caller-arena TLS publish gate decides from the body's Debug string", "same"),
            legacy(ALLOC, "ReclaimScope", "the checker's reclaim model, stale for scratch-local fns since #1208", "one model"),
        ],
        consumers: &[consumer("check (unbounded allocation, hot path)"), consumer("lsp (hale/allocSummary)"), consumer("claims (@budget)"), consumer("codegen (arena routing, elision)"), consumer("resource_budget")],
        invariants: &["the checker's reclaim model and codegen's routing agree; a stale copy is a registry violation, not a comment"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/hot_path_alloc.rs", "crates/hale-codegen/tests/scratch_local_free_fn.rs", "crates/hale-codegen/tests/fn_nonalloc_add.rs", "crates/hale-codegen/tests/method_scratch_elision.rs"],
        spec: &["spec/memory.md § Allocation routing", "spec/styleguide.md"],
        owned: &[],
        seams: &[
            Seam { symbol: "summarize_programs", allowed: &[(ALLOC, 5), (TLIB, 1), (LSP, 1), ("crates/hale-types/src/budget_check.rs", 1), (FRONTIER, 1), (MODEL_BUILDER, 1), ("crates/hale-types/src/quantitative.rs", 1), ("crates/hale-types/src/resource_budget.rs", 2), (STDLIB_BODIES, 1), (TOPOLOGY, 1)] },
            Seam { symbol: "unbounded_alloc_warnings(", allowed: &[(TLIB, 1), (V_CHECK, 1), (LSP, 1)] },
        ],
    },
    Family {
        name: "borrow_lifetime",
        layer: Layer::Effects,
        state: State::Canonical,
        kind: Kind::Law,
        answers: "Whether a borrowed handle outlives its holder (GH #730), decided from position over the owner structure.",
        inputs: &["ownership (Borrowed rows)", "bodies"],
        producer: Some(site("crates/hale-types/src/borrow_lifetime.rs", "borrow_lifetime_diags")),
        legacy: &[],
        consumers: &[consumer_at("check", V_CHECK, "borrow_lifetime_diags"), consumer_at("build, run, test, replay", TLIB, "check_bundle_for_build")],
        invariants: &["runs on every entry point: it does not run in the LSP or bench today (phase 2 closes that)", "its accept-set walk is an ownership residue, listed under `ownership` (borrow_lifetime.rs `accepts`)"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/check_borrow_lifetime.rs"],
        spec: &["spec/semantics.md § A borrow outlives its holder", "spec/decisions.md F.39"],
        owned: &[],
        seams: &[Seam { symbol: "borrow_lifetime_diags", allowed: &[("crates/hale-types/src/borrow_lifetime.rs", 3), (TLIB, 1), (V_CHECK, 1)] }],
    },
    Family {
        name: "bare_fallible",
        layer: Layer::Effects,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "Whether a fallible call's error is addressed.",
        inputs: &["expression types (Fallible)", "stdlib_surface (dual-mode calls)"],
        producer: Some(site("crates/hale-types/src/bare_fallible.rs", "bare_fallible_calls")),
        legacy: &[
            legacy(CHECK, "check_expr_addressed", "the checker judges user fallibility while bare_fallible judges stdlib dual-mode calls the checker types as Unknown: two producers split by callee kind", "one law once stdlib calls are typed"),
        ],
        consumers: &[consumer_at("check", V_CHECK, "bare_fallible_calls"), consumer_at("build, run, test, replay", TLIB, "check_bundle_for_build")],
        invariants: &["runs on every entry point (not the LSP or bench today)"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/check_strict_fallible.rs"],
        spec: &["spec/semantics.md § fallible"],
        owned: &[],
        seams: &[Seam { symbol: "bare_fallible_calls(", allowed: &[("crates/hale-types/src/bare_fallible.rs", 1), (TLIB, 1), (V_CHECK, 1)] }],
    },
    Family {
        name: "nonreturning",
        layer: Layer::Effects,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "Which `run()` bodies never return, which children are long-running, and whether the birth order or a pool starves because of it.",
        inputs: &["run bodies", "params order", "placement"],
        producer: Some(site(CHECK, "run_statically_nonreturning")),
        legacy: &[
            legacy(CHECK, "check_nested_long_running_child", "a second `long-running` predicate (a hand table naming std::http::Server) that disagrees with the first", "one predicate"),
            legacy(CHECK, "check_cooperative_pool_blocking", "the starvation and birth-order phases live inside the blocking check", "laws over rows"),
        ],
        consumers: &[consumer("check")],
        invariants: &["one definition of long-running"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/birth_order_trap.rs", "crates/hale-codegen/tests/birth_order_trap.rs"],
        spec: &["spec/semantics.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "working_set",
        layer: Layer::Effects,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "The estimated working set per locus and program, and the locality law over it.",
        inputs: &["declared shapes", "@locality"],
        producer: Some(site("crates/hale-types/src/working_set.rs", "compute_program_working_set")),
        legacy: &[],
        consumers: &[consumer_at("build", V_BUILD, "compute_program_working_set"), ],
        invariants: &["build-only today, and a warning without --strict; layer 7 (layout) reads it later"],
        missing: Missing::Hole,
        tests: &["crates/hale-cli/tests/target_model.rs"],
        spec: &["spec/memory.md"],
        owned: &[site("crates/hale-types/src/working_set.rs", "compute_locus_working_set"), site("crates/hale-types/src/working_set.rs", "compute_program_returns_entry_per_locus")],
        seams: &[],
    },
    // -------------------------------------------------------- Placement
    Family {
        name: "placement",
        layer: Layer::Placement,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which thread domain each instance runs in: pools, pinned threads, replicas, affinity, and the deployment plan.",
        inputs: &["placement and topology blocks", "main's params", "entrypoint", "ownership (nested fields)"],
        producer: Some(site(CHECK, "compute_pool_of_locus_type")),
        legacy: &[
            legacy(CHECK, "enclosing_field_placement", "owner-relative placement, one of four in-checker derivations (two more inline in the blocking and single-thread checks)", "one placement table per snapshot (phase: placement lane)"),
            legacy(BUS_GRAPH, "collect_subscriber_placements", "per type, first wins", "same"),
            legacy(OWNERSHIP_GRAPH, "collect_placements", "a verbatim copy of the previous", "same"),
            legacy(MODEL_BUILDER, "PlacedIn", "the model's arrangement, per instance with replicas", "projected from the table"),
            legacy("crates/hale-types/src/resource_budget.rs", "budget_for_programs", "counts placement entries, ignores replicas", "reads the table"),
            legacy(DESUGAR, "collect_off_owner_thread_fields", "a desugar-time placement read", "reads the table"),
            legacy(CG, "collect_main_placement", "codegen's DeploymentPlan, keyed by field name and locus type name", "codegen reads the table"),
            legacy(CG_DEPLOY, "DeploymentPlan", "the plan type lowering reads today", "becomes the layer-5 table"),
        ],
        consumers: &[consumer("check (rules 2-5, 13-18; F.31)"), consumer("sync_inference"), consumer("dispatch (domains)"), consumer("model (placed_in, affined_to)"), consumer("codegen (pools, mailboxes, affinity)"), consumer("lsp (hale/placement)"), consumer("deployment (reserved)")],
        invariants: &["F.38: placement is semantics-free, so a backend may Approximate it", "placement is a choice point: v1's declared placement is the single candidate"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/placement.rs", "crates/hale-types/tests/placement_pairings.rs", "crates/hale-codegen/tests/pool_affinity.rs", "crates/hale-codegen/tests/placement_where_async_io.rs", "crates/hale-types/tests/shadow_placement.rs (the shadow of compute_pool_of_locus_type against collect_subscriber_placements over the corpus; 20 classified divergences, all known old bugs)"],
        spec: &["spec/semantics.md § Placement block (F.31)", "spec/decisions.md F.31, F.35, F.38"],
        owned: &[],
        seams: &[
            Seam { symbol: "compute_pool_of_locus_type(", allowed: &[(CHECK, 2), (TLIB, 1)] },
            Seam { symbol: "collect_main_placement(", allowed: &[(CG, 2)] },
        ],
    },
    Family {
        name: "target_capability",
        layer: Layer::Placement,
        state: State::Migrating,
        kind: Kind::Capability,
        answers: "What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability.",
        inputs: &["--target", "a source `target` declaration", "stdlib_surface", "FFI signatures"],
        producer: None,
        legacy: &[
            legacy(CHECK, "wasm_unavailable_stdlib", "a hand-kept slice-pattern table keyed by leading namespace, consulted only when the SOURCE declares `target wasm` (never from `--target wasm32`), and only for call forms", "one CapabilityMatrix consulted by the driver before lowering"),
            legacy(CHECK, "wasm_target", "the source-declaration flag the table is gated on", "same"),
            legacy(CG, "link_wasm", "link-time refusals (link_libs) and the export list", "same"),
            legacy(CG_INST, "lotus_replay_start_ingress", "one of the per-site wasm skips; instantiation still emits pool shutdown and wait-abort on wasm where the main exit does not", "same"),
            legacy(CG, "is_wasm", "the backend configuration scattered across a dozen sites", "same"),
            legacy(CHECK, "ffi_type_unportable", "FFI portability per type", "a capability row"),
            legacy(CG_TARGET, "TargetSpec", "has_async_io is true for wasm32; the checker sees the target only under `hale build`", "the matrix is the one statement, on every entry point"),
        ],
        consumers: &[consumer("check"), consumer("build"), consumer("docs (systems/webassembly.md, generated from the matrix)")],
        invariants: &["Approximate is legitimate only in layers 5 and 7; everywhere else a target lowers or rejects, with the row's witness", "the docs' target statement is generated, never hand-maintained"],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/wasm_target_gating.rs", "crates/hale-codegen/tests/wasm_target.rs", "crates/hale-cli/tests/target_model.rs"],
        spec: &["spec/decisions.md F.35", "docs/src/systems/webassembly.md"],
        owned: &[],
        seams: &[Seam { symbol: "wasm_unavailable_stdlib(", allowed: &[(CHECK, 2)] }],
    },
    Family {
        name: "deployment",
        layer: Layer::Placement,
        state: State::Reserved,
        kind: Kind::Derivation,
        answers: "A deployment as typed rows: root and horizon, component identities, instances and incarnations, resources and allocations, endpoints and routes, hosting and authority, persistence obligations (the habitat, after phase 2).",
        inputs: &["placement", "bindings", "admitted artifacts", "a deployment dialect (later)"],
        producer: None,
        legacy: &[],
        consumers: &[consumer("fleet (today's JSON as an input adapter)"), consumer("dna (the habitat)"), consumer("ui (reserved)")],
        invariants: &["habitat owns the hosting allocation and its obligations; the tenant keeps its own root and authority", "desired, admitted and observed stay distinct"],
        missing: Missing::NotApplicable,
        tests: &[],
        spec: &["RFC #1212, the implementation plan §8", "GH #262"],
        owned: &[],
        seams: &[],
    },
    // -------------------------------------------------------- Lifecycle
    Family {
        name: "lifecycle_order",
        layer: Layer::Lifecycle,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "The happens-before order per instance: birth sequence, params open and settle, failure delivery and its execution domain, reclaim prerequisites, drain, restart, teardown.",
        inputs: &["ownership", "placement", "handler_routing", "flows", "restart", "the runtime protocol (lotus_failure_hold / await / defer_reclaim)"],
        producer: None,
        legacy: &[
            legacy(CG_INST, "lower_locus_instantiation_inner", "the birth sequence is the order of emit calls in a 4,900-line function; its eager teardown spine is one of two copies", "an explicit action plan (compiler- and runtime-owned actions with domain, prerequisites, liveness, completion) read by emission"),
            legacy(CG, "emit_deferred_entry_teardown", "the deferred teardown spine, the second copy; #1208's pool join was added here after four other sites already had it", "same"),
            legacy(CG, "emit_frame_teardown", "the frame flush order (drain, wait-abort, pinned-first, reverse push)", "same"),
            legacy(CG, "__reclaim_", "the reclaim spine", "same"),
            legacy(CG_DISSOLVE, "emit_locus_arena_destroy", "the cascade (field drains, field dissolves, arena destroy)", "same"),
            legacy(CG_RESTART, "define_restart_fns", "restart and resume", "same"),
            legacy(LOTUS, "lotus_failure_hold", "the hold/settle/defer/await protocol in the C runtime; verified against the table by a debug-build oracle before the table is trusted", "the oracle holds"),
        ],
        consumers: &[consumer("codegen (emission reads the order)"), consumer("closures (the event alphabet)"), consumer("transitions (reserved)"), consumer("deployment (reserved)")],
        invariants: &[
            "handlers run only on the queue owner's thread, so cross-thread failure delivery follows spec/runtime.md (a typed bus message): the first named decision, with its own regression test",
            "a spec/implementation disagreement is settled as a named decision, never by extraction picking a side",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/lifecycle_flow.rs", "crates/hale-codegen/tests/reclamation_spine.rs", "crates/hale-codegen/tests/main_locus_deferred_pool_join.rs", "crates/hale-codegen/tests/teardown_pinned_join_order.rs"],
        spec: &["spec/runtime.md (failure delivery; pool join rule b)", "spec/semantics.md § lifecycle"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "bus_inert",
        layer: Layer::Lifecycle,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Whether the program can ever have a bus cell in flight, so drains can be elided.",
        inputs: &["bus_graph", "stdlib_surface (which namespaces publish)"],
        producer: None,
        legacy: &[
            legacy(CG, "let dbg = format!(\"{:?}\", program.items);", "decided by searching the program's Debug string for `__Std`, `name: \"std\"` and tainted namespaces", "a query over the message graph"),
            legacy(CG, "stdlib_bus_tainted_namespaces", "the stdlib taint fixpoint, also over Debug strings, cached per process", "a column of the stdlib_surface rows"),
            legacy(TY_RESOLVED, "user", "the resolved program carries the desugared user program a second time so lowering's tier-1 bus-inert scan reads the same Debug text it always did", "the bus-inert verdict is a row of the resolved program, computed structurally and shadowed against the scan (phase 2)"),
        ],
        consumers: &[consumer_at("codegen", CG_BUS_RT, "emit_bus_drain")],
        invariants: &["a drain elision is a conclusion of the message graph, never of a string"],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/drain_elision.rs", "crates/hale-codegen/tests/log_routing.rs"],
        spec: &["spec/runtime.md § drain"],
        owned: &[],
        seams: &[],
    },
    // --------------------------------------------------------- Lowering
    Family {
        name: "law_backstops",
        layer: Layer::Lowering,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "The checker rules lowering re-judges because `build_executable` never runs the checker: self-containment, cross-pool bare statements, placement entries, pinned loci in loops.",
        inputs: &["the AST", "the lowering context"],
        producer: None,
        legacy: &[
            legacy(CG_INST, "CodegenError::Unsupported", "spanless refusals at lowering for rules the checker already states (rule 6's checker evaluator landed in phase 0; the backstop stays for harness builds that skip the checker); for a placed locus the checker types as Unknown, for an `accept()` with no parameter (the checker keys on `accept_param`, codegen on the method name), and for an adapter locus instantiated inline in a `bindings { }` block (which lowering pins without a placement entry), it is the only evaluator", "one pipeline guarantees the checker ran before lowering (phase 2), and the refusals become dead"),
        ],
        consumers: &[consumer("codegen harness builds")],
        invariants: &["a law is judged once, with a span"],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/self_containing_locus.rs", "crates/hale-codegen/tests/self_containing_locus.rs"],
        spec: &["spec/semantics.md rules 6, 17, 18; GH #813, #876"],
        owned: &[],
        seams: &[],
    },
    // -------------------------------------------------------------- Law
    Family {
        name: "model",
        layer: Layer::Law,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "The canonical semantic model of a checked bundle: fifteen entity tables, seventeen relation tables, holes, capabilities, provenance (GH #476).",
        inputs: &["a checked bundle", "top_scope", "bus_graph", "ownership", "placement", "effects", "alloc_summary", "topics", "bindings"],
        producer: Some(site(MODEL_BUILDER, "derive_application_model")),
        legacy: &[],
        consumers: &[consumer_at("claims", JUDGMENT, "derive_application_model"), consumer_at("topology", TOPOLOGY, "derive_application_model"), consumer_at("model dump", V_CHECK, "derive_application_model"), consumer_at("dispatch, obs ids", OPTIONS, "derive_application_model"), consumer("fleet (admits the artifact, never the model)")],
        invariants: &[
            "one constructor; no artifact → model, no plan → model, no hand-authored model",
            "hale-model is rebuilt on hale-graph (phase 1.1a): its seed, source and provenance ids and its provenance store are the graph core's, re-exported under the model's paths; its canary allows that one dependency and no other",
            "demand-gated: a no-claims check builds no model (GH #476 criterion 1, pinned by demand_gate.rs); phase 2 rewrites the gate as per-family accounting",
            "the model re-runs every derivation it consumes today (it reads nothing from the checker): those are listed under their families",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/demand_gate.rs", "crates/hale-model/tests/architecture.rs", "crates/hale-types/tests/topology_projection.rs"],
        spec: &["spec/model.md"],
        owned: &[],
        seams: &[Seam { symbol: "derive_application_model(", allowed: &[(MODEL_BUILDER, 1), (JUDGMENT, 1), (TOPOLOGY, 1), (V_CHECK, 1), (OPTIONS, 1)] }],
    },
    Family {
        name: "claims",
        layer: Layer::Law,
        state: State::Migrating,
        kind: Kind::Law,
        answers: "Every user law: lowered claim rows, the judged verdicts over the model and evidence, constitution identities, and the artifact's law account.",
        inputs: &["model", "claim and constitution declarations", "evidence (certificates, budgets)", "effects"],
        producer: Some(site(JUDGMENT, "claim_law_diags")),
        legacy: &[
            legacy(TOPO_LAW, "validate_law_account", "the CLI recomputes the law digest and re-judges certificate and document verdicts when it admits an artifact: a second law authority", "admission validates ties and reads verdicts; it re-derives none"),
            legacy(V_MATRIX, "constitution_identities", "re-runs the loader, the scope and the bus graph to re-derive identities the artifact already carries", "reads the artifact's section"),
            legacy(CLAIMS, "constitution_identities", "the identity derivation the matrix calls", "one derivation, projected"),
        ],
        consumers: &[consumer("check / verify"), consumer("topology (law section)"), consumer("fleet"), consumer("dna (dna_law.rs wording)"), consumer("model diff")],
        invariants: &[
            "structural compiler laws are evaluated through model_query with shared witness rendering; the judgment path stays for user claims (final direction)",
            "a registered rule without an evaluator fails the compiler's own build",
            "a non-holds verdict is never silent",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/claim_diags_snapshot.rs", "crates/hale-cli/tests/law_selection_reaches_the_artifact.rs", "crates/hale-cli/tests/dna_law.rs", "crates/hale-types/tests/one_reachability_engine.rs"],
        spec: &["spec/verification.md § Claims", "spec/model.md § Adding a judgment family"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "view",
        layer: Layer::Law,
        state: State::Reserved,
        kind: Kind::Derivation,
        answers: "A named query over the tables: a node selector, a relation set and an adequacy policy, rendered by a backend (hale ui, after phase 2).",
        inputs: &["the resolved program's tables", "a view declaration (its own surface)"],
        producer: None,
        legacy: &[],
        consumers: &[consumer("ui (reserved)"), consumer("bundle (reserved)")],
        invariants: &["a view is a query, not a claim: it yields a projection with an adequacy verdict", "adequate_for(RelationSet): a view may not show an absence its relations cannot see; a hole renders as a hole with its reason"],
        missing: Missing::Hole,
        tests: &[],
        spec: &["RFC #1212, the UI comment"],
        owned: &[],
        seams: &[],
    },
    // --------------------------------------------------------- Identity
    Family {
        name: "snapshot_identity",
        layer: Layer::Identity,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again over the merged program in the resolved-program step with the bundle's seeds and a named seed for the bundled stdlib, idempotently (one numbering; the second mint numbers only what the first did not see), with reliable provenance.",
        inputs: &["seed_loading", "desugar_sequence"],
        producer: Some(site("crates/hale-types/src/snapshot.rs", "mint")),
        legacy: &[
            legacy(TY_OWN, "BindingKey", "use sites inside the returned-bindings walk are resolved by span (identifiers are not minted sites); the row is keyed by the `let`'s snapshot identity", "use-site identity (phase 1.1 follow-up)"),
            legacy(M_IDS, "FunctionId", "model ids are ranks in a sorted string order (`L::f`, `(name, kind)`, path strings)", "same"),
            legacy(EFFECTS, "FnKey", "analysis keys are (locus name, fn name)", "same"),
            legacy(CHECK, "type_expr_key", "rule 12 compares stringified TypeExprs", "same"),
        ],
        consumers: &[consumer("every table"), consumer("the shadow facility (compares through an explicit correspondence, never raw id equality)"), consumer("lsp (a later incremental future)"), consumer("the resolved program (codegen's input is minted over the merged program)")],
        invariants: &[
            "addresses are not identities (declarations are cloned); spans are not (the stdlib's coordinates overlap user files; desugars share spans)",
            "snapshot-local uniqueness and provenance are the requirement; persistent identity across editor revisions is a separate problem",
            "canonical ids need real equality and hashing; the AST's structural NodeId equality stays separate",
            "the identity's types are hale_graph::ids (SeedId, SiteId; phase 1.1a); hale_types::snapshot::mint numbers every site the AST walk hale_syntax::sites reaches with one counter: after the entry point's last desugar, and again, idempotently, over the merged program in the resolved-program step, which numbers only what the first mint did not see (phase 1.1b); every entry point (check, build, run, test, replay, bench, the LSP) calls it after its last desugar with the source map check builds and the bundle carries the result; the resolved program mints with the bundle's seeds, the stdlib's sites under the named seed snapshot::STDLIB_SEED (its spans overlap the first file's); a generated site records the desugar that made it (Snapshot::origins); codegen's generic instantiation keeps the template's id, and the F.39 pre-pass numbers nothing: an unminted Struct or Call is an error",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding)", "crates/hale-codegen/tests/owner_table.rs"],
        spec: &["spec/decisions.md F.39, F.40"],
        owned: &[],
        seams: &[Seam { symbol: "mint(", allowed: &[(TY_RESOLVED, 1), (V_CHECK, 1), (V_BUILD, 1), (V_RUN, 2), (V_TEST, 1), (V_REPLAY, 1), (V_BENCH, 1), (LSP, 1)] }],
    },
    Family {
        name: "digests",
        layer: Layer::Identity,
        state: State::Migrating,
        kind: Kind::Digest,
        answers: "Every identity a build or an artifact carries, and what each covers: shape_hash, artifact_digest, model_hash, exec_digest, the toolchain and cache keys, source digests.",
        inputs: &["the model half", "the artifact", "sources", "BuildOptions", "compiler sources"],
        producer: Some(site(TOPOLOGY, "model_shape_hash")),
        legacy: &[
            legacy(TOPOLOGY, "fn model_shape_hash", "renders the whole artifact and scrapes `shape_hash` out of the text", "reads the digest from the rendered value"),
            legacy(V_CHECK, "--check-topology-shape", "scrapes `shape_hash` from text a second time", "same"),
            legacy(OPTIONS, "exec_digest", "the replay identity: HALE_TOOLCHAIN_SHA256 + version + options fingerprint + plan digest + sources; its logical source paths fall back to file names; build and run fingerprint `debug` differently, so a build's recording never replays", "one stated coverage, with tests that a covered change moves it"),
            legacy(CLI_BUILD_RS, "toolchain_digest", "the replay identity: every identity-covered crate (`hale_graph::identity::COVERED_CRATES`) plus hale-cli, walked through the one shared walk", "phase 2"),
            legacy(STALE, "compute_codegen_src_hash", "the stale-binary hash: codegen.rs, lotus_arena.c and every stdlib .hl seed, walked identically at build and run time through the shared walk", "one identity per snapshot; the stale check reads it"),
            legacy(IRIS_BUILD_RS, "covered_dirs", "the DNA toolchain cache key's compiler-source half: every identity-covered crate, from the one list", "the cache key is derived from the snapshot identity"),
            legacy(IRIS_LIB, "toolchain_hash", "the cache key itself (version, compiler sources, stdlib, embedded iris and DNA trees)", "the cache key is derived from the snapshot identity"),
            legacy(DNA_DIGEST, "EMBEDDED_DIRS", "DNA's embedded-source identity, its own directory list", "one inventory of what each identity covers"),
            legacy(EVIDENCE, "analysis_inputs_digest", "the evidence inputs digest (semantics version, stdlib source, compiler version, renames, the surface registry)", "same"),
            legacy(V_CHECK, "bundle.sources", "per-file FNV digests, rooted at hale.toml, set by check only; the LSP uses absolute paths; build leaves it empty", "one source map per snapshot"),
            legacy(LSP, "source_files", "the LSP's own source map", "same"),
            legacy(M_OBS, "fn digest", "the observed entity-id digest, keyed by (kind, name)", "keyed by snapshot identity"),
        ],
        consumers: &[consumer("replay (admission)"), consumer("topology / fleet (admission)"), consumer("dna (schema 1.19, semantics 2, shape_hash, artifact_digest)"), consumer("the runtime obs header"), consumer("the DNA host cache")],
        invariants: &[
            "external contracts are frozen through extraction: additive and unhashed sections are free; hash and replay identity change only through explicit versioned transitions with an exact diagnostic (#476's rule)",
            "a semantic producer moving between crates never makes a later edit invisible to cache or replay identity: the replay identity and the cache key walk every identity-covered crate; the stale-binary hash is a cheap warning over codegen.rs, the runtime and the stdlib seeds by design",
        ],
        missing: Missing::NotApplicable,
        tests: &["crates/hale-cli/tests/obs_model_hash.rs", "crates/hale-cli/tests/model_diff.rs", "crates/hale-cli/tests/replay_cli.rs", "crates/hale-cli/tests/stale_dna_warning.rs", "crates/hale-cli/tests/source_map.rs"],
        spec: &["spec/model.md § Identity and versioning"],
        owned: &[],
        seams: &[],
    },
];

/// The spec's numbered rules and their evaluators. A registered rule
/// whose evaluator is `None` fails the build (registry_guard.rs),
/// unless it is `Reserved`.
pub const RULES: &[Rule] = &[
    Rule {
        id: "semantics/placement/1",
        gist: "`placement { }` is main-locus-only",
        family: "placement",
        evaluator: Some(site(PARSER, "`placement` block is only valid inside")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/2",
        gist: "keys name main-locus params fields",
        family: "placement",
        evaluator: Some(site(CHECK, "check_placement_block")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/3",
        gist: "field values are locus types",
        family: "placement",
        evaluator: Some(site(CHECK, "check_placement_block")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/4",
        gist: "at most one entry per field",
        family: "placement",
        evaluator: Some(site(CHECK, "check_placement_block")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/5",
        gist: "pool names are identifiers; `main` always exists",
        family: "placement",
        evaluator: Some(site(PARSER, "parse_placement_block")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/6",
        gist: "pinned-class restrictions (no accept(), no closure whose epoch is birth or dissolve, the default) at the placement entry",
        family: "placement",
        evaluator: Some(site(CHECK, "is placed `pinned` but")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/7",
        gist: "dead bus receiver on a cooperative pool is an error",
        family: "blocking",
        evaluator: Some(site(CHECK, "check_cooperative_pool_blocking")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/8",
        gist: "a blocking syscall on a cooperative pool is a warning",
        family: "blocking",
        evaluator: Some(site(CHECK, "check_cooperative_pool_blocking")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/9",
        gist: "orphan bus topic (closed world)",
        family: "bus_graph",
        evaluator: Some(site(CHECK, "check_bus_graph")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/10",
        gist: "bus cycles: cross-locus warning, intra-locus unconditional error",
        family: "bus_graph",
        evaluator: Some(site(CHECK, "check_bus_cycles")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/11",
        gist: "bus backpressure heuristic",
        family: "bus_graph",
        evaluator: Some(site(CHECK, "check_bus_backpressure")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/12",
        gist: "one literal subject, one payload type",
        family: "bus_graph",
        evaluator: Some(site(CHECK, "check_bus_subject_types")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/13",
        gist: "degenerate `pinned(cores = ..)` is an error",
        family: "placement",
        evaluator: Some(site(CHECK, "check_placement_block")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/14",
        gist: "topology consistency and node/l3 resolution",
        family: "placement",
        evaluator: Some(site(CHECK, "check_topology_block")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/15",
        gist: "`replicas = K`: K >= 1, pinned only",
        family: "placement",
        evaluator: Some(site(CHECK, "check_placement_block")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/16",
        gist: "pool affinity agrees per pool",
        family: "placement",
        evaluator: Some(site(CHECK, "check_pool_affinity")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/17",
        gist: "a pinned locus is not instantiated in a loop",
        family: "placement",
        evaluator: Some(site(CHECK, "check_pinned_locus_in_loop")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/18",
        gist: "every placement entry is consumed exactly once",
        family: "placement",
        evaluator: Some(site(CHECK, "check_placement_entry_consumed")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/19",
        gist: "a bus payload is carriable",
        family: "bus_graph",
        evaluator: Some(site(CHECK, "check_bus_payload_carriable")),
        state: State::Migrating,
    },
];

/// Every Debug rendering with no prose around it (a `?}` placeholder in a
/// formatting macro whose template holds no space: a value, never a
/// message) in hale-syntax, hale-types, hale-model, hale-codegen, hale-cli
/// and hale-lsp, frozen with a verdict and an invocation count. The
/// fragment is the invocation collapsed to one line, so a multi-line
/// call is seen. A new one, or a changed count, fails registry_guard.rs.
pub const DEBUG_SCANS: &[DebugScan] = &[
    DebugScan { path: BUILD_ENV, fragment: "format!( \"target={:?};cpu={:?};dev={};debug={}\", o.target, o.target_cpu, o.dev_profile, o.", count: 1, verdict: ScanVerdict::Decides { family: "digests" } },
    DebugScan { path: BUILD_ENV, fragment: "format!(\";lto={l:?}\")", count: 1, verdict: ScanVerdict::Decides { family: "digests" } },
    DebugScan { path: "crates/hale-cli/src/verbs/misc.rs", fragment: "println!(\"{:#?}\", prog)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: CG, fragment: "format!(\"{:?}\", f.body)", count: 1, verdict: ScanVerdict::Decides { family: "alloc_summary" } },
    DebugScan { path: CG, fragment: "format!(\"{:?}\", it)", count: 1, verdict: ScanVerdict::Decides { family: "bus_inert" } },
    DebugScan { path: CG, fragment: "format!(\"{:?}\", other)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: CG, fragment: "format!(\"{:?}\", program.items)", count: 1, verdict: ScanVerdict::Decides { family: "bus_inert" } },
    DebugScan { path: LSP, fragment: "format!(\"pinned({:?})\", affinity)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", ExitCode::SUCCESS)", count: 2, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", affinity)", count: 1, verdict: ScanVerdict::Decides { family: "placement" } },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", c.kind)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", p)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", r)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", s.placement)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", site.escape)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", site.kind)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{:?}\", site.reason)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: LSP, fragment: "format!(\"{code:?}\")", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: "crates/hale-syntax/src/json_gen.rs", fragment: "format!(\"{:?}\", f)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: PARSER, fragment: "format!(\"{:?}\", err)", count: 21, verdict: ScanVerdict::Renders },
    DebugScan { path: CHECK, fragment: "format!(\"{:?}\", kind)", count: 1, verdict: ScanVerdict::Decides { family: "blocking" } },
    DebugScan { path: CHECK, fragment: "format!(\"{:?}\", p)", count: 1, verdict: ScanVerdict::Decides { family: "snapshot_identity" } },
    DebugScan { path: CHECK, fragment: "format!(\"{:?}({})\", class, type_expr_key(inner))", count: 1, verdict: ScanVerdict::Decides { family: "snapshot_identity" } },
    DebugScan { path: TLIB, fragment: "format!(\"{:?}\", d.kind)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: MODEL_BUILDER, fragment: "format!(\"{:?}:{}\", d.kind, d.display)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: MODEL_BUILDER, fragment: "format!( \"projection:{:?}({})\", class, type_descriptor(inner) )", count: 1, verdict: ScanVerdict::Decides { family: "snapshot_identity" } },
    DebugScan { path: TY_RESOLVED, fragment: "format!(\"{:?}\", d)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: PURITY, fragment: "format!(\"{:?}\", op)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: PURITY, fragment: "format!(\"{:?}\", subject)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: "crates/hale-types/src/secret_reveal.rs", fragment: "format!(\"{:?}\", fd)", count: 1, verdict: ScanVerdict::Decides { family: "effects" } },
    DebugScan { path: "crates/hale-types/src/secret_reveal.rs", fragment: "format!(\"{:?}\", lc.kind)", count: 1, verdict: ScanVerdict::Decides { family: "effects" } },
    DebugScan { path: "crates/hale-types/src/secret_reveal.rs", fragment: "format!(\"{:?}\", m)", count: 1, verdict: ScanVerdict::Decides { family: "effects" } },
    DebugScan { path: "crates/hale-types/src/secret_reveal.rs", fragment: "format!(\"{:?}\", other)", count: 3, verdict: ScanVerdict::Renders },
    DebugScan { path: "crates/hale-types/src/stdlib_names.rs", fragment: "format!(\"{:?}\", d)", count: 1, verdict: ScanVerdict::Decides { family: "stdlib_surface" } },
];

/// All families, in layer order.
pub fn families() -> &'static [Family] {
    FAMILIES
}

/// A family by name.
pub fn family(name: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.name == name)
}

/// The spec rules with evaluators.
pub fn rules() -> &'static [Rule] {
    RULES
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Reserved => "Reserved",
            State::Migrating => "Migrating",
            State::Canonical => "Canonical",
        }
    }
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Derivation => "derivation",
            Kind::Law => "law",
            Kind::Desugar => "desugar",
            Kind::Digest => "digest",
            Kind::Capability => "capability",
        }
    }
}

impl Layer {
    fn title(self) -> &'static str {
        match self {
            Layer::Parse => "Layer 1 — parse and desugar",
            Layer::Declarations => "Layer 2 — declaration graphs",
            Layer::Locus => "Layer 3 — the locus graph",
            Layer::Effects => "Layer 4 — effects",
            Layer::Placement => "Layer 5 — placement",
            Layer::Lifecycle => "Layer 6 — lifecycle order",
            Layer::Lowering => "Layer 8 — lowering",
            Layer::Law => "The law engine",
            Layer::Identity => "Identity",
        }
    }
    const ALL: &'static [Layer] = &[
        Layer::Parse,
        Layer::Declarations,
        Layer::Locus,
        Layer::Effects,
        Layer::Placement,
        Layer::Lifecycle,
        Layer::Lowering,
        Layer::Law,
        Layer::Identity,
    ];
}

impl Missing {
    fn label(self) -> &'static str {
        match self {
            Missing::Error => "a missing required row is a compiler error",
            Missing::Hole => "an unknown is a hole with a stated policy",
            Missing::NotApplicable => "n/a",
        }
    }
}

fn site_md(s: &Site) -> String {
    format!("`{}` · `{}`", s.path, s.symbol)
}

/// Render the registry as `spec/registry.md`.
pub fn render_markdown() -> String {
    let mut o = String::new();
    o.push_str("# The graph registry\n\n");
    o.push_str(
        "GENERATED from `crates/hale-graph/src/registry.rs` and held byte-equal by \
         `registry_matches_spec`. Do not edit: change the table and run \
         `HALE_REGEN_REGISTRY=1 cargo test -p hale-graph --test registry_matches_spec`. \
         The contract this index serves is `spec/model.md` § *The graph registry*.\n\n",
    );
    let n_can = FAMILIES
        .iter()
        .filter(|f| f.state == State::Canonical)
        .count();
    let n_mig = FAMILIES
        .iter()
        .filter(|f| f.state == State::Migrating)
        .count();
    let n_res = FAMILIES
        .iter()
        .filter(|f| f.state == State::Reserved)
        .count();
    let n_legacy: usize = FAMILIES.iter().map(|f| f.legacy.len()).sum();
    o.push_str(&format!(
        "{} families: {n_can} canonical, {n_mig} migrating (with {n_legacy} permitted legacy \
         producers), {n_res} reserved. {} spec rules with evaluators. {} frozen Debug-string sites, \
         of which {} decide a fact.\n\n",
        FAMILIES.len(),
        RULES.len(),
        DEBUG_SCANS.len(),
        DEBUG_SCANS.iter().filter(|d| matches!(d.verdict, ScanVerdict::Decides { .. })).count(),
    ));
    o.push_str("## Families\n\n");
    o.push_str("| family | layer | state | kind | producer | legacy | answers |\n|---|---|---|---|---|---|---|\n");
    for f in FAMILIES {
        o.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} |\n",
            f.name,
            f.layer.title().split(" — ").next().unwrap_or(""),
            f.state.label(),
            f.kind.label(),
            f.producer
                .as_ref()
                .map(|p| format!("`{}`", p.symbol))
                .unwrap_or_else(|| "—".into()),
            f.legacy.len(),
            f.answers.replace('|', "\\|"),
        ));
    }
    o.push('\n');
    for layer in Layer::ALL {
        let fams: Vec<&Family> = FAMILIES.iter().filter(|f| f.layer == *layer).collect();
        if fams.is_empty() {
            continue;
        }
        o.push_str(&format!("## {}\n\n", layer.title()));
        for f in fams {
            o.push_str(&format!(
                "### `{}` — {} · {}\n\n",
                f.name,
                f.state.label(),
                f.kind.label()
            ));
            o.push_str(&format!("**Answers.** {}\n\n", f.answers));
            o.push_str(&format!("**Inputs.** {}\n\n", f.inputs.join("; ")));
            match (&f.producer, f.state) {
                (Some(p), State::Canonical) => o.push_str(&format!("**Producer.** {}\n\n", site_md(p))),
                (Some(p), _) => o.push_str(&format!("**Producer (today's authority, migrating).** {}\n\n", site_md(p))),
                (None, State::Reserved) => o.push_str("**Producer.** none: reserved, computes nothing.\n\n"),
                (None, _) => o.push_str("**Producer.** none yet: the family has no authoritative producer today; the legacy list is the whole inventory.\n\n"),
            }
            if !f.legacy.is_empty() {
                o.push_str("**Legacy producers (permitted until removal).**\n\n");
                for l in f.legacy {
                    o.push_str(&format!(
                        "- {} — {}. *Removed when:* {}.\n",
                        site_md(&l.site),
                        l.note,
                        l.removal
                    ));
                }
                o.push('\n');
            }
            if !f.owned.is_empty() {
                let owned: Vec<String> = f.owned.iter().map(site_md).collect();
                o.push_str(&format!("**Also owned.** {}\n\n", owned.join("; ")));
            }
            o.push_str("**Consumers.** ");
            let cs: Vec<String> = f
                .consumers
                .iter()
                .map(|c| match &c.site {
                    Some(s) => format!("{} ({})", c.who, site_md(s)),
                    None => c.who.to_string(),
                })
                .collect();
            o.push_str(&cs.join("; "));
            o.push_str("\n\n");
            if !f.invariants.is_empty() {
                o.push_str("**Invariants.**\n\n");
                for i in f.invariants {
                    o.push_str(&format!("- {i}\n"));
                }
                o.push('\n');
            }
            o.push_str(&format!("**Missing data.** {}\n\n", f.missing.label()));
            if !f.tests.is_empty() {
                o.push_str(&format!("**Focused tests.** {}\n\n", f.tests.join("; ")));
            }
            if !f.spec.is_empty() {
                o.push_str(&format!("**Spec.** {}\n\n", f.spec.join("; ")));
            }
            if !f.seams.is_empty() {
                o.push_str("**Guarded seams.**\n\n");
                for s in f.seams {
                    let allowed: Vec<String> = s
                        .allowed
                        .iter()
                        .map(|(a, n)| format!("`{a}` ×{n}"))
                        .collect();
                    o.push_str(&format!(
                        "- `{}` may be referenced from: {}\n",
                        s.symbol,
                        allowed.join(", ")
                    ));
                }
                o.push('\n');
            }
        }
    }
    o.push_str("## Spec rules and their evaluators\n\n");
    o.push_str("A registered rule without an evaluator fails the compiler's own build.\n\n");
    o.push_str("| rule | gist | family | evaluator | state |\n|---|---|---|---|---|\n");
    for r in RULES {
        o.push_str(&format!(
            "| {} | {} | `{}` | {} | {} |\n",
            r.id,
            r.gist,
            r.family,
            r.evaluator
                .as_ref()
                .map(site_md)
                .unwrap_or_else(|| "—".into()),
            r.state.label()
        ));
    }
    o.push('\n');
    o.push_str("## Frozen Debug renderings\n\n");
    o.push_str(
        "Every Debug rendering with no prose around it (a `?}` placeholder in a formatting \
         macro whose template holds no space) in `hale-syntax`, `hale-types`, `hale-model`, \
         `hale-codegen`, `hale-cli` and `hale-lsp`, with the number of invocations that collapse \
         to the fragment. A message with prose around its `{:?}` is not listed: it is read by a \
         person. A site that *decides* derives a fact from a Debug string and is permitted only \
         until its family's table replaces it; a new site fails the guard.\n\n",
    );
    o.push_str("| path | invocation | count | verdict |\n|---|---|---|---|\n");
    for d in DEBUG_SCANS {
        let v = match d.verdict {
            ScanVerdict::Decides { family } => format!("decides (`{family}`)"),
            ScanVerdict::Renders => "renders".to_string(),
        };
        o.push_str(&format!(
            "| `{}` | `{}` | {} | {} |\n",
            d.path,
            d.fragment.replace('|', "\\|"),
            d.count,
            v
        ));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_lookup_and_render() {
        assert!(family("ownership").is_some());
        assert!(family("nope").is_none());
        let md = render_markdown();
        assert!(md.contains("### `ownership`"));
        assert!(md.contains("semantics/placement/6"));
    }
}
