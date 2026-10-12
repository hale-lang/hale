//! A String payload delivered to a long-lived subscriber on the thread
//! that published it leaves nothing in the subscriber's arena.
//!
//! A non-flat payload (one with String or Bytes fields) is serialized at
//! the publish and deserialized for each subscriber. A subscriber on
//! another thread had its cell materialized at drain into a region of the
//! delivery's own, gone when the handler returned
//! (`bus_async_wire_payload_reclaim.rs`); a subscriber on the publisher's
//! own thread was handed the struct at the publish, deserialized into its
//! LOCUS arena, so every delivery's owned bytes stayed there for the
//! subscriber's life: 1 KiB a message here, and the TCP substrate's
//! consumer ~34 B a connection (the listener's name and route, both on the
//! `Accepted` handle its listener on the same pool relays). Every
//! non-flat delivery now takes the one shape, whichever thread.
//!
//! The oracle is the subscriber's own arena, to the byte: the residency
//! dump's `used=` (`LOTUS_ARENA_RESIDENCY=1`) after a warm-up and again
//! after many more deliveries. A handler that keeps a payload field copies
//! it into the locus, so the field kept from the first delivery still
//! reads back whole after its region is gone (with the chunk pool off, so
//! a freed region's bytes are not handed out again intact).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const WARM: usize = 1000;
const MORE: usize = 20_000;

// A sink subscribed to an unkeyed subject and to a keyed topic, and a
// flood publishing to both: on main (the program's queue) or together on
// one async_io pool (its worker), the publisher and the subscriber always
// on one thread.
const FLOOD: &str = r#"
    type Payload { seq: Int; body: String; }
    topic KeyedT { payload: Payload; keyed_by seq; }
    locus Sink {
        params { got: Int = 0; kgot: Int = 0; kept: String = ""; }
        bus {
            subscribe "flood" as on_msg of type Payload;
            subscribe KeyedT as on_key where key == 7;
        }
        fn on_msg(m: Payload) {
            if self.got == 0 {
                self.kept = m.body;
            }
            self.got = self.got + 1;
        }
        fn on_key(m: Payload) {
            self.kgot = self.kgot + len(m.body);
        }
    }
    locus Flood {
        params { }
        bus {
            publish "flood" of type Payload;
            publish KeyedT;
        }
        fn go(n: Int) {
            // heap bytes, built at run time: a literal would sit in static
            // memory, and a dangling read of it would still read back
            let body = std::str::repeat("ab", 512);
            let mut i = 0;
            while i < n {
                "flood" <- Payload { seq: i, body: body };
                KeyedT <- Payload { seq: 7, body: body };
                i = i + 1;
                if i - (i / 100) * 100 == 0 {
                    std::time::sleep(1ms);
                }
            }
            std::time::sleep(50ms);
            println("mark");
            let _ = std::process::dump_arena_residency();
        }
    }
    RUNNER
"#;

const ON_MAIN: &str = r#"
    main locus App {
        params { sink: Sink = Sink { }; flood: Flood = Flood { }; }
        run() {
            self.flood.go(WARM);
            self.flood.go(MORE);
            println("kept ", self.sink.kept == std::str::repeat("ab", 512), " got ", self.sink.got, " kgot ", self.sink.kgot);
        }
    }
    fn main() { App { }; }
"#;

// On an async_io pool the same pair, children of one driver there.
const ON_POOL: &str = r#"
    locus Driver {
        params { sink: Sink = Sink { }; flood: Flood = Flood { }; }
        run() {
            self.flood.go(WARM);
            self.flood.go(MORE);
            println("kept ", self.sink.kept == std::str::repeat("ab", 512), " got ", self.sink.got, " kgot ", self.sink.kgot);
            std::process::exit(0);
        }
    }
    main locus App {
        params { driver: Driver = Driver { }; }
        placement { driver: cooperative(pool = p) where async_io; }
        run() { std::time::sleep(60s); }
    }
    fn main() { App { }; }
"#;

/// The sink's `used=` bytes in each of the run's dumps.
fn sink_used(stderr: &str) -> Vec<usize> {
    stderr
        .split("[arena_residency dump]")
        .skip(1)
        .map(|d| {
            d.lines()
                .filter(|l| l.contains("label=Sink]"))
                .filter_map(|l| l.rsplit("used=").next()?.trim().parse::<usize>().ok())
                .sum()
        })
        .collect()
}

fn flood_case(tag: &str, runner: &str) {
    let src = FLOOD
        .replace("RUNNER", runner)
        .replace("WARM", &WARM.to_string())
        .replace("MORE", &MORE.to_string());
    let bin = harness::unique_bin(tag);
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{tag}: {:?}\n{stdout}\n{stderr}", out.status);
    let report = stdout.lines().find(|l| l.starts_with("kept ")).unwrap_or_else(|| panic!("{tag}: no report:\n{stdout}"));
    assert_eq!(
        report,
        format!("kept true got {} kgot {}", WARM + MORE, (WARM + MORE) * 1024).as_str(),
        "{tag}: every delivery handled, and the field kept from the first reads back whole"
    );
    let used = sink_used(&stderr);
    // a dump after the warm-up, one after the rest (and the one at exit)
    assert!(used.len() >= 2, "{tag}: a dump after the warm-up and one after the rest:\n{stderr}");
    let grew = used[1].saturating_sub(used[0]);
    // 20,000 more deliveries of 1 KiB bodies on two subscriptions: the
    // deposit was 2 KiB each (~40 MB); flat is no growth at all, and a byte
    // a delivery would still say the payload landed in the locus
    assert!(
        grew < MORE,
        "{tag}: the sink's arena grew {grew} B over {MORE} deliveries ({} -> {}): a same-thread delivery's payload \
         is staying in the subscriber's arena",
        used[0],
        used[1]
    );
}

#[test]
fn a_same_thread_subscriber_on_main_keeps_no_payload() {
    flood_case("bus_same_thread_main", ON_MAIN);
}

#[test]
fn a_same_thread_subscriber_on_an_async_io_pool_keeps_no_payload() {
    flood_case("bus_same_thread_pool", ON_POOL);
}

/// Under AddressSanitizer, the chunk pool off: the region a delivery's
/// payload lives in is freed as its handler returns, and nothing — the
/// field the sink keeps, the keyed and unkeyed handlers — reads it after.
#[test]
fn a_same_thread_delivery_is_clean_under_asan() {
    let src = FLOOD.replace("RUNNER", ON_MAIN).replace("WARM", "50").replace("MORE", "500");
    let bin = harness::unique_bin("bus_same_thread_asan");
    harness::build_source_asan(&src, &bin);
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert!(out.status.success(), "{:?}\n{stderr}", out.status);
    assert!(stdout.contains("kept true got 550 kgot 563200"), "{stdout}");
}
