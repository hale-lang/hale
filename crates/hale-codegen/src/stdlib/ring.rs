//! `std::ring::*` path-call lowering — GH #244: the SPSC
//! observation ring as a lotus primitive over caller-provided
//! memory. The `__spsc_*` surface is deliberately raw (every
//! param an Int: addresses into an shm segment the caller
//! mapped, word values, counts) — pond's `observe` library
//! wraps it; the layout contract lives in spec/runtime.md and
//! the C in runtime/lotus_arena.c.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait RingStdlib<'ctx> {
    fn lower_std_ring_op(
        &mut self,
        c_fn: &str,
        arity: usize,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> RingStdlib<'ctx> for Cx<'ctx, 'p> {
    /// Shared lowering for the all-Int `std::ring::__spsc_*`
    /// primitives: lower each arg (must be Int), call the C fn,
    /// surface an i64 result (the call's return when it has one,
    /// else 0 so statement-position calls type uniformly).
    fn lower_std_ring_op(
        &mut self,
        c_fn: &str,
        arity: usize,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != arity {
            return Err(CodegenError::Unsupported(format!(
                "std::ring::{}: takes {} Int arg(s), got {}",
                c_fn, arity, args.len()
            )));
        }
        let mut lowered = Vec::with_capacity(arity);
        for (i, a) in args.iter().enumerate() {
            let (v, ty) = self.lower_expr(a, scope)?;
            if ty != CodegenTy::Int {
                return Err(CodegenError::Unsupported(format!(
                    "std::ring::{}: arg {} must be Int \
                     (addresses/words/counts), got {:?}",
                    c_fn,
                    i + 1,
                    ty
                )));
            }
            lowered.push(v.into());
        }
        let f = self
            .module
            .get_function(c_fn)
            .unwrap_or_else(|| panic!("{} declared", c_fn));
        let call = self
            .builder
            .build_call(f, &lowered, "ring.op")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i64_t = self.context.i64_type();
        let ret = call
            .try_as_basic_value()
            .left()
            .unwrap_or_else(|| i64_t.const_zero().into());
        Ok((ret, CodegenTy::Int))
    }
}
