//! The birth-order trap diagnostic (2026-08-03, downstream handoff).
//!
//! A params field whose `run()` runs inline on the main thread and
//! statically never returns blocks every param declared after it
//! from being born — `birth()` never runs, so subscriptions and
//! sockets those loci would establish silently never exist.
//!
//! The runtime behaviour this diagnostic describes is pinned
//! separately by `hale-codegen/tests/birth_order_trap.rs`; these
//! tests only assert that the compiler says so, and — the part that
//! matters more — that it stays quiet on every shape that works.

use hale_syntax::parse_source;
use hale_types::check_program;

fn msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn fires(src: &str) -> bool {
    msgs(src).iter().any(|m| m.contains("never BORN"))
}

const BLOCKER: &str = r#"
locus Forever { run() { while true { std::time::sleep(40ms); } } }
locus Later { birth() { println("b"); } }
"#;

fn app(params: &str, placement: &str) -> String {
    format!(
        "{}\nmain locus App {{\n    params {{ {} }}\n    placement {{ {} }}\n}}\nfn main() {{ App {{ }}; }}\n",
        BLOCKER, params, placement
    )
}

#[test]
fn fires_when_a_nonreturning_inline_child_precedes_a_sibling() {
    let src = app("f: Forever = Forever { }; l: Later = Later { };", "");
    assert!(fires(&src), "expected the warning:\n{:#?}", msgs(&src));
    let m = msgs(&src)
        .into_iter()
        .find(|m| m.contains("never BORN"))
        .expect("checked above");
    // The message must name the starved field and both remedies —
    // the report this came from cost weeks precisely because the
    // symptom (a silent idle) points nowhere near the cause.
    assert!(m.contains("l: Later"), "must name the starved field: {}", m);
    assert!(m.contains("BEFORE"), "must offer the reorder: {}", m);
    assert!(m.contains("pinned"), "must offer the placement fix: {}", m);
}

#[test]
fn fires_on_explicit_pool_main() {
    let src = app(
        "f: Forever = Forever { }; l: Later = Later { };",
        "f: cooperative(pool = main);",
    );
    assert!(
        fires(&src),
        "`cooperative(pool = main)` is the same inline thread as the \
         default and blocks identically:\n{:#?}",
        msgs(&src)
    );
}

// === negative controls =========================================
//
// Each of these is a shape that MEASURABLY works (verified against
// the built compiler 2026-08-03). A false positive here would push
// people to reorder params that were already correct.

#[test]
fn silent_when_the_blocker_is_declared_last() {
    let src = app("l: Later = Later { }; f: Forever = Forever { };", "");
    assert!(!fires(&src), "correct order must be clean:\n{:#?}", msgs(&src));
}

#[test]
fn silent_when_the_blocker_is_pinned() {
    let src = app("f: Forever = Forever { }; l: Later = Later { };", "f: pinned;");
    assert!(!fires(&src), "pinned runs on its own thread:\n{:#?}", msgs(&src));
}

#[test]
fn silent_when_the_blocker_is_on_another_pool() {
    let src = app(
        "f: Forever = Forever { }; l: Later = Later { };",
        "f: cooperative(pool = io);",
    );
    assert!(
        !fires(&src),
        "an off-main pool posts run() to a worker:\n{:#?}",
        msgs(&src)
    );
}

#[test]
fn silent_on_async_io() {
    let src = app(
        "f: Forever = Forever { }; l: Later = Later { };",
        "f: cooperative(pool = io) where async_io;",
    );
    assert!(!fires(&src), "async_io run-cells park:\n{:#?}", msgs(&src));
}

#[test]
fn silent_when_run_can_return() {
    let src = r#"
locus Bounded { run() { let mut i = 0; while i < 3 { i = i + 1; } } }
locus Later { birth() { println("b"); } }
main locus App { params { f: Bounded = Bounded { }; l: Later = Later { }; } }
fn main() { App { }; }
"#;
    assert!(
        !fires(src),
        "a loop that can exit does not block:\n{:#?}",
        msgs(src)
    );
}

#[test]
fn silent_when_run_has_a_break() {
    let src = r#"
locus Breaks { run() { while true { std::time::sleep(1ms); break; } } }
locus Later { birth() { println("b"); } }
main locus App { params { f: Breaks = Breaks { }; l: Later = Later { }; } }
fn main() { App { }; }
"#;
    assert!(!fires(src), "a `break` exits:\n{:#?}", msgs(src));
}

#[test]
fn silent_when_there_is_nothing_after_the_blocker() {
    let src = app("f: Forever = Forever { };", "");
    assert!(
        !fires(&src),
        "a keep-alive with no later sibling is the NORMAL shape and \
         must never warn:\n{:#?}",
        msgs(&src)
    );
}

/// Only the FIRST blocker is reported — later ones are consequences
/// of it, not independent defects, and reporting each would bury the
/// one the user has to fix.
#[test]
fn reports_only_the_first_blocker() {
    let src = app(
        "f: Forever = Forever { }; g: Forever = Forever { }; l: Later = Later { };",
        "",
    );
    let hits: Vec<String> = msgs(&src)
        .into_iter()
        .filter(|m| m.contains("never BORN"))
        .collect();
    assert_eq!(hits.len(), 1, "expected exactly one report: {:#?}", hits);
    assert!(
        hits[0].contains("`f`"),
        "must point at the first blocker: {}",
        hits[0]
    );
}
