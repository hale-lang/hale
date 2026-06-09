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
