//! One publisher's order holds across topics (spec/runtime.md § "Bus
//! message router", "One publisher's order"; friction sweep 2026-10-12,
//! downstream handoff). Devirtualization is invisible: a publish the
//! compiler turns into a direct handler call runs at once only when
//! nothing this thread sent earlier still waits on its own queue;
//! otherwise it is queued behind that cell, as the bus would have
//! queued it. Before, `TA <- a; TB <- b;` ran TB's handler first when TB
//! was devirtualized and TA was not.
//!
//! Each program is also built with every devirtualization off
//! (`BuildOptions::no_bus_devirt`, the reference lowering): the two runs
//! print the same lines, in the same order.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(tag: &str, src: &str, devirt: bool) -> String {
    let bin = harness::unique_bin(&format!("bus_order_{tag}_{devirt}"));
    let opts = hale_codegen::BuildOptions { no_bus_devirt: !devirt, ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{tag}: {:?}\n{stdout}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    stdout
}

fn same_with_and_without_devirt(tag: &str, src: &str) -> String {
    let devirt = run(tag, src, true);
    let reference = run(tag, src, false);
    assert_eq!(devirt, reference, "{tag}: the devirtualized program's order is not the bus's");
    devirt
}

/// `TB`'s one subscriber is the publisher's only field of its type, so
/// the send is rewritten into a direct call (the intra-tower rewrite);
/// `TA` has three subscribers and is queued. TA was sent first.
const REWRITTEN_AFTER_QUEUED: &str = r#"
    type A { n: Int; }
    type B { n: Int; }
    topic TA { payload: A; }
    topic TB { payload: B; }
    locus SubA { params { id: Int = 0; } bus { subscribe TA as on_a; } fn on_a(a: A) { println("sub" + to_string(self.id) + " got A"); } }
    locus Coord { params { x: Int = 0; } bus { subscribe TB as on_b; } fn on_b(b: B) { println("coord got B"); } }
    main locus App {
        params { a1: SubA = SubA { id: 1 }; a2: SubA = SubA { id: 2 }; a3: SubA = SubA { id: 3 }; c: Coord = Coord { }; }
        bus { publish TA; publish TB; }
        run() {
            TA <- A { n: 1 };
            TB <- B { n: 1 };
            yield;
            TB <- B { n: 2 };
            println("end");
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn a_rewritten_send_waits_behind_a_queued_one() {
    let out = same_with_and_without_devirt("rewritten", REWRITTEN_AFTER_QUEUED);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[..4],
        ["sub1 got A", "sub2 got A", "sub3 got A", "coord got B"],
        "TA was published first:\n{out}"
    );
    // Nothing waits after the yield's drain: the second send is the
    // direct call again, so its handler runs at the publish.
    assert_eq!(lines[4..], ["coord got B", "end"], "{out}");
}

/// `Tally.on_set` is quiet and its payload flat, so `Set` is lowered as
/// a direct call (two instances: no rewrite); `Show` prints and is
/// queued. A `Show` sent before a `Set` reads the tally as it was before
/// the `Set`, in the queued order, and one sent after reads it after.
const DIRECT_AFTER_QUEUED: &str = r#"
    type Set { n: Int; }
    type Show { tag: Int; }
    topic SetT { payload: Set; }
    topic ShowT { payload: Show; }
    locus Tally {
        params { id: Int = 0; last: Int = 0; }
        bus { subscribe SetT as on_set; subscribe ShowT as on_show; }
        fn on_set(s: Set) { self.last = s.n; }
        fn on_show(s: Show) { println("show " + to_string(s.tag) + " tally" + to_string(self.id) + " last=" + to_string(self.last)); }
    }
    main locus App {
        params { t1: Tally = Tally { id: 1 }; t2: Tally = Tally { id: 2 }; }
        bus { publish SetT; publish ShowT; }
        run() {
            ShowT <- Show { tag: 1 };
            SetT <- Set { n: 7 };
            ShowT <- Show { tag: 2 };
            yield;
            SetT <- Set { n: 9 };
            ShowT <- Show { tag: 3 };
            yield;
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn a_direct_call_waits_behind_a_queued_cell_for_its_subscriber() {
    let out = same_with_and_without_devirt("direct", DIRECT_AFTER_QUEUED);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "show 1 tally1 last=0",
            "show 1 tally2 last=0",
            "show 2 tally1 last=7",
            "show 2 tally2 last=7",
            "show 3 tally1 last=9",
            "show 3 tally2 last=9",
        ],
        "{out}"
    );
}
