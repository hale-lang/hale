//! m81 + m82: Stream locus + send/recv methods + non-self
//! method call support + low-level TCP primitives + the
//! locus-all-the-way-down lifecycle fix.
//!
//! What ships:
//!   - `__StdIoTcpStream` locus (bundled stdlib.hl) with
//!     conn_fd param, `fn send(msg: String) -> Int`,
//!     `fn recv(max: Int) -> String`, and `dissolve()` that
//!     closes the fd.
//!   - `std::io::tcp::__send` / `__recv` / `__connect`
//!     path-call primitives wiring lotus_tcp_send_str /
//!     lotus_tcp_recv_str / lotus_tcp_connect.
//!   - Non-self method calls (`obj.method(args)`) — the
//!     language addition needed for `s.send(msg)` style.
//!   - m82: locus-all-the-way-down. `let s = Stream { conn_fd:
//!     fd }; s.send(...)` now works — the binding is the
//!     user-visible handle, the locus instance lives until
//!     the binding's enclosing fn returns, and dissolve fires
//!     at scope exit instead of at the end of the
//!     struct-literal expression.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use hale_codegen::build_executable;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_stream_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn pick_free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

#[test]
fn non_self_method_call_works_on_user_locus() {
    // m81 prerequisite: `obj.method(args)` for non-self obj.
    // Verifies in isolation — no TCP, no custom dissolve.
    let src = r#"
        locus Greeter {
            params { name: String = "world"; }
            fn greet() {
                println("hello, ", self.name);
            }
        }

        fn main() {
            let g = Greeter { name: "stream-test" };
            g.greet();
        }
    "#;
    let bin = build_hale("greeter", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("hello, stream-test"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn non_self_method_call_with_args_and_return() {
    // Method that takes args + returns a value, called on a
    // non-self locus reference. Exercises the value-returning
    // path of lower_external_method_call.
    let src = r#"
        locus Adder {
            params { base: Int = 10; }
            fn plus(x: Int) -> Int {
                return self.base + x;
            }
        }

        fn main() {
            let a = Adder { base: 100 };
            let r = a.plus(7);
            println("r=", r);
        }
    "#;
    let bin = build_hale("adder", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("r=107"), "got: {:?}", stdout);
}

#[test]
fn tcp_primitives_round_trip_via_connect_send_recv() {
    // The C-level primitives — __connect / __send / __recv /
    // __close_fd — round-trip cleanly through a Rust echo
    // server. This is the value m81 ships at the substrate
    // level; the Stream locus on top is a thin wrapper m82
    // exercises end-to-end.
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let (server_done_tx, server_done_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 64];
        let n = sock.read(&mut buf).expect("read");
        let echo = format!("ECHO:{}", String::from_utf8_lossy(&buf[..n]));
        sock.write_all(echo.as_bytes()).expect("write");
        let _ = sock.shutdown(std::net::Shutdown::Both);
        let _ = server_done_tx.send(());
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::__connect("127.0.0.1", {});
            std::io::tcp::__send(fd, "hello-stream");
            let resp = std::io::tcp::__recv(fd, 64);
            println("got=", resp);
            std::io::tcp::__close_fd(fd);
        }}
        "#,
        port
    );
    let bin = build_hale("primitives", &src);
    let out = Command::new(&bin).output().expect("run hale");
    let _ = std::fs::remove_file(&bin);
    let _ = server_done_rx
        .recv_timeout(std::time::Duration::from_secs(2));

    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("got=ECHO:hello-stream"),
        "expected echoed bytes; got: {:?}",
        stdout
    );
}

