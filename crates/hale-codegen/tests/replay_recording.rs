//! RFC #296 Phase 1 — lossless recording mode (`LOTUS_OBS_RECORD`).
//!
//! What Phase 1 promises, and what these tests hold it to:
//!
//!   1. **Never drop.** Under recording, ring emission blocks the
//!      producer against the drain cursor instead of overwriting
//!      oldest; every record published before teardown reaches the
//!      file, verified by exact counts under wrap pressure
//!      (LOTUS_OBS_SLOTS far below the record volume).
//!   2. **Per-consumer order.** Queued deliveries get a consume
//!      record (private recorder namespace) on the consuming thread at
//!      handler invoke — the order replay reconstructs. Enqueue-time
//!      BUS_DELIVER can't give it (it lands on the publisher's
//!      ring).
//!   3. **Opt-in only.** Without LOTUS_OBS_RECORD there is no file,
//!      no consume record, and no disposition change — plain
//!      LOTUS_OBS observation streams are byte-compatible with
//!      pre-recording consumers.
//!
//! The counts are exact equalities, never `> 0` — the P20 lesson
//! from the RFC thread: a recorder that captured nothing passes any
//! vacuous check the first time a probe goes dark.

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

#[path = "support/obs.rs"]
mod obs;
use obs::rec_id_ekind;

const EK_BUS_PUBLISH: u32 = 1;
const EK_BUS_DELIVER: u32 = 2;
/// Recorder events ride PRIVATE rings (high bit set on the file
/// entry's ring field) in their own namespace — never iris ekinds.
const REC_EV_CONSUME: u32 = 2;
const PRIV_RING: u32 = 0x8000_0000;

fn is_consume(ring: u32, w0: u64) -> bool {
    ring & PRIV_RING != 0 && ((w0 >> 20) & 0x1F) as u32 == REC_EV_CONSUME
}
fn is_public_bus(ring: u32, w0: u64, want: u32) -> bool {
    ring & PRIV_RING == 0 && ((w0 >> 20) & 0x1F) as u32 == want
}

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_rec_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn rec_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale_rec_{}_{}.halerec",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&p);
    p
}

/// Same single-pool cascade as replay_determinism.rs, minus the
/// attach-window sleep: a recording is an observer from the first
/// probe, so nothing needs to attach. 32 seeds → 96 publishes,
/// 192 deliveries, all synchronous direct dispatch on the main
/// thread.
const CASCADE: &str = r#"
    type Seed { n: Int = 0; }
    type Derived { n: Int = 0; }

    locus Fan {
        params { seen: Int = 0; }
        bus {
            subscribe "det.seed" as on_seed of type Seed;
            publish "det.derived" of type Derived;
        }
        fn on_seed(s: Seed) {
            self.seen = self.seen + 1;
            "det.derived" <- Derived { n: s.n * 2 };
            "det.derived" <- Derived { n: s.n * 2 + 1 };
        }
    }

    locus SinkA {
        params { total: Int = 0; }
        bus { subscribe "det.derived" as on_d of type Derived; }
        fn on_d(d: Derived) { self.total = self.total + d.n; }
    }

    locus SinkB {
        params { count: Int = 0; }
        bus {
            subscribe "det.seed" as on_s of type Seed;
            subscribe "det.derived" as on_d of type Derived;
        }
        fn on_s(s: Seed) { self.count = self.count + 1; }
        fn on_d(d: Derived) { self.count = self.count + 1; }
    }

    main locus App {
        params {
            f: Fan = Fan { };
            a: SinkA = SinkA { };
            b: SinkB = SinkB { };
        }
        bus { publish "det.seed" of type Seed; }
        run() {
            let mut i = 0;
            while i < 32 {
                "det.seed" <- Seed { n: i };
                i = i + 1;
            }
        }
    }

    fn main() { App { }; }
"#;

const CASCADE_PUBLISH: usize = 32 + 64;
const CASCADE_DELIVER: usize = 64 + 128;

