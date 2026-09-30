//! The snapshot (F.40 phase 2.2): what a check needs, loaded once, and
//! the families derived from it, each computed on demand at most once.
//!
//! A [`Snapshot`] owns the loaded programs and their keys, the source
//! map, the import renames, the [`Config`] that shaped them, the desugar
//! sequence already run and the identities minted after it. Everything
//! else is a family, demanded by name:
//!
//! - [`Snapshot::demand_scope`]: the top scope, with its topic rows.
//! - [`Snapshot::demand_model`]: the application model, over the scope.
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
use hale_types::resolve::TopScope;
use hale_types::resolved::LoweringView;
use hale_types::symbol::SourceFile;
use hale_types::Bundle;

use crate::frontend::{
    collect_ap_files, collect_checkable, merge_programs, source_map, source_map_as_spelled,
    CheckableFailure, LoadMode,
};
use crate::imports::ImportRenames;
use crate::source::SourceProvider;

/// The families a snapshot produces, in the order a build demands them.
/// The names are the registry's (`spec/registry.md`); `lowering_view`
/// is the `demand` family's own, the view whose tables are the
/// ownership, bus-graph, dispatch and handler-routing families'.
pub const FAMILIES: [&str; 8] = [
    "seed_loading",
    "desugar_sequence",
    "snapshot_identity",
    "top_scope",
    "expression_typing",
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
        }
    }

    /// The LSP's: it checks a seed only once every file of it parsed,
    /// so it holds a whole program (GH #721).
    pub fn editor() -> Self {
        Config::check(true, false)
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
        d.finish()
    }
}

/// What identifies a snapshot. Two snapshots with different keys were
/// loaded from different inputs, and share no result.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotKey {
    /// The target the load started from, canonical where it exists.
    pub entry: PathBuf,
    pub target: String,
    pub config_digest: u64,
    /// The editor buffers the load read over the disk; the disk alone
    /// is [`SourceProvider::overlay_digest`]'s zero.
    pub overlay_digest: u64,
}

/// A family that was not computed because a prerequisite reported
/// errors: which family, and the errors.
#[derive(Debug, Clone)]
pub struct Blocked {
    pub family: &'static str,
    pub because: Vec<Diag>,
    /// The family's own producer refused, with no position to report
    /// it at: the lowering view's `resolve_program` (a bundled stdlib
    /// that does not parse, a site the mint left unnumbered). `None`
    /// when a prerequisite blocked it, whose errors are `because`.
    pub refused: Option<String>,
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
    scope: OnceCell<Result<Scope, Blocked>>,
    typing: OnceCell<Result<Vec<Diag>, Blocked>>,
    model: OnceCell<Result<ApplicationModel, Blocked>>,
    check: OnceCell<Result<Checked, Blocked>>,
    lowering: OnceCell<Result<LoweringView, Blocked>>,
    builds: [Cell<u32>; FAMILIES.len()],
}

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
}

