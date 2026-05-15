//! Integration test: build hello-world via the codegen path
//! and run the produced binary. This is the milestone-0
//! end-to-end gate — if it passes, the LLVM toolchain is
//! wired correctly for the simplest Aperio program.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use aperio_codegen::build_executable;

fn examples_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is `crates/aperio-codegen`; the example
    // fixtures live under `tests/fixtures/examples/` inside that
    // crate (moved out of the repo-root `examples/` during the
    // public-release cleanup).
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

/// Compile `source` to a fresh temp binary, run it, return
/// (stdout, status). Caller asserts on the output.
fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
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
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

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
    let program = aperio_syntax::parse_source(src).expect("parse");
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
    let program = aperio_syntax::parse_source(src).expect("parse");
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
fn build_user_fn_with_int_return() {
    let src = r#"
        fn add(a: Int, b: Int) -> Int {
            return a + b;
        }
        fn main() {
            println("2+3=", add(2, 3));
        }
    "#;
    let (stdout, status) = build_and_run("user_fn_int", src);
    assert!(status.success());
    assert!(stdout.contains("2+3=5"), "got: {:?}", stdout);
}

#[test]
fn build_void_user_fn() {
    let src = r#"
        fn greet(name: String) {
            println("hello ", name);
        }
        fn main() {
            greet("world");
        }
    "#;
    let (stdout, status) = build_and_run("user_fn_void", src);
    assert!(status.success());
    assert!(stdout.contains("hello world"), "got: {:?}", stdout);
}

#[test]
fn build_recursive_fib() {
    // Recursion: each call site can resolve `fib` because the
    // declare/lower split puts every fn in the user_fns table
    // before any body is lowered.
    let src = r#"
        fn fib(n: Int) -> Int {
            if n < 2 {
                return n;
            }
            return fib(n - 1) + fib(n - 2);
        }
        fn main() {
            println("fib(10)=", fib(10));
        }
    "#;
    let (stdout, status) = build_and_run("user_fn_fib", src);
    assert!(status.success());
    assert!(stdout.contains("fib(10)=55"), "got: {:?}", stdout);
}

#[test]
fn build_fn_calls_fn_calls_fn() {
    // a → b → c, with each carrying values and accumulating.
    let src = r#"
        fn c(x: Int) -> Int { return x * 2; }
        fn b(x: Int) -> Int { return c(x) + 1; }
        fn a(x: Int) -> Int { return b(x) - 1; }
        fn main() {
            println("a(3)=", a(3));
        }
    "#;
    // a(3) → b(3) → c(3)=6 → +1 = 7 → -1 = 6
    let (stdout, status) = build_and_run("user_fn_chain", src);
    assert!(status.success());
    assert!(stdout.contains("a(3)=6"), "got: {:?}", stdout);
}

#[test]
fn build_fn_with_float_and_bool() {
    let src = r#"
        fn scale(x: Float, by: Float) -> Float {
            return x * by;
        }
        fn is_positive(x: Int) -> Bool {
            return x > 0;
        }
        fn main() {
            println("2.5*4.0=", scale(2.5, 4.0));
            println("pos(7)=", is_positive(7));
            println("pos(-3)=", is_positive(-3));
        }
    "#;
    let (stdout, status) = build_and_run("user_fn_typed", src);
    assert!(status.success());
    assert!(stdout.contains("2.5*4.0=10"), "got: {:?}", stdout);
    assert!(stdout.contains("pos(7)=true"), "got: {:?}", stdout);
    assert!(stdout.contains("pos(-3)=false"), "got: {:?}", stdout);
}

#[test]
fn control_flow_example_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("07-control-flow");
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

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
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

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
fn build_locus_runtime_self_reads_in_run() {
    // self.n + self.greeting both read at runtime via GEP+load
    // inside run()'s while loop — no compile-time constant
    // shortcut.
    let src = r#"
        locus T {
            params {
                n: Int = 4;
                greeting: String = "hi";
            }
            run() {
                let mut i = 0;
                while i < self.n {
                    println(self.greeting, " ", i);
                    i = i + 1;
                }
            }
        }
        fn main() { T { n: 3, greeting: "yo" }; }
    "#;
    let (stdout, status) = build_and_run("locus_runtime_reads", src);
    assert!(status.success());
    assert!(stdout.contains("yo 0"), "got: {:?}", stdout);
    assert!(stdout.contains("yo 1"), "got: {:?}", stdout);
    assert!(stdout.contains("yo 2"), "got: {:?}", stdout);
    assert!(!stdout.contains("yo 3"), "got: {:?}", stdout);
}

