//! GH #1835 (slice 1): the requirements artifact and its drift against the
//! manifest (spec/verification.md § "The requirements artifact").
//!
//! The fixture organism under `fixtures/requirements/` reads every secret
//! its `hale.toml` declares for `api`, each the way a program reads one
//! today; `api.requirements.json` beside it is the artifact's baseline.
//! Regenerate deliberately:
//!
//! ```text
//! HALE_REGEN_REQUIREMENTS=1 cargo test -p hale-cli --test topology requirements_artifact::
//! ```
//!
//! The drift cases build an organism of their own in a scratch directory:
//! a secret read but undeclared, read by a program its declaration does
//! not name, read as a family when it is none, and declared for a program
//! that never reads it, each a warning (never an error: the check still
//! passes, `hale verify` fails on it), and silence when they agree. A
//! program a host command runs (`[dna.host.x]` naming its directory, or a
//! script in it) reads as `host.x` as well as by its directory's name.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

fn hale() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.env("HALE_SKIP_STALE_CHECK", "1");
    c
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/requirements")
}

fn dump(target: &Path) -> String {
    let out = hale().arg("check").arg("--dump-requirements").arg(target).output().expect("hale check");
    assert!(out.status.success(), "hale check --dump-requirements failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8")
}

#[test]
fn the_artifact_matches_its_baseline() {
    let got = dump(&fixture_dir().join("api"));
    let path = fixture_dir().join("api.requirements.json");
    if std::env::var("HALE_REGEN_REQUIREMENTS").is_ok() {
        std::fs::write(&path, &got).expect("write baseline");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("baseline present");
    assert!(
        got == want,
        "the requirements artifact moved from its committed baseline. If that is intended, regenerate with \
         HALE_REGEN_REQUIREMENTS=1 cargo test -p hale-cli --test topology requirements_artifact::\n--- got\n{got}"
    );
    assert_eq!(dump(&fixture_dir().join("api")), got, "the artifact is not reproducible");

    let d: Value = serde_json::from_str(&got).expect("json");
    assert_eq!(d["schema"], "hale.requirements");
    assert_eq!(d["version"], "1.0");
    assert_eq!(d["program"], "api");
    let named: Vec<(String, String, String)> = d["secrets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["source"].as_str().unwrap().into(), s["name"].as_str().unwrap().into(), s["kind"].as_str().unwrap().into()))
        .collect();
    let want: Vec<(String, String, String)> = [
        ("declared", "model-token", "entry"),
        ("declared", "operator-token", "credential"),
        ("declared", "upstream-key", "entry"),
        ("vault", "forge-token", "credential"),
        ("env", "SIGNING_KEY", "signer"),
    ]
    .iter()
    .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
    .collect();
    assert_eq!(named, want);
    let family = d["secrets"].as_array().unwrap().iter().find(|s| s["name"] == "model-token").unwrap();
    assert_eq!(family["family"], true);
    assert_eq!(family["variable"], "HALE_SECRET_MODEL_TOKEN");
    for s in d["secrets"].as_array().unwrap() {
        assert_eq!(s["readers"], serde_json::json!(["api"]));
        assert!(!s["reads"].as_array().unwrap().is_empty(), "{s}: no read placed");
    }
    // the computed vault entry is listed, never left out
    assert_eq!(d["unnamed"], serde_json::json!([{ "file": "main.hl", "line": 32, "col": 42, "kind": "credential", "field": "vault" }]));
}

#[test]
fn the_fixture_agrees_with_its_manifest() {
    let out = hale().arg("verify").arg(fixture_dir().join("api")).output().expect("hale verify");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the fixture drifts from its manifest:\n{stderr}");
    assert!(!stderr.contains("dna.secrets"), "{stderr}");
}

#[test]
fn a_program_that_does_not_typecheck_has_no_artifact() {
    let org = Organism::new("refused", "", "fn main() { let x: Int = \"no\"; }\n");
    let out = hale().arg("check").arg("--dump-requirements").arg(org.api()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refusing to emit a requirements artifact"));
}

/// A scratch organism: `hale.toml` with `[dna]` and the given secrets, and
/// the program `api/main.hl`.
struct Organism {
    root: PathBuf,
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

impl Organism {
    fn new(tag: &str, secrets: &str, main: &str) -> Organism {
        let root = std::env::temp_dir().join(format!(
            "hale_requirements_{}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            tag
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("api")).unwrap();
        std::fs::write(root.join("hale.toml"), format!("[dna]\nneeds = [\"memory\"]\n\n{secrets}")).unwrap();
        std::fs::write(root.join("api/main.hl"), main).unwrap();
        Organism { root }
    }

    fn api(&self) -> PathBuf {
        self.root.join("api")
    }

    fn check(&self, verb: &str, target: &Path) -> Output {
        hale().arg(verb).arg(target).output().unwrap()
    }
}

impl Drop for Organism {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A program reading the given variables, each through `std::env::var`.
fn reading(vars: &[&str]) -> String {
    let reads: Vec<String> = vars.iter().map(|v| format!("std::env::var(\"{v}\")")).collect();
    let body = if reads.is_empty() { "\"\"".to_string() } else { reads.join(" + ") };
    format!("fn entries() -> String {{ return {body}; }}\n\nfn main() {{ println(entries()); }}\n")
}

/// The drift warnings of a check (stderr's lines naming `[dna.secrets`).
fn warnings(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains(": warning: ") && l.contains("[dna.secrets"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_secret_read_but_undeclared_warns_at_the_read() {
    let org = Organism::new("undeclared", "", &reading(&["HALE_SECRET_OPERATOR_TOKEN"]));
    let out = org.check("check", &org.api());
    assert!(out.status.success(), "drift is a warning, never an error: {}", String::from_utf8_lossy(&out.stderr));
    let w = warnings(&out);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("api/main.hl:1:"), "at the read: {}", w[0]);
    assert!(w[0].contains("secret `operator-token` is read here (`HALE_SECRET_OPERATOR_TOKEN`)"), "{}", w[0]);
    assert!(w[0].contains("declares no `[dna.secrets.operator-token]`"), "{}", w[0]);
    assert!(w[0].contains("readers = [\"api\"]"), "{}", w[0]);
    // `hale verify` fails on it, as on any advisory
    assert!(!org.check("verify", &org.api()).status.success());
}

#[test]
fn a_secret_read_by_a_program_its_declaration_does_not_name_warns() {
    let org = Organism::new(
        "reader",
        "[dna.secrets.upstream-key]\nkind = \"slot\"\nreaders = [\"worker\"]\n",
        &reading(&["HALE_SECRET_UPSTREAM_KEY"]),
    );
    let out = org.check("check", &org.api());
    assert!(out.status.success());
    let w = warnings(&out);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("does not name `api` among its readers"), "{}", w[0]);
    assert!(w[0].contains("hale.toml:4)"), "names the manifest's table: {}", w[0]);
}

#[test]
fn a_family_read_of_a_single_secret_warns() {
    let org = Organism::new(
        "family",
        "[dna.secrets.model-token]\nkind = \"slot\"\nreaders = [\"api\"]\n",
        &reading(&["HALE_SECRET_MODEL_TOKEN", "HALE_SECRET_MODEL_TOKEN_FAMILY"]),
    );
    let w = warnings(&org.check("check", &org.api()));
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("as a family's members (`HALE_SECRET_MODEL_TOKEN_FAMILY`)"), "{}", w[0]);
    assert!(w[0].contains("is no family"), "{}", w[0]);
}

#[test]
fn a_secret_declared_but_never_read_warns_at_the_manifest() {
    let org = Organism::new(
        "unread",
        "[dna.secrets.operator-token]\nkind = \"drawn\"\nreaders = [\"api\"]\n\n\
         [dna.secrets.other-key]\nkind = \"slot\"\nreaders = [\"worker\"]\n",
        &reading(&[]),
    );
    let out = org.check("check", &org.api());
    assert!(out.status.success());
    let w = warnings(&out);
    // `other-key` is `worker`'s: not this program's to read
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("hale.toml:4:1: warning:"), "at the table: {}", w[0]);
    assert!(w[0].contains("`[dna.secrets.operator-token]` names `api` among its readers, but `api` never reads it"), "{}", w[0]);
    assert!(w[0].contains("HALE_SECRET_OPERATOR_TOKEN"), "{}", w[0]);
    // `--json` carries it as a record of its own
    let json = hale().arg("check").arg("--json").arg(org.api()).output().unwrap();
    let records: Vec<Value> =
        String::from_utf8_lossy(&json.stdout).lines().map(|l| serde_json::from_str(l).expect("ndjson")).collect();
    let r = records.iter().find(|r| r["message"].as_str().unwrap_or("").contains("never reads it")).expect("the record");
    assert_eq!(r["severity"], "warning");
    assert_eq!(r["line"], 4);
    assert!(r["file"].as_str().unwrap().ends_with("hale.toml"));
    // a file checked alone beside another never reports its siblings' reads missing
    std::fs::write(org.api().join("other.hl"), "fn other() -> String { return \"\"; }\n").unwrap();
    assert!(warnings(&org.check("check", &org.api().join("main.hl"))).is_empty());
}

#[test]
fn a_program_that_agrees_with_its_manifest_is_silent() {
    let org = Organism::new(
        "agrees",
        "[dna.secrets.operator-token]\nkind = \"drawn\"\nreaders = [\"api\"]\n\n\
         [dna.secrets.model-token]\nkind = \"slot\"\nfamily = true\nreaders = [\"api\"]\n\n\
         [dna.secrets.dashboard]\nkind = \"drawn\"\nperson = \"read\"\n",
        &reading(&["HALE_SECRET_OPERATOR_TOKEN", "HALE_SECRET_MODEL_TOKEN_FAMILY"]),
    );
    let out = org.check("verify", &org.api());
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(warnings(&out).is_empty());
}

#[test]
fn a_program_under_no_organism_is_never_compared() {
    let org = Organism::new("plain", "", &reading(&["HALE_SECRET_OPERATOR_TOKEN"]));
    std::fs::write(org.root.join("hale.toml"), "# a seed's manifest, declaring no organism\n").unwrap();
    let out = org.check("verify", &org.api());
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // …and its artifact still lists the read
    let d: Value = serde_json::from_str(&dump(&org.api())).unwrap();
    assert_eq!(d["secrets"][0]["name"], "operator-token");
}

/// An organism whose host command `[dna.host.x]` runs the program `x`
/// (`install`, by `run`), which reads `HALE_SECRET_X_TOKEN`; `secrets` the
/// manifest's `[dna.secrets]` and any other host command.
fn host_organism(tag: &str, install: &str, secrets: &str) -> Organism {
    let org = Organism::new(tag, &format!("[dna.host.x]\ninstall = {install}\n\n{secrets}"), &reading(&[]));
    std::fs::create_dir_all(org.root.join("x")).unwrap();
    std::fs::write(org.root.join("x/main.hl"), reading(&["HALE_SECRET_X_TOKEN"])).unwrap();
    std::fs::write(org.root.join("x/install.sh"), "#!/bin/sh\nexec hale run x \"$@\"\n").unwrap();
    org
}

#[test]
fn a_read_in_a_host_commands_program_is_the_host_commands() {
    // the command names the program's directory, or a script in it
    for install in ["[\"hale\", \"run\", \"x\", \"install\"]", "[\"sh\", \"x/install.sh\", \"install\", \"--id\", \"{id}\"]"] {
        let org = host_organism("host", install, "[dna.secrets.x-token]\nkind = \"drawn\"\nreaders = [\"host.x\"]\n");
        let x = org.root.join("x");
        let out = org.check("verify", &x);
        assert!(out.status.success(), "{install}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(warnings(&out).is_empty());
        // the artifact names it as the manifest does
        let d: Value = serde_json::from_str(&dump(&x)).unwrap();
        assert_eq!(d["program"], "x");
        assert_eq!(d["secrets"][0]["name"], "x-token");
        assert_eq!(d["secrets"][0]["readers"], serde_json::json!(["host.x"]), "{install}");
    }
    // the program by its directory's name agrees as well, and the artifact
    // keeps both when the declaration names both
    let org = host_organism(
        "either",
        "[\"sh\", \"x/install.sh\"]",
        "[dna.secrets.x-token]\nkind = \"drawn\"\nreaders = [\"x\", \"host.x\"]\n",
    );
    let out = org.check("verify", &org.root.join("x"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let d: Value = serde_json::from_str(&dump(&org.root.join("x"))).unwrap();
    assert_eq!(d["secrets"][0]["readers"], serde_json::json!(["x", "host.x"]));
}

#[test]
fn a_host_commands_read_its_readers_do_not_name_warns_naming_the_command() {
    let org = host_organism("hostmissing", "[\"sh\", \"x/install.sh\"]", "[dna.secrets.x-token]\nkind = \"drawn\"\nreaders = [\"api\"]\n");
    let x = org.root.join("x");
    let out = org.check("check", &x);
    assert!(out.status.success());
    let w = warnings(&out);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("does not name `host.x` or `x` among its readers"), "{}", w[0]);
    assert!(w[0].contains("add `\"host.x\"` to its `readers`"), "{}", w[0]);
    // the artifact says the name it should have
    let d: Value = serde_json::from_str(&dump(&x)).unwrap();
    assert_eq!(d["secrets"][0]["readers"], serde_json::json!(["host.x"]));
    // undeclared, it suggests the host command's name too
    let org = host_organism("hostundeclared", "[\"sh\", \"x/install.sh\"]", "");
    let w = warnings(&org.check("check", &org.root.join("x")));
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("readers = [\"host.x\"]"), "{}", w[0]);
}

#[test]
fn a_host_reader_never_read_by_the_program_is_not_the_programs_drift() {
    // `host.x` reads `other-token` in its script: the program `x` never does
    let org = host_organism(
        "hostunread",
        "[\"sh\", \"x/install.sh\"]",
        "[dna.secrets.x-token]\nkind = \"drawn\"\nreaders = [\"host.x\"]\n\n\
         [dna.secrets.other-token]\nkind = \"drawn\"\nreaders = [\"host.x\"]\n",
    );
    let out = org.check("verify", &org.root.join("x"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(warnings(&out).is_empty());
}

#[test]
fn a_read_a_host_command_may_make_through_a_script_the_checker_cannot_follow_is_not_judged() {
    // `[dna.host.y]` runs a script outside any program's directory: it may
    // run `x`, so a read its readers name `host.y` for is not drift…
    let org = host_organism(
        "opaque",
        "[\"sh\", \"x/install.sh\"]",
        "[dna.host.y]\ninstall = [\"sh\", \"scripts/y.sh\"]\n\n\
         [dna.secrets.x-token]\nkind = \"drawn\"\nreaders = [\"host.y\"]\n",
    );
    std::fs::create_dir_all(org.root.join("scripts")).unwrap();
    std::fs::write(org.root.join("scripts/y.sh"), "#!/bin/sh\nexec hale run x\n").unwrap();
    let out = org.check("verify", &org.root.join("x"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // …while one naming another program's directory runs that one, not `x`
    let org = host_organism(
        "other",
        "[\"sh\", \"x/install.sh\"]",
        "[dna.host.y]\ninstall = [\"hale\", \"run\", \"api\"]\n\n\
         [dna.secrets.x-token]\nkind = \"drawn\"\nreaders = [\"host.y\"]\n",
    );
    let w = warnings(&org.check("check", &org.root.join("x")));
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("does not name `host.x` or `x` among its readers"), "{}", w[0]);
}
