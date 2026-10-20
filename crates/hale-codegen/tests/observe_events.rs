//! A `std::observe::Otlp` param of the main locus (GH #1592 S4).
//!
//! - The backend sends the runtime's lifecycle events (birth, dissolve,
//!   restart, failure) as OTLP/HTTP JSON log records, opened by a start
//!   record carrying the build's identity; an empty `endpoint` takes the
//!   OTLP environment's. Its own sends are not events.
//! - It sends from its own pinned locus, each send bounded by
//!   `timeout_ms` in all: a collector that never answers costs the program
//!   nothing but the batches, which are dropped and counted
//!   (`hale_otlp_dropped_total` when the program holds a scrape too).
//! - The event ring is filled only in a program that holds an `Otlp`:
//!   any other emits no event call and does not ask for the ring.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

/// Read one HTTP request off `s`: its path and body.
fn read_request(s: &mut TcpStream) -> Option<(String, String)> {
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut r = BufReader::new(s.try_clone().ok()?);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let path = line.split(' ').nth(1)?.to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).ok()?;
        if h == "\r\n" || h.is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().ok()?;
        }
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    Some((path, String::from_utf8_lossy(&body).into_owned()))
}

/// A collector on a port of its own, answering 200 to every export and
/// handing each body over.
fn collector() -> (String, mpsc::Receiver<(String, String)>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = l.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            if let Some(req) = read_request(&mut s) {
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                if tx.send(req).is_err() {
                    break;
                }
            }
        }
    });
    (format!("http://{addr}/v1/logs"), rx)
}

const LIFECYCLE: &str = r#"
locus Pump {
    params { ticks: Int = 0; failures: Int = 0; }
    closure delivery { captures: ticks; epoch inline; }
    @unbounded
    run() {
        while !self.draining {
            self.ticks = self.ticks + 1;
            if self.ticks == 2 && self.failures == 0 {
                self.failures = self.failures + 1;
                violate delivery;
            }
            std::time::sleep(5ms);
        }
    }
}

main locus App {
    params {
        pump: Pump = Pump { };
        events: std::observe::Otlp = std::observe::Otlp { service: "lifecycle", interval: 100ms };
    }
    placement { pump: pinned; events: pinned; }
    on_failure(c: Pump, err: ClosureViolation) {
        restart(c) for 5;
    }
    run() {
        std::time::sleep(400ms);
    }
}
fn main() { App { }; }
"#;

#[test]
fn an_events_backend_sends_lifecycle_records() {
    let (url, rx) = collector();
    let bin = harness::unique_bin("observe_events_lifecycle");
    build_opts::build_source(LIFECYCLE, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", &url)
        .env("OTEL_RESOURCE_ATTRIBUTES", "deployment.environment=test, team = core")
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let mut records: Vec<serde_json::Value> = Vec::new();
    let mut resources = Vec::new();
    while let Ok((path, body)) = rx.recv_timeout(Duration::from_millis(500)) {
        assert_eq!(path, "/v1/logs");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"));
        let rl = &v["resourceLogs"][0];
        resources.push(rl["resource"].clone());
        records.extend(rl["scopeLogs"][0]["logRecords"].as_array().expect("records").iter().cloned());
    }
    assert!(!records.is_empty(), "the collector received the export");
    let attr = |r: &serde_json::Value, k: &str| -> String {
        r["attributes"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["key"] == k))
            .map(|x| {
                let v = &x["value"];
                v["stringValue"].as_str().or(v["intValue"].as_str()).unwrap_or("").to_string()
            })
            .unwrap_or_default()
    };
    // the resource: the service, the environment's, the build's identity
    let res = &resources[0];
    assert_eq!(attr(res, "service.name"), "lifecycle");
    assert_eq!(attr(res, "deployment.environment"), "test");
    assert_eq!(attr(res, "team"), "core");
    assert_eq!(attr(res, "hale.model_hash").len(), 16, "{res}");
    // the start record first, then the lifecycle
    assert_eq!(records[0]["body"]["stringValue"], "hale.start");
    let said = |body: &str, locus: &str| records.iter().any(|r| r["body"]["stringValue"] == body && attr(r, "hale.locus") == locus);
    assert!(said("locus.birth", "App") && said("locus.birth", "Pump"), "{records:#?}");
    assert!(said("locus.failure", "Pump") && said("locus.restart", "Pump"), "{records:#?}");
    let failure = records.iter().find(|r| r["body"]["stringValue"] == "locus.failure").unwrap();
    assert_eq!(failure["severityText"], "WARN");
    assert_eq!(attr(failure, "hale.closure"), "delivery");
    assert!(failure["timeUnixNano"].as_str().unwrap().parse::<u64>().unwrap() > 1_600_000_000_000_000_000);
    // the backend's own sends are not events
    assert!(!said("locus.birth", "__StdHttpConn"), "the backend's sends are not the program's");
}

