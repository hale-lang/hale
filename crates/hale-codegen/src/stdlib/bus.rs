//! `std::bus::*` path-call lowering for `__local_dispatch`. The
//! rest of the bus surface (publish, subscribe, wire) lives in
//! `crate::codegen` (Round 3 target).

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait BusStdlib<'ctx> {
    fn lower_std_bus_local_dispatch(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bus_transport_realize(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bus_transport_handle_op(
        &mut self,
        c_fn: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bus_binding_fail(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> BusStdlib<'ctx> for Cx<'ctx, 'p> {
    /// m105: lower `std::bus::__local_dispatch(subject: String,
    /// wire_bytes: Bytes) -> ()`. Hands wire bytes (received by an
    /// adapter from its transport) through the subject's registered
    /// deserialize fn into the local handler set. The Hale surface
    /// is the inbound counterpart to an adapter's outbound `send`.
    fn lower_std_bus_local_dispatch(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bus::__local_dispatch takes 2 args (subject, bytes), got {}",
                args.len()
            )));
        }
        let (subj_val, subj_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(subj_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bus::__local_dispatch: subject must be String, got {:?}",
                subj_ty
            )));
        }
        let subj_val = self.unpack_view_if_needed(subj_val, &subj_ty)?;
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bus::__local_dispatch: bytes must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        // The C primitive takes (subject, wire_ptr, wire_size).
        // Bytes carries an explicit length prefix; load it and
        // pass the body pointer plus the length explicitly so the
        // runtime doesn't have to peek at our Bytes layout.
        let i64_t = self.context.i64_type();
        let bytes_ptr = b_val.into_pointer_value();
        let len = self
            .builder
            .build_load(i64_t, bytes_ptr, "dispatch.bytes.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        // Body starts after the 8-byte length prefix.
        let body_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    bytes_ptr,
                    &[i64_t.const_int(8, false)],
                    "dispatch.bytes.body",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let f = self
            .module
            .get_function("lotus_bus_dispatch_wire")
            .expect("lotus_bus_dispatch_wire declared");
        self.builder
            .build_call(
                f,
                &[subj_val.into(), body_ptr.into(), len.into()],
                "bus.local_dispatch",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Match the udp surface: return 0 as Int for "success."
        // Callers normally invoke as a statement and ignore the
        // return; the value is here so expression-position calls
        // type-check uniformly.
        Ok((i64_t.const_zero().into(), CodegenTy::Int))
    }

    /// GH #233: `std::bus::__transport_realize(subject: String,
    /// path: String, role: Int) -> Int` — realize a unix bus
    /// transport (socket + bind + listen, or connect-with-retry)
    /// and register it with the fanout table. Returns the entry
    /// handle, 0 on failure. Called from
    /// __StdBusUnixTransport.birth() only.
    fn lower_std_bus_transport_realize(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::bus::__transport_realize takes 3 args \
                 (subject, path, role), got {}",
                args.len()
            )));
        }
        let (subj_val, subj_ty) = self.lower_expr(&args[0], scope)?;
        let (path_val, path_ty) = self.lower_expr(&args[1], scope)?;
        for (name, ty) in [("subject", &subj_ty), ("path", &path_ty)] {
            if !matches!(ty, CodegenTy::String | CodegenTy::StringView) {
                return Err(CodegenError::Unsupported(format!(
                    "std::bus::__transport_realize: {} must be String, got {:?}",
                    name, ty
                )));
            }
        }
        let subj_val = self.unpack_view_if_needed(subj_val, &subj_ty)?;
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let (role_val, role_ty) = self.lower_expr(&args[2], scope)?;
        if !matches!(role_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::bus::__transport_realize: role must be Int, got {:?}",
                role_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bus_transport_realize")
            .expect("lotus_bus_transport_realize declared");
        let handle = self
            .builder
            .build_call(
                f,
                &[subj_val.into(), path_val.into(), role_val.into()],
                "bus.transport.realize",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("realize returns i64");
        Ok((handle, CodegenTy::Int))
    }

    /// GH #233: shared lowering for the 1-arg handle ops —
    /// `std::bus::__transport_serve(h)` / `__transport_reclaim(h)`.
    fn lower_std_bus_transport_handle_op(
        &mut self,
        c_fn: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "{} takes 1 arg (handle), got {}",
                c_fn,
                args.len()
            )));
        }
        let (h_val, h_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(h_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "{}: handle must be Int, got {:?}",
                c_fn, h_ty
            )));
        }
        let f = self
            .module
            .get_function(c_fn)
            .unwrap_or_else(|| panic!("{} declared", c_fn));
        let call = self
            .builder
            .build_call(f, &[h_val.into()], "bus.transport.op")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i64_t = self.context.i64_type();
        // i64-returning ops (spawn_server) surface their status;
        // void ops (reclaim) type as Int 0 so statement-position
        // calls stay uniform.
        let ret = call
            .try_as_basic_value()
            .left()
            .unwrap_or_else(|| i64_t.const_zero().into());
        Ok((ret, CodegenTy::Int))
    }

    /// #227/#233: `std::bus::__binding_fail(subject: String,
    /// url: String)` — the structural-failure sink (stderr +
    /// exit(1)); called from __StdBusUnixTransport.birth() when
    /// realization fails and no handled route exists.
    fn lower_std_bus_binding_fail(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bus::__binding_fail takes 2 args (subject, url), got {}",
                args.len()
            )));
        }
        let (subj_val, subj_ty) = self.lower_expr(&args[0], scope)?;
        let (url_val, url_ty) = self.lower_expr(&args[1], scope)?;
        for (name, ty) in [("subject", &subj_ty), ("url", &url_ty)] {
            if !matches!(ty, CodegenTy::String | CodegenTy::StringView) {
                return Err(CodegenError::Unsupported(format!(
                    "std::bus::__binding_fail: {} must be String, got {:?}",
                    name, ty
                )));
            }
        }
        let subj_val = self.unpack_view_if_needed(subj_val, &subj_ty)?;
        let url_val = self.unpack_view_if_needed(url_val, &url_ty)?;
        let f = self
            .module
            .get_function("lotus_bus_binding_fail")
            .expect("lotus_bus_binding_fail declared");
        self.builder
            .build_call(f, &[subj_val.into(), url_val.into()], "bus.binding.fail")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i64_t = self.context.i64_type();
        Ok((i64_t.const_zero().into(), CodegenTy::Int))
    }
}
