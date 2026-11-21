//! GH #2131: the unit dialect's base integer is a parameter. A quantity,
//! a point, an identity or a range counts an `Int` or a `U64`, and every
//! rule a check holds it to is that integer's: a component's quantities
//! count one integer (an `Int` count and a `U64` count of one
//! denomination never mix, and the error names the conversion through the
//! count); an `Int` beside a scalar counting a `U64` is refused naming the
//! conversion, as beside a `U64`; a factor no value of the integer holds
//! is refused at the conversion; a range's bounds are values of it; a
//! literal's count is held to it; and an unsigned scalar has no `-`. What
//! the programs compute is `tests/hale/units_u64_*_test.hl`'s.

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

const WEI: &str = "unit wei;\nunit gwei = 1_000_000_000 wei;\nunit eth = 1_000_000_000 gwei;\n\
                   type Wei = quantity U64 in wei;\ntype Nonce = distinct U64;\n\
                   type Port = U64 { range: 1..65536; }\n";

fn with_wei(body: &str) -> String {
    format!("{WEI}fn main() {{\n{body}}}\n")
}

#[test]
fn an_int_count_and_a_u64_count_of_one_denomination_never_mix() {
    let src = "unit wei;\nunit gwei = 1_000_000_000 wei;\ntype Wei = quantity U64 in wei;\n\
               type Signed = quantity Int in gwei { round: floor; }\nfn main() {}\n";
    let mut program = parse_source(src).expect("parse");
    generate_json_parsers(&mut program);
    let all: Vec<_> = entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
    assert_eq!(all.len(), 1, "{all:#?}");
    let d = &all[0];
    assert_eq!(d.span.slice(src), "Int");
    assert_eq!(
        d.message,
        "type `Signed`: the units of `gwei` count a `U64`, as their quantity `Wei` does, and every quantity of them \
         counts it: a `U64` count and an `Int` count of one denomination never mix; write `quantity U64 in gwei`, or \
         count an `Int` in a unit of its own (`unit tick;`, `quantity Int in tick`) and convert through the count, \
         `U64::to_int(x) or ...`"
    );
    assert_eq!(d.related.len(), 1, "the component's quantity is the witness: {:#?}", d.related);
    assert_eq!(d.related[0].label, "`Wei` counts a `U64`");

    // the stdlib's time component counts an `Int`
    let (at, msg) = one("type Stamp = quantity U64 in 100ms { round: floor; }\nfn main() {}\n");
    assert_eq!(at, "U64");
    assert_eq!(
        msg,
        "type `Stamp`: the units of `ms` count an `Int`, as their quantity `Duration` (the stdlib's, `std::time`) \
         does, and every quantity of them counts it: an `Int` count and a `U64` count of one denomination never mix; \
         write `quantity Int in 100 ms`, or count a `U64` in a unit of its own (`unit tick;`, `quantity U64 in tick`) \
         and convert through the count, `U64::from_int(n)`"
    );
}

