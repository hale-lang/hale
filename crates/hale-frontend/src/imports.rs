use std::collections::BTreeMap;
use hale_syntax::ast::EffectClasses;
use super::diag::IoDiag;
use std::path::Path;
use std::path::PathBuf;
use hale_syntax::ast::Program;
use super::frontend::collect_target_files;
use super::diag::diag_file_name;
use super::diag::display_relative;
use super::source::SourceProvider;
use super::workspace::library_basis;
use super::workspace::library_id;
use super::workspace::seed_dir_for_entry_file;
use super::workspace::top_decl_ident;
/// Per-build path-rename table for cross-seed imports
/// (v1.x-IMPORT). Each entry maps a qualified-name segment vector
/// (e.g. `["foo", "Bar"]`) to the mangler-generated symbol name
/// (`__lib_foo_<stem>_Bar`). Passed to
/// `build_executable_with_imports` so codegen can resolve
/// `alias::Name` references in user code.
pub type ImportRenames = Vec<(Vec<String>, String)>;

/// GH #746: who declared which import alias, so an alias can be
/// scoped to its declaring seed the way the language scopes it.
///
/// An alias is seed-scoped (spec `projects.md`, "Scoped imports
/// (A4)"): a lib's imports are reachable inside its own body only.
/// `ImportRenames` above is one table per BUILD, keyed by the alias as
/// written, so two seeds that spell different libs `u` used to collide
/// in it — last row won and both seeds resolved to one lib, with no
/// diagnostic anywhere (`hale check` passed, the binary computed the
/// wrong value). This records the bindings as they are made; after
/// resolution, `scope_import_aliases` gives every binder of a
/// contested alias its own head and re-heads that seed's own
/// references, so the one table can tell the two apart.
pub struct AliasScopes {
    /// One row per import site: (declaring seed, alias, the lib the
    /// alias names). Seeds and libs are canonical paths — the same
    /// identity `seed_cache` and `name_library` key off, so two
    /// aliases for the same lib agree and never look contested.
    pub bindings: Vec<(PathBuf, String, PathBuf)>,
    /// Declaring seed -> its source files, canonical. A seed's files
    /// share one alias namespace (they share one decl namespace), so
    /// the rewrite applies to all of them.
    pub files: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// The two anchors a library's name is relative to, canonical:
    /// the entry's workspace root, and the entry seed's directory.
    workspace_root: Option<PathBuf>,
    entry_dir: PathBuf,
}

impl AliasScopes {
    /// The scopes of one load, whose entry seed lives in `entry_dir`
    /// and whose workspace (if any) is rooted at `workspace_root`.
    pub fn new(entry_dir: &Path, workspace_root: Option<&Path>) -> Self {
        // `hale run main.hl` reaches here with the entry's parent, the
        // empty path, which is the working directory.
        let canon = |p: &Path| {
            let p = if p.as_os_str().is_empty() { Path::new(".") } else { p };
            p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
        };
        AliasScopes {
            bindings: Vec::new(),
            files: BTreeMap::new(),
            workspace_root: workspace_root.map(canon),
            entry_dir: canon(entry_dir),
        }
    }

    /// The name a library's symbols are mangled under — the mangler's
    /// namespace key, so two apps importing the same lib produce
    /// identical mangled symbols (cross-app DTO contracts become
    /// symbol-identical without any annotation or config flag).
    ///
    /// The name is a function of the library alone: its canonical
    /// path (the identity `seed_cache` and `bindings` key off) taken
    /// relative to the workspace root, or to the entry seed's
    /// directory for a library outside the workspace
    /// ([`library_basis`]), then encoded injectively ([`library_id`]).
    /// Neither the import order nor the other libraries of the load
    /// enter into it, two libraries never share a name, and a tree
    /// moved or cloned as a whole keeps every name.
    pub fn name_library(&self, lib: &Path, directory: bool) -> String {
        let basis = library_basis(lib, self.workspace_root.as_deref(), &self.entry_dir);
        library_id(&basis, directory)
    }

    pub fn record_binding(&mut self, seed: &Path, alias: &str, lib: &Path) {
        self.bindings.push((
            seed.to_path_buf(),
            alias.to_string(),
            lib.to_path_buf(),
        ));
    }

    pub fn record_files(&mut self, seed: &Path, files: Vec<PathBuf>) {
        self.files
            .entry(seed.to_path_buf())
            .or_default()
            .extend(files);
    }
}

/// What an `import "path" as alias;` resolved to on disk.
pub enum ImportTarget {
    /// `<importer_dir>/<path>.hl` (single-file lib).
    SingleFile(PathBuf),
    /// `<importer_dir>/<path>/` or `<workspace_root>/<path>/`
    /// (directory bundle — one seed of multiple `.hl` files).
    Directory(PathBuf),
}

/// Try the three resolution strategies in order: entry-relative
/// single file, entry-relative directory, workspace-root directory.
/// Returns `None` if none of them hit.

pub fn resolve_import(
    importer_dir: &Path,
    workspace_root: Option<&Path>,
    import_path: &str,
    src: &dyn SourceProvider,
) -> Option<ImportTarget> {
    let single = importer_dir.join(format!("{}.hl", import_path));
    if src.exists(&single) {
        // GH #763: the entry file of a seed is the seed.
        if let Some(dir) = seed_dir_for_entry_file(&single, importer_dir, src) {
            return Some(ImportTarget::Directory(dir));
        }
        return Some(ImportTarget::SingleFile(single));
    }
    let dir_local = importer_dir.join(import_path);
    if src.is_dir(&dir_local) {
        return Some(ImportTarget::Directory(dir_local));
    }
    if let Some(root) = workspace_root {
        let dir_root = root.join(import_path);
        if src.is_dir(&dir_root) {
            return Some(ImportTarget::Directory(dir_root));
        }
    }
    None
}

