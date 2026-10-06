//! GH #1076, step U1: the unit dialect's declarations are rows, and the
//! laws judge them.
//!
//! `derive_unit_rows` makes one row per `unit` and scalar `type`
//! declaration and closes the catalogue from the equations; each law of
//! `unit_laws` is pinned here by its message, the source text at its
//! span, and its witness. Values of the new types are typed (U2, U3:
//! `unit_values.rs`, `unit_quantities.rs`); what remains here is where a
//! value of one may live, and what a cast's name means in a default.

use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::{parse_source, Diag, DiagKind};
use hale_types::placement::SiteUniverse;
use hale_types::unit_graph::{Denom, MachineRatio, Ratio};
use hale_types::units::{Node, RoundPolicy, ScalarKindRow, ScalarRow, UnitRows};
use num_bigint::BigInt;
use num_rational::BigRational;

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;

/// The committed example's declarations (the #1212 comment "The unit
/// dialect, concretely"), with its time units renamed (`ns` is `nsec`,
/// …: the stdlib's time catalogue declares those names, U4, and these
/// rows test a program's own catalogue beside it), the units it leaves
/// undeclared declared (`nsec`, `B`, `cent`, `mK`), `Duration`, `Time`
/// and `Bytes` renamed (the first two are the stdlib's, the third the
/// buffer type), `bp` defined against the number, and a `main` that
/// prints one line.
const EXAMPLE: &str = "\
unit usec = 1_000 nsec;
unit msec = 1_000 usec;
unit sec  = 1_000 msec;
unit minute = 60 sec;
unit nsec;

unit B;
unit KiB = 1024 B;
unit MiB = 1024 KiB;

unit cent;
unit USD = 100 cent;
unit bp = 1/10000;
unit pct = 100 bp;
unit tick;
unit mK;
unit K = 1_000 mK;

type Elapsed   = quantity Int in nsec;
type Instant   = point Elapsed;
type ByteCount = quantity Int in B;
type Money     = quantity Int in cent;
type Ratio     = quantity Int in bp;
type Tick      = quantity Int in tick;
type Price     = point Tick;

type TempDelta = quantity Int in mK;
type Kelvin    = point TempDelta;
type Celsius   = point TempDelta { origin: 273_150 mK; }

type OrderId   = distinct Int;
type SeqNo     = distinct Int;
type Session   = distinct Int { range: 0..64; }
type Byte      = Int { range: 0..256; }

type WireStamp = Instant in usec { round: floor; }
type Bucket    = quantity Int in 100 msec { round: floor; }
type Ledger    = quantity Int in cent { round: half_even; }

fn main() {
    println(\"declared\");
}
";

fn snapshot(src: &str) -> Snapshot {
    let program = parse_source(src).expect("parses");
    match Snapshot::from_program(program, Vec::new(), Config::check(true, false)) {
        Ok(s) => s,
        Err(_) => panic!("a bare program shapes"),
    }
}

fn rows(src: &str) -> UnitRows {
    snapshot(src).demand_units().expect("the rows read declarations only").clone()
}

fn diags(src: &str) -> Vec<Diag> {
    check_program(&parse_source(src).expect("parses"))
}

/// The source text a span covers.
fn at<'s>(src: &'s str, span: hale_syntax::Span) -> &'s str {
    span.slice(src)
}

/// The one error whose message starts with `head`.
fn the_error<'d>(all: &'d [Diag], head: &str) -> &'d Diag {
    let found: Vec<&Diag> = all.iter().filter(|d| d.message.starts_with(head)).collect();
    assert_eq!(found.len(), 1, "one `{head}` error: {:#?}", all.iter().map(|d| &d.message).collect::<Vec<_>>());
    assert_eq!(found[0].kind, DiagKind::Type);
    found[0]
}

/// A diagnostic's witness, as (the text at each step, its note).
fn witness<'s>(src: &'s str, d: &Diag) -> Vec<(&'s str, String)> {
    d.related.iter().map(|r| (at(src, r.span), r.label.clone())).collect()
}

fn unit(rows: &UnitRows, name: &str) -> usize {
    rows.units.iter().position(|u| u.name == name).unwrap_or_else(|| panic!("no unit `{name}`"))
}

fn scalar<'r>(rows: &'r UnitRows, name: &str) -> &'r ScalarRow {
    rows.scalars.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no type `{name}`"))
}

fn index(rows: &UnitRows, name: &str) -> usize {
    rows.scalars.iter().position(|s| s.name == name).unwrap_or_else(|| panic!("no type `{name}`"))
}

fn ratio(n: u64, d: u64) -> Ratio {
    Ratio::new(BigInt::from(n), BigInt::from(d)).unwrap()
}

