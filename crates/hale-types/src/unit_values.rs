//! Values of the unit dialect's identities and ranges (GH #1076, U2).
//!
//! An identity (`type OrderId = distinct Int;`) and a range (`type Byte =
//! Int { range: 0..256; }`) are named types the checker knows by their
//! scope entry ([`crate::symbol::TypeKind::Scalar`]) and their row of the
//! `unit_declarations` family ([`crate::units::ScalarRow`]), by name.
//! Both are represented as the integer they count, their base (`Int`, or
//! a `U64`: GH #2131, `ScalarRow::int`). This module states the rules the
//! checker applies to their values, once:
//!
//! - an integer literal where one is expected is a value of it, inside
//!   its range and its integer's ([`ScalarTypes::literal`]);
//! - a range widens to its parent, and a range rooted at its integer to
//!   that integer, for free; an identity widens to nothing
//!   ([`ScalarTypes::widens`]);
//! - equality and ordering hold within one type and along a widening;
//!   arithmetic on an identity is refused, and arithmetic on a range is
//!   arithmetic on its integer ([`ScalarTypes::binop`]);
//! - a cast `T(x)` is total, a widening, or a narrowing a policy has to
//!   discharge ([`ScalarTypes::cast`]).
//!
//! Quantities and points (U3) are typed by the same struct, in
//! [`crate::unit_quantities`]: the algebra, the literals and the
//! conversions between denominations.

use std::collections::BTreeMap;

use hale_syntax::ast::{BinOp, Expr, Literal, PrimType, UnaryOp};
use hale_syntax::{Diag, Span};

use crate::resolve::TopScope;
use crate::symbol::{TopSymbol, TypeKind};
use crate::ty::Ty;
use crate::units::{Base, ScalarKindRow, ScalarRow, UnitRows};

/// The identities and ranges of a program, by name, as the checker asks
/// about them.
pub struct ScalarTypes<'r> {
    pub(crate) rows: &'r UnitRows,
    /// The identities and ranges.
    by_name: BTreeMap<&'r str, usize>,
    /// The quantities and points (U3), by their declared names.
    pub(crate) quantities: BTreeMap<&'r str, usize>,
}

/// One step up a scalar's chain.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Up {
    /// The scalar it refines (a range of a range, of an identity).
    Scalar(usize),
    /// Its integer: a range written over `Int` (or `U64`, GH #2131).
    Int,
    /// Nothing: an identity, which widens to nothing.
    Nothing,
}

/// What a cast `T(x)` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    /// Nothing can be lost: an `Int` into an identity with no range, an
    /// identity's `Int`.
    Total,
    /// Into an ancestor: a range into its parent, an `Int`-rooted range
    /// into `Int`.
    Widening,
    /// Into a range the value may be outside of: fallible, discharged at
    /// the site by an `or`.
    Narrowing,
}

/// What [`ScalarTypes::binop`] decided.
pub enum BinopRule {
    /// The operands' scalar sides stand for their `Int`: type the
    /// operator over these.
    AsInt(Ty, Ty),
    /// A comparison the rules accept: `Bool`.
    Compared,
    /// Refused, with the message.
    Refused(String),
}

/// A range as a message writes it, half-open.
fn range_text((lo, hi): (i128, i128)) -> String {
    format!("{lo}..{hi}")
}

/// An integer literal, negated or not: its value.
pub fn int_literal(e: &Expr) -> Option<i128> {
    match e {
        Expr::Literal(Literal::Int(n), _) => Some(i128::from(*n)),
        // GH #2131: past `Int`'s max, a `U64`'s.
        Expr::Literal(Literal::U64(n), _) => Some(i128::from(*n)),
        Expr::Unary { op: UnaryOp::Neg, operand, .. } => match operand.as_ref() {
            Expr::Literal(Literal::Int(n), _) => Some(-i128::from(*n)),
            _ => None,
        },
        _ => None,
    }
}

