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

/// A downstream handoff: `violate` in a method that returns a value,
/// declared `fallible(ClosureViolation)` as F.42 requires. With no
/// parent handler the violation ends the process before the call can
/// fail, and the caller never observes a value or runs its `or`: exit 1
/// by the violation's own exit path, the runtime's line on stderr, and
/// no `b=` on stdout.
#[test]
fn violate_in_a_value_method_with_no_handler_ends_the_process() {
    let src = r#"
locus Picker {
    params { k: Int = 21; }
    closure bad_pick { captures: k; epoch inline; }
    fn pick(flag: Bool) -> Int fallible(ClosureViolation) {
        if flag { violate bad_pick; }
        return self.k * 2;
    }
    run() {
        let b = self.pick(true) or { println("or ran"); 7 };
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
    assert!(!stdout.contains("b=") && !stdout.contains("or ran"), "the caller observed the call: {stdout:?}");
}

/// The program of the downstream handoff's measurement (an `Int` result
/// of `104616817497632` after its supervisor's `on_failure` ran, when
/// lowering returned an `undef`), as F.42 writes it: the violating
/// method is `fallible(ClosureViolation)`, so when the parent's
/// `on_failure` absorbs the violation the call fails. The parent's
/// handler runs first, then the caller's `or` with `err` the record the
/// handler received; the method's tail does not run; the value is the
/// fallback. A call that does not violate still returns its value.
const ABSORBED_IN_A_FALLIBLE_METHOD: &str = r#"
locus Picker {
    params { k: Int = 21; }
    closure bad_pick { captures: k; epoch inline; }
    fn pick(flag: Bool) -> Int fallible(ClosureViolation) {
        if flag { violate bad_pick; }
        println("tail ran");
        return self.k * 2;
    }
}
main locus App {
    params { p: Picker = Picker { }; seen: Int = 0; }
    on_failure(c: Picker, err: ClosureViolation) {
        println("on_failure ", err.closure);
        self.seen = self.seen + 1;
    }
    run() {
        let a = self.p.pick(false) or 0;
        println("a=", a);
        let b = self.p.pick(true) or { println("or seen=", self.seen, " closure=", err.closure, " locus=", err.locus); 7 };
        println("b=", b);
        let c = self.p.pick(true) or 7;
        println("c=", c, " seen=", self.seen);
    }
}
fn main() { App { }; }
"#;

/// What it prints: `tail ran` once (the call that did not violate),
/// each handler before its `or`.
const ABSORBED_EXPECTED: &str = "tail ran\na=42\non_failure bad_pick\nor seen=1 closure=bad_pick locus=Picker\nb=7\non_failure bad_pick\nc=7 seen=2\n";

#[test]
fn absorbed_violate_in_a_fallible_method_is_the_call_s_failure() {
    let out = build_and_run_full("violate_value_absorbed", ABSORBED_IN_A_FALLIBLE_METHOD);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{:?}\n{stderr}", out.status);
    assert_eq!(stdout, ABSORBED_EXPECTED, "handler first, then the `or`, and no value: {stderr}");
}

/// The same program under AddressSanitizer, the arena's chunk pool off
/// (GH #816): the record the handler read is copied out to the caller's
/// error slot before the method's scratch is freed, and the `or` block
/// reads it there.
#[test]
fn absorbed_violate_in_a_fallible_method_is_clean_under_asan() {
    let bin = harness::unique_bin("lotus_test_violate_value_absorbed_asan");
    let opts = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(ABSORBED_IN_A_FALLIBLE_METHOD, &bin, &opts).expect("build");
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{:?}\n{stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    assert_eq!(stdout, ABSORBED_EXPECTED, "{stderr}");
}

/// The cross-pool case (decision L0-1; the shape of
/// `failure_delivery/fd_pool_owner_worker.hl`): the owner is placed on
/// pool `side` and main calls into its child, so the violation is raised
/// off the owner's domain. The owner's handler is posted to the pool's
/// worker and awaited (it sleeps first, so a caller that did not wait
/// would see `heard=0`); only then does the caller's `or` run.
#[test]
fn cross_pool_violate_in_a_fallible_method_awaits_the_owner_s_handler() {
    let src = r#"
@ffi("c") fn pthread_self() -> Int;

locus Kid {
    params { n: Int = 0; }
    closure fuse { captures: n; epoch inline; }
    fn pick() -> Int fallible(ClosureViolation) {
        violate fuse;
    }
}

locus Owner {
    params {
        worker: Int = pthread_self();
        kid: Kid = Kid { };
        posted_on: Int = 0;
        heard: Int = 0;
    }
    on_failure(c: Kid, err: ClosureViolation) {
        std::time::sleep(20ms);
        self.posted_on = pthread_self();
        self.heard = self.heard + 1;
    }
}

main locus App {
    params { o: Owner = Owner { }; }
    placement { o: cooperative(pool = side); }
    run() {
        let mine = pthread_self();
        let b = self.o.kid.pick() or { println("or heard=", self.o.heard, " closure=", err.closure); 7 };
        println("b=", b);
        if mine != self.o.worker { println("raised off the worker"); }
        if self.o.posted_on == self.o.worker { println("handled on the worker"); }
    }
}

fn main() { App { }; }
"#;
    let out = build_and_run_full("violate_value_cross_pool", src);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{:?}\n{stderr}", out.status);
    assert_eq!(
        stdout,
        "or heard=1 closure=fuse\nb=7\nraised off the worker\nhandled on the worker\n",
        "{stderr}"
    );
}

/// A method serving a perspective, the perspective fn and the impl both
/// `fallible(ClosureViolation)`: `hale check` passes (conformance matches
/// the fallibility, `hale-types`' `violate_fallible.rs` pins it), and the
/// build refuses the perspective call, as it refused every `fallible`
/// perspective call before F.42. This is why F.42 exempts a
/// perspective-served method: the day the call lowers, this pin fails
/// and the exemption goes.
#[test]
fn a_fallible_perspective_call_is_refused_at_the_build() {
    let src = r#"
perspective Router { fn route(code: Int) -> Int fallible(ClosureViolation); }
locus RouterV1 : serves Router {
    params { k: Int = 0; }
    closure c { captures: k; epoch inline; }
    fn route(code: Int) -> Int fallible(ClosureViolation) { if code > 5 { violate c; } return code + 100; }
}
locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; }
    on_failure(r: RouterV1, err: ClosureViolation) { println("absorbed ", err.closure); }
    run() { println(self.router.route(1) or 0); }
}
main locus App { params { gw: Gateway = Gateway { }; } }
fn main() { App { }; }
"#;
    let bin = harness::unique_bin("lotus_test_violate_fallible_perspective_call");
    let err = build_opts::build_source(src, &bin, &build_opts::options()).expect_err("the build refuses it");
    let _ = std::fs::remove_file(&bin);
    assert!(
        err.to_string().contains("fallible method call on non-locus value of type Perspective(\"Router\")"),
        "{err}"
    );
}
