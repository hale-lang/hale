//! Bus runtime hooks: queue drain, queue destroy, payload-type
//! registration (legacy + shm_ring). Round 3c of the codegen
//! model-org refactor.

use hale_syntax::ast::KeyFilter;
use inkwell::values::{FunctionValue, PointerValue};
use inkwell::AddressSpace;

use crate::bus::dispatch::BusDispatch;
use crate::codegen::{CodegenError, Cx};

pub(crate) trait BusRuntime<'ctx> {
    fn emit_bus_drain(&mut self) -> Result<(), CodegenError>;
    fn emit_bus_queue_destroy(&mut self) -> Result<(), CodegenError>;
    fn emit_bus_register_shm_ring(
        &mut self,
        subject: &str,
        self_ptr: PointerValue<'ctx>,
        handler_fn: FunctionValue<'ctx>,
    ) -> Result<(), CodegenError>;
    fn emit_bus_register(
        &mut self,
        subject: &str,
        self_ptr: PointerValue<'ctx>,
        handler_fn: FunctionValue<'ctx>,
        mailbox_or_null: Option<PointerValue<'ctx>>,
        payload_type: &str,
        key_filter: Option<&KeyFilter>,
        owned_beyond_scope: bool,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> BusRuntime<'ctx> for Cx<'ctx, 'p> {
    /// Pop the top deferred-dissolve frame and emit its drain →
    /// dissolve calls in reverse instantiation order. Called just
    /// before the body's final `ret` so the alloca slots are still
    /// live when their drain/dissolve methods read self.X.
    /// Emit a call to drain the cooperative-scheduler bus queue.
    /// Pops every enqueued (handler, self, payload) cell and
    /// invokes the handler. Handlers may enqueue more cells —
    /// the drain loop in the C runtime continues until the
    /// queue is empty at pop time. Called at the start of every
    /// `flush_dissolve_frame` so cooperative subscribers process
    /// pending cells BEFORE they themselves dissolve.
    fn emit_bus_drain(&mut self) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let queue_global = self
            .module
            .get_global("lotus.bus_queue.global")
            .expect("bus queue global declared");
        let queue_ptr = self
            .builder
            .build_load(
                ptr_t,
                queue_global.as_pointer_value(),
                "queue.cur",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let drain_fn = self
            .module
            .get_function("lotus_bus_queue_drain")
            .expect("lotus_bus_queue_drain declared");
        self.builder
            .build_call(
                drain_fn,
                &[queue_ptr.into()],
                "bus.queue.drain",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    fn emit_bus_queue_destroy(&mut self) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let queue_global = self
            .module
            .get_global("lotus.bus_queue.global")
            .expect("bus queue global declared");
        let queue_ptr = self
            .builder
            .build_load(
                ptr_t,
                queue_global.as_pointer_value(),
                "queue.destroy.cur",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let destroy_fn = self
            .module
            .get_function("lotus_bus_queue_destroy")
            .expect("lotus_bus_queue_destroy declared");
        self.builder
            .build_call(
                destroy_fn,
                &[queue_ptr.into()],
                "bus.queue.destroy.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let router_destroy_fn = self
            .module
            .get_function("lotus_bus_router_destroy")
            .expect("lotus_bus_router_destroy declared");
        self.builder
            .build_call(
                router_destroy_fn,
                &[],
                "bus.router.destroy.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // F.31 Phase 4: free pool structs + ring buffers. Runs
        // after shutdown_all (workers joined) + router_destroy
        // (no more dispatch attempts).
        if !self.main_cooperative_pools.is_empty() {
            let destroy_all_fn = self
                .module
                .get_function("lotus_coop_pool_destroy_all")
                .expect("lotus_coop_pool_destroy_all declared");
            self.builder
                .build_call(destroy_all_fn, &[], "coop_pool.destroy_all")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        Ok(())
    }

    /// Form K6b (2026-05-20): emit a shm_ring subscriber
    /// registration. Mirror of `emit_bus_register` for the
    /// shm_ring transport — instead of routing through the
    /// bus queue, this spawns a per-subject reader thread that
    /// directly calls `handler_fn(self_ptr, slot_ptr)` on each
    /// new published seqno.
    ///
    /// The handler fn signature codegen produces is exactly
    /// `void(void *self, void *payload)` — same as the
    /// existing `lotus_bus_register` handler signature, so the
    /// user's `fn on_foo(p: Payload)` lowers identically. The
    /// only difference is which runtime fn the registration
    /// call lands on.
    fn emit_bus_register_shm_ring(
        &mut self,
        subject: &str,
        self_ptr: PointerValue<'ctx>,
        handler_fn: FunctionValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let info = self
            .shm_ring_subjects
            .get(subject)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "emit_bus_register_shm_ring: subject `{}` not in \
                     shm_ring_subjects (pre-pass missed it?)",
                    subject
                ))
            })?;
        // Proposal B (2026-06-06): a `layout:`-bound subscriber reads
        // a foreign ring. Build the descriptor from the resolved
        // `ring_layout` and register through the layout-aware path;
        // the native LRSRNG1 path below is left untouched.
        if let Some(layout) = info.layout.clone() {
            return self.emit_bus_register_shm_ring_layout(
                subject, &info.shm_name, &layout, self_ptr, handler_fn,
            );
        }
        let payload_info = self
            .user_types
            .get(&info.payload_type_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "shm_ring subscribe `{}`: payload type `{}` not \
                     registered in user_types",
                    subject, info.payload_type_name
                ))
            })?;
        let slot_size = payload_info
            .struct_ty
            .size_of()
            .expect("flat struct has compile-time size");
        let slot_count = self.context.i64_type().const_int(info.slot_count, false);

        let subj_ptr = self
            .builder
            .build_global_string_ptr(
                subject,
                &format!("lotus.shm_ring.sub.subject.{}", subject),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let name_ptr = self
            .builder
            .build_global_string_ptr(
                &info.shm_name,
                &format!("lotus.shm_ring.sub.name.{}", subject),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        let reg_fn = self
            .module
            .get_function("lotus_bus_register_subscriber_shm_ring")
            .expect("lotus_bus_register_subscriber_shm_ring declared");
        self.builder
            .build_call(
                reg_fn,
                &[
                    subj_ptr.into(),
                    slot_size.into(),
                    slot_count.into(),
                    name_ptr.into(),
                    self_ptr.into(),
                    handler_ptr.into(),
                ],
                "shm_ring.sub.register",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    fn emit_bus_register(
        &mut self,
        subject: &str,
        self_ptr: PointerValue<'ctx>,
        handler_fn: FunctionValue<'ctx>,
        mailbox_or_null: Option<PointerValue<'ctx>>,
        payload_type: &str,
        key_filter: Option<&KeyFilter>,
        owned_beyond_scope: bool,
    ) -> Result<(), CodegenError> {
        let _ = self
            .bus_state
            .expect("subscriptions registered ⇒ bus_state initialized");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let subj_str = self.global_string(subject);
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        let mailbox_val = mailbox_or_null.unwrap_or_else(|| ptr_t.const_null());
        // F.36 Slice 3b: subscribe-side codec substitution. When
        // main has a `codec(L { ... })` clause for this subject,
        // the synthesized decode thunk replaces the default m70
        // deserializer. Both ptrs match `lotus_deserialize_fn`'s
        // ABI, so the runtime dispatch path (reader threads +
        // `lotus_bus_dispatch_wire`) is untouched.
        let deserialize_ptr = match self.codec_thunks.get(subject) {
            Some(thunks) => thunks.decode.as_global_value().as_pointer_value(),
            None => self
                .serializers
                .get(payload_type)
                .ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "no serializer synthesized for bus payload type `{}` — \
                         m60 should have created one in pass A3",
                        payload_type
                    ))
                })?
                .deserialize
                .as_global_value()
                .as_pointer_value(),
        };
        // Compute the optional coop_pool ptr (F.31 Phase 4) and
        // the Phase 3 key-filter triple (kind, lo, hi) up front,
        // then funnel through lotus_bus_register_keyed which
        // accepts both. Lotus_bus_register / _with_pool both
        // delegate to _keyed internally for kind=0 (no filter)
        // → backward compat preserved on the runtime side.
        let coop_pool_ptr = if let Some(pool_name) =
            self.current_cooperative_pool.clone()
        {
            let name_str = self.global_string(&pool_name);
            let lookup_fn = self
                .module
                .get_function("lotus_coop_pool_lookup")
                .expect("lotus_coop_pool_lookup declared in declare_builtins");
            self.builder
                .build_call(lookup_fn, &[name_str.into()], "coop_pool.lookup")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_coop_pool_lookup returns ptr")
                .into_pointer_value()
        } else if owned_beyond_scope {
            // Pool-inheritance fix (2026-05-29): no compile-time
            // placement name (this subscribe registers from inside
            // a method/handler body, not a main-locus params
            // field), but the locus is owned beyond this scope
            // (accept'd / field-owned / returned), so it outlives
            // the handler. Tag the subscription with the pool whose
            // worker is currently on-CPU — for a child instantiated
            // inside a pool worker, that's the parent's pool, so
            // dispatch routes to the right worker instead of
            // silently falling to the global queue (which only
            // fires if main happens to drain it). Returns null on
            // the main thread → unchanged for genuine main-pool
            // subscribers. Gated on ownership: a handler-local
            // `let`-bound subscriber is deregistered at scope exit,
            // so pool-tagging it would route to a worker that
            // drains it only after it's gone — keep those on the
            // global queue (prior behavior).
            let current_fn = self
                .module
                .get_function("lotus_coop_pool_current")
                .expect("lotus_coop_pool_current declared in declare_builtins");
            self.builder
                .build_call(current_fn, &[], "coop_pool.current")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_coop_pool_current returns ptr")
                .into_pointer_value()
        } else {
            ptr_t.const_null()
        };

        let i8_t = self.context.i8_type();
        let i64_t = self.context.i64_type();
        let (kind_v, key_lo_v, key_hi_v) =
            self.lower_subscribe_key_filter(self_ptr, key_filter, subject)?;
        let kind_iv = i8_t.const_int(kind_v as u64, false);

        let register_keyed_fn = self
            .module
            .get_function("lotus_bus_register_keyed")
            .expect("lotus_bus_register_keyed declared in declare_builtins");
        self.builder
            .build_call(
                register_keyed_fn,
                &[
                    subj_str.into(),
                    self_ptr.into(),
                    handler_ptr.into(),
                    mailbox_val.into(),
                    deserialize_ptr.into(),
                    coop_pool_ptr.into(),
                    kind_iv.into(),
                    key_lo_v.into(),
                    key_hi_v.into(),
                ],
                "bus.register_keyed.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let _ = i64_t; // silence unused if branches diverge later
        Ok(())
    }

}

