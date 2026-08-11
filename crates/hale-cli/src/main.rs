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
mod mcp;
mod pkg;
mod sign;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        return ExitCode::from(2);
    }
    let cmd = &args[1];

    if cmd == "--version" || cmd == "-V" || cmd == "version" {
        println!("hale {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if cmd == "--help" || cmd == "-h" || cmd == "help" {
        usage();
        return ExitCode::SUCCESS;
    }

    // Which targets exist, and what the compiler can actually do with
    // each. Naming a target and building it are different capabilities,
    // so the listing states the tier rather than implying parity.
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
    eprintln!("    hale check <file.hl | dir>    parse + typecheck");
    eprintln!("        [--dump-topology[=<path>]] [--check-topology[-shape] <path>]");
    eprintln!("        [--dump-effects-manifest] [--json] [--workspace]");
    eprintln!("        (`hale check --help` for all)");
    eprintln!("    hale verify <file.hl | dir>   check + FAIL on any advisory (discipline gate)");
    eprintln!("    hale run   <file.hl | dir>    compile + run as a native binary");
    eprintln!("    hale build <file.hl | dir>    parse + typecheck + emit native binary");
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
    eprintln!("    hale --version               print the version");
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
                        .map(|(p, d, src)| {
                            format!("{}: {}", p.display(), d.render(src))
                        })
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

fn resolve_import(
    importer_dir: &Path,
    workspace_root: Option<&Path>,
    import_path: &str,
) -> Option<ImportTarget> {
    let single = importer_dir.join(format!("{}.hl", import_path));
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

/// Collect every `.hl` file at an import target. SingleFile
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
    errors: &mut Vec<(PathBuf, hale_syntax::Diag, String)>,
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
                    eprintln!(
                        "could not read imported file {} (from import \"{}\"): {}",
                        file.display(),
                        imp.path,
                        e
                    );
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
                        errors.push((file.clone(), d, source.clone()));
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
            let cache_key = match &target {
                ImportTarget::Directory(d) => {
                    d.canonicalize().unwrap_or_else(|_| d.clone())
                }
                ImportTarget::SingleFile(f) => {
                    f.canonicalize().unwrap_or_else(|_| f.clone())
                }
            };
            if let Some(cached) = seed_cache.get(&cache_key) {
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
        {
            let cache_key = match &target {
                ImportTarget::Directory(d) => {
                    d.canonicalize().unwrap_or_else(|_| d.clone())
                }
                ImportTarget::SingleFile(f) => {
                    f.canonicalize().unwrap_or_else(|_| f.clone())
                }
            };
            seed_cache.insert(cache_key, seed_renames.clone());
        }
        if trace {
            eprintln!("[import]     build_seed_renames done (n={})", seed_renames.len());
        }
        // Mangle each file's program with the shared map.
        for pf in parsed_files.iter_mut() {
            if trace {
                eprintln!("[import]     mangle start: {}", pf.path.display());
            }
            hale_codegen::mangle::mangle_with_renames(&mut pf.program, &seed_renames);
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
    Vec<(PathBuf, hale_syntax::Diag, String)>,
> {
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut errors: Vec<(PathBuf, hale_syntax::Diag, String)> = Vec::new();
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
                errors.push((entry.to_path_buf(), d, entry_source.clone()));
            }
            return Err(errors);
        }
    };
    visited.insert(entry_canon.clone());
    // The entry file occupies base 0 (parse_source above = no shift);
    // imported files get subsequent virtual bases in resolve_imports.
    let mut file_bases: Vec<(u32, PathBuf, u32)> =
        vec![(0, entry_canon.clone(), entry_source.len() as u32)];
    sources.insert(entry_canon, entry_source);

    let entry_imports = entry_program.imports.clone();
    let mut effects = EffectTable::from_seed(&entry_program);
    let mut merged_items = entry_program.items;
    // Seed the merged table with the ENTRY's classes so the entry's
    // own `User(i)` indices stay identity — its items are already in
    // `merged_items` and are never walked.
    let mut renames: ImportRenames = Vec::new();
    let mut seed_cache: BTreeMap<PathBuf, std::collections::HashMap<String, String>> = BTreeMap::new();

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


/// Render a post-merge diagnostic, demultiplexing its (globally-unique,
/// `parse_source_at`-shifted) span back to the file it came from via
/// `file_bases`, so the output reads `path:line:col` against that file's
/// own source instead of an arbitrary file. Falls back to the entry
/// source if the span isn't in any known file range.
fn render_located(
    d: &hale_syntax::Diag,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    let off = d.span.start.as_usize() as u32;
    for (base, path, len) in file_bases {
        if off >= *base && off < base.saturating_add(*len) {
            if let Some(src) = sources.get(path) {
                return d.render_located(&path.display().to_string(), src, *base);
            }
        }
    }
    let any = sources.values().next().map(|s| s.as_str()).unwrap_or("");
    d.render(any)
}

fn parse_files(
    files: &[PathBuf],
) -> Result<
    (
        BTreeMap<PathBuf, Program>,
        BTreeMap<PathBuf, String>,
        Vec<(u32, PathBuf, u32)>,
    ),
    ExitCode,
> {
    let mut programs: BTreeMap<PathBuf, Program> = BTreeMap::new();
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    // (virtual base, path, len) — each file parsed at a distinct base so
    // merged spans demultiplex back to their file (see parse_source_at).
    let mut file_bases: Vec<(u32, PathBuf, u32)> = Vec::new();
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
        let base = file_bases.last().map(|(b, _, l)| b + l + 1).unwrap_or(0);
        file_bases.push((base, f.clone(), source.len() as u32));
        match hale_syntax::parse_source_at(&source, base) {
            Ok(p) => {
                programs.insert(f.clone(), p);
                sources.insert(f.clone(), source);
            }
            Err(diags) => {
                for d in &diags {
                    eprintln!("{}", d.render_located(&f.display().to_string(), &source, base));
                }
                had_error = true;
            }
        }
    }
    if had_error {
        return Err(ExitCode::from(1));
    }
    Ok((programs, sources, file_bases))
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
    u8,
> {
    let files = match collect_ap_files(target) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return Err(1);
        }
    };
    // `parse_files` still reports an `ExitCode` (its other two
    // callers hand one straight back); it only ever fails with 1.
    let (programs, sources, file_bases) =
        parse_files(&files).map_err(|_| 1u8)?;

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
            eprintln!("no .hl files in {}", target.display());
            return Err(1);
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
    let mut errors: Vec<(PathBuf, hale_syntax::Diag, String)> = Vec::new();
    let importer_dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    if resolve_imports(
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
    )
    .is_err()
    {
        for (path, d, src) in &errors {
            eprintln!("{}:", path.display());
            eprintln!("  {}", d.render(src));
        }
        return Err(1);
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
fn run_fleet_all() -> ExitCode {
    let cwd =
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
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
        return run_fleet_all();
    }
    let (sub, plan) = match (sub, plan) {
        (Some("check"), Some(p)) | (Some("dump"), Some(p)) => {
            (sub.unwrap(), p)
        }
        _ => {
            eprintln!("hale fleet check [plan.json]   compose and check");
            eprintln!("                                (no plan: every fleet in [fleets])");
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
    let (_d, _o, ids) = hale_types::claims::claims_report_with_identities(
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
            Err(code) => return code,
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
        let pre_diags = hale_types::apply_sync_inference(prog);
        if !pre_diags.is_empty() {
            let any_source = sources.values().next().map(|s| s.as_str()).unwrap_or("");
            for d in &pre_diags {
                eprintln!("{}", d.render(any_source));
            }
            return 1;
        }
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
    let checked = hale_types::check_bundle_opts(&bundle, allow_unowned);

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
        if off >= *base && off < base.saturating_add(*len) {
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
    format!(
        "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"severity\":\"{}\",\"kind\":\"{}\",\"message\":\"{}\"}}",
        esc(&file),
        line,
        col,
        severity,
        esc(d.kind_str()),
        esc(&d.message)
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
) -> ExitCode {
    let mut bin = std::env::temp_dir();
    let mut h = DefaultHasher::new();
    h.write_usize(program.items.len());
    h.write_u32(std::process::id());
    bin.push(format!("hale_run_{:016x}", h.finish()));
    if let Err(e) =
        hale_codegen::build_executable_with_imports(program, &bin, renames)
    {
        eprintln!("build error: {:?}", e);
        return ExitCode::from(1);
    }
    let status = std::process::Command::new(&bin).args(user_args).status();
    let _ = std::fs::remove_file(&bin);
    match status {
        Ok(s) => {
            ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8)
        }
        Err(e) => {
            eprintln!("could not execute compiled program: {}", e);
            ExitCode::from(1)
        }
    }
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
            for (path, d, src) in &errors {
                msg.push_str(&format!("{}: {}\n", path.display(), d.render(src)));
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
    let diags = hale_types::check_bundle_opts(&bundle, false);
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
        let diags = hale_types::check_bundle_opts(&bundle, allow_unowned);
        if !diags.is_empty() {
            for d in &diags {
                eprintln!("{}", render_located(d, &file_bases, &sources));
            }
            // Warnings print but don't fail the build; only errors do.
            if diags.iter().any(|d| d.is_error()) {
                return ExitCode::from(1);
            }
        }
        return compile_and_exec(&program, &renames, user_args);
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
        Err(code) => return code,
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
    let mut import_errors: Vec<(PathBuf, hale_syntax::Diag, String)> = Vec::new();
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
    )
    .is_err()
        || !import_errors.is_empty()
    {
        for (path, d, src) in &import_errors {
            eprintln!("{}:", path.display());
            eprintln!("  {}", d.render(src));
        }
        return ExitCode::from(1);
    }
    let mut program = Program {
        declared_effects: effects.declared_indices(),
        effect_defs: effects.defs,
        effect_names: effects.names,
        imports: Vec::new(),
        items: merged_items,
        span: merged.span,
    };
    // Rewrite qualified-path TypeExprs + synthesize JSON parsers +
    // apply sync inference before typecheck — the same pre-passes
    // `hale build <dir>` runs, so a directory `run` and `build`
    // agree.
    hale_codegen::mangle::apply_qualified_path_renames(&mut program, &renames);
    hale_syntax::json_gen::generate_json_parsers(&mut program);
    let pre_diags = hale_types::apply_sync_inference(&mut program);
    if !pre_diags.is_empty() {
        for d in &pre_diags {
            eprintln!("{}", render_located(d, &file_bases, &path_sources));
        }
        return ExitCode::from(1);
    }

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
    let diags = hale_types::check_bundle_opts(&bundle, allow_unowned);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &path_sources));
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    compile_and_exec(&program, &renames, user_args)
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
            Err(errors) => {
                for (path, d, src) in &errors {
                    eprintln!("{}:", path.display());
                    eprintln!("  {}", d.render(src));
                }
                return ExitCode::from(1);
            }
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
            Err(code) => return code,
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
        let mut import_errors: Vec<(PathBuf, hale_syntax::Diag, String)> = Vec::new();
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
        let mut with_imports = Program {
            declared_effects: effects.declared_indices(),
            effect_defs: effects.defs,
            effect_names: effects.names,
            imports: Vec::new(),
            items: merged_items,
            span: merged.span,
        };
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
    let pre_diags = hale_types::apply_sync_inference(&mut program);
    if !pre_diags.is_empty() {
        for d in &pre_diags {
            eprintln!("{}", render_located(d, &file_bases, &sources));
        }
        return ExitCode::from(1);
    }

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
    let diags = hale_types::check_bundle_opts(&bundle, allow_unowned);
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
