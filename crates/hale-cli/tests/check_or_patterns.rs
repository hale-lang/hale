//! Or-patterns in `match` arms (GH #1995 E2) and the string-match
//! advisory (GH #1995 S5), through the `hale` binary: what `hale check`
//! refuses `hale build` refuses at the same message, never in codegen,
//! and what the check passes builds. What an or-pattern computes is
//! `tests/hale/or_patterns_test.hl`'s.

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
        let d = std::env::temp_dir().join(format!("hale_check_or_patterns_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("main.hl"), src).unwrap();
        Scratch(d)
    }
    fn file(&self) -> String {
        self.0.join("main.hl").to_string_lossy().to_string()
    }
    fn check(&self, flags: &[&str]) -> (bool, String) {
        let file = self.file();
        let mut args = vec!["check"];
        args.extend_from_slice(flags);
        args.push(&file);
        hale(&args)
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

/// The check refuses `src` at `main.hl:<at>` with `kind: head`, and the
/// build stops at the same message, never reaching codegen.
fn refused(src: &str, tag: &str, at: &str, kind: &str, head: &str) {
    let s = Scratch::new(src, tag);
    let (ok, out) = s.check(&[]);
    assert!(!ok && out.contains(&format!("main.hl:{at}: {kind}: {head}")), "check: {out}");
    let (built, bout) = s.build();
    assert!(!built && bout.contains(head), "build: {bout}");
    assert!(!bout.contains("codegen error"), "the build stops at the check: {bout}");
}

const ENUM: &str = "type E = enum { A(String), B(Int), C(String), D };\n";

/// Every alternative binds the same names: one an alternative lacks is
/// refused at that alternative, one only a later alternative binds at its
/// binding, each naming the name and the alternative.
#[test]
fn an_alternative_binding_other_names_is_refused() {
    refused(
        &format!("{ENUM}fn f(e: E) -> String {{\n    return match e {{\n        E::A(p) | E::C(q) -> \"x\",\n        _ -> \"y\",\n    }};\n}}\nfn main() {{ println(f(E::D)); }}\n"),
        "lacks",
        "4:19",
        "type error",
        "the alternative `E::C(q)` does not bind `p`, which `E::A(p)` binds: every alternative of a match arm binds the same names",
    );
    refused(
        &format!("{ENUM}fn f(e: E) -> String {{\n    return match e {{\n        E::D | E::C(p) -> \"x\",\n        _ -> \"y\",\n    }};\n}}\nfn main() {{ println(f(E::D)); }}\n"),
        "extra",
        "4:21",
        "type error",
        "the alternative `E::C(p)` binds `p`, which `E::D` does not",
    );
}

/// Every alternative binds a name at one type.
#[test]
fn an_alternative_binding_a_name_at_another_type_is_refused() {
    refused(
        &format!("{ENUM}fn f(e: E) -> String {{\n    return match e {{\n        E::A(p) | E::B(p) -> \"x\",\n        _ -> \"y\",\n    }};\n}}\nfn main() {{ println(f(E::D)); }}\n"),
        "types",
        "4:24",
        "type error",
        "`p` is of type `Int` in the alternative `E::B(p)`, and of type `String` in `E::A(p)`: every alternative of a match arm binds each name at one type",
    );
    // A tuple's parts, the binding at another place in each alternative.
    refused(
        "fn f(p: (Int, String)) -> Int {\n    return match p {\n        (x, \"a\") | (0, x) -> 1,\n        _ -> 0,\n    };\n}\nfn main() { println(f((1, \"a\"))); }\n",
        "tuple_types",
        "3:24",
        "type error",
        "`x` is of type `String` in the alternative `(0, x)`, and of type `Int` in `(x, \"a\")`",
    );
}

/// `|` is a whole arm's: one inside a variant's values or a tuple is a
/// parse error where it is written, from the check and the build alike.
#[test]
fn a_nested_alternative_is_refused_where_it_is_written() {
    let head = "`|` separates the alternatives of a whole match arm, and is not written inside a tuple or a variant's values";
    refused(
        "type E = enum { A(Int), B };\nfn f(e: E) -> Int {\n    return match e {\n        E::A(1 | 2) -> 1,\n        _ -> 0,\n    };\n}\nfn main() { println(f(E::B)); }\n",
        "nested_variant",
        "4:16",
        "parse error",
        head,
    );
    refused(
        "fn f(p: (Int, Int)) -> Int {\n    return match p {\n        (1 | 2, x) -> x,\n        _ -> 0,\n    };\n}\nfn main() { println(f((1, 2))); }\n",
        "nested_tuple",
        "3:12",
        "parse error",
        head,
    );
}

/// F.18 counts each alternative: an enum covered only through
/// alternatives is exhaustive, and one alternative short is not.
#[test]
fn exhaustiveness_counts_each_alternative() {
    let s = Scratch::new(
        &format!("{ENUM}fn f(e: E) -> Int {{\n    return match e {{\n        E::A(_) | E::C(_) -> 1,\n        E::B(n) -> n,\n        E::D -> 0,\n    }};\n}}\nfn g(b: Bool) -> Int {{\n    return match b {{\n        true | false -> 1,\n    }};\n}}\nfn main() {{ println(f(E::D) + g(true)); }}\n"),
        "exhaustive",
    );
    let (ok, out) = s.check(&[]);
    assert!(ok, "check: {out}");
    let (built, bout) = s.build();
    assert!(built, "build: {bout}");
    refused(
        &format!("{ENUM}fn f(e: E) -> Int {{\n    return match e {{\n        E::A(_) | E::C(_) -> 1,\n        E::B(n) -> n,\n    }};\n}}\nfn main() {{ println(f(E::D)); }}\n"),
        "short",
        "3:12",
        "type error",
        "match is not exhaustive; add a `_` arm or cover all cases of `E`",
    );
}

/// An alternative an earlier arm or alternative already covers is never
/// tried: a warning, at the alternative, and the program still builds.
#[test]
fn a_redundant_alternative_is_warned() {
    let s = Scratch::new(
        "fn f(n: Int) -> Int {\n    return match n {\n        1 | 2 -> 1,\n        3 | 2 -> 2,\n        _ -> 0,\n    };\n}\nfn main() { println(f(2)); }\n",
        "redundant",
    );
    let (ok, out) = s.check(&[]);
    assert!(
        ok && out.contains(
            "main.hl:4:13: warning: the alternative `2` is never tried: `2`, earlier in this match, already matches every value it does"
        ),
        "check: {out}"
    );
    assert_eq!(out.matches("warning:").count(), 1, "only the redundant one: {out}");
    let (built, bout) = s.build();
    assert!(built, "build: {bout}");
}

const LADDER: &str = "type Action = enum { Create, Update, Delete, Archive };\nfn action_of(s: String) -> Action {\n    if s == \"update\" { return Action::Update; }\n    if s == \"delete\" { return Action::Delete; }\n    if s == \"archive\" { return Action::Archive; }\n    return Action::Create;\n}\nfn tier(v: String) -> Int {\n    if v == \"a\" {\n        return 1;\n    } else if v == \"b\" || v == \"c\" {\n        return 2;\n    } else if \"d\" == v {\n        return 3;\n    } else {\n        return 0;\n    }\n}\nfn two(v: String) -> Int {\n    if v == \"a\" { return 1; }\n    if v == \"b\" { return 2; }\n    return 0;\n}\nfn interleaved(v: String, w: String) -> Int {\n    if v == \"a\" { return 1; }\n    if w == \"b\" { return 2; }\n    if v == \"c\" { return 3; }\n    return 0;\n}\nfn matched(v: String) -> Int {\n    return match v { \"a\" -> 1, \"b\" | \"c\" -> 2, \"d\" -> 3, _ -> 0 };\n}\nfn main() {\n    let _ = action_of(\"x\");\n    println(tier(\"a\") + two(\"a\") + interleaved(\"a\", \"b\") + matched(\"a\"));\n}\n";

/// GH #1995 S5: three or more `if` tests in a row comparing one String
/// with literals, as sibling statements or `else if` links, are named
/// with the `match` they read as, only when asked for
/// (`--warn-string-match`, on `hale check` or `hale verify`: verify fails
/// on any finding, so it must not report a suggestion unasked). Two
/// tests, tests on different places, and a `match` are not.
#[test]
fn a_string_if_ladder_is_advised_when_asked_for() {
    let s = Scratch::new(LADDER, "ladder");
    let (ok, out) = s.check(&[]);
    assert!(ok && !out.contains("warning:"), "off by default: {out}");
    let (ok, out) = s.check(&["--warn-string-match"]);
    assert!(ok, "check: {out}");
    assert!(
        out.contains("main.hl:3:8: warning: `s` is compared with a string literal by 3 `if` tests in a row; a `match`"),
        "sibling ifs: {out}"
    );
    assert!(
        out.contains("main.hl:9:8: warning: `v` is compared with a string literal by 3 `if` tests in a row"),
        "an else-if chain: {out}"
    );
    assert_eq!(out.matches("warning:").count(), 2, "not two tests, other places or a match: {out}");
    let (_, out) = hale(&["verify", &s.file()]);
    assert!(!out.contains("is compared with a string literal"), "verify does not report it unasked: {out}");
    let (ok, out) = hale(&["verify", "--warn-string-match", &s.file()]);
    assert!(!ok && out.contains("is compared with a string literal by 3 `if` tests"), "verify, asked: {out}");
}

/// The advisory reads the program's own source: the stdlib's, and stdlib
/// text an expansion appends to the program (the streamed body readers,
/// whose SSE field dispatch is such a ladder), is never reported.
#[test]
fn the_string_match_advisory_never_reports_the_stdlib() {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.extend(["tests", "fixtures", "run-loop-reclaim", "reclaimed.hl"]);
    let (_, out) = hale(&["check", "--warn-string-match", &p.to_string_lossy()]);
    assert!(!out.contains("is compared with a string literal"), "a stdlib ladder reported: {out}");
}
