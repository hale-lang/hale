//! m79: std::time::* aliases + std::process::exit.

use std::process::Command;
use std::time::Instant;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_m79_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn std_time_sleep_blocks_for_at_least_requested_duration() {
    let src = r#"
        fn main() {
            println("before");
            std::time::sleep(50ms);
            println("after");
        }
    "#;
    let bin = build_aperio("std_time_sleep", src);
    let start = Instant::now();
    let out = Command::new(&bin).output().expect("run");
    let elapsed = start.elapsed();
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("before"), "got: {:?}", stdout);
    assert!(stdout.contains("after"), "got: {:?}", stdout);
    assert!(
        elapsed.as_millis() >= 50,
        "sleep returned too early: {:?}",
        elapsed
    );
}

#[test]
fn std_time_monotonic_returns_duration() {
    let src = r#"
        fn main() {
            let t0 = std::time::monotonic();
            std::time::sleep(20ms);
            let t1 = std::time::monotonic();
            let elapsed = t1 - t0;
            if elapsed > 20ms {
                println("ok");
            } else {
                println("fail");
            }
        }
    "#;
    let bin = build_aperio("std_time_monotonic", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "got: {:?}", stdout);
    assert!(!stdout.contains("fail"), "got: {:?}", stdout);
}

#[test]
fn std_process_exit_terminates_with_specified_code() {
    let src = r#"
        fn main() {
            println("before exit");
            std::process::exit(7);
            println("after exit");
        }
    "#;
    let bin = build_aperio("std_process_exit", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    // exit(7) → process exit code 7 (low byte of status on
    // POSIX, mapped through Rust's ExitStatus::code()).
    let code = out.status.code().expect("exit code present");
    assert_eq!(code, 7, "expected exit code 7; got: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("before exit"), "got: {:?}", stdout);
    assert!(
        !stdout.contains("after exit"),
        "code after exit() must not run; got: {:?}",
        stdout
    );
}

#[test]
fn std_process_exit_zero_is_clean_exit() {
    // Exit 0 also flows through the same path; ensures we
    // didn't accidentally hardcode non-zero somewhere.
    let src = r#"
        fn main() {
            std::process::exit(0);
        }
    "#;
    let bin = build_aperio("std_process_exit_zero", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success(), "exit(0) should be success: {:?}", out.status);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn legacy_time_sleep_still_works() {
    // Pre-m79 programs use `time::sleep(...)` directly. The
    // legacy dispatcher is still in lower_path_call so existing
    // examples and tests don't need to be touched. This test
    // locks that in.
    let src = r#"
        fn main() {
            time::sleep(20ms);
            println("legacy sleep ok");
        }
    "#;
    let bin = build_aperio("legacy_time", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("legacy sleep ok"), "got: {:?}", stdout);
}
