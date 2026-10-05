//! F.40 L5 (decision line 19) under GH #296 replay: a run canceled in
//! its queue reproduces its recording with no divergence.
//!
//! A child's run() posted to a pool, whose owner is torn down on that
//! pool's worker before the run is dequeued, is canceled by the
//! child's reclaim and dropped unrun. Recording drops it without a
//! consume record. Replay's ordering gate compares each dequeued cell
//! with the recorded consume stream, so the canceled run has to be
//! dropped before that comparison: compared, it matches no recorded
//! consume, is held, and is released by the hold timeout as an order
//! divergence — and the CLI's replay verification rejects a replay
//! that reproduced its recording exactly (L5 part 1 review).
//!
//! Each program is built, recorded with `LOTUS_OBS_RECORD`, and
//! replayed with `LOTUS_REPLAY` and `LOTUS_REPLAY_STATUS`; both runs
//! print exactly `delivered 1`, and the replay's status file counts
//! four consumes (Spawner's initialization, its birth, its run, its own
//! delivery) and nothing else. Pool-root initialization is a queued
//! job since F.40 P1-3, and the root's birth() one more since decision
//! line 3 put it on the pool's worker (L4 part 5): each is a recorded
//! start of its own, in posting order (init, birth, run), and both stay
//! ahead of run in the edited tapes too.
//! The control is the same program without the queued run (Kid has
//! no run()), and the classic pool is checked with the lifecycle
//! trace on and off; the async pool's drain gates the same way.
//!
//! The ticket and the hold: the gate drops a run only when it is
//! already canceled, and only looks — a live run keeps its ticket
//! linked while the gate holds it, so the child stays protected for
//! the whole hold, and the run is admitted (its ticket unlinked) only
//! when the drain dispatches it. Two edited recordings pin both
//! sides: one whose consumes put a delivery ahead of a live run, so
//! the run is held and then started at its recorded slot, once; and
//! one that never consumed the run, so the held run is canceled by a
//! reclaim during the hold and the gate's sweep ends it unrun.
//!
//! The hold buffer is freed by the thread that held, at its exit: the
//! edited-recording replays run under AddressSanitizer with no leak
//! suppression, on a classic pool's worker, an async pool's worker and
//! a pinned thread, each joined at the teardown after it held.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use hale_codegen::{build_executable_with_options, BuildOptions};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The review's reproduction: Spawner, on pool `side`, makes an `Own`
/// whose `Kid` posts its run() to `side`, then drops it — the teardown
/// on the worker cancels the queued run — and publishes to itself.
fn program(kid_runs: bool, placement: &str) -> String {
    let kid = if kid_runs {
        r#"locus Kid { run() { println("unexpected kid run"); } }"#
    } else {
        "locus Kid { }"
    };
    format!(
        r#"
type Ping {{ n: Int = 0; }}
{kid}
locus Own {{ params {{ kid: Kid = Kid {{ }}; }} }}
locus Spawner {{
    bus {{
        subscribe "probe.ping" as on_ping of type Ping;
        publish "probe.ping" of type Ping;
    }}
    fn on_ping(p: Ping) {{ println("delivered " + to_string(p.n)); }}
    run() {{ Own {{ }}; "probe.ping" <- Ping {{ n: 1 }}; }}
}}
main locus App {{
    params {{ spawner: Spawner = Spawner {{ }}; }}
    placement {{ spawner: {placement}; }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

const CLASSIC: &str = "cooperative(pool = side)";
#[cfg(target_os = "linux")]
const ASYNC: &str = "cooperative(pool = side) where async_io";

fn build(name: &str, src: &str, trace: bool) -> PathBuf {
    build_with(name, src, trace, false)
}

fn build_with(name: &str, src: &str, trace: bool, asan: bool) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_replay_canceled_{}", name));
    let options = BuildOptions {
        lifecycle_trace: trace,
        asan,
        ..build_opts::options()
    };
    build_executable_with_options(&program, &bin, &[], &options).expect("build");
    bin
}

/// Run `bin` under `env`; its stdout and stderr, after asserting a
/// clean exit.
fn run(bin: &Path, what: &str, env: &[(&str, &OsStr)]) -> (String, String) {
    let mut cmd = Command::new(bin);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{what} exited {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status
    );
    (stdout, stderr)
}

/// Replay `rec` with `bin`; assert its stdout, that it counted
/// `consumes` consumes and that every other counter is zero. Returns
/// the replay's stderr.
fn replay_clean(bin: &Path, rec: &Path, expect_stdout: &str, consumes: &str) -> String {
    let status = bin.with_extension("status");
    let (stdout, stderr) = run(
        bin,
        "the replay",
        &[
            ("LOTUS_REPLAY", rec.as_os_str()),
            ("LOTUS_REPLAY_STATUS", status.as_os_str()),
        ],
    );
    assert_eq!(stdout, expect_stdout, "replay's stdout; stderr:\n{stderr}");
    let text = std::fs::read_to_string(&status).expect("replay status file");
    let _ = std::fs::remove_file(&status);
    let mut counted = None;
    let mut nonzero = Vec::new();
    for line in text.lines() {
        let (key, value) = line.split_once('=').expect("key=value");
        if key == "consumes" {
            counted = Some(value.to_string());
        } else if value != "0" {
            nonzero.push(line.to_string());
        }
    }
    assert_eq!(
        counted.as_deref(),
        Some(consumes),
        "the replay's consumes:\n{text}\nstderr:\n{stderr}"
    );
    assert!(
        nonzero.is_empty(),
        "a replay that reproduced its recording counts no divergence \
         (order_divergences included): {nonzero:?}\n{text}\nstderr:\n{stderr}"
    );
    stderr
}

/// Record, replay, and hold the replay to its recording: `delivered 1`
/// both times; Spawner's initialization, its birth, its run and its own
/// delivery are the four consumes. The canceled Kid contributes none.
/// Returns the replay's stderr.
fn record_and_replay(name: &str, src: &str, trace: bool) -> String {
    let bin = build(name, src, trace);
    let rec = bin.with_extension("halerec");

    let (stdout, stderr) = run(&bin, "the recorded run", &[("LOTUS_OBS_RECORD", rec.as_os_str())]);
    assert_eq!(stdout, "delivered 1\n", "recorded run's stdout; stderr:\n{stderr}");
    assert!(rec.is_file(), "no recording produced");

    let consumes = pool_consumes(&std::fs::read(&rec).expect("recording"), 1);
    assert!(
        consumes.len() == 4 && starts_of_one_locus(&consumes[..3]) && consumes[3].2 != 0,
        "Spawner init, Spawner birth, Spawner.run, then the ping; no canceled Kid.run: \
         {consumes:?}"
    );
    let stderr = replay_clean(&bin, &rec, "delivered 1\n", "4");

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
    stderr
}

#[test]
fn a_queued_run_canceled_by_its_owners_teardown_replays_clean() {
    record_and_replay("classic", &program(true, CLASSIC), false);
}

#[test]
fn the_control_without_a_queued_run_replays_clean() {
    record_and_replay("control", &program(false, CLASSIC), false);
}

#[test]
fn the_traced_cancellation_replays_clean_and_is_named() {
    let stderr = record_and_replay("traced", &program(true, CLASSIC), true);
    assert!(
        stderr.contains("NotStarted(Acknowledged)"),
        "the replay canceled the queued run, and the trace names it:\n{stderr}"
    );
    record_and_replay("traced_control", &program(false, CLASSIC), true);
}

#[cfg(target_os = "linux")]
#[test]
fn a_queued_run_canceled_on_an_async_pool_replays_clean() {
    record_and_replay("async", &program(true, ASYNC), false);
    record_and_replay("async_control", &program(false, ASYNC), false);
}

/// The recorded consumes (private-ring entries of kind CONSUME) of the
/// consumer that consumed the `identified` identified deliveries, all
/// on one consumer: each entry's offset, its target locus and its
/// delivery identity (0 for a posted init, birth or run job).
fn pool_consumes(buf: &[u8], identified: usize) -> Vec<(usize, u32, u64)> {
    let mut all = Vec::new();
    let hlen = u32::from_le_bytes(buf[12..16].try_into().unwrap()) as usize;
    let mut end = buf.len();
    if &buf[end - 16..end - 8] == b"HALEEND0" {
        end -= 16;
    }
    let mut off = hlen;
    while off + 8 <= end {
        let word = |at: usize| u64::from_le_bytes(buf[at..at + 8].try_into().unwrap());
        let tag = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        if tag == 0 {
            let ring = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap());
            if ring & 0x8000_0000 != 0 && (word(off + 8) >> 20) & 0x1F == 2 {
                all.push((ring, off, (word(off + 8) & 0xF_FFFF) as u32, word(off + 16)));
            }
            off += 24;
        } else {
            off += 32 + ((word(off + 24) as usize + 7) & !7);
        }
    }
    let delivered: Vec<u32> = all.iter().filter(|e| e.3 != 0).map(|e| e.0).collect();
    assert!(
        delivered.len() == identified && delivered.iter().all(|r| *r == delivered[0]),
        "{identified} identified deliveries on one consumer: {all:?}"
    );
    all.iter()
        .filter(|e| e.0 == delivered[0])
        .map(|e| (e.1, e.2, e.3))
        .collect()
}

/// `starts` are recorded starts (identity 0) of one locus: a pool-placed
/// root's posted init, birth and run, which the gate tells apart only by
/// their place in the stream.
fn starts_of_one_locus(starts: &[(usize, u32, u64)]) -> bool {
    starts.iter().all(|c| c.2 == 0 && c.1 != 0 && c.1 == starts[0].1)
}

/// A pool-placed root's worker stream in its recording: each recorded
/// start (CONSUME of identity 0) as its target locus, and each journaled
/// `next_int` read as its bound (the read's recorded argument), in ring
/// order, on the private ring whose consumer recorded a start of that
/// locus: the worker the jobs were posted to.
#[derive(Debug, PartialEq)]
enum WorkerEvent {
    Start(u32),
    NextInt(i64),
}

fn worker_stream(buf: &[u8]) -> Vec<WorkerEvent> {
    let word = |at: usize| u64::from_le_bytes(buf[at..at + 8].try_into().unwrap());
    let hlen = u32::from_le_bytes(buf[12..16].try_into().unwrap()) as usize;
    let mut end = buf.len();
    if &buf[end - 16..end - 8] == b"HALEEND0" {
        end -= 16;
    }
    // (ring, kind, w0, w1) of every private-ring record, and the bound
    // of every journaled next_int (kind 3) by (consumer, seq).
    let mut records = Vec::new();
    let mut bounds = std::collections::HashMap::new();
    let mut off = hlen;
    while off + 8 <= end {
        let tag = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        let a = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap());
        if tag == 0 {
            if a & 0x8000_0000 != 0 {
                records.push((a, (word(off + 8) >> 20) & 0x1F, word(off + 8), word(off + 16)));
            }
            off += 24;
        } else {
            let size = word(off + 24) as usize;
            if tag == 2 && a == 3 {
                let body = &buf[off + 32..off + 32 + size];
                let args_len = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
                assert_eq!(args_len, 8, "next_int's argument is its bound");
                let bound = i64::from_le_bytes(body[4..12].try_into().unwrap());
                bounds.insert((word(off + 8), word(off + 16) & !(1 << 63)), bound);
            }
            off += 32 + ((size + 7) & !7);
        }
    }
    let worker = records
        .iter()
        .find(|r| r.1 == 2 && r.3 == 0)
        .expect("a recorded start")
        .0;
    let consumer = records
        .iter()
        .find(|r| r.0 == worker && r.1 == 1)
        .expect("the worker's consumer record")
        .3;
    records
        .iter()
        .filter(|r| r.0 == worker)
        .filter_map(|r| match r.1 {
            2 => Some(WorkerEvent::Start((r.2 & 0xF_FFFF) as u32)),
            4 if r.3 >> 56 == 3 => {
                let seq = r.3 & 0xFF_FFFF_FFFF_FFFF;
                Some(WorkerEvent::NextInt(bounds[&(consumer, seq)]))
            }
            _ => None,
        })
        .collect()
}

/// Decision line 3 (L4 part 5): a pool-placed root's birth() is a job
/// posted to its pool between the init and the run, so the worker's
/// recording carries three recorded starts of the root, in posting
/// order: init, birth, run. Each job makes one journaled read whose
/// bound names it (11 in the init, through the nested Probe's birth;
/// 22 in Spawner's birth; 33 in its run), so the stream pins which
/// start ran which job. Replay consumes the birth as one more start:
/// two replays of one recording print what it printed, count the three
/// starts, serve every journaled read in its recorded slot and count no
/// divergence.
#[test]
fn a_pool_placed_roots_init_birth_and_run_are_recorded_starts_in_posting_order() {
    let src = r#"
locus Probe {
    birth() { if std::rand::next_int(11) >= 0 { println("probe born"); } }
}
locus Spawner {
    params { probe: Probe = Probe { }; }
    birth() { if std::rand::next_int(22) >= 0 { println("spawner born"); } }
    run() { if std::rand::next_int(33) >= 0 { println("spawner ran"); } }
}
main locus App {
    params { spawner: Spawner = Spawner { }; }
    placement { spawner: cooperative(pool = side); }
}
fn main() { App { }; }
"#;
    let stdout = "probe born\nspawner born\nspawner ran\n";
    let bin = build_with("posted_birth", src, true, true);
    let rec = bin.with_extension("halerec");
    let (recorded, stderr) =
        run(&bin, "the recorded run", &[("LOTUS_OBS_RECORD", rec.as_os_str())]);
    assert_eq!(recorded, stdout, "recorded run's stdout; stderr:\n{stderr}");
    let stream = worker_stream(&std::fs::read(&rec).expect("recording"));
    let spawner = match stream.first() {
        Some(WorkerEvent::Start(locus)) => *locus,
        _ => panic!("the worker's stream opens with a recorded start: {stream:?}"),
    };
    assert_eq!(
        stream,
        [
            WorkerEvent::Start(spawner),
            WorkerEvent::NextInt(11),
            WorkerEvent::Start(spawner),
            WorkerEvent::NextInt(22),
            WorkerEvent::Start(spawner),
            WorkerEvent::NextInt(33),
        ],
        "Spawner's init (Probe's birth inside it), Spawner's birth, Spawner.run"
    );
    let first = replay_clean(&bin, &rec, stdout, "3");
    let second = replay_clean(&bin, &rec, stdout, "3");
    assert!(
        !first.contains("NotStarted") && !second.contains("NotStarted"),
        "no posted job is canceled on replay:\n{first}\n{second}"
    );
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
}

/// Build `src` traced and under AddressSanitizer (a ticket freed twice
/// or never, or a hold buffer its thread did not free, fails the run),
/// record it and assert the recorded run's stdout; the binary, the
/// recording's bytes and the consumes of the consumer that consumed
/// the `identified` identified deliveries.
fn record_edited(
    name: &str,
    src: &str,
    expect_stdout: &str,
    identified: usize,
) -> (PathBuf, Vec<u8>, Vec<(usize, u32, u64)>) {
    let bin = build_with(name, src, true, true);
    let rec = bin.with_extension("halerec");
    let (stdout, stderr) = run(&bin, "the recorded run", &[("LOTUS_OBS_RECORD", rec.as_os_str())]);
    assert_eq!(stdout, expect_stdout, "recorded run's stdout; stderr:\n{stderr}");
    let buf = std::fs::read(&rec).expect("recording");
    let _ = std::fs::remove_file(&rec);
    let consumes = pool_consumes(&buf, identified);
    (bin, buf, consumes)
}

/// Swap the two recorded consume frames at `a` and `b`, so replay
/// expects the second consume first and holds the first cell.
fn swap_frames(buf: &mut [u8], a: usize, b: usize) {
    let first: Vec<u8> = buf[a..a + 24].to_vec();
    buf.copy_within(b..b + 24, a);
    buf[b..b + 24].copy_from_slice(&first);
}

/// A live run the gate holds keeps its protection and starts at its
/// recorded slot, once: Spawner's run accepts a Kid, whose run is
/// queued, then publishes to itself. The recording's Kid.run and ping
/// consumes (after Spawner's init, birth and run) are swapped, so replay dequeues the live run, holds it
/// behind the ping, and starts it when the ping has been consumed.
#[test]
fn a_held_live_run_keeps_its_ticket_and_starts_at_its_recorded_slot() {
    let src = r#"
type Ping { n: Int = 0; }
locus Kid { run() { println("kid ran"); } }
locus Spawner {
    accept(c: Kid) { }
    bus {
        subscribe "probe.ping" as on_ping of type Ping;
        publish "probe.ping" of type Ping;
    }
    fn on_ping(p: Ping) { println("delivered " + to_string(p.n)); }
    run() { Kid { }; "probe.ping" <- Ping { n: 1 }; }
}
main locus App {
    params { spawner: Spawner = Spawner { }; }
    placement { spawner: cooperative(pool = side); }
}
fn main() { App { }; }
"#;
    let (bin, mut buf, consumes) = record_edited("held_live", src, "kid ran\ndelivered 1\n", 1);
    assert!(
        consumes.len() == 5
            && starts_of_one_locus(&consumes[..3])
            && consumes[3].2 == 0
            && consumes[3].1 != consumes[0].1
            && consumes[4].2 != 0,
        "Spawner init, Spawner birth, Spawner.run, Kid.run, then the ping: {consumes:?}"
    );
    swap_frames(&mut buf, consumes[3].0, consumes[4].0);
    let rec = bin.with_extension("halerec");
    std::fs::write(&rec, &buf).unwrap();

    let stderr = replay_clean(&bin, &rec, "delivered 1\nkid ran\n", "5");
    assert!(
        !stderr.contains("NotStarted"),
        "the held run was started, not canceled:\n{stderr}"
    );
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
}

/// A live run canceled while the gate holds it is ended by the gate,
/// unrun, without a divergence: Spawner's run accepts a flow child
/// whose param Kid queues its run ahead of the flow's. The recording's
/// Kid.run consume is removed, so replay holds the live Kid.run behind
/// the flow's run; that run completes and the flow's reclaim cancels
/// the held run, which the next sweep drops.
#[test]
fn a_run_canceled_while_held_is_dropped_without_a_divergence() {
    let src = r#"
type Ping { n: Int = 0; }
locus Kid { run() { println("kid ran"); } }
locus Flow {
    params { kid: Kid = Kid { }; }
    run() { println("flow ran"); }
}
locus Spawner {
    accept(f: Flow) { }
    release(f: Flow) { }
    bus {
        subscribe "probe.ping" as on_ping of type Ping;
        publish "probe.ping" of type Ping;
    }
    fn on_ping(p: Ping) { println("delivered " + to_string(p.n)); }
    run() { Flow { }; "probe.ping" <- Ping { n: 1 }; }
}
main locus App {
    params { spawner: Spawner = Spawner { }; }
    placement { spawner: cooperative(pool = side); }
}
fn main() { App { }; }
"#;
    let (bin, mut buf, consumes) =
        record_edited("held_canceled", src, "kid ran\nflow ran\ndelivered 1\n", 1);
    // Kid.run consumed at all means it was dequeued ahead of Flow.run,
    // whose completion reclaims Kid.
    assert!(
        consumes.len() == 6
            && starts_of_one_locus(&consumes[..3])
            && consumes[3..5].iter().all(|c| c.2 == 0 && c.1 != consumes[0].1)
            && consumes[3].1 != consumes[4].1
            && consumes[5].2 != 0,
        "Spawner init, Spawner birth, Spawner.run, Kid.run, Flow.run, then the ping: \
         {consumes:?}"
    );
    // Kid.run's consume becomes an entry replay does not index (the
    // recorder's enqueue kind), so the replay expects Flow.run next.
    let kid = consumes[3].0;
    let w0 = u64::from_le_bytes(buf[kid + 8..kid + 16].try_into().unwrap());
    let w0 = (w0 & !(0x1F << 20)) | (3 << 20);
    buf[kid + 8..kid + 16].copy_from_slice(&w0.to_le_bytes());
    let rec = bin.with_extension("halerec");
    std::fs::write(&rec, &buf).unwrap();

    let stderr = replay_clean(&bin, &rec, "flow ran\ndelivered 1\n", "5");
    assert!(
        stderr.contains("NotStarted(Acknowledged)"),
        "the flow's reclaim canceled the held run, and the trace names it:\n{stderr}"
    );
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
}

/// The hold buffer is its consumer thread's, and it is freed when that
/// thread exits: each replay below holds a cell on a thread the
/// teardown then joins, under AddressSanitizer with no suppression, so
/// a buffer its thread took to the exit fails the run as a leak. The
/// classic pool's worker holds in the two tests above; here an async
/// pool's worker holds a live run behind a delivery (the recording's
/// Kid.run and ping consumes, after Spawner's init, birth and run,
/// swapped), and a pinned thread holds one
/// mailbox delivery behind the other (its two consumes swapped). Each
/// replay releases the held cell at its recorded slot and counts no
/// divergence.
#[cfg(target_os = "linux")]
#[test]
fn a_hold_on_an_async_worker_or_a_pinned_thread_is_freed_at_its_teardown() {
    let async_src = r#"
type Ping { n: Int = 0; }
locus Kid { run() { println("kid ran"); } }
locus Spawner {
    accept(c: Kid) { }
    bus {
        subscribe "probe.ping" as on_ping of type Ping;
        publish "probe.ping" of type Ping;
    }
    fn on_ping(p: Ping) { println("delivered " + to_string(p.n)); }
    run() { Kid { }; "probe.ping" <- Ping { n: 1 }; }
}
main locus App {
    params { spawner: Spawner = Spawner { }; }
    placement { spawner: cooperative(pool = side) where async_io; }
}
fn main() { App { }; }
"#;
    let (bin, mut buf, consumes) =
        record_edited("hold_async", async_src, "kid ran\ndelivered 1\n", 1);
    assert!(
        consumes.len() == 5
            && starts_of_one_locus(&consumes[..3])
            && consumes[3].2 == 0
            && consumes[3].1 != consumes[0].1
            && consumes[4].2 != 0,
        "Spawner init, Spawner birth, Spawner.run, Kid.run, then the ping: {consumes:?}"
    );
    swap_frames(&mut buf, consumes[3].0, consumes[4].0);
    let rec = bin.with_extension("halerec");
    std::fs::write(&rec, &buf).unwrap();
    replay_clean(&bin, &rec, "delivered 1\nkid ran\n", "5");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);

    let pinned_src = r#"
type Note { n: Int = 0; }
locus Sink {
    bus {
        subscribe "sink.a" as on_a of type Note;
        subscribe "sink.b" as on_b of type Note;
    }
    fn on_a(x: Note) { println("a " + to_string(x.n)); }
    fn on_b(x: Note) { println("b " + to_string(x.n)); }
}
locus Feeder {
    bus {
        publish "sink.a" of type Note;
        publish "sink.b" of type Note;
    }
    run() { "sink.a" <- Note { n: 1 }; "sink.b" <- Note { n: 2 }; }
}
main locus App {
    params { sink: Sink = Sink { }; feeder: Feeder = Feeder { }; }
    placement { sink: pinned; }
}
fn main() { App { }; }
"#;
    let (bin, mut buf, consumes) = record_edited("hold_pinned", pinned_src, "a 1\nb 2\n", 2);
    let delivered: Vec<usize> = consumes.iter().filter(|c| c.2 != 0).map(|c| c.0).collect();
    assert_eq!(delivered.len(), 2, "the sink's two deliveries: {consumes:?}");
    swap_frames(&mut buf, delivered[0], delivered[1]);
    let rec = bin.with_extension("halerec");
    std::fs::write(&rec, &buf).unwrap();
    replay_clean(&bin, &rec, "b 2\na 1\n", &consumes.len().to_string());
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
}

/// Birth publishes are parked until readiness, which drains the pinned
/// mailbox before its blocking loop. Run publishes followed by yield use
/// that same non-blocking drain. Both must enforce the edited tape,
/// including releasing the first delivery from the hold when the ring
/// becomes empty. Self-publishing makes both paths independent of races
/// between an external publisher and the pinned thread's startup.
#[test]
fn pinned_readiness_and_yield_drains_follow_recorded_order() {
    let src = r#"
type Note { n: Int = 0; }
locus Sink {
    bus {
        subscribe "sink.note" as on_note of type Note;
        publish "sink.note" of type Note;
    }
    fn on_note(x: Note) { println("note " + to_string(x.n)); }
    birth() {
        "sink.note" <- Note { n: 1 };
        "sink.note" <- Note { n: 2 };
        println("queued");
        yield;
        println("yielded");
    }
}
main locus App {
    params { sink: Sink = Sink { }; }
    placement { sink: pinned; }
}
fn main() { App { }; }
"#;
    for phase in ["birth", "run"] {
        let src = src.replace("birth()", &format!("{phase}()"));
        let (recorded, replayed) = if phase == "birth" {
            ("queued\nyielded\nnote 1\nnote 2\n", "queued\nyielded\nnote 2\nnote 1\n")
        } else {
            ("queued\nnote 1\nnote 2\nyielded\n", "queued\nnote 2\nnote 1\nyielded\n")
        };
        let (bin, mut buf, consumes) = record_edited(
            &format!("hold_pinned_{phase}"), &src, recorded, 2,
        );
        assert_eq!(consumes.len(), 2, "the sink's two deliveries: {consumes:?}");
        swap_frames(&mut buf, consumes[0].0, consumes[1].0);
        let rec = bin.with_extension("halerec");
        std::fs::write(&rec, &buf).unwrap();
        replay_clean(&bin, &rec, replayed, "2");
        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_file(&rec);
    }
}
