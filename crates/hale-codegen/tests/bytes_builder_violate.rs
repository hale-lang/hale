//! F.27 violate routing for BytesBuilder snapshot()/finish().
//!
//! Both methods previously returned an empty Bytes blob silently
//! on payload-arena alloc failure (handle null, arena_create
//! fail, lotus_bytes_create fail) — indistinguishable from the
//! legitimate success-with-len-0 return. Per the empty-global
//! singleton (lotus_arena.c) success now always allocates a fresh
//! arena blob even for len=0; failures return the singleton.
//! Locus method bodies discriminate via
//! `std::bytes::__is_empty_global(out)` and `violate alloc_failed`
//! on the failure marker.
//!
//! Forcing the failure path without an allocator mock: call
//! `finish()` twice. The first call frees the underlying buffer
//! and clears `self.handle` to 0; the second call sees handle=0
//! and the C primitive's `if (!handle)` branch returns the
//! singleton — exactly the failure marker the violate route
//! intercepts.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let bin = harness::unique_bin(&format!("hale_test_bb_violate_{}", name));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn double_finish_routes_through_violate_alloc_failed() {
    // First finish() succeeds and clears self.handle to 0; the
    // second finish() now hits the null-handle branch in the C
    // primitive and returns the empty-global singleton. The locus
    // method body's __is_empty_global check fires and routes
    // through `violate alloc_failed`. The Parent's on_failure
    // absorbs the violation; `finish` is fallible(ClosureViolation)
    // (F.42), so the call then fails and its `or` runs with the
    // record; run() continues.
    let src = r#"
        locus Parent {
            accept(b: std::bytes::BytesBuilder) { }
            on_failure(b: std::bytes::BytesBuilder, err: ClosureViolation) {
                println("absorbed closure=", err.closure);
            }
            run() {
                let b = std::bytes::BytesBuilder { initial_cap: 64 };
                let first = b.finish() or raise;
                println("first len=", len(first));
                let second = b.finish() or { println("finish failed closure=", err.closure); std::bytes::from_string("") };
                println("parent.run continued");
            }
        }
        fn main() { Parent { }; }
    "#;
    let (stdout, status) = build_and_run("double_finish", src);
    assert!(status.success(), "non-zero: {:?}\nstdout:\n{}", status, stdout);
    assert!(
        stdout.contains("first len=0"),
        "first finish should succeed empty: {:?}",
        stdout
    );
    assert!(
        stdout.contains(
            "absorbed closure=alloc_failed\nfinish failed closure=alloc_failed\nparent.run continued"
        ),
        "expected the handler, then the call's `or` with the record, then run() going on: {:?}",
        stdout
    );
}

#[test]
fn snapshot_after_finish_routes_through_violate_alloc_failed() {
    // finish() clears self.handle to 0; a subsequent snapshot()
    // sees handle=0 in the C primitive and returns the singleton.
    // The locus method body routes through violate. Exercises the
    // snapshot path's discriminator independently from finish.
    let src = r#"
        locus Parent {
            accept(b: std::bytes::BytesBuilder) { }
            on_failure(b: std::bytes::BytesBuilder, err: ClosureViolation) {
                println("absorbed closure=", err.closure);
            }
            run() {
                let b = std::bytes::BytesBuilder { initial_cap: 64 };
                let done = b.finish() or raise;
                println("first len=", len(done));
                let snap = b.snapshot() or std::bytes::from_string("");
                println("parent.run continued len=", len(snap));
            }
        }
        fn main() { Parent { }; }
    "#;
    let (stdout, status) = build_and_run("snap_after_finish", src);
    assert!(status.success(), "non-zero: {:?}\nstdout:\n{}", status, stdout);
    assert!(
        stdout.contains("absorbed closure=alloc_failed"),
        "expected absorbed alloc_failed closure: {:?}",
        stdout
    );
    assert!(
        stdout.contains("parent.run continued"),
        "expected run() to keep going after the absorbed violation: {:?}",
        stdout
    );
}

#[test]
fn unhandled_snapshot_failure_exits_nonzero() {
    // No parent on_failure handler → violate's bare-panic branch
    // fires: dprintf to stderr + exit(1). Exercises the
    // unabsorbed-violation route end-to-end (the "process exits
    // non-zero with the violation report on stderr" rule from
    // F.27's stdlib doc).
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            let done = b.finish() or raise;
            let snap = b.snapshot() or raise;
            println("unreachable");
        }
    "#;
    let bin = harness::unique_bin("hale_test_bb_violate_unhandled");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        !output.status.success(),
        "expected non-zero exit on unhandled violation; stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("alloc_failed"),
        "expected closure name in stderr: {:?}",
        stderr
    );
}
