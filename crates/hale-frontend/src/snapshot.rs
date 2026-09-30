//! The snapshot (F.40 phase 2.2): what a check needs, loaded once, and
//! the families derived from it, each computed on demand at most once.
//!
//! A [`Snapshot`] owns the loaded programs and their keys, the source
//! map, the import renames, the [`Config`] that shaped them, the desugar
//! sequence already run and the identities minted after it. Everything
//! else is a family, demanded by name:
//!
//! - [`Snapshot::demand_scope`]: the top scope, with its topic rows.
//! - [`Snapshot::demand_bus_graph`], [`Snapshot::demand_ownership_graph`]
//!   and [`Snapshot::demand_handlers`]: the bus graph, the ownership
//!   graph and the handler rows over the checked programs, what the
//!   model reads beside the scope.
//! - [`Snapshot::demand_model`]: the application model, over the scope
//!   and those three.
//! - [`Snapshot::demand_check`]: what the checker reports — the scope's
//!   and the typing's diagnostics, and the laws judged over the model
//!   when the program declares any.
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

use hale_model::ApplicationModel;
use hale_syntax::api_gen::ApiSurface;
use hale_syntax::ast::{Import, Program, TopDecl};
use hale_syntax::Diag;
use hale_types::bus_graph::BusGraph;
use hale_types::handler_routing::HandlerRouting;
use hale_types::ownership_graph::OwnershipGraph;
use hale_types::resolve::TopScope;
use hale_types::resolved::LoweringView;
use hale_types::symbol::SourceFile;
use hale_types::Bundle;

use crate::frontend::{
    collect_ap_files, collect_checkable, link_checkable, merge_programs, seed_dir_of, source_map,
    CheckableFailure, LoadMode,
};
use crate::imports::ImportRenames;
use crate::source::SourceProvider;

/// The families a snapshot produces, in the order a build demands them.
/// The names are the registry's (`spec/registry.md`). `bus_graph`,
/// `ownership` and `handler_routing` count the checked programs' graphs,
/// the model's inputs; `lowering_view` is the `demand` family's own, the
/// view over the resolved program whose tables are lowering's ownership,
/// bus-graph, dispatch and handler-routing rows. Until the check runs
/// over the resolved program, a snapshot that is checked for its model
/// and lowered holds both shapes' graphs.
pub const FAMILIES: [&str; 11] = [
    "seed_loading",
    "desugar_sequence",
    "snapshot_identity",
    "top_scope",
    "expression_typing",
    "bus_graph",
    "ownership",
    "handler_routing",
    "model",
    "claims",
    "lowering_view",
];

/// The target a snapshot is checked for: what `where async_io` may
/// assume, and how a refusal names the platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The target's name: `host`, or the triple a build names.
    pub name: String,
    pub has_async_io: bool,
    /// The platform as the `async_io` diagnostic names it.
    pub label: &'static str,
}

