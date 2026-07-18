//! `hale fmt` CLI surface: --stdin round-trip, --check exit codes,
//! and in-place formatting. The formatter core (spacing rules,
//! idempotence, the token-equivalence gate) is anchored by
//! hale-syntax's unit tests + the corpus test; this covers the
//! command-line contract editors and CI hooks depend on.

use std::io::Write;
use std::process::{Command, Stdio};

const MESSY: &str = "fn main(){\nlet x=1+2;\nprintln( \"v\" ,x );\n}\n";
const CANON: &str =
    "fn main() {\n    let x = 1 + 2;\n    println(\"v\", x);\n}\n";

#[test]
fn stdin_formats_to_stdout() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["fmt", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(MESSY.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), CANON);
}

#[test]
fn check_mode_exits_nonzero_then_write_fixes() {
    let dir = std::env::temp_dir().join(format!(
        "hale_fmt_cli_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("messy.hl");
    std::fs::write(&f, MESSY).expect("write fixture");

    // --check: reports the file, exits 1, does NOT modify it.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["fmt", "--check"])
        .arg(&f)
        .output()
        .expect("run check");
    assert_eq!(out.status.code(), Some(1), "check must fail on messy");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("messy.hl"),
        "check lists the offender"
    );
    assert_eq!(std::fs::read_to_string(&f).expect("read"), MESSY);

    // Plain fmt: writes canonical form in place.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("fmt")
        .arg(&f)
        .output()
        .expect("run fmt");
    assert!(out.status.success());
    assert_eq!(std::fs::read_to_string(&f).expect("read"), CANON);

    // --check on canonical: clean exit, no output.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["fmt", "--check"])
        .arg(&f)
        .output()
        .expect("run check 2");
    assert!(out.status.success(), "canonical file passes --check");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unlexable_file_is_skipped_with_error() {
    let dir = std::env::temp_dir().join(format!(
        "hale_fmt_cli_bad_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("bad.hl");
    let bad = "fn main() { let s = \"unterminated; }\n";
    std::fs::write(&f, bad).expect("write fixture");

    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("fmt")
        .arg(&f)
        .output()
        .expect("run fmt");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not lex"),
        "reports the lex failure"
    );
    // File untouched.
    assert_eq!(std::fs::read_to_string(&f).expect("read"), bad);

    let _ = std::fs::remove_dir_all(&dir);
}
