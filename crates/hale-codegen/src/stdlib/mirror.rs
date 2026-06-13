//! `std::io::mirror::__*` primitives backing the `MirrorRing` stdlib locus
//! (#3 of the fast-protocol-I/O substrate plan). A double-mmap "magic ring":
//! `readable()` / `writable()` hand out `{ptr,len}` BytesMut windows over
//! the live / free regions (contiguous across the physical seam — zero
//! copy), `commit`/`consume` move the cursors, and `__recv_into` recvs
//! straight into the free window. The handle is carried as an `Int`
//! (pointer) in the locus's `handle` field, exactly like `BytesBuilder`.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait MirrorStdlib<'ctx> {
    fn lower_std_io_mirror(
        &mut self,
        op: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> MirrorStdlib<'ctx> for Cx<'ctx, 'p> {
    fn lower_std_io_mirror(
        &mut self,
        op: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());

        // Lower an Int arg to its SSA i64 value, checking the type.
        macro_rules! int_arg {
            ($idx:expr, $what:expr) => {{
                let (v, t) = self.lower_expr(&args[$idx], scope)?;
                if t != CodegenTy::Int {
                    return Err(CodegenError::Unsupported(format!(
                        "std::io::mirror::__{}: {} must be Int, got {:?}",
                        op, $what, t
                    )));
                }
                v.into_int_value()
            }};
        }
        // The handle (arg 0 for all but __new) → a pointer.
        let handle_ptr = |me: &mut Self, h: inkwell::values::IntValue<'ctx>| {
            me.builder
                .build_int_to_ptr(h, ptr_t, "mirror.handle.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
        };
        let call = |me: &mut Self, name: &str, a: &[inkwell::values::BasicMetadataValueEnum<'ctx>]| {
            let f = me.module.get_function(name).expect("mirror primitive declared");
            me.builder
                .build_call(f, a, &format!("{}.ret", name))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
        };

        match op {
            "__new" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(
                        "std::io::mirror::__new takes 1 arg (capacity)".into(),
                    ));
                }
                let cap = int_arg!(0, "capacity");
                let r = call(self, "lotus_mirror_ring_new", &[cap.into()])?
                    .try_as_basic_value()
                    .left()
                    .expect("returns ptr")
                    .into_pointer_value();
                // Carry the handle as Int (ptrtoint), like BytesBuilder.
                let as_int = self
                    .builder
                    .build_ptr_to_int(r, i64_t, "mirror.handle.int")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((as_int.into(), CodegenTy::Int))
            }
            "__free" | "__commit" | "__consume" => {
                let h = int_arg!(0, "handle");
                let hp = handle_ptr(self, h)?;
                if op == "__free" {
                    call(self, "lotus_mirror_ring_free", &[hp.into()])?;
                } else {
                    let n = int_arg!(1, "n");
                    let name = if op == "__commit" {
                        "lotus_mirror_ring_commit"
                    } else {
                        "lotus_mirror_ring_consume"
                    };
                    call(self, name, &[hp.into(), n.into()])?;
                }
                Ok((i64_t.const_zero().into(), CodegenTy::Int))
            }
            "__readable" | "__writable" => {
                let h = int_arg!(0, "handle");
                let hp = handle_ptr(self, h)?;
                let name = if op == "__readable" {
                    "lotus_mirror_ring_readable"
                } else {
                    "lotus_mirror_ring_writable"
                };
                let v = call(self, name, &[hp.into()])?
                    .try_as_basic_value()
                    .left()
                    .expect("returns view struct");
                Ok((v, CodegenTy::BytesMut))
            }
            "__len" | "__capacity" => {
                let h = int_arg!(0, "handle");
                let hp = handle_ptr(self, h)?;
                let name = if op == "__len" {
                    "lotus_mirror_ring_len"
                } else {
                    "lotus_mirror_ring_capacity"
                };
                let v = call(self, name, &[hp.into()])?
                    .try_as_basic_value()
                    .left()
                    .expect("returns i64");
                Ok((v, CodegenTy::Int))
            }
            "__recv_into" => {
                if args.len() != 3 {
                    return Err(CodegenError::Unsupported(
                        "std::io::mirror::__recv_into takes 3 args (handle, fd, max)".into(),
                    ));
                }
                let h = int_arg!(0, "handle");
                let hp = handle_ptr(self, h)?;
                let fd = int_arg!(1, "fd");
                let fd_i32 = self
                    .builder
                    .build_int_truncate(fd, i32_t, "mirror.fd.i32")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let max = int_arg!(2, "max");
                let v = call(self, "lotus_tcp_recv_into_mirror",
                            &[fd_i32.into(), hp.into(), max.into()])?
                    .try_as_basic_value()
                    .left()
                    .expect("returns i64");
                Ok((v, CodegenTy::Int))
            }
            _ => Err(CodegenError::Unsupported(format!(
                "unknown std::io::mirror primitive `__{}`",
                op
            ))),
        }
    }
}
