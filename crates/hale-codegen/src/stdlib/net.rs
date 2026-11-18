//! `std::net::*` (GH #1908): name resolution. `resolve` is `net.hl`'s
//! body; it reads the resolver's answer through `__resolve`, lowered
//! here over `lotus_net_resolve`.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// `std::net::__resolve(host: String) -> String`: the resolver's
    /// answer as `lotus_net_resolve` writes it into the current arena
    /// (`"ok\n<addr>…"` or `"err\n<kind>\n<detail>"`).
    pub(crate) fn lower_std_net_resolve(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::net::__resolve takes 1 arg (host), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(host_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::net::__resolve: host must be String, got {host_ty}"
            )));
        }
        let host_val = self.unpack_view_if_needed(host_val, &host_ty)?;
        let arena_ptr = self.current_arena_ptr()?;
        let f = self.module.get_function("lotus_net_resolve").expect("lotus_net_resolve declared");
        let v = self
            .builder
            .build_call(f, &[arena_ptr.into(), host_val.into()], "net.resolve")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((v, CodegenTy::String))
    }
}
