//! `hale api import` and `hale api drift`: a surface the program consumes,
//! read from an OpenAPI document or an MCP endpoint's tool list, and the
//! check that the source still says what the program was written against
//! (spec/api.md § Imported surfaces).
//!
//! The import writes Hale source: an `api NAME from KIND "LOCATION" { … }`
//! block of signature rows and the records and enums they name. The drift
//! check reads the source again, maps it as the import would, and compares
//! the two row by row, by shape: each row's request, response and wire
//! name with every record the shapes name expanded to its fields' JSON
//! keys and types. A record's name is not its shape, so a row whose types
//! were renamed in the genome but still match is in step.

pub mod fetch;
pub mod mapping;
pub mod yaml;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hale_syntax::ast::{ApiDecl, Program, TopDecl, TypeDecl, TypeDeclBody, TypeExpr};

use mapping::{Kind, Surface};

/// A source read and mapped as the surface `name` would be.
pub fn read_surface(
    name: &str,
    kind: Option<Kind>,
    source: &str,
    location: &str,
    base: &Path,
) -> Result<Surface, String> {
    if fetch::is_url(source) {
        let mcp = || -> Result<Surface, String> {
            let (tools, server) = fetch::mcp_tools(source)?;
            mapping::from_mcp(&tools, &server, name, location)
        };
        return match kind {
            Some(Kind::Mcp) => mcp(),
            Some(Kind::OpenApi) => mapping::from_openapi(&fetch::get_json(source)?, name, location),
            // a document by GET when the URL serves one, else an MCP endpoint
            None => match fetch::get_json(source) {
                Ok(doc) if doc.get("openapi").is_some() || doc.get("swagger").is_some() => {
                    mapping::from_openapi(&doc, name, location)
                }
                got => mcp().map_err(|e| match got {
                    Ok(_) => format!("{source} serves JSON that is no OpenAPI document, and is no MCP endpoint: {e}"),
                    Err(g) => format!("{source} is neither an OpenAPI document ({g}) nor an MCP endpoint ({e})"),
                }),
            },
        };
    }
    if kind == Some(Kind::Mcp) {
        return Err(format!("`{source}`: an MCP endpoint is an http:// or https:// URL"));
    }
    let path = base.join(source);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let doc = fetch::parse_document(&text, &path.display().to_string())?;
    mapping::from_openapi(&doc, name, location)
}

/// `to` relative to the directory `from`, as the import records a file
/// source beside the block that names it.
fn relative(to: &Path, from: &Path) -> String {
    let to = std::fs::canonicalize(to).unwrap_or_else(|_| to.to_path_buf());
    let from = std::fs::canonicalize(from).unwrap_or_else(|_| from.to_path_buf());
    let a: Vec<_> = to.components().collect();
    let b: Vec<_> = from.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return to.display().to_string();
    }
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &a[common..] {
        out.push(c);
    }
    out.display().to_string()
}

pub(crate) fn import_usage() -> &'static str {
    "\
usage: hale api import <openapi-url-or-file | mcp-url> --as NAME [--out FILE] [--openapi | --mcp]

Reads an OpenAPI 3 document (a JSON file, or an http(s) URL) or an MCP endpoint's
tool list (its `tools/list` over HTTP) and writes the surface the program consumes:
an `api NAME from openapi|mcp \"...\" { ... }` block with a row per operation or tool,
named by its operationId (or its method and path) or the tool's name, and the records
its request and response name. A shape the api codec cannot carry is
std::json::Value, with a comment naming why. The file is FILE (`-` for stdout), by
default the surface's name in snake case with `.hl`, in the current directory. A
URL is read as an OpenAPI document when it serves one and as an MCP endpoint
otherwise; `--openapi` or `--mcp` says which. Exit 0 written, 1 the source could
not be read or mapped, 2 usage.
"
}

pub(crate) fn drift_usage() -> &'static str {
    "\
usage: hale api drift [file.hl | dir] [--surface NAME]

Reads each imported surface's source again (the `from` of its `api` block) and
compares it to the rows the program was written against, row by row. Each
difference is one line on stdout:
  drift SURFACE added ROW
  drift SURFACE removed ROW
  drift SURFACE changed ROW request|response|wire
and a surface in step prints `SURFACE: in step with LOCATION (N rows)`. Exit 0 in
step, 1 drift, 2 a source that could not be read or a usage error.
`hale check --api` runs the same check over the program it describes.
"
}

