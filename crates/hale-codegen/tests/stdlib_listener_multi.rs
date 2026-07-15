//! m83: multi-accept Listener using m80 (function pointers)
//! + m81 (Stream + non-self method calls) + m82 (let-bound
//! scope-bound dissolve).
//!
//! `__StdIoTcpListener` now accepts in a loop bounded by
//! `max_accepts` and dispatches each connection through an
//! `on_connection: fn(Stream)` callback. Per-connection Stream
//! lifecycles are owned by `__handle_one_connection`, a free
//! fn whose scope-exit flush dissolves the Stream — closing
//! the fd between iterations. Tests prove:
//!
//! 1. The default callback runs end-to-end against a real
//!    Rust TCP client.
//! 2. A user-supplied callback receives a usable Stream and
//!    can call `s.recv` + `s.send` against it.
//! 3. The accept loop iterates max_accepts times, each
//!    iteration's fd closes after the callback returns.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;

use hale_codegen::build_executable;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_listener_multi_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

#[test]
fn listener_default_callback_handles_one_connection() {
    // Existing m73b behavior preserved: with max_accepts=1 (the
    // default) and the default `__default_on_connection`
    // callback, the Listener accepts exactly one connection,
    // runs the default callback (which logs the fd), and the
    // per-connection Stream's dissolve closes the fd.
    let port = pick_free_port();
    let src = format!(
        r#"
        fn main() {{
            std::io::tcp::Listener {{
                host: "127.0.0.1",
                port: {},
                max_accepts: 1
            }};
        }}
        "#,
        port
    );
    let bin = build_hale("default_cb", &src);

    // Spawn the Hale Listener; give it a moment to bind.
    let bin_path = bin.clone();
    let server_handle = thread::spawn(move || {
        Command::new(&bin_path).output().expect("run listener")
    });
    thread::sleep(Duration::from_millis(150));

    // One client connection, then close.
    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let _ = client.write_all(b"hi");
    drop(client);

    let out = server_handle.join().expect("listener thread joined");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("__default_on_connection fd="),
        "default callback didn't fire; got: {:?}",
        stdout
    );
}

#[test]
fn listener_user_callback_receives_usable_stream() {
    // The whole m80+m81+m82+m83 chain: a user-supplied free fn
    // is passed by name as on_connection. Listener loops once,
    // accepts, hands the Stream to the callback. The callback
    // calls `s.recv` + `s.send` against the Stream — proving
    // (a) fn-pointer dispatch reaches the callback, (b) Stream
    // is alive across method calls (m82 scope-bound dissolve),
    // (c) m81 send/recv primitives round-trip through real
    // socket I/O.
    let port = pick_free_port();
    let src = format!(
        r#"
        fn echo_handler(s: std::io::tcp::Stream) {{
            let req = s.recv(64) or raise;
            println("server got=", req);
            s.send("server says hi back") or raise;
        }}

        fn main() {{
            std::io::tcp::Listener {{
                host: "127.0.0.1",
                port: {},
                max_accepts: 1,
                on_connection: echo_handler
            }};
        }}
        "#,
        port
    );
    let bin = build_hale("user_cb", &src);

    let bin_path = bin.clone();
    let server_handle = thread::spawn(move || {
        Command::new(&bin_path).output().expect("run listener")
    });
    thread::sleep(Duration::from_millis(150));

    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    client.write_all(b"hello-server").expect("client write");
    let mut buf = [0u8; 64];
    let n = client.read(&mut buf).expect("client read");
    let response = String::from_utf8_lossy(&buf[..n]).to_string();
    drop(client);

    let out = server_handle.join().expect("listener thread joined");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("server got=hello-server"),
        "server didn't receive client bytes; got: {:?}",
        stdout
    );
    assert!(
        response.contains("server says hi back"),
        "client didn't see server response; got: {:?}",
        response
    );
}

#[test]
fn listener_handles_multiple_connections_in_sequence() {
    // max_accepts = 3: Listener loops three times. Each
    // iteration accepts a fresh fd, hands it to the callback,
    // and the per-connection Stream dissolves at end of
    // __handle_one_connection (closing the fd) before the
    // next accept begins. All three clients see their unique
    // response, proving fds don't bleed between iterations.
    let port = pick_free_port();
    let src = format!(
        r#"
        fn counted(s: std::io::tcp::Stream) {{
            let req = s.recv(64) or raise;
            println("handled=", req);
            s.send("ack") or raise;
        }}

        fn main() {{
            std::io::tcp::Listener {{
                host: "127.0.0.1",
                port: {},
                max_accepts: 3,
                on_connection: counted
            }};
        }}
        "#,
        port
    );
    let bin = build_hale("multi", &src);

    let bin_path = bin.clone();
    let server_handle = thread::spawn(move || {
        Command::new(&bin_path).output().expect("run listener")
    });
    thread::sleep(Duration::from_millis(150));

    let mut acks = Vec::new();
    for tag in &["one", "two", "three"] {
        let mut client = TcpStream::connect(("127.0.0.1", port))
            .expect("client connect");
        client
            .write_all(format!("conn-{}", tag).as_bytes())
            .expect("client write");
        let mut buf = [0u8; 16];
        let n = client.read(&mut buf).expect("client read");
        acks.push(String::from_utf8_lossy(&buf[..n]).to_string());
        drop(client);
    }

    let out = server_handle.join().expect("listener thread joined");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for tag in &["one", "two", "three"] {
        assert!(
            stdout.contains(&format!("handled=conn-{}", tag)),
            "missing handled=conn-{} in stdout: {:?}",
            tag,
            stdout
        );
    }
    assert_eq!(acks.len(), 3);
    for ack in &acks {
        assert!(
            ack.contains("ack"),
            "client didn't receive ack; got: {:?}",
            ack
        );
    }
}
