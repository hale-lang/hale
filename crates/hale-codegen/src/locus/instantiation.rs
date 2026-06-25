//! Locus instantiation lowering: the constructor path. Handles
//! arena acquire, capacity slot allocation, param init (including
//! recursive child instantiation), bus subscriber registration,
//! closure-eval scheduling, recovery-handler routing, pinned-thread
//! spawn, and the deferred-dissolve frame for long-lived loci.
//! Round 4d of the codegen model-org refactor — the heavyweight.

use std::collections::BTreeMap;

use hale_syntax::ast::{
    BirthCheckDecl, CapacitySlotKind, Expr, Literal, LocusMember,
    ProjectionClass, RecognitionSubMode, ScheduleClass,
    StructInit, TopDecl,
};
use inkwell::types::BasicType;
use inkwell::values::PointerValue;
use inkwell::AddressSpace;

use crate::bus::runtime::BusRuntime;
use crate::codegen::{
    chunk_hint_for_coop_pool, CodegenError, CodegenTy, Cx,
    DefaultInit, ParamValue, Scope, SelfCx, SlotForm, SyncMode,
};
use crate::locus::dissolve::LocusDissolve;
use crate::stdlib::time::TimeStdlib;

pub(crate) trait LocusInstantiate<'ctx> {
    fn lower_locus_instantiation(
        &mut self,
        locus_name: &str,
        inits: &[StructInit],
        scope: &Scope<'ctx>,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodegenError>;
}