impl<'r> ScalarTypes<'r> {
    /// The rows of the scalars `top` registered as scalar types: the
    /// identities and ranges, and the quantities and points.
    pub fn new(rows: &'r UnitRows, top: &TopScope) -> Self {
        let mut by_name = BTreeMap::new();
        let mut quantities = BTreeMap::new();
        for (i, s) in rows.scalars.iter().enumerate() {
            // The stdlib's `Duration` and `Time` (U4) are the primitives
            // a type position reads, never a scope entry.
            let typed = s.primitive.is_some()
                || matches!(
                    top.lookup(&s.name),
                    Some(TopSymbol::Type(info)) if matches!(info.kind, TypeKind::Scalar(_))
                );
            if typed {
                let map = match s.kind {
                    ScalarKindRow::Quantity | ScalarKindRow::Point => &mut quantities,
                    ScalarKindRow::Identity | ScalarKindRow::Range => &mut by_name,
                };
                map.entry(s.name.as_str()).or_insert(i);
            }
        }
        ScalarTypes { rows, by_name, quantities }
    }

    /// The rows of every scalar the rows hold, read without a scope: what
    /// `hale check --units` reads a checked program's conversions by (U5),
    /// where every declaration the laws accept is a scalar type.
    pub fn of_rows(rows: &'r UnitRows) -> Self {
        let mut by_name = BTreeMap::new();
        let mut quantities = BTreeMap::new();
        for (i, s) in rows.scalars.iter().enumerate() {
            let map = match s.kind {
                ScalarKindRow::Quantity | ScalarKindRow::Point => &mut quantities,
                ScalarKindRow::Identity | ScalarKindRow::Range => &mut by_name,
            };
            map.entry(s.name.as_str()).or_insert(i);
        }
        ScalarTypes { rows, by_name, quantities }
    }

