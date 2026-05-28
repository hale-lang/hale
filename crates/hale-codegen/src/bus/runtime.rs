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
    ) -> Result<(), CodegenError> {
        let _ = self
            .bus_state
            .expect("subscriptions registered ⇒ bus_state initialized");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let subj_str = self.global_string(subject);
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        let mailbox_val = mailbox_or_null.unwrap_or_else(|| ptr_t.const_null());
        let deserialize_ptr = self
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
            .as_pointer_value();
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
