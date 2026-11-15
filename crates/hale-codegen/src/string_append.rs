//! String building in linear time and memory (GH #1947).
//!
//! Two lowerings, both over the shapes `hale_types::string_append`
//! names:
//!
//! - **A `+` chain is one allocation.** `a + b + c + d` lowers to one
//!   `lotus_str_concat_n` over its operands instead of a concatenation
//!   per `+`, each of which the arena kept until the frame ended. A
//!   chain of two is still the one `lotus_str_concat` it was.
//! - **An appending assignment appends.** `s = s + a + b` (or `s += a`,
//!   or `s = f"{s}…"`) on a local declared `let mut` whose block appends
//!   to it is `lotus_str_append` into a growable buffer the local owns
//!   (an accumulator beside its slot, `lotus_str_acc_t`); on a String
//!   field of `self`, in a method with a scratch, it is
//!   `lotus_str_field_append` into the field's block. Any other place
//!   (`rec.body`, an element) builds the chain into a fresh String, one
//!   allocation per statement.
//!
//! Value semantics: a local's buffer is its own only until a read of the
//! local that may keep the value. Such a read clears the accumulator
//! (`emit_str_acc_release`, at the read, when it runs), so the next
//! append copies into a fresh buffer and the value read keeps what it
//! read. The reads that keep nothing are lowered without clearing it
//! ([`Cx::lower_operand_borrowed`], or `Cx::borrowed_args` for an
//! argument): an operand of a `+` chain (the chain copies it), an operand
//! of a comparison, the receiver of a slice (it copies), an argument of a
//! call that keeps none (`hale_types::string_append::
//! call_keeps_no_argument`, the rule `hale verify`'s finding reads too),
//! and the place an appending assignment reads. Every other path that
//! loads a String local goes through `lower_expr`'s identifier arm,
//! which clears. A plain store to the local, and its `let`, clear it too.

use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use hale_syntax::ast::*;
use hale_types::string_append::{append_place, appended_parts, appends_to_local, AppendPlace};

use crate::codegen::{BlockEnd, CodegenError, CodegenTy, Cx, Scope};

