//! Phase-A locus declaration: builds the LLVM struct layout +
//! declares lifecycle / user-fn LLVM function values. Round 4b
//! of the codegen model-org refactor.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    BusMember, CapacitySlotKind, ClosureAssertion,
    ClosureClause, EpochSpec, Expr, KeyFilter, LifecycleKind,
    Literal, LocusAnnotation, LocusDecl, LocusMember, ModeKind,
    ParamInit, ProjectionClass, ScheduleClass,
    TypeExpr,
};
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

use crate::codegen::{
    collect_sum_calls, count_self_field_accesses_in_locus,
    infer_accumulator_inner_type, locus_arena_elidable,
    locus_reads_self_children, param_value, AccumulatorKind,
    AccumulatorSlot, CapacitySlotLayout, CodegenError, CodegenTy,
    Cx, DefaultInit, LocusInfo, ParamValue, SlotForm, SyncMode,
};


/// Aliasing stage 2 (2026-07-02): `noalias` on `self` (param 0)
/// when BOTH reentrancy channels are provably closed for this
/// method:
/// - all declared params are by-value scalars (Int/Float/Bool/
///   Decimal/Duration) — nothing pointer-shaped can smuggle in a
///   second path into this locus's memory;
/// - the body is in the ELIDABLE set (proven non-allocating,
///   transitively): it cannot publish (payload copies allocate),
///   and with the task-10 exit-drain elision none of its callees
///   drain the cooperative queue — so no bus handler can run on
///   this locus (through the subscriber registry's foreign
///   pointer) inside the method's dynamic extent.
/// Nested `self.m()` calls are fine under LLVM's based-on
/// semantics: the callee's self is derived from this self.
fn apply_noalias_self_if_provable<'ctx>(
    cx: &Cx<'ctx, '_>,
    func: FunctionValue<'ctx>,
    locus_name: &str,
    method_name: &str,
    params: &[hale_syntax::ast::Param],
) {
    let elidable = cx
        .elidable_methods
        .get(locus_name)
        .map(|(e, _)| e.contains(method_name))
        .unwrap_or(false);
    if !elidable {
        return;
    }
    let all_scalar = params.iter().all(|p| {
        matches!(
            &p.ty,
            hale_syntax::ast::TypeExpr::Primitive(
                hale_syntax::ast::PrimType::Int
                    | hale_syntax::ast::PrimType::Float
                    | hale_syntax::ast::PrimType::Bool
                    | hale_syntax::ast::PrimType::Decimal
                    | hale_syntax::ast::PrimType::Duration,
                _,
            )
        )
    });
    if !all_scalar {
        return;
    }
    use inkwell::attributes::{Attribute, AttributeLoc};
    let noalias_kind = Attribute::get_named_enum_kind_id("noalias");
    func.add_attribute(
        AttributeLoc::Param(0),
        cx.context.create_enum_attribute(noalias_kind, 0),
    );
}

