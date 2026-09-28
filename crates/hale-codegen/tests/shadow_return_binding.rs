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
//! Each case runs the program with the matrix's two oracles
//! (`ownership_matrix.rs`): every `Box` prints a tag from its
//! `dissolve()`, and each tag must be printed exactly as often as its
//! literal was built, before the program's closing line; and
//! `LOTUS_ARENA_RESIDENCY=1` must find no live arena at exit.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

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
    build_executable(&program, &bin).expect("build");
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
    r
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
    assert!(
        r.stdout.lines().any(|l| l.trim_end() == "end"),
        "the program never reached its end:\n{}",
        r.stdout
    );
    assert_eq!(
        tags_before_end(&r.stdout, "D:o"),
        outer,
        "each returned box is reclaimed once, by its caller:\n{}",
        r.stdout
    );
    assert_eq!(
        tags_before_end(&r.stdout, "D:i"),
        inner,
        "each inner shadow is reclaimed once, by its block:\n{}",
        r.stdout
    );
    assert_eq!(
        live_arenas(&r.stderr),
        Some(0),
        "no arena outlives the program:\n{}",
        r.stderr
    );
}

/// The box, its factory, and a caller that reads the returned value
/// and lets it go at its own frame's end.
const BOX: &str = r#"
locus Box {
    params { tag: String; }
    dissolve() { println("D:" + self.tag); }
}

fn make(tag: String) -> Box {
    return Box { tag: tag };
}
"#;

#[test]
fn a_shadow_in_an_if_is_reclaimed_and_the_outer_binding_reaches_the_caller() {
    let src = format!(
        r#"{BOX}
fn e(t: Bool) -> Box {{
    let r = make("o");
    if t {{
        let r = make("i");
    }}
    return r;
}}

fn call(t: Bool) -> Int {{
    let b = e(t);
    println("got " + b.tag);
    return 1;
}}

fn main() {{
    let a = call(true);
    let c = call(false);
    println("end");
}}
"#
    );
    let r = run("shadow_return_if", &src);
    assert_eq!(
        r.stdout.lines().filter(|l| *l == "got o").count(),
        2,
        "the caller receives the outer box:\n{}",
        r.stdout
    );
    assert_reclaimed(&r, 2, 1);
}

#[test]
fn a_shadow_in_a_loop_is_reclaimed_every_iteration() {
    let src = format!(
        r#"{BOX}
fn l(n: Int) -> Box {{
    let r = make("o");
    let mut i = 0;
    while i < n {{
        let r = make("i");
        i = i + 1;
    }}
    return r;
}}

fn call(n: Int) -> Int {{
    let b = l(n);
    println("got " + b.tag);
    return 1;
}}

fn main() {{
    let a = call(3);
    println("end");
}}
"#
    );
    let r = run("shadow_return_loop", &src);
    assert!(
        r.stdout.contains("got o"),
        "the caller receives the outer box:\n{}",
        r.stdout
    );
    assert_reclaimed(&r, 1, 3);
}

#[test]
fn a_shadow_in_a_match_arm_is_reclaimed() {
    let src = format!(
        r#"{BOX}
fn m(k: Int) -> Box {{
    let r = make("o");
    match k {{
        1 -> {{
            let r = make("i");
        }},
        _ -> {{ }}
    }}
    return r;
}}

fn call(k: Int) -> Int {{
    let b = m(k);
    println("got " + b.tag);
    return 1;
}}

fn main() {{
    let a = call(1);
    let c = call(2);
    println("end");
}}
"#
    );
    let r = run("shadow_return_match", &src);
    assert_eq!(
        r.stdout.lines().filter(|l| *l == "got o").count(),
        2,
        "the caller receives the outer box:\n{}",
        r.stdout
    );
    assert_reclaimed(&r, 2, 1);
}
