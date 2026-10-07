//! v1.x-VIOLATE (F.27) — codegen / compiled-binary tests for
//! `violate NAME;`. Exercises the lowering end-to-end: compile
//! to a native binary, run it, verify stdout.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let bin = harness::unique_bin(&format!("lotus_test_{}", name));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn violate_in_birth_routes_to_parent_on_failure() {
    // F.27 extension (2026-05-19): violate inside birth() routes
    // through the parent's on_failure handler at construction,
    // not lazily on first method call. Pre-extension, codegen
    // rejected `violate` outside a user fn (lifecycle bodies set
    // current_user_fn_ret = None). The fix marks lifecycle bodies
    // as void-returning user-fn contexts so the same machinery
    // fires.
    let src = r#"
locus Child {
    closure birth_fatal { epoch inline; }
    birth() {
        violate birth_fatal;
    }
}

locus Parent {
    accept(c: Child) { }
    on_failure(c: Child, err: ClosureViolation) {
        println("absorbed=", err.closure);
    }
    run() {
        Child { };
        println("parent.run continued");
    }
}

fn main() { Parent { }; }
"#;
    let (stdout, status) = build_and_run("violate_birth", src);
    assert!(status.success(), "non-zero: {:?}\nstdout:\n{}", status, stdout);
    assert!(
        stdout.contains("absorbed=birth_fatal"),
        "expected absorbed birth_fatal closure: {:?}",
        stdout
    );
    assert!(
        stdout.contains("parent.run continued"),
        "expected run() to keep going after birth-time violation: {:?}",
        stdout
    );
}

// Note (2026-05-19): a companion `violate_in_dissolve_*` test
// would normally pair with `violate_in_birth_*` since both
// codegen-level restrictions are lifted by the same change.
// However, the v1 `parent_accepts_us` trade-off (accepted
// children skip dissolve bodies entirely; see comments in
// `lower_locus_instantiation`'s defer-branch) means the only
// pattern that can route a dissolve violation to a parent's
// `on_failure` — a Child explicitly accepted by Parent — has no
// dissolve firing in the first place. Dissolve-time violate is
// codegen-correct (the same machinery as birth-time fires) but
// not observable until that v1 trade-off is revisited. The
// BytesBuilder F.29 cascade path DOES fire `dissolve` on
// LocusRef-typed param fields; tests there cover the relevant
// surface.

#[test]
fn violate_routes_to_parent_on_failure_in_native_binary() {
    let src = r#"
locus Child {
    closure fatal_io { epoch inline; }
    fn step() {
        violate fatal_io;
    }
}

locus Parent {
    accept(c: Child) { }
    on_failure(c: Child, err: ClosureViolation) {
        println("absorbed closure=", err.closure);
    }
    run() {
        let c = Child { };
        c.step();
        println("parent.run continued");
    }
}

fn main() { Parent { }; }
"#;
    let (stdout, status) = build_and_run("violate_routes", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("absorbed closure=fatal_io"),
        "expected absorbed closure name in stdout; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("parent.run continued"),
        "expected run() to keep going after Child.step diverged; got: {:?}",
        stdout
    );
}

#[test]
fn self_draining_reads_true_after_violate_in_compiled() {
    let src = r#"
locus Child {
    closure fatal { epoch inline; }
    fn step() {
        violate fatal;
    }
    fn drained() -> Bool {
        return self.draining;
    }
}

locus Parent {
    accept(c: Child) { }
    on_failure(c: Child, err: ClosureViolation) { }
    run() {
        let c = Child { };
        c.step();
        if c.drained() {
            println("ok draining");
        } else {
            println("FAIL not draining");
        }
    }
}

fn main() { Parent { }; }
"#;
    let (stdout, status) = build_and_run("violate_draining", src);
    assert!(status.success());
    assert!(
        stdout.contains("ok draining"),
        "expected draining flag set; got: {:?}",
        stdout
    );
}

