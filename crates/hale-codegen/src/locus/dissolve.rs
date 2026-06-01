//! Locus dissolve-time codegen: per-field drain / dissolve / arena
//! destroy + the m43 / k_max / draining helpers that run against a
//! locus receiver. Round 4a of the codegen model-org refactor.

use std::collections::BTreeMap;

use hale_syntax::ast::{
    BirthCheckDecl, CapacitySlotKind, ProjectionClass,
    RecognitionSubMode, ScheduleClass,
};
use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, LocusInfo, Scope, SlotForm,
};

pub(crate) trait LocusDissolve<'ctx> {
    fn lower_locus_kmax(
        &mut self,
        locus_name: &str,
        struct_ty: StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_locus_draining(
        &mut self,
        locus_name: &str,
        struct_ty: StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn emit_method_return_deep_copy(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError>;
    fn emit_locus_field_dissolves(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError>;
    fn emit_locus_field_owned_branch(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        fname: &str,
        tag: &str,
    ) -> Result<
        (
            Option<inkwell::basic_block::BasicBlock<'ctx>>,
            Option<inkwell::basic_block::BasicBlock<'ctx>>,
        ),
        CodegenError,
    >;

    /// Phase-2 (3) drain cascade. Per spec/runtime.md: "drain()
    /// cascades depth-first; children first, then self." Walks
    /// LocusRef-typed param fields in declaration order and calls
    /// each child's drain method, expected to be invoked BEFORE
    /// the outer locus's own drain at every cascade-teardown site.
    /// The companion `emit_locus_field_dissolves` runs the second
    /// half (closures → dissolve → arena_destroy) AFTER outer's
    /// dissolve body.
    fn emit_locus_field_drains(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError>;
    fn emit_birth_check(
        &mut self,
        bc: &BirthCheckDecl,
        self_ptr: PointerValue<'ctx>,
        info: &LocusInfo<'ctx>,
        locus_name: &str,
        scope: &mut Scope<'ctx>,
    ) -> Result<(), CodegenError>;
    fn emit_locus_arena_destroy(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> LocusDissolve<'ctx> for Cx<'ctx, 'p> {
    /// B14: lower the synthetic `k_max` read on a locus receiver.
    /// `cs` describes the receiver — `self_ptr` may be the current
    /// self or any other LocusRef value, so the same lowering serves
    /// `self.k_max` and `g.k_max`.
    fn lower_locus_kmax(
        &mut self,
        locus_name: &str,
        struct_ty: StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let load_field = |this: &mut Self, fname: &str| {
            let (fidx, fty) = fields.get(fname).cloned().ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "k_max requires param `{}` on locus `{}`",
                    fname, locus_name
                ))
            })?;
            let ptr = this
                .builder
                .build_struct_gep(
                    struct_ty,
                    self_ptr,
                    fidx,
                    &format!("kmax.{}.ptr", fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let llvm_ty = this.llvm_basic_type(&fty);
            let val = this
                .builder
                .build_load(llvm_ty, ptr, &format!("kmax.{}", fname))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            Ok::<_, CodegenError>((val, fty))
        };
        let (b_v, b_ty) = load_field(self, "B")?;
        let (c_v, c_ty) = load_field(self, "c")?;
        let (sigma_v, sigma_ty) = load_field(self, "sigma")?;
        let (phi_v, phi_ty) = load_field(self, "phi")?;
        let b_f = self.coerce_to_float(b_v, &b_ty, "k_max.B")?;
        let c_f = self.coerce_to_float(c_v, &c_ty, "k_max.c")?;
        let sigma_f = self.coerce_to_float(sigma_v, &sigma_ty, "k_max.sigma")?;
        let phi_f = match phi_ty {
            CodegenTy::Float => phi_v.into_float_value(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "k_max requires param `phi` of type Float, got {:?}",
                    other
                )));
            }
        };
        let f64_t = self.context.f64_type();
        let one = f64_t.const_float(1.0);
        let one_minus_phi = self
            .builder
            .build_float_sub(one, phi_f, "k_max.1mphi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let term_left = self
            .builder
            .build_float_mul(one_minus_phi, c_f, "k_max.term_left")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let term_right = self
            .builder
            .build_float_mul(phi_f, sigma_f, "k_max.term_right")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let denom = self
            .builder
            .build_float_add(term_left, term_right, "k_max.denom")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let k_max = self
            .builder
            .build_float_div(b_f, denom, "k_max")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((k_max.into(), CodegenTy::Float))
    }

    /// B14: lower the synthetic `draining` read on a locus receiver.
    /// Mirror of the `self.draining` path that pulls the
    /// `__drain_requested` i64 slot and returns it as a Bool.
    fn lower_locus_draining(
        &mut self,
        locus_name: &str,
        struct_ty: StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let info = self
            .user_loci
            .get(locus_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "draining: no LocusInfo for `{}`",
                    locus_name
                ))
            })?;
        let dr_ptr = self
            .builder
            .build_struct_gep(
                struct_ty,
                self_ptr,
                info.drain_requested_field_idx,
                "draining.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i64_t = self.context.i64_type();
        let raw = self
            .builder
            .build_load(i64_t, dr_ptr, "draining.raw")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let zero = i64_t.const_int(0, false);
        let as_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                raw,
                zero,
                "draining",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((as_bool.into(), CodegenTy::Bool))
    }

    /// Bus-arena reclaim (2026-05-21): deep-copy a method's heap
    /// return value out of the per-call scratch into the caller's
    /// arena. Reads the caller arena from TLS via
    /// `lotus_caller_arena_or_global` — the caller (free fn,
    /// other method body, main, etc.) is expected to set TLS
    /// via `emit_set_caller_arena` immediately before the call,
    /// so the lookup is a single load on the fast path. Scalars
    /// pass through unchanged. The actual recursive copy is
    /// delegated to `emit_return_value_deep_copy`, which also
    /// handles Tuple/Array/TypeRef/Interface/Bytes/String.
    fn emit_method_return_deep_copy(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if !Self::ty_needs_self_field_deep_copy(ty) {
            // Same set of types that need cross-arena copy
            // for self-field stores. Scalars / views / loci
            // / cells pass through.
            return Ok(value);
        }
        // Read the caller-arena snapshot we took at body entry.
        // Reading TLS here would land in the wrong arena if any
        // nested method/stdlib call has clobbered it (every such
        // call publishes its own caller-arena before invoking),
        // so we keep the entry-time snapshot in a local alloca
        // and reuse it across every `return` in this body.
        let slot = self
            .current_method_caller_arena
            .expect("method scratch active implies caller-arena snapshot");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let dest_arena = self
            .builder
            .build_load(ptr_t, slot, "method.caller_arena.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        // 2026-05-22 PM (sret-style m49 follow-on): route through
        // the same-arena-skip wrapper. Combined with
        // `current_arena_override = caller_arena` set during
        // return-expr lowering in `lower_return`, fresh aggregate
        // literals at return position land directly in caller_arena
        // (via current_arena_ptr's override path) and the contains
        // check here passes them through unchanged — no second
        // memcpy, no fresh alloc. For non-literal returns (aliases,
        // self.field reads, let-bound transients in scratch), the
        // contains check fails and the existing recursive deep-copy
        // fires unchanged. Source of the SymbolBook leak class
        // bisected from the 2026-05-22 PM residency dump:
        // sweep_buy() returned a SweepResult literal that lived in
        // scratch (subregion of SymbolBook arena) and got memcpy'd
        // into caller_arena at this boundary — both halves of the
        // round-trip are now eliminated for the common literal case.
        let payload_val = value.is_pointer_value();
        if payload_val {
            self.emit_cross_arena_store_deep_copy_ptr(
                value, ty, dest_arena, "method.return",
            )
        } else {
            // Struct-by-value returns (Interface fat-pointer) — the
            // value is held in registers / a struct SSA, not an
            // arena-resident pointer. Same-arena skip is N/A; fall
            // through to the unconditional recursive deep-copy.
            self.emit_return_value_deep_copy(value, ty, dest_arena)
        }
    }

    /// Emit `lotus_arena_destroy(<load self_ptr->__arena>)` for a
    /// just-dissolved locus. Used in both the ephemeral-locus
    /// dissolve path (lower_locus_instantiation) and the deferred-
    /// dissolve flush at body exit. Safe to call after the
    /// dissolve method body has run; the arena is the LAST piece
    /// of the locus's state to go.
    /// Phase-2 (2): cascade dissolve for parent-owned child loci
    /// stored in `LocusRef`-typed param fields. Called right before
    /// the outer locus's own `arena_destroy` (both in the ephemeral
    /// dispatch and in `flush_dissolve_frame`). For each field whose
    /// declared type is `LocusRef(<inner>)`, this loads the inner's
    /// self_ptr from the field slot and emits the same `drain →
    /// __dissolve_closures → dissolve → arena_destroy` sequence
    /// the inner would run under ephemeral semantics. Without this
    /// cascade, locus literals constructed as field defaults
    /// (Phase-2 (2)'s motivating shape — `rx_buf: BytesBuilder =
    /// std::bytes::BytesBuilder { ... };`) leak their malloc-backed
    /// state at the outer's dissolve.
    ///
    /// Cascade ordering: outer's `field_drains → drain → closures →
    /// dissolve` runs first, then this cascade fires per child's
    /// `closures → dissolve → arena_destroy`, then outer's
    /// arena_destroy. The choice matches "outer's user dissolve
    /// body may still legitimately touch its inner field" — the
    /// inner is alive through outer's dissolve body and only torn
    /// down after. The child's drain step itself ran earlier via
    /// `emit_locus_field_drains` so that drain cascades depth-first
    /// per spec/runtime.md "drain() cascades depth-first; children
    /// first, then self."
    fn emit_locus_field_dissolves(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // F.31 Phase 3b: when this locus IS the main locus, skip
        // the cascade for fields whose placement is `pinned`. The
        // pinned children's lifecycle ran on the pthread; calling
        // their drain/dissolve here would double-dispatch. Their
        // pthread_join + arena_destroy happen via the
        // deferred-dissolve frame's flush at fn-scope exit.
        let is_main_locus = self
            .main_locus_name
            .as_ref()
            .map(|n| n == locus_name)
            .unwrap_or(false);
        // Sort field iteration by index so ordering is deterministic
        // (BTreeMap iter is sorted by key — name; we want index).
        let mut field_entries: Vec<(String, u32, CodegenTy)> = info
            .fields
            .iter()
            .map(|(n, (idx, ty))| (n.clone(), *idx, ty.clone()))
            .collect();
        field_entries.sort_by_key(|(_, idx, _)| *idx);
        for (fname, field_idx, field_ty) in field_entries {
            let inner_name = match field_ty {
                CodegenTy::LocusRef(n) => n,
                _ => continue,
            };
            // F.31 Phase 3b: skip cascade for pinned-placed fields.
            if is_main_locus
                && matches!(
                    self.main_placement_map.get(&fname),
                    Some(ScheduleClass::Pinned(_))
                )
            {
                continue;
            }
            let inner_info = match self.user_loci.get(&inner_name).cloned() {
                Some(i) => i,
                None => continue, // shouldn't happen for a typed field
            };
            // F.29 follow-up: ownership branch. Wrap the cascade
            // body in an `if (__locus_ref_owned_mask >> bit) & 1`
            // check so externally-provided fields (variable-ref
            // overrides) skip the per-child teardown — they're
            // owned by an outer scope and will tear themselves
            // down at THEIR scope exit. Without the gate, the
            // parent's cascade would dissolve the external and
            // its real owner's later teardown would hit freed
            // memory.
            let (owned_then_bb, owned_after_bb) = self
                .emit_locus_field_owned_branch(
                    info, self_ptr, locus_name, &fname,
                    "cascade.dissolve",
                )?;
            let after_bb = match (owned_then_bb, owned_after_bb) {
                (Some(then_bb), Some(after_bb)) => {
                    self.builder.position_at_end(then_bb);
                    Some(after_bb)
                }
                _ => None,
            };
            // GEP the field slot, load the inner self_ptr.
            let field_slot_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    field_idx,
                    &format!("{}.{}.cascade.gep", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let inner_ptr = self
                .builder
                .build_load(
                    ptr_t,
                    field_slot_ptr,
                    &format!("{}.{}.cascade.load", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            // __dissolve_closures → dissolve → arena_destroy. The
            // drain step ran earlier via `emit_locus_field_drains`
            // (depth-first before outer's drain) so this teardown
            // half can assume children have already drained.
            if let Some(closures_fn) = inner_info.dissolve_closures_fn {
                let (parent_self, handler_ptr) =
                    self.resolve_failure_route(&inner_name);
                self.builder
                    .build_call(
                        closures_fn,
                        &[
                            inner_ptr.into(),
                            parent_self.into(),
                            handler_ptr.into(),
                        ],
                        &format!("{}.cascade.closures", inner_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            if let Some(dissolve_fn) = inner_info.methods.get("dissolve") {
                if !inner_info.empty_lifecycle.contains("dissolve") {
                    self.builder
                        .build_call(
                            *dissolve_fn,
                            &[inner_ptr.into()],
                            &format!("{}.cascade.dissolve", inner_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            // Inner's arena_destroy. Even when inner allocates
            // nothing in its arena, the slot was created at birth
            // and must be destroyed for symmetry.
            self.emit_locus_arena_destroy(&inner_info, inner_ptr, &inner_name)?;
            // Close the owned-branch (if one was emitted): jump to
            // the after-bb and position the builder there so the
            // next field's emission begins in the correct block.
            if let Some(after_bb) = after_bb {
                self.builder
                    .build_unconditional_branch(after_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(after_bb);
            }
        }
        Ok(())
    }

    /// F.29 follow-up: emit the `owned-bit gate` around a
    /// per-field cascade body. Returns `(Some(then_bb), Some(after_bb))`
    /// when the field is LocusRef-typed and the locus has a
    /// non-empty ownership-mask layout — caller positions the
    /// builder at `then_bb`, emits the cascade body, then must
    /// branch to `after_bb` and position there at the end.
    /// Returns `(None, None)` for non-LocusRef fields (caller
    /// continues emission in-line as before). The cascade body
    /// stays linear in that no-gate case.
    fn emit_locus_field_owned_branch(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        fname: &str,
        tag: &str,
    ) -> Result<
        (
            Option<inkwell::basic_block::BasicBlock<'ctx>>,
            Option<inkwell::basic_block::BasicBlock<'ctx>>,
        ),
        CodegenError,
    > {
        let bit_pos = match info.locus_ref_bit_per_field.get(fname) {
            Some(&b) => b,
            None => return Ok((None, None)),
        };
        let func = self.current_fn.expect("current_fn set");
        let then_bb = self
            .context
            .append_basic_block(func, &format!("{}.{}.owned.then", tag, fname));
        let after_bb = self
            .context
            .append_basic_block(func, &format!("{}.{}.owned.after", tag, fname));
        let i64_t_local = self.context.i64_type();
        let mask_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.locus_ref_owned_mask_field_idx,
                &format!(
                    "{}.{}.{}.mask.ptr", locus_name, tag, fname
                ),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let mask = self
            .builder
            .build_load(
                i64_t_local,
                mask_ptr,
                &format!("{}.{}.{}.mask", locus_name, tag, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let bit = i64_t_local.const_int(1u64 << bit_pos, false);
        let anded = self
            .builder
            .build_and(
                mask,
                bit,
                &format!("{}.{}.{}.anded", locus_name, tag, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let owned = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                anded,
                i64_t_local.const_zero(),
                &format!("{}.{}.{}.owned", locus_name, tag, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(owned, then_bb, after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((Some(then_bb), Some(after_bb)))
    }

    /// Phase-2 (3) drain cascade. Per spec/runtime.md: "drain()
    /// cascades depth-first; children first, then self." Walks
    /// LocusRef-typed param fields in declaration order and calls
    /// each child's drain method, expected to be invoked BEFORE
    /// the outer locus's own drain at every cascade-teardown site.
    /// The companion `emit_locus_field_dissolves` runs the second
    /// half (closures → dissolve → arena_destroy) AFTER outer's
    /// dissolve body.
    fn emit_locus_field_drains(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // F.31 Phase 3b: skip cascade drain for pinned-placed
        // fields. See emit_locus_field_dissolves's matching guard.
        let is_main_locus = self
            .main_locus_name
            .as_ref()
            .map(|n| n == locus_name)
            .unwrap_or(false);
        let mut field_entries: Vec<(String, u32, CodegenTy)> = info
            .fields
            .iter()
            .map(|(n, (idx, ty))| (n.clone(), *idx, ty.clone()))
            .collect();
        field_entries.sort_by_key(|(_, idx, _)| *idx);
        for (fname, field_idx, field_ty) in field_entries {
            let inner_name = match field_ty {
                CodegenTy::LocusRef(n) => n,
                _ => continue,
            };
            if is_main_locus
                && matches!(
                    self.main_placement_map.get(&fname),
                    Some(ScheduleClass::Pinned(_))
                )
            {
                continue;
            }
            let inner_info = match self.user_loci.get(&inner_name).cloned() {
                Some(i) => i,
                None => continue,
            };
            let drain_fn = match inner_info.methods.get("drain") {
                Some(f) if !inner_info.empty_lifecycle.contains("drain") => *f,
                _ => continue,
            };
            // F.29 follow-up: ownership branch (same gate as
            // emit_locus_field_dissolves). Externally-provided
            // fields skip the cascade — their real owner runs
            // drain at THEIR scope exit.
            let (owned_then_bb, owned_after_bb) = self
                .emit_locus_field_owned_branch(
                    info, self_ptr, locus_name, &fname,
                    "cascade.drain",
                )?;
            let after_bb = match (owned_then_bb, owned_after_bb) {
                (Some(then_bb), Some(after_bb)) => {
                    self.builder.position_at_end(then_bb);
                    Some(after_bb)
                }
                _ => None,
            };
            let field_slot_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    field_idx,
                    &format!("{}.{}.drain.gep", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let inner_ptr = self
                .builder
                .build_load(
                    ptr_t,
                    field_slot_ptr,
                    &format!("{}.{}.drain.load", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            self.builder
                .build_call(
                    drain_fn,
                    &[inner_ptr.into()],
                    &format!("{}.cascade.drain", inner_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            if let Some(after_bb) = after_bb {
                self.builder
                    .build_unconditional_branch(after_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(after_bb);
            }
        }
        Ok(())
    }

    /// F.27 v2: emit one birth_check inline at the instantiation
    /// site. Evaluates the cond expression; if true, routes
    /// through the violate machinery (sets __drain_requested,
    /// indirect-calls parent.on_failure, OR panics with diagnostic
    /// when no handler), then BRANCHES to a continuation block
    /// instead of returning from the caller's LLVM function —
    /// which is the key difference from the regular Stmt::Violate
    /// codegen. After this routine returns, the builder is
    /// positioned at the "after this birth_check" block; a
    /// subsequent birth_check can be emitted onto that. The
    /// caller (e.g., Parent.run) keeps running normally after
    /// the instantiation when a parent handler absorbs the
    /// violation; the panic branch is the unabsorbed case and
    /// terminates the process per F.27's contract.
    ///
    /// Pre: `current_self` must be set to the newly-constructed
    /// locus so cond's `self.X` reads resolve against its
    /// fields.
    fn emit_birth_check(
        &mut self,
        bc: &BirthCheckDecl,
        self_ptr: PointerValue<'ctx>,
        info: &LocusInfo<'ctx>,
        locus_name: &str,
        scope: &mut Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        // 1. Lower cond → i1.
        let (cond_v, cond_ty) = self.lower_expr(&bc.cond, scope)?;
        if cond_ty != CodegenTy::Bool {
            return Err(CodegenError::Unsupported(format!(
                "birth_check cond must be Bool, got {:?}",
                cond_ty
            )));
        }
        let func = self.current_fn.expect("current_fn set");
        let route_bb = self
            .context
            .append_basic_block(func, "bcheck.route");
        let after_bb = self
            .context
            .append_basic_block(func, "bcheck.after");
        self.builder
            .build_conditional_branch(
                cond_v.into_int_value(),
                route_bb,
                after_bb,
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // 2. route_bb: violate machinery — copy of Stmt::Violate's
        // body (lines ~17065+), trimmed to the routing + ALWAYS
        // branching to after_bb at the end (instead of returning).
        self.builder.position_at_end(route_bb);
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();

        // Set __drain_requested = 1.
        let one = i64_t.const_int(1, false);
        let dr_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.drain_requested_field_idx,
                "bcheck.dr.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(dr_ptr, one)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Read parent_self + parent_on_failure from the locus
        // struct (set at instantiation via resolve_failure_route).
        let ps_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_self_field_idx,
                "bcheck.parent_self.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parent_self = self
            .builder
            .build_load(ptr_t, ps_ptr, "bcheck.parent_self")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let poh_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_on_failure_field_idx,
                "bcheck.parent_on_failure.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parent_on_failure = self
            .builder
            .build_load(ptr_t, poh_ptr, "bcheck.parent_on_failure")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();

        // Allocate + fill ClosureViolation.
        let viol_info = self
            .user_types
            .get("ClosureViolation")
            .cloned()
            .expect("ClosureViolation declared at startup");
        let size = viol_info
            .struct_ty
            .size_of()
            .expect("violation struct has known size");
        let viol_ptr = self.arena_alloc(size, "bcheck.viol.alloc")?;
        let locus_str = self.global_string(locus_name);
        let closure_str = self.global_string(&bc.closure_name.name);
        let f0 = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                0,
                "bcheck.viol.locus.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(f0, locus_str)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f1 = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                1,
                "bcheck.viol.closure.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(f1, closure_str)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f2 = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                2,
                "bcheck.viol.diff.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(f2, i64_t.const_zero())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Branch on parent_on_failure null.
        let route_then = self
            .context
            .append_basic_block(func, "bcheck.handler");
        let route_bare = self
            .context
            .append_basic_block(func, "bcheck.bare");
        let null_check = self
            .builder
            .build_is_not_null(parent_on_failure, "bcheck.has.handler")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(null_check, route_then, route_bare)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // route_then: indirect-call parent.on_failure(parent_self,
        // self_ptr, viol_ptr), then branch to after_bb.
        self.builder.position_at_end(route_then);
        let handler_callee_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.builder
            .build_indirect_call(
                handler_callee_ty,
                parent_on_failure,
                &[
                    parent_self.into(),
                    self_ptr.into(),
                    viol_ptr.into(),
                ],
                "bcheck.on_failure.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // route_bare: dprintf + exit(1) — unabsorbed violation.
        self.builder.position_at_end(route_bare);
        let fflush_fn = self
            .module
            .get_function("fflush")
            .expect("fflush declared");
        self.builder
            .build_call(
                fflush_fn,
                &[ptr_t.const_null().into()],
                "bcheck.fflush",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fmt = self.global_string(
            "runtime error: ClosureViolation: locus `%s` closure `%s` (birth_check, no parent handler)\n",
        );
        let dprintf_fn = self
            .module
            .get_function("dprintf")
            .expect("dprintf declared");
        self.builder
            .build_call(
                dprintf_fn,
                &[
                    i32_t.const_int(2, false).into(),
                    fmt.into(),
                    locus_str.into(),
                    closure_str.into(),
                ],
                "bcheck.dprintf",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let exit_fn = self
            .module
            .get_function("exit")
            .expect("exit declared");
        self.builder
            .build_call(
                exit_fn,
                &[i32_t.const_int(1, false).into()],
                "bcheck.exit",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // 3. Position at after_bb so subsequent birth_check
        // clauses (and the rest of instantiation) emit there.
        self.builder.position_at_end(after_bb);
        Ok(())
    }

    fn emit_locus_arena_destroy(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        // Arena-elision counterpart: when `__arena` was pointed
        // at the caller's arena at instantiation (see
        // `locus_arena_elidable` + the matching branch in
        // `lower_locus_instantiation`'s Fresh-strategy path),
        // there's nothing to tear down — no bus subscriptions
        // (predicate rejects them), no capacity slots (rejected),
        // and the arena belongs to someone else. Calling
        // `lotus_arena_destroy` here would free a live arena
        // that the surrounding fn still owns. Bail.
        if info.arena_elidable {
            return Ok(());
        }
        // 2026-06-01: reclaim this locus's accept'd children BEFORE
        // tearing down its arena (their subregions live inside it).
        // This is the single teardown chokepoint every dissolve path
        // funnels through — graceful-shutdown frame, parent
        // field-dissolve, a reclaimed flow/terminated child, and the
        // ephemeral scope-exit — so the cascade is uniform and
        // recursive (a reclaimed child reclaims its own grandchildren)
        // without duplicating the walk at each site. No-op unless this
        // locus both `accept`s and tracks a children buffer; idempotent
        // (each child's __reclaim is latched), and flow children that
        // self-reclaimed mid-life already removed themselves from the
        // tracker, so they aren't re-touched here.
        self.emit_accepted_children_reclaim(info, self_ptr, locus_name)?;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // Deregister from the bus router BEFORE freeing the arena.
        // Without this step, a stale entry in the C-runtime entries
        // vec would point self_ptr at memory whose arena is about
        // to be freed; a subsequent `<-` to one of this locus's
        // subscriptions would have dispatch read `*(arena_t **)
        // self_ptr` after free, then memcpy a payload into freed
        // chunks. Today's programs don't publish post-dissolve,
        // but the invariant is fragile — close it here using the
        // same null-subject-sentinel mechanism `quarantine(c)`
        // already uses (m41b / m45-followup-2). No-op when the
        // program has no subscribes.
        if self.bus_state.is_some() {
            let unsub_fn = self
                .module
                .get_function("lotus_bus_quarantine_self")
                .expect("lotus_bus_quarantine_self declared");
            self.builder
                .build_call(
                    unsub_fn,
                    &[self_ptr.into()],
                    &format!("{}.bus.deregister.call", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // 2026-05-29: free the growable accept'd-children tracker
        // buffer (heap-allocated by lotus_children_push, separate
        // from the arena). NULL-safe in the runtime, so a parent
        // that declared accept + iterates children but never
        // accepted one pays nothing. Only present on loci that
        // iterate `self.children` (children_field_idx is None
        // otherwise, including on the accept'd children themselves).
        if let Some(arr_idx) = info.children_field_idx {
            let arr_field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    arr_idx,
                    &format!("{}.children.free.ptr", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let buf = self
                .builder
                .build_load(ptr_t, arr_field_ptr, "children.buf")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let free_fn = self
                .module
                .get_function("lotus_children_free")
                .expect("lotus_children_free declared");
            self.builder
                .build_call(
                    free_fn,
                    &[buf.into()],
                    &format!("{}.children.free.call", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // F.22: tear down capacity slots in reverse declaration
        // order, before slot 0 / arena destroy. Each slot loads
        // its allocator pointer from `__slot_<name>` and calls
        // the matching destroy fn. Per spec §F.22, slot teardown
        // sits between drain/dissolve closures and the arena's
        // wholesale free, so cells outlive everything except
        // the arena itself during dissolve.
        //
        // v1.x-4b: slots whose bit in __slot_borrowed_mask is set
        // were borrowed from a parent (the parent still owns the
        // underlying allocator and will dissolve it via its own
        // slot-destroy pass — per F.4 depth-first cascade, this
        // locus has dissolved by the time the parent's destroy
        // runs). Skip the destroy call on those slots. Read the
        // mask once at the top of the destroy pass; per-slot
        // checks use a const bit mask.
        let i64_t_local = self.context.i64_type();
        let bool_t_local = self.context.bool_type();
        let mask_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.slot_borrowed_mask_field_idx,
                &format!("{}.__slot_borrowed_mask.dissolve.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let borrowed_mask = self
            .builder
            .build_load(
                i64_t_local,
                mask_field_ptr,
                &format!("{}.__slot_borrowed_mask.dissolve", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let destroy_func = self
            .current_fn
            .ok_or_else(|| {
                CodegenError::Unsupported(
                    "slot destroy emit requires a current fn context".into(),
                )
            })?;
        for (child_idx, slot) in info.capacity_slots.iter().enumerate().rev() {
            let slot_field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    slot.struct_field_idx,
                    &format!("{}.__slot_{}.ptr", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // Per-slot borrowed-bit check. AND with the bit mask;
            // compare != 0 → i1; conditional branch around the
            // destroy. Form-vec slots can't be borrowed (we reject
            // that at slot init), so the bit is always 0 for them
            // and the destroy always fires — the conditional is
            // cheap (one AND + one cmp + one cond_br) and uniform.
            let bit = i64_t_local.const_int(1u64 << child_idx, false);
            let masked = self
                .builder
                .build_and(
                    borrowed_mask,
                    bit,
                    &format!("{}.__slot_{}.is_borrowed.masked", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let is_borrowed = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    masked,
                    i64_t_local.const_int(0, false),
                    &format!("{}.__slot_{}.is_borrowed", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let _ = bool_t_local;
            let destroy_bb = self.context.append_basic_block(
                destroy_func,
                &format!("{}.__slot_{}.destroy_path", locus_name, slot.name),
            );
            let cont_bb = self.context.append_basic_block(
                destroy_func,
                &format!("{}.__slot_{}.destroy_cont", locus_name, slot.name),
            );
            self.builder
                .build_conditional_branch(is_borrowed, cont_bb, destroy_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(destroy_bb);
            match slot.form {
                Some(SlotForm::Vec) => {
                    // v1.x-FORM-2: free the vec's malloc'd buffer
                    // (if any). The struct field itself is part of
                    // the locus and dies with the arena.
                    let destroy_fn = self
                        .module
                        .get_function("lotus_vec_destroy")
                        .expect("lotus_vec_destroy extern declared");
                    self.builder
                        .build_call(
                            destroy_fn,
                            &[slot_field_ptr.into()],
                            &format!("{}.{}.destroy", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Some(SlotForm::Hashmap) => {
                    // v1.x-FORM-4: free the hashmap's slot-array
                    // buffer. The lotus_hashmap_t struct itself is
                    // inline in the locus and dies with the arena.
                    let destroy_fn = self
                        .module
                        .get_function("lotus_hashmap_destroy")
                        .expect("lotus_hashmap_destroy extern declared");
                    self.builder
                        .build_call(
                            destroy_fn,
                            &[slot_field_ptr.into()],
                            &format!("{}.{}.destroy", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Some(SlotForm::RingBuffer) => {
                    // v1.x-FORM-5: free the ring buffer's backing
                    // `buf`. The lotus_ring_buffer_t struct itself
                    // is inline in the locus and dies with the
                    // arena.
                    let destroy_fn = self
                        .module
                        .get_function("lotus_ring_buffer_destroy")
                        .expect("lotus_ring_buffer_destroy extern declared");
                    self.builder
                        .build_call(
                            destroy_fn,
                            &[slot_field_ptr.into()],
                            &format!("{}.{}.destroy", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                None => {
                    let allocator = self
                        .builder
                        .build_load(
                            ptr_t,
                            slot_field_ptr,
                            &format!("{}.__slot_{}", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let destroy_fn_name = match slot.kind {
                        CapacitySlotKind::Pool => "lotus_pool_destroy",
                        CapacitySlotKind::Heap => "lotus_heap_destroy",
                    };
                    let destroy_fn = self
                        .module
                        .get_function(destroy_fn_name)
                        .expect("F.22 allocator destroy extern declared");
                    self.builder
                        .build_call(
                            destroy_fn,
                            &[allocator.into()],
                            &format!("{}.{}.destroy", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(cont_bb);
        }

        // v1.x-3: if THIS locus is a recognition parent with a
        // shipped sub-mode, destroy its recpool now — after slot
        // teardown (existing pass above) and before arena teardown
        // (below). The F.4 depth-first cascade has already
        // dissolved every child by the time we get here; each
        // child's dissolve called the matching recpool_release
        // (no-op for slab; bitmap-clear for fixed) so it's safe
        // to wholesale-free the recpool's storage.
        if let ProjectionClass::Recognition(Some(params)) = info.projection_class {
            let destroy_fn_name = match params.sub_mode {
                RecognitionSubMode::FixedCell => "lotus_recpool_fixed_destroy",
                RecognitionSubMode::SharedSlab => "lotus_recpool_slab_destroy",
                _ => "", // typecheck-rejected; defense
            };
            if !destroy_fn_name.is_empty() {
                let recpool_field_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        self_ptr,
                        info.recpool_field_idx,
                        &format!("{}.__recpool.dissolve.ptr", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let recpool_handle = self
                    .builder
                    .build_load(
                        ptr_t,
                        recpool_field_ptr,
                        &format!("{}.__recpool.dissolve", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let destroy_fn = self
                    .module
                    .get_function(destroy_fn_name)
                    .expect("recpool destroy extern declared");
                self.builder
                    .build_call(
                        destroy_fn,
                        &[recpool_handle.into()],
                        &format!("{}.__recpool.destroy", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }

        // v1.x-3: tear down THIS locus's __arena. The release path
        // depends on __recpool_release_kind:
        //   0 → regular `lotus_arena_destroy(arena)` (top-level arena
        //       or subregion of a Chunked parent — both shapes the
        //       arena's own destroy handles cleanly).
        //   1 → `lotus_recpool_fixed_release(parent_pool, arena)`
        //       (arena lives inline in a fixed_cell; release just
        //       clears the bitmap bit so the slot is reusable).
        //   2 → `lotus_recpool_slab_release(parent_pool, arena)`
        //       (no-op — slab is freed wholesale at parent dissolve).
        let arena_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.arena_field_idx,
                &format!("{}.__arena.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let arena = self
            .builder
            .build_load(ptr_t, arena_field_ptr, &format!("{}.__arena", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let release_kind_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.recpool_release_kind_field_idx,
                &format!("{}.__recpool_release_kind.dissolve.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let release_kind = self
            .builder
            .build_load(
                i64_t_local,
                release_kind_ptr,
                &format!("{}.__recpool_release_kind.dissolve", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let release_pool_ptr_field = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.recpool_release_pool_field_idx,
                &format!("{}.__recpool_release_pool.dissolve.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let release_pool = self
            .builder
            .build_load(
                ptr_t,
                release_pool_ptr_field,
                &format!("{}.__recpool_release_pool.dissolve", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let regular_bb = self.context.append_basic_block(
            destroy_func,
            &format!("{}.arena.destroy.regular", locus_name),
        );
        let fixed_bb = self.context.append_basic_block(
            destroy_func,
            &format!("{}.arena.destroy.fixed", locus_name),
        );
        let slab_bb = self.context.append_basic_block(
            destroy_func,
            &format!("{}.arena.destroy.slab", locus_name),
        );
        let after_bb = self.context.append_basic_block(
            destroy_func,
            &format!("{}.arena.destroy.after", locus_name),
        );

        // 2026-05-30 idempotent-teardown latch. `__arena` is NULL'd
        // in `after_bb` once this locus is reclaimed, so a SECOND
        // arena-destroy — e.g. a child's run-completion reclaim
        // racing the parent's dissolve cascade for the same locus —
        // loads NULL here and branches straight to `after_bb`,
        // skipping the release (which would double-free). Whichever
        // teardown reaches the locus first wins; the rest no-op.
        // (Single-threaded by construction at the call sites that
        // currently collide: the pool workers are joined before the
        // parent's dissolve runs, so no atomic is needed yet.)
        let arena_is_null = self
            .builder
            .build_is_null(
                arena.into_pointer_value(),
                &format!("{}.arena.already_freed", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let do_destroy_bb = self.context.append_basic_block(
            destroy_func,
            &format!("{}.arena.destroy.do", locus_name),
        );
        self.builder
            .build_conditional_branch(arena_is_null, after_bb, do_destroy_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(do_destroy_bb);

        let is_zero = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                release_kind,
                i64_t_local.const_int(0, false),
                &format!("{}.release_kind.is_zero", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let recpool_dispatch_bb = self.context.append_basic_block(
            destroy_func,
            &format!("{}.arena.destroy.recpool", locus_name),
        );
        self.builder
            .build_conditional_branch(is_zero, regular_bb, recpool_dispatch_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Regular arena destroy.
        self.builder.position_at_end(regular_bb);
        let destroy = self
            .module
            .get_function("lotus_arena_destroy")
            .expect("lotus_arena_destroy declared");
        self.builder
            .build_call(destroy, &[arena.into()], &format!("{}.arena.destroy", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Recpool dispatch: kind 1 → fixed, else (kind 2) → slab.
        self.builder.position_at_end(recpool_dispatch_bb);
        let is_fixed = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                release_kind,
                i64_t_local.const_int(1, false),
                &format!("{}.release_kind.is_fixed", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_fixed, fixed_bb, slab_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(fixed_bb);
        let fixed_release_fn = self
            .module
            .get_function("lotus_recpool_fixed_release")
            .expect("lotus_recpool_fixed_release declared");
        self.builder
            .build_call(
                fixed_release_fn,
                &[release_pool.into(), arena.into()],
                &format!("{}.arena.recpool.fixed.release", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(slab_bb);
        let slab_release_fn = self
            .module
            .get_function("lotus_recpool_slab_release")
            .expect("lotus_recpool_slab_release declared");
        self.builder
            .build_call(
                slab_release_fn,
                &[release_pool.into(), arena.into()],
                &format!("{}.arena.recpool.slab.release", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(after_bb);
        // Set the latch: mark this locus reclaimed so any later
        // teardown of the same locus (the skip branch above) no-ops.
        // Reached from both the post-release path (arena was live →
        // now freed) and the skip branch (arena already NULL → store
        // is a harmless no-op).
        let null_arena = ptr_t.const_null();
        self.builder
            .build_store(arena_field_ptr, null_arena)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

}
