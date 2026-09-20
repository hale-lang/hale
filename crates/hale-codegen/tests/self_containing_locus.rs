//! GH #813 — the instantiation-path backstop.
//!
//! A locus reachable from its own param defaults sent
//! `lower_locus_instantiation` through the default, into the locus it
//! builds, into ITS default, until the compiler's stack ran out
//! ("thread 'main' has overflowed its stack", exit 134). `hale check`
//! now refuses the program at the param, but `build_executable` never
//! runs the checker — so the lowering keeps its own floor, and these
//! tests are about that floor: an `Unsupported` error, returned.
//!
//! A regression here does not fail politely. Without the guard this
//! file's first test does not assert anything — it aborts the test
//! process on a stack overflow, which is how it was proven to bite.
//!
//! The three shapes that must keep building are the point of the two
//! halves of the key. Re-entry is refused only from inside a param
//! DEFAULT, because nesting written out in source is bounded by the
//! source that spells it (`same_type_nesting_written_out_still_builds`
//! re-enters `Box` with an identical argument list and is an ordinary
//! program). And the key carries the field names the literal supplies,
//! because the defaults a literal expands are exactly the ones it does
//! not supply (`a_fully_supplied_literal_in_its_own_default_runs`).

use std::process::Command;

use hale_codegen::{build_executable, CodegenError};

#[path = "support/harness.rs"]
mod harness;

fn build_err(name: &str, source: &str) -> CodegenError {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_selfcontain_{}", name));
    let out = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    match out {
        Ok(_) => panic!("expected the instantiation-path guard to refuse it"),
        Err(e) => e,
    }
}

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_selfcontain_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

/// The refusal, as the message it carries — `CodegenError` is not
/// `Clone`, so a test that wants to say more about it reads this.
fn refusal_message(e: &CodegenError, locus: &str) -> String {
    let CodegenError::Unsupported(msg) = e else {
        panic!("expected Unsupported, got {:?}", e);
    };
    assert!(
        msg.contains("cannot contain itself by value")
            && msg.contains(locus),
        "the error names the locus and the rule: {}",
        msg
    );
    msg.clone()
}

/// The issue's program. Before the guard: stack overflow, SIGABRT.
#[test]
fn the_backstop_refuses_the_direct_cycle() {
    let src = r#"
        locus Node {
            params {
                n: Int = 0;
                next: Node = Node { n: 1 };
            }
        }
        fn main() { let node = Node { }; println("n=", node.n); }
    "#;
    refusal_message(&build_err("direct", src), "Node");
}

/// The same, through two types — the cycle the ancestor path catches
/// that a self-reference check would not. The error names the ring it
/// closed, since neither locus alone is the mistake.
#[test]
fn the_backstop_refuses_a_two_type_cycle() {
    let src = r#"
        locus Alpha {
            params { tag: Int = 0; beta: Beta = Beta { tag: 1 }; }
        }
        locus Beta {
            params { tag: Int = 0; alpha: Alpha = Alpha { tag: 2 }; }
        }
        fn main() { let a = Alpha { }; println("tag=", a.tag); }
    "#;
    let msg = refusal_message(&build_err("two_type", src), "Beta");
    assert!(
        msg.contains("`Beta` → `Alpha` → `Beta`"),
        "the error spells the ring out: {}",
        msg
    );
}

/// The non-cyclic control: a locus holding a DIFFERENT locus by
/// value builds and runs exactly as before.
#[test]
fn a_locus_holding_a_different_locus_still_runs() {
    let src = r#"
        locus Leaf { params { tag: Int = 7; } }
        locus Holder { params { leaf: Leaf = Leaf { tag: 3 }; } }
        fn main() { let h = Holder { }; println("tag=", h.leaf.tag); }
    "#;
    let (stdout, status) = build_and_run("control", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("tag=3"), "got: {:?}", stdout);
}

/// Same-type nesting written out in source, reached through an
/// interface-typed field. Both `Box` literals are instantiated with
/// the same argument list, one inside the other — identical states on
/// the path — and the program is correct, which is why the guard
/// fires only from inside a param default.
#[test]
fn same_type_nesting_written_out_still_builds() {
    let src = r#"
        interface Shape { fn area() -> Int; }
        locus Dot { params { n: Int = 1; } fn area() -> Int { return self.n; } }
        locus Box {
            params { inner: Shape; n: Int = 0; }
            fn area() -> Int { return self.n + self.inner.area(); }
        }
        fn main() {
            let b = Box { inner: Box { inner: Dot { n: 1 }, n: 2 }, n: 4 };
            println("area=", b.area());
        }
    "#;
    let (stdout, status) = build_and_run("nesting", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("area=7"), "got: {:?}", stdout);
}

/// A literal inside a locus's own default that supplies every param
/// expands no default, so it terminates — and does, because the
/// path's key carries the supplied names.
#[test]
fn a_fully_supplied_literal_in_its_own_default_runs() {
    let src = r#"
        locus Probe {
            params {
                n: Int = 0;
                m: Int = Probe { n: 1, m: 2 }.n;
            }
        }
        fn main() { let p = Probe { }; println("m=", p.m); }
    "#;
    let (stdout, status) = build_and_run("supplied", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("m=1"), "got: {:?}", stdout);
}

/// The pinned decision, from codegen's side: a param default that is
/// a CALL returning the same locus is not an instantiation-path
/// re-entry, because lowering a call emits a call — `make`'s body is
/// lowered once, as a function. So the COMPILER terminates and the
/// build succeeds.
///
/// The built program does not: `make()` builds a `Node` whose `next`
/// default calls `make()` again, and it overflows its own stack at
/// run time, like any other unbounded recursion (`@no_recursion` is
/// the contract for that). Which is why this test builds it and stops
/// there, deliberately, rather than running it.
#[test]
fn a_factory_call_in_a_default_still_builds() {
    let src = r#"
        locus Node {
            params {
                n: Int = 0;
                next: Node = make();
            }
        }
        fn make() -> Node { return Node { n: 5 }; }
        fn main() { let node = Node { n: 1 }; println("n=", node.n); }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_selfcontain_factory");
    let out = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    assert!(out.is_ok(), "the compiler terminates on it: {:?}", out.err());
}
