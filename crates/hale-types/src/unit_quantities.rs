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

use hale_syntax::ast::{BinOp, Expr, Literal, UnaryOp};
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

/// An operand's conversion the algebra asks for: from its type to the
/// denomination the operator holds at.
#[derive(Debug, Clone)]
pub struct Operand {
    pub from: Ty,
    pub to: Ty,
    pub scale: Scale,
}

/// What [`ScalarTypes::quantity_binop`] decided.
#[derive(Debug, Clone)]
pub enum QBinop {
    /// The operator's result, and each operand's conversion where it
    /// changes denomination (a widening, exact).
    Typed { ty: Ty, left: Option<Operand>, right: Option<Operand> },
    /// `q / n`, `n` an integer literal other than one: the quantity
    /// divided, a narrowing the site discharges.
    DividedByLiteral { ty: Ty, divisor: i64 },
    /// Refused, with the message and a note at each declaration.
    Refused { message: String, notes: Vec<(Span, String)> },
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
        let name = match ty {
            // The stdlib's `Duration` and `Time` (U4).
            Ty::Prim(p) => {
                let &i = self.quantities.get(crate::ty::prim_name(*p))?;
                return (self.rows.scalars[i].primitive == Some(*p)).then(|| self.declared(i)).flatten();
            }
            Ty::Named(name) => name,
            _ => return None,
        };
        if let Some(&i) = self.quantities.get(name.as_str()) {
            // The stdlib's declarations are the primitives, which a name
            // never reaches (`Ty::Named("Duration")` is no type).
            if self.rows.scalars[i].primitive.is_some() {
                return None;
            }
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
        self.rows.principal(component)
    }

    /// The factor from `from` to `to`, exact.
    pub fn factor(&self, from: &Denom, to: &Denom) -> Option<Ratio> {
        self.rows.catalogue.as_ref()?.factor(from, to)
    }

    fn one(&self, unit: usize) -> Denom {
        Denom { unit: self.rows.units[unit].site, multiple: Ratio::one() }
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
            .and_then(|d| self.rows.units.iter().position(|u| u.site == d.unit))
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
        Some(Denom { unit: self.rows.units[u].site, multiple })
    }

