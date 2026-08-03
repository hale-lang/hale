//! `std::str::contains` / `starts_with` / `ends_with` (#353 item 2).
//!
//! `lotus_str_contains` and `lotus_str_starts_with` have been in the
//! runtime for a long time — they even carry `memory(read)` attributes
//! from the pure-read work, so LICM can hoist them — but neither was
//! reachable from Hale. Exposing them was plumbing, not
//! implementation. `ends_with` is genuinely new, and its absence is
//! why the trio was unusable as a set: you could ask two of the three
//! questions and had to hand-roll the third.

use std::process::Command;

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir()
        .join(format!("hale-pred-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("run")
        .arg(&f)
        .output()
        .expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn the_predicates_answer_correctly() {
    let out = run(
        "fn main() {\n\
             println(std::str::contains(\"hello world\", \"o w\"));\n\
             println(std::str::contains(\"hello\", \"zz\"));\n\
             println(std::str::starts_with(\"hello\", \"he\"));\n\
             println(std::str::starts_with(\"hello\", \"lo\"));\n\
             println(std::str::ends_with(\"hello\", \"lo\"));\n\
             println(std::str::ends_with(\"hello\", \"he\"));\n\
         }",
        "answers",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["true", "false", "true", "false", "true", "false"],
        "predicate answers wrong: {:?}",
        got
    );
}

/// Empty needle is vacuously true, and a suffix longer than the string
/// is false rather than an out-of-bounds read — the two edges a
/// hand-rolled `ends_with` gets wrong.
#[test]
fn the_edges_are_right() {
    let out = run(
        "fn main() {\n\
             println(std::str::ends_with(\"hi\", \"\"));\n\
             println(std::str::ends_with(\"hi\", \"longer\"));\n\
             println(std::str::contains(\"hi\", \"\"));\n\
         }",
        "edges",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got, vec!["true", "false", "true"], "edges wrong: {:?}", got);
}

/// They are pure, so a `@no_syscall` fn may use them freely.
#[test]
fn the_predicates_are_pure() {
    let out = run(
        "@no_syscall @deterministic\n\
         fn has(s: String) -> Bool { return std::str::contains(s, \"a\"); }\n\
         fn main() { println(has(\"abc\")); }",
        "pure",
    );
    assert_eq!(out.trim(), "true", "must certify as pure: {}", out);
}
