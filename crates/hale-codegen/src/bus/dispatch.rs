//! Bus publish-site codegen + Phase-3 subscribe-key filter emission.
//! `lower_send` lowers `payload -> Topic` statements; `lower_send_shm_ring`
//! is the K (shm_ring) transport variant; `lower_subscribe_key_filter`
//! emits the runtime guard a keyed subscriber's body runs against the
//! incoming wire subject. Round 3b of the codegen model-org refactor.

use hale_syntax::ast::{Expr, KeyFilter, Literal, OrDisposition, Stmt};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, RoutingKeySubjectInfo, Scope,
};

/// Gap B (2026-07-17): a lowered `where key ==` filter. Scalar keys
/// keep the Phase-3 (kind, key_lo, key_hi) triple; String keys carry
/// the key's pointer — the runtime (`lotus_bus_register_keyed_str`)
/// hashes it and stores an owned copy at registration.
pub(crate) enum LoweredKeyFilter<'ctx> {
    Scalar(u8, IntValue<'ctx>, IntValue<'ctx>),
    Str(PointerValue<'ctx>),
}

pub(crate) trait BusDispatch<'ctx> {
    fn lower_subscribe_key_filter(
        &mut self,
        self_ptr: PointerValue<'ctx>,
        key_filter: Option<&KeyFilter>,
        subject: &str,
    ) -> Result<LoweredKeyFilter<'ctx>, CodegenError>;
    fn lower_send(
        &mut self,
        subject: &Expr,
        value: &Expr,
        or_disposition: Option<&OrDisposition>,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError>;
    fn lower_send_shm_ring(
        &mut self,
        subject: &str,
        value: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> BusDispatch<'ctx> for Cx<'ctx, 'p> {
    /// Emit a single subscription registration as one call to
    /// `lotus_bus_register(subject, self, handler, mailbox,
    /// deserialize_fn)`. The C runtime owns the entries vec and
    /// grows it on demand, so there's no compile-time-fixed
    /// capacity ceiling. `mailbox_or_null` is `Some(mb_ptr)` for
    /// pinned subscribers (cells route to that locus's mailbox)
    /// and `None` for cooperative subscribers (cells route to
    /// the global queue). m60: `payload_type` names the type
    /// declared in the matching `bus subscribe "..." of type T`
    /// — used to look up `__deserialize_T` so the reader thread
    /// (m59) can decode wire-format bytes into a struct before
    /// dispatching to the handler.
    /// Phase 3 (2026-05-25): lower a `where key == EXPR` clause's
    /// RHS into what the runtime registration expects — the
    /// (kind, key_lo, key_hi) triple for scalar keys
    /// (`lotus_bus_register_keyed`), or the key's String pointer
    /// for String keys (`lotus_bus_register_keyed_str`, Gap B
    /// 2026-07-17 — the runtime hashes it and stores an owned
    /// copy).
    ///
    /// v0.1 supports three RHS shapes:
    ///   - literal Int / Decimal / Time / Duration / Bool /
    ///     no-payload enum variant / String
    ///   - `self.<field>` path read (field must be a supported
    ///     key type, enforced here as a codegen-time diag)
    ///   - `_` sentinel → kind = 2 (catch-unmatched fallback)
    ///
    /// For absent filter: kind = 0; key_lo = key_hi = 0
    /// (receive-all, today's behavior).
    fn lower_subscribe_key_filter(
        &mut self,
        self_ptr: PointerValue<'ctx>,
        key_filter: Option<&KeyFilter>,
        subject: &str,
    ) -> Result<LoweredKeyFilter<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        match key_filter {
            None => Ok(LoweredKeyFilter::Scalar(
                0,
                i64_t.const_zero(),
                i64_t.const_zero(),
            )),
            Some(KeyFilter::Unmatched { .. }) => {
                // Phase 3 fallback policy (2026-05-25): register
                // with key_filter_kind=2. The runtime's keyed-
                // dispatch second-pass fires kind=2 entries when
                // no specific-key (kind=1) match was found.
                // Typecheck has already validated that this
                // subscribe targets a `on_unmatched: fallback`
                // topic; codegen trusts that contract.
                let _ = self_ptr;
                let _ = subject;
                Ok(LoweredKeyFilter::Scalar(
                    2,
                    i64_t.const_zero(),
                    i64_t.const_zero(),
                ))
            }
            Some(KeyFilter::Specific { expr, .. }) => {
                // Lower the EXPR. `lower_expr` reads `self.X`
                // through `self.current_self` (set by the caller —
                // `lower_locus_instantiation` wraps the
                // subscription loop with a temp-current_self
                // assignment so the just-constructed locus's
                // fields are visible).
                let _ = self_ptr;
                let scope = Scope::default();
                let (val, ty) = self.lower_expr(expr, &scope)?;
                // Gap B: String key — hand the pointer to the
                // runtime, which owns a copy + its hash.
                // (StringView is NOT accepted: its data pointer
                // isn't NUL-terminated, and the runtime key copy /
                // compare are strcmp-shaped. `std::str::clone` a
                // view first.)
                if matches!(ty, CodegenTy::String) {
                    return Ok(LoweredKeyFilter::Str(
                        val.into_pointer_value(),
                    ));
                }
                let (key_lo, key_hi) = self.key_value_to_i64_pair(val, &ty)
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "subscribe `{}`: `where key == EXPR` RHS \
                             of type {:?} is not a supported key type \
                             (must be Int / Decimal / Time / Duration \
                             / Bool / no-payload enum / String)",
                            subject, ty
                        ))
                    })?;
                Ok(LoweredKeyFilter::Scalar(1, key_lo, key_hi))
            }
        }
    }

    /// Lower a `subject <- payload;` statement to a single call to
    /// the C-runtime `lotus_bus_dispatch(queue, subject, payload, size)`.
    /// Subject must evaluate to a String pointer; payload must be a
    /// TypeRef value (a pointer to a user-type struct). The C
    /// runtime walks its (heap-grown) entries vec and routes each
    /// match either to the cooperative queue or to a pinned
    /// subscriber's mailbox, by mailbox-null-or-not at registration.
    fn lower_send(
        &mut self,
        subject: &Expr,
        value: &Expr,
        or_disposition: Option<&OrDisposition>,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        // Form K4c (2026-05-20): shm_ring short-circuit. If the
        // subject is a compile-time-constant string that matches
        // a registered shm_ring binding, route through
        // lotus_bus_publish_shm_ring (claim + memcpy + commit)
        // and skip the rest of the dispatch machinery — including
        // the bus_state check, since an shm_ring publisher
        // doesn't need a same-binary subscriber (subscribers may
        // be in another process attached to the same SHM
        // object).
        let shm_subject_const: Option<String> = match subject {
            Expr::Literal(Literal::String(s), _) => {
                if self.shm_ring_subjects.contains_key(s) {
                    Some(s.clone())
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(subj_str) = shm_subject_const {
            return self.lower_send_shm_ring(&subj_str, value, scope);
        }

        let _ = self.bus_state.ok_or_else(|| {
            CodegenError::Unsupported(
                "bus send `<-` used but no `bus subscribe` declared in \
                 program — nothing to dispatch to"
                    .to_string(),
            )
        })?;
        let (subj_val, subj_ty) = self.lower_expr(subject, scope)?;
        if !matches!(subj_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "bus send subject must be String; got {:?}",
                subj_ty
            )));
        }
        let subj_val = self.unpack_view_if_needed(subj_val, &subj_ty)?;
        // v1.x-FRAMEWORK: ephemeral-payload fast path. When the
        // value is a bare struct literal, the publisher-side
        // storage is dead after lotus_bus_dispatch returns (the
        // queue cell holds the canonical memcpy). Stack-alloca
        // the storage in the entry block + lower fields directly
        // into it, bypassing lower_user_type_instantiation's
        // arena_alloc per publish. Per-event publisher arena
        // bloat (≈sizeof(T) bytes / publish) goes away too.
        //
        // Falls through to the regular lower_expr path for any
        // value that isn't a bare struct literal (locus refs,
        // enum payloads, expressions producing already-allocated
        // pointers, etc.).
        let stack_payload: Option<(PointerValue<'ctx>, String)> = match value {
            Expr::Struct { path, inits, .. } => {
                let mangled: Option<String> = if path.segments.len() == 1 {
                    let name = path.segments[0].name.clone();
                    if self.user_types.contains_key(&name) {
                        Some(name)
                    } else {
                        None
                    }
                } else {
                    let segs: Vec<&str> = path
                        .segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect();
                    self.mangled_for_path(&segs).and_then(|m| {
                        if self.user_types.contains_key(&m) {
                            Some(m)
                        } else {
                            None
                        }
                    })
                };
                if let Some(mname) = mangled {
                    let info = self
                        .user_types
                        .get(&mname)
                        .cloned()
                        .expect("checked above");
                    let slot = self.alloca_in_entry(
                        info.struct_ty.into(),
                        &format!("{}.send.payload", mname),
                    )?;
                    self.populate_user_type_fields(
                        &mname, &info, inits, slot, scope,
                    )?;
                    Some((slot, mname))
                } else {
                    None
                }
            }
            _ => None,
        };
        let (payload_val, payload_ty): (BasicValueEnum<'ctx>, CodegenTy) =
            if let Some((slot, mname)) = &stack_payload {
                ((*slot).into(), CodegenTy::TypeRef(mname.clone()))
            } else {
                self.lower_expr(value, scope)?
            };
        // Flat-payload serialize-skip (2026-06-28): a transitively
        // pointer-free POD payload needs no arena rebinding, so route
        // it through the verbatim `lotus_bus_dispatch_flat` family
        // instead of the serialize → per-sub-deserialize wire path. We
        // still pass the serializer (remote fanout needs it). See
        // `bus_payload_is_flat`.
        let payload_is_flat = self.bus_payload_is_flat(&payload_ty);
        // m47-payloads-followup: bus payload is either a
        // user-type struct pointer OR a has-payload enum
        // pointer. Both lower to a ptr value + a sized storage
        // struct. m60: payload bytes flow through __serialize_T
        // before reaching lotus_bus_dispatch, so the wire format
        // is governed by the per-type serializer rather than
        // implicit struct-layout assumption.
        let (payload_type_name, payload_struct_ty) = match &payload_ty {
            CodegenTy::TypeRef(name) => {
                let info = self
                    .user_types
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload type `{}` not declared",
                            name
                        ))
                    })?;
                (name.clone(), info.struct_ty)
            }
            CodegenTy::Enum(name) => {
                let info = self
                    .user_enums
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload enum `{}` not declared",
                            name
                        ))
                    })?;
                if !info.has_payload {
                    return Err(CodegenError::Unsupported(format!(
                        "bus send of no-payload enum `{}` not supported \
                         at v0.1 — wrap in a struct or add a variant payload",
                        name
                    )));
                }
                (name.clone(), self.enum_storage_struct(&info))
            }
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "bus send payload must be a user-type or has-payload \
                     enum value; got {:?}",
                    other
                )));
            }
        };
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let payload_size_iv = payload_struct_ty
            .size_of()
            .expect("payload struct has known size");

        // m70: pass struct bytes directly to lotus_bus_dispatch
        // along with the per-subject __serialize_T fn pointer.
        // The dispatcher does local enqueue with struct bytes
        // (preserving in-process semantics: String pointers stay
        // valid because the publisher's arena outlives the
        // immediate dispatch), and serializes through the
        // supplied fn into wire bytes for cross-process fanout.
        // Pre-m70 lower_send allocated a scratch buffer + called
        // __serialize_T inline; m70 moves serialization into the
        // C runtime so the wire bytes are only computed when
        // they're about to be sent.
        // F.36 Slice 3b: when the subject is a compile-time-constant
        // string AND main has a `codec(L { ... })` clause for that
        // subject, route the wire-encode through the synthesized
        // encode thunk (which matches `lotus_serialize_fn`'s ABI).
        // The thunk invokes the user's `encode(v: T) -> Bytes
        // fallible(E)` method and translates the result into the
        // m70 fill-buffer shape, so the runtime dispatch machinery
        // is untouched.
        let codec_encode_override: Option<inkwell::values::FunctionValue<'ctx>> =
            match subject {
                Expr::Literal(Literal::String(s), _) => self
                    .codec_thunks
                    .get(s)
                    .map(|t| t.encode),
                _ => None,
            };
        let ser_fn = match codec_encode_override {
            Some(thunk) => thunk,
            None => self
                .serializers
                .get(&payload_type_name)
                .ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "no serializer for bus payload `{}` — pass A3 should \
                         have synthesized one",
                        payload_type_name
                    ))
                })?
                .serialize,
        };

        let queue_global = self
            .module
            .get_global("lotus.bus_queue.global")
            .expect("bus queue global declared");
        let queue_ptr = self
            .builder
            .build_load(
                ptr_t,
                queue_global.as_pointer_value(),
                "bus.dispatch.queue",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Phase 3 (2026-05-25): if the subject is keyed_by, route
        // through lotus_bus_dispatch_keyed with the key extracted
        // from the payload's keyed_by field. The subject must be
        // a compile-time-constant string for the lookup to fire;
        // computed subjects always go through the legacy unkeyed
        // path (no static way to know which topic they're for).
        let keyed_info: Option<RoutingKeySubjectInfo> = match subject {
            Expr::Literal(Literal::String(s), _) => {
                self.routing_key_subjects.get(s).cloned()
            }
            _ => None,
        };
        if let Some(info) = keyed_info {
            // GEP into payload at the keyed_by field, load it,
            // convert to (key_lo, key_hi) i64 pair.
            let payload_struct_info = self
                .user_types
                .get(&info.payload_type_name)
                .cloned()
                .expect("keyed topic's payload type registered");
            let (field_idx, field_ty) = payload_struct_info
                .fields
                .get(&info.keyed_by_field)
                .cloned()
                .ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "keyed_by field `{}` missing on payload `{}` \
                         (typecheck should have caught this)",
                        info.keyed_by_field, info.payload_type_name
                    ))
                })?;
            let field_slot = self
                .builder
                .build_struct_gep(
                    payload_struct_info.struct_ty,
                    payload_val.into_pointer_value(),
                    field_idx,
                    &format!(
                        "bus.send.key.{}.{}.ptr",
                        info.payload_type_name, info.keyed_by_field
                    ),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let llvm_field_ty = self.llvm_basic_type(&field_ty);
            let field_val = self
                .builder
                .build_load(
                    llvm_field_ty,
                    field_slot,
                    &format!(
                        "bus.send.key.{}.{}.load",
                        info.payload_type_name, info.keyed_by_field
                    ),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // Gap B (2026-07-17): String keys ride the existing
            // (key_lo, key_hi) i64 pair — key_lo is the FNV-1a
            // hash (lotus_route_key_hash), key_hi the field's
            // char* as an integer. Dispatch runs synchronously on
            // this thread, so the pointer is live for the whole
            // call, and remote fanout is unkeyed at v0.1, so it
            // never crosses an address space. String-keyed
            // registry entries compare hash-then-strcmp
            // (lotus_bus_key_matches).
            let (key_lo, key_hi) = if matches!(field_ty, CodegenTy::String)
            {
                let field_ptr = field_val.into_pointer_value();
                let i64_t = self.context.i64_type();
                let hash_fn = self
                    .module
                    .get_function("lotus_route_key_hash")
                    .expect("lotus_route_key_hash declared");
                let hash = self
                    .builder
                    .build_call(
                        hash_fn,
                        &[field_ptr.into()],
                        "bus.send.key.str.hash",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_route_key_hash returns i64")
                    .into_int_value();
                let ptr_int = self
                    .builder
                    .build_ptr_to_int(
                        field_ptr,
                        i64_t,
                        "bus.send.key.str.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                (hash, ptr_int)
            } else {
                self.key_value_to_i64_pair(field_val, &field_ty)
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "keyed_by field `{}.{}` of type {:?} is not \
                             a supported key type (typecheck should \
                             have caught this)",
                            info.payload_type_name,
                            info.keyed_by_field,
                            field_ty
                        ))
                    })?
            };
            // Phase 3 fail policy (2026-05-25): `on_unmatched:
            // fail` routes through the fallible dispatch variant
            // and branches on the no-match return path into the
            // `or` disposition. Typecheck has already validated
            // that this Send carries `or raise` or `or discard`
            // for fail topics; v0.2 extends to the err-payload
            // dispositions.
            if matches!(
                info.policy,
                Some(hale_syntax::ast::UnmatchedPolicy::Fail)
            ) {
                let dispatch_fallible_name = if payload_is_flat {
                    "lotus_bus_dispatch_keyed_fallible_flat"
                } else {
                    "lotus_bus_dispatch_keyed_fallible"
                };
                let dispatch_fallible_fn = self
                    .module
                    .get_function(dispatch_fallible_name)
                    .expect("lotus_bus_dispatch_keyed_fallible declared");
                let i32_t = self.context.i32_type();
                let matched = self
                    .builder
                    .build_call(
                        dispatch_fallible_fn,
                        &[
                            queue_ptr.into(),
                            subj_val.into(),
                            payload_val.into(),
                            payload_size_iv.into(),
                            ser_fn.as_global_value().as_pointer_value().into(),
                            key_lo.into(),
                            key_hi.into(),
                        ],
                        "bus.dispatch_keyed_fallible.call",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("dispatch_keyed_fallible returns i32")
                    .into_int_value();
                match or_disposition {
                    Some(OrDisposition::Raise(_)) => {
                        // No match → panic with a BusUnmatchedKey
                        // message naming subject + key.
                        let cond = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                matched,
                                i32_t.const_zero(),
                                "bus.fail.no_match",
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .and_then(|bb| bb.get_parent())
                            .expect("inside a function");
                        let panic_bb = self.context.append_basic_block(
                            current_fn,
                            "bus.fail.raise",
                        );
                        let cont_bb = self.context.append_basic_block(
                            current_fn,
                            "bus.fail.cont",
                        );
                        self.builder
                            .build_conditional_branch(cond, panic_bb, cont_bb)
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        self.builder.position_at_end(panic_bb);
                        let panic_fn = self
                            .module
                            .get_function("lotus_root_panic")
                            .expect("lotus_root_panic declared");
                        let typename_str = self.global_string(
                            "BusUnmatchedKey (no specific-key subscriber matched the keyed publish)",
                        );
                        let null_payload = ptr_t.const_null();
                        let zero_size = i64_t.const_zero();
                        self.builder
                            .build_call(
                                panic_fn,
                                &[
                                    null_payload.into(),
                                    zero_size.into(),
                                    typename_str.into(),
                                ],
                                "bus.fail.raise.call",
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        self.builder
                            .build_unreachable()
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        self.builder.position_at_end(cont_bb);
                    }
                    Some(OrDisposition::Discard(_)) => {
                        // Silently swallow on no-match. The
                        // dispatch already happened; just ignore
                        // the return value.
                        let _ = matched;
                    }
                    Some(OrDisposition::Substitute(rhs)) => {
                        // v0.2 (2026-05-26): on no-match, allocate
                        // a BusUnmatchedKey err payload, bind it
                        // as `err` in scope, and lower the RHS as
                        // a statement (Send is statement-level so
                        // the substitute's value is discarded).
                        let cond = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                matched,
                                i32_t.const_zero(),
                                "bus.fail.no_match",
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .and_then(|bb| bb.get_parent())
                            .expect("inside a function");
                        let nomatch_bb = self.context.append_basic_block(
                            current_fn,
                            "bus.fail.substitute",
                        );
                        let cont_bb = self.context.append_basic_block(
                            current_fn,
                            "bus.fail.cont",
                        );
                        self.builder
                            .build_conditional_branch(
                                cond, nomatch_bb, cont_bb,
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        self.builder.position_at_end(nomatch_bb);
                        let err_struct_ptr = self
                            .build_alloc_bus_unmatched_key(
                                subj_val.into_pointer_value(),
                                key_lo,
                                key_hi,
                            )?;
                        // Scope locals for TypeRef expect a slot
                        // alloca that HOLDS the struct pointer
                        // (one level of indirection), not the
                        // struct pointer directly. Allocate a
                        // pointer-slot and store the struct ptr
                        // into it.
                        let err_slot = self.alloca_for(
                            &CodegenTy::TypeRef(
                                "BusUnmatchedKey".to_string(),
                            ),
                            "bus.fail.err.slot",
                        )?;
                        self.builder
                            .build_store(err_slot, err_struct_ptr)
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let mut sub_scope = Scope {
                            locals: scope.locals.clone(),
                        };
                        sub_scope.locals.insert(
                            "err".to_string(),
                            (
                                err_slot,
                                CodegenTy::TypeRef(
                                    "BusUnmatchedKey".to_string(),
                                ),
                            ),
                        );
                        let rhs_stmt = Stmt::Expr((**rhs).clone());
                        self.lower_stmt(&rhs_stmt, &mut sub_scope)?;
                        self.builder
                            .build_unconditional_branch(cont_bb)
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        self.builder.position_at_end(cont_bb);
                    }
                    Some(OrDisposition::Fail(payload_expr, _)) => {
                        // v0.2: on no-match, divert into the
                        // enclosing fallible fn's err path with
                        // the payload expression evaluated against
                        // `err: BusUnmatchedKey`. The existing
                        // `current_user_fn_fallible` infra
                        // (declared err_alloca + path_alloca +
                        // exit_bb) carries the divert.
                        let enclosing = self
                            .current_user_fn_fallible
                            .clone()
                            .ok_or_else(|| {
                                CodegenError::Unsupported(
                                    "`or fail X` on bus send: \
                                     enclosing fn must be \
                                     fallible(E)"
                                        .to_string(),
                                )
                            })?;
                        let cond = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                matched,
                                i32_t.const_zero(),
                                "bus.fail.no_match",
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .and_then(|bb| bb.get_parent())
                            .expect("inside a function");
                        let nomatch_bb = self.context.append_basic_block(
                            current_fn,
                            "bus.fail.fail_divert",
                        );
                        let cont_bb = self.context.append_basic_block(
                            current_fn,
                            "bus.fail.cont",
                        );
                        self.builder
                            .build_conditional_branch(
                                cond, nomatch_bb, cont_bb,
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        self.builder.position_at_end(nomatch_bb);
                        let err_struct_ptr = self
                            .build_alloc_bus_unmatched_key(
                                subj_val.into_pointer_value(),
                                key_lo,
                                key_hi,
                            )?;
                        let err_slot = self.alloca_for(
                            &CodegenTy::TypeRef(
                                "BusUnmatchedKey".to_string(),
                            ),
                            "bus.fail.err.slot",
                        )?;
                        self.builder
                            .build_store(err_slot, err_struct_ptr)
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let mut sub_scope = Scope {
                            locals: scope.locals.clone(),
                        };
                        sub_scope.locals.insert(
                            "err".to_string(),
                            (
                                err_slot,
                                CodegenTy::TypeRef(
                                    "BusUnmatchedKey".to_string(),
                                ),
                            ),
                        );
                        let (payload_val, _) =
                            self.lower_expr(payload_expr, &sub_scope)?;
                        self.builder
                            .build_store(
                                enclosing.err_alloca,
                                payload_val,
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let bool_t = self.context.bool_type();
                        self.builder
                            .build_store(
                                enclosing.path_alloca,
                                bool_t.const_int(1, false),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let exit_bb = self
                            .current_user_fn_exit_bb
                            .expect("exit_bb set inside fn body");
                        self.builder
                            .build_unconditional_branch(exit_bb)
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        self.builder.position_at_end(cont_bb);
                    }
                    None => {
                        return Err(CodegenError::Unsupported(
                            "fail-topic publish without `or` \
                             disposition reached codegen; typecheck \
                             should have rejected"
                                .to_string(),
                        ));
                    }
                }
                return Ok(());
            }

            let dispatch_keyed_name = if payload_is_flat {
                "lotus_bus_dispatch_keyed_flat"
            } else {
                "lotus_bus_dispatch_keyed"
            };
            let dispatch_keyed_fn = self
                .module
                .get_function(dispatch_keyed_name)
                .expect("lotus_bus_dispatch_keyed declared in declare_builtins");
            self.builder
                .build_call(
                    dispatch_keyed_fn,
                    &[
                        queue_ptr.into(),
                        subj_val.into(),
                        payload_val.into(),
                        payload_size_iv.into(),
                        ser_fn.as_global_value().as_pointer_value().into(),
                        key_lo.into(),
                        key_hi.into(),
                    ],
                    "bus.dispatch_keyed.call",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok(());
        }

        // Static-devirt (build #1b): for a compile-time-literal subject
        // that the BusGraph cleared as eligible, route the publish
        // through lotus_bus_dispatch_static(id, ...) — reads the
        // per-subject bucket directly (no g_bus_entries scan, no
        // subject strcmp), then does the SAME deferred enqueue the
        // dynamic path does. `flat` composes with the existing flat-
        // skip (verbatim vs wire fanout); `no_pinned` drops the
        // g_bus_has_pinned acquire-load when the program is provably
        // single-threaded (#3). Ineligible / computed subjects are
        // UNCHANGED (the dynamic lotus_bus_dispatch path).
        let (static_id, static_direct): (Option<u32>, bool) = match subject {
            Expr::Literal(Literal::String(s), _) => (
                self.bus_devirt_ids.get(s).copied(),
                self.bus_devirt_direct.contains(s),
            ),
            _ => (None, false),
        };
        if let Some(id) = static_id {
            let i32_t = self.context.i32_type();
            let id_iv = i32_t.const_int(id as u64, false);
            // Direct-call devirt (build #1b slice-2): when the subject's
            // every subscriber is same-thread + QUIET (the BusGraph
            // `direct_call_eligible` gate, surfaced in
            // `bus_devirt_direct`) AND its payload is FLAT (the third
            // gate leg — checked here because flatness is a lowered-
            // payload property), collapse the deferred enqueue into a
            // SYNCHRONOUS direct call to each subscriber handler. The
            // flat payload is pointer-free POD, so the publisher's live
            // storage carries the exact bytes the deferred path would
            // memcpy into a cell — we pass it straight to the handler.
            // No cell, no enqueue, no drain round-trip. Reads the SAME
            // bucket `id`, in registration order, skipping the SAME
            // quarantined / keyed entries — so the only difference from
            // the deferred static path is WHEN the (isolated, quiet)
            // handler runs, which is unobservable. A non-flat payload
            // on an otherwise-direct subject falls through to the
            // static enqueue below (managed payloads keep the wire /
            // per-subscriber-arena path).
            if static_direct && payload_is_flat {
                // Direct-INLINE (slice-3): when this direct subject has a
                // SINGLE distinct subscriber handler (one subscriber
                // locus-type, any # of instances), bake that handler as a
                // DIRECT call and walk the per-subject bucket inline via
                // the layout-safe, `pure` accessors. The optimizer hoists
                // the loop-invariant count/self-ptr loads out of a publish
                // loop and inlines the baked handler body — collapsing the
                // singleton fast path to the Go-equivalent hoisted
                // dispatch. Resolve+dedup the registered handler
                // FunctionValue exactly as `emit_bus_register` does
                // (reclaim-wrapper if present, else the raw handler) so the
                // baked call is byte-identical to the helper's indirect one.
                let baked_handler: Option<
                    inkwell::values::FunctionValue<'ctx>,
                > = match subject {
                    Expr::Literal(Literal::String(s), _) => {
                        self.bus_devirt_direct_subs.get(s).and_then(|subs| {
                            let mut distinct: Vec<
                                inkwell::values::FunctionValue<'ctx>,
                            > = Vec::new();
                            for (locus, handler) in subs {
                                let info = self.user_loci.get(locus)?;
                                // Bake the RAW locus method, NOT the
                                // `__hwrap_*` reclaim-wrapper that
                                // `emit_bus_register` registers. The wrapper is
                                // exactly `{ raw(self, payload); if
                                // self.__drain_requested != 0 { reclaim } }`,
                                // and `__drain_requested` is set ONLY by
                                // `terminate;` — which the direct-call gate's
                                // `handler_is_quiet` rejects. So for a
                                // direct-call (quiet) subject the wrapper's
                                // reclaim tail is provably DEAD and raw ≡
                                // wrapper at runtime; the differential harness
                                // is the gate. Baking raw drops the wrapper's
                                // per-dispatch dead `__drain_requested` load +
                                // branch (and the never-taken
                                // `lotus_bus_quarantine_self`/recpool reclaim
                                // path), which otherwise — having unknown
                                // memory effects — sat in the publish loop body.
                                let raw =
                                    info.user_methods.get(handler).copied()?;
                                // A view-aggregate payload param
                                // (BytesView/StringView) can't be
                                // direct-called with the slot ptr —
                                // leave those to the queue path,
                                // where __hwrap_ loads the
                                // aggregate from the slot.
                                if matches!(
                                    raw.get_type()
                                        .get_param_types()
                                        .get(1),
                                    Some(
                                        inkwell::types::BasicTypeEnum::StructType(_)
                                    )
                                ) {
                                    return None;
                                }
                                if !distinct.iter().any(|f| *f == raw) {
                                    distinct.push(raw);
                                }
                            }
                            if distinct.len() == 1 {
                                Some(distinct[0])
                            } else {
                                // 0 (defensive) or ≥2 distinct handlers:
                                // can't bake one constant → keep the helper.
                                None
                            }
                        })
                    }
                    _ => None,
                };
                if let Some(handler_fn) = baked_handler {
                    let count_fn = self
                        .module
                        .get_function("lotus_bus_static_direct_count")
                        .expect(
                            "lotus_bus_static_direct_count declared in \
                             declare_builtins",
                        );
                    let selfptr_fn = self
                        .module
                        .get_function("lotus_bus_static_direct_selfptr")
                        .expect(
                            "lotus_bus_static_direct_selfptr declared in \
                             declare_builtins",
                        );
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_parent())
                        .expect("inside a function");
                    let entry_bb = self
                        .builder
                        .get_insert_block()
                        .expect("inside a block");
                    let cond_bb = self
                        .context
                        .append_basic_block(current_fn, "bus.direct.cond");
                    let body_bb = self
                        .context
                        .append_basic_block(current_fn, "bus.direct.body");
                    let call_bb = self
                        .context
                        .append_basic_block(current_fn, "bus.direct.call");
                    let iter_bb = self
                        .context
                        .append_basic_block(current_fn, "bus.direct.iter");
                    let end_bb = self
                        .context
                        .append_basic_block(current_fn, "bus.direct.end");
                    // n = count(id)
                    let n = self
                        .builder
                        .build_call(
                            count_fn,
                            &[id_iv.into()],
                            "bus.direct.count",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("count returns i64")
                        .into_int_value();
                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // cond: k < n
                    self.builder.position_at_end(cond_bb);
                    let k_phi = self
                        .builder
                        .build_phi(i64_t, "bus.direct.k")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    k_phi.add_incoming(&[(&i64_t.const_zero(), entry_bb)]);
                    let k_val = k_phi.as_basic_value().into_int_value();
                    let more = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULT,
                            k_val,
                            n,
                            "bus.direct.more",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_conditional_branch(more, body_bb, end_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // body: sp = selfptr(id, k); skip if null
                    self.builder.position_at_end(body_bb);
                    let sp = self
                        .builder
                        .build_call(
                            selfptr_fn,
                            &[id_iv.into(), k_val.into()],
                            "bus.direct.self",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("selfptr returns ptr")
                        .into_pointer_value();
                    let is_null = self
                        .builder
                        .build_is_null(sp, "bus.direct.skip")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_conditional_branch(is_null, iter_bb, call_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // call: handler(sp, &payload) — the BAKED direct call
                    self.builder.position_at_end(call_bb);
                    self.builder
                        .build_call(
                            handler_fn,
                            &[sp.into(), payload_val.into()],
                            "bus.direct.handler",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_unconditional_branch(iter_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // iter: k = k + 1; loop
                    self.builder.position_at_end(iter_bb);
                    let k_next = self
                        .builder
                        .build_int_add(
                            k_val,
                            i64_t.const_int(1, false),
                            "bus.direct.k.next",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    k_phi.add_incoming(&[(&k_next, iter_bb)]);
                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder.position_at_end(end_bb);
                    return Ok(());
                }
                // Multi-handler (or unresolved) direct subject: keep the
                // helper (can't bake one constant). It carries the same
                // defensive off-thread post.
                let dispatch_direct_fn = self
                    .module
                    .get_function("lotus_bus_dispatch_static_direct")
                    .expect(
                        "lotus_bus_dispatch_static_direct declared in \
                         declare_builtins",
                    );
                let _ = i64_t;
                self.builder
                    .build_call(
                        dispatch_direct_fn,
                        &[
                            id_iv.into(),
                            subj_val.into(),
                            payload_val.into(),
                            payload_size_iv.into(),
                        ],
                        "bus.dispatch_static_direct.call",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                return Ok(());
            }
            let flat_iv = i32_t.const_int(payload_is_flat as u64, false);
            // no_pinned is the EXACT negation of the mark_pinned
            // condition (`program_has_offthread`) — same source of
            // truth, so the static enqueue skips the g_bus_has_pinned
            // acquire-load only when the runtime provably never sets it.
            let no_pinned_iv =
                i32_t.const_int((!self.program_has_offthread) as u64, false);
            let dispatch_static_fn = self
                .module
                .get_function("lotus_bus_dispatch_static")
                .expect("lotus_bus_dispatch_static declared in declare_builtins");
            let _ = i64_t;
            self.builder
                .build_call(
                    dispatch_static_fn,
                    &[
                        queue_ptr.into(),
                        id_iv.into(),
                        subj_val.into(),
                        payload_val.into(),
                        payload_size_iv.into(),
                        ser_fn.as_global_value().as_pointer_value().into(),
                        flat_iv.into(),
                        no_pinned_iv.into(),
                    ],
                    "bus.dispatch_static.call",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok(());
        }

        let dispatch_name = if payload_is_flat {
            "lotus_bus_dispatch_flat"
        } else {
            "lotus_bus_dispatch"
        };
        let dispatch_fn = self
            .module
            .get_function(dispatch_name)
            .expect("lotus_bus_dispatch declared in declare_builtins");
        let _ = i64_t;
        self.builder
            .build_call(
                dispatch_fn,
                &[
                    queue_ptr.into(),
                    subj_val.into(),
                    payload_val.into(),
                    payload_size_iv.into(),
                    ser_fn.as_global_value().as_pointer_value().into(),
                ],
                "bus.dispatch.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Form K4c (2026-05-20): lower a Send statement whose
    /// subject is bound to a `shm_ring(...)` transport. Routes
    /// through `lotus_bus_publish_shm_ring(subject, &value,
    /// sizeof(value))` — the C runtime owns claim + memcpy +
    /// commit.
    ///
    /// Payload must be a struct literal or a struct-typed
    /// expression that lowers to a pointer + size we can pass
    /// directly to memcpy. The stack-alloca fast path (same as
    /// the normal Send lowering) is used when the payload is a
    /// bare struct literal — the publisher's local storage is
    /// dead after publish, so no arena allocation is needed.
    fn lower_send_shm_ring(
        &mut self,
        subject: &str,
        value: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        // Stack-alloca fast path for bare struct literals
        // (mirrors the normal lower_send shape).
        let stack_payload: Option<(PointerValue<'ctx>, String)> = match value {
            Expr::Struct { path, inits, .. } => {
                let mangled: Option<String> = if path.segments.len() == 1 {
                    let name = path.segments[0].name.clone();
                    if self.user_types.contains_key(&name) {
                        Some(name)
                    } else {
                        None
                    }
                } else {
                    let segs: Vec<&str> = path
                        .segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect();
                    self.mangled_for_path(&segs).and_then(|m| {
                        if self.user_types.contains_key(&m) {
                            Some(m)
                        } else {
                            None
                        }
                    })
                };
                if let Some(mname) = mangled {
                    let info = self
                        .user_types
                        .get(&mname)
                        .cloned()
                        .expect("checked above");
                    let slot = self.alloca_in_entry(
                        info.struct_ty.into(),
                        &format!("{}.shm.send.payload", mname),
                    )?;
                    self.populate_user_type_fields(
                        &mname, &info, inits, slot, scope,
                    )?;
                    Some((slot, mname))
                } else {
                    None
                }
            }
            _ => None,
        };
        // Compute the (data pointer, byte size) the runtime frames as one
        // record. Two shapes:
        //   - a flat struct payload (the typed path): pointer to the
        //     struct + its fixed `size_of`.
        //   - a Bytes / BytesView payload (the raw / variable-length path,
        //     the producer mirror of the BytesView consumer): the value's
        //     data pointer + its actual byte length, so a heterogeneous /
        //     variable-width record can be published.
        let (value_ptr, size_iv): (PointerValue<'ctx>, inkwell::values::IntValue<'ctx>) =
            if let Some((slot, mname)) = stack_payload {
                let info = self
                    .user_types
                    .get(&mname)
                    .cloned()
                    .expect("checked above");
                let size = info
                    .struct_ty
                    .size_of()
                    .expect("flat struct has compile-time size");
                (slot, size)
            } else {
                let (v, ty) = self.lower_expr(value, scope)?;
                match ty {
                    CodegenTy::Bytes | CodegenTy::BytesView => {
                        let blob = self.unpack_view_if_needed(v, &ty)?;
                        let len_fn = self
                            .module
                            .get_function("lotus_bytes_len")
                            .expect("lotus_bytes_len declared");
                        let len = self
                            .builder
                            .build_call(len_fn, &[blob.into()], "shm.send.bytes.len")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .try_as_basic_value()
                            .left()
                            .expect("lotus_bytes_len returns i64")
                            .into_int_value();
                        let data_fn = self
                            .module
                            .get_function("lotus_bytes_data")
                            .expect("lotus_bytes_data declared");
                        let data = self
                            .builder
                            .build_call(data_fn, &[blob.into()], "shm.send.bytes.data")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .try_as_basic_value()
                            .left()
                            .expect("lotus_bytes_data returns ptr")
                            .into_pointer_value();
                        (data, len)
                    }
                    CodegenTy::TypeRef(n) => {
                        let p = match v {
                            BasicValueEnum::PointerValue(p) => p,
                            _ => {
                                return Err(CodegenError::Unsupported(format!(
                                    "shm_ring send for subject `{}`: struct payload \
                                     `{}` must lower to a pointer",
                                    subject, n
                                )));
                            }
                        };
                        let info = self.user_types.get(&n).cloned().ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "shm_ring send for subject `{}`: payload type `{}` \
                                 not registered in user_types",
                                subject, n
                            ))
                        })?;
                        let size = info
                            .struct_ty
                            .size_of()
                            .expect("flat struct has compile-time size");
                        (p, size)
                    }
                    _ => {
                        return Err(CodegenError::Unsupported(format!(
                            "shm_ring send for subject `{}`: payload must be a \
                             struct, Bytes, or BytesView value; got {:?}",
                            subject, ty
                        )));
                    }
                }
            };
        let subj_ptr = self
            .builder
            .build_global_string_ptr(
                subject,
                &format!("lotus.shm_ring.send.subject.{}", subject),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        // Proposal B M3a: a `layout:`-bound topic frames a foreign
        // `byte_records` record via the layout publish path; the
        // native ring uses claim + memcpy + commit. Both take the
        // same (subject, value, size) arguments.
        let is_layout = self
            .shm_ring_subjects
            .get(subject)
            .map(|info| info.layout.is_some())
            .unwrap_or(false);
        let publish_fn_name = if is_layout {
            "lotus_bus_publish_shm_ring_layout"
        } else {
            "lotus_bus_publish_shm_ring"
        };
        let publish_fn = self
            .module
            .get_function(publish_fn_name)
            .unwrap_or_else(|| panic!("{} declared", publish_fn_name));
        self.builder
            .build_call(
                publish_fn,
                &[
                    subj_ptr.into(),
                    value_ptr.into(),
                    size_iv.into(),
                ],
                "bus.shm_ring.publish.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

}
