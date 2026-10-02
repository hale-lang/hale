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
    assert!(
        !ok,
        "`[ffi] link` cannot be satisfied on wasm32 and must be an \
         error, not a silent drop:\n{}",
        out
    );
    // T4 (F.40 P3): the refusal is the `LinkLibrary` cell's, located at
    // the package manifest's `link` line, and `hale check` gives the same
    // record for the same target; the host links `m`.
    let manifest = root.join("lib/glue/hale.toml");
    let want = format!(
        "{}:5:1: error: `[ffi] link = [\"m\"]` cannot be satisfied on wasm32 — there are no system dynamic \
         libraries to link against. Provide the code as `[ffi] csrc` so it can be compiled into the module, \
         or gate the dependency out of the wasm build.",
        manifest.display()
    );
    assert!(out.lines().any(|l| l == want), "build:\n{out}");
    let (code, check) = hale(&["check", root.join("app/main.hl").to_str().unwrap(), "--target", "wasm32"]);
    assert_eq!(code, 1, "{check}");
    assert!(check.lines().any(|l| l == want), "check:\n{check}");
    let (code, check) = hale(&["check", root.join("app/main.hl").to_str().unwrap()]);
    assert_eq!(code, 0, "the host links it:\n{check}");
    let _ = std::fs::remove_dir_all(&root);
}

/// `--link` is the same input as a manifest's `[ffi] link`: `hale check`
/// takes it and refuses it under wasm32 as the build does, named as the
/// flag (T4).
#[test]
fn a_link_flag_is_refused_on_wasm_by_check_and_build_alike() {
    let root = workspace("linkflag", GOOD_C, "");
    let main = root.join("app/main.hl");
    let want = "error: --link m: `[ffi] link = [\"m\"]` cannot be satisfied on wasm32";
    let (code, check) = hale(&["check", main.to_str().unwrap(), "--target", "wasm32", "--link", "m"]);
    assert_eq!(code, 1, "{check}");
    assert!(check.lines().any(|l| l.starts_with(want)), "check:\n{check}");
    let (code, build) = hale(&["build", main.to_str().unwrap(), "--target", "wasm32", "--link", "m"]);
    assert_ne!(code, 0, "{build}");
    assert!(build.lines().any(|l| l.starts_with(want)), "build:\n{build}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A capability refusal is not a toolchain failure (design §1.5): with
/// no clang and no wasm-ld on PATH, the build of the `link = ["m"]` app
/// reports the `[ffi] link` refusal, not "is clang installed?" — the
/// cell is asked before any tool is looked up.
#[test]
fn the_link_refusal_comes_before_any_tool_is_probed() {
    let root = workspace("nopath", GOOD_C, "link = [\"m\"]\n");
    let empty = root.join("empty-path");
    std::fs::create_dir_all(&empty).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["build", root.join("app/main.hl").to_str().unwrap(), "--target", "wasm32"])
        .env("PATH", &empty)
        .output()
        .expect("run hale build");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("cannot be satisfied on wasm32"), "{text}");
    assert!(!text.contains("is clang installed"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}

fn hale(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).output().expect("run hale");
    (
        out.status.code().unwrap_or(-1),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}
