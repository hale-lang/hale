//! Values of the unit dialect's identities and ranges (GH #1076, U2).
//!
//! An identity (`type OrderId = distinct Int;`) and a range (`type Byte =
//! Int { range: 0..256; }`) are named types the checker knows by their
//! scope entry ([`crate::symbol::TypeKind::Scalar`]) and their row of the
//! `unit_declarations` family ([`crate::units::ScalarRow`]), by name.
//! Both are represented as an `Int`. This module states the rules the
//! checker applies to their values, once:
//!
//! - an integer literal where one is expected is a value of it, inside
//!   its range ([`ScalarTypes::literal`]);
//! - a range widens to its parent, and an `Int`-rooted range to `Int`,
//!   for free; an identity widens to nothing ([`ScalarTypes::widens`]);
//! - equality and ordering hold within one type and along a widening;
//!   arithmetic on an identity is refused, and arithmetic on a range is
//!   arithmetic on its `Int` ([`ScalarTypes::binop`]);
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
    /// `Int`: a range written over `Int`.
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
            let typed = matches!(
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
                (None, Base::Int) => Up::Int,
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

    /// The identity `i` is, or is a range of; `None` for a range rooted
    /// at `Int`.
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
    /// point is its `Int`, every other type itself.
    pub fn representation(&self, ty: &Ty) -> Ty {
        if self.index(ty).is_some() || self.quantity(ty).is_some() {
            crate::symbol::TypeKind::scalar_representation()
        } else {
            ty.clone()
        }
    }

    /// Whether a value of `from` is a value of `to` for free: `to` is an
    /// ancestor of the scalar `from`, or `Int` above a range rooted there.
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
                Up::Int => return matches!(to, Ty::Prim(PrimType::Int)),
                Up::Nothing => return false,
            }
        }
        false
    }

    /// Whether `t` is an `Int` or a scalar, the two sides the scalar rules
    /// speak about.
    fn int_or_scalar(&self, t: &Ty) -> bool {
        matches!(t, Ty::Prim(PrimType::Int)) || self.index(t).is_some()
    }

    /// The literal `value` where a value of `to` is expected: `None` when
    /// `to` is no identity or range; `Some(Err)` when it is outside `to`'s
    /// range, at `span`.
    pub fn literal(&self, to: &Ty, value: i128, span: Span) -> Option<Result<(), Diag>> {
        let i = self.index(to)?;
        Some(match self.range(i) {
            Some((lo, hi)) if value < lo || value >= hi => {
                Err(Diag::ty(span, format!("`{value}` is outside `{}`'s range `{}`", self.name(i), range_text((lo, hi))))
                    .with_related(self.row(i).name_span, format!("`{}` is declared here", self.name(i))))
            }
            _ => Ok(()),
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
            (None, Ok(_)) => format!(
                "`{}` is an identity and widens to nothing: write `Int(…)` for its `Int`",
                self.display(got)
            ),
            (_, Ok(CastKind::Widening)) | (_, Err(_)) => self.distinct(want, got),
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
        // An `Int`-rooted range stands for its `Int`.
        let as_int = |t: &Ty| match self.index(t) {
            Some(i) if self.identity_root(i).is_none() => Ty::Prim(PrimType::Int),
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
                // A literal facing an identity is a value of it.
                for (lit, other) in [(l, rt), (r, lt)] {
                    if let (Some(v), Some(i)) = (int_literal(lit), self.index(other)) {
                        if self.identity_root(i).is_some() {
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
    /// for arithmetic on an identity; else the result, `Int`.
    pub fn unary(&self, op: UnaryOp, t: &Ty) -> Option<Result<Ty, String>> {
        self.index(t)?;
        Some(match op {
            UnaryOp::Not => Ok(Ty::Prim(PrimType::Bool)),
            UnaryOp::Neg | UnaryOp::BitNot => match self.no_arithmetic(t) {
                Some(why) => Err(why),
                None => Ok(Ty::Prim(PrimType::Int)),
            },
        })
    }

    /// The cast `to(x)` of a value of `from`: what it is, or why it is
    /// refused. `to` is `Int` (the `Int(…)` conversion) or a scalar.
    pub fn cast(&self, to: &Ty, from: &Ty) -> Result<CastKind, String> {
        let f = self.index(from);
        let from_int = matches!(from, Ty::Prim(PrimType::Int) | Ty::Unknown);
        match self.index(to) {
            None => match f {
                Some(i) if self.identity_root(i).is_some() => Ok(CastKind::Total),
                Some(_) => Ok(CastKind::Widening),
                None => Err(format!("`Int(…)` of a `{}` is no unit-dialect conversion", from.display())),
            },
            Some(t) => {
                if from_int {
                    return Ok(if self.range(t).is_some() { CastKind::Narrowing } else { CastKind::Total });
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
                    Err(format!(
                        "{}; a conversion between them goes through `Int`: `{}(Int(…))`",
                        self.distinct_pair(to, from),
                        self.name(t)
                    ))
                }
            }
        }
    }
}
