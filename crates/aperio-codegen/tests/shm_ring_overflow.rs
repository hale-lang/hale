//! Form K7 (2026-05-20) — back-pressure policy validation.
//! Three tests, one per policy:
//!
//!   - `fail`  : publisher panics with clear diagnostic when the
//!               ring fills (no consumer draining). Process
//!               exits non-zero with a diagnostic naming the
//!               policy and suggesting alternatives.
//!   - `block` : publisher blocks until a consumer catches up,
//!               then completes. Validates the wait/wake
//!               mechanism (no deadlock when a consumer is
//!               alive).
//!   - `drop`  : publisher overwrites; >slot_count publishes
//!               complete cleanly. Regression guard for the
//!               pre-K7 behavior.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("k7-{}-{}-{}", label, std::process::id(), nanos)
}

fn build_binary(src: &str, label: &str) -> PathBuf {
    let prog = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_k7_{}.bin", unique_tag(label)));
    build_executable(&prog, &bin).expect("build");
    bin
}

#[test]
fn fail_policy_panics_when_ring_fills() {
    // Publisher fires more publishes than slot_count with no
    // consumer attached. Under `on_overflow: fail`, the 3rd
    // publish (overflowing a 2-slot ring) should hit the FAIL
    // branch in lotus_shm_ring_claim and the publish_shm_ring
    // wrapper panics with a clear diagnostic.
    let tag = unique_tag("fail");
    let shm_name = format!("/aperio-{}", tag);
    let src = format!(
        r#"
        type T {{ x: Int; y: Int; }}
        topic Foo {{ payload: T; }}
        locus Producer {{
            bus {{ publish Foo; }}
            birth() {{
                Foo <- T {{ x: 1, y: 10 }};
                Foo <- T {{ x: 2, y: 20 }};
                Foo <- T {{ x: 3, y: 30 }};
                Foo <- T {{ x: 4, y: 40 }};
                Foo <- T {{ x: 5, y: 50 }};
            }}
        }}
        main locus App {{
            bindings {{
                Foo: shm_ring("{shm_name}", slot_count: 2, on_overflow: fail) where zero_copy;
            }}
        }}
        fn main() {{ App {{ }}; Producer {{ }}; }}
    "#
    );
    let bin = build_binary(&src, "fail");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(
        !out.status.success(),
        "publisher should panic on fail policy, but exited 0. stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ring full") && stderr.contains("on_overflow: fail"),
        "expected fail-policy diagnostic, got stderr:\n{}",
        stderr
    );
    // The atexit hook should still have unlinked the ring even
    // though we exited via _exit(1) inside the publish call — wait,
    // _exit() bypasses atexit. So the ring may persist. Clean up.
    let stripped = shm_name.trim_start_matches('/');
    let _ = std::fs::remove_file(format!("/dev/shm/{}", stripped));
}