/// `hale api import`.
pub fn run_import(rest: &[String]) -> ExitCode {
    let mut source = None;
    let mut name = None;
    let mut out = None;
    let mut kind = None;
    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].as_str();
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            rest.get(*i).cloned()
        };
        match a {
            "--as" => name = take(&mut i),
            "--out" => out = take(&mut i),
            "--openapi" => kind = Some(Kind::OpenApi),
            "--mcp" => kind = Some(Kind::Mcp),
            f if f.starts_with("--") => {
                eprintln!("hale api import: unknown flag `{f}`");
                eprint!("{}", import_usage());
                return ExitCode::from(2);
            }
            s if source.is_none() => source = Some(s.to_string()),
            s => {
                eprintln!("hale api import: one source at a time; `{s}` is a second");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let (Some(source), Some(name)) = (source, name) else {
        eprint!("{}", import_usage());
        return ExitCode::from(2);
    };
    if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) || !name.chars().all(|c| c.is_ascii_alphanumeric())
    {
        eprintln!("hale api import: `--as {name}`: a surface is named in UpperCamel case (`Petstore`)");
        return ExitCode::from(2);
    }
    let out_path = match out.as_deref() {
        Some("-") => None,
        Some(p) => Some(PathBuf::from(p)),
        None => Some(PathBuf::from(format!("{}.hl", snake_file(&name)))),
    };
    let out_dir = out_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|d| if d.as_os_str().is_empty() { Path::new(".") } else { d })
        .unwrap_or(Path::new("."))
        .to_path_buf();
    // the directory the file goes in, made first: a file source is
    // recorded relative to it
    if out_path.is_some() {
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            eprintln!("hale api import: could not make {}: {e}", out_dir.display());
            return ExitCode::from(1);
        }
    }
    let location = if fetch::is_url(&source) {
        source.clone()
    } else {
        relative(Path::new(&source), &out_dir)
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let surface = match read_surface(&name, kind, &source, &location, &cwd) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hale api import: {e}");
            return ExitCode::from(1);
        }
    };
    let text = match render_checked(&surface) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hale api import: {e}");
            return ExitCode::from(1);
        }
    };
    match out_path {
        None => print!("{text}"),
        Some(p) => {
            if let Err(e) = std::fs::write(&p, &text) {
                eprintln!("hale api import: could not write {}: {e}", p.display());
                return ExitCode::from(1);
            }
            eprintln!(
                "wrote {} ({} rows of `{name}` from {})",
                p.display(),
                surface.row_count(),
                surface.kind.word()
            );
        }
    }
    ExitCode::SUCCESS
}

/// `PetStore` as a file stem: `pet_store`.
fn snake_file(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// The surface's source, formatted, and parsed once to hold the import to
/// writing a program the parser reads.
fn render_checked(surface: &Surface) -> Result<String, String> {
    let raw = surface.render();
    let text = hale_syntax::fmt::format_source(&raw).map_err(|e| match e {
        hale_syntax::fmt::FmtError::Parse(d) => {
            format!(
                "the generated source does not lex: {}",
                d.first().map_or("", |d| d.message.as_str())
            )
        }
        hale_syntax::fmt::FmtError::Changed(_) => "the formatter refused the generated source".to_string(),
    })?;
    parse(&text, "the generated source")?;
    Ok(text)
}

fn parse(text: &str, what: &str) -> Result<Program, String> {
    let tokens =
        hale_syntax::lex(text).map_err(|d| format!("{what}: {}", d.first().map_or("", |d| d.message.as_str())))?;
    hale_syntax::parse(tokens, text).map_err(|d| format!("{what}: {}", d.first().map_or("", |d| d.message.as_str())))
}

// ---- the drift check ----

/// One row's shape, by slot: its request, its response and its wire name,
/// each with the records it names expanded.
type Shapes = BTreeMap<String, [String; 3]>;
const SLOTS: [&str; 3] = ["request", "response", "wire"];

/// The JSON key a field crosses as: its `json:"…"` tag, or its name.
fn wire_key(name: &str, tag: Option<&str>) -> String {
    tag.and_then(|t| {
        let at = t.find("json:\"")? + 6;
        let rest = &t[at..];
        let mut out = String::new();
        let mut cs = rest.chars();
        while let Some(c) = cs.next() {
            match c {
                '\\' => out.extend(cs.next()),
                '"' => return Some(out),
                c => out.push(c),
            }
        }
        None
    })
    .unwrap_or_else(|| name.to_string())
}

/// A type with every record and enum it names written out: a record as its
/// fields by JSON key (`?` on an optional one, its `has_` flag folded in),
/// an enum as its variants.
fn expand(te: &TypeExpr, types: &BTreeMap<&str, &TypeDecl>, stack: &mut Vec<String>) -> String {
    match te {
        TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            if segs == ["std", "json", "Value"] {
                return "any".into();
            }
            let [one] = segs.as_slice() else { return segs.join("::") };
            let Some(decl) = types.get(one) else {
                return one.to_string();
            };
            if stack.iter().any(|s| s == one) {
                return format!("rec({one})");
            }
            stack.push(one.to_string());
            let out = match &decl.body {
                TypeDeclBody::Struct(fields) => {
                    let names: Vec<&str> = fields.iter().map(|f| f.name.name.as_str()).collect();
                    let mut parts: Vec<String> = fields
                        .iter()
                        .filter(|f| !f.name.name.strip_prefix("has_").is_some_and(|x| names.contains(&x)))
                        .map(|f| {
                            let optional = names.contains(&format!("has_{}", f.name.name).as_str());
                            format!(
                                "{}{}:{}",
                                wire_key(&f.name.name, f.tag.as_deref()),
                                if optional { "?" } else { "" },
                                expand(&f.ty, types, stack)
                            )
                        })
                        .collect();
                    parts.sort();
                    format!("{{{}}}", parts.join(","))
                }
                TypeDeclBody::Enum(vs) => {
                    let mut names: Vec<&str> = vs.iter().map(|v| v.name.name.as_str()).collect();
                    names.sort();
                    format!("enum({})", names.join("|"))
                }
                TypeDeclBody::Alias(t) => expand(t, types, stack),
                TypeDeclBody::Scalar(_) => one.to_string(),
            };
            stack.pop();
            out
        }
        TypeExpr::Bounded { elem, cap, .. } => format!("bounded[{}; {cap}]", expand(elem, types, stack)),
        other => hale_types::surfaces::type_text(other),
    }
}

