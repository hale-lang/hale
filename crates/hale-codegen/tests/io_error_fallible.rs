//! #68 — `IoError` payload + fallible `std::io::fs::*` /
//! `std::io::tcp::*` surfaces.
//!
//! Each fs/tcp path-call now returns `fallible(IoError)` instead
//! of the legacy sentinel (-1 / empty string / 0). Agents write
//! `read_file(p) or raise` and the failure carries a structured
//! `IoError { kind, errno, path }` payload — the natural shape
//! that motivated the flip.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, src: &str) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_io_err_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status,
    )
}

#[test]
fn read_file_ok_path_returns_contents() {
    let tmp = harness::unique_bin(&format!("ioerr_ok_{}.txt", std::process::id()));
    std::fs::write(&tmp, "hello world").unwrap();
    let path_str = tmp.to_string_lossy().to_string();
    let src = format!(
        r#"
        fn main() {{
            let s = std::io::fs::read_file("{}") or raise;
            println(s);
        }}
        "#,
        path_str
    );
    let (stdout, _, status) = build_and_run("read_file_ok", &src);
    let _ = std::fs::remove_file(&tmp);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello world"), "got: {:?}", stdout);
}

#[test]
fn read_file_missing_substitute_uses_fallback() {
    // The natural agent shape: substitute the error with a
    // diagnostic-bearing default. `err` is in scope on the RHS.
    let src = r#"
        fn report(e: IoError) -> String {
            return "missing: " + e.kind;
        }
        fn main() {
            let s = std::io::fs::read_file("/no/such/path/ioerr_test")
                or report(err);
            println(s);
        }
    "#;
    let (stdout, _, status) = build_and_run("read_file_missing", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("missing: not_found"), "got: {:?}", stdout);
}

#[test]
fn read_file_missing_or_raise_panics_at_root() {
    let src = r#"
        fn main() {
            let _ = std::io::fs::read_file("/no/such/path/ioerr_test")
                or raise;
            println("UNREACHABLE");
        }
    "#;
    let (stdout, stderr, status) = build_and_run("read_file_panic", src);
    assert!(!status.success(), "expected non-zero exit; got: {:?}", status);
    assert!(!stdout.contains("UNREACHABLE"), "stdout: {:?}", stdout);
    assert!(
        stderr.contains("IoError") || stderr.contains("Hale panic"),
        "stderr: {:?}",
        stderr
    );
}

