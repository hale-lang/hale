//! RFC #296 Phase 0 — single-pool execution is deterministic by
//! construction. This test converts that implementation accident
//! into a stated, pinned guarantee.
//!
//! The claim (spec/testing.md § Determinism): a program whose loci
//! all run on the main cooperative scheduler — no `placement`, no
//! extra pools — produces the same publishes and the same
//! deliveries in the same order on every run, given the same
//! inputs. The cooperative pool is a single consumer thread by
//! construction (spec/runtime.md — the invariant devirtualization
//! rests on), so the delivery order not only exists, it cannot
//! vary.
//!
//! Pinning method: run the same multi-locus cascade program several
//! times under LOTUS_OBS=1 with an attached observer, and compare
//! the complete ordered sequence of BUS_PUBLISH / BUS_DELIVER
//! records across runs. Record timestamps differ run to run; the
//! (subject, kind, locus, seq) tuple sequence must not.
//!
//! This is deliberately an obs-record test in Rust (not a
//! `*_test.hl`): the assertion is about the runtime's event stream,
//! which spec/testing.md files under compiler/runtime output.

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

#[path = "support/obs.rs"]
mod obs;
use obs::{obs_bus_locus, read_u32, read_u64};

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_replay_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// A single-pool cascade: seeds fan out into a re-publishing locus
/// and two sinks, one of which subscribes to both stages. No
/// placement, no pools — everything on the main scheduler. No
/// time/entropy dependence in any handler (the one sleep runs
/// before the burst, purely so the test can attach its observer).
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
            std::time::sleep(600ms);
            let mut i = 0;
            while i < 32 {
                "det.seed" <- Seed { n: i };
                i = i + 1;
            }
        }
    }

    fn main() { App { }; }
"#;

/// Publishes: 32 seeds + 64 derived. Deliveries: seeds → Fan +
/// SinkB (64), derived → SinkA + SinkB (128).
const EXPECT_PUBLISH: usize = 32 + 64;
const EXPECT_DELIVER: usize = 64 + 128;

const EKIND_BUS_PUBLISH: u32 = 1;
const EKIND_BUS_DELIVER: u32 = 2;

/// One observed bus event, with everything nondeterministic
/// (timestamps) stripped: subject name instead of the per-run
/// manifest id, event kind, attributed locus, per-publisher seq.
#[derive(PartialEq, Eq, Debug, Clone)]
struct BusEvent {
    subject: String,
    ekind: u32,
    locus: u32,
    seq: u64,
}

/// Map manifest topic ids (registration-order, per-run) back to
/// subject names so run-to-run comparison does not depend on
/// registration order being stable — that stability is part of
/// what determinism buys, but the sequence comparison should not
/// assume the thing it is proving.
fn topic_names(seg: &[u8]) -> std::collections::HashMap<u32, String> {
    let manifest_off = read_u64(seg, 0x40) as usize;
    let entry_count = read_u32(seg, manifest_off) as usize;
    let pool_off = read_u32(seg, manifest_off + 8) as usize;
    let entries = manifest_off + 16;
    let mut out = std::collections::HashMap::new();
    for i in 0..entry_count {
        let e = entries + i * 32;
        if seg[e + 28] != 0 {
            continue; // not a topic row
        }
        let id = read_u32(seg, e);
        let name_off = read_u32(seg, e + 20) as usize;
        let name_len =
            seg[e + 24] as usize | ((seg[e + 25] as usize) << 8);
        let name = &seg[manifest_off + pool_off + name_off
            ..manifest_off + pool_off + name_off + name_len];
        out.insert(id, String::from_utf8_lossy(name).into_owned());
    }
    out
}

/// The complete ordered bus-event sequence of one run, and the
/// number of rings that carried bus records.
fn one_run(bin: &PathBuf) -> (Vec<BusEvent>, usize) {
    let mut child = Command::new(bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let pid = child.id();

    // Attach before the publish burst (which starts at t+600ms):
    // poll for the lazily-created segment, then bump
    // observer_count so ring emission is on for every record.
    let mut attached = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        if std::fs::metadata(format!("/dev/shm/hale-obs-{}", pid)).is_ok() {
            obs::attach_observer(pid);
            attached = true;
            break;
        }
    }
    if !attached {
        let _ = child.kill();
        panic!("obs segment for pid {} never appeared", pid);
    }
    // Leaked mapping stays valid past emitter exit (POSIX shm).
    let seg = obs::map_shm(pid).expect("map obs segment");
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "cascade exited nonzero");

    let names = topic_names(seg);
    let rings_off = read_u64(seg, 0x68) as usize;
    let ring_count = read_u32(seg, 0x1C) as usize;
    let ring_slots = read_u32(seg, 0x20) as usize;

    let mut events = Vec::new();
    let mut rings_with_bus = 0usize;
    for r in 0..ring_count {
        let rdesc = rings_off + r * 64;
        let data_off = read_u64(seg, rdesc) as usize;
        let head = read_u64(seg, rdesc + 8) as usize;
        let dropped = read_u64(seg, rdesc + 16);
        assert_eq!(dropped, 0, "ring {} dropped records", r);
        assert!(
            head <= ring_slots,
            "ring {} wrapped ({} records into {} slots) — the \
             sequence is no longer complete and the comparison \
             would be vacuous",
            r,
            head,
            ring_slots
        );
        let before = events.len();
        for i in 0..head {
            let slot = data_off + (i & (ring_slots - 1)) * 16;
            let w0 = read_u64(seg, slot);
            let w1 = read_u64(seg, slot + 8);
            let id = (w0 & 0xFFFFF) as u32;
            let ekind = ((w0 >> 20) & 0x1F) as u32;
            if ekind != EKIND_BUS_PUBLISH && ekind != EKIND_BUS_DELIVER {
                continue;
            }
            events.push(BusEvent {
                subject: names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("<unknown-{}>", id)),
                ekind,
                locus: obs_bus_locus(w1),
                seq: w1 & 0xFFF_FFFF_FFFF, // seq:44 low
            });
        }
        if events.len() > before {
            rings_with_bus += 1;
        }
    }
    (events, rings_with_bus)
}

#[test]
fn single_pool_bus_order_is_identical_across_runs() {
    let bin = build("phase0", CASCADE);

    let (baseline, baseline_rings) = one_run(&bin);

    // Negative control for the comparison itself (the P20 lesson:
    // a recorder that captured nothing passes any equality check).
    // The event counts must match the program's static shape.
    let publishes =
        baseline.iter().filter(|e| e.ekind == EKIND_BUS_PUBLISH).count();
    let delivers =
        baseline.iter().filter(|e| e.ekind == EKIND_BUS_DELIVER).count();
    assert_eq!(publishes, EXPECT_PUBLISH, "publish records incomplete");
    assert_eq!(delivers, EXPECT_DELIVER, "deliver records incomplete");

    // Single-pool: one consumer thread, so one ring carries the
    // entire bus stream. This is the structural fact the ordering
    // guarantee stands on — if it breaks, the claim breaks.
    assert_eq!(
        baseline_rings, 1,
        "expected the whole bus stream on the main thread's ring"
    );

    for run in 1..3 {
        let (events, rings) = one_run(&bin);
        assert_eq!(rings, 1);
        assert_eq!(
            events, baseline,
            "run {} diverged from run 0 — single-pool execution \
             is documented deterministic (spec/testing.md § \
             Determinism); a divergence here is a runtime bug, \
             not a flaky test",
            run
        );
    }

    let _ = std::fs::remove_file(&bin);
}
