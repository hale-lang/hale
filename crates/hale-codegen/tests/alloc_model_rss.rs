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

// === Phase C (2026-06-25): store-latest into a locus field is UNBOUNDED ===
//
// The note had hypothesized a "replace-vs-append refinement" treating a
// whole-value field store (`self.f = X{}`) as *bounded*. RSS falsified it:
// in Hale's bump arena a replaced value is not reclaimed until dissolve, so
// a per-iteration whole-value replace accumulates — even for a fixed-size
// `[Int; 4]`. These tests LOCK that finding so a future "optimization" that
// wrongly suppresses store-latest gets a red test (and so the model's
// flagging of it stays tied to measured RSS). See
// `notes/memory-bound-proofs.md` §"Phase C finding".
//
// All three loops share the runtime bound `self.n` (not a const literal),
// so loop-ranking can't prove them bounded — the genuine unbounded case.

/// In-place indexed write into a fixed array field. No allocation: the slot
/// is overwritten in existing inline storage. The model must NOT flag it,
/// and RSS stays at the runtime floor. This is the *fix* the diagnostic
/// steers toward.
const INPLACE_FIELD_WRITE: &str = r#"
    locus Acc {
        params { recent: [Int; 4] = [0,0,0,0]; n: Int = 3000000; sink: Int = 0; }
        run() {
            let mut i = 0;
            while i < self.n {
                self.recent[i % 4] = i;
                self.sink = self.sink + self.recent[0];
                i = i + 1;
            }
            print("sum="); println(self.sink);
            print("final_rss_mb="); println(std::process::rss_bytes() / 1048576);
        }
    }
    fn main() { Acc { }; }
"#;

/// Whole-value replace of a fixed-size `[Int; 4]` field each iteration.
/// 2026-07-01 inline fixed arrays: scalar `[T; N]` fields are laid out
/// INLINE in the locus struct, so this replace is an in-place element
/// memcpy — the RHS literal is scratch-reclaimed and nothing persists.
/// Historical behavior (pre-inline): the literal bump-allocated a fresh
/// 4-int array into the locus arena every trip (~130 MB over 3M trips).
const ARRAY_FIELD_REPLACE: &str = r#"
    locus Acc {
        params { recent: [Int; 4] = [0,0,0,0]; n: Int = 3000000; sink: Int = 0; }
        run() {
            let mut i = 0;
            while i < self.n {
                self.recent = [i, i, i, i];
                self.sink = self.sink + self.recent[0];
                i = i + 1;
            }
            print("sum="); println(self.sink);
            print("final_rss_mb="); println(std::process::rss_bytes() / 1048576);
        }
    }
    fn main() { Acc { }; }
"#;

#[test]
fn store_latest_field_replace_grows_inplace_stays_flat() {
    // Model side: the whole-value replace is still an unbounded site —
    // 2026-07-01 inline fixed arrays changed the MECHANISM but not the
    // verdict. The store itself is now an in-place element memcpy into
    // the inline field (nothing persists in the locus arena; the walker
    // records the RHS as Local, not StoredToSelf), but the RHS array
    // LITERAL arena-allocates in run()'s method scratch each iteration,
    // and scratch reclaims at method EXIT — one long-running activation
    // accumulates 32 B × trips regardless (measured 96 MB over 3M).
    // The in-place indexed write has no allocation site at all.
    assert!(
        model_has_unbounded_site(ARRAY_FIELD_REPLACE),
        "whole-value `self.recent = [..]` replace must stay flagged: the \
         RHS literal accumulates in the activation's scratch even though \
         the inline-array store itself no longer persists anything"
    );
    assert!(
        !model_has_unbounded_site(INPLACE_FIELD_WRITE),
        "in-place `self.recent[i] = v` writes allocate nothing — no false \
         unbounded"
    );

    // Runtime side: RSS confirms. The replace accumulates ~96 MB of
    // scratch-literal growth over 3M trips (was ~130 MB pre-inline, when
    // the store also leaked a persisted copy per trip into the locus
    // arena); the in-place write sits at the runtime floor (~5 MB).
    let inplace_rss = build_and_rss("inplace_field_write", INPLACE_FIELD_WRITE);
    let replace_rss = build_and_rss("array_field_replace", ARRAY_FIELD_REPLACE);

    // Threshold recalibrated 2026-07-01: pre-inline the gap was ~76 MB
    // (scratch literal + a persisted copy per trip in the locus arena);
    // inline arrays removed the ~35 MB persist component, leaving the
    // scratch-literal growth (~42 MB under the test harness build).
    assert!(
        replace_rss >= inplace_rss + 25,
        "store-latest field replace should add >25MB over the in-place \
         baseline: replace={}MB, inplace={}MB. If the gap collapsed, \
         per-iteration literal allocation now reclaims mid-activation — \
         revisit the model's scratch-accumulation verdict before trusting \
         any store-latest-is-bounded refinement.",
        replace_rss,
        inplace_rss
    );
}