impl Snapshot {
    /// Load `entry` as `mode` loads it, through `src`, and shape it as
    /// `config` says: the wasm entry wrap, the environment's
    /// constitutions, sync inference, the desugar sequence, then the
    /// identities minted with the source map. No family is computed
    /// yet.
    ///
    /// [`LoadMode::WholeSeed`] fails on a file that does not parse;
    /// [`LoadMode::SeedDirectoryOnly`] (the editor's) keeps the files
    /// that did parse and blocks the scope instead, so the editor
    /// reports the parse errors against the files that hold them.
    pub fn load(
        entry: &Path,
        mode: LoadMode,
        src: &dyn SourceProvider,
        config: Config,
    ) -> Result<Snapshot, LoadError> {
        // GH #409: the claims name the environment they were checked
        // for; its label travels beside the evaluation.
        hale_types::claims::set_env_binding(hale_types::claims::EnvBinding {
            name: config.environment.as_ref().map(|e| e.name.clone()),
            injected: config
                .environment
                .as_ref()
                .map(|e| e.adopt.clone())
                .unwrap_or_default(),
        });
        let key = SnapshotKey {
            entry: entry.canonicalize().unwrap_or_else(|_| entry.to_path_buf()),
            target: config.target.name.clone(),
            config_digest: config.digest(),
            overlay_digest: src.overlay_digest(),
        };
        let builds: [Cell<u32>; FAMILIES.len()] = Default::default();
        let loaded = match mode {
            LoadMode::WholeSeed => load_whole_seed(entry, src).map_err(LoadError::Load)?,
            LoadMode::SeedDirectoryOnly => load_seed_directory(entry, src),
        };
        let mut snap = Snapshot {
            key,
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
            scope: OnceCell::new(),
            typing: OnceCell::new(),
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
        // The editor's seed with a file that did not parse is not a
        // program: nothing is shaped or minted, and the scope blocks.
        if !snap.unparsed.is_empty() || snap.programs.is_empty() {
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
            LoadMode::WholeSeed => source_map(entry, &snap.file_bases, &snap.sources),
            LoadMode::SeedDirectoryOnly => source_map_as_spelled(&snap.file_bases, &snap.sources),
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
    /// through an `import`. Empty for the editor's load.
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
    /// library's `[ffi]` surface from. Empty for the editor's load.
    pub fn entry_imports(&self) -> &[Import] {
        &self.entry_imports
    }

    /// The api surface the desugar sequence generated a binding for,
    /// if the program (or `--api`) declared an entry.
    pub fn api_surface(&self) -> Option<&ApiSurface> {
        self.api_surface.as_ref()
    }

    /// The whole seed's program: a [`LoadMode::WholeSeed`] load holds
    /// exactly one, the target's files and every seed its imports
    /// reach, merged. `None` for the editor's load of several files.
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

    fn scope(&self) -> Result<&Scope, &Blocked> {
        self.scope
            .get_or_init(|| {
                if !self.unparsed.is_empty() || self.programs.is_empty() {
                    return Err(Blocked {
                        family: "top_scope",
                        because: self.unparsed.values().flatten().cloned().collect(),
                        refused: None,
                    });
                }
                self.count("top_scope");
                let (top, diags) = hale_types::resolve::build_top_scope(&self.bundle());
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

    /// The application model. A model describes a CHECKED program
    /// (GH #476 Change 9): it is blocked while the resolver or the
    /// checker reports an error other than a claim's.
    pub fn demand_model(&self) -> Result<&ApplicationModel, &Blocked> {
        self.model
            .get_or_init(|| {
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
                let scope = self.scope().map_err(Clone::clone)?;
                self.count("model");
                Ok(hale_types::model_builder::derive_application_model_in(
                    &self.bundle(),
                    &scope.top,
                ))
            })
            .as_ref()
    }

    /// The check: the typing's diagnostics, and — when the program
    /// denotes a model and declares a law — the laws judged over the
    /// model. A program that swears to nothing demands no model (the
    /// epic's demand rule, GH #476 criterion 1). A build's config
    /// ([`Config::build_rules`]) appends the build rules after them.
    pub fn demand_check(&self) -> Result<&Checked, &Blocked> {
        self.check
            .get_or_init(|| {
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
            })
            .as_ref()
    }

    /// The view codegen lowers: the check first, then
    /// [`hale_types::resolved::resolve_program`] over the snapshot's
    /// program, source map, renames and api config — the two lowering
    /// rewrites as relations, the stdlib merge, the mint over the
    /// merged program, and the tables. A check that reported an error
    /// blocks it, with the errors as the reason; a warning does not.
    pub fn demand_lowering(&self) -> Result<&LoweringView, &Blocked> {
        self.lowering
            .get_or_init(|| {
                let checked = self.demand_check().map_err(Clone::clone)?;
                let errors: Vec<Diag> =
                    checked.diags.iter().filter(|d| d.is_error()).cloned().collect();
                if !errors.is_empty() {
                    return Err(Blocked { family: "lowering_view", because: errors, refused: None });
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
    })
}

/// The editor's load ([`LoadMode::SeedDirectoryOnly`]): the file's
/// directory, each file parsed at its own base and kept as its own
/// program. A file that will not read is skipped; one that does not
/// parse is kept as text, with its diagnostics.
fn load_seed_directory(entry: &Path, src: &dyn SourceProvider) -> Loaded {
    // A directory that will not list leaves the file alone.
    let files = collect_ap_files(entry, LoadMode::SeedDirectoryOnly, src)
        .unwrap_or_else(|_| vec![entry.to_path_buf()]);
    let mut programs = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut file_bases: Vec<(u32, PathBuf, u32)> = Vec::new();
    let mut unparsed = BTreeMap::new();
    for f in &files {
        let Ok(source) = src.read(f) else { continue };
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
    Loaded {
        files,
        own_files: BTreeSet::new(),
        programs,
        sources,
        file_bases,
        import_renames: Vec::new(),
        entry_imports: Vec::new(),
        unparsed,
    }
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
        match Snapshot::load(entry, LoadMode::SeedDirectoryOnly, src, config) {
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

        // A build of the same seed refuses the build rules; an
        // environment and a wrapped entry are passes of its load.
        let build = load(&app, &Disk, Config::build(Target::host()));
        assert_ne!(disk.key().config_digest, build.key().config_digest);
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
        for f in ["desugar_sequence", "snapshot_identity", "top_scope", "expression_typing", "model", "claims", "lowering_view"] {
            assert_eq!(builds[f], 0, "{f} ran for a seed that did not parse");
        }
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
