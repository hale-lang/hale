//! GH #1835 (slice 1): the requirements artifact and its drift against the
//! manifest (spec/verification.md § "The requirements artifact", spec/dna.md
//! § Declared secrets).
//!
//! `hale check --dump-requirements` prints the secrets the checked program
//! reads (`hale_types::requirements`). Every check of a program under a
//! manifest that declares an organism (a `hale.toml` with a `[dna]` table,
//! at or above the program's directory) holds the program's reads of
//! declared secrets against `[dna.secrets]`, and warns on each difference:
//!
//! - a secret read in code (`HALE_SECRET_<NAME>`) that the manifest does not
//!   declare, or declares without the program among its readers, or reads
//!   as a family's prefix when it is no family: at the read;
//! - a secret the manifest names the program among the readers of that the
//!   program never reads: at the manifest's table, and only when the whole
//!   program was loaded (a seed directory, or a file with no other `.hl`
//!   beside it), so a file checked alone never reports its siblings' reads
//!   missing.
//!
//! Drift warns this release and is refused in the next (#1835's migration:
//! generate, compare, warn, then refuse).

use std::path::{Path, PathBuf};

use hale_frontend::snapshot::Snapshot;
use hale_types::requirements::{self as req, Place, SecretReads, Source};

/// The directory of the checked program: the target itself, or the file's.
fn seed_dir(target: &Path) -> PathBuf {
    let dir = if target.is_dir() { target.to_path_buf() } else { target.parent().unwrap_or(Path::new(".")).to_path_buf() };
    let dir = if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir };
    dir.canonicalize().unwrap_or(dir)
}

/// The program's name as a manifest's `readers` name it: its directory's.
pub(crate) fn program_name(target: &Path) -> String {
    seed_dir(target).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The reads of the snapshot's programs (its law programs left out).
pub(crate) fn reads(snap: &Snapshot) -> SecretReads {
    let bundle = snap.bundle();
    let programs: Vec<&hale_syntax::ast::Program> = bundle
        .programs
        .iter()
        .filter(|(k, _)| !hale_frontend::snapshot::is_std_law_program(Path::new(k.as_str())))
        .map(|(_, p)| *p)
        .collect();
    req::secret_reads(&programs, &bundle.import_renames)
}

/// A read's place: its file relative to the program's directory.
fn placer<'s>(snap: &'s Snapshot, root: PathBuf) -> impl Fn(hale_syntax::Span) -> Option<Place> + 's {
    move |span| {
        let off = span.start.as_usize() as u32;
        let (base, path, _) =
            snap.file_bases().iter().find(|(base, _, len)| hale_syntax::file_owns_offset(*base, *len, off))?;
        let src = snap.sources().get(path)?;
        let (line, col) = span.shifted(base.wrapping_neg()).line_col(src);
        Some(Place { file: hale_frontend::diag::display_relative(&root, path), line, col })
    }
}

/// `--dump-requirements`: the artifact.
pub(crate) fn artifact(target: &Path, snap: &Snapshot) -> String {
    let reads = reads(snap);
    let place = placer(snap, seed_dir(target));
    let secrets = req::secrets(&reads, &place);
    let mut unnamed: Vec<(Place, req::Kind, String)> =
        reads.unnamed.iter().filter_map(|u| Some((place(u.span)?, u.kind, u.field.clone()))).collect();
    unnamed.sort();
    unnamed.dedup();
    req::render_json(&program_name(target), &secrets, &unnamed)
}

