//! GH #1076, step U3: the values of quantities and points.
//!
//! A quantity (`quantity Int in cent`) and a point (`point Q`) are
//! nominal types represented as the `Int` they count
//! (`hale_types::unit_quantities`). An expression's type also carries its
//! denomination, which no declaration need have pinned: a synthesized
//! type, displayed `Money in 1/100 cent`. Pinned here: what a quantity
//! literal is; the algebra, as a matrix of operand kinds by operators;
//! every refusal by its message and the source text at its span; the
//! conversion row each kind of site records (its factor, its kind, its
//! policy and where the policy came from); the ratio product three ways;
//! and the law that no program with a narrowing nothing discharges
//! checks.

use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::{parse_source, Diag};
use hale_types::typed_bodies::{ConversionKind, ConversionRow, ConversionSite, Discharge};
use hale_types::units::RoundPolicy;

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;

/// The declarations every program below starts with: the committed
/// form's catalogue, its time units renamed (a duration suffix names no
/// unit until U4, law 10) and `Duration`, `Time` and `Bytes` renamed
/// (they are primitives until U4).
const DECLS: &str = "\
unit nsec;
unit usec = 1_000 nsec;
unit msec = 1_000 usec;
unit sec = 1_000 msec;
unit B;
unit KiB = 1024 B;
unit cent;
unit USD = 100 cent;
unit pct = 1/100;
unit bp = 1/100 pct;
unit tick;
unit mK;
unit K = 1_000 mK;
type Span = quantity Int in nsec;
type Instant = point Span;
type WireStamp = Instant in usec { round: floor; }
type Bucket = Span in 100 msec { round: floor; }
type Seconds = Span in sec;
type ByteCount = quantity Int in B;
type Money = quantity Int in cent;
type Rate = quantity Int in bp;
type Ledger = Money { round: half_even; }
type Tick = quantity Int in tick;
type Price = point Tick;
type TempDelta = quantity Int in mK;
type Kelvin = point TempDelta;
type Celsius = point TempDelta { origin: 273_150 mK; }
type OrderId = distinct Int;
type Byte = Int { range: 0..256; }
fn span(n: Int) -> Span { return n * 1nsec; }
fn amount(n: Int) -> Money { return n * 1USD; }
fn rate(n: Int) -> Rate { return n * 1bp; }
fn seconds(d: Seconds) -> Seconds { return d; }
fn wait(d: Span) -> Span { return d; }
fn whole(e: InexactError) -> Seconds { return 0sec; }
";

fn source(body: &str) -> String {
    format!("{DECLS}fn main() {{\n{body}    println(1);\n}}\n")
}

fn diags(body: &str) -> (String, Vec<Diag>) {
    let src = source(body);
    let all = check_program(&parse_source(&src).expect("parses"));
    (src, all)
}

/// Every error of `body`, as (the text at its span, its message).
fn errors(body: &str) -> Vec<(String, String)> {
    let (src, all) = diags(body);
    all.iter().filter(|d| d.is_error()).map(|d| (d.span.slice(&src).to_string(), d.message.clone())).collect()
}

fn clean(body: &str) {
    let found = errors(body);
    assert!(found.is_empty(), "checks clean: {found:#?}\n{body}");
}

fn one(body: &str, at: &str, message: &str) {
    assert_eq!(errors(body), [(at.to_string(), message.to_string())], "{body}");
}

/// The type the checker gives `expr`, read from the mismatch a `Bool`
/// binding of it reports.
fn ty_of(setup: &str, expr: &str) -> String {
    let found = errors(&format!("{setup}    let probe: Bool = {expr};\n"));
    let [(_, message)] = found.as_slice() else { panic!("one mismatch for `{expr}`: {found:#?}") };
    message
        .strip_prefix("let `probe`: expected `Bool`, got `")
        .and_then(|m| m.strip_suffix('`'))
        .unwrap_or_else(|| panic!("a mismatch: {message}"))
        .to_string()
}

/// One conversion row as a test reads it: the text at its site, its
/// target, its kind, its factor and shift, its policy, and a literal's
/// count, a split's divisor or a printed unit.
#[derive(Debug)]
struct Row {
    /// Where the site starts; not compared.
    start: usize,
    at: String,
    to: String,
    kind: ConversionKind,
    factor: String,
    offset: i64,
    policy: Option<Discharge>,
    count: Option<i64>,
    split: Option<i64>,
    printed: Option<String>,
}

