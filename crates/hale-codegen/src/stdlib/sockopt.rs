//! `std::io::sockopt::*` path-call lowering. One generic getter
//! routes the ~30 named-constant surface (IPPROTO_*, IP_*, SO_*,
//! SOL_SOCKET) to per-constant C primitives. The constant list
//! `SOCKOPT_NAMES` lives in `codegen.rs`, where declare-builtins
//! declares a getter for each; a call is dispatched from its
//! `std::io::sockopt` row.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx};

pub(crate) trait SockoptStdlib<'ctx> {
    fn lower_std_io_sockopt_getter(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> SockoptStdlib<'ctx> for Cx<'ctx, 'p> {
    /// 2026-05-26 — `std::io::sockopt::<NAME>() -> Int`. Each
    /// named constant resolves to a zero-arg call into the
    /// matching C getter (`lotus_sockopt_<NAME>`) which returns
    /// the platform's numeric value. Used as the level / name
    /// args to `std::io::udp::set_option_int` / friends.
    fn lower_std_io_sockopt_getter(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::io::sockopt::{} takes 0 args, got {}",
                name,
                args.len()
            )));
        }
        let f = self
            .module
            .get_function(&format!("lotus_sockopt_{}", name))
            .expect("sockopt getter declared");
        let v = self
            .builder
            .build_call(f, &[], &format!("sockopt.{}.ret", name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let v_i64 = self
            .builder
            .build_int_s_extend(v, self.context.i64_type(), "sockopt.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((v_i64.into(), CodegenTy::Int))
    }
}

#[cfg(test)]
mod tests {
    use crate::codegen::SOCKOPT_NAMES;

    /// The checker's stdlib surface and codegen's constant list are two
    /// spellings of one set. Codegen lowered `std::io::sockopt::*` from
    /// the start, but the checker's table had no such namespace, so every
    /// program using one was refused as "unknown stdlib namespace" and
    /// the docs said so. Each name codegen lowers is known to the
    /// checker, and the checker knows no name codegen would refuse.
    #[test]
    fn the_checker_knows_every_constant_codegen_lowers() {
        for name in SOCKOPT_NAMES {
            assert_eq!(
                hale_types::stdlib_surface::unknown_fn_error(&["std", "io", "sockopt", name]),
                None,
                "`std::io::sockopt::{name}` is lowered by codegen but refused by the checker"
            );
        }
        // (a variable, not a literal path: the registry parity scraper
        // reads every `"std", "io", …` literal in this source as a
        // dispatch arm)
        let bogus = String::from("NOT_A_CONSTANT");
        assert!(
            hale_types::stdlib_surface::unknown_fn_error(&["std", "io", "sockopt", &bogus]).is_some(),
            "the namespace is tabled, so a name codegen does not lower is refused"
        );
    }

    /// The other direction: a call dispatches from its row (F.40 phase
    /// 4, S3), and the row's arm calls the getter declare-builtins
    /// declared for the name, so every row is a name of the list.
    #[test]
    fn every_sockopt_row_has_a_declared_getter() {
        let rows: Vec<&str> = hale_types::stdlib_surface::rows()
            .filter(|(s, _)| s.ns == ["io", "sockopt"])
            .map(|(_, f)| f.name)
            .collect();
        assert_eq!(rows, SOCKOPT_NAMES, "the `std::io::sockopt` rows and `SOCKOPT_NAMES` differ");
    }
}
