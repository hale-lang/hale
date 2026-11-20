//! A seed's entry point is its `fn main`, or the `main locus` that one
//! instantiates (`spec/semantics.md` § "The entry locus").
//!
//! The programs are plain escaped string literals: `hale-corpus`
//! harvests `r#"…"#` literals out of test files into the corpus-wide
//! properties, and these are written to be refused.

use std::path::{Path, PathBuf};
use std::process::Command;

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

// ---- a `main locus` with no `fn main` ---------------------------------
//
// `spec/semantics.md` § "The entry locus": a seed's entry point is its
// top-level `fn main`, or the `main locus` that one instantiates. A seed
// that declares its own top-level `main locus` and no `fn main` has
// nothing that starts the locus. The build said so with codegen's
// spanless `program has no `fn main()``, then (2026-10-14) at the `main
// locus`'s name with the line to add, while `hale check` accepted the
// seed; since GH #2108 the check refuses it too, at the same name, in the
// same words, so every verb gives one answer.

const STARTS: &str = "a `main locus` is started by `fn main`, and this seed has none: add `fn main() { App { }; }`";

/// The shape: a `main locus` and nothing that starts it.
const MAIN_LOCUS_ONLY: &str = "main locus App {\n    run {\n        println(\"hi\");\n    }\n}\n";

/// `check`, `build` and `run` refuse it at the locus's name — line 1,
/// column 12 is the `App` in `main locus App {` — not with the spanless
/// codegen line, and the check and the build say the same thing.
#[test]
fn check_build_and_run_refuse_a_main_locus_with_no_fn_main_at_its_name() {
    let d = seed("main_locus_build", MAIN_LOCUS_ONLY);
    for verb in ["check", "build", "run"] {
        let (ok, out) = hale(&[verb, &d.to_string_lossy()]);
        assert!(!ok, "{verb} must fail:\n{out}");
        assert!(out.contains(STARTS), "{verb} names the line to add:\n{out}");
        assert!(out.contains(":1:12:"), "{verb}: the span is the locus's name:\n{out}");
        assert!(
            !out.contains("program has no `fn main()`"),
            "{verb}: the spanless codegen refusal is what this replaces:\n{out}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// An application another seed imports is started by the importer's
/// `fn main`: the importer checks and runs, and the imported seed, checked
/// alone, is the program it is and is refused.
#[test]
fn an_imported_main_locus_is_started_by_the_importer() {
    let d = seed(
        "main_locus_imported",
        "import \"./app\" as app;\n\nfn main() {\n    app::App { };\n}\n",
    );
    std::fs::create_dir_all(d.join("app")).expect("mkdir");
    std::fs::write(d.join("app/app.hl"), MAIN_LOCUS_ONLY).expect("write");
    for verb in ["check", "run"] {
        let (ok, out) = hale(&[verb, &d.to_string_lossy()]);
        assert!(ok, "{verb} of the importer must pass:\n{out}");
    }
    let (ok, out) = hale(&["check", &d.join("app").to_string_lossy()]);
    assert!(!ok && out.contains(STARTS), "the application alone is refused:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The control: the line the message names makes it a program.
#[test]
fn the_fn_main_the_message_names_starts_the_main_locus() {
    let src = format!("{MAIN_LOCUS_ONLY}\nfn main() {{ App {{ }}; }}\n");
    let d = seed("main_locus_started", &src);
    let (ok, out) = hale(&["run", &d.to_string_lossy()]);
    assert!(ok, "run must pass:\n{out}");
    assert!(out.contains("hi"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A program with neither keeps a plain line that still says what is
/// missing.
#[test]
fn a_program_with_no_main_at_all_says_so() {
    let d = seed("no_main", "fn helper() -> Int {\n    return 1;\n}\n");
    let (ok, out) = hale(&["build", &d.to_string_lossy()]);
    assert!(!ok, "build must fail:\n{out}");
    assert!(out.contains("program has no `fn main()`: a native program starts at its top-level `fn main`"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
