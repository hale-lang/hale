//! A test that can start a body or bring up compose never makes its own
//! temporary root: it takes the one whose end takes everything under it
//! down.
//!
//! A verb that applies memory's schema (`hale habitat graft`, `up`,
//! `migrate`, `hale dna upgrade`, `memory migrate`) with no
//! `HALE_DNA_MEMORY_DSN_OWNER` brings the organism's memory up in the
//! local habitat, and `dev` and `run` start bodies. Under a
//! root a test made by hand, nothing took the compose project down when
//! the root went: seven `knowledge-db` containers outlived their tests on
//! one box in a day. So, as `dna_fixture_ports_are_free` refuses a drawn
//! port, this refuses a hand-made root wherever such a verb is spelled:
//!
//! - a DNA fixture (`*.hl` under a `tests` directory of `dna/`) takes
//!   `dna::scratch_root` and ends with `dna::scratch_done`: no `mktemp`,
//!   no `/tmp/` path built from the pid (re-spelling a `scratch_root`
//!   path to hand it to `scratch_done` is not making one);
//! - a CLI test (`crates/hale-cli/tests/*.rs`) takes `scratch::Scratch`
//!   (`support/scratch.rs`): no `temp_dir()` root, no `mktemp`, no
//!   `tempdir`; the toolchain cache the tests share
//!   (`hale-tests-iris-cache`) is not a root;
//! - a script under `scripts/` that makes a `mktemp` root hands it to
//!   `scripts/compose-down-under.sh` before removing it.
//!
//! There is no allowlist.
//!
//! GH #1653: the organism's memory, nerves and senses' store are the local
//! habitat's allocations, and the developer's habitat is long-lived: a
//! teardown never takes it down. A test's `hale` runs under a habitat of
//! its own (`hale test` gives each file one beside its vault,
//! `support/vault.rs` each test process; a script keeps `XDG_DATA_HOME`
//! under its root). `hale test`, `support/reap.rs` and
//! `scripts/compose-down-under.sh` tear a habitat down through one
//! teardown, `hale habitat sweep` (its behavior is that module's unit
//! tests'), and `dna::scratch_done` keeps the same refusal of the
//! developer's habitat's project whatever a root's file names.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Fixture,
    Cli,
    Script,
}

fn comment(kind: Kind, line: &str) -> bool {
    let t = line.trim_start();
    match kind {
        Kind::Script => t.starts_with('#'),
        _ => t.starts_with("//"),
    }
}

const VERBS: &[&str] = &["upgrade", "dev", "run"];

/// Whether `line` runs a verb that can start a body or bring up compose.
fn runs_a_verb(kind: Kind, line: &str) -> bool {
    // `at` as a word of its own: `dna runner` and `memory migrates` are not
    let word = |at: &str| {
        line.match_indices(at).any(|(i, _)| {
            let after = line[i + at.len()..].chars().next();
            !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
    };
    let spelled = |sep: &str| VERBS.iter().any(|v| word(&format!("dna{sep}{v}"))) || word(&format!("memory{sep}migrate")) || word(&format!("habitat{sep}migrate")) || word(&format!("habitat{sep}graft")) || word(&format!("habitat{sep}up"));
    match kind {
        // an argv line `habitat\ngraft`, or a shell string `habitat graft`
        Kind::Fixture => spelled("\\n") || spelled(" "),
        Kind::Cli => {
            let squashed: String = line.split_whitespace().collect::<Vec<_>>().join("");
            VERBS.iter().any(|v| squashed.contains(&format!("\"dna\",\"{v}\"")))
                || squashed.contains("\"dna\",\"memory\"")
                || squashed.contains("\"habitat\",\"migrate\"")
                || squashed.contains("\"habitat\",\"graft\"")
                || squashed.contains("\"habitat\",\"up\"")
                || spelled(" ")
        }
        Kind::Script => spelled(" "),
    }
}

/// Whether `line` makes a root of its own.
fn makes_a_root(kind: Kind, line: &str) -> bool {
    match kind {
        Kind::Fixture => {
            line.contains("mktemp")
                || (line.contains("\"/tmp/") && line.contains("process::pid()") && !line.contains("scratch_done("))
        }
        Kind::Cli => {
            line.contains("mktemp")
                || line.contains("tempdir")
                || (line.contains("temp_dir()") && !line.contains("hale-tests-iris-cache"))
        }
        Kind::Script => line.contains("mktemp"),
    }
}

/// The offending lines of a file that runs such a verb, `(line, text)`.
fn violations(kind: Kind, text: &str) -> Vec<(usize, String)> {
    let live: Vec<(usize, &str)> = text.lines().enumerate().filter(|(_, l)| !comment(kind, l)).collect();
    if !live.iter().any(|(_, l)| runs_a_verb(kind, l)) {
        return Vec::new();
    }
    if kind == Kind::Script && text.contains("compose-down-under.sh") {
        return Vec::new();
    }
    live.into_iter().filter(|(_, l)| makes_a_root(kind, l)).map(|(n, l)| (n + 1, l.trim().to_string())).collect()
}

fn collect_fixtures(dir: &Path, under_tests: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for p in entries.flatten().map(|e| e.path()) {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "node_modules" || n == "test-results") {
                continue;
            }
            let in_tests = under_tests || p.file_name().is_some_and(|n| n == "tests");
            collect_fixtures(&p, in_tests, out);
        } else if under_tests && p.extension().is_some_and(|e| e == "hl") {
            out.push(p);
        }
    }
}

