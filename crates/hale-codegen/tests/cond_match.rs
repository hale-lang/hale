//! The scrutinee-less `match` — GH #509 §2.
//!
//! First-match-wins over guards was always expressible: `MatchArm`
//! carries a `guard`, so `match true { _ if cond -> … }` worked. You
//! just had to invent a scrutinee you then ignored and write `_ if`
//! on every arm.
//!
//! That shape is a `cond`, and it turns up wherever dispatch is a
//! ladder of tests rather than a shape match — HTTP routing, protocol
//! dispatch, tiered fallbacks. The sugar is parser-only and desugars
//! into exactly the ignored-scrutinee form, so typecheck, codegen,
//! the model and every judgment see the match they already saw.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_condmatch_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Arms are tried in order and the FIRST true one wins — the whole
/// point of the form. Both guards here are true, so a version that
/// evaluated them in any other order, or took the last match, gives
/// a different answer.
#[test]
fn arms_are_first_match_wins() {
    let out = run(
        "order",
        r#"
fn main() {
    let n = 5;
    println("r=", match { n > 1 -> "first", n > 2 -> "second", else -> "none" });
}
"#,
    );
    assert!(out.contains("r=first"), "{:?}", out);
}

/// `else` is the catch-all, and it is reachable.
#[test]
fn else_is_the_fallthrough() {
    let out = run(
        "fallthrough",
        r#"
fn main() {
    let n = 0;
    println("r=", match { n > 1 -> "hi", n < 0 -> "lo", else -> "mid" });
    println("only=", match { else -> 42 });
}
"#,
    );
    assert!(out.contains("r=mid"), "{:?}", out);
    assert!(out.contains("only=42"), "{:?}", out);
}

/// Both positions, and `self` methods as arm bodies — the routing
/// shape this exists for, where the arms must be direct calls on the
/// receiver rather than values.
#[test]
fn works_in_statement_position_and_calls_self_methods() {
    let out = run(
        "stmt",
        r#"
locus L {
    params { n: Int = 3; }
    fn a() -> Int { return 10; }
    fn b() -> Int { return 20; }
    fn pick() -> Int {
        return match { self.n == 1 -> self.a(), self.n == 3 -> self.b(), else -> 0 };
    }
    fn shout() {
        match {
            self.n == 3 -> { println("stmt=three"); },
            else -> { println("stmt=other"); },
        }
    }
}
fn main() { let l = L { }; println("pick=", l.pick()); l.shout(); }
"#,
    );
    assert!(out.contains("pick=20"), "{:?}", out);
    assert!(out.contains("stmt=three"), "{:?}", out);
}

/// It is SUGAR: the desugared form must behave identically, since
/// everything downstream is supposed to see the match it always saw.
#[test]
fn it_agrees_with_the_ignored_scrutinee_spelling() {
    let sugar = run(
        "sugar",
        r#"fn main() { let n = 7;
    println("v=", match { n < 5 -> "lo", n < 10 -> "mid", else -> "hi" }); }"#,
    );
    let explicit = run(
        "explicit",
        r#"fn main() { let n = 7;
    println("v=", match true { _ if n < 5 -> "lo", _ if n < 10 -> "mid", _ -> "hi" }); }"#,
    );
    assert_eq!(sugar, explicit, "the sugar must desugar to the same program");
    assert!(sugar.contains("v=mid"), "{:?}", sugar);
}
