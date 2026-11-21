//! The fixed-width integers `I32`, `U32`, `I16`, `U16`, `I8` and `U8` (GH
//! #2132, F.64). One widens implicitly only where nothing is lost: into a
//! wider type of the same signedness, an unsigned one into a strictly
//! wider signed one, anything into `Int`. A narrowing is a cast with an
//! `or`, refused bare; operands never widen, so a mixed operator is
//! refused naming the conversion; an integer literal takes the type of its
//! place and is held to its range where it is written; an unsigned one has
//! no unary `-`; a shift's count is an `Int`; a quantity may count one.
//! What the programs compute is `tests/hale/fixed_int_*_test.hl`'s.

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

fn clean(src: &str) {
    let errs = located(src);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn a_lossless_widening_is_implicit_everywhere_a_value_flows() {
    clean(
        "type R {\n    wide: U32;\n    n: Int;\n}\n\
         fn take(x: I32) -> Int {\n    return x;\n}\n\
         fn up(x: U8) -> U16 {\n    return x;\n}\n\
         fn main() {\n    let a: U8 = 200;\n    let b: U32 = a;\n    let c: I16 = a;\n    let d: Int = b;\n    \
         let e: U64 = b;\n    let s: I8 = -1;\n    let t: I32 = s;\n    let r = R { wide: a, n: s };\n    \
         let k = take(s) + take(c);\n    let mut w: U32 = 0;\n    w = up(a);\n}\n",
    );
}

#[test]
fn a_narrowing_is_refused_naming_the_cast() {
    let (at, msg) = one("fn main() {\n    let n = 300;\n    let a: U8 = n;\n}\n");
    assert_eq!(at, "n");
    assert_eq!(
        msg,
        "let `a`: expected `U8`, got `Int`: `Int` does not narrow to `U8` implicitly; write `U8(x) or ...`, which \
         says what becomes of a value outside `U8`'s range"
    );
    // a signed one into an unsigned one, whatever the widths
    let (_, msg) = one("fn f(x: I8) -> U32 {\n    return x;\n}\nfn main() {}\n");
    assert!(msg.starts_with("return: expected `U32`, got `I8`: `I8` does not narrow to `U32` implicitly"), "{msg}");
    // an unsigned one into a signed one of its own width
    let (_, msg) = one("fn f(x: I32) {}\nfn main() {\n    let a: U32 = 1;\n    f(a);\n}\n");
    assert!(msg.contains("`U32` does not narrow to `I32` implicitly; write `I32(x) or ...`"), "{msg}");
    // a signed one into a `U64`
    let (_, msg) = one("fn main() {\n    let a: I16 = 1;\n    let b: U64 = a;\n}\n");
    assert!(msg.ends_with("a `I16` may be negative, which no `U64` holds; write `U64(x) or ...`"), "{msg}");
}

#[test]
fn a_narrowing_cast_needs_its_or_and_a_widening_one_takes_none() {
    let (at, msg) = one("fn main() {\n    let n = 300;\n    let a = U8(n);\n}\n");
    assert_eq!(at, "U8(n)");
    assert!(
        msg.starts_with("`U8(…)` narrows `Int` into `U8`'s range `0..256` and this conversion says nothing about a value outside it"),
        "{msg}"
    );
    clean(
        "fn main() {\n    let n = 300;\n    let a = U8(n) or clamp;\n    let b = U8(n) or wrap;\n    let c = U8(n) or 0;\n    \
         let d = I16(Int(a)) or raise;\n    let e: U32 = U32(a);\n    let f = Int(e);\n}\n",
    );
    let (_, msg) = one("fn main() {\n    let a: U8 = 1;\n    let b = U32(a) or 0;\n}\n");
    assert!(msg.contains("is not fallible"), "{msg}");
}

#[test]
fn operands_never_widen() {
    let (at, msg) = one("fn main() {\n    let a: U8 = 1;\n    let b: U32 = 2;\n    let c = a + b;\n}\n");
    assert_eq!(at, "a + b");
    assert_eq!(msg, "binary op: `U8` and `U32` do not mix: operands never widen; make the `U8` a `U32` with `U32(x)`");
    let (_, msg) = one("fn n() -> Int {\n    return 1;\n}\nfn main() {\n    let a: I32 = 1;\n    let b = a < n();\n}\n");
    assert_eq!(
        msg,
        "binary op: `I32` and `Int` do not mix: operands never widen; make the `I32` an `Int` with `Int(x)`, or the \
         `Int` a `I32` with `I32(n) or ...`"
    );
    let (_, msg) = one("fn main() {\n    let a: I8 = 1;\n    let b: U8 = 2;\n    let c = a == b;\n}\n");
    assert!(msg.ends_with("operands never widen; convert one with a cast and an `or`: `I8(x) or ...`"), "{msg}");
}

#[test]
fn an_integer_literal_takes_its_place_type_and_is_held_to_its_range() {
    clean(
        "type R {\n    n: U8;\n}\n\
         fn f(x: I16) -> I16 {\n    return x + 1;\n}\n\
         fn main() {\n    let a: U8 = 255;\n    let b: I8 = -128;\n    let c: U32 = 4294967295;\n    \
         let d = f(-32768) * 1;\n    let r = R { n: 3 };\n    let arr: [I8; 2] = [-1, 127];\n    let m = max(a, 0);\n    \
         let e: U16 = if a > 1 { 5 } else { 65535 };\n    let k = match a {\n        0 -> 1,\n        255 -> b,\n        _ -> -5,\n    };\n    \
         let mask: U8 = ~0;\n}\n",
    );
    let (at, msg) = one("fn main() {\n    let a: U8 = 256;\n}\n");
    assert_eq!(at, "256");
    assert_eq!(
        msg,
        "integer literal 256 does not fit `U8`, which is 0 through 255: a wider type holds it (`Int`, or a wider \
         fixed-width integer)"
    );
    let (at, msg) = one("fn main() {\n    let a: I8 = -129;\n}\n");
    assert_eq!(at, "-129");
    assert!(msg.starts_with("integer literal -129 does not fit `I8`, which is -128 through 127"), "{msg}");
    let (at, msg) = one("fn main() {\n    let a: I8 = 1;\n    let b = a + 128;\n}\n");
    assert_eq!(at, "128");
    assert!(msg.starts_with("integer literal 128 does not fit `I8`"), "{msg}");
    let (_, msg) = one("fn main() {\n    let a: U16 = 1;\n    let b = match a {\n        70000 -> 1,\n        _ -> 2,\n    };\n}\n");
    assert!(msg.starts_with("integer literal 70000 does not fit `U16`"), "{msg}");
}

#[test]
fn an_unsigned_one_has_no_negation_and_a_shift_counts_in_int() {
    let (at, msg) = one("fn main() {\n    let a: U8 = -1;\n}\n");
    assert_eq!(at, "-1");
    assert_eq!(
        msg,
        "`-1` cannot be a `U8`: a `U8` is 0 through 255 and has no unary `-`; write the value itself, or keep it an `Int`"
    );
    let (at, msg) = one("fn main() {\n    let a: U16 = 1;\n    let b = -a;\n}\n");
    assert_eq!(at, "-a");
    assert!(msg.starts_with("`-` does not apply to `U16`: an unsigned fixed-width integer has"), "{msg}");
    clean("fn main() {\n    let a: I16 = 1;\n    let b = -a;\n    let c = a << 3;\n}\n");
    let (_, msg) = one("fn main() {\n    let a: U8 = 1;\n    let n: U8 = 2;\n    let b = a << n;\n}\n");
    assert_eq!(msg, "`<<`: a `U8` shifts by an `Int` count, not a `U8`");
    let (_, msg) = one("fn main() {\n    let a: U32 = 1;\n    let b = 1 >> a;\n}\n");
    assert_eq!(msg, "`>>`: a shift's count is an `Int`, not a `U32`; write `Int(n)`");
}

#[test]
fn a_unit_type_may_count_one() {
    clean(
        "unit tick;\ntype Ticks = quantity U32 in tick;\ntype Level = U8 { range: 0..100; }\ntype Port = distinct U16;\n\
         fn main() {\n    let n: U32 = 3;\n    let t = n * 1tick + 2tick;\n    let raw: U8 = 7;\n    \
         let l = Level(raw) or clamp;\n    let p = Port(80);\n    let q = U16(p);\n}\n",
    );
    let (_, msg) = one("unit tick;\ntype Ticks = quantity U32 in tick;\nfn main() {\n    let n = 3;\n    let k = n;\n    let t = k * 1tick;\n}\n");
    assert!(msg.contains("`U32(x) or ...`"), "{msg}");
}
