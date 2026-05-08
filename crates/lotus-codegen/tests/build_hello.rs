//! Integration test: build hello-world via the codegen path
//! and run the produced binary. This is the milestone-0
//! end-to-end gate — if it passes, the LLVM toolchain is
//! wired correctly for the simplest lotus program.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

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
fn build_if_else_picks_branch() {
    let src = r#"
        fn main() {
            let n = 7;
            if n > 5 {
                println("hi");
            } else {
                println("lo");
            }
            if n == 7 {
                println("seven");
            }
        }
    "#;
    let (stdout, status) = build_and_run("if_else", src);
    assert!(status.success());
    assert!(stdout.contains("hi"), "got: {:?}", stdout);
    assert!(stdout.contains("seven"), "got: {:?}", stdout);
    assert!(!stdout.contains("lo"), "got: {:?}", stdout);
}

#[test]
fn build_else_if_chain() {
    let src = r#"
        fn main() {
            let n = 2;
            if n == 1 {
                println("one");
            } else if n == 2 {
                println("two");
            } else if n == 3 {
                println("three");
            } else {
                println("other");
            }
        }
    "#;
    let (stdout, status) = build_and_run("else_if", src);
    assert!(status.success());
    assert!(stdout.contains("two"), "got: {:?}", stdout);
    assert!(!stdout.contains("one"), "got: {:?}", stdout);
    assert!(!stdout.contains("three"), "got: {:?}", stdout);
    assert!(!stdout.contains("other"), "got: {:?}", stdout);
}

#[test]
fn build_while_with_break_and_continue() {
    // n iterates 1..=10; skip 7 (continue), break at 9. Sum of
    // 1+2+3+4+5+6+8 == 29.
    let src = r#"
        fn main() {
            let mut n = 0;
            let mut sum = 0;
            while n < 10 {
                n = n + 1;
                if n == 7 {
                    continue;
                }
                if n == 9 {
                    break;
                }
                sum = sum + n;
            }
            println("sum=", sum, " n=", n);
        }
    "#;
    let (stdout, status) = build_and_run("while_break_continue", src);
    assert!(status.success());
    assert!(stdout.contains("sum=29 n=9"), "got: {:?}", stdout);
}

#[test]
fn build_nested_while_breaks_inner_only() {
    let src = r#"
        fn main() {
            let mut i = 0;
            let mut hits = 0;
            while i < 3 {
                let mut j = 0;
                while j < 10 {
                    if j == 2 {
                        break;
                    }
                    j = j + 1;
                    hits = hits + 1;
                }
                i = i + 1;
            }
            println("hits=", hits);
        }
    "#;
    // outer 3 iters * inner 2 hits each = 6.
    let (stdout, status) = build_and_run("nested_while", src);
    assert!(status.success());
    assert!(stdout.contains("hits=6"), "got: {:?}", stdout);
}

#[test]
fn build_if_both_branches_break() {
    // Both arms terminate; the `if` itself is Terminated. The
    // surrounding while still produces a valid binary because
    // the unreachable merge BB gets a `unreachable` terminator.
    let src = r#"
        fn main() {
            let mut n = 0;
            while n < 100 {
                if n == 5 {
                    break;
                } else {
                    n = n + 1;
                    continue;
                }
            }
            println("n=", n);
        }
    "#;
    let (stdout, status) = build_and_run("if_both_terminate", src);
    assert!(status.success());
    assert!(stdout.contains("n=5"), "got: {:?}", stdout);
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
fn build_time_sleep_blocks_for_at_least_requested_duration() {
    // 50ms is short enough to keep the test fast and long enough
    // that scheduler jitter on a busy CI host can't accidentally
    // take it under the floor. We measure with the host's
    // monotonic clock (std::time::Instant) which Rust documents
    // as monotonic on Linux.
    let src = r#"
        fn main() {
            println("before");
            time::sleep(50ms);
            println("after");
        }
    "#;
    let program = lotus_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("lotus_test_time_sleep");
    build_executable(&program, &bin).expect("build");

    let start = Instant::now();
    let output = Command::new(&bin).output().expect("run");
    let elapsed = start.elapsed();
    let _ = std::fs::remove_file(&bin);

    assert!(output.status.success(), "non-zero: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("before"), "got: {:?}", stdout);
    assert!(stdout.contains("after"), "got: {:?}", stdout);
    // Must sleep at least the requested duration. Process spawn
    // overhead is included; we only assert the floor.
    assert!(
        elapsed.as_millis() >= 50,
        "sleep returned too early: {:?}",
        elapsed
    );
    // Sanity ceiling so a hung test fails loud rather than hanging
    // CI. 5s is generous; if we hit it something is broken.
    assert!(
        elapsed.as_secs() < 5,
        "sleep took implausibly long: {:?}",
        elapsed
    );
}

#[test]
fn build_time_sleep_in_loop_accumulates() {
    // Three 30ms sleeps under a while loop — ~90ms total. Verifies
    // that the sleep call composes with control flow and that
    // basic-block management around the FFI loop is correct.
    let src = r#"
        fn main() {
            let mut i = 0;
            while i < 3 {
                time::sleep(30ms);
                i = i + 1;
            }
            println("i=", i);
        }
    "#;
    let program = lotus_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("lotus_test_time_sleep_loop");
    build_executable(&program, &bin).expect("build");

    let start = Instant::now();
    let output = Command::new(&bin).output().expect("run");
    let elapsed = start.elapsed();
    let _ = std::fs::remove_file(&bin);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("i=3"), "got: {:?}", stdout);
    assert!(
        elapsed.as_millis() >= 90,
        "loop sleep returned too early: {:?}",
        elapsed
    );
}

#[test]
fn build_duration_param_prints_as_ns() {
    // Duration self.X round-trips as a compile-time-known param
    // and prints with the same `<ns>ns` formatting the interpreter
    // uses, so the two paths agree.
    let src = r#"
        locus T {
            params {
                interval: Duration = 250ms;
            }
            birth() {
                println("interval=", self.interval);
            }
        }
        fn main() { T { }; }
    "#;
    let (stdout, status) = build_and_run("duration_param", src);
    assert!(status.success());
    assert!(
        stdout.contains("interval=250000000ns"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn control_flow_example_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("07-control-flow");
    src_path.push("main.lt");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = lotus_syntax::parse_source(&source).expect("parse");

    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_07_control_flow");

    build_executable(&program, &bin_path).expect("build");
    let output = Command::new(&bin_path).output().expect("run");
    let _ = std::fs::remove_file(&bin_path);

    assert!(output.status.success(), "non-zero: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("sum=29 stopped at n=9"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn monotonic_sleep_example_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("08-monotonic-sleep");
    src_path.push("main.lt");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = lotus_syntax::parse_source(&source).expect("parse");

    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_08_monotonic_sleep");

    build_executable(&program, &bin_path).expect("build");
    let start = Instant::now();
    let output = Command::new(&bin_path).output().expect("run");
    let elapsed = start.elapsed();
    let _ = std::fs::remove_file(&bin_path);

    assert!(output.status.success(), "non-zero: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tick 0"), "got: {:?}", stdout);
    assert!(stdout.contains("tick 2"), "got: {:?}", stdout);
    assert!(stdout.contains("done"), "got: {:?}", stdout);
    // 3 × 50ms sleeps = 150ms floor.
    assert!(
        elapsed.as_millis() >= 150,
        "example returned too early: {:?}",
        elapsed
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
