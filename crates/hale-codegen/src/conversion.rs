//! Conversions of the unit dialect's values (GH #1076), lowered from
//! their rows.
//!
//! The checker classifies every conversion and records it in the typed
//! bodies' `conversions` column (`hale_types::typed_bodies::ConversionRow`):
//! its kind, the range a narrowing checks against, the exact factor and
//! shift of a change of denomination, and what discharges it.
//! [`Cx::lower_conversion`] reads the row and decides nothing. An
//! identity, a range, a quantity and a point are each the integer it
//! counts here (`user_type_aliases`): an `Int`, or a `U64` (GH #2131),
//! whose arithmetic the conversion's is: a `U64`'s multiplication is
//! checked, its divisions and comparisons unsigned ([`Cx::unit_repr`]).
//!
//! - Between identities and ranges (U2), a total conversion and a
//!   widening emit nothing; a narrowing emits the two comparisons and,
//!   from the row's policy, the clamp (two selects), the wrap (an
//!   unsigned remainder of the distance from the low bound, on whichever
//!   side of it the value is), or the checked value the `or`'s join
//!   takes (the substitute, the handler, the raise).
//! - Between denominations (U3), one multiplication by the factor's
//!   numerator (and a point's shift), then, when the denominator is above
//!   one, one division by it: rounded by the row's policy (`trunc`,
//!   `floor`, `ceil`, `half_up`, `half_even`, each one shape), or checked
//!   exact for the `or`'s join, an `InexactError` on its error path.
//!   `.split(u)` divides by `u`'s count, flooring, and is the pair of the
//!   quotient and the remainder. A factor no `Int` holds is a located
//!   error, never a wrap.
//! - A quantity literal is the constant its row's count is; an
//!   expression the checker converted where it stands (an argument, a
//!   binding, an operand) has its row at its span, applied as it is
//!   lowered ([`Cx::convert_value`]); a printed quantity's row is the
//!   unit written after its count.
//!
//! Every row is read from the body being emitted (`Cx::current_body`),
//! the declaration the checker recorded it under, and from no other: a
//! span is a key within one body, and the stdlib's bodies, whose spans
//! start at 0 like the first user file's, find none of a user body's.

use hale_syntax::ast::{BinOp, Expr};
use hale_syntax::Span;
use hale_types::typed_bodies::{ConversionKind, ConversionRow, ConversionSite, Discharge, Scale, SiteKind};
use hale_types::units::RoundPolicy;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

use crate::codegen::{CodegenError, CodegenTy, Cx, FallibleCallResult, Scope};

