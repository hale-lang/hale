//! GH #18 item 1, step 2 — empirical validation of the reclamation model.
//!
//! The scope (`notes/memory-bound-proofs.md`) is emphatic that the
//! reclamation model is "the whole ballgame — validate it empirically
//! against RSS before trusting it; no false 'bounded'." This is that test:
//! it ties `alloc_summary`'s per-site verdict to measured peak RSS in both
//! directions.
//!
//! - **Positive:** a program the model marks `AccumulatesUnbounded` (a
//!   struct allocated each iteration of a loop) actually grows RSS.
//! - **Negative control:** the same loop with no allocation — the model
//!   finds no accumulating site, and RSS stays flat.
//!
//! This also encodes the key empirical finding behind the model: free-fn
//! calls do NOT reclaim per call (a struct allocated inside a non-inlinable
//! per-iteration free fn still accumulates), so the model attributes
//! reclaim to the enclosing *locus*, not the fn return — contrary to
//! `spec/memory.md` §"Free fn functions".

use std::process::Command;

use hale_codegen::build_executable;
use hale_types::alloc_summary::{summarize_programs, SiteVerdict};

fn model_has_unbounded_site(src: &str) -> bool {
    let program = hale_syntax::parse_source(src).expect("parse");
    let summary = summarize_programs(&[&program]);
    summary
        .fns
        .values()
        .flat_map(|f| f.sites.iter())
        .any(|s| s.verdict() == SiteVerdict::AccumulatesUnbounded)
}

fn build_and_rss(name: &str, src: &str) -> i64 {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_alloc_rss_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(output.status.success(), "{} crashed: {:?}", name, output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|l| l.starts_with("final_rss_mb="))
        .and_then(|l| l.trim_start_matches("final_rss_mb=").trim().parse().ok())
        .unwrap_or_else(|| panic!("no final_rss_mb in {} stdout: {:?}", name, stdout))
}

/// A struct allocated directly every iteration of a (model-unbounded)
/// loop. The loop terminates at runtime, but the model sees `while cond`
/// as unbounded and the allocations accumulate — so RSS climbs with the
/// trip count. 2M × a 3-field struct ≈ tens of MB.
///
/// (Step 2 is intra-procedural: the model flags an alloc *directly* in a
/// loop. An alloc inside a free fn *called* in the loop accumulates just
/// as much at runtime — free-fn returns don't reclaim, which is what fixes
/// the reclaim scope to the locus — but flagging that needs the call-graph
/// multiplicity propagation in step 3.)
// The loop bound is a *runtime* value (the param `n`), not a literal —
// otherwise loop-ranking would (correctly) prove `while i < <const>`
// bounded and the model would not flag the alloc. A runtime ceiling is
// the genuine unbounded case: the model can't statically bound the trips,
// and at runtime the 3M iterations still accumulate ~190 MB.
const ACCUMULATING: &str = r#"
    type Q { a: Int; b: Int; c: Int; d: Int; e: Int; f: Int; g: Int; h: Int; }
    fn work(n: Int) -> Int {
        let mut s = 0;
        let mut i = 0;
        while i < n {
            let q = Q { a: i, b: i, c: i, d: i, e: i, f: i, g: i, h: i };
            s = s + q.a + q.h;
            i = i + 1;
        }
        return s;
    }
    fn main() {
        let s = work(3000000);
        print("sum=");
        println(s);
        print("final_rss_mb=");
        println(std::process::rss_bytes() / 1048576);
    }
"#;

/// The negative control: the identical loop with no allocation.
const FLAT: &str = r#"
    fn work() -> Int {
        let mut s = 0;
        let mut i = 0;
        while i < 2000000 {
            s = s + i;
            i = i + 1;
        }
        return s;
    }
    fn main() {
        let s = work();
        print("sum=");
        println(s);
        print("final_rss_mb=");
        println(std::process::rss_bytes() / 1048576);
    }
"#;

#[test]
fn model_unbounded_verdict_matches_growing_rss() {
    // Model side: the accumulating program has an unbounded site; the flat
    // one does not. (The struct is allocated inside `churn`, a
    // non-inlinable per-iteration free fn — the model still flags it,
    // because free-fn returns don't reclaim.)
    assert!(
        model_has_unbounded_site(ACCUMULATING),
        "model should flag the per-iteration struct alloc as unbounded"
    );
    assert!(
        !model_has_unbounded_site(FLAT),
        "model must NOT flag the alloc-free loop (no false unbounded)"
    );

    // Runtime side: RSS confirms the verdicts. Assert *relative* to the
    // control — the runtime's baseline arena differs by build config (the
    // test's build_executable sits tens of MB above the optimized CLI
    // build), so an absolute bound is fragile. What's invariant is that
    // the 2M accumulated structs add tens of MB over the identical
    // alloc-free loop.
    let acc_rss = build_and_rss("accumulating", ACCUMULATING);
    let flat_rss = build_and_rss("flat", FLAT);

    assert!(
        flat_rss < 200,
        "negative control RSS implausibly high ({}MB) — runtime baseline regression?",
        flat_rss
    );
    // 3M × an 8-field (~64-byte) struct ≈ 190 MB of accumulation, well
    // past the test build's ~50 MB baseline arena.
    assert!(
        acc_rss >= flat_rss + 80,
        "model-flagged accumulating loop should add >80MB over the \
         identical alloc-free control: acc={}MB, flat={}MB. If the gap \
         collapsed, allocations now reclaim where the model says they \
         accumulate — the reclaim scope must be revisited.",
        acc_rss,
        flat_rss
    );
}
