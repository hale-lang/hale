//! GH #296 phase 5 — durable recording: a crash-truncated recording is
//! a usable prefix.
//!
//! The drain appends whole frames in stream order and stamps the
//! header identity eagerly (not only at finalize), so a run that
//! dies without tearing down leaves an artifact that is exact up to
//! one torn frame at the tail. What these tests hold that to:
//!
//!   1. **Kill mid-run, keep the prefix.** SIGKILL a recording run;
//!      the file has no clean-finalize trailer, but its header
//!      identity (model_hash) is already stamped — the artifact is
//!      attributable to its program without a finalize.
//!   2. **Fail closed by default.** Replaying the truncated file
//!      refuses loudly and names the opt-in.
//!   3. **Opt in, replay the prefix.** With
//!      LOTUS_REPLAY_ALLOW_TRUNCATED=1 the same file replays: the
//!      loader stops at the torn tail, says so, and the program runs
//!      to completion (post-tape reads fall back live and are
//!      counted, never fatal).
//!   4. **Durable grade still finalizes.** LOTUS_OBS_RECORD_DURABLE=1
//!      changes flush discipline, not the format — a clean run under
//!      it produces a trailer-finalized artifact that replays with
//!      zero order divergence.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use hale_codegen::{build_executable_with_options, BuildOptions};

#[path = "support/harness.rs"]
mod harness;

const REC_END_MAGIC: u64 = 0x30444E45454C4148; // "HALEEND0"

/// Build WITH a model hash (as the CLI does): the eager-stamp
/// assertion below is about identity reaching a crashed artifact's
/// header, which needs the binary to carry an identity at all.
fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(name.to_string(), &program);
    let bundle = hale_types::Bundle::new(programs);
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let bin = harness::unique_bin(&format!("hale_test_trunc_{}", name));
    let options = BuildOptions {
        model_hash: Some(model_hash),
        // A designed digest so the eager-stamp assertion can check
        // ALL FOUR words landed (review round 2, finding 2: the old
        // change-key ignored words 2-3 and could persist a
        // half-published digest).
        exec_digest: Some([0xA1A1, 0xB2B2, 0xC3C3, 0xD4D4]),
        ..BuildOptions::default()
    };
    build_executable_with_options(&program, &bin, &[], &options)
        .expect("build");
    bin
}

fn rec_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale_trunc_{}_{}.halerec",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&p);
    p
}

/// A slow steady publisher: ~2s of 5ms-spaced publishes, so there is
/// a wide window to SIGKILL the run mid-stream with entries already
/// drained to the file.
const SLOW_STREAM: &str = r#"
    type Tick { n: Int = 0; }

    locus Sink {
        params { seen: Int = 0; }
        bus { subscribe "trunc.tick" as on_t of type Tick; }
        fn on_t(t: Tick) { self.seen = self.seen + 1; }
    }

    main locus App {
        params { s: Sink = Sink { }; }
        bus { publish "trunc.tick" of type Tick; }
        run() {
            let mut i = 0;
            while i < 400 {
                "trunc.tick" <- Tick { n: i };
                std::time::sleep(5ms);
                i = i + 1;
            }
        }
    }

    fn main() { App { }; }
"#;

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

#[test]
fn killed_recording_keeps_an_admissible_prefix() {
    let bin = build("kill", SLOW_STREAM);
    let rec = rec_path("kill");

    let mut child = Command::new(&bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .spawn()
        .expect("spawn recorded run");

    // Wait until the drain has demonstrably written entries past the
    // header, then kill without ceremony.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let sz = std::fs::metadata(&rec).map(|m| m.len()).unwrap_or(0);
        if sz > 4096 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "recording never grew past the header (size {})",
            sz
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().expect("SIGKILL the recording run");
    let _ = child.wait();

    let bytes = std::fs::read(&rec).expect("read truncated recording");
    assert!(bytes.len() > 4096);
    // 1. No clean-finalize trailer: the run died, and the artifact
    //    must say so.
    assert_ne!(
        u64_at(&bytes, bytes.len() - 16),
        REC_END_MAGIC,
        "a SIGKILLed run must not carry a clean-finalize trailer"
    );
    // ...but the header identity is ALREADY stamped (the eager
    // stamp) — the crashed artifact is attributable, not anonymous,
    // and the digest is COMPLETE: identity is committed atomically
    // after every setter and stamped from the immutable committed
    // copy, so a crash can never persist a half-published digest.
    assert_ne!(
        u64_at(&bytes, 48),
        0,
        "model_hash must be stamped before finalize (eager stamp)"
    );
    assert_eq!(
        [
            u64_at(&bytes, 56),
            u64_at(&bytes, 64),
            u64_at(&bytes, 72),
            u64_at(&bytes, 80)
        ],
        [0xA1A1, 0xB2B2, 0xC3C3, 0xD4D4],
        "the crash artifact must carry the COMPLETE exec digest"
    );

    // 2. Default: fail closed, name the opt-in.
    let refused = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("run refused replay");
    assert!(
        !refused.status.success(),
        "replaying a truncated recording must refuse by default"
    );
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(
        err.contains("--allow-truncated"),
        "the refusal must name the opt-in; stderr:\n{}",
        err
    );

    // 3. Opted in: the prefix replays and the program completes.
    let replayed = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .env("LOTUS_REPLAY_ALLOW_TRUNCATED", "1")
        .output()
        .expect("run prefix replay");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(
        replayed.status.success(),
        "prefix replay must run to completion; stderr:\n{}",
        err
    );
    assert!(
        err.contains("replaying the recorded prefix"),
        "the loader must announce the truncated prefix; stderr:\n{}",
        err
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn durable_grade_records_and_finalizes_clean() {
    let bin = build("durable", SLOW_STREAM);
    let rec = rec_path("durable");

    let recorded = Command::new(&bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .env("LOTUS_OBS_RECORD_DURABLE", "1")
        .output()
        .expect("durable recorded run");
    assert!(
        recorded.status.success(),
        "durable recording run failed: {}",
        String::from_utf8_lossy(&recorded.stderr)
    );

    let bytes = std::fs::read(&rec).expect("read durable recording");
    assert_eq!(
        u64_at(&bytes, bytes.len() - 16),
        REC_END_MAGIC,
        "a clean durable run must finalize with the trailer"
    );

    let replayed = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay durable recording");
    assert!(
        replayed.status.success(),
        "durable recording must replay: {}",
        String::from_utf8_lossy(&replayed.stderr)
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&bin);
}
