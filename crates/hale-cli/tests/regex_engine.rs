//! `std::regex` (#353 item 4).
//!
//! ## The engine class is forced, not chosen
//!
//! Backtracking buys backreferences and lookaround at the cost of an
//! exponential worst case. You cannot bound a handler that runs one,
//! which makes it flatly incompatible with `@budget` and `@hot` — the
//! two things Hale sells. A Thompson NFA simulated over a state set
//! is O(pattern × text) with no backtracking, so a match is bounded
//! by construction.
//!
//! The price is no backreferences and no lookaround. For this
//! language that is the right side of the trade: a regex you cannot
//! put on a hot path is not much use in a language whose hot paths
//! carry allocation budgets.
//!
//! Supported: literals, `.`, `*`, `+`, `?`, `|`, grouping, character
//! classes with ranges and negation, and `\` escapes.

use std::process::Command;

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir()
        .join(format!("hale-re-{}-{}", std::process::id(), tag));
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
fn the_operators_work() {
    let out = run(
        "fn main() {\n\
             println(std::regex::matches(\"h.llo\", \"hello\"));\n\
             println(std::regex::matches(\"a+b\", \"aaab\"));\n\
             println(std::regex::matches(\"a+b\", \"b\"));\n\
             println(std::regex::matches(\"ab?c\", \"ac\"));\n\
             println(std::regex::matches(\"a|b\", \"b\"));\n\
             println(std::regex::matches(\"(ab)+\", \"abab\"));\n\
             println(std::regex::matches(\"(ab)+\", \"aba\"));\n\
         }",
        "ops",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["true", "true", "false", "true", "true", "true", "false"],
        "operator semantics wrong: {:?}",
        got
    );
}

#[test]
fn character_classes_including_negation() {
    let out = run(
        "fn main() {\n\
             println(std::regex::matches(\"[a-c]+\", \"abcb\"));\n\
             println(std::regex::matches(\"[a-c]+\", \"d\"));\n\
             println(std::regex::matches(\"[^a-c]+\", \"xyz\"));\n\
             println(std::regex::matches(\"[^a-c]+\", \"abc\"));\n\
         }",
        "class",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["true", "false", "true", "false"],
        "class semantics wrong: {:?}",
        got
    );
}

/// `matches` is a FULL match; `find` scans for the leftmost one and
/// returns a byte offset, or -1. Conflating the two is the usual
/// regex-API papercut.
#[test]
fn find_returns_the_leftmost_offset() {
    let out = run(
        "fn main() {\n\
             println(std::regex::find(\"l+\", \"hello\"));\n\
             println(std::regex::find(\"zz\", \"hello\"));\n\
             println(std::regex::find(\"h\", \"hello\"));\n\
         }",
        "find",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got, vec!["2", "-1", "0"], "find offsets wrong: {:?}", got);
}

/// A malformed pattern is reported rather than silently matching
/// nothing — otherwise a typo'd pattern looks like "no results".
#[test]
fn an_invalid_pattern_is_reported() {
    let out = run(
        "fn main() {\n\
             println(std::regex::valid(\"a(\"));\n\
             println(std::regex::valid(\"[a-\"));\n\
             println(std::regex::valid(\"a+b\"));\n\
         }",
        "valid",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["false", "false", "true"],
        "validity wrong: {:?}",
        got
    );
}

/// THE reason for the engine choice. A match must be usable from a
/// certified fn — if this fails, the linear-time property has been
/// lost or the classification is wrong.
#[test]
fn matching_is_pure_and_certifiable() {
    let out = run(
        "@no_syscall @deterministic\n\
         fn ok(p: String, t: String) -> Bool {\n\
             return std::regex::matches(p, t);\n\
         }\n\
         fn main() { println(ok(\"a+\", \"aaa\")); }",
        "pure",
    );
    assert_eq!(
        out.trim(),
        "true",
        "regex must certify under @no_syscall @deterministic: {}",
        out
    );
}
