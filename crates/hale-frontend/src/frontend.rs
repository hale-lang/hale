use super::imports::AliasScopes;
use std::collections::BTreeMap;
use std::process::ExitCode;
use super::imports::FileClaims;
use super::imports::ImportDiag;
use super::imports::ImportRenames;
use super::imports::ImportTarget;
use super::diag::IoDiag;
use std::path::Path;
use std::path::PathBuf;
use hale_syntax::ast::{EffectClasses, Program};
use super::workspace::find_workspace_root;
use super::source::SourceProvider;
use super::diag::render_diag_json;
use super::diag::render_located;
use super::imports::resolve_imports;
use super::imports::scope_import_aliases;
use super::imports::unscoped_alias_uses;

pub fn collect_target_files(
    t: &ImportTarget,
    src: &dyn SourceProvider,
) -> Result<Vec<PathBuf>, String> {
    match t {
        ImportTarget::SingleFile(p) => Ok(vec![p.clone()]),
        ImportTarget::Directory(d) => {
            let out = src.hl_files(d).map_err(|e| e.to_string())?;
            if out.is_empty() {
                return Err(format!(
                    "imported directory {} contains no .hl files",
                    d.display()
                ));
            }
            Ok(out)
        }
    }
}

/// Which files a load starts from, given the target it was handed.
///
/// One mode per entry-point shape, so the difference between the CLI's
/// load and the LSP's is this enum, not two walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LoadMode {
    /// The CLI (`check`, `build`, `run`, ...): the target as named — a
    /// directory is the seed, a file is a seed of one — and the loaders
    /// above then follow every `import` it declares
    /// ([`collect_checkable`]).
    WholeSeed,
    /// The LSP's: the whole seed as `hale check <dir>` lists it, from a
    /// file being edited. A FILE target stands for the directory around
    /// it (the F.19 seed of the file), and it is a member of that
    /// directory even when it exists only as an editor buffer; every
    /// `import` is then followed as [`LoadMode::WholeSeed`] follows it
    /// ([`link_checkable`]), through the same source provider, so a
    /// buffer wins over its file wherever the load reaches it. Where
    /// the CLI's load fails, the editor's keeps what it read: a member
    /// that does not parse or will not read is recorded
    /// (`Snapshot::unparsed`, `Snapshot::unreadable`), no import is
    /// followed, and the snapshot's scope is blocked.
    Editor,
}

/// The `.hl` files a load starts from: see [`LoadMode`].
pub fn collect_ap_files(
    target: &Path,
    mode: LoadMode,
    src: &dyn SourceProvider,
) -> Result<Vec<PathBuf>, String> {
    if src.is_dir(target) {
        let out = src.hl_files(target).map_err(|e| e.to_string())?;
        if out.is_empty() {
            return Err(format!("no .hl files in {}", target.display()));
        }
        return Ok(out);
    }
    match mode {
        LoadMode::WholeSeed => {
            if src.exists(target) {
                return Ok(vec![target.to_path_buf()]);
            }
            Err(format!("not a file or directory: {}", target.display()))
        }
        LoadMode::Editor => {
            let dir = seed_dir_of(target);
            // A directory that will not list leaves the target alone:
            // the file being edited is always checked.
            let mut out = src.hl_files(dir).unwrap_or_default();
            if !out.iter().any(|f| same_file(f, target)) {
                out.push(target.to_path_buf());
                out.sort();
            }
            Ok(out)
        }
    }
}

