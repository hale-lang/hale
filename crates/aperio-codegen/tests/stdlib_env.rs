//! m77: std::env basics — args + var.
//!
//! `std::env::args_count` / `arg(i)` / `var(name)` /
//! `var_exists(name)` reach process-level state captured in
//! main's prelude (argc/argv) or via libc getenv. Each
//! function follows the same path-call pattern as
//! std::process::pid and std::io::fs::*.
//!
//! These tests build small Aperio programs that consume the
//! env surface and assert on stdout. The tested binaries are
//! invoked with controlled argv / env so the assertions are
//! deterministic.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_env_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn args_count_reflects_argv() {
    let src = r#"
        fn main() {
            let n = std::env::args_count();
            println("argc=", n);
        }
    "#;
    let bin = build_aperio("argc", src);
    // Spawn with three extra args. The binary path itself is
    // argv[0], so args_count should report 4.
    let out = Command::new(&bin)
        .args(["alpha", "beta", "gamma"])
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("argc=4"),
        "expected argc=4 (bin + 3 args); got: {:?}",
        stdout
    );
}

#[test]
fn arg_returns_each_argv_entry_and_empty_for_out_of_range() {
    let src = r#"
        fn main() {
            let a1 = std::env::arg(1);
            let a2 = std::env::arg(2);
            let a99 = std::env::arg(99);
            let neg = std::env::arg(-5);
            println("a1=", a1);
            println("a2=", a2);
            println("a99=[", a99, "]");
            println("neg=[", neg, "]");
        }
    "#;
    let bin = build_aperio("arg_indexed", src);
    let out = Command::new(&bin)
        .args(["first", "second"])
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a1=first"), "got: {:?}", stdout);
    assert!(stdout.contains("a2=second"), "got: {:?}", stdout);
    assert!(stdout.contains("a99=[]"), "out-of-range arg should be empty; got: {:?}", stdout);
    assert!(stdout.contains("neg=[]"), "negative arg should be empty; got: {:?}", stdout);
}

#[test]
fn var_returns_environment_value() {
    let src = r#"
        fn main() {
            let v = std::env::var("APERIO_TEST_FOO");
            println("v=[", v, "]");
        }
    "#;
    let bin = build_aperio("var_present", src);
    let out = Command::new(&bin)
        .env("APERIO_TEST_FOO", "bar-baz")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("v=[bar-baz]"),
        "expected env value in stdout; got: {:?}",
        stdout
    );
}

#[test]
fn var_returns_empty_for_unset_variable() {
    let src = r#"
        fn main() {
            let v = std::env::var("APERIO_DEFINITELY_UNSET_VARIABLE_XYZZY");
            println("v=[", v, "]");
        }
    "#;
    let bin = build_aperio("var_absent", src);
    let out = Command::new(&bin)
        .env_remove("APERIO_DEFINITELY_UNSET_VARIABLE_XYZZY")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("v=[]"),
        "expected empty value for unset env; got: {:?}",
        stdout
    );
}

#[test]
fn var_exists_distinguishes_present_from_absent() {
    let src = r#"
        fn main() {
            let p = std::env::var_exists("APERIO_TEST_PRESENT");
            let a = std::env::var_exists("APERIO_DEFINITELY_NEVER_SET");
            println("present=", p);
            println("absent=", a);
        }
    "#;
    let bin = build_aperio("var_exists", src);
    let out = Command::new(&bin)
        .env("APERIO_TEST_PRESENT", "x")
        .env_remove("APERIO_DEFINITELY_NEVER_SET")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("present=true"), "got: {:?}", stdout);
    assert!(stdout.contains("absent=false"), "got: {:?}", stdout);
}

#[test]
fn arg_zero_is_a_path_to_the_binary() {
    // argv[0] is the binary path on most platforms. We don't
    // assert on the exact value (varies by exec mechanism) but
    // it must be non-empty.
    let src = r#"
        fn main() {
            let bin = std::env::arg(0);
            let n = std::env::args_count();
            if n > 0 {
                println("argv0_present=true");
                println("argv0=", bin);
            } else {
                println("argv0_present=false");
            }
        }
    "#;
    let bin = build_aperio("argv0", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("argv0_present=true"),
        "argv0 should always exist; got: {:?}",
        stdout
    );
}