/// GH #775: one diagnostic raised while resolving the import graph.
///
/// Every file of the graph is parsed at its own virtual base
/// (`parse_source_at`), so a diagnostic's span is an offset into the
/// whole BUNDLE, not into the file it was raised in. The file's own
/// text travels with the diagnostic so a caller can render it; the
/// `base` is what turns one coordinate space into the other.
///
/// Without it, the bare `d.render(source)` every consumer of this
/// vector used read a bundle offset as a position in a file that is
/// almost always shorter than the offset — so the file name and the
/// message came out right and the line and column did not, on
/// `build`, `run`, `test`, `bench` and `replay` alike (`check` and
/// `verify` take the other road, through [`CheckableFailure`], which
/// has demultiplexed through `file_bases` since GH #770). Render
/// through [`ImportDiag::render`], never by hand.
///
/// GH #806: a file of the graph that would not OPEN rides in the same
/// vector, as [`ImportDiag::Io`]. It is the same failure to the
/// callers — the import graph is incomplete, so the tree is refused —
/// and putting it here is what carries it to them: the resolver used
/// to print it and return a bare `Err(())`, which left `--json`
/// empty. Every consumer of this vector already reports what is in
/// it.
pub enum ImportDiag {
    /// A diagnostic raised in a file the resolver PARSED.
    Located {
        /// The file the diagnostic was raised in, as the resolver
        /// reached it. What the user sees is [`diag_file_name`] of
        /// it (GH #822) — the resolver reaches a library through the
        /// importer's own directory, so the path it holds is
        /// routinely `app/../lib/second.hl`, and `check`, which
        /// resolves the same diagnostic through the canonical
        /// `file_bases`, named the file differently from `build`.
        file: PathBuf,
        /// The virtual base `file` was parsed at; 0 for the entry
        /// file, which the load parses unshifted.
        base: u32,
        diag: hale_syntax::Diag,
        /// `file`'s own text: what the un-shifted span is resolved
        /// against, and the snippet under the message is cut from.
        source: String,
    },
    /// GH #806: a file of the graph that could not be read at all.
    /// It has no position to render — no text was ever loaded — so
    /// [`ImportDiag::render`] prints the sentence its site printed
    /// when the failure was an `eprintln!` there.
    Io(IoDiag),
}

impl ImportDiag {
    /// `path:line:col: kind: message`, positioned in the file that
    /// holds the error — the same rendering `render_located` produces
    /// for a merged-bundle span, and the one `check` prints.
    ///
    /// It reaches `Diag::render_located` directly rather than
    /// searching `file_bases`: the entry already knows its own file
    /// and base, so there is no window to test and no call to
    /// [`hale_syntax::file_owns_offset`].
    ///
    /// The FILE goes through [`diag_file_name`] (GH #822), which is
    /// what the `file_bases` road does too — one spelling, whichever
    /// road a command takes. [`ImportDiag::Io`]'s `text` is the
    /// sentence its failing site composed, so it is printed as it
    /// stands; the path in its RECORD goes through the same rule
    /// ([`IoDiag::record`]).
    pub fn render(&self) -> String {
        match self {
            ImportDiag::Located {
                file,
                base,
                diag,
                source,
            } => diag.render_located(&diag_file_name(file), source, *base),
            ImportDiag::Io(io) => io.text.clone(),
        }
    }
}

/// GH #860: the `import` that names nothing, as a LOCATED
/// diagnostic under its path literal.
///
/// GH #820's refusal — a single file of a library another import
/// already took as a directory — is raised the same way: it is a
/// finding about an `import` statement, positioned under the path
/// literal that has to change, and this is the one road from a
/// merged-bundle span to the file that owns it.
///
/// The resolver printed `could not resolve import "..."` and
/// returned a bare `Err(())`, which left the `errors` vector empty:
/// after GH #806 it was the last failure on the check path still
/// answering `--json` with an empty stream and exit 1. It is not an
/// io failure — nothing was opened, so there is no OS error to
/// report — and the statement HAS a span, so it belongs in
/// [`ImportDiag::Located`] like an imported file's parse error, and
/// every channel renders it with no new path: `render_diag_json`
/// under `--json`, `render_located` in text, [`ImportDiag::render`]
/// on `build` / `run` / `test`.
///
/// The importing FILE is recovered from the span through the
/// file-base table rather than passed in, because one call resolves
/// the UNION of a seed's imports (`collect_checkable`, and the
/// directory forms of `build` and `run`): the importer is
/// per-import, and the span is the only thing that knows which file
/// an import was written in.
///
/// `sources` is filled in when the owning file is missing from it.
/// A library file's text is inserted only AFTER its own imports are
/// followed, so an unresolvable import INSIDE a library would
/// otherwise reach `check` with a file window and no text to resolve
/// a position against — and `render_located` would fall back to
/// rendering it against whatever source came first.
pub fn unresolved_import_diag(
    path_span: hale_syntax::Span,
    message: String,
    fallback: IoDiag,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &mut BTreeMap<PathBuf, String>,
    src: &dyn SourceProvider,
) -> ImportDiag {
    let off = path_span.start.as_usize() as u32;
    for (base, path, len) in file_bases {
        if !hale_syntax::file_owns_offset(*base, *len, off) {
            continue;
        }
        let source = match sources.get(path) {
            Some(s) => s.clone(),
            None => match src.read(path) {
                Ok(s) => {
                    sources.insert(path.clone(), s.clone());
                    s
                }
                // The file was read once already to be parsed; if it
                // will not open now, the positionless record is
                // still a record.
                Err(_) => break,
            },
        };
        return ImportDiag::Located {
            file: path.clone(),
            base: *base,
            diag: hale_syntax::Diag::ty(path_span, message),
            source,
        };
    }
    ImportDiag::Io(fallback)
}

