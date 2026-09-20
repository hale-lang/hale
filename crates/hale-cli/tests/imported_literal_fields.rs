//! GH #707 — a qualified imported struct literal is validated against
//! the imported declaration, not waved through.
//!
//! `check_struct_literal` resolved `alias::Type { ... }` to the merged
//! (mangled) symbol for the RESULT type only and never looked at the
//! initializers, so an unknown field name in an imported literal
//! silently constructed the declared default: `alias::Resume { key:
//! "main" }` — a typo for `scope` — passed `hale check` and ran,
//! printing the empty default. The same literal on a LOCAL type was
//! rejected with "has no field". Reported from a downstream handoff,
//! where the typo sat in a recovery signal and the compile gate was
//! the only thing that could have caught it.
//!
//! Locked in here: unknown field name, wrong field type, an unknown
//! field in a NESTED imported literal, the located span in `--json`,
//! the `verify` gate — and that valid fields plus defaults still pass.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The library seed both cases import.
const LIB: &str = r#"
type Inner { n: Int = 0; }
type Resume { scope: String = ""; inner: Inner = Inner { }; }
"#;

/// Every `main.hl` below starts with a newline, so the offending
/// literal is on line 4 (1: blank, 2: import, 3: `fn main()`).
const UNKNOWN_FIELD: &str = r#"
import "lib" as sample;
fn main() {
    let r = sample::Resume { key: "main" };
    println("scope=[", r.scope, "]");
}
"#;

const WRONG_TYPE: &str = r#"
import "lib" as sample;
fn main() {
    let r = sample::Resume { scope: 7 };
    println("scope=[", r.scope, "]");
}
"#;

const NESTED_UNKNOWN_FIELD: &str = r#"
import "lib" as sample;
fn main() {
    let r = sample::Resume { inner: sample::Inner { m: 3 } };
    println("n=", to_string(r.inner.n));
}
"#;

const VALID: &str = r#"
import "lib" as sample;
fn main() {
    let r = sample::Resume { scope: "main", inner: sample::Inner { n: 3 } };
    let d = sample::Resume { };
    println("scope=[", r.scope, "] n=", to_string(r.inner.n),
            " d=[", d.scope, "]");
}
"#;

/// An app seed whose `main.hl` imports a sibling `lib/` seed.
fn app_seed(tag: &str, main_src: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_i707_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).expect("mkdir seed");
    std::fs::write(d.join("lib").join("types.hl"), LIB).expect("write lib");
    std::fs::write(d.join("main.hl"), main_src).expect("write main");
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
fn unknown_field_in_imported_literal_fails_check() {
    let d = app_seed("unknown", UNKNOWN_FIELD);
    let (out, code) = run("check", &d, &[]);
    assert_ne!(code, 0, "check must reject the typo:\n{}", out);
    // Demangled to the spelling the author wrote, and located on the
    // initializer rather than on the whole literal.
    assert!(
        out.contains("type `sample::Resume` has no field `key`"),
        "expected the local-type diagnostic, demangled:\n{}",
        out
    );
    assert!(
        out.contains("main.hl:4:"),
        "expected a located diagnostic in main.hl:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn unknown_field_in_imported_literal_fails_verify() {
    // `verify` runs the same checker: the discipline gate must see
    // this too, not just `check`.
    let d = app_seed("verify", UNKNOWN_FIELD);
    let (out, code) = run("verify", &d, &[]);
    assert_ne!(code, 0, "verify must reject the typo:\n{}", out);
    assert!(
        out.contains("type `sample::Resume` has no field `key`"),
        "expected the diagnostic under verify:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn json_diagnostic_carries_the_span() {
    let d = app_seed("json", UNKNOWN_FIELD);
    let (out, code) = run("check", &d, &["--json"]);
    assert_ne!(code, 0, "check --json must reject the typo:\n{}", out);
    let line = out
        .lines()
        .find(|l| l.contains("has no field"))
        .unwrap_or_else(|| panic!("no JSON diagnostic:\n{}", out));
    assert!(
        line.contains("\"line\":4") && line.contains("\"col\":"),
        "JSON diagnostic must carry the span: {}",
        line
    );
    assert!(
        line.contains("main.hl"),
        "JSON diagnostic must name the file: {}",
        line
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wrong_field_type_in_imported_literal_fails_check() {
    let d = app_seed("wrongty", WRONG_TYPE);
    let (out, code) = run("check", &d, &[]);
    assert_ne!(code, 0, "check must reject the wrong field type:\n{}", out);
    assert!(
        out.contains("field `scope` expects `String`, got `Int`"),
        "expected the type-mismatch diagnostic:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn unknown_field_in_nested_imported_literal_fails_check() {
    // The inner literal is an imported type reached through a field
    // of another imported type.
    let d = app_seed("nested", NESTED_UNKNOWN_FIELD);
    let (out, code) = run("check", &d, &[]);
    assert_ne!(code, 0, "check must reject the nested typo:\n{}", out);
    assert!(
        out.contains("type `sample::Inner` has no field `m`"),
        "expected the nested diagnostic:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn valid_imported_fields_and_defaults_still_check_and_run() {
    let d = app_seed("ok", VALID);
    let (out, code) = run("check", &d, &[]);
    assert_eq!(code, 0, "valid imported literals must still pass:\n{}", out);

    let (run_out, run_code) = run("run", &d.join("main.hl"), &[]);
    assert_eq!(run_code, 0, "run failed:\n{}", run_out);
    assert!(
        run_out.contains("scope=[main] n=3 d=[]"),
        "unexpected program output:\n{}",
        run_out
    );
    let _ = std::fs::remove_dir_all(&d);
}
