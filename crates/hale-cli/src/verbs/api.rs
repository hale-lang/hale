//! `hale api`: the generated forms of a surface (GH #1417, R8a).
//!
//! `export` writes a surface's bundle (the description, OpenAPI, JSON
//! Schema and MCP documents and the digest) and `--check` compares a
//! committed bundle to the surface. Every file is a pure function of the
//! surface's rows (spec/api.md § The clients): the same surface yields the
//! same bytes on any run and any checkout.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hale_frontend::snapshot::Snapshot;
use hale_types::surface_doc;
use hale_types::surfaces::{Schemas, SurfaceRows};

use crate::shared::options::flag_value_in;
use crate::verbs::check::run_impl::load_for_check;

pub(crate) fn api_usage() -> &'static str {
    "\
usage: hale api export --surface NAME [--out DIR | --check DIR] [file.hl | dir]

`export` writes a surface's bundle into DIR (the current directory by default):
  NAME.description.json   the surface-wide document: every member with its
                          roles, the exposures that serve it, the hubs and the
                          schemas (the description schema's inventory form)
  NAME.openapi.json       the OpenAPI form (hale check --api --surface NAME --openapi)
  NAME.json-schema.json   the JSON Schema form
  NAME.mcp.json           the MCP form
  DIGEST                  the surface's contract digest and the compiler's version
The files are rendered from the surface's rows alone, so two runs and two
checkouts of one program write the same bytes. `--check DIR` writes nothing:
it compares a committed bundle to the surface and exits 1 naming the drift.
"
}

/// A flag's value, or the usage error.
fn value(rest: &[String], flag: &str) -> Result<Option<String>, ExitCode> {
    flag_value_in(rest, flag).map_err(|msg| {
        eprintln!("hale api: {msg}");
        ExitCode::from(2)
    })
}

/// The flags of `export` that take a value, and the ones that do not.
const EXPORT_VALUED: [&str; 3] = ["--surface", "--out", "--check"];

/// The positional arguments of `rest`: everything that is neither a
/// flag nor a valued flag's value.
fn positionals(rest: &[String], valued: &[&str]) -> Result<Vec<String>, ExitCode> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if a.starts_with("--") {
            let base = a.split_once('=').map_or(a.as_str(), |(b, _)| b);
            if !valued.contains(&base) {
                eprintln!("hale api: unknown flag `{base}`");
                eprintln!("{}", api_usage());
                return Err(ExitCode::from(2));
            }
            i += if a.contains('=') { 1 } else { 2 };
            continue;
        }
        out.push(a.clone());
        i += 1;
    }
    Ok(out)
}

pub(crate) fn run_api(rest: &[String]) -> ExitCode {
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", api_usage());
        return ExitCode::SUCCESS;
    }
    match rest.first().map(String::as_str) {
        Some("export") => run_export(&rest[1..]),
        _ => {
            eprint!("{}", api_usage());
            ExitCode::from(2)
        }
    }
}

/// The checked snapshot of `target`, or the exit code that says why not.
/// A program that does not typecheck describes nothing, as for
/// `hale check --api`.
pub(crate) fn load_checked(target: &Path) -> Result<Snapshot, ExitCode> {
    let snap = load_for_check(target, &[], None).map_err(ExitCode::from)?;
    let checked: Vec<hale_syntax::Diag> = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    if let Some(d) = checked.iter().find(|d| d.is_error() && d.kind != hale_syntax::error::DiagKind::Claim) {
        eprintln!(
            "refusing to generate from `{}`: it does not typecheck, so what it generated would name a program \
             that does not exist. Fix the {} first.",
            target.display(),
            d.kind_str()
        );
        return Err(ExitCode::from(1));
    }
    Ok(snap)
}

