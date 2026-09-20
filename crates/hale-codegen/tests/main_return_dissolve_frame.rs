//! GH #789 — a `return` in `fn main` must not steal the loci bound
//! before it from main's other exit paths.
//!
//! `lower_return_inner`'s `in_main` arm called `flush_dissolve_frame`
//! at the `return` site — which POPS main's deferred-dissolve frame —
//! and then pushed an EMPTY frame back "so the post-flush bookkeeping
//! stays balanced". Every entry declared before the `return` was gone
//! from the frame the fall-through exit in `lower_program` flushes, so
//! a `main` with an early-return guard (a usage check, a flag test)
//! leaked whatever it had bound before the guard *whenever the guard
//! was not taken* — the common path. The program in the issue printed
//! only `dissolved b`; `a`'s `dissolve()` never ran, and neither did
//! its child cascade or arena destroy.
//!
//! The fix emits the teardown from a CLONE of the frame and leaves the
//! frame on the stack: `return` terminates its own block, so the frame
//! is still the truth for every other exit path. Emitting the same
//! entry on two exit paths is not a double teardown — the paths are
//! disjoint control flow — and an entry whose instantiation a path
//! never reached holds a NULL self slot that
//! `emit_deferred_entry_teardown`'s existing guard skips.
//!
//! These assert on OUTPUT, not just exit status: a missed dissolve is
//! a silent leak, not a crash. `dissolve()` printing its tag makes the
//! teardown set and its reverse-declaration ORDER observable.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    harness::unique_bin(&format!(
        "lt-main-ret-frame-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ))
}

/// Compile `src`, run it, return `(exit code, stdout lines)`.
fn run(tag: &str, src: &str) -> (Option<i32>, Vec<String>) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !stderr.contains("Sanitizer") && !stderr.contains("SIGSEGV"),
        "{} crashed during teardown; stderr: {}",
        tag,
        stderr
    );
    let lines = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    (out.status.code(), lines)
}

/// The issue's program, with the guard NOT taken. Both loci must
/// dissolve, in reverse declaration order. Before the fix this
/// printed `dissolved b` alone.
#[test]
fn untaken_return_guard_still_dissolves_what_preceded_it() {
    const SRC: &str = r#"
locus Noisy {
    params { tag: String = ""; }
    dissolve() { println("dissolved ", self.tag); }
}

fn main() {
    let a = Noisy { tag: "a" };
    if 1 == 2 {
        return 7;
    }
    let b = Noisy { tag: "b" };
}
"#;
    let (code, lines) = run("untaken", SRC);
    assert_eq!(code, Some(0), "fall-through main exits 0");
    assert_eq!(
        lines,
        vec!["dissolved b".to_string(), "dissolved a".to_string()],
        "GH #789: a locus bound BEFORE an untaken `return` guard must \
         still dissolve at main's fall-through exit, after the ones \
         bound later"
    );
}

/// The same program with the guard taken: `a` dissolves exactly once
/// (not twice — the return path and the fall-through path are
/// disjoint control flow) and `b`, never instantiated, never does.
/// The return's exit code survives the teardown.
#[test]
fn taken_return_guard_dissolves_each_locus_exactly_once() {
    const SRC: &str = r#"
locus Noisy {
    params { tag: String = ""; }
    dissolve() { println("dissolved ", self.tag); }
}

fn main() {
    let a = Noisy { tag: "a" };
    if 1 == 1 {
        return 7;
    }
    let b = Noisy { tag: "b" };
}
"#;
    let (code, lines) = run("taken", SRC);
    assert_eq!(code, Some(7), "the return's exit code survives teardown");
    assert_eq!(
        lines,
        vec!["dissolved a".to_string()],
        "the taken path dissolves `a` once and never reaches `b`"
    );
}

/// A locus bound inside the branch that returns is listed on main's
/// frame from that point on, so the fall-through exit emits a teardown
/// for it too. Its self slot is NULL on that path and the existing
/// guard must skip it — no phantom `dissolve()`, no NULL deref.
#[test]
fn a_locus_bound_only_on_the_return_path_is_skipped_on_the_other() {
    const SRC: &str = r#"
locus Noisy {
    params { tag: String = ""; }
    dissolve() { println("dissolved ", self.tag); }
}

fn main() {
    let a = Noisy { tag: "a" };
    if 1 == 2 {
        let skipped = Noisy { tag: "skipped" };
        return 9;
    }
    let b = Noisy { tag: "b" };
}
"#;
    let (code, lines) = run("nullslot", SRC);
    assert_eq!(code, Some(0));
    assert_eq!(
        lines,
        vec!["dissolved b".to_string(), "dissolved a".to_string()],
        "`skipped` was never instantiated on the fall-through path — \
         its NULL self slot must be skipped, not dissolved"
    );
}

