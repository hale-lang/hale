//! A loop body in a frame with a per-call scratch frees what an
//! iteration makes when the iteration ends (`hale_codegen`'s
//! `iteration_arena`, `hale_types::iteration_reclaim`).
//!
//! Before, a fresh value made directly in a long-lived `run()` loop body
//! was kept until the locus dissolved: `buf.snapshot()` per iteration
//! over a 1 GiB file held about 1 GB, and a `std::http::BodyReader`
//! drained by `chunk()` in such a loop peaked at the whole body's size,
//! while the same code moved into a method call stayed flat. A
//! model-weights downloader streams files of tens of GB, so the natural
//! loop (`let c = body.chunk() or { break; }; h.update(c);`) has to be
//! flat as written.
//!
//! The oracle is `LOTUS_ARENA_RESIDENCY=1`'s high-water mark of the
//! chunk bytes every arena holds at once, method scratch and iteration
//! arenas included (`[arena_residency peak] chunk_bytes_peak=N`), not
//! RSS: it counts exactly what the arenas keep, so a loop that keeps its
//! iterations shows a peak that grows with the trip count.
//!
//! What an iteration hands on keeps the ownership it had: a binding
//! declared before the loop that is assigned a value keeps the loop on
//! the frame's scratch, a value stored into `self` is copied into the
//! locus, and a value returned from inside the loop is copied to the
//! caller. Those programs are built under AddressSanitizer too, with the
//! chunk pool off, so a value freed with its iteration and read after
//! is a reported use-after-free, not a silent read.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The peak chunk bytes of a run, and its stdout.
fn peak_of(name: &str, src: &str, args: &[String]) -> (u64, String) {
    let bin = harness::unique_bin(&format!("loop_iteration_reclaim_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).args(args).env("LOTUS_ARENA_RESIDENCY", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{name}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    let peak = stderr
        .lines()
        .find_map(|l| l.strip_prefix("[arena_residency peak] chunk_bytes_peak="))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{name}: no residency peak line\nstderr:\n{stderr}"));
    eprintln!("{name}: chunk_bytes_peak={peak}");
    (peak, stdout)
}

const MIB: u64 = 1 << 20;

/// 2000 snapshots of a 64 KiB builder: 125 MiB if each is kept.
const SNAPSHOT_LOCUS: &str = r#"
locus Grow {
    params {
        n: Int = 2000;
    }
    fn fill(buf: std::bytes::BytesBuilder) {
        let mut i = 0;
        while i < 8192 {
            buf.append_u64_le(i);
            i = i + 1;
        }
    }
    fn piece(buf: std::bytes::BytesBuilder) -> Int {
        let c = buf.snapshot() or b"";
        return len(c);
    }
    fn churn(buf: std::bytes::BytesBuilder) -> Int {
        let mut k = 0;
        let mut total = 0;
        while k < self.n {
            let c = buf.snapshot() or b"";
            total = total + len(c);
            k = k + 1;
        }
        return total;
    }
    run() {
        let buf = std::bytes::BytesBuilder { initial_cap: 65536 };
        self.fill(buf);
        let mut k = 0;
        let mut total = 0;
        BODY
        println(total);
    }
}

fn main() {
    let g = Grow { };
}
"#;

fn snapshot_program(body: &str) -> String {
    SNAPSHOT_LOCUS.replace("BODY", body)
}

const SNAPSHOTS_BYTES: &str = "131072000";

/// The reported shape: a snapshot per iteration of `run()`'s own loop,
/// leaving by `break` from the `or` handler.
#[test]
fn a_snapshot_per_iteration_of_run_is_reclaimed_per_iteration() {
    let (peak, out) = peak_of(
        "run_break",
        &snapshot_program(
            "while true {
                if k == self.n {
                    break;
                }
                let c = buf.snapshot() or { break; };
                total = total + len(c);
                k = k + 1;
            }",
        ),
        &[],
    );
    assert!(out.contains(SNAPSHOTS_BYTES), "got:\n{out}");
    assert!(peak < 4 * MIB, "run() loop kept its iterations: peak {peak} bytes");
}

/// The `or b""` fallback spelling, and the loop as a `while` condition.
#[test]
fn a_snapshot_with_a_fallback_literal_is_reclaimed_per_iteration() {
    let (peak, out) = peak_of(
        "run_fallback",
        &snapshot_program(
            "while k < self.n {
                let c = buf.snapshot() or b\"\";
                total = total + len(c);
                k = k + 1;
            }",
        ),
        &[],
    );
    assert!(out.contains(SNAPSHOTS_BYTES), "got:\n{out}");
    assert!(peak < 4 * MIB, "run() loop kept its iterations: peak {peak} bytes");
}