/// Every quantity row `body` records (rows with a scale or a printed
/// unit), in source order, `body` checking clean.
fn rows(body: &str) -> Vec<Row> {
    let src = source(body);
    let main = src.find("fn main()").expect("a main");
    rows_of(&src).into_iter().filter(|r| r.start >= main).collect()
}

fn rows_of(src: &str) -> Vec<Row> {
    let program = parse_source(src).expect("parses");
    let errors: Vec<String> = check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot = Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let mut found: Vec<(ConversionSite, &ConversionRow)> =
        table.conversion_sites().filter(|(_, r)| r.scale.is_some() || r.printed.is_some()).collect();
    found.sort_by_key(|(s, r)| (r.span.start.as_usize(), r.span.end.as_usize(), matches!(s, ConversionSite::Printed { .. })));
    found
        .into_iter()
        .map(|(_, r)| Row {
            start: r.span.start.as_usize(),
            at: r.span.slice(src).to_string(),
            to: r.target.clone(),
            kind: r.kind,
            factor: r.scale.as_ref().map_or_else(String::new, |s| s.factor.to_string()),
            offset: r.scale.as_ref().map_or(0, |s| s.offset),
            policy: r.policy,
            count: r.count,
            split: r.split,
            printed: r.printed.clone(),
        })
        .collect()
}

impl PartialEq for Row {
    fn eq(&self, o: &Row) -> bool {
        (&self.at, &self.to, self.kind, &self.factor, self.offset, self.policy, self.count, self.split, &self.printed)
            == (&o.at, &o.to, o.kind, &o.factor, o.offset, o.policy, o.count, o.split, &o.printed)
    }
}

/// A row with nothing but its site, target, kind and factor said.
fn row(at: &str, to: &str, kind: ConversionKind, factor: &str) -> Row {
    Row {
        start: 0,
        at: at.into(),
        to: to.into(),
        kind,
        factor: factor.into(),
        offset: 0,
        policy: None,
        count: None,
        split: None,
        printed: None,
    }
}

fn round(policy: RoundPolicy, from_type: bool) -> Option<Discharge> {
    Some(Discharge::Round { policy, from_type })
}

// The literals.

/// A quantity literal is its component's quantity at its own unit: the
/// declared quantity when that unit is its denomination, else the
/// synthesized type. A unit's name is its own namespace: a local named
/// like a unit neither shadows it nor is shadowed by it.
#[test]
fn a_literal_is_its_quantity_at_its_own_unit() {
    assert_eq!(ty_of("", "5nsec"), "Span");
    assert_eq!(ty_of("", "3sec"), "Span in sec");
    assert_eq!(ty_of("", "1_250_000USD"), "Money in USD");
    assert_eq!(ty_of("", "3bp"), "Rate");
    assert_eq!(ty_of("", "4KiB"), "ByteCount in KiB");
    assert_eq!(ty_of("", "3cent"), "Money");
    clean("    let sec = 2;\n    let d = 3sec + 1sec;\n    let w = d.in(sec);\n    let n = sec + 1;\n");
    one("    let q = 3xyz;\n", "3xyz", "`3xyz`: no `unit` declares `xyz`");
    one("    let q = 3usc;\n", "3usc", "`3usc`: no `unit` declares `usc`; did you mean `usec`?");
}

/// A literal converts into the denomination it flows into at compile
/// time: its row holds the count lowering emits. A whole number of the
/// target is exact; one that is not is a narrowing like any other, the
/// target's `round:` discharging it or the check refusing it. Literal
/// arithmetic is not folded: each literal is its own constant.
#[test]
fn a_literal_converts_where_it_flows_at_compile_time() {
    use ConversionKind::*;
    let literal = |at: &str, to: &str, kind, factor: &str, count, policy| Row {
        count: Some(count),
        policy,
        ..row(at, to, kind, factor)
    };
    assert_eq!(
        rows("    let d: Span = 3sec;\n    let b: Bucket = 150msec;\n    let s: Seconds = 2_000msec;\n"),
        [
            literal("3sec", "Span", Widening, "1000000000", 3_000_000_000, None),
            literal("150msec", "Bucket", Narrowing, "1/100", 1, round(RoundPolicy::Floor, true)),
            // 2000 msec is 2 sec, exactly: no narrowing.
            literal("2_000msec", "Seconds", Widening, "1/1000", 2, None),
        ]
    );
    assert_eq!(
        rows("    let d = 3sec + 500msec;\n"),
        [
            literal("3sec", "Span in msec", Widening, "1000", 3_000, None),
            literal("500msec", "Span in msec", Widening, "1", 500, None),
        ]
    );
    one(
        "    let s: Seconds = 1_500msec;\n",
        "1_500msec",
        "`Seconds` from `Span in msec` divides by 1,000: say what happens to the remainder: convert explicitly \
         (`.in(u) or floor`, `Seconds(…) or half_even`, `or <value>`, `or raise`), or give `Seconds` a `round:` policy",
    );
}

