//! `std::io::fs::*` path-call lowering. 20 fns total — 10 fallible
//! (read_file, read_bytes, write_file, write_file_append, file_size,
//! mkdir, rename, unlink, mktemp, list_dir_count, list_dir_at) plus
//! 8 non-fallible legacy variants and the path predicates
//! `file_exists` / `extension`.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait IoFsStdlib<'ctx> {
    fn lower_std_io_fs_read_file_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_read_bytes_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_write_file_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_fn_name: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_io_fs_write_bytes_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_file_size_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_mkdir_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_rename_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_unlink_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_mktemp_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_list_dir_count_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_list_dir_at_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_fs_read_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_read_file(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_write_file(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_write_file_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_mkdir(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_file_size(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_file_exists(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_extension(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_list_dir_count(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_fs_list_dir_at(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> IoFsStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::io::fs::read_file(path) -> String fallible(IoError)`.
    /// 2026-05-21: routes through `lotus_fs_read_file_growing`,
    /// which doesn't trust fstat for sizing. For synthesized
    /// files (`/proc/*`, `/sys/*`, FIFO pipes) the previous
    /// fstat-then-read pattern returned an empty String because
    /// `st_size = 0` — surfaced by an attempt to read
    /// `/proc/self/statm` for process introspection. The
    /// growing-buffer variant reads into a doubling buffer
    /// (4 KiB → 64 MiB cap) and returns a NUL-terminated String
    /// anchored in the caller's arena. NULL return → IoError
    /// via the standard `complete_io_fallible_call` shape.
    fn lower_std_io_fs_read_file_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let arena = self.current_arena_ptr()?;
        let read_fn = self
            .module
            .get_function("lotus_fs_read_file_growing")
            .expect("lotus_fs_read_file_growing declared");
        let buf_ptr = self
            .builder
            .build_call(
                read_fn,
                &[arena.into(), path_val.into()],
                "fs.read_growing",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        // NULL → error.
        let i64_t = self.context.i64_type();
        let buf_as_int = self
            .builder
            .build_ptr_to_int(buf_ptr, i64_t, "buf.as_int")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                buf_as_int,
                i64_t.const_zero(),
                "read.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((buf_ptr.into(), CodegenTy::String)),
            "fs.read_file",
        )
    }

    /// `std::io::fs::read_bytes(path) -> Bytes fallible(IoError)`.
    fn lower_std_io_fs_read_bytes_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_bytes takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_bytes: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        // Global-arena wrapper — returns a pointer in the bus
        // payload arena so the value survives the call frame.
        let read_bytes_fn = self
            .module
            .get_function("lotus_fs_read_bytes_global")
            .expect("lotus_fs_read_bytes_global declared");
        let bytes_ptr = self
            .builder
            .build_call(
                read_bytes_fn,
                &[path_val.into()],
                "fs.read_bytes",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        // NULL pointer => error.
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                self.builder
                    .build_ptr_to_int(
                        bytes_ptr,
                        self.context.i64_type(),
                        "bytes.as_int",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
                self.context.i64_type().const_zero(),
                "read_bytes.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let _ = ptr_t;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((bytes_ptr.into(), CodegenTy::Bytes)),
            "fs.read_bytes",
        )
    }

    /// `std::io::fs::write_file{,append}(path, content) -> () fallible(IoError)`.
    fn lower_std_io_fs_write_file_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_fn_name: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::{} takes 2 args (path, content), got {}",
                c_fn_name.trim_start_matches("lotus_fs_"),
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "{}: path must be String, got {:?}",
                c_fn_name, path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (content_val, content_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(content_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "{}: content must be String, got {:?}",
                c_fn_name, content_ty
            )));
        }
        let content_val = self.unpack_view_if_needed(content_val, &content_ty)?;
        let len_fn = self
            .module
            .get_function("lotus_str_len")
            .expect("lotus_str_len declared");
        let len_v = self
            .builder
            .build_call(len_fn, &[content_val.into()], "wr.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        let write_fn = self
            .module
            .get_function(c_fn_name)
            .unwrap_or_else(|| panic!("{} declared", c_fn_name));
        let ret = self
            .builder
            .build_call(
                write_fn,
                &[path_val.into(), content_val.into(), len_v.into()],
                "wr.ret",
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
                self.context.i32_type().const_zero(),
                "wr.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(is_err, path_val, None, "fs.write_file")
    }

    /// GH #254 companion: `std::io::fs::write_bytes(path, b: Bytes)
    /// -> () fallible(IoError)` — the binary-safe write. Without it
    /// an in-memory archive (`std::tar` / `std::compress` output)
    /// could not be written to disk: `write_file` ships String
    /// content via strlen, truncating at the first NUL. Reuses
    /// `lotus_fs_write_file` (already buf+len at the C level) with
    /// the blob's data/len.
    fn lower_std_io_fs_write_bytes_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_bytes takes 2 args (path, b), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_bytes: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_bytes: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let data_fn = self
            .module
            .get_function("lotus_bytes_data")
            .expect("lotus_bytes_data declared");
        let len_fn = self
            .module
            .get_function("lotus_bytes_len")
            .expect("lotus_bytes_len declared");
        let data_v = self
            .builder
            .build_call(data_fn, &[b_val.into()], "wb.data")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        let len_v = self
            .builder
            .build_call(len_fn, &[b_val.into()], "wb.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        let write_fn = self
            .module
            .get_function("lotus_fs_write_file")
            .expect("lotus_fs_write_file declared");
        let ret = self
            .builder
            .build_call(
                write_fn,
                &[path_val.into(), data_v.into(), len_v.into()],
                "wb.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret,
                ret.get_type().const_zero(),
                "wb.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(is_err, path_val, None, "fs.write_bytes")
    }

    /// `std::io::fs::file_size(path) -> Int fallible(IoError)`.
    fn lower_std_io_fs_file_size_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_size takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_size: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let size_fn = self
            .module
            .get_function("lotus_fs_file_size")
            .expect("lotus_fs_file_size declared");
        let raw_size = self
            .builder
            .build_call(size_fn, &[path_val.into()], "fs.size")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                raw_size,
                self.context.i64_type().const_zero(),
                "size.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((raw_size.into(), CodegenTy::Int)),
            "fs.file_size",
        )
    }

    fn lower_std_io_fs_mkdir_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mkdir takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mkdir: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let mkdir_fn = self
            .module
            .get_function("lotus_fs_mkdir")
            .expect("lotus_fs_mkdir declared");
        let ret = self
            .builder
            .build_call(mkdir_fn, &[path_val.into()], "mkdir.ret")
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
                self.context.i32_type().const_zero(),
                "mkdir.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(is_err, path_val, None, "fs.mkdir")
    }

    /// C9: `std::io::fs::rename(src, dst) -> () fallible(IoError)`.
    /// Anchors the IoError.path to `dst` because the destination is
    /// the more diagnostic of the two on the common failure modes
    /// (target dir missing, target already a non-empty dir,
    /// cross-fs EXDEV).
    fn lower_std_io_fs_rename_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::rename takes 2 args (src, dst), got {}",
                args.len()
            )));
        }
        let (src_val, src_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(src_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::rename: src must be String, got {:?}",
                src_ty
            )));
        }
        let src_val = self.unpack_view_if_needed(src_val, &src_ty)?;
        let (dst_val, dst_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(dst_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::rename: dst must be String, got {:?}",
                dst_ty
            )));
        }
        let dst_val = self.unpack_view_if_needed(dst_val, &dst_ty)?;
        let rename_fn = self
            .module
            .get_function("lotus_fs_rename")
            .expect("lotus_fs_rename declared");
        let ret = self
            .builder
            .build_call(
                rename_fn,
                &[src_val.into(), dst_val.into()],
                "rename.ret",
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
                self.context.i32_type().const_zero(),
                "rename.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Diagnostic-path is the destination — see fn doc.
        self.complete_io_fallible_call(is_err, dst_val, None, "fs.rename")
    }

    /// C9: `std::io::fs::unlink(path) -> () fallible(IoError)`.
    fn lower_std_io_fs_unlink_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::unlink takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::unlink: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let unlink_fn = self
            .module
            .get_function("lotus_fs_unlink")
            .expect("lotus_fs_unlink declared");
        let ret = self
            .builder
            .build_call(unlink_fn, &[path_val.into()], "unlink.ret")
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
                self.context.i32_type().const_zero(),
                "unlink.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(is_err, path_val, None, "fs.unlink")
    }

    /// C9: `std::io::fs::mktemp(prefix, suffix) -> String fallible(IoError)`.
    /// Wraps mkstemps(3). Returns an arena-anchored path; caller
    /// owns cleanup. NULL pointer => error. The IoError.path field
    /// is the assembled `prefix + "XXXXXX" + suffix` template so
    /// agents can see which prefix/dir failed.
    fn lower_std_io_fs_mktemp_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mktemp takes 2 args (prefix, suffix), got {}",
                args.len()
            )));
        }
        let (prefix_val, prefix_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(prefix_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mktemp: prefix must be String, got {:?}",
                prefix_ty
            )));
        }
        let prefix_val = self.unpack_view_if_needed(prefix_val, &prefix_ty)?;
        let (suffix_val, suffix_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(suffix_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mktemp: suffix must be String, got {:?}",
                suffix_ty
            )));
        }
        let suffix_val = self.unpack_view_if_needed(suffix_val, &suffix_ty)?;
        // Compose the IoError.path string at call time:
        //   prefix + "XXXXXX" + suffix
        // The runtime mktemp builds the same shape; we reproduce
        // it here so the agent sees the template that failed
        // (not just the bare prefix or suffix). Anchored in the
        // current arena — only consumed on the err path, but
        // arena lifetime matches that of the surrounding fn so
        // it outlives any error-handler call site.
        let arena_ptr = self.current_arena_ptr()?;
        let xxx_ptr = self
            .builder
            .build_global_string_ptr("XXXXXX", "mktemp.xxx")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let concat_fn = self
            .module
            .get_function("lotus_str_concat")
            .expect("lotus_str_concat declared");
        let tmp1 = self
            .builder
            .build_call(
                concat_fn,
                &[arena_ptr.into(), prefix_val.into(), xxx_ptr.into()],
                "mktemp.tpl.lhs",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        let template_path = self
            .builder
            .build_call(
                concat_fn,
                &[arena_ptr.into(), tmp1.into(), suffix_val.into()],
                "mktemp.tpl",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        let mktemp_fn = self
            .module
            .get_function("lotus_fs_mktemp")
            .expect("lotus_fs_mktemp declared");
        let result_ptr = self
            .builder
            .build_call(
                mktemp_fn,
                &[prefix_val.into(), suffix_val.into()],
                "mktemp.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        // NULL => error.
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                self.builder
                    .build_ptr_to_int(
                        result_ptr,
                        self.context.i64_type(),
                        "mktemp.as_int",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
                self.context.i64_type().const_zero(),
                "mktemp.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            template_path,
            Some((result_ptr.into(), CodegenTy::String)),
            "fs.mktemp",
        )
    }

    /// `std::io::fs::list_dir_count(path) -> Int fallible(IoError)`.
    fn lower_std_io_fs_list_dir_count_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_count takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_count: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let exists_fn = self
            .module
            .get_function("lotus_fs_file_exists")
            .expect("lotus_fs_file_exists declared");
        let exists = self
            .builder
            .build_call(exists_fn, &[path_val.into()], "ldc.exists")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                exists,
                self.context.i32_type().const_zero(),
                "ldc.missing",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let count_fn = self
            .module
            .get_function("lotus_fs_list_dir_count")
            .expect("lotus_fs_list_dir_count declared");
        let count = self
            .builder
            .build_call(count_fn, &[path_val.into()], "ldc.body")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((count, CodegenTy::Int)),
            "fs.list_dir_count",
        )
    }

    /// `std::io::fs::list_dir_at(path, i) -> String fallible(IoError)`.
    fn lower_std_io_fs_list_dir_at_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at takes 2 args (path, i), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (idx_val, idx_ty) = self.lower_expr(&args[1], scope)?;
        if idx_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at: index must be Int, got {:?}",
                idx_ty
            )));
        }
        let at_fn = self
            .module
            .get_function("lotus_fs_list_dir_at")
            .expect("lotus_fs_list_dir_at declared");
        // F.8 sweep — see lower_std_str_builder_finish for rationale.
        self.emit_set_caller_arena()?;
        let s_ptr = self
            .builder
            .build_call(
                at_fn,
                &[path_val.into(), idx_val.into()],
                "lda.body",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        // NULL pointer => error (OOB or path issue).
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                self.builder
                    .build_ptr_to_int(
                        s_ptr,
                        self.context.i64_type(),
                        "lda.as_int",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
                self.context.i64_type().const_zero(),
                "lda.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((s_ptr.into(), CodegenTy::String)),
            "fs.list_dir_at",
        )
    }

    /// m89: Lower `std::io::fs::read_bytes(path: String) -> Bytes`.
    /// Routes to `lotus_fs_read_bytes_global` so the resulting
    /// Bytes blob lives in the lazy global payload arena (same
    /// lifetime story as read_file's String). Embedded NUL bytes
    /// are preserved because Bytes carries an explicit length
    /// prefix — the reason this exists alongside read_file.
    fn lower_std_io_fs_read_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_bytes takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_bytes: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let f = self
            .module
            .get_function("lotus_fs_read_bytes_global")
            .expect("lotus_fs_read_bytes_global declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.read_bytes.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("lotus_fs_read_bytes_global returns ptr");
        Ok((v, CodegenTy::Bytes))
    }

    /// Lower `std::io::fs::read_file(path: String) -> String`.
    /// Two-phase: stat the file to learn its size, allocate a
    /// (size+1)-byte buffer in the lazy global payload arena
    /// (so the resulting String outlives the call frame), then
    /// read into it and NUL-terminate at the actual bytes-read
    /// offset. If the file is missing or unreadable, both
    /// file_size and read_file return -1; we clamp to 0 and
    /// hand back an empty String. Callers that need to
    /// distinguish "empty file" from "missing file" use
    /// `std::io::fs::file_exists` first.
    fn lower_std_io_fs_read_file(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let i8_t = self.context.i8_type();
        let i64_t = self.context.i64_type();
        let zero64 = i64_t.const_zero();
        let one64 = i64_t.const_int(1, false);

        // 1. Get the file size.
        let size_fn = self
            .module
            .get_function("lotus_fs_file_size")
            .expect("lotus_fs_file_size declared");
        let size_call = self
            .builder
            .build_call(size_fn, &[path_val.into()], "fs.size")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let raw_size = size_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();

        // 2. Clamp negative size to 0 so the alloc/read paths
        //    proceed without a separate error branch.
        let is_neg = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                raw_size,
                zero64,
                "size.neg",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let safe_size = self
            .builder
            .build_select(is_neg, zero64, raw_size, "size.safe")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let alloc_size = self
            .builder
            .build_int_add(safe_size, one64, "alloc.size")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // 3. Allocate (size+1) bytes in the lazy global arena.
        let alloc_fn = self
            .module
            .get_function("lotus_bus_payload_arena_alloc")
            .expect("lotus_bus_payload_arena_alloc declared");
        let buf_call = self
            .builder
            .build_call(
                alloc_fn,
                &[alloc_size.into(), one64.into()],
                "fs.buf",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let buf_ptr = buf_call
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();

        // 4. Read into the buffer.
        let read_fn = self
            .module
            .get_function("lotus_fs_read_file")
            .expect("lotus_fs_read_file declared");
        let read_call = self
            .builder
            .build_call(
                read_fn,
                &[path_val.into(), buf_ptr.into(), safe_size.into()],
                "fs.read",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let raw_n = read_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();

        // 5. Clamp bytes-read to 0 for negative returns.
        let n_neg = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                raw_n,
                zero64,
                "n.neg",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let safe_n = self
            .builder
            .build_select(n_neg, zero64, raw_n, "n.safe")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();

        // 6. NUL-terminate at offset safe_n.
        let nul_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_t, buf_ptr, &[safe_n], "nul.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        self.builder
            .build_store(nul_ptr, i8_t.const_zero())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        Ok((buf_ptr.into(), CodegenTy::String))
    }

    /// Lower `std::io::fs::write_file(path: String, content:
    /// String) -> Int`. Returns 0 on success, -1 on error.
    /// Truncates any existing file. Length is computed from
    /// the content's String pointer via lotus_str_len (Hale
    /// Strings are NUL-terminated in memory).
    fn lower_std_io_fs_write_file(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file takes 2 args (path, content), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (content_val, content_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(content_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file: content must be String, got {:?}",
                content_ty
            )));
        }
        let content_val = self.unpack_view_if_needed(content_val, &content_ty)?;
        let i64_t = self.context.i64_type();

        // strlen(content) → i64 length on the wire.
        let len_fn = self
            .module
            .get_function("lotus_str_len")
            .expect("lotus_str_len declared");
        let len_call = self
            .builder
            .build_call(len_fn, &[content_val.into()], "wf.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let len_i64 = len_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();

        let write_fn = self
            .module
            .get_function("lotus_fs_write_file")
            .expect("lotus_fs_write_file declared");
        let write_call = self
            .builder
            .build_call(
                write_fn,
                &[path_val.into(), content_val.into(), len_i64.into()],
                "fs.write.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = write_call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "wf.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::write_file_append(path, content) -> Int`.
    /// Same shape as write_file but opens the file with O_APPEND
    /// instead of O_TRUNC. Returns 0 on success, -1 on error.
    /// Resolves the apps/log-router friction "no append primitive
    /// forces buffer-everything-then-flush at dissolve."
    fn lower_std_io_fs_write_file_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file_append takes 2 args (path, content), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file_append: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (content_val, content_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(content_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file_append: content must be String, got {:?}",
                content_ty
            )));
        }
        let content_val = self.unpack_view_if_needed(content_val, &content_ty)?;
        let i64_t = self.context.i64_type();
        let len_fn = self
            .module
            .get_function("lotus_str_len")
            .expect("lotus_str_len declared");
        let len_call = self
            .builder
            .build_call(len_fn, &[content_val.into()], "wfa.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let len_i64 = len_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let f = self
            .module
            .get_function("lotus_fs_write_file_append")
            .expect("lotus_fs_write_file_append declared");
        let call = self
            .builder
            .build_call(
                f,
                &[path_val.into(), content_val.into(), len_i64.into()],
                "fs.wfa.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "wfa.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::mkdir(path: String) -> Int`. Single-
    /// level only (NOT recursive). Returns 0 on success, -1 on
    /// error (errno set; EEXIST if the dir already exists).
    /// Resolves the apps/ssg friction "no mkdir / create_dir
    /// forces shell-out via README precondition."
    fn lower_std_io_fs_mkdir(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mkdir takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mkdir: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let i64_t = self.context.i64_type();
        let f = self
            .module
            .get_function("lotus_fs_mkdir")
            .expect("lotus_fs_mkdir declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.mkdir.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "mkdir.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::file_size(path: String) -> Int`.
    /// Returns the byte size or -1 on error.
    fn lower_std_io_fs_file_size(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_size takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_size: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let f = self
            .module
            .get_function("lotus_fs_file_size")
            .expect("lotus_fs_file_size declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.size.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let size_i64 = call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        Ok((size_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::file_exists(path: String) -> Bool`.
    /// Returns true if the path exists, false otherwise.
    fn lower_std_io_fs_file_exists(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_exists takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_exists: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let i32_t = self.context.i32_type();
        let i1_t = self.context.bool_type();
        let f = self
            .module
            .get_function("lotus_fs_file_exists")
            .expect("lotus_fs_file_exists declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.exists.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        // Truncate i32 0/1 to i1 for Hale Bool.
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "exists.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let _ = i1_t; // silence unused warning if any
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    /// Lower `std::io::fs::extension(path: String) -> String`.
    /// Returns the basename's last-dot suffix including the
    /// leading dot (".go", ".md"), or the empty string when
    /// there is no extension. Result lives in the global
    /// payload arena (same lifetime as list_dir / read_file).
    fn lower_std_io_fs_extension(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::extension takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::extension: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let f = self
            .module
            .get_function("lotus_fs_extension_global")
            .expect("lotus_fs_extension_global declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.extension.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("lotus_fs_extension_global returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// Phase 2e: lower `std::io::fs::list_dir_count(path: String)
    /// -> Int`. Returns the number of entries in `path` (skipping
    /// `.` / `..`), 0 on error or empty directory. Shares the
    /// global payload arena cache with `list_dir_at` so the
    /// directory read amortises across both calls.
    fn lower_std_io_fs_list_dir_count(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_count takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_count: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let f = self
            .module
            .get_function("lotus_fs_list_dir_count")
            .expect("lotus_fs_list_dir_count declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "ld.count.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((ret, CodegenTy::Int))
    }

    /// Phase 2e: lower `std::io::fs::list_dir_at(path: String,
    /// idx: Int) -> String`. Returns the `idx`-th entry name
    /// (0-indexed), or the empty string if out of range. Shares
    /// the global payload arena cache with `list_dir_count`.
    fn lower_std_io_fs_list_dir_at(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at takes 2 args (path, idx), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (idx_val, idx_ty) = self.lower_expr(&args[1], scope)?;
        if idx_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at: idx must be Int, got {:?}",
                idx_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_fs_list_dir_at")
            .expect("lotus_fs_list_dir_at declared");
        let call = self
            .builder
            .build_call(
                f,
                &[path_val.into(), idx_val.into()],
                "ld.at.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

}
