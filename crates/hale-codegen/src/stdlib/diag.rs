//! `std::diag::*` test-time gate counters (#7 of the fast-protocol-I/O
//! substrate plan). Read-only views over the process-wide heap-allocation
//! and I/O-syscall counters maintained by the `__wrap_*` shims in
//! `lotus_arena.c`. A steady-state region reads a counter before and after
//! and asserts the delta — the runtime/test-time complement to compile-time
//! `--warn-unbounded-alloc`. Both return `-1` when the wrap shim is absent
//! (sanitizer builds), so a caller can tell "gate unavailable" from a real 0.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait DiagStdlib<'ctx> {
    fn lower_std_diag_heap_alloc_count(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_diag_syscall_count(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> DiagStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::diag::heap_alloc_count() -> Int` — cumulative count of
    /// heap allocations (malloc / realloc / calloc / mmap) the runtime
    /// has made, or -1 if the wrap shim isn't compiled in.
    fn lower_std_diag_heap_alloc_count(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::diag::heap_alloc_count takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_diag_heap_alloc_count")
            .expect("lotus_diag_heap_alloc_count declared");
        let v = self
            .builder
            .build_call(f, &[], "diag.heap_alloc_count.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// `std::diag::syscall_count(name: String) -> Int` — cumulative
    /// count of the named I/O syscall ("recv", "recvmsg", "read",
    /// "write", "send", "sendto"). -1 for an unknown name or when the
    /// wrap shim isn't compiled in.
    fn lower_std_diag_syscall_count(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::diag::syscall_count takes 1 arg (name), got {}",
                args.len()
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(name_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::diag::syscall_count: name must be String, got {:?}",
                name_ty
            )));
        }
        let name_ptr = self.unpack_view_if_needed(name_val, &name_ty)?;
        let f = self
            .module
            .get_function("lotus_diag_syscall_count")
            .expect("lotus_diag_syscall_count declared");
        let v = self
            .builder
            .build_call(f, &[name_ptr.into()], "diag.syscall_count.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }
}
