//! `std::bytes::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait BytesStdlib<'ctx> {
    fn lower_std_bytes_at_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    /// `std::bytes::read_<type>_<endian>(b, off)` binary-pack readers
    /// (u8/u16/u32/u64, i8/i16/i32/i64, f32/f64), each
    /// `-> Int|Float fallible(IndexError)`. `name` is the bare fn name
    /// (e.g. "read_u32_le"); it carries width/signedness/endianness.
    fn lower_std_bytes_read(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_bytes_builder_new(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_bytes_builder_handle_arg(
        &mut self,
        arg: &Expr,
        scope: &Scope<'ctx>,
        diag_name: &str,
    ) -> Result<(BasicValueEnum<'ctx>, BasicValueEnum<'ctx>), CodegenError>;
    fn lower_std_bytes_builder_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_len(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_finish(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_shift_front(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_clear(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_snapshot(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_free(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_view(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_text_view(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_builder_append_slice(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    /// shm-ring-interop Proposal A (M2): append the low `width` bytes
    /// of an Int value (args: handle, value, width, big_endian).
    fn lower_std_bytes_builder_append_scalar(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    /// Append a Float as 8 (f64) or 4 (f32) raw IEEE bytes. `is_f32`
    /// truncates to f32 first (args: handle, value, big_endian).
    fn lower_std_bytes_builder_append_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        is_f32: bool,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    /// Zero-fill to the next `to_align` boundary (args: handle, to_align).
    fn lower_std_bytes_builder_append_pad(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_is_alloc_fail(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_clone(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_from_string(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_at(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_slice(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_from_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_bytes_concat(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> BytesStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::bytes::at(b, i) -> Int fallible(IndexError)`.
    /// Replaces the legacy -1 sentinel — agents reflexively wrap
    /// `bytes_at(b, i) or raise`, same shape as `vec.get(i)`.
    fn lower_std_bytes_at_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at takes 2 args (b, i), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at: b must be Bytes, got {:?}. To read \
                 a byte from a String, convert first via \
                 `std::bytes::from_string(s)`.",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let (i_val, i_ty) = self.lower_expr(&args[1], scope)?;
        if i_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at: i must be Int, got {:?}",
                i_ty
            )));
        }
        let at_fn = self
            .module
            .get_function("lotus_bytes_at")
            .expect("lotus_bytes_at declared");
        let raw = self
            .builder
            .build_call(
                at_fn,
                &[b_val.into(), i_val.into()],
                "bytes.at.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_bytes_at returns i64")
            .into_int_value();
        // -1 sentinel from the C primitive.
        let neg_one = self
            .context
            .i64_type()
            .const_int((-1i64) as u64, true);
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                raw,
                neg_one,
                "bytes.at.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Mirror the vec.get lazy-IndexError pattern. We need
        // bytes_len for the IndexError's `len` field — fetched
        // only on the err path.
        let payload_ty = CodegenTy::TypeRef("IndexError".to_string());
        let out_val_slot = self.alloca_for(&CodegenTy::Int, "bytes.at.out_val")?;
        let out_err_slot = self.alloca_for(&payload_ty, "bytes.at.out_err")?;
        self.builder
            .build_store(out_val_slot, raw)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let func = self
            .current_fn
            .expect("bytes.at inside fn body");
        let lazy_err_bb = self
            .context
            .append_basic_block(func, "bytes.at.lazy_err");
        let join_bb = self
            .context
            .append_basic_block(func, "bytes.at.join");
        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(lazy_err_bb);
        let len_fn = self
            .module
            .get_function("lotus_bytes_len")
            .expect("lotus_bytes_len declared");
        let len_ssa = self
            .builder
            .build_call(len_fn, &[b_val.into()], "bytes.len.for_err")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let ie_ptr = self.emit_index_error_alloc(
            "out_of_bounds",
            i_val.into_int_value(),
            len_ssa,
        )?;
        self.builder
            .build_store(out_err_slot, ie_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(CodegenTy::Int),
            payload_ty,
        })
    }

    fn lower_std_bytes_read(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        // Parse `read_<type>[_<endian>]` → width / signed / endian /
        // float. u8/i8 have no endian suffix (width 1).
        let spec = name.strip_prefix("read_").ok_or_else(|| {
            CodegenError::Unsupported(format!("not a bytes reader: {}", name))
        })?;
        let (tok, big_endian) = if let Some(t) = spec.strip_suffix("_le") {
            (t, false)
        } else if let Some(t) = spec.strip_suffix("_be") {
            (t, true)
        } else {
            (spec, false) // u8 / i8 — width 1, endianness irrelevant
        };
        let (width, is_signed, is_float): (i32, bool, bool) = match tok {
            "u8" => (1, false, false),
            "u16" => (2, false, false),
            "u32" => (4, false, false),
            "u64" => (8, false, false),
            "i8" => (1, true, false),
            "i16" => (2, true, false),
            "i32" => (4, true, false),
            "i64" => (8, true, false),
            "f32" => (4, false, true),
            "f64" => (8, false, true),
            _ => {
                return Err(CodegenError::Unsupported(format!(
                    "unknown bytes reader `std::bytes::{}`",
                    name
                )))
            }
        };

        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::{} takes 2 args (b, off), got {}",
                name,
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::{}: first arg must be Bytes, got {:?}",
                name, b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let (off_val, off_ty) = self.lower_expr(&args[1], scope)?;
        if off_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::{}: offset must be Int, got {:?}",
                name, off_ty
            )));
        }
        let off_ssa = off_val.into_int_value();

        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        // oob out-param (i64 *) — entry-block alloca so it's hoisted.
        let oob_slot = self.alloca_for(&CodegenTy::Int, "bytes.read.oob")?;
        let read_fn = self
            .module
            .get_function("lotus_bytes_read_uint")
            .expect("lotus_bytes_read_uint declared");
        let raw = self
            .builder
            .build_call(
                read_fn,
                &[
                    b_val.into(),
                    off_ssa.into(),
                    i32_t.const_int(width as u64, false).into(),
                    i32_t.const_int(is_signed as u64, false).into(),
                    i32_t.const_int(big_endian as u64, false).into(),
                    oob_slot.into(),
                ],
                "bytes.read.raw",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_bytes_read_uint returns i64")
            .into_int_value();
        // is_err = (*oob != 0)
        let oob_v = self
            .builder
            .build_load(i64_t, oob_slot, "bytes.read.oob.v")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                oob_v,
                i64_t.const_zero(),
                "bytes.read.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Success value: int readers hand back the raw i64; float
        // readers bit-cast the raw bits (f64) or truncate+bitcast+
        // fpext (f32 → Hale's f64 Float).
        let (success_val, success_ty): (BasicValueEnum<'ctx>, CodegenTy) =
            if !is_float {
                (raw.into(), CodegenTy::Int)
            } else if width == 8 {
                let f = self
                    .builder
                    .build_bit_cast(raw, self.context.f64_type(), "bytes.read.f64")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                (f, CodegenTy::Float)
            } else {
                let bits32 = self
                    .builder
                    .build_int_truncate(raw, i32_t, "bytes.read.f32.bits")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let f32v = self
                    .builder
                    .build_bit_cast(bits32, self.context.f32_type(), "bytes.read.f32")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_float_value();
                let f64v = self
                    .builder
                    .build_float_ext(f32v, self.context.f64_type(), "bytes.read.f32.ext")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                (f64v.into(), CodegenTy::Float)
            };

        // Lazy IndexError on the err path (mirrors bytes::at): fetch
        // len only when out of bounds.
        let payload_ty = CodegenTy::TypeRef("IndexError".to_string());
        let out_val_slot = self.alloca_for(&success_ty, "bytes.read.out_val")?;
        let out_err_slot = self.alloca_for(&payload_ty, "bytes.read.out_err")?;
        self.builder
            .build_store(out_val_slot, success_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let func = self.current_fn.expect("bytes.read inside fn body");
        let lazy_err_bb =
            self.context.append_basic_block(func, "bytes.read.lazy_err");
        let join_bb = self.context.append_basic_block(func, "bytes.read.join");
        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(lazy_err_bb);
        let len_fn = self
            .module
            .get_function("lotus_bytes_len")
            .expect("lotus_bytes_len declared");
        let len_ssa = self
            .builder
            .build_call(len_fn, &[b_val.into()], "bytes.read.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let ie_ptr =
            self.emit_index_error_alloc("out_of_bounds", off_ssa, len_ssa)?;
        self.builder
            .build_store(out_err_slot, ie_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(success_ty),
            payload_ty,
        })
    }

    /// C10 (pond follow-up): `std::bytes::builder_new() -> Bytes`.
    /// Allocates a doubling-realloc-backed buffer; Bytes is the
    /// carrier type for the opaque handle, matching the str-builder
    /// ergonomic. The append chunk and the finish result are both
    /// Bytes-shaped (length-prefixed) so embedded NULs survive the
    /// round-trip — pond/http/client + pond/agent/llm wanted this
    /// shape for chunked message-body accumulation.
    fn lower_std_bytes_builder_new(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__new takes 1 arg (initial_cap), got {}",
                args.len()
            )));
        }
        let (cap_val, cap_ty) = self.lower_expr(&args[0], scope)?;
        if cap_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__new: initial_cap must be Int, got {:?}",
                cap_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_builder_new")
            .expect("lotus_bytes_builder_new declared");
        let call = self
            .builder
            .build_call(f, &[cap_val.into()], "bb.new.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        // Carry the ptr as Int (i64) so the BytesBuilder locus's
        // `handle` param (typed Int) can hold it. The C primitive
        // declared a `ptr` return; ptrtoint here, inttoptr at the
        // matching consumer sites.
        let i64_t = self.context.i64_type();
        let as_int = self
            .builder
            .build_ptr_to_int(ptr, i64_t, "bb.new.handle.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((as_int.into(), CodegenTy::Int))
    }

    /// Helper: lower an `Int`-typed handle expression and emit
    /// `inttoptr` so the C-primitive call boundary receives a
    /// `ptr`. Returns the original `(IntValue, BasicValueEnum)`
    /// pair — caller forwards the int as the lowering's result
    /// (handles "return the handle unchanged" shapes) and uses
    /// the ptr as the C call arg. Used by every internal
    /// `std::bytes::builder::__*` lowering that consumes a
    /// `handle: Int` arg.
    fn lower_bytes_builder_handle_arg(
        &mut self,
        arg: &Expr,
        scope: &Scope<'ctx>,
        diag_name: &str,
    ) -> Result<(BasicValueEnum<'ctx>, BasicValueEnum<'ctx>), CodegenError> {
        let (h_val, h_ty) = self.lower_expr(arg, scope)?;
        if h_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "{}: handle must be Int (the BytesBuilder locus's \
                 internal handle field), got {:?}",
                diag_name, h_ty
            )));
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let ptr = self
            .builder
            .build_int_to_ptr(h_val.into_int_value(), ptr_t, "bb.handle.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((h_val, ptr.into()))
    }

    fn lower_std_bytes_builder_append_scalar(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 4 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append_scalar takes 4 args \
                 (handle, value, width, big_endian), got {}",
                args.len()
            )));
        }
        let (_h, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__append_scalar",
        )?;
        let (value, v_ty) = self.lower_expr(&args[1], scope)?;
        if v_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "__append_scalar: value must be Int, got {:?}",
                v_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let (width, _) = self.lower_expr(&args[2], scope)?;
        let width_i32 = self
            .builder
            .build_int_truncate(width.into_int_value(), i32_t, "bb.width")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let (be, _) = self.lower_expr(&args[3], scope)?;
        let be_i32 = self
            .builder
            .build_int_truncate(be.into_int_value(), i32_t, "bb.be")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_append_scalar")
            .expect("lotus_bytes_builder_append_scalar declared");
        let r = self
            .builder
            .build_call(
                f,
                &[handle_ptr.into(), value.into(), width_i32.into(), be_i32.into()],
                "bb.append_scalar.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((r, CodegenTy::Int))
    }

    fn lower_std_bytes_builder_append_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        is_f32: bool,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "__append_f{} takes 3 args (handle, value, big_endian), got {}",
                if is_f32 { 32 } else { 64 },
                args.len()
            )));
        }
        let (_h, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__append_float",
        )?;
        let (value, v_ty) = self.lower_expr(&args[1], scope)?;
        if v_ty != CodegenTy::Float {
            return Err(CodegenError::Unsupported(format!(
                "__append_float: value must be Float, got {:?}",
                v_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        // Reinterpret the float's bits as an i64, zero-extended from
        // i32 for the f32 case, then append `width` low bytes.
        let (bits, width): (BasicValueEnum<'ctx>, u64) = if is_f32 {
            let f32v = self
                .builder
                .build_float_trunc(
                    value.into_float_value(),
                    self.context.f32_type(),
                    "bb.f32.trunc",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let bits32 = self
                .builder
                .build_bit_cast(f32v, i32_t, "bb.f32.bits")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let bits64 = self
                .builder
                .build_int_z_extend(bits32, i64_t, "bb.f32.bits64")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            (bits64.into(), 4)
        } else {
            let bits64 = self
                .builder
                .build_bit_cast(value, i64_t, "bb.f64.bits")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            (bits64, 8)
        };
        let (be, _) = self.lower_expr(&args[2], scope)?;
        let be_i32 = self
            .builder
            .build_int_truncate(be.into_int_value(), i32_t, "bb.be")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_append_scalar")
            .expect("lotus_bytes_builder_append_scalar declared");
        let r = self
            .builder
            .build_call(
                f,
                &[
                    handle_ptr.into(),
                    bits.into(),
                    i32_t.const_int(width, false).into(),
                    be_i32.into(),
                ],
                "bb.append_float.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((r, CodegenTy::Int))
    }

    fn lower_std_bytes_builder_append_pad(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append_pad takes 2 args \
                 (handle, to_align), got {}",
                args.len()
            )));
        }
        let (_h, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__append_pad",
        )?;
        let (to_align, ta_ty) = self.lower_expr(&args[1], scope)?;
        if ta_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "__append_pad: to_align must be Int, got {:?}",
                ta_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_builder_append_pad")
            .expect("lotus_bytes_builder_append_pad declared");
        let r = self
            .builder
            .build_call(
                f,
                &[handle_ptr.into(), to_align.into()],
                "bb.append_pad.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((r, CodegenTy::Int))
    }

    /// C10 (pond follow-up): `std::bytes::builder_append(b: Bytes,
    /// chunk: Bytes) -> Bytes`. Mutates the builder in place,
    /// returns the same handle so fluent chaining works
    /// (`let b2 = builder_append(b, chunk);`). The C side reads
    /// `chunk`'s `[i64 len]` prefix — no strlen, so embedded NULs
    /// are appended verbatim.
    fn lower_std_bytes_builder_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append takes 2 args (handle, chunk), got {}",
                args.len()
            )));
        }
        let (_h_int, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__append",
        )?;
        let (chunk_val, chunk_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(chunk_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append: chunk must be Bytes, got \
                 {:?} (use `std::bytes::from_string(s)` to convert)",
                chunk_ty
            )));
        }
        let chunk_val = self.unpack_view_if_needed(chunk_val, &chunk_ty)?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_append")
            .expect("lotus_bytes_builder_append declared");
        let call = self
            .builder
            .build_call(
                f,
                &[handle_ptr.into(), chunk_val.into()],
                "bb.append.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Returns i64 status: 1=ok, 0=fail (realloc NULL or null
        // handle). The BytesBuilder locus's `append` method checks
        // this and routes to `violate alloc_failed` on 0 per F.27.
        let status = call
            .try_as_basic_value()
            .left()
            .expect("returns i64 status");
        Ok((status, CodegenTy::Int))
    }

    /// C10 (pond follow-up): `std::bytes::builder_len(b: Bytes) ->
    /// Int`. Inspect the running byte count without materializing
    /// the final Bytes blob.
    fn lower_std_bytes_builder_len(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__len takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (_, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__len",
        )?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_len")
            .expect("lotus_bytes_builder_len declared");
        let call = self
            .builder
            .build_call(f, &[handle_ptr.into()], "bb.len.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// C10 (pond follow-up): `std::bytes::builder_finish(b: Bytes)
    /// -> Bytes`. Materializes the accumulated body as a
    /// `[i64 len][u8 data[len]]` blob in the bus payload arena
    /// (lives for the rest of the program) and frees the builder.
    /// The handle must NOT be reused after finish. No trailing NUL
    /// — Bytes is length-prefixed, so embedded NULs survive.
    fn lower_std_bytes_builder_finish(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__finish takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (_, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__finish",
        )?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_finish")
            .expect("lotus_bytes_builder_finish declared");
        // F.8 sweep — see lower_std_str_builder_finish for the
        // full rationale. The C-side routes through the TLS via
        // lotus_caller_or_global_bytes_create.
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[handle_ptr.into()], "bb.finish.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// Phase 0: `std::bytes::builder_shift_front(b: Bytes, n: Int) -> Bytes`.
    /// Drops the first n bytes in place via memmove; capacity
    /// preserved. Returns the same builder pointer so call-site
    /// `b = builder_shift_front(b, n)` and statement use both work.
    fn lower_std_bytes_builder_shift_front(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__shift_front takes 2 args (handle, n), got {}",
                args.len()
            )));
        }
        let (h_int, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__shift_front",
        )?;
        let (n_val, n_ty) = self.lower_expr(&args[1], scope)?;
        if n_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__shift_front: n must be Int, got {:?}",
                n_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_builder_shift_front")
            .expect("lotus_bytes_builder_shift_front declared");
        self.builder
            .build_call(f, &[handle_ptr.into(), n_val.into()], "bb.shift")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((h_int, CodegenTy::Int))
    }

    /// Phase 0: `std::bytes::builder_clear(b: Bytes) -> Bytes`.
    /// Sets len=0, capacity preserved.
    fn lower_std_bytes_builder_clear(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__clear takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (h_int, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__clear",
        )?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_clear")
            .expect("lotus_bytes_builder_clear declared");
        self.builder
            .build_call(f, &[handle_ptr.into()], "bb.clear")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((h_int, CodegenTy::Int))
    }

    /// Phase 0: `std::bytes::builder_snapshot(b: Bytes) -> Bytes`.
    /// Copies the builder's current `[0..len)` into a fresh
    /// length-prefixed Bytes blob in the bus payload arena.
    /// Builder unchanged. The returned blob is a regular Bytes
    /// value — `len()` / `at()` / `slice()` all work on it.
    fn lower_std_bytes_builder_snapshot(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__snapshot takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (_, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__snapshot",
        )?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_snapshot")
            .expect("lotus_bytes_builder_snapshot declared");
        let call = self
            .builder
            .build_call(f, &[handle_ptr.into()], "bb.snapshot.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// Phase 0: `std::bytes::builder_free(b: Bytes)`. Dispose the
    /// builder's malloc-backed buffer without materializing a
    /// final Bytes blob. Pair with `builder_new()` in a long-lived
    /// holder's `dissolve()` to close the recv-loop leak that
    /// occurs when `finish()` is never called.
    fn lower_std_bytes_builder_free(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__free takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (h_int, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__free",
        )?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_free")
            .expect("lotus_bytes_builder_free declared");
        self.builder
            .build_call(f, &[handle_ptr.into()], "bb.free")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((h_int, CodegenTy::Int))
    }

    /// Phase-2 (1): lower `std::bytes::builder::__view(handle: Int)
    /// -> Bytes`. Returns a non-owning Bytes pointer aliasing the
    /// builder's `[i64 len][u8 data]` region — zero allocation,
    /// zero copy. Lifetime is documented-and-trusted (no borrow
    /// checker at v1): valid until the next mutation on the source
    /// builder (append / shift_front / clear / finish).
    fn lower_std_bytes_builder_view(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__view takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (_, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__view",
        )?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_view")
            .expect("lotus_bytes_builder_view declared");
        let call = self
            .builder
            .build_call(f, &[handle_ptr.into()], "bb.view.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // F.30b view-ABI compaction: helper returns the 16-byte
        // view struct by value. Downstream sites that need to
        // pass it through `unpack_view_if_needed` to dereference
        // the underlying data still take a BasicValueEnum.
        let view_val = call
            .try_as_basic_value()
            .left()
            .expect("lotus_bytes_builder_view returns view struct");
        Ok((view_val, CodegenTy::BytesView))
    }

    /// Phase-3 Site 2: lower `std::bytes::builder::__text_view(
    /// handle: Int) -> String`. Returns a non-owning String
    /// pointer aliasing the builder's buffer; the builder
    /// maintains `buf[len] == '\0'` after every mutation so the
    /// returned C-string is well-formed for the lotus_str_*
    /// surface. Lifetime: valid until the next mutation on the
    /// source builder (documented-and-trusted at v1).
    fn lower_std_bytes_builder_text_view(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__text_view takes 1 arg (handle), got {}",
                args.len()
            )));
        }
        let (_, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__text_view",
        )?;
        let f = self
            .module
            .get_function("lotus_bytes_builder_text_view")
            .expect("lotus_bytes_builder_text_view declared");
        let call = self
            .builder
            .build_call(f, &[handle_ptr.into()], "bb.text_view.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // F.30b view-ABI compaction: helper returns the 16-byte
        // view struct by value; the C-string ptr is recomputed at
        // unpack time from the builder's `buf`.
        let view_val = call
            .try_as_basic_value()
            .left()
            .expect("lotus_bytes_builder_text_view returns view struct");
        Ok((view_val, CodegenTy::StringView))
    }

    /// Phase-3 Site 1: lower `std::bytes::builder::__append_slice(
    /// handle: Int, src: Bytes, lo: Int, hi: Int) -> Int`. Copies
    /// src[lo..hi) directly into the builder's tail. Returns 1=ok
    /// / 0=fail (null handle, out-of-range, realloc NULL). The
    /// stdlib wrapper routes 0 through `violate alloc_failed`.
    /// Eliminates the slice+append pair's intermediate Bytes
    /// wrapper that otherwise lands in g_bus_payload_arena.
    fn lower_std_bytes_builder_append_slice(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 4 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append_slice takes 4 args \
                 (handle, src, lo, hi), got {}",
                args.len()
            )));
        }
        let (_, handle_ptr) = self.lower_bytes_builder_handle_arg(
            &args[0],
            scope,
            "std::bytes::builder::__append_slice",
        )?;
        let (src_val, src_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(src_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append_slice: src must be Bytes, \
                 got {:?}",
                src_ty
            )));
        }
        let src_val = self.unpack_view_if_needed(src_val, &src_ty)?;
        let (lo_val, lo_ty) = self.lower_expr(&args[2], scope)?;
        if lo_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append_slice: lo must be Int, \
                 got {:?}",
                lo_ty
            )));
        }
        let (hi_val, hi_ty) = self.lower_expr(&args[3], scope)?;
        if hi_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::builder::__append_slice: hi must be Int, \
                 got {:?}",
                hi_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_builder_append_slice")
            .expect("lotus_bytes_builder_append_slice declared");
        let call = self
            .builder
            .build_call(
                f,
                &[
                    handle_ptr.into(),
                    src_val.into(),
                    lo_val.into(),
                    hi_val.into(),
                ],
                "bb.append_slice.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let status = call
            .try_as_basic_value()
            .left()
            .expect("returns i64 status");
        Ok((status, CodegenTy::Int))
    }

    /// F.27 discriminator: lower `std::bytes::__is_alloc_fail(b:
    /// Bytes) -> Int`. Returns 1 iff `b` is the alloc-fail
    /// sentinel returned by BytesBuilder snapshot()/finish() on
    /// payload-arena alloc failure. Success paths always allocate
    /// a fresh blob via lotus_bytes_create (even for len=0), so
    /// the sentinel is unambiguous. Used inside the BytesBuilder
    /// locus method bodies to gate the `violate alloc_failed`
    /// route.
    fn lower_std_bytes_is_alloc_fail(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::__is_alloc_fail takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::__is_alloc_fail: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_bytes_is_alloc_fail")
            .expect("lotus_bytes_is_alloc_fail declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "bb.is_alloc_fail.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let status = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((status, CodegenTy::Int))
    }

    /// F.30 (2026-05-20): `std::bytes::clone(v: BytesView) -> Bytes`.
    /// Deep-copies the view's contents into the caller's arena
    /// (via Task 8's TLS routing), returning an owned Bytes
    /// blob that outlives the source builder. This is the
    /// explicit upgrade path BytesView signals when storage
    /// sites reject the read-only coercion. Also accepts
    /// `Bytes` as a no-op deep copy (useful for callers that
    /// want to clone-from-a-borrowed-source generically).
    fn lower_std_bytes_clone(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::clone takes 1 arg (view), got {}",
                args.len()
            )));
        }
        let (v_val, v_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(v_ty, CodegenTy::BytesView | CodegenTy::Bytes) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::clone: arg must be BytesView or Bytes, got {:?}",
                v_ty
            )));
        }
        let v_val = self.unpack_view_if_needed(v_val, &v_ty)?;
        let arena = self.current_arena_ptr()?;
        let f = self
            .module
            .get_function("lotus_bytes_clone")
            .expect("lotus_bytes_clone declared");
        let call = self
            .builder
            .build_call(
                f,
                &[arena.into(), v_val.into()],
                "bytes.clone.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// Phase 2g: lower `std::bytes::from_string(s: String) -> Bytes`.
    /// strlen the source, allocate a Bytes blob of that length in
    /// the global payload arena, memcpy the body. Symmetric inverse
    /// of std::str::from_bytes.
    fn lower_std_bytes_from_string(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_string takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_string: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let f = self
            .module
            .get_function("lotus_bytes_from_str")
            .expect("lotus_bytes_from_str declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "bytes_from_str.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// Phase 2g: lower `std::bytes::at(b: Bytes, i: Int) -> Int`.
    /// Byte-as-Int accessor — returns the i-th byte's unsigned
    /// value (0..255) sign-extended into i64. Returns -1 if i is
    /// out of range. Pairs with std::bytes::slice and std::bytes::
    /// from_string for binary protocol parsing.
    fn lower_std_bytes_at(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at takes 2 args (b, i), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            let hint = if matches!(b_ty, CodegenTy::String) {
                " — use `std::bytes::from_string(s)` to convert"
            } else {
                ""
            };
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at: b must be Bytes, got {:?}{}",
                b_ty, hint
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let (i_val, i_ty) = self.lower_expr(&args[1], scope)?;
        if i_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at: i must be Int, got {:?}",
                i_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_at")
            .expect("lotus_bytes_at declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into(), i_val.into()], "bytes_at.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((ret, CodegenTy::Int))
    }

    /// Phase 2g: lower `std::bytes::slice(b: Bytes, lo: Int, hi: Int)
    /// -> Bytes`. Half-open range [lo, hi); out-of-range bounds
    /// clamp; hi <= lo yields an empty Bytes. The result is a copy
    /// (not a view) so it composes with deep-copy-shaped lifetime
    /// conventions.
    fn lower_std_bytes_slice(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice takes 3 args (b, lo, hi), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice: b must be Bytes, got {:?} \
                 (use `std::bytes::from_string(s)` to convert from String)",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let (lo_val, lo_ty) = self.lower_expr(&args[1], scope)?;
        if lo_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice: lo must be Int, got {:?}",
                lo_ty
            )));
        }
        let (hi_val, hi_ty) = self.lower_expr(&args[2], scope)?;
        if hi_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice: hi must be Int, got {:?}",
                hi_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_slice")
            .expect("lotus_bytes_slice declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(
                f,
                &[b_val.into(), lo_val.into(), hi_val.into()],
                "bytes_slice.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// ws-echo `bytes-construction-from-ints`: lower
    /// `std::bytes::from_int(v: Int) -> Bytes`. Builds a single-
    /// byte Bytes blob from the low 8 bits of `v`. Anchored in
    /// the program-lifetime payload arena, so the returned
    /// pointer matches recv_bytes / bytes_slice lifetime
    /// conventions and can flow through bus payloads without
    /// extra copying.
    fn lower_std_bytes_from_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_int takes 1 arg (v), got {}",
                args.len()
            )));
        }
        let (v_val, v_ty) = self.lower_expr(&args[0], scope)?;
        if v_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_int: v must be Int, got {:?}",
                v_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_from_int")
            .expect("lotus_bytes_from_int declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(f, &[v_val.into()], "bytes_from_int.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// ws-echo `bytes-construction-from-ints`: lower
    /// `std::bytes::concat(a: Bytes, b: Bytes) -> Bytes`.
    /// Returns a fresh Bytes containing `a` followed by `b`,
    /// allocated in the program-lifetime payload arena. With
    /// `from_int`, recursive concat composes any outbound
    /// byte sequence (WebSocket frame headers, length prefixes,
    /// custom binary protocols).
    fn lower_std_bytes_concat(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::concat takes 2 args (a, b), got {}",
                args.len()
            )));
        }
        let (a_val, a_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(a_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::concat: a must be Bytes, got {:?}",
                a_ty
            )));
        }
        let a_val = self.unpack_view_if_needed(a_val, &a_ty)?;
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::concat: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_bytes_concat")
            .expect("lotus_bytes_concat declared");
        self.emit_set_caller_arena()?;
        let call = self
            .builder
            .build_call(
                f,
                &[a_val.into(), b_val.into()],
                "bytes_concat.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

}