impl<'ctx, 'p> LocusInstantiate<'ctx> for Cx<'ctx, 'p> {
    fn lower_locus_instantiation(
        &mut self,
        locus_name: &str,
        inits: &[StructInit],
        scope: &Scope<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        // m82: the let-binding above us may have signaled that
        // this locus's dissolve should be deferred to the
        // enclosing fn's scope-exit flush. Take the flag now —
        // before any nested `lower_expr` calls below — so default
        // / override expressions that themselves construct loci
        // don't accidentally consume our flag and skip their own
        // eager dissolve. Outermost instantiation owns it; nested
        // ones see false.
        let defer_for_let = std::mem::take(&mut self.defer_next_locus_dissolve);
        // Phase-2 (2): parent locus is constructing us as a field
        // default / override. Suppress eager dissolve — the parent
        // owns us and cascades dissolve from its own dispatch.
        // See `instantiating_for_parent_field` doc on the Codegen
        // struct. Same mem::take discipline as defer_for_let: only
        // the outermost instantiation in the expression takes it.
        let parent_owns_via_field =
            std::mem::take(&mut self.instantiating_for_parent_field);
        // F.31 (2026-05-23): consume any placement override the
        // caller set. The override is applied to a LOCAL clone of
        // `info` for this instantiation only — nested
        // instantiations see no override and fall back to their
        // own info default. Pinned-required struct fields (e.g.
        // mailbox slot for pinned subscribers) were pre-laid-out
        // at declare time based on `pinned_locus_types`, populated
        // from main's placement entries + adapter bindings at
        // codegen startup. So the override at instantiation time
        // is safe: the struct already has the necessary slots.
        //
        // Phase 3b: the F.29 cascade (emit_locus_field_dissolves +
        // _drains) now skips pinned-placed fields, so the parent's
        // dissolve doesn't double-dispatch the pinned child's
        // lifecycle (which runs on the pthread). The deferred-
        // dissolve frame's flush handles pthread_join +
        // arena_destroy via the existing is_pinned_entry path.
        let placement_override =
            std::mem::take(&mut self.placement_for_next_locus_instantiation);
        // F.31 Phase 4: consume the parallel pool-name override
        // before any recursion happens (a nested instantiation
        // in this locus's params-init loop would otherwise
        // clobber it). Stash the prior value to restore at exit.
        let coop_pool_override = std::mem::take(
            &mut self.cooperative_pool_for_next_locus_instantiation,
        );
        let prev_current_coop_pool = self.current_cooperative_pool.take();
        if coop_pool_override.is_some() {
            self.current_cooperative_pool = coop_pool_override;
        }
        let mut info = self
            .user_loci
            .get(locus_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "no locus `{}` declared",
                    locus_name
                ))
            })?;
        if let Some(sc) = placement_override {
            info.schedule_class = sc;
        }

        // Build a name → override-expr map for the call site.
        let overrides: BTreeMap<&str, &Expr> = inits
            .iter()
            .map(|i| (i.name.name.as_str(), &i.value))
            .collect();

        // 3d+3e: if the current self's locus accepts a child of
        // this locus's type, the parent is about to retain a
        // pointer to this instance via its synthetic
        // `__children[]` array (appended below in the accept/
        // append block). When the parent reads through that
        // array later — including in a *different* lifecycle
        // method than the one we're being instantiated in — a
        // stack alloca would dangle the moment the spawning
        // method returns. Detect that case here and route the
        // struct allocation through the parent's arena so it
        // lives until the parent's arena is destroyed. The
        // deferred-dissolve push at the end of this fn is also
        // suppressed so the spawning method's exit flush
        // doesn't tear the child down. v1 trade-off: the
        // child's drain()/dissolve() bodies don't fire on
        // process exit — a children-cascade at parent dissolve
        // would tighten this; deferred to v1.x. See the
        // resolution note in notes/hale-friction.md
        // `nested-locus-child-field-reads-return-garbage`.
        let parent_accepts_us = if let Some(cs) = self.current_self.as_ref() {
            self.user_loci
                .get(&cs.locus_name)
                .and_then(|p| p.accept_param.as_ref().cloned())
                .map(|(_, child_ty)| child_ty == locus_name)
                .unwrap_or(false)
        } else {
            false
        };
        // m90 (3f fix): if the current fn declares `-> Self` for this
        // locus, the instance can escape to the caller. A stack alloca
        // becomes dangling the moment the method returns, so the first
        // post-return read of `s.field` (or `s.method()`) sees still-
        // valid stack memory but the second sees overwritten state.
        // Detect the escape ahead of time and heap-allocate via the
        // program-lifetime payload arena instead. The eager dissolve
        // + arena_destroy are also skipped below — the locus is
        // semantically "moved" to the caller and lives for the
        // program. v1 trade-off; a return-slot ABI (caller-provided
        // out-pointer + scoped dissolve in the caller's frame) would
        // tighten this without leaking. The same heap path also
        // covers `let s = X{}; ...; return s;` because the let-bound
        // literal is instantiated with `current_user_fn_ret` still
        // pointing at the matching LocusRef.
        // G20 follow-up: the same routing fires when the fn returns
        // `Interface(I)` and this locus satisfies I — the
        // locus→interface coercion at the return site builds a fat
        // pointer whose data slot points at this locus, so the locus
        // must outlive the fn's subregion. The fat-pointer struct
        // itself is deep-copied into caller_arena by
        // emit_return_value_deep_copy.
        let returns_this_locus = self
            .current_user_fn_ret
            .as_ref()
            .and_then(|r| r.as_ref())
            .map(|t| match t {
                CodegenTy::LocusRef(n) => n == locus_name,
                CodegenTy::Interface(iface) => {
                    self.locus_satisfies_interface(locus_name, iface)
                }
                _ => false,
            })
            .unwrap_or(false);
        // 2026-05-24: consume the
        // `instantiating_into_payload_arena` flag. If set, our
        // PARENT is being m90-routed and we — as one of its
        // params fields — also need to land in payload arena
        // so the parent's struct slot points at program-
        // lifetime storage rather than the fn's stack frame.
        // The flag was set by the outer instantiation before
        // entering its params-init loop; we take it here so
        // GRANDCHILDREN see false and don't re-trigger unless
        // we ALSO m90-route ourselves (we do — same payload
        // alloc path as `returns_this_locus`).
        let routed_by_parent =
            std::mem::take(&mut self.instantiating_into_payload_arena);
        let go_to_payload_arena = returns_this_locus || routed_by_parent;
        // Pool-inheritance fix (2026-05-29): is this locus owned
        // beyond the enclosing scope? Only owned loci may inherit
        // the current pool at runtime (post run() / pool-tag their
        // subscriptions). A handler-local `let`-bound long-lived
        // locus is dissolved at the enclosing fn's scope exit, so
        // posting its run() (which would execute AFTER that
        // dissolve) is a use-after-free, and pool-tagging its
        // subscription would route to a worker that drains it only
        // after the locus is gone. For those, preserve the prior
        // behavior: synchronous run + global-queue subscription.
        // The canonical N-dynamic-children pattern is accept'd /
        // field-owned children, which ARE owned beyond scope.
        let owned_beyond_scope =
            parent_accepts_us || parent_owns_via_field || returns_this_locus;
        let self_ptr = if go_to_payload_arena {
            let alloc_fn = self
                .module
                .get_function("lotus_bus_payload_arena_alloc")
                .expect("lotus_bus_payload_arena_alloc declared");
            let i64_t = self.context.i64_type();
            let size = info
                .struct_ty
                .size_of()
                .expect("locus struct ty has known size");
            self.builder
                .build_call(
                    alloc_fn,
                    &[size.into(), i64_t.const_int(8, false).into()],
                    &format!("{}.self.heap", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_bus_payload_arena_alloc returns ptr")
                .into_pointer_value()
        } else if parent_owns_via_field && self.current_arena_override.is_some()
        {
            // Pool-inheritance follow-up (2026-05-29): a param-field
            // child (F.29 owned) must live in its OWNER's arena, not
            // a stack alloca in the instantiating method frame.
            // Previously this case fell through to the stack-alloca
            // branch below; that was a latent dangle on any cross-
            // lifecycle read of `self.children[i].<field>` (owner
            // birthed in one method, field read in another), and it
            // became a hard crash once an owner's `run()` is posted
            // to a pool — the posted run() executes AFTER the
            // instantiating frame returns, so the field-child's
            // stack slot is gone (garbage reads / segfault on the
            // first method call into the field-child). Allocate in
            // `current_arena_override`, which the owning locus's
            // instantiation set to its own arena before running its
            // params-init (where this field default is constructed),
            // so the field-child shares the owner's lifetime and is
            // wholesale-freed with the owner's arena — no dangle, no
            // per-instance leak into a longer-lived parent arena.
            let owner_arena = self
                .current_arena_override
                .expect("parent_owns_via_field guard checked is_some");
            let alloc_fn = self
                .module
                .get_function("lotus_arena_alloc")
                .expect("lotus_arena_alloc declared");
            let i64_t = self.context.i64_type();
            let size = info
                .struct_ty
                .size_of()
                .expect("locus struct ty has known size");
            self.builder
                .build_call(
                    alloc_fn,
                    &[
                        owner_arena.into(),
                        size.into(),
                        // align=16 (widest scalar — i128 / Decimal),
                        // per memory.md "Arena alignment contract".
                        // The stack-alloca path this replaces got
                        // 16-byte alignment for free from LLVM; an
                        // 8-byte arena alloc misaligns a Decimal
                        // field's i128 store (movaps). Over-aligning
                        // is always safe.
                        i64_t.const_int(16, false).into(),
                    ],
                    &format!("{}.self.in_owner_arena", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_arena_alloc returns ptr")
                .into_pointer_value()
        } else if parent_accepts_us {
            // 3d+3e fix: allocate the child struct in parent's arena.
            // Lives until parent's arena_destroy, so cross-lifecycle
            // reads through self.children stay valid (e.g. child
            // birthed in parent's birth(), read in parent's run()).
            let parent_self = self
                .current_self
                .as_ref()
                .cloned()
                .expect("parent_accepts_us implies current_self");
            let parent_info = self
                .user_loci
                .get(&parent_self.locus_name)
                .cloned()
                .expect("parent locus declared");
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let arena_field_ptr = self
                .builder
                .build_struct_gep(
                    parent_info.struct_ty,
                    parent_self.self_ptr,
                    parent_info.arena_field_idx,
                    &format!("{}.parent_arena.gep", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let parent_arena = self
                .builder
                .build_load(
                    ptr_t,
                    arena_field_ptr,
                    &format!("{}.parent_arena", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let alloc_fn = self
                .module
                .get_function("lotus_arena_alloc")
                .expect("lotus_arena_alloc declared");
            let i64_t = self.context.i64_type();
            let size = info
                .struct_ty
                .size_of()
                .expect("locus struct ty has known size");
            self.builder
                .build_call(
                    alloc_fn,
                    &[
                        parent_arena.into(),
                        size.into(),
                        i64_t.const_int(8, false).into(),
                    ],
                    &format!("{}.self.in_parent_arena", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_arena_alloc returns ptr")
                .into_pointer_value()
        } else if self.instantiating_persistent_singleton {
            // WASM entry-inversion: the `@export locus` singleton is
            // instantiated in `_hale_start` but OUTLIVES it — the host
            // calls its methods long after _hale_start returns. A stack
            // alloca would be a dangle (and O2's DSE drops the now-"dead"
            // field stores, so reads see garbage / aliased arrays — the
            // multi-array-field bug). Allocate it in the program-global
            // arena, which `_hale_start` creates and never destroys.
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let arena_global = self
                .module
                .get_global("lotus.arena.global")
                .expect("arena global declared");
            let global_arena = self
                .builder
                .build_load(
                    ptr_t,
                    arena_global.as_pointer_value(),
                    &format!("{}.singleton.arena", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let alloc_fn = self
                .module
                .get_function("lotus_arena_alloc")
                .expect("lotus_arena_alloc declared");
            let i64_t = self.context.i64_type();
            let size = info
                .struct_ty
                .size_of()
                .expect("locus struct ty has known size");
            self.builder
                .build_call(
                    alloc_fn,
                    &[
                        global_arena.into(),
                        size.into(),
                        i64_t.const_int(16, false).into(),
                    ],
                    &format!("{}.self.singleton", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_arena_alloc returns ptr")
                .into_pointer_value()
        } else if self.current_fn.is_some() {
            // Always hoist the locus-struct alloca to the fn's entry
            // block when we have one. A `build_alloca` placed at the
            // current insertion point inside a loop body lowers to a
            // dynamic-stack-alloc — LLVM consumes stack per iteration
            // without reclaiming until the fn returns. A statement-
            // position `Empty { };` in a tight loop blows the default
            // 8 MB stack at ~500k iterations.
            //
            // Entry-block hoist is correctness-preserving because the
            // struct's lifetime is fully contained within the
            // expression: arena_create → init → lifecycle → arena_
            // destroy. Subsequent iterations rewrite the same stack
            // slot. The arena-field nulling done by the helper is a
            // no-op for the immediate-dissolve case (the field is
            // overwritten with arena_create() before any read) but is
            // required for the deferred-dissolve case (where the
            // flush at fn-exit reads the field to decide whether to
            // tear the locus down). One helper covers both shapes.
            self.alloca_in_entry_with_nulled_arena(
                info.struct_ty,
                info.arena_field_idx,
                &format!("{}.self", locus_name),
            )?
        } else {
            // Defense — no current fn; fall back to current-position
            // alloca. Shouldn't be reached from normal user-fn codegen.
            self.builder
                .build_alloca(info.struct_ty, &format!("{}.self", locus_name))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };

        // First — initialize the synthetic `__arena` field
        // (struct slot 0) with a fresh arena. Allocations made
        // on behalf of this locus during the rest of
        // instantiation (composite-literal defaults / overrides)
        // and during its lifecycle method bodies will route
        // through `arena_alloc`, which prefers `current_self`'s
        // arena field over the program global.
        //
        // m22: if our parent is a chunked-class locus actively
        // accepting us (current_self set, parent declares
        // accept(child: ThisLocus), parent.projection_class ==
        // Chunked), allocate as a sub-region of the parent's
        // arena rather than a fresh top-level arena. Parent
        // tracks a slot index for us; on dissolve, our slot
        // returns to the parent's free-list for reuse.
        // Pick the arena-acquire strategy based on the parent's
        // projection class:
        //
        // - m22 chunked: parent's accept routes us through
        //   `lotus_arena_create_subregion(parent_arena)` — child
        //   gets its own arena, slot-bookkept on the parent's
        //   free-list.
        // - v1.x-3 recognition w/ fixed_cell: parent's accept
        //   routes us through `lotus_recpool_fixed_acquire`;
        //   the returned arena handle lives inline in the recpool
        //   cell. Child's release at dissolve clears the bitmap
        //   bit via `lotus_recpool_fixed_release` instead of
        //   `lotus_arena_destroy`.
        // - v1.x-3 recognition w/ shared_slab: parent's accept
        //   routes us through `lotus_recpool_slab_acquire`,
        //   which returns the SAME slab arena every sibling
        //   shares. Per-child release is a no-op; the whole
        //   slab frees at parent dissolve.
        // - otherwise (rich, top-level, parent doesn't accept
        //   us, or recognition w/ unshipped sub-mode after
        //   typecheck defense): fresh `lotus_arena_create()`.
        //
        // The strategy also determines what we stash on the
        // child's `__recpool_release_pool` / kind fields so
        // dissolve can route through the matching release fn.
        #[derive(Clone, Copy)]
        enum AcquireStrategy {
            Fresh,
            Subregion,
            RecpoolFixed,
            RecpoolSlab,
        }
        let (acquire_strategy, parent_self_ptr_opt) =
            if let Some(cs) = self.current_self.as_ref() {
                let parent_info = self
                    .user_loci
                    .get(&cs.locus_name)
                    .cloned()
                    .expect("current_self points to a declared locus");
                let parent_accepts_us = parent_info
                    .accept_param
                    .as_ref()
                    .map(|(_, child_ty)| child_ty == locus_name)
                    .unwrap_or(false);
                if parent_accepts_us {
                    match parent_info.projection_class {
                        ProjectionClass::Recognition(Some(p)) => match p.sub_mode {
                            RecognitionSubMode::FixedCell => {
                                (AcquireStrategy::RecpoolFixed, Some(cs.self_ptr))
                            }
                            RecognitionSubMode::SharedSlab => {
                                (AcquireStrategy::RecpoolSlab, Some(cs.self_ptr))
                            }
                            // Spillover + SummaryOnly are typecheck-
                            // rejected before codegen; defense: fall
                            // back to subregion shape rather than
                            // crash on missing recpool wiring.
                            _ => (AcquireStrategy::Subregion, Some(cs.self_ptr)),
                        },
                        ProjectionClass::Chunked => {
                            (AcquireStrategy::Subregion, Some(cs.self_ptr))
                        }
                        // Recognition(None) shouldn't appear in a
                        // locus-annotation context (parser produces
                        // Recognition(Some(_)) there), but defense:
                        // treat as Subregion.
                        ProjectionClass::Recognition(None) => {
                            (AcquireStrategy::Subregion, Some(cs.self_ptr))
                        }
                        ProjectionClass::Rich => (AcquireStrategy::Fresh, None),
                    }
                } else {
                    (AcquireStrategy::Fresh, None)
                }
            } else {
                (AcquireStrategy::Fresh, None)
            };

        // Recpool-strategy bookkeeping that must outlive this
        // block: we acquire the arena here (so __arena can be set
        // right after), but the child's __recpool_release_pool /
        // __recpool_release_kind stores are deferred until after
        // the unconditional zero-init pass further down (which
        // would otherwise clobber them).
        let mut pending_recpool_release: Option<(
            inkwell::values::BasicValueEnum,
            u64,
        )> = None;
        let new_arena = match acquire_strategy {
            AcquireStrategy::Fresh => {
                if info.arena_elidable {
                    // Locus body is provably non-allocating
                    // (see `locus_arena_elidable`). Point
                    // `__arena` at the caller's current arena
                    // instead of a fresh malloc'd one — nothing
                    // will allocate against it, and the matching
                    // dissolve skips `lotus_arena_destroy`.
                    let caller_arena = self.current_arena_ptr()?;
                    caller_arena.into()
                } else {
                    // Use the labeled variant so
                    // LOTUS_ARENA_RESIDENCY dumps name the locus
                    // owning each arena. The label is a global
                    // string literal — same lifetime as the
                    // binary, satisfies the residency-entry
                    // pointer-outlives-arena contract.
                    //
                    // F.32-3 (2026-05-25): when this locus is
                    // being instantiated on a non-`main`
                    // cooperative pool, swap to the `_sized`
                    // variant and pass a per-pool chunk-size
                    // hint computed from the loci-per-pool
                    // count. Keeps the pool worker's resident
                    // set inside its L2 slice as it rotates
                    // through subscribers. Main-pool / no-pool-
                    // override paths keep the existing
                    // `_labeled` call — the main thread isn't
                    // rotating through many loci per drain.
                    let label_ptr = self
                        .builder
                        .build_global_string_ptr(
                            locus_name,
                            &format!("{}.arena.label", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .as_pointer_value();
                    let coop_pool_for_hint = self
                        .current_cooperative_pool
                        .as_deref()
                        .filter(|p| *p != "main");
                    let call_site = if let Some(pool_name) =
                        coop_pool_for_hint
                    {
                        let hint = chunk_hint_for_coop_pool(
                            &self.main_cooperative_pools,
                            pool_name,
                        );
                        let sized_fn = self
                            .module
                            .get_function("lotus_arena_create_labeled_sized")
                            .expect("lotus_arena_create_labeled_sized declared");
                        let hint_val = self
                            .context
                            .i64_type()
                            .const_int(hint, false);
                        self.builder
                            .build_call(
                                sized_fn,
                                &[label_ptr.into(), hint_val.into()],
                                &format!("{}.arena", locus_name),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    } else {
                        let arena_create_labeled = self
                            .module
                            .get_function("lotus_arena_create_labeled")
                            .expect("lotus_arena_create_labeled declared");
                        self.builder
                            .build_call(
                                arena_create_labeled,
                                &[label_ptr.into()],
                                &format!("{}.arena", locus_name),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    call_site
                        .try_as_basic_value()
                        .left()
                        .expect("arena_create returns ptr")
                }
            }
            AcquireStrategy::Subregion => {
                let parent_self_ptr = parent_self_ptr_opt
                    .expect("subregion strategy requires parent self_ptr");
                let parent_info = self
                    .user_loci
                    .get(&self.current_self.as_ref().unwrap().locus_name)
                    .cloned()
                    .expect("parent declared");
                let arena_field_ptr = self
                    .builder
                    .build_struct_gep(
                        parent_info.struct_ty,
                        parent_self_ptr,
                        parent_info.arena_field_idx,
                        "parent.__arena.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let parent_arena = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        arena_field_ptr,
                        "parent.__arena",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                if info.arena_elidable {
                    // v1.x-FRAMEWORK: same predicate as the Fresh-strategy
                    // elision (locus body is provably non-allocating), but
                    // applied to chunked-class children. Point `__arena`
                    // at the parent's arena directly so the per-child cost
                    // drops one library call + one allocator init. The
                    // matching `emit_locus_arena_destroy` bails on
                    // `arena_elidable`, and the parent's arena owns the
                    // child struct memory anyway (allocated via
                    // `lotus_arena_alloc(parent_arena, ...)` above).
                    parent_arena
                } else {
                    let subregion_fn = self
                        .module
                        .get_function("lotus_arena_create_subregion")
                        .expect("lotus_arena_create_subregion declared");
                    self.builder
                        .build_call(
                            subregion_fn,
                            &[parent_arena.into()],
                            &format!("{}.arena.sub", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("subregion_create returns ptr")
                }
            }
            strategy @ (AcquireStrategy::RecpoolFixed | AcquireStrategy::RecpoolSlab) => {
                let parent_self_ptr = parent_self_ptr_opt
                    .expect("recpool strategy requires parent self_ptr");
                let parent_info = self
                    .user_loci
                    .get(&self.current_self.as_ref().unwrap().locus_name)
                    .cloned()
                    .expect("parent declared");
                // Load `parent.__recpool` — the recpool handle
                // allocated at the parent's own instantiation.
                let recpool_field_ptr = self
                    .builder
                    .build_struct_gep(
                        parent_info.struct_ty,
                        parent_self_ptr,
                        parent_info.recpool_field_idx,
                        "parent.__recpool.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let parent_recpool = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        recpool_field_ptr,
                        "parent.__recpool",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let (acquire_fn_name, kind_const) = match strategy {
                    AcquireStrategy::RecpoolFixed => {
                        ("lotus_recpool_fixed_acquire", 1u64)
                    }
                    AcquireStrategy::RecpoolSlab => {
                        ("lotus_recpool_slab_acquire", 2u64)
                    }
                    _ => unreachable!(),
                };
                let acquire_fn = self
                    .module
                    .get_function(acquire_fn_name)
                    .expect("recpool acquire extern declared");
                let cell_arena = self
                    .builder
                    .build_call(
                        acquire_fn,
                        &[parent_recpool.into()],
                        &format!("{}.arena.recpool", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("recpool acquire returns ptr");
                // Defer the child-side stores so the zero-init
                // pass below doesn't overwrite them.
                pending_recpool_release = Some((parent_recpool, kind_const));
                cell_arena
            }
        };
        let arena_field = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.arena_field_idx,
                &format!("{}.__arena.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(arena_field, new_arena)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // v1.x-4b: compute a borrow map for each child slot whose
        // matching parent slot has `as_parent_for ThisLocus`.
        // Map child-slot-name → (parent_slot_struct_field_idx,
        // parent_slot_idx_in_decl_order). The bit index in
        // __slot_borrowed_mask uses the CHILD slot's declaration-
        // order index (so a child with two slots, second borrowed,
        // OR-s in bit 1). Validation is codegen-side defensive:
        // kind + elem_ty must match between parent and child slot
        // (matching `pool entries of Int` ↔ `pool entries of Int`);
        // mismatches reject here rather than miscompiling.
        let borrow_map: BTreeMap<String, (u32, u32)> = if let Some(cs) =
            self.current_self.as_ref()
        {
            let cs_name = cs.locus_name.clone();
            match self.user_loci.get(&cs_name).cloned() {
                Some(parent_info) => {
                    let mut map: BTreeMap<String, (u32, u32)> =
                        BTreeMap::new();
                    for parent_slot in &parent_info.capacity_slots {
                        if let Some(child_locus) =
                            parent_slot.as_parent_for.as_ref()
                        {
                            if child_locus != locus_name {
                                continue;
                            }
                            // Find the child slot with the same
                            // name; codegen-side kind+elem_ty
                            // validation.
                            let (child_idx, child_slot) = match info
                                .capacity_slots
                                .iter()
                                .enumerate()
                                .find(|(_, s)| s.name == parent_slot.name)
                            {
                                Some((i, s)) => (i, s),
                                None => {
                                    // Typecheck already rejects
                                    // this; reach here only on a
                                    // typecheck/codegen drift —
                                    // defensive panic-equivalent.
                                    return Err(CodegenError::Unsupported(format!(
                                        "as_parent_for `{}`: parent slot `{}` \
                                         has no match on child — typecheck \
                                         should have caught this",
                                        child_locus, parent_slot.name
                                    )));
                                }
                            };
                            if child_slot.kind != parent_slot.kind {
                                return Err(CodegenError::Unsupported(format!(
                                    "as_parent_for `{}`: slot `{}` kind \
                                     mismatch — parent is `{:?}`, child is \
                                     `{:?}`",
                                    child_locus,
                                    parent_slot.name,
                                    parent_slot.kind,
                                    child_slot.kind
                                )));
                            }
                            if child_slot.elem_ty != parent_slot.elem_ty {
                                return Err(CodegenError::Unsupported(format!(
                                    "as_parent_for `{}`: slot `{}` cell-type \
                                     mismatch — parent stores {:?}, child \
                                     stores {:?}",
                                    child_locus,
                                    parent_slot.name,
                                    parent_slot.elem_ty,
                                    child_slot.elem_ty
                                )));
                            }
                            // Form-vec slots can't borrow — the
                            // inline { cap, len, buf } struct is
                            // by-value, not a pointer. Reject so
                            // the user gets a clear message.
                            if child_slot.form.is_some()
                                || parent_slot.form.is_some()
                            {
                                return Err(CodegenError::Unsupported(format!(
                                    "as_parent_for `{}`: slot `{}` is an \
                                     `@form(vec)` slot — form-vec slots can't \
                                     be borrowed (storage is inline, not a \
                                     pointer)",
                                    child_locus, parent_slot.name
                                )));
                            }
                            map.insert(
                                parent_slot.name.clone(),
                                (
                                    parent_slot.struct_field_idx,
                                    child_idx as u32,
                                ),
                            );
                        }
                    }
                    map
                }
                None => BTreeMap::new(),
            }
        } else {
            BTreeMap::new()
        };

        // F.22 capacity slots: after slot 0 (arena) is set, init
        // each declared slot in declaration order by calling
        // `lotus_pool_create(size, 8)` or `lotus_heap_create(size,
        // 8)` and storing the returned allocator pointer into the
        // slot's struct field. Per spec §F.22 §"Slot lifetime",
        // slot init runs after slot 0 and before the locus's own
        // field initializers. The 8-byte alignment matches Hale
        // v0's universal scalar alignment — every value-shape
        // type lays out at 8-byte alignment in the locus struct,
        // so cells inherit the same.
        //
        // v1.x-4b: slots in `borrow_map` skip the create call and
        // instead copy the parent's allocator pointer into the
        // child's slot, OR-ing the bit in __slot_borrowed_mask.
        // Dissolve will read the bit and skip the destroy call so
        // the parent's allocator outlives the child (F.4 depth-
        // first cascade: child dissolves first; parent dissolves
        // its own slot afterward).
        for slot in &info.capacity_slots {
            let slot_field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    slot.struct_field_idx,
                    &format!("{}.__slot_{}.ptr", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

            if let Some((parent_field_idx, child_slot_idx)) =
                borrow_map.get(&slot.name).copied()
            {
                // Borrow branch: load parent's slot ptr, store into
                // child's slot, OR the mask bit.
                let parent_cs = self
                    .current_self
                    .as_ref()
                    .expect("borrow_map only built when current_self set");
                let parent_struct_ty = parent_cs.struct_ty;
                let parent_self_ptr = parent_cs.self_ptr;
                let parent_slot_field_ptr = self
                    .builder
                    .build_struct_gep(
                        parent_struct_ty,
                        parent_self_ptr,
                        parent_field_idx,
                        &format!(
                            "{}.{}.parent_slot.ptr",
                            locus_name, slot.name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let ptr_t_local =
                    self.context.ptr_type(AddressSpace::default());
                let parent_allocator = self
                    .builder
                    .build_load(
                        ptr_t_local,
                        parent_slot_field_ptr,
                        &format!(
                            "{}.{}.parent_alloc",
                            locus_name, slot.name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(slot_field_ptr, parent_allocator)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                // OR the bit into __slot_borrowed_mask.
                let i64_t_local = self.context.i64_type();
                let mask_ptr = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        self_ptr,
                        info.slot_borrowed_mask_field_idx,
                        &format!(
                            "{}.__slot_borrowed_mask.borrow.ptr",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let prev_mask = self
                    .builder
                    .build_load(
                        i64_t_local,
                        mask_ptr,
                        &format!(
                            "{}.__slot_borrowed_mask.prev",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let bit = i64_t_local
                    .const_int(1u64 << child_slot_idx, false);
                let new_mask = self
                    .builder
                    .build_or(
                        prev_mask,
                        bit,
                        &format!(
                            "{}.__slot_borrowed_mask.or",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(mask_ptr, new_mask)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                continue;
            }

            match slot.form {
                Some(SlotForm::Vec) => {
                    // v1.x-FORM-2: form-vec slot. The field IS the
                    // inline { cap, len, buf } struct; lotus_vec_init
                    // takes its address and zeroes it in place.
                    let init_fn = self
                        .module
                        .get_function("lotus_vec_init")
                        .expect("lotus_vec_init extern declared");
                    self.builder
                        .build_call(
                            init_fn,
                            &[slot_field_ptr.into()],
                            &format!("{}.{}.init", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Some(SlotForm::RingBuffer) => {
                    // v1.x-FORM-5: form-ring-buffer slot. The field
                    // IS the inline lotus_ring_buffer_t struct;
                    // lotus_ring_buffer_init mallocs cap×elem_size
                    // bytes and pins them for the locus's lifetime
                    // (no growth ever — `push` returns 0/false when
                    // full per the form contract).
                    let cap = slot.ring_buffer_cap.ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "@form(ring_buffer) `{}`: slot `{}` missing \
                             `cap` arg (typecheck should have rejected \
                             this — contact compiler maintainer)",
                            locus_name, slot.name
                        ))
                    })?;
                    // cap + elem_size are both `size_t` — build/narrow at
                    // the target size_t width (i32 wasm32 / i64 native).
                    let elem_size = self.size_to_usize(
                        self.llvm_basic_type(&slot.elem_ty)
                            .size_of()
                            .expect("ring_buffer cell type has known size"),
                    )?;
                    let cap_const = self.usize_type().const_int(cap, false);
                    let init_fn = self
                        .module
                        .get_function("lotus_ring_buffer_init")
                        .expect("lotus_ring_buffer_init extern declared");
                    self.builder
                        .build_call(
                            init_fn,
                            &[
                                slot_field_ptr.into(),
                                cap_const.into(),
                                elem_size.into(),
                            ],
                            &format!("{}.{}.init", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Some(SlotForm::Hashmap) => {
                    // v1.x-FORM-4: form-hashmap slot. The field IS
                    // the inline lotus_hashmap_t struct;
                    // lotus_hashmap_init allocates the initial 8
                    // slots and stashes key_size / value_size /
                    // key_type_tag at fixed offsets. The cell type
                    // is a user-declared struct; the indexed-by
                    // field on that struct supplies the key type.
                    let cell_name = match &slot.elem_ty {
                        CodegenTy::TypeRef(n) => n.clone(),
                        _ => {
                            return Err(CodegenError::Unsupported(format!(
                                "@form(hashmap) `{}`: slot `{}` cell type \
                                 must be a user-declared struct; got {:?} \
                                 (typecheck should have rejected this — \
                                 contact compiler maintainer)",
                                locus_name, slot.name, slot.elem_ty
                            )));
                        }
                    };
                    let cell_info = self
                        .user_types
                        .get(&cell_name)
                        .cloned()
                        .ok_or_else(|| CodegenError::Unsupported(format!(
                            "@form(hashmap) `{}`: cell type `{}` not \
                             registered in user_types",
                            locus_name, cell_name
                        )))?;
                    let field_name = slot
                        .indexed_by
                        .as_ref()
                        .ok_or_else(|| CodegenError::Unsupported(format!(
                            "@form(hashmap) `{}`: slot `{}` missing \
                             indexed_by clause (typecheck should have \
                             rejected this — contact compiler maintainer)",
                            locus_name, slot.name
                        )))?;
                    let (_field_idx, key_codegen_ty) = cell_info
                        .fields
                        .get(field_name)
                        .cloned()
                        .ok_or_else(|| CodegenError::Unsupported(format!(
                            "@form(hashmap) `{}`: indexed-by field `{}` \
                             not on cell type `{}` (typecheck should have \
                             rejected this — contact compiler maintainer)",
                            locus_name, field_name, cell_name
                        )))?;
                    let key_type_tag: u64 = match key_codegen_ty {
                        CodegenTy::Int => 0,
                        CodegenTy::String => 1,
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "@form(hashmap) `{}`: indexed-by field \
                                 `{}` has type {:?}; v1 supports Int and \
                                 String key types only (other key types \
                                 are deferred — see spec/forms.md \
                                 @form(hashmap) key types section)",
                                locus_name, field_name, other
                            )));
                        }
                    };
                    let key_llvm_ty =
                        self.llvm_basic_type(&key_codegen_ty);
                    // The lotus_hashmap_init* `size_t` params are target-
                    // pointer-width (i32 wasm32); `size_of()` is i64 — narrow
                    // to match (no-op native). See the builtins note.
                    let key_size = self.size_to_usize(
                        key_llvm_ty.size_of().expect("key type has known size"),
                    )?;
                    let value_size = self.size_to_usize(
                        cell_info
                            .struct_ty
                            .size_of()
                            .expect("cell struct has known size"),
                    )?;
                    let key_type_tag_const = self
                        .context
                        .i32_type()
                        .const_int(key_type_tag, false);
                    // F.32-1α/β2/γ (2026-05-24 → 2026-05-25): pick
                    // the init variant from sync_mode. The runtime
                    // `sync_mode` field on lotus_hashmap_t routes
                    // every entry point through the right locking
                    // discipline at call time. Lockfree uses a
                    // different init signature (extra fixed_cap
                    // arg) — handled in its own branch.
                    match slot.sync_mode {
                        SyncMode::Lockfree { fixed_cap } => {
                            let init_fn = self
                                .module
                                .get_function("lotus_hashmap_init_lockfree")
                                .expect("lotus_hashmap_init_lockfree extern declared");
                            // fixed_cap is size_t — build it at the target
                            // size_t width (i32 wasm32 / i64 native).
                            let cap_const =
                                self.usize_type().const_int(fixed_cap, false);
                            self.builder
                                .build_call(
                                    init_fn,
                                    &[
                                        slot_field_ptr.into(),
                                        key_size.into(),
                                        value_size.into(),
                                        key_type_tag_const.into(),
                                        cap_const.into(),
                                    ],
                                    &format!("{}.{}.init", locus_name, slot.name),
                                )
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                        _ => {
                            let init_fn_name = match slot.sync_mode {
                                SyncMode::None => "lotus_hashmap_init",
                                SyncMode::Serialized => "lotus_hashmap_init_serialized",
                                SyncMode::Striped => "lotus_hashmap_init_striped",
                                SyncMode::Lockfree { .. } => unreachable!(),
                            };
                            let init_fn = self
                                .module
                                .get_function(init_fn_name)
                                .expect("lotus_hashmap_init[_serialized|_striped] extern declared");
                            self.builder
                                .build_call(
                                    init_fn,
                                    &[
                                        slot_field_ptr.into(),
                                        key_size.into(),
                                        value_size.into(),
                                        key_type_tag_const.into(),
                                    ],
                                    &format!("{}.{}.init", locus_name, slot.name),
                                )
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                    }
                }
                None => {
                    // Default F.22 lowering: allocate via
                    // lotus_pool_create / lotus_heap_create and
                    // store the returned allocator pointer in the
                    // slot's ptr field.
                    let cell_size = self
                        .llvm_basic_type(&slot.elem_ty)
                        .size_of()
                        .expect("cell type has known size at LLVM level");
                    let align_const =
                        self.context.i64_type().const_int(8, false);
                    let create_fn_name = match slot.kind {
                        CapacitySlotKind::Pool => "lotus_pool_create",
                        CapacitySlotKind::Heap => "lotus_heap_create",
                    };
                    let create_fn = self
                        .module
                        .get_function(create_fn_name)
                        .expect("F.22 allocator extern declared");
                    let allocator_ptr = self
                        .builder
                        .build_call(
                            create_fn,
                            &[cell_size.into(), align_const.into()],
                            &format!("{}.{}.create", locus_name, slot.name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("F.22 allocator create returns ptr");
                    self.builder
                        .build_store(slot_field_ptr, allocator_ptr)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
        }

        // Initialize each field. Overrides go through lower_expr in
        // the caller's scope so any expression — not just literals —
        // can be passed. Defaults are either pre-resolved scalar
        // literals (DefaultInit::Const → const_param) or deferred
        // expressions (DefaultInit::Expr → lower_expr) that may
        // construct composite values like `Kernel { ... }` at
        // the instantiation site.
        //
        // While evaluating field defaults / overrides, allocations
        // created by composite literals (the only kind that allocs
        // at this point) should land in THE NEW LOCUS'S arena —
        // they're effectively part of its initial state. We achieve
        // that by setting `current_arena_override` to the new
        // arena ptr; arena_alloc's lookup prefers an override over
        // both `current_self` (the parent, here) and the program
        // global.
        // F.29 follow-up: zero-init __locus_ref_owned_mask BEFORE
        // the field-init loop, so the OR-sets that fire when a
        // LocusRef-typed field is initialized via a locus literal
        // aren't clobbered by a later zero-init pass. The cascade
        // emitters read this mask at teardown; bits set here
        // survive past the loop to flag parent-owned children.
        {
            let i64_t_zero = self.context.i64_type();
            let zero_mask = i64_t_zero.const_int(0, false);
            let lrom_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.locus_ref_owned_mask_field_idx,
                    &format!(
                        "{}.__locus_ref_owned_mask.ptr", locus_name
                    ),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(lrom_ptr, zero_mask)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        let prev_arena_override = self.current_arena_override;
        self.current_arena_override = Some(new_arena.into_pointer_value());
        // F.31 (2026-05-23): if we're instantiating the main
        // locus, its `placement { }` block's entries override
        // the schedule_class for each field's child
        // instantiation. The lookup happens once per field
        // inside the loop below (set immediately before
        // lower_expr, consumed by the recursive call to
        // lower_locus_instantiation).
        let is_main_locus = self
            .main_locus_name
            .as_ref()
            .map(|n| n == locus_name)
            .unwrap_or(false);
        // F.31 Phase 3b (2026-05-23): during the params-init
        // loop, surface this locus as a "params-init parent"
        // context so children instantiated as field defaults
        // can resolve their `__parent_self` /
        // `__parent_on_failure` against this locus via
        // `resolve_failure_route`. Pre-F.31 this worked because
        // children were instantiated inside the parent's
        // lifecycle method bodies (where `current_self` was
        // already set by method-body lowering); for placement-
        // pinned children instantiated as main-locus params
        // defaults, no lifecycle method has been entered yet.
        //
        // We can't simply set `current_self = this locus` —
        // that would clobber `self.X` resolution inside the
        // default expressions of nested instantiations (e.g.
        // `Lang { flavor: self.flavor }` inside Walk's
        // method body must see Walk as current_self). Instead
        // use a dedicated transient field that
        // resolve_failure_route consults as a fallback when
        // current_self is None.
        let prev_params_init_self = self.params_init_self.take();
        self.params_init_self = Some(SelfCx {
            locus_name: locus_name.to_string(),
            struct_ty: info.struct_ty,
            self_ptr,
            fields: info.fields.clone(),
        });
        for (fname, default) in info.defaults.iter() {
            // F.31: per-field placement override for main-locus
            // params. Looked up by field name in
            // `main_placement_map`; absent fields default to
            // None (the recursive call will keep the locus's
            // own schedule_class — Cooperative under F.31).
            if is_main_locus {
                self.placement_for_next_locus_instantiation = self
                    .main_placement_map
                    .get(fname.as_str())
                    .cloned();
                // F.31 Phase 4: parallel pool-name set. None when
                // the field has no cooperative-pool entry (either
                // pinned, default-pool main, or no placement).
                self.cooperative_pool_for_next_locus_instantiation = self
                    .main_cooperative_pools
                    .get(fname.as_str())
                    .cloned();
            }
            // Phase-2 (2): set the parent-field flag so a locus
            // literal evaluated for this field doesn't run its
            // eager dissolve. The flag is taken on entry to
            // lower_locus_instantiation, so nested child literals
            // beyond this one (e.g. defaults that themselves
            // construct loci that themselves construct loci) see
            // false — only the immediate child gets parent-owned
            // semantics. Other expression shapes ignore the flag.
            // Cleared after each lower_expr so it doesn't leak
            // past this field's evaluation.
            //
            // F.29 follow-up (2026-05-19): after lower_expr, the
            // flag's state tells us whether this field's value-
            // expr produced a parent-owned locus literal:
            // - `lower_locus_instantiation` consumes the flag via
            //   `mem::take` when it enters the `parent_owns_via_field`
            //   branch (true → false). So `owned_via_literal` is
            //   `!self.instantiating_for_parent_field` after
            //   lower_expr returns.
            // - Non-literal exprs (variable ref, const, conditional
            //   without a literal in the value-producing branch)
            //   leave the flag at true, so `owned_via_literal` is
            //   false. Const/Required don't invoke lower_expr at
            //   all and short-circuit to false. We use this signal
            //   to OR-set the matching bit in
            //   `__locus_ref_owned_mask` below, so the cascade
            //   knows which children to tear down at this parent's
            //   dissolve vs. leave alone (they're owned by an
            //   outer scope).
            let prev_field_flag = self.instantiating_for_parent_field;
            self.instantiating_for_parent_field = true;
            // 2026-05-24: if THIS locus is m90-routed to payload
            // arena, route each child literal there too. Child's
            // own `lower_locus_instantiation` consumes the flag
            // (mem::take), so we reset per-field; non-locus
            // expressions ignore it.
            if go_to_payload_arena {
                self.instantiating_into_payload_arena = true;
            }
            let (val, val_ty, owned_via_literal, came_from_literal) =
                if let Some(expr) = overrides.get(fname.as_str()) {
                    // iris F.4 (2026-05-23): override expressions
                    // are written at the CALL site, so `self.X`
                    // inside them must resolve to the CALLER's
                    // params-init context — not this locus's. The
                    // recursive instantiation has already set
                    // `params_init_self` to THIS locus; restore
                    // the outer (saved in `prev_params_init_self`)
                    // for the duration of override lowering, then
                    // put ours back.
                    let inner_pis = self.params_init_self.take();
                    self.params_init_self = prev_params_init_self.clone();
                    let r = self.lower_expr(expr, scope)?;
                    self.params_init_self = inner_pis;
                    let owned = !self.instantiating_for_parent_field;
                    self.instantiating_for_parent_field = prev_field_flag;
                    let from_lit = matches!(
                        expr,
                        Expr::Literal(
                            Literal::String(_) | Literal::Bytes(_), _),
                    );
                    (r.0, r.1, owned, from_lit)
                } else {
                    match default {
                        DefaultInit::Const(pv) => {
                            self.instantiating_for_parent_field =
                                prev_field_flag;
                            let r = self.const_param(pv);
                            // ParamValue has no Bytes variant — only
                            // String literals can land here as a
                            // ParamValue-shaped default.
                            let from_lit = matches!(pv, ParamValue::String(_));
                            (r.0, r.1, false, from_lit)
                        }
                        DefaultInit::Expr(e) => {
                            let r = self.lower_expr(e, scope)?;
                            let owned = !self.instantiating_for_parent_field;
                            self.instantiating_for_parent_field =
                                prev_field_flag;
                            let from_lit = matches!(
                                e,
                                Expr::Literal(
                                    Literal::String(_) | Literal::Bytes(_),
                                    _,
                                ),
                            );
                            (r.0, r.1, owned, from_lit)
                        }
                        DefaultInit::Required => {
                            self.instantiating_for_parent_field =
                                prev_field_flag;
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` instantiation: param `{}` is \
                                 required (no default) — supply it as \
                                 `{} {{ {}: ... }}`",
                                locus_name, fname, locus_name, fname
                            )));
                        }
                    }
                };
            // 2026-05-24: ensure the payload-arena routing flag
            // doesn't leak past this field. A field whose value
            // is a locus literal already consumed the flag via
            // its own `lower_locus_instantiation` (mem::take).
            // A primitive-valued field (Decimal default, etc.)
            // doesn't go through that path, so the flag stays
            // set and would falsely route the NEXT iteration's
            // child — or, worse, the next top-level locus
            // instantiation in the program if this loop has no
            // more children. Clear unconditionally to scope the
            // routing to "the immediate locus literal in this
            // params-init slot."
            self.instantiating_into_payload_arena = false;
            let (slot_idx, declared_ty) = info
                .fields
                .get(fname)
                .cloned()
                .expect("field declared by declare_locus_struct");
            // 2026-05-16 — locus → interface coercion at struct/
            // locus literal init. Mirrors the call-site coercion in
            // lower_fn_call so a stateful locus can flow into an
            // interface-typed field. Builds a fat pointer
            // {data, vtable} and stores that.
            let (val, val_ty) = if let (
                CodegenTy::Interface(iface),
                CodegenTy::LocusRef(l),
            ) = (&declared_ty, &val_ty)
            {
                let fat = self.coerce_to_interface(
                    val.into_pointer_value(),
                    l,
                    iface,
                )?;
                (fat.into(), declared_ty.clone())
            } else if came_from_literal
                && matches!(
                    (&val_ty, &declared_ty),
                    (CodegenTy::String, CodegenTy::StringView)
                        | (CodegenTy::Bytes, CodegenTy::BytesView)
                )
            {
                // F.30b (5b): String/Bytes literal default → View
                // storage. The literal lives in the global string
                // table (program-lifetime), so wrapping it via
                // lotus_view_from_static_data (epoch = static
                // sentinel) is structurally safe; the read-site
                // unpack helper sees the sentinel and returns
                // `src` directly without an epoch check.
                let wrapped = self.wrap_literal_as_view(val)?;
                (wrapped, declared_ty.clone())
            } else {
                (val, val_ty)
            };
            if val_ty != declared_ty {
                return Err(CodegenError::Unsupported(format!(
                    "locus `{}` field `{}` type mismatch: declared {:?}, \
                     got {:?}",
                    locus_name, fname, declared_ty, val_ty
                )));
            }
            // Bus-arena reclaim follow-up (2026-05-21): when this
            // instantiation runs inside a method body, the
            // field-init expression's value may live in the
            // caller's per-method scratch. Storing the pointer
            // verbatim into the new locus's struct works at the
            // moment, but the method scratch destroys at method
            // exit and any later read of the field dereferences
            // freed memory. Surfaced by a factory-method pattern:
            // `Handle { store: self.store, key: k }` returned
            // from inside `Registry.handle_for()` — `k` was a
            // String concat in `handle_for()`'s scratch; the
            // Handle in lazy-global later read garbage for its
            // key when the chained `.touch()` call dereferenced
            // it.
            //
            // Anchor heap-typed values in the new locus's arena
            // before the store. `current_arena_override` was set
            // above to the new locus's __arena, so reusing the
            // same destination keeps the layout consistent with
            // the existing locus instantiation routing.
            let val = if let Some(dest_arena) = self.current_arena_override {
                if Self::ty_needs_self_field_deep_copy(&declared_ty) {
                    self.emit_return_value_deep_copy(
                        val, &declared_ty, dest_arena,
                    )?
                } else {
                    val
                }
            } else {
                val
            };
            let field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    slot_idx,
                    &format!("{}.{}.ptr", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(field_ptr, val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // F.29 follow-up: OR the bit for this field into
            // `__locus_ref_owned_mask` if the field is LocusRef-
            // typed AND the value came from a parent-owned locus
            // literal. Externally-provided overrides leave the
            // bit clear so the cascade skips them — closes the
            // double-dissolve regression where the cascade
            // tore down loci it didn't own.
            if owned_via_literal {
                if let Some(&bit_pos) =
                    info.locus_ref_bit_per_field.get(fname.as_str())
                {
                    let i64_t_local = self.context.i64_type();
                    let mask_ptr = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            info.locus_ref_owned_mask_field_idx,
                            &format!(
                                "{}.__locus_ref_owned_mask.set.ptr",
                                locus_name
                            ),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let prev_mask = self
                        .builder
                        .build_load(
                            i64_t_local,
                            mask_ptr,
                            &format!(
                                "{}.__locus_ref_owned_mask.prev",
                                locus_name
                            ),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let bit = i64_t_local.const_int(1u64 << bit_pos, false);
                    let new_mask = self
                        .builder
                        .build_or(
                            prev_mask,
                            bit,
                            &format!(
                                "{}.__locus_ref_owned_mask.or",
                                locus_name
                            ),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(mask_ptr, new_mask)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
        }
        self.current_arena_override = prev_arena_override;
        // F.31 Phase 3b: restore params-init-parent context.
        // The placement-pinned children we instantiated above
        // have already stored their __parent_self values via
        // resolve_failure_route's lookup of params_init_self.
        self.params_init_self = prev_params_init_self;
        // F.31 Phase 4: cooperative-pool restore is deferred to
        // function exit (after the run_bb code that consumes it
        // — see end of fn + the pinned-branch early-return).

        // Zero-init the synthetic children-tracker fields if this
        // locus iterates `self.children`: __children starts as a
        // NULL heap pointer, __child_count and __child_cap at 0.
        // lotus_children_push lazily allocates the buffer on the
        // first accept. The buffer slots themselves are written on
        // accept dispatch.
        if let (Some(arr_idx), Some(cnt_idx), Some(cap_idx)) = (
            info.children_field_idx,
            info.child_count_field_idx,
            info.child_cap_field_idx,
        ) {
            let i64_t = self.context.i64_type();
            let zero = i64_t.const_int(0, false);
            let null_ptr = self
                .context
                .ptr_type(AddressSpace::default())
                .const_null();
            let arr_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    arr_idx,
                    &format!("{}.children.ptr", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(arr_ptr, null_ptr)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let cnt_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    cnt_idx,
                    &format!("{}.child_count.ptr", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(cnt_ptr, zero)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let cap_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    cap_idx,
                    &format!("{}.child_cap.ptr", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(cap_ptr, zero)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // m40: zero-init the synthetic __restart_count field.
        // Always present on every locus struct so the
        // `restart(child)` recovery primitive can bump it
        // without first checking whether the locus opted in.
        // Cap of 2 attempts per locus lifetime — past that,
        // restart() returns false and the violation falls
        // through to the parent's collapse path.
        let rc_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.restart_count_field_idx,
                &format!("{}.__restart_count.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let zero = self.context.i64_type().const_int(0, false);
        self.builder
            .build_store(rc_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // m41: zero-init the synthetic __quarantined flag.
        let q_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.quarantined_field_idx,
                &format!("{}.__quarantined.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(q_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // m45: zero-init the synthetic __restart_in_place_pending
        // flag. restart_in_place(c) sets it to 1; the rerun
        // branch in __birth_closures reads + clears it.
        let rip_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.restart_in_place_pending_field_idx,
                &format!("{}.__restart_in_place_pending.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(rip_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // v1.x-VIOLATE (F.27): zero-init the synthetic
        // __drain_requested flag. `violate NAME;` sets it to 1;
        // `self.draining` reads it back as a Bool.
        let dr_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.drain_requested_field_idx,
                &format!("{}.__drain_requested.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(dr_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // v1.x-4b: zero-init the synthetic __slot_borrowed_mask.
        // Bits get OR'd in below during slot init when a parent
        // has `as_parent_for ThisLocus` for one of this child's
        // slots.
        //
        // NOTE: this zero-init runs AFTER slot init (line ~28018);
        // a borrow that ORs a bit during slot init would be
        // clobbered here. In practice no currently-exercised path
        // combines borrow with this ordering, but the v1 layout
        // ordering should be revisited.
        let sbm_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.slot_borrowed_mask_field_idx,
                &format!("{}.__slot_borrowed_mask.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(sbm_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // F.29 follow-up: __locus_ref_owned_mask is zero-init'd
        // earlier — see the matching block right before the
        // field-init loop. The bits OR'd in by the field-init
        // loop must survive past it; doing the zero-init here
        // would clobber them.

        // v1.x-3: init the three synthetic recpool fields.
        //
        // `__recpool` defaults to null and is overwritten below if
        // this locus is Recognition-class with a shipped sub-mode.
        // `__recpool_release_pool` + `__recpool_release_kind` stay
        // zero at instantiation; they're set later by the parent's
        // accept step when this locus is being acquired from a
        // recognition pool (so that at dissolve we route through
        // `lotus_recpool_*_release` instead of arena_destroy).
        let ptr_t_local = self.context.ptr_type(AddressSpace::default());
        let null_ptr = ptr_t_local.const_null();
        let recpool_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.recpool_field_idx,
                &format!("{}.__recpool.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(recpool_ptr, null_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let release_pool_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.recpool_release_pool_field_idx,
                &format!("{}.__recpool_release_pool.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(release_pool_ptr, null_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let release_kind_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.recpool_release_kind_field_idx,
                &format!("{}.__recpool_release_kind.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(release_kind_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // v1.x-3: if this locus declares Recognition with a shipped
        // sub-mode, allocate the recpool now. Subsequent child
        // accepts read this handle through `parent.__recpool` and
        // route the child's arena through `lotus_recpool_*_acquire`.
        // The recpool is destroyed inside `emit_locus_arena_destroy`
        // alongside the arena teardown, after the F.4 cascade has
        // dissolved every child.
        if let ProjectionClass::Recognition(Some(params)) = info.projection_class {
            let create_fn_name = match params.sub_mode {
                RecognitionSubMode::FixedCell => Some("lotus_recpool_fixed_create"),
                RecognitionSubMode::SharedSlab => Some("lotus_recpool_slab_create"),
                // Spillover + SummaryOnly are typecheck-rejected
                // before codegen; defense: skip allocation here so
                // a future code path that gets through doesn't
                // crash on a missing extern.
                RecognitionSubMode::Spillover | RecognitionSubMode::SummaryOnly => None,
            };
            if let Some(create_fn_name) = create_fn_name {
                let create_fn = self
                    .module
                    .get_function(create_fn_name)
                    .expect("recpool create extern declared");
                let cap_const = self
                    .context
                    .i64_type()
                    .const_int(params.cap, false);
                // Cell stride is derived from the parent's accept-
                // method param type. v1 ships single-accept-per-
                // locus; when multi-accept lands, this becomes a
                // max-of-sizeof over the accept-type union.
                // Empty accept set on a Recognition locus would be
                // a typecheck error in principle; defense: pass
                // size_of(unit) so the recpool allocates a degenerate
                // block rather than crashing.
                let bytes_const = match &info.accept_param {
                    Some((_, child_locus_name)) => {
                        let child_info = self
                            .user_loci
                            .get(child_locus_name)
                            .expect("accept target locus known");
                        child_info
                            .struct_ty
                            .size_of()
                            .expect("child locus struct size known")
                    }
                    None => self.context.i64_type().const_zero(),
                };
                let pool = self
                    .builder
                    .build_call(
                        create_fn,
                        &[cap_const.into(), bytes_const.into()],
                        &format!("{}.__recpool.create", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("recpool_create returns ptr");
                self.builder
                    .build_store(recpool_ptr, pool)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }

        // v1.x-3: if we acquired this locus's arena from a parent's
        // recpool, restore the child-side release stash that the
        // zero-init above cleared. Now `emit_locus_arena_destroy`
        // will route teardown through the matching recpool release
        // fn (kind=1 fixed, kind=2 slab) instead of arena_destroy.
        if let Some((parent_recpool, kind_const)) = pending_recpool_release {
            self.builder
                .build_store(release_pool_ptr, parent_recpool)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(
                    release_kind_ptr,
                    self.context.i64_type().const_int(kind_const, false),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // m42: init the synthetic __parent_self / __parent_on_failure
        // fields. Resolve the (parent_self, on_failure_fn) pair via
        // the same routing the birth/dissolve epochs use; the bus
        // drain loop's tick wrapper reads these later when firing
        // tick-epoch closures (it has no static call-site context
        // for parent routing, so we bake it onto the struct here).
        // Loci without tick closures still pay the 16 bytes — the
        // uniform layout is worth more than the overhead.
        let (parent_self_val, parent_handler_val) =
            self.resolve_failure_route(locus_name);
        let parent_self_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_self_field_idx,
                &format!("{}.__parent_self.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(parent_self_slot, parent_self_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parent_handler_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_on_failure_field_idx,
                &format!("{}.__parent_on_failure.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(parent_handler_slot, parent_handler_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // 2026-05-30: init __owner_self to null. Overwritten at accept
        // dispatch (below, for accept'd children) with the accept'ing
        // parent's self_ptr; stays null for non-accept'd loci.
        let owner_self_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.owner_self_field_idx,
                &format!("{}.__owner_self.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(
                owner_self_slot,
                self.context.ptr_type(AddressSpace::default()).const_null(),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m43: init each __duration_last_fire_<i> field to
        // monotonic-now so the first fire happens after the
        // declared `N` elapses (not immediately at birth).
        // One time::monotonic() call per duration closure —
        // a tiny cost paid only for loci that declare
        // duration epochs.
        if !info.duration_last_fire_field_idxs.is_empty() {
            let (now_v, _) = self.lower_time_monotonic(&[])?;
            let now = now_v.into_int_value();
            for (i, field_idx) in info
                .duration_last_fire_field_idxs
                .iter()
                .enumerate()
            {
                let slot = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        self_ptr,
                        *field_idx,
                        &format!(
                            "{}.__duration_last_fire[{}].ptr",
                            locus_name, i
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(slot, now)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }

        // m46: zero each closure-accumulator slot at instantiation.
        // The slot's type drives the zero choice (Int/Duration use
        // i64 zero; Float/Decimal use f64 zero). Each `sum(self.X)`
        // detected during locus-decl gave us one slot.
        for slots in info.accumulators_per_closure.values() {
            for (i, slot) in slots.iter().enumerate() {
                self.zero_accumulator_slot(
                    info.struct_ty,
                    self_ptr,
                    slot,
                    &format!(
                        "{}.__acc[{}].ptr",
                        locus_name, i
                    ),
                )?;
            }
        }

        // F.7 ordering: if we're inside a parent locus's lifecycle
        // method AND the parent has an accept(child: ThisLocus) that
        // matches our type, call parent.accept(parent_self, child)
        // BEFORE this child's own birth. This is how
        // `02-parent-child` wires the coordinator's accept callback
        // to each greeter instantiation in run().
        //
        // Additionally, when the parent's children array exists
        // (accept declared), append the child's self_ptr to it +
        // bump child_count so `for child in self.children { ... }`
        // can iterate later.
        if let Some(parent_self) = self.current_self.clone() {
            let parent_info = self
                .user_loci
                .get(&parent_self.locus_name)
                .cloned()
                .expect("current_self points to a declared locus");
            if let Some((_, expected_child)) = &parent_info.accept_param {
                if expected_child == locus_name {
                    // 2026-05-30: record the accept'ing parent as this
                    // child's owner, so a flow child's run-wrapper can
                    // fire `parent.release(owner, child)` on completion.
                    let owner_slot = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            info.owner_self_field_idx,
                            &format!("{}.__owner_self.accept.ptr", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(owner_slot, parent_self.self_ptr)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let accept_fn = parent_info
                        .methods
                        .get("accept")
                        .copied()
                        .expect("accept_param implies accept method");
                    // v1.x-FRAMEWORK: skip the parent.accept(...) call
                    // when the body is empty. The children-array
                    // append still fires below so `for child in
                    // self.children { ... }` keeps observing the
                    // accepted child. Per-child cost dominator on
                    // chunked-class accept-in-a-loop patterns —
                    // the empty body's trailing bus drain costs
                    // hundreds of ns per child.
                    if !parent_info.empty_lifecycle.contains("accept") {
                        self.builder
                            .build_call(
                                accept_fn,
                                &[
                                    parent_self.self_ptr.into(),
                                    self_ptr.into(),
                                ],
                                &format!("{}.accept.call", parent_self.locus_name),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                    // Append child_self to the parent's growable
                    // children buffer:
                    //   lotus_children_push(&__children,
                    //       &__child_count, &__child_cap, child)
                    // The helper grows the heap buffer on demand and
                    // bumps the count — no fixed cap, no adjacent-
                    // memory corruption past 16 children (2026-05-29).
                    if let (Some(arr_idx), Some(cnt_idx), Some(cap_idx)) = (
                        parent_info.children_field_idx,
                        parent_info.child_count_field_idx,
                        parent_info.child_cap_field_idx,
                    ) {
                        let arr_ptr = self
                            .builder
                            .build_struct_gep(
                                parent_info.struct_ty,
                                parent_self.self_ptr,
                                arr_idx,
                                "children.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let cnt_ptr = self
                            .builder
                            .build_struct_gep(
                                parent_info.struct_ty,
                                parent_self.self_ptr,
                                cnt_idx,
                                "child.count.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let cap_ptr = self
                            .builder
                            .build_struct_gep(
                                parent_info.struct_ty,
                                parent_self.self_ptr,
                                cap_idx,
                                "child.cap.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let push_fn = self
                            .module
                            .get_function("lotus_children_push")
                            .expect("lotus_children_push declared");
                        self.builder
                            .build_call(
                                push_fn,
                                &[
                                    arr_ptr.into(),
                                    cnt_ptr.into(),
                                    cap_ptr.into(),
                                    self_ptr.into(),
                                ],
                                &format!(
                                    "{}.accept.push",
                                    parent_self.locus_name
                                ),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                }
            }
        }

        // Bus subscription registration runs BEFORE birth so a
        // locus's own birth() can publish on subjects it
        // subscribes to (rare but legal). For each declared
        // `bus subscribe "S" as h ...`: append (S, self_ptr,
        // <Locus>.h) into the global bus table.
        // For pinned-with-subscriptions loci we'll call this loop
        // again BELOW (after the mailbox alloca), passing the
        // mailbox pointer; cooperative loci register here with
        // mailbox = None (route through the global queue).
        let pinned_subscriptions =
            matches!(info.schedule_class, ScheduleClass::Pinned(_))
                && !info.subscriptions.is_empty();
        if !pinned_subscriptions {
            // Phase 3 (2026-05-25): `where key == self.X` in any
            // subscribe clause needs `current_self` set to this
            // new locus so the key-filter EXPR's `self.X` reads
            // resolve correctly. Save / restore around the loop
            // (same pattern birth_check_decls uses below).
            let prev_self = self.current_self.clone();
            self.current_self = Some(SelfCx {
                locus_name: locus_name.to_string(),
                struct_ty: info.struct_ty,
                self_ptr,
                fields: info.fields.clone(),
            });
            for (subject, handler_name, payload_type, key_filter) in &info.subscriptions {
                let handler_fn = info
                    .user_methods
                    .get(handler_name)
                    .copied()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "locus `{}` subscribes to `{}` with handler `{}` \
                             but no such method declared",
                            locus_name, subject, handler_name
                        ))
                    })?;
                // Form K6b (2026-05-20): subscriber-side branch.
                // If the subject is shm_ring-bound, emit
                // lotus_bus_register_subscriber_shm_ring instead
                // of the normal lotus_bus_register. The reader
                // thread spawned by the C runtime invokes the
                // handler with the slot pointer.
                if self.shm_ring_subjects.contains_key(subject) {
                    self.emit_bus_register_shm_ring(
                        subject,
                        self_ptr,
                        handler_fn,
                    )?;
                } else {
                    // 2026-06-01: register the handler-reclaim wrapper
                    // (handler + post-dispatch `terminate;` check) in
                    // place of the raw handler, so a cooperative
                    // subscriber can end its own life from a bus
                    // handler. Falls back to the raw handler if no
                    // wrapper was synthesized.
                    let reg_handler = self
                        .handler_reclaim_wrappers
                        .get(&(locus_name.to_string(), handler_name.clone()))
                        .copied()
                        .unwrap_or(handler_fn);
                    self.emit_bus_register(
                        subject,
                        self_ptr,
                        reg_handler,
                        None,
                        payload_type,
                        key_filter.as_ref(),
                        owned_beyond_scope,
                    )?;
                }
            }
            self.current_self = prev_self;
        }

        // Fire birth → run in order. drain → dissolve are deferred
        // if this locus is long-lived (has any bus subscribe), so
        // it can keep receiving published events until its
        // enclosing scope ends. Otherwise (ephemeral), all four
        // fire immediately like before.
        //
        // F.4 depth-first cascade: any child loci instantiated
        // inside this locus's run() body have already gone
        // through their full birth → run → drain → dissolve
        // sequence (each via this same lowering, recursively)
        // before run() returns. Long-lived loci defer drain →
        // dissolve to scope end via `deferred_dissolves`; the
        // cascade still fires depth-first when those scope-exit
        // calls run.
        let is_long_lived = !info.subscriptions.is_empty();
        let is_pinned =
            matches!(info.schedule_class, ScheduleClass::Pinned(_));

        // m28a + m28b: pinned-class loci spawn a pthread that runs
        // the locus's full lifecycle on its own thread:
        //   birth → run → (mailbox loop, if subscriptions) → drain → dissolve
        // Main thread joins at scope exit (deferred_dissolves
        // frame) before destroying the locus's arena. We synthesize
        // a per-locus thread_main whose signature matches pthread's
        // start-routine contract exactly (`ptr (ptr)`), so
        // pthread_create gets a direct function pointer with
        // self_ptr as its argument — no C adapter, no args struct.
        //
        // m28b: when the locus declares bus subscriptions, the
        // synthesized thread_main includes a mailbox loop after
        // run() — the pinned thread blocks in
        // lotus_mailbox_drain_one until cells arrive, processes
        // them one at a time (handler-atomic per substrate cell),
        // and exits the loop only when shutdown is signaled. The
        // mailbox itself is allocated at instantiation time and
        // stored in the locus's __mailbox field so the dispatch
        // path (which only sees the table-recorded mailbox ptr)
        // and the deferred-dissolve flush (which signals
        // shutdown) can both reach it.
        //
        // Still gated: accept (children of pinned would need
        // cross-thread cascade-dissolve coordination which adds
        // significant complexity beyond m28b), closures.
        if is_pinned {
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            if info.methods.contains_key("accept") {
                return Err(CodegenError::Unsupported(format!(
                    "pinned locus `{}` declares `accept()`; pinned coordinators \
                     wait on a future cross-thread cascade-dissolve milestone",
                    locus_name
                )));
            }
            if info.birth_closures_fn.is_some()
                || info.dissolve_closures_fn.is_some()
            {
                return Err(CodegenError::Unsupported(format!(
                    "pinned locus `{}` declares closures; cross-thread closure \
                     routing not yet supported",
                    locus_name
                )));
            }

            let i64_t = self.context.i64_type();
            let i32_t = self.context.i32_type();

            // m28b: if the locus subscribes, allocate its mailbox
            // and store the pointer in the locus's __mailbox slot.
            // Then register all subscriptions with that mailbox so
            // bus dispatch routes cells here instead of to the
            // global queue.
            let mailbox_ptr_opt: Option<PointerValue<'ctx>> =
                if let Some(mb_idx) = info.mailbox_field_idx {
                    let create_fn = self
                        .module
                        .get_function("lotus_mailbox_create")
                        .expect("lotus_mailbox_create declared");
                    let mb_ptr = self
                        .builder
                        .build_call(
                            create_fn,
                            &[],
                            &format!("{}.mailbox.create", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("lotus_mailbox_create returns ptr")
                        .into_pointer_value();
                    let mb_slot = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            mb_idx,
                            &format!("{}.__mailbox.ptr", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(mb_slot, mb_ptr)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // Phase 3 same setup as the cooperative path
                    // above — set current_self for the key-filter
                    // EXPR's `self.X` reads.
                    let prev_self_pinned = self.current_self.clone();
                    self.current_self = Some(SelfCx {
                        locus_name: locus_name.to_string(),
                        struct_ty: info.struct_ty,
                        self_ptr,
                        fields: info.fields.clone(),
                    });
                    for (subject, handler_name, payload_type, key_filter) in
                        &info.subscriptions
                    {
                        let handler_fn = info
                            .user_methods
                            .get(handler_name)
                            .copied()
                            .ok_or_else(|| {
                                CodegenError::Unsupported(format!(
                                    "locus `{}` subscribes to `{}` with handler \
                                     `{}` but no such method declared",
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
                    self.current_self = prev_self_pinned;
                    Some(mb_ptr)
                } else {
                    None
                };

            // Synthesize __pinned_main_<LocusName>(self_ptr) -> ptr.
            // Body: birth → run → (mailbox loop if subscriptions) →
            // drain → dissolve, returning null.
            let saved_block = self
                .builder
                .get_insert_block()
                .expect("pinned spawn inside an active block");
            let thread_main_name =
                format!("__pinned_main_{}", locus_name);
            let thread_main_ty =
                ptr_t.fn_type(&[ptr_t.into()], false);
            let thread_main = self
                .module
                .add_function(&thread_main_name, thread_main_ty, None);
            let entry_bb = self
                .context
                .append_basic_block(thread_main, "entry");
            self.builder.position_at_end(entry_bb);
            let thread_self =
                thread_main.get_nth_param(0).unwrap().into_pointer_value();
            // 2026-05-23: stash this locus's mailbox in
            // TLS so time::sleep / yield inside birth() and run()
            // can drain it without going through the post-run
            // mailbox loop. Closes the "cooperative→pinned dispatch
            // silent mid-program" issue — long-running pinned
            // servers (typical mdgw shape) never return from run()
            // so the post-run drain loop never started.
            if let Some(mb_idx) = info.mailbox_field_idx {
                let mb_slot = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        thread_self,
                        mb_idx,
                        "thread.mailbox.tls.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let mb_val = self
                    .builder
                    .build_load(ptr_t, mb_slot, "thread.mailbox.tls")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let set_current_fn = self
                    .module
                    .get_function("lotus_mailbox_set_current")
                    .expect("lotus_mailbox_set_current declared");
                self.builder
                    .build_call(
                        set_current_fn,
                        &[mb_val.into()],
                        "mailbox.set_current",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            for kind in &["birth", "run"] {
                if let Some(method) = info.methods.get(*kind) {
                    let skip = info.empty_lifecycle.contains(*kind);
                    if !skip {
                        self.builder
                            .build_call(
                                *method,
                                &[thread_self.into()],
                                &format!(
                                    "{}.{}.thread_call",
                                    locus_name, kind
                                ),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                    }
                    // m42: tick fires after run() on the pinned
                    // thread too. Use the wrapper here (it loads
                    // parent fields from the struct) since we're
                    // off the main thread and resolve_failure_route
                    // wouldn't see the right `current_self`.
                    // m43-followup: duration fires here too via
                    // the matching wrapper, closing the v0 limit
                    // where pinned post-run() didn't fire duration.
                    if *kind == "run" {
                        for (wrapper_opt, tag) in [
                            (info.tick_wrapper_fn, "tick"),
                            (info.duration_wrapper_fn, "duration"),
                        ] {
                            if let Some(wrapper) = wrapper_opt {
                                self.builder
                                    .build_call(
                                        wrapper,
                                        &[thread_self.into()],
                                        &format!(
                                            "{}.{}.post_run.thread_call",
                                            locus_name, tag
                                        ),
                                    )
                                    .map_err(|e| {
                                        CodegenError::LlvmEmit(e.to_string())
                                    })?;
                            }
                        }
                    }
                }
            }
            // m28b: mailbox loop. Reload the mailbox ptr from the
            // locus's __mailbox slot (we're on the pinned thread,
            // not the main thread, so we can't capture mailbox_ptr
            // from the enclosing build context — re-derive it from
            // self_ptr). Loop calls lotus_mailbox_drain_one, which
            // returns 0 on shutdown-empty; break the loop and run
            // drain/dissolve.
            if let Some(mb_idx) = info.mailbox_field_idx {
                let mb_slot_in_thread = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        thread_self,
                        mb_idx,
                        "thread.mailbox.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let mb_in_thread = self
                    .builder
                    .build_load(ptr_t, mb_slot_in_thread, "thread.mailbox")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                let drain_one_fn = self
                    .module
                    .get_function("lotus_mailbox_drain_one")
                    .expect("lotus_mailbox_drain_one declared");
                let loop_header =
                    self.context.append_basic_block(thread_main, "mb.header");
                let loop_after =
                    self.context.append_basic_block(thread_main, "mb.after");
                self.builder
                    .build_unconditional_branch(loop_header)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(loop_header);
                let drained = self
                    .builder
                    .build_call(
                        drain_one_fn,
                        &[mb_in_thread.into()],
                        "mb.drain.one",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_mailbox_drain_one returns i32")
                    .into_int_value();
                let keep_going = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        drained,
                        i32_t.const_int(0, false),
                        "mb.keep.going",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_conditional_branch(
                        keep_going, loop_header, loop_after,
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(loop_after);
            }
            for kind in &["drain", "dissolve"] {
                if let Some(method) = info.methods.get(*kind) {
                    if info.empty_lifecycle.contains(*kind) {
                        continue;
                    }
                    self.builder
                        .build_call(
                            *method,
                            &[thread_self.into()],
                            &format!(
                                "{}.{}.thread_call",
                                locus_name, kind
                            ),
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?;
                }
            }
            self.builder
                .build_return(Some(&ptr_t.const_null()))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // Restore builder to the calling fn so the rest of
            // the instantiation (pthread_create) emits there.
            self.builder.position_at_end(saved_block);

            // pthread_t alloca in the enclosing fn frame — hoisted
            // to entry so a locus-instantiation-in-loop pattern
            // doesn't leak stack per iter (mirrors the cliff-lift
            // session's fix for the locus .self struct alloca).
            let tid_alloca = self
                .alloca_in_entry(i64_t.into(), &format!("{}.tid", locus_name))?;
            let thread_main_ptr =
                thread_main.as_global_value().as_pointer_value();
            let null_attr = ptr_t.const_null();
            let create_fn = self
                .module
                .get_function("pthread_create")
                .expect("pthread_create declared");
            self.builder
                .build_call(
                    create_fn,
                    &[
                        tid_alloca.into(),
                        null_attr.into(),
                        thread_main_ptr.into(),
                        self_ptr.into(),
                    ],
                    &format!("{}.pthread_create", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let _ = mailbox_ptr_opt;

            // m28c: optional CPU-core affinity. If the locus
            // declared `: schedule pinned(core = N)`, route the
            // freshly-created tid through pthread_setaffinity_np
            // (via the C-side helper) so the OS scheduler keeps
            // this thread on the requested logical CPU.
            if let ScheduleClass::Pinned(Some(core)) = info.schedule_class {
                let tid_for_aff = self
                    .builder
                    .build_load(i64_t, tid_alloca, "pinned.tid.aff")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let core_const = i32_t.const_int(core as u64, true);
                let set_aff_fn = self
                    .module
                    .get_function("lotus_set_core_affinity")
                    .expect("lotus_set_core_affinity declared");
                self.builder
                    .build_call(
                        set_aff_fn,
                        &[tid_for_aff.into(), core_const.into()],
                        &format!("{}.set_aff", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }

            // Defer pthread_join + arena destroy to scope exit.
            // flush_dissolve_frame skips drain/dissolve for pinned
            // entries — those already ran on the pinned thread
            // before it returned (and pthread_join blocks until
            // that return). For pinned-with-subscriptions, the
            // flush ALSO signals the mailbox shutdown before
            // joining, so the pinned thread breaks out of its
            // mailbox loop and proceeds to drain/dissolve.
            if let Some(top) = self.deferred_dissolves.last_mut() {
                top.push((self_ptr, locus_name.to_string(), Some(tid_alloca)));
            } else {
                return Err(CodegenError::Unsupported(format!(
                    "pinned locus `{}` instantiated outside any tracked \
                     scope (no deferred-dissolve frame)",
                    locus_name
                )));
            }

            // F.31 Phase 4: pinned-branch restore mirror.
            self.current_cooperative_pool = prev_current_coop_pool;
            return Ok(self_ptr);
        }

        // m39: birth-epoch closures fire right after birth()
        // returns. We emit birth() + __birth_closures + run() in
        // sequence — the closure check sits between birth (which
        // initializes state) and run (which depends on that
        // state's invariants). If birth violates and the parent
        // has a matching on_failure handler, that handler runs;
        // otherwise the runtime exits with a diagnostic.
        if let Some(birth_fn) = info.methods.get("birth") {
            if !info.empty_lifecycle.contains("birth") {
                self.builder
                    .build_call(
                        *birth_fn,
                        &[self_ptr.into()],
                        &format!("{}.birth.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }
        if let Some(birth_closures_fn) = info.birth_closures_fn {
            let (parent_self, handler_ptr) =
                self.resolve_failure_route(&locus_name);
            self.builder
                .build_call(
                    birth_closures_fn,
                    &[
                        self_ptr.into(),
                        parent_self.into(),
                        handler_ptr.into(),
                    ],
                    &format!("{}.__birth_closures.call", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // F.27 v2 (2026-05-20): birth_check synthesis hook. Each
        // `birth_check { COND } -> violate NAME;` clause is
        // evaluated AFTER birth() body + birth-epoch closures, at
        // the well-defined point where every field has its
        // declared post-birth value. The violate routing
        // (set __drain_requested, indirect-call parent.on_failure
        // or panic) is emitted INLINE here — we do not use the
        // standard Stmt::Violate path because that path's
        // divergent-return would return from the CALLER's LLVM
        // function (e.g., Parent.run), not from the conceptual
        // "construction step." Absorbed violations should leave
        // the caller running normally after the failing
        // instantiation expression; we branch to a continuation
        // block instead of returning. Unhandled violations
        // (parent_on_failure == null) still call exit(1) per the
        // existing F.27 contract — same panic-with-diagnostic as
        // a regular violate.
        let birth_check_decls: Vec<BirthCheckDecl> = self
            .program
            .items
            .iter()
            .find_map(|item| match item {
                TopDecl::Locus(l) if l.name.name == locus_name => {
                    Some(
                        l.members
                            .iter()
                            .filter_map(|m| match m {
                                LocusMember::BirthCheck(bc) => Some(bc.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .unwrap_or_default();
        if !birth_check_decls.is_empty() {
            // We need current_self set for `self.X` reads in the
            // cond expressions to resolve against the newly-
            // constructed locus.
            let prev_self = self.current_self.clone();
            self.current_self = Some(SelfCx {
                locus_name: locus_name.to_string(),
                struct_ty: info.struct_ty,
                self_ptr,
                fields: info.fields.clone(),
            });
            let mut scope = Scope::default();
            for bc in &birth_check_decls {
                self.emit_birth_check(&bc, self_ptr, &info, &locus_name, &mut scope)?;
            }
            self.current_self = prev_self;
        }
        // m41: gate run() on __quarantined. If a parent's
        // on_failure called quarantine(self) during the birth-
        // closure check above, the flag is now set and we skip
        // run() entirely. Drain / dissolve still fire below.
        if let Some(run_fn) = info.methods.get("run") {
            // 2026-05-30: a FLOW child — some declared locus has a
            // `release(c: ThisType)` — is reclaimed when its run()
            // completes, via the run-wrapper. So it must NOT elide the
            // run block / skip posting even when run() is empty: the
            // wrapper still has to fire to run the reclaim.
            let is_flow = self.user_loci.values().any(|p| {
                matches!(&p.release_param, Some((_, c)) if c == locus_name)
            });
            // Whole-block elide when run() body is empty AND no
            // tick/duration closures need to fire after run AND it's
            // not a flow. The quarantine guard would otherwise stand
            // around a single unconditional jump.
            let skip_run_block = info.empty_lifecycle.contains("run")
                && info.tick_closures_fn.is_none()
                && info.duration_closures_fn.is_none()
                && !is_flow;
            if skip_run_block {
                // No-op block elided.
            } else {
            let i64_t = self.context.i64_type();
            let q_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.quarantined_field_idx,
                    &format!("{}.run.quarantined.ptr", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let q_val = self
                .builder
                .build_load(i64_t, q_ptr, "run.quarantined")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let zero = i64_t.const_int(0, false);
            let active = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    q_val.into_int_value(),
                    zero,
                    "run.active",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let func = self.current_fn.expect("current_fn set");
            let run_bb =
                self.context.append_basic_block(func, "run.do");
            let after_run_bb =
                self.context.append_basic_block(func, "run.after");
            self.builder
                .build_conditional_branch(active, run_bb, after_run_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(run_bb);
            if !info.empty_lifecycle.contains("run") || is_flow {
                // F.31 Phase 4b + pool-inheritance fix (2026-05-29):
                // a non-empty run() either runs synchronously here
                // (main thread / non-pool context) or is posted to
                // a cooperative pool's worker so it executes as its
                // own schedulable cell (and, on an async_io pool,
                // its own parkable coro) rather than capturing the
                // caller's thread/coro. Which pool:
                //   - compile-time-known when this locus is a
                //     main-locus params field placed on a pool
                //     (`current_cooperative_pool`), OR
                //   - the pool we are *currently running on* at
                //     runtime (`lotus_coop_pool_current`) when this
                //     instantiation sits inside a method/handler
                //     body executing on a pool worker — the case a
                //     dynamically-instantiated or accept'd child
                //     hits, where codegen has no static placement
                //     name. Without this, such a child's run() ran
                //     synchronously and, on an async_io pool, a
                //     parking recv in it parked the PARENT's coro
                //     (the 1-connection-cap bug).
                // Post-vs-sync is a runtime branch on whether the
                // resolved pool ptr is null, so one emission covers
                // main-thread loci (null → sync) and pool-worker
                // children (non-null → post).
                let wrapper_opt = self
                    .coop_pool_run_wrappers
                    .get(locus_name)
                    .copied();
                if let Some(wrapper) = wrapper_opt {
                    let ptr_t =
                        self.context.ptr_type(AddressSpace::default());
                    let pool_ptr = if let Some(pool_name) =
                        self.current_cooperative_pool.clone()
                    {
                        let pool_name_str = self.global_string(&pool_name);
                        let lookup_fn = self
                            .module
                            .get_function("lotus_coop_pool_lookup")
                            .expect("lotus_coop_pool_lookup declared");
                        self.builder
                            .build_call(
                                lookup_fn,
                                &[pool_name_str.into()],
                                "coop_pool.lookup.for_run",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .try_as_basic_value()
                            .left()
                            .expect("lotus_coop_pool_lookup returns ptr")
                            .into_pointer_value()
                    } else if owned_beyond_scope {
                        let current_fn = self
                            .module
                            .get_function("lotus_coop_pool_current")
                            .expect("lotus_coop_pool_current declared");
                        self.builder
                            .build_call(
                                current_fn,
                                &[],
                                "coop_pool.current.for_run",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .try_as_basic_value()
                            .left()
                            .expect("lotus_coop_pool_current returns ptr")
                            .into_pointer_value()
                    } else {
                        // Handler-local / not owned beyond scope:
                        // null ptr → the runtime branch below takes
                        // the synchronous path (prior behavior; no
                        // post that would outlive the binding).
                        ptr_t.const_null()
                    };
                    let is_null = self
                        .builder
                        .build_is_null(pool_ptr, "run.pool.is_null")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let post_bb =
                        self.context.append_basic_block(func, "run.post");
                    let sync_bb =
                        self.context.append_basic_block(func, "run.sync");
                    let cont_bb = self
                        .context
                        .append_basic_block(func, "run.post.cont");
                    self.builder
                        .build_conditional_branch(is_null, sync_bb, post_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // Post path: hand run() to the resolved pool.
                    self.builder.position_at_end(post_bb);
                    let post_fn = self
                        .module
                        .get_function("lotus_coop_pool_post")
                        .expect("lotus_coop_pool_post declared");
                    let wrapper_ptr =
                        wrapper.as_global_value().as_pointer_value();
                    let null_ptr = ptr_t.const_null();
                    let zero =
                        self.context.i64_type().const_int(0, false);
                    self.builder
                        .build_call(
                            post_fn,
                            &[
                                pool_ptr.into(),
                                wrapper_ptr.into(),
                                self_ptr.into(),
                                null_ptr.into(),
                                zero.into(),
                            ],
                            "coop_pool.post_run",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_unconditional_branch(cont_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // Sync path: no pool in scope — run inline. Call
                    // the WRAPPER (not run_fn directly) so a `terminate`
                    // in a synchronously-run locus still triggers the
                    // wrapper's reclaim (2026-05-30). The wrapper runs
                    // run() then reclaims iff __drain_requested is set;
                    // for a non-terminating run() it's just run() + a
                    // cheap latch check.
                    self.builder.position_at_end(sync_bb);
                    let null_payload =
                        self.context.ptr_type(AddressSpace::default()).const_null();
                    self.builder
                        .build_call(
                            wrapper,
                            &[self_ptr.into(), null_payload.into()],
                            &format!("{}.run.call.via_wrapper", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_unconditional_branch(cont_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder.position_at_end(cont_bb);
                } else {
                    // No wrapper synthesized (run-less locus or a
                    // synthesis gap) — run synchronously.
                    self.builder
                        .build_call(
                            *run_fn,
                            &[self_ptr.into()],
                            &format!("{}.run.call", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            // m42: tick fires after run() returns — run() is a
            // substrate cell just like a bus handler. Place the
            // call in the active branch so it doesn't fire on a
            // skipped (quarantined) run().
            if let Some(tick_fn) = info.tick_closures_fn {
                let (parent_self_t, handler_ptr_t) =
                    self.resolve_failure_route(&locus_name);
                self.builder
                    .build_call(
                        tick_fn,
                        &[
                            self_ptr.into(),
                            parent_self_t.into(),
                            handler_ptr_t.into(),
                        ],
                        &format!(
                            "{}.__tick_closures.post_run.call",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            // m43: duration shares the cell-boundary cadence
            // with tick — fires only when declared `N` has
            // elapsed since last fire of each duration closure.
            if let Some(duration_fn) = info.duration_closures_fn {
                let (parent_self_d, handler_ptr_d) =
                    self.resolve_failure_route(&locus_name);
                self.builder
                    .build_call(
                        duration_fn,
                        &[
                            self_ptr.into(),
                            parent_self_d.into(),
                            handler_ptr_d.into(),
                        ],
                        &format!(
                            "{}.__duration_closures.post_run.call",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            self.builder
                .build_unconditional_branch(after_run_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(after_run_bb);
            }
        }
        // m82: `defer_for_let` joins `is_long_lived` as a reason to
        // route this locus through the deferred-dissolve frame
        // instead of dissolving eagerly here. Both end up on the
        // same flush path at fn-exit (drain → __dissolve_closures
        // → dissolve → arena_destroy), preserving F.4 ordering.
        // The semantic distinction:
        //   - `is_long_lived` (locus has bus subscriptions): MUST
        //     defer so the locus stays alive to receive published
        //     events between birth and scope exit.
        //   - `defer_for_let` (this is a let-binding RHS): chooses
        //     to defer so user code can call methods on the bound
        //     handle after the struct-literal expression returns.
        // Pinned loci already took the `is_pinned` branch above
        // and don't reach this block.
        // m90 (3f fix): when this instantiation will escape via fn
        // return (returns_this_locus from above), suppress the
        // eager dissolve + arena_destroy and DO NOT push onto the
        // deferred_dissolves frame either — the fn-exit flush
        // would otherwise dissolve it on the way out. The locus
        // leaks (heap allocation + uncleaned arena live until
        // process exit); see the alloca branch above for the
        // trade-off note.
        // 3d+3e fix: parent-accepted children behave the same way
        // — the parent's children-array retains the pointer past
        // the spawning fn's stack frame, so dissolve happens at
        // parent's arena_destroy (which frees the child's struct
        // memory wholesale). The child's drain/dissolve method
        // bodies are skipped for v1; a children-cascade at parent
        // dissolve tightens this in v1.x.
        let defer = is_long_lived
            || defer_for_let
            || returns_this_locus
            || parent_accepts_us
            || parent_owns_via_field;
        if !defer {
            // 2026-06-01: the MAIN locus dissolves eagerly here, right
            // after its run() returns — but its `params` fields may be
            // placed on cooperative pools whose worker threads are
            // still executing those fields' run() loops (e.g. a
            // std::http::Server on `cooperative(pool = io)`). Tearing
            // down a field's arena here while its pool worker is mid-
            // run() is a use-after-free (the worker's next subregion
            // op locks the freed parent arena → SIGSEGV; observed in
            // fathom refstore). Join all pool workers FIRST so no
            // worker can touch a field arena we're about to free.
            // This was added then reverted (b35a449) because a classic
            // pool worker blocked in std::http::Server's accept()
            // couldn't be woken → the join hung; that's now fixed by
            // the shutdown-interruptible accept in lotus_tcp_accept_one
            // (poll + pool-shutdown check). shutdown_all is idempotent
            // (worker_started gate), so the later main-exit join is a
            // no-op. Gated to the main locus: a non-main ephemeral
            // locus dissolving mid-program must not join global pools.
            if is_main_locus {
                self.emit_coop_pool_shutdown_all()?;
            }
            // Phase-2 (3): cascade child-field drains depth-first
            // BEFORE outer's drain, per spec/runtime.md "drain()
            // cascades depth-first; children first, then self."
            self.emit_locus_field_drains(&info, self_ptr, locus_name)?;
            // drain → __dissolve_closures → dissolve. Mirrors the
            // interpreter ordering in eval.rs::dissolve_locus:
            // drain body fires first, then dissolve-epoch closures
            // are evaluated, then the user's dissolve() body.
            if let Some(drain_fn) = info.methods.get("drain") {
                if !info.empty_lifecycle.contains("drain") {
                    self.builder
                        .build_call(
                            *drain_fn,
                            &[self_ptr.into()],
                            &format!("{}.drain.call", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            if let Some(closures_fn) = info.dissolve_closures_fn {
                let (parent_self, handler_ptr) =
                    self.resolve_failure_route(&locus_name);
                self.builder
                    .build_call(
                        closures_fn,
                        &[
                            self_ptr.into(),
                            parent_self.into(),
                            handler_ptr.into(),
                        ],
                        &format!(
                            "{}.__dissolve_closures.call",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            if let Some(dissolve_fn) = info.methods.get("dissolve") {
                if !info.empty_lifecycle.contains("dissolve") {
                    self.builder
                        .build_call(
                            *dissolve_fn,
                            &[self_ptr.into()],
                            &format!("{}.dissolve.call", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            // Phase-2 (2): cascade dissolve for parent-owned child
            // loci held as `LocusRef`-typed param fields. Runs
            // AFTER outer's user dissolve body so the body can
            // still legitimately read its inner fields.
            self.emit_locus_field_dissolves(&info, self_ptr, locus_name)?;
            // Wholesale-free the locus's arena. Per spec/memory.md:
            // "When the locus dissolves, the region is freed
            // wholesale." Anything allocated for this locus —
            // composite-default literals, ClosureViolations, bus
            // payload copies it received — goes here.
            self.emit_locus_arena_destroy(&info, self_ptr, locus_name)?;
        } else if returns_this_locus {
            // Intentionally no-op: see m90 note above. The locus
            // outlives this fn's frame by design.
        } else if parent_accepts_us {
            // Intentionally no-op: see 3d+3e note above. Parent's
            // arena_destroy will wholesale-free the child struct
            // when the parent itself dissolves. Drain/dissolve
            // bodies don't fire on the child — v1 trade-off,
            // matches `returns_this_locus`.
        } else if parent_owns_via_field {
            // Phase-2 (2): parent locus is initializing this child
            // as a field default. The parent's dissolve dispatch
            // cascades into this child below (cascade-locus-field
            // path); skip eager dissolve here. Without the cascade
            // wired up the child's dissolve body never fires —
            // ok for Phase 1 (no segfault) but leaks the malloc-
            // backed buffer for shapes like BytesBuilder. Phase 2
            // wires the cascade through the parent's arena_destroy
            // ordering.
        } else if let Some(top) = self.deferred_dissolves.last_mut() {
            top.push((self_ptr, locus_name.to_string(), None));
        } else {
            // Should be unreachable: every fn body / lifecycle
            // body opens a frame in lower_program/method body
            // setup. If we hit this, the locus instantiation is
            // outside any tracked scope and won't get cleaned up.
            return Err(CodegenError::Unsupported(format!(
                "deferred-dissolve locus `{}` instantiated outside any tracked \
                 scope (no deferred-dissolve frame); long-lived={}, let-bound={}",
                locus_name, is_long_lived, defer_for_let,
            )));
        }

        // F.31 Phase 4: cooperative-pool context restore. Runs
        // at the end of every successful instantiation path
        // (mirror in the pinned-branch early-return above).
        // Outer instantiations have their own
        // current_cooperative_pool stashed in
        // prev_current_coop_pool; we restore it here so
        // nesting works correctly.
        self.current_cooperative_pool = prev_current_coop_pool;
        Ok(self_ptr)
    }

}
