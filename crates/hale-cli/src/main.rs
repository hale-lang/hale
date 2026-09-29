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
use std::sync::atomic::{AtomicBool, Ordering};

use hale_syntax::ast::Program;

use hale_lsp as lsp;
use shared::imports::AliasScopes;
use shared::frontend::EntryCtx;
use shared::imports::FileClaims;
use shared::imports::ImportDiag;
use shared::imports::ImportRenames;
use shared::frontend::collect_ap_files;
use shared::frontend::collect_checkable;
use shared::frontend::merge_programs;
use shared::frontend::parse_files;
use shared::frontend::parse_with_imports;
use shared::imports::resolve_imports;
use shared::frontend::retain_owned_advisories;
use shared::imports::scope_import_aliases;
use shared::imports::unscoped_alias_uses;
use verbs::fmt::run_fmt;
use verbs::doc::run_doc;
use verbs::bench::run_bench;
use verbs::fleet::run_fleet;
use verbs::model::{diff_lines, model_usage, run_model_diff};
use verbs::init::run_init;
use verbs::help::{check_usage, subcommand_help, usage};
use shared::diag::{diag_file_name, json_escape, render_codegen_error, render_diag_json, render_flows, render_located, report_import_diags};
use shared::options::{CHECK_FLAGS, PER_SEED_FLAGS, VALUE_FLAGS, bind_build_env, collect_ffi_from_imports, exec_digest, flag_value_in, inject_adopt, model_identity, parse_build_options, parse_exec_build_options, resolve_build_env, resolve_env_constitution, split_target_args, take_output_flag};
use shared::process::{dies_with_us, wait_passing_signals, RunScratch};
use shared::stale::check_stale_cli;
use shared::workspace::{collect_seeds, find_workspace_root, seed_inputs};
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

