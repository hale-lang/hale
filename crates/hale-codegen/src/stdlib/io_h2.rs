//! `std::io::h2::__*` path-call lowering: the HTTP/2 server session of
//! `runtime/lotus_h2.c` (nghttp2 inside). Every primitive is one C call over
//! an `Int` handle and `Bytes` messages, so one lowering serves all twelve:
//! evaluate the arguments, declare the C function the first time a program
//! reaches it (a program that serves no HTTP/2 carries no declaration, and
//! the link takes the library's objects only when a declaration is used),
//! call it, and hand back an `Int` or a `Bytes`.

use hale_syntax::ast::Expr;
use hale_types::stdlib_surface::IntrinsicId;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

/// What an argument of the C function is.
#[derive(Clone, Copy, PartialEq)]
enum Arg {
    Int,
    Bytes,
}

/// The C function an id lowers to: its name, its arguments and whether it
/// returns a `Bytes`.
fn shape(id: IntrinsicId) -> (&'static str, &'static [Arg], bool) {
    use Arg::{Bytes as B, Int as I};
    use IntrinsicId as Id;
    match id {
        Id::IoH2OpenRaw => ("lotus_h2_server_open", &[], false),
        Id::IoH2FeedRaw => ("lotus_h2_feed", &[I, B], false),
        Id::IoH2DrainRaw => ("lotus_h2_drain", &[I], true),
        Id::IoH2PollRaw => ("lotus_h2_poll", &[I], false),
        Id::IoH2EvStreamRaw => ("lotus_h2_ev_stream", &[I], false),
        Id::IoH2EvCodeRaw => ("lotus_h2_ev_code", &[I], false),
        Id::IoH2EvBytesRaw => ("lotus_h2_ev_bytes", &[I], true),
        Id::IoH2RespondRaw => ("lotus_h2_respond", &[I, I, B, B, B], false),
        Id::IoH2ResetRaw => ("lotus_h2_reset", &[I, I, I], false),
        Id::IoH2GoawayRaw => ("lotus_h2_goaway", &[I, I], false),
        Id::IoH2AliveRaw => ("lotus_h2_alive", &[I], false),
        Id::IoH2CloseRaw => ("lotus_h2_close", &[I], false),
        other => unreachable!("{other:?} is not an io::h2 primitive"),
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    pub(crate) fn lower_std_io_h2(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let (c_fn, params, returns_bytes) = shape(id);
        let surface = format!("std::io::h2::{}", c_fn.trim_start_matches("lotus_h2_"));
        if args.len() != params.len() {
            return Err(CodegenError::Unsupported(format!(
                "{surface} takes {} args, got {}",
                params.len(),
                args.len()
            )));
        }
        let mut lowered: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        for (arg, want) in args.iter().zip(params) {
            let (v, ty) = self.lower_expr(arg, scope)?;
            match want {
                Arg::Int if ty == CodegenTy::Int => lowered.push(v.into()),
                Arg::Bytes if matches!(ty, CodegenTy::Bytes | CodegenTy::BytesView) => {
                    let v = self.unpack_view_if_needed(v, &ty)?;
                    lowered.push(v.into());
                }
                _ => {
                    return Err(CodegenError::Unsupported(format!(
                        "{surface}: an argument must be {}, got {:?}",
                        if *want == Arg::Int { "Int" } else { "Bytes" },
                        ty
                    )))
                }
            }
        }
        let f = match self.module.get_function(c_fn) {
            Some(f) => f,
            None => {
                let i64_t = self.context.i64_type();
                let ptr_t = self.context.ptr_type(AddressSpace::default());
                let tys: Vec<BasicMetadataTypeEnum<'ctx>> = params
                    .iter()
                    .map(|a| match a {
                        Arg::Int => i64_t.into(),
                        Arg::Bytes => ptr_t.into(),
                    })
                    .collect();
                let fn_ty = if returns_bytes { ptr_t.fn_type(&tys, false) } else { i64_t.fn_type(&tys, false) };
                self.module.add_function(c_fn, fn_ty, None)
            }
        };
        // A Bytes result is allocated in the caller's arena, as every
        // primitive that returns one does.
        if returns_bytes {
            self.emit_set_caller_arena()?;
        }
        let call = self
            .builder
            .build_call(f, &lowered, "h2.call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret = call.try_as_basic_value().left().expect("an io::h2 primitive returns a value");
        Ok((ret, if returns_bytes { CodegenTy::Bytes } else { CodegenTy::Int }))
    }
}
