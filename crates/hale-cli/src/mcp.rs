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

/// A JSON-RPC error: code and message.
pub struct RpcError(pub i64, pub String);

/// The stdio loop both servers share: one JSON-RPC request per line,
/// notifications consumed silently, `handle` answering each method
/// with a result or an error (an unknown method is `-32601`, so a
/// host learns it asked for something this server does not do).
pub fn serve_stdio(mut handle: impl FnMut(&str, &Value) -> Result<Value, RpcError>) -> ExitCode {
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
        // Notifications (no id) are consumed silently.
        let Some(id) = msg.get("id").cloned() else { continue };
        let resp = match handle(method, &msg) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(RpcError(code, message)) => json!({ "jsonrpc": "2.0", "id": id,
                "error": { "code": code, "message": message } }),
        };
        let _ = writeln!(writer, "{}", resp);
        let _ = writer.flush();
    }
    ExitCode::SUCCESS
}

fn method_not_found(method: &str) -> RpcError {
    RpcError(-32601, format!("method not found: {}", method))
}

pub fn run_mcp() -> ExitCode {
    serve_stdio(|method, msg| {
        Ok(match method {
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
            other => return Err(method_not_found(other)),
        })
    })
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
            "name": "hale_dna_work",
            "description": "A leg's verb against a DNA head's API (hale dna work): next (claim the next attempt for a position), brief (the hat, or --render prompt|text|agent), renew, allowance (ask the spine for the attempt's spend and wait for its answer), submit, settle, release, friction, run (one cycle through the project's performers), loop (worker mode, --parallel N; over MCP only with --once, so the call ends: one JSON line per child as it ends, then the loop's own). An external harness plugs in with next, brief --render agent, allowance before its first model call (it keeps its calls within what is granted), its own work, and submit --evidence-file. Positions are the graph's position:<name> ids. Every verb but loop prints one JSON object.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "verb": { "type": "string", "enum": ["next", "brief", "renew", "allowance", "submit", "settle", "release", "friction", "run", "loop"] },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "The verb's flags as given on the command line, e.g. [\"--as\", \"position:agent\", \"--api\", \"http://127.0.0.1:8793\"]." },
                    "project": { "type": "string", "description": "The project directory (default: the current one)." }
                },
                "required": ["verb"]
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
        "hale_dna_work" => {
            let verb = arg_str(args, "verb").ok_or("verb required")?;
            // a loop that runs until drained would hold this server forever
            // and be orphaned with it: over MCP a loop is one pass
            if verb == "loop" {
                let bounded = args
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().any(|v| v.as_str() == Some("--once") || v.as_str() == Some("--drain")))
                    .unwrap_or(false);
                if !bounded {
                    return Err("loop over MCP needs --once (each worker runs one task, then the call ends) or --drain; an unbounded loop belongs to a terminal".to_string());
                }
            }
            let mut cli = vec!["dna".to_string(), "work".to_string()];
            if let Some(p) = arg_str(args, "project") {
                cli.push(resolve_path(p)?.display().to_string());
            }
            cli.push(verb.to_string());
            if let Some(a) = args.get("args").and_then(Value::as_array) {
                for v in a {
                    if let Some(s) = v.as_str() {
                        cli.push(s.to_string());
                    }
                }
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


// ---- GH #1107, #1417 (R4 C): a served exposure as tools ----

/// The tools of a description: a tool per member the caller may call,
/// named as `hale check --api --mcp` names it (`surface_doc::tool_name`),
/// its input the request's schema made self-contained (the types it
/// reaches under `$defs`), `x-hale-requires` the roles. A member whose
/// request is not an object has no tool, as over `mcp::Rpc`: the wrapped
/// form needs the handler's parameter name, which a description does not
/// carry.
fn tools_of(doc: &Value) -> Vec<(String, String, Value)> {
    let schemas = doc.get("schemas").and_then(Value::as_object);
    let surface = doc.get("surface").and_then(Value::as_str).unwrap_or("the surface");
    let mut tools = Vec::new();
    for m in doc.get("members").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = m.get("name").and_then(Value::as_str) else { continue };
        let Some(req) = m.get("request") else { continue };
        let Some(input) = self_contained(req, schemas) else { continue };
        let requires: Vec<&str> = m
            .get("requires")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let needs = if requires.is_empty() { "Requires no role".to_string() } else { format!("Requires {}", requires.join(", ")) };
        let tool_name = hale_types::surface_doc::tool_name(name);
        let tool = json!({
            "name": tool_name,
            "description": format!(
                "rpc {} of {}. {} under the exposure's role source; the server still authorizes every request.",
                name, surface, needs
            ),
            "inputSchema": input,
            "x-hale-requires": requires,
        });
        tools.push((tool_name, name.to_string(), tool));
    }
    tools
}