impl Target {
    /// The machine the compiler runs on: every check's target.
    pub fn host() -> Self {
        let b = Bundle::new(BTreeMap::new());
        Target {
            name: "host".to_string(),
            has_async_io: b.target_has_async_io,
            label: b.target_label,
        }
    }
}

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
    /// The rules a build refuses beside the check (the borrow rule and
    /// bare fallible calls, `hale_types::build_rule_diags`), appended to
    /// the check's diagnostics, so they block lowering. `hale check`
    /// runs them itself, beside its reports.
    pub build_rules: bool,
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
            check_gates_lowering: true,
        }
    }

    /// The test harness's (codegen's `build_executable_with_options`,
    /// over [`Snapshot::from_program`]): a build's config whose
    /// lowering is not gated on a check. The harness runs no checker —
    /// a test that wants the check calls it itself, and the agreement
    /// sweep (`corpus_check_build_agreement`) compares the two — so its
    /// snapshot lowers what it is handed. The check is still a family
    /// of it, computed only when demanded.
    pub fn harness(target: Target) -> Self {
        Config { check_gates_lowering: false, ..Config::build(target) }
    }

    /// The LSP's: `hale check <dir>`'s report. It checks a seed only
    /// once every member of it read and parsed, so it holds a whole
    /// program (GH #721), and its check carries the build rules `hale
    /// check` runs beside its own (the borrow rule, bare fallible
    /// calls), so the editor shows every error the CLI prints.
    pub fn editor() -> Self {
        Config { build_rules: true, ..Config::check(true, false) }
    }

    fn digest(&self) -> u64 {
        let mut d = Digest::new();
        d.field(self.target.name.as_bytes());
        d.flag(self.target.has_async_io);
        d.field(self.target.label.as_bytes());
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
    /// view's own producer refusing (`resolve_program`: a bundled
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

/// The top scope and what building it reported.
struct Scope {
    top: TopScope,
    diags: Vec<Diag>,
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
    programs: BTreeMap<PathBuf, Program>,
    sources: BTreeMap<PathBuf, String>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    import_renames: ImportRenames,
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
    scope: OnceCell<Result<Scope, Blocked>>,
    typing: OnceCell<Result<Vec<Diag>, Blocked>>,
    bus_graph: OnceCell<Result<BusGraph, Blocked>>,
    ownership_graph: OnceCell<Result<OwnershipGraph, Blocked>>,
    handlers: OnceCell<Result<HandlerRouting, Blocked>>,
    model: OnceCell<Result<ApplicationModel, Blocked>>,
    check: OnceCell<Result<Checked, Blocked>>,
    lowering: OnceCell<Result<LoweringView, Blocked>>,
    builds: [Cell<u32>; FAMILIES.len()],
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
    programs: BTreeMap<PathBuf, Program>,
    sources: BTreeMap<PathBuf, String>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    import_renames: ImportRenames,
    entry_imports: Vec<Import>,
    unparsed: BTreeMap<PathBuf, Vec<Diag>>,
    unreadable: BTreeMap<PathBuf, String>,
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
    /// editor reports each against the file that holds it. Both fail
    /// on what the import graph refuses (an import that does not
    /// resolve, a library that does not parse or read).
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
            programs: std::iter::once((entry.clone(), program)).collect(),
            sources: BTreeMap::new(),
            file_bases: Vec::new(),
            import_renames,
            entry_imports: Vec::new(),
            unparsed: BTreeMap::new(),
            unreadable: BTreeMap::new(),
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
            programs: loaded.programs,
            sources: loaded.sources,
            file_bases: loaded.file_bases,
            import_renames: loaded.import_renames,
            entry_imports: loaded.entry_imports,
            source_map: Vec::new(),
            identities: hale_types::snapshot::Snapshot::default(),
            api_surface: None,
            unparsed: loaded.unparsed,
            unreadable: loaded.unreadable,
            scope: OnceCell::new(),
            typing: OnceCell::new(),
            bus_graph: OnceCell::new(),
            ownership_graph: OnceCell::new(),
            handlers: OnceCell::new(),
            model: OnceCell::new(),
            check: OnceCell::new(),
            lowering: OnceCell::new(),
            builds,
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
            // An environment binds law to an ENTRYPOINT, so the target
            // must be one — whether or not the environment contributes
            // a constitution. Checking this only while injecting meant
            // an environment with nothing to inject checked nothing,
            // and a matrix counted a library path as a covered pair.
            let has_main = snap.programs.values().any(|p| {
                p.items.iter().any(|i| matches!(i, TopDecl::Locus(l) if l.is_main))
            });
            if !has_main {
                return Err(LoadError::Refused(format!(
                    "{}: `--env` names a deployment target, and a \
                     deployment target is an ENTRYPOINT — this seed \
                     declares no `main locus`",
                    entry.display()
                )));
            }
            // The main locus exists, so every constitution lands.
            for c in &env.adopt {
                for prog in snap.programs.values_mut() {
                    inject_adopt(prog, c);
                }
            }
        }
        // The editor's seed with a member that did not parse or read is
        // not a program: nothing is shaped or minted, and the scope
        // blocks.
        if snap.has_hole() {
            return Ok(snap);
        }
        for prog in snap.programs.values_mut() {
            // FUv0.8.2 #4: the post-inference shape the build sees.
            // The pre-pass's resolver diagnostics are discarded: the
            // check raises them again through its own reporting path
            // (downstream handoff, 2026-08-11).
            let _ = hale_types::apply_sync_inference(prog);
        }
        {
            // F.40 phase 2.1b: the desugar sequence, the one every
            // entry point runs before its check: JSON Tier 2's parsers,
            // the api binding (GH #1106, bundle-wide), then the passes
            // that shape a declaration.
            let mut refs: Vec<&mut Program> = snap.programs.values_mut().collect();
            snap.api_surface = hale_types::desugar_sequence::desugar_before_check(
                &mut refs,
                &hale_types::desugar_sequence::Sequence {
                    import_renames: &snap.import_renames,
                    api: snap.config.api.as_deref(),
                    api_roles: snap.config.api_roles.as_deref(),
                },
            )
            .map_err(LoadError::Refused)?;
        }
        snap.count("desugar_sequence");
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
        Ok(snap)
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

    /// A member did not parse or read, or none was loaded: the seed is
    /// not a program, and its scope is blocked.
    fn has_hole(&self) -> bool {
        !self.unparsed.is_empty() || !self.unreadable.is_empty() || self.programs.is_empty()
    }

    /// Why a seed with a hole has no scope.
    fn hole_blocked(&self) -> Blocked {
        Blocked {
            family: "top_scope",
            because: self.unparsed.values().flatten().cloned().collect(),
            refused: (!self.unreadable.is_empty()).then(|| {
                self.unreadable
                    .iter()
                    .map(|(p, e)| unreadable_message(p, e))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
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
        b.target_has_async_io = self.config.target.has_async_io;
        b.target_label = self.config.target.label;
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

    /// How many times each family's producer ran for this snapshot:
    /// every family of [`FAMILIES`], zero when never demanded.
    pub fn builds(&self) -> BTreeMap<&'static str, u32> {
        FAMILIES
            .iter()
            .zip(&self.builds)
            .map(|(f, n)| (*f, n.get()))
            .collect()
    }

    fn count(&self, family: &'static str) {
        let i = FAMILIES
            .iter()
            .position(|f| *f == family)
            .expect("a snapshot counts only its own families");
        self.builds[i].set(self.builds[i].get() + 1);
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

    /// What the resolver and the checker report, before the laws.
    fn typing(&self) -> Result<&[Diag], &Blocked> {
        self.typing
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                self.count("expression_typing");
                let mut diags = scope.diags.clone();
                diags.extend(hale_types::check::check_bundle_scoped(
                    &self.bundle(),
                    &scope.top,
                    self.config.allow_unowned_subscriber,
                    self.config.whole_program,
                    self.config.whole_program,
                ));
                Ok(diags)
            })
            .as_ref()
            .map(Vec::as_slice)
    }

    /// The bus graph over the checked programs, with the scope's topic
    /// rows: the model's subjects, endpoints and dispatch gates.
    pub fn demand_bus_graph(&self) -> Result<&BusGraph, &Blocked> {
        self.bus_graph
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                self.count("bus_graph");
                Ok(hale_types::bus_graph::build_bus_graph(&self.bundle(), &scope.top))
            })
            .as_ref()
    }

    /// The ownership graph over the checked programs: the model's
    /// dynamic births.
    pub fn demand_ownership_graph(&self) -> Result<&OwnershipGraph, &Blocked> {
        self.ownership_graph
            .get_or_init(|| {
                let scope = self.scope().map_err(Clone::clone)?;
                self.count("ownership");
                Ok(hale_types::ownership_graph::build_ownership_graph(&self.bundle(), &scope.top))
            })
            .as_ref()
    }

    /// The handler rows of the checked programs: the model's
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
                };
                self.count("model");
                Ok(hale_types::model_builder::derive_application_model_over(
                    &self.bundle(),
                    &inputs,
                ))
            }))
            .as_ref()
    }

    /// The check: the typing's diagnostics, and — when the program
    /// denotes a model and declares a law — the laws judged over the
    /// model. A program that swears to nothing demands no model (the
    /// epic's demand rule, GH #476 criterion 1). A build's config
    /// ([`Config::build_rules`]) appends the build rules after them.
    pub fn demand_check(&self) -> Result<&Checked, &Blocked> {
        self.check
            .get_or_init(|| self.with_env(|| {
                let mut diags = self.typing().map_err(Clone::clone)?.to_vec();
                let bundle = self.bundle();
                if hale_types::denotes_a_model(&diags)
                    && hale_types::judgment::has_claim_surface(&bundle)
                {
                    if let Ok(model) = self.demand_model() {
                        self.count("claims");
                        diags.extend(hale_types::judgment::claim_law_diags_over(&bundle, model));
                    }
                }
                hale_types::finish_check_diags(&mut diags);
                if self.config.build_rules {
                    diags.extend(hale_types::build_rule_diags(&bundle));
                }
                Ok(Checked { diags })
            }))
            .as_ref()
    }

    /// The view codegen lowers: the check first, then
    /// [`hale_types::resolved::resolve_program`] over the snapshot's
    /// program, source map, renames and api config — the two lowering
    /// rewrites as relations, the stdlib merge, the mint over the
    /// merged program, and the tables. A check that reported an error
    /// blocks it, with the errors as the reason; a warning does not.
    /// The harness's snapshot ([`Config::harness`]) is not gated.
    pub fn demand_lowering(&self) -> Result<&LoweringView, &Blocked> {
        self.lowering
            .get_or_init(|| {
                if self.config.check_gates_lowering {
                    let checked = self.demand_check().map_err(Clone::clone)?;
                    let errors: Vec<Diag> =
                        checked.diags.iter().filter(|d| d.is_error()).cloned().collect();
                    if !errors.is_empty() {
                        return Err(Blocked { family: "lowering_view", because: errors, refused: None });
                    }
                }
                // A whole seed's load holds one program; the editor's
                // holds one per file, merged here as a directory build
                // merges them.
                let merged;
                let program = match self.program() {
                    Some(p) => p,
                    None => {
                        merged = merge_programs(self.programs.values())
                            .expect("a checked snapshot holds a program");
                        &merged
                    }
                };
                self.count("lowering_view");
                hale_types::resolved::resolve_program(
                    program,
                    &self.source_map,
                    &self.import_renames,
                    self.config.api.as_deref(),
                    self.config.api_roles.as_deref(),
                )
                .map_err(|msg| Blocked { family: "lowering_view", because: Vec::new(), refused: Some(msg) })
            })
            .as_ref()
    }
}

