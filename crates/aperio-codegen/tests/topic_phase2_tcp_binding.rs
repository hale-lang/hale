//! End-to-end exercise of the v1.x Phase 2 `bindings { Topic:
//! tcp("host", port) : connect; }` form. Sibling to
//! `bus_config.rs` (which drives the same runtime path through
//! `LOTUS_BUS_CONFIG` + unix sockets); this test instead drives
//! the codegen-emitted `lotus_bus_register_remote` call site
//! that comes from a literal `bindings` entry in the `main`
//! locus.
//!
//! Listener side: the `transport_tcp_driver.c` harness used by
//! `transport_tcp.rs` — it reads framed messages off a
//! `lotus_tcp_t` and writes each one (followed by "\n----\n")
//! to stdout. That gives us a way to capture the publisher's
//! wire bytes without re-implementing the framer or running a
//! second aperio binary.
//!
//! Wire-format note: the publisher serializes `Ping { n: Int; }`
//! by walking fields (m70 codec); for a single Int that's 8
//! bytes little-endian. The TCP adapter wraps those 8 bytes in
//! its 8-byte LE length header during `lotus_tcp_send`; the
//! driver's `lotus_tcp_recv` strips the header and hands the
//! payload to the test as-is.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn runtime_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("runtime");
    p.push("lotus_arena.c");
    p
}

fn tcp_driver_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("tests");
    p.push("transport_tcp_driver.c");
    p
}

fn build_tcp_driver(tag: &str) -> PathBuf {
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_m105_tcp_listener_{}", tag));
    let status = Command::new("clang")
        .arg(tcp_driver_c_path())
        .arg(runtime_c_path())
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building tcp driver");
    bin
}

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

#[test]
fn bindings_block_routes_tcp_publisher_to_remote_listener() {
    let sentinel: i64 = 0x4142_4344_4546_4748;
    let port = pick_free_port();

    let src = format!(
        r#"
        type Ping {{
            n: Int;
        }}

        topic Evt {{
            payload: Ping;
            subject: "evt";
        }}

        locus Sub {{
            bus {{
                subscribe Evt as on_evt;
            }}
            fn on_evt(p: Ping) {{
                println("local sub got n=", p.n);
            }}
        }}

        locus Pub {{
            bus {{
                publish Evt;
            }}
            birth() {{
                Evt <- Ping {{ n: {sentinel} }};
            }}
        }}

        main locus App {{
            bindings {{
                Evt: tcp("127.0.0.1", {port}) : connect;
            }}
        }}

        fn main() {{
            App {{ }};
            Sub {{ }};
            Pub {{ }};
        }}
    "#,
        sentinel = sentinel,
        port = port,
    );

    let driver = build_tcp_driver("bindings");

    // Listener first: lotus_tcp_create (LISTEN) blocks on accept;
    // the publisher's connect retries for ~1s on ECONNREFUSED so
    // either ordering works, but spawning listener first is the
    // natural shape.
    let listener = Command::new(&driver)
        .arg("listen")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let program = aperio_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "lotus_m105_tcp_publisher_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    build_executable(&program, &bin).expect("build publisher");

    let pub_out = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");

    let listen_out = listener.wait_with_output().expect("listener wait");

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);

    assert!(
        pub_out.status.success(),
        "publisher exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        pub_out.status,
        String::from_utf8_lossy(&pub_out.stdout),
        String::from_utf8_lossy(&pub_out.stderr),
    );
    assert!(
        listen_out.status.success(),
        "listener exited non-zero: {:?}\nstderr: {}",
        listen_out.status,
        String::from_utf8_lossy(&listen_out.stderr),
    );

    // The listener writes each framed message followed by
    // "\n----\n". With one publish from Pub.birth() we expect
    // exactly one message, 8 bytes long, holding the LE-encoded
    // sentinel.
    let stdout = &listen_out.stdout;
    let delim = b"\n----\n";
    let split_pos = stdout
        .windows(delim.len())
        .position(|w| w == delim)
        .expect("listener stdout must contain message delimiter");
    let msg = &stdout[..split_pos];
    assert_eq!(
        msg.len(),
        8,
        "expected 8-byte Int wire form; got {} bytes: {:?}\npub stderr: {}",
        msg.len(),
        msg,
        String::from_utf8_lossy(&pub_out.stderr),
    );
    let mut sentinel_bytes = [0u8; 8];
    sentinel_bytes.copy_from_slice(msg);
    let received = i64::from_le_bytes(sentinel_bytes);
    assert_eq!(
        received, sentinel,
        "wire bytes should match the published sentinel; pub stderr: {}",
        String::from_utf8_lossy(&pub_out.stderr),
    );

    // Local subscriber's println should have fired in the
    // publisher process too — confirms a bindings-block entry
    // still fans local subscribers + remote in one dispatch
    // (no regression of bus_config's local-and-remote property).
    assert!(
        String::from_utf8_lossy(&pub_out.stdout)
            .contains(&format!("local sub got n={}", sentinel)),
        "publisher's local subscriber should also have received the \
         publish; stdout was: {:?}",
        String::from_utf8_lossy(&pub_out.stdout),
    );
}

