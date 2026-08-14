//! GH #468 — live binding ingest must not lose messages at the
//! registry's edges. Both observed shapes were the same defect:
//! wire ingress dispatched against an INCOMPLETE deserializer
//! registry — not yet filled (the boot window between transport
//! realization and same-birth subscriber registrations) or already
//! torn down (a storm-descheduled reader draining its kernel queue
//! during teardown, after lotus_bus_router_destroy freed the
//! registry). The kernel itself never discards: AF_UNIX
//! SEQPACKET/STREAM return queued data before EOF even after
//! shutdown(SHUT_RDWR) — verified by probe while root-causing.
//!
//! The fix, held here:
//!   1. **Boot window buffers.** A reader that recv's with no
//!      matching deserializer buffers the wire bytes (bounded:
//!      64 msgs / 1 MiB per binding, oldest-first eviction,
//!      counted) and they are flushed FIFO the moment a matching
//!      registration lands — mid-run, no extra message needed.
//!   2. **Exit quiesces.** Codegen emits lotus_bus_ingress_quiesce
//!      at every main-exit point BEFORE pools join and loci
//!      dissolve: LISTEN fds half-close, readers drain their
//!      kernel queues to true EOF through the intact registry,
//!      and one final local drain delivers the tail to live
//!      handlers. Bounded (LOTUS_BUS_QUIESCE_MS, default 500).
//!   3. **Exit stays prompt.** A peer holding a connection open
//!      must not stall the quiesce — the half-close turns the
//!      reader's next recv into EOF once the queue is empty.
//!
//! The windows are made deterministic by two test-only env hooks
//! (LOTUS_BUS_TEST_BOOT_HOLD_MS stretches realize→registration;
//! LOTUS_BUS_TEST_READER_STALL_MS deschedules the reader), the
//! honest stand-ins for what a loaded CI runner does at random.

#![cfg(unix)]

use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hale_codegen::{build_executable_with_options, BuildOptions};

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(name.to_string(), &program);
    let bundle = hale_types::Bundle::new(programs);
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let bin = harness::unique_bin(&format!("hale_test_468_{}", name));
    let options = BuildOptions {
        model_hash: Some(model_hash),
        ..BuildOptions::default()
    };
    build_executable_with_options(&program, &bin, &[], &options)
        .expect("build");
    bin
}

fn unique_socket_path(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-468-{}-{}-{}.sock",
        std::env::temp_dir().display(),
        tag,
        std::process::id(),
        nanos
    )
}

/// Subscriber that WAITS mid-run until it has seen `want`
/// messages, then prints the sum — the boot-window canary: if the
/// boot-window publishes are dropped (old behavior) it times out
/// and exits 3; if they are buffered + flushed at registration it
/// completes.
fn waiting_sub_src(sock: &str, want: u32) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt468"; }}
        locus Sub {{
            params {{ seen: Int = 0; total: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                self.total = self.total + t.n;
                println("got=", t.n);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{sock}", role: listen); }}
            run() {{
                let mut waited = 0;
                while self.sub.seen < {want} {{
                    std::time::sleep(50ms);
                    waited = waited + 1;
                    if waited > 200 {{
                        std::process::exit(3);
                    }}
                }}
                println("total=", self.sub.total);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    )
}

/// Subscriber that does NOT wait: run() returns almost
/// immediately — the teardown canary. Its handler prints, so the
/// only way `got=` lines appear is the exit quiesce delivering
/// the kernel-queued tail through still-alive loci.
fn exiting_sub_src(sock: &str) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt468"; }}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Evt as on_evt; }}
            fn on_evt(t: T) {{
                self.seen = self.seen + 1;
                println("got=", t.n);
            }}
        }}
        main locus App {{
            params {{ sub: Sub = Sub {{ }}; }}
            bindings {{ Evt: unix("{sock}", role: listen); }}
            run() {{
                std::time::sleep(120ms);
                println("exiting seen=", self.sub.seen);
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    )
}

/// Publisher: connect (with the transport's built-in retry), send
/// the burst back-to-back — deliberately NO settle sleep before
/// the first send; racing the subscriber's boot is the point —
/// then exit, closing the socket with the burst possibly
/// undrained.
fn burst_pub_src(sock: &str) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt468"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{sock}", role: connect); }}
            run() {{
                Evt <- T {{ n: 7 }};
                Evt <- T {{ n: 11 }};
                Evt <- T {{ n: 23 }};
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    )
}

/// Publisher that connects and then just holds the connection
/// open, silent — the quiesce-must-not-hang canary.
fn silent_holder_src(sock: &str) -> String {
    format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt468"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{sock}", role: connect); }}
            run() {{ std::time::sleep(5s); }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    )
}