// The algebra.

/// The operands the matrix combines: a quantity at its own denomination,
/// the same quantity at another, another quantity, a dimensionless one,
/// an `Int`, a point, two points of one quantity with different origins,
/// and an identity.
const OPERANDS: &str = "    let q = span(1);\n    let s = 3sec;\n    let b = 4KiB;\n    let r = rate(3);\n    \
                        let i = 2;\n    let p = Instant(q);\n    let k = Kelvin(300K);\n    let c = Celsius(0mK);\n    \
                        let o = OrderId(1);\n";

/// What `expr` is over [`OPERANDS`]: its type, or its refusal.
fn outcome(expr: &str) -> String {
    let found = errors(&format!("{OPERANDS}    let probe: Bool = {expr};\n"));
    let mismatch = "let `probe`: expected `Bool`, got `";
    match found.as_slice() {
        [] => "Bool".to_string(),
        [(_, m)] if m.starts_with(mismatch) => m[mismatch.len()..m.len() - 1].to_string(),
        [(at, m)] => format!("refused at `{at}`: {m}"),
        more => panic!("one outcome for `{expr}`: {more:#?}"),
    }
}

/// The plan's table, as a matrix of operand kinds by operators: each
/// cell the result's type (a sum at the finer denomination, a ratio
/// product at the product of the two, a quotient of one quantity an
/// `Int`, a difference of points their quantity) or the refusal, naming
/// both.
#[test]
fn the_algebra_is_the_plans_table() {
    let pairs = [
        ("q", "q"),
        ("q", "s"),
        ("s", "q"),
        ("q", "b"),
        ("q", "r"),
        ("r", "q"),
        ("r", "r"),
        ("q", "i"),
        ("i", "q"),
        ("p", "q"),
        ("q", "p"),
        ("p", "p"),
        ("p", "s"),
        ("k", "c"),
        ("q", "o"),
    ];
    let mut table = String::new();
    for (a, b) in pairs {
        for op in ["+", "-", "*", "/", "%", "<"] {
            table.push_str(&format!("{a} {op} {b}: {}\n", outcome(&format!("{a} {op} {b}"))));
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/unit_quantity_algebra.txt");
    if std::env::var_os("HALE_REGEN_UNIT_ALGEBRA").is_some() {
        std::fs::write(&path, &table).expect("write the matrix");
    }
    let pinned = std::fs::read_to_string(&path).expect("the pinned matrix");
    assert_eq!(table, pinned, "the algebra moved; regenerate with HALE_REGEN_UNIT_ALGEBRA=1 and read the diff");
}

/// A refusal names both declarations: one note at each (a synthesized
/// type's at its quantity's), the error at the operator's expression.
#[test]
fn a_refusal_names_both_declarations() {
    let (src, all) = diags("    let bad = 5msec + 4KiB;\n");
    let errors: Vec<&Diag> = all.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(errors[0].span.slice(&src), "5msec + 4KiB");
    assert_eq!(
        errors[0].message,
        "`Span in msec` + `ByteCount in KiB`: different quantities, `Span` and `ByteCount`; `+` holds within one"
    );
    let notes: Vec<(&str, &str)> = errors[0].related.iter().map(|r| (r.span.slice(&src), r.label.as_str())).collect();
    assert_eq!(notes, [("Span", "`Span` is declared here"), ("ByteCount", "`ByteCount` is declared here")]);
    one("    let n = -Instant(span(1));\n", "-Instant(span(1))", "`-` of the point `Instant`: a point has no negation");
    one(
        "    let i = Int(span(1));\n",
        "Int(span(1))",
        "`Int(…)` of `Span`: a quantity's count in a unit is a quotient (`q / 1nsec`), a point's the quantity from an \
         origin's",
    );
    one("    let m = Money(5);\n", "Money(5)", "`Money(…)` of an `Int`: a count becomes a quantity by a unit (`n * 1cent`)");
    one(
        "    let x: Money = 3sec;\n",
        "3sec",
        "`Span in sec` is not `Money`: different quantities, `Span` and `Money`; no conversion holds between them",
    );
    one("    let y: Int = 3sec;\n", "3sec", "`Span in sec` is not an `Int`: a quantity's count in a unit is a quotient (`q / 1sec`)");
    one("    let z: Span = 5;\n", "5", "`Int` is not `Span`: a count becomes a quantity by a unit (`n * 1nsec`)");
    clean("    let zero: Span = 0;\n");
    one(
        "    let r = span(1).in(cent);\n",
        "cent",
        "`.in(…)`: `cent` is not a unit of `Span`: `Span` counts in the units of `Span`",
    );
    one(
        "    let c = Celsius(0mK);\n    let k: Kelvin = c;\n",
        "c",
        "`Celsius` and `Kelvin` are points of different origins: convert explicitly, `Kelvin(…)`",
    );
}

// The conversion rows.

/// Every kind of site a value changes denomination at records a row:
/// a binding, an argument, an operand, `.in(u)`, `.split(u)`, a cast, a
/// division by a literal, a compound assignment, a point's cast across
/// origins; a printed quantity records its unit. Each row has its exact
/// factor, its kind (a widening when the factor is whole), its policy and
/// whether the policy came from the site or from the target type.
#[test]
fn every_site_a_denomination_changes_is_a_row() {
    use ConversionKind::*;
    let body = "    let d = span(3_500_000_000);\n    let s = 3sec;\n    let a: Span = s;\n    let w = wait(s);\n    \
                let e = s + 1msec;\n    let f = d.in(sec) or floor;\n    let (whole, rest) = d.split(sec);\n    \
                let b = Bucket(d);\n    let h = d / 2 or floor;\n    println(s);\n    let mut t = 0nsec;\n    \
                t += s;\n    let c = Celsius(0mK);\n    let k = Kelvin(c);\n";
    assert_eq!(
        rows(body),
        [
            Row { count: Some(3), ..row("3sec", "Span in sec", Widening, "1") },
            row("s", "Span", Widening, "1000000000"),
            row("s", "Span", Widening, "1000000000"),
            row("s", "Span in msec", Widening, "1000"),
            Row { count: Some(1), ..row("1msec", "Span in msec", Widening, "1") },
            Row { policy: round(RoundPolicy::Floor, false), ..row("d.in(sec)", "Span in sec", Narrowing, "1/1000000000") },
            Row { split: Some(1_000_000_000), ..row("d.split(sec)", "Span", Widening, "1") },
            Row { policy: round(RoundPolicy::Floor, true), ..row("Bucket(d)", "Bucket", Narrowing, "1/100000000") },
            Row { policy: round(RoundPolicy::Floor, false), ..row("d / 2", "Span", Narrowing, "1/2") },
            Row { printed: Some("sec".into()), factor: String::new(), ..row("s", "String", Total, "") },
            Row { count: Some(0), ..row("0nsec", "Span", Widening, "1") },
            row("s", "Span", Widening, "1000000000"),
            row("Celsius(0mK)", "Celsius", Widening, "1"),
            Row { count: Some(0), ..row("0mK", "Celsius", Widening, "1") },
            Row { offset: 273_150, ..row("Kelvin(c)", "Kelvin", Widening, "1") },
        ]
    );
}

/// A return, a struct field's default and a parameter's default convert
/// into their declared types like any value: each a row, the literal's
/// count converted.
#[test]
fn a_return_and_a_default_convert_into_their_types() {
    use ConversionKind::*;
    let src = format!(
        "{DECLS}type Fee {{ m: Money = 3USD; }}\nfn charge(m: Money = 2USD) -> Money {{ return m; }}\n\
         fn main() {{\n    let f = Fee {{}};\n    println(f.m + charge());\n}}\n"
    );
    let found = rows_of(&src);
    let at = |text: &str| found.iter().filter(|r| r.at == text).collect::<Vec<_>>();
    assert_eq!(at("n * 1USD"), [&row("n * 1USD", "Money", Widening, "100")], "a return");
    assert_eq!(at("3USD"), [&Row { count: Some(300), ..row("3USD", "Money", Widening, "100") }], "a field's default");
    assert_eq!(at("2USD"), [&Row { count: Some(200), ..row("2USD", "Money", Widening, "100") }], "a parameter's default");
}

// The ratio product (decision 2), three ways.

/// A quantity scaled by a ratio is the quantity at the product of the two
/// denominations, exact: nothing is rounded at the product. The
/// narrowing is where the value meets a coarser type: the target's
/// `round:` discharges it, and with none the check refuses it. With
/// literal operands each literal is its own unit's (`USD`), so the
/// product counts `1/100 cent`; with runtime operands typed `Money` and
/// `Rate`, `1/10000 cent`.
#[test]
fn the_ratio_product_is_exact_and_narrows_at_the_boundary() {
    use ConversionKind::*;
    // Literal operands, the products bound separately.
    let literal = "    let fee = 1_250_000USD * 3bp;\n    let odd = 1_234_567USD * 3bp;\n    let paid: Ledger = odd;\n";
    assert_eq!(ty_of("", "1_250_000USD * 3bp"), "Money in 1/100 cent");
    assert_eq!(ty_of("", "1_234_567USD * 3bp"), "Money in 1/100 cent");
    let found = rows(literal);
    assert_eq!(
        found.iter().map(|r| (r.at.as_str(), r.count)).filter(|(_, c)| c.is_some()).collect::<Vec<_>>(),
        [("1_250_000USD", Some(1_250_000)), ("3bp", Some(3)), ("1_234_567USD", Some(1_234_567)), ("3bp", Some(3))],
        "each literal at its own unit"
    );
    assert_eq!(
        found.last(),
        Some(&Row { policy: round(RoundPolicy::HalfEven, true), ..row("odd", "Ledger", Narrowing, "1/100") }),
        "the narrowing is at the binding, its policy `Ledger`'s"
    );
    // Runtime operands.
    let runtime = "    let amt = amount(1_234_567);\n    let r = rate(3);\n    let odd = amt * r;\n    let paid: Ledger = odd;\n";
    assert_eq!(ty_of("    let amt = amount(1);\n    let r = rate(3);\n", "amt * r"), "Money in 1/10000 cent");
    assert_eq!(
        rows(runtime).last(),
        Some(&Row { policy: round(RoundPolicy::HalfEven, true), ..row("odd", "Ledger", Narrowing, "1/10000") })
    );
    // Into a type with no policy: refused, with the two ways to say it.
    one(
        "    let odd = 1_234_567USD * 3bp;\n    let bad: Money = odd;\n",
        "odd",
        "`Money` from `Money in 1/100 cent` divides by 100: say what happens to the remainder: convert explicitly \
         (`.in(u) or floor`, `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy",
    );
    clean(
        "    let odd = 1_234_567USD * 3bp;\n    let a: Money = odd.in(cent) or half_even;\n    \
         let b: Money = odd.in(cent) or 0cent;\n    let c = Money(odd) or half_even;\n",
    );
}

// The law.

/// No program with a narrowing nothing discharges checks: an implicit
/// one into a type with no `round:` (a binding, an argument, a return, a
/// struct field's default, a parameter's default, a compound assignment),
/// `.in(u)`, a cast, a division by a literal. Each discharged (a policy
/// word, a substitute, a handler, `or raise`, the target's `round:`)
/// checks clean.
#[test]
fn no_program_with_a_narrowing_and_no_policy_checks() {
    let refused = [
        "    let s: Seconds = span(1);\n",
        "    let s = seconds(span(1));\n",
        "    let s = span(1).in(sec);\n",
        "    let s = Seconds(span(1));\n",
        "    let s = span(1) / 2;\n",
        "    let mut s: Seconds = 1sec;\n    s += 1msec;\n",
        "    let s: Seconds = 1_500msec;\n",
    ];
    for body in refused {
        let found = errors(body);
        assert_eq!(found.len(), 1, "one refusal: {found:#?}\n{body}");
        assert!(found[0].1.contains("say what happens to the remainder"), "{}", found[0].1);
    }
    for (decl, body) in [
        ("fn f(d: Span) -> Seconds { return d; }\n", "    let s = f(span(1));\n"),
        ("type T { s: Seconds = 1_500msec; }\n", "    let t = T {};\n"),
        ("fn g(s: Seconds = 1_500msec) -> Seconds { return s; }\n", "    let s = g();\n"),
    ] {
        let src = format!("{DECLS}{decl}fn main() {{\n{body}    println(1);\n}}\n");
        let found: Vec<String> =
            check_program(&parse_source(&src).expect("parses")).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
        assert_eq!(found.len(), 1, "one refusal: {found:#?}\n{src}");
        assert!(found[0].contains("say what happens to the remainder"), "{}", found[0]);
    }
    clean(
        "    let a: Bucket = span(1);\n    let b = span(1).in(sec) or floor;\n    let c = span(1).in(sec) or 0sec;\n    \
         let d = Seconds(span(1)) or whole(err);\n    let e = span(1) / 2 or half_up;\n    let f = Bucket(span(1));\n    \
         let g = span(1).in(sec) or ceil;\n    let h = span(1).in(sec) or trunc;\n    let i = span(1).in(sec) or half_even;\n",
    );
}

/// A policy belongs to the family its narrowing admits: a rounding to a
/// conversion that divides, `clamp` and `wrap` to a range's.
#[test]
fn a_policy_belongs_to_its_narrowings_family() {
    one(
        "    let s = span(1).in(sec) or clamp;\n",
        "span(1).in(sec) or clamp",
        "`or clamp` is a range's policy, and this conversion divides: say what becomes of the remainder (`or floor`, \
         `or ceil`, `or trunc`, `or half_even`, `or half_up`), or `or <value>`",
    );
    one(
        "    let n = 300;\n    let b = Byte(n) or floor;\n",
        "Byte(n) or floor",
        "`or floor` rounds a conversion that divides, and this one narrows into a range: `or clamp`, `or wrap`, or \
         `or <value>`",
    );
}

/// A position the checker does not classify refuses a value counted in a
/// denomination no declaration names, with a located error, and never
/// stores it as the wrong count: a generic literal's field (its type is
/// known only where the literal flows), a generic fn's argument (a
/// monomorph is named by declared types), two arms of different
/// denominations.
#[test]
fn an_unclassified_position_refuses_a_synthesized_denomination() {
    let generic = "type Box<T> { v: T; }\nfn id<T>(x: T) -> T { return x; }\n";
    let check = |body: &str| -> Vec<(String, String)> {
        let src = format!("{DECLS}{generic}fn main() {{\n{body}    println(1);\n}}\n");
        check_program(&parse_source(&src).expect("parses"))
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| (d.span.slice(&src).to_string(), d.message))
            .collect()
    };
    assert_eq!(
        check("    let b: Box<Span> = Box { v: 2sec };\n"),
        [(
            "2sec".to_string(),
            "a value of `Span in sec` meets a place whose type is not known here, so it has nothing to be converted \
             into: convert it first (`Span(…)`)"
                .to_string()
        )]
    );
    assert_eq!(check("    let b: Box<Span> = Box { v: Span(2sec) };\n"), []);
    assert_eq!(
        check("    let a = id(3sec);\n"),
        [(
            "id".to_string(),
            "generic fn `id`: `Span in sec` is a denomination no declaration names, and a generic argument is a \
             declared type: bind the value to one first (`let x: T = …`)"
                .to_string()
        )]
    );
    assert_eq!(
        check("    let flag = true;\n    let e: Span = if flag { 1sec } else { 2msec };\n"),
        [(
            "if flag { 1sec } else { 2msec }".to_string(),
            "if-expression arms have mismatched types: then=`Span in sec`, else=`Span in msec`".to_string()
        )]
    );
}

/// `InexactError`, which a conversion that divides fails with under its
/// `or`, is a builtin type in a program with a quantity, and in no other.
#[test]
fn inexact_error_is_a_builtin_where_a_quantity_is_declared() {
    let row = hale_types::builtin_types::builtin_type("InexactError").expect("a builtin type");
    let fields: Vec<&str> = row.fields.iter().map(|(n, _)| *n).collect();
    assert_eq!(fields, ["kind", "value", "divisor"]);
    clean("    let s = Seconds(span(1)) or whole(err);\n");
    let injected = |src: &str| {
        let program = parse_source(src).expect("parses");
        let bundle = hale_types::Bundle::new(std::collections::BTreeMap::from([(String::new(), &program)]));
        hale_types::resolve::build_top_scope(&bundle).0.lookup("InexactError").is_some()
    };
    assert!(injected("unit cent;\ntype Money = quantity Int in cent;\nfn main() { println(1); }\n"));
    assert!(!injected("type Byte = Int { range: 0..256; }\nfn main() { println(1); }\n"));
}
