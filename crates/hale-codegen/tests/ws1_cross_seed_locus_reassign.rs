//! WS1#4 carrier — whole-reassignment of a nested locus param of a
//! cross-seed (imported) type.
//!
//! A downstream market-data gateway reported that `self.conn = ws::WsClient { url:
//! …, … }` (reconnecting by swapping the whole nested-locus param)
//! left the new instance half-initialized: `conn.url` logged
//! `(null)` and the first `read_msg()` crashed. In-place field
//! mutation (`self.conn.url = …`) worked. The single-seed form of
//! this passes at HEAD (verified 2026-06-11), so
//! this carrier exercises the untested axis: the reassigned type is
//! imported from another seed, and its handle-like fields are
//! established in `birth()`.
//!
//! Expectation: the reassigned instance is fully live — params land
//! (`url` = the new value, not null), `birth()` re-runs (the
//! handle `fd` = 7, `ready` = 1), and `read_msg()` returns without
//! crashing. A half-init regression shows up as a null/zero field
//! or a crash.

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
fn cross_seed_nested_locus_param_whole_reassignment_is_fully_initialized() {
    let consumer_src_path = fixtures_dir()
        .join("import-ws1-conn-reassign-consumer")
        .join("main.hl");

    let bin = harness::unique_bin(&format!("hale_ws1_xseed_reassign_{}", std::process::id()));
    build_opts::build_seed_dir(&consumer_src_path, &bin, &build_opts::options())
        .expect("build consumer + lib");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit (WS1#4 half-init carrier regressed — likely a \
         crash in read_msg() on a half-initialized reassigned conn): \
         {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Initial instance (default param + birth()).
    assert!(
        stdout.contains("init   url=wss://first fd=7 ready=1 read=8"),
        "initial cross-seed instance not fully initialized: {:?}",
        stdout
    );
    // After whole-reassignment: new url landed, birth() re-ran
    // (fd=7, ready=1), read_msg() ran on a fresh seq (fd + seq=1 = 8).
    assert!(
        stdout.contains("reconn url=wss://second fd=7 ready=1 read=8"),
        "reassigned cross-seed nested param is half-initialized \
         (a downstream app a market-data gateway shape): {:?}",
        stdout
    );
}
