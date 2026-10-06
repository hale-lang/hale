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
use hale_types::typed_bodies::{ConversionKind, ConversionRow, ConversionSite, Discharge, SiteKind};
use hale_types::units::RoundPolicy;

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;

/// The declarations every program below starts with: the committed
/// form's catalogue, its time units renamed (the stdlib's time catalogue
/// declares `ns` … `min`, U4, and these lines test a program's own) and
/// `Duration`, `Time` and `Bytes` renamed (the first two are the
/// stdlib's, the third the buffer type); and a mass counted in grams, so
/// a literal of a finer unit (`5mg`) is no whole count of its quantity.
/// A literal counts in its quantity's denomination when it is a whole
/// count of it, so the operands a test wants at another denomination
/// are made so: `seconds(3sec).in(sec)` is `Span in sec`.
const DECLS: &str = "\
unit nsec;
unit usec = 1_000 nsec;
unit msec = 1_000 usec;
unit sec = 1_000 msec;
unit mg;
unit g = 1_000 mg;
type Mass = quantity Int in g;
unit B;
unit KiB = 1024 B;
type KiBs = ByteCount in KiB;
fn kib(b: KiBs) -> KiBs { return b; }
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
    /// The type it converts from; not compared.
    from: String,
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
    found.sort_by_key(|(s, r)| (r.span.start.as_usize(), r.span.end.as_usize(), matches!(s.kind, SiteKind::Printed { .. })));
    found
        .into_iter()
        .map(|(_, r)| Row {
            start: r.span.start.as_usize(),
            at: r.span.slice(src).to_string(),
            from: r.from.display(),
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
        from: String::new(),
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

/// A quantity literal is its component's quantity at the quantity's
/// denomination when it is a whole count of it (U4: `3sec` is
/// 3,000,000,000 of `Span`, as `500ms` is 500,000,000 of `Duration`),
/// else at its own unit, the synthesized type (`5mg` of a `Mass` counted
/// in grams). A unit's name is its own namespace: a local named like a
/// unit neither shadows it nor is shadowed by it.
#[test]
fn a_literal_is_its_quantity_at_its_denomination_when_whole() {
    assert_eq!(ty_of("", "5nsec"), "Span");
    assert_eq!(ty_of("", "3sec"), "Span");
    assert_eq!(ty_of("", "1_250_000USD"), "Money");
    assert_eq!(ty_of("", "3bp"), "Rate");
    assert_eq!(ty_of("", "4KiB"), "ByteCount");
    assert_eq!(ty_of("", "3cent"), "Money");
    assert_eq!(ty_of("", "2g"), "Mass");
    assert_eq!(ty_of("", "5mg"), "Mass in mg");
    assert_eq!(ty_of("", "500ms"), "Duration");
    clean("    let sec = 2;\n    let d = 3sec + 1sec;\n    let w = d.in(sec) or floor;\n    let n = sec + 1;\n");
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
        rows(
            "    let d: Span = 3sec;\n    let b: Bucket = 150msec;\n    let s: Seconds = 2_000msec;\n    \
             let m: Mass = 2_000mg;\n"
        ),
        [
            // Already a count of `Span`: its own row, nothing to convert.
            literal("3sec", "Span", Widening, "1", 3_000_000_000, None),
            literal("150msec", "Bucket", Narrowing, "1/100000000", 1, round(RoundPolicy::Floor, true)),
            // 2000 msec is 2 sec, exactly: no narrowing.
            literal("2_000msec", "Seconds", Widening, "1/1000000000", 2, None),
            // 2000 mg is 2 g: a count of `Mass` already, whatever `mg`'s
            // factor.
            literal("2_000mg", "Mass", Widening, "1", 2, None),
        ]
    );
    assert_eq!(
        rows("    let d = 3sec + 500msec;\n"),
        [
            literal("3sec", "Span", Widening, "1", 3_000_000_000, None),
            literal("500msec", "Span", Widening, "1", 500_000_000, None),
        ]
    );
    one(
        "    let s: Seconds = 1_500msec;\n",
        "1_500msec",
        "`Seconds` from `Span` divides by 1,000,000,000: say what happens to the remainder: convert explicitly \
         (`.in(u) or floor`, `Seconds(…) or half_even`, `or <value>`, `or raise`), or give `Seconds` a `round:` policy",
    );
    one(
        "    let m: Mass = 1_500mg;\n",
        "1_500mg",
        "`Mass` from `Mass in mg` divides by 1,000: say what happens to the remainder: convert explicitly \
         (`.in(u) or floor`, `Mass(…) or half_even`, `or <value>`, `or raise`), or give `Mass` a `round:` policy",
    );
}

// The algebra.

/// The operands the matrix combines: a quantity at its own denomination,
/// the same quantity at another, another quantity, a dimensionless one,
/// an `Int`, a point, two points of one quantity with different origins,
/// and an identity.
const OPERANDS: &str = "    let q = span(1);\n    let s = seconds(3sec).in(sec);\n    let b = kib(4KiB).in(KiB);\n    \
                        let r = rate(3);\n    \
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
    assert_eq!(errors[0].message, "`Span` + `ByteCount`: different quantities, `Span` and `ByteCount`; `+` holds within one");
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
        "`Span` is not `Money`: different quantities, `Span` and `Money`; no conversion holds between them",
    );
    one(
        "    let y: Int = seconds(3sec).in(sec);\n",
        "seconds(3sec).in(sec)",
        "`Span in sec` is not an `Int`: a quantity's count in a unit is a quotient (`q / 1sec`)",
    );
    // U4: the stdlib's `Duration` is a quantity like any.
    one("    let y: Int = 3s;\n", "3s", "`Duration` is not an `Int`: a quantity's count in a unit is a quotient (`q / 1ns`)");
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
    let body = "    let d = span(3_500_000_000);\n    let s = seconds(3sec).in(sec);\n    let a: Span = s;\n    let w = wait(s);\n    \
                let e = s + 1msec;\n    let f = d.in(sec) or floor;\n    let (whole, rest) = d.split(sec);\n    \
                let b = Bucket(d);\n    let h = d / 2 or floor;\n    println(s);\n    let mut t = 0nsec;\n    \
                t += s;\n    let c = Celsius(0mK);\n    let k = Kelvin(c);\n";
    assert_eq!(
        rows(body),
        [
            row("seconds(3sec).in(sec)", "Span in sec", Widening, "1"),
            Row { count: Some(3), ..row("3sec", "Seconds", Widening, "1/1000000000") },
            row("s", "Span", Widening, "1000000000"),
            row("s", "Span", Widening, "1000000000"),
            row("s", "Span", Widening, "1000000000"),
            Row { count: Some(1_000_000), ..row("1msec", "Span", Widening, "1") },
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
/// count converted (`3_000msec` is a count of `Span`, 3,000,000,000,
/// and 3 of `Seconds`).
#[test]
fn a_return_and_a_default_convert_into_their_types() {
    use ConversionKind::*;
    let src = format!(
        "{DECLS}type Dose {{ m: Seconds = 3_000msec; }}\nfn dose(m: Seconds = 2_000msec) -> Seconds {{ return m; }}\n\
         fn bytes(b: KiBs) -> ByteCount {{ return b.in(KiB); }}\n\
         fn main() {{\n    let f = Dose {{}};\n    println(f.m + dose());\n    println(bytes(kib(1KiB)));\n}}\n"
    );
    let found = rows_of(&src);
    let at = |text: &str| found.iter().filter(|r| r.at == text).collect::<Vec<_>>();
    assert_eq!(
        at("b.in(KiB)"),
        [&row("b.in(KiB)", "ByteCount in KiB", Widening, "1"), &row("b.in(KiB)", "ByteCount", Widening, "1024")],
        "a return, of the value `.in(KiB)` made"
    );
    let seconds = |at: &str, count| Row { count: Some(count), ..row(at, "Seconds", Widening, "1/1000000000") };
    assert_eq!(at("3_000msec"), [&seconds("3_000msec", 3)], "a field's default");
    assert_eq!(at("2_000msec"), [&seconds("2_000msec", 2)], "a parameter's default");
}

/// A literal is a whole count of its quantity's denomination when its
/// value times its unit's factor is a whole number, whatever the factor
/// alone is: `1000mg` of a `Mass` counted in grams is 1 of `Mass`, as
/// `-2_000mg` is -2, while `5mg` and `1_500mg` are counted at their own
/// unit, `Mass in mg`. A whole count no `Int` holds is refused at the
/// literal, never wrapped and never kept at its own unit; the refused
/// literal is still its quantity, so what it flows into says nothing
/// more (`n * 1c` below is one error, not a second `Int is not A`).
#[test]
fn a_whole_count_is_its_quantity_whatever_the_units_factor() {
    use ConversionKind::*;
    assert_eq!(ty_of("", "1000mg"), "Mass");
    assert_eq!(ty_of("", "1_500mg"), "Mass in mg");
    let literal = |at: &str, to: &str, count| Row { count: Some(count), ..row(at, to, Widening, "1") };
    assert_eq!(
        rows("    let a = 1000mg;\n    let b = 5mg;\n    let c = 1_500mg;\n    let d = -2_000mg;\n"),
        [
            literal("1000mg", "Mass", 1),
            literal("5mg", "Mass in mg", 5),
            literal("1_500mg", "Mass in mg", 1_500),
            literal("2_000mg", "Mass", 2),
        ]
    );
    clean("    let x = if true { 1000mg } else { 1g };\n");
    one(
        "    let q = 100_000_000_000_000_000USD;\n",
        "100_000_000_000_000_000USD",
        "`100000000000000000USD` as a count of `Money` overflows an `Int`",
    );
    // Flowing into an operand, and into a narrowing, adds nothing.
    assert_eq!(
        errors("    let k: TempDelta = 10_000_000_000_000_000K + 1K;\n    let s: Seconds = 10_000_000_000sec;\n"),
        [
            (
                "10_000_000_000_000_000K".to_string(),
                "`10000000000000000K` as a count of `TempDelta` overflows an `Int`".to_string()
            ),
            ("10_000_000_000sec".to_string(), "`10000000000sec` as a count of `Span` overflows an `Int`".to_string()),
        ]
    );
    let errors_of = |src: &str| -> Vec<(String, String)> {
        check_program(&parse_source(src).expect("parses"))
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| (d.span.slice(src).to_string(), d.message))
            .collect()
    };
    let src = "unit a;\nunit b = 1_000_000_000_000 a;\nunit c = 1_000_000_000_000 b;\ntype A = quantity Int in a;\n\
               fn conv(n: Int) -> A { return n * 1c; }\nfn main() { println(conv(1)); }\n";
    assert_eq!(errors_of(src), [("1c".to_string(), "`1c` as a count of `A` overflows an `Int`".to_string())]);
}

/// A literal in a default is converted once per evaluation, into what
/// that evaluation's scope flows it into: its row is keyed by the
/// evaluation path as a cast's is. Where a local shadows `Bucket`,
/// `Bucket(2000msec)` calls `fake` and the literal stays a `Span` (2000);
/// elsewhere it is the cast's, into `sec` (2). Keyed by its span alone,
/// the literal had one row, and the shadowed evaluation was handed 2.
#[test]
fn a_literal_in_a_default_has_a_row_per_evaluation() {
    let src = "unit msec;\nunit sec = 1_000 msec;\ntype Span = quantity Int in msec;\n\
               type Bucket = Span in sec { round: floor; }\n\
               fn fake(d: Span) -> Bucket { return Bucket(d + 5000msec); }\n\
               type L { b: Bucket = Bucket(2000msec); }\n\
               fn main() {\n    let l = L {};\n    { let Bucket = fake; let m = L {}; println(m.b); }\n    println(l.b);\n}\n";
    let program = parse_source(src).expect("parses");
    let errors: Vec<String> = check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot = Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let mut literals = Vec::new();
    hale_syntax::sites::for_each_site(snapshot.program().expect("the program"), &mut |k, _, id| {
        if k == hale_syntax::sites::SiteKind::StructLiteral {
            literals.push(id.0);
        }
    });
    let [plain, shadowed] = literals[..] else { panic!("two literals: {literals:?}") };
    let mut found: Vec<(ConversionSite, Option<i64>)> = table
        .conversion_sites()
        .filter(|(s, r)| matches!(s.kind, SiteKind::Value { .. }) && r.span.slice(src) == "2000msec")
        .map(|(s, r)| (s, r.count))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let span = table.conversions().find(|r| r.span.slice(src) == "2000msec").expect("the literal's row").span;
    let at = |path: Vec<u32>| ConversionSite::new(SiteKind::value(span), path);
    let mut want = vec![(at(vec![plain]), Some(2)), (at(vec![shadowed]), Some(2000))];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(found, want, "one row per evaluation, each with its own count");
    assert_eq!(table.conversion(&at(vec![plain])).and_then(|r| r.count), Some(2), "the cast's evaluation: into `sec`");
    assert_eq!(table.conversion(&at(vec![shadowed])).and_then(|r| r.count), Some(2000), "`fake`'s: a `Span`");
    assert!(table.conversion(&at(vec![])).is_none(), "a default's literal has no evaluation-less row");
}

/// A default flows into its declared type as a binding's initializer does,
/// whatever the type's shape: an array literal's elements each flow into
/// the element type, each a row on the evaluation's path, the struct
/// literal that leaves the field or the call that leaves the parameter.
/// The walks converted a default only when the whole type was a quantity,
/// so these elements had no row there and kept their counts in `USD`.
#[test]
fn an_array_defaults_elements_convert_on_the_evaluation_path() {
    let src = "unit cent;\nunit USD = 100 cent;\ntype Money = quantity Int in cent;\n\
               type S { p: [Money; 2] = [3USD, 2USD]; }\n\
               fn take(q: [Money; 2] = [5USD, 4USD]) -> [Money; 2] { return q; }\n\
               fn main() {\n    let s = S {};\n    let q = take();\n    println(s.p[0]);\n    println(q[0]);\n}\n";
    let program = parse_source(src).expect("parses");
    let errors: Vec<String> = check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot = Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let (mut literal, mut call) = (None, None);
    hale_syntax::sites::for_each_site(snapshot.program().expect("the program"), &mut |k, span, id| match k {
        hale_syntax::sites::SiteKind::StructLiteral => literal = Some(id.0),
        hale_syntax::sites::SiteKind::Call if span.slice(src) == "take()" => call = Some(id.0),
        _ => {}
    });
    let (literal, call) = (literal.expect("`S {}`"), call.expect("`take()`"));
    let mut found: Vec<(String, Vec<u32>, String, Option<i64>)> = table
        .conversion_sites()
        .filter(|(s, r)| matches!(s.kind, SiteKind::Value { .. }) && r.span.slice(src).ends_with("USD"))
        .map(|(s, r)| (r.span.slice(src).to_string(), s.path, r.target.clone(), r.count))
        .collect();
    found.sort();
    let element = |at: &str, path: u32, count: i64| (at.to_string(), vec![path], "Money".to_string(), Some(count));
    assert_eq!(
        found,
        [element("2USD", literal, 200), element("3USD", literal, 300), element("4USD", call, 400), element("5USD", call, 500)],
        "each element into `Money`, on its evaluation's path and on no other"
    );
}

/// A default is refused where the same binding is. An array default whose
/// element narrows into an element type with no `round:` is refused by the
/// law: one error at the element, worded as the binding's. A tuple holding
/// a quantity at another denomination is a mismatch for a binding (only
/// an array literal's elements flow one by one), and for a default: the
/// default walk keeps it, worded as a default's.
#[test]
fn a_default_is_refused_where_the_same_binding_is() {
    let array = "[2_000msec, 1_500msec]";
    let binding = errors(&format!("    let s: [Seconds; 2] = {array};\n"));
    assert_eq!(binding.len(), 1, "{binding:#?}");
    assert_eq!(binding[0].0, "1_500msec");
    assert!(binding[0].1.contains("say what happens to the remainder"), "{}", binding[0].1);
    let errors_of = |decl: &str, body: &str| {
        let src = format!("{DECLS}{decl}fn main() {{\n{body}    println(1);\n}}\n");
        let found: Vec<(String, String)> = check_program(&parse_source(&src).expect("parses"))
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| (d.span.slice(&src).to_string(), d.message))
            .collect();
        found
    };
    for (decl, body) in [
        (format!("type T {{ s: [Seconds; 2] = {array}; }}\n"), "    let t = T {};\n"),
        (format!("fn g(s: [Seconds; 2] = {array}) -> [Seconds; 2] {{ return s; }}\n"), "    let s = g();\n"),
    ] {
        assert_eq!(errors_of(&decl, body), binding, "{decl}");
    }
    // `1_500mg` is no whole count of grams, so it is a `Mass in mg` (U4:
    // `3USD` would be a count of `Money` already).
    let tuple = "(1_500mg, 1)";
    one(
        &format!("    let t: (Mass, Int) = {tuple};\n"),
        tuple,
        "let `t`: expected `(Mass, Int)`, got `(Mass in mg, Int)`",
    );
    for (decl, body, place) in [
        (format!("type T {{ t: (Mass, Int) = {tuple}; }}\n"), "    let t = T {};\n", "field `t`"),
        (format!("fn g(t: (Mass, Int) = {tuple}) -> Int {{ return 1; }}\n"), "    let n = g();\n", "param `t`"),
    ] {
        let message = format!("{place}: declared `(Mass, Int)`, default is `(Mass in mg, Int)`");
        assert_eq!(errors_of(&decl, body), [(tuple.to_string(), message)], "{decl}");
    }
}

