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
//! Output binary defaults to the directory name (apps/ferryman/ →
//! apps/ferryman/ferryman) for dir targets, or the basename minus
//! .ap for file targets (hello-world.ap → hello-world).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aperio_syntax::ast::Program;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        usage();
        return ExitCode::from(2);
    }
    let cmd = &args[1];
    let target = PathBuf::from(&args[2]);

    match cmd.as_str() {
        "lex" => run_lex_file(&target),
        "parse" => run_parse_file(&target),
        "check" => run_check(&target),
        "run" => run_program(&target),
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

/// Parse a single entry file and recursively follow its
/// `import "..."` directives, merging all encountered top-level
/// declarations + imports into one logical Program. Paths
/// resolve RELATIVE to the importing file's directory, with the
/// `.ap` extension implicit (so `import "foo/bar"` from
/// `proj/main.ap` opens `proj/foo/bar.ap`).
///
/// Cycles are tolerated by short-circuiting on already-visited
/// canonical paths (the second sight contributes nothing new).
/// The merged Program's `imports` list is left empty since
/// resolution has already happened — no downstream pass needs
/// to re-walk it.
fn parse_with_imports(
    entry: &Path,
) -> Result<(Program, BTreeMap<PathBuf, String>), Vec<(PathBuf, aperio_syntax::Diag, String)>>
{
    let mut merged_items = Vec::new();
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut visited: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::new();
    let mut errors: Vec<(PathBuf, aperio_syntax::Diag, String)> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![entry.to_path_buf()];
    let mut entry_span_program: Option<Program> = None;

    while let Some(path) = stack.pop() {
        let canon = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => path.clone(),
        };
        if !visited.insert(canon.clone()) {
            continue;
        }
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("could not read {}: {}", path.display(), e);
                return Err(errors);
            }
        };
        let program = match aperio_syntax::parse_source(&source) {
            Ok(p) => p,
            Err(diags) => {
                for d in diags {
                    errors.push((path.clone(), d, source.clone()));
                }
                sources.insert(canon, source);
                continue;
            }
        };
        // Follow imports relative to THIS file's directory.
        // Imports beginning with `std/` are stdlib namespace
        // markers — the toolchain handles `time::sleep`,
        // `time::monotonic`, etc. as built-ins, so there's no
        // on-disk source to load. Silently skip those.
        let dir = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        for imp in &program.imports {
            if imp.path.starts_with("std/") || imp.path == "std" {
                continue;
            }
            let mut p = dir.clone();
            p.push(format!("{}.ap", imp.path));
            stack.push(p);
        }
        sources.insert(canon, source);
        if entry_span_program.is_none() {
            // Use the entry program's span / shape as the
            // skeleton; just collect items from imports into it.
            entry_span_program = Some(Program {
                items: Vec::new(),
                imports: Vec::new(),
                span: program.span,
            });
        }
        merged_items.extend(program.items);
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    let mut prog = entry_span_program.expect("at least one parse succeeded");
    prog.items = merged_items;
    Ok((prog, sources))
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
        let (program, sources) = match parse_with_imports(target) {
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
    // File targets follow `import "..."` directives starting from
    // the entry's directory; directory targets bundle every .ap
    // file in the directory as one seed (the per-dir package
    // model — apps/ferryman/{main,render,topology}.ap → one
    // binary). The directory shape is the user-facing answer to
    // the single-file-app-monolith friction; the file shape stays
    // for backwards compatibility and for one-off scripts.
    let (program, sources, output) = if target.is_file() {
        let (program, sources) = match parse_with_imports(target) {
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
        (program, sources, output)
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
        let merged = match merge_programs(programs.values()) {
            Some(m) => m,
            None => {
                eprintln!("no .ap files in {}", target.display());
                return ExitCode::from(1);
            }
        };
        // apps/ferryman/ → ferryman; output lands next to target.
        let bin_name = target
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main".to_string());
        let mut output = target.to_path_buf();
        output.push(&bin_name);
        let path_sources: BTreeMap<PathBuf, String> = sources
            .into_iter()
            .map(|(k, v)| (k, v))
            .collect();
        (merged, path_sources, output)
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
    match aperio_codegen::build_executable(&program, &output) {
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
