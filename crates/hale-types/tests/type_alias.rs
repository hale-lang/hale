//! GH #759 — what the CHECKER says about `type Name = Type;`.
//!
//! The behaviour tests for a transparent alias live in
//! `tests/hale/type_alias_test.hl` (run a program, check what it
//! computed). What belongs here is compiler OUTPUT: that the
//! alias unifies with its target rather than producing a second
//! nominal type, the one shape that is deliberately *not* yet
//! supported, and the cyclic-chain diagnostic that making the
//! form writable created.

use hale_syntax::parse_source;
use hale_types::symbol::Bundle;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    let mut programs: std::collections::BTreeMap<
        String,
        &hale_syntax::ast::Program,
    > = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.iter().map(|d| d.message.clone()).collect()
}

/// The issue's own program. `Thing` IS `Int` — no nominal wall.
#[test]
fn alias_of_a_primitive_unifies_with_its_target() {
    let src = r#"
        type Thing = Int;
        fn main() {
            let t: Thing = 3;
            let u: Int = t;
            println(u);
        }
    "#;
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

/// Transparent does not mean permissive: the alias still carries
/// its target's type, so a wrong value is still rejected — and the
/// message names what the alias expands to, not the alias.
#[test]
fn alias_still_rejects_a_value_of_the_wrong_type() {
    let src = r#"
        type Thing = Int;
        fn main() {
            let t: Thing = "three";
            println(t);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("expected `Int`")),
        "expected the target's name in the message, got {:?}",
        ds
    );
}

/// An alias chain that returns to its own name names nothing, and
/// every pass that follows a type would follow it forever. Making
/// the form writable is what put this within reach, so the
/// diagnostic ships with it.
#[test]
fn a_cyclic_alias_chain_is_an_error() {
    let src = r#"
        type A = B;
        type B = A;
        fn main() {
            let x: A = 1;
            println(x);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("type alias `A` is cyclic")),
        "expected a cyclic-alias diagnostic, got {:?}",
        ds
    );
}

/// NOT YET (GH #831, the follow-up to GH #759): the alias is
/// transparent in TYPE position, but a struct LITERAL must still
/// name the declaring type. Construction resolves the literal's
/// path through codegen's many `Expr::Struct` sites, none of which
/// consult the alias table; doing the checker half alone would
/// ship a check/build divergence, so it is both halves or neither.
/// Pinned so the day it starts working is a deliberate one.
#[test]
fn not_yet_struct_literal_spelled_with_the_alias_name() {
    let src = r#"
        type Row { id: Int; }
        type Row2 = Row;
        fn main() {
            let r = Row2 { id: 1 };
            println(r.id);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("`Row2` is not a struct type")),
        "expected the literal to be refused, got {:?}",
        ds
    );
}