/// A plain `while` in a long-lived method (one call, the whole loop),
/// and the method-per-iteration shape that was always flat.
#[test]
fn a_while_loop_in_a_long_lived_method_is_reclaimed_per_iteration() {
    let (in_method, out) = peak_of("method_loop", &snapshot_program("total = self.churn(buf);"), &[]);
    assert!(out.contains(SNAPSHOTS_BYTES), "got:\n{out}");
    assert!(in_method < 4 * MIB, "a method's loop kept its iterations: peak {in_method} bytes");
    let (per_call, out) = peak_of(
        "method_per_iteration",
        &snapshot_program(
            "while k < self.n {
                total = total + self.piece(buf);
                k = k + 1;
            }",
        ),
        &[],
    );
    assert!(out.contains(SNAPSHOTS_BYTES), "got:\n{out}");
    assert!(per_call < 4 * MIB, "a method per iteration kept its calls: peak {per_call} bytes");
}

/// Serve one HTTP/1.1 response of `len` bytes, framed by Content-Length,
/// written 64 KiB at a time, to the one connection the program makes.
fn serve_body(len: usize) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().expect("accept");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if conn.read(&mut byte).expect("read request") == 0 {
                return;
            }
            head.push(byte[0]);
        }
        let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
        conn.write_all(reply.as_bytes()).expect("write head");
        let piece = vec![7u8; 65536];
        let mut left = len;
        while left > 0 {
            let n = left.min(piece.len());
            if conn.write_all(&piece[..n]).is_err() {
                return;
            }
            left -= n;
        }
    });
    (addr.port(), handle)
}

/// The downloader's loop: `chunk()` in `run()`'s own loop, each piece
/// hashed and counted. 64 MiB of body, which the loop used to keep whole.
#[test]
fn a_body_reader_drained_in_run_is_reclaimed_per_iteration() {
    const BODY: usize = 64 << 20;
    let src = r#"
locus Download {
    params {
        url: String = "";
    }
    run() {
        let u = std::http::parse_url(self.url) or std::http::Url { };
        let body = std::http::BodyReader { req: std::http::ClientRequest { url: u, body: b"" } };
        let h = std::crypto::Sha256 { };
        let mut total = 0;
        while true {
            let c = body.chunk() or { break; };
            h.update(c);
            total = total + len(c);
        }
        println("bytes=", total);
        println("digest=", len(h.finish()));
    }
}

fn main() {
    let d = Download { url: std::env::arg(1) };
}
"#;
    let (port, server) = serve_body(BODY);
    let (peak, out) = peak_of("body_reader", src, &[format!("http://127.0.0.1:{port}/weights")]);
    server.join().expect("server");
    assert!(out.contains(&format!("bytes={BODY}")), "got:\n{out}");
    assert!(out.contains("digest=32"), "got:\n{out}");
    assert!(peak < 8 * MIB, "the body reader's loop kept its chunks: peak {peak} bytes");
}

/// What an iteration hands on survives it, read after the loop: a
/// binding declared before the loop (the loop keeps the frame's scratch),
/// a `self` field (copied into the locus), a value returned from inside
/// the loop (copied to the caller), a nested loop's value read by the
/// outer iteration, a `continue`d iteration. Built plainly and under
/// AddressSanitizer with the chunk pool off.
const ESCAPES: &str = r#"
locus Keep {
    params {
        kept: Bytes = b"";
        label: String = "";
    }
    fn first_long(buf: std::bytes::BytesBuilder, at_least: Int) -> Bytes {
        let mut i = 0;
        while i < 10 {
            buf.append_u8(i);
            let c = buf.snapshot() or b"";
            if len(c) >= at_least {
                return c;
            }
            i = i + 1;
        }
        return b"";
    }
    fn label_of(n: Int) -> String {
        for i in 0..n {
            let s = "row-" + to_string(i);
            if i == n - 1 {
                return s;
            }
        }
        return "";
    }
    run() {
        let buf = std::bytes::BytesBuilder { initial_cap: 64 };
        let mut last = b"";
        let mut k = 0;
        while k < 5 {
            buf.append_u8(k);
            let c = buf.snapshot() or b"";
            last = c;
            k = k + 1;
        }
        println("last=", len(last), " byte=", std::bytes::at(last, 4) or -1);
        k = 0;
        while k < 5 {
            buf.append_u8(9);
            let c = buf.snapshot() or b"";
            self.kept = c;
            self.label = "kept-" + to_string(len(c));
            k = k + 1;
        }
        println("kept=", len(self.kept), " label=", self.label);
        let r = self.first_long(buf, 13);
        println("returned=", len(r));
        println("label_of=", self.label_of(4));
        let mut outer_total = 0;
        let mut j = 0;
        while j < 3 {
            let row = "r" + to_string(j);
            let mut inner = 0;
            while inner < 3 {
                let cell = row + "c" + to_string(inner);
                outer_total = outer_total + len(cell);
                inner = inner + 1;
            }
            outer_total = outer_total + len(row);
            j = j + 1;
        }
        println("nested=", outer_total);
        let mut odd = 0;
        for i in 0..6 {
            let s = "n" + to_string(i);
            if i % 2 == 0 {
                continue;
            }
            odd = odd + len(s);
        }
        println("continued=", odd);
    }
}

