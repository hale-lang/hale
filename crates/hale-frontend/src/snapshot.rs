//! The snapshot (F.40 phase 2.2): what a check needs, loaded once, and
//! the families derived from it, each computed on demand at most once.
//!
//! A [`Snapshot`] owns the loaded programs and their keys, the source
//! map, the import renames, the [`Config`] that shaped them, the desugar
//! sequence already run and the identities minted after it. Everything
//! else is a family, demanded by name:
//!
//! - [`Snapshot::demand_entry`]: the entry row, which `main locus` is
//!   the program's entry, by identity.
//! - [`Snapshot::demand_scope`]: the top scope, with its topic rows.
//!   [`Snapshot::demand_editor_scope`] is the editor's reading of it:
//!   over a seed with a hole, the scope of the members that parsed,
//!   with the hole named.
//! - [`Snapshot::demand_bus_graph`], [`Snapshot::demand_ownership_graph`]
//!   and [`Snapshot::demand_handlers`]: the bus graph, the ownership
//!   graph and the handler rows over the checked programs, what the
//!   model reads beside the scope. The checker reads the handler rows
//!   too, demanded before it runs.
//! - [`Snapshot::demand_alloc_summary`]: the allocation summary, one
//!   over the checked programs and the stdlib's analysis copy, which
//!   the check's effects certificate engine and the effect rows read.
//! - [`Snapshot::demand_effects`]: the effect rows, one fixpoint over
//!   that summary.
//! - [`Snapshot::demand_placement`]: the placement table, which thread
//!   domain each instance of the deployed root's tower runs in.
//! - [`Snapshot::demand_lifecycle`]: the lifecycle plan, the obligations
//!   each instance template and the process owe, over the placement
//!   table, the handler rows, the flow rows and the bus graph.
//! - [`Snapshot::demand_model`]: the application model, over the scope
//!   and those three.
//! - [`Snapshot::demand_typing`]: the check's first stage, everything
//!   that needs no model — the scope's and the typing's diagnostics,
//!   then the build rules and the allocation advisory where the config
//!   asks for them.
//! - [`Snapshot::demand_laws`]: the check's second stage — the laws
//!   judged over the model, when the program declares any and denotes a
//!   model.
//! - [`Snapshot::demand_check`]: what the checker reports, the
//!   composition of the two stages: the typing's diagnostics followed by
//!   the laws', ordered and deduplicated as one pass over both would
//!   leave them.
//! - [`Snapshot::demand_typed_bodies`]: what the typing's check typed,
//!   keyed by declaration identity: the table lowering reads instead of
//!   typing again.
//! - [`Snapshot::demand_lowering`]: the view codegen lowers
//!   ([`LoweringView`]), after a check that reported no error.
//!
//! Four contracts hold, and `crates/hale-types/tests/demand_gate.rs`
//! pins each:
//!
//! 1. A prerequisite runs once: every family is a `OnceCell`, and a
//!    family that reads another demands it rather than building its
//!    own. [`Snapshot::builds`] counts each family's producer runs.
//! 2. A family nobody demands is never computed: a program with no
//!    claim surface checks without a model.
//! 3. A family whose prerequisite reported errors is [`Blocked`], not
//!    computed: a seed with a file that did not parse has no scope, a
//!    program that does not typecheck has no model, and a program whose
//!    check reported an error is not lowered.
//! 4. A changed entry, target, config or overlay is a different
//!    snapshot ([`SnapshotKey`]); two snapshots share no result.
//!
//! [`Snapshot::bundle`] is a borrowed view over the owned programs,
//! built for the call that needs it and never stored.

use std::cell::{Cell, OnceCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hale_graph::ids::SiteId;
use hale_model::ApplicationModel;
use hale_syntax::api_gen::ApiSurface;
use hale_syntax::ast::{Import, Program};
use hale_syntax::Diag;
use hale_types::alloc_summary::AllocSummary;
use hale_types::capability::uses::CapabilityUses;
use hale_types::capability::TargetRow;
use hale_types::binding_rows::BindingRows;
use hale_types::bus_graph::BusGraph;
use hale_types::effect_rows::EffectRows;
use hale_types::effects::EffectCertificates;
use hale_types::entry::EntryRow;
use hale_types::form_rows::FormRows;
use hale_types::handler_routing::HandlerRouting;
use hale_types::lifecycle::LifecyclePlan;
use hale_types::ownership_graph::OwnershipGraph;
use hale_types::placement::PlacementTable;
use hale_types::resolve::TopScope;
use hale_types::resolved::{IntraLocusStage, LoweringView};
use hale_types::symbol::SourceFile;
use hale_types::typed_bodies::{TypedBodies, TypingRecord};
use hale_types::Bundle;

use crate::frontend::{
    collect_ap_files, link_checkable, merge_programs, parse_checkable, seed_dir_of, source_map,
    CheckableFailure, LoadMode,
};
use crate::dependents::{Declaration, DependencyIndex, Dependents};
use crate::typing_reuse::{Plan, ReuseKey, Side, TypingReuse};
use crate::imports::ImportRenames;
use crate::source::SourceProvider;

/// The families a snapshot produces, in the order a build demands them.
/// The names are the registry's (`spec/registry.md`). `bus_graph`,
/// `ownership` and `handler_routing` count the checked programs' graphs,
/// the model's inputs; `intra_locus` is the intra-locus rewrite, whose
/// relation the check reads (rule 10) and whose program lowering
/// continues from; `lowering_view` is the `demand` family's own, the
/// view over the resolved program whose tables are lowering's ownership,
/// bus-graph, dispatch and handler-routing rows. Until the check runs
/// over the resolved program, a snapshot that is checked for its model
/// and lowered holds both shapes' graphs. `target_capability` counts the
/// effective-target row, which no consumer demands yet; `sync_inference`
/// counts the form rows ([`Snapshot::demand_forms`]); `typed_bodies` the
/// table the typing's record is packaged into
/// ([`Snapshot::demand_typed_bodies`]): `expression_typing`'s answers,
/// carried to lowering.
pub const FAMILIES: [&str; 21] = [
    "seed_loading",
    "desugar_sequence",
    "snapshot_identity",
    "entrypoint",
    "target_capability",
    "top_scope",
    "bindings",
    "sync_inference",
    "expression_typing",
    "typed_bodies",
    "bus_graph",
    "ownership",
    "handler_routing",
    "alloc_summary",
    "effects",
    "placement",
    "lifecycle_order",
    "model",
    "claims",
    "intra_locus",
    "lowering_view",
];

/// The check's two stages ([`Snapshot::demand_typing`],
/// [`Snapshot::demand_laws`]), counted by [`Snapshot::builds`] beside
/// the families: a stage is the `demand` family's composition of
/// families, not a family of the registry's.
pub const STAGES: [&str; 2] = ["typing_stage", "laws_stage"];

/// The target a snapshot is configured for: `--target`, or the host
/// when nothing names one. The effective-target row
/// ([`Snapshot::demand_target`]) reads it beside the source
/// declaration, and the check, the build and the editor act on the
/// row's effective target.
pub use hale_types::capability::ConfiguredTarget as Target;

/// A deployment environment the program is checked for: an
/// `[environments.<name>]` of `hale.toml` (GH #409).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub name: String,
    /// The constitutions it requires, adopted into the main locus's
    /// `claims` block as if the source had written `adopt C;`.
    pub adopt: Vec<String>,
}

/// What shaped a snapshot's programs, and the rules its check applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub target: Target,
    /// The `--api` entry the desugar sequence injects, if any.
    pub api: Option<String>,
    /// The roles the environment binds, baked into the api binding.
    pub api_roles: Option<String>,
    pub environment: Option<Environment>,
    /// The whole-program rules (F.18, GH #721): a bare callee or
    /// identifier nothing binds is an error. On for a whole seed, off
    /// for one file of a seed, which reads its siblings' names.
    pub whole_program: bool,
    /// `--allow-unowned-subscriber`.
    pub allow_unowned_subscriber: bool,
    /// `--wrap-main` (the browser playground's wasm build): the load
    /// wraps a bare `fn main` as the wasm `@export` entry before
    /// anything else shapes the program.
    pub wrap_main: bool,
    /// The rules a build refuses beside the check (the borrow rule,
    /// `hale_types::build_rule_diags`), appended to the check's
    /// diagnostics, so they block lowering. `hale check` runs them
    /// itself, beside its reports. Part of the typing stage.
    pub build_rules: bool,
    /// The allocation advisory (`hale_types::unbounded_alloc_warnings`,
    /// every site surveyed), appended to the typing stage after the
    /// build rules. The editor's: `hale check` runs it itself, beside
    /// its reports, under its own `--no-warn-unbounded-alloc`.
    pub alloc_advisory: bool,
    /// Whether the lowering view waits for a check with no error. On
    /// for every entry point; off only for the test harness's snapshot
    /// ([`Config::harness`]), which lowers what it is handed.
    pub check_gates_lowering: bool,
}

impl Config {
    /// `hale check`'s: the host, no api, no environment.
    pub fn check(whole_program: bool, allow_unowned_subscriber: bool) -> Self {
        Config {
            target: Target::host(),
            api: None,
            api_roles: None,
            environment: None,
            whole_program,
            allow_unowned_subscriber,
            wrap_main: false,
            build_rules: false,
            alloc_advisory: false,
            check_gates_lowering: true,
        }
    }

    /// A build's, for `target`: a build compiles exactly what it
    /// loaded, so it holds a whole program (GH #721), and it refuses
    /// the build rules. The caller sets the api, the roles and the
    /// environment its flags name.
    pub fn build(target: Target) -> Self {
        Config {
            target,
            api: None,
            api_roles: None,
            environment: None,
            whole_program: true,
            allow_unowned_subscriber: false,
            wrap_main: false,
            build_rules: true,
            alloc_advisory: false,
            check_gates_lowering: true,
        }
    }

    /// The test harness's (codegen's `build_executable_with_options`,
    /// over [`Snapshot::from_program`]): a build's config whose
    /// lowering is not gated on a check. The harness runs no checker —
    /// a test that wants the check calls it itself, and the agreement
    /// sweep (`corpus_check_build_agreement`) compares the two — so its
    /// snapshot lowers what it is handed, once the laws that replaced
    /// lowering's own refusals and target admission pass
    /// ([`hale_types::lowering_laws`], [`hale_types::capability::uses`]). The
    /// check is still a family of it, computed only when demanded.
    pub fn harness(target: Target) -> Self {
        Config { check_gates_lowering: false, ..Config::build(target) }
    }

    /// The LSP's: `hale check <dir>`'s report. It checks a seed only
    /// once every member of it read and parsed, so it holds a whole
    /// program (GH #721), and its check carries the build rules `hale
    /// check` runs beside its own (the borrow rule) and the allocation
    /// advisory, so the editor shows every finding the CLI prints, all
    /// of them before the laws.
    pub fn editor() -> Self {
        Config { build_rules: true, alloc_advisory: true, ..Config::check(true, false) }
    }

    fn digest(&self) -> u64 {
        let mut d = Digest::new();
        d.field(self.target.name.as_bytes());
        // The spec's identity beside the name, so two configured
        // targets that share a name never share a digest (design §1.7).
        d.field(self.target.spec.arch.llvm_name().as_bytes());
        d.field(self.target.spec.os.name().as_bytes());
        d.field(self.target.spec.env.name().as_bytes());
        d.flag(self.target.spec.has_async_io());
        d.field(self.target.spec.platform_label().as_bytes());
        // Whether `--target` named it: an explicit target overrides a
        // source declaration, the host fallback does not (T1(b)).
        d.flag(self.target.explicit);
        d.option(self.api.as_deref());
        d.option(self.api_roles.as_deref());
        match &self.environment {
            None => d.flag(false),
            Some(env) => {
                d.flag(true);
                d.field(env.name.as_bytes());
                d.count(env.adopt.len());
                for c in &env.adopt {
                    d.field(c.as_bytes());
                }
            }
        }
        d.flag(self.whole_program);
        d.flag(self.allow_unowned_subscriber);
        d.flag(self.wrap_main);
        d.flag(self.build_rules);
        d.flag(self.alloc_advisory);
        d.flag(self.check_gates_lowering);
        d.finish()
    }
}

/// What identifies a snapshot. Two snapshots with different keys were
/// loaded from different inputs, and share no result.
/// FNV-1a/64 over whatever the key hashes.
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotKey {
    /// The target the load started from, canonical where it exists.
    pub entry: PathBuf,
    /// How the target was loaded (the CLI's whole seed, or the
    /// editor's, which records a member it cannot load instead of
    /// failing); `None` for a bare program.
    pub mode: Option<LoadMode>,
    pub target: String,
    pub config_digest: u64,
    /// The editor buffers the load read over the disk; the disk alone
    /// is [`SourceProvider::overlay_digest`]'s zero.
    pub overlay_digest: u64,
    /// What the load actually read: every source unit's path and text
    /// (imports included). A bare program has no source, so its key
    /// carries the ordinal of its handoff instead: every
    /// [`Snapshot::from_program`] is its own load. Two loads that read
    /// different programs never share a key (outside review of #1283,
    /// finding 2).
    pub sources_digest: u64,
}

/// A family that was not computed because a prerequisite reported
/// errors: which family, and the errors.
#[derive(Debug, Clone)]
pub struct Blocked {
    pub family: &'static str,
    pub because: Vec<Diag>,
    /// What blocked it with no position to report it at: the lowering
    /// view's own producer refusing (`resolve_rewritten`: a bundled
    /// stdlib that does not parse, a site the mint left unnumbered), or
    /// the editor's seed members that would not read, one
    /// [`unreadable_message`] each. `None` when every reason is a
    /// diagnostic in `because`.
    pub refused: Option<String>,
}

/// What the editor reports for a seed member that would not read:
/// `seed member <file name>: <the OS error>`, the file's name and the
/// error `hale check` prints for it.
pub fn unreadable_message(path: &Path, os_error: &str) -> String {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    format!("seed member {name}: {os_error}")
}

/// Why a snapshot could not be made.
pub enum LoadError {
    /// The load failed (`hale check`'s load): an input that would not
    /// read, an import that did not resolve, a file that did not parse.
    /// Rendered by [`CheckableFailure::report`].
    Load(CheckableFailure),
    /// The config asks for what the program cannot carry: an
    /// environment for a seed with no main locus, an `--api` entry with
    /// nowhere to go.
    Refused(String),
}

/// What the check reports: the resolver's and the checker's
/// diagnostics, then the judged laws, in the user's spelling with no
/// repeats.
#[derive(Debug, Clone)]
pub struct Checked {
    pub diags: Vec<Diag>,
}

