//! 2026-05-27: spill-to-heap regression. Pre-fix, mailbox /
//! coop-pool / queue cells had a hard `LOTUS_PAYLOAD_MAX = 512`
//! cap and the runtime silently dropped any publish over that
//! size (see the prior `v0 limitation` comments on
//! `lotus_bus_queue_enqueue` / `lotus_mailbox_post` /
//! `lotus_coop_pool_post`). Surfaced by an L2 market-data
//! feed wanting to ship 3.3 KB book snapshots over `udp://`.
//! These tests exercise the spill path: payloads that exceed
//! `LOTUS_PAYLOAD_INLINE` (512 B) flow through a per-cell
//! `malloc` and free-after-dispatch, end-to-end.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_bus_large_payload_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn cooperative_queue_routes_payload_over_inline_threshold() {
    // [Int; 100] = 800 B, comfortably above the 512 B inline
    // threshold, so this publish hits the heap-spill branch in
    // `lotus_bus_queue_enqueue`. The first + last values pin
    // the round-trip: a truncating drop in the spill path
    // would either fail dispatch entirely or surface garbage
    // tail bytes.
    let src = r#"
        type BigPayload {
            counts: [Int; 100] = [0; 100];
            tag:    Int        = 0;
        }
        topic Snapshots { payload: BigPayload; }

        locus Subscriber {
            bus { subscribe Snapshots as h; }
            fn h(m: BigPayload) {
                println("tag=", m.tag);
                println("first=", m.counts[0]);
                println("last=", m.counts[99]);
            }
        }
        locus Publisher {
            bus { publish Snapshots; }
            birth() {
                let mut p = BigPayload { tag: 42 };
                p.counts[0]  = 1234567;
                p.counts[99] = 7654321;
                Snapshots <- p;
            }
        }
        fn main() { Subscriber { }; Publisher { }; }
    "#;
    let (stdout, status) = build_and_run("coop_queue", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("tag=42"), "got: {:?}", stdout);
    assert!(stdout.contains("first=1234567"), "got: {:?}", stdout);
    assert!(stdout.contains("last=7654321"), "got: {:?}", stdout);
}

#[test]
fn pinned_mailbox_routes_payload_over_inline_threshold() {
    // Same shape but the subscriber sits on a pinned OS
    // thread (via `placement { sub: pinned; }`), so the
    // dispatch flows through `lotus_mailbox_post` /
    // `lotus_mailbox_drain_one` rather than the cooperative
    // queue. Distinct heap-spill branch.
    let src = r#"
        type BigPayload {
            counts: [Int; 100] = [0; 100];
            tag:    Int        = 0;
        }
        topic Snapshots { payload: BigPayload; }

        locus Subscriber {
            bus { subscribe Snapshots as h; }
            fn h(m: BigPayload) {
                println("pinned_tag=", m.tag);
                println("pinned_first=", m.counts[0]);
                println("pinned_last=", m.counts[99]);
            }
            run() {
                // Forever-server shape so the pinned thread's
                // mailbox loop sees the publish mid-program
                // rather than at dissolve.
                let mut i = 0;
                while i < 10 {
                    std::time::sleep(10ms);
                    i = i + 1;
                }
            }
        }
        locus Publisher {
            bus { publish Snapshots; }
            run() {
                let mut p = BigPayload { tag: 99 };
                p.counts[0]  = 11;
                p.counts[99] = 999;
                std::time::sleep(5ms);
                Snapshots <- p;
            }
        }

        main locus App {
            params {
                sub: Subscriber = Subscriber { };
                pub: Publisher  = Publisher  { };
            }
            placement {
                sub: pinned;
            }
            run() {
                std::time::sleep(150ms);
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("pinned_mailbox", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("pinned_tag=99"), "got: {:?}", stdout);
    assert!(stdout.contains("pinned_first=11"), "got: {:?}", stdout);
    assert!(stdout.contains("pinned_last=999"), "got: {:?}", stdout);
}
