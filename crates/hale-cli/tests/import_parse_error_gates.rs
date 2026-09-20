//! GH #765 — `check` and `verify` fail when anything in the import
//! graph fails to parse.
//!
//! They used to report success. `resolve_imports` reports an imported
//! file's parse failure by pushing it into an `errors` out-parameter
//! and continuing — it still returns `Ok` — and `collect_checkable`,
//! the path `check` and `verify` share, tested only the `Err`. The
//! populated vector was never read, so the library's declarations were
//! simply absent from the merged program, the checker's tolerance for
//! unresolved qualified references (`lib::double(4)`) hid every
//! consequence, and the gate answered `ok: 1 file(s) typechecked`,
//! exit 0, on a tree `hale build` refused. An admission gate built on
//! check + verify was fail-open for any change that broke a library's
//! syntax. The other three call sites of `resolve_imports` test the
//! vector; this was the one they didn't.
//!
//! Pinned here: the failure itself, the LIBRARY file and its real
//! line/col in both renderings (a parse error's span used to be shifted
//! by `base` twice, so it demultiplexed to no file at all — see
//! `hale-syntax/tests/parse_source_at.rs`), `--json`, the `verify`
//! gate, a nested import two hops down, that a valid import still
//! checks clean, and that `build` still refuses.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The library seed the consumer imports. Missing the `;` before `}`,
/// so it does not parse — the error is at line 1, column 41.
const LIB_BROKEN: &str = "fn double(x: Int) -> Int { return x * 2 }\n";

const LIB_OK: &str = "fn double(x: Int) -> Int { return x * 2; }\n";

const CONSUMER: &str = r#"
import "lib" as lib;

main locus App {
    run() { println("d=", lib::double(4)); }
}

fn main() { App { }; }
"#;

/// A consumer whose library is itself a consumer of a broken seed:
/// the parse failure is two hops from the target.
const MID: &str = r#"
import "inner" as inner;

fn mid(x: Int) -> Int { return inner::double(x); }
"#;

const CONSUMER_OF_MID: &str = r#"
import "mid" as mid;

main locus App {
    run() { println("d=", mid::mid(4)); }
}

fn main() { App { }; }
"#;

/// A two-seed app: `<tmp>/main.hl` plus `<tmp>/lib/main.hl`.
fn seed(tag: &str, lib: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_i765_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).expect("mkdir seed");
    std::fs::write(d.join("lib").join("main.hl"), lib).expect("write lib");
    std::fs::write(d.join("main.hl"), CONSUMER).expect("write main");
    d
}

/// `<tmp>/main.hl` -> `mid/` -> `mid/inner/`, with the break at the
/// bottom. An import resolves relative to the seed that writes it, so
/// `inner` is a child of `mid`.
fn nested_seed(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_i765_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("mid").join("inner")).expect("mkdir");
    std::fs::write(d.join("mid").join("inner").join("main.hl"), LIB_BROKEN)
        .unwrap();
    std::fs::write(d.join("mid").join("main.hl"), MID).unwrap();
    std::fs::write(d.join("main.hl"), CONSUMER_OF_MID).unwrap();
    d
}

fn run(cmd: &str, target: &Path, extra: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg(cmd)
        .args(extra)
        .arg(target)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn check_fails_when_an_imported_library_does_not_parse() {
    let d = seed("check", LIB_BROKEN);
    let (out, code) = run("check", &d, &[]);
    assert_ne!(code, 0, "check must refuse the tree:\n{}", out);
    assert!(
        !out.contains("typechecked"),
        "check must not also report success:\n{}",
        out
    );
    // The LIBRARY file, at its own line and column — not the entry
    // file, and not an unlocated bare message.
    assert!(
        out.contains("lib/main.hl:1:41: parse error: expected ;, got RBrace"),
        "expected the located parse error from the library:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn verify_fails_when_an_imported_library_does_not_parse() {
    // `verify` is the discipline gate and shares the same collector:
    // it was fail-open in exactly the same way.
    let d = seed("verify", LIB_BROKEN);
    let (out, code) = run("verify", &d, &[]);
    assert_ne!(code, 0, "verify must refuse the tree:\n{}", out);
    assert!(
        !out.contains("0 findings"),
        "verify must not report a clean run:\n{}",
        out
    );
    assert!(
        out.contains("lib/main.hl:1:41: parse error:"),
        "expected the located parse error under verify:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn json_carries_the_library_file_line_and_message() {
    let d = seed("json", LIB_BROKEN);
    let (out, code) = run("check", &d, &["--json"]);
    assert_ne!(code, 0, "check --json must refuse the tree:\n{}", out);
    let line = out
        .lines()
        .find(|l| l.contains("parse error"))
        .unwrap_or_else(|| panic!("no JSON diagnostic at all:\n{}", out));
    assert!(
        line.contains("lib/main.hl"),
        "JSON must name the library file: {}",
        line
    );
    assert!(
        line.contains("\"line\":1") && line.contains("\"col\":41"),
        "JSON must carry the library-local position: {}",
        line
    );
    assert!(
        line.contains("\"severity\":\"error\"")
            && line.contains("expected ;, got RBrace"),
        "JSON must carry severity and message: {}",
        line
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_parse_failure_two_hops_down_still_fails_check() {
    let d = nested_seed("nested");
    let (out, code) = run("check", &d, &[]);
    assert_ne!(code, 0, "check must refuse a break two hops down:\n{}", out);
    assert!(
        out.contains("inner/main.hl:1:41: parse error:"),
        "expected the innermost seed's located parse error:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_valid_import_still_checks_clean() {
    // The guard must not cost a working program: same two seeds, one
    // semicolon added.
    let d = seed("valid", LIB_OK);
    let (out, code) = run("check", &d, &[]);
    assert_eq!(code, 0, "a valid import must still check:\n{}", out);
    assert!(
        out.contains("typechecked"),
        "expected the success line:\n{}",
        out
    );
    let (out, code) = run("verify", &d, &[]);
    assert_eq!(code, 0, "a valid import must still verify:\n{}", out);
    assert!(out.contains("0 findings"), "expected a clean verify:\n{}", out);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn build_still_refuses_the_same_tree() {
    // `build` always caught this — it tests the same vector — and must
    // keep doing so. Check and build now agree about the tree instead
    // of disagreeing.
    let d = seed("build", LIB_BROKEN);
    let (out, code) = run("build", &d, &[]);
    assert_ne!(code, 0, "build must still refuse the tree:\n{}", out);
    assert!(
        out.contains("parse error: expected ;, got RBrace"),
        "expected the parse error from build:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}
