//! `if let` and `let … else` (GH #1995 E3), through the `hale` binary:
//! what `hale check` refuses `hale build` refuses at the same message,
//! never in codegen, and what the check passes builds. What the forms
//! compute is `tests/hale/if_let_test.hl`'s.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(Path::new("/")).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// A scratch dir holding `main.hl`, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(src: &str, tag: &str) -> Scratch {
        let d = std::env::temp_dir().join(format!("hale_check_if_let_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("main.hl"), src).unwrap();
        Scratch(d)
    }
    fn file(&self) -> String {
        self.0.join("main.hl").to_string_lossy().to_string()
    }
    fn check(&self) -> (bool, String) {
        hale(&["check", &self.file()])
    }
    fn build(&self) -> (bool, String) {
        hale(&["build", &self.file(), "-o", &self.0.join("bin").to_string_lossy()])
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The check refuses `src` at `main.hl:<at>` with a type error starting
/// `head`, and the build stops at the same message, never reaching
/// codegen.
fn refused(src: &str, tag: &str, at: &str, head: &str) -> String {
    let s = Scratch::new(src, tag);
    let (ok, out) = s.check();
    assert!(!ok && out.contains(&format!("main.hl:{at}: type error: {head}")), "check: {out}");
    let (built, bout) = s.build();
    assert!(!built && bout.contains(head), "build: {bout}");
    assert!(!bout.contains("codegen error"), "the build stops at the check: {bout}");
    out
}

const SIDE: &str = "type Side = enum { Buy, Other(String) };\n";

/// The `else` of a `let … else` must leave, as F.48's definite return
/// judges a path: an `else` that can complete is refused at the block,
/// the completing path named as the related location.
#[test]
fn an_else_that_can_complete_is_refused() {
    let out = refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    let Side::Other(raw) = s else {{\n        let n = 1;\n    }};\n    return len(raw);\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "completes",
        "3:35",
        "the `else` of a `let … else` must leave (`return`, `fail`, `break`, `continue`, `terminate` or `violate`): \
         what follows the statement runs only when `Side::Other(raw)` matches; this `else` can complete: the `else` \
         block ends after this statement",
    );
    assert!(out.contains("note: the path that completes at") && out.contains("main.hl:4:9"), "{out}");
    assert!(!out.contains("a call completes"), "no call, no call hint: {out}");

    // an `if` with no `else` inside it
    refused(
        &format!(
            "{SIDE}fn f(s: Side, n: Int) -> Int {{\n    let Side::Other(raw) = s else {{\n        if n > 0 {{ return 1; }}\n    }};\n    return len(raw);\n}}\nfn main() {{ println(f(Side::Buy, 1)); }}\n"
        ),
        "no_else",
        "3:35",
        "the `else` of a `let … else` must leave",
    );

    // an empty block
    refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    let Side::Other(raw) = s else {{}};\n    return len(raw);\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "empty",
        "3:35",
        "the `else` of a `let … else` must leave",
    );
}

/// A call completes as far as F.48 goes, `std::process::exit` included:
/// the refusal says to write `return` after it.
#[test]
fn a_call_that_never_returns_is_a_call_that_completes() {
    let out = refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    let Side::Other(raw) = s else {{\n        std::process::exit(1);\n    }};\n    return len(raw);\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "exit",
        "3:35",
        "the `else` of a `let … else` must leave",
    );
    assert!(out.contains("`std::process::exit` included, so write `return` after it"), "{out}");
}

/// A pattern that can fail to match is refused in a plain `let`, which
/// used to be a parse error ("expected =, got ColonColon"; "expected
/// variable name, got IntLit"): "add `else { … }`".
#[test]
fn a_refutable_pattern_in_a_plain_let_is_refused() {
    refused(
        &format!("{SIDE}fn f(s: Side) -> Int {{\n    let Side::Other(raw) = s;\n    return len(raw);\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"),
        "enum",
        "3:9",
        "`Side::Other(raw)` can fail to match a `Side`, so a plain `let` cannot bind it: add `else { … }` that \
         leaves when it does not match (`let Side::Other(raw) = … else { return …; };`), or test it with `if let` \
         or `match`",
    );
    refused(
        "fn f(p: (Int, Int)) -> Int {\n    let (a, 0) = p;\n    return a;\n}\nfn main() { println(f((1, 0))); }\n",
        "tuple",
        "2:9",
        "`(a, 0)` can fail to match a `(Int, Int)`, so a plain `let` cannot bind it: add `else { … }`",
    );
}

