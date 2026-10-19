//! std::metrics (promoted from pond/metrics, 2026-07-18).
//!
//! Two layers: the direct test drives Registry/Counter/Gauge/
//! Histogram and `render()` with no sockets (idempotent
//! re-registration, cumulative buckets, +Inf, namespace prefixes);
//! the wire test mounts `std::metrics::Endpoint` as a Server
//! handler — with the Registry built by a returning free fn, the
//! shape that requires the Registry to own its storage as
//! param-default children — and scrapes it over real TCP.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;
#[path = "support/prometheus.rs"]
mod prometheus;

#[test]
fn registry_renders_counter_gauge_histogram() {
    let src = r#"
        fn main() {
            let reg = std::metrics::Registry { namespace: "app" };

            let hits = std::metrics::counter(reg, "hits",
                std::metrics::labels_one("route", "/api"));
            hits.inc();
            hits.add(2.0);
            // Re-registration returns a handle to the SAME series.
            let again = std::metrics::counter(reg, "hits",
                std::metrics::labels_one("route", "/api"));
            again.add(3.0);

            let temp = std::metrics::gauge(reg, "temp",
                std::metrics::labels_empty());
            temp.set(25.0);
            temp.sub(4.5);

            let lat = std::metrics::histogram(reg, "latency",
                "0.01 0.1 1.0", std::metrics::labels_empty());
            lat.observe(0.005);
            lat.observe(0.05);
            lat.observe(0.5);
            lat.observe(5.0);

            print(reg.render());
        }
    "#;
    let bin = harness::unique_bin(&format!("hale_metrics_direct_{}", std::process::id()));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exit: {:?}", out.status);
    let text = String::from_utf8_lossy(&out.stdout);

    // Counter: idempotent re-registration accumulated 1+2+3 on ONE
    // series, namespaced + labeled.
    assert!(text.contains("# TYPE app_hits counter"), "got:\n{}", text);
    assert!(text.contains("app_hits{route=\"/api\"} 6"), "got:\n{}", text);
    // Gauge: set then sub.
    assert!(text.contains("app_temp 20.5"), "got:\n{}", text);
    // Histogram: cumulative buckets 1/2/3 then +Inf catches the
    // out-of-range observe; sum and count follow.
    assert!(text.contains("# TYPE app_latency histogram"), "got:\n{}", text);
    assert!(text.contains("app_latency_bucket{le=\"0.01\"} 1"), "got:\n{}", text);
    assert!(text.contains("app_latency_bucket{le=\"0.1\"} 2"), "got:\n{}", text);
    assert!(text.contains("app_latency_bucket{le=\"1\"} 3"), "got:\n{}", text);
    assert!(text.contains("app_latency_bucket{le=\"+Inf\"} 4"), "got:\n{}", text);
    assert!(text.contains("app_latency_sum 5.555"), "got:\n{}", text);
    assert!(text.contains("app_latency_count 4"), "got:\n{}", text);
}

#[test]
fn render_keeps_every_digit() {
    // GH #988: a sample rendered through `to_string(Float)` kept six
    // significant digits, so a gauge of epoch seconds read 1.79056e+09 and
    // a counter past 999999 was rounded. Every value is exact now.
    let src = r#"
        fn main() {
            let reg = std::metrics::Registry { namespace: "app" };
            std::metrics::gauge(reg, "since", std::metrics::labels_empty()).set(std::math::int_to_float(1790558458));
            std::metrics::counter(reg, "big", std::metrics::labels_empty()).add(1234567.0);
            std::metrics::gauge(reg, "frac", std::metrics::labels_empty()).set(std::math::int_to_float(1790558458) + 0.25);
            std::metrics::gauge(reg, "small", std::metrics::labels_empty()).set(0.005);
            std::metrics::gauge(reg, "neg", std::metrics::labels_empty()).set(-3.75);
            std::metrics::gauge(reg, "missing", std::metrics::labels_empty()).set(std::math::nan());
            std::metrics::gauge(reg, "tiny", std::metrics::labels_empty()).set(0.0000000001);
            let lat = std::metrics::histogram(reg, "wait", "0.25 5000000.0", std::metrics::labels_empty());
            lat.observe(2500000.0);
            print(reg.render());
        }
    "#;
    let bin = harness::unique_bin(&format!("hale_metrics_digits_{}", std::process::id()));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exit: {:?}", out.status);
    let text = String::from_utf8_lossy(&out.stdout);
    for want in [
        "app_since 1790558458\n",
        "app_big 1234567\n",
        "app_frac 1790558458.25\n",
        "app_small 0.005\n",
        "app_neg -3.75\n",
        "app_missing NaN\n",
        "app_tiny 1e-10\n",
        "app_wait_bucket{le=\"5000000\"} 1\n",
        "app_wait_sum 2500000\n",
    ] {
        assert!(text.contains(want), "want {want:?} in:\n{text}");
    }
    assert!(!text.contains("e+"), "no sample in exponent form:\n{text}");
}