#[test]
fn the_committed_example_is_rows_and_checks_clean() {
    let errors: Vec<String> = diags(EXAMPLE).into_iter().filter(Diag::is_error).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let r = rows(EXAMPLE);
    // The stdlib's time catalogue first (U4): its 7 units, 6 equations
    // and 2 scalars (`Duration`, `Time`), then the example's 15, 10, 17.
    assert_eq!(r.units.len(), 7 + 15);
    assert_eq!(r.equations.len(), 6 + 10);
    assert_eq!(r.scalars.len(), 2 + 17);
    assert!(r.cycles.is_empty());
    let catalogue = r.catalogue.as_ref().expect("the catalogue closes");

    // The components: time, bytes, money, the dimensionless one, ticks,
    // temperature.
    let c = |n: &str| r.units[unit(&r, n)].component;
    let groups: [&[&str]; 6] =
        [&["nsec", "usec", "msec", "sec", "minute"], &["B", "KiB", "MiB"], &["cent", "USD"], &["bp", "pct"], &["tick"], &["mK", "K"]];
    for g in groups {
        assert!(g.iter().all(|n| c(n) == c(g[0])), "{g:?}");
    }
    let mut distinct: Vec<usize> = groups.iter().map(|g| c(g[0])).collect();
    distinct.dedup();
    assert_eq!(distinct.len(), 6);
    for n in ["bp", "pct"] {
        assert!(r.units[unit(&r, n)].dimensionless, "{n}");
    }
    assert!(!r.units[unit(&r, "cent")].dimensionless);
    let bp = r.equations.iter().find(|e| e.unit == unit(&r, "bp")).unwrap();
    assert_eq!((bp.target, &bp.factor), (Some(Node::Pure), &ratio(1, 10_000)));

    // Each component's quantity, and the boundary denominations of it.
    for q in ["Elapsed", "ByteCount", "Money", "Ratio", "Tick", "TempDelta"] {
        let s = scalar(&r, q);
        assert_eq!((s.kind, s.principal, s.of, s.policy), (ScalarKindRow::Quantity, true, None, None), "{q}");
    }
    assert_eq!(scalar(&r, "Ratio").component, Some(c("bp")));
    let bucket = scalar(&r, "Bucket");
    assert_eq!((bucket.kind, bucket.principal), (ScalarKindRow::Quantity, false));
    assert_eq!(bucket.of, Some(index(&r, "Elapsed")));
    assert_eq!(bucket.policy, Some(RoundPolicy::Floor));
    let d = bucket.denomination.as_ref().unwrap();
    assert_eq!((d.unit, &d.multiple), (r.units[unit(&r, "msec")].site, &ratio(100, 1)));
    let elapsed = scalar(&r, "Elapsed").denomination.as_ref().unwrap();
    let factor = catalogue.factor(d, elapsed).unwrap();
    assert_eq!(factor, ratio(100_000_000, 1));
    assert_eq!(factor.to_machine(), Ok(MachineRatio { numerator: 100_000_000, denominator: 1 }));
    let ledger = scalar(&r, "Ledger");
    assert_eq!((ledger.of, ledger.policy), (Some(index(&r, "Money")), Some(RoundPolicy::HalfEven)));

    // Points: over their quantity, at its denomination unless they name
    // one; an origin as a count of the point's own denomination.
    let instant = scalar(&r, "Instant");
    assert_eq!((instant.kind, instant.of), (ScalarKindRow::Point, Some(index(&r, "Elapsed"))));
    assert_eq!(instant.denomination.as_ref(), Some(elapsed));
    let stamp = scalar(&r, "WireStamp");
    assert_eq!((stamp.kind, stamp.of, stamp.policy), (ScalarKindRow::Point, Some(index(&r, "Instant")), Some(RoundPolicy::Floor)));
    assert_eq!(stamp.denomination.as_ref().unwrap().unit, r.units[unit(&r, "usec")].site);
    assert_eq!(stamp.component, Some(c("nsec")));
    assert_eq!(scalar(&r, "Price").of, Some(index(&r, "Tick")));
    assert_eq!(scalar(&r, "Celsius").origin, Some(BigRational::from_integer(BigInt::from(273_150))));
    assert_eq!(scalar(&r, "Kelvin").origin, None);

    // Identities and ranges.
    let session = scalar(&r, "Session");
    assert_eq!((session.kind, session.range, session.denomination.is_none()), (ScalarKindRow::Identity, Some((0, 64)), true));
    assert_eq!((scalar(&r, "OrderId").kind, scalar(&r, "OrderId").range), (ScalarKindRow::Identity, None));
    let byte = scalar(&r, "Byte");
    assert_eq!((byte.kind, byte.range, byte.of), (ScalarKindRow::Range, Some((0, 256)), None));
}

/// An origin written in a coarser unit than its point is counted in the
/// point's denomination; `..=` stores its bound plus one.
#[test]
fn an_origin_is_a_count_of_its_points_denomination_and_an_inclusive_range_is_half_open() {
    let src = "unit mK;\nunit K = 1_000 mK;\n\
               type TempDelta = quantity Int in mK;\n\
               type Celsius = point TempDelta { origin: 273 K; }\n\
               type Cold = point TempDelta in K { origin: -5 K; }\n\
               type Small = distinct Int { range: -3..=3; }\n";
    let r = rows(src);
    let count = |n: i64| Some(BigRational::from_integer(BigInt::from(n)));
    assert_eq!(scalar(&r, "Celsius").origin, count(273_000));
    assert_eq!(scalar(&r, "Cold").origin, count(-5));
    assert_eq!(scalar(&r, "Small").range, Some((-3, 4)));
}

// Law 1: a unit is declared once.

