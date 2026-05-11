//! 3f repro — `stale-locus-method-result-on-second-call`.
//!
//! Returning a locus from a method and calling further
//! methods on the returned handle from a *different* calling
//! locus. The friction note (2026-05-11) says the second
//! method call returns stale/zeroed state. m49 (5f9803d) and
//! m51/m53 (66bd912/1e7d06e) tightened return-deep-copy and
//! free-fn handle-rooting; this test pins whether the bug
//! still reproduces.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_cross_locus_return_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn second_method_call_on_returned_locus_stays_consistent() {
    // Shape from apps/reload-demo/fitter.ap (pre-workaround):
    //   let s = market.fit();      // MarketL.fit() -> SegmentL
    //   println("c1=", s.count()); // from FitterL.run
    //   println("c2=", s.count()); // 3f said this returned 0
    let src = r#"
        locus SegmentL {
            params { count_v: Int = 0; slope_v: Int = 0; }
            fn count() -> Int { return self.count_v; }
            fn slope() -> Int { return self.slope_v; }
        }

        locus MarketL {
            params { unused: Int = 0; }
            fn fit() -> SegmentL {
                return SegmentL { count_v: 5, slope_v: 7 };
            }
        }

        locus FitterL {
            params { unused: Int = 0; }
            fn step(market: MarketL) {
                let s = market.fit();
                println("c1=", s.count());
                println("sl=", s.slope());
                println("c2=", s.count());
                if s.count() >= 2 {
                    println("branch=taken");
                } else {
                    println("branch=skipped");
                }
            }
        }

        fn main() {
            let m = MarketL { unused: 0 };
            let f = FitterL { unused: 0 };
            f.step(m);
        }
    "#;
    let bin = build_aperio("repeat_calls", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Each line should agree on the SegmentL state.
    assert!(stdout.contains("c1=5"), "c1 wrong; got: {:?}", stdout);
    assert!(stdout.contains("sl=7"), "sl wrong; got: {:?}", stdout);
    assert!(
        stdout.contains("c2=5"),
        "c2 returned stale state — 3f reproduces; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("branch=taken"),
        "if-cond saw stale c2 — 3f reproduces; got: {:?}",
        stdout
    );
}

#[test]
fn let_bound_locus_returned_through_method_stays_consistent() {
    // The reload-demo shape: build a locus via `let s = X{}`,
    // mutate it inline, then `return s`. The let-binding goes
    // through the deferred-dissolve frame; without m90's
    // return-locus heap-alloc, the fn-exit flush would
    // dissolve+destroy s before its self_ptr escapes.
    let src = r#"
        locus SegL {
            params { n: Int = 0; }
            fn push() { self.n = self.n + 1; }
            fn count() -> Int { return self.n; }
        }

        locus FactoryL {
            params { unused: Int = 0; }
            fn build(times: Int) -> SegL {
                let s = SegL { n: 0 };
                let mut i = 0;
                while i < times {
                    s.push();
                    i = i + 1;
                }
                return s;
            }
        }

        fn main() {
            let f = FactoryL { unused: 0 };
            let s = f.build(4);
            println("c1=", s.count());
            println("c2=", s.count());
            println("c3=", s.count());
        }
    "#;
    let bin = build_aperio("let_bound_return", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("c1=4"), "c1 wrong; got: {:?}", stdout);
    assert!(stdout.contains("c2=4"), "c2 wrong; got: {:?}", stdout);
    assert!(stdout.contains("c3=4"), "c3 wrong; got: {:?}", stdout);
}
