//! `hale check --units`: the unit dialect's witness report (GH #1076, U5).
//!
//! A query over two things the check already holds, and no re-check: the
//! `unit_declarations` rows ([`crate::units::UnitRows`]) and the typed
//! bodies' `conversions` column ([`crate::typed_bodies::TypedBodies`]).
//! For each scalar declaration of the program it says what the rows say:
//! a quantity's or a point's denomination and the declaration that fixed
//! it, its policy and where that is written, a point's origin, the
//! headroom of its representation (`Int`'s range counted in its
//! denomination, stated in the unit the denomination is written against
//! and in the coarsest unit of its catalogue that still counts one), and
//! what a declared range fits in. For each narrowing the checker
//! recorded, it says the factor (or the range), the policy that discharged
//! it and where the policy came from: the site's `or`, or the target
//! type's `round:`.
//!
//! It is a development report, outside the hashed model half: no shape
//! hash reads it. Its text is stable (declarations in program order,
//! narrowings by site), so it can be recorded and diffed; `--json` is the
//! same fields as one object.

use std::collections::BTreeSet;

use hale_syntax::Span;
use num_bigint::BigInt;
use num_rational::BigRational;

use crate::placement::SiteUniverse;
use crate::typed_bodies::{ConversionKind, Discharge, TypedBodies};
use crate::unit_graph::Denom;
use crate::unit_quantities::QType;
use crate::unit_values::ScalarTypes;
use crate::units::{Base, ScalarKindRow, UnitRows};

/// A place in the program: where it is (`main.hl:12:5`) and its source
/// text, its whitespace collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub at: String,
    pub text: String,
}

/// The report.
#[derive(Debug, Clone, Default)]
pub struct UnitReport {
    /// The program's scalar declarations, in program order.
    pub declarations: Vec<Declaration>,
    /// Every narrowing the checker recorded, by site.
    pub narrowings: Vec<Narrowing>,
}

/// One scalar declaration.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub declared: Place,
    /// `quantity`, `point`, `identity` or `range`.
    pub kind: &'static str,
    /// A boundary denomination's quantity, a point's quantity, a range's
    /// parent (`Int` for a range written over it).
    pub of: Option<String>,
    /// A quantity's or a point's denomination as a type's name spells it,
    /// and the declaration whose `in` fixed it.
    pub denomination: Option<(String, Place)>,
    /// A point's origin as written (`273150 mK`).
    pub origin: Option<String>,
    /// Its `round:` policy and the declaration that writes it (its own or
    /// an ancestor's).
    pub policy: Option<(&'static str, Place)>,
    /// Half-open.
    pub range: Option<(i128, i128)>,
    pub headroom: Option<Headroom>,
    /// The narrowest machine width a declared range fits in.
    pub fits_in: Option<&'static str>,
}

/// The largest count of an `Int` at a denomination, in the unit the
/// denomination is written against, and in the coarsest unit of the
/// catalogue it is still one or more of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headroom {
    pub count: BigInt,
    pub unit: String,
    pub coarse: Option<(BigInt, String)>,
}

/// One narrowing: a conversion the checker recorded that divides (a
/// quantity's or a point's) or that may leave a range (an identity's or
/// a range's).
#[derive(Debug, Clone)]
pub struct Narrowing {
    pub site: Place,
    pub from: String,
    pub to: String,
    /// The exact factor `p/q`, `q > 1`, of a quantity's or a point's.
    pub factor: Option<String>,
    /// The range a value may be outside of, of an identity's or a range's.
    pub range: Option<(i128, i128)>,
    /// A literal's count, converted (and rounded) at compile time.
    pub literal: Option<i64>,
    /// The discharge as the program writes it (`or floor`, `or <value>`);
    /// `None` for a narrowing nothing discharges, which the check refuses.
    pub policy: Option<&'static str>,
    /// Where the policy came from: `None` at the site, else the target
    /// type's declaration that writes the `round:`.
    pub policy_from: Option<Place>,
    /// The headroom of the source when its denomination is one no
    /// declaration names (a ratio product's, a sum's meet): each finer
    /// denomination holds less.
    pub headroom: Option<(String, Headroom)>,
}

