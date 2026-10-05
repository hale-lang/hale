//! WS3.3 — a bus topic declared in a different file than its
//! publisher (and a cross-seed subscriber).
//!
//! pond reported (FRICTION, corrected 2026-06-08) that `publish T`
//! + `T <- v` only resolved a `topic T` declared in the *same*
//! `.hl` file as the publishing locus, forcing topics + publishers
//! to be collapsed into one file in every library. This builds a
//! two-file library seed — `topics.hl` declares `Heartbeat`,
//! `emitter.hl` publishes it by bare name and sends on it — imported
//! by a consumer that subscribes via the qualified `hb::Heartbeat`.
//! If the publish/send sites resolve the topic across the file
//! boundary, the consumer receives both messages.

use std::path::PathBuf;
use std::process::Command;


#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

#[test]
fn topic_decl_and_publisher_in_separate_lib_files() {
    let consumer_src_path = fixtures_dir()
        .join("import-ws33-topic-split-consumer")
        .join("main.hl");

    let bin = harness::unique_bin(&format!("hale_ws33_topic_split_{}", std::process::id()));
    build_opts::build_seed_dir(&consumer_src_path, &bin, &build_opts::options())
        .expect("build consumer + split-topic lib");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit (cross-file topic resolution regressed): {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("got 7"), "missing first publish: {:?}", stdout);
    assert!(stdout.contains("got 11"), "missing second publish: {:?}", stdout);
    assert!(stdout.contains("done"), "missing done sentinel: {:?}", stdout);
}
