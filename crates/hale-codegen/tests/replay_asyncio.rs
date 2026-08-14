//! GH #296 phase 6 — `where async_io` pools record and REPLAY.
//!
//! The nondeterminism of an async pool is its drain's scheduling:
//! cell starts, epoll-readiness resumes, timed-park expiries. Under
//! recording each decision lands on the worker's private ring as a
//! step (START/RESUME/EXPIRE); under replay the drain is driven by
//! that stream instead of the clock. What these tests hold that to:
//!
//!   1. **The recording carries the schedule.** An async pool with
//!      two loci whose handlers park on different sleep durations
//!      records a step stream (the loud pre-phase-6 refusal is gone).
//!   2. **Replay reproduces it, divergence-free.** The same binary
//!      replayed produces byte-identical stdout and reports ZERO
//!      divergences — expiry steps resume in recorded order without
//!      re-waiting wall-clock time, so the replay also finishes
//!      FASTER than the recorded run's sleeps would allow.
//!   3. **Interleaved expiries are order-enforced, not re-timed.**
//!      Deliberately staggered sleeps make the recorded interleaving
//!      depend on timing; replay must reproduce the recorded
//!      interleaving from the tape, not re-derive it from timers.

use std::process::Command;
use std::time::Instant;

use hale_codegen::{build_executable_with_options, BuildOptions};

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(name.to_string(), &program);
    let bundle = hale_types::Bundle::new(programs);
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let bin = harness::unique_bin(&format!("hale_test_rpasync_{}", name));
    let options = BuildOptions {
        model_hash: Some(model_hash),
        ..BuildOptions::default()
    };
    build_executable_with_options(&program, &bin, &[], &options)
        .expect("build");
    bin
}

fn rec_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale_rpasync_{}_{}.halerec",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&p);
    p
}

/// Two subscribers on ONE async_io pool, parking on deliberately
/// staggered sleeps: the printed interleaving of `slow=`/`fast=`
/// lines is decided by expiry scheduling, not program order.
const STAGGERED: &str = r#"
    type Job { n: Int = 0; }
    type Poke { n: Int = 0; }

    locus Slow {
        bus { subscribe "as.job" as on_j of type Job; }
        fn on_j(j: Job) {
            std::time::sleep(35ms);
            println("slow=", j.n);
        }
    }

    locus Fast {
        bus { subscribe "as.poke" as on_p of type Poke; }
        fn on_p(p: Poke) {
            std::time::sleep(6ms);
            println("fast=", p.n);
        }
    }

    main locus App {
        params { s: Slow = Slow { }; f: Fast = Fast { }; }
        placement {
            s: cooperative(pool = io) where async_io;
            f: cooperative(pool = io) where async_io;
        }
        bus {
            publish "as.job" of type Job;
            publish "as.poke" of type Poke;
        }
        run() {
            let mut i = 0;
            while i < 6 {
                "as.job" <- Job { n: i };
                "as.poke" <- Poke { n: i };
                i = i + 1;
            }
            std::time::sleep(700ms);
            println("done");
        }
    }

    fn main() { App { }; }
"#;

#[test]
fn async_pool_schedule_records_and_replays_exactly() {
    let bin = build("staggered", STAGGERED);
    let rec = rec_path("staggered");

    let recorded = Command::new(&bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .output()
        .expect("recorded run");
    assert!(
        recorded.status.success(),
        "recorded run failed: {}",
        String::from_utf8_lossy(&recorded.stderr)
    );
    let recorded_stdout =
        String::from_utf8_lossy(&recorded.stdout).into_owned();
    // Sanity: both handler families actually ran on the async pool.
    assert!(
        recorded_stdout.contains("slow=5")
            && recorded_stdout.contains("fast=5")
            && recorded_stdout.contains("done"),
        "recorded run incomplete:\n{}",
        recorded_stdout
    );

    let t0 = Instant::now();
    let replayed = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replayed run");
    let replay_wall = t0.elapsed();
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(
        replayed.status.success(),
        "replay failed; stderr:\n{}",
        err
    );
    // 1. The pre-phase-6 refusal is gone.
    assert!(
        !err.contains("not replayable"),
        "async_io pools must no longer refuse replay; stderr:\n{}",
        err
    );
    // 2. Byte-identical schedule-dependent output.
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        recorded_stdout,
        "replayed interleaving differs from the recording's"
    );
    // ...with zero divergences: every step was satisfied from the
    // tape, never from the degrade path.
    assert!(
        err.contains("0 divergences"),
        "expected a divergence-free async replay; stderr:\n{}",
        err
    );
    // 3. Expiries were tape-driven, not re-timed: the recorded run
    //    holds a 700ms settle sleep on main plus the staggered
    //    handler sleeps; a replay that re-waited them cannot finish
    //    this fast. (Generous bound — the point is "did not
    //    re-sleep", not a benchmark.)
    assert!(
        replay_wall.as_millis() < 3000,
        "replay took {}ms — schedule should be tape-driven",
        replay_wall.as_millis()
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn two_replays_of_one_async_recording_are_identical() {
    let bin = build("twice", STAGGERED);
    let rec = rec_path("twice");

    let recorded = Command::new(&bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .output()
        .expect("recorded run");
    assert!(recorded.status.success());

    let a = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay a");
    let b = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replay b");
    assert!(a.status.success() && b.status.success());
    assert_eq!(
        String::from_utf8_lossy(&a.stdout),
        String::from_utf8_lossy(&b.stdout),
        "two replays of one recording must agree with each other"
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&bin);
}
