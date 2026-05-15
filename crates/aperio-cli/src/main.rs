//! `aperio` command-line entry point.
//!
//! v0 commands:
//!     aperio lex   <file.ap>          tokenize and print tokens
//!     aperio parse <file.ap>          parse and print the AST
//!     aperio check <file.ap | dir>    parse + typecheck (no run)
//!     aperio run   <file.ap | dir>    parse + typecheck + interpret
//!     aperio build <file.ap | dir>    parse + typecheck + emit native binary
//!
//! `run`, `check`, and `build` all accept a single .ap file or a
//! directory. The directory shape is the per-dir seed model — every
//! .ap file in the directory contributes to one bundle (one binary
//! when built); top-level decls in any file are visible to every
//! file in the same directory. File order: alphabetical by name.
//! Output binary defaults to the directory name (myapp/ →
//! myapp/myapp) for dir targets, or the basename minus .ap for
//! file targets (hello-world.ap → hello-world).

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aperio_syntax::ast::Program;

mod pkg;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        return ExitCode::from(2);
    }
    let cmd = &args[1];

    // `fetch` is the one subcommand that doesn't take a target
    // file/dir — it defaults to the current working directory and
    // optionally accepts a repo-root override.
    if cmd == "fetch" {
        let root = if args.len() >= 3 {
            PathBuf::from(&args[2])
        } else {
            env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };
        return match pkg::fetch(&root) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("aperio fetch: {}", e);
                ExitCode::from(1)
            }
        };
    }

    if args.len() < 3 {
        usage();
        return ExitCode::from(2);
    }
    let target = PathBuf::from(&args[2]);

    match cmd.as_str() {
        "lex" => run_lex_file(&target),
        "parse" => run_parse_file(&target),
        "check" => run_check(&target),
        "run" => {
            // Plumb the script's argv into the interpreter so
            // `std::env::args_count` / `std::env::arg(i)` work.
            // Convention mirrors compiled binaries: argv[0] is
            // the script path; argv[1..] are the trailing CLI
            // args. `aperio run script.ap foo bar` → script
            // sees ["script.ap", "foo", "bar"].
            let mut user_args: Vec<String> =
                Vec::with_capacity(args.len().saturating_sub(2));
            user_args.push(args[2].clone());
            for a in args.iter().skip(3) {
                user_args.push(a.clone());
            }
            aperio_runtime::set_user_args(user_args);
            run_program(&target)
        }
        "build" => run_build(&target),
        other => {
            eprintln!("unknown command: {}", other);
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!("aperio — Aperio language CLI");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("    aperio lex   <file.ap>          tokenize and print tokens");
    eprintln!("    aperio parse <file.ap>          parse and print the AST");
    eprintln!("    aperio check <file.ap | dir>    parse + typecheck");
    eprintln!("    aperio run   <file.ap | dir>    parse + typecheck + interpret");
    eprintln!("    aperio build <file.ap | dir>    parse + typecheck + emit native binary");
    eprintln!("    aperio fetch [repo-root]        fetch git deps from aperio.toml into vendor/");
}

fn run_lex_file(path: &Path) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    };
    match aperio_syntax::lex(&source) {
        Ok(tokens) => {
            for t in &tokens {
                let (line, col) = t.span.line_col(&source);
                println!("{:>4}:{:<3} {:?}", line, col, t.kind);
            }
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d.render(&source));
            }
            ExitCode::from(1)
        }
    }
}

fn run_parse_file(path: &Path) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    };
    match aperio_syntax::parse_source(&source) {
        Ok(prog) => {
            println!("{:#?}", prog);
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d.render(&source));
            }
            ExitCode::from(1)
        }
    }
}

fn collect_ap_files(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    if target.is_dir() {
        let mut out: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(target).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("ap") {
                out.push(p);
            }
        }
        out.sort();
        if out.is_empty() {
            return Err(format!("no .ap files in {}", target.display()));
        }
        return Ok(out);
    }
    Err(format!("not a file or directory: {}", target.display()))
}

/// Per-build path-rename table for cross-seed imports
/// (v1.x-IMPORT). Each entry maps a qualified-name segment vector
/// (e.g. `["foo", "Bar"]`) to the mangler-generated symbol name
/// (`__lib_foo_<stem>_Bar`). Passed to
/// `build_executable_with_imports` so codegen can resolve
/// `alias::Name` references in user code.
type ImportRenames = Vec<(Vec<String>, String)>;

