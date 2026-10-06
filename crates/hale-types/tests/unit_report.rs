//! GH #1076 (U5): the report `hale check --units` prints
//! (`hale_types::unit_report`), read over a snapshot's unit rows and
//! typed bodies. The CLI's test pins the committed form's report whole;
//! these pin the fields one at a time: the width a range fits in, the
//! headroom at a denomination (a finer one holds less), a denomination
//! and a policy inherited along a chain (the stdlib's `Duration`
//! included), a literal converted at compile time, one line for a
//! default however many places evaluate it, and the one line of an empty
//! report.

use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::{parse_source, Span};
use hale_types::unit_report::{fits_in, render, report, Place, UnitReport};

fn report_of(src: &str) -> UnitReport {
    let program = parse_source(src).expect("parses");
    let snapshot = Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("loads"));
    let errors: Vec<String> = snapshot
        .demand_check()
        .map(|c| c.diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect())
        .unwrap_or_else(|b| b.because.iter().map(|d| d.message.clone()).collect());
    assert!(errors.is_empty(), "{errors:#?}");
    let units = snapshot.demand_units().unwrap_or_else(|_| panic!("unit rows"));
    let typed = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let place = |span: Span| {
        let (line, col) = span.line_col(src);
        Some(Place { at: format!("{line}:{col}"), text: hale_types::unit_report::collapsed(span.slice(src)) })
    };
    report(units, typed, &place)
}

#[test]
fn a_range_fits_the_narrowest_width_that_holds_it() {
    assert_eq!(fits_in(0, 256), "u8");
    assert_eq!(fits_in(0, 257), "u16");
    assert_eq!(fits_in(0, 65_536), "u16");
    assert_eq!(fits_in(1, 65_537), "u32");
    assert_eq!(fits_in(0, 1 << 32), "u32");
    assert_eq!(fits_in(0, (1 << 32) + 1), "u64");
    assert_eq!(fits_in(-5, 5), "i64", "a negative bound has only the signed width");
}

const CHAIN: &str = "\
unit cent;
unit USD = 100 cent;
unit bp = 1 / 10000;
type Money = quantity Int in cent;
type Rate = quantity Int in bp;
type Ledger = Money { round: half_even; }
type Wait = Duration { round: floor; }
type Bucket = Wait in 100ms;
type Small = Int { range: 0..300; }
type Window { b: Bucket = 150ms; }
fn amount(n: Int) -> Money { return n * 1cent; }
fn rate(n: Int) -> Rate { return n * 1bp; }
fn main() {
    let b: Bucket = 150ms;
    let w1 = Window { };
    let w2 = Window { };
    let paid: Ledger = amount(5) * rate(3) * rate(7);
    println(f\"{b} {w1.b} {w2.b} {paid}\");
}
";

#[test]
fn a_denomination_and_a_policy_are_read_along_the_chain() {
    let r = report_of(CHAIN);
    let wait = r.declarations.iter().find(|d| d.name == "Wait").expect("Wait");
    let (denomination, fixed) = wait.denomination.as_ref().expect("a denomination");
    assert_eq!(denomination, "ns");
    assert_eq!(fixed.text, "type Duration = quantity Int in ns", "the stdlib's declaration fixed it");
    assert!(fixed.at.starts_with("std::time, time.hl:"), "{}", fixed.at);
    assert_eq!(wait.of.as_deref(), Some("Duration"));
    let bucket = r.declarations.iter().find(|d| d.name == "Bucket").expect("Bucket");
    assert_eq!(bucket.denomination.as_ref().map(|(d, f)| (d.as_str(), f.at.as_str())), Some(("100 ms", "8:1")), "its own `in`");
    assert_eq!(bucket.policy.as_ref().map(|(p, f)| (*p, f.text.as_str())), Some(("floor", "type Wait = Duration { round: floor; }")));
    let small = r.declarations.iter().find(|d| d.name == "Small").expect("Small");
    assert_eq!((small.kind, small.of.as_deref(), small.fits_in), ("range", Some("Int"), Some("u16")));
}

#[test]
fn a_finer_denomination_has_less_headroom() {
    let r = report_of(CHAIN);
    let money = r.declarations.iter().find(|d| d.name == "Money").expect("Money");
    let h = money.headroom.as_ref().expect("headroom");
    assert_eq!((h.count.to_string(), h.unit.as_str()), ("9223372036854775807".to_string(), "cent"));
    assert_eq!(h.coarse.as_ref().map(|(c, u)| (c.to_string(), u.as_str())), Some(("92233720368547758".to_string(), "USD")));
    // Two ratio factors: the product counts 1/100000000 cent.
    let paid = r.narrowings.iter().find(|n| n.to == "Ledger").expect("the narrowing into Ledger");
    assert_eq!(paid.from, "Money in 1/100000000 cent");
    assert_eq!(paid.factor.as_deref(), Some("1/100000000"));
    let (of, h) = paid.headroom.as_ref().expect("the product's headroom");
    assert_eq!(of, "Money in 1/100000000 cent");
    assert_eq!((h.count.to_string(), h.unit.as_str()), ("92233720368".to_string(), "cent"));
}

#[test]
fn a_literal_is_converted_at_compile_time_and_a_default_is_one_line() {
    let text = render(&report_of(CHAIN));
    let narrowings = &text[text.find("narrowings:").expect("a narrowing section")..];
    assert_eq!(
        narrowings,
        "narrowings:\n\
         \n\
         10:27  150ms\n    Duration -> Bucket, factor 1/100000000\n    literal      : 1, converted at compile time\n    policy       : floor, the `round:` of `type Wait = Duration { round: floor; }` (7:1)\n\
         \n\
         14:21  150ms\n    Duration -> Bucket, factor 1/100000000\n    literal      : 1, converted at compile time\n    policy       : floor, the `round:` of `type Wait = Duration { round: floor; }` (7:1)\n\
         \n\
         17:24  amount(5) * rate(3) * rate(7)\n    Money in 1/100000000 cent -> Ledger, factor 1/100000000\n    policy       : half_even, the `round:` of `type Ledger = Money { round: half_even; }` (6:1)\n    headroom     : ±92233720368 cent (922337203 USD), of Money in 1/100000000 cent\n",
        "the field's default is evaluated twice and said once"
    );
}

#[test]
fn nothing_to_report_is_one_line() {
    let text = render(&report_of("fn main() {\n    let d = 1500ms;\n    println(f\"{d}\");\n}\n"));
    assert_eq!(text, "units: no quantity is declared\n");
}
