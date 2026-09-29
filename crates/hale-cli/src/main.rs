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

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::Ordering;


use hale_lsp as lsp;
use shared::imports::ImportDiag;
use verbs::test::run_test;
use verbs::run::run_program;
use verbs::build::{run_build};
use verbs::replay::run_replay;
use verbs::check::cli::{run_check_cli};
use verbs::check::run_impl::EffectTable;
use verbs::fmt::run_fmt;
use verbs::doc::run_doc;
use verbs::bench::run_bench;
use verbs::fleet::run_fleet;
use verbs::model::{model_usage, run_model_diff};
use verbs::init::run_init;
use verbs::help::{subcommand_help, usage};
use shared::options::{parse_exec_build_options, split_target_args};
use shared::stale::check_stale_cli;
use shared::workspace::seed_inputs;
mod build_env;
mod fleet;
mod dna;
mod iris;
mod api_client;
mod mcp;
mod pkg;
mod replay;
mod sign;
mod topology_graph;
mod fleet_model;
mod topology_law;
mod shared;
mod verbs;

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

    // GH #817: `--help` / `-h` right after a subcommand asks what
    // THAT subcommand takes. Answered here, once, before any command
    // parses its own arguments — otherwise the flag is whatever each
    // command does with an unrecognized first argument, and no two
    // agree (`hale build --help` read it as the target and failed
    // with `not a file or directory: --help`). Only the first
    // argument: further along it belongs to whatever is parsing
    // there.
    if matches!(args.get(2).map(String::as_str), Some("--help") | Some("-h"))
        && subcommand_help(cmd)
    {
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
            println!("{}{}\n", t.describe_from(&host), marker);
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
        // GH #1107: `hale mcp --app <socket>` serves a running api
        // binding's commands as tools and its reads as resources.
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return match rest.as_slice() {
            [] => mcp::run_mcp(),
            [flag, sock] if flag == "--app" => mcp::run_mcp_app(sock),
            _ => {
                eprintln!("usage: hale mcp [--app <socket>]");
                ExitCode::from(2)
            }
        };
    }

    // GH #1107: the generic clients of an api binding. They read the
    // description the binding serves and nothing else.
    if cmd == "describe" || cmd == "call" || cmd == "watch" || cmd == "admin" {
        let rest: Vec<String> = args.iter().skip(2).cloned().collect();
        return match cmd.as_str() {
            "describe" => api_client::run_describe(&rest),
            "call" => api_client::run_call(&rest),
            "watch" => api_client::run_watch(&rest),
            _ => api_client::run_admin(&rest),
        };
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
            eprint!("{}", model_usage());
            return ExitCode::from(2);
        }
        // The check pipeline's dump section reads PROCESS argv (it
        // is a top-level-command scope), so the flag cannot ride the
        // rest-args the shim forwards; the shim marks the demand on
        // the process instead.
        verbs::check::MODEL_DUMP_DEMANDED.store(true, Ordering::Relaxed);
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
    // GH #785: the stale-binary warning on every command that reads
    // source or materializes the embedded DNA, not only a build — an
    // edited dna/core that was never rebuilt in is announced by the
    // next `hale check` or `hale dna …`, before a fixture runs it.
    if matches!(cmd.as_str(), "check" | "verify" | "build" | "run" | "test" | "dna" | "inputs") {
        check_stale_cli();
    }
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

    // GH #861: `build` and `run` split their arguments the way
    // `check` does — the first argument that is not a flag IS the
    // target, and a flag is a flag wherever it stands. Before this,
    // the target was always argv[2] and `parse_build_options`
    // started at argv[3], so `hale build --dev app.hl` failed with
    // `not a file or directory: --dev` while `hale check --json
    // app.hl` was fine. One splitter, so the two commands cannot
    // drift apart again.
    if cmd == "build" || cmd == "run" {
        let (before, target, after) = split_target_args(&args[2..]);
        let target = match target {
            Some(t) => PathBuf::from(t),
            None => {
                usage();
                return ExitCode::from(2);
            }
        };
        if cmd == "build" {
            // `build` has no trailing operand of its own, so the
            // flags on both sides are one list.
            let mut flags = before;
            flags.extend(after);
            return run_build(&target, &flags);
        }
        // `hale run` compiles the program to a temporary binary
        // (the same codegen backend as `hale build`) and executes
        // it — there is no separate interpreter. The program's
        // trailing argv is forwarded to the exec'd process, so
        // `hale run script.hl foo bar` makes the program's
        // `std::env::arg(1..)` see ["foo", "bar"] exactly as a
        // built binary run directly would. That is why the
        // splitter's rule stops at the target here: after it, a
        // `--flag` is the PROGRAM's, not ours.
        let mut user_args = after;
        // GH #527 B3: `hale run --observe <target>` — the program
        // publishes its observation segment (LOTUS_OBS=1, inherited
        // by the child) and an iris session runs beside it for the
        // program's lifetime. The flag is consumed here; nothing
        // reaches the program's argv. Accepted immediately after
        // the target too, the spelling that shipped in B3. The
        // session writes to its own pipe and dies with this process
        // whatever kills it (GH #905) — the program's stdout stays
        // the command's output, and there is no orphan left holding
        // it open.
        let mut observe = before.iter().any(|f| f == "--observe");
        if !observe && user_args.first().map(String::as_str) == Some("--observe") {
            observe = true;
            user_args.remove(0);
        }
        // GH #904: everything else before the target is a BUILD
        // option, parsed by the parser `hale build` uses and
        // honored. `run` used to compile with `BuildOptions::
        // default()` no matter what was passed, so a build flag was
        // first read as the target, then (GH #900) named and
        // refused — and the documented spot-check `hale run
        // prog.hl` could exercise neither a dev build nor an FFI
        // program. One parser, so `build` and `run` cannot drift.
        let build_flags: Vec<String> = before
            .iter()
            .filter(|f| *f != "--observe")
            .cloned()
            .collect();
        let options = match parse_exec_build_options("run", &build_flags) {
            Ok(o) => o,
            Err(msg) => {
                eprintln!("{}", msg);
                eprintln!(
                    "(`hale run` takes its flags before the target; \
                     everything after the target is the program's argv)"
                );
                return ExitCode::from(2);
            }
        };
        if observe {
            // GH #887: `LOTUS_OBS=1` is for the PROGRAM, and it used
            // to be planted in this process's environment for the
            // child to inherit. `set_var` is undefined behaviour once
            // a process has threads, and `iris::spawn_session()` on
            // the next line starts one — so it travels on the child's
            // own `Command` instead, which is where it was always
            // meant to arrive. Nothing in this process reads it.
            let session = iris::spawn_session();
            let code = run_program(&target, &user_args, options, true);
            if let Some(mut s) = session {
                let _ = s.kill();
                let _ = s.wait();
            }
            return code;
        }
        return run_program(&target, &user_args, options, false);
    }

    if args.len() < 3 {
        usage();
        return ExitCode::from(2);
    }
    let target = PathBuf::from(&args[2]);

    match cmd.as_str() {
        "lex" => run_lex_file(&target),
        "parse" => run_parse_file(&target),
        other => {
            eprintln!("unknown command: {}", other);
            usage();
            ExitCode::from(2)
        }
    }
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
