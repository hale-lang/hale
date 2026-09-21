//! GH #534 (DNA F.1 / F.10): the CLI path — `hale check`, then `hale
//! build`, then run — over a consumer of a library that matches its
//! own enum and serves its own perspective. The checker is where F.1
//! first surfaced (`match is not exhaustive ... __lib_..._Color`), so
//! the codegen-level regression in `hale-codegen` is not enough.

use std::path::PathBuf;
use std::process::Command;

fn consumer_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("hale-codegen/tests/fixtures/import-enum-persp-consumer");
    p
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(consumer_dir())
        .output()
        .expect("invoke hale");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn consumer_of_enum_and_perspective_library_checks_builds_and_runs() {
    let (ok, text) = hale(&["check", "."]);
    assert!(ok, "hale check must accept the consumer:\n{text}");
    assert!(
        !text.contains("not exhaustive") && !text.contains("unknown perspective"),
        "F.1/F.10 diagnostics must be gone:\n{text}"
    );
    let (ok, text) = hale(&["build", "."]);
    assert!(ok, "hale build must succeed:\n{text}");
    let bin = consumer_dir().join("import-enum-persp-consumer");
    let out = Command::new(&bin).output().expect("run consumer");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "consumer exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for needle in ["first=red", "green=true", "which=second", "after=second", "label=b", "picked=g"] {
        assert!(stdout.contains(needle), "missing {needle}: {stdout:?}");
    }
    // GH #885: an imported enum value prints under the DECLARATION's
    // own name. `println(lib::Color::Red)` rendered
    // `__lib_lib_lib_Color::Red` — the mangled symbol the import
    // renamed the declaration to, which appears nowhere in the
    // author's program. `spec/semantics.md` § "Rendering values as
    // text" specifies `Enum::Variant`, which is also what the same
    // enum renders as inside its own seed; the alias is not part of
    // it, because an alias is per-importing-file and the rendering
    // is a property of the value.
    //
    // Both codegen sites that spell the name are here: a no-payload
    // enum (the names-array lookup) and a payload one (the
    // per-variant switch).
    for needle in ["printed=Color::Red", "payload=Reading::Value(7)"] {
        assert!(stdout.contains(needle), "missing {needle}: {stdout:?}");
    }
    assert!(
        !stdout.contains("__lib_"),
        "a mangled name reached the program's output: {stdout:?}"
    );
}
