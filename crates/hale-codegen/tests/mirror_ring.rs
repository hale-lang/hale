//! `std::io::MirrorRing` (#3 of the fast-protocol-I/O substrate plan): a
//! double-mmap wrap-free buffer. The decisive property: a record that
//! straddles the physical seam is written and read back as one contiguous
//! slice — `std::bytes::write_*` / `read_*` / `find_byte` operate across the
//! wrap with zero copies, on the raw `{ptr,len}` BytesMut window that
//! `writable()` / `readable()` hand out.

use hale_codegen::build_executable;
use std::process::Command;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    build_and_run_argv(name, src, &[])
}

fn build_and_run_argv(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_mirror_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

#[test]
fn recv_from_fills_the_ring_and_parses() {
    let port = pick_free_port();
    // Loopback: send a line-delimited message, recv it straight into the
    // mirror ring's free window (recv_from auto-commits), then scan it
    // zero-copy with find_byte over the readable window.
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::__send(cfd, "hello\nworld\n");
            std::io::tcp::set_recv_timeout(afd, 300ms) or raise;
            let ring = std::io::MirrorRing { cap: 4096 };
            let n = ring.recv_from(afd, 4096);
            let r = ring.readable();
            let nl = std::bytes::find_byte(r, 0, 10);
            let c0 = std::bytes::at(r, 0) or -1;
            println("n=", n, " len=", ring.len(), " nl=", nl, " c0=", c0);
        }
    "#;
    let (out, status) = build_and_run_argv("recv", src, &[&port.to_string()]);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    // "hello\nworld\n" = 12 bytes; first '\n' (10) at index 5; 'h' = 104.
    assert!(out.contains("n=12 len=12 nl=5 c0=104"),
        "recv_from must fill the ring and the window must parse; got: {:?}", out);
}

#[test]
fn read_write_across_the_seam_is_contiguous() {
    // cap = 4096 (page, power-of-two). Advance both cursors to offset 4090
    // (commit then skip 4090), so the next window begins 6 bytes before the
    // physical seam at 4096. A 12-byte record written there spans the seam;
    // via the double mapping it is one contiguous slice, so write_*/read_*
    // round-trip the values and find_byte locates a delimiter that sits
    // astride the wrap.
    let src = r#"
        fn main() {
            let ring = std::io::MirrorRing { cap: 4096 };
            ring.commit(4090);
            ring.skip(4090);

            let w = ring.writable();
            // u64 at [4090, 4098): 6 bytes before the seam, 2 after.
            std::bytes::write_u64_le(w, 0, 1234567890123) or raise;
            // a sentinel byte (10) at offset 8 (4098, just past the seam),
            // and a u16 at [10, 12).
            std::bytes::write_u8(w, 8, 10) or raise;
            std::bytes::write_u16_le(w, 9, 4242) or raise;
            ring.commit(11);

            let r = ring.readable();
            let a = std::bytes::read_u64_le(r, 0) or raise;     // spans the seam
            let nl = std::bytes::find_byte(r, 0, 10);            // delimiter past the seam
            let b = std::bytes::read_u16_le(r, 9) or raise;
            println("a=", a, " nl=", nl, " b=", b, " len=", ring.len());

            ring.skip(11);
            println("drained len=", ring.len());
        }
    "#;
    let (out, status) = build_and_run("seam", src);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    // a round-trips across the seam; the newline sentinel is at index 8;
    // the u16 reads back; the ring reports 11 buffered then 0 after skip.
    assert!(out.contains("a=1234567890123 nl=8 b=4242 len=11"),
        "seam-crossing read/write/find must be contiguous; got: {:?}", out);
    assert!(out.contains("drained len=0"), "got: {:?}", out);
}