#[test]
fn statement_after_violate_does_not_execute_in_compiled() {
    let src = r#"
locus Child {
    params { reached: Int = 0; }
    closure fatal { epoch inline; }
    fn step() {
        violate fatal;
        self.reached = 1;
    }
    fn check() -> Int { return self.reached; }
}

locus Parent {
    accept(c: Child) { }
    on_failure(c: Child, err: ClosureViolation) { }
    run() {
        let c = Child { };
        c.step();
        if c.check() == 0 {
            println("ok tail unreached");
        } else {
            println("FAIL tail ran");
        }
    }
}

fn main() { Parent { }; }
"#;
    let (stdout, status) = build_and_run("violate_divergent", src);
    assert!(status.success());
    assert!(
        stdout.contains("ok tail unreached"),
        "expected stmt after violate to be skipped; got: {:?}",
        stdout
    );
}

#[test]
fn birth_check_absorbed_by_parent_continues_run() {
    // F.27 v2 (2026-05-20): birth_check synthesis hook. After
    // birth() completes (with locus fully constructed), each
    // declared birth_check clause's cond is evaluated; if true,
    // the named closure violates through the parent's on_failure
    // handler. Unlike a regular `violate` inside a fn body, the
    // birth_check violate does NOT divergent-return from the
    // caller's fn — it branches to a continuation block so the
    // caller (here, Parent.run) keeps running normally after
    // the absorbed violation.
    let src = r#"
locus Child {
    params { initial_cap: Int = 64; handle: Int = 0; }
    closure birth_alloc_failed { captures: initial_cap; epoch inline; }
    birth() { self.handle = self.initial_cap - 64; }
    birth_check { self.handle < 0 } -> violate birth_alloc_failed;
}

locus Parent {
    accept(c: Child) { }
    on_failure(c: Child, err: ClosureViolation) {
        println("absorbed=", err.closure);
    }
    run() {
        Child { initial_cap: 64 };
        println("first passed");
        Child { initial_cap: 32 };
        println("parent.run continued");
    }
}

fn main() { Parent { }; }
"#;
    let (stdout, status) = build_and_run("birth_check_absorbed", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    // Order: first Child passes the check (cap=64 → handle=0,
    // cond false), prints "first passed". Second Child fails
    // (cap=32 → handle=-32, cond true), parent absorbs the
    // violation, prints "absorbed", and run() continues to
    // "parent.run continued".
    let lines: Vec<&str> = stdout.lines().collect();
    let pos = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("missing {:?} in:\n{}", needle, stdout))
    };
    let first = pos("first passed");
    let absorbed = pos("absorbed=birth_alloc_failed");
    let continued = pos("parent.run continued");
    assert!(first < absorbed, "first<absorbed: {}", stdout);
    assert!(absorbed < continued, "absorbed<continued: {}", stdout);
}

#[test]
fn birth_check_unhandled_exits_nonzero_with_diagnostic() {
    // Unhandled birth_check violation — no parent on_failure
    // matches the violating locus — takes the bare panic branch:
    // dprintf the closure name to stderr + exit(1). This is the
    // same "fail loud" terminal behavior as a regular unhandled
    // violate, but the diagnostic flags it as a birth_check
    // origin so operators can distinguish it from a method-body
    // violate when reading logs.
    let src = r#"
locus Child {
    params { initial_cap: Int = 64; handle: Int = 0; }
    closure birth_alloc_failed { captures: initial_cap; epoch inline; }
    birth() { self.handle = self.initial_cap - 64; }
    birth_check { self.handle < 0 } -> violate birth_alloc_failed;
}

fn main() {
    Child { initial_cap: 32 };
    println("unreachable");
}
"#;
    let bin = harness::unique_bin("lotus_test_birth_check_panic");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        !output.status.success(),
        "expected non-zero exit on unhandled birth_check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("birth_alloc_failed"),
        "expected closure name on stderr: {:?}",
        stderr
    );
    assert!(
        stderr.contains("birth_check"),
        "expected birth_check origin marker on stderr: {:?}",
        stderr
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("unreachable"),
        "unreachable line should NOT print: {:?}",
        stdout
    );
}