#[test]
fn a_unit_declared_twice_is_refused_at_the_second_naming_the_first() {
    let src = "unit cent;\nunit cent;\nfn main() { }\n";
    let all = diags(src);
    let d = the_error(&all, "unit `cent`");
    assert_eq!(
        d.message,
        "unit `cent` is declared twice: a unit is one node of the catalogue, so it is declared once, with at \
         most one equation; remove this declaration or give the unit another name"
    );
    assert_eq!(d.span.start.as_usize(), src.rfind("cent").unwrap());
    assert_eq!(witness(src, d), [("cent", "`cent` is first declared here".to_string())]);
    assert_eq!(d.related[0].span.start.as_usize(), src.find("cent").unwrap());
    assert!(rows(src).catalogue.is_none(), "a catalogue law failed");
}

// Law 2: an equation, a denomination and an origin name declared units.

#[test]
fn an_undeclared_unit_is_refused_at_the_name_with_the_nearest_suggested() {
    let src = "unit cent;\nunit mK;\nunit USD = 100 cnet;\n\
               type Money = quantity Int in cents;\n\
               type TempDelta = quantity Int in mK;\n\
               type Celsius = point TempDelta { origin: 273 K; }\n\
               type Wait = quantity Int in ms;\n";
    let all = diags(src);
    let d = the_error(&all, "unit `USD`");
    assert_eq!(
        d.message,
        "unit `USD`: its equation names `cnet`, which no `unit` declares: declare it (`unit cnet;`); did you \
         mean `cent`?"
    );
    assert_eq!(at(src, d.span), "cnet");
    assert!(d.related.is_empty());
    let d = the_error(&all, "type `Money`");
    assert_eq!(
        d.message,
        "type `Money`: its denomination names `cents`, which no `unit` declares: declare it (`unit cents;`); \
         did you mean `cent`?"
    );
    assert_eq!(at(src, d.span), "cents");
    let d = the_error(&all, "type `Celsius`");
    assert_eq!(
        d.message,
        "type `Celsius`: its origin names `K`, which no `unit` declares: declare it (`unit K;`); did you mean `mK`?"
    );
    assert_eq!(at(src, d.span), "K");
    // U4: `ms` is the stdlib's, whose component has its quantity
    // (law 4); the stdlib's declaration is no place in the program.
    let d = the_error(&all, "type `Wait`");
    assert_eq!(
        d.message,
        "type `Wait`: the units of `ms` already have their quantity, `Duration` (the stdlib's, `std::time`): a \
         component of the catalogue has one quantity, and every other type over it is a denomination of that one; \
         write `type Wait = Duration in ms;`, or give it a `round:` policy"
    );
    assert_eq!(at(src, d.span), "Wait");
    assert!(d.related.is_empty());
    assert!(rows(src).catalogue.is_none(), "an equation names no declared unit");
}

// Law 3: the catalogue closes.

#[test]
fn an_inconsistent_cycle_is_refused_with_both_paths() {
    // b = 6 a and c = 5 b make one c 30 a, so one a is 1/30 c; the
    // program says 1/31.
    let src = "unit a = 1/31 c;\nunit b = 6 a;\nunit c = 5 b;\nfn main() { }\n";
    let all = diags(src);
    let errors: Vec<&Diag> = all.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "{all:#?}");
    let d = errors[0];
    let r = rows(src);
    assert!(r.catalogue.is_none());
    assert_eq!(r.cycles.len(), 1);
    // The closure blames the equation that closes the cycle (here `c`'s),
    // and the witness is the other path, each step at its declaration.
    assert_eq!(at(src, d.span), "5 b");
    assert_eq!(
        d.message,
        "unit `c`: `unit c = 5 b` makes one `c` 5 `b`, and the other equations make it 31/6 `b`: every cycle of \
         equations multiplies to one, so one equation on this cycle is wrong; correct it (to `unit c = 31/6 b;` \
         if it is this one)"
    );
    assert_eq!(
        witness(src, d),
        [
            ("unit a = 1/31 c;", "`unit a = 1/31 c`: one `c` is 31 `a`".to_string()),
            ("unit b = 6 a;", "`unit b = 6 a`: one `a` is 1/6 `b`".to_string()),
        ]
    );
}

// Law 4: one quantity per component.

#[test]
fn a_second_quantity_in_a_component_is_refused_naming_the_first() {
    let src = "unit g;\nunit kg = 1000 g;\nunit cent;\n\
               type Mass = quantity Int in g;\n\
               type Weight = quantity Int in kg;\n\
               type Heavy = quantity Int in kg { round: floor; }\n\
               type Bad = Mass in cent;\n";
    let all = diags(src);
    let d = the_error(&all, "type `Weight`");
    assert_eq!(
        d.message,
        "type `Weight`: the units of `kg` already have their quantity, `Mass`: a component of the catalogue \
         has one quantity, and every other type over it is a denomination of that one; write `type Weight = \
         Mass in kg;`, or give it a `round:` policy"
    );
    assert_eq!(at(src, d.span), "Weight");
    assert_eq!(witness(src, d), [("Mass", "`Mass` is that component's quantity, declared with no policy".to_string())]);
    let d = the_error(&all, "type `Bad`");
    assert_eq!(
        d.message,
        "type `Bad`: `cent` is not a unit of `Mass`'s component, so `Bad` cannot be denominated in it: a \
         refinement of a quantity is a denomination of that quantity"
    );
    assert_eq!(at(src, d.span), "cent");
    // A boundary denomination is no second quantity: `Heavy` is `Mass`'s.
    assert!(all.iter().all(|d| !d.message.starts_with("type `Heavy`")), "{all:#?}");
    let r = rows(src);
    assert_eq!(scalar(&r, "Heavy").of, Some(index(&r, "Mass")));
    assert_eq!(scalar(&r, "Bad").of, Some(index(&r, "Mass")));
}