/// The typing's record: what the resolver and the checker report, the
/// effects certificate report, what the check typed (the record the
/// typed-body table is packaged from, [`Snapshot::demand_typed_bodies`]),
/// the per-declaration passes' results per declaration (what a later
/// snapshot of the seed may reuse), and what this one reused.
struct Typed {
    diags: Vec<Diag>,
    certificates: EffectCertificates,
    record: TypingRecord,
    by_decl: hale_types::check::ByDeclaration,
    reuse: TypingReuse,
}

/// The top scope and what building it reported.
struct Scope {
    top: TopScope,
    diags: Vec<Diag>,
}

/// The scope the editor answers a request from
/// ([`Snapshot::demand_editor_scope`]): the snapshot's top scope, and
/// the members it does not cover.
pub struct EditorScope<'a> {
    pub top: &'a TopScope,
    /// The seed's members that are not in `top` — they did not parse
    /// or would not read — sorted. Empty for a whole seed, whose `top`
    /// is [`Snapshot::demand_scope`]'s.
    pub hole: Vec<&'a Path>,
}

/// One load's inputs, and the families derived from them.
pub struct Snapshot {
    /// The environment this snapshot's claims are checked for and its
    /// artifact is labelled with; bound around each demand and each
    /// serialization by [`Snapshot::with_env`].
    env: hale_types::claims::EnvBinding,
    key: SnapshotKey,
    config: Config,
    files: Vec<PathBuf>,
    own_files: BTreeSet<PathBuf>,
    /// Each of the seed's own files as it parsed (at its base), before
    /// the merge and the sequence: what the file itself declares.
    members: BTreeMap<PathBuf, Program>,
    programs: BTreeMap<PathBuf, Program>,
    sources: BTreeMap<PathBuf, String>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    import_renames: ImportRenames,
    /// [`Snapshot::demangler`].
    demangler: OnceCell<hale_types::stdlib_bodies::Demangler>,
    /// The target's own `import`s, as written.
    entry_imports: Vec<Import>,
    source_map: Vec<SourceFile>,
    identities: hale_types::snapshot::Snapshot,
    /// The api surface the sequence generated a binding for, if any.
    api_surface: Option<ApiSurface>,
    /// The files that did not parse, with their diagnostics (bundle-
    /// global spans). A load that leaves any blocks the scope.
    unparsed: BTreeMap<PathBuf, Vec<Diag>>,
    /// The seed members that would not read, with the OS error. A load
    /// that leaves any blocks the scope.
    unreadable: BTreeMap<PathBuf, String>,
    /// What the import graph refused for the editor's seed whose every
    /// member parsed (an import that does not resolve, a library that
    /// does not parse or read): the members are kept, unlinked, and the
    /// scope blocks.
    unlinked: Option<CheckableFailure>,
    entry: OnceCell<Result<EntryRow, Blocked>>,
    target: OnceCell<Result<TargetRow, Blocked>>,
    capability_uses: OnceCell<Result<CapabilityUses, Blocked>>,
    scope: OnceCell<Result<Scope, Blocked>>,
    /// The editor's scope over the members that parsed, for a seed with
    /// a hole ([`Snapshot::demand_editor_scope`]).
    partial_scope: OnceCell<Result<Scope, Blocked>>,
    bindings: OnceCell<Result<BindingRows, Blocked>>,
    forms: OnceCell<Result<FormRows, Blocked>>,
    /// The typing's diagnostics, the effects certificate report its
    /// check produced ([`Snapshot::demand_effect_certificates`]), what
    /// it typed, the record the typed-body table is packaged from
    /// ([`Snapshot::demand_typed_bodies`]), what the per-declaration
    /// passes reported per declaration, and what was reused.
    typing: OnceCell<Result<Typed, Blocked>>,
    typed_bodies: OnceCell<Result<TypedBodies, Blocked>>,
    /// The editor's previous snapshot of this seed, offered for its
    /// typing's reuse ([`Snapshot::reusing_typing`]); taken when the
    /// typing runs.
    previous: Cell<Option<Box<Snapshot>>>,
    bus_graph: OnceCell<Result<BusGraph, Blocked>>,
    ownership_graph: OnceCell<Result<OwnershipGraph, Blocked>>,
    handlers: OnceCell<Result<HandlerRouting, Blocked>>,
    /// Shared with the effect rows, which hold the summary their walk
    /// read.
    alloc_summary: OnceCell<Result<Arc<AllocSummary>, Blocked>>,
    effects: OnceCell<Result<EffectRows, Blocked>>,
    placement: OnceCell<Result<PlacementTable, Blocked>>,
    lifecycle: OnceCell<Result<LifecyclePlan, Blocked>>,
    model: OnceCell<Result<ApplicationModel, Blocked>>,
    /// The typing stage, and how many of its diagnostics are the
    /// check's own (the scope's and the typing's, finished) before the
    /// build rules and the advisory: what the laws are deduplicated
    /// against.
    typing_stage: OnceCell<Result<(Checked, usize), Blocked>>,
    laws: OnceCell<Result<Checked, Blocked>>,
    check: OnceCell<Result<Checked, Blocked>>,
    intra_locus: OnceCell<Result<IntraLocusStage, Blocked>>,
    lowering: OnceCell<Result<LoweringView, Blocked>>,
    /// The top-level declarations ([`Snapshot::declarations`]), and the
    /// families' rows joined to them ([`Snapshot::declaration_dependents`]).
    declarations: OnceCell<Vec<Declaration>>,
    dependency_index: OnceCell<Result<DependencyIndex, Blocked>>,
    builds: [Cell<u32>; FAMILIES.len()],
    stage_builds: [Cell<u32>; STAGES.len()],
}

/// What [`Snapshot::from_program`]'s program is keyed and minted by:
/// it names no file.
const BARE_PROGRAM: &str = "program";

/// How many bare programs this process has been handed: a bare
/// snapshot's [`SnapshotKey::sources_digest`] is its handoff's ordinal.
static BARE_HANDOFFS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What a load mode read, before the sequence.
struct Loaded {
    files: Vec<PathBuf>,
    own_files: BTreeSet<PathBuf>,
    members: BTreeMap<PathBuf, Program>,
    programs: BTreeMap<PathBuf, Program>,
    sources: BTreeMap<PathBuf, String>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    import_renames: ImportRenames,
    entry_imports: Vec<Import>,
    unparsed: BTreeMap<PathBuf, Vec<Diag>>,
    unreadable: BTreeMap<PathBuf, String>,
    unlinked: Option<CheckableFailure>,
}

impl Snapshot {
    /// Load `entry` as `mode` loads it, through `src`, and shape it as
    /// `config` says: the wasm entry wrap, the environment's
    /// constitutions, sync inference, the desugar sequence, then the
    /// identities minted with the source map. No family is computed
    /// yet.
    ///
    /// [`LoadMode::WholeSeed`] fails on a file that does not parse or
    /// read; [`LoadMode::Editor`] keeps the members that did and
    /// records the rest ([`Snapshot::unparsed`],
    /// [`Snapshot::unreadable`]), blocking the scope instead, so the
    /// editor reports each against the file that holds it. On what the
    /// import graph refuses (an import that does not resolve, a library
    /// that does not parse or read) the whole seed's load fails, and the
    /// editor's keeps its members as they parsed and records the refusal
    /// ([`Snapshot::linked`]), the scope blocked.
    pub fn load(
        entry: &Path,
        mode: LoadMode,
        src: &dyn SourceProvider,
        config: Config,
    ) -> Result<Snapshot, LoadError> {
        let loaded = match mode {
            LoadMode::WholeSeed => load_whole_seed(entry, src),
            LoadMode::Editor => load_editor(entry, src),
        }
        .map_err(LoadError::Load)?;
        // the key names what was read, so it is computed after the load
        let mut h = Fnv::new();
        for (path, text) in &loaded.sources {
            h.write(path.to_string_lossy().as_bytes());
            h.write(b"\0");
            h.write(text.as_bytes());
            h.write(b"\0");
        }
        let key = SnapshotKey {
            entry: entry.canonicalize().unwrap_or_else(|_| entry.to_path_buf()),
            mode: Some(mode),
            target: config.target.name.clone(),
            config_digest: config.digest(),
            overlay_digest: src.overlay_digest(),
            sources_digest: h.finish(),
        };
        Snapshot::shape(entry, Some(mode), key, config, loaded)
    }

    /// A snapshot of a program a caller already holds: the test
    /// harness's (codegen's `build_executable_with_options`), whose
    /// program was parsed from a string and carries its imports
    /// already merged under `import_renames`. It is shaped as
    /// [`Snapshot::load`] shapes a loaded seed; it has no files, so no
    /// source map, and its sites are seeded by ordinal.
    pub fn from_program(
        program: Program,
        import_renames: ImportRenames,
        config: Config,
    ) -> Result<Snapshot, LoadError> {
        let entry = PathBuf::from(BARE_PROGRAM);
        // A bare program has no source to digest, and its in-memory
        // shape is not a fact the key decides: every handoff is its own
        // load, so the key carries the handoff's ordinal and two bare
        // snapshots never share a key.
        let key = SnapshotKey {
            entry: entry.clone(),
            mode: None,
            target: config.target.name.clone(),
            config_digest: config.digest(),
            overlay_digest: 0,
            sources_digest: BARE_HANDOFFS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        };
        let loaded = Loaded {
            files: Vec::new(),
            own_files: BTreeSet::new(),
            members: BTreeMap::new(),
            programs: std::iter::once((entry.clone(), program)).collect(),
            sources: BTreeMap::new(),
            file_bases: Vec::new(),
            import_renames,
            entry_imports: Vec::new(),
            unparsed: BTreeMap::new(),
            unreadable: BTreeMap::new(),
            unlinked: None,
        };
        Snapshot::shape(&entry, None, key, config, loaded)
    }

    /// Shape what a load read, as `config` says: the wasm entry wrap,
    /// the environment's constitutions, sync inference, the desugar
    /// sequence, then the identities minted with the source map `mode`
    /// builds (none for a bare program).
    fn shape(
        entry: &Path,
        mode: Option<LoadMode>,
        key: SnapshotKey,
        config: Config,
        loaded: Loaded,
    ) -> Result<Snapshot, LoadError> {
        // GH #409: the claims name the environment they were checked
        // for; its label travels beside the evaluation. The binding is
        // the snapshot's own and is scoped around each demand and each
        // serialization ([`Snapshot::with_env`]), never left on the
        // thread for the next snapshot to read.
        let env = hale_types::claims::EnvBinding {
            name: config.environment.as_ref().map(|e| e.name.clone()),
            injected: config
                .environment
                .as_ref()
                .map(|e| e.adopt.clone())
                .unwrap_or_default(),
        };
        let builds: [Cell<u32>; FAMILIES.len()] = Default::default();
        let mut snap = Snapshot {
            key,
            env,
            config,
            files: loaded.files,
            own_files: loaded.own_files,
            members: loaded.members,
            programs: loaded.programs,
            sources: loaded.sources,
            file_bases: loaded.file_bases,
            import_renames: loaded.import_renames,
            demangler: OnceCell::new(),
            entry_imports: loaded.entry_imports,
            source_map: Vec::new(),
            identities: hale_types::snapshot::Snapshot::default(),
            api_surface: None,
            unparsed: loaded.unparsed,
            unreadable: loaded.unreadable,
            unlinked: loaded.unlinked,
            entry: OnceCell::new(),
            target: OnceCell::new(),
            capability_uses: OnceCell::new(),
            scope: OnceCell::new(),
            partial_scope: OnceCell::new(),
            bindings: OnceCell::new(),
            forms: OnceCell::new(),
            typing: OnceCell::new(),
            typed_bodies: OnceCell::new(),
            previous: Cell::new(None),
            bus_graph: OnceCell::new(),
            ownership_graph: OnceCell::new(),
            handlers: OnceCell::new(),
            alloc_summary: OnceCell::new(),
            effects: OnceCell::new(),
            placement: OnceCell::new(),
            lifecycle: OnceCell::new(),
            model: OnceCell::new(),
            typing_stage: OnceCell::new(),
            laws: OnceCell::new(),
            check: OnceCell::new(),
            intra_locus: OnceCell::new(),
            lowering: OnceCell::new(),
            declarations: OnceCell::new(),
            dependency_index: OnceCell::new(),
            builds,
            stage_builds: Default::default(),
        };
        snap.count("seed_loading");
        if snap.config.wrap_main {
            // `--wrap-main`: the wasm `@export` entry synthesized from
            // a bare `fn main` on the AST, before anything else reads
            // the program, so the checker sees the synthesized `target
            // wasm` gate and `@export` locus and every diagnostic keeps
            // the user's own line and column.
            for prog in snap.programs.values_mut() {
                hale_syntax::desugar::wrap_main_as_wasm_export(prog);
            }
        }
        if let Some(env) = &snap.config.environment {
            // The constitutions land in the seed's own `main locus` the
            // build deploys. Whether there is an entry to deploy is the
            // entry row's, read once the load is minted (below): a seed
            // without one is refused, so what landed here is never
            // checked.
            let mut refs: Vec<&mut Program> = snap.programs.values_mut().collect();
            adopt_into_root(&mut refs, &env.adopt);
        }
        // The editor's seed with a member that did not parse or read, or
        // whose imports did not link, is not a program: nothing is
        // shaped or minted, and the scope blocks. (No editor config
        // carries an environment, and the other loads fail on a hole,
        // so no environment reaches here.)
        if snap.has_hole() {
            return Ok(snap);
        }
        // Sync inference writes nothing into the program: its pick is
        // the form rows' effective discipline (`demand_forms`), which
        // the check, the model and lowering read.
        let sequenced = {
            // F.40 phase 2.1b: the desugar sequence, the one every
            // entry point runs before its check: JSON Tier 2's parsers,
            // the api binding (GH #1106, bundle-wide), then the passes
            // that shape a declaration.
            let mut refs: Vec<&mut Program> = snap.programs.values_mut().collect();
            hale_types::desugar_sequence::desugar_before_check(
                &mut refs,
                &hale_types::desugar_sequence::Sequence {
                    import_renames: &snap.import_renames,
                    api: snap.config.api.as_deref(),
                    api_roles: snap.config.api_roles.as_deref(),
                },
            )
        };
        let refused = match sequenced {
            Ok(surface) => {
                snap.api_surface = surface;
                snap.count("desugar_sequence");
                None
            }
            Err(msg) => Some(msg),
        };
        // GH #408 Phase 0: the source map, then the identities minted
        // with it (F.40 phase 1.1b-iii), so each site's seed is the
        // file its span falls in.
        snap.source_map = match mode {
            Some(_) => source_map(entry, &snap.file_bases, &snap.sources),
            None => Vec::new(),
        };
        let names: Vec<String> = snap.programs.keys().map(|p| p.display().to_string()).collect();
        snap.identities = hale_types::snapshot::mint(
            names.iter().map(String::as_str).zip(snap.programs.values_mut()),
            &snap.source_map,
        );
        snap.count("snapshot_identity");
        if snap.config.environment.is_some() {
            // An environment binds law to an ENTRYPOINT, so the target
            // must be one — whether or not the environment contributes
            // a constitution. Checking this only while injecting meant
            // an environment with nothing to inject checked nothing,
            // and a matrix counted a library path as a covered pair.
            // The entry row answers (an imported or a module-nested
            // `main locus` is not the entry), before the sequence's own
            // refusal: an `--api` entry with no main locus to go on is
            // the same missing entry, and the environment names it.
            if !matches!(snap.demand_entry(), Ok(row) if row.entry().is_some()) {
                return Err(LoadError::Refused(format!(
                    "{}: `--env` names a deployment target, and a \
                     deployment target is an ENTRYPOINT — this seed \
                     declares no `main locus`",
                    entry.display()
                )));
            }
        }
        match refused {
            Some(msg) => Err(LoadError::Refused(msg)),
            None => Ok(snap),
        }
    }

