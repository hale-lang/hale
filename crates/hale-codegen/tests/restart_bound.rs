//! `restart(c) for N` — the bounded restart, and what happens when
//! the bound is spent.
//!
//! The retry-bound modifier checked and modelled (the topology
//! artifact carries `retry_bound`) but codegen refused to lower it:
//! "unsupported in codegen v0: recovery modifier (for/until) not
//! lowered". So the policy a consumer was promised it could read —
//! "declared cap 3, observed 3" — was unshippable, because any
//! program stating the bound could not be built (downstream
//! handoff). The workaround was to move the bound into the handler
//! body (`if self.fired <= 3 { restart(c); } else { quarantine(c); }`),
//! which states the same policy where nothing can read it.
//!
//! Exhausting the bound QUARANTINES the child: the supervisor tried
//! N times and is done with it. That is a real difference from an
//! unbounded `restart(c)`, which stops re-running at the default cap
//! but leaves the child live.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_restartbound_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// A child whose birth-epoch closure can never pass, so every
/// attempt fails and the only thing that stops the loop is the
/// bound. `run()` prints, so its ABSENCE is the quarantine
/// observation.
fn src(recovery: &str) -> String {
    format!(
        r#"
locus Worker {{
    params {{ attempts: Int = 0; target: Int = 99; }}
    closure reached {{ self.attempts ~~ self.target within 0; epoch birth; }}
    birth() {{
        self.attempts = self.attempts + 1;
        println("birth ", self.attempts);
    }}
    run() {{ println("RUN"); }}
}}
locus Coordinator {{
    params {{ fired: Int = 0; }}
    on_failure(c: Worker, err: ClosureViolation) {{
        self.fired = self.fired + 1;
        println("fail ", self.fired);
        {recovery}
    }}
    run() {{ Worker {{ target: 99 }}; }}
}}
fn main() {{ let c = Coordinator {{ }}; }}
"#,
        recovery = recovery
    )
}

fn run(tag: &str, recovery: &str) -> String {
    let bin = build_hale(tag, &src(recovery));
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "program must run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

#[test]
fn a_declared_bound_allows_exactly_that_many_restarts() {
    // `for 3`: birth, then three restarts — four attempts, four
    // failures. Notably ABOVE the default cap of 2, which is the
    // case a hardcoded cap silently clamped.
    let out = run("for3", "restart(c) for 3;");
    assert_eq!(count(&out, "birth "), 4, "3 restarts + the first birth: {:?}", out);
    assert_eq!(count(&out, "fail "), 4, "{:?}", out);
}

#[test]
fn exhausting_the_bound_quarantines_the_child() {
    let out = run("quar", "restart(c) for 3;");
    assert!(
        !out.contains("RUN"),
        "the supervisor gave up on this child, so it must not run: {:?}",
        out
    );
}

#[test]
fn a_bound_of_one_restarts_once() {
    let out = run("for1", "restart(c) for 1;");
    assert_eq!(count(&out, "birth "), 2, "{:?}", out);
    assert!(!out.contains("RUN"), "{:?}", out);
}

/// `for 0` is not a degenerate case to reject — it says "do not
/// restart this child," which is a coherent policy and must
/// quarantine on the first failure rather than restart once.
#[test]
fn a_bound_of_zero_quarantines_without_restarting() {
    let out = run("for0", "restart(c) for 0;");
    assert_eq!(count(&out, "birth "), 1, "no restart at all: {:?}", out);
    assert_eq!(count(&out, "fail "), 1, "{:?}", out);
    assert!(!out.contains("RUN"), "{:?}", out);
}

/// The bound is a ceiling, not a schedule: a child that recovers
/// stops failing, keeps its remaining budget, and runs.
#[test]
fn a_bound_that_is_not_exhausted_lets_the_child_run() {
    let src = r#"
locus Worker {
    params { attempts: Int = 0; target: Int = 3; }
    closure reached { self.attempts ~~ self.target within 0; epoch birth; }
    birth() {
        self.attempts = self.attempts + 1;
        println("birth ", self.attempts);
    }
    run() { println("RUN"); }
}
locus Coordinator {
    on_failure(c: Worker, err: ClosureViolation) {
        println("fail");
        restart(c) for 5;
    }
    run() { Worker { target: 3 }; }
}
fn main() { let c = Coordinator { }; }
"#;
    let bin = build_hale("ok", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(count(&stdout, "birth "), 3, "{:?}", stdout);
    assert_eq!(count(&stdout, "fail"), 2, "{:?}", stdout);
    assert!(stdout.contains("RUN"), "the closure passed: {:?}", stdout);
}

/// An unbounded `restart(c)` keeps the behaviour it had before the
/// modifier lowered: it stops re-running at the default cap, and the
/// child is NOT quarantined. Pinned because the bound is carried in
/// the same field the default seeds, so a mistake there would
/// silently change every existing supervisor.
#[test]
fn an_unbounded_restart_is_unchanged_and_does_not_quarantine() {
    let out = run("default", "restart(c);");
    assert_eq!(
        count(&out, "birth "),
        3,
        "default cap is 2 restarts: {:?}",
        out
    );
    assert!(
        out.contains("RUN"),
        "an unbounded restart does not quarantine: {:?}",
        out
    );
}

/// `quarantine(c) for d` carries a DIFFERENT modifier — a duration
/// after which the child is automatically restarted
/// (`spec/semantics.md` § quarantine). It is specified and still not
/// lowered, so it must refuse in terms of what it actually is,
/// rather than implying `for` belongs to restart alone.
#[test]
fn quarantine_for_a_duration_refuses_as_the_unlowered_feature_it_is() {
    let program = hale_syntax::parse_source(&src("quarantine(c) for 3;"))
        .expect("parse");
    let bin = harness::unique_bin("hale_test_restartbound_quarfor");
    let err = build_executable(&program, &bin).expect_err("must refuse");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("quarantine") && msg.contains("not lowered"),
        "should name the unlowered quarantine-duration form: {}",
        msg
    );
    let _ = std::fs::remove_file(&bin);
}

/// `until` is the other recovery modifier and remains unlowered;
/// its diagnostic should point at what does work.
#[test]
fn until_refuses_and_points_at_the_bound_that_works() {
    let program =
        hale_syntax::parse_source(&src("restart(c) until 3;")).expect("parse");
    let bin = harness::unique_bin("hale_test_restartbound_until");
    let err = build_executable(&program, &bin).expect_err("must refuse");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("until") && msg.contains("for N"),
        "should name `for N` as the shipped alternative: {}",
        msg
    );
    let _ = std::fs::remove_file(&bin);
}