    /// Whether the program has no scalar type of any kind.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty() && self.quantities.is_empty()
    }

    /// Whether the program has a quantity or a point (U3).
    pub fn has_quantities(&self) -> bool {
        !self.quantities.is_empty()
    }
    /// The row of `ty` when it is an identity or a range.
    pub fn index(&self, ty: &Ty) -> Option<usize> {
        match ty {
            Ty::Named(n) => self.by_name.get(n.as_str()).copied(),
            _ => None,
        }
    }

    pub fn row(&self, i: usize) -> &'r ScalarRow {
        &self.rows.scalars[i]
    }

    fn name(&self, i: usize) -> &'r str {
        &self.rows.scalars[i].display
    }

    fn up(&self, i: usize) -> Up {
        let row = &self.rows.scalars[i];
        match row.kind {
            ScalarKindRow::Identity => Up::Nothing,
            _ => match (row.parent, &row.written.base) {
                (Some(p), _) if self.by_name.contains_key(self.rows.scalars[p].name.as_str()) => Up::Scalar(p),
                (None, Base::Integer(_)) => Up::Int,
                _ => Up::Nothing,
            },
        }
    }

    /// The range a value of `i` lies in: its own, or the nearest
    /// ancestor's; `None` for an identity with no range (and any range
    /// over one that declares none).
    pub fn range(&self, i: usize) -> Option<(i128, i128)> {
        let mut at = i;
        for _ in 0..=self.rows.scalars.len() {
            if let Some(r) = self.rows.scalars[at].range {
                return Some(r);
            }
            match self.up(at) {
                Up::Scalar(p) => at = p,
                _ => return None,
            }
        }
        None
    }

    /// GH #2131: the values of `i`: its range, or its nearest ancestor's;
    /// with none, every value of the integer it counts. Half-open.
    pub fn bounds(&self, i: usize) -> (i128, i128) {
        self.range(i).unwrap_or_else(|| {
            let (lo, hi) = crate::units::int_bounds(self.rows.scalars[i].int);
            (lo, hi + 1)
        })
    }

    /// GH #2131: the integer a value of `ty` counts, when `ty` is a
    /// scalar of the unit dialect (an identity, a range, a quantity or a
    /// point, declared or synthesized): `Int`, or a `U64`.
    pub fn int_of(&self, ty: &Ty) -> Option<PrimType> {
        if let Some(i) = self.index(ty) {
            return Some(self.rows.scalars[i].int);
        }
        self.quantity(ty).map(|q| self.rows.scalars[q.row].int)
    }

    /// The identity `i` is, or is a range of; `None` for a range rooted
    /// at its integer.
    pub fn identity_root(&self, i: usize) -> Option<usize> {
        let mut at = i;
        for _ in 0..=self.rows.scalars.len() {
            match self.up(at) {
                Up::Scalar(p) => at = p,
                Up::Int => return None,
                Up::Nothing => return Some(at),
            }
        }
        None
    }

    /// `ty` as it is laid out: an identity, a range, a quantity or a
    /// point is the integer it counts (`Int`, a `U64`: GH #2131), every
    /// other type itself.
    pub fn representation(&self, ty: &Ty) -> Ty {
        // The stdlib's `Duration` and `Time` (U4) are their primitives'
        // class: an FFI cell, a payload field, a key reads them as it did.
        if matches!(ty, Ty::Prim(_)) {
            return ty.clone();
        }
        match self.int_of(ty) {
            Some(p) => Ty::Prim(p),
            None => ty.clone(),
        }
    }

    /// Whether a value of `from` is a value of `to` for free: `to` is an
    /// ancestor of the scalar `from`, or its integer above a range rooted
    /// there.
    pub fn widens(&self, from: &Ty, to: &Ty) -> bool {
        let Some(mut at) = self.index(from) else { return false };
        for _ in 0..=self.rows.scalars.len() {
            match self.up(at) {
                Up::Scalar(p) => {
                    if self.index(to) == Some(p) {
                        return true;
                    }
                    at = p;
                }
                Up::Int => return *to == Ty::Prim(self.rows.scalars[at].int),
                Up::Nothing => return false,
            }
        }
        false
    }

    /// Whether `t` is a machine integer (`Int`, `U64`) or a scalar, the
    /// two sides the scalar rules speak about.
    fn int_or_scalar(&self, t: &Ty) -> bool {
        matches!(t, Ty::Prim(p) if p.machine_int().is_some()) || self.index(t).is_some()
    }

    /// The literal `value` where a value of `to` is expected: `None` when
    /// `to` is no identity or range; `Some(Err)` when it is outside `to`'s
    /// range, or (GH #2131) is no value of the integer `to` counts, at
    /// `span`.
    pub fn literal(&self, to: &Ty, value: i128, span: Span) -> Option<Result<(), Diag>> {
        let i = self.index(to)?;
        let note = |d: Diag| d.with_related(self.row(i).name_span, format!("`{}` is declared here", self.name(i)));
        Some(match self.range(i) {
            Some((lo, hi)) if value < lo || value >= hi => {
                Err(note(Diag::ty(span, format!("`{value}` is outside `{}`'s range `{}`", self.name(i), range_text((lo, hi))))))
            }
            Some(_) => Ok(()),
            None => {
                let int = self.rows.scalars[i].int;
                let (lo, hi) = crate::units::int_bounds(int);
                if value < lo || value > hi {
                    let base = crate::ty::prim_name(int);
                    Err(note(Diag::ty(
                        span,
                        format!("`{value}` is no `{}`: it counts a `{base}`, which is {lo} through {hi}", self.name(i)),
                    )))
                } else {
                    Ok(())
                }
            }
        })
    }

    /// Why a value of `got` does not flow into `want` where the scalar
    /// rules speak about the pair (an `Int` or a scalar on both sides, one
    /// of them a scalar), at `span`; `None` when they do not.
    pub fn flow_refusal(&self, want: &Ty, got: &Ty, span: Span) -> Option<Diag> {
        if !(self.int_or_scalar(want) && self.int_or_scalar(got)) {
            return None;
        }
        let (w, g) = (self.index(want), self.index(got));
        if w.is_none() && g.is_none() {
            return None;
        }
        let message = match (w, self.cast(want, got)) {
            (Some(w), Ok(CastKind::Total)) => format!(
                "`{}` is not `{}`: an identity is reached only through the explicit conversion `{}(…)`",
                got.display(),
                self.name(w),
                self.name(w)
            ),
            (Some(w), Ok(CastKind::Narrowing)) => format!(
                "`{}` does not narrow to `{}` implicitly: write `{}(…) or …`, which says what becomes of a \
                 value outside `{}`",
                self.display(got),
                self.name(w),
                self.name(w),
                self.range(w).map(range_text).unwrap_or_default()
            ),
            (None, Ok(_)) => {
                let base = g.map_or("Int", |g| crate::ty::prim_name(self.rows.scalars[g].int));
                format!("`{}` is an identity and widens to nothing: write `{base}(…)` for its `{base}`", self.display(got))
            }
            (_, Ok(CastKind::Widening)) => self.distinct(want, got),
            // GH #2131: a scalar and an integer it does not count.
            (_, Err(why)) if self.int_mismatch(want, got).is_some() => why,
            (_, Err(_)) => self.distinct(want, got),
        };
        let mut d = Diag::ty(span, message);
        for i in [w, g].into_iter().flatten() {
            d = d.with_related(self.row(i).name_span, format!("`{}` is declared here", self.name(i)));
        }
        Some(d)
    }

    fn display(&self, t: &Ty) -> String {
        match self.index(t) {
            Some(i) => self.name(i).to_string(),
            None => t.display(),
        }
    }

    /// Two types the scalar rules keep apart.
    fn distinct(&self, a: &Ty, b: &Ty) -> String {
        format!("{}; convert one explicitly", self.distinct_pair(a, b))
    }

    fn distinct_pair(&self, a: &Ty, b: &Ty) -> String {
        let both_identities = [a, b].iter().all(|t| self.index(t).is_some_and(|i| self.identity_root(i) == Some(i)));
        format!(
            "`{}` and `{}` are distinct {}",
            self.display(a),
            self.display(b),
            if both_identities { "identities" } else { "types" }
        )
    }

    /// Why arithmetic on `t` is refused: it is an identity, or a range of
    /// one.
    fn no_arithmetic(&self, t: &Ty) -> Option<String> {
        let i = self.index(t)?;
        let root = self.identity_root(i)?;
        Some(if root == i {
            format!("`{}` is an identity; it has no arithmetic", self.name(i))
        } else {
            format!("`{}` is a range of the identity `{}`; it has no arithmetic", self.name(i), self.name(root))
        })
    }

    /// A binary operator over `lt` and `rt` (`l` and `r` the operands, for
    /// a literal's value); `None` when neither is an identity or a range.
    pub fn binop(&self, op: BinOp, lt: &Ty, rt: &Ty, l: &Expr, r: &Expr) -> Option<BinopRule> {
        use BinOp::*;
        let (li, ri) = (self.index(lt), self.index(rt));
        if li.is_none() && ri.is_none() {
            return None;
        }
        // A range rooted at its integer stands for that integer.
        let as_int = |t: &Ty| match self.index(t) {
            Some(i) if self.identity_root(i).is_none() => Ty::Prim(self.rows.scalars[i].int),
            _ => t.clone(),
        };
        Some(match op {
            Add | Sub | Mul | Div | Mod | BitAnd | BitOr | BitXor | Shl | Shr => {
                match self.no_arithmetic(lt).or_else(|| self.no_arithmetic(rt)) {
                    Some(why) => BinopRule::Refused(why),
                    None => BinopRule::AsInt(as_int(lt), as_int(rt)),
                }
            }
            Eq | NotEq | Lt | Gt | LtEq | GtEq => {
                // A literal facing an identity is a value of it, and so is
                // one facing a range of an integer other than `Int` (GH
                // #2131: a `U64`'s, where no `Int` widens).
                for (lit, other) in [(l, rt), (r, lt)] {
                    if let (Some(v), Some(i)) = (int_literal(lit), self.index(other)) {
                        if self.identity_root(i).is_some() || self.rows.scalars[i].int != PrimType::Int {
                            return Some(match self.literal(other, v, lit.span()) {
                                Some(Err(d)) => BinopRule::Refused(d.message),
                                _ => BinopRule::Compared,
                            });
                        }
                    }
                }
                if lt == rt || self.widens(lt, rt) || self.widens(rt, lt) {
                    BinopRule::Compared
                } else if matches!(lt, Ty::Unknown) || matches!(rt, Ty::Unknown) {
                    BinopRule::Compared
                } else if self.int_or_scalar(lt) && self.int_or_scalar(rt) {
                    BinopRule::Refused(self.distinct(lt, rt))
                } else {
                    // A scalar against another kind of value: the
                    // comparison's own rule, over the `Int` a range is.
                    let (l, r) = (as_int(lt), as_int(rt));
                    if self.index(&l).is_some() || self.index(&r).is_some() {
                        return None;
                    }
                    BinopRule::AsInt(l, r)
                }
            }
            And | Or => return None,
        })
    }

    /// A unary operator over `t`: `None` when `t` is no scalar; the refusal
    /// for arithmetic on an identity, and for `-` of a range of an unsigned
    /// integer (GH #2131); else the result, its integer.
    pub fn unary(&self, op: UnaryOp, t: &Ty) -> Option<Result<Ty, String>> {
        let i = self.index(t)?;
        let int = self.rows.scalars[i].int;
        Some(match op {
            UnaryOp::Not => Ok(Ty::Prim(PrimType::Bool)),
            UnaryOp::Neg | UnaryOp::BitNot => match self.no_arithmetic(t) {
                Some(why) => Err(why),
                None if op == UnaryOp::Neg && int.machine_int().is_some_and(|(_, signed)| !signed) => Err(format!(
                    "`-` of `{}`, which counts a `{}`: a `{}` is never negative and has no unary `-`",
                    self.name(i),
                    crate::ty::prim_name(int),
                    crate::ty::prim_name(int)
                )),
                None => Ok(Ty::Prim(int)),
            },
        })
    }

    /// GH #2131: why a value of `got` is no value of `want` when one is an
    /// identity or a range and the other a machine integer it does not
    /// count: the integers never convert implicitly, and the message names
    /// the conversion. `None` for every other pair.
    pub fn int_mismatch(&self, want: &Ty, got: &Ty) -> Option<String> {
        let int = |t: &Ty| match t {
            Ty::Prim(p) if p.machine_int().is_some() => Some(*p),
            _ => None,
        };
        let (scalar, other, into_scalar) = match (self.index(want), self.index(got), int(want), int(got)) {
            (Some(w), None, _, Some(g)) => (w, g, true),
            (None, Some(g), Some(w), _) => (g, w, false),
            _ => return None,
        };
        let base = self.rows.scalars[scalar].int;
        if base == other {
            return None;
        }
        let (b, o, name) = (crate::ty::prim_name(base), crate::ty::prim_name(other), self.name(scalar));
        let a = |n: &str| crate::units::article_int(n);
        Some(if into_scalar {
            format!(
                "`{o}` is not `{name}`: `{name}` counts {}, and {} never converts implicitly to one: write \
                 `{name}({})`",
                a(b),
                a(o),
                crate::units::int_conversion(other, base)
            )
        } else {
            format!(
                "`{name}` is not {}: it counts {}, and the two never convert implicitly: write `{b}(…)` for its `{b}`, \
                 then `{}`",
                a(o),
                a(b),
                crate::units::int_conversion(base, other)
            )
        })
    }

    /// The cast `to(x)` of a value of `from`: what it is, or why it is
    /// refused. `to` is an integer (the `Int(…)` and `U64(…)` conversions,
    /// GH #2131) or a scalar.
    pub fn cast(&self, to: &Ty, from: &Ty) -> Result<CastKind, String> {
        let f = self.index(from);
        match self.index(to) {
            None => match f {
                Some(i) if *to != Ty::Prim(self.rows.scalars[i].int) => Err(self
                    .int_mismatch(to, from)
                    .unwrap_or_else(|| format!("`{}(…)` of `{}` is no unit-dialect conversion", to.display(), self.name(i)))),
                Some(i) if self.identity_root(i).is_some() => Ok(CastKind::Total),
                Some(_) => Ok(CastKind::Widening),
                None => Err(format!("`{}(…)` of a `{}` is no unit-dialect conversion", to.display(), from.display())),
            },
            Some(t) => {
                let from_int = *from == Ty::Prim(self.rows.scalars[t].int) || matches!(from, Ty::Unknown);
                if from_int {
                    return Ok(if self.range(t).is_some() { CastKind::Narrowing } else { CastKind::Total });
                }
                if let Some(why) = self.int_mismatch(to, from) {
                    return Err(why);
                }
                let Some(i) = f else {
                    return Err(format!(
                        "`{}(…)` converts an `Int` or a scalar of its family; got `{}`",
                        self.name(t),
                        from.display()
                    ));
                };
                if i == t || self.widens(from, to) {
                    Ok(CastKind::Widening)
                } else if self.widens(to, from) {
                    Ok(if self.range(t) == self.range(i) { CastKind::Widening } else { CastKind::Narrowing })
                } else {
                    let base = crate::ty::prim_name(self.rows.scalars[t].int);
                    Err(format!(
                        "{}; a conversion between them goes through `{base}`: `{}({base}(…))`",
                        self.distinct_pair(to, from),
                        self.name(t)
                    ))
                }
            }
        }
    }
}
