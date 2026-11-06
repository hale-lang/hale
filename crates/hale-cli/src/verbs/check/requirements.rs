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
//! A reader is named as the manifest names it: a program by its directory,
//! and a host command (`[dna.host.<command>]`) as `host.<command>`. The
//! program a host command runs is the one whose directory its `install` or
//! `uninstall` names, itself or a file directly in it (`hale run node …`,
//! `sh node/install.sh …`), so a read in that program is the host
//! command's as well as the program's, and either name in `readers`
//! agrees with it. A host command that names no program's directory runs
//! a program the checker cannot tell, so a read whose `readers` name such
//! a command is not judged.
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

/// The names the manifest's `readers` give the checked program: its
/// directory's, and `host.<command>` for each host command that runs it.
struct Names {
    program: String,
    /// the host commands that run it, by the command's name
    hosts: Vec<String>,
    /// the host commands that name no program's directory: what they run
    /// the checker cannot tell
    opaque: Vec<String>,
}

impl Names {
    /// The checked program's names under `manifest` (none of its host
    /// commands' when there is no manifest).
    fn of(target: &Path, manifest: Option<(&Path, &str)>) -> Names {
        let program = program_name(target);
        let (hosts, opaque) = match manifest {
            Some((path, text)) => host_commands(path.parent().unwrap_or(Path::new(".")), text, &seed_dir(target)),
            None => Default::default(),
        };
        Names { program, hosts, opaque }
    }

    /// Whether a `readers` list names the program, by any of its names.
    fn named_by(&self, readers: &[String]) -> bool {
        readers.iter().any(|r| match r.strip_prefix("host.") {
            Some(cmd) => self.hosts.iter().any(|h| h == cmd),
            None => *r == self.program,
        })
    }

    /// Whether a `readers` list names a host command that may run the
    /// program for all the checker can tell.
    fn maybe_named_by(&self, readers: &[String]) -> bool {
        readers.iter().filter_map(|r| r.strip_prefix("host.")).any(|cmd| self.opaque.iter().any(|h| h == cmd))
    }

    /// The name a declaration should give it: the host command that runs
    /// it, else its directory's.
    fn preferred(&self) -> String {
        self.hosts.first().map_or_else(|| self.program.clone(), |h| format!("host.{h}"))
    }

    /// All its names, the host commands' first.
    fn all(&self) -> Vec<String> {
        let mut all: Vec<String> = self.hosts.iter().map(|h| format!("host.{h}")).collect();
        all.push(self.program.clone());
        all
    }

    /// All its names, each in backticks, `or` between.
    fn shown(&self) -> String {
        self.all().iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(" or ")
    }
}

/// The host commands of a manifest (`[dna.host.<command>]`) that run the
/// program in `seed`, and those that run a program the checker cannot
/// tell. A command runs the program whose directory one of its `install` or
/// `uninstall` words names (relative to the manifest's directory, where the
/// habitat runs it): the directory itself (`hale run node …`) or a file
/// directly in it (`sh node/install.sh …`). A command none of whose words
/// names a directory holding a `.hl` file, or a file directly in one, is
/// opaque: its script may run any program.
fn host_commands(root: &Path, text: &str, seed: &Path) -> (Vec<String>, Vec<String>) {
    let (mut runs, mut opaque) = (Vec::new(), Vec::new());
    let Ok(doc) = toml::from_str::<toml::Table>(text) else { return (runs, opaque) };
    let Some(cmds) = doc.get("dna").and_then(|d| d.get("host")).and_then(|h| h.as_table()) else { return (runs, opaque) };
    let program_dir = |d: &Path| {
        std::fs::read_dir(d).is_ok_and(|es| es.flatten().any(|e| e.path().extension().is_some_and(|x| x == "hl")))
    };
    for (name, t) in cmds {
        let words = ["install", "uninstall"]
            .iter()
            .filter_map(|k| t.get(*k).and_then(|v| v.as_array()))
            .flatten()
            .filter_map(|w| w.as_str())
            .filter(|w| !w.is_empty() && !w.starts_with('-') && !w.contains('{') && !Path::new(w).is_absolute());
        // the program directories its words name
        let mut dirs: Vec<PathBuf> = Vec::new();
        for w in words {
            let Ok(p) = root.join(w).canonicalize() else { continue };
            let dir = if p.is_dir() { p } else { p.parent().map(Path::to_path_buf).unwrap_or(p) };
            if program_dir(&dir) {
                dirs.push(dir);
            }
        }
        if dirs.iter().any(|d| d == seed) {
            runs.push(name.clone());
        } else if dirs.is_empty() {
            opaque.push(name.clone());
        }
    }
    (runs, opaque)
}

/// `--dump-requirements`: the artifact. A declared secret's readers are the
/// names the manifest's declaration gives the program for it, when it names
/// it, else the name it should (the host command that runs it, else its
/// directory's); any other secret's are all the program's names, which no
/// declaration narrows.
pub(crate) fn artifact(target: &Path, snap: &Snapshot) -> String {
    let reads = reads(snap);
    let place = placer(snap, seed_dir(target));
    let secrets = req::secrets(&reads, &place);
    let mut unnamed: Vec<(Place, req::Kind, String)> =
        reads.unnamed.iter().filter_map(|u| Some((place(u.span)?, u.kind, u.field.clone()))).collect();
    unnamed.sort();
    unnamed.dedup();
    let manifest = organism_manifest(target);
    let names = Names::of(target, manifest.as_ref().map(|(p, t)| (p.as_path(), t.as_str())));
    let declared = manifest.and_then(|(_, t)| crate::dna::secrets::declared(&t).ok()).unwrap_or_default();
    let readers = |s: &req::Secret| -> Vec<String> {
        if s.source != Source::Declared {
            return names.all();
        }
        let given: Vec<String> = match declared.get(&s.name) {
            Some(d) => d.readers.iter().filter(|r| names.named_by(std::slice::from_ref(r))).cloned().collect(),
            None => Vec::new(),
        };
        if given.is_empty() { vec![names.preferred()] } else { given }
    };
    req::render_json(&names.program, &secrets, &readers, &unnamed)
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
    let names = Names::of(target, Some((&manifest, &text)));
    let program = names.program.clone();
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
                     hands it: declare it there (`kind = \"drawn\"` or `\"slot\"`, `readers = [\"{preferred}\"]`). Drift \
                     between the code and the manifest is a warning this release and refused in the next",
                    name = r.name,
                    preferred = names.preferred(),
                ),
            ),
            // a reader the program is not, by any of its names, unless a
            // host command it names runs a program the checker cannot tell
            Some(d) if !names.named_by(&d.readers) && !names.maybe_named_by(&d.readers) => (
                "reader",
                format!(
                    "secret `{name}` is read here (`{var}`), but `[dna.secrets.{name}]` ({shown}:{line}) does not name \
                     {named} among its readers: add `\"{preferred}\"` to its `readers`{runs}. Drift between the code and \
                     the manifest is a warning this release and refused in the next",
                    name = r.name,
                    line = table_line(&text, &r.name),
                    named = names.shown(),
                    preferred = names.preferred(),
                    runs = match names.hosts.as_slice() {
                        [] => String::new(),
                        hosts => format!(
                            " (`[dna.host.{}]` runs `{program}`: a host command reads as `host.<command>`, and \
                             `\"{program}\"` would hand it to `{program}` run as a service)",
                            hosts.join("]`, `[dna.host.")
                        ),
                    },
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
    // declared for this program by its directory, never read by it: only
    // when the whole program was loaded. A host command's reading is not
    // the program's to show (its script may read the secret itself)
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
