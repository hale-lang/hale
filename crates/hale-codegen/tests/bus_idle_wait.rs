//! `lotus_bus_queue_idle_wait`: the main queue's bounded wait for work.
//! The C driver pins the contract: a wait with nothing enqueued returns
//! at its deadline, and an enqueue from another thread ends a 1 s wait
//! within a few milliseconds (microseconds on an idle box) with the
//! cell's handler already run.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

#[test]
fn a_foreign_enqueue_ends_the_wait_and_a_quiet_wait_ends_at_its_deadline() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = harness::unique_bin("lotus_bus_idle_wait_driver");
    let status = Command::new("clang")
        .arg(manifest.join("tests").join("bus_idle_wait_driver.c"))
        .arg(manifest.join("runtime").join("lotus_arena.c"))
        .arg("-O2")
        .arg("-lpthread")
        .arg("-lm")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building the idle-wait driver");
    let out = Command::new(&bin).output().expect("run driver");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "idle-wait contract violated.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok "));
}

/// The duration of `std::time::__idle_wait` is evaluated once, whichever
/// thread the call runs on: the main-queue wait and the `sleep` it falls
/// back to take the same value. A duration that counts its calls would
/// otherwise count twice on a pool worker, and sleep for the second value.
#[test]
fn the_duration_is_evaluated_once_for_both_wait_backends() {
    let src = r#"
fn next_delay() -> Duration {
    print("delay asked");
    return 1ms;
}

fn main() {
    std::time::__idle_wait(next_delay());
}
"#;
    let bin = harness::unique_bin("lotus_idle_wait_once");
    let ir = harness::build_source_ir_text(src, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    assert!(ir.contains("lotus_bus_idle_wait_main"), "the call lowers to the idle wait");
    let calls = ir
        .lines()
        .filter(|l| l.contains("call i64 @next_delay("))
        .count();
    assert_eq!(calls, 1, "one call of the duration fn for the one site:\n{ir}");
}