/// A `+` tree as lowered: the operands of a String chain, not yet
/// joined, or the value of a `+` that is not a String's.
pub(crate) enum AddTree<'ctx> {
    Str(Vec<BasicValueEnum<'ctx>>),
    Val(BasicValueEnum<'ctx>, CodegenTy),
}

/// The most operands one runtime call takes (the runtime measures that
/// many on its stack); a longer chain joins its tail first.
const STR_PARTS_MAX: usize = 16;

fn llvm_err(e: inkwell::builder::BuilderError) -> CodegenError {
    CodegenError::LlvmEmit(e.to_string())
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// `e` lowered as an operand that is read and not kept: an
    /// identifier naming a local with an accumulator is loaded without
    /// releasing its buffer; anything else is `lower_expr`.
    pub(crate) fn lower_operand_borrowed(
        &mut self,
        e: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if let Expr::Ident(id) = e {
            if let Some((slot, ty)) = scope.locals.get(&id.name).cloned() {
                if self.str_accs.contains_key(&slot) {
                    let llvm_ty = self.llvm_basic_type(&ty);
                    let v = self.builder.build_load(llvm_ty, slot, &id.name).map_err(llvm_err)?;
                    return Ok((v, ty));
                }
            }
        }
        self.lower_expr(e, scope)
    }

    /// The accumulator type, `lotus_str_acc_t`: `{ ptr buf, i64 len, i64
    /// cap, ptr arena }`.
    fn str_acc_type(&self) -> BasicTypeEnum<'ctx> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        self.context.struct_type(&[ptr_t.into(), i64_t.into(), i64_t.into(), ptr_t.into()], false).into()
    }

    /// Give up the buffer the accumulator holds: the next append copies.
    pub(crate) fn emit_str_acc_release(&mut self, acc: PointerValue<'ctx>) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let acc_ty = self.str_acc_type().into_struct_type();
        let buf = self.builder.build_struct_gep(acc_ty, acc, 0, "str.acc.buf").map_err(llvm_err)?;
        self.builder.build_store(buf, ptr_t.const_null()).map_err(llvm_err)?;
        Ok(())
    }

    /// `block.stmts[i]` just lowered: a `let mut` of a String whose later
    /// statements append to it gets an accumulator (once per slot), and
    /// the `let` clears it, since the value it binds is not a buffer the
    /// local owns.
    pub(crate) fn note_string_accumulator(
        &mut self,
        block: &Block,
        i: usize,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let Stmt::Let { is_mut: true, name, .. } = &block.stmts[i] else { return Ok(()) };
        let Some((slot, CodegenTy::String)) = scope.locals.get(&name.name).cloned() else { return Ok(()) };
        if self.current_fn.is_none() || !appends_to_local(&block.stmts[i + 1..], block.tail.as_deref(), &name.name) {
            return Ok(());
        }
        let acc = match self.str_accs.get(&slot) {
            Some(a) => *a,
            None => {
                let ty = self.str_acc_type();
                let a = self.alloca_in_entry(ty, &format!("{}.str.acc", name.name))?;
                self.str_accs.insert(slot, a);
                a
            }
        };
        self.emit_str_acc_release(acc)
    }

    /// `e` lowered as a `+` tree: a String chain's operands are lowered
    /// left to right and kept apart ([`AddTree::Str`]), a non-String side
    /// rendered as `+` renders it; a `+` of two values that are not
    /// Strings is the binary operator's value.
    pub(crate) fn lower_add_tree(&mut self, e: &Expr, scope: &Scope<'ctx>) -> Result<AddTree<'ctx>, CodegenError> {
        let Expr::Binary { op: BinOp::Add, left, right, span } = e else {
            let (v, t) = self.lower_operand_borrowed(e, scope)?;
            return Ok(AddTree::Val(v, t));
        };
        let l = self.lower_add_tree(left, scope)?;
        let r = self.lower_add_tree(right, scope)?;
        let is_str = |t: &AddTree<'ctx>| matches!(t, AddTree::Str(_) | AddTree::Val(_, CodegenTy::String));
        if is_str(&l) || is_str(&r) {
            let mut parts = self.add_tree_parts(l, left)?;
            parts.extend(self.add_tree_parts(r, right)?);
            return Ok(AddTree::Str(parts));
        }
        let (AddTree::Val(lv, lt), AddTree::Val(rv, rt)) = (l, r) else { unreachable!("neither side is a String chain") };
        let (v, t) = self.lower_binary_values(BinOp::Add, lv, lt, rv, rt, left, right, *span)?;
        Ok(AddTree::Val(v, t))
    }

    /// The String operands one side of a chain contributes: a chain's,
    /// a String, or a printable value rendered (`"n=" + n`).
    fn add_tree_parts(&mut self, t: AddTree<'ctx>, e: &Expr) -> Result<Vec<BasicValueEnum<'ctx>>, CodegenError> {
        match t {
            AddTree::Str(parts) => Ok(parts),
            AddTree::Val(v, CodegenTy::String) => Ok(vec![v]),
            AddTree::Val(v, ty) if self.value_to_string_supports(&ty) => {
                let s = self.value_to_string(v, &ty)?;
                Ok(vec![self.with_printed_unit(e, s)?])
            }
            AddTree::Val(_, ty) => Err(CodegenError::Unsupported(format!(
                "binary op operands of mixed types {} and {}",
                CodegenTy::String,
                ty
            ))),
        }
    }

    /// The runtime's `parts` argument: `parts` stored into the fn's one
    /// `[STR_PARTS_MAX x ptr]` (an entry-block alloca, shared by every
    /// call site of the fn: each fills it after its operands are
    /// lowered and before its call, so no two uses overlap). At most
    /// [`STR_PARTS_MAX`] parts; [`Self::fit_parts`] makes them so.
    fn str_parts_array(&mut self, parts: &[BasicValueEnum<'ctx>]) -> Result<PointerValue<'ctx>, CodegenError> {
        debug_assert!(parts.len() <= STR_PARTS_MAX);
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arr_t = ptr_t.array_type(STR_PARTS_MAX as u32);
        let func = self.current_fn.expect("a String chain is lowered inside a fn");
        let arr = match self.str_parts_arrays.get(&func) {
            Some(a) => *a,
            None => {
                let a = self.alloca_in_entry(arr_t.into(), "str.parts")?;
                self.str_parts_arrays.insert(func, a);
                a
            }
        };
        let i32_t = self.context.i32_type();
        for (i, p) in parts.iter().enumerate() {
            let slot = unsafe {
                self.builder
                    .build_gep(arr_t, arr, &[i32_t.const_zero(), i32_t.const_int(i as u64, false)], "str.part")
                    .map_err(llvm_err)?
            };
            self.builder.build_store(slot, p.into_pointer_value()).map_err(llvm_err)?;
        }
        Ok(arr)
    }

    /// `parts`, at most [`STR_PARTS_MAX`] of them: the operands past the
    /// limit joined into one String first.
    fn fit_parts(&mut self, mut parts: Vec<BasicValueEnum<'ctx>>) -> Result<Vec<BasicValueEnum<'ctx>>, CodegenError> {
        if parts.len() > STR_PARTS_MAX {
            let tail = parts.split_off(STR_PARTS_MAX - 1);
            let joined = self.concat_parts(tail)?;
            parts.push(joined);
        }
        Ok(parts)
    }

    /// One String of `parts`, joined in the current arena: the one
    /// concatenation for two, one `lotus_str_concat_n` for more.
    pub(crate) fn concat_parts(&mut self, parts: Vec<BasicValueEnum<'ctx>>) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match parts.len() {
            0 => Ok(self.global_string("").into()),
            1 => Ok(parts[0]),
            2 => self.str_concat(parts[0], parts[1]),
            _ => {
                let parts = self.fit_parts(parts)?;
                let n = parts.len();
                let arena = self.current_arena_ptr()?;
                let arr = self.str_parts_array(&parts)?;
                let f = self.module.get_function("lotus_str_concat_n").expect("lotus_str_concat_n declared");
                let n = self.context.i64_type().const_int(n as u64, false);
                let v = self
                    .builder
                    .build_call(f, &[arena.into(), arr.into(), n.into()], "str.concat_n")
                    .map_err(llvm_err)?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_concat_n returns ptr");
                Ok(v)
            }
        }
    }

    /// The String operands of the appended parts, left to right.
    fn lower_appended(&mut self, parts: &[&Expr], scope: &Scope<'ctx>) -> Result<Vec<BasicValueEnum<'ctx>>, CodegenError> {
        let mut out = Vec::new();
        for p in parts {
            let t = self.lower_add_tree(p, scope)?;
            out.extend(self.add_tree_parts(t, p)?);
        }
        Ok(out)
    }

    /// An appending assignment to a String place, lowered as an append
    /// when the place is a local or a field of `self` (module docs).
    /// `None`: not one, and the caller lowers the assignment as written.
    /// `slot` is the place's slot as the assignment resolved it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_lower_string_append(
        &mut self,
        target: &LValue,
        op: &AssignOp,
        value: &Expr,
        slot: PointerValue<'ctx>,
        slot_ty: &CodegenTy,
        assign_id: NodeId,
        scope: &Scope<'ctx>,
    ) -> Result<Option<BlockEnd>, CodegenError> {
        if *slot_ty != CodegenTy::String || self.current_fn.is_none() {
            return Ok(None);
        }
        let Some(parts) = appended_parts(target, op, value) else { return Ok(None) };
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        // `s += e` evaluates `e` before it reads `s`, as the compound
        // assignment always has; `s = s + e` reads `s` first.
        let reads_first = matches!(op, AssignOp::Eq);
        match append_place(target) {
            AppendPlace::Local(name) => {
                if self.owned_views.node(assign_id) {
                    return Ok(None);
                }
                // A part that assigns the local (inside a block value)
                // would change the buffer between the read and the
                // append: no accumulator for this statement.
                let acc = self
                    .str_accs
                    .get(&slot)
                    .copied()
                    .filter(|_| !parts.iter().any(|p| crate::codegen::expr_assigns(p, name)));
                let (cur, vals) = if reads_first {
                    let cur = self.builder.build_load(ptr_t, slot, name).map_err(llvm_err)?;
                    (cur, self.lower_appended(&parts, scope)?)
                } else {
                    let vals = self.lower_appended(&parts, scope)?;
                    (self.builder.build_load(ptr_t, slot, name).map_err(llvm_err)?, vals)
                };
                let new_val = match acc {
                    Some(acc) => {
                        let vals = self.fit_parts(vals)?;
                        let arena = self.current_arena_ptr()?;
                        let arr = self.str_parts_array(&vals)?;
                        let f = self.module.get_function("lotus_str_append").expect("lotus_str_append declared");
                        let n = self.context.i64_type().const_int(vals.len() as u64, false);
                        self.builder
                            .build_call(f, &[arena.into(), acc.into(), cur.into(), arr.into(), n.into()], "str.append")
                            .map_err(llvm_err)?
                            .try_as_basic_value()
                            .left()
                            .expect("lotus_str_append returns ptr")
                    }
                    None => {
                        let mut all = vec![cur];
                        all.extend(vals);
                        self.concat_parts(all)?
                    }
                };
                self.builder.build_store(slot, new_val).map_err(llvm_err)?;
                Ok(Some(BlockEnd::Open))
            }
            AppendPlace::SelfField(_) => {
                // The field append writes into `self.__arena`, which only a
                // method with a scratch owns as the destination (as
                // `emit_self_field_inplace_assign`).
                let (Some(_), Some(cs)) = (self.current_method_scratch, self.current_self.clone()) else {
                    return Ok(None);
                };
                let info = self.user_loci.get(&cs.locus_name).cloned().expect("current_self names a declared locus");
                let arena_field = self
                    .builder
                    .build_struct_gep(info.struct_ty, cs.self_ptr, info.arena_field_idx, "self.__arena.for_append_ptr")
                    .map_err(llvm_err)?;
                let arena = self.builder.build_load(ptr_t, arena_field, "self.__arena.for_append").map_err(llvm_err)?;
                let len_fn = self.module.get_function("lotus_str_field_len").expect("lotus_str_field_len declared");
                let read = |cx: &mut Self| -> Result<(BasicValueEnum<'ctx>, BasicValueEnum<'ctx>), CodegenError> {
                    let cur = cx.builder.build_load(ptr_t, slot, "self_field.append.cur").map_err(llvm_err)?;
                    let len = cx
                        .builder
                        .build_call(len_fn, &[arena.into(), cur.into()], "self_field.append.len")
                        .map_err(llvm_err)?
                        .try_as_basic_value()
                        .left()
                        .expect("lotus_str_field_len returns i64");
                    Ok((cur, len))
                };
                let ((cur, cur_len), vals) = if reads_first {
                    let c = read(self)?;
                    (c, self.lower_appended(&parts, scope)?)
                } else {
                    let vals = self.lower_appended(&parts, scope)?;
                    (read(self)?, vals)
                };
                let vals = self.fit_parts(vals)?;
                let now = self.builder.build_load(ptr_t, slot, "self_field.append.now").map_err(llvm_err)?;
                let arr = self.str_parts_array(&vals)?;
                let f = self.module.get_function("lotus_str_field_append").expect("lotus_str_field_append declared");
                let n = self.context.i64_type().const_int(vals.len() as u64, false);
                let out = self
                    .builder
                    .build_call(
                        f,
                        &[arena.into(), cur.into(), cur_len.into(), now.into(), arr.into(), n.into()],
                        "self_field.append",
                    )
                    .map_err(llvm_err)?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_field_append returns ptr");
                self.builder.build_store(slot, out).map_err(llvm_err)?;
                Ok(Some(BlockEnd::Open))
            }
            AppendPlace::Other => Ok(None),
        }
    }
}
