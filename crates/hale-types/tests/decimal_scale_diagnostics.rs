//! `Decimal<S>`: a Decimal's scale is part of its type (2026-10-12).
//! A literal is refused at the literal when the scale of the place it
//! flows into cannot hold it; two scales never mix, at a binding, an
//! argument or an operator; `d.rescale(T)` converts, and a narrowing
//! one that says nothing of the digits it drops is refused. The
//! diagnostics are pinned here; what the programs compute is
//! `tests/hale/decimal_scale_test.hl`'s.

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

fn one(src: &str) -> (String, String) {
    let errs = located(src);
    assert_eq!(errs.len(), 1, "{errs:?}");
    errs.into_iter().next().expect("one")
}

#[test]
fn a_literal_past_its_places_scale_is_refused_at_the_literal() {
    let (at, msg) = one("fn main() {\n    let p: Decimal<2> = 1.505d;\n}\n");
    assert_eq!(at, "1.505d");
    assert_eq!(
        msg,
        "Decimal literal `1.505d` cannot be represented: a `Decimal<2>` has 2 fractional digits, and this one \
         has a nonzero digit past them (it would be truncated); round it to 2 places, or give the place it \
         flows into a wider scale"
    );
    // A field, an argument and a return are places too.
    let (at, _) = one("type P { a: Decimal<0>; }\nfn main() {\n    let p = P { a: 0.5d };\n}\n");
    assert_eq!(at, "0.5d");
    let (at, _) = one("fn f(x: Decimal<1>) {}\nfn main() {\n    f(0.25d);\n}\n");
    assert_eq!(at, "0.25d");
    let (at, _) = one("fn f() -> Decimal<3> {\n    return 0.0001d;\n}\nfn main() {}\n");
    assert_eq!(at, "0.0001d");
    // Unannotated, a literal is plain `Decimal`'s, as before.
    let (at, msg) = one("fn main() {\n    let a = 0.1234567891d;\n}\n");
    assert_eq!(at, "0.1234567891d");
    assert!(msg.contains("a `Decimal` has 9 fractional digits"), "{msg}");
}

#[test]
fn a_literal_takes_the_scale_of_the_place_it_flows_into() {
    let src = "type P { a: Decimal<18>; b: Decimal<2> = 0.25d; }\n\
               const WEI: Decimal<18> = 0.000000000000000001d;\n\
               fn f(x: Decimal<18>) -> Decimal<18> { return x * 2d + 0.000000000000000001d; }\n\
               fn g() -> Decimal<4> { 2.5001d }\n\
               fn main() {\n\
                   let a: Decimal<18> = 0.000000000000000001d;\n\
                   let p = P { a: 0.000000000000000002d };\n\
                   let b = f(0.000000000000000003d) + a - 0.000000000000000004d;\n\
                   let c: [Decimal<18>; 2] = [1.5d, 0.000000000000000005d];\n\
                   let d: Decimal<18> = if b > 0d { 0.000000000000000006d } else { 1d };\n\
                   let e: Decimal<18> = std::str::parse_decimal(\"1\") or 0.000000000000000007d;\n\
                   let m = max(a, 0.000000000000000008d);\n\
               }\n";
    assert!(located(src).is_empty(), "{:?}", located(src));
}

#[test]
fn two_scales_never_mix() {
    let (at, msg) = one("fn main() {\n    let a: Decimal<18> = 1d;\n    let d: Decimal = a;\n}\n");
    assert_eq!(at, "a");
    assert_eq!(
        msg,
        "`Decimal<18>` is not `Decimal`: a Decimal's scale is part of its type, and two scales never mix; \
         convert explicitly (`.rescale(9)`)"
    );
    let (at, msg) =
        one("fn main() {\n    let a: Decimal<18> = 1d;\n    let b: Decimal = 2d;\n    let c = a + b;\n}\n");
    assert_eq!(at, "a + b");
    assert_eq!(
        msg,
        "binary op: `Decimal<18>` and `Decimal` are different scales: a Decimal's scale is part of its type, \
         and two scales never mix; convert one explicitly (`.rescale(18)`)"
    );
    let (at, _) = one("fn f(x: Decimal) {}\nfn main() {\n    let a: Decimal<2> = 1d;\n    f(a);\n}\n");
    assert_eq!(at, "a");
    let (at, _) = one("fn main() {\n    let a: Decimal<2> = 1d;\n    let b: Decimal<4> = 1d;\n    let c = a < b;\n}\n");
    assert_eq!(at, "a < b");
}

#[test]
fn a_narrowing_rescale_says_what_becomes_of_the_dropped_digits() {
    let (at, msg) = one("fn main() {\n    let a: Decimal<18> = 1d;\n    let b = a.rescale(2);\n}\n");
    assert_eq!(at, "a.rescale(2)");
    assert_eq!(
        msg,
        "`.rescale(2)` narrows `Decimal<18>` to `Decimal<2>`, dividing its mantissa by \
         10,000,000,000,000,000, and says nothing of the digits it drops: round them (`or trunc`, \
         `or floor`, `or ceil`, `or half_up`, `or half_even`), or refuse a nonzero one (`or <value>`, \
         `or raise`, `or handler(err)` with the `InexactError`)"
    );
    // Widening is exact, and a narrowing with its `or` is discharged.
    let src = "fn main() {\n    let a: Decimal<18> = 1d;\n    let b = a.rescale(2) or half_even;\n    \
               let c = b.rescale(18);\n    let d = a.rescale(4) or 0d;\n    let e: Decimal<18> = c + a;\n}\n";
    assert!(located(src).is_empty(), "{:?}", located(src));
    // A clamp is a range's policy, not a rounding.
    let (_, msg) = one("fn main() {\n    let a: Decimal<18> = 1d;\n    let b = a.rescale(2) or clamp;\n}\n");
    assert!(msg.starts_with("`or clamp` is a range's policy"), "{msg}");
}

#[test]
fn the_scale_is_an_integer_literal_from_zero_to_eighteen() {
    let (at, msg) = one("fn main() {\n    let a: Decimal<18> = 1d;\n    let n = 2;\n    let b = a.rescale(n);\n}\n");
    assert_eq!(at, "n");
    assert_eq!(msg, "`.rescale(S)`: the scale is part of the type, an integer literal from 0 to 18");
    let parsed = parse_source("fn main() {\n    let a: Decimal<19> = 1d;\n}\n");
    let ds = parsed.expect_err("a scale past 18 does not parse");
    assert!(
        ds.iter().any(|d| d.message
            == "`Decimal<19>`: a Decimal's scale is from 0 to 18 (an i128 mantissa at scale 18 still holds \
                twenty integer digits)"),
        "{ds:?}"
    );
}

#[test]
fn a_quantity_still_counts_an_int() {
    let (_, msg) = one("unit wei;\ntype Eth = quantity Decimal<18> in wei;\nfn main() {}\n");
    assert_eq!(msg, "type `Eth`: a quantity counts an `Int`, and `Decimal<18>` is not one: write `quantity Int in wei`");
}