/// The directory a file target's seed is: its parent, `.` for a bare
/// file name.
pub fn seed_dir_of(file: &Path) -> &Path {
    match file.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Two spellings of one file: canonically equal when both are on disk,
/// else equal as written.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

#[allow(clippy::type_complexity)]
pub fn parse_files(
    files: &[PathBuf],
    src: &dyn SourceProvider,
) -> Result<
    (
        BTreeMap<PathBuf, Program>,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
        EffectClasses,
    ),
    ParseFailure,
> {
    // #345: one effect-class table for the load; every file interns
    // into it, so a class has one index in every file.
    let mut effects = EffectClasses::default();
    let mut programs: BTreeMap<PathBuf, Program> = BTreeMap::new();
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    // (virtual base, path, len) — each file parsed at a distinct base so
    // merged spans demultiplex back to their file (see parse_source_at).
    let mut file_bases: Vec<(u32, PathBuf, u32)> = Vec::new();
    let mut had_error = false;
    // GH #777: carried to the caller instead of printed here, so the
    // reporting path that honours `--json` sees them.
    let mut parse_diags: Vec<hale_syntax::Diag> = Vec::new();
    // GH #806: and so does a file that would not open, for the same
    // reason — it was the one remaining failure on this path still
    // printed here and nowhere else.
    let mut io_diags: Vec<IoDiag> = Vec::new();
    for f in files {
        let source = match src.read(f) {
            Ok(s) => s,
            Err(e) => {
                io_diags.push(IoDiag::read(
                    f,
                    &e,
                    format!("{}: {}", f.display(), e),
                ));
                had_error = true;
                continue;
            }
        };
        let base = file_bases.last().map(|(b, _, l)| b + l + 1).unwrap_or(0);
        file_bases.push((base, f.clone(), source.len() as u32));
        let parsed = crate::parse_cache::parse_file(src, f, &source, base, &mut effects);
        // A file that did not parse still contributes its text: the
        // renderers resolve a span against the source of the file whose
        // base window holds it, and that file is this one.
        sources.insert(f.clone(), source);
        match parsed {
            Ok(p) => {
                programs.insert(f.clone(), p);
            }
            Err(diags) => {
                parse_diags.extend(diags);
                had_error = true;
            }
        }
    }
    if had_error {
        return Err(ParseFailure {
            diags: parse_diags,
            io: io_diags,
            file_bases,
            sources,
        });
    }
    Ok((programs, sources, file_bases, effects))
}

/// GH #777: a file the target itself OWNS did not parse.
///
/// `parse_files` predates the JSON reporting path: it rendered each
/// parse diagnostic straight to stderr as text and handed its caller a
/// bare exit code, so `hale check --json` answered a syntactically
/// broken seed with a non-zero exit and an EMPTY NDJSON stream — a CI
/// gate, an admission step or an LSP client saw a real failure with
/// nothing explaining it, for a whole error class. That is the same
/// defect [`CheckableFailure`] closed for IMPORTED files (GH #765) and
/// the one `diag_reporting.rs` records for the old sync-inference
/// pre-pass. The diagnostics now travel to the caller, which renders
/// them through the two helpers every other finding goes through. The
/// file map travels with them because the spans are bundle-global
/// offsets, and the source map carries the files that did NOT parse
/// too — they have no program, but their text is what a span resolves
/// against.
pub struct ParseFailure {
    pub diags: Vec<hale_syntax::Diag>,
    /// GH #806: the target's own files that would not OPEN. They have
    /// no diagnostic — there is no text to raise one against — and
    /// they used to be printed here and counted only as a non-zero
    /// exit, so `check --json` on a seed it could not read emitted
    /// nothing at all.
    pub io: Vec<IoDiag>,
    pub file_bases: Vec<(u32, PathBuf, u32)>,
    pub sources: BTreeMap<PathBuf, String>,
}

impl ParseFailure {
    /// Render as located text on stderr — what `build` and `run` have
    /// always printed, neither of which has a machine-readable channel
    /// — and hand back the exit status. `check` and `verify` take the
    /// other road, through [`CheckableFailure::report`].
    pub fn report_text(&self) -> ExitCode {
        // The unreadable files first, in the order the walk hit them:
        // that is where they printed from when the read failed, and a
        // file that never opened explains anything else the seed is
        // missing.
        for io in &self.io {
            eprintln!("{}", io.text);
        }
        for d in &self.diags {
            eprintln!(
                "{}",
                render_located(d, &self.file_bases, &self.sources)
            );
        }
        ExitCode::from(1)
    }
}

/// GH #765: how [`collect_checkable`] failed.
///
/// `code` is the exit status. `diags` carries findings the CALLER
/// must render: something in the IMPORT GRAPH that did not parse, an
/// import that could not be resolved at all, or (GH #777) a file the
/// target itself owns that did not parse. `io` (GH #806) carries the
/// inputs that could not be READ — a target that is not there, a file
/// of the seed or of the import graph that would not open. Those have
/// to reach the same reporting path every other check diagnostic
/// takes, or `--json` emits nothing and the position comes out
/// against the wrong file — the trap the pre-pass resolver comment in
/// `run_check_impl_labelled` records. The file map travels with them
/// because the spans are bundle-global offsets into files the caller
/// never saw.
pub struct CheckableFailure {
    pub code: u8,
    pub diags: Vec<hale_syntax::Diag>,
    pub io: Vec<IoDiag>,
    pub file_bases: Vec<(u32, PathBuf, u32)>,
    pub sources: BTreeMap<PathBuf, String>,
}

impl CheckableFailure {
    /// GH #806: an input that could not be read. Nothing has been
    /// printed yet — [`Self::report`] prints the sentence on stderr
    /// in text mode and the record on stdout under `--json`.
    pub fn from_io(io: IoDiag) -> Self {
        Self {
            code: 1,
            diags: Vec::new(),
            io: vec![io],
            file_bases: Vec::new(),
            sources: BTreeMap::new(),
        }
    }

    /// GH #777: a parse failure in the target's OWN files, carried
    /// here rather than printed by `parse_files`, so `check --json`
    /// and `verify --json` report a syntactic failure as records like
    /// every other finding instead of as an empty stream.
    pub fn from_parse(f: ParseFailure) -> Self {
        Self {
            code: 1,
            diags: f.diags,
            io: f.io,
            file_bases: f.file_bases,
            sources: f.sources,
        }
    }

    /// The located text a command with no machine-readable channel
    /// prints (`build`, `run`, `test`, `replay`, `bench`): the
    /// unreadable inputs first, then the diagnostics, one per line —
    /// what [`Self::report`] prints on stderr outside `--json`.
    pub fn text(&self) -> String {
        self.io
            .iter()
            .map(|io| io.text.clone())
            .chain(self.diags.iter().map(|d| render_located(d, &self.file_bases, &self.sources)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Render through the same two helpers the checker's own findings
    /// go through — so `--json` carries the offending file, line and
    /// message, and a span resolves against the file it actually lives
    /// in — then hand back the exit status.
    pub fn report(&self) -> u8 {
        let json_mode = std::env::args().any(|a| a == "--json");
        // GH #806: an unreadable input is a record too, at line 0 /
        // col 0 — it has no position, because it has no text. It
        // comes first: a file that never opened is why anything else
        // here is missing.
        for io in &self.io {
            if json_mode {
                println!("{}", io.record());
            } else {
                eprintln!("{}", io.text);
            }
        }
        for d in &self.diags {
            if json_mode {
                println!(
                    "{}",
                    render_diag_json(d, &self.file_bases, &self.sources)
                );
            } else {
                eprintln!(
                    "{}",
                    render_located(d, &self.file_bases, &self.sources)
                );
            }
        }
        self.code
    }
}

/// Parse a check target, resolving cross-seed imports.
///
/// Returns the program map the analysis walks plus the
/// `alias::name -> mangled` table it needs to link calls into an
/// imported seed. A single file follows its own `import`s; a
/// directory bundles its `.hl` files as one seed and resolves the
/// union of their imports — the same shapes `hale build` handles, so
/// `check` and `build` finally agree about what a program contains.
/// The last element is that union as written, the target's own
/// `import`s (the build reads each library's `[ffi]` from it).
#[allow(clippy::type_complexity)]
pub fn collect_checkable(
    target: &Path,
    src: &dyn SourceProvider,
) -> Result<
    (
        BTreeMap<PathBuf, Program>,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
        ImportRenames,
        std::collections::BTreeSet<PathBuf>,
        Vec<hale_syntax::ast::Import>,
    ),
    CheckableFailure,
> {
    let (files, own, programs, sources, file_bases, effects) = parse_checkable(target, src)?;
    link_checkable(target, &files, own, programs, sources, file_bases, effects, src)
}

thread_local! {
    static SEED_LOADS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many whole-seed loads this thread began ([`parse_checkable`],
/// which `Snapshot::load` and [`collect_checkable`] both start with):
/// the accounting a test reads to pin that the environment matrix loads
/// each pair's seed once (F.40 phase 4, A3).
pub fn seed_loads_on_this_thread() -> u64 {
    SEED_LOADS.with(|n| n.get())
}

/// The first half of a whole seed's load: the target's own files,
/// each parsed at its own base, before any `import` is followed, and
/// the effect-class table they were parsed through (the load's one,
/// which the link continues). [`collect_checkable`] links them after;
/// the snapshot's whole-seed load keeps them too, as its members.
#[allow(clippy::type_complexity)]
pub fn parse_checkable(
    target: &Path,
    src: &dyn SourceProvider,
) -> Result<
    (
        Vec<PathBuf>,
        std::collections::BTreeSet<PathBuf>,
        BTreeMap<PathBuf, Program>,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
        EffectClasses,
    ),
    CheckableFailure,
> {
    SEED_LOADS.with(|n| n.set(n.get() + 1));
    let files = match collect_ap_files(target, LoadMode::WholeSeed, src) {
        Ok(f) => f,
        Err(e) => {
            // GH #806: a target that is not there, or whose directory
            // would not open, is a record under `--json` instead of a
            // sentence on stderr with an empty stream behind it.
            return Err(CheckableFailure::from_io(IoDiag::target(
                target, e,
            )));
        }
    };
    // GH #777: a parse failure in the target's own files travels the
    // same road an imported file's does — the diagnostics reach the
    // one reporting site, which honours `--json`.
    let (programs, sources, file_bases, effects) = parse_files(&files, src).map_err(|mut f| {
        // A FILE target that will not open is the entry of a build: its
        // text is the sentence every build path has printed for it
        // (GH #903, `could not read <path>: <os error>`). A directory's
        // file keeps `<path>: <os error>`; the record is the OS error
        // either way.
        if !src.is_dir(target) {
            for io in &mut f.io {
                io.text = format!("could not read {}: {}", io.path.display(), io.message);
            }
        }
        CheckableFailure::from_parse(f)
    })?;

    // The files the target itself owns — everything else reached
    // from here arrived through an `import`.
    let own: std::collections::BTreeSet<PathBuf> =
        files.iter().filter_map(|f| f.canonicalize().ok()).collect();
    Ok((files, own, programs, sources, file_bases, effects))
}

/// The second half of a whole seed's load, over its own files already
/// parsed: merge them, follow every `import` they declare, and scope
/// the aliases. [`collect_checkable`] calls it after its parse, and the
/// editor's load ([`LoadMode::Editor`]) after its own, so the two loads
/// link a seed by one path. `target` is the seed (the directory, or
/// the one file), `own` the target's files as the caller canonicalized
/// them, `effects` the effect-class table they were parsed through: the
/// imported seeds are parsed through it too, so the merged program's
/// classes need no renumbering.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn link_checkable(
    target: &Path,
    files: &[PathBuf],
    own: std::collections::BTreeSet<PathBuf>,
    programs: BTreeMap<PathBuf, Program>,
    sources: BTreeMap<PathBuf, String>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    effects: EffectClasses,
    src: &dyn SourceProvider,
) -> Result<
    (
        BTreeMap<PathBuf, Program>,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
        ImportRenames,
        std::collections::BTreeSet<PathBuf>,
        Vec<hale_syntax::ast::Import>,
    ),
    CheckableFailure,
> {
    // A single file with no imports: the old behaviour, exactly.
    // A MULTI-file seed merges below even without imports —
    // downstream handoff: the per-file programs sent each file
    // through `apply_sync_inference`'s single-program resolver
    // pass alone, so a `topic` declared in one file and
    // subscribed from a sibling reported "unknown topic" under
    // `check` while `build` (which merges the seed) resolved it.
    let has_imports = programs.values().any(|p| !p.imports.is_empty());
    if !has_imports && programs.len() <= 1 {
        return Ok((programs, sources, file_bases, Vec::new(), own, Vec::new()));
    }

    let union_imports: Vec<hale_syntax::ast::Import> = programs
        .values()
        .flat_map(|p| p.imports.iter().cloned())
        .collect();
    let merged = match merge_programs(programs.values()) {
        Some(m) => m,
        None => {
            return Err(CheckableFailure::from_io(IoDiag::target(
                target,
                format!("no .hl files in {}", target.display()),
            )));
        }
    };
    let workspace_root = find_workspace_root(target);
    // The load's one effect-class table: the imported seeds intern into
    // it as they are parsed.
    let mut effects = effects;
    let mut merged_items = merged.items;
    let mut renames: ImportRenames = Vec::new();
    let mut seed_cache: BTreeMap<
        PathBuf,
        std::collections::HashMap<String, String>,
    > = BTreeMap::new();
    let mut path_sources: BTreeMap<PathBuf, String> =
        sources.clone().into_iter().collect();
    let mut visited: std::collections::BTreeSet<PathBuf> =
        files.iter().filter_map(|f| f.canonicalize().ok()).collect();
    // GH #820: as on the entry path — the seed being checked is not
    // one of the libraries its imports name.
    let mut claims: FileClaims = FileClaims::new();
    let mut file_bases = file_bases;
    let mut errors: Vec<ImportDiag> = Vec::new();
    let importer_dir = if src.is_dir(target) {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    // GH #746: the check target is one seed; its files share one alias
    // namespace, which is the union of imports resolved just below.
    let target_scope =
        target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
    let mut alias_scopes = AliasScopes::new(&importer_dir, workspace_root.as_deref());
    alias_scopes.record_files(&target_scope, own.iter().cloned().collect());
    let resolve_failed = resolve_imports(
        &union_imports,
        &importer_dir,
        workspace_root.as_deref(),
        &mut visited,
        &mut claims,
        &mut path_sources,
        &mut file_bases,
        &mut errors,
        &mut merged_items,
        &mut renames,
        &mut seed_cache,
        &mut effects,
        &target_scope,
        &mut alias_scopes,
        src,
    )
    .is_err();
    // GH #765: `resolve_imports` reports an imported file's PARSE
    // failure by pushing it into `errors` and continuing — it still
    // returns Ok. This site used to test only the `Err`, so the
    // populated vector was never read: a library that did not parse
    // left its declarations silently absent from the merged program,
    // the checker's tolerance for unresolved qualified references hid
    // every consequence, and `check` / `verify` answered `ok` with exit
    // 0 on a tree `build` refused. An admission gate built on check +
    // verify was fail-open for any change that broke a library's
    // syntax. The other three call sites test the vector; this is the
    // one `check` and `verify` share.
    //
    // The diagnostics go back to the CALLER rather than being rendered
    // here, so they take the same path every other check finding does
    // — `--json` carries them, and the span resolves to the library
    // file instead of to whichever source happened to be first.
    if resolve_failed || !errors.is_empty() {
        // GH #806: the vector carries two shapes now — a positioned
        // diagnostic from a file that PARSED badly, and a file that
        // would not open at all. They split here because they render
        // differently: one resolves against `file_bases`, the other
        // has no position to resolve.
        let mut diags: Vec<hale_syntax::Diag> = Vec::new();
        let mut io: Vec<IoDiag> = Vec::new();
        for e in errors {
            match e {
                ImportDiag::Located { diag, .. } => diags.push(diag),
                ImportDiag::Io(d) => io.push(d),
            }
        }
        return Err(CheckableFailure {
            code: 1,
            diags,
            io,
            file_bases,
            sources: path_sources,
        });
    }

    let mut program = Program {
        // The load's table as the last imported seed's parse left it,
        // not `merged`'s own, which predates the imports.
        declared_effects: {
            let mut d = effects.declared;
            d.sort_unstable();
            d
        },
        effect_defs: effects.defs,
        effect_names: effects.names,
        imports: Vec::new(),
        items: merged_items,
        span: merged.span,
    };
    // GH #762: a qualified path must name an alias its OWN seed
    // declares — `check` is where that has to be said, since the
    // reference otherwise resolves through another seed's row and
    // nothing downstream ever sees a problem.
    let unscoped = unscoped_alias_uses(
        &program,
        &file_bases,
        &path_sources,
        &alias_scopes,
        &seed_cache,
    );
    if !unscoped.is_empty() {
        return Err(CheckableFailure {
            code: 1,
            diags: unscoped.into_iter().map(|u| u.diag).collect(),
            io: Vec::new(),
            file_bases,
            sources: path_sources,
        });
    }
    // GH #746: scope a contested alias to its declaring seed before
    // anything resolves through the table.
    scope_import_aliases(
        &mut program,
        &mut renames,
        &file_bases,
        &alias_scopes,
        &seed_cache,
    );
    // Same pre-pass `run`/`build` apply: rewrite qualified-path
    // TypeExprs to their mangled targets, so a cross-seed payload
    // type resolves instead of rendering as `?`.
    hale_types::mangle::apply_qualified_path_renames(&mut program, &renames);

    let mut out: BTreeMap<PathBuf, Program> = BTreeMap::new();
    out.insert(target.to_path_buf(), program);
    Ok((out, path_sources, file_bases, renames, own, union_imports))
}

/// GH #408 Phase 0: the bundle's source map, one unit per file of
/// `file_bases`, so a span resolves to a file outside this process.
/// Every path is relative to one root (an absolute path would make an
/// artifact differ per machine, and it is meant to be comparable), with
/// forward slashes so a Windows-built artifact matches a Linux-built
/// one. The snapshot seeds each site from it: `check` mints with it,
/// and every verb that builds mints and resolves with the same map, so
/// a site has the same seed on every path. It is also the program's one
/// naming of its files: the execution identity frames these paths, in
/// this order (F.40 phase 4, I3).
pub fn source_map(
    target: &Path,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> Vec<hale_types::symbol::SourceFile> {
    // Root at the WORKSPACE, not the target. An imported seed
    // usually lives outside the target directory (`apps/api`
    // importing `../../lib`), so relativizing to the target left
    // those paths absolute — and an artifact carrying absolute
    // paths differs per machine, which defeats the comparability
    // it exists for.
    //
    // The nearest ancestor holding a `hale.toml` is the natural
    // root when every source is under it: it is where a fleet plan's
    // repo-relative paths are anchored too. Otherwise (no manifest,
    // or a file above it: `app/` with its own `hale.toml` importing
    // `../lib`) the deepest common ancestor of every source, which is
    // always fully relativizing. Paths are canonicalized first: the
    // target's own file arrives as written on the command line while
    // imported seeds arrive absolute, so stripping without this left
    // the target's path relative to the CWD — and the same sources
    // checked from two directories produced two different artifacts.
    let abs: Vec<PathBuf> = file_bases
        .iter()
        .map(|(_, p, _)| {
            p.canonicalize()
                .or_else(|_| std::path::absolute(p))
                .unwrap_or_else(|_| p.clone())
        })
        .collect();
    let start = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let start = start.canonicalize().unwrap_or(start);
    let manifest_root = {
        let mut d = Some(start.as_path());
        let mut found = None;
        while let Some(cur) = d {
            if cur.join("hale.toml").exists() {
                found = Some(cur.to_path_buf());
                break;
            }
            d = cur.parent();
        }
        found
    };
    let manifest_root = manifest_root.filter(|m| abs.iter().all(|p| p.starts_with(m)));
    let root = manifest_root.unwrap_or_else(|| {
        let mut common: Option<PathBuf> = None;
        for p in &abs {
            let dir = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            common = Some(match common {
                None => dir,
                Some(c) => {
                    let mut shared = PathBuf::new();
                    for (a, b) in c.components().zip(dir.components()) {
                        if a != b {
                            break;
                        }
                        shared.push(a);
                    }
                    shared
                }
            });
        }
        common.unwrap_or(start)
    });
    file_bases
        .iter()
        .zip(&abs)
        .enumerate()
        .map(|(i, ((base, path, len), abs))| {
            let rel = abs
                .strip_prefix(&root)
                .unwrap_or(abs)
                .to_string_lossy()
                .replace('\\', "/");
            let digest = sources
                .get(path)
                .map(|src| {
                    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                    for b in src.as_bytes() {
                        h ^= *b as u64;
                        h = h.wrapping_mul(0x0000_0100_0000_01b3);
                    }
                    format!("{:016x}", h)
                })
                .unwrap_or_else(|| "unknown".to_string());
            hale_types::symbol::SourceFile {
                id: i as u32,
                path: rel,
                digest,
                base: *base,
                len: *len,
            }
        })
        .collect()
}

/// Drop WARNING-level diagnostics whose span resolves to a file the
/// check target does not own. Errors always survive.
pub fn retain_owned_advisories(
    diags: &mut Vec<hale_syntax::Diag>,
    own_files: &std::collections::BTreeSet<PathBuf>,
    file_bases: &[(u32, PathBuf, u32)],
) {
    if own_files.is_empty() || file_bases.len() <= 1 {
        return;
    }
    diags.retain(|d| {
        if d.is_error() {
            return true;
        }
        match file_of_span(d.span.start.0, file_bases) {
            Some(p) => {
                // Compare canonically. `file_bases` carries paths as
                // they were passed in (often relative) while
                // `own_files` is canonicalized, so a plain set lookup
                // silently reported EVERY file as foreign — including
                // the target's own, whose advisories then vanished.
                // Suppressing the user's own findings is far worse
                // than the noise this filter exists to remove.
                let canon = p.canonicalize().unwrap_or(p);
                own_files.contains(&canon)
            }
            // Unattributable span: keep it rather than silently drop.
            None => true,
        }
    });
}

/// Which file a merged span belongs to, via the per-file virtual
/// base offsets `resolve_imports` records.
pub fn file_of_span(
    pos: u32,
    file_bases: &[(u32, PathBuf, u32)],
) -> Option<PathBuf> {
    let mut best: Option<(u32, &PathBuf)> = None;
    for (base, path, len) in file_bases {
        if pos >= *base && pos < base.saturating_add(*len + 1) {
            if best.map(|(b, _)| *base >= b).unwrap_or(true) {
                best = Some((*base, path));
            }
        }
    }
    best.map(|(_, p)| p.clone())
}

/// Merge a set of parsed Programs into a single Program by
/// concatenating their items. Used by directory-target builds:
/// every .hl file in the directory contributes its top-level
/// decls to one bundle, in alphabetical filename order (per
/// `collect_ap_files`'s sort). Returns `None` if the iterator
/// yielded zero programs. Mirrors the merge step inside
/// the whole-seed load but without the import-following
/// (directory targets see every file by enumeration; nothing to
/// follow).
pub fn merge_programs<'a, I>(programs: I) -> Option<Program>
where
    I: IntoIterator<Item = &'a Program>,
{
    let programs: Vec<&Program> = programs.into_iter().collect();
    let first = *programs.first()?;
    // #345: every file of a load is parsed through one effect-class
    // table, so concatenating items renumbers nothing; the merged
    // program carries the table as the load's last parse left it, the
    // longest one, every other being a prefix of it.
    let table = programs
        .iter()
        .max_by_key(|p| p.effect_names.len())
        .expect("at least one program");
    let mut declared: Vec<u16> =
        programs.iter().flat_map(|p| p.declared_effects.iter().copied()).collect();
    declared.sort_unstable();
    declared.dedup();
    let merged = Program {
        declared_effects: declared,
        effect_defs: table.effect_defs.clone(),
        effect_names: table.effect_names.clone(),
        items: programs.iter().flat_map(|p| p.items.iter().cloned()).collect(),
        imports: Vec::new(),
        span: first.span,
    };
    Some(merged)
}
