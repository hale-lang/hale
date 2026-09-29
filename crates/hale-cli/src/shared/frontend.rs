use super::imports::AliasScopes;
use std::collections::BTreeMap;
use crate::EffectTable;
use std::process::ExitCode;
use super::imports::FileClaims;
use super::imports::ImportDiag;
use super::imports::ImportRenames;
use super::imports::ImportTarget;
use super::diag::IoDiag;
use std::path::Path;
use std::path::PathBuf;
use hale_syntax::ast::Program;
use super::workspace::find_workspace_root;
use std::fs;
use super::diag::render_diag_json;
use super::diag::render_located;
use super::imports::resolve_imports;
use super::imports::scope_import_aliases;
use super::imports::unscoped_alias_uses;
/// Parse a single-file entry, follow its `import "..." as alias;`
/// directives, and produce the merged Program + per-build path-
/// rename table. Imports inside imported libs ARE followed
/// recursively (A4, G34) — relative paths are resolved against
/// each lib's own directory so a two-hop chain
/// `app → lib → lib/_util` works. The mangled prefix embeds the
/// importer's alias, so two parallel paths to the same lib live
/// as separate compiled copies (per-importer namespacing). Cycles
/// are bounded by the canonical-path `visited` set.
/// Per-build entry context that Stage-2 FFI uses to walk imports
/// after resolution. The caller resolves imports once for normal
/// codegen; this context lets a second walk (just for FFI
/// manifest pickup) happen against the same lookup roots without
/// re-reading the entry file.
pub struct EntryCtx {
    pub entry_dir: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub imports: Vec<hale_syntax::ast::Import>,
}

pub(crate) fn parse_with_imports(
    entry: &Path,
) -> Result<
    (
        Program,
        ImportRenames,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
        EntryCtx,
    ),
    Vec<ImportDiag>,