/// The alternatives of an or-pattern in an `if let` or a `let … else`
/// bind the same names, as a match arm's do (GH #1995 E2).
#[test]
fn an_or_pattern_binding_other_names_is_refused() {
    refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    if let Side::Other(a) | Side::Buy = s {{ return 1; }}\n    return 0;\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "if_let_or",
        "3:29",
        "the alternative `Side::Buy` does not bind `a`, which `Side::Other(a)` binds",
    );
    refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    let Side::Other(a) | Side::Buy = s else {{ return 0; }};\n    return 1;\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "let_else_or",
        "3:26",
        "the alternative `Side::Buy` does not bind `a`, which `Side::Other(a)` binds",
    );
}

/// `if let` is an expression as `if` is: used as a value it needs an
/// `else`, and both arms one type.
#[test]
fn an_if_let_value_needs_two_arms_of_one_type() {
    refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    let n = if let Side::Other(raw) = s {{ len(raw) }};\n    return n;\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "no_else_value",
        "3:13",
        "`if let` arms have mismatched types: then=`Int`, else=`()`",
    );
    refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    let n = if let Side::Other(raw) = s {{ len(raw) }} else {{ \"none\" }};\n    return n;\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "mismatch",
        "3:13",
        "`if let` arms have mismatched types: then=`Int`, else=`String`",
    );
}

/// An `if let` with no `else` at the end of a body that returns a value
/// is named as the path that falls through, as an `if` with no `else`
/// is (F.48).
#[test]
fn an_if_let_without_else_ending_a_valued_body_is_named() {
    refused(
        &format!(
            "{SIDE}fn f(s: Side) -> Int {{\n    if let Side::Other(raw) = s {{ return len(raw); }}\n}}\nfn main() {{ println(f(Side::Buy)); }}\n"
        ),
        "fall",
        "2:4",
        "fn `f` returns `Int`, and its body can reach its end without a value: this `if let` has no `else`, so when \
         its pattern does not match the body ends after it",
    );
}

/// A pattern that matches every value needs no `else`: a plain `let` of
/// it builds, and an `else` on it is dead, which is a warning.
#[test]
fn a_pattern_that_always_matches() {
    let s = Scratch::new(
        "type Meters = enum { M(Int) };\nfn f(m: Meters, p: (Int, Int)) -> Int {\n    let Meters::M(n) = m;\n    let (a, b) = p else { return 0; };\n    return n + a + b;\n}\nfn main() { println(f(Meters::M(1), (2, 3))); }\n",
        "always",
    );
    let (ok, out) = s.check();
    assert!(
        ok && out.contains("main.hl:4:25: warning: `(a, b)` matches every `(Int, Int)`, so this `else` never runs: remove it"),
        "check: {out}"
    );
    assert_eq!(out.matches("warning:").count(), 1, "only the dead else: {out}");
    let (built, bout) = s.build();
    assert!(built, "build: {bout}");
}

/// What the check passes builds: every leaving `else`, the expression
/// form, a chain mixing plain and `if let` links.
#[test]
fn the_accepted_forms_build() {
    let s = Scratch::new(
        &format!(
            "{SIDE}type E = enum {{ Bad(String) }};\n\
             fn a(s: Side) -> Int {{ let Side::Other(r) = s else {{ return 0; }}; return len(r); }}\n\
             fn b(xs: [Side; 2]) -> Int {{ let mut t = 0; for x in xs {{ let Side::Other(r) = x else {{ continue; }}; t = t + len(r); }} return t; }}\n\
             fn c(xs: [Side; 2]) -> Int {{ let mut i = 0; while i < 2 {{ let Side::Other(r) = xs[i] else {{ break; }}; i = i + len(r); }} return i; }}\n\
             fn d(s: Side) -> Int fallible(E) {{ let Side::Other(r) = s else {{ fail E::Bad(\"buy\"); }}; return len(r); }}\n\
             fn e(s: Side) -> Int {{ let v = if let Side::Other(r) = s {{ len(r) }} else {{ 0 }}; return v; }}\n\
             fn g(n: Int, s: Side) -> Int {{ if n > 1 {{ 1 }} else if let Side::Other(r) = s {{ len(r) }} else {{ 0 }} }}\n\
             fn main() {{ println(a(Side::Buy) + b([Side::Buy, Side::Other(\"x\")]) + c([Side::Other(\"x\"), Side::Buy]) + (d(Side::Buy) or 0) + e(Side::Buy) + g(0, Side::Buy)); }}\n"
        ),
        "built",
    );
    let (ok, out) = s.check();
    assert!(ok && !out.contains("warning:"), "check: {out}");
    let (built, bout) = s.build();
    assert!(built, "build: {bout}");
}
