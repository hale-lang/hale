//! v1.x-11: explicit Float → Int narrowing via `Int(x)`.
//!
//! Truncates toward zero (matches LLVM fptosi semantics). The
//! cast is opt-in — there is no implicit Float → Int conversion;
//! the user has to write `Int(...)` to commit to the narrowing.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_fp_to_int_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn float_truncates_toward_zero_positive() {
    let src = r#"
        fn main() {
            let f = 3.7;
            let n = Int(f);
            println(n);
        }
    "#;
    let (stdout, status) = build_and_run("pos", src);
    assert!(status.success());
    assert!(stdout.trim() == "3", "got: {:?}", stdout);
}

#[test]
fn float_truncates_toward_zero_negative() {
    let src = r#"
        fn main() {
            let f = 0.0 - 3.7;
            let n = Int(f);
            println(n);
        }
    "#;
    let (stdout, status) = build_and_run("neg", src);
    assert!(status.success());
    assert!(stdout.trim() == "-3", "got: {:?}", stdout);
}

#[test]
fn int_arg_is_identity() {
    let src = r#"
        fn main() {
            let n = Int(42);
            println(n);
        }
    "#;
    let (stdout, status) = build_and_run("identity", src);
    assert!(status.success());
    assert!(stdout.trim() == "42", "got: {:?}", stdout);
}

#[test]
fn int_in_arithmetic_chain() {
    // Result must usable directly in Int arithmetic; this exercises
    // the contract that the cast produces a real Int value, not a
    // shimmed wrapper.
    let src = r#"
        fn main() {
            let f = 1.5;
            let r = Int(f) + 10;
            println(r);
        }
    "#;
    let (stdout, status) = build_and_run("arith", src);
    assert!(status.success());
    assert!(stdout.trim() == "11", "got: {:?}", stdout);
}