/// The report over `rows` and `typed`. `place` resolves a span of the
/// program's own files; a span of the stdlib's (`Duration`, `Time`) is
/// resolved here.
pub fn report(rows: &UnitRows, typed: &TypedBodies, place: &dyn Fn(Span) -> Option<Place>) -> UnitReport {
    let types = ScalarTypes::of_rows(rows);
    let at = |universe: SiteUniverse, span: Span| -> Place {
        let found = match universe {
            SiteUniverse::User => place(span),
            _ => stdlib_place(span),
        };
        found.unwrap_or_else(|| Place { at: "(no location)".into(), text: String::new() })
    };
    let declaration_of = |i: usize| -> Place {
        let s = &rows.scalars[i];
        at(s.site.universe, s.span)
    };
    let mut declarations = Vec::new();
    for (i, s) in rows.scalars.iter().enumerate() {
        if s.site.universe != SiteUniverse::User {
            continue;
        }
        let q = types.quantity(&s.ty());
        let kind = match s.kind {
            ScalarKindRow::Quantity => "quantity",
            ScalarKindRow::Point => "point",
            ScalarKindRow::Identity => "identity",
            ScalarKindRow::Range => "range",
        };
        let of = match s.kind {
            ScalarKindRow::Quantity if s.principal => None,
            ScalarKindRow::Quantity | ScalarKindRow::Point => s.of.map(|o| rows.scalars[o].display.clone()),
            ScalarKindRow::Identity => None,
            ScalarKindRow::Range => match &s.written.base {
                Base::Scalar(p) => Some(rows.scalars[*p].display.clone()),
                _ => Some("Int".into()),
            },
        };
        let denomination = q.as_ref().and_then(|q| {
            let fixed = along(rows, i, |r| r.written.denomination.is_some())?;
            Some((types.denomination_display(q), declaration_of(fixed)))
        });
        let origin = s.written.origin.as_ref().map(|o| format!("{} {}", o.value, rows.refs[o.unit].name));
        let policy = along(rows, i, |r| r.policy.is_some()).map(|p| {
            let name = rows.scalars[p].policy.expect("the row found by its policy").name();
            (name, declaration_of(p))
        });
        declarations.push(Declaration {
            name: s.display.clone(),
            declared: declaration_of(i),
            kind,
            of,
            denomination,
            origin,
            policy,
            range: s.range,
            headroom: q.as_ref().and_then(|q| headroom(rows, q)),
            fits_in: s.range.map(|(lo, hi)| fits_in(lo, hi)),
        });
    }

    let mut narrowings: Vec<(Span, Narrowing)> = Vec::new();
    let mut seen = BTreeSet::new();
    for (_, row) in typed.conversion_sites() {
        if row.kind != ConversionKind::Narrowing {
            continue;
        }
        let factor = row.scale.as_ref().map(|s| s.factor.to_string());
        let (policy, from_type) = match row.policy {
            None => (None, false),
            Some(Discharge::Round { from_type, .. }) => (row.policy.map(Discharge::spelled), from_type),
            Some(d) => (Some(d.spelled()), false),
        };
        let target = types.quantity(&row.to);
        let policy_from = if from_type {
            target.as_ref().and_then(|t| along(rows, t.row, |r| r.policy.is_some())).map(declaration_of)
        } else {
            None
        };
        let source = types.quantity(&row.from).filter(|q| q.synthesized);
        let headroom = source.as_ref().and_then(|q| Some((types.quantity_display(q), headroom(rows, q)?)));
        let n = Narrowing {
            site: at(SiteUniverse::User, row.span),
            from: types.type_display(&row.from),
            to: types.type_display(&row.to),
            factor,
            range: if row.scale.is_none() { row.range } else { None },
            literal: row.count,
            policy,
            policy_from,
            headroom,
        };
        // A default evaluated in several places has a row per evaluation:
        // one line says them all when they agree.
        if seen.insert((row.span.start.as_usize(), row.span.end.as_usize(), render_narrowing(&n))) {
            narrowings.push((row.span, n));
        }
    }
    narrowings.sort_by_key(|(span, n)| (span.start.as_usize(), span.end.as_usize(), render_narrowing(n)));
    UnitReport { declarations, narrowings: narrowings.into_iter().map(|(_, n)| n).collect() }
}