/// Walk upward from `start` looking for a `Cargo.toml`; the first
/// directory containing one is treated as the workspace root.
/// Used for the workspace-root fallback in import resolution.
/// Returns `None` if no Cargo.toml is found before hitting the
/// filesystem root (standalone-shipped binaries hit this — they
/// can still use entry-relative imports, just not the
/// workspace-fallback path).
fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if cur.join("Cargo.toml").is_file() {
            return Some(cur);
        }
        cur = match cur.parent() {
            Some(p) => p.to_path_buf(),
            None => return None,
        };
    }
}

/// What an `import "path" as alias;` resolved to on disk.
enum ImportTarget {
    /// `<importer_dir>/<path>.ap` (single-file lib).
    SingleFile(PathBuf),
    /// `<importer_dir>/<path>/` or `<workspace_root>/<path>/`
    /// (directory bundle — one seed of multiple `.ap` files).
    Directory(PathBuf),
}

/// Try the three resolution strategies in order: entry-relative
/// single file, entry-relative directory, workspace-root directory.
/// Returns `None` if none of them hit.
fn resolve_import(
    importer_dir: &Path,
    workspace_root: Option<&Path>,
    import_path: &str,
) -> Option<ImportTarget> {
    let single = importer_dir.join(format!("{}.ap", import_path));
    if single.is_file() {
        return Some(ImportTarget::SingleFile(single));
    }
    let dir_local = importer_dir.join(import_path);
    if dir_local.is_dir() {
        return Some(ImportTarget::Directory(dir_local));
    }
    if let Some(root) = workspace_root {
        let dir_root = root.join(import_path);
        if dir_root.is_dir() {
            return Some(ImportTarget::Directory(dir_root));
        }
    }
    None
}

/// Collect every `.ap` file at an import target. SingleFile
/// resolves to one path; Directory enumerates the dir, sorting
/// alphabetically for deterministic merge order (mirrors the
/// per-dir seed convention from F.19).
fn collect_target_files(t: &ImportTarget) -> Result<Vec<PathBuf>, String> {
    match t {
        ImportTarget::SingleFile(p) => Ok(vec![p.clone()]),
        ImportTarget::Directory(d) => {
            let mut out = Vec::new();
            for entry in fs::read_dir(d).map_err(|e| e.to_string())? {
                let e = entry.map_err(|e| e.to_string())?;
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("ap") {
                    out.push(p);
                }
            }
            out.sort();
            if out.is_empty() {
                return Err(format!(
                    "imported directory {} contains no .ap files",
                    d.display()
                ));
            }
            Ok(out)
        }
    }
}

