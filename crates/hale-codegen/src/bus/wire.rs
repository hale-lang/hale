//! Bus wire-format codegen: per-payload-type serialize / deserialize
//! function synthesis. Emits LLVM functions that walk struct payloads
//! field-by-field (m70 wire format) into / out of caller buffers.
//! Round 3a of the codegen model-org refactor.
//!
//! Every value is dispatched on its shape in the carriage table
//! (`hale_types::wire_carriage::WIRE_TABLE`, GH #1951 item 4), the table
//! the check's refusal prints its list from: `emit_wire_ser_value` and
//! `emit_wire_de_value` are the one walk, and a record's fields, a list's
//! and an array's elements, a tuple's and an enum variant's values all go
//! through them.

use std::collections::BTreeMap;

use inkwell::types::StructType;
use inkwell::values::PointerValue;
use inkwell::AddressSpace;

use hale_types::wire_carriage::{self, WireShape};

use crate::codegen::{codegen_ty_size_bytes, CodegenError, CodegenTy, Cx, EnumInfo, SerializerPair};

pub(crate) trait BusWire<'ctx> {
    fn synthesize_serializer(
        &mut self,
        type_name: &str,
    ) -> Result<(), CodegenError>;
    /// A memcpy into the wire that honours the serializer's cap (GH
    /// #1155): outside a serializer body it is the plain memcpy.
    fn emit_ser_memcpy(
        &mut self,
        dst: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        n: inkwell::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<(), CodegenError>;

    fn emit_bounded_field_wire_memcpy(
        &mut self,
        field_storage_ptr: PointerValue<'ctx>,
        wire_buf: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        elem_ty: &CodegenTy,
        n: u64,
        to_wire: bool,
        site: &str,
        bound: Option<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )>,
    ) -> Result<(), CodegenError>;
    fn emit_de_need(
        &mut self,
        cursor: inkwell::values::IntValue<'ctx>,
        nbytes: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        site: &str,
    ) -> Result<(), CodegenError>;
    fn emit_de_remaining(
        &mut self,
        cursor: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        site: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError>;
    fn emit_de_legacy_len_check(
        &mut self,
        len: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        site: &str,
    ) -> Result<(), CodegenError>;
    fn emit_de_len_fits(
        &mut self,
        len: inkwell::values::IntValue<'ctx>,
        after_prefix: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        site: &str,
    ) -> Result<(), CodegenError>;
    fn wire_min_size(&self, t: &CodegenTy) -> u64;
    fn wire_min_size_of_fields(
        &self,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> u64;

    fn emit_array_field_serialize(
        &mut self,
        src_field_ptr: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        elem_ty: &CodegenTy,
        n: u64,
        fname: &str,
    ) -> Result<(), CodegenError>;
    fn emit_array_field_deserialize(
        &mut self,
        dst_field_ptr: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        elem_ty: &CodegenTy,
        n: u64,
        fname: &str,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<(), CodegenError>;
    fn emit_per_field_serialize(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError>;
    fn emit_per_field_deserialize(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError>;
    fn emit_per_field_deserialize_size(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError>;
}

impl<'ctx, 'p> BusWire<'ctx> for Cx<'ctx, 'p> {
    fn emit_ser_memcpy(
        &mut self,
        dst: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        n: inkwell::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<(), CodegenError> {
        let Some((base, cap)) = self.ser_bound else {
            return self.emit_memcpy_call(dst, src, n, name);
        };
        let i64_t = self.context.i64_type();
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let dst_i = self.builder.build_ptr_to_int(dst, i64_t, &format!("{name}.dst.i")).map_err(e)?;
        let base_i = self.builder.build_ptr_to_int(base, i64_t, &format!("{name}.base.i")).map_err(e)?;
        let off = self.builder.build_int_sub(dst_i, base_i, &format!("{name}.off")).map_err(e)?;
        let end = self.builder.build_int_add(off, n, &format!("{name}.end")).map_err(e)?;
        let fits = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULE, end, cap, &format!("{name}.fits"))
            .map_err(e)?;
        let func = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .expect("serializer body has a function");
        let write_bb = self.context.append_basic_block(func, &format!("{name}.write"));
        let join_bb = self.context.append_basic_block(func, &format!("{name}.join"));
        self.builder.build_conditional_branch(fits, write_bb, join_bb).map_err(e)?;
        self.builder.position_at_end(write_bb);
        self.emit_memcpy_call(dst, src, n, name)?;
        self.builder.build_unconditional_branch(join_bb).map_err(e)?;
        self.builder.position_at_end(join_bb);
        Ok(())
    }
    fn synthesize_serializer(
        &mut self,
        type_name: &str,
    ) -> Result<(), CodegenError> {
        if self.serializers.contains_key(type_name) {
            return Ok(());     /* already synthesized */
        }
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // The C `lotus_serialize_fn` / `lotus_deserialize_fn` ABIs return
        // `ssize_t` and take `size_t` lengths — both target-pointer-width
        // (i64 native, i32 wasm32). The synthesized fns MUST match, or the
        // C runtime's `call_indirect` (typed from the typedef) traps with a
        // signature mismatch under wasm32. No-op on native (usize == i64).
        let usize_t = self.usize_type();

        // Decide the synthesis strategy. Three shapes:
        //   - Struct payload (struct_layout = Some): per-field
        //     walk (m70 wire format).
        //   - Enum payload whose every variant value is fixed bytes (a
        //     scalar or a payload-free enum's tag; enum_size = Some,
        //     enum_walk = None): a memcpy of the enum's storage struct,
        //     as it always was.
        //   - Any other enum payload (enum_walk = Some, GH #1951 item 4):
        //     its tag, then the variant's values, each as the field walk
        //     carries it. Until then a `String` value was refused here and
        //     any other pointer (`Bytes`, a record, a list) was copied as
        //     the pointer.
        let struct_layout: Option<(
            StructType<'ctx>,
            Vec<String>,
            BTreeMap<String, (u32, CodegenTy)>,
        )>;
        let enum_size: Option<inkwell::values::IntValue<'ctx>>;
        let mut enum_walk: Option<EnumInfo> = None;
        if let Some(info) = self.user_types.get(type_name).cloned() {
            self.wire_check(&CodegenTy::TypeRef(type_name.to_string()), "", &mut Vec::new())?;
            struct_layout =
                Some((info.struct_ty, info.field_order, info.fields));
            enum_size = None;
        } else if let Some(info) = self.user_enums.get(type_name).cloned() {
            if !info.has_payload {
                return Err(CodegenError::Unsupported(format!(
                    "bus payload `{}` is a no-payload enum; wrap in a \
                     struct or add a variant payload",
                    type_name
                )));
            }
            self.wire_check(&CodegenTy::Enum(type_name.to_string()), "", &mut Vec::new())?;
            let fixed = info.variants.iter().all(|v| {
                v.field_tys
                    .iter()
                    .all(|t| matches!(self.wire_shape(t), Ok(WireShape::Scalar | WireShape::PayloadFreeEnum)))
            });
            let size = self
                .enum_storage_struct(&info)
                .size_of()
                .expect("enum storage struct has known size");
            if !fixed {
                enum_walk = Some(info);
            }
            struct_layout = None;
            enum_size = Some(size);
        } else {
            return Err(CodegenError::Unsupported(format!(
                "synthesize_serializer: type `{}` not declared",
                type_name
            )));
        }
        let _ = i32_t;
        let _ = i8_t;

        let saved_block = self.builder.get_insert_block();

        // ssize_t @__serialize_T(ptr src, ptr dst, size_t cap)
        let ser_ty = usize_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), usize_t.into()],
            false,
        );
        let ser_fn = self.module.add_function(
            &format!("__serialize_{}", type_name),
            ser_ty,
            None,
        );
        let ser_entry = self.context.append_basic_block(ser_fn, "entry");
        self.builder.position_at_end(ser_entry);
        let ser_src = ser_fn
            .get_nth_param(0)
            .expect("ser src arg")
            .into_pointer_value();
        let ser_dst = ser_fn
            .get_nth_param(1)
            .expect("ser dst arg")
            .into_pointer_value();
        // GH #1155: the cap is honoured — see `emit_ser_memcpy`.
        let cap = ser_fn.get_nth_param(2).expect("ser cap arg").into_int_value();
        let cap = if cap.get_type() == self.context.i64_type() {
            cap
        } else {
            self.builder
                .build_int_z_extend(cap, self.context.i64_type(), "ser.cap.i64")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        self.ser_bound = Some((ser_dst, cap));

        let total_written: inkwell::values::IntValue<'ctx> =
            if let Some((struct_ty, field_order, fields)) = &struct_layout
            {
                self.emit_per_field_serialize(
                    ser_src,
                    ser_dst,
                    *struct_ty,
                    field_order,
                    fields,
                )?
            } else if let Some(info) = &enum_walk {
                let cursor_alloca = self.wire_alloca(i64_t.into(), "ser.cursor")?;
                self.builder
                    .build_store(cursor_alloca, i64_t.const_zero())
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.emit_wire_ser_enum_cell(ser_src, info, ser_dst, cursor_alloca, type_name)?;
                self.builder
                    .build_load(i64_t, cursor_alloca, "ser.cursor.final")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value()
            } else {
                let size_iv = enum_size.expect("enum size present");
                self.emit_ser_memcpy(
                    ser_dst,
                    ser_src,
                    size_iv,
                    "ser.memcpy",
                )?;
                size_iv
            };
        self.ser_bound = None;
        // The body computes the byte count in i64; narrow to the ssize_t
        // return width (no-op native, i64->i32 trunc on wasm32).
        let total_written = self.size_to_usize(total_written)?;
        self.builder
            .build_return(Some(&total_written))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Two decoders of the one walk (2026-11-07, downstream handoff):
        // `__deserialize_T`, the local one — straight-line, no bound
        // checks, what the registry, a cross-pool create cell and every
        // in-process delivery decode with (a local cell is the program's
        // own serializer's bytes, not a wire frame; the create-cell
        // dispatch is one basic block, which lifecycle_flow holds) — and
        // `__deserialize_wire_T`, every read held to the frame, the
        // transport readers' entry (`lotus_bus_wire_decoder`).
        let mut de_fns = Vec::new();
        for (suffix, checked) in [("", false), ("wire_", true)] {
            self.de_checked = checked;
            // ssize_t @__deserialize_T(ptr src, size_t n, ptr dst, size_t cap)
            let de_ty = usize_t.fn_type(
                &[ptr_t.into(), usize_t.into(), ptr_t.into(), usize_t.into()],
                false,
            );
            let de_fn = self.module.add_function(
                &format!("__deserialize_{}{}", suffix, type_name),
                de_ty,
                None,
            );
            let de_entry = self.context.append_basic_block(de_fn, "entry");
            // 2026-05-27 — error block for length-prefix bound-check
            // failures inside variable-length field paths
            // (`String` / `Bytes`). The reader-thread caller
            // observes the -1 return as "drop this datagram" via the
            // existing `if (struct_size <= 0) continue;` guard. The
            // String/Bytes paths in `emit_per_field_deserialize{,_size}`
            // branch here when a decoded length is negative or
            // exceeds the remaining wire bytes — closes a real fault
            // mode (corrupt or cross-routed datagram) that would
            // otherwise hand a giant `size` straight to
            // `lotus_bus_payload_arena_alloc`, triggering the arena
            // cap-hit / NULL-deref symptom a downstream app reported.
            let de_fail = self.context.append_basic_block(de_fn, "wire.fail");
            self.builder.position_at_end(de_fail);
            // -1 in the ssize_t return width (i32 0xFFFFFFFF on wasm32); the
            // C caller checks `<= 0` to drop the datagram.
            let neg_one = usize_t.const_int((-1i64) as u64, true);
            self.builder
                .build_return(Some(&neg_one))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(de_entry);
            let de_src = de_fn
                .get_nth_param(0)
                .expect("de src arg")
                .into_pointer_value();
            let de_n = de_fn
                .get_nth_param(1)
                .expect("de n arg")
                .into_int_value();
            // The body's bound checks are i64; widen the size_t `n` (i32 on
            // wasm32) to i64 (no-op native).
            let de_n = if de_n.get_type() == i64_t {
                de_n
            } else {
                self.builder
                    .build_int_z_extend(de_n, i64_t, "de_n.i64")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };
            let de_dst = de_fn
                .get_nth_param(2)
                .expect("de dst arg")
                .into_pointer_value();
            let _ = de_fn.get_nth_param(3); // cap, reserved

            // A frame shorter than the type's minimum encoded size is
            // refused before any field is read (the empty datagram
            // included: no struct's form is shorter than its fixed
            // fields and length prefixes); the field walk then holds each
            // variable-length read to what remains.
            let wire_min = match (&struct_layout, &enum_walk) {
                (Some((_, field_order, fields)), _) => {
                    i64_t.const_int(self.wire_min_size_of_fields(field_order, fields), false)
                }
                // the tag
                (None, Some(_)) => i64_t.const_int(4, false),
                (None, None) => enum_size.expect("enum size present"),
            };
            self.emit_de_need(i64_t.const_int(0, false), wire_min, de_n, de_fail, "de.min")?;

            let de_struct_size: inkwell::values::IntValue<'ctx> =
                if let Some((struct_ty, field_order, fields)) = &struct_layout
                {
                    self.emit_per_field_deserialize(
                        de_src,
                        de_dst,
                        *struct_ty,
                        field_order,
                        fields,
                        de_n,
                        de_fail,
                    )?
                } else if let Some(info) = &enum_walk {
                    // decoded into the delivery's cell in place: the walk's
                    // pointers land in the payload arena
                    let cursor_alloca = self.wire_alloca(i64_t.into(), "de.cursor")?;
                    self.builder
                        .build_store(cursor_alloca, i64_t.const_zero())
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.emit_wire_de_enum_cell(de_dst, info, de_src, cursor_alloca, de_n, de_fail, type_name)?;
                    enum_size.expect("enum size present")
                } else {
                    // the frame holds the cell: the `de.min` check above
                    let size_iv = enum_size.expect("enum size present");
                    self.emit_memcpy_call(
                        de_dst,
                        de_src,
                        size_iv,
                        "de.memcpy",
                    )?;
                    size_iv
                };
            // Narrow the i64 struct size to the ssize_t return width.
            let de_struct_size = self.size_to_usize(de_struct_size)?;
            self.builder
                .build_return(Some(&de_struct_size))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            de_fns.push(de_fn);
        }
        self.de_checked = false;

        if let Some(b) = saved_block {
            self.builder.position_at_end(b);
        }

        self.serializers.insert(
            type_name.to_string(),
            SerializerPair { serialize: ser_fn, deserialize: de_fns[0], deserialize_wire: de_fns[1] },
        );
        Ok(())
    }

    /// Form H (2026-05-20): emit serialize IR for a fixed-size
    /// array bus payload field. Two element shapes supported:
    /// fixed-size primitives (single memcpy N*elem_size) and
    /// TypeRefs (loop N times, load each pointer slot, recurse
    /// on emit_per_field_serialize). The loop is unrolled at
    /// codegen-time — N is statically known and typically small
    /// (~10–20 for the canonical fixed-cap-array use case).
    /// bounded[T; N] (2026-07-02): scalar-element bounded fields
    /// serialize as their raw inline `{ i64 len, [N x T] }` bytes —
    /// fixed size, count travels in the bytes. Pointer-shaped
    /// elements (String/Bytes/TypeRef/an enum with payloads) travel
    /// counted instead (`emit_wire_ser_counted`, GH #1951 item 4).
    fn emit_bounded_field_wire_memcpy(
        &mut self,
        field_storage_ptr: PointerValue<'ctx>,
        wire_buf: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        elem_ty: &CodegenTy,
        n: u64,
        to_wire: bool,
        site: &str,
        bound: Option<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )>,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let st = self.llvm_bounded_storage_type(elem_ty, n);
        let total_iv = st.size_of().expect("bounded storage sized");
        let cursor_iv = self
            .builder
            .build_load(i64_t, cursor_alloca, &format!("{}.cursor", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        // Deserializing: the frame holds the whole storage, or the
        // decode fails.
        if let Some((wire_n, fail_block)) = bound {
            self.emit_de_need(cursor_iv, total_iv, wire_n, fail_block, site)?;
        }
        let wire_at_cursor = unsafe {
            self.builder
                .build_gep(
                    i8_t,
                    wire_buf,
                    &[cursor_iv],
                    &format!("{}.wire", site),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let (dst, src) = if to_wire {
            (wire_at_cursor, field_storage_ptr)
        } else {
            (field_storage_ptr, wire_at_cursor)
        };
        // GH #1174: the cap only means anything when writing INTO the
        // wire buffer; a deserialize copy writes into the field's own
        // storage, so it goes through the plain memcpy, not the
        // serializer's cap check.
        if to_wire {
            self.emit_ser_memcpy(dst, src, total_iv, site)?;
        } else {
            self.emit_memcpy_call(dst, src, total_iv, site)?;
            // The wire twin holds the count the storage carries to the
            // bound: a frame saying more elements than `N` would be a
            // malformed bounded object delivered (reviewer, 2026-11-07).
            if let Some((_, fail_block)) = bound {
                if self.de_checked {
                    let count = self
                        .builder
                        .build_load(i64_t, field_storage_ptr, &format!("{}.count", site))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let too_many = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::UGT,
                            count,
                            i64_t.const_int(n, false),
                            &format!("{}.count.bad", site),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let ok_block = self.context.append_basic_block(
                        self.builder
                            .get_insert_block()
                            .expect("insert block set")
                            .get_parent()
                            .expect("block has parent fn"),
                        &format!("{}.count.ok", site),
                    );
                    self.builder
                        .build_conditional_branch(too_many, fail_block, ok_block)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder.position_at_end(ok_block);
                }
            }
        }
        let after = self
            .builder
            .build_int_add(cursor_iv, total_iv, &format!("{}.after", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(cursor_alloca, after)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// The frame has `nbytes` more bytes at `cursor`, or the decode
    /// fails: `cursor + nbytes <= wire_n`, unsigned, and a sum that
    /// wraps (a length read off the wire as a negative i64) fails too.
    /// Every read of a generated deserializer goes through it, so a
    /// frame shorter than its type is refused (-1) rather than decoded
    /// from whatever the reader's buffer held before it (downstream
    /// handoff, 2026-11-07: an empty datagram was delivered as `n=0`,
    /// and a one-byte one as the previous frame with its first byte
    /// replaced).
    fn emit_de_need(
        &mut self,
        cursor: inkwell::values::IntValue<'ctx>,
        nbytes: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        site: &str,
    ) -> Result<(), CodegenError> {
        // the local decoder reads straight through: its bytes are the
        // program's own serializer's, and its create-cell dispatch must
        // stay one block
        if !self.de_checked {
            return Ok(());
        }
        let end = self
            .builder
            .build_int_add(cursor, nbytes, &format!("{}.need.end", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let past = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::UGT,
                end,
                wire_n,
                &format!("{}.need.past", site),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let wrapped = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::ULT,
                end,
                cursor,
                &format!("{}.need.wrapped", site),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let bad = self
            .builder
            .build_or(past, wrapped, &format!("{}.need.bad", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ok_block = self.context.append_basic_block(
            self.builder
                .get_insert_block()
                .expect("insert block set")
                .get_parent()
                .expect("block has parent fn"),
            &format!("{}.need.ok", site),
        );
        self.builder
            .build_conditional_branch(bad, fail_block, ok_block)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(ok_block);
        Ok(())
    }

    /// The local decoder's one guard, the one it always had
    /// (2026-05-27): a String's or Bytes's announced length may not
    /// exceed the bytes it was given, so a corrupt length never reaches
    /// the payload arena as a giant allocation. The wire decoder's
    /// `emit_de_need` subsumes it.
    fn emit_de_legacy_len_check(
        &mut self,
        len: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        site: &str,
    ) -> Result<(), CodegenError> {
        if self.de_checked {
            return Ok(());
        }
        let too_big = self
            .builder
            .build_int_compare(inkwell::IntPredicate::UGT, len, wire_n, &format!("{}.len.bad", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ok_block = self.context.append_basic_block(
            self.builder
                .get_insert_block()
                .expect("insert block set")
                .get_parent()
                .expect("block has parent fn"),
            &format!("{}.len.ok", site),
        );
        self.builder
            .build_conditional_branch(too_big, fail_block, ok_block)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(ok_block);
        Ok(())
    }

    /// The wire twin's check of a raw length read off the frame, before
    /// anything is added to it or allocated from it: `len <= wire_n -
    /// after_prefix`, unsigned (`after_prefix <= wire_n` holds, the
    /// prefix's own read having passed `emit_de_need`), so a negative
    /// length fails here as a huge one, and never reaches a sum that
    /// could wrap (reviewer, 2026-11-07: a Bytes length of -8 made
    /// `len + 8` zero, and a zero-byte Bytes object was delivered with
    /// the tail read from the prefix). A no-op in the local decoder.
    fn emit_de_len_fits(
        &mut self,
        len: inkwell::values::IntValue<'ctx>,
        after_prefix: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        site: &str,
    ) -> Result<(), CodegenError> {
        if !self.de_checked {
            return Ok(());
        }
        let remaining = self
            .builder
            .build_int_sub(wire_n, after_prefix, &format!("{}.fits.remaining", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let too_big = self
            .builder
            .build_int_compare(inkwell::IntPredicate::UGT, len, remaining, &format!("{}.fits.bad", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ok_block = self.context.append_basic_block(
            self.builder
                .get_insert_block()
                .expect("insert block set")
                .get_parent()
                .expect("block has parent fn"),
            &format!("{}.fits.ok", site),
        );
        self.builder
            .build_conditional_branch(too_big, fail_block, ok_block)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(ok_block);
        Ok(())
    }

    /// The bytes of the frame left after `cursor`: what a nested
    /// struct's walk, which reads from `src + cursor`, may consume.
    /// `cursor <= wire_n` holds here, every read before it having
    /// passed `emit_de_need`.
    fn emit_de_remaining(
        &mut self,
        cursor: inkwell::values::IntValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        site: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        self.builder
            .build_int_sub(wire_n, cursor, &format!("{}.remaining", site))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
    }

    /// A lower bound on the wire bytes a value of `t` encodes to: its
    /// fixed bytes, a String's or Bytes's 8-byte length prefix, a
    /// nested struct's fields' sum, an array's elements' sum, a bounded
    /// field's count and elements (its storage may carry alignment
    /// padding on top; the field's own check holds the exact size). A
    /// frame shorter than this is refused before any field is read.
    fn wire_min_size(&self, t: &CodegenTy) -> u64 {
        match t {
            CodegenTy::String | CodegenTy::Bytes => 8,
            CodegenTy::TypeRef(name) => self
                .user_types
                .get(name.as_str())
                .map(|info| self.wire_min_size_of_fields(&info.field_order, &info.fields))
                .unwrap_or(0),
            // the count and the elements at their wire size (an enum's
            // tag is four bytes: `codegen_ty_size_bytes` prices every
            // non-Decimal at eight, which overshot a `bounded[Enum; N]`
            // and refused every frame holding one)
            // a counted list (`WireShape::BoundedCounted`): its count alone
            CodegenTy::Bounded(elem, _) if self.bounded_elem_is_ptr(elem) => 8,
            CodegenTy::Bounded(elem, n) => 8 + n * self.wire_fixed_size(elem),
            CodegenTy::Array(elem, n) => n * self.wire_min_size(elem),
            // an enum's tag, with or without the values that follow it
            CodegenTy::Enum(_) => 4,
            CodegenTy::Tuple(es) => es.iter().map(|t| self.wire_min_size(t)).sum(),
            other if matches!(self.wire_shape(other), Ok(WireShape::Scalar)) => self.wire_fixed_size(other),
            _ => 0,
        }
    }

    fn wire_min_size_of_fields(
        &self,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> u64 {
        field_order
            .iter()
            .filter_map(|f| fields.get(f))
            .map(|(_, t)| self.wire_min_size(t))
            .sum()
    }

    fn emit_array_field_serialize(
        &mut self,
        src_field_ptr: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        elem_ty: &CodegenTy,
        n: u64,
        fname: &str,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // Inline fixed arrays (2026-07-01, array_inline_spec): the
        // field slot IS the [N x elem] storage — serialize straight
        // from it. Out-of-line arrays (non-scalar elements) keep the
        // legacy shape: the slot holds a pointer to separately-
        // allocated storage; load it before iterating.
        let arr_ptr = if Self::array_inline_spec(&CodegenTy::Array(
            Box::new(elem_ty.clone()),
            n,
        ))
        .is_some()
        {
            src_field_ptr
        } else {
            self.builder
                .build_load(ptr_t, src_field_ptr, "ser.arr.field.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value()
        };
        let arr_ty = self.llvm_array_storage_type(elem_ty, n);
        match elem_ty {
            CodegenTy::Int
            | CodegenTy::U64
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::Decimal(_)
            | CodegenTy::U256
            | CodegenTy::I256 => {
                let elem_bytes =
                    codegen_ty_size_bytes(self.context, elem_ty);
                let total_bytes = n * elem_bytes;
                let total_iv = i64_t.const_int(total_bytes, false);
                let cursor_iv = self
                    .builder
                    .build_load(i64_t, cursor_alloca, "ser.arr.cursor.load")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let dst_at_cursor = unsafe {
                    self.builder
                        .build_gep(i8_t, dst, &[cursor_iv], "ser.arr.dst")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                self.emit_ser_memcpy(
                    dst_at_cursor,
                    arr_ptr,
                    total_iv,
                    "ser.array.primitive.memcpy",
                )?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        total_iv,
                        "ser.cursor.after.arr.prim",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(())
            }
            CodegenTy::TypeRef(nested_name) => {
                let nested_info = self
                    .user_types
                    .get(nested_name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload array `{}: [{}; {}]` — nested \
                             type not declared",
                            fname, nested_name, n
                        ))
                    })?;
                for i in 0..n as usize {
                    let slot = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                arr_ty,
                                arr_ptr,
                                &[
                                    i64_t.const_zero(),
                                    i64_t.const_int(i as u64, false),
                                ],
                                "ser.arr.slot",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let nested_src = self
                        .builder
                        .build_load(ptr_t, slot, "ser.arr.nested.load")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let cursor_iv = self
                        .builder
                        .build_load(i64_t, cursor_alloca, "ser.arr.cursor.iter")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let dst_at_cursor = unsafe {
                        self.builder
                            .build_gep(
                                i8_t,
                                dst,
                                &[cursor_iv],
                                "ser.arr.dst.iter",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let written = self.emit_per_field_serialize(
                        nested_src,
                        dst_at_cursor,
                        nested_info.struct_ty,
                        &nested_info.field_order,
                        &nested_info.fields,
                    )?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            written,
                            "ser.cursor.after.arr.elem",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(())
            }
            other => Err(CodegenError::Unsupported(format!(
                "bus payload array `{}: [{}; {}]`: the record field walk takes \
                 scalars and records (`wire_array_fixed_walk`); any other element \
                 goes through the per-element loop",
                fname, other, n
            ))),        }
    }

    /// Form H (2026-05-20): emit deserialize IR for a fixed-size
    /// array bus payload field. Companion to
    /// `emit_array_field_serialize` — same N-unroll pattern, but
    /// allocates fresh nested structs on the deserialize side
    /// (matching the per-field TypeRef deserialize shape).
    fn emit_array_field_deserialize(
        &mut self,
        dst_field_ptr: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        elem_ty: &CodegenTy,
        n: u64,
        fname: &str,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // Allocate `[N x elem]` storage in the bus payload arena;
        // we'll store the pointer into the parent's array-field
        // slot (pointer-shaped) after filling. The arena alloc
        // matches what the array-repeat literal lowering does
        // (lower_expr's `Expr::ArrayRepeat` arm). 16-byte align
        // covers Decimal (i128) element types.
        let arr_ty = self.llvm_array_storage_type(elem_ty, n);
        // Inline fixed arrays (2026-07-01, array_inline_spec): the
        // parent's field slot IS the [N x elem] storage — decode
        // straight into it, no side allocation, no pointer store.
        // Out-of-line arrays keep the legacy shape below.
        let arr_ptr = if Self::array_inline_spec(&CodegenTy::Array(
            Box::new(elem_ty.clone()),
            n,
        ))
        .is_some()
        {
            dst_field_ptr
        } else {
            let arr_size = arr_ty
                .size_of()
                .expect("array storage type has known size");
            let alloc_fn = self
                .module
                .get_function("lotus_bus_payload_arena_alloc")
                .expect("lotus_bus_payload_arena_alloc declared");
            let p = self
                .builder
                .build_call(
                    alloc_fn,
                    &[
                        arr_size.into(),
                        i64_t.const_int(16, false).into(),
                    ],
                    "de.arr.alloc",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("payload arena alloc returns ptr")
                .into_pointer_value();
            // Store the array pointer in the parent's field slot now,
            // before filling — readers go through the pointer either
            // way and it simplifies error paths.
            self.builder
                .build_store(dst_field_ptr, p)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            p
        };
        let _ = ptr_t;
        match elem_ty {
            CodegenTy::Int
            | CodegenTy::U64
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::Decimal(_)
            | CodegenTy::U256
            | CodegenTy::I256 => {
                let elem_bytes =
                    codegen_ty_size_bytes(self.context, elem_ty);
                let total_bytes = n * elem_bytes;
                let total_iv = i64_t.const_int(total_bytes, false);
                let cursor_iv = self
                    .builder
                    .build_load(i64_t, cursor_alloca, "de.arr.cursor.load")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let src_at_cursor = unsafe {
                    self.builder
                        .build_gep(i8_t, src, &[cursor_iv], "de.arr.src")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                self.emit_de_need(cursor_iv, total_iv, wire_n, fail_block, "de.arr")?;
                self.emit_memcpy_call(
                    arr_ptr,
                    src_at_cursor,
                    total_iv,
                    "de.array.primitive.memcpy",
                )?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        total_iv,
                        "de.cursor.after.arr.prim",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(())
            }
            CodegenTy::TypeRef(nested_name) => {
                let nested_info = self
                    .user_types
                    .get(nested_name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload array `{}: [{}; {}]` — nested \
                             type not declared",
                            fname, nested_name, n
                        ))
                    })?;
                let nested_size = nested_info
                    .struct_ty
                    .size_of()
                    .expect("nested struct has known size");
                let nested_alloc_fn = self
                    .module
                    .get_function("lotus_bus_payload_arena_alloc")
                    .expect("lotus_bus_payload_arena_alloc declared");
                for i in 0..n as usize {
                    let slot = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                arr_ty,
                                arr_ptr,
                                &[
                                    i64_t.const_zero(),
                                    i64_t.const_int(i as u64, false),
                                ],
                                "de.arr.slot",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let nested_dst = self
                        .builder
                        .build_call(
                            nested_alloc_fn,
                            &[
                                nested_size.into(),
                                i64_t.const_int(16, false).into(),
                            ],
                            "de.arr.nested.alloc",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("payload arena alloc returns ptr")
                        .into_pointer_value();
                    let cursor_iv = self
                        .builder
                        .build_load(i64_t, cursor_alloca, "de.arr.cursor.iter")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let src_at_cursor = unsafe {
                        self.builder
                            .build_gep(
                                i8_t,
                                src,
                                &[cursor_iv],
                                "de.arr.src.iter",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let remaining = self.emit_de_remaining(cursor_iv, wire_n, "de.arr.elem")?;
                    let consumed = self.emit_per_field_deserialize_size(
                        src_at_cursor,
                        nested_dst,
                        nested_info.struct_ty,
                        &nested_info.field_order,
                        &nested_info.fields,
                        remaining,
                        fail_block,
                    )?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            consumed,
                            "de.cursor.after.arr.elem",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(slot, nested_dst)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let _ = ptr_t;
                }
                Ok(())
            }
            other => Err(CodegenError::Unsupported(format!(
                "bus payload array `{}: [{}; {}]`: the record field walk takes \
                 scalars and records (`wire_array_fixed_walk`); any other element \
                 goes through the per-element loop",
                fname, other, n
            ))),        }
    }

    /// m70: emit IR for the body of `__serialize_T` for a struct
    /// payload, walking fields in declared order. Returns the
    /// total bytes-written value (i64) for the fn's return.
    fn emit_per_field_serialize(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let cursor_alloca = self.wire_alloca(i64_t.into(), "ser.cursor")?;
        self.builder
            .build_store(cursor_alloca, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        for fname in field_order {
            let (idx, field_ty) = fields
                .get(fname)
                .cloned()
                .expect("field declared in field_order also present in fields");
            let src_field_ptr = self
                .builder
                .build_struct_gep(struct_ty, src, idx, "ser.field.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.emit_wire_ser_value(src_field_ptr, &field_ty, true, dst, cursor_alloca, fname)?;
        }

        let total = self
            .builder
            .build_load(i64_t, cursor_alloca, "ser.cursor.final")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        Ok(total)
    }

    /// m70: emit IR for the body of `__deserialize_T` for a
    /// struct payload, walking fields in declared order.
    /// Returns the in-memory struct size (the dst contains a
    /// concrete struct, regardless of how much wire was
    /// consumed). The walk is `emit_per_field_deserialize_size`'s;
    /// only the return differs.
    fn emit_per_field_deserialize(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        self.emit_per_field_deserialize_size(
            src,
            dst,
            struct_ty,
            field_order,
            fields,
            wire_n,
            fail_block,
        )?;
        let struct_size = struct_ty
            .size_of()
            .expect("payload struct has known size");
        Ok(struct_size)
    }

    /// Variant of `emit_per_field_deserialize` that returns the
    /// number of *wire bytes* consumed rather than the in-memory
    /// struct size. Needed by the nested-struct recursion in
    /// `emit_per_field_deserialize` so the caller can advance its
    /// wire cursor by the consumed amount. Same body, different
    /// return.
    fn emit_per_field_deserialize_size(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let cursor_alloca = self.wire_alloca(i64_t.into(), "de.nested.cursor")?;
        self.builder
            .build_store(cursor_alloca, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        for fname in field_order {
            let (idx, field_ty) = fields
                .get(fname)
                .cloned()
                .expect("field declared in field_order also present in fields");
            let dst_field_ptr = self
                .builder
                .build_struct_gep(struct_ty, dst, idx, "de.nested.field.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.emit_wire_de_value(dst_field_ptr, &field_ty, true, src, cursor_alloca, wire_n, fail_block, fname)?;
        }

        let total = self
            .builder
            .build_load(i64_t, cursor_alloca, "de.nested.cursor.final")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        Ok(total)
    }

}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Flat-payload predicate for the same-process bus serialize-skip
    /// optimization (2026-06-28). Answers: would a *verbatim* copy of
    /// this payload's struct storage be a complete, self-contained
    /// value — i.e. contain NO pointer into the publisher's arena that
    /// the wire round-trip would otherwise rebind into the subscriber's
    /// arena? If so, codegen routes the publish through
    /// `lotus_bus_dispatch_flat` (one verbatim cell copy for local
    /// fanout) instead of `lotus_bus_dispatch` (serialize → per-sub
    /// deserialize-into-sub-arena).
    ///
    /// CORRECTNESS CONTRACT: a `true` here MUST mean "no heap pointer in
    /// the bytes". A false positive aliases the publisher's arena into
    /// every subscriber cell — exactly the unbounded leak Task-11 fixed.
    /// So we DEFAULT TO `false` for anything not provably pointer-free.
    /// A false negative merely forgoes the optimization (still correct,
    /// just slower). The non-flat set here is therefore a *superset* of
    /// what `emit_per_field_serialize` deep-copies / pointer-follows.
    ///
    /// What is flat: a struct (`TypeRef`) every one of whose fields is a
    /// by-value scalar. In this codegen, ONLY scalars are stored inline
    /// in a struct — `llvm_basic_type` lowers `String`, `Bytes`, `Time`,
    /// `TypeRef` (nested struct), `Array`, `Tuple`, views, `Interface`,
    /// `Cell`, `Drain`, and payload-carrying `Enum`s all to `ptr`. So a
    /// nested struct or a fixed-size array FIELD is a heap pointer into
    /// the publisher's arena (its storage is allocated separately), NOT
    /// inline bytes — which is why we do NOT recurse through such fields
    /// and treat their mere presence as non-flat. (This is intentionally
    /// stricter than a structural "recurse into Array elem / nested
    /// struct" reading: in Hale's pointer representation those fields are
    /// not self-contained, and the serializer pointer-follows them, so
    /// excluding them keeps the predicate superset-safe.)
    pub(crate) fn bus_payload_is_flat(&self, ty: &CodegenTy) -> bool {
        match ty {
            // The bus payload container is always a struct (TypeRef) or
            // a has-payload enum (handled below). A struct is flat iff
            // every field is an inline-by-value scalar.
            CodegenTy::TypeRef(name) => match self.user_types.get(name) {
                Some(info) => info
                    .fields
                    .values()
                    .all(|(_, fty)| self.bus_field_is_flat_scalar(fty)),
                None => false,
            },
            // Anything else as a top-level payload (notably a
            // payload-carrying Enum, which is a pointer to tagged-union
            // storage whose body may carry variant pointers) is
            // conservatively NOT flat.
            _ => false,
        }
    }

    /// Is `ty`, AS A STRUCT FIELD, stored inline with no heap pointer?
    /// Only by-value scalars qualify (see `llvm_basic_type`): the
    /// fixed-width primitives plus a no-payload enum (an i32 tag). Every
    /// other variant lowers to a `ptr` (or a pointer-bearing by-value
    /// view struct) and so makes the enclosing struct non-flat.
    fn bus_field_is_flat_scalar(&self, ty: &CodegenTy) -> bool {
        match ty {
            CodegenTy::Int
            | CodegenTy::U64
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal(_)
            | CodegenTy::U256
            | CodegenTy::I256
            | CodegenTy::Duration => true,
            // No-payload enum is a plain i32 tag (value semantics); a
            // payload-carrying enum is a pointer to its storage struct.
            CodegenTy::Enum(name) => self
                .user_enums
                .get(name)
                .map(|info| !info.has_payload)
                .unwrap_or(false),
            // Explicitly NOT flat (pointer-bearing or pointer-typed):
            // String, Bytes, Time, BytesView, StringView, BytesMut,
            // LocusRef, TypeRef, Array, Tuple, FnPtr, Interface, Cell,
            // Drain — plus any future variant (default-false).
            _ => false,
        }
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The byte count of a fixed wire value (`WireShape::Scalar`, or a
    /// payload-free enum's `u32` tag).
    fn wire_fixed_size(&self, t: &CodegenTy) -> u64 {
        match t {
            CodegenTy::Enum(_) => 4,
            _ => codegen_ty_size_bytes(self.context, t),
        }
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// One value onto the wire at the cursor: the carriage table's every
    /// shape (`hale_types::wire_carriage::WIRE_TABLE`, GH #1951 item 4),
    /// dispatched on the shape [`Self::wire_shape`] classifies `ty` as.
    /// `src_field_ptr` is where the value lies: a record field's slot when
    /// `field` (a `bounded` list and a scalar-element array lie inline
    /// there, and the fixed bytes are copied as they always were), else a
    /// slot holding the value's SSA form — an enum's body, a tuple's, an
    /// array's or a list's element, a pointer for every non-scalar, read
    /// at align 1 (an enum body sits at offset 4).
    fn emit_wire_ser_value(
        &mut self,
        src_field_ptr: PointerValue<'ctx>,
        field_ty: &CodegenTy,
        field: bool,
        dst: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        fname: &str,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let field_ty = field_ty.clone();
        let shape = self.wire_shape(&field_ty).map_err(|what| wire_refusal(fname, &field_ty, what))?;
        let cursor_iv = self
            .builder
            .build_load(i64_t, cursor_alloca, "ser.cursor.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let dst_at_cursor = unsafe {
            self.builder
                .build_gep(i8_t, dst, &[cursor_iv], "ser.dst.cursor")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        match (shape, &field_ty) {
            (WireShape::String, _) => {
                // Wire: i64 LE length + N bytes (no NUL).
                let str_ptr = self.wire_slot_load_ptr(src_field_ptr, field, "ser.str.ptr")?;
                let str_len = self.emit_str_len_call(str_ptr)?;
                // Write 8-byte length prefix.
                let len_alloca = self.wire_alloca(i64_t.into(), "ser.str.len")?;
                self.builder
                    .build_store(len_alloca, str_len)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.emit_ser_memcpy(
                    dst_at_cursor,
                    len_alloca,
                    i64_t.const_int(8, false),
                    "ser.str.memcpy.len",
                )?;
                let after_len = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        i64_t.const_int(8, false),
                        "ser.cursor.after.len",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let dst_after_len = unsafe {
                    self.builder
                        .build_gep(
                            i8_t,
                            dst,
                            &[after_len],
                            "ser.dst.after.len",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                self.emit_ser_memcpy(
                    dst_after_len,
                    str_ptr,
                    str_len,
                    "ser.str.memcpy.bytes",
                )?;
                let after_bytes = self
                    .builder
                    .build_int_add(
                        after_len,
                        str_len,
                        "ser.cursor.after.bytes",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after_bytes)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Scalar | WireShape::PayloadFreeEnum, _) if field => {
                let nbytes = self.wire_fixed_size(&field_ty);
                let nbytes_iv = i64_t.const_int(nbytes, false);
                self.emit_ser_memcpy(
                    dst_at_cursor,
                    src_field_ptr,
                    nbytes_iv,
                    "ser.fixed.memcpy",
                )?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        nbytes_iv,
                        "ser.cursor.after",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Scalar | WireShape::PayloadFreeEnum, _) => {
                // Inside an enum's body or a tuple the slot is the value's
                // own width (a `Bool` is one byte): read it typed, and put
                // its wire bytes (zero-padded) through a scratch slot.
                let nbytes = self.wire_fixed_size(&field_ty);
                let scratch = self.wire_scratch()?;
                let v = self.wire_slot_load(self.llvm_basic_type(&field_ty), src_field_ptr, field, "ser.elem.fixed")?;
                self.builder.build_store(scratch, v).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let nbytes_iv = i64_t.const_int(nbytes, false);
                self.emit_ser_memcpy(dst_at_cursor, scratch, nbytes_iv, "ser.elem.fixed.memcpy")?;
                let after = self
                    .builder
                    .build_int_add(cursor_iv, nbytes_iv, "ser.cursor.after.elem")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Bytes, _) => {
                // A8 (G15): Bytes payload field. The in-memory
                // blob is `[i64 len][u8 data[len]]` (per
                // memory.md § Bytes); the wire format matches.
                // Load the Bytes pointer, read its length
                // prefix, then memcpy `8 + len` bytes from
                // the blob to dst. The deserializer mirrors
                // this — allocate `8 + len`, memcpy, store
                // the pointer.
                let bytes_ptr = self.wire_slot_load_ptr(src_field_ptr, field, "ser.bytes.ptr")?;
                let bytes_len = self
                    .builder
                    .build_load(i64_t, bytes_ptr, "ser.bytes.len")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let total = self
                    .builder
                    .build_int_add(
                        bytes_len,
                        i64_t.const_int(8, false),
                        "ser.bytes.total",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.emit_ser_memcpy(
                    dst_at_cursor,
                    bytes_ptr,
                    total,
                    "ser.bytes.memcpy",
                )?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        total,
                        "ser.cursor.after.bytes",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Struct | WireShape::JsonValue, CodegenTy::TypeRef(nested_name)) => {
                // Nested user-struct: recurse on its field
                // layout. The slot at `src_field_ptr` holds a
                // pointer to the nested storage (TypeRef
                // values are heap-allocated structs); load
                // it, then walk the nested fields starting at
                // the current dst cursor.
                let nested_info = self
                    .user_types
                    .get(nested_name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload field `{}: {}` — nested \
                             type not declared",
                            fname, nested_name
                        ))
                    })?;
                let nested_src = self.wire_slot_load_ptr(src_field_ptr, field, "ser.nested.load")?;
                let nested_written = self.emit_per_field_serialize(
                    nested_src,
                    dst_at_cursor,
                    nested_info.struct_ty,
                    &nested_info.field_order,
                    &nested_info.fields,
                )?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        nested_written,
                        "ser.cursor.after.nested",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::BoundedInline, CodegenTy::Bounded(belem, bn)) => {
                let storage = if field { src_field_ptr } else { self.wire_slot_load_ptr(src_field_ptr, field, "ser.bounded.storage")? };
                self.emit_bounded_field_wire_memcpy(storage, dst, cursor_alloca, belem, *bn, true, "ser.bounded", None)?;
            }
            (WireShape::BoundedCounted, CodegenTy::Bounded(belem, bn)) => {
                let storage = if field { src_field_ptr } else { self.wire_slot_load_ptr(src_field_ptr, field, "ser.list.storage")? };
                self.emit_wire_ser_counted(storage, belem, *bn, dst, cursor_alloca, fname)?;
            }
            (WireShape::Array, CodegenTy::Array(elem_ty, n)) if field && Self::wire_array_fixed_walk(elem_ty) => {
                // Form H (2026-05-20): a record field's array of scalars
                // (one memcpy) or of records (unrolled).
                self.emit_array_field_serialize(src_field_ptr, dst, cursor_alloca, elem_ty, *n, fname)?;
            }
            (WireShape::Array, CodegenTy::Array(elem_ty, n)) => {
                let storage = if field && Self::array_inline_spec(&field_ty).is_some() {
                    src_field_ptr
                } else {
                    self.wire_slot_load_ptr(src_field_ptr, field, "ser.arr.storage")?
                };
                let arr_ty = self.llvm_array_storage_type(elem_ty, *n);
                let elem_ty = (**elem_ty).clone();
                let site = format!("{fname}[]");
                self.emit_wire_loop(i64_t.const_int(*n, false), "ser.arr", &mut |cx, i| {
                    let slot = unsafe {
                        cx.builder
                            .build_in_bounds_gep(arr_ty, storage, &[i64_t.const_zero(), i], "ser.arr.elem")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    cx.emit_wire_ser_value(slot, &elem_ty, false, dst, cursor_alloca, &site)
                })?;
            }
            (WireShape::PayloadEnum, CodegenTy::Enum(name)) => {
                let info = self.user_enums.get(name.as_str()).cloned().expect("a payload enum is declared");
                let cell = self.wire_slot_load_ptr(src_field_ptr, field, "ser.enum.cell")?;
                self.emit_wire_ser_enum_cell(cell, &info, dst, cursor_alloca, fname)?;
            }
            (WireShape::Tuple, CodegenTy::Tuple(elems)) => {
                let tup = self.wire_slot_load_ptr(src_field_ptr, field, "ser.tuple")?;
                let st = self.llvm_tuple_storage_type(elems);
                for (i, et) in elems.iter().enumerate() {
                    let slot = self
                        .builder
                        .build_struct_gep(st, tup, i as u32, "ser.tuple.elem")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.emit_wire_ser_value(slot, et, false, dst, cursor_alloca, &format!("{fname}.{i}"))?;
                }
            }
            (shape, other) => {
                return Err(CodegenError::Unsupported(format!(
                    "bus payload field `{fname}: {other}`: classified as {shape:?}, a shape this walk has no arm for"
                )));
            }
        }
        Ok(())
    }

    /// One value off the wire at the cursor into `dst_field_ptr`: the
    /// twin of [`Self::emit_wire_ser_value`], with every read held to the
    /// frame in the wire decoder (`emit_de_need`), a count past a list's
    /// capacity and a tag past an enum's variants refused, and every
    /// pointer-shaped value (a string, a record, an enum's cell, a tuple,
    /// an array's storage) allocated in the payload arena as a nested
    /// record always was.
    fn emit_wire_de_value(
        &mut self,
        dst_field_ptr: PointerValue<'ctx>,
        field_ty: &CodegenTy,
        field: bool,
        src: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        fname: &str,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let field_ty = field_ty.clone();
        let shape = self.wire_shape(&field_ty).map_err(|what| wire_refusal(fname, &field_ty, what))?;
        let cursor_iv = self
            .builder
            .build_load(i64_t, cursor_alloca, "de.nested.cursor.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let src_at_cursor = unsafe {
            self.builder
                .build_gep(i8_t, src, &[cursor_iv], "de.nested.src.cursor")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        match (shape, &field_ty) {
            (WireShape::String, _) => {
                let len_alloca = self.wire_alloca(i64_t.into(), "de.nested.str.len.alloca")?;
                self.emit_de_need(
                    cursor_iv,
                    i64_t.const_int(8, false),
                    wire_n,
                    fail_block,
                    "de.str.len",
                )?;
                self.emit_memcpy_call(
                    len_alloca,
                    src_at_cursor,
                    i64_t.const_int(8, false),
                    "de.nested.str.memcpy.len",
                )?;
                let str_len = self
                    .builder
                    .build_load(i64_t, len_alloca, "de.nested.str.len")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                self.emit_de_legacy_len_check(str_len, wire_n, fail_block, "de.str")?;
                let after_len = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        i64_t.const_int(8, false),
                        "de.nested.cursor.after.len",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                // The length read off the wire, against the bytes
                // after it (not the whole frame: a nested or later
                // String used to pass on a length the frame held
                // only from its start), before anything is added to
                // it.
                self.emit_de_len_fits(str_len, after_len, wire_n, fail_block, "de.str")?;
                self.emit_de_need(after_len, str_len, wire_n, fail_block, "de.str.bytes")?;
                let src_after_len = unsafe {
                    self.builder
                        .build_gep(
                            i8_t,
                            src,
                            &[after_len],
                            "de.nested.src.after.len",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                let alloc_size = self
                    .builder
                    .build_int_add(
                        str_len,
                        i64_t.const_int(1, false),
                        "de.nested.str.alloc.size",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let alloc_fn = self
                    .module
                    .get_function("lotus_bus_payload_arena_alloc")
                    .expect("lotus_bus_payload_arena_alloc declared");
                let buf = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            alloc_size.into(),
                            i64_t.const_int(1, false).into(),
                        ],
                        "de.nested.str.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("payload arena alloc returns ptr")
                    .into_pointer_value();
                self.emit_memcpy_call(
                    buf,
                    src_after_len,
                    str_len,
                    "de.nested.str.memcpy.bytes",
                )?;
                let nul_slot = unsafe {
                    self.builder
                        .build_gep(
                            i8_t,
                            buf,
                            &[str_len],
                            "de.nested.str.nul.ptr",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                self.builder
                    .build_store(nul_slot, i8_t.const_int(0, false))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.wire_slot_store(dst_field_ptr, buf.into(), field)?;
                let after_bytes = self
                    .builder
                    .build_int_add(
                        after_len,
                        str_len,
                        "de.nested.cursor.after.bytes",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after_bytes)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Scalar | WireShape::PayloadFreeEnum, _) if field => {
                let nbytes = self.wire_fixed_size(&field_ty);
                let nbytes_iv = i64_t.const_int(nbytes, false);
                self.emit_de_need(cursor_iv, nbytes_iv, wire_n, fail_block, "de.fixed")?;
                self.emit_memcpy_call(
                    dst_field_ptr,
                    src_at_cursor,
                    nbytes_iv,
                    "de.nested.fixed.memcpy",
                )?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        nbytes_iv,
                        "de.nested.cursor.after",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Scalar | WireShape::PayloadFreeEnum, _) => {
                let nbytes = self.wire_fixed_size(&field_ty);
                let nbytes_iv = i64_t.const_int(nbytes, false);
                self.emit_de_need(cursor_iv, nbytes_iv, wire_n, fail_block, "de.elem.fixed")?;
                let scratch = self.wire_scratch()?;
                self.emit_memcpy_call(scratch, src_at_cursor, nbytes_iv, "de.elem.fixed.memcpy")?;
                let v = self
                    .builder
                    .build_load(self.llvm_basic_type(&field_ty), scratch, "de.elem.fixed")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.wire_slot_store(dst_field_ptr, v, field)?;
                let after = self
                    .builder
                    .build_int_add(cursor_iv, nbytes_iv, "de.cursor.after.elem")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Bytes, _) => {
                // A8 (G15) — nested-struct variant. Same shape
                // as the top-level Bytes deserializer above:
                // read 8-byte length, alloc `8 + len`, memcpy
                // the prefixed blob, store ptr into dst.
                let len_alloca = self.wire_alloca(i64_t.into(), "de.nested.bytes.len.alloca")?;
                self.emit_de_need(
                    cursor_iv,
                    i64_t.const_int(8, false),
                    wire_n,
                    fail_block,
                    "de.bytes.len",
                )?;
                self.emit_memcpy_call(
                    len_alloca,
                    src_at_cursor,
                    i64_t.const_int(8, false),
                    "de.nested.bytes.memcpy.len",
                )?;
                let bytes_len = self
                    .builder
                    .build_load(i64_t, len_alloca, "de.nested.bytes.len")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                self.emit_de_legacy_len_check(bytes_len, wire_n, fail_block, "de.bytes")?;
                // The raw length against what the frame holds after
                // its prefix, BEFORE the `+ 8`: a length of -8 made
                // the sum zero and passed the sum's check (reviewer,
                // 2026-11-07).
                let after_prefix = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        i64_t.const_int(8, false),
                        "de.nested.bytes.after.prefix",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.emit_de_len_fits(bytes_len, after_prefix, wire_n, fail_block, "de.bytes")?;
                let total = self
                    .builder
                    .build_int_add(
                        bytes_len,
                        i64_t.const_int(8, false),
                        "de.nested.bytes.total",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                // The prefix and the blob it announces, against
                // what the frame holds after the cursor.
                self.emit_de_need(cursor_iv, total, wire_n, fail_block, "de.bytes.blob")?;
                let alloc_fn = self
                    .module
                    .get_function("lotus_bus_payload_arena_alloc")
                    .expect("lotus_bus_payload_arena_alloc declared");
                let buf = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            total.into(),
                            i64_t.const_int(8, false).into(),
                        ],
                        "de.nested.bytes.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("payload arena alloc returns ptr")
                    .into_pointer_value();
                self.emit_memcpy_call(
                    buf,
                    src_at_cursor,
                    total,
                    "de.nested.bytes.memcpy.blob",
                )?;
                self.wire_slot_store(dst_field_ptr, buf.into(), field)?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        total,
                        "de.nested.cursor.after.bytes",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::Struct | WireShape::JsonValue, CodegenTy::TypeRef(nested_name)) => {
                let nested_info = self
                    .user_types
                    .get(nested_name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload field `{}: {}` — nested \
                             type not declared",
                            fname, nested_name
                        ))
                    })?;
                let nested_size = nested_info
                    .struct_ty
                    .size_of()
                    .expect("nested struct ty has known size");
                let alloc_fn = self
                    .module
                    .get_function("lotus_bus_payload_arena_alloc")
                    .expect("lotus_bus_payload_arena_alloc declared");
                let nested_dst = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            nested_size.into(),
                            // 16-align: see `de.nested.alloc` — a
                            // nested payload struct may carry an
                            // i128 / Decimal (align-16) field.
                            i64_t.const_int(16, false).into(),
                        ],
                        "de.nested.deep.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("payload arena alloc returns ptr")
                    .into_pointer_value();
                let remaining = self.emit_de_remaining(cursor_iv, wire_n, "de.nested")?;
                let nested_consumed = self.emit_per_field_deserialize_size(
                    src_at_cursor,
                    nested_dst,
                    nested_info.struct_ty,
                    &nested_info.field_order,
                    &nested_info.fields,
                    remaining,
                    fail_block,
                )?;
                self.wire_slot_store(dst_field_ptr, nested_dst.into(), field)?;
                let after = self
                    .builder
                    .build_int_add(
                        cursor_iv,
                        nested_consumed,
                        "de.nested.cursor.after.nested",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(cursor_alloca, after)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (WireShape::BoundedInline | WireShape::BoundedCounted, CodegenTy::Bounded(belem, bn)) => {
                let storage = if field {
                    dst_field_ptr
                } else {
                    let st = self.llvm_bounded_storage_type(belem, *bn);
                    let p = self.wire_payload_alloc(st.size_of().expect("bounded storage sized"), 16, "de.list.alloc")?;
                    self.wire_slot_store(dst_field_ptr, p.into(), field)?;
                    p
                };
                if shape == WireShape::BoundedInline {
                    self.emit_bounded_field_wire_memcpy(storage, src, cursor_alloca, belem, *bn, false, "de.bounded", Some((wire_n, fail_block)))?;
                } else {
                    self.emit_wire_de_counted(storage, belem, *bn, src, cursor_alloca, wire_n, fail_block, fname)?;
                }
            }
            (WireShape::Array, CodegenTy::Array(elem_ty, n)) if field && Self::wire_array_fixed_walk(elem_ty) => {
                self.emit_array_field_deserialize(dst_field_ptr, src, cursor_alloca, elem_ty, *n, fname, wire_n, fail_block)?;
            }
            (WireShape::Array, CodegenTy::Array(elem_ty, n)) => {
                let arr_ty = self.llvm_array_storage_type(elem_ty, *n);
                let storage = if field && Self::array_inline_spec(&field_ty).is_some() {
                    dst_field_ptr
                } else {
                    let p = self.wire_payload_alloc(arr_ty.size_of().expect("array storage sized"), 16, "de.arr.alloc")?;
                    self.wire_slot_store(dst_field_ptr, p.into(), field)?;
                    p
                };
                let elem_ty = (**elem_ty).clone();
                let site = format!("{fname}[]");
                self.emit_wire_loop(i64_t.const_int(*n, false), "de.arr", &mut |cx, i| {
                    let slot = unsafe {
                        cx.builder
                            .build_in_bounds_gep(arr_ty, storage, &[i64_t.const_zero(), i], "de.arr.elem")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    cx.emit_wire_de_value(slot, &elem_ty, false, src, cursor_alloca, wire_n, fail_block, &site)
                })?;
            }
            (WireShape::PayloadEnum, CodegenTy::Enum(name)) => {
                let info = self.user_enums.get(name.as_str()).cloned().expect("a payload enum is declared");
                let size = self.enum_storage_struct(&info).size_of().expect("enum storage sized");
                let cell = self.wire_payload_alloc(size, 16, "de.enum.alloc")?;
                self.wire_slot_store(dst_field_ptr, cell.into(), field)?;
                self.emit_wire_de_enum_cell(cell, &info, src, cursor_alloca, wire_n, fail_block, fname)?;
            }
            (WireShape::Tuple, CodegenTy::Tuple(elems)) => {
                let st = self.llvm_tuple_storage_type(elems);
                let tup = self.wire_payload_alloc(st.size_of().expect("tuple storage sized"), 16, "de.tuple.alloc")?;
                self.wire_slot_store(dst_field_ptr, tup.into(), field)?;
                for (i, et) in elems.iter().enumerate() {
                    let slot = self
                        .builder
                        .build_struct_gep(st, tup, i as u32, "de.tuple.elem")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.emit_wire_de_value(slot, et, false, src, cursor_alloca, wire_n, fail_block, &format!("{fname}.{i}"))?;
                }
            }
            (shape, other) => {
                return Err(CodegenError::Unsupported(format!(
                    "bus payload field `{fname}: {other}`: classified as {shape:?}, a shape this walk has no arm for"
                )));
            }
        }
        Ok(())
    }
}

/// The refusal of a value the wire format has no form for, naming where it
/// is and what the wire carries (`hale_types::wire_carriage::carried_list`,
/// the table the check's refusal prints).
fn wire_refusal(fname: &str, ty: &CodegenTy, what: &str) -> CodegenError {
    CodegenError::Unsupported(format!(
        "bus payload field `{fname}: {ty}` is {what}, which the wire format does not carry: a delivery serializes \
         each value, and carries {}",
        wire_carriage::carried_list()
    ))
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The carriage table's shape of a lowered type
    /// (`hale_types::wire_carriage::WireShape`), or what it is when the
    /// wire has no form for it. The checker classifies a `Ty` into the
    /// same table (`wire_carriage::ty_uncarried`).
    pub(crate) fn wire_shape(&self, ty: &CodegenTy) -> Result<WireShape, &'static str> {
        Ok(match ty {
            CodegenTy::Int
            | CodegenTy::U64
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::Decimal(_)
            | CodegenTy::U256
            | CodegenTy::I256 => WireShape::Scalar,
            CodegenTy::String => WireShape::String,
            CodegenTy::Bytes => WireShape::Bytes,
            CodegenTy::TypeRef(n) if n == hale_syntax::json_gen::JSON_VALUE => WireShape::JsonValue,
            CodegenTy::TypeRef(_) => WireShape::Struct,
            CodegenTy::Enum(n) => match self.user_enums.get(n.as_str()) {
                Some(i) if i.has_payload => WireShape::PayloadEnum,
                _ => WireShape::PayloadFreeEnum,
            },
            CodegenTy::Bounded(e, _) if self.bounded_elem_is_ptr(e) => WireShape::BoundedCounted,
            CodegenTy::Bounded(_, _) => WireShape::BoundedInline,
            CodegenTy::Array(_, _) => WireShape::Array,
            CodegenTy::Tuple(_) => WireShape::Tuple,
            CodegenTy::BytesView | CodegenTy::StringView | CodegenTy::BytesMut => return Err(wire_carriage::refusal::VIEW),
            CodegenTy::LocusRef(_) | CodegenTy::Interface(_) => {
                return Err(wire_carriage::refusal::HANDLE)
            }
            CodegenTy::FnPtr { .. } => return Err(wire_carriage::refusal::FN_VALUE),
            _ => return Err(wire_carriage::refusal::NO_FORM),
        })
    }

    /// A payload type's every value is one the wire carries and no type
    /// reaches itself (the walk is emitted inline, so a recursive type
    /// would never end; the check refuses one at the topic). Checked once
    /// per payload type, before its serializer is emitted.
    fn wire_check(&self, ty: &CodegenTy, path: &str, open: &mut Vec<String>) -> Result<(), CodegenError> {
        let shape = self.wire_shape(ty).map_err(|what| wire_refusal(path, ty, what))?;
        let named = match ty {
            CodegenTy::TypeRef(n) | CodegenTy::Enum(n) if matches!(shape, WireShape::Struct | WireShape::JsonValue | WireShape::PayloadEnum) => {
                Some(n.clone())
            }
            _ => None,
        };
        if let Some(n) = &named {
            if open.contains(n) {
                return Err(wire_refusal(path, ty, wire_carriage::refusal::RECURSIVE));
            }
            open.push(n.clone());
        }
        let sep = |p: &str, seg: &str| if p.is_empty() { seg.to_string() } else { format!("{p}.{seg}") };
        match ty {
            CodegenTy::TypeRef(n) => {
                let info = self.user_types.get(n.as_str()).cloned().ok_or_else(|| {
                    CodegenError::Unsupported(format!("bus payload field `{path}: {n}` — nested type not declared"))
                })?;
                for f in &info.field_order {
                    if let Some((_, t)) = info.fields.get(f) {
                        self.wire_check(t, &sep(path, f), open)?;
                    }
                }
            }
            CodegenTy::Enum(n) if shape == WireShape::PayloadEnum => {
                let info = self.user_enums.get(n.as_str()).cloned().expect("a payload enum is declared");
                for v in &info.variants {
                    for (i, t) in v.field_tys.iter().enumerate() {
                        self.wire_check(t, &sep(path, &format!("{}.{i}", v.name)), open)?;
                    }
                }
            }
            CodegenTy::Bounded(e, _) | CodegenTy::Array(e, _) => self.wire_check(e, &format!("{path}[]"), open)?,
            CodegenTy::Tuple(es) => {
                for (i, t) in es.iter().enumerate() {
                    self.wire_check(t, &sep(path, &i.to_string()), open)?;
                }
            }
            _ => {}
        }
        if named.is_some() {
            open.pop();
        }
        Ok(())
    }

    /// A value the walk carries without a loop or a switch: a scalar, a
    /// payload-free enum, a `String`, `Bytes`, a `bounded` list of inline
    /// elements, a record of those, a record field's array of scalars or
    /// records. A cross-pool spawn marshals only these: its create cell's
    /// dispatch is one straight-line block (the check refuses the rest,
    /// `wire_carriage::crosspool_uncarried`; this is lowering's backstop).
    pub(crate) fn wire_fixed_walk(&self, ty: &CodegenTy, field: bool, depth: u32) -> bool {
        if depth > 32 {
            return false;
        }
        match self.wire_shape(ty) {
            Ok(WireShape::Scalar | WireShape::PayloadFreeEnum | WireShape::String | WireShape::Bytes | WireShape::BoundedInline) => true,
            Ok(WireShape::Struct | WireShape::JsonValue) => match ty {
                CodegenTy::TypeRef(n) => self
                    .user_types
                    .get(n.as_str())
                    .is_some_and(|i| i.fields.values().all(|(_, t)| self.wire_fixed_walk(t, true, depth + 1))),
                _ => false,
            },
            Ok(WireShape::Array) => match ty {
                CodegenTy::Array(e, _) => field && Self::wire_array_fixed_walk(e) && self.wire_fixed_walk(e, false, depth + 1),
                _ => false,
            },
            _ => false,
        }
    }

    /// A record field's array the field walk has always carried: scalars
    /// (one memcpy) or records (unrolled, `emit_array_field_serialize`).
    /// Any other element goes through the per-element loop.
    fn wire_array_fixed_walk(elem: &CodegenTy) -> bool {
        matches!(
            elem,
            CodegenTy::Int
                | CodegenTy::U64
                | CodegenTy::Float
                | CodegenTy::Bool
                | CodegenTy::Time
                | CodegenTy::Duration
                | CodegenTy::Decimal(_)
                | CodegenTy::U256
                | CodegenTy::I256
                | CodegenTy::TypeRef(_)
        )
    }

    /// An `alloca` at the top of the entry block of the fn being emitted,
    /// so a walk emitted inside a loop (a list's elements) does not grow
    /// the stack each iteration. The builder returns to the end of the
    /// block it stood in (`alloca_in_entry`, for the fn the builder is in
    /// rather than `current_fn`, which a synthesized serializer does not
    /// set).
    pub(crate) fn wire_alloca(
        &mut self,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let here = self.builder.get_insert_block().expect("builder positioned in a fn");
        let entry = here.get_parent().and_then(|f| f.get_first_basic_block()).expect("fn has an entry block");
        match entry.get_first_instruction() {
            Some(first) => self.builder.position_before(&first),
            None => self.builder.position_at_end(entry),
        }
        let slot = self.builder.build_alloca(ty, name).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(here);
        // `position_before` adopted the alloca's empty location; the
        // statement's comes back, as `alloca_in_entry` restores it
        if let Some(loc) = self.di_current_loc {
            self.builder.set_current_debug_location(loc);
        }
        Ok(slot)
    }

    /// A zeroed 32-byte, 16-aligned scratch slot: a fixed value's wire
    /// bytes on their way between the frame and a slot of its own width.
    fn wire_scratch(&mut self) -> Result<PointerValue<'ctx>, CodegenError> {
        let t = self.context.i8_type().array_type(32);
        let slot = self.wire_alloca(t.into(), "wire.scratch")?;
        if let Some(i) = slot.as_instruction() {
            i.set_alignment(16).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder.build_store(slot, t.const_zero()).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(slot)
    }

    /// A value read from a slot: at align 1 outside a record field (an
    /// enum's body sits at offset 4, `lower_enum_variant_alloc_in`).
    fn wire_slot_load(
        &mut self,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
        slot: PointerValue<'ctx>,
        field: bool,
        name: &str,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, CodegenError> {
        let v = self.builder.build_load(ty, slot, name).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        if !field {
            if let Some(i) = inkwell::values::BasicValue::as_instruction_value(&v) {
                i.set_alignment(1).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }
        Ok(v)
    }

    fn wire_slot_load_ptr(
        &mut self,
        slot: PointerValue<'ctx>,
        field: bool,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        Ok(self.wire_slot_load(ptr_t.into(), slot, field, name)?.into_pointer_value())
    }

    /// A value stored into a slot, at align 1 outside a record field.
    fn wire_slot_store(
        &mut self,
        slot: PointerValue<'ctx>,
        v: inkwell::values::BasicValueEnum<'ctx>,
        field: bool,
    ) -> Result<(), CodegenError> {
        let st = self.builder.build_store(slot, v).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        if !field {
            st.set_alignment(1).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        Ok(())
    }

    /// `size` bytes from the payload arena, where a decoded record, string
    /// and enum cell live until the delivery is done.
    fn wire_payload_alloc(
        &mut self,
        size: inkwell::values::IntValue<'ctx>,
        align: u64,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let alloc_fn = self
            .module
            .get_function("lotus_bus_payload_arena_alloc")
            .expect("lotus_bus_payload_arena_alloc declared");
        Ok(self
            .builder
            .build_call(alloc_fn, &[size.into(), self.context.i64_type().const_int(align, false).into()], name)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("payload arena alloc returns ptr")
            .into_pointer_value())
    }

    /// `for i in 0..count { body(i) }`, emitted as a loop: a list's live
    /// elements, an array's `N`. The counter lives in the entry block.
    fn emit_wire_loop(
        &mut self,
        count: inkwell::values::IntValue<'ctx>,
        site: &str,
        body: &mut dyn FnMut(&mut Self, inkwell::values::IntValue<'ctx>) -> Result<(), CodegenError>,
    ) -> Result<(), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let i64_t = self.context.i64_type();
        let func = self.builder.get_insert_block().and_then(|b| b.get_parent()).expect("builder positioned in a fn");
        let counter = self.wire_alloca(i64_t.into(), &format!("{site}.i"))?;
        self.builder.build_store(counter, i64_t.const_zero()).map_err(e)?;
        let head = self.context.append_basic_block(func, &format!("{site}.head"));
        let each = self.context.append_basic_block(func, &format!("{site}.each"));
        let done = self.context.append_basic_block(func, &format!("{site}.done"));
        self.builder.build_unconditional_branch(head).map_err(e)?;
        self.builder.position_at_end(head);
        let i = self.builder.build_load(i64_t, counter, &format!("{site}.i.load")).map_err(e)?.into_int_value();
        let more = self.builder.build_int_compare(inkwell::IntPredicate::ULT, i, count, &format!("{site}.more")).map_err(e)?;
        self.builder.build_conditional_branch(more, each, done).map_err(e)?;
        self.builder.position_at_end(each);
        body(self, i)?;
        let next = self.builder.build_int_add(i, i64_t.const_int(1, false), &format!("{site}.next")).map_err(e)?;
        self.builder.build_store(counter, next).map_err(e)?;
        self.builder.build_unconditional_branch(head).map_err(e)?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// A `bounded[T; N]` of pointers (`WireShape::BoundedCounted`) onto the
    /// wire: its `i64` count, then that many elements, each as a value of
    /// `T`. `storage` is the list's inline `{ i64 len, [N x T] }`.
    fn emit_wire_ser_counted(
        &mut self,
        storage: PointerValue<'ctx>,
        elem: &CodegenTy,
        n: u64,
        dst: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        fname: &str,
    ) -> Result<(), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let st = self.llvm_bounded_storage_type(elem, n);
        let count = self.builder.build_load(i64_t, storage, "ser.list.count").map_err(e)?.into_int_value();
        let tmp = self.wire_alloca(i64_t.into(), "ser.list.count.tmp")?;
        self.builder.build_store(tmp, count).map_err(e)?;
        let cursor_iv = self.builder.build_load(i64_t, cursor_alloca, "ser.list.cursor").map_err(e)?.into_int_value();
        let at = unsafe { self.builder.build_gep(i8_t, dst, &[cursor_iv], "ser.list.dst").map_err(e)? };
        self.emit_ser_memcpy(at, tmp, i64_t.const_int(8, false), "ser.list.count.memcpy")?;
        let after = self.builder.build_int_add(cursor_iv, i64_t.const_int(8, false), "ser.list.after.count").map_err(e)?;
        self.builder.build_store(cursor_alloca, after).map_err(e)?;
        let data = self.builder.build_struct_gep(st, storage, 1, "ser.list.data").map_err(e)?;
        let arr_ty = self.llvm_array_storage_type(elem, n);
        let elem = elem.clone();
        let site = format!("{fname}[]");
        self.emit_wire_loop(count, "ser.list", &mut |cx, i| {
            let slot = unsafe {
                cx.builder
                    .build_in_bounds_gep(arr_ty, data, &[i64_t.const_zero(), i], "ser.list.elem")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };
            cx.emit_wire_ser_value(slot, &elem, false, dst, cursor_alloca, &site)
        })
    }

    /// The twin of [`Self::emit_wire_ser_counted`]: the count, refused by
    /// the wire decoder when it is negative or past `N` (an unsigned
    /// compare holds both), stored, then that many elements decoded into
    /// the slots.
    fn emit_wire_de_counted(
        &mut self,
        storage: PointerValue<'ctx>,
        elem: &CodegenTy,
        n: u64,
        src: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        fname: &str,
    ) -> Result<(), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let st = self.llvm_bounded_storage_type(elem, n);
        let cursor_iv = self.builder.build_load(i64_t, cursor_alloca, "de.list.cursor").map_err(e)?.into_int_value();
        self.emit_de_need(cursor_iv, i64_t.const_int(8, false), wire_n, fail_block, "de.list.count")?;
        let at = unsafe { self.builder.build_gep(i8_t, src, &[cursor_iv], "de.list.src").map_err(e)? };
        let tmp = self.wire_alloca(i64_t.into(), "de.list.count.tmp")?;
        self.emit_memcpy_call(tmp, at, i64_t.const_int(8, false), "de.list.count.memcpy")?;
        let count = self.builder.build_load(i64_t, tmp, "de.list.count").map_err(e)?.into_int_value();
        if self.de_checked {
            let bad = self
                .builder
                .build_int_compare(inkwell::IntPredicate::UGT, count, i64_t.const_int(n, false), "de.list.count.bad")
                .map_err(e)?;
            let func = self.builder.get_insert_block().and_then(|b| b.get_parent()).expect("builder positioned in a fn");
            let ok = self.context.append_basic_block(func, "de.list.count.ok");
            self.builder.build_conditional_branch(bad, fail_block, ok).map_err(e)?;
            self.builder.position_at_end(ok);
        }
        let after = self.builder.build_int_add(cursor_iv, i64_t.const_int(8, false), "de.list.after.count").map_err(e)?;
        self.builder.build_store(cursor_alloca, after).map_err(e)?;
        self.builder.build_store(storage, count).map_err(e)?;
        let data = self.builder.build_struct_gep(st, storage, 1, "de.list.data").map_err(e)?;
        let arr_ty = self.llvm_array_storage_type(elem, n);
        let elem = elem.clone();
        let site = format!("{fname}[]");
        self.emit_wire_loop(count, "de.list", &mut |cx, i| {
            let slot = unsafe {
                cx.builder
                    .build_in_bounds_gep(arr_ty, data, &[i64_t.const_zero(), i], "de.list.elem")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };
            cx.emit_wire_de_value(slot, &elem, false, src, cursor_alloca, wire_n, fail_block, &site)
        })
    }

    /// An enum cell (`{ i32 tag, body }`) onto the wire
    /// (`WireShape::PayloadEnum`): its tag as a `u32`, then the variant's
    /// values in order, each as a value of its type read from the body.
    fn emit_wire_ser_enum_cell(
        &mut self,
        cell: PointerValue<'ctx>,
        info: &EnumInfo,
        dst: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        fname: &str,
    ) -> Result<(), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let tag = self.load_enum_tag(info, cell)?;
        let tmp = self.wire_alloca(i32_t.into(), "ser.enum.tag.tmp")?;
        self.builder.build_store(tmp, tag).map_err(e)?;
        let cursor_iv = self.builder.build_load(i64_t, cursor_alloca, "ser.enum.cursor").map_err(e)?.into_int_value();
        let at = unsafe { self.builder.build_gep(i8_t, dst, &[cursor_iv], "ser.enum.dst").map_err(e)? };
        self.emit_ser_memcpy(at, tmp, i64_t.const_int(4, false), "ser.enum.tag.memcpy")?;
        let after = self.builder.build_int_add(cursor_iv, i64_t.const_int(4, false), "ser.enum.after.tag").map_err(e)?;
        self.builder.build_store(cursor_alloca, after).map_err(e)?;
        self.emit_wire_enum_switch(cell, info, tag, None, &mut |cx, slot, ty, site| {
            cx.emit_wire_ser_value(slot, ty, false, dst, cursor_alloca, &format!("{fname}.{site}"))
        })
    }

    /// The twin of [`Self::emit_wire_ser_enum_cell`], decoding into the
    /// cell at `cell`: the tag, refused by the wire decoder past the
    /// enum's last variant, then the variant's values into its body.
    fn emit_wire_de_enum_cell(
        &mut self,
        cell: PointerValue<'ctx>,
        info: &EnumInfo,
        src: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        wire_n: inkwell::values::IntValue<'ctx>,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        fname: &str,
    ) -> Result<(), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let cursor_iv = self.builder.build_load(i64_t, cursor_alloca, "de.enum.cursor").map_err(e)?.into_int_value();
        self.emit_de_need(cursor_iv, i64_t.const_int(4, false), wire_n, fail_block, "de.enum.tag")?;
        let at = unsafe { self.builder.build_gep(i8_t, src, &[cursor_iv], "de.enum.src").map_err(e)? };
        let tmp = self.wire_alloca(i32_t.into(), "de.enum.tag.tmp")?;
        self.emit_memcpy_call(tmp, at, i64_t.const_int(4, false), "de.enum.tag.memcpy")?;
        let tag = self.builder.build_load(i32_t, tmp, "de.enum.tag").map_err(e)?.into_int_value();
        if self.de_checked {
            let bad = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::UGE,
                    tag,
                    i32_t.const_int(info.variants.len() as u64, false),
                    "de.enum.tag.bad",
                )
                .map_err(e)?;
            let func = self.builder.get_insert_block().and_then(|b| b.get_parent()).expect("builder positioned in a fn");
            let ok = self.context.append_basic_block(func, "de.enum.tag.ok");
            self.builder.build_conditional_branch(bad, fail_block, ok).map_err(e)?;
            self.builder.position_at_end(ok);
        }
        let after = self.builder.build_int_add(cursor_iv, i64_t.const_int(4, false), "de.enum.after.tag").map_err(e)?;
        self.builder.build_store(cursor_alloca, after).map_err(e)?;
        let storage_ty = self.enum_storage_struct(info);
        let tag_ptr = self.builder.build_struct_gep(storage_ty, cell, 0, "de.enum.tag.ptr").map_err(e)?;
        self.builder.build_store(tag_ptr, tag).map_err(e)?;
        self.emit_wire_enum_switch(cell, info, tag, None, &mut |cx, slot, ty, site| {
            cx.emit_wire_de_value(slot, ty, false, src, cursor_alloca, wire_n, fail_block, &format!("{fname}.{site}"))
        })
    }

    /// A switch on an enum cell's `tag`, one case per variant with values,
    /// calling `each` with each value's slot in the body (8-byte strides,
    /// 16 for a `Decimal`, 32 for the wide integers, as
    /// `lower_enum_variant_alloc_in` lays them) in order. A variant with no
    /// values, and a tag past the last (which the wire decoder refused
    /// before: `default`), go on.
    fn emit_wire_enum_switch(
        &mut self,
        cell: PointerValue<'ctx>,
        info: &EnumInfo,
        tag: inkwell::values::IntValue<'ctx>,
        default: Option<inkwell::basic_block::BasicBlock<'ctx>>,
        each: &mut dyn FnMut(&mut Self, PointerValue<'ctx>, &CodegenTy, &str) -> Result<(), CodegenError>,
    ) -> Result<(), CodegenError> {
        if info.variants.iter().all(|v| v.field_tys.is_empty()) {
            return Ok(());
        }
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let at = self.builder.get_insert_block().expect("builder positioned in a fn");
        let func = at.get_parent().expect("block has a fn");
        let join = self.context.append_basic_block(func, "wire.enum.join");
        let storage_ty = self.enum_storage_struct(info);
        let mut cases = Vec::new();
        for (vi, v) in info.variants.iter().enumerate() {
            if v.field_tys.is_empty() {
                continue;
            }
            let bb = self.context.append_basic_block(func, &format!("wire.enum.v{vi}"));
            self.builder.position_at_end(bb);
            let body = self.builder.build_struct_gep(storage_ty, cell, 1, "wire.enum.body").map_err(e)?;
            let mut offset = 0u64;
            for (fi, ty) in v.field_tys.iter().enumerate() {
                let slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(i8_t, body, &[self.context.i64_type().const_int(offset, false)], "wire.enum.slot")
                        .map_err(e)?
                };
                each(self, slot, ty, &format!("{}.{fi}", v.name))?;
                offset += codegen_ty_size_bytes(self.context, ty);
            }
            self.builder.build_unconditional_branch(join).map_err(e)?;
            cases.push((i32_t.const_int(vi as u64, false), bb));
        }
        self.builder.position_at_end(at);
        self.builder.build_switch(tag, default.unwrap_or(join), &cases).map_err(e)?;
        self.builder.position_at_end(join);
        Ok(())
    }
}
