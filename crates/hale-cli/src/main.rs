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

mod lsp;
mod pkg;

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
    eprintln!("    hale run   <file.hl | dir>    compile + run as a native binary");
    eprintln!("    hale build <file.hl | dir>    parse + typecheck + emit native binary");
    eprintln!("    hale test  [file | dir]       compile + run *_test.hl (default: cwd)");
    eprintln!("        [-run <substr>] [--json]");
    eprintln!("    hale fmt   [file | dir] ...   canonical formatter (default: cwd)");
    eprintln!("        [--check] [--diff] [--stdin]");
    eprintln!("    hale doc   [file | dir]       render the seed's API reference (/// doc comments)");
    eprintln!("        [--json] [-o <path>]");
    eprintln!("    hale fetch [repo-root]        fetch git deps from hale.toml into vendor/");
    eprintln!("    hale lsp                      stdio Language Server (diagnostics)");
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
    let mut out_path: Option<PathBuf> = None;
    let mut target: Option<PathBuf> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--json" => {
                json = true;
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
            )?;
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
    let mut merged_items = entry_program.items;
    let mut renames: ImportRenames = Vec::new();

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
    )
    .is_err()
    {
        return Err(errors);
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    let mut merged = Program {
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

fn run_check(target: &Path) -> ExitCode {
    let files = match collect_ap_files(target) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };
    let (mut programs, sources, file_bases) = match parse_files(&files) {
        Ok(x) => x,
        Err(code) => return code,
    };

    // FUv0.8.2 #4: auto-apply sync inference before typecheck so
    // `hale check` validates the post-inference shape the build
    // path will see. Without this, `check` warns on
    // auto-inferable cross-pool calls while `build` silently
    // applies — same source, divergent answers.
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
            return ExitCode::from(1);
        }
    }

    let bundle_programs: BTreeMap<String, &Program> = programs
        .iter()
        .map(|(p, prog)| (p.display().to_string(), prog))
        .collect();
    let bundle = hale_types::Bundle {
        programs: bundle_programs,
    };
    // GH #18 item 1 (step 1): dump the per-method allocation summary +
    // call graph and exit. A diagnostic view of the scaffold; no
    // bound-proving yet.
    if std::env::args().any(|a| a == "--dump-alloc-summary") {
        print!("{}", hale_types::dump_alloc_summary(&bundle));
        return ExitCode::SUCCESS;
    }
    // GH #18 item 5: dump the per-program resource budget (pinned threads,
    // cooperative pools, bus subjects) and exit.
    if std::env::args().any(|a| a == "--dump-resource-budget") {
        print!("{}", hale_types::dump_resource_budget(&bundle));
        return ExitCode::SUCCESS;
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
                    return ExitCode::from(1);
                }
            };
            let ct: CeilingToml = match toml::from_str(&text) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("--check-resource-budget: invalid budget file `{}`: {}", path, e);
                    return ExitCode::from(1);
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
                return ExitCode::SUCCESS;
            }
            for v in &violations {
                eprintln!(
                    "resource budget exceeded: {} — raise the ceiling in `{}` if intentional",
                    v, path
                );
            }
            return ExitCode::from(1);
        }
    }
    let allow_unowned =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    let mut diags = hale_types::check_bundle_opts(&bundle, allow_unowned);
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
    // #8 LSP groundwork (2026-07-02): `hale check --json` emits
    // NDJSON diagnostics on STDOUT (one object per line: file,
    // line, col, severity, kind, message) for editor/LSP
    // consumption. The human rendering stays on stderr otherwise.
    // With `hale check` at ~10 ms on the largest apps, an
    // on-save/on-keystroke loop needs nothing more than this.
    let json_mode = std::env::args().any(|a| a == "--json");
    if !diags.is_empty() {
        for d in &diags {
            if json_mode {
                println!("{}", render_diag_json(d, &file_bases, &sources));
            } else {
                eprintln!("{}", render_located(d, &file_bases, &sources));
            }
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    if !json_mode {
        eprintln!("ok: {} file(s) typechecked", files.len());
    }
    ExitCode::SUCCESS
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
    let bundle = hale_types::Bundle { programs: bundle_programs };
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
        let bundle = hale_types::Bundle { programs: bundle_programs };
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
    let mut merged_items = merged.items;
    let mut renames: ImportRenames = Vec::new();
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
    let bundle = hale_types::Bundle {
        programs: bundle_programs,
    };
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
    let bundle = hale_types::Bundle {
        programs: bundle_programs,
    };
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
    let output = if options.target == hale_codegen::CompileTarget::Wasm32 {
        output.with_extension("wasm")
    } else {
        output
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
            eprintln!("codegen error: {}", e);
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
                    "--target requires a value (native|wasm32)".to_string()
                })?;
                opts.target = match v.as_str() {
                    "native" => hale_codegen::CompileTarget::Native,
                    "wasm32" | "wasm" => hale_codegen::CompileTarget::Wasm32,
                    other => {
                        return Err(format!(
                            "--target: unknown target `{}` (expected native|wasm32)",
                            other
                        ));
                    }
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
                    "baseline" => hale_codegen::TargetCpu::Baseline,
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
