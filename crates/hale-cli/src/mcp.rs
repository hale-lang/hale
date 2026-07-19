//! `hale mcp` — a Model Context Protocol server in the hale binary
//! (2026-07-19; replaces the separate Node hale-mcp).
//!
//! Transport: MCP stdio — newline-delimited JSON-RPC 2.0, one
//! message per line. Methods handled: initialize, ping,
//! tools/list, tools/call; notifications are ignored; unknown
//! requests answer an empty result so hosts don't hang.
//!
//! Two kinds of tools, both drift-proof by construction:
//!   * toolchain tools (check/verify/build/run/test/bench/fmt/
//!     doc/fetch) SELF-EXEC this very binary — the tool list and
//!     the CLI it describes are the same executable, so they
//!     cannot version-skew (the failure mode that made the old
//!     Node server advertise an interpreter that no longer
//!     existed and a formatter that didn't yet).
//!   * analysis tools (bus_graph/placement/enforcement/
//!     alloc_summary) call the hale-lsp crate directly — the same
//!     ~10 ms seed re-analysis the LSP's custom requests run.
//!
//! `hale_docs_search` greps the language spec EMBEDDED in the
//! binary (build.rs include_str's spec/*.md — 864 KB), so an
//! installed hale grounds language rules with no sibling checkout.
//!
//! Sandbox: when HALE_MCP_ROOT is set, every path argument must
//! resolve under it or the call is rejected — hosts can grant
//! "the hale tools" without granting arbitrary command execution.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{json, Value};

include!(concat!(env!("OUT_DIR"), "/spec_embed.rs"));

pub fn run_mcp() -> ExitCode {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF — host went away
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned();
        // Notifications (no id) are consumed silently.
        let Some(id) = id else { continue };

        let result = match method {
            "initialize" => {
                let proto = msg
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2024-11-05");
                json!({
                    "protocolVersion": proto,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "hale",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tool_list() }),
            "tools/call" => {
                let name = msg
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = msg
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match dispatch(name, &args) {
                    Ok((text, is_error)) => json!({
                        "content": [{ "type": "text", "text": text }],
                        "isError": is_error
                    }),
                    Err(e) => json!({
                        "content": [{ "type": "text",
                                       "text": format!("error: {}", e) }],
                        "isError": true
                    }),
                }
            }
            _ => Value::Null,
        };
        let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = writeln!(writer, "{}", resp);
        let _ = writer.flush();
    }
    ExitCode::SUCCESS
}

fn path_schema(desc: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": desc }
        },
        "required": ["path"]
    })
}