/// 1. The boot window: the listener is bound and its reader live,
///    but the boot thread is held for 300ms before the
///    subscriber's registration runs (the deterministic version of
///    "realization precedes same-birth registrations, stretched by
///    load"). The publisher's whole burst lands in that window.
///    Old behavior: all three silently dropped, subscriber times
///    out. Required: buffered, flushed at registration time, seen
///    MID-RUN (not at exit) — the waiter completes with the full
///    total.
#[test]
fn boot_window_publishes_are_buffered_and_delivered_mid_run() {
    let sock = unique_socket_path("bootwin");
    let sub_bin = build("bootwin_sub", &waiting_sub_src(&sock, 3));
    let pub_bin = build("bootwin_pub", &burst_pub_src(&sock));

    let sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_TEST_BOOT_HOLD_MS", "300")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    let p = Command::new(&pub_bin).output().expect("run publisher");
    assert!(
        p.status.success(),
        "publisher failed: {}",
        String::from_utf8_lossy(&p.stderr)
    );
    let out = sub.wait_with_output().expect("subscriber exit");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "subscriber failed (timed out waiting = the old drop \
         behavior): status {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("total=41"),
        "boot-window burst must arrive whole (7+11+23):\n{}",
        stdout
    );

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
}

/// 2. The teardown tail: the reader is descheduled for 250ms —
///    past the subscriber's entire (120ms) run — so the whole
///    burst is still in the kernel when main returns. The exit
///    quiesce must deliver it: listener half-closes, the stalled
///    reader wakes, accepts the queued backlog connection, drains
///    to EOF through the intact registry, and the final local
///    drain runs the handler on still-alive loci. Old behavior:
///    the registry was freed first and the tail vanished silently.
#[test]
fn exit_quiesce_delivers_the_kernel_queued_tail() {
    let sock = unique_socket_path("tail");
    let sub_bin = build("tail_sub", &exiting_sub_src(&sock));
    let pub_bin = build("tail_pub", &burst_pub_src(&sock));

    let sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_TEST_READER_STALL_MS", "250")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    let p = Command::new(&pub_bin).output().expect("run publisher");
    assert!(
        p.status.success(),
        "publisher failed: {}",
        String::from_utf8_lossy(&p.stderr)
    );
    let out = sub.wait_with_output().expect("subscriber exit");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "subscriber failed: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    // run() printed seen=0 (reader was stalled the whole run) …
    assert!(
        stdout.contains("exiting seen=0"),
        "test premise: nothing may be delivered before exit \
         (stall covers the run); got:\n{}",
        stdout
    );
    // … and the quiesce then delivered ALL of it before teardown.
    for n in ["got=7", "got=11", "got=23"] {
        assert!(
            stdout.contains(n),
            "exit quiesce must deliver the kernel-queued tail \
             ({} missing):\n{}",
            n,
            stdout
        );
    }

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);
}

/// 3. The bound: a connected peer that never sends and never
///    closes must not stall exit — the quiesce half-close turns
///    the reader's recv into EOF (empty queue), so the subscriber
///    exits promptly, well inside the 500ms default bound + slack.
#[test]
fn quiesce_does_not_stall_exit_on_a_silent_peer() {
    let sock = unique_socket_path("silent");
    let sub_bin = build("silent_sub", &exiting_sub_src(&sock));
    let holder_bin = build("silent_holder", &silent_holder_src(&sock));

    let sub = Command::new(&sub_bin)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    let mut holder = Command::new(&holder_bin)
        .spawn()
        .expect("spawn silent holder");

    let start = Instant::now();
    let out = sub.wait_with_output().expect("subscriber exit");
    let elapsed = start.elapsed();
    let _ = holder.kill();
    let _ = holder.wait();

    assert!(
        out.status.success(),
        "subscriber failed: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "exit stalled behind a silent peer: {:?}",
        elapsed
    );

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&holder_bin);
}
