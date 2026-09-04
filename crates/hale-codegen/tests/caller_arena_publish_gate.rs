//! GH #522 — where the caller-arena TLS publish is emitted, both
//! directions.
//!
//! GH #375 fixed a use-after-free by having free-fn prologues
//! publish their `__caller_arena` param to the TLS. The gate for
//! "does this body need it" was a substring search over `{:?}` of
//! the body, so ANY call at all armed it — including a call through
//! a function pointer in a two-deep helper chain, where the publish
//! lands inside a 10M-iteration loop.
//!
//! That cost `fn_modular` **+29%** from v0.14.0 onward, bisected
//! across released binaries (v0.13.0 17.99ms → v0.14.0 23.25ms, flat
//! either side) and confirmed by disassembly: `outer()` went from 5
//! instructions to 15, the difference being
//! `call <lotus_set_caller_arena>`.
//!
//! The gate now also consults `non_allocating` — the same
//! fixed-point classifier that already lets such a fn skip its m49
//! subregion. The TLS exists for allocation sites to read, so a body
//! that provably never allocates has no reader to heal.
//!
//! Both directions are pinned here because only one of them is a
//! performance question. Dropping a publish that an allocation could
//! observe is how #375 comes back, and `caller_arena_tls_unwind.rs`
//! (which runs clean under `LOTUS_ASAN=1`) is the standing
//! reproducer for that. This file states the narrower structural
//! claim: the publish is absent exactly where nothing can read it.
//!
//! Disassembly-based, so Linux-only — same constraint and same
//! rationale as `drain_elision.rs`.

#![cfg(target_os = "linux")]

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

/// Disassemble one function, asserting it survived optimization.
///
/// The assert is not defensiveness. The first version of this file
/// asked about functions LLVM had inlined away, so
/// `objdump --disassemble=<gone>` printed nothing, every count was
/// zero, and the test that should have caught a broken gate passed
/// for exactly the same reason as the one that should have caught a
/// slow one. A count of zero is only meaningful over code that
/// exists.
fn disasm(bin: &std::path::Path, func: &str) -> String {
    let out = Command::new("objdump")
        .args(["-d", &format!("--disassemble={}", func)])
        .arg(bin)
        .output()
        .expect("objdump runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains(&format!("<{}>:", func)),
        "`{}` is not in the binary — it was inlined, so this test \
         would measure nothing. Keep the fn address-taken (assign it \
         to a `fn` value chosen at runtime).",
        func
    );
    text
}

/// Whether the fn's PROLOGUE publishes — i.e. the publish precedes
/// any body work.
///
/// Counting publishes anywhere in the body is not enough, and the
/// first draft of this file got that wrong: call sites publish the
/// arena before stdlib and method calls independently of the
/// prologue gate, so an allocating body registers a nonzero count
/// even with the prologue publish removed entirely. The assertion
/// held while the property it named did not.
///
/// "First call" is not right either — an allocating fn creates its
/// m49 subregion first, so the publish is genuinely second:
///
/// ```text
///   call <lotus_arena_create_subregion>
///   call <lotus_set_caller_arena>      <- the prologue publish
///   call <lotus_obs_locus_birth>       <- body work begins
/// ```
///
/// So: skip arena plumbing, and the next call must be the publish.
fn publishes_in_prologue(bin: &std::path::Path, func: &str) -> bool {
    disasm(bin, func)
        .lines()
        .filter_map(|l| {
            let i = l.find("call")?;
            let sym = l[i..].split('<').nth(1)?.split('>').next()?;
            Some(sym.to_string())
        })
        .find(|sym| !sym.starts_with("lotus_arena_"))
        .map(|first| first == "lotus_set_caller_arena")
        .unwrap_or(false)
}

/// Count `lotus_set_caller_arena` call sites anywhere in one fn.
fn publish_sites(bin: &std::path::Path, func: &str) -> usize {
    disasm(bin, func)
        .lines()
        .filter(|l| l.contains("call") && l.contains("lotus_set_caller_arena"))
        .count()
}

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    bin
}