/// Width in bytes of a `ring_layout` scalar/len repr token. The
/// repr set is validated at typecheck (hale-types::check_ring_layout);
/// an unrecognized token here defaults to 8 (the widest), which is
/// harmless — the layout would already have been rejected upstream.
fn ring_repr_width(repr: &str) -> u64 {
    match repr {
        "u8" | "i8" => 1,
        "u16" | "i16" => 2,
        "u32" | "i32" | "f32" => 4,
        "u64" | "i64" | "f64" | "atomic_u64" => 8,
        _ => 8,
    }
}

/// Build the flat 16-entry descriptor (the slot contract documented
/// on `lotus_bus_register_subscriber_shm_ring_layout` in
/// `lotus_shm_ring.c`) from a resolved `ring_layout`.
///
/// Field roles are read by convention from the declared layout:
///   - `magic N;` → expected header magic at offset 0.
///   - a scalar named `version` with an `expect` → version check.
///   - a scalar named `buffer_size` → ring capacity source.
///   - the first `cursor`'s `at` → the published byte cursor offset.
///   - the `byte_records` framing's `len_prefix`/`align`/
///     `pad_sentinel` → record framing.
fn ring_layout_descriptor_words(
    decl: &hale_syntax::ast::RingLayoutDecl,
) -> [u64; 16] {
    use hale_syntax::ast::RingAttrValue;
    let mut w = [0u64; 16];

    // [0..2] magic
    if let Some(m) = decl.magic {
        w[0] = m as u64;
        w[1] = 1;
    }

    // [2..6] version: scalar named "version" carrying an expect.
    if let Some(v) = decl
        .scalars
        .iter()
        .find(|s| s.name.name == "version" && s.expect.is_some())
    {
        w[2] = v.at as u64;
        w[3] = ring_repr_width(&v.repr.name);
        w[4] = v.expect.unwrap_or(0) as u64;
        w[5] = 1;
    }

    // [6..9] buffer_size: scalar named "buffer_size".
    if let Some(b) = decl.scalars.iter().find(|s| s.name.name == "buffer_size") {
        w[6] = b.at as u64;
        w[7] = ring_repr_width(&b.repr.name);
        w[8] = 1;
    }

    // [9] data_at
    w[9] = decl.data_at.unwrap_or(0) as u64;

    // [10] cursor offset (first cursor's `at` attr)
    if let Some(c) = decl.cursors.first() {
        for a in &c.attrs {
            if a.key.name == "at" {
                if let RingAttrValue::Int(n) = &a.value {
                    w[10] = *n as u64;
                }
            }
        }
    }

    // [11..15] framing: len_prefix width, align, pad_sentinel.
    w[12] = 1; // align default (no sub-record padding)
    if let Some(f) = &decl.framing {
        for a in &f.attrs {
            match (a.key.name.as_str(), &a.value) {
                ("len_prefix", RingAttrValue::Ident(id)) => {
                    w[11] = ring_repr_width(&id.name);
                }
                ("len_prefix", RingAttrValue::Int(n)) => {
                    w[11] = *n as u64;
                }
                ("align", RingAttrValue::Int(n)) => {
                    w[12] = *n as u64;
                }
                ("pad_sentinel", RingAttrValue::Int(n)) => {
                    w[13] = *n as u64;
                    w[14] = 1;
                }
                _ => {}
            }
        }
    }

    w
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Proposal B (2026-06-06): register a subscriber on a
    /// `layout:`-bound shm_ring topic. Emits a private global holding
    /// the 16-entry descriptor built from the resolved `ring_layout`
    /// and calls `lotus_bus_register_subscriber_shm_ring_layout`,
    /// which attaches the foreign ring read-only and spawns the
    /// `byte_records` reader thread.
    /// Emit a private global holding the 16-entry descriptor built
    /// from a resolved `ring_layout`, returning a pointer to it. Both
    /// the subscriber (attach) and producer (create) register paths
    /// hand this pointer to the runtime.
    pub(crate) fn ring_layout_desc_global(
        &mut self,
        subject: &str,
        layout: &hale_syntax::ast::RingLayoutDecl,
    ) -> PointerValue<'ctx> {
        let i64_t = self.context.i64_type();
        let words = ring_layout_descriptor_words(layout);
        let const_vals: Vec<inkwell::values::IntValue<'ctx>> =
            words.iter().map(|wrd| i64_t.const_int(*wrd, false)).collect();
        let arr = i64_t.const_array(&const_vals);
        let arr_ty = i64_t.array_type(words.len() as u32);
        let desc_g = self.module.add_global(
            arr_ty,
            None,
            &format!("lotus.shm_ring.layout.desc.{}", subject),
        );
        desc_g.set_initializer(&arr);
        desc_g.set_constant(true);
        desc_g.set_linkage(inkwell::module::Linkage::Private);
        desc_g.as_pointer_value()
    }

    fn emit_bus_register_shm_ring_layout(
        &mut self,
        subject: &str,
        shm_name: &str,
        layout: &hale_syntax::ast::RingLayoutDecl,
        self_ptr: PointerValue<'ctx>,
        handler_fn: FunctionValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let desc_ptr = self.ring_layout_desc_global(subject, layout);

        let subj_ptr = self
            .builder
            .build_global_string_ptr(
                subject,
                &format!("lotus.shm_ring.layout.subject.{}", subject),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let name_ptr = self
            .builder
            .build_global_string_ptr(
                shm_name,
                &format!("lotus.shm_ring.layout.name.{}", subject),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        let reg_fn = self
            .module
            .get_function("lotus_bus_register_subscriber_shm_ring_layout")
            .expect("lotus_bus_register_subscriber_shm_ring_layout declared");
        self.builder
            .build_call(
                reg_fn,
                &[
                    subj_ptr.into(),
                    name_ptr.into(),
                    desc_ptr.into(),
                    self_ptr.into(),
                    handler_ptr.into(),
                ],
                "shm_ring.sub.register_layout",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Proposal B M3a (2026-06-06): register a PRODUCER for a
    /// `layout:`-bound topic the bundle publishes. Emits the
    /// descriptor global + a `lotus_bus_register_shm_ring_layout`
    /// call, which CREATES the foreign ring (this process owns it).
    /// `capacity` is the data-region size in bytes.
    pub(crate) fn emit_bus_register_shm_ring_layout_producer(
        &mut self,
        subject: &str,
        shm_name: &str,
        layout: &hale_syntax::ast::RingLayoutDecl,
        capacity: u64,
    ) -> Result<(), CodegenError> {
        let desc_ptr = self.ring_layout_desc_global(subject, layout);
        let subj_ptr = self
            .builder
            .build_global_string_ptr(
                subject,
                &format!("lotus.shm_ring.layout.psubject.{}", subject),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let name_ptr = self
            .builder
            .build_global_string_ptr(
                shm_name,
                &format!("lotus.shm_ring.layout.pname.{}", subject),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let cap_val = self.context.i64_type().const_int(capacity, false);
        let reg_fn = self
            .module
            .get_function("lotus_bus_register_shm_ring_layout")
            .expect("lotus_bus_register_shm_ring_layout declared");
        self.builder
            .build_call(
                reg_fn,
                &[
                    subj_ptr.into(),
                    name_ptr.into(),
                    desc_ptr.into(),
                    cap_val.into(),
                ],
                "shm_ring.register_layout_producer",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }
}
