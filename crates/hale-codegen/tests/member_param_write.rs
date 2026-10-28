//! A locus method that writes a heap value through a record parameter
//! writes into the caller's record, so the value is made in the arena
//! that owns the record, the one the caller published, as a free fn's
//! write lands in its `__caller_arena`.
//!
//! Before, `fn put(x: Bag, p: Bytes) { x.b = p; }` on a locus cloned `p`
//! into the method's own per-call scratch, which the method's return
//! frees: the caller's record was left holding a pointer into freed
//! memory, a heap-use-after-free freed in `Sink.put` in a program with no
//! loop. The same through a nested path (`x.inner.b = p`), an element of
//! an array parameter, and a record parameter handed on to another
//! method, a free fn or another locus's method, whose own caller arena
//! was the first method's scratch.
//!
//! A `bounded` field assigned whole through the parameter anchors its
//! elements in the same arena.
//!
//! Each program is built under AddressSanitizer with the chunk pool off,
//! so a read of the freed scratch is a report, not a silent read of
//! recycled bytes. The last one is #1765's per-iteration reclaim: a loop
//! handing such a method a record declared before it keeps the loop on
//! the frame's scratch, and the record still holds every write after the
//! loop.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The records, a byte-string maker whose result is on the heap (a
/// literal is static and would hide a dangling pointer), and the locus
/// `Sink` with `METHODS`, `main` running `MAIN` after binding `s` and
/// `bag`.
const PROGRAM: &str = r#"
type Inner { b: Bytes; }
type Bag { b: Bytes; s: String; inner: Inner; pair: (Int, String); items: bounded[Bytes; 4]; }

fn fresh(n: Int) -> Bytes {
    let buf = std::bytes::BytesBuilder { initial_cap: 16 };
    let mut i = 0;
    while i < n {
        buf.append_u8(97 + i);
        i = i + 1;
    }
    return buf.snapshot() or b"";
}

fn put_free(x: Bag, p: Bytes) { x.b = p; }

locus Other {
    fn put(x: Bag, p: Bytes) { x.b = p; }
}

locus Sink {
    params { o: Other = Other { }; }
    METHODS
}

fn main() {
    let s = Sink { };
    let bag = Bag { b: b"", s: "", inner: Inner { b: b"" }, pair: (0, "") };
    MAIN
}
"#;

/// Build `PROGRAM` with `methods` and `main` under AddressSanitizer, run
/// it, and hold it to `want` on stdout with no sanitizer report.
fn asan_clean(name: &str, methods: &str, main: &str, want: &[&str]) {
    let src = PROGRAM.replace("METHODS", methods).replace("MAIN", main);
    let bin = harness::unique_bin(&format!("member_param_write_{name}"));
    let opts = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(&src, &bin, &opts).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("AddressSanitizer"), "{name}: ASan report:\n{stderr}");
    assert!(out.status.success(), "{name}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    for w in want {
        assert!(stdout.contains(w), "{name}: missing {w:?} in:\n{stdout}");
    }
}

#[test]
fn a_bytes_field_written_through_a_record_parameter_outlives_the_method() {
    asan_clean(
        "bytes",
        "fn put(x: Bag, p: Bytes) { x.b = p; }",
        r#"s.put(bag, fresh(5));
    println("put=", std::hex::encode(bag.b));"#,
        &["put=6162636465"],
    );
}

#[test]
fn a_string_field_written_through_a_record_parameter_outlives_the_method() {
    asan_clean(
        "string",
        r#"fn name(x: Bag, s: String) { x.s = s; }
    fn grow(x: Bag, s: String) { x.s += s; }"#,
        r#"s.name(bag, "na" + to_string(42));
    println("name=", bag.s);
    s.grow(bag, "-g" + to_string(1));
    s.grow(bag, "-h" + to_string(2));
    println("grow=", bag.s);"#,
        &["name=na42", "grow=na42-g1-h2"],
    );
}

