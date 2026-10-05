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
        runtime: &str,
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
    ///
    /// `std::io::fs::__write_private(path, b)` lowers here too, through
    /// `lotus_fs_write_private`: the same buf+len write into a new 0600
    /// file in a directory the process owns (`std::secret`'s
    /// `Credential.write_private`).
    fn lower_std_io_fs_write_bytes_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        runtime: &str,
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
            .get_function(runtime)
            .expect("fs write runtime declared");
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

}