/// Resolve a flat list of import directives originating from one
/// importer directory: for each import, locate the target on disk
/// (entry-relative file or dir, workspace-root fallback dir),
/// parse every `.ap` file, mangle each sub-program with the
/// import alias + the file's stem, and merge the mangled items
/// into `merged_items`. Populates `renames` with
/// `(["<alias>", "<TopName>"], mangled_name)` entries so the
/// codegen can resolve `alias::Name` references downstream.
///
/// Imports inside the imported libs themselves are NOT followed
/// (strict barrier / no re-exports — see v1.x-IMPORT handoff).
fn resolve_imports(
    imports: &[aperio_syntax::ast::Import],
    importer_dir: &Path,
    workspace_root: Option<&Path>,
    visited: &mut std::collections::BTreeSet<PathBuf>,
    sources: &mut BTreeMap<PathBuf, String>,
    errors: &mut Vec<(PathBuf, aperio_syntax::Diag, String)>,
    merged_items: &mut Vec<aperio_syntax::ast::TopDecl>,
    renames: &mut ImportRenames,
) -> Result<(), ()> {
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
        let target = match resolve_import(importer_dir, workspace_root, &imp.path) {
            Some(t) => t,
            None => {
                eprintln!(
                    "could not resolve import \"{}\": tried {}/{}.ap, {}/{}/, \
                     and workspace-root/{}/",
                    imp.path,
                    importer_dir.display(),
                    imp.path,
                    importer_dir.display(),
                    imp.path,
                    imp.path,
                );
                return Err(());
            }
        };
        let files = match collect_target_files(&target) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("import \"{}\": {}", imp.path, e);
                return Err(());
            }
        };
        // Parse every file in the import target into a parallel
        // (file_path, stem, source, Program) list, recording the
        // canon path in `visited` so we don't double-parse.
        struct ParsedLibFile {
            path: PathBuf,
            canon: PathBuf,
            stem: String,
            source: String,
            program: aperio_syntax::ast::Program,
        }
        let mut parsed_files: Vec<ParsedLibFile> = Vec::new();
        for file in files {
            let canon = file.canonicalize().unwrap_or_else(|_| file.clone());
            if !visited.insert(canon.clone()) {
                continue;
            }
            let source = match fs::read_to_string(&file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "could not read imported file {} (from import \"{}\"): {}",
                        file.display(),
                        imp.path,
                        e
                    );
                    return Err(());
                }
            };
            let program = match aperio_syntax::parse_source(&source) {
                Ok(p) => p,
                Err(diags) => {
                    for d in diags {
                        errors.push((file.clone(), d, source.clone()));
                    }
                    sources.insert(canon, source);
                    continue;
                }
            };
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
                program,
            });
        }
        if parsed_files.is_empty() {
            continue;
        }
        // Build the unified rename map across every file in this
        // import target. Cross-file references inside the lib
        // (e.g. greet.ap uses a type declared in format.ap)
        // resolve through this shared map.
        let stem_prog_refs: Vec<(String, &aperio_syntax::ast::Program)> = parsed_files
            .iter()
            .map(|f| (f.stem.clone(), &f.program))
            .collect();
        let seed_renames =
            aperio_codegen::mangle::build_seed_renames(&stem_prog_refs, &alias);
        // Mangle each file's program with the shared map.
        for pf in parsed_files.iter_mut() {
            aperio_codegen::mangle::mangle_with_renames(&mut pf.program, &seed_renames);
        }
        // Populate the per-build path-rename table.
        for (name, mangled) in &seed_renames {
            renames.push((vec![alias.clone(), name.clone()], mangled.clone()));
        }
        // Move mangled items into the merged program; stash sources.
        for pf in parsed_files {
            merged_items.extend(pf.program.items);
            sources.insert(pf.canon, pf.source);
            let _ = pf.path; // path was only needed for diagnostics above
        }
    }
    Ok(())
}

/// Parse a single-file entry, follow its `import "..." as alias;`
/// directives, and produce the merged Program + per-build path-
/// rename table. Imports inside imported libs are NOT followed
/// (strict barrier). Cycles are tolerated by canonical-path
/// short-circuit.
fn parse_with_imports(
    entry: &Path,
) -> Result<
    (Program, ImportRenames, BTreeMap<PathBuf, String>),
    Vec<(PathBuf, aperio_syntax::Diag, String)>,
> {
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut errors: Vec<(PathBuf, aperio_syntax::Diag, String)> = Vec::new();
    let mut visited: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::new();

    let workspace_root = find_workspace_root(entry);
    let entry_dir = entry
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let entry_canon = entry.canonicalize().unwrap_or_else(|_| entry.to_path_buf());
    let entry_source = match fs::read_to_string(entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", entry.display(), e);
            return Err(errors);
        }
    };
    let entry_program = match aperio_syntax::parse_source(&entry_source) {
        Ok(p) => p,
        Err(diags) => {
            for d in diags {
                errors.push((entry.to_path_buf(), d, entry_source.clone()));
            }
            return Err(errors);
        }
    };
    visited.insert(entry_canon.clone());
    sources.insert(entry_canon, entry_source);

    let mut merged_items = entry_program.items;
    let mut renames: ImportRenames = Vec::new();

    if resolve_imports(
        &entry_program.imports,
        &entry_dir,
        workspace_root.as_deref(),
        &mut visited,
        &mut sources,
        &mut errors,
        &mut merged_items,
        &mut renames,
    )
    .is_err()
    {
        return Err(errors);
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    let merged = Program {
        imports: Vec::new(),
        items: merged_items,
        span: entry_program.span,
    };
    Ok((merged, renames, sources))
}


