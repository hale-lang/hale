//! A `Decimal` is an i128 mantissa at the fixed scale 9. A literal
//! that names a value no `Decimal` holds is refused at the literal,
//! with its span: a nonzero digit past the ninth fractional place
//! used to be dropped in silence (`0.1234567891d` was `0.123456789`,
//! downstream handoff). Zeros past the ninth place are the same value.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;

/// Every error of `src`, as (the text at its span, its message).
fn located(src: &str) -> Vec<(String, String)> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.span.slice(src).to_string(), d.message))
        .collect()
}

#[test]
fn a_decimal_literal_past_scale_nine_is_refused_at_the_literal() {
    for lit in ["0.1234567891d", "1.0000000001d", "0.0000000001d"] {
        let src = format!("fn main() {{\n    let a = {lit};\n}}\n");
        assert_eq!(
            located(&src),
            [(
                lit.to_string(),
                format!(
                    "Decimal literal `{lit}` cannot be represented: a `Decimal` has 9 fractional digits, \
                     and this one has a nonzero digit past the ninth (it would be truncated); round it to 9 places"
                )
            )],
            "{lit}"
        );
    }
    let big = format!("{}.0d", "9".repeat(40));
    let src = format!("fn main() {{\n    let a = {big};\n}}\n");
    assert_eq!(
        located(&src),
        [(big.clone(), format!("Decimal literal `{big}` cannot be represented: its mantissa at scale 9 does not fit an i128"))]
    );
}

#[test]
fn nine_places_and_trailing_zeros_are_accepted() {
    let src = "fn main() { let a = 0.123456789d; let b = 0.1234567890000d; let c = 19.99d; let d = 1d / 3d; }";
    assert!(located(src).is_empty(), "{:?}", located(src));
}