/// An array literal's elements, read as (the text at the row's site, the
/// type it converts from, its target, its factor, a literal's count).
fn element_rows(body: &str) -> Vec<(String, String, String, String, Option<i64>)> {
    rows(body).into_iter().map(|r| (r.at, r.from, r.to, r.factor, r.count)).collect()
}

fn element(at: &str, from: &str, to: &str, factor: &str, count: Option<i64>) -> (String, String, String, String, Option<i64>) {
    (at.into(), from.into(), to.into(), factor.into(), count)
}

/// An array literal into a place of an array type: each element flows into
/// the element type from its own type, its own row (its own factor, its
/// own count), whatever the first element is. Typed by its first element,
/// `[3cent, 2USD]` stored `2` cents for `2USD`, and a value element kept
/// the meet's row in place of its own. A literal counts in its quantity's
/// denomination when whole (U4), so `[3cent, 2USD]` is two counts of
/// `Money` already: their meet is `Money`, and each keeps its own row.
/// Elements of two denominations are a value beside a literal, or a
/// literal that is no whole count of its quantity (`1_500mg`).
#[test]
fn an_array_literals_elements_each_convert_from_their_own_type() {
    let annotated = |array: &str| element_rows(&format!("    let a: [Money; 2] = {array};\n"));
    assert_eq!(
        annotated("[3cent, 2USD]"),
        [element("3cent", "Money", "Money", "1", Some(3)), element("2USD", "Money", "Money", "1", Some(200))]
    );
    assert_eq!(
        annotated("[3USD, 2cent]"),
        [element("3USD", "Money", "Money", "1", Some(300)), element("2cent", "Money", "Money", "1", Some(2))]
    );
    // A value at another denomination, either side of a literal.
    let bytes = |array: &str| element_rows(&format!("    let a: [ByteCount; 2] = {array};\n"));
    assert_eq!(
        bytes("[kib(1KiB), 3B]"),
        [
            element("kib(1KiB)", "KiBs", "ByteCount", "1024", None),
            element("1KiB", "ByteCount", "KiBs", "1/1024", Some(1)),
            element("3B", "ByteCount", "ByteCount", "1", Some(3)),
        ]
    );
    assert_eq!(
        bytes("[3B, kib(1KiB)]"),
        [
            element("3B", "ByteCount", "ByteCount", "1", Some(3)),
            element("kib(1KiB)", "KiBs", "ByteCount", "1024", None),
            element("1KiB", "ByteCount", "KiBs", "1/1024", Some(1)),
        ]
    );
    // A value element: the meet (`Span`) converted `s` there; the place's
    // element type is `Seconds`, `s`'s own, so the meet's row is withdrawn
    // and `s` has none.
    assert_eq!(
        element_rows("    let s = seconds(3sec);\n    let a: [Seconds; 2] = [s, 2_000msec];\n"),
        [
            element("3sec", "Span", "Seconds", "1/1000000000", Some(3)),
            element("2_000msec", "Span", "Seconds", "1/1000000000", Some(2)),
        ]
    );
    // A type's `round:` discharges a narrowing element as it does a value.
    assert_eq!(
        element_rows("    let b: [Bucket; 2] = [1sec, 150msec];\n"),
        [
            element("1sec", "Span", "Bucket", "1/100000000", Some(10)),
            element("150msec", "Span", "Bucket", "1/100000000", Some(1)),
        ]
    );
    // A literal zero is a value of every quantity, in an array as alone.
    clean("    let z: [Money; 2] = [3USD, 0];\n");
    // The narrowing element alone is refused, worded as the binding's.
    one(
        "    let s: [Seconds; 2] = [1sec, 1_500msec];\n",
        "1_500msec",
        "`Seconds` from `Span` divides by 1,000,000,000: say what happens to the remainder: convert explicitly \
         (`.in(u) or floor`, `Seconds(…) or half_even`, `or <value>`, `or raise`), or give `Seconds` a `round:` policy",
    );
    // A literal at its own unit is the element whose type differs: the meet
    // (`Mass in mg`) widened `3g`; into `Mass` each converts from its own.
    one(
        "    let m: [Mass; 2] = [3g, 1_500mg];\n",
        "1_500mg",
        "`Mass` from `Mass in mg` divides by 1,000: say what happens to the remainder: convert explicitly \
         (`.in(u) or floor`, `Mass(…) or half_even`, `or <value>`, `or raise`), or give `Mass` a `round:` policy",
    );
    one("    let a: [Money; 2] = [3cent, 5];\n", "5", "`Int` is not `Money`: a count becomes a quantity by a unit (`n * 1cent`)");
    one(
        "    let a: [Span; 2] = [1sec, 3cent];\n",
        "3cent",
        "`Money` is not `Span`: different quantities, `Money` and `Span`; no conversion holds between them",
    );
    one("    let a: [Money; 2] = [3cent, \"x\"];\n", "\"x\"", "`[…]`: an element of `String` where `Money` is expected");
}

