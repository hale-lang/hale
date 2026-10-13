//! `std::decimal::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, FallibleCallResult, Scope};

pub(crate) trait DecimalStdlib<'ctx> {
    fn lower_std_decimal_to_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_decimal_format(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_decimal_from_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_decimal_from_float_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
}

impl<'ctx, 'p> DecimalStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::decimal::to_float(d: Decimal) -> Float` (2026-05-21).
    /// Direct i128 → f64 conversion at scale 9 via the new
    /// `lotus_decimal_to_float` C primitive. Replaces the
    /// `to_string(d)` → strip "ns"-like-suffix → `parse_float`
    /// ASCII round-trip downstream consumers were doing in
    /// hot paths (e.g. metrics gauges setting Float values
    /// from Decimal book prices). The i128 splits into hi/lo
    /// at the call boundary to match the codebase's existing
    /// Decimal C ABI convention.
    fn lower_std_decimal_to_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::to_float takes 1 arg (Decimal), got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        if ty != CodegenTy::Decimal {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::to_float: arg must be Decimal, got {:?}",
                ty
            )));
        }
        let i64_t = self.context.i64_type();
        let i128_v = v.into_int_value();
        let lo = self
            .builder
            .build_int_truncate(i128_v, i64_t, "dec.to_f.lo")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let shift = self.context.i128_type().const_int(64, false);
        let hi_wide = self
            .builder
            .build_right_shift(i128_v, shift, true, "dec.to_f.hi_w")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi = self
            .builder
            .build_int_truncate(hi_wide, i64_t, "dec.to_f.hi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_decimal_to_float")
            .expect("lotus_decimal_to_float declared");
        let res = self
            .builder
            .build_call(f, &[hi.into(), lo.into()], "dec.to_float.call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_decimal_to_float returns double");
        Ok((res, CodegenTy::Float))
    }

    /// GH #230 item 2: `std::decimal::format(d: Decimal,
    /// places: Int) -> String` — exactly `places` fraction
    /// digits (0..=9 clamped), round half-up. The explicit
    /// surface for fixed-places display; default printing keeps
    /// trimming trailing zeros (declared precision is not
    /// stored in the scale-9 repr).
    fn lower_std_decimal_format(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::format takes 2 args (Decimal, places), got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        if ty != CodegenTy::Decimal {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::format: arg 1 must be Decimal, got {:?}",
                ty
            )));
        }
        let (places_v, places_ty) = self.lower_expr(&args[1], scope)?;
        if places_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::format: arg 2 (places) must be Int, got {:?}",
                places_ty
            )));
        }
        let i64_t = self.context.i64_type();
        let i128_v = v.into_int_value();
        let lo = self
            .builder
            .build_int_truncate(i128_v, i64_t, "dec.fmt.lo")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let shift = self.context.i128_type().const_int(64, false);
        let hi_wide = self
            .builder
            .build_right_shift(i128_v, shift, true, "dec.fmt.hi_w")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi = self
            .builder
            .build_int_truncate(hi_wide, i64_t, "dec.fmt.hi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_decimal_format")
            .expect("lotus_decimal_format declared");
        let res = self
            .builder
            .build_call(
                f,
                &[hi.into(), lo.into(), places_v.into()],
                "dec.format.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_decimal_format returns ptr");
        Ok((res, CodegenTy::String))
    }

    /// `std::decimal::from_int(n: Int) -> Decimal`: `n` exactly, the
    /// mantissa `n × 10⁹` in the i128 (every Int fits: |n × 10⁹| <
    /// 2⁹⁴). Replaces `parse_decimal(to_string(n))` (downstream handoff).
    fn lower_std_decimal_from_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::from_int takes 1 arg (Int), got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        if ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::from_int: arg must be Int, got {:?}",
                ty
            )));
        }
        let i128_t = self.context.i128_type();
        let wide = self
            .builder
            .build_int_s_extend(v.into_int_value(), i128_t, "dec.from_int.wide")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let m = self
            .builder
            .build_int_mul(wide, i128_t.const_int(1_000_000_000, false), "dec.from_int")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((m.into(), CodegenTy::Decimal))
    }

    /// `std::decimal::from_float(x: Float) -> Decimal fallible(ParseError)`:
    /// `x`'s exact binary value rounded half away from zero at the ninth
    /// fractional digit (`lotus_decimal_from_float` states how). NaN, an infinity
    /// or a magnitude past Decimal's range (about 1.7e29) is
    /// `ParseError { kind: "from_float", input: to_string(x) }`, the text
    /// formatted on the error path only. An Int argument widens.
    fn lower_std_decimal_from_float_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::decimal::from_float takes 1 arg (Float), got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        let f64_t = self.context.f64_type();
        let x = match ty {
            CodegenTy::Float => v.into_float_value(),
            CodegenTy::Int => self
                .builder
                .build_signed_int_to_float(v.into_int_value(), f64_t, "dec.from_f.widen")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "std::decimal::from_float: arg must be Float, got {:?}",
                    other
                )))
            }
        };
        let i64_t = self.context.i64_type();
        let hi_slot = self.alloca_for(&CodegenTy::Int, "dec.from_f.hi.slot")?;
        let lo_slot = self.alloca_for(&CodegenTy::Int, "dec.from_f.lo.slot")?;
        for slot in [hi_slot, lo_slot] {
            self.builder
                .build_store(slot, i64_t.const_zero())
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        let f = self
            .module
            .get_function("lotus_decimal_from_float")
            .expect("lotus_decimal_from_float declared");
        let rc = self
            .builder
            .build_call(f, &[x.into(), hi_slot.into(), lo_slot.into()], "dec.from_f.rc")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, rc, i64_t.const_zero(), "dec.from_f.is_err")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi = self
            .builder
            .build_load(i64_t, hi_slot, "dec.from_f.hi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let lo = self
            .builder
            .build_load(i64_t, lo_slot, "dec.from_f.lo")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let i128_t = self.context.i128_type();
        let hi_w = self
            .builder
            .build_int_s_extend(hi, i128_t, "dec.from_f.hi_w")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi_sh = self
            .builder
            .build_left_shift(hi_w, i128_t.const_int(64, false), "dec.from_f.hi_sh")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let lo_w = self
            .builder
            .build_int_z_extend(lo, i128_t, "dec.from_f.lo_w")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let m = self
            .builder
            .build_or(hi_sh, lo_w, "dec.from_f.m")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_parse_fallible_call_with(
            is_err,
            move |cx| {
                let arena_ptr = cx.current_arena_ptr()?;
                let to_str = cx
                    .module
                    .get_function("lotus_str_from_float")
                    .expect("lotus_str_from_float declared");
                Ok(cx
                    .builder
                    .build_call(to_str, &[arena_ptr.into(), x.into()], "dec.from_f.input")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_from_float returns ptr"))
            },
            "from_float",
            Some((m.into(), CodegenTy::Decimal)),
            "decimal.from_float",
        )
    }
}
