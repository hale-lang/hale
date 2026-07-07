//! `std::io::stdin::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait IoStdinStdlib<'ctx> {
    fn lower_std_io_stdin_read_line(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_io_stdin_read_line_status(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> IoStdinStdlib<'ctx> for Cx<'ctx, 'p> {
    /// Lower `std::io::stdin::read_line() -> String`. Reads one
    /// line from stdin via the C-runtime shim (POSIX getline +
    /// payload-arena copy + newline stripping). Returns the empty
    /// string on EOF / IO error; pair with `read_line_status()` to
    /// distinguish an empty line from EOF.
    fn lower_std_io_stdin_read_line(
        &mut self,
        args: &[Expr],
        _scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::io::stdin::read_line takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_stdin_read_line")
            .expect("lotus_stdin_read_line declared");
        // F.5 fix (a downstream tool's issue tracker, 2026-05-22): publish the
        // current arena into the caller-arena TLS so the C-side
        // `lotus_bus_payload_arena_alloc` routes the returned
        // line through THIS frame's arena, not whatever stale
        // value the last nested call left behind. Without this,
        // a method-body `while`-loop calling read_line would crash
        // on the second iteration when the TLS pointed at an
        // already-destroyed nested-call subregion. Same prologue
        // every other String-returning stdlib primitive emits.
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[], "stdin.read_line.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("lotus_stdin_read_line returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// Lower `std::io::stdin::read_line_status() -> Int`. Returns
    /// the status of the most recent `read_line` call: 0 success,
    /// -1 EOF, -2 IO error, -3 OOM. The C shim returns i32; we
    /// sign-extend to i64 to match Hale's Int ABI.
    fn lower_std_io_stdin_read_line_status(
        &mut self,
        args: &[Expr],
        _scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::io::stdin::read_line_status takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_stdin_read_line_status")
            .expect("lotus_stdin_read_line_status declared");
        let call = self
            .builder
            .build_call(f, &[], "stdin.read_line_status.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i32_v = call
            .try_as_basic_value()
            .left()
            .expect("lotus_stdin_read_line_status returns i32")
            .into_int_value();
        let i64_t = self.context.i64_type();
        let widened = self
            .builder
            .build_int_s_extend(i32_v, i64_t, "stdin.status.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((widened.into(), CodegenTy::Int))
    }
}