/// GH #820: which library a file of the build belongs to, and the
/// `import` that put it there.
#[derive(Clone)]
pub struct FileClaim {
    /// The library's identity, as `seed_cache` keys it: the
    /// canonical directory for a directory import, the canonical
    /// file for a single-file one.
    pub lib_key: PathBuf,
    /// The alias the claiming import bound.
    pub alias: String,
    /// The path as WRITTEN in the claiming import, which is what the
    /// author has to change.
    pub import_path: String,
    /// The claiming import's path literal, for a located message.
    pub path_span: hale_syntax::Span,
    /// Did the claim come from the single-FILE spelling (resolution
    /// rule 1) rather than from a directory?
    pub single_file: bool,
}

/// Canonical file path -> the library that claimed it.
pub type FileClaims = BTreeMap<PathBuf, FileClaim>;

/// GH #820: `<file>:<line>` for an import's path literal.
///
/// The position of the OTHER import in a two-import conflict, quoted
/// inside the message so every channel carries it — a `related`
/// location would be dropped by [`ImportDiag::render`], which is what
/// `build`, `run` and `test` print.
///
/// `sources` alone is not enough: a library file's text is inserted
/// only AFTER its own imports are followed, so an `import` written
/// inside a library is not in it yet when a conflict is found. The
/// file is then read from disk, for the same reason
/// [`unresolved_import_diag`] reads it.
pub fn import_site(
    path_span: hale_syntax::Span,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
    src: &dyn SourceProvider,
) -> Option<String> {
    let off = path_span.start.as_usize() as u32;
    for (base, path, len) in file_bases {
        if !hale_syntax::file_owns_offset(*base, *len, off) {
            continue;
        }
        let text = match sources.get(path) {
            Some(s) => s.clone(),
            None => src.read(path).ok()?,
        };
        let (line, _) =
            path_span.shifted(base.wrapping_neg()).line_col(&text);
        return Some(format!("{}:{}", diag_file_name(path), line));
    }
    None
}

/// GH #820: a file of the build belongs to exactly ONE library.
///
/// `visited` is global across the build and holds canonical paths, so
/// whichever identity resolves second finds the files the first
/// already took and parses none of them. For the two spellings of one
/// library that is right — GH #763 made them the same `lib_key`, and
/// the second alias registers against the first's mangled names out
/// of `seed_cache`. For a non-entry single file of a
/// directory-imported library it is not: `import "../lib/helper" as h`
/// and `import "../lib" as a` are genuinely DIFFERENT libraries (one
/// file against the whole seed), and merge-once has no way to give
/// `helper.hl` two manglings. So whichever resolved second got only
/// the files the other did not take, and `a::helper_name` — or
/// `h::helper_name`, depending on the order — silently resolved to
/// nothing (GH #820).
///
/// The ruling (2026-09-20, GH #911): refuse the FILE spelling, at the
/// import that has to change, naming the library that already holds
/// the file and where that import is written. The message is located
/// at the single-file import in BOTH resolution orders — it is the
/// spelling being refused, so pointing at the directory import
/// instead would name a line there is nothing wrong with.
///
/// Two DIRECTORY identities can only meet over one file through a
/// symlink (a file's parent directory is otherwise unique). There is
/// no file spelling to refuse there, so the second import is refused
/// and the message says which file is claimed twice.
pub fn claim_library_files(
    files: &[PathBuf],
    lib_key: &Path,
    alias: &str,
    imp: &hale_syntax::ast::Import,
    single_file: bool,
    claims: &mut FileClaims,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &mut BTreeMap<PathBuf, String>,
    src: &dyn SourceProvider,
) -> Option<ImportDiag> {
    for file in files {
        let canon = file.canonicalize().unwrap_or_else(|_| file.clone());
        let fresh = FileClaim {
            lib_key: lib_key.to_path_buf(),
            alias: alias.to_string(),
            import_path: imp.path.clone(),
            path_span: imp.path_span,
            single_file,
        };
        let Some(prev) = claims.get(&canon).cloned() else {
            claims.insert(canon, fresh);
            continue;
        };
        if prev.lib_key == lib_key {
            continue;
        }
        let (refused, other) = if prev.single_file {
            (&prev, &fresh)
        } else {
            (&fresh, &prev)
        };
        let at = match import_site(other.path_span, file_bases, sources, src) {
            Some(s) => format!(" at {}", s),
            None => String::new(),
        };
        let message = if refused.single_file {
            format!(
                "`{}` is already part of the library imported as `{}`{}; \
                 a single file of a directory-imported library is not a \
                 library of its own — reach its declarations as \
                 `{}::<name>` and drop this import",
                refused.import_path, other.alias, at, other.alias,
            )
        } else {
            format!(
                "{} is already part of the library imported as `{}`{}; \
                 a file belongs to one library, so these two imports \
                 cannot both hold it",
                diag_file_name(&canon),
                other.alias,
                at,
            )
        };
        return Some(unresolved_import_diag(
            refused.path_span,
            message.clone(),
            IoDiag::target(&canon, message),
            file_bases,
            sources,
            src,
        ));
    }
    None
}

