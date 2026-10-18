//! GH #1530 seam fusion: a hop's set result handed back in the seam's own
//! frame.
//!
//! A seam that splits the set a hop returns (`let __seam_sN = hop(x);
//! match __seam_sN { .. }`, the binding named so by the expander only
//! when the `match` is its one use) calls the hop's **fused clone**: the
//! same method, lowered a second time with one more parameter, storage
//! for the set in the caller's frame. Each `return` of the clone writes
//! that storage and returns it: a member's value is deep-copied into the
//! caller's arena as the member it is, beside its tag as a constant,
//! where the method itself builds the set in its scratch and deep-copies
//! the whole set out behind runtime calls. The seam's dispatch then reads
//! the tag from its own frame, and once the clone is inlined into the
//! seam (every hop call is, `Cx::inline_new_hop_calls`) the tag is a
//! constant on each return path: optimization sends each return site
//! straight to its arm, with no tag read back and no switch.
//!
//! A program without a seam declares no clone, and its IR is what it was.

use hale_syntax::ast::{Expr, LocusDecl, LocusMember};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{BlockEnd, CodegenError, CodegenTy, Cx, EnumInfo, Scope};
use crate::locus::dissolve::LocusDissolve;

fn emit<T>(r: Result<T, inkwell::builder::BuilderError>) -> Result<T, CodegenError> {
    r.map_err(|e| CodegenError::LlvmEmit(e.to_string()))
}