/// An array literal with no element type to flow into: its elements meet
/// as a sum's operands do, at the finer denomination, each coarser element
/// widened exactly with its row; a nested literal meets at each level.
/// Elements of two quantities, a quantity beside a point or an `Int`, are
/// refused naming both. Elements all of one type (literals that are whole
/// counts of their quantity, U4) meet at that type: the meet converts
/// none, and each keeps its literal's own row.
#[test]
fn an_array_literals_elements_meet_where_nothing_types_it() {
    assert_eq!(ty_of("", "[3g, 1_500mg]"), "[Mass in mg; 2]");
    assert_eq!(ty_of("", "[1_500mg, 3g]"), "[Mass in mg; 2]");
    assert_eq!(ty_of("", "[[3g], [1_500mg]]"), "[[Mass in mg; 1]; 2]");
    assert_eq!(ty_of("", "[seconds(1sec), 1_500msec]"), "[Span; 2]");
    assert_eq!(ty_of("", "[1sec, 1_500msec]"), "[Span; 2]");
    assert_eq!(ty_of("", "[3USD, 2USD]"), "[Money; 2]");
    assert_eq!(
        element_rows("    let a = [3g, 1_500mg];\n"),
        [
            element("3g", "Mass", "Mass in mg", "1000", Some(3000)),
            element("1_500mg", "Mass in mg", "Mass in mg", "1", Some(1500)),
        ]
    );
    assert_eq!(
        element_rows("    let n = [[3g], [1_500mg]];\n"),
        [
            element("3g", "Mass", "Mass in mg", "1000", Some(3000)),
            element("1_500mg", "Mass in mg", "Mass in mg", "1", Some(1500)),
        ]
    );
    assert_eq!(
        element_rows("    let a = [1sec, 1_500msec];\n"),
        [element("1sec", "Span", "Span", "1", Some(1_000_000_000)), element("1_500msec", "Span", "Span", "1", Some(1_500_000_000))]
    );
    let (src, all) = diags("    let a = [1sec, 3cent];\n");
    let errors: Vec<&Diag> = all.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(errors[0].span.slice(&src), "[1sec, 3cent]");
    assert_eq!(errors[0].message, "`[…]`: elements of different quantities, `Span` and `Money`; an array holds one");
    let notes: Vec<(&str, &str)> = errors[0].related.iter().map(|r| (r.span.slice(&src), r.label.as_str())).collect();
    assert_eq!(notes, [("Span", "`Span` is declared here"), ("Money", "`Money` is declared here")]);
    one(
        "    let p = Instant(span(1));\n    let a = [p, 1sec];\n",
        "[p, 1sec]",
        "`[…]`: `Instant` is a point and `Span` a quantity; an array holds one or the other",
    );
    one(
        "    let a = [3cent, 5];\n",
        "[3cent, 5]",
        "`[…]`: elements of `Money` and `Int`: an `Int` is no quantity; a count becomes one by a unit (`n * 1cent`)",
    );
    one(
        "    let c = Celsius(0mK);\n    let k = Kelvin(300K);\n    let a = [c, k];\n",
        "[c, k]",
        "`[…]`: elements of points of different origins, `Celsius` and `Kelvin`; an array holds one: convert \
         explicitly, `Celsius(…)`",
    );
    // Only a quantity converts: a tuple's part at another denomination
    // has no meet.
    one(
        "    let a = [(3g, 1), (1_500mg, 1)];\n",
        "[(3g, 1), (1_500mg, 1)]",
        "`[…]`: elements of `(Mass, Int)` and `(Mass in mg, Int)`; an array holds one type",
    );
}

