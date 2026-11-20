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

/// The review's last routes: a user fn that happens to be called `push`,
/// handed a field of `self` (no call is exempt by its name); a free fn
/// handed a record field of another locus (`holder.bag`: a path through a
/// locus is that locus's storage, not the locus reference); and the same
/// through a body binding of the locus (`let alias = self;`, `let alias =
/// holder;`), which is passed whole but owns no path through it.
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
        let mut k = 0;
        while k < 2 {
            buf.append_u8(107);
            let alias = self;
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            k = k + 1;
        }
        println("self_alias=", std::str::from_bytes(self.bag.b));
        let mut n = 0;
        while n < 2 {
            buf.append_u8(108);
            let alias = holder;
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            n = n + 1;
        }
        println("holder_alias=", std::str::from_bytes(holder.bag.b));
    }
}

fn main() {
    let p = Probe { };
}
"#;

const NAMED_AND_HELD_OUT: &[&str] = &[
    "named=abcdefghiii",
    "held=abcdefghiiijj",
    "self_alias=abcdefghiiijjkk",
    "holder_alias=abcdefghiiijjkkll",
];

#[test]
fn what_a_push_named_fn_or_a_held_locus_field_takes_survives_under_asan() {
    plain_and_asan("named_and_held", NAMED_AND_HELD, NAMED_AND_HELD_OUT);
}

/// A sample of `iteration_reclaim`'s binding forms over a locus source
/// (its unit test `every_binding_form_gives_its_source_kind` covers every
/// form against every source): each binds `alias` to `self` or another
/// locus (`spare` is a second one, for the arms that do not run), the
/// last two through a match-arm binder `n` over `self` and over `holder`, codex's two through a block tail that shadows an outside
/// scalar `n`, and hands a path through it to a free fn that writes
/// there. Each loop stays on the scratch, so what the free fn wrote is
/// read after the loop.
const BOUND_FORMS: &str = r#"
type Box { b: Bytes; }
fn keep(bag: Box, piece: Bytes) { bag.b = piece; }
locus Holder {
    params { bag: Box = Box { b: b"" }; }
}
locus Probe {
    params { bag: Box = Box { b: b"" }; }
    run() {
        let buf = std::bytes::BytesBuilder { initial_cap: 64 };
        buf.append_str("abcdefgh");
        let holder = Holder { };
        let spare = Holder { };
        let n = 0;
        let mut i_tail_self = 0;
        while i_tail_self < 2 {
            buf.append_u8(109);
            let alias = { let n = self; n };
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            i_tail_self = i_tail_self + 1;
        }
        println("tail_self=", len(self.bag.b));
        let mut i_tail_holder = 0;
        while i_tail_holder < 2 {
            buf.append_u8(110);
            let alias = { let n = holder; n };
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            i_tail_holder = i_tail_holder + 1;
        }
        println("tail_holder=", len(holder.bag.b));
        let mut i_if = 0;
        while i_if < 2 {
            buf.append_u8(111);
            let alias = if n > 0 { spare } else { holder };
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            i_if = i_if + 1;
        }
        println("if=", len(holder.bag.b));
        let mut i_match = 0;
        while i_match < 2 {
            buf.append_u8(112);
            let alias = match n { 0 -> holder, _ -> spare, };
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            i_match = i_match + 1;
        }
        println("match=", len(holder.bag.b));
        let mut i_tuple = 0;
        while i_tuple < 2 {
            buf.append_u8(113);
            let (alias, z) = (holder, 1);
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            i_tuple = i_tuple + 1;
        }
        println("tuple=", len(holder.bag.b));
        let mut i_assigned = 0;
        while i_assigned < 2 {
            buf.append_u8(114);
            let mut alias = spare; alias = holder;
            let piece = buf.snapshot() or b"";
            keep(alias.bag, piece);
            i_assigned = i_assigned + 1;
        }
        println("assigned=", len(holder.bag.b));
        let mut i_match_self = 0;
        while i_match_self < 2 {
            buf.append_u8(115);
            let piece = buf.snapshot() or b"";
            match self { n -> { keep(n.bag, piece); } }
            i_match_self = i_match_self + 1;
        }
        println("match_self=", len(self.bag.b));
        let mut i_match_holder = 0;
        while i_match_holder < 2 {
            buf.append_u8(116);
            let piece = buf.snapshot() or b"";
            match holder { n -> { keep(n.bag, piece); } }
            i_match_holder = i_match_holder + 1;
        }
        println("match_holder=", len(holder.bag.b));
    }
}