#[test]
fn birth_check_passes_when_cond_false() {
    // Positive control: when birth_check's cond is false, the
    // violation is not fired and the locus proceeds normally
    // through run / drain / dissolve.
    let src = r#"
locus Child {
    params { ok: Int = 1; }
    closure never_fired { epoch inline; }
    birth_check { self.ok == 0 } -> violate never_fired;
    run() { println("child.run"); }
}

fn main() {
    Child { ok: 1 };
    println("main done");
}
"#;
    let (stdout, status) = build_and_run("birth_check_pass", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("child.run"), "child.run: {:?}", stdout);
    assert!(stdout.contains("main done"), "main done: {:?}", stdout);
}

/// Build `source`, run it, and keep all of what it said.
fn build_and_run_full(name: &str, source: &str) -> std::process::Output {
    let bin = harness::unique_bin(&format!("lotus_test_{}", name));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    output
}

/// A downstream handoff: `violate` in a method that returns a value.
/// With no parent handler the violation ends the process, and the
/// caller never observes a value: exit 1 by the violation's own exit
/// path, the runtime's line on stderr, and no `b=` on stdout.
#[test]
fn violate_in_a_value_method_with_no_handler_ends_the_process() {
    let src = r#"
locus Picker {
    params { k: Int = 21; }
    closure bad_pick { captures: k; epoch inline; }
    fn pick(flag: Bool) -> Int {
        if flag { violate bad_pick; }
        return self.k * 2;
    }
    run() {
        let b = self.pick(true);
        println("b=", b);
    }
}
fn main() { Picker { }; }
"#;
    let out = build_and_run_full("violate_value_no_handler", src);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "the violation's exit, not a signal: {:?}\n{stderr}", out.status);
    assert!(
        stderr.contains("runtime error: ClosureViolation: locus `Picker` closure `bad_pick` (inline, no parent handler)"),
        "{stderr}"
    );
    assert!(!stdout.contains("b="), "the caller observed a value: {stdout:?}");
}

/// KNOWN OPEN. When a parent's `on_failure` absorbs the violation, the
/// violating method returns to its caller (spec/semantics.md § "Inline
/// closure violation", step 5: "the method exits as a return does";
/// the caller goes on, as on_failure_per_child_type_test.hl pins), and
/// codegen hands that caller an LLVM `undef` of the declared return
/// type (`Stmt::Violate`'s `violate.return` block). The caller reads
/// garbage (measured `b=140727999518808`; a pointer-typed return would
/// be dereferenced). The spec does not say what a caller observes in
/// that case — the call diverging too, a declared fallback value, or a
/// check-time refusal — so this pins today's behaviour: the caller
/// continues past the call and the method's tail did not run. The
/// value is not asserted; it is undefined. When the decision is
/// written and implemented, this test flips to assert it.
#[test]
fn known_open_absorbed_violate_in_a_value_method_returns_an_undefined_value() {
    let src = r#"
locus Picker {
    params { k: Int = 21; }
    closure bad_pick { captures: k; epoch inline; }
    fn pick(flag: Bool) -> Int {
        if flag { violate bad_pick; }
        println("tail ran");
        return self.k * 2;
    }
}
main locus App {
    params { p: Picker = Picker { }; seen: Int = 0; }
    on_failure(c: Picker, err: ClosureViolation) { self.seen = self.seen + 1; }
    run() {
        let b = self.p.pick(true);
        println("b=", b, " seen=", self.seen);
    }
}
fn main() { App { }; }
"#;
    let out = build_and_run_full("violate_value_absorbed", src);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{:?}\n{stderr}", out.status);
    assert!(!stdout.contains("tail ran"), "violate diverges in the method: {stdout:?}");
    assert!(stdout.contains("b=") && stdout.contains(" seen=1"), "known open: the caller goes on with a value: {stdout:?}");
}