#[test]
fn stream_let_binding_round_trips_via_send_recv() {
    // m82: the documented-broken pattern from m81. With locus-
    // all-the-way-down, `let s = Stream { conn_fd: fd }` defers
    // Stream's dissolve to end-of-fn instead of end-of-
    // struct-literal-expression, so the user-visible binding
    // (the handle `s`) stays valid for subsequent method calls.
    // Round-trips through a real Rust echo server to prove the
    // fd is open across send + recv and only closes when main
    // returns.
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let (server_done_tx, server_done_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 64];
        let n = sock.read(&mut buf).expect("read");
        let echo = format!(
            "ECHO-LET:{}",
            String::from_utf8_lossy(&buf[..n])
        );
        sock.write_all(echo.as_bytes()).expect("write");
        let _ = sock.shutdown(std::net::Shutdown::Both);
        let _ = server_done_tx.send(());
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::__connect("127.0.0.1", {});
            let s = std::io::tcp::Stream {{ conn_fd: fd }};
            s.send("via-let") or raise;
            let resp = s.recv(64) or raise;
            println("got=", resp);
        }}
        "#,
        port
    );
    let bin = build_hale("let_binding", &src);
    let out = Command::new(&bin).output().expect("run hale");
    let _ = std::fs::remove_file(&bin);
    let _ = server_done_rx
        .recv_timeout(std::time::Duration::from_secs(2));

    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("got=ECHO-LET:via-let"),
        "expected echoed bytes via let-bound Stream; got: {:?}",
        stdout
    );
}

#[test]
fn borrowed_stream_does_not_close_shared_fd() {
    // 2026-05-29 (Bug B): a connection-owning locus that wraps its
    // long-lived fd in a *transient* Stream per operation — the
    // WsServerConn shape — must not have that wrapper's dissolve
    // close the shared fd. With `owns_fd: false`, two successive
    // `ping`s (each building + dropping its own borrowed Stream)
    // both reach the server. Pre-fix (unconditional __close_fd on
    // dissolve) the first ping closed the fd and the second send
    // hit a dead socket, so the server only ever saw "AAA".
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let (got_tx, got_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("set timeout");
        let mut acc = String::new();
        let mut buf = [0u8; 64];
        loop {
            match sock.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if acc.contains("AAA") && acc.contains("BBB") {
                        let _ = sock.write_all(b"BOTH");
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = got_tx.send(acc);
    });

    let src = format!(
        r#"
        locus Conn {{
            params {{ fd: Int = -1; }}
            // each ping wraps the SHARED fd in a borrowed Stream
            // that dissolves at fn-exit — must not close self.fd.
            fn ping(tag: String) {{
                let s = std::io::tcp::Stream {{ conn_fd: self.fd, owns_fd: false }};
                s.send(tag) or raise;
            }}
            fn ack() -> String {{
                let s = std::io::tcp::Stream {{ conn_fd: self.fd, owns_fd: false }};
                return s.recv(64) or raise;
            }}
        }}
        fn main() {{
            let fd = std::io::tcp::__connect("127.0.0.1", {});
            let c = Conn {{ fd: fd }};
            c.ping("AAA");
            c.ping("BBB");
            println("ack=", c.ack());
            std::io::tcp::__close_fd(fd);
        }}
        "#,
        port
    );
    let bin = build_hale("borrowed_fd", &src);
    let out = Command::new(&bin).output().expect("run hale");
    let _ = std::fs::remove_file(&bin);
    let server_got = got_rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or_default();

    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    // The decisive assertion: the SECOND ping arrived, so the first
    // borrowed Stream's dissolve left the fd open.
    assert!(
        server_got.contains("AAA") && server_got.contains("BBB"),
        "expected both pings over one shared fd; server got: {:?}",
        server_got
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ack=BOTH"),
        "expected server ack over the still-open fd; got: {:?}",
        stdout
    );
}

#[test]
fn stream_locus_is_declared_and_compiles() {
    // The Stream locus itself parses + lowers cleanly. The
    // method-via-let limitation (custom dissolve fires
    // eagerly on ephemeral instantiations) means we can't
    // exercise its methods through a let-binding here, but
    // a program that *references* Stream should still build
    // and run. Lifecycle-via-instantiation lands in m82.
    let src = r#"
        fn main() {
            // Statement-position Stream literal: instantiates
            // (with conn_fd=-1, the default), runs default
            // birth (no-op), default run (no-op), drain, and
            // dissolve. dissolve calls __close_fd(-1), which
            // is safe (close on -1 is a no-op in our C
            // wrapper).
            std::io::tcp::Stream { conn_fd: -1 };
            println("stream literal compiled and ran");
        }
    "#;
    let bin = build_hale("decl", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("stream literal compiled and ran"),
        "got: {:?}",
        stdout
    );
}
