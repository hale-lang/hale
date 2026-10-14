//! GH #911 (B6): a module-nested `fn main` is refused by `check`, so
//! `check` and `build` agree about the entry point.
//!
//! `spec/semantics.md` § "Declarations inside `module { }`" has always
//! said a seed's entry point is its **top-level** `fn main`, and
//! codegen reads `program.items` and nowhere else. What neither layer
//! did was SAY so: a seed whose only `fn main` sat one brace deeper
//! passed `hale check` (exit 0, `ok: 1 file(s) typechecked`) and then
//! failed to build with codegen's spanless
//! `program has no `fn main()``. Every other check in this family
//! reaches INSIDE a module (GH #825); the entry point is the one that
//! must not, so the fix is a refusal at the declaration rather than a
//! promotion of it.
//!
//! The ruling (Riley, 2026-09-20, GH #911): the entry point is
//! top-level only, and `check` refuses the nested one with a located
//! message.
//!
//! The programs are plain escaped string literals: `hale-corpus`
//! harvests `r#"…"#` literals out of test files into the corpus-wide
//! properties, and these two are written to be refused.

use std::path::{Path, PathBuf};
use std::process::Command;

const MSG: &str = "the entry point must be top-level";

/// The issue's shape: the program's only `fn main` inside a module.
const NESTED: &str =
    "module inner {\n    fn main() {\n        println(\"hi\");\n    }\n}\n";

/// The control: the same two declarations, `main` at the top level.
const FLAT: &str = "module inner {\n\
                    \x20   fn greet() -> String { return \"hi\"; }\n\
                    }\n\
                    \n\
                    fn main() {\n\
                    \x20   println(greet());\n\
                    }\n";

fn seed(tag: &str, src: &str) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_entry_point_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    std::fs::write(d.join("main.hl"), src).expect("write");
    d
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// `check` refuses it, at the declaration's own span — line 2,
/// column 8 is the `main` in `    fn main() {`.
#[test]
fn check_refuses_a_module_nested_main_at_its_span() {
    let d = seed("check", NESTED);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(!ok, "check must fail:\n{out}");
    assert!(out.contains(MSG), "{out}");
    assert!(out.contains("module inner"), "it names the module:\n{out}");
    assert!(out.contains(":2:8:"), "the span is the declaration:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The machine-readable stream carries it: a gate reading `--json`
/// used to see an empty diagnostic list and exit 0.
#[test]
fn json_diagnostics_carry_it() {
    let d = seed("json", NESTED);
    let (ok, out) = hale(&["check", "--json", &d.to_string_lossy()]);
    assert!(!ok, "{out}");
    let line = out
        .lines()
        .find(|l| l.contains(MSG))
        .unwrap_or_else(|| panic!("no json row for the entry point:\n{out}"));
    assert!(line.contains("\"line\":2"), "{line}");
    assert!(line.contains("\"col\":8"), "{line}");
    assert!(line.contains("\"severity\":\"error\""), "{line}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The agreement this closes: `build` refused the program all along,
/// and now it refuses it with the same sentence and a position, the
/// checker having spoken first.
#[test]
fn build_refuses_it_with_the_same_located_message() {
    let d = seed("build", NESTED);
    let (ok, out) = hale(&["build", &d.to_string_lossy()]);
    assert!(!ok, "build must fail:\n{out}");
    assert!(out.contains(MSG), "{out}");
    assert!(
        !out.contains("program has no `fn main()`"),
        "the spanless codegen refusal is what this replaces:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The control, end to end: a module full of declarations beside a
/// top-level `fn main` checks, builds and runs.
#[test]
fn a_top_level_main_beside_a_module_still_runs() {
    let d = seed("flat", FLAT);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(ok, "check must pass:\n{out}");
    let (ok, out) = hale(&["run", &d.to_string_lossy()]);
    assert!(ok, "run must pass:\n{out}");
    assert!(out.contains("hi"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
