//! GH #2132 (F.64): the fixed-width integers narrower than 64 bits,
//! `I32`, `U32`, `I16`, `U16`, `I8` and `U8` ([`CodegenTy::Narrow`]).
//!
//! A value is an LLVM `i32`, `i16` or `i8`: its declared width in a
//! record, a `bounded` element, a locus field, a topic payload and a
//! local, so a loop over them works lanes of that width. The operators
//! are checked as a `U64`'s are (GH #2121): an overflow
//! (`llvm.{s,u}{add,sub,mul}.with.overflow.iN`), a division or remainder
//! by zero, a signed division of the minimum by `-1`, a negation of the
//! minimum and a shift by a count outside `0` through the width less one
//! branch to `lotus_fixed_violation`, which prints the violation and
//! exits, so no value wraps unless the program asked (`T::wrapping_*`).
//! A widening into a wider integer is a sign or zero extension, at the
//! places the check let it flow ([`Cx::fit_int`]).

use hale_syntax::ast::{BinOp, Expr, PrimType};
use hale_types::stdlib_surface::{FixedIntOp, IntrinsicId};
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate as IP;

use crate::codegen::{CodegenError, CodegenTy, Cx, FallibleCallResult, Scope};

fn emit<T>(r: Result<T, inkwell::builder::BuilderError>) -> Result<T, CodegenError> {
    r.map_err(|e| CodegenError::LlvmEmit(e.to_string()))
}

/// What `lotus_fixed_violation` says happened (its `what` argument).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Violation {
    Add = 0,
    Sub = 1,
    Mul = 2,
    DivByZero = 3,
    RemByZero = 4,
    DivOverflow = 5,
    ShiftCount = 6,
    Neg = 7,
}

/// The primitive a machine-integer representation is (`Int`, `U64`, a
/// narrower one); `None` for every other.
pub(crate) fn int_prim(t: &CodegenTy) -> Option<PrimType> {
    match t {
        CodegenTy::Int => Some(PrimType::Int),
        CodegenTy::U64 => Some(PrimType::U64),
        CodegenTy::Narrow(p) => Some(*p),
        _ => None,
    }
}

