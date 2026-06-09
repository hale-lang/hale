//! Proposal A′ typo guard (2026-06-09): a `T::field` / `T::set_field`
//! accessor on a repr-tagged (wire-layout) struct must name a real field;
//! a typo is caught at typecheck (it would otherwise only fail at
//! codegen, since the accessor desugars to a `std::bytes::*` call). A
//! valid accessor is clean, and `T::x` on a non-wire type is not touched.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn typo_field_on_wire_struct_errors() {
    let msgs = check(
        r#"
        type L2 { price: Int `repr:"u32_le"`; }
        fn f(v: Bytes) -> Int { L2::pirce(v) or -1 }
    "#,
    );
    assert!(
        msgs.iter().any(|m| m.contains("no wire field `pirce`")),
        "typo'd accessor should be flagged; got: {:?}",
        msgs
    );
}

#[test]
fn typo_set_field_on_wire_struct_errors() {
    let msgs = check(
        r#"
        type L2 { price: Int `repr:"u32_le"`; }
        fn f(w: Bytes) { L2::set_pirce(w, 1) or raise; }
    "#,
    );
    assert!(
        msgs.iter().any(|m| m.contains("no wire field `pirce`")),
        "typo'd write accessor should be flagged; got: {:?}",
        msgs
    );
}

#[test]
fn valid_read_and_write_accessors_are_clean() {
    let msgs = check(
        r#"
        type L2 { kind: Int `repr:"u8"`; price: Int `repr:"u32_le"`; }
        fn r(v: Bytes) -> Int { L2::price(v) or -1 }
        fn w(b: Bytes) { L2::set_kind(b, 2) or raise; }
    "#,
    );
    assert!(
        !msgs.iter().any(|m| m.contains("no wire field")),
        "valid accessors must not be flagged; got: {:?}",
        msgs
    );
}

#[test]
fn non_wire_type_paths_are_not_touched() {
    // `T::x` where T has no repr-tagged field: not an accessor, so the
    // guard must stay silent (no "no wire field" diagnostic).
    let msgs = check(
        r#"
        type Plain { price: Int; }
        fn f(v: Bytes) -> Int { Plain::anything(v) or -1 }
    "#,
    );
    assert!(
        !msgs.iter().any(|m| m.contains("no wire field")),
        "non-wire type must not trigger the accessor guard; got: {:?}",
        msgs
    );
}
