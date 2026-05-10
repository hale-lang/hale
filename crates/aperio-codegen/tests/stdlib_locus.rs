//! m73a: stdlib loci via bundled-source concatenation.
//! m73b: real lifecycle bodies wired through `std::io::tcp::__*`
//! path-call primitives.
//!
//! The bundled stdlib source (`runtime/stdlib.ap`) declares
//! `__StdIoTcpListener`; codegen prepends those decls to the
//! user program before lowering and rewrites the path-qualified
//! instantiation `std::io::tcp::Listener` to the mangled name.
//!
//! These tests cover both layers:
//!   - build-time: unknown stdlib paths error cleanly; programs
//!     that don't reference std::* compile unaffected.
//!   - runtime: a real Aperio program that instantiates the
//!     Listener actually binds + accepts + closes via the
//!     lotus_tcp_listen_socket / accept_one / close_fd
//!     primitives. The test process plays the role of the
//!     remote peer that connects to the bound port.

use std::io::Write;
use std::process::{Command, Stdio};

use aperio_codegen::build_executable;

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

fn build_aperio_binary(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_locus_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn listener_binds_accepts_a_connection_and_exits_cleanly() {
    // The whole m73b chain end-to-end: parse-merge-lower puts
    // __StdIoTcpListener in user_loci; the user's path-qualified
    // instantiation rewrites to that mangled name; birth() calls
    // lotus_tcp_listen_socket via the __listen_socket path-call;
    // run() blocks on lotus_tcp_accept_one; this test process
    // plays the connecting peer; run() prints the accepted fd
    // and returns; dissolve() closes the listen fd; main() exits.
    let port = pick_free_port();
    let source = format!(
        r#"
        fn main() {{
            std::io::tcp::Listener {{ host: "127.0.0.1", port: {} }};
        }}
        "#,
        port
    );
    let bin = build_aperio_binary("listener_accepts", &source);

    let listener_proc = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener binary");

    // Connect, send a noop byte so the kernel completes the
    // handshake, then drop. The listener prints + closes the
    // accepted fd regardless of whether bytes flow on it; our
    // job here is just to satisfy the accept() blocking on it.
    // Retry the connect briefly because there's a race between
    // process spawn and the listener's bind+listen.
    let mut connected = None;
    for _ in 0..50 {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut s) => {
                let _ = s.write_all(b"x");
                drop(s);
                connected = Some(());
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    assert!(connected.is_some(), "could not connect to listener on port {}", port);

    let out = listener_proc.wait_with_output().expect("listener wait");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "listener exited non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("__StdIoTcpListener.birth host=127.0.0.1 port={}", port)),
        "birth diagnostic missing; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("__StdIoTcpListener.run accepted conn="),
        "accepted-conn diagnostic missing; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains(&format!("__StdIoTcpListener.dissolve host=127.0.0.1 port={}", port)),
        "dissolve diagnostic missing; got: {:?}",
        stdout
    );
}

#[test]
fn listener_uses_default_host_when_omitted() {
    // Standard locus default-params mechanism applies to stdlib
    // loci unchanged: omitting `host` falls back to the
    // "127.0.0.1" default declared in the bundled source.
    let port = pick_free_port();
    let source = format!(
        r#"
        fn main() {{
            std::io::tcp::Listener {{ port: {} }};
        }}
        "#,
        port
    );
    let bin = build_aperio_binary("default_host", &source);

    let listener_proc = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let mut connected = false;
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            connected = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(connected, "could not connect on default-host port {}", port);

    let out = listener_proc.wait_with_output().expect("listener wait");
    let _ = std::fs::remove_file(&bin);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("host=127.0.0.1"),
        "expected default host 127.0.0.1; got: {:?}",
        stdout
    );
}

#[test]
fn unknown_stdlib_path_struct_literal_errors_clearly() {
    let src = r#"
        fn main() {
            std::io::tcp::Nonexistent { port: 1 };
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("aperio_test_stdlib_locus_unknown");
    let result = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    assert!(result.is_err(), "expected build error for unknown stdlib path");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("std::io::tcp::Nonexistent"),
        "diagnostic should name the unresolved path; got: {}",
        msg
    );
}

#[test]
fn user_program_with_no_stdlib_use_still_compiles() {
    // Concatenating stdlib decls onto every user program must
    // not break programs that don't reference std::*. Locks in
    // that the bundled `__StdIoTcpListener` doesn't pollute the
    // user namespace or interfere with the existing main()
    // discovery, even now that its lifecycle bodies do real I/O.
    let src = r#"
        fn main() {
            println("hello, world");
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("aperio_test_stdlib_locus_no_use");
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello, world"));
    assert!(
        !stdout.contains("__StdIoTcpListener"),
        "stdlib output leaked into program that didn't use stdlib; got: {:?}",
        stdout
    );
}