    pub fn key(&self) -> &SnapshotKey {
        &self.key
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The files the load started from: the seed's, as the mode
    /// collects them, including one that would not read.
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// The target's own files, canonical: everything else arrived
    /// through an `import`. The editor's load spells a member that
    /// exists only as a buffer by its canonical directory.
    pub fn own_files(&self) -> &BTreeSet<PathBuf> {
        &self.own_files
    }

    pub fn programs(&self) -> &BTreeMap<PathBuf, Program> {
        &self.programs
    }

    /// One of the seed's own files as it parsed, at its base: before the
    /// merge, the imports and the sequence, so it holds what the file
    /// itself declares and nothing else. Kept beside the merged program
    /// for a reader that asks about one file (the editor's outline);
    /// `None` for a file that did not parse, is not the seed's, or a
    /// bare program. Any spelling of the path that names the file.
    pub fn member(&self, path: &Path) -> Option<&Program> {
        self.members.get(path).or_else(|| {
            let canon = path.canonicalize().ok()?;
            self.members
                .iter()
                .find(|(p, _)| p.canonicalize().ok().as_ref() == Some(&canon))
                .map(|(_, prog)| prog)
        })
    }

    /// The identities the load minted over [`Snapshot::programs`]: every
    /// site, and which declaration each use names.
    pub fn identities(&self) -> &hale_types::snapshot::Snapshot {
        &self.identities
    }

    pub fn sources(&self) -> &BTreeMap<PathBuf, String> {
        &self.sources
    }

    /// `(virtual base, path, len)` per file read, in base order.
    pub fn file_bases(&self) -> &[(u32, PathBuf, u32)] {
        &self.file_bases
    }

    pub fn import_renames(&self) -> &ImportRenames {
        &self.import_renames
    }

    /// The demangling of [`Snapshot::import_renames`], indexed once per
    /// snapshot (F.40 phase 3, X3): what puts a diagnostic in the
    /// author's spelling, for every publication made from this load.
    pub fn demangler(&self) -> &hale_types::stdlib_bodies::Demangler {
        self.demangler.get_or_init(|| hale_types::stdlib_bodies::Demangler::new(&self.import_renames))
    }

    /// The target's own `import`s, as written: what a build reads each
    /// library's `[ffi]` surface from. Empty for a seed with a hole,
    /// whose imports are not followed.
    pub fn entry_imports(&self) -> &[Import] {
        &self.entry_imports
    }

    /// The api surface the desugar sequence generated a binding for,
    /// if the program (or `--api`) declared an entry.
    pub fn api_surface(&self) -> Option<&ApiSurface> {
        self.api_surface.as_ref()
    }

    /// The whole seed's program: a load that read the whole seed holds
    /// exactly one, the target's files and every seed its imports
    /// reach, merged. `None` for the editor's seed with a hole, which
    /// holds one program per member that parsed.
    pub fn program(&self) -> Option<&Program> {
        match self.programs.len() {
            1 => self.programs.values().next(),
            _ => None,
        }
    }

    pub fn source_map(&self) -> &[SourceFile] {
        &self.source_map
    }

    /// The files that did not parse, with their diagnostics at
    /// bundle-global spans (the editor's load only).
    pub fn unparsed(&self) -> &BTreeMap<PathBuf, Vec<Diag>> {
        &self.unparsed
    }

    /// The seed members that would not read, with the OS error (the
    /// editor's load only).
    pub fn unreadable(&self) -> &BTreeMap<PathBuf, String> {
        &self.unreadable
    }

    /// What the import graph refused for the editor's seed, if it
    /// refused it: the snapshot then holds its members as they parsed
    /// ([`Snapshot::member`], with their sources and bases) and nothing
    /// linked, and every family is blocked.
    pub fn unlinked(&self) -> Option<&CheckableFailure> {
        self.unlinked.as_ref()
    }

    /// The snapshot, unless the import graph refused its seed: a reader
    /// that needs the seed linked (every editor request but the outline)
    /// gets the refusal as the whole seed's load reports it.
    pub fn linked(mut self) -> Result<Snapshot, CheckableFailure> {
        match self.unlinked.take() {
            Some(failure) => Err(failure),
            None => Ok(self),
        }
    }

    /// A member did not parse or read, the imports did not link, or no
    /// member was loaded: the seed is not a program, and its scope is
    /// blocked.
    fn has_hole(&self) -> bool {
        !self.unparsed.is_empty()
            || !self.unreadable.is_empty()
            || self.unlinked.is_some()
            || self.programs.is_empty()
    }

    /// Why a seed with a hole has no scope.
    fn hole_blocked(&self) -> Blocked {
        let refused: Vec<String> = self
            .unreadable
            .iter()
            .map(|(p, e)| unreadable_message(p, e))
            .chain(self.unlinked.iter().flat_map(|f| f.io.iter().map(|io| io.text.clone())))
            .collect();
        Blocked {
            family: "top_scope",
            because: self
                .unparsed
                .values()
                .flatten()
                .chain(self.unlinked.iter().flat_map(|f| &f.diags))
                .cloned()
                .collect(),
            refused: (!refused.is_empty()).then(|| refused.join("\n")),
        }
    }

    /// The programs as the checker's analyses take them: a view,
    /// built for the call that asks and never stored.
    pub fn bundle(&self) -> Bundle<'_> {
        let mut b = Bundle::new(
            self.programs
                .iter()
                .map(|(p, prog)| (p.display().to_string(), prog))
                .collect(),
        );
        b.import_renames = self.import_renames.clone();
        b.sources = self.source_map.clone();
        b.target = self.config.target.clone();
        b.snapshot = self.identities.clone();
        b
    }

    /// Run `f` with this snapshot's environment bound: what a demand
    /// runs under, and what a caller serializing this snapshot's
    /// artifact (`hale check --dump-topology`) wraps the serialization
    /// in, so the label and the claims' explanations are this
    /// snapshot's whatever was loaded since.
    pub fn with_env<R>(&self, f: impl FnOnce() -> R) -> R {
        hale_types::claims::with_env_binding(&self.env, f)
    }

