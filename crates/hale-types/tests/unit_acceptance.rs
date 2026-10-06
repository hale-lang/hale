//! GH #1076, step U5: the committed form's example (hale-lang/hale#1212,
//! "The unit dialect, concretely") is the dialect's acceptance. Its stated
//! results are asserted by the acceptance programs
//! (`tests/hale/units_*_test.hl`); here are its errors, each line it
//! marks as a type error refused with its exact wording at the text of
//! its span, and the lines of its text as committed that an earlier step
//! decided otherwise, each with what the compiler says of it. Then the
//! law: no program with a narrowing and no policy checks, one program per
//! form a narrowing takes.
//!
//! The programs are built from plain strings, as `unit_quantities.rs`'s
//! are, so the corpus harvester (which reads raw-string programs) does
//! not take these negative programs into the corpus pins.

use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::parse_source;
use hale_types::typed_bodies::{ConversionKind, SiteKind};

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;

/// The committed form's declarations as the acceptance programs write
/// them (crates/hale-cli/tests/units/committed_form.hl says why each
/// departs from the text as committed), and the values its `main` reads.
const DECLS: &str = "\
unit B;
unit KiB = 1024 B;
unit MiB = 1024 KiB;
unit cent;
unit USD = 100 cent;
unit bp = 1 / 10000;
unit pct = 100 bp;
unit tick;
unit mK;
type ByteCount = quantity Int in B;
type Money = quantity Int in cent;
type Ratio = quantity Int in bp;
type Tick = quantity Int in tick;
type Price = point Tick;
type TempDelta = quantity Int in mK;
type Kelvin = point TempDelta;
type Celsius = point TempDelta { origin: 273_150 mK; }
type OrderId = distinct Int;
type SeqNo = distinct Int { range: 0..65536; }
type Session = distinct Int { range: 0..64; }
type Byte = Int { range: 0..256; }
type WireStamp = Time in us { round: floor; }
type Bucket = quantity Int in 100ms { round: floor; }
type Ledger = quantity Int in cent { round: half_even; }
type Header { send_ns: Int; send_us: Int; session: Int; }
";

/// What `main` binds before the line under test: the committed form's
/// `hdr`, `bid`, `ask`, `id`, `d`, `n`, `odd`.
const PRELUDE: &str = "\
    let hdr = Header { send_ns: 1_700_000_000_123_456_789, send_us: 1_700_000_000_123_400, session: 70 };
    let bid = Price(100tick);
    let ask = Price(103tick);
    let id = OrderId(7);
    let d = 3s + 500ms;
    let n: Int = d / 1ms;
    let odd = 1_234_567USD * 3bp;
";

fn program(extra: &str, body: &str) -> String {
    format!("{DECLS}{extra}fn main() {{\n{PRELUDE}{body}    println(f\"{{d}} {{n}} {{odd}} {{id}} {{bid}} {{ask}} {{hdr.session}}\");\n}}\n")
}

/// Every error of `src`, as (the text at its span, its message).
fn errors_of(src: &str) -> Vec<(String, String)> {
    let all = check_program(&parse_source(src).expect("parses"));
    all.iter().filter(|d| d.is_error()).map(|d| (d.span.slice(src).to_string(), d.message.clone())).collect()
}

fn errors(extra: &str, body: &str) -> Vec<(String, String)> {
    errors_of(&program(extra, body))
}

fn one(body: &str, at: &str, message: &str) {
    assert_eq!(errors("", body), [(at.to_string(), message.to_string())], "{body}");
}

#[test]
fn the_declarations_and_the_prelude_check_clean() {
    assert_eq!(errors("", ""), Vec::<(String, String)>::new());
}

// ---------------------------------------------------------------- the
// three lines the example marks as type errors

#[test]
fn price_plus_price_is_refused() {
    // let mid = (bid + ask) / 2;            // type error: Price + Price
    one(
        "    let mid = (bid + ask) / 2;\n",
        "bid + ask",
        "`Price` + `Price`: two points do not add; their difference is a quantity (`b - a`), and a point moves by a quantity (`a + d`)",
    );
}

#[test]
fn a_duration_plus_a_byte_count_is_refused() {
    // let bad  = 5ms + 4KiB;                // type error: different quantities
    one(
        "    let bad = 5ms + 4KiB;\n",
        "5ms + 4KiB",
        "`Duration` + `ByteCount`: different quantities, `Duration` and `ByteCount`; `+` holds within one",
    );
}

#[test]
fn an_identity_has_no_arithmetic() {
    // let nope = id + id;                   // type error: identity has no arithmetic
    one("    let nope = id + id;\n", "id + id", "`OrderId` is an identity; it has no arithmetic");
}

// ---------------------------------------------------------------- the
// text as committed, where an earlier step decided otherwise

