//! A `std::observe::Scrape` param of the main locus (GH #1592).
//!
//! - A scrape serves every live `std::metrics::Registry`'s
//!   series, as `Registry.render()` writes them, and the runtime's own
//!   (`hale_*`): resident memory, CPU time, start time, the drain, live loci, restarts and
//!   failures, each pool's queue and parked coroutines, each program
//!   topic's traffic. Its path is the one served; a counter moves
//!   between two scrapes.
//! - The runtime counts in counters-only mode: no shared-memory segment.
//! - A program with a Registry and no `observe` emits no registration
//!   call (and no update call) and does not ask for counters-only mode.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;
#[path = "support/prometheus.rs"]
mod prometheus;

/// GET `path` from 127.0.0.1:`port`: the status and the body.
fn get(port: u16, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: t\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

/// The value of the sample line `name` in an exposition.
fn sample(body: &str, name: &str) -> f64 {
    body.lines()
        .find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix(' ')))
        .unwrap_or_else(|| panic!("no `{name}` in:\n{body}"))
        .parse()
        .expect("a number")
}

const SCRAPED: &str = r#"
type J {
    n: Int = 0;
}

topic Job {
    payload: J;
    subject: "job";
}

locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Job as on_job; }
    fn on_job(j: J) { self.seen = self.seen + j.n; }
}

main locus App {
    params {
        reg: std::metrics::Registry = std::metrics::Registry { namespace: "app" };
        worker: Worker = Worker { };
        metrics: std::observe::Scrape = std::observe::Scrape { port: 0, path: "/m" };
    }
    bus { publish Job; }
    placement { worker: cooperative(pool = jobs); metrics: pinned; }
    run() {
        let hits = std::metrics::counter(self.reg, "hits", std::metrics::labels_one("route", "/a b\"c\\d"));
        hits.inc();
        hits.add(2.5);
        let other = std::metrics::counter(self.reg, "hits", std::metrics::labels_one("route", "/z"));
        other.inc();
        let depth = std::metrics::gauge(self.reg, "depth", std::metrics::labels_empty());
        depth.set(1790558458.0);
        depth.sub(0.25);
        let lat = std::metrics::histogram(self.reg, "lat", "0.1 0.5 1", std::metrics::labels_two("a", "1", "b", "2"));
        lat.observe(0.2);
        lat.observe(3.0);
        lat.observe(0.05);
        let mut i = 0;
        while i < 5 {
            Job <- J { n: i };
            i = i + 1;
        }
        // let the worker take them before the exposition is compared
        std::time::sleep(200ms);
        print(self.reg.render());
        println("READY");
        let ticks = std::metrics::counter(self.reg, "ticks", std::metrics::labels_empty());
        let mut t = 0;
        while t < 60 && !self.draining {
            ticks.inc();
            std::time::sleep(50ms);
            t = t + 1;
        }
    }
}
fn main() { App { }; }
"#;

