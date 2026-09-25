//! GH #1066: `restart(c)` and `restart_in_place(c)` restart a child
//! from any failure, not only a birth-epoch one.
//!
//! `restart` only ever bumped the child's `__restart_count`; the one
//! place that looked was the birth-epoch closure fn, which re-runs
//! birth right after the handler returns. A failure anywhere else — a
//! `violate` in `run()`, the case a supervised connection hits — left
//! the count bumped and nothing reading it: the child's `run()` had
//! returned and never started again, while its owner believed it had
//! recovered.
//!
//! Each locus a failure can come from (one declaring a closure or a
//! `birth_check`) now gets:
//!
//! - `__restart_<L>(self)`: the restart itself — reset params to their
//!   declared defaults if `restart_in_place` asked for it, lower the
//!   drain latch the `violate` raised, re-run `birth()` and the
//!   birth-epoch closures. The caller then runs `run()` again.
//! - `__resume_<L>(self, phase, pre)`: what the child does once a HELD
//!   failure's handler has returned (spec/semantics.md § "on_failure(c,
//!   err)"), run by the runtime at settle when the child failed on the
//!   thread holding its parent open and so could not wait. Phase 0: its
//!   `run()` had returned — restart and run it again, or end as it
//!   would have. Phase 1: it had not started `run()` — restart, or
//!   start it.
//!
//! The decision is the one the birth-epoch protocol already made: the
//! handler bumped the count since `pre`, the count is within the
//! child's `__restart_bound`, the child is not quarantined — and the
//! process is not draining, since a drain would end it anyway.

use inkwell::types::StructType;
use inkwell::values::{FunctionValue, IntValue, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, Cx, DefaultInit, LocusInfo, Scope, SelfCx};