// The ratio product (decision 2), three ways.

/// A quantity scaled by a ratio is the quantity at the product of the two
/// denominations, exact: nothing is rounded at the product. The
/// narrowing is where the value meets a coarser type: the target's
/// `round:` discharges it, and with none the check refuses it. Each
/// literal counts in its quantity's denomination (`1_234_567USD` is
/// 123,456,700 cent, U4), so with literal operands the product counts
/// `1/10000 cent` (370,370,100), as it does with runtime operands typed
/// `Money` and `Rate`: decision 2's figure either way.
#[test]
fn the_ratio_product_is_exact_and_narrows_at_the_boundary() {
    use ConversionKind::*;
    // Literal operands, the products bound separately.
    let literal = "    let fee = 1_250_000USD * 3bp;\n    let odd = 1_234_567USD * 3bp;\n    let paid: Ledger = odd;\n";
    assert_eq!(ty_of("", "1_250_000USD * 3bp"), "Money in 1/10000 cent");
    assert_eq!(ty_of("", "1_234_567USD * 3bp"), "Money in 1/10000 cent");
    let found = rows(literal);
    assert_eq!(
        found.iter().map(|r| (r.at.as_str(), r.count)).filter(|(_, c)| c.is_some()).collect::<Vec<_>>(),
        [("1_250_000USD", Some(125_000_000)), ("3bp", Some(3)), ("1_234_567USD", Some(123_456_700)), ("3bp", Some(3))],
        "each literal at its quantity's denomination"
    );
    assert_eq!(
        found.last(),
        Some(&Row { policy: round(RoundPolicy::HalfEven, true), ..row("odd", "Ledger", Narrowing, "1/10000") }),
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
        "`Money` from `Money in 1/10000 cent` divides by 10,000: say what happens to the remainder: convert explicitly \
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
        check("    let b: Box<Span> = Box { v: seconds(2sec).in(sec) };\n"),
        [(
            "seconds(2sec).in(sec)".to_string(),
            "a value of `Span in sec` meets a place whose type is not known here, so it has nothing to be converted \
             into: convert it first (`Span(…)`)"
                .to_string()
        )]
    );
    assert_eq!(check("    let b: Box<Span> = Box { v: Span(seconds(2sec).in(sec)) };\n"), []);
    assert_eq!(
        check("    let a = id(seconds(3sec).in(sec));\n"),
        [(
            "id".to_string(),
            "generic fn `id`: `Span in sec` is a denomination no declaration names, and a generic argument is a \
             declared type: bind the value to one first (`let x: T = …`)"
                .to_string()
        )]
    );
    assert_eq!(
        check("    let flag = true;\n    let e: Mass = if flag { 2g } else { 2mg };\n"),
        [(
            "if flag { 2g } else { 2mg }".to_string(),
            "if-expression arms have mismatched types: then=`Mass`, else=`Mass in mg`".to_string()
        )]
    );
    // A literal is at its quantity's denomination wherever it is a whole
    // count of it (U4), so literals of one quantity meet these positions
    // as one type: a time literal in a generic literal, an argument, an
    // `if`'s arms.
    assert_eq!(check("    let b: Box<Span> = Box { v: 2sec };\n    let a = id(3sec);\n"), []);
    assert_eq!(check("    let flag = true;\n    let e: Span = if flag { 1sec } else { 2msec };\n"), []);
    assert_eq!(check("    let flag = true;\n    let e = if flag { 1s } else { 2ms };\n    let x = [1s, 500ms];\n"), []);
}