#[test]
fn ioerror_carries_errno_and_path_fields() {
    let src = r#"
        fn diagnose(e: IoError) -> String {
            return "kind=" + e.kind + " path=" + e.path;
        }
        fn main() {
            let s = std::io::fs::read_file("/no/such/ioerr_diag")
                or diagnose(err);
            println(s);
        }
    "#;
    let (stdout, _, status) = build_and_run("ioerr_fields", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("kind=not_found"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=/no/such/ioerr_diag"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn write_file_then_read_roundtrip_via_fallible() {
    let tmp = harness::unique_bin(&format!("ioerr_wf_{}.txt", std::process::id()));
    let path_str = tmp.to_string_lossy().to_string();
    let src = format!(
        r#"
        fn main() {{
            std::io::fs::write_file("{}", "round-trip") or raise;
            let s = std::io::fs::read_file("{}") or raise;
            println(s);
        }}
        "#,
        path_str, path_str
    );
    let (stdout, _, status) = build_and_run("wf_rt", &src);
    let _ = std::fs::remove_file(&tmp);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("round-trip"), "got: {:?}", stdout);
}

#[test]
fn mkdir_fallible_emits_already_exists_kind_on_existing_dir() {
    // mkdir's success is Unit, so the `or` substitute RHS has to
    // be Unit-typed too — a void helper that prints the IoError.
    let tmp = harness::unique_bin(&format!("ioerr_md_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let path_str = tmp.to_string_lossy().to_string();
    let src = format!(
        r#"
        fn show(e: IoError) {{
            println("kind=", e.kind);
        }}
        fn main() {{
            std::io::fs::mkdir("{}") or show(err);
        }}
        "#,
        path_str
    );
    let (stdout, _, status) = build_and_run("md_exists", &src);
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("kind=already_exists"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn file_size_ok_path_returns_size() {
    let tmp = harness::unique_bin(&format!("ioerr_fs_{}.txt", std::process::id()));
    std::fs::write(&tmp, "12345").unwrap();
    let path_str = tmp.to_string_lossy().to_string();
    let src = format!(
        r#"
        fn main() {{
            let n = std::io::fs::file_size("{}") or raise;
            println("size=", n);
        }}
        "#,
        path_str
    );
    let (stdout, _, status) = build_and_run("file_size_ok", &src);
    let _ = std::fs::remove_file(&tmp);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("size=5"), "got: {:?}", stdout);
}

#[test]
fn bytes_at_fallible_returns_byte_on_ok_indexerror_on_oob() {
    // bytes::at flipped from -1 sentinel to fallible(IndexError).
    // Same shape agents reach for over vec.get / pop.
    let src = r#"
        fn main() {
            let b = std::bytes::from_string("ab");
            let first = std::bytes::at(b, 0) or raise;
            println("first=", first);
            let bad = std::bytes::at(b, 9) or 999;
            println("bad=", bad);
        }
    "#;
    let (stdout, _, status) = build_and_run("bytes_at_fallible", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("first=97"), "got: {:?}", stdout);
    assert!(stdout.contains("bad=999"), "got: {:?}", stdout);
}

#[test]
fn or_over_non_fallible_path_call_has_clear_diagnostic() {
    // Friendly diagnostic when the agent wraps a non-fallible
    // stdlib path in `or`. Names the call and tells them to
    // remove the `or` clause.
    let src = r#"
        fn main() {
            let _ = std::str::lower("HI") or raise;
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_or_diag_{}", std::process::id()));
    let err = hale_codegen::build_executable(&program, &bin).expect_err("should reject");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{:?}", err);
    assert!(msg.contains("not a fallible call"), "got: {}", msg);
    assert!(msg.contains("std::str::lower"), "got: {}", msg);
}

#[test]
fn str_bytes_mismatch_diagnostic_suggests_converter() {
    let src = r#"
        fn main() {
            let b = std::bytes::from_string("hi");
            let _ = std::str::lower(b);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_str_diag_{}", std::process::id()));
    let err = hale_codegen::build_executable(&program, &bin).expect_err("should reject");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{:?}", err);
    assert!(msg.contains("from_bytes"), "got: {}", msg);
}

#[test]
fn bytes_str_mismatch_diagnostic_suggests_converter() {
    let src = r#"
        fn main() {
            let n = std::bytes::at("hi", 0);
            println(n);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_bytes_diag_{}", std::process::id()));
    let err = hale_codegen::build_executable(&program, &bin).expect_err("should reject");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{:?}", err);
    assert!(msg.contains("from_string"), "got: {}", msg);
}

#[test]
fn missing_std_prefix_diagnostic_suggests_correction() {
    // `env::args_count` (without std::) typo — diagnostic should
    // suggest std::env::args_count instead of "unknown path".
    let src = r#"
        fn main() {
            let n = env::args_count();
            println(n);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_typo_{}", std::process::id()));
    let err = hale_codegen::build_executable(&program, &bin).expect_err("should reject");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{:?}", err);
    assert!(msg.contains("did you mean"), "got: {}", msg);
    assert!(msg.contains("std::env::args_count"), "got: {}", msg);
}

#[test]
fn file_exists_unchanged_still_returns_bool() {
    // file_exists stays a predicate (per the IoError flip's
    // scope): it's not a failable operation.
    let src = r#"
        fn main() {
            if std::io::fs::file_exists("/no/such/path/xyz") {
                println("FAIL: shouldn't exist");
            }
            println("ok");
        }
    "#;
    let (stdout, _, status) = build_and_run("file_exists_bool", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("ok"), "got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "got: {:?}", stdout);
}
