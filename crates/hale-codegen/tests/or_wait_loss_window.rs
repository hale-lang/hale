//! GH #255 phase 1 — `or wait` publisher park.
//!
//! A publish with `or wait` on a transport-bound topic parks
//! through the binding's loss window instead of taking the
//! counted `dropped_lost` drop: the wait pumps main's drain
//! (loss dispatch → on_failure → restart-as-reconnect) and the
//! send proceeds on the re-armed binding. Assertions ride the
//! GH #236 counters: `dropped_lost=0` (nothing window-dropped)
//! and `waits=1` (the park actually happened), plus delivery to
//! the post-reconnect peer.
//!
//! The teardown test pins the no-hang property: a publisher
//! parked past main teardown wakes into the raise path
//! (`BusWaitAborted`, non-zero exit) rather than deadlocking
//! main's pinned join — a regression here shows up as a test
//! timeout.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_orwait_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn build_peer_driver(tag: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_orwait_peer_{}", tag));
    let status = Command::new("clang")
        .arg(manifest.join("tests").join("transport_driver.c"))
        .arg(manifest.join("runtime").join("lotus_arena.c"))
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building peer driver");
    bin
}

fn unique_socket_path(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}/hale-255-orwait-{}-{}-{}.sock",
        std::env::temp_dir().display(),
        tag,
        std::process::id(),
        nanos
    )
}

#[test]
fn or_wait_parks_through_loss_window_no_drops() {
    let sock = unique_socket_path("park");
    // No protective sleeps between loss and the next publish —
    // the wait itself must absorb the reconnect gap. Send 2 is
    // the detection casualty (the loss is only discovered when
    // its send fails — that's a send_failure, not a window
    // drop); send 3 parks and lands on the re-armed binding.
    let src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        main locus App {{
            bus {{ publish Evt; }}
            bindings {{ Evt: unix("{}", role: connect); }}
            on_failure(t: std::bus::UnixTransport, err: ClosureViolation) {{
                println("[sup] link lost — restarting");
                restart (t);
            }}
            run() {{
                Evt <- T {{ n: 1 }} or wait;
                std::time::sleep(400ms);
                Evt <- T {{ n: 2 }} or wait;
                Evt <- T {{ n: 3 }} or wait;
                println("recovered");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let bin = build("park", &src);
    let driver = build_peer_driver("park");

    // Peer 1: takes message 1, exits → EPIPE on send 2.
    let peer1 = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn peer 1");
    let mut publisher = Command::new(&bin)
        .env("LOTUS_BUS_COUNTERS_DUMP", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn publisher");
    let _ = peer1.wait_with_output();
    // Peer 2 binds while the publisher is parked in `or wait` on
    // send 3; the reconnect's connect-with-retry absorbs the gap.
    let peer2 = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn peer 2");
    let pub_out = publisher.wait_with_output().expect("publisher output");
    let peer2_out = peer2.wait_with_output().expect("peer 2 output");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&pub_out.stdout);
    let stderr = String::from_utf8_lossy(&pub_out.stderr);
    assert!(
        pub_out.status.success(),
        "or-wait publisher with a restart policy must survive the loss.\n\
         stdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("[sup] link lost") && stdout.contains("recovered"),
        "expected supervision + recovery.\nstdout: {:?}",
        stdout
    );
    assert!(
        !peer2_out.stdout.is_empty(),
        "peer 2 received nothing — the parked publish did not resume.\n\
         publisher stdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    // The counters are the contract: the park happened, and NO
    // publish was window-dropped.
    let counters = stderr
        .lines()
        .find(|l| l.contains("[bus counters]") && l.contains("subject=evt"))
        .unwrap_or_else(|| {
            panic!("no counters line for evt.\nstderr: {:?}", stderr)
        });
    assert!(
        counters.contains("dropped_lost=0"),
        "or wait must prevent window drops.\ncounters: {}",
        counters
    );
    assert!(
        counters.contains("waits=1"),
        "expected exactly one park.\ncounters: {}",
        counters
    );
}

#[test]
fn or_wait_parked_past_teardown_raises_not_hangs() {
    let sock = unique_socket_path("abort");
    // A PINNED child publishes `or wait` after the peer dies;
    // main's run() returns immediately, so by the time the
    // child's send fails and it parks, main is in teardown and
    // can never dispatch the loss. The teardown abort must wake
    // the child into the raise path — a hang here (the pinned
    // join waiting on a parked waiter) is the regression.
    let src = format!(
        r#"
        type T {{ n: Int = 0; }}
        topic Evt {{ payload: T; subject: "evt"; }}
        locus Pusher {{
            bus {{ publish Evt; }}
            run() {{
                Evt <- T {{ n: 1 }} or wait;
                std::time::sleep(400ms);
                Evt <- T {{ n: 2 }} or wait;
                Evt <- T {{ n: 3 }} or wait;
                println("pusher done");
            }}
        }}
        main locus App {{
            params {{ p: Pusher = Pusher {{ }}; }}
            placement {{ p: pinned(core = 0); }}
            bindings {{ Evt: unix("{}", role: connect); }}
            on_failure(t: std::bus::UnixTransport, err: ClosureViolation) {{
                restart (t);
            }}
            run() {{
                println("main returns");
            }}
        }}
        fn main() {{ App {{ }}; }}
    "#,
        sock
    );
    let bin = build("abort", &src);
    let driver = build_peer_driver("abort");
    let peer1 = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn peer 1");
    let out = Command::new(&bin).output().expect("run publisher");
    let _ = peer1.wait_with_output();
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);
    let _ = std::fs::remove_file(&sock);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The process must TERMINATE (the deadline oracle is the
    // real assertion — a hang times the test out) and the parked
    // wait must surface as the raise, not silence.
    assert!(
        !out.status.success(),
        "a wait parked past teardown must raise, not exit clean.\n\
         stdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        stderr.contains("BusWaitAborted"),
        "expected the BusWaitAborted raise diagnostic.\n\
         stdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
}