// The wire (decision 9).

/// The program the layout pins read: quantity and point fields of a
/// hashmap record keyed by a quantity, and a topic's payload and routing
/// key. A raw literal, so the corpus harvests it and the topology
/// baseline holds its shape hash.
const WIRE: &str = r#"unit nsec;
unit usec = 1_000 nsec;
unit msec = 1_000 usec;
unit cent;
unit USD = 100 cent;
unit mK;
type Span = quantity Int in nsec;
type Instant = point Span;
type Bucket = Span in 100 msec { round: floor; }
type Money = quantity Int in cent;
type TempDelta = quantity Int in mK;
type Celsius = point TempDelta { origin: 273_150 mK; }
type Fill { id: Int; price: Money; at: Instant; wait: Bucket; temp: Celsius; }
@form(hashmap)
locus Fills { capacity { pool rows of Fill indexed_by price; } }
topic Filled { payload: Fill; subject: "t.fills"; keyed_by price; }
fn main() {
    let f = Fill { id: 1, price: 5cent, at: Instant(1nsec), wait: 300msec, temp: Celsius(0mK) };
    println(f.price);
    let book = Fills { };
    book.set(f);
    let got = book.get(5cent) or Fill { id: 0, price: 0cent, at: Instant(0nsec), wait: 0msec, temp: Celsius(0mK) };
    println(got.id);
}
"#;

