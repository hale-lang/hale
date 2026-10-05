//! GH #1140: a `let` that shadows a name the fn returns is another
//! binding, and is reclaimed like one.
//!
//! ```text
//! fn e(t: Bool) -> Box {
//!     let r = make("o");
//!     if t { let r = make("i"); }
//!     return r;
//! }
//! ```
//!
//! The ownership pre-pass and codegen's `returned_bindings` used to
//! decide by NAME. The inner `r` counted as "the binding the fn
//! returns" as much as the outer one: it was routed to the caller and
//! exempted from its block's reclaim, so it leaked. The fresh-factory
//! walk saw the name bound twice, gave up, and the caller did not own
//! the outer value either. Both passes now resolve a returned name to
//! the declaration in scope where the return spells it.
//!
//! The oracles are the matrix's (`ownership_matrix.rs`), plus order.
//! Every `Box` prints a tag from its `dissolve()`. The tags are built
//! on the heap (`s + "o"`), because a literal's static bytes would hide
//! a use-after-free.
//!
//! - **Reclaimed:** each tag is printed exactly as often as its box was
//!   built, before the program's closing line, and
//!   `LOTUS_ARENA_RESIDENCY=1` finds no live arena at exit.
//! - **Safe:** a box the caller received is never dissolved before the
//!   caller read it, nor more often than it was received, and an ASan
//!   build of the same program (chunk recycling off, GH #816) reports no
//!   use-after-free or double free.
//!
//! The safety cases are the shapes resolving by binding must not break.
//! In each, the value reaches the caller through a `return`, an `=` or
//! a block tail inside an expression block. Resolving by name covered
//! them by coincidence; the walk has to cover them by resolution.

use std::collections::BTreeMap;
use std::process::Command;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

struct Run {
    stdout: String,
    stderr: String,
}

fn run(name: &str, src: &str) -> Run {
    let program = hale_syntax::parse_source(src).expect("parse");
    let errors: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect();
    assert!(
        errors.is_empty(),
        "the checker refused it: {errors:?}\n{src}"
    );
    let bin = harness::unique_bin(name);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let r = Run {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    };
    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        r.stdout,
        r.stderr
    );
    assert!(
        r.stdout.lines().any(|l| l.trim_end() == "end"),
        "the program never reached its end:\n{}",
        r.stdout
    );
    assert_safe(&r);
    assert_no_sanitizer_report(name, &program);
    r
}

/// The same program under AddressSanitizer: no use-after-free, no
/// double free. Leaks are the residency oracle's, so LeakSanitizer is
/// off here.
fn assert_no_sanitizer_report(name: &str, program: &hale_syntax::ast::Program) {
    let bin = harness::unique_bin(&format!("{name}_asan"));
    harness::build_asan(program, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .expect("run the asan build");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for marker in ["ERROR: AddressSanitizer", "heap-use-after-free", "double-free"] {
        assert!(
            !stderr.contains(marker),
            "{name}: the sanitizer reported `{marker}`:\n{stderr}"
        );
    }
    assert!(out.status.success(), "{name}: the asan build failed: {:?}\n{stderr}", out.status);
}

/// A box the caller received (`got <tag>`) is dissolved only after the
/// caller read it, and at most once per time it was received.
fn assert_safe(r: &Run) {
    let received: Vec<&str> = r
        .stdout
        .lines()
        .filter_map(|l| l.trim_end().strip_prefix("got "))
        .collect();
    let mut got: BTreeMap<&str, usize> = BTreeMap::new();
    let mut gone: BTreeMap<&str, usize> = BTreeMap::new();
    for line in r.stdout.lines().map(str::trim_end) {
        if let Some(tag) = line.strip_prefix("got ") {
            *got.entry(tag).or_default() += 1;
        } else if let Some(tag) = line.strip_prefix("D:") {
            if !received.contains(&tag) {
                continue;
            }
            let n = gone.entry(tag).or_default();
            *n += 1;
            assert!(
                *n <= got.get(tag).copied().unwrap_or(0),
                "`{tag}` was dissolved before its caller read it, or twice:\n{}",
                r.stdout
            );
        }
    }
}

/// Tag lines printed before `end`, as whole lines.
fn tags_before_end(stdout: &str, tag: &str) -> usize {
    stdout
        .lines()
        .take_while(|l| l.trim_end() != "end")
        .filter(|l| l.trim_end() == tag)
        .count()
}

/// `[arena_residency dump] N live arenas, …` from the atexit hook.
fn live_arenas(stderr: &str) -> Option<usize> {
    stderr
        .lines()
        .find(|l| l.contains("[arena_residency dump]"))
        .and_then(|l| l.split_whitespace().nth(2))
        .and_then(|n| n.parse().ok())
}

fn assert_reclaimed(r: &Run, outer: usize, inner: usize) {
    assert_eq!(
        tags_before_end(&r.stdout, "D:xo"),
        outer,
        "each returned box is reclaimed once, by its caller:\n{}",
        r.stdout
    );
    assert_eq!(
        tags_before_end(&r.stdout, "D:xi"),
        inner,
        "each inner shadow is reclaimed once:\n{}",
        r.stdout
    );
    assert_eq!(
        live_arenas(&r.stderr),
        Some(0),
        "no arena outlives the program:\n{}",
        r.stderr
    );
}

/// The box and its factory. Callers pass `s` = "x", so every tag is
/// built at run time.
const BOX: &str = r#"
locus Box {
    params { tag: String; }
    dissolve() { println("D:" + self.tag); }
}

fn make(tag: String) -> Box {
    return Box { tag: tag };
}
"#;

/// A caller that reads the returned box and lets it go at its own
/// frame's end, and a main that calls it with each argument.
fn with_callers(body: &str, arg: &str, calls: &[&str]) -> String {
    let calls: String = calls
        .iter()
        .enumerate()
        .map(|(i, c)| format!("    let c{i} = call({c});\n"))
        .collect();
    format!(
        "{BOX}\n{body}\nfn call(t: {arg}) -> Int {{\n    let b = e(t, \"x\");\n    println(\"got \" + b.tag);\n    return 1;\n}}\n\nfn main() {{\n{calls}    println(\"end\");\n}}\n"
    )
}

#[test]
fn a_shadow_in_an_if_is_reclaimed_and_the_outer_binding_reaches_the_caller() {
    let src = with_callers(
        r#"fn e(t: Bool, s: String) -> Box {
    let r = make(s + "o");
    if t {
        let r = make(s + "i");
    }
    return r;
}"#,
        "Bool",
        &["true", "false"],
    );
    let r = run("shadow_return_if", &src);
    assert_eq!(
        r.stdout.lines().filter(|l| *l == "got xo").count(),
        2,
        "the caller receives the outer box:\n{}",
        r.stdout
    );
    assert_reclaimed(&r, 2, 1);
}

