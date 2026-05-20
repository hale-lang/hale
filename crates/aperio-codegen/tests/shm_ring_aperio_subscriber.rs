//! Form K6b (2026-05-20) — end-to-end test for the Aperio-side
//! shm_ring subscriber. Two Aperio binaries:
//!
//!   publisher : Producer locus that does `Tick <- TickPayload {
//!               ... }` 5 times in its birth() lifecycle. Binding
//!               for `Tick` is `shm_ring(...)`.
//!   subscriber: Sub locus that has `bus { subscribe Tick as
//!               on_tick of type TickPayload; }` plus a handler
//!               that prints `tick px=N sz=M`. Same shm_ring
//!               binding.
//!
//! The subscriber spawns first (so its reader thread is polling
//! when the publisher creates the ring), the publisher publishes
//! 5 ticks, the subscriber's stdout is asserted to contain all 5
//! expected lines.
//!
//! Validates: register_subscriber_shm_ring + reader thread
//! polling + handler dispatch with the right slot pointer; the
//! handler reads fields through the slot pointer correctly.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("k6b-{}-{}-{}", label, std::process::id(), nanos)
}

fn build_binary(src: &str, label: &str) -> PathBuf {
    let prog = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_k6b_{}.bin", unique_tag(label)));
    build_executable(&prog, &bin).expect("build");
    bin
}

#[test]
fn aperio_subscriber_reads_shm_ring_publishes() {
    let shm_name = format!("/aperio-{}", unique_tag("e2e"));
    let n_msgs: i64 = 5;
    let slot_count: u64 = 8;

    // Subscriber prints `tick px=N sz=M` for each received tick.
    // Main sleeps long enough to let the reader thread receive
    // all expected messages before the process exits.
    let subscriber_src = format!(
        r#"
        type TickPayload {{
            px: Int;
            sz: Int;
        }}
        topic Tick {{ payload: TickPayload; }}

        locus Sub {{
            bus {{ subscribe Tick as on_tick of type TickPayload; }}
            fn on_tick(t: TickPayload) {{
                println("tick px=", t.px, " sz=", t.sz);
            }}
        }}

        main locus App {{
            bindings {{
                Tick: shm_ring("{shm_name}", slot_count: {slot_count}, on_overflow: drop) where zero_copy;
            }}
        }}

        fn main() {{
            App {{ }};
            Sub {{ }};
            // Give the reader thread time to receive + dispatch
            // all expected publishes.
            time::sleep(500ms);
        }}
    "#,
        shm_name = shm_name,
        slot_count = slot_count,
    );

    let publisher_src = format!(
        r#"
        type TickPayload {{
            px: Int;
            sz: Int;
        }}
        topic Tick {{ payload: TickPayload; }}

        locus Producer {{
            bus {{ publish Tick; }}
            birth() {{
                Tick <- TickPayload {{ px: 1, sz: 7 }};
                Tick <- TickPayload {{ px: 2, sz: 14 }};
                Tick <- TickPayload {{ px: 3, sz: 21 }};
                Tick <- TickPayload {{ px: 4, sz: 28 }};
                Tick <- TickPayload {{ px: 5, sz: 35 }};
            }}
        }}

        main locus App {{
            bindings {{
                Tick: shm_ring("{shm_name}", slot_count: {slot_count}, on_overflow: drop) where zero_copy;
            }}
        }}

        fn main() {{
            App {{ }};
            Producer {{ }};
        }}
    "#,
        shm_name = shm_name,
        slot_count = slot_count,
    );

    let sub_bin = build_binary(&subscriber_src, "sub");
    let pub_bin = build_binary(&publisher_src, "pub");

    // Spawn the subscriber first so its reader thread is polling
    // when the publisher creates the ring. The subscriber's own
    // App {} also creates the ring (race-tolerant via
    // lotus_shm_ring_open's "attach if exists" path).
    let subscriber = Command::new(&sub_bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");

    std::thread::sleep(std::time::Duration::from_millis(50));

    let pub_out = Command::new(&pub_bin)
        .output()
        .expect("run publisher");

    let sub_out = subscriber.wait_with_output().expect("wait sub");

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);

    assert!(
        pub_out.status.success(),
        "publisher failed: status={:?}\nstdout: {}\nstderr: {}",
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stdout),
        String::from_utf8_lossy(&pub_out.stderr),
    );
    assert!(
        sub_out.status.success(),
        "subscriber failed: status={:?}\nstdout: {}\nstderr: {}",
        sub_out.status,
        String::from_utf8_lossy(&sub_out.stdout),
        String::from_utf8_lossy(&sub_out.stderr),
    );
    let sub_stdout = String::from_utf8_lossy(&sub_out.stdout);
    for i in 1..=n_msgs {
        let want = format!("tick px={} sz={}", i, i * 7);
        assert!(
            sub_stdout.contains(&want),
            "subscriber stdout missing `{}`. Full stdout:\n{}",
            want,
            sub_stdout
        );
    }

    // Form K-cleanup (2026-05-20): after both binaries have
    // exited cleanly, the SHM object should be shm_unlink'd by
    // whichever process created it. /dev/shm/ should be empty
    // for this ring name.
    let stripped = shm_name.trim_start_matches('/');
    let shm_path = PathBuf::from(format!("/dev/shm/{}", stripped));
    assert!(
        !shm_path.exists(),
        "atexit cleanup failed: `{}` persists in /dev/shm/ after \
         both publisher and subscriber exited",
        shm_name
    );
}
