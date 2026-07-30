//! iris handoff P1 (2026-07-27) — `@form(vec).set` replaced-element
//! retire.
//!
//! Pointer-storage vec elements each own an arena block; `.set`
//! used to orphan the REPLACED element in the form owner's
//! program-lifetime arena: ~33B leaked per set, and the growing
//! arena made the deep-copy containment chunk-walk progressively
//! slower (the reported ~1µs/set, ~1000× `.get`). The fix retires
//! the old block (+ its non-surviving String fields, hashmap
//! retire-cell discipline) straight onto the reuse freelist, and
//! the deep-copy alloc consults that freelist — steady-state sets
//! ping-pong between reused blocks.
//!
//! The assertion is an IN-PROGRAM RSS budget via
//! `std::process::rss_bytes()`: after warmup, 2M churn sets may
//! not grow peak RSS by more than 16MB (pre-fix growth at this
//! scale: ~65MB and climbing linearly). Generous margin keeps CI
//! load out of the verdict; the ASan corpus job covers the
//! double-retire/UAF side.

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_vecretire_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn set_churn_is_flat_and_fast() {
    let src = r#"
        type Ent { seq: Int = 0; ts: Int = 0; seg: Int = -1; kind: Int = 0; }

        @form(vec)
        locus EntVec { capacity { heap items of Ent; } }

        locus Setter {
            params { v: EntVec; round: Int = 0; }
            birth() {
                let mut i = 0;
                while i < 100000 {
                    let e = self.v.get(i % 1024) or Ent { };
                    self.v.set(i % 1024, Ent {
                        seq: e.seq + 1, ts: self.round, seg: 0, kind: 1,
                    }) or discard;
                    i = i + 1;
                }
            }
        }

        main locus App {
            params { v: EntVec = EntVec { }; }
            birth() {
                let mut i = 0;
                while i < 1024 { self.v.push(Ent { }); i = i + 1; }
            }
            run() {
                // Warmup round, then measure.
                Setter { v: self.v, round: 0 };
                let base = std::process::rss_bytes();
                let mut n = 1;
                while n < 20 {
                    Setter { v: self.v, round: n };
                    n = n + 1;
                }
                let grown = std::process::rss_bytes() - base;
                if grown > 16777216 {
                    println("LEAK: rss grew ", grown, " bytes over 1.9M sets");
                    std::process::exit(1);
                }
                let probe = self.v.get(7) or Ent { };
                if probe.seq != 1960 {
                    println("BAD seq: ", probe.seq);
                    std::process::exit(1);
                }
                println("flat, seq ok");
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("churn", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("flat, seq ok"),
        "stdout: {:?}\nstderr: {:?}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn string_fields_retire_with_survivor_guard() {
    // Replaced elements' String fields retire; a field that
    // SURVIVES into the new element (tag aliasing) must not be
    // retired out from under it. Correctness assertions +
    // steady-state churn — corruption from a bad retire shows up
    // as a wrong tag or an ASan hit in the sanitizer job.
    let src = r#"
        type Row { name: String = ""; tag: String = ""; n: Int = 0; }

        @form(vec)
        locus Rows { capacity { heap items of Row; } }

        locus Churner {
            params { v: Rows; }
            birth() {
                let mut i = 0;
                while i < 200000 {
                    let idx = i % 64;
                    let old = self.v.get(idx) or Row { };
                    self.v.set(idx, Row {
                        name: "fresh",
                        tag: old.tag,
                        n: old.n + 1,
                    }) or discard;
                    i = i + 1;
                }
            }
        }

        main locus App {
            params { v: Rows = Rows { }; }
            birth() {
                let mut i = 0;
                while i < 64 {
                    self.v.push(Row { name: "seed", tag: "keep-me", n: 0 });
                    i = i + 1;
                }
            }
            run() {
                Churner { v: self.v };
                let probe = self.v.get(3) or Row { };
                if probe.tag != "keep-me" {
                    println("SURVIVOR LOST: ", probe.tag);
                    std::process::exit(1);
                }
                if probe.n != 3125 {
                    println("BAD n: ", probe.n);
                    std::process::exit(1);
                }
                println("survivor intact");
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("strings", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("survivor intact"),
        "stdout: {:?}\nstderr: {:?}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}
