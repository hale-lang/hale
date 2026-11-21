//! GH #2134 (F.63): `@vectorize` is held to what LLVM's loop vectorizer
//! did with each innermost loop of the fn, on the build's target.
//!
//! The kernels here are chosen to agree on x86-64 (SSE2 baseline and
//! any newer CPU) and aarch64 (NEON): a 64-bit integer sum vectorizes on
//! both, a loop whose trip count depends on what it computes vectorizes
//! on neither, and no 64-bit vector is wider than 64 lanes. A `Float`
//! sum is the target-dependent case (aarch64 keeps the order in an
//! in-order vector reduction, x86-64 has none), so its test reads the
//! verdict the target gives rather than fixing one.
//!
//! Each verdict is judged twice: in a build without DWARF (the harness's,
//! `hale run`'s: the loops carry synthetic locations, stripped after the
//! pipeline) and in one that emits it (`hale build`'s: the loops' real
//! file, line and column).

use hale_codegen::{BuildOptions, CodegenError, DebugSourceFile, DebugSources};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Build `src` with `options`; with `dwarf`, as `hale build` does, the
/// seed's file in the debug table.
fn build(src: &str, tag: &str, dwarf: bool, mut options: BuildOptions) -> Result<(), CodegenError> {
    let Ok(snap) = build_opts::load_seed(src, build_opts::harness_config(&options)) else { panic!("load") };
    if dwarf {
        options.debug = Some(DebugSources {
            files: snap
                .file_bases()
                .iter()
                .filter_map(|(base, path, len)| {
                    snap.sources().get(path).map(|text| DebugSourceFile {
                        base: *base,
                        len: *len,
                        path: path.clone(),
                        text: text.clone(),
                    })
                })
                .collect(),
        });
    }
    let view = snap.demand_lowering().map_err(|b| CodegenError::Unsupported(format!("{b:?}")))?;
    let bin = harness::unique_bin(tag);
    let out = hale_codegen::build_resolved(view, &bin, &options);
    let _ = std::fs::remove_file(&bin);
    out
}

fn both(src: &str, tag: &str) -> [Result<(), CodegenError>; 2] {
    [build(src, tag, false, build_opts::options()), build(src, tag, true, build_opts::options())]
}

/// The refusal: its message and the (line, column) of its span.
fn refused(r: Result<(), CodegenError>, src: &str) -> (String, (usize, usize)) {
    match r {
        Err(CodegenError::NotVectorized(msg, span, false)) => (msg, span.line_col(src)),
        other => panic!("expected a NotVectorized refusal, got {other:?}"),
    }
}

const SUM: &str = "@vectorize
fn total(xs: bounded[Int; 1024]) -> Int {
    let mut s = 0;
    for x in xs {
        s = s + x;
    }
    return s;
}

locus Acc {
    params { k: Int = 0; }
    @vectorize
    fn chained(xs: bounded[Int; 1024]) -> Int {
        return xs.map(it + 1).sum(0);
    }
}

fn main() {
    let mut xs: bounded[Int; 1024] = [];
    let mut i = 0;
    while i < 1000 {
        push(xs, i) or discard;
        i = i + 1;
    }
    let a = Acc { };
    println(total(xs) + a.chained(xs));
}
";

#[test]
fn an_integer_sum_and_an_element_chain_vectorize() {
    for r in both(SUM, "vec_sum") {
        r.expect("both loops vectorize on x86-64 and aarch64");
    }
}

/// The loop runs until what it computes reaches a bound: its trip count
/// is not known when it starts, which the check cannot see and no
/// target's vectorizer takes.
const UNTIL: &str = "@vectorize
fn until(xs: [Int; 1024], limit: Int) -> Int {
    let mut s = 0;
    let mut i = 0;
    while s < limit {
        s = s + xs[i % 1024];
        i = i + 1;
    }
    return i;
}

