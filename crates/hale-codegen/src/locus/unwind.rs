//! A failed assertion tears the tree down (spec/testing.md, "What runs
//! after a failed assertion").
//!
//! An assertion that fails in `fn main`'s own frame leaves through
//! main's test-failure exit (`main.test_fail`, GH #717). One that fails
//! anywhere else has no branch of main's to take: the runtime keeps a
//! registry of what main's teardown would reach — every frame entry a
//! body on the main thread holds and every statement-position literal
//! whose `run()` is in progress — and on the failure runs each one's
//! teardown, then exits 1 (`lotus_test_fail_unwind`).
//!
//! An instance is registered once it is built (`emit_unwind_push`) and
//! removed where its own teardown begins (`emit_unwind_pop`), with the
//! teardown of its declaration: `__unwind_<L>(slot, tid)`, the deferred
//! entry's spine over the slot that holds it (`emit_deferred_entry_
//! teardown`), synthesized at the end of lowering. The upkeep is emitted
//! behind `lotus.test_unwind`, a private flag the end of lowering sets
//! only when an assertion outside main's frame was lowered, so every
//! other program folds it away; the teardown functions are synthesized
//! only then.

use inkwell::module::Linkage;
use inkwell::values::{GlobalValue, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, Cx};

const GATE: &str = "lotus.test_unwind";