/// GH #476 Change 2: did `hale model dump` ask for the canonical
/// model? The command is a shim into the check pipeline, and the
/// pipeline's dump section reads PROCESS argv (it is a
/// top-level-command scope), so the demand cannot ride the rest-args
/// the shim forwards.
///
/// GH #887: it used to travel as `HALE_DUMP_MODEL` in the
/// environment. `std::env::set_var` is undefined behaviour in a
/// process that has threads — the environment is one table with no
/// lock, and every `getenv` in flight races it — and this CLI starts
/// them (the LSP, the observation reader, a child's pipes). The
/// demand is one process-global bit read by one in-process consumer,
/// so it is one process-global bit: set by the shim before anything
/// else runs, read where the flag is read, and (unlike the env var)
/// not inherited by any child.
static MODEL_DUMP_DEMANDED: AtomicBool = AtomicBool::new(false);

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
        MODEL_DUMP_DEMANDED.store(true, Ordering::Relaxed);
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
            // GH #1109: every role the entrypoint declares is mapped
            // here (a `[]` is explicitly nobody), and nothing is
            // mapped that it does not declare.
            for msg in role_coverage(&target, env, &spec.roles) {
                eprintln!("{}", msg);
                failed.push(format!("{} @ {} (roles)", ep, env));
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

/// GH #1109: the role-coverage rule of `--matrix`, per (entrypoint,
/// environment): the roles the entrypoint declares (plus `owner`
/// when it has an api binding) against the environment's `roles`
/// table. A declared role the table omits is a failure — an omission
/// is indistinguishable from a mistake, and `[]` says "nobody" on
/// purpose; a mapped role nothing declares is one too, because a
/// misspelt key would otherwise map nobody quietly.
fn role_coverage(
    target: &Path,
    env: &str,
    table: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let Ok((programs, _, _, _, _)) = collect_checkable(target) else {
        return Vec::new();
    };
    let refs: Vec<&hale_syntax::ast::Program> = programs.values().collect();
    let declared = hale_syntax::api_gen::declared_roles(&refs);
    let mut out = Vec::new();
    let missing: Vec<&String> = declared.iter().filter(|r| !table.contains_key(*r)).collect();
    if !missing.is_empty() {
        out.push(format!(
            "{} @ {}: role(s) {} are not mapped in [environments.{}.roles] — map each to \
             its members, or to [] to say explicitly that nobody holds it here",
            target.display(),
            env,
            missing.iter().map(|r| format!("`{}`", r)).collect::<Vec<_>>().join(", "),
            env
        ));
    }
    let extra: Vec<&String> = table.keys().filter(|k| !declared.iter().any(|d| d == *k)).collect();
    if !extra.is_empty() && !declared.is_empty() {
        out.push(format!(
            "{} @ {}: [environments.{}.roles] maps {}, which the entrypoint does not declare \
             (declared: {})",
            target.display(),
            env,
            env,
            extra.iter().map(|r| format!("`{}`", r)).collect::<Vec<_>>().join(", "),
            declared.iter().map(|r| format!("`{}`", r)).collect::<Vec<_>>().join(", ")
        ));
    }
    out
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
    // GH #1106: the api binding is bundle-wide (the main locus in one
    // file, subscribers in another), so it runs over every program of
    // the seed once the per-program passes are done.
    {
        let mut refs: Vec<&mut Program> = programs.values_mut().collect();
        hale_syntax::api_gen::generate_api(&mut refs, None);
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
    // GH #1107: the api binding's description, from the checked
    // bundle. Same refusal rule as the artifact. A program with no
    // `api:` entry prints nothing and succeeds: there is nothing to
    // describe, and `hale describe` says so.
    let dump_api = argv.iter().any(|a| a == "--dump-api");
    let dump_api_to = argv
        .iter()
        .find_map(|a| a.strip_prefix("--dump-api="))
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    if dump_api || dump_api_to.is_some() {
        if let Some(d) = checked
            .iter()
            .find(|d| d.is_error() && d.kind != hale_syntax::error::DiagKind::Claim)
        {
            eprintln!(
                "refusing to emit an api description: `{}` does not \
                 typecheck, so its description would name a program \
                 that does not exist. Fix the {} first.",
                target.display(),
                d.kind_str()
            );
            return 1;
        }
        let programs: Vec<&Program> = bundle.programs.values().copied().collect();
        // The bytes the binding serves, never re-serialized (a `Value`
        // round trip would sort the keys; spec/model.md promises the
        // two documents agree byte for byte).
        let text = match hale_syntax::api_gen::api_surface(&programs) {
            Some(surface) => hale_syntax::api_gen::describe(&surface) + "\n",
            None => String::new(),
        };
        match &dump_api_to {
            Some(path) => {
                if let Err(e) = std::fs::write(path, &text) {
                    eprintln!("could not write {}: {}", path, e);
                    return 2;
                }
            }
            None => print!("{}", text),
        }
    }
    // GH #476 Change 2: the canonical-model demand surface. Same
    // refusal rule as the artifact — a model of a program that does
    // not typecheck describes nothing.
    if argv.iter().any(|a| a == "--dump-model")
        || MODEL_DUMP_DEMANDED.load(Ordering::Relaxed)
    {
        if let Some(d) = checked.iter().find(|d| {
            d.is_error()
                && d.kind != hale_syntax::error::DiagKind::Claim
        }) {
            eprintln!(
                "refusing to derive a model: `{}` does not typecheck, \
                 so its model is not a truthful description of \
                 any program. Fix the {} first.",
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
                "internal error: derived model violates a model law: \
                 {:?} (this is a hale bug — please report it)",
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
    // GH #736: which `release` clause makes a locus type a flow. Whether
    // `T` is a flow is decided over the whole program, imported seeds
    // included, so a child still reclaimed when its `run()` returns after
    // its own owner dropped the hook is answered here: every clause that
    // names its type, with the file and line.
    if std::env::args().any(|a| a == "--flows") {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        let flows = hale_types::flows::survey(&progs);
        eprint!("{}", render_flows(&flows, &file_bases, &sources, &import_renames));
    }
    if std::env::args().any(|a| a == "--strict-secret") {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        diags.extend(hale_types::frontier::secret_taint_strict(&progs));
    }
    // GH #730 / #1048: a borrow must outlive its holder — a handle stored
    // by name into a locus-carrying field, or kept by a method
    // (`Router.add`), is never the holder's to reclaim, so the frame, the
    // dispatch or the binding that owns it must last longer than the
    // holder. Errors, with the witness call site where a parameter carries
    // the handle in. Beside it the GH #737 notice. `build`, `run` and
    // `test` refuse the same through `check_bundle_for_build`.
    {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        diags.extend(hale_types::borrow_lifetime::borrow_lifetime_diags_with_renames(
            &progs,
            &bundle.import_renames,
        ));
    }
    // GH #738: a bare fallible stdlib call — no `or` — is a warning by
    // default and an error under `--strict-fallible`; the default
    // flips at the next minor. The typing of the bare call is
    // unchanged (the legacy form still builds); this is the notice.
    {
        let strict = std::env::args().any(|a| a == "--strict-fallible");
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        diags.extend(hale_types::bare_fallible::bare_fallible_calls(&progs, strict));
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

/// Compile `program` to a temporary native binary and execute it,
/// forwarding `user_args` as the program's trailing argv. This is
/// the whole of `hale run` — the same codegen backend as `hale
/// build`, so there is no `run`-vs-`build` behavioral divergence.
fn compile_and_exec(
    program: &Program,
    renames: &[(Vec<String>, String)],
    user_args: &[String],
    // `LOTUS_OBS=1` on the child: `hale run --observe` (GH #527 B3).
    observe: bool,
    model_hash: u64,
    exec_digest: [u64; 4],
    obs_entity_ids: Vec<hale_model::obs_ids::ObsEntityId>,
    // GH #848: the file table and texts the bundle was parsed with,
    // so a span-carrying codegen error is reported at its source
    // location exactly as `hale build` reports it.
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
    // GH #904: the options `hale run` was given, already
    // fingerprinted into `exec_digest` by the caller. They were
    // `BuildOptions::default()` here whatever the command line
    // said.
    options: hale_codegen::BuildOptions,
) -> ExitCode {
    let scratch = match RunScratch::new("run") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hale run: {e}");
            return ExitCode::from(1);
        }
    };
    let bin = scratch.path("program");
    let options = hale_codegen::BuildOptions {
        model_hash: Some(model_hash),
        exec_digest: Some(exec_digest),
        obs_entity_ids,
        ..options
    };
    if let Err(e) = hale_codegen::build_executable_with_options(
        program, &bin, renames, &options,
    ) {
        eprintln!("{}", render_codegen_error(&e, file_bases, sources));
        return ExitCode::from(1);
    }
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(user_args);
    if observe {
        cmd.env("LOTUS_OBS", "1");
    }
    // The program is `hale run`'s foreground work, not a daemon it
    // launches: it holds this command's stdin/stdout/stderr, and
    // `hale` exists to wait for it and report how it ended. A `hale`
    // killed out from under it leaves it running against a caller
    // that cannot see its output end — GH #905.
    dies_with_us(&mut cmd);
    let status = wait_passing_signals(&mut cmd);
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
fn compile_test_binary(
    entry: &Path,
    scratch: &RunScratch,
) -> Result<PathBuf, String> {
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
    let diags = hale_types::check_bundle_for_build(&bundle, false);
    if diags.iter().any(|d| d.is_error()) {
        let mut msg = String::new();
        for d in diags.iter().filter(|d| d.is_error()) {
            msg.push_str(&render_located(d, &file_bases, &sources));
            msg.push('\n');
        }
        return Err(msg.trim_end().to_string());
    }
    // GH #1009: files compile concurrently now, so the path must be
    // unique per build and not merely per file name. The pid keeps
    // two `hale test` processes apart; the process-local counter
    // keeps two builds of this one apart structurally (the hash of
    // the entry path alone could, in principle, collide). The `.o`
    // (and `.ll`/`.bc` under the dump knobs) codegen writes beside
    // it is derived from this path, so it is unique too.
    static TEST_BIN_NONCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let nonce = TEST_BIN_NONCE.fetch_add(1, Ordering::Relaxed);
    let mut h = DefaultHasher::new();
    h.write(entry.display().to_string().as_bytes());
    let bin = scratch.path(&format!("test_{}_{:016x}", nonce, h.finish()));
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
        // GH #848: the per-fixture failure message is the located
        // rendering `build` prints, so a test that will not compile
        // names the line to open — it used to be the `{:?}` of the
        // error, span struct and all.
        return Err(render_codegen_error(&e, &file_bases, &sources));
    }
    Ok(bin)
}

/// `-j N` / `--jobs N` / `HALE_TEST_JOBS=N`: a positive worker count.
fn parse_test_jobs(v: &str) -> Result<usize, ()> {
    match v.trim().parse::<usize>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(()),
    }
}

/// Stack for a `hale test` worker thread. The compiler's passes are
/// recursive (descent parser, typechecker, lowering) and a spawned
/// thread's default is 2 MiB, well under the main thread's (8 MiB
/// under the usual `ulimit -s`) that `hale build` compiles on; a large
/// program that builds there must not overflow on a worker. The
/// reservation is virtual — pages are committed only as the stack is
/// used.
const TEST_WORKER_STACK: usize = 256 << 20;

/// GH #1009: compile and run every file, up to `jobs` at once, and
/// hand the outcomes back in `files` order — the order the report
/// prints them in, whatever order the workers finished in.
///
/// Each worker takes the next index off a shared counter, so a slow
/// file holds up one worker, not the queue. What a worker touches is
/// its own: the test binary's path is unique per build (pid + counter,
/// `compile_test_binary`), codegen's intermediates derive from it, the
/// runtime-object cache writes through a unique temp name and an
/// atomic rename, and each test's stdout and stderr come back through
/// its own pipes (`Command::output`) into its own `TestOutcome` —
/// nothing a test prints reaches the terminal, so two tests cannot
/// interleave there. The runner changes no process-wide state: no
/// `set_current_dir`, no `set_var`; every child inherits the cwd and
/// the environment `hale test` was started with, as it did serially.
///
/// `jobs == 1` runs on the calling thread, one file after another —
/// exactly the loop this replaced.
fn run_test_files(files: &[PathBuf], jobs: usize) -> Vec<TestOutcome> {
    // One private directory for the whole run: every test binary, and
    // the objects codegen writes beside it, live in it and go with it
    // when it drops — after the last worker is done, on every path out.
    let scratch = match RunScratch::new("test") {
        Ok(s) => s,
        Err(e) => {
            return files
                .iter()
                .map(|f| TestOutcome {
                    file: f.to_path_buf(),
                    passed: false,
                    message: Some(e.clone()),
                    elapsed_ms: 0,
                })
                .collect();
        }
    };
    let scratch = &scratch;
    let jobs = jobs.min(files.len()).max(1);
    if jobs == 1 {
        return files
            .iter()
            .map(|f| run_one_test_file(f, scratch))
            .collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<TestOutcome>>> =
        files.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            std::thread::Builder::new()
                .name("hale-test-worker".to_string())
                .stack_size(TEST_WORKER_STACK)
                .spawn_scoped(s, || loop {
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    let Some(f) = files.get(idx) else { break };
                    let outcome = run_one_test_file(f, scratch);
                    *slots[idx].lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(outcome);
                })
                .expect("spawn a hale test worker thread");
        }
    });
    // A worker that panicked re-raises at the end of the scope above,
    // as the panic did on the serial loop, so every slot is filled here.
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("every test file has an outcome")
        })
        .collect()
}

