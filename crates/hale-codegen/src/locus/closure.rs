//! Closure-check codegen: emits the body of the synthesized
//! `__birth_closures` / `__tick_closures` / `__duration_closures` /
//! `__explicit_closures` / `__dissolve_closures` functions per
//! locus. Walks each closure's `left == right within tolerance`
//! assertion against the substituted accumulator slots and routes
//! violations to `on_failure` if declared (else dprintf + exit).
//! Round 4e of the codegen model-org refactor.

use hale_syntax::ast::{ClosureAssertion, EpochSpec};
use inkwell::values::PointerValue;
use inkwell::AddressSpace;

use crate::codegen::{
    i128_const, AccumulatorCtx, CodegenError, CodegenTy, Cx, DefaultInit,
    Scope,
};

pub(crate) trait LocusClosure<'ctx> {
    fn lower_closure_check(
        &mut self,
        locus_name: &str,
        closure_name: &str,
        ass: &ClosureAssertion,
        parent_self_or_null: PointerValue<'ctx>,
        on_failure_or_null: PointerValue<'ctx>,
        epoch: EpochSpec,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> LocusClosure<'ctx> for Cx<'ctx, 'p> {
    /// Operand types must match each other AND the tolerance type.
    /// v0 supports Int / Duration / Float / Decimal closures.
    /// String / Bool / record-typed closures are rejected (would
    /// need a domain-specific approx-equal operator anyway).
    fn lower_closure_check(
        &mut self,
        locus_name: &str,
        closure_name: &str,
        ass: &ClosureAssertion,
        parent_self_or_null: PointerValue<'ctx>,
        on_failure_or_null: PointerValue<'ctx>,
        epoch: EpochSpec,
    ) -> Result<(), CodegenError> {
        let scope = Scope::default();

        // m46: install the accumulator-substitution context for
        // this closure (if it has any sum() slots), then sample-
        // update each slot before evaluating the assertion. The
        // assertion's `sum(...)` references will then load the
        // post-update value, so each fire's "running total" is
        // the natural reading of "sum across cells through this
        // moment."
        let info = self
            .user_loci
            .get(locus_name)
            .cloned()
            .expect("closure check on unknown locus");
        let cs = self
            .current_self
            .clone()
            .expect("lower_closure_check called outside a locus body");
        let slots = info
            .accumulators_per_closure
            .get(closure_name)
            .cloned()
            .unwrap_or_default();
        if !slots.is_empty() {
            for (i, slot) in slots.iter().enumerate() {
                self.update_accumulator_slot(
                    info.struct_ty,
                    cs.self_ptr,
                    slot,
                    &scope,
                    &format!(
                        "{}.{}.acc[{}].sample",
                        locus_name, closure_name, i
                    ),
                )?;
            }
            self.accumulator_ctx = Some(AccumulatorCtx {
                slots: slots.clone(),
                next_idx: 0,
                self_ptr: cs.self_ptr,
                struct_ty: info.struct_ty,
            });
        }

        let (lv, lt) = self.lower_expr(&ass.left, &scope)?;
        let (rv, rt) = self.lower_expr(&ass.right, &scope)?;
        if lt != rt {
            self.accumulator_ctx = None;
            return Err(CodegenError::Unsupported(format!(
                "closure `{}` on `{}`: left/right types differ ({:?} vs {:?})",
                closure_name, locus_name, lt, rt
            )));
        }
        let (tv, tt) = self.lower_expr(&ass.tolerance, &scope)?;
        if tt != lt {
            self.accumulator_ctx = None;
            return Err(CodegenError::Unsupported(format!(
                "closure `{}` on `{}`: tolerance type differs ({:?} vs operand {:?})",
                closure_name, locus_name, tt, lt
            )));
        }
        // Substitution complete; clear ctx so any later expression
        // lowering on this thread doesn't accidentally hit it.
        self.accumulator_ctx = None;

        // Track the signed-i64 diff for Int/Duration closures so
        // we can populate ClosureViolation.diff at routing time.
        // For Float/Decimal closures, diff is f64 and we store 0
        // in the violation (the interpreter exposes a polymorphic
        // diff there, which v0 codegen's static struct can't
        // express).
        let mut int_diff: Option<inkwell::values::IntValue<'ctx>> = None;

        let pass = match &lt {
            CodegenTy::Int | CodegenTy::Duration | CodegenTy::Decimal => {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let t = tv.into_int_value();
                let diff = self
                    .builder
                    .build_int_sub(l, r, "diff")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                int_diff = Some(diff);
                let zero: inkwell::values::IntValue<'ctx> =
                    if matches!(lt, CodegenTy::Decimal) {
                        i128_const(self.context, 0)
                    } else {
                        self.context.i64_type().const_int(0, false)
                    };
                let neg = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLT,
                        diff,
                        zero,
                        "diff.neg",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let neg_diff = self
                    .builder
                    .build_int_sub(zero, diff, "diff.neg.val")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let abs = self
                    .builder
                    .build_select(neg, neg_diff, diff, "abs")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLE,
                        abs,
                        t,
                        "closure.pass",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            CodegenTy::Float => {
                let l = lv.into_float_value();
                let r = rv.into_float_value();
                let t = tv.into_float_value();
                let diff = self
                    .builder
                    .build_float_sub(l, r, "fdiff")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let neg_diff = self
                    .builder
                    .build_float_neg(diff, "fdiff.neg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let zero = self.context.f64_type().const_float(0.0);
                let is_neg = self
                    .builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OLT,
                        diff,
                        zero,
                        "fdiff.neg",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let abs = self
                    .builder
                    .build_select(is_neg, neg_diff, diff, "fabs")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_float_value();
                self.builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OLE,
                        abs,
                        t,
                        "closure.pass",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "closure `{}` on `{}`: ~~ not defined for {:?}",
                    closure_name, locus_name, other
                )));
            }
        };

        let func = self
            .current_fn
            .expect("current_fn set in __closures body");
        let cont_bb = self
            .context
            .append_basic_block(func, "closure.cont");
        let fail_bb = self
            .context
            .append_basic_block(func, "closure.fail");
        self.builder
            .build_conditional_branch(pass, cont_bb, fail_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // fail_bb: route to parent's on_failure if non-null,
        // else fall back to dprintf+exit.
        self.builder.position_at_end(fail_bb);
        let route_bb = self
            .context
            .append_basic_block(func, "closure.fail.route");
        let bare_bb = self
            .context
            .append_basic_block(func, "closure.fail.bare");
        let post_bb = self
            .context
            .append_basic_block(func, "closure.fail.post");
        let null_check = self
            .builder
            .build_is_not_null(on_failure_or_null, "has.handler")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(null_check, route_bb, bare_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // route_bb: build a ClosureViolation, call parent's
        // on_failure(parent_self, child_self, violation). If the
        // handler returns (absorb), continue; if it bubbles, the
        // bubble path inside the handler exits the program before
        // returning. Either way we just branch to post_bb.
        self.builder.position_at_end(route_bb);
        let viol_info = self
            .user_types
            .get("ClosureViolation")
            .cloned()
            .expect("ClosureViolation declared at startup");
        let size = viol_info
            .struct_ty
            .size_of()
            .expect("violation struct has known size");
        let viol_ptr = self.arena_alloc(size, "viol.alloc")?;
        let locus_str = self.global_string(locus_name);
        let closure_str = self.global_string(closure_name);
        let f0 = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                0,
                "viol.locus.ptr",
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
                "viol.closure.ptr",
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
                "viol.diff.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i64_t = self.context.i64_type();
        let diff_val = int_diff.unwrap_or_else(|| i64_t.const_int(0, false));
        // m48: Decimal closures produce an i128 diff; the violation's
        // diff field is i64 (carries the natural domain's diff for
        // Int / Duration). Truncate i128 → i64 — diff is diagnostic
        // only, never recomputed against the original mantissa, so
        // precision loss past 2^63 ns / mantissa-units is acceptable
        // for v0.1.
        let diff_val = if diff_val.get_type().get_bit_width() != 64 {
            self.builder
                .build_int_truncate(diff_val, i64_t, "diff.trunc")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        } else {
            diff_val
        };
        self.builder
            .build_store(f2, diff_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // self.current_self.self_ptr is the failing locus's self —
        // pass it as the child_self arg.
        let child_self = self
            .current_self
            .as_ref()
            .expect("__closures runs with current_self set")
            .self_ptr;
        let cs_struct_ty = self
            .current_self
            .as_ref()
            .expect("__closures runs with current_self set")
            .struct_ty;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();
        let handler_callee_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );

        // m40: birth-epoch closures snapshot the pre-call value of
        // __restart_count so we can detect whether the parent's
        // on_failure body called restart(self). If it did and the
        // count is within the cap, we re-run birth() + the entire
        // __birth_closures fn before returning (a recursive call
        // into the synthesized eval fn). Dissolve-epoch closures
        // skip this — restart isn't applicable at end-of-life.
        let info = self
            .user_loci
            .get(locus_name)
            .cloned()
            .expect("locus declared in pass A1");
        let i64_t = self.context.i64_type();
        let pre_count: Option<inkwell::values::IntValue<'ctx>> =
            if matches!(epoch, EpochSpec::Birth)
                && info.birth_closures_fn.is_some()
                && info.methods.contains_key("birth")
            {
                let rc_ptr = self
                    .builder
                    .build_struct_gep(
                        cs_struct_ty,
                        child_self,
                        info.restart_count_field_idx,
                        "restart.count.pre.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let v = self
                    .builder
                    .build_load(i64_t, rc_ptr, "restart.count.pre")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Some(v.into_int_value())
            } else {
                None
            };

        self.builder
            .build_indirect_call(
                handler_callee_ty,
                on_failure_or_null,
                &[
                    parent_self_or_null.into(),
                    child_self.into(),
                    viol_ptr.into(),
                ],
                "on_failure.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        if let Some(pre) = pre_count {
            // Post-handler restart check.
            // bumped = post > pre; under_cap = post <= 2;
            // should_rerun = bumped && under_cap.
            let rc_ptr = self
                .builder
                .build_struct_gep(
                    cs_struct_ty,
                    child_self,
                    info.restart_count_field_idx,
                    "restart.count.post.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let post = self
                .builder
                .build_load(i64_t, rc_ptr, "restart.count.post")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let bumped = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::SGT,
                    post,
                    pre,
                    "restart.bumped",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let cap = i64_t.const_int(2, false);
            let under_cap = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::SLE,
                    post,
                    cap,
                    "restart.under_cap",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let should_rerun = self
                .builder
                .build_and(bumped, under_cap, "restart.should_rerun")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let func = self
                .current_fn
                .expect("current_fn set in __birth_closures body");
            let rerun_bb = self
                .context
                .append_basic_block(func, "restart.rerun");
            self.builder
                .build_conditional_branch(should_rerun, rerun_bb, post_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // rerun_bb: m45 — gate on
            // __restart_in_place_pending. If set, branch to a
            // zero-fields pass (re-init each user field from
            // its declared default) and clear the flag before
            // call_birth_bb. Otherwise branch direct to
            // call_birth_bb. Both converge on the call_birth
            // block which fires birth + __birth_closures.
            self.builder.position_at_end(rerun_bb);
            let rip_ptr = self
                .builder
                .build_struct_gep(
                    cs_struct_ty,
                    child_self,
                    info.restart_in_place_pending_field_idx,
                    "restart_in_place.pending.load.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let rip_val = self
                .builder
                .build_load(i64_t, rip_ptr, "restart_in_place.pending")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let is_in_place = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    rip_val,
                    i64_t.const_int(0, false),
                    "restart_in_place.is_pending",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let zero_fields_bb = self.context.append_basic_block(
                func,
                "restart_in_place.zero_fields",
            );
            let call_birth_bb = self.context.append_basic_block(
                func,
                "restart.call_birth",
            );
            self.builder
                .build_conditional_branch(
                    is_in_place,
                    zero_fields_bb,
                    call_birth_bb,
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

            // zero_fields_bb: re-store each declared default
            // into its user field, then clear the in-place
            // flag so a subsequent restart() (without _in_place)
            // doesn't accidentally repeat the zero pass.
            // Composite-default literals re-allocate in this
            // locus's own arena (via current_arena_override),
            // matching the instantiation-time discipline.
            self.builder.position_at_end(zero_fields_bb);
            let arena_slot = self
                .builder
                .build_struct_gep(
                    cs_struct_ty,
                    child_self,
                    info.arena_field_idx,
                    "restart_in_place.arena.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let locus_arena = self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    arena_slot,
                    "restart_in_place.arena",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            let prev_override = self.current_arena_override;
            self.current_arena_override = Some(locus_arena);
            let scope = Scope::default();
            let defaults_snapshot = info.defaults.clone();
            for (fname, default) in &defaults_snapshot {
                let (val, _) = match default {
                    DefaultInit::Const(pv) => self.const_param(pv),
                    DefaultInit::Expr(e) => self.lower_expr(e, &scope)?,
                    DefaultInit::Required => {
                        // restart_in_place rewinds state to its
                        // birth() configuration. A required param
                        // has no resettable default — the user-
                        // supplied value at instantiation is the
                        // only state — so we leave the field's
                        // current value in place. If the user
                        // wants a different restart-time value,
                        // they need a real default.
                        continue;
                    }
                };
                let (slot_idx, _) = info
                    .fields
                    .get(fname)
                    .cloned()
                    .expect("field declared by declare_locus_struct");
                let field_slot = self
                    .builder
                    .build_struct_gep(
                        cs_struct_ty,
                        child_self,
                        slot_idx,
                        &format!("restart_in_place.{}.ptr", fname),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(field_slot, val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            self.current_arena_override = prev_override;
            // Clear the pending flag; otherwise a subsequent
            // restart() (without _in_place) would zero again.
            self.builder
                .build_store(rip_ptr, i64_t.const_int(0, false))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_unconditional_branch(call_birth_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

            // call_birth_bb: call birth(self) + recursively
            // call __birth_closures(self, parent_self,
            // on_failure), then ret void. The recursive call
            // may itself fail + restart, so the cap is
            // enforced naturally as the counter accumulates
            // across attempts.
            self.builder.position_at_end(call_birth_bb);
            let birth_fn = *info
                .methods
                .get("birth")
                .expect("birth method present");
            self.builder
                .build_call(
                    birth_fn,
                    &[child_self.into()],
                    "restart.birth.call",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let birth_closures_fn = info
                .birth_closures_fn
                .expect("birth_closures_fn present");
            self.builder
                .build_call(
                    birth_closures_fn,
                    &[
                        child_self.into(),
                        parent_self_or_null.into(),
                        on_failure_or_null.into(),
                    ],
                    "restart.birth_closures.call",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        } else {
            self.builder
                .build_unconditional_branch(post_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // bare_bb: no handler — emit the v0 fallback report and
        // exit(1).
        self.builder.position_at_end(bare_bb);
        let msg = format!(
            "ClosureViolation: locus `{}` closure `{}` failed at dissolve\n",
            locus_name, closure_name
        );
        let msg_ptr = self.global_string(&msg);
        let dprintf_fn = self
            .module
            .get_function("dprintf")
            .expect("dprintf declared in declare_builtins");
        let i32_t = self.context.i32_type();
        let stderr_fd = i32_t.const_int(2, false);
        self.builder
            .build_call(
                dprintf_fn,
                &[stderr_fd.into(), msg_ptr.into()],
                "closure.dprintf",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let exit_fn = self
            .module
            .get_function("exit")
            .expect("exit declared in declare_builtins");
        self.builder
            .build_call(
                exit_fn,
                &[i32_t.const_int(1, false).into()],
                "closure.exit",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // post_bb: parent absorbed → continue with next closure.
        self.builder.position_at_end(post_bb);
        self.builder
            .build_unconditional_branch(cont_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // cont_bb: continue with next closure (or fall off body).
        self.builder.position_at_end(cont_bb);
        Ok(())
    }

}
