//! Closed-world topology rewrite: payload with nested-struct fields
//! must round-trip through the synthesized `self.<sub>.handler(v)`
//! call with the same field semantics as a hand-written method call.
//!
//! The rewrite (see `desugar_intra_locus_topics`) collapses
//! `publish→queue→drain→dispatch` into a direct method call when a
//! parent locus has a singleton field of the subscriber's type.
//! Fathom's smoke-topics work surfaced a suspected silent-miscompile
//! where the rewrite read wrong bytes from nested-struct fields
//! (`tk.side.kind` returning the trailing field's value instead of
//! the nested String). Bug doesn't reproduce on current HEAD; this
//! test locks in the working shape against regression.
//!
//! Coverage:
//! 1. Canonical: nested struct mid-payload, four fields total.
//! 2. Nested struct at first / middle / last field positions.
//! 3. Two-deep nesting (struct-in-struct).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn closed_world_canonical_nested_struct_string_field() {
    // The handoff's exact repro shape.
    let src = r#"
        type Side { kind: String = "bid"; }
        type Tick {
            symbol:   String = "";
            side:     Side   = Side { kind: "bid" };
            venue_ts: Time   = `2026-01-01T00:00:00Z`;
            recv_ts:  Time   = `2026-01-01T00:00:00Z`;
        }
        topic TickT { payload: Tick; subject: "md.tick"; }
        locus Sub {
            params { }
            bus { subscribe TickT as on_tick; }
            fn on_tick(tk: Tick) {
                println("side.kind=[", tk.side.kind, "]");
            }
        }
        locus Pub {
            params { sub: Sub; }
            bus { publish TickT; }
            run() {
                TickT <- Tick {
                    symbol:   "ABC-123",
                    side:     Side { kind: "bid" },
                    venue_ts: `2026-01-01T12:00:00Z`,
                    recv_ts:  `2026-01-01T13:00:00Z`,
                };
            }
        }
        fn main() {
            let s = Sub { };
            Pub { sub: s };
        }
    "#;
    let (stdout, status) = build_and_run("closed_world_canonical", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("side.kind=[bid]"),
        "expected nested side.kind to read 'bid'; got: {:?}",
        stdout
    );
}

#[test]
fn closed_world_nested_struct_at_first_position() {
    let src = r#"
        type Inner { v: String = ""; }
        type P { inner: Inner = Inner { v: "" }; a: Int = 0; b: Int = 0; }
        topic T { payload: P; subject: "x"; }
        locus Sub {
            params { }
            bus { subscribe T as on_msg; }
            fn on_msg(p: P) { println("v=[", p.inner.v, "]"); }
        }
        locus Pub {
            params { sub: Sub; }
            bus { publish T; }
            run() { T <- P { inner: Inner { v: "first" }, a: 1, b: 2 }; }
        }
        fn main() { let s = Sub { }; Pub { sub: s }; }
    "#;
    let (stdout, status) = build_and_run("closed_world_first_pos", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("v=[first]"), "got: {:?}", stdout);
}

#[test]
fn closed_world_nested_struct_at_last_position() {
    let src = r#"
        type Inner { v: String = ""; }
        type P { a: Int = 0; b: Int = 0; inner: Inner = Inner { v: "" }; }
        topic T { payload: P; subject: "x"; }
        locus Sub {
            params { }
            bus { subscribe T as on_msg; }
            fn on_msg(p: P) { println("v=[", p.inner.v, "]"); }
        }
        locus Pub {
            params { sub: Sub; }
            bus { publish T; }
            run() { T <- P { a: 1, b: 2, inner: Inner { v: "last" } }; }
        }
        fn main() { let s = Sub { }; Pub { sub: s }; }
    "#;
    let (stdout, status) = build_and_run("closed_world_last_pos", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("v=[last]"), "got: {:?}", stdout);
}

#[test]
fn closed_world_two_deep_nesting() {
    let src = r#"
        type Leaf { v: String = ""; }
        type Mid  { leaf: Leaf = Leaf { v: "" }; }
        type P    { a: Int = 0; mid: Mid = Mid { leaf: Leaf { v: "" } }; b: Int = 0; }
        topic T { payload: P; subject: "x"; }
        locus Sub {
            params { }
            bus { subscribe T as on_msg; }
            fn on_msg(p: P) { println("v=[", p.mid.leaf.v, "]"); }
        }
        locus Pub {
            params { sub: Sub; }
            bus { publish T; }
            run() {
                T <- P { a: 1, mid: Mid { leaf: Leaf { v: "deep" } }, b: 2 };
            }
        }
        fn main() { let s = Sub { }; Pub { sub: s }; }
    "#;
    let (stdout, status) = build_and_run("closed_world_two_deep", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("v=[deep]"), "got: {:?}", stdout);
}
