//! `std::env::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait EnvStdlib<'ctx> {
    fn lower_std_env_args_count(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_env_arg(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_env_arg_or(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_env_var(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_env_var_exists(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> EnvStdlib<'ctx> for Cx<'ctx, 'p> {
    /// Lower `std::env::args_count() -> Int`. Returns argc as
    /// captured in main's prelude (m77 codegen change).
    fn lower_std_env_args_count(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::env::args_count takes 0 args, got {}",
                args.len()
            )));
        }
        let i64_t = self.context.i64_type();
        let f = self
            .module
            .get_function("lotus_env_args_count")
            .expect("lotus_env_args_count declared");
        let call = self
            .builder
            .build_call(f, &[], "argc.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let raw = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ext = self
            .builder
            .build_int_s_extend(raw, i64_t, "argc.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ext.into(), CodegenTy::Int))
    }

    /// Lower `std::env::arg(i: Int) -> String`. Returns argv[i]
    /// for valid i; out-of-range indices return the empty
    /// String (the C runtime's stable g_empty_str). Negative i
    /// also returns empty rather than UB.
    fn lower_std_env_arg(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::env::arg takes 1 arg (index), got {}",
                args.len()
            )));
        }
        let (i_val, i_ty) = self.lower_expr(&args[0], scope)?;
        if i_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::env::arg: index must be Int, got {:?}",
                i_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i_i32 = self
            .builder
            .build_int_truncate(i_val.into_int_value(), i32_t, "arg.i.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_env_arg")
            .expect("lotus_env_arg declared");
        let call = self
            .builder
            .build_call(f, &[i_i32.into()], "arg.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// 2026-05-16: `std::env::arg_or(idx: Int, default: String)
    /// -> String` — return argv[idx] if present, otherwise the
    /// default. Collapses the 3-line pattern every CLI-style
    /// program reinvents:
    ///   let mut x = "";
    ///   if std::env::args_count() > idx { x = std::env::arg(idx); }
    ///   into:
    ///   let x = std::env::arg_or(idx, "");
    fn lower_std_env_arg_or(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::env::arg_or takes 2 args (index, default), got {}",
                args.len()
            )));
        }
        let (i_val, i_ty) = self.lower_expr(&args[0], scope)?;
        if i_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::env::arg_or: index must be Int, got {:?}",
                i_ty
            )));
        }
        let (d_val, d_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(d_ty, CodegenTy::String) {
            return Err(CodegenError::Unsupported(format!(
                "std::env::arg_or: default must be String, got {:?}",
                d_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i_i32 = self
            .builder
            .build_int_truncate(i_val.into_int_value(), i32_t, "arg_or.i.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let count_fn = self
            .module
            .get_function("lotus_env_args_count")
            .expect("lotus_env_args_count declared");
        let count = self
            .builder
            .build_call(count_fn, &[], "arg_or.count")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let present = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                count,
                i_i32,
                "arg_or.present",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let arg_fn = self
            .module
            .get_function("lotus_env_arg")
            .expect("lotus_env_arg declared");
        let arg_val = self
            .builder
            .build_call(arg_fn, &[i_i32.into()], "arg_or.arg")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        // lotus_env_arg returns a usable pointer for any idx
        // (out-of-range yields a NUL-terminated empty string per
        // the runtime contract), so it's always safe to call and
        // select between it and the default.
        let chosen = self
            .builder
            .build_select(present, arg_val, d_val, "arg_or.chosen")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((chosen, CodegenTy::String))
    }

    /// Lower `std::env::var(name: String) -> String`. Returns the
    /// env value or empty String for unset vars. Use
    /// `std::env::var_exists` to disambiguate.
    fn lower_std_env_var(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var takes 1 arg (name), got {}",
                args.len()
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(name_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var: name must be String, got {:?}",
                name_ty
            )));
        }
        let name_val = self.unpack_view_if_needed(name_val, &name_ty)?;
        let f = self
            .module
            .get_function("lotus_env_var")
            .expect("lotus_env_var declared");
        let call = self
            .builder
            .build_call(f, &[name_val.into()], "var.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// Lower `std::env::var_exists(name: String) -> Bool`.
    fn lower_std_env_var_exists(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var_exists takes 1 arg (name), got {}",
                args.len()
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(name_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var_exists: name must be String, got {:?}",
                name_ty
            )));
        }
        let name_val = self.unpack_view_if_needed(name_val, &name_ty)?;
        let i32_t = self.context.i32_type();
        let f = self
            .module
            .get_function("lotus_env_var_exists")
            .expect("lotus_env_var_exists declared");
        let call = self
            .builder
            .build_call(f, &[name_val.into()], "var_exists.ret")
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
                "var.exists.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_bool.into(), CodegenTy::Bool))
    }
}
