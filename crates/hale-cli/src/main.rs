//! `hale` command-line entry point.
//!
//! v0 commands:
//!     hale lex   <file.hl>          tokenize and print tokens
//!     hale parse <file.hl>          parse and print the AST
//!     hale check <file.hl | dir>    parse + typecheck (no run)
//!     hale run   <file.hl | dir>    parse + typecheck + interpret
//!     hale build <file.hl | dir>    parse + typecheck + emit native binary
//!
//! `run`, `check`, and `build` all accept a single .hl file or a
//! directory. The directory shape is the per-dir seed model — every
//! .hl file in the directory contributes to one bundle (one binary
//! when built); top-level decls in any file are visible to every
//! file in the same directory. File order: alphabetical by name.
//! Output binary defaults to the directory name (myapp/ →
//! myapp/myapp) for dir targets, or the basename minus .hl for
//! file targets (hello-world.hl → hello-world).

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hale_syntax::ast::Program;

use hale_lsp as lsp;
mod fleet;
mod dna;
mod iris;
mod mcp;
mod pkg;
mod replay;
mod sign;
mod topology_graph;
mod fleet_model;
mod topology_law;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        return ExitCode::from(2);
    }
    let cmd = &args[1];

    if cmd == "--version" || cmd == "-V" || cmd == "version" {
        // The first line is the version and nothing else: the DNA
        // fixtures, the body-provisioning script and the benchmark
        // harness read `$2` of it.
        println!("hale {}", env!("CARGO_PKG_VERSION"));
        // GH #726: the DNA source a binary carries is not implied by
        // its version — two builds of one version can embed
        // different `dna/` source, and a fixture that edited the
        // working tree without rebuilding measures the old one. The
        // second line names what this binary embeds
        // (`hale dna --embedded-digest` prints all 64 hex digits).
        println!("embedded dna: {}", hale_dna::embedded_short());
        return ExitCode::SUCCESS;
    }
    if cmd == "--help" || cmd == "-h" || cmd == "help" {
        usage();
        return ExitCode::SUCCESS;
    }

    // Which targets exist, and what the compiler can actually do with
    // each. Naming a target and building it are different capabilities,
    // so the listing states the tier rather than implying parity.
    // GH #527 B3: the embedded observer. `hale iris [port] [artifact]`,
    // `hale iris inspect <artifact> [url]`, `--where`, `--build-only`.
    if cmd == "iris" {
        return iris::run(&args[2..]);
    }
    // GH #528: `hale dna init|new|upgrade …` — the DNA attached to an
    // application as ordinary Hale source.
    if cmd == "dna" {
        return dna::run(&args[2..]);
    }
    // GH #566 F5: `hale node <name>` — the agent that expresses a fleet
    // plan's instances on one machine, from the record.
    if cmd == "node" {
        return dna::node(&args[2..]);
    }
    if cmd == "--list-targets" || cmd == "targets" {
        let host = hale_codegen::target::TargetSpec::host();
        for t in hale_codegen::target::TargetSpec::known() {
            let marker = if t.triple == host.triple {
                "  (host)"
            } else {
                ""
            };
            println!("{}{}\n", t.describe(), marker);
        }
        return ExitCode::SUCCESS;
    }

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
                eprintln!("hale fetch: {}", e);
                ExitCode::from(1)
            }
        };
    }

    // `init` bootstraps a project: a `hale.toml` skeleton, a
    // hello-world `main.hl` seed, a first native test, and a
    // `.gitignore` for the build artifact + vendor/. Defaults to
    // the current directory; strictly non-destructive (every file
    // that already exists is left untouched and reported).
    if cmd == "init" {
        let root = if args.len() >= 3 {
            PathBuf::from(&args[2])
        } else {
            env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };
        return run_init(&root);
    }

    // `test` is a discovery-driven subcommand: like `fetch` it
    // defaults its target to the current working directory, so
    // `hale test` (no path) is valid. An explicit file/dir and any
    // `-run` / `--json` flags are parsed inside `run_test`. Handled
    // here, before the `args.len() < 3` guard, so a bare `hale test`
    // doesn't fall into the usage-error path.
    if cmd == "test" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_test(&rest);
    }

    // `replay` takes a recording and a program (its own arg shape,
    // so it parses `rest` itself like `test` does). GH #296.
    if cmd == "replay" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_replay(&rest);
    }

    // `lsp` speaks stdio and takes no target — the seed to check is
    // derived per-document from the client's textDocument URIs.
    if cmd == "lsp" {
        return lsp::run_lsp();
    }

    // `mcp` speaks Model Context Protocol over stdio — the agent
    // surface for hosts without a shell. Tools self-exec this
    // binary (version-locked by construction) or call hale-lsp
    // directly.
    if cmd == "mcp" {
        return mcp::run_mcp();
    }

    // `fmt` is discovery-driven like `test`: a bare `hale fmt`
    // formats the current directory tree in place.
    if cmd == "fmt" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_fmt(&rest);
    }

    // `doc` renders a seed's API reference from `///` doc comments.
    if cmd == "doc" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_doc(&rest);
    }

    // GH #408: `hale fleet check|dump <plan.json>` — compose
    // topology artifacts into one fleet model. A CLIENT of the
    // artifact, never a second source analyzer: it reads exactly what
    // a third party would read.
    if cmd == "fleet" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_fleet(&rest);
    }

    // GH #476 Track A: `hale topology graph <artifact>` — deterministic
    // visuals from a committed topology artifact. Like `fleet`, an
    // artifact CLIENT: reads exactly what a third party reads, never
    // Hale source. Experimental surface pre-1.0.
    if cmd == "topology" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return topology_graph::run_topology(&rest);
    }

    // GH #476 Change 2: `hale model dump <target>` — derive and print
    // the canonical ApplicationModel (internal, non-stable format).
    // The DEMAND surface: this command is what builds the model;
    // plain `hale check` provably never does (HALE_MODEL_TRACE=1
    // shows the derivation line here and stays silent there).
    // Implemented as a shim into the check pipeline so bundle
    // loading, imports, and the ill-typed refusal are identical.
    if cmd == "model" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        // GH #527 B4: `hale model diff <a> <b> [--json|--text]` —
        // the semantic difference between two topology artifacts.
        if rest.first().map(String::as_str) == Some("diff") {
            return run_model_diff(&rest[1..]);
        }
        if rest.first().map(String::as_str) != Some("dump") {
            eprintln!("usage: hale model dump <file.hl | dir>");
            eprintln!("       hale model diff <a.topology> <b.topology> [--json|--text]");
            eprintln!();
            eprintln!("`dump` derives the canonical ApplicationModel (GH #476) and prints an internal,");
            eprintln!("non-stable dump (experimental, pre-1.0).");
            eprintln!("`diff` compares two --dump-topology artifacts: declarations (added / removed /");
            eprintln!("renamed / moved / split / joined / ambiguous), per-locus contract deltas, effect");
            eprintln!("and certificate deltas, law and adequacy deltas, and a source-only vs model-shape");
            eprintln!("classification. JSON (versioned, digest-bearing) by default; --text for a review view.");
            return ExitCode::from(2);
        }
        // The check pipeline's dump section reads PROCESS argv (it
        // is a top-level-command scope), so the flag cannot ride the
        // rest-args; the shim marks the demand via env instead.
        std::env::set_var("HALE_DUMP_MODEL", "1");
        let shim: Vec<String> = rest[1..].to_vec();
        return run_check_cli(&shim, false);
    }

    // `bench` is discovery-driven like `test`: *_bench.hl files,
    // bench_* fns, self-calibrating harness.
    if cmd == "bench" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_bench(&rest);
    }

    // `check` / `verify` take flags, so they get real argument
    // parsing rather than "the target is argv[2] and everything else
    // is scenery". Devex review of v0.15.0: an unknown flag, a
    // stray positional, and `--help` were all silently ignored while
    // the command still reported SUCCESS — the same fail-open that
    // made the topology gates untrustworthy.
    if cmd == "check" || cmd == "verify" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_check_cli(&rest, cmd == "verify");
    }
    // `hale inputs <seed>`: every file a build of the seed reads, one
    // canonical path per line — the seed's own `.hl` files and every
    // `.hl` of every directory they import, transitively, exactly as
    // the compiler resolves them (entry-relative, then workspace root).
    // For anything that has to know what a build depends on without
    // guessing from git: DNA's apply asks this before it lets a
    // candidate express, because an untracked, ignored or oddly named
    // source file beside the reviewed ones is compiled all the same.
    if cmd == "inputs" {
        if args.len() < 3 {
            eprintln!("usage: hale inputs <seed-dir | file.hl>");
            return ExitCode::from(2);
        }
        return match seed_inputs(Path::new(&args[2])) {
            Ok(files) => {
                for f in files {
                    println!("{}", f.display());
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("hale inputs: {e}");
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
        "run" => {
            // `hale run` compiles the program to a temporary binary
            // (the same codegen backend as `hale build`) and executes
            // it — there is no separate interpreter. The program's
            // trailing argv is forwarded to the exec'd process, so
            // `hale run script.hl foo bar` makes the program's
            // `std::env::arg(1..)` see ["foo", "bar"] exactly as a
            // built binary run directly would.
            let user_args: Vec<String> = args.iter().skip(3).cloned().collect();
            // GH #527 B3: `hale run --observe <target>` — the program
            // publishes its observation segment (LOTUS_OBS=1, inherited
            // by the child) and an iris session runs beside it for the
            // program's lifetime. The flag is consumed here; nothing
            // reaches the program's argv.
            if user_args.first().map(String::as_str) == Some("--observe") || target.to_str() == Some("--observe") {
                let (target, user_args) = if target.to_str() == Some("--observe") {
                    (PathBuf::from(user_args.first().cloned().unwrap_or_default()), user_args[1..].to_vec())
                } else {
                    (target.clone(), user_args[1..].to_vec())
                };
                std::env::set_var("LOTUS_OBS", "1");
                let session = iris::spawn_session();
                let code = run_program(&target, &user_args);
                if let Some(mut s) = session {
                    let _ = s.kill();
                    let _ = s.wait();
                }
                return code;
            }
            run_program(&target, &user_args)
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
    eprintln!("hale — Hale language CLI");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("    hale lex   <file.hl>          tokenize and print tokens");
    eprintln!("    hale parse <file.hl>          parse and print the AST");
    eprintln!("    hale init  [dir]              bootstrap a project (hale.toml, main.hl, first test)");
    eprintln!("    hale inputs <seed>            every file a build of the seed reads (imports followed)");
    eprintln!("    hale check <file.hl | dir>    parse + typecheck");
    eprintln!("        [--dump-topology[=<path>]] [--check-topology[-shape] <path>]");
    eprintln!("        [--dump-effects-manifest] [--json] [--workspace]");
    eprintln!("        (`hale check --help` for all)");
    eprintln!("    hale verify <file.hl | dir>   check + FAIL on any advisory (discipline gate)");
    eprintln!("    hale topology graph <artifact> render a --dump-topology artifact (svg|mermaid|dot; experimental)");
    eprintln!("    hale model dump <file.hl | dir> derive + print the canonical ApplicationModel (internal; experimental)");
    eprintln!("    hale model diff <a> <b>       semantic diff of two --dump-topology artifacts (--json|--text)");
    eprintln!("    hale run   <file.hl | dir>    compile + run as a native binary");
    eprintln!("    hale build <file.hl | dir>    parse + typecheck + emit native binary");
    eprintln!("    hale replay <rec> <file.hl>   re-run a LOTUS_OBS_RECORD recording");
    eprintln!("    hale iris  [port] [artifact]  the embedded observer: attach to LOTUS_OBS=1 processes, serve :8787");
    eprintln!("    hale iris inspect <artifact>  artifact-side inspector (drift / declared-but-silent / law)");
    eprintln!("    hale run --observe <target>   run with LOTUS_OBS=1 and an iris session beside it");
    eprintln!("    hale dna init|new|upgrade     attach the DNA to an application (vendor/dna, dna/, seeded Journal)");
    eprintln!("        [--diff: report first divergence, fail on any; per-category coverage on a match]");
    eprintln!("        [--json (with --diff): that verdict + coverage, machine-readable]");
    eprintln!("        [--at <n> | --at <consumer-id>:<ordinal>: SIGSTOP at that consume]");
    eprintln!("        [--allow-live-effects] [--allow-unverified-model] [--allow-truncated]");
    eprintln!("        [--feed: inject the recorded ingress tape into (possibly changed) code]");
    eprintln!("        [--allow-unmatched-feed: accept a partially-fed tape]");
    eprintln!("    hale test  [file | dir]       compile + run *_test.hl (default: cwd)");
    eprintln!("        [-run <substr>] [--json]");
    eprintln!("    hale bench [file | dir]       run *_bench.hl bench_* fns (default: cwd)");
    eprintln!("        [-run <substr>] [--json]");
    eprintln!("    hale fmt   [file | dir] ...   canonical formatter (default: cwd)");
    eprintln!("        [--check] [--diff] [--stdin]");
    eprintln!("    hale doc   [file | dir]       render the seed's API reference (/// doc comments)");
    eprintln!("        [--json] [-o <path>] [--stdlib: the std:: surface]");
    eprintln!("    hale fetch [repo-root]        fetch git deps from hale.toml into vendor/");
    eprintln!("    hale lsp                      stdio Language Server (diagnostics)");
    eprintln!("    hale mcp                      stdio Model Context Protocol server (agent tools)");
    eprintln!();
    eprintln!("    hale --version               print the version, and the embedded DNA source's digest");
    eprintln!("    hale --help                  print this help");
}


/// `hale fmt` — format `.hl` files in place (spec/testing.md:
/// Go-style, zero config). Targets may be files or directories
/// (recursed; `vendor/` and dot-dirs skipped); no target = cwd.
///
///   --check   don't write; exit 1 if any file would change,
///             listing them (CI gate)
///   --diff    don't write; print a unified-ish before/after for
///             files that would change
///   --stdin   read source on stdin, write formatted to stdout
///             (editor integration)
///
/// A file that doesn't lex is reported and skipped (exit 1): the
/// formatter never touches a file it can't fully tokenize. The
/// internal re-lex equivalence gate means a formatter bug can't
/// change what the compiler sees — on gate failure the file is
/// reported and left untouched.
fn run_fmt(rest: &[String]) -> ExitCode {
    let mut check = false;
    let mut diff = false;
    let mut stdin_mode = false;
    let mut targets: Vec<PathBuf> = Vec::new();
    for a in rest {
        match a.as_str() {
            "--check" => check = true,
            "--diff" => diff = true,
            "--stdin" => stdin_mode = true,
            other if other.starts_with('-') => {
                eprintln!("hale fmt: unknown flag {}", other);
                return ExitCode::from(2);
            }
            other => targets.push(PathBuf::from(other)),
        }
    }

    if stdin_mode {
        use std::io::Read;
        let mut src = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut src) {
            eprintln!("hale fmt: reading stdin: {}", e);
            return ExitCode::from(1);
        }
        return match hale_syntax::fmt::format_source(&src) {
            Ok(out) => {
                print!("{}", out);
                ExitCode::SUCCESS
            }
            Err(hale_syntax::fmt::FmtError::Parse(diags)) => {
                for d in &diags {
                    eprintln!("hale fmt: {:?}", d);
                }
                ExitCode::from(1)
            }
            Err(hale_syntax::fmt::FmtError::Changed(_)) => {
                eprintln!(
                    "hale fmt: internal error: formatting would alter \
                     the token stream (bug — input left untouched)"
                );
                ExitCode::from(1)
            }
        };
    }

    if targets.is_empty() {
        targets.push(
            env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for t in &targets {
        collect_hl_files(t, &mut files);
    }
    files.sort();
    files.dedup();

    let mut changed: Vec<PathBuf> = Vec::new();
    let mut failed = false;
    for f in &files {
        let src = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hale fmt: could not read {}: {}", f.display(), e);
                failed = true;
                continue;
            }
        };
        let out = match hale_syntax::fmt::format_source(&src) {
            Ok(o) => o,
            Err(hale_syntax::fmt::FmtError::Parse(_)) => {
                eprintln!(
                    "hale fmt: {}: does not lex — skipped",
                    f.display()
                );
                failed = true;
                continue;
            }
            Err(hale_syntax::fmt::FmtError::Changed(_)) => {
                eprintln!(
                    "hale fmt: {}: internal equivalence-gate failure \
                     (bug — file left untouched)",
                    f.display()
                );
                failed = true;
                continue;
            }
        };
        if out == src {
            continue;
        }
        changed.push(f.clone());
        if diff {
            print_fmt_diff(f, &src, &out);
        } else if !check {
            if let Err(e) = fs::write(f, &out) {
                eprintln!(
                    "hale fmt: could not write {}: {}",
                    f.display(),
                    e
                );
                failed = true;
            }
        }
    }

    if check {
        for f in &changed {
            println!("{}", f.display());
        }
        if !changed.is_empty() {
            return ExitCode::from(1);
        }
    } else if !diff {
        for f in &changed {
            println!("formatted {}", f.display());
        }
    }
    if failed {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// Recursively collect `.hl` files. Directories named `vendor` or
/// starting with `.` are skipped (vendored pins are frozen — see
/// pond's promotion banners — and formatting them would churn
/// upstream diffs).
fn collect_hl_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|e| e == "hl") {
            out.push(path.to_path_buf());
        }
        return;
    }
    if !path.is_dir() {
        eprintln!("hale fmt: {} not found", path.display());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "vendor" || name.starts_with('.') {
                continue;
            }
            collect_hl_files(&p, out);
        } else if name.ends_with(".hl") {
            out.push(p);
        }
    }
}

/// Minimal line-based change listing (not a real unified diff —
/// enough to see what fmt would do without writing).
fn print_fmt_diff(path: &Path, before: &str, after: &str) {
    println!("--- {}", path.display());
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    let n = b.len().max(a.len());
    for i in 0..n {
        let bl = b.get(i).copied().unwrap_or("");
        let al = a.get(i).copied().unwrap_or("");
        if bl != al {
            println!("{}: - {}", i + 1, bl);
            println!("{}: + {}", i + 1, al);
        }
    }
}


/// `hale doc [file | dir] [--json] [-o <path>]` — the API-reference
/// generator (spec/testing.md). Zero config: the convention is
/// `///` doc comments on the lines directly above a declaration
/// (decorator lines like `@hot` may sit between); the generator
/// renders every public top-level declaration — fns, loci (with
/// their params and documented methods), types, topics, interfaces,
/// consts — as Markdown (default, stdout or `-o`) or JSON records.
/// Names starting with `__` are internal and skipped. A file that
/// doesn't parse is reported and skipped (exit 1).
fn run_doc(rest: &[String]) -> ExitCode {
    let mut json = false;
    let mut stdlib = false;
    let mut out_path: Option<PathBuf> = None;
    let mut target: Option<PathBuf> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--stdlib" => {
                stdlib = true;
                i += 1;
            }
            "-o" | "--out" => match rest.get(i + 1) {
                Some(v) => {
                    out_path = Some(PathBuf::from(v));
                    i += 2;
                }
                None => {
                    eprintln!("hale doc: {} requires a path", rest[i]);
                    return ExitCode::from(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("hale doc: unknown flag {}", other);
                return ExitCode::from(2);
            }
            other => {
                target = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    if stdlib {
        return run_doc_stdlib(json, out_path);
    }
    let target = target
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // A seed = one directory (F.19): file target docs that file,
    // dir target docs every .hl directly in it.
    let mut files: Vec<PathBuf> = Vec::new();
    if target.is_file() {
        files.push(target.clone());
    } else if target.is_dir() {
        if let Ok(rd) = fs::read_dir(&target) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "hl") {
                    files.push(p);
                }
            }
        }
        files.sort();
    } else {
        eprintln!("hale doc: {} not found", target.display());
        return ExitCode::from(1);
    }

    let mut failed = false;
    let mut md = String::new();
    let seed_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| target.display().to_string());
    md.push_str(&format!("# API — {}\n", seed_name));
    let mut json_items: Vec<serde_json::Value> = Vec::new();

    for f in &files {
        let src = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hale doc: could not read {}: {}", f.display(), e);
                failed = true;
                continue;
            }
        };
        let program = match hale_syntax::parse_source(&src) {
            Ok(p) => p,
            Err(_) => {
                eprintln!(
                    "hale doc: {}: does not parse — skipped",
                    f.display()
                );
                failed = true;
                continue;
            }
        };
        let entries = doc_entries_for(&src, &program);
        if entries.is_empty() {
            continue;
        }
        md.push_str(&format!("\n## {}\n", f.display()));
        for e in &entries {
            md.push_str(&format!("\n### {}\n\n```hale\n{}\n```\n", e.name, e.signature));
            if !e.doc.is_empty() {
                md.push_str(&format!("\n{}\n", e.doc));
            }
            for m in &e.members {
                md.push_str(&format!(
                    "\n- `{}`{}\n",
                    m.signature,
                    if m.doc.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", m.doc.replace('\n', " "))
                    }
                ));
            }
            if json {
                json_items.push(serde_json::json!({
                    "file": f.display().to_string(),
                    "kind": e.kind,
                    "name": e.name,
                    "signature": e.signature,
                    "doc": e.doc,
                    "members": e.members.iter().map(|m| serde_json::json!({
                        "signature": m.signature, "doc": m.doc
                    })).collect::<Vec<_>>(),
                }));
            }
        }
    }

    let rendered = if json {
        serde_json::to_string_pretty(&json_items)
            .unwrap_or_else(|_| "[]".into())
            + "\n"
    } else {
        md
    };
    match out_path {
        Some(p) => {
            if let Err(e) = fs::write(&p, rendered) {
                eprintln!("hale doc: could not write {}: {}", p.display(), e);
                return ExitCode::from(1);
            }
            eprintln!("wrote {}", p.display());
        }
        None => print!("{}", rendered),
    }
    if failed {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}


