//! Bus wire-format codegen: per-payload-type serialize / deserialize
//! function synthesis. Emits LLVM functions that walk struct payloads
//! field-by-field (m70 wire format) into / out of caller buffers.
//! Round 3a of the codegen model-org refactor.

use std::collections::BTreeMap;

use inkwell::types::StructType;
use inkwell::values::PointerValue;
use inkwell::AddressSpace;

use crate::codegen::{codegen_ty_size_bytes, CodegenError, CodegenTy, Cx, SerializerPair};

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
        _parent_struct_ty: StructType<'ctx>,
        _field_idx: u32,
        elem_ty: &CodegenTy,
        n: u64,
        fname: &str,
    ) -> Result<(), CodegenError>;
    fn emit_array_field_deserialize(
        &mut self,
        dst_field_ptr: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        cursor_alloca: PointerValue<'ctx>,
        _parent_struct_ty: StructType<'ctx>,
        _field_idx: u32,
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

        // Decide the synthesis strategy. Two shapes:
        //   - Struct payload (struct_layout = Some): per-field
        //     walk (m70 wire format).
        //   - Enum payload (struct_layout = None, enum_size =
        //     Some): memcpy of the enum storage struct, after
        //     verifying no variant carries a String (would
        //     corrupt cross-process).
        let struct_layout: Option<(
            StructType<'ctx>,
            Vec<String>,
            BTreeMap<String, (u32, CodegenTy)>,
        )>;
        let enum_size: Option<inkwell::values::IntValue<'ctx>>;
        if let Some(info) = self.user_types.get(type_name).cloned() {
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
            // m70: refuse enum-with-String for cross-process —
            // per-variant per-field serialization is post-v1.
            for v in &info.variants {
                for ft in &v.field_tys {
                    if matches!(ft, CodegenTy::String) {
                        return Err(CodegenError::Unsupported(format!(
                            "bus payload `{}` variant `{}` has a String \
                             field; cross-process String inside an enum \
                             variant is post-v1 (m70 supports String only \
                             at the top-level struct)",
                            type_name, v.name
                        )));
                    }
                }
            }
            let size = self
                .enum_storage_struct(&info)
                .size_of()
                .expect("enum storage struct has known size");
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
            let wire_min = match &struct_layout {
                Some((_, field_order, fields)) => {
                    i64_t.const_int(self.wire_min_size_of_fields(field_order, fields), false)
                }
                None => enum_size.expect("enum size present"),
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
    /// elements (String/Bytes/TypeRef) cross-process are post-v1
    /// polish (callers get a focused reject at the dispatch arm).
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
            CodegenTy::Bounded(elem, n) => 8 + n * self.wire_fixed_size(elem),
            CodegenTy::Array(elem, n) => n * self.wire_min_size(elem),
            other if self.wire_fixed_field(other) => self.wire_fixed_size(other),
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
        _parent_struct_ty: StructType<'ctx>,
        _field_idx: u32,
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
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::Decimal(_) => {
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
                "bus payload array `{}: [{:?}; {}]` — array element type \
                 not supported in m70 wire format. Supported elements: \
                 primitives (Int, Float, Decimal, Bool, Duration, Time) \
                 and user-struct TypeRefs. Strings, Bytes, and nested \
                 arrays are post-v1 polish.",
                fname, other, n
            ))),
        }
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
        _parent_struct_ty: StructType<'ctx>,
        _field_idx: u32,
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
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::Decimal(_) => {
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
                "bus payload array `{}: [{:?}; {}]` — array element type \
                 not supported in m70 wire format on the deserialize side.",
                fname, other, n
            ))),
        }
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
        let i8_t = self.context.i8_type();
        let cursor_alloca = self
            .builder
            .build_alloca(i64_t, "ser.cursor")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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

            match &field_ty {
                CodegenTy::String => {
                    // Wire: i64 LE length + N bytes (no NUL).
                    let str_ptr = self
                        .builder
                        .build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            src_field_ptr,
                            "ser.str.ptr",
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?
                        .into_pointer_value();
                    let str_len = self.emit_str_len_call(str_ptr)?;
                    // Write 8-byte length prefix.
                    let len_alloca = self
                        .builder
                        .build_alloca(i64_t, "ser.str.len")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
                CodegenTy::Int
                | CodegenTy::Float
                | CodegenTy::Bool
                | CodegenTy::Time
                | CodegenTy::Duration
                | CodegenTy::Decimal(_)
                | CodegenTy::Enum(_)
                    if self.wire_fixed_field(&field_ty) =>
                {
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
                CodegenTy::Bytes => {
                    // A8 (G15): Bytes payload field. The in-memory
                    // blob is `[i64 len][u8 data[len]]` (per
                    // memory.md § Bytes); the wire format matches.
                    // Load the Bytes pointer, read its length
                    // prefix, then memcpy `8 + len` bytes from
                    // the blob to dst. The deserializer mirrors
                    // this — allocate `8 + len`, memcpy, store
                    // the pointer.
                    let bytes_ptr = self
                        .builder
                        .build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            src_field_ptr,
                            "ser.bytes.ptr",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
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
                CodegenTy::TypeRef(nested_name) => {
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
                    let nested_src = self
                        .builder
                        .build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            src_field_ptr,
                            "ser.nested.load",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
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
                CodegenTy::Bounded(belem, bn) => {
                    if Self::bounded_elem_is_ptr(belem) {
                        return Err(CodegenError::Unsupported(format!(
                            "bus payload field `{}: bounded[{:?}; {}]` — \
                             pointer-element bounded cross-process is \
                             post-v1 polish; scalar-element bounded \
                             travels as flat bytes",
                            fname, belem, bn
                        )));
                    }
                    self.emit_bounded_field_wire_memcpy(
                        src_field_ptr,
                        dst,
                        cursor_alloca,
                        belem,
                        *bn,
                        true,
                        "ser.bounded",
                        None,
                    )?;
                }
                CodegenTy::Array(elem_ty, n) => {
                    // Form H (2026-05-20): fixed-size array payload.
                    // Two shapes supported: arrays of fixed-size
                    // primitives (memcpy N*elem_size inline) and
                    // arrays of TypeRef (N iterations of nested
                    // struct serialize). Other elem shapes (Array,
                    // String, Bytes) stay deferred — the dominant
                    // use case is array-of-flat-struct (e.g. a
                    // fixed-cap `[Cell; 10]` where Cell is a flat
                    // record of scalars).
                    self.emit_array_field_serialize(
                        src_field_ptr,
                        dst,
                        cursor_alloca,
                        struct_ty,
                        idx,
                        elem_ty,
                        *n,
                        fname,
                    )?;
                }
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "bus payload field `{}: {:?}` — m70 wire format \
                         supports primitives, String, Bytes, fixed-size \
                         arrays of primitives or TypeRefs, and nested \
                         structs (whose leaves are primitives/String/Bytes); \
                         tuples / enums and arrays of variable-length \
                         elements cross-process are post-v1 polish",
                        fname, other
                    )));
                }
            }
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
        let i8_t = self.context.i8_type();
        let cursor_alloca = self
            .builder
            .build_alloca(i64_t, "de.nested.cursor")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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

            match &field_ty {
                CodegenTy::String => {
                    let len_alloca = self
                        .builder
                        .build_alloca(i64_t, "de.nested.str.len.alloca")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
                    // only from its start).
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
                    self.builder
                        .build_store(dst_field_ptr, buf)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
                CodegenTy::Int
                | CodegenTy::Float
                | CodegenTy::Bool
                | CodegenTy::Time
                | CodegenTy::Duration
                | CodegenTy::Decimal(_)
                | CodegenTy::Enum(_)
                    if self.wire_fixed_field(&field_ty) =>
                {
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
                CodegenTy::Bytes => {
                    // A8 (G15) — nested-struct variant. Same shape
                    // as the top-level Bytes deserializer above:
                    // read 8-byte length, alloc `8 + len`, memcpy
                    // the prefixed blob, store ptr into dst.
                    let len_alloca = self
                        .builder
                        .build_alloca(i64_t, "de.nested.bytes.len.alloca")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
                    let total = self
                        .builder
                        .build_int_add(
                            bytes_len,
                            i64_t.const_int(8, false),
                            "de.nested.bytes.total",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // The prefix and the blob it announces, against
                    // what the frame holds after the cursor; a length
                    // that wraps the sum fails as a short frame does.
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
                    self.builder
                        .build_store(dst_field_ptr, buf)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
                CodegenTy::TypeRef(nested_name) => {
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
                    self.builder
                        .build_store(dst_field_ptr, nested_dst)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
                CodegenTy::Bounded(belem, bn) => {
                    if Self::bounded_elem_is_ptr(belem) {
                        return Err(CodegenError::Unsupported(format!(
                            "bus payload field `{}: bounded[{:?}; {}]` — \
                             pointer-element bounded cross-process is \
                             post-v1 polish",
                            fname, belem, bn
                        )));
                    }
                    self.emit_bounded_field_wire_memcpy(
                        dst_field_ptr,
                        src,
                        cursor_alloca,
                        belem,
                        *bn,
                        false,
                        "de.bounded",
                        Some((wire_n, fail_block)),
                    )?;
                }
                CodegenTy::Array(elem_ty, n) => {
                    self.emit_array_field_deserialize(
                        dst_field_ptr,
                        src,
                        cursor_alloca,
                        struct_ty,
                        idx,
                        elem_ty,
                        *n,
                        fname,
                        wire_n,
                        fail_block,
                    )?;
                }
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "bus payload field `{}: {:?}` — m70 wire format \
                         supports primitives, String, Bytes, fixed-size \
                         arrays of primitives or TypeRefs, and nested \
                         structs",
                        fname, other
                    )));
                }
            }
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
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal(_)
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
    /// A field the wire carries as its fixed bytes: a scalar, or a
    /// payload-free enum's i32 tag. An enum with payloads is a pointer to
    /// its cell, which the wire does not carry (the check refuses it at
    /// the topic, `struct_field_not_carried`).
    fn wire_fixed_field(&self, t: &CodegenTy) -> bool {
        match t {
            CodegenTy::Enum(n) => self.user_enums.get(n.as_str()).is_some_and(|i| !i.has_payload),
            _ => true,
        }
    }

    /// The byte count of a fixed wire field (`wire_fixed_field`).
    fn wire_fixed_size(&self, t: &CodegenTy) -> u64 {
        match t {
            CodegenTy::Enum(_) => 4,
            _ => codegen_ty_size_bytes(self.context, t),
        }
    }
}
