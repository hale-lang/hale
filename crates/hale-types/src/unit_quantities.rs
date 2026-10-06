//! Values of the unit dialect's quantities and points (GH #1076, U3).
//!
//! A quantity (`type Money = quantity Int in cent;`) and a point (`type
//! Time = point Duration;`) are named types the checker knows by their
//! scope entry ([`crate::symbol::TypeKind::Scalar`]) and their row of the
//! `unit_declarations` family ([`crate::units::ScalarRow`]), by name, as
//! identities and ranges are ([`crate::unit_values`]); each is
//! represented as the `Int` it counts. An expression's type also carries
//! its denomination, which no declaration need have pinned: a
//! *synthesized* type, the same kind keyed by its quantity (or its
//! point's frame) and its denomination, named and displayed `Money in
//! 1/10000 cent` (the result of `d.in(s)`, of a sum at the meet, of a
//! ratio product). It has no policy and no range, and is an ordinary type
//! for a binding, an argument or a return. This module states, once:
//!
//! - what type a quantity literal is ([`ScalarTypes::literal_type`]);
//! - the algebra ([`ScalarTypes::quantity_binop`],
//!   [`ScalarTypes::quantity_unary`]): which operators hold, at which
//!   denomination, and the conversion each operand takes there;
//! - every conversion between two denominations as an exact [`Scale`]
//!   ([`ScalarTypes::scale`]), a widening when its denominator is one;
//! - `.in(u)`, `.split(u)` and a cast `T(x)`'s target;
//! - how a value prints ([`ScalarTypes::printed_unit`]).
//!
//! The checker records each conversion as a row of the typed bodies'
//! `conversions` column, and lowering emits one multiplication or one
//! division from the row.

use hale_syntax::Span;
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;

use crate::ty::Ty;
use crate::typed_bodies::Scale;
use crate::unit_graph::{Denom, Ratio};
use crate::unit_values::ScalarTypes;
use crate::units::{RoundPolicy, ScalarKindRow};

/// Whether a quantity's or a point's type is the one or the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QKind {
    Quantity,
    Point,
}

/// A quantity's or a point's type, as the algebra reads it.
#[derive(Debug, Clone)]
pub struct QType {
    /// The declaration the type is: the declared type's row, or for a
    /// synthesized type its base's.
    pub row: usize,
    /// What its denominations are of: a quantity's component's quantity
    /// (declared with no policy), a point's frame (the nearest point on
    /// its chain that states an origin, else the point declared with the
    /// word). Two types of one base convert by a factor alone.
    pub base: usize,
    pub kind: QKind,
    /// The component, as a unit row (`UnitRow::component`).
    pub component: usize,
    pub denom: Denom,
    /// A declared type's `round:` (its own or its nearest ancestor's).
    pub policy: Option<RoundPolicy>,
    /// A declared type's range.
    pub range: Option<(i128, i128)>,
    pub synthesized: bool,
}

/// A multiple as a type's name spells it: `100`, `1/10000`.
fn multiple_text(r: &Ratio) -> String {
    r.to_string()
}

