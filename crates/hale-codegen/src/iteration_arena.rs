//! Per-iteration reclaim of a loop body's values.
//!
//! A frame with a per-call scratch (a locus method, a lifecycle hook, a
//! handler) used to allocate everything its loops made into that scratch,
//! freed only when the frame returned: a `run()` that loops for its
//! locus's life kept every iteration's values until the locus dissolved
//! (`buf.snapshot()` per iteration over a 1 GiB file held about 1 GB),
//! while the same body moved into a method stayed flat, since the method's
//! own scratch dies with each call.
//!
//! A loop body that `hale_types::iteration_reclaim` admits now allocates
//! into an arena of its own for each iteration:
//!
//! - at the body's entry, `lotus_arena_iteration_enter(slot, outer)` frees
//!   the previous iteration's arena and makes a fresh one, owned by the
//!   arena the loop runs in (`outer`), and stores it in the loop's slot (an
//!   entry-block alloca, NULL before the first iteration);
//! - `current_arena_ptr` answers the slot's arena while the body is
//!   lowered, so the body's literals and the values its calls return land
//!   there (a call publishes it as the caller arena);
//! - where the loop is left (its exit block, which every `break` and the
//!   header's false edge reach), `lotus_arena_iteration_leave` frees the
//!   last iteration's arena (`lower_loop_stmt`);
//! - a `return`, a `fail` or a violation from inside the body leaves by
//!   the frame's epilogue, whose scratch destroy frees what the scratch
//!   owns, the open iteration arenas included. A `return`'s value is
//!   lowered outside the iteration arena (`iter_arena_suspended`), into
//!   the scratch the epilogue copies it out of.
//!
//! A body whose allocations never reach `current_arena_ptr` keeps no
//! trace: its enter call and store are erased once the body is lowered,
//! so a numeric loop pays nothing.

use inkwell::values::{FunctionValue, InstructionValue, PointerValue};
use inkwell::AddressSpace;

use hale_syntax::ast::{Block, Expr};
use hale_types::iteration_reclaim::{reclaims_per_iteration, OuterLocal};

use crate::codegen::{BlockEnd, CodegenError, CodegenTy, Cx, Scope};

/// One loop body lowered with an iteration arena of its own.
#[derive(Debug, Clone)]
pub(crate) struct IterArena<'ctx> {
    /// The fn the body is lowered in: an allocation lowered in any other
    /// (a helper synthesized mid-body) does not use the slot.
    func: FunctionValue<'ctx>,
    /// The entry-block slot holding the current iteration's arena.
    slot: PointerValue<'ctx>,
    /// How many allocations of the body the slot answered.
    uses: u32,
}

