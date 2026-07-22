//! `std::decimal::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

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
}