const SLOW: &str = r#"
locus Churn {
    params { n: Int = 0; }
}

main locus App {
    params {
        n: Int = 0;
        metrics: std::observe::Scrape = std::observe::Scrape { port: 0 };
        events: std::observe::Otlp = std::observe::Otlp { endpoint: "ENDPOINT", interval: 50ms, timeout_ms: 200 };
    }
    placement { metrics: pinned; events: pinned; }
    run() {
        let mut i = 0;
        while !self.draining {
            Churn { n: i };
            i = i + 1;
            std::time::sleep(10ms);
        }
    }
}
fn main() { App { }; }
"#;

/// A collector that takes the connection and never answers: every
/// send ends at its deadline, its batch dropped and counted, and the
/// program runs and ends as it would without it.
#[test]
fn a_collector_that_never_answers_costs_the_batches_not_the_program() {
    let silent = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = silent.local_addr().expect("addr");
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in silent.incoming().flatten() {
            held.push(s); // accepted, never read, never answered
        }
    });
    let src = SLOW.replace("ENDPOINT", &format!("http://{addr}/v1/logs"));
    let bin = harness::unique_bin("observe_events_silent");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let mut c = Command::new(&bin).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("spawn");
    let port = child::tcp_port(&mut c);
    let scrape = |port: u16| -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(s, "GET /metrics HTTP/1.1\r\nHost: t\r\n\r\n").unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).expect("read");
        raw
    };
    let dropped = |page: &str| -> f64 {
        page.lines()
            .find_map(|l| l.strip_prefix("hale_otlp_dropped_total "))
            .unwrap_or_else(|| panic!("no hale_otlp_dropped_total:\n{page}"))
            .parse()
            .unwrap()
    };
    // sends time out one after another; the program keeps going
    let start = Instant::now();
    let mut last = 0.0;
    while start.elapsed() < Duration::from_secs(10) && last < 2.0 {
        std::thread::sleep(Duration::from_millis(200));
        last = dropped(&scrape(port));
    }
    assert!(last >= 2.0, "batches the collector never took are counted: {last}");
    let sent = Instant::now();
    Command::new("kill").args(["-TERM", &c.id().to_string()]).status().expect("kill");
    let status = c.wait().expect("wait");
    let mut err = String::new();
    c.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(status.success(), "{status:?}\n{err}");
    assert!(sent.elapsed() < Duration::from_secs(3), "the drain does not wait on the collector: {:?}", sent.elapsed());
}

const PLAIN: &str = r#"
main locus App {
    params {
        reg: std::metrics::Registry = std::metrics::Registry { namespace: "app" };
        metrics: std::observe::Scrape = std::observe::Scrape { port: 0 };
    }
    placement { metrics: pinned; }
    run() {
        let hits = std::metrics::counter(self.reg, "hits", std::metrics::labels_empty());
        hits.inc();
    }
}
fn main() { App { }; }
"#;

/// The event ring is filled only in a program that holds an `Otlp`: one
/// that holds a scrape alone emits no event call, does not ask for the
/// ring, and its page has no OTLP series.
#[test]
fn a_program_without_events_fills_no_ring() {
    let calls = |ir: &str| -> Vec<String> {
        ir.lines()
            .filter(|l| l.contains("call") && (l.contains("@lotus_observe_event") || l.contains("@lotus_observe_events")))
            .map(|l| l.trim().to_string())
            .collect()
    };
    let bin = harness::unique_bin("observe_events_none_ir");
    let ir = harness::build_source_ir_text(PLAIN, &bin).expect("builds");
    assert!(calls(&ir).is_empty(), "no event calls without an `Otlp`:\n{:#?}", calls(&ir));
    assert!(ir.contains("@lotus_observe_events = i32 0") || ir.contains("@lotus_observe_events = global i32 0"), "the ring is off without an `Otlp`");

    let declared = PLAIN.replace(
        "        metrics: std::observe::Scrape = std::observe::Scrape { port: 0 };\n",
        "        metrics: std::observe::Scrape = std::observe::Scrape { port: 0 };\n        events: std::observe::Otlp = std::observe::Otlp { };\n",
    );
    let bin = harness::unique_bin("observe_events_some_ir");
    let ir = harness::build_source_ir_text(&declared, &bin).expect("builds");
    assert!(ir.contains("@lotus_observe_events = "), "the ring is asked for");
    assert!(calls(&ir).iter().any(|c| c.contains("@lotus_observe_events_take(")), "the backend drains it");

    // and at run: no OTLP series on a metrics-only page
    let bin = harness::unique_bin("observe_events_none_run");
    build_opts::build_source(
        &PLAIN.replace("        hits.inc();\n", "        hits.inc();\n        println(\"READY\");\n        while !self.draining { std::time::sleep(20ms); }\n"),
        &bin,
        &build_opts::options(),
    )
    .expect("build");
    let mut c = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn");
    let port = child::tcp_port(&mut c);
    let mut line = String::new();
    BufReader::new(c.stdout.take().unwrap()).read_line(&mut line).expect("read");
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(s, "GET /metrics HTTP/1.1\r\nHost: t\r\n\r\n").unwrap();
    let mut page = String::new();
    s.read_to_string(&mut page).expect("read");
    assert!(page.contains("app_hits 1"), "{page}");
    assert!(!page.contains("hale_otlp_dropped_total"), "{page}");
    Command::new("kill").args(["-TERM", &c.id().to_string()]).status().expect("kill");
    let _ = c.wait();
}

