//! `std::rand::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait RandStdlib<'ctx> {
    fn lower_std_rand_seed_from_time(
        &mut self,
        args: &[Expr],
    ) -> Result<(), CodegenError>;

    fn lower_std_rand_next_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> RandStdlib<'ctx> for Cx<'ctx, 'p> {
    /// ws-echo `random-seed-missing`: lower
    /// `std::rand::seed_from_time()` — re-seed the shared xorshift64*
    /// state from CLOCK_MONOTONIC. Library-internal use only; not
    /// cryptographically secure. Statement-position only.
    fn lower_std_rand_seed_from_time(
        &mut self,
        args: &[Expr],
    ) -> Result<(), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::rand::seed_from_time takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_rand_seed_from_time")
            .expect("lotus_rand_seed_from_time declared");
        self.builder
            .build_call(f, &[], "rand.seed")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// ws-echo `random-seed-missing`: lower
    /// `std::rand::next_int(max: Int) -> Int` — uniform-ish int in
    /// [0, max). max <= 0 returns 0. Auto-seeds from monotonic
    /// time on first call so callers that forget the explicit
    /// seed still get distinct values per process run.
    fn lower_std_rand_next_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::rand::next_int takes 1 arg (max), got {}",
                args.len()
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[0], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::rand::next_int: max must be Int, got {}",
                max_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_rand_next_int")
            .expect("lotus_rand_next_int declared");
        let call = self
            .builder
            .build_call(f, &[max_val.into()], "rand.next.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }
}