    /// How many times each family's producer ran for this snapshot,
    /// and each stage of the check: every family of [`FAMILIES`] and
    /// every stage of [`STAGES`], zero when never demanded.
    pub fn builds(&self) -> BTreeMap<&'static str, u32> {
        FAMILIES
            .iter()
            .zip(&self.builds)
            .chain(STAGES.iter().zip(&self.stage_builds))
            .map(|(f, n)| (*f, n.get()))
            .collect()
    }

    fn count(&self, family: &'static str) {
        let (names, cells): (&[&str], &[Cell<u32>]) = match STAGES.contains(&family) {
            true => (&STAGES, &self.stage_builds),
            false => (&FAMILIES, &self.builds),
        };
        let i = names
            .iter()
            .position(|f| *f == family)
            .expect("a snapshot counts only its own families and stages");
        cells[i].set(cells[i].get() + 1);
    }

    /// The entry row: which `main locus` is the program's entry, by the
    /// identity the load minted, with every `main locus` declared as its
    /// witness ([`hale_types::entry`]). It reads declarations only, so
    /// no diagnostic blocks it; a seed with a hole is not a program and
    /// has no identities, so its entry is blocked with its scope.
    pub fn demand_entry(&self) -> Result<&EntryRow, &Blocked> {
        self.entry
            .get_or_init(|| {
                if self.has_hole() {
                    return Err(Blocked { family: "entrypoint", ..self.hole_blocked() });
                }
                self.count("entrypoint");
                Ok(hale_types::entry::entry_row(&self.bundle()))
            })
            .as_ref()
    }

    /// The effective-target row (the `target_capability` family,
    /// `hale_types::capability::target_row`): the one target the check,
    /// the build and the editor act on, for analysis and emission alike
    /// (T1(b)) — an explicit `--target`, else a written
    /// `target wasm`/`browser_js` declaration (wasm32), else the host —
    /// with the refusal of a `--target` a written declaration
    /// contradicts. Its inputs are the key's configured target and the
    /// sources, so a cached row never serves another target's snapshot.
    /// It reads declarations only; a seed with a hole has no programs
    /// to read, so its row is blocked with its scope.
    pub fn demand_target(&self) -> Result<&TargetRow, &Blocked> {
        self.target
            .get_or_init(|| {
                if self.has_hole() {
                    return Err(Blocked { family: "target_capability", ..self.hole_blocked() });
                }
                self.count("target_capability");
                Ok(hale_types::capability::target_row(&self.bundle()))
            })
            .as_ref()
    }

    /// The use rows (the `target_capability` family's second product,
    /// `hale_types::capability::uses`): every way the program asks its
    /// target for a capability, over the resolved graph the allocation
    /// summary holds, each with its witness chain. The check holds them
    /// to the effective target's cells (the admission law), so the
    /// check, the build and the editor refuse alike. Blocked with the
    /// summary. The family's count is its row's: the uses are its second
    /// product, derived once beside it, never a second derivation.
    pub fn demand_capability_uses(&self) -> Result<&CapabilityUses, &Blocked> {
        self.capability_uses
            .get_or_init(|| {
                let summary = self.alloc_summary().map_err(Clone::clone)?.clone();
                Ok(hale_types::capability::uses::derive_capability_uses(&self.bundle(), &summary))
            })
            .as_ref()
    }

    /// The `top_scope` family's producer over the programs held, counted.
    fn build_scope(&self) -> (TopScope, Vec<Diag>) {
        self.count("top_scope");
        hale_types::resolve::build_top_scope(&self.bundle())
    }

    fn scope(&self) -> Result<&Scope, &Blocked> {
        self.scope
            .get_or_init(|| {
                if self.has_hole() {
                    return Err(self.hole_blocked());
                }
                let (top, diags) = self.build_scope();
                Ok(Scope { top, diags })
            })
            .as_ref()
    }

    /// The top scope, with the bundle's topic rows.
    pub fn demand_scope(&self) -> Result<&TopScope, &Blocked> {
        self.scope().map(|s| &s.top)
    }

    /// The binding rows ([`hale_types::binding_rows`]): one per
    /// `bindings { }` entry of the bundle, with the topic's wire key, the
    /// role the ends decide, the transport kind, the codec, whether the
    /// bundle produces the topic and the locus a transport's loss
    /// surfaces through. The checker's binding rules, the model's
    /// binding domains, the bus graph's bound-topic set and lowering's
    /// prelude read them. Blocked with the scope (the topic rows).
    pub fn demand_bindings(&self) -> Result<&BindingRows, &Blocked> {
        self.bindings
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                self.count("bindings");
                Ok(hale_types::binding_rows::derive_binding_rows(&self.bundle(), &scope.top))
            })
            .as_ref()
    }

    /// The matrix's cell for a binding row's transport kind on the
    /// effective target: `RemoteTransport(kind)` × the target row's
    /// class ([`Snapshot::demand_target`]). `Lower` where the target
    /// realizes the transport; `Reject` where it does not. Capability
    /// admission reports a rejected binding use against this effective
    /// target. `None` for a target with no class (a planned tier).
    /// Blocked with the target row.
    pub fn binding_cell(
        &self,
        row: &hale_types::binding_rows::BindingRow,
    ) -> Result<Option<&'static hale_types::capability::Behaviour>, &Blocked> {
        let class = self.demand_target()?.class;
        Ok(class.map(|class| hale_types::capability::transport::transport_cell(class, row.transport)))
    }

    /// The form rows ([`hale_types::form_rows`]): every `@form`
    /// declaration's written `sync` configuration and the discipline it
    /// gets, sync inference's pick for a `hashmap` form its author did
    /// not configure, over the scope and the placement table, per
    /// instance. A scope that reported a diagnostic gets no inference
    /// (the program does not build). Blocked with the scope.
    pub fn demand_forms(&self) -> Result<&FormRows, &Blocked> {
        self.forms
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                let placement = self.demand_placement().map_err(Clone::clone)?;
                self.count("sync_inference");
                Ok(hale_types::form_rows::form_rows(
                    &self.bundle(),
                    &scope.top,
                    placement,
                    scope.diags.is_empty(),
                ))
            })
            .as_ref()
    }

    /// The scope an editor request answers from while the seed is being
    /// typed: [`Snapshot::demand_scope`]'s for a whole seed; for the
    /// editor's seed with a hole ([`LoadMode::Editor`], a member that
    /// did not parse or would not read), the top scope over the members
    /// that parsed — each its own program, unshaped, no import followed
    /// — with the hole named. It is counted as the `top_scope` family:
    /// a seed with a hole never builds the whole scope, so the count
    /// stays one. Nothing else reads it — the check, the graphs and the
    /// model stay blocked, so no check runs over a partial program.
    /// Blocked when no member parsed or the imports did not link, and for
    /// any other load with a hole.
    pub fn demand_editor_scope(&self) -> Result<EditorScope<'_>, &Blocked> {
        if !self.has_hole() {
            return self.demand_scope().map(|top| EditorScope { top, hole: Vec::new() });
        }
        let partial = self
            .partial_scope
            .get_or_init(|| {
                if self.key.mode != Some(LoadMode::Editor) || self.programs.is_empty() {
                    return Err(self.hole_blocked());
                }
                let (top, diags) = self.build_scope();
                Ok(Scope { top, diags })
            })
            .as_ref()?;
        let mut hole: Vec<&Path> =
            self.unparsed.keys().chain(self.unreadable.keys()).map(PathBuf::as_path).collect();
        hole.sort();
        Ok(EditorScope { top: &partial.top, hole })
    }

    /// What the resolver and the checker report, before the laws. The
    /// checker reads the snapshot's families beside the scope
    /// ([`hale_types::check::CheckInputs`]), each demanded before it
    /// runs: the handler rows' producer is total over a program that
    /// does not typecheck (it reads declarations, not types), so a
    /// family the check reads is never one the check had to clear.
    fn typing(&self) -> Result<&[Diag], &Blocked> {
        self.typed().map(|t| t.diags.as_slice())
    }

    fn typed(&self) -> Result<&Typed, &Blocked> {
        self.typing
            .get_or_init(|| self.check_typing(true))
            .as_ref()
    }

    /// The typing's check over the snapshot's families, the scope's
    /// diagnostics first. `reusing`: the per-declaration passes the
    /// editor's previous snapshot of the seed answers are skipped
    /// ([`hale_types::check::check_bundle_by_declaration`]), the previous
    /// snapshot taken once every family the check reads is ready, so a
    /// blocked typing leaves it for the next ([`Snapshot::take_previous`]).
    fn check_typing(&self, reusing: bool) -> Result<Typed, Blocked> {
        let scope = self.scope().map_err(Clone::clone)?;
        // The effect rows on request: a codec binding's purity
        // assertion demands them, nothing else in the check does.
        let effects = || self.demand_effects().ok();
        let inputs = hale_types::check::CheckInputs {
            top: &scope.top,
            handlers: self.demand_handlers().map_err(Clone::clone)?,
            ownership: self.demand_ownership_graph().map_err(Clone::clone)?,
            effects: &effects,
            entry: self.demand_entry().map_err(Clone::clone)?,
            bindings: self.demand_bindings().map_err(Clone::clone)?,
            alloc_summary: self.demand_alloc_summary().map_err(Clone::clone)?,
            forms: self.demand_forms().map_err(Clone::clone)?,
            bus: self.demand_bus_graph().map_err(Clone::clone)?,
            intra_locus: &self.demand_intra_locus().map_err(Clone::clone)?.intra_locus,
            placement: self.demand_placement().map_err(Clone::clone)?,
            target: self.demand_target().map_err(Clone::clone)?,
            uses: self.demand_capability_uses().map_err(Clone::clone)?,
        };
        self.count("expression_typing");
        // The editor's previous snapshot of the seed, if it offered
        // one: what its per-declaration passes may stand in for.
        let offered = if reusing { self.previous.take() } else { None };
        let (plan, reuse) = match offered {
            None => (None, TypingReuse::Fresh),
            Some(prev) => match self.reuse_plan(&prev) {
                Ok((plan, reuse)) => (Some(plan), reuse),
                Err(why) => (None, TypingReuse::Whole(why)),
            },
        };
        let mut diags = scope.diags.clone();
        let (checked, certificates, record, by_decl) = hale_types::check::check_bundle_by_declaration(
            &self.bundle(),
            &inputs,
            self.config.allow_unowned_subscriber,
            self.config.whole_program,
            self.config.whole_program,
            &|key, i| plan.as_ref()?.get(key)?.get(i)?.clone(),
        );
        diags.extend(checked);
        Ok(Typed { diags, certificates, record, by_decl, reuse })
    }

    /// Offer the previous snapshot of this seed for the typing's reuse
    /// (F.40 phase 3, X2, [`crate::typing_reuse`]): the typing stage keeps
    /// the per-declaration results of every declaration unchanged since
    /// `previous` and no dependent of a changed one, and checks the rest.
    /// Opt-in; the editor opts in, every other entry point checks whole.
    /// `previous` is taken when the typing runs: a snapshot whose typing
    /// is blocked (a seed with a hole) leaves it to
    /// [`Snapshot::take_previous`]. A previous snapshot whose typing never
    /// ran, or that loaded another seed or under another config, offers
    /// nothing and the typing is whole ([`TypingReuse::Whole`]).
    pub fn reusing_typing(self, previous: Snapshot) -> Snapshot {
        drop(previous.previous.take());
        self.previous.set(Some(Box::new(previous)));
        self
    }

    /// The previous snapshot [`Snapshot::reusing_typing`] offered, if the
    /// typing has not taken it.
    pub fn take_previous(&self) -> Option<Snapshot> {
        self.previous.take().map(|b| *b)
    }

    /// What the typing stage reused, once it ran: nothing offered, the
    /// seed checked whole and why, or how many declarations were checked
    /// and reused.
    pub fn typing_reuse(&self) -> Option<&TypingReuse> {
        self.typing.get()?.as_ref().ok().map(|t| &t.reuse)
    }

    fn reuse_key(&self) -> ReuseKey {
        ReuseKey {
            entry: self.key.entry.clone(),
            mode: self.key.mode,
            target: self.key.target.clone(),
            config_digest: self.key.config_digest,
            import_renames: {
                // In the table's order, which the load makes stable
                // (`ImportRenames`).
                let mut d = Digest::new();
                d.count(self.import_renames.len());
                for (path, mangled) in &self.import_renames {
                    d.count(path.len());
                    for seg in path {
                        d.field(seg.as_bytes());
                    }
                    d.field(mangled.as_bytes());
                }
                d.finish()
            },
        }
    }

    fn reuse_plan(&self, prev: &Snapshot) -> Result<(Plan, TypingReuse), &'static str> {
        if prev.reuse_key() != self.reuse_key() {
            return Err("another load: the entry, mode, target, config or import renames changed");
        }
        let Some(Ok(done)) = prev.typing.get() else {
            return Err("the previous snapshot was not typed");
        };
        let prev_dependents = |site| prev.declaration_dependents(site).ok();
        let next_dependents = |site| self.declaration_dependents(site).ok();
        crate::typing_reuse::plan(
            &Side { programs: &prev.programs, decls: prev.declarations(), dependents: &prev_dependents },
            &done.by_decl,
            &Side { programs: &self.programs, decls: self.declarations(), dependents: &next_dependents },
        )
    }

    /// The effects certificate report the typing's check produced: each
    /// `@effects` and `@phase_effects` certificate with its diagnostics.
    /// The certificate evidence a law is judged against reads it (the
    /// check's laws, and the artifact's), so the engine runs once per
    /// snapshot. Blocked with the typing; no family of its own.
    pub fn demand_effect_certificates(&self) -> Result<&EffectCertificates, &Blocked> {
        self.typed().map(|t| &t.certificates)
    }

    /// The typed-body table ([`hale_types::typed_bodies`], F.40 phase 3,
    /// E4): what the typing's check typed, keyed by declaration
    /// identity — each closure's accumulator element types, each generic
    /// call's type arguments and unified params, the monomorph table,
    /// each fallible call — with the conformance column judged over the
    /// scope. The check records as it walks, so this packages that
    /// record and runs no second check; a site the checker could not
    /// type is a hole. Blocked with the typing, not with its
    /// diagnostics.
    ///
    /// A typing that reused a declaration's passes (X2,
    /// [`Snapshot::reusing_typing`]) never walked it, so its record lacks
    /// that declaration's bodies; the table of such a snapshot is
    /// packaged from a whole check instead. Only the editor reuses, and
    /// the editor never lowers.
    pub fn demand_typed_bodies(&self) -> Result<&TypedBodies, &Blocked> {
        self.typed_bodies
            .get_or_init(|| {
                let typed = self.typed().map_err(Clone::clone)?;
                let whole;
                let record = match typed.reuse {
                    TypingReuse::Reused { reused, .. } if reused > 0 => {
                        whole = self.check_typing(false)?.record;
                        &whole
                    }
                    _ => &typed.record,
                };
                let scope = self.scope().map_err(Clone::clone)?;
                self.count("typed_bodies");
                Ok(hale_types::typed_bodies::typed_bodies(&self.bundle(), &scope.top, record))
            })
            .as_ref()
    }

    /// The bus graph over the checked programs, with the scope's topic
    /// rows: the model's subjects, endpoints and dispatch gates. Its
    /// placement labels read the snapshot's placement table.
    pub fn demand_bus_graph(&self) -> Result<&BusGraph, &Blocked> {
        self.bus_graph
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                let bindings = self.demand_bindings().map_err(Clone::clone)?;
                let placement = self.demand_placement().map_err(Clone::clone)?;
                let entry = self.demand_entry().map_err(Clone::clone)?;
                self.count("bus_graph");
                Ok(hale_types::bus_graph::build_bus_graph(&self.bundle(), &scope.top, bindings, placement, entry))
            })
            .as_ref()
    }

    /// The ownership graph over the checked programs: the checker's
    /// unowned-subscriber rule (type-check rule 20) and the model's
    /// dynamic births. Edge classes read the snapshot's placement table.
    pub fn demand_ownership_graph(&self) -> Result<&OwnershipGraph, &Blocked> {
        self.ownership_graph
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                let placement = self.demand_placement().map_err(Clone::clone)?;
                let entry = self.demand_entry().map_err(Clone::clone)?;
                self.count("ownership");
                Ok(hale_types::ownership_graph::build_ownership_graph(&self.bundle(), &scope.top, placement, entry))
            })
            .as_ref()
    }

    /// The handler rows of the checked programs: the checker's
    /// duplicate-handler rule and `@supervised` law, and the model's
    /// supervision. Blocked with the scope, which a file that did not
    /// parse blocks.
    pub fn demand_handlers(&self) -> Result<&HandlerRouting, &Blocked> {
        self.handlers
            .get_or_init(|| {
                self.scope().map_err(Clone::clone)?;
                self.count("handler_routing");
                // In the bundle's order: a row's position is its
                // authored ordinal.
                let bundle = self.bundle();
                let programs: Vec<&Program> = bundle.programs.values().copied().collect();
                Ok(hale_types::handler_routing::handler_rows(
                    &programs,
                    &bundle.import_renames,
                    &bundle.snapshot,
                ))
            })
            .as_ref()
    }

    /// The effect rows over the checked programs: one fixpoint, with
    /// the stdlib's analysis copy beside them and cross-seed calls
    /// resolved through the import renames. Per fn: the resolved call
    /// targets, the effect set and its lower bound, what the walk could
    /// not see, the fn's own contribution, its purity. Blocked with the
    /// scope; like the handler rows, its producer reads declarations
    /// and bodies, not types, so it is total over a program that does
    /// not typecheck.
    pub fn demand_effects(&self) -> Result<&EffectRows, &Blocked> {
        self.effects
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                let summary = self.alloc_summary().map_err(Clone::clone)?.clone();
                self.count("effects");
                Ok(hale_types::effect_rows::derive_effect_rows(&self.bundle(), &scope.top, summary))
            })
            .as_ref()
    }

    fn alloc_summary(&self) -> Result<&Arc<AllocSummary>, &Blocked> {
        self.alloc_summary
            .get_or_init(|| {
                self.scope().map_err(Clone::clone)?;
                self.count("alloc_summary");
                Ok(Arc::new(hale_types::alloc_summary::derive_alloc_summary(&self.bundle())))
            })
            .as_ref()
    }

    /// The allocation summary over the checked programs: one per
    /// snapshot, with the stdlib's analysis copy beside them and
    /// cross-seed calls resolved through the import renames
    /// ([`hale_types::alloc_summary::derive_alloc_summary`]). The
    /// check's effects certificate engine reads it, and the effect rows
    /// walk it. Blocked with the scope; its producer reads declarations
    /// and bodies, not types, so it is total over a program that does
    /// not typecheck.
    pub fn demand_alloc_summary(&self) -> Result<&AllocSummary, &Blocked> {
        self.alloc_summary().map(|s| &**s)
    }

    /// The placement table: per static instance of the deployed root's
    /// tower (one per construction literal of the root, and one per
    /// adapter of its `bindings { }`), the declaration it realizes, the
    /// literal that builds it, its domain and what decided it; and every
    /// locus literal outside the tower, with the domains its enclosing
    /// scope runs in and its bound ([`hale_types::placement`]). Seeded
    /// from the entry row's lowering root, after the sequence and the
    /// mint, so every site it names is one a mint numbered. Blocked with
    /// the scope; it reads declarations and bodies, not types, so it is
    /// total over a program that does not typecheck. The check's F.31
    /// rule and sync inference read it (F.40 phase 3, P1), and type-check
    /// rule 20 climbs the construction paths it records (C4).
    /// The lowering laws read it too, including on the harness path (C7).
    pub fn demand_placement(&self) -> Result<&PlacementTable, &Blocked> {
        self.placement
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                let entry = self.demand_entry().map_err(Clone::clone)?;
                self.count("placement");
                Ok(hale_types::placement::derive_placement(&self.bundle(), &scope.top, entry))
            })
            .as_ref()
    }

    /// The top-level declarations of the programs held, in program then
    /// item order, each with its minted site ([`crate::dependents`]).
    /// Empty for a seed with a hole, which is not a program.
    pub fn declarations(&self) -> &[Declaration] {
        self.declarations.get_or_init(|| match self.has_hole() {
            true => Vec::new(),
            false => crate::dependents::declarations(&self.programs, &self.identities),
        })
    }

    /// Which declarations may check differently when the declaration at
    /// `site` changes, through the families' rows: the callgraph's
    /// readers closed, and its neighbours in the ownership graph, the bus
    /// graph, the placement table and the flow rows
    /// ([`crate::dependents`]). [`Dependents::Whole`] for a declaration no
    /// family places and for a site that names no declaration. Blocked
    /// with the scope.
    pub fn declaration_dependents(&self, site: SiteId) -> Result<Dependents, &Blocked> {
        let index = self.dependency_index()?;
        let decls = self.declarations();
        Ok(match decls.iter().position(|d| d.site == Some(site)) {
            Some(i) => index.dependents(decls, i),
            None => Dependents::Whole("the site names no declaration"),
        })
    }

    /// The declaration a site sits in: an index into
    /// [`Snapshot::declarations`].
    pub fn declaration_of(&self, site: SiteId) -> Option<usize> {
        self.dependency_index().ok()?.owner(site)
    }

    fn dependency_index(&self) -> Result<&DependencyIndex, &Blocked> {
        self.dependency_index
            .get_or_init(|| {
                let summary = self.alloc_summary().map_err(Clone::clone)?;
                let ownership = self.demand_ownership_graph().map_err(Clone::clone)?;
                let bus = self.demand_bus_graph().map_err(Clone::clone)?;
                let placement = self.demand_placement().map_err(Clone::clone)?;
                let programs: Vec<&Program> = self.programs.values().collect();
                let flows = hale_types::flows::survey(&programs, &self.import_renames);
                Ok(DependencyIndex::build(&crate::dependents::Families {
                    programs: &self.programs,
                    decls: self.declarations(),
                    summary,
                    ownership,
                    bus,
                    placement,
                    flows: &flows,
                }))
            })
            .as_ref()
    }

    /// The lifecycle plan ([`hale_types::lifecycle::derive`]): per
    /// instance template the placement table names, and per action, the
    /// obligation, with its guard, holder and domain, its entry and
    /// completion edges, its terminals, what it retains and the progress
    /// it owes, each rule with its decision line's status; and the
    /// process's own. Over the placement table, the handler rows, the
    /// flow rows of the checked programs and the bus graph, each
    /// demanded. Blocked with the scope; like its inputs it reads
    /// declarations and bodies, not types. No check builds it; the
    /// lowering view carries it to the emitters (F.40 phase 3, L4).
    pub fn demand_lifecycle(&self) -> Result<&LifecyclePlan, &Blocked> {
        self.lifecycle
            .get_or_init(|| {
                let placement = self.demand_placement().map_err(Clone::clone)?;
                let handlers = self.demand_handlers().map_err(Clone::clone)?;
                let bus = self.demand_bus_graph().map_err(Clone::clone)?;
                self.count("lifecycle_order");
                let bundle = self.bundle();
                let programs: Vec<&Program> = bundle.programs.values().copied().collect();
                let flows = hale_types::flows::survey(&programs, &bundle.import_renames);
                Ok(hale_types::lifecycle::derive::derive_lifecycle(&hale_types::lifecycle::derive::LifecycleInputs {
                    bundle: &bundle,
                    placement,
                    handlers,
                    flows: &flows,
                    bus,
                }))
            })
            .as_ref()
    }

    /// The application model, over the scope, the bus graph, the
    /// ownership graph and the handler rows, each demanded. A model
    /// describes a CHECKED program (GH #476 Change 9): it is blocked
    /// while the resolver or the checker reports an error other than a
    /// claim's.
    pub fn demand_model(&self) -> Result<&ApplicationModel, &Blocked> {
        self.model
            .get_or_init(|| self.with_env(|| {
                let typed = self.typing().map_err(Clone::clone)?;
                if !hale_types::denotes_a_model(typed) {
                    return Err(Blocked {
                        family: "model",
                        because: typed
                            .iter()
                            .filter(|d| d.is_error() && d.kind != hale_syntax::error::DiagKind::Claim)
                            .cloned()
                            .collect(),
                        refused: None,
                    });
                }
                let inputs = hale_types::model_builder::ModelInputs {
                    top: &self.scope().map_err(Clone::clone)?.top,
                    bus_graph: self.demand_bus_graph().map_err(Clone::clone)?,
                    ownership: self.demand_ownership_graph().map_err(Clone::clone)?,
                    handlers: self.demand_handlers().map_err(Clone::clone)?,
                    effects: self.demand_effects().map_err(Clone::clone)?,
                    forms: self.demand_forms().map_err(Clone::clone)?,
                    bindings: self.demand_bindings().map_err(Clone::clone)?,
                    placement: self.demand_placement().map_err(Clone::clone)?,
                };
                self.count("model");
                Ok(hale_types::model_builder::derive_application_model_over(
                    &self.bundle(),
                    &inputs,
                ))
            }))
            .as_ref()
    }

    /// The check's first stage, everything that needs no model: the
    /// scope's and the typing's diagnostics in the user's spelling with
    /// no repeats; then, where the config asks, the build rules
    /// ([`Config::build_rules`]) and the allocation advisory
    /// ([`Config::alloc_advisory`]). Blocked with the typing. The
    /// editor publishes it before the laws are judged.
    pub fn demand_typing(&self) -> Result<&Checked, &Blocked> {
        self.typing_stage().map(|(stage, _)| stage)
    }

    fn typing_stage(&self) -> Result<&(Checked, usize), &Blocked> {
        self.typing_stage
            .get_or_init(|| self.with_env(|| {
                let mut diags = self.typing().map_err(Clone::clone)?.to_vec();
                self.count("typing_stage");
                // The `bare_fallible` law, with the typing diagnostics:
                // it reads the typed-body table's fallible column, the
                // record the typing kept, so it runs no second check.
                let typed = self.demand_typed_bodies().map_err(Clone::clone)?;
                diags.extend(hale_types::bare_fallible::bare_fallible_calls(typed));
                hale_types::finish_check_diags(&mut diags);
                let own = diags.len();
                let bundle = self.bundle();
                if self.config.build_rules {
                    let ownership = self.demand_ownership_graph().map_err(Clone::clone)?;
                    diags.extend(hale_types::build_rule_diags(&bundle, &ownership.rows));
                }
                if self.config.alloc_advisory {
                    let summary = self.demand_alloc_summary().map_err(Clone::clone)?;
                    diags.extend(hale_types::unbounded_alloc_warnings(&bundle, summary, true));
                }
                Ok((Checked { diags }, own))
            }))
            .as_ref()
    }

    /// The check's second stage: when the program denotes a model and
    /// declares a law, the laws judged over the model, in the user's
    /// spelling, none repeating a diagnostic of the typing's own or an
    /// earlier law's; empty otherwise. A program that swears to nothing
    /// demands no model (the epic's demand rule, GH #476 criterion 1).
    /// Blocked with the typing.
    pub fn demand_laws(&self) -> Result<&Checked, &Blocked> {
        self.laws
            .get_or_init(|| self.with_env(|| {
                let typed = self.typing().map_err(Clone::clone)?;
                let (stage, own) = self.typing_stage().map_err(Clone::clone)?;
                self.count("laws_stage");
                let mut diags = Vec::new();
                let bundle = self.bundle();
                // The model is the typing's, before the `bare_fallible`
                // law: a law's finding, like a claim's, hides no law.
                if hale_types::denotes_a_model(typed) && hale_types::judgment::has_claim_surface(&bundle) {
                    if let Ok(model) = self.demand_model() {
                        self.count("claims");
                        let effects = self.demand_effect_certificates().map_err(Clone::clone)?;
                        let summary = self.demand_alloc_summary().map_err(Clone::clone)?;
                        diags = hale_types::judgment::claim_law_diags_over(&bundle, model, effects, summary);
                    }
                }
                hale_types::finish_check_diags_after(&stage.diags[..*own], &mut diags);
                Ok(Checked { diags })
            }))
            .as_ref()
    }

    /// The intra-locus rewrite over the snapshot's program
    /// ([`hale_types::resolved::rewrite_intra_locus`]): the sends
    /// lowering turns into direct calls, as a relation keyed by each
    /// send's id, and the rewritten program lowering continues from.
    /// The check reads the relation (rule 10: a cycle is synchronous
    /// only where every hop is a direct call) and lowering reads both,
    /// so the judgment and the lowering read one rewrite. It runs on a
    /// copy: the check judges the program as written. Blocked with a
    /// hole in the seed.
    pub fn demand_intra_locus(&self) -> Result<&IntraLocusStage, &Blocked> {
        self.intra_locus
            .get_or_init(|| {
                if self.has_hole() {
                    return Err(self.hole_blocked());
                }
                // A whole seed's load holds one program; the editor's
                // holds one per file, merged here as a directory build
                // merges them.
                let merged;
                let program = match self.program() {
                    Some(p) => p,
                    None => {
                        merged = merge_programs(self.programs.values())
                            .expect("a snapshot with no hole holds a program");
                        &merged
                    }
                };
                // The rewrite keeps a publish into a field the table runs
                // off its owner's thread on the bus.
                let placement = self.demand_placement().map_err(Clone::clone)?;
                self.count("intra_locus");
                Ok(hale_types::resolved::rewrite_intra_locus(program, placement))
            })
            .as_ref()
    }

    /// The check: [`Snapshot::demand_typing`]'s diagnostics followed by
    /// [`Snapshot::demand_laws`]', each stage demanded once. Every
    /// entry point reads it; the editor publishes the two stages apart,
    /// and its second publication is this.
    pub fn demand_check(&self) -> Result<&Checked, &Blocked> {
        self.check
            .get_or_init(|| {
                let typing = self.demand_typing().map_err(Clone::clone)?;
                let laws = self.demand_laws().map_err(Clone::clone)?;
                Ok(Checked { diags: typing.diags.iter().chain(&laws.diags).cloned().collect() })
            })
            .as_ref()
    }

    /// The laws that replaced lowering's backstops
    /// ([`hale_types::lowering_laws`], the `law_backstops` family) over
    /// the rows they read, for a view the check does not gate: the
    /// check runs the same laws among its rules, so a gated view has
    /// them already. Blocked with the rows.
    fn demand_lowering_laws(&self) -> Result<Vec<Diag>, Blocked> {
        let ownership = || self.demand_ownership_graph().ok();
        let inputs = hale_types::lowering_laws::LoweringLawInputs {
            placement: self.demand_placement().map_err(Clone::clone)?,
            bindings: self.demand_bindings().map_err(Clone::clone)?,
            ownership: &ownership,
        };
        let mut diags = self.with_env(|| hale_types::lowering_laws::lowering_laws(&self.bundle(), &inputs));
        hale_types::finish_check_diags(&mut diags);
        Ok(diags)
    }

    /// The view codegen lowers: the check first, then
    /// [`hale_types::resolved::resolve_rewritten`] over the intra-locus
    /// rewrite ([`Snapshot::demand_intra_locus`], which reads the
    /// placement table) with the snapshot's source map, renames, api
    /// config and form rows — the topic rewrite as a relation, the
    /// stdlib merge, the mint over the merged program, and the tables.
    /// A check that reported an error
    /// blocks it, with the errors as the reason; a warning does not.
    /// The harness's snapshot ([`Config::harness`]) is not gated on the
    /// check; target admission and the laws that replaced lowering's
    /// backstops still gate it.
    pub fn demand_lowering(&self) -> Result<&LoweringView, &Blocked> {
        self.lowering
            .get_or_init(|| {
                let errors: Vec<Diag> = if self.config.check_gates_lowering {
                    let checked = self.demand_check().map_err(Clone::clone)?;
                    checked.diags.iter().filter(|d| d.is_error()).cloned().collect()
                } else {
                    // Every lowering also obeys target admission (P3-3).
                    let row = self.demand_target().map_err(Clone::clone)?;
                    let uses = self.demand_capability_uses().map_err(Clone::clone)?;
                    let refused = hale_types::capability::uses::admission_diags(uses, row, &self.import_renames);
                    if !refused.is_empty() {
                        return Err(Blocked { family: "target_capability", because: refused, refused: None });
                    }
                    // The harness lowers without the check, but never
                    // without the laws that replaced lowering's own
                    // refusals (F.40 phase 3, C7): lowering judges none
                    // of them, so an ungated view would lower what they
                    // refuse.
                    self.demand_lowering_laws()?.into_iter().filter(|d| d.is_error()).collect()
                };
                if !errors.is_empty() {
                    return Err(Blocked { family: "lowering_view", because: errors, refused: None });
                }
                let stage = self.demand_intra_locus().map_err(Clone::clone)?;
                let forms = self.demand_forms().map_err(Clone::clone)?;
                let bindings = self.demand_bindings().map_err(Clone::clone)?;
                let placement = self.demand_placement().map_err(Clone::clone)?;
                let typed = self.demand_typed_bodies().map_err(Clone::clone)?;
                // Lowering's scope is this one, and its bus and ownership
                // graphs are these ones' rows (C5).
                let scope = self.scope().map_err(Clone::clone)?;
                let bus = self.demand_bus_graph().map_err(Clone::clone)?;
                let ownership = self.demand_ownership_graph().map_err(Clone::clone)?;
                // The effective target's column: what lowering reads for
                // every behaviour and obligation it emits per target. A
                // target with no column (Windows) never reaches a snapshot.
                let class = self.demand_target().map_err(Clone::clone)?.class.ok_or_else(|| Blocked {
                    family: "lowering_view",
                    because: Vec::new(),
                    refused: Some("the target has no column in the capability matrix".to_string()),
                })?;
                // The emitters read the lifecycle plan (F.40 phase 3, L4).
                let lifecycle = self.demand_lifecycle().map_err(Clone::clone)?.clone();
                // Lowering reads which `main locus` it deploys from the
                // entry row (F.40 phase 3, L4).
                let entry = self.demand_entry().map_err(Clone::clone)?.clone();
                // The dispatch plan's domains are the arrangement's, the
                // projection the model's arrangement rows are made of, over
                // the same programs, table and graph (F.40 phase 3, C5).
                let bundle = self.bundle();
                let programs: Vec<&Program> = bundle.programs.values().copied().collect();
                let arrangement =
                    hale_types::arrangement::project_arrangement(&programs, &bundle.snapshot, placement, ownership);
                self.count("lowering_view");
                let mut view = hale_types::resolved::resolve_rewritten(
                    stage,
                    &self.identities,
                    &self.source_map,
                    &self.import_renames,
                    self.config.api.as_deref(),
                    self.config.api_roles.as_deref(),
                    forms,
                    bindings,
                    placement,
                    typed,
                    &scope.top,
                    bus,
                    ownership,
                    &arrangement.domains(),
                    class,
                )
                .map_err(|msg| Blocked { family: "lowering_view", because: Vec::new(), refused: Some(msg) })?;
                view.lifecycle = Some(lifecycle);
                view.entry = Some(entry);
                Ok(view)
            })
            .as_ref()
    }
}