/// `hale check`'s load: the target and every seed its imports reach.
fn load_whole_seed(entry: &Path, src: &dyn SourceProvider) -> Result<Loaded, CheckableFailure> {
    let (programs, sources, file_bases, import_renames, own_files, entry_imports) =
        collect_checkable(entry, src)?;
    Ok(Loaded {
        files: own_files.iter().cloned().collect(),
        own_files,
        programs,
        sources,
        file_bases,
        import_renames,
        entry_imports,
        unparsed: BTreeMap::new(),
        unreadable: BTreeMap::new(),
    })
}

/// The editor's load ([`LoadMode::Editor`]): the seed of the file being
/// edited, each member parsed at its own base, then linked as `hale
/// check <dir>` links it ([`link_checkable`]: merged, every import
/// followed). A member that will not read is recorded with its OS
/// error, one that does not parse is kept as text with its diagnostics;
/// either leaves the members that parsed unlinked, one program each,
/// and blocks the scope.
fn load_editor(entry: &Path, src: &dyn SourceProvider) -> Result<Loaded, CheckableFailure> {
    // A directory that will not list leaves the file alone.
    let files = collect_ap_files(entry, LoadMode::Editor, src)
        .unwrap_or_else(|_| vec![entry.to_path_buf()]);
    let mut programs = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut file_bases: Vec<(u32, PathBuf, u32)> = Vec::new();
    let mut unparsed = BTreeMap::new();
    let mut unreadable = BTreeMap::new();
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
        match hale_syntax::parse_source_at(&source, base) {
            Ok(p) => {
                programs.insert(f.clone(), p);
            }
            Err(diags) => {
                unparsed.insert(f.clone(), diags);
            }
        }
        sources.insert(f.clone(), source);
    }
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
            programs,
            sources,
            file_bases,
            import_renames: Vec::new(),
            entry_imports: Vec::new(),
            unparsed,
            unreadable,
        });
    }
    let seed = if src.is_dir(entry) { entry } else { seed_dir_of(entry) };
    let (programs, sources, file_bases, import_renames, own_files, entry_imports) =
        link_checkable(seed, &files, own_files, programs, sources, file_bases, src)?;
    Ok(Loaded {
        files,
        own_files,
        programs,
        sources,
        file_bases,
        import_renames,
        entry_imports,
        unparsed,
        unreadable,
    })
}