    /// The type of `base`'s kind at `denom`: `base` itself when that is
    /// its own denomination, else the synthesized type.
    pub fn at_denomination(&self, base: usize, denom: &Denom) -> Ty {
        let row = &self.rows.scalars[base];
        let Some(own) = &row.denomination else { return row.ty() };
        if self.factor(denom, own) == Some(Ratio::one()) {
            return row.ty();
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
    fn note(&self, q: &QType) -> Option<(Span, String)> {
        let row = &self.rows.scalars[q.row];
        // The stdlib's declarations (`Duration`, `Time`) are in no file
        // of the program.
        (row.site.universe == crate::placement::SiteUniverse::User)
            .then(|| (row.name_span, format!("`{}` is declared here", row.display)))
    }

    /// The type a quantity literal of `unit` is, and its count there: its
    /// component's quantity at the quantity's denomination when the
    /// literal is a whole count of it (`500ms` is 500,000,000 of
    /// `Duration`, `3USD` 300 of `Money` in cent), else at the literal's
    /// own unit (`5mK` of a quantity in `K` is `TempDelta in mK`, 5); or
    /// why there is none.
    pub fn quantity_literal(&self, value: i64, unit: &str) -> Result<(Ty, i64), String> {
        let ty = self.literal_type(value, unit)?;
        let at_quantity = self.rows.unit_named(unit).and_then(|u| self.rows.literal_at_quantity(value, u));
        Ok(match at_quantity {
            Some((p, count)) if !matches!(ty, Ty::Unknown) => (self.rows.scalars[p].ty(), count),
            _ => (ty, value),
        })
    }

    /// The type a quantity literal of `unit` is at its own unit (its
    /// component's quantity there); or why there is none.
    fn literal_type(&self, value: i64, unit: &str) -> Result<Ty, String> {
        let Some(u) = self.rows.unit_named(unit) else {
            // The lexer reads every name after the digits as one unit, so
            // `1h30m` arrives here as `1` of `h30m`.
            if let Some(why) = self.rows.compound_literal(value, unit) {
                return Err(why);
            }
            return Err(format!("`{value}{unit}`: no `unit` declares `{unit}`{}", self.rows.unknown_unit_hint(unit)));
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

    fn operand(&self, t: &Ty, q: &QType, to: &Ty, at: &Denom) -> Option<Operand> {
        let target = QType { denom: at.clone(), ..q.clone() };
        let scale = self.scale(q, &target).ok()?;
        (scale.factor != Ratio::one() || scale.offset != 0).then(|| Operand { from: t.clone(), to: to.clone(), scale })
    }

    /// The same kind at `at`, each operand converted there.
    fn at_meet(&self, lt: &Ty, l: &QType, rt: &Ty, r: &QType, result_base: usize) -> Option<QBinop> {
        let at = self.meet(&l.denom, &r.denom)?;
        let ty = self.at_denomination(result_base, &at);
        let lto = self.at_denomination(l.base, &at);
        let rto = self.at_denomination(r.base, &at);
        Some(QBinop::Typed { ty, left: self.operand(lt, l, &lto, &at), right: self.operand(rt, r, &rto, &at) })
    }

    fn refused(&self, message: String, sides: &[&QType]) -> QBinop {
        let mut notes: Vec<(Span, String)> = Vec::new();
        for q in sides {
            let Some(n) = self.note(q) else { continue };
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
        QBinop::Refused { message, notes }
    }

    /// A binary operator with a quantity or a point on either side; `None`
    /// when neither is one (or the other side is not known).
    pub fn quantity_binop(&self, op: BinOp, lt: &Ty, rt: &Ty, _l: &Expr, r: &Expr) -> Option<QBinop> {
        use BinOp::*;
        let (lq, rq) = (self.quantity(lt), self.quantity(rt));
        if lq.is_none() && rq.is_none() {
            return None;
        }
        if matches!(op, And | Or) || matches!(lt, Ty::Unknown) || matches!(rt, Ty::Unknown) {
            return None;
        }
        let name = |t: &Ty| self.type_display(t);
        let sym = op_symbol(op);
        let is_int = |t: &Ty| matches!(t, Ty::Prim(hale_syntax::ast::PrimType::Int));
        let scalar_other = |t: &Ty| self.index(t).is_some();
        let quantity_side = lq.as_ref().or(rq.as_ref()).expect("one side is a quantity or a point");
        // A quantity with an identity or a range: nothing holds.
        if scalar_other(lt) || scalar_other(rt) {
            let (q_ty, other) = if scalar_other(lt) { (rt, lt) } else { (lt, rt) };
            return Some(self.refused(
                format!(
                    "`{}` {sym} `{}`: `{}` is {} and `{}` is an identity or a range; nothing holds between them",
                    name(lt),
                    name(rt),
                    name(q_ty),
                    article(quantity_side),
                    name(other)
                ),
                &[quantity_side],
            ));
        }
        let both = |m: String| self.refused(m, &[lq.as_ref(), rq.as_ref()].into_iter().flatten().collect::<Vec<_>>());
        match (op, lq.as_ref(), rq.as_ref()) {
            // Sums and differences.
            (Add | Sub, Some(a), Some(b)) if a.component != b.component => Some(both(format!(
                "`{}` {sym} `{}`: different quantities, `{}` and `{}`; `{sym}` holds within one",
                name(lt),
                name(rt),
                self.rows.scalars[a.base].display,
                self.rows.scalars[b.base].display
            ))),
            (Add | Sub, Some(a), Some(b)) => match (a.kind, b.kind, op) {
                (QKind::Quantity, QKind::Quantity, _) => self.at_meet(lt, a, rt, b, a.base),
                (QKind::Point, QKind::Quantity, _) => self.at_meet(lt, a, rt, b, a.base),
                (QKind::Quantity, QKind::Point, Add) => self.at_meet(lt, a, rt, b, b.base),
                (QKind::Point, QKind::Point, Sub) if a.base == b.base => {
                    let p = self.principal(a.component)?;
                    let at = self.meet(&a.denom, &b.denom)?;
                    let ty = self.at_denomination(p, &at);
                    let to = self.at_denomination(a.base, &at);
                    Some(QBinop::Typed { ty, left: self.operand(lt, a, &to, &at), right: self.operand(rt, b, &to, &at) })
                }
                (QKind::Point, QKind::Point, Sub) => Some(both(format!(
                    "`{}` - `{}`: two points of different origins; convert one to the other's first (`{}(…)`)",
                    name(lt),
                    name(rt),
                    self.rows.scalars[a.row].display
                ))),
                (QKind::Point, QKind::Point, _) => Some(both(format!(
                    "`{}` + `{}`: two points do not add; their difference is a quantity (`b - a`), and a point moves \
                     by a quantity (`a + d`)",
                    name(lt),
                    name(rt)
                ))),
                _ => Some(both(format!(
                    "`{}` - `{}`: a quantity minus a point has no meaning; a point minus a quantity is a point",
                    name(lt),
                    name(rt)
                ))),
            },
            (Add | Sub, _, _) => Some(both(format!(
                "`{}` {sym} `{}`: an `Int` is no quantity; a count becomes one by a unit (`n * 1{}`)",
                name(lt),
                name(rt),
                self.unit_hint(quantity_side)
            ))),
            // Products.
            (Mul, Some(q), None) | (Mul, None, Some(q)) if q.kind == QKind::Quantity && (is_int(lt) || is_int(rt)) => {
                let t = if lq.is_some() { lt } else { rt };
                Some(QBinop::Typed { ty: t.clone(), left: None, right: None })
            }
            // A quantity scaled by a dimensionless one (a ratio): the
            // quantity at the product of the two denominations, exact
            // (decision 2). With two ratios, the left is the quantity.
            (Mul, Some(a), Some(b)) if a.kind == QKind::Quantity && b.kind == QKind::Quantity => {
                let (q, v) = match (self.rows.pure_value(&b.denom), self.rows.pure_value(&a.denom)) {
                    (Some(v), _) => (a, v),
                    (None, Some(v)) => (b, v),
                    (None, None) => {
                        return Some(both(format!(
                            "`{}` * `{}`: a product of two quantities is a quantity only when one is dimensionless \
                             (a ratio); neither is",
                            name(lt),
                            name(rt)
                        )))
                    }
                };
                let at = Denom { unit: q.denom.unit, multiple: q.denom.multiple.times(&v) };
                Some(QBinop::Typed { ty: self.at_denomination(q.base, &at), left: None, right: None })
            }
            (Mul, _, _) => Some(both(format!(
                "`{}` * `{}`: {}",
                name(lt),
                name(rt),
                if quantity_side.kind == QKind::Point {
                    "a point has no product; scale the quantity it is from (`(p - origin) * n`)"
                } else {
                    "a quantity is scaled by an `Int` or by a dimensionless quantity"
                }
            ))),
            // Quotients.
            (Div, Some(q), None) if q.kind == QKind::Quantity && is_int(rt) => {
                match crate::unit_values::int_literal(r) {
                    Some(1) | None => Some(QBinop::Typed { ty: lt.clone(), left: None, right: None }),
                    Some(0) => Some(both(format!("`{}` / 0: a division by zero", name(lt)))),
                    Some(n) if n < 0 => Some(both(format!(
                        "`{}` / {n}: a literal divisor is positive; divide by `{}` and negate the quotient",
                        name(lt),
                        -n
                    ))),
                    Some(n) => Some(QBinop::DividedByLiteral { ty: lt.clone(), divisor: i64::try_from(n).ok()? }),
                }
            }
            (Div, Some(a), Some(b)) if a.kind == QKind::Quantity && b.kind == QKind::Quantity && a.component == b.component => {
                let at = self.meet(&a.denom, &b.denom)?;
                let lto = self.at_denomination(a.base, &at);
                let rto = self.at_denomination(b.base, &at);
                Some(QBinop::Typed {
                    ty: Ty::Prim(hale_syntax::ast::PrimType::Int),
                    left: self.operand(lt, a, &lto, &at),
                    right: self.operand(rt, b, &rto, &at),
                })
            }
            (Div, Some(a), Some(b)) if a.kind == QKind::Quantity && b.kind == QKind::Quantity => {
                Some(both(format!(
                    "`{}` / `{}`: a quotient of two quantities is a count only within one quantity (`q / 1{}`); \
                     these are `{}` and `{}`",
                    name(lt),
                    name(rt),
                    self.unit_hint(a),
                    self.rows.scalars[a.base].display,
                    self.rows.scalars[b.base].display
                )))
            }
            (Div, _, _) => Some(both(format!(
                "`{}` / `{}`: a quantity is divided by an `Int` or by a quantity of its own; {}",
                name(lt),
                name(rt),
                "a point is not divided"
            ))),
            (Mod | BitAnd | BitOr | BitXor | Shl | Shr, _, _) => Some(both(format!(
                "`{}` {sym} `{}`: `{sym}` has no meaning for a quantity or a point",
                name(lt),
                name(rt)
            ))),
            // Comparisons.
            (Eq | NotEq | Lt | Gt | LtEq | GtEq, Some(a), Some(b)) => {
                let same = a.component == b.component
                    && a.kind == b.kind
                    && (a.kind == QKind::Quantity || a.base == b.base);
                if !same {
                    return Some(both(format!(
                        "`{}` {sym} `{}`: {}; a comparison holds within one quantity, or between two points of \
                         one origin",
                        name(lt),
                        name(rt),
                        if a.component != b.component {
                            "different quantities"
                        } else if a.kind != b.kind {
                            "a quantity and a point"
                        } else {
                            "two points of different origins"
                        }
                    )));
                }
                let at = self.meet(&a.denom, &b.denom)?;
                let lto = self.at_denomination(a.base, &at);
                let rto = self.at_denomination(b.base, &at);
                Some(QBinop::Typed {
                    ty: Ty::Prim(hale_syntax::ast::PrimType::Bool),
                    left: self.operand(lt, a, &lto, &at),
                    right: self.operand(rt, b, &rto, &at),
                })
            }
            (Eq | NotEq | Lt | Gt | LtEq | GtEq, _, _) => Some(both(format!(
                "`{}` {sym} `{}`: a quantity or a point compares with its own kind, never an `{}`; compare with a \
                 quantity (`0{}`)",
                name(lt),
                name(rt),
                name(if lq.is_some() { rt } else { lt }),
                self.unit_hint(quantity_side)
            ))),
            (And | Or, _, _) => None,
        }
    }

    /// A unit of `q`'s denomination, for a hint (`cent`).
    pub fn unit_hint(&self, q: &QType) -> String {
        self.rows.units.iter().find(|u| u.site == q.denom.unit).map_or_else(String::new, |u| u.name.clone())
    }

    /// What a point of `q` moves by: its component's quantity at the
    /// point's denomination.
    pub fn point_delta(&self, q: &QType) -> Ty {
        match self.principal(q.component) {
            Some(p) => self.at_denomination(p, &q.denom),
            None => Ty::Unknown,
        }
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
            let Some(n) = self.note(q) else { continue };
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
        notes
    }

    /// A unary operator over a quantity or a point: `None` when `t` is
    /// neither; the result, or why it is refused.
    pub fn quantity_unary(&self, op: UnaryOp, t: &Ty) -> Option<Result<Ty, String>> {
        let q = self.quantity(t)?;
        Some(match (op, q.kind) {
            (UnaryOp::Neg, QKind::Quantity) => Ok(t.clone()),
            (UnaryOp::Neg, QKind::Point) => Err(format!("`-` of the point `{}`: a point has no negation", self.quantity_display(&q))),
            _ => Err(format!("`{}` is a quantity or a point; it has no logical or bitwise operator", self.quantity_display(&q))),
        })
    }

    /// The denomination `.in(…)`/`.split(…)`'s argument names, in `q`'s
    /// component: a unit's name (`s`), a multiple of one (`100ms`).
    pub fn named_denomination(&self, q: &QType, arg: &Expr) -> Result<Denom, String> {
        let (multiple, unit, text) = match arg {
            Expr::Ident(id) => (1, id.name.as_str(), id.name.clone()),
            Expr::Literal(Literal::Quantity { value, unit }, _) if *value > 0 => (*value as u64, unit.as_str(), format!("{value}{unit}")),
            _ => return Err("its argument names a unit (`.in(cent)`) or a multiple of one (`.in(100msec)`)".into()),
        };
        let Some(u) = self.rows.unit_named(unit) else {
            return Err(format!("no `unit` declares `{unit}`"));
        };
        if self.rows.units[u].component != q.component {
            return Err(format!(
                "`{text}` is not a unit of `{}`: `{}` counts in the units of `{}`",
                self.rows.scalars[q.base].display,
                self.quantity_display(q),
                self.rows.scalars[q.base].display
            ));
        }
        Ok(Denom { unit: self.rows.units[u].site, multiple: Ratio::new(BigInt::from(multiple), BigInt::from(1)).expect("positive") })
    }

    /// What `.split(u)` makes of a value of `q`: the type of the rest
    /// (`q` at the meet of its denomination and `u`'s), the conversion
    /// of `q` there, and `u`'s count there (the whole part's divisor).
    pub fn split(&self, t: &Ty, q: &QType, u: &Denom) -> Option<(Ty, Scale, BigInt)> {
        let at = self.meet(&q.denom, u)?;
        let rest = self.at_denomination(q.base, &at);
        let target = QType { denom: at.clone(), ..q.clone() };
        let scale = self.scale(q, &target).ok()?;
        let divisor = self.factor(u, &at)?;
        let _ = t;
        divisor.is_integral().then(|| (rest, scale, divisor.numerator().clone()))
    }

    /// What a printed value of `q` is followed by: a quantity at one of a
    /// unit, the unit (`1500msec`); at another denomination, its type
    /// (`37037 Money in 1/100 cent`, `3 Bucket`); a point, nothing.
    pub fn printed_unit(&self, q: &QType) -> Option<String> {
        if q.kind == QKind::Point {
            return None;
        }
        // A `Duration` prints as it always has (decision 8): its
        // representation class writes the count and `ns` itself.
        if !q.synthesized && self.rows.scalars[q.row].primitive.is_some() {
            return None;
        }
        let spelled = self.spelling(q.base, q.component, &q.denom);
        Some(if !spelled.contains(' ') {
            spelled
        } else {
            format!(" {}", self.quantity_display(q))
        })
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

fn article(q: &QType) -> &'static str {
    match q.kind {
        QKind::Quantity => "a quantity",
        QKind::Point => "a point",
    }
}

/// The operator as the program writes it.
pub fn op_symbol(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Eq => "==",
        NotEq => "!=",
        Lt => "<",
        Gt => ">",
        LtEq => "<=",
        GtEq => ">=",
        And => "&&",
        Or => "||",
        BitAnd => "&",
        BitOr => "|",
        BitXor => "^",
        Shl => "<<",
        Shr => ">>",
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
