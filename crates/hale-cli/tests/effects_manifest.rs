//! GH #265 step 7 — the `.hale.effects` manifest and its CI gate.
//!
//! The manifest is a **behavioural fingerprint**: declared contracts
//! plus INFERRED effect sets, stable-sorted. Its value is the diff —
//! a handler that quietly gains a syscall shows up in review the way
//! an API break shows in a `.d.ts` diff, even though no annotation
//! changed.

use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

const CLEAN: &str = r#"
type Ev { n: Int; }
topic T { payload: Ev; subject: "t"; }
locus Sink {
    bus { subscribe T as on_t; }
    fn on_t(e: Ev) {
        std::io::fs::write_file("/tmp/hale-manifest-test", "x") or discard;
    }
}
locus Api {
    bus { publish T; }
    @no_block fn emit(n: Int) {
        T <- Ev { n: n };
    }
}
fn main() { Sink { }; Api { }; }
"#;

/// The regressed variant: `Api::emit` silently gains a filesystem
/// write. No annotation changes — only the inferred set does.
const REGRESSED: &str = r#"
type Ev { n: Int; }
topic T { payload: Ev; subject: "t"; }
locus Sink {
    bus { subscribe T as on_t; }
    fn on_t(e: Ev) {
        std::io::fs::write_file("/tmp/hale-manifest-test", "x") or discard;
    }
}
locus Api {
    bus { publish T; }
    @no_block fn emit(n: Int) {
        T <- Ev { n: n };
        std::io::fs::write_file("/tmp/hale-sneaky", "x") or discard;
    }
}
fn main() { Sink { }; Api { }; }
"#;

fn workdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale-manifest-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn manifest_reports_inferred_effects_without_annotations() {
    let d = workdir("dump");
    let app = d.join("app.hl");
    std::fs::write(&app, CLEAN).unwrap();
    let out = hale()
        .arg("check")
        .arg(&app)
        .arg("--dump-effects-manifest")
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    let _ = std::fs::remove_dir_all(&d);
    // Declared contract shows up …
    assert!(
        text.contains("Api::emit") && text.contains("none={block}"),
        "declared contract missing: {}",
        text
    );
    // … and so does the INFERRED set of a fn with no annotation at
    // all. That is what makes this a fingerprint rather than an
    // annotation dump.
    assert!(
        text.contains("Sink::on_t") && text.contains("does={syscall}"),
        "inferred effect of an unannotated handler missing: {}",
        text
    );
}

#[test]
fn manifest_gate_passes_unchanged_and_catches_a_silent_regression() {
    let d = workdir("gate");
    let app = d.join("app.hl");
    let baseline = d.join("baseline.effects");
    std::fs::write(&app, CLEAN).unwrap();

    // Record the baseline.
    let dump = hale()
        .arg("check")
        .arg(&app)
        .arg("--dump-effects-manifest")
        .output()
        .expect("dump");
    std::fs::write(&baseline, &dump.stdout).unwrap();

    // Unchanged program → gate passes.
    let ok = hale()
        .arg("check")
        .arg(&app)
        .arg("--check-effects-manifest")
        .arg(&baseline)
        .output()
        .expect("gate");
    assert!(
        ok.status.success(),
        "unchanged program must pass the gate. stderr: {}",
        String::from_utf8_lossy(&ok.stderr)
    );

    // A handler silently gains a syscall → gate fails, naming it.
    std::fs::write(&app, REGRESSED).unwrap();
    let bad = hale()
        .arg("check")
        .arg(&app)
        .arg("--check-effects-manifest")
        .arg(&baseline)
        .output()
        .expect("gate");
    let err = String::from_utf8_lossy(&bad.stderr).to_string();
    let _ = std::fs::remove_dir_all(&d);
    assert!(
        !bad.status.success(),
        "a silent effect regression must fail the gate"
    );
    assert!(
        err.contains("effect manifest changed")
            && err.contains("Api::emit")
            && err.contains("syscall"),
        "the diff must name the fn and the gained effect: {}",
        err
    );
}
