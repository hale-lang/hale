//! GH #230: per-assertion granularity. A multi-assert test file
//! that fails no longer reads as 0/1 — the failure diagnostic
//! reports how many earlier assertions passed. The pass path
//! stays silent (spec/testing.md contract untouched).

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn failure_reports_earlier_passing_assertions() {
    let src = r#"
        fn main() {
            std::test::assert(1 == 1, "one");
            std::test::assert_eq_int(2, 2, "two");
            std::test::assert_eq_str("a", "a", "three");
            std::test::assert(1 == 2, "boom");
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_assert_granularity");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success());
    assert!(
        stdout.contains("ASSERTION FAILED: boom")
            && stdout.contains("(3 earlier assertion(s) passed)"),
        "expected granular failure trailer.\nstdout: {:?}",
        stdout
    );
}

#[test]
fn passing_file_stays_silent() {
    let src = r#"
        fn main() {
            std::test::assert(1 == 1, "one");
            std::test::assert_eq_int(2, 2, "two");
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_assert_silent");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "passing test must stay silent.\nstdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}
