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

/// A program that does not typecheck (a downstream handoff: its CI
/// gate reported effect "drift" for one).
const ILL_TYPED: &str = "fn half(n: Int) -> Int { return n / 2; }\n\
fn main() { let s: String = half(4); println(s); }\n";

/// The same program, well typed.
const WELL_TYPED: &str = "fn half(n: Int) -> Int { return n / 2; }\n\
fn main() { let s: Int = half(4); println(s); }\n";

/// Warnings and no error: an unbounded run loop rebuilding a field.
const WARNS_ONLY: &str = r#"type Cell { s: String; n: Int; }
locus Worker {
    params { st: Cell = Cell { s: "", n: 0 }; }
    run() {
        let mut i = 0;
        while true {
            self.st = Cell { s: "v" + i, n: i };
            i = i + 1;
        }
    }
}
main locus App {
    params { w: Worker = Worker { }; }
    placement { w: pinned; }
    run() { }
}
fn main() { App { }; }
"#;

/// Passes plain `check`; `--strict-secret` adds an "uncertified"
/// error (a `@secret` value reaches a call the strict walk does not
/// follow).
const STRICT_SECRET_ONLY: &str = "fn keep(@secret t: String) -> Int { if t == \"open\" { return 1; } return 0; }\n\
fn f(@secret token: String) -> Int { return keep(token); }\n\
fn main() { println(f(\"open\")); }\n";

/// An error the `--strict-secret` pass adds blocks the manifest like
/// any other: the gate judges the same list the diagnostics print.
#[test]
fn a_strict_secret_error_blocks_the_manifest_too() {
    let d = workdir("strict-secret");
    let app = d.join("app.hl");
    let baseline = d.join("baseline.effects");
    std::fs::write(&app, STRICT_SECRET_ONLY).unwrap();
    std::fs::write(&baseline, "# .hale.effects v1 — declared effect contracts\n").unwrap();
    let plain = hale().arg("check").arg(&app).output().expect("check");
    let plain_dump = hale().arg("check").arg(&app).arg("--dump-effects-manifest").output().expect("dump");
    let strict = hale().arg("check").arg(&app).arg("--strict-secret").output().expect("strict");
    let dump = hale()
        .arg("check")
        .arg(&app)
        .arg("--strict-secret")
        .arg("--dump-effects-manifest")
        .output()
        .expect("strict dump");
    let gate = hale()
        .arg("check")
        .arg(&app)
        .arg("--strict-secret")
        .arg("--check-effects-manifest")
        .arg(&baseline)
        .output()
        .expect("strict gate");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(plain.status.code(), Some(0), "the program checks without the flag");
    assert!(
        String::from_utf8_lossy(&plain_dump.stdout).contains("main  does="),
        "without the flag the manifest dumps: {}",
        String::from_utf8_lossy(&plain_dump.stdout)
    );
    assert_eq!(strict.status.code(), Some(1), "the strict walk refuses it");
    for (flag, out) in [("--dump-effects-manifest", &dump), ("--check-effects-manifest", &gate)] {
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.stdout.is_empty(), "{flag}: no manifest on stdout: {}", String::from_utf8_lossy(&out.stdout));
        assert_eq!(
            err.lines().filter(|l| *l == "check failed: no effects manifest").count(),
            1,
            "{flag}: one refusal line: {err}"
        );
        assert!(!err.contains("effect manifest changed"), "{flag}: nothing diffed: {err}");
        assert!(err.contains("uncertified"), "{flag}: the strict error still reports: {err}");
        assert_eq!(out.status.code(), Some(1), "{flag}: the check's exit code: {err}");
    }
}

#[test]
fn a_failed_check_writes_no_manifest_and_exits_with_the_checks_code() {
    let d = workdir("ill-typed");
    let app = d.join("app.hl");
    let baseline = d.join("baseline.effects");
    std::fs::write(&app, ILL_TYPED).unwrap();
    std::fs::write(&baseline, "# .hale.effects v1 — declared effect contracts\n").unwrap();
    let plain = hale().arg("check").arg(&app).output().expect("check");
    let dump = hale().arg("check").arg(&app).arg("--dump-effects-manifest").output().expect("dump");
    let gate = hale()
        .arg("check")
        .arg(&app)
        .arg("--check-effects-manifest")
        .arg(&baseline)
        .output()
        .expect("gate");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(plain.status.code(), Some(1), "the program does not check");
    for (flag, out) in [("--dump-effects-manifest", &dump), ("--check-effects-manifest", &gate)] {
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.stdout.is_empty(), "{flag}: no manifest on stdout: {}", String::from_utf8_lossy(&out.stdout));
        assert_eq!(
            err.lines().filter(|l| *l == "check failed: no effects manifest").count(),
            1,
            "{flag}: one refusal line: {err}"
        );
        assert!(!err.contains("effect manifest changed"), "{flag}: nothing diffed: {err}");
        assert!(err.contains("expected `String`, got `Int`"), "{flag}: the check's own error still reports: {err}");
        assert_eq!(out.status.code(), plain.status.code(), "{flag}: the check's exit code: {err}");
    }
}

#[test]
fn a_clean_check_and_a_warning_only_check_still_dump_the_manifest() {
    let d = workdir("well-typed");
    let app = d.join("app.hl");
    for (src, row) in [(WELL_TYPED, "main  does={syscall}"), (WARNS_ONLY, "Worker::run  does={alloc}")] {
        std::fs::write(&app, src).unwrap();
        let out = hale().arg("check").arg(&app).arg("--dump-effects-manifest").output().expect("dump");
        let (text, err) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "{err}");
        assert!(text.starts_with("# .hale.effects v1") && text.contains(row), "the manifest: {text}");
        assert!(!err.contains("no effects manifest"), "{err}");
        if src == WARNS_ONLY {
            assert!(err.contains("warning: unbounded allocation"), "the control warns: {err}");
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// The api runtime a serving program carries is stdlib source: the
/// manifest of the witness, which serves surfaces, has no `__StdApi*` row
/// under the program's name, and still has the program's own.
#[test]
fn the_appended_api_runtime_is_not_a_manifest_row() {
    let witness = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/hale/api/witness_test.hl");
    let out = hale().arg("check").arg(&witness).arg("--dump-effects-manifest").output().expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("__RpcSurface_2::check"), "the program's own rows are there: {text}");
    let runtime: Vec<&str> = text.lines().filter(|l| l.starts_with("__StdApi") || l.starts_with("__api_rpc_") || l.starts_with("__api_test_")).collect();
    assert!(runtime.is_empty(), "the runtime is not the program's: {runtime:?}");
}
