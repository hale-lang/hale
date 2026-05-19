//! Phase-2 (2): locus-typed param fields with lifecycle cascade.
//!
//! Pre-2026-05-19, a locus literal used as a default-init for
//! another locus's param field would compile but its eager
//! dissolve would tear down its state before the parent stored
//! the pointer — segfault on first method call. The fix:
//! `lower_locus_instantiation` consumes a new
//! `instantiating_for_parent_field` flag set by the param-init
//! loop and routes the child through a parent-owned path that
//! skips the eager dissolve. The parent's dissolve dispatch
//! then cascades into each `LocusRef`-typed field, running its
//! drain → __dissolve_closures → dissolve → arena_destroy in
//! turn.
//!
//! These tests verify the lifecycle end-to-end:
//!   - field init evaluates the child literal (runs birth)
//!   - method dispatch through the field works at runtime
//!   - parent's dissolve cascades into the child's dissolve
//!     (an observable side effect — println — runs)
//!   - the cascade ordering is "outer's user body → inner's
//!     dissolve → outer's arena_destroy"

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_locus_field_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn bytesbuilder_as_locus_param_default_runs_method() {
    // Smoke test: a Holder with a BytesBuilder param that
    // defaults to a fresh builder. The held builder must be
    // usable via method calls after the Holder is constructed —
    // pre-fix, this segfaulted because the inner builder
    // dissolved at the end of the Holder { } literal expression.
    let src = r#"
        locus Holder {
            params {
                buf: std::bytes::BytesBuilder = std::bytes::BytesBuilder { initial_cap: 64 };
            }
        }
        fn main() {
            let h = Holder { };
            h.buf.append(std::bytes::from_string("hello"));
            println("len=", h.buf.len());
            let v = h.buf.view();
            println("v0=", std::bytes::at(v, 0) or -1);
            println("v4=", std::bytes::at(v, 4) or -1);
        }
    "#;
    let (stdout, status) = build_and_run("bb_field", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
    // 'h' = 104, 'o' = 111
    assert!(stdout.contains("v0=104"), "got: {:?}", stdout);
    assert!(stdout.contains("v4=111"), "got: {:?}", stdout);
}

#[test]
fn cascade_fires_inner_dissolve_on_outer_scope_exit() {
    // Observable: a user-defined Inner with a printing dissolve
    // method. Inner is a default-init param on Outer. We
    // instantiate Outer inside a helper fn so its scope-exit
    // dissolve fires (and cascades). The expected output proves
    // both that the cascade ran and that ordering puts the
    // inner's dissolve AFTER the outer's body has finished.
    let src = r#"
        locus Inner {
            params { tag: Int = 0; }
            dissolve() {
                println("Inner.dissolve tag=", self.tag);
            }
        }
        locus Outer {
            params {
                a: Inner = Inner { tag: 1 };
                b: Inner = Inner { tag: 2 };
            }
            dissolve() {
                println("Outer.dissolve");
            }
        }
        fn use_outer() {
            let o = Outer { };
            println("body");
        }
        fn main() {
            println("before");
            use_outer();
            println("after");
        }
    "#;
    let (stdout, status) = build_and_run("cascade_order", src);
    assert!(status.success(), "non-zero: {:?}", status);
    let lines: Vec<&str> = stdout.lines().collect();
    // Expected ordering:
    //   before
    //   body
    //   Outer.dissolve         ← outer's user body first
    //   Inner.dissolve tag=1   ← cascade in field-index order
    //   Inner.dissolve tag=2
    //   after
    let pos = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("missing {:?} in:\n{}", needle, stdout))
    };
    let before = pos("before");
    let body = pos("body");
    let outer_diss = pos("Outer.dissolve");
    let inner1 = pos("Inner.dissolve tag=1");
    let inner2 = pos("Inner.dissolve tag=2");
    let after = pos("after");
    assert!(before < body, "before<body: {}", stdout);
    assert!(body < outer_diss, "body<Outer: {}", stdout);
    assert!(outer_diss < inner1, "outer<inner1 (cascade after user body): {}", stdout);
    assert!(inner1 < inner2, "inner1<inner2 (field-order): {}", stdout);
    assert!(inner2 < after, "inner2<after: {}", stdout);
}

#[test]
fn many_iterations_dont_blow_memory() {
    // Without the cascade dissolve, each Holder leaks one
    // malloc-backed BytesBuilder header + buffer per iteration.
    // 100_000 iterations × 64-byte buffer = ~6 MB of leaked
    // header structs + ~6 MB of buffers = ~12 MB. Still under a
    // CI box's RSS budget so a passing test doesn't prove no
    // leak — but a regression that pushes per-iter to KB-sized
    // leaks would be visible. The primary signal is "doesn't
    // crash, completes promptly."
    let src = r#"
        locus Holder {
            params {
                buf: std::bytes::BytesBuilder = std::bytes::BytesBuilder { initial_cap: 64 };
            }
        }
        fn run_one() {
            let h = Holder { };
            h.buf.append(std::bytes::from_string("data"));
        }
        fn main() {
            let mut i = 0;
            while i < 100000 {
                run_one();
                i = i + 1;
            }
            println("done");
        }
    "#;
    let (stdout, status) = build_and_run("loop_100k", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("done"), "missing done: {}", stdout);
}