/// The manifest that declares the organism the program is part of: the
/// nearest `hale.toml` at or above its directory with a `[dna]` table, no
/// higher than the repository's root (the directory holding `.git`).
fn organism_manifest(target: &Path) -> Option<(PathBuf, String)> {
    let mut dir = Some(seed_dir(target));
    while let Some(d) = dir {
        let path = d.join("hale.toml");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let has_dna = toml::from_str::<toml::Table>(&text).ok().is_some_and(|t| t.get("dna").is_some_and(|v| v.is_table()));
            if has_dna {
                return Some((path, text));
            }
        }
        if d.join(".git").exists() {
            return None;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

/// The line of `[dna.secrets.<name>]` in the manifest (1 when it is
/// written another way).
fn table_line(text: &str, name: &str) -> usize {
    let heads = [format!("[dna.secrets.{name}]"), format!("[dna.secrets.\"{name}\"]")];
    text.lines().position(|l| heads.iter().any(|h| l.trim_start().starts_with(h.as_str()))).map_or(1, |i| i + 1)
}

/// A finding anchored in the manifest rather than in the program.
pub(crate) struct ManifestWarning {
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}

impl ManifestWarning {
    pub(crate) fn render(&self) -> String {
        format!("{}:{}:1: warning: {}", hale_frontend::diag::diag_file_name(&self.path), self.line, self.message)
    }

    pub(crate) fn render_json(&self) -> String {
        hale_frontend::diag::render_json_record(
            &hale_frontend::diag::diag_file_name(&self.path),
            self.line,
            1,
            "warning",
            "warning",
            &self.message,
            "",
        )
    }
}

/// The drift between the program's reads of declared secrets and the
/// manifest's `[dna.secrets]`: warnings at the reads, and warnings at the
/// manifest. Nothing when no manifest declares an organism, or the target
/// is a test program (`*_test.hl`, `hale test`'s own).
pub(crate) fn drift(target: &Path, snap: &Snapshot) -> (Vec<hale_syntax::Diag>, Vec<ManifestWarning>) {
    let mut at_reads = Vec::new();
    let mut at_manifest = Vec::new();
    if target.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_test.hl")) {
        return (at_reads, at_manifest);
    }
    let Some((manifest, text)) = organism_manifest(target) else { return (at_reads, at_manifest) };
    let shown = hale_frontend::diag::diag_file_name(&manifest);
    let declared = match crate::dna::secrets::declared(&text) {
        Ok(d) => d,
        Err(why) => {
            at_manifest.push(ManifestWarning {
                path: manifest,
                line: 1,
                message: format!(
                    "the secrets this program reads are not compared with `[dna.secrets]`: the declaration is refused ({why})"
                ),
            });
            return (at_reads, at_manifest);
        }
    };
    let program = program_name(target);
    let reads = reads(snap);
    let declared_reads: Vec<&req::SecretRead> = reads.named.iter().filter(|r| r.source == Source::Declared).collect();
    // read in code: declared, naming this program, as a family when read as one
    let mut warned: std::collections::BTreeSet<(String, &str)> = Default::default();
    for r in &declared_reads {
        let var = if r.family { format!("{}{}", req::declared_var(&r.name), req::FAMILY_SUFFIX) } else { req::declared_var(&r.name) };
        let (what, message) = match declared.get(&r.name) {
            None => (
                "undeclared",
                format!(
                    "secret `{name}` is read here (`{var}`), but {shown} declares no `[dna.secrets.{name}]`, so no environment \
                     hands it: declare it there (`kind = \"drawn\"` or `\"slot\"`, `readers = [\"{program}\"]`). Drift between \
                     the code and the manifest is a warning this release and refused in the next",
                    name = r.name,
                ),
            ),
            Some(d) if !d.programs().contains(&program.as_str()) => (
                "reader",
                format!(
                    "secret `{name}` is read here (`{var}`), but `[dna.secrets.{name}]` ({shown}:{line}) does not name \
                     `{program}` among its readers: add `\"{program}\"` to its `readers`. Drift between the code and the \
                     manifest is a warning this release and refused in the next",
                    name = r.name,
                    line = table_line(&text, &r.name),
                ),
            ),
            Some(d) if r.family && !d.family => (
                "family",
                format!(
                    "secret `{name}` is read here as a family's members (`{var}`), but `[dna.secrets.{name}]` ({shown}:{line}) \
                     is no family, so no environment hands that variable: declare `family = true`, or read `{single}`. Drift \
                     between the code and the manifest is a warning this release and refused in the next",
                    name = r.name,
                    line = table_line(&text, &r.name),
                    single = req::declared_var(&r.name),
                ),
            ),
            Some(_) => continue,
        };
        // one warning per secret and difference, at its first read
        if warned.insert((r.name.clone(), what)) {
            at_reads.push(hale_syntax::Diag::warn(r.span, message));
        }
    }
    // declared for this program, never read by it: only when the whole
    // program was loaded
    if whole_program(target) {
        for (name, d) in &declared {
            if !d.programs().contains(&program.as_str()) || declared_reads.iter().any(|r| &r.name == name) {
                continue;
            }
            at_manifest.push(ManifestWarning {
                path: manifest.clone(),
                line: table_line(&text, name),
                message: format!(
                    "`[dna.secrets.{name}]` names `{program}` among its readers, but `{program}` never reads it: no read of \
                     `{var}` in its code. Read it (`std::secret::Credential {{ vault: std::env::var(\"{var}\") }}`), or take \
                     `\"{program}\"` out of its `readers`. Drift between the code and the manifest is a warning this release \
                     and refused in the next",
                    var = req::declared_var(name),
                ),
            });
        }
    }
    (at_reads, at_manifest)
}

/// Whether the check loaded the whole program: a seed directory, or a file
/// with no other non-test `.hl` file beside it.
fn whole_program(target: &Path) -> bool {
    if target.is_dir() {
        return true;
    }
    let dir = seed_dir(target);
    let own = target.file_name();
    std::fs::read_dir(dir).map_or(true, |es| {
        !es.flatten().any(|e| {
            let p = e.path();
            let name = e.file_name();
            p.extension().is_some_and(|x| x == "hl")
                && Some(name.as_os_str()) != own
                && !name.to_string_lossy().ends_with("_test.hl")
        })
    })
}