impl<'ctx, 'p> Cx<'ctx, 'p> {
    fn unwind_gate(&mut self) -> GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global(GATE) {
            return g;
        }
        let i8_t = self.context.i8_type();
        let g = self.module.add_global(i8_t, None, GATE);
        g.set_linkage(Linkage::Private);
        g.set_initializer(&i8_t.const_zero());
        g
    }

    /// Emit `body` behind the registry's flag: a load of a private global
    /// the end of lowering writes, which LLVM folds away when it is 0.
    fn emit_unwind_guarded(
        &mut self,
        tag: &str,
        body: impl FnOnce(&mut Self) -> Result<(), CodegenError>,
    ) -> Result<(), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let gate = self.unwind_gate();
        let i8_t = self.context.i8_type();
        let on = self
            .builder
            .build_load(i8_t, gate.as_pointer_value(), &format!("unwind.{tag}.gate"))
            .map_err(e)?
            .into_int_value();
        let on = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, on, i8_t.const_zero(), &format!("unwind.{tag}.on"))
            .map_err(e)?;
        let func = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .expect("the registry's upkeep is inside a function");
        let do_bb = self.context.append_basic_block(func, &format!("unwind.{tag}"));
        let cont_bb = self.context.append_basic_block(func, &format!("unwind.{tag}.cont"));
        self.builder.build_conditional_branch(on, do_bb, cont_bb).map_err(e)?;
        self.builder.position_at_end(do_bb);
        body(self)?;
        self.builder.build_unconditional_branch(cont_bb).map_err(e)?;
        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// The function-pointer global the record of an instance of
    /// `locus_name` names (filled at the end of lowering).
    fn unwind_fn_global(&mut self, locus_name: &str, pinned: bool) -> GlobalValue<'ctx> {
        let key = (locus_name.to_string(), pinned);
        if let Some(g) = self.unwind_fns.get(&key) {
            return *g;
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let name = format!("lotus.unwind_fn.{}{}", locus_name, if pinned { ".pinned" } else { "" });
        let g = self.module.add_global(ptr_t, None, &name);
        g.set_linkage(Linkage::Private);
        g.set_initializer(&ptr_t.const_null());
        self.unwind_fns.insert(key, g);
        g
    }

    /// Register the instance `slot` holds with the main thread's unwind
    /// registry: a failed assertion outside main's frame owes its
    /// teardown. `tid` is a pinned instance's thread-id slot.
    pub(crate) fn emit_unwind_push(
        &mut self,
        slot: PointerValue<'ctx>,
        locus_name: &str,
        tid: Option<PointerValue<'ctx>>,
    ) -> Result<(), CodegenError> {
        let g = self.unwind_fn_global(locus_name, tid.is_some());
        self.emit_unwind_guarded("push", |cx| {
            let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
            let ptr_t = cx.context.ptr_type(AddressSpace::default());
            let fnp = cx.builder.build_load(ptr_t, g.as_pointer_value(), "unwind.fn").map_err(e)?;
            let push = cx.module.get_function("lotus_unwind_push").expect("lotus_unwind_push declared");
            let tid_v = tid.unwrap_or_else(|| ptr_t.const_null());
            cx.builder.build_call(push, &[fnp.into(), slot.into(), tid_v.into()], "unwind.push").map_err(e)?;
            Ok(())
        })
    }

    /// The instance `slot` holds begins its own teardown: a failure no
    /// longer owes it.
    pub(crate) fn emit_unwind_pop(&mut self, slot: PointerValue<'ctx>) -> Result<(), CodegenError> {
        self.emit_unwind_guarded("pop", |cx| {
            let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
            let pop = cx.module.get_function("lotus_unwind_pop_slot").expect("lotus_unwind_pop_slot declared");
            cx.builder.build_call(pop, &[slot.into()], "unwind.pop").map_err(e)?;
            Ok(())
        })
    }

    /// A park point (a sleep slice, `yield;`): on main, a failure another
    /// thread raised is torn down here (`lotus_unwind_poll`).
    pub(crate) fn emit_unwind_poll(&mut self) -> Result<(), CodegenError> {
        self.emit_unwind_guarded("poll", |cx| {
            let poll = cx.module.get_function("lotus_unwind_poll").expect("lotus_unwind_poll declared");
            cx.builder
                .build_call(poll, &[], "unwind.poll")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            Ok(())
        })
    }

    /// `fn main`'s prelude: name the head of the test-failure teardown
    /// (`lotus.unwind_head`, filled at the end of lowering) and main's
    /// thread.
    pub(crate) fn emit_unwind_set_head(&mut self) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let g = match self.module.get_global("lotus.unwind_head") {
            Some(g) => g,
            None => {
                let g = self.module.add_global(ptr_t, None, "lotus.unwind_head");
                g.set_linkage(Linkage::Private);
                g.set_initializer(&ptr_t.const_null());
                g
            }
        };
        self.emit_unwind_guarded("head", |cx| {
            let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
            let head = cx.builder.build_load(ptr_t, g.as_pointer_value(), "unwind.head").map_err(e)?;
            let set = cx.module.get_function("lotus_unwind_set_head").expect("lotus_unwind_set_head declared");
            cx.builder.build_call(set, &[head.into()], "unwind.set_head").map_err(e)?;
            Ok(())
        })
    }

    /// The exit code `fn main` returns: `code`, or 1 when an assertion
    /// that failed off main's frame (on another thread, which asks main
    /// to tear down at its next drain point) left the latch up and main
    /// reached an exit first.
    pub(crate) fn emit_unwind_exit_code(
        &mut self,
        code: inkwell::values::IntValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let gate = self.unwind_gate();
        let i8_t = self.context.i8_type();
        let i64_t = self.context.i64_type();
        let on = self.builder.build_load(i8_t, gate.as_pointer_value(), "unwind.exit.gate").map_err(e)?.into_int_value();
        let on = self.builder.build_int_compare(inkwell::IntPredicate::NE, on, i8_t.const_zero(), "unwind.exit.on").map_err(e)?;
        let failed_fn = self.module.get_function("lotus_test_failed").expect("lotus_test_failed declared");
        let failed = self
            .builder
            .build_call(failed_fn, &[], "unwind.exit.failed")
            .map_err(e)?
            .try_as_basic_value()
            .left()
            .expect("lotus_test_failed returns i64")
            .into_int_value();
        let failed = self.builder.build_int_compare(inkwell::IntPredicate::NE, failed, i64_t.const_zero(), "unwind.exit.latched").map_err(e)?;
        let both = self.builder.build_and(on, failed, "unwind.exit.fail").map_err(e)?;
        let one = code.get_type().const_int(1, false);
        Ok(self.builder.build_select(both, one, code, "unwind.exit.code").map_err(e)?.into_int_value())
    }

    /// The end of lowering: the registry is in use iff an assertion
    /// outside main's frame was lowered. Then its flag is up, every
    /// declaration a record names gets its `__unwind_<L>`, and main's
    /// prelude names the test-failure exit's head.
    pub(crate) fn finish_unwind_registry(&mut self) -> Result<(), CodegenError> {
        let Some(gate) = self.module.get_global(GATE) else { return Ok(()) };
        if !self.test_unwind_used {
            return Ok(());
        }
        let i8_t = self.context.i8_type();
        gate.set_initializer(&i8_t.const_int(1, false));
        let saved_block = self.builder.get_insert_block();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let fn_ty = self.context.void_type().fn_type(&[ptr_t.into(), ptr_t.into()], false);
        for ((locus_name, pinned), g) in self.unwind_fns.clone() {
            let name = format!("__unwind_{}{}", locus_name, if pinned { "_pinned" } else { "" });
            let f = self.module.add_function(&name, fn_ty, Some(Linkage::Private));
            let entry = self.context.append_basic_block(f, "entry");
            self.builder.position_at_end(entry);
            self.di_begin_function();
            let slot = f.get_nth_param(0).expect("slot param").into_pointer_value();
            let tid = f.get_nth_param(1).expect("tid param").into_pointer_value();
            let prev_fn = self.current_fn.replace(f);
            let prev_self = self.current_self.take();
            let prev_main = std::mem::replace(&mut self.in_main, false);
            let r = self.emit_deferred_entry_teardown(slot, &locus_name, pinned.then_some(tid), true);
            self.current_fn = prev_fn;
            self.current_self = prev_self;
            self.in_main = prev_main;
            r?;
            self.builder.build_return(None).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            g.set_initializer(&f.as_global_value().as_pointer_value());
        }
        if let Some(head_g) = self.module.get_global("lotus.unwind_head") {
            let head = self.module.add_function("__unwind_head", self.context.void_type().fn_type(&[], false), Some(Linkage::Private));
            let entry = self.context.append_basic_block(head, "entry");
            self.builder.position_at_end(entry);
            self.di_begin_function();
            let prev_fn = self.current_fn.replace(head);
            let prev_main = std::mem::replace(&mut self.in_main, false);
            let r = self.emit_teardown_obligations(hale_types::lifecycle::Spine::MainTestFailure);
            self.current_fn = prev_fn;
            self.in_main = prev_main;
            r?;
            self.builder.build_return(None).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            head_g.set_initializer(&head.as_global_value().as_pointer_value());
        }
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }
        Ok(())
    }
}
