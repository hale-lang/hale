//! Phase-B locus method body emission: lifecycle (birth / run /
//! dissolve / accept), user `fn` members, modes (bulk / harmonic /
//! resolution), closure-eval synthetics. Round 4c of the codegen
//! model-org refactor.

use hale_syntax::ast::{
    EpochSpec, LifecycleKind, LocusDecl, LocusMember, ModeKind,
};
use inkwell::AddressSpace;

use crate::codegen::{BlockEnd, CodegenError, CodegenTy, Cx, Scope, SelfCx};
use crate::locus::closure::LocusClosure;
use crate::stdlib::time::TimeStdlib;

pub(crate) trait LocusMethodBodies<'ctx> {
    fn lower_locus_method_bodies(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> LocusMethodBodies<'ctx> for Cx<'ctx, 'p> {
    /// Pass C: lower each declared lifecycle method body. For birth
    /// and run, the method's only arg is `self_ptr`. For accept,
    /// the second arg is the child pointer, bound as a `LocusRef`
    /// local under the param's declared name.
    fn lower_locus_method_bodies(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError> {
        if !l.generics.is_empty() {
            // m63: generic templates have no method bodies to
            // lower until pinned by an instantiation site.
            return Ok(());
        }
        let info = self
            .user_loci
            .get(&l.name.name)
            .cloned()
            .expect("locus declared in pass A");
        for member in &l.members {
            if let LocusMember::Lifecycle(lc) = member {
                let kind: &'static str = match lc.kind {
                    LifecycleKind::Birth => "birth",
                    LifecycleKind::Run => "run",
                    LifecycleKind::Accept => "accept",
                    LifecycleKind::Release => "release",
                    LifecycleKind::Drain => "drain",
                    LifecycleKind::Dissolve => "dissolve",
                };
                let func = *info
                    .methods
                    .get(kind)
                    .expect("method declared in pass A2");
                let entry = self.context.append_basic_block(func, "entry");
                self.builder.position_at_end(entry);
                let self_ptr = func
                    .get_nth_param(0)
                    .expect("self_ptr param")
                    .into_pointer_value();
                self.current_fn = Some(func);
                // F.27 extension (2026-05-19): lifecycle bodies are
                // void-returning user-fn contexts from the violate
                // codegen's perspective. Marking the ret as
                // `Some(None)` (a "user fn returning void" frame)
                // lets `violate NAME;` inside birth / drain /
                // dissolve / accept / run route through the parent's
                // on_failure handler just like a regular method
                // body would, instead of erroring at codegen with
                // `"violate" outside a user fn`. Birth-time
                // allocation failures (BytesBuilder's prior
                // birth-fail leaves-handle-zero-and-waits caveat)
                // now route at construction. The divergent return
                // is `build_return(None)` for the void shape.
                self.current_user_fn_ret = Some(None);
                self.current_self = Some(SelfCx {
                    locus_name: l.name.name.clone(),
                    struct_ty: info.struct_ty,
                    self_ptr,
                    fields: info.fields.clone(),
                });
                self.loops.clear();
                self.push_dissolve_frame();
                // Stage-1 scratch elision (lifecycle hook). Hooks return
                // void (Unit), so gate 1 passes; gate 2 keeps the scratch
                // for any hook whose body isn't provably non-allocating
                // (most do real work). accept/release reads of the child
                // ref classify non-allocating; method calls on it stay
                // conservative, so a hook that calls into the child keeps
                // its scratch.
                let elide_scratch = self.method_scratch_elidable(
                    &lc.body,
                    &lc.params,
                    lc.ret.as_ref(),
                );
                if !elide_scratch {
                    self.open_method_scratch()?;
                }
                // Fn-call protocol shave (2026-07-02): elidable bodies
                // can't publish, so their empty-frame exit flush skips
                // the bus drain.
                let prev_skip_drain = self.current_fn_skip_exit_drain;
                self.current_fn_skip_exit_drain = elide_scratch;

                let mut scope = Scope::default();

                // accept/release get the child pointer as their second
                // arg; bind it under the source-level param name as a
                // LocusRef local so `g.X` lowers to GEP+load. (release
                // is the death-side bookend — same 2-arg shape.)
                if kind == "accept" || kind == "release" {
                    let (param_name, child_locus) = if kind == "accept" {
                        info.accept_param
                            .as_ref()
                            .expect("accept declared with accept_param")
                    } else {
                        info.release_param
                            .as_ref()
                            .expect("release declared with release_param")
                    };
                    let child_ptr = func
                        .get_nth_param(1)
                        .expect("child_ptr param")
                        .into_pointer_value();
                    // Stash through an alloca'd ptr slot so the
                    // existing Ident-resolution path works without
                    // special-casing fn args.
                    let slot = self
                        .builder
                        .build_alloca(
                            self.context.ptr_type(AddressSpace::default()),
                            param_name,
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(slot, child_ptr)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    scope.locals.insert(
                        param_name.clone(),
                        (slot, CodegenTy::LocusRef(child_locus.clone())),
                    );
                }

                let end = self.lower_block(&lc.body, &mut scope)?;
                if end == BlockEnd::Open {
                    self.flush_dissolve_frame()?;
                    self.close_method_scratch()?;
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                } else {
                    let _ = self.deferred_dissolves.pop();
                    self.current_method_scratch = None;
                    self.current_method_caller_arena = None;
                }
                self.current_fn_skip_exit_drain = prev_skip_drain;

                self.current_fn = None;
                self.current_self = None;
            }
        }

        // on_failure(child: ChildL, err: ClosureViolation) body.
        // LLVM sig: void(parent_self, child_self, violation_ptr).
        // Inside the body: bind the child param as a LocusRef
        // local (so c.field GEPs into child struct) and the err
        // param as a TypeRef("ClosureViolation") local (so
        // err.locus / err.closure GEP into the violation struct).
        if let (Some(failure_decl), Some((child_locus_name, ff))) =
            (l.members.iter().find_map(|m| match m {
                LocusMember::Failure(fd) => Some(fd),
                _ => None,
            }), info.failure_handler.as_ref())
        {
            let child_locus_name = child_locus_name.clone();
            let ff = *ff;
            let entry = self.context.append_basic_block(ff, "entry");
            self.builder.position_at_end(entry);
            let parent_self = ff
                .get_nth_param(0)
                .expect("parent_self param")
                .into_pointer_value();
            let child_self = ff
                .get_nth_param(1)
                .expect("child_self param")
                .into_pointer_value();
            let viol_ptr = ff
                .get_nth_param(2)
                .expect("violation param")
                .into_pointer_value();
            self.current_fn = Some(ff);
            self.current_user_fn_ret = None;
            self.current_self = Some(SelfCx {
                locus_name: l.name.name.clone(),
                struct_ty: info.struct_ty,
                self_ptr: parent_self,
                fields: info.fields.clone(),
            });
            self.loops.clear();
            self.push_dissolve_frame();

            let mut scope = Scope::default();
            // Bind c (the child) and err (the violation) as
            // alloca'd-pointer locals so the existing Ident
            // resolution path works.
            let child_param_name = failure_decl.params[0].name.name.clone();
            let err_param_name = failure_decl.params[1].name.name.clone();
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let child_slot = self
                .builder
                .build_alloca(ptr_t, &child_param_name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(child_slot, child_self)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            scope.locals.insert(
                child_param_name,
                (child_slot, CodegenTy::LocusRef(child_locus_name.clone())),
            );
            let err_slot = self
                .builder
                .build_alloca(ptr_t, &err_param_name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(err_slot, viol_ptr)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            scope.locals.insert(
                err_param_name,
                (err_slot, CodegenTy::TypeRef("ClosureViolation".into())),
            );

            let end = self.lower_block(&failure_decl.body, &mut scope)?;
            if end == BlockEnd::Open {
                // m46-vocab follow-up: on_failure runs
                // synchronously inside an outer substrate cell
                // (the closure-eval body that detected the
                // violation). Don't drain the bus queue here —
                // the outer cell owns that. A recursive drain
                // would pull queued cells mid-tick, advancing
                // accumulator state across "this fire's"
                // boundary. See `flush_dissolve_frame_kind`.
                self.flush_dissolve_frame_kind(false)?;
                self.builder
                    .build_return(None)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            } else {
                let _ = self.deferred_dissolves.pop();
            }

            self.current_fn = None;
            self.current_self = None;
        }

        // Synthetic __birth_closures + __dissolve_closures fns:
        // each evaluates every closure assertion in its epoch in
        // declaration order. Each assertion computes |left -
        // right| <= tolerance; on fail, write a ClosureViolation
        // report to stderr (fd 2 via dprintf) and exit non-zero,
        // OR route to the parent's on_failure if the call site
        // passed a non-null handler. Pass paths flow through
        // silently. Same body shape per epoch — only the closure
        // subset differs — so we use a small helper closure.
        for epoch in [
            EpochSpec::Birth,
            EpochSpec::Dissolve,
            EpochSpec::Tick,
            EpochSpec::Explicit,
        ]
        .iter()
        {
            let fn_slot = match epoch {
                EpochSpec::Birth => info.birth_closures_fn,
                EpochSpec::Dissolve => info.dissolve_closures_fn,
                EpochSpec::Tick => info.tick_closures_fn,
                EpochSpec::Explicit => info.explicit_closures_fn,
                _ => None,
            };
            let func = match fn_slot {
                Some(f) => f,
                None => continue,
            };
            let entry = self.context.append_basic_block(func, "entry");
            self.builder.position_at_end(entry);
            let self_ptr = func
                .get_nth_param(0)
                .expect("self_ptr param")
                .into_pointer_value();
            let parent_self_arg = func
                .get_nth_param(1)
                .expect("parent_self_or_null param")
                .into_pointer_value();
            let parent_handler_arg = func
                .get_nth_param(2)
                .expect("on_failure_or_null param")
                .into_pointer_value();
            self.current_fn = Some(func);
            self.current_user_fn_ret = None;
            self.current_self = Some(SelfCx {
                locus_name: l.name.name.clone(),
                struct_ty: info.struct_ty,
                self_ptr,
                fields: info.fields.clone(),
            });
            self.loops.clear();
            self.push_dissolve_frame();

            for (cname, assertion, c_epoch) in &info.closures {
                if c_epoch != epoch {
                    continue;
                }
                self.lower_closure_check(
                    &l.name.name,
                    cname,
                    assertion,
                    parent_self_arg,
                    parent_handler_arg,
                    c_epoch.clone(),
                )?;
            }

            // m42 + m44: tick AND explicit fire inside / from
            // contexts where re-entering the bus queue would
            // be wrong. Tick fires from the cooperative drain
            // (recursive drain would pull every remaining cell
            // into one tick's call stack). Explicit fires at
            // a user-chosen checkpoint inside the locus's
            // body — the surrounding body's normal
            // flush_dissolve_frame at scope exit will handle
            // any drain at the right time. So both pop the
            // frame manually and skip the drain. For
            // Birth + Dissolve the drain is historically OK
            // (those run outside the drain context).
            if matches!(epoch, EpochSpec::Tick | EpochSpec::Explicit) {
                let _ = self.deferred_dissolves.pop();
            } else {
                self.flush_dissolve_frame()?;
            }
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.current_fn = None;
            self.current_self = None;
        }

        // m43: __duration_closures body. Each duration-epoch
        // closure gates on monotonic-elapsed-since-last-fire
        // before evaluating the assertion. last_fire is
        // updated to monotonic-now BEFORE the assertion runs
        // so a routed-and-absorbed violation in on_failure
        // doesn't reset the interval clock. Per-closure last-
        // fire fields parallel info.duration_last_fire_field_idxs
        // in declaration order.
        if let Some(duration_fn) = info.duration_closures_fn {
            let entry =
                self.context.append_basic_block(duration_fn, "entry");
            self.builder.position_at_end(entry);
            let self_ptr = duration_fn
                .get_nth_param(0)
                .expect("self_ptr param")
                .into_pointer_value();
            let parent_self_arg = duration_fn
                .get_nth_param(1)
                .expect("parent_self_or_null param")
                .into_pointer_value();
            let parent_handler_arg = duration_fn
                .get_nth_param(2)
                .expect("on_failure_or_null param")
                .into_pointer_value();
            self.current_fn = Some(duration_fn);
            self.current_user_fn_ret = None;
            self.current_self = Some(SelfCx {
                locus_name: l.name.name.clone(),
                struct_ty: info.struct_ty,
                self_ptr,
                fields: info.fields.clone(),
            });
            self.loops.clear();
            self.push_dissolve_frame();

            let i64_t = self.context.i64_type();
            let mut duration_idx: usize = 0;
            // Snapshot info.closures to a local (we'll be
            // emitting nested LLVM that may otherwise alias
            // through self.user_loci while we work).
            let closures_snapshot = info.closures.clone();
            for (cname, assertion, c_epoch) in &closures_snapshot {
                let duration_expr = match c_epoch {
                    EpochSpec::Duration(e) => e.clone(),
                    _ => continue,
                };
                let last_field_idx = info
                    .duration_last_fire_field_idxs[duration_idx];
                duration_idx += 1;

                // last = load __duration_last_fire_<i>
                let last_slot = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        self_ptr,
                        last_field_idx,
                        &format!(
                            "{}.duration[{}].last.ptr",
                            l.name.name, cname
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let last = self
                    .builder
                    .build_load(i64_t, last_slot, "duration.last")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                // now = time::monotonic() (i64 ns)
                let (now_v, _) =
                    self.lower_time_monotonic(&[])?;
                let now = now_v.into_int_value();
                // Evaluate the duration expression in self-scope
                // — same approach as closure assertions, so it
                // can reference self.X (e.g.
                // `duration(self.poll_interval)`).
                let scope = Scope::default();
                let (dur_v, _) =
                    self.lower_expr(&duration_expr, &scope)?;
                let dur_n = dur_v.into_int_value();
                let elapsed = self
                    .builder
                    .build_int_sub(now, last, "duration.elapsed")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let should_fire = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SGE,
                        elapsed,
                        dur_n,
                        "duration.should_fire",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let fire_bb = self.context.append_basic_block(
                    duration_fn,
                    &format!("duration.{}.fire", cname),
                );
                let skip_bb = self.context.append_basic_block(
                    duration_fn,
                    &format!("duration.{}.skip", cname),
                );
                self.builder
                    .build_conditional_branch(should_fire, fire_bb, skip_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // fire_bb: store now -> last_fire, then run
                // the assertion check (which routes to
                // on_failure on violation). F.34: after the
                // assertion runs, zero any fields named in the
                // closure's `resets_per_epoch(...)` clause so the
                // next window starts clean. Order matters — the
                // reset MUST happen AFTER the assertion so the
                // current window's accumulated value is what's
                // judged.
                self.builder.position_at_end(fire_bb);
                self.builder
                    .build_store(last_slot, now)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.lower_closure_check(
                    &l.name.name,
                    cname,
                    assertion,
                    parent_self_arg,
                    parent_handler_arg,
                    c_epoch.clone(),
                )?;
                if let Some(reset_fields) =
                    info.resets_per_epoch_per_closure.get(cname).cloned()
                {
                    for fname in &reset_fields {
                        let (field_idx, field_ty) = info
                            .fields
                            .get(fname)
                            .map(|(i, t)| (*i, t.clone()))
                            .ok_or_else(|| {
                                CodegenError::Unsupported(format!(
                                    "resets_per_epoch: field `{}` not \
                                     found on locus `{}` (typecheck \
                                     should have rejected this)",
                                    fname, l.name.name
                                ))
                            })?;
                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                info.struct_ty,
                                self_ptr,
                                field_idx,
                                &format!(
                                    "{}.{}.reset.ptr",
                                    l.name.name, fname
                                ),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        match field_ty {
                            CodegenTy::Int => {
                                let zero = self
                                    .context
                                    .i64_type()
                                    .const_zero();
                                self.builder
                                    .build_store(field_ptr, zero)
                                    .map_err(|e| {
                                        CodegenError::LlvmEmit(e.to_string())
                                    })?;
                            }
                            CodegenTy::Float => {
                                let zero = self
                                    .context
                                    .f64_type()
                                    .const_zero();
                                self.builder
                                    .build_store(field_ptr, zero)
                                    .map_err(|e| {
                                        CodegenError::LlvmEmit(e.to_string())
                                    })?;
                            }
                            _ => {
                                return Err(CodegenError::Unsupported(
                                    format!(
                                        "resets_per_epoch: field `{}` on \
                                         locus `{}` has non-numeric type \
                                         (typecheck should have rejected \
                                         this)",
                                        fname, l.name.name
                                    ),
                                ));
                            }
                        }
                    }
                }
                self.builder
                    .build_unconditional_branch(skip_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(skip_bb);
            }

            // Same flush-skip rationale as tick: duration
            // fires inside the cooperative drain loop, so we
            // can't recursively re-enter the queue here.
            let _ = self.deferred_dissolves.pop();
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.current_fn = None;
            self.current_self = None;
        }

        // m42: tick_wrapper body. The bus drain loop calls
        // tick_wrapper(self) after each handler returns; the
        // wrapper loads the parent fields baked onto the struct
        // at instantiation time and forwards to the 3-arg
        // __tick_closures fn. This indirection lets us route
        // tick violations through the same parent on_failure
        // handler the birth/dissolve epochs use, without
        // changing the bus drain loop's signature.
        // m43-followup: duration uses the same shape so the
        // pinned post-run path has a 1-arg call site that can
        // route violations off-main-thread.
        let wrapper_pairs = [
            (info.tick_wrapper_fn, info.tick_closures_fn, "tick"),
            (
                info.duration_wrapper_fn,
                info.duration_closures_fn,
                "duration",
            ),
        ];
        for (wrapper_opt, eval_opt, tag) in wrapper_pairs {
            let (Some(wrapper_fn), Some(eval_fn)) = (wrapper_opt, eval_opt)
            else {
                continue;
            };
            let entry = self.context.append_basic_block(wrapper_fn, "entry");
            self.builder.position_at_end(entry);
            let self_ptr = wrapper_fn
                .get_nth_param(0)
                .expect("self_ptr param")
                .into_pointer_value();
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let parent_self_slot = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.parent_self_field_idx,
                    "parent_self.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let parent_self = self
                .builder
                .build_load(ptr_t, parent_self_slot, "parent_self")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            let parent_handler_slot = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.parent_on_failure_field_idx,
                    "parent_handler.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let parent_handler = self
                .builder
                .build_load(
                    ptr_t,
                    parent_handler_slot,
                    "parent_handler",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            self.builder
                .build_call(
                    eval_fn,
                    &[
                        self_ptr.into(),
                        parent_self.into(),
                        parent_handler.into(),
                    ],
                    &format!("{}.closures.call", tag),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // Locus user-fns (`fn` members): same body lowering as
        // lifecycle methods, but with their declared param list
        // (after self_ptr) bound as locals + their declared
        // return type tracked.
        for member in &l.members {
            if let LocusMember::Fn(fd) = member {
                // Open-question #24 MVP: fallible locus methods
                // take a separate body-lowering path that wires
                // a unified exit block + sret epilogue, parallel
                // to the free-fn fallible shape but minus the
                // `__caller_arena` plumbing (v0.1 value-only
                // scope-cut: see `is_value_only_codegen_ty`).
                if fd.fallible.is_some() {
                    let info_ref = info.clone();
                    self.lower_fallible_locus_method_body(l, &info_ref, fd)?;
                    continue;
                }
                let func = *info
                    .user_methods
                    .get(&fd.name.name)
                    .expect("locus fn declared in pass A2");
                let entry = self.context.append_basic_block(func, "entry");
                self.builder.position_at_end(entry);
                let self_ptr = func
                    .get_nth_param(0)
                    .expect("self_ptr param")
                    .into_pointer_value();
                self.current_fn = Some(func);
                let ret_ty = match &fd.ret {
                    None => None,
                    Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
                };
                self.current_user_fn_ret = Some(ret_ty.clone());
                self.current_self = Some(SelfCx {
                    locus_name: l.name.name.clone(),
                    struct_ty: info.struct_ty,
                    self_ptr,
                    fields: info.fields.clone(),
                });
                self.loops.clear();
                self.push_dissolve_frame();
                // Stage-1 scratch elision: skip the per-call subregion
                // malloc when the body provably allocates nothing and the
                // return is a by-value scalar (no return deep-copy). Leaving
                // `current_method_scratch` None routes the (absent)
                // allocations to `self.__arena` and no-ops destroy/close.
                let elide_scratch = self.method_scratch_elidable(
                    &fd.body,
                    &fd.params,
                    fd.ret.as_ref(),
                );
                if !elide_scratch {
                    self.open_method_scratch()?;
                }
                // Fn-call protocol shave (2026-07-02): elidable bodies
                // can't publish, so their empty-frame exit flush skips
                // the bus drain. (Counter.inc-class methods and
                // scalar getters drop to the C call shape.)
                let prev_skip_drain = self.current_fn_skip_exit_drain;
                self.current_fn_skip_exit_drain = elide_scratch;

                let mut scope = Scope::default();
                self.di_pending_params.clear();
                for (i, p) in fd.params.iter().enumerate() {
                    let lt = self.type_expr_to_codegen_ty(&p.ty)?;
                    let alloca = self.alloca_for(&lt, &p.name.name)?;
                    let v = func
                        .get_nth_param((i + 1) as u32)
                        .expect("locus method arg index in range");
                    self.builder
                        .build_store(alloca, v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    if self.di.is_some() {
                        // Stage-3 DWARF: declared as formal params by
                        // the body's first di_enter_stmt.
                        self.di_pending_params.push((
                            alloca,
                            p.name.name.clone(),
                            lt.clone(),
                        ));
                    }
                    scope.locals.insert(p.name.name.clone(), (alloca, lt));
                }

                // m42: gate subscribed bus-handler bodies on the
                // __quarantined flag at entry. m41b (m45-followup-2
                // form) nulls subjects in the C-runtime entries
                // vec so future publishes skip a quarantined
                // subscriber, but cells enqueued before quarantine
                // remain in the queue and would otherwise still
                // fire. This entry gate matches the interpreter's
                // `delivery.subscription.locus.quarantined`
                // check in dispatch_bus, so already-queued
                // deliveries observe the stop-trying signal.
                let is_subscribed_handler = info
                    .subscriptions
                    .iter()
                    .any(|(_, h, _, _)| h == &fd.name.name);
                if is_subscribed_handler {
                    let i64_t = self.context.i64_type();
                    let q_slot = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            info.quarantined_field_idx,
                            "handler.quarantined.ptr",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let q_val = self
                        .builder
                        .build_load(i64_t, q_slot, "handler.quarantined")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let is_q = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            q_val,
                            i64_t.const_int(0, false),
                            "handler.is_quarantined",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let skip_bb = self
                        .context
                        .append_basic_block(func, "handler.skip");
                    let body_bb = self
                        .context
                        .append_basic_block(func, "handler.body");
                    self.builder
                        .build_conditional_branch(is_q, skip_bb, body_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder.position_at_end(skip_bb);
                    // The entry (open_method_scratch) created this
                    // handler's scratch subregion before the
                    // quarantine gate; bailing out here without
                    // destroying it leaks one subregion per delivery
                    // to an already-quarantined subscriber. Destroy
                    // it (state stays live for the body path, which
                    // is mutually exclusive at run time and closes
                    // it normally at its own exit).
                    self.emit_method_scratch_destroy()?;
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder.position_at_end(body_bb);
                }

                let end = self.lower_block(&fd.body, &mut scope)?;
                if end == BlockEnd::Open {
                    // m42: if this user fn is a registered bus
                    // handler AND the locus has tick closures,
                    // fire __tick_closures HERE — after the
                    // body's effects but BEFORE the tail
                    // bus_queue_drain. The tail drain (m26)
                    // would otherwise recursively process the
                    // next queued cell first, and tick would
                    // see the next cell's state instead of
                    // this handler's. Tick is the natural
                    // "between substrate cells" point and
                    // belongs inline with the cell's body
                    // termination, ahead of any cooperative
                    // yield.
                    let is_subscribed_handler = info
                        .subscriptions
                        .iter()
                        .any(|(_, h, _, _)| h == &fd.name.name);
                    if is_subscribed_handler {
                        // Load parent_self and parent_handler
                        // once; both tick and duration fns
                        // need them and the loads are pure
                        // GEP+load.
                        let parent_self_slot = self
                            .builder
                            .build_struct_gep(
                                info.struct_ty,
                                self_ptr,
                                info.parent_self_field_idx,
                                "epoch.parent_self.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let parent_self_v = self
                            .builder
                            .build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                parent_self_slot,
                                "epoch.parent_self",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .into_pointer_value();
                        let parent_handler_slot = self
                            .builder
                            .build_struct_gep(
                                info.struct_ty,
                                self_ptr,
                                info.parent_on_failure_field_idx,
                                "epoch.parent_handler.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let parent_handler_v = self
                            .builder
                            .build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                parent_handler_slot,
                                "epoch.parent_handler",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .into_pointer_value();
                        if let Some(tick_fn) = info.tick_closures_fn {
                            self.builder
                                .build_call(
                                    tick_fn,
                                    &[
                                        self_ptr.into(),
                                        parent_self_v.into(),
                                        parent_handler_v.into(),
                                    ],
                                    &format!(
                                        "{}.{}.tick.post_handler.call",
                                        l.name.name, fd.name.name
                                    ),
                                )
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                        // m43: duration shares the cell-boundary
                        // cadence with tick — same call site,
                        // each closure self-gates on elapsed
                        // time inside the synthesized fn.
                        if let Some(duration_fn) =
                            info.duration_closures_fn
                        {
                            self.builder
                                .build_call(
                                    duration_fn,
                                    &[
                                        self_ptr.into(),
                                        parent_self_v.into(),
                                        parent_handler_v.into(),
                                    ],
                                    &format!(
                                        "{}.{}.duration.post_handler.call",
                                        l.name.name, fd.name.name
                                    ),
                                )
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                    }
                    // Subscribed bus handlers run synchronously
                    // inside a substrate cell — the outer
                    // `lotus_bus_queue_drain` loop in C is what
                    // popped this cell and called us. Emitting
                    // another drain at the handler's tail
                    // recursively re-enters that loop on the
                    // C stack, one frame per pending cell.
                    // 50k queued ticks → 50k frames → SIGSEGV
                    // (bench `bus_dispatch.hl` cliffs between
                    // 20k and 25k under the default 8 MB stack).
                    // The outer drain's for-loop already pumps
                    // the queue to empty; the tail drain is
                    // redundant for cooperative handlers.
                    // Same rationale as on_failure bodies
                    // (which already pass `drain_queue=false`).
                    if is_subscribed_handler {
                        self.flush_dissolve_frame_kind(false)?;
                    } else {
                        self.flush_dissolve_frame()?;
                    }
                    match ret_ty {
                        None => {
                            self.close_method_scratch()?;
                            self.builder
                                .build_return(None)
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                        Some(_) => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` method `{}` falls through without \
                                 returning a value",
                                l.name.name, fd.name.name
                            )));
                        }
                    }
                } else {
                    let _ = self.deferred_dissolves.pop();
                    self.current_method_scratch = None;
                    self.current_method_caller_arena = None;
                }
                self.current_fn_skip_exit_drain = prev_skip_drain;

                self.current_fn = None;
                self.current_user_fn_ret = None;
                self.current_self = None;
            }
        }

        // Mode bodies — same lowering as Fn members, with the
        // synthetic method name (bulk / harmonic / resolution).
        for member in &l.members {
            if let LocusMember::Mode(md) = member {
                let mode_name = match md.kind {
                    ModeKind::Bulk => "bulk",
                    ModeKind::Harmonic => "harmonic",
                    ModeKind::Resolution => "resolution",
                };
                let func = *info
                    .user_methods
                    .get(mode_name)
                    .expect("mode declared in pass A2");
                let entry = self.context.append_basic_block(func, "entry");
                self.builder.position_at_end(entry);
                let self_ptr = func
                    .get_nth_param(0)
                    .expect("self_ptr param")
                    .into_pointer_value();
                self.current_fn = Some(func);
                let ret_ty = match &md.ret {
                    None => None,
                    Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
                };
                self.current_user_fn_ret = Some(ret_ty.clone());
                self.current_self = Some(SelfCx {
                    locus_name: l.name.name.clone(),
                    struct_ty: info.struct_ty,
                    self_ptr,
                    fields: info.fields.clone(),
                });
                self.loops.clear();
                self.push_dissolve_frame();
                // Stage-1 scratch elision (mode body) — same gate as fn
                // members: non-allocating body + by-value scalar/Unit ret.
                let elide_scratch = self.method_scratch_elidable(
                    &md.body,
                    &md.params,
                    md.ret.as_ref(),
                );
                if !elide_scratch {
                    self.open_method_scratch()?;
                }
                // Fn-call protocol shave (2026-07-02): see fn members.
                let prev_skip_drain = self.current_fn_skip_exit_drain;
                self.current_fn_skip_exit_drain = elide_scratch;

                let mut scope = Scope::default();
                self.di_pending_params.clear();
                for (i, p) in md.params.iter().enumerate() {
                    let lt = self.type_expr_to_codegen_ty(&p.ty)?;
                    let alloca = self.alloca_for(&lt, &p.name.name)?;
                    let v = func
                        .get_nth_param((i + 1) as u32)
                        .expect("mode arg index in range");
                    self.builder
                        .build_store(alloca, v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    if self.di.is_some() {
                        self.di_pending_params.push((
                            alloca,
                            p.name.name.clone(),
                            lt.clone(),
                        ));
                    }
                    scope.locals.insert(p.name.name.clone(), (alloca, lt));
                }

                let end = self.lower_block(&md.body, &mut scope)?;
                if end == BlockEnd::Open {
                    self.flush_dissolve_frame()?;
                    match ret_ty {
                        None => {
                            self.close_method_scratch()?;
                            self.builder
                                .build_return(None)
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                        Some(_) => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` mode `{}` falls through without \
                                 returning a value",
                                l.name.name, mode_name
                            )));
                        }
                    }
                } else {
                    let _ = self.deferred_dissolves.pop();
                    self.current_method_scratch = None;
                    self.current_method_caller_arena = None;
                }
                self.current_fn_skip_exit_drain = prev_skip_drain;

                self.current_fn = None;
                self.current_user_fn_ret = None;
                self.current_self = None;
            }
        }
        Ok(())
    }

}