#[test]
fn build_locus_self_field_mutation() {
    // birth() sets self.count, run() increments it in a loop and
    // reads it back — exercises self.X = via GEP+store and the
    // birth → run state handoff.
    let src = r#"
        locus Counter {
            params {
                count: Int = 0;
                limit: Int = 5;
            }
            birth() {
                self.count = 10;
            }
            run() {
                while self.count < self.limit + 10 {
                    self.count = self.count + 1;
                }
                println("final=", self.count);
            }
        }
        fn main() { Counter { limit: 3 }; }
    "#;
    // birth: count=10. run loops while count<13 → count=13 then exit.
    let (stdout, status) = build_and_run("locus_self_mut", src);
    assert!(status.success());
    assert!(stdout.contains("final=13"), "got: {:?}", stdout);
}

#[test]
fn build_locus_compound_self_assignment() {
    let src = r#"
        locus C {
            params { n: Int = 0; }
            birth() {
                self.n += 100;
                self.n *= 2;
                self.n -= 7;
            }
            run() {
                println("n=", self.n);
            }
        }
        fn main() { C { }; }
    "#;
    // 0 + 100 = 100; * 2 = 200; - 7 = 193
    let (stdout, status) = build_and_run("locus_compound_self", src);
    assert!(status.success());
    assert!(stdout.contains("n=193"), "got: {:?}", stdout);
}

#[test]
fn build_short_ticker_with_sleep_and_self() {
    // Like 01-locus-with-run but with a 10ms interval to keep the
    // test fast. Exercises the canonical pattern: self.n bound,
    // self.interval threaded into time::sleep, mut local counter
    // — all in run().
    let src = r#"
        locus Ticker {
            params {
                n: Int = 3;
                interval: Duration = 10ms;
            }
            run() {
                let mut i = 0;
                while i < self.n {
                    println("tick ", i);
                    time::sleep(self.interval);
                    i = i + 1;
                }
            }
        }
        fn main() { Ticker { }; }
    "#;
    let (stdout, status) = build_and_run("short_ticker", src);
    assert!(status.success());
    assert!(stdout.contains("tick 0"), "got: {:?}", stdout);
    assert!(stdout.contains("tick 2"), "got: {:?}", stdout);
    assert!(!stdout.contains("tick 3"), "got: {:?}", stdout);
}

#[test]
fn locus_with_run_canonical_example_builds_and_runs() {
    // The canonical 01-locus-with-run, rebuilt against the
    // codegen path. 3 × 500ms sleeps land us at ~1.5s; tolerable
    // for a single integration test.
    let mut src_path = examples_dir();
    src_path.push("01-locus-with-run");
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_01_ticker");

    build_executable(&program, &bin_path).expect("build");
    let start = Instant::now();
    let output = Command::new(&bin_path).output().expect("run");
    let elapsed = start.elapsed();
    let _ = std::fs::remove_file(&bin_path);

    assert!(output.status.success(), "non-zero: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tick 0"), "got: {:?}", stdout);
    assert!(stdout.contains("tick 2"), "got: {:?}", stdout);
    assert!(
        elapsed.as_millis() >= 1500,
        "ticker returned too early: {:?}",
        elapsed
    );
}

#[test]
fn build_accept_runs_before_child_birth() {
    // F.7: parent.accept(self, child) fires before child.birth.
    // We verify by checking the println order.
    let src = r#"
        locus Child {
            params { id: Int = 0; }
            birth() {
                println("child.birth id=", self.id);
            }
        }
        locus Parent {
            params { _u: Int = 0; }
            accept(c: Child) {
                println("accept id=", c.id);
            }
            run() {
                Child { id: 1 };
                Child { id: 2 };
            }
        }
        fn main() { Parent { }; }
    "#;
    let (stdout, status) = build_and_run("accept_order", src);
    assert!(status.success());
    // Expected order per F.7: accept(1) → child.birth(1) → accept(2) → child.birth(2)
    let expected = "accept id=1\nchild.birth id=1\naccept id=2\nchild.birth id=2";
    assert!(
        stdout.contains(expected),
        "expected accept-before-birth ordering; got: {:?}",
        stdout
    );
}