/// `#/schemas/T` references rewritten to `#/$defs/T`.
fn defs_refs(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| match (k.as_str(), x.as_str().and_then(|s| s.strip_prefix("#/schemas/"))) {
                    ("$ref", Some(t)) => (k.clone(), json!(format!("#/$defs/{}", t))),
                    _ => (k.clone(), defs_refs(x)),
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(defs_refs).collect()),
        other => other.clone(),
    }
}

/// The `#/$defs/T` names a schema refers to.
fn def_names(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let Some(t) = m.get("$ref").and_then(Value::as_str).and_then(|s| s.strip_prefix("#/$defs/")) {
                out.push(t.to_string());
            }
            m.values().for_each(|x| def_names(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| def_names(x, out)),
        _ => {}
    }
}

/// A request schema as an MCP input schema: an object whose referenced
/// types ride in `$defs`; `None` for a request that is not an object.
fn self_contained(req: &Value, schemas: Option<&serde_json::Map<String, Value>>) -> Option<Value> {
    let root_name = req.get("$ref").and_then(Value::as_str).and_then(|s| s.strip_prefix("#/schemas/"));
    let mut root = match root_name {
        Some(n) => defs_refs(schemas?.get(n)?),
        None => defs_refs(req),
    };
    if root.get("type") != Some(&json!("object")) {
        return None;
    }
    let mut defs = serde_json::Map::new();
    let mut todo = Vec::new();
    def_names(&root, &mut todo);
    while let Some(t) = todo.pop() {
        if defs.contains_key(&t) || Some(t.as_str()) == root_name {
            continue;
        }
        let Some(s) = schemas.and_then(|m| m.get(&t)) else { continue };
        let s = defs_refs(s);
        def_names(&s, &mut todo);
        defs.insert(t, s);
    }
    if !defs.is_empty() {
        root.as_object_mut()?.insert("$defs".to_string(), Value::Object(defs));
    }
    Some(root)
}

