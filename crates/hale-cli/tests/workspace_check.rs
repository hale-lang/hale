//! `hale check --workspace` / `hale verify --workspace`.
//!
//! `check` operates on ONE seed and does not recurse — correctly, a
//! directory is one compilation unit. The consequence is that a
//! repository with many seeds needs something to enumerate them, and
//! until now that was a shell loop each project wrote itself. A
//! library or main-locus claim was enforced only if somebody
//! remembered to point `check` at that directory.
//!
//! This command does NOT connect seeds. Each stays its own closed
//! world and gets its own check; composing models across binaries is
//! a different feature with different semantics.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_ws_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn seed(root: &Path, name: &str, src: &str) {
    let d = root.join(name);
    std::fs::create_dir_all(&d).expect("mkdir seed");
    std::fs::write(d.join("main.hl"), src).expect("write seed");
}

fn hale(args: &[&std::ffi::OsStr]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

const OK_SRC: &str = r#"
    locus Q { params { n: Int = 0; } fn go() -> Int { return self.n; } }
    main locus App { params { q: Q = Q { }; } }
    fn main() { App { }; }
"#;

const BAD_SRC: &str = r#"
    locus Bad { params { n: Int = 0; } fn go() -> Int { return self.nope; } }
    main locus App { params { b: Bad = Bad { }; } }
    fn main() { App { }; }
"#;

/// The core property: EVERY seed runs, and one failure does not hide
/// the others. A runner that stopped at the first failure would
/// report a subset of the truth, which is the shape of thing this
/// command exists to eliminate.
#[test]
fn every_seed_runs_and_a_failure_does_not_short_circuit() {
    let r = root("all");
    // Alphabetically first so a short-circuiting runner would never
    // reach the two after it.
    seed(&r, "a-bad", BAD_SRC);
    seed(&r, "b-ok", OK_SRC);
    seed(&r, "c-ok", OK_SRC);

    let (out, code) =
        hale(&["check".as_ref(), "--workspace".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_ne!(code, 0, "a failing seed must fail the run:\n{}", out);
    for s in ["a-bad", "b-ok", "c-ok"] {
        assert!(
            out.contains(s),
            "seed `{}` must have been visited — a failure earlier in \
             the walk must not skip it:\n{}",
            s,
            out
        );
    }
    assert!(
        out.contains("1 of 3 seed(s) failed"),
        "the summary must say how many of how many:\n{}",
        out
    );
}

#[test]
fn a_clean_workspace_passes_and_counts_its_seeds() {
    let r = root("clean");
    seed(&r, "one", OK_SRC);
    seed(&r, "two", OK_SRC);

    let (out, code) =
        hale(&["check".as_ref(), "--workspace".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(code, 0, "no seed fails:\n{}", out);
    assert!(out.contains("2 seed(s) checked"), "got:\n{}", out);
}

/// A seed you do not own is not yours to gate. `vendor` and
/// dot-directories match `hale fmt`'s walk; `target` is build output.
#[test]
fn vendored_and_build_directories_are_not_seeds() {
    let r = root("skip");
    seed(&r, "mine", OK_SRC);
    seed(&r, "vendor/theirs", BAD_SRC);
    seed(&r, "target/generated", BAD_SRC);
    seed(&r, ".hidden/cache", BAD_SRC);

    let (out, code) =
        hale(&["check".as_ref(), "--workspace".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(
        code, 0,
        "only `mine` is owned; the broken ones are skipped:\n{}",
        out
    );
    assert!(out.contains("1 seed(s) checked"), "got:\n{}", out);
}

/// A nested seed is still a seed — `check` not recursing is exactly
/// why this command exists.
#[test]
fn nested_seeds_are_all_found() {
    let r = root("nested");
    seed(&r, "top", OK_SRC);
    seed(&r, "top/inner", OK_SRC);
    seed(&r, "top/inner/deeper", OK_SRC);

    let (out, code) =
        hale(&["check".as_ref(), "--workspace".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(code, 0, "{}", out);
    assert!(out.contains("3 seed(s) checked"), "got:\n{}", out);
}

/// Per-seed artifacts and one shared baseline are incompatible by
/// construction: N seeds are N models. Taking the last silently would
/// be the same fail-open the artifact gates were fixed for.
#[test]
fn per_seed_artifact_flags_are_rejected_with_workspace() {
    let r = root("artifact");
    seed(&r, "one", OK_SRC);

    for flag in ["--dump-topology", "--check-topology-shape=/tmp/x.json"] {
        let (out, code) = hale(&[
            "check".as_ref(),
            "--workspace".as_ref(),
            r.as_os_str(),
            flag.as_ref(),
        ]);
        assert_eq!(
            code, 2,
            "`{}` with --workspace must be a usage error:\n{}",
            flag, out
        );
        assert!(
            out.contains("per-seed"),
            "and say why:\n{}",
            out
        );
    }
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn a_root_with_no_seeds_is_a_usage_error() {
    let r = root("empty");
    std::fs::create_dir_all(&r).expect("mkdir");
    let (out, code) =
        hale(&["check".as_ref(), "--workspace".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "nothing to check is a usage error:\n{}", out);
}

/// `verify` gates advisories too, and `--workspace` composes with it
/// rather than being a `check`-only flag.
#[test]
fn verify_workspace_applies_the_stricter_gate() {
    let r = root("verify");
    seed(&r, "one", OK_SRC);
    let (out, code) =
        hale(&["verify".as_ref(), "--workspace".as_ref(), r.as_os_str()]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 0, "a clean seed passes verify too:\n{}", out);
}
