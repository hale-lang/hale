//! GH #2132 (F.64) with GH #2134 (F.63): the fixed-width integers' point
//! is lanes. A `U32` kernel written with `wrapping_*` is plain `i32`
//! arithmetic, which the loop vectorizer widens: four lanes a 128-bit
//! vector (NEON on aarch64, SSE2 on x86-64), so `@vectorize(min_width: 4)`
//! holds on both, where aarch64 has no `i64` vector multiply and the
//! same kernel over `Int` is interleaved scalar code there (F.63's
//! spike). A checked sum over a `bounded` (GH #2162, F.65) accumulates in
//! an `i64` and is checked once, after the loop, so it has no exit per
//! element and vectorizes too: `.map(it * 3).sum(U32(0))`, its `map`
//! computed wide, and an `I16` `for` sum. A checked sum this lowering does
//! not take (a step of two operators, a product) keeps its overflow branch
//! per element to the process-stopping violation, a side exit no
//! vectorizer takes, so `@vectorize` refuses it at the loop on every
//! target.

use hale_codegen::{BuildOptions, CodegenError};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build(src: &str, tag: &str) -> Result<(), CodegenError> {
    let options: BuildOptions = build_opts::options();
    let Ok(snap) = build_opts::load_seed(src, build_opts::harness_config(&options)) else { panic!("load") };
    let view = snap.demand_lowering().map_err(|b| CodegenError::Unsupported(format!("{b:?}")))?;
    let bin = harness::unique_bin(tag);
    let out = hale_codegen::build_resolved(view, &bin, &options);
    let _ = std::fs::remove_file(&bin);
    out
}

const MAIN: &str = "
fn main() {
    let mut xs: bounded[U32; 1024] = [];
    let mut ys: bounded[Int; 1024] = [];
    let mut i = 0;
    while i < 1000 {
        push(xs, U32(i) or wrap) or discard;
        push(ys, i) or discard;
        i = i + 1;
    }
    println(kernel(xs), \" \", wide(ys));
}
";

const WRAPPING: &str = "@vectorize(min_width: 4)
fn kernel(xs: bounded[U32; 1024]) -> U32 {
    let mut s: U32 = 0;
    for x in xs {
        s = U32::wrapping_add(s, U32::wrapping_mul(x, 3));
    }
    return s;
}

fn wide(ys: bounded[Int; 1024]) -> Int {
    return ys.map(it * 3).sum(0);
}
";

#[test]
fn a_u32_wrapping_kernel_vectorizes_four_lanes_wide() {
    build(&format!("{WRAPPING}{MAIN}"), "fixed_vec_u32").expect("four i32 lanes on x86-64 and aarch64");
}

/// The same kernel over `Int`, on aarch64: no `i64` vector multiply, so
/// LLVM interleaves it and `@vectorize` refuses it, where the `U32` one
/// above holds.
#[test]
#[cfg(target_arch = "aarch64")]
fn the_int_kernel_is_scalar_on_aarch64() {
    let src = format!("{WRAPPING}{MAIN}").replacen("fn wide(", "@vectorize\nfn wide(", 1);
    match build(&src, "fixed_vec_int") {
        Err(CodegenError::NotVectorized(msg, _, false)) => {
            assert!(msg.contains("`@vectorize` on `wide`"), "{msg}");
            assert!(!msg.contains("`kernel`"), "the U32 kernel holds: {msg}");
        }
        other => panic!("expected the Int kernel refused, got {other:?}"),
    }
}

/// A checked narrow sum over a `bounded` accumulates wide and is checked
/// at the loop's exit (F.65): its loop has no side exit and vectorizes,
/// the `map` step computed wide too, as does an `I16` sum written as a
/// `for` with `+=`. Plain `@vectorize`: an `i64` accumulator is two lanes
/// on a baseline x86-64 (SSE2) and four with AVX2, sixteen and eight on
/// aarch64.
#[test]
fn a_checked_narrow_sum_vectorizes() {
    let src = format!(
        "@vectorize\nfn checked(xs: bounded[U32; 1024]) -> U32 {{\n    return xs.map(it * 3).sum(U32(0));\n}}\n\
         @vectorize\nfn halves(hs: bounded[I16; 1024]) -> I16 {{\n    let mut s: I16 = 0;\n    for h in hs {{\n        s += h;\n    }}\n    return s;\n}}\n\
         {WRAPPING}{}",
        MAIN.replace("println(kernel(xs)", "let mut hs: bounded[I16; 1024] = [];\n    push(hs, 7) or discard;\n    println(checked(xs), halves(hs), kernel(xs)")
    );
    build(&src, "fixed_vec_checked").expect("the checked U32 and I16 sums vectorize");
}

/// What the wide sum does not take keeps its overflow branch per element
/// and is refused at its loop on every target: a step of two operators
/// (whose inner one is checked where it happens) and a product.
#[test]
fn a_checked_narrow_sum_it_does_not_take_does_not_vectorize() {
    let src = format!(
        "@vectorize\nfn two_ops(xs: bounded[U32; 1024]) -> U32 {{\n    return xs.map(it * 3 + 1).sum(U32(0));\n}}\n\
         @vectorize\nfn product(xs: bounded[U32; 1024]) -> U32 {{\n    let mut p: U32 = 1;\n    for x in xs {{\n        p = p * x;\n    }}\n    return p;\n}}\n\
         {WRAPPING}{}",
        MAIN.replace("println(kernel(xs)", "println(two_ops(xs), product(xs), kernel(xs)")
    );
    match build(&src, "fixed_vec_untaken") {
        Err(CodegenError::NotVectorized(msg, _, false)) => {
            assert!(msg.starts_with("`@vectorize` on `two_ops`: this loop does not vectorize on "), "{msg}");
            assert!(msg.contains("in `product`"), "{msg}");
        }
        other => panic!("expected both refused, got {other:?}"),
    }
}
