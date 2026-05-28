//! `std::io::sockopt::*` path-call lowering. One generic getter
//! routes the ~30 named-constant surface (IPPROTO_*, IP_*, SO_*,
//! SOL_SOCKET) to per-constant C primitives. The constant list
//! `SOCKOPT_NAMES` lives in `codegen.rs` because it's consulted at
//! declare-builtins time too.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx};

pub(crate) trait SockoptStdlib<'ctx> {
    fn lower_std_io_sockopt_getter(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> SockoptStdlib<'ctx> for Cx<'ctx, 'p> {
    /// 2026-05-26 — `std::io::sockopt::<NAME>() -> Int`. Each
    /// named constant resolves to a zero-arg call into the
    /// matching C getter (`lotus_sockopt_<NAME>`) which returns
    /// the platform's numeric value. Used as the level / name
    /// args to `std::io::udp::set_option_int` / friends.
    fn lower_std_io_sockopt_getter(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::io::sockopt::{} takes 0 args, got {}",
                name,
                args.len()
            )));
        }
        let f = self
            .module
            .get_function(&format!("lotus_sockopt_{}", name))
            .expect("sockopt getter declared");
        let v = self
            .builder
            .build_call(f, &[], &format!("sockopt.{}.ret", name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let v_i64 = self
            .builder
            .build_int_s_extend(v, self.context.i64_type(), "sockopt.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((v_i64.into(), CodegenTy::Int))
    }
}