/// The first scalar on `i`'s chain of parents (`i` itself first) that
/// `has`.
fn along(rows: &UnitRows, i: usize, has: impl Fn(&crate::units::ScalarRow) -> bool) -> Option<usize> {
    let mut at = Some(i);
    for _ in 0..=rows.scalars.len() {
        let a = at?;
        if has(&rows.scalars[a]) {
            return Some(a);
        }
        at = rows.scalars[a].parent;
    }
    None
}

/// A stdlib declaration's place: its file and line in the stdlib, and
/// its text.
fn stdlib_place(span: Span) -> Option<Place> {
    let at = crate::stdlib_bodies::stdlib_span_location(span)?;
    let text = hale_stdlib::AP_SOURCE.get(span.start.as_usize()..span.end.as_usize())?;
    Some(Place { at: format!("std::time, {at}"), text: collapsed(text) })
}

/// `text` on one line: every run of whitespace one space, a trailing `;`
/// dropped.
pub fn collapsed(text: &str) -> String {
    let one: Vec<&str> = text.split_whitespace().collect();
    one.join(" ").trim_end_matches(';').trim_end().to_string()
}

/// The headroom of an `Int` counted in `q`'s denomination.
fn headroom(rows: &UnitRows, q: &QType) -> Option<Headroom> {
    let catalogue = rows.catalogue.as_ref()?;
    let max = BigRational::from_integer(BigInt::from(i64::MAX));
    let count_in = |to: &Denom| -> Option<BigInt> {
        let f = catalogue.factor(&q.denom, to)?;
        Some((&max * BigRational::new(f.numerator().clone(), f.denominator().clone())).floor().to_integer())
    };
    let own = rows.units.iter().position(|u| u.site == q.denom.unit)?;
    let one = |u: usize| Denom { unit: rows.units[u].site, multiple: crate::unit_graph::Ratio::one() };
    let count = count_in(&one(own))?;
    let component = rows.units[own].component;
    let mut coarse: Option<(BigInt, usize)> = None;
    for (u, row) in rows.units.iter().enumerate() {
        if row.component != component || u == own {
            continue;
        }
        let Some(c) = count_in(&one(u)) else { continue };
        if c >= BigInt::from(1) && coarse.as_ref().map_or(true, |(best, _)| c < *best) {
            coarse = Some((c, u));
        }
    }
    let coarse = coarse.filter(|(c, _)| *c < count).map(|(c, u)| (c, rows.units[u].name.clone()));
    Some(Headroom { count, unit: rows.units[own].name.clone(), coarse })
}

/// The narrowest machine width that holds every value of the half-open
/// range `lo..hi`.
pub fn fits_in(lo: i128, hi: i128) -> &'static str {
    let top = hi - 1;
    if lo < 0 {
        return "i64";
    }
    if top <= i128::from(u8::MAX) {
        "u8"
    } else if top <= i128::from(u16::MAX) {
        "u16"
    } else if top <= i128::from(u32::MAX) {
        "u32"
    } else if top <= i128::from(u64::MAX) {
        "u64"
    } else {
        "i64"
    }
}

fn headroom_text(h: &Headroom) -> String {
    match &h.coarse {
        Some((c, u)) => format!("±{} {} ({} {})", h.count, h.unit, c, u),
        None => format!("±{} {}", h.count, h.unit),
    }
}

fn place_text(p: &Place) -> String {
    format!("`{}` ({})", p.text, p.at)
}

fn render_narrowing(n: &Narrowing) -> String {
    let mut out = format!("{}  {}\n", n.site.at, n.site.text);
    let by = match (&n.factor, n.range) {
        (Some(f), _) => format!(", factor {f}"),
        (None, Some((lo, hi))) => format!(", range {lo}..{hi}"),
        (None, None) => String::new(),
    };
    out.push_str(&format!("    {} -> {}{by}\n", n.from, n.to));
    if let Some(count) = n.literal {
        out.push_str(&format!("    literal      : {count}, converted at compile time\n"));
    }
    let policy = match (n.policy, &n.policy_from) {
        (None, _) => "none (the check refuses it)".to_string(),
        (Some(p), None) => format!("{p}, at the site"),
        (Some(p), Some(from)) => {
            format!("{}, the `round:` of {}", p.trim_start_matches("or "), place_text(from))
        }
    };
    out.push_str(&format!("    policy       : {policy}\n"));
    if let Some((of, h)) = &n.headroom {
        out.push_str(&format!("    headroom     : {}, of {of}\n", headroom_text(h)));
    }
    out
}