fn main() {
    let p = Probe { };
}
"#;

const BOUND_FORMS_OUT: &[&str] = &[
    "tail_self=10",
    "tail_holder=12",
    "if=14",
    "match=16",
    "tuple=18",
    "assigned=20",
    "match_self=22",
    "match_holder=24",
];

#[test]
fn every_sampled_binding_form_of_a_locus_survives_under_asan() {
    plain_and_asan("bound_forms", BOUND_FORMS, BOUND_FORMS_OUT);
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

/// `fn main`'s loops take the rule too (GH #2094): main allocates into
/// the program's arena, which owns the iteration arenas and lives until
/// the process exits. Each program is a shape the issue measured, its
/// loop in `fn main`; each kept every iteration until `main` returned
/// (peaks of 8 to 31 MiB at these trip counts) and holds one iteration
/// now: a helper building and summarizing records (hale-lang/bench
/// `bounded_records`, driven from main), a `to_json` encode, an
/// `@display` render and the f-string it desugars to, and the GH #2099
/// whole copy of a nested-record list into a record the body makes.
const MAIN_LOOPS: &[(&str, &str)] = &[
    (
        "records",
        r#"
type Tag { name: String = ""; }
type Row { id: String = ""; tags: bounded[Tag; 4]; }
type Rows { items: bounded[Row; 64]; }
type Sum { id: String = ""; tags: bounded[Tag; 4]; }
type Out { items: bounded[Sum; 64]; }

fn summary_of(r: Row) -> Sum {
    let mut v = Sum { id: r.id };
    v.tags = r.tags;
    return v;
}

fn summaries(rs: Rows) -> Out {
    let mut out = Out { };
    for r in rs.items {
        push(out.items, summary_of(r)) or discard;
    }
    return out;
}

fn round(k: Int) -> Int {
    let mut rs = Rows { };
    let mut j = 0;
    while j < 32 {
        let mut r = Row { id: f"row-{k}-{j}" };
        push(r.tags, Tag { name: f"t{j}" }) or discard;
        push(rs.items, r) or discard;
        j = j + 1;
    }
    let out = summaries(rs);
    let mut n = 0;
    for s in out.items {
        n = n + len(s.id) + count(s.tags);
    }
    return n;
}

fn main() {
    let mut total = 0;
    let mut k = 0;
    while k < 2000 {
        total = total + round(k);
        k = k + 1;
    }
    println(total);
}
"#,
    ),
    (
        "to_json",
        r#"
type Quote {
    sym: String `json:"sym"`;
    bid: Int    `json:"bid"`;
    seq: Int    `json:"seq"`;
}

fn main() {
    let mut total = 0;
    let mut i = 0;
    while i < 60000 {
        let q = Quote { sym: "BTCUSD", bid: 50000, seq: i };
        let s = Quote::to_json(q);
        total = total + len(s);
        i = i + 1;
    }
    println(total);
}
"#,
    ),
    (
        "display",
        r#"
@display("${dollars}.{cents:0>2} {currency}")
type Money { dollars: Int; cents: Int; currency: String; }

fn main() {
    let mut total = 0;
    let mut i = 0;
    while i < 100000 {
        let m = Money { dollars: 12, cents: i % 100, currency: "USD" };
        let s = f"{m}";
        let t = f"${m.dollars}.{m.cents:0>2} {m.currency}";
        total = total + len(s) + len(t);
        i = i + 1;
    }
    println(total);
}
"#,
    ),
    (
        "whole_copy",
        r#"
type Meta { note: String = ""; }
type Tag { name: String = ""; meta: Meta = Meta { }; }
type Row { id: String = ""; tags: bounded[Tag; 4]; }

fn main() {
    let mut r = Row { id: "r" };
    let mut j = 0;
    while j < 4 {
        push(r.tags, Tag { name: f"t{j}", meta: Meta { note: f"n{j}" } }) or discard;
        j = j + 1;
    }
    let mut total = 0;
    let mut i = 0;
    while i < 50000 {
        let mut q = Row { id: "q" };
        q.tags = r.tags;
        total = total + count(q.tags);
        i = i + 1;
    }
    println(total);
}
"#,
    ),
];

#[test]
fn a_fn_main_loop_is_reclaimed_per_iteration() {
    for (name, src) in MAIN_LOOPS {
        let (peak, out) = peak_of(&format!("main_{name}"), src, &[]);
        assert!(!out.trim().is_empty(), "{name}: printed nothing");
        assert!(peak < MIB, "{name}: main's loop kept its iterations: chunk_bytes_peak={peak}");
    }
}

/// What a `fn main` loop hands on survives it, read after the loop, plainly
/// and under AddressSanitizer with the chunk pool off: a value stored into
/// a locus field directly and through a method, a record and a `bounded`
/// list stored whole into a locus, a String pushed into a `@form(vec)`, a
/// value sent through a publisher's method, a nested `while` and a `for`
/// over an outside list. A body that makes a locus (let-bound and at
/// statement position) keeps the loop on main's arena, as in a method, and
/// a `return` from inside the loop leaves its iteration to the program's
/// arena.
const MAIN_HANDS_ON: &str = r#"
type Tag { name: String = ""; }
type Msg { text: String = ""; }
type Row { id: String = ""; tags: bounded[Tag; 4]; }

locus Keeper {
    params {
        last: String = "";
        direct: String = "";
        n: Int = 0;
        tags: bounded[Tag; 8];
        row: Row = Row { };
    }
    fn keep(s: String) { self.last = s; self.n = self.n + 1; }
    fn add(t: Tag) { push(self.tags, t) or discard; }
    fn first() -> String { return (at(self.tags, 0) or Tag { }).name; }
}

@form(vec)
locus Names { capacity { heap cells of String; } }

locus Sink {
    params { got: String = ""; seen: Int = 0; }
    bus { subscribe "hand.msg" as on_msg of type Msg; }
    fn on_msg(m: Msg) { self.got = m.text; self.seen = self.seen + 1; }
}

locus Pub {
    bus { publish "hand.msg" of type Msg; }
    fn send(s: String) { "hand.msg" <- Msg { text: s }; }
}

locus Child {
    params { name: String = ""; }
    fn shout() -> String { return self.name + "!"; }
}

fn label(i: Int) -> String { return f"v{i}"; }

fn main() {
    let k = Keeper { };
    let names = Names { };
    let sink = Sink { };
    let p = Pub { };
    let mut src = Row { id: "src" };
    push(src.tags, Tag { name: f"s{1}" }) or discard;
    let mut kept = 0;
    let mut i = 0;
    while i < 50 {
        k.keep(f"keep-{i}");
        k.direct = f"direct-{i}";
        if i < 8 { k.add(Tag { name: f"tag-{i}" }); }
        let mut r = Row { id: label(i) };
        push(r.tags, Tag { name: f"rt-{i}" }) or discard;
        k.row = r;
        k.row.tags = src.tags;
        names.push(f"name-{i}");
        p.send(f"msg-{i}");
        for t in src.tags {
            let u = f"{t.name}-{i}";
            kept = kept + len(u);
        }
        let mut j = 0;
        while j < 3 {
            let w = f"inner-{i}-{j}";
            kept = kept + len(w);
            j = j + 1;
        }
        i = i + 1;
    }
    std::time::sleep(50ms);
    println("last=", k.last, " direct=", k.direct, " n=", k.n);
    println("first=", k.first(), " row=", k.row.id, " row_tag=", (at(k.row.tags, 0) or Tag { }).name);
    println("names=", names.len(), " ", names.get(0) or "none", " ", names.get(49) or "none");
    println("got=", sink.got, " seen=", sink.seen, " kept=", kept);
    let mut n = 0;
    while n < 3 {
        let c = Child { name: f"c{n}" };
        println(c.shout());
        Child { name: f"stmt{n}" };
        n = n + 1;
    }
    let mut m = 0;
    while m < 10 {
        let z = f"z{m}";
        if m == 5 {
            println("returned ", z);
            return;
        }
        m = m + 1;
    }
    println("unreached");
}
"#;

#[test]
fn what_a_fn_main_loop_hands_on_survives_it_under_asan() {
    plain_and_asan(
        "main_hands_on",
        MAIN_HANDS_ON,
        &[
            "last=keep-49 direct=direct-49 n=50",
            "first=tag-0 row=v49 row_tag=s1",
            "names=50 name-0 name-49",
            "got=msg-49 seen=50 kept=",
            "c0!\nc1!\nc2!",
            "returned z5",
        ],
    );
}

/// The program above runs its first loop, the nested `while` and `for`
/// in it, and the returning loop on iteration arenas, and the loop making
/// a locus on main's arena: four enters in `main`, so what the run reads
/// after each loop was handed on from a reclaimed iteration.
#[test]
fn a_fn_main_loop_making_no_locus_takes_an_iteration_arena() {
    let bin = harness::unique_bin("loop_iteration_reclaim_main_ir");
    let ir = harness::build_source_ir_text(MAIN_HANDS_ON, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    let main = ir
        .split("\ndefine ")
        .find(|f| f.contains(" @main("))
        .expect("the program's main in the IR");
    let main = &main[..main.find("\n}\n").unwrap_or(main.len())];
    let enters = main.matches("call ptr @lotus_arena_iteration_enter(").count();
    assert_eq!(enters, 4, "main's loops on iteration arenas");
}

/// An error an `or raise` (or `or fail`) propagates leaves the frame, so
/// the payload a fallible primitive builds for it is made outside the
/// iteration arenas, as a `fail`'s value is: the loop a fallible fn
/// leaves by `or raise` hands the caller a payload whose input String the
/// iteration made, read after the fn returned. An error an `or` handles in
/// place (`v.get(j) or 100` past the end) stays in its iteration. A call
/// through a fn value is handed the iteration's arena when a fn of its
/// type that allocates is named as a value (`label_len`). Plainly and
/// under AddressSanitizer with the chunk pool off.
const ERRORS_AND_FN_VALUES: &str = r#"
@form(vec)
locus IntVec { capacity { heap items of Int; } }

fn shown(e: ParseError) -> Int { println("caught input=", e.input); return -1; }

fn sum_parsed(n: Int) -> Int fallible(ParseError) {
    let mut i = 0;
    let mut t = 0;
    while i < n {
        let text = f"{i}" + (if i == 7 { "x" } else { "" });
        let v = std::str::parse_int(text) or raise;
        t = t + v;
        i = i + 1;
    }
    return t;
}

fn twice(x: Int) -> Int { return x * 2; }
fn label_len(x: Int) -> Int { let s = f"label-{x}"; return len(s); }

fn main() {
    println("ok=", sum_parsed(5) or shown(err));
    println("failed=", sum_parsed(10) or shown(err));
    let v = IntVec { };
    let mut i = 0;
    while i < 10 { v.push(i); i = i + 1; }
    let mut acc = 0;
    let mut j = 0;
    while j < 12 {
        acc = acc + (v.get(j) or 100);
        j = j + 1;
    }
    println("acc=", acc);
    let f: fn(Int) -> Int = if std::process::pid() > 0 { twice } else { label_len };
    let g: fn(Int) -> Int = label_len;
    let mut k = 0;
    let mut s = 0;
    while k < 1000 {
        s = s + f(k) + g(k);
        k = k + 1;
    }
    println("s=", s);
}
"#;

#[test]
fn a_raised_error_and_a_fn_value_call_survive_their_iteration_under_asan() {
    plain_and_asan(
        "errors_and_fn_values",
        ERRORS_AND_FN_VALUES,
        &["ok=10", "caught input=7x", "failed=-1", "acc=245", "s=1007890"],
    );
}

/// A loop whose only allocation is the error it raises (`v.get(j) or
/// raise`, whose error branch a proven bound leaves dead), or whose only
/// calls are of fns that allocate nothing, directly or through a fn value
/// naming only such fns, opens no iteration arena: no enter call stands
/// in it to keep LLVM from vectorizing it, or to cost a call per
/// iteration (GH #2094's bench: `form_vec_get`, `vec_amortized`,
/// `fn_call`).
#[test]
fn a_loop_that_keeps_nothing_in_its_iteration_opens_no_arena() {
    let src = r#"
@form(vec)
locus IntVec { capacity { heap items of Int; } }

fn step(x: Int) -> Int { return x * 2 + 1; }
fn step_odd(x: Int) -> Int { return x * 2 + 3; }

fn main() {
    let v = IntVec { };
    let mut i = 0;
    while i < 1000 { v.push(i); i = i + 1; }
    let mut acc = 0;
    for j in 0..v.len() {
        acc = acc ^ (v.get(j) or raise);
    }
    let mut k = 0;
    while k < 1000 {
        acc = acc + (v.get(k) or raise);
        k = k + 1;
    }
    let f: fn(Int) -> Int = if std::process::pid() % 2 == 0 { step } else { step_odd };
    let mut n = 0;
    while n < 1000 {
        acc = acc + f(n) + step(n);
        n = n + 1;
    }
    println(acc);
}
"#;
    let bin = harness::unique_bin("loop_iteration_reclaim_no_arena_ir");
    let ir = harness::build_source_ir_text(src, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    let main = ir.split("\ndefine ").find(|f| f.contains(" @main(")).expect("the program's main in the IR");
    let main = &main[..main.find("\n}\n").unwrap_or(main.len())];
    assert_eq!(main.matches("call ptr @lotus_arena_iteration_enter(").count(), 0, "no loop of main keeps anything in its iteration");
}

/// A free fn's loops take iteration arenas owned by its per-call
/// subregion, which its epilogue destroys after the copy-out (GH #2094).
/// What each hands on survives, read after the fn returned: stores through
/// a locus parameter (a method storing a String, a record holding a
/// `bounded` list, a String pushed into a `@form(vec)`), a String and a
/// record made in the iteration and returned from inside the loop, a
/// failure made in it, a loop recursing into its own fn, and free fns
/// called from a method's loop. Plainly and under AddressSanitizer with the
/// chunk pool off; the IR test below holds every one of these loops to its
/// iteration arena.
const FREE_FN_HANDS_ON: &str = r#"
type Tag { name: String = ""; }
type Entry { name: String = ""; tags: bounded[Tag; 2]; }

locus Keeper {
    params { last: String = ""; n: Int = 0; }
    fn keep(s: String) { self.last = s; self.n = self.n + 1; }
}

@form(vec)
locus Names { capacity { heap cells of String; } }

locus Reg {
    params { last: Entry = Entry { }; n: Int = 0; }
    fn add(e: Entry) { self.last = e; self.n = self.n + 1; }
}

fn feed(k: Keeper, names: Names, reg: Reg, n: Int) {
    let mut i = 0;
    while i < n {
        k.keep(f"fed-{i}");
        names.push(f"nm-{i}");
        let mut e = Entry { name: f"e-{i}" };
        push(e.tags, Tag { name: f"et-{i}" }) or discard;
        reg.add(e);
        i = i + 1;
    }
}

fn find(n: Int) -> String {
    let mut i = 0;
    while i < 100 {
        let s = f"found-{i}";
        if i == n { return s; }
        i = i + 1;
    }
    return "none";
}

fn find_entry(n: Int) -> Entry {
    let mut i = 0;
    while i < 100 {
        let mut e = Entry { name: f"entry-{i}" };
        push(e.tags, Tag { name: f"tag-{i}" }) or discard;
        if i == n { return e; }
        i = i + 1;
    }
    return Entry { };
}

fn shown(e: String) -> String { return "caught:" + e; }

fn check(n: Int) -> String fallible(String) {
    let mut i = 0;
    while i < 100 {
        let msg = f"bad-{i}";
        if i == n { fail msg; }
        i = i + 1;
    }
    return "passed";
}

fn depth(n: Int) -> Int {
    if n == 0 { return 0; }
    let mut i = 0;
    let mut t = 0;
    while i < 3 {
        let s = f"d{n}-{i}";
        t = t + len(s) + depth(n - 1);
        i = i + 1;
    }
    return t;
}

fn sum_lens(n: Int) -> Int {
    let mut i = 0;
    let mut t = 0;
    while i < n {
        let s = f"piece-{i}";
        t = t + len(s);
        i = i + 1;
    }
    return t;
}

locus Driver {
    params { got: String = ""; total: Int = 0; }
    fn go(n: Int) {
        let mut i = 0;
        while i < n {
            self.got = find(i % 7);
            self.total = self.total + sum_lens(5);
            i = i + 1;
        }
    }
}

fn main() {
    let k = Keeper { };
    let names = Names { };
    let reg = Reg { };
    feed(k, names, reg, 30);
    println("fed ", k.last, " ", k.n, " ", names.len(), " ", names.get(29) or "?", " ", reg.last.name, " ", (at(reg.last.tags, 0) or Tag { }).name);
    println(find(42));
    let e = find_entry(17);
    println(e.name, " ", (at(e.tags, 0) or Tag { }).name);
    println(check(9) or shown(err));
    println(check(500) or shown(err));
    println("depth ", depth(4));
    let d = Driver { };
    d.go(20);
    println("driver ", d.got, " ", d.total);
}
"#;

#[test]
fn what_a_free_fn_loop_hands_on_survives_it_under_asan() {
    plain_and_asan(
        "free_fn_hands_on",
        FREE_FN_HANDS_ON,
        &[
            "fed fed-29 30 30 nm-29 e-29 et-29",
            "found-42",
            "entry-17 tag-17",
            "caught:bad-9",
            "passed",
            "depth 480",
            "driver found-5 700",
        ],
    );
}

#[test]
fn a_free_fn_loop_takes_an_iteration_arena() {
    let bin = harness::unique_bin("loop_iteration_reclaim_free_fn_ir");
    let ir = harness::build_source_ir_text(FREE_FN_HANDS_ON, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    for name in ["feed", "find", "find_entry", "check", "depth", "sum_lens"] {
        let head = format!(" @{name}(");
        let body = ir
            .split("\ndefine ")
            .find(|f| f.lines().next().is_some_and(|l| l.contains(&head)))
            .unwrap_or_else(|| panic!("`{name}` in the IR"));
        let body = &body[..body.find("\n}\n").unwrap_or(body.len())];
        assert_eq!(body.matches("call ptr @lotus_arena_iteration_enter(").count(), 1, "`{name}`'s loop on an iteration arena");
    }
}