#[test]
fn a_nested_path_written_through_a_record_parameter_outlives_the_method() {
    asan_clean(
        "nested",
        r#"fn deep(x: Bag, p: Bytes) { x.inner.b = p; }
    fn whole(x: Bag, p: Bytes) { x.inner = Inner { b: p }; }
    fn pair(x: Bag, n: Int) { x.pair = (n, "p" + to_string(n)); }
    fn slot(a: [Bytes; 2], p: Bytes) { a[1] = p; }
    fn take(x: Bag, y: Bag) { x.items = y.items; }"#,
        r#"s.deep(bag, fresh(6));
    println("deep=", std::hex::encode(bag.inner.b));
    s.whole(bag, fresh(3));
    println("whole=", std::hex::encode(bag.inner.b));
    s.pair(bag, 7);
    println("pair=", bag.pair.1);
    let a = [b"", b""];
    s.slot(a, fresh(2));
    println("slot=", std::hex::encode(a[1]));
    let from = Bag { b: b"", s: "", inner: Inner { b: b"" }, pair: (0, "") };
    push(from.items, fresh(4)) or discard;
    s.take(bag, from);
    for e in bag.items {
        println("items=", std::hex::encode(e));
    }"#,
        &["deep=616263646566", "whole=616263", "pair=p7", "slot=6162", "items=61626364"],
    );
}

/// The record handed on: to another method of the same locus, to a
/// method of a child locus, to a free fn, and as a part of itself
/// (`x.inner`). Each callee's caller arena is what the first method
/// publishes, which is its own caller's.
#[test]
fn a_record_parameter_handed_on_to_another_method_outlives_both() {
    asan_clean(
        "handed_on",
        r#"fn put(x: Bag, p: Bytes) { x.b = p; }
    fn relay(x: Bag, p: Bytes) { self.put(x, p); }
    fn relay2(x: Bag, p: Bytes) { self.relay(x, p); }
    fn via_child(x: Bag, p: Bytes) { self.o.put(x, p); }
    fn via_free(x: Bag, p: Bytes) { put_free(x, p); }
    fn put_inner(i: Inner, p: Bytes) { i.b = p; }
    fn via_part(x: Bag, p: Bytes) { self.put_inner(x.inner, p); }"#,
        r#"s.relay(bag, fresh(7));
    println("relay=", std::hex::encode(bag.b));
    s.relay2(bag, fresh(2));
    println("relay2=", std::hex::encode(bag.b));
    s.via_child(bag, fresh(4));
    println("via_child=", std::hex::encode(bag.b));
    s.via_free(bag, fresh(3));
    println("via_free=", std::hex::encode(bag.b));
    s.via_part(bag, fresh(5));
    println("via_part=", std::hex::encode(bag.inner.b));"#,
        &[
            "relay=61626364656667",
            "relay2=6162",
            "via_child=61626364",
            "via_free=616263",
            "via_part=6162636465",
        ],
    );
}

/// A loop inside the method writes through the parameter on every
/// iteration: the last write is what the caller reads.
#[test]
fn a_loop_in_the_method_writing_through_the_parameter_outlives_it() {
    asan_clean(
        "callee_loop",
        r#"fn many(x: Bag, n: Int) {
        let mut i = 0;
        while i < n {
            x.s = "it" + to_string(i);
            x.b = fresh(i + 1);
            i = i + 1;
        }
    }"#,
        r#"s.many(bag, 4);
    println("many=", bag.s, " ", std::hex::encode(bag.b));"#,
        &["many=it3 61626364"],
    );
}

