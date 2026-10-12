//! The checker / parser items of the 2026-10-12 friction sweep, held at
//! the CLI: what `hale check` says, and that `hale build` agrees.
//!
//! - F.43: the framework vocabulary is contextual. `run`, `epoch`,
//!   `restart`, `capacity`, `bulk` and the rest name fields, params,
//!   locals and fns; a locus method named like a hook is still refused,
//!   with a message that says the name IS the hook.
//! - `or discard` on a value-bearing fallible is legal as a whole
//!   statement (the value is thrown away with the error) and refused
//!   where the value is used, with the default to write instead.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch seed of one file, unique to this process and `tag`.
fn seed(src: &str, tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_friction_sweep_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), src).unwrap();
    d
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(Path::new("/")).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// `hale check` and `hale build` of one source: (check ok, check output,
/// build ok, build output).
fn check_and_build(src: &str, tag: &str) -> (bool, String, bool, String) {
    let d = seed(src, tag);
    let file = d.join("main.hl").to_string_lossy().to_string();
    let (checked, check_out) = hale(&["check", &file]);
    let (built, build_out) = hale(&["build", &file, "-o", &d.join("bin").to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&d);
    (checked, check_out, built, build_out)
}

#[test]
fn the_framework_words_name_fields_params_locals_and_fns() {
    let src = "type Job { run: Int; epoch: Int; restart: Bool = false; }\n\
               fn run(capacity: Int) -> Int { let where = capacity + 1; return where; }\n\
               locus Worker { params { tier: Int = 1; bulk: Int = 2; } fn restart() -> Int { return self.tier + self.bulk; } }\n\
               fn main() { let j = Job { run: 1, epoch: 2 }; let w = Worker { }; println(run(j.run + j.epoch) + w.restart()); }\n";
    let (checked, out, built, bout) = check_and_build(src, "words");
    assert!(checked, "check: {out}");
    assert!(built, "build: {bout}");
}

#[test]
fn a_locus_method_named_like_a_hook_is_refused_by_check_and_build() {
    let src = "locus Worker {\n    fn run() { }\n}\nfn main() { }\n";
    let (checked, out, built, bout) = check_and_build(src, "hook");
    assert!(!checked && out.contains("a locus method cannot be named `run`: that name is the `run` lifecycle hook"), "check: {out}");
    assert!(out.contains("declare the hook as `run() { … }` (no `fn`)"), "the fix: {out}");
    assert!(!built && bout.contains("cannot be named `run`"), "build: {bout}");
}

#[test]
fn or_discard_drops_a_value_in_a_statement_and_is_refused_where_it_is_used() {
    let stmt = "type Oops { why: String = \"\"; }\n\
                fn parse(s: String) -> Int fallible(Oops) { if s == \"bad\" { fail Oops { why: s }; } return 7; }\n\
                fn main() { parse(\"bad\") or discard; parse(\"ok\") or discard; println(\"ok\"); }\n";
    let (checked, out, built, bout) = check_and_build(stmt, "discard_stmt");
    assert!(checked, "check: {out}");
    assert!(built, "build: {bout}");

    let used = "type Oops { why: String = \"\"; }\n\
                fn parse(s: String) -> Int fallible(Oops) { return 7; }\n\
                fn main() { let x = parse(\"1\") or discard; println(x); }\n";
    let (checked, out, built, _) = check_and_build(used, "discard_used");
    assert!(!checked, "a used value is refused: {out}");
    assert!(
        out.contains("`or discard` has no value to give where the result is used: the call succeeds with a `Int`"),
        "{out}"
    );
    assert!(out.contains("Substitute a default (`or <default>`)"), "the fix: {out}");
    assert!(!built, "the build refuses it alike");
}
