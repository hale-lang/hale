//! `@form(...)` synthesized-method dispatchers: per-form method
//! call lowering for vec / hashmap / ring_buffer. Round 7b of the
//! codegen model-org refactor — corresponds to what the original
//! plan called Round 2 (form/) before that scope got absorbed into
//! Round 4's locus/instantiation.rs (where slot allocation lives).
//!
//! Lifted as inherent `impl<'ctx, 'p> Cx<'ctx, 'p>` blocks — call
//! sites need no `use` import.

pub(crate) mod bounded;

use hale_syntax::ast::Expr;
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{
    BceVecKey, CapacitySlotLayout, CodegenError, CodegenTy, Cx,
    FallibleCallResult, LocusInfo, Scope, SlotForm, TypeInfo,
};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// v1.x-FORM-2 PR6 (PR5 finale): inline-lower a synthesized
    /// @form(vec) fallible method (get, pop) as-if it were a
    /// fallible-ABI call. The C runtime returns 1=OK / 0=err;
    /// codegen inverts that to Hale's i1 path indicator
    /// (1=err / 0=ok) and writes a typed `IndexError` payload
    /// into the caller-provided out_err slot.
    ///
    /// **Performance shape (FORM-3, 2026-05-13):** the
    /// IndexError is constructed LAZILY in a dedicated err
    /// basic block — only when the operation fails — so the
    /// happy path pays no `arena_alloc` + payload-field stores.
    /// Earlier eager construction made `form_vec_get` 62× slower
    /// than hand-written C on the bench; lazy construction
    /// removes that overhead. The error semantics are the
    /// canonical `@form(vec)` contract
    /// (kind = "out_of_bounds" / "empty"; pop's err carries
    /// index=0, len=0; get's err carries index=i, len=current
    /// len at the bad access). `len` for get is read directly
    /// from the inline vec struct's `len` field via GEP — no
    /// function-call ABI on the hot path.
    pub(crate) fn try_lower_form_vec_fallible_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
        // BCE: canonical key of the receiver this call was made on
        // (`self` / `self.<field>`), or `None` for a non-self-rooted
        // receiver. Matched against the enclosing-loop registry so a
        // `V.get(loop_var)` over the same `V` skips its bounds check.
        recv_bce_key: Option<BceVecKey>,
    ) -> Result<Option<FallibleCallResult<'ctx>>, CodegenError> {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::Vec))
            .cloned()
        else {
            return Ok(None);
        };
        if !matches!(method_name, "get" | "set" | "pop") {
            return Ok(None);
        }
        let elem_ty = slot.elem_ty.clone();
        let payload_ty = CodegenTy::TypeRef("IndexError".to_string());

        let vec_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!(
                    "{}.__vec_{}.fallible.ptr",
                    locus_name, slot.name
                ),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // `set` is Unit-success — only get/pop need an out_val_slot.
        let out_val_slot_opt = if method_name == "set" {
            None
        } else {
            Some(self.alloca_for(
                &elem_ty,
                &format!("vec.{}.out_val.slot", method_name),
            )?)
        };
        let out_err_slot = self.alloca_for(
            &payload_ty,
            &format!("vec.{}.out_err.slot", method_name),
        )?;

        let llvm_elem_ty = self.llvm_basic_type(&elem_ty);
        // get/set/pop are now fully inlined (typed GEP + load/store),
        // so no `elem_size` (the byte-stride the C `lotus_vec_*` ABI
        // took) is needed here — the element stride is implicit in the
        // `llvm_elem_ty`-typed GEPs below.
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let zero_i32 = i32_t.const_int(0, false);

        let (c_ret, index_ssa) = match method_name {
            "get" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.get: expects 1 arg, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let (idx_val, idx_ty) = self.lower_expr(&args[0], scope)?;
                if idx_ty != CodegenTy::Int {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.get: index must be Int, got {:?}",
                        locus_name, idx_ty
                    )));
                }
                let idx_i64 = idx_val.into_int_value();
                // BCE (counted-loop bounds-check elimination): when
                // this get's index is the bare loop var of an
                // enclosing `for VAR in 0..V.len()` proven safe over
                // THIS same vec, `VAR < len(V)` holds by construction
                // — emit the branch-free load and a CONSTANT
                // `c_ret = 1`. The topmost (innermost) registered
                // frame whose `index_var` matches shadows outer
                // frames; BCE fires only if that frame is over this
                // exact vec. Downstream `is_err = (c_ret == 0)` folds
                // to const-false, so the lazy-IndexError block and any
                // enclosing `or` handler become statically dead and
                // LLVM removes them; the loop body vectorizes.
                let do_bce = matches!(&args[0], Expr::Ident(id)
                    if self
                        .bce_loops
                        .iter()
                        .rev()
                        .find(|f| f.index_var == id.name)
                        .map(|f| Some(&f.vec_key) == recv_bce_key.as_ref())
                        .unwrap_or(false));
                if do_bce {
                    let usize_t = self.usize_type();
                    let vec_struct_ty = self.context.struct_type(
                        &[usize_t.into(), usize_t.into(), ptr_t.into()],
                        false,
                    );
                    let buf_field_ptr = self
                        .builder
                        .build_struct_gep(
                            vec_struct_ty,
                            vec_field_ptr,
                            2,
                            &format!("{}.vec.get.bce.buf.ptr", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let buf = self
                        .builder
                        .build_load(
                            ptr_t,
                            buf_field_ptr,
                            &format!("{}.vec.get.bce.buf", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(
                                llvm_elem_ty,
                                buf,
                                &[idx_i64],
                                &format!(
                                    "{}.vec.get.bce.elem.ptr",
                                    locus_name
                                ),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let loaded = self
                        .builder
                        .build_load(
                            llvm_elem_ty,
                            elem_ptr,
                            &format!("{}.vec.get.bce.elem", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(out_val_slot_opt.unwrap(), loaded)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // The get cannot fail: c_ret = const 1.
                    (i32_t.const_int(1, false), idx_i64)
                } else {
                // FORM-vec hot-path inline (replaces the opaque
                // lotus_vec_get C call). The vec slot is the inline
                // struct `{ i64 cap, i64 len, ptr buf }`; on the
                // common (in-bounds) path this is a bounds-check +
                // GEP + typed load with no call. Produces the same
                // `c_ret` the downstream fallible/IndexError code
                // expects: 1 = ok, 0 = out-of-bounds (downstream
                // computes `is_err = (c_ret == 0)`). The unsigned
                // compare folds negative indices into the OOB path
                // exactly like the C `i < 0 || (size_t)i >= len`.
                let func = self
                    .current_fn
                    .expect("vec.get inside fn body");
                // The slot's cap/len are C `size_t` (i64 native,
                // i32 wasm32 — matching `usize_t`), so the struct
                // type and the len/cap loads MUST use `usize_t` or
                // the field offsets diverge from what the C runtime
                // (init / grow) wrote on wasm32.
                let usize_t = self.usize_type();
                let vec_struct_ty = self.context.struct_type(
                    &[usize_t.into(), usize_t.into(), ptr_t.into()],
                    false,
                );
                let len_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        1,
                        &format!("{}.vec.get.len.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let len_usize = self
                    .builder
                    .build_load(
                        usize_t,
                        len_field_ptr,
                        &format!("{}.vec.get.len", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                // Widen to i64 so the unsigned bounds-compare lines
                // up with the i64 index (no-op bitcast on native).
                let len_i64 = self
                    .builder
                    .build_int_z_extend_or_bit_cast(
                        len_usize,
                        i64_t,
                        &format!("{}.vec.get.len.i64", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let inbounds = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::ULT,
                        idx_i64,
                        len_i64,
                        &format!("{}.vec.get.inbounds", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let load_bb = self
                    .context
                    .append_basic_block(func, "vec.get.load");
                let oob_bb = self
                    .context
                    .append_basic_block(func, "vec.get.oob");
                let cont_bb = self
                    .context
                    .append_basic_block(func, "vec.get.cont");
                self.builder
                    .build_conditional_branch(inbounds, load_bb, oob_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // load_bb: buf[idx] -> out_val_slot, c_ret = 1
                self.builder.position_at_end(load_bb);
                let buf_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        2,
                        &format!("{}.vec.get.buf.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let buf = self
                    .builder
                    .build_load(
                        ptr_t,
                        buf_field_ptr,
                        &format!("{}.vec.get.buf", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(
                            llvm_elem_ty,
                            buf,
                            &[idx_i64],
                            &format!("{}.vec.get.elem.ptr", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                let loaded = self
                    .builder
                    .build_load(
                        llvm_elem_ty,
                        elem_ptr,
                        &format!("{}.vec.get.elem", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(out_val_slot_opt.unwrap(), loaded)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // oob_bb: nothing, fall through to cont with c_ret = 0
                self.builder.position_at_end(oob_bb);
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // cont_bb: c_ret = phi [1 from load, 0 from oob]
                self.builder.position_at_end(cont_bb);
                let one_i32 = i32_t.const_int(1, false);
                let cret_phi = self
                    .builder
                    .build_phi(i32_t, &format!("{}.vec.get.cret", locus_name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                cret_phi.add_incoming(&[
                    (&one_i32, load_bb),
                    (&zero_i32, oob_bb),
                ]);
                let c_ret = cret_phi.as_basic_value().into_int_value();
                (c_ret, idx_i64)
                }
            }
            "set" => {
                if args.len() != 2 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.set: expects 2 args (idx, value), got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let (idx_val, idx_ty) = self.lower_expr(&args[0], scope)?;
                if idx_ty != CodegenTy::Int {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.set: index must be Int, got {:?}",
                        locus_name, idx_ty
                    )));
                }
                let idx_i64 = idx_val.into_int_value();
                let (val, val_ty) = self.lower_expr(&args[1], scope)?;
                // A10 (G20): locus → interface coercion on set.
                let val = if let (
                    CodegenTy::Interface(iface),
                    CodegenTy::LocusRef(l),
                ) = (&elem_ty, &val_ty)
                {
                    self.coerce_to_interface(
                        val.into_pointer_value(),
                        l,
                        iface,
                    )?
                    .into()
                } else if val_ty != elem_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.set: value type mismatch: expected \
                         {:?}, got {:?}",
                        locus_name, elem_ty, val_ty
                    )));
                } else {
                    val
                };
                // Bus-arena reclaim follow-up (2026-05-21):
                // mirror the @form(vec).push deep-copy at the
                // set-from-method-body site. lotus_vec_set
                // memcpys the value into the slot, so any heap
                // fields inside `val` need to anchor in the
                // receiver's __arena instead of aliasing the
                // caller's per-method scratch.
                let ptr_t_for_arena =
                    self.context.ptr_type(AddressSpace::default());
                let dest_arena_field_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        locus_self_ptr,
                        info.arena_field_idx,
                        &format!("{}.__arena.for_set.ptr", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                let dest_arena = self
                    .builder
                    .build_load(
                        ptr_t_for_arena,
                        dest_arena_field_ptr,
                        &format!("{}.__arena.for_set", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .into_pointer_value();
                let val = self.emit_cross_arena_store_deep_copy_ptr(
                    val,
                    &elem_ty,
                    dest_arena,
                    &format!("{}.vec_set", locus_name),
                )?;
                // FORM-vec hot-path inline (replaces the opaque
                // lotus_vec_set C call). The vec slot is the inline
                // struct `{ usize cap, usize len, ptr buf }`. The
                // (already deep-copied) `val` SSA is stored straight
                // into `buf[idx]` on the in-bounds path — no call, no
                // alloca round-trip. Produces the same `c_ret` the
                // downstream fallible/IndexError code expects: 1 = ok,
                // 0 = out-of-bounds (downstream computes `is_err =
                // (c_ret == 0)`). The unsigned compare folds negative
                // indices into the OOB path exactly like the C
                // `i < 0 || (size_t)i >= len`.
                let func = self
                    .current_fn
                    .expect("vec.set inside fn body");
                // cap/len are C `size_t` (i64 native, i32 wasm32 —
                // matching `usize_t`); use that width so the field
                // offsets agree with the C runtime (init / grow) on
                // every target.
                let usize_t = self.usize_type();
                let vec_struct_ty = self.context.struct_type(
                    &[usize_t.into(), usize_t.into(), ptr_t.into()],
                    false,
                );
                let len_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        1,
                        &format!("{}.vec.set.len.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let len_usize = self
                    .builder
                    .build_load(
                        usize_t,
                        len_field_ptr,
                        &format!("{}.vec.set.len", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                // Widen to i64 so the unsigned bounds-compare lines up
                // with the i64 index (no-op bitcast on native).
                let len_i64 = self
                    .builder
                    .build_int_z_extend_or_bit_cast(
                        len_usize,
                        i64_t,
                        &format!("{}.vec.set.len.i64", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let inbounds = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::ULT,
                        idx_i64,
                        len_i64,
                        &format!("{}.vec.set.inbounds", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let store_bb = self
                    .context
                    .append_basic_block(func, "vec.set.store");
                let oob_bb = self
                    .context
                    .append_basic_block(func, "vec.set.oob");
                let cont_bb = self
                    .context
                    .append_basic_block(func, "vec.set.cont");
                self.builder
                    .build_conditional_branch(inbounds, store_bb, oob_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // store_bb: buf[idx] = val, c_ret = 1
                self.builder.position_at_end(store_bb);
                let buf_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        2,
                        &format!("{}.vec.set.buf.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let buf = self
                    .builder
                    .build_load(
                        ptr_t,
                        buf_field_ptr,
                        &format!("{}.vec.set.buf", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(
                            llvm_elem_ty,
                            buf,
                            &[idx_i64],
                            &format!("{}.vec.set.elem.ptr", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                self.builder
                    .build_store(elem_ptr, val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // oob_bb: nothing, fall through to cont with c_ret = 0
                self.builder.position_at_end(oob_bb);
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // cont_bb: c_ret = phi [1 from store, 0 from oob]
                self.builder.position_at_end(cont_bb);
                let one_i32 = i32_t.const_int(1, false);
                let cret_phi = self
                    .builder
                    .build_phi(i32_t, &format!("{}.vec.set.cret", locus_name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                cret_phi.add_incoming(&[
                    (&one_i32, store_bb),
                    (&zero_i32, oob_bb),
                ]);
                let c_ret = cret_phi.as_basic_value().into_int_value();
                (c_ret, idx_i64)
            }
            "pop" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.pop: takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                // FORM-vec hot-path inline (replaces the opaque
                // lotus_vec_pop C call). The vec slot is the inline
                // struct `{ usize cap, usize len, ptr buf }`. On the
                // non-empty path: decrement `len` (in usize width),
                // load `buf[new_len]` into the caller-provided out
                // slot — no call. Produces the same `c_ret` the
                // downstream code expects: 1 = ok, 0 = empty
                // (downstream computes `is_err = (c_ret == 0)`).
                let func = self
                    .current_fn
                    .expect("vec.pop inside fn body");
                // cap/len are C `size_t` (i64 native, i32 wasm32 —
                // matching `usize_t`); use that width so the field
                // offsets and the len decrement agree with the C
                // runtime on every target.
                let usize_t = self.usize_type();
                let vec_struct_ty = self.context.struct_type(
                    &[usize_t.into(), usize_t.into(), ptr_t.into()],
                    false,
                );
                let len_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        1,
                        &format!("{}.vec.pop.len.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let len_usize = self
                    .builder
                    .build_load(
                        usize_t,
                        len_field_ptr,
                        &format!("{}.vec.pop.len", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let nonempty = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        len_usize,
                        usize_t.const_int(0, false),
                        &format!("{}.vec.pop.nonempty", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let do_bb = self
                    .context
                    .append_basic_block(func, "vec.pop.do");
                let empty_bb = self
                    .context
                    .append_basic_block(func, "vec.pop.empty");
                let cont_bb = self
                    .context
                    .append_basic_block(func, "vec.pop.cont");
                self.builder
                    .build_conditional_branch(nonempty, do_bb, empty_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // do_bb: len -= 1 (usize); out = buf[new_len]; c_ret = 1
                self.builder.position_at_end(do_bb);
                let new_len_usize = self
                    .builder
                    .build_int_sub(
                        len_usize,
                        usize_t.const_int(1, false),
                        &format!("{}.vec.pop.new_len", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(len_field_ptr, new_len_usize)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                // GEP index can be any int width; widen new_len to i64
                // for a consistent index type (no-op on native).
                let new_len_i64 = self
                    .builder
                    .build_int_z_extend_or_bit_cast(
                        new_len_usize,
                        i64_t,
                        &format!("{}.vec.pop.new_len.i64", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let buf_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        2,
                        &format!("{}.vec.pop.buf.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let buf = self
                    .builder
                    .build_load(
                        ptr_t,
                        buf_field_ptr,
                        &format!("{}.vec.pop.buf", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(
                            llvm_elem_ty,
                            buf,
                            &[new_len_i64],
                            &format!("{}.vec.pop.elem.ptr", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                let popped = self
                    .builder
                    .build_load(
                        llvm_elem_ty,
                        elem_ptr,
                        &format!("{}.vec.pop.elem", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(out_val_slot_opt.unwrap(), popped)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // empty_bb: nothing, fall through to cont with c_ret = 0
                self.builder.position_at_end(empty_bb);
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // cont_bb: c_ret = phi [1 from do, 0 from empty]
                self.builder.position_at_end(cont_bb);
                let one_i32 = i32_t.const_int(1, false);
                let cret_phi = self
                    .builder
                    .build_phi(i32_t, &format!("{}.vec.pop.cret", locus_name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                cret_phi.add_incoming(&[
                    (&one_i32, do_bb),
                    (&zero_i32, empty_bb),
                ]);
                let c_ret = cret_phi.as_basic_value().into_int_value();
                let zero_i64 = i64_t.const_int(0, false);
                (c_ret, zero_i64)
            }
            _ => unreachable!("matched above"),
        };

        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                c_ret,
                zero_i32,
                &format!("{}.{}.is_err", locus_name, method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let kind_str = match method_name {
            "get" | "set" => "out_of_bounds",
            "pop" => "empty",
            _ => unreachable!(),
        };

        // FORM-3 (2026-05-13): lazy IndexError construction. The
        // happy path branches over the alloc + stores entirely.
        // Two consecutive cond_brs on `is_err` (here + the
        // enclosing `or` in `lower_or_expr`) collapse to one
        // under SimplifyCFG.
        let func = self
            .current_fn
            .expect("fallible-method call inside fn body");
        let lazy_err_bb = self.context.append_basic_block(
            func,
            &format!("vec.{}.lazy_err", method_name),
        );
        let join_bb = self.context.append_basic_block(
            func,
            &format!("vec.{}.lazy_join", method_name),
        );
        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(lazy_err_bb);
        let len_ssa: IntValue<'ctx> = match method_name {
            "get" | "set" => {
                // GEP into the inline vec struct's `len` field
                // (index 1 of `{ usize cap, usize len, ptr buf }`)
                // and load it directly — no function-call ABI. cap/len
                // are C `size_t` (i64 native, i32 wasm32 — matching
                // `usize_t`); use that width so the field offset and
                // load agree with the C runtime on every target, then
                // widen to i64 for the IndexError ctor.
                let usize_t = self.usize_type();
                let vec_struct_ty = self.context.struct_type(
                    &[usize_t.into(), usize_t.into(), ptr_t.into()],
                    false,
                );
                let len_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        1,
                        &format!("{}.vec.len.field.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let len_usize = self
                    .builder
                    .build_load(
                        usize_t,
                        len_field_ptr,
                        &format!("{}.vec.len.lazy", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                self.builder
                    .build_int_z_extend_or_bit_cast(
                        len_usize,
                        i64_t,
                        &format!("{}.vec.len.lazy.i64", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            "pop" => i64_t.const_int(0, false),
            _ => unreachable!(),
        };
        let ie_ptr = self.emit_index_error_alloc(
            kind_str,
            index_ssa,
            len_ssa,
        )?;
        self.builder
            .build_store(out_err_slot, ie_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);

        let success_ty = if method_name == "set" {
            None
        } else {
            Some(elem_ty)
        };
        Ok(Some(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: out_val_slot_opt,
            out_err_slot,
            success_ty,
            payload_ty,
        }))
    }

    /// v1.x-FORM-5: inline-lower a synthesized `@form(ring_buffer)`
    /// fallible method (`pop`). Same shape as
    /// `try_lower_form_vec_fallible_method` for `pop`: the C
    /// runtime returns i32 (1=OK / 0=empty), codegen inverts to
    /// i1 (1=err / 0=ok) and writes an `EmptyError { kind:
    /// "empty" }` payload lazily in the err basic block.
    pub(crate) fn try_lower_form_ring_buffer_fallible_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        _args: &[Expr],
        _scope: &Scope<'ctx>,
    ) -> Result<Option<FallibleCallResult<'ctx>>, CodegenError> {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::RingBuffer))
            .cloned()
        else {
            return Ok(None);
        };
        if method_name != "pop" {
            return Ok(None);
        }
        let elem_ty = slot.elem_ty.clone();
        let payload_ty = CodegenTy::TypeRef("EmptyError".to_string());

        let rb_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!(
                    "{}.__rb_{}.fallible.ptr",
                    locus_name, slot.name
                ),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let out_val_slot =
            self.alloca_for(&elem_ty, "rb.pop.out_val.slot")?;
        let out_err_slot =
            self.alloca_for(&payload_ty, "rb.pop.out_err.slot")?;

        let i32_t = self.context.i32_type();
        let zero_i32 = i32_t.const_int(0, false);

        let pop_fn = self
            .module
            .get_function("lotus_ring_buffer_pop")
            .expect("lotus_ring_buffer_pop declared");
        let c_ret = self
            .builder
            .build_call(
                pop_fn,
                &[rb_field_ptr.into(), out_val_slot.into()],
                &format!("{}.pop.call", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_ring_buffer_pop returns i32")
            .into_int_value();

        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                c_ret,
                zero_i32,
                &format!("{}.pop.is_err", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Lazy EmptyError construction on the err path.
        let func = self
            .current_fn
            .expect("fallible-method call inside fn body");
        let lazy_err_bb = self
            .context
            .append_basic_block(func, "rb.pop.lazy_err");
        let join_bb = self
            .context
            .append_basic_block(func, "rb.pop.lazy_join");
        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(lazy_err_bb);
        let ee_ptr = self.emit_empty_error_alloc("empty")?;
        self.builder
            .build_store(out_err_slot, ee_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);

        Ok(Some(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(elem_ty),
            payload_ty,
        }))
    }

    /// v1.x-FORM-4: inline-lower a synthesized `@form(hashmap)`
    /// fallible method (`get`, `remove`) as-if it were a
    /// fallible-ABI call. Parallel to
    /// `try_lower_form_vec_fallible_method` but with:
    ///   - the C ABI bakes key/value sizes at init time, so per-
    ///     call sites pass raw key/value pointers without size
    ///     args;
    ///   - the key is materialized into an alloca matching the
    ///     indexed-by field's type;
    ///   - `remove()` returns `() fallible(KeyError)` (Unit
    ///     success), so the FallibleCallResult carries
    ///     `success_ty = None` and no out_val_slot.
    pub(crate) fn try_lower_form_hashmap_fallible_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<FallibleCallResult<'ctx>>, CodegenError> {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::Hashmap))
            .cloned()
        else {
            return Ok(None);
        };
        if !matches!(method_name, "get" | "remove" | "key_at" | "entry_at") {
            return Ok(None);
        }

        // Look up the cell type + indexed-by field's CodegenTy
        // (codegen-side mirror of the typecheck synthesis at
        // resolve.rs:form_hashmap_value_and_key_ty).
        let cell_name = match &slot.elem_ty {
            CodegenTy::TypeRef(n) => n.clone(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "@form(hashmap) `{}`.{}: slot `{}` cell type must be a \
                     user-declared struct; got {:?}",
                    locus_name, method_name, slot.name, other
                )));
            }
        };
        let cell_info = self
            .user_types
            .get(&cell_name)
            .cloned()
            .ok_or_else(|| CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: cell type `{}` not registered",
                locus_name, method_name, cell_name
            )))?;
        let field_name = slot
            .indexed_by
            .as_ref()
            .ok_or_else(|| CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: slot `{}` missing indexed_by",
                locus_name, method_name, slot.name
            )))?
            .clone();
        let (_field_idx, key_codegen_ty) = cell_info
            .fields
            .get(&field_name)
            .cloned()
            .ok_or_else(|| CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: indexed-by field `{}` not on \
                 cell `{}`",
                locus_name, method_name, field_name, cell_name
            )))?;
        if !matches!(key_codegen_ty, CodegenTy::Int | CodegenTy::String) {
            return Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: key type {:?} unsupported; v1 \
                 supports Int and String keys only",
                locus_name, method_name, key_codegen_ty
            )));
        }
        let value_codegen_ty = CodegenTy::TypeRef(cell_name.clone());

        let hashmap_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!(
                    "{}.__hashmap_{}.fallible.ptr",
                    locus_name, slot.name
                ),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // 2026-05-16: key_at(i) / entry_at(i) — index-based
        // iteration. Takes Int, fails with IndexError. Walks the
        // hashmap slots in hash-table order (O(cap) per call;
        // O(len * cap) for a full sweep — fine at small/medium
        // scale, agents iterating 100k+ entries should populate a
        // parallel @form(vec) instead).
        if matches!(method_name, "key_at" | "entry_at") {
            return self
                .lower_form_hashmap_index_method(
                    info,
                    &slot,
                    &cell_info,
                    cell_name.clone(),
                    key_codegen_ty.clone(),
                    value_codegen_ty.clone(),
                    locus_name,
                    method_name,
                    args,
                    scope,
                    hashmap_field_ptr,
                )
                .map(Some);
        }

        let payload_ty = CodegenTy::TypeRef("KeyError".to_string());

        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: expects 1 arg, got {}",
                locus_name,
                method_name,
                args.len()
            )));
        }
        let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
        if key_ty != key_codegen_ty {
            return Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: key arg type mismatch: expected \
                 {:?}, got {:?}",
                locus_name, method_name, key_codegen_ty, key_ty
            )));
        }
        let key_alloca = self
            .alloca_for(&key_codegen_ty, &format!("hm.{}.key.slot", method_name))?;
        self.builder
            .build_store(key_alloca, key_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let out_err_slot = self.alloca_for(
            &payload_ty,
            &format!("hm.{}.out_err.slot", method_name),
        )?;
        let i32_t = self.context.i32_type();
        let zero_i32 = i32_t.const_int(0, false);

        let (c_ret, out_val_slot_opt, success_ty_opt) = match method_name {
            "get" => {
                // `out_val_slot` holds the surface-level success
                // value, which for a TypeRef cell is a *pointer
                // to* the struct (matching how `lower_or_expr`
                // loads it via `llvm_basic_type(TypeRef) = ptr`).
                // Arena-allocate a fresh buffer for the runtime
                // to memcpy `value_size` bytes into, then store
                // its pointer in the surface slot.
                let cell_struct_size = cell_info
                    .struct_ty
                    .size_of()
                    .expect("cell struct has known size");
                let value_buf_ptr = self.arena_alloc(
                    cell_struct_size,
                    "hm.get.value_buf",
                )?;
                let out_val_slot = self.alloca_for(
                    &value_codegen_ty,
                    "hm.get.out_val.slot",
                )?;
                self.builder
                    .build_store(out_val_slot, value_buf_ptr)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let get_fn = self
                    .module
                    .get_function("lotus_hashmap_get")
                    .expect("lotus_hashmap_get declared");
                let c_ret = self
                    .builder
                    .build_call(
                        get_fn,
                        &[
                            hashmap_field_ptr.into(),
                            key_alloca.into(),
                            value_buf_ptr.into(),
                        ],
                        &format!("{}.get.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_hashmap_get returns i32")
                    .into_int_value();
                (c_ret, Some(out_val_slot), Some(value_codegen_ty.clone()))
            }
            "remove" => {
                let remove_fn = self
                    .module
                    .get_function("lotus_hashmap_remove")
                    .expect("lotus_hashmap_remove declared");
                let c_ret = self
                    .builder
                    .build_call(
                        remove_fn,
                        &[hashmap_field_ptr.into(), key_alloca.into()],
                        &format!("{}.remove.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_hashmap_remove returns i32")
                    .into_int_value();
                (c_ret, None, None)
            }
            _ => unreachable!("matched above"),
        };

        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                c_ret,
                zero_i32,
                &format!("{}.{}.is_err", locus_name, method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // FORM-3 (2026-05-13): lazy KeyError construction. The
        // happy path branches over the arena_alloc + store
        // entirely — same pattern as the vec.get/pop fallible
        // shape above. Two consecutive cond_brs on `is_err`
        // (here + the enclosing `or` in `lower_or_expr`) collapse
        // to one under SimplifyCFG.
        //
        // Note: `hm.get`'s value-buffer arena_alloc (above) is
        // still eager because the C ABI takes the buffer pointer
        // at call time and the buffer must outlive the call so
        // the caller can read fields off it. That's a separate
        // optimization (e.g. caller-frame alloca + memcpy-on-bind)
        // beyond this fix.
        let func = self
            .current_fn
            .expect("fallible-method call inside fn body");
        let lazy_err_bb = self.context.append_basic_block(
            func,
            &format!("hm.{}.lazy_err", method_name),
        );
        let join_bb = self.context.append_basic_block(
            func,
            &format!("hm.{}.lazy_join", method_name),
        );
        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(lazy_err_bb);
        let ke_ptr = self.emit_key_error_alloc("missing_key")?;
        self.builder
            .build_store(out_err_slot, ke_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);

        Ok(Some(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: out_val_slot_opt,
            out_err_slot,
            success_ty: success_ty_opt,
            payload_ty,
        }))
    }

    pub(crate) fn lower_form_hashmap_index_method(
        &mut self,
        _info: &LocusInfo<'ctx>,
        _slot: &CapacitySlotLayout,
        cell_info: &TypeInfo<'ctx>,
        _cell_name: String,
        key_codegen_ty: CodegenTy,
        value_codegen_ty: CodegenTy,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
        hashmap_field_ptr: PointerValue<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: expects 1 arg (index), got {}",
                locus_name,
                method_name,
                args.len()
            )));
        }
        let (idx_val, idx_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(idx_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: index arg must be Int, got {:?}",
                locus_name, method_name, idx_ty
            )));
        }
        let idx_int = idx_val.into_int_value();
        let payload_ty = CodegenTy::TypeRef("IndexError".to_string());
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();

        // Output slot type + buffer: key_at writes the raw key
        // (Int = 8 bytes, String = ptr-to-cstr = 8 bytes); entry_at
        // writes the full cell struct.
        let (success_ty, out_val_slot, out_buf_ptr, c_fn_name) = match method_name {
            "key_at" => {
                let buf_size = self
                    .llvm_basic_type(&key_codegen_ty)
                    .size_of()
                    .expect("key type has known size");
                let buf_ptr = self.arena_alloc(buf_size, "hm.key_at.buf")?;
                let slot = self.alloca_for(&key_codegen_ty, "hm.key_at.out")?;
                // For pointer-shaped keys (String), the C primitive
                // memcpys the 8-byte pointer into buf_ptr; the
                // surface value is whatever's loaded from buf_ptr.
                // For Int, same — buf holds the i64 directly.
                (key_codegen_ty.clone(), slot, buf_ptr, "lotus_hashmap_key_at")
            }
            "entry_at" => {
                let cell_size = cell_info
                    .struct_ty
                    .size_of()
                    .expect("cell struct has known size");
                let buf_ptr = self.arena_alloc(cell_size, "hm.entry_at.buf")?;
                let slot = self.alloca_for(&value_codegen_ty, "hm.entry_at.out")?;
                // TypeRef cells are pointer-shaped at the surface;
                // store buf_ptr in the alloca so the load path sees
                // a pointer to the struct.
                self.builder
                    .build_store(slot, buf_ptr)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                (value_codegen_ty.clone(), slot, buf_ptr, "lotus_hashmap_value_at")
            }
            _ => unreachable!("matched by caller"),
        };

        let c_fn = self
            .module
            .get_function(c_fn_name)
            .expect("hashmap index primitive declared");
        let c_ret = self
            .builder
            .build_call(
                c_fn,
                &[
                    hashmap_field_ptr.into(),
                    idx_int.into(),
                    out_buf_ptr.into(),
                ],
                &format!("{}.{}.call", locus_name, method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("hashmap index primitive returns i32")
            .into_int_value();

        // For key_at, we need to materialize the surface value
        // from the buffer (the C primitive memcpyd into it but
        // the alloca'd surface slot is still uninitialized).
        if method_name == "key_at" {
            let loaded = self
                .builder
                .build_load(self.llvm_basic_type(&key_codegen_ty), out_buf_ptr, "key_at.val")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(out_val_slot, loaded)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                c_ret,
                i32_t.const_int(0, false),
                &format!("{}.{}.is_err", locus_name, method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Lazy IndexError emission on the miss side. The C primitive
        // already knows the table's len (we don't pass it back), so
        // populate IndexError.len with a lotus_hashmap_len call only
        // on the err path. Use the supplied index as IndexError.index.
        let func = self
            .current_fn
            .expect("hashmap index method inside fn body");
        let lazy_err_bb = self.context.append_basic_block(
            func,
            &format!("hm.{}.lazy_err", method_name),
        );
        let join_bb = self.context.append_basic_block(
            func,
            &format!("hm.{}.lazy_join", method_name),
        );
        let out_err_slot = self.alloca_for(
            &payload_ty,
            &format!("hm.{}.out_err.slot", method_name),
        )?;
        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(lazy_err_bb);
        let len_fn = self
            .module
            .get_function("lotus_hashmap_len")
            .expect("lotus_hashmap_len declared");
        let len_val = self
            .builder
            .build_call(
                len_fn,
                &[hashmap_field_ptr.into()],
                &format!("{}.{}.err.len", locus_name, method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_hashmap_len returns i64")
            .into_int_value();
        let ie_ptr = self.emit_index_error_alloc(
            "out_of_bounds",
            idx_int,
            len_val,
        )?;
        self.builder
            .build_store(out_err_slot, ie_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);

        let _ = i64_t;
        Ok(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(success_ty),
            payload_ty,
        })
    }

    /// v1.x-FORM-2 PR5: dispatch synthesized `@form(vec)` methods.
    /// Receiver must be a `@form(vec)` locus instance (or `self`
    /// inside such a locus). Routes to the `lotus_vec_*` C runtime
    /// on the inline `{ cap, len, buf }` slot.
    ///
    /// Returns:
    ///   Ok(None)        — receiver is not a form-vec locus, fall through
    ///   Ok(Some(None))  — handled, void return (push)
    ///   Ok(Some(Some))  — handled, value return (len, is_empty)
    ///
    /// `get` and `pop` are fallible; they land in PR6 once the
    /// sret + i1 ABI is wired. Until then this helper emits a
    /// "PR6 pending" diagnostic for those names.
    pub(crate) fn try_lower_form_vec_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<
        Option<Option<(BasicValueEnum<'ctx>, CodegenTy)>>,
        CodegenError,
    > {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::Vec))
            .cloned()
        else {
            return Ok(None);
        };
        let is_synth = matches!(
            method_name,
            "push" | "get" | "pop" | "len" | "is_empty"
            | "sort" | "sort_by" | "sort_desc_by"
        );
        if !is_synth {
            return Ok(None);
        }

        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let vec_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!("{}.__vec_{}.ptr", locus_name, slot.name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        match method_name {
            "push" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.push: expects 1 arg, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let (arg_val, arg_ty) =
                    self.lower_expr(&args[0], scope)?;
                // A10 (G20): if the cell type is an interface and
                // the arg is a concrete locus that satisfies it,
                // coerce locus → interface here the same way
                // `lower_user_fn_call` does. The cell stores the
                // 8-byte ptr to the fat-pointer struct (data +
                // vtable), arena-allocated.
                let arg_val = if let (
                    CodegenTy::Interface(iface),
                    CodegenTy::LocusRef(l),
                ) = (&slot.elem_ty, &arg_ty)
                {
                    self.coerce_to_interface(
                        arg_val.into_pointer_value(),
                        l,
                        iface,
                    )?
                    .into()
                } else if arg_ty != slot.elem_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.push arg type mismatch: \
                         expected {:?}, got {:?}",
                        locus_name, slot.elem_ty, arg_ty
                    )));
                } else {
                    arg_val
                };
                // Bus-arena reclaim (2026-05-21) follow-up:
                // when the push happens from a method body whose
                // scratch is destroyed at method-exit, the
                // freshly-built struct literal (arg_val) lives in
                // scratch. lotus_vec_push memcpys the struct
                // bytes into the vec's buffer in the receiver
                // locus's arena — but if the struct's fields are
                // heap pointers (String, Bytes, nested struct,
                // ...) those pointers still aim at scratch and
                // dangle after method exit. Deep-copy the elem
                // into the receiver's __arena BEFORE the push so
                // the heap fields are anchored in the vec's
                // owning arena, surviving every caller's scratch
                // destroy. Scalars / view types / loci /
                // fn-pointers pass through identically via
                // `emit_return_value_deep_copy`. For pushes from
                // main (scratch_active is false), the dest arena
                // is the same as the source — the deep-copy is a
                // wasted memcpy (matching the same trade-off
                // accepted for free-fn epilogues post the
                // Bytes-arm fix). Catches the regression pinned
                // by tests/cross_locus_from_method.rs.
                let dest_arena_field_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        locus_self_ptr,
                        info.arena_field_idx,
                        &format!("{}.__arena.for_push.ptr", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                let dest_arena = self
                    .builder
                    .build_load(
                        ptr_t,
                        dest_arena_field_ptr,
                        &format!("{}.__arena.for_push", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .into_pointer_value();
                let arg_val = self.emit_cross_arena_store_deep_copy_ptr(
                    arg_val,
                    &slot.elem_ty,
                    dest_arena,
                    &format!("{}.vec_push", locus_name),
                )?;
                // Materialize the (now arena-anchored) arg in an
                // alloca so we can hand its address to
                // lotus_vec_push. The runtime memcpys elem_size
                // bytes from this address. Entry-block hoist so
                // a push call in a loop body doesn't grow the
                // frame per iteration (1M pushes × 8 bytes = 8
                // MB → SIGSEGV).
                let llvm_elem_ty =
                    self.llvm_basic_type(&slot.elem_ty);
                let arg_alloca = self.alloca_in_entry(
                    llvm_elem_ty,
                    &format!("{}.push.arg", locus_name),
                )?;
                self.builder
                    .build_store(arg_alloca, arg_val)
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                let elem_size = self.size_to_usize(
                    llvm_elem_ty.size_of().expect("cell type has known size"),
                )?;
                // FORM-vec hot-path inline (replaces the opaque
                // lotus_vec_push C call). The vec slot is the inline
                // struct `{ i64 cap, i64 len, ptr buf }`. When
                // `len < cap` we have room: store the (already
                // arena-anchored) element into `buf[len]` and bump
                // `len` — no call, no memcpy machinery. Only the
                // grow case (len == cap) keeps the cold C call,
                // which reallocs, stores, and bumps len itself. The
                // deep-copy above ran on both paths, so heap fields
                // are anchored either way.
                let func = self
                    .current_fn
                    .expect("vec.push inside fn body");
                // cap/len are C `size_t` (i64 native, i32 wasm32 —
                // matching `usize_t`); use that width so the field
                // offsets and the len bump agree with the C runtime
                // (init / grow) on every target.
                let usize_t = self.usize_type();
                let vec_struct_ty = self.context.struct_type(
                    &[usize_t.into(), usize_t.into(), ptr_t.into()],
                    false,
                );
                let len_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        1,
                        &format!("{}.vec.push.len.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let len_usize = self
                    .builder
                    .build_load(
                        usize_t,
                        len_field_ptr,
                        &format!("{}.vec.push.len", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let cap_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        0,
                        &format!("{}.vec.push.cap.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let cap_usize = self
                    .builder
                    .build_load(
                        usize_t,
                        cap_field_ptr,
                        &format!("{}.vec.push.cap", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let has_room = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::ULT,
                        len_usize,
                        cap_usize,
                        &format!("{}.vec.push.has_room", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let fast_bb = self
                    .context
                    .append_basic_block(func, "vec.push.fast");
                let grow_bb = self
                    .context
                    .append_basic_block(func, "vec.push.grow");
                let cont_bb = self
                    .context
                    .append_basic_block(func, "vec.push.cont");
                self.builder
                    .build_conditional_branch(has_room, fast_bb, grow_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // fast_bb: buf[len] = *arg_alloca; len += 1
                self.builder.position_at_end(fast_bb);
                let buf_field_ptr = self
                    .builder
                    .build_struct_gep(
                        vec_struct_ty,
                        vec_field_ptr,
                        2,
                        &format!("{}.vec.push.buf.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let buf = self
                    .builder
                    .build_load(
                        ptr_t,
                        buf_field_ptr,
                        &format!("{}.vec.push.buf", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                let dst = unsafe {
                    self.builder
                        .build_gep(
                            llvm_elem_ty,
                            buf,
                            &[len_usize],
                            &format!("{}.vec.push.dst", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                // Typed whole-value copy of the element. Equivalent
                // to the C path's `memcpy(buf+len*es, elem, es)` for
                // any elem type (scalar, ptr, struct); LLVM lowers an
                // aggregate load/store to the right move.
                let elem_val = self
                    .builder
                    .build_load(
                        llvm_elem_ty,
                        arg_alloca,
                        &format!("{}.vec.push.elem", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(dst, elem_val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let new_len = self
                    .builder
                    .build_int_add(
                        len_usize,
                        usize_t.const_int(1, false),
                        &format!("{}.vec.push.new_len", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(len_field_ptr, new_len)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // grow_bb: cold path — realloc + store + len++ in C.
                self.builder.position_at_end(grow_bb);
                let push_fn = self
                    .module
                    .get_function("lotus_vec_push")
                    .expect("lotus_vec_push extern declared");
                self.builder
                    .build_call(
                        push_fn,
                        &[
                            vec_field_ptr.into(),
                            elem_size.into(),
                            arg_alloca.into(),
                        ],
                        &format!("{}.push.call", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(cont_bb);
                Ok(Some(None))
            }
            "len" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.len: takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let len_fn = self
                    .module
                    .get_function("lotus_vec_len")
                    .expect("lotus_vec_len extern declared");
                let result = self
                    .builder
                    .build_call(
                        len_fn,
                        &[vec_field_ptr.into()],
                        &format!("{}.len.call", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_vec_len returns i64");
                let _ = i64_t;
                Ok(Some(Some((result, CodegenTy::Int))))
            }
            "is_empty" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.is_empty: takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let is_empty_fn = self
                    .module
                    .get_function("lotus_vec_is_empty")
                    .expect("lotus_vec_is_empty extern declared");
                let result_i32 = self
                    .builder
                    .build_call(
                        is_empty_fn,
                        &[vec_field_ptr.into()],
                        &format!("{}.is_empty.call", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_vec_is_empty returns i32")
                    .into_int_value();
                // Convert C i32 (1=true, 0=false) to Hale i1 bool.
                let zero = i32_t.const_int(0, false);
                let result_i1 = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        result_i32,
                        zero,
                        &format!("{}.is_empty.bool", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                let _ = ptr_t;
                Ok(Some(Some((result_i1.into(), CodegenTy::Bool))))
            }
            "sort" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.sort takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let sort_fn_name = match slot.elem_ty {
                    CodegenTy::Int => "lotus_vec_sort_int",
                    CodegenTy::Float => "lotus_vec_sort_float",
                    CodegenTy::String => "lotus_vec_sort_string",
                    ref other => {
                        return Err(CodegenError::Unsupported(format!(
                            "@form(vec) `{}`.sort: cell type {:?} has no \
                             default ordering — use `sort_by(less_than)` \
                             with a user comparator instead.",
                            locus_name, other
                        )));
                    }
                };
                let sort_fn = self
                    .module
                    .get_function(sort_fn_name)
                    .expect("lotus_vec_sort_<prim> declared");
                self.builder
                    .build_call(
                        sort_fn,
                        &[vec_field_ptr.into()],
                        &format!("{}.sort.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(None))
            }
            "sort_by" | "sort_desc_by" => {
                let reverse = method_name == "sort_desc_by";
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.{}: expects 1 arg (cmp), got {}",
                        locus_name,
                        method_name,
                        args.len()
                    )));
                }
                let (cmp_val, cmp_ty) = self.lower_expr(&args[0], scope)?;
                let (cmp_params, cmp_ret) = match &cmp_ty {
                    CodegenTy::FnPtr { args: params, ret } => {
                        (params.clone(), ret.clone())
                    }
                    other => {
                        return Err(CodegenError::Unsupported(format!(
                            "@form(vec) `{}`.{}: cmp must be a fn-pointer \
                             `fn(T, T) -> Bool`, got {:?}",
                            locus_name, method_name, other
                        )));
                    }
                };
                if cmp_params.len() != 2
                    || cmp_params[0] != slot.elem_ty
                    || cmp_params[1] != slot.elem_ty
                {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.{}: cmp parameter types must \
                         both be {:?}, got {:?}",
                        locus_name, method_name, slot.elem_ty, cmp_params
                    )));
                }
                if cmp_ret.as_deref() != Some(&CodegenTy::Bool) {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(vec) `{}`.{}: cmp must return Bool, got {:?}",
                        locus_name, method_name, cmp_ret
                    )));
                }
                let cmp_fn_ptr = cmp_val.into_pointer_value();
                let tramp_fn = self.emit_or_get_sort_trampoline(
                    &slot.elem_ty,
                    reverse,
                )?;
                // Cookie: { arena: ptr, cmp: ptr }. Hoist to the
                // entry block — a raw build_alloca at the call site
                // would leak ~16 bytes/iter when sort_by runs inside
                // a hot loop (same pattern the cliff-lift session
                // fixed for locus instantiation).
                let cookie_ty = self.context.struct_type(
                    &[ptr_t.into(), ptr_t.into()],
                    false,
                );
                let cookie_alloca = self.alloca_in_entry(
                    cookie_ty.into(),
                    "sort.cookie",
                )?;
                let arena_at_call = self.current_arena_ptr()?;
                let arena_field = self
                    .builder
                    .build_struct_gep(cookie_ty, cookie_alloca, 0, "cookie.arena.ptr")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(arena_field, arena_at_call)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let cmp_field = self
                    .builder
                    .build_struct_gep(cookie_ty, cookie_alloca, 1, "cookie.cmp.ptr")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cmp_field, cmp_fn_ptr)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let llvm_elem_ty = self.llvm_basic_type(&slot.elem_ty);
                let elem_size = self.size_to_usize(
                    llvm_elem_ty.size_of().expect("cell type has known size"),
                )?;
                let sort_by_fn = self
                    .module
                    .get_function("lotus_vec_sort_by")
                    .expect("lotus_vec_sort_by declared");
                let tramp_ptr = tramp_fn.as_global_value().as_pointer_value();
                self.builder
                    .build_call(
                        sort_by_fn,
                        &[
                            vec_field_ptr.into(),
                            elem_size.into(),
                            tramp_ptr.into(),
                            cookie_alloca.into(),
                        ],
                        &format!("{}.{}.call", locus_name, method_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(None))
            }
            "get" | "pop" => Err(CodegenError::Unsupported(format!(
                "@form(vec) `{}`.{}: fallible method must be addressed via \
                 `or raise` / `or <expr>` / `or handler(err)`",
                locus_name, method_name
            ))),
            _ => unreachable!("is_synth guard"),
        }
    }

    /// v1.x-FORM-4: dispatcher for the infallible synth methods on
    /// `@form(hashmap)` loci — `set`, `has`, `len`, `is_empty`.
    /// `get` and `remove` are fallible and handled by
    /// `try_lower_form_hashmap_fallible_method`.
    ///
    /// Returns `Ok(None)` if the receiver isn't a hashmap-form
    /// locus or the method name isn't a synth. Otherwise
    /// `Ok(Some(...))` with the inner result: `Some` for
    /// value-producing methods, `None` for Unit-returning ones.
    /// Errors only for genuine codegen problems (arg-arity / type
    /// mismatch).
    pub(crate) fn try_lower_form_hashmap_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<
        Option<Option<(BasicValueEnum<'ctx>, CodegenTy)>>,
        CodegenError,
    > {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::Hashmap))
            .cloned()
        else {
            return Ok(None);
        };
        let is_synth = matches!(
            method_name,
            "set" | "has" | "len" | "is_empty" | "get" | "remove"
            | "key_at" | "entry_at" | "bump"
        );
        if !is_synth {
            return Ok(None);
        }

        let i32_t = self.context.i32_type();
        let hashmap_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!("{}.__hashmap_{}.ptr", locus_name, slot.name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        match method_name {
            "set" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.set: expects 1 arg, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                // Cell type + indexed-by field — needed to GEP the
                // key out of the value at this call site.
                let cell_name = match &slot.elem_ty {
                    CodegenTy::TypeRef(n) => n.clone(),
                    other => {
                        return Err(CodegenError::Unsupported(format!(
                            "@form(hashmap) `{}`.set: slot cell type must \
                             be a user-declared struct; got {:?}",
                            locus_name, other
                        )));
                    }
                };
                let cell_info = self
                    .user_types
                    .get(&cell_name)
                    .cloned()
                    .ok_or_else(|| CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.set: cell type `{}` \
                         unregistered",
                        locus_name, cell_name
                    )))?;
                let field_name = slot
                    .indexed_by
                    .as_ref()
                    .ok_or_else(|| CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.set: slot missing indexed_by",
                        locus_name
                    )))?
                    .clone();
                let (key_field_idx, _key_ty) = cell_info
                    .fields
                    .get(&field_name)
                    .cloned()
                    .ok_or_else(|| CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.set: indexed-by field `{}` \
                         not on cell `{}`",
                        locus_name, field_name, cell_name
                    )))?;

                let (arg_val, arg_ty) =
                    self.lower_expr(&args[0], scope)?;
                let expected_value_ty = CodegenTy::TypeRef(cell_name.clone());
                if arg_ty != expected_value_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.set arg type mismatch: \
                         expected {:?}, got {:?}",
                        locus_name, expected_value_ty, arg_ty
                    )));
                }

                // Bus-arena reclaim follow-up (2026-05-21): when
                // set() runs from inside a method body, the struct
                // literal lives in the caller's per-call scratch.
                // hashmap_set memcpys the struct bytes into the
                // slot — but heap-pointer fields (String / Bytes
                // / TypeRef / Tuple / Array) still alias scratch
                // and dangle on method exit. Deep-copy into the
                // receiver locus's __arena BEFORE the set so
                // those fields anchor where the map lives. Same
                // shape as the @form(vec).push deep-copy from
                // 5300071.
                let dest_arena_field_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        locus_self_ptr,
                        info.arena_field_idx,
                        &format!("{}.__arena.for_set.ptr", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                let ptr_t_for_arena =
                    self.context.ptr_type(AddressSpace::default());
                let dest_arena = self
                    .builder
                    .build_load(
                        ptr_t_for_arena,
                        dest_arena_field_ptr,
                        &format!("{}.__arena.for_set", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .into_pointer_value();
                // Cell single-owner (2026-07-18): the walk's
                // String/Bytes leaves must not skip-share a
                // same-arena pointer into the cell — a value read
                // out of another cell (or a self-storage field)
                // would alias it, and anchor retirement frees
                // aliased blobs (UAF) while in-place self-field
                // overwrites mutate them. Toggle the owned-clone
                // variants for the duration of this walk.
                // Cell single-owner, part 2 (2026-07-18): walk a
                // stack SNAPSHOT of the value struct, not the
                // source. The anchor walk rewrites heap-pointer
                // fields in place; when the source is a
                // self-storage struct (`m.set(self.rec)`), the
                // store-back would re-point the locus field at the
                // very blob the cell is about to own — the two
                // alias again (in-place field overwrites then
                // mutate the cell, and a later retire of either
                // dangles the other). Snapshotting first leaves the
                // source untouched; entry-block alloca so a hot
                // loop reuses one slot.
                let snap = self.alloca_in_entry(
                    cell_info.struct_ty.into(),
                    &format!("{}.set.snap", locus_name),
                )?;
                let snap_size = cell_info
                    .struct_ty
                    .size_of()
                    .expect("cell struct has known size");
                self.builder
                    .build_memcpy(
                        snap,
                        8,
                        arg_val.into_pointer_value(),
                        8,
                        snap_size,
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let arg_val: BasicValueEnum = snap.into();
                let prev_owned = self.cell_owned_clone;
                self.cell_owned_clone = true;
                let walk_res = self.emit_cross_arena_store_deep_copy(
                    arg_val,
                    &expected_value_ty,
                    dest_arena,
                    &format!("{}.hashmap_set", locus_name),
                );
                self.cell_owned_clone = prev_owned;
                let arg_val = walk_res?;
                // The value lowered to a TypeRef arrives as a
                // pointer to the struct (user_type instantiations
                // return `*StructTy`). Pass it directly as
                // value_ptr; GEP the indexed-by field through it
                // to derive the key_ptr.
                let value_ptr = arg_val.into_pointer_value();
                let key_field_ptr = self
                    .builder
                    .build_struct_gep(
                        cell_info.struct_ty,
                        value_ptr,
                        key_field_idx,
                        &format!("{}.set.key.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                let set_fn = self
                    .module
                    .get_function("lotus_hashmap_set")
                    .expect("lotus_hashmap_set extern declared");
                self.builder
                    .build_call(
                        set_fn,
                        &[
                            hashmap_field_ptr.into(),
                            key_field_ptr.into(),
                            value_ptr.into(),
                        ],
                        &format!("{}.set.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(None))
            }
            "has" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.has: expects 1 arg, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                // Look up the key type so we can validate the arg +
                // alloca the right shape.
                let cell_name = match &slot.elem_ty {
                    CodegenTy::TypeRef(n) => n.clone(),
                    _ => {
                        return Err(CodegenError::Unsupported(format!(
                            "@form(hashmap) `{}`.has: slot cell type must \
                             be a user-declared struct",
                            locus_name
                        )));
                    }
                };
                let cell_info = self
                    .user_types
                    .get(&cell_name)
                    .cloned()
                    .expect("cell type registered");
                let field_name = slot
                    .indexed_by
                    .as_ref()
                    .expect("indexed_by set on hashmap slot")
                    .clone();
                let (_, key_codegen_ty) = cell_info
                    .fields
                    .get(&field_name)
                    .cloned()
                    .expect("indexed_by field on cell");

                let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
                if key_ty != key_codegen_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.has: key arg type mismatch: \
                         expected {:?}, got {:?}",
                        locus_name, key_codegen_ty, key_ty
                    )));
                }
                let key_alloca = self
                    .alloca_for(&key_codegen_ty, "hm.has.key.slot")?;
                self.builder
                    .build_store(key_alloca, key_val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let has_fn = self
                    .module
                    .get_function("lotus_hashmap_has")
                    .expect("lotus_hashmap_has extern declared");
                let result_i32 = self
                    .builder
                    .build_call(
                        has_fn,
                        &[hashmap_field_ptr.into(), key_alloca.into()],
                        &format!("{}.has.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_hashmap_has returns i32")
                    .into_int_value();
                let zero = i32_t.const_int(0, false);
                let result_i1 = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        result_i32,
                        zero,
                        &format!("{}.has.bool", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(Some((result_i1.into(), CodegenTy::Bool))))
            }
            "len" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.len: takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let len_fn = self
                    .module
                    .get_function("lotus_hashmap_len")
                    .expect("lotus_hashmap_len extern declared");
                let result = self
                    .builder
                    .build_call(
                        len_fn,
                        &[hashmap_field_ptr.into()],
                        &format!("{}.len.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_hashmap_len returns i64");
                Ok(Some(Some((result, CodegenTy::Int))))
            }
            "is_empty" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(hashmap) `{}`.is_empty: takes no args, \
                         got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let is_empty_fn = self
                    .module
                    .get_function("lotus_hashmap_is_empty")
                    .expect("lotus_hashmap_is_empty extern declared");
                let result_i32 = self
                    .builder
                    .build_call(
                        is_empty_fn,
                        &[hashmap_field_ptr.into()],
                        &format!("{}.is_empty.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_hashmap_is_empty returns i32")
                    .into_int_value();
                let zero = i32_t.const_int(0, false);
                let result_i1 = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        result_i32,
                        zero,
                        &format!("{}.is_empty.bool", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(Some((result_i1.into(), CodegenTy::Bool))))
            }
            "get" | "remove" | "key_at" | "entry_at" => Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.{}: fallible method must be addressed \
                 via `or raise` / `or <expr>` / `or handler(err)`",
                locus_name, method_name
            ))),
            "bump" => self.lower_form_hashmap_bump(
                info,
                &slot,
                locus_self_ptr,
                hashmap_field_ptr,
                locus_name,
                args,
                scope,
            ),
            _ => unreachable!("is_synth guard"),
        }
    }

    pub(crate) fn lower_form_hashmap_bump(
        &mut self,
        info: &LocusInfo<'ctx>,
        slot: &CapacitySlotLayout,
        locus_self_ptr: PointerValue<'ctx>,
        hashmap_field_ptr: PointerValue<'ctx>,
        locus_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<Option<(BasicValueEnum<'ctx>, CodegenTy)>>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.bump: expects 1 arg (key), got {}",
                locus_name,
                args.len()
            )));
        }
        let cell_name = match &slot.elem_ty {
            CodegenTy::TypeRef(n) => n.clone(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "@form(hashmap) `{}`.bump: cell type must be a user \
                     struct; got {:?}",
                    locus_name, other
                )));
            }
        };
        let cell_info = self
            .user_types
            .get(&cell_name)
            .cloned()
            .expect("cell type registered");
        let indexed_by_name = slot
            .indexed_by
            .as_ref()
            .expect("indexed_by set on hashmap slot")
            .clone();

        // Find the (unique) Int field that isn't the indexed-by
        // field — that's the counter. Multiple Int fields, zero
        // Int fields, or extra non-Int fields → reject with a
        // pointer at the manual pattern.
        let mut counter_field: Option<(String, u32)> = None;
        let mut extras: Vec<String> = Vec::new();
        for (fname, (fidx, fty)) in &cell_info.fields {
            if *fname == indexed_by_name {
                continue;
            }
            if matches!(fty, CodegenTy::Int) {
                if counter_field.is_some() {
                    extras.push(fname.clone());
                } else {
                    counter_field = Some((fname.clone(), *fidx));
                }
            } else {
                extras.push(fname.clone());
            }
        }
        let (counter_name, counter_idx) = match (counter_field, extras.as_slice()) {
            (Some(c), []) => c,
            _ => {
                return Err(CodegenError::Unsupported(format!(
                    "@form(hashmap) `{}`.bump: cell `{}` must have exactly \
                     two fields — the indexed-by key (`{}`) and one Int \
                     counter. Use the explicit has/get/set pattern for \
                     richer cells.",
                    locus_name, cell_name, indexed_by_name
                )));
            }
        };

        // Lower the key.
        let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
        let (key_field_idx, key_codegen_ty) = cell_info
            .fields
            .get(&indexed_by_name)
            .cloned()
            .expect("indexed_by field on cell");
        if key_ty != key_codegen_ty {
            return Err(CodegenError::Unsupported(format!(
                "@form(hashmap) `{}`.bump: key arg type mismatch — \
                 expected {:?}, got {:?}",
                locus_name, key_codegen_ty, key_ty
            )));
        }

        // Alloca the key on the caller frame for the C ABI calls
        // (has + get + set all take a key pointer).
        let key_alloca = self.alloca_for(&key_codegen_ty, "bump.key.slot")?;
        self.builder
            .build_store(key_alloca, key_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Build a fresh entry buffer in the arena, populated on
        // both branches with the right (key, count) pair.
        let cell_size = cell_info
            .struct_ty
            .size_of()
            .expect("cell struct has known size");
        let new_entry_ptr = self.arena_alloc(cell_size, "bump.new_entry")?;
        let new_key_field_ptr = self
            .builder
            .build_struct_gep(
                cell_info.struct_ty,
                new_entry_ptr,
                key_field_idx,
                "bump.new.key.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(new_key_field_ptr, key_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let new_counter_field_ptr = self
            .builder
            .build_struct_gep(
                cell_info.struct_ty,
                new_entry_ptr,
                counter_idx,
                "bump.new.counter.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Call has(k) to decide init-vs-increment.
        let has_fn = self
            .module
            .get_function("lotus_hashmap_has")
            .expect("lotus_hashmap_has declared");
        let has_i32 = self
            .builder
            .build_call(
                has_fn,
                &[hashmap_field_ptr.into(), key_alloca.into()],
                "bump.has.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_hashmap_has returns i32")
            .into_int_value();
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let has_i1 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                has_i32,
                i32_t.const_int(0, false),
                "bump.has.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let func = self
            .current_fn
            .expect("bump inside fn body");
        let inc_bb = self.context.append_basic_block(func, "bump.inc");
        let init_bb = self.context.append_basic_block(func, "bump.init");
        let store_bb = self.context.append_basic_block(func, "bump.store");
        self.builder
            .build_conditional_branch(has_i1, inc_bb, init_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Increment branch: read prev counter via get; +1.
        self.builder.position_at_end(inc_bb);
        let prev_buf_ptr = self.arena_alloc(cell_size, "bump.prev_buf")?;
        let get_fn = self
            .module
            .get_function("lotus_hashmap_get")
            .expect("lotus_hashmap_get declared");
        // Ignore the bool — has() said true on the same key, so
        // get() can't miss (no concurrent mutation in v1).
        self.builder
            .build_call(
                get_fn,
                &[
                    hashmap_field_ptr.into(),
                    key_alloca.into(),
                    prev_buf_ptr.into(),
                ],
                "bump.get.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let prev_counter_ptr = self
            .builder
            .build_struct_gep(
                cell_info.struct_ty,
                prev_buf_ptr,
                counter_idx,
                "bump.prev.counter.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let prev_count = self
            .builder
            .build_load(i64_t, prev_counter_ptr, "bump.prev.count")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let next_count = self
            .builder
            .build_int_add(prev_count, i64_t.const_int(1, true), "bump.next.count")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(new_counter_field_ptr, next_count)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(store_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Init branch: count = 1.
        self.builder.position_at_end(init_bb);
        self.builder
            .build_store(new_counter_field_ptr, i64_t.const_int(1, true))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(store_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Store branch: route the populated entry through
        // lotus_hashmap_set.
        self.builder.position_at_end(store_bb);
        let set_fn = self
            .module
            .get_function("lotus_hashmap_set")
            .expect("lotus_hashmap_set declared");
        self.builder
            .build_call(
                set_fn,
                &[
                    hashmap_field_ptr.into(),
                    new_key_field_ptr.into(),
                    new_entry_ptr.into(),
                ],
                "bump.set.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let _ = locus_self_ptr;
        let _ = info;
        let _ = counter_name;
        Ok(Some(None))
    }

    /// v1.x-FORM-5: dispatch synthesized `@form(ring_buffer)`
    /// methods. Routes to the `lotus_ring_buffer_*` C runtime on
    /// the inline `{ cap, head, len, elem_size, buf }` slot.
    ///
    /// Three infallible methods (push, len, is_full) handled
    /// here; `pop` is fallible(EmptyError) and lives in
    /// `try_lower_form_ring_buffer_fallible_method`.
    pub(crate) fn try_lower_form_ring_buffer_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<
        Option<Option<(BasicValueEnum<'ctx>, CodegenTy)>>,
        CodegenError,
    > {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::RingBuffer))
            .cloned()
        else {
            return Ok(None);
        };
        let is_synth = matches!(method_name, "push" | "len" | "is_full");
        if !is_synth {
            return Ok(None);
        }

        let i32_t = self.context.i32_type();
        let rb_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!("{}.__rb_{}.ptr", locus_name, slot.name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        match method_name {
            "push" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(ring_buffer) `{}`.push: expects 1 arg, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let (arg_val, arg_ty) = self.lower_expr(&args[0], scope)?;
                if arg_ty != slot.elem_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(ring_buffer) `{}`.push arg type mismatch: \
                         expected {:?}, got {:?}",
                        locus_name, slot.elem_ty, arg_ty
                    )));
                }
                // Bus-arena reclaim follow-up (2026-05-21): same
                // cross-arena deep-copy as @form(vec).push +
                // @form(hashmap).set. Heap-pointer fields in the
                // pushed value would alias the caller's method
                // scratch and dangle on method exit; anchor them
                // in the receiver locus's __arena instead.
                let ptr_t_for_arena =
                    self.context.ptr_type(AddressSpace::default());
                let dest_arena_field_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        locus_self_ptr,
                        info.arena_field_idx,
                        &format!("{}.__arena.for_push.ptr", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                let dest_arena = self
                    .builder
                    .build_load(
                        ptr_t_for_arena,
                        dest_arena_field_ptr,
                        &format!("{}.__arena.for_push", locus_name),
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .into_pointer_value();
                let arg_val = self.emit_cross_arena_store_deep_copy_ptr(
                    arg_val,
                    &slot.elem_ty,
                    dest_arena,
                    &format!("{}.ring_buffer_push", locus_name),
                )?;
                let llvm_elem_ty = self.llvm_basic_type(&slot.elem_ty);
                let arg_alloca = self.alloca_in_entry(
                    llvm_elem_ty,
                    &format!("{}.push.arg", locus_name),
                )?;
                self.builder
                    .build_store(arg_alloca, arg_val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let push_fn = self
                    .module
                    .get_function("lotus_ring_buffer_push")
                    .expect("lotus_ring_buffer_push extern declared");
                let result_i32 = self
                    .builder
                    .build_call(
                        push_fn,
                        &[rb_field_ptr.into(), arg_alloca.into()],
                        &format!("{}.push.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_ring_buffer_push returns i32")
                    .into_int_value();
                let zero = i32_t.const_int(0, false);
                let result_i1 = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        result_i32,
                        zero,
                        &format!("{}.push.bool", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(Some((result_i1.into(), CodegenTy::Bool))))
            }
            "len" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(ring_buffer) `{}`.len: takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let len_fn = self
                    .module
                    .get_function("lotus_ring_buffer_len")
                    .expect("lotus_ring_buffer_len extern declared");
                let result = self
                    .builder
                    .build_call(
                        len_fn,
                        &[rb_field_ptr.into()],
                        &format!("{}.len.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_ring_buffer_len returns i64");
                Ok(Some(Some((result, CodegenTy::Int))))
            }
            "is_full" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(ring_buffer) `{}`.is_full: takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let is_full_fn = self
                    .module
                    .get_function("lotus_ring_buffer_is_full")
                    .expect("lotus_ring_buffer_is_full extern declared");
                let result_i32 = self
                    .builder
                    .build_call(
                        is_full_fn,
                        &[rb_field_ptr.into()],
                        &format!("{}.is_full.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_ring_buffer_is_full returns i32")
                    .into_int_value();
                let zero = i32_t.const_int(0, false);
                let result_i1 = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        result_i32,
                        zero,
                        &format!("{}.is_full.bool", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(Some((result_i1.into(), CodegenTy::Bool))))
            }
            _ => unreachable!("is_synth guard"),
        }
    }

    /// v1.x-FORM-6: inline-lower a synthesized `@form(lru_cache)`
    /// infallible method (`put`, `contains`, `len`). `get` is
    /// fallible — see `try_lower_form_lru_cache_fallible_method`.
    ///
    ///   Ok(None)        — receiver is not an lru_cache locus
    ///   Ok(Some(None))  — handled, void return (put)
    ///   Ok(Some(Some))  — handled, value return (contains, len)
    ///
    /// `put` mirrors `@form(hashmap).set`: it takes the whole cell
    /// struct, deep-copies heap fields into the receiver's arena
    /// (so they outlive the caller's method scratch), GEPs the
    /// indexed_by field out as the key, and hands both to
    /// `lotus_lru_put`, which inserts/updates and silently evicts
    /// the LRU entry on over-cap.
    pub(crate) fn try_lower_form_lru_cache_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<
        Option<Option<(BasicValueEnum<'ctx>, CodegenTy)>>,
        CodegenError,
    > {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::LruCache))
            .cloned()
        else {
            return Ok(None);
        };
        let is_synth = matches!(method_name, "put" | "contains" | "len");
        if !is_synth {
            return Ok(None);
        }

        let i32_t = self.context.i32_type();
        let lru_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!("{}.__lru_{}.ptr", locus_name, slot.name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        match method_name {
            "put" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.put: expects 1 arg, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let cell_name = match &slot.elem_ty {
                    CodegenTy::TypeRef(n) => n.clone(),
                    other => {
                        return Err(CodegenError::Unsupported(format!(
                            "@form(lru_cache) `{}`.put: slot cell type must \
                             be a user-declared struct; got {:?}",
                            locus_name, other
                        )));
                    }
                };
                let cell_info = self
                    .user_types
                    .get(&cell_name)
                    .cloned()
                    .ok_or_else(|| CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.put: cell type `{}` \
                         unregistered",
                        locus_name, cell_name
                    )))?;
                let field_name = slot
                    .indexed_by
                    .as_ref()
                    .ok_or_else(|| CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.put: slot missing indexed_by",
                        locus_name
                    )))?
                    .clone();
                let (key_field_idx, _key_ty) = cell_info
                    .fields
                    .get(&field_name)
                    .cloned()
                    .ok_or_else(|| CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.put: indexed-by field `{}` \
                         not on cell `{}`",
                        locus_name, field_name, cell_name
                    )))?;

                let (arg_val, arg_ty) = self.lower_expr(&args[0], scope)?;
                let expected_value_ty = CodegenTy::TypeRef(cell_name.clone());
                if arg_ty != expected_value_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.put arg type mismatch: \
                         expected {:?}, got {:?}",
                        locus_name, expected_value_ty, arg_ty
                    )));
                }

                // Deep-copy heap-pointer fields into the receiver
                // locus's __arena before the put — the struct
                // literal lives in the caller's per-call scratch and
                // would dangle on method exit otherwise. Same shape
                // as @form(hashmap).set.
                let dest_arena_field_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        locus_self_ptr,
                        info.arena_field_idx,
                        &format!("{}.__arena.for_put.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let ptr_t_for_arena =
                    self.context.ptr_type(AddressSpace::default());
                let dest_arena = self
                    .builder
                    .build_load(
                        ptr_t_for_arena,
                        dest_arena_field_ptr,
                        &format!("{}.__arena.for_put", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                // Cell single-owner: same rule as hashmap_set —
                // see the comment there.
                // Cell single-owner, part 2 (2026-07-18): walk a
                // stack SNAPSHOT of the value struct, not the
                // source. The anchor walk rewrites heap-pointer
                // fields in place; when the source is a
                // self-storage struct (`m.put(self.rec)`), the
                // store-back would re-point the locus field at the
                // very blob the cell is about to own — the two
                // alias again (in-place field overwrites then
                // mutate the cell, and a later retire of either
                // dangles the other). Snapshotting first leaves the
                // source untouched; entry-block alloca so a hot
                // loop reuses one slot.
                let snap = self.alloca_in_entry(
                    cell_info.struct_ty.into(),
                    &format!("{}.put.snap", locus_name),
                )?;
                let snap_size = cell_info
                    .struct_ty
                    .size_of()
                    .expect("cell struct has known size");
                self.builder
                    .build_memcpy(
                        snap,
                        8,
                        arg_val.into_pointer_value(),
                        8,
                        snap_size,
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let arg_val: BasicValueEnum = snap.into();
                let prev_owned = self.cell_owned_clone;
                self.cell_owned_clone = true;
                let walk_res = self.emit_cross_arena_store_deep_copy(
                    arg_val,
                    &expected_value_ty,
                    dest_arena,
                    &format!("{}.lru_put", locus_name),
                );
                self.cell_owned_clone = prev_owned;
                let arg_val = walk_res?;
                let value_ptr = arg_val.into_pointer_value();
                let key_field_ptr = self
                    .builder
                    .build_struct_gep(
                        cell_info.struct_ty,
                        value_ptr,
                        key_field_idx,
                        &format!("{}.put.key.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                let put_fn = self
                    .module
                    .get_function("lotus_lru_put")
                    .expect("lotus_lru_put extern declared");
                self.builder
                    .build_call(
                        put_fn,
                        &[
                            lru_field_ptr.into(),
                            key_field_ptr.into(),
                            value_ptr.into(),
                        ],
                        &format!("{}.put.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(None))
            }
            "contains" => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.contains: expects 1 arg, \
                         got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let cell_name = match &slot.elem_ty {
                    CodegenTy::TypeRef(n) => n.clone(),
                    _ => {
                        return Err(CodegenError::Unsupported(format!(
                            "@form(lru_cache) `{}`.contains: slot cell type \
                             must be a user-declared struct",
                            locus_name
                        )));
                    }
                };
                let cell_info = self
                    .user_types
                    .get(&cell_name)
                    .cloned()
                    .expect("cell type registered");
                let field_name = slot
                    .indexed_by
                    .as_ref()
                    .expect("indexed_by set on lru_cache slot")
                    .clone();
                let (_, key_codegen_ty) = cell_info
                    .fields
                    .get(&field_name)
                    .cloned()
                    .expect("indexed_by field on cell");

                let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
                if key_ty != key_codegen_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.contains: key arg type \
                         mismatch: expected {:?}, got {:?}",
                        locus_name, key_codegen_ty, key_ty
                    )));
                }
                let key_alloca =
                    self.alloca_for(&key_codegen_ty, "lru.contains.key.slot")?;
                self.builder
                    .build_store(key_alloca, key_val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let contains_fn = self
                    .module
                    .get_function("lotus_lru_contains")
                    .expect("lotus_lru_contains extern declared");
                let result_i32 = self
                    .builder
                    .build_call(
                        contains_fn,
                        &[lru_field_ptr.into(), key_alloca.into()],
                        &format!("{}.contains.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_lru_contains returns i32")
                    .into_int_value();
                let zero = i32_t.const_int(0, false);
                let result_i1 = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        result_i32,
                        zero,
                        &format!("{}.contains.bool", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(Some(Some((result_i1.into(), CodegenTy::Bool))))
            }
            "len" => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "@form(lru_cache) `{}`.len: takes no args, got {}",
                        locus_name,
                        args.len()
                    )));
                }
                let len_fn = self
                    .module
                    .get_function("lotus_lru_len")
                    .expect("lotus_lru_len extern declared");
                let result = self
                    .builder
                    .build_call(
                        len_fn,
                        &[lru_field_ptr.into()],
                        &format!("{}.len.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_lru_len returns i64");
                Ok(Some(Some((result, CodegenTy::Int))))
            }
            _ => unreachable!("is_synth guard"),
        }
    }

    /// v1.x-FORM-6: inline-lower the synthesized `@form(lru_cache)`
    /// fallible method `get(k: K) -> S fallible(KeyError)`. Mirrors
    /// `@form(hashmap).get`: materialize the key into an alloca,
    /// arena-allocate a value buffer for the runtime to memcpy the
    /// hit into, call `lotus_lru_get` (which also bumps recency on
    /// a hit), and lazily construct a `KeyError` on the miss path.
    pub(crate) fn try_lower_form_lru_cache_fallible_method(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<FallibleCallResult<'ctx>>, CodegenError> {
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.form == Some(SlotForm::LruCache))
            .cloned()
        else {
            return Ok(None);
        };
        if method_name != "get" {
            return Ok(None);
        }

        let cell_name = match &slot.elem_ty {
            CodegenTy::TypeRef(n) => n.clone(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "@form(lru_cache) `{}`.get: slot cell type must be a \
                     user-declared struct; got {:?}",
                    locus_name, other
                )));
            }
        };
        let cell_info = self
            .user_types
            .get(&cell_name)
            .cloned()
            .ok_or_else(|| CodegenError::Unsupported(format!(
                "@form(lru_cache) `{}`.get: cell type `{}` not registered",
                locus_name, cell_name
            )))?;
        let field_name = slot
            .indexed_by
            .as_ref()
            .ok_or_else(|| CodegenError::Unsupported(format!(
                "@form(lru_cache) `{}`.get: slot missing indexed_by",
                locus_name
            )))?
            .clone();
        let (_field_idx, key_codegen_ty) = cell_info
            .fields
            .get(&field_name)
            .cloned()
            .ok_or_else(|| CodegenError::Unsupported(format!(
                "@form(lru_cache) `{}`.get: indexed-by field `{}` not on \
                 cell `{}`",
                locus_name, field_name, cell_name
            )))?;
        if !matches!(key_codegen_ty, CodegenTy::Int | CodegenTy::String) {
            return Err(CodegenError::Unsupported(format!(
                "@form(lru_cache) `{}`.get: key type {:?} unsupported; v1 \
                 supports Int and String keys only",
                locus_name, key_codegen_ty
            )));
        }
        let value_codegen_ty = CodegenTy::TypeRef(cell_name.clone());

        let lru_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                locus_self_ptr,
                slot.struct_field_idx,
                &format!("{}.__lru_{}.fallible.ptr", locus_name, slot.name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let payload_ty = CodegenTy::TypeRef("KeyError".to_string());
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "@form(lru_cache) `{}`.get: expects 1 arg, got {}",
                locus_name,
                args.len()
            )));
        }
        let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
        if key_ty != key_codegen_ty {
            return Err(CodegenError::Unsupported(format!(
                "@form(lru_cache) `{}`.get: key arg type mismatch: expected \
                 {:?}, got {:?}",
                locus_name, key_codegen_ty, key_ty
            )));
        }
        let key_alloca = self.alloca_for(&key_codegen_ty, "lru.get.key.slot")?;
        self.builder
            .build_store(key_alloca, key_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let out_err_slot =
            self.alloca_for(&payload_ty, "lru.get.out_err.slot")?;
        let i32_t = self.context.i32_type();
        let zero_i32 = i32_t.const_int(0, false);

        // Surface success value for a TypeRef cell is a pointer to
        // the struct; arena-allocate the buffer the runtime memcpys
        // the hit into, then stash its pointer in the surface slot.
        let cell_struct_size = cell_info
            .struct_ty
            .size_of()
            .expect("cell struct has known size");
        let value_buf_ptr =
            self.arena_alloc(cell_struct_size, "lru.get.value_buf")?;
        let out_val_slot =
            self.alloca_for(&value_codegen_ty, "lru.get.out_val.slot")?;
        self.builder
            .build_store(out_val_slot, value_buf_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let get_fn = self
            .module
            .get_function("lotus_lru_get")
            .expect("lotus_lru_get declared");
        let c_ret = self
            .builder
            .build_call(
                get_fn,
                &[
                    lru_field_ptr.into(),
                    key_alloca.into(),
                    value_buf_ptr.into(),
                ],
                &format!("{}.get.call", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_lru_get returns i32")
            .into_int_value();

        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                c_ret,
                zero_i32,
                &format!("{}.get.is_err", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let func = self
            .current_fn
            .expect("fallible-method call inside fn body");
        let lazy_err_bb =
            self.context.append_basic_block(func, "lru.get.lazy_err");
        let join_bb =
            self.context.append_basic_block(func, "lru.get.lazy_join");
        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(lazy_err_bb);
        let ke_ptr = self.emit_key_error_alloc("missing_key")?;
        self.builder
            .build_store(out_err_slot, ke_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);

        Ok(Some(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: Some(out_val_slot),
            out_err_slot,
            success_ty: Some(value_codegen_ty),
            payload_ty,
        }))
    }

}