#[test]
fn a_literal_with_a_space_before_its_unit_is_two_tokens() {
    // Decision 3: the magnitude and the unit are adjacent.
    let src = program("", "    let fee = 1_250_000 USD * 3bp;\n");
    let errs = parse_source(&src).err().expect("a parse error");
    let found: Vec<(&str, &str)> = errs.iter().map(|d| (d.span.slice(&src), d.message.as_str())).collect();
    assert_eq!(found, [("USD", "expected ;, got Ident(\"USD\")")], "at the unit");
}

#[test]
fn the_time_catalogue_and_its_two_types_are_the_stdlibs() {
    // U4: `ns` … `day`, `Duration` and `Time` are declared by std::time.
    // The committed form's lines declaring them, in a program, declare
    // them a second time.
    let src = "\
unit us = 1_000 ns;
unit ms = 1_000 us;
unit s = 1_000 ms;
unit min = 60 s;
type Duration = quantity Int in ns;
type Time = point Duration;
fn main() {
    println(\"declared\");
}
";
    let stdlib_has = |u: &str| {
        format!(
            "unit `{u}` is declared twice: the stdlib's time catalogue declares `{u}` (`std::time`), and a unit is one node of the catalogue; write `{u}` for that unit (`unit tick = 10 ms;` joins a unit of the program's to it), or give this one another name"
        )
    };
    assert_eq!(
        errors_of(src),
        [
            ("us".to_string(), stdlib_has("us")),
            ("ms".to_string(), stdlib_has("ms")),
            ("s".to_string(), stdlib_has("s")),
            ("min".to_string(), stdlib_has("min")),
            (
                "Duration".to_string(),
                "type `Duration`: the units of `ns` already have their quantity, `Duration` (the stdlib's, `std::time`): a component of the catalogue has one quantity, and every other type over it is a denomination of that one; write `type Duration = Duration in ns;`, or give it a `round:` policy".to_string()
            ),
        ]
    );
}

#[test]
fn an_identity_with_no_range_narrows_nothing_so_its_or_is_refused() {
    // U2: the committed form's `type SeqNo = distinct Int;` has no range,
    // so `SeqNo(n)` is total and `or wrap` has nothing to discharge.
    let src = "\
type SeqNo = distinct Int;
fn main() {
    let n = 3500;
    let seq = SeqNo(n) or wrap;
    println(f\"{seq}\");
}
";
    assert_eq!(
        errors_of(src),
        [("SeqNo(n)".to_string(), "`SeqNo` is not fallible (it returns `SeqNo`); drop the `or` clause".to_string())]
    );
}

#[test]
fn a_quantity_named_bytes_is_accepted_and_shadowed_by_the_buffer_type() {
    // Decision 10 renamed the committed form's `Bytes` to `ByteCount`: the
    // buffer type is `Bytes`. FINDING (U5, about U1/U3): the committed line
    // `type Bytes = quantity Int in B;` is accepted, and a type position
    // still reads `Bytes` as the buffer, so a quantity literal annotated
    // `Bytes` checks clean here, and `hale build` refuses it as a literal
    // with no row (codegen: "quantity literal `4B` has no required
    // `expression_typing` row"). Pinned as the check says it today.
    let src = "\
unit B;
type Bytes = quantity Int in B;
fn main() {
    let n: Bytes = 4B;
    println(f\"{len(n)}\");
}
";
    assert_eq!(errors_of(src), Vec::<(String, String)>::new());
}

#[test]
fn d_in_s_divides_by_a_billion() {
    // The example's comment says `q = 1000`, for a `Duration` counted in
    // ms. `Duration` counts nanoseconds (U4's literal rule: `500ms` is
    // 500,000,000 of it), so `d.in(s)` divides by 1,000,000,000.
    let src = program("", "    let whole = d.in(s) or 0;\n    let secs = d.in(s) or floor;\n");
    assert_eq!(errors_of(&src), Vec::<(String, String)>::new());
    let snapshot = Snapshot::from_program(parse_source(&src).expect("parses"), Vec::new(), Config::check(true, false))
        .unwrap_or_else(|_| panic!("loads"));
    let typed = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let mut casts: Vec<(usize, String, String)> = typed
        .conversion_sites()
        .filter(|(site, row)| matches!(site.kind, SiteKind::Cast(_)) && row.kind == ConversionKind::Narrowing)
        .map(|(_, row)| (row.span.start.as_usize(), row.span.slice(&src).to_string(), row.scale.as_ref().expect("a scale").factor.to_string()))
        .collect();
    casts.sort();
    let found: Vec<(&str, &str)> = casts.iter().map(|(_, at, f)| (at.as_str(), f.as_str())).collect();
    assert_eq!(found, [("d.in(s)", "1/1000000000"), ("d.in(s)", "1/1000000000")]);
}

// ---------------------------------------------------------------- the
// law: a narrowing with no policy does not check, whatever its form

/// A name for `Duration in s` with no policy, and a parameter and a field
/// of `Money`, which has none either.
const FORMS: &str = "\
type Seconds = Duration in s;
type Bill { amount: Money; }
fn pay(m: Money) -> Money {
    return m;
}
";

/// The law over `body`: refused, with every error at `at` and none
/// elsewhere.
fn refused(body: &str, at: &str) -> Vec<String> {
    let found = errors(FORMS, body);
    let elsewhere: Vec<&(String, String)> = found.iter().filter(|(text, _)| text != at).collect();
    assert!(!found.is_empty(), "a narrowing with no policy checks: {body}");
    assert!(elsewhere.is_empty(), "every error is at `{at}`: {found:#?}");
    found.into_iter().map(|(_, m)| m).collect()
}

#[test]
fn no_program_with_a_narrowing_and_no_policy_checks() {
    let forms: [(&str, &str, &str); 11] = [
        // `.in(u)`, a quantity's.
        ("    let x = d.in(s);\n", "d.in(s)", "`Duration in s` from `Duration` divides by 1,000,000,000: say what happens to the remainder: `or floor`, `or <value>`, `or raise`"),
        // A cast into a quantity with no `round:`.
        ("    let x = Seconds(d);\n", "Seconds(d)", "`Seconds` from `Duration` divides by 1,000,000,000: say what happens to the remainder: `or floor`, `or <value>`, `or raise`, or give `Seconds` a `round:` policy"),
        // A division by a literal, a declared quantity's and `Duration`'s.
        ("    let spread = ask - bid;\n    let half = spread / 2;\n", "spread / 2","`Tick` divided by 2 leaves a remainder: say what happens to the remainder: `or floor`, `or <value>`, `or raise`, or give `Tick` a `round:` policy"),
        ("    let half = d / 2;\n", "d / 2", "`Duration` divided by 2 leaves a remainder: say what happens to the remainder: `or floor`, `or <value>`, `or raise`"),
        // An implicit narrowing: a binding, an argument, a return, a field,
        // a compound assignment.
        ("    let bad: Money = odd;\n", "odd", "`Money` from `Money in 1/10000 cent` divides by 10,000: say what happens to the remainder: convert explicitly (`.in(u) or floor`, `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy"),
        ("    let p = pay(odd);\n", "odd", "`Money` from `Money in 1/10000 cent` divides by 10,000: say what happens to the remainder: convert explicitly (`.in(u) or floor`, `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy"),
        ("    let bill = Bill { amount: odd };\n", "odd", "`Money` from `Money in 1/10000 cent` divides by 10,000: say what happens to the remainder: convert explicitly (`.in(u) or floor`, `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy"),
        ("    let mut total = 0cent;\n    total += odd;\n", "odd", "`Money` from `Money in 1/10000 cent` divides by 10,000: say what happens to the remainder: convert explicitly (`.in(u) or floor`, `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy"),
        // A literal that is no whole count of the type it flows into.
        ("    let t: Seconds = 1500ms;\n", "1500ms", "`Seconds` from `Duration` divides by 1,000,000,000: say what happens to the remainder: convert explicitly (`.in(u) or floor`, `Seconds(…) or half_even`, `or <value>`, `or raise`), or give `Seconds` a `round:` policy"),
        // A range's narrowing: a cast, and an implicit one.
        ("    let sess = Session(hdr.session);\n", "Session(hdr.session)", "`Session(…)` narrows `Int` into `Session`'s range `0..64` and this conversion says nothing about a value outside it: write `or <fallback>` for a value to use instead, `or clamp` for the nearest bound, `or wrap` to wrap around the range, `or handler(err)` to deal with the `RangeError` here, or `or raise` to hand it to the caller"),
        ("    let sess: Session = n;\n", "n", "`Int` does not narrow to `Session` implicitly: write `Session(…) or …`, which says what becomes of a value outside `0..64`"),
    ];
    for (body, at, message) in forms {
        assert_eq!(refused(body, at), [message.to_string()], "{body}");
    }
    // A return: `owed` returns the product into `Money`.
    let found = errors(&format!("{FORMS}fn owed() -> Money {{\n    return 1_234_567USD * 3bp;\n}}\n"), "    let o = owed();\n");
    assert_eq!(
        found,
        [(
            "1_234_567USD * 3bp".to_string(),
            "`Money` from `Money in 1/10000 cent` divides by 10,000: say what happens to the remainder: convert explicitly (`.in(u) or floor`, `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy".to_string()
        )]
    );
    // And each of them checks once it says what becomes of the remainder.
    let discharged = "\
    let a = d.in(s) or floor;
    let b = Seconds(d) or 0s;
    let spread = ask - bid;
    let c = spread / 2 or floor;
    let e = d / 2 or ceil;
    let f: Ledger = odd;
    let g = pay(odd.in(cent) or half_even);
    let h = Bill { amount: odd.in(cent) or half_even };
    let i: Bucket = 1500ms;
    let j = Session(hdr.session) or clamp;
    let k = Session(n) or 0;
";
    assert_eq!(errors("type Seconds = Duration in s;\ntype Bill { amount: Money; }\nfn pay(m: Money) -> Money {\n    return m;\n}\n", discharged), Vec::<(String, String)>::new());
}
