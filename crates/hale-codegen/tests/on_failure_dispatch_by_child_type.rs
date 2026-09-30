//! A locus with one `on_failure` per child type routes each child's
//! failure to the handler for THAT child's locus type (downstream
//! handoff).
//!
//! Both handlers take `ClosureViolation`, so the child's type is the
//! only thing that can tell them apart. Codegen used to keep a single
//! handler slot per locus: every `on_failure` overwrote it, so the
//! slot held the LAST-declared handler's child type, while the body
//! lowered into that fn was the FIRST-declared handler's. The last
//! type's failure ran the first handler's body, and the first type's
//! failure found "no parent handler" and exited.
//!
//! Each program below fails both children in turn and prints which
//! handler ran and which child it was handed.

use std::process::Command;

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_and_run(name: &str, source: &str) -> (String, String, bool) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("lotus_test_{}", name));
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

/// Two children violating from a method call on the parent's
/// `run()`, the first-declared handler's child failing first.
#[test]
fn each_child_type_reaches_its_own_handler() {
    let src = r#"
locus Alpha {
    params { why: String = "alpha-why"; n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    fn go() { self.n = 1; violate boom; }
}
locus Beta {
    params { why: String = "beta-why"; n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    fn go() { self.n = 2; violate boom; }
}
main locus App {
    params { a: Alpha = Alpha { }; b: Beta = Beta { }; }
    on_failure(x: Alpha, err: ClosureViolation) { println("ALPHA handler: ", x.why, " ", err.closure); }
    on_failure(y: Beta, err: ClosureViolation) { println("BETA handler: ", y.why, " ", err.closure); }
    run() {
        self.a.go();
        self.b.go();
        println("done");
    }
}
fn main() { App { }; }
"#;
    let (stdout, stderr, ok) = build_and_run("on_failure_by_child_type", src);
    assert!(ok, "non-zero exit; stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(
        stdout,
        "ALPHA handler: alpha-why boom\nBETA handler: beta-why boom\ndone\n",
        "each child's failure must run the handler for its own type; \
         stderr={stderr:?}"
    );
}

/// Outside review of #1276, finding 3: two handlers that share one span
/// (built by hand here; a synthetic AST with shared provenance, or a
/// desugar that stamps one span on several declarations, produces the
/// same) are still two handlers. Lowering joined a declaration to its
/// routing row by span, so both bodies went into the first row's fn,
/// the second fn had no body and the build failed; the join is by
/// identity now. A plain literal: the corpus harvests raw ones.
#[test]
fn handlers_sharing_a_span_each_lower_into_their_own_fn() {
    use hale_syntax::ast::{LocusMember, TopDecl};
    let src = "
locus Alpha {
    params { why: String = \"alpha-why\"; n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    fn go() { self.n = 1; violate boom; }
}
locus Beta {
    params { why: String = \"beta-why\"; n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    fn go() { self.n = 2; violate boom; }
}
main locus App {
    params { a: Alpha = Alpha { }; b: Beta = Beta { }; }
    on_failure(x: Alpha, err: ClosureViolation) { println(\"ALPHA handler: \", x.why); }
    on_failure(y: Beta, err: ClosureViolation) { println(\"BETA handler: \", y.why); }
    run() {
        self.b.go();
        self.a.go();
        println(\"done\");
    }
}
fn main() { App { }; }
";
    let mut program = hale_syntax::parse_source(src).expect("parse");
    for item in &mut program.items {
        let TopDecl::Locus(l) = item else { continue };
        if l.name.name != "App" {
            continue;
        }
        let mut handlers = l.members.iter_mut().filter_map(|m| match m {
            LocusMember::Failure(fd) => Some(fd),
            _ => None,
        });
        let first = handlers.next().expect("first handler");
        let second = handlers.next().expect("second handler");
        second.span = first.span;
    }
    let bin = harness::unique_bin("lotus_test_on_failure_shared_span");
    build_executable_with_options(&program, &bin, &[], &build_opts::options())
        .expect("two handlers sharing a span build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "non-zero exit; stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(
        stdout,
        "BETA handler: beta-why\nALPHA handler: alpha-why\ndone\n",
        "stderr={stderr:?}"
    );
}

/// The same pair, failing in the opposite order to the handlers'
/// declaration, so neither "first handler" nor "last handler" can
/// pass by accident.
#[test]
fn dispatch_does_not_follow_declaration_order() {
    let src = r#"
locus Alpha {
    params { why: String = "alpha-why"; n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    fn go() { self.n = 1; violate boom; }
}
locus Beta {
    params { why: String = "beta-why"; n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    fn go() { self.n = 2; violate boom; }
}
main locus App {
    params { a: Alpha = Alpha { }; b: Beta = Beta { }; }
    on_failure(x: Alpha, err: ClosureViolation) { println("ALPHA handler: ", x.why); }
    on_failure(y: Beta, err: ClosureViolation) { println("BETA handler: ", y.why); }
    run() {
        self.b.go();
        self.a.go();
        self.b.go();
        println("done");
    }
}
fn main() { App { }; }
"#;
    let (stdout, stderr, ok) = build_and_run("on_failure_by_child_type_rev", src);
    assert!(ok, "non-zero exit; stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(
        stdout,
        "BETA handler: beta-why\nALPHA handler: alpha-why\nBETA handler: beta-why\ndone\n",
        "stderr={stderr:?}"
    );
}

/// Three child types, each failing from its own `run()` while the
/// parent is still setting params — the held-failure route, resolved
/// against the parent being instantiated rather than a method body.
/// Each handler counts into its own field.
#[test]
fn held_failures_reach_their_own_handlers() {
    let src = r#"
locus A {
    params { n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    run() { self.n = 1; violate boom; }
}
locus B {
    params { n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    run() { self.n = 2; violate boom; }
}
locus C {
    params { n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    run() { self.n = 3; violate boom; }
}
locus Sup {
    params {
        got_a: Int = 0; got_b: Int = 0; got_c: Int = 0;
        c: C = C { }; a: A = A { }; b: B = B { };
    }
    on_failure(x: B, err: ClosureViolation) { self.got_b = self.got_b + x.n; }
    on_failure(x: C, err: ClosureViolation) { self.got_c = self.got_c + x.n; }
    on_failure(x: A, err: ClosureViolation) { self.got_a = self.got_a + x.n; }
}
fn main() {
    let s = Sup { };
    println("a=", s.got_a, " b=", s.got_b, " c=", s.got_c);
}
"#;
    let (stdout, stderr, ok) = build_and_run("on_failure_by_child_type_held", src);
    assert!(ok, "non-zero exit; stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "a=1 b=2 c=3\n", "stderr={stderr:?}");
}
