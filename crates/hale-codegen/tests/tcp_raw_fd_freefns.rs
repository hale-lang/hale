//! Gap E (2026-07-17) — raw-fd TCP free-fn coverage.
//!
//! pond's pq/keepalive work (FRICTION "Toolchain gotcha #2", early
//! July 2026) reported `std::io::tcp::__send_bytes` / `__recv_bytes`
//! SEGFAULTING natively when called from user free-fn wrappers on
//! their pinned binary, and steered pq to the `Stream` locus methods.
//! On current HEAD the free-fn surface round-trips cleanly across
//! every placement (pinned / classic cooperative / async_io), direct
//! and wrapper-fn shapes, plain and under ASan — but nothing pinned
//! that: the corpus oracle SKIPS server fixtures, so the raw-fd
//! family had zero native run coverage. This test is that coverage.
//!
//! The hale-side client deliberately routes through user free-fn
//! DISPATCH WRAPPERS (pond's exact shape — the segfault report said
//! "the moment I split send/recv into free-fn dispatch helpers"), on
//! a classic cooperative pool, doing repeated round-trips so a
//! stale-caller-arena class of bug (the #215 UAF family) has churn
//! to surface under.

use std::io::{Read, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "hale_tcp_raw_fd_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    bin
}

/// Hale client: __connect + wrapper-fn __send_bytes/__recv_bytes
/// round-trips against a Rust echo server, on a cooperative pool.
fn client_src(port: u16, rounds: u32) -> String {
    format!(
        r#"
        fn send_raw(handle: Int, b: Bytes) -> Int {{
            return std::io::tcp::__send_bytes(handle, b);
        }}
        fn recv_raw(handle: Int, max_bytes: Int) -> Bytes {{
            return std::io::tcp::__recv_bytes(handle, max_bytes);
        }}
        locus Client {{
            params {{ ok: Int = 0; }}
            run() {{
                let fd = std::io::tcp::__connect("127.0.0.1", {port});
                if fd < 0 {{
                    println("connect-failed");
                    std::process::exit(3);
                }}
                let mut i = 0;
                while i < {rounds} {{
                    let payload = std::bytes::from_string("rt-" + i + "-padpadpadpad");
                    let rc = send_raw(fd, payload);
                    if rc != 0 {{
                        println("send-failed at ", i);
                        std::process::exit(4);
                    }}
                    let back = recv_raw(fd, 4096);
                    if len(back) != len(payload) {{
                        println("mismatch at ", i, ": ", len(back), " vs ", len(payload));
                        std::process::exit(2);
                    }}
                    self.ok = self.ok + 1;
                    i = i + 1;
                }}
                std::io::tcp::__close_fd(fd);
                println("rounds-ok=", self.ok);
                std::process::exit(0);
            }}
        }}
        main locus App {{
            params {{ c: Client = Client {{ }}; }}
            placement {{ c: cooperative(pool = cli); }}
            run() {{ std::time::sleep(10s); println("TIMEOUT"); std::process::exit(1); }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    )
}

#[test]
fn raw_fd_freefns_roundtrip_via_wrapper_fns() {
    let port = pick_free_port();
    let rounds: u32 = 64;

    // Rust-side echo server (accept one conn, echo until EOF).
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        s.set_read_timeout(Some(Duration::from_secs(8))).ok();
        let mut buf = [0u8; 4096];
        loop {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if s.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let bin = build("wrapper_roundtrip", &client_src(port, rounds));
    let out = Command::new(&bin).output().expect("run client");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "client exited {:?} (a segfault here is the pond raw-fd \
         free-fn class)\nstdout: {}\nstderr: {}",
        out.status,
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains(&format!("rounds-ok={}", rounds)),
        "expected {} clean round-trips, got:\n{}",
        rounds,
        stdout
    );
    server.join().expect("server thread");
}
