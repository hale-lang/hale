//! Return-path codegen: deep-copy emission for locus-method
//! returns (m49 method-with-scratch + m90 caller-arena placement)
//! + the in-place self-field assign machinery. This is the home
//! for issue #9 (m90 return-slot ABI) work. Round 7c of the
//! codegen model-org refactor — the missing 4.5 from the original
//! Round 4 plan.
//!
//! Lifted as inherent `impl<'ctx, 'p> Cx<'ctx, 'p>` blocks — call
//! sites need no `use` import.

use hale_syntax::ast::Expr;
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, CodegenTy, Cx, EnumInfo, Scope, TypeInfo};

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
            | CodegenTy::Decimal
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

    pub(crate) fn emit_return_value_deep_copy(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        dest_arena: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match ty {
            CodegenTy::Int
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal
            | CodegenTy::Time
            | CodegenTy::Duration
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
                let f = self
                    .module
                    .get_function("lotus_str_clone")
                    .expect("lotus_str_clone declared");
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
                let f = self
                    .module
                    .get_function("lotus_bytes_clone")
                    .expect("lotus_bytes_clone declared");
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
                // dest_arena, then recursively deep-copy each
                // element. Layout matches Expr::Tuple lowering so
                // tup.0 / tup.1 reads work identically on the
                // returned tuple.
                let storage_ty = self.llvm_tuple_storage_type(elem_tys);
                let bytes = storage_ty
                    .size_of()
                    .expect("tuple storage type has known size");
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc")
                    .expect("lotus_arena_alloc declared");
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
                    .get_function("lotus_arena_alloc")
                    .expect("lotus_arena_alloc declared");
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
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc")
                    .expect("lotus_arena_alloc declared");
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
                    let dst_slot = self
                        .builder
                        .build_struct_gep(
                            struct_ty,
                            new_struct,
                            idx,
                            &format!("fn.ret.struct.dst.{}", fname),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
            let new_ptr =
                self.lower_enum_variant_alloc(info, i as u32, &copied_fields)?;
            self.current_arena_override = prev_override;
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            variant_blocks.push((
                i32_t.const_int(i as u64, false),
                bb,
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
            .map(|(c, bb, _)| (*c, *bb))
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
        for (_, bb, val) in &variant_blocks {
            incoming.push((val, *bb));
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
        // Scalars + views + LocusRefs + Cells: store the value
        // directly. No anchoring needed (the value's bytes live
        // in the slot itself, or the slot holds a stable pointer
        // to a long-lived locus).
        let compound = match slot_ty {
            CodegenTy::Int
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal
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

        // Anchor rhs's heap fields in self.__arena. Same-arena
        // skip and static-literal skip from 6a56d7c make this
        // identity for the common RMW + literal-field patterns.
        // For compound types, this rewrites rhs's struct fields
        // in place; the returned pointer is rhs's own pointer.
        let anchored = self.emit_cross_arena_store_deep_copy(
            rhs,
            slot_ty,
            dest_arena,
            "self_field_inplace",
        )?;

        // Existing struct pointer — slot is guaranteed non-null
        // because every locus field has either a default or an
        // instantiation-time value (typecheck-enforced), so by
        // the time any method body runs the slot has been
        // populated by lower_locus_instantiation's field init.
        let existing_ptr = self
            .builder
            .build_load(ptr_t, slot_ptr, "self_field.existing")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();

        let size = self.compound_storage_size(slot_ty)?;

        self.emit_memcpy_call(
            existing_ptr,
            anchored.into_pointer_value(),
            size,
            "self_field_inplace.memcpy",
        )?;

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
