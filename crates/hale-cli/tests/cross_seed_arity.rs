//! GH #1028 — `hale check` holds a call into an imported seed to the
//! callee's signature, as it holds a same-seed call.
//!
//! An imported free fn is called through its alias, `lib::add3(..)`,
//! and the checker typed that path as unknown: the call's arity, its
//! argument types and the value it returned were all unchecked, and a
//! call with an argument too few passed `hale check` to fail in `hale
//! build` ("required param at position 2 not provided"). Found when two
//! fixtures kept calling a host fn with five arguments after it gained
//! a sixth. The path now resolves through the same import table codegen
//! uses, to the library's own signature.
//!
//! Each case is a throwaway two-seed tree: `lib` declares the callee,
//! `app` imports it as `lib` and calls it. The last case is the
//! control — correct calls, fallible and not, stay clean.

use std::path::{Path, PathBuf};
use std::process::Command;

const LIB: &str = r#"
fn add3(a: Int, b: Int, c: Int) -> Int { return a + b + c; }
fn digit(s: String) -> Int fallible(String) {
    if s == "1" { return 1; }
    fail "not a digit";
}
locus Acc {
    params { total: Int = 0; }
    fn add2(a: Int, b: Int) -> Int { self.total = self.total + a + b; return self.total; }
}
fn make() -> Acc { return Acc { }; }
"#;

fn tree(tag: &str, app: &str) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_cross_seed_arity_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&d);
    for (name, src) in [("lib/main.hl", LIB), ("app/main.hl", app)] {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, src).expect("write");
    }
    d
}

fn check(dir: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", "app"])
        .current_dir(dir)
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

fn refused(tag: &str, app: &str, message: &str) {
    let d = tree(tag, app);
    let (ok, out) = check(&d);
    let _ = std::fs::remove_dir_all(&d);
    assert!(!ok, "{tag}: `hale check` accepted it:\n{out}");
    assert!(out.contains(message), "{tag}: expected `{message}` in:\n{out}");
    assert!(out.contains("app/main.hl:"), "{tag}: the error is located in the caller's seed:\n{out}");
}

#[test]
fn too_few_arguments_to_an_imported_fn() {
    // The issue's reproducer.
    refused(
        "few",
        "import \"../lib\" as lib;\nfn main() { let x = lib::add3(1, 2); println(x); }\n",
        "fn `lib::add3` takes at least 3 arguments, got 2",
    );
}

#[test]
fn too_many_arguments_to_an_imported_fn() {
    refused(
        "many",
        "import \"../lib\" as lib;\nfn main() { let x = lib::add3(1, 2, 3, 4); println(x); }\n",
        "fn `lib::add3` takes at most 3 arguments, got 4",
    );
}

#[test]
fn an_imported_fns_argument_and_result_types_are_checked() {
    refused(
        "argty",
        "import \"../lib\" as lib;\nfn main() { let x = lib::add3(1, 2, \"three\"); println(x); }\n",
        "argument 2 type mismatch: expected `Int`, got `String`",
    );
    refused(
        "retty",
        "import \"../lib\" as lib;\nfn main() { let s: String = lib::add3(1, 2, 3); println(s); }\n",
        "let `s`: expected `String`, got `Int`",
    );
}

#[test]
fn an_imported_fallible_fn_keeps_its_arity_and_its_or() {
    refused(
        "fallible_many",
        "import \"../lib\" as lib;\nfn main() { let d = lib::digit(\"1\", \"2\") or 0; println(d); }\n",
        "fn `lib::digit` takes at most 1 argument, got 2",
    );
}

#[test]
fn a_method_on_a_handle_an_imported_factory_returned() {
    // `lib::Acc { }` already typed its handle; `lib::make()` did not,
    // so the method call on its result was unchecked too.
    refused(
        "factory_method",
        "import \"../lib\" as lib;\nfn main() { let a = lib::make(); let x = a.add2(1); println(x); }\n",
        "method `add2` takes at least 2 arguments, got 1",
    );
    refused(
        "literal_method",
        "import \"../lib\" as lib;\nfn main() { let a = lib::Acc { }; let x = a.add2(1, 2, 3); println(x); }\n",
        "method `add2` takes at most 2 arguments, got 3",
    );
}

#[test]
fn correct_cross_seed_calls_stay_clean() {
    let d = tree(
        "ok",
        "import \"../lib\" as lib;\nfn main() {\n    let x = lib::add3(1, 2, 3);\n    let d = lib::digit(\"1\") or 0;\n    let a = lib::make();\n    let y = a.add2(x, d);\n    println(x + y);\n}\n",
    );
    let (ok, out) = check(&d);
    let _ = std::fs::remove_dir_all(&d);
    assert!(ok, "correct calls were refused:\n{out}");
}
