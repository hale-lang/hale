//! Conversions between the unit dialect's identities and ranges (GH
//! #1076, U2), lowered from their rows.
//!
//! The checker classifies every conversion and records it in the typed
//! bodies' `conversions` column (`hale_types::typed_bodies::ConversionRow`):
//! its kind, the range a narrowing checks against and what discharges it.
//! [`Cx::lower_conversion`] reads the row and decides nothing: an
//! identity or a range is an `Int` here (`user_type_aliases`), so a total
//! conversion and a widening emit nothing; a narrowing emits the two
//! comparisons and, from the row's policy, the clamp (two selects), the
//! wrap (an unsigned remainder of the distance from the low bound, on
//! whichever side of it the value is), or the checked value
//! the `or`'s join takes (the substitute, the handler, the raise).

use hale_syntax::ast::Expr;
use hale_syntax::Span;
use hale_types::typed_bodies::{ConversionKind, ConversionRow, ConversionSite, Discharge};
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

use crate::codegen::{CodegenError, CodegenTy, Cx, FallibleCallResult, Scope};

/// What a conversion lowered to.
pub(crate) enum Converted<'ctx> {
    /// The converted `Int`.
    Value(BasicValueEnum<'ctx>, CodegenTy),
    /// A narrowing its `or` discharges: the checked value, or the
    /// `RangeError`, for the `or`'s join (`lower_or_expr`).
    Checked(FallibleCallResult<'ctx>),
}

fn emit<T>(r: Result<T, inkwell::builder::BuilderError>) -> Result<T, CodegenError> {
    r.map_err(|e| CodegenError::LlvmEmit(e.to_string()))
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The conversion row of `e`, when `e` is a call the checker
    /// classified as one. No row is the total answer: the call is no
    /// conversion, whatever its callee is named (a local, a parameter or
    /// a fn the checker resolved the name to, or `Int(x)`, the numeric
    /// builtin), and is lowered as the call it is. Inside a default the
    /// row is the evaluation's being lowered (`default_evaluation`): the
    /// default's name means what that scope says, a cast in one and a
    /// local's call in another.
    pub(crate) fn conversion_row(&self, e: &Expr) -> Option<ConversionRow> {
        let Expr::Call { callee, id, .. } = e else { return None };
        let Expr::Ident(_) = callee.as_ref() else { return None };
        let site = match self.default_evaluation {
            Some(at) => ConversionSite::DefaultCast { at, call: id.0 },
            None => ConversionSite::Cast(id.0),
        };
        self.typed.conversion(site).cloned()
    }

    /// Lower the conversion `row` of the value `arg`.
    pub(crate) fn lower_conversion(
        &mut self,
        row: &ConversionRow,
        arg: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<Converted<'ctx>, CodegenError> {
        let (v, ty) = self.lower_expr(arg, scope)?;
        if ty != CodegenTy::Int {
            return Err(CodegenError::UnsupportedAt(
                format!("`{}(…)` converts an `Int`; its value lowered as {ty:?}", row.target),
                arg.span(),
            ));
        }
        let v = v.into_int_value();
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

/// A row's bound as a machine `Int`: a bound past an `Int`'s is the
/// nearest one, which no value is outside of.
fn saturate(b: i128) -> i64 {
    b.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}
