//! GH #265 step 7 — the **conformance loop**: differentially check
//! the static effect analysis against what the running binary
//! actually does.
//!
//! Every other test in the effect suite checks that the analysis
//! reports what we *expect*. This one checks the analysis against
//! *reality*: compile a program whose fns carry effect assertions,
//! run it under the runtime's own counters, and confirm the
//! observed behaviour matches the static claim. A fn the compiler
//! certified `@no_syscall` that performs a syscall at runtime is a
//! **caught soundness bug in the analysis itself** — the one thing
//! no amount of "the checker says what I expect" testing can find.
//!
//! Same philosophy as GenMC-in-CI (model-check what stress can't),
//! applied to effects: static classification is a claim, the
//! runtime is the oracle.
//!
//! This is the cheap, always-on half. The full loop the issue
//! describes — running the whole corpus under `LOTUS_OBS=1` and
//! differentially checking every asserted fn against emitted
//! records — rides the observation surface and is a follow-on; the
//! seam is `std::diag::syscall_count` / `heap_alloc_count`, which
//! the runtime already exposes for exactly this purpose.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(tag: &str, src: &str) -> (String, bool) {
    let program = hale_syntax::parse_source(src).expect("parse");
    // The program must also pass the static effect checks — if it
    // doesn't, the conformance question is moot.
    let diags = hale_types::check_program(&program);
    let hard: Vec<String> = diags
        .iter()
        .filter(|d| matches!(d.kind, hale_syntax::DiagKind::Type))
        .map(|d| d.message.clone())
        .collect();
    assert!(
        hard.is_empty(),
        "program must satisfy its own assertions statically first: {:?}",
        hard
    );
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_conform_{}_{}", tag, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.success(),
    )
}

/// A fn the compiler certified `@no_syscall` must perform no
/// syscalls at runtime. The runtime's own `std::diag::syscall_count`
/// is the oracle: sample it around the certified call and assert the
/// delta is zero.
#[test]
fn no_syscall_claim_holds_at_runtime() {
    let src = r#"
        @no_syscall fn compute(n: Int) -> Int {
            let mut acc = 0;
            let mut i = 0;
            while i < n {
                acc = acc + i * 3;
                i = i + 1;
            }
            return acc;
        }
        fn main() {
            let before = std::diag::syscall_count("write");
            let r = compute(1000);
            let after = std::diag::syscall_count("write");
            println("delta=", after - before);
            println("r=", r);
        }
    "#;
    let (out, ok) = build_and_run("nosys", src);
    assert!(ok, "program exited nonzero: {}", out);
    assert!(
        out.contains("delta=0"),
        "a @no_syscall-certified fn performed syscalls at runtime — the \
         STATIC ANALYSIS IS UNSOUND, not the test. stdout: {}",
        out
    );
}

/// The counted form: `@budget(alloc_per_call = 0)` claims zero arena
/// allocations per call. `std::diag::heap_alloc_count` is the
/// runtime oracle for that claim.
#[test]
fn zero_alloc_budget_holds_at_runtime() {
    let src = r#"
        @budget(alloc_per_call = 0) fn scale(n: Int) -> Int {
            return n * 7 + 1;
        }
        fn main() {
            let before = std::diag::heap_alloc_count();
            let mut i = 0;
            let mut acc = 0;
            while i < 500 {
                acc = acc + scale(i);
                i = i + 1;
            }
            let after = std::diag::heap_alloc_count();
            println("delta=", after - before);
            println("acc=", acc);
        }
    "#;
    let (out, ok) = build_and_run("zeroalloc", src);
    assert!(ok, "program exited nonzero: {}", out);
    assert!(
        out.contains("delta=0"),
        "a zero-alloc-certified fn allocated at runtime — the STATIC \
         ANALYSIS IS UNSOUND. stdout: {}",
        out
    );
}

/// The negative control: the oracle must be capable of *detecting*
/// the effect it's asked about. Without this, a broken counter would
/// make every conformance test vacuously pass.
#[test]
fn the_oracle_detects_effects_when_they_happen() {
    let src = r#"
        type Blob { a: Int; b: Int; }
        fn allocates(n: Int) -> Int {
            let b = Blob { a: n, b: n + 1 };
            return b.a + b.b;
        }
        fn main() {
            let before = std::diag::heap_alloc_count();
            let mut i = 0;
            let mut acc = 0;
            while i < 200 {
                acc = acc + allocates(i);
                i = i + 1;
            }
            let after = std::diag::heap_alloc_count();
            let grew: Bool = after > before;
            println("grew=", grew);
            println("acc=", acc);
        }
    "#;
    let (out, ok) = build_and_run("oracle", src);
    assert!(ok, "program exited nonzero: {}", out);
    assert!(
        out.contains("grew=true"),
        "the allocation oracle did not observe allocations that certainly \
         happened — the conformance tests above would be vacuous. \
         stdout: {}",
        out
    );
}
