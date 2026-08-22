//! GH #476 Change 9 (review round 1) — the checker and the document
//! must not disagree about a program.
//!
//! Law SELECTION decides which laws exist: it resolves group
//! members and refuses an unknown name or an unannounced empty
//! group. `hale check` reported those refusals; the artifact
//! lowering did not, because it ran only the clause enumeration and
//! never the group resolution. The model builder drops a selector
//! it cannot resolve and leaves the group entity memberless, and the
//! judgment declined to diagnose a memberless group precisely
//! because selection was supposed to have refused it — so the law
//! evaluated over an empty root set and could serialize as `holds`.
//!
//! Two machine-readable answers, opposite, about one program. These
//! tests run the real binary and require the artifact to carry the
//! selection issue and to refuse the dependent row.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_law_sel_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// (check stderr, parsed artifact)
fn check_and_dump(dir: &Path, src: &str) -> (String, serde_json::Value) {
    let file = dir.join("main.hl");
    std::fs::write(&file, src).unwrap();
    let checked = hale().arg("check").arg(&file).output().expect("check");
    let dumped = hale()
        .arg("check")
        .arg(&file)
        .arg("--dump-topology")
        .output()
        .expect("dump");
    let artifact: serde_json::Value = serde_json::from_slice(
        &dumped.stdout,
    )
    .unwrap_or_else(|e| {
        panic!(
            "artifact is not JSON ({}):\n{}",
            e,
            String::from_utf8_lossy(&dumped.stdout)
        )
    });
    (String::from_utf8_lossy(&checked.stderr).into_owned(), artifact)
}

fn law_row<'a>(
    artifact: &'a serde_json::Value,
    name: &str,
) -> &'a serde_json::Value {
    artifact["law"]["rows"]
        .as_array()
        .expect("law rows")
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no law row named `{}`", name))
}

fn issues(artifact: &serde_json::Value) -> Vec<String> {
    artifact["law"]["issues"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| i["message"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// An unknown group member: the reviewer's reproducer.
#[test]
fn an_unknown_group_member_reaches_the_artifact() {
    let dir = workdir("unknown");
    let (stderr, artifact) = check_and_dump(
        &dir,
        r#"
locus Worker {
    params { n: Int = 0; }
    fn run_job() { self.n = self.n + 1; }
}
group probes = { MissingWorker };
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { }; }
    claims {
        isolation: forbid reaches(probes, workers);
    }
    run() { self.w.run_job(); }
}
fn main() { App { }; }
"#,
    );
    // Premise: the checker refuses the program by name.
    assert!(
        stderr.contains("MissingWorker"),
        "fixture premise: check must reject the unknown member:\n{}",
        stderr
    );
    // The document says the same thing.
    let issues = issues(&artifact);
    assert!(
        issues.iter().any(|m| m.contains("MissingWorker")),
        "the artifact carries no selection issue for the unknown \
         member — the document and the checker disagree: {:?}",
        issues
    );
    assert_eq!(
        artifact["verdict"], "law_failed",
        "a document with an unsatisfied law account is not clean"
    );
    // …and the dependent law is not recorded as holding over a
    // domain the compiler just refused.
    assert_ne!(
        law_row(&artifact, "isolation")["verdict"], "holds",
        "the law reads as holding over an empty, refused domain"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty group that never said it could be empty. Same rule from
/// the other side: nothing is unknown here, the group is simply
/// vacuous, and vacuous truth is the fail-open `may_be_empty` exists
/// to make explicit.
#[test]
fn an_unannounced_empty_group_reaches_the_artifact() {
    let dir = workdir("empty");
    let (stderr, artifact) = check_and_dump(
        &dir,
        r#"
locus Worker {
    params { n: Int = 0; }
    fn run_job() { self.n = self.n + 1; }
}
group probes = { };
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { }; }
    claims {
        isolation: forbid reaches(probes, workers);
    }
    run() { self.w.run_job(); }
}
fn main() { App { }; }
"#,
    );
    assert!(
        stderr.contains("resolves to no declarations"),
        "fixture premise: check must reject the vacuous group:\n{}",
        stderr
    );
    let issues = issues(&artifact);
    assert!(
        issues.iter().any(|m| m.contains("resolves to no declarations")),
        "the artifact carries no selection issue for the empty \
         group: {:?}",
        issues
    );
    assert_ne!(
        law_row(&artifact, "isolation")["verdict"], "holds",
        "vacuous truth over an unannounced empty group is exactly \
         the fail-open this refuses"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The control that keeps the two above honest: `may_be_empty` is a
/// declared intent, so an empty group carrying it is NOT a selection
/// failure, and its law still holds vacuously — as it always has.
#[test]
fn a_declared_may_be_empty_group_still_holds_vacuously() {
    let dir = workdir("declared");
    let (stderr, artifact) = check_and_dump(
        &dir,
        r#"
locus Worker {
    params { n: Int = 0; }
    fn run_job() { self.n = self.n + 1; }
}
group probes = { } may_be_empty;
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { }; }
    claims {
        isolation: forbid reaches(probes, workers);
    }
    run() { self.w.run_job(); }
}
fn main() { App { }; }
"#,
    );
    assert!(
        !stderr.contains("resolves to no declarations"),
        "an author who declared the group may be empty was refused \
         anyway:\n{}",
        stderr
    );
    assert!(
        issues(&artifact).is_empty(),
        "a declared-empty group is not a selection issue: {:?}",
        issues(&artifact)
    );
    assert_eq!(
        law_row(&artifact, "isolation")["verdict"], "holds",
        "the declared vacuous case must keep holding"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