/// What the generators read of a loaded program: its rows and the schemas
/// of the types they name.
pub(crate) fn with_rows<T>(
    snap: &Snapshot,
    f: impl FnOnce(&SurfaceRows, &Schemas<'_>) -> Result<T, String>,
) -> Result<T, ExitCode> {
    let rows = snap.demand_surface_rows().map_err(|b| {
        eprintln!("refusing to generate: {}", b.because.first().map_or("blocked", |d| d.message.as_str()));
        ExitCode::from(1)
    })?;
    let programs: Vec<&hale_syntax::ast::Program> = snap.programs().values().collect();
    let schemas = Schemas::of(&programs);
    f(rows, &schemas).map_err(|msg| {
        eprintln!("hale api: {msg}");
        ExitCode::from(2)
    })
}

/// The compiler's version, as a bundle's `DIGEST` names it.
fn compiler() -> String {
    format!("hale {}", env!("CARGO_PKG_VERSION"))
}

/// A surface's bundle: each file's name and bytes, in the order written.
pub(crate) fn bundle(rows: &SurfaceRows, schemas: &Schemas<'_>, surface: &str) -> Result<Vec<(String, String)>, String> {
    let model = surface_doc::client_model(rows, schemas, surface)?;
    Ok(vec![
        (format!("{surface}.description.json"), surface_doc::surface_description(rows, schemas, surface)?.pretty()),
        (format!("{surface}.openapi.json"), surface_doc::openapi(rows, schemas, surface)?.pretty()),
        (format!("{surface}.json-schema.json"), surface_doc::json_schema(rows, schemas, surface)?.pretty()),
        (format!("{surface}.mcp.json"), surface_doc::mcp(rows, schemas, surface)?.pretty()),
        ("DIGEST".to_string(), format!("{}\n{}\n", model.digest, compiler())),
    ])
}

/// The digest line of a `DIGEST` file.
fn digest_of(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

fn run_export(rest: &[String]) -> ExitCode {
    let surface = match value(rest, "--surface") {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("hale api export needs --surface NAME: the surface whose bundle is written");
            return ExitCode::from(2);
        }
        Err(code) => return code,
    };
    let (out, check) = match (value(rest, "--out"), value(rest, "--check")) {
        (Ok(o), Ok(c)) => (o, c),
        (Err(code), _) | (_, Err(code)) => return code,
    };
    if out.is_some() && check.is_some() {
        eprintln!("hale api export: --check writes nothing; give --out or --check, not both");
        return ExitCode::from(2);
    }
    let pos = match positionals(rest, &EXPORT_VALUED) {
        Ok(p) => p,
        Err(code) => return code,
    };
    if pos.len() > 1 {
        eprintln!("hale api export takes ONE target, got {}: {}", pos.len(), pos.join(" "));
        return ExitCode::from(2);
    }
    let target = PathBuf::from(pos.first().map_or(".", String::as_str));
    let snap = match load_checked(&target) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let files = match with_rows(&snap, |rows, schemas| bundle(rows, schemas, &surface)) {
        Ok(f) => f,
        Err(code) => return code,
    };
    if let Some(dir) = check {
        return check_bundle(&PathBuf::from(dir), &surface, &files);
    }
    let dir = PathBuf::from(out.unwrap_or_else(|| ".".to_string()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("hale api export: cannot create {}: {e}", dir.display());
        return ExitCode::from(2);
    }
    for (name, text) in &files {
        if let Err(e) = std::fs::write(dir.join(name), text) {
            eprintln!("hale api export: cannot write {}: {e}", dir.join(name).display());
            return ExitCode::from(2);
        }
        eprintln!("wrote {}", dir.join(name).display());
    }
    ExitCode::SUCCESS
}

/// Compare a committed bundle to the surface's: exit 1, naming the drift.
/// The compiler's version is the bundle's provenance and not part of the
/// comparison; a bundle written by another version that still reads the
/// same is current.
fn check_bundle(dir: &Path, surface: &str, files: &[(String, String)]) -> ExitCode {
    let mut drift = Vec::new();
    let now = files.iter().find(|(n, _)| n == "DIGEST").map_or("", |(_, t)| digest_of(t));
    match std::fs::read_to_string(dir.join("DIGEST")) {
        Ok(t) if digest_of(&t) == now => {}
        Ok(t) => drift.push(format!(
            "the surface {surface} moved: the committed bundle is of {}, the surface is now {now}",
            digest_of(&t)
        )),
        Err(_) => drift.push(format!("{} is missing", dir.join("DIGEST").display())),
    }
    for (name, text) in files.iter().filter(|(n, _)| n != "DIGEST") {
        match std::fs::read_to_string(dir.join(name)) {
            Ok(have) if &have == text => {}
            Ok(_) => drift.push(format!("{} differs from what the surface generates", dir.join(name).display())),
            Err(_) => drift.push(format!("{} is missing", dir.join(name).display())),
        }
    }
    if drift.is_empty() {
        eprintln!("the bundle in {} is current: {surface} {now}", dir.display());
        return ExitCode::SUCCESS;
    }
    eprintln!("the bundle in {} has drifted from the surface:", dir.display());
    for d in &drift {
        eprintln!("  {d}");
    }
    eprintln!("regenerate it: hale api export --surface {surface} --out {} <target>", dir.display());
    ExitCode::from(1)
}
