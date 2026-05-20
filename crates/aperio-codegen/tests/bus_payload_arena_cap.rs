//! Phase-3 safety net (2026-05-19): hard byte cap on
//! g_bus_payload_arena. Allocations past the cap return NULL; the
//! existing alloc_fail_sentinel + empty-Bytes paths surface the
//! failure as degraded service (alloc_failed violations, empty
//! results) rather than OOM.
//!
//! Default cap is 64 MiB; overridable via the
//! LOTUS_BUS_PAYLOAD_ARENA_CAP env var (bytes). Tests exercise
//! the cap by setting a small value via env and forcing
//! allocations through BytesBuilder.snapshot() (which routes
//! through the alloc_fail_sentinel + violate alloc_failed pair
//! shipped earlier in this branch).

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_bus_arena_cap_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn snapshot_at_cap_violates_alloc_failed() {
    // Set the cap below the alloc size we'll force; the second
    // snapshot deposit exceeds the cap and the C primitive
    // returns the alloc_fail_sentinel, which the BytesBuilder
    // locus's snapshot() method routes through
    // `violate alloc_failed`. With a parent on_failure, the
    // violation is absorbed and run() continues.
    //
    // We allocate 1 MiB chunks via append_slice from a 1 MiB
    // source bytes blob and snapshot each time. The cap is set
    // to 2 MiB so the third snapshot exceeds the cap (taking
    // into account the initial chunk size + per-snapshot alloc).
    let src = r#"
        locus Parent {
            accept(b: std::bytes::BytesBuilder) { }
            on_failure(b: std::bytes::BytesBuilder, err: ClosureViolation) {
                println("absorbed=", err.closure);
            }
            run() {
                let b = std::bytes::BytesBuilder { initial_cap: 1048576 };
                let chunk = std::bytes::from_string(
                    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
                );
                let mut i = 0;
                while i < 100 {
                    b.append(chunk);
                    let snap = b.snapshot();
                    println("iter=", i, " len=", len(snap));
                    i = i + 1;
                }
                println("loop done");
            }
        }
        fn main() { Parent { }; }
    "#;
    let bin = build("snapshot_cap", src);
    let output = Command::new(&bin)
        .env("LOTUS_BUS_PAYLOAD_ARENA_CAP", "65536") // 64 KiB
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "non-zero with absorbed violations expected — stdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    // The cap diagnostic fires exactly once on first hit.
    assert!(
        stderr.contains("arena cap hit"),
        "expected arena cap hit diagnostic on stderr: {:?}",
        stderr
    );
    assert!(
        stderr.contains("g_bus_payload_arena"),
        "expected arena name in diagnostic: {:?}",
        stderr
    );
    // Once the cap fires, subsequent snapshots violate; the
    // parent absorbs and the loop keeps going.
    assert!(
        stdout.contains("absorbed=alloc_failed"),
        "expected absorbed alloc_failed in stdout: {:?}",
        stdout
    );
}

#[test]
fn default_cap_does_not_fire_for_modest_workload() {
    // The default 64 MiB cap should be ample for any normal
    // test workload. This negative test guards against
    // accidentally setting the default too low and breaking
    // unrelated tests.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 1024 };
            b.append(std::bytes::from_string("hello world"));
            let s = b.finish();
            println("len=", len(s));
        }
    "#;
    let bin = build("default_cap_safe", src);
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}/{}", stdout, stderr);
    assert!(stdout.contains("len=11"), "got: {:?}", stdout);
    assert!(
        !stderr.contains("arena cap hit"),
        "no cap diagnostic expected: {:?}",
        stderr
    );
}
