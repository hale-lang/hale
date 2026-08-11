//! Downstream handoff (2026-08-11) — two CLI diagnostic-reporting
//! defects, both worst on the duplicate-top-level-name error (the
//! easiest error to hit under the per-directory seed model):
//!
//! 1. The previous declaration's location was `{:?}`-formatted into
//!    the message (`... at Span { start: Pos(5), end: Pos(11) }`).
//!    It now rides as a structured related span, rendered by the
//!    text renderer as `note: previous declaration at path:line:col`
//!    and by `--json` as a `related` array.
//! 2. The `apply_sync_inference` pre-pass printed its resolver
//!    diagnostics through a bare `render()` + early bail: no
//!    filename, `--json` ignored (empty stdout, exit 1 — a CI gate
//!    saw a failed build with zero explaining diagnostics), wrong
//!    stream, and multi-file positions resolved against the wrong
//!    file's text. The pre-pass diags are now discarded and
//!    re-raised by `check_bundle` through the normal reporting path
//!    — exactly what `hale lsp` always did, which is why the LSP
//!    attributed the same diagnostic correctly while the CLI did
//!    not.

use std::path::{Path, PathBuf};
use std::process::Command;

fn seed_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_diagrep_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir seed");
    d
}

/// (stdout, stderr, exit code)
fn hale_check(args: &[&str], target: &Path) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .args(args)
        .arg(target)
        .output()
        .expect("run hale check");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn duplicate_name_json_emits_ndjson_with_its_file() {
    // Two files in one seed, each declaring `main`. `--json` must
    // emit NDJSON on stdout naming the SECOND file at a position
    // inside it — not plain text on stderr, and never an empty
    // stdout with exit 1.
    let d = seed_dir("json");
    std::fs::write(d.join("a.hl"), "fn main() {\n    println(\"a\");\n}\n")
        .unwrap();
    std::fs::write(d.join("b.hl"), "fn main() {\n    println(\"b\");\n}\n")
        .unwrap();

    let (stdout, stderr, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1, "duplicate main fails the check");
    assert!(
        !stdout.trim().is_empty(),
        "--json must not produce an empty stdout on a failing check \
         (stderr was: {})",
        stderr
    );
    let line = stdout.lines().next().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(line).expect("stdout is NDJSON");
    assert!(
        v["file"].as_str().unwrap().ends_with("b.hl"),
        "the second declaration is the one to change: {}",
        line
    );
    assert_eq!(v["line"], 1, "a position inside b.hl: {}", line);
    assert!(
        v["message"].as_str().unwrap().contains("duplicate top-level"),
        "got: {}",
        line
    );
    // The previous declaration rides as structured related info.
    assert!(
        v["related"][0]["file"].as_str().unwrap().ends_with("a.hl"),
        "related names the first declaration's file: {}",
        line
    );
    assert!(
        !stderr.contains("duplicate top-level"),
        "diagnostics belong on stdout under --json, not stderr: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn duplicate_name_plain_names_both_locations() {
    let d = seed_dir("plain");
    std::fs::write(
        d.join("a.hl"),
        "type Widget { n: Int; }\n\nfn main() {\n    println(\"a\");\n}\n",
    )
    .unwrap();
    std::fs::write(d.join("b.hl"), "type Widget { m: Int; }\n").unwrap();

    let (stdout, stderr, code) = hale_check(&[], &d);
    let all = format!("{}{}", stdout, stderr);
    assert_eq!(code, 1);
    assert!(
        all.contains("b.hl:1:6"),
        "the duplicate resolves to ITS file's coordinates: {}",
        all
    );
    assert!(
        all.contains("note: previous declaration at")
            && all.contains("a.hl:1:6"),
        "the first declaration renders as path:line:col: {}",
        all
    );
    assert!(
        !all.contains("Span {"),
        "no Debug-formatted span may reach a user: {}",
        all
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ordinary_json_shape_is_unchanged() {
    // The control from the report: an ordinary error keeps its
    // exact NDJSON shape, `related`-free.
    let d = seed_dir("control");
    std::fs::write(
        d.join("a.hl"),
        "fn main() {\n    let x: Int = \"nope\";\n    println(x);\n}\n",
    )
    .unwrap();

    let (stdout, _, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1);
    let v: serde_json::Value =
        serde_json::from_str(stdout.lines().next().unwrap()).expect("NDJSON");
    assert!(v.get("related").is_none(), "no related key when empty: {}", v);
    assert!(v["file"].as_str().unwrap().ends_with("a.hl"));
    let _ = std::fs::remove_dir_all(&d);
}

/// Downstream handoff (2026-08-11), soundness follow-through: the
/// handler/payload mismatch must refuse to BUILD, not just fail
/// `check` — before the fix it built and printed a live heap
/// address from safe code.
#[test]
fn payload_mismatch_refuses_to_build() {
    let d = seed_dir("buspay");
    std::fs::write(
        d.join("main.hl"),
        r#"
        type Greeting { text: String = "hello"; n: Int = 42; }
        type Other { a: Int = 0; b: Int = 0; }
        topic Hello { payload: Greeting; subject: "hello"; }
        locus Pub {
            bus { publish Hello; }
            birth() { Hello <- Greeting { text: "P", n: 7 }; }
        }
        locus Sub {
            bus { subscribe Hello as on_hello; }
            fn on_hello(msg: Other) { println("a=", msg.a); }
        }
        fn main() { Sub { }; Pub { }; }
        "#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&d)
        .output()
        .expect("run hale build");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "the reinterpreting program must not build: {}",
        all
    );
    assert!(
        all.contains("carries payload `Greeting`"),
        "the refusal names the mismatch: {}",
        all
    );
    let _ = std::fs::remove_dir_all(&d);
}