/// Every type the decls declare, by name.
fn types_of<'a>(decls: impl Iterator<Item = &'a TopDecl>) -> BTreeMap<&'a str, &'a TypeDecl> {
    decls
        .filter_map(|d| match d {
            TopDecl::Type(t) => Some((t.name.name.as_str(), t)),
            _ => None,
        })
        .collect()
}

fn shapes_of(api: &ApiDecl, types: &BTreeMap<&str, &TypeDecl>) -> Shapes {
    let mut out = Shapes::new();
    for r in &api.rows {
        let Some(sig) = &r.signature else { continue };
        let slot = |t: &Option<TypeExpr>| {
            t.as_ref()
                .map_or_else(|| "-".to_string(), |t| expand(t, types, &mut Vec::new()))
        };
        out.insert(
            r.method.name.clone(),
            [
                slot(&sig.request),
                slot(&sig.response),
                sig.wire.clone().unwrap_or_default(),
            ],
        );
    }
    out
}

/// One imported surface compared with its source.
pub struct Report {
    pub surface: String,
    pub location: String,
    pub rows: usize,
    /// `(kind, row, slot)`: `added`, `removed` or `changed` and the slot.
    pub drift: Vec<(&'static str, String, Option<&'static str>)>,
}

impl Report {
    pub fn lines(&self) -> Vec<String> {
        self.drift
            .iter()
            .map(|(kind, row, slot)| match slot {
                Some(s) => format!("drift {} {kind} {row} {s}", self.surface),
                None => format!("drift {} {kind} {row}", self.surface),
            })
            .collect()
    }
}