/// What a conversion lowered to.
pub(crate) enum Converted<'ctx> {
    /// The converted `Int`.
    Value(BasicValueEnum<'ctx>, CodegenTy),
    /// A narrowing its `or` discharges: the checked value, or the
    /// `RangeError` (`InexactError`), for the `or`'s join
    /// (`lower_or_expr`).
    Checked(FallibleCallResult<'ctx>),
}

fn emit<T>(r: Result<T, inkwell::builder::BuilderError>) -> Result<T, CodegenError> {
    r.map_err(|e| CodegenError::LlvmEmit(e.to_string()))
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The conversion row of `e` and the value it converts, when the
    /// checker classified `e` as one: a cast `T(x)` and its argument (none
    /// when it is given another count of them), a quantity's `x.in(u)` and
    /// `x.split(u)` and the receiver, by the call; a quantity divided by a
    /// literal and the dividend, by the division (U3). No row is the
    /// total answer: the call is no conversion, whatever its callee is
    /// named (a local, a parameter or a fn the checker resolved the name
    /// to, or `Int(x)`, the numeric builtin), and is lowered as the call
    /// it is; a division is the `Int`'s. An imported seed's `lib::T(x)`
    /// is a cast by the same row. Inside a default every row is
    /// the evaluation path's being lowered (`default_evaluation`): the
    /// default's name means what that scope says, a cast in one and a
    /// local's call in another, and its literals and values convert into
    /// what that scope flows them into.
    pub(crate) fn conversion_operand<'e>(&self, e: &'e Expr) -> Option<(ConversionRow, Option<&'e Expr>)> {
        match e {
            Expr::Call { callee, id, args, .. } => {
                let operand = match callee.as_ref() {
                    Expr::Ident(_) | Expr::Path(_) => match args.as_slice() {
                        [arg] => Some(arg),
                        _ => None,
                    },
                    // `Decimal<S>`'s `d.rescale(T)` is a conversion between
                    // scales, by the same rows.
                    Expr::Field { receiver, name, .. } if matches!(name.name.as_str(), "in" | "split" | "rescale") => {
                        Some(receiver.as_ref())
                    }
                    _ => return None,
                };
                let row = self.row(SiteKind::Cast(id.0))?;
                // A `Decimal<S>` row keyed at a path call is the scale its
                // result takes (`std::str::parse_decimal`, a generated
                // codec's), read by `decimal_scale_at_call`, not a cast:
                // the call is lowered as the call it is.
                if matches!(callee.as_ref(), Expr::Path(_))
                    && matches!(row.to, hale_types::ty::Ty::Prim(hale_syntax::ast::PrimType::Decimal(_)))
                {
                    return None;
                }
                Some((row.clone(), operand))
            }
            Expr::Binary { op: BinOp::Div, left, span, .. } if self.typed.has_quantity_rows() => {
                let row = self.row(SiteKind::divide(*span))?;
                Some((row.clone(), Some(left.as_ref())))
            }
            _ => None,
        }
    }

    /// GH #1076 (U3): `e`'s value `v`, converted by the row the checker
    /// recorded at `e` where it stands (an argument, a binding, a return,
    /// an operand): a widening, or a narrowing its target type's
    /// `round:` discharges. A quantity literal's row is its count, which
    /// [`Self::lower_quantity_literal`] emitted already. Every other
    /// expression is its value.
    pub(crate) fn convert_value(
        &mut self,
        e: &Expr,
        lowered: (BasicValueEnum<'ctx>, CodegenTy),
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !self.typed.has_quantity_rows() {
            return Ok(lowered);
        }
        let Some(row) = self.row(SiteKind::value(e.span())) else { return Ok(lowered) };
        if row.count.is_some() || row.scale.is_none() || matches!(e, hale_syntax::ast::Expr::Literal(..)) {
            return Ok(lowered);
        }
        let row = row.clone();
        let (v, ty) = lowered;
        if !is_quantity_repr(&ty) {
            return Err(CodegenError::UnsupportedAt(
                format!("the conversion into `{}` converts an `Int`; its value lowered as {ty}", row.target),
                e.span(),
            ));
        }
        match self.lower_scale(&row, v.into_int_value())? {
            Converted::Value(v, t) => Ok((v, t)),
            Converted::Checked(_) => Err(CodegenError::UnsupportedAt(
                format!("the conversion into `{}` is discharged by nothing where it stands", row.target),
                e.span(),
            )),
        }
    }

    /// GH #1076 (U3): a quantity literal, the constant its row's count
    /// is: the checker converted it into the denomination it flows into.
    /// With no row, a required row is missing, and the literal is refused
    /// where it is written.
    pub(crate) fn lower_quantity_literal(
        &mut self,
        value: u64,
        unit: &str,
        span: Span,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let missing = || {
            CodegenError::UnsupportedAt(
                format!(
                    "quantity literal `{value}{unit}` has no required `expression_typing` row: the checker converts a \
                     quantity literal into the denomination it flows into, and lowering emits that count"
                ),
                span,
            )
        };
        let (count, ty) = match self.row(SiteKind::value(span)) {
            Some(row) => (row.count.ok_or_else(missing)?, self.unit_repr(&row.to)),
            // U4: a body the checker types no row in (the stdlib's own,
            // a `bindings { }` initializer) has the literal's own row of
            // the stdlib's catalogue: a time literal is that count of
            // `Duration`, in every program.
            None if !self.current_body.is_some_and(|b| self.typed.has_conversions_in(b)) => {
                let (count, ty) = hale_types::units::stdlib_literal(value, unit).ok_or_else(missing)?;
                (i128::from(count), self.unit_repr(&ty))
            }
            None => return Err(missing()),
        };
        Ok((self.count_const(count, &ty).into(), ty))
    }

    /// A count the checker converted, as a constant of its integer's
    /// width: its bits (a `U64`'s past `Int`'s max included, GH #2131).
    fn count_const(&self, count: i128, ty: &CodegenTy) -> IntValue<'ctx> {
        let bits = ty.machine_int().map_or(64, |(b, _)| b);
        self.context.custom_width_int_type(bits).const_int(count as u64, false)
    }

    /// GH #1076: the constant a negated quantity literal (`-3b`) is where
    /// it flows: the checker converted the negated count at compile time
    /// and recorded it on the negation's row, so the negation emits that
    /// count and no arithmetic (downstream handoff: lowered as the
    /// literal's own count negated, the exact conversion the check
    /// proved, `-3b` into a type counted in thirds of `b`, was lost).
    /// `None` where the negation has no such row: it is the negation of
    /// the literal's value then.
    pub(crate) fn negated_literal_count(&self, span: Span) -> Option<(IntValue<'ctx>, CodegenTy)> {
        if !self.typed.has_quantity_rows() {
            return None;
        }
        let row = self.row(SiteKind::value(span))?;
        let ty = self.unit_repr(&row.to);
        Some((self.count_const(row.count?, &ty), ty))
    }

    /// GH #1076 (U4): what an arithmetic operator over a quantity or a
    /// point at `span` results in, from its row: the representation the
    /// operation is emitted in (`Int * Duration` a `Duration`, `Time -
    /// Time` a `Duration`, a quantity counted in a denomination no
    /// declaration names an `Int`). `None` for every other operator.
    pub(crate) fn operator_result(&self, span: Span) -> Option<CodegenTy> {
        if !self.typed.has_quantity_rows() {
            return None;
        }
        Some(self.unit_repr(&self.row(SiteKind::operator(span))?.to))
    }

    /// The representation a value of a unit-dialect type `t` lowers as:
    /// the stdlib's `Duration` and `Time` keep their own class (U4); every
    /// other quantity, point, identity or range, declared or synthesized
    /// (`Wei in gwei`), is the integer it counts (GH #2131: an `Int` or a
    /// `U64`, `scalar_ints`), as is that integer itself.
    pub(crate) fn unit_repr(&self, t: &hale_types::ty::Ty) -> CodegenTy {
        use hale_syntax::ast::{PrimType, TypeExpr};
        use hale_types::ty::Ty;
        match t {
            Ty::Prim(PrimType::Duration) => return CodegenTy::Duration,
            Ty::Prim(PrimType::Time) => return CodegenTy::Time,
            Ty::Prim(PrimType::Bool) => return CodegenTy::Bool,
            Ty::Prim(PrimType::Decimal(s)) => return CodegenTy::Decimal(*s),
            _ => {}
        }
        self.unit_int(t)
            .and_then(|p| self.type_expr_to_codegen_ty(&TypeExpr::Primitive(p, Span::new(0, 0))).ok())
            .unwrap_or(CodegenTy::Int)
    }

    /// GH #2131: the integer a value of the unit-dialect type `t` counts
    /// (`Int`, `U64`), or that integer itself; `Int` for the stdlib's
    /// `Duration` and `Time`, and for either at another denomination
    /// (`Duration in s`). `None` for every other type.
    pub(crate) fn unit_int(&self, t: &hale_types::ty::Ty) -> Option<hale_syntax::ast::PrimType> {
        use hale_syntax::ast::PrimType;
        use hale_types::ty::Ty;
        match t {
            Ty::Prim(p) if p.machine_int().is_some() => Some(*p),
            Ty::Prim(PrimType::Duration | PrimType::Time) => Some(PrimType::Int),
            Ty::Named(n) => match n.split_once(" in ") {
                Some(("Duration" | "Time", _)) => Some(PrimType::Int),
                Some((base, _)) => self.scalar_ints.get(base).copied(),
                None => self.scalar_ints.get(n.as_str()).copied(),
            },
            _ => None,
        }
    }

    /// GH #1076 (U3): the unit a printed value's row writes after its
    /// count (`msec`, ` Money in 1/100 cent`).
    pub(crate) fn printed_unit(&self, e: &Expr) -> Option<String> {
        if !self.typed.has_quantity_rows() {
            return None;
        }
        self.row(SiteKind::printed(e.span()))?.printed.clone()
    }

    /// `Decimal<S>`: the scale of the Decimal literal at `span`, from the
    /// row the checker recorded where it flows (`Decimal`'s 9 with none).
    pub(crate) fn decimal_scale_at_value(&self, span: Span) -> u8 {
        self.decimal_scale_of(self.row(SiteKind::value(span)))
    }

    /// `Decimal<S>`: the scale of the `std::str::parse_decimal` call being
    /// lowered (`current_call`), from the row the checker recorded at the
    /// call where its result flows (`Decimal`'s 9 with none).
    pub(crate) fn decimal_scale_at_call(&self) -> u8 {
        let row = self.current_call.and_then(|call| self.row(SiteKind::Cast(call.0)));
        self.decimal_scale_of(row)
    }

    fn decimal_scale_of(&self, row: Option<&ConversionRow>) -> u8 {
        match row.map(|r| &r.to) {
            Some(hale_types::ty::Ty::Prim(hale_syntax::ast::PrimType::Decimal(s))) => *s,
            _ => hale_syntax::decimal_literal::DEFAULT_SCALE,
        }
    }

    /// The key of the conversion of `kind` here: inside a default, on the
    /// evaluation path being lowered (`default_evaluation`), the
    /// checker's key for the same evaluation (`site` in `hale-types`).
    pub(crate) fn site(&self, kind: SiteKind) -> ConversionSite {
        ConversionSite::new(kind, self.default_evaluation.clone())
    }

    /// The row of the conversion of `kind` here, read from the body being
    /// emitted (`current_body`) and from no other: a span is a key within
    /// one body only, and a stdlib body's spans start at 0 like the first
    /// user file's, so the stdlib's bodies, which the checker records no
    /// row for, find none. Outside every declaration's body, no row.
    fn row(&self, kind: SiteKind) -> Option<&'p ConversionRow> {
        self.typed.conversion_in(self.current_body?, &self.site(kind))
    }

    /// Lower `f` as the body of the declaration `decl`: conversion rows
    /// are read from it until `f` returns, then from the body before.
    pub(crate) fn in_body<T>(
        &mut self,
        decl: hale_syntax::ast::NodeId,
        f: impl FnOnce(&mut Self) -> Result<T, CodegenError>,
    ) -> Result<T, CodegenError> {
        let prev = self.current_body.replace(decl);
        let out = f(self);
        self.current_body = prev;
        out
    }

    /// Lower `f` as the initializer of the constant `decl`, re-lowered at
    /// a use: its own evaluation, as the checker types it once in its own
    /// body outside every default, so its rows are read from `decl` on no
    /// evaluation path, whatever default the use stands in, and the
    /// caller's path is back when `f` returns.
    pub(crate) fn in_const_body<T>(
        &mut self,
        decl: hale_syntax::ast::NodeId,
        f: impl FnOnce(&mut Self) -> Result<T, CodegenError>,
    ) -> Result<T, CodegenError> {
        let evaluation = std::mem::take(&mut self.default_evaluation);
        let out = self.in_body(decl, f);
        self.default_evaluation = evaluation;
        out
    }

    /// Lower the conversion `row` of the value `arg`.
    pub(crate) fn lower_conversion(
        &mut self,
        row: &ConversionRow,
        arg: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<Converted<'ctx>, CodegenError> {
        let (v, ty) = self.lower_expr(arg, scope)?;
        if !is_quantity_repr(&ty) && !matches!(ty, CodegenTy::Decimal(_)) {
            return Err(CodegenError::UnsupportedAt(
                format!("`{}(…)` converts an `Int`; its value lowered as {ty}", row.target),
                arg.span(),
            ));
        }
        let v = v.into_int_value();
        if row.scale.is_some() {
            return self.lower_scale(row, v);
        }
        // GH #2132: a cast between two machine integers, one narrower than
        // 64 bits (`U8(n)`, `Int(b)`, `U64(i)`).
        if let (Some(from), hale_types::ty::Ty::Prim(to)) = (crate::stdlib::fixed::int_prim(&ty), &row.to) {
            if to.machine_int().is_some() && (from.is_narrow_int() || to.is_narrow_int()) {
                return self.lower_fixed_cast(row, v, from, *to);
            }
        }
        // GH #2131: an identity or a range is the integer it counts, an
        // `Int` or a `U64`, and is compared as that integer is.
        let repr = self.unit_repr(&row.to);
        if row.kind != ConversionKind::Narrowing {
            // An identity or a range is its integer: nothing to emit.
            return Ok(Converted::Value(v.into(), repr));
        }
        let Some((lo, hi)) = row.range else {
            return Err(CodegenError::UnsupportedAt(
                format!("the narrowing into `{}` has no range in its row", row.target),
                row.span,
            ));
        };
        let signed = repr.machine_int().map_or(true, |(_, s)| s);
        let (lo, last) = if signed { (i128::from(saturate(lo)), i128::from(saturate(hi - 1))) } else { (lo, hi - 1) };
        let (lo_c, last_c) = (self.count_const(lo, &repr), self.count_const(last, &repr));
        let (lt, gt) = if signed { (IntPredicate::SLT, IntPredicate::SGT) } else { (IntPredicate::ULT, IntPredicate::UGT) };
        let below = emit(self.builder.build_int_compare(lt, v, lo_c, "narrow.below"))?;
        let above = emit(self.builder.build_int_compare(gt, v, last_c, "narrow.above"))?;
        match row.policy {
            Some(Discharge::Clamp) => {
                let up = emit(self.builder.build_select(below, lo_c, v, "narrow.clamp.lo"))?.into_int_value();
                let c = emit(self.builder.build_select(above, last_c, up, "narrow.clamp"))?;
                Ok(Converted::Value(c, repr))
            }
            Some(Discharge::Wrap) => {
                let w = self.lower_wrap(row, v, lo, hi, signed)?;
                Ok(Converted::Value(w.into(), repr))
            }
            Some(_) => self.lower_checked_narrowing(row, (v, v), below, above, (lo, hi), &repr).map(Converted::Checked),
            None => Err(CodegenError::UnsupportedAt(
                format!(
                    "the narrowing into `{}` is discharged by nothing in its row; the check refuses a bare one",
                    row.target
                ),
                row.span,
            )),
        }
    }

    /// `or wrap`: `lo + ((v - lo) mod w)`, `w = hi - lo`, the remainder
    /// taken non-negative, for every value `v` of the integer and without
    /// overflow. The distance between `v` and `lo` is exact as an unsigned
    /// integer (the wrapping subtraction of the smaller from the larger),
    /// so each side takes an unsigned remainder: at or above `lo`, `t = (v
    /// - lo) urem w`; below it, `m = (lo - v) urem w` and `t = w - m`, or
    /// `0` when `m` is. `t < w`, so `lo + t` is at most `hi - 1`. Both
    /// sides are computed and one selected: neither can trap. A `U64`'s
    /// (GH #2131) compares unsigned, and a range holding every `U64` is
    /// every value already.
    fn lower_wrap(
        &mut self,
        row: &ConversionRow,
        v: IntValue<'ctx>,
        lo: i128,
        hi: i128,
        signed: bool,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let width = hi - lo;
        let fits = if signed { i64::try_from(width).is_ok() } else { u64::try_from(width).is_ok() };
        if !fits && !signed {
            return Ok(v);
        }
        if !fits {
            return Err(CodegenError::UnsupportedAt(
                format!("`{}`'s range is wider than an `Int`: `or wrap` has no width to wrap by", row.target),
                row.span,
            ));
        }
        let int_t = v.get_type();
        let (lo_c, width_c) = (int_t.const_int(lo as u64, false), int_t.const_int(width as u64, false));
        let zero = int_t.const_zero();
        let ge = if signed { IntPredicate::SGE } else { IntPredicate::UGE };
        let at_or_above = emit(self.builder.build_int_compare(ge, v, lo_c, "narrow.wrap.up"))?;
        let above = emit(self.builder.build_int_sub(v, lo_c, "narrow.wrap.above"))?;
        let above = emit(self.builder.build_int_unsigned_rem(above, width_c, "narrow.wrap.above.rem"))?;
        let below = emit(self.builder.build_int_sub(lo_c, v, "narrow.wrap.below"))?;
        let below = emit(self.builder.build_int_unsigned_rem(below, width_c, "narrow.wrap.below.rem"))?;
        let on_lo = emit(self.builder.build_int_compare(IntPredicate::EQ, below, zero, "narrow.wrap.below.zero"))?;
        let back = emit(self.builder.build_int_sub(width_c, below, "narrow.wrap.below.back"))?;
        let below = emit(self.builder.build_select(on_lo, zero, back, "narrow.wrap.below.off"))?.into_int_value();
        let off = emit(self.builder.build_select(at_or_above, above, below, "narrow.wrap.off"))?.into_int_value();
        emit(self.builder.build_int_add(lo_c, off, "narrow.wrap"))
    }

    /// A narrowing its `or` discharges: the value in the success slot,
    /// or a `RangeError` in the error slot, and the path bit the `or`
    /// branches on (1 = outside the range). The error's fields are
    /// `Int`s: a `U64`'s value or bound past `Int`'s max is `Int`'s max
    /// there (GH #2131).
    fn lower_checked_narrowing(
        &mut self,
        row: &ConversionRow,
        (v, stored): (IntValue<'ctx>, IntValue<'ctx>),
        below: IntValue<'ctx>,
        above: IntValue<'ctx>,
        (lo, hi): (i128, i128),
        repr: &CodegenTy,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_checked_narrowing_from(row, (v, stored), below, above, (lo, hi), repr, repr)
    }

    /// [`Self::lower_checked_narrowing`] of a value `v` of `source`, which
    /// is `stored` at the target `repr` when it is inside the range.
    #[allow(clippy::too_many_arguments)]
    fn lower_checked_narrowing_from(
        &mut self,
        row: &ConversionRow,
        (v, stored): (IntValue<'ctx>, IntValue<'ctx>),
        below: IntValue<'ctx>,
        above: IntValue<'ctx>,
        (lo, hi): (i128, i128),
        repr: &CodegenTy,
        source: &CodegenTy,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .ok_or_else(|| CodegenError::UnsupportedAt("a narrowing outside a fn body".into(), row.span))?;
        let outside = emit(self.builder.build_or(below, above, "narrow.outside"))?;
        let payload_ty = CodegenTy::TypeRef("RangeError".into());
        let out_val_slot = self.alloca_for(repr, "narrow.val.slot")?;
        let out_err_slot = self.alloca_for(&payload_ty, "narrow.err.slot")?;
        let ok_bb = self.context.append_basic_block(func, "narrow.ok");
        let err_bb = self.context.append_basic_block(func, "narrow.err");
        let join_bb = self.context.append_basic_block(func, "narrow.join");
        emit(self.builder.build_conditional_branch(outside, err_bb, ok_bb))?;

        self.builder.position_at_end(ok_bb);
        emit(self.builder.build_store(out_val_slot, stored))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(err_bb);
        let i64_t = self.context.i64_type();
        let (low, high) = (i64_t.const_int(saturate(lo) as u64, true), i64_t.const_int(saturate(hi) as u64, true));
        let value = self.saturate_to_int(v, source)?;
        let err = self.emit_range_error_alloc(&row.target, value, low, high, row.span)?;
        emit(self.builder.build_store(out_err_slot, err))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: outside,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(repr.clone()),
            payload_ty,
        })
    }

    /// GH #2132 (F.64): the cast `to(x)` of `v`, a value of the machine
    /// integer `from`, one of the two narrower than 64 bits. A total
    /// conversion and a widening are `v` at `to`'s width (sign-extended
    /// from a signed integer). A narrowing compares `v` against `to`'s
    /// range in `from`'s own width and signedness (a bound `from` cannot
    /// reach compares nothing), then: `or clamp` the nearest bound, `or
    /// wrap` the value modulo `to`'s width, which is `to`'s bits of it; any
    /// other policy the checked value, or the `RangeError` (its `value`
    /// the source's) for the `or`'s join.
    fn lower_fixed_cast(
        &mut self,
        row: &ConversionRow,
        v: IntValue<'ctx>,
        from: hale_syntax::ast::PrimType,
        to: hale_syntax::ast::PrimType,
    ) -> Result<Converted<'ctx>, CodegenError> {
        let repr = self.type_expr_to_codegen_ty(&hale_syntax::ast::TypeExpr::Primitive(to, Span::new(0, 0)))?;
        if row.kind != ConversionKind::Narrowing {
            return Ok(Converted::Value(self.resize_int(v, from, to)?.into(), repr));
        }
        let Some((lo, hi)) = row.range else {
            return Err(CodegenError::UnsupportedAt(format!("the narrowing into `{}` has no range in its row", row.target), row.span));
        };
        let source = self.type_expr_to_codegen_ty(&hale_syntax::ast::TypeExpr::Primitive(from, Span::new(0, 0)))?;
        let (from_lo, from_hi) = from.int_bounds().expect("a machine integer");
        let (_, signed) = from.machine_int().expect("a machine integer");
        let src_t = v.get_type();
        let last = hi - 1;
        let never = self.context.bool_type().const_zero();
        let at = |b: i128| src_t.const_int(b as u64, signed && b < 0);
        let (lt, gt) = if signed { (IntPredicate::SLT, IntPredicate::SGT) } else { (IntPredicate::ULT, IntPredicate::UGT) };
        let below = if lo <= from_lo { never } else { emit(self.builder.build_int_compare(lt, v, at(lo), "narrow.below"))? };
        let above = if last >= from_hi { never } else { emit(self.builder.build_int_compare(gt, v, at(last), "narrow.above"))? };
        match row.policy {
            Some(Discharge::Clamp) => {
                let v = if lo > from_lo { emit(self.builder.build_select(below, at(lo), v, "narrow.clamp.lo"))?.into_int_value() } else { v };
                let v = if last < from_hi { emit(self.builder.build_select(above, at(last), v, "narrow.clamp"))?.into_int_value() } else { v };
                Ok(Converted::Value(self.resize_int(v, from, to)?.into(), repr))
            }
            // `to`'s range is the whole of its width: wrapping into it is
            // keeping its bits (of the sign-extended value, from a signed
            // integer narrower than it)
            Some(Discharge::Wrap) => Ok(Converted::Value(self.resize_int(v, from, to)?.into(), repr)),
            Some(_) => {
                let stored = self.resize_int(v, from, to)?;
                self.lower_checked_narrowing_from(row, (v, stored), below, above, (lo, hi), &repr, &source).map(Converted::Checked)
            }
            None => Err(CodegenError::UnsupportedAt(
                format!("the narrowing into `{}` is discharged by nothing in its row; the check refuses a bare one", row.target),
                row.span,
            )),
        }
    }

    /// A `RangeError` in the current arena, its fields as the builtin
    /// table's row lays them out.
    ///
    /// Built outside the iteration arena when the `or` handling the error
    /// takes it out of the frame ([`Cx::error_payload`]).
    fn emit_range_error_alloc(&mut self, kind: &str, value: IntValue<'ctx>, low: IntValue<'ctx>, high: IntValue<'ctx>, at: Span) -> Result<inkwell::values::PointerValue<'ctx>, CodegenError> {
        self.error_payload(|cx| cx.emit_range_error_alloc_in(kind, value, low, high, at))
    }

    fn emit_range_error_alloc_in(
        &mut self,
        kind: &str,
        value: IntValue<'ctx>,
        low: IntValue<'ctx>,
        high: IntValue<'ctx>,
        at: Span,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodegenError> {
        let info = self.user_types.get("RangeError").cloned().ok_or_else(|| {
            CodegenError::UnsupportedAt("`RangeError` is not declared (the builtin table's row)".into(), at)
        })?;
        let size = info.struct_ty.size_of().expect("RangeError has a known size");
        let ptr = self.arena_alloc(size, "RangeError.alloc")?;
        let kind = self.global_string(kind);
        let fields: [(&str, BasicValueEnum<'ctx>); 4] =
            [("kind", kind.into()), ("value", value.into()), ("low", low.into()), ("high", high.into())];
        for (name, v) in fields {
            let (idx, _) = info.fields.get(name).cloned().expect("a field of the row");
            let at = emit(self.builder.build_struct_gep(info.struct_ty, ptr, idx, &format!("RangeError.{name}.ptr")))?;
            emit(self.builder.build_store(at, v))?;
        }
        Ok(ptr)
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// GH #1076 (U3): a change of denomination's row over `v`: one
    /// multiplication by the factor's numerator, a point's shift, then
    /// for a denominator above one the division its policy says: rounded,
    /// or checked exact for the `or`'s join. `.split(u)`'s row divides by
    /// `u`'s count instead and is the pair.
    pub(crate) fn lower_scale(&mut self, row: &ConversionRow, v: IntValue<'ctx>) -> Result<Converted<'ctx>, CodegenError> {
        let scale: &Scale = row.scale.as_ref().expect("a row with a scale");
        // GH #2131: the integer the value counts, whose arithmetic the
        // conversion's is: an `Int`'s wraps and divides signed, a `U64`'s
        // multiplication and shift are checked (an overflow a violation,
        // as its `*` is) and it divides unsigned. A `Decimal<S>`'s i128
        // mantissa (`.rescale(T)`) is signed.
        let repr = self.unit_repr(&row.to);
        let signed = repr.machine_int().map_or(true, |(_, s)| s);
        let int = self.unit_int(&row.to).unwrap_or(hale_syntax::ast::PrimType::Int);
        let (numerator, denominator) = scale.factor.to_machine_int(int).map_err(|o| {
            CodegenError::UnsupportedAt(
                format!(
                    "the conversion into `{}` is by the factor {}, which no `{}` holds",
                    row.target,
                    o.factor,
                    hale_types::ty::prim_name(int)
                ),
                row.span,
            )
        })?;
        // The value's own width: an integer's, or a `Decimal<S>`'s i128
        // mantissa.
        let int_t = v.get_type();
        let mut v = v;
        if numerator != 1 {
            let p = int_t.const_int(numerator, false);
            v = if signed {
                emit(self.builder.build_int_mul(v, p, "unit.scale"))?
            } else {
                self.lower_binop_values(hale_syntax::ast::BinOp::Mul, v, p, &repr)?
            };
        }
        if scale.offset != 0 {
            v = if signed {
                let k = int_t.const_int(scale.offset as u64, true);
                emit(self.builder.build_int_add(v, k, "unit.shift"))?
            } else {
                let (op, k) = if scale.offset > 0 {
                    (hale_syntax::ast::BinOp::Add, scale.offset.unsigned_abs())
                } else {
                    (hale_syntax::ast::BinOp::Sub, scale.offset.unsigned_abs())
                };
                self.lower_binop_values(op, v, int_t.const_int(k, false), &repr)?
            };
        }
        if let Some(d) = row.split {
            return self.lower_split(v, d, repr, signed);
        }
        if denominator == 1 {
            return Ok(Converted::Value(v.into(), repr));
        }
        let q = int_t.const_int(denominator, false);
        match row.policy {
            Some(Discharge::Round { policy, .. }) => {
                self.lower_rounding(v, q, policy, signed).map(|r| Converted::Value(r.into(), repr))
            }
            Some(Discharge::Clamp | Discharge::Wrap) | None => Err(CodegenError::UnsupportedAt(
                format!(
                    "the conversion into `{}` divides by {} and its row says nothing of the remainder; the check \
                     refuses a bare one",
                    row.target, denominator
                ),
                row.span,
            )),
            Some(_) => self.lower_checked_division(row, v, q, &repr).map(Converted::Checked),
        }
    }

    /// `op` over two values of the integer `repr`, by its own operator
    /// (a `U64`'s checked: GH #2131).
    fn lower_binop_values(
        &mut self,
        op: hale_syntax::ast::BinOp,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
        repr: &CodegenTy,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let (v, _) = self.lower_binop(op, l.into(), r.into(), repr)?;
        Ok(v.into_int_value())
    }

    /// The quotient and remainder of `v` by the positive `q`, signed or
    /// unsigned as the integer is.
    fn quo_rem(
        &mut self,
        v: IntValue<'ctx>,
        q: IntValue<'ctx>,
        signed: bool,
        at: &str,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let (quo, rem) = (format!("{at}.quo"), format!("{at}.rem"));
        Ok(if signed {
            (emit(self.builder.build_int_signed_div(v, q, &quo))?, emit(self.builder.build_int_signed_rem(v, q, &rem))?)
        } else {
            (emit(self.builder.build_int_unsigned_div(v, q, &quo))?, emit(self.builder.build_int_unsigned_rem(v, q, &rem))?)
        })
    }

    /// `v / q` rounded by `policy`, `q` positive: the truncating quotient
    /// and remainder, then one correction per policy. An unsigned value's
    /// remainder is never negative (GH #2131): `floor` is `trunc` there,
    /// and nothing rounds away below zero.
    fn lower_rounding(
        &mut self,
        v: IntValue<'ctx>,
        q: IntValue<'ctx>,
        policy: RoundPolicy,
        signed: bool,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let i64_t = v.get_type();
        let zero = i64_t.const_zero();
        let (quo, rem) = self.quo_rem(v, q, signed, "unit")?;
        if !signed {
            let one = i64_t.const_int(1, false);
            let away = match policy {
                RoundPolicy::Trunc | RoundPolicy::Floor => return Ok(quo),
                RoundPolicy::Ceil => emit(self.builder.build_int_compare(IntPredicate::NE, rem, zero, "unit.ceil.above"))?,
                RoundPolicy::HalfUp | RoundPolicy::HalfEven => {
                    // the remainder against the divisor's other part,
                    // with no doubling to overflow
                    let other = emit(self.builder.build_int_sub(q, rem, "unit.half.other"))?;
                    if policy == RoundPolicy::HalfUp {
                        emit(self.builder.build_int_compare(IntPredicate::UGE, rem, other, "unit.half.away"))?
                    } else {
                        let past = emit(self.builder.build_int_compare(IntPredicate::UGT, rem, other, "unit.half.past"))?;
                        let tie = emit(self.builder.build_int_compare(IntPredicate::EQ, rem, other, "unit.half.tie"))?;
                        let low = emit(self.builder.build_and(quo, one, "unit.half.low"))?;
                        let odd = emit(self.builder.build_int_compare(IntPredicate::NE, low, zero, "unit.half.odd"))?;
                        let tie_odd = emit(self.builder.build_and(tie, odd, "unit.half.tie_odd"))?;
                        emit(self.builder.build_or(past, tie_odd, "unit.half.away"))?
                    }
                }
            };
            // `quo` is at most the integer's max over `q`, `q > 1`: one
            // more never overflows.
            let up = emit(self.builder.build_int_z_extend(away, i64_t, "unit.up"))?;
            return emit(self.builder.build_int_add(quo, up, "unit.round"));
        }
        match policy {
            RoundPolicy::Trunc => Ok(quo),
            RoundPolicy::Floor => {
                let below = emit(self.builder.build_int_compare(IntPredicate::SLT, rem, zero, "unit.floor.below"))?;
                let down = emit(self.builder.build_int_z_extend(below, i64_t, "unit.floor.adj"))?;
                emit(self.builder.build_int_sub(quo, down, "unit.floor"))
            }
            RoundPolicy::Ceil => {
                let above = emit(self.builder.build_int_compare(IntPredicate::SGT, rem, zero, "unit.ceil.above"))?;
                let up = emit(self.builder.build_int_z_extend(above, i64_t, "unit.ceil.adj"))?;
                emit(self.builder.build_int_add(quo, up, "unit.ceil"))
            }
            RoundPolicy::HalfUp | RoundPolicy::HalfEven => {
                // |r| against q - |r|: twice the remainder against the
                // divisor, with no doubling to overflow.
                let neg = emit(self.builder.build_int_compare(IntPredicate::SLT, rem, zero, "unit.half.neg"))?;
                let negated = emit(self.builder.build_int_sub(zero, rem, "unit.half.negated"))?;
                let mag = emit(self.builder.build_select(neg, negated, rem, "unit.half.mag"))?.into_int_value();
                let other = emit(self.builder.build_int_sub(q, mag, "unit.half.other"))?;
                let away = if policy == RoundPolicy::HalfUp {
                    emit(self.builder.build_int_compare(IntPredicate::SGE, mag, other, "unit.half.away"))?
                } else {
                    let past = emit(self.builder.build_int_compare(IntPredicate::SGT, mag, other, "unit.half.past"))?;
                    let tie = emit(self.builder.build_int_compare(IntPredicate::EQ, mag, other, "unit.half.tie"))?;
                    let low = emit(self.builder.build_and(quo, i64_t.const_int(1, false), "unit.half.low"))?;
                    let odd = emit(self.builder.build_int_compare(IntPredicate::NE, low, zero, "unit.half.odd"))?;
                    let tie_odd = emit(self.builder.build_and(tie, odd, "unit.half.tie_odd"))?;
                    emit(self.builder.build_or(past, tie_odd, "unit.half.away"))?
                };
                let step = emit(self.builder.build_select(neg, i64_t.const_all_ones(), i64_t.const_int(1, false), "unit.half.step"))?
                    .into_int_value();
                let adj = emit(self.builder.build_select(away, step, zero, "unit.half.adj"))?.into_int_value();
                emit(self.builder.build_int_add(quo, adj, "unit.half"))
            }
        }
    }

    /// `v / q` checked exact: the quotient in the success slot, or an
    /// `InexactError` in the error slot, and the path bit the `or`
    /// branches on (1 = a remainder).
    fn lower_checked_division(
        &mut self,
        row: &ConversionRow,
        v: IntValue<'ctx>,
        q: IntValue<'ctx>,
        repr: &CodegenTy,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .ok_or_else(|| CodegenError::UnsupportedAt("a conversion outside a fn body".into(), row.span))?;
        let i64_t = v.get_type();
        let signed = repr.machine_int().map_or(true, |(_, s)| s);
        let (quo, rem) = self.quo_rem(v, q, signed, "unit")?;
        let inexact = emit(self.builder.build_int_compare(IntPredicate::NE, rem, i64_t.const_zero(), "unit.inexact"))?;
        let payload_ty = CodegenTy::TypeRef("InexactError".into());
        let success_ty = repr.clone();
        let out_val_slot = self.alloca_for(&success_ty, "unit.val.slot")?;
        let out_err_slot = self.alloca_for(&payload_ty, "unit.err.slot")?;
        let ok_bb = self.context.append_basic_block(func, "unit.ok");
        let err_bb = self.context.append_basic_block(func, "unit.err");
        let join_bb = self.context.append_basic_block(func, "unit.join");
        emit(self.builder.build_conditional_branch(inexact, err_bb, ok_bb))?;

        self.builder.position_at_end(ok_bb);
        emit(self.builder.build_store(out_val_slot, quo))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(err_bb);
        let info = self.user_types.get("InexactError").cloned().ok_or_else(|| {
            CodegenError::UnsupportedAt("`InexactError` is not declared (the builtin table's row)".into(), row.span)
        })?;
        let size = info.struct_ty.size_of().expect("InexactError has a known size");
        let ptr = self.arena_alloc(size, "InexactError.alloc")?;
        let kind = self.global_string(&row.target);
        // A `Decimal<S>`'s mantissa is an i128 and the error's fields are
        // `Int`s: each is the nearest `Int`, exact for a mantissa within
        // an `Int`'s range (the divisor, a power of ten, always is); a
        // `U64`'s past `Int`'s max is `Int`'s max (GH #2131).
        let (v, q) = (self.saturate_to_int(v, repr)?, self.saturate_to_int(q, repr)?);
        let fields: [(&str, BasicValueEnum<'ctx>); 3] = [("kind", kind.into()), ("value", v.into()), ("divisor", q.into())];
        for (name, value) in fields {
            let (idx, _) = info.fields.get(name).cloned().expect("a field of the row");
            let at = emit(self.builder.build_struct_gep(info.struct_ty, ptr, idx, &format!("InexactError.{name}.ptr")))?;
            emit(self.builder.build_store(at, value))?;
        }
        emit(self.builder.build_store(out_err_slot, ptr))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: inexact,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(success_ty),
            payload_ty,
        })
    }

    /// `v`, a value of `repr`, as an `Int`: itself when it is one, else
    /// the nearest `Int` (a `Decimal`'s i128, a `U64` past `Int`'s max).
    fn saturate_to_int(&mut self, v: IntValue<'ctx>, repr: &CodegenTy) -> Result<IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        if let Some((bits, false)) = repr.machine_int() {
            let v = if bits < 64 { emit(self.builder.build_int_z_extend(v, i64_t, "sat.wide"))? } else { v };
            let max = i64_t.const_int(i64::MAX as u64, false);
            let above = emit(self.builder.build_int_compare(IntPredicate::UGT, v, max, "sat.above"))?;
            return Ok(emit(self.builder.build_select(above, max, v, "sat.int"))?.into_int_value());
        }
        if v.get_type().get_bit_width() == 64 {
            return Ok(v);
        }
        // GH #2132: a signed integer narrower than an `Int` is one exactly
        if v.get_type().get_bit_width() < 64 {
            return emit(self.builder.build_int_s_extend(v, i64_t, "sat.wide"));
        }
        let wide = v.get_type();
        let (max, min) = (wide.const_int(i64::MAX as u64, false), wide.const_int(i64::MIN as u64, true));
        let above = emit(self.builder.build_int_compare(IntPredicate::SGT, v, max, "sat.above"))?;
        let below = emit(self.builder.build_int_compare(IntPredicate::SLT, v, min, "sat.below"))?;
        let v = emit(self.builder.build_select(above, max, v, "sat.hi"))?.into_int_value();
        let v = emit(self.builder.build_select(below, min, v, "sat.lo"))?.into_int_value();
        emit(self.builder.build_int_truncate(v, i64_t, "sat.int"))
    }

    /// `.split(u)`: the floored quotient by `u`'s count `d` and the
    /// remainder, which is then never negative, as a pair. The whole part
    /// is a count of the integer the value counts (GH #2131); an unsigned
    /// one's quotient is floored already.
    fn lower_split(&mut self, v: IntValue<'ctx>, d: i128, rest_ty: CodegenTy, signed: bool) -> Result<Converted<'ctx>, CodegenError> {
        let int_t = v.get_type();
        let zero = int_t.const_zero();
        let dc = int_t.const_int(d as u64, false);
        let (quo, rem) = self.quo_rem(v, dc, signed, "unit.split")?;
        let (whole, rest) = if signed {
            let below = emit(self.builder.build_int_compare(IntPredicate::SLT, rem, zero, "unit.split.below"))?;
            let down = emit(self.builder.build_int_z_extend(below, int_t, "unit.split.adj"))?;
            let whole = emit(self.builder.build_int_sub(quo, down, "unit.split.whole"))?;
            let lift = emit(self.builder.build_select(below, dc, zero, "unit.split.lift"))?.into_int_value();
            (whole, emit(self.builder.build_int_add(rem, lift, "unit.split.rest"))?)
        } else {
            (quo, rem)
        };
        // the whole part is the integer itself: a `Duration`'s an `Int`
        let whole_ty = match rest_ty {
            CodegenTy::Duration | CodegenTy::Time => CodegenTy::Int,
            ref other => other.clone(),
        };
        let elem_tys = vec![whole_ty, rest_ty];
        let storage_ty = self.llvm_tuple_storage_type(&elem_tys);
        let bytes = storage_ty.size_of().expect("tuple storage type has known size");
        let tup_ptr = self.arena_alloc(bytes, "unit.split.alloc")?;
        let i32_t = self.context.i32_type();
        for (i, part) in [whole, rest].into_iter().enumerate() {
            let slot = unsafe {
                emit(self.builder.build_gep(
                    storage_ty,
                    tup_ptr,
                    &[i32_t.const_int(0, false), i32_t.const_int(i as u64, false)],
                    &format!("unit.split.slot{i}"),
                ))?
            };
            emit(self.builder.build_store(slot, part))?;
        }
        Ok(Converted::Value(tup_ptr.into(), CodegenTy::Tuple(elem_tys)))
    }
}

/// Whether a value lowered as `ty` is one a conversion of the unit
/// dialect converts: a machine integer (an `Int`, a `U64`: GH #2131), or
/// a `Duration` or `Time` (U4). Two such values the checker compared are
/// compared as the counts they are.
pub(crate) fn is_quantity_repr(ty: &CodegenTy) -> bool {
    ty.machine_int().is_some()
}

/// A row's bound as a machine `Int`: a bound past an `Int`'s is the
/// nearest one, which no value is outside of.
fn saturate(b: i128) -> i64 {
    b.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}