#[test]
fn boundary_denominations_with_no_quantity_are_not_refused_here() {
    let src = "unit msec;\ntype Bucket = quantity Int in 100 msec { round: floor; }\nfn main() { }\n";
    assert!(diags(src).iter().all(|d| !d.is_error()), "{:#?}", diags(src));
    let r = rows(src);
    assert_eq!((scalar(&r, "Bucket").principal, scalar(&r, "Bucket").of), (false, None));
}

// Law 5: a quantity counts an `Int` and names its denomination.

#[test]
fn a_quantity_of_no_int_or_no_denomination_says_what_to_write() {
    let src = "unit g;\ntype F = quantity Float in g;\ntype N = quantity Int;\ntype I = Int in g;\n";
    let all = diags(src);
    let d = the_error(&all, "type `F`");
    assert_eq!(d.message, "type `F`: a quantity counts an `Int`, and `Float` is not one: write `quantity Int in g`");
    assert_eq!(at(src, d.span), "Float");
    let d = the_error(&all, "type `N`");
    assert_eq!(
        d.message,
        "type `N`: a quantity names its denomination, the unit it counts: write `quantity Int in <unit>` with a \
         declared unit"
    );
    assert_eq!(at(src, d.span), "N");
    let d = the_error(&all, "type `I`");
    assert_eq!(d.message, "type `I`: `Int in g` is a quantity without its word: write `quantity Int in g`");
    assert_eq!(at(src, d.span), "Int");
}

// Law 6: a point is over a quantity; an origin is a point's, in its
// component.

#[test]
fn a_point_over_no_quantity_and_a_misplaced_origin_are_refused() {
    let src = "unit g;\nunit cent;\n\
               type Mass = quantity Int in g;\n\
               type At = point Mass;\n\
               type P = point Int;\n\
               type Q = point At;\n\
               type S = distinct Int { origin: 5 g; }\n\
               type C = point Mass { origin: 3 cent; }\n\
               type D = point Mass in cent;\n";
    let all = diags(src);
    let d = the_error(&all, "type `P`");
    assert_eq!(d.message, "type `P`: a point is over a quantity, and `Int` is a primitive");
    assert_eq!(at(src, d.span), "Int");
    let d = the_error(&all, "type `Q`");
    assert_eq!(
        d.message,
        "type `Q`: a point is over a quantity, and `At` is a point; to refine the point, drop the word: `type Q \
         = At in <unit> { … }`"
    );
    let d = the_error(&all, "type `S`");
    assert_eq!(d.message, "type `S`: `origin:` places a point's zero, and `S` is an identity, not a point: remove it");
    assert_eq!(at(src, d.span), "origin: 5 g");
    let d = the_error(&all, "type `C`");
    assert_eq!(
        d.message,
        "type `C`: the origin is in `cent`, which is not a unit of `Mass`'s component: an origin is a count of the \
         point's own quantity"
    );
    assert_eq!(at(src, d.span), "cent");
    let d = the_error(&all, "type `D`");
    assert_eq!(
        d.message,
        "type `D`: `cent` is not a unit of `Mass`'s component, so the point cannot be denominated in it: a point \
         is denominated in a unit of its quantity"
    );
    assert_eq!(at(src, d.span), "cent");
}

// Law 7: `round:` and `range:`.

#[test]
fn round_and_range_clauses_that_mean_nothing_are_refused() {
    let src = "unit g;\n\
               type Mass = quantity Int in g { round: flor; }\n\
               type Id = distinct Int { round: floor; }\n\
               type Byte = Int { range: 0..256; }\n\
               type Wide = Byte { range: 0..300; }\n\
               type None = Int { range: 5..5; }\n\
               type Odd = Int { range: 0..N; }\n";
    let all = diags(src);
    let d = the_error(&all, "type `Mass`");
    assert_eq!(
        d.message,
        "type `Mass`: `flor` is not a rounding policy: `round:` names `floor`, `ceil`, `trunc`, `half_even` or \
         `half_up`; did you mean `floor`?"
    );
    assert_eq!(at(src, d.span), "flor");
    let d = the_error(&all, "type `Id`");
    assert_eq!(
        d.message,
        "type `Id`: `round:` is how a narrowing into a quantity or a point discards, and `Id` is an identity: \
         remove it"
    );
    let d = the_error(&all, "type `Wide`");
    assert_eq!(
        d.message,
        "type `Wide`: `range: 0..300` is not inside `Byte`'s `0..256`: a refinement narrows its parent's range, \
         never widens it"
    );
    assert_eq!(at(src, d.span), "range: 0..300");
    assert_eq!(witness(src, d), [("range: 0..256", "`Byte`'s range is `0..256`".to_string())]);
    let d = the_error(&all, "type `None`");
    assert_eq!(
        d.message,
        "type `None`: `range: 5..5` holds no value: its lower bound is below its upper (and `..=` includes the \
         upper)"
    );
    let d = the_error(&all, "type `Odd`");
    assert_eq!(
        d.message,
        "type `Odd`: a bound of `range:` is an integer literal (`0`, `-5`, `1_000`), and this one is not"
    );
    assert_eq!(at(src, d.span), "N");
}

