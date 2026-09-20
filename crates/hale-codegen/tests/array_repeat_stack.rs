//! GH #767 — a non-escaping `[c; N]` is a stack slot, not N arena
//! stores.
//!
//! `Expr::ArrayRepeat` used to lower to an arena allocation plus N
//! unrolled stores, in both the plain arm and the ascribed
//! (`lower_expr_into`) arm that `let t: [Int; N] = [0; N];` actually
//! takes. A free fn's temporaries live in the CALLER's arena until the
//! caller returns, so a fixed scratch table inside a helper was
//! per-call churn nothing reclaimed while the calling loop ran: 1.69 GB
//! of RSS over 200k calls in the measurement on #754, with 1024 stores
//! and 2090 lines of IR for one local.
//!
//! Now a `[c; N]` bound to a `let` whose every use in the fn is an
//! element access gets an entry-block `alloca` filled with one
//! `llvm.memset` (constant all-zero `c`) or a counted loop. The escape
//! analysis is deliberately one-sided — these tests pin BOTH sides, the
//! shapes that take the stack and the shapes that must keep the arena,
//! because the failure mode of a wrong "does not escape" is a dangling
//! stack pointer, not a leak.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    harness::unique_bin(&format!(
        "lt-array-repeat-stack-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ))
}

fn dump_ir(src: &str, tag: &str) -> String {
    let bin = unique_path(tag, "bin");
    let program = hale_syntax::parse_source(src).expect("parse");
    let ir_text = harness::build_ir_text(&program, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    ir_text
}

fn build_and_run(src: &str, tag: &str) -> (String, std::process::ExitStatus) {
    let bin = unique_path(tag, "bin");
    let program = hale_syntax::parse_source(src).expect("parse");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

/// Slice the IR between `define ... @<fn>(` and the matching `\n}` so
/// an assertion is about one fn and not the whole module (the runtime
/// prelude allocates plenty).
fn fn_body<'a>(ir: &'a str, fn_name: &str) -> &'a str {
    let marker = format!(" @{}(", fn_name);
    let at = ir
        .find(&marker)
        .unwrap_or_else(|| panic!("fn @{} not defined in IR", fn_name));
    let start = ir[..at].rfind("define").expect("`define` precedes the fn");
    let end = ir[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .unwrap_or(ir.len());
    &ir[start..end]
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// The headline: the exact shape the issue measured — a `[0; 1024]`
/// table inside a helper, every use an element read.
#[test]
fn a_non_escaping_table_is_one_alloca_and_one_memset() {
    let src = r#"
        fn scratch_sum(n: Int) -> Int {
            let t: [Int; 1024] = [0; 1024];
            let mut acc = 0;
            acc = acc + t[0];
            acc = acc + t[1023];
            return acc + n;
        }
        fn main() { println(scratch_sum(1)); }
    "#;
    let ir = dump_ir(src, "headline");
    let f = fn_body(&ir, "scratch_sum");
    assert!(
        f.contains("alloca [1024 x i64]"),
        "the table should be a frame slot:\n{}",
        f,
    );
    assert_eq!(
        count(f, "call ptr @lotus_arena_alloc"),
        0,
        "no arena allocation should remain for the table:\n{}",
        f,
    );
    assert_eq!(
        count(f, "llvm.memset"),
        1,
        "a zero fill is exactly one memset:\n{}",
        f,
    );
    // The 1024 unrolled stores are what made the IR 2090 lines.
    assert!(
        count(f, "\n  store ") < 16,
        "the unrolled stores should be gone; got {} in:\n{}",
        count(f, "\n  store "),
        f,
    );
}

/// A `let` inside a LOOP body must reach the entry block, or the frame
/// grows once per iteration — the class of GH #815's per-iteration
/// slot. One alloca, and the memset (the re-initialization the program
/// asked for) inside the body.
#[test]
fn a_table_declared_in_a_loop_is_hoisted_to_the_entry_block() {
    let src = r#"
        fn loop_local(n: Int) -> Int {
            let mut acc = 0;
            let mut i = 0;
            while i < n {
                let t: [Int; 64] = [0; 64];
                acc = acc + t[3];
                i = i + 1;
            }
            return acc;
        }
        fn main() { println(loop_local(4)); }
    "#;
    let ir = dump_ir(src, "loop-hoist");
    let f = fn_body(&ir, "loop_local");
    assert_eq!(
        count(f, "alloca [64 x i64]"),
        1,
        "exactly one slot, hoisted, not one per iteration:\n{}",
        f,
    );
    let entry = f.split("\nwhile.cond:").next().unwrap_or(f);
    assert!(
        entry.contains("alloca [64 x i64]"),
        "the slot must sit in the entry block:\n{}",
        f,
    );
    let body = match f.split_once("\nwhile.body:") {
        Some((_, rest)) => rest,
        None => panic!("expected a while.body block:\n{}", f),
    };
    assert!(
        body.contains("llvm.memset"),
        "the fill must still run every iteration:\n{}",
        f,
    );
}

/// Non-constant `c` cannot be a memset. Past the unroll threshold it
/// becomes a counted loop — never N stores.
#[test]
fn a_non_constant_fill_becomes_a_loop_not_n_stores() {
    let src = r#"
        fn filled(n: Int) -> Int {
            let t: [Int; 256] = [n; 256];
            return t[0] + t[255];
        }
        fn main() { println(filled(3)); }
    "#;
    let ir = dump_ir(src, "fill-loop");
    let f = fn_body(&ir, "filled");
    assert!(
        f.contains("alloca [256 x i64]"),
        "a non-constant fill still gets the frame slot:\n{}",
        f,
    );
    assert!(
        f.contains("array.fill.cond"),
        "expected a counted fill loop:\n{}",
        f,
    );
    assert!(
        count(f, "\n  store ") < 16,
        "256 unrolled stores should be gone; got {} in:\n{}",
        count(f, "\n  store "),
        f,
    );
}

/// Small arrays keep the unrolled form: below the threshold the stores
/// are cheaper than a loop and are the shape the rest of codegen has
/// always emitted.
#[test]
fn a_small_non_constant_fill_stays_unrolled() {
    let src = r#"
        fn small(n: Int) -> Int {
            let t: [Int; 4] = [n; 4];
            return t[0] + t[3];
        }
        fn main() { println(small(3)); }
    "#;
    let ir = dump_ir(src, "small-unrolled");
    let f = fn_body(&ir, "small");
    assert!(
        !f.contains("array.fill.cond"),
        "a 4-element fill should not build a loop:\n{}",
        f,
    );
    assert!(
        !f.contains("llvm.memset"),
        "a non-constant fill cannot be a memset:\n{}",
        f,
    );
    // One GEP + store straight into the slot per element — the
    // unrolled form. (The two later GEPs in this fn go through the
    // binding's own `%t` load, so they don't name the slot.)
    assert_eq!(
        count(f, "getelementptr [4 x i64], ptr %array.repeat.coerced.slot,"),
        4,
        "expected 4 unrolled element stores:\n{}",
        f,
    );
}

/// Escape 1: handed to another fn. The callee can keep the pointer for
/// as long as it likes, so the storage must outlive this frame.
#[test]
fn an_array_passed_to_a_fn_keeps_the_arena() {
    let src = r#"
        fn sink(a: [Int; 32]) -> Int { return a[0] + a[31]; }
        fn passes_it_on() -> Int {
            let t: [Int; 32] = [3; 32];
            return sink(t);
        }
        fn main() { println(passes_it_on()); }
    "#;
    let ir = dump_ir(src, "escape-arg");
    let f = fn_body(&ir, "passes_it_on");
    assert!(
        f.contains("call ptr @lotus_arena_alloc"),
        "an array handed to a callee must stay in the arena:\n{}",
        f,
    );
    assert!(
        !f.contains("alloca [32 x i64]"),
        "and must NOT become a frame slot:\n{}",
        f,
    );
}

/// Escape 2: returned. A frame slot would be dead on arrival.
#[test]
fn a_returned_array_keeps_the_arena() {
    let src = r#"
        fn returns_it() -> [Int; 32] {
            let t: [Int; 32] = [4; 32];
            return t;
        }
        fn main() {
            let r: [Int; 32] = returns_it();
            println(r[7]);
        }
    "#;
    let ir = dump_ir(src, "escape-return");
    let f = fn_body(&ir, "returns_it");
    assert!(
        f.contains("call ptr @lotus_arena_alloc"),
        "a returned array must stay in the arena:\n{}",
        f,
    );
    assert!(
        !f.contains("alloca [32 x i64]"),
        "and must NOT become a frame slot:\n{}",
        f,
    );
}

/// Escape 3: stored into a locus field, which outlives the method.
#[test]
fn an_array_stored_in_a_field_keeps_the_arena() {
    let src = r#"
        locus Holder {
            params {
                cells: [Int; 8] = [0; 8];
            }
            fn stash() {
                let t: [Int; 8] = [5; 8];
                self.cells = t;
            }
            run() {
                self.stash();
                println(self.cells[3]);
            }
        }
        fn main() { Holder { }; }
    "#;
    let ir = dump_ir(src, "escape-field");
    let f = fn_body(&ir, "Holder.stash");
    assert!(
        f.contains("call ptr @lotus_arena_alloc"),
        "an array stored into a field must stay in the arena:\n{}",
        f,
    );
    assert!(
        !f.contains("alloca [8 x i64]"),
        "and must NOT become a frame slot:\n{}",
        f,
    );
}

/// Escape 4: aliased by a second binding. `u` is an ordinary local the
/// analysis does not track, so `t` is out.
#[test]
fn an_aliased_array_keeps_the_arena() {
    let src = r#"
        fn aliased() -> Int {
            let t: [Int; 32] = [6; 32];
            let u = t;
            return u[0];
        }
        fn main() { println(aliased()); }
    "#;
    let ir = dump_ir(src, "escape-alias");
    let f = fn_body(&ir, "aliased");
    assert!(
        f.contains("call ptr @lotus_arena_alloc"),
        "an aliased array must stay in the arena:\n{}",
        f,
    );
    assert!(
        !f.contains("alloca [32 x i64]"),
        "and must NOT become a frame slot:\n{}",
        f,
    );
}

/// The cap. A cooperative-pool coroutine stack is 64 KiB
/// (`LOTUS_CORO_STACK_BYTES`), and any free fn can be reached from a
/// handler running on one, so a table past `STACK_ARRAY_MAX_BYTES`
/// keeps the arena rather than risking the overflow class that took
/// `udp` recv down in #221. `[Int; 2048]` is 16 KiB.
#[test]
fn a_table_over_the_cap_keeps_the_arena() {
    let src = r#"
        fn over_cap() -> Int {
            let big: [Int; 2048] = [0; 2048];
            return big[2047];
        }
        fn main() { println(over_cap()); }
    "#;
    let ir = dump_ir(src, "over-cap");
    let f = fn_body(&ir, "over_cap");
    assert!(
        f.contains("call ptr @lotus_arena_alloc"),
        "a 16 KiB table must stay in the arena:\n{}",
        f,
    );
    assert!(
        !f.contains("alloca [2048 x i64]"),
        "and must NOT become a frame slot:\n{}",
        f,
    );
    // The fill still improved: one memset, not 2048 stores.
    assert_eq!(
        count(f, "llvm.memset"),
        1,
        "the arena path gets the memset too:\n{}",
        f,
    );
}

/// The cap is a per-fn TOTAL, not per array: 8 KiB is one `[Int;
/// 1024]`, so a second one in the same body keeps the arena.
#[test]
fn the_cap_is_a_per_fn_budget() {
    let src = r#"
        fn two_tables(n: Int) -> Int {
            let a: [Int; 1024] = [0; 1024];
            let b: [Int; 1024] = [0; 1024];
            return a[0] + b[0] + n;
        }
        fn main() { println(two_tables(1)); }
    "#;
    let ir = dump_ir(src, "budget");
    let f = fn_body(&ir, "two_tables");
    assert_eq!(
        count(f, "alloca [1024 x i64]"),
        1,
        "only the first table fits the frame budget:\n{}",
        f,
    );
    assert_eq!(
        count(f, "call ptr @lotus_arena_alloc"),
        1,
        "the second falls back to the arena:\n{}",
        f,
    );
}

/// The values have to be right: a zero fill, a non-zero constant fill,
/// a Float fill, and element writes through the slot.
#[test]
fn stack_tables_compute_the_right_values() {
    let src = r#"
        fn zeros() -> Int {
            let t: [Int; 64] = [0; 64];
            let mut acc = 0;
            let mut i = 0;
            while i < 64 { acc = acc + t[i]; i = i + 1; }
            return acc;
        }
        fn sevens() -> Int {
            let t: [Int; 100] = [7; 100];
            let mut acc = 0;
            let mut i = 0;
            while i < 100 { acc = acc + t[i]; i = i + 1; }
            return acc;
        }
        fn written() -> Int {
            let mut t: [Int; 32] = [0; 32];
            let mut i = 0;
            while i < 32 { t[i] = i * 2; i = i + 1; }
            return t[0] + t[31];
        }
        fn floats() -> Float {
            let t: [Float; 40] = [0.5; 40];
            return t[0] + t[39];
        }
        fn main() {
            print("zeros=");
            println(zeros());
            print("sevens=");
            println(sevens());
            print("written=");
            println(written());
            print("floats=");
            println(floats());
        }
    "#;
    let (stdout, status) = build_and_run(src, "values");
    assert!(status.success(), "exit: {:?} out: {:?}", status, stdout);
    assert!(stdout.contains("zeros=0"), "got: {:?}", stdout);
    assert!(stdout.contains("sevens=700"), "got: {:?}", stdout);
    assert!(stdout.contains("written=62"), "got: {:?}", stdout);
    assert!(stdout.contains("floats=1"), "got: {:?}", stdout);
}

/// The measurement from the issue, as a bound. 200k calls to a helper
/// holding a `[0; 1024]` table grew RSS by 1.69 GB before this; the
/// table is now a frame slot and RSS is flat.
///
/// The bound is read INSIDE the child (`std::process::rss_bytes`), not
/// from the test process — measuring the harness would report cargo's
/// own footprint and the assertion would mean nothing (GH #810).
#[test]
fn two_hundred_thousand_calls_keep_rss_flat() {
    let src = r#"
        fn scratch_sum(n: Int) -> Int {
            let t: [Int; 1024] = [0; 1024];
            let mut acc = 0;
            acc = acc + t[0];
            acc = acc + t[1023];
            return acc + n;
        }
        fn main() {
            let mut i = 0;
            let mut total = 0;
            while i < 200000 {
                total = total + scratch_sum(i);
                i = i + 1;
            }
            print("total=");
            println(total);
            print("final_rss_mb=");
            println(std::process::rss_bytes() / 1048576);
        }
    "#;
    let (stdout, status) = build_and_run(src, "rss");
    assert!(status.success(), "exit: {:?} out: {:?}", status, stdout);
    assert!(
        stdout.contains("total=19999900000"),
        "the loop must still compute the same thing: {:?}",
        stdout,
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with("final_rss_mb="))
        .unwrap_or_else(|| panic!("no final_rss_mb in {:?}", stdout));
    let rss: i64 = line
        .trim_start_matches("final_rss_mb=")
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("unparseable: {:?}", line));
    // 400 MB, not 100: the neighbouring RSS-bounded tests
    // (`alloc_model_rss`, `json_range_helpers`) trip on a loaded
    // machine because the runtime's baseline arena inflates under
    // contention, and a bound that flakes teaches nothing. 1679 MB is
    // what the regression costs, so this still bites by 4x.
    assert!(
        rss < 400,
        "200k calls with a [0; 1024] local took {} MB of RSS — the \
         table is back in the caller's arena (it was 1679 MB before \
         GH #767, 73 MB after)",
        rss,
    );
}