fn parse_files(
    files: &[PathBuf],
) -> Result<(BTreeMap<PathBuf, Program>, BTreeMap<PathBuf, String>), ExitCode> {
    let mut programs: BTreeMap<PathBuf, Program> = BTreeMap::new();
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut had_error = false;
    for f in files {
        let source = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {}", f.display(), e);
                had_error = true;
                continue;
            }
        };
        match aperio_syntax::parse_source(&source) {
            Ok(p) => {
                programs.insert(f.clone(), p);
                sources.insert(f.clone(), source);
            }
            Err(diags) => {
                eprintln!("{}:", f.display());
                for d in &diags {
                    eprintln!("  {}", d.render(&source));
                }
                had_error = true;
            }
        }
    }
    if had_error {
        return Err(ExitCode::from(1));
    }
    Ok((programs, sources))
}

fn run_check(target: &Path) -> ExitCode {
    let files = match collect_ap_files(target) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };
    let (programs, sources) = match parse_files(&files) {
        Ok(x) => x,
        Err(code) => return code,
    };

    let bundle_programs: BTreeMap<String, &Program> = programs
        .iter()
        .map(|(p, prog)| (p.display().to_string(), prog))
        .collect();
    let bundle = aperio_types::Bundle {
        programs: bundle_programs,
    };
    let diags = aperio_types::check_bundle(&bundle);
    if !diags.is_empty() {
        let any_source = sources.values().next().map(|s| s.as_str()).unwrap_or("");
        for d in &diags {
            eprintln!("{}", d.render(any_source));
        }
        return ExitCode::from(1);
    }
    eprintln!("ok: {} file(s) typechecked", files.len());
    ExitCode::SUCCESS
}

fn run_program(target: &Path) -> ExitCode {
    // Single-file targets follow imports starting from the entry
    // file's directory. Directory targets bundle every .lt under
    // them as today (multi-file projects without import wiring
    // — useful for ad-hoc test setups).
    if target.is_file() {
        // `aperio run` ignores the import_renames table — the
        // interpreter doesn't currently resolve qualified-name
        // paths (it already fails on `std::http::Request { ... }`
        // per the known limitation). Cross-seed imports through
        // `aperio run` will likewise fail on `alias::Name` paths;
        // use `aperio build` for programs with imports.
        let (program, _renames, sources) = match parse_with_imports(target) {
            Ok(x) => x,
            Err(errors) => {
                for (path, d, src) in &errors {
                    eprintln!("{}:", path.display());
                    eprintln!("  {}", d.render(src));
                }
                return ExitCode::from(1);
            }
        };
        let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
        bundle_programs.insert(target.display().to_string(), &program);
        let bundle = aperio_types::Bundle { programs: bundle_programs };
        let diags = aperio_types::check_bundle(&bundle);
        if !diags.is_empty() {
            let any_source = sources.values().next().map(|s| s.as_str()).unwrap_or("");
            for d in &diags {
                eprintln!("{}", d.render(any_source));
            }
            return ExitCode::from(1);
        }
        let prog_refs: Vec<&Program> = vec![&program];
        return match aperio_runtime::run_bundle(&prog_refs) {
            Ok(code) => ExitCode::from(code as u8),
            Err(e) => {
                eprintln!("runtime error: {}", e);
                ExitCode::from(1)
            }
        };
    }

    let files = match collect_ap_files(target) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };
    let (programs, sources) = match parse_files(&files) {
        Ok(x) => x,
        Err(code) => return code,
    };

    let bundle_programs: BTreeMap<String, &Program> = programs
        .iter()
        .map(|(p, prog)| (p.display().to_string(), prog))
        .collect();
    let bundle = aperio_types::Bundle {
        programs: bundle_programs,
    };
    let diags = aperio_types::check_bundle(&bundle);
    if !diags.is_empty() {
        let any_source = sources.values().next().map(|s| s.as_str()).unwrap_or("");
        for d in &diags {
            eprintln!("{}", d.render(any_source));
        }
        return ExitCode::from(1);
    }

    let prog_refs: Vec<&Program> = programs.values().collect();
    match aperio_runtime::run_bundle(&prog_refs) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("runtime error: {}", e);
            ExitCode::from(1)
        }
    }
}

