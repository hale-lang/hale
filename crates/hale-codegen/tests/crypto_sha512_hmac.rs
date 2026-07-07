//! (a downstream app) handoff (2026-06-25) — `std::crypto::sha512` and
//! `std::crypto::hmac_sha512`. The 64-bit-word sibling of the
//! `crypto_sha256_hmac` tests, same shape (decimal-byte stdout → hex).
//!
//! Vectors:
//!   - FIPS 180-4: sha512("abc"), sha512(""), sha512(112-byte two-block)
//!   - RFC 4231 test 1: hmac_sha512(key=0x0b*20, msg="Hi There")
//!   - RFC 4231 test 2: hmac_sha512(key="Jefe", msg="what do ya want for
//!     nothing?") — the venue-signing shape (string key + string msg)

use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_crypto_sha512_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// `tag= b0 b1 …` (decimal bytes) → lowercase hex.
fn extract_hex(stdout: &str, tag: &str) -> String {
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{}=", tag)) {
            let mut hex = String::new();
            for token in rest.split_whitespace() {
                let v: u32 = token.parse().expect("decimal byte");
                hex.push_str(&format!("{:02x}", v));
            }
            return hex;
        }
    }
    panic!("tag {:?} not in stdout:\n{}", tag, stdout);
}

// A `print_digest` that emits 64 bytes (SHA-512 / HMAC-SHA512 output).
const PRELUDE: &str = r#"
    fn print_digest(name: String, d: Bytes) {
        let mut s = name + "=";
        let mut i = 0;
        while i < 64 {
            let b = std::bytes::at(d, i) or 0;
            s = s + " " + b;
            i = i + 1;
        }
        println(s);
    }
"#;

#[test]
fn sha512_fips_vectors() {
    let src = format!(
        r#"
        {PRELUDE}
        fn main() {{
            // FIPS 180-4 §C.1
            print_digest("abc", std::crypto::sha512(std::bytes::from_string("abc")));
            // Empty
            print_digest("empty", std::crypto::sha512(std::bytes::from_string("")));
            // FIPS 180-4 §C.2 — 112-byte input spanning two 128-byte blocks
            // after padding (forces the multi-block path).
            let b2 = std::bytes::from_string(
                "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
            );
            print_digest("b2", std::crypto::sha512(b2));
        }}
    "#
    );
    let bin = build("fips_vectors", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        extract_hex(&stdout, "abc"),
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
         2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        "stdout: {}",
        stdout
    );
    assert_eq!(
        extract_hex(&stdout, "empty"),
        "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
         47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
        "stdout: {}",
        stdout
    );
    assert_eq!(
        extract_hex(&stdout, "b2"),
        "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018\
         501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
        "stdout: {}",
        stdout
    );
}

#[test]
fn hmac_sha512_rfc4231() {
    // Test 1: key = 0x0b*20, msg = "Hi There".
    // Test 2: key = "Jefe", msg = "what do ya want for nothing?".
    let src = format!(
        r#"
        {PRELUDE}
        fn main() {{
            let mut key1 = std::bytes::from_string("");
            let mut i = 0;
            while i < 20 {{
                key1 = std::bytes::concat(key1, std::bytes::from_int(0x0B));
                i = i + 1;
            }}
            print_digest("hmac1",
                std::crypto::hmac_sha512(key1, std::bytes::from_string("Hi There")));

            print_digest("hmac2", std::crypto::hmac_sha512(
                std::bytes::from_string("Jefe"),
                std::bytes::from_string("what do ya want for nothing?")));
        }}
    "#
    );
    let bin = build("rfc4231", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        extract_hex(&stdout, "hmac1"),
        "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cde\
         daa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
        "stdout: {}",
        stdout
    );
    assert_eq!(
        extract_hex(&stdout, "hmac2"),
        "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea250554\
         9758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737",
        "stdout: {}",
        stdout
    );
}
