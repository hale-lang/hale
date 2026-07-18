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
use std::thread;
use std::time::Duration;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

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
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_metrics_direct_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
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
fn endpoint_scrapes_through_server_over_tcp() {
    let port = pick_free_port();
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
                port: {port}, max_accepts: 2, ready_signal: "READY",
                handler: std::metrics::Endpoint {{ registry: build_reg() }}
            }};
        }}
    "#
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_metrics_wire_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut ready = false;
    for _ in 0..50 {
        thread::sleep(Duration::from_millis(100));
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
    }
    assert!(ready, "server never started listening");

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
