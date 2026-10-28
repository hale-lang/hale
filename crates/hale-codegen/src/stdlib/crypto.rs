//! `std::crypto::*` path-call lowering.

use hale_syntax::ast::Expr;
use hale_types::stdlib_surface::IntrinsicId;
use inkwell::values::BasicValueEnum;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope,
};

pub(crate) trait CryptoStdlib<'ctx> {
    fn lower_std_crypto_sha1(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_sha256(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_sha256_raw(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_sha256_file_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_crypto_hmac_sha256(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_sha512(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_hmac_sha512(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_crc32(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;


    fn lower_std_crypto_ecdsa_p256_sign_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_crypto_ecdsa_p256_verify(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_ed25519_public_key(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_crypto_ed25519_sign_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_crypto_ed25519_verify(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Lower `args` (each Bytes, a view unpacked) for `std::crypto::<surface>`,
    /// whose parameters `names` are.
    fn lower_crypto_bytes_args(
        &mut self,
        surface: &str,
        names: &[&str],
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Vec<BasicValueEnum<'ctx>>, CodegenError> {
        if args.len() != names.len() {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::{surface} takes {} args ({}), got {}",
                names.len(),
                names.join(", "),
                args.len()
            )));
        }
        let mut vals = Vec::with_capacity(names.len());
        for (arg, name) in args.iter().zip(names) {
            let (v, ty) = self.lower_expr(arg, scope)?;
            if !matches!(ty, CodegenTy::Bytes | CodegenTy::BytesView) {
                return Err(CodegenError::Unsupported(format!(
                    "std::crypto::{surface}: {name} must be Bytes, got {ty:?}"
                )));
            }
            vals.push(self.unpack_view_if_needed(v, &ty)?);
        }
        Ok(vals)
    }
}

impl<'ctx, 'p> CryptoStdlib<'ctx> for Cx<'ctx, 'p> {
    /// ws-echo `sha1-base64-missing`: lower
    /// `std::crypto::sha1(b: Bytes) -> Bytes`. Returns a 20-byte
    /// digest. Stand-alone implementation in the C runtime per
    /// RFC 3174 — no OpenSSL dependency. Anchored in the
    /// program-lifetime payload arena.
    fn lower_std_crypto_sha1(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha1 takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha1: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_sha1")
            .expect("lotus_crypto_sha1 declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sha1.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// C3 (pond follow-up): lower
    /// `std::crypto::sha256(b: Bytes) -> Bytes`. Returns a 32-byte
    /// digest per FIPS 180-4. Stand-alone implementation in the
    /// C runtime — no libcrypto dependency. Anchored in the
    /// program-lifetime payload arena. Drops pond/crypto's
    /// ~140-line pure-Hale O(N²) sha256.hl.
    fn lower_std_crypto_sha256(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha256 takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha256: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_sha256")
            .expect("lotus_crypto_sha256 declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sha256.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// The `std::crypto::Sha256` locus's primitives: `__sha256_new() ->
    /// Int`, `__sha256_update(handle, b) -> Int`, `__sha256_finish(handle)
    /// -> Bytes` (the 32-byte digest; the context starts over) and
    /// `__sha256_free(handle) -> Int`. The handle is the C context's
    /// pointer as an i64, passed to the runtime as one.
    fn lower_std_crypto_sha256_raw(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let (surface, c_fn, arity) = match id {
            IntrinsicId::CryptoSha256NewRaw => ("__sha256_new", "lotus_crypto_sha256_new", 0),
            IntrinsicId::CryptoSha256UpdateRaw => {
                ("__sha256_update", "lotus_crypto_sha256_update", 2)
            }
            IntrinsicId::CryptoSha256FinishRaw => {
                ("__sha256_finish", "lotus_crypto_sha256_finish", 1)
            }
            IntrinsicId::CryptoSha256FreeRaw => ("__sha256_free", "lotus_crypto_sha256_free", 1),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "not a std::crypto::Sha256 primitive: {other:?}"
                )))
            }
        };
        if args.len() != arity {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::{surface} takes {arity} args, got {}",
                args.len()
            )));
        }
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        if arity >= 1 {
            let (h, ty) = self.lower_expr(&args[0], scope)?;
            if ty != CodegenTy::Int {
                return Err(CodegenError::Unsupported(format!(
                    "std::crypto::{surface}: handle must be Int, got {ty:?}"
                )));
            }
            call_args.push(h.into());
        }
        if arity == 2 {
            let (b, ty) = self.lower_expr(&args[1], scope)?;
            if !matches!(ty, CodegenTy::Bytes | CodegenTy::BytesView) {
                return Err(CodegenError::Unsupported(format!(
                    "std::crypto::{surface}: b must be Bytes, got {ty:?}"
                )));
            }
            call_args.push(self.unpack_view_if_needed(b, &ty)?.into());
        }
        let f = self
            .module
            .get_function(c_fn)
            .unwrap_or_else(|| panic!("{c_fn} declared"));
        let v = self
            .builder
            .build_call(f, &call_args, "sha256.raw")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns a value");
        let ty = if id == IntrinsicId::CryptoSha256FinishRaw {
            CodegenTy::Bytes
        } else {
            CodegenTy::Int
        };
        Ok((v, ty))
    }

    /// `std::crypto::sha256_file(path: String) -> Bytes fallible(IoError)`:
    /// the file read in fixed-size blocks through the SHA-256 core, so
    /// memory stays one block whatever the file's size. NULL from the
    /// runtime is the failure, errno its kind, the path its detail.
    fn lower_std_crypto_sha256_file_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha256_file takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(path_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha256_file: path must be String, got {path_ty:?}"
            )));
        }
        let path_val = self.unpack_view_if_needed(path_val, &path_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_sha256_file")
            .expect("lotus_crypto_sha256_file declared");
        let out = self
            .builder
            .build_call(f, &[path_val.into()], "sha256_file.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        let is_err = self
            .builder
            .build_is_null(out, "sha256_file.is_err")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.complete_io_fallible_call(
            is_err,
            path_val,
            Some((out.into(), CodegenTy::Bytes)),
            "crypto.sha256_file",
        )
    }

    /// C3 (pond follow-up): lower
    /// `std::crypto::hmac_sha256(key: Bytes, msg: Bytes) -> Bytes`.
    /// Returns the 32-byte HMAC tag per RFC 2104. Anchored in
    /// the payload arena. Drops pond/crypto's hmac.hl wrapper.
    fn lower_std_crypto_hmac_sha256(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::hmac_sha256 takes 2 args (key, msg), got {}",
                args.len()
            )));
        }
        let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(key_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::hmac_sha256: key must be Bytes, got {:?}",
                key_ty
            )));
        }
        let key_val = self.unpack_view_if_needed(key_val, &key_ty)?;
        let (msg_val, msg_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(msg_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::hmac_sha256: msg must be Bytes, got {:?}",
                msg_ty
            )));
        }
        let msg_val = self.unpack_view_if_needed(msg_val, &msg_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_hmac_sha256")
            .expect("lotus_crypto_hmac_sha256 declared");
        let call = self
            .builder
            .build_call(
                f,
                &[key_val.into(), msg_val.into()],
                "hmac_sha256.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// `std::crypto::sha512(b: Bytes) -> Bytes`. Returns a 64-byte digest
    /// per FIPS 180-4 — the 64-bit-word sibling of `sha256`. Hand-rolled in
    /// the C runtime (no libcrypto), anchored in the payload arena.
    fn lower_std_crypto_sha512(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha512 takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha512: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_sha512")
            .expect("lotus_crypto_sha512 declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sha512.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// 2026-06-25 ((a downstream app)): lower
    /// `std::crypto::hmac_sha512(key: Bytes, msg: Bytes) -> Bytes`.
    /// Returns the 64-byte HMAC tag per RFC 2104 — the sibling of
    /// `hmac_sha256`, with SHA-512 as the inner hash (128-byte block).
    fn lower_std_crypto_hmac_sha512(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::hmac_sha512 takes 2 args (key, msg), got {}",
                args.len()
            )));
        }
        let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(key_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::hmac_sha512: key must be Bytes, got {:?}",
                key_ty
            )));
        }
        let key_val = self.unpack_view_if_needed(key_val, &key_ty)?;
        let (msg_val, msg_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(msg_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::hmac_sha512: msg must be Bytes, got {:?}",
                msg_ty
            )));
        }
        let msg_val = self.unpack_view_if_needed(msg_val, &msg_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_hmac_sha512")
            .expect("lotus_crypto_hmac_sha512 declared");
        let call = self
            .builder
            .build_call(
                f,
                &[key_val.into(), msg_val.into()],
                "hmac_sha512.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// 2026-05-27: lower
    /// `std::crypto::crc32(b: Bytes) -> Int`. IEEE 802.3
    /// reversed polynomial (`0xEDB88320`), init `0xFFFFFFFF`,
    /// final XOR `0xFFFFFFFF` — the variant zlib's `crc32()`
    /// and Python's `binascii.crc32` return. Returns the
    /// 4-byte checksum as `Int` (caller casts/compares as
    /// needed). Non-fallible, no arena allocation.
    fn lower_std_crypto_crc32(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::crc32 takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(b_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::crc32: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let b_val = self.unpack_view_if_needed(b_val, &b_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_crc32")
            .expect("lotus_crypto_crc32 declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "crc32.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let iv = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((iv, CodegenTy::Int))
    }

    /// a downstream handoff (2026-06-02, fallible 2026-06-04): lower
    /// `std::crypto::ecdsa_p256_sign(key: Bytes, message: Bytes) ->
    /// Bytes fallible(CryptoError)`. ES256 — SHA-256 the message, ECDSA
    /// over P-256, the 64-byte raw r‖s signature (JWS/COSE form). `key`
    /// is a PEM EC private key (SEC1 or PKCS#8). Calls the
    /// NULL-returning runtime symbol; a NULL result (bad/unparseable key,
    /// non-P-256 curve, signing failure) becomes a `CryptoError { kind:
    /// "ecdsa_p256_sign", detail: "signing failed (bad key or non-P-256
    /// curve)" }`. Backed by OpenSSL in lotus_tls.c. Its one mode since
    /// F.40 phase 4, S5: the bare form that answered an empty Bytes is
    /// gone.
    fn lower_std_crypto_ecdsa_p256_sign_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::ecdsa_p256_sign takes 2 args (key, message), got {}",
                args.len()
            )));
        }
        let (key_val, key_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(key_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::ecdsa_p256_sign: key must be Bytes, got {:?}",
                key_ty
            )));
        }
        let (msg_val, msg_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(msg_ty, CodegenTy::Bytes | CodegenTy::BytesView) {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::ecdsa_p256_sign: message must be Bytes, got {:?}",
                msg_ty
            )));
        }
        let key_val = self.unpack_view_if_needed(key_val, &key_ty)?;
        let msg_val = self.unpack_view_if_needed(msg_val, &msg_ty)?;
        let f = self
            .module
            .get_function("lotus_crypto_ecdsa_p256_sign_or_null")
            .expect("lotus_crypto_ecdsa_p256_sign_or_null declared");
        let sig_ptr = self
            .builder
            .build_call(
                f,
                &[key_val.into(), msg_val.into()],
                "ecdsa.sign.or_null",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        // is_err = (sig_ptr == null)
        let is_err = self
            .builder
            .build_is_null(sig_ptr.into_pointer_value(), "ecdsa.sign.is_err")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let detail_ptr =
            self.global_string("signing failed (bad key or non-P-256 curve)");
        self.complete_crypto_fallible_call(
            is_err,
            detail_ptr.into(),
            "ecdsa_p256_sign",
            Some((sig_ptr, CodegenTy::Bytes)),
            "crypto.ecdsa_p256_sign",
        )
    }

    /// a downstream handoff (2026-06-02): lower
    /// `std::crypto::ecdsa_p256_verify(pubkey: Bytes, message: Bytes,
    /// sig: Bytes) -> Bool`. `pubkey` is a PEM SPKI EC public key;
    /// `sig` is the 64-byte raw r‖s. SHA-256 + ECDSA verify in
    /// lotus_tls.c (OpenSSL). The C fn returns i64 1/0; narrow to i1.
    fn lower_std_crypto_ecdsa_p256_verify(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::ecdsa_p256_verify takes 3 args (pubkey, \
                 message, sig), got {}",
                args.len()
            )));
        }
        let mut vals = Vec::with_capacity(3);
        for (i, name) in ["pubkey", "message", "sig"].iter().enumerate() {
            let (v, ty) = self.lower_expr(&args[i], scope)?;
            if !matches!(ty, CodegenTy::Bytes | CodegenTy::BytesView) {
                return Err(CodegenError::Unsupported(format!(
                    "std::crypto::ecdsa_p256_verify: {} must be Bytes, got {:?}",
                    name, ty
                )));
            }
            vals.push(self.unpack_view_if_needed(v, &ty)?);
        }
        let f = self
            .module
            .get_function("lotus_crypto_ecdsa_p256_verify")
            .expect("lotus_crypto_ecdsa_p256_verify declared");
        let call = self
            .builder
            .build_call(
                f,
                &[vals[0].into(), vals[1].into(), vals[2].into()],
                "ecdsa.verify.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let iv = call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        // i64 (1/0) -> i1 Bool.
        let b = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                iv,
                self.context.i64_type().const_zero(),
                "ecdsa.verify.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((b.into(), CodegenTy::Bool))
    }

    /// `std::crypto::__ed25519_public_key(seed: Bytes) -> Bytes`: the
    /// 32-byte Ed25519 public key of a 32-byte seed, or an empty Bytes
    /// for any other length — `ed25519_keypair`'s body (`crypto.hl`)
    /// refuses that with its CryptoError.
    fn lower_std_crypto_ed25519_public_key(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let vals = self.lower_crypto_bytes_args("__ed25519_public_key", &["seed"], args, scope)?;
        let f = self
            .module
            .get_function("lotus_ed25519_public_key")
            .expect("lotus_ed25519_public_key declared");
        let v = self
            .builder
            .build_call(f, &[vals[0].into()], "ed25519.public_key")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((v, CodegenTy::Bytes))
    }

    /// `std::crypto::ed25519_sign(secret: Bytes, message: Bytes) -> Bytes
    /// fallible(CryptoError)`: the 64-byte RFC 8032 signature. `secret` is
    /// the 32-byte seed or the 64-byte seed + public key; a NULL from the
    /// runtime (any other length, or a public half that is not the
    /// seed's) is `CryptoError { kind: "ed25519_sign", detail }`.
    fn lower_std_crypto_ed25519_sign_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let vals = self.lower_crypto_bytes_args("ed25519_sign", &["secret", "message"], args, scope)?;
        let f = self
            .module
            .get_function("lotus_ed25519_sign_or_null")
            .expect("lotus_ed25519_sign_or_null declared");
        let sig_ptr = self
            .builder
            .build_call(f, &[vals[0].into(), vals[1].into()], "ed25519.sign.or_null")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        let is_err = self
            .builder
            .build_is_null(sig_ptr.into_pointer_value(), "ed25519.sign.is_err")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let detail_ptr = self.global_string(
            "the secret key is neither a 32-byte seed nor a 64-byte seed and its public key",
        );
        self.complete_crypto_fallible_call(
            is_err,
            detail_ptr.into(),
            "ed25519_sign",
            Some((sig_ptr, CodegenTy::Bytes)),
            "crypto.ed25519_sign",
        )
    }

    /// `std::crypto::ed25519_verify(public: Bytes, message: Bytes,
    /// signature: Bytes) -> Bool`: RFC 8032 verification (S < L enforced);
    /// a public key that is not 32 bytes or a signature that is not 64 is
    /// `false`. The C fn returns i64 1/0; narrowed to i1.
    fn lower_std_crypto_ed25519_verify(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let vals =
            self.lower_crypto_bytes_args("ed25519_verify", &["public", "message", "signature"], args, scope)?;
        let f = self
            .module
            .get_function("lotus_ed25519_verify")
            .expect("lotus_ed25519_verify declared");
        let iv = self
            .builder
            .build_call(f, &[vals[0].into(), vals[1].into(), vals[2].into()], "ed25519.verify.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let b = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                iv,
                self.context.i64_type().const_zero(),
                "ed25519.verify.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((b.into(), CodegenTy::Bool))
    }
}
