//! Lowering's half of the owned-view analysis
//! (`hale_types::view_bindings`): the owned copy a flagged binding takes.

use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx};

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
}
