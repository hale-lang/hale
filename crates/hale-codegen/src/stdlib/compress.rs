//! `std::compress::*` path-call lowering — GH #254: one-shot
//! compression over `Bytes`. Four fns, one shape: 1 Bytes arg,
//! Bytes success value, `fallible(IoError)` via the NULL-pointer
//! sentinel + errno (EINVAL = corrupt/truncated/unsupported
//! input → kind "invalid"; ENOENT = zstd only, libzstd not
//! installed → kind "not_found"; EFBIG = output exceeds the
//! 1 GiB one-shot guard). The C side lives in
//! runtime/lotus_compress.c (own TU, links -lz; zstd is
//! dlopen'd).

use hale_syntax::ast::Expr;
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait CompressStdlib<'ctx> {
    fn lower_std_compress_fallible(
        &mut self,
        c_fn: &str,
        surface: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
}

impl<'ctx, 'p> CompressStdlib<'ctx> for Cx<'ctx, 'p> {
    /// Shared lowering for `std::compress::{gzip,gunzip,zstd,
    /// unzstd}(b: Bytes) -> Bytes fallible(IoError)`.
    fn lower_std_compress_fallible(
        &mut self,
        c_fn: &str,
        surface: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "{} takes 1 arg (b: Bytes), got {}",
                surface,
                args.len()
            )));
        }
        let (src_val, src_ty) = self.lower_expr(&args[0], scope)?;
        if src_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "{}: arg must be Bytes, got {:?} \
                 (use `std::bytes::from_string(s)` to convert)",
                surface, src_ty
            )));
        }
        let f = self
            .module
            .get_function(c_fn)
            .unwrap_or_else(|| panic!("{} declared", c_fn));
        let out_ptr = self
            .builder
            .build_call(f, &[src_val.into()], "compress.op")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        // NULL => error; the C side set errno so the IoError kind
        // resolves through the shared errno labeler.
        let is_err = self
            .builder
            .build_is_null(out_ptr, "compress.is_err")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let _ = ptr_t;
        let path_label = self
            .builder
            .build_global_string_ptr(surface, "compress.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((out_ptr.into(), CodegenTy::Bytes)),
            "compress",
        )
    }
}

/// Argument/return shapes for the `std::tar` family — small
/// enough to table-drive one shared lowering.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum TarArg {
    Bytes,
    Str,
    Int,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum TarRet {
    Bytes,
    Str,
    Int,
}

pub(crate) trait TarStdlib<'ctx> {
    fn lower_std_tar_fallible(
        &mut self,
        c_fn: &str,
        surface: &str,
        arg_spec: &[TarArg],
        ret: TarRet,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
}

impl<'ctx, 'p> TarStdlib<'ctx> for Cx<'ctx, 'p> {
    /// Shared lowering for `std::tar::*` (GH #254): typed args per
    /// `arg_spec`, C call, then the error sentinel — NULL for
    /// pointer returns, `< 0` for Int returns — routes through
    /// `complete_io_fallible_call` (errno set C-side; EINVAL =
    /// malformed archive / bad index → kind "invalid").
    fn lower_std_tar_fallible(
        &mut self,
        c_fn: &str,
        surface: &str,
        arg_spec: &[TarArg],
        ret: TarRet,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != arg_spec.len() {
            return Err(CodegenError::Unsupported(format!(
                "{} takes {} arg(s), got {}",
                surface,
                arg_spec.len(),
                args.len()
            )));
        }
        let mut lowered = Vec::with_capacity(args.len());
        for (i, (a, spec)) in args.iter().zip(arg_spec).enumerate() {
            let (v, ty) = self.lower_expr(a, scope)?;
            let v = match spec {
                TarArg::Bytes => {
                    if ty != CodegenTy::Bytes {
                        return Err(CodegenError::Unsupported(format!(
                            "{}: arg {} must be Bytes, got {:?}",
                            surface,
                            i + 1,
                            ty
                        )));
                    }
                    v
                }
                TarArg::Str => {
                    if !matches!(
                        ty,
                        CodegenTy::String | CodegenTy::StringView
                    ) {
                        return Err(CodegenError::Unsupported(format!(
                            "{}: arg {} must be String, got {:?}",
                            surface,
                            i + 1,
                            ty
                        )));
                    }
                    self.unpack_view_if_needed(v, &ty)?
                }
                TarArg::Int => {
                    if ty != CodegenTy::Int {
                        return Err(CodegenError::Unsupported(format!(
                            "{}: arg {} must be Int, got {:?}",
                            surface,
                            i + 1,
                            ty
                        )));
                    }
                    v
                }
            };
            lowered.push(v.into());
        }
        let f = self
            .module
            .get_function(c_fn)
            .unwrap_or_else(|| panic!("{} declared", c_fn));
        let call = self
            .builder
            .build_call(f, &lowered, "tar.op")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns a value");
        let (is_err, success) = match ret {
            TarRet::Bytes | TarRet::Str => {
                let p = call.into_pointer_value();
                let is_err = self
                    .builder
                    .build_is_null(p, "tar.is_err")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let ty = if ret == TarRet::Bytes {
                    CodegenTy::Bytes
                } else {
                    CodegenTy::String
                };
                (is_err, (p.into(), ty))
            }
            TarRet::Int => {
                let v = call.into_int_value();
                let is_err = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLT,
                        v,
                        self.context.i64_type().const_zero(),
                        "tar.is_err",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                (is_err, (v.into(), CodegenTy::Int))
            }
        };
        let path_label = self
            .builder
            .build_global_string_ptr(surface, "tar.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some(success),
            "tar",
        )
    }
}
