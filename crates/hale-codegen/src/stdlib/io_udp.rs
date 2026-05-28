//! `std::io::udp::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait IoUdpStdlib<'ctx> {
    fn lower_std_io_udp_bind_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_send_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_recv_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_recv_with_source_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_set_timeout_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_last_source_host(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_udp_last_source_port(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_udp_join_group_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_leave_group_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_udp_group_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_set_multicast_ttl_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_set_multicast_loop_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_udp_set_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_set_multicast_iface_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_set_option_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_set_option_bool_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_udp_setsockopt_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
        value_is_bool: bool,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_get_option_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_udp_recv_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_udp_close(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> IoUdpStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::io::fs::mkdir(path) -> () fallible(IoError)`.
    /// `std::io::udp::__bind(host: String, port: Int) -> Int
    /// fallible(IoError)`. Creates a UDP socket bound to
    /// (host, port). host="0.0.0.0" or "" → INADDR_ANY.
    fn lower_std_io_udp_bind_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__bind takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__bind: host must be String, got {:?}",
                host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__bind: port must be Int, got {:?}",
                port_ty
            )));
        }
        let i16_t = self.context.i16_type();
        let port_i16 = self
            .builder
            .build_int_truncate(port_val.into_int_value(), i16_t, "udp.port.i16")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_udp_bind")
            .expect("lotus_udp_bind declared");
        let fd_i32 = self
            .builder
            .build_call(f, &[host_val.into(), port_i16.into()], "udp.bind.fd")
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
                "udp.bind.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i64 = self
            .builder
            .build_int_s_extend(
                fd_i32,
                self.context.i64_type(),
                "udp.bind.fd.i64",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            host_val,
            Some((fd_i64.into(), CodegenTy::Int)),
            "udp.bind",
        )
    }

    /// `std::io::udp::__send(fd: Int, host: String, port: Int,
    /// msg: String) -> () fallible(IoError)`. Sends one
    /// datagram. Best-effort delivery per UDP semantics.
    fn lower_std_io_udp_send_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 4 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__send takes 4 args (fd, host, port, msg), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__send: fd must be Int, got {:?}", fd_ty
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__send: host must be String, got {:?}", host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (port_val, port_ty) = self.lower_expr(&args[2], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__send: port must be Int, got {:?}", port_ty
            )));
        }
        let (msg_val, msg_ty) = self.lower_expr(&args[3], scope)?;
        if !matches!(msg_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__send: msg must be String, got {:?}", msg_ty
            )));
        }
        let msg_val = self.unpack_view_if_needed(msg_val, &msg_ty)?;
        let i16_t = self.context.i16_type();
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let port_i16 = self
            .builder
            .build_int_truncate(port_val.into_int_value(), i16_t, "udp.port.i16")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_udp_sendto_str")
            .expect("lotus_udp_sendto_str declared");
        let ret = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), host_val.into(), port_i16.into(), msg_val.into()],
                "udp.send.ret",
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
                "udp.send.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(is_err, host_val, None, "udp.send")
    }

    /// `std::io::udp::__recv(fd: Int, max_bytes: Int) -> Bytes
    /// fallible(IoError)`. Receives one datagram.
    fn lower_std_io_udp_recv_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__recv takes 2 args (fd, max_bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__recv: fd must be Int, got {:?}", fd_ty
            )));
        }
        let (cap_val, cap_ty) = self.lower_expr(&args[1], scope)?;
        if cap_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__recv: max_bytes must be Int, got {:?}", cap_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let cap_i32 = self
            .builder
            .build_int_truncate(cap_val.into_int_value(), i32_t, "udp.cap.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_udp_recv_bytes_global")
            .expect("lotus_udp_recv_bytes_global declared");
        // F.8 sweep — see lower_std_str_builder_finish for rationale.
        self.emit_set_caller_arena()?;
        let blob_ptr = self
            .builder
            .build_call(f, &[fd_i32.into(), cap_i32.into()], "udp.recv.blob")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                self.builder
                    .build_ptr_to_int(
                        blob_ptr,
                        self.context.i64_type(),
                        "udp.recv.as_int",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
                self.context.i64_type().const_zero(),
                "udp.recv.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let label_ptr = self
            .builder
            .build_global_string_ptr("std::io::udp::recv", "udp.recv.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            label_ptr.into(),
            Some((blob_ptr.into(), CodegenTy::Bytes)),
            "udp.recv",
        )
    }

    /// 2026-05-26 — UDP P4 `recv_with_source(fd: Int,
    /// max_bytes: Int) -> Bytes fallible(IoError)`. Like `recv`
    /// but ALSO captures the sender's IP + port into thread-
    /// local storage; callers read them via
    /// `std::io::udp::last_source_host()` / `last_source_port()`.
    /// Same Bytes ABI as `recv`.
    fn lower_std_io_udp_recv_with_source_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::recv_with_source takes 2 args \
                 (fd, max_bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::recv_with_source: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[1], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::recv_with_source: max_bytes must be Int, got {:?}",
                max_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "udp.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let max_i32 = self
            .builder
            .build_int_truncate(max_val.into_int_value(), i32_t, "udp.max.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_udp_recv_bytes_with_source")
            .expect("lotus_udp_recv_bytes_with_source declared");
        let blob_ptr = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), max_i32.into()],
                "udp.recv_with_source.blob",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let is_err = self
            .builder
            .build_is_null(blob_ptr, "udp.recv_with_source.is_err")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let label_ptr = self.global_string("udp.recv_with_source");
        let _ = ptr_t;
        self.complete_io_fallible_call(
            is_err,
            label_ptr.into(),
            Some((blob_ptr.into(), CodegenTy::Bytes)),
            "udp.recv_with_source",
        )
    }

    /// 2026-05-26 — UDP P4 `set_recv_timeout(fd: Int,
    /// d: Duration) -> () fallible(IoError)` and its
    /// `set_send_timeout` sibling. Both take a Duration (i64
    /// nanoseconds at the ABI level) and convert to struct
    /// timeval inside the C primitive. d == 0 means "no
    /// timeout" (the default; blocking).
    fn lower_std_io_udp_set_timeout_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{} takes 2 args (fd, d), got {}",
                label, args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: fd must be Int, got {:?}",
                label, fd_ty
            )));
        }
        let (d_val, d_ty) = self.lower_expr(&args[1], scope)?;
        if d_ty != CodegenTy::Duration {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: d must be Duration, got {:?}",
                label, d_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "udp.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function(c_name)
            .expect("lotus_udp_set_*_timeout_ns declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), d_val.into()],
                &format!("udp.{}.ret", label),
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
                ret_i32,
                i32_t.const_zero(),
                &format!("udp.{}.is_err", label),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let path_anchor = self.global_string(label);
        self.complete_io_fallible_call(
            is_err,
            path_anchor.into(),
            None,
            &format!("udp.{}", label),
        )
    }

    /// 2026-05-26 — `std::io::udp::last_source_host() -> String`.
    /// Reads the thread-local source-IP cache populated by the
    /// last `recv_with_source` call. Returns "" if no
    /// recv_with_source has run on this thread yet.
    fn lower_std_io_udp_last_source_host(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::last_source_host takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_udp_last_source_host")
            .expect("lotus_udp_last_source_host declared");
        let v = self
            .builder
            .build_call(f, &[], "udp.last_source_host.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// 2026-05-26 — `std::io::udp::last_source_port() -> Int`.
    /// Reads the thread-local source-port cache.
    fn lower_std_io_udp_last_source_port(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::last_source_port takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_udp_last_source_port")
            .expect("lotus_udp_last_source_port declared");
        let v = self
            .builder
            .build_call(f, &[], "udp.last_source_port.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// 2026-05-26 — UDP multicast `join_group(fd: Int,
    /// group: String, iface: String) -> () fallible(IoError)`.
    /// iface = "" means INADDR_ANY (kernel picks).
    fn lower_std_io_udp_join_group_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_udp_group_fallible(
            args, scope,
            "lotus_udp_join_group",
            "join_group",
        )
    }

    fn lower_std_io_udp_leave_group_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_udp_group_fallible(
            args, scope,
            "lotus_udp_leave_group",
            "leave_group",
        )
    }

    /// Shared body for join_group / leave_group — both have the
    /// (fd, group, iface) signature and call the same shape of
    /// `setsockopt` underneath.
    fn lower_udp_group_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{} takes 3 args (fd, group, iface), got {}",
                label, args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: fd must be Int, got {:?}",
                label, fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "udp.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let (group_val, group_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(group_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: group must be String, got {:?}",
                label, group_ty
            )));
        }
        let group_val = self.unpack_view_if_needed(group_val, &group_ty)?;
        let (iface_val, iface_ty) = self.lower_expr(&args[2], scope)?;
        if !matches!(iface_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: iface must be String, got {:?}",
                label, iface_ty
            )));
        }
        let iface_val = self.unpack_view_if_needed(iface_val, &iface_ty)?;
        let f = self
            .module
            .get_function(c_name)
            .expect("lotus_udp_*_group declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), group_val.into(), iface_val.into()],
                &format!("udp.{}.ret", label),
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
                ret_i32,
                i32_t.const_zero(),
                &format!("udp.{}.is_err", label),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            group_val,
            None,
            &format!("udp.{}", label),
        )
    }

    /// 2026-05-26 — `set_multicast_ttl(fd: Int, ttl: Int) -> ()
    /// fallible(IoError)`. ttl in 0..255; out-of-range surfaces
    /// EINVAL via IoError.
    fn lower_std_io_udp_set_multicast_ttl_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_udp_set_int_fallible(
            args, scope,
            "lotus_udp_set_multicast_ttl",
            "set_multicast_ttl",
        )
    }

    /// 2026-05-26 — `set_multicast_loop(fd: Int, enabled: Bool)
    /// -> () fallible(IoError)`. Whether the sender receives
    /// its own multicast packets.
    fn lower_std_io_udp_set_multicast_loop_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_udp_set_int_fallible(
            args, scope,
            "lotus_udp_set_multicast_loop",
            "set_multicast_loop",
        )
    }

    /// Shared body for the (fd: Int, value: Int|Bool) shape.
    /// Both Int and Bool arg types are accepted at this layer —
    /// the C primitive is the same in both cases (the value is
    /// coerced to int).
    fn lower_udp_set_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{} takes 2 args (fd, value), got {}",
                label, args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: fd must be Int, got {:?}",
                label, fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "udp.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let (val_val, val_ty) = self.lower_expr(&args[1], scope)?;
        let val_i32 = match val_ty {
            CodegenTy::Int => self
                .builder
                .build_int_truncate(val_val.into_int_value(), i32_t, "udp.val.i32")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
            CodegenTy::Bool => self
                .builder
                .build_int_z_extend(
                    val_val.into_int_value(),
                    i32_t,
                    "udp.val.zext",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "std::io::udp::{}: value must be Int or Bool, got {:?}",
                    label, other
                )));
            }
        };
        let f = self
            .module
            .get_function(c_name)
            .expect("lotus_udp_set_* declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), val_i32.into()],
                &format!("udp.{}.ret", label),
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
                ret_i32,
                i32_t.const_zero(),
                &format!("udp.{}.is_err", label),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // IoError.path field gets the operation name as a
        // diagnostic anchor (no real path for setsockopt knobs).
        let path_anchor = self.global_string(label);
        self.complete_io_fallible_call(
            is_err,
            path_anchor.into(),
            None,
            &format!("udp.{}", label),
        )
    }

    /// 2026-05-26 — `set_multicast_iface(fd: Int, addr: String)
    /// -> () fallible(IoError)`. addr = "" means INADDR_ANY.
    fn lower_std_io_udp_set_multicast_iface_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::set_multicast_iface takes 2 args \
                 (fd, addr), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::set_multicast_iface: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "udp.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let (addr_val, addr_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(addr_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::set_multicast_iface: addr must be String, got {:?}",
                addr_ty
            )));
        }
        let addr_val = self.unpack_view_if_needed(addr_val, &addr_ty)?;
        let f = self
            .module
            .get_function("lotus_udp_set_multicast_iface")
            .expect("lotus_udp_set_multicast_iface declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), addr_val.into()],
                "udp.set_multicast_iface.ret",
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
                ret_i32,
                i32_t.const_zero(),
                "udp.set_multicast_iface.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            addr_val,
            None,
            "udp.set_multicast_iface",
        )
    }

    /// 2026-05-26 — `set_option_int(fd: Int, level: Int,
    /// name: Int, value: Int) -> () fallible(IoError)`. Raw
    /// setsockopt pass-through; level/name come from
    /// std::io::sockopt's named Int constants.
    fn lower_std_io_udp_set_option_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_udp_setsockopt_fallible(
            args, scope,
            "lotus_udp_setsockopt_int",
            "set_option_int",
            /* value_is_bool */ false,
        )
    }

    /// 2026-05-26 — `set_option_bool(fd: Int, level: Int,
    /// name: Int, enabled: Bool) -> () fallible(IoError)`.
    fn lower_std_io_udp_set_option_bool_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_udp_setsockopt_fallible(
            args, scope,
            "lotus_udp_setsockopt_bool",
            "set_option_bool",
            /* value_is_bool */ true,
        )
    }

    /// Shared body for set_option_int / set_option_bool. Both
    /// take (fd, level, name, value) — only the C function
    /// chosen and the value's expected type differ.
    fn lower_udp_setsockopt_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        c_name: &str,
        label: &str,
        value_is_bool: bool,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 4 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{} takes 4 args (fd, level, name, value), got {}",
                label, args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: fd must be Int, got {:?}",
                label, fd_ty
            )));
        }
        let (level_val, level_ty) = self.lower_expr(&args[1], scope)?;
        if level_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: level must be Int, got {:?}",
                label, level_ty
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[2], scope)?;
        if name_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: name must be Int, got {:?}",
                label, name_ty
            )));
        }
        let (val_val, val_ty) = self.lower_expr(&args[3], scope)?;
        let expected_val_ty = if value_is_bool {
            CodegenTy::Bool
        } else {
            CodegenTy::Int
        };
        if val_ty != expected_val_ty {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::{}: value must be {:?}, got {:?}",
                label, expected_val_ty, val_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "udp.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let level_i32 = self
            .builder
            .build_int_truncate(level_val.into_int_value(), i32_t, "udp.level.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let name_i32 = self
            .builder
            .build_int_truncate(name_val.into_int_value(), i32_t, "udp.name.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let val_i32 = if value_is_bool {
            self.builder
                .build_int_z_extend(
                    val_val.into_int_value(),
                    i32_t,
                    "udp.val.zext",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        } else {
            self.builder
                .build_int_truncate(val_val.into_int_value(), i32_t, "udp.val.i32")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let f = self
            .module
            .get_function(c_name)
            .expect("lotus_udp_setsockopt_* declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[
                    fd_i32.into(),
                    level_i32.into(),
                    name_i32.into(),
                    val_i32.into(),
                ],
                &format!("udp.{}.ret", label),
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
                ret_i32,
                i32_t.const_zero(),
                &format!("udp.{}.is_err", label),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let path_anchor = self.global_string(label);
        self.complete_io_fallible_call(
            is_err,
            path_anchor.into(),
            None,
            &format!("udp.{}", label),
        )
    }

    /// 2026-05-26 — `get_option_int(fd: Int, level: Int,
    /// name: Int) -> Int fallible(IoError)`. Sentinel on
    /// error: the C primitive returns INT_MIN.
    fn lower_std_io_udp_get_option_int_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::get_option_int takes 3 args \
                 (fd, level, name), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::get_option_int: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (level_val, level_ty) = self.lower_expr(&args[1], scope)?;
        if level_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::get_option_int: level must be Int, got {:?}",
                level_ty
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[2], scope)?;
        if name_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::get_option_int: name must be Int, got {:?}",
                name_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "udp.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let level_i32 = self
            .builder
            .build_int_truncate(level_val.into_int_value(), i32_t, "udp.level.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let name_i32 = self
            .builder
            .build_int_truncate(name_val.into_int_value(), i32_t, "udp.name.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_udp_getsockopt_int")
            .expect("lotus_udp_getsockopt_int declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), level_i32.into(), name_i32.into()],
                "udp.get_option_int.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let int_min = i32_t.const_int(0x80000000u64, false);
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                ret_i32,
                int_min,
                "udp.get_option_int.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i64 = self
            .builder
            .build_int_s_extend(
                ret_i32,
                self.context.i64_type(),
                "udp.get_option_int.i64",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let path_anchor = self.global_string("get_option_int");
        self.complete_io_fallible_call(
            is_err,
            path_anchor.into(),
            Some((ret_i64.into(), CodegenTy::Int)),
            "udp.get_option_int",
        )
    }

    /// Phase 1: lower `std::io::udp::recv_into(fd: Int, buf: Bytes,
    /// max_bytes: Int) -> Int`. Single recvfrom into the builder's
    /// tail (datagram boundaries preserved). Same return semantics
    /// as tcp_recv_into.
    fn lower_std_io_udp_recv_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        self.lower_recv_into_common(
            args,
            scope,
            "lotus_udp_recv_into",
            "std::io::udp::recv_into",
        )
    }

    /// `std::io::udp::__close(fd: Int) -> Int`. Mirrors
    /// `std::io::tcp::__close_fd` but routes to lotus_udp_close.
    fn lower_std_io_udp_close(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__close takes 1 arg (fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::udp::__close: fd must be Int, got {:?}",
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
            .get_function("lotus_udp_close")
            .expect("lotus_udp_close declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into()], "udp.close.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "udp.close.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

}
