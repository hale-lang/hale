//! v1.x-VIOLATE (F.27) — codegen / compiled-binary tests for
//! `violate NAME;`. Exercises the lowering end-to-end: compile
//! to a native binary, run it, verify stdout.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
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
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("lotus_test_birth_check_panic");
    build_executable(&program, &bin).expect("build");
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