/// A pinned publisher bursting into a main-tree subscriber: every
/// delivery crosses a thread boundary into the main bus queue, so
/// every one is dequeue-driven — 50 consume records on the
/// consumer's ring.
const PINNED_PUB: &str = r#"
    type Tick { n: Int = 0; }

    locus Sink {
        params { seen: Int = 0; }
        bus { subscribe "rec.tick" as on_t of type Tick; }
        fn on_t(t: Tick) { self.seen = self.seen + 1; }
    }

    locus Pub {
        bus { publish "rec.tick" of type Tick; }
        run() {
            std::time::sleep(400ms);
            let mut i = 0;
            while i < 50 {
                "rec.tick" <- Tick { n: i };
                i = i + 1;
            }
        }
    }

    main locus App {
        params { s: Sink = Sink { }; p: Pub = Pub { }; }
        placement { p: pinned(core = 0); }
        run() { std::time::sleep(1300ms); }
    }

    fn main() { App { }; }
"#;

fn run_recorded(bin: &PathBuf, rec: &PathBuf, extra: &[(&str, &str)]) {
    let mut cmd = Command::new(bin);
    cmd.env("LOTUS_OBS_RECORD", rec)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run recorded");
    assert!(
        out.status.success(),
        "recorded run exited nonzero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn recording_is_lossless_under_wrap_pressure() {
    let bin = build("lossless", CASCADE);
    let rec = rec_path("lossless");

    // 64-slot rings against ~300 records from one thread: without
    // the blocking disposition the ring wraps five times and the
    // oldest records are gone. The file must still hold every one.
    run_recorded(&bin, &rec, &[("LOTUS_OBS_SLOTS", "64")]);

    let r = obs::read_recording(&rec).expect("parse recording");
    assert!(r.clean, "no trailer — the recording did not finalize");
    assert_eq!(
        r.trailer_count,
        (r.entries.len() + r.payloads.len() + r.journal.len()
            + r.meta_entries) as u64,
        "trailer count disagrees with the entries on disk"
    );
    assert_eq!(r.ring_slots, 64);
    // Every capture here is an in-process struct: raw flag set, no
    // ingress, and METADATA ONLY on disk — an ABI snapshot would
    // carry heap pointers and padding (review round 2, finding 8).
    assert!(
        r.payloads.iter().all(|p| p.flags & 2 != 0
            && p.flags & 1 == 0
            && p.bytes.is_empty()
            && p.raw_size > 0),
        "raw struct captures must be metadata-only"
    );
    // Phase 2: one payload blob per QUEUED publish (the 32 seeds —
    // App's publishes route through the arena fanout). Fan's 64
    // derived publishes take the synchronous intra-tree desugar,
    // which deliberately captures nothing: a closed-world
    // same-thread call cannot carry external input, and replay
    // re-derives its payloads by re-execution. None marked ingress.
    assert_eq!(r.payloads.len(), 32, "queued-publish payloads lost");


    let publishes = r
        .entries
        .iter()
        .filter(|(ring, w0, _)| is_public_bus(*ring, *w0, EK_BUS_PUBLISH))
        .count();
    let delivers = r
        .entries
        .iter()
        .filter(|(ring, w0, _)| is_public_bus(*ring, *w0, EK_BUS_DELIVER))
        .count();
    assert_eq!(publishes, CASCADE_PUBLISH, "publish records lost");
    assert_eq!(delivers, CASCADE_DELIVER, "deliver records lost");

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
}

#[test]
fn queued_deliveries_get_consume_records_in_consumer_order() {
    let bin = build("consume", PINNED_PUB);
    let rec = rec_path("consume");
    run_recorded(&bin, &rec, &[]);

    let r = obs::read_recording(&rec).expect("parse recording");
    assert!(r.clean, "no trailer — the recording did not finalize");

    let consumes: Vec<_> = r
        .entries
        .iter()
        .filter(|(ring, w0, _)| is_consume(*ring, *w0))
        .collect();
    assert_eq!(
        consumes.len(),
        50,
        "every cross-thread delivery is dequeue-driven and must \
         stamp exactly one consume record"
    );

    // All on one ring (the single consumer thread), carrying full
    // 64-bit message ids (consumer:16 | seq:48) in gapless
    // per-publisher order, with the TARGET locus in w0's id field —
    // the (locus, msg_id) pair is the delivery identity.
    let ring0 = consumes[0].0;
    let mut last_seq = 0u64;
    for (ring, w0, w1) in consumes.iter() {
        assert_eq!(
            *ring, ring0,
            "consume records spread across rings — not one consumer"
        );
        assert_ne!(*w1, 0, "queued delivery lost its identity");
        let seq = *w1 & 0xFFFF_FFFF_FFFF; // low 48 = per-thread seq
        assert_eq!(
            seq,
            last_seq + 1,
            "one publisher, FIFO queue: consume order must be \
             the gapless publish order"
        );
        last_seq = seq;
        assert_ne!(
            (*w0 & 0xFFFFF) as u32,
            0,
            "consume record lost its target-locus identity"
        );
    }

    // The queued deliveries themselves are also all present.
    let delivers = r
        .entries
        .iter()
        .filter(|(ring, w0, _)| is_public_bus(*ring, *w0, EK_BUS_DELIVER))
        .count();
    assert_eq!(delivers, 50, "deliver records lost");

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
}

#[test]
fn two_recordings_of_a_single_pool_program_are_identical() {
    let bin = build("determ", CASCADE);
    let rec_a = rec_path("determ_a");
    let rec_b = rec_path("determ_b");
    run_recorded(&bin, &rec_a, &[]);
    run_recorded(&bin, &rec_b, &[]);

    // Compare the bus-event sequences (publish/deliver/consume),
    // stripped of timestamps. All bus records of a single-pool
    // program sit on the main thread's ring, and the drain
    // preserves per-ring order, so the filtered file order is the
    // execution order.
    let strip = |p: &PathBuf| -> Vec<(u32, u32, u64)> {
        obs::read_recording(p)
            .expect("parse recording")
            .entries
            .iter()
            .filter_map(|(ring, w0, w1)| {
                let (id, ekind) = rec_id_ekind(*w0);
                if is_public_bus(*ring, *w0, EK_BUS_PUBLISH)
                    || is_public_bus(*ring, *w0, EK_BUS_DELIVER)
                {
                    Some((id, ekind, *w1))
                } else if is_consume(*ring, *w0) {
                    Some((id, 100 + ekind, *w1))
                } else {
                    None
                }
            })
            .collect()
    };
    let a = strip(&rec_a);
    let b = strip(&rec_b);
    // Every publish and delivery, plus one consume per QUEUED
    // delivery (how many are queued vs synchronous-direct is the
    // dispatch planner's business — but it must be deterministic,
    // which the a == b below holds it to).
    let consumes = a
        .iter()
        .filter(|(_, ekind, _)| *ekind == 100 + REC_EV_CONSUME)
        .count();
    assert_eq!(
        a.len(),
        CASCADE_PUBLISH + CASCADE_DELIVER + consumes,
        "recording incomplete"
    );
    assert_eq!(
        a, b,
        "two recordings of a single-pool program diverged — \
         spec/testing.md § Determinism is violated"
    );

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec_a);
    let _ = std::fs::remove_file(&rec_b);
}

