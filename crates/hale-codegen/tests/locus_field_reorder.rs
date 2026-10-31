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

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-locus-field-reorder-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(tag: &str, src: &str) -> (String, std::process::ExitStatus) {
    let bin = unique_path(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
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

/// The reorder moves the fields a body touches most to the front of the
/// struct, and a teardown's order is the declaration's whatever the
/// layout (spec/runtime.md § Lifecycle obligations, line 12: an owner's
/// fields drain in their declaration order, and each dissolve completes
/// before the next). `last` is hammered in a loop, so the layout puts it
/// first; the exposure between the two parts is a stdlib locus the
/// lifecycle plan names no order for, which the cascade used to place by
/// layout, after `last`. Read off the emitted cascade: the drains and the
/// dissolves both walk `first`, `fixture`/`ex`, `last`.
#[test]
fn teardown_walks_fields_in_declaration_order_whatever_the_layout() {
    let src = r#"
type Req { n: Int; }
locus Part {
    params { name: String = ""; hits: Int = 0; }
    fn ask(r: Req) -> Req { return r; }
    drain() { println("drain " + self.name); }
    dissolve() { println("dissolve " + self.name); }
}
api S { rpc Part::ask; }
main locus Holder {
    params {
        first: Part = Part { name: "first" };
        fixture: std::api::test::Rpc = std::api::test::Rpc { };
        ex: api::Exposure = api::serve(S, self.fixture, as: "ex", receivers: { Part: self.first }, bound: 4, on_full: refuse);
        last: Part = Part { name: "last" };
    }
    run() {
        let mut i = 0;
        while i < 3 {
            self.last.hits = self.last.hits + 1;
            i = i + 1;
        }
    }
}
fn main() { Holder { }; }
"#;
    let bin = unique_path("teardown-order");
    let ir = harness::build_source_ir_text(src, &bin).expect("build");
    // `%Holder.<field>.<what>.gep`, in the order the IR emits them
    let order = |what: &str| -> Vec<String> {
        let suffix = format!(".{what}.gep");
        let mut seen: Vec<String> = Vec::new();
        for (at, _) in ir.match_indices("%Holder.") {
            let rest = &ir[at + "%Holder.".len()..];
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if rest[name.len()..].starts_with(&suffix) && !seen.contains(&name) {
                seen.push(name);
            }
        }
        seen
    };
    assert_eq!(order("drain"), vec!["first", "ex", "last"], "the drain cascade, in declaration order");
    assert_eq!(order("cascade"), vec!["first", "fixture", "ex", "last"], "the dissolve cascade, in declaration order");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text, "drain first\ndrain last\ndissolve first\ndissolve last\n", "{text}");
}