pub fn resolve_imports(
    imports: &[hale_syntax::ast::Import],
    importer_dir: &Path,
    workspace_root: Option<&Path>,
    visited: &mut std::collections::BTreeSet<PathBuf>,
    // GH #820: which library each file of the build belongs to. The
    // companion of `visited`: that set says a file has been taken,
    // this map says by WHOM, which is what makes a second claim on
    // one file reportable instead of silent.
    claims: &mut FileClaims,
    sources: &mut BTreeMap<PathBuf, String>,
    // Per-file (virtual base offset, canonical path, byte length). Each
    // file is parsed at a distinct base so merged spans are globally
    // unique and a diagnostic can be demultiplexed back to its file.
    file_bases: &mut Vec<(u32, PathBuf, u32)>,
    errors: &mut Vec<ImportDiag>,
    merged_items: &mut Vec<hale_syntax::ast::TopDecl>,
    renames: &mut ImportRenames,
    // iris F.10: per-canonical-lib seed_renames cache. A lib
    // reached a SECOND time (another importer, its own alias)
    // has all files in `visited`, so the parse+mangle work is
    // rightly skipped — but the new alias must still register
    // against the lib's mangled names, or every `alias::Name`
    // in the second importer leaks unrenamed into codegen
    // ("qualified type `g::Rect` not in stdlib path-renames
    // table" / "unknown type name in signature").
    seed_cache: &mut BTreeMap<PathBuf, std::collections::HashMap<String, String>>,
    // #345: the load's one user effect-class table. Every seed's items
    // are parsed through it, so a class is one index in every seed and
    // nothing is renumbered when the items are merged. A seed interns
    // after the seeds it imports, as the classes have always been
    // numbered.
    effects: &mut EffectClasses,
    // GH #746: the seed whose imports these are — the canonical path
    // of the entry target, or of the lib whose files are being
    // followed. Every alias in `imports` is recorded against it.
    scope_key: &Path,
    alias_scopes: &mut AliasScopes,
    // Where every file of the graph is read from (F.40 phase 2.1a).
    src: &dyn SourceProvider,
) -> Result<(), ()> {
    // Defensive guards + env-gated tracing. The guards bound the
    // resolver's accumulators so a future bug (or pathological
    // input) can't OOM the machine — pond surfaced a 27 GB freeze
    // 2026-05-17 when an upstream parser bug looped on mis-ordered
    // imports; that's fixed in hale-syntax now, but the caps stay
    // as a generic backstop. Real workloads sit ~1000x below the
    // ceilings (pond's largest demo: visited=14, renames=51).
    // HALE_IMPORT_DEBUG=1 enables per-call tracing for future
    // import-resolution debugging.
    if std::env::var("HALE_IMPORT_DEBUG").is_ok() {
        eprintln!(
            "[import] entry: dir={} imports={} visited={} renames={} merged_items={}",
            importer_dir.display(),
            imports.len(),
            visited.len(),
            renames.len(),
            merged_items.len(),
        );
    }
    if visited.len() > 2000 {
        eprintln!(
            "[import] ABORT: visited > 2000 ({}); recursion runaway, importer={}",
            visited.len(),
            importer_dir.display(),
        );
        std::process::exit(99);
    }
    if renames.len() > 200_000 {
        eprintln!(
            "[import] ABORT: renames > 200k ({}); rename-table runaway, importer={}",
            renames.len(),
            importer_dir.display(),
        );
        std::process::exit(99);
    }
    if merged_items.len() > 200_000 {
        eprintln!(
            "[import] ABORT: merged_items > 200k ({}); item-merge runaway, importer={}",
            merged_items.len(),
            importer_dir.display(),
        );
        std::process::exit(99);
    }
    for imp in imports {
        // `import "std" as ...;` would be malformed at the spec
        // level — std is the bundled namespace, not a vendored
        // lib. Defensive skip; the parser doesn't reject it yet.
        if imp.path.starts_with("std/") || imp.path == "std" {
            continue;
        }
        let alias = match &imp.alias {
            Some(a) => a.clone(),
            None => continue, // v1.x-IMPORT PR1 enforces; defensive.
        };
        let target = match resolve_import(importer_dir, workspace_root, &imp.path, src) {
            Some(t) => t,
            None => {
                // GH #860: the three places the resolver looked,
                // which is what makes the failure actionable — the
                // same list it printed on stderr, now the body of a
                // diagnostic positioned under the path literal.
                let tried = format!(
                    "tried {}/{}.hl, {}/{}/, and workspace-root/{}/",
                    importer_dir.display(),
                    imp.path,
                    importer_dir.display(),
                    imp.path,
                    imp.path,
                );
                errors.push(unresolved_import_diag(
                    imp.path_span,
                    format!(
                        "could not resolve import `{}` ({})",
                        imp.path, tried
                    ),
                    // The sentence the `eprintln!` printed, for the
                    // case no file window owns the span (there is
                    // none: every import comes from a parsed file).
                    IoDiag::target(
                        importer_dir,
                        format!(
                            "could not resolve import \"{}\": {}",
                            imp.path, tried
                        ),
                    ),
                    file_bases,
                    sources,
                    src,
                ));
                return Err(());
            }
        };
        let files = match collect_target_files(&target, src) {
            Ok(f) => f,
            Err(e) => {
                // GH #806: the sentence goes to the caller in the
                // errors vector rather than to stderr here, so the
                // one caller with a machine-readable channel can
                // emit it as a record instead.
                let path = match &target {
                    ImportTarget::Directory(d) => d.clone(),
                    ImportTarget::SingleFile(f) => f.clone(),
                };
                errors.push(ImportDiag::Io(IoDiag::target(
                    &path,
                    format!("import \"{}\": {}", imp.path, e),
                )));
                return Err(());
            }
        };
        // GH #746: the lib's identity, as `seed_cache` keys it — one
        // key per lib however many aliases reach it.
        let lib_key = match &target {
            ImportTarget::Directory(d) => {
                d.canonicalize().unwrap_or_else(|_| d.clone())
            }
            ImportTarget::SingleFile(f) => {
                f.canonicalize().unwrap_or_else(|_| f.clone())
            }
        };
        alias_scopes.record_binding(scope_key, &alias, &lib_key);
        // GH #820: before anything is parsed, check that no file of
        // this target already belongs to another library — the
        // `visited` skip below would otherwise hand this alias a
        // partial file set with no diagnostic anywhere.
        if let Some(conflict) = claim_library_files(
            &files,
            &lib_key,
            &alias,
            imp,
            matches!(target, ImportTarget::SingleFile(_)),
            claims,
            file_bases,
            sources,
            src,
        ) {
            errors.push(conflict);
            return Err(());
        }
        // Parse every file in the import target into a parallel
        // (file_path, stem, source, Program) list, recording the
        // canon path in `visited` so we don't double-parse.
        struct ParsedLibFile {
            path: PathBuf,
            canon: PathBuf,
            stem: String,
            source: String,
            base: u32,
            /// The file's tokens, parsed again through the load's
            /// effect-class table once its imports are resolved.
            tokens: Vec<hale_syntax::Token>,
            /// The first parse, through a table of its own: its
            /// diagnostics, its imports and its declarations' names.
            program: hale_syntax::ast::Program,
        }
        let mut parsed_files: Vec<ParsedLibFile> = Vec::new();
        for file in files {
            let canon = file.canonicalize().unwrap_or_else(|_| file.clone());
            if !visited.insert(canon.clone()) {
                continue;
            }
            let source = match src.read(&file) {
                Ok(s) => s,
                Err(e) => {
                    // GH #806: an imported file that will not open is
                    // reported like an imported file that will not
                    // parse — carried to the caller, which is the
                    // side that knows whether records or text are
                    // wanted. It used to print here and return a bare
                    // `Err(())`, so `check --json` said nothing at
                    // all.
                    errors.push(ImportDiag::Io(IoDiag::read(
                        &file,
                        &e,
                        format!(
                            "could not read imported file {} (from import \"{}\"): {}",
                            file.display(),
                            imp.path,
                            e
                        ),
                    )));
                    return Err(());
                }
            };
            let trace = std::env::var("HALE_IMPORT_DEBUG").is_ok();
            if trace {
                eprintln!("[import]     parse start: {}", file.display());
            }
            let base = file_bases
                .last()
                .map(|(b, _, l)| b + l + 1)
                .unwrap_or(0);
            file_bases.push((base, canon.clone(), source.len() as u32));
            let parsed = hale_syntax::lex_at(&source, base).and_then(|tokens| {
                let program = hale_syntax::parser::parse_in(
                    tokens.clone(),
                    &source,
                    &mut EffectClasses::default(),
                )?;
                Ok((tokens, program))
            });
            let (tokens, program) = match parsed {
                Ok(p) => p,
                Err(diags) => {
                    for d in diags {
                        // GH #775: the base travels with the
                        // diagnostic. `d`'s span is an offset into the
                        // merged bundle; `source` is this file alone.
                        errors.push(ImportDiag::Located {
                            file: file.clone(),
                            base,
                            diag: d,
                            source: source.clone(),
                        });
                    }
                    sources.insert(canon, source);
                    continue;
                }
            };
            if trace {
                eprintln!(
                    "[import]     parse done : {} (items={} imports={})",
                    file.display(),
                    program.items.len(),
                    program.imports.len(),
                );
            }
            let stem = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unnamed")
                .to_string();
            parsed_files.push(ParsedLibFile {
                path: file,
                canon,
                stem,
                source,
                base,
                tokens,
                program,
            });
        }
        if parsed_files.is_empty() {
            // Every file already visited: the lib was resolved
            // earlier under some other alias. Its decls are
            // merged and mangled; only THIS alias's rename rows
            // are missing. name_library keys mangled names
            // off the canonical path, so both aliases map to the
            // same single compiled copy.
            if let Some(cached) = seed_cache.get(&lib_key) {
                for (name, mangled) in cached {
                    renames.push((
                        vec![alias.clone(), name.clone()],
                        mangled.clone(),
                    ));
                }
            }
            continue;
        }
        // Build the unified rename map across every file in this
        // import target. Cross-file references inside the lib
        // (e.g. greet.hl uses a type declared in format.hl)
        // resolve through this shared map.
        let stem_prog_refs: Vec<(String, &hale_syntax::ast::Program)> = parsed_files
            .iter()
            .map(|f| (f.stem.clone(), &f.program))
            .collect();
        let trace = std::env::var("HALE_IMPORT_DEBUG").is_ok();
        if trace {
            eprintln!("[import]     build_seed_renames start (n_files={})", parsed_files.len());
        }
        // Compute a stable identifier for this lib derived from
        // the canonical path of its directory (or file). Same lib →
        // same id → same mangled names across importers. The
        // user-chosen `alias` is still used as the call-site
        // reference (`alias::Name`) in the path-rename table below,
        // but the mangled symbols themselves come from the path
        // identity.
        let lib_id = alias_scopes
            .name_library(&lib_key, matches!(target, ImportTarget::Directory(_)));
        let seed_renames =
            hale_types::mangle::build_seed_renames(&stem_prog_refs, &lib_id);
        // GH #714: the names that may head a qualified path in this
        // seed (its type decls). Everything else a path head can be
        // is a module alias of the seed's own imports, which the
        // mangler must leave intact so `alias::Name` still resolves
        // through the rename table — even when the seed also
        // declares a free fn of the alias's name.
        let seed_heads =
            hale_types::mangle::seed_path_heads(&stem_prog_refs);
        // GH #774: the seed as a whole, for binding a claim group
        // reference no declaration in it answers.
        let seed_binding = hale_types::mangle::SeedBinding {
            seed_id: &lib_id,
            declares_main: hale_types::mangle::seed_declares_main(
                &stem_prog_refs,
            ),
        };
        seed_cache.insert(lib_key.clone(), seed_renames.clone());
        // GH #746: the lib's own files, for the alias-scoping pass.
        alias_scopes.record_files(
            &lib_key,
            parsed_files.iter().map(|f| f.canon.clone()).collect(),
        );
        if trace {
            eprintln!("[import]     build_seed_renames done (n={})", seed_renames.len());
            eprintln!("[import]     library {} is named {}", lib_key.display(), lib_id);
        }
        // Populate the per-build path-rename table.
        for (name, mangled) in &seed_renames {
            renames.push((vec![alias.clone(), name.clone()], mangled.clone()));
        }
        if trace {
            eprintln!(
                "[import]   resolved '{}' as {}: +{} files, seed_renames={}, \
                 visited now {}, renames now {}",
                imp.path,
                alias,
                parsed_files.len(),
                seed_renames.len(),
                visited.len(),
                renames.len(),
            );
        }
        // A4 (G34): lift the v1 strict barrier — follow each
        // imported lib's own `import "..." as ...;` directives,
        // recursing with the lib's own directory as the importer
        // dir so its relative paths resolve correctly. Cycles are
        // bounded by the canonical-path `visited` set. The renames
        // table is shared across the whole build so every transitive
        // alias::Name reference resolves at codegen time. Mangled
        // prefixes embed the importer's alias, so two parallel
        // import paths to the same lib produce different mangled
        // copies (per-importer namespacing, no collision).
        let lib_dir = match &target {
            ImportTarget::Directory(d) => d.clone(),
            ImportTarget::SingleFile(p) => p
                .parent()
                .map(|d| d.to_path_buf())
                .unwrap_or_else(|| importer_dir.to_path_buf()),
        };
        for pf in parsed_files.iter() {
            if pf.program.imports.is_empty() {
                continue;
            }
            resolve_imports(
                &pf.program.imports,
                &lib_dir,
                workspace_root,
                visited,
                claims,
                sources,
                file_bases,
                errors,
                merged_items,
                renames,
                seed_cache,
                effects,
                // GH #746: these imports are declared by THIS lib, so
                // its aliases are recorded against the lib, not
                // against whoever imported it.
                &lib_key,
                alias_scopes,
                src,
            )?;
        }
        // Parse each file again, through the load's effect-class table —
        // after the seeds it imports, so its classes are numbered after
        // theirs — mangle it with the shared map, and move its items
        // into the merged program; stash sources. The second parse of
        // tokens that already parsed cannot fail.
        for mut pf in parsed_files {
            // The one thing the load's table adds is its size: a class
            // past the effect mask's capacity is refused here, where the
            // seed's own table did not reach it.
            pf.program = match hale_syntax::parser::parse_in(pf.tokens, &pf.source, effects) {
                Ok(p) => p,
                Err(diags) => {
                    for d in diags {
                        errors.push(ImportDiag::Located {
                            file: pf.path.clone(),
                            base: pf.base,
                            diag: d,
                            source: pf.source.clone(),
                        });
                    }
                    sources.insert(pf.canon, pf.source);
                    continue;
                }
            };
            if trace {
                eprintln!("[import]     mangle start: {}", pf.path.display());
            }
            hale_types::mangle::mangle_with_renames_in_seed(
                &mut pf.program,
                &seed_renames,
                &seed_heads,
                // GH #774: the seed identity a claim group reference
                // this seed never declares is bound to, so an
                // importer's same-named group cannot capture it.
                seed_binding,
            );
            if trace {
                eprintln!("[import]     mangle done : {}", pf.path.display());
            }
            merged_items.extend(pf.program.items);
            sources.insert(pf.canon, pf.source);
            let _ = pf.path; // path was only needed for diagnostics above
        }
    }
    Ok(())
}