/// #1765's rule: a loop body handing a call storage held outside it (a
/// record declared before the loop) is not reclaimed per iteration, so
/// what the method writes into the record, made in the arena the loop
/// runs in, is there after the loop. The loop runs in a `run()`, whose
/// scratch is the frame's arena, and in `main`.
#[test]
fn a_loop_handing_the_method_an_outer_record_keeps_every_write() {
    let src = r#"
type Bag { b: Bytes; s: String; }

locus Sink {
    fn put(x: Bag, p: Bytes) { x.b = p; }
    fn name(x: Bag, s: String) { x.s = s; }
}

locus Driver {
    params { sink: Sink = Sink { }; }
    run() {
        let buf = std::bytes::BytesBuilder { initial_cap: 16 };
        let outer = Bag { b: b"", s: "" };
        let mut i = 0;
        while i < 3 {
            buf.append_u8(97 + i);
            let piece = buf.snapshot() or b"";
            self.sink.put(outer, piece);
            self.sink.name(outer, "n" + to_string(i));
            i = i + 1;
        }
        println("run=", std::hex::encode(outer.b), " ", outer.s);
    }
}

fn main() {
    let d = Driver { };
    let s = Sink { };
    let buf = std::bytes::BytesBuilder { initial_cap: 16 };
    let outer = Bag { b: b"", s: "" };
    for i in 0..4 {
        buf.append_u8(100 + i);
        let piece = buf.snapshot() or b"";
        s.put(outer, piece);
        s.name(outer, "m" + to_string(i));
    }
    println("main=", std::hex::encode(outer.b), " ", outer.s);
}
"#;
    let bin = harness::unique_bin("member_param_write_outer_loop");
    let opts = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("AddressSanitizer"), "ASan report:\n{stderr}");
    assert!(out.status.success(), "exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    for w in ["run=616263 n2", "main=64656667 m3"] {
        assert!(stdout.contains(w), "missing {w:?} in:\n{stdout}");
    }
}

/// A member the bus delivers to is published no caller arena: its entry
/// snapshot is the program's payload-arena fallback, which lives as long
/// as the program. A write through its payload parameter keeps the
/// member's scratch there, as before, so 20,000 deliveries hold no more
/// than a few chunks, delivered on the publisher's pool and on a pool of
/// the subscriber's own.
#[test]
fn a_delivered_member_writing_through_its_payload_stays_flat() {
    let src = r#"
type Msg { n: Int; s: String; }
topic Ping { payload: Msg; subject: "t.ping"; }

locus Sub {
    params { seen: Int = 0; last: String = ""; }
    bus { subscribe Ping as on_ping; }
    fn on_ping(m: Msg) {
        m.s = "seen-" + to_string(m.n) + "-" + to_string(self.seen);
        self.last = m.s;
        self.seen = self.seen + 1;
    }
}

locus Pub {
    bus { publish Ping; }
    run() {
        let mut i = 0;
        while i < 20000 {
            Ping <- Msg { n: i, s: "" };
            i = i + 1;
            if i % 100 == 0 {
                yield;
            }
        }
    }
}

main locus App {
    params { sub: Sub = Sub { }; p: Pub = Pub { }; }
    PLACEMENT
    run() {
        std::time::sleep(500ms);
        println("last=", self.sub.last);
    }
}

fn main() { App { }; }
"#;
    for (name, placement) in [("same_pool", ""), ("own_pool", "placement { sub: cooperative(pool = io); }")] {
        let program = src.replace("PLACEMENT", placement);
        for asan in [false, true] {
            let bin = harness::unique_bin(&format!("member_param_write_delivered_{name}_{asan}"));
            let opts = hale_codegen::BuildOptions { asan, ..build_opts::options() };
            build_opts::build_source(&program, &bin, &opts).expect("build");
            let mut cmd = Command::new(&bin);
            // The fallback is capped: a member that stored into it on every
            // delivery would exhaust a small cap long before the last one.
            cmd.env("LOTUS_BUS_PAYLOAD_ARENA_CAP", "262144");
            if asan {
                cmd.env("LOTUS_NO_CHUNK_POOL", "1").env("ASAN_OPTIONS", "detect_leaks=0");
            } else {
                cmd.env("LOTUS_ARENA_RESIDENCY", "1");
            }
            let out = cmd.output().expect("run");
            let _ = std::fs::remove_file(&bin);
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            assert!(!stderr.contains("AddressSanitizer"), "{name}: ASan report:\n{stderr}");
            assert!(out.status.success(), "{name} asan={asan}: exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
            assert!(stdout.contains("last=seen-19999-19999"), "{name} asan={asan}: got:\n{stdout}");
            if !asan {
                let peak: u64 = stderr
                    .lines()
                    .find_map(|l| l.strip_prefix("[arena_residency peak] chunk_bytes_peak="))
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| panic!("{name}: no residency peak line\nstderr:\n{stderr}"));
                assert!(peak < 1 << 20, "{name}: 20,000 deliveries held {peak} bytes of chunks");
            }
        }
    }
}

