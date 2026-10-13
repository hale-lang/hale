//! Lowering's half of the owned-view analysis
//! (`hale_types::view_bindings`): the owned copy a flagged binding takes,
//! and the copy on write a write takes first — into a nested part of a
//! value read from a form's cell, or into a local bound from a place.

use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// An owned copy, in the frame's arena, of `val`, a value a binding
    /// would otherwise view in its place (`OwnedViews`): every String
    /// and Bytes copied even from the frame's own arena (a `run()` body
    /// allocates from the locus's), every record, tuple, array and
    /// payload-enum block fresh (`emit_owned_deep_copy`). Any other
    /// value is its own.
    pub(crate) fn own_viewed_value(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match ty {
            CodegenTy::String
            | CodegenTy::Bytes
            | CodegenTy::TypeRef(_)
            | CodegenTy::Tuple(_)
            | CodegenTy::Array(_, _)
            | CodegenTy::Enum(_) => {
                let dest = self.current_arena_ptr()?;
                self.emit_owned_deep_copy(val, ty, dest)
            }
            _ => Ok(val),
        }
    }

    /// The flag that says whether the local held in `slot` owns its part
    /// `field` (or the cell it was read from still does), made in the
    /// entry block the first time it is asked for.
    fn cell_part_flag(&mut self, slot: PointerValue<'ctx>, field: &str) -> Result<PointerValue<'ctx>, CodegenError> {
        let key = (slot, field.to_string());
        if let Some(flag) = self.cell_part_flags.get(&key) {
            return Ok(*flag);
        }
        let flag = self.alloca_in_entry(self.context.bool_type().into(), &format!("{field}.owned"))?;
        self.cell_part_flags.insert(key, flag);
        Ok(flag)
    }

    /// At a binding of a local whose parts a later write copies on write
    /// (`OwnedViews::cow_bind`): set each part's flag, `true` when the
    /// value bound is the frame's own (a fresh value or a copy), `false`
    /// when it is a read of a form's cell, whose nested blocks are the
    /// cell's, or a place's value another name reaches. `copied`: this
    /// binding bound a deep copy (a struct `let` of a place, GH #713, or
    /// an owned view), which owns every part whatever the analysis saw.
    pub(crate) fn set_cell_part_flags(
        &mut self,
        slot: PointerValue<'ctx>,
        fields: &[(String, bool)],
        copied: bool,
    ) -> Result<(), CodegenError> {
        for (field, owned) in fields {
            let flag = self.cell_part_flag(slot, field)?;
            let v = self.context.bool_type().const_int(u64::from(*owned || copied), false);
            self.builder.build_store(flag, v).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        Ok(())
    }

    /// Before a write into the local `name`, bound from a place
    /// (`OwnedViews::cow_write`'s whole-value part, `""`): unless its flag
    /// says the frame owns the value already, deep-copy it into the
    /// frame's arena and point the local at the copy, so the write never
    /// reaches the place it was read from (spec/types.md § A local bound
    /// from a place is yours). A loop variable of a walk that visits in
    /// place by design is left as it is.
    pub(crate) fn emit_local_cow(&mut self, name: &str, scope: &Scope<'ctx>) -> Result<(), CodegenError> {
        let Some((slot, ty)) = scope.locals.get(name).cloned() else { return Ok(()) };
        if self.cow_exempt_slots.contains(&slot)
            || !matches!(ty, CodegenTy::TypeRef(_) | CodegenTy::Tuple(_) | CodegenTy::Array(_, _) | CodegenTy::Enum(_))
        {
            return Ok(());
        }
        let func = self.current_fn.ok_or_else(|| CodegenError::Unsupported("a write outside a fn body".into()))?;
        let flag = self.cell_part_flags.get(&(slot, String::new())).copied();
        let copy_bb = self.context.append_basic_block(func, &format!("{name}.cow"));
        let join_bb = self.context.append_basic_block(func, &format!("{name}.cow.done"));
        let err = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        match flag {
            Some(flag) => {
                let owned = self
                    .builder
                    .build_load(self.context.bool_type(), flag, &format!("{name}.owned"))
                    .map_err(err)?
                    .into_int_value();
                self.builder.build_conditional_branch(owned, join_bb, copy_bb).map_err(err)?;
            }
            // No binding set a flag: copy at every write, which is never
            // wrong.
            None => {
                self.builder.build_unconditional_branch(copy_bb).map_err(err)?;
            }
        }
        self.builder.position_at_end(copy_bb);
        let old = self.builder.build_load(self.llvm_basic_type(&ty), slot, name).map_err(err)?;
        let copy = self.own_viewed_value(old, &ty)?;
        self.builder.build_store(slot, copy).map_err(err)?;
        if let Some(flag) = flag {
            self.builder.build_store(flag, self.context.bool_type().const_int(1, false)).map_err(err)?;
        }
        self.builder.build_unconditional_branch(join_bb).map_err(err)?;
        self.builder.position_at_end(join_bb);
        Ok(())
    }

    /// Before a write into the part `field` of the local `name`, a value
    /// read from a form's cell (`OwnedViews::cow_write`): unless its flag
    /// says the frame owns the part already, copy the part (and all it
    /// holds) into the frame's arena and point the local at the copy, so
    /// the write never reaches the cell. A part stored inline in the
    /// record (a scalar, a fixed array of scalars) is the local's own
    /// already and copies nothing.
    pub(crate) fn emit_cell_part_cow(
        &mut self,
        name: &str,
        field: &str,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let Some((slot, CodegenTy::TypeRef(tn))) = scope.locals.get(name).cloned() else { return Ok(()) };
        let Some(info) = self.user_types.get(&tn).cloned() else { return Ok(()) };
        let Some((idx, fty)) = info.fields.get(field).cloned() else { return Ok(()) };
        if !self.record_field_owns_block(&fty) {
            return Ok(());
        }
        let func = self.current_fn.ok_or_else(|| CodegenError::Unsupported("a write outside a fn body".into()))?;
        let flag = self.cell_part_flags.get(&(slot, field.to_string())).copied();
        let copy_bb = self.context.append_basic_block(func, &format!("{name}.{field}.cow"));
        let join_bb = self.context.append_basic_block(func, &format!("{name}.{field}.cow.done"));
        match flag {
            Some(flag) => {
                let owned = self
                    .builder
                    .build_load(self.context.bool_type(), flag, &format!("{name}.{field}.owned"))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                self.builder
                    .build_conditional_branch(owned, join_bb, copy_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            // No binding set a flag (none the analysis named): copy at
            // every write, which is never wrong.
            None => {
                self.builder.build_unconditional_branch(copy_bb).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }
        self.builder.position_at_end(copy_bb);
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let rec = self
            .builder
            .build_load(ptr_t, slot, name)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let field_ptr = self
            .builder
            .build_struct_gep(info.struct_ty, rec, idx, &format!("{name}.{field}.ptr"))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let old = self
            .builder
            .build_load(self.llvm_basic_type(&fty), field_ptr, &format!("{name}.{field}"))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let copy = self.own_viewed_value(old, &fty)?;
        self.builder.build_store(field_ptr, copy).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        if let Some(flag) = flag {
            self.builder
                .build_store(flag, self.context.bool_type().const_int(1, false))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder.build_unconditional_branch(join_bb).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder.position_at_end(join_bb);
        Ok(())
    }
}