/// GH #746: scope an import alias to the seed that declared it, in the
/// per-build rename table as the language scopes it in source.
///
/// The table is keyed by the alias as written, so two seeds that bind
/// the same alias name to DIFFERENT libs collided in it: the last row
/// pushed won and BOTH seeds' `alias::Name` references resolved to one
/// lib. `hale check` passed — it resolves through the same table — and
/// the binary computed the wrong value, silently.
///
/// The rule the language states is per-seed, so the fix is per-seed:
/// each binder of a contested alias gets a head of its own (`u` ->
/// `u$0`, `u$1`, in canonical-path order so a build is reproducible),
/// its rows are registered under that head, and its own files'
/// references are re-headed to match. `$` cannot occur in an
/// identifier, so a scoped head can never collide with a user name;
/// diagnostics demangle back to the alias the author wrote.
///
/// The contested plain keys are REMOVED, not left beside the scoped
/// ones. A reference the rewrite fails to reach then fails loudly
/// ("unknown qualified name `u::f`") instead of quietly resolving to
/// whichever lib the table happened to hold — the failure mode this
/// whole pass exists to end.
///
/// Uncontested aliases — every build until one of these appears,
/// including the many seeds that all say `as dna` for the same lib —
/// keep the plain head and take no rewrite at all.
///
/// Out of scope: one seed whose own files bind the same alias to two
/// libs. That is a single namespace disagreeing with itself, not a
/// build-global leak; it keeps the historical last-writer-wins
/// reading (deterministic here, by canonical-path order).
pub fn scope_import_aliases(
    program: &mut Program,
    renames: &mut ImportRenames,
    file_bases: &[(u32, PathBuf, u32)],
    scopes: &AliasScopes,
    seed_cache: &BTreeMap<PathBuf, std::collections::HashMap<String, String>>,
) {
    let mut libs_of_alias: BTreeMap<&str, std::collections::BTreeSet<&Path>> =
        BTreeMap::new();
    for (_, alias, lib) in &scopes.bindings {
        libs_of_alias
            .entry(alias.as_str())
            .or_default()
            .insert(lib.as_path());
    }
    let contested: std::collections::BTreeSet<&str> = libs_of_alias
        .iter()
        .filter(|(_, libs)| libs.len() > 1)
        .map(|(alias, _)| *alias)
        .collect();
    if contested.is_empty() {
        return;
    }
    // seed -> (alias -> scoped head), and the rows each scoped head
    // needs (the lib's `name -> mangled` map, as `seed_cache` has it).
    let mut heads: BTreeMap<&Path, std::collections::HashMap<String, String>> =
        BTreeMap::new();
    let mut scoped_rows: Vec<(String, &Path)> = Vec::new();
    for alias in &contested {
        let mut binders: Vec<(&Path, &Path)> = scopes
            .bindings
            .iter()
            .filter(|(_, a, _)| a == alias)
            .map(|(seed, _, lib)| (seed.as_path(), lib.as_path()))
            .collect();
        binders.sort();
        binders.dedup();
        for (i, (seed, lib)) in binders.iter().enumerate() {
            let head = format!("{}${}", alias, i);
            heads
                .entry(seed)
                .or_default()
                .insert((*alias).to_string(), head.clone());
            scoped_rows.push((head, lib));
        }
    }
    renames.retain(|(key, _)| {
        !key.first()
            .is_some_and(|head| contested.contains(head.as_str()))
    });
    for (head, lib) in &scoped_rows {
        let Some(names) = seed_cache.get(*lib) else { continue };
        let mut sorted: Vec<(&String, &String)> = names.iter().collect();
        sorted.sort();
        for (name, mangled) in sorted {
            renames.push((vec![head.clone(), name.clone()], mangled.clone()));
        }
    }
    // Re-head each seed's own references. Every file is parsed at its
    // own virtual base, so a merged item's span says which file it
    // came from (`locate_span`'s rule).
    for (seed, map) in &heads {
        let Some(files) = scopes.files.get(*seed) else { continue };
        // `file_bases` holds the path as the caller spelled it — the
        // entry path canonicalizes, `parse_files` (the directory
        // paths) does not — so compare both spellings.
        let ranges: Vec<(u32, u32)> = file_bases
            .iter()
            .filter(|(_, path, _)| {
                files.contains(path)
                    || path
                        .canonicalize()
                        .is_ok_and(|canon| files.contains(&canon))
            })
            .map(|(base, _, len)| (*base, base.saturating_add(*len)))
            .collect();
        if ranges.is_empty() {
            continue;
        }
        for item in &mut program.items {
            let off = item.span().start.as_usize() as u32;
            if ranges.iter().any(|(lo, hi)| off >= *lo && off < *hi) {
                hale_types::mangle::rewrite_import_alias_heads(item, map);
            }
        }
    }
    if std::env::var("HALE_IMPORT_DEBUG").is_ok() {
        for alias in &contested {
            eprintln!(
                "[import] alias `{}` names {} libs; scoped per seed (GH #746)",
                alias,
                libs_of_alias.get(*alias).map(|l| l.len()).unwrap_or(0),
            );
        }
    }
}

