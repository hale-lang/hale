//! F.32-1b (2026-05-25) — locus struct field reordering by
//! access frequency. Black-box behavioral coverage:
//!
//!   1. `reorder_preserves_field_value_reads` — a locus with
//!      multiple user fields read with varying frequency. After
//!      reorder, every field still returns its correctly-
//!      initialized default value (verifies the `fields` lookup
//!      map's indices got remapped consistent with the LLVM
//!      struct's permutation).
//!
//!   2. `reorder_preserves_field_overrides` — instantiation
//!      with field-name overrides (`Counter { hot: 999 }`) must
//!      land the value in the right cell regardless of struct
//!      position.
//!
//!   3. `reorder_preserves_field_writes` — write back to a
//!      field, then read; must round-trip.
//!
//! The reorder is an optimization, not a contract — these
//! tests pin down that the optimization doesn't break the
//! field-name → value contract. Direct verification of "hot
//! field ended up at index 0" would require LLVM IR dump
//! parsing; out of scope for v1.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-locus-field-reorder-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(tag: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run binary");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    (stdout, out.status)
}

#[test]
fn reorder_preserves_field_value_reads() {
    // `hot` accessed many times → expected to move to front.
    // `cold` accessed 0× → expected to move to back.
    // Defaults must still be read correctly post-reorder.
    let src = r#"
        locus Counter {
            params {
                cold:    Int = 100;
                hot:     Int = 7;
                warm:    Int = 50;
                chilled: Int = 25;
            }
            run() {
                let _ = self.hot;
                let _ = self.hot;
                let _ = self.hot;
                let _ = self.hot;
                let _ = self.warm;
                print("hot=");     println(self.hot);
                print("warm=");    println(self.warm);
                print("chilled="); println(self.chilled);
                print("cold=");    println(self.cold);
            }
        }
        fn main() { Counter { }; }
    "#;
    let (stdout, status) = build_and_run("reads", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("hot=7"), "got: {}", stdout);
    assert!(stdout.contains("warm=50"), "got: {}", stdout);
    assert!(stdout.contains("chilled=25"), "got: {}", stdout);
    assert!(stdout.contains("cold=100"), "got: {}", stdout);
}

#[test]
fn reorder_preserves_field_overrides() {
    // Field-name overrides at instantiation must land in the
    // right cell regardless of struct field position.
    let src = r#"
        locus Counter {
            params {
                cold: Int = 0;
                hot:  Int = 0;
                warm: Int = 0;
            }
            run() {
                let _ = self.hot;
                let _ = self.hot;
                print("hot=");  println(self.hot);
                print("warm="); println(self.warm);
                print("cold="); println(self.cold);
            }
        }
        fn main() {
            Counter { hot: 999, cold: 7, warm: 42 };
        }
    "#;
    let (stdout, status) = build_and_run("override", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("hot=999"), "got: {}", stdout);
    assert!(stdout.contains("warm=42"), "got: {}", stdout);
    assert!(stdout.contains("cold=7"), "got: {}", stdout);
}

#[test]
fn reorder_preserves_field_writes() {
    // Write-then-read round-trips through whatever struct
    // position the field ended up at.
    let src = r#"
        locus Box {
            params {
                a: Int = 1;
                b: Int = 2;
                c: Int = 3;
            }
            run() {
                let _ = self.b;
                let _ = self.b;
                let _ = self.b;
                self.a = 100;
                self.b = 200;
                self.c = 300;
                print("a="); println(self.a);
                print("b="); println(self.b);
                print("c="); println(self.c);
            }
        }
        fn main() { Box { }; }
    "#;
    let (stdout, status) = build_and_run("writes", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("a=100"), "got: {}", stdout);
    assert!(stdout.contains("b=200"), "got: {}", stdout);
    assert!(stdout.contains("c=300"), "got: {}", stdout);
}