// Law 8: an identity is `distinct Int`; a range refines `Int`, an
// identity or a range.

#[test]
fn an_identity_over_no_int_and_a_range_over_no_int_are_refused() {
    let src = "unit g;\n\
               type F = distinct Float;\n\
               type G = distinct Int in g;\n\
               type R = Float { range: 0..1; }\n\
               type Session = distinct Int;\n\
               type S = Session in g { range: 0..3; }\n\
               type Plain = Int { round: floor; }\n";
    let all = diags(src);
    let plain: Vec<&String> =
        all.iter().filter(|d| d.message.starts_with("type `Plain`: a refinement of `Int`")).map(|d| &d.message).collect();
    assert_eq!(
        plain,
        ["type `Plain`: a refinement of `Int` is a range, and states it: write `{ range: LO..HI; }`, or `type Plain \
          = Int;` for another name of `Int`"]
    );
    let d = the_error(&all, "type `F`");
    assert_eq!(d.message, "type `F`: an identity is `distinct Int`, and `Float` is not `Int`: write `distinct Int`");
    assert_eq!(at(src, d.span), "Float");
    let d = the_error(&all, "type `G`");
    assert_eq!(d.message, "type `G`: an identity counts nothing, so it has no denomination: remove `in g`");
    let d = the_error(&all, "type `R`");
    assert_eq!(
        d.message,
        "type `R`: a range type refines `Int`, an identity or another range, and `Float` is none of them"
    );
    assert_eq!(at(src, d.span), "Float");
    let d = the_error(&all, "type `S`");
    assert_eq!(d.message, "type `S`: a range counts nothing, so it has no denomination: remove `in g`");
}

// Law 9: a refinement refines a scalar or `Int`.

#[test]
fn a_refinement_of_a_struct_of_nothing_or_of_itself_is_refused() {
    let src = "unit g;\n\
               type Order { id: Int; }\n\
               type Mass = quantity Int in g;\n\
               type O = Order { range: 0..3; }\n\
               type M = point Mas;\n\
               type A = B { range: 0..3; }\n\
               type B = A { range: 0..2; }\n";
    let all = diags(src);
    let d = the_error(&all, "type `O`");
    assert_eq!(
        d.message,
        "type `O`: a refinement refines a quantity, a point, an identity, a range or `Int` (with a `range:`), and \
         `Order` is a struct"
    );
    assert_eq!(at(src, d.span), "Order");
    let d = the_error(&all, "type `M`");
    assert_eq!(d.message, "type `M`: `Mas` is not a declared type; did you mean `Mass`?");
    assert_eq!(at(src, d.span), "Mas");
    let d = the_error(&all, "type `A`");
    assert_eq!(
        d.message,
        "type `A` refines itself (`A` refines `B` refines `A`): a chain of refinements ends at a quantity, a \
         point, an identity or `Int`"
    );
    assert_eq!(at(src, d.span), "B");
}

// Law 1 against the stdlib's time catalogue (U4): a program's unit of a
// name the stdlib declares is a second declaration of that unit, whose
// first is in no file of the program; one written against a stdlib unit
// joins its component.

#[test]
fn a_unit_the_stdlib_declares_is_declared_twice_and_one_against_it_joins() {
    let src = "unit ms;\nfn main() { }\n";
    let all = diags(src);
    let d = the_error(&all, "unit `ms`");
    assert_eq!(
        d.message,
        "unit `ms` is declared twice: the stdlib's time catalogue declares `ms` (`std::time`), and a unit is one \
         node of the catalogue; write `ms` for that unit (`unit tick = 10 ms;` joins a unit of the program's to \
         it), or give this one another name"
    );
    assert_eq!(d.span.start.as_usize(), src.find("ms").unwrap());
    assert!(d.related.is_empty(), "the stdlib's declaration is no place in the program: {:?}", d.related);

    let src = "unit tick = 10 ms;\nfn main() { }\n";
    assert!(diags(src).iter().all(|d| !d.is_error()), "{:?}", diags(src));
    let r = rows(src);
    let tick = unit(&r, "tick");
    let ms = unit(&r, "ms");
    assert_eq!(r.units[tick].component, r.units[ms].component, "`tick` is in the time component");
    assert_eq!(r.units[ms].site.universe, SiteUniverse::StdlibAnalysis);
    assert_eq!(r.units[tick].site.universe, SiteUniverse::User);
    let to_ns = r.catalogue.as_ref().unwrap().factor(
        &Denom { unit: r.units[tick].site, multiple: ratio(1, 1) },
        &Denom { unit: r.units[unit(&r, "ns")].site, multiple: ratio(1, 1) },
    );
    assert_eq!(to_ns, Some(ratio(10_000_000, 1)));
}