pub(crate) trait LocusDeclare<'ctx> {
    fn declare_locus_struct(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError>;
    fn declare_locus_methods(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> LocusDeclare<'ctx> for Cx<'ctx, 'p> {
    /// Pass A: declare a locus's struct type + lifecycle method
    /// signatures. Body lowering happens later (pass C).
    ///
    /// Each lifecycle method takes the struct pointer as its first
    /// arg and returns void. accept additionally takes the child
    /// pointer.
    /// Pass A1: register a locus's struct type + field layout. Done
    /// before any method signatures (pass A2) so accept methods can
    /// reference any locus's struct type, regardless of declaration
    /// order in source.
    fn declare_locus_struct(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError> {
        if !l.generics.is_empty() {
            // m63: generic locus templates emit nothing at decl
            // time. Per-instantiation specialized LocusDecls get
            // synthesized in the m63 monomorphization pass and
            // flow through this same fn under their mangled
            // name.
            return Ok(());
        }
        // Resolve projection class: explicit annotation wins;
        // otherwise default per spec/memory.md (chunked if
        // accept declared, rich otherwise — recognition is
        // explicit-only since N≈100-500 is too aggressive for
        // implicit choice).
        let has_accept = l.members.iter().any(|m| {
            matches!(m, LocusMember::Lifecycle(lc)
                if matches!(lc.kind, LifecycleKind::Accept))
        });
        let projection_class: ProjectionClass = l
            .annotations
            .iter()
            .find_map(|a| match a {
                LocusAnnotation::Projection(pc) => Some(*pc),
                _ => None,
            })
            .unwrap_or(if has_accept {
                ProjectionClass::Chunked
            } else {
                ProjectionClass::Rich
            });

        // m25: schedule class. Default cooperative — even though
        // current codegen runs everything synchronously (which is
        // structurally greedy), cooperative is the spec's default
        // and the natural target for m26. Users who want today's
        // sync-everywhere semantics LOCKED IN as their design
        // choice (rather than incidental) can write
        // `: schedule greedy` explicitly.
        // F.31 (2026-05-23): per-locus schedule annotation is
        // removed. The locus's default placement is Cooperative;
        // Phase 3 of the F.31 substrate work (pending) reads the
        // main locus's `placement { }` block and overrides this
        // per main-locus params field. Until that lands, every
        // locus defaults to Cooperative — equivalent to today's
        // unannotated behavior. Pinned-via-placement requires
        // Phase 3.
        let schedule_class: ScheduleClass = ScheduleClass::Cooperative;

        // Each locus param must have either a literal default or
        // a typed default expression evaluable at instantiation
        // time. Scalar literals lock in `DefaultInit::Const` so
        // const_param can build them directly; non-literal defaults
        // (like `current_kernel: Kernel = Kernel { ... }`)
        // get `DefaultInit::Expr` and are evaluated at the
        // instantiation site through lower_expr. Type ascription is
        // REQUIRED for non-literal defaults (we don't infer a type
        // from an arbitrary expression here — the AST resolver
        // doesn't run in codegen v0).
        let mut fields: BTreeMap<String, (u32, CodegenTy)> = BTreeMap::new();
        let mut defaults: Vec<(String, DefaultInit)> = Vec::new();
        let mut llvm_field_tys: Vec<inkwell::types::BasicTypeEnum> =
            Vec::new();

        // Synthetic `__arena: ptr` is *always* the first field
        // (index 0). m20+ allocations on behalf of a locus go to
        // this arena; bus dispatch's payload-copy step pulls it
        // out of the subscriber's self_ptr at runtime via a
        // fixed-offset GEP. Keeping it at idx 0 means the dispatch
        // fn doesn't need to know the subscriber's specific locus
        // type to find the arena.
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        llvm_field_tys.push(ptr_t.into());
        let arena_field_idx: u32 = 0;
        let mut idx: u32 = 1;

        for member in &l.members {
            if let LocusMember::Params(pb) = member {
                for p in &pb.params {
                    // B7 / G19: when the param's ascribed type is
                    // a user-defined struct whose every field has
                    // a default, synthesize `T { }` as the param's
                    // own default. This drops the `=` requirement
                    // for the common "wrap an all-defaulted record
                    // in a locus param" pattern (e.g. config
                    // bundles, settings structs).
                    let synthesized_struct_default: Option<Expr> =
                        if matches!(&p.init, ParamInit::Inferred) {
                            p.ty.as_ref().and_then(|te| {
                                let TypeExpr::Named { path, generic_args, .. } = te
                                else { return None };
                                if !generic_args.is_empty() {
                                    return None;
                                }
                                if path.segments.len() != 1 {
                                    return None;
                                }
                                let name = &path.segments[0].name;
                                let info = self.user_types.get(name)?;
                                if info.field_order.is_empty() {
                                    return None;
                                }
                                if !info
                                    .field_order
                                    .iter()
                                    .all(|f| info.defaults.contains_key(f))
                                {
                                    return None;
                                }
                                Some(Expr::Struct {
                                    path: path.clone(),
                                    inits: Vec::new(),
                                    span: p.name.span,
                                })
                            })
                        } else {
                            None
                        };
                    let default_expr = match &p.init {
                        ParamInit::Value(e) => Some(e),
                        ParamInit::Inferred => {
                            if let Some(synth) = synthesized_struct_default.as_ref() {
                                Some(synth)
                            } else if p.ty.is_none() {
                                // 2026-05-16 — `name: T;` with no `=`
                                // declares a required param. The
                                // user must supply it at
                                // instantiation time; codegen
                                // accepts the param with an
                                // ascribed type and no default.
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` param `{}`: required \
                                     params need an explicit type \
                                     ascription (`name: T;`)",
                                    l.name.name, p.name.name
                                )));
                            } else {
                                None
                            }
                        }
                    };
                    let default_expr = match default_expr {
                        Some(e) => e,
                        None => {
                            let ascribed = p.ty.as_ref().expect("required param has ty");
                            let ty = self.type_expr_to_codegen_ty(ascribed)?;
                            fields.insert(p.name.name.clone(), (idx, ty.clone()));
                            defaults.push((p.name.name.clone(), DefaultInit::Required));
                            // Inline fixed arrays (array_inline_spec).
                            llvm_field_tys.push(self.llvm_field_storage_type(&ty));
                            idx += 1;
                            continue;
                        }
                    };
                    // Try to lock in as a literal Const first; fall
                    // back to deferred Expr if that fails.
                    let (default, default_ty): (DefaultInit, CodegenTy) =
                        match param_value(default_expr) {
                            Ok(pv) => {
                                let ty = match &pv {
                                    ParamValue::Int(_) => CodegenTy::Int,
                                    ParamValue::Float(_) => CodegenTy::Float,
                                    ParamValue::Bool(_) => CodegenTy::Bool,
                                    ParamValue::String(_) => CodegenTy::String,
                                    ParamValue::Duration(_) => {
                                        CodegenTy::Duration
                                    }
                                    ParamValue::Decimal(_) => CodegenTy::Decimal,
                                    ParamValue::Time(_) => CodegenTy::Time,
                                };
                                (DefaultInit::Const(pv), ty)
                            }
                            Err(_) => {
                                // Non-literal default → require an
                                // explicit type ascription so we
                                // know the field's LLVM shape
                                // without evaluating the default.
                                let ascribed = p.ty.as_ref().ok_or_else(|| {
                                    CodegenError::Unsupported(format!(
                                        "locus `{}` param `{}`: non-literal \
                                         default requires a type ascription",
                                        l.name.name, p.name.name
                                    ))
                                })?;
                                let ty = self.type_expr_to_codegen_ty(ascribed)?;
                                // m61c: bare-name struct literal in
                                // a typed param default rewrites to
                                // the mangled monomorph at decl
                                // time, so the deferred lower_expr
                                // at instantiation sees the right
                                // path. Mirrors the let-ascription
                                // hook from m61b.
                                let stored_default =
                                    match default_expr {
                                        Expr::Struct {
                                            path,
                                            inits,
                                            span,
                                        } => match self
                                            .resolve_generic_struct_path(
                                                path, ascribed,
                                            ) {
                                            Some(new_path) => {
                                                Expr::Struct {
                                                    path: new_path,
                                                    inits: inits.clone(),
                                                    span: *span,
                                                }
                                            }
                                            None => default_expr.clone(),
                                        },
                                        _ => default_expr.clone(),
                                    };
                                (DefaultInit::Expr(stored_default), ty)
                            }
                        };
                    // F.30b (5b): if the param is ascribed
                    // StringView/BytesView and the default is a
                    // String/Bytes literal, accept the mismatch and
                    // use the ascribed (view) type as the field's
                    // declared type. The storage-site wrap in
                    // lower_locus_instantiation converts the literal
                    // to a view at construction time.
                    let default_ty = if let Some(ascribed) = &p.ty {
                        let asc_ty = self.type_expr_to_codegen_ty(ascribed)?;
                        let literal_to_view = matches!(
                            (&default_ty, &asc_ty),
                            (CodegenTy::String, CodegenTy::StringView)
                                | (CodegenTy::Bytes, CodegenTy::BytesView)
                        );
                        if asc_ty != default_ty && !literal_to_view {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` param `{}`: declared {:?}, \
                                 default {:?}",
                                l.name.name, p.name.name, asc_ty, default_ty
                            )));
                        }
                        // Promote default_ty to the ascribed View
                        // type when the literal coercion applies, so
                        // downstream reads see the View type, not
                        // the literal's source type.
                        if literal_to_view { asc_ty } else { default_ty }
                    } else {
                        default_ty
                    };
                    fields.insert(
                        p.name.name.clone(),
                        (idx, default_ty.clone()),
                    );
                    defaults.push((p.name.name.clone(), default));
                    // Inline fixed arrays (array_inline_spec).
                    llvm_field_tys.push(self.llvm_field_storage_type(&default_ty));
                    idx += 1;
                }
            }
        }

        // F.32-1b (2026-05-25): snapshot the half-open range of
        // user-param fields in llvm_field_tys. [1, user_fields_end)
        // is the slice we may permute later by method-body access
        // frequency. arena lives at idx 0; capacity slots +
        // synthetic flags get pushed after this point and aren't
        // candidates for reorder.
        let user_fields_start_idx: u32 = 1;
        let user_fields_end_idx: u32 = idx;

        // If this locus declares accept AND any method body
        // iterates `for child in self.children`, append a
        // synthetic children array + counter at the end of the
        // struct so each accept dispatch can record the child's
        // self_ptr.
        //
        // When `accept` is declared but no body reads
        // `self.children`, we elide the storage entirely. The
        // append at accept-time then becomes a no-op (the
        // `children_field_idx` Option below gates it).
        //
        // For loci that DO iterate, the storage is a growable
        // heap buffer (2026-05-29): a `__children` pointer to a
        // `void**` buffer plus `__child_count` / `__child_cap`
        // i64 fields. `lotus_children_push` grows it on demand.
        // This replaced a fixed `[16]` inline array whose
        // unchecked accept-time append silently corrupted
        // adjacent struct memory once a parent accepted more than
        // 16 children (the bench surfaced it at k≈25) — fatal for
        // the daemon-server pattern that accepts one child per
        // connection.
        let uses_children = has_accept && locus_reads_self_children(l);
        let (children_field_idx, child_count_field_idx, child_cap_field_idx) =
            if uses_children {
                let i64_t = self.context.i64_type();
                // __children: ptr to a heap void** buffer.
                let arr_idx = idx;
                llvm_field_tys.push(ptr_t.into());
                idx += 1;
                let cnt_idx = idx;
                llvm_field_tys.push(i64_t.into());
                idx += 1;
                let cap_idx = idx;
                llvm_field_tys.push(i64_t.into());
                idx += 1;
                (Some(arr_idx), Some(cnt_idx), Some(cap_idx))
            } else {
                (None, None, None)
            };

        // m28b stage 2: pinned-class loci that declare bus
        // subscriptions get a synthetic `__mailbox: ptr` field.
        // The mailbox is allocated at instantiation and stored in
        // this slot so all three sites that need it can reach it
        // via self_ptr: subscribe registration (main thread),
        // synthesized thread_main's mailbox loop (pinned thread),
        // and the deferred-dissolve flush (main thread, signals
        // shutdown before pthread_join). Cooperative loci and
        // pinned loci without subscriptions don't need this.
        //
        // F.31 (2026-05-23): "is this locus pinned?" no longer
        // comes from a per-locus annotation. We consult
        // `pinned_locus_types`, populated at codegen startup
        // from main's `placement { }` entries + adapter
        // bindings. Any locus that's instantiated pinned-
        // equivalent anywhere in the bundle gets pinned-fields
        // in its struct layout. Per-instance core affinity
        // varies via the placement_override at instantiation
        // time; struct layout is type-level uniform.
        let is_pinned_locus_type =
            self.pinned_locus_types.contains(&l.name.name);
        let has_subscribe = is_pinned_locus_type
            && l.members.iter().any(|m| match m {
                LocusMember::Bus(b) => b.members.iter().any(|bm| {
                    matches!(bm, BusMember::Subscribe { .. })
                }),
                _ => false,
            });
        let mailbox_field_idx = if has_subscribe {
            let i = idx;
            llvm_field_tys.push(ptr_t.into());
            idx += 1;
            Some(i)
        } else {
            None
        };

        // m40: synthetic `__restart_count: i64` field, always
        // appended to every locus struct. Zero-initialized at
        // instantiation; bumped by the `restart(child)` recovery
        // primitive when the parent's on_failure handler asks
        // for a retry. The default cap is 2 attempts per locus
        // lifetime — past that, restart() no-ops and the
        // violation falls through to the parent's collapse path.
        // Always-present so the runtime check after on_failure
        // doesn't need to branch on whether the locus opted in.
        let i64_t_struct = self.context.i64_type();
        let restart_count_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // m41: synthetic `__quarantined: i64` flag, always
        // appended to every locus struct. Zero-initialized at
        // instantiation; set to 1 by the `quarantine(child)`
        // recovery primitive. The post-`__birth_closures`
        // dispatch in lower_locus_instantiation reads it and
        // skips `run()` if set. Drain / dissolve still fire
        // (those are cleanup, unconditional). Bus dispatch
        // gating waits on a C-runtime change with a fixed-offset
        // load — for now, quarantined loci still receive bus
        // messages but don't enter run().
        let quarantined_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // m45: synthetic __restart_in_place_pending flag —
        // distinguishes a pending `restart_in_place` re-run
        // (zero fields first) from a plain `restart` re-run
        // (state preserved). Zero-init at instantiation.
        let restart_in_place_pending_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // v1.x-VIOLATE (F.27): synthetic __drain_requested flag.
        // Zero-initialized at instantiation; set by `violate
        // NAME;` lowering when the time comes; read by
        // `self.draining` from user code. Always present so the
        // synthetic field surface stays uniform across loci with
        // or without inline closures.
        let drain_requested_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // v1.x-4b: synthetic __slot_borrowed_mask — bit N is set
        // iff the Nth capacity slot was borrowed from a parent
        // via `as_parent_for`. Zero-init at instantiation; OR-in
        // happens during slot init when a borrow swap occurs;
        // read at dissolve to skip the destroy call on borrowed
        // slots. Always present so the dissolve loop doesn't
        // need to branch on whether the locus opted into the
        // borrow surface.
        let slot_borrowed_mask_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // F.29 follow-up (2026-05-19): synthetic
        // __locus_ref_owned_mask — bit N is set iff the Nth
        // LocusRef-typed param field (in declaration order) was
        // initialized via a locus literal at this instantiation
        // (parent-owned), vs. an external override or non-locus
        // expression (parent does not own). The cascade helpers
        // `emit_locus_field_drains` / `emit_locus_field_dissolves`
        // branch on the bit and skip externally-provided children,
        // closing the double-dissolve regression where
        // `Pub { sub: external_s }` would otherwise have the
        // cascade tear down `external_s` while its real owner is
        // still alive. Cap of 64 LocusRef fields per locus is well
        // above any practical ceiling. Always present so the
        // cascade emission stays uniform.
        let locus_ref_owned_mask_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // v1.x-3: synthetic `__recpool: ptr` — parent-side recpool
        // handle (set only when this locus is Recognition class
        // with fixed_cell or shared_slab sub-mode; zero otherwise).
        // Uniform layout so the dissolve path can read it without
        // branching on the locus's class.
        let recpool_field_idx = idx;
        llvm_field_tys.push(self.context.ptr_type(AddressSpace::default()).into());
        idx += 1;
        // v1.x-3: synthetic `__recpool_release_pool: ptr` —
        // child-side back-reference to a recognition parent's
        // recpool. Set when the parent's projection class is
        // Recognition with a shipped sub-mode and we're being
        // accepted by it; zero otherwise.
        let recpool_release_pool_field_idx = idx;
        llvm_field_tys.push(self.context.ptr_type(AddressSpace::default()).into());
        idx += 1;
        // v1.x-3: synthetic `__recpool_release_kind: i64`
        // discriminator — 0=regular arena_destroy, 1=fixed_cell
        // release, 2=shared_slab release. Set at accept-by-recognition
        // parent; consumed in emit_locus_arena_destroy.
        let recpool_release_kind_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // m42: synthetic `__parent_self: ptr` and
        // `__parent_on_failure: ptr` fields. Always present
        // (uniform struct shape — the alternative would be
        // conditional layout that complicates bus dispatch's
        // type-erased self_ptr access). Set at instantiation
        // time from `resolve_failure_route`; read by
        // `__tick_closures_wrapper` when firing tick-epoch
        // closures from a non-static call site (the bus
        // drain). Cost: 16 bytes of overhead per locus
        // instance — negligible vs. the closure-routing
        // capability they unlock.
        let ptr_field_t = self.context.ptr_type(AddressSpace::default());
        let parent_self_field_idx = idx;
        llvm_field_tys.push(ptr_field_t.into());
        idx += 1;
        let parent_on_failure_field_idx = idx;
        llvm_field_tys.push(ptr_field_t.into());
        idx += 1;
        // 2026-05-30: synthetic `__owner_self: ptr` — the self_ptr of
        // the parent that accept'd this locus, stored UNCONDITIONALLY
        // at accept dispatch (unlike `__parent_self`, which is set
        // only when the parent has a matching on_failure handler).
        // Read by a flow child's run-wrapper to fire `parent.release(
        // owner, child)` when the child completes. Null for loci that
        // were never accept'd.
        let owner_self_field_idx = idx;
        llvm_field_tys.push(ptr_field_t.into());
        idx += 1;
        // Interest-based ownership, artifact #2b: append one
        // `__owner_for_<I>: ptr` field per interest-type `I` this locus
        // must forward (its entry in the whole-program forwarding sets).
        // Only carrying loci get fields — keeps every other struct lean.
        // Deterministic layout: the forwarding set is a BTreeSet, so the
        // fields append in sorted interest-type order. Threaded at birth
        // (3-way write in lower_locus_instantiation); read at the
        // non-singleton bubble seam. Empty under
        // `LOTUS_NO_OWNERSHIP_BUBBLE=1` (forwarding sets emptied).
        let mut owner_forward_field_idxs: BTreeMap<String, u32> =
            BTreeMap::new();
        if let Some(interests) =
            self.ownership_forwarding_sets.get(&l.name.name).cloned()
        {
            for interest in &interests {
                owner_forward_field_idxs.insert(interest.clone(), idx);
                llvm_field_tys.push(ptr_field_t.into());
                idx += 1;
            }
        }
        // m43: append one i64 __duration_last_fire field per
        // duration-epoch closure on this locus (in declaration
        // order). Init at instantiation to time::monotonic()
        // so the first fire happens after `N` elapses.
        let mut duration_last_fire_field_idxs: Vec<u32> = Vec::new();
        for member in &l.members {
            if let LocusMember::Closure(c) = member {
                let is_duration = c.clauses.iter().any(|cl| {
                    matches!(cl, ClosureClause::Epoch(EpochSpec::Duration(_)))
                });
                if is_duration {
                    duration_last_fire_field_idxs.push(idx);
                    llvm_field_tys.push(i64_t_struct.into());
                    idx += 1;
                }
            }
        }

        // m46: closure accumulators. For each `sum(expr)` call
        // detected in a closure's assertion (left/right/tolerance,
        // in that order), append one struct field of `expr`'s type.
        // v0 restricts inner exprs to `self.X` reads — type comes
        // straight from the locus's params. Anything else errors
        // with a clear message at struct-decl time. Per-closure
        // persists_through clauses are also stashed here for the
        // recovery-reset gating.
        let mut accumulators_per_closure: BTreeMap<
            String,
            Vec<AccumulatorSlot>,
        > = BTreeMap::new();
        let mut persists_through_per_closure: BTreeMap<String, Vec<String>> =
            BTreeMap::new();
        // v1.x-WINDOWED (F.34): per-closure list of field names to
        // zero at each `duration(N)` epoch fire. The runtime hook
        // in __duration_closures consumes this map.
        let mut resets_per_epoch_per_closure: BTreeMap<String, Vec<String>> =
            BTreeMap::new();
        for member in &l.members {
            let LocusMember::Closure(c) = member else {
                continue;
            };
            let mut accs: Vec<(AccumulatorKind, Option<Expr>)> = Vec::new();
            // v1.x-VIOLATE (F.27): assertion-less inline closures
            // have no accumulator-bearing exprs.
            if let Some(a) = &c.assertion {
                collect_sum_calls(&a.left, &mut accs);
                collect_sum_calls(&a.right, &mut accs);
                collect_sum_calls(&a.tolerance, &mut accs);
            }
            let mut slots: Vec<AccumulatorSlot> = Vec::new();
            for (kind, inner_opt) in accs {
                match kind {
                    AccumulatorKind::Sum => {
                        let inner = inner_opt.expect("sum carries inner");
                        let inner_ty = infer_accumulator_inner_type(
                            &l.name.name,
                            &c.name.name,
                            &inner,
                            &fields,
                        )?;
                        let llvm_ty: inkwell::types::BasicTypeEnum =
                            self.llvm_basic_type(&inner_ty);
                        llvm_field_tys.push(llvm_ty);
                        let slot_idx = idx;
                        idx += 1;
                        slots.push(AccumulatorSlot {
                            kind: AccumulatorKind::Sum,
                            inner_expr: Some(inner),
                            ty: inner_ty.clone(),
                            inner_ty,
                            field_idx: slot_idx,
                            field_idx_2: None,
                        });
                    }
                    AccumulatorKind::Count => {
                        // One i64 slot. Inner expr = none; output = Int.
                        llvm_field_tys.push(i64_t_struct.into());
                        let slot_idx = idx;
                        idx += 1;
                        slots.push(AccumulatorSlot {
                            kind: AccumulatorKind::Count,
                            inner_expr: None,
                            ty: CodegenTy::Int,
                            inner_ty: CodegenTy::Int,
                            field_idx: slot_idx,
                            field_idx_2: None,
                        });
                    }
                    AccumulatorKind::Mean => {
                        // Two slots: running sum (inner's type) +
                        // count (i64). Output is always Float.
                        let inner = inner_opt.expect("mean carries inner");
                        let inner_ty = infer_accumulator_inner_type(
                            &l.name.name,
                            &c.name.name,
                            &inner,
                            &fields,
                        )?;
                        let llvm_inner: inkwell::types::BasicTypeEnum =
                            self.llvm_basic_type(&inner_ty);
                        llvm_field_tys.push(llvm_inner);
                        let sum_idx = idx;
                        idx += 1;
                        llvm_field_tys.push(i64_t_struct.into());
                        let count_idx = idx;
                        idx += 1;
                        slots.push(AccumulatorSlot {
                            kind: AccumulatorKind::Mean,
                            inner_expr: Some(inner),
                            ty: CodegenTy::Float,
                            inner_ty,
                            field_idx: sum_idx,
                            field_idx_2: Some(count_idx),
                        });
                    }
                }
            }
            if !slots.is_empty() {
                accumulators_per_closure
                    .insert(c.name.name.clone(), slots);
            }
            let mut persists: Vec<String> = Vec::new();
            for clause in &c.clauses {
                if let ClosureClause::PersistsThrough(events) = clause {
                    for ev in events {
                        persists.push(ev.name.clone());
                    }
                }
            }
            if !persists.is_empty() {
                persists_through_per_closure
                    .insert(c.name.name.clone(), persists);
            }
            let mut resets_pe: Vec<String> = Vec::new();
            for clause in &c.clauses {
                if let ClosureClause::ResetsPerEpoch(fields) = clause {
                    for f in fields {
                        resets_pe.push(f.name.clone());
                    }
                }
            }
            if !resets_pe.is_empty() {
                resets_per_epoch_per_closure
                    .insert(c.name.name.clone(), resets_pe);
            }
        }

        // F.22 capacity slots: walk every `capacity { ... }` block
        // on this locus and append one slot field per declared slot.
        // Order is declaration order across all capacity blocks
        // (concatenated). Per-slot create/destroy live in
        // lower_locus_instantiation and emit_locus_arena_destroy.
        //
        // Default lowering: the field is a `ptr` holding the
        // allocator pointer (lotus_pool_t* or lotus_heap_t*).
        //
        // v1.x-FORM-2: if the locus carries `@form(vec)`, the heap
        // slot becomes an inline `{ i64 cap, i64 len, ptr buf }`
        // struct managed by the lotus_vec_* C runtime. No separate
        // allocator pointer; the substrate lives in place.
        //
        // Restriction 1 (locus cell rejection) is also enforced
        // here at codegen — typecheck duplicates the check for
        // better diagnostics, but routing the rejection through
        // codegen catches the case where typecheck is bypassed
        // (e.g. internal-test paths) AND grounds the error in
        // the same CodegenTy world the rest of codegen reasons
        // in.
        let is_form_vec = l
            .form
            .as_ref()
            .map(|f| f.name.name.as_str() == "vec")
            .unwrap_or(false);
        let is_form_hashmap = l
            .form
            .as_ref()
            .map(|f| f.name.name.as_str() == "hashmap")
            .unwrap_or(false);
        let is_form_ring_buffer = l
            .form
            .as_ref()
            .map(|f| f.name.name.as_str() == "ring_buffer")
            .unwrap_or(false);
        let mut capacity_slots: Vec<CapacitySlotLayout> = Vec::new();
        let mut seen_slot_names: BTreeSet<String> = BTreeSet::new();
        for member in &l.members {
            let LocusMember::Capacity(cb) = member else {
                continue;
            };
            for slot in &cb.slots {
                if !seen_slot_names.insert(slot.name.name.clone()) {
                    return Err(CodegenError::Unsupported(format!(
                        "locus `{}`: duplicate capacity slot `{}`",
                        l.name.name, slot.name.name
                    )));
                }
                let elem_ty = self.type_expr_to_codegen_ty(&slot.elem_ty)?;
                if matches!(elem_ty, CodegenTy::LocusRef(_)) {
                    return Err(CodegenError::Unsupported(format!(
                        "locus `{}`: capacity slot `{}` cell type is a \
                         locus reference; F.22 restriction 1 rejects \
                         locus-typed cells — route locus membership \
                         through `accept(c: ...)` instead",
                        l.name.name, slot.name.name
                    )));
                }
                // F.22 v1.x-4 (surface) + v1.x-4b (runtime):
                // `as_parent_for ChildL` shipped end-to-end. The
                // surface (parser + typecheck) was wired in
                // v1.x-4; the runtime mechanic — copy the parent's
                // allocator pointer into the child's same-named
                // slot at child instantiation, OR the bit in the
                // child's __slot_borrowed_mask, skip-destroy in
                // the child's dissolve — is handled by the slot-
                // init + slot-destroy loops below. The
                // declaration's `as_parent_for` field flows into
                // `CapacitySlotLayout.as_parent_for` so the child
                // instantiation site can detect a borrowable
                // parent-slot by name + kind + elem_ty.
                let slot_field_idx = idx;
                let form = if is_form_vec
                    && matches!(slot.kind, CapacitySlotKind::Heap)
                {
                    // @form(vec): emit the inline { cap, len, buf }
                    // struct instead of an allocator-pointer field.
                    // Typecheck (PR3a) already verified exactly one
                    // heap slot exists when @form(vec) is in play.
                    let vec_struct_ty = self.context.struct_type(
                        &[i64_t.into(), i64_t.into(), ptr_t.into()],
                        false,
                    );
                    llvm_field_tys.push(vec_struct_ty.into());
                    Some(SlotForm::Vec)
                } else if is_form_ring_buffer
                    && matches!(slot.kind, CapacitySlotKind::Pool)
                {
                    // @form(ring_buffer): inline { cap, head, len,
                    // elem_size, buf } struct, init at locus birth
                    // with the cap from the annotation arg. Layout
                    // matches the C-side `lotus_ring_buffer_t`
                    // exactly. Typecheck verifies the pool-slot
                    // shape and cap presence.
                    let rb_struct_ty = self.context.struct_type(
                        &[
                            i64_t.into(), // cap
                            i64_t.into(), // head
                            i64_t.into(), // len
                            i64_t.into(), // elem_size
                            ptr_t.into(), // buf
                        ],
                        false,
                    );
                    llvm_field_tys.push(rb_struct_ty.into());
                    Some(SlotForm::RingBuffer)
                } else if is_form_hashmap
                    && matches!(slot.kind, CapacitySlotKind::Pool)
                {
                    // v1.x-FORM-4: @form(hashmap): emit the inline
                    // lotus_hashmap_t-shaped struct instead of an
                    // allocator-pointer field. Layout matches the
                    // C-side struct exactly — LLVM naturally inserts
                    // the 4-byte pad between i32 key_type_tag and
                    // ptr slots for alignment. Typecheck (FORM-4 PR2)
                    // already verified exactly one pool slot with
                    // indexed_by exists when @form(hashmap) is in
                    // play.
                    //
                    // F.32-1α/β2 (2026-05-24 / 2026-05-25): trailing
                    // fields encode the sync-discipline state.
                    //   sync_mode: i32     — 0=NONE, 1=SERIALIZED, 2=STRIPED
                    //   mu: ptr            — pthread_mutex_t* (SERIALIZED only)
                    //   mu_grow: ptr       — pthread_rwlock_t* (STRIPED only)
                    //   cell_stride: i64   — padded cell size (β2: round up to LOTUS_CACHE_LINE)
                    //   cursor_i: i64      — monotonic-iteration cursor (2026-05-25 fix)
                    //   cursor_slot: i64   — cursor's slot index
                    //   tombstone_count: i64 — lockfree tombstones (F.32-1γ-v2 session 1)
                    //   lf_grow_phase: i32 — lockfree grow phase 0/1 (F.32-1γ-v2 session 3)
                    //   lf_writers_in_flight: i64 — in-flight lockfree op count
                    //
                    // Plain @form(hashmap) zeros sync_mode / mu /
                    // mu_grow at init; sets cell_stride to the
                    // packed value. SERIALIZED + STRIPED variants
                    // run their respective init to set everything.
                    let i32_t = self.context.i32_type();
                    let hashmap_struct_ty = self.context.struct_type(
                        &[
                            i64_t.into(),  // cap
                            i64_t.into(),  // len
                            i64_t.into(),  // key_size
                            i64_t.into(),  // value_size
                            i32_t.into(),  // key_type_tag
                            ptr_t.into(),  // slots (4-byte pad inserted before)
                            i32_t.into(),  // sync_mode (F.32-1α; β2 widens semantics)
                            ptr_t.into(),  // mu (F.32-1α; 4-byte pad inserted before)
                            ptr_t.into(),  // mu_grow (F.32-1β2)
                            i64_t.into(),  // cell_stride (F.32-1β2)
                            i64_t.into(),  // cursor_i (2026-05-25)
                            i64_t.into(),  // cursor_slot (2026-05-25)
                            i64_t.into(),  // tombstone_count (F.32-1γ-v2 session 1)
                            i32_t.into(),  // lf_grow_phase (F.32-1γ-v2 session 3)
                            i64_t.into(),  // lf_writers_in_flight (4-byte pad inserted before)
                        ],
                        false,
                    );
                    llvm_field_tys.push(hashmap_struct_ty.into());
                    Some(SlotForm::Hashmap)
                } else {
                    llvm_field_tys.push(ptr_t.into());
                    None
                };
                idx += 1;
                let ring_buffer_cap = if matches!(form, Some(SlotForm::RingBuffer)) {
                    // Extract `cap = N` from the form annotation
                    // args. Typecheck guarantees presence + valid
                    // form on @form(ring_buffer); codegen reads
                    // the int literal directly.
                    l.form
                        .as_ref()
                        .and_then(|f| {
                            f.args.iter().find(|a| a.name.name == "cap")
                        })
                        .and_then(|a| match &a.value {
                            Expr::Literal(Literal::Int(n), _) if *n > 0 => {
                                Some(*n as u64)
                            }
                            _ => None,
                        })
                } else {
                    None
                };
                // F.32-1α (2026-05-24): read the @form(hashmap)
                // `sync = X` kwarg if present. Typecheck has
                // already validated the value shape; codegen
                // just maps the recognized identifier to its
                // SyncMode variant. Unrecognized / absent →
                // SyncMode::None (single-pool, no runtime sync).
                let sync_mode = if matches!(form, Some(SlotForm::Hashmap)) {
                    let sync_name = l.form
                        .as_ref()
                        .and_then(|f| {
                            f.args.iter().find(|a| a.name.name == "sync")
                        })
                        .and_then(|a| match &a.value {
                            Expr::Ident(i) => Some(i.name.as_str().to_string()),
                            _ => None,
                        });
                    match sync_name.as_deref() {
                        Some("serialized") => SyncMode::Serialized,
                        Some("striped") => SyncMode::Striped,
                        Some("lockfree") => {
                            // F.32-1γ-v1: lockfree requires
                            // `cap = N` (validated by typecheck;
                            // codegen reads the int literal).
                            let cap = l.form
                                .as_ref()
                                .and_then(|f| {
                                    f.args.iter().find(|a| a.name.name == "cap")
                                })
                                .and_then(|a| match &a.value {
                                    Expr::Literal(Literal::Int(n), _) if *n > 0 => {
                                        Some(*n as u64)
                                    }
                                    _ => None,
                                })
                                .unwrap_or(0);
                            SyncMode::Lockfree { fixed_cap: cap }
                        }
                        _ => SyncMode::None,
                    }
                } else {
                    SyncMode::None
                };
                capacity_slots.push(CapacitySlotLayout {
                    name: slot.name.name.clone(),
                    kind: slot.kind,
                    elem_ty,
                    as_parent_for: slot
                        .as_parent_for
                        .as_ref()
                        .map(|i| i.name.clone()),
                    struct_field_idx: slot_field_idx,
                    form,
                    indexed_by: slot
                        .indexed_by
                        .as_ref()
                        .map(|i| i.name.clone()),
                    ring_buffer_cap,
                    sync_mode,
                });
            }
        }
        let _ = idx;

        // F.32-1b (2026-05-25): reorder user-param fields by
        // method-body access frequency so high-access fields
        // land near the front of the struct (first cache line
        // after the synthetic header). Capacity slots, accept-
        // children buffer, mailbox slot, synthetic flags
        // (restart_count / quarantined / drain_requested / ...)
        // all stay put — they have ABI-significant fixed
        // positions and are touched per-substrate-call, not
        // per-method-call.
        //
        // Only the half-open slice [user_fields_start_idx,
        // user_fields_end_idx) of llvm_field_tys is permuted.
        // The `fields` lookup map is updated to match; `defaults`
        // stays in declaration order (defaults evaluate in source
        // order at instantiation, independent of struct layout).
        if user_fields_end_idx > user_fields_start_idx + 1 {
            let user_count = (user_fields_end_idx - user_fields_start_idx) as usize;
            // Build list of (name, old_idx, decl_order) for user fields.
            let decl_order_by_name: BTreeMap<String, u32> = defaults
                .iter()
                .enumerate()
                .map(|(i, (n, _))| (n.clone(), i as u32))
                .collect();
            let mut user_field_names: Vec<(String, u32)> = fields
                .iter()
                .filter(|(_, (i, _))| {
                    *i >= user_fields_start_idx && *i < user_fields_end_idx
                })
                .map(|(n, (i, _))| (n.clone(), *i))
                .collect();
            // Sort by (access_count desc, decl_order asc). Stable
            // ordering across builds is critical — two compiles of
            // the same source must produce the same struct layout.
            let access_counts = count_self_field_accesses_in_locus(l);
            user_field_names.sort_by(|(a_n, _), (b_n, _)| {
                let a_count = access_counts.get(a_n).copied().unwrap_or(0);
                let b_count = access_counts.get(b_n).copied().unwrap_or(0);
                let a_decl = decl_order_by_name.get(a_n).copied().unwrap_or(u32::MAX);
                let b_decl = decl_order_by_name.get(b_n).copied().unwrap_or(u32::MAX);
                b_count.cmp(&a_count).then(a_decl.cmp(&b_decl))
            });
            // Apply permutation. We need:
            //   - a copy of the user portion of llvm_field_tys for source
            //   - a map from old_idx → new_idx so we can update fields
            let old_user_portion: Vec<inkwell::types::BasicTypeEnum> =
                llvm_field_tys[(user_fields_start_idx as usize)
                    ..(user_fields_end_idx as usize)]
                    .to_vec();
            let mut old_idx_to_new_idx: Vec<u32> = vec![0u32; user_count];
            for (new_local, (_, old_idx)) in user_field_names.iter().enumerate() {
                let old_local = *old_idx - user_fields_start_idx;
                let new_global = user_fields_start_idx + new_local as u32;
                old_idx_to_new_idx[old_local as usize] = new_global;
                // Place this field's LLVM type at its new global index.
                llvm_field_tys[new_global as usize] =
                    old_user_portion[old_local as usize];
            }
            // Update the `fields` lookup map's indices for user-field entries.
            for (_, (idx_ref, _)) in fields.iter_mut() {
                if *idx_ref >= user_fields_start_idx && *idx_ref < user_fields_end_idx {
                    let old_local = (*idx_ref - user_fields_start_idx) as usize;
                    *idx_ref = old_idx_to_new_idx[old_local];
                }
            }
        }

        let struct_ty = self
            .context
            .opaque_struct_type(&format!("locus.{}", l.name.name));
        struct_ty.set_body(&llvm_field_tys, false);

        // F.29 follow-up: assign bit positions for LocusRef-typed
        // param fields in declaration order. The cascade emitters
        // use these bits to discriminate parent-owned children
        // (default-init OR locus-literal override) from externally
        // provided ones (variable-ref override) — the latter
        // are skipped by the cascade so they don't get
        // double-dissolved by the parent's teardown alongside
        // their real owner's teardown. `defaults` is in
        // declaration order; we filter for fields whose codegen
        // type is `LocusRef`.
        let mut locus_ref_bit_per_field: BTreeMap<String, u32> =
            BTreeMap::new();
        let mut next_bit: u32 = 0;
        for (fname, _) in defaults.iter() {
            if let Some((_, ty)) = fields.get(fname) {
                if matches!(ty, CodegenTy::LocusRef(_)) {
                    locus_ref_bit_per_field.insert(fname.clone(), next_bit);
                    next_bit += 1;
                }
            }
        }
        if next_bit > 64 {
            return Err(CodegenError::Unsupported(format!(
                "locus `{}` declares more than 64 LocusRef-typed \
                 param fields ({}); the `__locus_ref_owned_mask` \
                 bitmask only carries 64 bits",
                l.name.name, next_bit
            )));
        }

        self.user_loci.insert(
            l.name.name.clone(),
            LocusInfo {
                struct_ty,
                fields,
                defaults,
                methods: BTreeMap::new(),
                accept_param: None,
                release_param: None,
                user_methods: BTreeMap::new(),
                subscriptions: Vec::new(),
                batch_handlers: std::collections::BTreeSet::new(),
                closures: Vec::new(),
                accumulators_per_closure,
                persists_through_per_closure,
                resets_per_epoch_per_closure,
                birth_closures_fn: None,
                dissolve_closures_fn: None,
                tick_closures_fn: None,
                tick_wrapper_fn: None,
                duration_closures_fn: None,
                duration_wrapper_fn: None,
                duration_last_fire_field_idxs,
                explicit_closures_fn: None,
                failure_handler: None,
                children_field_idx,
                child_count_field_idx,
                child_cap_field_idx,
                arena_field_idx,
                restart_count_field_idx,
                quarantined_field_idx,
                restart_in_place_pending_field_idx,
                drain_requested_field_idx,
                slot_borrowed_mask_field_idx,
                locus_ref_owned_mask_field_idx,
                locus_ref_bit_per_field,
                recpool_field_idx,
                recpool_release_pool_field_idx,
                recpool_release_kind_field_idx,
                parent_self_field_idx,
                owner_self_field_idx,
                owner_forward_field_idxs,
                parent_on_failure_field_idx,
                mailbox_field_idx,
                projection_class,
                schedule_class,
                capacity_slots,
                arena_elidable: locus_arena_elidable(l),
                empty_lifecycle: std::collections::BTreeSet::new(),
            },
        );
        Ok(())
    }

    /// Pass A2: declare each lifecycle method's LLVM function
    /// signature. Runs after every locus's struct type exists, so
    /// accept's child-locus param can resolve regardless of
    /// declaration order.
    ///
    /// Accepted lifecycle methods (codegen v0):
    /// - `birth(self_ptr)` — runs after instantiation fills fields
    /// - `accept(parent_self_ptr, child_ptr)` — runs once per child,
    ///   before that child's own `birth` (per F.7)
    /// - `run(self_ptr)` — runs after `birth`
    /// - `drain(self_ptr)` — runs after `run`, before `dissolve`,
    ///   after the body's child loci have already finished their
    ///   own drain/dissolve sequence (F.4 depth-first cascade)
    /// - `dissolve(self_ptr)` — runs last, before the alloca dies
    fn declare_locus_methods(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError> {
        if !l.generics.is_empty() {
            // m63: see declare_locus_struct — generic templates
            // skip method declaration too.
            return Ok(());
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();
        let mut methods: BTreeMap<&'static str, FunctionValue<'ctx>> =
            BTreeMap::new();
        let mut empty_lifecycle: std::collections::BTreeSet<&'static str> =
            std::collections::BTreeSet::new();
        let mut accept_param: Option<(String, String)> = None;
        let mut release_param: Option<(String, String)> = None;
        let mut user_methods: BTreeMap<String, FunctionValue<'ctx>> =
            BTreeMap::new();
        let mut subscriptions: Vec<(String, String, String, Option<KeyFilter>)> =
            Vec::new();
        // shm_ring batch consumers (2026-06-26): handler names whose
        // single param is `Drain<T>` → register through the batch path.
        let mut batch_handlers: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let mut closures: Vec<(String, ClosureAssertion, EpochSpec)> =
            Vec::new();
        let mut failure_handler: Option<(String, FunctionValue<'ctx>)> = None;

        // Pre-collect bus-handler method names so we can reject
        // defaults on them: bus dispatch is a fixed (self, payload)
        // C-runtime call that can't materialize default values for
        // extra params. Defaults on non-handler methods (called via
        // `self.method(...)`) work fine — m33 lifts that gate.
        let bus_handler_names: std::collections::BTreeSet<String> = l
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Bus(bb) => Some(bb.members.iter().filter_map(|bm| {
                    match bm {
                        BusMember::Subscribe { handler, .. } => Some(handler.name.clone()),
                        _ => None,
                    }
                })),
                _ => None,
            })
            .flatten()
            .collect();

        for member in &l.members {
            match member {
                LocusMember::Params(_) | LocusMember::Contract(_) => {
                    // Params handled in pass A1; contracts are a
                    // typecheck-only feature with no codegen ABI.
                }
                LocusMember::Bindings(_)
                | LocusMember::Placement(_)
                | LocusMember::BirthCheck(_) => {
                    // Bindings + placement emitted by main-locus
                    // prelude pass; birth_check clauses are emitted
                    // inline at instantiation (see
                    // lower_locus_instantiation's F.27 v2 block).
                    // None contributes to the method table.
                }
                LocusMember::Lifecycle(lc) => {
                    if lc.ret.is_some() {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` lifecycle `{:?}` declares a return \
                             type; only void is supported in v0",
                            l.name.name, lc.kind
                        )));
                    }
                    match lc.kind {
                        LifecycleKind::Birth
                        | LifecycleKind::Run
                        | LifecycleKind::Drain
                        | LifecycleKind::Dissolve => {
                            let kind: &'static str = match lc.kind {
                                LifecycleKind::Birth => "birth",
                                LifecycleKind::Run => "run",
                                LifecycleKind::Drain => "drain",
                                LifecycleKind::Dissolve => "dissolve",
                                _ => unreachable!(),
                            };
                            if !lc.params.is_empty() {
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` lifecycle `{}` declares \
                                     params; only the implicit self is \
                                     supported",
                                    l.name.name, kind
                                )));
                            }
                            let fn_ty =
                                void_t.fn_type(&[ptr_t.into()], false);
                            let func = self.module.add_function(
                                &format!("{}.{}", l.name.name, kind),
                                fn_ty,
                                None,
                            );
                            methods.insert(kind, func);
                            if lc.body.stmts.is_empty() && lc.body.tail.is_none() {
                                empty_lifecycle.insert(kind);
                            }
                        }
                        LifecycleKind::Accept => {
                            if lc.params.len() != 1 {
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` accept() must take exactly \
                                     one child param, got {}",
                                    l.name.name,
                                    lc.params.len()
                                )));
                            }
                            let p = &lc.params[0];
                            let child_ty = self.type_expr_to_codegen_ty(&p.ty)?;
                            let child_locus = match &child_ty {
                                CodegenTy::LocusRef(name) => name.clone(),
                                other => {
                                    return Err(CodegenError::Unsupported(
                                        format!(
                                            "locus `{}` accept() param must \
                                             be a locus type; got {:?}",
                                            l.name.name, other
                                        ),
                                    ));
                                }
                            };
                            let fn_ty = void_t
                                .fn_type(&[ptr_t.into(), ptr_t.into()], false);
                            let func = self.module.add_function(
                                &format!("{}.accept", l.name.name),
                                fn_ty,
                                None,
                            );
                            methods.insert("accept", func);
                            accept_param =
                                Some((p.name.name.clone(), child_locus));
                            if lc.body.stmts.is_empty() && lc.body.tail.is_none() {
                                empty_lifecycle.insert("accept");
                            }
                        }
                        LifecycleKind::Release => {
                            // Death-side bookend, same shape as accept:
                            // one typed child param, fn(parent, child).
                            // Declaring it marks the child type a flow
                            // (run-completion reclaims it).
                            if lc.params.len() != 1 {
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` release() must take exactly \
                                     one child param, got {}",
                                    l.name.name,
                                    lc.params.len()
                                )));
                            }
                            let p = &lc.params[0];
                            let child_ty = self.type_expr_to_codegen_ty(&p.ty)?;
                            let child_locus = match &child_ty {
                                CodegenTy::LocusRef(name) => name.clone(),
                                other => {
                                    return Err(CodegenError::Unsupported(
                                        format!(
                                            "locus `{}` release() param must \
                                             be a locus type; got {:?}",
                                            l.name.name, other
                                        ),
                                    ));
                                }
                            };
                            let fn_ty = void_t
                                .fn_type(&[ptr_t.into(), ptr_t.into()], false);
                            let func = self.module.add_function(
                                &format!("{}.release", l.name.name),
                                fn_ty,
                                None,
                            );
                            methods.insert("release", func);
                            release_param =
                                Some((p.name.name.clone(), child_locus));
                            if lc.body.stmts.is_empty() && lc.body.tail.is_none() {
                                empty_lifecycle.insert("release");
                            }
                        }
                    }
                }
                LocusMember::Bus(bb) => {
                    // Collect subscribe declarations; publish is
                    // typecheck-only (the `<-` operator does the
                    // emit at codegen). Subject must be a literal
                    // string at compile time.
                    //
                    // m60: each subscription also carries the
                    // payload type's name so registration can
                    // look up the matching __deserialize_T fn in
                    // cx.serializers. The `of type T` clause is
                    // optional in the AST but every example
                    // declares it; if it's missing we fall back
                    // to extracting the type from the handler's
                    // first param signature later in pass A2.
                    for bm in &bb.members {
                        match bm {
                            BusMember::Subscribe { subject, handler, ty, key_filter, .. } => {
                                let payload_type_name = ty
                                    .as_ref()
                                    .and_then(|t| {
                                        self.type_expr_to_codegen_ty(t).ok()
                                    })
                                    .and_then(|lt| match lt {
                                        CodegenTy::TypeRef(n) => Some(n),
                                        CodegenTy::Enum(n) => Some(n),
                                        // Raw-frame foreign-ring path: a
                                        // BytesView payload has no struct
                                        // type. The name isn't a user_type,
                                        // so the layout-subscriber lowering
                                        // resolves value_size = 0 (raw) and
                                        // the runtime hands the handler a
                                        // bounded view per record.
                                        CodegenTy::BytesView => {
                                            Some("BytesView".to_string())
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        CodegenError::Unsupported(format!(
                                            "locus `{}` subscribe `{}`: \
                                             missing or unsupported \
                                             payload type (m60 requires \
                                             a TypeRef, has-payload Enum, \
                                             or BytesView)",
                                            l.name.name, subject
                                        ))
                                    })?;
                                subscriptions.push((
                                    subject.canonical().to_string(),
                                    handler.name.clone(),
                                    payload_type_name,
                                    key_filter.clone(),
                                ));
                            }
                            BusMember::Publish { .. } => {
                                // No-op at codegen; type info
                                // already enforced by typechecker.
                            }
                        }
                    }
                }
                LocusMember::Fn(fd) => {
                    // Locus user-fn: declare as
                    // `<Locus>.<name>(self_ptr, ...args)`. Body
                    // lowered in pass C.
                    if !fd.generics.is_empty() {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` method `{}`: generics not lowered",
                            l.name.name, fd.name.name
                        )));
                    }
                    // Open-question #24 v0.2 (2026-05-25): user-
                    // declared `fn` members may carry
                    // `fallible(E)` with arbitrary success / err
                    // payload types — heap-bearing payloads
                    // (`String`, `Bytes`, struct fields with
                    // heap content) now ride through the same
                    // TLS-based caller-arena snapshot that non-
                    // fallible heap-returning locus methods
                    // already use (`open_method_scratch` +
                    // `emit_method_return_deep_copy`). Typecheck
                    // still gates bus-subscribed handlers and
                    // closure assertions — those can't be
                    // fallible because the substrate has no
                    // caller frame to address a value error.
                    let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum> =
                        Vec::with_capacity(fd.params.len() + 1);
                    llvm_param_tys.push(ptr_t.into());
                    let is_bus_handler =
                        bus_handler_names.contains(&fd.name.name);
                    let mut seen_default = false;
                    for p in &fd.params {
                        if p.default.is_some() {
                            if is_bus_handler {
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` method `{}`: bus-subscribed \
                                     handlers can't have default param values \
                                     (bus dispatch is fixed-arity self+payload)",
                                    l.name.name, fd.name.name
                                )));
                            }
                            seen_default = true;
                        } else if seen_default {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` method `{}`: required param `{}` \
                                 follows a defaulted param; defaults must form \
                                 a suffix",
                                l.name.name, fd.name.name, p.name.name
                            )));
                        }
                        let lt = self.type_expr_to_codegen_ty(&p.ty)?;
                        // Drain<T> as a bus-handler's single param marks
                        // a batch consumer. The LLVM type is `ptr` (the
                        // handle), so the fn signature is unchanged; only
                        // the registration path differs.
                        if is_bus_handler && matches!(lt, CodegenTy::Drain(_)) {
                            batch_handlers.insert(fd.name.name.clone());
                        }
                        llvm_param_tys.push(self.llvm_basic_type(&lt).into());
                    }
                    // Open-question #24 MVP: fallible-method ABI
                    // mirrors the free-fn fallible ABI minus the
                    // `__caller_arena` plumbing. For value-only
                    // success/err types (the MVP scope), no deep-
                    // copy is needed — the sret stores write
                    // primitive bytes directly into caller-owned
                    // slots.
                    // A2 (G2): normalize `-> ()` (empty tuple) to
                    // Unit — same shape `declare_user_fn` uses. The
                    // tuple-type lowerer rejects 0-element tuples;
                    // treating `-> ()` as "no return type" matches
                    // the downstream sret-slot logic that gates on
                    // `ret_codegen_ty.is_some()`.
                    let ret_te_normalized: Option<&TypeExpr> = match &fd.ret {
                        Some(TypeExpr::Tuple(parts, _)) if parts.is_empty() => None,
                        other => other.as_ref(),
                    };
                    let ret_codegen_ty: Option<CodegenTy> = match ret_te_normalized {
                        None => None,
                        Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
                    };
                    let err_payload_ty: Option<CodegenTy> = match &fd.fallible {
                        None => None,
                        Some(payload_te) => {
                            let pt = self.type_expr_to_codegen_ty(payload_te)?;
                            // Open-question #24 v0.2 (2026-05-25):
                            // heap-bearing payloads now flow through
                            // the same TLS caller-arena snapshot +
                            // `emit_method_return_deep_copy` machinery
                            // that non-fallible heap-returning locus
                            // methods use. No type-shape gate here
                            // anymore; the epilogue routes both ok
                            // and fail branches through the deep-copy
                            // (scalars pass through unchanged via
                            // `ty_needs_self_field_deep_copy`).
                            //
                            // Add sret slot params: out_val if
                            // success is non-Unit, then out_err.
                            // Mirrors the free-fn ABI ordering.
                            if ret_codegen_ty.is_some() {
                                llvm_param_tys.push(ptr_t.into());
                            }
                            llvm_param_tys.push(ptr_t.into());
                            Some(pt)
                        }
                    };
                    let fn_ty = if err_payload_ty.is_some() {
                        // Fallible: LLVM-level return is i1 (path
                        // indicator). Caller branches.
                        self.context
                            .bool_type()
                            .fn_type(&llvm_param_tys, false)
                    } else {
                        match &ret_codegen_ty {
                            None => void_t.fn_type(&llvm_param_tys, false),
                            Some(rt) => match rt {
                                CodegenTy::Bounded(_, _) => {
                                    return Err(CodegenError::Unsupported(
                                        "bounded[T; N] cannot be returned \
                                         by value"
                                            .into(),
                                    ));
                                }
                                CodegenTy::Int | CodegenTy::Duration => self
                                    .context
                                    .i64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Float => self
                                    .context
                                    .f64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Decimal => self
                                    .context
                                    .i128_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Bool => self
                                    .context
                                    .bool_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Enum(name) => {
                                    if self
                                        .user_enums
                                        .get(name.as_str())
                                        .map(|i| i.has_payload)
                                        .unwrap_or(false)
                                    {
                                        self.context
                                            .ptr_type(AddressSpace::default())
                                            .fn_type(&llvm_param_tys, false)
                                    } else {
                                        self.context
                                            .i32_type()
                                            .fn_type(&llvm_param_tys, false)
                                    }
                                }
                                CodegenTy::BytesView
                                | CodegenTy::StringView
                                | CodegenTy::BytesMut => self
                                    .view_struct_ty()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::String
                                | CodegenTy::Bytes
                                | CodegenTy::Time
                                | CodegenTy::LocusRef(_)
                                | CodegenTy::TypeRef(_)
                                | CodegenTy::Array(_, _)
                                | CodegenTy::Tuple(_)
                                | CodegenTy::FnPtr { .. }
                                | CodegenTy::Interface(_)
                                | CodegenTy::Cell(_, _)
                                | CodegenTy::Drain(_) => self
                                    .context
                                    .ptr_type(AddressSpace::default())
                                    .fn_type(&llvm_param_tys, false),
                            },
                        }
                    };
                    let func = self.module.add_function(
                        &format!("{}.{}", l.name.name, fd.name.name),
                        fn_ty,
                        None,
                    );
                    apply_noalias_self_if_provable(
                        self,
                        func,
                        &l.name.name,
                        &fd.name.name,
                        &fd.params,
                    );
                    user_methods.insert(fd.name.name.clone(), func);
                }
                LocusMember::Closure(c) => {
                    // m39 + m42 + m43 + m44: all five closure
                    // epochs now lower. Default (no epoch
                    // clause) = Dissolve, matching pre-m39
                    // semantics.
                    let mut epoch = EpochSpec::Dissolve;
                    for clause in &c.clauses {
                        match clause {
                            ClosureClause::Epoch(spec) => {
                                epoch = spec.clone();
                            }
                            ClosureClause::PersistsThrough(_)
                            | ClosureClause::ResetsOn(_)
                            | ClosureClause::ResetsPerEpoch(_)
                            | ClosureClause::Captures(_) => {
                                // Recovery-event hooks +
                                // v1.x-VIOLATE captures clause +
                                // v1.x-WINDOWED per-epoch reset
                                // (handled in the duration-fn body,
                                // not the epoch dispatch table).
                            }
                        }
                    }
                    // v1.x-VIOLATE (F.27): assertion-less inline
                    // closures don't go through this auto-epoch
                    // lowering pipeline (they fire via `violate`,
                    // not at epoch boundaries). Codegen for them
                    // is part of phase 4 of v1.x-VIOLATE; the
                    // parse + AST plumbing lands in phase 2.
                    let Some(assertion) = c.assertion.clone() else {
                        continue;
                    };
                    closures.push((
                        c.name.name.clone(),
                        assertion,
                        epoch,
                    ));
                }
                LocusMember::Failure(fd) => {
                    // on_failure(child: ChildL, err: ClosureViolation)
                    // is a handler closures route to when an
                    // unabsorbed violation reaches the parent.
                    if fd.params.len() != 2 {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` on_failure must take exactly two \
                             params (child + err), got {}",
                            l.name.name,
                            fd.params.len()
                        )));
                    }
                    let child_ty = self.type_expr_to_codegen_ty(&fd.params[0].ty)?;
                    let child_locus_name = match &child_ty {
                        CodegenTy::LocusRef(n) => n.clone(),
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` on_failure first param must be \
                                 a locus type; got {:?}",
                                l.name.name, other
                            )));
                        }
                    };
                    let err_ty = self.type_expr_to_codegen_ty(&fd.params[1].ty)?;
                    if err_ty != CodegenTy::TypeRef("ClosureViolation".into())
                    {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` on_failure second param must be \
                             ClosureViolation; got {:?}",
                            l.name.name, err_ty
                        )));
                    }
                    // Sig: void(parent_self, child_self, violation)
                    let fn_ty = void_t.fn_type(
                        &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
                        false,
                    );
                    let func = self.module.add_function(
                        &format!("{}.on_failure", l.name.name),
                        fn_ty,
                        None,
                    );
                    failure_handler = Some((child_locus_name, func));
                }
                LocusMember::Mode(md) => {
                    // Modes lower as locus methods named after
                    // the mode kind (bulk / harmonic /
                    // resolution). They share the locus's struct
                    // (per F.5: mode projections share the
                    // locus's arena). Callable via self.bulk()
                    // through the existing self.method() path.
                    let mode_name = match md.kind {
                        ModeKind::Bulk => "bulk",
                        ModeKind::Harmonic => "harmonic",
                        ModeKind::Resolution => "resolution",
                    };
                    let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum> =
                        Vec::with_capacity(md.params.len() + 1);
                    llvm_param_tys.push(ptr_t.into());
                    // m54: mode params accept defaults under the
                    // same suffix-only rule as locus fn methods.
                    // lower_self_method_call already handles the
                    // call-site fill-in (it dispatches uniformly
                    // on Fn / Mode via the program-walk that
                    // returns a MethodSig), so the only thing
                    // this declare-side block needs is the
                    // ordering check.
                    let mut seen_default = false;
                    for p in &md.params {
                        if p.default.is_some() {
                            seen_default = true;
                        } else if seen_default {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` mode `{}`: required param \
                                 `{}` follows a defaulted param; defaults \
                                 must form a suffix",
                                l.name.name, mode_name, p.name.name
                            )));
                        }
                        let lt = self.type_expr_to_codegen_ty(&p.ty)?;
                        llvm_param_tys.push(self.llvm_basic_type(&lt).into());
                    }
                    let fn_ty = match &md.ret {
                        None => void_t.fn_type(&llvm_param_tys, false),
                        Some(t) => {
                            let rt = self.type_expr_to_codegen_ty(t)?;
                            match rt {
                                CodegenTy::Bounded(_, _) => {
                                    return Err(CodegenError::Unsupported(
                                        "bounded[T; N] cannot be returned \
                                         by value"
                                            .into(),
                                    ));
                                }
                                CodegenTy::Int | CodegenTy::Duration => self
                                    .context
                                    .i64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Float => self
                                    .context
                                    .f64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Decimal => self
                                    .context
                                    .i128_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Bool => self
                                    .context
                                    .bool_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Enum(name) => {
                                    if self
                                        .user_enums
                                        .get(name.as_str())
                                        .map(|i| i.has_payload)
                                        .unwrap_or(false)
                                    {
                                        self.context
                                            .ptr_type(AddressSpace::default())
                                            .fn_type(&llvm_param_tys, false)
                                    } else {
                                        self.context
                                            .i32_type()
                                            .fn_type(&llvm_param_tys, false)
                                    }
                                }
                                CodegenTy::BytesView
                                | CodegenTy::StringView
                                | CodegenTy::BytesMut => self
                                    .view_struct_ty()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::String
                                | CodegenTy::Bytes
                                | CodegenTy::Time
                                | CodegenTy::LocusRef(_)
                                | CodegenTy::TypeRef(_)
                                | CodegenTy::Array(_, _)
                                | CodegenTy::Tuple(_)
                                | CodegenTy::FnPtr { .. }
                                | CodegenTy::Interface(_)
                                | CodegenTy::Cell(_, _)
                                | CodegenTy::Drain(_) => self
                                    .context
                                    .ptr_type(AddressSpace::default())
                                    .fn_type(&llvm_param_tys, false),
                            }
                        }
                    };
                    let func = self.module.add_function(
                        &format!("{}.{}", l.name.name, mode_name),
                        fn_ty,
                        None,
                    );
                    // Aliasing stage 2: modes are in the elidable
                    // fixpoint under their synthetic names.
                    apply_noalias_self_if_provable(
                        self,
                        func,
                        &l.name.name,
                        mode_name,
                        &md.params,
                    );
                    user_methods.insert(mode_name.to_string(), func);
                }
                LocusMember::Const(_)
                | LocusMember::Type(_) => {
                    return Err(CodegenError::Unsupported(format!(
                        "locus `{}` member kind not yet lowered to codegen",
                        l.name.name
                    )));
                }
                LocusMember::Capacity(_) => {
                    // F.22 slots have no method-decl phase — the
                    // struct layout, slot init, and slot destroy
                    // are already wired in declare_locus_struct,
                    // lower_locus_instantiation, and
                    // emit_locus_arena_destroy. The user-facing
                    // `self.X.acquire()` dispatch lands in #17.
                }
            }
        }

        // m39: declare per-epoch synthetic eval fns. Each has the
        // same signature — `(self_ptr, parent_self_or_null,
        // on_failure_fn_or_null)` — but lowers a different subset
        // of the closure list. Call sites pass the parent's self +
        // on_failure fn ptr if the parent has a matching handler,
        // else null/null. Bodies lowered in pass C.
        let has_birth = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Birth));
        let has_dissolve = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Dissolve));
        let has_tick = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Tick));
        let has_duration = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Duration(_)));
        let has_explicit = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Explicit));
        let make_eval_fn = |name: &str| {
            let fn_ty = void_t.fn_type(
                &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
                false,
            );
            self.module.add_function(name, fn_ty, None)
        };
        let birth_closures_fn = if has_birth {
            Some(make_eval_fn(&format!(
                "{}.__birth_closures",
                l.name.name
            )))
        } else {
            None
        };
        let dissolve_closures_fn = if has_dissolve {
            Some(make_eval_fn(&format!(
                "{}.__dissolve_closures",
                l.name.name
            )))
        } else {
            None
        };
        // m42: tick_closures_fn has the standard 3-arg shape
        // (self, parent, on_failure); tick_wrapper_fn is a
        // 1-arg adapter `(self) -> void` that loads parent +
        // on_failure from the struct's __parent_self /
        // __parent_on_failure fields and tail-calls the
        // 3-arg fn. The wrapper is what bus-handler thunks
        // call (they only have self in scope).
        let tick_closures_fn = if has_tick {
            Some(make_eval_fn(&format!(
                "{}.__tick_closures",
                l.name.name
            )))
        } else {
            None
        };
        let tick_wrapper_fn = if has_tick {
            let wrapper_ty = void_t.fn_type(&[ptr_t.into()], false);
            Some(self.module.add_function(
                &format!("{}.__tick_closures_wrapper", l.name.name),
                wrapper_ty,
                None,
            ))
        } else {
            None
        };
        // m43: __duration_closures has the standard 3-arg
        // shape. Bodies do their own gating on
        // monotonic-elapsed-since-last-fire per closure;
        // shared with tick at the lifecycle call sites
        // (post-handler, post-run).
        let duration_closures_fn = if has_duration {
            Some(make_eval_fn(&format!(
                "{}.__duration_closures",
                l.name.name
            )))
        } else {
            None
        };
        // m43-followup: 1-arg wrapper adapter, same shape as
        // tick_wrapper_fn. Needed for the pinned post-run() path,
        // where the calling context is the pinned thread (no
        // `current_self`), so the 3-arg fn's parent args can't be
        // resolved at the call site — they have to come from the
        // struct fields baked at instantiation time.
        let duration_wrapper_fn = if has_duration {
            let wrapper_ty = void_t.fn_type(&[ptr_t.into()], false);
            Some(self.module.add_function(
                &format!("{}.__duration_closures_wrapper", l.name.name),
                wrapper_ty,
                None,
            ))
        } else {
            None
        };
        // m44: __explicit_closures has the same 3-arg shape.
        // Called only by the `check_closures();` builtin —
        // user-triggered audit at a chosen checkpoint.
        let explicit_closures_fn = if has_explicit {
            Some(make_eval_fn(&format!(
                "{}.__explicit_closures",
                l.name.name
            )))
        } else {
            None
        };
        // m42: tick-call placement. An earlier draft wrapped
        // each subscribed handler with a post-call thunk;
        // that broke order because the handler's own tail
        // `bus_queue_drain` (m26) recursively processed
        // queued cells before the thunk's tick step ran.
        // Final design inlines the tick call into the
        // subscribed user-fn body just before its tail
        // drain — see Pass C's user-fn body lowering.

        // Stash the methods + accept_param onto the existing
        // LocusInfo.
        let info = self
            .user_loci
            .get_mut(&l.name.name)
            .expect("locus struct declared in pass A1");
        info.methods = methods;
        info.empty_lifecycle = empty_lifecycle;
        info.accept_param = accept_param;
        info.release_param = release_param;
        info.user_methods = user_methods;
        info.subscriptions = subscriptions;
        info.batch_handlers = batch_handlers;
        info.closures = closures;
        info.birth_closures_fn = birth_closures_fn;
        info.dissolve_closures_fn = dissolve_closures_fn;
        info.tick_closures_fn = tick_closures_fn;
        info.tick_wrapper_fn = tick_wrapper_fn;
        info.duration_closures_fn = duration_closures_fn;
        info.duration_wrapper_fn = duration_wrapper_fn;
        info.explicit_closures_fn = explicit_closures_fn;
        info.failure_handler = failure_handler;
        Ok(())
    }

}
