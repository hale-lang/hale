//! P23 (iris handoff-11) — an intra-subtree publish must not vanish
//! from observation.
//!
//! `desugar_intra_locus_topics` rewrites a `Topic <- payload` whose
//! only subscriber lives inside the publisher's own locus subtree
//! into a direct call to the subscriber's bus handler. Delivery
//! worked; observation did not: no BUS_PUBLISH, no BUS_DELIVER, no
//! counters — and no manifest row at all, so "declared but never
//! published" and "compiled to a direct call" were the same
//! observation (absence). Handoff-5 P17 already ruled on this
//! principle for the devirtualized direct dispatch flavors ("publish
//! once + deliver per matched target with full attribution"); this
//! test pins the same sentence for the AST-level rewrite.
//!
//! The program is iris's controlled pair fused into one binary:
//! `ToChild`'s one subscriber is the publisher's own child (the
//! rewritten flavor), `ToSibling`'s subscriber is a sibling (the
//! bus flavor, the in-program control). Both must register, count
//! pub == dlv == N, and emit attributed ring records. If the
//! desugar's eligibility rules ever change, `to_child` falls back
//! to the bus path and these assertions still hold — the contract
//! is flavor-independent by design.
//!
//! (iris's original repro also showed the sibling at dlv 0 — that
//! is the documented birth-order trap, not P23: their `Sib` was
//! declared AFTER the publisher whose `run()` never returns. `Sib`
//! is declared first here.)

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

#[path = "support/obs.rs"]
mod obs;
use obs::{obs_bus_locus, records, topic_counters, topic_id_and_line};

const EK_BUS_PUBLISH: u32 = 1;
const EK_BUS_DELIVER: u32 = 2;

const PAIR: &str = r#"
    type P { v: Int = 0; }

    topic ToChild { payload: P; }
    topic ToSibling { payload: P; }

    locus Kid {
        params { got: Int = 0; }
        bus { subscribe ToChild as on_p; }
        fn on_p(p: P) { self.got = self.got + 1; }
    }

    locus Sib {
        params { got: Int = 0; }
        bus { subscribe ToSibling as on_p; }
        fn on_p(p: P) { self.got = self.got + 1; }
    }

    locus Pub {
        params { k: Kid = Kid { }; }
        bus { publish ToChild; publish ToSibling; }
        run() {
            std::time::sleep(400ms);
            let mut i = 0;
            while i < 25 {
                ToChild <- P { v: i };
                ToSibling <- P { v: i };
                i = i + 1;
            }
            println("kid_got=", to_string(self.k.got));
        }
    }

    main locus App {
        params { s: Sib = Sib { }; p: Pub = Pub { }; }
        placement { p: pinned(core = 0); }
        run() { std::time::sleep(1200ms); }
    }

    fn main() { App { }; }
"#;

#[test]
fn intra_tree_publish_registers_counts_and_attributes() {
    let program = hale_syntax::parse_source(PAIR).expect("parse");
    let bin = harness::unique_bin("hale_test_obs_intra_tree");
    build_executable(&program, &bin).expect("build");

    let child = Command::new(&bin)
        .env("LOTUS_OBS", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();

    // Attach as an observer before the publish burst (t+400ms) so
    // ring records are emitted; hold a live mapping across exit so
    // the final counters are readable after teardown unlinks.
    let mut seg: Option<&'static [u8]> = None;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if std::fs::metadata(format!("/dev/shm/hale-obs-{}", pid)).is_ok() {
            obs::attach_observer(pid);
            seg = obs::map_shm(pid);
            break;
        }
    }
    let seg = seg.expect("obs segment never appeared");

    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "pair exited nonzero");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("kid_got=25"),
        "delivery itself must still work: {}",
        stdout
    );

    // 1. The rewritten topic REGISTERS — the exact observation P23
    // is about. Before the fix there was no `to_child` row at all.
    let (child_id, _) = topic_id_and_line(seg, b"ToChild")
        .expect("to_child has a manifest row (P23: it had none)");
    let (sib_id, _) = topic_id_and_line(seg, b"ToSibling")
        .expect("to_sibling has a manifest row (control)");

    // 2. Counters match the control: pub == dlv == 25 on both.
    let (c_pub, c_dlv, c_bytes) =
        topic_counters(seg, b"ToChild").expect("to_child counters");
    let (s_pub, s_dlv, _) =
        topic_counters(seg, b"ToSibling").expect("to_sibling counters");
    assert_eq!(c_pub, 25, "to_child published");
    assert_eq!(c_dlv, 25, "to_child delivered (the direct call IS the delivery)");
    assert!(c_bytes > 0, "to_child payload bytes counted");
    assert_eq!(s_pub, 25, "control published");
    assert_eq!(s_dlv, 25, "control delivered");

    // 3. Ring records with locus attribution, same as every other
    // flavor: publishes attribute the publisher, delivers the
    // subscriber.
    let pubs: Vec<u64> = records(seg, EK_BUS_PUBLISH)
        .into_iter()
        .filter(|(id, _)| *id == child_id)
        .map(|(_, w1)| w1)
        .collect();
    let dlvs: Vec<u64> = records(seg, EK_BUS_DELIVER)
        .into_iter()
        .filter(|(id, _)| *id == child_id)
        .map(|(_, w1)| w1)
        .collect();
    assert!(
        !pubs.is_empty(),
        "BUS_PUBLISH records exist for to_child"
    );
    assert!(
        !dlvs.is_empty(),
        "BUS_DELIVER records exist for to_child"
    );
    assert!(
        pubs.iter().any(|w1| obs_bus_locus(*w1) != 0),
        "publish records attribute the publisher locus"
    );
    assert!(
        dlvs.iter().any(|w1| obs_bus_locus(*w1) != 0),
        "deliver records attribute the subscriber locus"
    );
    let _ = sib_id;
    let _ = std::fs::remove_file(&bin);
}