/// GH #762: one qualified path written in a seed that does not
/// declare its head.
///
/// `base` is the virtual base the file was parsed at, so a caller
/// that renders against the file's own source (rather than through
/// `render_located`) can shift the span back into it.
pub struct UnscopedAliasUse {
    pub file: PathBuf,
    pub base: u32,
    pub diag: hale_syntax::Diag,
}

/// One qualified path as the lexer sees it: `head::next`, and the
/// span covering both segments.
pub struct QualifiedUse {
    pub head: String,
    pub text: String,
    pub span: hale_syntax::Span,
}

/// GH #762: every qualified path in `src`.
///
/// `offset` shifts the spans into the enclosing text and `limit`
/// clamps them: an f-string interpolation body is re-lexed from its
/// own text, whose byte offsets only approximate the file's when the
/// body carries escapes — the same clamp `FStringPart::Interp`
/// documents.
pub fn collect_qualified_uses(
    src: &str,
    offset: u32,
    limit: u32,
    out: &mut Vec<QualifiedUse>,
) {
    let Ok(tokens) = hale_syntax::lex(src) else { return };
    let place = |s: hale_syntax::Span| hale_syntax::Span {
        start: hale_syntax::Pos(s.start.0.saturating_add(offset).min(limit)),
        end: hale_syntax::Pos(s.end.0.saturating_add(offset).min(limit)),
    };
    for (i, t) in tokens.iter().enumerate() {
        // A path inside `f"{...}"` lives in the interpolation body,
        // which the lexer hands over as raw text; recurse so an
        // f-string is not a hole in the rule.
        if let hale_syntax::TokenKind::FStringLit(parts) = &t.kind {
            for p in parts {
                if let hale_syntax::lexer::FStringPart::Interp {
                    body,
                    start,
                    end,
                } = p
                {
                    collect_qualified_uses(
                        body,
                        offset.saturating_add(*start as u32),
                        limit.min(offset.saturating_add(*end as u32)),
                        out,
                    );
                }
            }
            continue;
        }
        let hale_syntax::TokenKind::Ident(head) = &t.kind else {
            continue;
        };
        if !matches!(
            tokens.get(i + 1).map(|n| &n.kind),
            Some(hale_syntax::TokenKind::ColonColon)
        ) {
            continue;
        }
        // Only the HEAD of a path: the middle of `a::b::c` and the
        // member of `x.y::z` are not alias positions.
        if i > 0
            && matches!(
                &tokens[i - 1].kind,
                hale_syntax::TokenKind::ColonColon
                    | hale_syntax::TokenKind::Dot
            )
        {
            continue;
        }
        let Some(seg) = tokens.get(i + 2) else { continue };
        let next = match &seg.kind {
            hale_syntax::TokenKind::Ident(s) => s.clone(),
            // `group g = { alias::* };` — a trailing glob member.
            hale_syntax::TokenKind::Star => "*".to_string(),
            // A member name may be a keyword (`expect_member_name`).
            other => other.keyword_lexeme().unwrap_or("_").to_string(),
        };
        out.push(QualifiedUse {
            head: head.clone(),
            text: format!("{}::{}", head, next),
            span: place(t.span.merge(seg.span)),
        });
    }
}