fn run_build(target: &Path) -> ExitCode {
    // Phase 2i: warn if the CLI binary was built against an older
    // codegen+runtime source tree than what's on disk now. Silent
    // miscompile (stale CLI emitting old lowering against new
    // source) is the worst failure mode for a cold-context agent —
    // see `apps/log-router/FRICTION.md` 2026-05-10. The check is
    // best-effort: it skips when source files aren't locatable
    // (installed binary, moved workspace), or when the user
    // explicitly opts out via `APERIO_SKIP_STALE_CHECK=1`.
    check_stale_cli();

    // File targets follow `import "..."` directives starting from
    // the entry's directory; directory targets bundle every .ap
    // file in the directory as one seed (the per-dir package
    // model — myapp/{main,render,topology}.ap → one binary). The
    // directory shape is the user-facing answer to the
    // single-file-app-monolith friction; the file shape stays for
    // backwards compatibility and for one-off scripts.
    let (program, renames, sources, output) = if target.is_file() {
        let (program, renames, sources) = match parse_with_imports(target) {
            Ok(x) => x,
            Err(errors) => {
                for (path, d, src) in &errors {
                    eprintln!("{}:", path.display());
                    eprintln!("  {}", d.render(src));
                }
                return ExitCode::from(1);
            }
        };
        // hello-world.ap → hello-world
        let output = target.with_extension("");
        (program, renames, sources, output)
    } else if target.is_dir() {
        let files = match collect_ap_files(target) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::from(1);
            }
        };
        let (programs, sources) = match parse_files(&files) {
            Ok(x) => x,
            Err(code) => return code,
        };
        // Collect the union of all imports across the bundle's
        // files. Multiple files in one seed may share an import
        // alias (e.g. both reference `lib/foo`); the visited-set
        // inside resolve_imports dedupes by canonical file path,
        // so the same import resolved twice is a no-op.
        let mut union_imports: Vec<aperio_syntax::ast::Import> = Vec::new();
        for prog in programs.values() {
            for imp in &prog.imports {
                union_imports.push(imp.clone());
            }
        }
        let merged = match merge_programs(programs.values()) {
            Some(m) => m,
            None => {
                eprintln!("no .ap files in {}", target.display());
                return ExitCode::from(1);
            }
        };
        // Resolve the union of imports against the directory's
        // own dir as the importer dir + the workspace fallback.
        let workspace_root = find_workspace_root(target);
        let mut merged_items = merged.items;
        let mut renames: ImportRenames = Vec::new();
        let mut path_sources: BTreeMap<PathBuf, String> =
            sources.into_iter().collect();
        let mut visited: std::collections::BTreeSet<PathBuf> =
            std::collections::BTreeSet::new();
        for f in &files {
            if let Ok(c) = f.canonicalize() {
                visited.insert(c);
            } else {
                visited.insert(f.clone());
            }
        }
        let mut import_errors: Vec<(PathBuf, aperio_syntax::Diag, String)> = Vec::new();
        if resolve_imports(
            &union_imports,
            target,
            workspace_root.as_deref(),
            &mut visited,
            &mut path_sources,
            &mut import_errors,
            &mut merged_items,
            &mut renames,
        )
        .is_err()
        {
            for (path, d, src) in &import_errors {
                eprintln!("{}:", path.display());
                eprintln!("  {}", d.render(src));
            }
            return ExitCode::from(1);
        }
        if !import_errors.is_empty() {
            for (path, d, src) in &import_errors {
                eprintln!("{}:", path.display());
                eprintln!("  {}", d.render(src));
            }
            return ExitCode::from(1);
        }
        let with_imports = Program {
            imports: Vec::new(),
            items: merged_items,
            span: merged.span,
        };
        // myapp/ → myapp; output lands next to target.
        let bin_name = target
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main".to_string());
        let mut output = target.to_path_buf();
        output.push(&bin_name);
        (with_imports, renames, path_sources, output)
    } else {
        eprintln!("not a file or directory: {}", target.display());
        return ExitCode::from(1);
    };

    // Typecheck before lowering. Render diagnostics against the
    // entry-file's source — diagnostic spans currently point into
    // the merged item stream which doesn't have a single source
    // string; this is good enough for v0.
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(target.display().to_string(), &program);
    let bundle = aperio_types::Bundle {
        programs: bundle_programs,
    };
    let diags = aperio_types::check_bundle(&bundle);
    if !diags.is_empty() {
        let any_source = sources.values().next().map(|s| s.as_str()).unwrap_or("");
        for d in &diags {
            eprintln!("{}", d.render(any_source));
        }
        return ExitCode::from(1);
    }
    match aperio_codegen::build_executable_with_imports(&program, &output, &renames) {
        Ok(()) => {
            eprintln!("built: {}", output.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("codegen error: {}", e);
            ExitCode::from(1)
        }
    }
}