> {
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut errors: Vec<ImportDiag> = Vec::new();
    let mut visited: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::new();
    // GH #820: the entry seed's own files are not claimed — a library
    // is what an `import` names, and the entry is not imported.
    let mut claims: FileClaims = FileClaims::new();

    let workspace_root = find_workspace_root(entry);
    let entry_dir = entry
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let entry_canon = entry.canonicalize().unwrap_or_else(|_| entry.to_path_buf());
    let entry_source = match fs::read_to_string(entry) {
        Ok(s) => s,
        Err(e) => {
            // GH #903: the last "print here, hand back nothing" site
            // on the import path. It printed the sentence itself and
            // returned an EMPTY vector, so every caller reported a
            // failure with no message — `hale test --json` emitted a
            // row whose `message` was the empty string. It travels as
            // an `ImportDiag::Io` like every other unreadable file of
            // the graph (GH #806), so the ONE rendering path prints
            // the same sentence and the `--json` channels carry it.
            errors.push(ImportDiag::Io(IoDiag::read(
                entry,
                &e,
                format!("could not read {}: {}", entry.display(), e),
            )));
            return Err(errors);
        }
    };
    let entry_program = match hale_syntax::parse_source(&entry_source) {
        Ok(p) => p,
        Err(diags) => {
            for d in diags {
                // The entry file is parsed unshifted (`parse_source`),
                // so its own base is 0.
                errors.push(ImportDiag::Located {
                    file: entry.to_path_buf(),
                    base: 0,
                    diag: d,
                    source: entry_source.clone(),
                });
            }
            return Err(errors);
        }
    };
    visited.insert(entry_canon.clone());
    // The entry file occupies base 0 (parse_source above = no shift);
    // imported files get subsequent virtual bases in resolve_imports.
    let mut file_bases: Vec<(u32, PathBuf, u32)> =
        vec![(0, entry_canon.clone(), entry_source.len() as u32)];
    // GH #746: the entry file is a seed of one, and its aliases are
    // scoped to it like any lib's.
    let entry_scope = entry_canon.clone();
    sources.insert(entry_canon, entry_source);

    let entry_imports = entry_program.imports.clone();
    let mut effects = EffectTable::from_seed(&entry_program);
    let mut merged_items = entry_program.items;
    // Seed the merged table with the ENTRY's classes so the entry's
    // own `User(i)` indices stay identity — its items are already in
    // `merged_items` and are never walked.
    let mut renames: ImportRenames = Vec::new();
    let mut seed_cache: BTreeMap<PathBuf, std::collections::HashMap<String, String>> = BTreeMap::new();
    let mut alias_scopes = AliasScopes::default();
    alias_scopes.record_files(&entry_scope, vec![entry_scope.clone()]);

    if resolve_imports(
        &entry_program.imports,
        &entry_dir,
        workspace_root.as_deref(),
        &mut visited,
        &mut claims,
        &mut sources,
        &mut file_bases,
        &mut errors,
        &mut merged_items,
        &mut renames,
        &mut seed_cache,
        &mut effects,
        &entry_scope,
        &mut alias_scopes,
    )
    .is_err()
    {
        return Err(errors);
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    // #345: user effect-class tables are per-seed — each seed interns
    // its own `effect NAME;` from zero, so the same index means a
    // DIFFERENT class in a different seed. `resolve_imports` unions the
    // names and rewrites each seed's indices into this table before
    // merging its items, so the merged program carries one table that
    // every `User(i)` in `merged_items` agrees on.
    let declared: Vec<u16> = effects.declared_indices();
    let effect_defs = effects.defs;
    let effect_names = effects.names;
    let mut merged = Program {
        effect_names,
        declared_effects: declared,
        effect_defs,
        imports: Vec::new(),
        items: merged_items,
        span: entry_program.span,
    };
    // GH #762: refuse a reference to an alias the seed it is written
    // in never declared, before the table can answer it out of
    // another seed's import row.
    let unscoped = unscoped_alias_uses(
        &merged,
        &file_bases,
        &sources,
        &alias_scopes,
        &seed_cache,
    );
    if !unscoped.is_empty() {
        for u in unscoped {
            // The span is in the merged coordinate space; the base it
            // was raised at comes back out of it at render time, the
            // same way every other entry in this vector does.
            let src = sources.get(&u.file).cloned().unwrap_or_default();
            errors.push(ImportDiag::Located {
                file: u.file,
                base: u.base,
                diag: u.diag,
                source: src,
            });
        }
        return Err(errors);
    }
    // GH #746: before anything resolves through the table, scope any
    // alias two seeds bound to different libs.
    scope_import_aliases(
        &mut merged,
        &mut renames,
        &file_bases,
        &alias_scopes,
        &seed_cache,
    );
    // brained F.1 (2026-05-23): rewrite `alias::Name` type
    // references in the entry program's TypeExprs to the
    // matching mangled single name. Lets the typechecker
    // resolve qualified-path cell types in @form annotations
    // (and any other TypeExpr position) the same way it
    // resolves bare type names. Codegen-side
    // `mangled_for_path` still handles expression-position
    // qualified paths separately — those don't round-trip
    // through typecheck so they stay opaque to it.
    hale_codegen::mangle::apply_qualified_path_renames(&mut merged, &renames);
    let ctx = EntryCtx {
        entry_dir,
        workspace_root,
        imports: entry_imports,
    };
    Ok((merged, renames, sources, file_bases, ctx))
}

pub(crate) fn collect_target_files(t: &ImportTarget) -> Result<Vec<PathBuf>, String> {
    match t {
        ImportTarget::SingleFile(p) => Ok(vec![p.clone()]),
        ImportTarget::Directory(d) => {
            let mut out = Vec::new();
            for entry in fs::read_dir(d).map_err(|e| e.to_string())? {
                let e = entry.map_err(|e| e.to_string())?;
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("hl") {
                    out.push(p);
                }
            }
            out.sort();
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

pub(crate) fn collect_ap_files(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    if target.is_dir() {
        let mut out: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(target).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("hl") {
                out.push(p);
            }
        }
        out.sort();
        if out.is_empty() {
            return Err(format!("no .hl files in {}", target.display()));
        }
        return Ok(out);
    }
    Err(format!("not a file or directory: {}", target.display()))
}

pub(crate) fn parse_files(
    files: &[PathBuf],
) -> Result<
    (
        BTreeMap<PathBuf, Program>,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
    ),
    ParseFailure,
> {
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
        let source = match fs::read_to_string(f) {
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
        let parsed = hale_syntax::parse_source_at(&source, base);
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
    Ok((programs, sources, file_bases))
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
pub(crate) struct ParseFailure {
    pub(crate) diags: Vec<hale_syntax::Diag>,
    /// GH #806: the target's own files that would not OPEN. They have
    /// no diagnostic — there is no text to raise one against — and
    /// they used to be printed here and counted only as a non-zero
    /// exit, so `check --json` on a seed it could not read emitted
    /// nothing at all.
    pub(crate) io: Vec<IoDiag>,
    pub(crate) file_bases: Vec<(u32, PathBuf, u32)>,
    pub(crate) sources: BTreeMap<PathBuf, String>,
}

impl ParseFailure {
    /// Render as located text on stderr — what `build` and `run` have
    /// always printed, neither of which has a machine-readable channel
    /// — and hand back the exit status. `check` and `verify` take the
    /// other road, through [`CheckableFailure::report`].
    pub(crate) fn report_text(&self) -> ExitCode {
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
pub(crate) struct CheckableFailure {
    pub(crate) code: u8,
    pub(crate) diags: Vec<hale_syntax::Diag>,
    pub(crate) io: Vec<IoDiag>,
    pub(crate) file_bases: Vec<(u32, PathBuf, u32)>,
    pub(crate) sources: BTreeMap<PathBuf, String>,
}

impl CheckableFailure {
    /// GH #806: an input that could not be read. Nothing has been
    /// printed yet — [`Self::report`] prints the sentence on stderr
    /// in text mode and the record on stdout under `--json`.
    pub(crate) fn from_io(io: IoDiag) -> Self {
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
    pub(crate) fn from_parse(f: ParseFailure) -> Self {
        Self {
            code: 1,
            diags: f.diags,
            io: f.io,
            file_bases: f.file_bases,
            sources: f.sources,
        }
    }

    /// Render through the same two helpers the checker's own findings
    /// go through — so `--json` carries the offending file, line and
    /// message, and a span resolves against the file it actually lives
    /// in — then hand back the exit status.
    pub(crate) fn report(&self) -> u8 {
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
#[allow(clippy::type_complexity)]
pub(crate) fn collect_checkable(
    target: &Path,
) -> Result<
    (
        BTreeMap<PathBuf, Program>,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
        ImportRenames,
        std::collections::BTreeSet<PathBuf>,
    ),
    CheckableFailure,
> {
    let files = match collect_ap_files(target) {
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
    let (programs, sources, file_bases) =
        parse_files(&files).map_err(CheckableFailure::from_parse)?;

    // The files the target itself owns — everything else reached
    // from here arrived through an `import`.
    let own: std::collections::BTreeSet<PathBuf> =
        files.iter().filter_map(|f| f.canonicalize().ok()).collect();

    // A single file with no imports: the old behaviour, exactly.
    // A MULTI-file seed merges below even without imports —
    // downstream handoff: the per-file programs sent each file
    // through `apply_sync_inference`'s single-program resolver
    // pass alone, so a `topic` declared in one file and
    // subscribed from a sibling reported "unknown topic" under
    // `check` while `build` (which merges the seed) resolved it.
    let has_imports = programs.values().any(|p| !p.imports.is_empty());
    if !has_imports && programs.len() <= 1 {
        return Ok((programs, sources, file_bases, Vec::new(), own));
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
    let mut effects = EffectTable::from_seed(&merged);
    let mut merged_items = merged.items;
    // Same identity-seeding rule as the entry path: `merged`'s own
    // items are already in `merged_items` and are never walked, so its
    // table must come first.
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
    let importer_dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    // GH #746: the check target is one seed; its files share one alias
    // namespace, which is the union of imports resolved just below.
    let target_scope =
        target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
    let mut alias_scopes = AliasScopes::default();
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
        // The UNIONED table from the merge above, not `merged`'s own —
        // `merged.effect_names` is the pre-import table and every
        // imported seed's `User(i)` was remapped into this one.
        declared_effects: effects.declared_indices(),
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
    hale_codegen::mangle::apply_qualified_path_renames(&mut program, &renames);

    let mut out: BTreeMap<PathBuf, Program> = BTreeMap::new();
    out.insert(target.to_path_buf(), program);
    Ok((out, path_sources, file_bases, renames, own))
}

/// Drop WARNING-level diagnostics whose span resolves to a file the
/// check target does not own. Errors always survive.
pub(crate) fn retain_owned_advisories(
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
pub(crate) fn file_of_span(
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
/// `parse_with_imports` but without the import-following
/// (directory targets see every file by enumeration; nothing to
/// follow).
pub(crate) fn merge_programs<'a, I>(programs: I) -> Option<Program>
where
    I: IntoIterator<Item = &'a Program>,
{
    let mut iter = programs.into_iter();
    let first = iter.next()?;
    // #345: same per-seed index hazard as the import path. Each file
    // interns its own `effect NAME;` from zero, so concatenating items
    // without remapping makes file A's class 0 and file B's class 0
    // the same bit — `@effects(none: {money})` in one file would then
    // be checked against `pii` in another. (Observed: the diagnostic
    // reported reaching `pii` for a `none: {money}` assertion.)
    let mut effects = EffectTable::default();
    let mut take = |p: &Program| -> Vec<hale_syntax::ast::TopDecl> {
        let mut items = p.items.clone();
        if !p.effect_names.is_empty() {
            let map = effects.absorb(p);
            hale_syntax::ast::remap_user_effects(&mut items, &map);
        }
        items
    };
    let mut items = take(first);
    for p in iter {
        items.extend(take(p));
    }
    let merged = Program {
        declared_effects: effects.declared_indices(),
        effect_defs: effects.defs,
        effect_names: effects.names,
        items,
        imports: Vec::new(),
        span: first.span,
    };
    Some(merged)
}
