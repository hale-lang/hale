//! The lifecycle trace's emission side (F.40 phase 3, L2).
//!
//! Under `BuildOptions::lifecycle_trace` every traced lifecycle step is
//! emitted as
//!
//! ```text
//! if (lotus_lc_enter(kind, self, spine, type)) {
//!     <the step>
//!     lotus_lc_ev(kind, "Completed", self, spine, type);
//! }
//! ```
//!
//! `lotus_lc_enter` writes the `Entered` line and answers 1, or answers
//! 0 when the debug runtime's `LOTUS_LIFECYCLE_SKIP` names the kind: the
//! step and both its events are then gone, which is how a negative
//! control removes a step. The runtime mints the instance and its
//! incarnation from `self` (`lotus_arena.c` § The lifecycle trace); the
//! compiler passes only the kind, the spine and the declaration's name.
//!
//! With the knob off, [`Cx::lc_step`] emits the step alone and nothing
//! of the trace is declared, so the IR is the release build's.

use inkwell::values::PointerValue;
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, Cx, LocusInfo};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Emit `step` as the traced obligation `kind` of the instance at
    /// `self_ptr` (of declaration `ty`), or of the process when
    /// `self_ptr` is `None`. The step must leave the builder in an
    /// unterminated block.
    pub(crate) fn lc_step(
        &mut self,
        kind: &'static str,
        self_ptr: Option<PointerValue<'ctx>>,
        ty: Option<&str>,
        step: impl FnOnce(&mut Self) -> Result<(), CodegenError>,
    ) -> Result<(), CodegenError> {
        if !self.lifecycle_trace {
            return step(self);
        }
        let func = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("a traced step is emitted inside a function");
        let args = self.lc_args(kind, self_ptr, ty);
        let enter = self.lc_fn("lotus_lc_enter", true);
        let go = self
            .builder
            .build_call(enter, &[args[0].into(), args[1].into(), args[2].into(), args[3].into()], "lc.enter")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_lc_enter returns i32")
            .into_int_value();
        let zero = self.context.i32_type().const_zero();
        let take = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, go, zero, "lc.take")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let step_bb = self.context.append_basic_block(func, &format!("lc.{kind}"));
        let after_bb = self.context.append_basic_block(func, &format!("lc.{kind}.after"));
        self.builder
            .build_conditional_branch(take, step_bb, after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(step_bb);
        step(self)?;
        self.lc_event_with(kind, "Completed", &args)?;
        self.builder
            .build_unconditional_branch(after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(after_bb);
        Ok(())
    }

    /// One event line with no step around it: an `Entered` whose
    /// completion another place emits, or a terminal.
    pub(crate) fn lc_event(
        &mut self,
        kind: &'static str,
        point: &'static str,
        self_ptr: Option<PointerValue<'ctx>>,
        ty: Option<&str>,
    ) -> Result<(), CodegenError> {
        if !self.lifecycle_trace {
            return Ok(());
        }
        let args = self.lc_args(kind, self_ptr, ty);
        self.lc_event_with(kind, point, &args)
    }

    /// A teardown spine's own two steps for one instance: `drain()`
    /// (Drain), then the dissolve-epoch closures and `dissolve()`
    /// (Dissolve). An empty or absent method is still the step's
    /// position, so the trace reports the step either way.
    pub(crate) fn emit_traced_drain_dissolve(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let method = |name: &str| info.methods.get(name).copied().filter(|_| !info.empty_lifecycle.contains(name));
        let drain_call = method("drain");
        let dissolve_call = method("dissolve");
        let closures = info.dissolve_closures_fn;
        self.lc_step("Drain", Some(self_ptr), Some(locus_name), |cx| {
            if let Some(drain_fn) = drain_call {
                cx.builder
                    .build_call(drain_fn, &[self_ptr.into()], &format!("{}.drain.call", locus_name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            Ok(())
        })?;
        self.lc_step("Dissolve", Some(self_ptr), Some(locus_name), |cx| {
            if let Some(closures_fn) = closures {
                let (parent_self, handler_ptr) = cx.resolve_failure_route(locus_name);
                cx.builder
                    .build_call(
                        closures_fn,
                        &[self_ptr.into(), parent_self.into(), handler_ptr.into()],
                        &format!("{}.__dissolve_closures.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            if let Some(dissolve_fn) = dissolve_call {
                cx.builder
                    .build_call(dissolve_fn, &[self_ptr.into()], &format!("{}.dissolve.call", locus_name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            Ok(())
        })
    }

    /// An instance's event under the spine already set: the Reclaim
    /// pair the arena chokepoint emits past its latch.
    pub(crate) fn lc_in_spine_event(
        &mut self,
        kind: &'static str,
        point: &'static str,
        self_ptr: PointerValue<'ctx>,
        ty: &str,
    ) -> Result<(), CodegenError> {
        self.lc_event(kind, point, Some(self_ptr), Some(ty))
    }

    /// At a pinned locus's thread start: the trace names the thread
    /// `pinned:<n>` whether or not it serves a mailbox.
    pub(crate) fn lc_pinned_thread(&mut self) -> Result<(), CodegenError> {
        if !self.lifecycle_trace {
            return Ok(());
        }
        let f = match self.module.get_function("lotus_lc_pinned_thread") {
            Some(f) => f,
            None => {
                let ty = self.context.void_type().fn_type(&[], false);
                self.module.add_function("lotus_lc_pinned_thread", ty, None)
            }
        };
        self.builder.build_call(f, &[], "").map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Run `body` with the trace naming `spine` as the steps' spine.
    pub(crate) fn lc_in_spine<T>(
        &mut self,
        spine: &'static str,
        body: impl FnOnce(&mut Self) -> Result<T, CodegenError>,
    ) -> Result<T, CodegenError> {
        let outer = std::mem::replace(&mut self.lc_spine, spine);
        let r = body(self);
        self.lc_spine = outer;
        r
    }

    fn lc_event_with(
        &mut self,
        _kind: &'static str,
        point: &'static str,
        args: &[PointerValue<'ctx>; 4],
    ) -> Result<(), CodegenError> {
        let ev = self.lc_fn("lotus_lc_ev", false);
        let point = self.global_string(point);
        self.builder
            .build_call(
                ev,
                &[args[0].into(), point.into(), args[1].into(), args[2].into(), args[3].into()],
                "",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// (kind, self, spine, type) as the runtime takes them.
    fn lc_args(
        &mut self,
        kind: &'static str,
        self_ptr: Option<PointerValue<'ctx>>,
        ty: Option<&str>,
    ) -> [PointerValue<'ctx>; 4] {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let kind = self.global_string(kind);
        let spine = self.lc_spine;
        let spine = self.global_string(spine);
        let ty = match ty {
            Some(t) => self.global_string(t),
            None => ptr_t.const_null(),
        };
        [kind, self_ptr.unwrap_or_else(|| ptr_t.const_null()), spine, ty]
    }

    /// `lotus_lc_enter` (i32 result) or `lotus_lc_ev` (void), declared
    /// on first use so a build without the trace declares neither.
    fn lc_fn(&mut self, name: &str, enter: bool) -> inkwell::values::FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let ty = if enter {
            self.context
                .i32_type()
                .fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into()], false)
        } else {
            self.context
                .void_type()
                .fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into()], false)
        };
        self.module.add_function(name, ty, None)
    }
}
