//! GH #1076, step U1: the unit dialect's declarations parse and are not
//! yet checked.
//!
//! Until the dialect's rows and laws land, the checker refuses a `unit`
//! declaration, a scalar `type` declaration and a quantity literal with
//! one located error each, and nothing else: a use of a scalar type's
//! name is not a second error.

use hale_syntax::parse_source;
use hale_types::check_program;

const NOT_YET: &str = "the unit dialect's declarations are parsed and not yet checked (GH #1076)";

/// Every diagnostic `src` gets, as (message, the source text its span
/// covers).
fn diags(src: &str) -> Vec<(String, String)> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| (d.message.clone(), d.span.slice(src).to_string()))
        .collect()
}

#[test]
fn a_unit_declaration_gets_exactly_the_one_located_error() {
    let got = diags("unit us = 1_000 ns;\n\nfn main() {\n    println(1);\n}\n");
    assert_eq!(
        got,
        vec![(format!("unit `us`: {NOT_YET}"), "unit us = 1_000 ns;".to_string())]
    );
}

#[test]
fn each_declaration_and_literal_gets_one_error() {
    let src = "unit tick;\n\
               type Money = quantity Int in cent { round: half_even; }\n\
               type Session = distinct Int { range: 0..64; }\n\
               fn pay(m: Money) -> Money {\n    return m;\n}\n\
               fn main() {\n    let fee = 3bp;\n    println(1);\n}\n";
    let got = diags(src);
    assert_eq!(
        got,
        vec![
            (format!("unit `tick`: {NOT_YET}"), "unit tick;".to_string()),
            (
                format!("type `Money`: {NOT_YET}"),
                "type Money = quantity Int in cent { round: half_even; }".to_string()
            ),
            (
                format!("type `Session`: {NOT_YET}"),
                "type Session = distinct Int { range: 0..64; }".to_string()
            ),
            (format!("quantity literal `3bp`: {NOT_YET}"), "3bp".to_string()),
        ]
    );
}