#[test]
fn endpoint_scrapes_through_server_over_tcp() {
    // port 0, read back from the listening socket: a port drawn here and
    // handed over is free, not reserved, and another test of a parallel
    // run can bind it first. Read from `/proc`, not a probe connect, so
    // the one accept is the scrape's.
    let src = format!(
        r#"
        fn build_reg() -> std::metrics::Registry {{
            let reg = std::metrics::Registry {{ namespace: "app" }};
            let hits = std::metrics::counter(reg, "hits",
                std::metrics::labels_empty());
            hits.inc();
            return reg;
        }}
        fn main() {{
            std::http::Server {{
                port: 0, max_accepts: 1, ready_signal: "READY",
                handler: std::metrics::Endpoint {{ registry: build_reg() }}
            }};
        }}
    "#
    );
    let bin = harness::unique_bin(&format!("hale_metrics_wire_{}", std::process::id()));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn server");

    let port = child::tcp_port(&mut child);

    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(s, "GET /metrics HTTP/1.1\r\nHost: t\r\n\r\n").expect("send");
    let mut buf = String::new();
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let _ = s.read_to_string(&mut buf);

    assert!(buf.starts_with("HTTP/1.1 200"), "got:\n{}", buf);
    assert!(
        buf.contains("Content-Type: text/plain; version=0.0.4"),
        "got:\n{}",
        buf
    );
    // The scrape must see the series registered inside build_reg():
    // the Registry's param-default storage survives the builder's
    // scope because it is owned by the returned Registry.
    assert!(buf.contains("app_hits 1"), "got:\n{}", buf);

    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
}

/// A family with several labelled series is written whole under one
/// `# TYPE` line, so the page parses as Prometheus text: the renderer
/// wrote a TYPE line per series, and the map's order scattered a
/// family's series between others', both of which the text format
/// refuses. `Registry.render()` and `Endpoint` share the renderer
/// `std::observe::Scrape` uses (`__metrics_render_series`).
#[test]
fn a_family_of_labelled_series_renders_under_one_type_line() {
    let src = r#"
        fn main() {
            let reg = std::metrics::Registry { namespace: "app" };
            let a = std::metrics::counter(reg, "hits", std::metrics::labels_one("route", "/a"));
            let g = std::metrics::gauge(reg, "depth", std::metrics::labels_empty());
            let b = std::metrics::counter(reg, "hits", std::metrics::labels_one("route", "/b"));
            let r = std::metrics::histogram(reg, "lat", "0.5 1 2", std::metrics::labels_one("op", "read"));
            let g2 = std::metrics::gauge(reg, "temp", std::metrics::labels_two("room", "a \"b\"", "floor", "1"));
            let w = std::metrics::histogram(reg, "lat", "0.5 1 2", std::metrics::labels_one("op", "write"));
            let c = std::metrics::counter(reg, "hits", std::metrics::labels_one("route", "/c\\d"));
            a.inc();
            b.add(2.0);
            c.add(3.0);
            g.set(4.0);
            g2.set(21.5);
            r.observe(1.5);
            w.observe(0.25);
            w.observe(9.0);
            print(reg.render());
        }
    "#;
    let bin = harness::unique_bin("hale_metrics_families");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exit: {:?}", out.status);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let families = prometheus::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let names: Vec<&str> = families.iter().map(|(n, _)| n.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, ["app_depth", "app_hits", "app_lat", "app_temp"], "{text}");
    for (fam, kind) in [("app_hits", "counter"), ("app_lat", "histogram")] {
        assert_eq!(text.matches(&format!("# TYPE {fam} {kind}\n")).count(), 1, "{text}");
    }
    assert_eq!(text.lines().filter(|l| l.starts_with("app_hits{")).count(), 3, "{text}");
    assert!(text.contains("app_temp{room=\"a \\\"b\\\"\",floor=\"1\"} 21.5\n"), "{text}");
    assert!(text.contains("app_lat_bucket{op=\"write\",le=\"+Inf\"} 2\n"), "{text}");

    // the checker refuses what the renderer used to write
    let old = "# TYPE app_hits counter\napp_hits{route=\"/a\"} 1\n# TYPE app_depth gauge\napp_depth 4\n\
               # TYPE app_hits counter\napp_hits{route=\"/b\"} 2\n";
    assert!(prometheus::parse(old).is_err(), "a second TYPE line is refused");
    let scattered = "# TYPE app_hits counter\napp_hits{route=\"/a\"} 1\n# TYPE app_depth gauge\napp_depth 4\n\
                     app_hits{route=\"/b\"} 2\n";
    assert!(prometheus::parse(scattered).is_err(), "a family's series apart is refused");
}
