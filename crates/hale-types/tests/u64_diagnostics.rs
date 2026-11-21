//! `U64` (GH #2121): a 64-bit unsigned integer never mixes with another
//! type implicitly. An operator, a binding or a return that puts one
//! beside a non-literal `Int` is refused naming the conversion; an integer
//! literal takes the type of its place, and one that no `U64` holds (a
//! negative one, or past `Int`'s max in a place that is no `U64`) is
//! refused where it is written; a shift's count is an `Int`; a `U64` has
//! no unary `-`; the unit dialect's scalars count an `Int`, so a quantity,
//! an identity or a range over a `U64` is refused at the type. What the
//! programs compute is `tests/hale/u64_*_test.hl`'s.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::json_gen::generate_json_parsers;
use hale_syntax::parse_source;

/// Every error of `src`, as (the text at its span, its message).
fn located(src: &str) -> Vec<(String, String)> {
    let mut program = parse_source(src).expect("parse");
    generate_json_parsers(&mut program);
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
fn an_int_operand_is_refused_naming_the_conversion() {
    let (at, msg) = one("fn n() -> Int {\n    return 1;\n}\nfn main() {\n    let a: U64 = 1;\n    let b = a + n();\n}\n");
    assert_eq!(at, "a + n()");
    assert_eq!(
        msg,
        "binary op: `U64` and `Int` do not mix: a `U64` never converts implicitly; make the `Int` a `U64` with \
         `U64::from_int(n)`, or the `U64` an `Int` with `U64::to_int(x) or ...`"
    );
    // a comparison too, either side
    let (at, msg) = one("fn main() {\n    let a: U64 = 1;\n    let n = 3;\n    let b = n < a;\n}\n");
    assert_eq!(at, "n < a");
    assert!(msg.contains("`U64::from_int(n)`"), "{msg}");
    // and the other widths
    let (_, msg) = one("fn main() {\n    let a: U64 = 1;\n    let b = a == U256::from_int(1);\n}\n");
    assert!(msg.starts_with("binary op: `U64` and `U256` do not mix"), "{msg}");
}

#[test]
fn a_binding_an_argument_and_a_return_name_the_conversion() {
    let (at, msg) = one("fn main() {\n    let n = 3;\n    let c: U64 = n;\n}\n");
    assert_eq!(at, "n");
    assert_eq!(msg, "let `c`: expected `U64`, got `Int`: a `U64` never converts implicitly; write `U64::from_int(n)`");
    let (_, msg) = one("fn f(x: U64) -> Int {\n    return x;\n}\nfn main() {}\n");
    assert_eq!(
        msg,
        "return: expected `Int`, got `U64`: a `U64` never converts implicitly; write `U64::to_int(x) or ...`"
    );
    let (at, msg) = one("fn f(x: U64) -> Float {\n    return Float(x);\n}\nfn main() {\n    let a: U64 = 1;\n    let i = Int(a);\n}\n");
    assert_eq!(at, "a");
    assert_eq!(
        msg,
        "`Int(…)` of a `U64` is not a cast: a `U64` past 2^63 - 1 has no `Int`, so the conversion can fail: write \
         `U64::to_int(x) or ...`"
    );
}

#[test]
fn an_integer_literal_takes_its_place_type_and_is_held_to_its_range() {
    // every place a literal is a `U64` checks clean
    let errs = located(
        "type R {\n    n: U64;\n}\n\
         fn f(x: U64) -> U64 {\n    return x + 1;\n}\n\
         fn main() {\n    let a: U64 = 18446744073709551615;\n    let b = 9223372036854775808;\n    \
         let c: U64 = 0xFFFF_FFFF_FFFF_FFFF;\n    let d = f(7) * 2 + a % 10;\n    let r = R { n: 3 };\n    \
         let arr: [U64; 2] = [1, 2];\n    let m = max(a, 0);\n    let e: U64 = if a > 1 { 5 } else { b };\n    \
         let k = match a {\n        0 -> 1,\n        18446744073709551615 -> b,\n        _ -> a,\n    };\n    \
         let mask: U64 = ~0 >> 1;\n}\n",
    );
    assert!(errs.is_empty(), "{errs:?}");
    // a negative literal is no `U64`
    let (at, msg) = one("fn main() {\n    let c: U64 = -5;\n}\n");
    assert_eq!(at, "-5");
    assert_eq!(
        msg,
        "`-5` cannot be a `U64`: a `U64` is 0 through 2^64 - 1 (18446744073709551615) and has no unary `-`; write the \
         value itself, or keep it an `Int`"
    );
    // a literal past `Int`'s max is a `U64`, and nothing else
    let (at, msg) = one("fn main() {\n    let c: Int = 9223372036854775808;\n}\n");
    assert_eq!(at, "9223372036854775808");
    assert_eq!(
        msg,
        "integer literal 9223372036854775808 is past `Int`'s max (9223372036854775807): only a `U64` holds it, and its \
         place is `Int`: give the place the type `U64`"
    );
}

#[test]
fn a_literal_past_two_to_the_64_does_not_lex() {
    let errs = parse_source("fn main() {\n    let c = 18446744073709551616;\n}\n").expect_err("a lex error");
    assert_eq!(
        errs[0].message,
        "integer literal 18446744073709551616 is past the largest integer, 2^64 - 1 (18446744073709551615, a `U64`'s max)"
    );
}

#[test]
fn a_shift_counts_in_int_and_a_u64_has_no_sign() {
    let (at, msg) = one("fn main() {\n    let a: U64 = 1;\n    let b = a << a;\n}\n");
    assert_eq!(at, "a << a");
    assert_eq!(msg, "`<<`: a `U64` shifts by an `Int` count, not a `U64`");
    let (at, msg) = one("fn main() {\n    let a: U64 = 1;\n    let b = 1 << a;\n}\n");
    assert_eq!(at, "1 << a");
    assert_eq!(msg, "`<<`: a shift's count is an `Int`, not a `U64`; write `U64::to_int(n) or ...`");
    let (at, msg) = one("fn main() {\n    let a: U64 = 1;\n    let b = -a;\n}\n");
    assert_eq!(at, "-a");
    assert!(msg.starts_with("`-` does not apply to `U64`:"), "{msg}");
    assert!(msg.contains("it is never negative, so it has no unary `-`"), "{msg}");
    let errs = located(
        "fn main() {\n    let a: U64 = 1;\n    let b = (a << 3) >> 1 & a | a ^ ~a;\n    \
         let d = min(a, a) == max(a, abs(a));\n    let f = Float(a);\n}\n",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn a_scalar_of_the_unit_dialect_counts_an_int_not_a_u64() {
    let (at, msg) = one("unit cent;\ntype Money = quantity U64 in cent;\nfn main() {}\n");
    assert_eq!(at, "U64");
    assert_eq!(msg, "type `Money`: a quantity counts an `Int`, and `U64` is not one: write `quantity Int in cent`");
    let (at, msg) = one("type Id = distinct U64;\nfn main() {}\n");
    assert_eq!(at, "U64");
    assert!(msg.starts_with("type `Id`: an identity is `distinct Int`"), "{msg}");
    let (_, msg) = one("type Small = U64 { range: 0..10; };\nfn main() {}\n");
    assert!(msg.starts_with("type `Small`: a range type refines `Int`"), "{msg}");
}

#[test]
fn a_u64_crosses_the_ffi_as_a_uint64_t() {
    let errs = located("@ffi(\"c\")\nfn take(x: U64) -> U64;\nfn main() {}\n");
    assert!(errs.is_empty(), "{errs:?}");
}
