//! The birth spine, read from the lifecycle plan (F.40 phase 3, L4).
//!
//! An instantiation's steps from params settle to run start are the
//! plan's ordered obligations for its declaration
//! (`hale_types::lifecycle::spine`): `accept(c)` (line 5), subscription
//! registration and readiness (line 6), `birth()` with its birth-epoch
//! closures and `birth_check` (line 8), and the run's start. The
//! instantiation's emitter asks [`Cx::birth_spine_order`] for the order
//! of the steps it owes and emits each where the plan places it; a
//! pinned locus's thread function emits the steps the plan gives its
//! thread (birth, its checks, readiness, run) in the same order. The
//! params bracket settles where the params loop ends, the position the
//! plan's `ParamsSettle` row has: settled once the last field is stored.

use hale_syntax::ast::{BirthCheckDecl, LocusMember, TopDecl};
use hale_types::lifecycle::ObligationKind;
use inkwell::values::{FunctionValue, PointerValue};
use inkwell::AddressSpace;

use crate::bus::runtime::BusRuntime;
use crate::codegen::{CodegenError, Cx, LocusInfo, Scope, SelfCx};
use crate::locus::dissolve::LocusDissolve;

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The order the plan places the birth-spine steps `kinds` in for
    /// the declaration `locus`.
    pub(crate) fn birth_spine_order(
        &self,
        locus: &str,
        kinds: &[ObligationKind],
    ) -> Result<Vec<ObligationKind>, CodegenError> {
        self.lifecycle.birth_order(locus, kinds).map_err(CodegenError::Unsupported)
    }

    /// Whether the plan owes an instance of `locus` readiness (line 6):
    /// delivery to it eligible once its `birth()` has completed. Total: no
    /// template of `locus` in the plan means none of its spines owes
    /// readiness.
    pub(crate) fn owes_readiness(&self, locus: &str) -> bool {
        let p = self.lifecycle;
        p.templates(locus).any(|s| p.birth_spine(s).iter().any(|st| st.kind == ObligationKind::Readiness))
    }

    /// Open `self_ptr`'s readiness window before its first
    /// registration: what is published to it waits for
    /// [`Cx::emit_readiness`].
    pub(crate) fn emit_hold_delivery(&mut self, self_ptr: PointerValue<'ctx>, on_birth_thread: bool) -> Result<(), CodegenError> {
        let f = self.bus_ready_fn("lotus_bus_hold_delivery");
        self.builder
            .build_call(f, &[self_ptr.into(), self.context.i32_type().const_int(u64::from(on_birth_thread), false).into()], "readiness.hold")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// The Readiness step: delivery to `self_ptr` is eligible from
    /// here, and what was published to it during its birth goes out in
    /// order.
    pub(crate) fn emit_readiness(
        &mut self,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        spine: &'static str,
    ) -> Result<(), CodegenError> {
        let f = self.bus_ready_fn("lotus_bus_ready");
        self.lc_in_spine(spine, |cx| {
            cx.lc_step("Readiness", Some(self_ptr), Some(locus_name), |cx| {
                cx.builder
                    .build_call(f, &[self_ptr.into()], "readiness.ready")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(())
            })
        })
    }

    /// Line 6 at a baked direct publish: while any subscriber's readiness
    /// window is open, the publish goes through
    /// `lotus_bus_dispatch_static_direct`, whose gate parks a cell for a
    /// subscriber not yet born; otherwise the baked call runs. Leaves the
    /// builder at the baked path's start and returns the block both
    /// paths join at.
    pub(crate) fn emit_direct_publish_readiness_guard(
        &mut self,
        func: FunctionValue<'ctx>,
        id: inkwell::values::IntValue<'ctx>,
        subject: inkwell::values::BasicValueEnum<'ctx>,
        payload: inkwell::values::BasicValueEnum<'ctx>,
        size: inkwell::values::IntValue<'ctx>,
    ) -> Result<inkwell::basic_block::BasicBlock<'ctx>, CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let held = self.emit_any_subscriber_unready()?;
        let held_bb = self.context.append_basic_block(func, "bus.direct.unready.helper");
        let baked_bb = self.context.append_basic_block(func, "bus.direct.ready");
        let join_bb = self.context.append_basic_block(func, "bus.direct.join");
        self.builder.build_conditional_branch(held, held_bb, baked_bb).map_err(e)?;
        self.builder.position_at_end(held_bb);
        let helper = self
            .module
            .get_function("lotus_bus_dispatch_static_direct")
            .expect("lotus_bus_dispatch_static_direct declared in declare_builtins");
        self.builder
            .build_call(helper, &[id.into(), subject.into(), payload.into(), size.into()], "bus.direct.unready.call")
            .map_err(e)?;
        self.builder.build_unconditional_branch(join_bb).map_err(e)?;
        self.builder.position_at_end(baked_bb);
        Ok(join_bb)
    }

    /// All optimized publish paths consult the same readiness windows.
    pub(crate) fn emit_any_subscriber_unready(
        &mut self,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let i64_t = self.context.i64_type();
        let g = match self.module.get_global("lotus_bus_unready_count") {
            Some(g) => g,
            None => self.module.add_global(i64_t, None, "lotus_bus_unready_count"),
        };
        let n = self.builder.build_load(i64_t, g.as_pointer_value(), "bus.direct.unready").map_err(e)?.into_int_value();
        if let Some(inst) = n.as_instruction() {
            inst.set_alignment(8).map_err(|m| CodegenError::LlvmEmit(m.to_string()))?;
            inst.set_atomic_ordering(inkwell::AtomicOrdering::Monotonic)
                .map_err(|m| CodegenError::LlvmEmit(m.to_string()))?;
        }
        let held = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, n, i64_t.const_zero(), "bus.direct.held")
            .map_err(e)?;
        Ok(held)
    }

    /// `lotus_bus_hold_delivery` / `lotus_bus_ready`, declared on first
    /// use so a program that owes no readiness declares neither.
    fn bus_ready_fn(&mut self, name: &str) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let args = if name == "lotus_bus_hold_delivery" {
            vec![ptr_t.into(), self.context.i32_type().into()]
        } else {
            vec![ptr_t.into()]
        };
        let ty = self.context.void_type().fn_type(&args, false);
        self.module.add_function(name, ty, None)
    }

    /// m28b: a pinned locus registers every
    /// subscription with that mailbox, so bus dispatch routes cells there
    /// instead of to the global queue. Its Subscribe step, on the
    /// instantiating thread, after its thread initializes params.
    pub(crate) fn emit_pinned_mailbox_registration(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
        owned_beyond_scope: bool,
    ) -> Result<Option<PointerValue<'ctx>>, CodegenError> {
        let Some(mb_idx) = info.mailbox_field_idx else { return Ok(None) };
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let mb_slot = self.builder
            .build_struct_gep(info.struct_ty, self_ptr, mb_idx, "pinned.mailbox.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let mb_ptr = self.builder
            .build_load(ptr_t, mb_slot, "pinned.mailbox")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        // Phase 3 same setup as the cooperative path — set current_self
        // for the key-filter EXPR's `self.X` reads (and clear
        // in_params_default: member text, not default text — finding 4).
        let prev_self_pinned = self.current_self.clone();
        let prev_ipd_pinned = std::mem::replace(&mut self.in_params_default, false);
        self.current_self = Some(SelfCx {
            locus_name: locus_name.to_string(),
            struct_ty: info.struct_ty,
            self_ptr,
            fields: info.fields.clone(),
        });
        for (subject, handler_name, payload_type, key_filter) in &info.subscriptions {
            let handler_fn = info.user_methods.get(handler_name).copied().ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "locus `{}` subscribes to `{}` with handler `{}` but no such method declared",
                    locus_name, subject, handler_name
                ))
            })?;
            self.emit_bus_register(
                subject,
                self_ptr,
                handler_fn,
                Some(mb_ptr),
                payload_type,
                key_filter.as_ref(),
                owned_beyond_scope,
            )?;
        }
        self.in_params_default = prev_ipd_pinned;
        self.current_self = prev_self_pinned;
        Ok(Some(mb_ptr))
    }

    /// The declaration's `birth_check` clauses.
    pub(crate) fn birth_check_decls(&self, locus_name: &str) -> Vec<BirthCheckDecl> {
        // GH #884: module nesting flattened — the locus being
        // instantiated may be declared inside a `module { }`.
        hale_syntax::ast::flat_decls(&self.program.items)
            .find_map(|item| match item {
                TopDecl::Locus(l) if l.name.name == locus_name => Some(
                    l.members
                        .iter()
                        .filter_map(|m| match m {
                            LocusMember::BirthCheck(bc) => Some(bc.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// F.27 v2 (2026-05-20): each `birth_check { COND } -> violate NAME;`
    /// clause, evaluated after `birth()` and the birth-epoch closures, at
    /// the point where every field has its declared post-birth value. The
    /// violate routing (set `__drain_requested`, call the route bound at
    /// birth or exit) is emitted inline, branching to a continuation
    /// rather than returning from the function that holds the literal: an
    /// absorbed violation leaves it running after the instantiation.
    pub(crate) fn emit_birth_checks(
        &mut self,
        checks: &[BirthCheckDecl],
        self_ptr: PointerValue<'ctx>,
        info: &LocusInfo<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        if checks.is_empty() {
            return Ok(());
        }
        // current_self is the new locus, so the conds' `self.X` reads
        // resolve against its fields (in_params_default cleared: the
        // conds are member text — finding 4).
        let prev_self = self.current_self.clone();
        let prev_ipd_bc = std::mem::replace(&mut self.in_params_default, false);
        self.current_self = Some(SelfCx {
            locus_name: locus_name.to_string(),
            struct_ty: info.struct_ty,
            self_ptr,
            fields: info.fields.clone(),
        });
        let mut scope = Scope::default();
        let mut out = Ok(());
        for bc in checks {
            out = self.emit_birth_check(bc, self_ptr, info, locus_name, &mut scope);
            if out.is_err() {
                break;
            }
        }
        self.in_params_default = prev_ipd_bc;
        self.current_self = prev_self;
        out
    }

    /// The birth checks on a pinned locus's thread function (C38, L4):
    /// emitted inside `thread_main`, so the context that names the
    /// enclosing function's frame (its arena override, its method
    /// scratch) is set aside while they are, and the violation is
    /// allocated in the locus's own arena (`current_arena_ptr` reads
    /// `current_self`'s before any user fn's).
    pub(crate) fn emit_pinned_birth_checks(
        &mut self,
        checks: &[BirthCheckDecl],
        info: &LocusInfo<'ctx>,
        thread_self: PointerValue<'ctx>,
        thread_main: FunctionValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        if checks.is_empty() {
            return Ok(());
        }
        let prev_fn = self.current_fn.replace(thread_main);
        let prev_override = self.current_arena_override.take();
        let prev_scratch = self.current_method_scratch.take();
        let prev_method_caller = self.current_method_caller_arena.take();
        let out = self.emit_birth_checks(checks, thread_self, info, locus_name);
        self.current_fn = prev_fn;
        self.current_arena_override = prev_override;
        self.current_method_scratch = prev_scratch;
        self.current_method_caller_arena = prev_method_caller;
        out
    }

    /// On a pinned locus's thread, between its birth checks and its run
    /// (C38, L4): a check's failure its owner is still holding (the
    /// owner's params open on the instantiating thread) is waited for, so
    /// the decision is the handler's; a restart the handler asked for
    /// takes effect before `run()`. The same gate the instantiating
    /// thread emits before a cooperative child's run, with nothing to
    /// resume at settle: this thread never holds its owner open.
    pub(crate) fn emit_pinned_birth_gate(
        &mut self,
        info: &LocusInfo<'ctx>,
        thread_self: PointerValue<'ctx>,
        restart: FunctionValue<'ctx>,
        pre: inkwell::values::IntValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let func = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("inside the pinned thread function");
        let _ = self.emit_failure_await(thread_self, None, 1, pre)?;
        let req = self.emit_restart_requested(info, thread_self, pre)?;
        let restart_bb = self.context.append_basic_block(func, "pinned.gate.restart");
        let go_bb = self.context.append_basic_block(func, "pinned.gate.go");
        self.builder.build_conditional_branch(req, restart_bb, go_bb).map_err(e)?;
        self.builder.position_at_end(restart_bb);
        self.emit_restart_call(restart, thread_self, locus_name, "PinnedMain", "pinned.gate.restart.call")?;
        self.builder.build_unconditional_branch(go_bb).map_err(e)?;
        self.builder.position_at_end(go_bb);
        Ok(())
    }
}
