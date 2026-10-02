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
const QUALIFIED_SUBJECTS: &str = "crates/hale-types/src/qualified_subjects.rs";
const DESUGAR_SEQ: &str = "crates/hale-types/src/desugar_sequence.rs";
const HANDLER_ROUTING: &str = "crates/hale-types/src/handler_routing.rs";
const EFFECTS: &str = "crates/hale-types/src/effects.rs";
const EFFECT_ROWS: &str = "crates/hale-types/src/effect_rows.rs";
const ENTRY: &str = "crates/hale-types/src/entry.rs";
const LIFECYCLE: &str = "crates/hale-types/src/lifecycle.rs";
const LIFECYCLE_TRACE: &str = "crates/hale-types/src/lifecycle/trace.rs";
const PLACEMENT: &str = "crates/hale-types/src/placement.rs";
const FRONTIER: &str = "crates/hale-types/src/frontier.rs";
const EVIDENCE: &str = "crates/hale-types/src/evidence.rs";
const ALLOC: &str = "crates/hale-types/src/alloc_summary.rs";
const ALLOC_ROUTING: &str = "crates/hale-types/src/alloc_routing.rs";
const PURITY: &str = "crates/hale-types/src/purity.rs";
const TOPOLOGY: &str = "crates/hale-types/src/topology.rs";
const JUDGMENT: &str = "crates/hale-types/src/judgment.rs";
const CLAIMS: &str = "crates/hale-types/src/claims.rs";
const SYNC: &str = "crates/hale-types/src/sync_inference.rs";
const FORM_ROWS: &str = "crates/hale-types/src/form_rows.rs";
const TOPIC_ID: &str = "crates/hale-types/src/topic_identity.rs";
const STDLIB_SURFACE: &str = "crates/hale-types/src/stdlib_surface.rs";
const STDLIB_BODIES: &str = "crates/hale-types/src/stdlib_bodies.rs";
const CG: &str = "crates/hale-codegen/src/codegen.rs";
const CG_INST: &str = "crates/hale-codegen/src/locus/instantiation.rs";
const CG_DECL: &str = "crates/hale-codegen/src/locus/decl.rs";
const CG_METHOD: &str = "crates/hale-codegen/src/locus/method.rs";
const CG_DISSOLVE: &str = "crates/hale-codegen/src/locus/dissolve.rs";
const CG_RESTART: &str = "crates/hale-codegen/src/locus/restart.rs";
const CG_CHANNELS: &str = "crates/hale-codegen/src/channels/mod.rs";
const CG_WIRE: &str = "crates/hale-codegen/src/bus/wire.rs";
const CG_BUS_RT: &str = "crates/hale-codegen/src/bus/runtime.rs";
const CG_TYPES: &str = "crates/hale-codegen/src/types/mod.rs";
const CG_DEPLOY: &str = "crates/hale-codegen/src/deployment.rs";
const TY_TARGET: &str = "crates/hale-types/src/target.rs";
const CAPABILITY: &str = "crates/hale-types/src/capability.rs";
const FRONTEND: &str = "crates/hale-frontend/src/frontend.rs";
const IMPORTS: &str = "crates/hale-frontend/src/imports.rs";
const SNAPSHOT: &str = "crates/hale-frontend/src/snapshot.rs";
const TY_SNAPSHOT: &str = "crates/hale-types/src/snapshot.rs";
const SITES: &str = "crates/hale-syntax/src/sites.rs";
const OPTIONS: &str = "crates/hale-cli/src/shared/options.rs";
const BUILD_ENV: &str = "crates/hale-cli/src/build_env.rs";
const STALE: &str = "crates/hale-cli/src/shared/stale.rs";
const V_CHECK: &str = "crates/hale-cli/src/verbs/check/run_impl.rs";
const V_MATRIX: &str = "crates/hale-cli/src/verbs/check/matrix.rs";
const V_BUILD: &str = "crates/hale-cli/src/verbs/build.rs";
const CLI_DNA: &str = "crates/hale-cli/src/dna.rs";
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
        state: State::Canonical,
        kind: Kind::Desugar,
        answers: "Which source units form the snapshot: the entry, every imported seed, their merge order and the spans' virtual bases.",
        inputs: &[".hl files", "import directives", "the workspace root (hale.toml)", "editor overlays (LSP)"],
        producer: Some(site(FRONTEND, "collect_checkable")),
        legacy: &[],
        consumers: &[consumer("check"), consumer("build"), consumer("run"), consumer("test"), consumer("replay"), consumer("bench"), consumer("lsp"), consumer("dna (via the CLI)")],
        invariants: &[
            "one loader, one merge order, for every entry point",
            "an unresolved import is a diagnostic, never a silently smaller program",
            "one seed for every entry point: the editor's load (`LoadMode::Editor`) is `hale check <dir>`'s — the open file's directory, every `import` followed through the buffers (`link_checkable`) — and differs only in tolerance: a member that does not parse or will not read is recorded (`Snapshot::unparsed`, `Snapshot::unreadable`) and blocks the scope instead of failing the load, a link the import graph refuses keeps the members as they parsed with the refusal recorded (`Snapshot::unlinked`) and blocks the scope, which every request but the outline reads as the refused load (`Snapshot::linked`), and the LSP publishes an unreadable member as `seed member <name>: <os error>` against the member and the open file, never a clean seed the CLI cannot load",
        ],
        missing: Missing::NotApplicable,
        tests: &["crates/hale-cli/tests/imports.rs (diamond_import, three_hop_import, import_library_key)", "crates/hale-cli/tests/source_map.rs", "crates/hale-cli/tests/lsp.rs (lsp_and_check_agree_over_a_seed_that_imports, lsp_reports_an_unreadable_seed_member_as_check_does, lsp_outline_survives_a_link_failure)"],
        spec: &["spec/projects.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "qualified_names",
        layer: Layer::Parse,
        state: State::Canonical,
        kind: Kind::Desugar,
        answers: "What a qualified or aliased name denotes: the library identity, the mangled declaration, the construction target, the bus subject a path names.",
        inputs: &["import aliases", "the seed cache", "hale_stdlib::PATH_RENAMES", "declaration names"],
        producer: Some(site(IMPORTS, "resolve_imports")),
        legacy: &[],
        consumers: &[consumer("check"), consumer("build"), consumer("lsp (hover, definition, references)")],
        invariants: &[
            "a name resolves once per snapshot; the checker and lowering see the same target",
            "a construction path spelled with a type alias is resolved once, bundle-wide, in the desugar sequence before the check (`resolve_construction_aliases`), so the checker and lowering read the same target name; the checker follows no alias of its own, so a fragment checked without the sequence is not resolved a second way",
            "a qualified bus subject (`subscribe` / `publish alias::Topic`, `alias::Topic <- v`, a `bindings` entry) is resolved once, in the desugar sequence before the check (`resolve_qualified_bus_subjects`), to the single-segment topic the imported declaration ends up at: the checker (`resolve_bus_subject` keeps the leaf and an `Unknown` payload only for a path no rename names), the model and lowering read the rewritten program, and the resolved-program step does not resolve it again",
            "a qualified path in the checker is resolved through the scope's one import table (`KnownNames::import_target`, built from the bundle's renames): a qualified type, an imported fn (`imported_fn`) and an imported enum's constructor arm (`match_is_exhaustive`) name the same declaration by the same row, compared by identity and never by the text of the merged name, and the checker holds no second scan of the rename vector for any of them",
            "a library's name (`AliasScopes::name_library`) is a function of its own path alone — relative to the entry's workspace root, else to the entry seed's directory (`library_basis`), encoded injectively (`library_id`) — so no two libraries of a load share a name, and neither import order, the other imports of the build nor the tree's absolute location changes the symbols a library is mangled under; a declaration's full name (`mangle::mangled`) encodes the library, the file stem and the declaration as one injective tuple, so no two declarations of a load share one either",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/import_library_key.rs", "crates/hale-cli/tests/import_library_names.rs", "crates/hale-cli/tests/cross_seed_arity.rs", "crates/hale-codegen/tests/cross_seed_imports.rs", "crates/hale-types/tests/type_alias.rs", "crates/hale-types/tests/qualified_subjects.rs", "crates/hale-cli/tests/import_qualified_topic.rs"],
        spec: &["spec/semantics.md § Cross-seed namespace resolution", "spec/projects.md"],
        owned: &[site(TY_MANGLE, "resolve_construction_aliases"), site(IMPORTS, "name_library"), site(QUALIFIED_SUBJECTS, "resolve_qualified_bus_subjects")],
        seams: &[
            Seam { symbol: "resolve_construction_aliases(", allowed: &[(TY_MANGLE, 1), (DESUGAR_SEQ, 1)] },
            Seam { symbol: "resolve_qualified_bus_subjects(", allowed: &[(QUALIFIED_SUBJECTS, 1), (DESUGAR_SEQ, 1)] },
            Seam { symbol: "name_library(", allowed: &[(IMPORTS, 2)] },
        ],
    },
    Family {
        name: "desugar_sequence",
        layer: Layer::Parse,
        state: State::Migrating,
        kind: Kind::Desugar,
        answers: "Which rewrites the program receives before checking, in which order: the declaration-shaping passes only (JSON parsers, the api surface, unit returns, construction aliases, qualified bus subjects, the omitted `run`, repr accessors). Sync inference is not a rewrite: its pick is a form row (`sync_inference`). The topic-reference and intra-locus rewrites are not desugars: they erase a written declaration reference the checker's laws and the model read, and run in lowering's resolved program, after the check.",
        inputs: &["the merged program", "--api / --env (roles)", "the cross-seed rename table"],
        producer: Some(site(DESUGAR_SEQ, "desugar_before_check")),
        legacy: &[
            legacy(CG, "build_executable_with_options", "codegen's adapter for a bare program is the test harness's snapshot, not a second pipeline: it builds `Snapshot::from_program` (shaped by the one load, with no source map and no check before lowering, `Config::harness`) and demands the lowering view; its seam allows only the definition, so no non-test caller bypasses the verbs' snapshot (tests are not scanned by the seam guard)", "the harness builds from a loaded seed, as the verbs do"),
        ],
        consumers: &[consumer("check"), consumer("build"), consumer("run"), consumer("test"), consumer("replay"), consumer("bench"), consumer("lsp"), consumer_at("codegen", CG, "build_resolved")],
        invariants: &[
            "one order, run once per snapshot, before the first law is judged",
            "the sequence is called from the snapshot's load (`Snapshot::load`, `Snapshot::from_program`) and from `check_program` (the test entry), and from nowhere else: every entry point runs it before it mints its snapshot; the bundled stdlib goes through the same passes (`bundled_stdlib`)",
            "codegen never re-desugars",
            "the topic-reference and intra-locus rewrites are lowering's, on lowering's copy of the program, never the checked one, and each is recorded as a relation: `TopicRewrite` rows (`topic_rewrites`, and `written_topics` on the bus graph's subjects) and `IntraLocusRewrite` rows (`intra_locus`, and `direct_sends`); the intra-locus rewrite is its own stage, `rewrite_intra_locus` (over the sequence's program, its qualified bus subjects already resolved; the snapshot's `intra_locus` count), which the check demands for rule 10 and lowering continues from, so the judgment and the lowering read one rewrite; `resolve_rewritten` runs the topic rewrite, appends the stdlib, mints and derives the tables",
            "a desugar that copies a subtree clears the copy's identities (`hale_syntax::sites::clear_ids_in_*`): a verb mints after the sequence and the resolved program mints again after its rewrites, and two sites with one id is a panic; every corpus program is minted first and resolved second by a test",
            "the omitted `run` is synthesized in the sequence, before the check, and marked `LifecycleDecl::synthesized`: a rule about the run's body reads the empty body and answers both spellings alike (spec semantics.md § run()); what lists the hooks the author wrote (the application model, whose function and phase tables are the artifact's identity, the allocation summary, the effect contracts, the mint's `OmittedRun` origin) reads the marker, never the absence of author text, and a synthesized `run` moves no artifact and no diagnostic",
        ],
        missing: Missing::NotApplicable,
        tests: &["crates/hale-cli/tests/api_description.rs", "crates/hale-codegen/tests/framework_elision.rs", "crates/hale-types/tests/topology_projection.rs (the_omitted_run_moves_no_artifact_and_no_diagnostic)", "crates/hale-types/tests/bus_graph.rs (a_rewritten_send_stays_on_the_resolved_graph, a_rewritten_topic_reference_stays_on_the_resolved_graph)"],
        spec: &["spec/semantics.md"],
        owned: &[site(TY_RESOLVED, "resolve_program"), site(TY_RESOLVED, "rewrite_intra_locus"), site(TY_RESOLVED, "resolve_rewritten"), site(DESUGAR, "desugar_intra_locus_topics"), site(DESUGAR, "desugar_topics"), site(DESUGAR_SEQ, "bundled_stdlib"), site(DESUGAR, "desugar_omitted_run"), site(DESUGAR, "desugar_repr_accessors")],
        seams: &[
            Seam { symbol: "desugar_topics(", allowed: &[(DESUGAR, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "desugar_before_check(", allowed: &[(DESUGAR_SEQ, 1), (TLIB, 1), (SNAPSHOT, 1)] },
            Seam { symbol: "desugar_omitted_run(", allowed: &[(DESUGAR, 1), (DESUGAR_SEQ, 1)] },
            Seam { symbol: "desugar_repr_accessors(", allowed: &[(DESUGAR, 1), (DESUGAR_SEQ, 1)] },
            Seam { symbol: "resolve_program(", allowed: &[(TY_RESOLVED, 1)] },
            Seam { symbol: "rewrite_intra_locus(", allowed: &[(TY_RESOLVED, 2), (SNAPSHOT, 1), (TLIB, 1)] },
            Seam { symbol: "resolve_rewritten(", allowed: &[(TY_RESOLVED, 2), (SNAPSHOT, 1)] },
            Seam { symbol: "desugar_intra_locus_topics(", allowed: &[(DESUGAR, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "build_executable_with_options(", allowed: &[(CG, 1)] },
        ],
    },
    Family {
        name: "sync_inference",
        layer: Layer::Parse,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "Which sync discipline each `@form` declaration gets: one row per declaration with the author's configuration (omitted, a written discipline, `none` included, or an argument naming none) and the effective discipline, inference's pick for a `hashmap` form left unconfigured, from the domains each of its instances is called from; two queries, explicitly configured and safe for cross-domain access.",
        inputs: &["placement (the table, per instance)", "top_scope", "form declarations", "method call sites"],
        producer: Some(site(FORM_ROWS, "form_rows")),
        legacy: &[
            legacy(ALLOC, "summarize_identified", "the allocation summary reads a written `sync =` argument for `sync_forms`, the only answer for the stdlib's analysis copy, which no snapshot's rows hold; the effects engine adds the rows' (`add_sync_forms`)", "the stdlib's forms are rows of the snapshot (the stdlib merged once)"),
        ],
        consumers: &[
            consumer_at("the snapshot (one row set per snapshot, after the mint, over its scope and placement table)", SNAPSHOT, "demand_forms"),
            consumer_at("check (F.31 cross-pool verdicts: the one predicate, safe for cross-domain access)", CHECK, "check_placement_single_thread"),
            consumer_at("check (instance aliasing: a field behind a sync discipline)", CHECK, "locus_has_unsynchronized_state"),
            consumer_at("the effects certificate engine (a call into a sync-bearing form or its holder can take its lock)", ALLOC, "add_sync_forms"),
            consumer_at("sync inference (its candidates: the forms not explicitly configured)", SYNC, "infer_sync_for_bundle"),
            consumer_at("model (`sync_form`, read by the `depends` law)", MODEL_BUILDER, "derive_application_model_over"),
            consumer_at("the lowering view (the snapshot's rows, and the merged stdlib's as written)", TY_RESOLVED, "resolve_program"),
            consumer_at("codegen (the slot layout)", CG_DECL, "sync_mode"),
            consumer("lsp"),
        ],
        invariants: &[
            "every entry point sees the same discipline for the same program",
            "one row per `@form` declaration, found by the identity the load minted (a monomorph by its template's) or by name: the configuration and the effective discipline are separate columns",
            "an explicit `sync = none` is configuration: inference does not run over it, and the row does not call it safe for cross-domain access",
            "one predicate per question: inference's candidates are the forms not explicitly configured, and the F.31 cross-pool exemption is safe for cross-domain access; sync inference runs once per snapshot, and the cross-pool diagnostic's hint reads its reasoning from the rows",
            "nothing writes the discipline into the program: the check, the effects engine, the model and lowering read the row, and a declaration with no row (the stdlib's, merged for lowering) reads its written argument",
            "the readers that ask whether a form synchronizes as one question (the model's `sync_form`, the effects engine, instance aliasing) ask safe for cross-domain access alone (`FormRows::synchronizes`): an explicit `sync = none` takes no lock and is not one",
            "inference is per instance (the placement correspondence's K-5): an access `self.f.m()` is made from the domain of each instance of its enclosing locus and reaches that instance's own `f` (a held one by its source row, so its holders share it); the rule is applied per accessed instance and a type gets the most synchronized discipline any instance needs, never a union of domains per type; what the table does not know (a dynamic literal of unknown domains, a held instance whose source is unlinked, the instance below one) is a domain apart from every other, never main",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/form_rows.rs", "crates/hale-frontend/src/snapshot.rs (the_form_rows_are_one_family_by_identity)", "crates/hale-types/tests/placement.rs"],
        spec: &["spec/forms.md § Cross-pool sync disciplines", "spec/semantics.md § A form's sync discipline"],
        owned: &[site(SYNC, "infer_sync_for_bundle")],
        seams: &[
            // The snapshot's, and the entries of a bundle no snapshot
            // holds: `check_bundle`, `check_bundle_opts_scoped` and
            // `derive_application_model`, `effect_certificates`.
            Seam { symbol: "form_rows(", allowed: &[(FORM_ROWS, 1), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2), (EFFECTS, 1)] },
        ],
    },
    Family {
        name: "effect_class_table",
        layer: Layer::Parse,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "The user effect classes of a load: one table every seed is parsed through, so a class (its name, its identity in the program's one class namespace) has one `User(i)` index in every seed; which were declared, which are composed, and the one expansion of a composed class.",
        inputs: &["`effect` declarations and class references per seed", "the load's parse order (a seed after the seeds it imports)"],
        producer: Some(site("crates/hale-syntax/src/ast.rs", "EffectClasses")),
        legacy: &[],
        owned: &[site("crates/hale-syntax/src/lib.rs", "parse_source_at_in"), site("crates/hale-types/src/effect_classes.rs", "EffectClassTable")],
        consumers: &[
            consumer_at("the load (own files, the editor's members, every imported seed after its imports)", FRONTEND, "parse_source_at_in"),
            consumer_at("effects (contracts, phase contracts, the declared manifest)", EFFECTS, "EffectClassTable::of("),
            consumer_at("the effect rows (one table per snapshot, carried on the rows: the model's effect-class rows and atoms, and the inferred manifest's class names, read it there)", EFFECT_ROWS, "EffectClassTable::of("),
            consumer_at("effects (causes)", FRONTIER, "EffectClassTable::of("),
            consumer_at("alloc_summary (what `@effects(is: …)` carries)", ALLOC, "EffectClassTable::of("),
            consumer_at("quantitative (user-class budgets)", "crates/hale-types/src/quantitative.rs", "EffectClassTable::of("),
            consumer_at("claims (lowering: class references, undeclared classes)", "crates/hale-types/src/claim_lowering.rs", "EffectClassTable::of("),
            consumer_at("topology (derived effect sets)", TOPOLOGY, "EffectClassTable::of("),
        ],
        invariants: &[
            "one class, one index, per load: every seed is parsed through the load's one table (`parse_source_at_in`, `parser::parse_in`), so merging seeds renumbers nothing; an imported seed is numbered after the seeds it imports (it is parsed again, through the table, once they are), the order the classes have always been numbered in",
            "one expansion: a composed class's mask, its atoms and whether its definition is cyclic are `EffectClassTable`'s; no analysis walks a definition itself or reads a program's table directly",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/cross_seed_effects.rs", "crates/hale-cli/tests/xseed_user_effects.rs", "crates/hale-types/src/effect_classes.rs (one_table_one_expansion)", "crates/hale-frontend/src/snapshot.rs (a_seed_is_numbered_after_the_seeds_it_imports)"],
        spec: &["spec/verification.md § Default-on & opt-in analyses"],
        seams: &[
            Seam { symbol: "EffectClassTable::of(", allowed: &[("crates/hale-types/src/effect_classes.rs", 1), (EFFECTS, 3), (EFFECT_ROWS, 1), (FRONTIER, 1), (ALLOC, 1), ("crates/hale-types/src/quantitative.rs", 1),("crates/hale-types/src/claim_lowering.rs", 1), (TOPOLOGY, 1)] },
            // a program's class definitions are read by the parser that
            // writes them, the load that carries them, and the table
            // (`resolved.rs` only builds an empty stdlib program)
            Seam { symbol: "effect_defs", allowed: &[("crates/hale-syntax/src/ast.rs", 1), (PARSER, 9), (FRONTEND, 3), ("crates/hale-types/src/effect_classes.rs", 2), (TY_RESOLVED, 1)] },
        ],
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
            legacy(TLIB, "check_bundle_opts_scoped", "`check_program` (the test entry): built here, once, for its checker and the model its laws are judged over. Beside it the model of a bundle no snapshot holds (`derive_application_model`: `claim_law_diags`, the hale-types tests, and the artifact and model-hash entries over a bare bundle; since 2.3 no verb reaches it), the certificate report of such a bundle (`effect_certificates`, for its form rows) and the lowering view (once, for the ownership graph and the bus graph) rebuild it; every verb and the LSP (its diagnostics and every request) build one per snapshot (`demand_scope`) and pass it to the checker, the model and the model's graphs", "every consumer demands the scope from a snapshot (2.3)"),
        ],
        consumers: &[consumer_at("check", CHECK, "check_bundle_scoped"), consumer_at("check (type expressions: the scope's name table)", CHECK, "&top.names"),consumer_at("demand (every verb, the LSP's diagnostics and its requests: one scope per snapshot)", SNAPSHOT, "build_top_scope"), consumer_at("model (the snapshot's scope, handed in)", MODEL_BUILDER, "ModelInputs"), consumer_at("resolved program (lowering)", TY_RESOLVED, "build_top_scope"), consumer_at("lsp (definition, placement, the allocation survey: the snapshot's scope)", LSP, "demand_scope"), consumer_at("lsp (completion, hover, references, enforcement: the editor's scope, over the members that parsed while one does not)", LSP, "demand_editor_scope")],
        invariants: &[
            "one namespace decision: module-nested declarations and imported seeds resolve the same way everywhere",
            "the editor's scope over a seed with a hole (`demand_editor_scope`) is the same producer over the members that parsed, counted as this family; the whole scope, the check and everything after it stay blocked, so no check runs over a partial program",
            "one name table: the checker resolves every type expression against the scope's own (`TopScope::names`, the declared loci, types and perspectives with each alias's expanded target and the bundle's import renames), built once with the symbols, and keeps none of its own",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/checks_inside_modules.rs", "crates/hale-cli/tests/check_unknown_identifier.rs", "crates/hale-types/tests/type_alias.rs"],
        spec: &["spec/semantics.md"],
        owned: &[],
        seams: &[Seam { symbol: "build_top_scope(", allowed: &[(RESOLVE, 1), (TLIB, 3), (SYNC, 1), (EFFECTS, 1), (TY_RESOLVED, 1), (SNAPSHOT, 1)] }],
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
        inputs: &["@form arguments", "capacity declarations", "indexed_by", "sync discipline (the `sync_inference` form rows)"],
        producer: Some(site(CHECK, "check_form_shape")),
        legacy: &[
            legacy(CG_DECL, "ring_buffer_cap", "codegen reads the form's `cap =` argument again for the slot layout (a ring buffer's or LRU cache's capacity, a lockfree map's fixed capacity); the `sync` discipline it reads from the form row", "codegen reads the form rows"),
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
        inputs: &["locus declarations (is_main, imported, the __lib_ prefix, module nesting)", "the minted sites"],
        producer: Some(site(ENTRY, "entry_row")),
        legacy: &[
            legacy(ENTRY, "lowering_root", "the row's provisional column: the `main locus` lowering deploys today, `collect_main_placement`'s choice copied exactly (the first `is_main && !__lib_` over the flat declarations, module-nested ones included), so the placement-safety rules (the placement table the F.31 rule reads, the pinned-in-a-loop rule) guard the threads lowering spawns while a seed whose only `main` is module-nested has no entry", "L4: lowering reads `entry`, and the column goes"),
            legacy(CG, "collect_main_placement", "`is_main && !__lib_` over flat declarations", "one definition of the entry, as a row"),
            legacy(CG_INST, "let is_main_locus", "`is_main_locus` compares type names at instantiation (and twice more in dissolve.rs)", "same"),
            legacy(CG_DISSOLVE, "let is_main_locus", "the same comparison in the cascade", "same"),
            legacy(CG, "is_main_entry", "the deferred entry teardown compares the entry's locus name with `main_locus_name` to decide whether it joins the pools (#1208)", "same"),
            legacy(CG, "emit_bindings_prelude", "the connect-transport loss handler is looked up in the locus named by `main_locus_name`", "same"),
            legacy(CG, "in_main", "whether lowering is inside `fn main` is a flag set while main's body is emitted (and cleared around a generic fn lowered from inside it); the frame flush's main-exit wait-abort and `return`-from-main's teardown key on it", "same"),
            legacy(CG, "collect_shm_ring_subjects", "the shm-ring subjects are read from the first `is_main && !__lib_` over the flat declarations, `collect_main_placement`'s choice made again", "same"),
            legacy(CG, "synthesize_codec_thunks_for_main_bindings", "the binding codec thunks are synthesized for the first `is_main && !__lib_` over the flat declarations, the same choice made again", "same"),
            legacy(CG, "let has_socket_binding", "whether the program has a socket binding (so the cooperative queue is locked) asks the TOP-LEVEL `is_main && !__lib_` declarations only, where `collect_main_placement` walks the flat declarations: a module-nested root's bindings are not seen", "same"),
            // The checker's own readers of `main` that E0 did not switch.
            legacy(CHECK, "check_placement_entry_consumed", "rule 18's scope is the LAST `is_main && !__lib_` over every declaration, module-nested ones included (lowering takes the first; the two differ only under rule 1's error)", "reads `lowering_root`, since the rule guards what lowering emits; reads the entry with L4"),
            legacy(CHECK, "check_cooperative_pool_blocking", "the blocking check reads the placement and params of EVERY `is_main` declaration, module-nested and imported ones included, with no mark or name filter", "reads `lowering_root`, since the starvation it reports is on the threads lowering spawns; reads the entry with L4"),
            legacy(CHECK, "check_instance_aliasing", "instance aliasing relates the placed fields of the LAST `is_main` declaration's static params tower, with no filter (an imported `main` included)", "same"),
            legacy(CHECK, "check_pool_affinity", "validates EVERY `is_main` declaration's own placement block (an affinity with no named pool, two affinities for one pool), deployed or not: validation of each declaration, which derives no entry fact", "none for the entry: it leaves this inventory when it walks the row's witness (`mains`) instead of the declarations (L4)"),
            legacy(CHECK, "let api_bound", "`check_bus_graph`'s orphan lint is lifted when ANY `is_main` declaration carries an `api:` binding: a module-nested one, or an imported one whose api entry is inert (GH #1104 piece 5)", "reads the entry, whose binding is the one that binds (L4)"),
            // The `--api` and `--env` injections, and the api surface.
            legacy(DESUGAR_SEQ, "let at = programs", "the `--api` injection target in `desugar_before_check`: the first program holding a top-level `is_main`, an imported one included", "reads the entry: the binding goes on the entry, and a seed with none is refused (L4)"),
            legacy(API_GEN, "inject_api_entry", "the injected `api:` entry goes on that program's first `is_main` at any depth, module-nested and imported ones included", "same"),
            legacy(API_GEN, "api_surface", "the api surface is built around the first `is_main && !imported` carrying an `api:` entry, at any depth (a module-nested `main` included)", "same"),
            legacy(API_GEN, "declared_roles", "`owner` joins the declared roles when any `is_main && !imported` declaration at any depth carries an `api:` entry", "same"),
            legacy(SNAPSHOT, "inject_adopt", "an environment's constitution is adopted into EVERY top-level `is_main` of the program (an imported one included, a module-nested one not), and a program with none refuses it", "reads the entry: an environment binds law to the entry (L4)"),
            legacy(CLAIMS, "has_main = true", "world-tier claims are gathered from every `is_main` at any depth, and `has_main` refuses a top-level `claims` block in a seed that closes", "same"),
            // The model, the graphs' closed worlds, effects, the editor, the DNA.
            legacy(MODEL_BUILDER, "let main_decl", "the model's arrangement root is the first `is_main` among the model's loci, with no filter", "reads the entry (L4)"),
            legacy(MODEL_BUILDER, "let entrypoint = ast", "the model's `entrypoint` name is the first `is_main` among its loci, else `main`", "same"),
            legacy(BUS_GRAPH, "let has_entry_point", "the bus graph's closed world is any top-level `is_main` or top-level `fn main`, an imported `main` included; deliberately broader than rule 9's", "reads the entry, beside the `fn main` entry point (L4)"),
            legacy(OWNERSHIP_GRAPH, "let has_entry_point", "the ownership DAG's closed world, the bus graph's test made again", "same"),
            legacy(TY_MANGLE, "seed_declares_main", "whether an imported seed declares a `main locus` (GH #774): any top-level `is_main` over the seed's files", "reads the imported seed's own entry row (L4)"),
            legacy(EFFECTS, "placement_implied_diags", "the async_io pool's locus types come from every top-level `is_main` declaration's placement, an imported one included and a module-nested one not", "reads `lowering_root`, since the pool is one lowering spawns; reads the entry with L4"),
            legacy(LSP, "placement_of", "the editor's placement view shows every top-level `is_main` declaration's placement", "same"),
            legacy(CLI_DNA, "main_of", "`hale dna init` takes the LAST top-level `is_main` over the seed's files, parse-only (no import is resolved, so no mark exists)", "reads the entry row over the seed's own files, as `seed_entry_kind` does (L4)"),
            // Readers of a declaration's own `main` keyword: they derive
            // no entry fact, and are listed so the inventory is whole.
            legacy(CHECK, "if parent.is_main", "`check_nested_long_running_child` exempts a `main locus`, as a parent and as a child, from the long-running-child rule: a property of each declaration, which derives no entry fact", "none for the entry: it leaves this inventory when it reads the row's witness (`mains`) instead of the keyword (L4)"),
            legacy(OWNERSHIP_GRAPH, "entry.singleton |= l.is_main", "every `main locus` declaration is a singleton in the ownership graph: a property of each declaration, which derives no entry fact", "same"),
            legacy(ALLOC, "let mut eager_only_loci", "every top-level `main locus` declaration is excluded from eager reclamation, conservatively: a property of each declaration, which derives no entry fact", "same"),
        ],
        consumers: &[
            consumer_at("check (rule 1's count reads the witness: the seed's own mains, module-nested ones included)", CHECK, "check_main_and_bindings"),
            consumer_at("check (the pinned-in-a-loop rule reads the lowering root)", CHECK, "check_pinned_locus_in_loop"),
            consumer_at("placement (the table is seeded from the lowering root; the F.31 rule and the form rows' sync inference read it per instance)", PLACEMENT, "derive_placement"),
            consumer_at("check (rule 9's closed world is a program with an entry)", CHECK, "check_bus_graph"),
            consumer_at("check --matrix (a seed is an entrypoint when its row has an entry; the row is built over the seed's own files, since no import holds the entry, so a seed whose import does not resolve is still counted and its pair reports the import)", V_MATRIX, "seed_entry_kind"),
            consumer_at("--env on check and the build paths (the load refuses an environment for a seed with no entry, after the mint, before the sequence's own refusal)", SNAPSHOT, "demand_entry"),
            consumer("build"),
            consumer("dna"),
            consumer("codegen"),
        ],
        invariants: &[
            "the checker builds no row: the snapshot demands it before the check and hands it in (`CheckInputs::entry`); a bundle no snapshot holds (the test entries) builds it once, the form rows read the snapshot's (`demand_forms`), and the check matrix builds one over each seed's own files (no import holds the entry, and a seed whose import does not resolve is still an entrypoint)",
            "one row per snapshot (`Snapshot::demand_entry`, the `entrypoint` count): the entry by its minted site, and every `main locus` the bundle declares as the witness, each with whether it is imported and whether it is module-nested; it reads declarations only, so no diagnostic blocks it, and a seed with a hole (no identities) blocks it with its scope",
            "an imported `main` is not the entry (decision 1, E0): a library's `main locus` is a declaration the importing seed does not run, and the entry is the importing seed's own; a seed whose only `main` is imported has no entry (`NoEntry::OnlyImported`). Imported is one definition, the rename pass's mark (`imported`, GH #1104 piece 5); the `__lib_` name the same pass gives is spelling, not a second test of the entry (the provisional lowering root copies lowering's name test, until L4)",
            "a module-nested `main` is not the entry (decision 2, E0): the entry is a top-level `main locus` of the seed's own files, so rule 9's closed world is the top-level one; a seed whose only `main` is module-nested has no entry (`NoEntry::OnlyModuleNested`), and a seed with both keeps the top-level one. Rule 1 still counts a module-nested `main` (GH #825): the count reads the witness, not the entry",
            "the decisions bind what reads the entry (rule 9's closed world, `--env`, `--matrix`), not the placement-safety rules: until lowering reads the entry (L4) it deploys a module-nested `main` as its root and spawns its pinned threads, so the placement table (which the F.31 rule reads) and the pinned-in-a-loop rule read the row's provisional `lowering_root`, and a seed whose only `main` is module-nested has no entry and its cross-pool call is still refused (the outside review of #1293, finding 1)",
            "with more than one candidate (rule 1's error) the entry is the last, as the checker's pool map (since replaced by the placement table) took it before the row; the lowering root is the first non-`__lib_` `main`, nested or not, as lowering takes it",
            "the legacy sites use three definitions of the main locus today, and lowering's `in_main` a fourth, of fn main; the row has one",
            "every non-test reader of `LocusDecl::is_main` is the producer or a legacy row here; the parser's main-only member rules are syntax and are not rows, and `sync_inference`'s test helper is test code",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-frontend/src/snapshot.rs (the_entry_row_is_the_seeds_own_top_level_main_locus)", "crates/hale-cli/tests/check_entry_decisions.rs", "crates/hale-cli/tests/nested_main_transition.rs (the nested-main transition end to end: refused, and deployed, as a top-level main)", "crates/hale-cli/tests/entry_point_placement.rs", "crates/hale-types/tests/bus_graph.rs"],
        spec: &["spec/semantics.md § Bundle-wide rules"],
        owned: &[],
        seams: &[Seam { symbol: "entry_row(", allowed: &[(ENTRY, 1), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2), (EFFECTS, 1), (V_MATRIX, 1), (SYNC, 1)] }],
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
            legacy(OWNERSHIP_GRAPH, "build_ownership_graph", "which accepting ancestor owns a method-body birth, keyed (enclosing locus, child type) by name, the child type resolved by `child_locus_name`; built once in the resolved program for lowering and once per snapshot over the checked programs for the model (`demand_ownership_graph`)", "phase 3, one ownership table with both relations, when the lowering view's graphs fold into the snapshot: the check still runs over the checked programs and the resolved program is built only on build paths (`resolve_program`), so the model's graph and lowering's are two builds over two program forms (at the phase-2 close)"),
            legacy(MODEL_BUILDER, "Owns", "the model's params-field tree from main, a third ownership account", "projected from the one table"),
            legacy(TY_OWN, "extend_fresh_factories", "the carrier-arm fixpoint that widens the factory set", "phase 3, as a judgment migration: the carrier fold widens the set lowering reads and the checker reads the unextended set, so giving the checker the extended set changes which bindings it checks as factory-returned (lowering-only at the phase-2 close; deferred in its exit comment)"),
            legacy("crates/hale-types/src/borrow_lifetime.rs", "accepts", "the borrow-lifetime law rebuilds the accept sets from the AST for itself", "reads `accepts_ancestor`"),
            legacy(CHECK, "check_unowned_subscriber_locus", "the unowned-subscriber rule over its own name-keyed locus index; skipped by `--allow-unowned-subscriber` on some verbs and hard-coded off on others", "phase 3, as a judgment migration with spec text: measured on the 2.3 checker branch, reading ownership from the graph changes three shapes (an aliased accept type stops erroring, a false positive today; a module-path accept type and a generic accept type start), and two loci of one name flip with declaration order, since this index keeps the last declaration and the scope the first"),
            legacy(CG_INST, "parent_accepts_us", "a monomorphised parent reads its own accept param: graph rows are per template", "phase 3, when the graph keys rows by the template's identity and lowering asks by it: its rows are per declaration, keyed by locus name, so a locus codegen monomorphised (`Holder_Int`) finds none and lowering reads its own `accept_param` (at the phase-2 close); the monomorph-to-template join is the one round 3's finding 16 names for the handler rows"),
        ],
        consumers: &[consumer_at("codegen", CG_INST, "site_owner"), consumer_at("borrow_lifetime", "crates/hale-types/src/borrow_lifetime.rs", "borrow_lifetime_diags"), consumer_at("model (dynamic births: the snapshot's graph)", SNAPSHOT, "demand_ownership_graph"), consumer("alloc_summary (eager-only accept sets)")],
        invariants: &[
            "a locus instantiation with no row is a CodegenError (F.39)",
            "ids, not names or spans: declarations are cloned and the stdlib's coordinates overlap user files",
            "the ownership matrix stays green with an empty KNOWN_OPEN",
            "`fresh_factories` is read by lowering and the checker with the bundle's import renames; a factory's returned name is the declaration the snapshot resolves it to, so a fn whose returned name an inner `let` shadows is a factory of the outer binding (the #1140 shape; its escape walk still reads every binding spelling the name as the returned one, the conservative side). The carrier-arm extension (`extend_fresh_factories`) is folded in for lowering only",
            "which declaration a returned or escaping name denotes is read from the snapshot (`Snapshot::declaration_of` over `binding_of`, resolved once by the mint), never resolved again: `returned_bindings` (the binding facts and the pre-pass), `fresh_factories`, borrow_lifetime's `returned_decls` and alloc_summary's escape tags each key a binding by its declaration's SiteId; a `let` or a use the snapshot did not mint answers by name in `returned_bindings` (the conservative side) and resolves to nothing elsewhere, and every entry point mints",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/owner_table.rs", "crates/hale-codegen/tests/ownership_matrix.rs", "crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding)", "crates/hale-codegen/tests/ownership_bubble.rs"],
        spec: &["spec/decisions.md F.39", "spec/semantics.md § Dissolve timing rules"],
        owned: &[site(TY_OWN, "resolve_binding_facts"), site(OWNERSHIP_GRAPH, "bubble_plans"), site(OWNERSHIP_GRAPH, "compute_forwarding_sets"), site(OWNERSHIP_GRAPH, "classify_owner_kind"), site(OWNERSHIP_GRAPH, "classify_edge"), site(TY_OWN, "fresh_factories")],
        seams: &[
            Seam { symbol: "resolve_owners(", allowed: &[(TY_RESOLVED, 1), (TY_OWN, 1)] },
            Seam { symbol: "build_ownership_graph(", allowed: &[(OWNERSHIP_GRAPH, 1), (SNAPSHOT, 1), (TLIB, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "fresh_factories(", allowed: &[(TY_RESOLVED, 1), (TY_OWN, 1), (CHECK, 1)] },
            Seam { symbol: "resolve_binding_facts(", allowed: &[(TY_OWN, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "returned_bindings(", allowed: &[(TY_OWN, 3)] },
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
            legacy(TLIB, "bundle_bus_graph", "the check and the model of a bundle no snapshot holds (the test entries: `check_bundle`, `check_bundle_opts_scoped`, `claim_law_diags`, the hale-types tests, the artifact's bundle entry) build the bus graph here, once per entry, beside the ownership graph and the handler rows, and the check's intra-locus relation beside it (`bundle_intra_locus`, the stage over the bundle's programs merged); every verb reads its snapshot's", "those callers hold a snapshot"),
            legacy(TY_RESOLVED, "build_bus_graph", "built once in the resolved program, over the desugared program, for lowering; the snapshot builds a second over the checked programs for the check, the model and hale/busGraph (`demand_bus_graph`)", "phase 3, one graph per snapshot, when the lowering view's graphs fold into the snapshot: the check still runs over the checked programs, not the resolved one, so lowering's graph (with the intra-locus and topic rewrites recorded on it) and the snapshot's are built over two program forms (at the phase-2 close)"),
        ],
        consumers: &[
            consumer_at("check (rule 9: the wire rows, their bound, cross-seed and wildcard columns, over the entry row's closed world; the snapshot's graph, `CheckInputs::bus`)", CHECK, "check_bus_graph"),
            consumer_at("check (rule 10: the edges by declaration, through `cycle_from`, each edge's send joined to the intra-locus relation by its id, `CheckInputs::intra_locus`)", CHECK, "check_bus_cycles"),
            consumer_at("check (rule 7: the placed declaration's row, `external_handlers`)", CHECK, "check_cooperative_pool_blocking"),
            consumer("check (rules 11, 12, 19)"),
            consumer_at("model (subjects, endpoints and gates: the snapshot's graph)", SNAPSHOT, "demand_bus_graph"),
            consumer("topology"),
            consumer("dispatch"),
            consumer_at("lsp (hale/busGraph: the model's graph, so eligibility is the diagnostics pass's)", LSP, "demand_bus_graph"),
            consumer_at("codegen (a rewritten publish, found by its call's id in the relation: the probes and the reclaimed subregion)", CG, "intra_locus_rewrite"),
        ],
        invariants: &[
            "one graph, over one program shape, per snapshot; rule 10's cycle graph is a query over it (`cycle_from`)",
            "the checker's bus rules (7, 9, 10) compare subjects under the canonical key, the wire subject (`Subject`, `wires`): a topic published by name and subscribed by its literal subject is one subject; the gates and the model keep `BusSubject::canonical()`'s keys (`subjects`)",
            "an edge belongs to the locus declaration that wrote its handler (`BusEdge::decl`), never to a name: two loci of one name have their own edges",
            "a subject the graph cannot resolve is a hole (`holes`): it forms no edge and no rule calls it an orphan",
            "the intra-locus rewrite is a relation on the graph, never an erased publisher (boundary 7)",
            "lowering reads the relation (`LoweringView::intra_locus`) by the call's id, which the call that replaces a send keeps (`IntraLocusRewrite::send`); it never classifies a call as a rewritten publish from the call's shape",
            "rule 10 calls a cycle synchronous only where the relation holds every send of it (`BusEdge::send`, the snapshot's `intra_locus` stage, the one lowering continues from); a same-declaration cycle with a send the relation does not hold is carried by the queue, and the subjects' spelling never decides which",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/bus_graph.rs", "crates/hale-types/tests/bus_rules_over_graph.rs", "crates/hale-types/tests/bus_payload_handler.rs", "crates/hale-codegen/tests/bus_devirt_differential.rs"],
        spec: &["spec/semantics.md rules 7, 9-12, 19", "spec/verification.md § Bus-graph property checks"],
        owned: &[site(BUS_GRAPH, "dispatch_gates"), site(BUS_GRAPH, "cycle_from"), site(BUS_GRAPH, "external_handlers")],
        seams: &[
            Seam { symbol: "build_bus_graph(", allowed: &[(BUS_GRAPH, 1), (SNAPSHOT, 1), (TLIB, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "collect_bus_walk(", allowed: &[(BUS_GRAPH, 2)] },
            Seam { symbol: "dispatch_gates(", allowed: &[(BUS_GRAPH, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "cycle_from(", allowed: &[(BUS_GRAPH, 1), (CHECK, 2)] },
            Seam { symbol: "external_handlers(", allowed: &[(BUS_GRAPH, 1), (CHECK, 1)] },
        ],
    },
    Family {
        name: "topics",
        layer: Layer::Locus,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "What each topic is on the wire: its subject, payload contract, routing key, bounds and shed policy; and which topic a send's subject names.",
        inputs: &["topic declarations", "send subjects", "subscribe bounds", "bindings (shm_ring)"],
        producer: Some(site(TOPIC_ID, "topic_wire_subjects")),
        legacy: &[],
        consumers: &[consumer_at("check (the topic rows, built once per bundle on the TopScope)", RESOLVE, "build_top_scope"), consumer_at("model (the scope's topic rows: each topic's wire, and the gate merge per wire)", MODEL_BUILDER, "ModelInputs"), consumer_at("codegen (the lowering view's rows: dispatch, bindings, codecs, runtime shape registration)", CG, "topics: &resolved.top.topics"), consumer_at("codegen (the shm-ring bindings: each bound topic's wire and payload, from its row)", CG, "collect_shm_ring_subjects"), consumer_at("codegen (the routing-key, on_full-fail and subscriber-bound tables, from the rows)", CG, "collect_routing_key_subjects"), consumer("topology (topic shapes)"), consumer_at("resolved program (the intra-locus relation's wire subjects)", TY_RESOLVED, "topic_wire_subjects")],
        invariants: &[
            "delivery joins on the subject's identity, never on the written topic name (spec/model.md rule 8)",
            "a literal subject at a delivery site (a literal subscription, a literal send) names only the topic that OWNS that wire subject, `TopicRows::by_wire`, never one whose declared segment or name it happens to spell; a topic reference names its declaration; a wire subject two topics carry names neither and is an error",
            "lowering reads the topic rows of the lowering view's scope (`Cx::topics`, over the program it lowers): a topic's wire subject, payload, routing key, bound and on_full policy come from its row, and codegen computes no wire subject of its own",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-codegen/tests/topic_declarations.rs", "crates/hale-codegen/tests/replica_keys.rs", "crates/hale-codegen/tests/serializer_shape.rs", "crates/hale-types/src/topic_identity.rs (a_subject_names_its_topic_by_one_rule, a_shared_subject_names_no_topic)"],
        spec: &["spec/semantics.md § Topic declarations", "spec/semantics.md § Phase 3: routing keys"],
        owned: &[site(TOPIC_ID, "TopicRows::of"), site(TOPIC_ID, "by_wire")],
        seams: &[
            Seam { symbol: "topic_wire_subjects(", allowed: &[(TOPIC_ID, 2), (BUS_GRAPH, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "TopicRows::of(", allowed: &[(TOPIC_ID, 1), (RESOLVE, 1)] },
            Seam { symbol: "by_wire(", allowed: &[(TOPIC_ID, 6), (CHECK, 2)] },
        ],
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
            legacy(TY_RESOLVED, "from_gates", "the resolved program derives lowering's plan with an empty domain map (#464's widening is a separate optimization); the model derives its own with the arrangement's domains for `same_domain`", "phase 3, one plan: lowering's plan takes the arrangement's domains only with #464's widening, an optimization with its own bench gate not yet taken on, so the two plans still differ by their domain maps (at the phase-2 close)"),
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
            legacy(CG_CHANNELS, "resolve_failure_route", "the parent instance is the lowering context's (supervising parent, then self, then params-init self); the handler is the row's", "phase 3, when the instance is a row of an instance tree: the snapshot has no instance-tree family, so the parent instance is still the lowering context's (at the phase-2 close)"),
            legacy(CG, "__StdBusUnixConnectTransport", "the transport-loss handler is picked by name through the routing table", "the bindings family names the transport's locus"),
            legacy(MODEL_BUILDER, "fn_rows", "the model's function rows key a failure handler by a signature string built from its params' written types", "phase 3, keyed by the row's SiteId, with handler routing by identity: the routing rows carry SiteId columns no model reader joins on yet (the phase-2 exit's #1199 re-measurement), and round 3's findings 16 (a monomorph's rows found by linear scan) and 17 (a generic supervisor's unsubstituted row) land with that join"),
            legacy(MODEL_BUILDER, "SupervisedRef::External", "a child the routing rows resolve as external is recorded by its written name", "phase 3, keyed by the row's SiteId, with the same join as `fn_rows` (still by written name at the phase-2 close)"),
            legacy(CG_INST, "settles_failures", "whether the parent has any handler, read from the lowering's handler table at the params-settle bracket", "phase 3, a query over the routing rows: lowering still reads its own `LocusInfo::failure_handlers`, and joins the rows to LLVM functions by the two positional ordinal joins the #1199 re-measurement found unchanged (at the phase-2 close)"),
        ],
        consumers: &[consumer_at("codegen (the handler table)", CG_DECL, "handlers_of"), consumer_at("codegen (handler bodies, by the row's ordinal)", CG_METHOD, "handlers_of"), consumer_at("codegen (__parent_on_failure)", CG_CHANNELS, "resolve_failure_route"), consumer_at("codegen (restart in place)", CG_RESTART, "restarts_in_place"), consumer_at("model (supervises, over the snapshot's rows: `demand_handlers`)", MODEL_BUILDER, "Supervises"), consumer_at("check (duplicate handlers, over the snapshot's rows handed in: `CheckInputs`)", CHECK, "check_duplicate_failure_handlers"), consumer_at("check (@supervised, over the same rows)", FRONTIER, "supervised_diags")],
        invariants: &[
            "the child type is resolved once, by `child_locus_name`; lowering, the checker and the model read the same row",
            "the checker builds no rows: the snapshot demands them before the check (`CheckInputs`), and the checker's duplicate-handler rule, the `@supervised` law and the model read that one build; a bundle no snapshot holds (the test entries) builds them once, in `bundle_handler_rows`",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/lifecycle_flow.rs (on_failure_dispatch_by_child_type)", "tests/hale/on_failure_per_child_type_test.hl", "crates/hale-types/tests/violate.rs", "crates/hale-types/tests/handler_routing_probes.rs"],
        spec: &["spec/semantics.md § failure", "spec/runtime.md (failure delivery)"],
        owned: &[site(HANDLER_ROUTING, "child_locus_name")],
        seams: &[
            Seam { symbol: "handler_rows(", allowed: &[(HANDLER_ROUTING, 1), (TY_RESOLVED, 1), (SNAPSHOT, 1), (TLIB, 1)] },
            Seam { symbol: "child_locus_name(", allowed: &[(HANDLER_ROUTING, 2), (OWNERSHIP_GRAPH, 1), (TY_OWN, 1), ("crates/hale-types/src/flows.rs", 1)] },
            Seam { symbol: "DeclaredNames::of(", allowed: &[(HANDLER_ROUTING, 1), (OWNERSHIP_GRAPH, 1), (TY_OWN, 1), ("crates/hale-types/src/flows.rs", 1)] },
        ],
    },
    Family {
        name: "flows",
        layer: Layer::Locus,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which children are flows (released per completion) and which are resident.",
        inputs: &["release declarations", "accept declarations", "declared loci and type aliases (handler_routing's resolver)", "import renames"],
        producer: Some(site("crates/hale-types/src/flows.rs", "survey")),
        legacy: &[],
        consumers: &[
            consumer_at("check --flows (each flow type as written, with its clauses)", V_CHECK, "flows::survey("),
            consumer_at("check (a daemon-shaped locus that accepts a child type it releases no clause for: a law over the rows)", CHECK, "check_accept_release"),
            consumer_at("resolved program (the lowering view's rows, over the merged program)", TY_RESOLVED, "flows::survey("),
            consumer_at("codegen (run elision, run-end reclaim and the release call: `Cx::is_flow`, one row read)", CG, "is_flow"),
            consumer_at("codegen (the generic-instantiation queue: each locus specialization it creates asks the row for its template's clauses, under the substitution its synthesis applies)", CG, "specialize("),
        ],
        invariants: &[
            "a release clause's child is resolved once, by `child_locus_name` (handler_routing's resolver: aliases, generic instantiations, qualified paths), into the row (`FlowClause::locus`); lowering's flow-ness is a row read (`flows::is_flow`), never a comparison of its own",
            "the flow facts cover the specializations lowering creates: a clause whose type mentions its owner's type parameters names no locus by itself and carries its template (`FlowClause::template`: the owner's identity, its parameters in order, the type as written); `FlowRows::specialize` answers for one specialization by resolving the template's type under the substitution lowering's synthesis applied, so `Manager<Worker>`'s `release(c: T)` makes `Worker` a flow exactly as a concrete `release(c: Worker)` does",
            "the checker's accept/release rule judges over the rows: the release clauses a locus declares are the rows' clauses inside its declaration",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/release_reclaims_flow.rs", "crates/hale-codegen/tests/release_two_parents.rs", "crates/hale-codegen/tests/release_generic_owner.rs", "tests/hale/release_generic_owner_test.hl", "crates/hale-types/src/flows.rs (a_clause_names_the_locus_lowering_names, a_template_clause_names_the_specialization_s_argument)"],
        spec: &["spec/semantics.md § release(c) and flow children"],
        owned: &[],
        seams: &[Seam { symbol: "flows::survey(", allowed: &[(CHECK, 1), (TY_RESOLVED, 1), (V_CHECK, 1)] }],
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
        consumers: &[consumer_at("check --dump-api (the snapshot's surface, the one its binding serves)", V_CHECK, "api_surface"), consumer("build (the description the binding serves)"), consumer("describe / call / watch / admin"), consumer("ui (reserved)"), consumer("bundle (reserved)")],
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
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which effect classes each fn and locus reaches (the callgraph fixpoint), the declared classes and their `causes:`/`depends:` DAG, and the certificate relating the two.",
        inputs: &["effect annotations", "the callgraph", "stdlib_surface (leaf effects)", "effect_class_table", "ffi names"],
        producer: Some(site(EFFECT_ROWS, "derive_effect_rows")),
        legacy: &[],
        consumers: &[consumer_at("check", CHECK, "check_decorator_stacks"), consumer("claims (certificate, causes, depends, budget)"), consumer_at("the certificate evidence (the check's effects certificate report, read by the check's laws and the artifact's)", SNAPSHOT, "demand_effect_certificates"), consumer_at("check (a codec binding's purity assertion reads the purity column, demanding the rows only when a codec reaches it)", CHECK, "CheckInputs"), consumer_at("model (effect labels, lower bounds and direct contributions, the last read by the reachability judgment's `effects(C)` test, and the summary the rows' walk read: the snapshot's rows, handed in)", MODEL_BUILDER, "ModelInputs"), consumer_at("the effects manifest (`--dump-effects-manifest`, `--check-effects-manifest`: the snapshot's rows, cross-seed calls resolved through the renames)", V_CHECK, "demand_effects"), consumer_at("replay (the live-effects gate reads the manifest over the snapshot's rows, and refuses when they are blocked)", V_REPLAY, "demand_effects"), consumer("doc")],
        invariants: &["derived ⊆ declared is the certificate; budgets are judged through evidence (spec/verification.md)", "effects run once per snapshot, not once per consumer: `Snapshot::demand_effects` runs `derive_effect_rows` once over the snapshot's allocation summary (`demand_alloc_summary`: the checked programs and the stdlib's analysis copy, cross-seed calls resolved through the import renames), counted as `effects`, blocked with the scope", "an unresolved edge is coverage, never a violation: a row's `effects` saturates to `UNCLASSIFIED` when the walk reaches what it cannot name, its `known` set is the lower bound an unresolved edge never erases, and `unknown` says the walk reached such an edge", "a row is keyed by the fn's name (`FnKey`) until the `snapshot_identity` family's declaration rows carry it", "a fn's direct contribution is a column (`direct`; `EffectRows::direct` answers any key, a bodyless one by what it carries): the model's function rows and absorbed paths, which the reachability judgment's `effects(C)` destination test reads, take it from the rows, and nothing outside the producer folds a body for it", "purity and the lower bound are columns of the rows: one walk answers a fn's saturating set and its lower bound (`infer_effect_bounds`), and the purity walk runs only inside the producer; the checker's codec law reads the purity column through `CheckInputs::effects`, a demand made only when a codec binding reaches the assertion, so a check of a program that binds no codec runs no effects fixpoint", "the effects certificate engine runs once per snapshot, in the check (`check_bundle_reporting`); the certificate evidence reads that report (`Snapshot::demand_effect_certificates`, handed to `derive_certificate_evidence_over`) and never runs the engine itself; a bundle no check ran over runs it once for itself (`effect_certificates`)"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/effect_assertions.rs", "crates/hale-types/tests/effects_cross_seed_correction.rs", "crates/hale-cli/tests/effects_baseline_gate.rs", "crates/hale-cli/tests/effects_manifest.rs"],
        spec: &["spec/verification.md § Default-on & opt-in analyses", "spec/verification.md § Claims"],
        owned: &[site(FRONTIER, "infer_effects"), site(FRONTIER, "infer_effect_bounds"), site(PURITY, "infer_purity_for_bundle"), site(EFFECT_ROWS, "EffectRows"), site(EVIDENCE, "derive_certificate_evidence"), site(EVIDENCE, "derive_certificate_evidence_over")],
        seams: &[
            Seam { symbol: "infer_effects(", allowed: &[(FRONTIER, 4), (CLAIMS, 1), (TOPOLOGY, 1)] },
            Seam { symbol: "effect_manifest_with_inference(", allowed: &[(EFFECTS, 1), (TLIB, 1), (V_REPLAY, 1)] },
            Seam { symbol: "infer_purity_for_bundle(", allowed: &[(PURITY, 2), (EFFECT_ROWS, 1)] },
            Seam { symbol: "infer_effect_bounds(", allowed: &[(FRONTIER, 2), (EFFECT_ROWS, 1)] },
            Seam { symbol: "direct_effects(", allowed: &[(EFFECT_ROWS, 2)] },
            Seam { symbol: "effect_report_grouped(", allowed: &[(EFFECTS, 3), (CHECK, 1)] },
            Seam { symbol: "derive_certificate_evidence(", allowed: &[(EVIDENCE, 1)] },
            Seam { symbol: "derive_certificate_evidence_over(", allowed: &[(EVIDENCE, 2), (JUDGMENT, 1), (TOPOLOGY, 1)] },
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
        producer: Some(site(ALLOC, "derive_alloc_summary")),
        legacy: &[
            legacy(CHECK, "check_hot_path_alloc", "hot-path allocation lint over a hand-kept receiver list, keyed by name and `__lib_` suffix", "a law over the rows"),
            legacy(ALLOC, "ReclaimScope", "the checker's reclaim model, stale for scratch-local fns since #1208", "one model"),
        ],
        consumers: &[consumer_at("the effects certificate engine (the check's `@effects`, `@phase_effects` and placement diagnostics: the snapshot's summary, handed in)", CHECK, "CheckInputs"), consumer_at("effects (the rows walk the snapshot's summary and hold it, shared)", SNAPSHOT, "demand_effects"), consumer_at("check (the unbounded-allocation advisory and `--dump-alloc-summary`: the snapshot's summary)", V_CHECK, "demand_alloc_summary"), consumer_at("lsp (the advisory in the diagnostics, and hale/allocSummary: the snapshot's summary)", LSP, "demand_alloc_summary"), consumer_at("model (its function rows, dispatch sites and holes: the snapshot's summary's own rows)", MODEL_BUILDER, "own_rows"), consumer_at("topology (the artifact's fn sort, labels, derived effects and through-stdlib contraction: the snapshot's summary, handed in)", TOPOLOGY, "dump_topology_over"), consumer("check (hot path)"), consumer_at("claims (@budget: the counting engines over the snapshot's summary's own rows, handed to the evidence)", EVIDENCE, "derive_certificate_evidence_over"), consumer_at("codegen (arena routing at an allocation)", CG, "current_arena_ptr"), consumer_at("codegen (a free fn's scratch: the view's non-allocating and scratch-local rows)", CG, "alloc_routing"), consumer_at("codegen (a locus's arena, and its hooks', methods' and modes' scratch: the elision rows, a monomorph's specialized)", CG, "locus_elision"), consumer_at("resource_budget (the fd sites and the fd-leak warnings: the snapshot's summary's own rows)", "crates/hale-types/src/resource_budget.rs", "own_rows"), consumer_at("frontier (the `causes:` engine: the summary's own rows)", FRONTIER, "causes_inner")],
        invariants: &[
            "one summary per snapshot: `Snapshot::demand_alloc_summary` runs `derive_alloc_summary` once over the checked programs with the stdlib's analysis copy beside them (cross-seed calls resolved through the import renames), counted as `alloc_summary`, blocked with the scope; the check's effects certificate engine reads it (`CheckInputs::alloc_summary`) and the effect rows walk it, so neither builds its own; `summarize_identified` is the one constructor, and `derive_alloc_summary` (which places the stdlib's analysis copy beside the programs itself) its one caller outside tests, so no reader builds a summary variant of its own",
            "the checker's reclaim model and codegen's routing agree; a stale copy is a registry violation, not a comment",
            "lowering routes from the view's rows (`LoweringView::alloc_routing`, `derive_alloc_routing` over `merged` with the view's renames) and derives none: a free fn skips its per-call scratch when the rows call it non-allocating (FORM-3, a greatest fixpoint keyed by name), and its body allocates into its own subregion when they call it scratch-local (#1148); a locus's arena is elided, and a lifecycle hook, `fn` method or mode lowers without its per-call scratch, when its elision row says so (`LocusElision`, keyed by the locus and the member's position; a generic locus's monomorph, which lowering synthesizes, takes `AllocRouting::specialize` over the synthesized declaration); the FORM-3 classifier (`fn_body_definitely_non_allocating`) is the rows' own and lowering calls it nowhere; the two method-elision stages still classify a self field differently (stage 1, the per-member verdict, by its literal default or its ascription through aliases; stage 2, the `self.m()` sets, by a primitive ascription only), and a monomorph still has no stage-2 set, both as they were; a free fn's entry publishes its caller's arena to the caller-arena TLS when its row says so (`caller_arena_publish`: not non-allocating, and a call or a struct literal anywhere in its body, found by a structural walk, a string literal spelling `Call {` or `Struct {` counting as the Debug-string test it replaced counted it; a generic fn's monomorph takes `AllocRouting::specialize_fn`), and no body's Debug rendering decides it; the rows are #1208's classification moved as it was, so its builtin list is still a hand-kept subset of the checker's `BARE_BUILTIN_CALLEES` with no agreement test, its `std::` namespaces a per-namespace claim no stdlib_surface row states, and its qualified-path lookup checks the import renames before `PATH_RENAMES` where `resolved::lookup_qualified_path` checks them in the other order",
            "a body's escape tags key a binding by the declaration its escaping uses name (`Snapshot::declaration_of`), so an inner shadow of a returned name is its own, local binding (the #1140 shape), and close over `let x = y;` aliases to the declaration y names, as borrow_lifetime's `returned_decls` does; programs minted by different snapshots are summarized each with its own identities (`summarize_identified`: a bundle's programs beside the bundled stdlib's analysis copy)",
            "each seed's names resolve in its own scope: the programs minted with one set of identities are one scope, a body's bare free-fn name (and the locus a call's result is typed by) resolves only to a fn of its own scope, and the import renames are the bundle's names (the stdlib's analysis copy imports nothing), so a stdlib body's builtin `count(...)` is the builtin, never a user fn called `count` (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)",
            "the unbounded-invocation fixpoint (`AllocSummary::unbounded_invoked`) seeds from what the program reaches: beside the stdlib's analysis copy, `AllocSummary::reached` is the program's own fns, what their calls reach (the interface fan-out included) and the hooks and bus handlers of every locus they start (a struct literal of it in a reached body, or a param field of a started locus by declared type or default literal; every locus of the program's own is started), and only those seed it or call, so a stdlib loop the program never starts invokes nothing (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)",
            "the run-to-exit rule (a `main` and no long-lived entry: no leak sites) reads the program's own entries, never the stdlib's analysis copy's (`AllocSummary::analysis_copy` names the copy's fns, `is_own` the program's), since the copy always carries `run` hooks (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)",
            "a program's declaration is the row where the stdlib's analysis copy declares the same name: the copy's top-level free fn, locus or interface of a name a checked program declares stays out of the summary, so a stdlib source file checked as itself keeps its own rows (its bodies, its spans, resolved in its own scope); only stdlib source shares such a name (a classified correction, pinned in `alloc_summary_construction_correction.rs`)",
            "the advisory, `--dump-alloc-summary` and the editor's hale/allocSummary read the snapshot's summary and report the program's own rows judged over the whole of it (the stdlib's analysis copy and the renames included: a classified correction, pinned per target in `alloc_summary_correction.rs`); a leak site is the program's own (`AllocSummary::is_own`) and is left out of the advisory only when it has no author position (`AuthorPositions::has`: its span at or beyond `API_SYNTH_BASE`, or in a declaration the origin rows mark synthesized whose offset no source file owns), never by its owner's name; the check's warnings, the editor's diagnostics and the editor's hale/allocSummary decide with one function (`advisory_leak_sites`)",
            "the model projects the program's own rows; a call into the analysis copy is the unresolved row: the model, the `@budget` engines (`budget_check`, `quantitative`), the artifact's user rows, the frontier's `causes:` engine and the resource budget (its fd sites and fd-leak warnings) read the snapshot's summary through `AllocSummary::own_rows` (the program's fns and loci; a call resolved into the stdlib's analysis copy is the unresolved call by the method's bare name, the receiver kept; a dispatch keeps its alternatives among the program's own loci, renumbered in key order, through the copy's interface is no dispatch, and through the program's own with none of its conformers left is the dead site), which is the program-alone summary field for field, so the copy moves no `shape_hash`, the build and replay identity, and the copy's fd-acquiring bodies are not the program's resources (pinned per target in `alloc_summary_own_rows.rs`); what the model would gain from the copy's rows is a separate correction",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/hot_path_alloc.rs", "crates/hale-types/tests/alloc_summary_construction_correction.rs", "crates/hale-types/tests/alloc_summary_correction.rs", "crates/hale-types/tests/alloc_summary_own_rows.rs", "crates/hale-codegen/tests/scratch_local_free_fn.rs", "crates/hale-codegen/tests/fn_nonalloc_add.rs", "crates/hale-codegen/tests/method_scratch_elision.rs"],
        spec: &["spec/memory.md § Allocation routing", "spec/styleguide.md"],
        owned: &[site(ALLOC, "summarize_identified"), site(ALLOC_ROUTING, "derive_alloc_routing")],
        seams: &[
            Seam { symbol: "summarize_identified(", allowed: &[(ALLOC, 4)] },
            Seam { symbol: "derive_alloc_summary(", allowed: &[(ALLOC, 1), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2), (EFFECTS, 1), (EVIDENCE, 1), (JUDGMENT, 1), (TOPOLOGY, 1), ("crates/hale-types/src/resource_budget.rs", 1)] },
            Seam { symbol: "own_rows(", allowed: &[(ALLOC, 1), (MODEL_BUILDER, 1), ("crates/hale-types/src/budget_check.rs", 1), ("crates/hale-types/src/quantitative.rs", 1), (FRONTIER, 1), ("crates/hale-types/src/resource_budget.rs", 2)] },
            Seam { symbol: "derive_alloc_routing(", allowed: &[(ALLOC_ROUTING, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "fn_body_definitely_non_allocating(", allowed: &[(ALLOC_ROUTING, 12)] },
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
        consumers: &[consumer_at("check", V_CHECK, "borrow_lifetime_diags"), consumer_at("build, run, test, replay, bench (a build config's snapshot check, `Config::build_rules`)", TLIB, "build_rule_diags")],
        invariants: &["runs on every entry point: it does not run in the LSP or bench today (phase 2 closes that)", "its accept-set walk and its returned-binding walk are ownership residues, listed under `ownership` (borrow_lifetime.rs `accepts`, `returned_decls`)"],
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
        consumers: &[consumer_at("check", V_CHECK, "bare_fallible_calls"), consumer_at("build, run, test, replay, bench (a build config's snapshot check, `Config::build_rules`)", TLIB, "build_rule_diags")],
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
        inputs: &["placement and topology blocks", "entrypoint (the lowering root, never the entry)", "the construction templates: the root's literals, the entry's implicit construction of a root no literal builds, `fn main`'s own literals, the root's `bindings { }` adapters", "the params towers each template builds", "the minted sites of the snapshot and of the stdlib analysis copy", "free fns and locus bodies (dynamic sites, their domains and bounds)"],
        producer: Some(site(PLACEMENT, "derive_placement")),
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
        consumers: &[consumer("check (rules 2-5, 13-18)"), consumer_at("check (F.31: the caller per instance, the receiver by its row)", CHECK, "check_placement_single_thread"), consumer_at("sync_inference (accessor domains per instance)", SYNC, "infer_sync_for_bundle"), consumer("dispatch (domains)"), consumer("model (placed_in, affined_to)"), consumer("codegen (pools, mailboxes, affinity)"), consumer("lsp (hale/placement)"), consumer("deployment (reserved)")],
        invariants: &[
            "placement is keyed by instance, never by type: one row per static instance of each construction template (a key is its origin, its field path, its replica), and a type's answer is the set of its instances' domains",
            "the entry is a construction scope: a root no literal builds is the entry's implicit template (`Origin::Entry`, bound `Once`), and `fn main`'s own literals are templates bound by their statement's loop context; an adapter is an origin of its own, built once",
            "nested rows inherit their owner's domain unless a root field's entry or a binding decides it; pinned domains are per anchor and per replica, pool domains one per name with at most one affinity",
            "the root is `lowering_root`, never the entry; an imported `main` is never the root",
            "every root entry decides exactly one field family in each construction template, or it is a hole",
            "unknown is a hole, not a default: an unresolved declaration, an unenumerable initializer, a held instance (`Reuse`) and a dynamic site of unknown domain each carry their policy, and none is main",
            "a held instance's subtree lives in its holder's domain: the held row keeps its `Reuse` hole and its owner's domain, the source's actual rows (never the declaration's defaults) are projected under it, inherited, and each of those rows names its own source row, the one it was built as (`built_by`); where the source is not linked, nothing below the held row is asserted, and an instance there runs in an unknown domain; a question of where an instance runs skips the source's rows, a count of instances skips the held ones",
            "every site the table names carries the universe that minted it (`SiteRef`); lowering joins the stdlib's into its merged mint once, totally and injectively",
            "the checker builds no table: the snapshot demands it and hands it to the check (`CheckInputs::placement`) and to the form rows; a bundle no snapshot holds (the test entries `check_bundle`, `check_bundle_opts_scoped`, `derive_application_model`, `effect_certificates`) builds it once, over a minted bundle (an unminted one names no site and gets an empty table, which judges nothing)",
            "a consumer asks where instances run of `PlacementTable::running` (the handed-off rows skipped): a type's answer is the domains of its instances, compared per instance, never collapsed to one per type; an enclosing locus with no static instance is a hole that disables the F.31 proof, never a default to main or to pinned",
            "F.38: placement is semantics-free, so a backend may Approximate it",
            "placement is a choice point: v1's declared placement is the single candidate",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/placement.rs", "crates/hale-types/tests/placement_pairings.rs", "crates/hale-codegen/tests/pool_affinity.rs", "crates/hale-codegen/tests/placement_where_async_io.rs", "crates/hale-types/tests/placement_table.rs (the table through the frontend's load: the correspondence's coverage cases 1 to 16, the two universes joined into lowering's mint, the table's laws over every clean fixture, and a check that demands it once)", "crates/hale-types/tests/shadow_placement.rs (the table against every legacy producer the snapshot reaches, over the corpus, tests/hale, the DNA seeds and the coverage fixtures, every divergence classified under the correspondence's rows and pinned per producer, rows and declaration)", "crates/hale-types/tests/form_rows.rs (sync inference per instance, K-5)"],
        spec: &["spec/semantics.md § Placement block (F.31)", "spec/decisions.md F.31, F.35, F.38"],
        owned: &[],
        seams: &[
            Seam { symbol: "derive_placement(", allowed: &[(PLACEMENT, 1), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2), (EFFECTS, 1), (SYNC, 1)] },
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
        producer: Some(site(CAPABILITY, "derive_capability_matrix")),
        legacy: &[
            legacy(CHECK, "wasm_unavailable_stdlib", "a hand-kept slice-pattern table keyed by leading namespace, consulted only when the SOURCE declares `target wasm` (never from `--target wasm32`), and only for call forms", "one CapabilityMatrix consulted by the driver before lowering"),
            legacy(CHECK, "wasm_target", "the source-declaration flag the table is gated on", "same"),
            legacy(CG, "link_wasm", "link-time refusals (link_libs) and the export list", "same"),
            legacy(CG_INST, "lotus_replay_start_ingress", "one of the per-site wasm skips; on wasm the eager main-locus spine emits the pool join where every other spine omits it, and every spine but the deferred entry emits wait-abort", "same"),
            legacy(CG, "is_wasm", "31 sites read it: 14 are emission choices (a TargetSpec query, never a cell), the rest decide a behaviour, the link path or an obligation, each classified in the lowering shadow's site inventory", "emission configuration through TargetSpec only; every capability through the matrix"),
            legacy(CHECK, "ffi_type_unportable", "FFI portability per type", "a capability row"),
            legacy(TY_TARGET, "TargetSpec", "has_async_io is true for wasm32; the checker sees the target only under `hale build`", "the matrix is the one statement, on every entry point"),
        ],
        consumers: &[
            consumer("check"),
            consumer("build"),
            consumer_at("docs (systems/webassembly.md § What wasm32 can do, and spec/ffi.md's refused-namespace table: regions rendered from the cells)", CAPABILITY, "render_markdown"),
            consumer_at("the effective-target row, on the snapshot (consulted by nothing yet)", SNAPSHOT, "demand_target"),
        ],
        invariants: &[
            "Approximate is legitimate only in layers 5 and 7; everywhere else a target lowers or rejects, with the row's witness",
            "the docs' target statement is generated, never hand-maintained",
            "every (target class, capability), (target class, invocation) and (target class, obligation) pair has exactly one written cell: no default arm, the musl column written out like the others",
            "behaviours and obligations are distinct types: an obligation is omitted only on a premise (a Reject behaviour, a Refused invocation, a registered proof) that holds on its own target, and a Lower behaviour requires only Lower behaviours",
            "a capability refusal depends only on the program, the configuration and the target; a failure that depends on the machine running the compiler stays a toolchain error",
            "a target's class comes from its (arch, os, env), never from a triple's name; the effective target is a function of the snapshot key's configured target and its sources",
        ],
        missing: Missing::Error,
        tests: &[
            "crates/hale-types/tests/wasm_target_gating.rs",
            "crates/hale-codegen/tests/wasm_target.rs",
            "crates/hale-cli/tests/target_model.rs",
            "crates/hale-types/src/capability/laws.rs (the matrix's laws: one cell per pair, anchored witnesses, premises, requires, KNOWN_OPEN still today's answer)",
            "crates/hale-types/tests/shadow_capability.rs (the checker rows against their cells over the corpus, tests/hale, the DNA seeds and the wasm programs, on three columns: 0 divergences)",
            "crates/hale-codegen/tests/shadow_capability_lowering.rs (the codegen rows, the thread behaviours and @ffi(\"js\") against their cells, both targets built; 7 classified divergences, all the design's: 3 PoolJoin, 4 @ffi(\"js\") native links)",
            "crates/hale-cli/tests/shadow_capability_cli.rs (run, replay, record and --wrap-main against their cells; 1 classified divergence, T1's --wrap-main spelling)",
            "crates/hale-types/tests/capability_doc_matches.rs (both document regions equal the rendered matrix)",
        ],
        spec: &["spec/decisions.md F.35", "spec/ffi.md § The `target` declaration + stdlib gating", "docs/src/systems/webassembly.md"],
        owned: &[],
        seams: &[
            Seam { symbol: "wasm_unavailable_stdlib(", allowed: &[(CHECK, 2)] },
            // the definition and the document rendering, and the laws
            Seam { symbol: "derive_capability_matrix(", allowed: &[(CAPABILITY, 2), ("crates/hale-types/src/capability/laws.rs", 12)] },
            Seam { symbol: "target_row(", allowed: &[(CAPABILITY, 1), (SNAPSHOT, 1)] },
        ],
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
            legacy(CG, "lower_program", "fn main's fall-through exit quiesces ingress and joins the pools before its flush, a third copy of the join-before-free order (the wait-abort comes from the flush's `in_main` gate)", "same"),
            legacy(CG, "main_test_fail_bb", "fn main's test-failure exit repeats the quiesce and the join before its frame teardown, a fourth copy", "same"),
            legacy(CG, "lower_return_inner", "a `return` from fn main repeats the quiesce and the join before its teardown, a fifth copy", "same"),
            legacy(CG, "__reclaim_", "the reclaim spine", "same"),
            legacy(CG_DISSOLVE, "emit_locus_arena_destroy", "the cascade (field drains, field dissolves, arena destroy)", "same"),
            legacy(CG_RESTART, "define_restart_fns", "restart and resume", "same"),
        ],
        consumers: &[consumer("codegen (emission reads the order)"), consumer("closures (the event alphabet)"), consumer("transitions (reserved)"), consumer("deployment (reserved)")],
        invariants: &[
            "handlers run only on the queue owner's thread, so cross-thread failure delivery follows spec/runtime.md (a typed bus message): the first named decision, with its own regression test",
            "a spec/implementation disagreement is settled as a named decision, never by extraction picking a side",
            "an obligation is keyed by its source site (the declaration and P1's construction template); the runtime mints the instance and its incarnation, the table never does",
            "every obligation ends in exactly one of its named terminal alternatives; lifetime (what stays alive until which event) and progress (what makes it reach a terminal) are separate fields",
            "each rule says whether it is shipped, adopted, known open at an inventory row, or pending on a named condition",
            "the runtime's protocol is checked by its trace, not trusted: a trace build reports the hold, the settle and each held delivery, and the trace oracle holds them to the plan's edges (delivered after the owner's settle, before its birth), with negative controls that remove or reorder a step and fail it, over the lifecycle fixtures, every runnable example and every cell of the lifecycle matrix",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/lifecycle_flow.rs", "crates/hale-codegen/tests/reclamation_spine.rs", "crates/hale-codegen/tests/main_locus_deferred_pool_join.rs", "crates/hale-codegen/tests/teardown_pinned_join_order.rs", "crates/hale-types/src/lifecycle.rs (the schema's laws: every decision line binds a kind, the Pending lines are the named ones, the doc table is the data)", "crates/hale-codegen/tests/lifecycle_fixtures.rs (a fixture per decision line under tests/fixtures/lifecycle/; KNOWN_OPEN pins today's outcome where it differs from the adopted one; the trace oracle holds each run to its line's plan, TRACE_KNOWN_OPEN names today's departures, CONTROLS fail it)", "crates/hale-types/src/lifecycle/trace.rs (the trace's parser and oracle)", "crates/hale-codegen/tests/corpus_oracle.rs (corpus_traces_keep_the_lifecycle_laws: every runnable example, traced)", "crates/hale-codegen/tests/lifecycle_matrix.rs (failure phase × tree position × domain, a generated program per cell held to its outcome, its trace plan, ASan on the sample and the let-bound differential; KNOWN_OPEN names today's failing cells; HALE_MATRIX=full runs every cell)"],
        spec: &["spec/runtime.md (failure delivery; pool join rule b)", "spec/runtime.md § Lifecycle obligations (the decision lines, adopted and shipped told apart)", "spec/runtime.md § The lifecycle trace (a debug aid, not a contract)", "spec/semantics.md § lifecycle"],
        owned: &[site(LIFECYCLE, "LifecyclePlan"), site(LIFECYCLE_TRACE, "Expected")],
        seams: &[],
    },
    Family {
        name: "bus_inert",
        layer: Layer::Lifecycle,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Whether the program can ever have a bus cell in flight, so drains can be elided.",
        inputs: &["the user program's declarations (topics, perspectives, bus and bindings blocks, accepts)", "the names the user program spells (`hale_syntax::names`)", "the stdlib's bus-taint column (which `std::` namespaces reach bus surface)"],
        producer: Some(site("crates/hale-types/src/bus_inert.rs", "bus_inert")),
        legacy: &[],
        consumers: &[
            consumer_at("the resolved program (a row of the lowering view, over the user program before the stdlib merge)", TY_RESOLVED, "bus_inert::bus_inert("),
            consumer_at("codegen (drain elision: the row, read)", CG, "resolved.bus_inert"),
            consumer_at("codegen", CG_BUS_RT, "emit_bus_drain"),
        ],
        invariants: &[
            "a drain elision is a conclusion of structural rows, never of a rendering: the declarations the user program carries, and the names it spells (`hale_syntax::names`, an exhaustive walk) against the stdlib's bus-taint column (`bus_tainted_namespaces`, a fixpoint over the stdlib's declarations by the same walk); lowering reads the verdict (`LoweringView::bus_inert`) and derives none",
            "the test is by name and over-approximates (a local spelled like a tainted namespace keeps the drains); a query over the message graph that follows which stdlib subscribers a program actually reaches would elide more, and is a separate change with its own shadow",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/drain_elision.rs", "crates/hale-codegen/tests/log_routing.rs", "crates/hale-types/src/bus_inert.rs (the_verdict_reads_declarations_and_names)"],
        spec: &["spec/runtime.md § drain"],
        owned: &[site("crates/hale-types/src/bus_inert.rs", "bus_tainted_namespaces"), site("crates/hale-syntax/src/names.rs", "for_each_spelled")],
        // the definition and its unit test, and the resolved program
        seams: &[Seam { symbol: "bus_inert(", allowed: &[("crates/hale-types/src/bus_inert.rs", 2), (TY_RESOLVED, 1)] }],
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
            legacy(CG_INST, "CodegenError::Unsupported", "spanless refusals at lowering for rules the checker already states (rule 6's checker evaluator landed in phase 0; the backstop stays for harness builds that skip the checker); for a placed locus the checker types as Unknown, for an `accept()` with no parameter (the checker keys on `accept_param`, codegen on the method name), and for an adapter locus instantiated inline in a `bindings { }` block (which lowering pins without a placement entry), it is the only evaluator", "phase 3, when one pipeline guarantees the checker ran before lowering and the refusals become dead: every verb checks before it lowers, but the test harness's adapter `build_executable_with_options` builds through a harness snapshot that does not gate lowering on a check (`Config::harness`), and over three hundred test files build through it (at the phase-2 close)"),
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
        inputs: &["a checked bundle", "top_scope", "bus_graph", "ownership", "handler_routing", "placement", "effects", "alloc_summary", "topics", "bindings"],
        producer: Some(site(MODEL_BUILDER, "derive_application_model_over")),
        legacy: &[],
        consumers: &[consumer_at("demand (every verb and the LSP: the claims, over the snapshot's scope and graphs)", SNAPSHOT, "derive_application_model_over"), consumer_at("a bundle no snapshot holds (the test entry's)", TLIB, "derive_application_model_over"), consumer_at("claims (a caller not on the snapshot)", JUDGMENT, "derive_application_model"), consumer_at("topology (`hale check`'s artifact and both gates: the snapshot's model)", V_CHECK, "dump_topology_over"), consumer_at("topology (a bundle no snapshot holds)", TOPOLOGY, "derive_application_model"), consumer_at("model dump (the check's snapshot)", V_CHECK, "demand_model"),consumer_at("the build identity: the model hash and the obs ids (build, run, replay: the snapshot's model)", OPTIONS, "demand_model"), consumer("fleet (admits the artifact, never the model)")],
        invariants: &[
            "one constructor; no artifact → model, no plan → model, no hand-authored model",
            "hale-model is rebuilt on hale-graph (phase 1.1a): its seed, source and provenance ids and its provenance store are the graph core's, re-exported under the model's paths; its canary allows that one dependency and no other",
            "demand-gated: a no-claims check builds no model (GH #476 criterion 1); demand_gate.rs pins it as per-family accounting over `Snapshot::builds` (the `demand` family): the LSP's diagnostics path builds none, `hale check` of a program with claims builds one, which `--dump-model` reuses",
            "the model builds none of the families it reads beside the program (2.3): the scope with its topic rows, the bus graph, the ownership graph, the handler rows and the effect rows (with the stdlib-merged summary their walk read) arrive as `ModelInputs`, each demanded once from the snapshot over the checked programs; the allocation summary and the placement it still re-runs for itself are listed under their families",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/demand_gate.rs", "crates/hale-model/tests/architecture.rs", "crates/hale-types/tests/topology_projection.rs"],
        spec: &["spec/model.md"],
        owned: &[site(MODEL_BUILDER, "ModelInputs"), site(TLIB, "derive_application_model")],
        seams: &[
            Seam { symbol: "derive_application_model(", allowed: &[(TLIB, 1), (JUDGMENT, 1), (TOPOLOGY, 2)] },
            Seam { symbol: "derive_application_model_over(", allowed: &[(MODEL_BUILDER, 1), (TLIB, 1), (SNAPSHOT, 1)] },
        ],
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
        answers: "The identity of every semantic site in a snapshot: `(seed, index)`, minted after the entry point's desugars with the bundle's source map, and again in the resolved-program step (numbered over the user program before the intra-locus rewrite, so the sends it records are numbered on every path, and minted over the merged program with the bundle's seeds and a named seed for the bundled stdlib), idempotently (one numbering; a later mint numbers only what an earlier one did not see), with reliable provenance; and which declaration each use names (`binding_of`), resolved once by the mint.",
        inputs: &["seed_loading", "desugar_sequence"],
        producer: Some(site("crates/hale-types/src/snapshot.rs", "mint")),
        legacy: &[
            legacy(M_IDS, "FunctionId", "model ids are ranks in a sorted string order (`L::f`, `(name, kind)`, path strings)", "same"),
            legacy(EFFECTS, "FnKey", "analysis keys are (locus name, fn name)", "same"),
            legacy(CHECK, "type_expr_key", "rule 12 compares stringified TypeExprs", "same"),
        ],
        consumers: &[consumer("every table"), consumer("the shadow facility (compares through an explicit correspondence, never raw id equality)"), consumer("lsp (a later incremental future)"), consumer("the resolved program (codegen's input is minted over the merged program)")],
        invariants: &[
            "addresses are not identities (declarations are cloned); spans are not (the stdlib's coordinates overlap user files; desugars share spans)",
            "snapshot-local uniqueness and provenance are the requirement; persistent identity across editor revisions is a separate problem",
            "canonical ids need real equality and hashing; the AST's structural NodeId equality stays separate",
            "the identity's types are hale_graph::ids (SeedId, SiteId; phase 1.1a); hale_types::snapshot::mint numbers every site the AST walk hale_syntax::sites reaches with one counter: after the entry point's last desugar, and again, idempotently, in the resolved-program step over the merged program, numbering only what an earlier mint did not see (phase 1.1b); before the intra-locus rewrite that step only numbers the user program (`snapshot::number`: the rewrite moves a send's id onto its call and records it), which makes no snapshot of it; every entry point calls it after its last desugar with its source map and the bundle carries the result (every verb and the LSP through the snapshot's load, the test harness through `Snapshot::from_program`, with no source map); the lowering view mints with the bundle's seeds, the stdlib's sites under the named seed snapshot::STDLIB_SEED (its spans overlap the first file's); a generated site records the desugar that made it (Snapshot::origins); codegen's generic instantiation keeps the template's id, and the F.39 pre-pass numbers nothing: an unminted Struct or Call is an error",
            "every identifier expression is a `Use` site and every name a declaration with no site of its own binds (a fn's or a hook's parameter, a match pattern's binding, a tuple `let`'s name, a `shm_write` binding) a `Binder` site, their ids on their `Ident`s (use-site identity, phase 2)",
            "`binding_of` is one resolution per use, keyed by identity, never by name or span: the mint resolves each `Use` site, and each `Assign` site's head, to the `Let`, `For` or `Binder` site it names under the checker's scoping (`check::ScopeStack`), once per snapshot (the load's, the lowering view's, the bundled stdlib's analysis copy's once per process: `demand_gate` pins it); a use that names no local binding has no row; every reader asks `Snapshot::declaration_of` and resolves nothing itself, and every entry point mints (`check_program` too)",
            "a check of a bundle no entry minted (`Bundle::new` over parsed programs, a library caller's or a test's) mints a copy of its programs once, with its source map, before any family is derived (`with_identities`, the no-snapshot adapter of `check_bundle` and `check::check_bundle`): its bus graph's sends and the intra-locus rewrite's relation, whose own numbering keeps those ids, name a send by one id, so rule 10's join answers on that path as on the snapshot's; a send with no id reaching the join is refused as an internal failure naming the send, never judged as queued",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding)", "crates/hale-codegen/tests/owner_table.rs", "crates/hale-types/tests/snapshot.rs (each_use_resolves_to_the_declaration_in_scope)", "crates/hale-types/tests/demand_gate.rs (each_snapshot_resolves_its_uses_once)", "crates/hale-syntax/tests/sites.rs"],
        spec: &["spec/decisions.md F.39, F.40"],
        owned: &[site(SITES, "SiteKind"), site(TY_SNAPSHOT, "resolve_uses"), site(TY_SNAPSHOT, "declaration_of"), site(TY_SNAPSHOT, "number")],
        seams: &[Seam { symbol: "mint(", allowed: &[(TY_RESOLVED, 1), (SNAPSHOT, 1), (TLIB, 2), (STDLIB_BODIES, 1), (ALLOC, 1), (SYNC, 1)] }],
    },
    Family {
        name: "demand",
        layer: Layer::Identity,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, each member's own program beside the merged one, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph and handler rows, the effect rows, the model, the check with the effects certificate report its typing produced, the lowering view), each at most once, blocking a family whose prerequisite reported errors.",
        inputs: &["seed_loading", "desugar_sequence", "snapshot_identity", "the config (target, api, api roles, environment, the check's rules)", "editor overlays (LSP)", "a consumer's request"],
        producer: Some(site(SNAPSHOT, "Snapshot")),
        legacy: &[],
        owned: &[site(SNAPSHOT, "demand_entry"), site(SNAPSHOT, "demand_scope"), site(SNAPSHOT, "demand_editor_scope"), site(SNAPSHOT, "demand_bus_graph"), site(SNAPSHOT, "demand_ownership_graph"), site(SNAPSHOT, "demand_handlers"), site(SNAPSHOT, "demand_effects"), site(SNAPSHOT, "demand_effect_certificates"), site(SNAPSHOT, "demand_model"), site(SNAPSHOT, "demand_check"), site(SNAPSHOT, "demand_lowering"), site(SNAPSHOT, "from_program"), site(SNAPSHOT, "SnapshotKey")],
        consumers: &[
            consumer_at("check", V_CHECK, "demand_check"),
            consumer_at("the checker's rules (the handler rows: duplicate handlers, `@supervised`; the entry row: rule 1, rule 9, and pinned-in-a-loop through its lowering root; the placement table: the F.31 rule, per instance; the bus graph: rules 7, 9 and 10), demanded before the check runs; the effect rows' purity column for a codec binding, demanded only when one reaches the assertion", SNAPSHOT, "CheckInputs"),
            consumer_at("topology (`--dump-topology`, `--check-topology`, `--check-topology-shape`: one artifact of the snapshot's model)", V_CHECK, "dump_topology_over"),
            consumer_at("the api description (`--dump-api`: the surface the snapshot's sequence generated the binding for)", V_CHECK, "api_surface"),
            consumer_at("the model dump (`--dump-model`: the check's own model when it judged a law)", V_CHECK, "demand_model"),
            consumer_at("the build identity (build, run, replay: the model hash and the obs ids from the snapshot's model, the plan digest from its lowering view)", OPTIONS, "model_identity"),
            consumer_at("build", V_BUILD, "demand_lowering"),
            consumer_at("run <file> and run <dir>", V_RUN, "demand_lowering"),
            consumer_at("test (a build config for the host, the dev profile)", V_TEST, "demand_lowering"),
            consumer_at("replay (the identity admitted before lowering)", V_REPLAY, "demand_lowering"),
            consumer_at("bench (the driver an overlay on the bench file)", V_BENCH, "demand_lowering"),
            consumer_at("the test harness (`Snapshot::from_program`, lowering not gated on a check)", CG, "demand_lowering"),
            consumer_at("lsp (diagnostics: one snapshot per document event)", LSP, "demand_check"),
            consumer_at("lsp (every request loads the snapshot the diagnostics load, `editor_snapshot`, one per request)", LSP, "editor_snapshot"),
            consumer_at("lsp (definition, placement, the allocation survey)", LSP, "demand_scope"),
            consumer_at("lsp (completion, hover, references, enforcement)", LSP, "demand_editor_scope"),
            consumer_at("lsp (hale/busGraph)", LSP, "demand_bus_graph"),
            consumer_at("model (the effect rows, demanded with the graphs it reads)", SNAPSHOT, "demand_effects"),
            consumer_at("the effects manifest (`--dump-effects-manifest`, `--check-effects-manifest`)", V_CHECK, "demand_effects"),
            consumer_at("replay (the live-effects gate)", V_REPLAY, "demand_effects"),
            consumer_at("the check's laws (their certificate evidence reads the typing's effects certificate report)", SNAPSHOT, "demand_effect_certificates"),
            consumer_at("topology (the artifact's law evidence reads the same report)", V_CHECK, "demand_effect_certificates"),
            consumer_at("lsp (documentSymbol: the open file's member program, its own declarations as written)", LSP, "snap.member("),
        ],
        invariants: &[
            "every editor request reads the snapshot: the outline reads the open file's member program (`Snapshot::member`, the file as it parsed, before the merge and the sequence), so it answers while another member does not parse or an import does not resolve (the editor's load keeps the members of a seed whose link it refused, every family blocked) and lists only what the file itself declares; a file that does not parse has no outline",
            "a prerequisite runs once: every family is a `OnceCell` of its snapshot, and a family that reads another demands it rather than building its own; `Snapshot::builds` counts each family's demands on the snapshot (its producer's runs in the snapshot's own cell), and no count exceeds one on any consumer on the snapshot. It does not count what is rebuilt outside those cells: the lowering view's `resolve_rewritten` builds its own top scope, ownership graph, bus graph and handler rows, so on a build path `build_top_scope` runs more often than `builds()` says. Those rebuilds are the legacy rows `check_bundle_opts_scoped` (top_scope), `build_ownership_graph` (ownership) and the resolved program's `build_bus_graph` (bus_graph), and the resolved program's reference in the `handler_rows(` seam (handler_routing)",
            "a family nobody requested is not computed: the no-claims editor path builds no model (GH #476 criterion 1), nor the graphs it reads",
            "the model's inputs are families (2.3): `demand_model` demands the scope, the bus graph, the ownership graph, the handler rows and the effect rows over the checked programs (the `bus_graph`, `ownership`, `handler_routing` and `effects` counts), each once; lowering's graphs are the lowering view's own, over the resolved program, until the check runs over it",
            "the checker's inputs are families (2.3): the typing demands the handler rows and the entry row before the checker runs and hands them in (`CheckInputs`), so a check with a law builds the handler rows once for the checker and the model together; both producers read declarations, not types, so they are total over a program that does not typecheck",
            "a family whose prerequisite reported errors is `Blocked { family, because }`, not computed: an editor seed with a member that did not parse or would not read has no scope (the editor's requests read `demand_editor_scope`, the scope over the members that parsed with the hole named, and never a scope of their own), a program that does not typecheck has no model, a program whose check reported an error has no lowering view; a ready result may still hold typed holes",
            "the lowering view (`LoweringView`, the `lowering_view` count) is a family: `demand_lowering` demands the check, then runs `resolve_rewritten` once over the intra-locus stage (`demand_intra_locus`, the `intra_locus` count, which the check demanded too) with the snapshot's source map, renames and api config; `build_resolved` reads it by reference; every build path (build, run, test, replay, bench) demands it from a `Snapshot::load`, and the test harness from `Snapshot::from_program`, whose config (`Config::harness`) does not gate lowering on the check",
            "a changed entry, load mode, target, config, overlay or source text is a distinct snapshot (`SnapshotKey`, computed after the load from what it read; a bare program is its own load); two snapshots share no result, and a snapshot is dropped on any change (incremental reuse is a later future)",
            "the bundle is a borrowed view (`Snapshot::bundle`), built per call, never stored",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/demand_gate.rs", "crates/hale-frontend/src/snapshot.rs (a_changed_input_is_a_distinct_snapshot_and_shares_no_result, a_file_that_does_not_parse_blocks_the_scope_and_its_dependents, the_editor_scope_covers_the_members_that_parsed_and_names_the_hole, a_member_that_will_not_read_blocks_the_scope, a_program_that_does_not_typecheck_blocks_the_model, a_check_with_errors_blocks_the_lowering_view, the_lowering_view_is_resolved_once_after_the_check)", "crates/hale-lsp/src/lib.rs (a_request_builds_only_the_families_it_reads, a_request_over_a_seed_with_a_hole_answers_from_the_members_that_parsed)"],
        spec: &["spec/decisions.md F.40", "RFC #1212 § phase 2 (demand and readiness)"],
        seams: &[
            Seam { symbol: "Snapshot::load(", allowed: &[(SNAPSHOT, 1), (V_CHECK, 1), (V_BUILD, 1), (V_RUN, 1), (V_TEST, 1), (V_REPLAY, 1), (V_BENCH, 1), (LSP, 1)] },
            Seam { symbol: "Snapshot::from_program(", allowed: &[(CG, 1)] },
        ],
    },
    Family {
        name: "digests",
        layer: Layer::Identity,
        state: State::Migrating,
        kind: Kind::Digest,
        answers: "Every identity a build or an artifact carries, and what each covers: shape_hash, artifact_digest, model_hash, exec_digest, the toolchain and cache keys, source digests, and the snapshot key they were derived under.",
        inputs: &["the model half", "the artifact", "sources", "BuildOptions", "compiler sources", "the snapshot key"],
        producer: Some(site(TOPOLOGY, "model_shape_hash")),
        legacy: &[
            legacy(OPTIONS, "exec_digest", "the replay identity: HALE_TOOLCHAIN_SHA256 + version + options fingerprint + plan digest + sources; its logical source paths fall back to file names; build and run fingerprint `debug` differently, so a build's recording never replays", "one stated coverage, with tests that a covered change moves it"),
            legacy(CLI_BUILD_RS, "toolchain_digest", "the replay identity: `hale_graph::identity::identity_files`, every identity-covered crate (`COVERED_CRATES`, hale-cli among them) and the manifest files (`Cargo.lock`, the ts-shim manifest), walked through the one shared walk", "phase 3, one stated coverage with `exec_digest`: the CLI's `build` verb still owns semantic work (its snapshot's config from the flags, the `[ffi]` pickup, the identity it stamps; see `identity_files`), so the replay identity still walks the compiler sources at build time, the CLI's crate among them (at the phase-2 close)"),
            legacy(STALE, "compute_codegen_src_hash", "the stale-binary hash: codegen.rs, lotus_arena.c and every stdlib .hl seed, walked identically at build and run time through the shared walk", "one identity per snapshot; the stale check reads it"),
            legacy(IRIS_BUILD_RS, "identity_files", "the DNA toolchain cache key's compiler-source half: the replay identity's selection, every identity-covered crate and the manifest files; hale-cli is covered because the cache builds a host through its `build` verb, whose Rust still owns its snapshot's config from the flags, the `[ffi]` pickup and the identity it stamps (the load and the pre-check sequence are hale-frontend's since 2.2b), until that work moves into hale-frontend","the cache key is derived from the snapshot identity"),
            legacy(IRIS_LIB, "toolchain_hash", "the cache key itself (version, compiler sources and manifests, stdlib, embedded iris and DNA trees)", "the cache key is derived from the snapshot identity"),
            legacy(DNA_DIGEST, "EMBEDDED_DIRS", "DNA's embedded-source identity, its own directory list", "one inventory of what each identity covers"),
            legacy(EVIDENCE, "analysis_inputs_digest", "the evidence inputs digest (semantics version, stdlib source, compiler version, renames, the surface registry)", "same"),
            legacy(SNAPSHOT, "b.sources", "per-file FNV digests, set by the snapshot, rooted at hale.toml for every load mode, the editor's and its requests' included", "one source map per snapshot"),
            legacy(M_OBS, "fn digest", "the observed entity-id digest, keyed by (kind, name)", "keyed by snapshot identity"),
        ],
        consumers: &[consumer("replay (admission)"), consumer("topology / fleet (admission)"), consumer("dna (schema 1.19, semantics 2, shape_hash, artifact_digest)"), consumer("the runtime obs header"), consumer("the DNA host cache")],
        invariants: &[
            "external contracts are frozen through extraction: additive and unhashed sections are free; hash and replay identity change only through explicit versioned transitions with an exact diagnostic (#476's rule)",
            "a build's identities read one snapshot (2.3, `model_identity`): the model hash (P26) is the snapshot model's `shape_hash`, read from the model (`project_shape_hash`, the value its artifact stamps, never scraped from a rendered artifact), the obs ids are that model's entities, and the plan digest `exec_digest` frames is its lowering view's plan; beside them the snapshot key (`SnapshotKey`: the entry, the load mode, the target, the config digest, the overlay digest, the digest of the source text read) names the load all three were derived from. The key is snapshot-local: no binary or recording carries it",
            "a semantic producer moving between crates never makes a later edit invisible to cache or replay identity: the replay identity and the cache key fold one selection, every identity-covered crate (the CLI among them until hale-frontend owns its semantic work) and the manifest files; the stale-binary hash is a cheap warning over codegen.rs, the runtime and the stdlib seeds by design",
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
        gist: "bus cycles: a queued cycle warns, an unconditional intra-locus cycle of direct calls is an error",
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
/// message) in hale-syntax, hale-types, hale-model, hale-codegen,
/// hale-frontend, hale-cli and hale-lsp, frozen with a verdict and an invocation count. The
/// fragment is the invocation collapsed to one line, so a multi-line
/// call is seen. A new one, or a changed count, fails registry_guard.rs.
pub const DEBUG_SCANS: &[DebugScan] = &[
    DebugScan { path: BUILD_ENV, fragment: "format!( \"target={:?};cpu={:?};dev={};debug={}\", o.target, o.target_cpu, o.dev_profile, o.", count: 1, verdict: ScanVerdict::Decides { family: "digests" } },
    DebugScan { path: BUILD_ENV, fragment: "format!(\";lto={l:?}\")", count: 1, verdict: ScanVerdict::Decides { family: "digests" } },
    DebugScan { path: "crates/hale-cli/src/verbs/misc.rs", fragment: "println!(\"{:#?}\", prog)", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: CG, fragment: "format!(\"{:?}\", other)", count: 1, verdict: ScanVerdict::Renders },
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
    DebugScan { path: DESUGAR_SEQ, fragment: "format!(\"{:?}\", d)", count: 1, verdict: ScanVerdict::Renders },
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
         `hale-codegen`, `hale-frontend`, `hale-cli` and `hale-lsp`, with the number of invocations that collapse \
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