/// The imported surfaces the files declare (`only` names one), each
/// compared with what its source says now.
pub fn drift(files: &[PathBuf], only: Option<&str>) -> Result<Vec<Report>, String> {
    let mut programs = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("could not read {}: {e}", f.display()))?;
        programs.push((f.clone(), parse(&text, &f.display().to_string())?));
    }
    let types = types_of(
        programs
            .iter()
            .flat_map(|(_, p)| hale_syntax::ast::flat_decls(&p.items)),
    );
    let mut reports = Vec::new();
    for (file, p) in &programs {
        for d in hale_syntax::ast::flat_decls(&p.items) {
            let TopDecl::Api(api) = d else { continue };
            let Some(src) = &api.source else { continue };
            if only.is_some_and(|o| o != api.name.name) {
                continue;
            }
            let kind = if src.kind.name == "mcp" {
                Kind::Mcp
            } else {
                Kind::OpenApi
            };
            let base = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let fresh = read_surface(&api.name.name, Some(kind), &src.location, &src.location, &base)
                .map_err(|e| format!("`{}`: {e}", api.name.name))?;
            let text = render_checked(&fresh)?;
            let now = parse(&text, "the source's surface")?;
            let now_types = types_of(hale_syntax::ast::flat_decls(&now.items));
            let now_api = hale_syntax::ast::flat_decls(&now.items)
                .find_map(|d| match d {
                    TopDecl::Api(a) => Some(a),
                    _ => None,
                })
                .ok_or("the source's surface has no `api` block")?;
            let was = shapes_of(api, &types);
            let is = shapes_of(now_api, &now_types);
            let mut drift = Vec::new();
            for (row, shape) in &is {
                match was.get(row) {
                    None => drift.push(("added", row.clone(), None)),
                    Some(old) => {
                        for (i, slot) in SLOTS.iter().enumerate() {
                            if old[i] != shape[i] {
                                drift.push(("changed", row.clone(), Some(*slot)));
                            }
                        }
                    }
                }
            }
            for row in was.keys().filter(|r| !is.contains_key(*r)) {
                drift.push(("removed", row.clone(), None));
            }
            drift.sort_by(|a, b| (&a.1, a.0, a.2).cmp(&(&b.1, b.0, b.2)));
            reports.push(Report {
                surface: api.name.name.clone(),
                location: src.location.clone(),
                rows: was.len(),
                drift,
            });
        }
    }
    if let Some(o) = only {
        if reports.is_empty() {
            return Err(format!(
                "no imported surface `{o}` is declared (an `api {o} from openapi|mcp \"...\"` block)"
            ));
        }
    }
    Ok(reports)
}

/// [`drift`] over files of several seeds: the `.hl` files among `files`
/// that exist, once each, grouped by directory, a seed's own types the
/// scope of its blocks.
pub fn drift_by_seed(files: &[PathBuf]) -> Result<Vec<Report>, String> {
    let mut seeds: BTreeMap<PathBuf, std::collections::BTreeSet<PathBuf>> = BTreeMap::new();
    for f in files {
        if !f.is_file() || f.extension().is_none_or(|x| x != "hl") {
            continue;
        }
        let f = std::fs::canonicalize(f).unwrap_or_else(|_| f.clone());
        let dir = f.parent().map(Path::to_path_buf).unwrap_or_default();
        seeds.entry(dir).or_default().insert(f);
    }
    let mut out = Vec::new();
    for files in seeds.values() {
        out.extend(drift(&files.iter().cloned().collect::<Vec<_>>(), None)?);
    }
    Ok(out)
}

/// The `.hl` files of a target: the file, or a directory's own.
fn hl_files(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(target)
        .map_err(|e| format!("could not read {}: {e}", target.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "hl"))
        .collect();
    out.sort();
    Ok(out)
}

/// Prints each report's lines on `out` (in step, or each drift) and
/// answers the exit code: 0 in step, 1 drift.
pub fn print_reports(reports: &[Report], out: &mut dyn std::io::Write) -> u8 {
    let mut code = 0;
    for r in reports {
        if r.drift.is_empty() {
            let _ = writeln!(out, "{}: in step with {} ({} rows)", r.surface, r.location, r.rows);
        } else {
            code = 1;
            for l in r.lines() {
                let _ = writeln!(out, "{l}");
            }
        }
    }
    code
}

/// `hale api drift`.
pub fn run_drift(rest: &[String]) -> ExitCode {
    let mut target = None;
    let mut only = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--surface" => {
                i += 1;
                only = rest.get(i).cloned();
                if only.is_none() {
                    eprintln!("hale api drift: --surface takes a NAME");
                    return ExitCode::from(2);
                }
            }
            f if f.starts_with("--") => {
                eprintln!("hale api drift: unknown flag `{f}`");
                eprint!("{}", drift_usage());
                return ExitCode::from(2);
            }
            t if target.is_none() => target = Some(PathBuf::from(t)),
            t => {
                eprintln!("hale api drift: one target at a time; `{t}` is a second");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let target = target.unwrap_or_else(|| PathBuf::from("."));
    let reports = match hl_files(&target).and_then(|files| drift(&files, only.as_deref())) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("hale api drift: {e}");
            return ExitCode::from(2);
        }
    };
    if reports.is_empty() {
        eprintln!("hale api drift: `{}` declares no imported surface", target.display());
        return ExitCode::SUCCESS;
    }
    ExitCode::from(print_reports(&reports, &mut std::io::stdout()))
}