/// Whether a `let` binding is one the expander splits at once.
pub(crate) fn is_fused_binding(name: &str, at: u32) -> bool {
    name.starts_with("__seam_s") && hale_types::seam_expand::is_seam_pos(at)
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The set `ret` names when a seam's fused binding may take it.
    fn fusable_set(&self, ret: Option<&CodegenTy>) -> Option<EnumInfo> {
        let Some(CodegenTy::Enum(set)) = ret else { return None };
        self.user_enums.get(set).filter(|i| i.set && i.has_payload).cloned()
    }

    /// At a locus method call returning `ret`: when the seam binding being
    /// lowered takes the set (`fused_call`), the method's fused clone and
    /// the frame storage to pass it as its last argument.
    pub(crate) fn take_fused_call(
        &mut self,
        locus: &str,
        method: &str,
        func: FunctionValue<'ctx>,
        ret: Option<&CodegenTy>,
    ) -> Result<Option<(FunctionValue<'ctx>, PointerValue<'ctx>)>, CodegenError> {
        if !self.fused_call {
            return Ok(None);
        }
        let Some(info) = self.fusable_set(ret) else { return Ok(None) };
        self.fused_call = false;
        let key = (locus.to_string(), method.to_string());
        let clone = match self.fused_clones.get(&key) {
            Some(f) => *f,
            None => {
                let ty = func.get_type();
                let mut params: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
                    ty.get_param_types().into_iter().map(Into::into).collect();
                params.push(self.context.ptr_type(AddressSpace::default()).into());
                let fn_ty = match ty.get_return_type() {
                    Some(r) => r.fn_type(&params, false),
                    None => self.context.void_type().fn_type(&params, false),
                };
                let f = self.module.add_function(&format!("{locus}.{method}.fused"), fn_ty, None);
                self.fused_clones.insert(key.clone(), f);
                self.fused_pending.push(key);
                f
            }
        };
        let storage = self.enum_storage_struct(&info);
        let dest = self.alloca_in_entry(storage.as_basic_type_enum(), "seam.fused.set")?;
        Ok(Some((clone, dest)))
    }

    /// Write the variant `tag` of `info`, its payload `vals`, at `dest`.
    pub(crate) fn fill_enum_variant_at(
        &mut self,
        info: &EnumInfo,
        tag: u32,
        vals: &[(BasicValueEnum<'ctx>, CodegenTy)],
        dest: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let storage = self.enum_storage_struct(info);
        let tag_ptr = emit(self.builder.build_struct_gep(storage, dest, 0, "enum.tag.ptr"))?;
        emit(self.builder.build_store(tag_ptr, self.context.i32_type().const_int(tag as u64, false)))?;
        if vals.is_empty() {
            return Ok(());
        }
        let body = emit(self.builder.build_struct_gep(storage, dest, 1, "enum.body.ptr"))?;
        let mut offset = 0u64;
        for (v, ty) in vals {
            let at = self.context.i64_type().const_int(offset, false);
            let slot =
                emit(unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), body, &[at], "enum.field.ptr") })?;
            emit(self.builder.build_store(slot, *v))?;
            offset += crate::codegen::codegen_ty_size_bytes(self.context, ty);
        }
        Ok(())
    }

    /// `return e;` in a fused clone whose declared result is the set
    /// `set`: the caller's storage `dest` filled and returned.
    pub(crate) fn lower_fused_return(
        &mut self,
        e: &Expr,
        scope: &Scope<'ctx>,
        set: &str,
        dest: PointerValue<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let info = self
            .user_enums
            .get(set)
            .cloned()
            .ok_or_else(|| CodegenError::UnsupportedAt(format!("the set `{set}` has no declaration"), e.span()))?;
        let in_fallible = self.current_user_fn_exit_bb.is_some();
        let scratch = self.current_method_scratch.is_some();
        // the member the checker flowed into the set here, if one
        let member = self.injection_at(e).filter(|i| i.set == set).map(|i| i.member);
        // as the ordinary return does: a fresh value is built in the
        // caller's arena, and the deep copy below passes it through
        let saved = self.current_arena_override;
        if scratch {
            let slot = self.current_method_caller_arena.expect("method scratch implies a caller-arena snapshot");
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let caller = emit(self.builder.build_load(ptr_t, slot, "method.caller_arena.for_return"))?;
            self.current_arena_override = Some(caller.into_pointer_value());
        }
        let lowered = match &member {
            Some(_) => {
                self.suppress_inject = Some(e.span());
                let r = self.lower_expr(e, scope);
                self.suppress_inject = None;
                r
            }
            None => self.lower_expr(e, scope),
        };
        self.current_arena_override = saved;
        let (v, ty) = lowered?;
        match member {
            Some(m) => {
                let tag = info.variants.iter().position(|x| x.name == m).ok_or_else(|| {
                    CodegenError::UnsupportedAt(format!("the set `{set}` has no member `{m}`"), e.span())
                })? as u32;
                let copied = if scratch { self.emit_method_return_deep_copy(v, &ty)? } else { v };
                self.fill_enum_variant_at(&info, tag, &[(copied, ty)], dest)?;
            }
            None => {
                if ty != CodegenTy::Enum(set.to_string()) {
                    return Err(CodegenError::UnsupportedAt(
                        format!("a fused return of the set `{set}` got {ty:?}"),
                        e.span(),
                    ));
                }
                // a value of the set as a whole: copied out as the set,
                // then into the caller's storage
                let copied = if scratch { self.emit_method_return_deep_copy(v, &ty)? } else { v };
                let storage = self.enum_storage_struct(&info);
                let whole = emit(self.builder.build_load(storage, copied.into_pointer_value(), "fused.set"))?;
                emit(self.builder.build_store(dest, whole))?;
            }
        }
        if in_fallible {
            // the exit the method's own return takes (its epilogue's ok
            // branch and cleanup), here at the return site: a return
            // goes on to its caller's arm without meeting the other
            // returns, and the failure path keeps the shared exit
            let out_val = self
                .current_user_fn_fallible
                .as_ref()
                .and_then(|f| f.out_val_param)
                .expect("a fallible clone returning a set has a value slot");
            emit(self.builder.build_store(out_val, dest))?;
            self.emit_method_exit_epilogue_draining()?;
            emit(self.builder.build_return(Some(&self.context.bool_type().const_int(0, false))))?;
        } else {
            if scratch {
                self.emit_method_exit_epilogue()?;
            }
            emit(self.builder.build_return(Some(&dest)))?;
        }
        Ok(BlockEnd::Terminated)
    }

    /// `emit_method_exit_epilogue` with the queue drain a fallible
    /// method's exit runs (its cleanup flushes with `flush_dissolve_frame`).
    fn emit_method_exit_epilogue_draining(&mut self) -> Result<(), CodegenError> {
        let saved_frame = self.deferred_dissolves.last().cloned().unwrap_or_default();
        self.flush_dissolve_frame()?;
        self.push_dissolve_frame();
        if let Some(f) = self.deferred_dissolves.last_mut() {
            *f = saved_frame;
        }
        let (scratch, caller) = (self.current_method_scratch, self.current_method_caller_arena);
        self.close_method_scratch()?;
        self.current_method_scratch = scratch;
        self.current_method_caller_arena = caller;
        Ok(())
    }

    /// The fused clones the seams asked for, their bodies lowered as the
    /// methods' own with `fused_dest` their last parameter.
    pub(crate) fn emit_fused_clones(&mut self, loci: &[LocusDecl]) -> Result<(), CodegenError> {
        while let Some((locus, method)) = self.fused_pending.pop() {
            let Some(l) = loci.iter().find(|l| l.name.name == locus) else { continue };
            let Some((member_idx, fd)) = l.members.iter().enumerate().find_map(|(i, m)| match m {
                LocusMember::Fn(fd) if fd.name.name == method => Some((i, fd)),
                _ => None,
            }) else {
                continue;
            };
            let clone = self.fused_clones[&(locus.clone(), method.clone())];
            let dest = clone
                .get_last_param()
                .expect("a fused clone takes the set's storage")
                .into_pointer_value();
            let info = self.user_loci.get(&locus).cloned().expect("locus declared in pass A");
            let saved = self.fused_dest.replace(dest);
            let r = self.in_body(l.id, |cx| {
                if fd.fallible.is_some() {
                    cx.lower_fallible_locus_method_body_into(l, &info, fd, clone)
                } else {
                    cx.lower_user_fn_member_body(l, &info, member_idx, fd, clone)
                }
            });
            self.fused_dest = saved;
            r?;
        }
        Ok(())
    }
}