#[test]
fn bindings_block_routes_tcp_remote_to_local_subscriber() {
    // Listen-side mirror of the previous test. Exercises:
    //   - the TCP arm of lotus_bus_reader_thread_main (accept,
    //     recv-loop, dispatch into local handlers)
    //   - the TCP arm of lotus_bus_remote_destroy_all (shutdown
    //     on lotus_tcp_t.conn_fd to unblock recv, then join)
    //
    // Peer side is the transport_tcp_driver's `connect` mode,
    // which uses lotus_tcp_send (so the same 8-byte LE length
    // framer the listening side decodes). The byte payload is
    // the wire form for `Ping { n: sentinel }` — for a single
    // Int that's the 8-byte LE-encoded i64. Choosing a sentinel
    // with no embedded NULs so it survives the driver's strlen-
    // based argv → send path: 0x4142434445464748 LE = "HGFEDCBA".
    let sentinel: i64 = 0x4142_4344_4546_4748;
    let port = pick_free_port();

    let src = format!(
        r#"
        type Ping {{
            n: Int;
        }}

        topic Evt {{
            payload: Ping;
            subject: "evt";
        }}

        locus Sub {{
            bus {{
                subscribe Evt as on_evt;
            }}
            fn on_evt(p: Ping) {{
                println("sub got n=", p.n);
            }}
        }}

        main locus App {{
            bindings {{
                Evt: tcp("127.0.0.1", {port}) : listen;
            }}
        }}

        fn main() {{
            App {{ }};
            Sub {{ }};
            time::sleep(500ms);
            yield;
        }}
    "#,
        port = port,
    );

    let driver = build_tcp_driver("bindings-listen");

    let program = aperio_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "lotus_m105_tcp_listener_bin_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    build_executable(&program, &bin).expect("build listener binary");

    // Spawn the aperio listener first so its reader thread is
    // inside accept() before the driver connects. The driver's
    // connect-with-retry shields us from a small race here, but
    // listener-first keeps the test deterministic (and matches
    // the bus_subscriber.rs unix flow).
    let listener = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener binary");
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Driver sends one framed message of the 8 LE bytes that
    // decode back to the sentinel int.
    let sentinel_le = sentinel.to_le_bytes();
    let sentinel_str =
        std::str::from_utf8(&sentinel_le).expect("sentinel LE has no NULs");

    let connect_out = Command::new(&driver)
        .arg("connect")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(sentinel_str)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run connector");

    let listen_out = listener.wait_with_output().expect("listener wait");

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&driver);

    assert!(
        connect_out.status.success(),
        "connector exited non-zero: {:?}\nstderr: {}",
        connect_out.status,
        String::from_utf8_lossy(&connect_out.stderr),
    );
    assert!(
        listen_out.status.success(),
        "listener binary exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        listen_out.status,
        String::from_utf8_lossy(&listen_out.stdout),
        String::from_utf8_lossy(&listen_out.stderr),
    );

    let stdout = String::from_utf8_lossy(&listen_out.stdout);
    let expected = format!("sub got n={}", sentinel);
    assert!(
        stdout.contains(&expected),
        "listener binary should have dispatched the recv'd message \
         to the local subscriber; expected '{}' in stdout, got: {:?}\n\
         stderr: {}",
        expected,
        stdout,
        String::from_utf8_lossy(&listen_out.stderr),
    );
}
