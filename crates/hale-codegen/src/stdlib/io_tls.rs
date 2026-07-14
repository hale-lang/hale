//! `std::io::tls::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait IoTlsStdlib<'ctx> {
    fn lower_std_io_tls_connect_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_tls_upgrade_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_tls_send_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tls_recv_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tls_close(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tls_recv_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> IoTlsStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::io::tls::connect(host, port) -> Int fallible(IoError)`.
    /// Same fallible shape as `tcp::connect` — the C primitive
    /// opens a TCP socket, wraps in SSL, performs the TLS
    /// handshake, returns an opaque handle (>=0) or -1 on error.
    fn lower_std_io_tls_connect_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::connect takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::connect: host must be String, got {:?}",
                host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::connect: port must be Int, got {:?}",
                port_ty
            )));
        }
        let i16_t = self.context.i16_type();
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let port_i16 = self
            .builder
            .build_int_truncate(port_val.into_int_value(), i16_t, "tls.port.i16")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tls_connect")
            .expect("lotus_tls_connect declared");
        let h_i32 = self
            .builder
            .build_call(
                f,
                &[host_val.into(), port_i16.into()],
                "tls.connect.handle",
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
                h_i32,
                i32_t.const_zero(),
                "tls.connect.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let h_i64 = self
            .builder
            .build_int_s_extend(h_i32, i64_t, "tls.connect.handle.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            host_val,
            Some((h_i64.into(), CodegenTy::Int)),
            "tls.connect",
        )
    }

    /// `std::io::tls::upgrade(fd: Int, host: String, verify: Bool)
    /// -> Int fallible(IoError)`. Wraps an already-connected TCP fd in
    /// a client TLS session (STARTTLS-style). Same fallible shape and
    /// IoError path convention as `connect` (path anchor = host); the
    /// C primitive returns the opaque handle (>=0) or -1 on error. The
    /// fd-ownership asymmetry (upgrade does NOT close the fd on
    /// failure — the caller owns it) lives in the C runtime, not here.
    fn lower_std_io_tls_upgrade_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::upgrade takes 3 args (fd, host, verify), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::upgrade: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::upgrade: host must be String, got {:?}",
                host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (verify_val, verify_ty) = self.lower_expr(&args[2], scope)?;
        if verify_ty != CodegenTy::Bool {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::upgrade: verify must be Bool, got {:?}",
                verify_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "tls.up.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let verify_i32 = self
            .builder
            .build_int_z_extend(
                verify_val.into_int_value(),
                i32_t,
                "tls.up.verify.zext",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tls_upgrade")
            .expect("lotus_tls_upgrade declared");
        let h_i32 = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), host_val.into(), verify_i32.into()],
                "tls.upgrade.handle",
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
                h_i32,
                i32_t.const_zero(),
                "tls.upgrade.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let h_i64 = self
            .builder
            .build_int_s_extend(h_i32, i64_t, "tls.upgrade.handle.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            host_val,
            Some((h_i64.into(), CodegenTy::Int)),
            "tls.upgrade",
        )
    }

    /// `std::io::tls::send_bytes(handle: Int, bytes: Bytes) -> Int`.
    /// Non-fallible at the language level: returns 0 on success or
    /// -1 on error. Mirrors tcp::__send_bytes.
    fn lower_std_io_tls_send_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::send_bytes takes 2 args (handle, bytes), got {}",
                args.len()
            )));
        }
        let (h_val, h_ty) = self.lower_expr(&args[0], scope)?;
        if h_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::send_bytes: handle must be Int, got {:?}",
                h_ty
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::send_bytes: bytes must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let h_i32 = self
            .builder
            .build_int_truncate(h_val.into_int_value(), i32_t, "tls.sb.h.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tls_send_bytes")
            .expect("lotus_tls_send_bytes declared");
        let ret_i32 = self
            .builder
            .build_call(f, &[h_i32.into(), b_val.into()], "tls.send_bytes.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "tls.send_bytes.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// `std::io::tls::recv_bytes(handle: Int, max: Int) -> Bytes`.
    /// Non-fallible: returns up to max bytes on success, or an
    /// empty Bytes on error / peer-closed. Mirrors tcp::__recv_bytes.
    fn lower_std_io_tls_recv_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::recv_bytes takes 2 args (handle, max), got {}",
                args.len()
            )));
        }
        let (h_val, h_ty) = self.lower_expr(&args[0], scope)?;
        if h_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::recv_bytes: handle must be Int, got {:?}",
                h_ty
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[1], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::recv_bytes: max must be Int, got {:?}",
                max_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let h_i32 = self
            .builder
            .build_int_truncate(h_val.into_int_value(), i32_t, "tls.rb.h.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let max_i32 = self
            .builder
            .build_int_truncate(max_val.into_int_value(), i32_t, "tls.rb.max.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tls_recv_bytes")
            .expect("lotus_tls_recv_bytes declared");
        let ptr = self
            .builder
            .build_call(f, &[h_i32.into(), max_i32.into()], "tls.rb.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// `std::io::tls::close(handle: Int) -> Int`. Non-fallible:
    /// returns 0 on success, -1 on error (bad handle). Mirrors
    /// tcp::close_fd.
    fn lower_std_io_tls_close(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::close takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (h_val, h_ty) = self.lower_expr(&args[0], scope)?;
        if h_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tls::close: handle must be Int, got {:?}",
                h_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let h_i32 = self
            .builder
            .build_int_truncate(h_val.into_int_value(), i32_t, "tls.close.h.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tls_close")
            .expect("lotus_tls_close declared");
        let ret_i32 = self
            .builder
            .build_call(f, &[h_i32.into()], "tls.close.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "tls.close.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Phase 1: lower `std::io::tls::recv_into(handle: Int,
    /// buf: Bytes, max_bytes: Int) -> Int`. SSL_read into the
    /// builder's tail. Same return semantics as tcp_recv_into.
    fn lower_std_io_tls_recv_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        self.lower_recv_into_common(
            args,
            scope,
            "lotus_tls_recv_into",
            "std::io::tls::recv_into",
        )
    }

}