/// Merge a set of parsed Programs into a single Program by
/// concatenating their items. Used by directory-target builds:
/// every .ap file in the directory contributes its top-level
/// decls to one bundle, in alphabetical filename order (per
/// `collect_ap_files`'s sort). Returns `None` if the iterator
/// yielded zero programs. Mirrors the merge step inside
/// `parse_with_imports` but without the import-following
/// (directory targets see every file by enumeration; nothing to
/// follow).
fn merge_programs<'a, I>(programs: I) -> Option<Program>
where
    I: IntoIterator<Item = &'a Program>,
{
    let mut iter = programs.into_iter();
    let first = iter.next()?;
    let mut merged = Program {
        items: first.items.clone(),
        imports: Vec::new(),
        span: first.span,
    };
    for p in iter {
        merged.items.extend(p.items.clone());
    }
    Some(merged)
}

/// Phase 2i: warn when the CLI binary's bundled codegen + runtime
/// source snapshots are stale relative to the workspace's on-disk
/// source. Both the baked-in hash (set at build time by
/// `build.rs`) and the runtime-recomputed hash use the same
/// algorithm — DefaultHasher over each file's bytes, salted with
/// the relative path — so they match exactly when the on-disk
/// tree is the one the binary was built against.
///
/// Skipped silently when:
///  - `APERIO_SKIP_STALE_CHECK=1` is set,
///  - the baked codegen directory doesn't exist on this host
///    (installed binary, moved workspace),
///  - `build.rs` couldn't locate the workspace at build time
///    (the env vars are empty).
fn check_stale_cli() {
    if env::var_os("APERIO_SKIP_STALE_CHECK")
        .filter(|v| !v.is_empty() && v != "0")
        .is_some()
    {
        return;
    }
    let baked_hash = env!("APERIO_CODEGEN_SRC_HASH");
    let baked_dir = env!("APERIO_CODEGEN_DIR");
    if baked_hash.is_empty() || baked_dir.is_empty() {
        return;
    }
    let codegen_dir = Path::new(baked_dir);
    if !codegen_dir.exists() {
        return;
    }
    let current = compute_codegen_src_hash(codegen_dir);
    if current != baked_hash {
        eprintln!(
            "warning: aperio CLI binary was built against an older \
             codegen+runtime source tree."
        );
        eprintln!(
            "         {} has changed since the CLI was built; the \
             emitted binary may use stale lowering.",
            codegen_dir.display()
        );
        eprintln!(
            "         Rebuild with: cargo build -p aperio-cli"
        );
        eprintln!(
            "         (Set APERIO_SKIP_STALE_CHECK=1 to silence \
             this warning.)"
        );
    }
}

fn compute_codegen_src_hash(codegen_dir: &Path) -> String {
    let mut paths: Vec<PathBuf> = vec![
        codegen_dir.join("src").join("codegen.rs"),
        codegen_dir.join("runtime").join("lotus_arena.c"),
    ];
    let stdlib_dir = codegen_dir.join("runtime").join("stdlib");
    if let Ok(entries) = fs::read_dir(&stdlib_dir) {
        let mut stdlib_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    == Some("ap")
            })
            .map(|e| e.path())
            .collect();
        stdlib_files.sort();
        paths.extend(stdlib_files);
    }
    let mut hasher = DefaultHasher::new();
    for path in &paths {
        if let Ok(bytes) = fs::read(path) {
            hasher.write(path.to_string_lossy().as_bytes());
            hasher.write(&[0u8]);
            hasher.write(&bytes);
        }
    }
    format!("{:016x}", hasher.finish())
}