/// The per-locus restart entry points.
#[derive(Clone, Copy)]
pub(crate) struct RestartFns<'ctx> {
    pub(crate) restart: FunctionValue<'ctx>,
    pub(crate) resume: FunctionValue<'ctx>,
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Can a failure be routed from this locus — does it declare a
    /// closure (any epoch, `inline` ones included: they carry no
    /// assertion, so `LocusInfo::closures` leaves them out) or a
    /// `birth_check`? Only those pay for restart points.
    fn locus_declares_failures(&self, locus_name: &str) -> bool {
        hale_syntax::ast::flat_decls(&self.program.items).any(|item| {
            matches!(item, hale_syntax::ast::TopDecl::Locus(l)
                if l.name.name == locus_name
                    && l.members.iter().any(|m| {
                        matches!(
                            m,
                            hale_syntax::ast::LocusMember::Closure(_)
                                | hale_syntax::ast::LocusMember::BirthCheck(_)
                        )
                    }))
        })
    }

    /// Declare `__restart_<L>` / `__resume_<L>` for every locus a
    /// failure can come from. Bodies follow in
    /// [`Cx::define_restart_fns`], after user fns are declared (a
    /// param default may call one).
    pub(crate) fn declare_restart_fns(&mut self) {
        let void_t = self.context.void_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let names: Vec<String> = self.user_loci.keys().cloned().collect();
        for name in names {
            if !self.locus_declares_failures(&name) {
                continue;
            }
            let restart = self.module.add_function(
                &format!("__restart_{name}"),
                void_t.fn_type(&[ptr_t.into()], false),
                None,
            );
            let resume = self.module.add_function(
                &format!("__resume_{name}"),
                void_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false),
                None,
            );
            self.restart_fns.insert(name, RestartFns { restart, resume });
        }
    }

    /// The child's `__restart_count`, now.
    pub(crate) fn emit_restart_count(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.restart_count_field_idx,
                "restart.count.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(self
            .builder
            .build_load(self.context.i64_type(), ptr, "restart.count")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value())
    }

    /// i1: did a handler ask to restart this child since its count was
    /// `pre`, and may it? Bumped, within `__restart_bound`, not
    /// quarantined, the process not draining.
    pub(crate) fn emit_restart_requested(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        pre: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let zero = i64_t.const_zero();
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let post = self.emit_restart_count(info, self_ptr)?;
        let bumped = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SGT, post, pre, "restart.bumped")
            .map_err(e)?;
        let bound_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.restart_bound_field_idx,
                "restart.bound.ptr",
            )
            .map_err(e)?;
        let bound = self
            .builder
            .build_load(i64_t, bound_ptr, "restart.bound")
            .map_err(e)?
            .into_int_value();
        let under = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLE, post, bound, "restart.under_cap")
            .map_err(e)?;
        let q_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.quarantined_field_idx,
                "restart.quarantined.ptr",
            )
            .map_err(e)?;
        let q = self
            .builder
            .build_load(i64_t, q_ptr, "restart.quarantined")
            .map_err(e)?
            .into_int_value();
        let live = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, q, zero, "restart.not_quarantined")
            .map_err(e)?;
        let mut ok = self.builder.build_and(bumped, under, "restart.ok").map_err(e)?;
        ok = self.builder.build_and(ok, live, "restart.ok").map_err(e)?;
        if !self.is_wasm {
            let draining = self.emit_process_draining_load("restart.process_draining")?;
            let not_draining = self
                .builder
                .build_int_compare(inkwell::IntPredicate::EQ, draining, zero, "restart.not_draining")
                .map_err(e)?;
            ok = self.builder.build_and(ok, not_draining, "restart.ok").map_err(e)?;
        }
        Ok(ok)
    }

    /// GH #1069: i1 — does this child stay allocated although it asked
    /// to be reclaimed? True for a child that FAILED (its latch holds
    /// `LATCH_FAILED`, not `terminate`'s 1), is held by an owner that
    /// reclaims it later (`__held_by_owner`), and was not accepted
    /// (`__owner_self` null — an accepted child's tracker expects it
    /// gone). Such a child has stopped running, but its parent's field
    /// or its binding still names it; its memory lives until that
    /// owner's teardown, as quarantine already promises.
    pub(crate) fn emit_failed_child_is_kept(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let load = |cx: &mut Self, idx: u32, name: &str| -> Result<IntValue<'ctx>, CodegenError> {
            let p = cx
                .builder
                .build_struct_gep(info.struct_ty, self_ptr, idx, &format!("{name}.ptr"))
                .map_err(e)?;
            Ok(cx.builder.build_load(i64_t, p, name).map_err(e)?.into_int_value())
        };
        let dr = load(self, info.drain_requested_field_idx, "kept.latch")?;
        let held = load(self, info.held_by_owner_field_idx, "kept.held")?;
        let owner_ptr = self
            .builder
            .build_struct_gep(info.struct_ty, self_ptr, info.owner_self_field_idx, "kept.owner.ptr")
            .map_err(e)?;
        let owner = self
            .builder
            .build_load(ptr_t, owner_ptr, "kept.owner")
            .map_err(e)?
            .into_pointer_value();
        let failed = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                dr,
                i64_t.const_int(crate::LATCH_FAILED, false),
                "kept.failed",
            )
            .map_err(e)?;
        let is_held = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, held, i64_t.const_zero(), "kept.is_held")
            .map_err(e)?;
        let unaccepted = self.builder.build_is_null(owner, "kept.unaccepted").map_err(e)?;
        let k = self.builder.build_and(failed, is_held, "kept").map_err(e)?;
        Ok(self.builder.build_and(k, unaccepted, "kept").map_err(e)?)
    }

    /// GH #1069: if `kept`, drop the child's bus subscriptions now — a
    /// failed child stops, whether or not its memory has to wait for
    /// its owner. The same runtime walk `quarantine(c)` uses.
    pub(crate) fn emit_stop_kept_child(
        &mut self,
        kept: IntValue<'ctx>,
        self_ptr: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        if self.bus_state.is_none() {
            return Ok(());
        }
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let func = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("inside a function");
        let stop_bb = self.context.append_basic_block(func, "kept.stop");
        let cont_bb = self.context.append_basic_block(func, "kept.cont");
        self.builder.build_conditional_branch(kept, stop_bb, cont_bb).map_err(e)?;
        self.builder.position_at_end(stop_bb);
        let unsub = self
            .module
            .get_function("lotus_bus_quarantine_self")
            .expect("lotus_bus_quarantine_self declared");
        self.builder.build_call(unsub, &[self_ptr.into()], "kept.unsubscribe").map_err(e)?;
        self.builder.build_unconditional_branch(cont_bb).map_err(e)?;
        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// i64 from `lotus_failure_await(self, resume, phase, pre)` — 0
    /// nothing outstanding, 1 waited for another thread's handler, 2
    /// the runtime resumes this child at settle — behind one monotonic
    /// load of the outstanding count, so the common case is a load and
    /// an untaken branch.
    pub(crate) fn emit_failure_await(
        &mut self,
        self_ptr: PointerValue<'ctx>,
        resume: Option<FunctionValue<'ctx>>,
        phase: u64,
        pre: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let held_g = self
            .module
            .get_global("lotus_held_failure_count")
            .expect("lotus_held_failure_count declared");
        let held = self
            .builder
            .build_load(i64_t, held_g.as_pointer_value(), "await.held")
            .map_err(e)?
            .into_int_value();
        if let Some(inst) = held.as_instruction() {
            inst.set_alignment(8).map_err(|m| CodegenError::LlvmEmit(m.to_string()))?;
            inst.set_atomic_ordering(inkwell::AtomicOrdering::Monotonic)
                .map_err(|m| CodegenError::LlvmEmit(m.to_string()))?;
        }
        let any = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, held, i64_t.const_zero(), "await.any")
            .map_err(e)?;
        let func = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("inside a function");
        let from_bb = self.builder.get_insert_block().expect("inside a block");
        let ask_bb = self.context.append_basic_block(func, "await.ask");
        let join_bb = self.context.append_basic_block(func, "await.join");
        self.builder.build_conditional_branch(any, ask_bb, join_bb).map_err(e)?;
        self.builder.position_at_end(ask_bb);
        let await_fn = self
            .module
            .get_function("lotus_failure_await")
            .expect("lotus_failure_await declared");
        let resume_ptr = match resume {
            Some(f) => f.as_global_value().as_pointer_value(),
            None => ptr_t.const_null(),
        };
        let asked = self
            .builder
            .build_call(
                await_fn,
                &[
                    self_ptr.into(),
                    resume_ptr.into(),
                    i64_t.const_int(phase, false).into(),
                    pre.into(),
                ],
                "await.call",
            )
            .map_err(e)?
            .try_as_basic_value()
            .left()
            .expect("lotus_failure_await returns i64")
            .into_int_value();
        let ask_end = self.builder.get_insert_block().expect("inside a block");
        self.builder.build_unconditional_branch(join_bb).map_err(e)?;
        self.builder.position_at_end(join_bb);
        let phi = self.builder.build_phi(i64_t, "await.result").map_err(e)?;
        phi.add_incoming(&[(&i64_t.const_zero(), from_bb), (&asked, ask_end)]);
        Ok(phi.as_basic_value().into_int_value())
    }

    /// Re-store each declared default into its param field — what
    /// `restart_in_place` resets. A required param (no default) keeps
    /// its value: the one given at instantiation is the only state it
    /// has. Composite defaults allocate in the locus's own arena.
    pub(crate) fn emit_reset_params_to_defaults(
        &mut self,
        info: &LocusInfo<'ctx>,
        struct_ty: StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let arena_slot = self
            .builder
            .build_struct_gep(struct_ty, self_ptr, info.arena_field_idx, "restart_in_place.arena.ptr")
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
                DefaultInit::Required => continue,
            };
            let (slot_idx, _) = info
                .fields
                .get(fname)
                .cloned()
                .expect("field declared by declare_locus_struct");
            let field_slot = self
                .builder
                .build_struct_gep(struct_ty, self_ptr, slot_idx, &format!("restart_in_place.{}.ptr", fname))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(field_slot, val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.current_arena_override = prev_override;
        Ok(())
    }

    /// Emit the bodies of every declared `__restart_<L>` /
    /// `__resume_<L>`.
    pub(crate) fn define_restart_fns(&mut self) -> Result<(), CodegenError> {
        let saved_block = self.builder.get_insert_block();
        let entries: Vec<(String, RestartFns<'ctx>)> =
            self.restart_fns.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (name, fns) in entries {
            let info = self
                .user_loci
                .get(&name)
                .cloned()
                .expect("restart fns are declared for user loci");
            self.define_restart_fn(&name, &info, fns.restart)?;
            self.define_resume_fn(&name, &info, fns)?;
        }
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }
        Ok(())
    }

    fn define_restart_fn(
        &mut self,
        name: &str,
        info: &LocusInfo<'ctx>,
        f: FunctionValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let entry = self.context.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        self.di_begin_function();
        let self_arg = f.get_nth_param(0).expect("self param").into_pointer_value();
        let prev_fn = self.current_fn.replace(f);
        let prev_self = self.current_self.replace(SelfCx {
            locus_name: name.to_string(),
            struct_ty: info.struct_ty,
            self_ptr: self_arg,
            fields: info.fields.clone(),
        });
        let prev_ipd = std::mem::replace(&mut self.in_params_default, false);

        // restart_in_place: back to the declared defaults first.
        let rip_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_arg,
                info.restart_in_place_pending_field_idx,
                "restart.in_place.ptr",
            )
            .map_err(e)?;
        let rip = self
            .builder
            .build_load(i64_t, rip_ptr, "restart.in_place")
            .map_err(e)?
            .into_int_value();
        let in_place = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, rip, i64_t.const_zero(), "restart.is_in_place")
            .map_err(e)?;
        let reset_bb = self.context.append_basic_block(f, "restart.reset");
        let rerun_bb = self.context.append_basic_block(f, "restart.rerun");
        self.builder.build_conditional_branch(in_place, reset_bb, rerun_bb).map_err(e)?;
        self.builder.position_at_end(reset_bb);
        self.emit_reset_params_to_defaults(info, info.struct_ty, self_arg)?;
        self.builder.build_store(rip_ptr, i64_t.const_zero()).map_err(e)?;
        self.builder.build_unconditional_branch(rerun_bb).map_err(e)?;

        // The failure raised the drain latch (`violate` sets it so the
        // child stops); the restarted child is live again.
        self.builder.position_at_end(rerun_bb);
        let dr_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_arg,
                info.drain_requested_field_idx,
                "restart.drain_requested.ptr",
            )
            .map_err(e)?;
        self.builder.build_store(dr_ptr, i64_t.const_zero()).map_err(e)?;
        if let Some(birth) = info.methods.get("birth") {
            if !info.empty_lifecycle.contains("birth") {
                self.builder
                    .build_call(*birth, &[self_arg.into()], "restart.birth")
                    .map_err(e)?;
            }
        }
        if let Some(bc) = info.birth_closures_fn {
            let ps_ptr = self
                .builder
                .build_struct_gep(info.struct_ty, self_arg, info.parent_self_field_idx, "restart.parent_self.ptr")
                .map_err(e)?;
            let ps = self.builder.build_load(ptr_t, ps_ptr, "restart.parent_self").map_err(e)?;
            let h_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_arg,
                    info.parent_on_failure_field_idx,
                    "restart.parent_on_failure.ptr",
                )
                .map_err(e)?;
            let h = self.builder.build_load(ptr_t, h_ptr, "restart.parent_on_failure").map_err(e)?;
            self.builder
                .build_call(bc, &[self_arg.into(), ps.into(), h.into()], "restart.birth_closures")
                .map_err(e)?;
        }
        self.builder.build_return(None).map_err(e)?;

        self.in_params_default = prev_ipd;
        self.current_self = prev_self;
        self.current_fn = prev_fn;
        Ok(())
    }

    fn define_resume_fn(
        &mut self,
        name: &str,
        info: &LocusInfo<'ctx>,
        fns: RestartFns<'ctx>,
    ) -> Result<(), CodegenError> {
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let f = fns.resume;
        let entry = self.context.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);
        self.di_begin_function();
        let self_arg = f.get_nth_param(0).expect("self param").into_pointer_value();
        let phase = f.get_nth_param(1).expect("phase param").into_int_value();
        let pre = f.get_nth_param(2).expect("pre param").into_int_value();
        let prev_fn = self.current_fn.replace(f);

        let restart_bb = self.context.append_basic_block(f, "resume.restart");
        let carry_on_bb = self.context.append_basic_block(f, "resume.carry_on");
        let end_bb = self.context.append_basic_block(f, "resume.end");
        let run_bb = self.context.append_basic_block(f, "resume.run");
        let do_run_bb = self.context.append_basic_block(f, "resume.do_run");
        let ret_bb = self.context.append_basic_block(f, "resume.ret");

        let req = self.emit_restart_requested(info, self_arg, pre)?;
        self.builder.build_conditional_branch(req, restart_bb, carry_on_bb).map_err(e)?;

        self.builder.position_at_end(restart_bb);
        self.builder
            .build_call(fns.restart, &[self_arg.into()], "resume.restart.call")
            .map_err(e)?;
        self.builder.build_unconditional_branch(run_bb).map_err(e)?;

        // No restart. Phase 1 (run() not started yet): start it, as the
        // child would have. Phase 0 (run() had returned): end as the run
        // wrapper would have.
        self.builder.position_at_end(carry_on_bb);
        let before_run = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, phase, i64_t.const_zero(), "resume.before_run")
            .map_err(e)?;
        self.builder.build_conditional_branch(before_run, run_bb, end_bb).map_err(e)?;

        self.builder.position_at_end(end_bb);
        if let Some(run_end) = self.run_end_fns.get(name).copied() {
            self.builder
                .build_call(run_end, &[self_arg.into()], "resume.run_end")
                .map_err(e)?;
        }
        self.builder.build_unconditional_branch(ret_bb).map_err(e)?;

        // Run it — unless the handler quarantined it.
        self.builder.position_at_end(run_bb);
        let q_ptr = self
            .builder
            .build_struct_gep(info.struct_ty, self_arg, info.quarantined_field_idx, "resume.quarantined.ptr")
            .map_err(e)?;
        let q = self
            .builder
            .build_load(i64_t, q_ptr, "resume.quarantined")
            .map_err(e)?
            .into_int_value();
        let active = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, q, i64_t.const_zero(), "resume.active")
            .map_err(e)?;
        self.builder.build_conditional_branch(active, do_run_bb, ret_bb).map_err(e)?;
        self.builder.position_at_end(do_run_bb);
        if let Some(wrapper) = self.coop_pool_run_wrappers.get(name).copied() {
            self.builder
                .build_call(wrapper, &[self_arg.into(), ptr_t.const_null().into()], "resume.run")
                .map_err(e)?;
        }
        for (wrapper, tag) in [(info.tick_wrapper_fn, "tick"), (info.duration_wrapper_fn, "duration")] {
            if let Some(w) = wrapper {
                self.builder
                    .build_call(w, &[self_arg.into()], &format!("resume.{tag}"))
                    .map_err(e)?;
            }
        }
        self.builder.build_unconditional_branch(ret_bb).map_err(e)?;

        self.builder.position_at_end(ret_bb);
        self.builder.build_return(None).map_err(e)?;
        self.current_fn = prev_fn;
        Ok(())
    }
}
