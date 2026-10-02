//! GH #813 — a self-containing locus never reaches lowering.
//!
//! A locus reachable from its own param defaults sent
//! `lower_locus_instantiation` through the default, into the locus it
//! builds, into ITS default, until the compiler's stack ran out
//! ("thread 'main' has overflowed its stack", exit 134). `hale check`
//! refuses the program at the param. `build_executable` never runs the
//! checker, so lowering used to keep a re-entry guard of its own; since
//! F.40 phase 3, C7 the law is one the harness's lowering view demands
//! too (`hale_types::lowering_laws`), the guard is gone, and these
//! tests pin that the harness refuses every shape the guard refused,
//! with the law's wording, before lowering starts.
//!
//! A regression here does not fail politely. Without the law at the
//! harness this file's first test does not assert anything — it aborts
//! the test process on a stack overflow, which is how it was proven to
//! bite.
//!
//! The shapes that must keep building are the point of the two halves
//! of the key. Only a param DEFAULT's literal is an edge, because
//! nesting written out in source is bounded by the source that spells
//! it (`same_type_nesting_written_out_still_builds` nests `Box` in
//! `Box` with an identical argument list and is an ordinary program).
//! And the key carries the field names the literal supplies, because
//! the defaults a literal expands are exactly the ones it does not
//! supply (`a_fully_supplied_literal_in_its_own_default_runs`).

use std::process::Command;

use hale_codegen::{build_executable_with_options, CodegenError};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_err(name: &str, source: &str) -> CodegenError {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_selfcontain_{}", name));
    let out = build_executable_with_options(&program, &bin, &[], &build_opts::options());
    let _ = std::fs::remove_file(&bin);
    match out {
        Ok(_) => panic!("expected the self-containment law to refuse it"),
        Err(e) => e,
    }
}

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_selfcontain_{}", name));
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
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

/// The issue's program. Before any guard: stack overflow, SIGABRT.
#[test]
fn the_law_refuses_the_direct_cycle() {
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

/// The same, through two types — the cycle a self-reference check
/// would not catch. The error names the ring it closed, from the locus
/// whose param it is reported at, since neither locus alone is the
/// mistake.
#[test]
fn the_law_refuses_a_two_type_cycle() {
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
        msg.contains("param `beta` of `Alpha`") && msg.contains("`Alpha` → `Beta` → `Alpha`"),
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

/// A param default that is a CALL returning the same locus: lowering a
/// call emits a call — `make`'s body is lowered once, as a function —
/// so the compiler would terminate on it, and the built program would
/// recurse until it overflowed its own stack at run time (`make()`
/// builds a `Node` whose `next` default calls `make()` again).
///
/// GH #870 refuses it at the param, having asked whether `make` builds
/// a fresh `Node` (see `hale-types/tests/self_containing_locus.rs`).
/// Until F.40 phase 3, C7 a harness build skipped that law and built
/// the program; the harness demands the law now, so it is refused here
/// too, with the factory spelled out.
#[test]
fn a_factory_call_in_a_default_is_refused_at_the_harness_too() {
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
    let msg = refusal_message(&build_err("factory", src), "Node");
    assert!(msg.contains("defaults to `make()`, which builds a fresh `Node`"), "the factory edge: {msg}");
}

/// F.40 phase 3, C7: a literal in a conditional default is one lowering
/// expands (it lowers every branch), and the law's walk now reaches it;
/// it used to stop at an `if`, and the guard refused the program
/// without a span. Spelled without a raw string so the corpus does not
/// harvest it.
#[test]
fn a_cycle_through_a_conditional_default_is_refused() {
    let src = "locus Node {\n\
               \x20   params {\n\
               \x20       n: Int = 0;\n\
               \x20       next: Node = if true { Node { n: 1 } } else { Node { n: 2 } };\n\
               \x20   }\n\
               }\n\
               fn main() { let node = Node { }; println(\"n=\", node.n); }\n";
    refusal_message(&build_err("conditional", src), "Node");
}
