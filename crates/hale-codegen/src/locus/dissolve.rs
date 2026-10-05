//! Locus dissolve-time codegen: per-field drain / dissolve / arena
//! destroy + the m43 / k_max / draining helpers that run against a
//! locus receiver. Round 4a of the codegen model-org refactor.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    BirthCheckDecl, CapacitySlotKind, ProjectionClass,
    RecognitionSubMode, ScheduleClass,
};
use hale_types::lifecycle::spine::{ReclaimStep, CASCADE_STEPS};
use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, LocusInfo, Scope, SlotForm,
};

/// The halves of a contract-typed field's recorded teardown, as indices
/// into its pair (`Cx::contract_teardown_table`).
pub(crate) const CONTRACT_DRAIN: u64 = 0;
pub(crate) const CONTRACT_REST: u64 = 1;

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

    /// GH #871: the cascade arm for a param field that holds an
    /// owned child WITHOUT naming its type — an `interface` slot or
    /// a `perspective(P)` handle. Runs one half of the child's
    /// teardown through the pair the instantiation recorded in
    /// `__owned_child_reclaim_<f>` (`Cx::contract_teardown_table`):
    /// its drain ([`CONTRACT_DRAIN`], in the owner's drain cascade) or
    /// the rest of its spine ([`CONTRACT_REST`], in the owner's
    /// dissolve cascade).
    #[allow(clippy::too_many_arguments)]
    fn emit_owned_contract_child_teardown(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        fname: &str,
        field_idx: u32,
        via_fat_pointer: bool,
        half: u64,
    ) -> Result<(), CodegenError>;

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
    fn emit_reclaimed_child_skip(
        &mut self,
        inner_info: &LocusInfo<'ctx>,
        inner_ptr: PointerValue<'ctx>,
        locus_name: &str,
        fname: &str,
        tag: &str,
        owns_claim: bool,
    ) -> Result<inkwell::basic_block::BasicBlock<'ctx>, CodegenError>;
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
        // GH #1039: a locus is also draining once the process is —
        // the SIGINT / SIGTERM drain raises one runtime flag rather
        // than walking every locus. A plain monotonic load, so a hot
        // handler's `if !self.draining` stays a load, not a call.
        // Where no signal reaches the program the flag is always 0, and
        // the term is the `DrainTerm` obligation's omission.
        let raw = if !self.cells.emits(hale_types::capability::Obligation::DrainTerm) {
            raw
        } else {
            self.reads_draining = true;
            // GH #1077: the locus whose code runs this read is the one
            // that can answer a drain — the enclosing `self`, whatever
            // receiver is read.
            match self.current_self.as_ref().map(|cs| cs.locus_name.clone()) {
                Some(reader) => {
                    self.drain_observer_loci.insert(reader);
                }
                None => self.drain_read_outside_locus = true,
            }
            let proc_raw = self.emit_process_draining_load("draining.process")?;
            self.builder
                .build_or(raw, proc_raw, "draining.any")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
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
        let retain = self.emit_reclaim_scope_enter(self_ptr)?;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // F.31 Phase 3b: when this locus IS the main locus, skip
        // the cascade for fields whose placement is `pinned`. The
        // pinned children's lifecycle ran on the pthread; calling
        // their drain/dissolve here would double-dispatch. Their
        // pthread_join + arena_destroy happen via the
        // deferred-dissolve frame's flush at fn-scope exit.
        let is_main_locus = self.is_lowering_root(locus_name);
        // The fields in the order the plan places their teardowns (line
        // 12: declaration order). Physical release can remain deferred.
        let field_entries = self.cascade_field_entries(info, locus_name)?;
        for (fname, field_idx, field_ty) in field_entries {
            // F.31 Phase 3b: skip cascade for pinned-placed fields.
            if is_main_locus
                && matches!(
                    self.deployment.main_placement_map.get(&fname),
                    Some(ScheduleClass::Pinned(_))
                )
            {
                continue;
            }
            // GH #871: a field typed by a CONTRACT — an `interface`
            // or a `perspective(P)` — holds a parent-owned child
            // just as a `LocusRef` field does, and this walk used to
            // step straight over it because the declared type is not
            // `LocusRef`. Nothing else tore those children down: the
            // literal took `lower_locus_instantiation`'s
            // `parent_owns_via_field` no-op branch (so no frame owns
            // it) and the owner's cascade never looked at the field.
            // `Queries { j: Churner { } }` lost the `Churner` AND
            // the `Rows` `@form(vec)` under it; every perspective
            // holder lost its designated impl.
            //
            // The teardown is an indirect call because the impl is
            // chosen per instantiation, not per owner type — see
            // `owned_child_reclaim_field_idxs`.
            if matches!(
                field_ty,
                CodegenTy::Interface(_) | CodegenTy::Perspective(_)
            ) {
                let via_fat_pointer =
                    matches!(field_ty, CodegenTy::Interface(_));
                self.emit_owned_contract_child_teardown(
                    info,
                    self_ptr,
                    locus_name,
                    &fname,
                    field_idx,
                    via_fat_pointer,
                    CONTRACT_REST,
                )?;
                continue;
            }
            let inner_name = match field_ty {
                CodegenTy::LocusRef(n) => n,
                _ => continue,
            };
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
            // GH #1036: step over a child that was already reclaimed
            // (see `emit_reclaimed_child_skip`).
            let skip_bb = self.emit_reclaimed_child_skip(
                &inner_info, inner_ptr, locus_name, &fname, "cascade", false,
            )?;
            // __dissolve_closures → dissolve → arena_destroy. The
            // drain step ran earlier via `emit_locus_field_drains`
            // (depth-first before outer's drain) so this teardown
            // half can assume children have already drained.
            let closures = inner_info.dissolve_closures_fn;
            let dissolve_call = inner_info
                .methods
                .get("dissolve")
                .copied()
                .filter(|_| !inner_info.empty_lifecycle.contains("dissolve"));
            self.lc_in_spine("Cascade", |cx| {
                cx.lc_step("Dissolve", Some(inner_ptr), Some(&inner_name), |cx| {
                    if let Some(closures_fn) = closures {
                        let (parent_self, handler_ptr) = cx.resolve_failure_route(&inner_name);
                        cx.builder
                            .build_call(
                                closures_fn,
                                &[inner_ptr.into(), parent_self.into(), handler_ptr.into()],
                                &format!("{}.cascade.closures", inner_name),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                    if let Some(dissolve_fn) = dissolve_call {
                        cx.builder
                            .build_call(dissolve_fn, &[inner_ptr.into()], &format!("{}.cascade.dissolve", inner_name))
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                    Ok(())
                })
            })?;
            // GH #750: descend. Inner's own locus-typed param
            // fields are parent-owned exactly as inner is — their
            // instantiation took the `parent_owns_via_field`
            // no-op branch, so nothing else ever tears them down.
            // Before this, the cascade stopped one level below the
            // locus being dissolved: a grandchild's `dissolve()`
            // body never ran, its capacity slots were never freed
            // and its arena was never destroyed. The leak hid
            // behind the chunk pool — the grandchild's arena
            // pointer lives in the child's arena chunk, which goes
            // back to `g_chunk_pool` intact, so LeakSanitizer still
            // reached it — until some later allocation recycled
            // that chunk and overwrote the last reference.
            //
            // Ordering mirrors this locus's own teardown: inner's
            // `dissolve()` body ran above and could still read its
            // fields; the grandchildren go now, and inner's arena
            // (which holds their structs) goes after them.
            if self.locus_cascade_path.iter().all(|n| n != &inner_name)
                && inner_name != locus_name
            {
                self.locus_cascade_path.push(locus_name.to_string());
                let deeper = self.emit_locus_field_dissolves(
                    &inner_info,
                    inner_ptr,
                    &inner_name,
                );
                self.locus_cascade_path.pop();
                deeper?;
            }
            // Inner's arena_destroy. Even when inner allocates
            // nothing in its arena, the slot was created at birth
            // and must be destroyed for symmetry. Its Reclaim is the
            // cascade's step, as the plan holds it, whatever frame the
            // cascade runs in. Preserve the owner hold for storage release.
            self.lc_in_spine("Cascade", |cx| {
                cx.emit_locus_arena_destroy_owned(&inner_info, inner_ptr, &inner_name, Some(self_ptr))
            })?;
            self.builder
                .build_unconditional_branch(skip_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(skip_bb);
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
        self.emit_reclaim_scope_leave(retain)?;
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

    /// GH #871: tear down the owned child behind a contract-typed
    /// param field.
    ///
    /// The `LocusRef` arm of the cascade can emit a static
    /// `__dissolve` / `__reclaim` chain because the field's declared
    /// type IS the child's type. An `interface` slot and a
    /// `perspective(P)` handle name a contract instead, and which
    /// locus satisfies it is an instantiation-site decision
    /// (`Gateway { router: RouterV2 { } }` over a `= RouterV1 { }`
    /// default). The cascade, though, is emitted once per OWNER
    /// type — inside `__reclaim_<Owner>`, inside the deferred-frame
    /// teardown, inside every eager-dissolve site. So the
    /// instantiation records the child's teardown pair
    /// (`{ __drain_<Impl>, __reclaim_drained_<Impl> }`) in
    /// `__owned_child_reclaim_<f>` and this emits the indirect call
    /// to one half.
    ///
    /// Three guards, all runtime: the F.29 owned-bit (a child handed
    /// in from outside belongs to its own owner and is left alone),
    /// a null pair (the field was never initialized from a literal on
    /// this path) and a null child pointer. Both halves are idempotent
    /// — each latches on a NULL `__arena` — so a second pass over the
    /// same field is a no-op rather than a double free.
    ///
    /// Ordering (line 12, C32): the child's teardown splits in two as
    /// a `LocusRef` field's does, its drain in the owner's drain
    /// cascade, before the owner's own drain, and the rest after the
    /// owner's dissolve body. Its whole spine used to run at the
    /// second point, drain included.
    fn emit_owned_contract_child_teardown(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        fname: &str,
        field_idx: u32,
        via_fat_pointer: bool,
        half: u64,
    ) -> Result<(), CodegenError> {
        let reclaim_slot_idx =
            match info.owned_child_reclaim_field_idxs.get(fname) {
                Some(&i) => i,
                // No slot means the field can't hold an owned child
                // (declare_locus_struct allots one per interface /
                // perspective param). Nothing to do.
                None => return Ok(()),
            };
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let func = self.current_fn.expect("current_fn set");
        // F.29 ownership gate, same as the LocusRef arm.
        let (owned_then_bb, owned_after_bb) = self
            .emit_locus_field_owned_branch(
                info,
                self_ptr,
                locus_name,
                fname,
                "cascade.contract",
            )?;
        let owned_after_bb = match (owned_then_bb, owned_after_bb) {
            (Some(then_bb), Some(after_bb)) => {
                self.builder.position_at_end(then_bb);
                Some(after_bb)
            }
            // Unreachable in practice: the slot and the bit are
            // allotted by the same filter. Emitting the body
            // ungated would be wrong, so bail instead.
            _ => return Ok(()),
        };
        let reclaim_ptr_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                reclaim_slot_idx,
                &format!("{}.{}.contract.reclaim.ptr", locus_name, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let pair = self
            .builder
            .build_load(
                ptr_t,
                reclaim_ptr_slot,
                &format!("{}.{}.contract.reclaim", locus_name, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let held_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                field_idx,
                &format!("{}.{}.contract.gep", locus_name, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let held = self
            .builder
            .build_load(
                ptr_t,
                held_slot,
                &format!("{}.{}.contract.load", locus_name, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let fp_null = self
            .builder
            .build_is_null(
                pair,
                &format!("{}.{}.contract.fp.null", locus_name, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let held_null = self
            .builder
            .build_is_null(
                held,
                &format!("{}.{}.contract.held.null", locus_name, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let skip = self
            .builder
            .build_or(
                fp_null,
                held_null,
                &format!("{}.{}.contract.skip", locus_name, fname),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let do_bb = self.context.append_basic_block(
            func,
            &format!("cascade.contract.{}.do", fname),
        );
        let done_bb = self.context.append_basic_block(
            func,
            &format!("cascade.contract.{}.done", fname),
        );
        self.builder
            .build_conditional_branch(skip, done_bb, do_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(do_bb);
        // An interface field holds a `{data, vtable}` fat pointer
        // (allocated in THIS locus's arena, which is destroyed after
        // this cascade, so the read is in bounds); `data` is the
        // impl's self pointer. A perspective field holds that self
        // pointer directly — dispatch goes through the program-global
        // slot, so the field is there for ownership alone.
        let child = if via_fat_pointer {
            let fat_ty = self.iface_fat_struct_ty();
            let data_gep = self
                .builder
                .build_struct_gep(
                    fat_ty,
                    held,
                    0,
                    &format!("{}.{}.contract.data.gep", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_load(
                    ptr_t,
                    data_gep,
                    &format!("{}.{}.contract.data", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value()
        } else {
            held
        };
        let i32_t = self.context.i32_type();
        let half_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                ptr_t.array_type(2),
                pair,
                &[i32_t.const_zero(), i32_t.const_int(half, false)],
                &format!("{}.{}.contract.half.ptr", locus_name, fname),
            )
        }
        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let half_fp = self
            .builder
            .build_load(ptr_t, half_ptr, &format!("{}.{}.contract.half", locus_name, fname))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let reclaim_ty =
            self.context.void_type().fn_type(&[ptr_t.into()], false);
        let call = if half == CONTRACT_DRAIN { "drain" } else { "reclaim" };
        if half == CONTRACT_REST {
            let request = self.module.get_function("lotus_reclaim_request").expect("reclaim request declared");
            self.builder.build_call(request, &[child.into(), self_ptr.into(), half_fp.into()],
                &format!("{}.{}.contract.reclaim.call", locus_name, fname))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        } else {
        self.builder
            .build_indirect_call(
                reclaim_ty,
                half_fp,
                &[child.into()],
                &format!("{}.{}.contract.{}.call", locus_name, fname, call),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(done_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(done_bb);
        if let Some(after_bb) = owned_after_bb {
            self.builder
                .build_unconditional_branch(after_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(after_bb);
        }
        Ok(())
    }

    /// GH #1036: branch past a child's whole per-child cascade body
    /// when it was already reclaimed. A handler that violated its
    /// closure (reclaimed by its `__hwrap_*` wrapper) or a
    /// `terminate` tore down the child's fields, closures and arena
    /// and left its arena-destroy latch (`__arena`, slot 0) null. Its
    /// struct lives in the OWNER's arena, so the latch is still
    /// readable from the owner's cascade; everything below it is not
    /// (the child's own children lived in its freed arena). Testing
    /// the latch only at the child's own arena destroy, as the
    /// dissolve walk used to, descended into the freed grandchildren
    /// first and re-ran the child's `drain()` / `dissolve()`.
    ///
    /// Emits `null ptr, pending claim, or null latch -> skip`. Only
    /// the winning shared spine's own storage step bypasses its claim.
    /// Leaves the builder in the live block and returns the skip block:
    /// the caller emits the per-child body and continues there.
    fn emit_reclaimed_child_skip(
        &mut self,
        inner_info: &LocusInfo<'ctx>,
        inner_ptr: PointerValue<'ctx>,
        locus_name: &str,
        fname: &str,
        tag: &str,
        owns_claim: bool,
    ) -> Result<inkwell::basic_block::BasicBlock<'ctx>, CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let func = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .ok_or_else(|| {
                CodegenError::Unsupported(
                    "cascade outside a function".to_string(),
                )
            })?;
        let name = |what: &str| format!("{}.{}.{}.{}", locus_name, fname, tag, what);
        let nonnull_bb = self.context.append_basic_block(func, &name("nonnull"));
        let live_bb = self.context.append_basic_block(func, &name("live"));
        let skip_bb = self.context.append_basic_block(func, &name("reclaimed"));
        let is_null = self
            .builder
            .build_is_null(inner_ptr, &name("null"))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_null, skip_bb, nonnull_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(nonnull_bb);
        // The winning shared spine must finish its own storage release.
        // A null claim pointer bypasses only that claim, preserving the
        // current thread's queued/active retirement checks.
        let claim_ptr = if owns_claim {
            ptr_t.const_null()
        } else {
            self.builder.build_struct_gep(
                inner_info.struct_ty, inner_ptr, inner_info.reclaim_claimed_field_idx, &name("claim.ptr"),
            ).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let pending = self.module.get_function("lotus_reclaim_pending").expect("pending declared");
        let pending = self.builder.build_call(pending, &[inner_ptr.into(), claim_ptr.into()], "reclaim.pending")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?.try_as_basic_value().left().expect("i64").into_int_value();
        let pending = self.builder.build_int_compare(inkwell::IntPredicate::NE, pending, self.context.i64_type().const_zero(), "reclaim.retired")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let unclaimed_bb = self.context.append_basic_block(func, &name("unclaimed"));
        self.builder.build_conditional_branch(pending, skip_bb, unclaimed_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(unclaimed_bb);
        let latch_ptr = self
            .builder
            .build_struct_gep(inner_info.struct_ty, inner_ptr, 0, &name("latch.ptr"))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let latch = self
            .builder
            .build_load(ptr_t, latch_ptr, &name("latch"))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let reclaimed = self
            .builder
            .build_is_null(latch, &name("done"))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(reclaimed, skip_bb, live_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(live_bb);
        Ok(skip_bb)
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
        let is_main_locus = self.is_lowering_root(locus_name);
        // The fields in the order the plan chains their drains (line 12:
        // declaration order), each drained after its own fields.
        let field_entries = self.cascade_field_entries(info, locus_name)?;
        for (fname, field_idx, field_ty) in field_entries {
            // C32 (line 12): an owned child behind a contract-typed
            // field drains here, before its owner's drain, like every
            // owned field, through the drain half its instantiation
            // recorded; the rest of its spine runs in the dissolve
            // cascade.
            if matches!(field_ty, CodegenTy::Interface(_) | CodegenTy::Perspective(_)) {
                let via_fat_pointer = matches!(field_ty, CodegenTy::Interface(_));
                self.emit_owned_contract_child_teardown(
                    info,
                    self_ptr,
                    locus_name,
                    &fname,
                    field_idx,
                    via_fat_pointer,
                    CONTRACT_DRAIN,
                )?;
                continue;
            }
            let inner_name = match field_ty {
                CodegenTy::LocusRef(n) => n,
                _ => continue,
            };
            if is_main_locus
                && matches!(
                    self.deployment.main_placement_map.get(&fname),
                    Some(ScheduleClass::Pinned(_))
                )
            {
                // C52 (line 12): the replicas of a pinned field whose
                // join records the root keeps are joined here, as its
                // drain, in its owner's cascade; one the building frame
                // keeps is joined by that frame's flush entries.
                self.emit_instance_pinned_join(info, self_ptr, &fname, &inner_name)?;
                continue;
            }
            let inner_info = match self.user_loci.get(&inner_name).cloned() {
                Some(i) => i,
                None => continue,
            };
            let drain_fn = match inner_info.methods.get("drain") {
                Some(f) if !inner_info.empty_lifecycle.contains("drain") => {
                    Some(*f)
                }
                _ => None,
            };
            // GH #750: the drain half of the cascade descends too —
            // "drain() cascades depth-first; children first, then
            // self" holds at every level, not just the first.
            // Unlike the dissolve half (where every descendant has
            // an arena to destroy), a subtree with no `drain()`
            // anywhere below it has nothing to emit, so skip it and
            // keep the IR identical for the common shape.
            // The trace build reports every owned field's drain step,
            // an empty one included, at every level: it descends into
            // a subtree with no `drain()` too.
            let descend = inner_name != locus_name
                && self.locus_cascade_path.iter().all(|n| n != &inner_name)
                && (self.lifecycle_trace || self.locus_descendants_have_drain(&inner_name));
            if drain_fn.is_none() && !descend && !self.lifecycle_trace {
                continue;
            }
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
            // GH #1036: the drain half steps over a reclaimed child as
            // the dissolve half does — its `drain()` already ran, and
            // its fields lived in its freed arena.
            let skip_bb = self.emit_reclaimed_child_skip(
                &inner_info, inner_ptr, locus_name, &fname, "drain", false,
            )?;
            if descend {
                self.locus_cascade_path.push(locus_name.to_string());
                let deeper = self.emit_locus_field_drains(
                    &inner_info,
                    inner_ptr,
                    &inner_name,
                );
                self.locus_cascade_path.pop();
                deeper?;
            }
            self.lc_in_spine("Cascade", |cx| {
                cx.lc_step("Drain", Some(inner_ptr), Some(&inner_name), |cx| {
                    if let Some(drain_fn) = drain_fn {
                        cx.builder
                            .build_call(drain_fn, &[inner_ptr.into()], &format!("{}.cascade.drain", inner_name))
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                    Ok(())
                })
            })?;
            self.builder
                .build_unconditional_branch(skip_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(skip_bb);
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

        // Set __drain_requested = 1.
        // A failure: LATCH_FAILED, not `terminate`'s 1 (GH #1069).
        let one = i64_t.const_int(crate::LATCH_FAILED, false);
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
        self.emit_on_failure_call(
            parent_on_failure,
            parent_self,
            self_ptr,
            viol_ptr,
            true,
            "bcheck.on_failure.call",
        )?;
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
        self.emit_locus_arena_destroy_owned(info, self_ptr, locus_name, None)
    }

}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The fields the dissolve cascade walks for `locus_name`, in the
    /// order the plan places them (`LifecyclePlan::cascade_fields`: line
    /// 12's declaration order), the fields the plan names no order for in
    /// their declaration order between them. Refused where the plan
    /// places the owner's cascade steps (`LifecyclePlan::cascade_order`)
    /// in an order the walk cannot emit: the fields' drains before the
    /// owner's drain, its dissolve after, the fields' dissolves after it,
    /// its reclaim last.
    pub(crate) fn cascade_field_entries(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_name: &str,
    ) -> Result<Vec<(String, u32, CodegenTy)>, CodegenError> {
        let mut entries: Vec<(String, u32, CodegenTy)> =
            info.fields.iter().map(|(n, (idx, ty))| (n.clone(), *idx, ty.clone())).collect();
        entries.sort_by_key(|(_, idx, _)| *idx);
        if !self.cascade_orders.contains_key(locus_name) {
            let plan = self.lifecycle.ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "`{locus_name}`: the lowering view carries no lifecycle plan, and the dissolve cascade is read from it"
                ))
            })?;
            let steps = plan.cascade_order(locus_name).map_err(CodegenError::Unsupported)?;
            if steps != CASCADE_STEPS {
                return Err(CodegenError::Unsupported(format!(
                    "`{locus_name}`: the lifecycle plan orders its cascade {steps:?}, which the dissolve cascade cannot emit"
                )));
            }
            let fields = plan.cascade_fields(locus_name);
            self.cascade_orders.insert(locus_name.to_string(), fields);
        }
        let order = &self.cascade_orders[locus_name];
        // The plan's fields keep the positions they hold among the
        // entries, in the plan's order.
        let at: Vec<usize> = (0..entries.len()).filter(|&i| order.contains(&entries[i].0)).collect();
        let mut placed: Vec<(String, u32, CodegenTy)> = at.iter().map(|&i| entries[i].clone()).collect();
        placed.sort_by_key(|(n, _, _)| order.iter().position(|o| o == n));
        for (slot, e) in at.into_iter().zip(placed) {
            entries[slot] = e;
        }
        Ok(entries)
    }

    /// The order the plan places a reclaim's steps in for the
    /// declaration `locus` (`LifecyclePlan::reclaim_order`), refused
    /// where it could not be emitted: a release or the cancellation
    /// before the latch would run on every teardown of the struct, the
    /// children after the arena's release would be torn down out of a
    /// freed arena, and the struct released before its arena would lose
    /// the arena's handle.
    pub(crate) fn reclaim_spine_order(&mut self, locus: &str) -> Result<Vec<ReclaimStep>, CodegenError> {
        if let Some(order) = self.reclaim_orders.get(locus) {
            return Ok(order.clone());
        }
        let plan = self.lifecycle.ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "`{locus}`: the lowering view carries no lifecycle plan, and the reclaim spine is read from it"
            ))
        })?;
        let order = plan.reclaim_order(locus).map_err(CodegenError::Unsupported)?;
        let at = |s: ReclaimStep| order.iter().position(|&x| x == s).expect("every step is ordered");
        let refuse = |a: ReclaimStep, b: ReclaimStep| -> Result<(), CodegenError> {
            if at(a) > at(b) {
                return Err(CodegenError::Unsupported(format!(
                    "`{locus}`: the lifecycle plan places the reclaim's {} before its {}, which the reclaim cannot emit",
                    b.name(),
                    a.name()
                )));
            }
            Ok(())
        };
        refuse(ReclaimStep::Latch, ReclaimStep::CancelQueuedRuns)?;
        refuse(ReclaimStep::Latch, ReclaimStep::ReleaseArena)?;
        refuse(ReclaimStep::Latch, ReclaimStep::ReleaseStruct)?;
        refuse(ReclaimStep::Children, ReclaimStep::ReleaseArena)?;
        refuse(ReclaimStep::Children, ReclaimStep::WaitForRuns)?;
        refuse(ReclaimStep::CancelQueuedRuns, ReclaimStep::WaitForRuns)?;
        refuse(ReclaimStep::WaitForRuns, ReclaimStep::ReleaseArena)?;
        refuse(ReclaimStep::WaitForRuns, ReclaimStep::ReleaseStruct)?;
        refuse(ReclaimStep::ReleaseArena, ReclaimStep::ReleaseStruct)?;
        self.reclaim_orders.insert(locus.to_string(), order.clone());
        Ok(order)
    }

    pub(crate) fn emit_reclaim_scope_enter(
        &mut self,
        owner: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let f = self
            .module
            .get_function("lotus_reclaim_scope_enter")
            .expect("scope enter declared");
        Ok(self
            .builder
            .build_call(f, &[owner.into()], "reclaim.scope")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("ptr")
            .into_pointer_value())
    }

    pub(crate) fn emit_reclaim_scope_leave(
        &mut self,
        scope: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let f = self
            .module
            .get_function("lotus_reclaim_scope_leave")
            .expect("scope leave declared");
        self.builder
            .build_call(f, &[scope.into()], "")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Logical teardown stays at the replacement/cascade site. Only the
    /// physical release may cross a main-queue handler boundary: a started
    /// run can need a later handler before giving up its storage hold.
    fn emit_locus_arena_destroy_owned(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        owner: Option<PointerValue<'ctx>>,
    ) -> Result<(), CodegenError> {
        // Only the exact instance claimed by this generated reclaim
        // function may bypass its claim. Recursive field releases still
        // check the child's claim independently.
        let owns_claim = self.current_fn.is_some()
            && (self.current_fn == self.reclaim_fns.get(locus_name).copied()
                || self.current_fn == self.module.get_function(&format!("__reclaim_drained_{locus_name}")))
            && self.current_self.as_ref().is_some_and(|s| s.self_ptr == self_ptr);
        let skip = self.emit_reclaimed_child_skip(
            info, self_ptr, locus_name, "storage", "release", owns_claim,
        )?;
        // iris P4: LOCUS_DISSOLVE probe at THE teardown
        // chokepoint (every dissolve path funnels here). No-op
        // unless LOTUS_OBS=1.
        {
            // #328: branch-gate on `lotus_obs_live`, matching the birth
            // probe and the bus publish/deliver probes. This one was
            // the more expensive of the pair — ~2.04ns per dissolve
            // against the birth probe's ~0.85 — because the teardown
            // chokepoint sits in the middle of code LLVM would
            // otherwise optimize freely.
            let obs_live = self.obs_live_check()?;
            let current_fn = self
                .builder
                .get_insert_block()
                .and_then(|bb| bb.get_parent())
                .expect("inside a function");
            let obs_bb = self
                .context
                .append_basic_block(current_fn, "locus.obs.dissolve");
            let obs_cont_bb = self
                .context
                .append_basic_block(current_fn, "locus.obs.dissolve.cont");
            self.builder
                .build_conditional_branch(obs_live, obs_bb, obs_cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(obs_bb);

            let obs_fn = self
                .module
                .get_function("lotus_obs_locus_dissolve")
                .expect("lotus_obs_locus_dissolve declared");
            let i64_t = self.context.i64_type();
            self.builder
                .build_call(
                    obs_fn,
                    &[self_ptr.into(), i64_t.const_zero().into()],
                    &format!("{}.obs.dissolve", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_unconditional_branch(obs_cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(obs_cont_bb);
        }
        // Logical retirement is emitted here; physical release follows
        // the remaining plan steps in a callback after the runtime's holds.
        for step in self.reclaim_spine_order(locus_name)? {
            match step {
                ReclaimStep::Children if !info.arena_elidable => {
                    self.emit_accepted_children_reclaim(info, self_ptr, locus_name)?;
                }
                ReclaimStep::Latch => {
                    self.emit_drain_observer_count(locus_name, -1)?;
                    self.lc_in_spine_event("Reclaim", "Entered", self_ptr, locus_name)?;
                }
                ReclaimStep::CancelQueuedRuns => {
                    let cancel = self.module.get_function("lotus_run_cancel_only").expect("cancel declared");
                    self.builder.build_call(cancel, &[self_ptr.into()], "")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    if !info.arena_elidable && self.bus_state.is_some() {
                        let unsub = self.module.get_function("lotus_bus_quarantine_self").expect("quarantine declared");
                        self.builder.build_call(unsub, &[self_ptr.into()], &format!("{locus_name}.bus.deregister.call"))
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                }
                ReclaimStep::WaitForRuns => {
                    let release = self.locus_storage_release_fn(info, locus_name)?;
                    if let Some(owner) = owner {
                        let request = self.module.get_function("lotus_reclaim_request").expect("request declared");
                        self.builder.build_call(request, &[
                            self_ptr.into(), owner.into(), release.as_global_value().as_pointer_value().into(),
                        ], "").map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    } else {
                        self.builder.build_call(release, &[self_ptr.into()], "")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                }
                ReclaimStep::Children | ReclaimStep::ReleaseArena | ReclaimStep::ReleaseStruct => {}
            }
        }
        self.builder
            .build_unconditional_branch(skip)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(skip);
        Ok(())
    }

    fn locus_storage_release_fn(
        &mut self,
        info: &LocusInfo<'ctx>,
        locus_name: &str,
    ) -> Result<inkwell::values::FunctionValue<'ctx>, CodegenError> {
        let name = format!("__release_storage_{}_{}", locus_name, self.lc_spine);
        if let Some(f) = self.module.get_function(&name) {
            return Ok(f);
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let f = self.module.add_function(
            &name,
            self.context.void_type().fn_type(&[ptr_t.into()], false),
            Some(inkwell::module::Linkage::Internal),
        );
        let saved_block = self.builder.get_insert_block();
        let saved_fn = self.current_fn.replace(f);
        let saved_di_loc = self.di_current_loc;
        let saved_di_pos = self.di_current_pos;
        let entry = self.context.append_basic_block(f, "entry");
        let live = self.context.append_basic_block(f, "live");
        let release = self.context.append_basic_block(f, "release");
        let done = self.context.append_basic_block(f, "done");
        self.builder.position_at_end(entry);
        self.di_begin_function();
        let child = f.get_first_param().expect("self").into_pointer_value();
        let arena_slot = self
            .builder
            .build_struct_gep(info.struct_ty, child, info.arena_field_idx, "arena.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let arena = self
            .builder
            .build_load(ptr_t, arena_slot, "arena")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let dead = self
            .builder
            .build_is_null(arena, "dead")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(dead, done, live)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(live);
        let owner_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                child,
                info.owner_self_field_idx,
                "owner.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let owner = self
            .builder
            .build_load(ptr_t, owner_slot, "owner")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let defer = self
            .module
            .get_function("lotus_reclaim_defer")
            .expect("defer declared");
        let deferred = self
            .builder
            .build_call(
                defer,
                &[
                    child.into(),
                    owner.into(),
                    f.as_global_value().as_pointer_value().into(),
                ],
                "deferred",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("i64")
            .into_int_value();
        let deferred = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                deferred,
                i64_t.const_zero(),
                "pending",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(deferred, done, release)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(release);
        let enter = self
            .module
            .get_function("lotus_reclaim_release_enter")
            .expect("release enter declared");
        let active = self
            .builder
            .build_call(enter, &[child.into(), owner.into()], "release.active")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("ptr");
        self.emit_locus_storage_release_now(info, child, locus_name)?;
        let leave = self
            .module
            .get_function("lotus_reclaim_release_leave")
            .expect("release leave declared");
        self.builder
            .build_call(leave, &[active.into()], "")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(done);
        self.builder
            .build_return(None)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.current_fn = saved_fn;
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }
        self.di_current_loc = saved_di_loc;
        self.di_current_pos = saved_di_pos;
        match saved_di_loc {
            Some(loc) => self.builder.set_current_debug_location(loc),
            None => self.builder.unset_current_debug_location(),
        }
        Ok(f)
    }

    fn emit_locus_storage_release_now(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arena_slot = self.builder.build_struct_gep(
            info.struct_ty, self_ptr, info.arena_field_idx, &format!("{locus_name}.__arena.ptr"),
        ).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        for step in self.reclaim_spine_order(locus_name)? {
            match step {
                ReclaimStep::WaitForRuns => {
                    // The callback is entered only after runtime retention
                    // permits release. Wait for admitted runs and complete
                    // deferred descendants before freeing any owned storage.
                    self.emit_run_cancel_queued(self_ptr, locus_name)?;
                    let flush = self.module.get_function("lotus_reclaim_flush_owned").expect("flush declared");
                    self.builder.build_call(flush, &[self_ptr.into()], "")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                ReclaimStep::ReleaseArena if !info.arena_elidable => {
                    let arena = self.builder.build_load(ptr_t, arena_slot, &format!("{locus_name}.__arena"))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?.into_pointer_value();
                    self.emit_reclaim_release_arena(info, self_ptr, locus_name, arena)?;
                }
                ReclaimStep::ReleaseStruct => {
                    let tag = if info.arena_elidable { "elide" } else { "release" };
                    self.emit_reclaim_release_struct(info, self_ptr, locus_name, arena_slot, tag)?;
                }
                ReclaimStep::Children | ReclaimStep::Latch | ReclaimStep::CancelQueuedRuns | ReclaimStep::ReleaseArena => {}
            }
        }
        self.lc_in_spine_event("Reclaim", "Completed", self_ptr, locus_name)?;
        Ok(())
    }

    fn emit_reclaim_release_arena(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        arena: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
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
                Some(SlotForm::LruCache) => {
                    // v1.x-FORM-6: free the LRU cache's malloc'd
                    // slot table. The lotus_lru_t header itself is
                    // inline in the locus and dies with the arena.
                    let destroy_fn = self
                        .module
                        .get_function("lotus_lru_free")
                        .expect("lotus_lru_free extern declared");
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
            .build_load(i64_t, release_kind_ptr, &format!("{}.__recpool_release_kind.dissolve", locus_name))
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
            .build_load(ptr_t, release_pool_ptr_field, &format!("{}.__recpool_release_pool.dissolve", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let func = self.current_fn.ok_or_else(|| {
            CodegenError::Unsupported("arena release emit requires a current fn context".into())
        })?;
        let regular_bb = self.context.append_basic_block(func, &format!("{}.arena.destroy.regular", locus_name));
        let recpool_dispatch_bb = self.context.append_basic_block(func, &format!("{}.arena.destroy.recpool", locus_name));
        let fixed_bb = self.context.append_basic_block(func, &format!("{}.arena.destroy.fixed", locus_name));
        let slab_bb = self.context.append_basic_block(func, &format!("{}.arena.destroy.slab", locus_name));
        let released_bb = self.context.append_basic_block(func, &format!("{}.arena.destroy.release_struct", locus_name));
        let is_zero = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                release_kind,
                i64_t.const_int(0, false),
                &format!("{}.release_kind.is_zero", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_zero, regular_bb, recpool_dispatch_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Regular arena destroy.
        self.builder.position_at_end(regular_bb);
        let destroy = self.module.get_function("lotus_arena_destroy").expect("lotus_arena_destroy declared");
        self.builder
            .build_call(destroy, &[arena.into()], &format!("{}.arena.destroy", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(released_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Recpool dispatch: kind 1 → fixed, else (kind 2) → slab.
        self.builder.position_at_end(recpool_dispatch_bb);
        let is_fixed = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                release_kind,
                i64_t.const_int(1, false),
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
            .build_unconditional_branch(released_bb)
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
            .build_unconditional_branch(released_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(released_bb);
        Ok(())
    }

    fn emit_reclaim_release_struct(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        arena_field_ptr: PointerValue<'ctx>,
        tag: &str,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        self.builder
            .build_store(arena_field_ptr, ptr_t.const_null())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let owner_self_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.owner_self_field_idx,
                &format!("{}.__owner_self.{}.ptr", locus_name, tag),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let owner_self_val = self
            .builder
            .build_load(ptr_t, owner_self_slot, &format!("{}.__owner_self.{}", locus_name, tag))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let release_fn = self
            .module
            .get_function("lotus_child_struct_release")
            .expect("lotus_child_struct_release declared");
        let struct_size = info.struct_ty.size_of().expect("locus struct ty has known size");
        let name = if tag == "release" {
            format!("{}.child_struct.release", locus_name)
        } else {
            format!("{}.child_struct.release.{}", locus_name, tag)
        };
        self.builder
            .build_call(release_fn, &[owner_self_val.into(), self_ptr.into(), struct_size.into()], &name)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Decision line 19: the Reclaim bracket begins by canceling the
    /// runs still queued for the instance, on whatever pool, before its
    /// arena or struct is released, so a queued run finds the child
    /// whole or its ticket canceled. One call per reclaim path, past
    /// the `__arena` latch.
    fn emit_run_cancel_queued(
        &mut self,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let cancel_fn = self
            .module
            .get_function("lotus_run_cancel_queued")
            .expect("lotus_run_cancel_queued declared");
        self.builder
            .build_call(cancel_fn, &[self_ptr.into()], &format!("{}.runs.cancel", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// GH #750: does any locus strictly BELOW `name` in the
    /// parent-owned param-field tree declare a non-empty `drain()`?
    ///
    /// The drain half of the teardown cascade descends into a
    /// child's own locus-typed fields, but a subtree with no drain
    /// anywhere below it has nothing to emit — descending into it
    /// would add an ownership-gate branch and a field load per
    /// level for no calls. Answering this first keeps the emitted
    /// IR identical for the (dominant) drain-less shape.
    ///
    /// The walk is over locus TYPES, so a type reachable from
    /// itself would recur forever; `seen` bounds it. Skipping an
    /// already-seen type is sound for the question asked: its own
    /// drain, and its descendants', were already accounted for the
    /// first time it was reached.
    pub(crate) fn locus_descendants_have_drain(&self, name: &str) -> bool {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert(name.to_string());
        self.locus_descendants_have_drain_walk(name, &mut seen)
    }

    fn locus_descendants_have_drain_walk(
        &self,
        name: &str,
        seen: &mut BTreeSet<String>,
    ) -> bool {
        let children: Vec<String> = match self.user_loci.get(name) {
            Some(info) => info
                .fields
                .values()
                .filter_map(|(_, ty)| match ty {
                    CodegenTy::LocusRef(n) => Some(n.clone()),
                    _ => None,
                })
                .collect(),
            None => return false,
        };
        for child in children {
            if !seen.insert(child.clone()) {
                continue;
            }
            let declares_drain =
                self.user_loci.get(&child).map_or(false, |i| {
                    i.methods.contains_key("drain")
                        && !i.empty_lifecycle.contains("drain")
                });
            if declares_drain
                || self.locus_descendants_have_drain_walk(&child, seen)
            {
                return true;
            }
        }
        false
    }
}
