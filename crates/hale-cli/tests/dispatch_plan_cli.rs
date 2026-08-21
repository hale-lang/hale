//! GH #476 Change 8 — the dispatch plan, proven against the thing
//! that actually lowers.
//!
//! `DispatchPlan` has two fact sources by design: the model's bus
//! graph (built over the AUTHORED bundle) and codegen's (built over
//! the merged, desugared program its own emission uses). The
//! decision LADDER is shared — `DispatchFlavor::of` — but shared
//! code proves nothing about facts, so this differential compares
//! the two plans over the real corpus, through the real binaries:
//! `hale model dump` prints the model's plan, `HALE_DISPATCH_TRACE`
//! prints codegen's.
//!
//! The second test pins the consequence: the plan is part of the
//! execution identity, so a build that lowers dispatch differently
//! is a different build and its recording is not admitted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(name: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("hale_dispatch_cli_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// `subject -> flavor` from `hale model dump`'s plan section.
fn model_plan(prog: &Path) -> BTreeMap<String, String> {
    let out = hale()
        .arg("model")
        .arg("dump")
        .arg(prog)
        .output()
        .expect("hale model dump");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = BTreeMap::new();
    let mut in_plan = false;
    for line in text.lines() {
        if line.starts_with("dispatch_plan (") {
            in_plan = true;
            continue;
        }
        if !in_plan {
            continue;
        }
        let Some(rest) = line.strip_prefix("  ") else { break };
        let mut it = rest.split(' ');
        let (Some(subject), Some(flavor)) = (it.next(), it.next()) else {
            break;
        };
        rows.insert(subject.to_string(), flavor.to_string());
    }
    rows
}

/// `subject -> flavor` from the backend's own plan.
fn codegen_plan(dir: &Path, prog: &str) -> BTreeMap<String, String> {
    let out = hale()
        .current_dir(dir)
        .arg("build")
        .arg(prog)
        .env("HALE_DISPATCH_TRACE", "1")
        .output()
        .expect("hale build");
    assert!(
        out.status.success(),
        "build of {} failed: {}",
        prog,
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stderr);
    let mut rows = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("[hale-dispatch] ") else {
            continue;
        };
        let mut it = rest.split(' ');
        if let (Some(subject), Some(flavor)) = (it.next(), it.next()) {
            rows.insert(subject.to_string(), flavor.to_string());
        }
    }
    rows
}

/// Every subject the MODEL plans must be planned the same way by
/// codegen. The converse does not hold and must not be asserted:
/// codegen merges the Hale-source stdlib, so its plan additionally
/// carries stdlib subjects (`log.**`, `io.tcp.**`) that no user
/// model has any business naming.
#[test]
fn the_model_plan_agrees_with_the_codegen_plan_over_the_corpus() {
    let corpus =
        repo_root().join("crates/hale-codegen/tests/fixtures/examples");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("corpus")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("main.hl").is_file())
        .collect();
    dirs.sort();
    let work = workdir("differential");
    let mut compared = 0usize;
    let mut subjects = 0usize;
    for d in dirs {
        let main = d.join("main.hl");
        let model = model_plan(&main);
        if model.is_empty() {
            continue; // no bus surface: nothing to disagree about
        }
        // Build in a copy so the corpus tree stays clean.
        let name = d.file_name().unwrap().to_string_lossy().into_owned();
        let dst = work.join(&name);
        let _ = std::fs::create_dir_all(&dst);
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_file() {
                let _ = std::fs::copy(
                    e.path(),
                    dst.join(e.file_name()),
                );
            }
        }
        let cg = codegen_plan(&dst, "main.hl");
        assert!(
            !cg.is_empty(),
            "{}: codegen printed no plan at all",
            name
        );
        for (subject, flavor) in &model {
            assert_eq!(
                cg.get(subject),
                Some(flavor),
                "{}: the model plans `{}` as {} but codegen lowers \
                 it as {:?} — the two fact sources have drifted",
                name,
                subject,
                flavor,
                cg.get(subject)
            );
            subjects += 1;
        }
        compared += 1;
    }
    assert!(
        compared >= 15 && subjects >= 15,
        "differential covered too little to mean anything: \
         {} programs / {} subjects",
        compared,
        subjects
    );
    let _ = std::fs::remove_dir_all(&work);
}

const BUS_PROG: &str = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "id.evt"; }
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sub = Sub { }; }
    bus { publish Evt; }
    run() { Evt <- T { n: 1 }; println("done"); }
}
fn main() { App { }; }
"#;

/// The recording header's execution identity (4×u64 at offset 56).
fn recorded_exec_digest(rec: &Path) -> [u64; 4] {
    let b = std::fs::read(rec).expect("recording");
    assert!(b.len() >= 112, "truncated recording");
    let mut out = [0u64; 4];
    for (i, part) in out.iter_mut().enumerate() {
        let o = 56 + i * 8;
        *part = u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    }
    out
}

fn record_with(dir: &Path, prog: &Path, tag: &str, devirt: bool) -> PathBuf {
    let rec = dir.join(format!("{}.halerec", tag));
    let mut cmd = hale();
    cmd.arg("run").arg(prog).env("LOTUS_OBS_RECORD", &rec);
    if !devirt {
        cmd.env("LOTUS_NO_BUS_DEVIRT", "1");
    }
    let out = cmd.output().expect("hale run");
    assert!(
        out.status.success(),
        "recorded run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    rec
}

/// Same sources, same toolchain, same options — but one build
/// lowers every subject dynamically and the other devirtualizes.
/// Those are different executables, so they must not share an
/// execution identity, and a recording from one must not be
/// admitted against the other.
#[test]
fn a_different_lowering_is_a_different_build_identity() {
    let dir = workdir("identity");
    let prog = dir.join("app.hl");
    std::fs::write(&prog, BUS_PROG).unwrap();

    let devirt = record_with(&dir, &prog, "devirt", true);
    let dynamic = record_with(&dir, &prog, "dynamic", false);
    let d1 = recorded_exec_digest(&devirt);
    let d2 = recorded_exec_digest(&dynamic);
    assert_ne!(
        d1, d2,
        "the all-dynamic control arm and the devirtualized build \
         stamped the SAME execution identity — the dispatch plan is \
         not part of what a build is"
    );

    // And the boundary is enforced, not merely visible: replaying
    // the all-dynamic recording against the (devirtualized) default
    // build is refused by name.
    let out = hale()
        .arg("replay")
        .arg(&dynamic)
        .arg(&prog)
        // The program prints, so replay's live-effect gate fires
        // first; accept it explicitly to reach the identity check
        // this test is about.
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && err.contains("recorded from different build inputs"),
        "replay across the lowering boundary was admitted:\n{}",
        err
    );
    let _ = std::fs::remove_dir_all(&dir);
}