#[test]
fn build_child_field_read_in_accept() {
    // The child's contract field is readable from the parent's
    // accept() body via `g.field`. Each child carries its own
    // overridden value.
    let src = r#"
        locus Greeter {
            params { greeting: String = "default"; }
            contract { expose greeting: String; }
        }
        locus Coord {
            params { _u: Int = 0; }
            contract { consume greeting: String; }
            accept(g: Greeter) {
                println("got: ", g.greeting);
            }
            run() {
                Greeter { greeting: "alpha" };
                Greeter { greeting: "beta" };
                Greeter { greeting: "gamma" };
            }
        }
        fn main() { Coord { }; }
    "#;
    let (stdout, status) = build_and_run("child_field", src);
    assert!(status.success());
    assert!(stdout.contains("got: alpha"), "got: {:?}", stdout);
    assert!(stdout.contains("got: beta"), "got: {:?}", stdout);
    assert!(stdout.contains("got: gamma"), "got: {:?}", stdout);
}

#[test]
fn build_accept_with_self_reads() {
    // Parent's accept body can mix `self.X` (parent state) with
    // `g.X` (child state) freely — they GEP through different
    // structs.
    let src = r#"
        locus Worker {
            params { value: Int = 0; }
            contract { expose value: Int; }
        }
        locus Boss {
            params { factor: Int = 10; }
            contract { consume value: Int; }
            accept(w: Worker) {
                println("scaled=", w.value * self.factor);
            }
            run() {
                Worker { value: 3 };
                Worker { value: 7 };
            }
        }
        fn main() { Boss { factor: 100 }; }
    "#;
    let (stdout, status) = build_and_run("accept_self_mix", src);
    assert!(status.success());
    assert!(stdout.contains("scaled=300"), "got: {:?}", stdout);
    assert!(stdout.contains("scaled=700"), "got: {:?}", stdout);
}

#[test]
fn parent_child_canonical_example_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("02-parent-child");
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_02_parent_child");

    build_executable(&program, &bin_path).expect("build");
    let output = Command::new(&bin_path).output().expect("run");
    let _ = std::fs::remove_file(&bin_path);

    assert!(output.status.success(), "non-zero: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("greeting from child: hello"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("greeting from child: hi"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("greeting from child: yo"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn build_time_monotonic_returns_duration() {
    // Two reads of the monotonic clock with a 20ms sleep between
    // them; the elapsed Duration must be at least the sleep
    // interval. Exercises:
    //   - time::monotonic() in expression position (let binding)
    //   - Duration - Duration → Duration
    //   - Duration > Duration comparison
    //   - if/else branching on the comparison
    let src = r#"
        fn main() {
            let t0 = time::monotonic();
            time::sleep(20ms);
            let t1 = time::monotonic();
            let elapsed = t1 - t0;
            if elapsed > 20ms {
                println("ok");
            } else {
                println("fail");
            }
        }
    "#;
    let (stdout, status) = build_and_run("time_monotonic", src);
    assert!(status.success());
    assert!(stdout.contains("ok"), "got: {:?}", stdout);
    assert!(!stdout.contains("fail"), "got: {:?}", stdout);
}

#[test]
fn stateful_locus_example_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("10-stateful-locus");
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_10_stateful_locus");

    build_executable(&program, &bin_path).expect("build");
    let output = Command::new(&bin_path).output().expect("run");
    let _ = std::fs::remove_file(&bin_path);

    assert!(output.status.success(), "non-zero: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("total=160"), "got: {:?}", stdout);
    assert!(stdout.contains("step=30"), "got: {:?}", stdout);
}

#[test]
fn functions_example_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("09-functions");
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_09_functions");

    build_executable(&program, &bin_path).expect("build");
    let output = Command::new(&bin_path).output().expect("run");
    let _ = std::fs::remove_file(&bin_path);

    assert!(output.status.success(), "non-zero: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("square(7)=49"), "got: {:?}", stdout);
    assert!(stdout.contains("fib(12)=144"), "got: {:?}", stdout);
    assert!(
        stdout.contains("square(square(3))=81"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn hello_world_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("hello-world");
    src_path.push("main.ap");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = aperio_syntax::parse_source(&source).expect("parse");

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
