//! Integration test: build hello-world via the codegen path
//! and run the produced binary. This is the milestone-0
//! end-to-end gate — if it passes, the LLVM toolchain is
//! wired correctly for the simplest lotus program.

use std::path::PathBuf;
use std::process::Command;

use lotus_codegen::build_executable;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("examples");
    p
}

/// Compile `source` to a fresh temp binary, run it, return
/// (stdout, status). Caller asserts on the output.
fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = lotus_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn build_int_param_and_println() {
    // Mixed-type println in a single composed printf call.
    let src = r#"
        locus N {
            params {
                name: String = "x";
                n: Int = 42;
                f: Float = 1.5;
                flag: Bool = true;
            }
            birth() {
                println("name=", self.name);
                println("n=", self.n, " f=", self.f, " flag=", self.flag);
            }
        }
        fn main() { N { }; }
    "#;
    let (stdout, status) = build_and_run("mixed_println", src);
    assert!(status.success(), "binary exited non-zero: {:?}", status);
    assert!(stdout.contains("name=x"), "got: {:?}", stdout);
    assert!(stdout.contains("n=42 f=1.5 flag=true"), "got: {:?}", stdout);
}

#[test]
fn build_let_and_int_arithmetic() {
    let src = r#"
        fn main() {
            let a = 2 + 3 * 4;       // 14
            let b = a - 5;           // 9
            let c = b * 10 / 3;      // 30
            let d = b % 4;           // 1
            println("a=", a, " b=", b, " c=", c, " d=", d);
        }
    "#;
    let (stdout, status) = build_and_run("let_int_arith", src);
    assert!(status.success());
    assert!(
        stdout.contains("a=14 b=9 c=30 d=1"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn build_let_and_float_arithmetic() {
    let src = r#"
        fn main() {
            let pi = 3.14;
            let r = pi * 2.0;
            println("r=", r);
        }
    "#;
    let (stdout, status) = build_and_run("let_float_arith", src);
    assert!(status.success());
    assert!(stdout.contains("r=6.28"), "got: {:?}", stdout);
}

#[test]
fn build_comparisons_yield_bool() {
    let src = r#"
        fn main() {
            let lt = 5 < 10;
            let eq = 7 == 7;
            let gt = 1 > 100;
            println("lt=", lt, " eq=", eq, " gt=", gt);
        }
    "#;
    let (stdout, status) = build_and_run("comparisons", src);
    assert!(status.success());
    assert!(
        stdout.contains("lt=true eq=true gt=false"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn build_let_mut_and_assign() {
    let src = r#"
        fn main() {
            let mut n = 0;
            n = n + 1;       // 1
            n = n + 10;      // 11
            let mut f = 1.0;
            f = f * 2.5;     // 2.5
            println("n=", n, " f=", f);
        }
    "#;
    let (stdout, status) = build_and_run("let_mut_assign", src);
    assert!(status.success());
    assert!(stdout.contains("n=11 f=2.5"), "got: {:?}", stdout);
}

#[test]
fn build_compound_assign() {
    let src = r#"
        fn main() {
            let mut n = 0;
            n += 2;          // 2
            n *= 5;          // 10
            n -= 3;          // 7
            n /= 2;          // 3
            n %= 2;          // 1
            println("n=", n);
        }
    "#;
    let (stdout, status) = build_and_run("compound_assign", src);
    assert!(status.success());
    assert!(stdout.contains("n=1"), "got: {:?}", stdout);
}

#[test]
fn build_int_override_at_instantiation() {
    // Instantiation overrides the param default.
    let src = r#"
        locus M {
            params { n: Int = 0; }
            birth() {
                println("n=", self.n);
            }
        }
        fn main() { M { n: 99 }; }
    "#;
    let (stdout, status) = build_and_run("int_override", src);
    assert!(status.success());
    assert!(stdout.contains("n=99"), "got: {:?}", stdout);
}

#[test]
fn mutable_counter_example_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("06-mutable-counter");
    src_path.push("main.lt");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = lotus_syntax::parse_source(&source).expect("parse");

    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_06_mutable_counter");

    build_executable(&program, &bin_path).expect("build");

    let output = Command::new(&bin_path).output().expect("run");
    let _ = std::fs::remove_file(&bin_path);

    assert!(
        output.status.success(),
        "binary exited non-zero: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("n=2"),
        "expected n=2 in stdout; got: {:?}",
        stdout
    );
}

#[test]
fn hello_world_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("hello-world");
    src_path.push("main.lt");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = lotus_syntax::parse_source(&source).expect("parse");

    // Use a temp file so the test is hermetic and doesn't
    // collide with any existing binary in examples/.
    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_hello_world");

    build_executable(&program, &bin_path).expect("build");

    let output = Command::new(&bin_path)
        .output()
        .expect("run produced binary");
    let _ = std::fs::remove_file(&bin_path);

    assert!(
        output.status.success(),
        "binary exited non-zero: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello, world"),
        "expected greeting in stdout; got: {:?}",
        stdout
    );
}
