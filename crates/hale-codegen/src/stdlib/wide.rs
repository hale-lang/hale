//! GH #1934: `U256` and `I256`, the 256-bit integers.
//!
//! A value is an LLVM `i256` (32 bytes, the target's i256 alignment: 16 on
//! x86-64 and aarch64). The operators are checked: an overflow, a
//! division by zero, a negated `I256::min()` or a negative shift count
//! branches to `lotus_wide_violation`, which prints the violation and
//! exits, so no value ever wraps. LLVM expands every i256 and i512
//! operation used here inline (`ExpandLargeDivRem` for the divisions, the
//! multiply and the overflow intrinsics in the legalizer) on x86-64,
//! aarch64 and wasm32, so nothing links against a compiler-rt builtin.
//! Text and bytes go through the runtime (`lotus_wide_*` in
//! `lotus_arena.c`), which reads a value from a 32-byte stack slot.

use hale_syntax::ast::{BinOp, Expr};
use hale_types::stdlib_surface::IntrinsicId;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::IntPredicate as IP;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

fn emit<T>(r: Result<T, inkwell::builder::BuilderError>) -> Result<T, CodegenError> {
    r.map_err(|e| CodegenError::LlvmEmit(e.to_string()))
}

/// What `lotus_wide_violation` says happened (its `what` argument).
#[derive(Clone, Copy)]
enum Violation {
    Add = 0,
    Sub = 1,
    Mul = 2,
    Div = 3,
    DivByZero = 5,
    Neg = 6,
    NegativeInt = 7,
    NegativeShift = 8,
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    pub(crate) fn wide_int_type(&self) -> inkwell::types::IntType<'ctx> {
        self.context.custom_width_int_type(256)
    }

    fn wide_const(&self, words: [u64; 4]) -> IntValue<'ctx> {
        self.wide_int_type().const_int_arbitrary_precision(&words)
    }

    /// `I256::min()`: -2^255.
    fn wide_min_signed(&self) -> IntValue<'ctx> {
        self.wide_const([0, 0, 0, 1u64 << 63])
    }

    /// Branch to the violation when `bad` holds; continue after it
    /// otherwise.
    fn wide_violation_if(&mut self, bad: IntValue<'ctx>, what: Violation, signed: bool) -> Result<(), CodegenError> {
        let func = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .ok_or_else(|| CodegenError::LlvmEmit("a 256-bit operator outside a function".into()))?;
        let fail = self.context.append_basic_block(func, "wide.violation");
        let ok = self.context.append_basic_block(func, "wide.ok");
        emit(self.builder.build_conditional_branch(bad, fail, ok))?;
        self.builder.position_at_end(fail);
        let f = self.module.get_function("lotus_wide_violation").expect("lotus_wide_violation declared");
        let i32_t = self.context.i32_type();
        emit(self.builder.build_call(
            f,
            &[i32_t.const_int(what as u64, false).into(), i32_t.const_int(u64::from(signed), false).into()],
            "",
        ))?;
        emit(self.builder.build_unreachable())?;
        self.builder.position_at_end(ok);
        Ok(())
    }

    /// `llvm.{s,u}{add,sub,mul}.with.overflow.i256`: the wrapped result
    /// and whether it wrapped.
    fn wide_with_overflow(
        &mut self,
        op: &str,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
        signed: bool,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let name = format!("llvm.{}{}.with.overflow", if signed { "s" } else { "u" }, op);
        let intrinsic = inkwell::intrinsics::Intrinsic::find(&name)
            .ok_or_else(|| CodegenError::LlvmEmit(format!("{name} not found")))?;
        let f = intrinsic
            .get_declaration(&self.module, &[self.wide_int_type().into()])
            .ok_or_else(|| CodegenError::LlvmEmit(format!("{name}.i256 not declared")))?;
        let pair = emit(self.builder.build_call(f, &[l.into(), r.into()], "wide.ovf"))?
            .try_as_basic_value()
            .left()
            .expect("an overflow intrinsic returns a pair")
            .into_struct_value();
        let v = emit(self.builder.build_extract_value(pair, 0, "wide.ovf.v"))?.into_int_value();
        let o = emit(self.builder.build_extract_value(pair, 1, "wide.ovf.o"))?.into_int_value();
        Ok((v, o))
    }

    /// Whether `a / b` is `I256::min() / -1`, the one signed quotient
    /// past the range.
    fn wide_min_by_minus_one(&mut self, l: IntValue<'ctx>, r: IntValue<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        let min = self.wide_min_signed();
        let all = self.wide_int_type().const_all_ones();
        let lmin = emit(self.builder.build_int_compare(IP::EQ, l, min, "wide.lmin"))?;
        let rneg1 = emit(self.builder.build_int_compare(IP::EQ, r, all, "wide.rneg1"))?;
        emit(self.builder.build_and(lmin, rneg1, "wide.min_by_neg1"))
    }

    /// A binary operator over two `U256`s or two `I256`s (the checker
    /// refuses any other mix; a shift is [`Self::lower_wide_shift`]).
    pub(crate) fn lower_wide_binop(
        &mut self,
        op: BinOp,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let signed = matches!(ty, CodegenTy::I256);
        let zero = self.wide_int_type().const_zero();
        let v: IntValue<'ctx> = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let (name, what) = match op {
                    BinOp::Add => ("add", Violation::Add),
                    BinOp::Sub => ("sub", Violation::Sub),
                    _ => ("mul", Violation::Mul),
                };
                let (v, o) = self.wide_with_overflow(name, l, r, signed)?;
                self.wide_violation_if(o, what, signed)?;
                v
            }
            BinOp::Div | BinOp::Mod => {
                let by_zero = emit(self.builder.build_int_compare(IP::EQ, r, zero, "wide.by_zero"))?;
                self.wide_violation_if(by_zero, Violation::DivByZero, signed)?;
                if signed {
                    let edge = self.wide_min_by_minus_one(l, r)?;
                    if op == BinOp::Div {
                        self.wide_violation_if(edge, Violation::Div, signed)?;
                        emit(self.builder.build_int_signed_div(l, r, "wide.sdiv"))?
                    } else {
                        // `I256::min() % -1` is 0, which fits; LLVM's
                        // `srem` of it is undefined, so the divisor is 1
                        // there instead.
                        let one = self.wide_int_type().const_int(1, false);
                        let d = emit(self.builder.build_select(edge, one, r, "wide.srem.d"))?.into_int_value();
                        emit(self.builder.build_int_signed_rem(l, d, "wide.srem"))?
                    }
                } else if op == BinOp::Div {
                    emit(self.builder.build_int_unsigned_div(l, r, "wide.udiv"))?
                } else {
                    emit(self.builder.build_int_unsigned_rem(l, r, "wide.urem"))?
                }
            }
            BinOp::BitAnd => emit(self.builder.build_and(l, r, "wide.and"))?,
            BinOp::BitOr => emit(self.builder.build_or(l, r, "wide.or"))?,
            BinOp::BitXor => emit(self.builder.build_xor(l, r, "wide.xor"))?,
            BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                let pred = match (op, signed) {
                    (BinOp::Eq, _) => IP::EQ,
                    (BinOp::NotEq, _) => IP::NE,
                    (BinOp::Lt, true) => IP::SLT,
                    (BinOp::Lt, false) => IP::ULT,
                    (BinOp::Gt, true) => IP::SGT,
                    (BinOp::Gt, false) => IP::UGT,
                    (BinOp::LtEq, true) => IP::SLE,
                    (BinOp::LtEq, false) => IP::ULE,
                    (BinOp::GtEq, true) => IP::SGE,
                    _ => IP::UGE,
                };
                let c = emit(self.builder.build_int_compare(pred, l, r, "wide.cmp"))?;
                return Ok((c.into(), CodegenTy::Bool));
            }
            _ => {
                return Err(CodegenError::Unsupported(
                    "a 256-bit integer's operator the checker admits and lowering does not know".into(),
                ));
            }
        };
        Ok((v.into(), ty.clone()))
    }

    /// `x << n` / `x >> n`, the count an `Int`. Bits shifted past either
    /// end are dropped (a bit operation, not arithmetic: `*` is the
    /// checked multiply); a count of 256 or more shifts every bit out,
    /// and an `I256`'s `>>` fills with its sign (`x >> n` is
    /// `floor(x / 2^n)`). A negative count is a violation.
    pub(crate) fn lower_wide_shift(
        &mut self,
        op: BinOp,
        l: IntValue<'ctx>,
        count: IntValue<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let signed = matches!(ty, CodegenTy::I256);
        let i64_t = self.context.i64_type();
        let negative = emit(self.builder.build_int_compare(IP::SLT, count, i64_t.const_zero(), "wide.shneg"))?;
        self.wide_violation_if(negative, Violation::NegativeShift, signed)?;
        let wide = self.wide_int_type();
        let big = emit(self.builder.build_int_compare(IP::SGT, count, i64_t.const_int(255, false), "wide.shbig"))?;
        let clamped = emit(self.builder.build_select(big, i64_t.const_int(255, false), count, "wide.shc"))?.into_int_value();
        let n = emit(self.builder.build_int_z_extend(clamped, wide, "wide.shn"))?;
        let v = match op {
            BinOp::Shl => {
                let s = emit(self.builder.build_left_shift(l, n, "wide.shl"))?;
                emit(self.builder.build_select(big, wide.const_zero(), s, "wide.shl.r"))?.into_int_value()
            }
            _ if signed => emit(self.builder.build_right_shift(l, n, true, "wide.ashr"))?,
            _ => {
                let s = emit(self.builder.build_right_shift(l, n, false, "wide.lshr"))?;
                emit(self.builder.build_select(big, wide.const_zero(), s, "wide.lshr.r"))?.into_int_value()
            }
        };
        Ok((v.into(), ty.clone()))
    }

    /// Unary `-` of an `I256` (`-I256::min()` is a violation) and `~` of
    /// either.
    pub(crate) fn lower_wide_neg(
        &mut self,
        v: IntValue<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let min = self.wide_min_signed();
        let is_min = emit(self.builder.build_int_compare(IP::EQ, v, min, "wide.negmin"))?;
        self.wide_violation_if(is_min, Violation::Neg, true)?;
        let n = emit(self.builder.build_int_neg(v, "wide.neg"))?;
        Ok((n.into(), ty.clone()))
    }

    pub(crate) fn lower_wide_bitnot(
        &mut self,
        v: IntValue<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let n = emit(self.builder.build_not(v, "wide.not"))?;
        Ok((n.into(), ty.clone()))
    }

    /// `abs(x)`: an `I256`'s magnitude (`abs(I256::min())` is a
    /// violation), a `U256` itself.
    pub(crate) fn lower_wide_abs(
        &mut self,
        v: IntValue<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !matches!(ty, CodegenTy::I256) {
            return Ok((v.into(), ty.clone()));
        }
        let zero = self.wide_int_type().const_zero();
        let is_neg = emit(self.builder.build_int_compare(IP::SLT, v, zero, "wide.abs.isneg"))?;
        let (n, _) = self.lower_wide_neg(v, ty)?;
        let r = emit(self.builder.build_select(is_neg, n.into_int_value(), v, "wide.abs"))?;
        Ok((r, ty.clone()))
    }

    /// `v` in a fresh 32-byte stack slot, for a runtime call that reads
    /// it (`const uint64_t *`). An entry-block alloca has the i256's ABI
    /// alignment, so the store is aligned.
    fn wide_spill(&mut self, v: IntValue<'ctx>) -> Result<PointerValue<'ctx>, CodegenError> {
        let slot = self.alloca_in_entry(self.wide_int_type().into(), "wide.slot")?;
        emit(self.builder.build_store(slot, v))?;
        Ok(slot)
    }

    /// The decimal text of a 256-bit integer (`to_string`, `println`,
    /// `+` with a `String`, an f-string), in the current arena.
    pub(crate) fn wide_to_string(
        &mut self,
        v: IntValue<'ctx>,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        self.wide_text_call("lotus_wide_to_string", v, signed)
    }

    fn wide_text_call(&mut self, f: &str, v: IntValue<'ctx>, signed: bool) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let slot = self.wide_spill(v)?;
        let arena = self.current_arena_ptr()?;
        let func = self.module.get_function(f).unwrap_or_else(|| panic!("{f} declared"));
        let i32_t = self.context.i32_type();
        Ok(emit(self.builder.build_call(
            func,
            &[arena.into(), slot.into(), i32_t.const_int(u64::from(signed), false).into()],
            "wide.text",
        ))?
        .try_as_basic_value()
        .left()
        .expect("returns ptr"))
    }

    /// The operands of an intrinsic call, each lowered, with a `String`
    /// view unpacked.
    fn wide_args(&mut self, args: &[Expr], want: usize, what: &str, scope: &Scope<'ctx>) -> Result<Vec<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        if args.len() != want {
            return Err(CodegenError::Unsupported(format!("{what} takes {want} argument(s), got {}", args.len())));
        }
        let mut out = Vec::with_capacity(want);
        for a in args {
            let (v, t) = self.lower_expr(a, scope)?;
            let v = match t {
                CodegenTy::StringView | CodegenTy::BytesView => self.unpack_view_if_needed(v, &t)?,
                _ => v,
            };
            out.push((v, t));
        }
        Ok(out)
    }

    /// `lotus_wide_parse` / `lotus_wide_from_be_bytes` into a zeroed
    /// slot: the status, and the value read (zero when it did not read).
    fn wide_read_call(
        &mut self,
        f: &str,
        input: BasicValueEnum<'ctx>,
        signed: Option<bool>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let slot = self.wide_spill(self.wide_int_type().const_zero())?;
        let func = self.module.get_function(f).unwrap_or_else(|| panic!("{f} declared"));
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = vec![input.into()];
        if let Some(s) = signed {
            call_args.push(self.context.i32_type().const_int(u64::from(s), false).into());
        }
        call_args.push(slot.into());
        let rc = emit(self.builder.build_call(func, &call_args, "wide.read"))?
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let v = emit(self.builder.build_load(self.wide_int_type(), slot, "wide.read.v"))?.into_int_value();
        Ok((rc, v))
    }

    /// The natively lowered `std::u256::*` / `std::i256::*` functions.
    pub(crate) fn lower_std_wide(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        use IntrinsicId as Id;
        let signed = matches!(
            id,
            Id::I256AddOverflowsRaw
                | Id::I256FromBeBytesRaw
                | Id::I256FromBitsRaw
                | Id::I256LowRaw
                | Id::I256MulDivRaw
                | Id::I256MulDivOverflowsRaw
                | Id::I256MulOverflowsRaw
                | Id::I256ParseRaw
                | Id::I256ParsesRaw
                | Id::I256SubOverflowsRaw
                | Id::I256FromInt
                | Id::I256Max
                | Id::I256Min
                | Id::I256ToBeBytes
                | Id::I256ToHex
        );
        let ty = if signed { CodegenTy::I256 } else { CodegenTy::U256 };
        let wide = self.wide_int_type();
        let what = if signed { "an I256 intrinsic" } else { "a U256 intrinsic" };
        match id {
            Id::U256FromInt | Id::I256FromInt => {
                let a = self.wide_args(args, 1, what, scope)?;
                let n = a[0].0.into_int_value();
                if !signed {
                    let neg = emit(self.builder.build_int_compare(IP::SLT, n, self.context.i64_type().const_zero(), "wide.fromneg"))?;
                    self.wide_violation_if(neg, Violation::NegativeInt, false)?;
                }
                let v = if signed {
                    emit(self.builder.build_int_s_extend(n, wide, "wide.from_int"))?
                } else {
                    emit(self.builder.build_int_z_extend(n, wide, "wide.from_int"))?
                };
                Ok((v.into(), ty))
            }
            Id::U256Max => Ok((wide.const_all_ones().into(), ty)),
            Id::I256Max => Ok((self.wide_const([u64::MAX, u64::MAX, u64::MAX, u64::MAX >> 1]).into(), ty)),
            Id::I256Min => Ok((self.wide_min_signed().into(), ty)),
            Id::U256LowRaw | Id::I256LowRaw => {
                let a = self.wide_args(args, 1, what, scope)?;
                let v = emit(self.builder.build_int_truncate(a[0].0.into_int_value(), self.context.i64_type(), "wide.low"))?;
                Ok((v.into(), CodegenTy::Int))
            }
            // the same 256 bits read as the other type
            Id::U256FromBitsRaw | Id::I256FromBitsRaw => {
                let a = self.wide_args(args, 1, what, scope)?;
                Ok((a[0].0, ty))
            }
            Id::U256AddOverflowsRaw
            | Id::I256AddOverflowsRaw
            | Id::U256SubOverflowsRaw
            | Id::I256SubOverflowsRaw
            | Id::U256MulOverflowsRaw
            | Id::I256MulOverflowsRaw => {
                let a = self.wide_args(args, 2, what, scope)?;
                let op = match id {
                    Id::U256AddOverflowsRaw | Id::I256AddOverflowsRaw => "add",
                    Id::U256SubOverflowsRaw | Id::I256SubOverflowsRaw => "sub",
                    _ => "mul",
                };
                let (_, o) = self.wide_with_overflow(op, a[0].0.into_int_value(), a[1].0.into_int_value(), signed)?;
                Ok((o.into(), CodegenTy::Bool))
            }
            Id::U256MulDivRaw | Id::I256MulDivRaw | Id::U256MulDivOverflowsRaw | Id::I256MulDivOverflowsRaw => {
                let a = self.wide_args(args, 3, what, scope)?;
                let (q, fits) = self.wide_mul_div(
                    a[0].0.into_int_value(),
                    a[1].0.into_int_value(),
                    a[2].0.into_int_value(),
                    signed,
                )?;
                if matches!(id, Id::U256MulDivRaw | Id::I256MulDivRaw) {
                    Ok((q.into(), ty))
                } else {
                    let over = emit(self.builder.build_not(fits, "wide.md.over"))?;
                    Ok((over.into(), CodegenTy::Bool))
                }
            }
            Id::U256ParseRaw | Id::I256ParseRaw | Id::U256ParsesRaw | Id::I256ParsesRaw => {
                let a = self.wide_args(args, 1, what, scope)?;
                let (rc, v) = self.wide_read_call("lotus_wide_parse", a[0].0, Some(signed))?;
                if matches!(id, Id::U256ParseRaw | Id::I256ParseRaw) {
                    Ok((v.into(), ty))
                } else {
                    let ok = emit(self.builder.build_int_compare(IP::EQ, rc, self.context.i64_type().const_zero(), "wide.parses"))?;
                    Ok((ok.into(), CodegenTy::Bool))
                }
            }
            Id::U256FromBeBytesRaw | Id::I256FromBeBytesRaw => {
                let a = self.wide_args(args, 1, what, scope)?;
                let (_, v) = self.wide_read_call("lotus_wide_from_be_bytes", a[0].0, None)?;
                Ok((v.into(), ty))
            }
            Id::U256ToHex | Id::I256ToHex => {
                let a = self.wide_args(args, 1, what, scope)?;
                let s = self.wide_text_call("lotus_wide_to_hex", a[0].0.into_int_value(), signed)?;
                Ok((s, CodegenTy::String))
            }
            Id::U256ToBeBytes | Id::I256ToBeBytes => {
                let a = self.wide_args(args, 1, what, scope)?;
                let slot = self.wide_spill(a[0].0.into_int_value())?;
                let arena = self.current_arena_ptr()?;
                let f = self.module.get_function("lotus_wide_to_be_bytes").expect("lotus_wide_to_be_bytes declared");
                let b = emit(self.builder.build_call(f, &[arena.into(), slot.into()], "wide.bytes"))?
                    .try_as_basic_value()
                    .left()
                    .expect("returns ptr");
                Ok((b, CodegenTy::Bytes))
            }
            _ => Err(CodegenError::Unsupported(format!("{what}: not one the 256-bit lowering knows"))),
        }
    }

    /// `a * b / c` with the product exact in 512 bits: the quotient
    /// (rounded toward zero, as `/` rounds) truncated to 256 bits, and
    /// whether it fits. `c` must not be zero (the Hale body refuses it
    /// first); a zero `c` here divides by one instead of trapping.
    fn wide_mul_div(
        &mut self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
        c: IntValue<'ctx>,
        signed: bool,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let wide = self.wide_int_type();
        let w512 = self.context.custom_width_int_type(512);
        let ext = |cx: &mut Self, v: IntValue<'ctx>, n: &str| -> Result<IntValue<'ctx>, CodegenError> {
            if signed {
                emit(cx.builder.build_int_s_extend(v, w512, n))
            } else {
                emit(cx.builder.build_int_z_extend(v, w512, n))
            }
        };
        let zero = wide.const_zero();
        let c_zero = emit(self.builder.build_int_compare(IP::EQ, c, zero, "wide.md.c0"))?;
        let c = emit(self.builder.build_select(c_zero, wide.const_int(1, false), c, "wide.md.c"))?.into_int_value();
        let aw = ext(self, a, "wide.md.a")?;
        let bw = ext(self, b, "wide.md.b")?;
        let cw = ext(self, c, "wide.md.cw")?;
        // Two 256-bit factors never overflow 512 bits, signed or not.
        let p = emit(self.builder.build_int_mul(aw, bw, "wide.md.p"))?;
        let q = if signed {
            // |p| < 2^510 and |c| >= 1, so the i512 quotient never
            // overflows either (no MIN / -1 at 512 bits).
            emit(self.builder.build_int_signed_div(p, cw, "wide.md.q"))?
        } else {
            emit(self.builder.build_int_unsigned_div(p, cw, "wide.md.q"))?
        };
        let t = emit(self.builder.build_int_truncate(q, wide, "wide.md.t"))?;
        let back = ext(self, t, "wide.md.back")?;
        let fits = emit(self.builder.build_int_compare(IP::EQ, back, q, "wide.md.fits"))?;
        Ok((t, fits))
    }
}

