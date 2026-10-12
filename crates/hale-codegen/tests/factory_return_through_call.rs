//! A locus handed back through a call that returns its argument stays
//! alive and is reclaimed once, by the caller.
//!
//! `fn via_let(n: Int) -> Buf { let x = make(n, 2.0); return pass(x); }`
//! with `fn pass(a: Buf) -> Buf { return a; }`: the value leaves
//! `via_let` through `pass`, which names no binding, so `via_let`'s
//! frame reclaimed `x` on the way out and the caller got a dissolved
//! `@form(vec)` whose buffer was gone — `len()` 0, every `get` failing,
//! exit 0. The unbound `return pass(make(n, 1.0))` did the same through
//! the frame temporary. Only `main`'s frame hid it, by outliving every
//! use (friction sweep 2026-10-12, downstream handoff).
//!
//! Two halves fix it. A free fn's exit flush skips the entry holding
//! the locus it returns. And a call to a fn that hands back its
//! parameters is, to the fresh-factory walk, the arguments it can hand
//! back, so `via_let` is a factory and its caller owns what it returns.
//! `pick` hands back one of two: the other is the frame's, and goes.
//!
//! Each `dissolve()` prints its tag, so a missing teardown is a missing
//! line and a double one a repeated line; the values are read from the
//! vec's heap buffer, so the ASan run sees a read of a freed one.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const SRC: &str = r#"
@form(vec)
locus Buf {
    params { n: Int; }
    capacity { heap data of Float; }
    dissolve() { println("D:", self.n); }
}
fn make(n: Int, seed: Float) -> Buf {
    let b = Buf { n: n };
    let mut i = 0;
    let mut v = seed;
    while i < n { b.push(v); v = v + 1.0; i = i + 1; }
    return b;
}
fn pass(a: Buf) -> Buf { return a; }
fn pass2(a: Buf) -> Buf { return pass(a); }
fn pick(a: Buf, b: Buf, first: Bool) -> Buf {
    if first { return a; }
    return b;
}
fn via_direct(n: Int) -> Buf { return make(n, 10.0); }
fn via_temp(n: Int) -> Buf { return pass(make(n, 20.0)); }
fn via_let(n: Int) -> Buf {
    let x = make(n, 30.0);
    return pass2(x);
}
fn via_literal(n: Int) -> Buf { return pass(Buf { n: n }); }
fn via_pick(n: Int) -> Buf {
    let x = make(n, 40.0);
    let y = make(n + 10, 50.0);
    return pick(x, y, false);
}
fn via_fallible(n: Int) -> Buf fallible(String) {
    if n < 0 { fail "negative"; }
    let x = make(n, 60.0);
    return pass(x);
}
fn show(label: String, b: Buf) {
    println(label, " len=", b.len(), " first=", b.get(0) or -1.0, " last=", b.get(b.len() - 1) or -1.0);
}
@unbounded
fn work() {
    let mut i = 0;
    while i < 2 {
        let d = via_direct(1);
        let t = via_temp(2);
        let l = via_let(3);
        let k = via_literal(4);
        let p = via_pick(5);
        let f = via_fallible(6) or make(0, 0.0);
        show("direct", d);
        show("temp", t);
        show("let", l);
        println("literal n=", k.n, " len=", k.len());
        show("pick", p);
        show("fallible", f);
        i = i + 1;
    }
}
fn main() {
    work();
    println("end");
}
"#;

/// One iteration's reads, the same both times round.
const ROUND: &str = "direct len=1 first=10 last=10
temp len=2 first=20 last=21
let len=3 first=30 last=32
literal n=4 len=0
pick len=15 first=50 last=64
fallible len=6 first=60 last=65
";

/// The whole run: `pick`'s first argument (n=5) goes as `via_pick`
/// returns; the first round's values go as the second round rebinds
/// each slot (in binding order, `via_pick`'s own n=5 between `k`'s and
/// `p`'s), and the second round's at `work`'s exit (in reverse).
fn expected() -> String {
    [
        "D:5\n",
        ROUND,
        "D:1\nD:2\nD:3\nD:4\nD:5\nD:15\nD:6\n",
        ROUND,
        "D:6\nD:15\nD:4\nD:3\nD:2\nD:1\nend\n",
    ]
    .concat()
}

#[test]
fn a_locus_handed_back_through_a_call_is_the_callers() {
    let bin = harness::unique_bin("factory_return_through_call");
    build_opts::build_source(SRC, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stderr: {stderr}", out.status);
    assert_eq!(stdout, expected(), "stderr: {stderr}");
    assert!(
        stderr.contains("0 live arenas"),
        "every Buf's arena is reclaimed by exit; stderr: {stderr}"
    );
}

#[test]
fn a_locus_handed_back_through_a_call_reads_back_under_asan() {
    let bin = harness::unique_bin("factory_return_through_call_asan");
    harness::build_source_asan(SRC, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert_eq!(stdout, expected(), "stderr: {stderr}");
}
