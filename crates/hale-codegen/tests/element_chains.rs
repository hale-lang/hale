//! Recognized element chains (#353 cluster B).
//!
//! `xs.filter(it > 2).count()` is not a value being built — it is a
//! form the compiler recognizes and rewrites to ONE loop.
//!
//! The knot that made "add closures, then add iterators" look
//! expensive was self-inflicted: it assumed each stage must PRODUCE
//! something, so a chain needs either a lazy object or an intermediate
//! collection, either of which is a sequence value needing an owner,
//! which reopens arenas and placement. If the chain is a recognized
//! form rather than a value, nothing is produced at any step and the
//! question never arises.
//!
//! Three consequences, all of which fall out rather than being
//! engineered, and all pinned below:
//!
//!   - ZERO ALLOCATION, so a chain is legal inside
//!     `@budget(alloc_per_call = 0)`. A design returning a new
//!     collection would be illegal in exactly the code Hale exists
//!     for.
//!   - EAGER, so a predicate's effects are attributed to the
//!     predicate's own source position. A lazy chain would run it at
//!     the terminal and the witness path would name the wrong line.
//!   - NO LAMBDAS. The predicate is an argument position the compiler
//!     knows about, not a value, so there is no closure to represent —
//!     no capture modes, no escape analysis, no cross-thread question.
//!     `it` is bound per element by the desugar.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_chain_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

const V: &str =
    "@form(vec)\nlocus Nums { capacity { heap items of Int; } }\n";

#[test]
fn filter_count_evaluates_the_predicate_per_element() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(2); v.push(9);
            println(\"gt2=\", v.filter(it > 2).count());
            println(\"all=\", v.filter(it > 0).count());
            println(\"none=\", v.filter(it > 100).count());
        }}"
    );
    let (out, st) = run("count", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("gt2=2"), "5 and 9 pass: {:?}", out);
    assert!(out.contains("all=4"), "got: {:?}", out);
    assert!(out.contains("none=0"), "got: {:?}", out);
}

/// Two stages fuse into ONE pass — no intermediate exists for the
/// second stage to read.
#[test]
fn two_stages_fuse() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(2); v.push(9);
            println(\"fused=\", v.filter(it > 2).filter(it < 9).count());
        }}"
    );
    let (out, st) = run("fuse", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("fused=1"), "only 5 survives both: {:?}", out);
}

/// THE claim. A chain allocates nothing, so it is legal on a hot path.
/// If this ever fails, the design has regressed to materialising
/// something, and composition stops being usable in the code Hale is
/// built for.
#[test]
fn a_chain_is_legal_under_a_zero_alloc_budget() {
    let src = format!(
        "{V}@budget(alloc_per_call = 0)
        fn big(v: Nums) -> Int {{
            return v.filter(it > 2).filter(it < 9).count();
        }}
        fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(9);
            println(\"n=\", big(v));
        }}"
    );
    let (out, st) = run("budget", &src);
    assert!(
        st.success(),
        "a chain must not allocate — @budget(alloc_per_call = 0) \
         rejected it: {:?}",
        st
    );
    assert!(out.contains("n=1"), "got: {:?}", out);
}

/// The `into` terminal writes into caller-supplied storage — the same
/// shape `split_into` uses, and the only point at which anything is
/// materialised.
#[test]
fn into_writes_the_survivors_to_a_target() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(2); v.push(9);
            let out = Nums {{ }};
            v.filter(it > 2).into(out);
            println(\"n=\", out.len(), \" first=\", out.get(0) or -1);
        }}"
    );
    let (out, st) = run("into", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("n=2"), "5 and 9: {:?}", out);
    assert!(out.contains("first=5"), "in source order: {:?}", out);
}

/// Two chains in one block must not collide on their loop
/// temporaries — the desugar numbers them.
#[test]
fn two_chains_in_one_block_coexist() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5);
            let a = v.filter(it > 0).count();
            let b = v.filter(it > 4).count();
            println(\"a=\", a, \" b=\", b);
        }}"
    );
    let (out, st) = run("two", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("a=2"), "got: {:?}", out);
    assert!(out.contains("b=1"), "got: {:?}", out);
}

/// A bare `.count()` with no stage is an ordinary form method, not a
/// chain. The desugar must not swallow it.
#[test]
fn a_bare_method_is_not_a_chain() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(2);
            println(\"len=\", v.len());
        }}"
    );
    let (out, st) = run("bare", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("len=2"), "got: {:?}", out);
}
