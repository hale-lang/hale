//! `lotus` command-line entry point.
//!
//! v0 commands:
//!     lotus lex   <file.lt>           tokenize and print tokens
//!     lotus parse <file.lt>           parse and print the AST
//!     lotus check <file.lt | dir>     parse + typecheck (no run)
//!     lotus run   <file.lt | dir>     parse + typecheck + interpret
//!
//! `run` and `check` accept a single .lt file or a directory.
//! When given a directory, every .lt file in it is treated as
//! one bundle (multi-file project — e.g. trellis-pair).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lotus_syntax::ast::Program;

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
    eprintln!("lotus — lotus language CLI");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("    lotus lex   <file.lt>           tokenize and print tokens");
    eprintln!("    lotus parse <file.lt>           parse and print the AST");
    eprintln!("    lotus check <file.lt | dir>     parse + typecheck");
    eprintln!("    lotus run   <file.lt | dir>     parse + typecheck + interpret");
    eprintln!("    lotus build <file.lt>           parse + typecheck + emit native binary");
}

fn run_lex_file(path: &Path) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    };
    match lotus_syntax::lex(&source) {
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
    match lotus_syntax::parse_source(&source) {
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

fn collect_lt_files(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    if target.is_dir() {
        let mut out: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(target).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("lt") {
                out.push(p);
            }
        }
        out.sort();
        if out.is_empty() {
            return Err(format!("no .lt files in {}", target.display()));
        }
        return Ok(out);
    }
    Err(format!("not a file or directory: {}", target.display()))
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
        match lotus_syntax::parse_source(&source) {
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
    let files = match collect_lt_files(target) {
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
    let bundle = lotus_types::Bundle {
        programs: bundle_programs,
    };
    let diags = lotus_types::check_bundle(&bundle);
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
    let files = match collect_lt_files(target) {
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
    let bundle = lotus_types::Bundle {
        programs: bundle_programs,
    };
    let diags = lotus_types::check_bundle(&bundle);
    if !diags.is_empty() {
        let any_source = sources.values().next().map(|s| s.as_str()).unwrap_or("");
        for d in &diags {
            eprintln!("{}", d.render(any_source));
        }
        return ExitCode::from(1);
    }

    let prog_refs: Vec<&Program> = programs.values().collect();
    match lotus_runtime::run_bundle(&prog_refs) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("runtime error: {}", e);
            ExitCode::from(1)
        }
    }
}

fn run_build(target: &Path) -> ExitCode {
    if !target.is_file() {
        eprintln!("`lotus build` accepts a single .lt file in milestone 0");
        return ExitCode::from(1);
    }
    let source = match fs::read_to_string(target) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", target.display(), e);
            return ExitCode::from(1);
        }
    };
    let program = match lotus_syntax::parse_source(&source) {
        Ok(p) => p,
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d.render(&source));
            }
            return ExitCode::from(1);
        }
    };
    // Typecheck before lowering.
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(target.display().to_string(), &program);
    let bundle = lotus_types::Bundle {
        programs: bundle_programs,
    };
    let diags = lotus_types::check_bundle(&bundle);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", d.render(&source));
        }
        return ExitCode::from(1);
    }
    // Output binary alongside the source: hello-world.lt → hello-world.
    let output = target.with_extension("");
    match lotus_codegen::build_executable(&program, &output) {
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
