//! Go-style struct field tags (2026-06-09): a backtick metadata string
//! after a field is parsed and stored verbatim on the `StructField`, and
//! ignored by everything that doesn't read it. The backtick lexer is
//! shared with time literals (expression position); in field-declaration
//! position it's a tag. General-purpose metadata — the binary-pack layer
//! (Proposal A′) is the first consumer.

use hale_syntax::ast::{TopDecl, TypeDeclBody};
use hale_syntax::parse_source;

fn fields(src: &str) -> Vec<(String, Option<String>)> {
    let prog = parse_source(src).expect("parse");
    for item in &prog.items {
        if let TopDecl::Type(td) = item {
            if let TypeDeclBody::Struct(fs) = &td.body {
                return fs
                    .iter()
                    .map(|f| (f.name.name.clone(), f.tag.clone()))
                    .collect();
            }
        }
    }
    panic!("no struct type in source");
}

#[test]
fn field_tags_are_parsed_and_stored() {
    let got = fields(
        r#"
        type L2 {
            kind:  Int `wire:"u8"`;
            price: Int `wire:"u32_le"`;
            qty:   Int `wire:"u32_le" json:"quantity"`;
            note:  Int;
        }
    "#,
    );
    assert_eq!(
        got,
        vec![
            ("kind".to_string(), Some("wire:\"u8\"".to_string())),
            ("price".to_string(), Some("wire:\"u32_le\"".to_string())),
            (
                "qty".to_string(),
                Some("wire:\"u32_le\" json:\"quantity\"".to_string())
            ),
            ("note".to_string(), None),
        ]
    );
}

#[test]
fn tag_coexists_with_a_default() {
    let got = fields(
        r#"
        type T { count: Int = 0 `json:"n"`; }
    "#,
    );
    assert_eq!(got, vec![("count".to_string(), Some("json:\"n\"".to_string()))]);
}

/// A framework keyword is legal as a struct-literal FIELD NAME, the
/// same way it is legal in the field declaration.
///
/// Reported downstream: `tier` was declarable, readable and
/// assignable (`self.r.tier = 9` typechecks) — only naming it in a
/// literal was blocked, which is the strangest possible surface for
/// the restriction. `parse_struct_init` already accepted these
/// keywords; the struct-literal LOOKAHEAD did not, so `Row { tier: 1 }`
/// fell through to "expression followed by a block" and reported
/// `expected ;, got LBrace` at `Row {` without ever naming `tier`.
#[test]
fn a_framework_keyword_parses_as_a_struct_literal_field() {
    for kw in ["tier", "run", "bus", "type", "capacity", "params"] {
        let src = format!(
            "type Row {{ {kw}: Int = 0; }}\n\
             fn main() {{ let r = Row {{ {kw}: 1 }}; println(r.{kw}); }}\n",
            kw = kw
        );
        assert!(
            hale_syntax::parse_source(&src).is_ok(),
            "`{}` must parse as a struct-literal field name — it is \
             already legal in the declaration",
            kw
        );
    }
}

/// The lookahead must still tell a struct literal from a block, or
/// the fix above would have made every `Foo { … }` a struct literal.
#[test]
fn a_block_is_still_not_a_struct_literal() {
    let src = "fn main() { let x = 1; if x > 0 { println(\"hi\"); } }";
    assert!(
        hale_syntax::parse_source(src).is_ok(),
        "an `if` block must not be parsed as a struct literal"
    );
}
