//! `std::crypto::ecdsa_p256_sign` / `ecdsa_p256_verify` — ES256
//! (ECDSA over P-256 + SHA-256), the primitive Coinbase Advanced
//! Trade (and most CDP/JWT) auth needs. Added for the fathom
//! handoff. Signatures are raw r‖s (64 bytes, JWS/COSE form).
//!
//! Compiled-path test (the C is OpenSSL in lotus_tls.c, no
//! interpreter surface). Covers:
//!   * round-trip: a hale-made signature verifies with the public key;
//!   * tamper: a different message does NOT verify;
//!   * KAT / interop: a signature produced OUT OF BAND by Python's
//!     `cryptography` (PyJWT's backend) verifies in hale — i.e. hale's
//!     verify accepts an external ES256 signature, and the raw-r‖s /
//!     curve / hash wiring matches the rest of the world.
//!
//! The keypair is a fixed P-256 test key (generated with
//! `openssl ecparam -genkey -name prime256v1`); the external sig was
//! produced by `cryptography` over b"hello ES256" with this key.

use std::process::Command;

use hale_codegen::build_executable;

// Fixed P-256 test keypair (NOT a secret — generated for this test).
const PRIV_SEC1_PEM: &str = r#"-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIJrK0USBk0pXfFnQtXL9xFkQSdZ9C1OUbBcO5dnIWy8/oAoGCCqGSM49
AwEHoUQDQgAEiGxPneeFcgjIV3jH5esGYi0uNMCRw16VEVuDZZbkiQ05htqoeEZY
ZRF072NcMoZ2mbTgdIhBH9E13hgKZmhQFQ==
-----END EC PRIVATE KEY-----
"#;

const PUB_SPKI_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEiGxPneeFcgjIV3jH5esGYi0uNMCR
w16VEVuDZZbkiQ05htqoeEZYZRF072NcMoZ2mbTgdIhBH9E13hgKZmhQFQ==
-----END PUBLIC KEY-----
"#;

// `cryptography`-made ES256 sig over b"hello ES256" with the key
// above, raw r‖s, STANDARD base64 (decoded in-language via
// std::text::base64::decode). Proves verify interoperates with an
// external signer.
const EXTERNAL_SIG_STD_B64: &str =
    "Ddn0I00RsANZl/oE0IIdMswZ8c1D/JkNrY/U+SCmsTIiuhLGSI9H/B6CF15AdOT1D/b5eDpeI98ulGwqAOywTw==";

fn build_and_run(name: &str, source: &str) -> String {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_ecdsa_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn ecdsa_p256_sign_verify_and_external_interop() {
    // Real newlines -> the `\n` escape hale's lexer expects in a
    // string literal, so the PEM reaches the OpenSSL parser intact.
    let priv_lit = PRIV_SEC1_PEM.replace('\n', "\\n");
    let pub_lit = PUB_SPKI_PEM.replace('\n', "\\n");

    let src = format!(
        r#"
fn main() {{
    let privk = std::bytes::from_string("{priv_lit}");
    let pubk  = std::bytes::from_string("{pub_lit}");
    let msg = std::bytes::from_string("hello ES256");

    // Round-trip: our own signature verifies.
    let sig = std::crypto::ecdsa_p256_sign(privk, msg);
    println("rt=", std::crypto::ecdsa_p256_verify(pubk, msg, sig));

    // Tamper: a different message must not verify against `sig`.
    let other = std::bytes::from_string("hello ES257");
    println("tamper=", std::crypto::ecdsa_p256_verify(pubk, other, sig));

    // Interop KAT: an externally-produced signature verifies.
    let ext = std::text::base64::decode("{ext}");
    println("kat=", std::crypto::ecdsa_p256_verify(pubk, msg, ext));
}}
"#,
        priv_lit = priv_lit,
        pub_lit = pub_lit,
        ext = EXTERNAL_SIG_STD_B64,
    );

    let out = build_and_run("interop", &src);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["rt=true", "tamper=false", "kat=true"],
        "ecdsa_p256 sign/verify/interop diverged; got: {:?}",
        out
    );
}

#[test]
fn ecdsa_p256_sign_fallible_or_raise_success() {
    // The `or`-context form: a valid key signs and the success
    // value flows through `or raise` as a plain Bytes — the
    // signature still verifies.
    let priv_lit = PRIV_SEC1_PEM.replace('\n', "\\n");
    let pub_lit = PUB_SPKI_PEM.replace('\n', "\\n");
    let src = format!(
        r#"
fn main() {{
    let privk = std::bytes::from_string("{priv_lit}");
    let pubk  = std::bytes::from_string("{pub_lit}");
    let msg = std::bytes::from_string("hello ES256");
    let sig = std::crypto::ecdsa_p256_sign(privk, msg) or raise;
    println("len=", len(sig));
    println("rt=", std::crypto::ecdsa_p256_verify(pubk, msg, sig));
}}
"#,
        priv_lit = priv_lit,
        pub_lit = pub_lit,
    );
    let out = build_and_run("fallible_ok", &src);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["len=64", "rt=true"],
        "fallible ecdsa sign success path diverged; got: {:?}",
        out
    );
}

#[test]
fn ecdsa_p256_sign_fallible_binds_crypto_error_on_bad_key() {
    // A bad key drives the failure branch; `or handler(err)` binds
    // the implicit `err: CryptoError` and the handler reads its
    // `kind` / `detail` fields, then substitutes a fallback Bytes.
    let src = r#"
fn diag(e: CryptoError) -> Bytes {
    println("kind=", e.kind);
    println("detail=", e.detail);
    return std::bytes::from_string("");
}
fn main() {
    let badk = std::bytes::from_string("-----BEGIN EC PRIVATE KEY-----\nnot a real key\n-----END EC PRIVATE KEY-----\n");
    let msg = std::bytes::from_string("hello ES256");
    let sig = std::crypto::ecdsa_p256_sign(badk, msg) or diag(err);
    println("fallback_len=", len(sig));
}
"#;
    let out = build_and_run("fallible_err", src);
    assert!(
        out.contains("kind=ecdsa_p256_sign"),
        "expected CryptoError.kind bound; got: {:?}",
        out
    );
    assert!(
        out.contains("detail=signing failed"),
        "expected CryptoError.detail bound; got: {:?}",
        out
    );
    assert!(
        out.contains("fallback_len=0"),
        "expected the substitute fallback Bytes; got: {:?}",
        out
    );
}
