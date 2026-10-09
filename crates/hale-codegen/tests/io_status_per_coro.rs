//! A read's I/O status belongs to the coroutine that made it.
//!
//! The runtime keeps the outcome of the last send/recv for
//! `Stream.recv`/`recv_bytes` to ask after the call. It used to be a
//! thread-local, and coroutines of one `async_io` pool share their
//! worker thread, interleaving at their parks: a connection parked in a
//! read, while another connection's read failed (a peer that reset), woke
//! to a good read and found the neighbour's errno. Here two connections
//! (two listeners' handlers) are in `recv_bytes` on one pool; one is reset, the other then sends,
//! and the one that sends is answered with what it sent.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/ports.rs"]
mod ports;

/// Close with an RST rather than a FIN: the peer's read fails ECONNRESET.
fn reset(s: TcpStream) {
    let l = libc::linger { l_onoff: 1, l_linger: 0 };
    let r = unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            &l as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        )
    };
    assert_eq!(r, 0, "SO_LINGER");
    drop(s);
}

fn connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return s,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Err(e) => panic!("connect: {e:?}"),
        }
    }
}

#[test]
fn a_good_read_does_not_see_a_neighbours_reset() {
    let (port_a, port_b) = (ports::free_port(), ports::free_port());
    let src = format!(
        r#"
        fn handle(s: std::io::tcp::Stream) {{
            // answers what it read, or ERR! when its own read failed
            let got = s.recv_bytes(64) or std::bytes::from_string("ERR!");
            s.send_bytes(got) or discard;
        }}

        main locus App {{
            params {{
                la: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host: "127.0.0.1",
                    port: {port_a},
                    max_accepts: 1,
                    on_connection: handle,
                }};
                lb: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host: "127.0.0.1",
                    port: {port_b},
                    max_accepts: 1,
                    on_connection: handle,
                }};
            }}
            placement {{
                la: cooperative(pool = io) where async_io;
                lb: cooperative(pool = io) where async_io;
            }}
            run() {{
                std::time::sleep(6s);
            }}
        }}

        fn main() {{
            App {{ }};
        }}
    "#
    );
    let bin = harness::unique_bin("hale_test_io_status_per_coro");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");

    // the good one connects first and sits in its read
    let mut good = connect(port_a);
    thread::sleep(Duration::from_millis(200));
    // the second connection is in its read too, and then resets
    let bad = connect(port_b);
    thread::sleep(Duration::from_millis(200));
    reset(bad);
    thread::sleep(Duration::from_millis(300));
    // the first read completes now, after the neighbour's failed
    good.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    good.write_all(b"hello").expect("write");
    let mut buf = [0u8; 16];
    let n = good.read(&mut buf).unwrap_or(0);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    assert_eq!(&buf[..n], b"hello", "the read that succeeded reports success");
}
