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
//! - a free fn's iteration arenas (GH #2094) are owned by its per-call
//!   subregion, which its epilogue destroys after the copy-out, as a
//!   method's scratch; `fn main`'s by the program's arena, which lives
//!   until the process exits: a main loop left by anything but its exit
//!   (a `return`, a failed assertion, the drain, a signal) leaves its
//!   last iteration's arena to it.
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
        | CodegenTy::U256
        | CodegenTy::I256
        | CodegenTy::Time => OuterLocal::Scalar,
        CodegenTy::LocusRef(_) | CodegenTy::Interface(_) => OuterLocal::Locus,
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
    /// iteration arena? Only in a frame whose arena outlives the loop and
    /// frees what it owns when the frame is done with it
    /// ([`Self::frame_owns_iteration_arenas`]), outside an arena override,
    /// and when `hale_types::iteration_reclaim` admits the body over the
    /// bindings `scope` holds at the loop.
    fn loop_body_reclaims(&self, body: &Block, scope: &Scope<'ctx>) -> bool {
        if !self.frame_owns_iteration_arenas()
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

    /// Is the frame being lowered one whose loops may take iteration
    /// arenas: one whose arena is the iteration arenas' owner, outliving
    /// every loop in it and freeing the arenas still open when it goes.
    ///
    /// - A locus member with a per-call scratch: its epilogue destroys
    ///   the scratch, and with it the open iteration arenas, on every way
    ///   out (a `return`, a `fail`, a violation).
    /// - `fn main`'s own frame (GH #2094): its arena is the program's,
    ///   which lives until the process exits, so an iteration arena left
    ///   open by a `return`, a failed assertion, the drain or a signal
    ///   goes with the process, as everything main made does.
    /// - A free fn (GH #2094): the per-call subregion every allocating fn
    ///   opens at entry ([`Self::iteration_owner`]), which its epilogue
    ///   destroys after copying the returned value (or the failure) into
    ///   the caller's arena, as a method's scratch is.
    fn frame_owns_iteration_arenas(&self) -> bool {
        let method = self.current_method_scratch.is_some() && self.current_self.is_some();
        let main = self.in_main
            && self.current_self.is_none()
            && self.current_method_scratch.is_none()
            && self.current_user_fn_caller_arena.is_none();
        method || main || self.free_fn_frame()
    }

    /// Lowering a free fn's body: its caller arena and its own subregion
    /// are set, and no locus member's frame is.
    fn free_fn_frame(&self) -> bool {
        !self.in_main
            && self.current_self.is_none()
            && self.current_method_scratch.is_none()
            && self.current_user_fn_caller_arena.is_some()
            && self.current_user_fn_arena.is_some()
    }

    /// The arena a loop's iteration arenas are carved from and owned by:
    /// the enclosing loop's iteration arena when there is one, a free fn's
    /// own subregion (even where its body allocates into its caller's
    /// arena, so its epilogue frees what a `return` leaves open), else the
    /// arena the loop runs in (a method's scratch, the program's).
    fn iteration_owner(&mut self) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(enclosing) = self.iteration_arena_ptr()? {
            return Ok(enclosing);
        }
        match self.current_user_fn_arena.filter(|_| self.free_fn_frame()) {
            Some(slot) => {
                let ptr_t = self.context.ptr_type(AddressSpace::default());
                Ok(self
                    .builder
                    .build_load(ptr_t, slot, "iter.arena.owner")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value())
            }
            None => self.current_arena_ptr(),
        }
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
        let outer = self.iteration_owner()?;
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

    /// Build a fallible primitive's error payload (`build`): outside every
    /// iteration arena when the `or` handling it takes the error out of
    /// the frame (`or raise`, `or fail`: [`Cx::or_error_leaves_frame`]), as
    /// a `fail`'s value is, so a loop whose only allocation is the error
    /// it raises opens no iteration arena (`v.get(j) or raise`, whose
    /// error branch a proven bound leaves dead). An error an `or` handles
    /// in place stays in the iteration it is handled in.
    pub(crate) fn error_payload<T>(&mut self, build: impl FnOnce(&mut Self) -> T) -> T {
        if self.or_error_leaves_frame {
            self.outside_iteration_arenas(build)
        } else {
            build(self)
        }
    }

    /// Publish the caller arena for a call of a locus method declared
    /// with `params` returning `ret` (`Cx::emit_set_caller_arena_for`):
    /// the frame's, outside every iteration arena, when the method leaves
    /// nothing in it. A method allocates into its scratch, carved from its
    /// own locus's arena; its caller's arena takes only the heap value its
    /// epilogue copies out and what it anchors writing through a record
    /// parameter. So one returning nothing or a scalar, and taking only
    /// scalars, Strings, Bytes, views and locus references, lets a loop
    /// that just calls it (`h.a(); h.b();`) open no iteration arena.
    /// Handing the frame's arena is safe whatever the method does: it
    /// outlives the iteration.
    pub(crate) fn emit_set_caller_arena_for_method(
        &mut self,
        params: &[hale_syntax::ast::Param],
        ret: Option<&hale_syntax::ast::TypeExpr>,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        if self.method_leaves_caller_arena_alone(params, ret) {
            return self.outside_iteration_arenas(|cx| cx.emit_set_caller_arena_for(args, scope));
        }
        self.emit_set_caller_arena_for(args, scope)
    }

    fn method_leaves_caller_arena_alone(
        &self,
        params: &[hale_syntax::ast::Param],
        ret: Option<&hale_syntax::ast::TypeExpr>,
    ) -> bool {
        use hale_syntax::ast::TypeExpr;
        let ret_ok = match ret {
            None => true,
            Some(TypeExpr::Tuple(parts, _)) if parts.is_empty() => true,
            Some(t) => self.type_expr_to_codegen_ty(t).is_ok_and(|t| outer_local(&t) == OuterLocal::Scalar),
        };
        ret_ok
            && params.iter().all(|p| {
                self.type_expr_to_codegen_ty(&p.ty).is_ok_and(|t| {
                    matches!(
                        t,
                        CodegenTy::String | CodegenTy::Bytes | CodegenTy::StringView | CodegenTy::BytesView
                    ) || matches!(outer_local(&t), OuterLocal::Scalar | OuterLocal::Locus)
                })
            })
    }

    /// GH #2105: the arena the copy a whole store `target = value` makes
    /// into a local's `bounded` field goes in, when the store retires what
    /// it replaces (`OwnedViews::retires_field`: no other name in scope
    /// reaches the field's earlier copies) and copies (a fresh value is
    /// bound as it is): the field's own arena, made fresh here by
    /// `lotus_arena_iteration_enter`, which frees (or empties in place)
    /// the one the previous such store copied into. Only those stores
    /// allocate in it, so what it frees is exactly the elements they made;
    /// a block that reached the field any other way lies elsewhere and is
    /// never freed by it.
    ///
    /// The arena is owned by the frame's own arena (outside any iteration
    /// arena: a local declared in a loop's body keeps it from one
    /// iteration to the next, where the next store frees it) and its slot
    /// is an entry-block alloca of this function per local and field. A
    /// store the column does not name, or one with no declaration, keeps
    /// the current arena (`None`).
    pub(crate) fn field_copy_arena(
        &mut self,
        target: &hale_syntax::ast::LValue,
        assign: hale_syntax::ast::NodeId,
        value: &Expr,
    ) -> Result<Option<PointerValue<'ctx>>, CodegenError> {
        use hale_syntax::ast::LValueSeg;
        if Self::expr_is_fresh_value(value) || !self.owned_views.retires_field(assign) {
            return Ok(None);
        }
        let Some(func) = self.current_fn else { return Ok(None) };
        let Some((decl, _)) = hale_types::iteration_reclaim::decl_of(self.uses)(assign) else { return Ok(None) };
        let mut path = Vec::new();
        for seg in &target.tail {
            match seg {
                LValueSeg::Field(f) => path.push(f.name.clone()),
                LValueSeg::Index(_) => return Ok(None),
            }
        }
        let key = (func, decl, path.join("."));
        let slot = match self.field_arenas.get(&key) {
            Some(slot) => *slot,
            None => {
                let slot = self.deferred_dissolve_slot_alloca("field.arena")?;
                self.field_arenas.insert(key, slot);
                slot
            }
        };
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let owner = self.outside_iteration_arenas(|cx| cx.current_arena_ptr())?;
        let prev = self.builder.build_load(ptr_t, slot, "field.arena.prev").map_err(e)?;
        let enter = self.module.get_function("lotus_arena_iteration_enter").expect("lotus_arena_iteration_enter declared");
        let arena = self
            .builder
            .build_call(enter, &[prev.into(), owner.into()], "field.arena")
            .map_err(e)?
            .try_as_basic_value()
            .left()
            .expect("iteration_enter returns ptr")
            .into_pointer_value();
        self.builder.build_store(slot, arena).map_err(e)?;
        Ok(Some(arena))
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
