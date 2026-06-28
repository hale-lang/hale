//! Channels codegen: fallible(E) value-error protocol + structural-
//! channel routing (resolve_failure_route walks the parent chain for
//! the matching on_failure handler). Closure-assertion lowering
//! shipped in Round 4e at locus/closure.rs.
//!
//! Lifted as inherent `impl<'ctx, 'p> Cx<'ctx, 'p>` blocks — call
//! sites need no `use` import. Round 6 of the codegen model-org
//! refactor.

use hale_syntax::ast::{Expr, FnDecl, LocusDecl, OrDisposition, Stmt, TypeExpr};
use inkwell::types::BasicType;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue, PointerValue};
use inkwell::AddressSpace;

use crate::locus::dissolve::LocusDissolve;
use crate::codegen::{
    bce_receiver_key, view_coerces_to, BlockEnd, CodegenError, CodegenTy,
    Cx, FallibleCallResult, FallibleCtx, FnSig, LocusInfo, Scope, SelfCx,
};
use crate::stdlib::bytes::BytesStdlib;
use crate::stdlib::crypto::CryptoStdlib;
use crate::stdlib::io_file::IoFileStdlib;
use crate::stdlib::io_fs::IoFsStdlib;
use crate::stdlib::io_tcp::IoTcpStdlib;
use crate::stdlib::io_tls::IoTlsStdlib;
use crate::stdlib::io_udp::IoUdpStdlib;
use crate::stdlib::process::ProcessStdlib;
use crate::stdlib::str::StrStdlib;

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Resolve the (parent_self, on_failure_fn) pair for a child
    /// of `child_locus_name` whose closure may fail at dissolve.
    /// Reads `current_self` (set while we're in the parent's
    /// lifecycle body) and that parent's `failure_handler`. If
    /// the parent declares an on_failure that takes this child
    /// type, returns the parent's self_ptr + the handler fn ptr.
    /// Otherwise returns (null, null) — the closure-fail path
    /// will fall back to the v0 dprintf+exit report.
    pub(crate) fn resolve_failure_route(
        &self,
        child_locus_name: &str,
    ) -> (PointerValue<'ctx>, PointerValue<'ctx>) {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let null_ptr = ptr_t.const_null();
        // F.31 Phase 3b (2026-05-23): fall back to
        // `params_init_self` when `current_self` is None. This
        // covers placement-pinned children instantiated as
        // main-locus params defaults — they get instantiated
        // INSIDE `lower_locus_instantiation` (during the parent's
        // params-init loop), before any lifecycle method body
        // has been entered, so `current_self` isn't set. The
        // params-init loop sets `params_init_self` to the parent
        // being instantiated for exactly this lookup.
        let cs = match self.current_self.as_ref() {
            Some(cs) => cs,
            None => match self.params_init_self.as_ref() {
                Some(cs) => cs,
                None => return (null_ptr, null_ptr),
            },
        };
        let Some(parent_info) = self.user_loci.get(&cs.locus_name) else {
            return (null_ptr, null_ptr);
        };
        let Some((expected_child, handler_fn)) =
            parent_info.failure_handler.as_ref()
        else {
            return (null_ptr, null_ptr);
        };
        if expected_child != child_locus_name {
            return (null_ptr, null_ptr);
        }
        (
            cs.self_ptr,
            handler_fn.as_global_value().as_pointer_value(),
        )
    }

    /// Open-question #24 MVP (2026-05-25): lower a fallible
    /// locus member fn's body. Parallel to `lower_user_fn_body`'s
    /// fallible path but slimmed for the value-only scope-cut:
    /// no `__caller_arena` plumbing, no per-call subregion, no
    /// deep-copy in the exit epilogue. The runtime ABI is:
    ///
    ///     <Locus>.<method>(self_ptr,
    ///                      <user_params...>,
    ///                      [out_val: T* if T != Unit],
    ///                      out_err: E*) -> i1
    ///
    /// `Stmt::Return e;` stores `e` into a local `ret_alloca`,
    /// keeps the path indicator at 0, and br's to the unified
    /// exit. `Stmt::Fail e;` stores `e` into `err_alloca`, flips
    /// the path indicator to 1, and br's to exit. The exit
    /// epilogue branches on the path indicator and stores the
    /// matching local into the caller-provided sret slot.
    /// Value-only payload types (Int / Bool / Decimal / no-
    /// payload Enum / flat struct) survive the sret store
    /// without a deep-copy step. Heap-bearing payloads were
    /// rejected at declare-time by `is_value_only_codegen_ty`.
    pub(crate) fn lower_fallible_locus_method_body(
        &mut self,
        l: &LocusDecl,
        info: &LocusInfo<'ctx>,
        fd: &FnDecl,
    ) -> Result<(), CodegenError> {
        let func = *info
            .user_methods
            .get(&fd.name.name)
            .expect("fallible locus fn declared in pass A2");
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        let self_ptr = func
            .get_nth_param(0)
            .expect("self_ptr param")
            .into_pointer_value();
        self.current_fn = Some(func);
        // A2 (G2): normalize `-> ()` to None — same shape the
        // declare path uses for fallible signatures.
        let ret_te_normalized: Option<&TypeExpr> = match &fd.ret {
            Some(TypeExpr::Tuple(parts, _)) if parts.is_empty() => None,
            other => other.as_ref(),
        };
        let ret_ty: Option<CodegenTy> = match ret_te_normalized {
            None => None,
            Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
        };
        let payload_ty: CodegenTy = self.type_expr_to_codegen_ty(
            fd.fallible.as_ref().expect("called only when fallible"),
        )?;
        self.current_user_fn_ret = Some(ret_ty.clone());
        self.current_self = Some(SelfCx {
            locus_name: l.name.name.clone(),
            struct_ty: info.struct_ty,
            self_ptr,
            fields: info.fields.clone(),
        });
        self.loops.clear();
        self.push_dissolve_frame();
        // Open-question #24 v0.2: open a per-call scratch
        // subregion + snapshot caller_arena via TLS. The body
        // allocates transients into the scratch; the epilogue
        // deep-copies the ok/fail payload out into caller_arena
        // before destroying the scratch. Same shape as non-
        // fallible heap-returning locus methods.
        self.open_method_scratch()?;

        // ret_alloca + err_alloca + path_alloca. `alloca_for`
        // picks the right LLVM type for each.
        let ret_alloca = match &ret_ty {
            None => None,
            Some(rt) => Some(self.alloca_for(rt, "fn.ret.slot")?),
        };
        let err_alloca = self.alloca_for(&payload_ty, "fn.err.slot")?;
        let bool_t = self.context.bool_type();
        let path_alloca = self
            .builder
            .build_alloca(bool_t, "fn.fail.path")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(path_alloca, bool_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Find the sret slot params on the LLVM function. Slot
        // 0 is self_ptr; slots 1..=n are the user params; the
        // tail two (or one, for Unit success) are out_val /
        // out_err. Mirrors `declare_locus_methods`'s fallible
        // signature build above.
        let n_user_params = fd.params.len() as u32;
        let out_val_param = if ret_ty.is_some() {
            Some(
                func.get_nth_param(n_user_params + 1)
                    .expect("out_val sret param")
                    .into_pointer_value(),
            )
        } else {
            None
        };
        let out_err_slot_idx = if ret_ty.is_some() {
            n_user_params + 2
        } else {
            n_user_params + 1
        };
        let out_err_param = func
            .get_nth_param(out_err_slot_idx)
            .expect("out_err sret param")
            .into_pointer_value();

        let exit_bb = self.context.append_basic_block(func, "fn.exit");
        self.current_user_fn_exit_bb = Some(exit_bb);
        self.current_user_fn_ret_alloca = ret_alloca;
        self.current_user_fn_fallible = Some(FallibleCtx {
            err_alloca,
            path_alloca,
            out_val_param,
            out_err_param,
            payload_ty: payload_ty.clone(),
        });

        // Bind user params under their source-level names.
        let mut scope = Scope::default();
        for (i, p) in fd.params.iter().enumerate() {
            let lt = self.type_expr_to_codegen_ty(&p.ty)?;
            let alloca = self.alloca_for(&lt, &p.name.name)?;
            let v = func
                .get_nth_param((i + 1) as u32)
                .expect("locus method arg index in range");
            self.builder
                .build_store(alloca, v)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            scope.locals.insert(p.name.name.clone(), (alloca, lt));
        }

        let end = self.lower_block(&fd.body, &mut scope)?;
        if end == BlockEnd::Open {
            // Fall-through. For Unit success (`() fallible(E)`)
            // this is the natural shape: the body fires `fail`
            // on the error paths and otherwise runs to the
            // closing brace. path_alloca defaults to 0, so we
            // just br to exit and the ok arm of the epilogue
            // (which is a no-op store for Unit) takes us out.
            // For typed-return methods the user owes us an
            // explicit `return`.
            if ret_ty.is_some() {
                return Err(CodegenError::Unsupported(format!(
                    "locus `{}` method `{}`: fallible body falls through \
                     without `return` or `fail`",
                    l.name.name, fd.name.name
                )));
            }
            self.builder
                .build_unconditional_branch(exit_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // Emit the epilogue at exit_bb. Both `return` and
        // `fail` paths br to exit_bb (Stmt::Return / Stmt::Fail
        // detect `current_user_fn_exit_bb.is_some()` and route
        // through it). The epilogue branches on path_alloca.
        self.emit_fallible_locus_method_exit_epilogue(
            ret_ty.as_ref(),
            &payload_ty,
            path_alloca,
            ret_alloca,
            err_alloca,
            out_val_param,
            out_err_param,
        )?;

        self.current_user_fn_exit_bb = None;
        self.current_user_fn_ret_alloca = None;
        self.current_user_fn_fallible = None;
        self.current_fn = None;
        self.current_user_fn_ret = None;
        self.current_self = None;
        Ok(())
    }

    pub(crate) fn emit_fallible_locus_method_exit_epilogue(
        &mut self,
        ret_ty: Option<&CodegenTy>,
        payload_ty: &CodegenTy,
        path_alloca: PointerValue<'ctx>,
        ret_alloca: Option<PointerValue<'ctx>>,
        err_alloca: PointerValue<'ctx>,
        out_val_param: Option<PointerValue<'ctx>>,
        out_err_param: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let bool_t = self.context.bool_type();
        let func = self
            .current_fn
            .expect("current_fn set during fallible method body");
        let exit_bb = self
            .current_user_fn_exit_bb
            .expect("exit_bb set during fallible method body");
        self.builder.position_at_end(exit_bb);

        let ok_bb =
            self.context.append_basic_block(func, "fn.exit.ok");
        let fail_bb =
            self.context.append_basic_block(func, "fn.exit.fail");
        let cleanup_bb =
            self.context.append_basic_block(func, "fn.exit.cleanup");

        let path_v = self
            .builder
            .build_load(bool_t, path_alloca, "fn.fail.path.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        self.builder
            .build_conditional_branch(path_v, fail_bb, ok_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // OK branch. Load ret_alloca → (deep-copy into caller
        // arena for heap-bearing types) → store into out_val.
        // For Unit success (ret_ty None, out_val_param None)
        // the branch falls through with no store.
        self.builder.position_at_end(ok_bb);
        if let (Some(rt), Some(ret_slot), Some(out_val)) =
            (ret_ty, ret_alloca, out_val_param)
        {
            let llvm_ret_ty = self.llvm_basic_type(rt);
            let raw = self
                .builder
                .build_load(llvm_ret_ty, ret_slot, "fn.ret.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let copied = self.emit_method_return_deep_copy(raw, rt)?;
            self.builder
                .build_store(out_val, copied)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(cleanup_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // FAIL branch. Same deep-copy treatment for the err
        // payload — for value-only err types this is a no-op
        // pass-through (scalars / Cells), for heap-bearing
        // payloads it anchors the bytes in caller_arena before
        // the scratch goes away.
        self.builder.position_at_end(fail_bb);
        {
            let llvm_err_ty = self.llvm_basic_type(payload_ty);
            let raw = self
                .builder
                .build_load(llvm_err_ty, err_alloca, "fn.err.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let copied = self.emit_method_return_deep_copy(raw, payload_ty)?;
            self.builder
                .build_store(out_err_param, copied)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(cleanup_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // CLEANUP: shared scratch-destroy + flush + `ret i1
        // path`. Order matters: deep-copy already happened
        // above, so destroying the scratch here only frees
        // bytes the caller is no longer reading.
        self.builder.position_at_end(cleanup_bb);
        self.close_method_scratch()?;
        self.flush_dissolve_frame()?;
        self.builder
            .build_return(Some(&path_v))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// v1.x-FORM-2 PR6: fallible-fn exit epilogue. Branches on
    /// the i1 path indicator. Both branches deep-copy their
    /// respective local stage into caller_arena and store the
    /// resulting SSA into the caller-provided sret slot
    /// (`out_val_param` / `out_err_param`). The branches then
    /// converge on a cleanup block that runs the shared
    /// deferred-dissolves flush + subregion destroy + `ret i1
    /// path`.
    pub(crate) fn emit_fallible_fn_exit_epilogue(
        &mut self,
        sig: &FnSig<'ctx>,
        fallible: &FallibleCtx<'ctx>,
        caller_arena_alloca: PointerValue<'ctx>,
        fn_arena_alloca: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let bool_t = self.context.bool_type();
        let func = self
            .current_fn
            .expect("current_fn set during fn body lowering");

        let ok_bb = self.context.append_basic_block(func, "fn.exit.ok");
        let fail_bb = self.context.append_basic_block(func, "fn.exit.fail");
        let cleanup_bb =
            self.context.append_basic_block(func, "fn.exit.cleanup");

        // Builder is positioned at exit_bb on entry (caller did so).
        // Load the path indicator and conditional-branch.
        let path_v = self
            .builder
            .build_load(bool_t, fallible.path_alloca, "fn.fail.path.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        self.builder
            .build_conditional_branch(path_v, fail_bb, ok_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // OK branch: deep-copy ret_alloca → caller_arena, store
        // into out_val. v1 rejects void+fallible at declare time
        // (out_val_param is always Some when fallible).
        self.builder.position_at_end(ok_bb);
        if let (Some(ret_ty), Some(out_val_param)) =
            (&sig.ret, fallible.out_val_param)
        {
            let ret_alloca = self
                .current_user_fn_ret_alloca
                .expect("ret_alloca set when ret type is Some");
            let llvm_ret_ty = self.llvm_basic_type(ret_ty);
            let raw_ret = self
                .builder
                .build_load(llvm_ret_ty, ret_alloca, "fn.ret.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let dest_arena = self
                .builder
                .build_load(ptr_t, caller_arena_alloca, "caller_arena.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            let copied =
                self.emit_return_value_deep_copy(raw_ret, ret_ty, dest_arena)?;
            self.builder
                .build_store(out_val_param, copied)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(cleanup_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // FAIL branch: deep-copy err_alloca → caller_arena, store
        // into out_err.
        self.builder.position_at_end(fail_bb);
        {
            let llvm_err_ty = self.llvm_basic_type(&fallible.payload_ty);
            let raw_err = self
                .builder
                .build_load(llvm_err_ty, fallible.err_alloca, "fn.err.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let dest_arena = self
                .builder
                .build_load(ptr_t, caller_arena_alloca, "caller_arena.fail.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            let copied_err = self.emit_return_value_deep_copy(
                raw_err,
                &fallible.payload_ty,
                dest_arena,
            )?;
            self.builder
                .build_store(fallible.out_err_param, copied_err)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(cleanup_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // CLEANUP: same shared work the non-fallible epilogue does
        // — drain deferred dissolves and destroy the per-call
        // subregion — then `ret i1 path`. Doing flush exactly
        // once here is why the branches don't run their own
        // cleanup. `path_v` dominates this block (exit_bb is the
        // only predecessor of ok/fail/exit_bb chain).
        self.builder.position_at_end(cleanup_bb);
        self.flush_dissolve_frame()?;
        let arena_destroy = self
            .module
            .get_function("lotus_arena_destroy")
            .expect("lotus_arena_destroy declared");
        let fn_arena_loaded = self
            .builder
            .build_load(ptr_t, fn_arena_alloca, "fn.arena.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_call(
                arena_destroy,
                &[fn_arena_loaded.into()],
                "fn.arena.destroy",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_return(Some(&path_v))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// v1.x-FORM-2 PR6: lower an `inner or disposition` expression
    /// where `inner` resolves to a fallible call. Allocates the
    /// sret slots, emits the call (i1 path return), and branches:
    /// ok branch loads the success slot; err branch either
    /// substitutes per `or <expr>` / `or handler(err)` or
    /// propagates per `or raise` (sret-copy into enclosing
    /// fallible fn OR `lotus_root_panic` if escaping the
    /// implicit main locus). Substitute joins via phi; raise
    /// terminates the err branch, so the join only has the ok
    /// branch as predecessor.
    pub(crate) fn lower_or_expr(
        &mut self,
        inner: &Expr,
        disposition: &OrDisposition,
        scope: &Scope<'ctx>,
    ) -> Result<(Option<BasicValueEnum<'ctx>>, Option<CodegenTy>), CodegenError> {
        let call = self.lower_fallible_call(inner, scope)?;
        let func = self
            .current_fn
            .ok_or_else(|| {
                CodegenError::Unsupported(
                    "`or` expression outside a fn body".into(),
                )
            })?;
        let ok_bb = self.context.append_basic_block(func, "or.ok");
        let err_bb = self.context.append_basic_block(func, "or.err");
        self.builder
            .build_conditional_branch(call.i1_path, err_bb, ok_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // OK branch: load the success value out of the sret slot
        // when there is one. v1.x-FORM-4: `() fallible(E)` calls
        // (e.g. hashmap.remove) carry `success_ty = None` and no
        // out_val_slot — the ok branch just falls through to the
        // join.
        self.builder.position_at_end(ok_bb);
        let ok_v_opt: Option<BasicValueEnum<'ctx>> =
            match (&call.success_ty, call.out_val_slot) {
                (Some(succ_ty), Some(slot)) => {
                    let llvm_succ_ty = self.llvm_basic_type(succ_ty);
                    let v = self
                        .builder
                        .build_load(llvm_succ_ty, slot, "or.ok.val")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    Some(v)
                }
                _ => None,
            };
        let ok_end_bb = self
            .builder
            .get_insert_block()
            .expect("ok branch open");

        // ERR branch.
        self.builder.position_at_end(err_bb);
        let err_join: Option<(Option<BasicValueEnum<'ctx>>, inkwell::basic_block::BasicBlock<'ctx>)> =
            match disposition {
                OrDisposition::Raise(_) => {
                    self.lower_or_raise(&call)?;
                    None
                }
                OrDisposition::Fail(payload, _) => {
                    // B3 / G6: `or fail X` — evaluate X as the
                    // enclosing fallible fn's declared payload type
                    // and divert to the err-path exit. Symmetric to
                    // `or raise` but the caller picks the payload
                    // (often to translate one error shape into
                    // another), rather than re-emitting the inner
                    // call's payload verbatim.
                    self.lower_or_fail(payload, &call, scope)?;
                    None
                }
                OrDisposition::Discard(_) => {
                    // `or discard`: success type must be Unit;
                    // err branch is a no-op (no fallback expr
                    // evaluated, err value swallowed).
                    if call.success_ty.is_some() {
                        return Err(CodegenError::Unsupported(format!(
                            "`or discard` requires the underlying call's \
                             success type to be Unit (since discard \
                             produces no value to bind); got {:?}. Use \
                             `or <default-value>` or `or raise` for \
                             value-bearing fallible calls.",
                            call.success_ty
                        )));
                    }
                    let sub_end_bb = self
                        .builder
                        .get_insert_block()
                        .expect("discard branch open");
                    Some((None, sub_end_bb))
                }
                OrDisposition::Substitute(rhs) => {
                    // `err` binding implicit on substitute RHS, per
                    // AST docstring (`Expr::Or`). The binding's
                    // alloca is the inner call's out_err_slot, so
                    // reads of `err.field` GEP through it the same
                    // way reads of any locally-bound TypeRef would.
                    let mut sub_scope = Scope {
                        locals: scope.locals.clone(),
                    };
                    sub_scope.locals.insert(
                        "err".to_string(),
                        (call.out_err_slot, call.payload_ty.clone()),
                    );
                    let (sub_v, sub_ty) = if call.success_ty.is_none() {
                        // v1.x-FORM-4: Unit-success fallible (e.g.
                        // hashmap.remove / write_file). The substitute RHS
                        // is Unit-typed.
                        let mut sub_mut_scope = Scope {
                            locals: sub_scope.locals.clone(),
                        };
                        match rhs.as_ref() {
                            // A `{ block }` disposer — `… or { println(…); }`
                            // or `… or { return; }`. Lower the block body
                            // directly: `lower_stmt` has no
                            // `Stmt::Expr(Block)` form, so wrapping it as a
                            // statement hits the "expression statement other
                            // than locus literal or builtin call" reject
                            // (this is the `or { block }`-in-statement-
                            // position gap; it bit the `let _: ()` form too).
                            Expr::Block(b) => {
                                self.lower_block(b, &mut sub_mut_scope)?;
                            }
                            // Any other Unit-typed RHS — `ignore(err)`,
                            // `noop()` — lower as a statement so a Unit-
                            // returning fn call works without the
                            // expression-position "fn returns no value"
                            // reject.
                            _ => {
                                let s = Stmt::Expr((**rhs).clone());
                                self.lower_stmt(&s, &mut sub_mut_scope)?;
                            }
                        }
                        (None, None)
                    } else {
                        self.lower_expr_opt(rhs, &sub_scope)?
                    };
                    // A disposer that always diverges — `or { return …; }`
                    // or `or { fail …; }` — terminates the err branch and
                    // produces NO substitute value: `lower_block_as_expr`
                    // hands back a placeholder `(undef, Int)` for the
                    // unreachable fall-through. Detect the terminator and
                    // treat it like `or raise`: the err branch is closed,
                    // only the ok value reaches the join, and the
                    // placeholder type is irrelevant (so the fallible's
                    // success type may be String / Bytes / a struct, not
                    // just the spurious Int).
                    let diverged = self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_some();
                    if diverged {
                        None
                    } else {
                        if sub_ty != call.success_ty {
                            return Err(CodegenError::Unsupported(format!(
                                "`or` substitute type mismatch: expected \
                                 {:?}, got {:?}",
                                call.success_ty, sub_ty
                            )));
                        }
                        // The rhs lowering may have created intermediate
                        // blocks; the current insert block is where we
                        // need to br from for the phi to see the right
                        // predecessor.
                        let sub_end_bb = self
                            .builder
                            .get_insert_block()
                            .expect("substitute branch open");
                        Some((sub_v, sub_end_bb))
                    }
                }
            };

        // JOIN.
        let join_bb = self.context.append_basic_block(func, "or.join");
        self.builder.position_at_end(ok_end_bb);
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        match err_join {
            None => {
                // Raise: err branch terminated. Only ok flows to
                // the join. Result is the ok-branch SSA directly
                // (or None for Unit-success).
                self.builder.position_at_end(join_bb);
                Ok((ok_v_opt, call.success_ty))
            }
            Some((sub_v, sub_end_bb)) => {
                self.builder.position_at_end(sub_end_bb);
                self.builder
                    .build_unconditional_branch(join_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(join_bb);
                match (ok_v_opt, sub_v, &call.success_ty) {
                    (Some(ok_v), Some(sv), Some(succ_ty)) => {
                        let llvm_succ_ty = self.llvm_basic_type(succ_ty);
                        let phi = self
                            .builder
                            .build_phi(llvm_succ_ty, "or.result")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        phi.add_incoming(&[
                            (&ok_v, ok_end_bb),
                            (&sv, sub_end_bb),
                        ]);
                        Ok((Some(phi.as_basic_value()), call.success_ty))
                    }
                    _ => Ok((None, call.success_ty)),
                }
            }
        }
    }

    /// v1.x-FORM-4: wrapper around `lower_expr` for sub-expressions
    /// whose result may be Unit. `lower_expr` returns
    /// `(BasicValueEnum, CodegenTy)` — for `or`-disposition
    /// substitute RHSs that may evaluate to Unit (matching a
    /// `() fallible(E)` call), we need the option-wrapped variant.
    /// Falls through to `lower_expr` for the value-producing case
    /// and special-cases `Expr::Tuple([])` (the Unit literal `()`)
    /// for the no-value case.
    pub(crate) fn lower_expr_opt(
        &mut self,
        e: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<(Option<BasicValueEnum<'ctx>>, Option<CodegenTy>), CodegenError> {
        if let Expr::Tuple(parts, _) = e {
            if parts.is_empty() {
                return Ok((None, None));
            }
        }
        let (v, ty) = self.lower_expr(e, scope)?;
        Ok((Some(v), Some(ty)))
    }

    /// v1.x-FORM-2 PR6: lower the inner expression of an `or`,
    /// emitting the fallible-ABI call. Today only handles direct
    /// `Expr::Ident(fn_name)` callees that resolve to a fallible
    /// user fn. Synthesized form-vec methods (`l.get(i)`,
    /// `l.pop()`) lower as-if-fallible inline in commit 4 (PR5
    /// finale); other shapes (path calls, generic-fn callees)
    /// reject with a clear diagnostic.
    pub(crate) fn lower_fallible_call(
        &mut self,
        inner: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let (callee, args) = match inner {
            Expr::Call { callee, args, .. } => (callee.as_ref(), args),
            _ => {
                return Err(CodegenError::Unsupported(format!(
                    "`or` requires a fallible call expression; got {:?}",
                    std::mem::discriminant(inner)
                )));
            }
        };
        // Resolve callee to a free-fn name in `user_fns`. Three
        // shapes route here:
        //   - `Expr::Ident(id)` — bare free-fn call.
        //   - `Expr::Field { receiver, name }` — method call;
        //     dispatched to lower_fallible_method_call.
        //   - `Expr::Path(qn)` — path-qualified call. Two
        //     sub-cases: imported-lib free fns resolve via
        //     `mangled_for_path` into `user_fns` exactly like
        //     bare idents; stdlib path-calls (`std::io::fs::*`,
        //     etc.) route to `try_lower_fallible_stdlib_path_call`
        //     for the per-path synthesis (see #68 / IoError flip).
        let resolved_name: String = match callee {
            Expr::Ident(id) => id.name.clone(),
            Expr::Field { receiver, name, .. } => {
                return self.lower_fallible_method_call(
                    receiver, &name.name, args, scope,
                );
            }
            Expr::Path(qn) => {
                let segs: Vec<&str> =
                    qn.segments.iter().map(|s| s.name.as_str()).collect();
                // Try stdlib fallible synth first — the fs/tcp
                // path-calls that #68 flipped to fallible(IoError)
                // emit their wrappers inline here.
                if let Some(result) =
                    self.try_lower_fallible_stdlib_path_call(&segs, args, scope)?
                {
                    return Ok(result);
                }
                // Otherwise the path is either an imported-lib free
                // fn (mangled name lives in `user_fns`) or unknown.
                match self.mangled_for_path(&segs) {
                    Some(mangled) => mangled,
                    None => {
                        return Err(CodegenError::Unsupported(format!(
                            "`or` over unknown path call `{}`",
                            segs.join("::")
                        )));
                    }
                }
            }
            _ => {
                return Err(CodegenError::Unsupported(format!(
                    "`or` callee shape not yet supported: {:?}",
                    std::mem::discriminant(callee)
                )));
            }
        };
        let sig = self
            .user_fns
            .get(&resolved_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "`or` over call to unknown fn `{}`",
                    resolved_name
                ))
            })?;
        let payload_ty = sig.fallible.clone().ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "`or` applied to non-fallible fn `{}`",
                resolved_name
            ))
        })?;
        // A2 (G2): `-> () fallible(E)` is the Unit-success shape.
        // `success_ty` stays None; out_val sret slot is skipped at
        // call site; FallibleCallResult carries `out_val_slot: None`
        // so the downstream `or` machinery falls through cleanly.
        let success_ty: Option<CodegenTy> = sig.ret.clone();
        if args.len() > sig.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "fallible fn `{}` expects at most {} args, got {}",
                resolved_name,
                sig.params.len(),
                args.len()
            )));
        }
        for (i, default) in sig.defaults.iter().enumerate() {
            if i >= args.len() && default.is_none() {
                return Err(CodegenError::Unsupported(format!(
                    "fallible fn `{}`: required param at position {} not \
                     provided (only {} args given)",
                    resolved_name,
                    i,
                    args.len()
                )));
            }
        }

        // Allocate the sret slots BEFORE lowering args so the
        // alloca lands in the entry block (LLVM's mem2sem treats
        // entry-block allocas specially) and the slot pointers
        // are stable across arg-expression lowering.
        // A2 (G2): Unit-success fallible fns have no out_val slot —
        // skip the alloca + arg push entirely. The callee's ABI is
        // `(__caller_arena, params..., out_err) -> i1`.
        let out_val_slot: Option<PointerValue<'ctx>> = match &success_ty {
            Some(st) => Some(self.alloca_for(st, "or.out_val.slot")?),
            None => None,
        };
        let out_err_slot = self.alloca_for(&payload_ty, "or.out_err.slot")?;

        let caller_arena_at_call = self.current_arena_ptr()?;
        let mut llvm_args: Vec<BasicMetadataValueEnum<'ctx>> =
            Vec::with_capacity(sig.params.len() + 3);
        llvm_args.push(caller_arena_at_call.into());
        for i in 0..sig.params.len() {
            let (v, ty) = if i < args.len() {
                self.lower_expr(&args[i], scope)?
            } else {
                let default = sig
                    .defaults[i]
                    .as_ref()
                    .expect("checked above");
                self.lower_expr(default, scope)?
            };
            // Same arg-coercion shape as lower_user_fn_call.
            let v = if let (CodegenTy::Interface(iface), CodegenTy::LocusRef(l)) =
                (&sig.params[i], &ty)
            {
                let fat = self.coerce_to_interface(
                    v.into_pointer_value(),
                    l,
                    iface,
                )?;
                fat.into()
            } else if sig.params[i] == CodegenTy::Float && ty == CodegenTy::Int {
                let widened = self.coerce_to_float(
                    v,
                    &ty,
                    &format!("fallible fn `{}` arg {}", resolved_name, i),
                )?;
                widened.into()
            } else if view_coerces_to(&ty, &sig.params[i]) {
                // F.30 / F.30b (5a): BytesView → Bytes / StringView →
                // String at user-defined fallible-fn arg sites.
                // Mirrors the non-fallible fn-arg coercion in
                // lower_user_fn_call. The unpack helper emits the
                // epoch check + extracts the underlying data ptr.
                self.unpack_view_if_needed(v, &ty)?
            } else if ty != sig.params[i] {
                return Err(CodegenError::Unsupported(format!(
                    "fallible fn `{}` arg {} type mismatch: expected {:?}, \
                     got {:?}",
                    resolved_name, i, sig.params[i], ty
                )));
            } else {
                v
            };
            llvm_args.push(v.into());
        }
        if let Some(slot) = &out_val_slot {
            llvm_args.push((*slot).into());
        }
        llvm_args.push(out_err_slot.into());

        let call = self
            .builder
            .build_call(
                sig.func,
                &llvm_args,
                &format!("{}.fallible.call", resolved_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i1_path = call
            .try_as_basic_value()
            .left()
            .expect("fallible fn returns i1")
            .into_int_value();

        Ok(FallibleCallResult {
            i1_path,
            out_val_slot,
            out_err_slot,
            success_ty,
            payload_ty,
        })
    }

    /// Dispatcher for `path or raise` where the path resolves to a
    /// fallible stdlib path-call (`std::io::fs::read_file`, etc.).
    /// Returns:
    ///   - `Ok(Some(result))` — the path matched a known fallible
    ///     stdlib surface; the call was lowered.
    ///   - `Ok(None)` — the path didn't match; caller falls through
    ///     to other resolution paths.
    ///   - `Err(_)` — the path matched but lowering failed (arity,
    ///     type, etc.).
    /// Specific paths are wired by individual `lower_std_*_fallible`
    /// methods that this dispatcher routes to. See the IoError flip
    /// for the fs/tcp surface.
    pub(crate) fn try_lower_fallible_stdlib_path_call(
        &mut self,
        segs: &[&str],
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<FallibleCallResult<'ctx>>, CodegenError> {
        // Per-path wrappers for the fs/tcp surfaces flipped to
        // `fallible(IoError)`. Each helper evaluates args, calls
        // the underlying C primitive, and feeds the sentinel
        // result + path into `complete_io_fallible_call` to build
        // the lazy-IoError branch.
        match segs {
            ["std", "io", "fs", "read_file"] => Ok(Some(
                self.lower_std_io_fs_read_file_fallible(args, scope)?,
            )),
            ["std", "io", "fs", "read_bytes"] => Ok(Some(
                self.lower_std_io_fs_read_bytes_fallible(args, scope)?,
            )),
            ["std", "io", "fs", "write_file"] => Ok(Some(
                self.lower_std_io_fs_write_file_fallible(
                    args, scope, "lotus_fs_write_file",
                )?,
            )),
            ["std", "io", "fs", "write_file_append"] => Ok(Some(
                self.lower_std_io_fs_write_file_fallible(
                    args, scope, "lotus_fs_write_file_append",
                )?,
            )),
            ["std", "io", "fs", "file_size"] => Ok(Some(
                self.lower_std_io_fs_file_size_fallible(args, scope)?,
            )),
            ["std", "io", "fs", "mkdir"] => Ok(Some(
                self.lower_std_io_fs_mkdir_fallible(args, scope)?,
            )),
            // C9 (pond/logfmt + pond/agent/sandbox).
            ["std", "io", "fs", "rename"] => Ok(Some(
                self.lower_std_io_fs_rename_fallible(args, scope)?,
            )),
            ["std", "io", "fs", "unlink"] => Ok(Some(
                self.lower_std_io_fs_unlink_fallible(args, scope)?,
            )),
            ["std", "io", "fs", "mktemp"] => Ok(Some(
                self.lower_std_io_fs_mktemp_fallible(args, scope)?,
            )),
            ["std", "io", "fs", "list_dir_count"] => Ok(Some(
                self.lower_std_io_fs_list_dir_count_fallible(args, scope)?,
            )),
            ["std", "io", "fs", "list_dir_at"] => Ok(Some(
                self.lower_std_io_fs_list_dir_at_fallible(args, scope)?,
            )),
            ["std", "io", "tcp", "listen_socket"] => Ok(Some(
                self.lower_std_io_tcp_listen_socket_fallible(args, scope)?,
            )),
            ["std", "io", "tcp", "connect"] => Ok(Some(
                self.lower_std_io_tcp_connect_fallible(args, scope)?,
            )),
            ["std", "io", "tcp", "accept_one"] => Ok(Some(
                self.lower_std_io_tcp_accept_one_fallible(args, scope)?,
            )),
            // TLS: connect handshakes + system trust verification,
            // so the failure surface is rich enough to warrant
            // `fallible(IoError)`. send_bytes / recv_bytes / close
            // stay non-fallible (Int 0/-1 returns) to mirror the
            // tcp shape.
            ["std", "io", "tls", "connect"] => Ok(Some(
                self.lower_std_io_tls_connect_fallible(args, scope)?,
            )),
            ["std", "bytes", "at"] => Ok(Some(
                self.lower_std_bytes_at_fallible(args, scope)?,
            )),
            // shm-ring-interop Proposal A: binary-pack readers
            // `read_<type>_<endian>(b, off) -> Int|Float
            // fallible(IndexError)`.
            ["std", "bytes", n] if n.starts_with("read_") => Ok(Some(
                self.lower_std_bytes_read(n, args, scope)?,
            )),
            // A1 zero-copy write: `write_<type>_<endian>(w, off, val) -> ()
            // fallible(IndexError)`.
            ["std", "bytes", n] if n.starts_with("write_") => Ok(Some(
                self.lower_std_bytes_write(n, args, scope)?,
            )),
            ["std", "str", "parse_int"] => Ok(Some(
                self.lower_std_str_parse_int_fallible(args, scope)?,
            )),
            ["std", "str", "parse_float"] => Ok(Some(
                self.lower_std_str_parse_float_fallible(args, scope)?,
            )),
            ["std", "str", "parse_decimal"] => Ok(Some(
                self.lower_std_str_parse_decimal_fallible(args, scope)?,
            )),
            // 2026-05-26: range-bounded variants for allocation-
            // free JSON walks. Take (json, start, end_exclusive)
            // instead of an owned substring.
            ["std", "str", "range_parse_int"] => Ok(Some(
                self.lower_std_str_range_parse_int_fallible(args, scope)?,
            )),
            ["std", "str", "range_parse_decimal"] => Ok(Some(
                self.lower_std_str_range_parse_decimal_fallible(args, scope)?,
            )),
            // C4 (pond/crypto follow-up): CSPRNG getrandom.
            ["std", "os", "getrandom"] => Ok(Some(
                self.lower_std_os_getrandom_fallible(args, scope)?,
            )),
            // 2026-06-04: ECDSA P-256 signing in `or` context →
            // fallible(CryptoError). Bare calls keep the empty-bytes
            // form via the non-fallible dispatcher.
            ["std", "crypto", "ecdsa_p256_sign"] => Ok(Some(
                self.lower_std_crypto_ecdsa_p256_sign_fallible(args, scope)?,
            )),
            // C2 (pond/subprocess): synchronous run + async
            // lifecycle primitives. `run` is user-facing; the
            // `__*` variants are stdlib internals consumed by
            // process.hl's spawn/wait/kill wrappers.
            ["std", "process", "run"] => Ok(Some(
                self.lower_std_process_run_fallible(args, scope)?,
            )),
            ["std", "process", "__spawn"] => Ok(Some(
                self.lower_std_process_spawn_fallible(args, scope)?,
            )),
            // UDP primitives: `__bind` returns Int fd, `__send`
            // returns (), `__recv` returns Bytes.
            ["std", "io", "udp", "__bind"]
            | ["std", "io", "udp", "bind"] => Ok(Some(
                self.lower_std_io_udp_bind_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "__send"]
            | ["std", "io", "udp", "send"] => Ok(Some(
                self.lower_std_io_udp_send_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "__recv"]
            | ["std", "io", "udp", "recv"] => Ok(Some(
                self.lower_std_io_udp_recv_fallible(args, scope)?,
            )),
            // 2026-05-26: UDP multicast (P1) + setsockopt
            // pass-through (P2).
            ["std", "io", "udp", "join_group"] => Ok(Some(
                self.lower_std_io_udp_join_group_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "leave_group"] => Ok(Some(
                self.lower_std_io_udp_leave_group_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "set_multicast_ttl"] => Ok(Some(
                self.lower_std_io_udp_set_multicast_ttl_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "set_multicast_loop"] => Ok(Some(
                self.lower_std_io_udp_set_multicast_loop_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "set_multicast_iface"] => Ok(Some(
                self.lower_std_io_udp_set_multicast_iface_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "set_option_int"] => Ok(Some(
                self.lower_std_io_udp_set_option_int_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "set_option_bool"] => Ok(Some(
                self.lower_std_io_udp_set_option_bool_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "get_option_int"] => Ok(Some(
                self.lower_std_io_udp_get_option_int_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "recv_with_source"] => Ok(Some(
                self.lower_std_io_udp_recv_with_source_fallible(args, scope)?,
            )),
            ["std", "io", "udp", "set_recv_timeout"] => Ok(Some(
                self.lower_std_io_udp_set_timeout_fallible(
                    args, scope,
                    "lotus_udp_set_recv_timeout_ns",
                    "set_recv_timeout",
                )?,
            )),
            ["std", "io", "udp", "set_send_timeout"] => Ok(Some(
                self.lower_std_io_udp_set_timeout_fallible(
                    args, scope,
                    "lotus_udp_set_send_timeout_ns",
                    "set_send_timeout",
                )?,
            )),
            // 2026-05-27 — TCP send/recv timeouts. Same helper
            // as udp; the C side shares the underlying
            // sock_set_timeout_ns. Sole reason for a separate
            // path-call site (vs. one shared `std::io::sock`
            // namespace) is the typecheck-level fd-type
            // discrimination: a tcp fd shouldn't accept a udp-
            // shaped op.
            ["std", "io", "tcp", "set_recv_timeout"] => Ok(Some(
                self.lower_std_io_udp_set_timeout_fallible(
                    args, scope,
                    "lotus_tcp_set_recv_timeout_ns",
                    "set_recv_timeout",
                )?,
            )),
            ["std", "io", "tcp", "set_send_timeout"] => Ok(Some(
                self.lower_std_io_udp_set_timeout_fallible(
                    args, scope,
                    "lotus_tcp_set_send_timeout_ns",
                    "set_send_timeout",
                )?,
            )),
            // 2026-06-13 — TCP_NODELAY (Nagle off). The headline
            // socket-option gap: latency-sensitive TCP protocols
            // need to disable Nagle and could not from Hale before.
            ["std", "io", "tcp", "set_nodelay"] => Ok(Some(
                self.lower_std_io_tcp_set_nodelay(args, scope)?,
            )),
            // 2026-06-13 — recv_stamped (#1): one-time SO_TIMESTAMPNS
            // opt-in so recv_stamped_into reads the kernel RX timestamp
            // with no per-recv syscall.
            ["std", "io", "tcp", "set_rx_timestamps"] => Ok(Some(
                self.lower_std_io_tcp_set_rx_timestamps(args, scope)?,
            )),
            // TLS fast-path siblings — same fd+Bool helper; the C side
            // resolves the handle to the underlying socket fd (2026-06-14).
            ["std", "io", "tls", "set_nodelay"] => Ok(Some(
                self.lower_tcp_set_bool_opt_fallible(
                    args, scope, "lotus_tls_set_nodelay", "set_nodelay",
                )?,
            )),
            ["std", "io", "tls", "set_rx_timestamps"] => Ok(Some(
                self.lower_tcp_set_bool_opt_fallible(
                    args, scope, "lotus_tls_set_rx_timestamps", "set_rx_timestamps",
                )?,
            )),
            // TLS siblings — same helper; the first arg is a TLS handle, the
            // C side resolves it to the connection's underlying fd. Bounds a
            // blocking SSL_read so a half-open connection is detected
            // (WsClient liveness fix) rather than hanging forever.
            ["std", "io", "tls", "set_recv_timeout"] => Ok(Some(
                self.lower_std_io_udp_set_timeout_fallible(
                    args, scope,
                    "lotus_tls_set_recv_timeout_ns",
                    "set_recv_timeout",
                )?,
            )),
            ["std", "io", "tls", "set_send_timeout"] => Ok(Some(
                self.lower_std_io_udp_set_timeout_fallible(
                    args, scope,
                    "lotus_tls_set_send_timeout_ns",
                    "set_send_timeout",
                )?,
            )),
            // File primitives: only the `__`-prefixed forms map
            // here. The user-facing `open` / `write_bytes` / `seek`
            // resolve via STDLIB_FN_RENAMES to the Hale-level
            // wrappers in runtime/stdlib/file.hl that bridge
            // File ↔ fd (open returns a File, write_bytes/seek
            // take a File).
            ["std", "io", "file", "__open"] => Ok(Some(
                self.lower_std_io_file_open_fallible(args, scope)?,
            )),
            ["std", "io", "file", "__write_bytes"] => Ok(Some(
                self.lower_std_io_file_write_bytes_fallible(args, scope)?,
            )),
            ["std", "io", "file", "__seek"] => Ok(Some(
                self.lower_std_io_file_seek_fallible(args, scope)?,
            )),
            ["std", "process", "__wait_pid"] => Ok(Some(
                self.lower_std_process_wait_pid_fallible(args, scope)?,
            )),
            ["std", "process", "__kill_escalate"] => Ok(Some(
                self.lower_std_process_kill_escalate_fallible(args, scope)?,
            )),
            ["std", "process", "__pipe_read"] => Ok(Some(
                self.lower_std_process_pipe_read_fallible(args, scope)?,
            )),
            ["std", "process", "__pipe_write"] => Ok(Some(
                self.lower_std_process_pipe_write_fallible(args, scope)?,
            )),
            // Known stdlib paths that AREN'T fallible — surface a
            // focused diagnostic instead of "unknown path call".
            // Each name here is a path-call that returns a value
            // directly (no error channel); the user accidentally
            // wrapped it in `or`. The fix is to remove the clause.
            ["std", "bytes", "slice"]
            | ["std", "bytes", "from_string"]
            | ["std", "str", "from_bytes"]
            | ["std", "str", "lower"]
            | ["std", "str", "upper"]
            | ["std", "str", "trim"]
            | ["std", "str", "substring"]
            | ["std", "str", "replace"]
            | ["std", "str", "repeat"]
            | ["std", "str", "pad_left"]
            | ["std", "str", "pad_right"]
            | ["std", "str", "index_of"]
            | ["std", "str", "can_parse_int"]
            | ["std", "str", "can_parse_float"]
            | ["std", "str", "builder_new"]
            | ["std", "str", "builder_append"]
            | ["std", "str", "builder_len"]
            | ["std", "str", "builder_finish"]
            | ["std", "bytes", "builder", "__new"]
            | ["std", "bytes", "builder", "__append"]
            | ["std", "bytes", "builder", "__append_str"]
            | ["std", "bytes", "builder", "__len"]
            | ["std", "bytes", "builder", "__finish"]
            | ["std", "bytes", "builder", "__shift_front"]
            | ["std", "bytes", "builder", "__clear"]
            | ["std", "bytes", "builder", "__snapshot"]
            | ["std", "bytes", "builder", "__free"]
            | ["std", "bytes", "builder", "__view"]
            | ["std", "bytes", "builder", "__text_view"]
            | ["std", "bytes", "builder", "__append_slice"]
            | ["std", "bytes", "__is_alloc_fail"]
            | ["std", "bytes", "clone"]
            | ["std", "str", "clone"]
            | ["std", "math", "sqrt"]
            | ["std", "math", "exp"]
            | ["std", "math", "log"]
            | ["std", "math", "floor"]
            | ["std", "math", "ceil"]
            | ["std", "math", "pow"]
            | ["std", "math", "tanh"]
            | ["std", "math", "nan"]
            | ["std", "math", "inf"]
            | ["std", "math", "is_nan"]
            | ["std", "math", "sin"]
            | ["std", "math", "cos"]
            | ["std", "math", "tan"]
            | ["std", "math", "asin"]
            | ["std", "math", "acos"]
            | ["std", "math", "atan"]
            | ["std", "math", "atan2"]
            | ["std", "io", "fs", "file_exists"]
            | ["std", "io", "tcp", "close_fd"]
            | ["std", "io", "stdin", "read_line"]
            | ["std", "io", "stdin", "read_line_status"]
            | ["std", "env", "args_count"]
            | ["std", "env", "arg"]
            | ["std", "env", "arg_or"]
            | ["std", "env", "var"]
            | ["std", "env", "var_exists"]
            | ["std", "process", "pid"]
            | ["std", "time", "monotonic"]
            | ["std", "time", "sleep"]
            | ["std", "text", "is_alpha"]
            | ["std", "text", "is_digit"]
            | ["std", "text", "is_alnum"]
            | ["std", "text", "is_whitespace"]
            | ["std", "text", "is_word_char"]
            | ["std", "text", "tokenize_words_into"] => Err(CodegenError::Unsupported(format!(
                "`{}` is not a fallible call — remove the `or` clause. \
                 Returns its value directly; failures (if any) use the \
                 sentinel-with-discriminator idiom or are infallible.",
                segs.join("::")
            ))),
            _ => Ok(None),
        }
    }

    /// Shared completion helper for the IoError-bearing fallible
    /// path-calls. The caller has evaluated args, run the C
    /// primitive, and computed `is_err: i1` from the primitive's
    /// sentinel return. This helper:
    ///   1. Branches on is_err.
    ///   2. On err path: emits the lazy IoError into out_err_slot.
    ///   3. Joins; returns the FallibleCallResult.
    /// `success_value` is Some(value, ty) for value-returning
    /// surfaces (read_file → String, file_size → Int) or None for
    /// Unit-returning surfaces (write_file, mkdir).
    pub(crate) fn complete_io_fallible_call(
        &mut self,
        is_err: IntValue<'ctx>,
        path_str_ptr: BasicValueEnum<'ctx>,
        success_value: Option<(BasicValueEnum<'ctx>, CodegenTy)>,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let payload_ty = CodegenTy::TypeRef("IoError".to_string());
        let out_err_slot = self.alloca_for(
            &payload_ty,
            &format!("{}.out_err.slot", label),
        )?;
        let (out_val_slot_opt, success_ty_opt) = match &success_value {
            Some((_, ty)) => (
                Some(self.alloca_for(
                    ty,
                    &format!("{}.out_val.slot", label),
                )?),
                Some(ty.clone()),
            ),
            None => (None, None),
        };

        let func = self
            .current_fn
            .expect("fallible-stdlib-path call inside fn body");
        let lazy_err_bb = self.context.append_basic_block(
            func,
            &format!("{}.lazy_err", label),
        );
        let store_ok_bb = self.context.append_basic_block(
            func,
            &format!("{}.store_ok", label),
        );
        let join_bb = self.context.append_basic_block(
            func,
            &format!("{}.join", label),
        );

        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, store_ok_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // err path: build IoError, store in out_err_slot.
        self.builder.position_at_end(lazy_err_bb);
        let ie_ptr = self.emit_io_error_alloc(path_str_ptr)?;
        self.builder
            .build_store(out_err_slot, ie_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // ok path: store the success value (if any).
        self.builder.position_at_end(store_ok_bb);
        if let (Some(out_val_slot), Some((val, _))) = (out_val_slot_opt, &success_value) {
            self.builder
                .build_store(out_val_slot, *val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: out_val_slot_opt,
            out_err_slot,
            success_ty: success_ty_opt,
            payload_ty,
        })
    }

    /// 2026-05-17 — analog of `complete_io_fallible_call` for
    /// `std::str::parse_int` / `parse_float`. Difference: the err
    /// payload is `ParseError { kind, input }` instead of IoError.
    /// Caller provides the kind tag string + input pointer.
    pub(crate) fn complete_parse_fallible_call(
        &mut self,
        is_err: IntValue<'ctx>,
        input_str_ptr: BasicValueEnum<'ctx>,
        kind_tag: &str,
        success_value: Option<(BasicValueEnum<'ctx>, CodegenTy)>,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let payload_ty = CodegenTy::TypeRef("ParseError".to_string());
        let out_err_slot = self.alloca_for(
            &payload_ty,
            &format!("{}.out_err.slot", label),
        )?;
        let (out_val_slot_opt, success_ty_opt) = match &success_value {
            Some((_, ty)) => (
                Some(self.alloca_for(
                    ty,
                    &format!("{}.out_val.slot", label),
                )?),
                Some(ty.clone()),
            ),
            None => (None, None),
        };

        let func = self
            .current_fn
            .expect("fallible-stdlib-path call inside fn body");
        let lazy_err_bb = self.context.append_basic_block(
            func,
            &format!("{}.lazy_err", label),
        );
        let store_ok_bb = self.context.append_basic_block(
            func,
            &format!("{}.store_ok", label),
        );
        let join_bb = self.context.append_basic_block(
            func,
            &format!("{}.join", label),
        );

        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, store_ok_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // err path: build ParseError, store in out_err_slot.
        self.builder.position_at_end(lazy_err_bb);
        let pe_ptr = self.emit_parse_error_alloc(input_str_ptr, kind_tag)?;
        self.builder
            .build_store(out_err_slot, pe_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // ok path: store the success value (if any).
        self.builder.position_at_end(store_ok_bb);
        if let (Some(out_val_slot), Some((val, _))) =
            (out_val_slot_opt, &success_value)
        {
            self.builder
                .build_store(out_val_slot, *val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: out_val_slot_opt,
            out_err_slot,
            success_ty: success_ty_opt,
            payload_ty,
        })
    }

    /// Allocate a `ParseError { kind, input }` in the current
    /// arena, populate, return pointer.
    ///
    /// v1.x polish (2026-05-20): converts a user-declared
    /// `type ParseError` that's missing the stdlib's expected
    /// `kind: String` / `input: String` fields from a hard
    /// runtime panic into a clean codegen-time diagnostic.
    /// Users hitting this either rename their type or use the
    /// `std::str::ParseError` path-rename for the stdlib's
    /// shape.
    pub(crate) fn emit_parse_error_alloc(
        &mut self,
        input_str_ptr: BasicValueEnum<'ctx>,
        kind_tag: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let info = self
            .user_types
            .get("ParseError")
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(
                    "ParseError type missing — `declare_builtin_parse_error_type` \
                     should have injected it; sequencing bug?".to_string(),
                )
            })?;
        let size = info
            .struct_ty
            .size_of()
            .expect("ParseError has known size");
        let alloc_ptr = self.arena_alloc(size, "ParseError.alloc")?;

        let kind_ptr = self.global_string(kind_tag);
        let (kind_idx, _) = info.fields.get("kind").cloned().ok_or_else(|| {
            CodegenError::Unsupported(
                "user-declared `type ParseError` is missing the stdlib's \
                 expected `kind: String` field — std::str::parse_* fns need \
                 to allocate `ParseError { kind, input }` on failure. \
                 Either match the stdlib shape (`type ParseError { kind: \
                 String; input: String; }`), rename your type (e.g. \
                 `MyParseError`), or use the `std::str::ParseError` qualified \
                 path where you need the stdlib's"
                    .to_string(),
            )
        })?;
        let kind_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                alloc_ptr,
                kind_idx,
                "ParseError.kind.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(kind_field_ptr, kind_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let (input_idx, _) = info.fields.get("input").cloned().ok_or_else(|| {
            CodegenError::Unsupported(
                "user-declared `type ParseError` is missing the stdlib's \
                 expected `input: String` field — see the `kind` field \
                 diagnostic above for the fix options"
                    .to_string(),
            )
        })?;
        let input_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                alloc_ptr,
                input_idx,
                "ParseError.input.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(input_field_ptr, input_str_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        Ok(alloc_ptr)
    }

    /// 2026-06-04 — analog of `complete_parse_fallible_call` for the
    /// crypto surface (`std::crypto::ecdsa_p256_sign` flipped to
    /// `fallible(CryptoError)` in `or` context). The err payload is
    /// `CryptoError { kind, detail }`. Caller provides the kind tag
    /// (e.g. "ecdsa_p256_sign") and a `detail` String pointer naming
    /// what failed.
    pub(crate) fn complete_crypto_fallible_call(
        &mut self,
        is_err: IntValue<'ctx>,
        detail_str_ptr: BasicValueEnum<'ctx>,
        kind_tag: &str,
        success_value: Option<(BasicValueEnum<'ctx>, CodegenTy)>,
        label: &str,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let payload_ty = CodegenTy::TypeRef("CryptoError".to_string());
        let out_err_slot = self.alloca_for(
            &payload_ty,
            &format!("{}.out_err.slot", label),
        )?;
        let (out_val_slot_opt, success_ty_opt) = match &success_value {
            Some((_, ty)) => (
                Some(self.alloca_for(
                    ty,
                    &format!("{}.out_val.slot", label),
                )?),
                Some(ty.clone()),
            ),
            None => (None, None),
        };

        let func = self
            .current_fn
            .expect("fallible-stdlib-path call inside fn body");
        let lazy_err_bb = self.context.append_basic_block(
            func,
            &format!("{}.lazy_err", label),
        );
        let store_ok_bb = self.context.append_basic_block(
            func,
            &format!("{}.store_ok", label),
        );
        let join_bb = self.context.append_basic_block(
            func,
            &format!("{}.join", label),
        );

        self.builder
            .build_conditional_branch(is_err, lazy_err_bb, store_ok_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // err path: build CryptoError, store in out_err_slot.
        self.builder.position_at_end(lazy_err_bb);
        let ce_ptr = self.emit_crypto_error_alloc(detail_str_ptr, kind_tag)?;
        self.builder
            .build_store(out_err_slot, ce_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // ok path: store the success value (if any).
        self.builder.position_at_end(store_ok_bb);
        if let (Some(out_val_slot), Some((val, _))) =
            (out_val_slot_opt, &success_value)
        {
            self.builder
                .build_store(out_val_slot, *val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.builder
            .build_unconditional_branch(join_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult {
            i1_path: is_err,
            out_val_slot: out_val_slot_opt,
            out_err_slot,
            success_ty: success_ty_opt,
            payload_ty,
        })
    }

    /// Allocate a `CryptoError { kind, detail }` in the current
    /// arena, populate, return pointer. Mirrors
    /// `emit_parse_error_alloc`; converts a user-declared
    /// `type CryptoError` missing the stdlib's expected `kind:
    /// String` / `detail: String` fields into a clean codegen-time
    /// diagnostic instead of a runtime panic.
    pub(crate) fn emit_crypto_error_alloc(
        &mut self,
        detail_str_ptr: BasicValueEnum<'ctx>,
        kind_tag: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let info = self
            .user_types
            .get("CryptoError")
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(
                    "CryptoError type missing — `declare_builtin_crypto_error_type` \
                     should have injected it; sequencing bug?".to_string(),
                )
            })?;
        let size = info
            .struct_ty
            .size_of()
            .expect("CryptoError has known size");
        let alloc_ptr = self.arena_alloc(size, "CryptoError.alloc")?;

        let kind_ptr = self.global_string(kind_tag);
        let (kind_idx, _) = info.fields.get("kind").cloned().ok_or_else(|| {
            CodegenError::Unsupported(
                "user-declared `type CryptoError` is missing the stdlib's \
                 expected `kind: String` field — std::crypto fns need to \
                 allocate `CryptoError { kind, detail }` on failure. Either \
                 match the stdlib shape (`type CryptoError { kind: String; \
                 detail: String; }`), rename your type (e.g. `MyCryptoError`), \
                 or use the `std::crypto::CryptoError` qualified path where you \
                 need the stdlib's"
                    .to_string(),
            )
        })?;
        let kind_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                alloc_ptr,
                kind_idx,
                "CryptoError.kind.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(kind_field_ptr, kind_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let (detail_idx, _) = info.fields.get("detail").cloned().ok_or_else(|| {
            CodegenError::Unsupported(
                "user-declared `type CryptoError` is missing the stdlib's \
                 expected `detail: String` field — see the `kind` field \
                 diagnostic above for the fix options"
                    .to_string(),
            )
        })?;
        let detail_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                alloc_ptr,
                detail_idx,
                "CryptoError.detail.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(detail_field_ptr, detail_str_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        Ok(alloc_ptr)
    }

    /// v1.x-FORM-2 PR6: lower the err branch of an `or raise`.
    /// Inside a fallible(E) fn: sret-copy the inner payload into
    /// the enclosing fn's err alloca, flip its path indicator to
    /// 1, branch to the enclosing exit. Otherwise (the implicit
    /// main locus's body or any non-fallible frame reachable
    /// from it): call `lotus_root_panic` and emit unreachable.
    /// Either way the err branch terminates — the caller's join
    /// block has only the ok branch as a predecessor.
    /// B3 / G6: `or fail X` shares the err-exit shape of
    /// `or raise` — only the source of the payload differs.
    /// Reject if the enclosing fn isn't fallible (the payload
    /// has no slot to land in), and reject if the payload type
    /// doesn't match the fn's declared error type.
    pub(crate) fn lower_or_fail(
        &mut self,
        payload: &Expr,
        _call: &FallibleCallResult<'ctx>,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let enclosing = self.current_user_fn_fallible.clone().ok_or_else(|| {
            CodegenError::Unsupported(
                "`or fail X` outside a fallible(E) fn (use `or raise` \
                 if you want to propagate the inner call's payload, or \
                 `or <fallback>` to substitute a value)".into(),
            )
        })?;

        // m67-style rewrite: bare-name struct literals in fail
        // position resolve against the declared payload type,
        // matching the Stmt::Fail and lower_return shapes.
        let payload_ty = enclosing.payload_ty.clone();
        let rewritten;
        let e_to_lower: &Expr = match payload {
            Expr::Struct { path, inits, span } => {
                match self.resolve_generic_struct_path_for_codegen_ty(
                    path,
                    &payload_ty,
                ) {
                    Some(new_path) => {
                        rewritten = Expr::Struct {
                            path: new_path,
                            inits: inits.clone(),
                            span: *span,
                        };
                        &rewritten
                    }
                    None => payload,
                }
            }
            _ => payload,
        };

        let (v, got_ty) = self.lower_expr(e_to_lower, scope)?;
        if got_ty != payload_ty {
            return Err(CodegenError::Unsupported(format!(
                "`or fail` payload type mismatch: enclosing fn declared \
                 fallible({:?}), got {:?}",
                payload_ty, got_ty
            )));
        }
        self.builder
            .build_store(enclosing.err_alloca, v)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let bool_t = self.context.bool_type();
        self.builder
            .build_store(enclosing.path_alloca, bool_t.const_int(1, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let exit_bb = self
            .current_user_fn_exit_bb
            .expect("exit_bb set inside a fn body");
        self.builder
            .build_unconditional_branch(exit_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    pub(crate) fn lower_or_raise(
        &mut self,
        call: &FallibleCallResult<'ctx>,
    ) -> Result<(), CodegenError> {
        match self.current_user_fn_fallible.clone() {
            Some(enclosing) => {
                if call.payload_ty != enclosing.payload_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "`or raise` payload type mismatch: inner is {:?}, \
                         enclosing fn declared fallible({:?})",
                        call.payload_ty, enclosing.payload_ty
                    )));
                }
                let llvm_err_ty = self.llvm_basic_type(&call.payload_ty);
                let v = self
                    .builder
                    .build_load(
                        llvm_err_ty,
                        call.out_err_slot,
                        "or.raise.payload.load",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(enclosing.err_alloca, v)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let bool_t = self.context.bool_type();
                self.builder
                    .build_store(
                        enclosing.path_alloca,
                        bool_t.const_int(1, false),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let exit_bb = self
                    .current_user_fn_exit_bb
                    .expect("exit_bb set inside a fn body");
                self.builder
                    .build_unconditional_branch(exit_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            None => {
                // Implicit-main-locus escape: value error past every
                // fallible frame → panic via the runtime fn. The
                // typename arg is the discriminator a future
                // routing-through-main-locus-on_failure extension
                // would key on; today the runtime fn just dprintf
                // + exit(1).
                let typename_ptr =
                    self.payload_typename_global(&call.payload_ty);
                let llvm_err_ty = self.llvm_basic_type(&call.payload_ty);
                let payload_size = llvm_err_ty
                    .size_of()
                    .expect("payload type has known size");
                let root_panic_fn = self
                    .module
                    .get_function("lotus_root_panic")
                    .expect("lotus_root_panic declared in declare_builtins");
                self.builder
                    .build_call(
                        root_panic_fn,
                        &[
                            call.out_err_slot.into(),
                            payload_size.into(),
                            typename_ptr.into(),
                        ],
                        "or.raise.root_panic",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_unreachable()
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// v1.x-FORM-2 PR6 (PR5 finale): lower a fallible method call
    /// inside `Expr::Or` — `l.get(i) or ...` or `self.pop() or
    /// ...`. Resolves the receiver to a locus + self_ptr, then
    /// dispatches to `try_lower_form_vec_fallible_method` for
    /// the synthesized @form(vec) get/pop. User-declared
    /// fallible locus methods are not yet wired (see PR6 commit
    /// 1's note about declare_locus_struct).
    pub(crate) fn lower_fallible_method_call(
        &mut self,
        receiver: &Expr,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let (info, self_ptr, locus_name) =
            if matches!(receiver, Expr::KwSelf(_)) {
                let cs = self.current_self.as_ref().cloned().ok_or_else(
                    || {
                        CodegenError::Unsupported(
                            "`self.{method}` fallible call outside a locus \
                             method"
                                .into(),
                        )
                    },
                )?;
                let info = self
                    .user_loci
                    .get(&cs.locus_name)
                    .cloned()
                    .expect("current_self points to a declared locus");
                let locus_name = cs.locus_name.clone();
                (info, cs.self_ptr, locus_name)
            } else {
                let (recv_val, recv_ty) =
                    self.lower_expr(receiver, scope)?;
                let locus_name = match recv_ty {
                    CodegenTy::LocusRef(n) => n,
                    other => {
                        return Err(CodegenError::Unsupported(format!(
                            "fallible method call on non-locus value of \
                             type {:?}",
                            other
                        )));
                    }
                };
                let info = self
                    .user_loci
                    .get(&locus_name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "fallible method call: unknown locus `{}`",
                            locus_name
                        ))
                    })?;
                (info, recv_val.into_pointer_value(), locus_name)
            };

        // BCE: the receiver's canonical vec key — `Local(name)` for a
        // local vec instance (`v.get(i)`, the common `@form(vec)`
        // shape), `SelfLocus` for `self.get(i)`, or `SelfField(f)` for
        // `self.data.get(i)`. Matched against the enclosing-loop
        // registry in the vec `.get` arm; `None` for deeper receiver
        // shapes, which never BCE.
        let recv_bce_key = bce_receiver_key(receiver);
        if let Some(result) = self
            .try_lower_form_vec_fallible_method(
                &info,
                self_ptr,
                &locus_name,
                method_name,
                args,
                scope,
                recv_bce_key,
            )?
        {
            return Ok(result);
        }
        if let Some(result) = self
            .try_lower_form_hashmap_fallible_method(
                &info,
                self_ptr,
                &locus_name,
                method_name,
                args,
                scope,
            )?
        {
            return Ok(result);
        }
        if let Some(result) = self
            .try_lower_form_ring_buffer_fallible_method(
                &info,
                self_ptr,
                &locus_name,
                method_name,
                args,
                scope,
            )?
        {
            return Ok(result);
        }
        // Open-question #24 MVP (2026-05-25): user-declared
        // fallible locus member fns. The declare-time path
        // emitted the LLVM fn with the fallible ABI (i1 ret +
        // sret slots); we look it up here, build the call with
        // the same shape, and return a FallibleCallResult so
        // the surrounding `or` machinery dispatches the same
        // way it does for free-fn fallibles.
        if let Some(result) = self.try_lower_user_locus_fallible_method(
            &info,
            self_ptr,
            &locus_name,
            method_name,
            args,
            scope,
        )? {
            return Ok(result);
        }
        Err(CodegenError::Unsupported(format!(
            "fallible method `{}.{}` — not a synthesized @form(vec) \
             get/pop, @form(hashmap) get/remove/key_at/entry_at, \
             @form(ring_buffer) pop, or a user-declared `fn` member \
             with `fallible(E)`",
            locus_name, method_name
        )))
    }

    /// Allocate an `IoError` struct in the current arena and
    /// populate its three fields (kind / errno / path). Used by
    /// every fallible stdlib I/O wrapper (`std::io::fs::*`,
    /// `std::io::tcp::*`). The errno is fetched via the runtime
    /// helper `lotus_get_errno` and the kind tag via
    /// `lotus_io_error_kind(errno)` — both immediately after the
    /// failing primitive call (POSIX errno is sticky until the
    /// next syscall sets it).
    pub(crate) fn emit_io_error_alloc(
        &mut self,
        path_str_ptr: BasicValueEnum<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let info = self
            .user_types
            .get("IoError")
            .cloned()
            .expect("IoError injected by hale-types resolver");
        let size = info
            .struct_ty
            .size_of()
            .expect("IoError has known size");
        let alloc_ptr = self.arena_alloc(size, "IoError.alloc")?;

        // Fetch errno + kind from the runtime.
        let get_errno_fn = self
            .module
            .get_function("lotus_get_errno")
            .expect("lotus_get_errno declared");
        let errno_i32 = self
            .builder
            .build_call(get_errno_fn, &[], "ioerr.errno.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_get_errno returns i32")
            .into_int_value();
        let i64_t = self.context.i64_type();
        let errno_i64 = self
            .builder
            .build_int_s_extend(errno_i32, i64_t, "ioerr.errno.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let kind_fn = self
            .module
            .get_function("lotus_io_error_kind")
            .expect("lotus_io_error_kind declared");
        let kind_ptr = self
            .builder
            .build_call(kind_fn, &[errno_i32.into()], "ioerr.kind.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_io_error_kind returns ptr");

        // Store fields. Order matches inject_form_stdlib_types:
        // kind (0), errno (1), path (2).
        let (kind_idx, _) = info
            .fields
            .get("kind")
            .cloned()
            .expect("IoError.kind field");
        let kind_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                alloc_ptr,
                kind_idx,
                "IoError.kind.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(kind_field_ptr, kind_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let (errno_idx, _) = info
            .fields
            .get("errno")
            .cloned()
            .expect("IoError.errno field");
        let errno_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                alloc_ptr,
                errno_idx,
                "IoError.errno.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(errno_field_ptr, errno_i64)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let (path_idx, _) = info
            .fields
            .get("path")
            .cloned()
            .expect("IoError.path field");
        let path_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                alloc_ptr,
                path_idx,
                "IoError.path.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(path_field_ptr, path_str_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        Ok(alloc_ptr)
    }

    /// v1.x-FORM-2 PR6: lower `fail <expr>;` inside a fallible(E)
    /// fn. Symmetric to `return e;` but writes the payload into
    /// the local err stage + flips the path indicator to i1 1,
    /// then branches to the unified fn.exit block. The fail
    /// branch of the epilogue deep-copies the payload into the
    /// caller-provided out_err sret slot.
    pub(crate) fn lower_fail(
        &mut self,
        value: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let fallible = self
            .current_user_fn_fallible
            .clone()
            .ok_or_else(|| {
                CodegenError::Unsupported(
                    "`fail` outside a fallible(E) fn".into(),
                )
            })?;
        let exit_bb = self.current_user_fn_exit_bb.ok_or_else(|| {
            CodegenError::Unsupported(
                "`fail` outside a free-fn body".into(),
            )
        })?;

        // m67-style rewrite: a bare-name struct literal in fail
        // position resolves against the declared payload type, so
        // `fail Foo { ... }` lowers as `fail Foo_T_U { ... }` when
        // the payload is `Foo<T, U>`. Mirrors the lower_return shape.
        let rewritten;
        let payload_ty = fallible.payload_ty.clone();
        let e_to_lower: &Expr = match value {
            Expr::Struct { path, inits, span } => {
                match self.resolve_generic_struct_path_for_codegen_ty(
                    path,
                    &payload_ty,
                ) {
                    Some(new_path) => {
                        rewritten = Expr::Struct {
                            path: new_path,
                            inits: inits.clone(),
                            span: *span,
                        };
                        &rewritten
                    }
                    None => value,
                }
            }
            _ => value,
        };

        let (v, got_ty) = self.lower_expr(e_to_lower, scope)?;
        if got_ty != payload_ty {
            return Err(CodegenError::Unsupported(format!(
                "`fail` payload type mismatch: fallible declared {:?}, \
                 got {:?}",
                payload_ty, got_ty
            )));
        }

        self.builder
            .build_store(fallible.err_alloca, v)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let bool_t = self.context.bool_type();
        self.builder
            .build_store(fallible.path_alloca, bool_t.const_int(1, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder
            .build_unconditional_branch(exit_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        Ok(BlockEnd::Terminated)
    }

}
