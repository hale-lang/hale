//! GH #2121: `U64`, the 64-bit unsigned integer.
//!
//! A value is an LLVM `i64` read as unsigned: the same 8 bytes as an
//! `Int` in every layout, so only the operators, the conversions and the
//! text differ. The operators are checked: an overflow (`llvm.u{add,sub,
//! mul}.with.overflow.i64`), a division or remainder by zero, and a shift
//! by a count outside 0 through 63 branch to `lotus_u64_violation`, which
//! prints the violation and exits, so no value ever wraps unless the
//! program asked (`U64::wrapping_*`). Text and bytes go through the
//! runtime (`lotus_u64_*` in `lotus_arena.c`), by value.

use hale_syntax::ast::{BinOp, Expr};
use hale_types::stdlib_surface::IntrinsicId;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate as IP;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

fn emit<T>(r: Result<T, inkwell::builder::BuilderError>) -> Result<T, CodegenError> {
    r.map_err(|e| CodegenError::LlvmEmit(e.to_string()))
}

/// What `lotus_u64_violation` says happened (its `what` argument).
#[derive(Clone, Copy)]
enum Violation {
    Add = 0,
    Sub = 1,
    Mul = 2,
    DivByZero = 3,
    RemByZero = 4,
    NegativeInt = 5,
    ShiftCount = 6,
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Branch to the violation when `bad` holds; continue after it
    /// otherwise.
    fn u64_violation_if(&mut self, bad: IntValue<'ctx>, what: Violation) -> Result<(), CodegenError> {
        let func = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .ok_or_else(|| CodegenError::LlvmEmit("a U64 operator outside a function".into()))?;
        let fail = self.context.append_basic_block(func, "u64.violation");
        let ok = self.context.append_basic_block(func, "u64.ok");
        emit(self.builder.build_conditional_branch(bad, fail, ok))?;
        self.builder.position_at_end(fail);
        let f = self.module.get_function("lotus_u64_violation").expect("lotus_u64_violation declared");
        emit(self.builder.build_call(f, &[self.context.i32_type().const_int(what as u64, false).into()], ""))?;
        emit(self.builder.build_unreachable())?;
        self.builder.position_at_end(ok);
        Ok(())
    }

    /// `llvm.u{add,sub,mul}.with.overflow.i64`: the wrapped result and
    /// whether it wrapped.
    fn u64_with_overflow(
        &mut self,
        op: &str,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let name = format!("llvm.u{op}.with.overflow");
        let intrinsic = inkwell::intrinsics::Intrinsic::find(&name)
            .ok_or_else(|| CodegenError::LlvmEmit(format!("{name} not found")))?;
        let f = intrinsic
            .get_declaration(&self.module, &[self.context.i64_type().into()])
            .ok_or_else(|| CodegenError::LlvmEmit(format!("{name}.i64 not declared")))?;
        let pair = emit(self.builder.build_call(f, &[l.into(), r.into()], "u64.ovf"))?
            .try_as_basic_value()
            .left()
            .expect("an overflow intrinsic returns a pair")
            .into_struct_value();
        let v = emit(self.builder.build_extract_value(pair, 0, "u64.ovf.v"))?.into_int_value();
        let o = emit(self.builder.build_extract_value(pair, 1, "u64.ovf.o"))?.into_int_value();
        Ok((v, o))
    }