/// `hale check`'s load: the target and every seed its imports reach.
fn load_whole_seed(entry: &Path, src: &dyn SourceProvider) -> Result<Loaded, CheckableFailure> {
    let (files, own, programs, sources, file_bases, effects) = parse_checkable(entry, src)?;
    let members = programs.clone();
    let (programs, sources, file_bases, import_renames, own_files, entry_imports) =
        link_checkable(entry, &files, own, programs, sources, file_bases, effects, src)?;
    Ok(Loaded {
        files: own_files.iter().cloned().collect(),
        own_files,
        members,
        programs,
        sources,
        file_bases,
        import_renames,
        entry_imports,
        unparsed: BTreeMap::new(),
        unreadable: BTreeMap::new(),
        unlinked: None,
    })
}

/// The editor's load ([`LoadMode::Editor`]): the seed of the file being
/// edited, each member parsed at its own base, then linked as `hale
/// check <dir>` links it ([`link_checkable`]: merged, every import
/// followed). A member that will not read is recorded with its OS
/// error, one that does not parse is kept as text with its diagnostics;
/// either leaves the members that parsed unlinked, one program each,
/// and blocks the scope. A link the import graph refuses keeps the
/// members as they parsed and no program, with the refusal recorded
/// (`Snapshot::unlinked`): the scope blocks, and the outline still
/// reads each member.
fn load_editor(entry: &Path, src: &dyn SourceProvider) -> Result<Loaded, CheckableFailure> {
    // A directory that will not list leaves the file alone.
    let files = collect_ap_files(entry, LoadMode::Editor, src)
        .unwrap_or_else(|_| vec![entry.to_path_buf()]);
    let mut programs = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut file_bases: Vec<(u32, PathBuf, u32)> = Vec::new();
    let mut unparsed = BTreeMap::new();
    let mut unreadable = BTreeMap::new();
    // #345: the load's one effect-class table, as `parse_files` keeps it.
    let mut effects = hale_syntax::ast::EffectClasses::default();
    for f in &files {
        let source = match src.read(f) {
            Ok(s) => s,
            Err(e) => {
                unreadable.insert(f.clone(), e.to_string());
                continue;
            }
        };
        let base = file_bases.last().map(|(b, _, l)| b + l + 1).unwrap_or(0);
        file_bases.push((base, f.clone(), source.len() as u32));
        match crate::parse_cache::parse_file(src, f, &source, base, &mut effects) {
            Ok(p) => {
                programs.insert(f.clone(), p);
            }
            Err(diags) => {
                unparsed.insert(f.clone(), diags);
            }
        }
        sources.insert(f.clone(), source);
    }
    let members = programs.clone();
    // The seed's own members, canonical; a member that exists only as a
    // buffer (or a link to nothing) by its canonical directory.
    let own_files: BTreeSet<PathBuf> = files
        .iter()
        .map(|f| {
            f.canonicalize().unwrap_or_else(|_| {
                match (seed_dir_of(f).canonicalize(), f.file_name()) {
                    (Ok(dir), Some(name)) => dir.join(name),
                    _ => f.clone(),
                }
            })
        })
        .collect();
    if !unparsed.is_empty() || !unreadable.is_empty() || programs.is_empty() {
        return Ok(Loaded {
            files,
            own_files,
            members,
            programs,
            sources,
            file_bases,
            import_renames: Vec::new(),
            entry_imports: Vec::new(),
            unparsed,
            unreadable,
            unlinked: None,
        });
    }
    let seed = if src.is_dir(entry) { entry } else { seed_dir_of(entry) };
    // What the members read, kept for a link the import graph refuses.
    let read = (own_files.clone(), sources.clone(), file_bases.clone());
    match link_checkable(seed, &files, own_files, programs, sources, file_bases, effects, src) {
        Ok((programs, sources, file_bases, import_renames, own_files, entry_imports)) => Ok(Loaded {
            files,
            own_files,
            members,
            programs,
            sources,
            file_bases,
            import_renames,
            entry_imports,
            unparsed,
            unreadable,
            unlinked: None,
        }),
        // An import that does not resolve, a library that does not parse
        // or read: the members stay as they parsed, with their sources
        // and bases, and nothing is linked.
        Err(failure) => {
            let (own_files, sources, file_bases) = read;
            Ok(Loaded {
                files,
                own_files,
                members,
                programs: BTreeMap::new(),
                sources,
                file_bases,
                import_renames: Vec::new(),
                entry_imports: Vec::new(),
                unparsed,
                unreadable,
                unlinked: Some(failure),
            })
        }
    }
}

