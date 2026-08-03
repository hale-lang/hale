//! `std::str::split_into` (#353 item 2).
//!
//! Splitting a string is the most common single operation in service
//! code, and Hale had no way to do it.
//!
//! What Hale cannot do is RETURN a sequence: arrays are fixed-size
//! types (`[Int; 3]` and `[Int; 2]` do not unify, and there is no
//! general slice), and growable collections exist only as locus-owned
//! `@form(...)`. So rather than guess at the sequence-value model —
//! the language's largest open question — this uses the shape the
//! stdlib already adopted for exactly that reason,
//! `text::tokenize_words_into`: write into caller-supplied storage.
//!
//! That is not a workaround. It is the allocation-visible form. The
//! caller owns the storage, so the cost lands in the caller's budget
//! instead of hiding behind a return value, and a `@hot` handler can
//! reuse one vec across calls rather than allocating a fresh
//! collection per message.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(
    name: &str,
    src: &str,
    argv: &[&str],
) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_split_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn it_splits_on_a_separator() {
    let src = r#"
        @form(vec)
        locus Fields { capacity { heap items of String; } }
        fn main() {
            let f = Fields { };
            std::str::split_into("alpha,beta,gamma", ",", f);
            println("n=", f.len());
            let mut i = 0;
            while i < f.len() {
                println(f.get(i) or "");
                i = i + 1;
            }
        }
    "#;
    let (stdout, status) = build_and_run("split_basic", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("n=3"), "got: {:?}", stdout);
    for w in ["alpha", "beta", "gamma"] {
        assert!(stdout.contains(w), "missing {}: {:?}", w, stdout);
    }
}

/// Adjacent and trailing separators produce EMPTY fields rather than
/// being collapsed. `"a,,b,"` is four fields, which is what every
/// split worth having does — collapsing them silently loses a column
/// in a CSV row and the loss is invisible at the call site.
#[test]
fn empty_fields_are_preserved() {
    let src = r#"
        @form(vec)
        locus Fields { capacity { heap items of String; } }
        fn main() {
            let f = Fields { };
            std::str::split_into("a,,b,", ",", f);
            println("n=", f.len());
            println("[", f.get(1) or "?", "]");
            println("[", f.get(3) or "?", "]");
        }
    "#;
    let (stdout, status) = build_and_run("split_empty", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("n=4"), "got: {:?}", stdout);
    assert!(stdout.contains("[]"), "empty fields must survive: {:?}", stdout);
}

/// A separator that does not occur yields the whole string as one
/// field — not zero fields, which would silently drop the input.
#[test]
fn a_missing_separator_yields_the_whole_string() {
    let src = r#"
        @form(vec)
        locus Fields { capacity { heap items of String; } }
        fn main() {
            let f = Fields { };
            std::str::split_into("nosep", ",", f);
            println("n=", f.len());
            println(f.get(0) or "?");
        }
    "#;
    let (stdout, status) = build_and_run("split_nosep", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("n=1"), "got: {:?}", stdout);
    assert!(stdout.contains("nosep"), "got: {:?}", stdout);
}

/// A multi-byte separator, since the naive implementation advances by
/// one byte and silently mis-splits.
#[test]
fn a_multi_byte_separator_works() {
    let src = r#"
        @form(vec)
        locus Fields { capacity { heap items of String; } }
        fn main() {
            let f = Fields { };
            std::str::split_into("a::b::c", "::", f);
            println("n=", f.len());
            println(f.get(1) or "?");
        }
    "#;
    let (stdout, status) = build_and_run("split_multi", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("n=3"), "got: {:?}", stdout);
    assert!(stdout.contains("b"), "got: {:?}", stdout);
}

// ---- join, the pair --------------------------------------------
//
// Note `join` RETURNS where `split_into` writes. A String is already
// a value in Hale, so joining never meets the sequence-value question
// that forces split's shape. The asymmetry reflects the language, not
// an inconsistency in the API.

#[test]
fn split_and_join_round_trip() {
    let src = r#"
        @form(vec)
        locus Fields { capacity { heap items of String; } }
        fn main() {
            let f = Fields { };
            std::str::split_into("alpha,beta,gamma", ",", f);
            println(std::str::join(f, ","));
            println(std::str::join(f, "-"));
            println(std::str::join(f, ""));
        }
    "#;
    let (stdout, status) = build_and_run("join_round", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("alpha,beta,gamma"),
        "join(split(s, sep), sep) must reproduce s: {:?}",
        stdout
    );
    assert!(stdout.contains("alpha-beta-gamma"), "got: {:?}", stdout);
    assert!(stdout.contains("alphabetagamma"), "got: {:?}", stdout);
}

/// An empty collection joins to the empty string, not to a separator
/// and not to a crash — the case every hand-rolled join gets wrong.
#[test]
fn joining_nothing_is_empty() {
    let src = r#"
        @form(vec)
        locus Fields { capacity { heap items of String; } }
        fn main() {
            let e = Fields { };
            println("[", std::str::join(e, ","), "]");
        }
    "#;
    let (stdout, status) = build_and_run("join_empty", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("[]"),
        "empty join must be empty, with no stray separator: {:?}",
        stdout
    );
}
