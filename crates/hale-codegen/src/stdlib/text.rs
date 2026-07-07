//! `std::text::*` path-call lowering (subset).
//!
//! Round 1.8 extracts the self-contained primitives — byte-class
//! predicates + base64 codec. `std::text::tokenize_words_into` digs
//! into locus-internal types (LocusInfo, CapacitySlotLayout, SlotForm)
//! and stays in `codegen.rs` until Round 4 (locus extraction) exposes
//! that surface.

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait TextStdlib<'ctx> {
    fn lower_std_text_byte_pred(
        &mut self,
        which: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_text_base64_encode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_text_base64_decode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_text_base64_url_encode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> TextStdlib<'ctx> for Cx<'ctx, 'p> {
    /// 2026-05-16: std::text byte-class predicates. Each takes a
    /// single Int (byte value) and returns Bool. Lowering is
    /// inline IR — no libc, no extern call — so the predicate is
    /// effectively a few `icmp` + `and` / `or` instructions, and
    /// LLVM can fold it across loop bodies.
    ///
    /// Naming: byte-level (not char-level) because Hale Strings
    /// are UTF-8 byte sequences at the runtime. ASCII range only
    /// at v1; agents needing Unicode classification fall back to
    /// std::str::* or open-coded checks.
    fn lower_std_text_byte_pred(
        &mut self,
        which: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::text::{} takes 1 arg (byte value as Int), got {}",
                which,
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::text::{}: arg must be Int (byte value), got {:?}",
                which, ty
            )));
        }
        let b = v.into_int_value();
        let i64_t = self.context.i64_type();
        let bool_t = self.context.bool_type();

        // Helper: emit `lo <= b && b <= hi` as i1.
        let in_range = |me: &Self, lo: i64, hi: i64, label: &str|
            -> Result<inkwell::values::IntValue<'ctx>, CodegenError>
        {
            let lo_c = i64_t.const_int(lo as u64, true);
            let hi_c = i64_t.const_int(hi as u64, true);
            let ge = me
                .builder
                .build_int_compare(inkwell::IntPredicate::SGE, b, lo_c, &format!("{}.ge", label))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let le = me
                .builder
                .build_int_compare(inkwell::IntPredicate::SLE, b, hi_c, &format!("{}.le", label))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            me.builder
                .build_and(ge, le, &format!("{}.and", label))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
        };
        let eq_const = |me: &Self, n: i64, label: &str|
            -> Result<inkwell::values::IntValue<'ctx>, CodegenError>
        {
            let c = i64_t.const_int(n as u64, true);
            me.builder
                .build_int_compare(inkwell::IntPredicate::EQ, b, c, label)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
        };

        let result = match which {
            "is_alpha" => {
                let lower = in_range(self, b'a' as i64, b'z' as i64, "alpha.lo")?;
                let upper = in_range(self, b'A' as i64, b'Z' as i64, "alpha.up")?;
                self.builder
                    .build_or(lower, upper, "alpha.or")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            "is_digit" => in_range(self, b'0' as i64, b'9' as i64, "digit")?,
            "is_alnum" => {
                let lower = in_range(self, b'a' as i64, b'z' as i64, "alnum.lo")?;
                let upper = in_range(self, b'A' as i64, b'Z' as i64, "alnum.up")?;
                let digit = in_range(self, b'0' as i64, b'9' as i64, "alnum.dig")?;
                let lu = self
                    .builder
                    .build_or(lower, upper, "alnum.lu")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_or(lu, digit, "alnum.or")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            "is_whitespace" => {
                // space, tab, newline, carriage return — matches
                // the C isspace minus \v and \f (rare in practice).
                let sp = eq_const(self, b' ' as i64, "ws.sp")?;
                let tab = eq_const(self, b'\t' as i64, "ws.tab")?;
                let nl = eq_const(self, b'\n' as i64, "ws.nl")?;
                let cr = eq_const(self, b'\r' as i64, "ws.cr")?;
                let a = self
                    .builder
                    .build_or(sp, tab, "ws.a")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let b_ = self
                    .builder
                    .build_or(nl, cr, "ws.b")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_or(a, b_, "ws.or")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            "is_word_char" => {
                // Letter, digit, underscore, apostrophe. The
                // apostrophe matches the convention every
                // wordfreq agent reaches for (so "don't" stays
                // one token).
                let lower = in_range(self, b'a' as i64, b'z' as i64, "wc.lo")?;
                let upper = in_range(self, b'A' as i64, b'Z' as i64, "wc.up")?;
                let digit = in_range(self, b'0' as i64, b'9' as i64, "wc.dig")?;
                let underscore = eq_const(self, b'_' as i64, "wc.us")?;
                let apos = eq_const(self, b'\'' as i64, "wc.hl")?;
                let lu = self
                    .builder
                    .build_or(lower, upper, "wc.lu")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let lud = self
                    .builder
                    .build_or(lu, digit, "wc.lud")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let luda = self
                    .builder
                    .build_or(lud, apos, "wc.luda")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_or(luda, underscore, "wc.or")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            _ => unreachable!("filtered by dispatcher"),
        };
        let _ = bool_t;
        Ok((result.into(), CodegenTy::Bool))
    }

    /// ws-echo `sha1-base64-missing`: lower
    /// `std::text::base64::encode(b: Bytes) -> String`. Standard
    /// alphabet, `=` padding to multiple of 4. Anchored in the
    /// payload arena.
    fn lower_std_text_base64_encode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::encode takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::encode: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_text_base64_encode")
            .expect("lotus_text_base64_encode declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "b64.encode.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// a downstream handoff (2026-06-02): lower
    /// `std::text::base64::url_encode(b: Bytes) -> String`. RFC 4648
    /// §5 URL-safe alphabet (`-`/`_` for 62/63) with NO padding —
    /// the form JWT/JWS, OAuth, and webhook signatures use. Same
    /// shape as `encode`; only the C backer differs.
    fn lower_std_text_base64_url_encode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::url_encode takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::url_encode: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_text_base64url_encode")
            .expect("lotus_text_base64url_encode declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "b64url.encode.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// v1.x-16: lower
    /// `std::text::base64::decode(s: String) -> Bytes`. Standard
    /// alphabet, padding tolerated, whitespace ignored. Returns
    /// the empty Bytes blob on parse failure (non-alphabet char,
    /// wrong length, too much padding). Anchored in the payload
    /// arena.
    fn lower_std_text_base64_decode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::decode takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::decode: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let f = self
            .module
            .get_function("lotus_text_base64_decode")
            .expect("lotus_text_base64_decode declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "b64.decode.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }
}