/// GH #409: adopt constitution `name` into the `main locus` `l`, as if
/// the source had written `adopt name;` — the same evaluation, the same
/// closed world, the same union with whatever the source already
/// adopts — creating the `claims` block if `l` has none. `l` is the
/// seed's own `main locus` the build deploys ([`adopt_into_root`]).
///
/// A duplicate is not added: an entrypoint that already writes
/// `adopt Dev;` and is also deployed to an environment requiring `Dev`
/// adopts it once, not twice.
pub fn inject_adopt(l: &mut hale_syntax::ast::LocusDecl, name: &str) {
    use hale_syntax::ast::{ClaimsBlock, Ident, LocusMember};
    let id = Ident::new(name, l.name.span);
    if let Some(LocusMember::Claims(cb)) = l
        .members
        .iter_mut()
        .find(|m| matches!(m, LocusMember::Claims(_)))
    {
        if !cb.adopts.iter().any(|a| a.name == name) {
            cb.adopts.push(id);
        }
    } else {
        l.members.push(LocusMember::Claims(ClaimsBlock {
            entries: Vec::new(),
            adopts: vec![id],
            lib_tier: false,
            span: l.name.span,
        }));
    }
}

/// Adopt each of `names` ([`inject_adopt`]) into the seed's own `main
/// locus` the build deploys: the entry row's lowering root, found over
/// the programs as they stand (F.40 phase 3), never an imported
/// library's `main locus` or a second one. In every seed an
/// environment may be bound to it is the entry (a seed with no entry is
/// refused once the load is minted, so what lands there is never
/// checked), and it is the entry once lowering deploys the entry (L4).
pub fn adopt_into_root(programs: &mut [&mut Program], names: &[String]) {
    let row = {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        hale_types::entry::entry_row_in(&ro)
    };
    let Some((at, item)) = row.lowering_root.as_ref().and_then(|m| m.index_in()) else { return };
    if let Some(l) = hale_syntax::ast::locus_at_mut(&mut programs[at].items, item) {
        for name in names {
            inject_adopt(l, name);
        }
    }
}

/// FNV-1a/64 over length-framed fields: two different field lists
/// never frame to one byte string.
pub(crate) struct Digest(u64);