#[test]
fn block_policy_does_not_deadlock_with_live_consumer() {
    // Publisher fires more publishes than slot_count with a
    // consumer attached. Under `on_overflow: block`, the
    // publisher should briefly block when the ring fills, then
    // resume as the consumer drains. Both processes must exit
    // cleanly.
    //
    // slot_count: 4, n_msgs: 16 → publisher must wait at least
    // (n_msgs - slot_count) / drain_rate before completing. The
    // consumer drains at ~10us/msg (handler is a no-op println);
    // 12 ms of effective backlog spread over the publish loop is
    // fine.
    let tag = unique_tag("block");
    let shm_name = format!("/aperio-{}", tag);
    let n_msgs: i64 = 16;
    let slot_count: u64 = 4;

    let subscriber_src = format!(
        r#"
        type T {{ x: Int; y: Int; }}
        topic Foo {{ payload: T; }}
        locus Sub {{
            bus {{ subscribe Foo as on_foo of type T; }}
            fn on_foo(t: T) {{ println("got x=", t.x); }}
        }}
        main locus App {{
            bindings {{
                Foo: shm_ring("{shm_name}", slot_count: {slot_count}, on_overflow: block) where zero_copy;
            }}
        }}
        fn main() {{
            App {{ }}; Sub {{ }};
            time::sleep(500ms);
        }}
    "#
    );

    let publisher_src = format!(
        r#"
        type T {{ x: Int; y: Int; }}
        topic Foo {{ payload: T; }}
        locus Pub {{
            bus {{ publish Foo; }}
            birth() {{
                Foo <- T {{ x: 1,  y: 0 }};
                Foo <- T {{ x: 2,  y: 0 }};
                Foo <- T {{ x: 3,  y: 0 }};
                Foo <- T {{ x: 4,  y: 0 }};
                Foo <- T {{ x: 5,  y: 0 }};
                Foo <- T {{ x: 6,  y: 0 }};
                Foo <- T {{ x: 7,  y: 0 }};
                Foo <- T {{ x: 8,  y: 0 }};
                Foo <- T {{ x: 9,  y: 0 }};
                Foo <- T {{ x: 10, y: 0 }};
                Foo <- T {{ x: 11, y: 0 }};
                Foo <- T {{ x: 12, y: 0 }};
                Foo <- T {{ x: 13, y: 0 }};
                Foo <- T {{ x: 14, y: 0 }};
                Foo <- T {{ x: 15, y: 0 }};
                Foo <- T {{ x: 16, y: 0 }};
            }}
        }}
        main locus App {{
            bindings {{
                Foo: shm_ring("{shm_name}", slot_count: {slot_count}, on_overflow: block) where zero_copy;
            }}
        }}
        fn main() {{ App {{ }}; Pub {{ }}; }}
    "#
    );

    let sub_bin = build_binary(&subscriber_src, "blocksub");
    let pub_bin = build_binary(&publisher_src, "blockpub");

    let sub = Command::new(&sub_bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sub");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let pub_out = Command::new(&pub_bin).output().expect("run pub");
    let sub_out = sub.wait_with_output().expect("wait sub");

    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);

    assert!(
        pub_out.status.success(),
        "publisher should not deadlock under block policy. stderr: {}",
        String::from_utf8_lossy(&pub_out.stderr),
    );
    assert!(
        sub_out.status.success(),
        "subscriber failed: {}",
        String::from_utf8_lossy(&sub_out.stderr),
    );
    // Under block policy, NO messages should be lost. Consumer
    // sees all 16.
    let sub_stdout = String::from_utf8_lossy(&sub_out.stdout);
    for i in 1..=n_msgs {
        let want = format!("got x={}", i);
        assert!(
            sub_stdout.contains(&want),
            "subscriber missed `{}` under block policy. Full stdout:\n{}",
            want,
            sub_stdout
        );
    }
}

#[test]
fn drop_policy_overwrites_without_panic() {
    // Regression guard for pre-K7 behavior. Publisher fires more
    // publishes than slot_count with no consumer. Under
    // `on_overflow: drop`, the publisher completes silently
    // (overwrites wrapped slots).
    let tag = unique_tag("drop");
    let shm_name = format!("/aperio-{}", tag);
    let src = format!(
        r#"
        type T {{ x: Int; y: Int; }}
        topic Foo {{ payload: T; }}
        locus Producer {{
            bus {{ publish Foo; }}
            birth() {{
                Foo <- T {{ x: 1, y: 0 }};
                Foo <- T {{ x: 2, y: 0 }};
                Foo <- T {{ x: 3, y: 0 }};
                Foo <- T {{ x: 4, y: 0 }};
                Foo <- T {{ x: 5, y: 0 }};
            }}
        }}
        main locus App {{
            bindings {{
                Foo: shm_ring("{shm_name}", slot_count: 2, on_overflow: drop) where zero_copy;
            }}
        }}
        fn main() {{ App {{ }}; Producer {{ }}; }}
    "#
    );
    let bin = build_binary(&src, "drop");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "drop publisher should exit cleanly. stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    // atexit fires on clean exit → ring is unlinked.
    let stripped = shm_name.trim_start_matches('/');
    assert!(
        !PathBuf::from(format!("/dev/shm/{}", stripped)).exists(),
        "drop publisher's ring should be unlinked at clean exit",
    );
}