/// GH #409: adopt constitution `name` into `prog`'s main locus, as if
/// the source had written `adopt name;` — the same evaluation, the same
/// closed world, the same union with whatever the source already
/// adopts — creating the `claims` block if the main has none. False
/// when `prog` declares no main locus.
///
/// A duplicate is not added: an entrypoint that already writes
/// `adopt Dev;` and is also deployed to an environment requiring `Dev`
/// adopts it once, not twice.
pub fn inject_adopt(prog: &mut Program, name: &str) -> bool {
    use hale_syntax::ast::{ClaimsBlock, Ident, LocusMember};
    let mut found = false;
    for item in &mut prog.items {
        let TopDecl::Locus(l) = item else { continue };
        if !l.is_main {
            continue;
        }
        found = true;
        let id = Ident { name: name.to_string(), span: l.name.span };
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
    found
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
    use crate::source::{Disk, Overlay};

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
        match Snapshot::load(entry, LoadMode::Editor, src, config) {
            Ok(s) => s,
            Err(_) => panic!("the editor's load does not fail"),
        }
    }

    fn errors(s: &Snapshot) -> usize {
        s.demand_check().expect("checked").diags.iter().filter(|d| d.is_error()).count()
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
            has_async_io: false,
            label: "musl Linux",
        };
        let musl = load(&app, &Disk, musl);
        assert_ne!(disk.key().target, musl.key().target);
        assert_ne!(disk.key().config_digest, musl.key().config_digest);

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