    /// A binary operator over two `U64`s (the checker refuses any other
    /// mix; an integer literal beside one is one, and a shift is
    /// [`Self::lower_u64_shift`]).
    pub(crate) fn lower_u64_binop(
        &mut self,
        op: BinOp,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let zero = self.context.i64_type().const_zero();
        let v: IntValue<'ctx> = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let (name, what) = match op {
                    BinOp::Add => ("add", Violation::Add),
                    BinOp::Sub => ("sub", Violation::Sub),
                    _ => ("mul", Violation::Mul),
                };
                let (v, o) = self.u64_with_overflow(name, l, r)?;
                self.u64_violation_if(o, what)?;
                v
            }
            BinOp::Div | BinOp::Mod => {
                let by_zero = emit(self.builder.build_int_compare(IP::EQ, r, zero, "u64.by_zero"))?;
                if op == BinOp::Div {
                    self.u64_violation_if(by_zero, Violation::DivByZero)?;
                    emit(self.builder.build_int_unsigned_div(l, r, "u64.udiv"))?
                } else {
                    self.u64_violation_if(by_zero, Violation::RemByZero)?;
                    emit(self.builder.build_int_unsigned_rem(l, r, "u64.urem"))?
                }
            }
            BinOp::BitAnd => emit(self.builder.build_and(l, r, "u64.and"))?,
            BinOp::BitOr => emit(self.builder.build_or(l, r, "u64.or"))?,
            BinOp::BitXor => emit(self.builder.build_xor(l, r, "u64.xor"))?,
            BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                let pred = match op {
                    BinOp::Eq => IP::EQ,
                    BinOp::NotEq => IP::NE,
                    BinOp::Lt => IP::ULT,
                    BinOp::Gt => IP::UGT,
                    BinOp::LtEq => IP::ULE,
                    _ => IP::UGE,
                };
                let c = emit(self.builder.build_int_compare(pred, l, r, "u64.cmp"))?;
                return Ok((c.into(), CodegenTy::Bool));
            }
            BinOp::Shl | BinOp::Shr => return self.lower_u64_shift(op, l, r),
            _ => {
                return Err(CodegenError::Unsupported(
                    "a U64 operator the checker admits and lowering does not know".into(),
                ));
            }
        };
        Ok((v.into(), CodegenTy::U64))
    }

    /// `x << n` / `x >> n`, the count an `Int`: bits shifted out are
    /// dropped and `>>` fills with zeros (`lshr`). A count outside 0
    /// through 63 is a violation (`U64::wrapping_shl` / `wrapping_shr`
    /// take it modulo 64), never LLVM's poison.
    pub(crate) fn lower_u64_shift(
        &mut self,
        op: BinOp,
        l: IntValue<'ctx>,
        count: IntValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let i64_t = self.context.i64_type();
        // unsigned: a negative count is past 63 too
        let bad = emit(self.builder.build_int_compare(IP::UGT, count, i64_t.const_int(63, false), "u64.shbad"))?;
        self.u64_violation_if(bad, Violation::ShiftCount)?;
        let v = match op {
            BinOp::Shl => emit(self.builder.build_left_shift(l, count, "u64.shl"))?,
            _ => emit(self.builder.build_right_shift(l, count, false, "u64.lshr"))?,
        };
        Ok((v.into(), CodegenTy::U64))
    }

    /// `~x`.
    pub(crate) fn lower_u64_bitnot(&mut self, v: IntValue<'ctx>) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let n = emit(self.builder.build_not(v, "u64.not"))?;
        Ok((n.into(), CodegenTy::U64))
    }

    /// The decimal text of a `U64` (`to_string`, `+` with a `String`, an
    /// f-string), in the current arena.
    pub(crate) fn u64_to_string(&mut self, v: IntValue<'ctx>) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        self.u64_text_call("lotus_u64_to_string", v)
    }

    fn u64_text_call(&mut self, f: &str, v: IntValue<'ctx>) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let arena = self.current_arena_ptr()?;
        let func = self.module.get_function(f).unwrap_or_else(|| panic!("{f} declared"));
        Ok(emit(self.builder.build_call(func, &[arena.into(), v.into()], "u64.text"))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr"))
    }

    /// `Float(x)`: the nearest double (`uitofp`), exact through 2^53.
    pub(crate) fn u64_to_float(&mut self, v: IntValue<'ctx>) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        Ok(emit(self.builder.build_unsigned_int_to_float(v, self.context.f64_type(), "u64.uitofp"))?.into())
    }

    /// The operands of an intrinsic call, each lowered, with a `String`
    /// view unpacked.
    fn u64_args(&mut self, args: &[Expr], want: usize, scope: &Scope<'ctx>) -> Result<Vec<BasicValueEnum<'ctx>>, CodegenError> {
        if args.len() != want {
            return Err(CodegenError::Unsupported(format!(
                "a U64 intrinsic takes {want} argument(s), got {}",
                args.len()
            )));
        }
        let mut out = Vec::with_capacity(want);
        for a in args {
            let (v, t) = self.lower_expr(a, scope)?;
            let v = match t {
                CodegenTy::StringView | CodegenTy::BytesView => self.unpack_view_if_needed(v, &t)?,
                _ => v,
            };
            out.push(v);
        }
        Ok(out)
    }

    /// The natively lowered `std::u64::*` functions.
    pub(crate) fn lower_std_u64(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        use IntrinsicId as Id;
        let i64_t = self.context.i64_type();
        match id {
            Id::U64FromInt => {
                let a = self.u64_args(args, 1, scope)?;
                let n = a[0].into_int_value();
                let neg = emit(self.builder.build_int_compare(IP::SLT, n, i64_t.const_zero(), "u64.fromneg"))?;
                self.u64_violation_if(neg, Violation::NegativeInt)?;
                Ok((n.into(), CodegenTy::U64))
            }
            Id::U64Max => Ok((i64_t.const_all_ones().into(), CodegenTy::U64)),
            // the same 64 bits read as the other type
            Id::U64FromBits => {
                let a = self.u64_args(args, 1, scope)?;
                Ok((a[0], CodegenTy::U64))
            }
            Id::U64ToBits => {
                let a = self.u64_args(args, 1, scope)?;
                Ok((a[0], CodegenTy::Int))
            }
            Id::U64WrappingAdd | Id::U64WrappingSub | Id::U64WrappingMul => {
                let a = self.u64_args(args, 2, scope)?;
                let (l, r) = (a[0].into_int_value(), a[1].into_int_value());
                let v = match id {
                    Id::U64WrappingAdd => emit(self.builder.build_int_add(l, r, "u64.wadd"))?,
                    Id::U64WrappingSub => emit(self.builder.build_int_sub(l, r, "u64.wsub"))?,
                    _ => emit(self.builder.build_int_mul(l, r, "u64.wmul"))?,
                };
                Ok((v.into(), CodegenTy::U64))
            }
            // the count modulo 64, as Rust's `wrapping_shl` takes it
            Id::U64WrappingShl | Id::U64WrappingShr => {
                let a = self.u64_args(args, 2, scope)?;
                let n = emit(self.builder.build_and(a[1].into_int_value(), i64_t.const_int(63, false), "u64.wsh.n"))?;
                let l = a[0].into_int_value();
                let v = if id == Id::U64WrappingShl {
                    emit(self.builder.build_left_shift(l, n, "u64.wshl"))?
                } else {
                    emit(self.builder.build_right_shift(l, n, false, "u64.wshr"))?
                };
                Ok((v.into(), CodegenTy::U64))
            }
            Id::U64ToHex => {
                let a = self.u64_args(args, 1, scope)?;
                let s = self.u64_text_call("lotus_u64_to_hex", a[0].into_int_value())?;
                Ok((s, CodegenTy::String))
            }
            Id::U64ToBeBytes | Id::U64ToLeBytes => {
                let a = self.u64_args(args, 1, scope)?;
                let arena = self.current_arena_ptr()?;
                let f = self.module.get_function("lotus_u64_to_bytes").expect("lotus_u64_to_bytes declared");
                let big = self.context.i32_type().const_int(u64::from(id == Id::U64ToBeBytes), false);
                let b = emit(self.builder.build_call(f, &[arena.into(), a[0].into(), big.into()], "u64.bytes"))?
                    .try_as_basic_value()
                    .left()
                    .expect("returns ptr");
                Ok((b, CodegenTy::Bytes))
            }
            Id::U64ParseRaw | Id::U64ParsesRaw => {
                let a = self.u64_args(args, 1, scope)?;
                let slot = self.alloca_in_entry(i64_t.into(), "u64.parse.slot")?;
                emit(self.builder.build_store(slot, i64_t.const_zero()))?;
                let f = self.module.get_function("lotus_u64_parse").expect("lotus_u64_parse declared");
                let rc = emit(self.builder.build_call(f, &[a[0].into(), slot.into()], "u64.parse"))?
                    .try_as_basic_value()
                    .left()
                    .expect("returns i64")
                    .into_int_value();
                if id == Id::U64ParseRaw {
                    let v = emit(self.builder.build_load(i64_t, slot, "u64.parse.v"))?;
                    Ok((v, CodegenTy::U64))
                } else {
                    let ok = emit(self.builder.build_int_compare(IP::EQ, rc, i64_t.const_zero(), "u64.parses"))?;
                    Ok((ok.into(), CodegenTy::Bool))
                }
            }
            // `U64::from_float`'s body refused what is outside the range
            // first; truncated toward zero
            Id::U64FromFloatRaw => {
                let a = self.u64_args(args, 1, scope)?;
                let v = emit(self.builder.build_float_to_unsigned_int(a[0].into_float_value(), i64_t, "u64.fptoui"))?;
                Ok((v.into(), CodegenTy::U64))
            }
            _ => Err(CodegenError::Unsupported("a U64 intrinsic the lowering does not know".into())),
        }
    }
}
