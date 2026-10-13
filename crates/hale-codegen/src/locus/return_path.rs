//! Return-path codegen: deep-copy emission for locus-method
//! returns (m49 method-with-scratch + m90 caller-arena placement)
//! + the in-place self-field assign machinery. This is the home
//! for issue #9 (m90 return-slot ABI) work. Round 7c of the
//! codegen model-org refactor — the missing 4.5 from the original
//! Round 4 plan.
//!
//! Lifted as inherent `impl<'ctx, 'p> Cx<'ctx, 'p>` blocks — call
//! sites need no `use` import.

use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, CodegenTy, Cx, EnumInfo};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Cross-arena store deep-copy for **pointer-storage**
    /// containers — @form(vec).push, @form(vec).set,
    /// @form(ring_buffer).push. Those slots hold an 8-byte
    /// pointer (`elem_size = sizeof(ptr)`); the source struct
    /// itself must outlive the caller's method scratch, so a
    /// fresh allocation in `dest_arena` is mandatory.
    ///
    /// Same shape as d9335bf: emit a runtime
    /// `lotus_arena_contains_ptr(dest_arena, src)` check; on hit,
    /// pass src through (the slot will store the existing
    /// pointer, which is already long-lived). On miss, allocate
    /// a fresh outer struct in dest_arena, walk + deep-copy
    /// fields recursively, return the new pointer.
    pub(crate) fn emit_cross_arena_store_deep_copy_ptr(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        dest_arena: PointerValue<'ctx>,
        site_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        // Pass-through arms (scalars / views / locus refs / leaf
        // heap types) match the existing emit_return_value_deep_copy
        // contract — no value to skip-check, and the helper does
        // the right thing (identity for scalars, same-arena clone
        // for String/Bytes).
        match ty {
            CodegenTy::Int
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal(_)
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::FnPtr { .. }
            | CodegenTy::BytesView
            | CodegenTy::StringView
            | CodegenTy::LocusRef(_)
            | CodegenTy::String
            | CodegenTy::Bytes => {
                return self.emit_return_value_deep_copy(value, ty, dest_arena);
            }
            _ => {}
        }
        let src_ptr = value.into_pointer_value();
        let contains_fn = self
            .module
            .get_function("lotus_arena_contains_ptr")
            .expect("lotus_arena_contains_ptr declared");
        let contains_i32 = self
            .builder
            .build_call(
                contains_fn,
                &[dest_arena.into(), src_ptr.into()],
                &format!("{}.in_dest_arena", site_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_arena_contains_ptr returns i32")
            .into_int_value();
        let i32_t = self.context.i32_type();
        let cond = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                contains_i32,
                i32_t.const_zero(),
                &format!("{}.same_arena", site_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let current_fn = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("inside a function");
        let skip_bb = self.context.append_basic_block(
            current_fn,
            &format!("{}.deep_copy.skip", site_name),
        );
        let copy_bb = self.context.append_basic_block(
            current_fn,
            &format!("{}.deep_copy.do", site_name),
        );
        let join_bb = self.context.append_basic_block(
            current_fn,
            &format!("{}.deep_copy.join", site_name),
        );
        self.builder
            .build_conditional_branch(cond, skip_bb, copy_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(skip_bb);
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(copy_bb);
        let copied = self.emit_return_value_deep_copy(value, ty, dest_arena)?;
        let copy_end = self
            .builder
            .get_insert_block()
            .expect("copy block still positioned");
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let phi = self
            .builder
            .build_phi(ptr_t, &format!("{}.deep_copy.phi", site_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let src_basic: BasicValueEnum = src_ptr.into();
        phi.add_incoming(&[(&src_basic, skip_bb), (&copied, copy_end)]);
        Ok(phi.as_basic_value())
    }

    /// GH #577: a store that always OWNS. `emit_cross_arena_store_deep_copy_ptr`
    /// passes a pointer through when it already lives in `dest_arena`;
    /// for a container whose `set` retires the replaced element that
    /// pass-through let two slots share one struct (a value read from
    /// the same vec, or one value pushed twice), and retiring one freed
    /// the other's. A vec's elements are the vec's: a heap-bearing value
    /// is copied into `dest_arena` on every push and set, whatever
    /// arena it came from; scalars and locus refs pass through.
    pub(crate) fn emit_owned_store_copy_ptr(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        dest_arena: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match ty {
            CodegenTy::String | CodegenTy::Bytes | CodegenTy::TypeRef(_) | CodegenTy::Tuple(_) | CodegenTy::Array(_, _) => {
                self.emit_return_value_deep_copy(value, ty, dest_arena)
            }
            _ => Ok(value),
        }
    }

    /// A field of a retiring `@form(hashmap)` cell that points at a
    /// block of its own (a nested record, a tuple, an out-of-line fixed
    /// array): the cell's own deep copy, since the retire frees the
    /// block when the cell is replaced or removed and a block shared
    /// with another owner would be freed under it. With `old_field`
    /// (`sync = none`: the value the key's cell holds now, its struct
    /// type and the field's index) a field still carrying that cell's
    /// own block at the same field keeps it — the get-modify-set of a
    /// cell with a large nested field stays allocation-free, and the
    /// retire sees the block survive.
    pub(crate) fn emit_cell_owned_block_store(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        dest_arena: PointerValue<'ctx>,
        old_field: Option<(PointerValue<'ctx>, inkwell::types::StructType<'ctx>, u32)>,
        site_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let Some((old_v, struct_ty, idx)) = old_field else {
            return self.emit_return_value_deep_copy(value, ty, dest_arena);
        };
        let err = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let src = value.into_pointer_value();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let current_fn = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("inside a function");
        let has_old_bb = self
            .context
            .append_basic_block(current_fn, &format!("{}.own.has_old", site_name));
        let keep_bb = self
            .context
            .append_basic_block(current_fn, &format!("{}.own.keep", site_name));
        let copy_bb = self
            .context
            .append_basic_block(current_fn, &format!("{}.own.copy", site_name));
        let join_bb = self
            .context
            .append_basic_block(current_fn, &format!("{}.own.join", site_name));
        let no_old = self
            .builder
            .build_is_null(old_v, &format!("{}.own.no_old", site_name))
            .map_err(err)?;
        self.builder
            .build_conditional_branch(no_old, copy_bb, has_old_bb)
            .map_err(err)?;

        self.builder.position_at_end(has_old_bb);
        let old_slot = self
            .builder
            .build_struct_gep(struct_ty, old_v, idx, &format!("{}.own.old.ptr", site_name))
            .map_err(err)?;
        let old_block = self
            .builder
            .build_load(ptr_t, old_slot, &format!("{}.own.old", site_name))
            .map_err(err)?
            .into_pointer_value();
        let same = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                src,
                old_block,
                &format!("{}.own.same", site_name),
            )
            .map_err(err)?;
        self.builder
            .build_conditional_branch(same, keep_bb, copy_bb)
            .map_err(err)?;

        self.builder.position_at_end(keep_bb);
        self.builder.build_unconditional_branch(join_bb).map_err(err)?;

        self.builder.position_at_end(copy_bb);
        let copied = self.emit_return_value_deep_copy(value, ty, dest_arena)?;
        let copy_end = self
            .builder
            .get_insert_block()
            .expect("copy block still positioned");
        self.builder.build_unconditional_branch(join_bb).map_err(err)?;

        self.builder.position_at_end(join_bb);
        let phi = self
            .builder
            .build_phi(ptr_t, &format!("{}.own.phi", site_name))
            .map_err(err)?;
        let src_basic: BasicValueEnum = src.into();
        phi.add_incoming(&[(&src_basic, keep_bb), (&copied, copy_end)]);
        Ok(phi.as_basic_value())
    }

    /// A deep copy of `value` into `dest_arena` that OWNS every leaf:
    /// a String or Bytes already in `dest_arena` is copied too
    /// (`cell_owned_clone`), where the plain copy passes it through.
    /// For a value stored into a locus's own fields outside a compound
    /// self-field store — a param's value at construction, the built
    /// params a restart-in-place snapshots and restores. Those values
    /// are evaluated in the locus's arena, so a plain copy kept a
    /// String one field or param shared with another (`o: Outer = Outer
    /// { s: self.text, inner: Inner { s: self.text } }`), and the
    /// retire of a replaced value — its String fixups, its nested-block
    /// walk (`lotus_self_field_retire_replaced`) — then freed it under
    /// the others. Records, tuples, arrays and payload-enum blocks are
    /// always fresh in this copy.
    pub(crate) fn emit_owned_deep_copy(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        dest_arena: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let prev = std::mem::replace(&mut self.cell_owned_clone, true);
        let out = self.emit_return_value_deep_copy(value, ty, dest_arena);
        self.cell_owned_clone = prev;
        out
    }

    pub(crate) fn emit_return_value_deep_copy(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        dest_arena: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match ty {
            CodegenTy::Bounded(_, _) => Err(CodegenError::Unsupported(
                "bounded[T; N] is not a first-class value — it cannot \
                 be deep-copied on its own"
                    .into(),
            )),
            CodegenTy::Int
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal(_)
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::BytesMut
            | CodegenTy::Drain(_)
            | CodegenTy::Perspective(_)
            | CodegenTy::FnPtr { .. } => Ok(value),
            CodegenTy::Enum(name) => {
                let info = self
                    .user_enums
                    .get(name.as_str())
                    .cloned();
                match info {
                    Some(info) if info.has_payload => {
                        // m51: per-variant switch + recursive
                        // payload deep-copy. See
                        // emit_enum_payload_deep_copy.
                        self.emit_enum_payload_deep_copy(
                            &info,
                            value.into_pointer_value(),
                            dest_arena,
                        )
                    }
                    _ => Ok(value),
                }
            }
            CodegenTy::String => {
                // Cell stores force same-arena inputs to an owned
                // copy (cell single-owner rule); everything else
                // keeps the skip-sharing clone.
                let clone_name = if self.cell_owned_clone {
                    "lotus_str_clone_cell_owned"
                } else {
                    "lotus_str_clone"
                };
                let f = self
                    .module
                    .get_function(clone_name)
                    .expect("str clone builtin declared");
                let res = self
                    .builder
                    .build_call(
                        f,
                        &[dest_arena.into(), value.into_pointer_value().into()],
                        "fn.ret.str.clone",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_clone returns ptr");
                Ok(res)
            }
            CodegenTy::Bytes => {
                // Bus-arena reclaim (2026-05-21): clone the Bytes
                // payload into dest_arena via the length-aware
                // copier. Previously this arm passed through on
                // the assumption that the value already lived in
                // the program-lifetime payload arena, but with
                // per-method scratch a Bytes built inside a method
                // body lives in the scratch and would dangle on
                // method-exit destroy. The free-fn path pays a
                // one-memcpy cost when src/dest are the same
                // arena; for moderately-sized binary payloads
                // that's negligible vs the alternative of leaving
                // a dangling pointer. Use `lotus_bytes_clone`
                // (defined alongside `lotus_bytes_from_buf`) so
                // the length prefix is honored.
                let clone_name = if self.cell_owned_clone {
                    "lotus_bytes_clone_cell_owned"
                } else {
                    "lotus_bytes_clone"
                };
                let f = self
                    .module
                    .get_function(clone_name)
                    .expect("bytes clone builtin declared");
                let res = self
                    .builder
                    .build_call(
                        f,
                        &[dest_arena.into(), value.into_pointer_value().into()],
                        "deep_copy.bytes.clone",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_bytes_clone returns ptr");
                Ok(res)
            }
            CodegenTy::BytesView | CodegenTy::StringView => {
                // F.30: a view returned from a fn keeps aliasing
                // whatever buffer the source builder owned. The
                // caller is responsible for treating it as a view
                // (lifetime tied to the source builder); no
                // deep-copy here. If the caller wants owned
                // storage they must explicitly clone via
                // `std::bytes::clone(v)` / `std::str::clone(v)`
                // (see F.30 spec for the surface).
                Ok(value)
            }
            CodegenTy::Tuple(elem_tys) => {
                // Allocate a fresh tuple-storage struct in
                // dest_arena (from the retire freelist when a block
                // fits, as the struct arm does: a vec element's tuple
                // field is freed when the element is replaced), then
                // recursively deep-copy each element. Layout matches Expr::Tuple lowering so
                // tup.0 / tup.1 reads work identically on the
                // returned tuple.
                let storage_ty = self.llvm_tuple_storage_type(elem_tys);
                let bytes = storage_ty
                    .size_of()
                    .expect("tuple storage type has known size");
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc_reusable")
                    .expect("lotus_arena_alloc_reusable declared");
                let i64_t = self.context.i64_type();
                let new_tup = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            dest_arena.into(),
                            bytes.into(),
                            // 16-byte align matches the rest of the
                            // codebase's struct-alloc default — i128
                            // fields nested in a tuple element need
                            // movdqa-compatible alignment. Segfault
                            // repro: 3+ Decimal fields in a
                            // @form(hashmap) Cell, post-Phase-4
                            // scratch where the deep-copy went via
                            // this arm.
                            i64_t.const_int(16, false).into(),
                        ],
                        "fn.ret.tuple.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("arena_alloc returns ptr")
                    .into_pointer_value();
                let i32_t = self.context.i32_type();
                let src_ptr = value.into_pointer_value();
                for (i, elem_ty) in elem_tys.iter().enumerate() {
                    let src_slot = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                src_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("fn.ret.tup.src.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    let llvm_elem_ty = self.llvm_basic_type(elem_ty);
                    let elem_val = self
                        .builder
                        .build_load(
                            llvm_elem_ty,
                            src_slot,
                            &format!("fn.ret.tup.src.load{}", i),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let copied = self.emit_return_value_deep_copy(
                        elem_val, elem_ty, dest_arena,
                    )?;
                    let dst_slot = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                new_tup,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("fn.ret.tup.dst.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    self.builder
                        .build_store(dst_slot, copied)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(new_tup.into())
            }
            CodegenTy::Array(elem_ty, n) => {
                // m51: deep-copy a fixed-size array. Allocate
                // `[n x llvm(elem)]` in dest_arena, GEP each slot
                // in the source, recurse on the element value, and
                // store into the destination slot. Layout matches
                // the array-literal allocation path so callers see
                // the returned array's slot loads identically.
                let arr_ty =
                    self.llvm_array_storage_type(elem_ty, *n);
                let bytes = arr_ty
                    .size_of()
                    .expect("array storage type has known size");
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc_reusable")
                    .expect("lotus_arena_alloc_reusable declared");
                let i64_t = self.context.i64_type();
                let new_arr = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            dest_arena.into(),
                            bytes.into(),
                            // 16-byte align for i128 elements / i128
                            // nested in element structs. Same root
                            // cause as the tuple/struct arms.
                            i64_t.const_int(16, false).into(),
                        ],
                        "fn.ret.arr.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("arena_alloc returns ptr")
                    .into_pointer_value();
                let i32_t = self.context.i32_type();
                let src_ptr = value.into_pointer_value();
                let llvm_elem_ty = self.llvm_basic_type(elem_ty);
                for i in 0..*n {
                    let src_slot = unsafe {
                        self.builder
                            .build_gep(
                                arr_ty,
                                src_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i, false),
                                ],
                                &format!("fn.ret.arr.src.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    let elem_val = self
                        .builder
                        .build_load(
                            llvm_elem_ty,
                            src_slot,
                            &format!("fn.ret.arr.src.load{}", i),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let copied = self.emit_return_value_deep_copy(
                        elem_val, elem_ty, dest_arena,
                    )?;
                    let dst_slot = unsafe {
                        self.builder
                            .build_gep(
                                arr_ty,
                                new_arr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i, false),
                                ],
                                &format!("fn.ret.arr.dst.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    self.builder
                        .build_store(dst_slot, copied)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(new_arr.into())
            }
            CodegenTy::TypeRef(name) => {
                // m51: deep-copy a user-defined struct. Allocate a
                // fresh struct in dest_arena, walk each declared
                // field by its struct slot index, recursively copy
                // the loaded value, and store into the
                // destination. Field order matches the original
                // declaration via TypeInfo.field_order.
                let info = self
                    .user_types
                    .get(name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "free-fn return of unknown type `{}`",
                            name
                        ))
                    })?;
                let struct_ty = info.struct_ty;
                let bytes = struct_ty
                    .size_of()
                    .expect("user-type struct has known size");
                // iris handoff P1: consult the retire freelist so a
                // steady-state form replace (vec.set) ping-pongs
                // between reused blocks instead of growing the
                // owner's arena ~sizeof(elem) per call.
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc_reusable")
                    .expect("lotus_arena_alloc_reusable declared");
                let i64_t = self.context.i64_type();
                let new_struct = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            dest_arena.into(),
                            bytes.into(),
                            // 16-byte align — matches the standard
                            // user-struct alloc path (arena_alloc's
                            // default after the 2026-05-20 i128
                            // alignment fix). i128 (Decimal) fields
                            // generate movdqa on x86_64 which traps
                            // on 8-byte alignment. Segfault repro:
                            // 3+ Decimal fields in a @form(hashmap)
                            // Cell, triggered by the Phase-4
                            // method-scratch deep-copy going through
                            // this arm.
                            i64_t.const_int(16, false).into(),
                        ],
                        "fn.ret.struct.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("arena_alloc returns ptr")
                    .into_pointer_value();
                let src_ptr = value.into_pointer_value();
                for fname in &info.field_order {
                    let (idx, fty) = info
                        .fields
                        .get(fname)
                        .cloned()
                        .expect("field_order lists declared fields");
                    let src_slot = self
                        .builder
                        .build_struct_gep(
                            struct_ty,
                            src_ptr,
                            idx,
                            &format!("fn.ret.struct.src.{}", fname),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let dst_slot = self
                        .builder
                        .build_struct_gep(
                            struct_ty,
                            new_struct,
                            idx,
                            &format!("fn.ret.struct.dst.{}", fname),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // Inline fixed arrays: the slot IS the [N x T]
                    // element bytes — memcpy them across. Loading it
                    // as a `ptr` (the pre-inline shape) would read
                    // element bytes as an address and recurse on
                    // garbage.
                    if Self::array_inline_spec(&fty).is_some() {
                        let size = self.compound_storage_size(&fty)?;
                        self.emit_memcpy_call(
                            dst_slot,
                            src_slot,
                            size,
                            &format!("fn.ret.struct.inline_arr.{}", fname),
                        )?;
                        continue;
                    }
                    // bounded[T; N]: memcpy the inline { len, data }
                    // storage, then anchor the DST copy's live
                    // pointer elements into dest_arena (no-op for
                    // scalar elements).
                    if let CodegenTy::Bounded(belem, bcap) = &fty {
                        let size = self.compound_storage_size(&fty)?;
                        self.emit_memcpy_call(
                            dst_slot,
                            src_slot,
                            size,
                            &format!("fn.ret.struct.bounded.{}", fname),
                        )?;
                        self.anchor_bounded_elems_in_place(
                            dst_slot,
                            belem,
                            *bcap,
                            dest_arena,
                            &format!("fn.ret.struct.bounded.{}", fname),
                        )?;
                        continue;
                    }
                    let llvm_field_ty = self.llvm_basic_type(&fty);
                    let field_val = self
                        .builder
                        .build_load(
                            llvm_field_ty,
                            src_slot,
                            &format!("fn.ret.struct.load.{}", fname),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let copied = self.emit_return_value_deep_copy(
                        field_val, &fty, dest_arena,
                    )?;
                    self.builder
                        .build_store(dst_slot, copied)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(new_struct.into())
            }
            CodegenTy::LocusRef(_) => {
                // B1 / G3: free fns returning a LocusRef. The
                // m90 heap-alloc path in `lower_locus_instantiation`
                // (triggered by `current_user_fn_ret` matching the
                // locus name) routes the instantiation into the
                // lazy global payload arena, so the returned
                // pointer is already program-lifetime-safe.
                // Pass-through here — same shape as the Bytes
                // arm above. Matches the locus-method m90 return
                // path that already covers `fn(...) -> Self`
                // inside a locus body.
                Ok(value)
            }
            CodegenTy::Interface(_) => {
                // G20 / F.20 Phase B follow-up: allocate a fresh
                // 16-byte fat-pointer struct in dest_arena, then
                // copy the {data, vtable} slots over. The vtable is
                // a static global so no copy is needed for it. The
                // data pointer is program-lifetime safe in two
                // shapes: (a) the underlying locus was freshly
                // instantiated inside this fn — the m90 routing
                // extension routed it to the payload arena; (b) the
                // interface value was passed in / loaded from
                // storage — the data pointer was already
                // caller-or-program-lifetime, so it stays valid
                // past this fn's subregion destroy.
                let src_ptr = value.into_pointer_value();
                let fat_struct_ty = self.iface_fat_struct_ty();
                let ptr_t = self.context.ptr_type(AddressSpace::default());
                let i64_t = self.context.i64_type();
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc")
                    .expect("lotus_arena_alloc declared");
                let new_fat = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            dest_arena.into(),
                            i64_t.const_int(16, false).into(),
                            // 16-byte align for parity with the
                            // sibling arms. The fat-pointer itself
                            // is two i64 slots, but uniform 16-align
                            // avoids future surprises if the layout
                            // ever sprouts a wider slot.
                            i64_t.const_int(16, false).into(),
                        ],
                        "fn.ret.iface.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_arena_alloc returns ptr")
                    .into_pointer_value();
                for (i, slot) in ["data", "vtable"].iter().enumerate() {
                    let src_slot = self
                        .builder
                        .build_struct_gep(
                            fat_struct_ty,
                            src_ptr,
                            i as u32,
                            &format!("fn.ret.iface.src.{}", slot),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let val = self
                        .builder
                        .build_load(
                            ptr_t,
                            src_slot,
                            &format!("fn.ret.iface.load.{}", slot),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let dst_slot = self
                        .builder
                        .build_struct_gep(
                            fat_struct_ty,
                            new_fat,
                            i as u32,
                            &format!("fn.ret.iface.dst.{}", slot),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(dst_slot, val)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(new_fat.into())
            }
            CodegenTy::Cell(_, _) => Err(CodegenError::Unsupported(format!(
                "free-fn return of {:?}: F.22 capacity-slot cells \
                 can't cross fn boundaries — the cell's lifetime is \
                 the locus's slot, and the caller's frame doesn't \
                 know which slot to release/free into. Round-trip \
                 cells inside the locus body instead.",
                ty
            ))),
        }
    }

    /// m51: switch-on-tag deep-copy for a has-payload enum return
    /// value. We pre-load the tag, then dispatch through a switch
    /// where each case alloc's a fresh storage struct in
    /// dest_arena, deep-copies the variant's payload fields via
    /// load_enum_payload_fields + recursive emit_return_value_deep_copy,
    /// and writes them back via lower_enum_variant_alloc. The new
    /// pointers PHI-join into a single returned ptr value.
    pub(crate) fn emit_enum_payload_deep_copy(
        &mut self,
        info: &EnumInfo,
        src_ptr: PointerValue<'ctx>,
        dest_arena: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .expect("enum deep-copy emitted inside a fn");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i32_t = self.context.i32_type();
        let tag = self.load_enum_tag(info, src_ptr)?;
        let entry_bb = self
            .builder
            .get_insert_block()
            .expect("builder positioned");
        let cont_bb = self.context.append_basic_block(func, "enum.dc.cont");
        let default_bb = self.context.append_basic_block(func, "enum.dc.default");
        let mut variant_blocks: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            BasicValueEnum<'ctx>,
        )> = Vec::new();
        // Set up per-variant blocks first; switch wires after.
        for (i, _) in info.variants.iter().enumerate() {
            let bb = self
                .context
                .append_basic_block(func, &format!("enum.dc.v{}", i));
            self.builder.position_at_end(bb);
            // Push the caller_arena_override so payload allocations
            // for this variant happen in dest_arena, not the fn
            // subregion. Wait — actually, we need the *new enum
            // struct* to land in dest_arena via lower_enum_variant_alloc,
            // which calls arena_alloc through current_arena_ptr.
            // Override current_arena_override for this stretch.
            let prev_override = self.current_arena_override;
            self.current_arena_override = Some(dest_arena);
            // Load the variant's payload fields from src_ptr, then
            // deep-copy each one into dest_arena.
            let raw_fields = self.load_enum_payload_fields(info, src_ptr, i)?;
            let mut copied_fields: Vec<(BasicValueEnum<'ctx>, CodegenTy)> =
                Vec::with_capacity(raw_fields.len());
            for (val, fty) in raw_fields {
                let copied = self.emit_return_value_deep_copy(
                    val, &fty, dest_arena,
                )?;
                copied_fields.push((copied, fty));
            }
            // Allocate the new enum value in dest_arena (the
            // override routes lower_enum_variant_alloc's
            // arena_alloc there).
            let new_ptr = self.lower_enum_variant_alloc_in(
                info,
                i as u32,
                &copied_fields,
                Some(dest_arena),
            )?;
            self.current_arena_override = prev_override;
            // The payload's own deep copy (a nested enum's switch, a
            // record's) adds blocks: the PHI's edge comes from the one
            // the builder now stands in, not from `bb`, which only the
            // switch targets.
            let end_bb = self
                .builder
                .get_insert_block()
                .expect("builder positioned in enum.dc variant");
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            variant_blocks.push((
                i32_t.const_int(i as u64, false),
                bb,
                end_bb,
                new_ptr.into(),
            ));
        }
        // Default block: should be unreachable (tag is always one
        // of the declared variants). Fall through with a null ptr
        // PHI value to keep IR well-formed.
        self.builder.position_at_end(default_bb);
        self.builder
            .build_unconditional_branch(cont_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Wire the switch from entry_bb.
        self.builder.position_at_end(entry_bb);
        let cases: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = variant_blocks
            .iter()
            .map(|(c, bb, _, _)| (*c, *bb))
            .collect();
        self.builder
            .build_switch(tag, default_bb, &cases)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // PHI in cont.
        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(ptr_t, "enum.dc.phi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let mut incoming: Vec<(
            &dyn inkwell::values::BasicValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = Vec::new();
        for (_, _, end_bb, val) in &variant_blocks {
            incoming.push((val, *end_bb));
        }
        let null = ptr_t.const_null();
        incoming.push((&null, default_bb));
        phi.add_incoming(&incoming);
        Ok(phi.as_basic_value())
    }

    /// Bus-arena reclaim (2026-05-21): when a method body stores
    /// a heap-typed value into `self.X`, deep-copy the value into
    /// `self.__arena` before the store so the persisted pointer
    /// outlives the per-method scratch's destroy at method exit.
    /// Returns `value` unchanged when:
    ///   * no scratch is active (top-level free fn / synthesized
    ///     closure body / etc. — allocations land where the
    ///     existing routing puts them);
    ///   * the slot type is a scalar / view / locus-ref / cell;
    /// otherwise routes through `emit_return_value_deep_copy`
    /// targeting `self.__arena`.
    /// In-place assign for a self-field slot whose existing storage
    /// can be mutated. Used at `self.X = expr` and `self.X[i] = expr`
    /// sites where the slot already holds a stable pointer to a
    /// struct in `self.__arena` (typical: locus field with a default
    /// `Struct { }` initializer, or array element of a fixed-size
    /// array of structs with default-initialized elements — every
    /// such slot is non-null by the time any method body runs).
    ///
    /// Pre-rework, both sites called `maybe_self_field_heap_copy` +
    /// `build_store`. The helper allocated a fresh struct in
    /// `self.__arena`, copied fields into it, returned the new
    /// pointer; the store wrote that pointer over the slot's
    /// existing pointer. The old struct became unreachable garbage
    /// in `self.__arena`, accumulating ~sizeof(struct) bytes per
    /// assign × per-frame rate (~50 B × 30 frames/sec for a downstream
    /// WsClient.last_message; ~32 B × 30 deltas/sec × 3 levels for
    /// SymbolBook's `self.bids[i] = BookLevel{...}`).
    ///
    /// In-place: anchor the rhs's heap fields in `self.__arena`
    /// (via `emit_cross_arena_store_deep_copy`'s anchor-in-place
    /// rewrite on the source struct), then memcpy the rhs's bytes
    /// over the EXISTING struct's bytes at the slot's pointer. The
    /// slot's pointer doesn't change — no `build_store` needed. Any
    /// number of assigns to the same slot stays within the
    /// originally-allocated struct's bytes; the only growth is per
    /// heap-field deep-copy when the rhs's heap field has new
    /// content (e.g., a fresh dynamic String).
    pub(crate) fn emit_self_field_inplace_assign(
        &mut self,
        slot_ptr: PointerValue<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        slot_ty: &CodegenTy,
    ) -> Result<(), CodegenError> {
        // 2026-07-01 inline fixed arrays: the slot IS the [N x T]
        // storage (not a pointer to it), so whole-array assignment
        // is a memcpy of the element bytes into the slot — in every
        // context, scratch or not. No anchoring: inline eligibility
        // requires scalar elements (no heap fields to chase). This
        // must come before every other branch — the legacy paths
        // below either store a pointer over the slot (corrupting
        // inline data with an 8-byte ptr) or load a "pointer" out
        // of it (reading element bytes as an address).
        if Self::array_inline_spec(slot_ty).is_some() {
            let size = self.compound_storage_size(slot_ty)?;
            self.emit_memcpy_call(
                slot_ptr,
                rhs.into_pointer_value(),
                size,
                "inline_array.assign.memcpy",
            )?;
            return Ok(());
        }
        // Scalars + views + LocusRefs + Cells: store the value
        // directly. No anchoring needed (the value's bytes live
        // in the slot itself, or the slot holds a stable pointer
        // to a long-lived locus).
        let compound = match slot_ty {
            CodegenTy::Int
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal(_)
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::FnPtr { .. }
            | CodegenTy::BytesView
            | CodegenTy::StringView
            | CodegenTy::LocusRef(_)
            | CodegenTy::Cell(_, _) => false,
            // String/Bytes: slot holds a pointer. We have an
            // "existing buffer" — see the in-place-assign branch
            // below for the per-delta-leak fix.
            CodegenTy::String | CodegenTy::Bytes => false,
            _ => true,
        };

        // 2026-05-22 PM: in-place String/Bytes reassignment for
        // the `self.X = heap_value` field-assign hot path. Calls
        // lotus_str_assign_in_place / lotus_bytes_assign_in_place
        // which reuse the old buffer when new fits — eliminates
        // the per-update leak class (a downstream daemon / SymbolBook's
        // `self.last_venue_ts = venue_ts`). Only fires inside a
        // method-with-scratch (the only context where `self.__arena`
        // is the dest); outside (synthesized closure-eval bodies,
        // etc.) the legacy maybe_self_field_heap_copy path stays
        // unchanged.
        let inplace_helper_name: Option<&'static str> = match slot_ty {
            CodegenTy::String => Some("lotus_str_assign_in_place"),
            CodegenTy::Bytes => Some("lotus_bytes_assign_in_place"),
            _ => None,
        };
        if let Some(helper_name) = inplace_helper_name {
            if self.current_method_scratch.is_some() {
                let cs = self
                    .current_self
                    .clone()
                    .expect("scratch active implies current_self");
                let info = self
                    .user_loci
                    .get(&cs.locus_name)
                    .cloned()
                    .expect("current_self points to a declared locus");
                let ptr_t = self
                    .context
                    .ptr_type(AddressSpace::default());
                let arena_field_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        cs.self_ptr,
                        info.arena_field_idx,
                        "self.__arena.for_heap_assign_ptr",
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                let dest_arena = self
                    .builder
                    .build_load(
                        ptr_t,
                        arena_field_ptr,
                        "self.__arena.for_heap_assign",
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .into_pointer_value();
                let existing = self
                    .builder
                    .build_load(
                        ptr_t,
                        slot_ptr,
                        "self_field.heap.existing",
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .into_pointer_value();
                let assign_fn = self
                    .module
                    .get_function(helper_name)
                    .expect("in-place assign helper declared");
                let result = self
                    .builder
                    .build_call(
                        assign_fn,
                        &[
                            dest_arena.into(),
                            existing.into(),
                            rhs.into_pointer_value().into(),
                        ],
                        "self_field.heap.assign_in_place",
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?
                    .try_as_basic_value()
                    .left()
                    .expect("assign_in_place helper returns ptr");
                self.builder
                    .build_store(slot_ptr, result)
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
                return Ok(());
            }
        }

        if !compound {
            let value = self.maybe_self_field_heap_copy(rhs, slot_ty)?;
            self.builder
                .build_store(slot_ptr, value)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok(());
        }

        // Without method scratch (e.g., a synthesized closure-eval
        // body), the rhs is already in self.__arena or static
        // — no anchoring needed. Preserve the legacy store-the-
        // pointer behavior; mutating in place isn't a meaningful
        // optimization in those contexts.
        if self.current_method_scratch.is_none() {
            self.builder
                .build_store(slot_ptr, rhs)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok(());
        }

        // In-place compound mutation.
        let cs = self
            .current_self
            .clone()
            .expect("scratch active implies current_self");
        let info = self
            .user_loci
            .get(&cs.locus_name)
            .cloned()
            .expect("current_self points to a declared locus");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arena_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                cs.self_ptr,
                info.arena_field_idx,
                "self.__arena.for_inplace_ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let dest_arena = self
            .builder
            .build_load(
                ptr_t,
                arena_field_ptr,
                "self.__arena.for_inplace",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();

        // Existing struct pointer — slot is guaranteed non-null
        // because every locus field has either a default or an
        // instantiation-time value (typecheck-enforced), so by
        // the time any method body runs the slot has been
        // populated by lower_locus_instantiation's field init.
        // Loaded BEFORE the anchor walk: the Gap A fixup below
        // needs the pre-replace field pointers.
        let existing_ptr = self
            .builder
            .build_load(ptr_t, slot_ptr, "self_field.existing")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();

        // Gap A (2026-07-17, notes/anchor-retirement.md): a compound
        // store REPLACES the existing struct's String clones — the
        // memcpy below overwrites their pointers, orphaning them in
        // the locus lifetime arena (the sme/causality construction-
        // position leak). Capture, per String field, the pointer
        // being replaced (oldp, from the existing struct) and the
        // rhs field pointer BEFORE the anchor walk rewrites it
        // (rawp); after the memcpy, one runtime call per field
        // retires oldp and force-copies a skip-shared same-arena
        // pointer so self-storage slots never alias
        // (lotus_str_field_replace_fixup). String and — GH #1033 —
        // Bytes fields (lotus_bytes_field_replace_fixup): a Bytes
        // field's replaced blob was never retired, so `self.f =
        // Frame { data: b"", .. }` in a loop grew one blob per write.
        // Nested compound fields keep today's behavior.
        let mut retire_fields: Vec<(
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            &'static str,
        )> = Vec::new();
        if let CodegenTy::TypeRef(tn) = slot_ty {
            if let Some(tinfo) = self.user_types.get(tn).cloned() {
                let rhs_ptr = rhs.into_pointer_value();
                for fname in &tinfo.field_order {
                    let (idx, fty) = tinfo
                        .fields
                        .get(fname)
                        .cloned()
                        .expect("field_order lists declared fields");
                    let fixup_name = match fty {
                        CodegenTy::String => "lotus_str_field_replace_fixup",
                        CodegenTy::Bytes => "lotus_bytes_field_replace_fixup",
                        _ => continue,
                    };
                    let old_slot = self
                        .builder
                        .build_struct_gep(
                            tinfo.struct_ty,
                            existing_ptr,
                            idx,
                            &format!("retire.{}.old.ptr", fname),
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?;
                    let oldp = self
                        .builder
                        .build_load(
                            ptr_t,
                            old_slot,
                            &format!("retire.{}.old", fname),
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?
                        .into_pointer_value();
                    let raw_slot = self
                        .builder
                        .build_struct_gep(
                            tinfo.struct_ty,
                            rhs_ptr,
                            idx,
                            &format!("retire.{}.raw.ptr", fname),
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?;
                    let rawp = self
                        .builder
                        .build_load(
                            ptr_t,
                            raw_slot,
                            &format!("retire.{}.raw", fname),
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?
                        .into_pointer_value();
                    // old_slot doubles as the destination slot for
                    // the post-memcpy fixup (same GEP).
                    retire_fields.push((old_slot, oldp, rawp, fixup_name));
                }
            }
        }

        // Anchor rhs's heap fields in self.__arena. Same-arena
        // skip and static-literal skip from 6a56d7c make this
        // identity for the common RMW + literal-field patterns.
        // For compound types, this rewrites rhs's struct fields
        // in place; the returned pointer is rhs's own pointer.
        // GH #1490: the struct's top-level String fields — the ones
        // the fixups below retire — clone into field blocks.
        self.slot_field_clone = !retire_fields.is_empty();
        // A form whose cells' nested blocks are retired (a `@form(vec)`
        // element replaced or popped, a `@form(hashmap)` cell replaced
        // or removed) keeps those blocks in this very arena. The walk's
        // same-arena skip would store a block a cell still owns into
        // this field — `self.saved = self.get(k)` in the map's own
        // method, or a value visited in place by `for x in self.items` —
        // and the cell's retire would then free it under the field.
        // Here the field takes its own deep copy of every nested block
        // (`cell_compound_owner`, always-copy), and the Strings and
        // Bytes inside those copies are owned too (`cell_owned_clone`).
        // The walk rewrites the struct it walks, field by field, so it
        // walks a stack SNAPSHOT of the rhs, as `@form(hashmap).set`
        // does: the rhs may be a cell itself (`for x in self.items {
        // self.saved = x; }` visits the vec's elements in place), and
        // rewriting it would hand the element and the field one fresh
        // block, which the element's retire then frees under the field.
        //
        // The same rule holds wherever the store itself retires nested
        // blocks: a value that owns a block of its own — a record with
        // a nested record, tuple, out-of-line array or payload enum, or
        // a tuple or out-of-line array itself — has the blocks the
        // replaced value alone reached retired after the memcpy
        // (`lotus_self_field_retire_replaced`, below). A block shared
        // with any other owner (another field, a cell, a child's
        // param) would be freed under it, so the field owns every
        // nested block it stores: a copy, unless it is the very block
        // the field already holds at that place (`cell_compound_owner`
        // with the existing value — `self.f = F { n: i, inner:
        // self.f.inner }` keeps `inner` and copies nothing).
        let owns_retiring_cells = self.locus_retires_cell_blocks(&cs.locus_name);
        let nested_desc = self.self_field_retire_desc(slot_ty);
        let prev_owned = self.cell_owned_clone;
        let mut rhs = rhs;
        if owns_retiring_cells || nested_desc.is_some() {
            // The walk's outermost record takes it (a tuple or array is
            // deep-copied whole, every block of it fresh).
            if matches!(slot_ty, CodegenTy::TypeRef(_)) {
                self.cell_compound_owner =
                    if owns_retiring_cells { Some(None) } else { Some(Some(existing_ptr)) };
            }
            self.cell_owned_clone = true;
            if let CodegenTy::TypeRef(tn) = slot_ty {
                if let Some(tinfo) = self.user_types.get(tn).cloned() {
                    let snap = self.alloca_in_entry(tinfo.struct_ty.into(), "self_field.snap")?;
                    let snap_size = tinfo.struct_ty.size_of().expect("struct has known size");
                    self.builder
                        .build_memcpy(snap, 8, rhs.into_pointer_value(), 8, snap_size)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    rhs = snap.into();
                }
            }
        }
        let anchored = self.emit_cross_arena_store_deep_copy(
            rhs,
            slot_ty,
            dest_arena,
            "self_field_inplace",
        );
        self.slot_field_clone = false;
        self.cell_compound_owner = None;
        self.cell_owned_clone = prev_owned;
        let anchored = anchored?;

        let size = self.compound_storage_size(slot_ty)?;

        // What the replaced value reached, read before the memcpy
        // overwrites it: a stack copy of the block's bytes, walked by
        // the retire below.
        let old_snap = match &nested_desc {
            Some(desc) if !desc.is_empty() => {
                let storage = self.self_field_block_type(slot_ty)?;
                let snap = self.alloca_in_entry(storage, "self_field.old_snap")?;
                self.emit_memcpy_call(snap, existing_ptr, size, "self_field.old_snap.memcpy")?;
                Some(snap)
            }
            _ => None,
        };

        self.emit_memcpy_call(
            existing_ptr,
            anchored.into_pointer_value(),
            size,
            "self_field_inplace.memcpy",
        )?;

        if let Some(desc) = nested_desc {
            // A tuple or array value was deep-copied whole into a fresh
            // block of this arena, and its bytes now live in the
            // field's block: the fresh block is scrap.
            let scrap = if matches!(slot_ty, CodegenTy::TypeRef(_)) {
                ptr_t.const_null()
            } else {
                anchored.into_pointer_value()
            };
            let i64_t = self.context.i64_type();
            let scrap_size = if matches!(slot_ty, CodegenTy::TypeRef(_)) {
                i64_t.const_zero()
            } else {
                self.builder
                    .build_int_z_extend_or_bit_cast(size, i64_t, "self_field.scrap_size")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };
            let desc_ptr = if desc.is_empty() {
                ptr_t.const_null()
            } else {
                self.self_field_retire_desc_global(&desc).as_pointer_value()
            };
            let old_ptr = old_snap.unwrap_or_else(|| ptr_t.const_null());
            let retire_fn = self
                .module
                .get_function("lotus_self_field_retire_replaced")
                .expect("lotus_self_field_retire_replaced declared");
            self.builder
                .build_call(
                    retire_fn,
                    &[
                        dest_arena.into(),
                        old_ptr.into(),
                        existing_ptr.into(),
                        scrap.into(),
                        scrap_size.into(),
                        desc_ptr.into(),
                        i64_t.const_int(desc.len() as u64, false).into(),
                    ],
                    "self_field_inplace.retire_nested",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        if !retire_fields.is_empty() {
            for (dest_slot, oldp, rawp, fixup_name) in retire_fields {
                let fixup_fn = self
                    .module
                    .get_function(fixup_name)
                    .expect("field replace fixup declared");
                self.builder
                    .build_call(
                        fixup_fn,
                        &[
                            dest_arena.into(),
                            dest_slot.into(),
                            oldp.into(),
                            rawp.into(),
                        ],
                        "self_field_inplace.retire_fixup",
                    )
                    .map_err(|e| {
                        CodegenError::LlvmEmit(e.to_string())
                    })?;
            }
        }

        // No build_store — slot's pointer is unchanged.
        Ok(())
    }

    pub(crate) fn maybe_self_field_heap_copy(
        &mut self,
        value: BasicValueEnum<'ctx>,
        slot_ty: &CodegenTy,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if self.current_method_scratch.is_none() {
            return Ok(value);
        }
        if !Self::ty_needs_self_field_deep_copy(slot_ty) {
            return Ok(value);
        }
        let cs = self
            .current_self
            .clone()
            .expect("scratch active implies current_self");
        let info = self
            .user_loci
            .get(&cs.locus_name)
            .cloned()
            .expect("current_self points to a declared locus");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arena_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                cs.self_ptr,
                info.arena_field_idx,
                "self.__arena.for_field_copy",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let dest_arena = self
            .builder
            .build_load(ptr_t, arena_field_ptr, "self.__arena.field_dest")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        self.emit_return_value_deep_copy(value, slot_ty, dest_arena)
    }

}
