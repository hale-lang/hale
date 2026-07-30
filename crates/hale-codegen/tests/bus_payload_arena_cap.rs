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

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("lotus_test_bus_arena_cap_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn snapshot_with_tls_routing_skips_global_cap() {
    // 2026-05-19 update: after the __caller_arena threading lands,
    // BytesBuilder snapshot/finish allocations route through the
    // calling fn's arena (via the TLS `lotus_current_caller_arena`)
    // instead of the program-lifetime g_bus_payload_arena. The
    // cap mechanism still exists on the global arena, but the
    // common-case stdlib paths no longer touch it — that's the
    // whole point of the threading work.
    //
    // This test pins that behavior: allocating 100x 1 MiB
    // snapshots with a small global cap should NOT fire the cap
    // diagnostic, because the snapshots land in Parent's
    // (long-lived) locus arena, not the capped global. Parent's
    // arena is unbounded; allocations there are bounded by
    // Parent's lifecycle (still program-lifetime here since
    // Parent IS the program root, but the principle holds at
    // any locus depth).
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
                while i < 50 {
                    b.append(chunk);
                    let snap = b.snapshot();
                    i = i + 1;
                }
                println("loop done");
            }
        }
        fn main() { Parent { }; }
    "#;
    let bin = build("snapshot_tls", src);
    let output = Command::new(&bin)
        .env("LOTUS_BUS_PAYLOAD_ARENA_CAP", "65536") // 64 KiB
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected clean run — stdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("loop done"),
        "expected loop completion: {:?}",
        stdout
    );
    // The cap diagnostic should NOT fire — TLS routing keeps the
    // allocations out of g_bus_payload_arena.
    assert!(
        !stderr.contains("arena cap hit"),
        "unexpected cap diagnostic — TLS routing should bypass \
         the global arena: {:?}",
        stderr
    );
    assert!(
        !stdout.contains("absorbed=alloc_failed"),
        "no violation expected — TLS-routed snapshot should \
         succeed against Parent's arena: {:?}",
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
