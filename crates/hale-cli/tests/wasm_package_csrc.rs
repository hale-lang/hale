//! wasm32: a package's `[ffi] csrc` must actually be compiled (#213).
//!
//! `link_wasm` was called without `options`, so `csrc_files` and
//! `link_libs` never reached the wasm path. Every `@ffi("c")` symbol a
//! package defined in C surfaced as an undefined `env` import, which
//! `--allow-undefined` swallowed, and the generated JS loader stubbed
//! unknown imports with `() => 0`. The build reported success and
//! every call returned 0 forever — the worst available failure mode,
//! because nothing anywhere said anything.
//!
//! ## How these tests discriminate
//!
//! Before the fix the build SUCCEEDED, so "it builds" proves nothing.
//! The discriminator is that a *broken* C source must now break the
//! build: if csrc is compiled, a syntax error in it is a compile
//! error; if csrc is ignored, the build sails past. That is a
//! one-bit signal requiring no wasm parsing.

use std::path::PathBuf;
use std::process::Command;

/// A package with an `[ffi] csrc`, imported by an app. `[ffi]` is read
/// from imported PACKAGES (paths resolve against the lib dir), not
/// from the entry program's own manifest.
fn workspace(tag: &str, c_body: &str, link_line: &str) -> PathBuf {
    let root = std::env::temp_dir()
        .join(format!("hale-w213-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("app")).expect("mkdir app");
    std::fs::create_dir_all(root.join("lib/glue")).expect("mkdir lib");
    std::fs::write(root.join("hale.toml"), "name = \"w213\"\n").unwrap();
    std::fs::write(
        root.join("lib/glue/hale.toml"),
        format!("name = \"glue\"\n\n[ffi]\ncsrc = [\"glue.c\"]\n{}", link_line),
    )
    .unwrap();
    std::fs::write(root.join("lib/glue/glue.c"), c_body).unwrap();
    std::fs::write(
        root.join("lib/glue/g.hl"),
        "@ffi(\"c\")\nfn tsa_answer(n: Int) -> Int;\n\n\
         fn ask(n: Int) -> Int { return tsa_answer(n); }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/main.hl"),
        "import \"lib/glue\" as glue;\n\n\
         @export\nfn go(n: Int) -> Int { return glue::ask(n); }\n\n\
         fn main() { println(glue::ask(7)); }\n",
    )
    .unwrap();
    root
}

fn build_wasm(root: &PathBuf) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(root.join("app/main.hl"))
        .arg("--target")
        .arg("wasm32")
        .output()
        .expect("run hale build");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// Hale's `Int` is i64, so the C must be `long long`. A plain `int`
/// links but traps at call time with a wasm signature mismatch —
/// which is itself an improvement over the old silent stub, and is
/// pinned separately below.
const GOOD_C: &str = "long long tsa_answer(long long n) { return n * 6; }\n";

#[test]
fn a_package_csrc_builds_for_wasm() {
    let root = workspace("good", GOOD_C, "");
    let (ok, out) = build_wasm(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(ok, "a well-formed package csrc must build for wasm32:\n{}", out);
}

/// THE discriminator. Before the fix this passed, because the source
/// was never handed to a compiler.
#[test]
fn a_broken_package_csrc_fails_the_wasm_build() {
    let root = workspace("broken", "this is not C at all;\n", "");
    let (ok, out) = build_wasm(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        !ok,
        "a package csrc with a syntax error must fail the wasm build. \
         If this passes, csrc is being ignored again and every \
         @ffi(\"c\") symbol it defines is silently a stub:\n{}",
        out
    );
    assert!(
        out.contains("csrc") || out.contains("freestanding"),
        "the diagnostic should say which csrc failed and that the wasm \
         build is freestanding — a bare clang error leaves the reader \
         guessing why a source that builds natively does not build \
         here:\n{}",
        out
    );
}

/// `link = [...]` names a system dynamic library. wasm has neither a
/// dynamic linker nor system libraries, so this must be refused rather
/// than dropped — dropping it silently is the same class of bug this
/// issue is about, one level up.
#[test]
fn a_system_link_dependency_is_refused_on_wasm() {
    let root = workspace("link", GOOD_C, "link = [\"m\"]\n");
    let (ok, out) = build_wasm(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        !ok,
        "`[ffi] link` cannot be satisfied on wasm32 and must be an \
         error, not a silent drop:\n{}",
        out
    );
    assert!(
        out.contains("no system dynamic libraries")
            || out.contains("cannot be satisfied"),
        "the diagnostic must say why, and point at csrc as the \
         alternative:\n{}",
        out
    );
}