// Law 11: no unit takes a name the lexer reads before a unit (U4): `d`,
// the Decimal literal's suffix, and a Float's exponent. And law 2's hint
// for a retired time unit: `m` is `min` now, whatever is nearest by
// spelling.

#[test]
fn a_unit_named_like_a_literal_suffix_is_refused_at_its_name() {
    let src = "unit d;\nunit e5;\nunit E2x;\nunit dd;\nunit e;\nfn main() { }\n";
    let all = diags(src);
    let d = the_error(&all, "unit `d`");
    assert_eq!(
        d.message,
        "unit `d`: a unit named `d` collides with the Decimal literal's suffix: `3d` is the Decimal `3`; give it \
         another name"
    );
    assert_eq!(at(src, d.span), "d");
    assert_eq!(d.span.start.as_usize(), src.find("d;").unwrap());
    let d = the_error(&all, "unit `e5`");
    assert_eq!(
        d.message,
        "unit `e5`: a unit named `e5` collides with a Float literal's exponent: `3e5` reads as the Float `3e5`; \
         give it another name"
    );
    assert_eq!(at(src, d.span), "e5");
    let d = the_error(&all, "unit `E2x`");
    assert_eq!(
        d.message,
        "unit `E2x`: a unit named `E2x` collides with a Float literal's exponent: `3E2x` reads as the Float `3E2`; \
         give it another name"
    );
    // `3dd` and `3e` are quantity literals: the lexer reads `d` only
    // alone, and `e` as an exponent only before a digit or a sign.
    assert_eq!(all.iter().filter(|d| d.is_error()).count(), 3, "{all:#?}");
    assert!(rows(src).catalogue.is_some(), "law 11 is no catalogue law");

    let src = "type Wait = quantity Int in m;\nfn main() { }\n";
    let all = diags(src);
    let d = the_error(&all, "type `Wait`");
    assert_eq!(d.message, "type `Wait`: its denomination names `m`, which no `unit` declares: declare it (`unit m;`); minutes are `min`");
    assert_eq!(at(src, d.span), "m");
}

// Law 10: every unit written against a number is in one dimensionless
// component.

#[test]
fn units_written_against_a_number_share_one_component() {
    let src = "unit bp = 1/10000;\nunit pct = 1/100;\nunit dozen = 12;\nunit tick;\n\
               type Ratio = quantity Int in bp;\n\
               type Share = quantity Int in pct;\n";
    let r = rows(src);
    let c = |n: &str| r.units[unit(&r, n)].component;
    assert_eq!((c("bp"), c("pct"), c("dozen")), (c("bp"), c("bp"), c("bp")));
    assert!(["bp", "pct", "dozen"].iter().all(|n| r.units[unit(&r, n)].dimensionless));
    assert!(!r.units[unit(&r, "tick")].dimensionless);
    let catalogue = r.catalogue.as_ref().unwrap();
    let pct = scalar(&r, "Share").denomination.as_ref().unwrap();
    let bp = scalar(&r, "Ratio").denomination.as_ref().unwrap();
    assert_eq!(catalogue.factor(pct, bp), Some(ratio(100, 1)));
    // One component, so one quantity: `Share` is a second.
    let all = diags(src);
    let d = the_error(&all, "type `Share`");
    assert_eq!(witness(src, d), [("Ratio", "`Ratio` is that component's quantity, declared with no policy".to_string())]);
}

// Where a value of a new type lives (U3 deleted the not-yet boundary).

/// Every place U1's boundary refused a quantity (a field, an alias's
/// target, a parameter, a return, a generic argument, a binding, a
/// literal, a cast) now holds one: the program checks clean.
#[test]
fn a_quantity_is_a_value_wherever_a_value_lives() {
    let src = "unit cent;\n\
               type Money = quantity Int in cent;\n\
               type Order { total: Money; }\n\
               type Wallet = Money;\n\
               type Pair<T> { a: T; b: T; }\n\
               topic Paid {\n    payload: Order;\n    subject: \"paid\";\n}\n\
               fn pay(m: Money) -> Money {\n    return m;\n}\n\
               fn both(p: Pair<Money>) {\n}\n\
               fn main() {\n    let x: Money = pay(1cent);\n    let f = 3cent;\n    let m = Money(f);\n    \
               let w: Wallet = m;\n    let o = Order { total: x + w };\n    println(o.total);\n}\n";
    let errors: Vec<String> = diags(src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "checks clean: {errors:#?}");
}