/// The report as text: one header line, then each declaration, then each
/// narrowing. With nothing to say it is the one line `units: no quantity
/// is declared`.
pub fn render(report: &UnitReport) -> String {
    if report.declarations.is_empty() && report.narrowings.is_empty() {
        return "units: no quantity is declared\n".to_string();
    }
    let plural = |n: usize, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    let mut out = format!(
        "units: {}, {}\n",
        plural(report.declarations.len(), "declaration"),
        plural(report.narrowings.len(), "narrowing")
    );
    for d in &report.declarations {
        out.push_str(&format!("\n{}  {}\n", d.declared.at, d.declared.text));
        let kind = match &d.of {
            Some(of) if d.kind == "quantity" => format!("quantity, a denomination of {of}"),
            Some(of) => format!("{} of {of}", d.kind),
            None => d.kind.to_string(),
        };
        out.push_str(&format!("    kind         : {kind}\n"));
        if let Some((denomination, fixed)) = &d.denomination {
            let by = if *fixed == d.declared { "its own `in`".to_string() } else { place_text(fixed) };
            out.push_str(&format!("    denomination : {denomination}, fixed by {by}\n"));
        }
        if let Some(origin) = &d.origin {
            out.push_str(&format!("    origin       : {origin}\n"));
        }
        if let Some((policy, from)) = &d.policy {
            let by = if *from == d.declared { "its own `round:`".to_string() } else { place_text(from) };
            out.push_str(&format!("    policy       : {policy}, from {by}\n"));
        }
        if let Some((lo, hi)) = d.range {
            out.push_str(&format!("    range        : {lo}..{hi}\n"));
        }
        if let Some(h) = &d.headroom {
            out.push_str(&format!("    headroom     : {}\n", headroom_text(h)));
        }
        if let Some(width) = d.fits_in {
            out.push_str(&format!("    fits in      : {width}\n"));
        }
    }
    if !report.narrowings.is_empty() {
        out.push_str("\nnarrowings:\n");
        for n in &report.narrowings {
            out.push_str(&format!("\n{}", render_narrowing(n)));
        }
    }
    out
}

fn place_json(p: &Place) -> serde_json::Value {
    serde_json::json!({ "at": p.at, "text": p.text })
}

fn headroom_json(h: &Headroom) -> serde_json::Value {
    serde_json::json!({
        "count": h.count.to_string(),
        "unit": h.unit,
        "coarse": h.coarse.as_ref().map(|(c, u)| serde_json::json!({ "count": c.to_string(), "unit": u })),
    })
}

/// The report as one JSON object: the text's fields, a big count as a
/// string (a headroom need not fit a JSON number).
pub fn render_json(report: &UnitReport) -> serde_json::Value {
    let declarations: Vec<serde_json::Value> = report
        .declarations
        .iter()
        .map(|d| {
            serde_json::json!({
                "name": d.name,
                "declared": place_json(&d.declared),
                "kind": d.kind,
                "of": d.of,
                "denomination": d.denomination.as_ref().map(|(s, _)| s),
                "denomination_fixed_by": d.denomination.as_ref().map(|(_, p)| place_json(p)),
                "origin": d.origin,
                "policy": d.policy.as_ref().map(|(p, _)| p),
                "policy_from": d.policy.as_ref().map(|(_, p)| place_json(p)),
                "range": d.range.map(|(lo, hi)| [lo.to_string(), hi.to_string()]),
                "headroom": d.headroom.as_ref().map(headroom_json),
                "fits_in": d.fits_in,
            })
        })
        .collect();
    let narrowings: Vec<serde_json::Value> = report
        .narrowings
        .iter()
        .map(|n| {
            serde_json::json!({
                "site": place_json(&n.site),
                "from": n.from,
                "to": n.to,
                "factor": n.factor,
                "range": n.range.map(|(lo, hi)| [lo.to_string(), hi.to_string()]),
                "literal": n.literal,
                "policy": n.policy,
                "policy_from": n.policy_from.as_ref().map(place_json),
                "headroom": n.headroom.as_ref().map(|(of, h)| {
                    let mut v = headroom_json(h);
                    v["of"] = serde_json::Value::String(of.clone());
                    v
                }),
            })
        })
        .collect();
    serde_json::json!({ "report": "units", "declarations": declarations, "narrowings": narrowings })
}
