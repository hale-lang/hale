//! bounded[T; N] (2026-07-02): the intrinsic vocabulary over the
//! inline `{ i64 len, [N x T] }` storage. Free-fn-shaped grammar
//! intrinsics (like `len(s)`) so the types-have-no-methods axiom
//! holds:
//!
//!   push(f, x)  -> () fallible(CapacityError)   // full = error
//!   at(f, i)    -> T  fallible(IndexError)
//!   count(f)    -> Int
//!   clear(f)                                     // len = 0
//!
//! plus `for x in f` iteration (lower_for_bounded in codegen.rs).
//! The receiver is any bounded-typed lvalue (a `type` field or a
//! locus params field); mutation happens in place — the storage is
//! inline in the containing struct, so there is nothing to allocate
//! or anchor.

use inkwell::values::BasicValueEnum;
use inkwell::values::PointerValue;

use hale_syntax::ast::Expr;

use crate::codegen::CodegenError;
use crate::codegen::CodegenTy;
use crate::codegen::Cx;
use crate::codegen::FallibleCallResult;
use crate::codegen::Scope;

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Static (no-IR) type resolution for intrinsic dispatch —
    /// enough of the expression grammar to type a bounded
    /// receiver: locals, `self`, and field chains through
    /// TypeRef / LocusRef.
    pub(crate) fn static_expr_codegen_ty(
        &self,
        e: &Expr,
        scope: &Scope<'ctx>,
    ) -> Option<CodegenTy> {
        match e {
            Expr::Ident(id) => {
                scope.locals.get(&id.name).map(|(_, t)| t.clone())
            }
            Expr::KwSelf(_) => self
                .current_self
                .as_ref()
                .map(|cs| CodegenTy::LocusRef(cs.locus_name.clone())),
            Expr::Field { receiver, name, .. } => {
                match self.static_expr_codegen_ty(receiver, scope)? {
                    CodegenTy::TypeRef(n) => self
                        .user_types
                        .get(&n)
                        .and_then(|ti| ti.fields.get(&name.name))
                        .map(|(_, t)| t.clone()),
                    CodegenTy::LocusRef(n) => self
                        .user_loci
                        .get(&n)
                        .and_then(|li| li.fields.get(&name.name))
                        .map(|(_, t)| t.clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Elem + cap when args[0] statically types as bounded[T; N].
    pub(crate) fn bounded_recv_spec(
        &self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Option<(CodegenTy, u64)> {
        match self.static_expr_codegen_ty(args.first()?, scope)? {
            CodegenTy::Bounded(elem, cap) => Some((*elem, cap)),
            _ => None,
        }
    }

    /// Lower args[0] to the storage pointer (SSA of a bounded value
    /// IS the storage address) and return (len_ptr, data_base_ptr).
    fn bounded_storage_ptrs(
        &mut self,
        recv: &Expr,
        elem: &CodegenTy,
        cap: u64,
        scope: &Scope<'ctx>,
    ) -> Result<
        (PointerValue<'ctx>, PointerValue<'ctx>, PointerValue<'ctx>),
        CodegenError,
    > {
        let (v, ty) = self.lower_expr(recv, scope)?;
        if !matches!(ty, CodegenTy::Bounded(_, _)) {
            return Err(CodegenError::Unsupported(format!(
                "bounded intrinsic receiver lowered to {:?}, expected \
                 bounded[T; N]",
                ty
            )));
        }
        let storage = v.into_pointer_value();
        let st = self.llvm_bounded_storage_type(elem, cap);
        let len_ptr = self
            .builder
            .build_struct_gep(st, storage, 0, "bounded.len.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let data_ptr = self
            .builder
            .build_struct_gep(st, storage, 1, "bounded.data.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((storage, len_ptr, data_ptr))
    }

    /// count(f) -> Int
    pub(crate) fn lower_bounded_count(
        &mut self,
        args: &[Expr],
        elem: &CodegenTy,
        cap: u64,
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let (_, len_ptr, _) =
            self.bounded_storage_ptrs(&args[0], elem, cap, scope)?;
        let len = self
            .builder
            .build_load(self.context.i64_type(), len_ptr, "bounded.count")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((len, CodegenTy::Int))
    }

    /// clear(f) — len = 0. Returns Int 0 (statement-idiom value,
    /// same convention as the Unit-ish stdlib calls).
    pub(crate) fn lower_bounded_clear(
        &mut self,
        args: &[Expr],
        elem: &CodegenTy,
        cap: u64,
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let (_, len_ptr, _) =
            self.bounded_storage_ptrs(&args[0], elem, cap, scope)?;
        let zero = self.context.i64_type().const_int(0, false);
        self.builder
            .build_store(len_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((zero.into(), CodegenTy::Int))
    }

    /// push(f, x) / at(f, i) — the fallible pair, dispatched from
    /// `lower_fallible_call`'s Ident arm. Returns None when args[0]
    /// isn't bounded (the caller falls through to user fns).
    pub(crate) fn try_lower_bounded_fallible_intrinsic(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<FallibleCallResult<'ctx>>, CodegenError> {
        if !matches!(name, "push" | "at") || args.len() != 2 {
            return Ok(None);
        }
        let Some((elem, cap)) = self.bounded_recv_spec(args, scope)
        else {
            return Ok(None);
        };
        let (_, len_ptr, data_ptr) =
            self.bounded_storage_ptrs(&args[0], &elem, cap, scope)?;
        let i64_t = self.context.i64_type();
        let len = self
            .builder
            .build_load(i64_t, len_ptr, "bounded.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let elem_llvm = self.llvm_basic_type(&elem);
        let func = self.current_fn.expect("bounded intrinsic in fn body");

        match name {
            "push" => {
                let (xv, xty) = self.lower_expr(&args[1], scope)?;
                // F.23 Int → Float widening, same as field-init.
                let xv = if elem == CodegenTy::Float
                    && xty == CodegenTy::Int
                {
                    self.coerce_to_float(xv, &xty, "push element")?.into()
                } else {
                    xv
                };
                let is_full = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SGE,
                        len,
                        i64_t.const_int(cap, false),
                        "bounded.push.full",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let do_bb =
                    self.context.append_basic_block(func, "bounded.push.do");
                let full_bb = self
                    .context
                    .append_basic_block(func, "bounded.push.fullpath");
                let join_bb = self
                    .context
                    .append_basic_block(func, "bounded.push.join");
                let out_err_slot = self.alloca_for(
                    &CodegenTy::TypeRef("CapacityError".into()),
                    "bounded.push.err.slot",
                )?;
                self.builder
                    .build_conditional_branch(is_full, full_bb, do_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(do_bb);
                let slot_ptr = unsafe {
                    self.builder
                        .build_gep(
                            elem_llvm,
                            data_ptr,
                            &[len],
                            "bounded.push.slot",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                self.builder
                    .build_store(slot_ptr, xv)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let next = self
                    .builder
                    .build_int_add(
                        len,
                        i64_t.const_int(1, false),
                        "bounded.push.next",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(len_ptr, next)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(join_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(full_bb);
                let err_ptr =
                    self.emit_capacity_error_alloc(cap, len)?;
                self.builder
                    .build_store(out_err_slot, err_ptr)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(join_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(join_bb);
                Ok(Some(FallibleCallResult {
                    i1_path: is_full,
                    out_val_slot: None,
                    out_err_slot,
                    success_ty: None,
                    payload_ty: CodegenTy::TypeRef(
                        "CapacityError".into(),
                    ),
                }))
            }
            "at" => {
                let (iv, ity) = self.lower_expr(&args[1], scope)?;
                if ity != CodegenTy::Int {
                    return Err(CodegenError::Unsupported(format!(
                        "at(f, i): index must be Int, got {:?}",
                        ity
                    )));
                }
                let idx = iv.into_int_value();
                let neg = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLT,
                        idx,
                        i64_t.const_int(0, false),
                        "bounded.at.neg",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let past = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SGE,
                        idx,
                        len,
                        "bounded.at.past",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let oob = self
                    .builder
                    .build_or(neg, past, "bounded.at.oob")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let ok_bb =
                    self.context.append_basic_block(func, "bounded.at.ok");
                let err_bb =
                    self.context.append_basic_block(func, "bounded.at.err");
                let join_bb =
                    self.context.append_basic_block(func, "bounded.at.join");
                let out_val_slot =
                    self.alloca_for(&elem, "bounded.at.val.slot")?;
                let out_err_slot = self.alloca_for(
                    &CodegenTy::TypeRef("IndexError".into()),
                    "bounded.at.err.slot",
                )?;
                self.builder
                    .build_conditional_branch(oob, err_bb, ok_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(ok_bb);
                let slot_ptr = unsafe {
                    self.builder
                        .build_gep(
                            elem_llvm,
                            data_ptr,
                            &[idx],
                            "bounded.at.slot",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                let v = self
                    .builder
                    .build_load(elem_llvm, slot_ptr, "bounded.at.val")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(out_val_slot, v)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(join_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(err_bb);
                let err_ptr = self.emit_index_error_alloc(
                    "out_of_bounds",
                    idx,
                    len,
                )?;
                self.builder
                    .build_store(out_err_slot, err_ptr)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(join_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(join_bb);
                Ok(Some(FallibleCallResult {
                    i1_path: oob,
                    out_val_slot: Some(out_val_slot),
                    out_err_slot,
                    success_ty: Some(elem),
                    payload_ty: CodegenTy::TypeRef("IndexError".into()),
                }))
            }
            _ => unreachable!("gated above"),
        }
    }

    /// Allocate + populate a `CapacityError { cap: Int, count: Int }`
    /// in the current arena. Mirror of `emit_index_error_alloc`.
    pub(crate) fn emit_capacity_error_alloc(
        &mut self,
        cap: u64,
        count: inkwell::values::IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let info = self
            .user_types
            .get("CapacityError")
            .cloned()
            .expect("CapacityError declared at startup");
        let size = info
            .struct_ty
            .size_of()
            .expect("CapacityError has known size");
        let alloc_ptr = self.arena_alloc(size, "CapacityError.alloc")?;
        let i64_t = self.context.i64_type();
        for (fname, v) in [
            ("cap", i64_t.const_int(cap, false)),
            ("count", count),
        ] {
            let (idx, _) = info
                .fields
                .get(fname)
                .cloned()
                .expect("CapacityError field");
            let p = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    alloc_ptr,
                    idx,
                    &format!("CapacityError.{}.ptr", fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(p, v)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        Ok(alloc_ptr)
    }

    /// Register the built-in `CapacityError` record (mirror of
    /// `declare_builtin_index_error_type`). Fields: `cap: Int`,
    /// `count: Int`.
    pub(crate) fn declare_builtin_capacity_error_type(&mut self) {
        if self.user_types.contains_key("CapacityError") {
            return;
        }
        let i64_t = self.context.i64_type();
        let mut fields: std::collections::BTreeMap<
            String,
            (u32, CodegenTy),
        > = std::collections::BTreeMap::new();
        fields.insert("cap".into(), (0, CodegenTy::Int));
        fields.insert("count".into(), (1, CodegenTy::Int));
        let field_order = vec!["cap".to_string(), "count".to_string()];
        let struct_ty =
            self.context.opaque_struct_type("type.CapacityError");
        struct_ty
            .set_body(&[i64_t.into(), i64_t.into()], false);
        self.user_types.insert(
            "CapacityError".to_string(),
            crate::codegen::TypeInfo {
                struct_ty,
                fields,
                field_order,
                defaults: std::collections::BTreeMap::new(),
            },
        );
    }
}
