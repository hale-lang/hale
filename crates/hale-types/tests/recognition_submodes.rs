//! v1.x-3 PR3 — typecheck rejection for unshipped recognition
//! sub-modes.
//!
//! v1 ships `fixed_cell` + `shared_slab`. `spillover` and
//! `summary_only` parse + typecheck through to a "v1.x pending"
//! diagnostic at resolve-time. The diagnostic fires at the
//! locus-name span so the user sees the rejection alongside the
//! annotation they wrote.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;
use entries::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn fixed_cell_typechecks_clean() {
    let src = r#"
locus Coord : projection recognition(cap=4, fixed_cell) {
    accept(c: Leaf) { }
}
locus Leaf { }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("recognition sub-mode")),
        "fixed_cell must not raise a recognition-sub-mode diag, got: {:?}",
        msgs
    );
}

#[test]
fn shared_slab_typechecks_clean() {
    let src = r#"
locus Coord : projection recognition(cap=4, shared_slab) {
    accept(c: Leaf) { }
}
locus Leaf { }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("recognition sub-mode")),
        "shared_slab must not raise a recognition-sub-mode diag, got: {:?}",
        msgs
    );
}

#[test]
fn spillover_rejected_with_v1x_pending() {
    let src = r#"
locus Coord : projection recognition(cap=4, spillover) {
    accept(c: Leaf) { }
}
locus Leaf { }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("`spillover`") && m.contains("v1.x pending")),
        "spillover must reject with v1.x-pending diag, got: {:?}",
        msgs
    );
}

#[test]
fn summary_only_rejected_with_v1x_pending() {
    let src = r#"
locus Coord : projection recognition(cap=4, summary_only) {
    accept(c: Leaf) { }
}
locus Leaf { }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("`summary_only`") && m.contains("v1.x pending")),
        "summary_only must reject with v1.x-pending diag, got: {:?}",
        msgs
    );
}