/// The `fn_modular` shape, reduced: a helper that calls another
/// helper through an OPAQUE function pointer and does arithmetic on
/// the result. Nothing here allocates, so nothing here can read the
/// TLS, so the publish is dead code in the hot loop.
///
/// The pointer is opaque on purpose — a direct call would let the
/// optimizer see through it and prove the point for the wrong
/// reason. This is precisely the shape the old syntactic gate could
/// not reason about: it saw `Call {` and gave up.
#[test]
fn a_non_allocating_body_publishes_nothing_even_through_a_fn_pointer() {
    const SRC: &str = r#"
        fn inner(x: Int) -> Int { return x + 1; }
        fn inner_odd(x: Int) -> Int { return x + 2; }
        fn outer(x: Int, g: fn(Int) -> Int) -> Int { return g(x) * 3; }
        fn outer_odd(x: Int, g: fn(Int) -> Int) -> Int { return g(x) * 5; }
        fn main() {
            let pid = std::process::pid();
            let f: fn(Int, fn(Int) -> Int) -> Int =
                if pid % 2 == 0 { outer } else { outer_odd };
            let g: fn(Int) -> Int =
                if pid % 2 == 0 { inner } else { inner_odd };
            let mut acc = 0;
            let mut i = 0;
            while i < 1000 { acc = f(i, g); i = i + 1; }
            println(acc);
        }
    "#;
    let bin = build("hale_test_ca_nonalloc", SRC);
    let n = publish_sites(&bin, "outer");
    let _ = std::fs::remove_file(&bin);
    assert_eq!(
        n, 0,
        "`outer` allocates nothing, so the caller-arena publish is \
         unreadable work in the caller's loop — got {} site(s)",
        n
    );
}

/// The other direction, and the one that keeps #375 fixed: a body
/// that DOES allocate keeps its publish. Here the free fn builds a
/// locus, which is the exact construct whose C-primitive allocation
/// read the stale TLS in the original report.
#[test]
fn an_allocating_body_still_publishes() {
    const SRC: &str = r#"
        locus Holder {
            params { v: Int = 0; }
            fn read() -> Int { return self.v; }
        }
        fn make(seed: Int) -> Int {
            let h = Holder { v: seed };
            return h.read();
        }
        fn make_odd(seed: Int) -> Int {
            let h = Holder { v: seed + 1 };
            return h.read();
        }
        fn main() {
            let f: fn(Int) -> Int =
                if std::process::pid() % 2 == 0 { make } else { make_odd };
            println(f(7));
        }
    "#;
    let bin = build("hale_test_ca_alloc", SRC);
    let prologue = publishes_in_prologue(&bin, "make");
    let _ = std::fs::remove_file(&bin);
    assert!(
        prologue,
        "a free fn that instantiates a locus must re-heal the TLS on \
         ENTRY (GH #375) — its first call is not the publish"
    );
}

/// A String-building body allocates too, and must keep its publish.
///
/// Worth a case of its own because it is the shape where the two
/// halves of the gate disagree: `non_allocating` correctly calls a
/// String concat allocating, while the syntactic half sees no
/// `Call {` at all. The `&&` must not let that through.
#[test]
fn a_string_building_body_still_publishes() {
    const SRC: &str = r#"
        fn label(n: Int) -> String {
            let s = "n=" + to_string(n);
            return s + "!";
        }
        fn label_odd(n: Int) -> String {
            let s = "m=" + to_string(n);
            return s + "?";
        }
        fn main() {
            let f: fn(Int) -> String =
                if std::process::pid() % 2 == 0 { label } else { label_odd };
            println(f(3));
        }
    "#;
    let bin = build("hale_test_ca_string", SRC);
    let prologue = publishes_in_prologue(&bin, "label");
    let n = publish_sites(&bin, "label");
    let _ = std::fs::remove_file(&bin);
    assert!(
        prologue,
        "a String-building free fn allocates and must publish on \
         entry — its first call is not the publish (saw {} publish \
         site(s) in total)",
        n
    );
}
