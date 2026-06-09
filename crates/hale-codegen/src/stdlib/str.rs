//! `std::str::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait StrStdlib<'ctx> {
    fn lower_std_str_parse_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_str_parse_float_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_str_parse_decimal_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_str_byte_at_unchecked(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_json_scan(
        &mut self,
        fn_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_range_parse_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_str_range_parse_decimal_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_str_range_eq(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_index_of(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_can_parse_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_case_fold(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        which: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_substring(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_repeat(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_pad(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        which: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_replace(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_builder_new(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_builder_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_builder_len(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_builder_finish(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_clone(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_can_parse_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_str_from_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> StrStdlib<'ctx> for Cx<'ctx, 'p> {
    /// 2026-05-17 — `std::str::parse_int(s) -> Int
    /// fallible(ParseError)`. Composes `lotus_str_can_parse_int`
    /// (success predicate) with `lotus_str_parse_int` (value
    /// extractor) so the err arm fires when the input isn't
    /// parseable.
    fn lower_std_str_parse_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_int takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_int: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let can_fn = self
            .module
            .get_function("lotus_str_can_parse_int")
            .expect("lotus_str_can_parse_int declared");
        let can_i32 = self
            .builder
            .build_call(can_fn, &[s_val.into()], "parse_int.can")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                can_i32,
                self.context.i32_type().const_zero(),
                "parse_int.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parse_fn = self
            .module
            .get_function("lotus_str_parse_int")
            .expect("lotus_str_parse_int declared");
        let value_i64 = self
            .builder
            .build_call(parse_fn, &[s_val.into()], "parse_int.value")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        self.complete_parse_fallible_call(
            is_err,
            s_val,
            "parse_int",
            Some((value_i64, CodegenTy::Int)),
            "str.parse_int",
        )
    }

    /// 2026-05-17 — `std::str::parse_float(s) -> Float
    /// fallible(ParseError)`. Same shape as parse_int_fallible.
    fn lower_std_str_parse_float_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_float takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_float: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let can_fn = self
            .module
            .get_function("lotus_str_can_parse_float")
            .expect("lotus_str_can_parse_float declared");
        let can_i32 = self
            .builder
            .build_call(can_fn, &[s_val.into()], "parse_float.can")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                can_i32,
                self.context.i32_type().const_zero(),
                "parse_float.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parse_fn = self
            .module
            .get_function("lotus_str_parse_float")
            .expect("lotus_str_parse_float declared");
        let value_f64 = self
            .builder
            .build_call(parse_fn, &[s_val.into()], "parse_float.value")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns f64");
        self.complete_parse_fallible_call(
            is_err,
            s_val,
            "parse_float",
            Some((value_f64, CodegenTy::Float)),
            "str.parse_float",
        )
    }

    /// `std::str::parse_decimal(s) -> Decimal fallible(ParseError)`.
    /// Same shape as parse_int_fallible. The C primitive returns
    /// the i128 mantissa via two i64 out-params (hi:lo split) to
    /// match the lotus_decimal_to_string convention; this lowering
    /// reconstructs the i128 LLVM value via sext+shl+or.
    fn lower_std_str_parse_decimal_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_decimal takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_decimal: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let i64_t = self.context.i64_type();
        let i128_t = self.context.i128_type();
        let can_fn = self
            .module
            .get_function("lotus_str_can_parse_decimal")
            .expect("lotus_str_can_parse_decimal declared");
        let can_i32 = self
            .builder
            .build_call(can_fn, &[s_val.into()], "parse_decimal.can")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                can_i32,
                self.context.i32_type().const_zero(),
                "parse_decimal.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi_slot = self.alloca_in_entry(i64_t.into(), "parse_decimal.hi")?;
        let lo_slot = self.alloca_in_entry(i64_t.into(), "parse_decimal.lo")?;
        let parse_fn = self
            .module
            .get_function("lotus_str_parse_decimal")
            .expect("lotus_str_parse_decimal declared");
        self.builder
            .build_call(
                parse_fn,
                &[s_val.into(), hi_slot.into(), lo_slot.into()],
                "parse_decimal.split",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi_i64 = self
            .builder
            .build_load(i64_t, hi_slot, "parse_decimal.hi.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let lo_i64 = self
            .builder
            .build_load(i64_t, lo_slot, "parse_decimal.lo.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let hi_wide = self
            .builder
            .build_int_s_extend(hi_i64, i128_t, "parse_decimal.hi.sext")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let lo_wide = self
            .builder
            .build_int_z_extend(lo_i64, i128_t, "parse_decimal.lo.zext")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let shift = i128_t.const_int(64, false);
        let hi_shifted = self
            .builder
            .build_left_shift(hi_wide, shift, "parse_decimal.hi.shl")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let value_i128 = self
            .builder
            .build_or(hi_shifted, lo_wide, "parse_decimal.value")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_parse_fallible_call(
            is_err,
            s_val,
            "parse_decimal",
            Some((value_i128.into(), CodegenTy::Decimal)),
            "str.parse_decimal",
        )
    }

    /// 2026-05-26 — `std::str::byte_at_unchecked(s: String, i: Int)
    /// -> Int`. Direct byte access at offset i, NO bounds check
    /// — caller must ensure 0 <= i < len(s). Returns the byte
    /// value 0..255 as Int. Used by stdlib scan helpers (JSON
    /// walkers) where the bound is externally known and a
    /// per-access strlen would tank perf. Misuse → UB.
    fn lower_std_str_byte_at_unchecked(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::byte_at_unchecked takes 2 args (s, i), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::byte_at_unchecked: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (i_val, i_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(i_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::byte_at_unchecked: i must be Int, got {:?}",
                i_ty
            )));
        }
        // Inline as `zext(load i8, ptr + i)` instead of a call — a Hale
        // String is a `char*`, so byte i is one GEP + load. This is the
        // hot path for every byte-scanning routine (the JSON cursor, the
        // pack readers, hand-rolled scans); a function call per byte
        // dominated their cost. The "unchecked" contract already promises
        // the caller guarantees `0 <= i < len`, so no bounds check (a
        // buggy caller gets a raw OOB load — same UB the name implies).
        let i8_t = self.context.i8_type();
        let s_ptr = s_val.into_pointer_value();
        let idx = i_val.into_int_value();
        let byte_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_t, s_ptr, &[idx], "byte_at.gep")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let byte = self
            .builder
            .build_load(i8_t, byte_ptr, "byte_at.byte")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let v = self
            .builder
            .build_int_z_extend(byte, self.context.i64_type(), "byte_at.zext")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((v.into(), CodegenTy::Int))
    }

    /// JSON Tier-3 Level-A scan primitive: `(json: String, from: Int) ->
    /// Int`, dispatched to a `lotus_json_next_*` SIMD runtime fn. Shares
    /// the `byte_at_unchecked` shape (String blob + offset → offset).
    fn lower_json_scan(
        &mut self,
        fn_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::json::{} takes 3 args (json, from, len), got {}",
                fn_name,
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::json::{}: first arg must be String, got {:?}",
                fn_name, s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (from_val, from_ty) = self.lower_expr(&args[1], scope)?;
        let (len_val, len_ty) = self.lower_expr(&args[2], scope)?;
        if !matches!(from_ty, CodegenTy::Int) || !matches!(len_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::json::{}: from and len must be Int, got {:?}, {:?}",
                fn_name, from_ty, len_ty
            )));
        }
        let f = self
            .module
            .get_function(fn_name)
            .unwrap_or_else(|| panic!("{} declared", fn_name));
        let v = self
            .builder
            .build_call(f, &[s_val.into(), from_val.into(), len_val.into()], "json.scan.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// 2026-05-26 — `std::str::range_parse_int(json: String, start: Int,
    /// end_exclusive: Int) -> Int fallible(ParseError)`. Range-
    /// bounded variant of parse_int — no need to materialize the
    /// substring as an owned String. Used by the JSON walk to
    /// dodge per-field allocations.
    fn lower_std_str_range_parse_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_int takes 3 args \
                 (json, start, end_exclusive), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_int: json must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (start_val, start_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(start_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_int: start must be Int, got {:?}",
                start_ty
            )));
        }
        let (end_val, end_ty) = self.lower_expr(&args[2], scope)?;
        if !matches!(end_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_int: end_exclusive must be Int, got {:?}",
                end_ty
            )));
        }
        let can_fn = self
            .module
            .get_function("lotus_str_can_parse_int_range")
            .expect("lotus_str_can_parse_int_range declared");
        let can_i32 = self
            .builder
            .build_call(
                can_fn,
                &[s_val.into(), start_val.into(), end_val.into()],
                "range_parse_int.can",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                can_i32,
                self.context.i32_type().const_zero(),
                "range_parse_int.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parse_fn = self
            .module
            .get_function("lotus_str_parse_int_range")
            .expect("lotus_str_parse_int_range declared");
        let value_i64 = self
            .builder
            .build_call(
                parse_fn,
                &[s_val.into(), start_val.into(), end_val.into()],
                "range_parse_int.value",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        self.complete_parse_fallible_call(
            is_err,
            s_val,
            "range_parse_int",
            Some((value_i64, CodegenTy::Int)),
            "str.range_parse_int",
        )
    }

    /// 2026-05-26 — `std::str::range_parse_decimal(json: String,
    /// start: Int, end_exclusive: Int) -> Decimal
    /// fallible(ParseError)`. Range-bounded parse_decimal.
    fn lower_std_str_range_parse_decimal_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_decimal takes 3 args \
                 (json, start, end_exclusive), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_decimal: json must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (start_val, start_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(start_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_decimal: start must be Int, got {:?}",
                start_ty
            )));
        }
        let (end_val, end_ty) = self.lower_expr(&args[2], scope)?;
        if !matches!(end_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_parse_decimal: end_exclusive must be Int, got {:?}",
                end_ty
            )));
        }
        let i64_t = self.context.i64_type();
        let i128_t = self.context.i128_type();
        let can_fn = self
            .module
            .get_function("lotus_str_can_parse_decimal_range")
            .expect("lotus_str_can_parse_decimal_range declared");
        let can_i32 = self
            .builder
            .build_call(
                can_fn,
                &[s_val.into(), start_val.into(), end_val.into()],
                "range_parse_decimal.can",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                can_i32,
                self.context.i32_type().const_zero(),
                "range_parse_decimal.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi_slot = self.alloca_in_entry(i64_t.into(), "range_parse_decimal.hi")?;
        let lo_slot = self.alloca_in_entry(i64_t.into(), "range_parse_decimal.lo")?;
        let parse_fn = self
            .module
            .get_function("lotus_str_parse_decimal_range")
            .expect("lotus_str_parse_decimal_range declared");
        self.builder
            .build_call(
                parse_fn,
                &[
                    s_val.into(),
                    start_val.into(),
                    end_val.into(),
                    hi_slot.into(),
                    lo_slot.into(),
                ],
                "range_parse_decimal.split",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let hi_i64 = self
            .builder
            .build_load(i64_t, hi_slot, "range_parse_decimal.hi.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let lo_i64 = self
            .builder
            .build_load(i64_t, lo_slot, "range_parse_decimal.lo.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let hi_wide = self
            .builder
            .build_int_s_extend(hi_i64, i128_t, "range_parse_decimal.hi.sext")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let lo_wide = self
            .builder
            .build_int_z_extend(lo_i64, i128_t, "range_parse_decimal.lo.zext")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let shift = i128_t.const_int(64, false);
        let hi_shifted = self
            .builder
            .build_left_shift(hi_wide, shift, "range_parse_decimal.hi.shl")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let value_i128 = self
            .builder
            .build_or(hi_shifted, lo_wide, "range_parse_decimal.value")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_parse_fallible_call(
            is_err,
            s_val,
            "range_parse_decimal",
            Some((value_i128.into(), CodegenTy::Decimal)),
            "str.range_parse_decimal",
        )
    }

    /// 2026-05-26 — `std::str::range_eq(json: String, start: Int,
    /// end_exclusive: Int, expected: String) -> Bool`. True iff
    /// json[start..end_exclusive] == expected, byte-for-byte. No
    /// substring materialization.
    fn lower_std_str_range_eq(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 4 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_eq takes 4 args \
                 (json, start, end_exclusive, expected), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_eq: json must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (start_val, start_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(start_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_eq: start must be Int, got {:?}",
                start_ty
            )));
        }
        let (end_val, end_ty) = self.lower_expr(&args[2], scope)?;
        if !matches!(end_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_eq: end_exclusive must be Int, got {:?}",
                end_ty
            )));
        }
        let (t_val, t_ty) = self.lower_expr(&args[3], scope)?;
        if !matches!(t_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::range_eq: expected must be String, got {:?}",
                t_ty
            )));
        }
        let t_val = self.unpack_view_if_needed(t_val, &t_ty)?;
        let f = self
            .module
            .get_function("lotus_str_range_eq")
            .expect("lotus_str_range_eq declared");
        let i32_v = self
            .builder
            .build_call(
                f,
                &[
                    s_val.into(),
                    start_val.into(),
                    end_val.into(),
                    t_val.into(),
                ],
                "str.range_eq.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        // Convert i32 → i1 (Bool ABI) via ne-zero.
        let b = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                i32_v,
                self.context.i32_type().const_zero(),
                "str.range_eq.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((b.into(), CodegenTy::Bool))
    }

    /// Lower `std::str::parse_int(s: String) -> Int`. Atoi-ish:
    /// returns 0 on parse failure or empty input. Disambiguate
    /// via `std::str::can_parse_int` if needed. Strict trailing-
    /// char check — "42abc" rejects, returns 0.
    /// Lower `std::str::index_of(s: String, sub: String) -> Int`.
    /// Returns the byte index of the first occurrence of `sub` in
    /// `s`, or -1 when `sub` doesn't appear. Empty needle returns
    /// 0 by convention. Wraps `lotus_str_index_of` directly. m84:
    /// the substring-search primitive HTTP request parsing leans
    /// on (find ` ` between method and path, `\r\n` to bound the
    /// request line).
    fn lower_std_str_index_of(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::index_of takes 2 args (s, sub), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::index_of: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (sub_val, sub_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(sub_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::index_of: sub must be String, got {:?}",
                sub_ty
            )));
        }
        let sub_val = self.unpack_view_if_needed(sub_val, &sub_ty)?;
        let f = self
            .module
            .get_function("lotus_str_index_of")
            .expect("lotus_str_index_of declared");
        let call = self
            .builder
            .build_call(
                f,
                &[s_val.into(), sub_val.into()],
                "str.index_of.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// Lower `std::str::can_parse_int(s: String) -> Bool`.
    fn lower_std_str_can_parse_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_int takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_int: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let i32_t = self.context.i32_type();
        let f = self
            .module
            .get_function("lotus_str_can_parse_int")
            .expect("lotus_str_can_parse_int declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "can.parse.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "can.parse.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    /// v1.x: `std::str::lower(s)` / `std::str::upper(s)` (ASCII
    /// case folding) and `std::str::trim(s)` (whitespace strip).
    /// All take one String, return a new String in the bus
    /// payload arena.
    fn lower_std_str_case_fold(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        which: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{} takes 1 arg (s), got {}",
                which,
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            let hint = if matches!(s_ty, CodegenTy::Bytes) {
                " — use `std::str::from_bytes(b)` to convert"
            } else {
                ""
            };
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: s must be String, got {:?}{}",
                which, s_ty, hint
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let extern_name = match which {
            "lower" => "lotus_str_lower",
            "upper" => "lotus_str_upper",
            "trim"  => "lotus_str_trim",
            _ => unreachable!(),
        };
        let f = self
            .module
            .get_function(extern_name)
            .expect("string fold/strip extern declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[s_val.into()], &format!("str.{}.ret", which))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// 2026-05-17: `std::str::substring(s, lo, hi) -> String`.
    /// Byte-indexed slice over a String. Same shape as the
    /// compose `std::str::from_bytes(std::bytes::slice(
    /// std::bytes::from_string(s), lo, hi))` but one call.
    /// Negative lo / hi past end / inverted bounds collapse to
    /// "". Result lives in the global payload arena.
    fn lower_std_str_substring(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::substring takes 3 args (s, lo, hi), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::substring: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (lo_val, lo_ty) = self.lower_expr(&args[1], scope)?;
        if lo_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::str::substring: lo must be Int, got {:?}",
                lo_ty
            )));
        }
        let (hi_val, hi_ty) = self.lower_expr(&args[2], scope)?;
        if hi_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::str::substring: hi must be Int, got {:?}",
                hi_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_substring")
            .expect("lotus_str_substring declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(
                f,
                &[s_val.into(), lo_val.into(), hi_val.into()],
                "str.substring.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call.try_as_basic_value().left().expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x: `std::str::repeat(s, n) -> String`. Concatenates `s`
    /// with itself n times. n <= 0 returns empty.
    fn lower_std_str_repeat(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::repeat takes 2 args (s, n), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::repeat: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let (n_val, n_ty) = self.lower_expr(&args[1], scope)?;
        if n_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::str::repeat: n must be Int, got {:?}",
                n_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_repeat")
            .expect("lotus_str_repeat declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[s_val.into(), n_val.into()], "str.repeat.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call.try_as_basic_value().left().expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x: `std::str::pad_left(s, width, pad) -> String` and
    /// `pad_right`. `pad` is a single-char String; only the first
    /// byte is used. If s is already >= width, returns s unchanged.
    fn lower_std_str_pad(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        which: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{} takes 3 args (s, width, pad), got {}",
                which,
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        let (w_val, w_ty) = self.lower_expr(&args[1], scope)?;
        let (p_val, p_ty) = self.lower_expr(&args[2], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: s must be String, got {:?}",
                which, s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        if w_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: width must be Int, got {:?}",
                which, w_ty
            )));
        }
        if !matches!(p_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: pad must be String, got {:?}",
                which, p_ty
            )));
        }
        let p_val = self.unpack_view_if_needed(p_val, &p_ty)?;
        let extern_name = match which {
            "pad_left" => "lotus_str_pad_left",
            "pad_right" => "lotus_str_pad_right",
            _ => unreachable!(),
        };
        let f = self
            .module
            .get_function(extern_name)
            .expect("pad extern declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(
                f,
                &[s_val.into(), w_val.into(), p_val.into()],
                &format!("str.{}.ret", which),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call.try_as_basic_value().left().expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x: `std::str::replace(s, needle, replacement) -> String`.
    /// Naive O(n*m) scan; greedy-forward (each match advances by
    /// needle_len, not 1). Empty needle is a no-op (avoids the
    /// infinite-replace footgun).
    fn lower_std_str_replace(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::replace takes 3 args (s, needle, replacement), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        let (n_val, n_ty) = self.lower_expr(&args[1], scope)?;
        let (r_val, r_ty) = self.lower_expr(&args[2], scope)?;
        for (label, ty) in &[
            ("s", &s_ty),
            ("needle", &n_ty),
            ("replacement", &r_ty),
        ] {
            if **ty != CodegenTy::String {
                return Err(CodegenError::Unsupported(format!(
                    "std::str::replace: {} must be String, got {:?}",
                    label, ty
                )));
            }
        }
        let f = self
            .module
            .get_function("lotus_str_replace")
            .expect("lotus_str_replace declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(
                f,
                &[s_val.into(), n_val.into(), r_val.into()],
                "str.replace.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x-15: `std::str::builder_new() -> Bytes`. Allocates a
    /// doubling-realloc-backed buffer; Bytes is the carrier type
    /// (opaque — users shouldn't index into it, only pass through
    /// to the other builder_* fns). Resolves the
    /// reader-list_item-quadratic-concat friction by turning N
    /// append calls into amortized O(N) total cost.
    fn lower_std_str_builder_new(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_new takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_builder_new")
            .expect("lotus_str_builder_new declared");
        let call = self
            .builder
            .build_call(f, &[], "sb.new.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// v1.x-15: `std::str::builder_append(b: Bytes, s: String) -> Bytes`.
    /// Mutates the builder in place; returns the builder pointer so
    /// both statement (`builder_append(b, "x");`) and expression
    /// (`let b2 = builder_append(b, "x");`, fluent chaining) usages
    /// work. The pointer is the same one passed in — the type-level
    /// return lets the expression dispatcher hand back a usable value.
    fn lower_std_str_builder_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_append takes 2 args (b, s), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_append: builder must be Bytes \
                 (from builder_new), got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let (s_val, s_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_append: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let f = self
            .module
            .get_function("lotus_str_builder_append")
            .expect("lotus_str_builder_append declared");
        self.builder
            .build_call(f, &[b_val.into(), s_val.into()], "sb.append")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((b_val, CodegenTy::Bytes))
    }

    /// v1.x-15: `std::str::builder_len(b: Bytes) -> Int`. Inspect
    /// the running length without materializing the final String.
    fn lower_std_str_builder_len(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_len takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_len: builder must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_str_builder_len")
            .expect("lotus_str_builder_len declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sb.len.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// v1.x-15: `std::str::builder_finish(b: Bytes) -> String`.
    /// Materializes the accumulated string in the bus payload
    /// arena (lives for the rest of the program) and frees the
    /// builder. The Bytes handle must NOT be reused after finish.
    fn lower_std_str_builder_finish(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_finish takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_finish: builder must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_str_builder_finish")
            .expect("lotus_str_builder_finish declared");
        // F.6/F.8 sweep (iris FRICTION): publish the current arena
        // into the caller-arena TLS so the C-side's
        // lotus_bus_payload_arena_alloc routes the returned String
        // through this frame's arena. Same prologue as
        // stdin::read_line and the str_lower / str_upper family.
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sb.finish.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// F.30 companion: `std::str::clone(v: StringView) -> String`.
    /// Reuses the existing m49 `lotus_str_clone` machinery — same
    /// arena-arg shape — with the caller's current_arena as the
    /// target. Also accepts `String` as a no-op clone.
    fn lower_std_str_clone(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::clone takes 1 arg (view), got {}",
                args.len()
            )));
        }
        let (v_val, v_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(v_ty, CodegenTy::StringView | CodegenTy::String) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::clone: arg must be StringView or String, got {:?}",
                v_ty
            )));
        }
        let v_val = self.unpack_view_if_needed(v_val, &v_ty)?;
        let arena = self.current_arena_ptr()?;
        let f = self
            .module
            .get_function("lotus_str_clone")
            .expect("lotus_str_clone declared");
        let call = self
            .builder
            .build_call(
                f,
                &[arena.into(), v_val.into()],
                "str.clone.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// v1.x-16: `std::str::can_parse_float(s: String) -> Bool`.
    fn lower_std_str_can_parse_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_float takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_float: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let i32_t = self.context.i32_type();
        let f = self
            .module
            .get_function("lotus_str_can_parse_float")
            .expect("lotus_str_can_parse_float declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "can.parse.float.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "can.parse.float.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    /// Phase 2g: lower `std::str::from_bytes(b: Bytes) -> String`.
    /// Allocates a (len+1)-byte buffer in the global payload arena,
    /// memcpys the Bytes body, NUL-terminates. Embedded NULs in the
    /// source persist in the buffer but the strlen-based String view
    /// will truncate at the first — by design (callers who need
    /// NUL-safe handling stay in Bytes).
    fn lower_std_str_from_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::from_bytes takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::str::from_bytes: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_str_from_bytes")
            .expect("lotus_str_from_bytes declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "str_from_bytes.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

}