/// What a binding of type `ty`, declared outside a loop body, holds.
fn outer_local(ty: &CodegenTy) -> OuterLocal {
    match ty {
        CodegenTy::Int
        | CodegenTy::Float
        | CodegenTy::Bool
        | CodegenTy::Duration
        | CodegenTy::Decimal(_)
        | CodegenTy::Time => OuterLocal::Scalar,
        CodegenTy::LocusRef(_) | CodegenTy::Interface(_) | CodegenTy::Perspective(_) => OuterLocal::Locus,
        CodegenTy::Bounded(..) | CodegenTy::BytesMut => OuterLocal::Buffer,
        _ => OuterLocal::Heap,
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The current iteration arena, when an allocation lowered here
    /// belongs to the innermost per-iteration loop body: loaded from its
    /// slot. `None` outside such a body, while a `return`'s value is
    /// lowered, in another fn, or in the entry block (where the slot is
    /// still NULL).
    pub(crate) fn iteration_arena_ptr(&mut self) -> Result<Option<PointerValue<'ctx>>, CodegenError> {
        if self.iter_arena_suspended > 0 {
            return Ok(None);
        }
        let Some(top) = self.iter_arenas.last() else {
            return Ok(None);
        };
        let func = top.func;
        let slot = top.slot;
        if self.current_fn != Some(func) {
            return Ok(None);
        }
        let Some(bb) = self.builder.get_insert_block() else {
            return Ok(None);
        };
        if bb.get_parent() != Some(func) || func.get_first_basic_block() == Some(bb) {
            return Ok(None);
        }
        if let Some(top) = self.iter_arenas.last_mut() {
            top.uses += 1;
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arena = self
            .builder
            .build_load(ptr_t, slot, "iter.arena.cur")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        Ok(Some(arena))
    }

    /// Does `body`, about to be lowered at the builder's position, get an
    /// iteration arena? Only in a frame with a per-call scratch (whose
    /// self-field stores copy into the locus, and whose epilogue frees
    /// what the scratch owns), outside an arena override, and when
    /// `hale_types::iteration_reclaim` admits the body over the bindings
    /// `scope` holds at the loop.
    fn loop_body_reclaims(&self, body: &Block, scope: &Scope<'ctx>) -> bool {
        if self.current_method_scratch.is_none()
            || self.current_self.is_none()
            || self.current_arena_override.is_some()
            || self.current_fn.is_none()
        {
            return false;
        }
        let outer = |name: &str| scope.locals.get(name).map(|(_, ty)| outer_local(ty));
        let table = self.owner_table;
        let produces_locus = |e: &Expr| table.entry_of(e).is_some();
        let decl_of = hale_types::iteration_reclaim::decl_of(self.uses);
        reclaims_per_iteration(body, &outer, &produces_locus, &decl_of)
    }

    /// Lower a loop's body, the builder at the body's entry: with an
    /// iteration arena of its own when [`Self::loop_body_reclaims`] says
    /// so, its slot then queued for the loop's exit to free
    /// (`iter_arena_leaves`).
    pub(crate) fn lower_loop_body(&mut self, body: &Block, scope: &mut Scope<'ctx>) -> Result<BlockEnd, CodegenError> {
        if !self.loop_body_reclaims(body, scope) {
            return self.lower_block(body, scope);
        }
        let func = self.current_fn.expect("checked by loop_body_reclaims");
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let slot = self.deferred_dissolve_slot_alloca("iter.arena")?;
        let outer_uses = self.iter_arenas.last().map(|c| c.uses);
        let outer = self.current_arena_ptr()?;
        let cur = self.builder.build_load(ptr_t, slot, "iter.arena.prev").map_err(e)?;
        let enter = self.module.get_function("lotus_arena_iteration_enter").expect("lotus_arena_iteration_enter declared");
        let call = self.builder.build_call(enter, &[cur.into(), outer.into()], "iter.arena.enter").map_err(e)?;
        let arena = call.try_as_basic_value().left().expect("iteration_enter returns ptr");
        let store = self.builder.build_store(slot, arena).map_err(e)?;
        let call_instr: InstructionValue<'ctx> = arena
            .into_pointer_value()
            .as_instruction()
            .expect("a call's result is its instruction");
        self.iter_arenas.push(IterArena { func, slot, uses: 0 });
        let end = self.lower_block(body, scope);
        let ctx = self.iter_arenas.pop().expect("pushed above");
        let end = end?;
        if ctx.uses == 0 {
            // Nothing the body made reached the arena: no trace of it.
            store.erase_from_basic_block();
            call_instr.erase_from_basic_block();
            if let (Some(n), Some(top)) = (outer_uses, self.iter_arenas.last_mut()) {
                top.uses = n;
            }
        } else {
            self.iter_arena_leaves.push(slot);
        }
        Ok(end)
    }

    /// Lower a loop statement (`lower`), then free, at the loop's exit
    /// where the builder stands, the last iteration's arena of every
    /// body the loop lowered with one.
    pub(crate) fn lower_loop_stmt(
        &mut self,
        lower: impl FnOnce(&mut Self) -> Result<BlockEnd, CodegenError>,
    ) -> Result<BlockEnd, CodegenError> {
        let mark = self.iter_arena_leaves.len();
        let end = lower(self);
        let slots = self.iter_arena_leaves.split_off(mark);
        let end = end?;
        if end == BlockEnd::Open && !slots.is_empty() {
            let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let leave = self.module.get_function("lotus_arena_iteration_leave").expect("lotus_arena_iteration_leave declared");
            for slot in slots.into_iter().rev() {
                let cur = self.builder.build_load(ptr_t, slot, "iter.arena.last").map_err(e)?;
                self.builder.build_call(leave, &[cur.into()], "iter.arena.leave").map_err(e)?;
                self.builder.build_store(slot, ptr_t.const_null()).map_err(e)?;
            }
        }
        Ok(end)
    }

    /// Lower a value that leaves the frame (`return`, `fail`) outside
    /// every iteration arena.
    pub(crate) fn outside_iteration_arenas<T>(&mut self, lower: impl FnOnce(&mut Self) -> T) -> T {
        self.iter_arena_suspended += 1;
        let out = lower(self);
        self.iter_arena_suspended -= 1;
        out
    }
}
