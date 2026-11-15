//! `std::io::file::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait IoFileStdlib<'ctx> {
    fn lower_std_io_file_open_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_file_write_bytes_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_file_seek_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_file_read_line(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_file_close(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_file_at_eof(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> IoFileStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::io::file::__open(path: String, mode: String) ->
    /// Int fallible(IoError)`. Returns the held fd as Int.
    /// IoError.path is anchored to the input `path`.
    fn lower_std_io_file_open_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__open takes 2 args (path, mode), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__open: path must be String, got {}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (mode_val, mode_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(mode_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__open: mode must be String, got {}",
                mode_ty
            )));
        }
        let mode_val = self.unpack_view_if_needed(mode_val, &mode_ty)?;
        let f = self
            .module
            .get_function("lotus_file_open")
            .expect("lotus_file_open declared");
        let fd_i32 = self
            .builder
            .build_call(f, &[path_val.into(), mode_val.into()], "file.open.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                fd_i32,
                self.context.i32_type().const_zero(),
                "file.open.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i64 = self
            .builder
            .build_int_s_extend(
                fd_i32,
                self.context.i64_type(),
                "file.open.fd.i64",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((fd_i64.into(), CodegenTy::Int)),
            "file.open",
        )
    }

    /// `std::io::file::__write_bytes(fd: Int, b: Bytes) -> ()
    /// fallible(IoError)`. Writes the full Bytes payload.
    fn lower_std_io_file_write_bytes_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__write_bytes takes 2 args (fd, bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__write_bytes: fd must be Int, got {}",
                fd_ty
            )));
        }
        let (bytes_val, bytes_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(bytes_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__write_bytes: bytes must be Bytes, got {}",
                bytes_ty
            )));
        }
        let bytes_val = self.unpack_view_if_needed(bytes_val, &bytes_ty)?;
        // Bytes ABI: ptr → [i64 len][u8 data[len]]. Decode the
        // length prefix then call lotus_file_write_all on the
        // body pointer.
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let bytes_ptr = bytes_val.into_pointer_value();
        let len = self
            .builder
            .build_load(i64_t, bytes_ptr, "bytes.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let body_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    bytes_ptr,
                    &[i64_t.const_int(8, false)],
                    "bytes.body",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let _ = ptr_t;
        let f = self
            .module
            .get_function("lotus_file_write_all")
            .expect("lotus_file_write_all declared");
        let ret = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), body_ptr.into(), len.into()],
                "file.write.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                ret,
                i32_t.const_zero(),
                "file.write.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Anchor IoError.path to a static "fd" label since the
        // caller's path string isn't available here.
        let label_ptr = self
            .builder
            .build_global_string_ptr("std::io::file::write_bytes", "file.write.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            label_ptr.into(),
            None,
            "file.write_bytes",
        )
    }

    /// `std::io::file::__seek(fd: Int, offset: Int) -> ()
    /// fallible(IoError)`.
    fn lower_std_io_file_seek_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__seek takes 2 args (fd, offset), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__seek: fd must be Int, got {}",
                fd_ty
            )));
        }
        let (off_val, off_ty) = self.lower_expr(&args[1], scope)?;
        if off_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__seek: offset must be Int, got {}",
                off_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_file_seek")
            .expect("lotus_file_seek declared");
        let ret = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), off_val.into()],
                "file.seek.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                ret,
                i32_t.const_zero(),
                "file.seek.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let label_ptr = self
            .builder
            .build_global_string_ptr("std::io::file::seek", "file.seek.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(is_err, label_ptr.into(), None, "file.seek")
    }

    /// `std::io::file::__read_line(fd: Int) -> String`. Returns
    /// the next '\n'-terminated line (newline included if
    /// present), or "" at EOF / on read error. Caller's at_eof
    /// loop disambiguates EOF from a real empty line.
    fn lower_std_io_file_read_line(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__read_line takes 1 arg (fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__read_line: fd must be Int, got {}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_file_read_line_global")
            .expect("lotus_file_read_line_global declared");
        // F.8 sweep — see lower_std_str_builder_finish for rationale.
        self.emit_set_caller_arena()?;
        let s_ptr = self
            .builder
            .build_call(f, &[fd_i32.into()], "file.read_line.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        Ok((s_ptr.into(), CodegenTy::String))
    }

    /// `std::io::file::__close(fd: Int) -> Int`. Non-fallible
    /// — the File locus's dissolve() best-effort-closes; errors
    /// from close are rare and not actionable for the caller in
    /// the dissolution path. Returns the close() ret (0 or -1).
    fn lower_std_io_file_close(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__close takes 1 arg (fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__close: fd must be Int, got {}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_file_close")
            .expect("lotus_file_close declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into()], "file.close.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "file.close.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// `std::io::file::__at_eof(fd: Int) -> Bool`. Returns true
    /// (i1=1) when the fd has no more bytes to read. Non-fallible
    /// at this surface; under-the-hood errors collapse to true
    /// (treat-as-EOF) so callers driving a read-line loop don't
    /// hang on a malfunctioning fd.
    fn lower_std_io_file_at_eof(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__at_eof takes 1 arg (fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::file::__at_eof: fd must be Int, got {}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let bool_t = self.context.bool_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_file_at_eof")
            .expect("lotus_file_at_eof declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into()], "file.eof.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        // Map: 1 => true; 0 or -1 => false (-1 path collapses
        // error to "not at EOF" so the caller keeps reading and
        // surfaces the error on the next read_line call where
        // it's actionable).
        let one = i32_t.const_int(1, false);
        let is_eof = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                ret_i32,
                one,
                "file.eof.is_eof",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let as_bool = self
            .builder
            .build_int_z_extend_or_bit_cast(is_eof, bool_t, "file.eof.bool")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((as_bool.into(), CodegenTy::Bool))
    }

}
