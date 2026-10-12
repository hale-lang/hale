//! Item 3 (downstream handoff 2026-07-15) — N reader loci sharing one
//! `where async_io` pool must all make progress: a reader whose
//! `udp::recv` is waiting for a datagram must PARK (yield the worker) so
//! a sibling reader queued behind it on the same pool can start and
//! receive its own traffic.
//!
//! The bug: `std::io::udp::recv` did a blocking `recvfrom`, pinning the
//! single pool worker inside the syscall. A second reader's `run()` cell
//! then sat undrained until the first recv returned — with no recv
//! timeout the first reader blocked forever on a quiet socket and the
//! second reader NEVER started. (TCP recv already parked, which is why
//! the same shape worked over WebSockets.)
//!
//! Distinguishing test: reader A parks on a socket that never receives a
//! datagram, WITH NO recv timeout — under the old blocking recv it pins
//! the worker indefinitely. Reader B, on the same pool, must still
//! receive the datagram the sender aims at it. Under the fix A parks and
//! B runs concurrently; under the old behavior B never prints its
//! marker. The main locus exits on a bounded timer so a regression fails
//! (marker absent) instead of hanging.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!("lt-async-udp-multi-{}-{}-{}.bin", tag, std::process::id(), nanos));
    p
}

#[test]
fn two_udp_readers_share_one_async_pool() {
    // A's socket is bound but never sent to (A parks forever), so it
    // binds port 0; B's receives the sole datagram, from a sender told
    // B's address before either is up, so that address is reserved for
    // the run (`child::Reserved`): a port drawn here and handed over is
    // free, not reserved, and another test of a parallel run can bind it
    // first.
    let reserved = child::Reserved::new();
    let (host_b, port_b) = (reserved.host(), reserved.port());

    let src = format!(
        r#"
        fn __empty(e: IoError) -> Bytes {{ return b""; }}
        locus Reader {{
            params {{ tag: String = ""; host: String = "127.0.0.1"; port: Int = 0; }}
            run() {{
                let fd = std::io::udp::bind(self.host, self.port) or raise;
                // NO recv timeout: block/park indefinitely. Under the old
                // blocking recv, reader A here pins the worker forever.
                let msg = std::io::udp::recv(fd, 2048) or __empty(err);
                println("[", self.tag, "] got ", len(msg), " bytes");
                std::io::udp::close(fd);
            }}
        }}
        locus Sender {{
            params {{ host: String = ""; port: Int = 0; }}
            run() {{
                std::time::sleep(300ms);
                let fd = std::io::udp::bind("", 0) or raise;
                std::io::udp::send(fd, self.host, self.port, "hello-B") or discard;
                std::io::udp::close(fd);
            }}
        }}
        main locus App {{
            params {{
                a: Reader = Reader {{ tag: "A", port: 0 }};
                b: Reader = Reader {{ tag: "B", host: "{host_b}", port: {port_b} }};
                tx: Sender = Sender {{ host: "{host_b}", port: {port_b} }};
            }}
            placement {{
                a: cooperative(pool = io) where async_io;
                b: cooperative(pool = io) where async_io;
                tx: pinned;
            }}
            run() {{ std::time::sleep(1500ms); std::process::exit(0); }}
        }}
        fn main() {{ App {{ }}; }}
        "#,
    );

    let bin = unique_path("two_readers");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    drop(reserved);
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // B received its datagram even though A is parked forever on a silent
    // socket — proof the two readers share the pool concurrently. Under a
    // blocking recv, A would pin the worker and this marker never prints.
    assert!(
        stdout.contains("[B] got 7 bytes"),
        "reader B must receive its datagram while reader A is parked on a \
         silent socket (async_io pool must multiplex parked readers); got:\n{}",
        stdout
    );
}
