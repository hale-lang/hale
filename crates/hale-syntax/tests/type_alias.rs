//! GH #759 — the alias form of `type`.
//!
//! `spec/grammar.ebnf` has always listed `type Name = Type ;` and
//! the AST has always had `TypeDeclBody::Alias`, but the branch
//! that builds it was unreachable: the enum split was written
//! `eat(&TokenKind::Ident("enum".to_string()))`, and `eat`
//! compares only the token's DISCRIMINANT — so every identifier
//! matched. `type Thing = Int;` entered the enum branch, demanded
//! `{`, and reported "expected {, got Semi" at the `;`.
//!
//! These tests pin the three outcomes that split: the alias
//! parses (top level, in a module, and as a locus member), the
//! enum form still parses, and a malformed alias still errors.

use hale_syntax::ast::{LocusMember, TopDecl, TypeDeclBody, TypeExpr};
use hale_syntax::parse_source;

fn type_decls(src: &str) -> Vec<hale_syntax::ast::TypeDecl> {
    let prog = parse_source(src).expect("parse");
    prog.items
        .iter()
        .filter_map(|i| match i {
            TopDecl::Type(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn parses_alias_of_a_primitive_at_top_level() {
    let decls = type_decls("type Thing = Int;\n");
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].name.name, "Thing");
    match &decls[0].body {
        TypeDeclBody::Alias(TypeExpr::Primitive(p, _)) => {
            assert_eq!(format!("{:?}", p), "Int");
        }
        other => panic!("expected an alias of Int, got {:?}", other),
    }
}

#[test]
fn parses_alias_of_a_user_type_and_of_a_generic_instantiation() {
    let decls = type_decls(
        "type Row { id: Int; }\n\
         type Row2 = Row;\n\
         type Names = Vec<String>;\n",
    );
    assert_eq!(decls.len(), 3);

    match &decls[1].body {
        TypeDeclBody::Alias(TypeExpr::Named { path, generic_args, .. }) => {
            assert_eq!(path.segments[0].name, "Row");
            assert!(generic_args.is_empty());
        }
        other => panic!("expected an alias of Row, got {:?}", other),
    }
    match &decls[2].body {
        TypeDeclBody::Alias(TypeExpr::Named { path, generic_args, .. }) => {
            assert_eq!(path.segments[0].name, "Vec");
            assert_eq!(generic_args.len(), 1);
        }
        other => panic!("expected an alias of Vec<String>, got {:?}", other),
    }
}

#[test]
fn parses_alias_inside_a_module() {
    let prog = parse_source(
        "module geo {\n    type Meters = Int;\n}\n",
    )
    .expect("parse");
    let m = prog
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::Module(m) => Some(m),
            _ => None,
        })
        .expect("module decl present");
    let t = m
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::Type(t) => Some(t),
            _ => None,
        })
        .expect("type decl inside the module");
    assert_eq!(t.name.name, "Meters");
    assert!(matches!(t.body, TypeDeclBody::Alias(_)));
}

#[test]
fn parses_alias_as_a_locus_member() {
    let prog =
        parse_source("locus Box {\n    type Slot = Int;\n}\n").expect("parse");
    let l = prog
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::Locus(l) => Some(l),
            _ => None,
        })
        .expect("locus decl present");
    let t = l
        .members
        .iter()
        .find_map(|m| match m {
            LocusMember::Type(t) => Some(t),
            _ => None,
        })
        .expect("type member");
    assert_eq!(t.name.name, "Slot");
    assert!(matches!(t.body, TypeDeclBody::Alias(_)));
}

/// The enum form shares the `=` with the alias form; the split is
/// the contextual keyword `enum`. It must still parse as an enum,
/// and an identifier that merely *looks* like a type must not be
/// mistaken for it (the exact confusion that hid the alias).
#[test]
fn enum_form_still_parses_and_only_enum_takes_the_enum_branch() {
    let decls = type_decls("type Color = enum { Red, Green };\n");
    assert_eq!(decls.len(), 1);
    match &decls[0].body {
        TypeDeclBody::Enum(vs) => {
            assert_eq!(vs.len(), 2);
            assert_eq!(vs[0].name.name, "Red");
        }
        other => panic!("expected an enum, got {:?}", other),
    }

    // `Int` is not `enum`, so this is an alias — not "expected {".
    assert!(matches!(
        type_decls("type Thing = Int;\n")[0].body,
        TypeDeclBody::Alias(_)
    ));
}

#[test]
fn alias_with_no_target_is_a_parse_error() {
    let diags = parse_source("type Thing = ;\n").expect_err("must not parse");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("expected type expression")),
        "expected a type-expression diagnostic, got {:?}",
        diags
    );
}

#[test]
fn alias_without_a_semicolon_is_a_parse_error() {
    let diags =
        parse_source("type Thing = Int\nfn main() { }\n").expect_err("must not parse");
    assert!(
        diags.iter().any(|d| d.message.contains("expected ;")),
        "expected a missing-semicolon diagnostic, got {:?}",
        diags
    );
}
