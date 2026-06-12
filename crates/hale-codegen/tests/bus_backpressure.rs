//! GH #125 — bounded bus queue + backpressure.
//!
//! A producer that floods the cooperative bus (publishes a large batch
//! before the consumer drains) must not grow resident memory without
//! bound. Pre-#125 the queue doubled on every fill, so a `birth()` that
//! published 2M messages buffered the whole backlog (~1 GB, 2M cells ×
//! ~552 B). Now the single-threaded publisher BLOCKS once the queue hits
//! the cap — it inline-drains the queue (runs the oldest handlers) to make
//! space — so resident memory stays bounded while every message is still
//! delivered.

use std::process::Command;

use hale_codegen::build_executable;

/// Build, run, and read `final_rss_mb=` from stdout (MB). Panics if the
/// program crashes or never prints the line — which also asserts the flood
/// ran to completion (the line is only printed once the count reaches N).
fn build_and_rss(name: &str, src: &str) -> i64 {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_bus_bp_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(output.status.success(), "{} crashed: {:?}", name, output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|l| l.starts_with("final_rss_mb="))
        .and_then(|l| l.trim_start_matches("final_rss_mb=").trim().parse().ok())
        .unwrap_or_else(|| panic!("no final_rss_mb in {} stdout: {:?}", name, stdout))
}

/// A `Source` whose `birth()` publishes 2M ticks before yielding — the
/// flooding producer. The `Sink` counts them and prints RSS once it has
/// seen all N (so the print also proves none were lost).
const FLOOD: &str = r#"
    type Tick { n: Int; }
    locus Sink {
        params { count: Int = 0; n: Int = 0; acc: Int = 0; }
        bus { subscribe "tick" as on_tick of type Tick; }
        fn on_tick(t: Tick) {
            self.acc = self.acc + t.n;
            self.count = self.count + 1;
            if self.count == self.n {
                print("final_rss_mb=");
                println(std::process::rss_bytes() / 1048576);
            }
        }
    }
    locus Source {
        params { n: Int = 0; }
        bus { publish "tick" of type Tick; }
        birth() { let mut i = 0; while i < self.n { "tick" <- Tick { n: i }; i = i + 1; } }
    }
    fn main() { Sink { n: 2000000 }; Source { n: 2000000 }; }
"#;

#[test]
fn flood_producer_does_not_grow_rss_unbounded() {
    let rss = build_and_rss("flood", FLOOD);
    // Unbounded (pre-#125) this floods to ~1 GB (2M cells × ~552 B). Bounded
    // at the default 8192-cell cap it stays well under 200 MB — the gap is
    // ~20x, so an absolute threshold is robust to build-config RSS variance.
    // Reaching the print at all also proves all 2M messages were delivered.
    assert!(
        rss < 200,
        "flood RSS {} MB — the bus queue did not stay bounded (expected < 200; \
         unbounded would be ~1 GB)",
        rss
    );
}
