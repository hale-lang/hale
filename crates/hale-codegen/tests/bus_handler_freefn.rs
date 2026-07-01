//! Regression: calling a free fn (or any synchronously-invoked body whose
//! tail flush drains the bus queue) from inside a bus handler must NOT
//! corrupt delivery order. The free-fn exit ran `flush_dissolve_frame`
//! (which drains), so a free-fn call mid-dispatch re-entered the drain loop
//! and popped the NEXT cell first — handlers nested, delivery came out
//! reversed (30/20/10) and `self` read stale (every line saw the final
//! count). A `self`-method that returns explicitly was fine (its closed
//! block skips the draining flush), which masked the bug. Fixed by a
//! re-entrancy guard in `lotus_bus_queue_drain` (a nested drain on the same
//! queue is a no-op; the outer for-loop pumps FIFO). This pins the
//! free-fn-in-handler shape to in-order delivery.

use std::process::Command;

use hale_codegen::build_executable;

fn run_lines(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    // pid alone is NOT unique here: both #[test] fns run inside one
    // test-binary process, so parallel execution raced on the same
    // artifact path (each test intermittently ran the OTHER test's
    // binary). A per-call counter disambiguates.
    static NEXT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    bin.push(format!("hale_bus_freefn_{}_{}", std::process::id(), n));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect()
}

const SHARED_LOCI: &str = r#"
    type Ping { n: Int; }
    locus Pinger {
        bus { publish "pings" of type Ping; }
        birth() {
            "pings" <- Ping { n: 10 };
            "pings" <- Ping { n: 20 };
            "pings" <- Ping { n: 30 };
        }
    }
"#;

#[test]
fn free_fn_call_in_handler_preserves_delivery_order() {
    // The handler factors its formatting into a top-level free fn — the
    // everyday "pull helper logic out of the handler" pattern.
    let src = format!(
        r#"
        {SHARED_LOCI}
        fn label(n: Int, c: Int) -> String {{
            return "ping " + to_string(n) + " count " + to_string(c);
        }}
        locus Echo {{
            params {{ count: Int = 0; }}
            bus {{ subscribe "pings" as on_ping of type Ping; }}
            fn on_ping(p: Ping) {{
                self.count = self.count + 1;
                println(label(p.n, self.count));
            }}
        }}
        fn main() {{ Echo {{ }}; Pinger {{ }}; }}
    "#
    );
    let lines = run_lines(&src);
    assert_eq!(
        lines,
        vec![
            "ping 10 count 1".to_string(),
            "ping 20 count 2".to_string(),
            "ping 30 count 3".to_string(),
        ],
        "free-fn call in a handler must keep FIFO delivery + fresh self"
    );
}

#[test]
fn handler_republish_still_delivers_in_order() {
    // The re-entrancy guard must NOT drop messages a handler publishes:
    // the outer drain loop picks up newly-enqueued cells.
    let src = r#"
        type Ev { n: Int; }
        fn tag(n: Int) -> Int { return n; }
        locus Worker {
            params { seen: Int = 0; }
            bus { subscribe "ev" as on_ev of type Ev; publish "ev" of type Ev; }
            fn on_ev(e: Ev) {
                self.seen = self.seen + 1;
                println("ev ", tag(e.n), " seen ", self.seen);
                if e.n == 1 { "ev" <- Ev { n: 99 }; }
            }
        }
        locus Src {
            bus { publish "ev" of type Ev; }
            birth() { "ev" <- Ev { n: 1 }; "ev" <- Ev { n: 2 }; }
        }
        fn main() { Worker { }; Src { }; }
    "#;
    let lines = run_lines(src);
    assert_eq!(
        lines,
        vec![
            "ev 1 seen 1".to_string(),
            "ev 2 seen 2".to_string(),
            "ev 99 seen 3".to_string(),
        ],
        "a handler-published event must still be delivered (outer loop)"
    );
}