fn main() {
    let k = Keep { };
}
"#;

const ESCAPES_OUT: &[&str] = &[
    "last=5 byte=4",
    "kept=10 label=kept-10",
    "returned=13",
    "label_of=row-3",
    "nested=42",
    "continued=6",
];

#[test]
fn what_an_iteration_hands_on_survives_it() {
    let (_, out) = peak_of("escapes", ESCAPES, &[]);
    for want in ESCAPES_OUT {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
}

#[test]
fn what_an_iteration_hands_on_survives_it_under_asan() {
    let bin = harness::unique_bin("loop_iteration_reclaim_escapes_asan");
    let opts = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(ESCAPES, &bin, &opts).expect("build");
    let out = Command::new(&bin).env("ASAN_OPTIONS", "detect_leaks=0").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "ASan report:\n{stderr}");
    for want in ESCAPES_OUT {
        assert!(stdout.contains(want), "missing {want:?} in:\n{stdout}");
    }
}

/// A free fn handed a record declared before the loop writes into it
/// (`push(bag.items, piece)`), and what it writes is made in its caller
/// arena. With that loop reclaimed per iteration the record kept pointers
/// into a freed iteration arena, read after the loop: a call that may
/// write into an argument is handed none of the storage held outside the
/// body, or the body is not reclaimed (`hale_types::iteration_reclaim`).
/// The same through a nested loop, and through an alias of the record
/// declared in the body. Under AddressSanitizer with the chunk pool off,
/// a read of a freed iteration is a report.
const HANDED_OUT: &str = r#"
type Bag { items: bounded[Bytes; 8]; }
fn keep(bag: Bag, piece: Bytes) { push(bag.items, piece) or discard; }
locus Hand {
    run() {
        let buf = std::bytes::BytesBuilder { initial_cap: 64 };
        buf.append_str("abcdefgh");
        let outer = Bag { };
        let mut i = 0;
        while i < 3 {
            let piece = buf.snapshot() or b"";
            keep(outer, piece);
            i = i + 1;
        }
        println("flat=", len(at(outer.items, 0) or b""), " ", std::str::from_bytes(at(outer.items, 2) or b""));
        let nested = Bag { };
        let mut j = 0;
        while j < 2 {
            let mut k = 0;
            while k < 2 {
                buf.append_u8(105);
                let piece = buf.snapshot() or b"";
                keep(nested, piece);
                k = k + 1;
            }
            j = j + 1;
        }
        println("nested=", len(at(nested.items, 0) or b""), " ", len(at(nested.items, 3) or b""));
        let aliased = Bag { };
        let mut n = 0;
        while n < 2 {
            let o = aliased;
            let piece = buf.snapshot() or b"";
            keep(o, piece);
            n = n + 1;
        }
        println("aliased=", len(at(aliased.items, 1) or b""));
    }
}

fn main() {
    let h = Hand { };
}
"#;

// `let o = aliased` is a copy of the record (a local bound from a place is
// yours, copied at its first write), so the writes land in the copy and
// `aliased` stays empty: the alias arm is there for what ASan sees.
const HANDED_OUT_OUT: &[&str] = &["flat=8 abcdefgh", "nested=9 12", "aliased=0"];

/// The same write through a free fn held in a fn-typed locus field:
/// `self.callback(outer, piece)` is a call of `keep`, not a method of the
/// locus, so it is judged as the free call it is; and through a field of
/// `self` handed to the free fn (`keep(self.bag, piece)`), which before
/// per-iteration reclaim lived as long as `run()` did. Flat and nested.
const CALLED_THROUGH: &str = r#"
type Bag { items: bounded[Bytes; 8]; }
fn keep(bag: Bag, piece: Bytes) { push(bag.items, piece) or discard; }
locus Probe {
    params {
        callback: fn(Bag, Bytes) = keep;
        bag: Bag = Bag { };
    }
    run() {
        let buf = std::bytes::BytesBuilder { initial_cap: 64 };
        buf.append_str("abcdefgh");
        let outer = Bag { };
        let mut i = 0;
        while i < 3 {
            let piece = buf.snapshot() or b"";
            self.callback(outer, piece);
            i = i + 1;
        }
        println("callback=", std::str::from_bytes(at(outer.items, 2) or b""));
        let nested = Bag { };
        let mut j = 0;
        while j < 2 {
            let mut k = 0;
            while k < 2 {
                buf.append_u8(105);
                let piece = buf.snapshot() or b"";
                self.callback(nested, piece);
                k = k + 1;
            }
            j = j + 1;
        }
        println("nested=", len(at(nested.items, 3) or b""));
        let mut n = 0;
        while n < 2 {
            buf.append_u8(106);
            let piece = buf.snapshot() or b"";
            keep(self.bag, piece);
            n = n + 1;
        }
        println("self_field=", std::str::from_bytes(at(self.bag.items, 1) or b""));
    }
}

