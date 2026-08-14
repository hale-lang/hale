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

    let replayed = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replayed run");
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

/// The REAL fast-forward proof (review round: the old three-second
/// bound was satisfiable by a fully live re-execution). The recorded
/// run holds a 2.5s coro park; the recorded wall time proves it. A
/// replay that re-waited that park cannot finish under the bound
/// asserted here — only a tape-driven EXPIRE can.
#[test]
fn expiry_fast_forwards_from_the_tape() {
    const LONG_PARK: &str = r#"
        type Job { n: Int = 0; }
        locus Slow {
            params { seen: Int = 0; }
            bus { subscribe "ff.job" as on_j of type Job; }
            fn on_j(j: Job) {
                std::time::sleep(2500ms);
                self.seen = self.seen + 1;
                println("done=", j.n);
            }
        }
        main locus App {
            params { s: Slow = Slow { }; }
            placement { s: cooperative(pool = io) where async_io; }
            bus { publish "ff.job" of type Job; }
            run() {
                "ff.job" <- Job { n: 7 };
                let mut waited = 0;
                while self.s.seen < 1 {
                    std::time::sleep(50ms);
                    waited = waited + 1;
                    if waited > 200 { std::process::exit(3); }
                }
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("ffwd", LONG_PARK);
    let rec = rec_path("ffwd");

    let t0 = Instant::now();
    let recorded = Command::new(&bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .output()
        .expect("recorded run");
    let record_wall = t0.elapsed();
    assert!(
        recorded.status.success(),
        "recorded run failed: {}",
        String::from_utf8_lossy(&recorded.stderr)
    );
    // The park is real: the recorded run cannot beat its own sleep.
    assert!(
        record_wall.as_millis() >= 2400,
        "recorded run finished in {}ms — the park never happened",
        record_wall.as_millis()
    );

    let t1 = Instant::now();
    let replayed = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .output()
        .expect("replayed run");
    let replay_wall = t1.elapsed();
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(replayed.status.success(), "stderr:\n{}", err);
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        String::from_utf8_lossy(&recorded.stdout)
    );
    assert!(err.contains("0 divergences"), "stderr:\n{}", err);
    // Strictly below the recorded park: only a tape-driven EXPIRE
    // gets here. (Margin for loaded machines; the recorded lower
    // bound above is what makes this a proof rather than a vibe.)
    assert!(
        replay_wall.as_millis() < 1500,
        "replay took {}ms against a {}ms recorded park — the \
         schedule was re-timed, not tape-driven",
        replay_wall.as_millis(),
        record_wall.as_millis()
    );

    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&bin);
}

/// The RESUME path, adversarially: two coros parked on real fds, a
/// third on a timer — READINESS ARRIVES IN THE OPPOSITE ORDER of
/// the recorded resumes, and replay must hold the early-ready coro
/// on ready_head until its recorded turn. Mixed decision stream:
/// STARTs (three cells), RESUMEs (two fd parks), EXPIRE (the timer).
#[test]
fn readiness_resumes_follow_the_tape_not_arrival_order() {
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};

    let (port_a, port_b) = {
        let a = TcpListener::bind("127.0.0.1:0").unwrap();
        let b = TcpListener::bind("127.0.0.1:0").unwrap();
        let (pa, pb) = (
            a.local_addr().unwrap().port(),
            b.local_addr().unwrap().port(),
        );
        drop(a);
        drop(b);
        (pa, pb)
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ready = format!(
        "{}/hale-296-resume-{}-{}.ready",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    );
    let src = format!(
        r#"
        type Kick {{ n: Int = 0; }}
        fn handle_a(s: std::io::tcp::Stream) {{
            let b = std::bytes::BytesBuilder {{ }};
            let got = std::io::tcp::recv_into(s.conn_fd, b, 64);
            println("ra=", got);
        }}
        fn handle_b(s: std::io::tcp::Stream) {{
            let b = std::bytes::BytesBuilder {{ }};
            let got = std::io::tcp::recv_into(s.conn_fd, b, 64);
            println("rb=", got);
        }}
        locus Timer {{
            bus {{ subscribe "rs.kick" as on_k of type Kick; }}
            fn on_k(k: Kick) {{
                std::time::sleep(300ms);
                println("timer=", k.n);
            }}
        }}
        main locus App {{
            params {{
                t: Timer = Timer {{ }};
                la: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:          "127.0.0.1",
                    port:          {port_a},
                    max_accepts:   1,
                    on_connection: handle_a,
                }};
                lb: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:          "127.0.0.1",
                    port:          {port_b},
                    max_accepts:   1,
                    on_connection: handle_b,
                }};
            }}
            placement {{
                t:  cooperative(pool = io) where async_io;
                la: cooperative(pool = io) where async_io;
                lb: cooperative(pool = io) where async_io;
            }}
            bus {{ publish "rs.kick" of type Kick; }}
            run() {{
                std::io::fs::write_file("{ready}", "ready") or discard;
                "rs.kick" <- Kick {{ n: 1 }};
                std::time::sleep(1800ms);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    );
    let bin = build("resume", &src);
    let rec = rec_path("resume");

    let wait_ready = |ready: &str| {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(20);
        while !std::path::Path::new(ready).exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "program never reached run()"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    // drive(sends_a_first): connect A then B (accept order fixed),
    // then deliver DATA in the given order.
    let drive = |a_first: bool| {
        wait_ready(&ready);
        let mut ca =
            TcpStream::connect(("127.0.0.1", port_a)).expect("a");
        let mut cb =
            TcpStream::connect(("127.0.0.1", port_b)).expect("b");
        std::thread::sleep(std::time::Duration::from_millis(150));
        let (first, second): (&mut TcpStream, &mut TcpStream) =
            if a_first {
                (&mut ca, &mut cb)
            } else {
                (&mut cb, &mut ca)
            };
        first.write_all(b"hello").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(250));
        second.write_all(b"hello").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        (ca, cb) // keep the peers open past the reads
    };

    // Record: A's data first — recorded resume order is A, B.
    let _ = std::fs::remove_file(&ready);
    let mut child = Command::new(&bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn recorded");
    let _peers = drive(true);
    let recorded = child.wait_with_output().expect("recorded exit");
    assert!(
        recorded.status.success(),
        "recorded run failed: {}",
        String::from_utf8_lossy(&recorded.stderr)
    );
    let recorded_stdout =
        String::from_utf8_lossy(&recorded.stdout).into_owned();
    assert!(
        recorded_stdout.contains("ra=5")
            && recorded_stdout.contains("rb=5")
            && recorded_stdout.contains("timer=1"),
        "recorded run incomplete:\n{}",
        recorded_stdout
    );
    let a_pos = recorded_stdout.find("ra=").unwrap();
    let b_pos = recorded_stdout.find("rb=").unwrap();
    assert!(
        a_pos < b_pos,
        "premise: the recording resumed A before B\n{}",
        recorded_stdout
    );

    // Replay: B's data arrives FIRST. B's readiness is early — the
    // drain must hold it on ready_head and resume A at its recorded
    // turn. stdout must be byte-identical to the recording.
    let _ = std::fs::remove_file(&ready);
    let mut child = Command::new(&bin)
        .env("LOTUS_REPLAY", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn replay");
    let _peers = drive(false);
    let replayed = child.wait_with_output().expect("replay exit");
    let err = String::from_utf8_lossy(&replayed.stderr);
    assert!(replayed.status.success(), "stderr:\n{}", err);
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        recorded_stdout,
        "reversed readiness must not reorder the replay;\nstderr:\n{}",
        err
    );
    assert!(err.contains("0 divergences"), "stderr:\n{}", err);

    let _ = std::fs::remove_file(&ready);
    let _ = std::fs::remove_file(&rec);
    let _ = std::fs::remove_file(&bin);
}