#[test]
fn a_shadow_in_a_loop_is_reclaimed_every_iteration() {
    let src = with_callers(
        r#"fn e(n: Int, s: String) -> Box {
    let r = make(s + "o");
    let mut i = 0;
    while i < n {
        let r = make(s + "i");
        i = i + 1;
    }
    return r;
}"#,
        "Int",
        &["3"],
    );
    let r = run("shadow_return_loop", &src);
    assert!(
        r.stdout.contains("got xo"),
        "the caller receives the outer box:\n{}",
        r.stdout
    );
    assert_reclaimed(&r, 1, 3);
}

#[test]
fn a_shadow_in_a_match_arm_is_reclaimed() {
    let src = with_callers(
        r#"fn e(k: Int, s: String) -> Box {
    let r = make(s + "o");
    match k {
        1 -> {
            let r = make(s + "i");
        },
        _ -> { }
    }
    return r;
}"#,
        "Int",
        &["1", "2"],
    );
    let r = run("shadow_return_match", &src);
    assert_eq!(
        r.stdout.lines().filter(|l| *l == "got xo").count(),
        2,
        "the caller receives the outer box:\n{}",
        r.stdout
    );
    assert_reclaimed(&r, 2, 1);
}

// ---- the shapes resolution must cover (review of PR #1210) --------

#[test]
fn a_write_inside_an_expression_block_to_the_returned_binding_is_the_callers() {
    let src = with_callers(
        r#"fn e(t: Bool, s: String) -> Box {
    let mut r = make(s + "o");
    let n = if t { r = Box { tag: s + "i" }; 0 } else { 1 };
    return r;
}"#,
        "Bool",
        &["true", "false"],
    );
    let r = run("shadow_return_expr_write", &src);
    assert!(
        r.stdout.contains("got xi") && r.stdout.contains("got xo"),
        "{}",
        r.stdout
    );
}

#[test]
fn a_shadow_returned_from_inside_an_expression_block_is_the_callers() {
    let src = with_callers(
        r#"fn e(t: Bool, s: String) -> Box {
    let r = make(s + "o");
    let n = if t { let r = make(s + "i"); return r; 0 } else { 1 };
    return r;
}"#,
        "Bool",
        &["true", "false"],
    );
    let r = run("shadow_return_expr_return", &src);
    assert!(
        r.stdout.contains("got xi") && r.stdout.contains("got xo"),
        "{}",
        r.stdout
    );
}

#[test]
fn an_expression_block_tail_that_flows_into_the_returned_binding_is_the_callers() {
    let src = with_callers(
        r#"fn e(t: Bool, s: String) -> Box {
    let r = make(s + "o");
    let y = if t { let r = Box { tag: s + "i" }; r } else { make(s + "e") };
    if t { return y; }
    return r;
}"#,
        "Bool",
        &["true", "false"],
    );
    let r = run("shadow_return_expr_tail", &src);
    assert!(
        r.stdout.contains("got xi") && r.stdout.contains("got xo"),
        "{}",
        r.stdout
    );
}

#[test]
fn an_outer_binding_and_a_shadow_both_returned_are_each_reclaimed_once() {
    let src = with_callers(
        r#"fn e(t: Int, s: String) -> Box {
    let r = make(s + "o");
    let n = if t == 1 { return r; 0 } else { 1 };
    if t == 2 {
        let r = make(s + "i");
        return r;
    }
    return make(s + "z");
}"#,
        "Int",
        &["1", "2", "3"],
    );
    let r = run("shadow_return_both", &src);
    assert!(
        r.stdout.contains("got xo")
            && r.stdout.contains("got xi")
            && r.stdout.contains("got xz"),
        "{}",
        r.stdout
    );
}