#[test]
fn a_scrape_serves_the_registries_and_the_runtime() {
    let bin = harness::unique_bin("observe_scrape");
    build_opts::build_source(SCRAPED, &bin, &build_opts::options()).expect("build");
    let mut c = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    let port = child::tcp_port(&mut c);
    let mut out = BufReader::new(c.stdout.take().unwrap());
    let mut rendered = String::new();
    loop {
        let mut line = String::new();
        assert!(out.read_line(&mut line).expect("read") > 0, "the program ended before READY:\n{rendered}");
        if line.trim_end() == "READY" {
            break;
        }
        rendered.push_str(&line);
    }

    // no shared-memory segment: counters-only mode keeps them private
    assert!(
        !std::path::Path::new(&format!("/dev/shm/hale-obs-{}", c.id())).exists(),
        "a scrape makes no observation segment"
    );

    let (status, first) = get(port, "/m");
    assert_eq!(status, 200, "{first}");
    assert_eq!(get(port, "/metrics").0, 404, "only the declared path is served");
    assert_eq!(get(port, "/m?x=1").0, 200, "a query is not part of the path");

    // every line Registry.render() wrote is served, the `# TYPE` line of a
    // family with several series once
    let served: BTreeSet<&str> = first.lines().collect();
    let app: BTreeSet<&str> = rendered.lines().collect();
    for l in &app {
        assert!(served.contains(l), "`{l}` is rendered and not served:\n{first}");
    }
    prometheus::parse(&first).unwrap_or_else(|e| panic!("the page parses as Prometheus text: {e}\n{first}"));
    let types = first.lines().filter(|l| *l == "# TYPE app_hits counter").count();
    assert_eq!(types, 1, "one TYPE line for a family:\n{first}");
    assert!(first.contains("app_hits{route=\"/a b\\\"c\\\\d\"} 3.5\n"), "{first}");
    assert!(first.contains("app_lat_bucket{a=\"1\",b=\"2\",le=\"0.5\"} 2\n"), "{first}");

    // the runtime's own
    assert!(sample(&first, "hale_process_resident_memory_bytes") > 0.0, "{first}");
    // its CPU time, user and system (a counter), and when it started, in
    // unix seconds (a gauge): within the last hour, and not after now
    assert!(first.contains("# TYPE hale_process_cpu_seconds_total counter\n"), "{first}");
    assert!(sample(&first, "hale_process_cpu_seconds_total") >= 0.0, "{first}");
    assert!(first.contains("# TYPE hale_process_start_time_seconds gauge\n"), "{first}");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let started = sample(&first, "hale_process_start_time_seconds");
    assert!(started <= now && started > now - 3600.0, "started {started}, now {now}:\n{first}");
    assert_eq!(sample(&first, "hale_draining"), 0.0);
    assert!(sample(&first, "hale_loci_live") >= 2.0, "{first}");
    assert_eq!(sample(&first, "hale_restarts_total"), 0.0);
    assert_eq!(sample(&first, "hale_failures_total"), 0.0);
    assert_eq!(sample(&first, "hale_pool_queue_depth{pool=\"jobs\"}"), 0.0, "{first}");
    assert_eq!(sample(&first, "hale_pool_parked_coroutines{pool=\"jobs\"}"), 0.0, "{first}");
    assert_eq!(sample(&first, "hale_topic_published_total{topic=\"job\"}"), 5.0, "{first}");
    assert_eq!(sample(&first, "hale_topic_delivered_total{topic=\"job\"}"), 5.0, "{first}");
    assert!(!first.contains("topic=\"__"), "the runtime's own plumbing is not a program topic:\n{first}");

    // a live counter moves
    let t0 = sample(&first, "app_ticks");
    std::thread::sleep(Duration::from_millis(400));
    let (_, second) = get(port, "/m");
    assert!(sample(&second, "app_ticks") > t0, "{second}");

    Command::new("kill").args(["-TERM", &c.id().to_string()]).status().expect("kill");
    let status = c.wait().expect("wait");
    let mut err = String::new();
    c.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(status.success(), "{status:?}\n{err}");
}

const REGISTRY: &str = r#"
main locus App {
    params {
        reg: std::metrics::Registry = std::metrics::Registry { namespace: "app" };
    }
    run() {
        let hits = std::metrics::counter(self.reg, "hits", std::metrics::labels_empty());
        hits.inc();
        let g = std::metrics::gauge(self.reg, "g", std::metrics::labels_empty());
        g.set(2.0);
        let h = std::metrics::histogram(self.reg, "h", "1 2", std::metrics::labels_empty());
        h.observe(1.5);
        println(self.reg.render());
    }
}
fn main() { App { }; }
"#;

/// Where [`REGISTRY`] takes a scrape param, and the param.
const SCRAPE_AT: &str = "        reg: std::metrics::Registry = std::metrics::Registry { namespace: \"app\" };\n";
const SCRAPE: &str = "        reg: std::metrics::Registry = std::metrics::Registry { namespace: \"app\" };\n        metrics: std::observe::Scrape = std::observe::Scrape { port: 0 };\n";

#[test]
fn a_registry_without_observe_registers_nothing() {
    let calls = |ir: &str| -> Vec<String> {
        ir.lines()
            .filter(|l| l.contains("call") && l.contains("@lotus_observe_"))
            .map(|l| l.trim().to_string())
            .collect()
    };
    let bin = harness::unique_bin("observe_none_ir");
    let ir = harness::build_source_ir_text(REGISTRY, &bin).expect("builds");
    assert!(calls(&ir).is_empty(), "no registration or update calls without a scrape:\n{:#?}", calls(&ir));
    assert!(!ir.contains("@lotus_observe_metrics ="), "no counters-only mode without a scrape");

    // the same program holding a scrape makes them
    let declared = REGISTRY.replace(SCRAPE_AT, SCRAPE);
    let bin = harness::unique_bin("observe_some_ir");
    let ir = harness::build_source_ir_text(&declared, &bin).expect("builds");
    let made = calls(&ir).join("\n");
    for f in ["@lotus_observe_registry(", "@lotus_observe_series(", "@lotus_observe_add(", "@lotus_observe_set("] {
        assert!(made.contains(f), "`{f}` with a scrape:\n{made}");
    }
    assert!(ir.contains("@lotus_observe_metrics = "), "counters-only mode is asked for");
}