/// GH #762: refuse a qualified path whose head is an import alias the
/// seed it is written in never declared.
///
/// An alias binds in its declaring seed ONLY (spec `projects.md`,
/// "Scoped imports (A4)": there are no re-exports). The per-build
/// path-rename table is one table keyed by the alias as written, so a
/// seed that writes `u::f()` while importing nothing resolved through
/// ANOTHER seed's import row: a library silently called whatever lib
/// its app happened to spell `u`. Since GH #746 that fails loudly
/// when two seeds contest the alias, and stayed silent — the shape
/// this rule ends — whenever it was uncontested.
///
/// The scan is lexical on purpose. A qualified path is `IDENT :: ...`
/// in the token stream wherever it stands — a call, a const, a type,
/// a struct literal, a `bindings { }` topic, a group member, a claim
/// operand, an f-string interpolation — so the tokens see every
/// position at once, including the ones an AST walker would have to
/// be taught one at a time, and the span comes from the very token
/// the author typed. It runs before any rename, so heads are still
/// spelled the way the source spells them.
///
/// Exempt: `std::`, the bundled namespace no seed imports; the seed's
/// own aliases; and a head naming one of the seed's own declarations
/// (`Type::member` is not an alias at all). A head NO seed in the
/// build declares is left alone: nothing resolves through it, so it
/// is already refused downstream, and this rule is about the
/// reference that silently borrows another seed's import.
pub fn unscoped_alias_uses(
    program: &Program,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
    scopes: &AliasScopes,
    seed_cache: &BTreeMap<PathBuf, std::collections::HashMap<String, String>>,
) -> Vec<UnscopedAliasUse> {
    let mut out: Vec<UnscopedAliasUse> = Vec::new();
    // One seed can only reach its own aliases: there is no other
    // seed's import row to borrow.
    if scopes.bindings.is_empty() || scopes.files.len() < 2 {
        return out;
    }
    // alias -> the seeds that declare it (with the lib each names),
    // and each seed -> the aliases it declares.
    let mut declarers: BTreeMap<&str, Vec<(&Path, &Path)>> = BTreeMap::new();
    let mut aliases_of: BTreeMap<&Path, std::collections::BTreeSet<&str>> =
        BTreeMap::new();
    for (seed, alias, lib) in &scopes.bindings {
        declarers
            .entry(alias.as_str())
            .or_default()
            .push((seed.as_path(), lib.as_path()));
        aliases_of
            .entry(seed.as_path())
            .or_default()
            .insert(alias.as_str());
    }
    for v in declarers.values_mut() {
        v.sort();
        v.dedup();
    }
    // file -> the seed that owns it. `file_bases` carries the path as
    // the caller spelled it and `scopes.files` canonicalizes, so the
    // lookup tries both spellings (`scope_import_aliases`'s rule).
    let mut seed_of_file: BTreeMap<&Path, &Path> = BTreeMap::new();
    for (seed, files) in &scopes.files {
        for f in files {
            seed_of_file.insert(f.as_path(), seed.as_path());
        }
    }
    let seed_for = |p: &Path| -> Option<&Path> {
        if let Some(s) = seed_of_file.get(p) {
            return Some(s);
        }
        let canon = p.canonicalize().ok()?;
        seed_of_file.get(canon.as_path()).copied()
    };
    // What each seed DECLARES, as its own source spells it: an
    // imported seed's names are `seed_cache`'s keys (the rename
    // table's left-hand side), and the entry seed's are the items of
    // the merged program that no mangling touched.
    let mut own_names: BTreeMap<&Path, std::collections::BTreeSet<&str>> =
        BTreeMap::new();
    for (seed, names) in seed_cache {
        let e = own_names.entry(seed.as_path()).or_default();
        for n in names.keys() {
            e.insert(n.as_str());
        }
    }
    let ranges: Vec<(u32, u32, &Path)> = file_bases
        .iter()
        .filter_map(|(base, path, len)| {
            seed_for(path).map(|s| (*base, base.saturating_add(*len), s))
        })
        .collect();
    for item in &program.items {
        let off = item.span().start.as_usize() as u32;
        let Some((_, _, seed)) =
            ranges.iter().find(|(lo, hi, _)| off >= *lo && off < *hi)
        else {
            continue;
        };
        if let Some(name) = top_decl_ident(item) {
            own_names.entry(seed).or_default().insert(name);
        }
    }
    // What each seed could possibly borrow: an alias ANOTHER seed
    // declares and it does not, as it would have to be spelled
    // (`u::`). A file whose text holds none of them cannot hold a
    // borrowed reference, so it is never lexed — which is what keeps
    // this off the `check` latency budget for the ordinary build,
    // where nobody borrows anything.
    let mut borrowable: BTreeMap<&Path, Vec<String>> = BTreeMap::new();
    for seed in scopes.files.keys() {
        let seed = seed.as_path();
        let mine = aliases_of.get(seed);
        borrowable.insert(
            seed,
            declarers
                .iter()
                .filter(|(alias, binders)| {
                    !mine.is_some_and(|m| m.contains(**alias))
                        && binders.iter().any(|(s, _)| *s != seed)
                })
                .map(|(alias, _)| format!("{}::", alias))
                .collect(),
        );
    }
    for (base, path, _) in file_bases {
        let Some(seed) = seed_for(path) else { continue };
        let Some(src) = sources.get(path) else { continue };
        let Some(needles) = borrowable.get(seed) else { continue };
        if !needles.iter().any(|n| src.contains(n.as_str())) {
            continue;
        }
        let mine = aliases_of.get(seed);
        let declared = own_names.get(seed);
        let mut uses: Vec<QualifiedUse> = Vec::new();
        collect_qualified_uses(src, 0, u32::MAX, &mut uses);
        for u in uses {
            if u.head == "std" {
                continue;
            }
            if mine.is_some_and(|a| a.contains(u.head.as_str())) {
                continue;
            }
            if declared.is_some_and(|d| d.contains(u.head.as_str())) {
                continue;
            }
            let Some(binders) = declarers.get(u.head.as_str()) else {
                continue;
            };
            let Some((declaring, lib)) =
                binders.iter().find(|(s, _)| *s != seed)
            else {
                continue;
            };
            let here = path.parent().unwrap_or(Path::new("."));
            // A single-file lib is imported without its extension.
            let lib_path = if lib.extension().and_then(|s| s.to_str())
                == Some("hl")
            {
                lib.with_extension("")
            } else {
                lib.to_path_buf()
            };
            let msg = format!(
                "`{}`: `{}` is not an import of this seed; `{}` \
                 declares it. An import alias binds in the seed that \
                 declares it and is not re-exported (spec \
                 `projects.md`, \"Scoped imports (A4)\") — add \
                 `import \"{}\" as {};` to this seed to reach that \
                 library",
                u.text,
                u.head,
                display_relative(here, declaring),
                display_relative(here, &lib_path),
                u.head,
            );
            out.push(UnscopedAliasUse {
                file: path.clone(),
                base: *base,
                diag: hale_syntax::Diag::ty(u.span.shifted(*base), msg),
            });
        }
    }
    out
}