impl Digest {
    pub(crate) fn new() -> Self {
        Digest(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub(crate) fn count(&mut self, n: usize) {
        self.bytes(&(n as u64).to_le_bytes());
    }

    pub(crate) fn field(&mut self, bytes: &[u8]) {
        self.count(bytes.len());
        self.bytes(bytes);
    }

    fn flag(&mut self, b: bool) {
        self.bytes(&[b as u8]);
    }

    fn option(&mut self, s: Option<&str>) {
        match s {
            None => self.flag(false),
            Some(s) => {
                self.flag(true);
                self.field(s.as_bytes());
            }
        }
    }

    pub(crate) fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::ast::TopDecl;
    use crate::source::{Disk, Overlay};
    use hale_types::target::TargetSpec;

    const CLEAN: &str = "locus W { params { n: Int = 0; } fn bump() { self.n = self.n + 1; } }\n\
                         main locus App { params { w: W = W { }; } run() { self.w.bump(); } }\n\
                         fn main() { App { }; }\n";
    const MISTYPED: &str = "fn main() { let x: Int = \"text\"; }\n";

    /// A scratch directory of this test's own: the pid keeps two runs
    /// apart, the name keeps two tests of one run apart.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("hale-frontend-snapshot-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn load(entry: &Path, src: &dyn SourceProvider, config: Config) -> Snapshot {
        load_as(entry, LoadMode::Editor, src, config)
    }

    fn load_as(entry: &Path, mode: LoadMode, src: &dyn SourceProvider, config: Config) -> Snapshot {
        match Snapshot::load(entry, mode, src, config) {
            Ok(s) => s,
            Err(_) => panic!("the {mode:?} load does not fail"),
        }
    }

    fn errors(s: &Snapshot) -> usize {
        s.demand_check().expect("checked").diags.iter().filter(|d| d.is_error()).count()
    }

    /// Each member's own program is kept beside the merged one: what the
    /// file itself declares, at its base, before the merge and the
    /// sequence; kept whole beside a member that did not parse.
    #[test]
    fn each_member_keeps_its_own_program_beside_the_merged_one() {
        let d = scratch("members");
        std::fs::write(d.join("app.hl"), CLEAN).unwrap();
        std::fs::write(d.join("extra.hl"), "fn helper() -> Int { return 1; }\n").unwrap();
        let names = |p: &Program| -> Vec<String> {
            p.items
                .iter()
                .filter_map(|i| match i {
                    TopDecl::Fn(f) => Some(f.name.name.clone()),
                    TopDecl::Locus(l) => Some(l.name.name.clone()),
                    _ => None,
                })
                .collect()
        };
        // the editor's load of a file is its directory's seed; `hale
        // check`'s is the directory itself
        for (entry, mode) in [(d.join("app.hl"), LoadMode::Editor), (d.clone(), LoadMode::WholeSeed)] {
            let s = load_as(&entry, mode, &Disk, Config::editor());
            assert_eq!(s.programs().len(), 1, "{mode:?}: one merged program");
            let merged = names(s.program().unwrap());
            assert!(merged.contains(&"helper".to_string()) && merged.contains(&"App".to_string()), "{mode:?}: {merged:?}");
            let app = s.member(&d.join("app.hl")).expect("app.hl is a member");
            assert_eq!(names(app), vec!["W", "App", "main"], "{mode:?}");
            let extra = s.member(&d.join("extra.hl")).expect("extra.hl is a member");
            assert_eq!(names(extra), vec!["helper"], "{mode:?}");
            let base = s.file_bases().iter().find(|(_, p, _)| p.ends_with("extra.hl")).unwrap().0;
            assert_eq!(extra.items[0].span().start.0, base, "{mode:?}: at the member's base");
        }
        std::fs::write(d.join("broken.hl"), "fn broken( {\n").unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        assert!(s.member(&d.join("broken.hl")).is_none());
        assert_eq!(names(s.member(&d.join("extra.hl")).expect("parsed")), vec!["helper"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An import that does not resolve: the whole seed's load fails, and
    /// the editor's keeps each member as it parsed, at its base, with the
    /// refusal recorded; nothing is linked, every family is blocked, and
    /// a reader that needs the seed linked gets the refusal (outside
    /// review of #1295, finding 2).
    #[test]
    fn an_unlinked_editor_seed_keeps_its_members_and_blocks_its_families() {
        let d = scratch("unlinked");
        std::fs::write(d.join("app.hl"), "import \"missing\" as m;\nfn helper() -> Int { return 1; }\nfn main() { }\n")
            .unwrap();
        assert!(load_whole_seed(&d.join("app.hl"), &Disk).is_err(), "the whole seed's load fails");
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        assert!(s.unlinked().is_some_and(|f| !f.diags.is_empty() || !f.io.is_empty()), "the refusal is recorded");
        let app = s.member(&d.join("app.hl")).expect("the member is kept");
        assert!(matches!(&app.items[..], [TopDecl::Fn(h), TopDecl::Fn(m)] if h.name.name == "helper" && m.name.name == "main"));
        assert!(s.sources().contains_key(&d.join("app.hl")) && !s.file_bases().is_empty(), "with its source and base");
        assert!(s.programs().is_empty(), "nothing is linked");
        assert!(s.demand_scope().is_err() && s.demand_editor_scope().is_err());
        assert!(s.demand_check().is_err() && s.demand_bus_graph().is_err() && s.demand_model().is_err());
        assert!(s.demand_lowering().is_err());
        assert_eq!(s.builds()["top_scope"], 0, "no scope is built");
        assert!(s.linked().is_err(), "a reader that needs it linked is refused");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #345: a load parses every seed through one effect-class table,
    /// and numbers an imported seed after the seeds it imports: `a`
    /// refers to `money` before `b` (which it imports) declares it, and
    /// still `b`'s classes come first, as the merge's renumbering used
    /// to leave them. Every use names one index.
    #[test]
    fn a_seed_is_numbered_after_the_seeds_it_imports() {
        let d = scratch("effect-order");
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        std::fs::write(
            d.join("app.hl"),
            "import \"a\" as a;\nfn main() { println(a::f(1)); }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("a/a.hl"),
            "import \"../b\" as b;\neffect audit;\n\
             @effects(is: { money })\nfn f(n: Int) -> Int { return b::g(n); }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("b/b.hl"),
            "effect pii;\neffect money;\n@effects(is: { pii })\nfn g(n: Int) -> Int { return n; }\n",
        )
        .unwrap();
        let s = load_as(&d.join("app.hl"), LoadMode::WholeSeed, &Disk, Config::check(true, false));
        let p = s.program().expect("one merged program");
        assert_eq!(p.effect_names, ["pii", "money", "audit"]);
        assert_eq!(p.declared_effects, [0, 1, 2]);
        // each fn's `is:` class, by the fn's last name segment
        let carried: BTreeMap<String, u16> = p
            .items
            .iter()
            .filter_map(|i| match i {
                TopDecl::Fn(f) => f.effects.iter().find_map(|a| match a {
                    hale_syntax::ast::EffectAssert::Carries(cs) => match cs.as_slice() {
                        [hale_syntax::ast::EffectClass::User(c)] => {
                            Some((f.name.name.rsplit('_').next().unwrap_or("").to_string(), *c))
                        }
                        _ => None,
                    },
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        let want: BTreeMap<String, u16> = [("f".to_string(), 1), ("g".to_string(), 0)].into_iter().collect();
        assert_eq!(carried, want, "a's `money` and b's `pii`");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Contract 4: a changed entry, target, config or overlay is a
    /// different snapshot, and each computes its own results.
    #[test]
    fn a_changed_input_is_a_distinct_snapshot_and_shares_no_result() {
        let d = scratch("keys");
        let app = d.join("app.hl");
        std::fs::write(&app, CLEAN).unwrap();
        let other = scratch("keys-other").join("other.hl");
        std::fs::write(&other, CLEAN).unwrap();

        let disk = load(&app, &Disk, Config::editor());
        assert_eq!(disk.key(), load(&app, &Disk, Config::editor()).key(), "the same inputs, the same key");

        let mut buffers = BTreeMap::new();
        buffers.insert(app.clone(), MISTYPED.to_string());
        let edited = load(&app, &Overlay::new(&buffers), Config::editor());
        assert_ne!(disk.key().overlay_digest, edited.key().overlay_digest);

        let one_file = load(&app, &Disk, Config::check(false, false));
        assert_ne!(disk.key().config_digest, one_file.key().config_digest);

        // A build of the same seed refuses the build rules in its check
        // (as the editor's does), `hale check` runs them beside it; an
        // environment and a wrapped entry are passes of a build's load.
        let check = load(&app, &Disk, Config::check(true, false));
        let build = load(&app, &Disk, Config::build(Target::host()));
        assert_ne!(check.key().config_digest, build.key().config_digest);
        let mut deployed = Config::build(Target::host());
        deployed.environment = Some(Environment { name: "prod".to_string(), adopt: Vec::new() });
        assert_ne!(build.key().config_digest, deployed.digest());
        let mut wrapped = Config::build(Target::host());
        wrapped.wrap_main = true;
        assert_ne!(build.key().config_digest, wrapped.digest());

        let mut musl = Config::editor();
        musl.target = Target {
            name: "x86_64-unknown-linux-musl".to_string(),
            spec: TargetSpec::parse("x86_64-unknown-linux-musl").unwrap(),
            explicit: true,
        };
        let musl = load(&app, &Disk, musl);
        assert_ne!(disk.key().target, musl.key().target);
        assert_ne!(disk.key().config_digest, musl.key().config_digest);
        // The spec is folded beside the name: one name over two specs
        // is two digests.
        let mut renamed = Config::editor();
        renamed.target = Target { name: "host".to_string(), spec: musl.config.target.spec, explicit: false };
        assert_ne!(Config::editor().digest(), renamed.digest());
        // `--target` naming the host is not the host fallback: the one
        // overrides a source declaration, the other does not.
        let mut named_host = Config::editor();
        named_host.target.explicit = true;
        assert_ne!(Config::editor().digest(), named_host.digest());

        assert_ne!(disk.key().entry, load(&other, &Disk, Config::editor()).key().entry);

        // No result crosses: the buffer's program is mistyped, the
        // disk's is not, and each snapshot typed its own once.
        assert_eq!(errors(&disk), 0);
        assert!(errors(&edited) > 0);
        assert_eq!(disk.builds()["expression_typing"], 1);
        assert_eq!(edited.builds()["expression_typing"], 1);
        assert_eq!(musl.builds()["expression_typing"], 0, "never demanded");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(other.parent().unwrap());
    }

    /// The effective-target row (design §1.3, §1.7, T1(b)): a function
    /// of the key's configured target and the sources, demanded once per
    /// snapshot. A host and a wasm32 snapshot of one seed differ in
    /// `target` and `config_digest` and each derive their own row;
    /// nothing else is computed for it. With no `--target`, a written
    /// declaration selects wasm32; an explicit `--target` of another
    /// class is refused at the declaration.
    #[test]
    fn the_target_row_is_the_configured_target_and_the_sources() {
        use hale_types::capability::{Selection, TargetClass};
        let d = scratch("target-row");
        let app = d.join("app.hl");
        std::fs::write(&app, "fn main() {\n    let _ = std::process::pid();\n}\n").unwrap();
        let wasm = Target {
            name: "wasm32-unknown-unknown".to_string(),
            spec: TargetSpec::parse("wasm32").unwrap(),
            explicit: true,
        };

        let host = load(&app, &Disk, Config::build(Target::host()));
        let on_wasm = load(&app, &Disk, Config::build(wasm.clone()));
        assert_ne!(host.key().target, on_wasm.key().target);
        assert_ne!(host.key().config_digest, on_wasm.key().config_digest);

        let row = host.demand_target().expect("the row reads declarations only");
        assert_eq!(row.configured.name, "host");
        assert_eq!(row.declaration, None);
        assert_eq!((row.class, row.selected_by), (Some(TargetClass::PosixAsync), Selection::Host));
        let row = on_wasm.demand_target().unwrap();
        assert_eq!(row.configured.name, "wasm32-unknown-unknown");
        assert_eq!((row.class, row.selected_by), (Some(TargetClass::Wasm32), Selection::Configured));
        assert_eq!(row.wasm32_selector(), "`--target wasm32`");
        let _ = on_wasm.demand_target();
        for s in [&host, &on_wasm] {
            assert_eq!(s.builds()["target_capability"], 1, "one derivation per snapshot");
            assert_eq!(s.builds()["expression_typing"], 0, "the row computes nothing else");
        }

        // A written declaration with no `--target` selects wasm32, for
        // analysis and emission alike.
        std::fs::write(&app, "target wasm { }\n\nfn main() {\n    println(\"hi\");\n}\n").unwrap();
        let declared = load(&app, &Disk, Config::build(Target::host()));
        assert_ne!(declared.key().sources_digest, host.key().sources_digest);
        let row = declared.demand_target().unwrap();
        let decl = row.declaration.as_ref().expect("the declaration is recorded");
        assert_eq!((decl.name.as_str(), decl.span.start.0, decl.synthesized), ("wasm", 0, false));
        assert!(decl.file.ends_with("app.hl"), "{}", decl.file);
        assert_eq!((row.class, row.selected_by), (Some(TargetClass::Wasm32), Selection::Declared));
        assert!(row.refusals.is_empty());
        assert_eq!(row.wasm32_selector(), "`target wasm`");

        // `--target` naming the host contradicts it: refused at the
        // declaration, and the host stays the effective target.
        let mut named = Target::host();
        named.explicit = true;
        let refused = load(&app, &Disk, Config::build(named));
        let row = refused.demand_target().unwrap();
        assert_eq!((row.class, row.selected_by), (Some(TargetClass::PosixAsync), Selection::Configured));
        let messages: Vec<(u32, &str)> = row.refusals.iter().map(|d| (d.span.start.0, d.message.as_str())).collect();
        let want = format!(
            "this program declares `target wasm`, and is being checked for `{}`: build it with \
             `--target wasm32`, or drop the declaration",
            TargetSpec::host().triple
        );
        assert_eq!(messages, vec![(0, want.as_str())]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// No target leaks across the editor's snapshots (design §1.7, P3 2
    /// of 3): one session's buffer gains a `target wasm { }` line and
    /// loses it again. The stdlib refusal appears and disappears with the
    /// line, each snapshot derives its own row once, and the use rows
    /// are the same three times: what changed is the target, not the
    /// program's uses.
    #[test]
    fn the_editor_follows_a_target_line_in_and_out() {
        let d = scratch("target-edit");
        let app = d.join("app.hl");
        let body = "fn main() {\n    let _ = std::process::pid();\n}\n";
        std::fs::write(&app, body).unwrap();
        let mut uses = Vec::new();
        for (text, refused) in
            [(body.to_string(), false), (format!("{body}\ntarget wasm {{ }}\n"), true), (body.to_string(), false)]
        {
            let mut buffers = BTreeMap::new();
            buffers.insert(app.clone(), text);
            let s = load(&app, &Overlay::new(&buffers), Config::editor());
            let diags = &s.demand_check().expect("the check runs").diags;
            let gate = diags.iter().filter(|d| d.message.contains("is unavailable under `target wasm`")).count();
            assert_eq!(gate, usize::from(refused), "{diags:?}");
            assert_eq!(s.builds()["target_capability"], 1, "one derivation per snapshot");
            uses.push(s.demand_capability_uses().expect("the use rows").clone());
        }
        assert!(uses.windows(2).all(|w| w[0] == w[1]), "the uses do not depend on the target");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Contract 3: a seed with a file that did not parse has no scope,
    /// and nothing after the scope is computed.
    #[test]
    fn a_file_that_does_not_parse_blocks_the_scope_and_its_dependents() {
        let d = scratch("blocked");
        std::fs::write(d.join("app.hl"), CLEAN).unwrap();
        std::fs::write(d.join("broken.hl"), "fn broken( {\n").unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        let blocked = s.demand_check().expect_err("a seed that did not parse is not checked");
        assert_eq!(blocked.family, "top_scope");
        assert!(!blocked.because.is_empty());
        assert!(s.unparsed().contains_key(&d.join("broken.hl")));
        assert_eq!(s.demand_model().expect_err("no model either").family, "top_scope");
        let builds = s.builds();
        assert_eq!(builds["seed_loading"], 1);
        let Err(blocked) = s.demand_lowering() else { panic!("nor a lowering view") };
        assert_eq!(blocked.family, "top_scope");
        for f in FAMILIES.iter().filter(|f| **f != "seed_loading") {
            assert_eq!(builds[f], 0, "{f} ran for a seed that did not parse");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The editor's scope over a seed with a hole: the members that
    /// parsed, with the hole named, built once and counted as the
    /// scope; the check stays blocked, and a whole seed's editor scope
    /// is its scope.
    #[test]
    fn the_editor_scope_covers_the_members_that_parsed_and_names_the_hole() {
        let d = scratch("editor-scope");
        std::fs::write(d.join("app.hl"), CLEAN).unwrap();
        std::fs::write(d.join("broken.hl"), "fn broken( {\n").unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        assert!(s.demand_scope().is_err(), "the whole scope is blocked");
        let scope = s.demand_editor_scope().expect("the members that parsed");
        assert_eq!(scope.hole, vec![d.join("broken.hl").as_path()]);
        assert!(scope.top.lookup("App").is_some() && scope.top.lookup("broken").is_none());
        let again = s.demand_editor_scope().expect("still there");
        assert!(std::ptr::eq(scope.top, again.top), "built once");
        assert_eq!(s.demand_check().expect_err("never checked").family, "top_scope");
        assert_eq!(s.builds()["top_scope"], 1);
        assert_eq!(s.builds()["expression_typing"], 0);

        std::fs::remove_file(d.join("broken.hl")).unwrap();
        let whole = load(&d.join("app.hl"), &Disk, Config::editor());
        let scope = whole.demand_editor_scope().expect("a whole seed");
        assert!(scope.hole.is_empty());
        assert!(std::ptr::eq(scope.top, whole.demand_scope().unwrap()), "the snapshot's scope");
        assert_eq!(whole.builds()["top_scope"], 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Contract 3 for a member that will not read (a dangling symlink):
    /// the editor's load records it with the OS error, and the scope is
    /// blocked with the message the LSP publishes, as it is for a file
    /// that did not parse; `hale check`'s load of the same seed fails.
    #[cfg(unix)]
    #[test]
    fn a_member_that_will_not_read_blocks_the_scope() {
        let d = scratch("unreadable");
        std::fs::write(d.join("app.hl"), CLEAN).unwrap();
        std::os::unix::fs::symlink("absent.hl", d.join("missing.hl")).unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        let os_error = s.unreadable().get(&d.join("missing.hl")).expect("the member is recorded").clone();
        let blocked = s.demand_check().expect_err("a seed with a hole is not checked");
        assert_eq!(blocked.family, "top_scope");
        assert_eq!(blocked.refused.as_deref(), Some(format!("seed member missing.hl: {os_error}").as_str()));
        for f in FAMILIES.iter().filter(|f| **f != "seed_loading") {
            assert_eq!(s.builds()[f], 0, "{f} ran for a seed with an unreadable member");
        }
        assert!(load_whole_seed(&d, &Disk).is_err(), "the CLI's load of the seed fails");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Contract 3, one family later: a program that does not typecheck
    /// is checked, and has no model.
    #[test]
    fn a_program_that_does_not_typecheck_blocks_the_model() {
        let d = scratch("untyped");
        std::fs::write(d.join("app.hl"), MISTYPED).unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        assert!(errors(&s) > 0);
        let blocked = s.demand_model().expect_err("a mistyped program denotes no model");
        assert_eq!(blocked.family, "model");
        assert!(blocked.because.iter().all(|d| d.is_error()) && !blocked.because.is_empty());
        assert_eq!(s.builds()["model"], 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Contract 3 at lowering: a check that reported an error blocks
    /// the lowering view, with the errors as the reason.
    #[test]
    fn a_check_with_errors_blocks_the_lowering_view() {
        let d = scratch("unlowered");
        std::fs::write(d.join("app.hl"), MISTYPED).unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        let Err(blocked) = s.demand_lowering() else { panic!("a mistyped program is not lowered") };
        assert_eq!(blocked.family, "lowering_view");
        assert!(!blocked.because.is_empty() && blocked.because.iter().all(|d| d.is_error()));
        assert!(blocked.refused.is_none());
        assert_eq!(s.builds()["lowering_view"], 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The entry row (F.40 phase 3, E0): the seed's own top-level `main
    /// locus`, by the site the load minted; the three ways to have none
    /// (no `main`, only an imported one, only a module-nested one), and
    /// a seed with an imported, a module-nested and a top-level one
    /// keeping the top-level one. Demanded twice, built once. Beside
    /// each, the provisional lowering root: the first `main locus` that
    /// is not a library's, nested or not, which is not always the entry.
    #[test]
    fn the_entry_row_is_the_seeds_own_top_level_main_locus() {
        use hale_types::entry::NoEntry;
        let d = scratch("entry");
        let lib = d.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("main.hl"), "main locus LibHead { params { n: Int = 0; } }\n").unwrap();
        let row_of = |name: &str, text: &str| {
            let seed = d.join(name);
            std::fs::create_dir_all(&seed).unwrap();
            std::fs::write(seed.join("main.hl"), text).unwrap();
            // The editor's load links a seed's imports as `hale check
            // <dir>` does.
            let s = load(&seed, &Disk, Config::check(true, false));
            let row = s.demand_entry().expect("an entry row").clone();
            assert!(std::ptr::eq(s.demand_entry().unwrap(), s.demand_entry().unwrap()));
            assert_eq!(s.builds()["entrypoint"], 1, "{name}: built once");
            for m in &row.mains {
                let decl = m.decl(&s.bundle()).expect("the row names a declaration");
                assert_eq!(decl.name.name, m.name);
                assert_eq!(m.site, s.identities().site_id(decl.id), "{name}: by the minted site");
                assert!(m.site.is_some(), "{name}: the load minted it");
            }
            row
        };

        let none = row_of("none", "fn main() { }\n");
        assert_eq!(none.no_entry(), Some(NoEntry::NoMain));
        assert!(none.mains.is_empty());
        assert!(none.lowering_root.is_none());
        let root = |row: &hale_types::entry::EntryRow| row.lowering_root.as_ref().map(|m| m.name.clone());

        let imported = row_of("imported", "import \"../lib\" as lib;\nfn main() { }\n");
        assert_eq!(imported.no_entry(), Some(NoEntry::OnlyImported), "decision 1: {imported:?}");
        assert_eq!(imported.mains.len(), 1);
        assert!(imported.mains[0].imported && !imported.mains[0].module_nested);
        assert!(imported.mains[0].name.starts_with("__lib_"), "the rename pass marked what it renamed");
        assert_eq!(root(&imported), None, "lowering deploys no library's main");

        let nested = row_of("nested", "module inner {\n    main locus App { params { n: Int = 0; } }\n}\nfn main() { }\n");
        assert_eq!(nested.no_entry(), Some(NoEntry::OnlyModuleNested), "decision 2: {nested:?}");
        assert!(nested.mains[0].module_nested && !nested.mains[0].imported);
        assert_eq!(root(&nested).as_deref(), Some("App"), "no entry, and still lowering's root (until L4)");

        let all = row_of(
            "all",
            "import \"../lib\" as lib;\n\
             module inner {\n    main locus Other { params { n: Int = 0; } }\n}\n\
             main locus App { params { n: Int = 0; } }\n\
             fn main() { App { }; }\n",
        );
        let entry = all.entry().expect("the top-level one is the entry");
        assert_eq!(entry.name, "App");
        assert!(!entry.imported && !entry.module_nested);
        assert_eq!(all.mains.len(), 3, "the witness keeps every declaration: {all:?}");
        assert_eq!(all.candidates().count(), 1);
        assert_eq!(all.own().count(), 2, "rule 1 counts the module-nested one");
        // Lowering takes the first in declaration order (rule 1 refuses
        // the program before it builds).
        assert_eq!(root(&all).as_deref(), Some("Other"), "{all:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The lowering view carries the snapshot's entry row (F.40 phase 3,
    /// L4), and lowering reads which `main locus` it deploys from the
    /// row's lowering root. The two disagree on one shape: a seed whose
    /// only `main` is module-nested has no entry (decision 2) and still a
    /// lowering root, which lowering deploys until it reads the entry.
    #[test]
    fn the_lowering_view_carries_the_entry_row() {
        let d = scratch("view_entry");
        let view_of = |name: &str, text: &str| {
            let seed = d.join(name);
            std::fs::create_dir_all(&seed).unwrap();
            std::fs::write(seed.join("main.hl"), text).unwrap();
            let s = load(&seed, &Disk, Config::check(true, false));
            let row = s.demand_entry().expect("an entry row").clone();
            let view = s.demand_lowering().unwrap_or_else(|b| panic!("{name}: the view is blocked: {:?}", b.because));
            assert_eq!(view.entry(), Some(&row), "{name}: the view carries the snapshot's row");
            row
        };
        let root = |row: &hale_types::entry::EntryRow| row.lowering_root.as_ref().map(|m| m.name.clone());
        let top = view_of("top", "main locus App { params { n: Int = 0; } }\nfn main() { App { }; }\n");
        assert_eq!(top.entry().map(|m| m.name.as_str()), Some("App"));
        assert_eq!(root(&top).as_deref(), Some("App"), "the entry is the root lowering deploys");
        let nested = view_of(
            "nested",
            "module inner {\n    main locus App { params { n: Int = 0; } }\n}\nfn main() { App { }; }\n",
        );
        assert!(nested.entry().is_none(), "decision 2: no entry");
        assert_eq!(root(&nested).as_deref(), Some("App"), "and still the root lowering deploys");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The binding rows (F.40 phase 3, P2): one row per `bindings { }`
    /// entry, by the entry's minted site, with the role the ends decide
    /// (a publish-only topic connects, a subscribe-only one listens, an
    /// explicit role wins, a topic with neither end has none), the topic's
    /// wire key, the loss locus of a `unix` transport and the bound-topic
    /// set as the rows' projection. Demanded twice, built once, and the
    /// bus graph reads the same set.
    #[test]
    fn the_binding_rows_decide_the_role_once() {
        use hale_syntax::ast::TransportRole;
        use hale_types::capability::Transport;
        let d = scratch("bindings");
        std::fs::write(
            d.join("app.hl"),
            "type Beat { n: Int; }\n\
             topic Out { payload: Beat; subject: \"demo.out\"; }\n\
             topic In { payload: Beat; subject: \"demo.in\"; }\n\
             topic Both { payload: Beat; subject: \"demo.both\"; }\n\
             topic Free { payload: Beat; subject: \"demo.free\"; }\n\
             topic Ring { payload: Beat; subject: \"demo.ring\"; }\n\
             locus Tap { bus { subscribe Both as on_both; } fn on_both(b: Beat) { } }\n\
             main locus App {\n\
                 params { t: Tap = Tap { }; }\n\
                 bus { publish Out; subscribe In as on_in; publish Both; }\n\
                 bindings {\n\
                     Out: unix(\"/tmp/p2-out.sock\");\n\
                     In: unix(\"/tmp/p2-in.sock\");\n\
                     Both: unix(\"/tmp/p2-both.sock\", role: listen);\n\
                     Free: unix(\"/tmp/p2-free.sock\");\n\
                     Ring: shm_ring(\"/p2ring\", on_overflow: drop);\n\
                 }\n\
                 fn on_in(b: Beat) { }\n\
             }\n\
             fn main() { App { }; }\n",
        )
        .unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::check(true, false));
        let rows = s.demand_bindings().expect("binding rows");
        assert!(std::ptr::eq(rows, s.demand_bindings().unwrap()));
        assert_eq!(s.builds()["bindings"], 1, "built once");
        let row = |topic: &str| rows.rows.iter().find(|r| r.topic == topic).expect("a row per entry");
        assert_eq!(rows.rows.len(), 5);
        for r in &rows.rows {
            assert!(r.site.is_some(), "{}: the load minted the entry", r.topic);
            assert_eq!(r.entry(&s.bundle()).expect("the row names its entry").topic.name, r.topic);
            assert!(r.is_main && !r.imported && !r.module_nested);
        }
        assert_eq!((row("Out").transport, row("Out").role), (Transport::Unix, Some(TransportRole::Connect)));
        assert_eq!(row("Out").key(), "demo.out");
        assert_eq!(row("Out").loss_locus, Some("__StdBusUnixConnectTransport"));
        assert_eq!(row("In").role, Some(TransportRole::Listen));
        assert_eq!(row("In").loss_locus, Some("__StdBusUnixListenTransport"));
        assert_eq!(row("Both").role, Some(TransportRole::Listen), "an explicit role wins over the ends");
        assert!(row("Both").publishes && row("Both").subscribes);
        assert_eq!(row("Free").role, None, "no end: the checker's diagnostic, no role");
        assert_eq!(row("Free").loss_locus, None);
        assert_eq!((row("Ring").transport, row("Ring").role), (Transport::ShmRing, None));
        assert!(row("Out").producer && !row("In").producer);
        let bound = rows.bound_subjects();
        assert!(bound.contains("Out") && bound.contains("demo.out"), "both grains: {bound:?}");
        assert_eq!(rows.bound_names().len(), 5);
        // The transport kind is admitted by the matrix's cell on the
        // effective target: every kind lowers on the host.
        for r in &rows.rows {
            assert!(s.binding_cell(r).expect("the target row").expect("a class").is_lower(), "{}", r.topic);
        }
        let wasm = Target {
            name: "wasm32-unknown-unknown".to_string(),
            spec: TargetSpec::parse("wasm32").unwrap(),
            explicit: true,
        };
        let on_wasm = load(&d.join("app.hl"), &Disk, Config::build(wasm));
        let wasm_rows = on_wasm.demand_bindings().expect("binding rows");
        let lowered = |t: Transport| {
            let r = wasm_rows.rows.iter().find(|r| r.transport == t).expect("a row of the kind");
            on_wasm.binding_cell(r).unwrap().unwrap().is_lower()
        };
        assert!(!lowered(Transport::Unix) && !lowered(Transport::ShmRing), "wasm refuses both native substrates");
        // The bus graph's gate reads the projection.
        let graph = s.demand_bus_graph().expect("bus graph");
        for wire in ["Out", "In", "Both"] {
            let info = graph.subjects.get(wire).unwrap_or_else(|| panic!("{wire} is in the graph"));
            assert!(!info.eligible, "{wire}: a bound subject is not devirtualized");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The form rows (F.40 phase 3, C1): one row per `@form` declaration,
    /// found by the identity the load minted, its discipline inference's
    /// for a map two pools write (the one instance `fn main` hands both
    /// workers; inference is per instance, P1). Demanded twice, built
    /// once.
    #[test]
    fn the_form_rows_are_one_family_by_identity() {
        use hale_types::form_rows::Discipline;
        let d = scratch("forms");
        std::fs::write(
            d.join("app.hl"),
            "type Entry { k: Int; v: Int; }\n\
             type Tick { n: Int; }\n\
             @form(hashmap)\nlocus Registry { capacity { pool entries of Entry indexed_by k; } }\n\
             @form(vec)\nlocus Log { capacity { heap lines of Entry; } }\n\
             locus IoWorker { params { reg: Registry = Registry { }; }\n\
                 bus { subscribe \"tick\" as on_tick of type Tick; }\n\
                 fn on_tick(t: Tick) { self.reg.set(Entry { k: t.n, v: 1 }); } }\n\
             locus CompWorker { params { reg: Registry = Registry { }; }\n\
                 bus { subscribe \"tick\" as on_tick of type Tick; }\n\
                 fn on_tick(t: Tick) { self.reg.set(Entry { k: t.n, v: 2 }); } }\n\
             main locus App {\n\
                 params { io: IoWorker = IoWorker { }; cpu: CompWorker = CompWorker { }; }\n\
                 placement { io: cooperative(pool = io); cpu: cooperative(pool = compute); }\n\
                 bus { publish \"tick\" of type Tick; }\n\
                 run() { } }\n\
             fn main() {\n\
                 let reg = Registry { };\n\
                 App { io: IoWorker { reg: reg }, cpu: CompWorker { reg: reg } };\n\
             }\n",
        )
        .unwrap();
        let s = load_as(&d.join("app.hl"), LoadMode::WholeSeed, &Disk, Config::check(true, false));
        let rows = s.demand_forms().expect("the form rows");
        assert!(std::ptr::eq(rows, s.demand_forms().unwrap()));
        assert_eq!(s.builds()["sync_inference"], 1, "built once");
        assert_eq!(rows.rows().len(), 2, "every form, not only maps");
        let bundle = s.bundle();
        for row in rows.rows() {
            let decl = bundle
                .programs
                .values()
                .flat_map(|p| p.items.iter())
                .find_map(|i| match i {
                    TopDecl::Locus(l) if l.name.name == row.locus => Some(l),
                    _ => None,
                })
                .expect("the row names a declaration");
            assert!(!row.id.is_none(), "the load minted it");
            assert_eq!(row.id.0, decl.id.0, "by the minted identity");
            assert!(std::ptr::eq(rows.of(decl).unwrap(), row));
        }
        let reg = rows.named("Registry").unwrap();
        assert_eq!(reg.effective, Discipline::Striped, "two writer pools on a hot path");
        assert!(reg.safe_for_cross_domain_access());
        assert_eq!(rows.named("Log").unwrap().effective, Discipline::None);
        // Nothing is written into the program on the author's behalf,
        // and lowering lays the map out by the row.
        let registry = |items: &[TopDecl]| {
            items
                .iter()
                .find_map(|i| match i {
                    TopDecl::Locus(l) if l.name.name == "Registry" => Some(l.clone()),
                    _ => None,
                })
                .expect("the map")
        };
        let written = bundle.programs.values().find_map(|p| {
            p.items.iter().any(|i| matches!(i, TopDecl::Locus(l) if l.name.name == "Registry")).then(|| registry(&p.items))
        });
        assert!(written.unwrap().form.unwrap().args.iter().all(|a| a.name.name != "sync"), "no injected argument");
        let view = s.demand_lowering().expect("the lowering view");
        assert_eq!(view.forms.effective(&registry(&view.merged.items)), Discipline::Striped);
        assert_eq!(s.builds()["sync_inference"], 1, "lowering reads the snapshot's rows");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Everything a snapshot holds that a reader can see, as text: the
    /// key, the files and their bases, the source map, every member and
    /// program as parsed, shaped and minted, the identities, the check
    /// and the families it built.
    fn observable(s: &Snapshot) -> String {
        let members: Vec<String> = s.files().iter().map(|f| format!("member {:?}", s.member(f))).collect();
        format!(
            "key {:?}\nfiles {:?}\nbases {:?}\nsources {:?}\nmap {:?}\nrenames {:?}\nmembers {members:?}\nprograms {:?}\nids {:?}\ncheck {:?}\nbuilds {:?}",
            s.key(),
            s.files(),
            s.file_bases(),
            s.sources(),
            s.source_map(),
            s.import_renames(),
            s.programs(),
            s.identities(),
            s.demand_check().map(|c| &c.diags),
            s.builds(),
        )
    }

    /// Parse reuse (F.40 phase 3, X1): two loads over the same text share
    /// each file's parse — the second parses nothing — and differ from
    /// each other, and from a load that reuses nothing, in nothing a
    /// reader can see; the first load's shaping and mint did not reach
    /// the kept products. An edit reparses the edited file alone. The
    /// seed holds what moves with a base or a table: two members and an
    /// imported library parsed twice (through its own table and the
    /// load's), user effect classes, an f-string's interpolation, and,
    /// last, a member that does not parse.
    #[test]
    fn a_reused_parse_is_the_parse() {
        use crate::parse_cache::ParseCache;
        let d = scratch("parse-reuse");
        std::fs::create_dir_all(d.join("lib")).unwrap();
        std::fs::write(
            d.join("lib/lib.hl"),
            "effect audit;\n@effects(is: { audit })\nfn stamp(n: Int) -> Int { return n + 1; }\n",
        )
        .unwrap();
        std::fs::write(d.join("a.hl"), "effect money;\nfn helper(n: Int) -> String { return f\"n = {n + 1}\"; }\n").unwrap();
        let app = d.join("app.hl");
        let text = |k: i64| {
            format!(
                "import \"lib\" as lib;\n{CLEAN}fn tick() -> Int {{ let s = helper({k}); println(s); return lib::stamp({k}); }}\n"
            )
        };
        let mut buffers = BTreeMap::new();
        buffers.insert(app.clone(), text(1));
        let cache = ParseCache::new();
        let plain = load(&app, &Overlay::new(&buffers), Config::editor());
        let first = load(&app, &Overlay::new(&buffers).reusing(&cache), Config::editor());
        let parsed = cache.misses();
        assert!(parsed == 4 && cache.hits() == 0, "two members, the library twice: {parsed}");
        let second = load(&app, &Overlay::new(&buffers).reusing(&cache), Config::editor());
        assert_eq!((cache.hits(), cache.misses()), (parsed, parsed), "the second load parses nothing");
        assert!(errors(&plain) == 0 && s_has_lib(&plain), "a clean, linked seed");
        assert_eq!(observable(&first), observable(&plain), "a parse kept is the parse");
        assert_eq!(observable(&second), observable(&plain), "a parse reused is the parse");

        buffers.insert(app.clone(), text(2));
        let edited = load(&app, &Overlay::new(&buffers).reusing(&cache), Config::editor());
        assert_eq!(cache.misses(), parsed + 1, "the edited file alone is parsed");
        let fresh = load(&app, &Overlay::new(&buffers), Config::editor());
        assert_eq!(observable(&edited), observable(&fresh));
        assert_ne!(observable(&edited), observable(&plain));

        // The whole seed's load reads through a reusing provider too.
        let whole = load_as(&d, LoadMode::WholeSeed, &Overlay::new(&buffers).reusing(&cache), Config::check(true, false));
        let disk_whole = load_as(&d, LoadMode::WholeSeed, &Overlay::new(&buffers), Config::check(true, false));
        assert_eq!(observable(&whole), observable(&disk_whole));

        // A file that does not parse: its diagnostics, at its base, kept
        // and reused like a program.
        std::fs::write(d.join("broken.hl"), "fn broken( {\n").unwrap();
        let broken = |src: &dyn SourceProvider| {
            let s = load(&app, src, Config::editor());
            format!("{:?} {:?}", s.unparsed(), s.file_bases())
        };
        let want = broken(&Overlay::new(&buffers));
        let hits = cache.hits();
        assert_eq!(broken(&Overlay::new(&buffers).reusing(&cache)), want);
        assert_eq!(broken(&Overlay::new(&buffers).reusing(&cache)), want);
        assert!(cache.hits() > hits);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn s_has_lib(s: &Snapshot) -> bool {
        s.file_bases().iter().any(|(_, p, _)| p.ends_with("lib/lib.hl"))
    }

    /// Contract 1 at lowering: the view is resolved once, after the
    /// check it is gated on, however often it is demanded.
    #[test]
    fn the_lowering_view_is_resolved_once_after_the_check() {
        let d = scratch("lowered");
        std::fs::write(d.join("app.hl"), CLEAN).unwrap();
        let s = load(&d.join("app.hl"), &Disk, Config::editor());
        let first = s.demand_lowering().expect("a clean program is lowered") as *const LoweringView;
        let again = s.demand_lowering().expect("still lowered") as *const LoweringView;
        assert_eq!(first, again, "the second demand reads the first result");
        let builds = s.builds();
        assert_eq!(builds["expression_typing"], 1);
        assert_eq!(builds["lowering_view"], 1);
        assert_eq!(builds["model"], 0, "a program with no claims lowers without a model");
        let _ = std::fs::remove_dir_all(&d);
    }
}