fn files_with(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e == ext)).collect()
}

fn scanned() -> Vec<(Kind, String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    let mut fixtures = Vec::new();
    collect_fixtures(&root.join("dna"), false, &mut fixtures);
    let groups = [
        (Kind::Fixture, fixtures),
        (Kind::Cli, files_with(&root.join("crates/hale-cli/tests"), "rs")),
        (Kind::Script, files_with(&root.join("scripts"), "sh")),
    ];
    for (kind, mut files) in groups {
        files.sort();
        for p in files {
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            // this guard spells what it refuses
            // the suite harness owns its slice's roots and sweeps them itself
            if rel.ends_with("dna_fixture_roots_are_scratch.rs") || rel.ends_with("dna_native_suite.rs") {
                continue;
            }
            out.push((kind, rel, text));
        }
    }
    out
}

#[test]
fn no_test_that_starts_a_body_or_compose_makes_its_own_root() {
    let mut offenders = Vec::new();
    for (kind, rel, text) in scanned() {
        for (line, src) in violations(kind, &text) {
            offenders.push(format!("{rel}:{line}: {src}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "a test that runs a `hale dna` verb able to start a body or bring up compose makes its root \
         with `dna::scratch_root` (a fixture), `scratch::Scratch` (a CLI test) or hands it to \
         `scripts/compose-down-under.sh` (a script), never by hand ({} found):\n{}\n",
        offenders.len(),
        offenders.join("\n")
    );
}

#[test]
fn the_scan_is_not_vacuous() {
    let files = scanned();
    let running = |k: Kind| files.iter().filter(|(kind, _, text)| *kind == k && text.lines().any(|l| !comment(k, l) && runs_a_verb(k, l))).count();
    assert!(running(Kind::Fixture) >= 5, "only {} DNA fixtures run such a verb; the scan is broken", running(Kind::Fixture));
    assert!(running(Kind::Cli) >= 10, "only {} CLI tests run such a verb; the scan is broken", running(Kind::Cli));
    assert!(running(Kind::Script) >= 1, "no script runs such a verb; the scan is broken");
}

#[test]
fn the_matchers_see_what_they_name() {
    // what runs a verb
    assert!(runs_a_verb(Kind::Fixture, r#"let made = in_dir(scratch, toolchain() + "\nhabitat\ngraft\nbooks");"#));
    assert!(runs_a_verb(Kind::Fixture, r#"exec env HALE_DNA_HABITAT=\"$3\" \"$2\" habitat graft app\nsh\n"#));
    assert!(runs_a_verb(Kind::Fixture, r#"let up = in_dir(app, toolchain() + "\ndna\nupgrade");"#));
    assert!(!runs_a_verb(Kind::Fixture, r#"dna_cli(repo, "task\ndone\nt1")"#));
    assert!(!runs_a_verb(Kind::Fixture, r#"toolchain() + "\ndna\nrunner""#));
    assert!(!runs_a_verb(Kind::Fixture, r#"std::test::assert(dsn.ok, "memory migrates: " + dsn.why);"#));
    assert!(runs_a_verb(Kind::Fixture, r#"in_dir(app, toolchain() + "\ndna\nmemory\nmigrate")"#));
    assert!(runs_a_verb(Kind::Cli, r#"let (ok, out) = hale(&["habitat", "graft", "demo-app"], &d);"#));
    assert!(runs_a_verb(Kind::Cli, r#".args(["habitat",  "up", "--no-body"])"#));
    assert!(!runs_a_verb(Kind::Cli, r#"hale(&["dna", "--embedded-digest"])"#));
    assert!(runs_a_verb(Kind::Script, r#"LOTUS_DUMP_IR=1 "$hale" habitat graft "$tmp/warm" >/dev/null"#));
    // what makes a root
    assert!(makes_a_root(Kind::Fixture, r#"let repo = "/tmp/dna-public-" + to_string(std::process::pid());"#));
    assert!(makes_a_root(Kind::Fixture, r#"let made = dna::run_tool("mktemp\n-d\n/tmp/hale-x.XXXXXX");"#));
    assert!(!makes_a_root(Kind::Fixture, r#"dna::scratch_done("/tmp/dna-legs-placeholder-" + to_string(std::process::pid()));"#));
    assert!(!makes_a_root(Kind::Fixture, r#"let scratch = dna::scratch_root("legs");"#));
    assert!(makes_a_root(Kind::Cli, r#"let d = std::env::temp_dir().join(format!("hale_dna_x_{}", std::process::id()));"#));
    assert!(!makes_a_root(Kind::Cli, r#".env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))"#));
    assert!(!makes_a_root(Kind::Cli, r#"let d = scratch::Scratch::new(format!("hale_dna_x_{}", std::process::id()));"#));
    assert!(makes_a_root(Kind::Script, r#"tmp=$(mktemp -d "${TMPDIR:-/tmp}/hale-warm.XXXXXX")"#));
    // a script that hands its root to the teardown passes
    assert!(violations(Kind::Script, "tmp=$(mktemp -d)\ntrap '\"$here/compose-down-under.sh\" \"$tmp\"' EXIT\n\"$hale\" habitat graft \"$tmp/w\"\n").is_empty());
    assert_eq!(violations(Kind::Script, "tmp=$(mktemp -d)\ntrap 'rm -rf \"$tmp\"' EXIT\n\"$hale\" habitat graft \"$tmp/w\"\n").len(), 1);
}

/// GH #1653: the three Rust and shell teardowns are one, `hale habitat
/// sweep`, so they cannot drift apart; the fixtures' carries the same
/// refusal of the developer's habitat.
#[test]
fn a_teardown_never_takes_the_developers_habitat() {
    let repo = repo_root();
    let read = |p: &str| std::fs::read_to_string(repo.join(p)).unwrap();
    assert!(read("scripts/compose-down-under.sh").contains("habitat sweep \"$root\""), "compose-down-under.sh sweeps through the toolchain");
    assert!(read("crates/hale-cli/tests/support/reap.rs").contains(".args([\"habitat\", \"sweep\"])"), "the CLI tests' reap sweeps through the toolchain");
    assert!(read("crates/hale-cli/src/verbs/test.rs").contains("crate::dna::habitat::down_under(dir)"), "hale test's teardown is the sweep's");
    let habitat = read("crates/hale-cli/src/dna/habitat.rs");
    assert!(habitat.contains("*p != mine && Some(p) != current.as_ref()"), "the sweep refuses the developer's habitat");
    assert!(read("dna/core/workspace.hl").contains("project == mine"), "dna::scratch_projects_in refuses the developer's habitat");
}
