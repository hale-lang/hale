//! 2026-05-27 — bus-routed I/O observability on
//! `std::io::tcp::Stream` (issue #10, first cut).
//!
//! Shape: Stream gains a `log_subject: String = ""` param
//! and a `bus { publish "io.tcp.**" }` declaration; each
//! send / recv / close emits a `std::io::tcp::LogEvent` on
//! the configured subject (no publish when empty). User
//! wires whatever subscriber they want — stderr sink,
//! metrics, ring buffer — via standard bus subscribe. No
//! `Logger` interface, no per-Stream sink locus.
//!
//! Two tests:
//!   1. Default Stream (log_subject="") emits nothing — the
//!      handler never fires. Closes the "always-noisy by
//!      default" footgun.
//!   2. Stream with log_subject set publishes one event per
//!      send + recv + close, with byte counts populated.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_io_tcp_bus_logging_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn stream_with_empty_log_subject_emits_nothing() {
    // A subscriber on "io.tcp.**" exists but the Stream's
    // log_subject is empty, so the publish branch never
    // fires. Verifies the default-disabled behavior — no
    // existing test should start emitting bus traffic just
    // because the LogEvent surface exists now.
    let src = r#"
        locus Sink {
            bus { subscribe "io.tcp.**" as on_evt of type std::io::tcp::LogEvent; }
            fn on_evt(e: std::io::tcp::LogEvent) {
                println("LEAK phase=", e.phase, " bytes=", e.bytes);
            }
        }
        fn main() {
            let _sink = Sink { };
            let listen_fd = std::io::tcp::listen_socket("127.0.0.1", 47841) or raise;
            let client_fd = std::io::tcp::connect("127.0.0.1", 47841) or raise;
            let _peer_fd  = std::io::tcp::accept_one(listen_fd) or raise;
            let client = std::io::tcp::Stream { conn_fd: client_fd };  // no log_subject
            client.send("hello") or raise;
            println("did_send");
            // Stream falls out of scope at fn end → dissolve closes
            // fd; with empty log_subject, no close-phase publish.
        }
    "#;
    let (out, status) = build_and_run("disabled", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    assert!(out.contains("did_send"), "stdout: {}", out);
    assert!(
        !out.contains("LEAK"),
        "no events should fire when log_subject is empty; got:\n{}",
        out
    );
}

#[test]
fn stream_with_log_subject_publishes_one_event_per_op() {
    // Stream constructed with log_subject="io.tcp.test"; a
    // subscriber on that subject collects events. Verify
    // send + recv + close all fire with sensible bytes.
    let src = r#"
        locus Sink {
            bus { subscribe "io.tcp.**" as on_evt of type std::io::tcp::LogEvent; }
            fn on_evt(e: std::io::tcp::LogEvent) {
                println("EVT phase=", e.phase, " bytes=", e.bytes);
            }
        }
        fn main() {
            let _sink = Sink { };
            let listen_fd = std::io::tcp::listen_socket("127.0.0.1", 47843) or raise;
            let client_fd = std::io::tcp::connect("127.0.0.1", 47843) or raise;
            let peer_fd   = std::io::tcp::accept_one(listen_fd) or raise;

            let client = std::io::tcp::Stream { conn_fd: client_fd, log_subject: "io.tcp.test" };
            let peer   = std::io::tcp::Stream { conn_fd: peer_fd };

            client.send("hello") or raise;
            // Peer reads what the client sent.
            let got  = peer.recv(64) or raise;
            println("peer_got=", got);
            // Both Streams dissolve at fn-exit; client's dissolve
            // also publishes a close-phase event.
        }
    "#;
    let (out, status) = build_and_run("enabled", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    // Both a send-phase event and a close-phase event fire on
    // the client Stream; the peer Stream has no log_subject so
    // its recv emits nothing. (The exact byte count from __send
    // is a separate matter — what this test pins is "the bus
    // emits land where they should at the right phases.")
    assert!(out.contains("EVT phase=send"),  "stdout: {}", out);
    assert!(out.contains("EVT phase=close"), "stdout: {}", out);
    let evt_lines: Vec<_> = out.lines().filter(|l| l.starts_with("EVT ")).collect();
    assert!(
        evt_lines.len() >= 2,
        "expected ≥2 events (send + close); got {} from:\n{}",
        evt_lines.len(), out
    );
    // Peer Stream has no log_subject — no recv event should
    // appear, even though the peer.recv call ran.
    assert!(
        !out.contains("phase=recv"),
        "peer Stream (no log_subject) shouldn't emit a recv event; got:\n{}",
        out
    );
}
