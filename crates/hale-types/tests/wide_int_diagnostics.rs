//! `U256` and `I256` (GH #1934): the 256-bit integers never mix with
//! another type implicitly. An operator, a binding or a return that puts
//! one beside an `Int` (or the other 256-bit type) is refused naming the
//! conversion; a shift's count is an `Int`; a `U256` has no unary `-`.
//! The `json:` tag's `hex` option is for an `Int`, `U256` or `I256` field
//! (or a list of them), and an unknown option names both options. What the
//! programs compute is `tests/hale/u256_*_test.hl`'s.

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
    let (at, msg) = one("fn main() {\n    let a = U256::from_int(1);\n    let b = a + 1;\n}\n");
    assert_eq!(at, "a + 1");
    assert_eq!(
        msg,
        "binary op: `U256` and `Int` do not mix: a 256-bit integer never converts implicitly; make the `Int` a \
         `U256` with `U256::from_int(n)`"
    );
    // a comparison too, either side
    let (at, msg) = one("fn main() {\n    let a = I256::from_int(1);\n    let b = 0 < a;\n}\n");
    assert_eq!(at, "0 < a");
    assert!(msg.contains("`I256::from_int(n)`"), "{msg}");
}

#[test]
fn the_two_widths_do_not_mix_either() {
    let (at, msg) = one("fn main() {\n    let a = U256::from_int(1);\n    let b = I256::from_int(1) == a;\n}\n");
    assert_eq!(at, "I256::from_int(1) == a");
    assert_eq!(
        msg,
        "binary op: `I256` and `U256` do not mix: a 256-bit integer never converts implicitly; convert one \
         explicitly (`I256::from_u256(x) or ...`)"
    );
}

#[test]
fn a_binding_and_a_return_name_the_conversion() {
    let (at, msg) = one("fn main() {\n    let c: U256 = 5;\n}\n");
    assert_eq!(at, "5");
    assert_eq!(
        msg,
        "let `c`: expected `U256`, got `Int`: a 256-bit integer never converts implicitly; write `U256::from_int(n)`"
    );
    let (_, msg) = one("fn f(x: I256) -> Int {\n    return x;\n}\nfn main() {}\n");
    assert_eq!(
        msg,
        "return: expected `Int`, got `I256`: a 256-bit integer never converts implicitly; write `I256::to_int(x) or ...`"
    );
}

#[test]
fn a_shift_counts_in_int_and_a_u256_has_no_sign() {
    let (at, msg) = one("fn main() {\n    let a = U256::from_int(1);\n    let b = a << a;\n}\n");
    assert_eq!(at, "a << a");
    assert_eq!(msg, "`<<`: a `U256` shifts by an `Int` count, not a `U256`");
    let (at, msg) = one("fn main() {\n    let a = U256::from_int(1);\n    let b = -a;\n}\n");
    assert_eq!(at, "-a");
    assert!(msg.starts_with("`-` does not apply to `U256`:"), "{msg}");
    assert!(msg.contains("it has no sign to negate (`I256` does)"), "{msg}");
    // what does apply checks clean
    let errs = located(
        "fn main() {\n    let a = U256::from_int(1);\n    let i = I256::from_int(-1);\n    \
         let b = (a << 3) >> 1 & a | a ^ ~a;\n    let c = -i * i / i % i;\n    let d = min(a, a) == max(a, abs(a));\n}\n",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn the_hex_tag_option_is_for_integers() {
    let errs = located(
        "type R {\n    a: Int `json:\"a,hex\"`;\n    b: U256 `json:\",hex\"`;\n    c: bounded[I256; 2] `json:\",hex\"`;\n    \
         d: String `json:\"d,hex\"`;\n    e: Decimal<2> `json:\",hex\"`;\n    f: Int `json:\",lower\"`;\n}\n\
         fn f() -> String { return R::to_json(R { a: 1, b: U256::from_int(1), d: \"\", e: 0d, f: 0 }); }\n",
    );
    let msgs: Vec<&str> = errs.iter().map(|(_, m)| m.as_str()).collect();
    assert!(
        msgs.contains(
            &"`R.d`: `json:` tag option `hex` writes an integer as `0x` hex text, and the field is `String`: it is for \
              an `Int`, `U64`, `U256` or `I256` field or a list of them"
        ),
        "{msgs:?}"
    );
    assert!(msgs.iter().any(|m| m.starts_with("`R.e`: `json:` tag option `hex`")), "{msgs:?}");
    assert!(
        msgs.contains(
            &"`R.f`: `json:` tag option `lower` is not one: the options are `string`, a Decimal written as a JSON \
              string (`json:\"price,string\"`), and `hex`, an `Int`, `U64`, `U256` or `I256` written as `0x` \
              hex text (`json:\"gasUsed,hex\"`)"
        ),
        "{msgs:?}"
    );
    assert_eq!(msgs.len(), 3, "an Int, a U256 and a list of I256 take `hex`: {msgs:?}");
}

#[test]
fn a_256_bit_integer_crosses_no_ffi_boundary() {
    let errs = located("@ffi(\"c\")\nfn take(x: U256) -> Int;\nfn main() {}\n");
    assert!(
        errs.iter().any(|(_, m)| m.contains("U256 / I256 (i256) has no portable C mapping")),
        "{errs:?}"
    );
}
