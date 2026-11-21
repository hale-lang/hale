//! GH #2134 (F.63): `hale build` reports a `@vectorize` loop the
//! optimizer did not vectorize at the loop, in the program's file, line
//! and column; and a build that does not check the assertion (a dev
//! build, at O1) says so instead of passing it in silence.

use std::path::PathBuf;
use std::process::Command;

/// A loop whose trip count depends on what it computes: no target's
/// vectorizer takes it, and the check cannot see that.
const UNTIL: &str = "@vectorize
fn until(xs: [Int; 1024], limit: Int) -> Int {
    let mut s = 0;
    let mut i = 0;
    while s < limit {
        s = s + xs[i % 1024];
        i = i + 1;
    }
    return i;
}

fn main() {
    let xs: [Int; 1024] = [3; 1024];
    println(until(xs, 5000));
}
";

fn seed(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hale_vectorize_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let file = dir.join("main.hl");
    std::fs::write(&file, UNTIL).expect("write");
    file
}

fn build(file: &PathBuf, dev: bool) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hale"));
    cmd.arg("build").arg(file).arg("-o").arg(file.with_extension("bin"));
    if dev {
        cmd.arg("--dev");
    }
    let out = cmd.output().expect("hale build");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn a_loop_that_does_not_vectorize_fails_the_build_at_the_loop() {
    let file = seed("refused");
    let (ok, err) = build(&file, false);
    let _ = std::fs::remove_dir_all(file.parent().unwrap());
    assert!(!ok, "{err}");
    assert!(err.contains("main.hl:5:5"), "at the `while`: {err}");
    assert!(err.contains("`@vectorize` on `until`: this loop does not vectorize on "), "{err}");
    assert!(err.contains("its trip count is not known when the loop starts"), "{err}");
}

#[test]
fn a_dev_build_says_it_did_not_check() {
    let file = seed("dev");
    let (ok, err) = build(&file, true);
    let _ = std::fs::remove_dir_all(file.parent().unwrap());
    assert!(ok, "{err}");
    assert!(err.contains("note: `@vectorize` was not checked: a dev build"), "{err}");
}