/// A parameter's default is typed at its declaration, a struct field's
/// at each literal that leaves it, in a walk whose findings the check
/// discards but for the quantity rules' own: a quantity literal of a unit
/// with no quantity, and a quantity's cast of an `Int`, are each one
/// error at their place. A literal into its field's quantity converts.
#[test]
fn a_quantity_value_in_a_default_is_judged_where_it_is_written() {
    let no_quantity = "`3cent`: the units of `cent` have no quantity: declare one (`type T = quantity Int in cent;`)";
    let cases = [
        ("unit cent;\nfn take(n: Int = 3cent) {\n    println(n);\n}\nfn main() {\n    take();\n}\n", "3cent", no_quantity),
        ("unit cent;\ntype S { n: Int = 3cent; }\nfn main() {\n    let s = S {};\n    println(s.n);\n}\n", "3cent", no_quantity),
        (
            "unit cent;\ntype Money = quantity Int in cent;\ntype S { n: Int = Money(1); }\n\
             fn main() {\n    let s = S {};\n    println(s.n);\n}\n",
            "Money(1)",
            "`Money(…)` of an `Int`: a count becomes a quantity by a unit (`n * 1cent`)",
        ),
    ];
    for src in [
        "type ItemId = distinct Int;\ntype S { n: ItemId = ItemId(1); }\nfn main() {\n    let s = S {};\n    println(s.n);\n}\n",
        "unit cent;\nunit USD = 100 cent;\ntype Money = quantity Int in cent;\ntype S { n: Money = 3USD; }\n\
         fn main() {\n    let s = S {};\n    println(s.n);\n}\n",
    ] {
        let errors: Vec<String> = diags(src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
        assert!(errors.is_empty(), "a conversion: {errors:#?}\n{src}");
    }
    for (src, place, message) in cases {
        let all = diags(src);
        let errors: Vec<&Diag> = all.iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), 1, "one error: {:#?}\n{src}", errors.iter().map(|d| &d.message).collect::<Vec<_>>());
        assert_eq!(at(src, errors[0].span), place, "{src}");
        assert_eq!(errors[0].message, message);
    }
}

/// What a cast is when the callee is the declaration: `Money(1)` of an
/// `Int` is refused, a count becoming a quantity by a unit. A local, a
/// parameter or a fn of the name is what the name means at the call, as
/// it is for every other name, and an alias is not a unit-dialect type.
/// An identity's cast of an `Int` is a conversion.
#[test]
fn a_call_through_a_name_that_shadows_a_unit_type_is_no_cast() {
    let prelude = "unit cent;\ntype Money = quantity Int in cent;\nfn id(n: Int) -> Int {\n    return n;\n}\n";
    let shadowed = [
        "fn main() {\n    let Money = id;\n    println(Money(1));\n}\n",
        "fn apply(Money: fn(Int) -> Int) -> Int {\n    return Money(1);\n}\nfn main() {\n    println(apply(id));\n}\n",
    ];
    for body in shadowed {
        for ty in ["quantity Int in cent", "distinct Int"] {
            let src = format!("{prelude}{body}").replace("quantity Int in cent", ty);
            let errors: Vec<String> = diags(&src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
            assert!(errors.is_empty(), "the local is the callee: {errors:#?}\n{src}");
        }
    }
    let cast = format!("{prelude}fn main() {{\n    println(Money(1));\n}}\n");
    let all = diags(&cast);
    let errors: Vec<&Diag> = all.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "one error: {:#?}", errors.iter().map(|d| &d.message).collect::<Vec<_>>());
    assert_eq!(at(&cast, errors[0].span), "Money(1)");
    assert_eq!(errors[0].message, CAST_OF_AN_INT);
    for (ty, what) in [("Int", "an alias is no unit-dialect type"), ("distinct Int", "an identity's cast is a conversion")] {
        let other = cast.replace("quantity Int in cent", ty);
        let errors: Vec<String> = diags(&other).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
        assert!(errors.is_empty(), "{what}: {errors:#?}");
    }
}

/// The refusal of `Money(1)`, a quantity's cast of an `Int`.
const CAST_OF_AN_INT: &str = "`Money(…)` of an `Int`: a count becomes a quantity by a unit (`n * 1cent`)";

/// A struct field's default is evaluated at each literal that leaves the
/// field, in the literal's scope, so what a cast's name in it means is
/// that scope's: a local or a parameter of the constructing fn is the
/// callee. With nothing shadowing it, the cast is one error at the
/// default, however many literals leave the field; a default no literal
/// leaves is never evaluated.
#[test]
fn a_cast_in_a_struct_default_is_judged_where_the_default_is_evaluated() {
    let prelude =
        "unit cent;\ntype Money = quantity Int in cent;\ntype S { n: Int = Money(1); }\nfn id(n: Int) -> Int {\n    return n;\n}\n";
    let shadowed = [
        "fn main() {\n    let Money = id;\n    let s = S {};\n    println(s.n);\n}\n",
        "fn make(Money: fn(Int) -> Int) -> Int {\n    let s = S {};\n    return s.n;\n}\nfn main() {\n    println(make(id));\n}\n",
    ];
    for body in shadowed {
        let src = format!("{prelude}{body}");
        let errors: Vec<String> = diags(&src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
        assert!(errors.is_empty(), "the caller's name is the callee: {errors:#?}\n{src}");
    }
    let unshadowed = format!(
        "{prelude}fn other() -> Int {{\n    let s = S {{}};\n    return s.n;\n}}\nfn main() {{\n    let s = S {{}};\n    println(s.n + other());\n}}\n"
    );
    let all = diags(&unshadowed);
    let errors: Vec<&Diag> = all.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "one error: {:#?}", errors.iter().map(|d| &d.message).collect::<Vec<_>>());
    let default = unshadowed.find("Money(1)").expect("the default");
    assert_eq!(errors[0].span.start.as_usize(), default, "at the default's `Money(1)`");
    assert_eq!(at(&unshadowed, errors[0].span), "Money(1)");
    assert_eq!(errors[0].message, CAST_OF_AN_INT);
    let written = format!("{prelude}fn main() {{\n    let s = S {{ n: 2 }};\n    println(s.n);\n}}\n");
    let errors: Vec<String> = diags(&written).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "a default no literal leaves is never evaluated: {errors:#?}");
    let alias = unshadowed.replace("quantity Int in cent", "Int");
    let errors: Vec<String> = diags(&alias).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "an alias is no unit-dialect type: {errors:#?}");
}