/// Two guards in one `main`: the second `return` must not lose what
/// the first one's frame held either, and the fall-through exit owns
/// all three.
#[test]
fn two_return_guards_in_one_main() {
    const SRC: &str = r#"
locus Noisy {
    params { tag: String = ""; }
    dissolve() { println("dissolved ", self.tag); }
}

fn main() {
    let a = Noisy { tag: "a" };
    if 1 == 2 { return 1; }
    let b = Noisy { tag: "b" };
    if 1 == 2 { return 2; }
    let c = Noisy { tag: "c" };
}
"#;
    let (code, lines) = run("twoguards", SRC);
    assert_eq!(code, Some(0));
    assert_eq!(
        lines,
        vec![
            "dissolved c".to_string(),
            "dissolved b".to_string(),
            "dissolved a".to_string(),
        ],
        "each untaken guard must leave the whole frame behind"
    );
}

/// A `return` nested two blocks deep (an `if` inside a `while` inside
/// an `if`). `main`'s body owns ONE dissolve frame — `if`, `while` and
/// plain blocks do not push one — so the nesting must not change which
/// entries the `return` tears down.
#[test]
fn return_nested_two_blocks_deep_taken() {
    const SRC: &str = r#"
locus Noisy {
    params { tag: String = ""; }
    dissolve() { println("dissolved ", self.tag); }
}

fn main() {
    let a = Noisy { tag: "a" };
    if 1 == 1 {
        let inner = Noisy { tag: "inner" };
        while 1 == 1 {
            if 1 == 1 {
                return 3;
            }
        }
    }
    let b = Noisy { tag: "b" };
}
"#;
    let (code, lines) = run("deep-taken", SRC);
    assert_eq!(code, Some(3));
    assert_eq!(
        lines,
        vec!["dissolved inner".to_string(), "dissolved a".to_string()],
        "a deeply nested `return` tears down everything main owns at \
         that point, innermost binding first"
    );
}

/// The same shape with the deep `return` unreachable at runtime: the
/// fall-through exit still owns every locus, including the one bound
/// in the block that contains the `return`.
#[test]
fn return_nested_two_blocks_deep_untaken() {
    const SRC: &str = r#"
locus Noisy {
    params { tag: String = ""; }
    dissolve() { println("dissolved ", self.tag); }
}

fn main() {
    let a = Noisy { tag: "a" };
    if 1 == 1 {
        let inner = Noisy { tag: "inner" };
        while 1 == 2 {
            if 1 == 1 {
                return 3;
            }
        }
    }
    let b = Noisy { tag: "b" };
}
"#;
    let (code, lines) = run("deep-untaken", SRC);
    assert_eq!(code, Some(0));
    assert_eq!(
        lines,
        vec![
            "dissolved b".to_string(),
            "dissolved inner".to_string(),
            "dissolved a".to_string(),
        ],
        "GH #789: the nested untaken `return` must leave all three \
         entries on main's frame"
    );
}

/// The Crumb batch-3 (2026-07-28) ordering the `in_main` arm owes:
/// `return f()` evaluates `f` BEFORE any teardown. The frame clone is
/// taken after the return expression is lowered, so a locus `f` itself
/// instantiated is torn down on this path too — and `a`, bound before
/// the guard, still is.
#[test]
fn return_calling_a_fn_that_instantiates_a_locus() {
    const SRC: &str = r#"
locus Noisy {
    params { tag: String = ""; }
    dissolve() { println("dissolved ", self.tag); }
}

fn code() -> Int {
    let t = Noisy { tag: "fn-local" };
    println("evaluated");
    return 5;
}

fn main() {
    let a = Noisy { tag: "a" };
    if 1 == 1 {
        return code();
    }
    let b = Noisy { tag: "b" };
}
"#;
    let (code, lines) = run("retcall", SRC);
    assert_eq!(code, Some(5), "the called fn's value is the exit code");
    assert_eq!(
        lines,
        vec![
            "evaluated".to_string(),
            "dissolved fn-local".to_string(),
            "dissolved a".to_string(),
        ],
        "the return expression must run in a live world, and `a` must \
         still dissolve on the way out"
    );
}
