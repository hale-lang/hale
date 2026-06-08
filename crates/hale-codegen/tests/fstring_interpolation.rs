//! v1.x-10: f-string interpolation (`f"hello {name}"`).
//!
//! The lexer recognizes the `f"..."` prefix and splits the body
//! into literal + Interp parts. The parser sub-parses each Interp
//! as an Hale expression and lowers the whole literal to
//! `Lit + to_string(expr) + Lit + ...` joined by `+`. Plain
//! double-quoted strings keep their old semantics — `{` and `}` in
//! a normal string remain literal characters.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    // Salt with pid + a process-wide counter so two tests can never
    // share a temp-build path under nextest's parallel execution
    // (the bytes_pack_read flake class).
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    bin.push(format!(
        "hale_test_fstring_{}_{}_{}",
        name,
        std::process::id(),
        seq
    ));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn fstring_ident_interpolation() {
    let src = r#"
        fn main() {
            let name = "world";
            println(f"hello {name}");
        }
    "#;
    let (stdout, status) = build_and_run("ident", src);
    assert!(status.success());
    assert!(
        stdout.contains("hello world"),
        "got: {:?}", stdout
    );
}

#[test]
fn fstring_int_interpolation_uses_to_string() {
    let src = r#"
        fn main() {
            let n = 42;
            println(f"n={n}");
        }
    "#;
    let (stdout, status) = build_and_run("int", src);
    assert!(status.success());
    assert!(stdout.contains("n=42"), "got: {:?}", stdout);
}

#[test]
fn fstring_multi_interpolation_round_trips() {
    let src = r#"
        fn main() {
            let a = 1;
            let b = 2;
            let c = "three";
            println(f"{a} + {b} = {c}");
        }
    "#;
    let (stdout, status) = build_and_run("multi", src);
    assert!(status.success());
    assert!(stdout.contains("1 + 2 = three"), "got: {:?}", stdout);
}

#[test]
fn fstring_with_dotted_member_access() {
    let src = r#"
        type User { name: String; age: Int; }
        fn main() {
            let u = User { name: "alice", age: 30 };
            println(f"{u.name} is {u.age}");
        }
    "#;
    let (stdout, status) = build_and_run("dotted", src);
    assert!(status.success());
    assert!(stdout.contains("alice is 30"), "got: {:?}", stdout);
}

#[test]
fn fstring_arithmetic_expression_in_interp() {
    // Interp body is sub-parsed as a full Hale expression, so
    // arithmetic and grouping work too.
    let src = r#"
        fn main() {
            let a = 5;
            let b = 7;
            println(f"sum={a + b}");
        }
    "#;
    let (stdout, status) = build_and_run("arith", src);
    assert!(status.success());
    assert!(stdout.contains("sum=12"), "got: {:?}", stdout);
}

#[test]
fn fstring_escaped_braces_pass_through_literally() {
    let src = r#"
        fn main() {
            println(f"json={{\"k\": 1}}");
        }
    "#;
    let (stdout, status) = build_and_run("escape", src);
    assert!(status.success());
    assert!(
        stdout.contains("json={\"k\": 1}"),
        "doubled braces should collapse to a single literal pair; got: {:?}",
        stdout
    );
}

#[test]
fn fstring_only_interpolation_no_surround() {
    let src = r#"
        fn main() {
            let n = 99;
            println(f"{n}");
        }
    "#;
    let (stdout, status) = build_and_run("bare", src);
    assert!(status.success());
    assert!(stdout.contains("99"), "got: {:?}", stdout);
}

#[test]
fn plain_string_with_braces_still_works() {
    // Backward compat: a regular "..." string keeps `{` and `}`
    // as literal characters. Several existing .hl sources rely
    // on this (e.g. apps/operational-graph/main.hl printing JSON).
    let src = r#"
        fn main() {
            println("{unchanged}");
        }
    "#;
    let (stdout, status) = build_and_run("plain", src);
    assert!(status.success());
    assert!(stdout.contains("{unchanged}"), "got: {:?}", stdout);
}