fn main() {
    let xs: [Int; 1024] = [3; 1024];
    println(until(xs, 5000));
}
";

#[test]
fn a_loop_whose_trip_count_it_computes_is_refused_at_the_loop() {
    for r in both(UNTIL, "vec_until") {
        let (msg, at) = refused(r, UNTIL);
        assert_eq!(at, (5, 5), "at the `while`: {msg}");
        assert!(msg.starts_with("`@vectorize` on `until`: this loop does not vectorize on "), "{msg}");
        assert!(msg.contains("its trip count is not known when the loop starts"), "{msg}");
        assert!(msg.contains("(LLVM: could not determine number of loop iterations)"), "{msg}");
    }
}

#[test]
fn a_vector_narrower_than_min_width_is_refused() {
    let src = SUM.replacen("@vectorize\nfn total", "@vectorize(min_width: 64)\nfn total", 1);
    for r in both(&src, "vec_width") {
        let (msg, at) = refused(r, &src);
        assert_eq!(at, (4, 5), "at the `for`: {msg}");
        assert!(msg.contains("lanes wide, narrower than `min_width: 64`"), "{msg}");
    }
}

/// Every failing loop is named, the first at the refusal's span.
#[test]
fn each_loop_that_does_not_vectorize_is_named() {
    let src = UNTIL.replace(
        "fn main() {",
        "@vectorize
fn again(xs: [Int; 1024], limit: Int) -> Int {
    let mut s = 0;
    let mut i = 0;
    while s < limit {
        s = s + xs[i % 1024];
        i = i + 2;
    }
    return i;
}

fn main() {
    println(again([4; 1024], 7000));",
    );
    let (msg, _) = refused(build(&src, "vec_two", true, build_opts::options()), &src);
    assert!(msg.contains("\n  and the loop"), "{msg}");
    assert!(msg.contains("`again`") && msg.contains("`until`"), "{msg}");
}

/// A `Float` sum keeps its written order (Hale does not reassociate
/// floating-point addition), so whether it vectorizes is the target's
/// answer: an in-order vector reduction (aarch64) or none (x86-64).
#[test]
fn a_float_sum_is_the_targets_answer() {
    let src = "@vectorize
fn ftotal(xs: bounded[Float; 1024]) -> Float {
    let mut s = 0.0;
    for x in xs {
        s = s + x;
    }
    return s;
}

fn main() {
    let mut xs: bounded[Float; 1024] = [];
    let mut i = 0;
    while i < 1000 {
        push(xs, 1.5) or discard;
        i = i + 1;
    }
    println(ftotal(xs));
}
";
    let r = build(src, "vec_float", true, build_opts::options());
    if cfg!(target_arch = "aarch64") {
        r.expect("aarch64 keeps the order in an in-order vector reduction");
    } else {
        let (msg, _) = refused(r, src);
        assert!(msg.contains("a `Float` sum or product"), "{msg}");
    }
}

/// A dev build optimizes at O1, which does not vectorize: the assertion
/// is not checked there (the CLI says so), and the program builds.
#[test]
fn a_dev_build_does_not_check() {
    let mut options = build_opts::options();
    options.dev_profile = true;
    build(UNTIL, "vec_dev", false, options).expect("not checked at O1");
}

/// `fn main` is lowered apart from every other fn; its loops are held to
/// its `@vectorize` all the same (review of #2144).
#[test]
fn fn_mains_own_loops_are_held() {
    let src = "@vectorize(min_width: 64)
fn main() {
    let xs: [Int; 1024] = [std::process::pid(); 1024];
    let mut s = 0;
    for i in 0..1024 {
        s = s + xs[i] * i;
    }
    println(s);
}
";
    for r in both(src, "vec_main") {
        let (msg, at) = refused(r, src);
        assert!(msg.starts_with("`@vectorize` on `main`: "), "{msg}");
        assert_eq!(at, (5, 5), "at main's loop: {msg}");
    }
}
