//! Lever 1 (2026-07-16) — `std::io::udp::Reader`, the ergonomic
//! event-driven datagram-ingest handle.
//!
//! `Reader` bundles a bound socket + a single reused receive buffer and
//! exposes `next() -> BytesView fallible(IoError)`: on a `where async_io`
//! pool `next()` parks on EPOLLIN (kernel-woken, no busy-poll) and
//! returns a ZERO-COPY view of each datagram aliasing the reused buffer.
//! It's the hand-rolled "bind + BytesBuilder + recv_into + view" fast
//! path baked into one handle so it's the path of least resistance.
//!
//! Two properties pinned here:
//!   1. correctness — datagrams are received intact via the view;
//!   2. bounded RSS — a long recv loop with the reused buffer does not
//!      grow resident memory (the reused-buffer guarantee).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!("lt-udp-reader-{}-{}-{}.bin", tag, std::process::id(), nanos));
    p
}

fn free_udp_port() -> u16 {
    let s = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind probe");
    s.local_addr().expect("local_addr").port()
}

#[test]
fn reader_receives_datagrams_via_zero_copy_view() {
    let port = free_udp_port();
    let src = format!(
        r#"
        locus Ingest {{
            params {{
                r: std::io::udp::Reader =
                    std::io::udp::Reader {{ addr: "127.0.0.1", port: {port}, cap: 2048 }};
                total: Int = 0;
            }}
            run() {{
                let mut n = 0;
                while n < 5 {{
                    let dg = self.r.next() or raise;
                    println("recv ", len(dg), " bytes");
                    self.total = self.total + len(dg);
                    n = n + 1;
                }}
                println("total=", self.total);
                std::process::exit(0);
            }}
        }}
        locus Sender {{
            run() {{
                std::time::sleep(300ms);
                let fd = std::io::udp::bind("127.0.0.1", 0) or raise;
                let mut i = 0;
                while i < 5 {{
                    std::io::udp::send(fd, "127.0.0.1", {port}, "hello-datagram") or discard;
                    std::time::sleep(20ms);
                    i = i + 1;
                }}
                std::io::udp::close(fd);
            }}
        }}
        main locus App {{
            params {{ ing: Ingest = Ingest {{ }}; snd: Sender = Sender {{ }}; }}
            placement {{
                ing: cooperative(pool = rx) where async_io;
                snd: pinned;
            }}
            run() {{ std::time::sleep(5s); std::process::exit(0); }}
        }}
        fn main() {{ App {{ }}; }}
        "#,
    );

    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = unique_path("recv");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Each "hello-datagram" is 14 bytes; 5 of them = 70. The view reads
    // the exact datagram bytes.
    assert!(
        stdout.contains("recv 14 bytes"),
        "Reader.next() must surface each datagram's exact length via the \
         zero-copy view; got:\n{}",
        stdout
    );
    assert!(
        stdout.contains("total=70"),
        "Reader must receive all 5 datagrams intact (5 * 14 = 70); got:\n{}",
        stdout
    );
}

#[test]
fn reader_recv_loop_is_rss_flat() {
    // The reused-buffer guarantee: draining thousands of datagrams
    // through one Reader must not grow resident memory. A per-datagram
    // allocation into a never-returning run() loop would show as RSS
    // growth; the reused buffer keeps it flat.
    let port = free_udp_port();
    let src = format!(
        r#"
        locus Ingest {{
            params {{
                r: std::io::udp::Reader =
                    std::io::udp::Reader {{ addr: "127.0.0.1", port: {port}, cap: 2048 }};
                acc: Int = 0;
            }}
            run() {{
                // Warm up (bind + any one-time growth) before sampling.
                let mut w = 0;
                while w < 500 {{ let dg = self.r.next() or raise; self.acc = self.acc + len(dg); w = w + 1; }}
                let rss0 = std::process::rss_bytes();
                let mut n = 0;
                while n < 40000 {{ let dg = self.r.next() or raise; self.acc = self.acc + len(dg); n = n + 1; }}
                let rss1 = std::process::rss_bytes();
                println("rss_growth_kb=", (rss1 - rss0) / 1024);
                std::process::exit(0);
            }}
        }}
        locus Sender {{
            run() {{
                std::time::sleep(400ms);
                let fd = std::io::udp::bind("127.0.0.1", 0) or raise;
                let mut i = 0;
                while i < 41000 {{
                    std::io::udp::send(fd, "127.0.0.1", {port}, "payload-bytes-abcdef0123") or discard;
                    // Pace so the kernel recv buffer doesn't overflow and
                    // drop everything before the reader drains it.
                    if i - (i / 20) * 20 == 0 {{ std::time::sleep(1ms); }}
                    i = i + 1;
                }}
                std::time::sleep(3s);
            }}
        }}
        main locus App {{
            params {{ ing: Ingest = Ingest {{ }}; snd: Sender = Sender {{ }}; }}
            placement {{
                ing: cooperative(pool = rx) where async_io;
                snd: pinned;
            }}
            run() {{ std::time::sleep(30s); std::process::exit(0); }}
        }}
        fn main() {{ App {{ }}; }}
        "#,
    );

    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = unique_path("rss");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);

    let growth: i64 = stdout
        .lines()
        .find_map(|l| l.strip_prefix("rss_growth_kb="))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no rss_growth_kb in stdout:\n{}", stdout));

    // Reused buffer → flat RSS. A generous 4 MiB ceiling absorbs
    // allocator slack while failing hard on any per-datagram
    // accumulation (40k * ~24 B would be ~1 MiB of unreclaimed growth,
    // far more with arena chunking).
    assert!(
        growth <= 4096,
        "a Reader recv loop must be RSS-flat (reused buffer); grew {growth} KiB \
         over 40k datagrams — the receive buffer is not being reused:\n{}",
        stdout
    );
}
