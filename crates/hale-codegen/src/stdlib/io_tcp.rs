//! `std::io::tcp::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait IoTcpStdlib<'ctx> {
    fn lower_std_io_tcp_listen_socket_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_tcp_connect_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_tcp_accept_one_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_tcp_listen_socket(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_connect(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_accept_one(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_send_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_send(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_recv(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_recv_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_recv_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_recv_stamped_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_last_recv_ns(
        &mut self,
        args: &[Expr],
        c_fn: &str,
        label: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_close_fd(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_shutdown_listen_socket(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_io_tcp_set_recv_timeout(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> IoTcpStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::io::tcp::listen_socket(host, port) -> Int fallible(IoError)`.
    /// Path field of the IoError carries "host:port".
    fn lower_std_io_tcp_listen_socket_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::listen_socket takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::listen_socket: host must be String, got {:?}",
                host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::listen_socket: port must be Int, got {:?}",
                port_ty
            )));
        }
        let listen_fn = self
            .module
            .get_function("lotus_tcp_listen_socket")
            .expect("lotus_tcp_listen_socket declared");
        let port_i32 = self
            .builder
            .build_int_truncate(
                port_val.into_int_value(),
                self.context.i32_type(),
                "listen.port.i32",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i32 = self
            .builder
            .build_call(
                listen_fn,
                &[host_val.into(), port_i32.into()],
                "listen.fd",
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
                fd_i32,
                self.context.i32_type().const_zero(),
                "listen.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i64 = self
            .builder
            .build_int_s_extend(
                fd_i32,
                self.context.i64_type(),
                "listen.fd.i64",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            host_val,
            Some((fd_i64.into(), CodegenTy::Int)),
            "tcp.listen_socket",
        )
    }

    /// `std::io::tcp::connect(host, port) -> Int fallible(IoError)`.
    fn lower_std_io_tcp_connect_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::connect takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::connect: host must be String, got {:?}",
                host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::connect: port must be Int, got {:?}",
                port_ty
            )));
        }
        let connect_fn = self
            .module
            .get_function("lotus_tcp_connect")
            .expect("lotus_tcp_connect declared");
        let port_i32 = self
            .builder
            .build_int_truncate(
                port_val.into_int_value(),
                self.context.i32_type(),
                "connect.port.i32",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i32 = self
            .builder
            .build_call(
                connect_fn,
                &[host_val.into(), port_i32.into()],
                "connect.fd",
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
                fd_i32,
                self.context.i32_type().const_zero(),
                "connect.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i64 = self
            .builder
            .build_int_s_extend(
                fd_i32,
                self.context.i64_type(),
                "connect.fd.i64",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            host_val,
            Some((fd_i64.into(), CodegenTy::Int)),
            "tcp.connect",
        )
    }

    /// `std::io::tcp::accept_one(listen_fd) -> Int fallible(IoError)`.
    /// Path carries "" — no file path; agents inspect errno to
    /// distinguish "would block" from "listen fd closed."
    fn lower_std_io_tcp_accept_one_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::accept_one takes 1 arg (listen_fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::accept_one: listen_fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let accept_fn = self
            .module
            .get_function("lotus_tcp_accept_one")
            .expect("lotus_tcp_accept_one declared");
        let fd_i32 = self
            .builder
            .build_int_truncate(
                fd_val.into_int_value(),
                self.context.i32_type(),
                "accept.lfd.i32",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let conn_i32 = self
            .builder
            .build_call(accept_fn, &[fd_i32.into()], "accept.conn")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                conn_i32,
                self.context.i32_type().const_zero(),
                "accept.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let conn_i64 = self
            .builder
            .build_int_s_extend(
                conn_i32,
                self.context.i64_type(),
                "accept.conn.i64",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Empty-string path — no filename for socket ops.
        let empty_path = self.global_string("");
        self.complete_io_fallible_call(
            is_err,
            empty_path.into(),
            Some((conn_i64.into(), CodegenTy::Int)),
            "tcp.accept_one",
        )
    }

    /// Lower `std::io::tcp::__listen_socket(host: String,
    /// port: Int) -> Int`. host is passed through as the
    /// NUL-terminated string pointer Hale uses for String
    /// values; port is i64 truncated to i16 (port range fits).
    /// Returns the listen_fd as Int, sign-extended from i32.
    /// Stdlib loci call this from birth() and stash the result
    /// on self.
    fn lower_std_io_tcp_listen_socket(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__listen_socket takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__listen_socket: host must be String, got {:?}",
                host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__listen_socket: port must be Int, got {:?}",
                port_ty
            )));
        }
        let i16_t = self.context.i16_type();
        let i64_t = self.context.i64_type();
        let port_i16 = self
            .builder
            .build_int_truncate(port_val.into_int_value(), i16_t, "port.i16")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_listen_socket")
            .expect("lotus_tcp_listen_socket declared");
        let call = self
            .builder
            .build_call(f, &[host_val.into(), port_i16.into()], "listen.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let fd_i64 = self
            .builder
            .build_int_s_extend(fd_i32, i64_t, "fd.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((fd_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__connect(host: String, port: Int)
    /// -> Int`. Returns conn_fd (or -1 on error) as Int.
    fn lower_std_io_tcp_connect(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__connect takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__connect: host must be String, got {:?}",
                host_ty
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__connect: port must be Int, got {:?}",
                port_ty
            )));
        }
        let i16_t = self.context.i16_type();
        let i64_t = self.context.i64_type();
        let port_i16 = self
            .builder
            .build_int_truncate(port_val.into_int_value(), i16_t, "port.i16")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_connect")
            .expect("lotus_tcp_connect declared");
        let call = self
            .builder
            .build_call(f, &[host_val.into(), port_i16.into()], "connect.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let fd_i64 = self
            .builder
            .build_int_s_extend(fd_i32, i64_t, "connfd.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((fd_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__accept_one(listen_fd: Int) -> Int`.
    /// Returns the conn_fd (or -1 on error) as Int.
    fn lower_std_io_tcp_accept_one(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__accept_one takes 1 arg (listen_fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__accept_one: listen_fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "lfd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_accept_one")
            .expect("lotus_tcp_accept_one declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into()], "accept.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let conn_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let conn_i64 = self
            .builder
            .build_int_s_extend(conn_i32, i64_t, "conn.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((conn_i64.into(), CodegenTy::Int))
    }

    /// m89: Lower `std::io::tcp::__send_bytes(fd: Int, b: Bytes) -> Int`.
    /// Wraps `lotus_tcp_send_bytes` — same shape as `__send` but
    /// uses the explicit Bytes length, no NUL truncation. Returns
    /// the i32-cast-to-i64 result (0 on success, -1 on error)
    /// so user code can branch on it.
    fn lower_std_io_tcp_send_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send_bytes takes 2 args (fd, bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send_bytes: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send_bytes: bytes must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        // Truncate fd's Int (i64) → i32 to match the C ABI.
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(
                fd_val.into_int_value(),
                i32_t,
                "fd.i32",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_send_bytes")
            .expect("lotus_tcp_send_bytes declared");
        let call = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), b_val.into()],
                "tcp.send_bytes.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        // Sign-extend i32 → i64 so the result fits the Int contract.
        let i64_t = self.context.i64_type();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "send_bytes.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__send(fd: Int, msg: String) -> Int`.
    /// Writes msg's bytes (strlen-determined length) to fd.
    /// Returns 0 on success, -1 on error.
    fn lower_std_io_tcp_send(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send takes 2 args (fd, msg), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (msg_val, msg_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(msg_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send: msg must be String, got {:?}",
                msg_ty
            )));
        }
        let msg_val = self.unpack_view_if_needed(msg_val, &msg_ty)?;
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "send.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_send_str")
            .expect("lotus_tcp_send_str declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into(), msg_val.into()], "send.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "send.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__recv(fd: Int, max_bytes: Int) ->
    /// String`. Returns up to max_bytes from fd as a String
    /// (allocated in the lazy global arena, stable for program
    /// lifetime). Empty String on EOF or error.
    fn lower_std_io_tcp_recv(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv takes 2 args (fd, max_bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[1], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv: max_bytes must be Int, got {:?}",
                max_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "recv.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let max_i32 = self
            .builder
            .build_int_truncate(max_val.into_int_value(), i32_t, "recv.max.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_recv_str")
            .expect("lotus_tcp_recv_str declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into(), max_i32.into()], "recv.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// Phase 2g: lower `std::io::tcp::__recv_bytes(fd: Int,
    /// max_bytes: Int) -> Bytes`. Mirrors `__recv` but returns
    /// a length-prefixed Bytes blob anchored in the global
    /// payload arena, so embedded NUL bytes survive intact.
    /// Empty Bytes (length 0) on EOF, fd errors, or cap <= 0.
    fn lower_std_io_tcp_recv_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv_bytes takes 2 args (fd, max_bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv_bytes: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[1], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv_bytes: max_bytes must be Int, got {:?}",
                max_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "recvb.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let max_i32 = self
            .builder
            .build_int_truncate(
                max_val.into_int_value(),
                i32_t,
                "recvb.max.i32",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_recv_bytes")
            .expect("lotus_tcp_recv_bytes declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into(), max_i32.into()], "recvb.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// Phase 1: lower `std::io::tcp::recv_into(fd: Int, buf: Bytes,
    /// max_bytes: Int) -> Int`. `buf` is a builder handle (from
    /// `std::bytes::builder_new`). Reads up to max_bytes into the
    /// builder's tail; grows on insufficient headroom; bumps the
    /// builder's len by the count read. Returns POSIX read(2)
    /// semantics: > 0 bytes appended, 0 peer closed, -1 error.
    /// Zero allocation in g_bus_payload_arena.
    fn lower_std_io_tcp_recv_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        self.lower_recv_into_common(
            args,
            scope,
            "lotus_tcp_recv_into",
            "std::io::tcp::recv_into",
        )
    }

    /// `std::io::tcp::recv_stamped_into(fd, buf, max) -> Int`
    /// (2026-06-13). Identical contract to `recv_into` (same builder
    /// destination + `>0` / `0` EOF / `-1` fatal / `-2` retryable
    /// sentinels) but issues one `recvmsg(2)` that also captures the
    /// kernel RX timestamp. Read it with `last_recv_kernel_ns()`
    /// immediately after.
    fn lower_std_io_tcp_recv_stamped_into(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        self.lower_recv_into_common(
            args,
            scope,
            "lotus_tcp_recv_stamped",
            "std::io::tcp::recv_stamped_into",
        )
    }

    /// `std::io::tcp::last_recv_kernel_ns() -> Int` /
    /// `last_recv_user_ns() -> Int` (2026-06-13). Zero-arg reads of the
    /// thread-local stamps set by the most recent `recv_stamped_into`
    /// on this thread (errno-style, same idiom as
    /// `udp::last_source_*`). `kernel_ns` is 0 when no kernel timestamp
    /// was delivered (timestamps not enabled, or the platform lacks
    /// `SO_TIMESTAMPNS`).
    fn lower_std_io_tcp_last_recv_ns(
        &mut self,
        args: &[Expr],
        c_fn: &str,
        label: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::{} takes 0 args, got {}",
                label, args.len()
            )));
        }
        let f = self
            .module
            .get_function(c_fn)
            .expect("lotus_tcp_last_recv_*_ns declared");
        let v = self
            .builder
            .build_call(f, &[], &format!("tcp.{}.ret", label))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__close_fd(fd: Int) -> Int`. Returns
    /// 0 on success, -1 on error (errno set). Most callers
    /// discard the return.
    fn lower_std_io_tcp_close_fd(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__close_fd takes 1 arg (fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__close_fd: fd must be Int, got {:?}",
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
            .get_function("lotus_tcp_close_fd")
            .expect("lotus_tcp_close_fd declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into()], "close.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "close.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// `std::io::tcp::__set_recv_timeout_ns(fd: Int, ns: Int) -> Int`.
    /// SO_RCVTIMEO on the socket; on a main-thread blocking accept()
    /// this bounds the idle wait (accept returns -1 after `ns`).
    /// Returns the setsockopt result (0 / -1).
    fn lower_std_io_tcp_set_recv_timeout(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__set_recv_timeout_ns takes 2 args \
                 (fd, ns), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__set_recv_timeout_ns: fd must be Int, \
                 got {:?}",
                fd_ty
            )));
        }
        let (ns_val, ns_ty) = self.lower_expr(&args[1], scope)?;
        if ns_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__set_recv_timeout_ns: ns must be Int, \
                 got {:?}",
                ns_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "to.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_set_recv_timeout_ns")
            .expect("lotus_tcp_set_recv_timeout_ns declared");
        let call = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), ns_val.into_int_value().into()],
                "to.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "to.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// `std::io::tcp::__shutdown_listen_socket(fd: Int) -> Int`.
    /// C-iii (2026-05-21): graceful interrupt for a blocking
    /// accept(). Calls `shutdown(fd, SHUT_RDWR)` to release any
    /// thread sitting in accept on this listen socket. The fd
    /// stays open — dissolve() handles the close. Returns the
    /// syscall return value (0 / -1). Safe to call from any
    /// thread, including cross-scheduler — that's the whole
    /// point of this primitive.
    fn lower_std_io_tcp_shutdown_listen_socket(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__shutdown_listen_socket takes 1 arg \
                 (fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__shutdown_listen_socket: fd must \
                 be Int, got {:?}",
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
            .get_function("lotus_tcp_shutdown_listen_socket")
            .expect("lotus_tcp_shutdown_listen_socket declared");
        let ret_i32 = self
            .builder
            .build_call(f, &[fd_i32.into()], "shutdown_listen.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "shutdown_listen.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

}