/// A fresh vault directory for one test file's run (mode 700), under
/// `<tmp>/hale-test-vaults-<uid>/<pid>-<n>`: this user's root, this
/// process, a counter, so parallel files never share one. The root is
/// refused when it is not a directory this user owns (another user's, a
/// planted symlink), so a test's secrets never go where someone else can
/// reach them. The first call sweeps the vaults of processes that are
/// gone (a run killed before it removed its own).
fn test_vault_dir() -> Result<PathBuf, String> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let uid = 0;
    let root = std::env::temp_dir().join(format!("hale-test-vaults-{uid}"));
    let _ = std::fs::create_dir(&root);
    let meta = std::fs::symlink_metadata(&root).map_err(|e| format!("test vault root {}: {e}", root.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if !meta.is_dir() || meta.uid() != uid {
            return Err(format!("test vault root {} is not a directory of this user's", root.display()));
        }
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("test vault root {}: {e}", root.display()))?;
    }
    #[cfg(not(unix))]
    let _ = meta;
    if n == 0 {
        sweep_dead_test_vaults(&root);
    }
    let dir = root.join(format!("{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).map_err(|e| format!("test vault {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("test vault {}: {e}", dir.display()))?;
    }
    Ok(dir)
}

/// Remove each `<pid>` / `<pid>-<n>` vault under `root` whose process
/// no longer runs. The Rust tests' `support/vault.rs` keeps its vaults
/// under the same root, one per test module and process, and sweeps them
/// the same way.
fn sweep_dead_test_vaults(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(pid) = name.split('-').next().unwrap_or("").parse::<i32>() else { continue };
        #[cfg(unix)]
        let gone = pid > 0
            && unsafe { libc::kill(pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        #[cfg(not(unix))]
        let gone = false;
        if gone {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Compile and run one `_test.hl` file and judge it by the
/// `spec/testing.md` contract.
fn run_one_test_file(f: &Path, scratch: &RunScratch) -> TestOutcome {
    let start = std::time::Instant::now();
    let (passed, message) = match compile_test_binary(f, scratch) {
        Err(diag) => (false, Some(diag)),
        Ok(bin) => {
            let mut cmd = std::process::Command::new(&bin);
            // A test that builds a program (a child it signals, a
            // tool it drives) builds it with THIS toolchain, not
            // whichever `hale` PATH finds first; a caller's own
            // HALE_BIN wins (GH #1039).
            if std::env::var_os("HALE_BIN").is_none() {
                if let Ok(me) = std::env::current_exe() {
                    cmd.env("HALE_BIN", me);
                }
            }
            // Each test file runs with a vault of its own, made empty for
            // the run and removed after it: a test that provisions a secret,
            // or a fixture that runs `hale dna init` (which draws the
            // organism's), never writes into the developer's vault, and no
            // test reaches a real vault (`HALE_VAULT_ADDR`). Everything the
            // test starts inherits it.
            // `HALE_TEST_KEEP_VAULT=1` keeps it, and says where.
            let vault = match test_vault_dir() {
                Ok(v) => v,
                Err(why) => {
                    let _ = std::fs::remove_file(&bin);
                    return TestOutcome {
                        file: f.to_path_buf(),
                        passed: false,
                        message: Some(format!("no vault for the test: {why}")),
                        elapsed_ms: start.elapsed().as_millis(),
                    };
                }
            };
            cmd.env("HALE_VAULT_DIR", &vault).env_remove("HALE_VAULT_ADDR");
            let output = cmd.output();
            let _ = std::fs::remove_file(&bin);
            if std::env::var("HALE_TEST_KEEP_VAULT").is_ok_and(|v| v == "1") {
                eprintln!("hale test: {}'s vault is kept at {}", f.display(), vault.display());
            } else {
                let _ = std::fs::remove_dir_all(&vault);
            }
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
    TestOutcome {
        file: f.to_path_buf(),
        passed,
        message,
        elapsed_ms: start.elapsed().as_millis(),
    }
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
    let mut jobs: Option<usize> = None;
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
        } else if a == "-j" || a == "--jobs" {
            match args.get(i + 1).map(|v| parse_test_jobs(v)) {
                Some(Ok(n)) => {
                    jobs = Some(n);
                    i += 2;
                }
                Some(Err(())) => {
                    eprintln!(
                        "hale test: {} takes a positive integer, got `{}`",
                        a,
                        args[i + 1]
                    );
                    return ExitCode::from(2);
                }
                None => {
                    eprintln!("hale test: {} requires a job count", a);
                    return ExitCode::from(2);
                }
            }
        } else if let Some(v) = a
            .strip_prefix("--jobs=")
            .or_else(|| a.strip_prefix("-j="))
            .or_else(|| a.strip_prefix("-j").filter(|v| !v.is_empty()))
        {
            match parse_test_jobs(v) {
                Ok(n) => jobs = Some(n),
                Err(()) => {
                    eprintln!(
                        "hale test: -j/--jobs takes a positive integer, got `{}`",
                        v
                    );
                    return ExitCode::from(2);
                }
            }
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
    // GH #1009: `-j` wins, then `HALE_TEST_JOBS`, then one worker per
    // available core.
    let jobs = match jobs {
        Some(n) => n,
        None => match env::var("HALE_TEST_JOBS") {
            Ok(v) if !v.is_empty() => match parse_test_jobs(&v) {
                Ok(n) => n,
                Err(()) => {
                    eprintln!(
                        "hale test: HALE_TEST_JOBS takes a positive integer, got `{}`",
                        v
                    );
                    return ExitCode::from(2);
                }
            },
            _ => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        },
    };

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

    let outcomes = run_test_files(&files, jobs);

    let passed = outcomes.iter().filter(|o| o.passed).count();
    let failed = outcomes.len() - passed;

    if json {
        let mut buf = String::from("[");
        for (idx, o) in outcomes.iter().enumerate() {
            if idx > 0 {
                buf.push(',');
            }
            // GH #867: the row's `file` is spelled by the rule every
            // `--json` record's `file` is spelled by — canonical,
            // absolute, symlinks resolved, no `..` (GH #822). The row
            // says which test ran rather than where an error is, but
            // a tool joining these rows to `check --json` records on
            // `file` needs the two to agree, and this one carried the
            // command line's spelling verbatim. The PASS/FAIL lines
            // below keep that spelling: a human reads them beside the
            // command they just typed.
            buf.push_str(&format!(
                "{{\"file\":\"{}\",\"status\":\"{}\"",
                json_escape(&diag_file_name(&o.file)),
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
    // GH #904: the `hale build` options, in `replay`'s own hand —
    // a recording made under `hale run --dev` is admitted by `hale
    // replay --dev` and by nothing else, so the flag set has to be
    // reachable here too.
    let mut build_flags: Vec<String> = Vec::new();
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
                        "hale replay: --at takes N or consumer:N (positive)"
                    );
                    return ExitCode::from(2);
                }
            }
        } else if VALUE_FLAGS.contains(&a) {
            // A build option whose value is the next argv entry.
            build_flags.push(a.to_string());
            if let Some(v) = args.get(i + 1) {
                build_flags.push(v.clone());
            }
            i += 2;
        } else if a == "--dev" {
            build_flags.push(a.to_string());
            i += 1;
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
    // GH #904: one `BuildOptions`, from `hale build`'s parser, for
    // the fingerprint AND the compile below — a replay recompiles
    // the program, so it admits against what IT builds.
    let build_options =
        match parse_exec_build_options("replay", &build_flags) {
            Ok(o) => o,
            Err(msg) => {
                eprintln!("{}", msg);
                return ExitCode::from(2);
            }
        };
    let (rec_path, prog) = match (rec_arg, prog) {
        (Some(r), Some(p)) => (r, p),
        _ => {
            eprintln!(
                "usage: hale replay <recording> <program.hl> \
                 [--diff [--json]] \
                 [--at <n> | --at <consumer-id>:<ordinal>] \
                 [--allow-live-effects] [--allow-unverified-model] \
                 [--allow-truncated] [--feed] [--allow-unmatched-feed] \
                 [--dev] [--target-cpu <v>] [--link <lib>] [--csrc <f.c>]"
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
    let diags = hale_types::check_bundle_for_build(&bundle, false);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &sources));
        }
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let options_fp = build_env::options_fingerprint(&build_options);
    let (plan_digest, obs_ids) = model_identity(&bundle, &build_options);
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

    let scratch = match RunScratch::new("replay") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hale replay: {e}");
            return ExitCode::from(1);
        }
    };
    let bin = scratch.path("program");
    let options = hale_codegen::BuildOptions {
        model_hash: Some(model_hash),
        exec_digest: Some(digest),
        obs_entity_ids: obs_ids.clone(),
        ..build_options
    };
    if let Err(e) = hale_codegen::build_executable_with_options(
        &program, &bin, &renames, &options,
    ) {
        eprintln!("{}", render_codegen_error(&e, &file_bases, &sources));
        return ExitCode::from(1);
    }

    let verify_path = if diff {
        Some(scratch.path("verify.halerec"))
    } else {
        None
    };

    let status_path = scratch.path("status");
    // Pre-create 0600 so the child's fopen("w") inherits restrictive
    // permissions whoever else can list the scratch directory.
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

fn run_program(
    target: &Path,
    user_args: &[String],
    // GH #904: the build options this `run` was given — the same
    // set, from the same parser, that `hale build` takes. They are
    // fingerprinted into the execution identity below, so a
    // recording carries the options it was made under.
    mut options: hale_codegen::BuildOptions,
    // GH #527 B3 / GH #887: `--observe` publishes the program's
    // observation segment. It is the CHILD's setting, so it rides
    // down to the `Command` that starts the child rather than being
    // planted in this process's environment for it to inherit.
    observe: bool,
) -> ExitCode {
    // GH #1109: `--env` names the deployment target; its role table is
    // part of the binary (and so of the fingerprint below), so it is
    // resolved first. The constitution it binds is adopted once the
    // program is parsed.
    let env_spec = match resolve_build_env(target, &mut options) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
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
        let diags = hale_types::check_bundle_for_build(&bundle, allow_unowned);
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
        let options_fp = build_env::options_fingerprint(&options);
        let (plan_digest, obs_ids) = model_identity(&bundle, &options);
        let digest =
            exec_digest(&sources, target, &options_fp, plan_digest);
        return compile_and_exec(
            &program,
            &renames,
            user_args,
            observe,
            model_hash,
            digest,
            obs_ids,
            &file_bases,
            &sources,
            options,
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
    // GH #820: as on the entry path — the seed being built is not one
    // of the libraries its imports name.
    let mut claims: FileClaims = FileClaims::new();
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
        &mut claims,
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
    // GH #1106: `--api` injects the entry; the pass lowers it (and any
    // entry the source spelled) before the checker sees the program.
    if let Some(path) = &options.api {
        if let Err(msg) = hale_syntax::api_gen::inject_api_entry(&mut program, path) {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    }
    if let Err(msg) = bind_build_env(&mut program, &env_spec, &options) {
        eprintln!("{}", msg);
        return ExitCode::from(2);
    }
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
    let diags = hale_types::check_bundle_for_build(&bundle, allow_unowned);
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
    let options_fp = build_env::options_fingerprint(&options);
    let (plan_digest, obs_ids) = model_identity(&bundle, &options);
    let digest =
        exec_digest(&path_sources, target, &options_fp, plan_digest);
    compile_and_exec(
        &program,
        &renames,
        user_args,
        observe,
        model_hash,
        digest,
        obs_ids,
        &file_bases,
        &path_sources,
        options,
    )
}

fn run_build(target: &Path, flags: &[String]) -> ExitCode {
    let (flags_without_out, out_override) = match take_output_flag(flags) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let flags: &[String] = &flags_without_out;
    // Phase 2i: warn if the CLI binary was built against an older
    // codegen+runtime source tree than what's on disk now. Silent
    // miscompile (stale CLI emitting old lowering against new
    // source) is the worst failure mode for a cold-context agent —
    // see `apps/log-router/FRICTION.md` 2026-05-10. The check is
    // best-effort: it skips when source files aren't locatable
    // (installed binary, moved workspace), or when the user
    // explicitly opts out via `HALE_SKIP_STALE_CHECK=1`. Since GH #785
    // the dispatcher runs it for every source-reading command, this
    // one included, so it is not run again here.

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
        // GH #820: as on the entry path — the seed being run is not
        // one of the libraries its imports name.
        let mut claims: FileClaims = FileClaims::new();
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
            &mut claims,
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
    // Options first: the check answers target questions (GH #970), so
    // it has to know the target, and `--api` (GH #1106) shapes the
    // program before the bundle borrows it.
    let mut options = match parse_build_options("build", flags) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    // GH #1109: `--env` — the deployment target's role table and law.
    let env_spec = match resolve_build_env(target, &mut options) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    if let Some(path) = &options.api {
        if let Err(msg) = hale_syntax::api_gen::inject_api_entry(&mut program, path) {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    }
    if let Err(msg) = bind_build_env(&mut program, &env_spec, &options) {
        eprintln!("{}", msg);
        return ExitCode::from(2);
    }
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
    bundle.target_has_async_io = options.target.spec().has_async_io();
    bundle.target_label = options.target.spec().platform_label();
    let allow_unowned =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    let diags = hale_types::check_bundle_for_build(&bundle, allow_unowned);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &sources));
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    // P26: stamp the model identity of the bundle just checked into
    // the binary, for the observation segment header.
    options.model_hash =
        Some(hale_types::topology::model_shape_hash(&bundle));
    // GH #476 Change 8: the canonical entity ids a consumer joins
    // the live manifest to that model with, and the dispatch
    // plan's digest — held here and folded into the execution
    // identity once the options are FINAL (below), since the
    // fingerprint covers options that are still being set.
    let (plan_digest, obs_ids) = model_identity(&bundle, &options);
    options.obs_entity_ids = obs_ids;
    // WASM plan: a wasm build emits `<stem>.wasm` (a relocatable wasm
    // object at this stage) rather than the extension-less native binary.
    // Output naming is a property of the target, not a special case
    // spelled at this one call site (GH #445).
    let output = {
        // A foreign native target with no cross toolchain here ends at
        // its relocatable object (GH #970), so it is named as one.
        let spec = options.target.spec();
        let names = spec.filenames();
        let ext = if spec.links_from(&hale_codegen::target::TargetSpec::host()) {
            names.executable
        } else {
            names.object
        };
        let named = if ext.is_empty() {
            output
        } else {
            output.with_extension(ext)
        };
        // `-o` names the artifact exactly: no extension is added or
        // swapped, and its directory is made.
        match &out_override {
            Some(p) => {
                if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        eprintln!("hale build: cannot create {}: {e}", dir.display());
                        return ExitCode::from(1);
                    }
                }
                p.clone()
            }
            None => named,
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
        // (`--dev` / HALE_DEV — LLVM O1 instead of the O3 release
        // default — is set by `parse_build_options` since GH #904.
        // It was read from `env::args()` here, inside this branch,
        // so `LOTUS_NO_DEBUGINFO=1 hale build --dev` silently built
        // at O3 and no other caller of the parser saw the flag at
        // all.)
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
        &build_env::options_fingerprint(&options),
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
            if let hale_codegen::CompileTarget::Foreign(spec) = options.target {
                if !spec.links_from(&hale_codegen::target::TargetSpec::host()) {
                    eprintln!(
                        "note: a relocatable object for {}, not an executable — \
                         this host has no toolchain to link for that platform \
                         (GH #970)",
                        spec.triple
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            // GH #241 / GH #808 / GH #848: a span-carrying codegen
            // error renders like a check diagnostic (file:line:col +
            // source caret) and everything else keeps the bare line
            // — through the one helper every command that compiles
            // now reports through.
            eprintln!(
                "{}",
                render_codegen_error(&e, &file_bases, &sources)
            );
            ExitCode::from(1)
        }
    }
}