/// A quantity or a point field is the `Int` it counts wherever it is laid
/// out (a hashmap key, a routing key, a flat payload), and on the wire
/// its tag names its denomination: the one tag kind U3 adds, `q(…)`, so
/// two processes whose fields count in different denominations disagree
/// in the shape hash the observer protocol compares.
#[test]
fn a_quantity_field_is_its_int_tagged_by_its_denomination() {
    let program = parse_source(WIRE).expect("parses");
    let errors: Vec<String> = check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(
        hale_types::topic_identity::canonical_type_shape(&program.items, "Fill"),
        "id:i;price:q(cent);at:q(nsec point);wait:q(100 msec);temp:q(mK point 273150 mK)"
    );
    let hash = |src: &str| {
        let program = parse_source(src).expect("parses");
        let topic = program
            .items
            .iter()
            .find_map(|i| match i {
                hale_syntax::ast::TopDecl::Topic(t) => Some(t.clone()),
                _ => None,
            })
            .expect("a topic");
        let shape = hale_types::topic_identity::canonical_topic_shape(&program.items, &topic);
        hale_types::topic_identity::topic_shape_hash("t.fills", &shape)
    };
    let in_dollars = WIRE.replace("type Money = quantity Int in cent;", "type Money = quantity Int in USD;");
    assert_ne!(hash(WIRE), hash(&in_dollars), "another denomination, another hash");
    let as_int = WIRE.replace("type Fill { id: Int; price: Money;", "type Fill { id: Int; price: Int;");
    assert_ne!(hash(WIRE), hash(&as_int), "a quantity is not tagged as a bare `Int`");
    let bundle = hale_types::Bundle::new(std::collections::BTreeMap::from([(String::new(), &program)]));
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let int = hale_types::ty::Ty::Prim(hale_syntax::ast::PrimType::Int);
    for name in ["Money", "Instant", "Bucket", "Celsius"] {
        let t = hale_types::ty::Ty::Named(name.to_string());
        assert_eq!(hale_types::ty::is_key_eligible(&t, &top), hale_types::ty::is_key_eligible(&int, &top), "{name}");
        assert_eq!(hale_types::ty::is_flat_shapeable(&t, &top), hale_types::ty::is_flat_shapeable(&int, &top), "{name}");
    }
    assert!(hale_types::ty::is_flat_shapeable(&hale_types::ty::Ty::Named("Fill".into()), &top), "a flat payload");
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