const RESTARTED: &str = r#"
locus Pump {
    params { ticks: Int = 0; failures: Int = 0; }
    closure delivery { captures: ticks; epoch inline; }
    @unbounded
    run() {
        while !self.draining {
            self.ticks = self.ticks + 1;
            if (self.ticks == 2 && self.failures == 0) || (self.ticks == 30 && self.failures == 1) {
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
        fired: Int = 0;
        metrics: std::observe::Scrape = std::observe::Scrape { port: 0 };
    }
    placement { pump: pinned; metrics: pinned; }
    on_failure(c: Pump, err: ClosureViolation) {
        self.fired = self.fired + 1;
        restart(c) for 5;
    }
    run() {
        let mut i = 0;
        while self.fired < 2 && i < 400 {
            std::time::sleep(10ms);
            i = i + 1;
        }
        println("READY");
        while !self.draining {
            std::time::sleep(20ms);
        }
    }
}
fn main() { App { }; }
"#;

#[test]
fn a_scrape_counts_restarts_and_failures() {
    let bin = harness::unique_bin("observe_restarts");
    build_opts::build_source(RESTARTED, &bin, &build_opts::options()).expect("build");
    let mut c = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    let port = child::tcp_port(&mut c);
    let mut out = BufReader::new(c.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).expect("read");
    assert_eq!(line.trim_end(), "READY");
    let (_, body) = get(port, "/metrics");
    assert_eq!(sample(&body, "hale_failures_total"), 2.0, "{body}");
    assert_eq!(sample(&body, "hale_restarts_total"), 2.0, "{body}");
    Command::new("kill").args(["-TERM", &c.id().to_string()]).status().expect("kill");
    let status = c.wait().expect("wait");
    assert!(status.success(), "{status:?}");
}

/// A program that uses neither `std::metrics` nor `observe` links none
/// of either: what of `metrics.hl` it does not call is dropped before
/// the link, as any unused stdlib declaration is, and the runtime's
/// observe support (the series table, the counters-only segment, the
/// pools' parked counts) is compiled out of the runtime it links
/// (`-DLOTUS_OBSERVE` is passed only for a program that observes), so
/// its binary is the one it was before #1592: on `fn main() { }` the
/// same `.text` size and the same instruction count (measured on the
/// PR). Pinned here by the symbols: what is not linked has none.
#[test]
fn a_program_that_observes_nothing_links_none_of_it() {
    let symbols = |src: &str, tag: &str| -> Vec<String> {
        let bin = harness::unique_bin(tag);
        build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
        let out = Command::new("nm").arg(&bin).output().expect("nm");
        let _ = std::fs::remove_file(&bin);
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_whitespace().last())
            .filter(|s| s.contains("lotus_observe") || s.contains("Metrics") || s.contains("__metrics_") || s.contains("lotus_obs_live_loci"))
            .map(String::from)
            .collect()
    };
    for (tag, src) in [
        ("observe_none_empty", "fn main() { }\n"),
        (
            "observe_none_logs",
            "main locus App {\n    run() {\n        std::log::StdoutSink { };\n        let log = std::log::Logger { name: \"app\" };\n        log.info(\"up\");\n    }\n}\nfn main() { App { }; }\n",
        ),
    ] {
        let found = symbols(src, tag);
        assert!(found.is_empty(), "{tag}: nothing of metrics or observe is linked: {found:?}");
    }
    // the controls: a program that uses std::metrics links it, and one that
    // observes links the runtime's support too
    let metrics = symbols(REGISTRY, "observe_none_registry");
    assert!(metrics.iter().any(|s| s.contains("Metrics")), "{metrics:?}");
    assert!(!metrics.iter().any(|s| s.contains("lotus_observe")), "{metrics:?}");
    let observed = REGISTRY.replace(SCRAPE_AT, SCRAPE);
    let both = symbols(&observed, "observe_some_registry");
    assert!(both.iter().any(|s| s.contains("lotus_observe_registry")), "{both:?}");
}
