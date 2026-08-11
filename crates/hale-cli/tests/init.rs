//! `hale init` — project bootstrap. The scaffold must be the
//! canonical minimal project: fmt-clean, runnable, testable, and
//! verifiable out of the box; the command must be strictly
//! non-destructive on re-run and in partially-scaffolded
//! directories.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str], cwd: Option<&Path>) -> (String, i32) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let out = c.output().expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_init_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn init_scaffold_runs_tests_and_verifies() {
    let d = root("full");
    let (out, code) = hale(&["init", d.to_str().unwrap()], None);
    assert_eq!(code, 0, "{}", out);
    for f in ["hale.toml", "main.hl", "tests/main_test.hl", ".gitignore"] {
        assert!(d.join(f).exists(), "scaffold creates {}: {}", f, out);
    }

    // Canonical from birth: the generated files pass the fmt gate.
    let (out, code) = hale(&["fmt", "--check", d.to_str().unwrap()], None);
    assert_eq!(code, 0, "scaffold must be fmt-canonical: {}", out);

    // Runs...
    let (out, code) = hale(&["run", d.to_str().unwrap()], None);
    assert_eq!(code, 0, "{}", out);
    assert!(out.contains("Hello from Hale."), "{}", out);

    // ...its test passes (cross-seed import of the parent)...
    let (out, code) = hale(&["test", "."], Some(&d));
    assert_eq!(code, 0, "{}", out);
    assert!(out.contains("1 passed, 0 failed"), "{}", out);

    // ...and the discipline gate is clean.
    let (out, code) = hale(&["verify", "."], Some(&d));
    assert_eq!(code, 0, "verify must be clean: {}", out);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn init_is_non_destructive() {
    let d = root("keep");
    std::fs::create_dir_all(&d).unwrap();
    // A pre-existing main.hl must survive byte-for-byte.
    let mine = "fn main() { println(\"mine\"); }\n";
    std::fs::write(d.join("main.hl"), mine).unwrap();

    let (out, code) = hale(&["init", d.to_str().unwrap()], None);
    assert_eq!(code, 0, "{}", out);
    assert!(out.contains("kept"), "reports the kept file: {}", out);
    assert_eq!(
        std::fs::read_to_string(d.join("main.hl")).unwrap(),
        mine,
        "existing files are never touched"
    );
    assert!(d.join("hale.toml").exists(), "missing files are filled in");

    // Second full run: everything exists, nothing changes.
    let (out, code) = hale(&["init", d.to_str().unwrap()], None);
    assert_eq!(code, 0, "{}", out);
    assert!(
        out.contains("nothing to do"),
        "idempotent re-run: {}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}