/// A record a field of `self` holds (`self.bag`, `self.outer.bag`)
/// handed to a callee that writes through it, as a place or through a
/// block or an `if`: the locus's arena owns the record, so the call
/// publishes that arena. Before, the call published the method's
/// scratch, so the record kept a block its return freed (a
/// use-after-free, through a free fn or another locus's method).
const SELF_HELD: &str = r#"
type Bag { b: Bytes; s: String; }
type Outer { bag: Bag; }

fn put_free(x: Bag, p: Bytes) { x.b = p; }

locus Sink {
    fn put(x: Bag, p: Bytes) { x.b = p; }
    fn name(x: Bag, s: String) { x.s = s; }
}

locus Holder {
    params {
        bag: Bag = Bag { b: b"", s: "" };
        outer: Outer = Outer { bag: Bag { b: b"", s: "" } };
        sink: Sink = Sink { };
    }
    fn fill(n: Int) {
        let buf = std::bytes::BytesBuilder { initial_cap: 16 };
        let mut i = 0;
        while i <= n {
            buf.append_u8(97 + i);
            i = i + 1;
        }
        let piece = buf.snapshot() or b"";
        put_free(self.bag, piece);
        self.sink.name({ self.bag }, "f" + to_string(n * 1000));
        self.sink.put(if n > 100 { self.bag } else { self.outer.bag }, piece);
    }
    fn churn(k: Int) {
        let mut i = 0;
        while i < k {
            self.fill(i % 7);
            i = i + 1;
        }
    }
    fn show() {
        println("bag=", std::hex::encode(self.bag.b), " ", self.bag.s, " outer=", std::hex::encode(self.outer.bag.b));
    }
}

fn main() {
    let h = Holder { };
    h.fill(1);
    h.show();
    h.churn(ROUNDS);
    h.show();
}
"#;

#[test]
fn a_record_a_locus_holds_handed_to_a_writer_outlives_the_method() {
    let src = SELF_HELD.replace("ROUNDS", "10");
    let bin = harness::unique_bin("member_param_write_self_held_asan");
    let opts = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(&src, &bin, &opts).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("AddressSanitizer"), "ASan report:\n{stderr}");
    assert!(out.status.success(), "exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
    for w in ["bag=6162 f1000 outer=6162", "bag=616263 f2000 outer=616263"] {
        assert!(stdout.contains(w), "missing {w:?} in:\n{stdout}");
    }
}

/// A record parameter handed on through a carrier: a block, an `if` and
/// a `match` yielding the parameter carry the caller's record itself, not
/// a copy, so the writer is handed the arena that owns it, as for the
/// parameter handed by name.
#[test]
fn a_record_parameter_handed_on_through_a_carrier_outlives_both() {
    asan_clean(
        "carriers",
        r#"fn put(x: Bag, p: Bytes) { x.b = p; }
    fn via_block(x: Bag, p: Bytes) { self.put({ x }, p); }
    fn via_if(x: Bag, p: Bytes) { self.put(if len(p) > 0 { x } else { x }, p); }
    fn via_match(x: Bag, p: Bytes) { self.put(match len(p) { 0 -> x, _ -> x, }, p); }
    fn via_free_block(x: Bag, p: Bytes) { put_free({ x }, p); }"#,
        r#"s.via_block(bag, fresh(2));
    println("block=", std::hex::encode(bag.b));
    s.via_if(bag, fresh(3));
    println("if=", std::hex::encode(bag.b));
    s.via_match(bag, fresh(4));
    println("match=", std::hex::encode(bag.b));
    s.via_free_block(bag, fresh(5));
    println("free_block=", std::hex::encode(bag.b));"#,
        &["block=6162", "if=616263", "match=61626364", "free_block=6162636465"],
    );
}
