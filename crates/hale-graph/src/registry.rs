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
const BINDING_ROWS: &str = "crates/hale-types/src/binding_rows.rs";
const LIFECYCLE: &str = "crates/hale-types/src/lifecycle.rs";
const LIFECYCLE_TRACE: &str = "crates/hale-types/src/lifecycle/trace.rs";
const LIFECYCLE_DERIVE: &str = "crates/hale-types/src/lifecycle/derive.rs";
const LIFECYCLE_PROJECT: &str = "crates/hale-types/src/lifecycle/project.rs";
const LIFECYCLE_SPINE: &str = "crates/hale-types/src/lifecycle/spine.rs";
const PLACEMENT: &str = "crates/hale-types/src/placement.rs";
const ARRANGEMENT: &str = "crates/hale-types/src/arrangement.rs";
const LOWERING_LAWS: &str = "crates/hale-types/src/lowering_laws.rs";
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
const TYPED_BODIES: &str = "crates/hale-types/src/typed_bodies.rs";
const BUILTIN_SIGS: &str = "crates/hale-types/src/builtin_sigs.rs";
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
const CG_BUS_RT: &str = "crates/hale-codegen/src/bus/runtime.rs";
const CG_TYPES: &str = "crates/hale-codegen/src/types/mod.rs";
const CAPABILITY: &str = "crates/hale-types/src/capability.rs";
const CAPABILITY_TRANSPORT: &str = "crates/hale-types/src/capability/transport.rs";
const CAPABILITY_USES: &str = "crates/hale-types/src/capability/uses.rs";
const FRONTEND: &str = "crates/hale-frontend/src/frontend.rs";
const IMPORTS: &str = "crates/hale-frontend/src/imports.rs";
const SNAPSHOT: &str = "crates/hale-frontend/src/snapshot.rs";
const DEPENDENTS: &str = "crates/hale-frontend/src/dependents.rs";
const TYPING_REUSE: &str = "crates/hale-frontend/src/typing_reuse.rs";
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
            "parse reuse (F.40 phase 3, X1, `hale_frontend::parse_cache`): every file a load parses — a seed's own (`parse_files`, the editor's load), an imported library's two parses (through its own effect-class table, then the load's) — is parsed through the provider's cache when it carries one (`SourceProvider::parses`: the LSP's `Overlay::reusing`, one cache per server; the disk carries none). The cache holds the parser's product alone: the program or the diagnostics `parse_source_at_in` gives for the text at base 0, and the effect-class table the parse left, keyed by the path, the exact text and the table the parse started from (#345: the load's one table is an input of each file's parse). Each load recreates the rest: the file's base in its own source map, the product moved there (`hale_syntax::shift::shift_program`, exhaustive over the AST), then its own shaping and mint. Nothing shaped or minted is kept — shaping reads the whole load and its config, identities are snapshot-local — and the snapshot's key, a whole load's, is never a member's. Nothing invalidates an entry (its product is a function of its key); a path keeps its four most recently used entries",
        ],
        missing: Missing::NotApplicable,
        tests: &["crates/hale-cli/tests/imports.rs (diamond_import, three_hop_import, import_library_key)", "crates/hale-cli/tests/source_map.rs", "crates/hale-cli/tests/lsp.rs (lsp_and_check_agree_over_a_seed_that_imports, lsp_reports_an_unreadable_seed_member_as_check_does, lsp_outline_survives_a_link_failure)", "crates/hale-frontend/src/snapshot.rs (a_reused_parse_is_the_parse)", "crates/hale-syntax/src/shift.rs (the oracle over every .hl in the tree)"],
        spec: &["spec/projects.md"],
        owned: &[site("crates/hale-frontend/src/parse_cache.rs", "ParseCache"), site("crates/hale-syntax/src/shift.rs", "shift_program"), site("crates/hale-syntax/src/shift.rs", "shift_item"), site("crates/hale-syntax/src/shift.rs", "erase_positions")],
        // A load parses through `parse_cache::parse_file` (the cache, or
        // the parse it stands for); the imported library's parse through
        // tokens it already lexed is the uncached path's own.
        seams: &[
            Seam { symbol: "parse_source_at_in(", allowed: &[("crates/hale-syntax/src/lib.rs", 2), ("crates/hale-syntax/src/shift.rs", 2), ("crates/hale-frontend/src/parse_cache.rs", 2), ("crates/hale-types/src/effect_classes.rs", 2)] },
            Seam { symbol: "parse_in(", allowed: &[("crates/hale-syntax/src/parser.rs", 2), ("crates/hale-syntax/src/lib.rs", 1), (IMPORTS, 2)] },
        ],
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
            "the topic-reference and intra-locus rewrites are lowering's, on lowering's copy of the program, never the checked one, and each is recorded as a relation: `TopicRewrite` rows (`topic_rewrites`, and `written_topics` on the bus graph's subjects) and `IntraLocusRewrite` rows (`intra_locus`, and `direct_sends`); the intra-locus rewrite is its own stage, `rewrite_intra_locus` (over the sequence's program, its qualified bus subjects already resolved, keeping on the bus a publish into a field the placement table runs off its owner's thread; the snapshot's `intra_locus` count), which the check demands for rule 10 and lowering continues from, so the judgment and the lowering read one rewrite; `resolve_rewritten` runs the topic rewrite, appends the stdlib, mints and derives the tables",
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
            consumer_at("the load (own files, the editor's members, every imported seed after its imports; through the provider's parse cache when it carries one)", FRONTEND, "parse_file"),
            consumer_at("effects (contracts, phase contracts, the declared manifest)", EFFECTS, "EffectClassTable::of("),
            consumer_at("the effect rows (one table per snapshot, carried on the rows: the model's effect-class rows and atoms, and the inferred manifest's class names, read it there)", EFFECT_ROWS, "EffectClassTable::of("),
            consumer_at("effects (causes)", FRONTIER, "EffectClassTable::of("),
            consumer_at("alloc_summary (what `@effects(is: …)` carries)", ALLOC, "EffectClassTable::of("),
            consumer_at("quantitative (user-class budgets)", "crates/hale-types/src/quantitative.rs", "EffectClassTable::of("),
            consumer_at("claims (lowering: class references, undeclared classes)", "crates/hale-types/src/claim_lowering.rs", "EffectClassTable::of("),
            consumer_at("topology (derived effect sets)", TOPOLOGY, "EffectClassTable::of("),
        ],
        invariants: &[
            "one class, one index, per load: every seed is parsed through the load's one table (`parse_source_at_in`, `parser::parse_in`, or a parse the provider's cache made from an equal table, which it keys on: `parse_cache`), so merging seeds renumbers nothing; an imported seed is numbered after the seeds it imports (it is parsed again, through the table, once they are), the order the classes have always been numbered in",
            "one expansion: a composed class's mask, its atoms and whether its definition is cyclic are `EffectClassTable`'s; no analysis walks a definition itself or reads a program's table directly",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/cross_seed_effects.rs", "crates/hale-cli/tests/xseed_user_effects.rs", "crates/hale-types/src/effect_classes.rs (one_table_one_expansion)", "crates/hale-frontend/src/snapshot.rs (a_seed_is_numbered_after_the_seeds_it_imports)"],
        spec: &["spec/verification.md § Default-on & opt-in analyses"],
        seams: &[
            Seam { symbol: "EffectClassTable::of(", allowed: &[("crates/hale-types/src/effect_classes.rs", 1), (EFFECTS, 3), (EFFECT_ROWS, 1), (FRONTIER, 1), (ALLOC, 1), ("crates/hale-types/src/quantitative.rs", 1),("crates/hale-types/src/claim_lowering.rs", 1), (TOPOLOGY, 1)] },
            // a program's class definitions are read by the parser that
            // writes them, the load that carries them, and the table
            // (`resolved.rs` only builds an empty stdlib program; the
            // shifter names the field in its exhaustive pattern and moves
            // nothing in it)
            Seam { symbol: "effect_defs", allowed: &[("crates/hale-syntax/src/ast.rs", 1), (PARSER, 9), (FRONTEND, 3), ("crates/hale-types/src/effect_classes.rs", 2), (TY_RESOLVED, 1), ("crates/hale-syntax/src/shift.rs", 1)] },
        ],
    },
    // ---------------------------------------------------- Declarations
    Family {
        name: "unit_catalogue",
        layer: Layer::Declarations,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Exact rational relationships between resolved unit identities, cycle consistency witnesses, and coarsest widening-compatible denominations. This is a compiler API; source declarations and expression typing do not demand it yet.",
        inputs: &["unit declaration SiteIds", "equations with declaration SiteIds and positive exact rational factors"],
        producer: Some(site("crates/hale-types/src/unit_graph.rs", "close")),
        legacy: &[],
        consumers: &[
            consumer_at("conversion queries (reduced factor, exactness and equation witness)", "crates/hale-types/src/unit_graph.rs", "conversion"),
            consumer_at("unpinned denomination queries (rational gcd and necessary input witnesses)", "crates/hale-types/src/unit_graph.rs", "meet"),
        ],
        invariants: &[
            "unit identity is the snapshot's SiteId, never a display spelling, span or NodeId equality",
            "factors are positive arbitrary-precision rationals; no machine overflow, rounding, runtime base unit or default loss policy",
            "every cycle has product one; an inconsistent catalogue returns only errors with witnessed cycles, never a partially usable closure",
            "a conversion between disconnected components or through an unknown unit has no answer",
            "denomination is the rational gcd of the input units, potentially unnamed, with an irredundant set of input witnesses; 6, 10 and 15 require three witnesses",
            "a denominator of one proves denomination conversion exactness only, not that a runtime range or representation width can hold the result",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/src/unit_graph.rs"],
        spec: &["spec/units.md"],
        owned: &[],
        seams: &[],
    },
    Family {
        name: "top_scope",
        layer: Layer::Declarations,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "What every top-level name denotes: the symbol table over the merged program.",
        inputs: &["the merged program", "import renames"],
        producer: Some(site(RESOLVE, "build_top_scope")),
        legacy: &[
            legacy(TLIB, "check_bundle_opts_scoped", "`check_program` (the test entry): built here, once, for its checker and the model its laws are judged over. Beside it the model of a bundle no snapshot holds (`derive_application_model`: `claim_law_diags`, the hale-types tests, and the artifact and model-hash entries over a bare bundle; since 2.3 no verb reaches it), the certificate report of such a bundle (`effect_certificates`, for its form rows) and `resolve_program` (the bare program's test entry, once over that program for the scope and the graphs it hands the view) rebuild it; the lowering view reads its snapshot's (F.40 phase 3, C5); every verb and the LSP (its diagnostics and every request) build one per snapshot (`demand_scope`) and pass it to the checker, the model and the model's graphs", "every consumer demands the scope from a snapshot (2.3)"),
        ],
        consumers: &[consumer_at("check", CHECK, "check_bundle_scoped"), consumer_at("check (type expressions: the scope's name table)", CHECK, "&top.names"),consumer_at("demand (every verb, the LSP's diagnostics and its requests: one scope per snapshot)", SNAPSHOT, "build_top_scope"), consumer_at("model (the snapshot's scope, handed in)", MODEL_BUILDER, "ModelInputs"), consumer_at("resolved program (lowering: the snapshot's scope, handed in; its topic rows, and the stdlib's bus rows and typed-body pairs answered over it)", TY_RESOLVED, "top: &TopScope"), consumer_at("resolve_program (the bare program's test entry: once over that program)", TY_RESOLVED, "build_top_scope"), consumer_at("lsp (definition, placement, the allocation survey: the snapshot's scope)", LSP, "demand_scope"), consumer_at("lsp (completion, hover, references, enforcement: the editor's scope, over the members that parsed while one does not)", LSP, "demand_editor_scope")],
        invariants: &[
            "one namespace decision: module-nested declarations and imported seeds resolve the same way everywhere",
            "the editor's scope over a seed with a hole (`demand_editor_scope`) is the same producer over the members that parsed, counted as this family; the whole scope, the check and everything after it stay blocked, so no check runs over a partial program",
            "one name table: the checker resolves every type expression against the scope's own (`TopScope::names`, the declared loci, types and perspectives with each alias's expanded target and the bundle's import renames), built once with the symbols, and keeps none of its own",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/src/lifecycle/derive.rs (shared ancestry regression)", "crates/hale-types/tests/checks_inside_modules.rs", "crates/hale-cli/tests/check_unknown_identifier.rs", "crates/hale-types/tests/type_alias.rs"],
        spec: &["spec/semantics.md"],
        owned: &[],
        seams: &[Seam { symbol: "build_top_scope(", allowed: &[(RESOLVE, 1), (TLIB, 3), (SYNC, 1), (EFFECTS, 1), (TY_RESOLVED, 1), (SNAPSHOT, 1), (LIFECYCLE_DERIVE, 1)] }],
    },
    Family {
        name: "expression_typing",
        layer: Layer::Declarations,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "The type of every expression, and the typed edges (calls, sends, field reads) the locus graph is built from.",
        inputs: &["top_scope", "declarations", "bodies"],
        producer: Some(site(CHECK, "check_bundle_scoped")),
        legacy: &[],
        consumers: &[consumer_at("the snapshot (one typed-body table per snapshot, packaged on demand from the check's record)", SNAPSHOT, "demand_typed_bodies"), consumer_at("the check of a bundle no snapshot holds (the record packaged for the `bare_fallible` law)", CHECK, "check_bundle_reporting"), consumer_at("codegen (an accumulator slot's element type, the closure's typed-body row)", CG, "accumulator_element_type"), consumer_at("codegen (a bare builtin's arity and result: its signature row)", CG, "builtin_sig"), consumer("every layer"), ],
        invariants: &[
            "expression typing is not a layer: it is the derivation inside layer 3 that produces typed edges, and it stays Rust (final direction)",
            "codegen types no value the checker typed: an accumulator's element type is the closure's typed-body row, and a hole is refused at its span",
            "the checker's answers are carried, never re-derived: the check records them as it walks, and one typed-body table per snapshot packages the record (`demand_typed_bodies`, no second check but for a typing that reused a declaration, the snapshot family's X2 row; the check demands it once, for the `bare_fallible` law), keyed by declaration identity (a body by its declaration's site, a call by its `Call` site, a monomorph by its template's site and type arguments, never by a name string), with five columns: accumulator element types, generic calls' type arguments and unified params, the monomorph table, conformance per (locus, interface) pair, fallible calls (the callee's mark and what addresses the call); a site the checker could not type is a hole with its reason",
            "the bare builtins (`len`, `to_string`, the `Int` / `Float` casts, `abs` / `min` / `max`, `starts_with` / `contains`) are typed by one signature table (`BARE_BUILTIN_SIGS`), lowering's inference written down: the checker types a call by its row where lowering lowers it and leaves it `Unknown` where lowering refuses, and lowering reads each builtin's arity and result from the same row",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/typed_bodies.rs", "crates/hale-types/tests/codegen_fixtures_typecheck.rs", "crates/hale-codegen/tests/corpus_check_build_agreement.rs"],
        spec: &["spec/types.md"],
        owned: &[site(RESOLVE, "infer_literal_ty"), site(TYPED_BODIES, "typed_bodies"), site(BUILTIN_SIGS, "BARE_BUILTIN_SIGS")],
        seams: &[Seam { symbol: "typed_bodies(", allowed: &[(TYPED_BODIES, 1), (SNAPSHOT, 1), (CHECK, 1)] }],
    },
    Family {
        name: "generics",
        layer: Layer::Declarations,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which monomorph a generic call instantiates and how its bindings unify.",
        inputs: &["generic declarations", "call arguments", "the mangled token vocabulary"],
        producer: Some(site(CHECK, "unify_generic_ty")),
        legacy: &[],
        consumers: &[consumer_at("check (a mangled monomorph name: the table's row)", CHECK, "resolve_generic_monomorph"), consumer_at("codegen (a generic fn call's type arguments and specialization: the call's typed-body row, and the monomorph table's row for them)", CG, "generic_call_instance"), consumer("codegen")],
        invariants: &[
            "one unification; the monomorph set is a row lowering reads",
            "lowering infers no generic argument: a generic fn call's type arguments are its typed-body row (inside a fn or locus specialization, the owning body's row typed for that monomorph, including params defaults in their declaring locus) and the specialization's name is the monomorph table's; a hole, or a call with no row, is refused at the call",
            "one monomorph table per snapshot (the typed-body table's `monomorphs`), keyed by the template's site and its type arguments, never by a name string: its producer parses a mangled name once, for each name the program spells (a written instantiation as the checker resolves it, an annotation, a struct literal's path), against the bundle's templates by identity; the checker's lookups read the row",
            "a generic call inside a generic fn or locus body is typed again for each of the enclosing template's monomorphs, the template's parameters bound to its arguments, and recorded under them by its source body and call identities; that walk reports nothing and queues concrete instantiations reached in annotations for the same producer to walk",
            "omitted function and method defaults are typed at each invocation in the caller's scope; their generic call rows retain the default's source identity and distinguish the invocation path (nested defaults included) and the caller's specialization, and lowering reads that corresponding row without inference; a supplied argument evaluates no default",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/generic_monomorph_agreement.rs", "crates/hale-types/tests/typed_bodies.rs"],
        spec: &["spec/types.md"],
        owned: &[site(CHECK, "monomorph_table"), site(CHECK, "specialize_generic_bodies"), site(CHECK, "record_specialized_type")],
        seams: &[],
    },
    Family {
        name: "surfaces",
        layer: Layer::Declarations,
        state: State::Canonical,
        kind: Kind::Law,
        answers: "Which surface is visible at which depth edge: contract exposure, interface conformance, perspective designation and `serves` conformance.",
        inputs: &["type, interface, contract, perspective declarations", "locus members"],
        producer: Some(site(CHECK, "conformance_witness")),
        legacy: &[],
        consumers: &[
            consumer_at("check", CHECK, "check_contract_expose_validity"),
            consumer_at("check", CHECK, "check_serves_conformance"),
            consumer_at("check", CHECK, "check_reperspective"),
            consumer_at("check (interface coercion, F.20)", CHECK, "check_structural_impl"),
            consumer_at("check (a bus adapter binding's `__StdBusAdapter` contract)", CHECK, "check_main_and_bindings"),
            consumer_at("the typed-body table (the conformance column: every declared locus and interface pair, every generic locus specialization, the merged stdlib's pairs in the lowering view)", TYPED_BODIES, "typed_bodies"),
            consumer_at("codegen (storage routing: the conformance column)", CG_TYPES, "locus_satisfies_interface"),
            consumer("codegen (vtable swap)"),
        ],
        invariants: &[
            "F.8 compatibility, F.14 and F.20 satisfaction are judged once, with a witness",
            "one conformance function (`conformance_witness`), its witness the first requirement unmet in the interface's method order, rendered by each caller in its own words; the bus adapter's contract is judged without the error channel, as it always was",
            "storage routing asks the conformance column, never the method names, and requires the checker's verdict that the locus satisfies the interface: a locus whose methods match the interface's by name and not by signature, or a generic locus's specialization, is no interface's, so its literal stays the frame's (a classified correction: the name comparison sent both to the program-lifetime payload arena; pinned in `conformance_routing_correction.rs`)",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/perspective_serves.rs", "crates/hale-types/tests/duplicate_member.rs", "crates/hale-types/tests/typed_bodies.rs", "crates/hale-codegen/tests/conformance_routing_correction.rs"],
        spec: &["spec/types.md", "spec/semantics.md"],
        owned: &[site(CHECK, "conformance")],
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
            legacy(ENTRY, "lowering_root", "the row's provisional column: the `main locus` lowering deploys today (the first `is_main && !__lib_` over the flat declarations, module-nested ones included), the placement table's root and so the deployment plan's (`collect_main_placement` reads the table), and the root every comparison lowering makes against the main locus reads (`is_lowering_root`, through the lowering view); so the placement-safety rules (the F.31 rule, the blocking check and the pinned-in-a-loop rule) guard the threads lowering spawns while a seed whose only `main` is module-nested has no entry; the api binding, `--api`'s entry and an environment's constitutions land on the same root, the one deployed, and the model's `entrypoint` and the editor's placement view name it. Lowering reading the entry instead is a behaviour change for that seed (it deploys it today, `nested_main_transition` case (b)) and waits on a ruling", "L4: lowering reads `entry`, and the column goes"),
            legacy(CG, "in_main", "whether lowering is inside `fn main` is a flag set while main's body is emitted (and cleared around a generic fn and a pinned init lowered from inside it); `return`-from-main's exit and the assertion-failure exit's routing key on it. It is a definition of `fn main`, which the row does not carry", "same"),
            // Readers added after E0 by other steps, listed so the
            // inventory is whole; not switched here.
            legacy(BINDING_ROWS, "binds_on_main", "whether the program's own `main locus` binds a transport (E2): a row of ANY `is_main && !imported` declaration, module-nested ones and a second one included, where lowering's prelude takes the lowering root's entries alone", "reads the rows of `lowering_root`, then of the entry (L4)"),
            legacy(LIFECYCLE_DERIVE, "s.decl.is_main", "a top-level or body construction of any `main locus` declaration takes the deferred main-entry teardown spine (L4-2), keyed on the constructed declaration's keyword rather than on the row's lowering root", "reads `lowering_root` (and the entry once lowering does), L4"),
        ],
        consumers: &[
            consumer_at("check (rule 1's count reads the witness: the seed's own mains, module-nested ones included)", CHECK, "check_main_and_bindings"),
            consumer_at("placement (the table is seeded from the lowering root; the F.31 rule, the form rows' sync inference, the blocking check, the pinned-in-a-loop rule, instance aliasing and the model's arrangement read it)", PLACEMENT, "derive_placement"),
            consumer_at("check (rule 9's closed world is a program with an entry, and its api exemption is the entry's `api:` binding, never an imported `main`'s, which is inert)", CHECK, "check_bus_graph"),
            consumer_at("check (each `main locus`'s own `placement { }` block is validated over the witness, deployed or not: an affinity on no named pool, two for one pool)", CHECK, "check_pool_affinity"),
            consumer_at("bus_graph (the closed world: the entry, or a top-level `fn main`)", BUS_GRAPH, "build_bus_graph"),
            consumer_at("ownership (the closed world: the entry, or a top-level `fn main`, the bus graph's test; the producer hands the walk its row, and the rows of a bundle no snapshot holds, `OwnershipRows::of`, build theirs; the stdlib's rows have none)", OWNERSHIP_GRAPH, "collect_ownership_walk"),
            consumer_at("effects (the async_io placement-implied advisory: the lowering root's placement, the pools lowering spawns)", EFFECTS, "placement_implied_diags"),
            consumer_at("check --matrix (a seed is an entrypoint when its row has an entry; the row is built over the seed's own files, since no import holds the entry, so a seed whose import does not resolve is still counted and its pair reports the import)", V_MATRIX, "seed_entry_kind"),
            consumer_at("--env on check and the build paths (the load refuses an environment for a seed with no entry, after the mint, before the sequence's own refusal)", SNAPSHOT, "demand_entry"),
            consumer("build"),
            consumer("dna"),
            consumer_at("codegen (the deployment plan's root is the placement table's, the row's lowering root)", CG, "collect_main_placement"),
            consumer_at("the lowering view (the snapshot's row, carried to codegen)", SNAPSHOT, "view.entry = Some(entry)"),
            consumer_at("codegen (every comparison against the main locus reads the row's lowering root, found in lowering's program by its site: instantiation's and the cascade's placement overrides, the deferred entry's pool join, the bindings prelude's loss handler, and `root_bindings`, which the shm-ring subjects and the binding codec thunks read)", CG, "is_lowering_root"),
            consumer_at("the api binding (GH #1106: generated from the lowering root's `api:` entry, the `main locus` it joins as a param, never an imported library's or a second one; `--api` puts its entry there, and a seed with no lowering root is refused, saying whether it has no `main locus` or only an imported one)", DESUGAR_SEQ, "desugar_before_check"),
            consumer_at("check (the api entry's knobs and what the api leaves out, over the surface generated from the lowering root)", CHECK, "check_api_binding"),
            consumer_at("check --matrix (the roles an environment maps: `owner` joins them when the lowering root carries an `api:` entry, the binding that is generated)", V_MATRIX, "role_coverage"),
            consumer_at("--env and --matrix (an environment's constitutions are adopted into the lowering root, the seed's own `main locus` the build deploys: the entry, in every seed an environment may be bound to)", SNAPSHOT, "adopt_into_root"),
            consumer_at("model (the header's `entrypoint` names the root the arrangement is rooted at, the placement table's, so the lowering root; `main` when there is none)", MODEL_BUILDER, "derive_application_model_over"),
            consumer_at("lsp (`hale/placement` shows the lowering root's params and placement block, a module-nested root included, never an imported library's `main locus`)", LSP, "placement_of"),
            consumer_at("hale dna init (the seed's entry and its file, by the row over the seed's own files that parse, as `seed_entry_kind` reads it: no import is resolved, and none holds the entry)", CLI_DNA, "main_of"),
            consumer_at("claims (the world tier: the row's `world()`, every `main locus` the bundle declares, whose inline claims are the world law, an imported application's included, GH #733; a bundle that closes a world refuses a top-level `claims` block of the closing seed's own)", CLAIMS, "enumerate_clauses"),
            consumer_at("the cross-seed rename (GH #774: an imported seed closes a world, so its constitutions' vocabulary is its own, when the row over its own files, none of them renamed yet, has an entry)", TY_MANGLE, "seed_declares_main"),
        ],
        invariants: &[
            "the checker builds no row: the snapshot demands it before the check and hands it in (`CheckInputs::entry`); a bundle no snapshot holds (the test entries) builds it once, the form rows read the snapshot's (`demand_forms`), and the check matrix builds one over each seed's own files (no import holds the entry, and a seed whose import does not resolve is still an entrypoint)",
            "what lands in the lowering root before the mint (an environment's constitutions, `--api`'s entry, the generated api binding) finds it by the producer's row over the programs as they stand (`entry_row_in`, each program named by its index), and a reader handed programs and no bundle (the roles `--matrix` maps) reads it there; an injection adds members and trailing top-level items, never a `main locus` or a module, so the snapshot's row names the same declaration after the mint",
            "one row per snapshot (`Snapshot::demand_entry`, the `entrypoint` count): the entry by its minted site, and every `main locus` the bundle declares as the witness, each with whether it is imported and whether it is module-nested; it reads declarations only, so no diagnostic blocks it, and a seed with a hole (no identities) blocks it with its scope",
            "an imported `main` is not the entry (decision 1, E0): a library's `main locus` is a declaration the importing seed does not run, and the entry is the importing seed's own; a seed whose only `main` is imported has no entry (`NoEntry::OnlyImported`). Imported is one definition, the rename pass's mark (`imported`, GH #1104 piece 5); the `__lib_` name the same pass gives is spelling, not a second test of the entry (the provisional lowering root copies lowering's name test, until L4)",
            "a module-nested `main` is not the entry (decision 2, E0): the entry is a top-level `main locus` of the seed's own files, so rule 9's closed world is the top-level one; a seed whose only `main` is module-nested has no entry (`NoEntry::OnlyModuleNested`), and a seed with both keeps the top-level one. Rule 1 still counts a module-nested `main` (GH #825): the count reads the witness, not the entry",
            "the decisions bind what reads the entry (rule 9's closed world, `--env`, `--matrix`), not the placement-safety rules: until lowering reads the entry (L4) it deploys a module-nested `main` as its root and spawns its pinned threads, so the placement table, which the F.31 rule and the pinned-in-a-loop rule read, is seeded from the row's provisional `lowering_root`, and a seed whose only `main` is module-nested has no entry and its cross-pool call is still refused (the outside review of #1293, finding 1)",
            "with more than one candidate (rule 1's error) the entry is the last, as the checker's pool map (since replaced by the placement table) took it before the row; the lowering root is the first non-`__lib_` `main`, nested or not, as lowering takes it",
            "the checker, the api binding and the environment, the model, the editor and the CLI read the row (F.40 phase 3, the entry's consumers), and lowering's comparisons against the main locus read it too (L4); the legacy sites left are the row's own provisional column (`lowering_root`), lowering's `in_main`, a definition of fn main, and two readers added after E0 (the binding rows' `binds_on_main`, the lifecycle's deferred main-entry spine); the row has one definition",
            "lowering derives no main locus (L4): the lowering view carries the row (`LoweringView::entry`), and codegen's comparisons against the main locus read its lowering root by identity (`is_lowering_root`, `root_bindings`), never a type name or the first `is_main && !__lib_`",
            "the world is wider than the entry, on purpose: `EntryRow::world` is every `main locus` the bundle declares (the entry's, a module-nested one's, an imported application's), because decisions 1 and 2 say which declaration the seed runs, and an application's inline law keeps binding when another seed imports it (GH #733, `crates/hale-cli/tests/imported_main_claims.rs`); the claims read the row's column and gather no `is_main` of their own",
            "three readers of the `main` keyword read a property of each declaration and derive no entry fact, so they are no definition of the entry and no legacy row, whichever `main` is the entry: `check_nested_long_running_child` exempts every `main locus`, as a parent (`parent.is_main`) and as a child (`!l.is_main`), from the long-running-child rule, since a root is supervised by no parent; the ownership graph makes every `main locus` declaration a singleton (`singleton |= l.is_main`), one instance per declaration, deployed or not; and the allocation summary defers every `main locus` declaration, module-nested ones included, from eager reclamation (`if l.is_main` in its flat walk), conservatively. Each expression is a seam, so a second reader of the keyword in those files is a new row to classify",
            "every non-test reader of `LocusDecl::is_main` is the producer, a legacy row here, or one of the three per-declaration readers above; the binding rows copy the keyword into each row as a column (`is_main: l.is_main`), the parser's main-only member rules are syntax and are not rows, and `sync_inference`'s test helper is test code",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-frontend/src/snapshot.rs (the_entry_row_is_the_seeds_own_top_level_main_locus)", "crates/hale-cli/tests/check_entry_decisions.rs", "crates/hale-cli/tests/check_entry_consumers.rs (each consumer's correction on the decided shapes, beside a control)", "crates/hale-lsp/src/lib.rs (the_placement_view_is_the_deployed_roots)","crates/hale-cli/tests/nested_main_transition.rs (the nested-main transition end to end: refused, and deployed, as a top-level main)", "crates/hale-cli/tests/entry_point_placement.rs", "crates/hale-types/tests/bus_graph.rs"],
        spec: &["spec/semantics.md § Bundle-wide rules"],
        owned: &[],
        seams: &[
            Seam { symbol: "entry_row(", allowed: &[(ENTRY, 2), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2), (EFFECTS, 1), (V_MATRIX, 1), (SYNC, 1), (PLACEMENT, 1), (TY_RESOLVED, 1), (OWNERSHIP_GRAPH, 1)] },
            Seam { symbol: "is_lowering_root(", allowed: &[(CG, 2), (CG_INST, 1), (CG_DISSOLVE, 2)] },
            // The per-declaration readers of the `main` keyword (the
            // invariant above): each where it is, once.
            Seam { symbol: "world()", allowed: &[(CLAIMS, 1)] },
            Seam { symbol: "parent.is_main", allowed: &[(CHECK, 1)] },
            Seam { symbol: "!l.is_main)", allowed: &[(CHECK, 1)] },
            Seam { symbol: "singleton |= l.is_main", allowed: &[(OWNERSHIP_GRAPH, 1)] },
            Seam { symbol: "if l.is_main {", allowed: &[(ALLOC, 1)] },
        ],
    },
    Family {
        name: "ownership",
        layer: Layer::Locus,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Who owns each locus-producing expression and each instance: the tower, with its two relations `accepts_ancestor` and `owner_of_site`; and, per binding site, whether its value is handed back, moved by `=`, or a frame-local array.",
        inputs: &["locus declarations (params, accept, release)", "bodies (let, assign, return, field initialisers, placement entries)", "fresh factories (one producer)", "returned bindings", "placement (the table: each bubbling edge's class, per instance)"],
        producer: Some(site(TY_OWN, "resolve_owners")),
        legacy: &[],
        consumers: &[consumer_at("codegen", CG_INST, "site_owner"), consumer_at("codegen (a monomorph's accept rows: its template's, specialized at synthesis)", CG, "specialized_accepts"), consumer_at("codegen (whether the enclosing locus accepts the child it births)", CG_INST, "parent_accepts_us"), consumer_at("borrow_lifetime (a bare literal's enclosing locus accepts its child: `accepts_ancestor`, over the snapshot's graph's rows)", "crates/hale-types/src/borrow_lifetime.rs", "accepts_ancestor"), consumer_at("lowering view (its graph: the snapshot's rows through the correspondence, then the stdlib's; the bubble plans, `accepts` and the accept rows lowering reads)", TY_RESOLVED, "lowering_ownership_graph"), consumer_at("model (dynamic births: the snapshot's graph)", SNAPSHOT, "demand_ownership_graph"), consumer_at("a declaration's dependents (X2: a locus's births, accepts and instantiations make its neighbours through the ownership graph, the snapshot's graph)", SNAPSHOT, "declaration_dependents"), consumer_at("check (type-check rule 20, the unowned-subscriber rule: `owner_of_site` over the snapshot's graph, handed in through `CheckInputs`)", CHECK, "check_unowned_subscriber_locus"), consumer("alloc_summary (eager-only accept sets)"), consumer_at("check and the harness (the cross-pool spawn law reads the bubble plan)", LOWERING_LAWS, "cross_pool_spawn_used_as_a_value")],
        invariants: &[
            "a locus instantiation with no row is a CodegenError (F.39)",
            "ids, not names or spans: declarations are cloned and the stdlib's coordinates overlap user files",
            "the ownership matrix stays green with an empty KNOWN_OPEN",
            "the model's `Owns` edges are the placement table's `owner` column projected with the arrangement (P1 4 of 6, C3): an arranged instance is owned by the arranged instance its row names as owner; `owns.push(` has that one writer",
            "the model reads construction context from the ownership graph (C3): the shared walk records body births, field defaults and binding adapters with their explicit fields and literal identities. `unarranged_births` follows the defaults each construction actually evaluates; the same default can be arranged for one holder and dynamic for another, while an overridden default adds no birth. The arrangement projection (`project_arrangement`, which the model's rows are made from) supplies the literal identities its placement rows represent, including held sources, and joins each remaining birth by resolved declaration site (name fallback only for unminted bundles). No model-side params-span test or free-function walk remains; copied API-binding expressions retain their construction context despite overlapping spans",
            "the graph keeps each `accept` param as a row of the declaring locus's identity, its type as written (`AcceptRows`); a monomorph's accept set is its template's rows, asked for by the template's identity and specialized by the instantiation's substitution (`AcceptRows::specialize`, as `FlowRows::specialize` does for release clauses), filled at synthesis; lowering never reads a monomorph's own `accept_param` for ownership",
            "`fresh_factories` is read by lowering and the checker with the bundle's import renames; a factory's returned name is the declaration the snapshot resolves it to, so a fn whose returned name an inner `let` shadows is a factory of the outer binding (the #1140 shape; its escape walk still reads every binding spelling the name as the returned one, the conservative side). The carrier-arm extension (`extend_fresh_factories`) is one set both read: lowering through the owner table, and the checker's self-containment law through `extended_factory_rows`, which gives each fn the fold adds a row whose products are its arms' (F.40 phase 3, C5: the decision measured no diagnostic moving over the corpus, `tests/hale` and the DNA seeds; the fold's one addition there, in the DNA core, already had a row)",
            "which declaration a returned or escaping name denotes is read from the snapshot (`Snapshot::declaration_of` over `binding_of`, resolved once by the mint), never resolved again: `returned_bindings` (the binding facts and the pre-pass), `fresh_factories`, borrow_lifetime's `returned_decls` and alloc_summary's escape tags each key a binding by its declaration's SiteId; a `let` or a use the snapshot did not mint answers by name in `returned_bindings` (the conservative side) and resolves to nothing elsewhere, and every entry point mints",
            "the checker builds no graph: the snapshot demands it before the check and hands it in (`CheckInputs::ownership`), so a check with a law builds it once for the checker and the model together; a bundle no snapshot holds (the test entries) builds it once, through the producer (`build_ownership_graph`), and the model over that bundle reads the same build",
            "one graph per snapshot (F.40 phase 3, C5): the graph keeps the rows it is assembled from (`OwnershipRows`), and lowering's graph is the snapshot's rows, each site and accepting locus found in the merged program through the view's correspondence, followed by the stdlib's rows over the merged program's tail (`stdlib_ownership_rows`), the one part no snapshot holds; one procedure assembles either (`lowering_ownership_graph`)",
            "the tower's accept relation is one relation, `accepts_ancestor` (`OwnershipRows::accepts_ancestor`): the climb finds an owner by it, and the borrow-lifetime law asks it of a bare literal's enclosing locus, over the snapshot's graph's rows (a bundle no snapshot holds walks its own, `OwnershipRows::of`)",
            "rule 20 is judged by declaration identity (`owner_of_site`): the graph lists every locus declaration in declaration order, and a site carries the declaration its literal names (`child_decl`, joined by the shared resolver's declaration identity, with a name fallback for unminted input) and the child an `accept` must name to own it (`child_key`: an import or `std::` path resolved as an `accept`'s type is, a template specialized by its binding's or field's declared type); the nearest accepting ancestor owns a birth in a handler as it owns any other, but only if one accepts the child on every construction path of the enclosing locus (`construction_paths`, over the placement table: each instance row under its owner row's declaration, a template's top as a root with no ancestor, a literal directly in `fn main` included, each dynamic site under its enclosing locus or in a free fn whose callers are not followed; with the walk's own body and params-default edges, since the table does not enumerate a dynamically built locus's params subtree), and the diagnostic names a path with none. `child_ty`, `resolution` and bubble plans use the resolved child name, following imports and aliases and retaining a known generic specialization; plain records and unresolved paths are not births. Lowering's bundle carries its minted snapshot, so the declaration join keeps its identity. Lowering's `instantiated_by` still records no free-fn birth",
            "the holes: where the graph cannot decide, `owner_of_site` answers None and rule 20 does not fire, since unknown ownership is not proven absence: an open world (no entry, so a consumer may complete the tower), a generic template no declared type specializes, a qualified path that names no declaration, and an unanalyzable site; a literal assigned to a field of `self` is judged by the `accept` edges, since the graph records no assignment target. A construction path the placement table records as a hole cuts the other way, since an unknown path cannot prove an owner: a held instance the table does not link, a field whose initializer is no literal, a dynamic site of unknown domain and an owner row whose declaration does not resolve each count as a path with no ancestor, and so does a free fn; a held row the table links adds no path (its source row is the construction)",
            "a bubbling edge's class reads the placement table per instance: each row of the enclosing locus is paired with the row that owns it, every other instance with every domain of the owner; SameTower when every pair shares its domain, CrossPool when none does and the owner has one, Mixed otherwise or where an instance runs where the table cannot say. The graph reads no `placement { }` block itself",
            "the owner is a fact of the site, the mechanism of the instance (U-1): a Mixed edge keeps its resolved owner, takes the same-tower birth or the cross-pool post per enclosing instance where the owner is a singleton on main (`lotus_on_main_thread` at the literal), and is refused at the literal (`CodegenError::UnsupportedAt`, naming every instance and its domain) for a value use or a non-singleton owner; it is never lowered as a transient birth",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/model_arrangement.rs", "crates/hale-codegen/tests/owner_table.rs", "crates/hale-codegen/tests/ownership_matrix.rs", "crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding)", "crates/hale-codegen/tests/ownership_bubble.rs", "crates/hale-cli/tests/check_unowned_subscriber.rs (rule 20, one test per class of the migration)", "crates/hale-types/tests/ownership_graph.rs (lowerings_graph_is_the_snapshots_rows_through_the_correspondence)", "crates/hale-types/tests/self_containing_locus.rs (a_factory_returning_a_carrier_is_reported, a_carrier_with_an_accessor_arm_is_not_a_cycle: the checker reads the extended factory set)", "crates/hale-cli/tests/check_borrow_lifetime.rs (the law over `accepts_ancestor`)", "crates/hale-types/tests/ownership_graph.rs (the O rows: nested under pinned, on the owner's pool, per-instance pairing, adapter, mixed)", "crates/hale-codegen/tests/ownership_bubble_mixed.rs (U-1: both instances' owner, count, retention, birth thread and teardown, both arms and ASan; the transient control; the located refusal)", "crates/hale-codegen/tests/ownership_bubble_crosspool.rs (O-1 under ASan)"],
        spec: &["spec/decisions.md F.39", "spec/semantics.md § Dissolve timing rules", "spec/semantics.md § Placement block (type-check rule 20)", "spec/semantics.md § Locus instantiation (accept bubbling: the owner per site, the delivery per instance)", "spec/runtime.md § Interest-based ownership (accept bubbling)"],
        owned: &[site(TY_OWN, "resolve_binding_facts"), site(OWNERSHIP_GRAPH, "bubble_plans"), site(OWNERSHIP_GRAPH, "compute_forwarding_sets"), site(OWNERSHIP_GRAPH, "classify_owner_kind"), site(OWNERSHIP_GRAPH, "classify_edge"), site(TY_OWN, "fresh_factories")],
        seams: &[
            Seam { symbol: "resolve_owners(", allowed: &[(TY_RESOLVED, 1), (TY_OWN, 1)] },
            Seam { symbol: "build_ownership_graph(", allowed: &[(OWNERSHIP_GRAPH, 1), (SNAPSHOT, 1), (TLIB, 2), (CHECK, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "stdlib_ownership_rows(", allowed: &[(OWNERSHIP_GRAPH, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "lowering_ownership_graph(", allowed: &[(OWNERSHIP_GRAPH, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "accepts_ancestor(", allowed: &[(OWNERSHIP_GRAPH, 3), ("crates/hale-types/src/borrow_lifetime.rs", 1)] },
            Seam { symbol: "fresh_factories(", allowed: &[(TY_RESOLVED, 1), (TY_OWN, 2)] },
            Seam { symbol: "extended_factory_rows(", allowed: &[(TY_OWN, 1), (LOWERING_LAWS, 1)] },
            Seam { symbol: "extend_fresh_factories(", allowed: &[(TY_OWN, 3)] },
            Seam { symbol: "resolve_binding_facts(", allowed: &[(TY_OWN, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "returned_bindings(", allowed: &[(TY_OWN, 3)] },
            Seam { symbol: "bubble_plans(", allowed: &[(OWNERSHIP_GRAPH, 1), (TY_RESOLVED, 1), (LOWERING_LAWS, 1)] },
            Seam { symbol: "owns.push(", allowed: &[(MODEL_BUILDER, 1)] },
            Seam { symbol: "owner_of_site(", allowed: &[(CHECK, 1)] },
            Seam { symbol: "demand_ownership_graph(", allowed: &[(SNAPSHOT, 7), (V_CHECK, 1)] },
        ],
    },
    Family {
        name: "bus_graph",
        layer: Layer::Locus,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "The message graph: subjects, publishers, subscribers, handlers, and the per-subject devirtualization gates.",
        inputs: &["topics", "bus blocks", "sends", "bindings", "placement (the table: every label, and so the direct-call gate)"],
        producer: Some(site(BUS_GRAPH, "build_bus_graph")),
        legacy: &[],
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
            consumer_at("lowering view (its graph: the snapshot's rows through the correspondence, each user site keyed by the topic rewrite's wire, then the stdlib's; the plan lowering reads is its gates)", TY_RESOLVED, "lowering_bus_graph"),
            consumer_at("the no-snapshot entries (`check_bundle`, `check_bundle_opts_scoped`, `claim_law_diags`, the hale-types tests, the artifact's bundle entry, and `resolve_program` for a bare program): the producer itself, once per entry, over the entry's scope", TLIB, "build_bus_graph"),
        ],
        invariants: &[
            "one graph, over one program shape, per snapshot; rule 10's cycle graph is a query over it (`cycle_from`)",
            "lowering derives no graph of the user's program: its graph is the snapshot's rows (`BusRows`), each user site found in the merged program through the view's correspondence and keyed by the wire the topic rewrite gave it, followed by the stdlib's rows over the merged program's tail (`stdlib_bus_rows`), the one part no snapshot holds; the subjects and their gates are assembled from the rows by one procedure (`BusRows::subjects`) on both sides",
            "a bundle no snapshot holds builds its graph through the snapshot's producer (`build_bus_graph`), never through a wrapper of its own",
            "the checker's bus rules (7, 9, 10) compare subjects under the canonical key, the wire subject (`Subject`, `wires`): a topic published by name and subscribed by its literal subject is one subject; the gates and the model keep `BusSubject::canonical()`'s keys (`subjects`)",
            "an edge belongs to the locus declaration that wrote its handler (`BusEdge::decl`), never to a name: two loci of one name have their own edges",
            "a subject the graph cannot resolve is a hole (`holes`): it forms no edge and no rule calls it an orphan",
            "the intra-locus rewrite is a relation on the graph, never an erased publisher (boundary 7)",
            "lowering reads the relation (`LoweringView::intra_locus`) by the call's id, which the call that replaces a send keeps (`IntraLocusRewrite::send`); it never classifies a call as a rewritten publish from the call's shape",
            "rule 10 calls a cycle synchronous only where the relation holds every send of it (`BusEdge::send`, the snapshot's `intra_locus` stage, the one lowering continues from); a same-declaration cycle with a send the relation does not hold is carried by the queue, and the subjects' spelling never decides which",
            "a placement label is the placement table's answer for the type, by the name lowering keys on: `SameThread` only when every instance runs on main (a nested instance inherits its owner's domain, an adapter is pinned, an instance the table cannot place is `Unknown` and never main); the graph reads no `placement { }` block itself",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/bus_graph.rs (lowerings_graph_is_the_snapshots_rows_through_the_correspondence)", "crates/hale-types/tests/lowering_correspondence.rs", "crates/hale-types/tests/bus_graph.rs (the B rows: nested, imported root, qualified field, last-segment collision, two domains, adapter)", "crates/hale-types/tests/bus_rules_over_graph.rs", "crates/hale-types/tests/bus_payload_handler.rs", "crates/hale-codegen/tests/bus_devirt_differential.rs", "crates/hale-codegen/tests/nested_offthread_delivery.rs (the dispatch-plan flavor of a nested subscriber, a nested publisher and an adapter's publication)"],
        spec: &["spec/semantics.md rules 7, 9-12, 19", "spec/verification.md § Bus-graph property checks"],
        owned: &[site(BUS_GRAPH, "dispatch_gates"), site(BUS_GRAPH, "cycle_from"), site(BUS_GRAPH, "external_handlers")],
        seams: &[
            Seam { symbol: "build_bus_graph(", allowed: &[(BUS_GRAPH, 1), (SNAPSHOT, 1), (TLIB, 2), (CHECK, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "collect_bus_walk(", allowed: &[(BUS_GRAPH, 2)] },
            Seam { symbol: "stdlib_bus_rows(", allowed: &[(BUS_GRAPH, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "lowering_bus_graph(", allowed: &[(BUS_GRAPH, 1), (TY_RESOLVED, 1)] },
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
            Seam { symbol: "topic_wire_subjects(", allowed: &[(TOPIC_ID, 2), (TY_RESOLVED, 1)] },
            Seam { symbol: "TopicRows::of(", allowed: &[(TOPIC_ID, 1), (RESOLVE, 1)] },
            Seam { symbol: "by_wire(", allowed: &[(TOPIC_ID, 6), (CHECK, 2)] },
        ],
    },
    Family {
        name: "bindings",
        layer: Layer::Locus,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which topics are bound to which transport, in which role, with which codec, and whether the transport can carry the payload.",
        inputs: &["bindings blocks", "topics", "transport specs", "purity (codecs)"],
        producer: Some(site(BINDING_ROWS, "derive_binding_rows")),
        legacy: &[],
        consumers: &[
            consumer_at("check (the binding rules walk the rows: topic, duplicate, role, adapter, ring layout, constraints, codec; the `or wait` legality check and the api gates read the bound-topic set)", CHECK, "check_main_and_bindings"),
            consumer_at("check (a binding's `where` constraints are held to its transport's guarantee: the capability module's table, read through the row's transport kind)", CAPABILITY_TRANSPORT, "guarantee"),
            consumer_at("the transport's cell on the effective target: `RemoteTransport(kind)` × the target row's backend, read through the snapshot (verdict-neutral: the adapter's wasm refusal is a late link refusal today, a known-open cell)", SNAPSHOT, "binding_cell"),
            consumer_at("model (main's binding thread domains: the role and the transport kind)", MODEL_BUILDER, "ModelInputs"),
            consumer_at("bus graph (the bound-topic set, at both grains, is the rows' projection)", BUS_GRAPH, "collect_bus_walk"),
            consumer_at("codegen (the prelude, the shm-ring subjects, the codec thunks and the pinned adapter loci read each entry's transport kind, role, codec and producer-versus-attach from its row, through the lowering view; the entry's own text supplies the transport's parameters; a missing row is an error)", CG, "root_bindings"),
            consumer_at("codegen (whether a thread crosses the bus boundary: an entry of the program's own main, `binds_on_main`, beside the placement table's domains)", CG, "program_has_offthread"),
            consumer("api_surface"),
        ],
        invariants: &[
            "the transport-loss handler is named by the row: a `unix` entry's row carries the stdlib locus its transport instantiates (`loss_locus`, by role), lowering instantiates that locus, and a connect entry's is the locus whose failure the main locus's `on_failure` handles, so the handler is main's routing row for the locus the bindings row names, not one picked by a spelled name",
            "lowering holds the rows (`LoweringView::bindings`, the snapshot's) and finds an entry's by the id the mint kept (`BindingRows::for_entry`); it decides no transport, role, codec or producer-versus-attach itself, and an entry with no row is a `CodegenError`, not a guess",
            "F.36 and F.37: binding failure is structural; codec purity is a law over rows",
            "one row per snapshot (`Snapshot::demand_bindings`, the `bindings` count): one row per `bindings { }` entry of every locus of the bundle, an imported main's and a module-nested one's included, each with the entry's site, the topic and its wire key, the transport kind, the role, the codec, whether the bundle produces the topic and the stdlib locus a transport's loss surfaces through; the checker builds none (`CheckInputs::bindings`), and a bundle no snapshot holds builds it once",
            "the role is decided once, over the topic's ends read by wire subject (`desugar::role_from_ends` over the row's `publishes` and `subscribes`): the entry's own role wins, otherwise publish-only is `Connect` and subscribe-only is `Listen`, and a `unix` entry with neither is the checker's diagnostic. The checker, the model and lowering read it; the desugar's in-place fill applies the same pure rule over the topic names before the topic rewrite erases them, and agrees with it over the corpus",
            "what a transport carries is data beside the matrix, not a branch in the checker: the transport kind's guarantee for each `where` constraint (`capability::transport::GUARANTEES`, three rows of four cells, the former `transport_satisfies` cell for cell, its words verbatim), which does not vary by target; and whether a target realizes the transport at all is the matrix's own `RemoteTransport(kind)` row, asked through the snapshot's target row (`Snapshot::binding_cell`)",
            "the bound-topic set is the rows' projection (`bound_names`, `bound_subjects`): the `or wait` legality check, the api gates and the bus graph's eligibility gate read it, and none walks `bindings { }` itself. An imported main's entries are in the set, as they were in each of the walks it replaces",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/binding_imported_main.rs", "crates/hale-codegen/tests/bindings_codec_clause.rs"],
        spec: &["spec/decisions.md F.36, F.37", "spec/semantics.md § Operational constraints (Form K)"],
        owned: &[],
        seams: &[
            Seam { symbol: "derive_binding_rows(", allowed: &[(BINDING_ROWS, 1), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2)] },
            Seam { symbol: "role_from_ends(", allowed: &[(DESUGAR, 2), (BINDING_ROWS, 1)] },
        ],
    },
    Family {
        name: "dispatch",
        layer: Layer::Locus,
        state: State::Migrating,
        kind: Kind::Derivation,
        answers: "How each bus subject dispatches: dynamic, static bucket or static direct, given its gates and the arrangement.",
        inputs: &["bus_graph (gates, the payload_flat column among them)", "placement (domains: the arrangement projection, `project_arrangement`, with the ownership graph's births outside it)", "--no-bus-devirt"],
        producer: Some(site(M_DISPATCH, "fn derive")),
        legacy: &[
            legacy(TY_RESOLVED, "from_gates", "the plan is derived twice, from two gate sets: the model's (`derive`) from the checked graph's gates, lowering's (the resolved program) from the lowering graph's, which are the same rows re-keyed by wire plus the stdlib's; by one function (`from_gates`) with one domain map (`domain_map` over the arrangement projection, keyed by the gates' spelling of a locus); held equal on the subjects the model's gates name by the law (`dispatch_plan_law.rs`: every column, the subscriber column over the model's loci and in each plan's own order)", "phase 4, with `stdlib_surface`: one derivation needs the stdlib's rows at the snapshot, so that the model's gates and lowering's are one set (F.40 phase 3, C5 2 of 2, restated)"),
        ],
        consumers: &[consumer_at("codegen", CG, "build_resolved"), consumer_at("codegen", "crates/hale-codegen/src/bus/dispatch.rs", "bus_devirt"), consumer_at("exec_digest (the resolved program's plan)", OPTIONS, "resolved.plan.digest()"), consumer("model dump")],
        invariants: &[
            "which flavour a subject gets is a plan conclusion, never a model row (spec/model.md)",
            "lowering reads one plan, derived once per snapshot in the resolved program; the execution digest frames that plan's digest, which covers what lowering reads (each subject, its flavor and its subscribers) and keeps a reserved 0 byte where `same_domain` sat, until the same-domain flavors (GH #464) lower by it",
            "both plans take their domains from one function, `domain_map`: the arranged (locus, domain) pairs minus every locus the arrangement does not fully place (a template disagreement or a birth outside it), keyed by the gates' spelling of a locus, the raw post-merge symbol, never a display name; the model feeds it its arrangement rows and placement holes, lowering the projection (`project_arrangement`) those rows are made of, over the snapshot's programs, table and ownership graph, so lowering demands no model",
            "the model's plan is lowering's on every subject the model's gates name, column for column, the domain lists and `same_domain` included; the subscriber column is compared over the model's loci and in no order (the model's gates hold subscribers sorted, lowering's in registration order; a subject the stdlib's rows also subscribe carries their loci in lowering's row only), over the corpus examples, tests/hale and the DNA mains, build and harness snapshots",
            "the direct tier takes all three gate legs, same-thread, quiet and the payload_flat column (`bus_graph::payload_is_flat`, codegen's flatness rule over resolved types); codegen reads the flavor and refuses a plan whose column disagrees with the lowered payload, and the codec's own flatness equals the column at every publish over the corpus",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/dispatch_plan_cli.rs", "crates/hale-codegen/tests/bus_devirt_direct.rs", "crates/hale-cli/tests/dispatch_payload_flat.rs (every wire payload alternative through both publish arms against the codec, the column against the codec at every publish over the corpus, the plan change recorded as a compatibility change: --dump-model's row, a pre-change recording refused by its exec digest and admitted with --allow-unverified-model, a post-change recording replayed)", "crates/hale-types/tests/dispatch_plan_law.rs (the model's plan is lowering's on the shared subjects over 334 views, with a control per column)", "crates/hale-types/tests/dispatch_plan.rs (an_imported_seeds_loci_have_their_domains: a two-seed fixture's imported loci have their domains, same-domain on main)", "crates/hale-model/src/dispatch_plan.rs (same_domain_is_no_part_of_the_digest)"],
        spec: &["spec/model.md § Derived products", "spec/decisions.md F.38", "spec/runtime.md § Placement classes (the dispatch plan)"],
        owned: &[site(M_DISPATCH, "domain_map")],
        seams: &[
            Seam { symbol: "DispatchPlan::derive(", allowed: &[(MODEL_BUILDER, 1)] },
            Seam { symbol: "from_gates(", allowed: &[(M_DISPATCH, 2), (TY_RESOLVED, 1)] },
            // The one domain map: the model's rows in `derive`, the
            // arrangement projection's in `Arrangement::domains`.
            Seam { symbol: "domain_map(", allowed: &[(M_DISPATCH, 1), (ARRANGEMENT, 1)] },
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
            legacy(CG_CHANNELS, "resolve_failure_route", "the parent instance is the lowering context's (supervising parent, then self, then params-init self); the handler is the row's", "phase 3, when the instance is a row of an instance tree: the snapshot has no instance-tree family, so the parent instance is still the lowering context's (at the phase-2 close)"),
        ],
        consumers: &[consumer_at("codegen (the handler table, one fn per row keyed by the row's site)", CG_DECL, "handlers_of_instance"), consumer_at("codegen (handler bodies, by the row's site)", CG_METHOD, "handlers_of_instance"), consumer_at("codegen (concrete handler rows at locus synthesis)", CG, "self.handlers.specialize"), consumer_at("codegen (a route: the row's handler fn)", CG_CHANNELS, "failure_handler_for"), consumer_at("codegen (__parent_on_failure)", CG_CHANNELS, "resolve_failure_route"), consumer_at("codegen (the params-settle bracket: whether the locus has a handler)", CG_INST, "settles_failures"), consumer_at("codegen (restart in place)", CG_RESTART, "restarts_in_place"), consumer_at("model (supervises, over the snapshot's rows: `demand_handlers`)", MODEL_BUILDER, "Supervises"), consumer_at("check (duplicate handlers, over the snapshot's rows handed in: `CheckInputs`)", CHECK, "check_duplicate_failure_handlers"), consumer_at("check (@supervised, over the same rows)", FRONTIER, "supervised_diags"), consumer_at("ownership births (resolved child, declaring template and identity)", OWNERSHIP_GRAPH, "identify_child")],
        invariants: &[
            "the child type is resolved once, by `child_locus_name`; lowering, the checker and the model read the same row",
            "a row carries its parent declaration's site (`parent_id`) and a reader holding a locus declaration asks for its rows by that identity (`handlers_of_decl`, `route_decl`): a monomorph keeps its template's id and selects concrete rows by that identity and its specialization name (`handlers_of_instance`, `route_instance`); lowering's handler fn is a column of the row, held per locus keyed by the row's site (`LocusInfo::failure_handlers`), and the handler table, the body pass and a route each join by that site, never by the row's ordinal",
            "a row carries the site of the declaration its child resolves to (`child_decl`, a monomorph's template's, from the same resolution as `child`: `child_locus`), qualified by the store that minted it; the model's supervision rows join parent and child to its locus table by those sites, and a child declared outside the snapshot's programs (a stdlib locus) is `SupervisedRef::External`, whose written name is its display, never a join key; only a bundle no entry point minted joins by name",
            "the model's failure-handler function rows are keyed by the handler's site (`handler_fn_rows`), the routing row's identity; the signature string is the row's name, which ranks it among the function rows (two handlers are two rows whatever their signatures spell), and no reader looks a handler up by it",
            "a generic supervisor's child types are substituted at synthesis by the handler producer (`specialize`) using the same substitution as the locus; each concrete row preserves its template handler's site and recovery ops, resolves the concrete child's declaration in the original bundle, and is indexed by template identity and specialization name. Dispatch, handler body layouts and restart-in-place attribution read those concrete rows; the declaration-level snapshot rows are unchanged",
            "the checker builds no rows: the snapshot demands them before the check (`CheckInputs`), and the checker's duplicate-handler rule, the `@supervised` law and the model read that one build; a bundle no snapshot holds (the test entries) builds them once, in `bundle_handler_rows`",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/lifecycle_flow.rs (on_failure_dispatch_by_child_type)", "tests/hale/on_failure_per_child_type_test.hl", "crates/hale-types/tests/violate.rs", "crates/hale-types/tests/handler_routing_probes.rs"],
        spec: &["spec/semantics.md § failure", "spec/runtime.md (failure delivery)"],
        owned: &[site(HANDLER_ROUTING, "child_locus_name"), site(HANDLER_ROUTING, "resolve_locus_type")],
        seams: &[
            Seam { symbol: "handler_rows(", allowed: &[(HANDLER_ROUTING, 1), (TY_RESOLVED, 1), (SNAPSHOT, 1), (TLIB, 1)] },
            Seam { symbol: "child_locus_name(", allowed: &[(HANDLER_ROUTING, 1), (OWNERSHIP_GRAPH, 2), (TY_OWN, 1), ("crates/hale-types/src/flows.rs", 1)] },
            Seam { symbol: "child_locus(", allowed: &[(HANDLER_ROUTING, 3)] },
            Seam { symbol: "resolve_locus_type(", allowed: &[(HANDLER_ROUTING, 3), (OWNERSHIP_GRAPH, 2)] },
            Seam { symbol: "DeclaredNames::of(", allowed: &[(HANDLER_ROUTING, 1), (OWNERSHIP_GRAPH, 1), (TY_OWN, 1), ("crates/hale-types/src/flows.rs", 1)] },
        ],
    },
    Family {
        name: "flows",
        layer: Layer::Locus,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which children are flows (released per completion) and which are resident; and, per locus declaration, whether its `run()` is long-running and whether it never returns.",
        inputs: &["release declarations", "accept declarations", "run bodies", "declared loci and type aliases (handler_routing's resolver)", "import renames"],
        producer: Some(site("crates/hale-types/src/flows.rs", "survey")),
        legacy: &[],
        consumers: &[
            consumer_at("check --flows (each flow type as written, with its clauses)", V_CHECK, "flows::survey("),
            consumer_at("check (a daemon-shaped locus that accepts a child type it releases no clause for: a law over the rows)", CHECK, "check_accept_release"),
            consumer_at("check (the run rows: the long-running-child rule's long-running column, the starvation and birth-order laws' never-returns column; one survey per check, handed to all four)", CHECK, "run_of"),
            consumer_at("resolved program (the lowering view's rows, over the merged program)", TY_RESOLVED, "flows::survey("),
            consumer_at("a declaration's dependents (X2: a flow child and its `release` owners are neighbours, `Snapshot::declaration_dependents`)", SNAPSHOT, "flows::survey("),
            consumer_at("the lifecycle plan (an accepted flow is torn down by the reclaim its run's end runs, a resident by its owner's cascade: the snapshot's rows over the checked programs)", SNAPSHOT, "flows::survey("),
            consumer_at("codegen (run elision, run-end reclaim and the release call: `Cx::is_flow`, one row read)", CG, "is_flow"),
            consumer_at("codegen (the generic-instantiation queue: each locus specialization it creates asks the row for its template's clauses, under the substitution its synthesis applies)", CG, "specialize("),
        ],
        invariants: &[
            "a release clause's child is resolved once, by `child_locus_name` (handler_routing's resolver: aliases, generic instantiations, qualified paths), into the row (`FlowClause::locus`); lowering's flow-ness is a row read (`flows::is_flow`), never a comparison of its own",
            "the flow facts cover the specializations lowering creates: a clause whose type mentions its owner's type parameters names no locus by itself and carries its template (`FlowClause::template`: the owner's identity, its parameters in order, the type as written); `FlowRows::specialize` answers for one specialization by resolving the template's type under the substitution lowering's synthesis applied, so `Manager<Worker>`'s `release(c: T)` makes `Worker` a flow exactly as a concrete `release(c: Worker)` does",
            "the checker's accept/release rule judges over the rows: the release clauses a locus declares are the rows' clauses inside its declaration",
            "every locus declaration, a module's included, has a run row (`RunRow`: the declaration's name and span, which `FlowRows::run_of` finds it by, and two columns, long-running and never-returns, the `nonreturning` family's two definitions); the checker surveys the rows once per check and its four readers share that survey",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/release_reclaims_flow.rs", "crates/hale-codegen/tests/release_two_parents.rs", "crates/hale-codegen/tests/release_generic_owner.rs", "tests/hale/release_generic_owner_test.hl", "crates/hale-types/src/flows.rs (a_clause_names_the_locus_lowering_names, a_template_clause_names_the_specialization_s_argument)"],
        spec: &["spec/semantics.md § release(c) and flow children"],
        owned: &[],
        seams: &[Seam { symbol: "flows::survey(", allowed: &[(CHECK, 1), (TY_RESOLVED, 1), (V_CHECK, 1), (SNAPSHOT, 2), (LIFECYCLE_DERIVE, 1)] }],
    },
    Family {
        name: "restart",
        layer: Layer::Locus,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which loci declare restart operations, which restart in place, and what the restart bound is.",
        inputs: &["closure and birth-check declarations", "handler_routing (the rows' recovery ops)"],
        producer: Some(site(HANDLER_ROUTING, "handler_rows")),
        legacy: &[],
        consumers: &[consumer_at("codegen (which loci get restart points: __restart_<L>, __resume_<L>)", CG_RESTART, "can_fail"), consumer_at("codegen (the `for N` retry bound)", CG, "retry_bound_at"), consumer("codegen (__restart_<L>, __resume_<L>)"), consumer("model")],
        invariants: &[
            "a recovery op is a row with a witness, per (parent, child)",
            "every `for` bound a recovery statement writes is an entry of the rows, keyed by the statement's span (`HandlerRouting::retry_bound_at`): a literal is its value, any other expression the site of the expression written there, which lowering lowers once where the statement runs. The model's `retry_bound` is the last literal of a handler's entries, so the bound modelled and the bound lowered are one fact",
            "which loci a failure can originate in (a closure of any epoch, `inline` ones and so every `violate` included, or a `birth_check`) is a column of the rows per locus declaration (`HandlerRouting::can_fail`), over the merged program lowering walks; lowering emits restart points exactly where it answers yes. A monomorph is no declaration and is not in the column, so a generic locus that declares a closure gets no restart points (known open)",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/restart_in_place_params.rs", "crates/hale-codegen/tests/restart_bound.rs", "crates/hale-types/tests/handler_routing_probes.rs (the failure column, the bounds)", "crates/hale-codegen/tests/restart_spine_ir.rs"],
        spec: &["spec/semantics.md § supervision"],
        owned: &[site(HANDLER_ROUTING, "recovery_ops")],
        seams: &[
            Seam { symbol: "recovery_ops(", allowed: &[(HANDLER_ROUTING, 2)] },
            Seam { symbol: "can_fail(", allowed: &[(HANDLER_ROUTING, 1), (CG_RESTART, 1)] },
            Seam { symbol: "retry_bound_at(", allowed: &[(HANDLER_ROUTING, 1), (CG, 1)] },
        ],
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
        consumers: &[consumer_at("check", CHECK, "check_decorator_stacks"), consumer("claims (certificate, causes, depends, budget)"), consumer_at("the certificate evidence (the check's effects certificate report, read by the check's laws and the artifact's)", SNAPSHOT, "demand_effect_certificates"), consumer_at("check (a codec binding's purity assertion reads the purity column, demanding the rows only when a codec reaches it)", CHECK, "CheckInputs"), consumer_at("check (the blocking warning: which helpers hold a cooperative worker, the BLOCK class with its leaves and resolved targets, demanding the rows only once a placed field has a `run()` to walk)", CHECK, "worker_holding_fns"), consumer_at("model (effect labels, lower bounds and direct contributions, the last read by the reachability judgment's `effects(C)` test, and the summary the rows' walk read: the snapshot's rows, handed in)", MODEL_BUILDER, "ModelInputs"), consumer_at("the effects manifest (`--dump-effects-manifest`, `--check-effects-manifest`: the snapshot's rows, cross-seed calls resolved through the renames)", V_CHECK, "demand_effects"), consumer_at("replay (the live-effects gate reads the manifest over the snapshot's rows, and refuses when they are blocked)", V_REPLAY, "demand_effects"), consumer("doc")],
        invariants: &["derived ⊆ declared is the certificate; budgets are judged through evidence (spec/verification.md)", "effects run once per snapshot, not once per consumer: `Snapshot::demand_effects` runs `derive_effect_rows` once over the snapshot's allocation summary (`demand_alloc_summary`: the checked programs and the stdlib's analysis copy, cross-seed calls resolved through the import renames), counted as `effects`, blocked with the scope", "an unresolved edge is coverage, never a violation: a row's `effects` saturates to `UNCLASSIFIED` when the walk reaches what it cannot name, its `known` set is the lower bound an unresolved edge never erases, and `unknown` says the walk reached such an edge", "an indirect call is one rule for every certificate and budget reader: a call through a function value the summary resolves (the `alloc_summary` family's function-value rule) is its alternatives, each judged as the call of that fn, and one it does not resolve (`CallEdge::indirect`: a function-typed parameter, an unfollowed local or a computed callee no function value of the program can be) is a call that may do anything, never a call to nothing: the effects engine and the rows read it as `UNCLASSIFIED`, `@budget` and the quantitative dimensions as unbounded, the model as an `IndirectCall` hole, so every certificate over it is refused or uncertified (a classified correction, F.40 E5: a call through an unresolved local was a call to nothing, pinned in `indirect_calls.rs`)", "a row is keyed by the fn's name (`FnKey`) until the `snapshot_identity` family's declaration rows carry it", "a fn's direct contribution is a column (`direct`; `EffectRows::direct` answers any key, a bodyless one by what it carries): the model's function rows and absorbed paths, which the reachability judgment's `effects(C)` destination test reads, take it from the rows, and nothing outside the producer folds a body for it", "purity and the lower bound are columns of the rows: one walk answers a fn's saturating set and its lower bound (`infer_effect_bounds`), and the purity walk runs only inside the producer; the checker's codec law reads the purity column through `CheckInputs::effects`, a demand made only when a codec binding reaches the assertion, and the blocking check its BLOCK class, a demand made only once a placement entry puts a field with a `run()` on a classic pool or main, so a check of a program that does neither runs no effects fixpoint", "the effects certificate engine runs once per snapshot, in the check (`check_bundle_reporting`); the certificate evidence reads that report (`Snapshot::demand_effect_certificates`, handed to `derive_certificate_evidence_over`) and never runs the engine itself; a bundle no check ran over runs it once for itself (`effect_certificates`)"],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/effect_assertions.rs", "crates/hale-types/tests/effects_cross_seed_correction.rs", "crates/hale-types/tests/indirect_calls.rs", "crates/hale-cli/tests/effects_baseline_gate.rs", "crates/hale-cli/tests/effects_manifest.rs"],
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
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which fns block (a cooperative worker would be held), and whether the program places anything off the main thread.",
        inputs: &["stdlib_surface (holds_cooperative_worker)", "effects (the rows' BLOCK class, their resolved targets and unresolved leaves)", "placement (the table's domains)", "bindings (the program's own main's entries)"],
        producer: Some(site(CHECK, "blocking_path_match")),
        legacy: &[],
        consumers: &[consumer("check (rules 7, 8)"), consumer("effects (@no_block)"), consumer_at("codegen (mark_pinned, no_pinned dispatch: `program_has_offthread` over the view's placement table and binding rows)", CG, "program_has_offthread")],
        invariants: &[
            "one leaf set (GH #830) and one propagation",
            "whether a thread crosses the bus boundary is read off rows, once, in lowering (`program_has_offthread`): a domain of the placement table that is not main (`PlacementTable::places_off_main`: a pinned anchor, a non-main pool, an adapter binding's thread, the api binding's synthesized pool) or an entry of the program's own `main locus` (`BindingRows::binds_on_main`: not an imported main, a module-nested one included, as lowering's prelude lowers it); the `lotus_bus_mark_pinned` call and every static dispatch's `no_pinned` flag are that one value and its negation",
            "an imported library's `main locus` is never deployed, so its `placement { }` block places nothing and its `bindings { }` bind nothing: a program importing one is not off-thread for it (a classified correction, pinned in `offthread_imported_main.rs`), and a deployed module-nested main's binding makes its program off-thread (the same correction's other half: the old binding term scanned top-level items only)",
            "the helpers that block are the effect rows' (`worker_holding_fns`, demanded through `CheckInputs::effects` only once a placed field has a `run()` to walk): a fn whose `direct` BLOCK comes from a leaf `holds_cooperative_worker` names (the BLOCK class includes `std::time::sleep`, which yields the worker, so the leaf test stays beside the row), closed over the rows' resolved targets; the check folds no call graph of its own",
            "the rule's horizon: the rows' propagation sees what the old name-keyed walk did not (a qualified cross-seed call, a stdlib body behind a handle method, another locus's method), but the `run()` walk consults the set only at a bare call or a `self.m()` call, so the horizon decides what a helper reaches and never which call in `run()` is looked at; the rule's diagnostics over the corpus, tests/hale and the DNA seeds are the old walk's (spec/verification.md § Concurrency & placement safety)",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/placement.rs", "crates/hale-codegen/tests/bus_devirt_no_pinned.rs", "crates/hale-types/tests/checks_inside_modules.rs", "crates/hale-cli/tests/offthread_imported_main.rs"],
        spec: &["spec/semantics.md rules 7, 8", "spec/verification.md § Concurrency & placement safety", "spec/semantics.md § The entry locus"],
        owned: &[site(CHECK, "worker_holding_fns"), site(PLACEMENT, "places_off_main"), site(BINDING_ROWS, "binds_on_main")],
        seams: &[
            Seam { symbol: "places_off_main(", allowed: &[(PLACEMENT, 1), (CG, 1)] },
            Seam { symbol: "binds_on_main(", allowed: &[(BINDING_ROWS, 1), (CG, 1)] },
        ],
    },
    Family {
        name: "alloc_summary",
        layer: Layer::Effects,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Where each allocation lands and when it is reclaimed: per-fn allocation, escape, scratch eligibility, method-scratch elision, stack arrays, arena elision.",
        inputs: &["bodies", "signatures (non_allocating, fallible, ffi)", "the callgraph", "ownership (accept sets)"],
        producer: Some(site(ALLOC, "derive_alloc_summary")),
        legacy: &[],
        consumers: &[consumer_at("the effects certificate engine (the check's `@effects`, `@phase_effects` and placement diagnostics: the snapshot's summary, handed in)", CHECK, "CheckInputs"), consumer_at("effects (the rows walk the snapshot's summary and hold it, shared)", SNAPSHOT, "demand_effects"), consumer_at("check (the unbounded-allocation advisory and `--dump-alloc-summary`: the snapshot's summary)", V_CHECK, "demand_alloc_summary"), consumer_at("the editor's typing stage (the advisory in the diagnostics, `Config::alloc_advisory`: the snapshot's summary)", SNAPSHOT, "typing_stage"), consumer_at("lsp (hale/allocSummary: the snapshot's summary)", LSP, "demand_alloc_summary"),consumer_at("model (its function rows, dispatch sites and holes: the snapshot's summary's own rows)", MODEL_BUILDER, "own_rows"), consumer_at("topology (the artifact's fn sort, labels, derived effects and through-stdlib contraction: the snapshot's summary, handed in)", TOPOLOGY, "dump_topology_over"), consumer_at("check (the hot-path lint: a law over the program's own rows and their columns, the snapshot's summary, handed in)", CHECK, "check_hot_path_alloc"), consumer_at("claims (@budget: the counting engines over the snapshot's summary's own rows, handed to the evidence)", EVIDENCE, "derive_certificate_evidence_over"), consumer_at("codegen (arena routing at an allocation)", CG, "current_arena_ptr"), consumer_at("codegen (a free fn's scratch: the view's non-allocating and scratch-local rows)", CG, "alloc_routing"), consumer_at("codegen (a locus's arena, and its hooks', methods' and modes' scratch: the elision rows, a monomorph's specialized)", CG, "locus_elision"), consumer_at("resource_budget (the fd sites and the fd-leak warnings: the snapshot's summary's own rows)", "crates/hale-types/src/resource_budget.rs", "own_rows"), consumer_at("frontier (the `causes:` engine: the summary's own rows)", FRONTIER, "causes_inner"), consumer_at("a declaration's dependents (X2: the call edges of the rows and of the declaration bodies)", DEPENDENTS, "declaration_bodies")],
        invariants: &[
            "one summary per snapshot: `Snapshot::demand_alloc_summary` runs `derive_alloc_summary` once over the checked programs with the stdlib's analysis copy beside them (cross-seed calls resolved through the import renames), counted as `alloc_summary`, blocked with the scope; the check's effects certificate engine reads it (`CheckInputs::alloc_summary`) and the effect rows walk it, so neither builds its own; `summarize_identified` is the one constructor, and `derive_alloc_summary` (which places the stdlib's analysis copy beside the programs itself) its one caller outside tests, so no reader builds a summary variant of its own",
            "the checker's reclaim model and codegen's routing agree; a stale copy is a registry violation, not a comment: `summarize_identified` classifies each free fn's frame (`FnSummary::frame`: `ScratchLocal { recursive }` or `CallersArena`; `None` for a method, a hook, a mode, `main` and an unclassified row) with the classifier lowering reads (`alloc_routing::scratch_local_free_fns`), over the declarations lowering runs it over (the programs, their imported seeds under their mangled names, and the stdlib, beside the program whether or not the summary holds the analysis copy's rows) with the same renames, so the frame is lowering's row on every target (pinned in `alloc_summary_reclaim_boundary.rs`)",
            "the reclaim boundary is relative to the loop analyzed (E3b, a classified correction, pinned per target in `alloc_summary_reclaim_boundary.rs`): a `Local` allocation of a scratch-local fn that is not recursive reclaims at its return (`ReclaimScope::FnReturn`), which `ReclaimScope::accumulates_in_loop` places inside each iteration of a caller's loop and outside every iteration of the fn's own (`LoopAt`); such a fn stops the scratchless long-lived propagation as a method's per-call scratch does; `Local` is not scratch, so a `Local` of a `CallersArena` fn run once per iteration of an unbounded loop in a long-lived frame lands in that frame's arena and accumulates (`LeakReason::InCallersArena`), a locus instantiation excepted; an escaping value keeps its boundary, and a recursive fn keeps the locus boundary; the model, the budgets and `shape_hash` read none of it",
            "lowering routes from the view's rows (`LoweringView::alloc_routing`, `derive_alloc_routing` over `merged` with the view's renames) and derives none: a free fn skips its per-call scratch when the rows call it non-allocating (FORM-3, a greatest fixpoint keyed by name), and its body allocates into its own subregion when they call it scratch-local (#1148); a locus's arena is elided, and a lifecycle hook, `fn` method or mode lowers without its per-call scratch, when its elision row says so (`LocusElision`, keyed by the locus and the member's position; a generic locus's monomorph, which lowering synthesizes, takes `AllocRouting::specialize` over the synthesized declaration); the FORM-3 classifier (`fn_body_definitely_non_allocating`) is the rows' own and lowering calls it nowhere; the two method-elision stages still classify a self field differently (stage 1, the per-member verdict, by its literal default or its ascription through aliases; stage 2, the `self.m()` sets, by a primitive ascription only), and a monomorph still has no stage-2 set, both as they were; a free fn's entry publishes its caller's arena to the caller-arena TLS when its row says so (`caller_arena_publish`: not non-allocating, and a call or a struct literal anywhere in its body, found by a structural walk, a string literal spelling `Call {` or `Struct {` counting as the Debug-string test it replaced counted it; a generic fn's monomorph takes `AllocRouting::specialize_fn`), and no body's Debug rendering decides it; the rows are #1208's classification moved as it was, so its builtin list is still a hand-kept subset of the checker's `BARE_BUILTIN_CALLEES` with no agreement test, its `std::` namespaces a per-namespace claim no stdlib_surface row states, and its qualified-path lookup checks the import renames before `PATH_RENAMES` where `resolved::lookup_qualified_path` checks them in the other order",
            "a body's escape tags key a binding by the declaration its escaping uses name (`Snapshot::declaration_of`), so an inner shadow of a returned name is its own, local binding (the #1140 shape), and close over `let x = y;` aliases to the declaration y names, as borrow_lifetime's `returned_decls` does; programs minted by different snapshots are summarized each with its own identities (`summarize_identified`: a bundle's programs beside the bundled stdlib's analysis copy)",
            "each seed's names resolve in its own scope: the programs minted with one set of identities are one scope, a body's bare free-fn name (and the locus a call's result is typed by) resolves only to a fn of its own scope, and the import renames are the bundle's names (the stdlib's analysis copy imports nothing), so a stdlib body's builtin `count(...)` is the builtin, never a user fn called `count` (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)",
            "the unbounded-invocation fixpoint (`AllocSummary::unbounded_invoked`) seeds from what the program reaches: beside the stdlib's analysis copy, `AllocSummary::reached` is the program's own fns, what their calls reach (the interface fan-out included) and the hooks and bus handlers of every locus they start (a struct literal of it in a reached body, or a param field of a started locus by declared type or default literal; every locus of the program's own is started), and only those seed it or call, so a stdlib loop the program never starts invokes nothing (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)",
            "the run-to-exit rule (a `main` and no long-lived entry: no leak sites) reads the program's own entries, never the stdlib's analysis copy's (`AllocSummary::analysis_copy` names the copy's fns, `is_own` the program's), since the copy always carries `run` hooks (a classified correction, pinned per target in `alloc_summary_construction_correction.rs`)",
            "a program's declaration is the row where the stdlib's analysis copy declares the same name: the copy's top-level free fn, locus or interface of a name a checked program declares stays out of the summary, so a stdlib source file checked as itself keeps its own rows (its bodies, its spans, resolved in its own scope); only stdlib source shares such a name (a classified correction, pinned in `alloc_summary_construction_correction.rs`)",
            "the rows carry what the hot-path lint reads: a fn's `@hot` and whether it is a mode (`FnSummary::hot`, `mode`), the order of the declarations (`decl_index`), where a site or a call is written (`in_loop`, which a `return` / `fail` payload does not reset as it resets `loop_depth`), a struct literal written as a whole statement (`AllocSite::bare_stmt`) or as the whole right side of a `self.<field> =` replace (`self_replace`, the statement's span; an in-place replace, which allocates nothing, is in `FnSummary::in_place_sites`, not `sites`), the `let` a call is the value of (`CallEdge::let_span`), how a call is spelled (`CallSpelling`), and the allocating receives (`CallEdge::allocating_recv`, the one list, which `@budget` reads too)",
            "the hot-path lint is a law over the rows (`check_hot_path_alloc`, over the summary the check is handed): the program's own fns, a mode aside, in declaration order, each finding where its site or call is written, a bus handler's at any depth, `@hot` an error and `@unbounded` silencing the advisory; no reader walks bodies for it, so it sees what the summary's walk sees (a publish, a bare block, `violate`, a recovery and `shm_write` included, which the lint's own walk skipped; an index expression's subscript included, walked as any operand is, as the lint's walk walked it, so a `@hot` fn keeps that rejection (a classified correction, pinned in `hot_path_alloc.rs`); a callee that is an expression of its own excluded, which it reached) and its diagnostics over the corpus, the targets and the lint's pins are the lint's, in order (pinned in `hot_path_alloc.rs`)",
            "a module-nested declaration is summarized like a top-level one: every declaration pass walks `module { … }` nesting (`flat_decls`; a module is a namespace, not an analysis boundary, GH #764), so a module-nested fn or locus member has a row and a call into it resolves, and the model holes out no module-nested body; the one body with no row is an `on_failure` handler, summarized as a declaration body (a classified correction, pinned in `alloc_summary_construction_correction.rs`)",
            "the advisory, `--dump-alloc-summary` and the editor's hale/allocSummary read the snapshot's summary and report the program's own rows judged over the whole of it (the stdlib's analysis copy and the renames included: a classified correction, pinned per target in `alloc_summary_correction.rs`); a leak site is the program's own (`AllocSummary::is_own`) and is left out of the advisory only when it has no author position (`AuthorPositions::has`: its span at or beyond `API_SYNTH_BASE`, or in a declaration the origin rows mark synthesized whose offset no source file owns), never by its owner's name; the check's warnings, the editor's diagnostics and the editor's hale/allocSummary decide with one function (`advisory_leak_sites`)",
            "a call through a function value resolves to the program's function values (F.40 E5): the summary marks a call through a function-typed parameter, a local its walk does not follow to a fn, or a computed callee `CallEdge::indirect`, and `resolve_function_values` rewrites it into one alternative per fn some expression of the bundle reads as a value (`fn_values`: every expression the identity walk reaches, a callee written as a name or a path and a name a binding in scope spells excluded; resolved as a `let` of it resolves) whose arity is the call's and whose declared signature can be the parameter's or the bound field's declared type, sharing a dispatch group (`CallEdge::via_value` the callee as written); a call no such value can be stays indirect, and so does every call of a bundle that reads a locus's method as a value, which this does not follow",
            "the model projects the program's own rows; a call into the analysis copy is the unresolved row: the model, the `@budget` engines (`budget_check`, `quantitative`), the artifact's user rows, the frontier's `causes:` engine and the resource budget (its fd sites and fd-leak warnings) read the snapshot's summary through `AllocSummary::own_rows` (the program's fns and loci; a call resolved into the stdlib's analysis copy is the unresolved call by the method's bare name, the receiver kept; a dispatch keeps its alternatives among the program's own loci, renumbered in key order, through the copy's interface is no dispatch, and through the program's own with none of its conformers left is the dead site; a function-value dispatch keeps its alternatives among the program's own fns and the leaves the program itself reads as values (`AllocSummary::copy_alternative`), its groups numbered after the interface groups, and with none left is the indirect call as written), which is the program-alone summary field for field, so the copy moves no `shape_hash`, the build and replay identity, and the copy's fd-acquiring bodies are not the program's resources (pinned per target in `alloc_summary_own_rows.rs`); what the model would gain from the copy's rows is a separate correction",
            "a body the program writes that is no row is a declaration body (`AllocSummary::declaration_bodies`, `DeclarationBody`): an `on_failure` handler, a params block's initializers, a constant's value, a `birth_check`, a perspective's fns, params and `stable_when`, a synthesized hook, each wherever it is declared (a `module { }`'s included, through `flat_decls`, as the rows are), and every subexpression a walk does not descend into (a callee that is no name; an index's subscript is the row's own, walked), each walked as a row is and keyed to the site of the declaration it is a member of; no judgment reads them, so the rows and every consumer's answer are what they were without them (X2, a review fix: `hale check` with every dump over the corpus, `tests/hale` and every `dna/**/main.hl` unchanged), and the declaration dependents relation reads their call edges as the declaration's own",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/hot_path_alloc.rs", "crates/hale-types/tests/alloc_summary_construction_correction.rs", "crates/hale-types/tests/alloc_summary_correction.rs", "crates/hale-types/tests/alloc_summary_own_rows.rs", "crates/hale-types/tests/alloc_summary_reclaim_boundary.rs", "crates/hale-codegen/tests/alloc_model_rss.rs", "crates/hale-codegen/tests/scratch_local_free_fn.rs", "crates/hale-codegen/tests/fn_nonalloc_add.rs", "crates/hale-codegen/tests/method_scratch_elision.rs"],
        spec: &["spec/memory.md § Allocation routing", "spec/verification.md § Memory-bound proofs (item 1): the reclaim boundary", "spec/styleguide.md"],
        owned: &[site(ALLOC, "summarize_identified"), site(ALLOC_ROUTING, "derive_alloc_routing"), site(ALLOC_ROUTING, "scratch_local_free_fns")],
        seams: &[
            Seam { symbol: "scratch_local_free_fns(", allowed: &[(ALLOC_ROUTING, 1), (ALLOC, 1)] },
            Seam { symbol: "summarize_identified(", allowed: &[(ALLOC, 4)] },
            Seam { symbol: "allocating_recv(", allowed: &[(ALLOC, 2)] },
            Seam { symbol: "check_hot_path_alloc(", allowed: &[(CHECK, 2)] },
            Seam { symbol: "derive_alloc_summary(", allowed: &[(ALLOC, 1), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2), (EFFECTS, 1), (EVIDENCE, 1), (JUDGMENT, 1), (TOPOLOGY, 1), ("crates/hale-types/src/resource_budget.rs", 1)] },
            Seam { symbol: "own_rows(", allowed: &[(ALLOC, 1), (MODEL_BUILDER, 1), ("crates/hale-types/src/budget_check.rs", 1), ("crates/hale-types/src/quantitative.rs", 1), (FRONTIER, 1), ("crates/hale-types/src/resource_budget.rs", 2)] },
            Seam { symbol: "derive_alloc_routing(", allowed: &[(ALLOC_ROUTING, 1), (TY_RESOLVED, 1)] },
            Seam { symbol: "fn_body_definitely_non_allocating(", allowed: &[(ALLOC_ROUTING, 12)] },
            Seam { symbol: "unbounded_alloc_warnings(", allowed: &[(TLIB, 1), (V_CHECK, 1), (SNAPSHOT, 1)] },
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
        invariants: &["runs on every entry point: it does not run in the LSP or bench today (phase 2 closes that)", "which locus accepts which child is the ownership rows' relation (`accepts_ancestor`, the snapshot's graph's rows, handed in); it builds no accept set of its own"],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/check_borrow_lifetime.rs"],
        spec: &["spec/semantics.md § A borrow outlives its holder", "spec/decisions.md F.39"],
        owned: &[],
        seams: &[Seam { symbol: "borrow_lifetime_diags", allowed: &[("crates/hale-types/src/borrow_lifetime.rs", 3), (TLIB, 1), (V_CHECK, 1)] }],
    },
    Family {
        name: "bare_fallible",
        layer: Layer::Effects,
        state: State::Canonical,
        kind: Kind::Law,
        answers: "Whether a fallible call's error is addressed.",
        inputs: &["typed_bodies (the fallible_calls column: each call's callee mark and what addresses it)"],
        producer: Some(site("crates/hale-types/src/bare_fallible.rs", "bare_fallible_calls")),
        legacy: &[],
        consumers: &[consumer_at("check, verify, build, run, test, replay, bench, the LSP (the snapshot's check, with the typing diagnostics)", SNAPSHOT, "bare_fallible_calls"), consumer_at("the check of a bundle no snapshot holds (the tests' entries)", CHECK, "bare_fallible_calls")],
        invariants: &[
            "one rule for user and stdlib calls: only an `or` handles a fallible call; any other position (an argument, an operand, a `match` scrutinee, a `let` initializer, a statement, a returned value) is the GH #738 error, so `hale check` refuses what `hale build` refuses, save a call through an interface-typed local or parameter: the checker types that receiver `Unknown`, so the column holds no row for the call and only the build refuses it (a typing limitation to lift)",
            "the law reads the column, never the signature table or the syntax tree: the checker records each fallible call's callee mark (`Declared`, `Typed`, `Stdlib`) and what addresses it as it walks",
            "an `or`'s handler takes the implicit `or raise` exactly where lowering does (`expr_is_fallible_call`): a `Declared` callee; any other fallible handler is refused with the nested spelling, a limitation to lift, not a rule",
            "the checker types a fallible value no `or` addresses as its success type, so a bare call reports the law's one error and no type mismatch",
            "runs on every entry point, the LSP and bench included (the snapshot's check)",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-cli/tests/check_strict_fallible.rs", "crates/hale-types/tests/typed_bodies.rs"],
        spec: &["spec/semantics.md § A bare fallible call is an error", "spec/types.md"],
        owned: &[],
        seams: &[Seam { symbol: "bare_fallible_calls(", allowed: &[("crates/hale-types/src/bare_fallible.rs", 1), (SNAPSHOT, 1), (CHECK, 1)] }],
    },
    Family {
        name: "nonreturning",
        layer: Layer::Effects,
        state: State::Canonical,
        kind: Kind::Law,
        answers: "Which `run()` bodies never return, which children are long-running, and whether the birth order or a pool starves because of it.",
        inputs: &["flows (each locus declaration's run row: the long-running and never-returns columns)", "params order", "placement (the table's rows for the deployed root's fields)"],
        producer: Some(site("crates/hale-types/src/flows.rs", "run_statically_nonreturning")),
        legacy: &[],
        consumers: &[
            consumer_at("check (the nested-long-running-child rule reads the long-running column)", CHECK, "check_nested_long_running_child"),
            consumer_at("check (the starvation and birth-order laws read the never-returns column)", CHECK, "never_returns"),
        ],
        invariants: &[
            "two definitions, two columns of the flow rows' run row (`flows::RunRow`), named in spec/runtime.md § Typecheck enforcement: long-running is a `run()` body with a statement of its own (a nested child's `run()` completes before its parent's begins, so any body delays the parent, whether or not it returns), never-returns is a terminal `while` with no exit whose condition never flips false (only such a body starves the cells a pool runs after it); every body that never returns is long-running, not the converse, and a child whose `run()` is `std::time::sleep(1m)` keeps the long-running-child error",
            "the checker surveys the flow rows once per check and the three rules read the columns; none decides either question itself; a stdlib locus, whose body the checker does not see, is both when it is on the known-long-running allowlist (`KNOWN_LONG_RUNNING_STDLIB_LOCI`)",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/birth_order_trap.rs", "crates/hale-codegen/tests/birth_order_trap.rs", "crates/hale-codegen/tests/nested_long_running_child.rs", "crates/hale-types/src/flows.rs (long_running_and_never_returns_are_two_columns)"],
        spec: &["spec/semantics.md", "spec/runtime.md § Typecheck enforcement"],
        owned: &[site(CHECK, "check_pool_starvation"), site(CHECK, "check_birth_order")],
        seams: &[Seam { symbol: "run_statically_nonreturning(", allowed: &[("crates/hale-types/src/flows.rs", 2)] }],
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
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which thread domain each instance runs in: pools, pinned threads, replicas, affinity, and the deployment plan.",
        inputs: &["placement and topology blocks", "entrypoint (the lowering root, never the entry)", "the construction templates: the root's literals, the entry's implicit construction of a root no literal builds, `fn main`'s own literals, the root's `bindings { }` adapters", "the params towers each template builds", "the minted sites of the snapshot and of the stdlib analysis copy", "free fns and locus bodies (dynamic sites, their domains and bounds)"],
        producer: Some(site(PLACEMENT, "derive_placement")),
        legacy: &[],
        consumers: &[consumer_at("check (rule 6, the lowering laws)", LOWERING_LAWS, "pinned_features"), consumer_at("check (instance aliasing, #334: where each of the deployed root's params fields runs, the table's root and the domains of its rows)", CHECK, "check_instance_aliasing"), consumer_at("check (rule 17, the lowering laws: the root's constructions and their bounds)", LOWERING_LAWS, "pinned_root_in_a_loop"), consumer_at("check (rule 18, the lowering laws: the root and its constructions)", LOWERING_LAWS, "placement_entry_consumed"), consumer("check (rules 2-5, 13-16)"), consumer_at("check (F.31: the caller per instance, the receiver by its row's `owner_relative`)", CHECK, "check_placement_single_thread"), consumer_at("check (the blocking check and the starvation and birth-order laws: where each of the deployed root's fields runs, its pool's `async_io`, and the declarations it realizes)", CHECK, "root_field_placements"), consumer_at("check (type-check rule 20: the construction paths of a handler birth's enclosing locus, `OwnershipGraph::construction_paths` over the table handed in, derived only when that locus does not accept the child itself)", CHECK, "check_unowned_subscriber_locus"), consumer_at("sync_inference (accessor domains per instance)", SYNC, "infer_sync_for_bundle"), consumer_at("dispatch (domains: both plans' domain map, `Arrangement::domains`, over the arrangement projection)", ARRANGEMENT, "fn domains"), consumer_at("model (the arrangement: instances, owners, placed_in and affined_to, the table's rows projected through `project_arrangement`, user-only)", MODEL_BUILDER, "derive_application_model_over"), consumer_at("the intra-locus rewrite (a publish into a field off its owner's thread stays on the bus: `PlacementTable::off_owner_fields`)", TY_RESOLVED, "rewrite_intra_locus"), consumer_at("codegen (the deployment plan, the table's lowering view: the root, and per root field an entry decides its schedule class, pool, NUMA node and replica cores; the pools' async_io and affinity; the pinned anchors' and pooled rows' realized declarations, an adapter among the anchors)", CG, "collect_main_placement"), consumer_at("resource budget (the threads, partitioned by the scope that creates them: the root's pinned anchors under their construction's bound, the adapters once; the worker pools, main never one)", "crates/hale-types/src/resource_budget.rs", "budget_for_programs"), consumer_at("codegen (whether a thread crosses the bus boundary: a domain that is not main, `places_off_main`, over the lowering view's table, the snapshot's, handed in)", CG, "program_has_offthread"),consumer_at("codegen (the registration route: the pinned anchors whose tree holds a subscriber, by lowered name, each given a mailbox its descendants' subscriptions route to)", TY_RESOLVED, "route_anchors"), consumer_at("bus_graph (every placement label and the direct-call gate: the set of each type's instances' domains)", BUS_GRAPH, "type_placements"), consumer_at("check (a subscriber's `bounded(N, …)`, legal only where every instance runs on main: B-2, read only when a subscriber is bounded)", CHECK, "check_bounded_bus"), consumer_at("ownership (each bubbling edge's class: the enclosing instances paired with their owner rows)", OWNERSHIP_GRAPH, "relate"), consumer("lsp (hale/placement)"), consumer("deployment (reserved)")],
        invariants: &[
            "placement is keyed by instance, never by type: one row per static instance of each construction template (a key is its origin, its field path, its replica), and a type's answer is the set of its instances' domains",
            "the entry is a construction scope: a root no literal builds is the entry's implicit template (`Origin::Entry`, bound `Once`), and `fn main`'s own literals are templates bound by their statement's loop context; an adapter is an origin of its own, built once",
            "nested rows inherit their owner's domain unless a root field's entry or a binding decides it; pinned domains are per anchor and per replica, pool domains one per name with at most one affinity",
            "the root is `lowering_root`, never the entry; an imported `main` is never the root",
            "every root entry decides exactly one field family in each construction template, or it is a hole",
            "unknown is a hole, not a default: an unresolved declaration, an unenumerable initializer, a held instance (`Reuse`) and a dynamic site of unknown domain each carry their policy, and none is main",
            "a held instance's subtree lives in its holder's domain: the held row keeps its `Reuse` hole and its owner's domain, the source's actual rows (never the declaration's defaults) are projected under it, inherited, and each of those rows names its own source row, the one it was built as (`built_by`); where the source is not linked, nothing below the held row is asserted, and an instance there runs in an unknown domain; a question of where an instance runs skips the source's rows, a count of instances skips the held ones",
            "every site the table names carries the universe that minted it (`SiteRef`); lowering joins the stdlib's into its merged mint once, totally and injectively",
            "the checker builds no table: the snapshot demands it and hands it to the check (`CheckInputs::placement`) and to the form rows; a bundle no snapshot holds (the test entries `check_bundle`, `check_bundle_opts_scoped`, `derive_application_model`, `effect_certificates`) builds it once, over a minted bundle (an unminted one names no site and gets an empty table, which judges nothing), except that the model's test entry (`derive_application_model`, and the check's claims through it) mints a copy of an unminted bundle first, since the model's arrangement is the table's rows", "the model's arrangement is the table's rows projected (`placed_in.push(` and `affined_to.push(` have one writer): the deployed root's templates, each instance where it runs, user declarations only; the table, not the arrangement, answers every placement question. The projection is one function, `project_arrangement` (F.40 phase 3, C5): the model builder makes its rows and placement holes from it, and lowering reads the dispatch plans' domains from it over the same snapshot's programs, table and ownership graph",
            "a consumer asks where instances run of `PlacementTable::running` (the handed-off rows skipped): a type's answer is the domains of its instances, compared per instance, never collapsed to one per type; an enclosing locus with no static instance is a hole that disables the F.31 proof, never a default to main or to pinned",
            "a count over the table is over templates: a domain belongs to a template, as the key that anchors it does, and has one anchor per live occurrence; a count sums the templates, each times its bound (`Unbounded` makes the count an uncertainty with its reason), takes the maximum over the alternatives of one step (one occurrence takes one), counts each replica row once and never multiplies it by K again, and counts an adapter once, never under a root construction's bound (`PlacementTable::templates`, `per_occurrence`)",
            "the resource budget counts the resource, read from the table: OS threads are the pinned anchors (one per replica) times their construction's bound, plus one per adapter; a pool is one worker however many instances it holds, an affinity is a column of its domain and never a thread, and `main` is never a pool; only the deployed root's rows count, so an imported or non-root `main`'s entries cost nothing; a binding's reader thread and a stdlib transport's serve thread are not placement facts and are named as not counted",
            "lowering's deployment plan is the table's lowering view, read once before lowering (`collect_main_placement` over `LoweringView::placement`): no field name or written type decides a schedule class, a pool or a type set; a user site the table names is the node of the same index in the merged program, and the deciding entry's written selector says only whether one core is emitted alone or as a set",
            "subscriptions follow the tower (U-6): a nested instance's subscriptions register with its anchor's route, the pinned anchor's mailbox or the pool anchor's pool, never the program-wide queue; the route exists before the anchor's params are initialized and outlives every registration routed to it (each descendant deregisters in its dissolve, on the anchor's thread, and the join retires what is left before it destroys the mailbox)",
            "a pinned anchor's subtree initializes on the anchor's thread: the instantiating thread creates the route and the thread, the thread initializes the params (every nested construction, registration, birth and inline run(), a yield draining the anchor's mailbox) and reports ready, and the instantiating thread waits for it, servicing its own mailbox as a yield there would, before the literal completes; an override written at the literal is the instantiating code's and is evaluated there, and a locus it builds as the field's value is built on the anchor's thread; the init's temporaries are dissolved at its end, on the anchor's thread",
            "a pool anchor's subtree initializes on the pool's worker, as the anchor's first job there: the instantiating thread posts the params' initialization to the pool and waits for it as for a pinned anchor, and the worker runs every nested construction, registration, birth and inline run(), and the params' settle, a yield draining the pool's queue; the job never parks (on an async_io pool it runs on the worker's own stack), so the roots of one pool initialize in post order; a worker blocked on the instantiating thread's decision runs the posted initialization in place; the anchor's own birth() stays on the instantiating thread and its run() is posted behind the job",
            "the model's arrangement projects the table but covers less than the table, and the table answers every placement question: a literal `fn main` builds besides the root is not arranged (M-7: its birth is a hole of the model's dynamic births, the ownership family's legacy row, and arranging it moves with that row), nor is a row realizing a stdlib declaration (U-4: `Realizes` names an entry of `entities.loci`, which the hashed half renders, so arranging one moves `shape_hash`; carrying them is a model schema change of its own)",
            "F.38: placement is semantics-free, so a backend may Approximate it",
            "placement is a choice point: v1's declared placement is the single candidate",
            "the authored blocks are validated before they are rows: `check_pool_affinity` judges each `main locus`'s own `placement { }` block (an affinity on no named pool, two affinities for one pool), deployed or not, over the entry row's witness (`mains`), and that judgment is what lets the table keep one affinity per pool domain (rule 16); the per-block pool-to-affinity map it keeps is the block's well-formedness, not a placement fact, so it derives no row and no consumer reads it",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/placement_golden.rs (static instance rows, root, template bounds, domains and holes, plus the dynamic domain and hole projection read by consumers, over the corpus, tests/hale, DNA and placement fixtures; each seed by id, shared row groups by hash; HALE_SHADOW_REGEN regenerates)", "crates/hale-types/tests/placement.rs", "crates/hale-types/tests/placement_pairings.rs", "crates/hale-codegen/tests/pool_affinity.rs", "crates/hale-codegen/tests/placement_where_async_io.rs", "crates/hale-types/tests/placement_table.rs (the table through the frontend's load: the correspondence's coverage cases 1 to 16, the two universes joined into lowering's mint, the table's laws over every clean fixture, and a check that demands it once)", "crates/hale-types/tests/form_rows.rs (sync inference per instance, K-5)", "crates/hale-codegen/tests/nested_offthread_delivery.rs (the receiving thread of a nested subscriber, recorded inside its handler and inside its birth() and run(), in both devirtualization arms; the startup handshake, a nested child waiting during its anchor's initialization for a delivery through the anchor's route, or for a reply from a subscriber on the instantiating thread (and its control with that thread's drain off); a temporary of a pinned default, dissolved at the init's end; the same startup under a pool anchor (the review's program beside pinned, a child two levels down, two pool roots on one pool, an async_io pool, the round trip through main and its control, a worker blocked on a held failure running the init); the route's IR, for both threads, and its teardown under ASan)", "crates/hale-types/tests/intra_locus_pool_safety.rs (the intra-locus rewrite keeps a publish into a field off its owner's thread on the bus, through the lowering view)", "crates/hale-types/tests/model_arrangement.rs (the model's arrangement as the table's rows: M-1, M-3, M-5, M-7, M-9, U-4 and U-5, each against the old build's ids and hashes)", "crates/hale-types/tests/resource_budget.rs (the budget's rows R-1 to R-7 and checkpoint 4 through the frontend's load: replicas, one worker per pool, affinity, imported roots, constructions times their bounds and the uncertainty, adapters once, main never a pool, the dump's counted and not-counted lines)", "crates/hale-codegen/tests/placement_occurrences.rs (checkpoint 1: one template, one domain, its bound multiplying the budget; two live occurrences of one literal on two anchor threads, each nested subscriber on its own, in both devirtualization arms)"],
        spec: &["spec/semantics.md § Placement block (F.31)", "spec/decisions.md F.31, F.35, F.38", "spec/runtime.md § Placement classes (m28b: subscriptions follow the tower)"],
        owned: &[],
        seams: &[
            Seam { symbol: "derive_placement(", allowed: &[(PLACEMENT, 2), (SNAPSHOT, 1), (CHECK, 1), (TLIB, 2), (EFFECTS, 1), (SYNC, 1)] },
            Seam { symbol: "bundle_placement(", allowed: &[(PLACEMENT, 1), (LIFECYCLE_DERIVE, 1)] },
            Seam { symbol: "collect_main_placement(", allowed: &[(CG, 2)] },
            Seam { symbol: "route_anchors(", allowed: &[(TY_RESOLVED, 2)] },
            // The model's arrangement has one writer: the projection of
            // the table's rows (P1 part 4).
            Seam { symbol: "placed_in.push(", allowed: &[(MODEL_BUILDER, 1)] },
            Seam { symbol: "affined_to.push(", allowed: &[(MODEL_BUILDER, 1)] },
            // And one projection, read by the model and by lowering's
            // dispatch plan (C5): the builder, the snapshot's lowering
            // view and the bare program's (`resolve_program`).
            Seam { symbol: "project_arrangement(", allowed: &[(MODEL_BUILDER, 1), (SNAPSHOT, 1), (TY_RESOLVED, 1)] },
            // The authored blocks' validation: one definition and one
            // call, in the check.
            Seam { symbol: "check_pool_affinity(", allowed: &[(CHECK, 2)] },
        ],
    },
    Family {
        name: "target_capability",
        layer: Layer::Placement,
        state: State::Canonical,
        kind: Kind::Capability,
        answers: "What a target can lower and what it refuses: the wasm stdlib refusals, link refusals, per-site skips, async_io availability, FFI portability.",
        inputs: &["--target", "a source `target` declaration", "stdlib_surface", "FFI signatures"],
        producer: Some(site(CAPABILITY, "derive_capability_matrix")),
        legacy: &[],
        consumers: &[
            consumer("check"),
            consumer("build"),
            consumer_at("docs (systems/webassembly.md § What wasm32 can do, and spec/ffi.md's refused-namespace table: regions rendered from the cells)", CAPABILITY, "render_markdown"),
            consumer_at("the effective-target row, on the snapshot: the check's target and its conflict refusal, the editor's alike", SNAPSHOT, "demand_target"),
            consumer_at("hale build's backend and artifact naming, from the effective target", "crates/hale-cli/src/verbs/build.rs", "compile_target(row.effective)"),
            consumer_at("hale run and replay refuse a program whose effective target is wasm32", "crates/hale-cli/src/shared/options.rs", "refuse_unexecutable"),
            consumer_at("the use rows, on the snapshot: the check's input beside the row", SNAPSHOT, "demand_capability_uses"),
            consumer_at("the admission law: every use's cell for the effective target, in the check of every entry point", CHECK, "admission_diags"),
            consumer_at("hale check and hale build: every link input (--link, each package's [ffi] link) held to LinkLibrary before any tool, located at its manifest line or flag", "crates/hale-cli/src/shared/options.rs", "link_refusals"),
            consumer_at("a build handed link libraries (the harness, a library build): LinkLibrary read off the lowering view's column before lowering and before any tool", CG, "Capability::LinkLibrary"),
            consumer_at("lowering: every behaviour and obligation emitted or omitted per target, read off the lowering view's column (`LoweringView::cells`)", CG, "self.cells."),
            consumer_at("the wasm32 link: the module's fixed exports are ExportSurface's lowering data", CG, "Lowering::Exports(fixed)"),
            consumer_at("the check of an @ffi or @export signature: the FfiType cells for its ABI on the effective target", CHECK, "ffi_type_refusal"),
            consumer_at("the teardown spines: the pool join, wait-abort and ingress quiesce each spine owes, in the lifecycle plan's order", CG, "emit_teardown_obligations"),
        ],
        invariants: &[
            "Approximate is legitimate only in layers 5 and 7; everywhere else a target lowers or rejects, with the row's witness",
            "the docs' target statement is generated, never hand-maintained",
            "every (target class, capability), (target class, invocation) and (target class, obligation) pair has exactly one written cell: no default arm, the musl column written out like the others",
            "behaviours and obligations are distinct types: an obligation is omitted only on a premise (a Reject behaviour, a Refused invocation, a registered proof) that holds on its own target, and a Lower behaviour requires only Lower behaviours",
            "a capability refusal depends only on the program, the configuration and the target; a failure that depends on the machine running the compiler stays a toolchain error",
            "a target's class comes from its (arch, os, env), never from a triple's name; the effective target is a function of the snapshot key's configured target and its sources",
            "one effective target for analysis and emission alike (T1(b)): an explicit `--target`, else a written `target wasm`/`browser_js` declaration (wasm32), else the host; a `--target` of another class than a written declaration's is refused at the declaration, and a declaration `--wrap-main` injects never selects",
            "every use is read off resolved identities (a call, a method through its receiver's type, a cross-seed alias, a construction and the lifecycle it implies), never a `std::` spelling of the call; a type-only mention is not a use",
            "the program's own sources are the horizon: a use is refused once, at its first site in them, naming the capability, the target and its witness chain; a callee beyond it is refused at the call that crosses into it",
            "an unresolved requirement is never an admission on a target that rejects anything in its family: a hole is refused there and recorded elsewhere",
            "a policy refusal comes before any tool is probed: a link input the target refuses is refused by the check and by the build before clang, wasm-ld or zig is looked up, located at its input (T4)",
            "emission configuration through TargetSpec only, every capability through the matrix: codegen's remaining wasm-ness reads are the emission choices and the link path, and every behaviour and obligation it emits per target is a read of the lowering view's cells, which the view takes from the effective target and lowering refuses options of another class against",
            "the matrix selects a target's lifecycle obligations and the lifecycle plan orders them, alike in every teardown spine (the quiesce, then the wait-abort, then the pool join)",
            "the portable subset prints the same bytes natively and under node; the comparison proves agreement for its programs and admits nothing outside them",
        ],
        missing: Missing::Error,
        tests: &[
            "crates/hale-types/tests/wasm_target_gating.rs",
            "crates/hale-codegen/tests/wasm_target.rs",
            "crates/hale-cli/tests/target_model.rs",
            "crates/hale-cli/tests/wasm_package_csrc.rs (T4: a package's [ffi] link refused by check and build alike at the manifest's line; --link named as the flag; the refusal before any tool, on a PATH with no clang)",
            "crates/hale-cli/tests/target_precedence.rs (the precedence table: source x --target, check, build and the editor agreeing per cell; wasm-flower built for its declared target; run refusing a declared program; the agreement test: every wasm-relevant program's located refusals equal on check, build and the editor, with and without --target wasm32)",
            "crates/hale-types/src/capability/laws.rs (the matrix's laws: one cell per pair, anchored witnesses, premises, requires; KNOWN_OPEN empty since T2, T3 and T5 landed, a new entry held to today's answer)",
            "crates/hale-types/src/target.rs (async_io_follows_the_libc: TargetSpec::has_async_io is the runtime's shape, the AsyncIoPool cell what a program may ask for; they differ on wasm32 alone)",
            "crates/hale-types/tests/shadow_capability.rs (the checker rows against their cells over the corpus, tests/hale, the DNA seeds and the wasm programs, on three columns: 0 divergences)",
            "crates/hale-codegen/tests/shadow_capability_lowering.rs (the codegen rows, the thread behaviours and @ffi(\"js\") against their cells, both targets built, a harness build refused by the admission before lowering; 7 classified divergences, all the design's: 4 declared export-only programs and 3 declared @ffi(\"js\") programs a harness lowers natively anyway, where codegen still says `program has no fn main()` and the native link fails)",
            "crates/hale-cli/tests/shadow_capability_cli.rs (run, replay, record and --wrap-main against their cells; 0 divergences)",
            "crates/hale-types/tests/capability_uses.rs (the use producer's acceptance cases: a stdlib call, a construction with its lifecycle, a handle's method, a wrapper refused once, module-nested and on_failure bodies, a hole, declaration rows, an export-only program, an exported run(); T2's pinned, pool, async_io and transport refusals at the entry or binding; T3's stub namespaces; T5's @ffi(\"js\") on a native target, called or not; each type-only variant admitted)",
            "crates/hale-types/tests/capability_doc_matches.rs (both document regions equal the rendered matrix)",
            "crates/hale-codegen/tests/target_lifecycle_cells.rs (the five teardown spines on both targets, each owing what its target's cells select, in the plan's order)",
            "crates/hale-codegen/tests/portable_subset.rs (the design's programs and the playground's examples print the same bytes natively and under node; skipped, naming what is missing, without node, clang or wasm-ld)",
        ],
        spec: &["spec/decisions.md F.35", "spec/ffi.md § The `target` declaration + stdlib gating", "docs/src/systems/webassembly.md"],
        owned: &[site(CAPABILITY_USES, "derive_capability_uses"), site(CAPABILITY_USES, "admission_diags")],
        seams: &[
            // the definition and the document rendering, the laws, and
            // the use producer and the admission law
            Seam {
                symbol: "derive_capability_matrix(",
                allowed: &[
                    // the definition, the document rendering, the
                    // lowering view's column (an obligation, a behaviour)
                    // and the checker's FFI type cell
                    (CAPABILITY, 5),
                    (CAPABILITY_TRANSPORT, 1),
                    ("crates/hale-types/src/capability/laws.rs", 12),
                    (CAPABILITY_USES, 2),
                    // the target model's test, holding has_async_io to the cell
                    ("crates/hale-types/src/target.rs", 1),
                    // LinkLibrary, read before any tool by the CLI
                    ("crates/hale-cli/src/shared/options.rs", 1),
                ],
            },
            // the definition, the snapshot's family, and the two checks
            // of a bundle no snapshot holds
            Seam {
                symbol: "derive_capability_uses(",
                allowed: &[(CAPABILITY_USES, 1), (SNAPSHOT, 1), (CHECK, 1), ("crates/hale-types/src/lib.rs", 1)],
            },
            // the definition, the snapshot's family, and the check of a
            // bundle no snapshot holds (the tests' entries)
            Seam { symbol: "target_row(", allowed: &[(CAPABILITY, 1), (SNAPSHOT, 1), (CHECK, 1), ("crates/hale-types/src/lib.rs", 1)] },
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
        inputs: &[
            "placement (the instance templates: the static towers, replicas apart, and the dynamic sites with their domains and bounds; the domains)",
            "handler_routing (the owner's handler for each child, and whether it restarts)",
            "flows (an accepted flow is reclaimed at its run's end, a resident by its owner's cascade)",
            "bus_graph (which instances subscribe)",
            "the declarations each template realizes (run, the closures' epochs, birth_check, violate sites, accept, on_failure, the params bracket)",
            "the decision lines (`hale_types::lifecycle::DECISION_LINES`) and the inventory rows they name",
            "the runtime protocol (lotus_failure_hold / await / defer_reclaim; a run's retention, queued or started, lotus_run_cancel_only / lotus_run_cancel_queued and deferred storage release)",
        ],
        producer: Some(site(LIFECYCLE_DERIVE, "derive_lifecycle")),
        legacy: &[
            legacy(CG, "emit_deferred_entry_teardown", "the deferred teardown spine, and the eager one inline in `lower_locus_instantiation_inner` (a statement literal's), two copies of one order; #1208's pool join was added here after four other sites already had it. Each instance's own steps on both spines are the plan's (the spine law reads them), but across instances the two orders differ for a main locus's own pinned fields in a program with pools: the eager spine emits the main locus's head (quiesce, wait-abort, pool join) before joining them, the deferred one joins them first (C14's re-order), and the plan states the head before every field's drain (rule (b)); one order needs a ruling", "an explicit action plan (compiler- and runtime-owned actions with domain, prerequisites, liveness, completion) read by emission"),
            legacy(CG, "emit_frame_teardown", "the frame flush: at fn main's exits its pre-drain and the process rows after it are the exit spine's, read from the plan (`emit_flush_obligations`), and any other fn's flush pre-drains only; the entry order (subscription-less pinned first, then reverse push, a locus's own pinned fields pushed after it, C14) is still the code's, and stating it in the plan contradicts rule (b)'s edge from a deferred main locus's pool join to its pinned field's drain", "same"),
        ],
        consumers: &[
            consumer_at("the lowering view (the snapshot's plan, carried to the emitters)", SNAPSHOT, "view.lifecycle = Some(lifecycle)"),
            consumer_at("codegen (the process rows of every teardown spine, in the plan's order as the target's cells select them: a spine's head before its frame's pre-drain, the pre-drain and the rows after it in a fn main exit's flush)", CG, "emit_process_rows"),
            consumer_at("codegen (fn main's three exits, the fall-through, the test failure and `return`, through one helper: the exit spine's process rows, the frame, the process exit tail C24)", CG, "emit_main_exit"),
            consumer("codegen (emission reads the order, spine by spine, through `LifecyclePlan::spine`)"),
            consumer_at("codegen (the restart and the resume: __restart_<L>, __resume_<L>)", CG_RESTART, "define_restart_fns"),
            consumer("closures (the event alphabet)"),
            consumer("transitions (reserved)"),
            consumer("deployment (reserved)"),
        ],
        invariants: &[
            "equivalent parent execution contexts are represented once per owner (instantiating domain, queue domain, handler state); occurrence bounds still sum every owner, and domain claims retain every distinct context without enumerating ancestry paths",
            "handlers run only on the queue owner's thread, so cross-thread failure delivery follows spec/runtime.md (a typed bus message): the first named decision, with its own regression test",
            "a spec/implementation disagreement is settled as a named decision, never by extraction picking a side",
            "an obligation is keyed by its source site (the declaration and P1's construction template); the runtime mints the instance and its incarnation, the table never does",
            "every obligation ends in exactly one of its named terminal alternatives; lifetime (what stays alive until which event) and progress (what makes it reach a terminal) are separate fields",
            "each rule says whether it is shipped, adopted, known open at an inventory row, or pending on a named condition",
            "the runtime's protocol is checked by its trace, not trusted: a trace build reports the hold, the settle and each held delivery, and the trace oracle holds them to the plan's edges (delivered after the owner's settle, before its birth), with negative controls that remove or reorder a step and fail it, over the lifecycle fixtures, every runnable example and every cell of the lifecycle matrix",
            "a run posted to a pool ends in exactly one of decision line 19's named terminals (L5): it is retained against its child's teardown, whose Reclaim begins by canceling the child's queued runs (`lotus_run_cancel_only`, past the reclaim latch on every reclaim path, before storage release), so a run queued on any pool finds its child whole or ends NotStarted(Acknowledged), and physical release waits for the child's started runs, each holding its child and owned descendants from admission until it returns; a queued main-thread handler defers physical release until it returns, while drain and dissolve remain before replacement construction; child Reclaim completion precedes owner Reclaim completion; a post refused at shutdown ends NotStarted(Shutdown(PoolShutdown)), a cell the pools' teardown frees NotStarted(Shutdown(PoolTeardown)), and a run post that cannot allocate aborts; under replay the ordering gate drops a canceled run before comparing it with the recording and holds a live one without admitting it, in a hold buffer that is its consumer thread's and is freed at the thread's exit",
            "one plan per snapshot, over the placement table's instance templates: a held row and the rows projected under it are their source's and owe nothing of their own, a hole owes nothing, and a dynamic literal's own fields are instances of their field literals in its domains, a field literal reached through several constructions of its owner's declaration one template that keeps each one's owner, domains and bound (its bound their sum, a nested field's the product of its owner's with its own placement), and a body or accepted literal one template whose owners are every template of its enclosing locus, each occurrence on the domain its enclosing occurrence runs it on (its bound the table's, which covers every enclosing scope); no check builds the plan",
            "a failure's rows are guarded, one set per source an instance can raise (a birth-epoch closure, the birth_check, a violate in run(), in a handler or in drain(), a dissolve-epoch closure): its delivery in place, the held alternative where the owner's params can still be open, and the recovery decision and the restart, performed or refused under teardown; a path through the plan picks one",
            "existence, each edge and each domain claim carry their own rule and status, so a row shipped to exist can carry a claim known open (decision L0-1 at C36) or pending (line 3); a domain is claimed only where the placement table and the rule resolve one for every occurrence: one domain, or the set where a template's occurrences are built under parents on different domains (each occurrence held to one of them), never one parent's alone",
            "the plan the trace oracle holds a lifecycle fixture's or a matrix cell's run to is the producer's for its program, rendered along the run's path (`lifecycle::project::expected`): no hand-written expectation stands beside it, the negative controls' own plans aside and the hand-written plans of the fixtures the producer does not derive yet (`UNDERIVED`: mixed-instance cancellation and repeated main literals), and a run's known departures are named per inventory row in the known-open tables",
            "a row whose every terminal is a not-started one is owed by no subject: a restart refused under teardown (RD, C42), a run a resumed locus does not declare (line 13, C48)",
            "a pinned or pool anchor's params initialize and settle on its own domain (C49/C50); each static nested field's instantiating domain follows that initialization, while the pool anchor's own birth retains its caller's domain; a nested run inside a pool init is inline and owes no queue admission or queued-run cancellation",
            "an emitter reads its spine's obligations for an instance template from the plan (`LifecyclePlan::spine`, `birth_spine`, `birth_order`, `reclaim_order`, `cascade_order`), on the path no failure takes, each after every row its entry edges reach and ties in the producer's order; the steps it emits for a spine are exactly those, which the trace build shows per instance and spine over every fixture program with exact named departures in SPINE_KNOWN_OPEN (C29's alternate field-replacement spine remains to be derived), the Reclaim on the spine the plan holds it on and a run's Cancellation on its Reclaim's, read on the shutdown path (`shutdown_spine`)",
            "the process edges are the plan's (L4 3): the producer states every teardown spine's process rows (the eager main locus's C13, the deferred main entry's C19, `fn main`'s fall-through, test-failure and `return` exits C21–C23) as the ingress quiesce, the wait-abort and, with pools, the pool join with its progress, the exits then their frame's pre-drain (without pools the pre-drain before the wait-abort), and the edges between them; emission reads that order (`process_order`) and the capability matrix selects which rows a target emits (line 16); a run takes one of `fn main`'s exits, and a main locus a fn other than `main` builds owes its head before `fn main`'s exit",
            "a `Run` is owed exactly where lowering calls `run()` (C53): the producer and lowering read one test, `lifecycle::run_is_called` over `lifecycle::body_is_empty` (a body, or a flow's run, whose wrapper reclaims it), and a pinned locus's thread takes the step whatever the body; an empty `run() { }`, written or not, owes none",
            "the birth spine is emitted from the plan (L4 1): an instantiation's steps from params settle to the run's start (accept, registration, `birth()` with its birth-epoch closures and `birth_check`, readiness, the run's start) come in the order `birth_order` reads from the plan's rows and edges for its declaration, the registrations on the instantiating thread and the rest there or, for a pinned locus, on its thread; the emitter refuses a plan that puts a registration after the birth; a pinned locus's `birth_check` runs on its thread before `run()` (C38)",
            "readiness is the plan's (line 6): delivery to a subscriber is eligible once its birth and checks complete; what is published to it before is parked in order, never dropped (including a pinned subscriber, whose mailbox remains current for already-born nested subscribers), bounded at the queue's cap for a publisher on another thread, and dropped with the subscriber if it is quarantined first; no direct call delivers to it during its birth (a rewritten send keeps its exact receiver and uses its registered route while that receiver is held, and a baked direct publish uses runtime dispatch while any window is open)",
            "the reclaim spine is emitted from the plan (L4 2): every teardown funnels into `emit_locus_arena_destroy`, which emits an instance's reclaim in the order `reclaim_order` reads for its declaration: the owned children's reclaims before physical release (line 14), guarded logical entry, cancellation of still-queued runs, waiting for admitted runs and deferred descendants in the retained release callback (line 19, R19), the arena's release, then the struct's; the emitter refuses cancellation before entry, a wait before cancellation, or any storage release before its dependencies; a field owned through a pool-placed owner's params runs and subscribes on that pool (line 3, C12)",
            "the dissolve cascade is emitted from the plan (L4 2): an owner's teardown around its fields comes in the order `cascade_order` reads (the fields' drains, the owner's drain, its dissolve, the fields' dissolves, the reclaim), over the fields in the order `cascade_fields` reads (declaration order, line 12); a field typed by an interface or a perspective drains before its owner like any field (its slot holds the impl's drain and rest halves, C32), and a pinned locus's thread drains its fields before its own drain() (C9)",
            "restart and resume are emitted from the plan (L4 4): a restart's steps come in the order `recovery_order` reads from the plan's rows and edges for its declaration (the recovery decision once the handler has returned, the Restart after its completion (line RD), then the incarnation the restart begins: its rows owed once per incarnation, `birth()` with its birth-epoch closures, then `run()` only where the template declares one (line 13, C48)); a restart tears nothing down, the instance is the same one (C42); `__restart_<L>` emits the entry and the birth, the decision and the run stay with the caller on the spine that decides, and `__resume_<L>` starts a run only where the plan owes one (a flow's run end, its reclaim, aside). Departures the trace shows are named in SPINE_KNOWN_OPEN: a held run() failure's phase-0 resume the producer does not derive (C43), a held failure's restart performed by the resume at settle where the producer holds it on PoolRun (C42), and a restart asked for under the owner's teardown performed (C42, the adopted refusal's mechanism L5's)",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/src/lifecycle/derive.rs (shared ancestry regression)", "crates/hale-codegen/tests/reclaim_spine_ir.rs (reclaim ordering through retained callbacks, contract teardown halves, pinned field drains and declaration order)", "crates/hale-codegen/tests/birth_spine_ir.rs (the birth spine's IR per shape: a root child, a nested child, each replica, a pinned child with its birth_check before its run, a cross-pool bubble accepted before birth)", "crates/hale-codegen/tests/restart_spine_ir.rs (the restart and resume spines' IR per shape: a root child, a nested child, a pinned child, each replica, a pool-placed child, restart_in_place under a generic supervisor, a locus with no run() resumed without one)", "crates/hale-types/tests/lifecycle_plan.rs (a_restart_reads_decision_entry_birth_then_run)", "crates/hale-codegen/tests/topic_phase2.rs (a_births_own_publishes_are_delivered_after_it_in_order; sends directly in birth or in its helpers retain receiver identity and wait for readiness; pinned birth bursts and managed payloads run under ASan)", "crates/hale-types/tests/lifecycle_plan.rs (the plan through the snapshot: demanded once and built by no check; its laws over every corpus program the snapshot scopes, edges naming its own rows and acyclic on events, birth and teardown owed once per template, every line a decision line, every delivery to an owner of known domain naming its domain; the rows lines 1, 3, 7, 12, 13, 14, 18, 19 and RD are about; started-run retention through physical reclaim; statement-position subscribers torn down at frame exit)", "crates/hale-codegen/tests/lifecycle_flow.rs", "crates/hale-codegen/tests/reclamation_spine.rs", "crates/hale-codegen/tests/main_locus_deferred_pool_join.rs", "crates/hale-codegen/tests/teardown_pinned_join_order.rs", "crates/hale-codegen/tests/frame_flush_ir.rs (the frame flush per fn main exit, with and without a pool: the head's rows, then the pre-drain and the rows after it, the entries subscription-less pinned first then reverse push; another fn's flush pre-drains and aborts no wait)", "crates/hale-codegen/tests/reclaim_cancel_ir.rs (queued cancellation precedes the storage callback on every spine, and its hold wait dominates physical releases)", "crates/hale-codegen/tests/replay_canceled_run.rs (a canceled run replays clean; a held run keeps its retention; the hold buffer is freed by each thread that held, under ASan with no suppression)", "crates/hale-types/src/lifecycle.rs (the schema's laws: every decision line binds a kind, the Pending lines are the named ones, the doc table is the data)", "crates/hale-codegen/tests/lifecycle_fixtures.rs (a fixture per decision line under tests/fixtures/lifecycle/; KNOWN_OPEN pins today's outcome where it differs from the adopted one; the trace oracle holds each run to the producer's plan for the fixture's program, rendered along the run's path and its line's known-open rules, or, for a fixture in UNDERIVED, to its hand-written plan; TRACE_KNOWN_OPEN names today's departures, CONTROLS fail it; every_spine_emits_the_plans_obligations_in_order holds each instance's emitted steps on the eight instance spines (the deferred entry's and the deferred main entry's among them) and reclaim/cancellation on every spine to the plan's order)", "crates/hale-types/tests/lifecycle_plan.rs (every_declaration_reads_one_birth_order_over_the_corpus, the_reader_orders_each_spine_by_the_plans_edges)", "crates/hale-types/src/lifecycle/trace.rs (the trace's parser and oracle)", "crates/hale-codegen/tests/corpus_oracle.rs (corpus_traces_keep_the_lifecycle_laws: every runnable example, traced)", "crates/hale-codegen/tests/lifecycle_matrix.rs (failure phase × tree position × domain, a generated program per cell held to its outcome, the producer's plan for the program rendered along the cell's run, ASan on the sample and the let-bound differential; KNOWN_OPEN names today's failing cells; HALE_MATRIX=full runs every cell)", "crates/hale-codegen/tests/target_lifecycle_cells.rs (the five teardown spines on both targets: each owes what its target's cells select, in the order the plan's process rows state, before the cascade; the call counts pinned per shape, variant and target)"],
        spec: &["spec/runtime.md (failure delivery; pool join rule b)", "spec/runtime.md § Lifecycle obligations (the decision lines, adopted and shipped told apart)", "spec/runtime.md § The lifecycle trace (a debug aid, not a contract)", "spec/runtime.md § Lossless recording mode (the replay hold and a canceled run)", "spec/semantics.md § lifecycle"],
        owned: &[site(LIFECYCLE, "LifecyclePlan"), site(LIFECYCLE_TRACE, "Expected"), site(LIFECYCLE_PROJECT, "expected"), site(LIFECYCLE_SPINE, "birth_order"), site(LIFECYCLE_SPINE, "reclaim_order"), site(LIFECYCLE_SPINE, "cascade_order"), site(LIFECYCLE_SPINE, "cascade_fields"), site(LIFECYCLE_SPINE, "recovery_order")],
        seams: &[
            Seam { symbol: "derive_lifecycle(", allowed: &[(LIFECYCLE_DERIVE, 1), (SNAPSHOT, 1)] },
            // The process rows reach emission through one reader, and fn
            // main's exits through one helper.
            Seam { symbol: "process_order(", allowed: &[(LIFECYCLE_SPINE, 1), (CG, 1)] },
            Seam { symbol: "emit_main_exit(", allowed: &[(CG, 4)] },
            Seam { symbol: "recovery_order(", allowed: &[(LIFECYCLE_SPINE, 1), (CG_RESTART, 1)] },
        ],
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
        answers: "The laws that replaced lowering's own refusals of rules the spec states, and the one refusal still left: a cross-pool spawn used as a value in another locus's params default.",
        inputs: &["the AST", "the placement table", "the binding rows", "the ownership graph"],
        producer: Some(site(LOWERING_LAWS, "lowering_laws")),
        legacy: &[
            legacy(CG_INST, "CodegenError::Unsupported", "one spanless refusal left at lowering: the cross-pool spawn used as a value where the literal sits in another locus's params default (lowering expands the default under the instantiating locus's self and keys the bubble plan by it, so the plan entry the literal meets depends on who instantiates its locus)", "the cross-pool residue: a row giving each params-default literal its instantiation context, the locus whose self lowering expands it under"),
        ],
        consumers: &[consumer_at("the check (every verb and the LSP)", CHECK, "lowering_laws"), consumer_at("the harness's lowering view (`Config::harness`), which is not gated on the check", SNAPSHOT, "lowering_laws")],
        invariants: &[
            "a law is judged once, with a span",
            "lowering judges no shape a law in `lowering_laws` covers: the check runs the laws among its rules, and the harness's lowering view demands them before it lowers, so those refusals reach no entry point unlocated (C7)",
            "rule 6 is judged per pinned instance, by the locus it realizes (an override literal's, a stdlib locus's), over the placement table's rows: a `pinned` entry's field and each replica, and an adapter inline in `bindings { }` (C7, 1)",
            "rule 17 is judged per root construction over the placement table: a literal of the root declaration (as resolved) written inside a loop body, whose template holds a row a `pinned` entry decides (C7, 2)",
            "rule 18 is judged per entry of the lowering root (the placement table's root) over the inits its constructions supply, or the params default when one leaves the field or none builds the root (C7, 3)",
            "a cross-pool spawn is judged from the ownership graph's resolved birth rows and bubble plan: `bare_statement` records whether that literal is a discarded expression statement; a value use in a locus's own member bodies (fn defaults and birth checks included) is refused at the literal. The law does not rewalk literals or join by their written final segments (C7, 4; C3)",
            "self-containment (GH #813, #870) is judged over every locus's params defaults, keyed (locus, supplied fields) as lowering expands them, through every literal anywhere in a default (each branch of an `if` or `match`, each statement of a block) and every fresh-factory product: a cycle is refused at the param that closes it, and lowering keeps no re-entry guard (C7, 5)",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/placement.rs", "crates/hale-cli/tests/check_lowering_laws.rs (`hale check` and `hale build`)", "crates/hale-codegen/tests/harness_lowering_laws.rs (the harness, which skips the check)", "crates/hale-codegen/tests/deferred_slot_per_iteration.rs (rule 17 at the harness)", "crates/hale-codegen/tests/placement_factory_default.rs (rule 18 at the harness)", "crates/hale-types/tests/ownership_graph.rs (the cross-pool spawn law and its residue)", "crates/hale-types/tests/self_containing_locus.rs", "crates/hale-codegen/tests/self_containing_locus.rs"],
        spec: &["spec/semantics.md rules 6, 17, 18 and § accept bubbling", "spec/types.md § A locus may not contain itself by value; GH #813, #870, #876"],
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
            "the model builds none of the families it reads beside the program (2.3): the scope with its topic rows, the bus graph, the ownership graph, the handler rows, the effect rows (with the stdlib-merged summary their walk read), the form rows and the placement table arrive as `ModelInputs`, each demanded once from the snapshot over the checked programs; the allocation summary it still re-runs for itself is listed under its family",
            "the arrangement is the placement table's rows projected, under three identity contracts kept apart: shape identity (the arrangement is outside the shape half, so no change to it moves `shape_hash`); observation entity ids (they stamp subjects, locus declarations and the deployed root's bindings, never instances); and arrangement-instance correspondence (`LocusInstanceId` is the index in path order, stable across no change of the arrangement; a consumer joins instances by path, and a replica index is the replica row's own, never a descendant's)",
        ],
        missing: Missing::Hole,
        tests: &["crates/hale-types/tests/demand_gate.rs", "crates/hale-model/tests/architecture.rs", "crates/hale-types/tests/topology_projection.rs", "crates/hale-types/tests/model_arrangement.rs (the arrangement's three identity contracts, each pinned against the build before the switch)"],
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
        consumers: &[consumer("check / verify"), consumer_at("the check's laws stage (judged over the snapshot's model, after its typing stage)", SNAPSHOT, "demand_laws"), consumer("topology (law section)"), consumer("fleet"), consumer("dna (dna_law.rs wording)"), consumer("model diff")],
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
        ],
        consumers: &[consumer("every table"), consumer("the shadow facility (compares through an explicit correspondence, never raw id equality)"), consumer("lsp (a later incremental future)"), consumer("the resolved program (codegen's input is minted over the merged program)"), consumer_at("the model's test entry (a bundle nothing minted is minted over clones of its programs, since the arrangement is the placement table's rows)", TLIB, "derive_application_model")],
        invariants: &[
            "addresses are not identities (declarations are cloned); spans are not (the stdlib's coordinates overlap user files; desugars share spans)",
            "snapshot-local uniqueness and provenance are the requirement; persistent identity across editor revisions is a separate problem",
            "canonical ids need real equality and hashing; the AST's structural NodeId equality stays separate",
            "the identity's types are hale_graph::ids (SeedId, SiteId; phase 1.1a); hale_types::snapshot::mint numbers every site the AST walk hale_syntax::sites reaches with one counter: after the entry point's last desugar, and again, idempotently, in the resolved-program step over the merged program, numbering only what an earlier mint did not see (phase 1.1b); before the intra-locus rewrite that step only numbers the user program (`snapshot::number`: the rewrite moves a send's id onto its call and records it), which makes no snapshot of it; every entry point calls it after its last desugar with its source map and the bundle carries the result (every verb and the LSP through the snapshot's load, the test harness through `Snapshot::from_program`, with no source map, and the bare program's view, `resolved::resolve_program`, before its rewrite, so its correspondence has checked identities to join to); the lowering view mints with the bundle's seeds, the stdlib's sites under the named seed snapshot::STDLIB_SEED (its spans overlap the first file's); a generated site records the desugar that made it (Snapshot::origins); codegen's generic instantiation keeps the template's id, and the F.39 pre-pass numbers nothing: an unminted Struct or Call is an error",
            "every identifier expression is a `Use` site and every name a declaration with no site of its own binds (a fn's or a hook's parameter, a match pattern's binding, a tuple `let`'s name, a `shm_write` binding) a `Binder` site, their ids on their `Ident`s (use-site identity, phase 2)",
            "`binding_of` is one resolution per use, keyed by identity, never by name or span: the mint resolves each `Use` site, and each `Assign` site's head, to the `Let`, `For` or `Binder` site it names under the checker's scoping (`check::ScopeStack`), once per snapshot (the load's, the lowering view's, the bundled stdlib's analysis copy's once per process: `demand_gate` pins it); a use that names no local binding has no row; every reader asks `Snapshot::declaration_of` and resolves nothing itself, and every entry point mints (`check_program` too)",
            "a check of a bundle no entry minted (`Bundle::new` over parsed programs, a library caller's or a test's) mints a copy of its programs once, with its source map, before any family is derived (`with_identities`, the no-snapshot adapter of `check_bundle` and `check::check_bundle`): its bus graph's sends and the intra-locus rewrite's relation, whose own numbering keeps those ids, name a send by one id, so rule 10's join answers on that path as on the snapshot's; a send with no id reaching the join is refused as an internal failure naming the send, never judged as queued",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-codegen/tests/ownership_reclaim.rs (shadow_return_binding)", "crates/hale-codegen/tests/owner_table.rs", "crates/hale-types/tests/snapshot.rs (each_use_resolves_to_the_declaration_in_scope)", "crates/hale-types/tests/demand_gate.rs (each_snapshot_resolves_its_uses_once)", "crates/hale-syntax/tests/sites.rs"],
        spec: &["spec/decisions.md F.39, F.40"],
        owned: &[site(SITES, "SiteKind"), site(TY_SNAPSHOT, "resolve_uses"), site(TY_SNAPSHOT, "declaration_of"), site(TY_SNAPSHOT, "number")],
        seams: &[
            Seam { symbol: "mint(", allowed: &[(TY_RESOLVED, 2), (SNAPSHOT, 1), (TLIB, 2), (STDLIB_BODIES, 1), (ALLOC, 1), (SYNC, 1)] },
            Seam { symbol: "type_expr_identity(", allowed: &[(CHECK, 7)] },
        ],
    },
    Family {
        name: "demand",
        layer: Layer::Identity,
        state: State::Canonical,
        kind: Kind::Derivation,
        answers: "Which families a consumer's request computes, and in which order: a snapshot owns one load (the programs and their keys, each member's own program beside the merged one, the source map, the import renames, the config that shaped them, the sequence already run, the mint) and derives each family on request (`Snapshot::demand_*`: the scope, the checked programs' bus graph, ownership graph and handler rows, the effect rows, the model, the check in its two stages with the effects certificate report its typing produced, the lowering view), each at most once, blocking a family whose prerequisite reported errors.",
        inputs: &["seed_loading", "desugar_sequence", "snapshot_identity", "the config (target, api, api roles, environment, the check's rules)", "editor overlays (LSP)", "a consumer's request"],
        producer: Some(site(SNAPSHOT, "Snapshot")),
        legacy: &[],
        owned: &[site(SNAPSHOT, "demand_entry"), site(SNAPSHOT, "demand_scope"), site(SNAPSHOT, "demand_editor_scope"), site(SNAPSHOT, "demand_bus_graph"), site(SNAPSHOT, "demand_ownership_graph"), site(SNAPSHOT, "demand_handlers"), site(SNAPSHOT, "demand_effects"), site(SNAPSHOT, "demand_effect_certificates"), site(SNAPSHOT, "demand_model"), site(SNAPSHOT, "demand_typing"), site(SNAPSHOT, "demand_laws"), site(SNAPSHOT, "demand_check"), site(SNAPSHOT, "demand_lowering"), site(SNAPSHOT, "from_program"), site(SNAPSHOT, "SnapshotKey"), site(SNAPSHOT, "declarations"), site(SNAPSHOT, "declaration_dependents"), site(DEPENDENTS, "DependencyIndex"), site(SNAPSHOT, "reusing_typing"), site(SNAPSHOT, "typing_reuse"), site(TYPING_REUSE, "ReuseKey"), site(TYPING_REUSE, "plan")],
        consumers: &[
            consumer_at("check", V_CHECK, "demand_check"),
            consumer_at("the checker's rules (the handler rows: duplicate handlers, `@supervised`; the entry row: rule 1 and rule 9; the placement table: the F.31 rule per instance, the blocking check, pinned-in-a-loop, rule 20's construction paths; the bus graph: rules 7, 9 and 10; the ownership graph: rule 20), demanded before the check runs; the effect rows' purity column for a codec binding, demanded only when one reaches the assertion", SNAPSHOT, "CheckInputs"),
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
            consumer_at("lsp (the first publication: one snapshot per publish pass, its typing stage)", LSP, "demand_typing"),
            consumer_at("lsp (the typing stage reuses the seed's last typed snapshot, `State::typed`)", LSP, "reusing_typing"),
            consumer_at("lsp (the second publication: the whole check, for the files the laws add to)", LSP, "demand_check"),
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
            "the check is two stages and their composition (F.40 phase 3, X1): `demand_typing` is everything that needs no model — the scope's and the typing's diagnostics finished (`finish_check_diags`: the user's spelling, no repeat), then the build rules where the config asks (`Config::build_rules`: a build's and the editor's) and the allocation advisory where it asks (`Config::alloc_advisory`: the editor's; `hale check` runs its own beside its reports, under its flag); `demand_laws` is the laws judged over the model when the typing's own diagnostics denote one and the program has a claim surface, finished after the typing's own (`finish_check_diags_after`), and empty otherwise; `demand_check` is the typing stage's diagnostics followed by the laws', so every entry point's set is one pass's over both by construction, and only the position of a build rule or an advisory relative to a law differs from the single pass it replaced (both stages before; the laws last). Each stage is counted once (`Snapshot::builds`, the `STAGES` beside the families)",
            "the editor publishes the two stages apart (X1, `check_and_publish`): the first publication is every file of the seed with the typing stage, placed as `hale check` places it (each diagnostic spelled, suppressed — `retain_owned_advisories`, a stdlib-origin span — and placed on its own), with every file the seed's last publication covered and this one does not published empty (`State::published`); the second is the whole check (`demand_check`, the laws after the typing), sent only for the files whose list it changes, so a file's final list is its first followed by the laws placed in it, every file's last publication is `hale check`'s for it, and a seed with no broken law gets one publication; a seed the snapshot does not check (a refused load, a hole, the stdlib cache) gets its one. Before each publication, and before the laws are judged, the pass asks whether a document event is queued behind it: if one is, the buffers it read are superseded, what is unsent is discarded with the rest of the pass, `published` keeps only what was sent, and the pass's files join the next pass (`State::pending`)",
            "a prerequisite runs once: every family is a `OnceCell` of its snapshot, and a family that reads another demands it rather than building its own; `Snapshot::builds` counts each family's demands on the snapshot (its producer's runs in the snapshot's own cell), and no count exceeds one on any consumer on the snapshot. It does not count what is rebuilt outside those cells: the lowering view's `resolve_rewritten` builds its own handler rows (the resolved program's reference in the `handler_rows(` seam, handler_routing). Its scope is the snapshot's, and its ownership and bus graphs are the snapshot's rows read through the view's correspondence with the stdlib's after them (F.40 phase 3, C5), so on a build path the scope and each graph are derived once per snapshot, as `builds()` says",
            "a family nobody requested is not computed: the no-claims editor path builds no model (GH #476 criterion 1); the bus graph (rules 7, 9 and 10) and the ownership graph (rule 20) it builds are the check's, once each",
            "the model's inputs are families (2.3): `demand_model` demands the scope, the bus graph, the ownership graph, the handler rows and the effect rows over the checked programs (the `bus_graph`, `ownership`, `handler_routing` and `effects` counts), each once; lowering's graphs are the lowering view's own, over the resolved program, until the check runs over it",
            "the checker's inputs are families (2.3): the typing demands the handler rows, the entry row and the ownership graph before the checker runs and hands them in (`CheckInputs`), so a check with a law builds the handler rows and the ownership graph once for the checker and the model together; the three producers read declarations and bodies, not types, so they are total over a program that does not typecheck",
            "a family whose prerequisite reported errors is `Blocked { family, because }`, not computed: an editor seed with a member that did not parse or would not read has no scope (the editor's requests read `demand_editor_scope`, the scope over the members that parsed with the hole named, and never a scope of their own), a program that does not typecheck has no model, a program whose check reported an error has no lowering view; a ready result may still hold typed holes",
            "the lowering view (`LoweringView`, the `lowering_view` count) is a family: `demand_lowering` demands the check and the lifecycle plan (which it carries to the emitters, `LoweringView::lifecycle`), then runs `resolve_rewritten` once over the intra-locus stage (`demand_intra_locus`, the `intra_locus` count, which the check demanded too) with the snapshot's identities, source map, renames and api config; `build_resolved` reads it by reference; the view's correspondence (`hale_types::correspondence`, F.40 phase 3, C5) places every site of the merged program it lowers as a checked site (the same index, kind and span), the call the intra-locus rewrite put in a send's place (the send, by the rewrite's relation) or the stdlib analysis copy's site in the same walk position, and every checked site as the image of one merged site or the subject of a send a rewrite erased (`TopicRewrite::erased`, `IntraLocusRewrite::erased`); a site it cannot place refuses the view, and the law holds over the corpus, `tests/hale` and the DNA seeds (`lowering_correspondence`); every build path (build, run, test, replay, bench) demands it from a `Snapshot::load`, and the test harness from `Snapshot::from_program`, whose config (`Config::harness`) does not gate lowering on the check",
            "a changed entry, load mode, target, config, overlay or source text is a distinct snapshot (`SnapshotKey`, computed after the load from what it read; a bare program is its own load); two snapshots share no result but the typing reuse below, which a second key names",
            "the editor's typing stage is incremental by declaration and equals the full one by construction (F.40 phase 3, X2, `Snapshot::reusing_typing`, `hale_frontend::typing_reuse`): the snapshot key stays the whole seed's; a second key (`ReuseKey`: the entry, the load mode, the target, the config digest, the import renames, one row per alias and declaration in the load's stable order) names the reuse, and a previous snapshot whose second key differs, or whose typing never ran, offers nothing. The check's per-declaration passes (the checker's walk of a top-level declaration and the reveal rule's, `check::check_bundle_by_declaration`, `DeclChecked`) keep their result per declaration; a new snapshot of the seed reuses it for every declaration that is unchanged (the previous one moved by the distance its start moved is the new one, position for position), is no dependent of a changed one through the families (`declaration_dependents`, in the previous snapshot and the new one) and has no diagnostic position outside itself, moved the same distance, and runs the passes for the rest; a changed declaration is one whose bodies alone changed (equal once positions are erased and bodies set aside), and any other change (a declaration added, removed, renamed or reordered, an edit to what a declaration declares, a changed declaration the families do not place, a changed set of programs) checks the seed whole (`TypingReuse::Whole`, with the reason). Everything else the typing stage runs (the scope, the bundle-wide rules, the build rules, the advisory) runs whole, so the incremental stage's diagnostics are the full stage's, ordered and deduplicated as one pass leaves them; the laws stage stays whole. Reuse is opt-in and only the LSP opts in (`State::typed`, one typed snapshot per seed, kept across a hole), so `hale check` runs the whole check. The check records what it typed as it walks, so a typing that reused a declaration holds no record of its bodies, and the typed-body table of such a snapshot (`demand_typed_bodies`) is packaged from a whole check; the editor never lowers",
            "diagnostic spellings use one indexed rename table (`stdlib_bodies::Demangler`) per snapshot (`Snapshot::demangler`, F.40 phase 3, X3); import rows occur once per alias and declaration in a stable load order, and the indexed demangler preserves longest-name-first row application, including overlapping names, alias precedence and replacements that create a later row's name. The editor reuses this index for typing and final diagnostics; flow displays, model absorption displays and certificate evidence use the same demangler",
            "the bundle is a borrowed view (`Snapshot::bundle`), built per call, never stored",
            "a declaration's dependents are the families' (F.40 phase 3, X2, `Snapshot::declaration_dependents`, `hale_frontend::dependents`), never the text's: the unit is a top-level declaration of a snapshot program (`Snapshot::declarations`, a module one, named by its own minted site); for a fn or a locus the answer is itself, every declaration whose resolved calls reach it (the callgraph whose targets the effect rows copy, read from the allocation summary's call edges: the rows' and the declaration bodies', which are every body the reveal rule reads that is no row, each its declaration's own; a call to a fn with no row by its bare name; its readers closed transitively), and its neighbours in the ownership graph (births, accepts, instantiations), the bus graph (one subject's publishers, subscribers and topic), the placement table (what an instance or a dynamic site realizes, where its literal sits, its owner's and its enclosing scope's, joined by site) and the flow rows (a flow child and its `release` owners); a name joins every declaration that declares it, so a shared name answers both; a placement hole that leaves its literal's declaration unresolved makes the declaration it sits in a dependent of every one, and so does a call the summary records no edge for (by its span, over the site walk: a call in a position no summary body covers, today a closure's clauses and a type's field defaults, neither of which the reveal rule reads); any other declaration, one with no site, and a site that names none is `Dependents::Whole`. The relation's domain is an edit to a declaration's bodies: over the example corpus, every body edit of a mutation (emptied, an ill-typed `let`, a print, a call added) re-derives typing diagnostics only in declarations the relation names before the edit or after it, and an edit to a declared surface reaches a reader no family names, which is why such an edit is the seed's",
        ],
        missing: Missing::Error,
        tests: &["crates/hale-types/tests/demand_gate.rs (a_build_derives_the_scope_and_each_graph_once_lowering_included: the lowering view derives no scope or graph of its own, F.40 phase 3, C5)", "crates/hale-types/tests/declaration_dependents.rs (a_declarations_dependents_cover_what_a_fresh_check_rederives, an_edit_to_a_declared_surface_escapes_the_families)","crates/hale-cli/tests/lsp.rs (the_check_is_its_typing_stage_followed_by_its_laws_stage, lsp_and_check_agree_over_a_seed_with_laws, lsp_publishes_the_typing_stage_first_and_the_laws_replace_it, the_incremental_typing_stage_is_the_full_one, the_incremental_typing_stage_is_the_full_one_over_the_dna_host, the_incremental_typing_stage_is_the_full_one_through_handler_and_initializer_calls, lsp_publishes_what_check_reports_after_a_helper_edit_through_handler_and_initializer_calls)", "crates/hale-cli/tests/lsp_latency.rs (lsp_latency_first_and_final_publication: the editor's latency, first and final publication apart; a measurement, ignored by default)", "crates/hale-frontend/src/snapshot.rs (a_changed_input_is_a_distinct_snapshot_and_shares_no_result, a_file_that_does_not_parse_blocks_the_scope_and_its_dependents, the_editor_scope_covers_the_members_that_parsed_and_names_the_hole, a_member_that_will_not_read_blocks_the_scope, a_program_that_does_not_typecheck_blocks_the_model, a_check_with_errors_blocks_the_lowering_view, the_lowering_view_is_resolved_once_after_the_check)", "crates/hale-lsp/src/lib.rs (a_request_builds_only_the_families_it_reads, a_request_over_a_seed_with_a_hole_answers_from_the_members_that_parsed, the_laws_replace_the_typing_stage_unless_a_newer_event_supersedes_them, a_pass_reuses_the_seeds_last_typed_snapshot)"],
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
        gist: "pinned-class restrictions (no accept(), no closure whose epoch is birth or dissolve, the default) on every pinned instance, a placement entry's or an adapter binding's",
        family: "placement",
        evaluator: Some(site(LOWERING_LAWS, "pinned_features")),
        state: State::Canonical,
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
        evaluator: Some(site(LOWERING_LAWS, "pinned_root_in_a_loop")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/18",
        gist: "every placement entry is consumed exactly once",
        family: "placement",
        evaluator: Some(site(LOWERING_LAWS, "placement_entry_consumed")),
        state: State::Canonical,
    },
    Rule {
        id: "semantics/placement/19",
        gist: "a bus payload is carriable",
        family: "bus_graph",
        evaluator: Some(site(CHECK, "check_bus_payload_carriable")),
        state: State::Migrating,
    },
    Rule {
        id: "semantics/placement/20",
        gist: "a subscriber born in a bus handler is owned",
        family: "ownership",
        evaluator: Some(site(CHECK, "check_unowned_subscriber_locus")),
        state: State::Canonical,
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
    DebugScan { path: CHECK, fragment: "format!(\"{:?}\", p)", count: 2, verdict: ScanVerdict::Decides { family: "snapshot_identity" } },
    DebugScan { path: CHECK, fragment: "format!(\"{:?}({})\", class, type_expr_text(inner))", count: 1, verdict: ScanVerdict::Renders },
    DebugScan { path: CHECK, fragment: "format!(\"{:?}({})\", class, type_expr_identity(inner, known))", count: 1, verdict: ScanVerdict::Decides { family: "snapshot_identity" } },
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
    // The totals are not rendered: a count line that every change to any family
    // moves made two unrelated registry changes conflict textually on one line at
    // every rebase. `registry_is_well_formed` counts the families, their legacy
    // producers, the rules and the frozen Debug-string sites in code.
    o.push_str(
        "The families, their legacy producers, the spec rules and the frozen Debug-string sites \
         are counted by `registry_is_well_formed`, not here: a rendered total moved with every \
         change to any family, so two unrelated changes conflicted on one line at every rebase.\n\n",
    );
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
