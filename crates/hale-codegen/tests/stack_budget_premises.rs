//! The premises `@budget(stack_bytes)` rests on (#326).
//!
//! The estimator is 32 bytes of call overhead, 8 per parameter, 8 per
//! local. Whether that is sound has nothing to do with the arithmetic
//! and everything to do with a fact about Hale's memory model:
//!
//!     ALMOST NOTHING IS ON THE STACK.
//!
//! Fixed arrays, structs and string/bytes buffers are arena-allocated,
//! so a local is a pointer and 8 bytes is close to *correct* for the
//! scalars that remain. The same estimator in C would be wrong by
//! orders of magnitude — a `char buf[65536]` local is 8 bytes to this
//! model and 64 KiB to the machine.
//!
//! That premise is load-bearing and invisible. If a shape ever became
//! stack-allocated — for performance, for a new backend, for wasm —
//! the estimate would silently under-count by the size of that shape,
//! and every `@budget(stack_bytes)` certificate in the tree would
//! quietly become wrong. Nothing would fail; the numbers would just
//! stop meaning anything.
//!
//! So the premise is pinned here rather than assumed. These tests do
//! not check the budget arithmetic (that has its own tests) — they
//! check the fact the arithmetic depends on.
//!
//! What they deliberately do NOT establish: that the estimate bounds
//! actual machine stack. It does not, and cannot. Register spills are
//! invisible to any source-level model, and at `-O3` with
//! `target-cpu=native` a spilled AVX-512 register is 64 bytes against
//! a model whose unit is 8. See spec/verification.md for the full
//! statement; settling that needs post-codegen measurement
//! (`.stack_sizes`), which is not wired up.

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Emit pre-optimization IR for a program and return it.
fn ir_for(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_stackprem_{}_{}",
        name,
        std::process::id()
    ));
    std::env::set_var("LOTUS_DUMP_IR", "1");
    build_executable(&program, &bin).expect("build");
    std::env::remove_var("LOTUS_DUMP_IR");
    let ll = bin.with_extension("ll");
    let ir = std::fs::read_to_string(&ll).expect("IR dumped");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    ir
}

/// Stack bytes an `alloca` reserves, when that is statically
/// obvious. Only the TYPE position is parsed — matching digits
/// anywhere in the line picks up SSA names (`%req96`) and `align 8`,
/// which is how the first version of this test reported a String
/// local as a 96-byte stack buffer.
///
/// Handles the two shapes that matter: `alloca [N x T]` and
/// `alloca T, i64 N`. Anything else counts as small, which is the
/// safe direction for a premise check — a missed shape shows up as a
/// passing test, not a false alarm, and the budget arithmetic has its
/// own coverage.
fn alloca_extent(line: &str) -> usize {
    let Some(rest) = line.split_once("alloca ").map(|(_, r)| r) else {
        return 0;
    };
    let rest = rest.trim();
    // `[4096 x i64]`
    if let Some(inner) = rest.strip_prefix('[') {
        if let Some((n, _)) = inner.split_once(" x ") {
            return n.trim().parse::<usize>().unwrap_or(0);
        }
    }
    // An aggregate type contains its own commas — `{ i64, i64 }` was
    // matching the explicit-count branch below and reporting its
    // alignment as a size. Only a SIMPLE type can carry a count.
    if rest.starts_with('{') || rest.starts_with('<') {
        return 0;
    }
    // `i8, i64 65536, align 16`
    if let Some((_, count)) = rest.split_once(", i64 ") {
        return count
            .split(|c: char| !c.is_ascii_digit())
            .find(|s| !s.is_empty())
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
    }
    0
}

fn big_allocas(ir: &str, min_elems: usize) -> Vec<String> {
    ir.lines()
        .filter(|l| l.contains("alloca"))
        .filter(|l| alloca_extent(l) > min_elems)
        .map(|l| l.trim().to_string())
        .collect()
}

