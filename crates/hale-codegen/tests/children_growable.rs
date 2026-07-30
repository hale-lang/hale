//! 2026-05-29 — growable accept'd-children buffer.
//!
//! The old `__children[16]` fixed inline array had no bounds
//! check at accept-time (`__children[count++] = child`), so a
//! parent that declared `accept` AND iterated `self.children`
//! silently corrupted adjacent struct memory once it accepted
//! more than 16 children — the daemon-server pattern that
//! accepts one child per connection. The storage is now a heap
//! `void**` buffer grown by `lotus_children_push`, tracked by
//! `__children` (ptr) + `__child_count` + `__child_cap` i64
//! fields. These tests accept far more than 16 children and
//! verify the iteration sees every one with the right value.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_children_growable_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn run(bin: &std::path::PathBuf) -> String {
    let out = Command::new(bin).output().expect("run hale");
    let _ = std::fs::remove_file(bin);
    assert!(
        out.status.success(),
        "non-zero exit: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn accepts_far_more_than_sixteen_children() {
    // 50 leaves accepted in a loop, then iterated via two modes.
    // Pre-fix the 17th accept wrote past `__children[16]` into
    // adjacent struct fields, corrupting `__child_count` / the
    // mode reads (or crashing). Expect count=50, bulk=sum(0..49).
    let src = r#"
        locus LeafL {
            params { value: Int = 0; }
            contract { expose value: Int; }
        }
        locus AggregatorL {
            contract { consume value: Int; }
            accept(c: LeafL) { }
            mode bulk() -> Int {
                let mut total: Int = 0;
                for child in self.children { total = total + child.value; }
                return total;
            }
            mode harmonic() -> Int {
                let mut n: Int = 0;
                for child in self.children { n = n + 1; }
                return n;
            }
            run() {
                let mut i: Int = 0;
                while i < 50 {
                    LeafL { value: i };
                    i = i + 1;
                }
                println("count=", self.harmonic());
                println("bulk=", self.bulk());
            }
        }
        fn main() { AggregatorL { }; }
    "#;
    let bin = build("fifty", src);
    let stdout = run(&bin);
    assert!(
        stdout.contains("count=50"),
        "expected all 50 children tracked; got: {:?}",
        stdout
    );
    // sum(0..=49) = 1225 — every value read intact, no corruption.
    assert!(
        stdout.contains("bulk=1225"),
        "expected bulk=1225 (sum 0..49); got: {:?}",
        stdout
    );
}

#[test]
fn growth_spans_several_doublings() {
    // 300 children forces the heap buffer through several
    // realloc doublings (8 → 16 → ... → 512). Verifies the
    // grow-and-copy path preserves every prior slot.
    let src = r#"
        locus LeafL {
            params { value: Int = 0; }
            contract { expose value: Int; }
        }
        locus AggregatorL {
            contract { consume value: Int; }
            accept(c: LeafL) { }
            mode harmonic() -> Int {
                let mut n: Int = 0;
                for child in self.children { n = n + 1; }
                return n;
            }
            mode bulk() -> Int {
                let mut total: Int = 0;
                for child in self.children { total = total + child.value; }
                return total;
            }
            run() {
                let mut i: Int = 0;
                while i < 300 {
                    LeafL { value: 1 };
                    i = i + 1;
                }
                println("count=", self.harmonic());
                println("sum=", self.bulk());
            }
        }
        fn main() { AggregatorL { }; }
    "#;
    let bin = build("threehundred", src);
    let stdout = run(&bin);
    assert!(
        stdout.contains("count=300") && stdout.contains("sum=300"),
        "expected 300 children each value 1 after several doublings; got: {:?}",
        stdout
    );
}
