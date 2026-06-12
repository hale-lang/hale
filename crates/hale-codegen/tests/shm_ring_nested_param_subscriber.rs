//! WS3.5 — shm_ring subscriber instantiated as a *nested locus
//! param* (not top-level in `fn main()`).
//!
//! pond/fathom reported (FRICTION § shm-ring remaining nit) that an
//! shm_ring subscriber only spawned its reader thread when
//! instantiated top-level (`Sub { }` directly in `fn main()`); as a
//! nested locus-param (`params { sub: Sub = Sub { }; }`) it
//! silently no-op'd — no reader thread, no dispatch. The handoff's
//! acceptable outcomes were "wire it or reject at typecheck"; this
//! verifies it is in fact *wired* at HEAD and locks it in.
//!
//! The subscriber here is a param of the **main locus** itself —
//! the canonical gateway shape (a gateway locus owns the shm_ring
//! binding and a child subscriber as a param). A sibling test
//! covers an intermediate parent.
//!
//! Mirrors `shm_ring_hale_subscriber.rs`: a publisher binary
//! writes ticks into a named ring; a subscriber binary whose Sub is
//! a nested param must still receive all of them.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("ws35-{}-{}-{}", label, std::process::id(), nanos)
}

fn build_binary(src: &str, label: &str) -> PathBuf {
    let prog = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_ws35_{}.bin", unique_tag(label)));
    build_executable(&prog, &bin).expect("build");
    bin
}

const PUBLISHER: &str = r#"
    type TickPayload { px: Int; sz: Int; }
    topic Tick { payload: TickPayload; }
    locus Producer {
        bus { publish Tick; }
        birth() {
            Tick <- TickPayload { px: 1, sz: 7 };
            Tick <- TickPayload { px: 2, sz: 14 };
            Tick <- TickPayload { px: 3, sz: 21 };
            Tick <- TickPayload { px: 4, sz: 28 };
            Tick <- TickPayload { px: 5, sz: 35 };
        }
    }
    main locus App {
        bindings { Tick: shm_ring("__NAME__", slot_count: 8, on_overflow: drop) where zero_copy; }
    }
    fn main() { App { }; Producer { }; }
"#;

/// Subscriber whose `Sub` is a **param of the main locus** (the
/// gateway shape) rather than a top-level `Sub { }`.
const SUBSCRIBER_MAIN_PARAM: &str = r#"
    type TickPayload { px: Int; sz: Int; }
    topic Tick { payload: TickPayload; }
    locus Sub {
        bus { subscribe Tick as on_tick; }
        fn on_tick(t: TickPayload) { println("tick px=", t.px, " sz=", t.sz); }
    }
    main locus App {
        params { sub: Sub = Sub { }; }
        bindings { Tick: shm_ring("__NAME__", slot_count: 8, on_overflow: drop) where zero_copy; }
    }
    fn main() { App { }; time::sleep(500ms); }
"#;

/// Subscriber whose `Sub` is a param of an **intermediate** locus
/// instantiated in `fn main()`.
const SUBSCRIBER_NESTED_PARENT: &str = r#"
    type TickPayload { px: Int; sz: Int; }
    topic Tick { payload: TickPayload; }
    locus Sub {
        bus { subscribe Tick as on_tick; }
        fn on_tick(t: TickPayload) { println("tick px=", t.px, " sz=", t.sz); }
    }
    locus Parent { params { sub: Sub = Sub { }; } }
    main locus App {
        bindings { Tick: shm_ring("__NAME__", slot_count: 8, on_overflow: drop) where zero_copy; }
    }
    fn main() { App { }; Parent { }; time::sleep(500ms); }
"#;

fn run_case(subscriber_template: &str, label: &str) {
    let shm_name = format!("/hale-{}", unique_tag(label));
    let sub_src = subscriber_template.replace("__NAME__", &shm_name);
    let pub_src = PUBLISHER.replace("__NAME__", &shm_name);

    let sub_bin = build_binary(&sub_src, &format!("sub-{}", label));
    let pub_bin = build_binary(&pub_src, &format!("pub-{}", label));

    // Subscriber first so its reader thread is polling.
    let subscriber = Command::new(&sub_bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subscriber");
    std::thread::sleep(std::time::Duration::from_millis(60));
    let pub_out = Command::new(&pub_bin).output().expect("run publisher");
    let sub_out = subscriber.wait_with_output().expect("wait sub");
    let _ = std::fs::remove_file(&sub_bin);
    let _ = std::fs::remove_file(&pub_bin);

    assert!(
        pub_out.status.success(),
        "[{}] publisher failed: {:?} stderr={}",
        label,
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stderr)
    );
    assert!(
        sub_out.status.success(),
        "[{}] subscriber failed: {:?} stderr={}",
        label,
        sub_out.status,
        String::from_utf8_lossy(&sub_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&sub_out.stdout);
    for i in 1..=5 {
        let want = format!("tick px={} sz={}", i, i * 7);
        assert!(
            stdout.contains(&want),
            "[{}] nested-param subscriber missing `{}` — the reader \
             thread did not spawn for the nested instantiation. \
             Full stdout:\n{}",
            label,
            want,
            stdout
        );
    }
}

#[test]
fn shm_ring_subscriber_as_main_locus_param_receives() {
    run_case(SUBSCRIBER_MAIN_PARAM, "mainparam");
}

#[test]
fn shm_ring_subscriber_nested_in_parent_receives() {
    run_case(SUBSCRIBER_NESTED_PARENT, "parent");
}