/// GH #2121, #2132: whether a value lowered as `got` stands in a place
/// declared `want` once [`Cx::fit_int`] has fitted it: an integer
/// literal the check typed as the place's fixed-width integer where
/// lowering read no row of it (an `Int`, from a default lowered from its
/// constant or a monomorph's walk), or a lossless widening the check let
/// flow (`PrimType::widens_to`).
pub(crate) fn int_fits(got: &CodegenTy, want: &CodegenTy) -> bool {
    match (got, want) {
        (CodegenTy::Int, CodegenTy::U64 | CodegenTy::Narrow(_)) => true,
        _ => match (int_prim(got), int_prim(want)) {
            (Some(g), Some(w)) => g.widens_to(w),
            _ => false,
        },
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// `v`, a value of `got`, as a value of `want` where [`int_fits`] says
    /// it stands there: truncated (a literal the check held to `want`'s
    /// range), or sign- or zero-extended (a widening). Every other pair is
    /// `v` itself, for the caller's own rule to judge.
    pub(crate) fn fit_int(
        &mut self,
        v: BasicValueEnum<'ctx>,
        got: &CodegenTy,
        want: &CodegenTy,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if got == want || !int_fits(got, want) || !v.is_int_value() {
            return Ok(v);
        }
        let (Some(g), Some(w)) = (int_prim(got), int_prim(want)) else { return Ok(v) };
        Ok(self.resize_int(v.into_int_value(), g, w)?.into())
    }

    /// `v`, a value of the machine integer `from`, at `to`'s width: a
    /// truncation into a narrower one, an extension (signed as `from`
    /// is) into a wider one.
    pub(crate) fn resize_int(&mut self, v: IntValue<'ctx>, from: PrimType, to: PrimType) -> Result<IntValue<'ctx>, CodegenError> {
        let (fb, fs) = from.machine_int().expect("a machine integer");
        let (tb, _) = to.machine_int().expect("a machine integer");
        let target = self.context.custom_width_int_type(tb);
        Ok(if tb < fb {
            emit(self.builder.build_int_truncate(v, target, "fixed.trunc"))?
        } else if tb > fb {
            if fs {
                emit(self.builder.build_int_s_extend(v, target, "fixed.sext"))?
            } else {
                emit(self.builder.build_int_z_extend(v, target, "fixed.zext"))?
            }
        } else {
            v
        })
    }

    /// A narrower integer as the `Int` that holds it (to print it, to
    /// hand it to `Int(x)`).
    pub(crate) fn narrow_to_i64(&mut self, v: IntValue<'ctx>, p: PrimType) -> Result<IntValue<'ctx>, CodegenError> {
        self.resize_int(v, p, PrimType::Int)
    }

    /// Branch to the violation when `bad` holds; continue after it
    /// otherwise.
    pub(crate) fn fixed_violation_if(&mut self, bad: IntValue<'ctx>, what: Violation, p: PrimType) -> Result<(), CodegenError> {
        let func = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .ok_or_else(|| CodegenError::LlvmEmit("a fixed-width operator outside a function".into()))?;
        let fail = self.context.append_basic_block(func, "fixed.violation");
        let ok = self.context.append_basic_block(func, "fixed.ok");
        emit(self.builder.build_conditional_branch(bad, fail, ok))?;
        self.builder.position_at_end(fail);
        let f = self.module.get_function("lotus_fixed_violation").expect("lotus_fixed_violation declared");
        let (bits, signed) = p.machine_int().expect("a machine integer");
        let i32_t = self.context.i32_type();
        emit(self.builder.build_call(
            f,
            &[
                i32_t.const_int(what as u64, false).into(),
                i32_t.const_int(u64::from(bits), false).into(),
                i32_t.const_int(u64::from(signed), false).into(),
            ],
            "",
        ))?;
        emit(self.builder.build_unreachable())?;
        self.builder.position_at_end(ok);
        Ok(())
    }

    /// `llvm.{s,u}{add,sub,mul}.with.overflow.iN`: the wrapped result and
    /// whether it wrapped.
    fn fixed_with_overflow(
        &mut self,
        op: &str,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
        signed: bool,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let name = format!("llvm.{}{op}.with.overflow", if signed { "s" } else { "u" });
        let intrinsic = inkwell::intrinsics::Intrinsic::find(&name)
            .ok_or_else(|| CodegenError::LlvmEmit(format!("{name} not found")))?;
        let f = intrinsic
            .get_declaration(&self.module, &[l.get_type().into()])
            .ok_or_else(|| CodegenError::LlvmEmit(format!("{name} not declared")))?;
        let pair = emit(self.builder.build_call(f, &[l.into(), r.into()], "fixed.ovf"))?
            .try_as_basic_value()
            .left()
            .expect("an overflow intrinsic returns a pair")
            .into_struct_value();
        let v = emit(self.builder.build_extract_value(pair, 0, "fixed.ovf.v"))?.into_int_value();
        let o = emit(self.builder.build_extract_value(pair, 1, "fixed.ovf.o"))?.into_int_value();
        Ok((v, o))
    }

    /// A binary operator over two values of the narrower integer `p`
    /// (the checker refuses any other mix; an integer literal beside one
    /// is one, and a shift is [`Self::lower_narrow_shift`]).
    pub(crate) fn lower_narrow_binop(
        &mut self,
        op: BinOp,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let p = int_prim(ty).expect("a narrower integer");
        let (bits, signed) = p.machine_int().expect("a machine integer");
        let int_t = self.context.custom_width_int_type(bits);
        // an operand lowered as an `Int` where its row was not read is an
        // integer literal the check held to `p`'s range
        let l = if l.get_type().get_bit_width() != bits { emit(self.builder.build_int_truncate(l, int_t, "fixed.lit"))? } else { l };
        let r = if r.get_type().get_bit_width() != bits && !matches!(op, BinOp::Shl | BinOp::Shr) {
            emit(self.builder.build_int_truncate(r, int_t, "fixed.lit"))?
        } else {
            r
        };
        let zero = int_t.const_zero();
        let v: IntValue<'ctx> = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let (name, what) = match op {
                    BinOp::Add => ("add", Violation::Add),
                    BinOp::Sub => ("sub", Violation::Sub),
                    _ => ("mul", Violation::Mul),
                };
                let (v, o) = self.fixed_with_overflow(name, l, r, signed)?;
                self.fixed_violation_if(o, what, p)?;
                v
            }
            BinOp::Div | BinOp::Mod => {
                let by_zero = emit(self.builder.build_int_compare(IP::EQ, r, zero, "fixed.by_zero"))?;
                self.fixed_violation_if(by_zero, if op == BinOp::Div { Violation::DivByZero } else { Violation::RemByZero }, p)?;
                if signed {
                    // the minimum over `-1` is one past the maximum (and its
                    // remainder traps in LLVM): a violation for `/`, zero
                    // for `%`, which is what the remainder is
                    let min = int_t.const_int(1u64 << (bits - 1), false);
                    let is_min = emit(self.builder.build_int_compare(IP::EQ, l, min, "fixed.is_min"))?;
                    let by_neg1 = emit(self.builder.build_int_compare(IP::EQ, r, int_t.const_all_ones(), "fixed.by_neg1"))?;
                    let over = emit(self.builder.build_and(is_min, by_neg1, "fixed.div_over"))?;
                    if op == BinOp::Div {
                        self.fixed_violation_if(over, Violation::DivOverflow, p)?;
                        emit(self.builder.build_int_signed_div(l, r, "fixed.sdiv"))?
                    } else {
                        let one = int_t.const_int(1, false);
                        let safe_r = emit(self.builder.build_select(over, one, r, "fixed.srem.r"))?.into_int_value();
                        emit(self.builder.build_int_signed_rem(l, safe_r, "fixed.srem"))?
                    }
                } else if op == BinOp::Div {
                    emit(self.builder.build_int_unsigned_div(l, r, "fixed.udiv"))?
                } else {
                    emit(self.builder.build_int_unsigned_rem(l, r, "fixed.urem"))?
                }
            }
            BinOp::BitAnd => emit(self.builder.build_and(l, r, "fixed.and"))?,
            BinOp::BitOr => emit(self.builder.build_or(l, r, "fixed.or"))?,
            BinOp::BitXor => emit(self.builder.build_xor(l, r, "fixed.xor"))?,
            BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                let pred = match (op, signed) {
                    (BinOp::Eq, _) => IP::EQ,
                    (BinOp::NotEq, _) => IP::NE,
                    (BinOp::Lt, true) => IP::SLT,
                    (BinOp::Gt, true) => IP::SGT,
                    (BinOp::LtEq, true) => IP::SLE,
                    (_, true) => IP::SGE,
                    (BinOp::Lt, false) => IP::ULT,
                    (BinOp::Gt, false) => IP::UGT,
                    (BinOp::LtEq, false) => IP::ULE,
                    (_, false) => IP::UGE,
                };
                let c = emit(self.builder.build_int_compare(pred, l, r, "fixed.cmp"))?;
                return Ok((c.into(), CodegenTy::Bool));
            }
            BinOp::Shl | BinOp::Shr => return self.lower_narrow_shift(op, l, r, ty),
            _ => {
                return Err(CodegenError::Unsupported(format!(
                    "a `{}` operator the checker admits and lowering does not know",
                    hale_types::ty::prim_name(p)
                )));
            }
        };
        Ok((v.into(), ty.clone()))
    }

    /// `x << n` / `x >> n`, the count an `Int`: bits shifted out are
    /// dropped, and `>>` fills with the sign bit of a signed integer
    /// (`ashr`) and with zeros of an unsigned one (`lshr`). A count outside
    /// 0 through the width less one is a violation (`T::wrapping_shl` /
    /// `wrapping_shr` take it modulo the width), never LLVM's poison.
    pub(crate) fn lower_narrow_shift(
        &mut self,
        op: BinOp,
        l: IntValue<'ctx>,
        count: IntValue<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let p = int_prim(ty).expect("a narrower integer");
        let (bits, signed) = p.machine_int().expect("a machine integer");
        let count_t = count.get_type();
        // unsigned: a negative count is past the width too
        let bad = emit(self.builder.build_int_compare(IP::UGT, count, count_t.const_int(u64::from(bits - 1), false), "fixed.shbad"))?;
        self.fixed_violation_if(bad, Violation::ShiftCount, p)?;
        let n = self.count_at(count, bits)?;
        let v = match op {
            BinOp::Shl => emit(self.builder.build_left_shift(l, n, "fixed.shl"))?,
            _ => emit(self.builder.build_right_shift(l, n, signed, "fixed.shr"))?,
        };
        Ok((v.into(), ty.clone()))
    }

    /// A shift count (an `Int`) at the shifted value's width.
    fn count_at(&mut self, count: IntValue<'ctx>, bits: u32) -> Result<IntValue<'ctx>, CodegenError> {
        let t = self.context.custom_width_int_type(bits);
        Ok(if count.get_type().get_bit_width() > bits {
            emit(self.builder.build_int_truncate(count, t, "fixed.count"))?
        } else if count.get_type().get_bit_width() < bits {
            emit(self.builder.build_int_z_extend(count, t, "fixed.count"))?
        } else {
            count
        })
    }

    /// `-x` of a signed narrower integer: checked, as `0 - x` is (the
    /// minimum has no negation).
    pub(crate) fn lower_narrow_neg(&mut self, v: IntValue<'ctx>, ty: &CodegenTy) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let p = int_prim(ty).expect("a narrower integer");
        let zero = v.get_type().const_zero();
        let (n, o) = self.fixed_with_overflow("sub", zero, v, true)?;
        self.fixed_violation_if(o, Violation::Neg, p)?;
        Ok((n.into(), ty.clone()))
    }

    /// `abs(x)`: an unsigned integer's is itself; a signed one's is checked
    /// (the minimum has none).
    pub(crate) fn lower_narrow_abs(&mut self, v: IntValue<'ctx>, ty: &CodegenTy) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let p = int_prim(ty).expect("a narrower integer");
        let (_, signed) = p.machine_int().expect("a machine integer");
        if !signed {
            return Ok((v.into(), ty.clone()));
        }
        let zero = v.get_type().const_zero();
        let is_neg = emit(self.builder.build_int_compare(IP::SLT, v, zero, "fixed.abs.neg"))?;
        // `0 - x` overflows only at the minimum, which is negative
        let (n, o) = self.fixed_with_overflow("sub", zero, v, true)?;
        self.fixed_violation_if(o, Violation::Neg, p)?;
        let r = emit(self.builder.build_select(is_neg, n, v, "fixed.abs"))?;
        Ok((r, ty.clone()))
    }

    /// The operands of an intrinsic call, each lowered and fitted to
    /// `want` (an operand the check typed `p`, or a literal of it).
    fn fixed_args(&mut self, args: &[Expr], want: &[CodegenTy], scope: &Scope<'ctx>) -> Result<Vec<IntValue<'ctx>>, CodegenError> {
        if args.len() != want.len() {
            return Err(CodegenError::Unsupported(format!(
                "a fixed-width integer's intrinsic takes {} argument(s), got {}",
                want.len(),
                args.len()
            )));
        }
        let mut out = Vec::with_capacity(args.len());
        for (a, w) in args.iter().zip(want) {
            let (v, t) = self.lower_expr(a, scope)?;
            out.push(self.fit_int(v, &t, w)?.into_int_value());
        }
        Ok(out)
    }

    /// The natively lowered `std::i32::*` … `std::u8::*` functions.
    pub(crate) fn lower_std_fixed(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let (op, p) = id.fixed_int().expect("a fixed-width integer's intrinsic");
        let (bits, signed) = p.machine_int().expect("a machine integer");
        let int_t = self.context.custom_width_int_type(bits);
        let ty = CodegenTy::Narrow(p);
        let v = match op {
            // fallible: lowered under their `or` (`lower_std_fixed_checked`)
            FixedIntOp::CheckedAdd | FixedIntOp::CheckedSub | FixedIntOp::CheckedMul | FixedIntOp::CheckedDiv => {
                return Err(CodegenError::Unsupported(format!(
                    "`{}::checked_*` is fallible and is lowered under its `or`",
                    hale_types::ty::prim_name(p)
                )));
            }
            FixedIntOp::Max => {
                if signed {
                    int_t.const_int((1u64 << (bits - 1)) - 1, false)
                } else {
                    int_t.const_all_ones()
                }
            }
            FixedIntOp::Min => {
                if signed {
                    int_t.const_int(1u64 << (bits - 1), false)
                } else {
                    int_t.const_zero()
                }
            }
            FixedIntOp::WrappingAdd | FixedIntOp::WrappingSub | FixedIntOp::WrappingMul => {
                let a = self.fixed_args(args, &[ty.clone(), ty.clone()], scope)?;
                let (l, r) = (a[0], a[1]);
                match op {
                    FixedIntOp::WrappingAdd => emit(self.builder.build_int_add(l, r, "fixed.wadd"))?,
                    FixedIntOp::WrappingSub => emit(self.builder.build_int_sub(l, r, "fixed.wsub"))?,
                    _ => emit(self.builder.build_int_mul(l, r, "fixed.wmul"))?,
                }
            }
            // the count modulo the width, as Rust's `wrapping_shl` takes it
            FixedIntOp::WrappingShl | FixedIntOp::WrappingShr => {
                let l = self.fixed_args(&args[..1], &[ty.clone()], scope)?[0];
                let c = self.lower_expr(&args[1], scope)?.0.into_int_value();
                let masked = emit(self.builder.build_and(c, c.get_type().const_int(u64::from(bits - 1), false), "fixed.wsh.n"))?;
                let n = self.count_at(masked, bits)?;
                if op == FixedIntOp::WrappingShl {
                    emit(self.builder.build_left_shift(l, n, "fixed.wshl"))?
                } else {
                    emit(self.builder.build_right_shift(l, n, signed, "fixed.wshr"))?
                }
            }
        };
        Ok((v.into(), ty))
    }

    /// `T::checked_add / checked_sub / checked_mul / checked_div(a, b)`
    /// under its `or`: the operator's value in the success slot, or an
    /// `ArithError { kind, op }` in the error slot (`kind` `overflow`, or
    /// `division_by_zero` for a zero divisor; `op` the call), and the path
    /// bit the `or` branches on. Lowered at the call, so a program that
    /// names none carries none.
    pub(crate) fn lower_std_fixed_checked(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let (op, p) = id.fixed_int().expect("a fixed-width integer's intrinsic");
        let (bits, signed) = p.machine_int().expect("a machine integer");
        let int_t = self.context.custom_width_int_type(bits);
        let (lowered, ty) = {
            let (av, at) = self.lower_expr(&args[0], scope)?;
            let (bv, bt) = self.lower_expr(&args[1], scope)?;
            // the operands' own type (a literal's the other's)
            let ty = if crate::stdlib::fixed::int_prim(&at) == Some(p) { at.clone() } else { bt.clone() };
            ((self.fit_int(av, &at, &ty)?.into_int_value(), self.fit_int(bv, &bt, &ty)?.into_int_value()), ty)
        };
        let (a, b) = lowered;
        let name = hale_types::ty::prim_name(p);
        let (verb, value, bad, by_zero) = match op {
            FixedIntOp::CheckedAdd | FixedIntOp::CheckedSub | FixedIntOp::CheckedMul => {
                let (verb, short) = match op {
                    FixedIntOp::CheckedAdd => ("checked_add", "add"),
                    FixedIntOp::CheckedSub => ("checked_sub", "sub"),
                    _ => ("checked_mul", "mul"),
                };
                let (v, o) = self.fixed_with_overflow(short, a, b, signed)?;
                (verb, v, o, None)
            }
            _ => {
                let zero = emit(self.builder.build_int_compare(IP::EQ, b, int_t.const_zero(), "checked.by_zero"))?;
                let over = if signed {
                    let min = int_t.const_int(1u64 << (bits - 1), false);
                    let is_min = emit(self.builder.build_int_compare(IP::EQ, a, min, "checked.is_min"))?;
                    let by_neg1 = emit(self.builder.build_int_compare(IP::EQ, b, int_t.const_all_ones(), "checked.by_neg1"))?;
                    emit(self.builder.build_and(is_min, by_neg1, "checked.div_over"))?
                } else {
                    self.context.bool_type().const_zero()
                };
                let bad = emit(self.builder.build_or(zero, over, "checked.bad"))?;
                // a divisor that cannot trap where the result is not used
                let safe = emit(self.builder.build_select(bad, int_t.const_int(1, false), b, "checked.divisor"))?.into_int_value();
                let v = if signed {
                    emit(self.builder.build_int_signed_div(a, safe, "checked.sdiv"))?
                } else {
                    emit(self.builder.build_int_unsigned_div(a, safe, "checked.udiv"))?
                };
                ("checked_div", v, bad, Some(zero))
            }
        };
        let func = self
            .current_fn
            .ok_or_else(|| CodegenError::Unsupported(format!("`{name}::{verb}` outside a fn body")))?;
        let payload_ty = CodegenTy::TypeRef("ArithError".into());
        let out_val_slot = self.alloca_for(&ty, "checked.val.slot")?;
        let out_err_slot = self.alloca_for(&payload_ty, "checked.err.slot")?;
        let ok_bb = self.context.append_basic_block(func, "checked.ok");
        let err_bb = self.context.append_basic_block(func, "checked.err");
        let join_bb = self.context.append_basic_block(func, "checked.join");
        emit(self.builder.build_conditional_branch(bad, err_bb, ok_bb))?;

        self.builder.position_at_end(ok_bb);
        emit(self.builder.build_store(out_val_slot, value))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(err_bb);
        let op_text = format!("{name}::{verb}");
        let err = self.error_payload(|cx| -> Result<inkwell::values::PointerValue<'ctx>, CodegenError> {
            let info = cx.user_types.get("ArithError").cloned().ok_or_else(|| {
                CodegenError::Unsupported("`ArithError` is not declared (the builtin table's row)".into())
            })?;
            let size = info.struct_ty.size_of().expect("ArithError has a known size");
            let ptr = cx.arena_alloc(size, "ArithError.alloc")?;
            let overflow = cx.global_string("overflow");
            let kind: BasicValueEnum<'ctx> = match by_zero {
                Some(z) => {
                    let dz = cx.global_string("division_by_zero");
                    emit(cx.builder.build_select(z, dz, overflow, "checked.kind"))?
                }
                None => overflow.into(),
            };
            let op_g = cx.global_string(&op_text);
            for (field, v) in [("kind", kind), ("op", op_g.into())] {
                let (idx, _) = info.fields.get(field).cloned().expect("a field of the row");
                let at = emit(cx.builder.build_struct_gep(info.struct_ty, ptr, idx, &format!("ArithError.{field}.ptr")))?;
                emit(cx.builder.build_store(at, v))?;
            }
            Ok(ptr)
        })?;
        emit(self.builder.build_store(out_err_slot, err))?;
        emit(self.builder.build_unconditional_branch(join_bb))?;

        self.builder.position_at_end(join_bb);
        Ok(FallibleCallResult { i1_path: bad, out_val_slot: Some(out_val_slot), out_err_slot, success_ty: Some(ty), payload_ty })
    }
}
