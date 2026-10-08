//! The runtime's HTTP/2 session (`runtime/lotus_h2.c`, nghttp2 inside),
//! driven through the C surface `std::io::h2::__*` lowers to, with no
//! socket anywhere: bytes in with `__feed`, bytes out with `__drain`, events
//! with `__poll`. The program is the whole proof that the library is built
//! into the runtime and is sans-I/O (it has no descriptor to read).
//!
//! A client's preface and an empty SETTINGS are fed; the session answers
//! with its own SETTINGS (type 4), a window update (8) and the
//! acknowledgement of the client's (4, flag 1). A request (HEADERS, 1) fed
//! after is one `OPEN` event with its header lines and one `END`; the answer
//! submitted for the stream drains as HEADERS, DATA and the trailers'
//! HEADERS (1, 0, 1).

use std::process::Command;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

const PROGRAM: &str = r#"
    fn zeros(n: Int) -> Bytes {
        let mut b = std::bytes::from_string("");
        let mut i = 0;
        while i < n {
            b = std::bytes::concat(b, std::bytes::from_int(0));
            i = i + 1;
        }
        return b;
    }

    fn byte(v: Int) -> Bytes {
        return std::bytes::from_int(v);
    }

    // the frame types of a stream of frames, each as type/flags
    fn frames(b: Bytes) -> String {
        let mut off = 0;
        let mut s = "";
        while off + 9 <= len(b) {
            let n = (std::bytes::read_u8(b, off) or 0) * 65536 + (std::bytes::read_u8(b, off + 1) or 0) * 256 + (std::bytes::read_u8(b, off + 2) or 0);
            let t = std::bytes::read_u8(b, off + 3) or 0;
            let f = std::bytes::read_u8(b, off + 4) or 0;
            s = s + to_string(t) + "/" + to_string(f) + " ";
            off = off + 9 + n;
        }
        return s;
    }

    fn main() {
        let h = std::io::h2::__open();
        println("open=", h != 0);

        // the client's preface, then an empty SETTINGS
        let preface = std::bytes::from_string("PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        let settings = std::bytes::concat(std::bytes::concat(zeros(3), byte(4)), zeros(5));
        let fed = std::io::h2::__feed(h, std::bytes::concat(preface, settings));
        println("fed=", fed);
        println("out=", frames(std::io::h2::__drain(h)));
        println("quiet=", len(std::io::h2::__drain(h)));

        // GET / with :authority a, stream 1: END_HEADERS | END_STREAM
        let block = std::bytes::concat(std::bytes::concat(byte(130), byte(134)), std::bytes::concat(byte(132), std::bytes::concat(byte(65), std::bytes::concat(byte(1), byte(97)))));
        let head = std::bytes::concat(std::bytes::concat(zeros(2), byte(6)), std::bytes::concat(byte(1), std::bytes::concat(byte(5), std::bytes::concat(zeros(3), byte(1)))));
        let fed2 = std::io::h2::__feed(h, std::bytes::concat(head, block));
        println("fed2=", fed2);
        let mut k = std::io::h2::__poll(h);
        while k != 0 {
            let text = std::str::replace(std::str::from_bytes(std::io::h2::__ev_bytes(h)), "\n", "|");
            println("event=", k, " stream=", std::io::h2::__ev_stream(h), " code=", std::io::h2::__ev_code(h), " bytes=", text);
            k = std::io::h2::__poll(h);
        }

        // the answer: headers, a body, trailers
        let rc = std::io::h2::__respond(h, 1, std::bytes::from_string(":status: 200\ncontent-type: application/grpc\n"), std::bytes::from_string("hello"), std::bytes::from_string("grpc-status: 0\n"));
        println("respond=", rc);
        println("answer=", frames(std::io::h2::__drain(h)));
        // a second stream, opened (END_HEADERS only) and reset: CANCEL
        let head3 = std::bytes::concat(std::bytes::concat(zeros(2), byte(6)), std::bytes::concat(byte(1), std::bytes::concat(byte(4), std::bytes::concat(zeros(3), byte(3)))));
        let fed3 = std::io::h2::__feed(h, std::bytes::concat(head3, block));
        println("fed3=", fed3);
        println("reset=", std::io::h2::__reset(h, 3, 8));
        println("rst=", frames(std::io::h2::__drain(h)));
        println("alive=", std::io::h2::__alive(h));
        println("goaway=", std::io::h2::__goaway(h, 0));
        println("bye=", frames(std::io::h2::__drain(h)));
        println("close=", std::io::h2::__close(h));
    }
"#;

#[test]
fn the_handshake_and_a_unary_exchange_round_trip_with_no_socket() {
    let bin = harness::unique_bin("io_h2_session");
    build_opts::build_source(PROGRAM, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let line = |key: &str| -> String {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or_else(|| panic!("no `{key}` in:\n{stdout}"))
            .to_string()
    };
    assert_eq!(line("open="), "true");
    assert_eq!(line("fed="), "33");
    // the server's SETTINGS first; the client's acknowledged
    let out_frames = line("out=");
    assert!(out_frames.starts_with("4/0 "), "{out_frames}");
    assert!(out_frames.contains("4/1 "), "{out_frames}");
    assert_eq!(line("quiet="), "0");
    assert_eq!(line("fed2="), "15");
    assert!(
        stdout.contains("event=1 stream=1 code=0 bytes=:method: GET|:scheme: http|:path: /|:authority: a|"),
        "{stdout}"
    );
    assert!(stdout.contains("event=3 stream=1 code=0"), "{stdout}");
    assert_eq!(line("respond="), "0");
    assert_eq!(line("answer="), "1/4 0/0 1/5 ");
    assert_eq!(line("fed3="), "15");
    assert_eq!(line("reset="), "0");
    assert_eq!(line("rst="), "3/0 ");
    assert_eq!(line("alive="), "1");
    assert_eq!(line("goaway="), "0");
    assert_eq!(line("bye="), "7/0 ");
    assert_eq!(line("close="), "1");
}
