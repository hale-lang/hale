//! C9 — `std::io::fs::{rename, unlink, mktemp}`.
//!
//! Each surface returns `fallible(IoError)`. Single end-to-end
//! integration test that exercises all three in one program:
//!
//! 1. `mktemp` a path with a known prefix + suffix.
//! 2. `write_file` content to that path.
//! 3. `rename` to a sibling path in the same dir.
//! 4. `read_file` the renamed path and assert content roundtrips.
//! 5. `unlink` the renamed path.
//! 6. Assert `file_exists` is false after unlink.
//!
//! Resolves pond/logfmt FRICTION "no-rename-no-unlink-in-fs-stdlib"
//! and pond/agent/sandbox FRICTION "missing mktemp primitive".

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, src: &str) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_test_fs_c9_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status,
    )
}

#[test]
fn mktemp_write_rename_read_unlink_roundtrip() {
    // The whole pond-rotation lifecycle in one program:
    //   - mktemp gives us an existing-but-empty path with a known
    //     prefix and suffix.
    //   - We overwrite it with known content via write_file.
    //   - rename moves it to a sibling path with a `.renamed`
    //     suffix.
    //   - read_file on the new path returns the original content.
    //   - unlink removes the renamed path; file_exists is false
    //     after.
    //
    // The test cleans up after itself by unlink — but if the
    // happy-path assertion fires before unlink, the orphan path
    // lives in /tmp until the OS reaps it (acceptable for /tmp).
    let src = r#"
        fn main() {
            let original = std::io::fs::mktemp("/tmp/hale_test_", ".tmp") or raise;
            std::io::fs::write_file(original, "c9 roundtrip payload") or raise;
            let renamed = original + ".renamed";
            std::io::fs::rename(original, renamed) or raise;
            let got = std::io::fs::read_file(renamed) or raise;
            println("got=", got);
            std::io::fs::unlink(renamed) or raise;
            if std::io::fs::file_exists(renamed) {
                println("still_exists=true");
            } else {
                println("still_exists=false");
            }
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("roundtrip", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("got=c9 roundtrip payload"),
        "expected roundtripped content; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("still_exists=false"),
        "expected unlink to remove the path; got: {:?}",
        stdout
    );
}

#[test]
fn rename_missing_src_surfaces_not_found() {
    // Failure mode: `rename` on a path that doesn't exist surfaces
    // ENOENT — IoError.kind is "not_found". The diagnostic-path
    // anchored on IoError.path is the destination, not the source,
    // matching the codegen convention documented in the runtime
    // (target dir / collision is the more diagnostic of the two).
    let src = r#"
        fn show(e: IoError) {
            println("kind=", e.kind);
            println("path=", e.path);
        }
        fn main() {
            std::io::fs::rename("/no/such/hale_c9_src", "/tmp/hale_c9_dst")
                or show(err);
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("rename_missing_src", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("kind=not_found"),
        "expected not_found; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=/tmp/hale_c9_dst"),
        "expected destination path in IoError; got: {:?}",
        stdout
    );
}

#[test]
fn unlink_missing_surfaces_not_found() {
    let src = r#"
        fn show(e: IoError) {
            println("kind=", e.kind);
        }
        fn main() {
            std::io::fs::unlink("/no/such/path/hale_c9_unl") or show(err);
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("unlink_missing", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("kind=not_found"),
        "expected not_found; got: {:?}",
        stdout
    );
}

#[test]
fn mktemp_bad_prefix_dir_surfaces_io_error() {
    // mkstemps fails when the prefix points at a directory that
    // doesn't exist — the IoError.path captures the assembled
    // template so the agent sees the prefix + XXXXXX + suffix
    // shape that hit the failure. The `or substitute` shape
    // keeps the let-binding's value-arm and err-arm shapes
    // aligned at String.
    let src = r#"
        fn report(e: IoError) -> String {
            return "kind=" + e.kind + " path=" + e.path;
        }
        fn main() {
            let result = std::io::fs::mktemp("/no/such/dir/hale_c9_", ".tmp")
                or report(err);
            println(result);
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("mktemp_bad_dir", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("kind=not_found"),
        "expected not_found; got: {:?}",
        stdout
    );
    // The diagnostic-path is the assembled template, so it should
    // contain both the prefix and the suffix.
    assert!(
        stdout.contains("path=/no/such/dir/hale_c9_XXXXXX.tmp"),
        "expected template path; got: {:?}",
        stdout
    );
}
