//! Conversions of the unit dialect's values (GH #1076), lowered from
//! their rows.
//!
//! The checker classifies every conversion and records it in the typed
//! bodies' `conversions` column (`hale_types::typed_bodies::ConversionRow`):
//! its kind, the range a narrowing checks against, the exact factor and
//! shift of a change of denomination, and what discharges it.
//! [`Cx::lower_conversion`] reads the row and decides nothing. An
//! identity, a range, a quantity and a point are each an `Int` here
//! (`user_type_aliases`).
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
    /// it is; a division is the `Int`'s. Inside a default every row is
    /// the evaluation path's being lowered (`default_evaluation`): the
    /// default's name means what that scope says, a cast in one and a
    /// local's call in another, and its literals and values convert into
    /// what that scope flows them into.
    pub(crate) fn conversion_operand<'e>(&self, e: &'e Expr) -> Option<(ConversionRow, Option<&'e Expr>)> {
        match e {
            Expr::Call { callee, id, args, .. } => {
                let operand = match callee.as_ref() {
                    Expr::Ident(_) => match args.as_slice() {
                        [arg] => Some(arg),
                        _ => None,
                    },
                    Expr::Field { receiver, name, .. } if matches!(name.name.as_str(), "in" | "split") => {
                        Some(receiver.as_ref())
                    }
                    _ => return None,
                };
                let row = self.row(SiteKind::Cast(id.0))?;
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
                format!("the conversion into `{}` converts an `Int`; its value lowered as {ty:?}", row.target),
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
        value: i64,
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
            Some(row) => (row.count.ok_or_else(missing)?, quantity_repr(&row.to)),
            // U4: a body the checker types no row in (the stdlib's own,
            // a `bindings { }` initializer) has the literal's own row of
            // the stdlib's catalogue: a time literal is that count of
            // `Duration`, in every program.
            None if !self.current_body.is_some_and(|b| self.typed.has_conversions_in(b)) => {
                let (count, ty) = hale_types::units::stdlib_literal(value, unit).ok_or_else(missing)?;
                (count, quantity_repr(&ty))
            }
            None => return Err(missing()),
        };
        Ok((self.context.i64_type().const_int(count as u64, true).into(), ty))
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
        Some(quantity_repr(&self.row(SiteKind::operator(span))?.to))
    }

    /// GH #1076 (U3): the unit a printed value's row writes after its
    /// count (`msec`, ` Money in 1/100 cent`).
    pub(crate) fn printed_unit(&self, e: &Expr) -> Option<String> {
        if !self.typed.has_quantity_rows() {
            return None;
        }
        self.row(SiteKind::printed(e.span()))?.printed.clone()
    }

    /// The key of the conversion of `kind` here: inside a default, on the
    /// evaluation path being lowered (`default_evaluation`), the
    /// checker's key for the same evaluation (`site` in `hale-types`).
    fn site(&self, kind: SiteKind) -> ConversionSite {
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

    /// Lower the conversion `row` of the value `arg`.
    pub(crate) fn lower_conversion(
        &mut self,
        row: &ConversionRow,
        arg: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<Converted<'ctx>, CodegenError> {
        let (v, ty) = self.lower_expr(arg, scope)?;
        if !is_quantity_repr(&ty) {
            return Err(CodegenError::UnsupportedAt(
                format!("`{}(…)` converts an `Int`; its value lowered as {ty:?}", row.target),
                arg.span(),
            ));
        }
        let v = v.into_int_value();
        if row.scale.is_some() {
            return self.lower_scale(row, v);
        }
        if row.kind != ConversionKind::Narrowing {
            // An identity or a range is its `Int`: nothing to emit.
            return Ok(Converted::Value(v.into(), CodegenTy::Int));
        }
        let Some((lo, hi)) = row.range else {
            return Err(CodegenError::UnsupportedAt(
                format!("the narrowing into `{}` has no range in its row", row.target),
                row.span,
            ));
        };
        let i64_t = self.context.i64_type();
        let (lo, last) = (saturate(lo), saturate(hi - 1));
        let (lo_c, last_c) = (i64_t.const_int(lo as u64, true), i64_t.const_int(last as u64, true));
        let below = emit(self.builder.build_int_compare(IntPredicate::SLT, v, lo_c, "narrow.below"))?;
        let above = emit(self.builder.build_int_compare(IntPredicate::SGT, v, last_c, "narrow.above"))?;
        match row.policy {
            Some(Discharge::Clamp) => {
                let up = emit(self.builder.build_select(below, lo_c, v, "narrow.clamp.lo"))?.into_int_value();
                let c = emit(self.builder.build_select(above, last_c, up, "narrow.clamp"))?;
                Ok(Converted::Value(c, CodegenTy::Int))
            }
            Some(Discharge::Wrap) => self.lower_wrap(row, v, lo, hi).map(|w| Converted::Value(w.into(), CodegenTy::Int)),
            Some(_) => self.lower_checked_narrowing(row, v, below, above, lo_c, last_c).map(Converted::Checked),
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
    /// taken non-negative, for every `Int` `v` and without overflow. The
    /// distance between `v` and `lo` is exact as an unsigned `Int` (the
    /// wrapping subtraction of the smaller from the larger), so each side
    /// takes an unsigned remainder: at or above `lo`, `t = (v - lo) urem
    /// w`; below it, `m = (lo - v) urem w` and `t = w - m`, or `0` when
    /// `m` is. `t < w`, so `lo + t` is at most `hi - 1`. Both sides are
    /// computed and one selected: neither can trap.
    fn lower_wrap(&mut self, row: &ConversionRow, v: IntValue<'ctx>, lo: i64, hi: i128) -> Result<IntValue<'ctx>, CodegenError> {
        let width = hi - i128::from(lo);
        let Ok(width) = i64::try_from(width) else {
            return Err(CodegenError::UnsupportedAt(
                format!("`{}`'s range is wider than an `Int`: `or wrap` has no width to wrap by", row.target),
                row.span,
            ));
        };
        let i64_t = self.context.i64_type();
        let (lo_c, width_c) = (i64_t.const_int(lo as u64, true), i64_t.const_int(width as u64, true));
        let zero = i64_t.const_zero();
        let at_or_above = emit(self.builder.build_int_compare(IntPredicate::SGE, v, lo_c, "narrow.wrap.up"))?;
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
    /// branches on (1 = outside the range).
    fn lower_checked_narrowing(
        &mut self,
        row: &ConversionRow,
        v: IntValue<'ctx>,
        below: IntValue<'ctx>,
        above: IntValue<'ctx>,
        lo_c: IntValue<'ctx>,
        last_c: IntValue<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .ok_or_else(|| CodegenError::UnsupportedAt("a narrowing outside a fn body".into(), row.span))?;
        let outside = emit(self.builder.build_or(below, above, "narrow.outside"))?;
        let payload_ty = CodegenTy::TypeRef("RangeError".into());
        let out_val_slot = self.alloca_for(&CodegenTy::Int, "narrow.val.slot")?;
        let out_err_slot = self.alloca_for(&payload_ty, "narrow.err.slot")?;
        let ok_bb = self.context.append_basic_block(func, "narrow.ok");
        let err_bb = self.context.append_basic_block(func, "narrow.err");
        let join_bb = self.context.append_basic_block(func, "narrow.join");
        emit(self.builder.build_conditional_branch(outside, err_bb, ok_bb))?;

        self.builder.position_at_end(ok_bb);
        emit(self.builder.build_store(out_val_slot, v))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(err_bb);
        let high = emit(self.builder.build_int_add(last_c, self.context.i64_type().const_int(1, false), "narrow.high"))?;
        let err = self.emit_range_error_alloc(&row.target, v, lo_c, high, row.span)?;
        emit(self.builder.build_store(out_err_slot, err))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: outside,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(CodegenTy::Int),
            payload_ty,
        })
    }

    /// A `RangeError` in the current arena, its fields as the builtin
    /// table's row lays them out.
    fn emit_range_error_alloc(
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
        let machine = scale.factor.to_machine().map_err(|o| {
            CodegenError::UnsupportedAt(
                format!("the conversion into `{}` is by the factor {}, which no `Int` holds", row.target, o.factor),
                row.span,
            )
        })?;
        let i64_t = self.context.i64_type();
        let mut v = v;
        if machine.numerator != 1 {
            let p = i64_t.const_int(machine.numerator as u64, true);
            v = emit(self.builder.build_int_mul(v, p, "unit.scale"))?;
        }
        if scale.offset != 0 {
            let k = i64_t.const_int(scale.offset as u64, true);
            v = emit(self.builder.build_int_add(v, k, "unit.shift"))?;
        }
        if let Some(d) = row.split {
            return self.lower_split(v, d, quantity_repr(&row.to));
        }
        if machine.denominator == 1 {
            return Ok(Converted::Value(v.into(), quantity_repr(&row.to)));
        }
        let q = i64_t.const_int(machine.denominator as u64, true);
        match row.policy {
            Some(Discharge::Round { policy, .. }) => {
                self.lower_rounding(v, q, policy).map(|r| Converted::Value(r.into(), quantity_repr(&row.to)))
            }
            Some(Discharge::Clamp | Discharge::Wrap) | None => Err(CodegenError::UnsupportedAt(
                format!(
                    "the conversion into `{}` divides by {} and its row says nothing of the remainder; the check \
                     refuses a bare one",
                    row.target, machine.denominator
                ),
                row.span,
            )),
            Some(_) => self.lower_checked_division(row, v, q).map(Converted::Checked),
        }
    }

    /// `v / q` rounded by `policy`, `q` positive: the truncating quotient
    /// and remainder, then one correction per policy.
    fn lower_rounding(&mut self, v: IntValue<'ctx>, q: IntValue<'ctx>, policy: RoundPolicy) -> Result<IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let zero = i64_t.const_zero();
        let quo = emit(self.builder.build_int_signed_div(v, q, "unit.quo"))?;
        let rem = emit(self.builder.build_int_signed_rem(v, q, "unit.rem"))?;
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
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .ok_or_else(|| CodegenError::UnsupportedAt("a conversion outside a fn body".into(), row.span))?;
        let i64_t = self.context.i64_type();
        let quo = emit(self.builder.build_int_signed_div(v, q, "unit.quo"))?;
        let rem = emit(self.builder.build_int_signed_rem(v, q, "unit.rem"))?;
        let inexact = emit(self.builder.build_int_compare(IntPredicate::NE, rem, i64_t.const_zero(), "unit.inexact"))?;
        let payload_ty = CodegenTy::TypeRef("InexactError".into());
        let out_val_slot = self.alloca_for(&CodegenTy::Int, "unit.val.slot")?;
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
            success_ty: Some(quantity_repr(&row.to)),
            payload_ty,
        })
    }

    /// `.split(u)`: the floored quotient by `u`'s count `d` and the
    /// remainder, which is then never negative, as a pair.
    fn lower_split(&mut self, v: IntValue<'ctx>, d: i64, rest_ty: CodegenTy) -> Result<Converted<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let zero = i64_t.const_zero();
        let dc = i64_t.const_int(d as u64, true);
        let quo = emit(self.builder.build_int_signed_div(v, dc, "unit.split.quo"))?;
        let rem = emit(self.builder.build_int_signed_rem(v, dc, "unit.split.rem"))?;
        let below = emit(self.builder.build_int_compare(IntPredicate::SLT, rem, zero, "unit.split.below"))?;
        let down = emit(self.builder.build_int_z_extend(below, i64_t, "unit.split.adj"))?;
        let whole = emit(self.builder.build_int_sub(quo, down, "unit.split.whole"))?;
        let lift = emit(self.builder.build_select(below, dc, zero, "unit.split.lift"))?.into_int_value();
        let rest = emit(self.builder.build_int_add(rem, lift, "unit.split.rest"))?;
        let elem_tys = vec![CodegenTy::Int, rest_ty];
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

/// The representation a value of a unit-dialect type `t` lowers as: the
/// stdlib's `Duration` and `Time` keep their own class (U4), every other
/// quantity, point, identity or range is the `Int` it counts.
pub(crate) fn quantity_repr(t: &hale_types::ty::Ty) -> CodegenTy {
    use hale_syntax::ast::PrimType;
    match t {
        hale_types::ty::Ty::Prim(PrimType::Duration) => CodegenTy::Duration,
        hale_types::ty::Ty::Prim(PrimType::Time) => CodegenTy::Time,
        hale_types::ty::Ty::Prim(PrimType::Bool) => CodegenTy::Bool,
        _ => CodegenTy::Int,
    }
}

/// Whether a value lowered as `ty` is one a conversion of the unit
/// dialect converts: an `Int`, or a `Duration` or `Time` (U4).
fn is_quantity_repr(ty: &CodegenTy) -> bool {
    matches!(ty, CodegenTy::Int | CodegenTy::Duration | CodegenTy::Time)
}

/// A row's bound as a machine `Int`: a bound past an `Int`'s is the
/// nearest one, which no value is outside of.
fn saturate(b: i128) -> i64 {
    b.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}