/// THE premise. A large fixed-size array local must not become a
/// stack allocation, or the estimator's 8-bytes-per-local under-counts
/// it by 32 KiB.
#[test]
fn a_large_array_local_is_not_on_the_stack() {
    let ir = ir_for(
        "array",
        "@budget(stack_bytes = 64)\n\
         fn big(n: Int) -> Int {\n\
             let mut buf: [Int; 4096] = [0; 4096];\n\
             let mut i = 0;\n\
             while i < 4096 { buf[i] = i * n; i = i + 1; }\n\
             return buf[n];\n\
         }\n\
         fn main() { println(big(3)); }",
    );
    let big = big_allocas(&ir, 1024);
    assert!(
        big.is_empty(),
        "a [Int; 4096] local became a stack allocation. \
         `@budget(stack_bytes)` estimates 8 bytes for it, so every \
         such certificate now under-counts by ~32 KiB — see \
         spec/verification.md § stack_bytes. Offending alloca(s):\n{}",
        big.join("\n")
    );
}

/// Same premise for a wide struct: the estimator charges 8 for the
/// local regardless of the struct's size.
#[test]
fn a_wide_struct_local_is_not_on_the_stack() {
    let ir = ir_for(
        "struct",
        "type Wide {\n\
             a: Int; b: Int; c: Int; d: Int; e: Int; f: Int;\n\
             g: Int; h: Int; i: Int; j: Int; k: Int; l: Int;\n\
         }\n\
         @budget(stack_bytes = 64)\n\
         fn mk(n: Int) -> Int {\n\
             let w = Wide { a: n, b: n, c: n, d: n, e: n, f: n,\n\
                            g: n, h: n, i: n, j: n, k: n, l: n };\n\
             return w.a + w.l;\n\
         }\n\
         fn main() { println(mk(2)); }",
    );
    let big = big_allocas(&ir, 64);
    assert!(
        big.is_empty(),
        "a wide struct local became a stack allocation; the estimator \
         charges 8 bytes for it regardless of width:\n{}",
        big.join("\n")
    );
}

/// A String local is a pointer — the buffer is arena/heap. The
/// estimator's `Bytes/String local carries a pointer` comment is the
/// claim; this is the check.
#[test]
fn a_string_local_is_a_pointer() {
    let ir = ir_for(
        "string",
        "@budget(stack_bytes = 64)\n\
         fn s(n: Int) -> Int {\n\
             let t = \"abcdefghijklmnopqrstuvwxyz\";\n\
             return std::str::index_of(t, \"z\") + n;\n\
         }\n\
         fn main() { println(s(1)); }",
    );
    let big = big_allocas(&ir, 64);
    assert!(
        big.is_empty(),
        "a String local put its buffer on the stack:\n{}",
        big.join("\n")
    );
}

/// The budget must still FIRE — a premise test that passed because
/// the analysis had been disabled would be worthless.
#[test]
fn the_budget_still_rejects_an_over_deep_chain() {
    let src = "@budget(stack_bytes = 8)\n\
               fn a(n: Int) -> Int { let x = n; return b(x); }\n\
               fn b(n: Int) -> Int { let y = n; return y; }\n\
               fn main() { println(a(1)); }";
    let program = hale_syntax::parse_source(src).expect("parse");
    let ds: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("stack_bytes")),
        "an 8-byte budget over a two-frame chain must be rejected — \
         if this stops firing the premise tests above are guarding \
         nothing: {:?}",
        ds
    );
}

/// Negative control for the detector itself.
///
/// The three tests above pass by finding NO large alloca, so they are
/// only as good as `alloca_extent`. The first version of it matched
/// digits anywhere in the line and read `%req96` as a 96-byte buffer —
/// a detector that cried wolf. Overcorrecting to one that never fires
/// would be worse: every premise test would pass forever and guard
/// nothing.
#[test]
fn the_detector_recognises_a_stack_buffer() {
    assert_eq!(
        alloca_extent("  %buf = alloca [4096 x i64], align 8"),
        4096,
        "the `[N x T]` shape must be measured"
    );
    assert_eq!(
        alloca_extent("  %b = alloca i8, i64 65536, align 16"),
        65536,
        "the explicit-count shape must be measured"
    );
    // ...and must NOT fire on an ordinary scalar slot whose SSA name
    // or alignment happens to contain a large number.
    assert_eq!(
        alloca_extent("  %req96 = alloca { i64, i64 }, align 8"),
        0,
        "an SSA name is not a size"
    );
    assert_eq!(
        alloca_extent("  %x = alloca i64, align 8"),
        0,
        "a scalar slot is not a buffer"
    );
    assert!(
        !big_allocas("  %buf = alloca [4096 x i64], align 8", 1024)
            .is_empty(),
        "big_allocas must report a genuine 4096-element buffer"
    );
}