/// The casts of an `Int` refused among `src`'s errors, with the source
/// position and text at each, and every other error.
fn casts_and_rest(src: &str) -> (Vec<(usize, String)>, Vec<String>) {
    let all = diags(src);
    let (casts, rest): (Vec<&Diag>, Vec<&Diag>) =
        all.iter().filter(|d| d.is_error()).partition(|d| d.message == CAST_OF_AN_INT);
    (
        casts.iter().map(|d| (d.span.start.as_usize(), at(src, d.span).to_string())).collect(),
        rest.iter().map(|d| d.message.clone()).collect(),
    )
}

/// An omitted struct default is typed where it is evaluated, at the
/// literal that leaves it, with every scope it opens, the parameter
/// defaults its calls leave and the field defaults its own literals
/// leave: a name it binds itself, or its caller binds, is the callee; a
/// cast with nothing shadowing it is one error at its place, wherever the
/// nesting reaches it; and a default whose literal leaves the same field
/// is not entered again.
#[test]
fn an_omitted_struct_default_is_typed_where_it_is_evaluated() {
    let prelude = "unit cent;\ntype Money = quantity Int in cent;\nfn id(n: Int) -> Int {\n    return n;\n}\n";
    let clean = [
        // The default's own block binds the name.
        "type S { n: Int = {\n    let Money = id;\n    Money(1)\n}; }\nfn main() {\n    let s = S {};\n    println(s.n);\n}\n",
        // A parameter default the field's default leaves, in the caller's scope.
        "fn take(n: Int = Money(1)) -> Int {\n    return n;\n}\ntype S { n: Int = take(); }\n\
         fn main() {\n    let Money = id;\n    let s = S {};\n    println(s.n);\n}\n",
        // A nested literal's omitted field, in the caller's scope.
        "type Inner { n: Int = Money(1); }\ntype Outer { inner: Inner = Inner {}; }\n\
         fn main() {\n    let Money = id;\n    let o = Outer {};\n    println(o.inner.n);\n}\n",
    ];
    for body in clean {
        let src = format!("{prelude}{body}");
        let (casts, rest) = casts_and_rest(&src);
        assert!(casts.is_empty() && rest.is_empty(), "checks clean: {casts:?} {rest:#?}\n{src}");
    }
    let refused = [
        "type Inner { n: Int = Money(1); }\ntype Outer { inner: Inner = Inner {}; }\n\
         fn main() {\n    let o = Outer {};\n    println(o.inner.n);\n}\n",
    ];
    for body in refused {
        let src = format!("{prelude}{body}");
        let (casts, rest) = casts_and_rest(&src);
        let place = src.find("Money(1)").expect("the cast");
        assert_eq!(casts, [(place, "Money(1)".to_string())], "one error, at the cast\n{src}");
        assert!(rest.is_empty(), "the cast is the only error: {rest:#?}\n{src}");
    }
    // A parameter's default is typed at its declaration, where the name
    // is the declaration: one error there, whoever leaves it.
    let through_fn = format!(
        "{prelude}fn take(n: Int = Money(1)) -> Int {{\n    return n;\n}}\ntype S {{ n: Int = take(); }}\n\
         fn main() {{\n    let s = S {{}};\n    println(s.n);\n}}\n"
    );
    let (casts, rest) = casts_and_rest(&through_fn);
    let place = through_fn.find("Money(1)").expect("the cast");
    assert_eq!(casts, [(place, "Money(1)".to_string())], "one error, at the cast\n{through_fn}");
    assert!(rest.is_empty(), "the cast is the only error: {rest:#?}");
}

/// A default whose own literal leaves the same field stops at that field:
/// one error, and the walk ends.
#[test]
fn a_self_referential_default_chain_is_entered_once() {
    let src = "unit cent;\ntype Money = quantity Int in cent;\ntype T { n: Int = Money(1); t: T = T {}; }\n\
               fn main() {\n    let x = T {};\n    println(x.n);\n}\n";
    let (casts, _) = casts_and_rest(src);
    let place = src.find("Money(1)").expect("the cast");
    assert_eq!(casts, [(place, "Money(1)".to_string())]);
}

/// The rows are one cell of the snapshot, derived once however often the
/// check and its consumers read them.
#[test]
fn the_rows_are_derived_once_per_snapshot() {
    let s = snapshot(EXAMPLE);
    s.demand_check().expect("checked");
    let _ = s.demand_units();
    let _ = s.demand_units();
    assert_eq!(s.builds().get("unit_declarations"), Some(&1));
}