fn tool_list() -> Vec<Value> {
    vec![
        json!({
            "name": "hale_check",
            "description": "Type-check a Hale file or directory (parse + typecheck + advisory analyses, ~10 ms, no binary). The fast oracle: 'ok' or precise diagnostics. json: true emits one JSON object per diagnostic.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "json": { "type": "boolean" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "hale_verify",
            "description": "The Layer-2 discipline gate: hale check's exact analysis, but ANY finding (advisory or error) fails. What CI runs.",
            "inputSchema": path_schema("File or directory (a Hale seed = one directory).")
        }),
        json!({
            "name": "hale_build",
            "description": "Build a Hale file or directory to a native binary (lands next to the source, named after it).",
            "inputSchema": path_schema("File or directory to build.")
        }),
        json!({
            "name": "hale_run",
            "description": "Compile to a native binary and execute it (same codegen as hale_build — there is no interpreter). Optional args forward to the program's argv.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "hale_test",
            "description": "Compile + run *_test.hl files (pass = exit 0 + silent). Optional run substring filters; json: true for structured results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "run": { "type": "string" },
                    "json": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "hale_bench",
            "description": "Run *_bench.hl benchmarks (zero-param bench_* fns; self-calibrating; reports ns/op + allocs/op). json: true for records.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "run": { "type": "string" },
                    "json": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "hale_fmt",
            "description": "Canonical formatter (zero config). Formats in place; check: true lists files that would change without writing (a finding, not an error); diff: true previews.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "check": { "type": "boolean" },
                    "diff": { "type": "boolean" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "hale_doc",
            "description": "API reference from /// doc comments (Markdown; json: true for records). stdlib: true renders the std:: surface instead of a seed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "stdlib": { "type": "boolean" },
                    "json": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "hale_fetch",
            "description": "Fetch git dependencies declared in hale.toml into vendor/.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "hale_docs_search",
            "description": "Search the Hale language specification (embedded in this binary) for a substring; returns file:line snippets. Ground a rule before writing code.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": "number" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "hale_bus_graph",
            "description": "The seed's whole message topology: per subject, publishers, subscribers (locus + handler + placement), payload types, static-dispatch verdicts. One call instead of a grep session.",
            "inputSchema": path_schema("A file in the seed to analyze.")
        }),
        json!({
            "name": "hale_placement",
            "description": "The main locus's placement map — every params field with its resolved thread/pool assignment.",
            "inputSchema": path_schema("A file in the seed to analyze.")
        }),
        json!({
            "name": "hale_enforcement",
            "description": "Every user fn/method with its @hot / @budget / fallible / @unbounded contract — the certification map to consult before touching a hot path.",
            "inputSchema": path_schema("A file in the seed to analyze.")
        }),
        json!({
            "name": "hale_alloc_summary",
            "description": "The allocation-bound survey's leak sites with positions, plus the full text dump.",
            "inputSchema": path_schema("A file in the seed to analyze.")
        }),
    ]
}

/// Resolve + sandbox a path argument. HALE_MCP_ROOT (when set)
/// must contain the resolved path.
fn resolve_path(p: &str) -> Result<PathBuf, String> {
    let abs = std::fs::canonicalize(p)
        .unwrap_or_else(|_| PathBuf::from(p));
    if let Ok(root) = std::env::var("HALE_MCP_ROOT") {
        let root = std::fs::canonicalize(&root)
            .unwrap_or_else(|_| PathBuf::from(&root));
        if abs != root && !abs.starts_with(&root) {
            return Err(format!(
                "path escapes HALE_MCP_ROOT ({}): {}",
                root.display(),
                abs.display()
            ));
        }
    }
    Ok(abs)
}

/// Self-exec this binary with the given CLI args; returns
/// (combined output, nonzero-exit).
fn self_exec(args: &[String]) -> Result<(String, bool), String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("current_exe: {}", e))?;
    let out = std::process::Command::new(&exe)
        .args(args)
        .output()
        .map_err(|e| format!("exec: {}", e))?;
    let mut text = format!(
        "$ hale {}\n[{}]\n\n",
        args.join(" "),
        if out.status.success() {
            "ok".to_string()
        } else {
            format!("exit {}", out.status.code().unwrap_or(-1))
        }
    );
    let body = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let body = body.trim();
    text.push_str(if body.is_empty() { "(no output)" } else { body });
    Ok((text, !out.status.success()))
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn arg_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn dispatch(name: &str, args: &Value) -> Result<(String, bool), String> {
    match name {
        "hale_check" => {
            let p = resolve_path(arg_str(args, "path").ok_or("path required")?)?;
            let mut cli = vec!["check".into(), p.display().to_string()];
            if arg_bool(args, "json") {
                cli.push("--json".into());
            }
            self_exec(&cli)
        }
        "hale_verify" => {
            let p = resolve_path(arg_str(args, "path").ok_or("path required")?)?;
            self_exec(&["verify".into(), p.display().to_string()])
        }
        "hale_build" => {
            let p = resolve_path(arg_str(args, "path").ok_or("path required")?)?;
            self_exec(&["build".into(), p.display().to_string()])
        }
        "hale_run" => {
            let p = resolve_path(arg_str(args, "path").ok_or("path required")?)?;
            let mut cli = vec!["run".into(), p.display().to_string()];
            if let Some(a) = args.get("args").and_then(Value::as_array) {
                for v in a {
                    if let Some(s) = v.as_str() {
                        cli.push(s.to_string());
                    }
                }
            }
            self_exec(&cli)
        }
        "hale_test" | "hale_bench" => {
            let sub = if name == "hale_test" { "test" } else { "bench" };
            let mut cli = vec![sub.to_string()];
            if let Some(p) = arg_str(args, "path") {
                cli.push(resolve_path(p)?.display().to_string());
            }
            if let Some(r) = arg_str(args, "run") {
                cli.push("-run".into());
                cli.push(r.to_string());
            }
            if arg_bool(args, "json") {
                cli.push("--json".into());
            }
            self_exec(&cli)
        }
        "hale_fmt" => {
            let p = resolve_path(arg_str(args, "path").ok_or("path required")?)?;
            let mut cli = vec!["fmt".into()];
            let checking = arg_bool(args, "check");
            if checking {
                cli.push("--check".into());
            }
            if arg_bool(args, "diff") {
                cli.push("--diff".into());
            }
            cli.push(p.display().to_string());
            let (text, failed) = self_exec(&cli)?;
            // --check's would-change exit is a finding, not a tool
            // failure.
            Ok((text, failed && !checking))
        }
        "hale_doc" => {
            let mut cli = vec!["doc".into()];
            if arg_bool(args, "stdlib") {
                cli.push("--stdlib".into());
            } else if let Some(p) = arg_str(args, "path") {
                cli.push(resolve_path(p)?.display().to_string());
            }
            if arg_bool(args, "json") {
                cli.push("--json".into());
            }
            self_exec(&cli)
        }
        "hale_fetch" => {
            let mut cli = vec!["fetch".into()];
            if let Some(r) = arg_str(args, "repo_root") {
                cli.push(resolve_path(r)?.display().to_string());
            }
            self_exec(&cli)
        }
        "hale_docs_search" => {
            let query = arg_str(args, "query").ok_or("query required")?;
            let max = args
                .get("max_results")
                .and_then(Value::as_u64)
                .unwrap_or(12) as usize;
            let needle = query.to_lowercase();
            let mut hits = Vec::new();
            'outer: for (fname, text) in SPEC_FILES {
                for (i, line) in text.lines().enumerate() {
                    if line.to_lowercase().contains(&needle) {
                        hits.push(format!(
                            "{}:{}: {}",
                            fname,
                            i + 1,
                            line.trim()
                        ));
                        if hits.len() >= max {
                            break 'outer;
                        }
                    }
                }
            }
            let text = if hits.is_empty() {
                format!("No matches for \"{}\" in the spec.", query)
            } else {
                format!(
                    "Matches for \"{}\" in the embedded spec:\n\n{}",
                    query,
                    hits.join("\n")
                )
            };
            Ok((text, false))
        }
        "hale_bus_graph" | "hale_placement" | "hale_enforcement"
        | "hale_alloc_summary" => {
            let p = resolve_path(arg_str(args, "path").ok_or("path required")?)?;
            let v = match name {
                "hale_bus_graph" => hale_lsp::bus_graph_for_path(&p),
                "hale_placement" => hale_lsp::placement_for_path(&p),
                "hale_enforcement" => hale_lsp::enforcement_for_path(&p),
                _ => hale_lsp::alloc_summary_for_path(&p),
            };
            Ok((
                serde_json::to_string_pretty(&v)
                    .unwrap_or_else(|_| "{}".into()),
                false,
            ))
        }
        other => Err(format!("unknown tool: {}", other)),
    }
}