#[test]
fn plain_observation_never_emits_consume_records() {
    // The negative control for opt-in: LOTUS_OBS=1 without
    // LOTUS_OBS_RECORD must produce a stream a pre-addendum
    // consumer can read — ekind 8 must not appear.
    let bin = build("optin", PINNED_PUB);
    let child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        if std::fs::metadata(format!("/dev/shm/hale-obs-{}", pid)).is_ok() {
            obs::attach_observer(pid);
            break;
        }
    }
    let seg = obs::map_shm(pid).expect("map segment");
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success());
    // ekind 8 in the PUBLIC segment is iris SUPERV_TRANS; the
    // recorder must never emit anything of its own there (its
    // events live on private rings that exist only in the file).
    let stray = obs::records(seg, 8);
    assert!(
        stray.is_empty(),
        "recorder bookkeeping leaked into the public protocol stream"
    );
    // ...while the deliveries themselves demonstrably happened
    // (the vacuity guard for the assertion above).
    let delivers = obs::records(seg, EK_BUS_DELIVER);
    assert!(!delivers.is_empty(), "no deliveries — vacuous control");
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn ring_exhaustion_fails_a_recorded_run_loudly() {
    // One ring, two emitting threads (main + the pinned publisher):
    // the second thread cannot get a ring, and a recording must
    // never silently drop a thread's records — the run fails with
    // the documented diagnostic instead.
    let bin = build("exhaust", PINNED_PUB);
    let rec = rec_path("exhaust");
    let out = Command::new(&bin)
        .env("LOTUS_OBS_RECORD", &rec)
        .env("LOTUS_OBS_RINGS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run");
    assert_eq!(
        out.status.code(),
        Some(70),
        "expected the ring-exhaustion failure, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("LOTUS_OBS_RECORD") && stderr.contains("LOTUS_OBS_RINGS"),
        "diagnostic must name the knob that fixes it:\n{}",
        stderr
    );
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&rec);
}