// === Phase D / D2 (2026-06-25): a growing @form(vec) insert accumulates ===
//
// D2 flags `v.push(x)` where `v`'s declared type is a growing `@form(vec |
// hashmap)` locus, in an unbounded context. `lotus_vec_push` grows a
// geometric doubling buffer (cap*2) with the element memcpy'd in, so the
// vec genuinely accumulates with the push count. This ties that verdict to
// measured RSS so a future change that wrongly treats a vec insert as
// bounded gets a red test. (Element is a 32-byte struct for a clean signal
// above the test build's ~50 MB baseline; the loop bound `self.n` is
// runtime, so loop-ranking can't prove it bounded.)

const VEC_PUSH_GROWS: &str = r#"
    type Cell { a: Int; b: Int; c: Int; d: Int; }
    @form(vec) locus CellVec { capacity { heap items of Cell; } }
    locus W {
        params { buf: CellVec = CellVec { }; n: Int = 3000000; }
        run() {
            let mut i = 0;
            while i < self.n {
                self.buf.push(Cell { a: i, b: i, c: i, d: i });
                i = i + 1;
            }
            print("len="); println(self.buf.len());
            print("final_rss_mb="); println(std::process::rss_bytes() / 1048576);
        }
    }
    fn main() { W { }; }
"#;

/// The control: the identical loop and vec, but no push — RSS stays at the
/// runtime floor and the model finds no collection-insert site.
const VEC_PUSH_CONTROL: &str = r#"
    type Cell { a: Int; b: Int; c: Int; d: Int; }
    @form(vec) locus CellVec { capacity { heap items of Cell; } }
    locus W {
        params { buf: CellVec = CellVec { }; n: Int = 3000000; sink: Int = 0; }
        run() {
            let mut i = 0;
            while i < self.n { self.sink = self.sink + i; i = i + 1; }
            print("sum="); println(self.sink);
            print("final_rss_mb="); println(std::process::rss_bytes() / 1048576);
        }
    }
    fn main() { W { }; }
"#;

#[test]
fn collection_insert_growth_matches_rss() {
    // Model side: the vec push is an unbounded collection-insert site; the
    // control (no push) has none.
    assert!(
        model_has_unbounded_site(VEC_PUSH_GROWS),
        "a vec push in an unbounded loop should be flagged unbounded"
    );
    assert!(
        !model_has_unbounded_site(VEC_PUSH_CONTROL),
        "the no-push control must not be flagged"
    );

    // Runtime side: the vec accumulates ~150 MB over 3M pushes of a 32-byte
    // cell; the control sits at the floor. Assert relative to the control.
    let grow_rss = build_and_rss("vec_push_grows", VEC_PUSH_GROWS);
    let ctrl_rss = build_and_rss("vec_push_control", VEC_PUSH_CONTROL);

    assert!(
        grow_rss >= ctrl_rss + 80,
        "a growing @form(vec) insert should add >80MB over the no-push \
         control: grow={}MB, ctrl={}MB. If the gap collapsed, vec pushes no \
         longer accumulate where the model says they do — the D2 \
         collection-insert verdict must be revisited.",
        grow_rss,
        ctrl_rss
    );
}