/// A count with its thousands separated, for a message: `100,000,000`.
pub fn grouped(n: &BigInt) -> String {
    let digits = n.to_string();
    let (sign, digits) = match digits.strip_prefix('-') {
        Some(d) => ("-", d.to_string()),
        None => ("", digits),
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

fn rational(r: &Ratio) -> BigRational {
    BigRational::new(r.numerator().clone(), r.denominator().clone())
}

/// The greatest positive rational both `a` and `b` are whole multiples
/// of: the finer of two denominations' meet.
fn rational_gcd(a: &Ratio, b: &Ratio) -> Ratio {
    Ratio::new(a.numerator().gcd(b.numerator()), a.denominator().lcm(b.denominator()))
        .expect("the gcd of two positive rationals is positive")
}

impl<'r> ScalarTypes<'r> {
    /// The quantity or point `ty` is, declared or synthesized; `None` for
    /// every other type, and for a declaration the laws left without a
    /// denomination or a catalogue.
    pub fn quantity(&self, ty: &Ty) -> Option<QType> {
        let Ty::Named(name) = ty else { return None };
        if let Some(&i) = self.quantities.get(name.as_str()) {
            return self.declared(i);
        }
        let (base, spelling) = name.split_once(" in ")?;
        let &b = self.quantities.get(base)?;
        let denom = self.parse_spelling(spelling)?;
        let mut q = self.declared(b)?;
        q.denom = denom;
        q.policy = None;
        q.range = None;
        q.synthesized = true;
        Some(q)
    }

    fn declared(&self, i: usize) -> Option<QType> {
        let rows = self.rows;
        let r = &rows.scalars[i];
        let kind = match r.kind {
            ScalarKindRow::Quantity => QKind::Quantity,
            ScalarKindRow::Point => QKind::Point,
            _ => return None,
        };
        rows.catalogue.as_ref()?;
        let (component, denom) = (r.component?, r.denomination.clone()?);
        let mut policy = None;
        let mut at = Some(i);
        for _ in 0..=rows.scalars.len() {
            let Some(a) = at else { break };
            if let Some(p) = rows.scalars[a].policy {
                policy = Some(p);
                break;
            }
            at = rows.scalars[a].parent;
        }
        Some(QType { row: i, base: self.base_of(i), kind, component, denom, policy, range: r.range, synthesized: false })
    }

    /// A quantity's component's quantity, or a point's frame.
    fn base_of(&self, i: usize) -> usize {
        let rows = &self.rows.scalars;
        match rows[i].kind {
            ScalarKindRow::Point => {
                let mut at = i;
                for _ in 0..=rows.len() {
                    if rows[at].written.origin.is_some() {
                        return at;
                    }
                    match rows[at].parent {
                        Some(p) if rows[p].kind == ScalarKindRow::Point => at = p,
                        _ => return at,
                    }
                }
                at
            }
            _ if rows[i].principal => i,
            _ => rows[i].of.filter(|p| rows[*p].principal).unwrap_or(i),
        }
    }

    /// The component's quantity: what a literal of one of its units is,
    /// and what two points of it differ by.
    fn principal(&self, component: usize) -> Option<usize> {
        self.rows.scalars.iter().position(|s| s.principal && s.component == Some(component))
    }

    /// The factor from `from` to `to`, exact.
    pub fn factor(&self, from: &Denom, to: &Denom) -> Option<Ratio> {
        self.rows.catalogue.as_ref()?.factor(from, to)
    }

    fn one(&self, unit: usize) -> Denom {
        Denom { unit: self.rows.units[unit].site.id, multiple: Ratio::one() }
    }

    /// The denomination as a type's name spells it: the first unit it is
    /// one of (`msec`); else the unit it is the smallest whole multiple
    /// of (`100 msec`); else a fraction of its base's unit (`1/10000
    /// cent`).
    fn spelling(&self, base: usize, component: usize, denom: &Denom) -> String {
        let mut best: Option<(Ratio, usize)> = None;
        for (j, u) in self.rows.units.iter().enumerate() {
            if u.component != component {
                continue;
            }
            let Some(m) = self.factor(denom, &self.one(j)) else { continue };
            if m == Ratio::one() {
                return u.name.clone();
            }
            if m.is_integral() && best.as_ref().map_or(true, |(b, _)| m.numerator() < b.numerator()) {
                best = Some((m, j));
            }
        }
        if let Some((m, j)) = best {
            return format!("{} {}", multiple_text(&m), self.rows.units[j].name);
        }
        let unit = self.rows.scalars[base]
            .denomination
            .as_ref()
            .and_then(|d| self.rows.units.iter().position(|u| u.site.id == d.unit))
            .unwrap_or(component);
        let m = self.factor(denom, &self.one(unit)).unwrap_or_else(|| denom.multiple.clone());
        format!("{} {}", multiple_text(&m), self.rows.units[unit].name)
    }

    fn parse_spelling(&self, spelling: &str) -> Option<Denom> {
        let (multiple, unit) = match spelling.split_once(' ') {
            Some((m, u)) => {
                let (n, d) = m.split_once('/').unwrap_or((m, "1"));
                (Ratio::new(n.parse().ok()?, d.parse().ok()?)?, u)
            }
            None => (Ratio::one(), spelling),
        };
        let u = self.rows.unit_named(unit)?;
        Some(Denom { unit: self.rows.units[u].site.id, multiple })
    }

    /// The type of `base`'s kind at `denom`: `base` itself when that is
    /// its own denomination, else the synthesized type.
    pub fn at_denomination(&self, base: usize, denom: &Denom) -> Ty {
        let row = &self.rows.scalars[base];
        let Some(own) = &row.denomination else { return Ty::Named(row.name.clone()) };
        if self.factor(denom, own) == Some(Ratio::one()) {
            return Ty::Named(row.name.clone());
        }
        let component = row.component.unwrap_or(0);
        Ty::Named(format!("{} in {}", row.name, self.spelling(base, component, denom)))
    }

    /// A quantity's or a point's type as a message writes it.
    pub fn quantity_display(&self, q: &QType) -> String {
        let row = &self.rows.scalars[q.row];
        if q.synthesized {
            format!("{} in {}", row.display, self.spelling(q.base, q.component, &q.denom))
        } else {
            row.display.clone()
        }
    }

    /// `t` as a message writes it.
    pub fn type_display(&self, t: &Ty) -> String {
        match self.quantity(t) {
            Some(q) => self.quantity_display(&q),
            None => match t {
                Ty::Named(n) => self.rows.scalars.iter().find(|s| &s.name == n).map_or_else(|| t.display(), |s| s.display.clone()),
                _ => t.display(),
            },
        }
    }

    /// The note at the declaration a quantity's type is (a synthesized
    /// type's base).
    fn note(&self, q: &QType) -> (Span, String) {
        let row = &self.rows.scalars[q.row];
        (row.name_span, format!("`{}` is declared here", row.display))
    }

    /// The type a quantity literal of `unit` is: its component's
    /// quantity, at the literal's own unit; or why there is none.
    pub fn literal_type(&self, value: i64, unit: &str) -> Result<Ty, String> {
        let Some(u) = self.rows.unit_named(unit) else {
            let suggestion = crate::stdlib_surface::nearest_name(unit, self.rows.units.iter().map(|u| u.name.as_str()))
                .map(|n| format!("; did you mean `{n}`?"))
                .unwrap_or_default();
            return Err(format!("`{value}{unit}`: no `unit` declares `{unit}`{suggestion}"));
        };
        let component = self.rows.units[u].component;
        let Some(p) = self.principal(component) else {
            return Err(format!(
                "`{value}{unit}`: the units of `{unit}` have no quantity: declare one (`type T = quantity Int in {unit};`)"
            ));
        };
        if !self.quantities.contains_key(self.rows.scalars[p].name.as_str()) || self.rows.catalogue.is_none() {
            return Ok(Ty::Unknown);
        }
        Ok(self.at_denomination(p, &self.one(u)))
    }

    /// The finer of two denominations of one component: the coarsest
    /// both are whole multiples of.
    pub fn meet(&self, a: &Denom, b: &Denom) -> Option<Denom> {
        let anchor = Denom { unit: a.unit, multiple: Ratio::one() };
        let (ma, mb) = (self.factor(a, &anchor)?, self.factor(b, &anchor)?);
        Some(Denom { unit: a.unit, multiple: rational_gcd(&ma, &mb) })
    }

    /// An origin's count, as a count of `to`.
    fn origin_in(&self, frame: usize, to: &Denom) -> Option<BigRational> {
        let row = &self.rows.scalars[frame];
        let Some(origin) = &row.origin else { return Some(BigRational::from_integer(BigInt::from(0))) };
        let own = row.denomination.as_ref()?;
        Some(origin * rational(&self.factor(own, to)?))
    }

    /// The conversion of a value of `from` into `to`, two types of one
    /// component and kind (a quantity into a point is the point at that
    /// count from its origin): the exact factor, and a point's shift by
    /// the two origins' difference. `Err` when a point's shift is no
    /// whole count of the target, or overflows an `Int`.
    pub fn scale(&self, from: &QType, to: &QType) -> Result<Scale, String> {
        let factor = self.factor(&from.denom, &to.denom).ok_or_else(|| "two components".to_string())?;
        let shift = match (from.kind, to.kind) {
            (QKind::Point, QKind::Point) if from.base != to.base => {
                let a = self.origin_in(from.base, &to.denom).unwrap_or_default();
                let b = self.origin_in(to.base, &to.denom).unwrap_or_default();
                a - b
            }
            _ => BigRational::from_integer(BigInt::from(0)),
        };
        let k = shift * BigRational::from_integer(factor.denominator().clone());
        if !k.is_integer() {
            return Err(format!(
                "the origins of `{}` and `{}` differ by a fraction of `{}`'s denomination, which no count of it holds",
                self.quantity_display(from),
                self.quantity_display(to),
                self.quantity_display(to)
            ));
        }
        let offset = i64::try_from(k.to_integer()).map_err(|_| {
            format!(
                "the origins of `{}` and `{}` differ by more than an `Int` counts",
                self.quantity_display(from),
                self.quantity_display(to)
            )
        })?;
        Ok(Scale { factor, offset })
    }

    /// A unit of `q`'s denomination, for a hint (`cent`).
    pub fn unit_hint(&self, q: &QType) -> String {
        self.rows.units.iter().find(|u| u.site.id == q.denom.unit).map_or_else(String::new, |u| u.name.clone())
    }

    /// `q`'s quantity, or its point's frame, as a message writes it.
    pub fn base_display(&self, q: &QType) -> String {
        self.rows.scalars[q.base].display.clone()
    }

    /// One note at each declaration `sides` are (a synthesized type's
    /// base's), once.
    pub fn notes(&self, sides: &[Option<&QType>]) -> Vec<(Span, String)> {
        let mut notes: Vec<(Span, String)> = Vec::new();
        for q in sides.iter().flatten() {
            let n = self.note(q);
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
        notes
    }
    /// Why a cast `to(x)` of a value of `from` is refused, or the
    /// conversion it is. `to` is a declared quantity or point.
    pub fn quantity_cast(&self, to: &QType, from: &Ty) -> Result<Scale, String> {
        let target = self.quantity_display(to);
        let Some(f) = self.quantity(from) else {
            return Err(match from {
                Ty::Prim(hale_syntax::ast::PrimType::Int) => format!(
                    "`{target}(…)` of an `Int`: a count becomes a quantity by a unit (`n * 1{}`)",
                    self.unit_hint(to)
                ),
                other => format!("`{target}(…)` converts a quantity or a point of its own; got `{}`", self.type_display(other)),
            });
        };
        if f.component != to.component {
            return Err(format!(
                "`{target}(…)` of `{}`: different quantities, `{}` and `{}`; no conversion holds between them",
                self.quantity_display(&f),
                self.rows.scalars[to.base].display,
                self.rows.scalars[f.base].display
            ));
        }
        if f.kind == QKind::Point && to.kind == QKind::Quantity {
            return Err(format!(
                "`{target}(…)` of the point `{}`: a point is no quantity; the quantity between two points is their \
                 difference (`p - q`)",
                self.quantity_display(&f)
            ));
        }
        self.scale(&f, to)
    }
}

/// The count `value` of `from`'s denomination as a count of the target,
/// under `scale`, rounded by `policy` when the factor divides: the
/// checker's compile-time conversion of a quantity literal. `None` when
/// the division leaves a remainder and no policy says what becomes of
/// it, or the count overflows an `Int`.
pub fn convert_count(value: i64, scale: &Scale, policy: Option<RoundPolicy>) -> Option<Result<i64, ()>> {
    let n = BigInt::from(value) * scale.factor.numerator() + BigInt::from(scale.offset);
    let d = scale.factor.denominator().clone();
    let (q, r) = n.div_rem(&d);
    let rounded = if r == BigInt::from(0) {
        q
    } else {
        let policy = policy?;
        let neg = r < BigInt::from(0);
        let twice = BigInt::from(2) * if neg { -r.clone() } else { r.clone() };
        let away = if neg { q.clone() - 1 } else { q.clone() + 1 };
        match policy {
            RoundPolicy::Trunc => q,
            RoundPolicy::Floor => if neg { away } else { q },
            RoundPolicy::Ceil => if neg { q } else { away },
            RoundPolicy::HalfUp => if twice >= d { away } else { q },
            RoundPolicy::HalfEven => {
                if twice > d || (twice == d && q.is_odd()) {
                    away
                } else {
                    q
                }
            }
        }
    };
    Some(i64::try_from(rounded).map_err(|_| ()))
}
