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

/// GH #831 (the follow-up to GH #759): CONSTRUCTION was the one
/// position the alias was not transparent in. It is now — a struct
/// literal spelled with the alias name builds the declaration the
/// chain ends at.
///
/// This was held back from #759 because the checker's half is one
/// hop while codegen reads a literal's path at roughly twenty
/// `Expr::Struct` sites, and half a fix is a check/build
/// divergence. Codegen resolves the alias once on the merged AST
/// instead (`mangle::resolve_construction_aliases`), so the two
/// layers answer from the same rule. The behaviour half —
/// "constructs, and the fields are the target's" — lives in
/// `tests/hale/type_alias_test.hl`.
#[test]
fn struct_literal_may_be_spelled_with_the_alias_name() {
    let src = r#"
        type Row { id: Int; }
        type Row2 = Row;
        fn main() {
            let r = Row2 { id: 1 };
            println(r.id);
        }
    "#;
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

/// And it is still the TARGET's declaration that validates the
/// initializers — the alias adds no fields and forgives no typo.
#[test]
fn a_literal_through_an_alias_is_checked_against_the_target() {
    let src = r#"
        type Row { id: Int; }
        type Row2 = Row;
        fn main() {
            let r = Row2 { nope: 1 };
            println(r.id);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("has no field `nope`")),
        "expected the target's field check, got {:?}",
        ds
    );
}

/// The four spellings the rule has to reach: a chain of aliases, an
/// alias declared inside a `module` (one flat top-level namespace),
/// an alias of a LOCUS, and an alias of an ENUM in a variant path.
/// Each is one program so a failure names which shape broke.
#[test]
fn a_chain_of_aliases_constructs_the_declaration_it_ends_at() {
    let src = r#"
        type Row { id: Int; }
        type Row2 = Row;
        type Row3 = Row2;
        fn main() {
            let r = Row3 { id: 1 };
            println(r.id);
        }
    "#;
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn an_alias_declared_in_a_module_constructs() {
    let src = r#"
        type Row { id: Int; }
        module geo {
            type Row2 = Row;
        }
        fn main() {
            let r = Row2 { id: 1 };
            println(r.id);
        }
    "#;
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn an_alias_of_a_locus_instantiates_the_locus() {
    let src = r#"
        type Row { id: Int; }
        locus Holder {
            params { seed: Row; }
            fn tag() -> Int { return self.seed.id; }
        }
        type Held = Holder;
        fn main() {
            let h = Held { seed: Row { id: 3 } };
            println(h.tag());
        }
    "#;
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

/// An enum's variant path is the same rule one position over:
/// `C2::Red` where `type C2 = Color;` names `Color::Red`, both
/// where it is CONSTRUCTED and where it is matched. The match arm
/// matters because without it a program with a `_` arm checked
/// clean and then failed to build with "constructor pattern:
/// unknown enum".
#[test]
fn an_alias_of_an_enum_names_its_variants() {
    let src = r#"
        type Color = enum { Red, Green };
        type C2 = Color;
        fn main() {
            let c: Color = C2::Red;
            let mut out = "?";
            match c {
                C2::Red -> { out = "red"; },
                C2::Green -> { out = "green"; },
            }
            println(out);
        }
    "#;
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

/// Transparent, not permissive. An alias of something that cannot
/// be constructed with `{ }` is still refused, with the same
/// message, and codegen's table draws the line over the same
/// declarations — so neither layer invents a literal the other one
/// cannot build.
#[test]
fn an_alias_of_a_non_struct_target_is_still_refused() {
    let src = r#"
        type Row { id: Int; }
        type Thing = Int;
        type TwoRows = [Row; 2];
        fn main() {
            let t = Thing { id: 1 };
            let u = TwoRows { id: 2 };
            println(t.id + u.id);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("`Thing` is not a struct type")),
        "expected the primitive alias to be refused, got {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("`TwoRows` is not a struct type")),
        "expected the array alias to be refused, got {:?}",
        ds
    );
}

/// GH #834 refuses the alias form's own parameter list at the
/// parser. What it must NOT refuse is the shape one token away: an
/// alias whose TARGET is a generic instantiation. That names a
/// concrete type, so the alias is transparent onto the monomorph —
/// `IntPair` and `Pair<Int>` are the same type, and the value the
/// monomorph's own name constructs satisfies the ascription.
#[test]
fn alias_of_a_generic_instantiation_is_its_monomorph() {
    let src = r#"
        type Pair<T> { a: T; b: T; }
        type IntPair = Pair<Int>;
        fn total(p: IntPair) -> Int { return p.a + p.b; }
        fn main() {
            let p: IntPair = Pair_Int { a: 1, b: 2 };
            println(total(p));
        }
    "#;
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}
