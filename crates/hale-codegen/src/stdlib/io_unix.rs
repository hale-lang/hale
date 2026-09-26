//! `std::io::unix::*` path-call lowering (GH #1106).
//!
//! AF_UNIX stream sockets with the fd-level shape `std::io::tcp`
//! has: `listen_socket(path)` binds and listens, `connect(path)` /
//! `connect_wait(path, wait)` return a connected fd, and accept,
//! recv, send, shutdown and close are the tcp primitives, which are
//! address-family agnostic. The API binding's socket side is built
//! on these, and so is a test that drives it.

use hale_syntax::ast::Expr;

use crate::codegen::{CodegenError, CodegenTy, Cx, FallibleCallResult, Scope};

pub(crate) trait IoUnixStdlib<'ctx> {
    fn lower_std_io_unix_listen_socket_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_unix_connect_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_io_unix_connect_wait_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
}

impl<'ctx, 'p> IoUnixStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::io::unix::listen_socket(path) -> Int fallible(IoError)`.
    /// The IoError's path field carries the socket path.
    fn lower_std_io_unix_listen_socket_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::listen_socket takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::listen_socket: path must be String, got {:?}",
                path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let listen_fn = self
            .module
            .get_function("lotus_unix_listen_socket")
            .expect("lotus_unix_listen_socket declared");
        let fd_i32 = self
            .builder
            .build_call(listen_fn, &[path_val.into()], "unix.listen.fd")
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
                "unix.listen.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i64 = self
            .builder
            .build_int_s_extend(fd_i32, self.context.i64_type(), "unix.listen.fd.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((fd_i64.into(), CodegenTy::Int)),
            "unix.listen_socket",
        )
    }

    /// `std::io::unix::connect(path) -> Int fallible(IoError)`: one
    /// attempt; a missing or refusing socket fails at once.
    fn lower_std_io_unix_connect_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_unix_connect_common(args, scope, false)
    }

    /// `std::io::unix::connect_wait(path, wait) -> Int
    /// fallible(IoError)`: retry a missing or refusing socket for up
    /// to `wait` (a Duration), for a caller racing its peer's listen.
    fn lower_std_io_unix_connect_wait_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_unix_connect_common(args, scope, true)
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    pub(crate) fn lower_std_io_unix_peer(
        &mut self,
        which: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(inkwell::values::BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::peer_{} takes 1 arg (fd), got {}",
                which,
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::peer_{}: fd must be Int, got {:?}",
                which, fd_ty
            )));
        }
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), self.context.i32_type(), "peer.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function(&format!("lotus_unix_peer_{}", which))
            .expect("lotus_unix_peer_* declared");
        let v = self
            .builder
            .build_call(f, &[fd_i32.into()], "peer.cred")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// GH #1109: `std::io::unix::user_id(name) -> Int` and
    /// `group_id(name) -> Int`: the host's account database, -1 when
    /// it has no such name.
    pub(crate) fn lower_std_io_unix_name_id(
        &mut self,
        which: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(inkwell::values::BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::{} takes 1 arg (name), got {}",
                which,
                args.len()
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(name_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::{}: name must be String, got {:?}",
                which, name_ty
            )));
        }
        let name_val = self.unpack_view_if_needed(name_val, &name_ty)?;
        let f = self
            .module
            .get_function(&format!("lotus_unix_{}", which))
            .expect("lotus_unix_{user,group}_id declared");
        let v = self
            .builder
            .build_call(f, &[name_val.into()], "unix.name_id")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// GH #1109: `std::io::unix::in_group(uid, gid) -> Bool`: the uid's
    /// primary or supplementary group membership per the host.
    pub(crate) fn lower_std_io_unix_in_group(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(inkwell::values::BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::in_group takes 2 args (uid, gid), got {}",
                args.len()
            )));
        }
        let (uid_val, uid_ty) = self.lower_expr(&args[0], scope)?;
        let (gid_val, gid_ty) = self.lower_expr(&args[1], scope)?;
        if uid_ty != CodegenTy::Int || gid_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::in_group: uid and gid must be Int, got {:?} and {:?}",
                uid_ty, gid_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_unix_in_group")
            .expect("lotus_unix_in_group declared");
        let ret = self
            .builder
            .build_call(f, &[uid_val.into(), gid_val.into()], "unix.in_group")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let b = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret,
                self.context.i32_type().const_zero(),
                "unix.in_group.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((b.into(), CodegenTy::Bool))
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    fn lower_unix_connect_common(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        with_wait: bool,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let name = if with_wait { "connect_wait" } else { "connect" };
        let want = if with_wait { 2 } else { 1 };
        if args.len() != want {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::{} takes {} arg(s) (path{}), got {}",
                name,
                want,
                if with_wait { ", wait" } else { "" },
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::io::unix::{}: path must be String, got {:?}",
                name, path_ty
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let i64_t = self.context.i64_type();
        let wait_ns = if with_wait {
            let (w, wty) = self.lower_expr(&args[1], scope)?;
            if !matches!(wty, CodegenTy::Duration | CodegenTy::Int) {
                return Err(CodegenError::Unsupported(format!(
                    "std::io::unix::connect_wait: wait must be a Duration, got {:?}",
                    wty
                )));
            }
            w.into_int_value()
        } else {
            i64_t.const_zero()
        };
        let connect_fn = self
            .module
            .get_function("lotus_unix_connect_wait")
            .expect("lotus_unix_connect_wait declared");
        let fd_i32 = self
            .builder
            .build_call(
                connect_fn,
                &[path_val.into(), wait_ns.into()],
                "unix.connect.fd",
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
                "unix.connect.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i64 = self
            .builder
            .build_int_s_extend(fd_i32, i64_t, "unix.connect.fd.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((fd_i64.into(), CodegenTy::Int)),
            "unix.connect",
        )
    }
}
