//! `std::term::*` + raw stdout byte I/O path-call lowering (pond P4,
//! stage 1). The five terminal/OS shims pond vendored as FFI glue, moved
//! into the stdlib — see `notes/stdlib-term-primitives.md`. Stage 1 is the
//! two `ConsoleSink`/frame-renderer needs: `is_tty` and `write_bytes`.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait TermStdlib<'ctx> {
    fn lower_std_term_is_tty(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_stdout_write_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> TermStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::term::is_tty(fd: Int) -> Bool` — `isatty(fd)`. Lets a logger
    /// probe "is stderr a tty?" without vendoring an FFI shim.
    fn lower_std_term_is_tty(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::term::is_tty takes 1 arg (fd: Int), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(fd_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::term::is_tty: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_term_is_tty")
            .expect("lotus_term_is_tty declared");
        let ret = self
            .builder
            .build_call(f, &[fd_val.into()], "is_tty.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_term_is_tty returns i64")
            .into_int_value();
        // i64 0/1 → Bool (i1) via ne-zero.
        let b = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret,
                self.context.i64_type().const_zero(),
                "is_tty.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((b.into(), CodegenTy::Bool))
    }

    /// `std::io::stdout::write_bytes(s: String) -> Int` — `fflush(stdout)`
    /// then a raw `write(1, ...)`, bypassing the prelude's `_IOLBF`
    /// line-buffering so a multi-line frame isn't flushed per newline.
    /// Returns the byte count written, `-1` on error (sentinel, matching
    /// the planned `std::io::stdin::read_byte` shape — a write error on
    /// this hot path is a control outcome the caller checks, not a heavier
    /// fallible return). The `fflush` keeps ordering consistent with any
    /// buffered `println` output.
    fn lower_std_io_stdout_write_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::stdout::write_bytes takes 1 arg (s: String), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::stdout::write_bytes: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let f = self
            .module
            .get_function("lotus_term_write_stdout")
            .expect("lotus_term_write_stdout declared");
        let ret = self
            .builder
            .build_call(f, &[s_val.into()], "write_bytes.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_term_write_stdout returns i64");
        Ok((ret, CodegenTy::Int))
    }
}