fn main() {
    let p = Probe { };
}
"#;

const CALLED_THROUGH_OUT: &[&str] = &["callback=abcdefgh", "nested=12", "self_field=abcdefghiiiijj"];

/// The review's last two routes: a user fn that happens to be called
/// `push`, handed a field of `self` (no call is exempt by its name), and a
/// free fn handed a record field of another locus (`holder.bag`: a path
/// through a locus is that locus's storage, not the locus reference).
const NAMED_AND_HELD: &str = r#"
type Box { b: Bytes; }
fn push(bag: Box, piece: Bytes) { bag.b = piece; }
fn keep(bag: Box, piece: Bytes) { bag.b = piece; }
locus Holder {
    params { bag: Box = Box { b: b"" }; }
}
locus Probe {
    params { bag: Box = Box { b: b"" }; }
    run() {
        let buf = std::bytes::BytesBuilder { initial_cap: 64 };
        buf.append_str("abcdefgh");
        let mut i = 0;
        while i < 3 {
            buf.append_u8(105);
            let piece = buf.snapshot() or b"";
            push(self.bag, piece);
            i = i + 1;
        }
        println("named=", std::str::from_bytes(self.bag.b));
        let holder = Holder { };
        let mut j = 0;
        while j < 2 {
            buf.append_u8(106);
            let piece = buf.snapshot() or b"";
            keep(holder.bag, piece);
            j = j + 1;
        }
        println("held=", std::str::from_bytes(holder.bag.b));
    }
}

fn main() {
    let p = Probe { };
}
"#;

const NAMED_AND_HELD_OUT: &[&str] = &["named=abcdefghiii", "held=abcdefghiiijj"];

#[test]
fn what_a_push_named_fn_or_a_held_locus_field_takes_survives_under_asan() {
    plain_and_asan("named_and_held", NAMED_AND_HELD, NAMED_AND_HELD_OUT);
}

/// Build `src` plainly and under AddressSanitizer (chunk pool off) and
/// hold each run to `want`, with no sanitizer report.
fn plain_and_asan(name: &str, src: &str, want: &[&str]) {
    for asan in [false, true] {
        let bin = harness::unique_bin(&format!("loop_iteration_reclaim_{name}_{asan}"));
        let opts = hale_codegen::BuildOptions { asan, ..build_opts::options() };
        build_opts::build_source(src, &bin, &opts).expect("build");
        let out = Command::new(&bin).env("ASAN_OPTIONS", "detect_leaks=0").output().expect("run");
        let _ = std::fs::remove_file(&bin);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(out.status.success(), "{name} asan={asan}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
        assert!(!stderr.contains("AddressSanitizer"), "{name}: ASan report:\n{stderr}");
        for w in want {
            assert!(stdout.contains(w), "{name} asan={asan}: missing {w:?} in:\n{stdout}");
        }
    }
}

#[test]
fn what_a_callback_or_a_free_fn_writes_into_outside_storage_survives_under_asan() {
    plain_and_asan("called_through", CALLED_THROUGH, CALLED_THROUGH_OUT);
}

#[test]
fn what_a_free_fn_writes_into_an_outside_record_survives_under_asan() {
    for asan in [false, true] {
        let bin = harness::unique_bin(&format!("loop_iteration_reclaim_handed_out_{asan}"));
        let opts = hale_codegen::BuildOptions { asan, ..build_opts::options() };
        build_opts::build_source(HANDED_OUT, &bin, &opts).expect("build");
        let out = Command::new(&bin).env("ASAN_OPTIONS", "detect_leaks=0").output().expect("run");
        let _ = std::fs::remove_file(&bin);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(out.status.success(), "asan={asan}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
        assert!(!stderr.contains("AddressSanitizer"), "ASan report:\n{stderr}");
        for want in HANDED_OUT_OUT {
            assert!(stdout.contains(want), "asan={asan}: missing {want:?} in:\n{stdout}");
        }
    }
}
