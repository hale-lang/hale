//! `lotus_bus_queue_idle_wait`: the main queue's bounded wait for work.
//! The C driver pins the contract: a wait with nothing enqueued returns
//! at its deadline, and an enqueue from another thread ends a 1 s wait
//! within a few milliseconds (microseconds on an idle box) with the
//! cell's handler already run.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

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