#[test]
fn an_int_beside_a_u64_scalar_is_refused_naming_the_conversion() {
    let (at, msg) = one(&with_wei("    let w: Wei = 5wei;\n    let n = 3;\n    let a = w * n;\n"));
    assert_eq!(at, "w * n");
    assert_eq!(msg, "`Wei` * `Int`: `Wei` counts a `U64`, and an `Int` never mixes with it: write `U64::from_int(n)`");
    let (_, msg) = one(&with_wei("    let w: Wei = 5wei;\n    let n = 3;\n    let a = w / n;\n"));
    assert!(msg.starts_with("`Wei` / `Int`: `Wei` counts a `U64`"), "{msg}");
    let (_, msg) = one(&with_wei("    let mut w: Wei = 5wei;\n    let n = 3;\n    w *= n;\n"));
    assert!(msg.contains("scales by a `U64`"), "{msg}");
    // a literal is a `U64` there, and a `U64` scales it
    let errs = located(&with_wei(
        "    let w: Wei = 5wei;\n    let n: U64 = 3;\n    let a = w * n + 2 * w;\n    let b = w / 2 or floor;\n    \
         let c: U64 = w / 1wei;\n    let mut d = w;\n    d *= 4;\n    let (whole, rest) = w.split(gwei);\n    \
         let e: U64 = whole;\n",
    ));
    assert!(errs.is_empty(), "{errs:?}");

    // an identity and a range take a `U64`, never an `Int`
    let (at, msg) = one(&with_wei("    let n = 3;\n    let c: Nonce = n;\n"));
    assert_eq!(at, "n");
    assert_eq!(
        msg,
        "`Int` is not `Nonce`: `Nonce` counts a `U64`, and an `Int` never converts implicitly to one: write \
         `Nonce(U64::from_int(n))`"
    );
    let (at, _) = one(&with_wei("    let n = 3;\n    let c = Nonce(n);\n"));
    assert_eq!(at, "Nonce(n)");
    let (_, msg) = one(&with_wei("    let c: Nonce = 3;\n    let i: Int = c;\n"));
    assert_eq!(
        msg,
        "`Nonce` is not an `Int`: it counts a `U64`, and the two never convert implicitly: write `U64(…)` for its \
         `U64`, then `U64::to_int(x) or ...`"
    );
    let (at, _) = one(&with_wei("    let c: Nonce = 3;\n    let i = Int(c);\n"));
    assert_eq!(at, "Int(c)");
    let errs = located(&with_wei(
        "    let c: Nonce = 18446744073709551615;\n    let u: U64 = U64(c);\n    let d = Nonce(7);\n    \
         let p: Port = 443;\n    let q: U64 = p + 1;\n    let ok = p == 443 && c != d;\n",
    ));
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn a_factor_is_held_to_the_integer_counted() {
    // one `b` is 10^19 `a`: a `U64` holds the factor, an `Int` does not
    let catalogue = "unit a;\nunit a9 = 1_000_000_000 a;\nunit b = 10_000_000_000 a9;\n";
    let u64_src = format!(
        "{catalogue}type Big = quantity U64 in a;\ntype Coarse = Big in b {{ round: floor; }}\n\
         fn widen(x: Coarse) -> Big {{\n    return x;\n}}\nfn main() {{}}\n"
    );
    assert!(located(&u64_src).is_empty(), "{:?}", located(&u64_src));
    let int_src = u64_src.replace("quantity U64 in a", "quantity Int in a");
    let (at, msg) = one(&int_src);
    assert_eq!(at, "x");
    assert_eq!(msg, "the conversion into `Big` is by the factor 10000000000000000000, which no `Int` holds");
}

#[test]
fn a_range_bound_is_a_value_of_the_integer_counted() {
    let (at, msg) = one("type Port = U64 { range: -1..65536; }\nfn main() {}\n");
    assert_eq!(at, "range: -1..65536");
    assert_eq!(
        msg,
        "type `Port`: `range: -1..65536` holds values no `U64` holds: a `U64` is never negative, and a range over it \
         bounds its values"
    );
    let (_, msg) = one("type Wide = Int { range: 0..9223372036854775809; }\nfn main() {}\n");
    assert_eq!(
        msg,
        "type `Wide`: `range: 0..9223372036854775809` holds values no `Int` holds: an `Int` is \
         -9223372036854775808 through 9223372036854775807, and a range over it bounds its values"
    );
    // a `U64`'s whole range, written inclusively
    assert!(located("type All = U64 { range: 0..=18446744073709551615; }\nfn main() {}\n").is_empty());
}

#[test]
fn a_literal_and_a_negation_are_held_to_the_integer_counted() {
    let (at, msg) = one(&with_wei("    let w: Wei = 20eth;\n"));
    assert_eq!(at, "20eth");
    assert_eq!(msg, "`20eth` as a count of `Wei` overflows a `U64`");
    assert!(located(&with_wei("    let w: Wei = 18eth;\n")).is_empty(), "18 eth is a U64's");
    let (at, msg) = one(&with_wei("    let c: Nonce = -1;\n"));
    assert_eq!(at, "-1");
    assert_eq!(msg, "`-1` is no `Nonce`: it counts a `U64`, which is 0 through 18446744073709551615");
    let (at, msg) = one(&with_wei("    let w: Wei = 5wei;\n    let x = -w;\n"));
    assert_eq!(at, "-w");
    assert_eq!(msg, "`-` of `Wei`, which counts a `U64`: a `U64` is never negative and has no unary `-`");
    let (_, msg) = one(&with_wei("    let p: Port = 80;\n    let x = -p;\n"));
    assert_eq!(msg, "`-` of `Port`, which counts a `U64`: a `U64` is never negative and has no unary `-`");
}

#[test]
fn a_quantity_literals_magnitude_is_a_u64s_and_its_count_the_integers() {
    assert!(located(&with_wei("    let w: Wei = 18446744073709551615wei;\n")).is_empty(), "the top of a U64");
    let (at, msg) = one("unit g;\ntype Mass = quantity Int in g;\nfn main() {\n    let m = 9223372036854775808g;\n}\n");
    assert_eq!(at, "9223372036854775808g");
    assert_eq!(msg, "`9223372036854775808g` as a count of `Mass` overflows an `Int`");
    let errs = parse_source("unit wei;\nfn main() {\n    let w = 18446744073709551616wei;\n}\n").expect_err("a lex error");
    assert_eq!(
        errs[0].message,
        "quantity literal 18446744073709551616wei: its magnitude is past the largest integer, 2^64 - 1 \
         (18446744073709551615, a `U64`'s max)"
    );
}