/// The exporter's own work is not recorded, but only the exporter's: an
/// ordinary locus on the same cooperative pool worker still has its
/// births and failures sent.
const SHARED_POOL: &str = r#"
locus Pump {
    params { ticks: Int = 0; failures: Int = 0; }
    closure delivery { captures: ticks; epoch inline; }
    @unbounded
    run() {
        while !self.draining {
            self.ticks = self.ticks + 1;
            if self.ticks == 2 && self.failures == 0 {
                self.failures = self.failures + 1;
                violate delivery;
            }
            std::time::sleep(5ms);
        }
    }
}

main locus App {
    params {
        pump: Pump = Pump { };
        events: std::observe::Otlp = std::observe::Otlp { service: "shared", interval: 100ms };
    }
    placement {
        pump: cooperative(pool = io) where async_io;
        events: cooperative(pool = io) where async_io;
    }
    on_failure(c: Pump, err: ClosureViolation) {
        restart(c) for 5;
    }
    run() {
        std::time::sleep(600ms);
    }
}
fn main() { App { }; }
"#;

#[test]
fn an_ordinary_locus_on_the_exporters_pool_still_emits_events() {
    let (url, rx) = collector();
    let bin = harness::unique_bin("observe_events_shared_pool");
    build_opts::build_source(SHARED_POOL, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).env("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", &url).output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let mut seen = String::new();
    while let Ok((_, body)) = rx.recv_timeout(Duration::from_millis(500)) {
        seen.push_str(&body);
    }
    for want in ["locus.failure", "locus.restart"] {
        assert!(seen.contains(want), "{want} from a locus on the exporter's pool: {seen}");
    }
    assert!(!seen.contains("__StdHttpConn"), "the backend's sends are not events: {seen}");
}

/// `max_batch: 0` is one event a batch, not a spin that never drains.
#[test]
fn a_zero_max_batch_still_drains() {
    let (url, rx) = collector();
    let src = LIFECYCLE.replace("interval: 100ms", "interval: 100ms, max_batch: 0");
    let bin = harness::unique_bin("observe_events_zero_batch");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).env("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", &url).output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let mut seen = String::new();
    while let Ok((_, body)) = rx.recv_timeout(Duration::from_millis(500)) {
        seen.push_str(&body);
    }
    assert!(seen.contains("locus.birth") && seen.contains("locus.failure"), "{seen}");
}

/// Every combination defines both flags the runtime references, or
/// neither (a program observing nothing links the runtime it did).
#[test]
fn each_observe_combination_defines_what_the_runtime_references() {
    let otlp = "        events: std::observe::Otlp = std::observe::Otlp { };\n";
    let scrape = "        metrics: std::observe::Scrape = std::observe::Scrape { port: 0 };\n";
    let base = PLAIN.replace(scrape, "").replace("    placement { metrics: pinned; }\n", "");
    let with = |extra: &str| base.replace("        reg:", &format!("{extra}        reg:"));
    let flags = |src: &str, name: &str| -> (bool, bool) {
        let bin = harness::unique_bin(name);
        let ir = harness::build_source_ir_text(src, &bin).expect("builds");
        (ir.contains("@lotus_observe_metrics = "), ir.contains("@lotus_observe_events = "))
    };
    assert_eq!(flags(&base, "observe_flags_none"), (false, false));
    assert_eq!(flags(&with(scrape), "observe_flags_scrape"), (true, true));
    assert_eq!(flags(&with(otlp), "observe_flags_otlp"), (true, true));
    assert_eq!(flags(&with(&format!("{scrape}{otlp}")), "observe_flags_both"), (true, true));
}