/// `hale mcp --app <endpoint> [--token T]`: the same stdio transport, but
/// the tools are the members of the exposure's description for the caller
/// the endpoint names. A tool call is one call through the endpoint,
/// naming the digest of the description read at start; its text is the
/// response, or the handler error or refusal with `isError`. An
/// `mcp://host:port` endpoint is an `mcp::Rpc` listener: its tools are its
/// own `tools/list` and the bridge forwards the JSON-RPC.
pub fn run_mcp_app(args: &[String]) -> ExitCode {
    use crate::api_client::{call, fetch_description, member_names, stream_topics, Endpoint, Kind};
    let mut args = args.to_vec();
    let token = match args.iter().position(|a| a == "--token") {
        Some(i) if i + 1 < args.len() => {
            let t = args.remove(i + 1);
            args.remove(i);
            Some(t)
        }
        Some(_) => {
            eprintln!("hale mcp --app: --token requires a value");
            return ExitCode::from(2);
        }
        None => std::env::var("HALE_API_TOKEN").ok().filter(|t| !t.is_empty()),
    };
    let [target] = args.as_slice() else {
        eprintln!("usage: hale mcp --app <endpoint> [--token T]");
        return ExitCode::from(2);
    };
    let ep = match Endpoint::parse(target) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hale mcp --app: {}", e);
            return ExitCode::from(2);
        }
    };
    if let Endpoint::Mcp(hp) = &ep {
        return bridge_mcp(hp, token.as_deref());
    }
    let raw = match fetch_description(&ep, token.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hale mcp --app: {}", e);
            return ExitCode::from(1);
        }
    };
    let doc: Value = match serde_json::from_str(&raw) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hale mcp --app: the description is not JSON: {}", e);
            return ExitCode::from(1);
        }
    };
    let tools = tools_of(&doc);
    let list: Vec<Value> = tools.iter().map(|t| t.2.clone()).collect();
    let digest = doc.get("digest").and_then(Value::as_str).map(str::to_string);
    let exposure = doc.get("exposure").and_then(Value::as_str).unwrap_or("the exposure").to_string();
    let streams = stream_topics(&doc);
    let skipped: Vec<String> = member_names(&doc)
        .into_iter()
        .filter(|m| !tools.iter().any(|t| &t.1 == m))
        .collect();
    let instructions = format!(
        "{} over {}. Tools are the members this caller may call; the server still authorizes every request. {}{}",
        exposure,
        ep.show(),
        if streams.is_empty() { String::new() } else { format!("Streams ({}) have no tool: `hale watch` tails one. ", streams.join(", ")) },
        if skipped.is_empty() { String::new() } else { format!("No tool for {} (a request that is not an object).", skipped.join(", ")) },
    );
    serve_stdio(move |method, msg| {
        Ok(match method {
            "initialize" => {
                let proto = msg
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2024-11-05");
                json!({
                    "protocolVersion": proto,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": format!("hale-app:{}", exposure), "version": env!("CARGO_PKG_VERSION") },
                    "instructions": instructions
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": list }),
            "tools/call" => {
                let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let args = msg.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({}));
                let text_of = |t: String, is_error: bool| json!({ "content": [{ "type": "text", "text": t }], "isError": is_error });
                match tools.iter().find(|t| t.0 == name) {
                    None => text_of(format!("`{}` is not a tool of {}", name, exposure), true),
                    Some((_, member, _)) => match call(&ep, token.as_deref(), member, &args.to_string(), digest.as_deref()) {
                        Ok(r) if r.kind == Kind::Result => text_of(r.body, false),
                        Ok(r) => text_of(r.failure_text(), true),
                        Err(e) => text_of(e, true),
                    },
                }
            }
            other => return Err(method_not_found(other)),
        })
    })
}

/// stdio to an `mcp::Rpc` listener: `tools/list` and `tools/call` go to
/// `POST /mcp` as they are; the answer's result (or error) comes back
/// under the host's request id.
fn bridge_mcp(hp: &str, token: Option<&str>) -> ExitCode {
    use crate::api_client::http_request;
    let hp = hp.to_string();
    let token = token.map(str::to_string);
    serve_stdio(move |method, msg| {
        if !matches!(method, "initialize" | "ping" | "tools/list" | "tools/call") {
            return Err(method_not_found(method));
        }
        let mut m = json!({ "jsonrpc": "2.0", "id": 1, "method": method });
        if let Some(p) = msg.get("params") {
            m["params"] = p.clone();
        }
        let headers: Vec<(&str, String)> = token.iter().map(|t| ("Authorization", format!("Bearer {}", t))).collect();
        let r = http_request(&hp, "POST", "/mcp", &headers, Some(&m.to_string())).map_err(|e| RpcError(-32000, e))?;
        let v: Value = serde_json::from_str(r.body.trim())
            .map_err(|e| RpcError(-32000, format!("mcp::Rpc answered a body that is not JSON: {}", e)))?;
        match v.get("error") {
            Some(e) => Err(RpcError(
                e.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                format!(
                    "{}{}",
                    e.get("message").and_then(Value::as_str).unwrap_or("error"),
                    e.get("data").map(|d| format!(": {}", d)).unwrap_or_default()
                ),
            )),
            None => Ok(v.get("result").cloned().unwrap_or_else(|| json!({}))),
        }
    })
}
