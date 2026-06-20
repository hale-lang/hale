//! `std::math::*` path-call lowering. Four generic helpers
//! (unary / nullary_float / is_nan / binary) cover the libm
//! wrappers; dispatch in `codegen.rs` picks the libm symbol.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait MathStdlib<'ctx> {
    fn lower_std_math_unary(
        &mut self,
        libm_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_math_nullary_float(
        &mut self,
        sym: &str,
        surface_name: &str,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_math_is_nan(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_math_binary(
        &mut self,
        libm_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_math_int_to_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_math_float_to_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_math_to_int(
        &mut self,
        surface_name: &str,
        do_round: bool,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> MathStdlib<'ctx> for Cx<'ctx, 'p> {
    /// std::math::<sqrt|exp|log|floor|ceil> — single Float arg →
    /// Float result. Routes to the libm extern declared in
    /// declare_builtins. Int args coerce to Float via sitofp at
    /// the call site (same Int→Float widening this commit adds).
    fn lower_std_math_unary(
        &mut self,
        libm_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::math::{} takes 1 argument, got {}",
                libm_name,
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        let f = self.coerce_to_float(v, &ty, &format!("std::math::{}", libm_name))?;
        let func = self
            .module
            .get_function(libm_name)
            .expect("libm fn declared in declare_builtins");
        let call = self
            .builder
            .build_call(
                func,
                &[f.into()],
                &format!("std::math::{}.call", libm_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let result = call
            .try_as_basic_value()
            .left()
            .expect("libm unary returns f64");
        Ok((result, CodegenTy::Float))
    }

    /// C8 (pond follow-up): nullary Float-returning math
    /// primitive. Used for `std::math::nan()` and
    /// `std::math::inf()`. Routes to a `lotus_math_*` C-runtime
    /// wrapper rather than emitting an LLVM `fdiv 0.0/0.0` (for
    /// nan) or the largest f64 constant (for inf) so the surface
    /// is one consistent symbol family with `tanh` / `is_nan`,
    /// and so behavior matches C's NAN / INFINITY macros across
    /// host platforms. IEEE 754 quiet-NaN semantics in both cases.
    fn lower_std_math_nullary_float(
        &mut self,
        sym: &str,
        surface_name: &str,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::math::{} takes 0 arguments, got {}",
                surface_name,
                args.len()
            )));
        }
        let func = self
            .module
            .get_function(sym)
            .expect("lotus_math_nan / lotus_math_inf declared in declare_builtins");
        let call = self
            .builder
            .build_call(func, &[], &format!("std::math::{}.call", surface_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let result = call
            .try_as_basic_value()
            .left()
            .expect("lotus_math_nan / lotus_math_inf returns f64");
        Ok((result, CodegenTy::Float))
    }

    /// C8 (pond follow-up): `std::math::is_nan(f: Float) -> Bool`.
    /// Canonical IEEE 754 NaN test (`f != f`). C primitive returns
    /// i32 (0/1); truncated to i1 here via the same compare-to-
    /// zero pattern `lotus_fs_file_exists` uses.
    fn lower_std_math_is_nan(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::math::is_nan takes 1 argument, got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        let f = self.coerce_to_float(v, &ty, "std::math::is_nan")?;
        let func = self
            .module
            .get_function("lotus_math_is_nan")
            .expect("lotus_math_is_nan declared in declare_builtins");
        let call = self
            .builder
            .build_call(func, &[f.into()], "std::math::is_nan.call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("lotus_math_is_nan returns i32")
            .into_int_value();
        let i32_t = self.context.i32_type();
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "is_nan.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    /// std::math::pow — two Float args → Float result. Same
    /// libm pass-through pattern as the unary helper. Int args
    /// coerce.
    fn lower_std_math_binary(
        &mut self,
        libm_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::math::{} takes 2 arguments, got {}",
                libm_name,
                args.len()
            )));
        }
        let (a_val, a_ty) = self.lower_expr(&args[0], scope)?;
        let a_f = self.coerce_to_float(a_val, &a_ty, &format!("std::math::{} arg 0", libm_name))?;
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        let b_f = self.coerce_to_float(b_val, &b_ty, &format!("std::math::{} arg 1", libm_name))?;
        let func = self
            .module
            .get_function(libm_name)
            .expect("libm fn declared in declare_builtins");
        let call = self
            .builder
            .build_call(
                func,
                &[a_f.into(), b_f.into()],
                &format!("std::math::{}.call", libm_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let result = call
            .try_as_basic_value()
            .left()
            .expect("libm binary returns f64");
        Ok((result, CodegenTy::Float))
    }

    /// std::math::int_to_float(i: Int) -> Float. Explicit widening
    /// conversion (`sitofp`). Numeric consumers used to round-trip
    /// Int→Float through ASCII (`to_string` + `parse_float`); this
    /// is the direct lowering. Reuses `coerce_to_float`, which is
    /// the Int→f64 `sitofp` already used at Float-arg sites — so an
    /// already-Float arg passes through unchanged.
    fn lower_std_math_int_to_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::math::int_to_float takes 1 argument, got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        let f = self.coerce_to_float(v, &ty, "std::math::int_to_float")?;
        Ok((f.into(), CodegenTy::Float))
    }

    /// std::math::float_to_int(f: Float) -> Int. Explicit narrowing
    /// conversion (`fptosi`, round-toward-zero — the same semantics
    /// as a C `(long)` cast). Int→Float widening is implicit at
    /// Float-arg sites, but Float→Int narrowing stays explicit (no
    /// silent truncation), so this is its blessed entry point. An
    /// Int arg passes through unchanged.
    fn lower_std_math_float_to_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        self.lower_std_math_to_int("float_to_int", false, args, scope)
    }

    /// Shared Float→Int narrowing. `do_round == false` truncates
    /// toward zero (`fptosi`, C `(long)` semantics) — the
    /// `float_to_int` / `trunc` surface. `do_round == true` rounds
    /// half away from zero, the conventional `round()`: shift by
    /// `copysign(0.5, f)` before the `fptosi`, so `3.7 -> 4`,
    /// `2.5 -> 3`, `-2.5 -> -3`. The shift is computed with a
    /// compare + select (no `llvm.round` / `llvm.copysign`
    /// intrinsic), so the whole path is native LLVM that lowers to
    /// `f64.lt` / `select` / `f64.add` / `i64.trunc_f64_s` on
    /// wasm32 — no libm symbol, no host import (unlike sin / cos).
    /// An Int arg passes through unchanged for generic callers.
    fn lower_std_math_to_int(
        &mut self,
        surface_name: &str,
        do_round: bool,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::math::{} takes 1 argument, got {}",
                surface_name,
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        let i64_t = self.context.i64_type();
        let int_val = match ty {
            CodegenTy::Float => {
                let mut f = v.into_float_value();
                if do_round {
                    let f64_t = self.context.f64_type();
                    let zero = f64_t.const_zero();
                    let is_neg = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OLT,
                            f,
                            zero,
                            "round.is_neg",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let offset = self
                        .builder
                        .build_select(
                            is_neg,
                            f64_t.const_float(-0.5),
                            f64_t.const_float(0.5),
                            "round.offset",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_float_value();
                    f = self
                        .builder
                        .build_float_add(f, offset, "round.shifted")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                self.builder
                    .build_float_to_signed_int(f, i64_t, "float.to.int")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            CodegenTy::Int => v.into_int_value(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "std::math::{}: expected Float (or Int \
                     passthrough), got {:?}",
                    surface_name, other
                )))
            }
        };
        Ok((int_val.into(), CodegenTy::Int))
    }
}
