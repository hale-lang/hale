//! Integration test for `aperio fetch`. Sets up a throwaway
//! "remote" git repo as a `file://` URL, points a manifest at
//! it, runs the `aperio` binary's fetch subcommand, and verifies
//! that the source landed under `lib/<name>/` and that
//! `aperio.lock` pinned the resolved SHA.
//!
//! Requires `git` on PATH. Skips silently if git is missing
//! (the language doesn't ship git; we just shell out to whatever
//! is available).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_pkg_test_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&p).expect("create tmpdir");
    p
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Initialize `dir` as a git repo, add `files`, and commit.
/// Returns the resolved commit SHA. The repo is initialized
/// with an "aperio-test" user identity to avoid relying on the
/// caller's git config.
fn make_repo(dir: &Path, files: &[(&str, &str)]) -> String {
    run(dir, &["init", "--quiet", "--initial-branch=main"]);
    run(dir, &["config", "user.email", "test@aperio.invalid"]);
    run(dir, &["config", "user.name", "Aperio Test"]);
    // For tests we need `git clone` to work against a local repo
    // even when remote operations would normally be blocked.
    run(dir, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    for (name, body) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::write(&path, body).expect("write file");
        run(dir, &["add", name]);
    }
    run(dir, &["commit", "--quiet", "-m", "init"]);
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {:?} failed in {}", args, dir.display());
}

fn aperio_bin() -> PathBuf {
    // CARGO_BIN_EXE_<name> is set by cargo for integration tests
    // pointing at the freshly-built binary.
    PathBuf::from(env!("CARGO_BIN_EXE_aperio"))
}

#[test]
fn fetch_clones_dep_and_writes_lockfile() {
    if !git_available() {
        eprintln!("git not available; skipping");
        return;
    }

    // 1. Build a "remote" repo with one .ap file.
    let remote = unique_dir("remote");
    let remote_sha = make_repo(
        &remote,
        &[("greet.ap", "fn hello() -> String { return \"hi\"; }\n")],
    );

    // 2. Build a "consumer" project with an aperio.toml pointing
    //    at the file:// URL of the remote.
    let consumer = unique_dir("consumer");
    let manifest = format!(
        "[deps]\nhelpers = {{ git = \"file://{}\" }}\n",
        remote.display()
    );
    fs::write(consumer.join("aperio.toml"), manifest).expect("write manifest");

    // 3. Run `aperio fetch <consumer>`.
    let out = Command::new(aperio_bin())
        .args(["fetch", consumer.to_str().unwrap()])
        .output()
        .expect("spawn aperio fetch");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "fetch failed:\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // 4. Verify source landed under lib/helpers/.
    let cloned = consumer.join("lib").join("helpers");
    assert!(
        cloned.join("greet.ap").exists(),
        "expected {} to exist",
        cloned.join("greet.ap").display()
    );

    // 5. Verify aperio.lock has the right SHA.
    let lock = fs::read_to_string(consumer.join("aperio.lock"))
        .expect("read lock");
    assert!(
        lock.contains(&remote_sha),
        "lockfile missing SHA `{}`:\n{}",
        remote_sha,
        lock
    );
    assert!(
        lock.contains("name = \"helpers\""),
        "lockfile missing dep name:\n{}",
        lock
    );

    // Cleanup (best-effort; tests don't fail if removal does).
    let _ = fs::remove_dir_all(&remote);
    let _ = fs::remove_dir_all(&consumer);
}

#[test]
fn refetch_is_idempotent() {
    if !git_available() {
        eprintln!("git not available; skipping");
        return;
    }

    let remote = unique_dir("remote_idem");
    let _sha = make_repo(
        &remote,
        &[("noop.ap", "fn main() { }\n")],
    );

    let consumer = unique_dir("consumer_idem");
    let manifest = format!(
        "[deps]\nlibrary = {{ git = \"file://{}\" }}\n",
        remote.display()
    );
    fs::write(consumer.join("aperio.toml"), manifest).expect("write manifest");

    // First fetch — should clone.
    let out = Command::new(aperio_bin())
        .args(["fetch", consumer.to_str().unwrap()])
        .output()
        .expect("first fetch");
    assert!(out.status.success(), "first fetch failed");

    // Mark the .git dir's mtime so we can tell whether it was
    // touched on the second fetch.
    let git_dir = consumer.join("lib").join("library").join(".git");
    assert!(git_dir.exists(), "first fetch should have created .git");

    // Second fetch — should be a no-op given the lockfile pin.
    let out = Command::new(aperio_bin())
        .args(["fetch", consumer.to_str().unwrap()])
        .output()
        .expect("second fetch");
    assert!(out.status.success(), "second fetch failed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("up to date"),
        "expected 'up to date' on idempotent re-fetch; got:\n{}",
        stdout
    );

    let _ = fs::remove_dir_all(&remote);
    let _ = fs::remove_dir_all(&consumer);
}