/// `hale doc --stdlib` — the `std::` API reference. Merges three
/// sources of truth: the rename table (public `std::` path per
/// mangled decl), the bundled stdlib source (decl shapes + `///`
/// doc comments — public method surface of each locus), and the
/// typecheck signature table (the C-primitive-backed free fns that
/// have no .hl decl). Grouped by namespace; Markdown or JSON like
/// the seed mode.
fn run_doc_stdlib(json: bool, out_path: Option<PathBuf>) -> ExitCode {
    use hale_syntax::ast::{LocusMember, TopDecl};
    let src = hale_codegen::stdlib_doc_source();
    let program = match hale_syntax::parse_source(src) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("hale doc --stdlib: bundled stdlib does not parse (bug)");
            return ExitCode::from(1);
        }
    };
    // mangled name -> public path segments
    let mut public: BTreeMap<&str, String> = BTreeMap::new();
    for (segs, mangled) in hale_codegen::stdlib_path_renames() {
        public.insert(*mangled, segs.join("::"));
    }
    // Signatures written against internal names (a locus param
    // typed `__StdMetricsMap`) display their public paths.
    let demangle = |sig: &str| -> String {
        let mut out = sig.to_string();
        for (mangled, pubpath) in &public {
            if out.contains(mangled) {
                out = out.replace(mangled, pubpath);
            }
        }
        out
    };

    // namespace ("std::metrics") -> entries
    let mut groups: BTreeMap<String, Vec<DocEntry>> = BTreeMap::new();
    let ns_of = |path: &str| -> String {
        match path.rfind("::") {
            Some(i) => path[..i].to_string(),
            None => path.to_string(),
        }
    };

    for item in &program.items {
        match item {
            TopDecl::Fn(fd) => {
                let Some(path) = public.get(fd.name.name.as_str()) else {
                    continue;
                };
                // Leaf name first (demangle would otherwise expand
                // the fn's own mangled name to its full path), then
                // demangle the param/return types.
                let leaf = path.rsplit("::").next().unwrap_or(path);
                let sig = demangle(
                    &doc_fn_signature(fd).replacen(&fd.name.name, leaf, 1),
                );
                groups.entry(ns_of(path)).or_default().push(DocEntry {
                    kind: "fn",
                    name: path.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        fd.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Type(t) => {
                let Some(path) = public.get(t.name.name.as_str()) else {
                    continue;
                };
                use hale_syntax::ast::TypeDeclBody;
                let leaf = path.rsplit("::").next().unwrap_or(path);
                let sig = match &t.body {
                    TypeDeclBody::Struct(fields) => {
                        let fs = fields
                            .iter()
                            .filter(|f| !f.name.name.starts_with("__"))
                            .map(|f| {
                                format!(
                                    "{}: {};",
                                    f.name.name,
                                    demangle(&lsp::type_expr_str(&f.ty))
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        format!("type {} {{ {} }}", leaf, fs)
                    }
                    TypeDeclBody::Enum(vs) => {
                        let names = vs
                            .iter()
                            .map(|v| v.name.name.clone())
                            .collect::<Vec<_>>()
                            .join(" | ");
                        format!("type {} = enum {{ {} }}", leaf, names)
                    }
                    TypeDeclBody::Alias(inner) => format!(
                        "type {} = {}",
                        leaf,
                        demangle(&lsp::type_expr_str(inner))
                    ),
                };
                groups.entry(ns_of(path)).or_default().push(DocEntry {
                    kind: "type",
                    name: path.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        t.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Locus(l) => {
                let Some(path) = public.get(l.name.name.as_str()) else {
                    continue;
                };
                let mut members = Vec::new();
                let mut params_sig = String::new();
                for m in &l.members {
                    match m {
                        LocusMember::Params(pb) => {
                            // Skip __-named params AND params whose
                            // type demangles to nothing public
                            // (internal owned-storage wiring like
                            // Router's entry list).
                            let ps = pb
                                .params
                                .iter()
                                .filter(|p| !p.name.name.starts_with("__"))
                                .filter_map(|p| match &p.ty {
                                    Some(t) => {
                                        let ty =
                                            demangle(&lsp::type_expr_str(t));
                                        if ty.contains("__") {
                                            None
                                        } else {
                                            Some(format!(
                                                "{}: {}",
                                                p.name.name, ty
                                            ))
                                        }
                                    }
                                    None => Some(p.name.name.clone()),
                                })
                                .collect::<Vec<_>>()
                                .join("; ");
                            params_sig = ps;
                        }
                        LocusMember::Fn(fd) => {
                            if fd.name.name.starts_with("__") {
                                continue;
                            }
                            members.push(DocMember {
                                signature: demangle(&doc_fn_signature(fd)),
                                doc: doc_comment_above(
                                    src,
                                    fd.name.span.start.as_usize(),
                                ),
                            });
                        }
                        _ => {}
                    }
                }
                let leaf = path.rsplit("::").next().unwrap_or(path);
                let sig = if params_sig.is_empty() {
                    format!("locus {}", leaf)
                } else {
                    demangle(&format!(
                        "locus {} {{ params {{ {} }} }}",
                        leaf, params_sig
                    ))
                };
                groups.entry(ns_of(path)).or_default().push(DocEntry {
                    kind: "locus",
                    name: path.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        l.name.span.start.as_usize(),
                    ),
                    members,
                });
            }
            _ => {}
        }
    }

    // Signature-table fns with no .hl decl (C-primitive-backed).
    let covered: std::collections::BTreeSet<String> = groups
        .values()
        .flatten()
        .map(|e| e.name.clone())
        .collect();
    for surface in hale_types::stdlib_surface::SURFACES {
        for entry in surface.fns {
            let f = entry.name;
            if f.starts_with("__") {
                continue;
            }
            let mut segs: Vec<&str> = vec!["std"];
            segs.extend(surface.ns.iter().copied());
            segs.push(f);
            let path = segs.join("::");
            if covered.contains(&path) {
                continue;
            }
            let sig = match hale_types::stdlib_surface::signature_for(&segs)
            {
                Some(sig) => {
                    let ps = sig
                        .params
                        .iter()
                        .map(|t| lsp::sig_ty_str(t).to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut d = format!(
                        "fn {}({}) -> {}",
                        f,
                        ps,
                        lsp::sig_ty_str(&sig.ret)
                    );
                    if let Some(e) = sig.fallible {
                        d.push_str(&format!(" fallible({})", e));
                    }
                    d
                }
                None => format!("fn {}(…)", f),
            };
            groups.entry(ns_of(&path)).or_default().push(DocEntry {
                kind: "fn",
                name: path,
                signature: sig,
                doc: String::new(),
                members: Vec::new(),
            });
        }
    }

    // Render.
    let mut md = String::from("# API — std\n");
    let mut json_items: Vec<serde_json::Value> = Vec::new();
    for (ns, entries) in &groups {
        md.push_str(&format!("\n## {}\n", ns));
        for e in entries {
            md.push_str(&format!(
                "\n### {}\n\n```hale\n{}\n```\n",
                e.name, e.signature
            ));
            if let Some(cls) = effect_line(&e.name) {
                md.push_str(&format!("\n{}\n", cls));
            }
            if !e.doc.is_empty() {
                md.push_str(&format!("\n{}\n", e.doc));
            }
            for m in &e.members {
                md.push_str(&format!(
                    "\n- `{}`{}\n",
                    m.signature,
                    if m.doc.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", m.doc.replace('\n', " "))
                    }
                ));
            }
            if json {
                json_items.push(serde_json::json!({
                    "kind": e.kind,
                    "name": e.name,
                    "signature": e.signature,
                    "effects": effect_classes(&e.name),
                    "doc": e.doc,
                    "members": e.members.iter().map(|m| serde_json::json!({
                        "signature": m.signature, "doc": m.doc
                    })).collect::<Vec<_>>(),
                }));
            }
        }
    }
    let rendered = if json {
        serde_json::to_string_pretty(&json_items)
            .unwrap_or_else(|_| "[]".into())
            + "\n"
    } else {
        md
    };
    match out_path {
        Some(p) => {
            if let Err(e) = fs::write(&p, rendered) {
                eprintln!("hale doc: could not write {}: {}", p.display(), e);
                return ExitCode::from(1);
            }
            eprintln!("wrote {}", p.display());
        }
        None => print!("{}", rendered),
    }
    ExitCode::SUCCESS
}

struct DocMember {
    signature: String,
    doc: String,
}

struct DocEntry {
    kind: &'static str,
    name: String,
    signature: String,
    doc: String,
    members: Vec<DocMember>,
}

/// The effect classes for a `std::` path, for `--json` consumers.
/// Empty vec = pure; `None` = no registry row (a locus or type).
fn effect_classes(path: &str) -> Option<Vec<String>> {
    let segs: Vec<&str> = path.split("::").collect();
    let set = hale_types::stdlib_surface::effects_for(&segs)?;
    Some(hale_types::frontier::render_effects(set))
}

/// The effect classification for a `std::` path, as a doc line.
///
/// Read straight out of the registry rather than written down here:
/// every surface entry already carries an `EffectSet`, and the
/// generator was walking those entries to print signatures while
/// ignoring the column sitting next to them. Deriving it means the
/// published catalogue cannot drift from what the checker enforces —
/// a hand-maintained table of 327 rows certainly would.
///
/// `None` for anything with no registry row (locus and type paths,
/// which are tracked separately) so those entries render unchanged.
fn effect_line(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split("::").collect();
    let set = hale_types::stdlib_surface::effects_for(&segs)?;
    let classes = hale_types::frontier::render_effects(set);
    if classes.is_empty() {
        // PURE is a real answer, and a useful one: it is what makes a
        // fn callable from a `@no_syscall` / `@deterministic` context.
        return Some("**Effects:** none — callable under any assertion.".into());
    }
    Some(format!(
        "**Effects:** {}",
        classes
            .iter()
            .map(|c| format!("`{}`", c))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The `///` block directly above the line holding `anchor`
/// (byte offset). Decorator lines (`@hot`, `@form(...)`) between
/// the docs and the declaration are stepped over.
fn doc_comment_above(src: &str, anchor: usize) -> String {
    let lines: Vec<&str> = src.lines().collect();
    // Line index containing the anchor offset.
    let mut off = 0usize;
    let mut anchor_line = 0usize;
    for (i, l) in lines.iter().enumerate() {
        let end = off + l.len() + 1;
        if anchor < end {
            anchor_line = i;
            break;
        }
        off = end;
    }
    let mut i = anchor_line;
    // Step over decorator-only lines above the decl.
    while i > 0 {
        let prev = lines[i - 1].trim();
        if prev.starts_with('@') {
            i -= 1;
        } else {
            break;
        }
    }
    let mut docs: Vec<&str> = Vec::new();
    while i > 0 {
        let prev = lines[i - 1].trim();
        if let Some(text) = prev.strip_prefix("///") {
            docs.push(text.strip_prefix(' ').unwrap_or(text));
            i -= 1;
        } else {
            break;
        }
    }
    docs.reverse();
    docs.join("\n").trim().to_string()
}

fn doc_fn_signature(fd: &hale_syntax::ast::FnDecl) -> String {
    let ps = fd
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name.name, lsp::type_expr_str(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sig = format!("fn {}({})", fd.name.name, ps);
    if let Some(r) = &fd.ret {
        sig.push_str(&format!(" -> {}", lsp::type_expr_str(r)));
    }
    if let Some(e) = &fd.fallible {
        sig.push_str(&format!(" fallible({})", lsp::type_expr_str(e)));
    }
    sig
}

fn doc_entries_for(
    src: &str,
    program: &hale_syntax::ast::Program,
) -> Vec<DocEntry> {
    use hale_syntax::ast::{LocusMember, TopDecl, TypeDeclBody};
    let mut out = Vec::new();
    for item in &program.items {
        match item {
            TopDecl::Fn(fd) => {
                if fd.name.name.starts_with("__")
                    || fd.name.name == "main"
                {
                    continue;
                }
                out.push(DocEntry {
                    kind: "fn",
                    name: fd.name.name.clone(),
                    signature: doc_fn_signature(fd),
                    doc: doc_comment_above(
                        src,
                        fd.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Locus(l) => {
                if l.name.name.starts_with("__") {
                    continue;
                }
                let mut sig = format!("locus {}", l.name.name);
                let mut members = Vec::new();
                for m in &l.members {
                    match m {
                        LocusMember::Params(pb) => {
                            let ps = pb
                                .params
                                .iter()
                                .map(|p| match &p.ty {
                                    Some(t) => format!(
                                        "{}: {}",
                                        p.name.name,
                                        lsp::type_expr_str(t)
                                    ),
                                    None => p.name.name.clone(),
                                })
                                .collect::<Vec<_>>()
                                .join("; ");
                            sig.push_str(&format!(
                                " {{ params {{ {} }} }}",
                                ps
                            ));
                        }
                        LocusMember::Fn(fd) => {
                            if fd.name.name.starts_with("__") {
                                continue;
                            }
                            members.push(DocMember {
                                signature: doc_fn_signature(fd),
                                doc: doc_comment_above(
                                    src,
                                    fd.name.span.start.as_usize(),
                                ),
                            });
                        }
                        _ => {}
                    }
                }
                out.push(DocEntry {
                    kind: "locus",
                    name: l.name.name.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        l.name.span.start.as_usize(),
                    ),
                    members,
                });
            }
            TopDecl::Type(t) => {
                if t.name.name.starts_with("__") {
                    continue;
                }
                let sig = match &t.body {
                    TypeDeclBody::Struct(fields) => {
                        let fs = fields
                            .iter()
                            .map(|f| {
                                format!(
                                    "{}: {};",
                                    f.name.name,
                                    lsp::type_expr_str(&f.ty)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        format!("type {} {{ {} }}", t.name.name, fs)
                    }
                    TypeDeclBody::Enum(vs) => {
                        let names = vs
                            .iter()
                            .map(|v| v.name.name.clone())
                            .collect::<Vec<_>>()
                            .join(" | ");
                        format!("type {} = enum {{ {} }}", t.name.name, names)
                    }
                    TypeDeclBody::Alias(inner) => format!(
                        "type {} = {}",
                        t.name.name,
                        lsp::type_expr_str(inner)
                    ),
                };
                out.push(DocEntry {
                    kind: "type",
                    name: t.name.name.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        t.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Topic(t) => {
                let mut sig = format!(
                    "topic {} {{ payload: {}",
                    t.name.name,
                    lsp::type_expr_str(&t.payload)
                );
                if let Some(k) = &t.keyed_by {
                    sig.push_str(&format!("; keyed_by {}", k.name));
                }
                sig.push_str(" }");
                out.push(DocEntry {
                    kind: "topic",
                    name: t.name.name.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        t.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Interface(iface) => {
                let ms = iface
                    .methods
                    .iter()
                    .map(|m| {
                        let ps = m
                            .params
                            .iter()
                            .map(|p| {
                                format!(
                                    "{}: {}",
                                    p.name.name,
                                    lsp::type_expr_str(&p.ty)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        let ret = m
                            .ret
                            .as_ref()
                            .map(|r| {
                                format!(" -> {}", lsp::type_expr_str(r))
                            })
                            .unwrap_or_default();
                        format!("fn {}({}){};", m.name.name, ps, ret)
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(DocEntry {
                    kind: "interface",
                    name: iface.name.name.clone(),
                    signature: format!(
                        "interface {} {{ {} }}",
                        iface.name.name, ms
                    ),
                    doc: doc_comment_above(
                        src,
                        iface.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Const(c) => {
                out.push(DocEntry {
                    kind: "const",
                    name: c.name.name.clone(),
                    signature: format!(
                        "const {}: {}",
                        c.name.name,
                        lsp::type_expr_str(&c.ty)
                    ),
                    doc: doc_comment_above(
                        src,
                        c.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            _ => {}
        }
    }
    out
}


/// `hale bench [file | dir] [-run <substr>] [--json]` — the Layer-3
/// runner (spec/testing.md). Discovers `*_bench.hl` files; each
/// zero-param free fn named `bench_*` is a benchmark. The runner
/// appends a synthesized driver `main` to a temp copy IN THE SAME
/// DIRECTORY (so relative imports resolve identically), compiles at
/// the release profile with the same `[ffi]` pickup as build/test,
/// and runs it. The driver self-calibrates: batch sizes grow ×10
/// until a batch takes ≥100ms, then reports ns/op and allocs/op
/// (`std::diag::heap_alloc_count` — shown as `-` when the counting
/// shim is absent). Baselines and `-compare` remain planned.
fn run_bench(args: &[String]) -> ExitCode {
    let mut target: Option<PathBuf> = None;
    let mut run_filter: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-run" || a == "--run" {
            match args.get(i + 1) {
                Some(v) => {
                    run_filter = Some(v.clone());
                    i += 2;
                }
                None => {
                    eprintln!("hale bench: {} requires a substring", a);
                    return ExitCode::from(2);
                }
            }
        } else if a == "--json" {
            json = true;
            i += 1;
        } else if a.starts_with('-') {
            eprintln!("hale bench: unknown flag `{}`", a);
            return ExitCode::from(2);
        } else if target.is_none() {
            target = Some(PathBuf::from(a));
            i += 1;
        } else {
            eprintln!("hale bench: unexpected extra argument `{}`", a);
            return ExitCode::from(2);
        }
    }
    let target = target
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut files: Vec<PathBuf> = Vec::new();
    if target.is_file() {
        files.push(target.clone());
    } else if target.is_dir() {
        collect_bench_files(&target, &mut files);
        files.sort();
    } else {
        eprintln!("hale bench: {} not found", target.display());
        return ExitCode::from(1);
    }
    if files.is_empty() {
        eprintln!("hale bench: no *_bench.hl files under {}", target.display());
        return ExitCode::from(1);
    }

    let mut failed = false;
    let mut json_items: Vec<serde_json::Value> = Vec::new();
    for f in &files {
        match run_bench_file(f, run_filter.as_deref()) {
            Ok(results) => {
                for r in results {
                    if json {
                        json_items.push(serde_json::json!({
                            "file": f.display().to_string(),
                            "name": r.name,
                            "iters": r.iters,
                            "ns_per_op": r.ns_per_op,
                            "allocs_per_op": r.allocs_per_op,
                        }));
                    } else {
                        let allocs = match r.allocs_per_op {
                            Some(a) => format!("{} allocs/op", a),
                            None => "- allocs/op".to_string(),
                        };
                        println!(
                            "{:<40} {:>12} iters {:>12} ns/op   {}",
                            r.name, r.iters, r.ns_per_op, allocs
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!("hale bench: {}: {}", f.display(), e);
                failed = true;
            }
        }
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_items)
                .unwrap_or_else(|_| "[]".into())
        );
    }
    if failed {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn collect_bench_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "vendor" || name.starts_with('.') {
                continue;
            }
            collect_bench_files(&p, out);
        } else if name.ends_with("_bench.hl") {
            out.push(p);
        }
    }
}

struct BenchResult {
    name: String,
    iters: i64,
    ns_per_op: i64,
    allocs_per_op: Option<i64>,
}

fn run_bench_file(
    entry: &Path,
    filter: Option<&str>,
) -> Result<Vec<BenchResult>, String> {
    let src = fs::read_to_string(entry)
        .map_err(|e| format!("read: {}", e))?;
    let program = hale_syntax::parse_source(&src)
        .map_err(|_| "does not parse".to_string())?;

    // Contract: bench_* zero-param free fns; no main of its own
    // (the driver synthesizes one).
    let mut benches: Vec<String> = Vec::new();
    let mut has_main = false;
    for item in &program.items {
        if let hale_syntax::ast::TopDecl::Fn(fd) = item {
            if fd.name.name == "main" {
                has_main = true;
            }
            if fd.name.name.starts_with("bench_") && fd.params.is_empty() {
                if let Some(f) = filter {
                    if !fd.name.name.contains(f) {
                        continue;
                    }
                }
                benches.push(fd.name.name.clone());
            }
        }
    }
    if has_main {
        return Err(
            "a *_bench.hl must not define `main` — the runner \
             synthesizes the driver"
                .into(),
        );
    }
    if benches.is_empty() {
        return Ok(Vec::new());
    }

    // Synthesized driver: per bench fn, calibrate batch ×10 until a
    // batch takes >= 100ms, then report the final batch's numbers.
    let mut driver = String::from("\n// --- hale bench driver (synthesized) ---\n");
    for b in &benches {
        driver.push_str(&format!(
            r#"fn __bench_drive_{b}() {{
    let mut batch = 1;
    let mut elapsed = 1;
    let mut allocs = 0;
    while true {{
        let a0 = std::diag::heap_alloc_count();
        let t0 = std::time::monotonic_ns();
        let mut i = 0;
        while i < batch {{ {b}(); i = i + 1; }}
        let t1 = std::time::monotonic_ns();
        let a1 = std::diag::heap_alloc_count();
        elapsed = t1 - t0;
        if elapsed < 1 {{ elapsed = 1; }}
        allocs = a1 - a0;
        if a0 < 0 {{ allocs = 0 - batch; }}
        if elapsed >= 100000000 {{ break; }}
        if batch >= 100000000 {{ break; }}
        batch = batch * 10;
    }}
    println("HALE_BENCH {b} ", batch, " ", elapsed / batch, " ", allocs / batch);
}}
"#,
            b = b
        ));
    }
    driver.push_str("fn main() {\n");
    for b in &benches {
        driver.push_str(&format!("    __bench_drive_{}();\n", b));
    }
    driver.push_str("}\n");

    // Temp copy in the SAME directory so relative imports resolve.
    let dir = entry.parent().unwrap_or(Path::new("."));
    let stem = entry
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bench".into());
    let tmp_src = dir.join(format!(
        ".{}_driver_{}.hl",
        stem,
        std::process::id()
    ));
    let mut augmented = src.clone();
    augmented.push_str(&driver);
    fs::write(&tmp_src, &augmented)
        .map_err(|e| format!("write driver: {}", e))?;

    let compile = (|| -> Result<PathBuf, String> {
        let (prog, renames, _sources, _bases, ctx) =
            match parse_with_imports(&tmp_src) {
                Ok(x) => x,
                Err(errors) => {
                    let msg = errors
                        .iter()
                        .map(ImportDiag::render)
                        .collect::<Vec<_>>()
                        .join("\n");
                    return Err(msg);
                }
            };
        let mut bin = std::env::temp_dir();
        let mut h = DefaultHasher::new();
        h.write(entry.display().to_string().as_bytes());
        h.write_u32(std::process::id());
        bin.push(format!("hale_bench_{:016x}", h.finish()));
        let options = collect_ffi_from_imports(
            &ctx.imports,
            &ctx.entry_dir,
            ctx.workspace_root.as_deref(),
        );
        // Release profile on purpose: benchmarks measure the
        // shipped optimization level.
        hale_codegen::build_executable_with_options(
            &prog, &bin, &renames, &options,
        )
        .map_err(|e| format!("codegen error: {:?}", e))?;
        Ok(bin)
    })();
    let _ = fs::remove_file(&tmp_src);
    let bin = compile?;

    let out = std::process::Command::new(&bin)
        .output()
        .map_err(|e| format!("run: {}", e));
    let _ = fs::remove_file(&bin);
    let out = out?;
    if !out.status.success() {
        return Err(format!(
            "bench binary exited {:?}:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("HALE_BENCH ") else {
            // Benchmarks may print their own output — pass through.
            if !line.trim().is_empty() {
                println!("{}", line);
            }
            continue;
        };
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() != 4 {
            continue;
        }
        let allocs: i64 = parts[3].parse().unwrap_or(0);
        results.push(BenchResult {
            name: parts[0].to_string(),
            iters: parts[1].parse().unwrap_or(0),
            ns_per_op: parts[2].parse().unwrap_or(0),
            allocs_per_op: if allocs < 0 { None } else { Some(allocs) },
        });
    }
    Ok(results)
}

fn run_lex_file(path: &Path) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    };
    match hale_syntax::lex(&source) {
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
    match hale_syntax::parse_source(&source) {
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

/// Per-build path-rename table for cross-seed imports
/// (v1.x-IMPORT). Each entry maps a qualified-name segment vector
/// (e.g. `["foo", "Bar"]`) to the mangler-generated symbol name
/// (`__lib_foo_<stem>_Bar`). Passed to
/// `build_executable_with_imports` so codegen can resolve
/// `alias::Name` references in user code.
type ImportRenames = Vec<(Vec<String>, String)>;

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
#[derive(Default)]
struct AliasScopes {
    /// One row per import site: (declaring seed, alias, the lib the
    /// alias names). Seeds and libs are canonical paths — the same
    /// identity `seed_cache` and `lib_canonical_id` key off, so two
    /// aliases for the same lib agree and never look contested.
    bindings: Vec<(PathBuf, String, PathBuf)>,
    /// Declaring seed -> its source files, canonical. A seed's files
    /// share one alias namespace (they share one decl namespace), so
    /// the rewrite applies to all of them.
    files: BTreeMap<PathBuf, Vec<PathBuf>>,
}

impl AliasScopes {
    fn record_binding(&mut self, seed: &Path, alias: &str, lib: &Path) {
        self.bindings.push((
            seed.to_path_buf(),
            alias.to_string(),
            lib.to_path_buf(),
        ));
    }

    fn record_files(&mut self, seed: &Path, files: Vec<PathBuf>) {
        self.files
            .entry(seed.to_path_buf())
            .or_default()
            .extend(files);
    }
}

/// Walk upward from `start` looking for a `Cargo.toml`; the first
/// directory containing one is treated as the workspace root.
/// Used for the workspace-root fallback in import resolution.
/// Returns `None` if no Cargo.toml is found before hitting the
/// filesystem root (standalone-shipped binaries hit this — they
/// can still use entry-relative imports, just not the
/// workspace-fallback path).
/// Walk up from `start` looking for a workspace anchor. Hale
/// repos are anchored by `hale.toml`; hale's own dev tree
/// is also a cargo workspace, so `Cargo.toml` works as a fallback
/// anchor for compiler-side development. The first one found
/// wins. The result is the directory containing the anchor.
///
/// 2026-05-22: anchor used as the basis for path-based mangling
/// (`lib_canonical_id`). Two consumers in the same workspace
/// importing the same lib produce identical mangled names
/// because they compute the lib's path relative to the same
/// root.
/// `find_workspace_root` for sibling modules.
pub(crate) fn find_workspace_root_pub(start: &Path) -> Option<PathBuf> {
    find_workspace_root(start)
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    // Canonicalize first so the walk-up traverses real ancestor
    // directories regardless of whether `start` came in relative
    // (e.g., `hale build apps/a/main.hl` from the repo root).
    // Without this, relative paths walk `apps/a/main.hl` →
    // `apps/a` → `apps` → "" and never reach the actual
    // workspace root containing the hale.toml.
    let canon = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut cur = if canon.is_file() {
        canon.parent()?.to_path_buf()
    } else {
        canon
    };
    loop {
        if cur.join("hale.toml").is_file() || cur.join("Cargo.toml").is_file()
        {
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
    /// `<importer_dir>/<path>.hl` (single-file lib).
    SingleFile(PathBuf),
    /// `<importer_dir>/<path>/` or `<workspace_root>/<path>/`
    /// (directory bundle — one seed of multiple `.hl` files).
    Directory(PathBuf),
}

/// Try the three resolution strategies in order: entry-relative
/// single file, entry-relative directory, workspace-root directory.
/// Returns `None` if none of them hit.
/// Stable, sanitized identifier for an imported lib seed. Used
/// as the mangler's namespace key so two apps importing the same
/// lib produce identical mangled symbols (cross-app DTO contracts
/// become symbol-identical without any annotation or config flag).
///
/// Identity basis:
///   - Workspace-root-relative path when a workspace root is in
///     scope (`<repo>/hale.toml` found by `find_workspace_root`).
///     Two apps in the same monorepo importing the same lib see
///     the same relative path → same id.
///   - File-name fallback when no workspace root is available
///     (single-file builds outside any toml-rooted repo). Less
///     collision-safe but the only stable thing visible.
///
/// All non-identifier characters in the path collapse to `_` so
/// the result is a valid C / LLVM symbol component.
fn lib_canonical_id(target: &ImportTarget, workspace_root: Option<&Path>) -> String {
    let path = match target {
        ImportTarget::SingleFile(p) => p.clone(),
        ImportTarget::Directory(d) => d.clone(),
    };
    let canon = path.canonicalize().unwrap_or(path);
    let basis: PathBuf = if let Some(root) = workspace_root {
        let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        canon
            .strip_prefix(&root_canon)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| {
                // Lib lives outside the workspace root — fall
                // back to its file name so we still get SOMETHING
                // stable for the mangler. Two such libs at
                // different paths but sharing a basename would
                // collide; an explicit out-of-workspace import is
                // unusual enough that we accept this.
                canon
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| canon.clone())
            })
    } else {
        canon
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| canon.clone())
    };
    // Single-file imports keep the `.hl` suffix in the path which
    // would sanitize to `_ap` — strip it for readability.
    let basis_str = basis.to_string_lossy();
    let basis_str = basis_str.strip_suffix(".hl").unwrap_or(&basis_str);
    sanitize_identifier(basis_str)
}

fn sanitize_identifier(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    // Collapse runs of underscores so deeply-nested paths don't
    // produce eye-watering `___` sequences in symbol names.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_underscore = false;
    for ch in out.chars() {
        if ch == '_' {
            if !prev_underscore {
                collapsed.push('_');
            }
            prev_underscore = true;
        } else {
            collapsed.push(ch);
            prev_underscore = false;
        }
    }
    collapsed.trim_matches('_').to_string()
}

/// GH #763: `<dir>/main.hl` and `<dir>` name the SAME library.
///
/// A seed is a directory (F.19): every `.hl` file in it shares one
/// declaration namespace, and `main.hl` is that seed's entry file,
/// not a library of its own. So `import "../lib/main"` names the
/// seed `../lib`, exactly as `import "../lib"` does, and this
/// collapses the first spelling onto the second before anything
/// downstream derives an identity from the target.
///
/// Without the collapse the two spellings produced two library
/// identities — two `lib_key`s in `resolve_imports`, two `lib_id`s
/// in `lib_canonical_id`, two sets of mangled symbols. The `visited`
/// set is global across the build, so whichever spelling resolved
/// second found every file already parsed, registered no rename rows
/// under its own key, and its `alias::Name` references died at
/// codegen as `unknown qualified name` while the other alias worked.
///
/// Any OTHER single file stays its own library: rule 1 of the
/// resolution order (spec `projects.md`) is a real single-file
/// library, and only the `main.hl` entry spelling is a second name
/// for the directory around it.
///
/// `import "main"` from inside the directory itself is left alone —
/// collapsing it would make a seed import itself.
fn seed_dir_for_entry_file(single: &Path, importer_dir: &Path) -> Option<PathBuf> {
    if single.file_name().and_then(|s| s.to_str()) != Some("main.hl") {
        return None;
    }
    let dir = single.parent()?;
    if !dir.is_dir() {
        return None;
    }
    let canon_dir = dir.canonicalize().ok()?;
    let canon_importer = importer_dir.canonicalize().ok()?;
    if canon_dir == canon_importer {
        return None;
    }
    Some(dir.to_path_buf())
}

fn resolve_import(
    importer_dir: &Path,
    workspace_root: Option<&Path>,
    import_path: &str,
) -> Option<ImportTarget> {
    let single = importer_dir.join(format!("{}.hl", import_path));
    if single.is_file() {
        // GH #763: the entry file of a seed is the seed.
        if let Some(dir) = seed_dir_for_entry_file(&single, importer_dir) {
            return Some(ImportTarget::Directory(dir));
        }
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

/// Collect every `.hl` file at an import target. SingleFile
/// resolves to one path; Directory enumerates the dir, sorting
/// alphabetically for deterministic merge order (mirrors the
/// per-dir seed convention from F.19).
/// The transitive input set of a seed: its own `.hl` files and those
/// of every imported directory; the `hale.toml` of every such
/// directory when it has one, and every C source that manifest
/// declares under `[ffi] csrc`, exactly as the native build adds them
/// (a declared source is an input whether or not it exists yet — the
/// build reads it and fails on it). Canonical, sorted, each once. A
/// file that does not parse is still an input; only its imports go
/// unfollowed.
fn seed_inputs(target: &Path) -> Result<Vec<PathBuf>, String> {
    let workspace_root = find_workspace_root(target);
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut dirs: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut queue: Vec<PathBuf> = collect_ap_files(target)?;
    while let Some(f) = queue.pop() {
        let canon = f.canonicalize().unwrap_or_else(|_| f.clone());
        if !seen.insert(canon.clone()) {
            continue;
        }
        let dir = canon.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        dirs.insert(dir.clone());
        let Ok(src) = fs::read_to_string(&canon) else { continue };
        let Ok(program) = hale_syntax::parse_source(&src) else { continue };
        for imp in &program.imports {
            if let Some(t) = resolve_import(&dir, workspace_root.as_deref(), &imp.path) {
                if let Ok(files) = collect_target_files(&t) {
                    queue.extend(files);
                }
            }
        }
    }
    for dir in dirs {
        let manifest = dir.join("hale.toml");
        if !manifest.is_file() {
            continue;
        }
        seen.insert(manifest.canonicalize().unwrap_or(manifest));
        if let Ok(Some(ffi)) = crate::pkg::read_lib_ffi(&dir) {
            for csrc in ffi.csrc {
                let p = dir.join(csrc);
                seen.insert(p.canonicalize().unwrap_or(p));
            }
        }
    }
    Ok(seen.into_iter().collect())
}

fn collect_target_files(t: &ImportTarget) -> Result<Vec<PathBuf>, String> {
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

/// Resolve a flat list of import directives originating from one
/// importer directory: for each import, locate the target on disk
/// (entry-relative file or dir, workspace-root fallback dir),
/// parse every `.hl` file, mangle each sub-program with the
/// import alias + the file's stem, and merge the mangled items
/// into `merged_items`. Populates `renames` with
/// `(["<alias>", "<TopName>"], mangled_name)` entries so the
/// codegen can resolve `alias::Name` references downstream.
///
/// Imports inside the imported libs ARE followed (A4, G34): for
/// each lib file's own `import` directives, recurse with the lib's
/// directory as the importer_dir. The `visited` canonical-path set
/// breaks cycles. Each lib gets its own alias-prefixed mangled
/// names, so a transitive util lib reached through two different
/// libs lives twice in the binary — no re-export, no dedup, just
/// per-importer scoped resolution.
/// #345: the merged user effect-class table.
///
/// Carries declared-ness, not just names. A name reaches the table two
/// ways — an `effect NAME;` DECLARATION, or a mere REFERENCE in an
/// `@effects(...)` clause — and only the first makes the class real.
/// Without the distinction a typo interns a fresh class that nothing
/// carries, so `@effects(none: { monye })` is vacuously satisfied and
/// reports success: the exact silently-false certificate this analysis
/// exists to rule out.
#[derive(Default)]
struct EffectTable {
    names: Vec<String>,
    declared: std::collections::BTreeSet<String>,
    /// #354: composed definitions, index-parallel to `names`. Members
    /// are remapped into THIS table on absorb — a definition holds
    /// `EffectClass::User` indices, so carrying it across a seed
    /// boundary without remapping aliases it exactly like any other
    /// class reference.
    defs: Vec<Option<Vec<hale_syntax::ast::EffectClass>>>,
}

impl EffectTable {
    fn from_seed(p: &Program) -> Self {
        let mut t = EffectTable::default();
        t.absorb(p);
        t
    }

    /// Union `p`'s table into this one and return the index map that
    /// rewrites `p`'s `User(i)` into this table.
    fn absorb(&mut self, p: &Program) -> Vec<u16> {
        for &i in &p.declared_effects {
            if let Some(n) = p.effect_names.get(i as usize) {
                self.declared.insert(n.clone());
            }
        }
        let map: Vec<u16> = p
            .effect_names
            .iter()
            .map(|n| {
                let at = self
                    .names
                    .iter()
                    .position(|e| e == n)
                    .unwrap_or_else(|| {
                        self.names.push(n.clone());
                        self.defs.push(None);
                        self.names.len() - 1
                    });
                at as u16
            })
            .collect();
        // Carry definitions across, remapping their MEMBERS. A member
        // is a `User(i)` in the source seed's numbering; storing it
        // unremapped would silently point the definition at whatever
        // class holds that index in the merged table.
        for (i, def) in p.effect_defs.iter().enumerate() {
            let Some(members) = def else { continue };
            let Some(&to) = map.get(i) else { continue };
            let remapped: Vec<hale_syntax::ast::EffectClass> = members
                .iter()
                .map(|m| match m {
                    hale_syntax::ast::EffectClass::User(j) => map
                        .get(*j as usize)
                        .map(|&k| hale_syntax::ast::EffectClass::User(k))
                        .unwrap_or(*m),
                    other => *other,
                })
                .collect();
            if let Some(slot) = self.defs.get_mut(to as usize) {
                *slot = Some(remapped);
            }
        }
        map
    }

    fn declared_indices(&self) -> Vec<u16> {
        self.names
            .iter()
            .enumerate()
            .filter(|(_, n)| self.declared.contains(*n))
            .map(|(i, _)| i as u16)
            .collect()
    }
}

/// GH #806: an input `check` and `verify` needed and could not READ.
///
/// Not a [`hale_syntax::Diag`]: there is no text to position it in —
/// the file never opened, or the target is not there at all. That is
/// why it travelled as a bare `eprintln!` plus a non-zero exit for as
/// long as it did, and why it was the last thing on the check path
/// still doing so: under `--json` the stream was EMPTY and the exit
/// was 1, the exact shape GH #777 retired for diagnostics, so a gate
/// reading the stream could not tell a missing target or an
/// unreadable file from a crash. It is a record like everything else
/// now, at `"line":0,"col":0` — no position, because there is no text
/// to have a position in — and the text channel prints exactly what
/// it always printed.
struct IoDiag {
    /// The path the failure is about: the target itself when the
    /// target cannot be read, otherwise the file that would not open.
    path: PathBuf,
    /// What the TEXT channel prints — byte for byte the line the
    /// `eprintln!` at the failing site printed before this existed.
    text: String,
    /// The record's `message`: the OS error, which is the part a
    /// consumer can act on.
    message: String,
}

impl IoDiag {
    /// A file that would not open. `text` is the failing site's own
    /// sentence — it names the importing seed, or the file, as it
    /// always did — and the record carries the OS error alone.
    fn read(path: &Path, err: &std::io::Error, text: String) -> Self {
        Self {
            path: path.to_path_buf(),
            text,
            message: err.to_string(),
        }
    }

    /// A target (or an import target) whose `.hl` files could not be
    /// collected. Those messages are prose — `not a file or
    /// directory: …` — and for the commonest case of all, a path that
    /// is simply not there, the OS has the better answer: ask it, and
    /// the record says `No such file or directory (os error 2)`. A
    /// target that DOES stat (an empty directory, a directory whose
    /// `read_dir` failed with an error already in the sentence) keeps
    /// the sentence.
    fn target(path: &Path, text: String) -> Self {
        let message = match fs::metadata(path) {
            Err(e) => e.to_string(),
            Ok(_) => text.clone(),
        };
        Self {
            path: path.to_path_buf(),
            text,
            message,
        }
    }

    /// The NDJSON record, through the one writer every `--json`
    /// record is formatted by.
    fn record(&self) -> String {
        render_json_record(
            &self.path.display().to_string(),
            0,
            0,
            "error",
            "io error",
            &self.message,
            "",
        )
    }
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
enum ImportDiag {
    /// A diagnostic raised in a file the resolver PARSED.
    Located {
        /// The file the diagnostic was raised in, as the resolver
        /// reached it — that spelling is what the user sees.
        file: PathBuf,
        /// The virtual base `file` was parsed at; 0 for the entry
        /// file, which `parse_with_imports` parses unshifted.
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
    /// and base, so there is no window to test and no fourth copy of
    /// [`file_owns_offset`].
    fn render(&self) -> String {
        match self {
            ImportDiag::Located {
                file,
                base,
                diag,
                source,
            } => diag.render_located(
                &file.display().to_string(),
                source,
                *base,
            ),
            ImportDiag::Io(io) => io.text.clone(),
        }
    }
}

/// Render every import diagnostic in `errors` to stderr, one per
/// line-group, and hand back the failing exit code.
fn report_import_diags(errors: &[ImportDiag]) -> ExitCode {
    for e in errors {
        eprintln!("{}", e.render());
    }
    ExitCode::from(1)
}

fn resolve_imports(
    imports: &[hale_syntax::ast::Import],
    importer_dir: &Path,
    workspace_root: Option<&Path>,
    visited: &mut std::collections::BTreeSet<PathBuf>,
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
    // #345: the MERGED user effect-class table. Each seed interns its
    // own `effect NAME;` declarations from zero, so the same index
    // means different classes in different seeds. Names are unioned
    // here and each seed's items are remapped into this table before
    // they are merged.
    effects: &mut EffectTable,
    // GH #746: the seed whose imports these are — the canonical path
    // of the entry target, or of the lib whose files are being
    // followed. Every alias in `imports` is recorded against it.
    scope_key: &Path,
    alias_scopes: &mut AliasScopes,
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
        let target = match resolve_import(importer_dir, workspace_root, &imp.path) {
            Some(t) => t,
            None => {
                eprintln!(
                    "could not resolve import \"{}\": tried {}/{}.hl, {}/{}/, \
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
        // Parse every file in the import target into a parallel
        // (file_path, stem, source, Program) list, recording the
        // canon path in `visited` so we don't double-parse.
        struct ParsedLibFile {
            path: PathBuf,
            canon: PathBuf,
            stem: String,
            source: String,
            program: hale_syntax::ast::Program,
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
            let program = match hale_syntax::parse_source_at(&source, base) {
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
                program,
            });
        }
        if parsed_files.is_empty() {
            // Every file already visited: the lib was resolved
            // earlier under some other alias. Its decls are
            // merged and mangled; only THIS alias's rename rows
            // are missing. lib_canonical_id keys mangled names
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
        // Compute a stable, sanitized identifier for this lib
        // derived from the canonical path of its directory (or
        // file). Same lib → same id → same mangled names across
        // importers. The user-chosen `alias` is still used as
        // the call-site reference (`alias::Name`) in the path-
        // rename table below, but the mangled symbols themselves
        // come from the path identity.
        let lib_id = lib_canonical_id(&target, workspace_root);
        let seed_renames =
            hale_codegen::mangle::build_seed_renames(&stem_prog_refs, &lib_id);
        // GH #714: the names that may head a qualified path in this
        // seed (its type decls). Everything else a path head can be
        // is a module alias of the seed's own imports, which the
        // mangler must leave intact so `alias::Name` still resolves
        // through the rename table — even when the seed also
        // declares a free fn of the alias's name.
        let seed_heads =
            hale_codegen::mangle::seed_path_heads(&stem_prog_refs);
        // GH #774: the seed as a whole, for binding a claim group
        // reference no declaration in it answers.
        let seed_binding = hale_codegen::mangle::SeedBinding {
            seed_id: &lib_id,
            declares_main: hale_codegen::mangle::seed_declares_main(
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
        }
        // Mangle each file's program with the shared map.
        for pf in parsed_files.iter_mut() {
            if trace {
                eprintln!("[import]     mangle start: {}", pf.path.display());
            }
            hale_codegen::mangle::mangle_with_renames_in_seed(
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
            )?;
        }
        // Move mangled items into the merged program; stash sources.
        for mut pf in parsed_files {
            // Remap BEFORE merging: `User(i)` indices are seed-local,
            // so concatenating two seeds' items without this aliases
            // seed A's class 0 onto seed B's class 0.
            if !pf.program.effect_names.is_empty() {
                let map = effects.absorb(&pf.program);
                hale_syntax::ast::remap_user_effects(
                    &mut pf.program.items,
                    &map,
                );
            }
            merged_items.extend(pf.program.items);
            sources.insert(pf.canon, pf.source);
            let _ = pf.path; // path was only needed for diagnostics above
        }
    }
    Ok(())
}

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

fn parse_with_imports(
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
fn scope_import_aliases(
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
                hale_codegen::mangle::rewrite_import_alias_heads(item, map);
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
struct UnscopedAliasUse {
    file: PathBuf,
    base: u32,
    diag: hale_syntax::Diag,
}

/// One qualified path as the lexer sees it: `head::next`, and the
/// span covering both segments.
struct QualifiedUse {
    head: String,
    text: String,
    span: hale_syntax::Span,
}

/// GH #762: every qualified path in `src`.
///
/// `offset` shifts the spans into the enclosing text and `limit`
/// clamps them: an f-string interpolation body is re-lexed from its
/// own text, whose byte offsets only approximate the file's when the
/// body carries escapes — the same clamp `FStringPart::Interp`
/// documents.
fn collect_qualified_uses(
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

/// The name a top-level decl introduces. Mirrors the mangler's
/// private `top_decl_name`; used only for the "a path head may name
/// this seed's own declaration" exemption below, where a miss costs
/// an exemption and never a false finding.
fn top_decl_ident(d: &hale_syntax::ast::TopDecl) -> Option<&str> {
    use hale_syntax::ast::TopDecl as T;
    match d {
        T::Locus(l) => Some(&l.name.name),
        T::Perspective(p) => Some(&p.name.name),
        T::Type(t) => Some(&t.name.name),
        T::Const(c) => Some(&c.name.name),
        T::Fn(f) => Some(&f.name.name),
        T::Interface(i) => Some(&i.name.name),
        T::Topic(t) => Some(&t.name.name),
        T::RingLayout(r) => Some(&r.name.name),
        T::Target(t) => Some(&t.name.name),
        T::Group(g) => Some(&g.name.name),
        T::Module(_) | T::Claims(_) | T::Constitution(_) => None,
    }
}

/// Render `to` for someone reading a diagnostic about a file in
/// `from_dir`: relative when the two share a root, else as it is.
fn display_relative(from_dir: &Path, to: &Path) -> String {
    let from = from_dir
        .canonicalize()
        .unwrap_or_else(|_| from_dir.to_path_buf());
    let to = to.canonicalize().unwrap_or_else(|_| to.to_path_buf());
    let fc: Vec<_> = from.components().collect();
    let tc: Vec<_> = to.components().collect();
    let common = fc
        .iter()
        .zip(tc.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return to.display().to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..fc.len() {
        parts.push("..".to_string());
    }
    for c in &tc[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        return ".".to_string();
    }
    parts.join("/")
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
fn unscoped_alias_uses(
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

/// Render a post-merge diagnostic, demultiplexing its (globally-unique,
/// `parse_source_at`-shifted) span back to the file it came from via
/// `file_bases`, so the output reads `path:line:col` against that file's
/// own source instead of an arbitrary file. Falls back to the entry
/// source if the span isn't in any known file range.
/// Resolve a merged-bundle span to `(path, line, col)` via the
/// file-base table. Shared by the text and JSON renderers for both
/// primary and related spans.
fn locate_span(
    span: hale_syntax::Span,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> Option<(String, usize, usize)> {
    let off = span.start.as_usize() as u32;
    for (base, path, len) in file_bases {
        if file_owns_offset(*base, *len, off) {
            let src = sources.get(path)?;
            let (l, c) = span.shifted(base.wrapping_neg()).line_col(src);
            return Some((path.display().to_string(), l, c));
        }
    }
    None
}

fn render_located(
    d: &hale_syntax::Diag,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    let off = d.span.start.as_usize() as u32;
    for (base, path, len) in file_bases {
        if file_owns_offset(*base, *len, off) {
            if let Some(src) = sources.get(path) {
                let mut out =
                    d.render_located(&path.display().to_string(), src, *base);
                // Secondary locations, each resolved through the
                // file table — a related span may live in a
                // DIFFERENT file than the primary.
                for (rspan, label) in &d.related {
                    if let Some((rf, rl, rc)) =
                        locate_span(*rspan, file_bases, sources)
                    {
                        out.push_str(&format!(
                            "\n    note: {} at {}:{}:{}",
                            label, rf, rl, rc
                        ));
                    }
                }
                return out;
            }
        }
    }
    let any = sources.values().next().map(|s| s.as_str()).unwrap_or("");
    d.render(any)
}

/// Does the file parsed at `base`, `len` bytes long, own merged
/// offset `off`? The window both renderers above and
/// `render_diag_json` below test a span against.
///
/// It is INCLUSIVE of `base + len`, the one-past-the-last-byte
/// position the `Eof` token carries (`Span::new(pos, pos)` at the end
/// of the source). A parse error that cites EOF — `expected }, got
/// Eof`, the missing closing brace, the commonest syntactic mistake
/// there is — sits exactly there, and a half-open window put it in no
/// file at all: it rendered with no filename and, once parse errors
/// reached `render_diag_json`, as `"file":"","line":0,"col":0`
/// (GH #777). Files are parsed at bases spaced `len + 1` apart
/// (`parse_files`, `resolve_imports`), so that byte belongs to no
/// other file; `file_of_span` has always read the window this way.
fn file_owns_offset(base: u32, len: u32, off: u32) -> bool {
    off >= base && off <= base.saturating_add(len)
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
struct ParseFailure {
    diags: Vec<hale_syntax::Diag>,
    /// GH #806: the target's own files that would not OPEN. They have
    /// no diagnostic — there is no text to raise one against — and
    /// they used to be printed here and counted only as a non-zero
    /// exit, so `check --json` on a seed it could not read emitted
    /// nothing at all.
    io: Vec<IoDiag>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    sources: BTreeMap<PathBuf, String>,
}

impl ParseFailure {
    /// Render as located text on stderr — what `build` and `run` have
    /// always printed, neither of which has a machine-readable channel
    /// — and hand back the exit status. `check` and `verify` take the
    /// other road, through [`CheckableFailure::report`].
    fn report_text(&self) -> ExitCode {
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

fn parse_files(
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
struct CheckableFailure {
    code: u8,
    diags: Vec<hale_syntax::Diag>,
    io: Vec<IoDiag>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    sources: BTreeMap<PathBuf, String>,
}

impl CheckableFailure {
    /// GH #806: an input that could not be read. Nothing has been
    /// printed yet — [`Self::report`] prints the sentence on stderr
    /// in text mode and the record on stdout under `--json`.
    fn from_io(io: IoDiag) -> Self {
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
    fn from_parse(f: ParseFailure) -> Self {
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
    fn report(&self) -> u8 {
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
fn collect_checkable(
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
fn retain_owned_advisories(
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
fn file_of_span(
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

/// Flags `check` / `verify` accept, and whether each takes a value.
/// Anything not on this list is a usage error rather than something
/// quietly ignored.
///
/// `--dump-topology` is deliberately absent from the value-taking
/// set: it takes its destination in the `=<path>` form ONLY, and a
/// bare `--dump-topology` writes to stdout. Making it consume a
/// following token would make `hale check --dump-topology app.hl`
/// ambiguous — is `app.hl` the artifact destination or the target? —
/// and flags are supposed to be positionable on either side.
/// Check every deployment declared in `[fleets]`.
///
/// Separate from `--matrix`, which is the ENTRYPOINT x ENVIRONMENT
/// axis. A fleet is an arrangement of deployed instances; an
/// environment is law bound to an entrypoint. A workspace declares
/// both, and `production` in one need not mean `production` in the
/// other.
fn run_fleet_all(from: Option<&Path>, if_declared: bool) -> ExitCode {
    let cwd = from
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut dir = cwd.canonicalize().unwrap_or(cwd);
    let manifest = loop {
        let m = dir.join("hale.toml");
        if m.exists() {
            break Some(m);
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break None,
        }
    };
    let Some(manifest) = manifest else {
        if if_declared {
            println!("no hale.toml at or above {}: no fleets declared", dir.display());
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "`hale fleet check` with no plan checks every fleet in \
             `[fleets]`, and no `hale.toml` was found at or above the \
             current directory. Name a plan explicitly, or add one."
        );
        return ExitCode::from(2);
    };
    let fleets = match crate::pkg::read_fleets(&manifest) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    // GH #408 Phase 7: `[fleet_trust]` binds every declared fleet.
    // A key that fails to load is a configuration error for the
    // whole run — skipping it would narrow the trust set silently.
    let trust_paths = match crate::pkg::read_fleet_trust(&manifest) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    let trust = match sign::Trust::load(&trust_paths) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    if fleets.is_empty() {
        if if_declared {
            println!("{} declares no [fleets]", manifest.display());
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "{} declares no `[fleets]`. Add `<name> = \"<plan path>\"` \
             entries, or name a plan explicitly — reporting success for \
             zero deployments would say nothing.",
            manifest.display()
        );
        return ExitCode::from(2);
    }
    let base = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut failed: Vec<(String, u8)> = Vec::new();
    for (name, rel) in &fleets {
        let path = base.join(rel);
        println!("=== fleet `{}` ({}) ===", name, path.display());
        if !path.exists() {
            eprintln!("  plan not found: {}", path.display());
            failed.push((name.clone(), 2));
            continue;
        }
        // Every fleet runs. Stopping at the first failure would report
        // a subset of the deployments as if it were all of them.
        match fleet::compose(&path, &trust) {
            Ok(artifact) => {
                let v: serde_json::Value =
                    serde_json::from_str(&artifact).unwrap_or_default();
                println!(
                    "  ok — {} instance(s), {} route(s), fleet_shape_hash {}",
                    v["instances"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["routes"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["fleet_shape_hash"].as_str().unwrap_or("?")
                );
            }
            Err(errs) => {
                for e in &errs {
                    eprintln!("  {}", e);
                }
                failed.push((name.clone(), 1));
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!("ok: {} fleet(s) checked", fleets.len());
        return ExitCode::SUCCESS;
    }
    eprintln!("{} of {} fleet(s) failed:", failed.len(), fleets.len());
    for (n, _) in &failed {
        eprintln!("  {}", n);
    }
    // Worst code wins, so a missing plan is not masked by an ordinary
    // claim failure elsewhere.
    ExitCode::from(failed.iter().map(|(_, c)| *c).max().unwrap_or(1))
}

/// `hale fleet check <plan>` / `hale fleet dump <plan>`.
///
/// `check` composes and reports; `dump` writes the fleet artifact.
/// Both fail on anything a composition cannot honestly build on — an
/// unverifiable component, a semantics mismatch, a component whose
/// own law fails, or endpoints that disagree about a wire contract.
fn run_fleet(rest: &[String]) -> ExitCode {
    let sub = rest.first().map(String::as_str);

    // GH #408 Phase 7: key handling and attestation live under the
    // same verb as composition — the fleet is where certificates
    // change hands.
    match sub {
        Some("keygen") => {
            let Some(prefix) = rest.get(1) else {
                eprintln!("hale fleet keygen <prefix>   write <prefix>.pem + <prefix>.pub.pem");
                return ExitCode::from(2);
            };
            return match sign::keygen(Path::new(prefix)) {
                Ok(key_id) => {
                    println!(
                        "ok: {prefix}.pem (private, 0600) and \
                         {prefix}.pub.pem — key_id {key_id}"
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::from(1)
                }
            };
        }
        Some("sign") => {
            let (file, key) = match (rest.get(1), rest.get(2), rest.get(3)) {
                (Some(f), Some(flag), Some(k)) if flag == "--key" => (f, k),
                _ => {
                    eprintln!("hale fleet sign <file> --key <priv.pem>   write <file>.sig (ES256 over exact bytes)");
                    return ExitCode::from(2);
                }
            };
            return match sign::sign(Path::new(file), Path::new(key)) {
                Ok((sig_path, key_id)) => {
                    println!(
                        "ok: {} — key_id {}",
                        sig_path.display(),
                        key_id
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::from(1)
                }
            };
        }
        Some("attest") => {
            let Some(plan) = rest.get(1) else {
                eprintln!("hale fleet attest <plan.json>   compare each instance's binary to its binary_sha256");
                return ExitCode::from(2);
            };
            return match fleet::attest(Path::new(plan)) {
                Ok(msg) => {
                    println!("{}", msg);
                    ExitCode::SUCCESS
                }
                Err(errs) => {
                    for e in &errs {
                        eprintln!("{}", e);
                    }
                    ExitCode::from(1)
                }
            };
        }
        _ => {}
    }

    // `--trust <pub.pem>` (repeatable) on check/dump: strict when
    // given, exactly like `[fleet_trust]` in the manifest.
    let mut args: Vec<&String> = Vec::new();
    let mut trust_paths: Vec<PathBuf> = Vec::new();
    // GH #566 F5: `--in <dir>` checks the fleets a workspace elsewhere
    // declares (a DNA candidate's worktree); `--if-declared` makes a
    // workspace with no fleets a success that says so, for a
    // verification step that runs on every workspace.
    let mut from: Option<PathBuf> = None;
    let mut if_declared = false;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if a == "--trust" {
            match it.next() {
                Some(k) => trust_paths.push(PathBuf::from(k)),
                None => {
                    eprintln!("--trust needs a public key path");
                    return ExitCode::from(2);
                }
            }
        } else if a == "--in" {
            match it.next() {
                Some(d) => from = Some(PathBuf::from(d)),
                None => {
                    eprintln!("--in needs a directory");
                    return ExitCode::from(2);
                }
            }
        } else if a == "--if-declared" {
            if_declared = true;
        } else {
            args.push(a);
        }
    }
    let sub = args.first().map(|s| s.as_str());
    let plan = args.get(1);
    // GH #408 Phase 5: `hale fleet check` with no plan checks EVERY
    // deployment the workspace declares. A repository usually has
    // more than one — production, staging, a reconciliation
    // arrangement — and checking whichever one you remembered to name
    // is the same partial-coverage problem `--matrix` solves for
    // entrypoints.
    if sub == Some("check") && plan.is_none() {
        if !trust_paths.is_empty() {
            eprintln!(
                "--trust with no plan: the all-fleets form takes its \
                 trust roots from `[fleet_trust]` in hale.toml, so one \
                 flag cannot quietly rebind every deployment"
            );
            return ExitCode::from(2);
        }
        return run_fleet_all(from.as_deref(), if_declared);
    }
    let (sub, plan) = match (sub, plan) {
        (Some("check"), Some(p)) | (Some("dump"), Some(p)) => {
            (sub.unwrap(), p)
        }
        _ => {
            eprintln!("hale fleet check [plan.json]   compose and check");
            eprintln!("                                (no plan: every fleet in [fleets];");
            eprintln!("                                 --in <dir> another workspace's, --if-declared: none is ok)");
            eprintln!("hale fleet dump  <plan.json>    write the fleet artifact");
            eprintln!("hale fleet attest <plan.json>   binaries match the plan's sha256 rows");
            eprintln!("hale fleet keygen <prefix>      ES256 keypair for signing");
            eprintln!("hale fleet sign <file> --key K  detached .sig over exact bytes");
            eprintln!();
            eprintln!("check/dump take --trust <pub.pem> (repeatable):");
            eprintln!("with trust roots declared, every component must");
            eprintln!("verify under one of them.");
            eprintln!();
            eprintln!("A plan names exact application INSTANCES and the");
            eprintln!("routes between them. It composes artifacts, never");
            eprintln!("source: matching wire identities establish");
            eprintln!("compatibility, but only an explicit route creates");
            eprintln!("a fleet edge.");
            return ExitCode::from(2);
        }
    };
    let trust = match sign::Trust::load(&trust_paths) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    match fleet::compose(Path::new(plan), &trust) {
        Ok(artifact) => {
            if sub == "dump" {
                print!("{}", artifact);
            } else {
                let v: serde_json::Value =
                    serde_json::from_str(&artifact).unwrap_or_default();
                println!(
                    "ok: fleet `{}` composed — {} instance(s), {} \
                     route(s), fleet_shape_hash {}",
                    v["name"].as_str().unwrap_or("?"),
                    v["instances"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["routes"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["fleet_shape_hash"].as_str().unwrap_or("?")
                );
            }
            ExitCode::SUCCESS
        }
        Err(errs) => {
            for e in &errs {
                eprintln!("{}", e);
            }
            ExitCode::from(1)
        }
    }
}

/// Flags that describe ONE evaluation: an artifact to emit, or a
/// baseline to gate against. Meaningless when the command runs many
/// evaluations.
const PER_SEED_FLAGS: &[&str] = &[
    "--dump-topology",
    "--dump-model",
    "--check-topology",
    "--check-topology-shape",
    "--dump-effects-manifest",
    "--check-effects-manifest",
    "--dump-resource-budget",
    "--check-resource-budget",
    "--dump-alloc-summary",
];

const CHECK_FLAGS: &[(&str, bool)] = &[
    ("--allow-unowned-subscriber", false),
    ("--check-effects-manifest", true),
    ("--check-resource-budget", true),
    ("--check-topology", true),
    ("--check-topology-shape", true),
    ("--dump-alloc-summary", false),
    ("--dump-effects-manifest", false),
    ("--dump-resource-budget", false),
    ("--dump-topology", false),
    // GH #476 Change 2: derive + print the canonical
    // ApplicationModel (internal format). The demand surface the
    // `hale model dump` shim routes through.
    ("--dump-model", false),
    ("--json", false),
    ("--no-warn-unbounded-alloc", false),
    // Retired opt-in spelling from when the unbounded-alloc survey
    // was off by default. It is a no-op now (the survey is on), but
    // it was accepted for a release and rejecting it would break
    // pipelines that still pass it. Accepted-because-ignored is
    // exactly what this list is replacing, so it is written down
    // rather than left to chance.
    ("--warn-unbounded-alloc", false),
    ("--warn-resource-leak", false),
    // GH #436
    ("--strict-secret", false),
    ("--sealable", false),
    ("--workspace", false),
    // GH #409
    ("--env", true),
    ("--matrix", false),
];

fn check_usage(verify: bool) {
    let cmd = if verify { "verify" } else { "check" };
    let what = if verify {
        "typecheck + analyze; EVERY finding, advisory included, fails \
         the run"
    } else {
        "typecheck + analyze a seed"
    };
    println!("hale {} [flags] <file | dir>    {}", cmd, what);
    println!();
    println!("A directory is ONE seed: the `.hl` files directly inside");
    println!("it are checked together, without recursing. Flags may");
    println!("appear before or after the target.");
    println!();
    println!("  --workspace                    check EVERY seed under the");
    println!("                                 target, each independently.");
    println!("                                 Skips vendor/, target/ and");
    println!("                                 dot-dirs. Every seed runs even");
    println!("                                 if an earlier one fails. It does");
    println!("                                 NOT connect seeds to each other.");
    println!("  --env <name>                   also adopt the constitution that");
    println!("                                 `[environments.<name>]` in hale.toml");
    println!("                                 requires. One entrypoint deployed to");
    println!("                                 two environments is checked twice.");
    println!("  --matrix                       check every (entrypoint, environment)");
    println!("                                 pair the manifest declares. An");
    println!("                                 entrypoint listed in NO environment");
    println!("                                 is an error, not a skip. Cannot be");
    println!("                                 combined with --env, --workspace, or");
    println!("                                 any per-evaluation artifact flag.");
    println!();
    println!("Claims and topology (spec/verification.md):");
    println!("  --dump-topology[=<path>]        emit the topology artifact");
    println!("                                 (JSON; stdout when bare).");
    println!("                                 Observational: it does not");
    println!("                                 change the exit status.");
    println!("  --check-topology <path>         gate on an EXACT artifact");
    println!("                                 snapshot — law, model and");
    println!("                                 provenance. Source motion");
    println!("                                 trips it.");
    println!("  --check-topology-shape <path>   gate on the model identity");
    println!("                                 (`shape_hash`) alone. Immune");
    println!("                                 to comments moving and to");
    println!("                                 claim renames.");
    println!();
    println!("Effects and budgets:");
    println!("  --dump-effects-manifest         emit the effects manifest");
    println!("  --check-effects-manifest <path> gate on a manifest baseline");
    println!("  --dump-resource-budget          emit the resource budget");
    println!("  --check-resource-budget <path>  gate on a budget baseline");
    println!("  --dump-alloc-summary            emit the allocation summary");
    println!();
    println!("Advisories:");
    println!("  --warn-resource-leak            enable the resource-leak lint");
    println!("  --strict-secret                 fail-closed `@secret` containment check");
    println!("  --sealable                      report which loci could be `@sealed`");
    println!("  --no-warn-unbounded-alloc       silence the unbounded-alloc lint");
    println!("  --allow-unowned-subscriber      permit a subscriber with no owner");
    println!("  --json                          machine-readable diagnostics");
}

/// `hale init [dir]` — bootstrap a project. Writes the canonical
/// minimal scaffold: a `hale.toml` skeleton (the manifest is
/// `[deps]`-only by design — a project is identified by its
/// directory name and its source, per spec/packages.md), a
/// hello-world `main.hl`, a first `tests/*_test.hl` so `hale test` works
/// from minute one, and a `.gitignore` covering the build artifact
/// and `vendor/`. Strictly non-destructive: an existing file is
/// never touched, only reported — so `init` is also safe to run in
/// a partially-scaffolded directory to fill in what's missing.
fn run_init(root: &Path) -> ExitCode {
    if let Err(e) = fs::create_dir_all(root) {
        eprintln!("hale init: cannot create {}: {}", root.display(), e);
        return ExitCode::from(1);
    }
    let name = root
        .canonicalize()
        .ok()
        .and_then(|p| {
            p.file_name().map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "app".to_string());

    let manifest = "\
# hale.toml — the project manifest. The only section is [deps]:\n\
# a project is identified by its directory name and its source\n\
# (no [package] metadata table exists). `hale fetch` clones each\n\
# dep into vendor/<name>/ and pins the resolved SHA in hale.lock.\n\
#\n\
# [deps]\n\
# helpers = { git = \"https://github.com/me/helpers\", tag = \"v0.1.0\" }\n\
\n\
[deps]\n";

    let main_hl = r#"/// Entry seed. Every `.hl` file in this directory shares one
/// scope — decompose by concern, not by visibility.
fn greeting() -> String {
    return "Hello from Hale.";
}

fn main() {
    println(greeting());
}
"#;

    // Tests live in a SUBDIRECTORY: a seed is the `.hl` files
    // directly in one directory, and a test file carries its own
    // `fn main` — beside main.hl it would collide. `tests/` is its
    // own seed importing the parent, the pond/stdlib convention.
    let test_hl = r#"// `hale test` discovers *_test.hl recursively. Each test file is
// its own program: it imports the seed under test and asserts —
// typechecked, next to the code.

import ".." as app;

fn main() {
    std::test::assert_eq_str(app::greeting(), "Hello from Hale.", "greeting");
}
"#;

    let gitignore = format!(
        "# the build artifact (`hale build .` names it after the directory)\n\
         /{}\n\
         # toolchain-managed dependency clones (`hale fetch`)\n\
         /vendor/\n",
        name
    );

    let files: &[(&str, &str)] = &[
        ("hale.toml", manifest),
        ("main.hl", main_hl),
        ("tests/main_test.hl", test_hl),
        (".gitignore", &gitignore),
    ];
    let mut wrote = 0usize;
    for (fname, content) in files {
        let path = root.join(fname);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if path.exists() {
            println!("kept    {} (already exists)", path.display());
            continue;
        }
        if let Err(e) = fs::write(&path, content) {
            eprintln!("hale init: cannot write {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
        println!("created {}", path.display());
        wrote += 1;
    }
    if wrote == 0 {
        println!("nothing to do — every scaffold file already exists");
    } else {
        println!();
        println!("next steps:");
        println!("    hale run {}      # compile + run", root.display());
        println!("    hale test {}     # run tests/", root.display());
        println!("    hale check {}    # typecheck + analyze", root.display());
    }
    ExitCode::SUCCESS
}

/// Every SEED under `root`: a directory holding one or more `.hl`
/// files directly. `check` operates on one seed and does not recurse
/// — correctly, since a directory is one compilation unit — so a
/// repository with many seeds needs something to enumerate them.
///
/// Skips `vendor` and dot-directories, matching `hale fmt`'s walk,
/// plus `target`. A seed you do not own is not yours to gate.
fn collect_seeds(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else { return };
    let mut has_hl = false;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "vendor" || name == "target" || name.starts_with('.')
            {
                continue;
            }
            subdirs.push(p);
        } else if name.ends_with(".hl") {
            has_hl = true;
        }
    }
    if has_hl {
        out.push(root.to_path_buf());
    }
    subdirs.sort();
    for d in subdirs {
        collect_seeds(&d, out);
    }
}

/// A `--flag value` / `--flag=value` reader over an explicit argv
/// slice. `flag_value` inside `run_check_impl` reads the process
/// argv; the arg parser needs the same rules before it has decided
/// what to run.
fn flag_value_in(
    rest: &[String],
    flag: &str,
) -> Result<Option<String>, String> {
    if let Some(eq) = rest.iter().find(|a| {
        a.starts_with(flag) && a.as_bytes().get(flag.len()) == Some(&b'=')
    }) {
        let v = &eq[flag.len() + 1..];
        if v.is_empty() {
            return Err(format!("{}= requires a value", flag));
        }
        return Ok(Some(v.to_string()));
    }
    if let Some(i) = rest.iter().position(|a| a == flag) {
        return match rest.get(i + 1) {
            Some(v) if !v.starts_with('-') => Ok(Some(v.clone())),
            _ => Err(format!(
                "{} requires a value. Use `{} <name>` or `{}=<name>`.",
                flag, flag, flag
            )),
        };
    }
    Ok(None)
}

/// Which constitution does environment `env` require? Walks up from
/// the target for the nearest `hale.toml`, so `hale check apps/a
/// --env prod` works from anywhere in the tree.
fn resolve_env_constitution(
    target: &Path,
    env: &str,
) -> Result<Vec<String>, String> {
    let start = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let mut dir = start.canonicalize().unwrap_or(start);
    loop {
        let m = dir.join("hale.toml");
        if m.exists() {
            let (envs, base) = crate::pkg::read_claims_config(&m)?;
            return match envs.get(env) {
                Some(spec) => {
                    let mut v: Vec<String> = Vec::new();
                    if let Some(b) = base {
                        v.push(b);
                    }
                    if let Some(c) = &spec.constitution {
                        if Some(c) != v.first() {
                            v.push(c.clone());
                        }
                    }
                    Ok(v)
                }
                None => Err(format!(
                    "no environment `{}` in {} (declared: {})",
                    env,
                    m.display(),
                    if envs.is_empty() {
                        "none".to_string()
                    } else {
                        envs.keys().cloned().collect::<Vec<_>>().join(", ")
                    }
                )),
            };
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => {
                return Err(format!(
                    "`--env {}` needs a `hale.toml` declaring \
                     `[environments.{}]`; none found at or above {}",
                    env,
                    env,
                    target.display()
                ))
            }
        }
    }
}

/// GH #409: check every (entrypoint, environment) pair declared in
/// `hale.toml`.
///
/// The property being enforced is "any entrypoint satisfies the
/// claimset for wherever it deploys" — universal quantification over
/// entrypoints, each still checked independently in its own closed
/// world. It composes nothing; it is the workspace sweep with a
/// constitution bound per pair.
fn run_matrix(root: &Path, verify: bool) -> ExitCode {
    let manifest_path = root.join("hale.toml");
    let (envs, base) =
        match crate::pkg::read_claims_config(&manifest_path) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::from(2);
            }
        };
    if envs.is_empty() {
        eprintln!(
            "no `[environments.<name>]` sections in {} — `--matrix` \
             checks entrypoints against the claimset each deployment \
             target requires, so it needs at least one",
            manifest_path.display()
        );
        return ExitCode::from(2);
    }

    // Every seed that declares a `main locus` is an entrypoint, and
    // every entrypoint must be accounted for. An entrypoint nobody
    // listed is silently unconstrained — the exact failure this
    // feature exists to remove — so it is an error, not a skip.
    let mut seeds = Vec::new();
    collect_seeds(root, &mut seeds);
    let mut entrypoints: Vec<PathBuf> = Vec::new();
    let mut unparseable: Vec<PathBuf> = Vec::new();
    for s in seeds {
        match seed_entry_kind(&s) {
            EntryKind::Yes => entrypoints.push(s),
            EntryKind::No => {}
            EntryKind::Unparseable(f) => unparseable.push(f),
        }
    }

    let mut bound: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for (env, spec) in &envs {
        for e in &spec.entrypoints {
            let p = root.join(e);
            bound.entry(p).or_default().push(env.clone());
        }
    }

    let mut failed: Vec<String> = Vec::new();
    let mut seen_identity: BTreeMap<String, (String, String)> =
        BTreeMap::new();
    for f in &unparseable {
        eprintln!(
            "{} does not parse, so whether it is an entrypoint is \
             unknown — and an unknown entrypoint cannot be shown to \
             be covered by any environment. Fix the syntax first",
            f.display()
        );
        failed.push(format!("{} (unparseable)", f.display()));
    }
    for e in &entrypoints {
        let canon = e.canonicalize().unwrap_or_else(|_| e.clone());
        let listed: Vec<&String> = bound
            .iter()
            .filter(|(p, _)| {
                p.canonicalize().map(|c| c == canon).unwrap_or(false)
            })
            .flat_map(|(_, v)| v.iter())
            .collect();
        if listed.is_empty() {
            eprintln!(
                "entrypoint {} is in no environment. Every entrypoint \
                 must say where it deploys — one that is listed \
                 nowhere is checked against no claimset at all",
                e.display()
            );
            failed.push(format!("{} (unbound)", e.display()));
        }
    }

    for (env, spec) in &envs {
        for ep in &spec.entrypoints {
            let target = root.join(ep);
            if !target.exists() {
                eprintln!(
                    "environment `{}` lists {}, which does not exist",
                    env,
                    target.display()
                );
                failed.push(format!("{} @ {} (missing)", ep, env));
                continue;
            }
            println!("=== {} @ {} ===", target.display(), env);
            // The base first, then the environment's own addition.
            // Every pair carries the base, so an environment can only
            // ADD law — monotonicity by construction rather than a
            // rule the manifest is trusted to respect.
            let mut adopt: Vec<String> = Vec::new();
            if let Some(b) = &base {
                adopt.push(b.clone());
            }
            if let Some(c) = &spec.constitution {
                if Some(c) != base.as_ref() {
                    adopt.push(c.clone());
                }
            }
            let code = run_check_impl_labelled(
                &target, verify, &adopt, Some(env),
            );
            if code != 0 {
                failed.push(format!("{} @ {}", ep, env));
            }
            // Review finding 3: prove the entrypoints in ONE
            // environment resolved the SAME claimset, not merely the
            // same NAME. Constitution names are flat and unmangled,
            // so two seeds can each declare `Core` with different
            // clauses and both would satisfy the binding. The digest
            // covers the normalized closure, so agreement is real.
            for (name, digest) in
                constitution_identities(&target, &adopt)
            {
                // The `[claims] base` is ONE constitution carried by
                // every environment, so it must agree workspace-wide.
                // Keying it per-environment meant two environments
                // with disjoint entrypoints never shared a key, and a
                // base resolving to different closures in dev and
                // prod went undetected — the mechanism proved
                // consistency WITHIN each environment and nothing
                // about the base being shared.
                let key = if Some(&name) == base.as_ref() {
                    format!("base::{}", name)
                } else {
                    format!("env::{}::{}", env, name)
                };
                let scope = if Some(&name) == base.as_ref() {
                    "the workspace base".to_string()
                } else {
                    format!("environment `{}`", env)
                };
                match seen_identity.get(&key) {
                    Some((prev_digest, prev_ep))
                        if *prev_digest != digest =>
                    {
                        eprintln!(
                            "{} resolves `{}` to two different \
                             claimsets: {} sees {}, {} sees {}. One \
                             name must mean one law — the entrypoints \
                             are importing different declarations \
                             that happen to share it",
                            scope, name, prev_ep, prev_digest, ep, digest
                        );
                        failed.push(format!(
                            "{} @ {} (constitution identity)",
                            ep, env
                        ));
                    }
                    Some(_) => {}
                    None => {
                        seen_identity
                            .insert(key, (digest, ep.clone()));
                    }
                }
            }
        }
    }

    println!();
    if failed.is_empty() {
        let pairs: usize =
            envs.values().map(|s| s.entrypoints.len()).sum();
        println!(
            "ok: {} (entrypoint, environment) pair(s) checked",
            pairs
        );
        return ExitCode::SUCCESS;
    }
    eprintln!("{} pair(s) failed:", failed.len());
    for f in &failed {
        eprintln!("  {}", f);
    }
    ExitCode::from(1)
}

/// The `(name, digest)` of each constitution adopted when `target`
/// is checked with `adopt`. Reads the same artifact section a
/// third party would, rather than a private side channel.
fn constitution_identities(
    target: &Path,
    adopt: &[String],
) -> Vec<(String, String)> {
    let (programs, _s, _fb, renames, _own) = match collect_checkable(target)
    {
        Ok(x) => x,
        Err(_) => return Vec::new(),
    };
    let mut programs = programs;
    for c in adopt {
        for prog in programs.values_mut() {
            inject_adopt(prog, c);
        }
    }
    let bundle_programs: BTreeMap<String, &Program> = programs
        .iter()
        .map(|(p, prog)| (p.display().to_string(), prog))
        .collect();
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames;
    let (top, _d) = hale_types::resolve::build_top_scope(&bundle);
    let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
    let progs: Vec<&Program> =
        bundle.programs.values().copied().collect();
    let ids = hale_types::claims::constitution_identities(
        &progs,
        &graph,
        &bundle.import_renames,
    );
    // ROOTS, not the whole closure: the manifest asked for these by
    // name, so these are what must agree across entrypoints. The
    // closure follows from them.
    ids.roots.into_iter().map(|i| (i.name, i.digest)).collect()
}

/// Is this seed an entrypoint? Parse-only — an entrypoint is a
/// structural fact, and a seed that fails to TYPECHECK is still an
/// entrypoint whose absence from the matrix must be reported.
///
/// A seed that fails to PARSE is `Unknown`, never `No`. Treating an
/// unparseable file as "not a main" made a syntax error erase an
/// entrypoint from coverage entirely: a broken seed listed in no
/// environment reported `ok: 1 pair(s) checked`, exit 0, while the
/// same seed made valid was correctly flagged. Breaking your file
/// became a way out of the gate — in the mechanism built to stop law
/// going missing quietly.
enum EntryKind {
    Yes,
    No,
    Unparseable(PathBuf),
}

fn seed_entry_kind(dir: &Path) -> EntryKind {
    let Ok(entries) = fs::read_dir(dir) else {
        return EntryKind::No;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"))
        .collect();
    files.sort();
    for p in files {
        let Ok(src) = fs::read_to_string(&p) else {
            return EntryKind::Unparseable(p);
        };
        match hale_syntax::parse_source(&src) {
            Ok(prog) => {
                if prog.items.iter().any(|i| {
                    matches!(i, hale_syntax::ast::TopDecl::Locus(l) if l.is_main)
                }) {
                    return EntryKind::Yes;
                }
            }
            Err(_) => return EntryKind::Unparseable(p),
        }
    }
    EntryKind::No
}

/// `--workspace`: check EVERY seed under the target, independently.
///
/// It does not connect them. Each seed is its own closed world and
/// gets its own check; this exists so that no library or main-locus
/// claim is silently skipped because nobody remembered to point
/// `check` at that directory. Cross-binary composition is a separate
/// thing entirely and is not what this does.
fn run_workspace(root: &Path, verify: bool) -> ExitCode {
    let mut seeds = Vec::new();
    collect_seeds(root, &mut seeds);
    if seeds.is_empty() {
        eprintln!(
            "no seeds under {} — a seed is a directory holding `.hl` \
             files",
            root.display()
        );
        return ExitCode::from(2);
    }

    let mut failed: Vec<(PathBuf, u8)> = Vec::new();
    for seed in &seeds {
        println!("=== {} ===", seed.display());
        let code = run_check_impl(seed, verify);
        // Every seed runs. Stopping at the first failure would make
        // the command report a subset of the truth, and the whole
        // point is that nothing is silently skipped.
        if code != 0 {
            failed.push((seed.clone(), code));
        }
    }

    println!();
    if failed.is_empty() {
        println!("ok: {} seed(s) checked", seeds.len());
        return ExitCode::SUCCESS;
    }
    eprintln!(
        "{} of {} seed(s) failed:",
        failed.len(),
        seeds.len()
    );
    for (p, code) in &failed {
        eprintln!("  {} (exit {})", p.display(), code);
    }
    // The worst code wins, so a usage error is not masked by an
    // ordinary check failure in another seed.
    ExitCode::from(failed.iter().map(|(_, c)| *c).max().unwrap_or(1))
}

/// Parse `check` / `verify` arguments: exactly one positional target,
/// only known flags, values present where required, `--help`
/// answered rather than treated as a path.
fn run_check_cli(rest: &[String], verify: bool) -> ExitCode {
    let cmd = if verify { "verify" } else { "check" };
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        check_usage(verify);
        return ExitCode::SUCCESS;
    }

    let mut positionals: Vec<&String> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if let Some(name) = a.strip_prefix("--").map(|_| a.as_str()) {
            // `--flag=value` carries its own value; `--flag value`
            // consumes the next token so it is never mistaken for
            // the target.
            let (base, has_eq) = match name.split_once('=') {
                Some((b, _)) => (b, true),
                None => (name, false),
            };
            let Some((_, takes_value)) =
                CHECK_FLAGS.iter().find(|(f, _)| *f == base)
            else {
                eprintln!("unknown flag for `hale {}`: {}", cmd, base);
                eprintln!("Run `hale {} --help` for the flag list.", cmd);
                return ExitCode::from(2);
            };
            if *takes_value && !has_eq {
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        positionals.push(a);
        i += 1;
    }

    let env_name = match flag_value_in(rest, "--env") {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };

    // GH #409: the (entrypoint x environment) matrix.
    let matrix = rest.iter().any(|a| a == "--matrix");
    if matrix {
        // A matrix is N evaluations, so a single artifact on stdout
        // or a single baseline to diff against is meaningless — two
        // concatenated artifacts are not valid JSON, and one baseline
        // compared to N models reports a failure that means nothing.
        // `--workspace` already rejects these; `--matrix` silently
        // did the wrong thing.
        for f in PER_SEED_FLAGS {
            if rest.iter().any(|a| a == f || a.starts_with(&format!("{}=", f)))
            {
                eprintln!(
                    "`{}` is per-evaluation and cannot be combined \
                     with --matrix — a matrix is many evaluations, so \
                     there is no single artifact to emit or gate \
                     against. Run it against one (entrypoint, \
                     environment) pair with `--env`.",
                    f
                );
                return ExitCode::from(2);
            }
        }
        // …and the selectors that would silently do nothing.
        for f in ["--workspace", "--env"] {
            if rest.iter().any(|a| a == f || a.starts_with(&format!("{}=", f)))
            {
                eprintln!(
                    "`{}` cannot be combined with --matrix: the matrix \
                     already enumerates every (entrypoint, \
                     environment) pair the manifest declares",
                    f
                );
                return ExitCode::from(2);
            }
        }
        let root = match positionals.len() {
            0 => std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from(".")),
            1 => PathBuf::from(positionals[0]),
            _ => {
                eprintln!("hale {} --matrix takes at most one root", cmd);
                return ExitCode::from(2);
            }
        };
        return run_matrix(&root, verify);
    }

    let workspace = rest.iter().any(|a| a == "--workspace");
    if workspace {
        // `--workspace` sweeps every seed, libraries included, and an
        // environment binds law to an ENTRYPOINT. Accepting the
        // combination and ignoring it reported "N seed(s) checked"
        // with no environment law applied — a green run the user
        // believes was gated.
        if env_name.is_some() {
            eprintln!(
                "`--env` cannot be combined with --workspace: an \
                 environment binds law to an entrypoint, and a \
                 workspace sweep checks every seed including \
                 libraries. Use `--matrix` for every (entrypoint, \
                 environment) pair, or `--env` against one entrypoint."
            );
            return ExitCode::from(2);
        }
        // Per-seed artifacts and one shared baseline are
        // incompatible by construction: N seeds produce N models, so
        // a single `--dump-topology` would interleave them on stdout
        // and a single `--check-topology` would compare N models to
        // one file. Silently taking the last would be the fail-open
        // shape this command exists to remove.
        for f in PER_SEED_FLAGS {
            if rest.iter().any(|a| a == f || a.starts_with(&format!("{}=", f)))
            {
                eprintln!(
                    "`{}` is per-seed and cannot be combined with \
                     --workspace — every seed is its own model, so \
                     there is no single artifact to emit or gate \
                     against. Run it against one seed.",
                    f
                );
                return ExitCode::from(2);
            }
        }
        let root = match positionals.len() {
            0 => std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from(".")),
            1 => PathBuf::from(positionals[0]),
            _ => {
                eprintln!(
                    "hale {} --workspace takes at most one root",
                    cmd
                );
                return ExitCode::from(2);
            }
        };
        return run_workspace(&root, verify);
    }

    match positionals.len() {
        1 => {}
        0 => {
            eprintln!("hale {} needs a target (a .hl file or a seed \
                       directory).", cmd);
            eprintln!("Run `hale {} --help` for usage.", cmd);
            return ExitCode::from(2);
        }
        _ => {
            // Silently checking only the first would be the same
            // fail-open shape as the rest of this review: the
            // command reports on less than it was handed.
            eprintln!(
                "hale {} takes ONE target, got {}: {}",
                cmd,
                positionals.len(),
                positionals
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            eprintln!(
                "A directory is one seed. Check each seed separately."
            );
            return ExitCode::from(2);
        }
    }

    // `--env X` binds the constitution `[environments.X]` requires,
    // resolved from the nearest `hale.toml` at or above the target.
    let adopt = match &env_name {
        None => Vec::new(),
        Some(e) => match resolve_env_constitution(
            &PathBuf::from(positionals[0]),
            e,
        ) {
            Ok(c) => c,
            Err(msg) => {
                eprintln!("{}", msg);
                return ExitCode::from(2);
            }
        },
    };
    ExitCode::from(run_check_impl_labelled(
        &PathBuf::from(positionals[0]),
        verify,
        &adopt,
        env_name.as_deref(),
    ))
}


/// Shared core of `hale check` (advisories print, only errors
/// fail) and `hale verify` (every finding fails — the CI
/// discipline gate; same ~10 ms analysis, no execution).
/// Returns the process exit CODE rather than an `ExitCode`, because
/// `--workspace` runs this once per seed and has to aggregate the
/// results — and `ExitCode` is opaque, so a caller cannot ask whether
/// one succeeded.
/// Add `adopt <name>;` to a program's main-locus `claims` block,
/// creating the block if the main has none. Returns whether a main
/// was found.
///
/// A duplicate is not added: an entrypoint that already writes
/// `adopt Dev;` and is also deployed to an environment requiring
/// `Dev` adopts it once, not twice.
fn inject_adopt(prog: &mut hale_syntax::ast::Program, name: &str) -> bool {
    use hale_syntax::ast::{ClaimsBlock, Ident, LocusMember, TopDecl};
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

fn run_check_impl(target: &Path, gate_warnings: bool) -> u8 {
    run_check_impl_env(target, gate_warnings, &[])
}

/// GH #409: `adopt_env` names a constitution the *deployment target*
/// requires, from `[environments.<name>]` in `hale.toml`. It is
/// injected into the main locus's `claims` block exactly as if the
/// source had written `adopt C;` — same evaluation, same closed
/// world, same union with whatever the source already adopts.
///
/// Binding it here rather than in source is what lets ONE entrypoint
/// satisfy different claimsets in different environments: it cannot
/// write two conflicting `adopt` lines, but it can be checked twice.
fn run_check_impl_env(
    target: &Path,
    gate_warnings: bool,
    adopt_env: &[String],
) -> u8 {
    run_check_impl_labelled(target, gate_warnings, adopt_env, None)
}

fn run_check_impl_labelled(
    target: &Path,
    gate_warnings: bool,
    adopt_env: &[String],
    env_label: Option<&str>,
) -> u8 {
    hale_types::claims::set_env_binding(hale_types::claims::EnvBinding {
        name: env_label.map(str::to_string),
        injected: adopt_env.to_vec(),
    });
    // `check` MUST resolve cross-seed imports the same way `build`
    // and `run` do. It used to bundle only the target's own `.hl`
    // files, so an imported seed's bodies were never in the program
    // the analysis walked — and every cross-seed call was an
    // unresolved edge. Effect assertions, budgets and taint therefore
    // stopped dead at a seed boundary while still reporting success,
    // and a cross-seed payload type rendered as `?`. Codegen resolved
    // these names all along; only the analysis phases could not see
    // them.
    let (mut programs, sources, file_bases, import_renames, own_files) =
        match collect_checkable(target) {
            Ok(x) => x,
            // GH #765: a failure that carries diagnostics renders them
            // here, honouring `--json` and resolving each span against
            // the file it lives in — including a file reached only
            // through an `import`.
            Err(f) => return f.report(),
        };

    // FUv0.8.2 #4: auto-apply sync inference before typecheck so
    // `hale check` validates the post-inference shape the build
    // path will see. Without this, `check` warns on
    // auto-inferable cross-pool calls while `build` silently
    // applies — same source, divergent answers.
    // An environment binds law to an ENTRYPOINT, so the target must
    // be one — whether or not that environment happens to contribute
    // a constitution. Checking this only while injecting meant a
    // `source_only` environment with no workspace base injected
    // nothing, checked nothing, and reported success for a library
    // path; a matrix could count that as a covered pair.
    if env_label.is_some() {
        let has_main = programs.values().any(|p| {
            p.items.iter().any(|i| {
                matches!(i, hale_syntax::ast::TopDecl::Locus(l) if l.is_main)
            })
        });
        if !has_main {
            eprintln!(
                "{}: `--env` names a deployment target, and a \
                 deployment target is an ENTRYPOINT — this seed \
                 declares no `main locus`",
                target.display()
            );
            return 2;
        }
    }
    for cname in adopt_env {
        let mut injected = false;
        for prog in programs.values_mut() {
            if inject_adopt(prog, cname) {
                injected = true;
            }
        }
        if !injected {
            eprintln!(
                "{}: no `main locus` to adopt `{}` into — an \
                 environment binds a constitution to an ENTRYPOINT, \
                 and this seed declares none",
                target.display(),
                cname
            );
            return 2;
        }
    }
    for prog in programs.values_mut() {
        // JSON Tier 2: synthesize `__json_parse_<T>` + rewrite
        // `T::from_json` before typecheck, so the generated parser is
        // checked and callers must address its `fallible(JsonError)`.
        hale_syntax::json_gen::generate_json_parsers(prog);
        // Downstream handoff (2026-08-11): the pre-pass's resolver
        // diagnostics are DISCARDED, not printed-and-bailed. They
        // are re-raised by `check_bundle` below through the normal
        // reporting path — which honours `--json`, names the file,
        // and resolves multi-file spans; the bare `render` bail here
        // did none of those (empty NDJSON on a duplicate `main`, a
        // position from the wrong file). `hale lsp` has always done
        // exactly this — it is why the LSP attributed the same
        // diagnostic correctly while the CLI did not.
        let _ = hale_types::apply_sync_inference(prog);
    }

    let bundle_programs: BTreeMap<String, &Program> = programs
        .iter()
        .map(|(p, prog)| (p.display().to_string(), prog))
        .collect();
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = import_renames.clone();
    // GH #408 Phase 0: hand the source map to the artifact, so a span
    // resolves to a file outside this process. Paths are relative to
    // the checked target (an absolute path would make the artifact
    // differ per machine, and it is meant to be comparable), with
    // forward slashes so a Windows-built artifact matches a
    // Linux-built one.
    {
        // Root at the WORKSPACE, not the target. An imported seed
        // usually lives outside the target directory (`apps/api`
        // importing `../../lib`), so relativizing to the target left
        // those paths absolute — and an artifact carrying absolute
        // paths differs per machine, which defeats the comparability
        // it exists for.
        //
        // The nearest ancestor holding a `hale.toml` is the natural
        // root: it is where a fleet plan's repo-relative paths are
        // anchored too. Failing that, the deepest common ancestor of
        // every source, which is always fully relativizing.
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
        let root = manifest_root.unwrap_or_else(|| {
            let mut common: Option<PathBuf> = None;
            for (_, p, _) in &file_bases {
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
        bundle.sources = file_bases
            .iter()
            .enumerate()
            .map(|(i, (base, path, len))| {
                // Canonicalize first: the target's own file arrives
                // as written on the command line while imported seeds
                // arrive absolute, so stripping without this left the
                // target's path relative to the CWD — and the same
                // sources checked from two directories produced two
                // different artifacts.
                let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
                let rel = abs
                    .strip_prefix(&root)
                    .unwrap_or(&abs)
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
            .collect();
    }
    // GH #18 item 1 (step 1): dump the per-method allocation summary +
    // call graph and exit. A diagnostic view of the scaffold; no
    // bound-proving yet.
    if std::env::args().any(|a| a == "--dump-alloc-summary") {
        print!("{}", hale_types::dump_alloc_summary(&bundle));
        return 0;
    }
    // GH #18 item 5: dump the per-program resource budget (pinned threads,
    // cooperative pools, bus subjects) and exit.
    // GH #265 step 7: the `.hale.effects` manifest — declared
    // contracts + inferred effect sets, stable-sorted. Emit it for
    // review, or DIFF it against a committed copy so an effect
    // regression (a handler that quietly gained a syscall) fails CI
    // the way an API break does.
    if std::env::args().any(|a| a == "--dump-effects-manifest") {
        print!("{}", hale_types::dump_effects_manifest(&bundle));
    }
    if let Some(path) = std::env::args()
        .position(|a| a == "--check-effects-manifest")
        .and_then(|i| std::env::args().nth(i + 1))
    {
        let current = hale_types::dump_effects_manifest(&bundle);
        match std::fs::read_to_string(&path) {
            Ok(expected) => {
                if expected != current {
                    eprintln!(
                        "effect manifest changed — {} no longer matches the \
                         program's effects.",
                        path
                    );
                    for line in diff_lines(&expected, &current) {
                        eprintln!("{}", line);
                    }
                    eprintln!(
                        "\nIf the change is intended, regenerate:\n  \
                         hale check <target> --dump-effects-manifest > {}",
                        path
                    );
                    return 1;
                }
            }
            Err(_) => {
                eprintln!(
                    "effect manifest baseline not found: {}\nCreate it:\n  \
                     hale check <target> --dump-effects-manifest > {}",
                    path, path
                );
                return 1;
            }
        }
    }
    if std::env::args().any(|a| a == "--dump-resource-budget") {
        print!("{}", hale_types::dump_resource_budget(&bundle));
        return 0;
    }
    // GH #382 phase 2: the topology artifact — the serialized model
    // (sorts, relations) + every named claim's result, with a
    // `shape_hash` identity over the model half. Emit for review /
    // third-party re-evaluation, or DIFF against a committed copy so
    // an unreviewed topology or law change fails CI.
    // Dump-mode must not change what `check` MEANS. Returning
    // SUCCESS here made `hale check failing.hl --dump-topology` exit
    // 0 with no diagnostics, so a CI job that added the flag to
    // collect an artifact silently stopped gating — the same file
    // without the flag exits 1 with its witness. Print the artifact,
    // then fall through to the ordinary checker so the exit status
    // and diagnostics are unchanged by observing the program.
    //
    // Flag operands accept both spellings (`--flag value` and
    // `--flag=value`). Previously `--check-topology=base.json` was
    // silently ignored and the command SUCCEEDED — the worst
    // failure mode for a CI gate, since the job looks green while
    // gating nothing. A missing operand is likewise a hard usage
    // error rather than a silent no-op.
    let argv: Vec<String> = std::env::args().collect();
    let flag_value = |flag: &str| -> Result<Option<String>, String> {
        if let Some(eq) = argv.iter().find(|a| {
            a.starts_with(flag) && a.as_bytes().get(flag.len()) == Some(&b'=')
        }) {
            let v = &eq[flag.len() + 1..];
            if v.is_empty() {
                return Err(format!("{}= requires a path", flag));
            }
            return Ok(Some(v.to_string()));
        }
        if let Some(i) = argv.iter().position(|a| a == flag) {
            return match argv.get(i + 1) {
                Some(v) if !v.starts_with('-') => Ok(Some(v.clone())),
                _ => Err(format!(
                    "{} requires a path. Use `{} <path>` or \
                     `{}=<path>`.",
                    flag, flag, flag
                )),
            };
        }
        Ok(None)
    };

    let dump_topology = argv.iter().any(|a| a == "--dump-topology");
    // `=<path>` ONLY, never "consume the next token". Sharing
    // `flag_value` here was destructive: it took the following
    // argument as the destination, so `hale check --dump-topology
    // app.hl` — a flag order this command now accepts — OVERWROTE
    // `app.hl` with the artifact. Losing the file you asked it to
    // inspect is the worst possible reading of an ambiguous
    // argument, and the ambiguity is unresolvable in general
    // because the value is optional. A bare `--dump-topology`
    // writes to stdout.
    let dump_topology_to = argv
        .iter()
        .find_map(|a| a.strip_prefix("--dump-topology="))
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    // One analysis pass, shared by the artifact gate below and the
    // diagnostic report further down. `check_bundle_opts` is the
    // expensive part of `check`, and nothing mutates `bundle`
    // between the two, so running it twice would just double the
    // cost of every `--dump-topology` invocation.
    let allow_unowned =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    // F.18: a whole seed (a directory) is checked to what `build`
    // accepts — a call to a bare name nothing binds is an error here;
    // one file of a seed keeps the permissive reading for a sibling's fn
    //
    // GH #721: a bare IDENTIFIER nothing binds follows the same line.
    // One file of a multi-file seed reads consts its siblings declare,
    // so the leniency is load-bearing there and only there.
    let whole_seed = target.is_dir();
    let checked = hale_types::check_bundle_opts_scoped(
        &bundle,
        allow_unowned,
        whole_seed,
        whole_seed,
    );

    if dump_topology || dump_topology_to.is_some() {
        // The artifact's EXISTENCE means the model is sound.
        //
        // A program that fails to typecheck still produced a full
        // artifact — populated relations, and claims evaluated over a
        // graph derived from source the compiler could not
        // understand. A claim would report `"result": "holds"` for a
        // program that cannot compile: a certificate asserting a
        // property of something that will never run. Worse for a
        // consumer than no artifact, because an admission step
        // looking for "no violated claims" passes it.
        //
        // A VIOLATED claim is the opposite case and still emits: the
        // model is well-defined, the row is a truthful report, and
        // being able to replay a violation independently is the point
        // of publishing the model at all. `DiagKind::Claim` is what
        // separates the two.
        if let Some(d) = checked
            .iter()
            .find(|d| d.is_error() && d.kind != hale_syntax::error::DiagKind::Claim)
        {
            eprintln!(
                "refusing to emit a topology artifact: `{}` does not \
                 typecheck, so its model is not a truthful \
                 description of any program. Fix the {} first.",
                target.display(),
                d.kind_str()
            );
            return 1;
        }
        let artifact = hale_types::topology::dump_topology(&bundle);
        match &dump_topology_to {
            Some(path) => {
                if let Err(e) = std::fs::write(path, &artifact) {
                    eprintln!("could not write {}: {}", path, e);
                    return 2;
                }
            }
            None => print!("{}", artifact),
        }
    }
    // GH #476 Change 2: the canonical-model demand surface. Same
    // refusal rule as the artifact — a model of a program that does
    // not typecheck describes nothing.
    if argv.iter().any(|a| a == "--dump-model")
        || std::env::var("HALE_DUMP_MODEL").as_deref() == Ok("1")
    {
        if let Some(d) = checked.iter().find(|d| {
            d.is_error()
                && d.kind != hale_syntax::error::DiagKind::Claim
        }) {
            eprintln!(
                "refusing to derive a model: `{}` does not typecheck,                  so its model is not a truthful description of any                  program. Fix the {} first.",
                target.display(),
                d.kind_str()
            );
            return 1;
        }
        let model =
            hale_types::model_builder::derive_application_model(&bundle);
        if let Err(e) = model.validate() {
            // A builder bug, never user error: the derivation
            // produced a value that is not a model. Loud, named, and
            // fatal — an invalid model must not print as one.
            eprintln!(
                "internal error: derived model violates a model law:                  {:?} (this is a hale bug — please report it)",
                e
            );
            return 2;
        }
        print!(
            "{}",
            hale_types::model_builder::render_internal(&model)
        );
    }
    // P2 from the devex review: `--check-topology` compares the
    // ENTIRE artifact text, so a claim rename or a comment-only edit
    // that moves every provenance offset fails the gate and reports
    // that the program's "model" changed — even when `shape_hash`,
    // which is the model's identity, did not. Both gates now exist
    // and are named for what they compare:
    //   --check-topology        exact artifact snapshot (law +
    //                           model + provenance)
    //   --check-topology-shape  the model identity only
    // `shape_hash` was already verified stable across renames and
    // source motion, and sensitive to a real graph change, so it is
    // the right key for the loose gate.
    let shape_gate = match flag_value("--check-topology-shape") {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{}", msg);
            return 2;
        }
    };
    if let Some(path) = shape_gate {
        let current = hale_types::topology::dump_topology(&bundle);
        // The hash VALUE, not the raw line — the gate's whole point
        // is that this is the model's identity, and a diagnostic
        // that makes you read past `"shape_hash": ` and a trailing
        // comma to compare two hex strings is working against that.
        let hash_of = |s: &str| -> Option<String> {
            s.lines()
                .find(|l| l.contains("\"shape_hash\""))
                .and_then(|l| l.split(':').nth(1))
                .map(|v| v.trim().trim_matches(',').trim_matches('"').to_string())
        };
        match std::fs::read_to_string(&path) {
            Ok(expected) if matches!(
                hale_types::topology::verify_artifact_digest(&expected),
                Some(false)
            ) =>
            {
                // This gate greps ONE line out of the baseline, so a
                // baseline whose `shape_hash` line was edited to match
                // would pass while the model it claims to describe
                // says otherwise. The whole-body digest is what makes
                // the grepped line trustworthy; a baseline that fails
                // it is not a mismatch to report, it is a file that
                // cannot be reasoned about.
                eprintln!(
                    "topology baseline {} is corrupt: its \
                     `artifact_digest` does not match its contents. \
                     Regenerate it:\n  hale check <target> \
                     --dump-topology > {}",
                    path, path
                );
                return 2;
            }
            Ok(expected) => match (hash_of(&expected), hash_of(&current)) {
                (Some(a), Some(b)) if a != b => {
                    eprintln!(
                        "topology SHAPE changed — the program's \
                         model no longer matches {}.\n  baseline: \
                         {}\n  current:  {}\n\nClaim renames and \
                         source motion do NOT affect this gate; a \
                         changed graph does. Regenerate:\n  hale \
                         check <target> --dump-topology > {}",
                        path, a, b, path
                    );
                    return 1;
                }
                (None, _) | (_, None) => {
                    eprintln!(
                        "topology baseline {} has no shape_hash line",
                        path
                    );
                    return 2;
                }
                _ => {}
            },
            Err(_) => {
                eprintln!(
                    "topology baseline not found: {}\nCreate \
                     it:\n  hale check <target> --dump-topology > {}",
                    path, path
                );
                return 1;
            }
        }
    }
    let check_topology_path = match flag_value("--check-topology") {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{}", msg);
            return 2;
        }
    };
    if let Some(path) = check_topology_path {
        let current = hale_types::topology::dump_topology(&bundle);
        match std::fs::read_to_string(&path) {
            Ok(expected) => {
                if expected != current {
                    eprintln!(
                        "topology artifact changed — {} no longer matches \
                         byte-for-byte. This gate covers law, model AND \
                         provenance, so a claim rename or source motion \
                         trips it even when the model is identical; use \
                         --check-topology-shape to gate the model alone.",
                        path
                    );
                    for line in diff_lines(&expected, &current) {
                        eprintln!("{}", line);
                    }
                    eprintln!(
                        "\nIf the change is intended, regenerate:\n  \
                         hale check <target> --dump-topology > {}",
                        path
                    );
                    return 1;
                }
            }
            Err(_) => {
                eprintln!(
                    "topology baseline not found: {}\nCreate it:\n  \
                     hale check <target> --dump-topology > {}",
                    path, path
                );
                return 1;
            }
        }
    }
    // GH #18 item 5: the CI gate. `--check-resource-budget <path>` reads a
    // TOML ceiling file and fails the build if any count exceeds it.
    {
        let cli_args: Vec<String> = std::env::args().collect();
        let ceiling_path = cli_args
            .iter()
            .position(|a| a == "--check-resource-budget")
            .and_then(|i| cli_args.get(i + 1));
        if let Some(path) = ceiling_path {
            #[derive(serde::Deserialize, Default)]
            #[serde(deny_unknown_fields)]
            struct CeilingToml {
                pinned_threads: Option<usize>,
                cooperative_pools: Option<usize>,
                bus_subjects: Option<usize>,
                fd_open_sites: Option<usize>,
            }
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("--check-resource-budget: cannot read `{}`: {}", path, e);
                    return 1;
                }
            };
            let ct: CeilingToml = match toml::from_str(&text) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("--check-resource-budget: invalid budget file `{}`: {}", path, e);
                    return 1;
                }
            };
            let ceiling = hale_types::resource_budget::ResourceCeiling {
                pinned_threads: ct.pinned_threads,
                cooperative_pools: ct.cooperative_pools,
                bus_subjects: ct.bus_subjects,
                fd_open_sites: ct.fd_open_sites,
            };
            let violations = hale_types::check_resource_ceiling(&bundle, &ceiling);
            if violations.is_empty() {
                println!("resource budget OK (within `{}`)", path);
                return 0;
            }
            for v in &violations {
                eprintln!(
                    "resource budget exceeded: {} — raise the ceiling in `{}` if intentional",
                    v, path
                );
            }
            return 1;
        }
    }
    let mut diags = checked;
    // Advisories about code the target does not own are dropped.
    //
    // `check` resolving imports is what makes cross-seed ERRORS
    // visible, and that is the point — a soundness violation reached
    // through an import is still your violation. But the same change
    // drags every advisory lint in every imported seed into the
    // target's output: checking one downstream app began reporting 47
    // hot-path warnings from `lib/` and `pond/`, and since `hale
    // verify` gates on ANY finding, 10 of 12 apps that passed it
    // started failing. A gate that goes red for library internals
    // you cannot edit from here is a gate people switch off.
    //
    // Nothing is lost: an advisory about a seed is reported when that
    // seed is checked, which is how a multi-seed project is checked
    // anyway — a real multi-seed project checks every seed directly).
    // Errors are NEVER filtered, wherever they originate.

    // GH #18 item 1 → M3 stage 5 (2026-07-02): unbounded-allocation
    // warnings are DEFAULT-ON (Riley's flip call after the 402-warning
    // audit: every audited true positive preserved, every residual FP
    // in a documented accepted class — see
    // notes/unbounded-alloc-audit-2026-07-02.md). The analysis itself
    // spares run-to-exit programs (a `main` with no run loop and no
    // bus handler warns nothing), so scripts still owe nothing.
    //
    // Surfaces:
    //  - default: the whole-program survey, every site.
    //  - `--no-warn-unbounded-alloc` — the opt-OUT.
    //  - `--warn-unbounded-alloc` — accepted-and-ignored (former
    //    opt-in spelling).
    //  - `@unbounded fn` carves a fn out; `@bounded locus` is now
    //    redundant with the default but still accepted.
    // Warnings print but never fail the build (only errors do).
    let survey_all =
        !std::env::args().any(|a| a == "--no-warn-unbounded-alloc");
    diags.extend(hale_types::unbounded_alloc_warnings(&bundle, survey_all));
    // GH #18 item 5: opt-in fd-resource-leak warnings.
    if std::env::args().any(|a| a == "--warn-resource-leak") {
        diags.extend(hale_types::resource_leak_warnings(&bundle));
    }
    // GH #436: opt-in fail-closed `@secret` containment. The default
    // `@secret` pass is a LINT (warnings, narrow traversal). This one
    // walks every branch, propagates aliases, and reports
    // `uncertified` for anything it cannot follow — it newly fails
    // programs that compile today, which is why it is a flag and not
    // the default. See spec/verification.md § "Secrets".
    // GH #436: which loci could be `@sealed` today, and what it would
    // cost. `@sealed` is opt-in, so adopting it across an existing
    // codebase is otherwise a question you can only answer by reading.
    if std::env::args().any(|a| a == "--sealable") {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        let rows = hale_types::sealability::survey(&progs);
        eprint!("{}", hale_types::sealability::render(&rows));
    }
    if std::env::args().any(|a| a == "--strict-secret") {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        diags.extend(hale_types::frontier::secret_taint_strict(&progs));
    }
    // #8 LSP groundwork (2026-07-02): `hale check --json` emits
    // NDJSON diagnostics on STDOUT (one object per line: file,
    // line, col, severity, kind, message) for editor/LSP
    // consumption. The human rendering stays on stderr otherwise.
    // With `hale check` at ~10 ms on the largest apps, an
    // on-save/on-keystroke loop needs nothing more than this.
    let json_mode = std::env::args().any(|a| a == "--json");
    // Every diagnostic renders in the spelling the author wrote.
    // Effect witnesses were demangled at their source, but any other
    // check that names a type or method — the no-locus-return rule,
    // for one — still emitted `__lib_lib_a_b_OrderBook.query_bulk`,
    // a symbol that appears nowhere in their program. Doing it once
    // here covers every pass rather than each remembering.
    hale_types::stdlib_bodies::demangle_imports(&mut diags, &import_renames);
    retain_owned_advisories(&mut diags, &own_files, &file_bases);
    if !diags.is_empty() {
        for d in &diags {
            if json_mode {
                println!("{}", render_diag_json(d, &file_bases, &sources));
            } else {
                eprintln!("{}", render_located(d, &file_bases, &sources));
            }
        }
        // check: warnings print but don't fail; only errors do.
        // verify: everything gates.
        if gate_warnings {
            if !json_mode {
                eprintln!(
                    "verify: {} finding(s) — the discipline gate \
                     fails on advisories too",
                    diags.len()
                );
            }
            return 1;
        }
        if diags.iter().any(|d| d.is_error()) {
            return 1;
        }
    }
    if !json_mode {
        // Count the target's own files, not `programs` entries — a
        // multi-file seed merges into one program before checking.
        let n_files = own_files.len().max(programs.len());
        if gate_warnings {
            eprintln!("verified: {} file(s), 0 findings", n_files);
        } else {
            eprintln!("ok: {} file(s) typechecked", n_files);
        }
    }
    0
}

/// The one NDJSON record writer for `hale check --json` and
/// `hale verify --json`.
///
/// Both producers format here: a `Diag` through [`render_diag_json`],
/// which resolves its position against the file windows first, and an
/// unreadable input through [`IoDiag::record`] (GH #806), which has
/// no position and passes `0, 0`. One writer is the point — the
/// record shape is a consumed contract (`spec/projects.md`, the
/// `hale check --json` row), and a second `format!` of it somewhere
/// else is how the two drift.
///
/// `related` is already-formatted JSON, `,"related":[…]` or empty.
fn render_json_record(
    file: &str,
    line: usize,
    col: usize,
    severity: &str,
    kind: &str,
    message: &str,
    related: &str,
) -> String {
    format!(
        "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"severity\":\"{}\",\"kind\":\"{}\",\"message\":\"{}\"{}}}",
        json_escape(file),
        line,
        col,
        severity,
        json_escape(kind),
        json_escape(message),
        related
    )
}

/// One NDJSON diagnostic line for `hale check --json`.
fn render_diag_json(
    d: &hale_syntax::Diag,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    fn esc(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 8);
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32))
                }
                c => out.push(c),
            }
        }
        out
    }
    let off = d.span.start.as_usize() as u32;
    let mut file = String::new();
    let mut line = 0usize;
    let mut col = 0usize;
    for (base, path, len) in file_bases {
        if file_owns_offset(*base, *len, off) {
            if let Some(src) = sources.get(path) {
                let (l, c) = d
                    .span
                    .shifted(base.wrapping_neg())
                    .line_col(src);
                file = path.display().to_string();
                line = l;
                col = c;
            }
            break;
        }
    }
    let severity = if d.is_error() { "error" } else { "warning" };
    // Secondary locations ride along as a `related` array (absent
    // when empty, so existing consumers see an unchanged shape).
    let related = if d.related.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = d
            .related
            .iter()
            .filter_map(|(rspan, label)| {
                let (rf, rl, rc) =
                    locate_span(*rspan, file_bases, sources)?;
                Some(format!(
                    "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"note\":\"{}\"}}",
                    esc(&rf),
                    rl,
                    rc,
                    esc(label)
                ))
            })
            .collect();
        if entries.is_empty() {
            String::new()
        } else {
            format!(",\"related\":[{}]", entries.join(","))
        }
    };
    render_json_record(
        &file,
        line,
        col,
        severity,
        d.kind_str(),
        &d.message,
        &related,
    )
}

/// Compile `program` to a temporary native binary and execute it,
/// forwarding `user_args` as the program's trailing argv. This is
/// the whole of `hale run` — the same codegen backend as `hale
/// build`, so there is no `run`-vs-`build` behavioral divergence.
fn compile_and_exec(
    program: &Program,
    renames: &[(Vec<String>, String)],
    user_args: &[String],
    model_hash: u64,
    exec_digest: [u64; 4],
    obs_entity_ids: Vec<hale_model::obs_ids::ObsEntityId>,
) -> ExitCode {
    let mut bin = std::env::temp_dir();
    let mut h = DefaultHasher::new();
    h.write_usize(program.items.len());
    h.write_u32(std::process::id());
    bin.push(format!("hale_run_{:016x}", h.finish()));
    let options = hale_codegen::BuildOptions {
        model_hash: Some(model_hash),
        exec_digest: Some(exec_digest),
        obs_entity_ids,
        ..Default::default()
    };
    if let Err(e) = hale_codegen::build_executable_with_options(
        program, &bin, renames, &options,
    ) {
        eprintln!("build error: {:?}", e);
        return ExitCode::from(1);
    }
    let status = std::process::Command::new(&bin).args(user_args).status();
    let _ = std::fs::remove_file(&bin);
    match status {
        Ok(s) => {
            // GH #577: a program killed by a signal says so — a segfault
            // that printed nothing used to look like `exit 1`
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = s.signal() {
                let name = match sig {
                    libc::SIGSEGV => "SIGSEGV",
                    libc::SIGABRT => "SIGABRT",
                    libc::SIGBUS => "SIGBUS",
                    libc::SIGFPE => "SIGFPE",
                    libc::SIGILL => "SIGILL",
                    libc::SIGKILL => "SIGKILL",
                    libc::SIGTERM => "SIGTERM",
                    _ => "signal",
                };
                eprintln!("hale run: the program was killed by {name} (signal {sig})");
                return ExitCode::from((128 + sig).clamp(0, 255) as u8);
            }
            ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8)
        }
        Err(e) => {
            eprintln!("could not execute compiled program: {}", e);
            ExitCode::from(1)
        }
    }
}

/// GH #296: build-manifest identity — a FRAMED SHA-256 over the
/// build inputs this binary can see:
///
///   - the toolchain source hash (`HALE_CODEGEN_SRC_HASH`, computed
///     by build.rs over the codegen + runtime + stdlib sources this
///     CLI was linked against — so two different compiler/runtime
///     commits under one version differ);
///   - the CLI crate version;
///   - the build options that alter emitted code;
///   - every source file's FULL normalized path, byte length, and
///     contents, each length-framed (no concatenation ambiguity).
///
/// Structural `shape_hash` says "same model"; this says "same build
/// inputs". Residue it cannot see: the LLVM/libc toolchain outside
/// this binary and the linker environment — a post-link binary
/// digest is the staged stronger form.
/// GH #476 Change 8: everything the BUILD needs from the canonical
/// model, from ONE derivation — the dispatch plan's digest (folded
/// into the execution identity below) and the canonical entity ids
/// codegen stamps into the observation manifest.
///
/// `LOTUS_NO_BUS_DEVIRT=1` (the differential harness's control arm)
/// makes codegen emit the empty plan — every subject dynamic — so
/// the identity folded into the exec digest must be the EMPTY
/// plan's, not the model's. Otherwise the control arm and the live
/// arm would share a build identity while running different
/// lowerings, and a recording taken under one would be admitted
/// against the other.
/// The build-options half of the execution identity. One spelling,
/// so `hale build` and `hale run` fingerprint the same options the
/// same way (they did not: the build path never computed a digest
/// at all — GH #476 Change 8 review).
fn options_fingerprint(o: &hale_codegen::BuildOptions) -> String {
    format!(
        "target={:?};cpu={:?};dev={};debug={}",
        o.target,
        o.target_cpu,
        o.dev_profile,
        o.debug.is_some()
    )
}

fn model_identity(
    bundle: &hale_types::Bundle<'_>,
) -> (u64, Vec<hale_model::obs_ids::ObsEntityId>) {
    let model = hale_types::model_builder::derive_application_model(bundle);
    let plan_digest = if std::env::var("LOTUS_NO_BUS_DEVIRT")
        .map(|v| v == "1" || v == "true" || v == "TRUE")
        .unwrap_or(false)
    {
        hale_model::dispatch_plan::DispatchPlan::default().digest()
    } else {
        hale_model::dispatch_plan::DispatchPlan::derive(&model).digest()
    };
    (plan_digest, hale_model::obs_ids::obs_entity_ids(&model))
}

fn exec_digest(
    sources: &BTreeMap<PathBuf, String>,
    entry: &Path,
    options_fp: &str,
    plan_digest: u64,
) -> [u64; 4] {
    let mut buf: Vec<u8> = Vec::new();
    let frame = |b: &[u8], buf: &mut Vec<u8>| {
        buf.extend_from_slice(&(b.len() as u64).to_le_bytes());
        buf.extend_from_slice(b);
    };
    // Round 3: the toolchain half is the FULL workspace-source
    // SHA-256 (parser, analyses, codegen, every runtime TU incl.
    // lotus_obs.c, stdlib seeds, the replay CLI) + rustc version +
    // git commit — computed by build.rs. The old 64-bit stale-CLI
    // hash covered three files and missed the record/replay
    // implementation entirely.
    frame(env!("HALE_TOOLCHAIN_SHA256").as_bytes(), &mut buf);
    frame(env!("CARGO_PKG_VERSION").as_bytes(), &mut buf);
    frame(options_fp.as_bytes(), &mut buf);
    // GH #476 Change 8: the DISPATCH PLAN is part of what a build
    // is. Two builds of byte-identical sources by one toolchain
    // still run different code if the bus lowering differs (an
    // env-forced all-dynamic plan, a future placement-driven
    // flavor), and a recording admitted across that boundary would
    // replay against a program whose bus behaves differently. The
    // plan's digest is framed here so the boundary is a refusal.
    frame(&plan_digest.to_le_bytes(), &mut buf);
    buf.extend_from_slice(&(sources.len() as u64).to_le_bytes());
    // Logical (entry-relative) source ids: identical trees checked
    // out under different roots are the same build inputs.
    let base = entry.parent().map(Path::to_path_buf);
    for (path, src) in sources {
        let logical = base
            .as_deref()
            .and_then(|b| path.strip_prefix(b).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        frame(logical.as_bytes(), &mut buf);
        frame(src.as_bytes(), &mut buf);
    }
    let d = openssl::sha::sha256(&buf);
    let mut out = [0u64; 4];
    for (i, part) in out.iter_mut().enumerate() {
        *part = u64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().unwrap());
    }
    if out == [0; 4] {
        out[0] = 1;
    }
    out
}

/// Escape a string for embedding in a JSON string literal.
/// Shared by `hale test --json` (mirrors the private `esc` inside
/// `render_diag_json`).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Verdict for one `*_test.hl` file.
struct TestOutcome {
    file: PathBuf,
    passed: bool,
    /// Failure detail: the captured `ASSERTION FAILED …` lines, the
    /// nonzero-exit note, or the compile diagnostic. `None` on pass.
    message: Option<String>,
    elapsed_ms: u128,
}

/// Recursively collect `*_test.hl` files under `target`. A file
/// target is taken as-is (an explicitly-named file runs regardless
/// of suffix — the user asked for it); a directory is walked
/// depth-first, gathering only names ending in `_test.hl`. Entries
/// are visited in sorted order at every level for determinism.
fn collect_test_files(target: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if target.is_file() {
        out.push(target.to_path_buf());
        return Ok(());
    }
    if target.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(target)
            .map_err(|e| format!("{}: {}", target.display(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                // `vendor/` and dot-directories are skipped (spec/testing.md):
                // a toolchain-managed tree, and the DNA's `.hale/` — its
                // worktrees carry the application's own tests (GH #529 D7)
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name == "vendor" || name.starts_with('.') {
                    continue;
                }
                collect_test_files(&p, out)?;
            } else if p
                .file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.ends_with("_test.hl"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
        return Ok(());
    }
    Err(format!("not a file or directory: {}", target.display()))
}

/// Compile one test file to a temporary native binary, returning
/// its path on success or a rendered diagnostic string on failure.
/// A compile/typecheck error is a test failure — it comes back as
/// the `Err` message. Mirrors `run_program`'s single-file pipeline
/// (parse_with_imports → check_bundle_opts → build) but stops at
/// the binary so the caller can `.output()`-capture the run.
fn compile_test_binary(entry: &Path) -> Result<PathBuf, String> {
    let (program, renames, sources, file_bases, ctx) = match parse_with_imports(entry) {
        Ok(x) => x,
        Err(errors) => {
            let mut msg = String::new();
            for e in &errors {
                msg.push_str(&e.render());
                msg.push('\n');
            }
            return Err(msg.trim_end().to_string());
        }
    };
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(entry.display().to_string(), &program);
    // The rename table must reach the analysis here too, not only in
    // `check`. Without it a cross-seed call is an unresolved edge, so
    // an effect assertion violated one seed away compiles, links and
    // ships — a downstream fleet gates on `build` across 109 binaries,
    // and "it built" must not be weaker than "it checked" on a
    // contract the compiler already knows how to evaluate.
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    let diags = hale_types::check_bundle_opts_whole_program(&bundle, false);
    if diags.iter().any(|d| d.is_error()) {
        let mut msg = String::new();
        for d in diags.iter().filter(|d| d.is_error()) {
            msg.push_str(&render_located(d, &file_bases, &sources));
            msg.push('\n');
        }
        return Err(msg.trim_end().to_string());
    }
    let mut bin = std::env::temp_dir();
    let mut h = DefaultHasher::new();
    h.write(entry.display().to_string().as_bytes());
    h.write_u32(std::process::id());
    bin.push(format!("hale_test_{:016x}", h.finish()));
    // Stage-2 FFI pickup, same as `hale build` (2026-07-18; closes
    // pond FRICTION "hale test cannot link @ffi libs"): a test that
    // imports an FFI-bearing lib (sqlite et al.) needs the lib's
    // hale.toml [ffi] link/csrc surface on the link line, or every
    // such test dies with undefined lotus_* references regardless
    // of the test's own correctness.
    let mut options = collect_ffi_from_imports(
        &ctx.imports,
        &ctx.entry_dir,
        ctx.workspace_root.as_deref(),
    );
    // Tests are rebuilt every run — take the dev profile's build
    // latency win; the exit-code contract doesn't time anything.
    options.dev_profile = true;
    if let Err(e) = hale_codegen::build_executable_with_options(
        &program, &bin, &renames, &options,
    ) {
        return Err(format!("codegen error: {:?}", e));
    }
    Ok(bin)
}

/// `hale test [file | dir] [-run <substr>] [--json]`.
///
/// Discovers `*_test.hl` files, compiles+runs each as an ordinary
/// Hale binary, and reports per the `spec/testing.md` exit-code
/// contract: PASS iff the process exits 0 with empty stdout; any
/// other outcome (nonzero exit, stdout, or a compile error) is a
/// FAIL. Exits SUCCESS when every test passes, `1` when any fails.
fn run_test(args: &[String]) -> ExitCode {
    let mut target: Option<PathBuf> = None;
    let mut run_filter: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        // Accept the spec's single-dash `-run` and the CLI's
        // `--`-convention `--run`, in both space- and `=`-separated
        // forms.
        if a == "-run" || a == "--run" {
            match args.get(i + 1) {
                Some(v) => {
                    run_filter = Some(v.clone());
                    i += 2;
                }
                None => {
                    eprintln!("hale test: {} requires a substring argument", a);
                    return ExitCode::from(2);
                }
            }
        } else if let Some(v) = a.strip_prefix("-run=").or_else(|| a.strip_prefix("--run=")) {
            run_filter = Some(v.to_string());
            i += 1;
        } else if a == "--json" {
            json = true;
            i += 1;
        } else if a.starts_with('-') {
            eprintln!("hale test: unknown flag `{}`", a);
            return ExitCode::from(2);
        } else if target.is_none() {
            target = Some(PathBuf::from(a));
            i += 1;
        } else {
            eprintln!("hale test: unexpected extra argument `{}`", a);
            return ExitCode::from(2);
        }
    }
    let target = target
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut files: Vec<PathBuf> = Vec::new();
    if let Err(e) = collect_test_files(&target, &mut files) {
        eprintln!("hale test: {}", e);
        return ExitCode::from(2);
    }
    files.sort();
    files.dedup();
    if let Some(sub) = &run_filter {
        files.retain(|f| f.to_string_lossy().contains(sub.as_str()));
    }

    if files.is_empty() {
        if json {
            println!("[]");
        } else if let Some(sub) = &run_filter {
            println!(
                "no `_test.hl` files matching `{}` under {}",
                sub,
                target.display()
            );
        } else {
            println!("no `_test.hl` files found under {}", target.display());
        }
        // Nothing to run is not an error.
        return ExitCode::SUCCESS;
    }

    let mut outcomes: Vec<TestOutcome> = Vec::with_capacity(files.len());
    for f in &files {
        let start = std::time::Instant::now();
        let (passed, message) = match compile_test_binary(f) {
            Err(diag) => (false, Some(diag)),
            Ok(bin) => {
                let output = std::process::Command::new(&bin).output();
                let _ = std::fs::remove_file(&bin);
                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        // spec/testing.md: pass = exit 0 AND empty stdout.
                        if out.status.success() && out.stdout.is_empty() {
                            (true, None)
                        } else {
                            let mut m = String::new();
                            let body = stdout.trim_end();
                            if !body.is_empty() {
                                m.push_str(body);
                            }
                            if !out.status.success() {
                                if !m.is_empty() {
                                    m.push('\n');
                                }
                                match out.status.code() {
                                    Some(c) => {
                                        m.push_str(&format!("(exited with code {})", c))
                                    }
                                    None => m.push_str("(terminated by signal)"),
                                }
                            } else if !body.is_empty() {
                                // Exit 0 but produced output — a passing
                                // test must be silent (spec contract).
                                m = format!(
                                    "test exited 0 but produced stdout \
                                     (a passing test must be silent):\n{}",
                                    body
                                );
                            }
                            (false, Some(m))
                        }
                    }
                    Err(e) => {
                        (false, Some(format!("could not execute compiled test: {}", e)))
                    }
                }
            }
        };
        outcomes.push(TestOutcome {
            file: f.clone(),
            passed,
            message,
            elapsed_ms: start.elapsed().as_millis(),
        });
    }

    let passed = outcomes.iter().filter(|o| o.passed).count();
    let failed = outcomes.len() - passed;

    if json {
        let mut buf = String::from("[");
        for (idx, o) in outcomes.iter().enumerate() {
            if idx > 0 {
                buf.push(',');
            }
            buf.push_str(&format!(
                "{{\"file\":\"{}\",\"status\":\"{}\"",
                json_escape(&o.file.display().to_string()),
                if o.passed { "pass" } else { "fail" }
            ));
            if let Some(m) = &o.message {
                buf.push_str(&format!(",\"message\":\"{}\"", json_escape(m)));
            }
            buf.push_str(&format!(",\"elapsed_ms\":{}}}", o.elapsed_ms));
        }
        buf.push(']');
        println!("{}", buf);
    } else {
        for o in &outcomes {
            if o.passed {
                println!("ok   {}", o.file.display());
            } else {
                println!("FAIL {}", o.file.display());
                if let Some(m) = &o.message {
                    for line in m.lines() {
                        println!("     {}", line);
                    }
                }
            }
        }
        println!();
        println!("{} passed, {} failed", passed, failed);
    }

    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `hale replay <recording> <program.hl> [--diff [--json]] [--at N]`
/// — GH #296. Re-runs a recorded execution: the same binary (model
/// identity checked against the recording header), with the
/// runtime serving journaled inputs (time/entropy/env) and
/// enforcing each consumer's recorded delivery order. `--diff`
/// records the replay and reports the first divergence, or — on a
/// match — which categories it compared (GH #728), with `--json`
/// carrying the same verdict and counts machine-readably. `--at N`
/// stops the program (SIGSTOP) at the Nth consume so a debugger
/// can attach.
fn run_replay(args: &[String]) -> ExitCode {
    let mut rec_arg: Option<PathBuf> = None;
    let mut prog: Option<PathBuf> = None;
    let mut diff = false;
    let mut json = false;
    let mut at: Option<String> = None;
    let mut allow_live_effects = false;
    let mut allow_unverified = false;
    let mut allow_truncated = false;
    let mut feed = false;
    let mut allow_unmatched_feed = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--diff" {
            diff = true;
            i += 1;
        } else if a == "--json" {
            // GH #728: the `--diff` verdict, machine-readable —
            // per-category coverage, not just a match/diverge bit.
            json = true;
            i += 1;
        } else if a == "--allow-live-effects" {
            allow_live_effects = true;
            i += 1;
        } else if a == "--allow-unverified-model" {
            allow_unverified = true;
            i += 1;
        } else if a == "--allow-truncated" {
            allow_truncated = true;
            i += 1;
        } else if a == "--feed" {
            feed = true;
            i += 1;
        } else if a == "--allow-unmatched-feed" {
            allow_unmatched_feed = true;
            i += 1;
        } else if a == "--at" {
            // N (process-wide consume ordinal — only meaningful
            // for a single consumer) or consumer:N (stable across
            // multi-consumer runs).
            match args.get(i + 1) {
                Some(v)
                    if v.parse::<u64>().map(|n| n > 0).unwrap_or(false)
                        || v.split_once(':').is_some_and(|(c, n)| {
                            c.parse::<u64>().is_ok()
                                && n.parse::<u64>()
                                    .map(|n| n > 0)
                                    .unwrap_or(false)
                        }) =>
                {
                    at = Some(v.clone());
                    i += 2;
                }
                _ => {
                    eprintln!(
                        "hale replay: --at takes N or consumer:N                          (positive)"
                    );
                    return ExitCode::from(2);
                }
            }
        } else if a.starts_with('-') {
            eprintln!("hale replay: unknown flag `{}`", a);
            return ExitCode::from(2);
        } else if rec_arg.is_none() {
            rec_arg = Some(PathBuf::from(a));
            i += 1;
        } else if prog.is_none() {
            prog = Some(PathBuf::from(a));
            i += 1;
        } else {
            eprintln!("hale replay: unexpected extra argument `{}`", a);
            return ExitCode::from(2);
        }
    }
    if feed && (diff || at.is_some()) {
        eprintln!(
            "hale replay: --feed re-executes changed code against \
             the recorded ingress tape — there is no recorded \
             schedule to --diff against or --at-stop on"
        );
        return ExitCode::from(2);
    }
    if json && !diff {
        eprintln!(
            "hale replay: --json reports the --diff comparison \
             verdict; pass --diff"
        );
        return ExitCode::from(2);
    }
    let (rec_path, prog) = match (rec_arg, prog) {
        (Some(r), Some(p)) => (r, p),
        _ => {
            eprintln!(
                "usage: hale replay <recording> <program.hl> \
                 [--diff [--json]] \
                 [--at <n> | --at <consumer-id>:<ordinal>] \
                 [--allow-live-effects] [--allow-unverified-model] \
                 [--allow-truncated] [--feed] [--allow-unmatched-feed]"
            );
            return ExitCode::from(2);
        }
    };

    // ONE file object for admission AND execution (review round 2,
    // finding 1): open O_NOFOLLOW, parse this object, and later dup
    // THIS descriptor into the child — never reopen the pathname.
    let rec_file = {
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&rec_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "hale replay: could not open `{}`: {}",
                    rec_path.display(),
                    e
                );
                return ExitCode::from(1);
            }
        }
    };
    let rec = match replay::parse_file(&rec_file, &rec_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("hale replay: {}", e);
            return ExitCode::from(1);
        }
    };
    if !rec.clean {
        if !allow_truncated {
            eprintln!(
                "hale replay: `{}` has no clean-finalize trailer — \
                 the recording is crash-truncated; re-record, or \
                 pass --allow-truncated to replay the recorded \
                 prefix",
                rec_path.display()
            );
            return ExitCode::from(1);
        }
        eprintln!(
            "hale replay: `{}` has no clean finalize — replaying \
             the recorded prefix (--allow-truncated)",
            rec_path.display()
        );
    }

    if !prog.is_file() {
        eprintln!(
            "hale replay: `{}` is not a file (directory seeds are \
             not replayable yet)",
            prog.display()
        );
        return ExitCode::from(1);
    }
    // Same compile pipeline as `hale run` (parse → check → model
    // hash), so a recording is admitted against exactly what runs.
    let (program, renames, sources, file_bases, _ctx) =
        match parse_with_imports(&prog) {
            Ok(x) => x,
            Err(errors) => return report_import_diags(&errors),
        };
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(prog.display().to_string(), &program);
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    let diags = hale_types::check_bundle_opts_whole_program(&bundle, false);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &sources));
        }
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let options_fp =
        options_fingerprint(&hale_codegen::BuildOptions::default());
    let (plan_digest, obs_ids) = model_identity(&bundle);
    let digest = exec_digest(&sources, &prog, &options_fp, plan_digest);

    // GH #296 phase 5b (review round): a binding backend with no
    // replay class cannot be suppressed OR injected — replaying or
    // feeding a program that carries one would touch live shared
    // memory. Fail closed with the backend named; no flag overrides
    // this (the runtime independently refuses at open). Applies to
    // strict replay and feed alike.
    {
        fn scan_shm_ring(items: &[hale_syntax::ast::TopDecl]) -> bool {
            items.iter().any(|item| match item {
                hale_syntax::ast::TopDecl::Locus(l) => {
                    l.members.iter().any(|m| match m {
                        hale_syntax::ast::LocusMember::Bindings(b) => {
                            b.entries.iter().any(|e| {
                                matches!(
                                    e.transport,
                                    hale_syntax::ast::TransportSpec::ShmRing { .. }
                                )
                            })
                        }
                        _ => false,
                    })
                }
                hale_syntax::ast::TopDecl::Module(md) => {
                    scan_shm_ring(&md.items)
                }
                _ => false,
            })
        }
        if bundle.programs.values().any(|p| scan_shm_ring(&p.items)) {
            eprintln!(
                "hale replay: this program binds a `shm_ring` — that \
                 backend has no replay class yet (it cannot be \
                 suppressed or injected, and a replayed/fed process \
                 must not touch live shared memory). Remove the \
                 binding or re-record without it (GH #296)"
            );
            return ExitCode::from(1);
        }
    }

    // Ordered BEFORE identity admission: this refusal is inherent
    // to the PROGRAM, so it must not be masked by a
    // recording-mismatch message (and module-gate canaries can
    // assert it with any recording).
    // Safe by default (review finding 3, both rounds): re-execution
    // repeats the program's real side effects. The gate consumes
    // the TYPED inferred rows, and it must fail CLOSED on the two
    // paths the first version failed open on: `unclassified`
    // ("may do anything") and a `publish` whose subject is bound to
    // an external transport (the send is generated deployment
    // machinery, invisible in user-level effects). Coarse by class
    // granularity — over-refusing is the safe direction;
    // per-primitive replay classes are the staged refinement.
    // phase 5b review round: --feed bypasses IDENTITY admission
    // (changed code is its point), never EFFECT safety — feeding a
    // tape is not an opt-in to live syscalls/FFI. Both flags
    // together are the explicit backtest-with-live-effects spelling.
    if !allow_live_effects {
        let programs: Vec<&Program> =
            bundle.programs.values().copied().collect();
        let rows =
            hale_types::effects::effect_manifest_with_inference(&programs);
        let mut residue = std::collections::BTreeSet::new();
        for row in &rows {
            for class in &row.inferred {
                if class == "syscall"
                    || class == "ffi"
                    || class == "unclassified"
                {
                    residue.insert(class.clone());
                }
            }
        }
        // External transport bindings: any `bindings { }` block
        // makes publishes (and listener startup) touch the real
        // world during re-execution.
        // Modules nest top declarations arbitrarily deep (round 3,
        // finding 2) — walk them, or a module-contained binding
        // fails open.
        fn scan_bindings(items: &[hale_syntax::ast::TopDecl]) -> bool {
            items.iter().any(|item| match item {
                hale_syntax::ast::TopDecl::Locus(l) => {
                    l.members.iter().any(|m| {
                        matches!(
                            m,
                            hale_syntax::ast::LocusMember::Bindings(_)
                        )
                    })
                }
                hale_syntax::ast::TopDecl::Module(md) => {
                    scan_bindings(&md.items)
                }
                _ => false,
            })
        }
        // phase 5b: native unix/udp `bindings { }` no longer force
        // the flag — the runtime suppresses those transports under
        // replay (opens nothing, sends nothing) and injects the
        // recorded ingress tape in the listeners' stead. Hermeticity
        // is a BINDING-KIND capability, not a blanket assumption
        // (review round): a backend with no replay class fails
        // CLOSED below. The residue that remains here is genuinely
        // user-level: syscall/ffi writes repeat, unclassified may do
        // anything.
        let _ = scan_bindings;
        if !residue.is_empty() {
            eprintln!(
                "hale replay: this program can reach the live world \
                 during re-execution ({{{}}}) — unclassified calls \
                 may do anything, and syscall/ffi writes repeat. \
                 Refusing by default; pass --allow-live-effects if \
                 you accept that",
                residue.into_iter().collect::<Vec<_>>().join(", ")
            );
            return ExitCode::from(1);
        }
    }

    // Admission, strongest check first (review finding 2):
    // exec_digest is framed-SHA-256 build-input identity;
    // shape_hash alone is only structural compatibility and admits
    // behaviorally different bodies.
    //
    // phase 5b, feed mode: admission is DELIBERATELY not required —
    // feeding a tape to changed code is the entire point. Say what
    // is being fed to what, and skip the checks.
    if feed {
        if rec.model_hash != model_hash {
            eprintln!(
                "hale replay: feed — recording is from model \
                 {:016x}, this program is {:016x} (admission not \
                 required in feed mode; subjects matched by name)",
                rec.model_hash, model_hash
            );
        }
    }
    if !feed && rec.exec_digest != [0; 4] && rec.exec_digest != digest {
        eprintln!(
            "hale replay: `{}` was recorded from different build \
             inputs (recorded exec digest {:016x}…, this compile \
             is {:016x}…) — a structurally compatible model is not \
             the same executable; re-record, or accept behavioral \
             divergence explicitly with --allow-unverified-model",
            rec_path.display(),
            rec.exec_digest[0],
            digest[0]
        );
        if !allow_unverified {
            return ExitCode::from(1);
        }
    }
    if !feed && rec.model_hash != 0 && rec.model_hash != model_hash {
        eprintln!(
            "hale replay: `{}` was recorded from a different model \
             (recorded shape_hash {:016x}, this program is {:016x}) \
             — refusing to misreplay; re-record against this \
             program",
            rec_path.display(),
            rec.model_hash,
            model_hash
        );
        return ExitCode::from(1);
    }
    if !feed
        && (rec.exec_digest == [0; 4] || rec.model_hash == 0)
        && !allow_unverified
    {
        eprintln!(
            "hale replay: `{}` carries no execution identity \
             (unstamped build) — exact replay cannot be admitted. \
             Pass --allow-unverified-model to proceed anyway",
            rec_path.display()
        );
        return ExitCode::from(1);
    }

    if rec.env_redacted && !feed {
        // (feed does not serve the env journal at all — review
        // round 2, finding 7: this warning is replay-only.)
        eprintln!(
            "hale replay: this recording withholds env VALUES \
             (default policy; record with LOTUS_OBS_RECORD_ENV=full \
             to include them) — env reads will replay as named \
             withheld divergences"
        );
    } else if rec.env_redacted && feed {
        eprintln!(
            "hale replay: feed note — the tape withheld env values; \
             feed does not replay env reads, so the current process \
             environment is used"
        );
    }
    let rec_abs = rec_path
        .canonicalize()
        .unwrap_or_else(|_| rec_path.clone());

    let mut bin = std::env::temp_dir();
    let mut h = DefaultHasher::new();
    h.write_usize(program.items.len());
    h.write_u32(std::process::id());
    bin.push(format!("hale_replay_{:016x}", h.finish()));
    let options = hale_codegen::BuildOptions {
        model_hash: Some(model_hash),
        exec_digest: Some(digest),
        obs_entity_ids: obs_ids.clone(),
        ..Default::default()
    };
    if let Err(e) = hale_codegen::build_executable_with_options(
        &program, &bin, &renames, &options,
    ) {
        eprintln!("build error: {:?}", e);
        return ExitCode::from(1);
    }

    let verify_path = if diff {
        let mut v = std::env::temp_dir();
        v.push(format!(
            "hale_replay_verify_{}_{:016x}.halerec",
            std::process::id(),
            model_hash
        ));
        let _ = std::fs::remove_file(&v);
        Some(v)
    } else {
        None
    };

    let mut status_path = std::env::temp_dir();
    status_path.push(format!(
        "hale_replay_status_{}_{:016x}",
        std::process::id(),
        model_hash
    ));
    let _ = std::fs::remove_file(&status_path);
    // Pre-create 0600 so the child's fopen("w") inherits restrictive
    // permissions rather than racing a predictable /tmp name.
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&status_path);
    }
    let mut cmd = std::process::Command::new(&bin);
    if feed {
        cmd.env("LOTUS_REPLAY_FEED", &rec_abs);
        if allow_unmatched_feed {
            cmd.env("LOTUS_REPLAY_FEED_ALLOW_UNMATCHED", "1");
        }
    } else {
        cmd.env("LOTUS_REPLAY", &rec_abs);
    }
    if allow_truncated {
        cmd.env("LOTUS_REPLAY_ALLOW_TRUNCATED", "1");
    }
    if allow_unverified {
        cmd.env("LOTUS_REPLAY_ALLOW_UNVERIFIED", "1");
    }
    cmd.env("LOTUS_REPLAY_STATUS", &status_path);
    // Round 3 + review round 2, finding 1 (artifact trust): the
    // CHILD reads THE file object the CLI admitted — `rec_file`
    // itself, opened once at the top, its descriptor dup2'd to a
    // fixed number post-fork. There is no reopen and therefore no
    // path re-resolution window. The runtime still independently
    // revalidates the structure, snapshots the bytes into anonymous
    // memory, and (defense in depth) refuses a model identity that
    // disagrees with the binary's own.
    {
        use std::os::unix::io::AsRawFd;
        use std::os::unix::process::CommandExt;
        let raw = rec_file.as_raw_fd();
        const REPLAY_FD: i32 = 973;
        cmd.env("LOTUS_REPLAY_FD", REPLAY_FD.to_string());
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(raw, REPLAY_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        // Keep the admitted File alive across spawn.
        std::mem::forget(rec_file);
    }
    if let Some(spec) = &at {
        match spec.split_once(':') {
            Some((c, n)) => {
                cmd.env("LOTUS_REPLAY_AT_CONSUMER", c);
                cmd.env("LOTUS_REPLAY_AT", n);
            }
            None => {
                cmd.env("LOTUS_REPLAY_AT", spec);
            }
        }
    }
    if let Some(v) = &verify_path {
        cmd.env("LOTUS_OBS_RECORD", v);
        // The verification recording must apply the SAME env policy
        // the original did, or a full-env recording diffs against a
        // redacted one and reports a spurious withheld divergence.
        if !rec.env_redacted {
            cmd.env("LOTUS_OBS_RECORD_ENV", "full");
        }
    }
    // Test hook (review round 2, finding 1): hold between admission
    // and spawn so the one-object invariant can be tested against a
    // deliberate path replacement, deterministically.
    if let Ok(hold) = std::env::var("HALE_REPLAY_TEST_HOLD") {
        while !std::path::Path::new(&hold).exists() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    let status = cmd.status();
    let _ = std::fs::remove_file(&bin);
    let code = match status {
        Ok(s) => s.code().unwrap_or(1).clamp(0, 255) as u8,
        Err(e) => {
            eprintln!("could not execute compiled program: {}", e);
            if let Some(v) = &verify_path {
                let _ = std::fs::remove_file(v);
            }
            return ExitCode::from(1);
        }
    };

    // The runtime's machine-readable verdict (review finding 6:
    // success must come from the verdict, not from the absence of
    // one comparator mismatch).
    let runtime_divergences: u64 = std::fs::read_to_string(&status_path)
        .ok()
        .map(|body| {
            body.lines()
                .filter_map(|l| l.split_once('='))
                .filter(|(k, _)| {
                    *k != "consumes" && *k != "post_prefix_live_fallback"
                })
                .filter_map(|(_, v)| v.trim().parse::<u64>().ok())
                .sum()
        })
        .unwrap_or_else(|| {
            eprintln!(
                "hale replay: no runtime verdict was written — \
                 treating as divergent"
            );
            1
        });
    let _ = std::fs::remove_file(&status_path);

    if let Some(v) = &verify_path {
        if !rec.async_schedule_capable {
            eprintln!(
                "hale replay: note — this recording predates \
                 async-schedule support; schedule comparison is \
                 skipped (coverage limitation, not a divergence)"
            );
        }
        if runtime_divergences > 0 {
            let msg = format!(
                "{} runtime divergences (see the summary above)",
                runtime_divergences
            );
            eprintln!("replay DIVERGED: {}", msg);
            if json {
                println!("{}", replay::diverged_json(&msg));
            }
            let _ = std::fs::remove_file(v);
            return ExitCode::from(1);
        }
        let result = match replay::parse(v) {
            Ok(vr) => match replay::diff(&rec, &vr, !rec.clean) {
                None => {
                    // GH #728: name the categories the match
                    // actually compared. "0 consumes across 0
                    // consumers" read as a verified queued schedule
                    // when direct dispatch means there was never a
                    // queued schedule to verify — and it said
                    // nothing about the public bus events and
                    // payloads that WERE compared.
                    let cov = replay::Coverage::of(&rec);
                    if json {
                        println!("{}", cov.json());
                    } else {
                        println!("{}", cov.human());
                    }
                    ExitCode::from(code)
                }
                Some(msg) => {
                    eprintln!("replay DIVERGED: {}", msg);
                    if json {
                        println!("{}", replay::diverged_json(&msg));
                    }
                    ExitCode::from(1)
                }
            },
            Err(e) => {
                eprintln!(
                    "hale replay: verification recording unreadable: {}",
                    e
                );
                if json {
                    println!(
                        "{}",
                        replay::diverged_json(&format!(
                            "verification recording unreadable: {}",
                            e
                        ))
                    );
                }
                ExitCode::from(1)
            }
        };
        let _ = std::fs::remove_file(v);
        return result;
    }
    ExitCode::from(code)
}

fn run_program(target: &Path, user_args: &[String]) -> ExitCode {
    // Both single-file and directory targets resolve cross-seed
    // imports and thread the per-build path-rename table into
    // codegen — `run` and `build` agree (WS3.3). A single file
    // follows `import "..."` from its own directory; a directory
    // bundles its `.hl` files as one seed and resolves the union
    // of their imports (see the directory branch below).
    if target.is_file() {
        // `compile_and_exec` passes `renames` to
        // `build_executable_with_imports`, so qualified
        // `alias::Name` references in the entry file resolve the
        // same way `hale build` resolves them.
        let (program, renames, sources, file_bases, _ctx) = match parse_with_imports(target) {
            Ok(x) => x,
            Err(errors) => return report_import_diags(&errors),
        };
        let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
        bundle_programs.insert(target.display().to_string(), &program);
        // The rename table must reach the analysis here too, not only in
        // `check`. Without it a cross-seed call is an unresolved edge, so
        // an effect assertion violated one seed away compiles, links and
        // ships — a downstream fleet gates on `build` across 109 binaries,
        // and "it built" must not be weaker than "it checked" on a
        // contract the compiler already knows how to evaluate.
        let mut bundle = hale_types::Bundle::new(bundle_programs);
        bundle.import_renames = renames.clone();
        let allow_unowned =
            std::env::args().any(|a| a == "--allow-unowned-subscriber");
        let diags = hale_types::check_bundle_opts_whole_program(&bundle, allow_unowned);
        if !diags.is_empty() {
            for d in &diags {
                eprintln!("{}", render_located(d, &file_bases, &sources));
            }
            // Warnings print but don't fail the build; only errors do.
            if diags.iter().any(|d| d.is_error()) {
                return ExitCode::from(1);
            }
        }
        // P26: stamp the model identity of the bundle just checked.
        let model_hash =
            hale_types::topology::model_shape_hash(&bundle);
        let options_fp =
            options_fingerprint(&hale_codegen::BuildOptions::default());
        let (plan_digest, obs_ids) = model_identity(&bundle);
        let digest =
            exec_digest(&sources, target, &options_fp, plan_digest);
        return compile_and_exec(
            &program, &renames, user_args, model_hash, digest, obs_ids,
        );
    }

    let files = match collect_ap_files(target) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };
    let (programs, sources, mut file_bases) = match parse_files(&files) {
        Ok(x) => x,
        // `run` has no machine-readable channel: the same located
        // text it always printed (GH #777 moved the printing here).
        Err(f) => return f.report_text(),
    };

    // WS3.3 (2026-06-11): a directory `hale run` now resolves
    // cross-seed imports the same way `hale build <dir>` does.
    // Previously it bundled the directory's files but silently
    // dropped every `import "..."`, so a dir-seed app importing a
    // vendored library failed on `alias::Name` references — the
    // exact pond / downstream apps "qualified type not in path-renames table"
    // friction, and the reason a topic decl had to live in the same
    // file as its publisher. `run` and `build` now produce the same
    // merged-and-resolved program for a directory; `run` execs it
    // instead of writing a binary.
    let mut union_imports: Vec<hale_syntax::ast::Import> = Vec::new();
    for prog in programs.values() {
        for imp in &prog.imports {
            union_imports.push(imp.clone());
        }
    }
    let merged = match merge_programs(programs.values()) {
        Some(m) => m,
        None => {
            eprintln!("no .hl files in {}", target.display());
            return ExitCode::from(1);
        }
    };
    let workspace_root = find_workspace_root(target);
    let mut effects = EffectTable::from_seed(&merged);
    let mut merged_items = merged.items;
    // Same identity-seeding rule as the entry path: `merged`'s own
    // items are already in `merged_items` and are never walked, so its
    // table must come first.
    let mut renames: ImportRenames = Vec::new();
    let mut seed_cache: BTreeMap<PathBuf, std::collections::HashMap<String, String>> = BTreeMap::new();
    let mut path_sources: BTreeMap<PathBuf, String> = sources.into_iter().collect();
    let mut visited: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::new();
    for f in &files {
        match f.canonicalize() {
            Ok(c) => visited.insert(c),
            Err(_) => visited.insert(f.clone()),
        };
    }
    let mut import_errors: Vec<ImportDiag> = Vec::new();
    // GH #746: the directory is one seed; its aliases are scoped to it.
    let target_scope =
        target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
    let mut alias_scopes = AliasScopes::default();
    alias_scopes.record_files(
        &target_scope,
        files
            .iter()
            .map(|f| f.canonicalize().unwrap_or_else(|_| f.clone()))
            .collect(),
    );
    if resolve_imports(
        &union_imports,
        target,
        workspace_root.as_deref(),
        &mut visited,
        &mut path_sources,
        &mut file_bases,
        &mut import_errors,
        &mut merged_items,
        &mut renames,
        &mut seed_cache,
        &mut effects,
        &target_scope,
        &mut alias_scopes,
    )
    .is_err()
        || !import_errors.is_empty()
    {
        return report_import_diags(&import_errors);
    }
    let mut program = Program {
        declared_effects: effects.declared_indices(),
        effect_defs: effects.defs,
        effect_names: effects.names,
        imports: Vec::new(),
        items: merged_items,
        span: merged.span,
    };
    // GH #762: refuse a reference to an alias this seed never
    // declared, the same as `check` does.
    let unscoped = unscoped_alias_uses(
        &program,
        &file_bases,
        &path_sources,
        &alias_scopes,
        &seed_cache,
    );
    if !unscoped.is_empty() {
        for u in &unscoped {
            eprintln!("{}", render_located(&u.diag, &file_bases, &path_sources));
        }
        return ExitCode::from(1);
    }
    // Rewrite qualified-path TypeExprs + synthesize JSON parsers +
    // apply sync inference before typecheck — the same pre-passes
    // `hale build <dir>` runs, so a directory `run` and `build`
    // agree.
    scope_import_aliases(
        &mut program,
        &mut renames,
        &file_bases,
        &alias_scopes,
        &seed_cache,
    );
    hale_codegen::mangle::apply_qualified_path_renames(&mut program, &renames);
    hale_syntax::json_gen::generate_json_parsers(&mut program);
    // Pre-pass diags are re-raised by `check_bundle_opts` below
    // through the normal rendering — bailing here double-reported
    // (see the `check` site for the full story).
    let _ = hale_types::apply_sync_inference(&mut program);

    let bundle_programs: BTreeMap<String, &Program> =
        std::iter::once((target.display().to_string(), &program)).collect();
    // The rename table must reach the analysis here too, not only in
    // `check`. Without it a cross-seed call is an unresolved edge, so
    // an effect assertion violated one seed away compiles, links and
    // ships — a downstream fleet gates on `build` across 109 binaries,
    // and "it built" must not be weaker than "it checked" on a
    // contract the compiler already knows how to evaluate.
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    let allow_unowned =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    let diags = hale_types::check_bundle_opts_whole_program(&bundle, allow_unowned);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &path_sources));
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    // P26: stamp the model identity of the bundle just checked.
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let options_fp =
        options_fingerprint(&hale_codegen::BuildOptions::default());
    let (plan_digest, obs_ids) = model_identity(&bundle);
    let digest =
        exec_digest(&path_sources, target, &options_fp, plan_digest);
    compile_and_exec(
        &program, &renames, user_args, model_hash, digest, obs_ids,
    )
}

fn run_build(target: &Path) -> ExitCode {
    // Phase 2i: warn if the CLI binary was built against an older
    // codegen+runtime source tree than what's on disk now. Silent
    // miscompile (stale CLI emitting old lowering against new
    // source) is the worst failure mode for a cold-context agent —
    // see `apps/log-router/FRICTION.md` 2026-05-10. The check is
    // best-effort: it skips when source files aren't locatable
    // (installed binary, moved workspace), or when the user
    // explicitly opts out via `HALE_SKIP_STALE_CHECK=1`.
    check_stale_cli();

    // File targets follow `import "..."` directives starting from
    // the entry's directory; directory targets bundle every .hl
    // file in the directory as one seed (the per-dir package
    // model — myapp/{main,render,topology}.hl → one binary). The
    // directory shape is the user-facing answer to the
    // single-file-app-monolith friction; the file shape stays for
    // backwards compatibility and for one-off scripts.
    let (mut program, renames, sources, file_bases, output, entry_ctx) = if target.is_file() {
        let (program, renames, sources, file_bases, ctx) = match parse_with_imports(target) {
            Ok(x) => x,
            Err(errors) => return report_import_diags(&errors),
        };
        // hello-world.hl → hello-world
        let output = target.with_extension("");
        (program, renames, sources, file_bases, output, ctx)
    } else if target.is_dir() {
        let files = match collect_ap_files(target) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::from(1);
            }
        };
        let (programs, sources, mut dir_file_bases) = match parse_files(&files) {
            Ok(x) => x,
            // As for `run`: located text, unchanged (GH #777).
            Err(f) => return f.report_text(),
        };
        // Collect the union of all imports across the bundle's
        // files. Multiple files in one seed may share an import
        // alias (e.g. both reference `lib/foo`); the visited-set
        // inside resolve_imports dedupes by canonical file path,
        // so the same import resolved twice is a no-op.
        let mut union_imports: Vec<hale_syntax::ast::Import> = Vec::new();
        for prog in programs.values() {
            for imp in &prog.imports {
                union_imports.push(imp.clone());
            }
        }
        let merged = match merge_programs(programs.values()) {
            Some(m) => m,
            None => {
                eprintln!("no .hl files in {}", target.display());
                return ExitCode::from(1);
            }
        };
        // Resolve the union of imports against the directory's
        // own dir as the importer dir + the workspace fallback.
        let workspace_root = find_workspace_root(target);
        let mut effects = EffectTable::from_seed(&merged);
    let mut merged_items = merged.items;
        // Identity-seeded: `merged`'s items are already merged.
        let mut renames: ImportRenames = Vec::new();
    let mut seed_cache: BTreeMap<PathBuf, std::collections::HashMap<String, String>> = BTreeMap::new();
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
        let mut import_errors: Vec<ImportDiag> = Vec::new();
        // GH #746: the directory is one seed; its aliases are scoped
        // to it.
        let target_scope =
            target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
        let mut alias_scopes = AliasScopes::default();
        alias_scopes.record_files(
            &target_scope,
            files
                .iter()
                .map(|f| f.canonicalize().unwrap_or_else(|_| f.clone()))
                .collect(),
        );
        if resolve_imports(
            &union_imports,
            target,
            workspace_root.as_deref(),
            &mut visited,
            &mut path_sources,
            &mut dir_file_bases,
            &mut import_errors,
            &mut merged_items,
            &mut renames,
            &mut seed_cache,
            &mut effects,
            &target_scope,
            &mut alias_scopes,
        )
        .is_err()
            || !import_errors.is_empty()
        {
            return report_import_diags(&import_errors);
        }
        let mut with_imports = Program {
            declared_effects: effects.declared_indices(),
            effect_defs: effects.defs,
            effect_names: effects.names,
            imports: Vec::new(),
            items: merged_items,
            span: merged.span,
        };
        // GH #762: refuse a reference to an alias this seed never
        // declared, the same as `check` does.
        let unscoped = unscoped_alias_uses(
            &with_imports,
            &dir_file_bases,
            &path_sources,
            &alias_scopes,
            &seed_cache,
        );
        if !unscoped.is_empty() {
            for u in &unscoped {
                eprintln!(
                    "{}",
                    render_located(&u.diag, &dir_file_bases, &path_sources)
                );
            }
            return ExitCode::from(1);
        }
        // GH #746: scope a contested alias to its declaring seed.
        scope_import_aliases(
            &mut with_imports,
            &mut renames,
            &dir_file_bases,
            &alias_scopes,
            &seed_cache,
        );
        // brained F.1: rewrite qualified-path TypeExprs in the
        // entry program before typecheck (see parse_with_imports
        // for the rationale).
        hale_codegen::mangle::apply_qualified_path_renames(
            &mut with_imports,
            &renames,
        );
        // myapp/ → myapp; output lands next to target. When the
        // user passes `.` (or any path without a useful trailing
        // component — `./`, `..`), `Path::file_name` returns None;
        // canonicalize to recover the actual directory name so the
        // emitted binary is `<dir>/<dir>` instead of `<dir>/main`.
        let bin_name = target
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| {
                target.canonicalize().ok().and_then(|p| {
                    p.file_name().map(|s| s.to_string_lossy().into_owned())
                })
            })
            .unwrap_or_else(|| "main".to_string());
        let mut output = target.to_path_buf();
        output.push(&bin_name);
        let ctx = EntryCtx {
            entry_dir: target.to_path_buf(),
            workspace_root,
            imports: union_imports,
        };
        (with_imports, renames, path_sources, dir_file_bases, output, ctx)
    } else {
        eprintln!("not a file or directory: {}", target.display());
        return ExitCode::from(1);
    };

    // FUv0.8.2 #4 (2026-05-25): auto-apply sync inference
    // BEFORE typecheck. Walks the program, runs F.32-1∞ on
    // `@form(hashmap)` loci without explicit `sync = `, and
    // injects the picked discipline as a synthetic FormArg.
    // The subsequent typecheck sees an explicit sync and the
    // F.32-0 cross-pool diagnostic stays quiet for auto-
    // inferable cases. Loci with existing sync kwarg or
    // single-pool use are left alone.

    // `--wrap-main` (browser playground): synthesize the wasm `@export`
    // entry from a bare `fn main` on the AST, BEFORE typecheck — so the
    // checker sees the synthesized `target wasm` gate + `@export` locus,
    // and every diagnostic keeps the user's original line/col (no textual
    // wrap, no offset). Wasm-only: there is no native entry inversion to
    // wrap, so on a native build it is a hard error rather than a silent
    // no-op (which would mask a misconfigured playground build).
    if std::env::args().any(|a| a == "--wrap-main") {
        let args: Vec<String> = std::env::args().collect();
        let target_wasm = args.windows(2).any(|w| {
            w[0] == "--target" && (w[1] == "wasm32" || w[1] == "wasm")
        });
        if !target_wasm {
            eprintln!(
                "error: --wrap-main requires --target wasm32 — it \
                 synthesizes the wasm @export entry from `fn main`, and \
                 there is no native entry-inversion to wrap"
            );
            return ExitCode::from(2);
        }
        hale_syntax::desugar::wrap_main_as_wasm_export(&mut program);
    }

    hale_syntax::json_gen::generate_json_parsers(&mut program);
    // Pre-pass diags are re-raised by `check_bundle_opts` below
    // through the normal rendering — bailing here double-reported
    // (see the `check` site for the full story).
    let _ = hale_types::apply_sync_inference(&mut program);

    // Typecheck before lowering. Render diagnostics against the
    // entry-file's source — diagnostic spans currently point into
    // the merged item stream which doesn't have a single source
    // string; this is good enough for v0.
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(target.display().to_string(), &program);
    // The rename table must reach the analysis here too, not only in
    // `check`. Without it a cross-seed call is an unresolved edge, so
    // an effect assertion violated one seed away compiles, links and
    // ships — a downstream fleet gates on `build` across 109 binaries,
    // and "it built" must not be weaker than "it checked" on a
    // contract the compiler already knows how to evaluate.
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    let allow_unowned =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    let diags = hale_types::check_bundle_opts_whole_program(&bundle, allow_unowned);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &sources));
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    let mut options = match parse_build_options() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    // P26: stamp the model identity of the bundle just checked into
    // the binary, for the observation segment header.
    options.model_hash =
        Some(hale_types::topology::model_shape_hash(&bundle));
    // GH #476 Change 8: the canonical entity ids a consumer joins
    // the live manifest to that model with, and the dispatch
    // plan's digest — held here and folded into the execution
    // identity once the options are FINAL (below), since the
    // fingerprint covers options that are still being set.
    let (plan_digest, obs_ids) = model_identity(&bundle);
    options.obs_entity_ids = obs_ids;
    // WASM plan: a wasm build emits `<stem>.wasm` (a relocatable wasm
    // object at this stage) rather than the extension-less native binary.
    // Output naming is a property of the target, not a special case
    // spelled at this one call site (GH #445).
    let output = {
        let ext = options.target.spec().filenames().executable;
        if ext.is_empty() {
            output
        } else {
            output.with_extension(ext)
        }
    };
    // F.32-2 (2026-05-25): operator-facing per-locus working-set
    // report + budget gate.
    //
    // * `--locality-report` emits the full per-locus table on
    //   stderr (informational; build proceeds).
    // * `--target-cache l1|l2|l3` evaluates each locus against
    //   the named cache tier's budget. Over-budget loci surface
    //   as a stderr warning by default, or — with `--strict` —
    //   a build error (exit 1 before codegen).
    // * Both flags can be combined: `--locality-report
    //   --target-cache l2` shows everything AND gates.
    //
    // The estimator is approximate (alignment padding partially
    // accounted, method scratch heuristic-only). The budget
    // gate consults the same numbers the report shows, so a
    // warning matches what the report attributes to each
    // locus.
    let cli_args: Vec<String> = std::env::args().collect();
    let want_report = cli_args.iter().any(|a| a == "--locality-report");
    let target_cache_arg: Option<&str> = {
        let mut found = None;
        let mut it = cli_args.iter();
        while let Some(a) = it.next() {
            if a == "--target-cache" {
                found = it.next().map(|s| s.as_str());
                break;
            }
        }
        found
    };
    let strict = cli_args.iter().any(|a| a == "--strict");
    // Resolve the global target tier early so a parse error
    // surfaces before any analysis runs.
    let global_target: Option<hale_types::working_set::CacheTier> =
        match target_cache_arg {
            Some(raw) => match hale_types::working_set::parse_cache_tier(raw) {
                Some(t) => Some(t),
                None => {
                    eprintln!(
                        "error: --target-cache: unknown tier `{}` \
                         (expected l1 / l2 / l3)",
                        raw
                    );
                    return ExitCode::from(2);
                }
            },
            None => None,
        };
    let any_locality_annotation = program.items.iter().any(|item| {
        matches!(item, hale_syntax::ast::TopDecl::Locus(l) if l.locality.is_some())
    });
    if strict && global_target.is_none() && !any_locality_annotation {
        // `--strict` gates the working-set breaches that
        // surface from `--target-cache` or `@locality(...)`.
        // Without either, no budget applies and `--strict`
        // is a no-op — surface the misconfiguration so a CI
        // job doesn't silently believe it's enforcing
        // anything.
        eprintln!(
            "warning: --strict has no effect without \
             --target-cache l1|l2|l3 or `@locality(...)` annotations"
        );
    }
    // Always run the per-locus evaluator — even without
    // `--target-cache`, loci carrying `@locality(L1|L2|L3)` are
    // a hard contract and need checking. The early exit when
    // there's nothing to evaluate is cheap.
    if want_report || global_target.is_some() || any_locality_annotation {
        let map =
            hale_types::working_set::compute_program_working_set(
                &program.items,
            );
        if want_report {
            eprint!(
                "{}",
                hale_types::working_set::render_locality_report(&map)
            );
        }
        let breaches =
            hale_types::working_set::breaches_with_per_locus_budgets(
                &map,
                &program.items,
                global_target,
            );
        if !breaches.is_empty() {
            let severity = if strict { "error" } else { "warning" };
            eprint!(
                "{}",
                hale_types::working_set::render_breach_diagnostic(
                    &breaches, severity,
                )
            );
            if strict {
                return ExitCode::from(1);
            }
        }
    }
    // Stage-2 FFI: append the FFI surface declared by each
    // imported lib's hale.toml [ffi] section. CLI flags from
    // parse_build_options come first (preserves the manual
    // escape hatch); toml-sourced flags append. Duplicates are
    // tolerated — clang's `-lX -lX` is harmless, and the linker
    // dedupes csrc translation-unit contents at symbol level.
    let toml_opts = collect_ffi_from_imports(
        &entry_ctx.imports,
        &entry_ctx.entry_dir,
        entry_ctx.workspace_root.as_deref(),
    );
    options.link_libs.extend(toml_opts.link_libs);
    options.csrc_files.extend(toml_opts.csrc_files);
    // 2026-07-01 debug story stage 2: DWARF line tables, ON by
    // default (debug sections cost binary bytes, zero runtime
    // speed). LOTUS_NO_DEBUGINFO=1 opts out. The source table is
    // the same (base, path, len) file map diagnostics demux with,
    // plus each file's text for line-start computation.
    let no_dbg = std::env::var("LOTUS_NO_DEBUGINFO")
        .map(|v| v == "1" || v == "true" || v == "TRUE")
        .unwrap_or(false);
    if !no_dbg {
        // #8 dev profile (2026-07-02): `hale build --dev` (or
        // HALE_DEV=1) trades runtime speed for build latency —
        // LLVM O1 instead of the O3 release default. Profiled: the
        // front-end is ~35 ms even on the largest apps; LLVM is
        // 97% of build wall time.
        options.dev_profile = std::env::args().any(|a| a == "--dev")
            || std::env::var("HALE_DEV").is_ok();
        options.debug = Some(hale_codegen::DebugSources {
            files: file_bases
                .iter()
                .filter_map(|(base, path, len)| {
                    sources.get(path).map(|text| {
                        hale_codegen::DebugSourceFile {
                            base: *base,
                            len: *len,
                            path: path.clone(),
                            text: text.clone(),
                        }
                    })
                })
                .collect(),
        });
    }
    // The execution identity, stamped LAST: every option that
    // alters emitted code is set by now, and the digest frames the
    // finalized fingerprint together with the dispatch plan. Before
    // this, `hale build` artifacts carried a model hash and no
    // execution identity at all — so a recording from one could not
    // be refused against a differently-lowered sibling, which is
    // exactly what the identity is for.
    options.exec_digest = Some(exec_digest(
        &sources,
        target,
        &options_fingerprint(&options),
        plan_digest,
    ));
    match hale_codegen::build_executable_with_options(
        &program,
        &output,
        &renames,
        &options,
    ) {
        Ok(()) => {
            eprintln!("built: {}", output.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            // GH #241: span-carrying codegen errors render like
            // check diagnostics (file:line:col + source caret);
            // everything else keeps the bare line.
            if let hale_codegen::CodegenError::UnsupportedAt(msg, span) = &e {
                let d = hale_syntax::Diag::codegen(*span, msg.clone());
                eprintln!("{}", render_located(&d, &file_bases, &sources));
            } else {
                eprintln!("codegen error: {}", e);
            }
            ExitCode::from(1)
        }
    }
}

/// Stage-2 FFI (2026-05-22): walk a program's top-level imports,
/// resolve each one against the entry's directory + workspace
/// root (same lookup `resolve_imports` uses), and accumulate the
/// `[ffi]` section of each imported lib's `hale.toml` into a
/// `BuildOptions`. `csrc` paths are resolved relative to the
/// lib's own directory; `link` libs append unconditionally.
///
/// Single-file imports (`import "helpers"` resolving to
/// `helpers.hl`) carry no `hale.toml` and contribute nothing
/// here. Imports that don't resolve are silently skipped — the
/// main resolver surfaces those as diagnostics; double-erroring
/// here just adds noise.
///
/// De-duplication: a lib referenced under two aliases or pulled
/// in transitively (Stage 2 only walks the top-level imports;
/// transitive FFI is a Stage 2-follow-on if/when needed)
/// contributes its flags once per unique lib directory.
fn collect_ffi_from_imports(
    imports: &[hale_syntax::ast::Import],
    importer_dir: &Path,
    workspace_root: Option<&Path>,
) -> hale_codegen::BuildOptions {
    let mut opts = hale_codegen::BuildOptions::default();
    let mut seen_dirs: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::new();
    for imp in imports {
        if imp.path.starts_with("std/") || imp.path == "std" {
            continue;
        }
        let target = match resolve_import(importer_dir, workspace_root, &imp.path) {
            Some(t) => t,
            None => continue,
        };
        let lib_dir = match target {
            ImportTarget::SingleFile(_) => continue,
            ImportTarget::Directory(d) => d,
        };
        let canon = lib_dir.canonicalize().unwrap_or_else(|_| lib_dir.clone());
        if !seen_dirs.insert(canon) {
            continue;
        }
        match crate::pkg::read_lib_ffi(&lib_dir) {
            Ok(Some(ffi)) => {
                for lib in ffi.link {
                    opts.link_libs.push(lib);
                }
                for csrc in ffi.csrc {
                    let csrc_path = lib_dir.join(csrc);
                    opts.csrc_files.push(csrc_path);
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!(
                    "warning: reading hale.toml in {}: {}",
                    lib_dir.display(),
                    e,
                );
            }
        }
    }
    opts
}

/// Stage-1 FFI (2026-05-22): parse `--link` / `--csrc` flags from
/// `hale build`'s trailing argv. Each flag is repeatable; the
/// flag and its value are two separate argv entries (no `=`
/// shorthand at Stage 1). Unknown flags surface as a clear
/// diagnostic so the user knows we didn't silently swallow them.
fn parse_build_options() -> Result<hale_codegen::BuildOptions, String> {
    let mut opts = hale_codegen::BuildOptions::default();
    let args: Vec<String> = std::env::args().collect();
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--link" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--link requires a library name (e.g. --link raylib)"
                        .to_string()
                })?;
                opts.link_libs.push(v.clone());
                i += 2;
            }
            "--csrc" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--csrc requires a path to a .c file".to_string()
                })?;
                opts.csrc_files.push(std::path::PathBuf::from(v));
                i += 2;
            }
            // F.32-2 (2026-05-25): operator-facing per-locus
            // working-set report. Consumed in main.rs before
            // codegen; recognized here so parse_build_options
            // doesn't error out on an unknown flag.
            "--locality-report" => {
                i += 1;
            }
            // F.32-2 v0.2 (2026-05-25): cache-budget gate.
            // `--target-cache l1|l2|l3` runs the working-set
            // estimator against the named tier and emits a
            // warning (or, with `--strict`, a build error) for
            // any locus whose total exceeds the budget. The
            // value is taken from the next argv entry, parallel
            // to --link / --csrc. Consumed in main.rs; just
            // skipped here so the unknown-flag arm doesn't
            // fire.
            "--target-cache" => {
                // Eat the tier value too; main.rs will re-parse
                // env::args. Defensive: if --target-cache is
                // the last arg we still consume one entry and
                // let main.rs surface the missing-value error
                // (keeps parse_build_options simple).
                if args.get(i + 1).is_some() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--strict" => {
                i += 1;
            }
            // Browser-playground entry synthesis (handled in the build
            // flow, before typecheck — see `wrap_main_as_wasm_export`).
            // Accepted here so it isn't an "unknown flag".
            "--wrap-main" => {
                i += 1;
            }
            // WASM plan: select the compilation backend. Distinct from
            // `--target-cache` (a working-set gate). `wasm32` emits the
            // relocatable wasm object for the browser/full-stack-web target.
            "--target" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--target requires a value (native|wasm32|<triple>)".to_string()
                })?;
                // Canonical triples, not just the two aliases. A target
                // the compiler can NAME is not necessarily one it can
                // BUILD, so say which of the two this is rather than
                // failing later inside the linker (GH #445).
                let spec = hale_codegen::target::TargetSpec::parse(v)
                    .map_err(|e| format!("--target: {}", e))?;
                if spec.support()
                    == hale_codegen::target::TargetSupport::Planned
                {
                    return Err(format!(
                        "--target: `{}` is not buildable yet\n\n{}\n\n\
                         The target model knows this platform; the codegen \
                         and runtime for it do not exist yet. Track GH #445.",
                        spec.triple,
                        spec.describe(),
                    ));
                }
                opts.target = if spec.is_wasm() {
                    hale_codegen::CompileTarget::Wasm32
                } else {
                    hale_codegen::CompileTarget::Native
                };
                i += 2;
            }
            // Backend CPU tuning for the native target. `native` tunes to
            // the host (best perf, not portable); `baseline` pins a
            // portable x86-64-v3 baseline for distributed artifacts.
            "--target-cpu" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--target-cpu requires a value (native|baseline)".to_string()
                })?;
                opts.target_cpu = match v.as_str() {
                    "native" => hale_codegen::TargetCpu::Native,
                    "baseline" | "x86-64-v3" => hale_codegen::TargetCpu::X86_64V3,
                    other => {
                        return Err(format!(
                            "--target-cpu: unknown value `{}` (expected native|baseline)",
                            other
                        ));
                    }
                };
                i += 2;
            }
            // #8 dev profile (2026-07-02): LLVM O1 instead of the
            // O3 release default — build-latency mode. Consumed in
            // run_build via env::args (options finalization);
            // recognized here so the arg parser doesn't reject it.
            "--dev" => {
                i += 1;
            }
            other => {
                return Err(format!(
                    "unknown `hale build` flag: {}",
                    other
                ));
            }
        }
    }
    Ok(opts)
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
fn merge_programs<'a, I>(programs: I) -> Option<Program>
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

/// Phase 2i: warn when the CLI binary's bundled codegen + runtime
/// source snapshots are stale relative to the workspace's on-disk
/// source. Both the baked-in hash (set at build time by
/// `build.rs`) and the runtime-recomputed hash use the same
/// algorithm — DefaultHasher over each file's bytes, salted with
/// the relative path — so they match exactly when the on-disk
/// tree is the one the binary was built against.
///
/// Skipped silently when:
///  - `HALE_SKIP_STALE_CHECK=1` is set,
///  - the baked codegen directory doesn't exist on this host
///    (installed binary, moved workspace),
///  - `build.rs` couldn't locate the workspace at build time
///    (the env vars are empty).
fn check_stale_cli() {
    if env::var_os("HALE_SKIP_STALE_CHECK")
        .filter(|v| !v.is_empty() && v != "0")
        .is_some()
    {
        return;
    }
    let baked_hash = env!("HALE_CODEGEN_SRC_HASH");
    let baked_dir = env!("HALE_CODEGEN_DIR");
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
            "warning: hale CLI binary was built against an older \
             codegen+runtime source tree."
        );
        eprintln!(
            "         {} has changed since the CLI was built; the \
             emitted binary may use stale lowering.",
            codegen_dir.display()
        );
        eprintln!(
            "         Rebuild with: cargo build -p hale-cli"
        );
        eprintln!(
            "         (Set HALE_SKIP_STALE_CHECK=1 to silence \
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
                    == Some("hl")
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


/// GH #265: minimal line diff for the effect-manifest gate — enough
/// to show WHICH fn's effects changed without pulling in a diff
/// crate. Lines are stable-sorted by fn name, so a set difference is
/// an accurate rendering.
fn diff_lines(expected: &str, current: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let a: BTreeSet<&str> = expected.lines().collect();
    let b: BTreeSet<&str> = current.lines().collect();
    let mut out = Vec::new();
    for gone in a.difference(&b) {
        out.push(format!("  - {}", gone));
    }
    for added in b.difference(&a) {
        out.push(format!("  + {}", added));
    }
    out
}

/// GH #527 B4: `hale model diff <a> <b> [--json|--text]`.
fn run_model_diff(rest: &[String]) -> ExitCode {
    let mut paths: Vec<&String> = Vec::new();
    let mut text = false;
    for a in rest {
        match a.as_str() {
            "--json" => text = false,
            "--text" => text = true,
            "--help" | "-h" => {
                eprintln!("usage: hale model diff <a.topology> <b.topology> [--json|--text]");
                return ExitCode::SUCCESS;
            }
            f if f.starts_with("--") => {
                eprintln!("hale model diff: unknown flag `{f}`");
                return ExitCode::from(2);
            }
            _ => paths.push(a),
        }
    }
    if paths.len() != 2 {
        eprintln!("usage: hale model diff <a.topology> <b.topology> [--json|--text]");
        return ExitCode::from(2);
    }
    let mut admitted = Vec::new();
    for p in &paths {
        let raw = match std::fs::read_to_string(p) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("hale model diff: cannot read {p}: {e}");
                return ExitCode::from(2);
            }
        };
        match hale_types::topology_diff::admit(p, &raw) {
            Ok(a) => admitted.push(a),
            Err(e) => {
                eprintln!("hale model diff: {e}");
                return ExitCode::from(2);
            }
        }
    }
    let d = hale_types::topology_diff::diff(&admitted[0], &admitted[1]);
    if text {
        print!("{}", hale_types::topology_diff::render_text(&d));
    } else {
        println!("{}", serde_json::to_string_pretty(&d).unwrap_or_default());
    }
    ExitCode::SUCCESS
}
