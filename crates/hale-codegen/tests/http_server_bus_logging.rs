//! 2026-05-27 — bus-routed observability on
//! `std::http::Server` (closes #10). Mirrors the
//! `std::io::tcp::Stream` shape that landed in #16: a
//! `log_subject: String = ""` param on the Server, plus a
//! `bus { publish "io.http.**" of type LogEvent; }`
//! declaration. Server emits listen-start / accept /
//! listen-close events when log_subject is set; empty
//! subject (default) costs one len-check branch per event.
//!
//! The reused `std::io::tcp::LogEvent` type lets a single
//! subscriber locus see both TCP-layer and HTTP-layer
//! events on the same `io.**` wildcard.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_http_server_bus_logging_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn server_with_log_subject_publishes_lifecycle_events() {
    // Server with log_subject set + a sibling Sink locus
    // subscribed to `io.http.**`. The Server's max_accepts=0
    // (cap is satisfied immediately with no connections), so
    // run() returns without touching the accept path; we get
    // the listen_start + listen_close events.
    let src = r#"
        locus Handler {
            fn handle(req: std::http::Request) -> std::http::Response {
                return std::http::Response { status: 200, body: "ok" };
            }
        }
        locus Sink {
            bus { subscribe "io.http.**" as on_evt of type std::io::tcp::LogEvent; }
            fn on_evt(e: std::io::tcp::LogEvent) {
                println("EVT phase=", e.phase, " fd=", e.fd);
            }
        }
        fn main() {
            let _sink = Sink { };
            std::http::Server {
                host:        "127.0.0.1",
                port:        47861,
                handler:     Handler { },
                max_accepts: 0,
                log_subject: "io.http.test",
            };
            // Server.run() returns immediately because
            // max_accepts=0 satisfies the loop cap before
            // accepting; dissolve() fires at scope exit,
            // emitting the listen_close event.
        }
    "#;
    let (out, status) = build_and_run("lifecycle", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    assert!(
        out.contains("EVT phase=listen_start"),
        "expected listen_start event; stdout:\n{}", out
    );
    assert!(
        out.contains("EVT phase=listen_close"),
        "expected listen_close event; stdout:\n{}", out
    );
}

#[test]
fn server_with_empty_log_subject_emits_nothing() {
    // Same shape, but Server is constructed without a
    // log_subject (the default empty string). A Sink
    // subscribed to "io.http.**" still exists but its
    // handler must never fire — that's the gated-by-default
    // contract.
    let src = r#"
        locus Handler {
            fn handle(req: std::http::Request) -> std::http::Response {
                return std::http::Response { status: 200, body: "ok" };
            }
        }
        locus Sink {
            bus { subscribe "io.http.**" as on_evt of type std::io::tcp::LogEvent; }
            fn on_evt(e: std::io::tcp::LogEvent) {
                println("LEAK phase=", e.phase, " fd=", e.fd);
            }
        }
        fn main() {
            let _sink = Sink { };
            std::http::Server {
                host:        "127.0.0.1",
                port:        47863,
                handler:     Handler { },
                max_accepts: 0,
                // no log_subject — defaulted to ""
            };
            println("done");
        }
    "#;
    let (out, status) = build_and_run("disabled", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    assert!(out.contains("done"), "stdout: {}", out);
    assert!(
        !out.contains("LEAK"),
        "no events should fire when log_subject is empty; got:\n{}",
        out
    );
}
